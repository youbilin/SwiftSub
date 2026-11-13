use super::*;

#[derive(Clone, Serialize, Deserialize)]
struct ProfileConfig {
    routing: TemplateSnapshot,
    #[serde(default)]
    membership: Option<HashSet<i64>>,
    #[serde(default = "default_filename")]
    filename: String,
}
fn default_filename() -> String {
    "swiftsub.yaml".into()
}

impl SwiftSubApp {
    pub fn refresh_profile_nodes(&mut self, cx: &mut Context<Self>) {
        self.rebuild_nodes(cx);
    }

    pub fn profile_includes(&self, id: i64) -> bool {
        self.profile_membership
            .as_ref()
            .is_none_or(|ids| ids.contains(&id))
    }

    fn current_profile_config(&self) -> ProfileConfig {
        ProfileConfig {
            routing: self
                .template_baseline
                .clone()
                .unwrap_or_else(|| self.active_template_snapshot()),
            membership: self.profile_membership.clone(),
            filename: self.subscription_filename.clone(),
        }
    }

    pub(super) fn save_active_profile(&self) -> Result<(), String> {
        if self.profile_id <= 0 {
            return Ok(());
        }
        let json =
            serde_json::to_string(&self.current_profile_config()).map_err(|e| e.to_string())?;
        self.store
            .lock()
            .expect("store mutex poisoned")
            .save_profile_options(self.profile_id, &json)
            .map_err(|e| e.to_string())
    }

    pub(super) fn restore_selected_profile(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let data = self
            .profiles
            .iter()
            .find(|p| p.id == self.profile_id)
            .and_then(|p| p.options_json.as_deref());
        if let Some(data) = data {
            match serde_json::from_str::<ProfileConfig>(data) {
                Ok(config) => {
                    if let Ok(options) =
                        config.routing.generate_options(&GenerateOptions::default())
                    {
                        self.options = options;
                        self.enabled_modules = config.routing.enabled_modules.into_iter().collect();
                        self.disabled_rule_sets =
                            config.routing.disabled_rule_sets.into_iter().collect();
                        self.custom_rule_sets = config.routing.custom_rule_sets;
                        self.custom_groups = config.routing.custom_groups;
                        self.chains = config.routing.chains;
                        self.profile_membership = config.membership;
                        self.subscription_filename = config.filename;
                    }
                }
                Err(e) => {
                    self.store_error = Some(format!("配置读取失败：{e}"));
                }
            }
        }
        let name = self
            .profiles
            .iter()
            .find(|p| p.id == self.profile_id)
            .map(|p| p.name.clone())
            .unwrap_or_default();
        self.profile_name_input
            .update(cx, |s, cx| s.set_value(name, window, cx));
        self.subscription_filename_input.update(cx, |s, cx| {
            s.set_value(self.subscription_filename.clone(), window, cx)
        });
        self.rule_provider_input.update(cx, |s, cx| {
            s.set_value(self.options.rule_provider_base_url.clone(), window, cx)
        });
        self.refresh_server_url();
    }

    pub fn switch_profile(&mut self, id: i64, window: &mut Window, cx: &mut Context<Self>) {
        if self.template_dialog_open {
            self.show_toast("请先应用或放弃分流草稿".into(), cx);
            return;
        }
        if let Err(e) = self.save_active_profile() {
            self.show_toast(e, cx);
            return;
        }
        self.profiles = self
            .store
            .lock()
            .expect("store mutex poisoned")
            .list_profiles()
            .unwrap_or_default();
        let Some(profile) = self.profiles.iter().find(|p| p.id == id).cloned() else {
            return;
        };
        self.profile_id = id;
        self.profile_token = profile.token;
        self.profile_delete_armed = false;
        self.profile_membership = None;
        self.disabled_nodes.clear();
        self.node_order.clear();
        self.restore_selected_profile(window, cx);
        let _ = self
            .store
            .lock()
            .expect("store mutex poisoned")
            .set_setting("active_profile", &id.to_string());
        self.rebuild_nodes(cx);
    }

    pub fn create_profile(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.template_dialog_open {
            self.show_toast("请先应用或放弃分流草稿".into(), cx);
            return;
        }
        let name = self.profile_name_input.read(cx).value().trim().to_string();
        if name.is_empty() || self.profiles.iter().any(|p| p.name == name) {
            self.show_toast("请输入未使用的配置名称".into(), cx);
            return;
        }
        let result = (|| {
            let store = self.store.lock().expect("store mutex poisoned");
            let profile = store
                .get_or_create_profile(&name)
                .map_err(|e| e.to_string())?;
            store
                .save_profile_options(
                    profile.id,
                    &serde_json::to_string(&self.current_profile_config())
                        .map_err(|e| e.to_string())?,
                )
                .map_err(|e| e.to_string())?;
            Ok::<_, String>(profile.id)
        })();
        match result {
            Ok(id) => self.switch_profile(id, window, cx),
            Err(e) => self.show_toast(e, cx),
        }
    }

    pub fn rename_profile(&mut self, cx: &mut Context<Self>) {
        let name = self.profile_name_input.read(cx).value().trim().to_string();
        if name.is_empty()
            || self
                .profiles
                .iter()
                .any(|p| p.id != self.profile_id && p.name == name)
        {
            self.show_toast("名称为空或已存在".into(), cx);
            return;
        }
        let result = self
            .store
            .lock()
            .expect("store mutex poisoned")
            .rename_profile(self.profile_id, &name);
        match result {
            Ok(()) => {
                self.profiles = self
                    .store
                    .lock()
                    .unwrap()
                    .list_profiles()
                    .unwrap_or_default();
                cx.notify();
            }
            Err(e) => self.show_toast(e.to_string(), cx),
        }
    }

    pub fn delete_active_profile(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.template_dialog_open {
            self.show_toast("请先关闭分流编辑器".into(), cx);
            return;
        }
        if self.profiles.len() < 2 {
            self.show_toast("至少保留一个配置".into(), cx);
            return;
        }
        if !self.profile_delete_armed {
            self.profile_delete_armed = true;
            self.show_toast("再次点击删除当前配置，其订阅地址将失效".into(), cx);
            return;
        }
        let result = self.store.lock().unwrap().delete_profile(self.profile_id);
        match result {
            Ok(true) => {
                self.profiles = self
                    .store
                    .lock()
                    .unwrap()
                    .list_profiles()
                    .unwrap_or_default();
                if let Some(p) = self.profiles.first() {
                    self.switch_profile(p.id, window, cx);
                }
            }
            Ok(false) => {}
            Err(e) => self.show_toast(e.to_string(), cx),
        }
    }

    pub fn toggle_profile_subscription(&mut self, id: i64, cx: &mut Context<Self>) {
        let mut ids = self
            .profile_membership
            .clone()
            .unwrap_or_else(|| self.subscriptions.iter().map(|s| s.id).collect());
        if !ids.remove(&id) {
            ids.insert(id);
        }
        self.profile_membership = Some(ids);
        self.rebuild_nodes(cx);
    }

    pub(super) fn publish_profiles(&self) {
        let Some(server) = &self.server else {
            return;
        };
        let profiles = self
            .store
            .lock()
            .expect("store mutex poisoned")
            .list_profiles()
            .unwrap_or_default();
        let mut published = HashMap::new();
        for profile in profiles {
            let (yaml, userinfo) = if profile.id == self.profile_id {
                (self.preview.clone(), self.aggregate_userinfo())
            } else {
                // Rebuild inactive profiles from current shared source payloads, preserving membership.
                let generated = profile
                    .options_json
                    .as_deref()
                    .and_then(|v| serde_json::from_str::<ProfileConfig>(v).ok())
                    .and_then(|config| {
                        let options = config
                            .routing
                            .generate_options(&GenerateOptions::default())
                            .ok()?;
                        let store = self.store.lock().ok()?;
                        let mut nodes = Vec::new();
                        for sub in self.subscriptions.iter().filter(|s| {
                            s.enabled
                                && config
                                    .membership
                                    .as_ref()
                                    .is_none_or(|ids| ids.contains(&s.id))
                        }) {
                            if let Some(parsed) =
                                sub.payload.as_deref().and_then(|p| parse_content(p).ok())
                            {
                                let mut imported = build_nodes(
                                    parsed.proxies,
                                    sub.id.to_string(),
                                    sub.name.clone(),
                                );
                                let overrides =
                                    store.list_node_overrides(sub.id).unwrap_or_default();
                                for n in &mut imported {
                                    if let Some(o) = overrides.get(&n.source_key) {
                                        if let Some(extras) = &o.extras {
                                            if let Ok(proxy) = Proxy::from_yaml_text(extras) {
                                                n.proxy = proxy;
                                            }
                                        }
                                        if let Some(alias) = &o.alias {
                                            n.set_name(alias.clone());
                                        }
                                        if let Some(enabled) = o.enabled {
                                            n.enabled = enabled;
                                        }
                                        n.listen_port = o.listen_port;
                                    }
                                }
                                nodes.extend(imported);
                            }
                        }
                        generate_profile(&nodes, &options).ok().map(|p| p.yaml)
                    })
                    .unwrap_or_default();
                let _ = self
                    .store
                    .lock()
                    .unwrap()
                    .save_profile_yaml(profile.id, &generated);
                (generated, None)
            };
            published.insert(
                profile.token,
                ProfileSnapshot {
                    yaml,
                    userinfo_header: userinfo,
                    update_interval_hours: 24,
                },
            );
        }
        server.replace_profiles(published);
    }
}
