use super::*;

#[derive(Clone, Debug)]
pub struct ConfigRevision {
    pub id: i64,
    pub config_id: String,
    pub name: String,
    pub yaml: String,
    pub source_json: Option<String>,
    pub reason: String,
    pub created_at: i64,
}

impl Store {
    pub fn config_history(&self, id: &str) -> Result<Vec<ConfigRevision>> {
        let mut statement = self.conn.prepare("SELECT id,config_id,name,yaml,source_json,reason,created_at FROM config_revisions WHERE config_id=?1 ORDER BY id DESC LIMIT 20")?;
        Ok(statement
            .query_map([id], |row| {
                Ok(ConfigRevision {
                    id: row.get(0)?,
                    config_id: row.get(1)?,
                    name: row.get(2)?,
                    yaml: row.get(3)?,
                    source_json: row.get(4)?,
                    reason: row.get(5)?,
                    created_at: row.get(6)?,
                })
            })?
            .collect::<rusqlite::Result<_>>()?)
    }
}

pub(super) fn record_revision(conn: &Connection, config: &SavedConfig, reason: &str) -> Result<()> {
    let last: Option<(i64, String, String, Option<String>, String, i64)> = conn.query_row(
        "SELECT id,name,yaml,source_json,reason,created_at FROM config_revisions WHERE config_id=?1 ORDER BY id DESC LIMIT 1", [&config.id],
        |row| Ok((row.get(0)?,row.get(1)?,row.get(2)?,row.get(3)?,row.get(4)?,row.get(5)?))
    ).optional()?;
    if last.as_ref().is_some_and(|(_, name, yaml, source, _, _)| {
        name == &config.name && yaml == &config.yaml && source == &config.source_json
    }) {
        return Ok(());
    }
    let count: i64 = conn.query_row(
        "SELECT COUNT(*) FROM config_revisions WHERE config_id=?1",
        [&config.id],
        |row| row.get(0),
    )?;
    if let Some((id, _, _, _, previous_reason, at)) = last
        && reason == "自动保存"
        && previous_reason == reason
        && count > 1
        && now_ts().saturating_sub(at) < 60
    {
        conn.execute(
            "UPDATE config_revisions SET name=?1,yaml=?2,source_json=?3,created_at=?4 WHERE id=?5",
            params![config.name, config.yaml, config.source_json, now_ts(), id],
        )?;
    } else {
        conn.execute("INSERT INTO config_revisions(config_id,name,yaml,source_json,reason,created_at) VALUES(?1,?2,?3,?4,?5,?6)", params![config.id,config.name,config.yaml,config.source_json,reason,now_ts()])?;
    }
    conn.execute("DELETE FROM config_revisions WHERE config_id=?1 AND id NOT IN (SELECT id FROM config_revisions WHERE config_id=?1 ORDER BY id DESC LIMIT 20)", [&config.id])?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn history_coalesces_retains_twenty_and_survives_backup_and_rollback() {
        let root = std::env::temp_dir().join(format!("swiftsub-history-{}", Uuid::new_v4()));
        let directory = root.join("configs");
        let store = Store::open(root.join("store.db")).unwrap();
        let first = store
            .save_yaml_config(&directory, None, "initial", "proxies: []", Some("metadata"))
            .unwrap();
        let save = |yaml: &str, reason: &str| {
            store
                .save_config_batch(
                    &directory,
                    &[SavedConfigWrite {
                        file_baseline: None,
                        id: Some(&first.id),
                        name: "initial",
                        yaml,
                        source_json: Some("metadata"),
                        reason,
                    }],
                )
                .unwrap();
        };
        save("proxies: [one]", "自动保存");
        save("proxies: [two]", "自动保存");
        assert_eq!(store.config_history(&first.id).unwrap().len(), 2);
        for index in 0..25 {
            save(&format!("proxies: [{index}]"), "同步订阅");
        }
        assert_eq!(store.config_history(&first.id).unwrap().len(), 20);
        let revision = store.config_history(&first.id).unwrap()[5].clone();
        save(&revision.yaml, "恢复历史版本");
        assert_eq!(store.list_saved_configs().unwrap()[0].id, first.id);
        assert_eq!(
            store.config_history(&first.id).unwrap()[0].reason,
            "恢复历史版本"
        );
        let profile = store.get_or_create_profile("profile").unwrap();
        store.save_profile_yaml(profile.id, "proxies: []").unwrap();
        store.set_setting("draft:test", "unsaved draft").unwrap();
        let backup = root.join("backup.db");
        store.backup_to(backup.to_str().unwrap()).unwrap();
        drop(store);
        let restored = Store::open(&backup).unwrap();
        restored.validate_database().unwrap();
        restored
            .repair_saved_config_files(&root.join("restored-configs"))
            .unwrap();
        assert_eq!(restored.config_history(&first.id).unwrap().len(), 20);
        assert_eq!(
            restored.list_saved_configs().unwrap()[0].yaml,
            revision.yaml
        );
        assert_eq!(
            restored.get_setting("draft:test").unwrap().as_deref(),
            Some("unsaved draft")
        );
        assert_eq!(restored.list_profiles().unwrap().len(), 1);
        assert_eq!(
            std::fs::read_to_string(
                root.join("restored-configs")
                    .join(format!("{}.yaml", first.id))
            )
            .unwrap(),
            revision.yaml
        );
        drop(restored);
        std::fs::remove_dir_all(root).unwrap();
    }
}
