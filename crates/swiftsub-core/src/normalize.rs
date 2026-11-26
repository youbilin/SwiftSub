use std::collections::HashSet;

use serde_yaml_ng::{Mapping, Value};

use crate::error::Result;
use crate::model::{Node, Proxy};
use crate::parser::parse_content;
use crate::region::Region;

pub fn dedupe_proxies(proxies: Vec<Proxy>) -> (Vec<Proxy>, usize) {
    let mut seen = HashSet::new();
    let mut unique = Vec::with_capacity(proxies.len());
    let mut removed = 0;

    for proxy in proxies {
        if seen.insert(proxy.identity_key()) {
            unique.push(proxy);
        } else {
            removed += 1;
        }
    }

    (unique, removed)
}

pub fn build_nodes(
    proxies: Vec<Proxy>,
    source_id: impl Into<String>,
    source_name: impl Into<String>,
) -> Vec<Node> {
    let source_id = source_id.into();
    let source_name = source_name.into();
    proxies
        .into_iter()
        .map(|proxy| Node::new(proxy, source_id.clone(), source_name.clone()))
        .collect()
}

pub fn ensure_unique_names(nodes: &mut [Node]) {
    const RESERVED: &[&str] = &[
        "DIRECT",
        "REJECT",
        "REJECT-DROP",
        "PASS",
        "COMPATIBLE",
        "GLOBAL",
    ];

    let mut used: HashSet<String> = HashSet::new();
    for node in nodes.iter_mut() {
        let mut name = node.name().to_string();
        if name.trim().is_empty() {
            name = format!("{}-{}", node.proxy.kind(), node.proxy.port());
        }
        if RESERVED
            .iter()
            .any(|reserved| reserved.eq_ignore_ascii_case(&name))
        {
            name = format!("{name} (node)");
        }

        let base = name.clone();
        let mut index = 2;
        while !used.insert(name.clone()) {
            name = format!("{base} {index}");
            index += 1;
        }
        node.set_name(name);
    }
}

pub fn apply_name_template(nodes: &mut [Node], template: &str) {
    if template.trim().is_empty() {
        return;
    }

    for (index, node) in nodes.iter_mut().enumerate() {
        let rendered = template
            .replace("{index}", &(index + 1).to_string())
            .replace("{index:02}", &format!("{:02}", index + 1))
            .replace("{region}", node.region.label())
            .replace("{region_code}", node.region.code())
            .replace("{protocol}", node.proxy.kind())
            .replace("{source}", &node.source_name)
            .replace("{name}", &node.original_name);
        if !rendered.trim().is_empty() {
            node.set_name(rendered);
        }
    }
    ensure_unique_names(nodes);
}

pub fn enabled_nodes(nodes: &[Node]) -> Vec<Node> {
    nodes.iter().filter(|node| node.enabled).cloned().collect()
}

pub fn group_nodes_by_region(nodes: &[Node], region: Region) -> Vec<&Node> {
    nodes.iter().filter(|node| node.region == region).collect()
}

pub fn proxies_to_clash_yaml(proxies: &[Proxy]) -> Result<String> {
    let mut root = Mapping::new();
    root.insert(
        Value::String("proxies".to_string()),
        Value::Sequence(proxies.iter().map(Proxy::to_value).collect()),
    );
    Ok(serde_yaml_ng::to_string(&Value::Mapping(root))?)
}

pub fn merge_manual_proxies_yaml(
    existing: Option<&str>,
    new_proxies: Vec<Proxy>,
) -> Result<(String, usize)> {
    let mut all = Vec::new();
    if let Some(existing) = existing.filter(|existing| !existing.trim().is_empty()) {
        all.extend(parse_content(existing)?.proxies);
    }
    all.extend(new_proxies);
    let (merged, _) = dedupe_proxies(all);
    let payload = proxies_to_clash_yaml(&merged)?;
    Ok((payload, merged.len()))
}
