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
    let temporary = path.with_extension(format!("{}.tmp", Uuid::new_v4().simple()));
    let result = (|| {
        let mut file = OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&temporary)?;
        file.write_all(yaml.as_bytes())?;
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
        if name.trim().is_empty() || yaml.trim().is_empty() {
            return Err(Error::new(ErrorKind::InvalidInput, "配置名称和 YAML 不能为空").into());
        }
        let previous = if let Some(id) = id {
            Some(self.conn.query_row("SELECT id,name,yaml,source_json,created_at,updated_at FROM saved_configs WHERE id=?1", [id], row_config)?)
        } else {
            None
        };
        let id = previous
            .as_ref()
            .map(|c| c.id.clone())
            .unwrap_or_else(|| Uuid::new_v4().simple().to_string());
        Uuid::parse_str(&id).map_err(|_| Error::new(ErrorKind::InvalidData, "无效配置 ID"))?;
        fs::create_dir_all(directory)?;
        let config = SavedConfig {
            id,
            name: name.trim().into(),
            yaml: yaml.into(),
            source_json: source_json.map(str::to_string),
            created_at: previous
                .as_ref()
                .map(|c| c.created_at)
                .unwrap_or_else(now_ts),
            updated_at: now_ts(),
        };
        let tx = self.conn.unchecked_transaction()?;
        tx.execute("INSERT INTO saved_configs (id,name,yaml,source_json,created_at,updated_at) VALUES (?1,?2,?3,?4,?5,?6) ON CONFLICT(id) DO UPDATE SET name=excluded.name,yaml=excluded.yaml,source_json=excluded.source_json,updated_at=excluded.updated_at", params![config.id,config.name,config.yaml,config.source_json,config.created_at,config.updated_at])?;
        let path = config.path(directory);
        atomic_yaml(&path, &config.yaml)?;
        if let Err(error) = tx.commit() {
            if let Some(previous) = previous {
                atomic_yaml(&path, &previous.yaml)?;
            } else {
                fs::remove_file(&path)?;
            }
            return Err(error.into());
        }
        Ok(config)
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
    // SQLite is authoritative; repair interrupted file writes and directory migrations on startup.
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
            if fs::read(&path).ok().as_deref() != Some(config.yaml.as_bytes()) {
                atomic_yaml(&path, &config.yaml)?;
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
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
