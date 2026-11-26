use super::*;

#[derive(Serialize, Deserialize)]
pub(super) struct SavedYamlDraft {
    pub name: String,
    pub yaml: String,
    #[serde(default)]
    pub file_baseline: Option<String>,
}

#[derive(Deserialize)]
#[serde(untagged)]
enum StoredDraft {
    Current(SavedYamlDraft),
    Legacy((String, String)),
}

pub(super) fn read_yaml_draft(store: &Store, id: &str) -> Result<Option<SavedYamlDraft>, String> {
    let Some(json) = store
        .get_setting(&format!("yaml-editor:{id}"))
        .map_err(|error| error.to_string())?
        .filter(|json| !json.trim().is_empty())
    else {
        return Ok(None);
    };
    match serde_json::from_str::<StoredDraft>(&json)
        .map_err(|error| format!("YAML 草稿读取失败，原数据已保留：{error}"))?
    {
        StoredDraft::Current(draft) => Ok(Some(draft)),
        StoredDraft::Legacy((name, yaml)) => Ok(Some(SavedYamlDraft {
            name,
            yaml,
            file_baseline: None,
        })),
    }
}

impl SwiftSubApp {
    pub(crate) fn check_saved_file_state(&mut self) -> Result<(), String> {
        let Some(config) = self
            .saved_config_id
            .as_ref()
            .and_then(|id| self.saved_configs.iter().find(|config| &config.id == id))
        else {
            return Ok(());
        };
        let path = config.path(&self.data_dir.join("configs"));
        match std::fs::read(&path) {
            Ok(content) => {
                let baseline = self.saved_file_baseline.as_deref().unwrap_or(&config.yaml);
                self.saved_external_change = content != baseline.as_bytes();
                if self.saved_external_change {
                    return Err(
                        "配置文件已被外部修改，原文件与当前草稿均已保留。可载入文件修改或另存一份"
                            .into(),
                    );
                }
                if self.saved_file_baseline.as_deref() == Some(config.yaml.as_str()) {
                    self.saved_file_baseline = None;
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                self.saved_external_change = false;
            }
            Err(error) => return Err(format!("配置文件读取失败：{error}")),
        }
        Ok(())
    }

    pub fn load_saved_external_file(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let result = (|| -> Result<(), String> {
            self.checkpoint_yaml_editor(cx)?;
            let config = self
                .saved_config_id
                .as_ref()
                .and_then(|id| self.saved_configs.iter().find(|config| &config.id == id))
                .cloned()
                .ok_or("请选择配置")?;
            let yaml = std::fs::read_to_string(config.path(&self.data_dir.join("configs")))
                .map_err(|error| format!("配置文件读取失败：{error}"))?;
            let store = self.store.lock().map_err(|error| error.to_string())?;
            for prefix in ["yaml-editor", "visual-form", "visual-meta"] {
                if let Some(draft) = store
                    .get_setting(&format!("{prefix}:{}", config.id))
                    .map_err(|error| error.to_string())?
                    .filter(|draft| !draft.is_empty())
                {
                    store
                        .set_setting(
                            &format!("preserved-{prefix}:{}:{}", config.id, uuid::Uuid::new_v4()),
                            &draft,
                        )
                        .map_err(|error| error.to_string())?;
                }
            }
            drop(store);
            self.saved_file_baseline = (yaml != config.yaml).then(|| yaml.clone());
            self.saved_external_change = false;
            self.saved_config_error = None;
            self.saved_config_yaml
                .update(cx, |input, cx| input.set_value(yaml, window, cx));
            self.load_saved_visual(window, cx);
            if self.saved_config_error.is_some() {
                self.saved_editor.tab = super::super::SavedEditorTab::Yaml;
            }
            self.checkpoint_yaml_editor(cx)?;
            Ok(())
        })();
        match result {
            Ok(()) => self.show_toast("已载入文件修改，之前的编辑草稿已保留".into(), cx),
            Err(error) => self.saved_config_error = Some(error),
        }
        cx.notify();
    }

    pub fn recover_saved_yaml_draft(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let result = (|| -> Result<String, String> {
            let id = self.saved_config_id.clone().ok_or("请选择配置")?;
            let store = self.store.lock().map_err(|error| error.to_string())?;
            for prefix in ["yaml-editor", "visual-meta", "visual-form"] {
                let key = format!("{prefix}:{id}");
                if let Some(draft) = store
                    .get_setting(&key)
                    .map_err(|error| error.to_string())?
                    .filter(|draft| !draft.trim().is_empty())
                {
                    store
                        .set_setting(
                            &format!("preserved-{prefix}:{id}:{}", uuid::Uuid::new_v4()),
                            &draft,
                        )
                        .map_err(|error| error.to_string())?;
                }
            }
            for prefix in ["yaml-editor", "visual-meta", "visual-form"] {
                store
                    .set_setting(&format!("{prefix}:{id}"), "")
                    .map_err(|error| error.to_string())?;
            }
            Ok(id)
        })();
        match result {
            Ok(id) => {
                self.saved_config_id = None;
                self.open_saved_config(id, window, cx);
                self.show_toast("损坏草稿已备份，已恢复保存的配置".into(), cx);
            }
            Err(error) => self.saved_config_error = Some(error),
        }
        cx.notify();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn old_yaml_drafts_restore_and_file_baselines_survive_restart() {
        let store = Store::open_in_memory().unwrap();
        store
            .set_setting("yaml-editor:test", "[\"name\",\"proxies: []\"]")
            .unwrap();
        assert_eq!(
            read_yaml_draft(&store, "test").unwrap().unwrap().name,
            "name"
        );
        let draft = SavedYamlDraft {
            name: "external".into(),
            yaml: "unfinished".into(),
            file_baseline: Some("external version".into()),
        };
        store
            .set_setting("yaml-editor:test", &serde_json::to_string(&draft).unwrap())
            .unwrap();
        assert_eq!(
            read_yaml_draft(&store, "test")
                .unwrap()
                .unwrap()
                .file_baseline,
            draft.file_baseline
        );
        store.set_setting("yaml-editor:test", "{broken").unwrap();
        assert!(read_yaml_draft(&store, "test").is_err());
        assert_eq!(
            store.get_setting("yaml-editor:test").unwrap().as_deref(),
            Some("{broken")
        );
    }
}
