use super::*;

#[derive(Serialize, Deserialize)]
struct SavedDraft {
    snapshot: TemplateSnapshot,
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
}

impl SwiftSubApp {
    pub(super) fn start_desktop_lifecycle(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.close_to_tray = self
            .store
            .lock()
            .unwrap()
            .get_setting("close_to_tray")
            .ok()
            .flatten()
            .as_deref()
            == Some("true");
        let weak = cx.entity().downgrade();
        window.on_window_should_close(cx, move |window, cx| {
            weak.update(cx, |this, cx| {
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
                        if ticks % 4 == 0 {
                            if let Err(e) = this.autosave_saved_editor(window, cx) {
                                this.saved_config_error = Some(e);
                            }
                            if let Err(e) = this.checkpoint_yaml_editor(cx) {
                                this.saved_config_error = Some(e);
                            }
                            if let Err(e) = this.autosave_routing(window, cx) {
                                this.template_error = Some(e);
                            }
                            if let Err(e) = this.checkpoint_draft(cx) {
                                this.store_error = Some(format!("草稿自动保存失败：{e}"));
                            }
                        }
                        for action in this.desktop.actions() {
                            match action {
                                crate::desktop::DesktopAction::Show => this.desktop.show(),
                                crate::desktop::DesktopAction::Copy => this.copy_server_url(cx),
                                crate::desktop::DesktopAction::Refresh => this.refresh_all(cx),
                                crate::desktop::DesktopAction::Exit => {
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

    pub fn set_close_to_tray(&mut self, value: bool, cx: &mut Context<Self>) {
        if value && !self.desktop.available {
            self.show_toast("托盘不可用，关闭窗口将正常退出".into(), cx);
            return;
        }
        let result = self
            .store
            .lock()
            .unwrap()
            .set_setting("close_to_tray", if value { "true" } else { "false" });
        match result {
            Ok(()) => {
                self.close_to_tray = value;
                cx.notify();
            }
            Err(e) => self.show_toast(e.to_string(), cx),
        }
    }
    pub fn set_autostart(&mut self, value: bool, cx: &mut Context<Self>) {
        match crate::desktop::set_autostart(value) {
            Ok(()) => {
                self.autostart = value;
                cx.notify();
            }
            Err(e) => self.show_toast(format!("自启设置失败：{e}"), cx),
        }
    }

    pub(super) fn checkpoint_draft(&self, cx: &Context<Self>) -> Result<(), String> {
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
        snapshot.listeners = base.listeners.clone();
        snapshot.extra_yaml = base.extra_yaml.clone();
        snapshot.enabled_modules = self.template_enabled_modules.iter().cloned().collect();
        snapshot.disabled_rule_sets = self.template_disabled_rule_sets.iter().cloned().collect();
        snapshot.custom_rule_sets = self.template_custom_rule_sets.clone();
        snapshot.policy_groups = self.template_policy_groups.clone();
        snapshot.rule_set_targets = self.template_rule_set_targets.clone();
        snapshot.rule_order = self.template_rule_order.clone();
        snapshot.custom_groups = self.custom_groups.clone();
        snapshot.chains = self.chains.clone();
        let draft = SavedDraft {
            snapshot,
            name: self.template_name.read(cx).value().to_string(),
            edit_id: self.template_edit_id,
            raw_rules: self.template_custom_rules.read(cx).value().to_string(),
            raw_advanced: self.advanced_draft.read(cx).value().to_string(),
            policy_edit: self.template_policy_edit,
            policy_name: self.template_policy_rename.read(cx).value().to_string(),
            health_url: self.template_health_url.read(cx).value().to_string(),
            health_interval: self.template_health_interval.read(cx).value().to_string(),
        };
        let json = serde_json::to_string(&draft).map_err(|e| e.to_string())?;
        let store = self.store.lock().unwrap();
        let key = format!("draft:{}", self.profile_id);
        if store
            .get_setting(&key)
            .map_err(|e| e.to_string())?
            .as_deref()
            == Some(json.as_str())
        {
            return Ok(());
        }
        store.set_setting(&key, &json).map_err(|e| e.to_string())
    }
    pub(super) fn clear_saved_draft(&self) {
        let _ = self
            .store
            .lock()
            .unwrap()
            .set_setting(&format!("draft:{}", self.profile_id), "");
    }
    pub(super) fn restore_draft(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let saved = self
            .store
            .lock()
            .unwrap()
            .get_setting(&format!("draft:{}", self.profile_id))
            .ok()
            .flatten();
        let Some(saved) = saved.filter(|s| !s.is_empty()) else {
            return;
        };
        match serde_json::from_str::<SavedDraft>(&saved) {
            Ok(draft) => {
                self.load_template_draft(draft.snapshot, draft.name, draft.edit_id, window, cx);
                self.template_custom_rules
                    .update(cx, |s, cx| s.set_value(draft.raw_rules, window, cx));
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
                self.recovered_draft = true;
                self.template_error = None;
                self.desktop.show();
            }
            Err(e) => self.store_error = Some(format!("草稿无法读取，原数据已保留：{e}")),
        }
    }
}
