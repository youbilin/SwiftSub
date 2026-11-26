use std::collections::HashSet;

use serde_yaml_ng::Value;
use swiftsub_core::Error;
use swiftsub_core::GenerateOptions;
use swiftsub_core::Rule;
use swiftsub_core::catalog::Preset;
use swiftsub_core::generate_profile;
use swiftsub_core::generator::RuleSetBinding;
use swiftsub_core::model::{ChainSpec, CustomGroupSpec, GroupKind, Node, NodeFilter};
use swiftsub_core::normalize::{
    apply_name_template, build_nodes, dedupe_proxies, merge_manual_proxies_yaml,
};
use swiftsub_core::parser::parse_content;
use swiftsub_core::region::Region;

fn sample_nodes() -> Vec<Node> {
    let content = include_str!("fixtures/sample-subscription.yaml");
    let parsed = parse_content(content).expect("fixture should parse");
    build_nodes(parsed.proxies, "fixture", "测试订阅")
}

#[test]
fn deleted_builtin_groups_stay_absent_and_match_falls_back_to_direct() {
    use swiftsub_core::model::PolicyGroupSpec;
    let mut options = GenerateOptions::default();
    for name in ["节点选择", "自动选择", "漏网之鱼"] {
        options.policy_groups.push(PolicyGroupSpec {
            name: name.into(),
            builtin: true,
            disabled: true,
            removed: true,
            ..Default::default()
        });
    }
    let profile = generate_profile(&sample_nodes(), &options).unwrap();
    for name in ["节点选择", "自动选择", "漏网之鱼"] {
        assert!(!profile.groups.iter().any(|group| group.name == name));
    }
    assert_eq!(profile.rules.last().unwrap().to_line(), "MATCH,DIRECT");
    assert!(options.policy_groups.iter().all(|group| group.removed));
}

#[test]
fn disabled_chains_filters_and_nodes_are_dormant_and_enabled_chains_join_all_node_groups() {
    use swiftsub_core::model::PolicyGroupSpec;
    let mut nodes = sample_nodes();
    let mut options = GenerateOptions::default();
    let mut chain = ChainSpec::new(
        "Chain",
        nodes[0].source_key.clone(),
        nodes[0].name(),
        nodes[1].source_key.clone(),
        nodes[1].name(),
    );
    chain.disabled = true;
    options.chains.push(chain);
    let mut filter = CustomGroupSpec::new("Filtered", NodeFilter::default());
    filter.disabled = true;
    options.custom_groups.push(filter);
    options.policy_groups.push(PolicyGroupSpec {
        name: "All".into(),
        members: vec!["Chain".into(), "Filtered".into(), nodes[2].name().into()],
        include_all_nodes: true,
        ..Default::default()
    });
    options
        .custom_rules
        .push(Rule::new("DOMAIN", "chain.example.org", "Chain"));
    options
        .custom_rules
        .push(Rule::new("DOMAIN", "node.example.org", nodes[2].name()));
    nodes[2].enabled = false;
    let profile = generate_profile(&nodes, &options).unwrap();
    assert!(
        !profile
            .groups
            .iter()
            .any(|group| matches!(group.name.as_str(), "Chain" | "Filtered"))
    );
    assert!(!profile.rules.iter().any(|rule| matches!(
        rule.payload.as_deref(),
        Some("chain.example.org" | "node.example.org")
    )));
    options.chains[0].disabled = false;
    let enabled = generate_profile(&nodes, &options).unwrap();
    assert!(
        enabled
            .groups
            .iter()
            .find(|group| group.name == "All")
            .unwrap()
            .proxies
            .iter()
            .any(|name| name == "Chain")
    );
    assert!(
        enabled
            .groups
            .iter()
            .find(|group| group.name == "节点选择")
            .unwrap()
            .proxies
            .iter()
            .any(|name| name == "Chain")
    );
    assert!(options.custom_groups[0].disabled);
}

#[test]
fn dns_defaults_and_invalid_authored_groups_and_providers_do_not_produce_broken_output() {
    let nodes = sample_nodes();
    let mut options = GenerateOptions::default();
    options.dns.domestic.clear();
    options.dns.fake_ip_range = "invalid".into();
    let profile = generate_profile(&nodes, &options).unwrap();
    let root: Value = serde_yaml_ng::from_str(&profile.yaml).unwrap();
    assert_eq!(root["dns"]["fake-ip-range"].as_str(), Some("198.18.0.1/16"));
    assert!(!root["dns"]["nameserver"].as_sequence().unwrap().is_empty());
    options.dns_override = true;
    assert!(
        generate_profile(&nodes, &options)
            .unwrap_err()
            .to_string()
            .contains("Fake IP")
    );
    options = GenerateOptions::default();
    options.test_url = "http://".into();
    assert!(generate_profile(&nodes, &options).is_err());
    options = GenerateOptions::default();
    for name in ["节点选择", "DIRECT", "duplicate,comma", " trailing "] {
        options.custom_groups = vec![CustomGroupSpec::new(name, NodeFilter::default())];
        let error = generate_profile(&nodes, &options).unwrap_err().to_string();
        assert!(error.contains(name), "{error}");
    }
    options.custom_groups = vec![CustomGroupSpec::new(nodes[0].name(), NodeFilter::default())];
    assert!(
        generate_profile(&nodes, &options)
            .unwrap_err()
            .to_string()
            .contains("重复")
    );
    options.custom_groups.clear();
    options.custom_rule_sets = vec![RuleSetBinding {
        id: "invalid-provider".into(),
        name: "Invalid provider".into(),
        url: "https://example.org/rules".into(),
        behavior: "aaa".into(),
        format: "yaml".into(),
        target: "DIRECT".into(),
        no_resolve: false,
    }];
    assert!(
        generate_profile(&nodes, &options)
            .unwrap_err()
            .to_string()
            .contains("invalid-provider")
    );
    options.custom_rule_sets.clear();
    options.custom_rules = vec![Rule::new("aaa", "example.org", "DIRECT")];
    let error = generate_profile(&nodes, &options).unwrap_err().to_string();
    assert!(error.contains("AAA,example.org,DIRECT"), "{error}");
}

#[test]
fn corrupt_manual_cache_is_preserved_instead_of_replaced() {
    let corrupt = "proxies: [unfinished";
    let nodes = sample_nodes()
        .into_iter()
        .map(|node| node.proxy)
        .collect::<Vec<_>>();
    assert!(merge_manual_proxies_yaml(Some(corrupt), nodes).is_err());
}

#[test]
fn edited_rule_sets_generate_inline_content_and_restore_original_sources() {
    let nodes = sample_nodes();
    let mut options = GenerateOptions::default();
    options.rule_set_overrides.insert(
        "openai".into(),
        vec!["+.example.org".into(), "api.example.net".into()],
    );
    let generated = generate_profile(&nodes, &options).unwrap();
    let yaml: Value = serde_yaml_ng::from_str(&generated.yaml).unwrap();
    let provider = &yaml["rule-providers"]["openai"];
    assert_eq!(provider["type"].as_str(), Some("inline"));
    assert_eq!(provider["format"].as_str(), Some("yaml"));
    assert_eq!(provider["payload"].as_sequence().unwrap().len(), 2);
    assert!(provider.get("url").is_none());
    assert!(
        generated
            .rules
            .iter()
            .any(|rule| rule.payload.as_deref() == Some("openai"))
    );
    options.rule_set_overrides.clear();
    let restored: Value =
        serde_yaml_ng::from_str(&generate_profile(&nodes, &options).unwrap().yaml).unwrap();
    assert_eq!(
        restored["rule-providers"]["openai"]["type"].as_str(),
        Some("http")
    );
    assert!(
        restored["rule-providers"]["openai"]["url"]
            .as_str()
            .unwrap()
            .ends_with(".mrs")
    );
    options.rule_set_overrides.insert(
        "openai".into(),
        vec!["DOMAIN-SUFFIX,example.org,REJECT".into()],
    );
    assert!(generate_profile(&nodes, &options).is_err());
}

#[test]
fn dormant_workspace_groups_and_rules_restore_after_roundtrip_with_hundred_nodes() {
    use swiftsub_core::model::PolicyGroupSpec;
    let links = (0..100)
        .map(|i| format!("trojan://test@node-{i}.example.org:443#HK-{i}"))
        .collect::<Vec<_>>()
        .join("\n");
    let nodes = build_nodes(parse_content(&links).unwrap().proxies, "test", "100 nodes");
    let mut options = GenerateOptions {
        policy_groups: vec![
            PolicyGroupSpec {
                id: "outlet-id".into(),
                name: "出口".into(),
                description: "保留编辑说明".into(),
                members: vec!["HK-0".into()],
                ..Default::default()
            },
            PolicyGroupSpec {
                name: "上层".into(),
                members: vec!["出口".into(), "DIRECT".into()],
                ..Default::default()
            },
        ],
        custom_rules: vec![Rule::new("DOMAIN-SUFFIX", "example.org", "出口")],
        custom_rule_sets: vec![RuleSetBinding {
            id: "external".into(),
            name: "外部".into(),
            url: "https://example.org/rules.list".into(),
            behavior: "classical".into(),
            format: "text".into(),
            target: "出口".into(),
            no_resolve: false,
        }],
        rule_set_targets: [("openai".into(), "出口".into())].into(),
        ..Default::default()
    };
    let original = generate_profile(&nodes, &options).unwrap();
    options.policy_groups[0].disabled = true;
    let dormant = generate_profile(&nodes, &options).unwrap();
    assert_eq!(dormant.stats.proxies, 100);
    assert!(!dormant.groups.iter().any(|group| group.name == "出口"));
    assert!(!dormant.rules.iter().any(|rule| rule.target == "出口"));
    assert_eq!(
        dormant
            .groups
            .iter()
            .find(|group| group.name == "上层")
            .unwrap()
            .proxies,
        ["DIRECT"]
    );
    assert_eq!(options.policy_groups[1].members, ["出口", "DIRECT"]);
    let mut restored = options.clone();
    restored.policy_groups =
        serde_json::from_str(&serde_json::to_string(&options.policy_groups).unwrap()).unwrap();
    restored.policy_groups[0].disabled = false;
    assert_eq!(restored.policy_groups[0].id, "outlet-id");
    assert_eq!(restored.policy_groups[0].description, "保留编辑说明");
    assert_eq!(
        generate_profile(&nodes, &restored).unwrap().yaml,
        original.yaml
    );
    restored.disabled_rule_sets.push("external".into());
    assert!(
        !generate_profile(&nodes, &restored)
            .unwrap()
            .yaml
            .contains("RULE-SET,external,")
    );
}

#[test]
fn workspace_health_parameters_and_disabled_builtin_override_generate_correctly() {
    use swiftsub_core::model::PolicyGroupSpec;
    let mut options = GenerateOptions {
        policy_groups: vec![
            PolicyGroupSpec {
                name: "测速出口".into(),
                kind: GroupKind::UrlTest,
                members: vec!["DIRECT".into()],
                tolerance: Some(65),
                test_interval: Some(90),
                ..Default::default()
            },
            PolicyGroupSpec {
                name: "均衡出口".into(),
                kind: GroupKind::LoadBalance,
                members: vec!["DIRECT".into()],
                strategy: Some("round-robin".into()),
                ..Default::default()
            },
            PolicyGroupSpec {
                name: "OpenAI".into(),
                builtin: true,
                members: vec!["DIRECT".into()],
                ..Default::default()
            },
        ],
        ..Default::default()
    };
    // Use the catalog's actual localized service name for the override.
    options.policy_groups[2].name = swiftsub_core::catalog::module("ai").unwrap().name.into();
    let profile = generate_profile(&sample_nodes(), &options).unwrap();
    assert!(profile.yaml.contains("tolerance: 65"));
    assert!(profile.yaml.contains("strategy: round-robin"));
    assert!(profile.yaml.contains("interval: 90"));
    let modules = swiftsub_core::catalog::MODULES
        .iter()
        .filter(|module| {
            module.id != "ai"
                && !matches!(module.kind, swiftsub_core::catalog::ModuleKind::Region(_))
        })
        .map(|module| module.id.to_string())
        .collect();
    options.enabled_modules = Some(modules);
    let profile = generate_profile(&sample_nodes(), &options).unwrap();
    assert!(
        !profile
            .groups
            .iter()
            .any(|group| group.name == options.policy_groups[2].name)
    );
    options.policy_groups[1].strategy = Some("invalid".into());
    assert!(generate_profile(&sample_nodes(), &options).is_err());
}

#[test]
fn generates_a_referentially_valid_standard_profile() {
    let profile = generate_profile(&sample_nodes(), &GenerateOptions::default()).unwrap();
    assert_eq!(profile.stats.proxies, 5);
    assert!(profile.stats.groups >= 10);
    assert!(profile.stats.rules >= 10);
    assert!(profile.stats.rule_sets >= 10);
    assert!(profile.yaml.contains("proxy-groups:"));
    assert!(profile.yaml.contains("rule-providers:"));
    assert!(
        profile
            .yaml
            .contains("https://github.com/MetaCubeX/meta-rules-dat")
    );

    let value: Value = serde_yaml_ng::from_str(&profile.yaml).unwrap();
    let root = value.as_mapping().unwrap();

    let proxy_names: HashSet<String> = root
        .get(Value::String("proxies".into()))
        .and_then(Value::as_sequence)
        .unwrap()
        .iter()
        .filter_map(|proxy| {
            proxy
                .as_mapping()
                .and_then(|map| map.get(Value::String("name".into())))
                .and_then(Value::as_str)
                .map(str::to_string)
        })
        .collect();

    let groups = root
        .get(Value::String("proxy-groups".into()))
        .and_then(Value::as_sequence)
        .unwrap();
    let group_names: HashSet<String> = groups
        .iter()
        .filter_map(|group| {
            group
                .as_mapping()
                .and_then(|map| map.get(Value::String("name".into())))
                .and_then(Value::as_str)
                .map(str::to_string)
        })
        .collect();

    const POLICIES: &[&str] = &[
        "DIRECT",
        "REJECT",
        "REJECT-DROP",
        "PASS",
        "COMPATIBLE",
        "GLOBAL",
    ];
    for group in groups {
        let map = group.as_mapping().unwrap();
        let name = map
            .get(Value::String("name".into()))
            .and_then(Value::as_str)
            .unwrap();
        let members = map
            .get(Value::String("proxies".into()))
            .and_then(Value::as_sequence)
            .unwrap();
        assert!(!members.is_empty(), "group {name} is empty");
        for member in members {
            let member = member.as_str().unwrap();
            assert!(
                POLICIES.contains(&member)
                    || group_names.contains(member)
                    || proxy_names.contains(member),
                "group {name} references unknown target {member}"
            );
        }
    }

    let rules = root
        .get(Value::String("rules".into()))
        .and_then(Value::as_sequence)
        .unwrap();
    for rule in rules {
        let rule = rule.as_str().unwrap();
        let parts: Vec<&str> = rule.split(',').collect();
        let target = if parts[0] == "MATCH" {
            parts[1]
        } else {
            parts.get(2).copied().unwrap_or_default()
        };
        assert!(
            POLICIES.contains(&target) || group_names.contains(target),
            "rule '{rule}' references unknown target '{target}'"
        );
    }
}

#[test]
fn minimal_preset_keeps_china_and_non_china_routing() {
    let options = GenerateOptions {
        preset: Preset::Minimal,
        ..Default::default()
    };
    let profile = generate_profile(&sample_nodes(), &options).unwrap();

    assert!(profile.yaml.contains("广告拦截"));
    assert!(profile.yaml.contains("国内直连"));
    assert!(!profile.yaml.contains("name: AI 服务"));
    assert!(profile.stats.groups < 15);
    assert!(profile.yaml.contains("geolocation-!cn"));
    assert!(profile.yaml.contains("RULE-SET,geolocation-cn,国内直连"));
    assert!(profile.yaml.contains("RULE-SET,geolocation-!cn,全球代理"));
    assert_eq!(profile.rules.last().unwrap().to_line(), "MATCH,漏网之鱼");
    let selected = profile
        .groups
        .iter()
        .find(|g| g.name == "节点选择")
        .unwrap();
    assert!(selected.proxies.contains(&"自动选择".to_string()));
}

#[test]
fn unknown_custom_rule_target_is_rejected() {
    let mut options = GenerateOptions::default();
    options
        .custom_rules
        .push(Rule::new("DOMAIN", "example.com", "不存在的代理组"));
    let error = generate_profile(&sample_nodes(), &options).unwrap_err();
    assert!(matches!(error, Error::UnknownRuleTarget(_)));
}

#[test]
fn empty_node_list_is_rejected() {
    let error = generate_profile(&[], &GenerateOptions::default()).unwrap_err();
    assert!(matches!(error, Error::NoNodes));
}

#[test]
fn duplicate_nodes_are_deduped_by_identity() {
    let content = include_str!("fixtures/sample-subscription.yaml");
    let parsed = parse_content(content).unwrap();
    let mut proxies = parsed.proxies.clone();

    let mut renamed = proxies[0].clone();
    renamed.set_name("完全不同的名字");
    proxies.push(renamed);

    let (unique, removed) = dedupe_proxies(proxies);
    assert_eq!(removed, 1);
    assert_eq!(unique.len(), 5);
}

#[test]
fn invalid_reality_node_is_skipped_with_warning() {
    let mut nodes = sample_nodes();
    let invalid = "proxies:\n  - name: 坏节点\n    type: vless\n    server: bad.example.com\n    port: 443\n    uuid: 11111111-2222-3333-4444-555555555555\n    reality-opts:\n      public-key: not-a-valid-key\n";
    let parsed = parse_content(invalid).unwrap();
    nodes.extend(build_nodes(parsed.proxies, "bad", "坏订阅"));

    let profile = generate_profile(&nodes, &GenerateOptions::default()).unwrap();
    assert!(
        profile
            .warnings
            .iter()
            .any(|warning| warning.contains("REALITY"))
    );
    assert!(!profile.yaml.contains("坏节点"));
}

#[test]
fn custom_filter_groups_and_chains_are_generated() {
    let mut options = GenerateOptions::default();
    options.custom_groups.push(CustomGroupSpec {
        disabled: false,
        name: "美国节点".into(),
        kind: GroupKind::Select,
        filter: NodeFilter {
            regions: vec![Region::Us],
            ..Default::default()
        },
    });
    let nodes = sample_nodes();
    let entry = nodes.iter().find(|node| node.region == Region::Hk).unwrap();
    let landing = nodes.iter().find(|node| node.region == Region::Us).unwrap();
    options.chains.push(ChainSpec::new(
        "美链",
        entry.proxy.identity_key(),
        entry.name(),
        landing.proxy.identity_key(),
        landing.name(),
    ));

    let profile = generate_profile(&nodes, &options).unwrap();
    assert!(profile.yaml.contains("name: 美国节点"));
    assert!(profile.yaml.contains("name: 美链"));
    assert!(profile.yaml.contains("dialer-proxy: 香港 01"));
    assert!(profile.yaml.contains("美链 · 美国 洛杉矶 01"));
    assert!(!profile.yaml.contains("美链 中转"));

    let value: Value = serde_yaml_ng::from_str(&profile.yaml).unwrap();
    let root = value.as_mapping().unwrap();
    let proxies = root
        .get(Value::String("proxies".into()))
        .and_then(Value::as_sequence)
        .unwrap();
    let chain_proxy = proxies.iter().find_map(|proxy| {
        let map = proxy.as_mapping()?;
        let name = map.get(Value::String("name".into()))?.as_str()?;
        name.starts_with("美链 · ").then_some(map)
    });
    let chain_proxy = chain_proxy.expect("chain proxy should exist");
    assert_eq!(
        chain_proxy
            .get(Value::String("dialer-proxy".into()))
            .and_then(Value::as_str),
        Some("香港 01")
    );
}

#[test]
fn disabled_rule_sets_are_skipped() {
    let options = GenerateOptions {
        disabled_rule_sets: vec![
            "openai".to_string(),
            "netflix".to_string(),
            "netflix-ip".to_string(),
        ],
        ..GenerateOptions::default()
    };
    let profile = generate_profile(&sample_nodes(), &options).unwrap();
    assert!(profile.yaml.contains("rule-providers:"));
    assert!(!profile.yaml.contains("openai"));
    assert!(!profile.yaml.contains("netflix"));
    assert!(
        !profile
            .yaml
            .lines()
            .any(|line| line.trim() == "- RULE-SET,openai,AI 服务")
    );
    let full = generate_profile(&sample_nodes(), &GenerateOptions::default()).unwrap();
    assert!(profile.stats.rule_sets < full.stats.rule_sets);
    assert!(profile.stats.rules < full.stats.rules);
}

#[test]
fn custom_rule_sets_generate_remote_providers() {
    let options = GenerateOptions {
        custom_rule_sets: vec![
            RuleSetBinding {
                id: "bm-netflix".to_string(),
                name: "Netflix".to_string(),
                url: "https://example.com/Netflix.yaml".to_string(),
                behavior: "classical".to_string(),
                format: "yaml".to_string(),
                target: "Netflix".to_string(),
                no_resolve: false,
            },
            RuleSetBinding {
                id: "acl-banad".to_string(),
                name: "广告拦截".to_string(),
                url: "https://example.com/BanAD.list".to_string(),
                behavior: "classical".to_string(),
                format: "text".to_string(),
                target: "REJECT".to_string(),
                no_resolve: false,
            },
        ],
        ..GenerateOptions::default()
    };
    let profile = generate_profile(&sample_nodes(), &options).unwrap();
    let value: Value = serde_yaml_ng::from_str(&profile.yaml).unwrap();
    let root = value.as_mapping().unwrap();
    let provider = root
        .get(Value::String("rule-providers".into()))
        .and_then(Value::as_mapping)
        .and_then(|providers| providers.get(Value::String("bm-netflix".into())))
        .and_then(Value::as_mapping)
        .expect("custom provider should exist");
    assert_eq!(
        provider
            .get(Value::String("behavior".into()))
            .and_then(Value::as_str),
        Some("classical")
    );
    assert_eq!(
        provider
            .get(Value::String("format".into()))
            .and_then(Value::as_str),
        Some("yaml")
    );
    assert_eq!(
        provider
            .get(Value::String("url".into()))
            .and_then(Value::as_str),
        Some("https://example.com/Netflix.yaml")
    );
    assert!(profile.yaml.contains("RULE-SET,bm-netflix,Netflix"));
    let acl_provider = root
        .get(Value::String("rule-providers".into()))
        .and_then(Value::as_mapping)
        .and_then(|providers| providers.get(Value::String("acl-banad".into())))
        .and_then(Value::as_mapping)
        .expect("text provider should exist");
    assert_eq!(
        acl_provider
            .get(Value::String("format".into()))
            .and_then(Value::as_str),
        Some("text")
    );
    assert_eq!(
        acl_provider
            .get(Value::String("behavior".into()))
            .and_then(Value::as_str),
        Some("classical")
    );
    assert!(profile.yaml.contains("RULE-SET,acl-banad,REJECT"));
}

#[test]
fn manual_imports_accumulate_and_dedupe() {
    let first = parse_content(
        "ss://YWVzLTI1Ni1nY206cGFzc3dvcmQ=@1.2.3.4:8388#节点A\n\
         trojan://pass@example.com:443#节点B",
    )
    .unwrap();
    let (payload, count) = merge_manual_proxies_yaml(None, first.proxies).unwrap();
    assert_eq!(count, 2);
    assert!(payload.contains("proxies:"));

    let second = parse_content(
        "ss://YWVzLTI1Ni1nY206cGFzc3dvcmQ=@1.2.3.4:8388#节点A重命名\n\
         vless://11111111-2222-3333-4444-555555555555@example.com:443#节点C",
    )
    .unwrap();
    let (payload, count) = merge_manual_proxies_yaml(Some(&payload), second.proxies).unwrap();
    assert_eq!(count, 3);
    assert!(payload.contains("节点B"));
    assert!(payload.contains("节点C"));
}

#[test]
fn name_template_and_unique_names() {
    let mut nodes = sample_nodes();
    apply_name_template(&mut nodes, "{region} {index:02}");

    assert!(nodes.iter().any(|node| node.name() == "香港 01"));
    assert!(nodes.iter().any(|node| node.name() == "日本 02"));

    let names: HashSet<&str> = nodes.iter().map(Node::name).collect();
    assert_eq!(names.len(), nodes.len());
}

#[test]
fn modern_reality_link_fields_survive_manual_storage_and_profile_generation() {
    let link = "vless://11111111-2222-3333-4444-555555555555@example.com:443?security=reality&pbk=jNXHt1yRo0vDuchQlIP6Z0ZvjT3KtzVI-T4E7RoLJS0&sid=1234&type=splithttp&mode=packet-up&fp=chrome&path=%2Ftest&encryption=mlkem768x25519plus.native.0rtt.example#modern";
    for (suffix, expected) in [("", true), ("&support-x25519mlkem768=false", false)] {
        // Insert the query before the name fragment.
        let link = link.replace("#modern", &format!("{suffix}#modern"));
        let parsed = parse_content(&link).unwrap();
        assert!(parsed.warnings.is_empty());
        let (stored_yaml, _) = merge_manual_proxies_yaml(None, parsed.proxies).unwrap();
        let restored = parse_content(&stored_yaml).unwrap();
        let nodes = build_nodes(restored.proxies, "manual", "manual");
        let profile = generate_profile(&nodes, &GenerateOptions::default()).unwrap();
        let yaml: Value = serde_yaml_ng::from_str(&profile.yaml).unwrap();
        let node = &yaml["proxies"][0];
        assert_eq!(
            node["reality-opts"]["support-x25519mlkem768"].as_bool(),
            Some(expected)
        );
        assert_eq!(node["reality-opts"]["short-id"].as_str(), Some("1234"));
        assert_eq!(node["network"].as_str(), Some("xhttp"));
        assert_eq!(node["xhttp-opts"]["mode"].as_str(), Some("packet-up"));
        assert_eq!(node["xhttp-opts"]["path"].as_str(), Some("/test"));
        assert_eq!(
            node["encryption"].as_str(),
            Some("mlkem768x25519plus.native.0rtt.example")
        );
    }
}

#[test]
fn policy_groups_generate_types_members_and_builtin_rule_target_overrides() {
    use swiftsub_core::model::PolicyGroupSpec;
    let kinds = [
        GroupKind::Select,
        GroupKind::UrlTest,
        GroupKind::Fallback,
        GroupKind::LoadBalance,
    ];
    let policy_groups: Vec<_> = kinds
        .iter()
        .enumerate()
        .map(|(i, kind)| PolicyGroupSpec {
            name: format!("专线{i}"),
            kind: *kind,
            members: if *kind == GroupKind::Select {
                vec!["自动选择".into(), "DIRECT".into()]
            } else {
                vec![]
            },
            include_all_nodes: *kind != GroupKind::Select,
            ..Default::default()
        })
        .collect();
    let options = GenerateOptions {
        policy_groups,
        rule_set_targets: [
            ("netflix".into(), "专线0".into()),
            ("netflix-ip".into(), "专线1".into()),
        ]
        .into_iter()
        .collect(),
        ..Default::default()
    };
    let profile = generate_profile(&sample_nodes(), &options).unwrap();
    let yaml: Value = serde_yaml_ng::from_str(&profile.yaml).unwrap();
    let groups = yaml["proxy-groups"].as_sequence().unwrap();
    for (index, kind) in kinds.iter().enumerate() {
        let name = format!("专线{index}");
        let group = groups
            .iter()
            .find(|group| group["name"].as_str() == Some(&name))
            .unwrap();
        assert_eq!(group["type"].as_str(), Some(kind.as_str()));
        if *kind == GroupKind::Select {
            assert_eq!(group["proxies"][0].as_str(), Some("自动选择"));
            assert_eq!(group["proxies"][1].as_str(), Some("DIRECT"));
        } else {
            assert_eq!(
                group["proxies"].as_sequence().unwrap().len(),
                sample_nodes().len()
            );
            assert_eq!(group["interval"].as_u64(), Some(300));
        }
    }
    assert!(profile.yaml.contains("RULE-SET,netflix,专线0"));
    assert!(
        profile
            .yaml
            .contains("RULE-SET,netflix-ip,专线1,no-resolve")
    );
}

#[test]
fn invalid_policy_group_names_members_and_cycles_fail_validation() {
    use swiftsub_core::model::PolicyGroupSpec;
    let group = |name: &str, members: &[&str]| PolicyGroupSpec {
        name: name.into(),
        kind: GroupKind::Select,
        members: members.iter().map(|name| (*name).into()).collect(),
        include_all_nodes: false,
        ..Default::default()
    };
    for groups in [
        vec![group("节点选择", &["DIRECT"])],
        vec![group("DIRECT", &["自动选择"])],
        vec![group("bad,name", &["DIRECT"])],
        vec![group("空组", &[])],
        vec![group("孤立组", &["不存在"])],
        vec![group("重复组", &["DIRECT"]), group("重复组", &["DIRECT"])],
        vec![group("自引用", &["自引用"])],
        vec![group("循环A", &["循环B"]), group("循环B", &["循环A"])],
    ] {
        let options = GenerateOptions {
            policy_groups: groups,
            ..Default::default()
        };
        assert!(
            generate_profile(&sample_nodes(), &options).is_err(),
            "{:?}",
            options.policy_groups
        );
    }
    let options = GenerateOptions {
        policy_groups: vec![group("上游", &["下游"]), group("下游", &["DIRECT"])],
        ..Default::default()
    };
    assert!(generate_profile(&sample_nodes(), &options).is_ok());
}

#[test]
fn builtin_rule_target_must_exist_and_is_never_silently_replaced() {
    let options = GenerateOptions {
        rule_set_targets: [("netflix".into(), "不存在的策略组".into())]
            .into_iter()
            .collect(),
        ..Default::default()
    };
    let error = generate_profile(&sample_nodes(), &options).unwrap_err();
    assert!(error.to_string().contains("不存在的策略组"));
}

#[test]
fn builtin_overrides_preserve_references_and_override_health_checks() {
    use swiftsub_core::model::PolicyGroupSpec;
    let options = GenerateOptions {
        policy_groups: vec![PolicyGroupSpec {
            name: "自动选择".into(),
            builtin: true,
            kind: GroupKind::Fallback,
            include_all_nodes: true,
            test_url: Some("https://example.org/check".into()),
            test_interval: Some(77),
            ..Default::default()
        }],
        ..Default::default()
    };
    let profile = generate_profile(&sample_nodes(), &options).unwrap();
    let group = profile
        .groups
        .iter()
        .find(|g| g.name == "自动选择")
        .unwrap();
    assert_eq!(group.kind, GroupKind::Fallback);
    assert_eq!(group.interval, Some(77));
    assert_eq!(group.url.as_deref(), Some("https://example.org/check"));
    assert_eq!(group.proxies.len(), 5);
    let mut invalid = options.clone();
    invalid.policy_groups[0].name = "不存在".into();
    assert!(generate_profile(&sample_nodes(), &invalid).is_err());
    invalid = options.clone();
    invalid.policy_groups.push(invalid.policy_groups[0].clone());
    assert!(generate_profile(&sample_nodes(), &invalid).is_err());
    invalid = options;
    invalid.policy_groups[0].test_interval = Some(0);
    assert!(generate_profile(&sample_nodes(), &invalid).is_err());
}

#[test]
fn final_order_survives_target_changes_and_reports_unreachable_conflicts() {
    use swiftsub_core::generator::rule_identity;
    let direct = Rule::new("DOMAIN", "example.org", "DIRECT");
    let reject = Rule::new("DOMAIN", "example.org", "REJECT");
    let catchall = Rule::match_all("漏网之鱼");
    let options = GenerateOptions {
        custom_rules: vec![
            direct.clone(),
            direct.clone(),
            reject.clone(),
            catchall.clone(),
        ],
        rule_order: vec![rule_identity(&catchall), rule_identity(&direct)],
        ..Default::default()
    };
    let p = generate_profile(&sample_nodes(), &options).unwrap();
    assert_eq!(p.rules[0].kind, "MATCH");
    assert_eq!(p.rules[1].to_line(), direct.to_line());
    assert_eq!(
        p.rules
            .iter()
            .filter(|r| r.to_line() == direct.to_line())
            .count(),
        1
    );
    assert!(p.warnings.iter().any(|w| w.contains("重复规则")));
    assert!(p.warnings.iter().any(|w| w.contains("目标冲突")));
    assert!(p.warnings.iter().any(|w| w.contains("无法命中")));
    assert_ne!(rule_identity(&direct), rule_identity(&reject));
    let mut changed = options.clone();
    changed.custom_rules = vec![reject.clone()];
    let reordered = generate_profile(&sample_nodes(), &changed).unwrap();
    assert_eq!(reordered.rules[1].to_line(), reject.to_line());
}

#[test]
fn generation_report_is_bound_to_output_body_and_traces_sources() {
    use sha2::{Digest, Sha256};
    let nodes = sample_nodes();
    let profile = generate_profile(&nodes, &GenerateOptions::default()).unwrap();
    let body = profile.yaml.splitn(4, '\n').nth(3).unwrap();
    let expected = format!("{:x}", Sha256::digest(body.as_bytes()));
    assert_eq!(
        profile.trace["body_sha256"].as_str(),
        Some(expected.as_str())
    );
    assert!(profile.yaml.contains(&expected));
    assert_eq!(profile.trace["nodes"][0]["source_id"], "fixture");
    assert_eq!(
        profile.trace["rules"].as_array().unwrap().len(),
        profile.stats.rules
    );
    assert!(Rule::parse("DOMAIN,example.org,DIRECT,unsupported").is_err());
    assert!(Rule::parse("DOMAIN,example.org,DIRECT,no-resolve,extra").is_err());
}

#[test]
fn multi_hop_chain_listeners_dns_and_extra_yaml_are_validated() {
    use swiftsub_core::generator::ListenerSpec;
    let nodes = sample_nodes();
    let hops = nodes.iter().take(3).map(|n| n.source_key.clone()).collect();
    let mut chain = ChainSpec::new("three-hop", "", "", "", "");
    chain.hops = hops;
    let options = GenerateOptions {
        chains: vec![chain],
        listeners: vec![ListenerSpec {
            name: "node-port".into(),
            port: 19001,
            proxy: nodes[0].name().into(),
            kind: "mixed".into(),
            listen: "127.0.0.1".into(),
            udp: false,
        }],
        extra_yaml: "log-level: debug\n".into(),
        ..Default::default()
    };
    let profile = generate_profile(&nodes, &options).unwrap();
    let yaml: Value = serde_yaml_ng::from_str(&profile.yaml).unwrap();
    assert_eq!(yaml["proxies"].as_sequence().unwrap().len(), 7);
    let first_clone = &yaml["proxies"][5];
    let second_clone = &yaml["proxies"][6];
    assert_eq!(first_clone["dialer-proxy"].as_str(), Some(nodes[0].name()));
    assert_eq!(second_clone["dialer-proxy"], first_clone["name"]);
    assert_eq!(yaml["listeners"][0]["type"], "mixed");
    assert_eq!(yaml["log-level"], "debug");
    let mut invalid = options.clone();
    invalid.listeners[0].port = invalid.mixed_port;
    assert!(generate_profile(&nodes, &invalid).is_err());
    invalid = options.clone();
    invalid.listeners[0].proxy = "missing".into();
    assert!(generate_profile(&nodes, &invalid).is_err());
    invalid = options.clone();
    invalid.chains[0].hops[1] = "missing".into();
    assert!(generate_profile(&nodes, &invalid).is_err());
    invalid = options.clone();
    invalid.chains[0].hops[1] = invalid.chains[0].hops[0].clone();
    assert!(generate_profile(&nodes, &invalid).is_err());
    invalid = options;
    invalid.extra_yaml = "rules: []".into();
    assert!(generate_profile(&nodes, &invalid).is_err());
}

#[test]
fn presets_and_legacy_region_selection_do_not_create_region_groups() {
    use swiftsub_core::catalog::{self, ModuleKind};
    let region_names: HashSet<_> = catalog::MODULES
        .iter()
        .filter(|m| matches!(m.kind, ModuleKind::Region(_)))
        .map(|m| m.name)
        .collect();
    for preset in [Preset::Minimal, Preset::Standard, Preset::Full] {
        assert!(
            catalog::modules_for_preset(preset)
                .iter()
                .all(|m| !matches!(m.kind, ModuleKind::Region(_)))
        );
        let options = GenerateOptions {
            preset,
            enabled_modules: Some(catalog::MODULES.iter().map(|m| m.id.to_string()).collect()),
            ..Default::default()
        };
        let profile = generate_profile(&sample_nodes(), &options).unwrap();
        for group in &profile.groups {
            assert!(!region_names.contains(group.name.as_str()));
            assert!(
                group
                    .proxies
                    .iter()
                    .all(|p| !region_names.contains(p.as_str()))
            );
        }
    }
}

#[test]
fn retired_region_targets_migrate_but_authored_groups_survive() {
    use swiftsub_core::model::PolicyGroupSpec;
    let mut options = GenerateOptions::default();
    options.policy_groups.push(PolicyGroupSpec {
        name: "香港".into(),
        builtin: true,
        ..Default::default()
    });
    options
        .custom_rules
        .push(Rule::parse("DOMAIN-SUFFIX,example.test,香港").unwrap());
    let profile = generate_profile(&sample_nodes(), &options).unwrap();
    assert!(profile.yaml.contains("DOMAIN-SUFFIX,example.test,节点选择"));
    assert!(!profile.groups.iter().any(|g| g.name == "香港"));
    options.policy_groups[0].builtin = false;
    options.policy_groups[0].include_all_nodes = true;
    let profile = generate_profile(&sample_nodes(), &options).unwrap();
    assert!(profile.groups.iter().any(|g| g.name == "香港"));
    assert!(profile.yaml.contains("DOMAIN-SUFFIX,example.test,香港"));
}

#[test]
fn mixed_source_broad_rules_follow_service_rules_and_precede_final() {
    use swiftsub_core::generator::RuleSetBinding;
    let mut options = GenerateOptions {
        preset: Preset::Full,
        ..Default::default()
    };
    for (id, target, behavior, no_resolve) in [
        ("loy-cn", "国内直连", "ipcidr", true),
        ("acl-global", "全球代理", "classical", false),
        ("bm-youtube", "YouTube", "classical", false),
    ] {
        options.custom_rule_sets.push(RuleSetBinding {
            id: id.into(),
            name: id.into(),
            url: format!("https://example.com/{id}.yaml"),
            target: target.into(),
            behavior: behavior.into(),
            format: "yaml".into(),
            no_resolve,
        });
    }
    let profile = generate_profile(&sample_nodes(), &options).unwrap();
    let lines: Vec<_> = profile.rules.iter().map(Rule::to_line).collect();
    let at = |id: &str| {
        lines
            .iter()
            .position(|r| r.starts_with(&format!("RULE-SET,{id},")))
            .unwrap()
    };
    assert!(at("google-gemini") < at("google"));
    assert!(at("google-gemini") < at("category-ai-chat-!cn"));
    assert!(at("youtube") < at("google"));
    assert!(at("onedrive") < at("microsoft"));
    assert!(at("xbox") < at("microsoft"));
    assert!(at("bm-youtube") < at("acl-global"));
    assert!(at("acl-global") < at("loy-cn"));
    assert_eq!(lines.last().unwrap(), "MATCH,漏网之鱼");
    for module in swiftsub_core::catalog::modules_for_preset(Preset::Full) {
        assert!(
            profile.groups.iter().any(|g| g.name == module.name),
            "missing {}",
            module.name
        );
        for set in module.rule_sets {
            assert!(profile.yaml.contains(&format!("RULE-SET,{},", set.id)));
        }
    }
}

#[test]
fn picked_rules_and_metadata_are_independent_of_their_original_service_group() {
    use swiftsub_core::generator::RuleSetBinding;
    use swiftsub_core::model::PolicyGroupSpec;
    let mut options = GenerateOptions {
        preset: Preset::Minimal,
        enabled_modules: Some(vec!["select".into(), "auto".into(), "final".into()]),
        policy_groups: vec![PolicyGroupSpec {
            name: "Media".into(),
            members: vec!["DIRECT".into()],
            ..Default::default()
        }],
        rule_set_targets: [("openai".into(), "Media".into())].into(),
        test_tolerance: 120,
        ..Default::default()
    };
    let generated = generate_profile(&sample_nodes(), &options).unwrap();
    assert!(
        generated
            .rules
            .iter()
            .any(|rule| rule.to_line() == "RULE-SET,openai,Media")
    );
    assert!(!generated.groups.iter().any(|group| group.name == "AI 服务"));
    assert!(
        !generated
            .rules
            .iter()
            .any(|rule| rule.payload.as_deref() == Some("category-ai-chat-!cn"))
    );
    assert_eq!(
        generated
            .groups
            .iter()
            .find(|group| group.name == "自动选择")
            .unwrap()
            .tolerance,
        Some(120)
    );
    options.custom_rule_sets.push(RuleSetBinding {
        id: "openai".into(),
        name: "Edited source".into(),
        url: "https://example.org/edited.yaml".into(),
        behavior: "classical".into(),
        format: "yaml".into(),
        target: "Media".into(),
        no_resolve: false,
    });
    let generated = generate_profile(&sample_nodes(), &options).unwrap();
    assert_eq!(
        generated
            .rules
            .iter()
            .filter(|rule| rule.payload.as_deref() == Some("openai"))
            .count(),
        1
    );
    assert!(generated.yaml.contains("https://example.org/edited.yaml"));
    options.policy_groups[0].disabled = true;
    assert!(
        !generate_profile(&sample_nodes(), &options)
            .unwrap()
            .yaml
            .contains("RULE-SET,openai,")
    );
    options.policy_groups[0].disabled = false;
    assert!(
        generate_profile(&sample_nodes(), &options)
            .unwrap()
            .yaml
            .contains("RULE-SET,openai,Media")
    );
}
