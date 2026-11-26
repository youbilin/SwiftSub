mod history;
mod saved_configs;
pub use history::ConfigRevision;
pub use saved_configs::{SavedConfig, SavedConfigWrite};
use std::collections::HashMap;
use std::path::Path;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use rusqlite::{Connection, OptionalExtension, Row, params};
use uuid::Uuid;

const SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS settings (
    key TEXT PRIMARY KEY,
    value TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS subscriptions (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    name TEXT NOT NULL,
    kind TEXT NOT NULL,
    url TEXT,
    user_agent TEXT,
    enabled INTEGER NOT NULL DEFAULT 1,
    refresh_minutes INTEGER NOT NULL DEFAULT 1440,
    payload TEXT,
    etag TEXT,
    last_modified TEXT,
    last_ok_at INTEGER,
    last_error TEXT,
    traffic_upload INTEGER NOT NULL DEFAULT 0,
    traffic_download INTEGER NOT NULL DEFAULT 0,
    traffic_total INTEGER NOT NULL DEFAULT 0,
    expire_at INTEGER,
    node_count INTEGER NOT NULL DEFAULT 0,
    sort_order INTEGER NOT NULL DEFAULT 0,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS node_overrides (
    sub_id INTEGER NOT NULL REFERENCES subscriptions(id) ON DELETE CASCADE,
    node_key TEXT NOT NULL,
    alias TEXT,
    enabled INTEGER,
    listen_port INTEGER,
    sort_order INTEGER,
    extras TEXT,
    PRIMARY KEY (sub_id, node_key)
);

CREATE TABLE IF NOT EXISTS templates (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    name TEXT NOT NULL,
    builtin INTEGER NOT NULL DEFAULT 0,
    config_json TEXT NOT NULL,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS profiles (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    name TEXT NOT NULL,
    template_id INTEGER,
    token TEXT NOT NULL UNIQUE,
    options_json TEXT,
    last_yaml TEXT,
    generated_at INTEGER,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS saved_configs (
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL,
    yaml TEXT NOT NULL,
    source_json TEXT,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS refresh_logs (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    sub_id INTEGER REFERENCES subscriptions(id) ON DELETE SET NULL,
    at INTEGER NOT NULL,
    ok INTEGER NOT NULL,
    message TEXT
);

CREATE TABLE IF NOT EXISTS config_revisions (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    config_id TEXT NOT NULL REFERENCES saved_configs(id) ON DELETE CASCADE,
    name TEXT NOT NULL,
    yaml TEXT NOT NULL,
    source_json TEXT,
    reason TEXT NOT NULL,
    created_at INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS revisions_by_config ON config_revisions(config_id,id DESC);
"#;

#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    #[error("sqlite: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
}

pub type Result<T, E = StoreError> = std::result::Result<T, E>;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SubscriptionKind {
    Url,
    Yaml,
    Links,
    File,
}

impl SubscriptionKind {
    pub fn as_str(self) -> &'static str {
        match self {
            SubscriptionKind::Url => "url",
            SubscriptionKind::Yaml => "yaml",
            SubscriptionKind::Links => "links",
            SubscriptionKind::File => "file",
        }
    }

    pub fn from_db(value: &str) -> Self {
        match value {
            "yaml" => SubscriptionKind::Yaml,
            "links" => SubscriptionKind::Links,
            "file" => SubscriptionKind::File,
            _ => SubscriptionKind::Url,
        }
    }
}

#[derive(Clone, Debug)]
pub struct Subscription {
    pub id: i64,
    pub name: String,
    pub kind: SubscriptionKind,
    pub url: Option<String>,
    pub user_agent: Option<String>,
    pub enabled: bool,
    pub refresh_minutes: u32,
    pub payload: Option<String>,
    pub etag: Option<String>,
    pub last_modified: Option<String>,
    pub last_ok_at: Option<i64>,
    pub last_error: Option<String>,
    pub traffic_upload: u64,
    pub traffic_download: u64,
    pub traffic_total: u64,
    pub expire_at: Option<i64>,
    pub node_count: usize,
    pub sort_order: i64,
}

#[derive(Clone, Debug)]
pub struct NewSubscription {
    pub name: String,
    pub kind: SubscriptionKind,
    pub url: Option<String>,
    pub payload: Option<String>,
    pub user_agent: Option<String>,
}

impl NewSubscription {
    pub fn url(name: impl Into<String>, url: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            kind: SubscriptionKind::Url,
            url: Some(url.into()),
            payload: None,
            user_agent: None,
        }
    }

    pub fn manual(
        name: impl Into<String>,
        kind: SubscriptionKind,
        payload: impl Into<String>,
    ) -> Self {
        Self {
            name: name.into(),
            kind,
            url: None,
            payload: Some(payload.into()),
            user_agent: None,
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct TrafficSnapshot {
    pub upload: u64,
    pub download: u64,
    pub total: u64,
    pub expire_at: Option<i64>,
}

#[derive(Clone, Debug)]
pub struct NodeOverride {
    pub sub_id: i64,
    pub node_key: String,
    pub alias: Option<String>,
    pub enabled: Option<bool>,
    pub listen_port: Option<u16>,
    pub sort_order: Option<i64>,
    pub extras: Option<String>,
}

#[derive(Clone, Debug)]
pub struct Profile {
    pub id: i64,
    pub name: String,
    pub token: String,
    pub options_json: Option<String>,
    pub last_yaml: Option<String>,
    pub generated_at: Option<i64>,
}

#[derive(Clone, Debug)]
pub struct CustomTemplate {
    pub id: i64,
    pub name: String,
    pub config_json: String,
    pub created_at: i64,
}

#[derive(Clone, Debug)]
pub struct RefreshLog {
    pub id: i64,
    pub sub_id: Option<i64>,
    pub at: i64,
    pub ok: bool,
    pub message: Option<String>,
}

pub struct Store {
    conn: Connection,
}

impl Store {
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let conn = Connection::open(path)?;
        Self::from_connection(conn)
    }

    pub fn open_in_memory() -> Result<Self> {
        let conn = Connection::open_in_memory()?;
        Self::from_connection(conn)
    }

    fn from_connection(conn: Connection) -> Result<Self> {
        conn.busy_timeout(Duration::from_secs(5))?;
        conn.execute_batch("PRAGMA foreign_keys = ON; PRAGMA journal_mode = WAL;")?;

        let migration = conn.unchecked_transaction()?;
        let version: i64 = migration.query_row("PRAGMA user_version", [], |row| row.get(0))?;
        if version > 4 {
            return Err(rusqlite::Error::InvalidQuery.into());
        }
        if version < 1 {
            migration.execute_batch(SCHEMA)?;
            migration.pragma_update(None, "user_version", 1)?;
        }
        if version < 2 {
            let has_extras = {
                let mut statement = migration.prepare("PRAGMA table_info(node_overrides)")?;
                let names = statement.query_map([], |row| row.get::<_, String>(1))?;
                names.filter_map(Result::ok).any(|name| name == "extras")
            };
            if !has_extras {
                migration.execute("ALTER TABLE node_overrides ADD COLUMN extras TEXT", [])?;
            }
            migration.pragma_update(None, "user_version", 2)?;
        }

        if version < 3 {
            migration.execute_batch(SCHEMA)?;
            migration.pragma_update(None, "user_version", 3)?;
        }
        if version < 4 {
            migration.execute_batch(SCHEMA)?;
            migration.execute("INSERT INTO config_revisions(config_id,name,yaml,source_json,reason,created_at) SELECT id,name,yaml,source_json,'升级前配置',updated_at FROM saved_configs WHERE id NOT IN (SELECT config_id FROM config_revisions)", [])?;
            migration.pragma_update(None, "user_version", 4)?;
        }

        migration.commit()?;

        Ok(Self { conn })
    }

    pub fn add_subscription(&self, subscription: &NewSubscription) -> Result<i64> {
        let now = now_ts();
        let sort_order: i64 = self.conn.query_row(
            "SELECT COALESCE(MAX(sort_order), 0) + 1 FROM subscriptions",
            [],
            |row| row.get(0),
        )?;

        self.conn.execute(
            "INSERT INTO subscriptions
             (name, kind, url, user_agent, enabled, refresh_minutes, payload, sort_order, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, 1, 1440, ?5, ?6, ?7, ?7)",
            params![
                subscription.name,
                subscription.kind.as_str(),
                subscription.url,
                subscription.user_agent,
                subscription.payload,
                sort_order,
                now,
            ],
        )?;

        Ok(self.conn.last_insert_rowid())
    }

    pub fn list_subscriptions(&self) -> Result<Vec<Subscription>> {
        let mut statement = self.conn.prepare(
            "SELECT id, name, kind, url, user_agent, enabled, refresh_minutes, payload,
                    etag, last_modified, last_ok_at, last_error, traffic_upload,
                    traffic_download, traffic_total, expire_at, node_count, sort_order
             FROM subscriptions
             ORDER BY sort_order, id",
        )?;
        let rows = statement.query_map([], map_subscription)?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    pub fn get_subscription(&self, id: i64) -> Result<Option<Subscription>> {
        Ok(self
            .conn
            .query_row(
                "SELECT id, name, kind, url, user_agent, enabled, refresh_minutes, payload,
                        etag, last_modified, last_ok_at, last_error, traffic_upload,
                        traffic_download, traffic_total, expire_at, node_count, sort_order
                 FROM subscriptions WHERE id = ?1",
                [id],
                map_subscription,
            )
            .optional()?)
    }

    pub fn update_subscription_meta(
        &self,
        id: i64,
        name: &str,
        url: Option<&str>,
        enabled: bool,
        refresh_minutes: u32,
        user_agent: Option<&str>,
    ) -> Result<()> {
        self.conn.execute(
            "UPDATE subscriptions
             SET etag = CASE WHEN url IS NOT ?2 OR user_agent IS NOT ?5 THEN NULL ELSE etag END,
                 last_modified = CASE WHEN url IS NOT ?2 OR user_agent IS NOT ?5 THEN NULL ELSE last_modified END,
                 last_ok_at = CASE WHEN url IS NOT ?2 OR user_agent IS NOT ?5 THEN NULL ELSE last_ok_at END,
                 name = ?1, url = ?2, enabled = ?3, refresh_minutes = ?4,
                 user_agent = ?5, updated_at = ?6
             WHERE id = ?7",
            params![
                name,
                url,
                enabled as i64,
                refresh_minutes,
                user_agent,
                now_ts(),
                id
            ],
        )?;
        Ok(())
    }

    pub fn set_subscription_enabled(&self, id: i64, enabled: bool) -> Result<()> {
        self.conn.execute(
            "UPDATE subscriptions SET enabled = ?1, updated_at = ?2 WHERE id = ?3",
            params![enabled as i64, now_ts(), id],
        )?;
        Ok(())
    }

    pub fn delete_subscription(&self, id: i64) -> Result<()> {
        self.conn
            .execute("DELETE FROM subscriptions WHERE id = ?1", [id])?;
        Ok(())
    }

    pub fn save_fetch_success(
        &self,
        id: i64,
        payload: &str,
        etag: Option<&str>,
        last_modified: Option<&str>,
        traffic: &TrafficSnapshot,
        node_count: usize,
    ) -> Result<()> {
        let now = now_ts();
        self.conn.execute(
            "UPDATE subscriptions
             SET payload = ?1, etag = ?2, last_modified = ?3, last_ok_at = ?4,
                 last_error = NULL, traffic_upload = ?5, traffic_download = ?6,
                 traffic_total = ?7, expire_at = ?8, node_count = ?9, updated_at = ?4
             WHERE id = ?10",
            params![
                payload,
                etag,
                last_modified,
                now,
                traffic.upload as i64,
                traffic.download as i64,
                traffic.total as i64,
                traffic.expire_at,
                node_count as i64,
                id
            ],
        )?;
        Ok(())
    }

    pub fn save_manual_payload(&self, id: i64, payload: &str, node_count: usize) -> Result<()> {
        self.conn.execute(
            "UPDATE subscriptions
             SET payload = ?1, kind = 'yaml', node_count = ?2, last_error = NULL, updated_at = ?3
             WHERE id = ?4",
            params![payload, node_count as i64, now_ts(), id],
        )?;
        Ok(())
    }

    pub fn set_subscription_node_count(&self, id: i64, node_count: usize) -> Result<()> {
        self.conn.execute(
            "UPDATE subscriptions SET node_count = ?1, updated_at = ?2 WHERE id = ?3",
            params![node_count as i64, now_ts(), id],
        )?;
        Ok(())
    }

    pub fn save_fetch_not_modified(
        &self,
        id: i64,
        traffic: Option<&TrafficSnapshot>,
        etag: Option<&str>,
        last_modified: Option<&str>,
    ) -> Result<()> {
        let now = now_ts();
        match traffic {
            Some(traffic) => {
                self.conn.execute(
                    "UPDATE subscriptions
                     SET last_ok_at = ?1, last_error = NULL, traffic_upload = ?2,
                         traffic_download = ?3, traffic_total = ?4, expire_at = ?5,
                         updated_at = ?1, etag = COALESCE(?7, etag),
                         last_modified = COALESCE(?8, last_modified)
                     WHERE id = ?6",
                    params![
                        now,
                        traffic.upload as i64,
                        traffic.download as i64,
                        traffic.total as i64,
                        traffic.expire_at,
                        id,
                        etag,
                        last_modified
                    ],
                )?;
            }
            None => {
                self.conn.execute(
                    "UPDATE subscriptions
                     SET last_ok_at = ?1, last_error = NULL, updated_at = ?1,
                         etag = COALESCE(?3, etag), last_modified = COALESCE(?4, last_modified)
                     WHERE id = ?2",
                    params![now, id, etag, last_modified],
                )?;
            }
        }
        Ok(())
    }

    pub fn set_subscription_refresh_minutes(&self, id: i64, minutes: u32) -> Result<()> {
        self.conn.execute(
            "UPDATE subscriptions SET refresh_minutes = ?1, updated_at = ?2 WHERE id = ?3",
            params![minutes.min(i32::MAX as u32) as i64, now_ts(), id],
        )?;
        Ok(())
    }

    pub fn save_fetch_error(&self, id: i64, message: &str) -> Result<()> {
        self.conn.execute(
            "UPDATE subscriptions
             SET last_error = ?1, updated_at = ?2
             WHERE id = ?3",
            params![message, now_ts(), id],
        )?;
        Ok(())
    }

    pub fn log_refresh(&self, sub_id: Option<i64>, ok: bool, message: Option<&str>) -> Result<()> {
        let tx = self.conn.unchecked_transaction()?;
        tx.execute(
            "INSERT INTO refresh_logs (sub_id, at, ok, message) VALUES (?1, ?2, ?3, ?4)",
            params![sub_id, now_ts(), ok as i64, message],
        )?;
        tx.execute("DELETE FROM refresh_logs WHERE id NOT IN (SELECT id FROM refresh_logs ORDER BY id DESC LIMIT 1000)", [])?;
        tx.commit()?;
        Ok(())
    }

    pub fn recent_refresh_logs(&self, limit: u32) -> Result<Vec<RefreshLog>> {
        let mut statement = self.conn.prepare(
            "SELECT id, sub_id, at, ok, message FROM refresh_logs
             ORDER BY at DESC, id DESC LIMIT ?1",
        )?;
        let rows = statement.query_map([limit], |row| {
            Ok(RefreshLog {
                id: row.get(0)?,
                sub_id: row.get(1)?,
                at: row.get(2)?,
                ok: row.get::<_, i64>(3)? != 0,
                message: row.get(4)?,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    pub fn list_node_overrides(&self, sub_id: i64) -> Result<HashMap<String, NodeOverride>> {
        let mut statement = self.conn.prepare(
            "SELECT sub_id, node_key, alias, enabled, listen_port, sort_order, extras
             FROM node_overrides WHERE sub_id = ?1",
        )?;
        let rows = statement.query_map([sub_id], |row| {
            Ok(NodeOverride {
                sub_id: row.get(0)?,
                node_key: row.get(1)?,
                alias: row.get(2)?,
                enabled: row.get::<_, Option<i64>>(3)?.map(|value| value != 0),
                listen_port: row
                    .get::<_, Option<i64>>(4)?
                    .and_then(|value| u16::try_from(value).ok()),
                sort_order: row.get(5)?,
                extras: row.get(6)?,
            })
        })?;

        let mut overrides = HashMap::new();
        for row in rows {
            let value = row?;
            overrides.insert(value.node_key.clone(), value);
        }
        Ok(overrides)
    }

    pub fn upsert_node_override(&self, value: &NodeOverride) -> Result<()> {
        self.conn.execute(
            "INSERT INTO node_overrides (sub_id, node_key, alias, enabled, listen_port, sort_order, extras)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
             ON CONFLICT(sub_id, node_key) DO UPDATE SET
                alias = excluded.alias,
                enabled = excluded.enabled,
                listen_port = excluded.listen_port,
                sort_order = excluded.sort_order,
                extras = excluded.extras",
            params![
                value.sub_id,
                value.node_key,
                value.alias,
                value.enabled.map(|enabled| enabled as i64),
                value.listen_port.map(i64::from),
                value.sort_order,
                value.extras
            ],
        )?;
        Ok(())
    }

    pub fn set_node_override_alias(&self, sub_id: i64, node_key: &str, alias: &str) -> Result<()> {
        self.conn.execute(
            "INSERT INTO node_overrides (sub_id, node_key, alias) VALUES (?1, ?2, ?3)
             ON CONFLICT(sub_id, node_key) DO UPDATE SET alias = excluded.alias",
            params![sub_id, node_key, alias],
        )?;
        Ok(())
    }

    pub fn set_node_override_fields(&self, sub_id: i64, node_key: &str, yaml: &str) -> Result<()> {
        self.conn.execute(
            "INSERT INTO node_overrides (sub_id, node_key, extras) VALUES (?1, ?2, ?3)
             ON CONFLICT(sub_id, node_key) DO UPDATE SET extras = excluded.extras",
            params![sub_id, node_key, yaml],
        )?;
        Ok(())
    }

    pub fn set_node_override_enabled(
        &self,
        sub_id: i64,
        node_key: &str,
        enabled: bool,
    ) -> Result<()> {
        self.conn.execute(
            "INSERT INTO node_overrides (sub_id, node_key, enabled) VALUES (?1, ?2, ?3)
             ON CONFLICT(sub_id, node_key) DO UPDATE SET enabled = excluded.enabled",
            params![sub_id, node_key, enabled as i64],
        )?;
        Ok(())
    }

    pub fn delete_node_override(&self, sub_id: i64, node_key: &str) -> Result<()> {
        self.conn.execute(
            "DELETE FROM node_overrides WHERE sub_id = ?1 AND node_key = ?2",
            params![sub_id, node_key],
        )?;
        Ok(())
    }

    pub fn list_custom_templates(&self) -> Result<Vec<CustomTemplate>> {
        let mut statement = self.conn.prepare(
            "SELECT id, name, config_json, created_at FROM templates
             WHERE builtin = 0 ORDER BY updated_at DESC, id DESC",
        )?;
        let rows = statement.query_map([], |row| {
            Ok(CustomTemplate {
                id: row.get(0)?,
                name: row.get(1)?,
                config_json: row.get(2)?,
                created_at: row.get(3)?,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    pub fn add_custom_template(&self, name: &str, config_json: &str) -> Result<i64> {
        let now = now_ts();
        self.conn.execute(
            "INSERT INTO templates (name, builtin, config_json, created_at, updated_at)
             VALUES (?1, 0, ?2, ?3, ?3)",
            params![name, config_json, now],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    /// Persist a routing snapshot and an optional template in one SQLite transaction.
    pub fn save_routing_snapshot(
        &self,
        settings: &HashMap<String, String>,
        template: Option<(Option<i64>, &str, &str)>,
    ) -> Result<Option<i64>> {
        self.save_routing_profile_snapshot(settings, template, None)
    }

    /// Commit active profile, template and legacy settings together.
    pub fn save_routing_profile_snapshot(
        &self,
        settings: &HashMap<String, String>,
        template: Option<(Option<i64>, &str, &str)>,
        profile: Option<(i64, &str)>,
    ) -> Result<Option<i64>> {
        self.save_routing_profile_with_node_overrides(settings, template, profile, &[])
    }

    /// Keep node edits and all references to them in the same transaction.
    pub fn save_routing_profile_with_node_overrides(
        &self,
        settings: &HashMap<String, String>,
        template: Option<(Option<i64>, &str, &str)>,
        profile: Option<(i64, &str)>,
        overrides: &[NodeOverride],
    ) -> Result<Option<i64>> {
        let tx = self.conn.unchecked_transaction()?;
        for value in overrides {
            tx.execute(
                "INSERT INTO node_overrides (sub_id,node_key,alias,enabled,listen_port,sort_order,extras)
                 VALUES (?1,?2,?3,?4,?5,?6,?7)
                 ON CONFLICT(sub_id,node_key) DO UPDATE SET alias=excluded.alias,enabled=excluded.enabled,
                 listen_port=excluded.listen_port,sort_order=excluded.sort_order,extras=excluded.extras",
                params![value.sub_id, value.node_key, value.alias, value.enabled.map(i64::from),
                    value.listen_port.map(i64::from), value.sort_order, value.extras],
            )?;
        }
        if let Some((id, json)) = profile
            && tx.execute(
                "UPDATE profiles SET options_json=?1,updated_at=?2 WHERE id=?3",
                params![json, now_ts(), id],
            )? != 1
        {
            return Err(rusqlite::Error::QueryReturnedNoRows.into());
        }
        let template_id = if let Some((id, name, json)) = template {
            let now = now_ts();
            if let Some(id) = id {
                let changed = tx.execute("UPDATE templates SET name=?1, config_json=?2, updated_at=?3 WHERE id=?4 AND builtin=0", params![name,json,now,id])?;
                if changed != 1 {
                    return Err(rusqlite::Error::QueryReturnedNoRows.into());
                }
                Some(id)
            } else {
                tx.execute("INSERT INTO templates (name,builtin,config_json,created_at,updated_at) VALUES (?1,0,?2,?3,?3)",params![name,json,now])?;
                Some(tx.last_insert_rowid())
            }
        } else {
            None
        };
        for (key, value) in settings {
            tx.execute("INSERT INTO settings (key,value) VALUES (?1,?2) ON CONFLICT(key) DO UPDATE SET value=excluded.value",params![key,value])?;
        }
        tx.commit()?;
        Ok(template_id)
    }

    pub fn delete_custom_template(&self, id: i64) -> Result<()> {
        self.conn
            .execute("DELETE FROM templates WHERE id = ?1 AND builtin = 0", [id])?;
        Ok(())
    }

    pub fn backup_to(&self, path: &str) -> Result<()> {
        self.conn.execute("VACUUM INTO ?1", params![path])?;
        Ok(())
    }

    pub fn validate_database(&self) -> Result<()> {
        let integrity: String = self
            .conn
            .query_row("PRAGMA integrity_check", [], |row| row.get(0))?;
        if integrity != "ok" {
            return Err(rusqlite::Error::InvalidQuery.into());
        }
        let mut check = self.conn.prepare("PRAGMA foreign_key_check")?;
        if check.query([])?.next()?.is_some() {
            return Err(rusqlite::Error::InvalidQuery.into());
        }
        Ok(())
    }

    pub fn set_setting(&self, key: &str, value: &str) -> Result<()> {
        self.conn.execute(
            "INSERT INTO settings (key, value) VALUES (?1, ?2)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            params![key, value],
        )?;
        Ok(())
    }

    pub fn get_setting(&self, key: &str) -> Result<Option<String>> {
        Ok(self
            .conn
            .query_row("SELECT value FROM settings WHERE key = ?1", [key], |row| {
                row.get(0)
            })
            .optional()?)
    }

    pub fn all_settings(&self) -> Result<HashMap<String, String>> {
        let mut statement = self.conn.prepare("SELECT key, value FROM settings")?;
        let rows = statement.query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?;
        Ok(rows.collect::<rusqlite::Result<HashMap<_, _>>>()?)
    }

    pub fn get_or_create_profile(&self, name: &str) -> Result<Profile> {
        if let Some(profile) = self.list_profiles()?.into_iter().find(|p| p.name == name) {
            return Ok(profile);
        }

        let now = now_ts();
        let token = Uuid::new_v4().simple().to_string();
        self.conn.execute(
            "INSERT INTO profiles (name, token, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?3)",
            params![name, token, now],
        )?;
        self.get_profile_by_token(&token)?
            .ok_or(StoreError::Sqlite(rusqlite::Error::QueryReturnedNoRows))
    }

    pub fn list_profiles(&self) -> Result<Vec<Profile>> {
        let mut stmt = self.conn.prepare("SELECT id, name, token, options_json, last_yaml, generated_at FROM profiles ORDER BY id")?;
        Ok(stmt
            .query_map([], map_profile)?
            .collect::<rusqlite::Result<Vec<_>>>()?)
    }

    pub fn rename_profile(&self, id: i64, name: &str) -> Result<()> {
        self.conn.execute(
            "UPDATE profiles SET name=?1, updated_at=?2 WHERE id=?3",
            params![name, now_ts(), id],
        )?;
        Ok(())
    }

    pub fn delete_profile(&self, id: i64) -> Result<bool> {
        Ok(self.conn.execute(
            "DELETE FROM profiles WHERE id=?1 AND (SELECT COUNT(*) FROM profiles)>1",
            [id],
        )? > 0)
    }

    pub fn default_profile(&self) -> Result<Option<Profile>> {
        Ok(self
            .conn
            .query_row(
                "SELECT id, name, token, options_json, last_yaml, generated_at
                 FROM profiles ORDER BY id LIMIT 1",
                [],
                map_profile,
            )
            .optional()?)
    }

    pub fn get_profile_by_token(&self, token: &str) -> Result<Option<Profile>> {
        Ok(self
            .conn
            .query_row(
                "SELECT id, name, token, options_json, last_yaml, generated_at
                 FROM profiles WHERE token = ?1",
                [token],
                map_profile,
            )
            .optional()?)
    }

    pub fn save_profile_yaml(&self, id: i64, yaml: &str) -> Result<()> {
        let now = now_ts();
        let changed = self.conn.execute(
            "UPDATE profiles SET last_yaml = ?1, generated_at = ?2, updated_at = ?2 WHERE id = ?3",
            params![yaml, now, id],
        )?;
        if changed == 0 {
            return Err(rusqlite::Error::QueryReturnedNoRows.into());
        }
        Ok(())
    }

    /// Accept a background generation only while all input recipes still match.
    pub fn save_generated_outputs(
        &self,
        outputs: &[(i64, Option<String>, String)],
    ) -> Result<bool> {
        let tx = self.conn.unchecked_transaction()?;
        for (id, expected_options, _) in outputs {
            let actual: Option<Option<String>> = tx
                .query_row(
                    "SELECT options_json FROM profiles WHERE id=?1",
                    [id],
                    |row| row.get(0),
                )
                .optional()?;
            if actual.as_ref() != Some(expected_options) {
                return Ok(false);
            }
        }
        for (id, _, yaml) in outputs {
            tx.execute(
                "UPDATE profiles SET last_yaml=?1,generated_at=?2,updated_at=?2 WHERE id=?3",
                params![yaml, now_ts(), id],
            )?;
        }
        tx.commit()?;
        Ok(true)
    }

    pub fn save_profile_options(&self, id: i64, options_json: &str) -> Result<()> {
        self.conn.execute(
            "UPDATE profiles SET options_json = ?1, updated_at = ?2 WHERE id = ?3",
            params![options_json, now_ts(), id],
        )?;
        Ok(())
    }
}

fn map_subscription(row: &Row<'_>) -> rusqlite::Result<Subscription> {
    let kind: String = row.get(2)?;
    Ok(Subscription {
        id: row.get(0)?,
        name: row.get(1)?,
        kind: SubscriptionKind::from_db(&kind),
        url: row.get(3)?,
        user_agent: row.get(4)?,
        enabled: row.get::<_, i64>(5)? != 0,
        refresh_minutes: row.get::<_, i64>(6)?.try_into().unwrap_or(1440),
        payload: row.get(7)?,
        etag: row.get(8)?,
        last_modified: row.get(9)?,
        last_ok_at: row.get(10)?,
        last_error: row.get(11)?,
        traffic_upload: row.get::<_, i64>(12)?.try_into().unwrap_or_default(),
        traffic_download: row.get::<_, i64>(13)?.try_into().unwrap_or_default(),
        traffic_total: row.get::<_, i64>(14)?.try_into().unwrap_or_default(),
        expire_at: row.get(15)?,
        node_count: row.get::<_, i64>(16)?.try_into().unwrap_or_default(),
        sort_order: row.get(17)?,
    })
}

fn map_profile(row: &Row<'_>) -> rusqlite::Result<Profile> {
    Ok(Profile {
        id: row.get(0)?,
        name: row.get(1)?,
        token: row.get(2)?,
        options_json: row.get(3)?,
        last_yaml: row.get(4)?,
        generated_at: row.get(5)?,
    })
}

pub fn now_ts() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs() as i64)
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn subscriptions_roundtrip_and_fetch_state() {
        let store = Store::open_in_memory().unwrap();
        let first = store
            .add_subscription(&NewSubscription::url("机场 A", "https://example.com/sub"))
            .unwrap();
        let second = store
            .add_subscription(&NewSubscription::manual(
                "手动节点",
                SubscriptionKind::Links,
                "trojan://pass@example.com:443#HK",
            ))
            .unwrap();

        let listed = store.list_subscriptions().unwrap();
        assert_eq!(listed.len(), 2);
        assert_eq!(listed[0].id, first);
        assert_eq!(listed[1].id, second);
        assert_eq!(listed[0].kind, SubscriptionKind::Url);

        let traffic = TrafficSnapshot {
            upload: 10,
            download: 20,
            total: 1000,
            expire_at: Some(1_700_000_000),
        };
        store
            .save_fetch_success(
                first,
                "proxies: []",
                Some("etag-1"),
                Some("last-modified"),
                &traffic,
                3,
            )
            .unwrap();
        let saved = store.get_subscription(first).unwrap().unwrap();
        assert_eq!(saved.node_count, 3);
        assert_eq!(saved.traffic_total, 1000);
        assert_eq!(saved.etag.as_deref(), Some("etag-1"));
        assert!(saved.last_error.is_none());

        store.save_fetch_error(first, "timeout").unwrap();
        assert_eq!(
            store
                .get_subscription(first)
                .unwrap()
                .unwrap()
                .last_error
                .as_deref(),
            Some("timeout")
        );

        store
            .save_fetch_not_modified(first, None, None, None)
            .unwrap();
        let saved = store.get_subscription(first).unwrap().unwrap();
        assert!(saved.last_error.is_none());
        assert!(saved.last_ok_at.is_some());
        assert_eq!(saved.etag.as_deref(), Some("etag-1"));
        store
            .save_fetch_not_modified(first, None, Some("etag-2"), None)
            .unwrap();
        let saved = store.get_subscription(first).unwrap().unwrap();
        assert_eq!(saved.etag.as_deref(), Some("etag-2"));
        assert_eq!(saved.last_modified.as_deref(), Some("last-modified"));
        assert_eq!(saved.payload.as_deref(), Some("proxies: []"));
        assert_eq!(saved.traffic_total, traffic.total);
        store
            .save_fetch_not_modified(first, Some(&traffic), None, Some("new-last-modified"))
            .unwrap();
        let saved = store.get_subscription(first).unwrap().unwrap();
        assert_eq!(saved.etag.as_deref(), Some("etag-2"));
        assert_eq!(saved.last_modified.as_deref(), Some("new-last-modified"));

        store.set_subscription_refresh_minutes(first, 60).unwrap();
        assert_eq!(
            store
                .get_subscription(first)
                .unwrap()
                .unwrap()
                .refresh_minutes,
            60
        );

        store.delete_subscription(second).unwrap();
        assert_eq!(store.list_subscriptions().unwrap().len(), 1);
    }

    #[test]
    fn node_overrides_and_settings_roundtrip() {
        let store = Store::open_in_memory().unwrap();
        let id = store
            .add_subscription(&NewSubscription::url("机场", "https://example.com/sub"))
            .unwrap();

        store
            .upsert_node_override(&NodeOverride {
                sub_id: id,
                node_key: "abc".into(),
                alias: Some("香港 01".into()),
                enabled: Some(false),
                listen_port: Some(33011),
                sort_order: Some(2),
                extras: None,
            })
            .unwrap();

        let overrides = store.list_node_overrides(id).unwrap();
        let value = overrides.get("abc").unwrap();
        assert_eq!(value.alias.as_deref(), Some("香港 01"));
        assert_eq!(value.listen_port, Some(33011));
        assert_eq!(value.enabled, Some(false));

        store.set_node_override_enabled(id, "abc", true).unwrap();
        let overrides = store.list_node_overrides(id).unwrap();
        let value = overrides.get("abc").unwrap();
        assert_eq!(value.enabled, Some(true));
        assert_eq!(value.alias.as_deref(), Some("香港 01"));

        store
            .set_node_override_alias(id, "abc", "香港 01 改名")
            .unwrap();
        let overrides = store.list_node_overrides(id).unwrap();
        let value = overrides.get("abc").unwrap();
        assert_eq!(value.alias.as_deref(), Some("香港 01 改名"));
        assert_eq!(value.enabled, Some(true));
        assert_eq!(value.listen_port, Some(33011));

        store.delete_node_override(id, "abc").unwrap();
        assert!(store.list_node_overrides(id).unwrap().is_empty());

        store.set_setting("theme", "dark").unwrap();
        assert_eq!(store.get_setting("theme").unwrap().as_deref(), Some("dark"));
    }

    #[test]
    fn custom_templates_roundtrip() {
        let store = Store::open_in_memory().unwrap();
        let id = store
            .add_custom_template("我的模板", r#"{"preset":"standard"}"#)
            .unwrap();
        let templates = store.list_custom_templates().unwrap();
        assert_eq!(templates.len(), 1);
        assert_eq!(templates[0].name, "我的模板");
        store.delete_custom_template(id).unwrap();
        assert!(store.list_custom_templates().unwrap().is_empty());
    }

    #[test]
    fn profile_and_refresh_logs() {
        let store = Store::open_in_memory().unwrap();
        let profile = store.get_or_create_profile("默认配置").unwrap();
        assert_eq!(profile.token.len(), 32);

        store.save_profile_yaml(profile.id, "rules: []").unwrap();
        let by_token = store.get_profile_by_token(&profile.token).unwrap().unwrap();
        assert_eq!(by_token.last_yaml.as_deref(), Some("rules: []"));

        store.log_refresh(None, true, Some("ok")).unwrap();
        let logs = store.recent_refresh_logs(10).unwrap();
        assert_eq!(logs.len(), 1);
        assert!(logs[0].ok);
    }
}

#[cfg(test)]
mod routing_transaction_tests {
    use super::*;
    #[test]
    fn node_edit_and_renamed_references_commit_or_roll_back_together() {
        let store = Store::open_in_memory().unwrap();
        let sub_id = store
            .add_subscription(&NewSubscription::url("test", "https://example.org/sub"))
            .unwrap();
        let profile = store.get_or_create_profile("test").unwrap();
        store
            .set_node_override_alias(sub_id, "stable-key", "original")
            .unwrap();
        store
            .save_profile_options(profile.id, "original references")
            .unwrap();
        let edited = NodeOverride {
            sub_id,
            node_key: "stable-key".into(),
            alias: Some("renamed".into()),
            enabled: Some(true),
            listen_port: None,
            sort_order: None,
            extras: Some("edited fields".into()),
        };
        assert!(
            store
                .save_routing_profile_with_node_overrides(
                    &HashMap::new(),
                    None,
                    Some((-1, "renamed references")),
                    std::slice::from_ref(&edited)
                )
                .is_err()
        );
        assert_eq!(
            store.list_node_overrides(sub_id).unwrap()["stable-key"]
                .alias
                .as_deref(),
            Some("original")
        );
        assert_eq!(
            store
                .get_profile_by_token(&profile.token)
                .unwrap()
                .unwrap()
                .options_json
                .as_deref(),
            Some("original references")
        );
        store
            .save_routing_profile_with_node_overrides(
                &HashMap::new(),
                None,
                Some((profile.id, "renamed references")),
                &[edited],
            )
            .unwrap();
        assert_eq!(
            store.list_node_overrides(sub_id).unwrap()["stable-key"]
                .alias
                .as_deref(),
            Some("renamed")
        );
        assert_eq!(
            store
                .get_profile_by_token(&profile.token)
                .unwrap()
                .unwrap()
                .options_json
                .as_deref(),
            Some("renamed references")
        );
    }
    #[test]
    fn template_update_and_settings_commit_or_rollback_together() {
        let store = Store::open_in_memory().unwrap();
        let id = store.add_custom_template("original", "{}").unwrap();
        store.set_setting("existing", "old").unwrap();
        let profile = store.get_or_create_profile("test").unwrap();
        store
            .save_profile_options(profile.id, "old-profile")
            .unwrap();
        store.conn.execute_batch("CREATE TRIGGER reject_routing BEFORE INSERT ON settings WHEN NEW.key='blocked' BEGIN SELECT RAISE(FAIL,'injected write failure'); END;").unwrap();
        let settings = HashMap::from([
            ("existing".into(), "new".into()),
            ("blocked".into(), "value".into()),
        ]);
        assert!(
            store
                .save_routing_profile_snapshot(
                    &settings,
                    Some((Some(id), "renamed", "{\"new\":true}")),
                    Some((profile.id, "new-profile"))
                )
                .is_err()
        );
        assert_eq!(
            store.get_setting("existing").unwrap().as_deref(),
            Some("old")
        );
        assert_eq!(store.list_custom_templates().unwrap()[0].name, "original");
        assert!(store.get_setting("blocked").unwrap().is_none());
        assert_eq!(
            store
                .get_profile_by_token(&profile.token)
                .unwrap()
                .unwrap()
                .options_json
                .as_deref(),
            Some("old-profile")
        );
        store
            .conn
            .execute_batch("DROP TRIGGER reject_routing;")
            .unwrap();
        store
            .save_routing_profile_snapshot(
                &settings,
                Some((Some(id), "renamed", "{\"new\":true}")),
                Some((profile.id, "new-profile")),
            )
            .unwrap();
        assert_eq!(
            store
                .get_profile_by_token(&profile.token)
                .unwrap()
                .unwrap()
                .options_json
                .as_deref(),
            Some("new-profile")
        );
        assert_eq!(store.list_custom_templates().unwrap()[0].id, id);
        assert_eq!(store.list_custom_templates().unwrap()[0].name, "renamed");
        assert_eq!(
            store.get_setting("existing").unwrap().as_deref(),
            Some("new")
        );
        assert!(
            store
                .save_routing_snapshot(&settings, Some((Some(id + 999), "missing", "{}")))
                .is_err()
        );
    }
}

#[cfg(test)]
mod profile_tests {
    use super::*;

    #[test]
    fn background_outputs_reject_changed_recipes_and_roll_back_write_failures() {
        let store = Store::open_in_memory().unwrap();
        let one = store.get_or_create_profile("one").unwrap();
        let two = store.get_or_create_profile("two").unwrap();
        store.save_profile_yaml(one.id, "old-one").unwrap();
        store.save_profile_yaml(two.id, "old-two").unwrap();
        let outputs = vec![
            (one.id, None, "new-one".into()),
            (two.id, None, "new-two".into()),
        ];
        store.save_profile_options(two.id, "changed").unwrap();
        assert!(!store.save_generated_outputs(&outputs).unwrap());
        assert_eq!(
            store
                .get_profile_by_token(&one.token)
                .unwrap()
                .unwrap()
                .last_yaml
                .as_deref(),
            Some("old-one")
        );
        let outputs = vec![
            (one.id, None, "new-one".into()),
            (two.id, Some("changed".into()), "new-two".into()),
        ];
        store.conn.execute_batch(&format!("CREATE TRIGGER fail_output BEFORE UPDATE OF last_yaml ON profiles WHEN NEW.id={} BEGIN SELECT RAISE(FAIL,'injected failure'); END;", two.id)).unwrap();
        assert!(store.save_generated_outputs(&outputs).is_err());
        assert_eq!(
            store
                .get_profile_by_token(&one.token)
                .unwrap()
                .unwrap()
                .last_yaml
                .as_deref(),
            Some("old-one")
        );
        assert_eq!(
            store
                .get_profile_by_token(&two.token)
                .unwrap()
                .unwrap()
                .last_yaml
                .as_deref(),
            Some("old-two")
        );
    }

    #[test]
    fn changed_subscription_request_clears_validators_and_keeps_payload() {
        let store = Store::open_in_memory().unwrap();
        let id = store
            .add_subscription(&NewSubscription::url("test", "https://old.example/sub"))
            .unwrap();
        store
            .save_fetch_success(
                id,
                "cached",
                Some("old-etag"),
                Some("old-date"),
                &TrafficSnapshot::default(),
                1,
            )
            .unwrap();
        store
            .update_subscription_meta(
                id,
                "renamed",
                Some("https://old.example/sub"),
                true,
                60,
                None,
            )
            .unwrap();
        assert_eq!(
            store.get_subscription(id).unwrap().unwrap().etag.as_deref(),
            Some("old-etag")
        );
        store
            .update_subscription_meta(
                id,
                "renamed",
                Some("https://new.example/sub"),
                true,
                60,
                Some("custom-agent"),
            )
            .unwrap();
        let subscription = store.get_subscription(id).unwrap().unwrap();
        assert!(subscription.etag.is_none());
        assert!(subscription.last_modified.is_none());
        assert!(subscription.last_ok_at.is_none());
        assert_eq!(subscription.payload.as_deref(), Some("cached"));
    }
    #[test]
    fn profiles_preserve_identity_settings_and_last_profile() {
        let store = Store::open_in_memory().unwrap();
        let one = store.get_or_create_profile("one").unwrap();
        let two = store.get_or_create_profile("two").unwrap();
        assert_ne!(one.id, two.id);
        assert_ne!(one.token, two.token);
        store.save_profile_options(one.id, "first").unwrap();
        store.save_profile_options(two.id, "second").unwrap();
        assert_eq!(
            store
                .get_or_create_profile("one")
                .unwrap()
                .options_json
                .as_deref(),
            Some("first")
        );
        store.rename_profile(two.id, "renamed").unwrap();
        assert_eq!(
            store
                .get_profile_by_token(&two.token)
                .unwrap()
                .unwrap()
                .name,
            "renamed"
        );
        assert!(store.delete_profile(one.id).unwrap());
        assert!(!store.delete_profile(two.id).unwrap());
    }
}
