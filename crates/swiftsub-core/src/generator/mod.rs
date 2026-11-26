mod dns;
mod mrs;
mod rule_content;
mod rule_validation;
pub mod singbox;
pub use mrs::decode_mrs_payload;
pub use rule_content::parse_rule_set_payload;
pub use rule_validation::validate_rule_definitions;
pub use singbox::{generate_singbox_json, proxy_to_singbox_outbound};

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

#[derive(Clone, Debug, Serialize)]
pub struct GenerateOptions {
    pub preset: Preset,
    pub enabled_modules: Option<Vec<String>>,
    pub test_url: String,
    pub test_interval: u32,
    pub test_tolerance: u32,
    pub mixed_port: u16,
    pub allow_lan: bool,
    pub rule_provider_base_url: String,
    pub disabled_rule_sets: Vec<String>,
    pub custom_rule_sets: Vec<RuleSetBinding>,
    pub custom_rules: Vec<Rule>,
    pub custom_groups: Vec<CustomGroupSpec>,
    pub policy_groups: Vec<PolicyGroupSpec>,
    pub group_order: Vec<String>,
    pub rule_set_targets: HashMap<String, String>,
    pub chains: Vec<ChainSpec>,
    pub dns: DnsOptions,
    pub dns_override: bool,
    pub rule_set_overrides: HashMap<String, Vec<String>>,
    pub rule_order: Vec<String>,
    pub listeners: Vec<ListenerSpec>,
    pub extra_yaml: String,
}

impl GenerateOptions {
    pub fn resolved_group_name<'a>(&'a self, name: &'a str) -> &'a str {
        self.policy_groups
            .iter()
            .find(|group| group.builtin_name() == Some(name))
            .map(|group| group.name.as_str())
            .unwrap_or(name)
    }

    /// Migrate targets of retired built-in region groups; explicitly authored groups remain available.
    pub fn remove_legacy_region_groups(&mut self) {
        let authored: HashSet<_> = self
            .policy_groups
            .iter()
            .filter(|g| !g.builtin)
            .map(|g| g.name.clone())
            .chain(self.custom_groups.iter().map(|g| g.name.clone()))
            .chain(self.chains.iter().map(|g| g.name.clone()))
            .collect();
        let retired: HashSet<_> = catalog::MODULES
            .iter()
            .filter(|m| matches!(m.kind, ModuleKind::Region(_)))
            .map(|m| m.name)
            .filter(|name| !authored.contains(*name))
            .collect();
        if let Some(modules) = &mut self.enabled_modules {
            modules.retain(|id| {
                !catalog::module(id).is_some_and(|m| matches!(m.kind, ModuleKind::Region(_)))
            });
        }
        self.policy_groups
            .retain(|g| !g.builtin_name().is_some_and(|name| retired.contains(name)));
        let redirect = |target: &mut String| {
            if retired.contains(target.as_str()) {
                *target = SELECT_GROUP.into();
            }
        };
        for group in &mut self.policy_groups {
            for member in &mut group.members {
                redirect(member);
            }
            let mut seen = HashSet::new();
            group.members.retain(|m| seen.insert(m.clone()));
        }
        for binding in &mut self.custom_rule_sets {
            redirect(&mut binding.target);
        }
        for rule in &mut self.custom_rules {
            redirect(&mut rule.target);
        }
        for target in self.rule_set_targets.values_mut() {
            redirect(target);
        }
        for listener in &mut self.listeners {
            redirect(&mut listener.proxy);
        }
    }
}

impl Default for GenerateOptions {
    fn default() -> Self {
        Self {
            preset: Preset::Standard,
            enabled_modules: None,
            test_url: HEALTH_CHECK_URL.to_string(),
            test_interval: 300,
            test_tolerance: 50,
            mixed_port: 7897,
            allow_lan: false,
            rule_provider_base_url:
                "https://github.com/MetaCubeX/meta-rules-dat/raw/refs/heads/meta/geo".to_string(),
            disabled_rule_sets: Vec::new(),
            custom_rule_sets: Vec::new(),
            custom_rules: Vec::new(),
            custom_groups: Vec::new(),
            policy_groups: Vec::new(),
            group_order: Vec::new(),
            rule_set_targets: HashMap::new(),
            chains: Vec::new(),
            dns: DnsOptions::default(),
            dns_override: false,
            rule_set_overrides: HashMap::new(),
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
    if !valid_http_url(&options.test_url) || options.test_interval == 0 {
        return Err(Error::InvalidConfig(
            "全局测速地址须为有效 HTTP / HTTPS 链接，间隔必须大于零".into(),
        ));
    }
    let mut normalized = options.clone();
    normalized.remove_legacy_region_groups();
    let aliases: HashMap<String, String> = normalized
        .policy_groups
        .iter()
        .filter_map(|group| {
            group
                .builtin_name()
                .filter(|name| *name != group.name)
                .map(|name| (name.to_string(), group.name.clone()))
        })
        .collect();
    if let Some(name) = aliases.values().find(|name| {
        catalog::MODULES
            .iter()
            .any(|module| module.name == name.as_str())
    }) {
        return Err(Error::InvalidConfig(format!(
            "策略组名称与内置组重名：{name}"
        )));
    }
    for name in nodes
        .iter()
        .map(Node::name)
        .chain(
            normalized
                .policy_groups
                .iter()
                .filter(|group| !group.builtin)
                .map(|group| group.name.as_str()),
        )
        .chain(
            normalized
                .custom_groups
                .iter()
                .map(|group| group.name.as_str()),
        )
        .chain(normalized.chains.iter().map(|chain| chain.name.as_str()))
    {
        if aliases.contains_key(name) {
            return Err(Error::InvalidConfig(format!(
                "名称与内置组原名称重名：{name}"
            )));
        }
    }
    let resolve_alias = |target: &mut String| {
        if let Some(name) = aliases.get(target) {
            *target = name.clone();
        }
    };
    for group in &mut normalized.policy_groups {
        for member in &mut group.members {
            resolve_alias(member);
        }
    }
    for binding in &mut normalized.custom_rule_sets {
        resolve_alias(&mut binding.target);
    }
    for rule in &mut normalized.custom_rules {
        resolve_alias(&mut rule.target);
    }
    for target in normalized.rule_set_targets.values_mut() {
        resolve_alias(target);
    }
    for listener in &mut normalized.listeners {
        resolve_alias(&mut listener.proxy);
    }
    for name in &mut normalized.group_order {
        resolve_alias(name);
    }
    for line in &mut normalized.rule_order {
        if let Ok(mut rule) = Rule::parse(line) {
            resolve_alias(&mut rule.target);
            *line = rule.to_line();
        }
    }
    if normalized.dns_override {
        normalized.dns.validate()?;
    } else {
        normalized.dns = DnsOptions::default();
    }
    let active_modules = resolve_enabled_modules(&normalized);
    let mut dormant: HashSet<_> = normalized
        .policy_groups
        .iter()
        .filter(|group| group.disabled || group.removed)
        .map(|group| group.name.clone())
        .collect();
    dormant.extend(
        normalized
            .custom_groups
            .iter()
            .filter(|group| group.disabled)
            .map(|group| group.name.clone()),
    );
    dormant.extend(
        normalized
            .chains
            .iter()
            .filter(|chain| chain.disabled)
            .map(|chain| chain.name.clone()),
    );
    dormant.extend(
        nodes
            .iter()
            .filter(|node| !node.enabled)
            .map(|node| node.name().to_string()),
    );
    dormant.extend(
        catalog::MODULES
            .iter()
            .filter(|module| {
                !matches!(module.kind, ModuleKind::Region(_))
                    && !active_modules.iter().any(|active| active.id == module.id)
            })
            .map(|module| normalized.resolved_group_name(module.name).to_string()),
    );
    // An authored group or node can use a currently inactive catalog label.
    for name in normalized
        .policy_groups
        .iter()
        .filter(|group| !group.builtin && !group.disabled && !group.removed)
        .map(|group| group.name.as_str())
        .chain(
            normalized
                .custom_groups
                .iter()
                .filter(|group| !group.disabled)
                .map(|group| group.name.as_str()),
        )
        .chain(
            normalized
                .chains
                .iter()
                .filter(|chain| !chain.disabled)
                .map(|chain| chain.name.as_str()),
        )
        .chain(nodes.iter().filter(|node| node.enabled).map(Node::name))
    {
        dormant.remove(name);
    }
    normalized
        .policy_groups
        .retain(|group| !group.disabled && !group.removed && !dormant.contains(&group.name));
    normalized.custom_groups.retain(|group| !group.disabled);
    normalized.chains.retain(|chain| !chain.disabled);
    for group in &mut normalized.policy_groups {
        for member in &mut group.members {
            if dormant.contains(member) {
                *member = "DIRECT".into();
            }
        }
    }
    normalized.custom_rule_sets.retain(|binding| {
        !dormant.contains(&binding.target) && !normalized.disabled_rule_sets.contains(&binding.id)
    });
    for (id, target) in &normalized.rule_set_targets {
        if dormant.contains(target) && !normalized.disabled_rule_sets.contains(id) {
            normalized.disabled_rule_sets.push(id.clone());
        }
    }
    normalized
        .custom_rules
        .retain(|rule| !dormant.contains(&rule.target));
    for listener in &mut normalized.listeners {
        if dormant.contains(&listener.proxy) {
            listener.proxy = "DIRECT".into();
        }
    }
    let options = &normalized;
    let mut warnings = Vec::new();
    let mut nodes = enabled_nodes(nodes);
    for node in &mut nodes {
        if let Some(target) = node.proxy.get_str("dialer-proxy")
            && let Some(name) = aliases.get(target)
        {
            node.proxy.set_dialer_proxy(name);
        }
        if node
            .proxy
            .get_str("dialer-proxy")
            .is_some_and(|target| dormant.contains(target))
        {
            node.proxy.set_dialer_proxy("DIRECT");
        }
    }
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

    let modules = active_modules;
    let mut groups = build_groups(&nodes, &modules, options, &mut warnings);
    let builtin_names: HashSet<String> = groups.iter().map(|g| g.name.clone()).collect();
    let mut output_names = HashSet::new();
    for group in &mut groups {
        group.name = options.resolved_group_name(&group.name).to_string();
        if !output_names.insert(group.name.clone()) {
            return Err(Error::InvalidConfig(format!(
                "策略组名称重复：{}",
                group.name
            )));
        }
    }
    let final_group = groups
        .iter()
        .position(|group| group.name == options.resolved_group_name(FINAL_GROUP))
        .map(|index| groups.remove(index));

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
    )?);
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
            || (!spec.builtin && (used_group_names.contains(name) || aliases.contains_key(name)))
            || (spec.builtin
                && !spec.builtin_name().is_some_and(|origin| {
                    builtin_names.contains(origin) && overridden.insert(origin.to_string())
                }))
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
            members.extend(options.chains.iter().map(|chain| chain.name.clone()));
        }
        let mut seen = HashSet::new();
        members.retain(|member| seen.insert(member.clone()));
        let mut group = ProxyGroup::new(name, spec.kind, members);
        if spec.kind == GroupKind::UrlTest {
            group.tolerance = spec.tolerance;
        }
        if spec.kind == GroupKind::LoadBalance {
            let strategy = spec.strategy.as_deref().unwrap_or("consistent-hashing");
            if !["consistent-hashing", "round-robin", "sticky-sessions"].contains(&strategy) {
                return Err(Error::InvalidLink(format!("负载均衡策略无效：{name}")));
            }
            group.strategy = Some(strategy.into());
        }
        if spec.kind != GroupKind::Select {
            let url = spec.test_url.as_deref().unwrap_or(&options.test_url);
            let interval = spec.test_interval.unwrap_or(options.test_interval);
            if !valid_http_url(url) || interval == 0 {
                return Err(Error::UnknownRuleTarget(format!(
                    "策略组「{name}」测速参数无效"
                )));
            }
            group = group.with_health_check(url, interval);
        }
        if spec.builtin {
            let Some(index) = groups.iter().position(|g| g.name == name) else {
                return Err(Error::UnknownRuleTarget(format!(
                    "内置策略组不存在：{name}"
                )));
            };
            groups[index] = group;
        } else {
            groups.push(group);
        }
    }

    for group in &mut groups {
        for member in &mut group.proxies {
            resolve_alias(member);
        }
    }
    let positions: HashMap<&str, usize> = options
        .group_order
        .iter()
        .enumerate()
        .map(|(index, name)| (name.as_str(), index))
        .collect();
    groups.sort_by_key(|group| {
        positions
            .get(group.name.as_str())
            .copied()
            .unwrap_or(usize::MAX)
    });
    let group_names: HashSet<String> = groups.iter().map(|group| group.name.clone()).collect();
    let proxy_names = used_proxy_names;

    if let Some(name) = group_names.intersection(&proxy_names).next() {
        return Err(Error::InvalidConfig(format!(
            "节点与策略组名称重复：{name}，请修改节点或自定义组名称"
        )));
    }
    let routing_targets: HashSet<_> = group_names.union(&proxy_names).cloned().collect();
    let (rules, providers) = build_rules(&modules, options, &routing_targets, &mut warnings)?;
    for group in &mut groups {
        if group.kind == GroupKind::UrlTest && group.tolerance.is_none() {
            group.tolerance = Some(options.test_tolerance);
        }
    }
    validate_group_references(&groups, &group_names, &proxy_names)?;

    let mut proxies: Vec<Value> = nodes.iter().map(|node| node.proxy.to_value()).collect();
    proxies.extend(chain_proxies.iter().map(Proxy::to_value));
    let mut root = build_root(proxies, &groups, &rules, &providers, options);
    if options.mixed_port == 0
        || options.dns_override
            && (options.dns.domestic.is_empty() || options.dns.foreign.is_empty())
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
            if let Some(map) = value.as_mapping_mut()
                && let Some(kind) = map.remove(Value::String("kind".into()))
            {
                map.insert(Value::String("type".into()), kind);
            }
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

    validate_rule_definitions(
        &rules,
        root.get(Value::String("sub-rules".into())),
        &routing_targets,
        &providers.keys().cloned().collect(),
    )?;

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
        "# Generated by SwiftSub {}\n# Body SHA-256 at generation: {digest}\n# Saved copies can be edited in My Configurations.\n{}",
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
    for spec in options
        .policy_groups
        .iter()
        .filter(|spec| spec.builtin && (spec.disabled || spec.removed))
    {
        if let Some(module) = catalog::MODULES
            .iter()
            .find(|module| spec.builtin_name() == Some(module.name))
        {
            ids.remove(module.id);
        }
    }

    catalog::MODULES
        .iter()
        .filter(|module| ids.contains(module.id) && !matches!(module.kind, ModuleKind::Region(_)))
        .collect()
}

fn build_groups(
    nodes: &[Node],
    modules: &[&'static Module],
    options: &GenerateOptions,
    _warnings: &mut Vec<String>,
) -> Vec<ProxyGroup> {
    let node_names: Vec<String> = nodes.iter().map(|node| node.name().to_string()).collect();
    let active = |id: &str| modules.iter().any(|module| module.id == id);
    let available_members = |members: Vec<String>| {
        members
            .into_iter()
            .filter(|name| {
                (name != SELECT_GROUP || active("select")) && (name != AUTO_GROUP || active("auto"))
            })
            .collect::<Vec<_>>()
    };

    let mut groups = Vec::new();
    if active("select") {
        groups.push(ProxyGroup::new(
            SELECT_GROUP,
            GroupKind::Select,
            available_members(
                std::iter::once(AUTO_GROUP.to_string())
                    .chain(node_names.clone())
                    .chain(options.chains.iter().map(|chain| chain.name.clone()))
                    .chain(std::iter::once("DIRECT".to_string()))
                    .collect(),
            ),
        ));
    }
    if active("auto") {
        groups.push(
            ProxyGroup::new(
                AUTO_GROUP,
                GroupKind::UrlTest,
                node_names
                    .clone()
                    .into_iter()
                    .chain(options.chains.iter().map(|chain| chain.name.clone()))
                    .collect(),
            )
            .with_health_check(&options.test_url, options.test_interval),
        );
    }

    let mut service_members = available_members(vec![
        SELECT_GROUP.to_string(),
        AUTO_GROUP.to_string(),
        "DIRECT".to_string(),
    ]);
    service_members.extend(node_names.clone());
    service_members.extend(options.chains.iter().map(|chain| chain.name.clone()));

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
                available_members(vec!["DIRECT".to_string(), SELECT_GROUP.to_string()]),
            )),
            ModuleKind::RejectFirst => groups.push(ProxyGroup::new(
                module.name,
                GroupKind::Select,
                available_members(vec![
                    "REJECT".to_string(),
                    "DIRECT".to_string(),
                    SELECT_GROUP.to_string(),
                ]),
            )),
            _ => {}
        }
    }

    if active("final") {
        groups.push(ProxyGroup::new(
            FINAL_GROUP,
            GroupKind::Select,
            service_members,
        ));
    }

    groups
}

fn build_custom_groups(
    nodes: &[Node],
    options: &GenerateOptions,
    used_group_names: &mut HashSet<String>,
    warnings: &mut Vec<String>,
) -> Result<Vec<ProxyGroup>> {
    let mut groups = Vec::new();

    for spec in &options.custom_groups {
        reserve_authored_group_name(&spec.name, used_group_names)?;
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
        let mut group = ProxyGroup::new(&spec.name, kind, members);
        if kind != GroupKind::Select {
            group = group.with_health_check(&options.test_url, options.test_interval);
        }
        groups.push(group);
    }

    Ok(groups)
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
        reserve_authored_group_name(&chain.name, used_group_names)?;
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
        groups.push(ProxyGroup::new(
            &chain.name,
            GroupKind::Select,
            vec![previous],
        ));
    }

    Ok((proxies, groups))
}

fn reserve_authored_group_name(base: &str, used: &mut HashSet<String>) -> Result<()> {
    const RESERVED: &[&str] = &[
        "DIRECT",
        "REJECT",
        "REJECT-DROP",
        "PASS",
        "COMPATIBLE",
        "GLOBAL",
    ];

    if base.is_empty()
        || base.trim() != base
        || base.contains([',', '\n', '\r'])
        || used.contains(base)
        || RESERVED
            .iter()
            .any(|reserved| reserved.eq_ignore_ascii_case(base))
    {
        return Err(Error::InvalidConfig(format!(
            "策略组名称无效或重复：{base}"
        )));
    }
    used.insert(base.to_string());
    Ok(())
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
    payload: Option<Vec<String>>,
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

    let active_ids: HashSet<_> = modules.iter().map(|module| module.id).collect();
    let custom_ids: HashSet<_> = options
        .custom_rule_sets
        .iter()
        .map(|binding| binding.id.as_str())
        .collect();
    let mut ordered: Vec<&Module> = catalog::MODULES
        .iter()
        .filter(|module| {
            !module.rule_sets.is_empty()
                && (active_ids.contains(module.id)
                    || module.rule_sets.iter().any(|spec| {
                        options.rule_set_targets.get(spec.id).is_some_and(|target| {
                            target != module.name && group_names.contains(target)
                        })
                    }))
        })
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
            if disabled_rule_sets.contains(spec.id)
                || custom_ids.contains(spec.id)
                || (!active_ids.contains(module.id)
                    && !options.rule_set_targets.get(spec.id).is_some_and(|target| {
                        target != module.name && group_names.contains(target)
                    }))
            {
                continue;
            }
            if let Some(payload) = options.rule_set_overrides.get(spec.id) {
                parse_rule_set_payload(&payload.join("\n"), spec.behavior.as_str()).map_err(
                    |error| Error::InvalidConfig(format!("规则集「{}」：{error}", spec.id)),
                )?;
            } else if !valid_http_url(&format!(
                "{}/{}",
                options.rule_provider_base_url.trim_end_matches('/'),
                spec.path
            )) {
                return Err(Error::InvalidConfig(format!(
                    "规则集「{}」来源链接无效",
                    spec.id
                )));
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
                    payload: options.rule_set_overrides.get(spec.id).cloned(),
                });
            let target = options
                .rule_set_targets
                .get(spec.id)
                .map(String::as_str)
                .unwrap_or_else(|| options.resolved_group_name(module.name));
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

    let mut binding_ids = HashSet::new();
    for binding in &options.custom_rule_sets {
        if binding.id.trim().is_empty()
            || binding.id != binding.id.trim()
            || binding.id.contains([',', '/', '\\', '\n', '\r'])
            || !binding_ids.insert(&binding.id)
        {
            return Err(Error::InvalidConfig(format!(
                "规则集 ID 无效或重复：{}",
                binding.id
            )));
        }
        if let Some(payload) = options.rule_set_overrides.get(&binding.id) {
            parse_rule_set_payload(&payload.join("\n"), &binding.behavior).map_err(|error| {
                Error::InvalidConfig(format!(
                    "规则集「{}」（{}）：{error}",
                    binding.name, binding.id
                ))
            })?;
        } else if !valid_http_url(&binding.url) {
            return Err(Error::InvalidConfig(format!(
                "规则集「{}」（{}）来源链接无效",
                binding.name, binding.id
            )));
        }
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
            "domain" => catalog::RuleSetBehavior::Domain,
            _ => {
                return Err(Error::InvalidConfig(format!(
                    "规则集「{}」（{}）内容类型无效：{}",
                    binding.name, binding.id, binding.behavior
                )));
            }
        };
        let format = match binding.format.as_str() {
            "yaml" => "yaml",
            "text" => "text",
            "mrs" => "mrs",
            _ => {
                return Err(Error::InvalidConfig(format!(
                    "规则集「{}」（{}）文件格式无效：{}",
                    binding.name, binding.id, binding.format
                )));
            }
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
            payload: options.rule_set_overrides.get(&binding.id).cloned(),
        });
        let rule = Rule::rule_set(
            binding.id.clone(),
            binding.target.clone(),
            binding.no_resolve,
        );
        let priority = catalog::MODULES
            .iter()
            .find(|m| options.resolved_group_name(m.name) == binding.target)
            .map(catalog::rule_priority)
            .unwrap_or(400);
        if priority >= 800 {
            low_priority.push(rule);
        } else {
            high_priority.push(rule);
        }
    }

    let mut custom = Vec::with_capacity(options.custom_rules.len());
    for (index, rule) in options.custom_rules.iter().enumerate() {
        let line = rule.to_line();
        Rule::parse(&line).map_err(|error| {
            Error::InvalidConfig(format!("自定义规则第 {} 条「{line}」：{error}", index + 1))
        })?;
        validate_rule_target(rule, group_names)?;
        for provider in rule.referenced_rule_sets() {
            if !providers.contains_key(&provider) {
                return Err(Error::UnknownRuleTarget(format!(
                    "rule-set '{provider}' is not registered"
                )));
            }
        }
        custom.push(rule.clone());
    }

    let priority = |rule: &Rule| {
        let rank = catalog::MODULES
            .iter()
            .find(|m| options.resolved_group_name(m.name) == rule.target)
            .map(catalog::rule_priority)
            .unwrap_or(400);
        if rank == 800 && rule.no_resolve {
            900
        } else {
            rank
        }
    };
    high_priority.sort_by_key(priority);
    low_priority.sort_by_key(priority);
    let mut rules = Vec::with_capacity(high_priority.len() + custom.len() + low_priority.len() + 1);
    rules.extend(high_priority);
    rules.extend(custom);
    rules.extend(low_priority);

    if !rules.iter().any(|rule| rule.kind == "MATCH") {
        let final_name = options.resolved_group_name(FINAL_GROUP);
        let select_name = options.resolved_group_name(SELECT_GROUP);
        rules.push(Rule::match_all(if group_names.contains(final_name) {
            final_name
        } else if group_names.contains(select_name) {
            select_name
        } else {
            "DIRECT"
        }));
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
                                && old.source_ip == rule.source_ip
                        })
                    })
                })
                .unwrap_or(usize::MAX)
        });
    }

    let mut seen = HashSet::new();
    let mut matchers: HashMap<(String, Option<String>, bool), String> = HashMap::new();
    rules.retain(|rule| {
        if !seen.insert(rule.to_line()) {
            warnings.push(format!("重复规则已合并：{}", rule.to_line()));
            return false;
        }
        let key = (rule.kind.clone(), rule.payload.clone(), rule.source_ip);
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
    if let Some(index) = rules.iter().position(|r| r.kind == "MATCH")
        && index + 1 < rules.len()
    {
        warnings.push(format!(
            "第 {} 条 MATCH 后有 {} 条规则无法命中，请调整最终顺序",
            index + 1,
            rules.len() - index - 1
        ));
    }
    Ok((rules, providers))
}

pub fn rule_identity(rule: &Rule) -> String {
    rule.to_line()
}

fn valid_http_url(value: &str) -> bool {
    url::Url::parse(value)
        .is_ok_and(|url| matches!(url.scheme(), "http" | "https") && url.host_str().is_some())
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
    if rule.kind == "SUB-RULE"
        || POLICIES.contains(&rule.target.as_str())
        || group_names.contains(&rule.target)
    {
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
    insert(
        &mut root,
        "hosts",
        Value::Mapping(options.dns.hosts.clone()),
    );
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
            insert(&mut provider, "behavior", string(spec.behavior.as_str()));
            if let Some(payload) = &spec.payload {
                insert(&mut provider, "type", string("inline"));
                insert(&mut provider, "format", string("yaml"));
                insert(
                    &mut provider,
                    "payload",
                    Value::Sequence(payload.iter().map(|s| string(s)).collect()),
                );
            } else {
                insert(&mut provider, "type", string("http"));
                insert(&mut provider, "format", string(spec.format));
                insert(&mut provider, "url", string(&spec.url));
                insert(&mut provider, "interval", number(86_400));
            }
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
