use std::collections::HashSet;

use serde_yaml_ng::Value;
use swiftsub_core::catalog::{self, Preset};
use swiftsub_core::generator::{ListenerSpec, RuleSetBinding};
use swiftsub_core::model::{
    ChainSpec, CustomGroupSpec, GroupKind, Node, NodeFilter, PolicyGroupSpec,
};
use swiftsub_core::normalize::build_nodes;
use swiftsub_core::parser::parse_content;
use swiftsub_core::{GenerateOptions, Rule, generate_profile};

fn nodes() -> Vec<Node> {
    let parsed = parse_content(include_str!("fixtures/sample-subscription.yaml")).unwrap();
    build_nodes(parsed.proxies, "fixture", "测试订阅")
}

fn renamed_spec(origin: &str, name: &str, kind: GroupKind, members: &[&str]) -> PolicyGroupSpec {
    PolicyGroupSpec {
        id: format!("builtin:{origin}"),
        name: name.into(),
        kind,
        members: members.iter().map(|name| (*name).into()).collect(),
        builtin: true,
        ..Default::default()
    }
}

#[test]
fn builtin_rename_updates_default_and_authored_references_without_changing_rule_priority() {
    let ai = catalog::module("ai").unwrap().name;
    let mut options = GenerateOptions {
        enabled_modules: Some(vec!["ai".into(), "telegram".into()]),
        ..Default::default()
    };
    options.policy_groups.push(PolicyGroupSpec {
        name: "工作".into(),
        members: vec!["节点选择".into(), ai.into()],
        ..Default::default()
    });
    options
        .custom_rules
        .push(Rule::new("DOMAIN", "work.example", "节点选择"));
    options
        .rule_order
        .push("DOMAIN,work.example,节点选择".into());
    options.custom_rule_sets.push(RuleSetBinding {
        id: "work-domains".into(),
        name: "工作域名".into(),
        url: "https://example.org/work.yaml".into(),
        behavior: "domain".into(),
        format: "yaml".into(),
        target: ai.into(),
        no_resolve: false,
    });
    options.rule_set_targets.insert("openai".into(), ai.into());
    options.listeners.push(ListenerSpec {
        name: "work-listener".into(),
        port: 7898,
        proxy: "节点选择".into(),
        kind: "mixed".into(),
        listen: "127.0.0.1".into(),
        udp: true,
    });
    let nodes = nodes();
    let before = generate_profile(&nodes, &options).unwrap();
    options.policy_groups.extend([
        PolicyGroupSpec {
            include_all_nodes: true,
            ..renamed_spec(
                "节点选择",
                "手动出口",
                GroupKind::Select,
                &["自动选择", "DIRECT"],
            )
        },
        PolicyGroupSpec {
            include_all_nodes: true,
            ..renamed_spec("自动选择", "最快出口", GroupKind::UrlTest, &[])
        },
        renamed_spec(
            "漏网之鱼",
            "默认出口",
            GroupKind::Select,
            &["节点选择", "DIRECT"],
        ),
        renamed_spec(
            ai,
            "智能服务",
            GroupKind::Select,
            &["节点选择", "自动选择", "DIRECT"],
        ),
    ]);
    let saved_options = serde_json::to_value(&options).unwrap();
    let profile = generate_profile(&nodes, &options).unwrap();
    let names: HashSet<_> = profile
        .groups
        .iter()
        .map(|group| group.name.as_str())
        .collect();
    for old in ["节点选择", "自动选择", "漏网之鱼", ai] {
        assert!(!names.contains(old));
        assert!(
            profile
                .groups
                .iter()
                .all(|group| !group.proxies.iter().any(|member| member == old))
        );
        assert!(profile.rules.iter().all(|rule| rule.target != old));
    }
    for name in ["手动出口", "最快出口", "默认出口", "智能服务"] {
        assert!(names.contains(name));
    }
    assert_eq!(profile.rules.last().unwrap().to_line(), "MATCH,默认出口");
    assert_eq!(
        profile.rules.first().unwrap().to_line(),
        "DOMAIN,work.example,手动出口"
    );
    assert!(
        profile
            .rules
            .iter()
            .any(|rule| rule.payload.as_deref() == Some("openai") && rule.target == "智能服务")
    );
    assert!(
        profile.rules.iter().any(
            |rule| rule.payload.as_deref() == Some("work-domains") && rule.target == "智能服务"
        )
    );
    let identity = |rules: &[Rule]| {
        rules
            .iter()
            .map(|rule| {
                (
                    rule.kind.clone(),
                    rule.payload.clone(),
                    rule.no_resolve,
                    rule.source_ip,
                )
            })
            .collect::<Vec<_>>()
    };
    assert_eq!(identity(&before.rules), identity(&profile.rules));
    let yaml: Value = serde_yaml_ng::from_str(&profile.yaml).unwrap();
    assert_eq!(yaml["listeners"][0]["proxy"].as_str(), Some("手动出口"));
    assert_eq!(serde_json::to_value(&options).unwrap(), saved_options);
}

#[test]
fn removed_renamed_builtins_stay_removed_and_repair_remaining_references() {
    let mut options = GenerateOptions {
        preset: Preset::Minimal,
        enabled_modules: Some(Vec::new()),
        ..Default::default()
    };
    for (origin, name) in [
        ("节点选择", "手动出口"),
        ("自动选择", "最快出口"),
        ("漏网之鱼", "默认出口"),
    ] {
        let mut spec = renamed_spec(origin, name, GroupKind::Select, &[]);
        spec.disabled = true;
        spec.removed = true;
        options.policy_groups.push(spec);
    }
    options.policy_groups.push(PolicyGroupSpec {
        name: "保留组".into(),
        members: vec!["节点选择".into(), "手动出口".into()],
        ..Default::default()
    });
    options
        .custom_rules
        .push(Rule::new("DOMAIN", "disabled.example", "节点选择"));
    options.listeners.push(ListenerSpec {
        name: "listener".into(),
        port: 7898,
        proxy: "默认出口".into(),
        kind: "mixed".into(),
        listen: "127.0.0.1".into(),
        udp: false,
    });
    let mut nodes = nodes();
    nodes[0].proxy.set_dialer_proxy("节点选择");
    let profile = generate_profile(&nodes, &options).unwrap();
    assert_eq!(profile.groups.len(), 1);
    assert_eq!(profile.groups[0].name, "保留组");
    assert_eq!(profile.groups[0].proxies, ["DIRECT"]);
    assert_eq!(profile.rules.last().unwrap().to_line(), "MATCH,DIRECT");
    assert!(
        !profile
            .rules
            .iter()
            .any(|rule| rule.payload.as_deref() == Some("disabled.example"))
    );
    let yaml: Value = serde_yaml_ng::from_str(&profile.yaml).unwrap();
    assert_eq!(yaml["listeners"][0]["proxy"].as_str(), Some("DIRECT"));
    assert_eq!(yaml["proxies"][0]["dialer-proxy"].as_str(), Some("DIRECT"));
}

#[test]
fn saved_group_order_applies_to_builtin_custom_filter_and_chain_groups_and_yaml() {
    let nodes = nodes();
    let mut options = GenerateOptions {
        preset: Preset::Minimal,
        enabled_modules: Some(Vec::new()),
        ..Default::default()
    };
    options.policy_groups.push(PolicyGroupSpec {
        name: "工作".into(),
        members: vec!["节点选择".into()],
        ..Default::default()
    });
    options
        .custom_groups
        .push(CustomGroupSpec::new("筛选", NodeFilter::default()));
    options.chains.push(ChainSpec::new(
        "链式",
        &nodes[0].source_key,
        nodes[0].name(),
        &nodes[1].source_key,
        nodes[1].name(),
    ));
    let before = generate_profile(&nodes, &options).unwrap();
    let order = ["工作", "链式", "筛选", "漏网之鱼", "自动选择", "节点选择"];
    options.group_order = order.iter().map(|name| (*name).into()).collect();
    options.group_order.insert(0, "已删除组".into());
    let profile = generate_profile(&nodes, &options).unwrap();
    assert_eq!(
        profile
            .groups
            .iter()
            .map(|group| group.name.as_str())
            .collect::<Vec<_>>(),
        order
    );
    let yaml: Value = serde_yaml_ng::from_str(&profile.yaml).unwrap();
    assert_eq!(
        yaml["proxy-groups"]
            .as_sequence()
            .unwrap()
            .iter()
            .map(|group| group["name"].as_str().unwrap())
            .collect::<Vec<_>>(),
        order
    );
    assert_eq!(profile.rules, before.rules);
    assert_eq!(profile.stats.proxies, before.stats.proxies);
}

#[test]
fn conflicting_builtin_renames_fail_without_creating_duplicate_groups() {
    let mut options = GenerateOptions {
        preset: Preset::Minimal,
        ..Default::default()
    };
    options.policy_groups.push(renamed_spec(
        "节点选择",
        "重名",
        GroupKind::Select,
        &["DIRECT"],
    ));
    options.policy_groups.push(renamed_spec(
        "自动选择",
        "重名",
        GroupKind::UrlTest,
        &["DIRECT"],
    ));
    assert!(generate_profile(&nodes(), &options).is_err());
    options.policy_groups[1].name = "节点选择".into();
    assert!(generate_profile(&nodes(), &options).is_err());
    options.policy_groups.pop();
    options.policy_groups.push(PolicyGroupSpec {
        name: "节点选择".into(),
        members: vec!["DIRECT".into()],
        ..Default::default()
    });
    assert!(generate_profile(&nodes(), &options).is_err());
}
