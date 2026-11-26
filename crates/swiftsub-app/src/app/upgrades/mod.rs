use super::*;

pub struct Upgrades {
    pub profile_pulls: HashMap<String, (i64, String)>,
}

impl Upgrades {
    pub(super) fn new(_window: &mut Window, _cx: &mut Context<SwiftSubApp>) -> Self {
        Self {
            profile_pulls: HashMap::new(),
        }
    }
}

impl SwiftSubApp {
    pub(in crate::app) fn load_profile_pull_history(&mut self) {
        let store = self.store.lock().unwrap();
        for token in self
            .profiles
            .iter()
            .map(|profile| &profile.token)
            .chain(self.saved_configs.iter().map(|config| &config.id))
        {
            if let Ok(Some(value)) = store.get_setting(&format!("client-pull:{token}"))
                && let Ok(pull) = serde_json::from_str::<(i64, String)>(&value)
            {
                self.upgrades.profile_pulls.insert(token.clone(), pull);
            }
        }
    }

    pub(in crate::app) fn persist_profile_pulls(&mut self, cx: &mut Context<Self>) {
        let Some(server) = &self.server else {
            return;
        };
        let pulls = server.profile_pulls();
        let store = self.store.lock().unwrap();
        let mut changed = false;
        for (token, pull) in pulls {
            let value = (pull.at, pull.etag);
            if self.upgrades.profile_pulls.get(&token) == Some(&value) {
                continue;
            }
            match serde_json::to_string(&value)
                .map_err(|error| error.to_string())
                .and_then(|encoded| {
                    store
                        .set_setting(&format!("client-pull:{token}"), &encoded)
                        .map_err(|error| error.to_string())
                }) {
                Ok(()) => {
                    self.upgrades.profile_pulls.insert(token, value);
                    changed = true;
                }
                Err(error) => self.store_error = Some(format!("拉取时间保存失败：{error}")),
            }
        }
        if changed {
            cx.notify();
        }
    }

    pub fn profile_pull_status(&self, token: &str, yaml: &str) -> String {
        let Some(server) = &self.server else {
            return "本地订阅服务未启动".into();
        };
        let current = server.published_etag(token);
        if !server.published_matches(token, yaml) {
            return "本地订阅更新中 · 当前修改尚未发布".into();
        }
        let live = server
            .profile_pulls()
            .remove(token)
            .map(|pull| (pull.at, pull.etag));
        let previous = self.upgrades.profile_pulls.get(token);
        let pull = live.as_ref().or(previous);
        match pull {
            Some((at, etag)) => format!(
                "{} · 最近拉取 {}",
                if current.as_ref() == Some(etag) {
                    "当前版本已拉取"
                } else {
                    "等待拉取当前版本"
                },
                format_timestamp(Some(*at))
            ),
            None => "等待首次拉取".into(),
        }
    }

    pub fn saved_output_status(&self, cx: &Context<Self>) -> String {
        if self.saved_config_error.is_some() {
            return "保存失败 · 客户端继续使用已保存配置".into();
        }
        if self.saved_config_dirty(cx) || self.saved_visual_dirty(cx) || self.saved_editor.pending {
            return "未保存修改 · 客户端使用已保存配置".into();
        }
        if self.server.is_none() {
            return "已保存 · 本地订阅服务未启动".into();
        }
        let updated = self
            .saved_config_id
            .as_ref()
            .and_then(|id| self.saved_configs.iter().find(|config| &config.id == id))
            .map(|config| format_timestamp(Some(config.updated_at)))
            .unwrap_or_default();
        let pull = self
            .saved_config_id
            .as_deref()
            .map(|token| {
                let yaml = self
                    .saved_configs
                    .iter()
                    .find(|config| config.id == token)
                    .map(|config| config.yaml.as_str())
                    .unwrap_or_default();
                self.profile_pull_status(token, yaml)
            })
            .unwrap_or_default();
        format!("已保存 {updated} · {pull}")
    }
}
