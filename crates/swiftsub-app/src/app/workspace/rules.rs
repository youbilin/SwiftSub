use super::*;

#[derive(Clone)]
pub(crate) struct WorkspaceRuleChoice {
    pub binding: RuleSetBinding,
    pub source: String,
    pub active: bool,
}

fn read_rule_content_drafts(
    store: &Store,
    profile_id: i64,
) -> Result<HashMap<String, String>, String> {
    let json = store
        .get_setting(&format!("rule-content-drafts:{profile_id}"))
        .map_err(|error| error.to_string())?;
    match json.filter(|json| !json.trim().is_empty()) {
        Some(json) => serde_json::from_str(&json)
            .map_err(|error| format!("规则正文草稿读取失败，原数据已保留：{error}")),
        None => Ok(HashMap::new()),
    }
}

fn clear_rule_content_draft(
    store: &Store,
    profile_id: i64,
    binding: &RuleSetBinding,
) -> Result<(), String> {
    let mut drafts = read_rule_content_drafts(store, profile_id)?;
    if drafts
        .remove(&SwiftSubApp::workspace_rule_cache_key(binding))
        .is_none()
    {
        return Ok(());
    }
    let json = serde_json::to_string(&drafts).map_err(|error| error.to_string())?;
    store
        .set_setting(&format!("rule-content-drafts:{profile_id}"), &json)
        .map_err(|error| error.to_string())
}

impl SwiftSubApp {
    pub(super) fn workspace_show_rule_payload(
        &mut self,
        payload: Vec<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let original = payload.join("\n");
        let draft = self
            .workspace
            .rule_content_binding
            .as_ref()
            .and_then(|binding| {
                self.workspace_rule_content_drafts()
                    .get(&Self::workspace_rule_cache_key(binding))
                    .cloned()
            });
        self.workspace.rule_content_editing = self.workspace_rule_choices().iter().any(|choice| {
            choice.active
                && choice.binding.target == self.workspace.rule_content_group
                && Some(&choice.binding.id) == self.workspace.rule_content_id.as_ref()
        });
        self.workspace.rule_content_original = original.clone();
        self.workspace.rule_content_loading = false;
        self.workspace.rule_content.update(cx, |input, cx| {
            input.set_value(
                if self.workspace.rule_content_editing {
                    draft.unwrap_or(original)
                } else {
                    original
                },
                window,
                cx,
            )
        });
    }
    pub(crate) fn workspace_rule_choices(&self) -> Vec<WorkspaceRuleChoice> {
        let mut choices = Vec::new();
        let base = self.template_base_options.as_ref().unwrap_or(&self.options);
        let mut seen = HashSet::new();
        let active_targets: HashSet<_> = self.workspace_active_rule_targets().into_iter().collect();
        for binding in &self.template_custom_rule_sets {
            seen.insert(binding.id.clone());
            let source = rule_catalog::SOURCES
                .iter()
                .find(|source| {
                    source
                        .entries
                        .iter()
                        .any(|entry| format!("{}-{}", source.id, entry.id) == binding.id)
                })
                .map(|source| source.name)
                .unwrap_or("自定义链接");
            choices.push(WorkspaceRuleChoice {
                binding: binding.clone(),
                source: source.into(),
                active: !self.template_disabled_rule_sets.contains(&binding.id)
                    && active_targets.contains(&binding.target),
            });
        }
        for module in catalog::MODULES
            .iter()
            .filter(|module| !matches!(module.kind, catalog::ModuleKind::Region(_)))
        {
            for spec in module.rule_sets {
                if !seen.insert(spec.id.into()) {
                    continue;
                }
                let target = self
                    .template_rule_set_targets
                    .get(spec.id)
                    .cloned()
                    .unwrap_or_else(|| self.workspace_resolved_group_name(module.name).into());
                let active = !self.template_disabled_rule_sets.contains(spec.id)
                    && active_targets.contains(&target)
                    && (self.template_enabled_modules.contains(module.id)
                        || target != self.workspace_resolved_group_name(module.name));
                choices.push(WorkspaceRuleChoice {
                    binding: RuleSetBinding {
                        id: spec.id.into(),
                        name: crate::ui::rule_set_label(spec.id),
                        url: format!(
                            "{}/{}",
                            base.rule_provider_base_url.trim_end_matches('/'),
                            spec.path
                        ),
                        behavior: spec.behavior.as_str().into(),
                        format: "mrs".into(),
                        target,
                        no_resolve: spec.no_resolve,
                    },
                    source: "内置规则".into(),
                    active,
                });
            }
        }
        for source in rule_catalog::SOURCES {
            for entry in source.entries {
                let id = format!("{}-{}", source.id, entry.id);
                if !seen.insert(id) {
                    continue;
                }
                choices.push(WorkspaceRuleChoice {
                    binding: rule_catalog::binding_for(
                        source,
                        entry,
                        rule_catalog::mirror_from_base(&base.rule_provider_base_url),
                        &base.rule_provider_base_url,
                        &self.template_enabled_modules,
                    ),
                    source: source.name.into(),
                    active: false,
                });
            }
        }
        choices
    }

    pub(crate) fn workspace_group_rules(&self, target: &str) -> Vec<WorkspaceRuleChoice> {
        self.workspace_rule_choices()
            .into_iter()
            .filter(|choice| choice.active && choice.binding.target == target)
            .collect()
    }

    pub fn workspace_group_settings(
        &mut self,
        name: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.workspace_close_rule_content(cx);
        if self.workspace.rule_content_id.is_some() {
            return;
        }
        let same = self
            .template_policy_edit
            .and_then(|index| self.template_policy_groups.get(index))
            .is_some_and(|group| group.name == name);
        if !same {
            self.workspace_rules_for(name.clone(), window, cx);
            if !self
                .template_policy_edit
                .and_then(|index| self.template_policy_groups.get(index))
                .is_some_and(|group| group.name == name)
            {
                return;
            }
        }
        self.workspace.group_settings_open = !self.workspace.group_settings_open;
        self.workspace.rule_picker_open = false;
        cx.notify();
    }

    pub fn workspace_begin_rule_picker(
        &mut self,
        target: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.workspace_close_rule_content(cx);
        if self.workspace.rule_content_id.is_some() {
            return;
        }
        if !self.finish_workspace_fields(window, cx) {
            return;
        }
        self.workspace.rules_target = Some(target.clone());
        self.template_rules_target_filter = target;
        self.workspace.rule_picker_open = true;
        self.workspace.manual_disabled_edit = None;
        self.workspace.rule_picker_kind = "规则库".into();
        self.workspace.rule_picks.clear();
        self.workspace.rule_metadata_id = None;
        self.workspace.group_settings_open = false;
        cx.notify();
    }

    pub(super) fn workspace_activate_rule_target(&mut self, target: &str) {
        if let Some(module) = catalog::MODULES
            .iter()
            .find(|module| module.name == self.workspace_builtin_name(target))
            && self.template_enabled_modules.insert(module.id.into())
        {
            // Enabling a target for one selected rule does not add its other preset rules.
            for spec in module.rule_sets {
                self.template_disabled_rule_sets.insert(spec.id.into());
            }
        }
        if let Some(group) = self
            .template_policy_groups
            .iter_mut()
            .find(|group| group.name == target)
        {
            group.disabled = false;
            group.removed = false;
        }
    }

    pub fn workspace_add_selected_rules(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.workspace_add_rules(self.workspace.rule_picks.clone(), true, window, cx);
    }

    pub fn workspace_add_rule(&mut self, id: String, window: &mut Window, cx: &mut Context<Self>) {
        self.workspace_add_rules(HashSet::from([id]), false, window, cx);
    }

    fn workspace_add_rules(
        &mut self,
        ids: HashSet<String>,
        close: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(target) = self.workspace.rules_target.clone() else {
            return;
        };
        if !self.finish_workspace_fields(window, cx) {
            return;
        }
        let choices: Vec<_> = self
            .workspace_rule_choices()
            .into_iter()
            .filter(|choice| ids.contains(&choice.binding.id))
            .collect();
        if choices.is_empty() {
            return;
        }
        let original_modules = self.template_enabled_modules.clone();
        let original_disabled = self.template_disabled_rule_sets.clone();
        let original_targets = self.template_rule_set_targets.clone();
        let original_bindings = self.template_custom_rule_sets.clone();
        let original_groups = self.template_policy_groups.clone();
        self.workspace_activate_rule_target(&target);
        for choice in choices {
            let id = choice.binding.id.clone();
            if let Some(binding) = self
                .template_custom_rule_sets
                .iter_mut()
                .find(|binding| binding.id == id)
            {
                binding.target = target.clone();
            } else if catalog::MODULES
                .iter()
                .any(|module| module.rule_sets.iter().any(|spec| spec.id == id))
            {
                self.template_rule_set_targets
                    .insert(id.clone(), target.clone());
            } else {
                let mut binding = choice.binding;
                binding.target = target.clone();
                self.template_custom_rule_sets.push(binding);
            }
            self.template_disabled_rule_sets.remove(&id);
        }
        match self.autosave_routing(window, cx) {
            Ok(()) => {
                if close {
                    self.workspace.rule_picker_open = false;
                }
                self.workspace.rule_picks.retain(|id| !ids.contains(id));
                if self
                    .workspace
                    .rule_content_id
                    .as_ref()
                    .is_some_and(|id| ids.contains(id))
                {
                    self.workspace.rule_content_editing = true;
                }
                self.show_toast(format!("规则已添加到 {target}"), cx);
            }
            Err(error) => {
                self.template_enabled_modules = original_modules;
                self.template_disabled_rule_sets = original_disabled;
                self.template_rule_set_targets = original_targets;
                self.template_custom_rule_sets = original_bindings;
                self.template_policy_groups = original_groups;
                self.template_error = Some(error);
            }
        }
        cx.notify();
    }

    pub(crate) fn workspace_active_rule_targets(&self) -> Vec<String> {
        let mut targets: Vec<String> = ["DIRECT", "REJECT", "REJECT-DROP"]
            .into_iter()
            .map(str::to_string)
            .collect();
        targets.extend(
            catalog::MODULES
                .iter()
                .filter(|module| {
                    (self.template_enabled_modules.contains(module.id)
                        || matches!(module.id, "select" | "auto" | "final"))
                        && !matches!(module.kind, catalog::ModuleKind::Region(_))
                        && !self.template_policy_groups.iter().any(|group| {
                            group.builtin_name() == Some(module.name)
                                && (group.disabled || group.removed)
                        })
                })
                .map(|module| self.workspace_resolved_group_name(module.name).to_string()),
        );
        targets.extend(
            self.template_policy_groups
                .iter()
                .filter(|group| !group.disabled && !group.removed)
                .map(|group| group.name.clone()),
        );
        targets.extend(
            self.custom_groups
                .iter()
                .filter(|group| !group.disabled)
                .map(|group| group.name.clone()),
        );
        targets.extend(
            self.chains
                .iter()
                .filter(|group| !group.disabled)
                .map(|group| group.name.clone()),
        );
        let mut seen = HashSet::new();
        targets.retain(|target| seen.insert(target.clone()));
        targets
    }

    pub(crate) fn workspace_rule_target_has_card(&self, target: &str) -> bool {
        catalog::MODULES.iter().any(|module| {
            module.name == target && !matches!(module.kind, catalog::ModuleKind::Region(_))
        }) || self
            .template_policy_groups
            .iter()
            .any(|group| group.name == target)
    }

    pub fn workspace_edit_rule_metadata(
        &mut self,
        id: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(choice) = self
            .workspace_rule_choices()
            .into_iter()
            .find(|choice| choice.binding.id == id)
        else {
            return;
        };
        let binding = choice.binding;
        self.workspace.rule_metadata_id = Some(id);
        self.workspace
            .rule_metadata_name
            .update(cx, |input, cx| input.set_value(binding.name, window, cx));
        self.workspace
            .rule_metadata_url
            .update(cx, |input, cx| input.set_value(binding.url, window, cx));
        self.workspace.rule_metadata_behavior = binding.behavior;
        self.workspace.rule_metadata_format = binding.format;
        self.workspace.rule_metadata_no_resolve = binding.no_resolve;
        self.workspace.rule_picker_open = false;
        cx.notify();
    }

    pub fn workspace_save_rule_metadata(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(id) = self.workspace.rule_metadata_id.clone() else {
            return;
        };
        let Some(mut binding) = self
            .workspace_rule_choices()
            .into_iter()
            .find(|choice| choice.binding.id == id)
            .map(|choice| choice.binding)
        else {
            return;
        };
        binding.name = self
            .workspace
            .rule_metadata_name
            .read(cx)
            .value()
            .trim()
            .into();
        binding.url = self
            .workspace
            .rule_metadata_url
            .read(cx)
            .value()
            .trim()
            .into();
        binding.behavior = self.workspace.rule_metadata_behavior.clone();
        binding.format = self.workspace.rule_metadata_format.clone();
        binding.no_resolve = self.workspace.rule_metadata_no_resolve;
        if binding.name.is_empty()
            || !url::Url::parse(&binding.url).is_ok_and(|url| {
                matches!(url.scheme(), "http" | "https") && url.host_str().is_some()
            })
            || (binding.behavior == "classical" && binding.format == "mrs")
        {
            self.template_error = Some(
                "请填写名称与有效的 HTTP / HTTPS 链接；classical 规则使用 YAML 或文本格式".into(),
            );
            cx.notify();
            return;
        }
        let original = self.template_custom_rule_sets.clone();
        if let Some(existing) = self
            .template_custom_rule_sets
            .iter_mut()
            .find(|existing| existing.id == id)
        {
            *existing = binding;
        } else {
            self.template_custom_rule_sets.push(binding);
        }
        match self.autosave_routing(window, cx) {
            Ok(()) => {
                self.workspace.rule_metadata_id = None;
                self.workspace_close_rule_content(cx);
            }
            Err(error) => {
                self.template_custom_rule_sets = original;
                self.template_error = Some(error);
            }
        }
        cx.notify();
    }

    pub(crate) fn workspace_rule_cache_key(binding: &RuleSetBinding) -> String {
        format!("{}|{}|{}", binding.id, binding.url, binding.behavior)
    }

    pub(super) fn workspace_rule_content_drafts(&self) -> HashMap<String, String> {
        self.workspace_read_rule_content_drafts()
            .unwrap_or_default()
    }

    pub(crate) fn workspace_read_rule_content_drafts(
        &self,
    ) -> Result<HashMap<String, String>, String> {
        let store = self.store.lock().map_err(|error| error.to_string())?;
        read_rule_content_drafts(&store, self.profile_id)
    }

    pub(crate) fn workspace_checkpoint_rule_content(
        &self,
        cx: &Context<Self>,
    ) -> Result<(), String> {
        let Some(binding) = &self.workspace.rule_content_binding else {
            return Ok(());
        };
        if self.workspace.rule_content_id.is_none()
            || !self.workspace.rule_content_editing
            || self.workspace.rule_content_loading
        {
            return Ok(());
        }
        let text = self.workspace.rule_content.read(cx).value().to_string();
        let key = Self::workspace_rule_cache_key(binding);
        let mut drafts = self.workspace_read_rule_content_drafts()?;
        if text == self.workspace.rule_content_original {
            drafts.remove(&key);
        } else {
            drafts.insert(key, text);
        }
        let json = serde_json::to_string(&drafts).map_err(|error| error.to_string())?;
        let store = self.store.lock().map_err(|error| error.to_string())?;
        let key = format!("rule-content-drafts:{}", self.profile_id);
        if store
            .get_setting(&key)
            .map_err(|error| error.to_string())?
            .as_deref()
            != Some(&json)
        {
            store
                .set_setting(&key, &json)
                .map_err(|error| error.to_string())?;
        }
        Ok(())
    }

    pub(super) fn workspace_clear_rule_content_draft(&self) -> Result<(), String> {
        let Some(binding) = &self.workspace.rule_content_binding else {
            return Ok(());
        };
        let store = self.store.lock().map_err(|error| error.to_string())?;
        clear_rule_content_draft(&store, self.profile_id, binding)
    }

    pub fn workspace_close_rule_content(&mut self, cx: &mut Context<Self>) {
        if let Err(error) = self.workspace_checkpoint_rule_content(cx) {
            self.workspace.rule_content_error = Some(error);
            cx.notify();
            return;
        }
        self.workspace.rule_content_id = None;
        self.workspace.rule_content_request += 1;
        cx.notify();
    }

    pub fn workspace_cancel_rule_content(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Err(error) = self.workspace_clear_rule_content_draft() {
            self.workspace.rule_content_error = Some(error);
            cx.notify();
            return;
        }
        self.workspace.rule_content_editing = false;
        let original = self.workspace.rule_content_original.clone();
        self.workspace
            .rule_content
            .update(cx, |input, cx| input.set_value(original, window, cx));
        self.workspace.rule_content_error = None;
        cx.notify();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cancelling_body_preserves_corrupt_and_unrelated_source_drafts() {
        let store = Store::open_in_memory().unwrap();
        let profile = store.get_or_create_profile("test").unwrap();
        let setting = format!("rule-content-drafts:{}", profile.id);
        let binding = RuleSetBinding {
            id: "test".into(),
            name: "test".into(),
            url: "https://example.org/test.yaml".into(),
            behavior: "domain".into(),
            format: "yaml".into(),
            target: "DIRECT".into(),
            no_resolve: false,
        };
        let corrupt = "{\"original-draft\":";
        store.set_setting(&setting, corrupt).unwrap();
        assert!(clear_rule_content_draft(&store, profile.id, &binding).is_err());
        assert_eq!(
            store.get_setting(&setting).unwrap().as_deref(),
            Some(corrupt)
        );
        let current = SwiftSubApp::workspace_rule_cache_key(&binding);
        let foreign = "test|https://old.example/test.yaml|domain";
        let drafts = HashMap::from([
            (current.clone(), "example.org".to_string()),
            (foreign.to_string(), "unfinished,body".to_string()),
        ]);
        store
            .set_setting(&setting, &serde_json::to_string(&drafts).unwrap())
            .unwrap();
        clear_rule_content_draft(&store, profile.id, &binding).unwrap();
        let remaining = read_rule_content_drafts(&store, profile.id).unwrap();
        assert!(!remaining.contains_key(&current));
        assert_eq!(remaining.get(foreign), drafts.get(foreign));
    }
}
