use super::*;
use std::fs::{self, OpenOptions};
use std::io::{Error, ErrorKind, Write};
use std::path::PathBuf;

#[derive(Clone, Debug)]
pub struct SavedConfig {
    pub id: String,
    pub name: String,
    pub yaml: String,
    pub source_json: Option<String>,
    pub created_at: i64,
    pub updated_at: i64,
}
pub struct SavedConfigWrite<'a> {
    pub id: Option<&'a str>,
    pub name: &'a str,
    pub yaml: &'a str,
    pub source_json: Option<&'a str>,
    pub reason: &'a str,
    /// Explicitly loaded external file content; otherwise the saved database body is the baseline.
    pub file_baseline: Option<&'a str>,
}
impl SavedConfig {
    pub fn path(&self, directory: &Path) -> PathBuf {
        directory.join(format!("{}.yaml", self.id))
    }
}
fn row_config(row: &Row<'_>) -> rusqlite::Result<SavedConfig> {
    Ok(SavedConfig {
        id: row.get(0)?,
        name: row.get(1)?,
        yaml: row.get(2)?,
        source_json: row.get(3)?,
        created_at: row.get(4)?,
        updated_at: row.get(5)?,
    })
}
fn atomic_yaml(path: &Path, yaml: &str) -> std::io::Result<()> {
    atomic_file(path, yaml.as_bytes())
}
fn atomic_file(path: &Path, data: &[u8]) -> std::io::Result<()> {
    let temporary = path.with_extension(format!("{}.tmp", Uuid::new_v4().simple()));
    let result = (|| {
        let mut file = OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&temporary)?;
        file.write_all(data)?;
        file.sync_all()?;
        drop(file);
        fs::rename(&temporary, path)
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result
}
impl Store {
    pub fn check_saved_config_files(&self, directory: &Path) -> Result<()> {
        for config in self.list_saved_configs()? {
            Uuid::parse_str(&config.id)
                .map_err(|_| Error::new(ErrorKind::InvalidData, "无效配置 ID"))?;
            match fs::read(config.path(directory)) {
                Ok(data) if data != config.yaml.as_bytes() => {
                    return Err(Error::new(
                        ErrorKind::InvalidData,
                        format!(
                            "配置「{}」有未载入的文件修改，请先载入或另存；原文件已保留",
                            config.name
                        ),
                    )
                    .into());
                }
                Ok(_) => {}
                Err(error) if error.kind() == ErrorKind::NotFound => {}
                Err(error) => return Err(error.into()),
            }
        }
        Ok(())
    }
    pub fn list_saved_configs(&self) -> Result<Vec<SavedConfig>> {
        let mut statement = self.conn.prepare("SELECT id,name,yaml,source_json,created_at,updated_at FROM saved_configs ORDER BY updated_at DESC,created_at DESC,rowid DESC")?;
        Ok(statement
            .query_map([], row_config)?
            .collect::<rusqlite::Result<_>>()?)
    }
    pub fn save_yaml_config(
        &self,
        directory: &Path,
        id: Option<&str>,
        name: &str,
        yaml: &str,
        source_json: Option<&str>,
    ) -> Result<SavedConfig> {
        self.save_config_batch(
            directory,
            &[SavedConfigWrite {
                id,
                name,
                yaml,
                source_json,
                reason: "保存配置",
                file_baseline: None,
            }],
        )?
        .pop()
        .ok_or_else(|| Error::new(ErrorKind::InvalidData, "没有保存结果").into())
    }

    pub fn save_config_batch(
        &self,
        directory: &Path,
        updates: &[SavedConfigWrite<'_>],
    ) -> Result<Vec<SavedConfig>> {
        let mut prepared = Vec::new();
        let mut seen = std::collections::HashSet::new();
        for update in updates {
            if update.name.trim().is_empty() || update.yaml.trim().is_empty() {
                return Err(
                    Error::new(ErrorKind::InvalidInput, "Name and YAML are required").into(),
                );
            }
            let previous = update.id.map(|id| self.conn.query_row("SELECT id,name,yaml,source_json,created_at,updated_at FROM saved_configs WHERE id=?1", [id], row_config)).transpose()?;
            let id = previous
                .as_ref()
                .map(|config| config.id.clone())
                .unwrap_or_else(|| Uuid::new_v4().simple().to_string());
            Uuid::parse_str(&id)
                .map_err(|_| Error::new(ErrorKind::InvalidInput, "Invalid config ID"))?;
            if !seen.insert(id.clone()) {
                return Err(Error::new(ErrorKind::InvalidInput, "Duplicate config ID").into());
            }
            let config = SavedConfig {
                id,
                name: update.name.trim().into(),
                yaml: update.yaml.into(),
                source_json: update.source_json.map(str::to_owned),
                created_at: previous
                    .as_ref()
                    .map(|config| config.created_at)
                    .unwrap_or_else(now_ts),
                updated_at: now_ts(),
            };
            let file_before = match fs::read(config.path(directory)) {
                Ok(data) => Some(data),
                Err(error) if error.kind() == ErrorKind::NotFound => None,
                Err(error) => return Err(error.into()),
            };
            if let Some(data) = &file_before {
                let baseline = update
                    .file_baseline
                    .or_else(|| previous.as_ref().map(|old| old.yaml.as_str()));
                if baseline.map(str::as_bytes) != Some(data.as_slice()) {
                    return Err(Error::new(
                        ErrorKind::InvalidData,
                        format!(
                            "配置「{}」的文件已被外部修改，请载入文件修改或另存一份；原文件已保留",
                            config.name
                        ),
                    )
                    .into());
                }
            }
            prepared.push((config, previous, file_before, update.reason));
        }
        fs::create_dir_all(directory)?;
        let tx = self.conn.unchecked_transaction()?;
        let mut written = Vec::new();
        let result = (|| -> Result<()> {
            for (config, previous, file_before, reason) in &prepared {
                tx.execute("INSERT INTO saved_configs(id,name,yaml,source_json,created_at,updated_at) VALUES(?1,?2,?3,?4,?5,?6) ON CONFLICT(id) DO UPDATE SET name=excluded.name,yaml=excluded.yaml,source_json=excluded.source_json,updated_at=excluded.updated_at", params![config.id,config.name,config.yaml,config.source_json,config.created_at,config.updated_at])?;
                if let Some(previous) = previous {
                    super::history::record_revision(&tx, previous, "修改前配置")?;
                }
                super::history::record_revision(&tx, config, reason)?;
                let current_file = match fs::read(config.path(directory)) {
                    Ok(data) => Some(data),
                    Err(error) if error.kind() == ErrorKind::NotFound => None,
                    Err(error) => return Err(error.into()),
                };
                if current_file != *file_before {
                    return Err(Error::new(
                        ErrorKind::InvalidData,
                        format!("配置「{}」的文件在保存期间改变，请重新载入", config.name),
                    )
                    .into());
                }
                atomic_yaml(&config.path(directory), &config.yaml)?;
                written.push((config, file_before));
            }
            tx.commit()?;
            Ok(())
        })();
        if let Err(error) = result {
            let mut rollback_errors = Vec::new();
            for (config, file_before) in written.into_iter().rev() {
                let restored = if let Some(data) = file_before {
                    atomic_file(&config.path(directory), data)
                } else {
                    fs::remove_file(config.path(directory))
                };
                if let Err(rollback_error) = restored {
                    rollback_errors.push(format!("{}: {rollback_error}", config.name));
                }
            }
            if !rollback_errors.is_empty() {
                return Err(Error::other(format!(
                    "{error}；文件回滚未完成：{}",
                    rollback_errors.join("；")
                ))
                .into());
            }
            return Err(error);
        }
        Ok(prepared
            .into_iter()
            .map(|(config, _, _, _)| config)
            .collect())
    }
    pub fn delete_saved_config(&self, directory: &Path, id: &str) -> Result<()> {
        Uuid::parse_str(id).map_err(|_| Error::new(ErrorKind::InvalidInput, "无效配置 ID"))?;
        let path = directory.join(format!("{id}.yaml"));
        let temporary = path.with_extension(format!("{}.deleted", Uuid::new_v4().simple()));
        let tx = self.conn.unchecked_transaction()?;
        tx.execute("DELETE FROM saved_configs WHERE id=?1", [id])?;
        let moved = if path.exists() {
            fs::rename(&path, &temporary)?;
            true
        } else {
            false
        };
        if let Err(error) = tx.commit() {
            if moved {
                fs::rename(&temporary, &path)?;
            }
            return Err(error.into());
        }
        if moved {
            let _ = fs::remove_file(temporary);
        }
        Ok(())
    }
    // Rebuild missing files after migration; existing files may contain external edits.
    pub fn repair_saved_config_files(&self, directory: &Path) -> Result<()> {
        let configs = self.list_saved_configs()?;
        if configs.is_empty() {
            return Ok(());
        }
        fs::create_dir_all(directory)?;
        for config in configs {
            Uuid::parse_str(&config.id)
                .map_err(|_| Error::new(ErrorKind::InvalidData, "无效配置 ID"))?;
            let path = config.path(directory);
            match fs::read(&path) {
                Ok(_) => {}
                Err(error) if error.kind() == ErrorKind::NotFound => {
                    atomic_yaml(&path, &config.yaml)?
                }
                Err(error) => return Err(error.into()),
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn repair_and_default_saves_preserve_external_edits() {
        let root = std::env::temp_dir().join(format!("swiftsub-external-{}", Uuid::new_v4()));
        let store = Store::open_in_memory().unwrap();
        let first = store
            .save_yaml_config(&root, None, "external", "proxies: []", None)
            .unwrap();
        let second = store
            .save_yaml_config(&root, None, "missing", "rules: []", None)
            .unwrap();
        let external = b"# external comment\r\nproxies: []\r\n";
        fs::write(first.path(&root), external).unwrap();
        assert!(store.check_saved_config_files(&root).is_err());
        fs::remove_file(second.path(&root)).unwrap();
        store.repair_saved_config_files(&root).unwrap();
        assert_eq!(fs::read(first.path(&root)).unwrap(), external);
        assert_eq!(fs::read_to_string(second.path(&root)).unwrap(), second.yaml);
        let error = store
            .save_config_batch(
                &root,
                &[
                    SavedConfigWrite {
                        id: Some(&second.id),
                        name: "missing",
                        yaml: "rules: [new]",
                        source_json: None,
                        reason: "sync",
                        file_baseline: None,
                    },
                    SavedConfigWrite {
                        id: Some(&first.id),
                        name: "external",
                        yaml: "proxies: [new]",
                        source_json: None,
                        reason: "sync",
                        file_baseline: None,
                    },
                ],
            )
            .unwrap_err()
            .to_string();
        assert!(error.contains("external"), "{error}");
        assert_eq!(fs::read(first.path(&root)).unwrap(), external);
        assert_eq!(fs::read_to_string(second.path(&root)).unwrap(), second.yaml);
        for old in [first, second] {
            let current = store
                .list_saved_configs()
                .unwrap()
                .into_iter()
                .find(|config| config.id == old.id)
                .unwrap();
            assert_eq!(current.yaml, old.yaml);
            assert_eq!(store.config_history(&old.id).unwrap().len(), 1);
        }
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn explicit_loaded_file_baseline_rejects_subsequent_external_changes() {
        let root = std::env::temp_dir().join(format!("swiftsub-baseline-{}", Uuid::new_v4()));
        let store = Store::open_in_memory().unwrap();
        let config = store
            .save_yaml_config(&root, None, "loaded", "proxies: []", None)
            .unwrap();
        let loaded = "# loaded externally\nproxies: []\n";
        let later = "# another external edit\nproxies: []\n";
        fs::write(config.path(&root), later).unwrap();
        let write = SavedConfigWrite {
            id: Some(&config.id),
            name: "loaded",
            yaml: "# repaired\nproxies: []\n",
            source_json: None,
            reason: "loaded file",
            file_baseline: Some(loaded),
        };
        assert!(store.save_config_batch(&root, &[write]).is_err());
        assert_eq!(fs::read_to_string(config.path(&root)).unwrap(), later);
        assert_eq!(store.list_saved_configs().unwrap()[0].yaml, config.yaml);
        assert_eq!(store.config_history(&config.id).unwrap().len(), 1);
        fs::write(config.path(&root), loaded).unwrap();
        let saved = store
            .save_config_batch(
                &root,
                &[SavedConfigWrite {
                    id: Some(&config.id),
                    name: "loaded",
                    yaml: "# repaired\nproxies: []\n",
                    source_json: None,
                    reason: "loaded file",
                    file_baseline: Some(loaded),
                }],
            )
            .unwrap()
            .pop()
            .unwrap();
        assert_eq!(fs::read_to_string(config.path(&root)).unwrap(), saved.yaml);
        assert_eq!(store.config_history(&config.id).unwrap().len(), 2);
        store.check_saved_config_files(&root).unwrap();
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn later_sql_failure_restores_exact_file_bytes_and_missing_file_state() {
        let root = std::env::temp_dir().join(format!("swiftsub-rollback-{}", Uuid::new_v4()));
        let store = Store::open_in_memory().unwrap();
        let configs: Vec<_> = ["external", "missing", "failure"]
            .into_iter()
            .map(|name| {
                store
                    .save_yaml_config(&root, None, name, "proxies: []", Some("metadata"))
                    .unwrap()
            })
            .collect();
        let external = "# preserve this exact text\r\nproxies: []\r\n";
        fs::write(configs[0].path(&root), external).unwrap();
        fs::remove_file(configs[1].path(&root)).unwrap();
        store.conn.execute_batch(&format!("CREATE TRIGGER fail_later BEFORE UPDATE ON saved_configs WHEN NEW.id = '{}' BEGIN SELECT RAISE(ABORT,'later failure'); END;", configs[2].id)).unwrap();
        let writes: Vec<_> = configs
            .iter()
            .enumerate()
            .map(|(index, config)| SavedConfigWrite {
                id: Some(&config.id),
                name: "updated",
                yaml: "proxies: [new]",
                source_json: None,
                reason: "batch",
                file_baseline: (index == 0).then_some(external),
            })
            .collect();
        let error = store
            .save_config_batch(&root, &writes)
            .unwrap_err()
            .to_string();
        assert!(error.contains("later failure"), "{error}");
        assert_eq!(
            fs::read(configs[0].path(&root)).unwrap(),
            external.as_bytes()
        );
        assert!(!configs[1].path(&root).exists());
        assert_eq!(
            fs::read_to_string(configs[2].path(&root)).unwrap(),
            configs[2].yaml
        );
        for old in configs {
            let current = store
                .list_saved_configs()
                .unwrap()
                .into_iter()
                .find(|config| config.id == old.id)
                .unwrap();
            assert_eq!(
                (current.name, current.yaml, current.source_json),
                (old.name, old.yaml, old.source_json)
            );
            assert_eq!(store.config_history(&old.id).unwrap().len(), 1);
        }
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn batch_failure_rolls_back_prior_files_records_and_history() {
        let root = std::env::temp_dir().join(format!("swiftsub-batch-{}", Uuid::new_v4()));
        let store = Store::open_in_memory().unwrap();
        let first = store
            .save_yaml_config(&root, None, "first", "proxies: []", None)
            .unwrap();
        let second = store
            .save_yaml_config(&root, None, "second", "rules: []", None)
            .unwrap();
        fs::remove_file(second.path(&root)).unwrap();
        fs::create_dir(second.path(&root)).unwrap();
        let result = store.save_config_batch(
            &root,
            &[
                SavedConfigWrite {
                    id: Some(&first.id),
                    name: "first",
                    yaml: "proxies: [new]",
                    source_json: None,
                    reason: "同步",
                    file_baseline: None,
                },
                SavedConfigWrite {
                    id: Some(&second.id),
                    name: "second",
                    yaml: "rules: [new]",
                    source_json: None,
                    reason: "同步",
                    file_baseline: None,
                },
            ],
        );
        assert!(result.is_err());
        assert_eq!(fs::read_to_string(first.path(&root)).unwrap(), first.yaml);
        for config in [&first, &second] {
            assert_eq!(store.config_history(&config.id).unwrap().len(), 1);
            assert_eq!(
                store
                    .list_saved_configs()
                    .unwrap()
                    .iter()
                    .find(|saved| saved.id == config.id)
                    .unwrap()
                    .yaml,
                config.yaml
            );
        }
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn independent_files_update_duplicate_reopen_and_delete() {
        let root =
            std::env::temp_dir().join(format!("swiftsub-library-{}", Uuid::new_v4().simple()));
        let db = root.join("store.db");
        let dir = root.join("configs");
        let store = Store::open(&db).unwrap();
        let first = store
            .save_yaml_config(&dir, None, "同名", "proxies: []", Some("source"))
            .unwrap();
        let second = store
            .save_yaml_config(&dir, None, "同名", "proxies: [two]", None)
            .unwrap();
        assert_ne!(first.id, second.id);
        store
            .save_yaml_config(
                &dir,
                Some(&first.id),
                "改名",
                "proxies: [one]",
                Some("new source"),
            )
            .unwrap();
        assert_eq!(fs::read_to_string(second.path(&dir)).unwrap(), second.yaml);
        assert_eq!(store.list_saved_configs().unwrap().len(), 2);
        // SQL failure must leave the old file intact.
        store.conn.execute_batch("CREATE TRIGGER fail_config BEFORE UPDATE ON saved_configs BEGIN SELECT RAISE(ABORT,'failure'); END;").unwrap();
        assert!(
            store
                .save_yaml_config(&dir, Some(&first.id), "失败", "invalid", None)
                .is_err()
        );
        assert_eq!(
            fs::read_to_string(first.path(&dir)).unwrap(),
            "proxies: [one]"
        );
        drop(store);
        let store = Store::open(&db).unwrap();
        fs::remove_file(first.path(&dir)).unwrap();
        store.repair_saved_config_files(&dir).unwrap();
        assert_eq!(
            fs::read_to_string(first.path(&dir)).unwrap(),
            "proxies: [one]"
        );
        assert_eq!(
            store
                .list_saved_configs()
                .unwrap()
                .iter()
                .find(|c| c.id == first.id)
                .unwrap()
                .source_json
                .as_deref(),
            Some("new source")
        );
        store.delete_saved_config(&dir, &second.id).unwrap();
        assert!(!second.path(&dir).exists());
        assert_eq!(store.list_saved_configs().unwrap().len(), 1);
        assert!(store.delete_saved_config(&dir, "../escape").is_err());
        drop(store);
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn upgrades_version_two_without_losing_existing_profiles() {
        let root =
            std::env::temp_dir().join(format!("swiftsub-upgrade-{}", Uuid::new_v4().simple()));
        let path = root.join("old.db");
        let store = Store::open(&path).unwrap();
        let profile = store.get_or_create_profile("existing").unwrap();
        store.save_profile_yaml(profile.id, "proxies: []").unwrap();
        store
            .conn
            .execute_batch("DROP TABLE saved_configs; PRAGMA user_version=2;")
            .unwrap();
        drop(store);
        let store = Store::open(&path).unwrap();
        assert!(store.list_saved_configs().unwrap().is_empty());
        assert_eq!(
            store.list_profiles().unwrap()[0].last_yaml.as_deref(),
            Some("proxies: []")
        );
        store
            .save_yaml_config(&root.join("configs"), None, "saved", "proxies: []", None)
            .unwrap();
        assert_eq!(store.list_saved_configs().unwrap().len(), 1);
        drop(store);
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn filesystem_failure_does_not_create_or_replace_records() {
        let root =
            std::env::temp_dir().join(format!("swiftsub-library-{}", Uuid::new_v4().simple()));
        fs::create_dir_all(&root).unwrap();
        let blocker = root.join("not-a-directory");
        fs::write(&blocker, "blocked").unwrap();
        let store = Store::open_in_memory().unwrap();
        assert!(
            store
                .save_yaml_config(&blocker, None, "test", "proxies: []", None)
                .is_err()
        );
        assert!(store.list_saved_configs().unwrap().is_empty());
        let dir = root.join("configs");
        let saved = store
            .save_yaml_config(&dir, None, "old", "proxies: []", None)
            .unwrap();
        fs::remove_file(saved.path(&dir)).unwrap();
        fs::create_dir(saved.path(&dir)).unwrap();
        assert!(
            store
                .save_yaml_config(&dir, Some(&saved.id), "new", "proxies: [new]", None)
                .is_err()
        );
        assert_eq!(store.list_saved_configs().unwrap()[0].yaml, "proxies: []");
        fs::remove_dir(saved.path(&dir)).unwrap();
        store.repair_saved_config_files(&dir).unwrap();
        assert_eq!(fs::read_to_string(saved.path(&dir)).unwrap(), "proxies: []");
        fs::remove_dir_all(root).unwrap();
    }
}
