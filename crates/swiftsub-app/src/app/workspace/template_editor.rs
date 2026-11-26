use super::*;

#[derive(Clone)]
pub(crate) struct TemplateGroupRow {
    pub name: String,
    pub kind: GroupKind,
    pub custom_index: Option<usize>,
    pub builtin_module_id: Option<&'static str>,
    pub is_core: bool,
    pub is_aux: bool,
    pub rule_count: usize,
}

#[derive(Clone)]
pub(crate) struct TemplateRuleChoice {
    pub binding: RuleSetBinding,
    pub source: String,
    pub active: bool,
    pub enabled: bool,
    pub is_custom: bool,
}

pub struct TemplateEditor {
    pub open: bool,
    pub edit_id: Option<i64>,
    pub name: Entity<InputState>,
    pub base_preset: Preset,
    pub enabled_modules: HashSet<String>,
    pub disabled_rule_sets: HashSet<String>,
    pub rule_set_targets: HashMap<String, String>,
    pub policy_groups: Vec<PolicyGroupSpec>,
    pub custom_rule_sets: Vec<RuleSetBinding>,
    pub manual_rules: Vec<Rule>,
    pub expanded_group: Option<String>,
    pub new_group_open: bool,
    pub new_group_name: Entity<InputState>,
    pub new_group_kind: GroupKind,
    pub builtin_picker_open: bool,
    pub builtin_search: Entity<InputState>,
    pub renaming_group: Option<usize>,
    pub rename_input: Entity<InputState>,
    pub rule_add_open: bool,
    pub rule_add_tab: String,
    pub rule_search: Entity<InputState>,
    pub rule_source: String,
    pub new_rule_set_url: Entity<InputState>,
    pub manual_rule_input: Entity<InputState>,
    pub editing_manual_index: Option<usize>,
    pub editing_rule_set_id: Option<String>,
    pub edit_rs_name: Entity<InputState>,
    pub edit_rs_url: Entity<InputState>,
    pub edit_rs_behavior: String,
    pub edit_rs_format: String,
    pub edit_rs_target: String,
    pub edit_rs_no_resolve: bool,
    pub error: Option<String>,
    pub(crate) base_snapshot: Option<TemplateSnapshot>,
}

impl TemplateEditor {
    pub(super) fn new(window: &mut Window, cx: &mut Context<SwiftSubApp>) -> Self {
        Self {
            open: false,
            edit_id: None,
            name: cx.new(|cx| InputState::new(window, cx).placeholder("模板名称")),
            base_preset: Preset::Minimal,
            enabled_modules: HashSet::new(),
            disabled_rule_sets: HashSet::new(),
            rule_set_targets: HashMap::new(),
            policy_groups: Vec::new(),
            custom_rule_sets: Vec::new(),
            manual_rules: Vec::new(),
            expanded_group: None,
            new_group_open: false,
            new_group_name: cx.new(|cx| InputState::new(window, cx).placeholder("策略组名称")),
            new_group_kind: GroupKind::Select,
            builtin_picker_open: false,
            builtin_search: cx.new(|cx| InputState::new(window, cx).placeholder("筛选内置组…")),
            renaming_group: None,
            rename_input: cx.new(|cx| InputState::new(window, cx).placeholder("新名称")),
            rule_add_open: false,
            rule_add_tab: "规则库".into(),
            rule_search: cx.new(|cx| InputState::new(window, cx).placeholder("搜索规则集…")),
            rule_source: String::new(),
            new_rule_set_url: cx.new(|cx| {
                InputState::new(window, cx).placeholder("https://... (.mrs / .yaml / .txt)")
            }),
            manual_rule_input: cx
                .new(|cx| InputState::new(window, cx).placeholder("DOMAIN-SUFFIX,example.com")),
            editing_manual_index: None,
            editing_rule_set_id: None,
            edit_rs_name: cx.new(|cx| InputState::new(window, cx).placeholder("规则集名称")),
            edit_rs_url: cx.new(|cx| InputState::new(window, cx).placeholder("规则集链接")),
            edit_rs_behavior: "domain".into(),
            edit_rs_format: "mrs".into(),
            edit_rs_target: rule_catalog::SELECT_GROUP.to_string(),
            edit_rs_no_resolve: false,
            error: None,
            base_snapshot: None,
        }
    }

    fn reset_subpanels(&mut self) {
        self.rule_add_open = false;
        self.editing_manual_index = None;
        self.editing_rule_set_id = None;
        self.renaming_group = None;
        self.error = None;
    }
}

impl SwiftSubApp {
    pub fn open_template_editor_from_preset(
        &mut self,
        preset: Preset,
        suggested_name: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let modules: HashSet<String> = catalog::modules_for_preset(preset)
            .into_iter()
            .map(|module| module.id.to_string())
            .collect();
        let snapshot = TemplateSnapshot {
            preset,
            enabled_modules: modules.into_iter().collect(),
            disabled_rule_sets: Vec::new(),
            custom_rule_sets: Vec::new(),
            custom_rule_lines: Vec::new(),
            rule_order: Vec::new(),
            dns: self.options.dns.clone(),
            dns_override: self.options.dns_override,
            rule_set_overrides: HashMap::new(),
            listeners: Vec::new(),
            extra_yaml: String::new(),
            rule_provider_base_url: self.options.rule_provider_base_url.clone(),
            policy_groups: Vec::new(),
            group_order: Vec::new(),
            rule_set_targets: HashMap::new(),
            custom_groups: Vec::new(),
            chains: Vec::new(),
            test_url: self.options.test_url.clone(),
            test_interval: self.options.test_interval,
            test_tolerance: self.options.test_tolerance,
            mixed_port: self.options.mixed_port,
            allow_lan: self.options.allow_lan,
        };
        self.populate_template_editor(None, suggested_name, snapshot, window, cx);
    }

    pub fn open_template_editor_from_current(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let snapshot = self
            .draft_template_snapshot(cx)
            .unwrap_or_else(|_| self.active_template_snapshot());
        self.populate_template_editor(None, "", snapshot, window, cx);
    }

    pub fn open_edit_custom_template(
        &mut self,
        id: i64,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(template) = self.custom_templates.iter().find(|t| t.id == id).cloned() else {
            return;
        };
        let snapshot: TemplateSnapshot = match serde_json::from_str(&template.config_json) {
            Ok(snapshot) => snapshot,
            Err(error) => {
                self.show_toast(format!("模板解析失败：{error}"), cx);
                return;
            }
        };
        self.populate_template_editor(Some(id), &template.name, snapshot, window, cx);
    }

    fn populate_template_editor(
        &mut self,
        edit_id: Option<i64>,
        name: &str,
        mut snapshot: TemplateSnapshot,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        snapshot.remove_legacy_regions();
        let manual_rules =
            parse_custom_rule_lines(&snapshot.custom_rule_lines.join("\n")).unwrap_or_default();
        let editor = &mut self.workspace.template_editor;
        editor.open = true;
        editor.edit_id = edit_id;
        editor.base_preset = snapshot.preset;
        editor.enabled_modules = snapshot.enabled_modules.iter().cloned().collect();
        for required in ["select", "auto", "final"] {
            editor.enabled_modules.insert(required.to_string());
        }
        editor.disabled_rule_sets = snapshot.disabled_rule_sets.iter().cloned().collect();
        editor.rule_set_targets = snapshot.rule_set_targets.clone();
        editor.policy_groups = snapshot
            .policy_groups
            .iter()
            .filter(|g| !g.builtin && !g.removed)
            .cloned()
            .collect();
        editor.custom_rule_sets = snapshot.custom_rule_sets.clone();
        editor.manual_rules = manual_rules;
        editor.expanded_group = None;
        editor.new_group_open = false;
        editor.builtin_picker_open = false;
        editor.new_group_kind = GroupKind::Select;
        editor.rule_add_tab = "规则库".into();
        editor.rule_source.clear();
        editor.reset_subpanels();
        editor.base_snapshot = Some(snapshot);

        editor.name.update(cx, |input, cx| {
            input.set_value(name.to_string(), window, cx)
        });
        editor
            .new_group_name
            .update(cx, |input, cx| input.set_value("", window, cx));
        editor
            .builtin_search
            .update(cx, |input, cx| input.set_value("", window, cx));
        editor
            .rule_search
            .update(cx, |input, cx| input.set_value("", window, cx));
        editor
            .new_rule_set_url
            .update(cx, |input, cx| input.set_value("", window, cx));
        editor
            .manual_rule_input
            .update(cx, |input, cx| input.set_value("", window, cx));
        cx.notify();
    }

    pub fn close_template_editor(&mut self, cx: &mut Context<Self>) {
        self.workspace.template_editor.open = false;
        self.workspace.template_editor.reset_subpanels();
        cx.notify();
    }

    pub fn template_editor_handle_escape(&mut self, cx: &mut Context<Self>) -> bool {
        let editor = &mut self.workspace.template_editor;
        if !editor.open {
            return false;
        }
        if editor.editing_rule_set_id.take().is_some() {
            cx.notify();
            return true;
        }
        if editor.rule_add_open {
            editor.rule_add_open = false;
            editor.editing_manual_index = None;
            cx.notify();
            return true;
        }
        if editor.renaming_group.take().is_some() {
            cx.notify();
            return true;
        }
        if editor.new_group_open || editor.builtin_picker_open {
            editor.new_group_open = false;
            editor.builtin_picker_open = false;
            cx.notify();
            return true;
        }
        if editor.expanded_group.take().is_some() {
            cx.notify();
            return true;
        }
        self.close_template_editor(cx);
        true
    }

    fn template_editor_base_url(&self) -> &str {
        self.workspace
            .template_editor
            .base_snapshot
            .as_ref()
            .map(|s| s.rule_provider_base_url.as_str())
            .filter(|s| !s.is_empty())
            .unwrap_or(&self.options.rule_provider_base_url)
    }

    pub fn template_editor_targets(&self) -> Vec<String> {
        let editor = &self.workspace.template_editor;
        let mut targets = Vec::new();
        for module in catalog::MODULES {
            if !matches!(module.kind, catalog::ModuleKind::Region(_))
                && (matches!(module.id, "select" | "auto" | "final")
                    || editor.enabled_modules.contains(module.id))
                && !targets.iter().any(|t| t == module.name)
            {
                targets.push(module.name.to_string());
            }
        }
        for group in &editor.policy_groups {
            if !targets.iter().any(|t| t == &group.name) {
                targets.push(group.name.clone());
            }
        }
        for builtin in ["DIRECT", "REJECT"] {
            if !targets.iter().any(|t| t == builtin) {
                targets.push(builtin.to_string());
            }
        }
        targets
    }

    pub(crate) fn template_editor_rule_choices(&self) -> Vec<TemplateRuleChoice> {
        let editor = &self.workspace.template_editor;
        let base_url = self.template_editor_base_url();
        let active_targets: HashSet<String> = self.template_editor_targets().into_iter().collect();
        let mut choices = Vec::new();
        let mut seen = HashSet::new();

        for binding in &editor.custom_rule_sets {
            seen.insert(binding.id.clone());
            let source = rule_catalog::SOURCES
                .iter()
                .find(|s| {
                    s.entries
                        .iter()
                        .any(|e| format!("{}-{}", s.id, e.id) == binding.id)
                })
                .map(|s| s.name)
                .unwrap_or("自定义链接");
            let enabled = !editor.disabled_rule_sets.contains(&binding.id);
            choices.push(TemplateRuleChoice {
                binding: binding.clone(),
                source: source.into(),
                active: enabled && active_targets.contains(&binding.target),
                enabled,
                is_custom: true,
            });
        }

        for module in catalog::MODULES
            .iter()
            .filter(|m| !matches!(m.kind, catalog::ModuleKind::Region(_)))
        {
            for spec in module.rule_sets {
                if !seen.insert(spec.id.to_string()) {
                    continue;
                }
                let target = editor
                    .rule_set_targets
                    .get(spec.id)
                    .cloned()
                    .unwrap_or_else(|| module.name.to_string());
                let owned = editor.enabled_modules.contains(module.id) || target != module.name;
                let enabled = !editor.disabled_rule_sets.contains(spec.id);
                let active = enabled && owned && active_targets.contains(&target);
                choices.push(TemplateRuleChoice {
                    binding: RuleSetBinding {
                        id: spec.id.into(),
                        name: crate::ui::rule_set_label(spec.id),
                        url: format!("{}/{}", base_url.trim_end_matches('/'), spec.path),
                        behavior: spec.behavior.as_str().into(),
                        format: "mrs".into(),
                        target,
                        no_resolve: spec.no_resolve,
                    },
                    source: "内置".into(),
                    active,
                    enabled,
                    is_custom: false,
                });
            }
        }

        for source in rule_catalog::SOURCES {
            for entry in source.entries {
                let id = format!("{}-{}", source.id, entry.id);
                if !seen.insert(id) {
                    continue;
                }
                choices.push(TemplateRuleChoice {
                    binding: rule_catalog::binding_for(
                        source,
                        entry,
                        rule_catalog::mirror_from_base(base_url),
                        base_url,
                        &editor.enabled_modules,
                    ),
                    source: source.name.into(),
                    active: false,
                    enabled: true,
                    is_custom: false,
                });
            }
        }

        choices
    }

    pub(crate) fn template_editor_group_rule_sets(&self, target: &str) -> Vec<TemplateRuleChoice> {
        let editor = &self.workspace.template_editor;
        self.template_editor_rule_choices()
            .into_iter()
            .filter(|choice| {
                if choice.binding.target != target {
                    return false;
                }
                if choice.is_custom {
                    return true;
                }
                // Built-in rule set belongs to this group if its module is enabled or explicitly retargeted here
                catalog::MODULES.iter().any(|m| {
                    m.rule_sets.iter().any(|s| s.id == choice.binding.id)
                        && (editor.enabled_modules.contains(m.id)
                            || editor
                                .rule_set_targets
                                .get(&choice.binding.id)
                                .is_some_and(|t| t == target))
                })
            })
            .collect()
    }

    pub(crate) fn template_editor_group_rows(&self) -> Vec<TemplateGroupRow> {
        let editor = &self.workspace.template_editor;
        let all_choices = self.template_editor_rule_choices();

        let count_for = |target: &str| -> usize {
            let rs_count = all_choices
                .iter()
                .filter(|c| c.active && c.binding.target == target)
                .count();
            let manual_count = editor
                .manual_rules
                .iter()
                .filter(|r| r.target == target)
                .count();
            rs_count + manual_count
        };

        let mut rows = Vec::new();
        let make_builtin = |id: &'static str, is_core: bool| {
            catalog::module(id).map(|module| TemplateGroupRow {
                name: module.name.to_string(),
                kind: if id == "auto" {
                    GroupKind::UrlTest
                } else {
                    GroupKind::Select
                },
                custom_index: None,
                builtin_module_id: Some(module.id),
                is_core,
                is_aux: false,
                rule_count: count_for(module.name),
            })
        };

        rows.extend(make_builtin("select", true));
        rows.extend(make_builtin("auto", true));

        for module in catalog::MODULES {
            if !matches!(module.kind, catalog::ModuleKind::Region(_))
                && !matches!(module.id, "select" | "auto" | "final")
                && editor.enabled_modules.contains(module.id)
            {
                rows.extend(make_builtin(module.id, false));
            }
        }

        for (idx, group) in editor.policy_groups.iter().enumerate() {
            rows.push(TemplateGroupRow {
                name: group.name.clone(),
                kind: group.kind,
                custom_index: Some(idx),
                builtin_module_id: None,
                is_core: false,
                is_aux: false,
                rule_count: count_for(&group.name),
            });
        }

        rows.extend(make_builtin("final", true));

        for aux in ["DIRECT", "REJECT"] {
            rows.push(TemplateGroupRow {
                name: aux.to_string(),
                kind: GroupKind::Select,
                custom_index: None,
                builtin_module_id: None,
                is_core: false,
                is_aux: true,
                rule_count: count_for(aux),
            });
        }

        rows
    }

    pub fn template_editor_toggle_expand(&mut self, name: String, cx: &mut Context<Self>) {
        let editor = &mut self.workspace.template_editor;
        if editor.expanded_group.as_deref() == Some(&name) {
            editor.expanded_group = None;
        } else {
            editor.expanded_group = Some(name);
        }
        editor.reset_subpanels();
        cx.notify();
    }

    fn template_editor_retarget(&mut self, old: &str, new: &str) {
        let editor = &mut self.workspace.template_editor;
        for binding in &mut editor.custom_rule_sets {
            if binding.target == old {
                binding.target = new.to_string();
            }
        }
        for rule in &mut editor.manual_rules {
            if rule.target == old {
                rule.target = new.to_string();
            }
        }
        for target in editor.rule_set_targets.values_mut() {
            if target == old {
                *target = new.to_string();
            }
        }
    }

    pub fn template_editor_toggle_module(&mut self, module_id: String, cx: &mut Context<Self>) {
        if matches!(module_id.as_str(), "select" | "auto" | "final") {
            return;
        }
        let Some(module) = catalog::module(&module_id) else {
            return;
        };
        let was_enabled = self
            .workspace
            .template_editor
            .enabled_modules
            .contains(&module_id);
        if was_enabled {
            self.workspace
                .template_editor
                .enabled_modules
                .remove(&module_id);
            for spec in module.rule_sets {
                self.workspace
                    .template_editor
                    .rule_set_targets
                    .remove(spec.id);
            }
            self.template_editor_retarget(module.name, rule_catalog::SELECT_GROUP);
            if self.workspace.template_editor.expanded_group.as_deref() == Some(module.name) {
                self.workspace.template_editor.expanded_group = None;
                self.workspace.template_editor.reset_subpanels();
            }
        } else {
            self.workspace
                .template_editor
                .enabled_modules
                .insert(module_id);
            for spec in module.rule_sets {
                self.workspace
                    .template_editor
                    .disabled_rule_sets
                    .remove(spec.id);
            }
        }
        self.workspace.template_editor.error = None;
        cx.notify();
    }

    pub fn template_editor_add_group(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let name = self
            .workspace
            .template_editor
            .new_group_name
            .read(cx)
            .value()
            .trim()
            .to_string();
        if name.is_empty()
            || name.contains([',', '\n', '\r'])
            || [
                "DIRECT",
                "REJECT",
                "REJECT-DROP",
                "PASS",
                "COMPATIBLE",
                "GLOBAL",
            ]
            .iter()
            .any(|reserved| reserved.eq_ignore_ascii_case(&name))
        {
            self.workspace.template_editor.error = Some("策略组名称无效".into());
            cx.notify();
            return;
        }
        if self.template_editor_targets().iter().any(|t| t == &name)
            || catalog::MODULES.iter().any(|m| m.name == name)
        {
            self.workspace.template_editor.error = Some("该名称已存在".into());
            cx.notify();
            return;
        }
        let kind = self.workspace.template_editor.new_group_kind;
        let mut spec = PolicyGroupSpec {
            id: uuid::Uuid::new_v4().to_string(),
            name: name.clone(),
            kind,
            include_all_nodes: true,
            ..Default::default()
        };
        if kind == GroupKind::Select {
            spec.members.push("自动选择".into());
            spec.members.push("DIRECT".into());
        }
        let editor = &mut self.workspace.template_editor;
        editor.policy_groups.push(spec);
        editor.new_group_open = false;
        editor.expanded_group = Some(name);
        editor.reset_subpanels();
        editor
            .new_group_name
            .update(cx, |input, cx| input.set_value("", window, cx));
        cx.notify();
    }

    pub fn template_editor_remove_custom_group(&mut self, idx: usize, cx: &mut Context<Self>) {
        if idx >= self.workspace.template_editor.policy_groups.len() {
            return;
        }
        let removed = self.workspace.template_editor.policy_groups.remove(idx);
        self.template_editor_retarget(&removed.name, rule_catalog::SELECT_GROUP);
        let editor = &mut self.workspace.template_editor;
        if editor.expanded_group.as_deref() == Some(&removed.name) {
            editor.expanded_group = None;
            editor.reset_subpanels();
        }
        editor.error = None;
        cx.notify();
    }

    pub fn template_editor_start_rename(
        &mut self,
        idx: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(name) = self
            .workspace
            .template_editor
            .policy_groups
            .get(idx)
            .map(|g| g.name.clone())
        else {
            return;
        };
        let editor = &mut self.workspace.template_editor;
        editor.renaming_group = Some(idx);
        editor
            .rename_input
            .update(cx, |input, cx| input.set_value(name, window, cx));
        cx.notify();
    }

    pub fn template_editor_commit_rename(&mut self, cx: &mut Context<Self>) {
        let Some(idx) = self.workspace.template_editor.renaming_group else {
            return;
        };
        let new_name = self
            .workspace
            .template_editor
            .rename_input
            .read(cx)
            .value()
            .trim()
            .to_string();
        let Some(old_name) = self
            .workspace
            .template_editor
            .policy_groups
            .get(idx)
            .map(|g| g.name.clone())
        else {
            self.workspace.template_editor.renaming_group = None;
            cx.notify();
            return;
        };
        if new_name == old_name {
            self.workspace.template_editor.renaming_group = None;
            cx.notify();
            return;
        }
        if new_name.is_empty()
            || new_name.contains([',', '\n', '\r'])
            || [
                "DIRECT",
                "REJECT",
                "REJECT-DROP",
                "PASS",
                "COMPATIBLE",
                "GLOBAL",
            ]
            .iter()
            .any(|reserved| reserved.eq_ignore_ascii_case(&new_name))
        {
            self.workspace.template_editor.error = Some("策略组名称无效".into());
            cx.notify();
            return;
        }
        if self
            .template_editor_targets()
            .iter()
            .any(|t| t == &new_name)
            || catalog::MODULES.iter().any(|m| m.name == new_name)
        {
            self.workspace.template_editor.error = Some("该名称已存在".into());
            cx.notify();
            return;
        }
        self.workspace.template_editor.policy_groups[idx].name = new_name.clone();
        self.template_editor_retarget(&old_name, &new_name);
        let editor = &mut self.workspace.template_editor;
        if editor.expanded_group.as_deref() == Some(&old_name) {
            editor.expanded_group = Some(new_name);
        }
        editor.renaming_group = None;
        editor.error = None;
        cx.notify();
    }

    pub fn template_editor_toggle_rule_set(
        &mut self,
        id: String,
        enabled: bool,
        cx: &mut Context<Self>,
    ) {
        let disabled = &mut self.workspace.template_editor.disabled_rule_sets;
        if enabled {
            disabled.remove(&id);
        } else {
            disabled.insert(id);
        }
        cx.notify();
    }

    pub fn template_editor_remove_rule_set(&mut self, id: String, cx: &mut Context<Self>) {
        let editor = &mut self.workspace.template_editor;
        let was_custom = editor.custom_rule_sets.iter().any(|b| b.id == id);
        if was_custom {
            editor.custom_rule_sets.retain(|b| b.id != id);
            editor.disabled_rule_sets.remove(&id);
        } else {
            editor.disabled_rule_sets.insert(id.clone());
            editor.rule_set_targets.remove(&id);
        }
        if editor.editing_rule_set_id.as_deref() == Some(&id) {
            editor.editing_rule_set_id = None;
        }
        cx.notify();
    }

    pub fn template_editor_add_catalog_rule(
        &mut self,
        id: String,
        target: String,
        cx: &mut Context<Self>,
    ) {
        let Some(choice) = self
            .template_editor_rule_choices()
            .into_iter()
            .find(|c| c.binding.id == id)
        else {
            return;
        };
        let editor = &mut self.workspace.template_editor;
        let is_aux = matches!(target.as_str(), "DIRECT" | "REJECT" | "REJECT-DROP");
        if let Some(existing) = editor.custom_rule_sets.iter_mut().find(|b| b.id == id) {
            existing.target = target;
        } else if let Some(module) = catalog::MODULES
            .iter()
            .find(|m| m.rule_sets.iter().any(|s| s.id == id))
            && editor.enabled_modules.contains(module.id)
            && (!is_aux || target == module.name)
        {
            if target == module.name {
                editor.rule_set_targets.remove(&id);
            } else {
                editor.rule_set_targets.insert(id.clone(), target);
            }
        } else {
            let mut binding = choice.binding;
            binding.target = target;
            editor.custom_rule_sets.push(binding);
            editor.rule_set_targets.remove(&id);
        }
        editor.disabled_rule_sets.remove(&id);
        editor.error = None;
        cx.notify();
    }

    pub fn template_editor_add_url_rule(
        &mut self,
        target: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let url = self
            .workspace
            .template_editor
            .new_rule_set_url
            .read(cx)
            .value()
            .trim()
            .to_string();
        match rule_set_binding_from_url(&url) {
            Ok(mut binding) => {
                binding.target = target;
                let editor = &mut self.workspace.template_editor;
                if let Some(existing) = editor
                    .custom_rule_sets
                    .iter_mut()
                    .find(|item| item.url == binding.url)
                {
                    existing.target = binding.target;
                    let existing_id = existing.id.clone();
                    editor.disabled_rule_sets.remove(&existing_id);
                } else {
                    let base_id = binding.id.clone();
                    let mut suffix = 2;
                    while editor.custom_rule_sets.iter().any(|b| b.id == binding.id) {
                        binding.id = format!("{base_id}-{suffix}");
                        suffix += 1;
                    }
                    editor.disabled_rule_sets.remove(&binding.id);
                    editor.custom_rule_sets.push(binding);
                }
                editor.error = None;
                editor
                    .new_rule_set_url
                    .update(cx, |input, cx| input.set_value("", window, cx));
                cx.notify();
            }
            Err(error) => {
                self.workspace.template_editor.error = Some(error);
                cx.notify();
            }
        }
    }

    pub fn template_editor_start_edit_rule_set(
        &mut self,
        id: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(choice) = self
            .template_editor_rule_choices()
            .into_iter()
            .find(|c| c.binding.id == id)
        else {
            return;
        };
        let b = choice.binding;
        let editor = &mut self.workspace.template_editor;
        editor.editing_rule_set_id = Some(id);
        editor.rule_add_open = false;
        editor.edit_rs_behavior = b.behavior;
        editor.edit_rs_format = b.format;
        editor.edit_rs_target = b.target;
        editor.edit_rs_no_resolve = b.no_resolve;
        editor
            .edit_rs_name
            .update(cx, |input, cx| input.set_value(b.name, window, cx));
        editor
            .edit_rs_url
            .update(cx, |input, cx| input.set_value(b.url, window, cx));
        editor.error = None;
        cx.notify();
    }

    pub fn template_editor_save_rule_set(&mut self, cx: &mut Context<Self>) {
        let Some(id) = self.workspace.template_editor.editing_rule_set_id.clone() else {
            return;
        };
        let name = self
            .workspace
            .template_editor
            .edit_rs_name
            .read(cx)
            .value()
            .trim()
            .to_string();
        let url = self
            .workspace
            .template_editor
            .edit_rs_url
            .read(cx)
            .value()
            .trim()
            .to_string();
        let behavior = self.workspace.template_editor.edit_rs_behavior.clone();
        let format = self.workspace.template_editor.edit_rs_format.clone();
        let target = self.workspace.template_editor.edit_rs_target.clone();
        let no_resolve = self.workspace.template_editor.edit_rs_no_resolve;

        if name.is_empty()
            || !url::Url::parse(&url)
                .is_ok_and(|u| matches!(u.scheme(), "http" | "https") && u.host_str().is_some())
            || (behavior == "classical" && format == "mrs")
        {
            self.workspace.template_editor.error =
                Some("请填写有效名称与链接（classical 不支持 mrs）".into());
            cx.notify();
            return;
        }
        let binding = RuleSetBinding {
            id: id.clone(),
            name,
            url,
            behavior,
            format,
            target,
            no_resolve,
        };
        let editor = &mut self.workspace.template_editor;
        if let Some(existing) = editor.custom_rule_sets.iter_mut().find(|b| b.id == id) {
            *existing = binding;
        } else {
            editor.custom_rule_sets.push(binding);
        }
        editor.rule_set_targets.remove(&id);
        editor.editing_rule_set_id = None;
        editor.error = None;
        cx.notify();
    }

    pub fn template_editor_save_manual_rule(
        &mut self,
        target: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let text = self
            .workspace
            .template_editor
            .manual_rule_input
            .read(cx)
            .value()
            .trim()
            .to_string();
        if text.is_empty() {
            self.workspace.template_editor.error = Some("请输入规则内容".into());
            cx.notify();
            return;
        }
        let line = contextual_rule(&text, &target);
        let parsed = match parse_custom_rule_lines(&line) {
            Ok(mut rules) if rules.len() == 1 => rules.remove(0),
            _ => {
                self.workspace.template_editor.error =
                    Some("规则格式无效，例如 DOMAIN-SUFFIX,example.com".into());
                cx.notify();
                return;
            }
        };
        let editor = &mut self.workspace.template_editor;
        let line_str = parsed.to_line();
        if let Some(edit_idx) = editor.editing_manual_index {
            if editor
                .manual_rules
                .iter()
                .enumerate()
                .any(|(i, r)| i != edit_idx && r.to_line() == line_str)
            {
                editor.error = Some("该规则已存在".into());
                cx.notify();
                return;
            }
            if let Some(slot) = editor.manual_rules.get_mut(edit_idx) {
                *slot = parsed;
            }
            editor.editing_manual_index = None;
            editor.rule_add_open = false;
        } else {
            if editor.manual_rules.iter().any(|r| r.to_line() == line_str) {
                editor.error = Some("该规则已存在".into());
                cx.notify();
                return;
            }
            editor.manual_rules.push(parsed);
        }
        editor.error = None;
        editor
            .manual_rule_input
            .update(cx, |input, cx| input.set_value("", window, cx));
        cx.notify();
    }

    pub fn template_editor_start_edit_manual(
        &mut self,
        idx: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(body) = self
            .workspace
            .template_editor
            .manual_rules
            .get(idx)
            .map(Rule::to_body)
        else {
            return;
        };
        let editor = &mut self.workspace.template_editor;
        editor.editing_rule_set_id = None;
        editor.rule_add_open = true;
        editor.rule_add_tab = "手写".into();
        editor.editing_manual_index = Some(idx);
        editor
            .manual_rule_input
            .update(cx, |input, cx| input.set_value(body, window, cx));
        editor.error = None;
        cx.notify();
    }

    pub fn template_editor_remove_manual(&mut self, idx: usize, cx: &mut Context<Self>) {
        let editor = &mut self.workspace.template_editor;
        if idx >= editor.manual_rules.len() {
            return;
        }
        editor.manual_rules.remove(idx);
        match editor.editing_manual_index {
            Some(editing) if editing == idx => editor.editing_manual_index = None,
            Some(editing) if editing > idx => editor.editing_manual_index = Some(editing - 1),
            _ => {}
        }
        cx.notify();
    }

    pub fn save_template_editor(&mut self, cx: &mut Context<Self>) {
        let name = self
            .workspace
            .template_editor
            .name
            .read(cx)
            .value()
            .trim()
            .to_string();
        let edit_id = self.workspace.template_editor.edit_id;
        if name.is_empty() {
            self.workspace.template_editor.error = Some("请输入模板名称".into());
            cx.notify();
            return;
        }
        if self
            .custom_templates
            .iter()
            .any(|t| t.name == name && Some(t.id) != edit_id)
        {
            self.workspace.template_editor.error = Some("已有同名模板".into());
            cx.notify();
            return;
        }

        let valid_targets: HashSet<String> = self.template_editor_targets().into_iter().collect();
        let mut enabled: HashSet<String> = self.workspace.template_editor.enabled_modules.clone();
        for required in ["select", "auto", "final"] {
            enabled.insert(required.to_string());
        }

        let mut snapshot = self
            .workspace
            .template_editor
            .base_snapshot
            .clone()
            .unwrap_or_else(|| self.active_template_snapshot());
        snapshot.preset = self.workspace.template_editor.base_preset;
        snapshot.enabled_modules = enabled.iter().cloned().collect();
        snapshot.enabled_modules.sort();

        let mut disabled_rule_sets: Vec<String> = self
            .workspace
            .template_editor
            .disabled_rule_sets
            .iter()
            .cloned()
            .collect();
        disabled_rule_sets.sort();
        snapshot.disabled_rule_sets = disabled_rule_sets;

        snapshot.rule_set_targets = self
            .workspace
            .template_editor
            .rule_set_targets
            .iter()
            .filter(|(_, target)| valid_targets.contains(target.as_str()))
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect();

        // Keep non-tombstone built-in overrides only for enabled modules; attach custom groups
        let mut policy_groups: Vec<PolicyGroupSpec> = snapshot
            .policy_groups
            .into_iter()
            .filter(|g| {
                g.builtin
                    && !g.removed
                    && !g.disabled
                    && g.builtin_name().is_some_and(|origin| {
                        catalog::MODULES
                            .iter()
                            .find(|m| m.name == origin)
                            .is_some_and(|m| enabled.contains(m.id))
                    })
            })
            .collect();
        policy_groups.extend(self.workspace.template_editor.policy_groups.clone());
        snapshot.policy_groups = policy_groups;

        snapshot.custom_rule_sets = self.workspace.template_editor.custom_rule_sets.clone();
        snapshot.custom_rule_lines = self
            .workspace
            .template_editor
            .manual_rules
            .iter()
            .map(Rule::to_line)
            .collect();
        snapshot.chains.clear();

        if let Err(error) = self.validate_template_snapshot(&snapshot) {
            self.workspace.template_editor.error = Some(error);
            cx.notify();
            return;
        }
        let json = match serde_json::to_string(&snapshot) {
            Ok(json) => json,
            Err(error) => {
                self.workspace.template_editor.error = Some(error.to_string());
                cx.notify();
                return;
            }
        };
        let result = {
            let store = lock_store(&self.store);
            if let Some(id) = edit_id {
                store.update_custom_template(id, &name, &json)
            } else {
                store.add_custom_template(&name, &json).map(|_| ())
            }
        };
        match result {
            Ok(()) => {
                self.load_templates();
                self.workspace.template_editor.open = false;
                self.workspace.template_editor.reset_subpanels();
                self.show_toast(format!("模板「{name}」已保存"), cx);
                cx.notify();
            }
            Err(error) => {
                self.workspace.template_editor.error = Some(format!("保存失败：{error}"));
                cx.notify();
            }
        }
    }
}
