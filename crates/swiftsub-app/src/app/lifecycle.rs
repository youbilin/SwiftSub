use super::*;

#[derive(Serialize, Deserialize)]
struct SavedDraft {
    snapshot: TemplateSnapshot,
    #[serde(default)]
    node_order: Option<Vec<String>>,
    name: String,
    edit_id: Option<i64>,
    raw_rules: String,
    raw_advanced: String,
    #[serde(default)]
    policy_edit: Option<usize>,
    #[serde(default)]
    policy_name: String,
    #[serde(default)]
    health_url: String,
    #[serde(default)]
    health_interval: String,
    #[serde(default)]
    description: String,
    #[serde(default)]
    tolerance: String,
    #[serde(default)]
    chain_edit: Option<usize>,
    #[serde(default)]
    chain_name: String,
    #[serde(default)]
    chain_entry_key: Option<String>,
    #[serde(default)]
    chain_entry_name: Option<String>,
    #[serde(default)]
    chain_exit_key: Option<String>,
    #[serde(default)]
    chain_exit_name: Option<String>,
}

impl SwiftSubApp {
    pub(super) fn start_desktop_lifecycle(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.close_to_tray = lock_store(&self.store)
            .get_setting("close_to_tray")
            .ok()
            .flatten()
            .as_deref()
            == Some("true");
        let weak = cx.entity().downgrade();
        window.on_window_should_close(cx, move |window, cx| {
            weak.update(cx, |this, cx| {
                this.persist_profile_pulls(cx);
                if let Err(e) = this.autosave_saved_editor(window, cx) {
                    this.saved_config_error = Some(e);
                }
                if let Err(e) = this
                    .checkpoint_yaml_editor(cx)
                    .and_then(|_| this.checkpoint_draft(cx))
                {
                    this.show_toast(format!("草稿保存失败，窗口保持打开：{e}"), cx);
                    return false;
                }
                if this.close_to_tray && this.desktop.available {
                    this.desktop.hide();
                    return false;
                }
                true
            })
            .unwrap_or(true)
        });
        if std::env::args().any(|a| a == "--background") {
            self.desktop.hide();
        }
        cx.spawn_in(window, async move |this, cx| {
            let mut ticks = 0;
            loop {
                cx.background_executor()
                    .timer(Duration::from_millis(500))
                    .await;
                let Some(this) = this.upgrade() else {
                    break;
                };
                let result = cx.update(|window, cx| {
                    this.update(cx, |this, cx| {
                        ticks += 1;
                        if let Some(error) = this.publication.take_error() {
                            this.store_error =
                                Some(format!("后台配置更新失败，继续提供上次成功配置：{error}"));
                            cx.notify();
                        }
                        if this.server_bind_lan && ticks % 60 == 0 {
                            this.refresh_lan_interfaces(cx);
                        }
                        if ticks % 4 == 0 {
                            this.persist_profile_pulls(cx);
                            if this.saved_config_id.is_some() {
                                if let Err(e) = this.autosave_saved_editor(window, cx)
                                    && this.saved_config_error.as_ref() != Some(&e)
                                {
                                    this.saved_config_error = Some(e);
                                    cx.notify();
                                }
                                if let Err(e) = this.checkpoint_yaml_editor(cx)
                                    && this.saved_config_error.as_ref() != Some(&e)
                                {
                                    this.saved_config_error = Some(e);
                                    cx.notify();
                                }
                            }
                            if this.template_session_active
                                && let Err(e) = this.autosave_routing(window, cx)
                                && this.template_error.as_ref() != Some(&e)
                            {
                                this.template_error = Some(e);
                                cx.notify();
                            }
                            if let Err(e) = this.checkpoint_draft(cx) {
                                this.store_error = Some(format!("草稿自动保存失败：{e}"));
                                cx.notify();
                            }
                        }
                        for action in this.desktop.actions() {
                            match action {
                                crate::desktop::DesktopAction::Show => this.desktop.show(),
                                crate::desktop::DesktopAction::Copy => this.copy_server_url(cx),
                                crate::desktop::DesktopAction::Refresh => this.refresh_all(cx),
                                crate::desktop::DesktopAction::Exit => {
                                    this.persist_profile_pulls(cx);
                                    if let Err(e) = this.autosave_saved_editor(window, cx) {
                                        this.saved_config_error = Some(e);
                                    }
                                    match this
                                        .checkpoint_yaml_editor(cx)
                                        .and_then(|_| this.checkpoint_draft(cx))
                                    {
                                        Ok(()) => cx.quit(),
                                        Err(e) => {
                                            this.desktop.show();
                                            this.show_toast(format!("草稿保存失败：{e}"), cx);
                                        }
                                    }
                                }
                            }
                        }
                    })
                });
                if result.is_err() {
                    break;
                }
            }
        })
        .detach();
    }

    pub(super) fn checkpoint_draft(&self, cx: &Context<Self>) -> Result<(), String> {
        self.checkpoint_rule_visibility()?;
        self.checkpoint_generated_name(cx)?;
        self.workspace_checkpoint_rule_content(cx)?;
        if !self.template_session_active {
            return Ok(());
        }
        let mut snapshot = self
            .template_baseline
            .clone()
            .unwrap_or_else(|| self.active_template_snapshot());
        let base = self.template_base_options.as_ref().unwrap_or(&self.options);
        snapshot.preset = base.preset;
        snapshot.dns = base.dns.clone();
        snapshot.dns_override = base.dns_override;
        snapshot.rule_set_overrides = base.rule_set_overrides.clone();
        snapshot.listeners = base.listeners.clone();
        snapshot.extra_yaml = base.extra_yaml.clone();
        snapshot.enabled_modules = self.template_enabled_modules.iter().cloned().collect();
        snapshot.disabled_rule_sets = self.template_disabled_rule_sets.iter().cloned().collect();
        snapshot.custom_rule_sets = self.template_custom_rule_sets.clone();
        snapshot.policy_groups = self.template_policy_groups.clone();
        snapshot.group_order = self.template_group_order.clone();
        snapshot.rule_set_targets = self.template_rule_set_targets.clone();
        snapshot.rule_order = self.template_rule_order.clone();
        snapshot.custom_groups = self.custom_groups.clone();
        snapshot.chains = self.chains.clone();
        let draft = SavedDraft {
            snapshot,
            node_order: Some(self.node_order.clone()),
            name: self.template_name.read(cx).value().to_string(),
            edit_id: self.template_edit_id,
            raw_rules: self.template_custom_rules.read(cx).value().to_string(),
            raw_advanced: self.advanced_draft.read(cx).value().to_string(),
            policy_edit: self.template_policy_edit,
            policy_name: self.template_policy_rename.read(cx).value().to_string(),
            health_url: self.template_health_url.read(cx).value().to_string(),
            health_interval: self.template_health_interval.read(cx).value().to_string(),
            description: self.workspace.description.read(cx).value().to_string(),
            tolerance: self.workspace.tolerance.read(cx).value().to_string(),
            chain_edit: self.workspace.edit_chain,
            chain_name: self.chain_name.read(cx).value().to_string(),
            chain_entry_key: self.chain_entry_key.clone(),
            chain_entry_name: self.chain_entry_name.clone(),
            chain_exit_key: self.chain_landing_key.clone(),
            chain_exit_name: self.chain_landing_name.clone(),
        };
        let json = serde_json::to_string(&draft).map_err(|e| e.to_string())?;
        let key = format!("draft:{}", self.profile_id);
        if self
            .last_draft_checkpoint
            .borrow()
            .as_ref()
            .is_some_and(|(cached_key, cached_json)| cached_key == &key && cached_json == &json)
        {
            return Ok(());
        }
        let store = lock_store(&self.store);
        if store
            .get_setting(&key)
            .map_err(|e| e.to_string())?
            .as_deref()
            != Some(json.as_str())
        {
            store.set_setting(&key, &json).map_err(|e| e.to_string())?;
        }
        *self.last_draft_checkpoint.borrow_mut() = Some((key, json));
        Ok(())
    }
    pub(super) fn clear_saved_draft(&self) {
        let key = format!("draft:{}", self.profile_id);
        let _ = lock_store(&self.store).set_setting(&key, "");
        *self.last_draft_checkpoint.borrow_mut() = None;
    }
    pub(super) fn restore_draft(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let saved = lock_store(&self.store)
            .get_setting(&format!("draft:{}", self.profile_id))
            .ok()
            .flatten();
        let Some(saved) = saved.filter(|s| !s.is_empty()) else {
            return;
        };
        match serde_json::from_str::<SavedDraft>(&saved) {
            Ok(draft) => {
                self.load_template_draft(draft.snapshot, draft.name, draft.edit_id, window, cx);
                if let Some(order) = draft.node_order {
                    self.node_order = order;
                    self.apply_workspace_node_order();
                }
                self.template_custom_rules
                    .update(cx, |s, cx| s.set_value(draft.raw_rules, window, cx));
                self.workspace.rule_repair_open =
                    parse_custom_rule_lines(&self.template_custom_rules.read(cx).value()).is_err();
                self.advanced_draft
                    .update(cx, |s, cx| s.set_value(draft.raw_advanced, window, cx));
                self.template_policy_edit = draft
                    .policy_edit
                    .filter(|i| *i < self.template_policy_groups.len());
                self.template_policy_rename
                    .update(cx, |s, cx| s.set_value(draft.policy_name, window, cx));
                self.template_health_url
                    .update(cx, |s, cx| s.set_value(draft.health_url, window, cx));
                self.template_health_interval
                    .update(cx, |s, cx| s.set_value(draft.health_interval, window, cx));
                self.workspace
                    .description
                    .update(cx, |s, cx| s.set_value(draft.description, window, cx));
                self.workspace
                    .tolerance
                    .update(cx, |s, cx| s.set_value(draft.tolerance, window, cx));
                self.workspace.edit_chain =
                    draft.chain_edit.filter(|index| *index < self.chains.len());
                self.chain_name.update(cx, |input, cx| {
                    input.set_value(draft.chain_name, window, cx)
                });
                self.chain_entry_key = draft.chain_entry_key;
                self.chain_entry_name = draft.chain_entry_name;
                self.chain_landing_key = draft.chain_exit_key;
                self.chain_landing_name = draft.chain_exit_name;
                self.recovered_draft = true;
                self.template_error = None;
                if !std::env::args().any(|arg| arg == "--background") {
                    self.desktop.show();
                }
            }
            Err(e) => self.store_error = Some(format!("草稿无法读取，原数据已保留：{e}")),
        }
    }
}
