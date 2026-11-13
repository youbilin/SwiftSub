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
fn minimal_preset_has_no_service_groups() {
    let options = GenerateOptions {
        preset: Preset::Minimal,
        ..Default::default()
    };
    let profile = generate_profile(&sample_nodes(), &options).unwrap();

    assert!(profile.yaml.contains("广告拦截"));
    assert!(profile.yaml.contains("国内直连"));
    assert!(!profile.yaml.contains("name: AI 服务"));
    assert!(profile.stats.groups < 15);
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
