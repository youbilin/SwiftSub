use super::*;

fn validate_yaml(yaml: &str) -> Result<(), String> {
    let value: serde_yaml_ng::Value =
        serde_yaml_ng::from_str(yaml).map_err(|e| format!("YAML 格式错误：{e}"))?;
    let map = value.as_mapping().ok_or("配置必须是 YAML 对象")?;
    if !map.contains_key("proxies") && !map.contains_key("proxy-providers") {
        return Err("配置需要 proxies 或 proxy-providers".into());
    }
    for key in ["proxies", "proxy-groups", "rules", "listeners"] {
        if map.get(key).is_some_and(|v| !v.is_sequence()) {
            return Err(format!("{key} 必须是列表"));
        }
    }
    for key in ["proxy-providers", "rule-providers", "dns"] {
        if map.get(key).is_some_and(|v| !v.is_mapping()) {
            return Err(format!("{key} 必须是对象"));
        }
    }
    Ok(())
}
impl SwiftSubApp {
    pub(super) fn reload_saved_configs(&mut self) {
        let result = (|| {
            let store = self.store.lock().unwrap();
            let directory = self.data_dir.join("configs");
            // Preserve each existing profile's latest output once when upgrading.
            for profile in store.list_profiles()? {
                let key = format!("saved-output-migrated:{}", profile.id);
                if store.get_setting(&key)?.is_none() {
                    if let Some(yaml) = profile.last_yaml.filter(|yaml| !yaml.trim().is_empty()) {
                        let existing = store
                            .list_saved_configs()?
                            .into_iter()
                            .any(|c| c.source_json == profile.options_json && c.yaml == yaml);
                        if !existing {
                            store.save_yaml_config(
                                &directory,
                                None,
                                &profile.name,
                                &yaml,
                                profile.options_json.as_deref(),
                            )?;
                        }
                    }
                    store.set_setting(&key, "true")?;
                }
            }
            store.repair_saved_config_files(&directory)?;
            store.list_saved_configs()
        })();
        match result {
            Ok(configs) => {
                self.saved_configs = configs;
                self.publish_profiles();
            }
            Err(e) => self.saved_config_error = Some(format!("配置库读取失败：{e}")),
        }
    }
    pub fn copy_saved_config_url(&mut self, cx: &mut Context<Self>) {
        if let (Some(server), Some(id)) = (&self.server, &self.saved_config_id) {
            cx.write_to_clipboard(ClipboardItem::new_string(
                server.profile_url(id, "config.yaml"),
            ));
            self.show_toast("已复制此配置的独立订阅地址".into(), cx);
        }
    }
    pub fn saved_config_dirty(&self, cx: &Context<Self>) -> bool {
        self.saved_config_id
            .as_ref()
            .and_then(|id| self.saved_configs.iter().find(|c| &c.id == id))
            .is_some_and(|config| {
                config.name != self.saved_config_name.read(cx).value().trim()
                    || config.yaml != self.saved_config_yaml.read(cx).value().as_ref()
            })
    }
    pub fn open_saved_config(&mut self, id: String, window: &mut Window, cx: &mut Context<Self>) {
        // Keep unfinished text when moving between entries; switching never silently loses edits.
        if let Err(e) = self.checkpoint_yaml_editor(cx) {
            self.saved_config_error = Some(e);
            return;
        }
        let Some(config) = self.saved_configs.iter().find(|c| c.id == id).cloned() else {
            return;
        };
        self.saved_config_id = Some(id.clone());
        self.saved_config_delete_armed = false;
        self.saved_config_error = None;
        let draft = self
            .store
            .lock()
            .unwrap()
            .get_setting(&format!("yaml-editor:{id}"))
            .ok()
            .flatten()
            .and_then(|v| serde_json::from_str::<(String, String)>(&v).ok());
        let (name, yaml) = draft.unwrap_or((config.name, config.yaml));
        self.saved_config_name
            .update(cx, |s, cx| s.set_value(name, window, cx));
        self.saved_config_yaml
            .update(cx, |s, cx| s.set_value(yaml, window, cx));
        self.page = Page::SavedConfigs;
        cx.notify();
    }
    pub(super) fn checkpoint_yaml_editor(&self, cx: &Context<Self>) -> Result<(), String> {
        if let Some(id) = &self.saved_config_id {
            let key = format!("yaml-editor:{id}");
            let text = if self.saved_config_dirty(cx) {
                serde_json::to_string(&(
                    self.saved_config_name.read(cx).value().to_string(),
                    self.saved_config_yaml.read(cx).value().to_string(),
                ))
                .map_err(|e| e.to_string())?
            } else {
                String::new()
            };
            let store = self.store.lock().unwrap();
            if store
                .get_setting(&key)
                .map_err(|e| e.to_string())?
                .as_deref()
                != Some(text.as_str())
            {
                store.set_setting(&key, &text).map_err(|e| e.to_string())?;
            }
        }
        Ok(())
    }
    pub fn save_generated_config(
        &mut self,
        update: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Err(e) = self.autosave_routing(window, cx) {
            self.show_toast(format!("请先修正分流配置：{e}"), cx);
            return;
        }
        if let Err(e) = validate_yaml(&self.preview) {
            self.show_toast(e, cx);
            return;
        }
        let existing = if update {
            self.editing_saved_config
                .as_ref()
                .and_then(|id| self.saved_configs.iter().find(|c| &c.id == id))
                .cloned()
        } else {
            None
        };
        if update && existing.is_none() {
            self.show_toast("原配置已删除，请保存为新配置".into(), cx);
            return;
        }
        let name = existing
            .as_ref()
            .map(|c| c.name.clone())
            .unwrap_or_else(|| format!("我的配置 {}", Local::now().format("%Y-%m-%d %H:%M:%S")));
        let result = self
            .profile_snapshot_json(self.active_template_snapshot())
            .and_then(|source| {
                self.store
                    .lock()
                    .unwrap()
                    .save_yaml_config(
                        &self.data_dir.join("configs"),
                        existing.as_ref().map(|c| c.id.as_str()),
                        &name,
                        &self.preview,
                        Some(&source),
                    )
                    .map_err(|e| e.to_string())
            });
        match result {
            Ok(config) => {
                self.editing_saved_config = None;
                if self.saved_config_id.as_ref() == Some(&config.id) {
                    self.saved_config_id = None;
                }
                let _ = self
                    .store
                    .lock()
                    .unwrap()
                    .set_setting(&format!("yaml-editor:{}", config.id), "");
                self.reload_saved_configs();
                self.open_saved_config(config.id, window, cx);
                self.request_close_template(window, cx);
                self.show_toast(
                    if update {
                        "已更新这份配置"
                    } else {
                        "已保存新的独立 YAML 配置"
                    }
                    .into(),
                    cx,
                );
            }
            Err(e) => self.show_toast(format!("保存失败：{e}"), cx),
        }
    }
    pub fn save_yaml_editor(&mut self, copy: bool, window: &mut Window, cx: &mut Context<Self>) {
        let name = self.saved_config_name.read(cx).value().trim().to_string();
        let yaml = self.saved_config_yaml.read(cx).value().to_string();
        let Some(config) = self
            .saved_config_id
            .as_ref()
            .and_then(|id| self.saved_configs.iter().find(|c| &c.id == id))
            .cloned()
        else {
            return;
        };
        let result = validate_yaml(&yaml).and_then(|_| {
            self.store
                .lock()
                .unwrap()
                .save_yaml_config(
                    &self.data_dir.join("configs"),
                    if copy { None } else { Some(&config.id) },
                    &name,
                    &yaml,
                    config.source_json.as_deref(),
                )
                .map_err(|e| e.to_string())
        });
        match result {
            Ok(saved) => {
                self.saved_config_error = None;
                // The old editor text has now been explicitly saved, including when creating a copy.
                let _ = self
                    .store
                    .lock()
                    .unwrap()
                    .set_setting(&format!("yaml-editor:{}", config.id), "");
                self.saved_config_id = None;
                self.reload_saved_configs();
                self.open_saved_config(saved.id, window, cx);
                self.show_toast(
                    if copy {
                        "已另存一份新配置"
                    } else {
                        "配置和独立 YAML 文件已更新"
                    }
                    .into(),
                    cx,
                );
            }
            Err(e) => {
                self.saved_config_error = Some(e);
                cx.notify();
            }
        }
    }
    pub fn load_saved_generator(
        &mut self,
        id: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(config) = self.saved_configs.iter().find(|c| c.id == id).cloned() else {
            return;
        };
        let result = config
            .source_json
            .as_deref()
            .ok_or("这份配置没有原生成设置，可直接编辑 YAML".to_string())
            .and_then(|s| {
                serde_json::from_str::<super::profiles::ProfileConfig>(s).map_err(|e| e.to_string())
            });
        match result {
            Ok(source) => {
                if let Err(e) = self.checkpoint_yaml_editor(cx) {
                    self.saved_config_error = Some(e);
                    return;
                }
                if self.template_session_active {
                    self.request_close_template(window, cx);
                }
                if self.template_session_active {
                    self.show_toast("请先修正未完成的分流配置，编辑内容已保留".into(), cx);
                    return;
                }
                self.profile_membership = source.membership;
                self.load_template_draft(source.routing, config.name, None, window, cx);
                self.editing_saved_config = Some(id);
                self.refresh_profile_nodes(cx);
                if let Err(e) = self.autosave_routing(window, cx) {
                    self.template_error = Some(e);
                }
                self.show_toast(
                    "已载入原生成设置，修改后可更新原配置或保存为新配置".into(),
                    cx,
                );
            }
            Err(e) => self.show_toast(e, cx),
        }
    }
    pub fn export_saved_yaml(&mut self, cx: &mut Context<Self>) {
        let yaml = self.saved_config_yaml.read(cx).value().to_string();
        if let Err(e) = validate_yaml(&yaml) {
            self.saved_config_error = Some(e);
            cx.notify();
            return;
        }
        let name = self.saved_config_name.read(cx).value().to_string();
        let Some(path) = rfd::FileDialog::new()
            .set_file_name(format!(
                "{}.yaml",
                name.replace(['/', '\\', ':', '*', '?', '"', '<', '>', '|'], "_")
            ))
            .add_filter("YAML", &["yaml", "yml"])
            .save_file()
        else {
            return;
        };
        match std::fs::write(path, yaml) {
            Ok(()) => self.show_toast("YAML 已导出".into(), cx),
            Err(e) => self.show_toast(e.to_string(), cx),
        }
    }
    pub fn delete_yaml_config(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(id) = self.saved_config_id.clone() else {
            return;
        };
        if !self.saved_config_delete_armed {
            self.saved_config_delete_armed = true;
            cx.notify();
            return;
        }
        let result = self
            .store
            .lock()
            .unwrap()
            .delete_saved_config(&self.data_dir.join("configs"), &id);
        match result {
            Ok(()) => {
                let _ = self
                    .store
                    .lock()
                    .unwrap()
                    .set_setting(&format!("yaml-editor:{id}"), "");
                self.saved_config_id = None;
                self.saved_config_delete_armed = false;
                if self.editing_saved_config.as_ref() == Some(&id) {
                    self.editing_saved_config = None;
                }
                self.reload_saved_configs();
                if let Some(next) = self.saved_configs.first() {
                    self.open_saved_config(next.id.clone(), window, cx);
                }
                cx.notify();
            }
            Err(e) => {
                self.saved_config_error = Some(e.to_string());
                cx.notify();
            }
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn validates_yaml_before_saving_without_reformatting_it() {
        for text in [
            "",
            "a: [",
            "- a",
            "proxies: wrong",
            "proxy-providers: []",
            "rules: []",
        ] {
            assert!(validate_yaml(text).is_err(), "{text}");
        }
        for text in [
            "# keep comment\nproxies: []\nrules: [MATCH,DIRECT]",
            "proxy-providers: {}\nproxy-groups: []",
        ] {
            assert!(validate_yaml(text).is_ok());
        }
    }
}
