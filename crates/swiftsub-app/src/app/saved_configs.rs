use super::*;
use gpui_kit::Focusable;
mod files;

#[derive(Serialize, Deserialize)]
struct GeneratedAssociation {
    id: String,
    name: String,
    yaml: String,
    source_json: Option<String>,
}

pub(super) fn association_json(config: &swiftsub_store::SavedConfig) -> Result<String, String> {
    serde_json::to_string(&GeneratedAssociation {
        id: config.id.clone(),
        name: config.name.clone(),
        yaml: config.yaml.clone(),
        source_json: config.source_json.clone(),
    })
    .map_err(|e| e.to_string())
}

fn restore_association(
    json: &str,
    configs: &[swiftsub_store::SavedConfig],
) -> Result<swiftsub_store::SavedConfig, String> {
    let baseline: GeneratedAssociation =
        serde_json::from_str(json).map_err(|e| format!("保存目标读取失败：{e}"))?;
    let mut config = configs
        .iter()
        .find(|config| config.id == baseline.id)
        .cloned()
        .ok_or("上次保存的配置已删除，请另存为新配置")?;
    // Restore the historical baseline, so later manual edits still trigger conflict protection.
    config.name = baseline.name;
    config.yaml = baseline.yaml;
    config.source_json = baseline.source_json;
    Ok(config)
}

fn clash_import_uri(subscription_url: &str, name: &str) -> String {
    let mut uri = url::Url::parse("clash://install-config").expect("static URI");
    // Verge reads all remaining text after url= as the nested URL. Keep url last.
    uri.query_pairs_mut()
        .append_pair("name", name)
        .append_pair("url", subscription_url);
    uri.into()
}

pub(super) fn validate_yaml(yaml: &str) -> Result<(), String> {
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
    crate::saved_document::SavedDocument::parse(yaml, None)?.validate()
}

pub(super) fn generate_yaml_output(
    nodes: &[Node],
    options: &GenerateOptions,
) -> Result<String, String> {
    if !nodes.iter().any(|node| node.enabled) {
        return Err("请先添加并启用节点".into());
    }
    let yaml = generate_profile(nodes, options)
        .map_err(|error| error.to_string())?
        .yaml;
    validate_yaml(&yaml)?;
    Ok(yaml)
}

pub(super) fn save_generated_yaml(
    store: &Store,
    directory: &std::path::Path,
    previous: Option<&swiftsub_store::SavedConfig>,
    name: &str,
    yaml: &str,
    source: &str,
) -> Result<swiftsub_store::SavedConfig, String> {
    let name = name.trim();
    if name.is_empty() {
        return Err("请输入配置名称".into());
    }
    validate_yaml(yaml)?;
    if let Some(previous) = previous {
        let current = store
            .list_saved_configs()
            .map_err(|error| error.to_string())?
            .into_iter()
            .find(|config| config.id == previous.id)
            .ok_or("原配置已删除，请存为新配置")?;
        if current.yaml != previous.yaml
            || current.name != previous.name
            || current.source_json != previous.source_json
        {
            return Err("这份配置已在「我的配置」中修改，请另存为新配置，避免覆盖修改".into());
        }
        for prefix in ["yaml-editor", "visual-form", "visual-meta"] {
            if store
                .get_setting(&format!("{prefix}:{}", current.id))
                .map_err(|error| error.to_string())?
                .is_some_and(|draft| !draft.trim().is_empty())
            {
                return Err(
                    "这份配置还有未完成的编辑，请先在「我的配置」中处理，或另存为新配置".into(),
                );
            }
        }
        if std::fs::read_to_string(current.path(directory)).is_ok_and(|text| text != current.yaml) {
            return Err("配置文件已被外部修改，请另存为新配置，避免覆盖修改".into());
        }
        // Repeated save/import keeps one subscription and avoids redundant history entries.
        if current.name == name
            && current.yaml == yaml
            && current.source_json.as_deref() == Some(source)
            && std::fs::read_to_string(current.path(directory)).is_ok_and(|text| text == yaml)
        {
            return Ok(current);
        }
    }
    store
        .save_yaml_config(
            directory,
            previous.map(|config| config.id.as_str()),
            name,
            yaml,
            Some(source),
        )
        .map_err(|error| error.to_string())
}

impl SwiftSubApp {
    pub fn config_subscription_url(&self, id: &str) -> Option<String> {
        self.server
            .as_ref()
            .map(|server| server.profile_url(id, &format!("{id}.yaml")))
    }

    pub fn config_lan_subscription_url(&self, id: &str) -> Option<String> {
        self.server.as_ref().and_then(|server| {
            server.lan_profile_url_with_host(
                self.server_lan_host.as_deref(),
                id,
                &format!("{id}.yaml"),
            )
        })
    }

    pub fn copy_config_subscription(&mut self, id: String, cx: &mut Context<Self>) {
        if !self.saved_configs.iter().any(|config| config.id == id) {
            return;
        }
        if self.server.is_none() {
            self.start_server();
        }
        self.publish_profiles();
        match self.config_subscription_url(&id) {
            Some(url) => {
                cx.write_to_clipboard(ClipboardItem::new_string(url));
                self.show_toast("已复制此配置的本机订阅地址".into(), cx);
            }
            None => self.show_toast(
                self.server_error
                    .clone()
                    .unwrap_or("本地订阅服务未启动".into()),
                cx,
            ),
        }
    }

    pub fn copy_config_lan_subscription(&mut self, id: String, cx: &mut Context<Self>) {
        if !self.saved_configs.iter().any(|config| config.id == id) {
            return;
        }
        if !self.server_bind_lan {
            self.show_toast("请先在设置中开启「允许局域网导入」".into(), cx);
            return;
        }
        if self.server.is_none() {
            self.start_server();
        }
        self.publish_profiles();
        match self.config_lan_subscription_url(&id) {
            Some(url) => {
                cx.write_to_clipboard(ClipboardItem::new_string(url.clone()));
                self.show_toast(format!("已复制局域网订阅地址：{url}"), cx);
            }
            None => self.show_toast(
                self.server_error
                    .clone()
                    .unwrap_or("未检测到可用局域网 IPv4 地址，请检查网络连接".into()),
                cx,
            ),
        }
    }

    pub(super) fn restore_generated_target(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.restore_rule_visibility();
        let result = (|| {
            let store = self.store.lock().map_err(|e| e.to_string())?;
            let json = store
                .get_setting(&format!("generated-target:{}", self.profile_id))
                .map_err(|e| e.to_string())?;
            match json.filter(|json| !json.is_empty()) {
                Some(json) => restore_association(
                    &json,
                    &store.list_saved_configs().map_err(|e| e.to_string())?,
                )
                .map(Some),
                None => Ok(None),
            }
        })();
        match result {
            Ok(target) => self.generated_saved_config = target,
            Err(error) => self.generated_save_error = Some(error),
        }
        self.reset_generated_name(self.generated_saved_config.is_some(), window, cx);
        self.workspace.selected_template_id = self
            .store
            .lock()
            .ok()
            .and_then(|store| {
                store
                    .get_setting(&format!("selected-template:{}", self.profile_id))
                    .ok()
                    .flatten()
            })
            .and_then(|value| value.parse::<i64>().ok())
            .filter(|id| {
                self.custom_templates
                    .iter()
                    .any(|template| template.id == *id)
            });
        let name_draft = self
            .store
            .lock()
            .ok()
            .and_then(|store| {
                store
                    .get_setting(&format!("generated-name:{}", self.profile_id))
                    .ok()
                    .flatten()
            })
            .and_then(|json| serde_json::from_str::<(String, bool, bool)>(&json).ok());
        if let Some((name, open, update)) = name_draft.filter(|(_, open, _)| *open) {
            self.generated_config_name
                .update(cx, |input, cx| input.set_value(name, window, cx));
            self.generated_save_open = open;
            self.generated_save_update = update && self.generated_saved_config.is_some();
        }
    }

    pub fn cancel_generated_name(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.generated_save_open = false;
        self.reset_generated_name(self.generated_saved_config.is_some(), window, cx);
        cx.notify();
    }

    pub(super) fn checkpoint_generated_name(&self, cx: &Context<Self>) -> Result<(), String> {
        let json = serde_json::to_string(&(
            self.generated_config_name.read(cx).value().to_string(),
            self.generated_save_open,
            self.generated_save_update,
        ))
        .map_err(|e| e.to_string())?;
        let store = self.store.lock().map_err(|e| e.to_string())?;
        let key = format!("generated-name:{}", self.profile_id);
        if store
            .get_setting(&key)
            .map_err(|e| e.to_string())?
            .as_deref()
            != Some(&json)
        {
            store.set_setting(&key, &json).map_err(|e| e.to_string())?;
        }
        let selection = self
            .workspace
            .selected_template_id
            .map(|id| id.to_string())
            .unwrap_or_default();
        let key = format!("selected-template:{}", self.profile_id);
        if store
            .get_setting(&key)
            .map_err(|e| e.to_string())?
            .as_deref()
            != Some(&selection)
        {
            store
                .set_setting(&key, &selection)
                .map_err(|e| e.to_string())?;
        }
        Ok(())
    }
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
                self.refresh_config_summaries();
                self.publish_profiles();
            }
            Err(e) => self.saved_config_error = Some(format!("配置库读取失败：{e}")),
        }
    }
    pub(super) fn refresh_config_summaries(&mut self) {
        self.workspace.config_summaries = self
            .saved_configs
            .iter()
            .map(|config| {
                (
                    config.id.clone(),
                    workspace::ConfigSummary::from_config(config),
                )
            })
            .collect();
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
        if self.saved_config_id.as_deref() == Some(&id) {
            self.page = Page::SavedConfigs;
            cx.notify();
            return;
        }
        if let Err(e) = self.autosave_saved_editor(window, cx) {
            self.saved_config_error = Some(e);
        }
        // Keep unfinished text when moving between entries; switching never silently loses edits.
        if let Err(e) = self.checkpoint_yaml_editor(cx) {
            self.saved_config_error = Some(e);
            return;
        }
        let Some(config) = self.saved_configs.iter().find(|c| c.id == id).cloned() else {
            return;
        };
        self.saved_config_id = Some(id.clone());
        self.saved_file_baseline = None;
        self.saved_external_change = false;
        self.saved_draft_corrupt = false;
        self.saved_config_delete_armed = false;
        self.saved_config_error = None;
        let draft = match files::read_yaml_draft(&self.store.lock().unwrap(), &id) {
            Ok(draft) => draft,
            Err(error) => {
                self.saved_draft_corrupt = true;
                self.saved_config_error = Some(error);
                None
            }
        };
        let (name, yaml) = if let Some(draft) = draft {
            self.saved_file_baseline = draft.file_baseline;
            (draft.name, draft.yaml)
        } else {
            (config.name, config.yaml)
        };
        self.saved_config_name
            .update(cx, |s, cx| s.set_value(name, window, cx));
        self.saved_config_yaml
            .update(cx, |s, cx| s.set_value(yaml, window, cx));
        self.load_saved_visual(window, cx);
        if let Err(error) = self.check_saved_file_state() {
            self.saved_config_error = Some(error);
        }
        self.page = Page::SavedConfigs;
        cx.notify();
    }
    pub(super) fn check_yaml_draft_storage(&self) -> Result<(), String> {
        if self.saved_draft_corrupt {
            return Err("编辑草稿读取失败，原数据已保留，请恢复保存的配置后继续".into());
        }
        if let Some(id) = &self.saved_config_id {
            let store = self.store.lock().map_err(|error| error.to_string())?;
            files::read_yaml_draft(&store, id)?;
        }
        Ok(())
    }
    pub(super) fn checkpoint_yaml_editor(&self, cx: &Context<Self>) -> Result<(), String> {
        self.check_yaml_draft_storage()?;
        self.checkpoint_saved_form(cx)?;
        if let Some(id) = &self.saved_config_id {
            let key = format!("yaml-editor:{id}");
            let text = if self.saved_config_dirty(cx) || self.saved_file_baseline.is_some() {
                serde_json::to_string(&files::SavedYamlDraft {
                    name: self.saved_config_name.read(cx).value().to_string(),
                    yaml: self.saved_config_yaml.read(cx).value().to_string(),
                    file_baseline: self.saved_file_baseline.clone(),
                })
                .map_err(|e| e.to_string())?
            } else {
                String::new()
            };
            let store = self.store.lock().unwrap();
            files::read_yaml_draft(&store, id)?;
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
    pub fn begin_save_generated(
        &mut self,
        update: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.generated_save_open && self.generated_save_update == update {
            self.generated_config_name
                .read(cx)
                .focus_handle(cx)
                .focus(window, cx);
            return;
        }
        self.reset_generated_name(update, window, cx);
        self.generated_save_update = update;
        self.generated_save_error = None;
        self.generated_save_open = true;
        self.generated_config_name
            .read(cx)
            .focus_handle(cx)
            .focus(window, cx);
        cx.notify();
    }

    pub(super) fn reset_generated_name(
        &mut self,
        update: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let name = if update {
            self.generated_saved_config.as_ref().map(|c| c.name.clone())
        } else {
            None
        }
        .unwrap_or_else(|| format!("配置 {}", Local::now().format("%m-%d %H:%M")));
        self.generated_config_name
            .update(cx, |input, cx| input.set_value(name, window, cx));
    }

    pub fn request_save_generated(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.save_generation_draft(window, cx);
    }

    fn persist_generated_config(
        &mut self,
        update: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Result<swiftsub_store::SavedConfig, String> {
        let yaml = self.prepare_yaml_output(window, cx)?;
        let previous = if update {
            Some(
                self.generated_saved_config
                    .clone()
                    .ok_or("原配置已删除，请另存为新配置")?,
            )
        } else {
            None
        };
        if previous
            .as_ref()
            .is_some_and(|config| self.saved_config_id.as_ref() == Some(&config.id))
        {
            self.checkpoint_yaml_editor(cx)?;
        }
        let name = self.generated_config_name.read(cx).value().to_string();
        let source = self.profile_snapshot_json(self.active_template_snapshot())?;
        let config = save_generated_yaml(
            &*self.store.lock().map_err(|error| error.to_string())?,
            &self.data_dir.join("configs"),
            previous.as_ref(),
            &name,
            &yaml,
            &source,
        )?;
        self.generated_saved_config = Some(config.clone());
        let association_result = self
            .store
            .lock()
            .map_err(|e| e.to_string())?
            .set_setting(
                &format!("generated-target:{}", self.profile_id),
                &association_json(&config)?,
            )
            .map_err(|e| e.to_string());
        self.generated_save_open = false;
        self.generated_save_error = None;
        // A clean saved editor will reload this output on its next visit.
        if self.saved_config_id.as_ref() == Some(&config.id) {
            self.saved_config_id = None;
        }
        self.saved_configs
            .retain(|existing| existing.id != config.id);
        self.saved_configs.insert(0, config.clone());
        self.refresh_config_summaries();
        self.checkpoint_config_generation(&config, cx)?;
        self.publish_profiles();
        self.checkpoint_generated_name(cx)?;
        cx.notify();
        association_result.map_err(|e| format!("配置已保存，但保存目标未能记住：{e}"))?;
        Ok(config)
    }
    pub fn save_generated_config(
        &mut self,
        update: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        match self.persist_generated_config(update, window, cx) {
            Ok(_) => {
                self.show_toast(
                    if update {
                        "已保存修改到我的配置"
                    } else {
                        "已保存到我的配置"
                    }
                    .into(),
                    cx,
                );
                true
            }
            Err(error) => {
                self.generated_save_error = Some(error.clone());
                self.show_toast(format!("保存失败：{error}"), cx);
                false
            }
        }
    }

    pub fn import_generated_to_clash(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.workspace.saved_editor_open {
            self.import_saved_to_clash(window, cx);
            return;
        }
        if self.clash_import_busy {
            return;
        }
        let update = if self.generated_save_open {
            self.generated_save_update
        } else {
            let update = self.generated_saved_config.is_some();
            self.reset_generated_name(update, window, cx);
            update
        };
        match self.persist_generated_config(update, window, cx) {
            Ok(config) => self.begin_clash_import(config, true, window, cx),
            Err(error) => {
                self.generated_save_error = Some(error.clone());
                self.show_toast(format!("无法导入：{error}"), cx);
            }
        }
    }

    fn prepare_clash_import(
        &mut self,
        config: &swiftsub_store::SavedConfig,
    ) -> Result<String, String> {
        if self.server.is_none() {
            self.start_server();
        }
        let server = self.server.as_ref().ok_or_else(|| {
            self.server_error
                .clone()
                .unwrap_or_else(|| "本地订阅服务未启动".into())
        })?;
        // publish_profiles makes persisted records available before the client receives the URI.
        if !server.published_matches(&config.id, &config.yaml) {
            return Err("此配置尚未发布，请重试导入".into());
        }
        Ok(clash_import_uri(
            &server.profile_url(&config.id, &format!("{}.yaml", config.id)),
            &config.name,
        ))
    }

    pub(super) fn begin_clash_import(
        &mut self,
        config: swiftsub_store::SavedConfig,
        generated: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let profile_id = self.profile_id;
        let uri = match self.prepare_clash_import(&config) {
            Ok(uri) => uri,
            Err(error) => {
                let message = format!("配置已保存，导入未完成：{error}");
                if generated {
                    self.generated_save_error = Some(message.clone());
                } else {
                    self.saved_config_error = Some(message.clone());
                }
                self.show_toast(message, cx);
                cx.notify();
                return;
            }
        };
        let observer = self.server.as_ref().unwrap().publisher();
        let previous_pull = observer
            .profile_pull(&config.id)
            .map_or(0, |pull| pull.requests);
        let expected_etag = ProfileSnapshot {
            yaml: config.yaml.clone(),
            ..Default::default()
        }
        .etag();
        let subscription_url = self.config_subscription_url(&config.id).unwrap_or_default();
        self.clash_import_busy = true;
        cx.notify();
        cx.spawn_in(window, async move |this, cx| {
            let token = config.id.clone();
            let result = cx
                .background_executor()
                .spawn(async move {
                    crate::desktop::open_clash_verge(&uri)?;
                    for _ in 0..60 {
                        if observer.profile_pull(&token).is_some_and(|pull| pull.requests > previous_pull && pull.etag == expected_etag) {
                            return Ok::<_, String>(true);
                        }
                        std::thread::sleep(Duration::from_millis(250));
                    }
                    Ok(false)
                })
                .await;
            let _ = cx.update(|_, cx| {
                this.update(cx, |this, cx| {
                    this.clash_import_busy = false;
                    match result {
                        Ok(true) => {
                            this.generated_save_error = None;
                            this.saved_config_error = None;
                            this.show_toast(format!("Clash Verge 已读取「{}」的本地订阅", config.name), cx)
                        }
                        Ok(false) => {
                            let message = format!("已打开 Clash Verge，但客户端尚未读取「{}」。可在客户端粘贴订阅地址：{subscription_url}", config.name);
                            if generated && this.generated_saved_config.as_ref().is_some_and(|current| current.id == config.id) {
                                this.generated_save_error = Some(message.clone());
                            } else if !generated && this.saved_config_id.as_ref() == Some(&config.id) {
                                this.saved_config_error = Some(message.clone());
                            }
                            this.show_toast(message, cx);
                        }
                        Err(error) => {
                            let message = format!("「{}」已保存，导入未完成：{error}", config.name);
                            if generated && this.profile_id == profile_id {
                                this.generated_save_error = Some(message.clone());
                            } else if !generated
                                && this.saved_config_id.as_deref() == Some(&config.id)
                            {
                                this.saved_config_error = Some(message.clone());
                            }
                            this.show_toast(message, cx);
                        }
                    }
                    cx.notify();
                })
            });
        })
        .detach();
    }
    pub fn request_saved_copy(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Err(error) = self.flush_saved_fields(window, cx) {
            self.saved_config_error = Some(error);
            cx.notify();
            return;
        }
        let base = format!("{} 副本", self.saved_config_name.read(cx).value().trim());
        let mut name = base.clone();
        let mut suffix = 2;
        while self.saved_configs.iter().any(|config| config.name == name) {
            name = format!("{base} {suffix}");
            suffix += 1;
        }
        self.saved_editor
            .copy_name
            .update(cx, |input, cx| input.set_value(name, window, cx));
        self.saved_editor.copy_open = true;
        self.saved_config_error = None;
        cx.notify();
    }

    pub fn save_yaml_editor(
        &mut self,
        copy: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let preparation = if copy {
            self.flush_saved_fields(window, cx)
                .and_then(|_| self.checkpoint_saved_form(cx))
                .and_then(|_| self.checkpoint_yaml_editor(cx))
        } else {
            self.autosave_saved_editor(window, cx)
        };
        if let Err(e) = preparation {
            self.saved_config_error = Some(e);
            cx.notify();
            return false;
        }
        let name = if copy && self.saved_editor.copy_open {
            self.saved_editor
                .copy_name
                .read(cx)
                .value()
                .trim()
                .to_string()
        } else {
            self.saved_config_name.read(cx).value().trim().to_string()
        };
        let yaml = self.saved_config_yaml.read(cx).value().to_string();
        let Some(config) = self
            .saved_config_id
            .as_ref()
            .and_then(|id| self.saved_configs.iter().find(|c| &c.id == id))
            .cloned()
        else {
            return false;
        };
        if !copy {
            if let Err(error) = validate_yaml(&yaml) {
                self.saved_config_error = Some(error);
                cx.notify();
                return false;
            }
            self.show_toast("配置和独立 YAML 文件已保存".into(), cx);
            return true;
        }
        let result = validate_yaml(&yaml).and_then(|_| {
            let source = super::saved_editor::saved_edit_source(
                &config,
                &yaml,
                self.saved_editor.document.as_ref(),
                self.saved_editor.tab == SavedEditorTab::Yaml,
                self.saved_editor.pending,
            )?;
            self.store
                .lock()
                .unwrap()
                .save_yaml_config(
                    &self.data_dir.join("configs"),
                    None,
                    &name,
                    &yaml,
                    source.as_deref(),
                )
                .map_err(|e| e.to_string())
        });
        match result {
            Ok(saved) => {
                self.saved_config_error = None;
                // Keep the original editor draft intact when opening the new independent copy.
                self.saved_config_id = None;
                self.reload_saved_configs();
                self.open_saved_config(saved.id, window, cx);
                if self.workspace.saved_editor_open {
                    self.page = Page::Generator;
                }
                self.show_toast("已另存一份新配置".into(), cx);
                true
            }
            Err(e) => {
                self.saved_config_error = Some(e);
                cx.notify();
                false
            }
        }
    }
    pub fn import_saved_to_clash(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.clash_import_busy {
            return;
        }
        if let Err(e) = self.autosave_saved_editor(window, cx) {
            self.saved_config_error = Some(e);
            cx.notify();
            return;
        }
        let Some(config) = self
            .saved_config_id
            .as_ref()
            .and_then(|id| self.saved_configs.iter().find(|c| &c.id == id))
            .cloned()
        else {
            return;
        };
        self.publish_profiles();
        self.begin_clash_import(config, false, window, cx);
    }
    pub fn import_yaml_config(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(path) = rfd::FileDialog::new()
            .add_filter("YAML", &["yaml", "yml"])
            .pick_file()
        else {
            return;
        };
        let name = path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("导入配置");
        let result = std::fs::read_to_string(&path)
            .map_err(|e| e.to_string())
            .and_then(|yaml| {
                validate_yaml(&yaml)?;
                self.store
                    .lock()
                    .unwrap()
                    .save_yaml_config(&self.data_dir.join("configs"), None, name, &yaml, None)
                    .map_err(|e| e.to_string())
            });
        match result {
            Ok(config) => {
                self.reload_saved_configs();
                self.edit_library_config(config.id, window, cx);
                self.show_toast("已导入 YAML".into(), cx);
            }
            Err(e) => self.show_toast(format!("导入失败：{e}"), cx),
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
    pub fn delete_library_config(&mut self, id: String, cx: &mut Context<Self>) {
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
                if self.saved_config_id.as_ref() == Some(&id) {
                    self.saved_config_id = None;
                    self.workspace.saved_editor_open = false;
                    self.page = Page::SavedConfigs;
                }
                self.saved_config_delete_armed = false;
                if self
                    .generated_saved_config
                    .as_ref()
                    .is_some_and(|config| config.id == id)
                {
                    self.generated_saved_config = None;
                    self.generated_save_open = false;
                    self.generated_save_error = None;
                }
                self.reload_saved_configs();
                self.publish_profiles();
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

    struct TestDirectory(std::path::PathBuf);

    impl TestDirectory {
        fn new() -> Self {
            Self(std::env::temp_dir().join(format!(
                "swiftsub-generated-save-{}-{}",
                std::process::id(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_nanos()
            )))
        }
    }

    impl Drop for TestDirectory {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn generated_save_uses_current_nodes_and_keeps_one_record_on_repeat_and_update() {
        let directory = TestDirectory::new();
        let store = Store::open_in_memory().unwrap();
        let mut nodes = build_nodes(
            (0..100)
                .map(|index| {
                    Proxy::new(
                        "trojan",
                        format!("Node-{index}"),
                        format!("node-{index}.example.org"),
                        443,
                    )
                })
                .collect(),
            "test",
            "test",
        );
        for node in &mut nodes {
            node.proxy.fields_mut().insert(
                "password".into(),
                serde_yaml_ng::Value::String("test-password".into()),
            );
        }
        nodes[0].enabled = false;
        let options = GenerateOptions {
            preset: Preset::Minimal,
            ..Default::default()
        };
        let yaml = generate_yaml_output(&nodes, &options).unwrap();
        let source = r#"{"routing":{"preset":"minimal"}}"#;
        let first =
            save_generated_yaml(&store, &directory.0, None, "  日常配置  ", &yaml, source).unwrap();
        let parsed: serde_yaml_ng::Value = serde_yaml_ng::from_str(&first.yaml).unwrap();
        assert_eq!(parsed["proxies"].as_sequence().unwrap().len(), 99);
        assert_eq!(first.name, "日常配置");
        let repeated = save_generated_yaml(
            &store,
            &directory.0,
            Some(&first),
            &first.name,
            &yaml,
            source,
        )
        .unwrap();
        assert_eq!(repeated.id, first.id);
        assert_eq!(store.config_history(&first.id).unwrap().len(), 1);
        nodes[1].set_name("Updated node");
        let changed_yaml = generate_yaml_output(&nodes, &options).unwrap();
        let updated = save_generated_yaml(
            &store,
            &directory.0,
            Some(&repeated),
            &first.name,
            &changed_yaml,
            source,
        )
        .unwrap();
        assert_eq!(updated.id, first.id);
        assert_ne!(updated.yaml, first.yaml);
        assert_eq!(updated.yaml, changed_yaml);
        assert_eq!(store.list_saved_configs().unwrap().len(), 1);
        assert_eq!(
            std::fs::read_to_string(updated.path(&directory.0)).unwrap(),
            changed_yaml
        );
    }

    #[test]
    fn generated_output_rejects_no_enabled_nodes_and_invalid_save_without_writes() {
        let options = GenerateOptions::default();
        assert!(
            generate_yaml_output(&[], &options)
                .unwrap_err()
                .contains("启用节点")
        );
        let mut nodes = build_nodes(
            vec![Proxy::new("direct", "test", "127.0.0.1", 1)],
            "test",
            "test",
        );
        nodes[0].enabled = false;
        assert!(generate_yaml_output(&nodes, &options).is_err());
        let directory = TestDirectory::new();
        let store = Store::open_in_memory().unwrap();
        assert!(
            save_generated_yaml(&store, &directory.0, None, "test", "proxies: wrong", "{}")
                .is_err()
        );
        assert!(save_generated_yaml(&store, &directory.0, None, " ", "proxies: []", "{}").is_err());
        assert!(store.list_saved_configs().unwrap().is_empty());
        assert!(!directory.0.exists());
    }

    #[test]
    fn restart_restores_same_save_target_and_keeps_historical_conflict_baseline() {
        let directory = TestDirectory::new();
        std::fs::create_dir_all(&directory.0).unwrap();
        let database = directory.0.join("test.sqlite3");
        let first;
        {
            let store = Store::open(&database).unwrap();
            first = save_generated_yaml(&store, &directory.0, None, "日常", "proxies: []", "{}")
                .unwrap();
            store
                .set_setting("generated-target:1", &association_json(&first).unwrap())
                .unwrap();
        }
        let store = Store::open(&database).unwrap();
        let association = store.get_setting("generated-target:1").unwrap().unwrap();
        let restored =
            restore_association(&association, &store.list_saved_configs().unwrap()).unwrap();
        assert_eq!(restored.id, first.id);
        let repeated = save_generated_yaml(
            &store,
            &directory.0,
            Some(&restored),
            "日常",
            "proxies: []",
            "{}",
        )
        .unwrap();
        assert_eq!(repeated.id, first.id);
        assert_eq!(store.config_history(&first.id).unwrap().len(), 1);
        store
            .save_yaml_config(
                &directory.0,
                Some(&first.id),
                "手动修改",
                "proxies: []\n# manual",
                None,
            )
            .unwrap();
        let restored =
            restore_association(&association, &store.list_saved_configs().unwrap()).unwrap();
        assert_eq!(restored.name, "日常");
        let error = save_generated_yaml(
            &store,
            &directory.0,
            Some(&restored),
            "日常",
            "proxies: []",
            "{}",
        )
        .unwrap_err();
        assert!(error.contains("避免覆盖"));
        assert_eq!(store.list_saved_configs().unwrap()[0].name, "手动修改");
        assert_eq!(store.list_saved_configs().unwrap().len(), 1);
        assert!(
            restore_association(&association, &[])
                .unwrap_err()
                .contains("已删除")
        );
        assert!(restore_association("invalid", &store.list_saved_configs().unwrap()).is_err());
    }

    #[test]
    fn saving_does_not_overwrite_external_file_edits() {
        let directory = TestDirectory::new();
        let store = Store::open_in_memory().unwrap();
        let first =
            save_generated_yaml(&store, &directory.0, None, "test", "proxies: []", "{}").unwrap();
        let manual = "proxies: []\n# edited outside SwiftSub";
        std::fs::write(first.path(&directory.0), manual).unwrap();
        assert!(
            save_generated_yaml(
                &store,
                &directory.0,
                Some(&first),
                "test",
                "proxies: []",
                "{}"
            )
            .unwrap_err()
            .contains("外部修改")
        );
        assert_eq!(
            std::fs::read_to_string(first.path(&directory.0)).unwrap(),
            manual
        );
        assert_eq!(store.config_history(&first.id).unwrap().len(), 1);
    }

    #[test]
    fn generated_save_preserves_saved_editor_drafts_external_edits_and_deleted_records() {
        let directory = TestDirectory::new();
        let store = Store::open_in_memory().unwrap();
        let first =
            save_generated_yaml(&store, &directory.0, None, "test", "proxies: []", "{}").unwrap();
        for prefix in ["yaml-editor", "visual-form", "visual-meta"] {
            let key = format!("{prefix}:{}", first.id);
            store.set_setting(&key, "unfinished text").unwrap();
            let error = save_generated_yaml(
                &store,
                &directory.0,
                Some(&first),
                "test",
                "proxies: []\nrules: []",
                "{}",
            )
            .unwrap_err();
            assert!(error.contains("未完成"), "{error}");
            assert_eq!(
                store.get_setting(&key).unwrap().as_deref(),
                Some("unfinished text")
            );
            assert_eq!(store.list_saved_configs().unwrap()[0].yaml, first.yaml);
            store.set_setting(&key, "").unwrap();
        }
        let manual_yaml = "proxies: []\n# manual edit\nrules: []";
        store
            .save_yaml_config(&directory.0, Some(&first.id), "renamed", manual_yaml, None)
            .unwrap();
        let error = save_generated_yaml(
            &store,
            &directory.0,
            Some(&first),
            "test",
            "proxies: []",
            "{}",
        )
        .unwrap_err();
        assert!(error.contains("避免覆盖"));
        assert_eq!(
            std::fs::read_to_string(first.path(&directory.0)).unwrap(),
            manual_yaml
        );
        store.delete_saved_config(&directory.0, &first.id).unwrap();
        let error = save_generated_yaml(
            &store,
            &directory.0,
            Some(&first),
            "test",
            "proxies: []",
            "{}",
        )
        .unwrap_err();
        assert!(error.contains("已删除"));
        assert!(store.list_saved_configs().unwrap().is_empty());
    }
    #[test]
    fn verge_import_preserves_unicode_name_and_nested_subscription_url() {
        let name = "工作 & 游戏 + 100% #1";
        let nested = "http://127.0.0.1:8765/p/id/config.yaml?token=a+b%2Fc&x=1#section";
        let uri = clash_import_uri(nested, name);
        let parsed = url::Url::parse(&uri).unwrap();
        let pairs: Vec<_> = parsed.query_pairs().collect();
        assert_eq!(parsed.scheme(), "clash");
        assert_eq!(parsed.host_str(), Some("install-config"));
        assert_eq!(pairs[0].0, "name");
        assert_eq!(pairs[0].1, name);
        assert_eq!(pairs[1].0, "url");
        assert_eq!(pairs[1].1, nested);
        assert_eq!(pairs.len(), 2);
        // The raw tail must be one encoded URL, with no outer parameters after it.
        let raw_tail = parsed.query().unwrap().split_once("url=").unwrap().1;
        assert!(!raw_tail.contains('&'));
        assert!(!raw_tail.contains('+'));
    }

    #[test]
    fn validates_yaml_before_saving_without_reformatting_it() {
        for text in [
            "",
            "a: [",
            "- a",
            "proxies: wrong",
            "proxy-providers: []",
            "rules: []",
            "proxies: []\nrules: ['FINAL,DIRECT']",
        ] {
            assert!(validate_yaml(text).is_err(), "{text}");
        }
        for text in [
            "# keep comment\nproxies: []\nrules: ['MATCH,DIRECT']",
            "proxy-providers: {}\nproxy-groups: []",
        ] {
            assert!(validate_yaml(text).is_ok(), "{text}");
        }
    }
}
