use std::time::Instant;
use swiftsub_core::{
    GenerateOptions, catalog::Preset, generate_profile, normalize::build_nodes,
    parser::parse_content,
};

fn main() {
    let links = (0..100)
        .map(|i| format!("trojan://benchmark-only@node-{i}.example.org:443#HK-{i}"))
        .collect::<Vec<_>>()
        .join("\n");
    let start = Instant::now();
    let parsed = parse_content(&links).unwrap();
    let nodes = build_nodes(parsed.proxies, "benchmark", "synthetic");
    println!(
        "nodes=100 parse_normalize_ms={:.3}",
        start.elapsed().as_secs_f64() * 1000.0
    );
    let output = std::env::args().nth(1).map(std::path::PathBuf::from);
    if let Some(directory) = &output {
        std::fs::create_dir_all(directory).unwrap();
    }
    for (name, preset) in [
        ("minimal", Preset::Minimal),
        ("standard", Preset::Standard),
        ("full", Preset::Full),
    ] {
        let options = GenerateOptions {
            preset,
            ..Default::default()
        };
        let mut durations = Vec::new();
        let mut last = None;
        for _ in 0..20 {
            let start = Instant::now();
            let profile = generate_profile(&nodes, &options).unwrap();
            durations.push(start.elapsed().as_secs_f64() * 1000.0);
            last = Some(profile);
        }
        durations.sort_by(f64::total_cmp);
        let profile = last.unwrap();
        println!(
            "preset={name} nodes=100 median_ms={:.3} p95_ms={:.3} yaml_bytes={} groups={} rules={}",
            durations[10],
            durations[18],
            profile.yaml.len(),
            profile.stats.groups,
            profile.stats.rules
        );
        if let Some(directory) = &output {
            std::fs::write(directory.join(format!("{name}.yaml")), profile.yaml).unwrap();
        }
    }
    if let Some(directory) = &output {
        use swiftsub_core::model::{
            ChainSpec, CustomGroupSpec, GroupKind, NodeFilter, PolicyGroupSpec,
        };
        use swiftsub_core::{Rule, region::Region};
        let mut options = GenerateOptions {
            policy_groups: [
                ("手动出口", GroupKind::Select),
                ("测速出口", GroupKind::UrlTest),
                ("备用出口", GroupKind::Fallback),
                ("均衡出口", GroupKind::LoadBalance),
            ]
            .into_iter()
            .map(|(name, kind)| PolicyGroupSpec {
                id: name.into(),
                name: name.into(),
                kind,
                members: vec!["HK-0".into(), "HK-1".into(), "DIRECT".into()],
                tolerance: Some(65),
                strategy: Some("round-robin".into()),
                test_interval: Some(90),
                ..Default::default()
            })
            .collect(),
            custom_groups: vec![CustomGroupSpec::new(
                "香港筛选",
                NodeFilter::new(vec![Region::Hk]),
            )],
            chains: vec![ChainSpec {
                name: "三跳链路".into(),
                hops: vec!["HK-0".into(), "HK-1".into(), "HK-2".into()],
                ..ChainSpec::new("", "", "", "", "")
            }],
            custom_rules: vec![Rule::new("DOMAIN-SUFFIX", "example.org", "手动出口")],
            rule_set_targets: [("openai".into(), "手动出口".into())].into(),
            ..Default::default()
        };
        options.policy_groups[0]
            .members
            .extend(["香港筛选".into(), "三跳链路".into()]);
        std::fs::write(
            directory.join("workspace-upgraded.yaml"),
            generate_profile(&nodes, &options).unwrap().yaml,
        )
        .unwrap();
        options.policy_groups[0].disabled = true;
        std::fs::write(
            directory.join("workspace-dormant.yaml"),
            generate_profile(&nodes, &options).unwrap().yaml,
        )
        .unwrap();
        options.policy_groups[0].disabled = false;
        options.rule_set_overrides.insert(
            "openai".into(),
            vec!["+.openai.com".into(), "api.example.org".into()],
        );
        options.custom_rule_sets.extend([
            swiftsub_core::generator::RuleSetBinding {
                id: "edited-classical".into(),
                name: "Edited rules".into(),
                url: "https://example.org/rules.yaml".into(),
                behavior: "classical".into(),
                format: "yaml".into(),
                target: "手动出口".into(),
                no_resolve: false,
            },
            swiftsub_core::generator::RuleSetBinding {
                id: "edited-ipcidr".into(),
                name: "Edited IP ranges".into(),
                url: "https://example.org/ip.txt".into(),
                behavior: "ipcidr".into(),
                format: "text".into(),
                target: "DIRECT".into(),
                no_resolve: true,
            },
        ]);
        options.rule_set_overrides.insert(
            "edited-classical".into(),
            vec![
                "DOMAIN-SUFFIX,example.org".into(),
                "IP-CIDR,192.0.2.0/24,no-resolve".into(),
            ],
        );
        options.rule_set_overrides.insert(
            "edited-ipcidr".into(),
            vec!["192.0.2.0/24".into(), "2001:db8::/32".into()],
        );
        std::fs::write(
            directory.join("workspace-edited-rules.yaml"),
            generate_profile(&nodes, &options).unwrap().yaml,
        )
        .unwrap();
        let special = parse_content("proxies:\n  - {name: named-direct, type: direct}\n  - name: peer-wireguard\n    type: wireguard\n    ip: 172.16.0.2\n    private-key: eCtXsJZ27+4PbhDkHnB923tkUn2Gj59wZw5wFA75MnU=\n    peers:\n      - {server: 192.0.2.1, port: 2480, public-key: 'Cr8hWlKvtDt7nrvf+f0brNQQzabAqrjfBvas9pmowjo=', allowed-ips: ['0.0.0.0/0']}\n").unwrap();
        let mut complex_nodes = nodes[..98].to_vec();
        complex_nodes.extend(build_nodes(special.proxies, "special", "synthetic-special"));
        assert_eq!(complex_nodes.len(), 100);
        options.custom_rules.extend([
            Rule::parse("AND,((DOMAIN,example.net),(NOT,((NETWORK,UDP)))),手动出口").unwrap(),
            Rule::parse(r"DOMAIN-REGEX,^test[a-z]{1,3}\.example\.org$,named-direct").unwrap(),
            Rule::parse("IP-CIDR,192.0.2.0/24,named-direct,src,no-resolve").unwrap(),
            Rule::parse("DSCP,18/20-30,DIRECT").unwrap(),
            Rule::parse("SUB-RULE,(DOMAIN-SUFFIX,sub.example.org),subnet").unwrap(),
        ]);
        options.extra_yaml = "sub-rules:\n  subnet:\n    - 'OR,((NETWORK,TCP),(DOMAIN,sub.example.org)),named-direct'\n    - 'MATCH,DIRECT'\n".into();
        std::fs::write(
            directory.join("workspace-complex-rules.yaml"),
            generate_profile(&complex_nodes, &options).unwrap().yaml,
        )
        .unwrap();
        let mut renamed = options.clone();
        renamed.policy_groups.extend([
            PolicyGroupSpec {
                id: "builtin:节点选择".into(),
                name: "默认手选".into(),
                builtin: true,
                members: vec!["自动选择".into(), "DIRECT".into()],
                include_all_nodes: true,
                ..Default::default()
            },
            PolicyGroupSpec {
                id: "builtin:自动选择".into(),
                name: "自动测速".into(),
                builtin: true,
                kind: GroupKind::UrlTest,
                include_all_nodes: true,
                ..Default::default()
            },
            PolicyGroupSpec {
                id: "builtin:漏网之鱼".into(),
                name: "其他流量".into(),
                builtin: true,
                members: vec!["节点选择".into(), "自动选择".into(), "DIRECT".into()],
                ..Default::default()
            },
        ]);
        renamed
            .listeners
            .push(swiftsub_core::generator::ListenerSpec {
                name: "renamed-group".into(),
                port: 7900,
                proxy: "节点选择".into(),
                kind: "mixed".into(),
                listen: "127.0.0.1".into(),
                udp: true,
            });
        let mut reordered_nodes = complex_nodes.clone();
        reordered_nodes.reverse();
        renamed.group_order = generate_profile(&reordered_nodes, &renamed)
            .unwrap()
            .groups
            .into_iter()
            .rev()
            .map(|group| group.name)
            .collect();
        std::fs::write(
            directory.join("workspace-renamed-ordered.yaml"),
            generate_profile(&reordered_nodes, &renamed).unwrap().yaml,
        )
        .unwrap();
        renamed
            .policy_groups
            .iter_mut()
            .find(|group| group.builtin_name() == Some("节点选择"))
            .unwrap()
            .disabled = true;
        std::fs::write(
            directory.join("workspace-renamed-dormant.yaml"),
            generate_profile(&reordered_nodes, &renamed).unwrap().yaml,
        )
        .unwrap();
        println!(
            "workspace variants: 100 nodes, four policy types, filter, three-hop chain and dormant restoration"
        );
    }
}
