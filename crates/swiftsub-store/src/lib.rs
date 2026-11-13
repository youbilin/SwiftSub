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

CREATE TABLE IF NOT EXISTS refresh_logs (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    sub_id INTEGER REFERENCES subscriptions(id) ON DELETE SET NULL,
    at INTEGER NOT NULL,
    ok INTEGER NOT NULL,
    message TEXT
);
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

        let version: i64 = conn.query_row("PRAGMA user_version", [], |row| row.get(0))?;
        if version < 1 {
            conn.execute_batch(SCHEMA)?;
            conn.pragma_update(None, "user_version", 1)?;
        }
        if version < 2 {
            let has_extras = {
                let mut statement = conn.prepare("PRAGMA table_info(node_overrides)")?;
                let names = statement.query_map([], |row| row.get::<_, String>(1))?;
                names.filter_map(Result::ok).any(|name| name == "extras")
            };
            if !has_extras {
                conn.execute("ALTER TABLE node_overrides ADD COLUMN extras TEXT", [])?;
            }
            conn.pragma_update(None, "user_version", 2)?;
        }

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
             SET name = ?1, url = ?2, enabled = ?3, refresh_minutes = ?4,
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
    ) -> Result<()> {
        let now = now_ts();
        match traffic {
            Some(traffic) => {
                self.conn.execute(
                    "UPDATE subscriptions
                     SET last_ok_at = ?1, last_error = NULL, traffic_upload = ?2,
                         traffic_download = ?3, traffic_total = ?4, expire_at = ?5,
                         updated_at = ?1
                     WHERE id = ?6",
                    params![
                        now,
                        traffic.upload as i64,
                        traffic.download as i64,
                        traffic.total as i64,
                        traffic.expire_at,
                        id
                    ],
                )?;
            }
            None => {
                self.conn.execute(
                    "UPDATE subscriptions
                     SET last_ok_at = ?1, last_error = NULL, updated_at = ?1
                     WHERE id = ?2",
                    params![now, id],
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
        self.conn.execute(
            "INSERT INTO refresh_logs (sub_id, at, ok, message) VALUES (?1, ?2, ?3, ?4)",
            params![sub_id, now_ts(), ok as i64, message],
        )?;
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

    pub fn delete_custom_template(&self, id: i64) -> Result<()> {
        self.conn
            .execute("DELETE FROM templates WHERE id = ?1 AND builtin = 0", [id])?;
        Ok(())
    }

    pub fn backup_to(&self, path: &str) -> Result<()> {
        self.conn.execute("VACUUM INTO ?1", params![path])?;
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
        if let Some(profile) = self.default_profile()? {
            return Ok(profile);
        }

        let now = now_ts();
        let token = Uuid::new_v4().simple().to_string();
        self.conn.execute(
            "INSERT INTO profiles (name, token, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?3)",
            params![name, token, now],
        )?;
        self.default_profile()?
            .ok_or(StoreError::Sqlite(rusqlite::Error::QueryReturnedNoRows))
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
        self.conn.execute(
            "UPDATE profiles SET last_yaml = ?1, generated_at = ?2, updated_at = ?2 WHERE id = ?3",
            params![yaml, now, id],
        )?;
        Ok(())
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

        store.save_fetch_not_modified(first, None).unwrap();
        let saved = store.get_subscription(first).unwrap().unwrap();
        assert!(saved.last_error.is_none());
        assert!(saved.last_ok_at.is_some());

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
