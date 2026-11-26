use super::*;

#[derive(Clone, Serialize, Deserialize)]
pub(super) struct ProfileConfig {
    pub(super) routing: TemplateSnapshot,
    #[serde(default)]
    pub(super) membership: Option<HashSet<i64>>,
    #[serde(default)]
    pub(super) node_order: Option<Vec<String>>,
    #[serde(default = "default_filename")]
    filename: String,
}
fn default_filename() -> String {
    "swiftsub.yaml".into()
}

fn validate_profile_recipe(json: &str) -> Result<ProfileConfig, String> {
    let config: ProfileConfig = serde_json::from_str(json).map_err(|error| error.to_string())?;
    config
        .routing
        .generate_options(&GenerateOptions::default())?;
    Ok(config)
}

impl SwiftSubApp {
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
            node_order: Some(self.node_order.clone()),
            filename: self.subscription_filename.clone(),
        }
    }

    pub(super) fn profile_snapshot_json(
        &self,
        snapshot: TemplateSnapshot,
    ) -> Result<String, String> {
        let mut config = self.current_profile_config();
        config.routing = snapshot;
        serde_json::to_string(&config).map_err(|e| e.to_string())
    }

    pub(super) fn save_active_profile(&self) -> Result<(), String> {
        if self.profile_id <= 0 {
            return Ok(());
        }
        let json =
            serde_json::to_string(&self.current_profile_config()).map_err(|e| e.to_string())?;
        let store = self.store.lock().map_err(|error| error.to_string())?;
        if let Some(previous) = store
            .get_profile_by_token(&self.profile_token)
            .map_err(|error| error.to_string())?
            && let Some(previous) = previous.options_json
        {
            validate_profile_recipe(&previous).map_err(|error| {
                format!("工作区原设置无法读取，已保留原设置和上次输出；请先修正分流：{error}")
            })?;
        }
        store
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
                Ok(mut config) => {
                    if let Some(order) = config.node_order {
                        self.node_order = order;
                    }
                    config.routing.remove_legacy_regions();
                    match config.routing.generate_options(&GenerateOptions::default()) {
                        Ok(options) => {
                            self.options = options;
                            self.enabled_modules =
                                config.routing.enabled_modules.into_iter().collect();
                            self.disabled_rule_sets =
                                config.routing.disabled_rule_sets.into_iter().collect();
                            self.custom_rule_sets = config.routing.custom_rule_sets;
                            self.custom_groups = config.routing.custom_groups;
                            self.chains = config.routing.chains;
                            self.profile_membership = config.membership;
                            self.subscription_filename = config.filename;
                        }
                        Err(error) => {
                            self.store_error =
                                Some(format!("工作区分流需要修正，原设置已保留：{error}"));
                            self.profile_membership = config.membership;
                            self.subscription_filename = config.filename;
                            self.load_template_draft(
                                config.routing,
                                String::new(),
                                None,
                                window,
                                cx,
                            );
                        }
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
        self.restore_generated_target(window, cx);
    }

    pub(super) fn publish_profiles(&self) {
        let Some(server) = &self.server else {
            return;
        };
        let snapshots = {
            let store = self.store.lock().expect("store mutex poisoned");
            let (Ok(profiles), Ok(configs)) = (store.list_profiles(), store.list_saved_configs())
            else {
                return;
            };
            let mut published = HashMap::new();
            for profile in profiles {
                let membership = profile
                    .options_json
                    .as_deref()
                    .and_then(|json| serde_json::from_str::<ProfileConfig>(json).ok())
                    .and_then(|config| config.membership);
                published.insert(
                    profile.token,
                    ProfileSnapshot {
                        yaml: profile.last_yaml.unwrap_or_default(),
                        userinfo_header: profile_userinfo(&self.subscriptions, membership.as_ref()),
                        update_interval_hours: 24,
                    },
                );
            }
            for config in configs {
                published.insert(
                    config.id,
                    ProfileSnapshot {
                        yaml: config.yaml,
                        update_interval_hours: 1,
                        ..Default::default()
                    },
                );
            }
            published
        };
        self.publication.submit(publication::PublishJob {
            store: Arc::clone(&self.store),
            publisher: server.publisher(),
            active_profile: self.profile_id,
            subscriptions: self.subscriptions.clone(),
            node_order: self.node_order.clone(),
            parsed_sources: self.parsed_sources.clone(),
            snapshots,
        });
    }
}

pub(super) fn profile_userinfo(
    subscriptions: &[StoredSubscription],
    membership: Option<&HashSet<i64>>,
) -> Option<String> {
    let mut info = SubscriptionInfo::default();
    let mut has_data = false;
    for s in subscriptions
        .iter()
        .filter(|s| s.enabled && membership.is_none_or(|ids| ids.contains(&s.id)))
    {
        info.upload = info.upload.saturating_add(s.traffic_upload);
        info.download = info.download.saturating_add(s.traffic_download);
        info.total = info.total.saturating_add(s.traffic_total);
        has_data |= s.traffic_upload > 0 || s.traffic_download > 0 || s.traffic_total > 0;
        if let Some(expire) = s.expire_at.filter(|e| *e > 0) {
            info.expire = Some(info.expire.map_or(expire, |old| old.min(expire)));
            has_data = true;
        }
    }
    has_data.then(|| info.to_header_value())
}
