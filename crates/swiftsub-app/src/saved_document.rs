//! Edit the saved YAML itself. Unowned fields stay in the document; generator options are never reapplied.
use serde::{Deserialize, Serialize};
use serde_yaml_ng::{Mapping, Value};
use std::collections::{HashMap, HashSet};
use swiftsub_core::{catalog, generator::RuleSetBinding};

pub fn text(value: &Value, key: &str) -> String {
    value
        .get(key)
        .and_then(Value::as_str)
        .unwrap_or_default()
        .into()
}
fn put(value: &mut Value, key: &str, data: impl Into<Value>) {
    value
        .as_mapping_mut()
        .expect("validated mapping")
        .insert(Value::String(key.into()), data.into());
}
fn strings(value: Option<&Value>) -> Vec<String> {
    value
        .and_then(Value::as_sequence)
        .map(|v| {
            v.iter()
                .filter_map(Value::as_str)
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default()
}
fn values(items: Vec<String>) -> Value {
    Value::Sequence(items.into_iter().map(Value::String).collect())
}
const POLICIES: &[&str] = &[
    "DIRECT",
    "REJECT",
    "REJECT-DROP",
    "PASS",
    "COMPATIBLE",
    "GLOBAL",
];

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct EditorMetadata {
    #[serde(default)]
    pub disabled_nodes: Vec<DisabledNode>,
    #[serde(default)]
    pub disabled_rules: Vec<DisabledRule>,
    #[serde(default)]
    pub disabled_groups: Vec<DisabledGroup>,
    #[serde(default)]
    pub node_sources: Vec<NodeSource>,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct DisabledNode {
    pub value: Value,
    pub index: usize,
    pub memberships: Vec<(String, usize)>,
    pub placeholders: Vec<String>,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct DisabledRule {
    pub index: usize,
    pub line: String,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct DisabledGroup {
    pub value: Value,
    pub index: usize,
    pub rules: Vec<(usize, String)>,
    pub memberships: Vec<(String, Vec<String>, Vec<String>)>,
    pub references: Vec<(String, String, String, String)>,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct NodeSource {
    pub source_id: i64,
    pub original_name: String,
    pub name: String,
}
#[derive(Clone, Debug)]
pub struct SavedDocument {
    pub root: Value,
    pub metadata: EditorMetadata,
}

/// Split logical expressions only at top-level commas, preserving the payload verbatim.
pub fn rule_parts(line: &str) -> Vec<&str> {
    let mut depth = 0i32;
    let mut start = 0;
    let mut parts = Vec::new();
    for (i, c) in line.char_indices() {
        match c {
            '(' => depth += 1,
            ')' => depth -= 1,
            ',' if depth == 0 => {
                parts.push(line[start..i].trim());
                start = i + 1;
            }
            _ => {}
        }
    }
    parts.push(line[start..].trim());
    parts
}
pub fn rule_target(line: &str) -> Option<&str> {
    let parts = rule_parts(line);
    if parts.len() < 2 || parts[0] == "SUB-RULE" {
        return None;
    }
    let index = parts.len() - 1 - usize::from(parts.last() == Some(&"no-resolve"));
    (index > 0).then_some(parts[index])
}
fn retarget(line: &str, target: &str) -> String {
    let mut parts = rule_parts(line);
    let index = parts.len() - 1 - usize::from(parts.last() == Some(&"no-resolve"));
    parts[index] = target;
    parts.join(",")
}

impl SavedDocument {
    pub fn parse(yaml: &str, source: Option<&str>) -> Result<Self, String> {
        let root: Value =
            serde_yaml_ng::from_str(yaml).map_err(|e| format!("YAML 格式错误：{e}"))?;
        if !root.is_mapping()
            || (!root.get("proxies").is_some() && root.get("proxy-providers").is_none())
        {
            return Err("配置需要 proxies 或 proxy-providers".into());
        }
        for key in ["proxies", "proxy-groups", "rules", "listeners"] {
            if root.get(key).is_some_and(|v| !v.is_sequence()) {
                return Err(format!("{key} 必须是列表"));
            }
        }
        for key in ["proxy-providers", "rule-providers", "dns"] {
            if root.get(key).is_some_and(|v| !v.is_mapping()) {
                return Err(format!("{key} 必须是对象"));
            }
        }
        for key in ["proxies", "proxy-groups"] {
            if let Some(items) = root.get(key).and_then(Value::as_sequence) {
                if items
                    .iter()
                    .any(|v| !v.is_mapping() || text(v, "name").is_empty())
                {
                    return Err(format!("{key} 中存在缺少名称的条目，请在 YAML 中修正"));
                }
            }
        }
        if root
            .get("rules")
            .and_then(Value::as_sequence)
            .is_some_and(|rules| rules.iter().any(|r| !r.is_string()))
        {
            return Err("rules 中必须是完整的规则文本".into());
        }
        let metadata = source
            .and_then(|s| serde_json::from_str::<serde_json::Value>(s).ok())
            .and_then(|s| s.get("visual_editor").cloned())
            .and_then(|s| serde_json::from_value(s).ok())
            .unwrap_or_default();
        Ok(Self { root, metadata })
    }
    pub fn source_json(&self, original: Option<&str>) -> Result<String, String> {
        let mut source = original
            .and_then(|s| serde_json::from_str::<serde_json::Value>(s).ok())
            .filter(serde_json::Value::is_object)
            .unwrap_or_else(|| serde_json::json!({}));
        if let Some(map) = source.as_object_mut() {
            map.remove("routing");
            map.remove("membership");
            map.remove("filename");
        }
        source["visual_editor"] =
            serde_json::to_value(&self.metadata).map_err(|e| e.to_string())?;
        serde_json::to_string(&source).map_err(|e| e.to_string())
    }
    pub fn yaml(&self) -> Result<String, String> {
        serde_yaml_ng::to_string(&self.root).map_err(|e| e.to_string())
    }
    pub fn items(&self, key: &str) -> Vec<Value> {
        self.root
            .get(key)
            .and_then(Value::as_sequence)
            .cloned()
            .unwrap_or_default()
    }
    pub fn names(&self, key: &str) -> Vec<String> {
        self.items(key).iter().map(|v| text(v, "name")).collect()
    }
    pub fn rules(&self) -> Vec<String> {
        strings(self.root.get("rules"))
    }
    pub fn group(&self, name: &str) -> Option<Value> {
        self.items("proxy-groups")
            .into_iter()
            .find(|g| text(g, "name") == name)
    }
    pub fn members(&self, name: &str) -> Vec<String> {
        self.group(name)
            .map(|g| strings(g.get("proxies")))
            .unwrap_or_default()
    }
    pub fn targets(&self) -> Vec<String> {
        let mut all = self.names("proxy-groups");
        all.extend(self.names("proxies"));
        all.extend(["DIRECT".into(), "REJECT".into(), "REJECT-DROP".into()]);
        all
    }
    pub fn set_items(&mut self, key: &str, items: Vec<Value>) {
        put(&mut self.root, key, Value::Sequence(items));
    }
    fn set_rules(&mut self, rules: Vec<String>) {
        put(&mut self.root, "rules", values(rules));
    }
    pub fn transaction(
        &mut self,
        action: impl FnOnce(&mut Self) -> Result<(), String>,
    ) -> Result<(), String> {
        let mut next = self.clone();
        action(&mut next)?;
        next.validate()?;
        *self = next;
        Ok(())
    }
    pub fn validate(&self) -> Result<(), String> {
        let mut names: HashSet<String> = POLICIES.iter().map(|s| s.to_string()).collect();
        for key in ["proxies", "proxy-groups"] {
            for name in self.names(key) {
                if name.trim() != name
                    || name.is_empty()
                    || name.contains([',', '\n', '\r'])
                    || !names.insert(name.clone())
                {
                    return Err(format!("名称无效或重复：{name}"));
                }
            }
        }
        let groups = self.items("proxy-groups");
        let group_names: HashSet<_> = self.names("proxy-groups").into_iter().collect();
        let mut graph = HashMap::<String, Vec<String>>::new();
        for group in groups {
            let name = text(&group, "name");
            let members = strings(group.get("proxies"));
            let dynamic = group
                .get("use")
                .and_then(Value::as_sequence)
                .is_some_and(|v| !v.is_empty())
                || [
                    "include-all",
                    "include-all-proxies",
                    "include-all-providers",
                ]
                .iter()
                .any(|k| group.get(*k).and_then(Value::as_bool) == Some(true));
            if members.is_empty() && !dynamic {
                return Err(format!("策略组「{name}」至少需要一个成员"));
            }
            for member in &members {
                if !names.contains(member) {
                    return Err(format!("策略组「{name}」引用不存在的成员：{member}"));
                }
            }
            graph.insert(
                name,
                members
                    .into_iter()
                    .filter(|m| group_names.contains(m))
                    .collect(),
            );
            for provider in strings(group.get("use")) {
                if self
                    .root
                    .get("proxy-providers")
                    .and_then(|v| v.get(&provider))
                    .is_none()
                {
                    return Err(format!("未知节点提供者：{provider}"));
                }
            }
        }
        fn visit(
            n: &str,
            graph: &HashMap<String, Vec<String>>,
            active: &mut HashSet<String>,
            done: &mut HashSet<String>,
        ) -> Result<(), String> {
            if done.contains(n) {
                return Ok(());
            }
            if !active.insert(n.into()) {
                return Err(format!("策略组循环引用：{n}"));
            }
            if let Some(edges) = graph.get(n) {
                for edge in edges {
                    visit(edge, graph, active, done)?;
                }
            }
            active.remove(n);
            done.insert(n.into());
            Ok(())
        }
        let mut done = HashSet::new();
        for n in graph.keys() {
            visit(n, &graph, &mut HashSet::new(), &mut done)?;
        }
        let rules = self.rules();
        let mut final_seen = false;
        for line in &rules {
            let parts = rule_parts(line);
            if parts.len() < 2 {
                return Err(format!("规则不完整：{line}"));
            }
            if final_seen {
                return Err("兜底规则必须在最后".into());
            }
            if let Some(target) = rule_target(line) {
                if !names.contains(target) {
                    return Err(format!("规则引用不存在的目标：{target}"));
                }
            }
            if parts[0] == "RULE-SET"
                && self
                    .root
                    .get("rule-providers")
                    .and_then(|v| v.get(parts[1]))
                    .is_none()
            {
                return Err(format!("规则集未定义：{}", parts[1]));
            }
            final_seen = matches!(parts[0], "MATCH" | "FINAL");
        }
        for node in self.items("proxies") {
            let dialer = text(&node, "dialer-proxy");
            if !dialer.is_empty() && !names.contains(&dialer) {
                return Err(format!("节点引用不存在的上游：{dialer}"));
            }
        }
        for listener in self.items("listeners") {
            let proxy = text(&listener, "proxy");
            if !proxy.is_empty() && !names.contains(&proxy) {
                return Err(format!("监听引用不存在的目标：{proxy}"));
            }
        }
        Ok(())
    }
    pub fn rename(&mut self, old: &str, new: &str) -> Result<(), String> {
        let new = new.trim();
        if old == new {
            return Ok(());
        }
        if new.is_empty()
            || new.contains([',', '\n', '\r'])
            || self.targets().iter().any(|n| n == new)
            || self
                .metadata
                .disabled_groups
                .iter()
                .any(|g| text(&g.value, "name") == new)
            || self
                .metadata
                .disabled_nodes
                .iter()
                .any(|n| text(&n.value, "name") == new)
        {
            return Err("名称不能为空、重复或包含逗号".into());
        }
        for key in ["proxy-groups", "proxies", "listeners"] {
            let mut items = self.items(key);
            for item in &mut items {
                if key != "listeners" && text(item, "name") == old {
                    put(item, "name", new);
                }
                if key == "proxy-groups" {
                    let members = strings(item.get("proxies"))
                        .into_iter()
                        .map(|m| if m == old { new.into() } else { m })
                        .collect();
                    if item.get("proxies").is_some() {
                        put(item, "proxies", values(members));
                    }
                }
                for field in ["dialer-proxy", "proxy"] {
                    if text(item, field) == old {
                        put(item, field, new);
                    }
                }
            }
            if self.root.get(key).is_some() {
                self.set_items(key, items);
            }
        }
        self.set_rules(
            self.rules()
                .iter()
                .map(|r| {
                    if rule_target(r) == Some(old) {
                        retarget(r, new)
                    } else {
                        r.clone()
                    }
                })
                .collect(),
        );
        for node in &mut self.metadata.disabled_nodes {
            for (group, _) in &mut node.memberships {
                if group == old {
                    *group = new.into();
                }
            }
            for group in &mut node.placeholders {
                if group == old {
                    *group = new.into();
                }
            }
        }
        for rule in &mut self.metadata.disabled_rules {
            if rule_target(&rule.line) == Some(old) {
                rule.line = retarget(&rule.line, new);
            }
        }
        for origin in &mut self.metadata.node_sources {
            if origin.name == old {
                origin.name = new.into();
            }
        }
        for disabled in &mut self.metadata.disabled_groups {
            let mut members = strings(disabled.value.get("proxies"));
            for m in &mut members {
                if m == old {
                    *m = new.into();
                }
            }
            if disabled.value.get("proxies").is_some() {
                put(&mut disabled.value, "proxies", values(members));
            }
            for (_, line) in &mut disabled.rules {
                if rule_target(line) == Some(old) {
                    *line = retarget(line, new);
                }
            }
            for (name, before, after) in &mut disabled.memberships {
                if name == old {
                    *name = new.into();
                }
                for member in before.iter_mut().chain(after.iter_mut()) {
                    if member == old {
                        *member = new.into();
                    }
                }
            }
            for (_, name, _, replacement) in &mut disabled.references {
                if name == old {
                    *name = new.into();
                }
                if replacement == old {
                    *replacement = new.into();
                }
            }
        }
        Ok(())
    }
    pub fn update_group(
        &mut self,
        old: &str,
        name: &str,
        kind: &str,
        url: &str,
        interval: &str,
    ) -> Result<(), String> {
        if !["select", "url-test", "fallback", "load-balance"].contains(&kind) {
            return Err("请选择支持的策略组类型".into());
        }
        self.rename(old, name)?;
        let mut groups = self.items("proxy-groups");
        let g = groups
            .iter_mut()
            .find(|g| text(g, "name") == name.trim())
            .ok_or("策略组不存在")?;
        put(g, "type", kind);
        if kind != "select" {
            let parsed = url::Url::parse(url.trim()).map_err(|_| "测速地址无效")?;
            if !["http", "https"].contains(&parsed.scheme()) {
                return Err("测速地址必须是 HTTP / HTTPS".into());
            }
            let seconds: u32 = interval.trim().parse().map_err(|_| "测速间隔需要正整数")?;
            if seconds == 0 {
                return Err("测速间隔需要大于 0".into());
            }
            put(g, "url", url.trim());
            put(g, "interval", Value::from(seconds));
        }
        self.set_items("proxy-groups", groups);
        Ok(())
    }
    pub fn unique_name(&self, base: &str) -> String {
        let mut names = self.targets();
        names.extend(
            self.metadata
                .disabled_nodes
                .iter()
                .map(|n| text(&n.value, "name")),
        );
        names.extend(
            self.metadata
                .disabled_groups
                .iter()
                .map(|n| text(&n.value, "name")),
        );
        let mut n = base.to_string();
        let mut i = 2;
        while names.contains(&n) {
            n = format!("{base} {i}");
            i += 1;
        }
        n
    }
    pub fn add_group(&mut self, name: &str, copy: Option<&str>) -> Result<String, String> {
        let name = self.unique_name(name);
        let mut group = copy.and_then(|n| self.group(n)).unwrap_or_else(|| {
            serde_yaml_ng::from_str("name: new\ntype: select\nproxies: [DIRECT]\n").unwrap()
        });
        put(&mut group, "name", name.clone());
        let mut groups = self.items("proxy-groups");
        groups.push(group);
        self.set_items("proxy-groups", groups);
        Ok(name)
    }
    pub fn remove_group(&mut self, name: &str) -> Result<(), String> {
        if ["节点选择", "自动选择", "漏网之鱼"].contains(&name) {
            return Err("基础策略组需要保留".into());
        }
        if self.group(name).is_none() {
            return Err("策略组不存在".into());
        }
        let fallback = if self.group("节点选择").is_some() {
            "节点选择"
        } else {
            "DIRECT"
        };
        self.set_rules(
            self.rules()
                .into_iter()
                .filter_map(|r| {
                    if rule_target(&r) != Some(name) {
                        Some(r)
                    } else if matches!(rule_parts(&r).first(), Some(&"MATCH" | &"FINAL")) {
                        Some(retarget(&r, fallback))
                    } else {
                        None
                    }
                })
                .collect(),
        );
        self.metadata
            .disabled_rules
            .retain(|r| rule_target(&r.line) != Some(name));
        let mut groups = self.items("proxy-groups");
        groups.retain(|g| text(g, "name") != name);
        for group in &mut groups {
            let mut members = strings(group.get("proxies"));
            for member in &mut members {
                if member == name {
                    *member = "DIRECT".into();
                }
            }
            let mut seen = HashSet::new();
            members.retain(|m| seen.insert(m.clone()));
            if group.get("proxies").is_some() {
                put(group, "proxies", values(members));
            }
        }
        self.set_items("proxy-groups", groups);
        for key in ["proxies", "listeners"] {
            let mut items = self.items(key);
            for item in &mut items {
                for field in ["dialer-proxy", "proxy"] {
                    if text(item, field) == name {
                        put(item, field, fallback);
                    }
                }
            }
            if self.root.get(key).is_some() {
                self.set_items(key, items);
            }
        }
        for node in &mut self.metadata.disabled_nodes {
            node.memberships.retain(|(group, _)| group != name);
            node.placeholders.retain(|group| group != name);
        }
        Ok(())
    }
    pub fn member(&mut self, group: &str, member: &str, enabled: bool) -> Result<(), String> {
        if group == member {
            return Err("策略组不能引用自身".into());
        }
        let mut groups = self.items("proxy-groups");
        let g = groups
            .iter_mut()
            .find(|g| text(g, "name") == group)
            .ok_or("组不存在")?;
        let mut members = strings(g.get("proxies"));
        if enabled {
            if !members.iter().any(|m| m == member) {
                members.push(member.into());
            }
        } else {
            members.retain(|m| m != member);
        }
        put(g, "proxies", values(members));
        self.set_items("proxy-groups", groups);
        Ok(())
    }
    pub fn move_item(&mut self, key: &str, from: usize, to: usize) -> Result<(), String> {
        let mut items = self.items(key);
        if from >= items.len() || to >= items.len() {
            return Err("位置已改变，请重试".into());
        }
        let item = items.remove(from);
        items.insert(to, item);
        self.set_items(key, items);
        Ok(())
    }
    pub fn move_member(&mut self, group: &str, from: usize, to: usize) -> Result<(), String> {
        let mut groups = self.items("proxy-groups");
        let g = groups
            .iter_mut()
            .find(|g| text(g, "name") == group)
            .ok_or("组不存在")?;
        let mut members = strings(g.get("proxies"));
        if from >= members.len() || to >= members.len() {
            return Err("成员位置已改变".into());
        }
        let m = members.remove(from);
        members.insert(to, m);
        put(g, "proxies", values(members));
        self.set_items("proxy-groups", groups);
        Ok(())
    }
    pub fn toggle_node(&mut self, name: &str, enabled: bool) -> Result<(), String> {
        if enabled {
            let pos = self
                .metadata
                .disabled_nodes
                .iter()
                .position(|n| text(&n.value, "name") == name)
                .ok_or("找不到停用节点")?;
            let saved = self.metadata.disabled_nodes.remove(pos);
            let mut proxies = self.items("proxies");
            proxies.insert(saved.index.min(proxies.len()), saved.value);
            self.set_items("proxies", proxies);
            let mut groups = self.items("proxy-groups");
            for group in &mut groups {
                let group_name = text(group, "name");
                let mut members = strings(group.get("proxies"));
                if let Some((_, index)) = saved.memberships.iter().find(|(n, _)| n == &group_name) {
                    if saved.placeholders.contains(&group_name) && members == ["DIRECT"] {
                        members.clear();
                    }
                    if !members.iter().any(|n| n == name) {
                        members.insert((*index).min(members.len()), name.into());
                    }
                    put(group, "proxies", values(members));
                }
            }
            self.set_items("proxy-groups", groups);
            for disabled in &mut self.metadata.disabled_groups {
                let group = &mut disabled.value;
                let group_name = text(group, "name");
                let mut members = strings(group.get("proxies"));
                if let Some((_, index)) = saved.memberships.iter().find(|(n, _)| n == &group_name) {
                    if saved.placeholders.contains(&group_name) && members == ["DIRECT"] {
                        members.clear();
                    }
                    if !members.iter().any(|m| m == name) {
                        members.insert((*index).min(members.len()), name.into());
                    }
                    put(group, "proxies", values(members));
                }
            }
        } else {
            if self.rules().iter().any(|r| rule_target(r) == Some(name))
                || self
                    .items("proxies")
                    .iter()
                    .any(|p| text(p, "dialer-proxy") == name)
                || self
                    .items("listeners")
                    .iter()
                    .any(|p| text(p, "proxy") == name)
            {
                return Err("此节点被规则、链路或监听直接引用，请先修改目标".into());
            }
            let mut proxies = self.items("proxies");
            let index = proxies
                .iter()
                .position(|p| text(p, "name") == name)
                .ok_or("节点不存在")?;
            let value = proxies.remove(index);
            self.set_items("proxies", proxies);
            let mut memberships = Vec::new();
            let mut placeholders = Vec::new();
            let mut groups = self.items("proxy-groups");
            for g in &mut groups {
                let group_name = text(g, "name");
                let mut members = strings(g.get("proxies"));
                if let Some(index) = members.iter().position(|n| n == name) {
                    memberships.push((group_name.clone(), index));
                    members.retain(|m| m != name);
                    if members.is_empty() {
                        members.push("DIRECT".into());
                        placeholders.push(group_name);
                    }
                    put(g, "proxies", values(members));
                }
            }
            self.set_items("proxy-groups", groups);
            for disabled in &mut self.metadata.disabled_groups {
                let group = &mut disabled.value;
                let group_name = text(group, "name");
                let mut members = strings(group.get("proxies"));
                if let Some(index) = members.iter().position(|m| m == name) {
                    memberships.push((group_name.clone(), index));
                    members.retain(|m| m != name);
                    if members.is_empty() {
                        members.push("DIRECT".into());
                        placeholders.push(group_name);
                    }
                    put(group, "proxies", values(members));
                }
            }
            self.metadata.disabled_nodes.push(DisabledNode {
                value,
                index,
                memberships,
                placeholders,
            });
        }
        Ok(())
    }
    pub fn set_rule_enabled(&mut self, index: usize, enabled: bool) -> Result<(), String> {
        let mut rules = self.rules();
        if enabled {
            let saved = self
                .metadata
                .disabled_rules
                .get(index)
                .cloned()
                .ok_or("停用规则不存在")?;
            rules.insert(
                saved.index.min(rules.len().saturating_sub(usize::from(
                    rules.last().is_some_and(|r| {
                        matches!(rule_parts(r).first(), Some(&"MATCH" | &"FINAL"))
                    }),
                ))),
                saved.line,
            );
            self.metadata.disabled_rules.remove(index);
        } else {
            let line = rules.get(index).ok_or("规则不存在")?.clone();
            if matches!(rule_parts(&line).first(), Some(&"MATCH" | &"FINAL")) {
                return Err("兜底规则需要保留，可修改其目标".into());
            }
            rules.remove(index);
            self.metadata
                .disabled_rules
                .push(DisabledRule { index, line });
        }
        self.set_rules(rules);
        Ok(())
    }
    pub fn set_rule_target(&mut self, index: usize, target: &str) -> Result<(), String> {
        let mut rules = self.rules();
        let line = rules.get_mut(index).ok_or("规则不存在")?;
        if rule_target(line).is_none() {
            return Err("此规则请在 YAML 中编辑".into());
        }
        *line = retarget(line, target);
        self.set_rules(rules);
        Ok(())
    }
    pub fn add_rule(&mut self, line: &str) -> Result<(), String> {
        let line = line.trim();
        if rule_target(line).is_none() {
            return Err("请输入包含目标策略的完整规则".into());
        }
        let mut rules = self.rules();
        if matches!(rule_parts(line).first(), Some(&"MATCH" | &"FINAL")) {
            rules.retain(|r| !matches!(rule_parts(r).first(), Some(&"MATCH" | &"FINAL")));
            rules.push(line.into());
        } else {
            let pos = rules
                .iter()
                .position(|r| matches!(rule_parts(r).first(), Some(&"MATCH" | &"FINAL")))
                .unwrap_or(rules.len());
            rules.insert(pos, line.into());
        }
        self.set_rules(rules);
        Ok(())
    }
    pub fn catalog_rule_enabled(&self, id: &str) -> bool {
        self.rules().iter().any(|line| {
            let p = rule_parts(line);
            p.first() == Some(&"RULE-SET") && p.get(1) == Some(&id)
        })
    }
    pub fn catalog_rule_target(&self, id: &str) -> Option<String> {
        self.rules().iter().find_map(|line| {
            let p = rule_parts(line);
            if p.first() == Some(&"RULE-SET") && p.get(1) == Some(&id) {
                rule_target(line).map(str::to_string)
            } else {
                None
            }
        })
    }
    pub fn set_scoped_binding(
        &mut self,
        binding: &RuleSetBinding,
        enabled: bool,
        scope: Option<&str>,
    ) -> Result<(), String> {
        if !enabled && scope.is_some() {
            let scope = scope.unwrap();
            let rules = self
                .rules()
                .into_iter()
                .filter(|line| {
                    let p = rule_parts(line);
                    !(p.first() == Some(&"RULE-SET")
                        && p.get(1) == Some(&binding.id.as_str())
                        && rule_target(line) == Some(scope))
                })
                .collect();
            self.set_rules(rules);
            return Ok(());
        }
        self.add_binding(binding, enabled)?;
        if enabled && scope.is_some() {
            let mut rules = self.rules();
            for line in &mut rules {
                let p = rule_parts(line);
                if p.first() == Some(&"RULE-SET") && p.get(1) == Some(&binding.id.as_str()) {
                    *line = retarget(line, scope.unwrap());
                }
            }
            self.set_rules(rules);
        }
        Ok(())
    }
    pub fn add_binding(&mut self, binding: &RuleSetBinding, enabled: bool) -> Result<(), String> {
        if !enabled {
            let mut rules = self.rules();
            rules.retain(|line| {
                let p = rule_parts(line);
                !(p.first() == Some(&"RULE-SET") && p.get(1) == Some(&binding.id.as_str()))
            });
            self.set_rules(rules);
            return Ok(());
        }
        self.metadata.disabled_rules.retain(|r| {
            let p = rule_parts(&r.line);
            !(p.first() == Some(&"RULE-SET") && p.get(1) == Some(&binding.id.as_str()))
        });
        self.ensure_target(&binding.target)?;
        let mut providers = self
            .root
            .get("rule-providers")
            .and_then(Value::as_mapping)
            .cloned()
            .unwrap_or_default();
        let provider_key = Value::String(binding.id.clone());
        if let Some(existing) = providers.get(&provider_key) {
            if text(existing, "url") != binding.url
                || text(existing, "behavior") != binding.behavior
                || text(existing, "format") != binding.format
            {
                return Err(format!(
                    "规则集 ID「{}」已被不同内容占用，请先修改自定义规则集 ID",
                    binding.id
                ));
            }
        } else {
            let mut spec = Mapping::new();
            for (k, v) in [
                ("type", "http"),
                ("behavior", binding.behavior.as_str()),
                ("format", binding.format.as_str()),
                ("url", binding.url.as_str()),
            ] {
                spec.insert(Value::String(k.into()), Value::String(v.into()));
            }
            spec.insert(Value::String("interval".into()), Value::from(86400));
            spec.insert(
                Value::String("path".into()),
                Value::String(format!("./rules/{}.{}", binding.id, binding.format)),
            );
            providers.insert(provider_key, Value::Mapping(spec));
        }
        put(&mut self.root, "rule-providers", Value::Mapping(providers));
        if !self.catalog_rule_enabled(&binding.id) {
            let line = swiftsub_core::model::Rule::rule_set(
                &binding.id,
                &binding.target,
                binding.no_resolve,
            )
            .to_line();
            let rank = rule_rank(&line);
            let mut rules = self.rules();
            let pos = rules
                .iter()
                .position(|r| rule_rank(r) > rank)
                .unwrap_or(rules.len());
            rules.insert(pos, line);
            self.set_rules(rules);
        }
        Ok(())
    }
    fn ensure_target(&mut self, name: &str) -> Result<(), String> {
        if self
            .metadata
            .disabled_groups
            .iter()
            .any(|g| text(&g.value, "name") == name)
        {
            return self.toggle_group(name, true);
        }
        if self.targets().iter().any(|n| n == name) {
            return Ok(());
        }
        let module = catalog::MODULES
            .iter()
            .find(|m| m.name == name)
            .ok_or_else(|| format!("目标不存在：{name}"))?;
        let kind = if module.id == "auto" {
            "url-test"
        } else {
            "select"
        };
        let members = if module.id == "select" || module.id == "auto" {
            let names = self.names("proxies");
            if names.is_empty() {
                vec!["DIRECT".into()]
            } else {
                names
            }
        } else if matches!(module.kind, catalog::ModuleKind::DirectFirst) {
            vec!["DIRECT".into()]
        } else if matches!(module.kind, catalog::ModuleKind::RejectFirst) {
            vec!["REJECT".into(), "DIRECT".into()]
        } else {
            self.ensure_target("节点选择")?;
            vec!["节点选择".into(), "DIRECT".into()]
        };
        let mut group = Value::Mapping(Mapping::new());
        put(&mut group, "name", name);
        put(&mut group, "type", kind);
        put(&mut group, "proxies", values(members));
        if kind == "url-test" {
            put(&mut group, "url", "https://www.gstatic.com/generate_204");
            put(&mut group, "interval", Value::from(300));
        }
        let mut groups = self.items("proxy-groups");
        groups.push(group);
        self.set_items("proxy-groups", groups);
        Ok(())
    }
    pub fn reset_group(&mut self, name: &str) -> Result<(), String> {
        let mut defaults = SavedDocument {
            root: Value::Mapping(Mapping::new()),
            metadata: EditorMetadata::default(),
        };
        defaults.set_items("proxies", self.items("proxies"));
        defaults.ensure_target(name)?;
        let reset = defaults.group(name).ok_or("该组没有内置默认值")?;
        let mut groups = self.items("proxy-groups");
        let group = groups
            .iter_mut()
            .find(|g| text(g, "name") == name)
            .ok_or("组不存在")?;
        // Restore editable policy fields without removing provider filters or extension fields.
        for key in ["type", "proxies", "url", "interval"] {
            if let Some(v) = reset.get(key) {
                put(group, key, v.clone());
            }
        }
        self.set_items("proxy-groups", groups);
        if self.group("节点选择").is_none()
            && name != "节点选择"
            && self.members(name).iter().any(|m| m == "节点选择")
        {
            self.ensure_target("节点选择")?;
        }
        Ok(())
    }
    pub fn update_node(
        &mut self,
        old: &str,
        name: &str,
        server: &str,
        port: &str,
    ) -> Result<(), String> {
        let port: u16 = port.trim().parse().map_err(|_| "端口需为 1–65535")?;
        if port == 0 || server.trim().is_empty() {
            return Err("请填写节点地址和有效端口".into());
        }
        self.rename(old, name)?;
        let mut nodes = self.items("proxies");
        let node = nodes
            .iter_mut()
            .find(|n| text(n, "name") == name.trim())
            .ok_or("节点不存在")?;
        put(node, "server", server.trim());
        put(node, "port", Value::from(port));
        self.set_items("proxies", nodes);
        Ok(())
    }
    pub fn toggle_group(&mut self, name: &str, enabled: bool) -> Result<(), String> {
        if enabled {
            let index = self
                .metadata
                .disabled_groups
                .iter()
                .position(|g| text(&g.value, "name") == name)
                .ok_or("没有这份停用策略组")?;
            let saved = self.metadata.disabled_groups.remove(index);
            let mut groups = self.items("proxy-groups");
            if self.targets().iter().any(|n| n == name) {
                return Err("同名组或节点已存在".into());
            }
            groups.insert(saved.index.min(groups.len()), saved.value);
            for (group, before, after) in &saved.memberships {
                if let Some(g) = groups.iter_mut().find(|g| text(g, "name") == *group) {
                    let current = strings(g.get("proxies"));
                    if &current == after {
                        put(g, "proxies", values(before.clone()));
                    } else if !current.iter().any(|n| n == name) {
                        let mut current = current;
                        current.push(name.into());
                        put(g, "proxies", values(current));
                    }
                }
            }
            self.set_items("proxy-groups", groups);
            let mut rules = self.rules();
            for (index, line) in saved.rules {
                if !rules.contains(&line) {
                    let pos = index.min(
                        rules
                            .iter()
                            .position(|r| {
                                matches!(rule_parts(r).first(), Some(&"MATCH" | &"FINAL"))
                            })
                            .unwrap_or(rules.len()),
                    );
                    rules.insert(pos, line);
                }
            }
            self.set_rules(rules);
            for (section, entry, field, replacement) in saved.references {
                let mut items = self.items(&section);
                for item in &mut items {
                    if text(item, "name") == entry && text(item, &field) == replacement {
                        put(item, &field, name);
                    }
                }
                self.set_items(&section, items);
            }
        } else {
            if ["节点选择", "自动选择", "漏网之鱼"].contains(&name)
                || self.rules().iter().any(|r| {
                    matches!(rule_parts(r).first(), Some(&"MATCH" | &"FINAL"))
                        && rule_target(r) == Some(name)
                })
            {
                return Err("基础与最终兜底策略组需要保留".into());
            }
            let groups = self.items("proxy-groups");
            let index = groups
                .iter()
                .position(|g| text(g, "name") == name)
                .ok_or("策略组不存在")?;
            let value = groups[index].clone();
            let rules = self
                .rules()
                .into_iter()
                .enumerate()
                .filter(|(_, r)| rule_target(r) == Some(name))
                .collect();
            let before = groups
                .iter()
                .filter_map(|g| {
                    let members = strings(g.get("proxies"));
                    members
                        .iter()
                        .any(|m| m == name)
                        .then(|| (text(g, "name"), members))
                })
                .collect::<Vec<_>>();
            let fallback = if self.group("节点选择").is_some() {
                "节点选择"
            } else {
                "DIRECT"
            }
            .to_string();
            let mut references = Vec::new();
            for section in ["proxies", "listeners"] {
                for item in self.items(section) {
                    for field in ["dialer-proxy", "proxy"] {
                        if text(&item, field) == name {
                            references.push((
                                section.into(),
                                text(&item, "name"),
                                field.into(),
                                fallback.clone(),
                            ));
                        }
                    }
                }
            }
            self.remove_group(name)?;
            let memberships = before
                .into_iter()
                .map(|(name, prior)| {
                    let after = self.members(&name);
                    (name, prior, after)
                })
                .collect();
            self.metadata.disabled_groups.push(DisabledGroup {
                value,
                index,
                rules,
                memberships,
                references,
            });
        }
        Ok(())
    }
    pub fn sync_source_nodes(
        &mut self,
        source_id: i64,
        nodes: Vec<Value>,
        replace: bool,
    ) -> Result<(), String> {
        let incoming: HashSet<_> = nodes.iter().map(|n| text(n, "name")).collect();
        if replace {
            let missing = self
                .metadata
                .node_sources
                .iter()
                .filter(|s| s.source_id == source_id && !incoming.contains(&s.original_name))
                .map(|s| s.name.clone())
                .collect::<Vec<_>>();
            for name in &missing {
                if self.names("proxies").contains(name) {
                    self.toggle_node(name, false)?;
                }
                self.metadata
                    .disabled_nodes
                    .retain(|n| text(&n.value, "name") != *name);
            }
            self.metadata
                .node_sources
                .retain(|s| !(s.source_id == source_id && missing.contains(&s.name)));
        }
        for mut node in nodes {
            let original = text(&node, "name");
            if let Some(origin) = self
                .metadata
                .node_sources
                .iter()
                .find(|s| s.source_id == source_id && s.original_name == original)
                .cloned()
            {
                put(&mut node, "name", origin.name.clone());
                if let Some(disabled) = self
                    .metadata
                    .disabled_nodes
                    .iter_mut()
                    .find(|n| text(&n.value, "name") == origin.name)
                {
                    disabled.value = node;
                } else {
                    let mut proxies = self.items("proxies");
                    if let Some(existing) =
                        proxies.iter_mut().find(|p| text(p, "name") == origin.name)
                    {
                        *existing = node;
                    } else {
                        proxies.push(node);
                    }
                    self.set_items("proxies", proxies);
                }
            } else {
                let identity = swiftsub_core::model::Proxy::from_value(node.clone())
                    .ok()
                    .map(|p| p.identity_key());
                let existing = self.items("proxies").into_iter().find(|candidate| {
                    text(candidate, "name") == original
                        && !self
                            .metadata
                            .node_sources
                            .iter()
                            .any(|s| s.name == original)
                        && identity.as_ref().is_some_and(|id| {
                            swiftsub_core::model::Proxy::from_value(candidate.clone())
                                .is_ok_and(|p| p.identity_key() == *id)
                        })
                });
                if existing.is_some() {
                    self.metadata.node_sources.push(NodeSource {
                        source_id,
                        original_name: original.clone(),
                        name: original,
                    });
                    continue;
                }
                let name = self.unique_name(&original);
                put(&mut node, "name", name.clone());
                let mut proxies = self.items("proxies");
                proxies.push(node);
                self.set_items("proxies", proxies);
                self.metadata.node_sources.push(NodeSource {
                    source_id,
                    original_name: original,
                    name: name.clone(),
                });
                for group in ["节点选择", "自动选择"] {
                    if self.group(group).is_some() {
                        self.member(group, &name, true)?;
                    }
                }
            }
        }
        Ok(())
    }
    pub fn set_basic(
        &mut self,
        port: &str,
        mode: &str,
        level: &str,
        lan: bool,
        dns: bool,
    ) -> Result<(), String> {
        let port: u16 = port.trim().parse().map_err(|_| "监听端口需为 1–65535")?;
        if port == 0 {
            return Err("端口不能为 0".into());
        }
        put(&mut self.root, "mixed-port", Value::from(port));
        put(&mut self.root, "mode", mode);
        put(&mut self.root, "log-level", level);
        put(&mut self.root, "allow-lan", lan);
        let mut config = self
            .root
            .get("dns")
            .cloned()
            .unwrap_or(Value::Mapping(Mapping::new()));
        put(&mut config, "enable", dns);
        put(&mut self.root, "dns", config);
        Ok(())
    }
}
fn rule_rank(line: &str) -> u16 {
    let p = rule_parts(line);
    if matches!(p.first(), Some(&"MATCH" | &"FINAL")) {
        return 1000;
    }
    let target = rule_target(line).unwrap_or_default();
    let rank = catalog::MODULES
        .iter()
        .find(|m| m.name == target)
        .map(catalog::rule_priority)
        .unwrap_or(400);
    if rank == 800 && p.last() == Some(&"no-resolve") {
        900
    } else {
        rank
    }
}
#[cfg(test)]
mod tests {
    use super::*;
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
        assert!(d.transaction(|d| d.remove_group("漏网之鱼")).is_err());
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
        assert!(
            restored
                .transaction(|d| d.toggle_group("漏网之鱼", false))
                .is_err()
        );
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
        let mut restored =
            SavedDocument::parse(&current.yaml, current.source_json.as_deref()).unwrap();
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
}
