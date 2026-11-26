use super::*;
pub mod dns;
mod drafts;
pub(crate) mod rule_updates;
pub(super) mod rules;
pub(crate) mod template_editor;
mod undo;

pub use template_editor::TemplateEditor;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct GenerationDefaults {
    pub dns: swiftsub_core::generator::DnsOptions,
    pub test_url: String,
    pub test_interval: u32,
    #[serde(default = "default_test_tolerance")]
    pub test_tolerance: u32,
    pub mixed_port: u16,
    pub allow_lan: bool,
}

impl GenerationDefaults {
    fn from_options(options: &GenerateOptions) -> Self {
        Self {
            dns: options.dns.clone(),
            test_url: options.test_url.clone(),
            test_interval: options.test_interval,
            test_tolerance: options.test_tolerance,
            mixed_port: options.mixed_port,
            allow_lan: options.allow_lan,
        }
    }
    pub(crate) fn from_store(store: &Mutex<Store>) -> Option<Self> {
        store
            .lock()
            .ok()?
            .get_setting("generation_defaults")
            .ok()
            .flatten()
            .and_then(|s| serde_json::from_str(&s).ok())
    }
    pub(crate) fn apply(&self, options: &mut GenerateOptions) {
        if options.dns_override {
            options.dns = self.dns.clone();
        }
        options.test_url = self.test_url.clone();
        options.test_interval = self.test_interval;
        options.test_tolerance = self.test_tolerance;
        options.mixed_port = self.mixed_port;
        options.allow_lan = self.allow_lan;
    }
    fn validate(&self) -> Result<(), String> {
        let mut options = GenerateOptions {
            preset: Preset::Minimal,
            dns_override: true,
            ..Default::default()
        };
        self.apply(&mut options);
        let nodes = build_nodes(
            vec![Proxy::new("direct", "设置校验节点", "127.0.0.1", 1)],
            "validation",
            "validation",
        );
        generate_profile(&nodes, &options)
            .map(|_| ())
            .map_err(|error| error.to_string())
    }
}

#[derive(Clone, Debug, Default)]
pub struct ConfigSummary {
    pub proxies: usize,
    pub groups: usize,
    pub rules: usize,
    pub generated: bool,
    pub invalid: bool,
}

impl ConfigSummary {
    pub fn from_config(config: &swiftsub_store::SavedConfig) -> Self {
        let generated = config
            .source_json
            .as_deref()
            .and_then(|source| serde_json::from_str::<serde_json::Value>(source).ok())
            .is_some_and(|source| source.get("routing").is_some());
        let Ok(root) = serde_yaml_ng::from_str::<serde_yaml_ng::Mapping>(&config.yaml) else {
            return Self {
                generated,
                invalid: true,
                ..Default::default()
            };
        };
        let count = |key: &str| {
            root.get(serde_yaml_ng::Value::String(key.into()))
                .and_then(serde_yaml_ng::Value::as_sequence)
                .map_or(0, Vec::len)
        };
        Self {
            proxies: count("proxies"),
            groups: count("proxy-groups"),
            rules: count("rules"),
            generated,
            invalid: false,
        }
    }

    pub fn text(&self) -> String {
        if self.invalid {
            "配置内容需要检查".into()
        } else {
            format!(
                "{} 个节点 · {} 个策略组 · {} 条规则",
                self.proxies, self.groups, self.rules
            )
        }
    }
}

pub struct Workspace {
    pub template_editor: TemplateEditor,
    pub group_rules_tab: bool,
    pub saved_editor_open: bool,
    pub sidebar_width: f32,
    pub resizing_sidebar: bool,
    pub dns_advanced_open: bool,
    pub confirm_delete_config: Option<String>,
    pub config_summaries: HashMap<String, ConfigSummary>,
    pub(crate) rule_visibility: super::layout::RuleVisibility,
    pub(crate) global_defaults: Option<GenerationDefaults>,
    pub settings_allow_lan: bool,
    pub settings_ipv6: bool,
    pub dns_editor: dns::DnsEditor,
    pub rule_content_id: Option<String>,
    pub rule_content_binding: Option<RuleSetBinding>,
    pub rule_content: Entity<TextareaState>,
    pub rule_content_original: String,
    pub rule_content_loading: bool,
    pub rule_content_error: Option<String>,
    pub rule_content_editing: bool,
    pub rule_content_request: u64,
    pub rule_content_group: String,
    pub rule_content_search: Entity<InputState>,
    pub rule_content_cache: HashMap<String, Vec<String>>,
    pub rule_cache_times: HashMap<String, i64>,
    pub rule_updates_busy: bool,
    pub rules_updated_at: Option<i64>,
    pub rule_updates_error: Option<String>,
    pub rule_picker_open: bool,
    pub rule_picker_kind: String,
    pub rule_picks: HashSet<String>,
    pub rules_target: Option<String>,
    pub group_settings_open: bool,
    pub rule_metadata_id: Option<String>,
    pub rule_metadata_name: Entity<InputState>,
    pub rule_metadata_url: Entity<InputState>,
    pub rule_metadata_behavior: String,
    pub rule_metadata_format: String,
    pub rule_metadata_no_resolve: bool,
    pub manual_edit: Option<usize>,
    pub manual_disabled_edit: Option<String>,
    pub categories_closed: HashSet<String>,
    pub save_template_open: bool,
    pub selected_template_id: Option<i64>,
    pub member_source: String,
    pub node_source: String,
    pub member_region: String,
    pub description: Entity<InputState>,
    pub tolerance: Entity<InputState>,
    pub manual_rule: Entity<InputState>,
    pub settings_loaded: bool,
    pub mixed_port: Entity<InputState>,
    pub domestic_dns: Entity<InputState>,
    pub foreign_dns: Entity<InputState>,
    pub fake_ip_range: Entity<InputState>,
    pub filter_sources: Entity<InputState>,
    pub chain_hops: Entity<InputState>,
    pub edit_filter: Option<usize>,
    pub edit_chain: Option<usize>,
    pub new_group_open: bool,
    pub node_scroll: gpui_kit::UniformListScrollHandle,
    pub routing_undo: Option<undo::RoutingUndo>,
    pub group_scroll: ScrollHandle,
    pub reveal_group: std::cell::RefCell<Option<String>>,
    pub rule_repair_open: bool,
    pub rule_source: String,
    pub settings_error_section: Option<&'static str>,
    pub global_test_url: Entity<InputState>,
    pub global_test_interval: Entity<InputState>,
    pub global_test_tolerance: Entity<InputState>,
}

impl Workspace {
    pub fn new(window: &mut Window, cx: &mut Context<SwiftSubApp>) -> Self {
        Self {
            template_editor: TemplateEditor::new(window, cx),
            group_rules_tab: false,
            saved_editor_open: false,
            sidebar_width: 300.0,
            resizing_sidebar: false,
            dns_advanced_open: false,
            confirm_delete_config: None,
            config_summaries: HashMap::new(),
            rule_visibility: Default::default(),
            global_defaults: None,
            settings_allow_lan: false,
            settings_ipv6: false,
            dns_editor: dns::DnsEditor::new(window, cx),
            rule_content_id: None,
            rule_content_binding: None,
            rule_content: cx.new(|cx| TextareaState::new(window, cx).searchable(true)),
            rule_content_original: String::new(),
            rule_content_loading: false,
            rule_content_error: None,
            rule_content_editing: false,
            rule_content_request: 0,
            rule_content_group: String::new(),
            rule_content_search: cx
                .new(|cx| InputState::new(window, cx).placeholder("搜索规则内容…")),
            rule_content_cache: HashMap::new(),
            rule_cache_times: HashMap::new(),
            rule_updates_busy: false,
            rules_updated_at: None,
            rule_updates_error: None,
            rule_picker_open: false,
            rule_picker_kind: "规则库".into(),
            rule_picks: HashSet::new(),
            rules_target: None,
            group_settings_open: false,
            rule_metadata_id: None,
            rule_metadata_name: cx.new(|cx| InputState::new(window, cx).placeholder("规则集名称")),
            rule_metadata_url: cx
                .new(|cx| InputState::new(window, cx).placeholder("规则集来源链接")),
            rule_metadata_behavior: "domain".into(),
            rule_metadata_format: "yaml".into(),
            rule_metadata_no_resolve: false,
            manual_edit: None,
            manual_disabled_edit: None,
            categories_closed: HashSet::new(),
            save_template_open: false,
            selected_template_id: None,
            member_source: String::new(),
            node_source: String::new(),
            member_region: String::new(),
            settings_loaded: false,
            edit_filter: None,
            edit_chain: None,
            new_group_open: false,
            node_scroll: gpui_kit::UniformListScrollHandle::new(),
            routing_undo: None,
            group_scroll: ScrollHandle::new(),
            reveal_group: std::cell::RefCell::new(None),
            rule_repair_open: false,
            rule_source: "builtin".into(),
            settings_error_section: None,
            global_test_url: cx.new(|cx| InputState::new(window, cx).placeholder("测速地址")),
            global_test_interval: cx
                .new(|cx| InputState::new(window, cx).placeholder("测速周期（秒）")),
            global_test_tolerance: cx
                .new(|cx| InputState::new(window, cx).placeholder("测速容差（毫秒）")),
            filter_sources: cx.new(|cx| {
                InputState::new(window, cx).placeholder("来源名称或 ID，逗号分隔；留空全部")
            }),
            chain_hops: cx.new(|cx| {
                InputState::new(window, cx).placeholder("可选多跳：按顺序填写节点名，逗号分隔")
            }),
            mixed_port: cx.new(|cx| InputState::new(window, cx).placeholder("混合端口")),
            domestic_dns: cx
                .new(|cx| InputState::new(window, cx).placeholder("国内 DNS，逗号分隔")),
            foreign_dns: cx.new(|cx| InputState::new(window, cx).placeholder("国外 DNS，逗号分隔")),
            fake_ip_range: cx.new(|cx| InputState::new(window, cx).placeholder("Fake IP 地址段")),
            description: cx.new(|cx| InputState::new(window, cx).placeholder("组说明（可选）")),
            tolerance: cx
                .new(|cx| InputState::new(window, cx).placeholder("测速容差 ms，留空使用默认")),
            manual_rule: cx.new(|cx| {
                InputState::new(window, cx)
                    .placeholder("例如 DOMAIN-SUFFIX,example.org；目标自动绑定当前组")
            }),
        }
    }
}

pub fn workspace_category(id: &str) -> &'static str {
    match id {
        "select" | "auto" | "final" | "ad" | "private" | "cn" | "global" => "核心与基础",
        "ai" | "gemini" | "google" | "github" | "microsoft" | "apple" | "onedrive"
        | "googlefcm" => "AI 与技术",
        "youtube" | "netflix" | "disney" | "spotify" | "bilibili" | "tiktok" | "hbo"
        | "primevideo" | "hulu" | "twitch" | "bahamut" => "影音媒体",
        "telegram" | "x" | "meta" | "discord" | "reddit" | "line" | "whatsapp" => "社交与通讯",
        "steam" | "epicgames" | "playstation" | "xbox" | "nintendo" => "游戏",
        _ => "其他服务",
    }
}

fn fetch_rule_payload(
    binding: &RuleSetBinding,
    options: &FetchOptions,
) -> Result<Vec<String>, String> {
    let mut urls = vec![binding.url.clone()];
    if binding.format == "mrs" {
        if let Ok(bytes) = swiftsub_fetch::fetch_rule_bytes(&binding.url, options)
            && let Ok(payload) =
                swiftsub_core::generator::decode_mrs_payload(&bytes, &binding.behavior)
        {
            return Ok(payload);
        }
        let mut url = url::Url::parse(&binding.url).map_err(|e| e.to_string())?;
        let stem = url
            .path()
            .strip_suffix(".mrs")
            .ok_or("MRS 地址缺少 .mrs 后缀")?
            .to_string();
        url.set_path(&format!("{stem}.yaml"));
        urls = vec![url.to_string()];
        url.set_path(&format!("{stem}.txt"));
        urls.push(url.to_string());
    }
    let mut last_error = String::new();
    for url in urls {
        match fetch_subscription(&url, options, &CacheValidators::default()) {
            Ok(FetchOutcome::Fetched(result)) => {
                match swiftsub_core::generator::parse_rule_set_payload(
                    &result.content,
                    &binding.behavior,
                ) {
                    Ok(payload) => return Ok(payload),
                    Err(error) => last_error = error.to_string(),
                }
            }
            Ok(_) => last_error = "来源没有返回内容".into(),
            Err(error) => last_error = error.to_string(),
        }
    }
    Err(format!(
        "读取规则集失败：{last_error}。可重新读取，或在规则信息中检查来源链接。"
    ))
}

impl SwiftSubApp {
    pub fn finish_workspace_fields(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        self.finish_policy_details(window, cx)
    }

    pub fn workspace_repair_rules(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        match self.autosave_routing(window, cx) {
            Ok(()) => {
                self.workspace.rule_repair_open = false;
                self.show_toast("规则已修正，配置已更新".into(), cx);
            }
            Err(error) => self.template_error = Some(error),
        }
        cx.notify();
    }

    pub fn workspace_restore_valid_rules(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let text = self
            .options
            .custom_rules
            .iter()
            .map(Rule::to_line)
            .collect::<Vec<_>>()
            .join("\n");
        self.template_custom_rules
            .update(cx, |input, cx| input.set_value(text, window, cx));
        self.workspace_repair_rules(window, cx);
    }

    pub fn workspace_save_group_settings(
        &mut self,
        index: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.save_policy_details(index, window, cx);
        if self.template_error.is_some() {
            return;
        }
        match self.autosave_routing(window, cx) {
            Ok(()) => {
                self.workspace.group_settings_open = false;
                self.show_toast("组设置已保存".into(), cx);
            }
            Err(error) => {
                self.template_error = Some(error);
                if parse_custom_rule_lines(&self.template_custom_rules.read(cx).value()).is_err() {
                    match self.checkpoint_draft(cx) {
                        Ok(()) => {
                            self.workspace.group_settings_open = false;
                            self.show_toast("组设置已保留到草稿，修正规则后生效".into(), cx);
                        }
                        Err(error) => self.template_error = Some(error),
                    }
                }
            }
        }
        cx.notify();
    }

    pub fn workspace_remove_group(
        &mut self,
        name: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self
            .template_policy_edit
            .and_then(|i| self.template_policy_groups.get(i))
            .is_some_and(|g| g.name == name)
        {
            // Removing this group also removes its unfinished parameter form.
            self.template_policy_edit = None;
            self.workspace.group_settings_open = false;
        }
        self.workspace_delete_group(name, window, cx);
        cx.notify();
    }
    pub fn workspace_edit_group(
        &mut self,
        name: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.open_template_dialog(window, cx);
        if let Err(error) = self.workspace_checkpoint_rule_content(cx) {
            self.template_error = Some(error);
            cx.notify();
            return;
        }
        if !self.finish_policy_details(window, cx) {
            return;
        }
        self.template_tab = TemplateTab::PolicyGroups;
        self.workspace.group_settings_open = false;
        self.workspace.rule_picker_open = false;
        self.workspace.rule_metadata_id = None;
        if self.custom_groups.iter().any(|group| group.name == name)
            || self.chains.iter().any(|chain| chain.name == name)
        {
            let was_expanded = self.workspace.rules_target.as_deref() == Some(name.as_str())
                && self.template_policy_edit.is_none();
            self.template_tab = if self.chains.iter().any(|chain| chain.name == name) {
                TemplateTab::Tools
            } else {
                TemplateTab::PolicyGroups
            };
            self.template_policy_edit = None;
            self.workspace.rules_target = if was_expanded { None } else { Some(name) };
            cx.notify();
            return;
        }
        self.workspace.rules_target = Some(name.clone());
        if let Some(index) = self
            .template_policy_groups
            .iter()
            .position(|group| group.name == name)
        {
            if self.template_policy_edit == Some(index) {
                self.template_policy_edit = None;
            } else {
                self.edit_template_policy_group(index, window, cx);
            }
        } else {
            self.edit_builtin_policy_group(name, window, cx);
        }
        cx.notify();
    }

    pub fn workspace_rules_for(
        &mut self,
        name: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.finish_policy_details(window, cx) {
            return;
        }
        if let Err(error) = self.workspace_checkpoint_rule_content(cx) {
            self.workspace.rule_content_error = Some(error);
            cx.notify();
            return;
        }
        self.workspace.rules_target = Some(name.clone());
        self.template_rules_target_filter = name.clone();
        if let Some(index) = self
            .template_policy_groups
            .iter()
            .position(|group| group.name == name)
        {
            self.edit_template_policy_group(index, window, cx);
        } else if self.workspace_rule_target_has_card(&name) {
            self.edit_builtin_policy_group(name, window, cx);
        } else {
            self.template_policy_edit = None;
            self.workspace.group_settings_open = false;
        }
        self.template_tab = if self
            .custom_groups
            .iter()
            .any(|group| Some(&group.name) == self.workspace.rules_target.as_ref())
            || self
                .chains
                .iter()
                .any(|chain| Some(&chain.name) == self.workspace.rules_target.as_ref())
        {
            TemplateTab::Tools
        } else {
            TemplateTab::PolicyGroups
        };
        cx.notify();
    }

    pub fn workspace_add_manual(
        &mut self,
        target: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(original) = self.workspace.manual_disabled_edit.clone() {
            self.save_disabled_manual(original, target, window, cx);
            return;
        }
        if !self.finish_workspace_fields(window, cx) {
            return;
        }
        let text = self
            .workspace
            .manual_rule
            .read(cx)
            .value()
            .trim()
            .to_string();
        let line = contextual_rule(&text, &target);
        match parse_custom_rule_lines(&line) {
            Ok(rules) if rules.len() == 1 => {
                let current = self.template_custom_rules.read(cx).value().to_string();
                let mut lines = match parse_custom_rule_lines(&current) {
                    Ok(rules) => rules.iter().map(Rule::to_line).collect::<Vec<_>>(),
                    Err(error) => {
                        self.template_error = Some(error);
                        cx.notify();
                        return;
                    }
                };
                let line = rules[0].to_line();
                if lines.iter().enumerate().any(|(index, existing)| {
                    Some(index) != self.workspace.manual_edit && *existing == line
                }) {
                    self.show_toast("此规则已添加".into(), cx);
                    return;
                }
                let original_order = self.template_rule_order.clone();
                let original_modules = self.template_enabled_modules.clone();
                let original_disabled = self.template_disabled_rule_sets.clone();
                let original_groups = self.template_policy_groups.clone();
                if let Some(index) = self.workspace.manual_edit {
                    if let Some(existing) = lines.get_mut(index) {
                        for key in &mut self.template_rule_order {
                            if key == existing {
                                *key = line.clone();
                            }
                        }
                        *existing = line;
                    } else {
                        self.template_error = Some("这条规则已被移除，请重新选择".into());
                        cx.notify();
                        return;
                    }
                } else {
                    lines.push(line);
                }
                self.workspace_activate_rule_target(&target);
                self.template_custom_rules.update(cx, |input, cx| {
                    input.set_value(lines.join("\n"), window, cx)
                });
                self.template_error = None;
                if let Err(error) = self.autosave_routing(window, cx) {
                    self.template_custom_rules
                        .update(cx, |input, cx| input.set_value(current, window, cx));
                    self.template_rule_order = original_order;
                    self.template_enabled_modules = original_modules;
                    self.template_disabled_rule_sets = original_disabled;
                    self.template_policy_groups = original_groups;
                    self.template_error = Some(error);
                } else {
                    self.workspace
                        .manual_rule
                        .update(cx, |input, cx| input.set_value("", window, cx));
                    self.workspace.manual_edit = None;
                    self.workspace.rule_picker_open = false;
                }
            }
            _ => self.template_error = Some("请输入一条有效规则，不需要填写末尾目标".into()),
        }
        cx.notify();
    }

    fn workspace_delete_group(
        &mut self,
        name: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let select_name = self.workspace_resolved_group_name("节点选择").to_string();
        let target = if name == select_name
            || self
                .template_policy_groups
                .iter()
                .any(|group| group.name == select_name && (group.disabled || group.removed))
        {
            "DIRECT"
        } else {
            &select_name
        }
        .to_string();
        if !self.finish_policy_details(window, cx) {
            return;
        }
        let mut snapshot = match self.draft_template_snapshot(cx) {
            Ok(snapshot) => snapshot,
            Err(error) => {
                self.template_error = Some(error);
                cx.notify();
                return;
            }
        };
        let before = snapshot.clone();
        let builtin = snapshot
            .policy_groups
            .iter()
            .find(|group| group.name == name)
            .and_then(PolicyGroupSpec::builtin_name)
            .map(str::to_string)
            .or_else(|| {
                catalog::MODULES
                    .iter()
                    .find(|module| module.name == name)
                    .map(|module| module.name.to_string())
            });
        let undo_name = self.template_name.read(cx).value().to_string();
        let undo_id = self.template_edit_id;
        let previous_nodes = self.nodes.clone();
        let previous_order = self.node_order.clone();
        let previous_visibility = self.workspace.rule_visibility.clone();
        let removed_chain = self.chains.iter().position(|chain| chain.name == name);
        for group in &mut snapshot.policy_groups {
            let mut next = Vec::with_capacity(group.members.len());
            let mut seen = HashSet::new();
            for member in group.members.drain(..) {
                let replaced = if member == name {
                    if group.name == target {
                        continue;
                    }
                    target.clone()
                } else {
                    member
                };
                if seen.insert(replaced.clone()) {
                    next.push(replaced);
                }
            }
            if !group.include_all_nodes && next.is_empty() && !group.disabled && !group.removed {
                next.push("DIRECT".into());
            }
            group.members = next;
        }
        for binding in &mut snapshot.custom_rule_sets {
            if binding.target == name {
                binding.target = target.clone();
            }
        }
        for value in snapshot.rule_set_targets.values_mut() {
            if value == &name {
                *value = target.clone();
            }
        }
        retarget_rule_lines(&mut snapshot.custom_rule_lines, &name, &target);
        retarget_rule_lines(&mut snapshot.rule_order, &name, &target);
        retarget_rule_lines(
            &mut self.workspace.rule_visibility.disabled_manual,
            &name,
            &target,
        );
        for listener in &mut snapshot.listeners {
            if listener.proxy == name {
                listener.proxy = target.clone();
            }
        }
        snapshot.policy_groups.retain(|group| group.name != name);
        snapshot.group_order.retain(|group| group != &name);
        if let Some(module) = catalog::MODULES
            .iter()
            .find(|module| builtin.as_deref() == Some(module.name))
        {
            snapshot.enabled_modules.retain(|id| id != module.id);
            snapshot.policy_groups.push(PolicyGroupSpec {
                id: format!("builtin:{}", module.name),
                name: name.clone(),
                builtin: true,
                disabled: true,
                removed: true,
                ..Default::default()
            });
        }
        snapshot.custom_groups.retain(|group| group.name != name);
        snapshot.chains.retain(|chain| chain.name != name);
        self.node_order.retain(|key| key != &chain_order_key(&name));
        // Commit only after validating replacement references and cycles. A failed delete leaves the draft intact.
        let result = (|| {
            let mut overrides = Vec::new();
            for node in &mut self.nodes {
                if node.proxy.get_str("dialer-proxy") == Some(&name) {
                    node.proxy.set_dialer_proxy("DIRECT");
                    overrides.push(node.clone());
                }
            }
            let overrides = overrides
                .iter()
                .map(|node| self.workspace_node_override(node))
                .collect::<Result<Vec<_>, _>>()?;
            self.persist_template_with_node_overrides(
                snapshot.clone(),
                None,
                &overrides,
                window,
                cx,
            )
        })();
        if let Err(error) = result {
            self.nodes = previous_nodes;
            self.node_order = previous_order;
            self.workspace.rule_visibility = previous_visibility;
            self.template_error = Some(error);
            cx.notify();
            return;
        }
        self.template_baseline = Some(snapshot.clone());
        let template_name = self.template_name.read(cx).value().to_string();
        self.load_template_draft(snapshot, template_name, self.template_edit_id, window, cx);
        if let Some(removed) = removed_chain
            && let Some(editing) = self.workspace.edit_chain
        {
            if removed == editing {
                self.reset_chain_editor(window, cx);
            } else if removed < editing {
                self.workspace.edit_chain = Some(editing - 1);
            }
        }
        self.workspace_remember_undo(before, undo_name, undo_id, format!("已删除「{name}」"));
        cx.notify();
    }

    pub fn workspace_load_template(
        &mut self,
        id: i64,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Err(error) = self.workspace_checkpoint_rule_content(cx) {
            self.template_error = Some(error);
            cx.notify();
            return;
        }
        let Some(template) = self
            .custom_templates
            .iter()
            .find(|template| template.id == id)
            .cloned()
        else {
            return;
        };
        let snapshot: TemplateSnapshot = match serde_json::from_str(&template.config_json) {
            Ok(snapshot) => snapshot,
            Err(error) => {
                self.show_toast(format!("模板读取失败：{error}"), cx);
                return;
            }
        };
        if let Err(error) = self
            .validate_template_snapshot(&snapshot)
            .and_then(|_| self.autosave_routing(window, cx))
        {
            self.template_error = Some(error);
            cx.notify();
            return;
        }
        let before = self.active_template_snapshot();
        let undo_name = self.template_name.read(cx).value().to_string();
        let undo_id = self.template_edit_id;
        self.close_template_dialog(window, cx);
        if self.template_session_active {
            return;
        }
        self.load_template_draft(snapshot, template.name, Some(id), window, cx);
        if let Err(error) = self.autosave_routing(window, cx) {
            self.template_error = Some(error);
        } else {
            self.workspace_remember_undo(
                before,
                undo_name,
                undo_id,
                "已替换分流模板，节点保持不变".into(),
            );
        }
        cx.notify();
    }

    pub fn workspace_preset(
        &mut self,
        preset: Preset,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Err(error) = self.workspace_checkpoint_rule_content(cx) {
            self.template_error = Some(error);
            cx.notify();
            return;
        }
        if let Err(error) = self.autosave_routing(window, cx) {
            self.template_error = Some(error);
            cx.notify();
            return;
        }
        let before = self.active_template_snapshot();
        let undo_name = self.template_name.read(cx).value().to_string();
        let undo_id = self.template_edit_id;
        self.close_template_dialog(window, cx);
        if self.template_session_active {
            return;
        }
        self.set_preset(preset, cx);
        self.workspace.selected_template_id = None;
        self.open_template_dialog(window, cx);
        self.workspace_remember_undo(
            before,
            undo_name,
            undo_id,
            "已替换分流模板，节点保持不变".into(),
        );
        self.workspace.settings_loaded = false;
    }

    pub fn workspace_load_settings(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let defaults = self
            .workspace
            .global_defaults
            .clone()
            .unwrap_or_else(|| GenerationDefaults::from_options(&self.options));
        let base = &defaults;
        self.workspace.settings_allow_lan = defaults.allow_lan;
        self.workspace.settings_ipv6 = defaults.dns.ipv6;
        self.workspace.dns_editor.load(&defaults.dns, window, cx);
        let port = base.mixed_port.to_string();
        let domestic = base.dns.domestic.join(", ");
        let foreign = base.dns.foreign.join(", ");
        let range = base.dns.fake_ip_range.clone();
        let test_url = base.test_url.clone();
        let test_interval = base.test_interval.to_string();
        let test_tolerance = base.test_tolerance.to_string();
        for (input, value) in [
            (&self.workspace.mixed_port, port),
            (&self.workspace.domestic_dns, domestic),
            (&self.workspace.foreign_dns, foreign),
            (&self.workspace.fake_ip_range, range),
            (&self.workspace.global_test_url, test_url),
            (&self.workspace.global_test_interval, test_interval),
            (&self.workspace.global_test_tolerance, test_tolerance),
        ] {
            input.update(cx, |input, cx| input.set_value(value, window, cx));
        }
        self.workspace.settings_loaded = true;
    }

    pub fn workspace_save_settings(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(port) = self
            .workspace
            .mixed_port
            .read(cx)
            .value()
            .parse::<u16>()
            .ok()
            .filter(|port| *port > 0)
        else {
            self.template_error = Some("端口须为 1–65535".into());
            cx.notify();
            return;
        };
        let mut defaults = self
            .workspace
            .global_defaults
            .clone()
            .unwrap_or_else(|| GenerationDefaults::from_options(&self.options));
        let base = &mut defaults;
        base.mixed_port = port;
        base.dns = match self.workspace.dns_editor.read(cx) {
            Ok(dns) => dns,
            Err(error) => {
                self.template_error = Some(error);
                cx.notify();
                return;
            }
        };
        base.allow_lan = self.workspace.settings_allow_lan;
        self.workspace_save_defaults(defaults, window, cx);
        cx.notify();
    }

    pub fn workspace_save_health(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let url = self
            .workspace
            .global_test_url
            .read(cx)
            .value()
            .trim()
            .to_string();
        let interval = self
            .workspace
            .global_test_interval
            .read(cx)
            .value()
            .trim()
            .parse::<u32>();
        let tolerance = self
            .workspace
            .global_test_tolerance
            .read(cx)
            .value()
            .trim()
            .parse::<u32>();
        let (Ok(interval), Ok(tolerance)) = (interval, tolerance) else {
            self.template_error =
                Some("测速地址须为 HTTP / HTTPS，周期须为正整数，容差须为非负整数".into());
            cx.notify();
            return;
        };
        if !url::Url::parse(&url)
            .is_ok_and(|u| matches!(u.scheme(), "http" | "https") && u.host_str().is_some())
            || interval == 0
        {
            self.template_error =
                Some("测速地址须为 HTTP / HTTPS，周期须为正整数，容差须为非负整数".into());
            cx.notify();
            return;
        }
        let mut defaults = self
            .workspace
            .global_defaults
            .clone()
            .unwrap_or_else(|| GenerationDefaults::from_options(&self.options));
        let base = &mut defaults;
        base.test_url = url;
        base.test_interval = interval;
        base.test_tolerance = tolerance;
        self.workspace_save_defaults(defaults, window, cx);
        cx.notify();
    }

    pub fn workspace_load_global_defaults(&mut self) {
        self.workspace.rules_updated_at = self
            .store
            .lock()
            .ok()
            .and_then(|store| store.get_setting("builtin-rules-updated-at").ok().flatten())
            .and_then(|value| value.parse().ok());
        self.workspace.global_defaults = GenerationDefaults::from_store(&self.store);
        if let Some(defaults) = &self.workspace.global_defaults {
            defaults.apply(&mut self.options);
        }
    }

    fn workspace_save_defaults(
        &mut self,
        defaults: GenerationDefaults,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Err(error) = defaults.validate() {
            self.template_error = Some(error);
            cx.notify();
            return;
        }
        let result = serde_json::to_string(&defaults)
            .map_err(|e| e.to_string())
            .and_then(|json| {
                lock_store(&self.store)
                    .set_setting("generation_defaults", &json)
                    .map_err(|e| e.to_string())
            });
        if let Err(error) = result {
            self.template_error = Some(error);
            cx.notify();
            return;
        }
        defaults.apply(&mut self.options);
        if let Some(base) = &mut self.template_base_options {
            defaults.apply(base);
        }
        self.workspace.global_defaults = Some(defaults);
        self.template_error = None;
        self.template_baseline = Some(self.active_template_snapshot());
        self.refresh_advanced_draft(true, window, cx);
        self.regenerate(cx);
        self.show_toast("全局生成设置已保存".into(), cx);
    }

    pub fn workspace_toggle_dns(
        &mut self,
        checked: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let base = self
            .template_base_options
            .get_or_insert_with(|| self.options.clone());
        base.dns_override = checked;
        if !checked {
            base.dns = swiftsub_core::generator::DnsOptions::default();
        }
        if let Some(defaults) = &self.workspace.global_defaults {
            defaults.apply(base);
        }
        self.refresh_advanced_draft(true, window, cx);
        if let Err(error) = self.autosave_routing(window, cx) {
            self.template_error = Some(error);
        }
        cx.notify();
    }

    fn workspace_rule_binding(&self, id: &str) -> Option<RuleSetBinding> {
        if let Some(binding) = self.template_custom_rule_sets.iter().find(|b| b.id == id) {
            return Some(binding.clone());
        }
        let base = self.template_base_options.as_ref().unwrap_or(&self.options);
        for module in catalog::MODULES {
            if let Some(spec) = module.rule_sets.iter().find(|s| s.id == id) {
                return Some(RuleSetBinding {
                    id: id.into(),
                    name: id.into(),
                    url: format!(
                        "{}/{}",
                        base.rule_provider_base_url.trim_end_matches('/'),
                        spec.path
                    ),
                    behavior: spec.behavior.as_str().into(),
                    format: "mrs".into(),
                    target: self
                        .template_rule_set_targets
                        .get(id)
                        .cloned()
                        .unwrap_or_else(|| module.name.into()),
                    no_resolve: spec.no_resolve,
                });
            }
        }
        for source in rule_catalog::SOURCES {
            if let Some(entry) = source
                .entries
                .iter()
                .find(|e| format!("{}-{}", source.id, e.id) == id)
            {
                return Some(rule_catalog::binding_for(
                    source,
                    entry,
                    rule_catalog::mirror_from_base(&base.rule_provider_base_url),
                    &base.rule_provider_base_url,
                    &self.template_enabled_modules,
                ));
            }
        }
        None
    }

    pub fn workspace_view_rule_content(
        &mut self,
        id: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Err(error) = self.workspace_checkpoint_rule_content(cx) {
            self.workspace.rule_content_error = Some(error);
            cx.notify();
            return;
        }
        let Some(binding) = self
            .workspace_rule_choices()
            .into_iter()
            .find(|choice| choice.binding.id == id)
            .map(|choice| choice.binding)
            .or_else(|| self.workspace_rule_binding(&id))
        else {
            return;
        };
        self.workspace.rule_content_request += 1;
        let request = self.workspace.rule_content_request;
        let key = Self::workspace_rule_cache_key(&binding);
        if !self.workspace.rule_content_cache.contains_key(&key)
            && let Some(cache) = self
                .store
                .lock()
                .ok()
                .and_then(|store| rule_updates::read_cache(&store, &key))
        {
            self.workspace
                .rule_cache_times
                .insert(key.clone(), cache.updated_at);
            self.workspace
                .rule_content_cache
                .insert(key.clone(), cache.payload);
        }
        self.workspace.rule_content_group = self
            .workspace
            .rules_target
            .clone()
            .unwrap_or_else(|| binding.target.clone());
        self.workspace.rule_content_id = Some(id.clone());
        self.workspace
            .reveal_group
            .replace(Some(self.workspace.rule_content_group.clone()));
        self.workspace.rule_content_binding = Some(binding.clone());
        self.workspace.group_settings_open = false;
        self.workspace.rule_metadata_id = None;
        self.workspace.rule_content_error = None;
        self.workspace.rule_content_editing = false;
        let base = self.template_base_options.as_ref().unwrap_or(&self.options);
        if let Some(payload) = base
            .rule_set_overrides
            .get(&id)
            .cloned()
            .or_else(|| self.workspace.rule_content_cache.get(&key).cloned())
        {
            self.workspace_show_rule_payload(payload, window, cx);
            cx.notify();
            return;
        }
        self.workspace.rule_content_loading = true;
        self.workspace.rule_content_original.clear();
        self.workspace
            .rule_content
            .update(cx, |s, cx| s.set_value("", window, cx));
        let fetch_opts = self.fetch_options();
        cx.spawn_in(window, async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move { fetch_rule_payload(&binding, &fetch_opts) })
                .await;
            let _ = cx.update(|window, cx| {
                this.update(cx, |this, cx| {
                    if this.workspace.rule_content_id.as_ref() != Some(&id)
                        || this.workspace.rule_content_request != request
                    {
                        return;
                    }
                    this.workspace.rule_content_loading = false;
                    match result {
                        Ok(payload) => {
                            if let Ok(store) = this.store.lock()
                                && let Ok(cache) =
                                    rule_updates::save_cache(&store, &key, payload.clone())
                            {
                                this.workspace
                                    .rule_cache_times
                                    .insert(key.clone(), cache.updated_at);
                            }
                            this.workspace
                                .rule_content_cache
                                .insert(key, payload.clone());
                            this.workspace_show_rule_payload(payload, window, cx);
                        }
                        Err(error) => {
                            this.workspace.rule_content_error = Some(error);
                            if let Some(binding) = &this.workspace.rule_content_binding
                                && let Some(draft) = this
                                    .workspace_rule_content_drafts()
                                    .get(&Self::workspace_rule_cache_key(binding))
                                    .cloned()
                            {
                                this.workspace.rule_content_editing =
                                    this.workspace_rule_choices().iter().any(|choice| {
                                        choice.active
                                            && choice.binding.target
                                                == this.workspace.rule_content_group
                                            && Some(&choice.binding.id)
                                                == this.workspace.rule_content_id.as_ref()
                                    });
                                this.workspace
                                    .rule_content
                                    .update(cx, |s, cx| s.set_value(draft, window, cx));
                            }
                        }
                    }
                    cx.notify();
                })
            });
        })
        .detach();
        cx.notify();
    }

    pub fn workspace_save_rule_content(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(binding) = self.workspace.rule_content_binding.clone() else {
            return;
        };
        if let Err(error) = self.workspace_checkpoint_rule_content(cx) {
            self.workspace.rule_content_error = Some(error);
            cx.notify();
            return;
        }
        let text = self.workspace.rule_content.read(cx).value().to_string();
        let payload =
            match swiftsub_core::generator::parse_rule_set_payload(&text, &binding.behavior) {
                Ok(payload) => payload,
                Err(error) => {
                    self.workspace.rule_content_error = Some(error.to_string());
                    cx.notify();
                    return;
                }
            };
        let original_base = self.template_base_options.clone();
        let normalized = payload.join("\n");
        self.template_base_options
            .get_or_insert_with(|| self.options.clone())
            .rule_set_overrides
            .insert(binding.id.clone(), payload);
        match self.autosave_routing(window, cx) {
            Ok(()) => {
                if let Err(error) = self.workspace_clear_rule_content_draft() {
                    self.workspace.rule_content_error = Some(error);
                    cx.notify();
                    return;
                }
                self.workspace.rule_content_original = normalized.clone();
                self.workspace
                    .rule_content
                    .update(cx, |input, cx| input.set_value(normalized, window, cx));
                self.workspace.rule_content_editing = false;
                self.workspace.rule_content_error = None;
                self.show_toast("规则集修改已保存到当前配置".into(), cx);
            }
            Err(error) => {
                self.template_base_options = original_base;
                self.workspace.rule_content_error = Some(error);
            }
        }
        cx.notify();
    }

    pub fn workspace_restore_rule_content(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(id) = self.workspace.rule_content_id.clone() else {
            return;
        };
        if let Err(error) = self.workspace_checkpoint_rule_content(cx) {
            self.workspace.rule_content_error = Some(error);
            cx.notify();
            return;
        }
        let original = self.template_base_options.clone();
        self.template_base_options
            .get_or_insert_with(|| self.options.clone())
            .rule_set_overrides
            .remove(&id);
        if let Err(error) = self.autosave_routing(window, cx) {
            self.template_base_options = original;
            self.workspace.rule_content_error = Some(error);
            cx.notify();
            return;
        }
        if let Err(error) = self.workspace_clear_rule_content_draft() {
            self.workspace.rule_content_error = Some(error);
            cx.notify();
            return;
        }
        self.workspace.rule_content_editing = false;
        if let Some(binding) = &self.workspace.rule_content_binding {
            self.workspace
                .rule_content_cache
                .remove(&Self::workspace_rule_cache_key(binding));
        }
        self.workspace_view_rule_content(id, window, cx);
    }

    pub fn workspace_edit_filter(
        &mut self,
        index: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(group) = self.custom_groups.get(index).cloned() else {
            return;
        };
        self.open_custom_dialog(CustomDialog::Filter, cx);
        self.workspace.edit_filter = Some(index);
        self.filter_regions = group.filter.regions.into_iter().collect();
        for (input, value) in [
            (&self.filter_name, group.name),
            (
                &self.filter_include,
                group.filter.include_keywords.join(", "),
            ),
            (
                &self.filter_exclude,
                group.filter.exclude_keywords.join(", "),
            ),
            (
                &self.workspace.filter_sources,
                group.filter.sources.join(", "),
            ),
        ] {
            input.update(cx, |input, cx| input.set_value(value, window, cx));
        }
    }

    pub fn workspace_edit_chain(
        &mut self,
        index: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(chain) = self.chains.get(index).cloned() else {
            return;
        };
        if !self.finish_workspace_fields(window, cx) {
            return;
        }
        self.custom_dialog = CustomDialog::None;
        self.custom_error = None;
        self.template_tab = TemplateTab::Tools;
        self.workspace.edit_chain = Some(index);
        let entry = chain.hops.first().unwrap_or(&chain.entry_node);
        let exit = chain.hops.last().unwrap_or(&chain.landing_node);
        let resolve = |selector: &str| {
            self.nodes
                .iter()
                .find(|node| node.source_key == selector || node.name() == selector)
                .map(|node| (node.source_key.clone(), node.name().to_string()))
        };
        let (entry_key, entry_name) =
            resolve(entry).unwrap_or_else(|| (entry.clone(), chain.entry_name.clone()));
        let (exit_key, exit_name) =
            resolve(exit).unwrap_or_else(|| (exit.clone(), chain.landing_name.clone()));
        self.chain_entry_key = Some(entry_key);
        self.chain_entry_name = Some(entry_name);
        self.chain_landing_key = Some(exit_key);
        self.chain_landing_name = Some(exit_name);
        self.chain_name
            .update(cx, |input, cx| input.set_value(chain.name, window, cx));
        let hops = chain
            .hops
            .iter()
            .map(|key| {
                self.nodes
                    .iter()
                    .find(|node| node.source_key == *key || node.name() == key)
                    .map(|node| node.name().to_string())
                    .unwrap_or_else(|| key.clone())
            })
            .collect::<Vec<_>>()
            .join(", ");
        self.workspace
            .chain_hops
            .update(cx, |input, cx| input.set_value(hops, window, cx));
    }
}

pub(super) fn contextual_rule(text: &str, target: &str) -> String {
    if let Ok(mut rule) = Rule::parse_body(text) {
        rule.target = target.into();
        return rule.to_line();
    }
    format!("{text},{target}")
}

pub(super) fn retarget_rule_lines(lines: &mut [String], old: &str, new: &str) {
    for line in lines {
        if let Ok(mut rule) = Rule::parse(line)
            && rule.target == old
        {
            rule.target = new.into();
            *line = rule.to_line();
        }
    }
}

pub(super) fn builtin_group_options(
    base: &GenerateOptions,
    enabled: &HashSet<String>,
    name: &str,
) -> GenerateOptions {
    let mut modules: Vec<_> = enabled.iter().cloned().collect();
    if let Some(module) = catalog::MODULES.iter().find(|module| module.name == name)
        && !modules.iter().any(|id| id == module.id)
    {
        modules.push(module.id.into());
    }
    GenerateOptions {
        preset: base.preset,
        enabled_modules: Some(modules),
        ..Default::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn global_defaults_validate_independently_of_unfinished_routing() {
        let mut options = GenerateOptions::default();
        options
            .custom_rules
            .push(Rule::new("aaa", "example.org", "missing"));
        let mut defaults = GenerationDefaults::from_options(&options);
        defaults.validate().unwrap();
        defaults.test_interval = 0;
        assert!(defaults.validate().is_err());
        defaults.test_interval = 300;
        defaults.dns.fake_ip_range = "198.18.0.1/99".into();
        assert!(defaults.validate().is_err());
    }

    #[test]
    fn global_defaults_only_override_dns_when_enabled_and_preserve_group_overrides() {
        let mut options = GenerateOptions::default();
        let original_dns = options.dns.clone();
        let mut defaults = GenerationDefaults::from_options(&options);
        defaults.dns.domestic = vec!["https://example.org/dns-query".into()];
        defaults.test_url = "https://example.org/204".into();
        defaults.test_interval = 600;
        options.policy_groups.push(PolicyGroupSpec {
            name: "Custom".into(),
            test_interval: Some(123),
            ..Default::default()
        });
        defaults.apply(&mut options);
        assert_eq!(options.dns, original_dns);
        assert_eq!(options.test_interval, 600);
        assert_eq!(options.policy_groups[0].test_interval, Some(123));
        options.dns_override = true;
        let roundtrip: GenerationDefaults =
            serde_json::from_str(&serde_json::to_string(&defaults).unwrap()).unwrap();
        roundtrip.apply(&mut options);
        assert_eq!(options.dns, defaults.dns);
    }

    #[test]
    fn contextual_rules_keep_no_resolve_and_bind_only_current_target() {
        let line = contextual_rule("IP-CIDR,192.0.2.0/24,no-resolve", "出口");
        let rule = Rule::parse(&line).unwrap();
        assert_eq!(rule.target, "出口");
        assert!(rule.no_resolve);
        assert_eq!(
            contextual_rule("DOMAIN-SUFFIX,example.org", "AI"),
            "DOMAIN-SUFFIX,example.org,AI"
        );
    }

    #[test]
    fn renamed_targets_keep_global_rule_positions_and_options() {
        let mut order = vec![
            "DOMAIN-SUFFIX,example.org,旧组".into(),
            "IP-CIDR,192.0.2.0/24,其他,no-resolve".into(),
            "MATCH,旧组".into(),
        ];
        retarget_rule_lines(&mut order, "旧组", "新组");
        assert_eq!(
            order,
            [
                "DOMAIN-SUFFIX,example.org,新组",
                "IP-CIDR,192.0.2.0/24,其他,no-resolve",
                "MATCH,新组"
            ]
        );
    }

    #[test]
    fn group_rename_preserves_invalid_rules_for_visible_repair() {
        let mut lines = vec![
            "aaa".into(),
            "# keep comment".into(),
            "DOMAIN-SUFFIX,example.org,旧组".into(),
        ];
        retarget_rule_lines(&mut lines, "旧组", "新组");
        assert_eq!(lines[0], "aaa");
        assert_eq!(lines[1], "# keep comment");
        assert_eq!(lines[2], "DOMAIN-SUFFIX,example.org,新组");
        let error = parse_custom_rule_lines(&lines.join("\n")).unwrap_err();
        assert!(error.contains("自定义规则第 1 行"));
        assert!(error.contains("aaa"));
    }

    #[test]
    fn opening_builtin_group_ignores_unrelated_invalid_routing() {
        let base = GenerateOptions {
            extra_yaml: "aaa: [".into(),
            rule_set_overrides: [("openai".into(), vec!["".into()])].into(),
            custom_rules: vec![Rule::match_all("不存在的组")],
            ..Default::default()
        };
        let nodes = build_nodes(
            vec![Proxy::new("direct", "校验节点", "127.0.0.1", 1)],
            "validation",
            "validation",
        );
        let options = builtin_group_options(&base, &HashSet::new(), "AI 服务");
        let generated = generate_profile(&nodes, &options).unwrap();
        assert!(generated.groups.iter().any(|group| group.name == "AI 服务"));
        assert!(
            generated
                .groups
                .iter()
                .any(|group| group.name == "节点选择")
        );
    }
}
