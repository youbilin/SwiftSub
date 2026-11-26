use super::*;
use crate::saved_document::{SavedDocument, text, unsigned};
use serde_yaml_ng::Value;
use swiftsub_store::SavedConfig;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum SavedEditorTab {
    Groups,
    Rules,
    Yaml,
}
pub struct SavedEditor {
    pub document: Option<SavedDocument>,
    pub tab: SavedEditorTab,
    pub search: Entity<InputState>,
    pub config_search: Entity<InputState>,
    pub member_search: Entity<InputState>,
    pub selected_group: Option<String>,
    pub group_name: Entity<InputState>,
    pub group_kind: String,
    pub group_url: Entity<InputState>,
    pub group_interval: Entity<InputState>,
    pub selected_node: Option<String>,
    pub node_name: Entity<InputState>,
    pub node_server: Entity<InputState>,
    pub node_port: Entity<InputState>,
    pub new_rule: Entity<InputState>,
    pub rule_scope: Option<String>,
    pub rule_only_enabled: bool,
    pub rule_current_group_only: bool,
    pub mixed_port: Entity<InputState>,
    pub mode: String,
    pub log_level: String,
    pub allow_lan: bool,
    pub dns: bool,
    pub pending: bool,
    pub more: bool,
    pub list_collapsed: bool,
    pub node_import: bool,
    pub node_sources_open: HashSet<i64>,
    pub undo: Vec<SavedDocument>,
    pub redo: Vec<SavedDocument>,
    pub group_expanded: bool,
    pub editing_group: bool,
    pub editing_node: bool,
    pub group_rules: bool,
    pub member_picker: bool,
    pub picking_rules: bool,
    pub rule_picker_kind: String,
    pub rule_picks: HashSet<String>,
    pub rule_edit: Option<usize>,
    pub rule_edit_disabled: bool,
    pub copy_open: bool,
    pub copy_name: Entity<InputState>,
    pub chains_tab: bool,
    pub chain_name: Entity<InputState>,
    pub chain_entry: Option<String>,
    pub chain_exit: Option<String>,
    pub chain_edit: Option<String>,
    group_baseline: Option<(String, String, String, String)>,
    node_baseline: Option<(String, String, String)>,
    basic_baseline: Option<(String, String, String, bool, bool)>,
}
#[derive(Serialize, Deserialize)]
struct SavedFormDraft {
    yaml: String,
    group: Option<String>,
    group_values: (String, String, String, String),
    group_baseline: Option<(String, String, String, String)>,
    node: Option<String>,
    node_values: (String, String, String),
    node_baseline: Option<(String, String, String)>,
    basic: (String, String, String, bool, bool),
    basic_baseline: Option<(String, String, String, bool, bool)>,
}

pub(super) fn saved_edit_source(
    existing: &SavedConfig,
    yaml: &str,
    visual: Option<&SavedDocument>,
    raw: bool,
    pending: bool,
) -> Result<Option<String>, String> {
    if yaml == existing.yaml && !pending {
        return Ok(existing.source_json.clone());
    }
    if raw {
        // YAML edits replace disabled-item metadata but retain unrelated extension metadata.
        let doc = SavedDocument::parse(yaml, None)?;
        doc.validate()?;
        return doc.source_json(existing.source_json.as_deref()).map(Some);
    }
    visual
        .ok_or_else(|| "没有可编辑的配置，请检查 YAML".to_string())?
        .source_json(existing.source_json.as_deref())
        .map(Some)
}
struct SavedVisualDrafts {
    meta: Option<(String, crate::saved_document::EditorMetadata)>,
    form: Option<SavedFormDraft>,
}

fn read_visual_drafts(store: &Store, id: &str) -> Result<SavedVisualDrafts, String> {
    fn read<T: serde::de::DeserializeOwned>(store: &Store, key: &str) -> Result<Option<T>, String> {
        store
            .get_setting(key)
            .map_err(|error| error.to_string())?
            .filter(|json| !json.trim().is_empty())
            .map(|json| {
                serde_json::from_str(&json)
                    .map_err(|error| format!("节点与策略组草稿读取失败，原数据已保留：{error}"))
            })
            .transpose()
    }
    Ok(SavedVisualDrafts {
        meta: read(store, &format!("visual-meta:{id}"))?,
        form: read(store, &format!("visual-form:{id}"))?,
    })
}

impl SavedEditor {
    pub fn new(window: &mut Window, cx: &mut Context<SwiftSubApp>) -> Self {
        let mut input =
            |hint: &str| cx.new(|cx| InputState::new(window, cx).placeholder(hint.to_string()));
        Self {
            document: None,
            tab: SavedEditorTab::Groups,
            search: input("搜索节点 / 策略组 / 规则"),
            config_search: input("搜索我的配置"),
            member_search: input("搜索可用成员"),
            selected_group: None,
            group_name: input("策略组名称"),
            group_kind: "select".into(),
            group_url: input("测速地址"),
            group_interval: input("间隔（秒）"),
            selected_node: None,
            node_name: input("节点名称"),
            node_server: input("服务器地址"),
            node_port: input("端口"),
            new_rule: input("DOMAIN-SUFFIX,example.com,节点选择"),
            rule_scope: None,
            rule_only_enabled: false,
            rule_current_group_only: false,
            mixed_port: input("监听端口"),
            mode: "rule".into(),
            log_level: "info".into(),
            allow_lan: false,
            dns: true,
            pending: false,
            more: false,
            list_collapsed: false,
            node_import: false,
            node_sources_open: HashSet::new(),
            undo: Vec::new(),
            redo: Vec::new(),
            group_baseline: None,
            node_baseline: None,
            basic_baseline: None,
            group_expanded: false,
            editing_group: false,
            editing_node: false,
            group_rules: false,
            member_picker: false,
            picking_rules: false,
            rule_picker_kind: "规则库".into(),
            rule_picks: HashSet::new(),
            rule_edit: None,
            rule_edit_disabled: false,
            copy_open: false,
            copy_name: input("新配置名称"),
            chains_tab: false,
            chain_name: input("链式节点名称，可留空"),
            chain_entry: None,
            chain_exit: None,
            chain_edit: None,
        }
    }
}
impl SwiftSubApp {
    pub fn saved_open_group(
        &mut self,
        name: String,
        edit: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let closing = !edit
            && self.saved_editor.group_expanded
            && self.saved_editor.selected_group.as_ref() == Some(&name);
        self.select_saved_group(name, window, cx);
        if self.saved_config_error.is_none() {
            self.saved_editor.group_expanded = !closing;
            self.saved_editor.editing_group = edit;
            self.saved_editor.member_picker = false;
        }
        cx.notify();
    }

    pub fn saved_open_node(&mut self, name: String, window: &mut Window, cx: &mut Context<Self>) {
        self.select_saved_node(name, window, cx);
        self.saved_editor.editing_node = self.saved_config_error.is_none();
        cx.notify();
    }

    pub fn saved_begin_rule(
        &mut self,
        target: String,
        index: Option<usize>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Err(error) = self.flush_saved_fields(window, cx) {
            self.saved_config_error = Some(error);
            cx.notify();
            return;
        }
        let value = index
            .and_then(|index| {
                self.saved_editor
                    .document
                    .as_ref()?
                    .rules()
                    .get(index)
                    .cloned()
            })
            .unwrap_or_default();
        self.saved_editor
            .new_rule
            .update(cx, |input, cx| input.set_value(value, window, cx));
        self.saved_editor
            .search
            .update(cx, |input, cx| input.set_value("", window, cx));
        self.saved_editor.rule_scope = Some(target);
        self.saved_editor.rule_edit = index;
        self.saved_editor.rule_edit_disabled = false;
        self.saved_editor.rule_picks.clear();
        self.saved_editor.rule_picker_kind = if index.is_some() {
            "自定义"
        } else {
            "规则库"
        }
        .into();
        self.saved_editor.picking_rules = true;
        cx.notify();
    }

    pub fn saved_begin_disabled_rule(
        &mut self,
        target: String,
        index: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.saved_begin_rule(target, None, window, cx);
        let Some(line) = self.saved_editor.document.as_ref().and_then(|doc| {
            doc.metadata
                .disabled_rules
                .get(index)
                .map(|rule| rule.line.clone())
        }) else {
            return;
        };
        self.saved_editor
            .new_rule
            .update(cx, |input, cx| input.set_value(line, window, cx));
        self.saved_editor.rule_edit = Some(index);
        self.saved_editor.rule_edit_disabled = true;
        self.saved_editor.rule_picker_kind = "自定义".into();
        cx.notify();
    }

    pub fn saved_save_custom_rule(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let input = self
            .saved_editor
            .new_rule
            .read(cx)
            .value()
            .trim()
            .to_string();
        if input.starts_with("http://") || input.starts_with("https://") {
            self.saved_add_rule(window, cx);
        } else {
            let parsed: Result<Rule, swiftsub_core::Error> = Rule::parse(&input).or_else(|_| {
                let mut rule = Rule::parse_body(&input)?;
                rule.target = self
                    .saved_editor
                    .rule_scope
                    .clone()
                    .unwrap_or("DIRECT".into());
                Ok(rule)
            });
            match parsed {
                Ok(rule) => {
                    let index = self.saved_editor.rule_edit;
                    let disabled = self.saved_editor.rule_edit_disabled;
                    self.edit_saved_document(window, cx, |doc| {
                        if let Some(index) = index.filter(|_| disabled) {
                            doc.metadata
                                .disabled_rules
                                .get_mut(index)
                                .ok_or("规则已改变")?
                                .line = rule.to_line();
                            Ok(())
                        } else if let Some(index) = index {
                            let mut rules = doc.items("rules");
                            *rules.get_mut(index).ok_or("规则已改变")? =
                                Value::String(rule.to_line());
                            doc.set_items("rules", rules);
                            Ok(())
                        } else {
                            doc.add_rule(&rule.to_line())
                        }
                    });
                }
                Err(error) => self.saved_config_error = Some(error.to_string()),
            }
        }
        if self.saved_config_error.is_none() {
            self.saved_editor.picking_rules = false;
        }
        cx.notify();
    }

    pub fn saved_begin_chain(
        &mut self,
        name: Option<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Err(error) = self.flush_saved_fields(window, cx) {
            self.saved_config_error = Some(error);
            cx.notify();
            return;
        }
        let node = name
            .as_deref()
            .and_then(|name| self.saved_editor.document.as_ref()?.editable_node(name));
        self.saved_editor.chain_entry = node
            .as_ref()
            .map(|node| text(node, "dialer-proxy"))
            .filter(|name| !name.is_empty());
        self.saved_editor.chain_exit = name
            .as_deref()
            .and_then(|name| self.saved_editor.document.as_ref()?.chain_exit(name));
        self.saved_editor.chain_name.update(cx, |input, cx| {
            input.set_value(name.clone().unwrap_or_default(), window, cx)
        });
        self.saved_editor.chain_edit = name;
        self.saved_editor.chains_tab = true;
        cx.notify();
    }

    pub fn saved_save_chain(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some((entry, exit)) = self
            .saved_editor
            .chain_entry
            .clone()
            .zip(self.saved_editor.chain_exit.clone())
        else {
            self.saved_config_error = Some("请选择入口和出口节点".into());
            cx.notify();
            return;
        };
        let editing = self.saved_editor.chain_edit.clone();
        let name = self.saved_editor.chain_name.read(cx).value().to_string();
        self.edit_saved_document(window, cx, |doc| {
            doc.save_chain(editing.as_deref(), &name, &entry, &exit)
                .map(|_| ())
        });
        if self.saved_config_error.is_none() {
            self.saved_editor.chain_edit = None;
            self.saved_editor.chain_entry = None;
            self.saved_editor.chain_exit = None;
            self.saved_editor
                .chain_name
                .update(cx, |input, cx| input.set_value("", window, cx));
            self.autosave_saved_editor(window, cx)
                .unwrap_or_else(|error| self.saved_config_error = Some(error));
            self.show_toast("链式节点已保存".into(), cx);
        }
        cx.notify();
    }

    pub(super) fn load_saved_visual(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.saved_editor.copy_open = false;
        self.saved_editor.group_expanded = false;
        self.saved_editor.editing_group = false;
        self.saved_editor.editing_node = false;
        self.saved_editor.picking_rules = false;
        self.saved_editor.member_picker = false;
        self.saved_editor.chains_tab = false;
        self.saved_editor.document = None;
        self.saved_editor.pending = false;
        self.saved_editor.selected_group = None;
        self.saved_editor.selected_node = None;
        self.saved_editor.group_baseline = None;
        self.saved_editor.node_baseline = None;
        self.saved_editor.basic_baseline = None;
        self.saved_editor.undo.clear();
        self.saved_editor.redo.clear();
        let source = self
            .saved_config_id
            .as_ref()
            .and_then(|id| self.saved_configs.iter().find(|c| &c.id == id))
            .filter(|c| c.yaml == self.saved_config_yaml.read(cx).value().as_ref())
            .and_then(|c| c.source_json.as_deref());
        match SavedDocument::parse(self.saved_config_yaml.read(cx).value().as_ref(), source)
            .and_then(|doc| {
                doc.validate()?;
                Ok(doc)
            }) {
            Ok(doc) => {
                self.saved_editor.document = Some(doc);
                self.saved_editor.pending = false;
                self.saved_editor.tab = SavedEditorTab::Groups;
                self.saved_editor.more = false;
                self.saved_editor.selected_group = None;
                self.saved_editor.selected_node = None;
                self.saved_editor.group_baseline = None;
                self.saved_editor.node_baseline = None;
                self.saved_editor.undo.clear();
                self.saved_editor.redo.clear();
                self.saved_editor.rule_scope = None;
                self.saved_editor.rule_only_enabled = false;
                self.saved_editor.rule_current_group_only = false;
                self.saved_editor.node_sources_open.clear();
                self.saved_editor
                    .config_search
                    .update(cx, |s, cx| s.set_value("", window, cx));
                self.saved_editor
                    .search
                    .update(cx, |s, cx| s.set_value("", window, cx));
                let group = self
                    .saved_editor
                    .document
                    .as_ref()
                    .and_then(|d| d.names("proxy-groups").first().cloned());
                if let Some(group) = group {
                    self.populate_saved_group(group, window, cx);
                }
                self.populate_saved_basic(window, cx);
                self.restore_saved_form(window, cx);
            }
            Err(e) => {
                self.saved_editor.document = None;
                self.saved_editor.tab = SavedEditorTab::Yaml;
                self.saved_config_error = Some(e);
            }
        }
    }
    fn write_saved_visual(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Result<(), String> {
        let yaml = self
            .saved_editor
            .document
            .as_ref()
            .ok_or("没有可编辑的配置")?
            .yaml()?;
        self.saved_config_yaml
            .update(cx, |s, cx| s.set_value(yaml, window, cx));
        self.saved_editor.pending = true;
        self.saved_config_error = None;
        cx.notify();
        Ok(())
    }
    pub fn edit_saved_document(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
        action: impl FnOnce(&mut SavedDocument) -> Result<(), String>,
    ) {
        let result = (|| {
            self.flush_saved_fields(window, cx)?;
            let doc = self
                .saved_editor
                .document
                .as_mut()
                .ok_or("配置无法可视化编辑，请检查 YAML")?;
            let before = doc.clone();
            doc.transaction(action)?;
            if doc.root != before.root || doc.metadata != before.metadata {
                self.saved_editor.undo.push(before);
                if self.saved_editor.undo.len() > 32 {
                    self.saved_editor.undo.remove(0);
                }
                self.saved_editor.redo.clear();
                if self
                    .saved_editor
                    .selected_node
                    .as_ref()
                    .is_some_and(|name| doc.editable_node(name).is_none())
                {
                    self.saved_editor.selected_node = None;
                    self.saved_editor.node_baseline = None;
                }
                self.write_saved_visual(window, cx)?;
            }
            Ok::<_, String>(())
        })();
        if let Err(e) = result {
            self.saved_config_error = Some(e);
        }
        cx.notify();
    }
    fn populate_saved_group(&mut self, name: String, window: &mut Window, cx: &mut Context<Self>) {
        let Some(group) = self
            .saved_editor
            .document
            .as_ref()
            .and_then(|d| d.editable_group(&name))
        else {
            return;
        };
        let kind = text(&group, "type");
        let url = group
            .get("url")
            .and_then(Value::as_str)
            .unwrap_or("https://www.gstatic.com/generate_204")
            .to_string();
        let interval = unsigned(group.get("interval")).unwrap_or(300).to_string();
        self.saved_editor
            .group_name
            .update(cx, |s, cx| s.set_value(name.clone(), window, cx));
        self.saved_editor
            .group_url
            .update(cx, |s, cx| s.set_value(url.clone(), window, cx));
        self.saved_editor
            .group_interval
            .update(cx, |s, cx| s.set_value(interval.clone(), window, cx));
        self.saved_editor.group_kind = kind.clone();
        self.saved_editor.group_baseline = Some((name.clone(), kind, url, interval));
        self.saved_editor.selected_group = Some(name);
        self.saved_editor
            .member_search
            .update(cx, |s, cx| s.set_value("", window, cx));
    }
    pub fn select_saved_group(
        &mut self,
        name: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Err(e) = self.flush_saved_fields(window, cx) {
            self.saved_config_error = Some(e);
            cx.notify();
            return;
        }
        self.populate_saved_group(name, window, cx);
        self.saved_config_error = None;
        cx.notify();
    }
    pub fn select_saved_node(&mut self, name: String, window: &mut Window, cx: &mut Context<Self>) {
        if let Err(e) = self.flush_saved_fields(window, cx) {
            self.saved_config_error = Some(e);
            cx.notify();
            return;
        }
        let Some(node) = self
            .saved_editor
            .document
            .as_ref()
            .and_then(|d| d.editable_node(&name))
        else {
            return;
        };
        let server = text(&node, "server");
        let port = unsigned(node.get("port")).unwrap_or(0).to_string();
        self.saved_editor
            .node_name
            .update(cx, |s, cx| s.set_value(name.clone(), window, cx));
        self.saved_editor
            .node_server
            .update(cx, |s, cx| s.set_value(server.clone(), window, cx));
        self.saved_editor
            .node_port
            .update(cx, |s, cx| s.set_value(port.clone(), window, cx));
        self.saved_editor.node_baseline = Some((name.clone(), server, port));
        self.saved_editor.selected_node = Some(name);
        cx.notify();
    }
    fn populate_saved_basic(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(doc) = &self.saved_editor.document else {
            return;
        };
        let root = &doc.root;
        let port = unsigned(root.get("mixed-port")).unwrap_or(7897).to_string();
        self.saved_editor.mode = root
            .get("mode")
            .and_then(Value::as_str)
            .unwrap_or("rule")
            .into();
        self.saved_editor.log_level = root
            .get("log-level")
            .and_then(Value::as_str)
            .unwrap_or("info")
            .into();
        self.saved_editor.allow_lan = root
            .get("allow-lan")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        self.saved_editor.dns = root
            .get("dns")
            .and_then(|v| v.get("enable"))
            .and_then(Value::as_bool)
            .unwrap_or(true);
        self.saved_editor
            .mixed_port
            .update(cx, |s, cx| s.set_value(port.clone(), window, cx));
        self.saved_editor.basic_baseline = Some((
            port,
            self.saved_editor.mode.clone(),
            self.saved_editor.log_level.clone(),
            self.saved_editor.allow_lan,
            self.saved_editor.dns,
        ));
    }
    pub(crate) fn flush_saved_fields(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Result<(), String> {
        if self.saved_config_id.is_none()
            || self.saved_editor.document.is_none()
            || self.saved_editor.tab == SavedEditorTab::Yaml
        {
            return Ok(());
        }
        let group = (
            self.saved_editor
                .group_name
                .read(cx)
                .value()
                .trim()
                .to_string(),
            self.saved_editor.group_kind.clone(),
            self.saved_editor.group_url.read(cx).value().to_string(),
            self.saved_editor
                .group_interval
                .read(cx)
                .value()
                .to_string(),
        );
        let node = (
            self.saved_editor
                .node_name
                .read(cx)
                .value()
                .trim()
                .to_string(),
            self.saved_editor.node_server.read(cx).value().to_string(),
            self.saved_editor.node_port.read(cx).value().to_string(),
        );
        let basic = (
            self.saved_editor.mixed_port.read(cx).value().to_string(),
            self.saved_editor.mode.clone(),
            self.saved_editor.log_level.clone(),
            self.saved_editor.allow_lan,
            self.saved_editor.dns,
        );
        let group_changed = self
            .saved_editor
            .group_baseline
            .as_ref()
            .is_some_and(|b| b != &group);
        let node_changed = self
            .saved_editor
            .node_baseline
            .as_ref()
            .is_some_and(|b| b != &node);
        let basic_changed = self
            .saved_editor
            .basic_baseline
            .as_ref()
            .is_some_and(|b| b != &basic);
        if !group_changed && !node_changed && !basic_changed {
            return Ok(());
        }
        let old_group = self.saved_editor.selected_group.clone();
        let old_node = self.saved_editor.selected_node.clone();
        let Some(doc) = self.saved_editor.document.as_mut() else {
            return Ok(());
        };
        let before = doc.clone();
        doc.transaction(|doc| {
            if group_changed {
                doc.update_group(
                    old_group.as_deref().ok_or("未选择组")?,
                    &group.0,
                    &group.1,
                    &group.2,
                    &group.3,
                )?;
            }
            if node_changed {
                doc.update_node(
                    old_node.as_deref().ok_or("未选择节点")?,
                    &node.0,
                    &node.1,
                    &node.2,
                )?;
            }
            if basic_changed {
                doc.set_basic(&basic.0, &basic.1, &basic.2, basic.3, basic.4)?;
            }
            Ok(())
        })?;
        self.saved_editor.undo.push(before);
        self.saved_editor.redo.clear();
        if group_changed {
            if self.saved_editor.rule_scope == old_group {
                self.saved_editor.rule_scope = Some(group.0.clone());
            }
            self.saved_editor.selected_group = Some(group.0.clone());
            self.saved_editor.group_baseline = Some(group);
        }
        if node_changed {
            self.saved_editor.selected_node = Some(node.0.clone());
            self.saved_editor.node_baseline = Some(node);
        }
        if basic_changed {
            self.saved_editor.basic_baseline = Some(basic);
        }
        self.write_saved_visual(window, cx)
    }
    pub fn set_saved_tab(
        &mut self,
        tab: SavedEditorTab,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let previous_tab = self.saved_editor.tab;
        if tab == SavedEditorTab::Yaml
            && self.saved_editor.tab != SavedEditorTab::Yaml
            && let Err(e) = self.autosave_saved_editor(window, cx)
        {
            self.saved_config_error = Some(e);
            cx.notify();
            return;
        }

        if self.saved_editor.tab == SavedEditorTab::Yaml && tab != SavedEditorTab::Yaml {
            let source = self
                .saved_config_id
                .as_ref()
                .and_then(|id| self.saved_configs.iter().find(|c| &c.id == id))
                .filter(|c| c.yaml == self.saved_config_yaml.read(cx).value().as_ref())
                .and_then(|c| c.source_json.as_deref());
            match SavedDocument::parse(self.saved_config_yaml.read(cx).value().as_ref(), source) {
                Ok(doc) => {
                    if let Err(e) = doc.validate() {
                        self.saved_config_error = Some(e);
                        cx.notify();
                        return;
                    }

                    self.saved_editor.document = Some(doc);
                    self.saved_editor.undo.clear();
                    self.saved_editor.redo.clear();
                    self.saved_editor.selected_group = None;
                    self.saved_editor.selected_node = None;
                    self.saved_editor.group_baseline = None;
                    self.saved_editor.node_baseline = None;
                    self.populate_saved_basic(window, cx);
                    let first = self
                        .saved_editor
                        .document
                        .as_ref()
                        .and_then(|d| d.names("proxy-groups").first().cloned());
                    if let Some(first) = first {
                        self.populate_saved_group(first, window, cx);
                    }
                }
                Err(e) => {
                    self.saved_config_error = Some(e);
                    cx.notify();
                    return;
                }
            }
        } else if let Err(e) = self.flush_saved_fields(window, cx) {
            self.saved_config_error = Some(e);
            cx.notify();
            return;
        }
        if tab == SavedEditorTab::Groups {
            if let Some(group) = self.saved_editor.selected_group.clone().filter(|name| {
                self.saved_editor
                    .document
                    .as_ref()
                    .is_some_and(|doc| doc.group(name).is_some())
            }) {
                self.populate_saved_group(group, window, cx);
            }
        } else if tab == SavedEditorTab::Rules && previous_tab == SavedEditorTab::Groups {
            self.saved_editor.rule_scope = self.saved_editor.selected_group.clone();
        }
        self.saved_editor.tab = tab;
        self.saved_editor.more = false;
        self.saved_config_error = None;
        cx.notify();
    }
    pub fn saved_visual_dirty(&self, cx: &Context<Self>) -> bool {
        let e = &self.saved_editor;
        e.group_baseline.as_ref().is_some_and(|b| {
            b != &(
                e.group_name.read(cx).value().trim().to_string(),
                e.group_kind.clone(),
                e.group_url.read(cx).value().to_string(),
                e.group_interval.read(cx).value().to_string(),
            )
        }) || e.node_baseline.as_ref().is_some_and(|b| {
            b != &(
                e.node_name.read(cx).value().trim().to_string(),
                e.node_server.read(cx).value().to_string(),
                e.node_port.read(cx).value().to_string(),
            )
        }) || e.basic_baseline.as_ref().is_some_and(|b| {
            b != &(
                e.mixed_port.read(cx).value().to_string(),
                e.mode.clone(),
                e.log_level.clone(),
                e.allow_lan,
                e.dns,
            )
        })
    }
    pub(super) fn checkpoint_saved_form(&self, cx: &Context<Self>) -> Result<(), String> {
        let Some(id) = &self.saved_config_id else {
            return Ok(());
        };
        self.check_yaml_draft_storage()?;
        let store = self.store.lock().map_err(|error| error.to_string())?;
        read_visual_drafts(&store, id)?;
        drop(store);
        let e = &self.saved_editor;
        let encoded = if self.saved_visual_dirty(cx) {
            serde_json::to_string(&SavedFormDraft {
                yaml: self.saved_config_yaml.read(cx).value().to_string(),
                group: e.selected_group.clone(),
                group_values: (
                    e.group_name.read(cx).value().to_string(),
                    e.group_kind.clone(),
                    e.group_url.read(cx).value().to_string(),
                    e.group_interval.read(cx).value().to_string(),
                ),
                group_baseline: e.group_baseline.clone(),
                node: e.selected_node.clone(),
                node_values: (
                    e.node_name.read(cx).value().to_string(),
                    e.node_server.read(cx).value().to_string(),
                    e.node_port.read(cx).value().to_string(),
                ),
                node_baseline: e.node_baseline.clone(),
                basic: (
                    e.mixed_port.read(cx).value().to_string(),
                    e.mode.clone(),
                    e.log_level.clone(),
                    e.allow_lan,
                    e.dns,
                ),
                basic_baseline: e.basic_baseline.clone(),
            })
            .map_err(|e| e.to_string())?
        } else {
            String::new()
        };
        let meta_key = format!("visual-meta:{id}");
        let meta = if e.pending && e.tab != SavedEditorTab::Yaml {
            e.document
                .as_ref()
                .map(|d| {
                    serde_json::to_string(&(
                        self.saved_config_yaml.read(cx).value().to_string(),
                        &d.metadata,
                    ))
                })
                .transpose()
                .map_err(|e| e.to_string())?
                .unwrap_or_default()
        } else {
            String::new()
        };
        {
            let store = lock_store(&self.store);
            if store
                .get_setting(&meta_key)
                .map_err(|e| e.to_string())?
                .as_deref()
                != Some(meta.as_str())
            {
                store
                    .set_setting(&meta_key, &meta)
                    .map_err(|e| e.to_string())?;
            }
        }
        let key = format!("visual-form:{id}");
        let store = lock_store(&self.store);
        if store
            .get_setting(&key)
            .map_err(|e| e.to_string())?
            .as_deref()
            != Some(encoded.as_str())
        {
            store
                .set_setting(&key, &encoded)
                .map_err(|e| e.to_string())?;
        }
        Ok(())
    }
    fn restore_saved_form(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(id) = &self.saved_config_id else {
            return;
        };
        let drafts = read_visual_drafts(&lock_store(&self.store), id);
        let drafts = match drafts {
            Ok(drafts) => drafts,
            Err(error) => {
                self.saved_draft_corrupt = true;
                self.saved_config_error = Some(error);
                return;
            }
        };
        if let Some((yaml, metadata)) = drafts.meta
            && yaml == self.saved_config_yaml.read(cx).value().as_ref()
            && let Some(doc) = self.saved_editor.document.as_mut()
        {
            doc.metadata = metadata;
            self.saved_editor.pending = true;
        }
        let saved = drafts.form;
        let Some(draft) =
            saved.filter(|d| d.yaml == self.saved_config_yaml.read(cx).value().as_ref())
        else {
            return;
        };
        self.saved_editor.selected_group = draft.group;
        self.saved_editor.group_baseline = draft.group_baseline;
        self.saved_editor
            .group_name
            .update(cx, |s, cx| s.set_value(draft.group_values.0, window, cx));
        self.saved_editor.group_kind = draft.group_values.1;
        self.saved_editor
            .group_url
            .update(cx, |s, cx| s.set_value(draft.group_values.2, window, cx));
        self.saved_editor
            .group_interval
            .update(cx, |s, cx| s.set_value(draft.group_values.3, window, cx));
        self.saved_editor.selected_node = draft.node;
        self.saved_editor.node_baseline = draft.node_baseline;
        self.saved_editor
            .node_name
            .update(cx, |s, cx| s.set_value(draft.node_values.0, window, cx));
        self.saved_editor
            .node_server
            .update(cx, |s, cx| s.set_value(draft.node_values.1, window, cx));
        self.saved_editor
            .node_port
            .update(cx, |s, cx| s.set_value(draft.node_values.2, window, cx));
        self.saved_editor.basic_baseline = draft.basic_baseline;
        self.saved_editor
            .mixed_port
            .update(cx, |s, cx| s.set_value(draft.basic.0, window, cx));
        self.saved_editor.mode = draft.basic.1;
        self.saved_editor.log_level = draft.basic.2;
        self.saved_editor.allow_lan = draft.basic.3;
        self.saved_editor.dns = draft.basic.4;
    }
    pub(crate) fn autosave_saved_editor(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Result<(), String> {
        let Some(id) = self.saved_config_id.clone() else {
            return Ok(());
        };
        if self.saved_draft_corrupt {
            return Err("编辑草稿读取失败，原数据已保留，请恢复保存的配置后继续".into());
        }
        let draft_state = self.check_yaml_draft_storage().and_then(|_| {
            let store = self.store.lock().map_err(|error| error.to_string())?;
            read_visual_drafts(&store, &id).map(|_| ())
        });
        if let Err(error) = draft_state {
            self.saved_draft_corrupt = true;
            return Err(error);
        }
        self.flush_saved_fields(window, cx)?;
        self.check_saved_file_state()?;
        if !self.saved_config_dirty(cx) && !self.saved_editor.pending {
            return Ok(());
        }
        let Some(existing) = self.saved_configs.iter().find(|c| c.id == id).cloned() else {
            return Ok(());
        };
        let name = self.saved_config_name.read(cx).value().trim().to_string();
        if name.is_empty() {
            return Err("配置名称不能为空".into());
        }
        let yaml = self.saved_config_yaml.read(cx).value().to_string();
        super::saved_configs::validate_yaml(&yaml)?;
        let source = saved_edit_source(
            &existing,
            &yaml,
            self.saved_editor.document.as_ref(),
            self.saved_editor.tab == SavedEditorTab::Yaml,
            self.saved_editor.pending,
        )?;
        let saved = {
            let store = self.store.lock().map_err(|error| error.to_string())?;
            if !store
                .list_saved_configs()
                .map_err(|error| error.to_string())?
                .iter()
                .any(|current| {
                    current.id == existing.id
                        && current.yaml == existing.yaml
                        && current.name == existing.name
                        && current.source_json == existing.source_json
                })
            {
                return Err("此配置已被其他操作修改，当前草稿已保留，请另存一份或重新载入".into());
            }
            store
                .save_config_batch(
                    &self.data_dir.join("configs"),
                    &[swiftsub_store::SavedConfigWrite {
                        file_baseline: self.saved_file_baseline.as_deref(),
                        id: Some(&id),
                        name: &name,
                        yaml: &yaml,
                        source_json: source.as_deref(),
                        reason: "自动保存",
                    }],
                )
                .map_err(|e| e.to_string())?
                .pop()
                .ok_or("没有保存结果")?
        };
        if let Some(current) = self.saved_configs.iter_mut().find(|c| c.id == id) {
            *current = saved;
        }
        self.saved_editor.pending = false;
        self.saved_file_baseline = None;
        self.saved_external_change = false;
        self.saved_config_error = None;
        {
            let store = lock_store(&self.store);
            store
                .set_setting(&format!("visual-meta:{id}"), "")
                .map_err(|e| e.to_string())?;
            store
                .set_setting(&format!("visual-form:{id}"), "")
                .map_err(|e| e.to_string())?;
            store
                .set_setting(&format!("yaml-editor:{id}"), "")
                .map_err(|e| e.to_string())?;
        }
        self.publish_profiles();
        cx.notify();
        Ok(())
    }
    pub fn saved_undo(&mut self, redo: bool, window: &mut Window, cx: &mut Context<Self>) {
        let stack = if redo {
            &mut self.saved_editor.redo
        } else {
            &mut self.saved_editor.undo
        };
        let Some(previous) = stack.pop() else {
            return;
        };
        if let Some(current) = self.saved_editor.document.replace(previous) {
            if redo {
                self.saved_editor.undo.push(current);
            } else {
                self.saved_editor.redo.push(current);
            }
        }
        self.saved_editor.group_baseline = None;
        self.saved_editor.node_baseline = None;
        self.saved_editor.selected_node = None;
        let group = self
            .saved_editor
            .selected_group
            .clone()
            .filter(|n| {
                self.saved_editor
                    .document
                    .as_ref()
                    .is_some_and(|d| d.group(n).is_some())
            })
            .or_else(|| {
                self.saved_editor
                    .document
                    .as_ref()
                    .and_then(|d| d.names("proxy-groups").first().cloned())
            });
        if let Some(group) = group {
            self.populate_saved_group(group, window, cx);
        }
        self.populate_saved_basic(window, cx);
        if let Err(e) = self.write_saved_visual(window, cx) {
            self.saved_config_error = Some(e);
        }
        cx.notify();
    }
    pub fn saved_add_group(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let mut created = None;
        self.edit_saved_document(window, cx, |doc| {
            created = Some(doc.add_group("新策略组", None)?);
            Ok(())
        });
        if let Some(name) = created {
            self.populate_saved_group(name, window, cx);
        }
        cx.notify();
    }
    pub fn saved_delete_group(
        &mut self,
        name: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.edit_saved_document(window, cx, |d| d.remove_group(&name));
        if self
            .saved_editor
            .document
            .as_ref()
            .is_some_and(|d| d.group(&name).is_none())
            && self.saved_editor.selected_group.as_ref() == Some(&name)
        {
            self.saved_editor.group_baseline = None;
            self.saved_editor.selected_group = None;
            if let Some(first) = self
                .saved_editor
                .document
                .as_ref()
                .and_then(|d| d.names("proxy-groups").first().cloned())
            {
                self.populate_saved_group(first, window, cx);
            }
        }
    }
    pub fn saved_catalog_binding(
        &mut self,
        binding: RuleSetBinding,
        enabled: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let scope = self.saved_editor.rule_scope.clone();
        self.edit_saved_document(window, cx, |doc| {
            doc.set_scoped_binding(&binding, enabled, scope.as_deref())
        });
        if self.saved_config_error.is_none() && self.saved_editor.tab == SavedEditorTab::Rules {
            self.saved_editor.rule_scope = scope;
        }
    }
    pub fn saved_add_rule(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let line = self.saved_editor.new_rule.read(cx).value().to_string();
        if line.trim().starts_with("https://") || line.trim().starts_with("http://") {
            match rule_set_binding_from_url(line.trim()) {
                Ok(mut binding) => {
                    if let Some(scope) = &self.saved_editor.rule_scope {
                        binding.target = scope.clone();
                    }
                    self.saved_catalog_binding(binding, true, window, cx);
                }
                Err(e) => self.saved_config_error = Some(e),
            }
        } else {
            self.edit_saved_document(window, cx, |doc| doc.add_rule(&line));
        }
        if self.saved_config_error.is_none() {
            self.saved_editor
                .new_rule
                .update(cx, |s, cx| s.set_value("", window, cx));
        }
    }
    pub fn saved_library_nodes(&self) -> Vec<(i64, String, Vec<Proxy>)> {
        self.subscriptions
            .iter()
            .filter_map(|s| {
                let parsed = self
                    .parsed_sources
                    .parse(s.id, s.payload.as_deref()?)
                    .ok()?;
                Some((s.id, s.name.clone(), parsed.proxies))
            })
            .collect()
    }
    pub fn saved_import_source(
        &mut self,
        id: i64,
        nodes: Vec<Value>,
        replace: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.edit_saved_document(window, cx, |doc| doc.sync_source_nodes(id, nodes, replace));
    }
    pub fn saved_toggle_group(
        &mut self,
        name: String,
        enabled: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.edit_saved_document(window, cx, |d| d.toggle_group(&name, enabled));
        if self.saved_config_error.is_none() {
            if enabled {
                self.populate_saved_group(name, window, cx);
            } else if self.saved_editor.selected_group.as_ref() == Some(&name) {
                self.saved_editor.group_baseline = None;
                self.saved_editor.selected_group = None;
                if let Some(first) = self
                    .saved_editor
                    .document
                    .as_ref()
                    .and_then(|d| d.names("proxy-groups").first().cloned())
                {
                    self.populate_saved_group(first, window, cx);
                }
            }
        }
    }
    pub fn saved_set_rule_target(
        &mut self,
        index: usize,
        target: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.edit_saved_document(window, cx, |doc| doc.set_rule_target(index, &target));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn visual_and_raw_copies_preserve_appropriate_metadata_without_stale_recipes() {
        let yaml = "proxies:\n  - {name: first, type: http, server: example.org, port: 443}\n  - {name: second, type: http, server: example.net, port: 443}\nproxy-groups:\n  - {name: Select, type: select, proxies: [first, second]}\nrules: ['MATCH,DIRECT']\n";
        let existing = SavedConfig { id: "test".into(), name: "test".into(), yaml: yaml.into(), source_json: Some(r#"{"routing":{"old":true},"membership":[1],"filename":"old.yaml","external":{"keep":true}}"#.into()), created_at: 0, updated_at: 0 };
        let mut doc = SavedDocument::parse(yaml, existing.source_json.as_deref()).unwrap();
        doc.transaction(|doc| doc.toggle_node("first", false))
            .unwrap();
        let edited = doc.yaml().unwrap();
        let source = saved_edit_source(&existing, &edited, Some(&doc), false, true)
            .unwrap()
            .unwrap();
        let mut reopened = SavedDocument::parse(&edited, Some(&source)).unwrap();
        assert_eq!(reopened.metadata.disabled_nodes.len(), 1);
        reopened
            .transaction(|doc| doc.toggle_node("first", true))
            .unwrap();
        assert!(reopened.names("proxies").contains(&"first".into()));
        let source: serde_json::Value = serde_json::from_str(&source).unwrap();
        assert_eq!(source["external"]["keep"], true);
        assert!(source.get("routing").is_none());
        let raw_source = saved_edit_source(&existing, &edited, Some(&doc), true, true)
            .unwrap()
            .unwrap();
        let raw_doc = SavedDocument::parse(&edited, Some(&raw_source)).unwrap();
        assert!(raw_doc.metadata.disabled_nodes.is_empty());
        let raw_source: serde_json::Value = serde_json::from_str(&raw_source).unwrap();
        assert_eq!(raw_source["external"]["keep"], true);
        assert_eq!(
            saved_edit_source(&existing, yaml, None, true, false).unwrap(),
            existing.source_json
        );
    }

    #[test]
    fn malformed_visual_drafts_are_reported_and_remain_intact() {
        let store = Store::open_in_memory().unwrap();
        for prefix in ["visual-form", "visual-meta"] {
            let key = format!("{prefix}:test");
            store.set_setting(&key, "{unfinished").unwrap();
            assert!(read_visual_drafts(&store, "test").is_err());
            assert_eq!(
                store.get_setting(&key).unwrap().as_deref(),
                Some("{unfinished")
            );
            store.set_setting(&key, "").unwrap();
        }
    }
}
