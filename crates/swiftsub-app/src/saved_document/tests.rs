use super::*;

#[test]
fn quoted_numbers_local_proxies_and_rule_parameter_names_remain_editable() {
    let mut doc = SavedDocument::parse("mixed-port: '7897'\nproxies:\n  - {name: local, type: direct}\nproxy-groups:\n  - {name: src, type: select, proxies: [local]}\n  - {name: no-resolve, type: url-test, proxies: [local], interval: '300'}\nrules:\n  - 'DOMAIN-REGEX,^test[a-z]{1,3}\\.example\\.org$,src'\n  - 'IP-CIDR,192.0.2.0/24,no-resolve,src,no-resolve'\n  - 'MATCH,no-resolve'\n", None).unwrap();
    doc.validate().unwrap();
    assert_eq!(rule_target(&doc.rules()[1]), Some("no-resolve"));
    doc.transaction(|doc| doc.update_node("local", "renamed-direct", "", "0"))
        .unwrap();
    assert_eq!(doc.members("src"), ["renamed-direct"]);
    assert!(doc.items("proxies")[0].get("port").is_none());
    doc.transaction(|doc| doc.rename("src", "regex-target"))
        .unwrap();
    assert_eq!(rule_target(&doc.rules()[0]), Some("regex-target"));
    assert!(doc.rules()[0].contains("{1,3}"));
    doc.transaction(|doc| doc.rename("no-resolve", "ip-target"))
        .unwrap();
    assert_eq!(
        doc.rules()[1],
        "IP-CIDR,192.0.2.0/24,ip-target,src,no-resolve"
    );
    assert_eq!(doc.rules()[2], "MATCH,ip-target");
    doc.validate().unwrap();
}
fn fixture() -> SavedDocument {
    SavedDocument::parse(r#"
# comment preserved on name-only save by the controller
mixed-port: 7897
proxies:
  - {name: node-a, type: ss, server: test.invalid, port: 443, cipher: aes-128-gcm, password: secret, future-node-field: keep}
  - {name: node-b, type: ss, server: test.invalid, port: 444, cipher: aes-128-gcm, password: secret}
proxy-groups:
  - {name: 节点选择, type: select, proxies: [node-a, node-b, 自动选择], future-group-field: keep}
  - {name: 自动选择, type: url-test, proxies: [node-a, node-b], url: 'https://example.com/check', interval: 300}
  - {name: Video, type: select, proxies: [节点选择, DIRECT]}
  - {name: 漏网之鱼, type: select, proxies: [节点选择, DIRECT]}
rules:
  - DOMAIN-SUFFIX,video.example,Video
  - AND,((NETWORK,UDP),(DST-PORT,443)),Video
  - MATCH,漏网之鱼
listeners:
  - {name: listener, type: mixed, port: 8000, proxy: Video}
dns: {enable: true, nameserver: ['https://dns.example/query'], future-dns: keep}
tun: {enable: true, future-tun: keep}
future-top-level: {nested: [a,b,c]}
"#,None).unwrap()
}
#[test]
fn edits_actual_yaml_and_preserves_unknown_fields_and_complex_rule_payload() {
    let mut d = fixture();
    let original = d.root.clone();
    d.transaction(|d| {
        d.update_group(
            "Video",
            "Stream",
            "fallback",
            "https://example.com/test",
            "600",
        )
    })
    .unwrap();
    assert_eq!(d.root["future-top-level"], original["future-top-level"]);
    assert_eq!(d.root["tun"], original["tun"]);
    assert_eq!(d.root["dns"], original["dns"]);
    assert_eq!(d.root["proxies"], original["proxies"]);
    assert_eq!(text(&d.items("listeners")[0], "proxy"), "Stream");
    assert_eq!(d.rules()[1], "AND,((NETWORK,UDP),(DST-PORT,443)),Stream");
    assert!(d.rules().iter().all(|r| rule_target(r) != Some("Video")));
    assert_eq!(
        d.group("节点选择").unwrap()["future-group-field"],
        Value::String("keep".into())
    );
}
#[test]
fn chain_edits_keep_position_disabled_state_outlet_and_references_after_reopen() {
    let mut doc = fixture();
    let name = doc.save_chain(None, "", "node-a", "node-b").unwrap();
    doc.transaction(|doc| {
        doc.member("Video", &name, true)?;
        doc.move_item("proxies", 2, 0)?;
        doc.toggle_node(&name, false)
    })
    .unwrap();
    let source = doc.source_json(None).unwrap();
    let mut reopened = SavedDocument::parse(&doc.yaml().unwrap(), Some(&source)).unwrap();
    assert_eq!(reopened.chain_exit(&name).as_deref(), Some("node-b"));
    reopened
        .transaction(|doc| {
            doc.save_chain(Some(&name), "新链路", "node-b", "node-a")
                .map(|_| ())
        })
        .unwrap();
    assert!(!reopened.names("proxies").contains(&"新链路".into()));
    assert_eq!(reopened.metadata.disabled_nodes[0].index, 0);
    let edited = reopened.editable_node("新链路").unwrap();
    assert_eq!(text(&edited, "dialer-proxy"), "node-b");
    assert_eq!(unsigned(edited.get("port")), Some(443));
    assert_eq!(text(&edited, "future-node-field"), "keep");
    assert_eq!(reopened.chain_exit("新链路").as_deref(), Some("node-a"));
    reopened
        .transaction(|doc| {
            doc.rename("node-a", "新出口")?;
            doc.toggle_node("新链路", true)
        })
        .unwrap();
    assert_eq!(reopened.names("proxies")[0], "新链路");
    assert!(reopened.members("Video").contains(&"新链路".into()));
    assert!(!reopened.members("Video").contains(&name));
    assert_eq!(reopened.chain_exit("新链路").as_deref(), Some("新出口"));
    let persisted = SavedDocument::parse(
        &reopened.yaml().unwrap(),
        Some(&reopened.source_json(None).unwrap()),
    )
    .unwrap();
    assert_eq!(persisted.chain_exit("新链路").as_deref(), Some("新出口"));
    assert_eq!(
        persisted.root["future-top-level"],
        fixture().root["future-top-level"]
    );
}

#[test]
fn invalid_chain_edits_roll_back_name_credentials_and_metadata() {
    let mut doc = fixture();
    doc.save_chain(None, "Chain", "node-a", "node-b").unwrap();
    doc.validate().unwrap();
    let before = doc.clone();
    for (entry, exit, name) in [
        ("Chain", "node-b", "Renamed"),
        ("node-a", "node-a", "Renamed"),
        ("missing", "node-b", "Renamed"),
        ("node-a", "node-b", "Video"),
    ] {
        assert!(
            doc.transaction(|doc| doc.save_chain(Some("Chain"), name, entry, exit).map(|_| ()))
                .is_err()
        );
        assert_eq!(doc.root, before.root);
        assert_eq!(doc.metadata, before.metadata);
    }
    assert!(
        doc.transaction(|doc| {
            doc.save_chain(Some("Chain"), "Renamed", "node-a", "node-b")?;
            let mut node = doc.editable_node("node-a").unwrap();
            put(&mut node, "dialer-proxy", "Renamed");
            let mut nodes = doc.items("proxies");
            nodes[0] = node;
            doc.set_items("proxies", nodes);
            Ok(())
        })
        .is_err()
    );
    assert_eq!(doc.root, before.root);
    assert_eq!(doc.metadata, before.metadata);
}

#[test]
fn cycle_invalid_interval_and_duplicate_names_roll_back_whole_change() {
    let mut d = fixture();
    let before = d.root.clone();
    assert!(
        d.transaction(|d| d.member("自动选择", "节点选择", true))
            .is_err()
    );
    assert_eq!(d.root, before);
    assert!(
        d.transaction(|d| d.update_group(
            "Video",
            "Renamed",
            "url-test",
            "https://example.com",
            "0"
        ))
        .is_err()
    );
    assert_eq!(d.root, before);
    assert!(d.transaction(|d| d.rename("Video", "node-a")).is_err());
    assert_eq!(d.root, before);
}
#[test]
fn disabled_nodes_and_memberships_survive_reopen_and_restore() {
    let mut d = fixture();
    d.transaction(|d| d.toggle_node("node-a", false)).unwrap();
    assert!(!d.names("proxies").contains(&"node-a".into()));
    assert!(!d.members("节点选择").contains(&"node-a".into()));
    let source = d.source_json(None).unwrap();
    let mut restored = SavedDocument::parse(&d.yaml().unwrap(), Some(&source)).unwrap();
    assert_eq!(restored.metadata, d.metadata);
    restored
        .transaction(|d| d.toggle_node("node-a", true))
        .unwrap();
    assert_eq!(restored.members("自动选择"), vec!["node-a", "node-b"]);
    assert_eq!(restored.names("proxies"), vec!["node-a", "node-b"]);
}
#[test]
fn last_node_uses_direct_fallback_then_restores_without_cycles() {
    let mut d = fixture();
    d.transaction(|d| {
        d.toggle_node("node-a", false)?;
        d.toggle_node("node-b", false)
    })
    .unwrap();
    assert_eq!(d.members("自动选择"), vec!["DIRECT"]);
    d.transaction(|d| d.toggle_node("node-b", true)).unwrap();
    assert_eq!(d.members("自动选择"), vec!["node-b"]);
}
#[test]
fn rules_can_be_disabled_retargeted_reordered_and_restored() {
    let mut d = fixture();
    d.transaction(|d| d.set_rule_enabled(0, false)).unwrap();
    assert_eq!(d.metadata.disabled_rules.len(), 1);
    d.transaction(|d| d.set_rule_enabled(0, true)).unwrap();
    assert_eq!(d.rules()[0], "DOMAIN-SUFFIX,video.example,Video");
    d.transaction(|d| d.set_rule_target(0, "DIRECT")).unwrap();
    d.transaction(|d| d.move_item("rules", 0, 1)).unwrap();
    assert!(d.rules()[1].ends_with(",DIRECT"));
    let before = d.root.clone();
    assert!(d.transaction(|d| d.move_item("rules", 2, 0)).is_err());
    assert_eq!(d.root, before);
    assert!(d.transaction(|d| d.set_rule_enabled(2, false)).is_err());
}
#[test]
fn mixed_rule_sources_accumulate_independently_and_never_replace_custom_provider() {
    let mut d = fixture();
    let binding = |id: &str, target: &str| RuleSetBinding {
        id: id.into(),
        name: id.into(),
        url: format!("https://example.com/{id}.yaml"),
        behavior: "classical".into(),
        format: "yaml".into(),
        target: target.into(),
        no_resolve: false,
    };
    let a = binding("bm-video", "Video");
    let b = binding("acl-ads", "广告拦截");
    d.transaction(|d| {
        d.add_binding(&a, true)?;
        d.add_binding(&b, true)
    })
    .unwrap();
    assert!(d.group("广告拦截").is_some());
    d.transaction(|d| d.add_binding(&a, false)).unwrap();
    assert!(d.catalog_rule_enabled("acl-ads"));
    assert!(!d.catalog_rule_enabled("bm-video"));
    let mut conflict = b.clone();
    conflict.url = "https://example.com/other.yaml".into();
    let before = d.root.clone();
    assert!(d.transaction(|d| d.add_binding(&conflict, true)).is_err());
    assert_eq!(d.root, before);
    assert_eq!(d.rules().last().unwrap(), "MATCH,漏网之鱼");
}
#[test]
fn node_rename_updates_members_without_touching_credentials() {
    let mut d = fixture();
    d.transaction(|d| d.update_node("node-a", "New Name", "new.invalid", "8443"))
        .unwrap();
    let node = &d.items("proxies")[0];
    assert_eq!(text(node, "password"), "secret");
    assert_eq!(text(node, "future-node-field"), "keep");
    assert!(d.members("节点选择").contains(&"New Name".into()));
    assert!(!d.members("自动选择").contains(&"node-a".into()));
}
#[test]
fn copy_and_delete_group_preserve_dependent_references() {
    let mut d = fixture();
    d.transaction(|d| {
        d.add_group("Video Copy", Some("Video"))?;
        Ok(())
    })
    .unwrap();
    assert!(d.group("Video Copy").is_some());
    d.transaction(|d| d.remove_group("Video")).unwrap();
    assert!(d.group("Video").is_none());
    assert_eq!(text(&d.items("listeners")[0], "proxy"), "节点选择");
    assert_eq!(d.rules().last().unwrap(), "MATCH,漏网之鱼");
    d.transaction(|d| d.remove_group("漏网之鱼")).unwrap();
    assert_eq!(d.rules().last().unwrap(), "MATCH,节点选择");
    d.transaction(|d| d.remove_group("节点选择")).unwrap();
    assert_eq!(d.rules().last().unwrap(), "MATCH,DIRECT");
    d.transaction(|d| d.remove_group("自动选择")).unwrap();
    d.validate().unwrap();
}
#[test]
fn basic_settings_keep_dns_and_extension_options() {
    let mut d = fixture();
    let original = d.root.clone();
    d.transaction(|d| d.set_basic("9000", "rule", "warning", true, false))
        .unwrap();
    assert_eq!(d.root["dns"]["future-dns"], original["dns"]["future-dns"]);
    assert_eq!(d.root["dns"]["nameserver"], original["dns"]["nameserver"]);
    assert_eq!(d.root["tun"], original["tun"]);
}
#[test]
fn disabling_group_restores_members_rules_and_listener_after_reopen() {
    let mut doc = fixture();
    doc.transaction(|d| d.member("节点选择", "Video", true))
        .unwrap_err();
    let before = doc.root.clone();
    doc.transaction(|d| d.toggle_group("Video", false)).unwrap();
    assert!(doc.group("Video").is_none());
    assert!(doc.rules().iter().all(|r| rule_target(r) != Some("Video")));
    assert_eq!(text(&doc.items("listeners")[0], "proxy"), "节点选择");
    let source = doc.source_json(None).unwrap();
    let mut restored = SavedDocument::parse(&doc.yaml().unwrap(), Some(&source)).unwrap();
    restored
        .transaction(|d| d.toggle_group("Video", true))
        .unwrap();
    assert_eq!(restored.root, before);
    restored
        .transaction(|d| d.toggle_group("漏网之鱼", false))
        .unwrap();
    assert_eq!(restored.rules().last().unwrap(), "MATCH,节点选择");
    let source = restored.source_json(None).unwrap();
    restored = SavedDocument::parse(&restored.yaml().unwrap(), Some(&source)).unwrap();
    restored
        .transaction(|d| d.toggle_group("漏网之鱼", true))
        .unwrap();
    assert_eq!(restored.root, before);
}

#[test]
fn disabled_nodes_can_be_edited_restored_and_deleted_with_direct_references() {
    let mut doc = fixture();
    let mut proxies = doc.items("proxies");
    put(&mut proxies[1], "dialer-proxy", "node-a");
    doc.set_items("proxies", proxies);
    let mut listeners = doc.items("listeners");
    put(&mut listeners[0], "proxy", "node-a");
    doc.set_items("listeners", listeners);
    doc.set_rules(vec![
        "DOMAIN,example.org,node-a".into(),
        "MATCH,node-a".into(),
    ]);
    put(
        &mut doc.root,
        "sub-rules",
        serde_yaml_ng::from_str::<Value>("custom: ['DOMAIN,sub.example.org,node-a']").unwrap(),
    );
    put(
        &mut doc.root,
        "rule-providers",
        serde_yaml_ng::from_str::<Value>(
            "custom: {type: http, url: 'https://example.org/rules', proxy: node-a}",
        )
        .unwrap(),
    );
    doc.validate().unwrap();
    doc.transaction(|doc| doc.toggle_node("node-a", false))
        .unwrap();
    assert_eq!(doc.rules(), vec!["MATCH,DIRECT"]);
    assert_eq!(text(&doc.items("proxies")[0], "dialer-proxy"), "DIRECT");
    let source = doc.source_json(None).unwrap();
    doc = SavedDocument::parse(&doc.yaml().unwrap(), Some(&source)).unwrap();
    doc.transaction(|doc| doc.update_node("node-a", "Edited", "edited.example.org", "8443"))
        .unwrap();
    assert!(doc.names("proxies").iter().all(|name| name != "Edited"));
    doc.transaction(|doc| doc.toggle_node("Edited", true))
        .unwrap();
    assert_eq!(
        doc.rules(),
        vec!["DOMAIN,example.org,Edited", "MATCH,Edited"]
    );
    assert_eq!(text(&doc.items("listeners")[0], "proxy"), "Edited");
    assert_eq!(text(&doc.items("proxies")[1], "dialer-proxy"), "Edited");
    assert_eq!(
        doc.root["sub-rules"]["custom"][0].as_str(),
        Some("DOMAIN,sub.example.org,Edited")
    );
    assert_eq!(
        doc.root["rule-providers"]["custom"]["proxy"].as_str(),
        Some("Edited")
    );
    assert_eq!(
        doc.editable_node("Edited").unwrap()["password"].as_str(),
        Some("secret")
    );
    doc.transaction(|doc| doc.toggle_node("Edited", false))
        .unwrap();
    doc.transaction(|doc| doc.remove_node("Edited")).unwrap();
    assert!(doc.editable_node("Edited").is_none());
    doc.validate().unwrap();
}

#[test]
fn disabled_groups_remain_editable_and_do_not_overwrite_a_changed_match_target() {
    let mut doc = fixture();
    doc.transaction(|doc| doc.toggle_group("漏网之鱼", false))
        .unwrap();
    doc.transaction(|doc| {
        doc.update_group(
            "漏网之鱼",
            "Fallback",
            "select",
            "https://example.org",
            "300",
        )
    })
    .unwrap();
    let index = doc.rules().len() - 1;
    doc.transaction(|doc| doc.set_rule_target(index, "DIRECT"))
        .unwrap();
    doc.transaction(|doc| doc.toggle_group("Fallback", true))
        .unwrap();
    assert!(doc.group("Fallback").is_some());
    assert_eq!(doc.rules().last().unwrap(), "MATCH,DIRECT");
    doc.transaction(|doc| doc.toggle_group("Fallback", false))
        .unwrap();
    doc.transaction(|doc| doc.remove_group("Fallback")).unwrap();
    assert!(doc.editable_group("Fallback").is_none());
    doc.validate().unwrap();
}
#[test]
fn disabled_group_members_follow_node_renames() {
    let mut doc = fixture();
    doc.transaction(|d| d.member("Video", "node-a", true))
        .unwrap();
    doc.transaction(|d| d.toggle_group("Video", false)).unwrap();
    doc.transaction(|d| d.rename("node-a", "Renamed")).unwrap();
    doc.transaction(|d| d.toggle_group("Video", true)).unwrap();
    assert!(doc.members("Video").contains(&"Renamed".into()));
}
#[test]
fn source_sync_updates_only_its_nodes_and_keeps_local_aliases() {
    let mut doc = fixture();
    let other = doc.items("proxies")[0].clone();
    let incoming = |name: &str, port: u16| {
        let mut n = other.clone();
        put(&mut n, "name", name);
        put(&mut n, "port", Value::from(port));
        n
    };
    doc.transaction(|d| {
        d.sync_source_nodes(
            42,
            vec![incoming("remote-a", 1234), incoming("remote-b", 1235)],
            true,
        )
    })
    .unwrap();
    doc.transaction(|d| d.rename("remote-a", "Local Alias"))
        .unwrap();
    doc.transaction(|d| {
        d.sync_source_nodes(
            42,
            vec![incoming("remote-a", 4321), incoming("remote-c", 4322)],
            true,
        )
    })
    .unwrap();
    assert_eq!(doc.items("proxies")[0], other);
    assert!(!doc.names("proxies").contains(&"remote-b".into()));
    let updated = doc
        .items("proxies")
        .into_iter()
        .find(|n| text(n, "name") == "Local Alias")
        .unwrap();
    assert_eq!(updated["port"], Value::from(4321));
    assert!(doc.members("自动选择").contains(&"remote-c".into()));
    let before = doc.names("proxies").len();
    doc.transaction(|d| d.sync_source_nodes(42, vec![incoming("remote-a", 4321)], false))
        .unwrap();
    assert_eq!(doc.names("proxies").len(), before);
}
#[test]
fn upstream_rename_keeps_disabled_node_disabled_and_restores_memberships() {
    let mut doc = fixture();
    let mut node = doc.items("proxies")[0].clone();
    let old = text(&node, "name");
    doc.transaction(|doc| doc.sync_source_nodes(8, vec![node.clone()], false))
        .unwrap();
    doc.transaction(|doc| doc.toggle_node(&old, false)).unwrap();
    put(&mut node, "name", "upstream-renamed");
    doc.transaction(|doc| doc.sync_source_nodes(8, vec![node], true))
        .unwrap();
    assert!(!doc.names("proxies").contains(&"upstream-renamed".into()));
    assert!(
        doc.metadata
            .disabled_nodes
            .iter()
            .any(|node| text(&node.value, "name") == "upstream-renamed")
    );
    doc.transaction(|doc| doc.toggle_node("upstream-renamed", true))
        .unwrap();
    assert!(doc.names("proxies").contains(&"upstream-renamed".into()));
    assert!(!doc.names("proxies").contains(&old));
    doc.validate().unwrap();
}

#[test]
fn node_replacement_survives_disabled_group_restore_and_retargets_sub_rules() {
    let mut doc = SavedDocument::parse("proxies:\n  - {name: old, type: trojan, server: old.example.org, port: 443, password: test}\n  - {name: new, type: trojan, server: new.example.org, port: 443, password: test}\nproxy-groups:\n  - {name: Select, type: select, proxies: [old, new]}\nrules:\n  - DOMAIN,example.org,old\n  - MATCH,DIRECT\nsub-rules:\n  custom: ['DOMAIN,sub.example.org,old']\nrule-providers:\n  provider: {type: http, url: 'https://example.org/rules', proxy: old}\n", None).unwrap();
    doc.transaction(|doc| doc.toggle_group("Select", false))
        .unwrap();
    doc.transaction(|doc| {
        doc.replace_node_references("old", "new")?;
        doc.toggle_node("old", false)
    })
    .unwrap();
    doc.metadata
        .disabled_nodes
        .retain(|node| text(&node.value, "name") != "old");
    let source = doc.source_json(None).unwrap();
    doc = SavedDocument::parse(&doc.yaml().unwrap(), Some(&source)).unwrap();
    doc.transaction(|doc| doc.toggle_group("Select", true))
        .unwrap();
    assert_eq!(doc.members("Select"), vec!["new"]);
    assert_eq!(doc.rules()[0], "DOMAIN,example.org,new");
    assert_eq!(
        doc.root["sub-rules"]["custom"][0].as_str(),
        Some("DOMAIN,sub.example.org,new")
    );
    assert_eq!(
        doc.root["rule-providers"]["provider"]["proxy"].as_str(),
        Some("new")
    );
    doc.validate().unwrap();
}
#[test]
fn hundred_node_sync_preserves_local_fields_alias_and_disabled_state() {
    let mut doc = fixture();
    let sample = doc.items("proxies")[0].clone();
    let nodes: Vec<_> = (0..100)
        .map(|index| {
            let mut node = sample.clone();
            put(&mut node, "name", format!("remote-{index}"));
            put(&mut node, "port", Value::from(1000 + index));
            node
        })
        .collect();
    doc.transaction(|doc| doc.sync_source_nodes(99, nodes.clone(), false))
        .unwrap();
    doc.transaction(|doc| doc.rename("remote-0", "local-alias"))
        .unwrap();
    let mut proxies = doc.items("proxies");
    let local = proxies
        .iter_mut()
        .find(|node| text(node, "name") == "local-alias")
        .unwrap();
    put(local, "password", "local-secret");
    put(local, "udp", false);
    doc.set_items("proxies", proxies);
    doc.transaction(|doc| doc.toggle_node("remote-1", false))
        .unwrap();
    let source = doc.source_json(None).unwrap();
    doc = SavedDocument::parse(&doc.yaml().unwrap(), Some(&source)).unwrap();
    let mut incoming = nodes;
    put(&mut incoming[0], "name", "upstream-renamed");
    put(&mut incoming[2], "port", Value::from(9000));
    doc.transaction(|doc| doc.sync_source_nodes(99, incoming, true))
        .unwrap();
    let local = doc
        .items("proxies")
        .into_iter()
        .find(|node| text(node, "name") == "local-alias")
        .unwrap();
    assert_eq!(local["password"].as_str(), Some("local-secret"));
    assert_eq!(local["udp"].as_bool(), Some(false));
    assert!(!doc.names("proxies").contains(&"remote-1".into()));
    assert_eq!(
        doc.metadata
            .node_sources
            .iter()
            .filter(|source| source.source_id == 99)
            .count(),
        100
    );
    assert_eq!(
        doc.items("proxies")
            .into_iter()
            .find(|node| text(node, "name") == "remote-2")
            .unwrap()["port"],
        Value::from(9000)
    );
    doc.validate().unwrap();
}
#[test]
fn source_sync_preserves_disabled_node_and_is_independent_of_another_source() {
    let mut doc = fixture();
    let value = doc.items("proxies")[0].clone();
    doc.transaction(|d| d.sync_source_nodes(1, vec![value.clone()], true))
        .unwrap();
    doc.transaction(|d| d.sync_source_nodes(2, vec![value.clone()], true))
        .unwrap();
    let first = doc
        .metadata
        .node_sources
        .iter()
        .find(|n| n.source_id == 1)
        .unwrap()
        .name
        .clone();
    let second = doc
        .metadata
        .node_sources
        .iter()
        .find(|n| n.source_id == 2)
        .unwrap()
        .name
        .clone();
    assert_ne!(first, second);
    doc.transaction(|d| d.toggle_node(&first, false)).unwrap();
    doc.transaction(|d| d.sync_source_nodes(1, vec![value], true))
        .unwrap();
    assert!(!doc.names("proxies").contains(&first));
    assert!(doc.names("proxies").contains(&second));
    assert!(
        doc.metadata
            .disabled_nodes
            .iter()
            .any(|n| text(&n.value, "name") == first)
    );
}
#[test]
fn scoped_rule_selection_moves_target_and_clears_only_current_group() {
    let mut doc = fixture();
    let b = RuleSetBinding {
        id: "source-list".into(),
        name: "source".into(),
        url: "https://example.com/source.yaml".into(),
        behavior: "domain".into(),
        format: "yaml".into(),
        target: "Video".into(),
        no_resolve: false,
    };
    doc.transaction(|d| d.set_scoped_binding(&b, true, Some("Video")))
        .unwrap();
    doc.transaction(|d| d.set_scoped_binding(&b, false, Some("DIRECT")))
        .unwrap();
    assert!(doc.catalog_rule_enabled(&b.id));
    let mut direct = b.clone();
    direct.target = "DIRECT".into();
    doc.transaction(|d| d.set_scoped_binding(&direct, true, Some("DIRECT")))
        .unwrap();
    assert_eq!(doc.catalog_rule_target(&b.id), Some("DIRECT".into()));
    doc.transaction(|d| d.set_scoped_binding(&b, false, Some("Video")))
        .unwrap();
    assert!(doc.catalog_rule_enabled(&b.id));
    doc.transaction(|d| d.set_scoped_binding(&direct, false, Some("DIRECT")))
        .unwrap();
    assert!(!doc.catalog_rule_enabled(&b.id));
}
#[test]
fn source_metadata_replaces_stale_generator_recipe_and_keeps_extension_data() {
    let doc = fixture();
    let source=doc.source_json(Some(r#"{"routing":{"old":"recipe"},"membership":[1],"filename":"old.yaml","external":{"keep":true}}"#)).unwrap();
    let json: serde_json::Value = serde_json::from_str(&source).unwrap();
    assert!(json.get("routing").is_none());
    assert_eq!(json["external"]["keep"], true);
    assert!(json.get("visual_editor").is_some());
}
#[test]
fn toggling_group_and_node_in_either_order_keeps_valid_memberships() {
    let mut doc = fixture();
    doc.transaction(|d| d.member("Video", "node-a", true))
        .unwrap();
    doc.transaction(|d| d.toggle_group("Video", false)).unwrap();
    doc.transaction(|d| d.toggle_node("node-a", false)).unwrap();
    doc.transaction(|d| d.toggle_group("Video", true)).unwrap();
    assert!(!doc.members("Video").contains(&"node-a".into()));
    doc.transaction(|d| d.toggle_node("node-a", true)).unwrap();
    assert!(doc.members("Video").contains(&"node-a".into()));
    doc.transaction(|d| d.toggle_group("Video", false)).unwrap();
    doc.transaction(|d| d.toggle_node("node-a", false)).unwrap();
    doc.transaction(|d| d.toggle_node("node-a", true)).unwrap();
    doc.transaction(|d| d.toggle_group("Video", true)).unwrap();
    assert!(doc.members("Video").contains(&"node-a".into()));
}
#[test]
fn visual_edit_persists_to_same_file_and_survives_database_reopen() {
    use std::{
        fs,
        time::{SystemTime, UNIX_EPOCH},
    };
    use swiftsub_store::Store;
    let directory = std::env::temp_dir().join(format!(
        "swiftsub-visual-store-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let files = directory.join("configs");
    let db_path = directory.join("test.db");
    fs::create_dir_all(&directory).unwrap();
    let store = Store::open(&db_path).unwrap();
    let doc = fixture();
    let yaml = doc.yaml().unwrap();
    let first = store
        .save_yaml_config(&files, None, "First", &yaml, None)
        .unwrap();
    let second = store
        .save_yaml_config(&files, None, "Second", &yaml, None)
        .unwrap();
    let mut changed = SavedDocument::parse(&first.yaml, None).unwrap();
    changed
        .transaction(|d| {
            d.toggle_group("Video", false)?;
            d.toggle_node("node-a", false)
        })
        .unwrap();
    let source = changed.source_json(None).unwrap();
    let edited = changed.yaml().unwrap();
    let saved = store
        .save_yaml_config(&files, Some(&first.id), "Renamed", &edited, Some(&source))
        .unwrap();
    assert_eq!(saved.id, first.id);
    assert_eq!(saved.path(&files), first.path(&files));
    assert_eq!(
        fs::read_to_string(second.path(&files)).unwrap(),
        second.yaml
    );
    drop(store);
    let reopened = Store::open(&db_path).unwrap();
    let all = reopened.list_saved_configs().unwrap();
    assert_eq!(all.len(), 2);
    let current = all.iter().find(|c| c.id == first.id).unwrap();
    assert_eq!(current.name, "Renamed");
    assert_eq!(
        fs::read_to_string(current.path(&files)).unwrap(),
        current.yaml
    );
    let mut restored = SavedDocument::parse(&current.yaml, current.source_json.as_deref()).unwrap();
    restored
        .transaction(|d| {
            d.toggle_node("node-a", true)?;
            d.toggle_group("Video", true)
        })
        .unwrap();
    restored.validate().unwrap();
    assert!(restored.group("Video").is_some());
    assert!(restored.names("proxies").contains(&"node-a".into()));
    drop(reopened);
    for record in all {
        fs::remove_file(record.path(&files)).unwrap();
    }
    fs::remove_dir(&files).unwrap();
    for name in ["test.db", "test.db-wal", "test.db-shm"] {
        let file = directory.join(name);
        if file.exists() {
            fs::remove_file(file).unwrap();
        }
    }
    fs::remove_dir(&directory).unwrap();
}
#[test]
fn source_import_adopts_matching_existing_nodes_without_duplicates() {
    let mut doc = fixture();
    let original = doc.items("proxies")[0].clone();
    let count = doc.items("proxies").len();
    doc.transaction(|d| d.sync_source_nodes(7, vec![original], false))
        .unwrap();
    assert_eq!(doc.items("proxies").len(), count);
    assert_eq!(doc.metadata.node_sources[0].name, "node-a");
}
