mod dns;

use std::collections::{HashMap, HashSet};

use indexmap::IndexMap;
use serde::{Deserialize, Serialize};
use serde_yaml_ng::{Mapping, Value};

use crate::catalog::{self, Module, ModuleKind, Preset};
use crate::error::{Error, Result};
use crate::model::{
    ChainSpec, CustomGroupSpec, GroupKind, Node, PolicyGroupSpec, Proxy, ProxyGroup, Rule,
};
use crate::normalize::{enabled_nodes, ensure_unique_names};
use crate::region::Region;

pub use dns::DnsOptions;

const HEALTH_CHECK_URL: &str = "https://www.gstatic.com/generate_204";
const SELECT_GROUP: &str = "节点选择";
const AUTO_GROUP: &str = "自动选择";
const FINAL_GROUP: &str = "漏网之鱼";

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct RuleSetBinding {
    pub id: String,
    pub name: String,
    pub url: String,
    pub behavior: String,
    pub format: String,
    pub target: String,
    pub no_resolve: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ListenerSpec {
    pub name: String,
    pub port: u16,
    pub proxy: String,
    #[serde(default = "default_listener_kind")]
    pub kind: String,
    #[serde(default = "default_listener_address")]
    pub listen: String,
    #[serde(default)]
    pub udp: bool,
}
fn default_listener_kind() -> String {
    "mixed".into()
}
fn default_listener_address() -> String {
    "127.0.0.1".into()
}

#[derive(Clone, Debug)]
pub struct GenerateOptions {
    pub preset: Preset,
    pub enabled_modules: Option<Vec<String>>,
    pub test_url: String,
    pub test_interval: u32,
    pub mixed_port: u16,
    pub allow_lan: bool,
    pub rule_provider_base_url: String,
    pub disabled_rule_sets: Vec<String>,
    pub custom_rule_sets: Vec<RuleSetBinding>,
    pub custom_rules: Vec<Rule>,
    pub custom_groups: Vec<CustomGroupSpec>,
    pub policy_groups: Vec<PolicyGroupSpec>,
    pub rule_set_targets: HashMap<String, String>,
    pub chains: Vec<ChainSpec>,
    pub dns: DnsOptions,
    pub rule_order: Vec<String>,
    pub listeners: Vec<ListenerSpec>,
    pub extra_yaml: String,
}

impl Default for GenerateOptions {
    fn default() -> Self {
        Self {
            preset: Preset::Standard,
            enabled_modules: None,
            test_url: HEALTH_CHECK_URL.to_string(),
            test_interval: 300,
            mixed_port: 7897,
            allow_lan: false,
            rule_provider_base_url:
                "https://github.com/MetaCubeX/meta-rules-dat/raw/refs/heads/meta/geo".to_string(),
            disabled_rule_sets: Vec::new(),
            custom_rule_sets: Vec::new(),
            custom_rules: Vec::new(),
            custom_groups: Vec::new(),
            policy_groups: Vec::new(),
            rule_set_targets: HashMap::new(),
            chains: Vec::new(),
            dns: DnsOptions::default(),
            rule_order: Vec::new(),
            listeners: Vec::new(),
            extra_yaml: String::new(),
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct ProfileStats {
    pub proxies: usize,
    pub groups: usize,
    pub rules: usize,
    pub rule_sets: usize,
}

#[derive(Clone, Debug)]
pub struct GeneratedProfile {
    pub yaml: String,
    pub stats: ProfileStats,
    pub warnings: Vec<String>,
    pub rules: Vec<Rule>,
    pub groups: Vec<ProxyGroup>,
    pub trace: serde_json::Value,
}

pub fn generate_profile(nodes: &[Node], options: &GenerateOptions) -> Result<GeneratedProfile> {
    let mut warnings = Vec::new();
    let mut nodes = enabled_nodes(nodes);
    nodes.retain(|node| {
        if let Some(issue) = node.proxy.fatal_issue() {
            warnings.push(format!("已跳过节点「{}」：{issue}", node.name()));
            false
        } else {
            true
        }
    });
    ensure_unique_names(&mut nodes);
    if nodes.is_empty() {
        return Err(Error::NoNodes);
    }

    let modules = resolve_enabled_modules(options);
    let mut groups = build_groups(&nodes, &modules, options, &mut warnings);
    let builtin_names: HashSet<String> = groups.iter().map(|g| g.name.clone()).collect();
    let final_group = groups.pop();

    let mut used_group_names: HashSet<String> =
        groups.iter().map(|group| group.name.clone()).collect();
    if let Some(final_group) = &final_group {
        used_group_names.insert(final_group.name.clone());
    }
    let mut used_proxy_names: HashSet<String> =
        nodes.iter().map(|node| node.name().to_string()).collect();

    groups.extend(build_custom_groups(
        &nodes,
        options,
        &mut used_group_names,
        &mut warnings,
    ));
    let (chain_proxies, chain_groups) = build_chains(
        &nodes,
        options,
        &mut used_group_names,
        &mut used_proxy_names,
    )?;
    groups.extend(chain_groups);
    if let Some(final_group) = final_group {
        groups.push(final_group);
    }

    let mut overridden = HashSet::new();
    for spec in &options.policy_groups {
        let name = spec.name.trim();
        if name.is_empty()
            || name.contains([',', '\n', '\r'])
            || name != spec.name
            || (used_group_names.contains(name) && !spec.builtin)
            || (spec.builtin
                && (!builtin_names.contains(name) || !overridden.insert(name.to_string())))
            || used_proxy_names.contains(name)
            || [
                "DIRECT",
                "REJECT",
                "REJECT-DROP",
                "PASS",
                "COMPATIBLE",
                "GLOBAL",
            ]
            .iter()
            .any(|reserved| reserved.eq_ignore_ascii_case(name))
        {
            return Err(Error::UnknownRuleTarget(format!(
                "策略组名称无效或重复：{}",
                spec.name
            )));
        }
        if spec.kind == GroupKind::Relay {
            return Err(Error::UnknownRuleTarget(format!(
                "策略组「{name}」请使用独立的链式代理配置"
            )));
        }
        used_group_names.insert(name.to_string());
        let mut members = spec.members.clone();
        if spec.include_all_nodes {
            members.extend(nodes.iter().map(|node| node.name().to_string()));
        }
        let mut seen = HashSet::new();
        members.retain(|member| seen.insert(member.clone()));
        let mut group = ProxyGroup::new(name, spec.kind, members);
        if spec.kind != GroupKind::Select {
            let url = spec.test_url.as_deref().unwrap_or(&options.test_url);
            let interval = spec.test_interval.unwrap_or(options.test_interval);
            if !(url.starts_with("https://") || url.starts_with("http://")) || interval == 0 {
                return Err(Error::UnknownRuleTarget(format!(
                    "策略组「{name}」测速参数无效"
                )));
            }
            group = group.with_health_check(url, interval);
        }
        if spec.builtin {
            let index = groups
                .iter()
                .position(|g| g.name == name)
                .expect("validated builtin");
            groups[index] = group;
        } else {
            groups.push(group);
        }
    }

    let group_names: HashSet<String> = groups.iter().map(|group| group.name.clone()).collect();
    let proxy_names = used_proxy_names;

    let (rules, providers) = build_rules(&modules, options, &group_names, &mut warnings)?;
    validate_group_references(&groups, &group_names, &proxy_names)?;

    let mut proxies: Vec<Value> = nodes.iter().map(|node| node.proxy.to_value()).collect();
    proxies.extend(chain_proxies.iter().map(Proxy::to_value));
    let mut root = build_root(proxies, &groups, &rules, &providers, options);
    if options.mixed_port == 0 || options.dns.domestic.is_empty() || options.dns.foreign.is_empty()
    {
        return Err(Error::InvalidLink(
            "监听端口不能为零，DNS 列表不能为空".into(),
        ));
    }
    let mut listener_names = HashSet::new();
    let mut ports = HashSet::from([options.mixed_port]);
    let mut listeners = Vec::new();
    for listener in &options.listeners {
        if listener.name.trim().is_empty()
            || !listener_names.insert(&listener.name)
            || listener.port == 0
            || !ports.insert(listener.port)
            || !matches!(listener.kind.as_str(), "mixed" | "http" | "socks")
            || listener.listen.parse::<std::net::IpAddr>().is_err()
            || !(group_names.contains(&listener.proxy)
                || proxy_names.contains(&listener.proxy)
                || matches!(listener.proxy.as_str(), "DIRECT" | "REJECT"))
        {
            return Err(Error::InvalidLink(format!(
                "监听配置无效、端口冲突或目标不存在：{}",
                listener.name
            )));
        }
        listeners.push(serde_yaml_ng::to_value(listener)?);
    }
    if !listeners.is_empty() {
        // Mihomo uses 'type', whereas the Rust/UI field is named 'kind'.
        for value in &mut listeners {
            let map = value.as_mapping_mut().expect("listener mapping");
            let kind = map
                .remove(Value::String("kind".into()))
                .expect("listener kind");
            map.insert(Value::String("type".into()), kind);
        }
        root.insert(
            Value::String("listeners".into()),
            Value::Sequence(listeners),
        );
    }
    if !options.extra_yaml.trim().is_empty() {
        let extra: Value = serde_yaml_ng::from_str(&options.extra_yaml)?;
        let extra = extra
            .as_mapping()
            .ok_or_else(|| Error::InvalidLink("扩展 YAML 必须是顶层映射".into()))?;
        for (key, value) in extra {
            let key_str = key
                .as_str()
                .ok_or_else(|| Error::InvalidLink("扩展 YAML 键必须为字符串".into()))?;
            if matches!(
                key_str,
                "proxies"
                    | "proxy-groups"
                    | "rules"
                    | "rule-providers"
                    | "listeners"
                    | "mixed-port"
                    | "allow-lan"
            ) {
                return Err(Error::InvalidLink(format!(
                    "{key_str} 请通过对应编辑器修改"
                )));
            }
            root.insert(key.clone(), value.clone());
        }
    }

    let body = serde_yaml_ng::to_string(&Value::Mapping(root))?;
    use sha2::{Digest, Sha256};
    let digest = format!("{:x}", Sha256::digest(body.as_bytes()));
    let trace = serde_json::json!({
        "format":"swiftsub-generation-report","version":1,"body_sha256":digest,
        "generator_version":env!("CARGO_PKG_VERSION"),
        "nodes":nodes.iter().map(|n|serde_json::json!({"name":n.name(),"protocol":n.proxy.kind(),"source_id":n.source_id,"source_name":n.source_name,"source_key":n.source_key})).collect::<Vec<_>>(),
        "groups":groups.iter().map(|g|serde_json::json!({"name":g.name,"type":g.kind.as_str(),"members":g.proxies})).collect::<Vec<_>>(),
        "rules":rules.iter().enumerate().map(|(i,r)|serde_json::json!({"position":i+1,"rule":r.to_line(),"origin":if options.custom_rules.contains(r){"custom"}else if r.kind=="RULE-SET"{"rule-provider"}else{"builtin"}})).collect::<Vec<_>>(),
        "warnings":warnings,
    });
    let yaml = format!(
        "# Generated by SwiftSub {}\n# Body SHA-256: {digest}\n# This file is generated; manual edits will be overwritten.\n{}",
        env!("CARGO_PKG_VERSION"),
        body
    );

    let stats = ProfileStats {
        proxies: nodes.len() + chain_proxies.len(),
        groups: groups.len(),
        rules: rules.len(),
        rule_sets: providers.len(),
    };

    Ok(GeneratedProfile {
        yaml,
        stats,
        warnings,
        rules,
        groups,
        trace,
    })
}

fn resolve_enabled_modules(options: &GenerateOptions) -> Vec<&'static Module> {
    let mut ids: HashSet<String> = match &options.enabled_modules {
        Some(modules) => modules.iter().cloned().collect(),
        None => catalog::modules_for_preset(options.preset)
            .into_iter()
            .map(|module| module.id.to_string())
            .collect(),
    };

    for required in ["select", "auto", "final"] {
        ids.insert(required.to_string());
    }

    catalog::MODULES
        .iter()
        .filter(|module| ids.contains(module.id))
        .collect()
}

fn build_groups(
    nodes: &[Node],
    modules: &[&'static Module],
    options: &GenerateOptions,
    warnings: &mut Vec<String>,
) -> Vec<ProxyGroup> {
    let node_names: Vec<String> = nodes.iter().map(|node| node.name().to_string()).collect();

    let mut groups = Vec::new();
    groups.push(ProxyGroup::new(
        SELECT_GROUP,
        GroupKind::Select,
        node_names.clone(),
    ));
    groups.push(
        ProxyGroup::new(AUTO_GROUP, GroupKind::UrlTest, node_names.clone())
            .with_health_check(&options.test_url, options.test_interval),
    );

    let mut region_groups = Vec::new();
    for module in modules {
        if let ModuleKind::Region(region) = module.kind {
            let names: Vec<String> = nodes
                .iter()
                .filter(|node| node.region == region)
                .map(|node| node.name().to_string())
                .collect();
            if names.is_empty() {
                continue;
            }
            groups.push(
                ProxyGroup::new(module.name, GroupKind::UrlTest, names)
                    .with_health_check(&options.test_url, options.test_interval),
            );
            region_groups.push(module.name.to_string());
        }
    }

    if region_groups.is_empty() {
        warnings.push("未识别到地区分组，服务组将只包含节点选择与自动选择".to_string());
    }

    let mut service_members = vec![SELECT_GROUP.to_string(), AUTO_GROUP.to_string()];
    service_members.extend(region_groups.iter().cloned());

    for module in modules {
        match module.kind {
            ModuleKind::Service => groups.push(ProxyGroup::new(
                module.name,
                GroupKind::Select,
                service_members.clone(),
            )),
            ModuleKind::DirectFirst => groups.push(ProxyGroup::new(
                module.name,
                GroupKind::Select,
                vec!["DIRECT".to_string(), SELECT_GROUP.to_string()],
            )),
            ModuleKind::RejectFirst => groups.push(ProxyGroup::new(
                module.name,
                GroupKind::Select,
                vec![
                    "REJECT".to_string(),
                    "DIRECT".to_string(),
                    SELECT_GROUP.to_string(),
                ],
            )),
            _ => {}
        }
    }

    groups.push(ProxyGroup::new(
        FINAL_GROUP,
        GroupKind::Select,
        vec![SELECT_GROUP.to_string(), "DIRECT".to_string()],
    ));

    groups
}

fn build_custom_groups(
    nodes: &[Node],
    options: &GenerateOptions,
    used_group_names: &mut HashSet<String>,
    warnings: &mut Vec<String>,
) -> Vec<ProxyGroup> {
    let mut groups = Vec::new();

    for spec in &options.custom_groups {
        let members: Vec<String> = nodes
            .iter()
            .filter(|node| spec.filter.matches(node))
            .map(|node| node.name().to_string())
            .collect();
        if members.is_empty() {
            warnings.push(format!("筛选组「{}」没有匹配节点，已跳过", spec.name));
            continue;
        }

        let kind = match spec.kind {
            GroupKind::UrlTest | GroupKind::Fallback | GroupKind::LoadBalance => spec.kind,
            _ => GroupKind::Select,
        };
        let name = unique_group_name(&spec.name, used_group_names);
        let mut group = ProxyGroup::new(name, kind, members);
        if kind != GroupKind::Select {
            group = group.with_health_check(&options.test_url, options.test_interval);
        }
        groups.push(group);
    }

    groups
}

fn build_chains(
    nodes: &[Node],
    options: &GenerateOptions,
    used_group_names: &mut HashSet<String>,
    used_proxy_names: &mut HashSet<String>,
) -> Result<(Vec<Proxy>, Vec<ProxyGroup>)> {
    let mut proxies = Vec::new();
    let mut groups = Vec::new();

    for chain in &options.chains {
        let selectors = if chain.hops.is_empty() {
            vec![chain.entry_node.clone(), chain.landing_node.clone()]
        } else {
            chain.hops.clone()
        };
        if selectors.len() < 2 || selectors.len() > 16 {
            return Err(Error::InvalidConfig(format!(
                "链式代理「{}」需 2–16 跳",
                chain.name
            )));
        }
        let mut route = Vec::new();
        let mut seen = HashSet::new();
        for selector in &selectors {
            let matches: Vec<_> = nodes
                .iter()
                .filter(|n| n.source_key == *selector || n.name() == selector)
                .collect();
            if matches.len() != 1 || !seen.insert(matches[0].source_key.clone()) {
                route.clear();
                break;
            }
            route.push(matches[0]);
        }
        if route.len() != selectors.len() {
            return Err(Error::InvalidConfig(format!(
                "链式代理「{}」节点不存在、禁用、重名或重复",
                chain.name
            )));
        }
        let mut previous = route[0].name().to_string();
        for (index, node) in route.iter().enumerate().skip(1) {
            let mut proxy = node.proxy.clone();
            let name = unique_proxy_name(
                &if chain.hops.is_empty() {
                    format!("{} · {}", chain.name, node.name())
                } else {
                    format!("{} · hop{} · {}", chain.name, index + 1, node.name())
                },
                used_proxy_names,
            );
            proxy.set_name(name.clone());
            proxy.set_dialer_proxy(previous);
            previous = name;
            proxies.push(proxy);
        }
        let group_name = unique_group_name(&chain.name, used_group_names);
        groups.push(ProxyGroup::new(
            group_name,
            GroupKind::Select,
            vec![previous],
        ));
    }

    Ok((proxies, groups))
}

fn unique_group_name(base: &str, used: &mut HashSet<String>) -> String {
    const RESERVED: &[&str] = &[
        "DIRECT",
        "REJECT",
        "REJECT-DROP",
        "PASS",
        "COMPATIBLE",
        "GLOBAL",
    ];

    let base = base.trim();
    let base = if base.is_empty() {
        "自定义组"
    } else {
        base
    };
    let mut candidate = base.to_string();
    let mut index = 2;
    while used.contains(&candidate)
        || RESERVED
            .iter()
            .any(|reserved| reserved.eq_ignore_ascii_case(&candidate))
    {
        candidate = format!("{base} {index}");
        index += 1;
    }
    used.insert(candidate.clone());
    candidate
}

fn unique_proxy_name(base: &str, used: &mut HashSet<String>) -> String {
    let base = base.trim();
    let base = if base.is_empty() {
        "链式节点"
    } else {
        base
    };
    let mut candidate = base.to_string();
    let mut index = 2;
    while used.contains(&candidate) {
        candidate = format!("{base} {index}");
        index += 1;
    }
    used.insert(candidate.clone());
    candidate
}

#[derive(Clone, Debug)]
struct ProviderSpec {
    behavior: catalog::RuleSetBehavior,
    format: &'static str,
    url: String,
}

fn build_rules(
    modules: &[&'static Module],
    options: &GenerateOptions,
    group_names: &HashSet<String>,
    warnings: &mut Vec<String>,
) -> Result<(Vec<Rule>, IndexMap<String, ProviderSpec>)> {
    let mut providers: IndexMap<String, ProviderSpec> = IndexMap::new();
    let mut high_priority = Vec::new();
    let mut low_priority = Vec::new();
    let disabled_rule_sets: HashSet<&str> = options
        .disabled_rule_sets
        .iter()
        .map(String::as_str)
        .collect();

    let mut ordered: Vec<&Module> = modules
        .iter()
        .filter(|module| !module.rule_sets.is_empty())
        .copied()
        .collect();
    let catalog_index = |module: &Module| {
        catalog::MODULES
            .iter()
            .position(|candidate| candidate.id == module.id)
            .unwrap_or(usize::MAX)
    };
    ordered.sort_by_key(|module| (catalog::rule_priority(module), catalog_index(module)));

    for module in ordered {
        for spec in module.rule_sets {
            if disabled_rule_sets.contains(spec.id) {
                continue;
            }
            providers
                .entry(spec.id.to_string())
                .or_insert_with(|| ProviderSpec {
                    behavior: spec.behavior,
                    format: "mrs",
                    url: format!(
                        "{}/{}",
                        options.rule_provider_base_url.trim_end_matches('/'),
                        spec.path
                    ),
                });
            let target = options
                .rule_set_targets
                .get(spec.id)
                .map(String::as_str)
                .unwrap_or(module.name);
            let rule = Rule::rule_set(
                spec.id,
                target,
                spec.no_resolve || spec.behavior == catalog::RuleSetBehavior::IpCidr,
            );
            validate_rule_target(&rule, group_names)?;
            if catalog::rule_priority(module) < 800 {
                high_priority.push(rule);
            } else {
                low_priority.push(rule);
            }
        }
    }

    for binding in &options.custom_rule_sets {
        validate_rule_target(
            &Rule::rule_set(
                binding.id.clone(),
                binding.target.clone(),
                binding.no_resolve,
            ),
            group_names,
        )?;
        let behavior = match binding.behavior.as_str() {
            "ipcidr" => catalog::RuleSetBehavior::IpCidr,
            "classical" => catalog::RuleSetBehavior::Classical,
            _ => catalog::RuleSetBehavior::Domain,
        };
        let format = match binding.format.as_str() {
            "yaml" => "yaml",
            "text" => "text",
            _ => "mrs",
        };
        if behavior == catalog::RuleSetBehavior::Classical && format == "mrs" {
            return Err(Error::InvalidLink(format!(
                "rule-set '{}' cannot use classical behavior with mrs format",
                binding.id
            )));
        }
        providers.entry(binding.id.clone()).or_insert(ProviderSpec {
            behavior,
            format,
            url: binding.url.clone(),
        });
        high_priority.push(Rule::rule_set(
            binding.id.clone(),
            binding.target.clone(),
            binding.no_resolve,
        ));
    }

    let mut custom = Vec::with_capacity(options.custom_rules.len());
    for rule in &options.custom_rules {
        validate_rule_target(rule, group_names)?;
        if rule.kind == "RULE-SET" {
            let provider = rule.payload.as_deref().unwrap_or_default();
            if !providers.contains_key(provider) {
                return Err(Error::UnknownRuleTarget(format!(
                    "rule-set '{provider}' is not registered"
                )));
            }
        }
        custom.push(rule.clone());
    }

    let mut rules = Vec::with_capacity(high_priority.len() + custom.len() + low_priority.len() + 1);
    rules.extend(high_priority);
    rules.extend(custom);
    rules.extend(low_priority);

    if !rules.iter().any(|rule| rule.kind == "MATCH") {
        rules.push(Rule::match_all(FINAL_GROUP));
    }

    let ranks: HashMap<&str, usize> = options
        .rule_order
        .iter()
        .enumerate()
        .map(|(i, key)| (key.as_str(), i))
        .collect();
    if !ranks.is_empty() {
        rules.sort_by_key(|rule| {
            ranks
                .get(rule_identity(rule).as_str())
                .copied()
                .or_else(|| {
                    // Preserve position on a target edit when no exact identity remains.
                    options.rule_order.iter().position(|key| {
                        Rule::parse(key).is_ok_and(|old| {
                            old.kind == rule.kind
                                && old.payload == rule.payload
                                && old.no_resolve == rule.no_resolve
                        })
                    })
                })
                .unwrap_or(usize::MAX)
        });
    }

    let mut seen = HashSet::new();
    let mut matchers: HashMap<(String, Option<String>), String> = HashMap::new();
    rules.retain(|rule| {
        if !seen.insert(rule.to_line()) {
            warnings.push(format!("重复规则已合并：{}", rule.to_line()));
            return false;
        }
        let key = (rule.kind.clone(), rule.payload.clone());
        if let Some(target) = matchers.get(&key) {
            if target != &rule.target {
                warnings.push(format!(
                    "规则目标冲突：{}，先匹配 {}，随后 {}",
                    rule_identity(rule),
                    target,
                    rule.target
                ));
            }
        } else {
            matchers.insert(key, rule.target.clone());
        }
        true
    });
    if let Some(index) = rules.iter().position(|r| r.kind == "MATCH") {
        if index + 1 < rules.len() {
            warnings.push(format!(
                "第 {} 条 MATCH 后有 {} 条规则无法命中，请调整最终顺序",
                index + 1,
                rules.len() - index - 1
            ));
        }
    }
    Ok((rules, providers))
}

pub fn rule_identity(rule: &Rule) -> String {
    rule.to_line()
}

fn validate_rule_target(rule: &Rule, group_names: &HashSet<String>) -> Result<()> {
    const POLICIES: &[&str] = &[
        "DIRECT",
        "REJECT",
        "REJECT-DROP",
        "PASS",
        "COMPATIBLE",
        "GLOBAL",
    ];
    if POLICIES.contains(&rule.target.as_str()) || group_names.contains(&rule.target) {
        Ok(())
    } else {
        Err(Error::UnknownRuleTarget(rule.target.clone()))
    }
}

fn validate_group_references(
    groups: &[ProxyGroup],
    group_names: &HashSet<String>,
    proxy_names: &HashSet<String>,
) -> Result<()> {
    const POLICIES: &[&str] = &[
        "DIRECT",
        "REJECT",
        "REJECT-DROP",
        "PASS",
        "COMPATIBLE",
        "GLOBAL",
    ];
    for group in groups {
        for target in &group.proxies {
            if POLICIES.contains(&target.as_str())
                || group_names.contains(target)
                || proxy_names.contains(target)
            {
                continue;
            }
            return Err(Error::UnknownRuleTarget(format!(
                "group '{}' references unknown target '{target}'",
                group.name
            )));
        }
        if group.proxies.is_empty() {
            return Err(Error::UnknownRuleTarget(format!(
                "group '{}' has no proxies",
                group.name
            )));
        }
    }
    // Explicit group references can form cycles even when every name exists.
    fn visit<'a>(
        name: &'a str,
        groups: &'a [ProxyGroup],
        visiting: &mut HashSet<&'a str>,
        done: &mut HashSet<&'a str>,
    ) -> Result<()> {
        if done.contains(name) {
            return Ok(());
        }
        if !visiting.insert(name) {
            return Err(Error::UnknownRuleTarget(format!(
                "策略组存在循环引用：{name}"
            )));
        }
        if let Some(group) = groups.iter().find(|group| group.name == name) {
            for member in &group.proxies {
                if groups.iter().any(|group| group.name == *member) {
                    visit(member, groups, visiting, done)?;
                }
            }
        }
        visiting.remove(name);
        done.insert(name);
        Ok(())
    }
    let mut visiting = HashSet::new();
    let mut done = HashSet::new();
    for group in groups {
        visit(&group.name, groups, &mut visiting, &mut done)?;
    }
    Ok(())
}

fn build_root(
    proxies: Vec<Value>,
    groups: &[ProxyGroup],
    rules: &[Rule],
    providers: &IndexMap<String, ProviderSpec>,
    options: &GenerateOptions,
) -> Mapping {
    let mut root = Mapping::new();
    insert(&mut root, "mixed-port", number(options.mixed_port as u64));
    insert(&mut root, "allow-lan", Value::Bool(options.allow_lan));
    insert(&mut root, "mode", string("rule"));
    insert(&mut root, "log-level", string("info"));
    insert(&mut root, "unified-delay", Value::Bool(true));
    insert(&mut root, "tcp-concurrent", Value::Bool(true));
    insert(&mut root, "find-process-mode", string("strict"));

    let mut profile = Mapping::new();
    insert(&mut profile, "store-selected", Value::Bool(true));
    insert(&mut profile, "store-fake-ip", Value::Bool(false));
    insert(&mut root, "profile", Value::Mapping(profile));

    insert(&mut root, "dns", dns::build_dns(&options.dns));
    insert(&mut root, "sniffer", sniffer());

    insert(&mut root, "geodata-mode", Value::Bool(true));
    insert(&mut root, "geo-auto-update", Value::Bool(true));
    insert(&mut root, "geodata-loader", string("standard"));
    insert(&mut root, "geo-update-interval", number(24));
    insert(&mut root, "geox-url", geox_url());

    insert(&mut root, "proxies", Value::Sequence(proxies));
    insert(
        &mut root,
        "proxy-groups",
        Value::Sequence(groups.iter().map(ProxyGroup::to_value).collect()),
    );

    if !providers.is_empty() {
        let mut provider_map = Mapping::new();
        for (id, spec) in providers {
            let mut provider = Mapping::new();
            insert(&mut provider, "type", string("http"));
            insert(&mut provider, "behavior", string(spec.behavior.as_str()));
            insert(&mut provider, "format", string(spec.format));
            insert(&mut provider, "url", string(&spec.url));
            insert(&mut provider, "interval", number(86_400));
            provider_map.insert(Value::String(id.clone()), Value::Mapping(provider));
        }
        insert(&mut root, "rule-providers", Value::Mapping(provider_map));
    }

    insert(
        &mut root,
        "rules",
        Value::Sequence(
            rules
                .iter()
                .map(|rule| Value::String(rule.to_line()))
                .collect(),
        ),
    );

    root
}

fn sniffer() -> Value {
    let mut sniffer = Mapping::new();
    insert(&mut sniffer, "enable", Value::Bool(true));
    insert(&mut sniffer, "parse-pure-ip", Value::Bool(true));

    let mut sniff = Mapping::new();

    let mut tls = Mapping::new();
    insert(
        &mut tls,
        "ports",
        Value::Sequence(vec![number(443), number(8443)]),
    );
    insert(&mut sniff, "TLS", Value::Mapping(tls));

    let mut http = Mapping::new();
    insert(
        &mut http,
        "ports",
        Value::Sequence(vec![number(80), string("8080-8880")]),
    );
    insert(&mut http, "override-destination", Value::Bool(true));
    insert(&mut sniff, "HTTP", Value::Mapping(http));

    let mut quic = Mapping::new();
    insert(
        &mut quic,
        "ports",
        Value::Sequence(vec![number(443), number(8443)]),
    );
    insert(&mut sniff, "QUIC", Value::Mapping(quic));

    insert(&mut sniffer, "sniff", Value::Mapping(sniff));
    Value::Mapping(sniffer)
}

fn geox_url() -> Value {
    let mut urls = Mapping::new();
    insert(
        &mut urls,
        "geoip",
        string("https://testingcf.jsdelivr.net/gh/MetaCubeX/meta-rules-dat@release/geoip.dat"),
    );
    insert(
        &mut urls,
        "geosite",
        string("https://testingcf.jsdelivr.net/gh/MetaCubeX/meta-rules-dat@release/geosite.dat"),
    );
    insert(
        &mut urls,
        "mmdb",
        string("https://testingcf.jsdelivr.net/gh/MetaCubeX/meta-rules-dat@release/country.mmdb"),
    );
    insert(
        &mut urls,
        "asn",
        string("https://github.com/xishang0128/geoip/releases/download/latest/GeoLite2-ASN.mmdb"),
    );
    Value::Mapping(urls)
}

fn insert(mapping: &mut Mapping, key: &str, value: Value) {
    mapping.insert(Value::String(key.to_string()), value);
}

fn string(value: &str) -> Value {
    Value::String(value.to_string())
}

fn number(value: u64) -> Value {
    Value::Number(serde_yaml_ng::Number::from(value))
}

#[allow(dead_code)]
fn region_group_name(region: Region) -> &'static str {
    region.label()
}
