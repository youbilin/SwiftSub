//! Edit the saved YAML itself. Unowned fields stay in the document; generator options are never reapplied.
#[cfg(test)]
mod tests;
mod validate;

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
pub fn unsigned(value: Option<&Value>) -> Option<u64> {
    match value? {
        Value::Number(number) => number.as_u64(),
        Value::String(value) => value.trim().parse().ok(),
        _ => None,
    }
}
fn put(value: &mut Value, key: &str, data: impl Into<Value>) {
    if let Some(map) = value.as_mapping_mut() {
        map.insert(Value::String(key.into()), data.into());
    }
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
type IndexedRules = Vec<(usize, String)>;

fn merge_local_node_fields(incoming: &mut Value, existing: &Value, baseline: Option<&Value>) {
    let (Some(incoming), Some(existing)) = (incoming.as_mapping_mut(), existing.as_mapping())
    else {
        return;
    };
    let baseline = baseline.and_then(Value::as_mapping);
    let mut keys: HashSet<Value> = existing.keys().cloned().collect();
    if let Some(baseline) = baseline {
        keys.extend(baseline.keys().cloned());
    }
    for key in keys {
        if key.as_str() == Some("name") {
            continue;
        }
        let local = existing.get(&key);
        let changed = baseline.is_some_and(|baseline| local != baseline.get(&key));
        if changed || (!incoming.contains_key(&key) && local.is_some()) {
            if let Some(value) = local {
                incoming.insert(key, value.clone());
            } else {
                incoming.remove(&key);
            }
        }
    }
}

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
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub chain_exits: HashMap<String, String>,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct DisabledNode {
    pub value: Value,
    pub index: usize,
    pub memberships: Vec<(String, usize)>,
    pub placeholders: Vec<String>,
    #[serde(default)]
    pub rules: Vec<(usize, String)>,
    #[serde(default)]
    pub rule_replacements: Vec<(usize, String)>,
    #[serde(default)]
    pub references: Vec<(String, String, String, String)>,
    #[serde(default)]
    pub sub_rules: Vec<(String, usize, String, String)>,
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
    #[serde(default)]
    pub rule_replacements: Vec<(usize, String)>,
    #[serde(default)]
    pub sub_rules: Vec<(String, usize, String, String)>,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct NodeSource {
    pub source_id: i64,
    pub original_name: String,
    pub name: String,
    #[serde(default)]
    pub original: Option<Value>,
}
#[derive(Clone, Debug)]
pub struct SavedDocument {
    pub root: Value,
    pub metadata: EditorMetadata,
}

/// Split logical expressions only at top-level commas, preserving the payload verbatim.
pub fn rule_parts(line: &str) -> Vec<&str> {
    swiftsub_core::Rule::parts(line)
}
pub fn rule_target(line: &str) -> Option<&str> {
    let parts = rule_parts(line);
    let kind = parts.first()?;
    if kind.eq_ignore_ascii_case("SUB-RULE") {
        return None;
    }
    let index = if kind.eq_ignore_ascii_case("MATCH") || kind.eq_ignore_ascii_case("FINAL") {
        1
    } else {
        2
    };
    parts.get(index).copied()
}
fn retarget(line: &str, target: &str) -> String {
    let mut parts = rule_parts(line);
    let index = if parts.first().is_some_and(|kind| {
        kind.eq_ignore_ascii_case("MATCH") || kind.eq_ignore_ascii_case("FINAL")
    }) {
        1
    } else {
        2
    };
    if index < parts.len() {
        parts[index] = target;
    }
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
        for key in ["proxy-providers", "rule-providers", "dns", "sub-rules"] {
            if root.get(key).is_some_and(|v| !v.is_mapping()) {
                return Err(format!("{key} 必须是对象"));
            }
        }
        for key in ["proxies", "proxy-groups"] {
            if let Some(items) = root.get(key).and_then(Value::as_sequence)
                && items
                    .iter()
                    .any(|v| !v.is_mapping() || text(v, "name").is_empty())
            {
                return Err(format!("{key} 中存在缺少名称的条目，请在 YAML 中修正"));
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
    pub fn editable_group(&self, name: &str) -> Option<Value> {
        self.group(name).or_else(|| {
            self.metadata
                .disabled_groups
                .iter()
                .find(|group| text(&group.value, "name") == name)
                .map(|group| group.value.clone())
        })
    }
    pub fn editable_node(&self, name: &str) -> Option<Value> {
        self.items("proxies")
            .into_iter()
            .find(|node| text(node, "name") == name)
            .or_else(|| {
                self.metadata
                    .disabled_nodes
                    .iter()
                    .find(|node| text(&node.value, "name") == name)
                    .map(|node| node.value.clone())
            })
    }
    pub fn members(&self, name: &str) -> Vec<String> {
        self.editable_group(name)
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
            if text(&node.value, "name") == old {
                put(&mut node.value, "name", new);
            }
            for field in ["dialer-proxy", "proxy"] {
                if text(&node.value, field) == old {
                    put(&mut node.value, field, new);
                }
            }
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
            for (_, entry, _, replacement) in &mut node.references {
                if entry == old {
                    *entry = new.into();
                }
                if replacement == old {
                    *replacement = new.into();
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
        if let Some(exit) = self.metadata.chain_exits.remove(old) {
            self.metadata.chain_exits.insert(new.into(), exit);
        }
        for exit in self.metadata.chain_exits.values_mut() {
            if exit == old {
                *exit = new.into();
            }
        }
        for disabled in &mut self.metadata.disabled_groups {
            if text(&disabled.value, "name") == old {
                put(&mut disabled.value, "name", new);
            }
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
        self.retarget_references_to(old, new, new)
    }
    /// Redirect references before removing a node, including saved disabled-state references.
    #[cfg(test)]
    pub fn replace_node_references(&mut self, old: &str, new: &str) -> Result<(), String> {
        if old == new || !self.names("proxies").contains(&new.to_string()) {
            return Err("请选择另一个启用的替代节点".into());
        }
        self.retarget_references_to(old, new, new)
    }
    fn retarget_references_to(
        &mut self,
        old: &str,
        new: &str,
        member_target: &str,
    ) -> Result<(), String> {
        fn replace_value(value: &mut Value, old: &str, new: &str, member_target: &str) {
            if let Some(members) = value.get("proxies").and_then(Value::as_sequence) {
                let mut seen = HashSet::new();
                let members = members
                    .iter()
                    .cloned()
                    .map(|member| {
                        if member.as_str() == Some(old) {
                            Value::from(member_target)
                        } else {
                            member
                        }
                    })
                    .filter(|member| seen.insert(member.clone()))
                    .collect();
                put(value, "proxies", Value::Sequence(members));
            }
            for field in ["dialer-proxy", "proxy"] {
                if text(value, field) == old {
                    put(
                        value,
                        field,
                        if field == "dialer-proxy" && member_target == "DIRECT" {
                            "DIRECT"
                        } else {
                            new
                        },
                    );
                }
            }
        }
        for section in ["proxy-groups", "proxies", "listeners", "tunnels"] {
            let mut items = self.items(section);
            for item in &mut items {
                replace_value(item, old, new, member_target);
            }
            if self.root.get(section).is_some() {
                self.set_items(section, items);
            }
        }
        for section in ["proxy-providers", "rule-providers"] {
            if let Some(providers) = self.root.get_mut(section).and_then(Value::as_mapping_mut) {
                for value in providers.values_mut() {
                    replace_value(value, old, new, member_target);
                }
            }
        }
        if let Some(sub_rules) = self
            .root
            .get_mut("sub-rules")
            .and_then(Value::as_mapping_mut)
        {
            for rules in sub_rules.values_mut().filter_map(Value::as_sequence_mut) {
                for rule in rules {
                    if let Some(line) = rule.as_str()
                        && rule_target(line) == Some(old)
                    {
                        *rule = Value::from(retarget(line, new));
                    }
                }
            }
        }
        self.set_rules(
            self.rules()
                .into_iter()
                .map(|rule| {
                    if rule_target(&rule) == Some(old) {
                        retarget(&rule, new)
                    } else {
                        rule
                    }
                })
                .collect(),
        );
        for rule in &mut self.metadata.disabled_rules {
            if rule_target(&rule.line) == Some(old) {
                rule.line = retarget(&rule.line, new);
            }
        }
        for node in &mut self.metadata.disabled_nodes {
            replace_value(&mut node.value, old, new, member_target);
            for (_, rule) in node
                .rules
                .iter_mut()
                .chain(node.rule_replacements.iter_mut())
            {
                if rule_target(rule) == Some(old) {
                    *rule = retarget(rule, new);
                }
            }
            for (_, _, before, after) in &mut node.sub_rules {
                if rule_target(before) == Some(old) {
                    *before = retarget(before, new);
                }
                if rule_target(after) == Some(old) {
                    *after = retarget(after, new);
                }
            }
        }
        for group in &mut self.metadata.disabled_groups {
            replace_value(&mut group.value, old, new, member_target);
            for (_, rule) in group
                .rules
                .iter_mut()
                .chain(group.rule_replacements.iter_mut())
            {
                if rule_target(rule) == Some(old) {
                    *rule = retarget(rule, new);
                }
            }
            for (_, _, before, after) in &mut group.sub_rules {
                if rule_target(before) == Some(old) {
                    *before = retarget(before, new);
                }
                if rule_target(after) == Some(old) {
                    *after = retarget(after, new);
                }
            }
            for (_, before, after) in &mut group.memberships {
                for member in before.iter_mut().chain(after.iter_mut()) {
                    if member == old {
                        *member = member_target.into();
                    }
                }
            }
            for (_, _, _, replacement) in &mut group.references {
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
        let mut group = self.editable_group(name.trim()).ok_or("策略组不存在")?;
        put(&mut group, "type", kind);
        if kind != "select" {
            let parsed = url::Url::parse(url.trim()).map_err(|_| "测速地址无效")?;
            if !["http", "https"].contains(&parsed.scheme()) {
                return Err("测速地址必须是 HTTP / HTTPS".into());
            }
            let seconds: u32 = interval.trim().parse().map_err(|_| "测速间隔需要正整数")?;
            if seconds == 0 {
                return Err("测速间隔需要大于 0".into());
            }
            put(&mut group, "url", url.trim());
            put(&mut group, "interval", Value::from(seconds));
        }
        let mut groups = self.items("proxy-groups");
        if let Some(index) = groups
            .iter()
            .position(|group| text(group, "name") == name.trim())
        {
            groups[index] = group;
            self.set_items("proxy-groups", groups);
        } else if let Some(disabled) = self
            .metadata
            .disabled_groups
            .iter_mut()
            .find(|group| text(&group.value, "name") == name.trim())
        {
            disabled.value = group;
        }
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

    pub fn chain_exit(&self, name: &str) -> Option<String> {
        let node = self.editable_node(name)?;
        if let Some(exit) = self.metadata.chain_exits.get(name)
            && self.editable_node(exit).is_some()
        {
            return Some(exit.clone());
        }
        let outbound = |mut value: Value| {
            if let Some(map) = value.as_mapping_mut() {
                map.remove("name");
                map.remove("dialer-proxy");
            }
            value
        };
        let current = outbound(node);
        self.names("proxies")
            .into_iter()
            .find(|candidate| {
                candidate != name
                    && self
                        .editable_node(candidate)
                        .is_some_and(|node| outbound(node) == current)
            })
            .or_else(|| Some(name.into()))
    }

    pub fn save_chain(
        &mut self,
        editing: Option<&str>,
        typed_name: &str,
        entry: &str,
        exit: &str,
    ) -> Result<String, String> {
        if entry == exit || editing == Some(entry) {
            return Err("入口和出口需要是不同的节点，入口不能是当前链式节点".into());
        }
        if self.editable_node(entry).is_none() {
            return Err("入口节点不存在".into());
        }
        let mut value = self.editable_node(exit).ok_or("请选择出口节点")?;
        let name = if typed_name.trim().is_empty() {
            self.unique_name(&format!("{entry} | {exit}"))
        } else {
            typed_name.trim().to_string()
        };
        if let Some(old) = editing {
            self.rename(old, &name)?;
        } else if self.targets().contains(&name)
            || self
                .metadata
                .disabled_nodes
                .iter()
                .any(|node| text(&node.value, "name") == name)
            || self
                .metadata
                .disabled_groups
                .iter()
                .any(|group| text(&group.value, "name") == name)
        {
            return Err("名称已被其他节点或策略组使用".into());
        }
        put(&mut value, "name", name.clone());
        put(&mut value, "dialer-proxy", entry);
        let mut nodes = self.items("proxies");
        if editing.is_some() {
            if let Some(node) = nodes.iter_mut().find(|node| text(node, "name") == name) {
                *node = value;
            } else if let Some(node) = self
                .metadata
                .disabled_nodes
                .iter_mut()
                .find(|node| text(&node.value, "name") == name)
            {
                node.value = value;
            } else {
                return Err("链式节点不存在".into());
            }
        } else {
            nodes.push(value);
        }
        self.set_items("proxies", nodes);
        if editing != Some(exit) {
            self.metadata.chain_exits.insert(name.clone(), exit.into());
        }
        Ok(name)
    }
    pub fn add_group(&mut self, name: &str, copy: Option<&str>) -> Result<String, String> {
        let name = self.unique_name(name);
        let mut group = copy.and_then(|n| self.group(n)).unwrap_or_else(|| {
            let mut default_members = Vec::new();
            for preferred in ["节点选择", "自动选择"] {
                if self.group(preferred).is_some() {
                    default_members.push(preferred.to_string());
                }
            }
            default_members.push("DIRECT".into());
            default_members.extend(self.names("proxies"));
            let mut seen = HashSet::new();
            default_members.retain(|member| seen.insert(member.clone()));
            let mut map = Mapping::new();
            map.insert(Value::String("name".into()), Value::String("new".into()));
            map.insert(Value::String("type".into()), Value::String("select".into()));
            map.insert(Value::String("proxies".into()), values(default_members));
            Value::Mapping(map)
        });
        put(&mut group, "name", name.clone());
        let mut groups = self.items("proxy-groups");
        groups.push(group);
        self.set_items("proxy-groups", groups);
        Ok(name)
    }
    pub fn remove_group(&mut self, name: &str) -> Result<(), String> {
        if self.editable_group(name).is_none() {
            return Err("策略组不存在".into());
        }
        let fallback = if name != "节点选择" && self.group("节点选择").is_some() {
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
        self.set_items("proxy-groups", groups);
        self.metadata
            .disabled_groups
            .retain(|group| text(&group.value, "name") != name);
        self.retarget_references_to(name, fallback, "DIRECT")?;
        for node in &mut self.metadata.disabled_nodes {
            node.memberships.retain(|(group, _)| group != name);
            node.placeholders.retain(|group| group != name);
        }
        for group in &mut self.metadata.disabled_groups {
            group.memberships.retain(|(group, _, _)| group != name);
            group
                .references
                .retain(|(section, entry, _, _)| section != "proxy-groups" || entry != name);
        }
        Ok(())
    }
    pub fn remove_node(&mut self, name: &str) -> Result<(), String> {
        if self.editable_node(name).is_none() {
            return Err("节点不存在".into());
        }
        let mut proxies = self.items("proxies");
        proxies.retain(|node| text(node, "name") != name);
        self.set_items("proxies", proxies);
        self.metadata
            .disabled_nodes
            .retain(|node| text(&node.value, "name") != name);
        self.retarget_references_to(name, "DIRECT", "DIRECT")?;
        self.metadata
            .node_sources
            .retain(|source| source.name != name);
        self.metadata
            .chain_exits
            .retain(|chain, exit| chain != name && exit != name);
        for group in &mut self.metadata.disabled_groups {
            group
                .references
                .retain(|(section, entry, _, _)| section != "proxies" || entry != name);
        }
        for node in &mut self.metadata.disabled_nodes {
            node.references
                .retain(|(section, entry, _, _)| section != "proxies" || entry != name);
        }
        Ok(())
    }
    pub fn member(&mut self, group: &str, member: &str, enabled: bool) -> Result<(), String> {
        if group == member {
            return Err("策略组不能引用自身".into());
        }
        let mut value = self.editable_group(group).ok_or("组不存在")?;
        let mut members = strings(value.get("proxies"));
        if enabled {
            if !members.iter().any(|m| m == member) {
                members.push(member.into());
            }
        } else {
            members.retain(|m| m != member);
        }
        put(&mut value, "proxies", values(members));
        self.set_editable_group(group, value)
    }
    pub fn set_all_nodes_in_group(&mut self, group: &str, enabled: bool) -> Result<(), String> {
        let mut value = self.editable_group(group).ok_or("组不存在")?;
        let mut members = strings(value.get("proxies"));
        let proxy_names = self.names("proxies");
        if enabled {
            for proxy in proxy_names {
                if proxy != group && !members.iter().any(|m| m == &proxy) {
                    members.push(proxy);
                }
            }
        } else {
            let proxy_set: HashSet<String> = proxy_names.into_iter().collect();
            members.retain(|m| !proxy_set.contains(m));
            if members.is_empty() {
                members.push("DIRECT".into());
            }
        }
        put(&mut value, "proxies", values(members));
        self.set_editable_group(group, value)
    }
    pub fn move_item(&mut self, key: &str, from: usize, to: usize) -> Result<(), String> {
        let mut items = self.items(key);
        if from >= items.len() || to >= items.len() {
            return Err("位置已改变，请重试".into());
        }
        let item = items.remove(from);
        items.insert(to, item);
        if key == "rules"
            && items.iter().enumerate().any(|(index, item)| {
                item.as_str().is_some_and(|line| {
                    matches!(rule_parts(line).first(), Some(&"MATCH" | &"FINAL"))
                }) && index + 1 != items.len()
            })
        {
            return Err("兜底规则需要放在最后，避免后续规则无法命中".into());
        }
        self.set_items(key, items);
        Ok(())
    }
    pub fn move_member(&mut self, group: &str, from: usize, to: usize) -> Result<(), String> {
        let mut value = self.editable_group(group).ok_or("组不存在")?;
        let mut members = strings(value.get("proxies"));
        if from >= members.len() || to >= members.len() {
            return Err("成员位置已改变".into());
        }
        let m = members.remove(from);
        members.insert(to, m);
        put(&mut value, "proxies", values(members));
        self.set_editable_group(group, value)
    }
    fn set_editable_group(&mut self, name: &str, value: Value) -> Result<(), String> {
        let mut groups = self.items("proxy-groups");
        if let Some(index) = groups.iter().position(|group| text(group, "name") == name) {
            groups[index] = value;
            self.set_items("proxy-groups", groups);
        } else if let Some(group) = self
            .metadata
            .disabled_groups
            .iter_mut()
            .find(|group| text(&group.value, "name") == name)
        {
            group.value = value;
        } else {
            return Err("组不存在".into());
        }
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
            let rules = saved.rules.clone();
            let replacements = saved.rule_replacements.clone();
            let references = saved.references.clone();
            let sub_rules = saved.sub_rules.clone();
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
            self.restore_target_rules(rules, &replacements);
            self.restore_target_fields(name, references);
            self.restore_sub_rules(sub_rules);
        } else {
            let mut proxies = self.items("proxies");
            let index = proxies
                .iter()
                .position(|p| text(p, "name") == name)
                .ok_or("节点不存在")?;
            let value = proxies.remove(index);
            self.set_items("proxies", proxies);
            let (rules, rule_replacements) = self.take_target_rules(name, "DIRECT");
            let references = self.redirect_target_fields(name, "DIRECT");
            let sub_rules = self.redirect_sub_rules(name, "DIRECT");
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
                rules,
                rule_replacements,
                references,
                sub_rules,
            });
        }
        Ok(())
    }
    fn take_target_rules(&mut self, name: &str, fallback: &str) -> (IndexedRules, IndexedRules) {
        let original: Vec<_> = self
            .rules()
            .into_iter()
            .enumerate()
            .filter(|(_, line)| rule_target(line) == Some(name))
            .collect();
        let replacements: Vec<_> = original
            .iter()
            .filter(|(_, line)| matches!(rule_parts(line).first(), Some(&"MATCH" | &"FINAL")))
            .map(|(index, line)| (*index, retarget(line, fallback)))
            .collect();
        let rules = self
            .rules()
            .into_iter()
            .filter_map(|line| {
                if rule_target(&line) != Some(name) {
                    Some(line)
                } else if matches!(rule_parts(&line).first(), Some(&"MATCH" | &"FINAL")) {
                    Some(retarget(&line, fallback))
                } else {
                    None
                }
            })
            .collect();
        self.set_rules(rules);
        (original, replacements)
    }
    fn restore_target_rules(
        &mut self,
        original: Vec<(usize, String)>,
        replacements: &[(usize, String)],
    ) {
        let mut rules = self.rules();
        for (index, line) in original {
            if matches!(rule_parts(&line).first(), Some(&"MATCH" | &"FINAL")) {
                if let Some((_, replacement)) =
                    replacements.iter().find(|(saved, _)| *saved == index)
                    && let Some(current) = rules.iter_mut().find(|current| *current == replacement)
                {
                    *current = line;
                }
            } else if !rules.contains(&line) {
                let end = rules
                    .iter()
                    .position(|line| matches!(rule_parts(line).first(), Some(&"MATCH" | &"FINAL")))
                    .unwrap_or(rules.len());
                rules.insert(index.min(end), line);
            }
        }
        self.set_rules(rules);
    }
    fn redirect_target_fields(
        &mut self,
        name: &str,
        fallback: &str,
    ) -> Vec<(String, String, String, String)> {
        let mut references = Vec::new();
        for section in [
            "proxies",
            "listeners",
            "tunnels",
            "proxy-providers",
            "rule-providers",
        ] {
            let Some(value) = self.root.get_mut(section) else {
                continue;
            };
            let entries: Vec<(String, &mut Value)> = match value {
                Value::Sequence(items) => items
                    .iter_mut()
                    .map(|item| (text(item, "name"), item))
                    .collect(),
                Value::Mapping(items) => items
                    .iter_mut()
                    .filter_map(|(key, item)| key.as_str().map(|name| (name.to_string(), item)))
                    .collect(),
                _ => continue,
            };
            for (entry, item) in entries {
                for field in ["dialer-proxy", "proxy"] {
                    if text(item, field) == name {
                        let replacement = if field == "dialer-proxy" {
                            "DIRECT"
                        } else {
                            fallback
                        };
                        references.push((
                            section.into(),
                            entry.clone(),
                            field.into(),
                            replacement.into(),
                        ));
                        put(item, field, replacement);
                    }
                }
            }
        }
        references
    }
    fn restore_target_fields(
        &mut self,
        name: &str,
        references: Vec<(String, String, String, String)>,
    ) {
        for (section, entry, field, replacement) in references {
            let Some(value) = self.root.get_mut(&section) else {
                continue;
            };
            let item = match value {
                Value::Sequence(items) => items.iter_mut().find(|item| text(item, "name") == entry),
                Value::Mapping(items) => items.get_mut(Value::String(entry)),
                _ => None,
            };
            if let Some(item) = item
                && text(item, &field) == replacement
            {
                put(item, &field, name);
            }
        }
    }
    fn redirect_sub_rules(
        &mut self,
        name: &str,
        fallback: &str,
    ) -> Vec<(String, usize, String, String)> {
        let mut saved = Vec::new();
        if let Some(sub_rules) = self
            .root
            .get_mut("sub-rules")
            .and_then(Value::as_mapping_mut)
        {
            for (key, rules) in sub_rules {
                let Some(key) = key.as_str() else {
                    continue;
                };
                let Some(rules) = rules.as_sequence_mut() else {
                    continue;
                };
                for (index, rule) in rules.iter_mut().enumerate() {
                    if let Some(line) = rule.as_str()
                        && rule_target(line) == Some(name)
                    {
                        let after = retarget(line, fallback);
                        saved.push((key.into(), index, line.into(), after.clone()));
                        *rule = Value::from(after);
                    }
                }
            }
        }
        saved
    }
    fn restore_sub_rules(&mut self, saved: Vec<(String, usize, String, String)>) {
        for (key, index, before, after) in saved {
            if let Some(rule) = self
                .root
                .get_mut("sub-rules")
                .and_then(|rules| rules.get_mut(&key))
                .and_then(Value::as_sequence_mut)
                .and_then(|rules| rules.get_mut(index))
                && rule.as_str() == Some(&after)
            {
                *rule = Value::from(before);
            }
        }
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
        if !enabled && let Some(scope) = scope {
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
        if enabled && let Some(scope) = scope {
            let mut rules = self.rules();
            for line in &mut rules {
                let p = rule_parts(line);
                if p.first() == Some(&"RULE-SET") && p.get(1) == Some(&binding.id.as_str()) {
                    *line = retarget(line, scope);
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
            let mut list = vec!["节点选择".into(), "DIRECT".into()];
            list.extend(self.names("proxies"));
            list
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
    pub fn update_node(
        &mut self,
        old: &str,
        name: &str,
        server: &str,
        port: &str,
    ) -> Result<(), String> {
        let original = self.editable_node(old).ok_or("节点不存在")?;
        let proxy =
            swiftsub_core::Proxy::from_value(original).map_err(|error| error.to_string())?;
        if !proxy.uses_server_port() {
            return self.rename(old, name);
        }
        let port: u16 = port.trim().parse().map_err(|_| "端口需为 1–65535")?;
        if port == 0 || server.trim().is_empty() {
            return Err("请填写节点地址和有效端口".into());
        }
        self.rename(old, name)?;
        let mut nodes = self.items("proxies");
        if let Some(node) = nodes
            .iter_mut()
            .find(|node| text(node, "name") == name.trim())
        {
            put(node, "server", server.trim());
            put(node, "port", Value::from(port));
            self.set_items("proxies", nodes);
        } else if let Some(node) = self
            .metadata
            .disabled_nodes
            .iter_mut()
            .find(|node| text(&node.value, "name") == name.trim())
        {
            put(&mut node.value, "server", server.trim());
            put(&mut node.value, "port", Value::from(port));
        }
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
            for (group, before, after) in &saved.memberships {
                if let Some(disabled) = self
                    .metadata
                    .disabled_groups
                    .iter_mut()
                    .find(|disabled| text(&disabled.value, "name") == *group)
                    && strings(disabled.value.get("proxies")) == *after
                {
                    put(&mut disabled.value, "proxies", values(before.clone()));
                }
            }
            self.restore_target_rules(saved.rules, &saved.rule_replacements);
            self.restore_target_fields(name, saved.references);
            self.restore_sub_rules(saved.sub_rules);
        } else {
            let groups = self.items("proxy-groups");
            let index = groups
                .iter()
                .position(|g| text(g, "name") == name)
                .ok_or("策略组不存在")?;
            let value = groups[index].clone();
            let before = groups
                .iter()
                .chain(
                    self.metadata
                        .disabled_groups
                        .iter()
                        .map(|group| &group.value),
                )
                .filter_map(|g| {
                    let members = strings(g.get("proxies"));
                    members
                        .iter()
                        .any(|m| m == name)
                        .then(|| (text(g, "name"), members))
                })
                .collect::<Vec<_>>();
            let fallback = if name != "节点选择" && self.group("节点选择").is_some() {
                "节点选择"
            } else {
                "DIRECT"
            }
            .to_string();
            let (rules, rule_replacements) = self.take_target_rules(name, &fallback);
            let references = self.redirect_target_fields(name, &fallback);
            let sub_rules = self.redirect_sub_rules(name, &fallback);
            let replace_members = |group: &mut Value| {
                if group.get("proxies").is_some() {
                    let mut seen = HashSet::new();
                    let members = strings(group.get("proxies"))
                        .into_iter()
                        .map(|member| {
                            if member == name {
                                "DIRECT".into()
                            } else {
                                member
                            }
                        })
                        .filter(|member| seen.insert(member.clone()))
                        .collect();
                    put(group, "proxies", values(members));
                }
            };
            let mut groups = groups;
            groups.remove(index);
            for group in &mut groups {
                replace_members(group);
            }
            self.set_items("proxy-groups", groups);
            for group in &mut self.metadata.disabled_groups {
                replace_members(&mut group.value);
            }
            let memberships = before
                .into_iter()
                .map(|(name, prior)| {
                    let after = self
                        .editable_group(&name)
                        .map(|group| strings(group.get("proxies")))
                        .unwrap_or_default();
                    (name, prior, after)
                })
                .collect();
            self.metadata.disabled_groups.push(DisabledGroup {
                value,
                index,
                rules,
                memberships,
                references,
                rule_replacements,
                sub_rules,
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
        // An upstream rename is still the same node; keep any local alias and references.
        for node in &nodes {
            let original_name = text(node, "name");
            let identity = swiftsub_core::Proxy::from_value(node.clone())
                .ok()
                .map(|proxy| proxy.identity_key());
            let renamed = self
                .metadata
                .node_sources
                .iter()
                .find(|source| {
                    source.source_id == source_id
                        && source.original_name != original_name
                        && source
                            .original
                            .as_ref()
                            .and_then(|value| swiftsub_core::Proxy::from_value(value.clone()).ok())
                            .is_some_and(|proxy| Some(proxy.identity_key()) == identity)
                })
                .cloned();
            if let Some(source) = renamed {
                let name = if source.name == source.original_name
                    && !self.names("proxies").contains(&original_name)
                {
                    self.rename(&source.name, &original_name)?;
                    original_name.clone()
                } else {
                    source.name.clone()
                };
                if let Some(metadata) = self
                    .metadata
                    .node_sources
                    .iter_mut()
                    .find(|item| item.source_id == source_id && item.name == name)
                {
                    metadata.original_name = original_name;
                }
            }
        }
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
                if self.editable_node(name).is_some() {
                    self.remove_node(name)?;
                }
            }
            self.metadata
                .node_sources
                .retain(|s| !(s.source_id == source_id && missing.contains(&s.name)));
        }
        for mut node in nodes {
            let original = text(&node, "name");
            let upstream = node.clone();
            if let Some(origin) = self
                .metadata
                .node_sources
                .iter()
                .find(|s| s.source_id == source_id && s.original_name == original)
                .cloned()
            {
                let existing = self
                    .items("proxies")
                    .into_iter()
                    .find(|value| text(value, "name") == origin.name)
                    .or_else(|| {
                        self.metadata
                            .disabled_nodes
                            .iter()
                            .find(|value| text(&value.value, "name") == origin.name)
                            .map(|value| value.value.clone())
                    });
                if let Some(existing) = existing {
                    merge_local_node_fields(&mut node, &existing, origin.original.as_ref());
                }
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
                if let Some(source) = self.metadata.node_sources.iter_mut().find(|source| {
                    source.source_id == source_id && source.original_name == original
                }) {
                    source.original = Some(upstream);
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
                        original: Some(upstream),
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
                    original: Some(upstream),
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
