//! Phase 2 (P1) application state and actions:
//! - LAN fixed port, private-only access control, firewall helper, token rotation, QR modal
//! - Global fetch proxy configuration (system / direct / custom)
//! - Windows silent auto-start, full portable backup/restore, config revision history rollback
//! - Batch node enable/disable/rename, single-node share link export, subscription health & refresh logs

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use swiftsub_fetch::{FetchOptions, ProxyMode};
use swiftsub_store::{ConfigRevision, RefreshLog, SavedConfigWrite, Store};

use super::*;

#[derive(Clone, Debug)]
pub struct QrModal {
    pub title: String,
    pub subtitle: String,
    pub content: String,
    pub svg: Vec<u8>,
    pub size: usize,
    pub runs: Vec<(usize, usize, usize)>,
    pub sensitive: bool,
}

#[derive(Clone, Debug)]
pub struct BackupPreview {
    pub path: PathBuf,
    pub subscriptions: usize,
    pub saved_configs: usize,
    pub templates: usize,
}

#[derive(Clone, Debug)]
pub struct RevisionSummary {
    pub revision: ConfigRevision,
    pub proxies: usize,
    pub groups: usize,
    pub rules: usize,
}

#[derive(Clone, Debug)]
pub struct HistoryModal {
    pub config_id: String,
    pub config_name: String,
    pub revisions: Vec<RevisionSummary>,
}

pub struct Phase2State {
    pub server_fixed_port: Option<u16>,
    pub server_fixed_port_input: Entity<InputState>,
    pub server_lan_private_only: bool,
    pub fetch_proxy_mode: String,
    pub fetch_proxy_url: Entity<InputState>,
    pub fetch_proxy_url_value: String,
    pub auto_start: bool,
    pub backup_restore_pending: bool,
    pub backup_preview: Option<BackupPreview>,
    pub qr_modal: Option<QrModal>,
    pub history_modal: Option<HistoryModal>,
    pub batch_rename_open: bool,
    pub batch_rename_prefix: Entity<InputState>,
    pub batch_rename_find: Entity<InputState>,
    pub batch_rename_replace: Entity<InputState>,
    pub batch_rename_error: Option<String>,
    pub sub_logs_open: bool,
    pub refresh_logs: Vec<RefreshLog>,
    pub theme_mode: crate::ui::AppThemeMode,
    pub summary_cache: HashMap<String, ((i64, usize), workspace::ConfigSummary)>,
    pub mihomo_binary_path: Option<PathBuf>,
    pub mihomo_checking: bool,
}

impl Phase2State {
    pub(super) fn new(
        store: &Mutex<Store>,
        window: &mut Window,
        cx: &mut Context<SwiftSubApp>,
    ) -> Self {
        let (fixed_port, lan_private_only, proxy_mode, proxy_url, theme_mode, mihomo_binary_path) = {
            let guard = lock_store(store);
            let fixed = guard
                .get_setting("server_fixed_port")
                .ok()
                .flatten()
                .and_then(|v| v.trim().parse::<u16>().ok())
                .filter(|&port| port > 0);
            let private_only = guard
                .get_setting("server_lan_private_only")
                .ok()
                .flatten()
                .as_deref()
                != Some("false");
            let mode = guard
                .get_setting("fetch_proxy_mode")
                .ok()
                .flatten()
                .filter(|m| matches!(m.as_str(), "system" | "direct" | "custom"))
                .unwrap_or_else(|| "system".to_string());
            let url = guard
                .get_setting("fetch_proxy_url")
                .ok()
                .flatten()
                .unwrap_or_default();
            let theme = crate::ui::AppThemeMode::from_setting(
                guard.get_setting("theme").ok().flatten().as_deref(),
            );
            let mihomo = guard
                .get_setting("mihomo_binary_path")
                .ok()
                .flatten()
                .map(|p| PathBuf::from(p.trim()))
                .filter(|p| p.is_file())
                .or_else(crate::desktop::detect_mihomo_binary);
            (fixed, private_only, mode, url, theme, mihomo)
        };
        let fixed_port_text = fixed_port.map(|p| p.to_string()).unwrap_or_default();
        let server_fixed_port_input = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("留空自动选择（如 17899）")
                .default_value(fixed_port_text)
        });
        let fetch_proxy_url = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("例如 http://127.0.0.1:7897 或 socks5://127.0.0.1:7897")
                .default_value(proxy_url.clone())
        });
        let batch_rename_prefix =
            cx.new(|cx| InputState::new(window, cx).placeholder("统一前缀（可选），例如 [主力] "));
        let batch_rename_find =
            cx.new(|cx| InputState::new(window, cx).placeholder("查找文本（可选）"));
        let batch_rename_replace =
            cx.new(|cx| InputState::new(window, cx).placeholder("替换为（留空即删除匹配文本）"));

        Self {
            server_fixed_port: fixed_port,
            server_fixed_port_input,
            server_lan_private_only: lan_private_only,
            fetch_proxy_mode: proxy_mode,
            fetch_proxy_url,
            fetch_proxy_url_value: proxy_url,
            auto_start: crate::desktop::is_auto_start_enabled(),
            backup_restore_pending: false,
            backup_preview: None,
            qr_modal: None,
            history_modal: None,
            batch_rename_open: false,
            batch_rename_prefix,
            batch_rename_find,
            batch_rename_replace,
            batch_rename_error: None,
            sub_logs_open: false,
            refresh_logs: Vec::new(),
            theme_mode,
            summary_cache: HashMap::new(),
            mihomo_binary_path,
            mihomo_checking: false,
        }
    }

    pub fn has_modal(&self) -> bool {
        self.qr_modal.is_some()
            || self.history_modal.is_some()
            || self.backup_preview.is_some()
            || self.batch_rename_open
            || self.sub_logs_open
    }
}

/// Apply a staged `restore-pending.db` backup before the main SQLite database is opened.
/// Preserves the previous database as `swiftsub.db.bak`.
pub(super) fn apply_pending_backup_restore(data_dir: &Path) -> Option<String> {
    let pending = data_dir.join("restore-pending.db");
    if !pending.exists() {
        return None;
    }
    let validated = Store::open(&pending).and_then(|store| store.validate_database());
    if let Err(error) = validated {
        let _ = std::fs::remove_file(&pending);
        return Some(format!("待恢复备份校验失败，已保留原数据：{error}"));
    }
    let db_path = data_dir.join("swiftsub.db");
    let bak_path = data_dir.join("swiftsub.db.bak");
    for suffix in ["-wal", "-shm"] {
        let _ = std::fs::remove_file(data_dir.join(format!("swiftsub.db{suffix}")));
        let _ = std::fs::remove_file(data_dir.join(format!("restore-pending.db{suffix}")));
    }
    if db_path.exists() {
        let _ = std::fs::remove_file(&bak_path);
        if let Err(error) = std::fs::rename(&db_path, &bak_path) {
            return Some(format!("备份旧数据失败，未执行恢复：{error}"));
        }
    }
    match std::fs::rename(&pending, &db_path) {
        Ok(()) => Some("已从备份恢复全部数据（原数据库已保留为 swiftsub.db.bak）".into()),
        Err(error) => {
            if bak_path.exists() && !db_path.exists() {
                let _ = std::fs::rename(&bak_path, &db_path);
            }
            Some(format!("应用备份失败，已回退原数据：{error}"))
        }
    }
}

fn summarize_yaml_counts(yaml: &str) -> (usize, usize, usize) {
    let Ok(root) = serde_yaml_ng::from_str::<serde_yaml_ng::Mapping>(yaml) else {
        return (0, 0, 0);
    };
    let count = |key: &str| {
        root.get(serde_yaml_ng::Value::String(key.into()))
            .and_then(serde_yaml_ng::Value::as_sequence)
            .map_or(0, Vec::len)
    };
    (count("proxies"), count("proxy-groups"), count("rules"))
}

pub fn format_bytes_human(bytes: u64) -> String {
    const KB: f64 = 1024.0;
    const MB: f64 = KB * 1024.0;
    const GB: f64 = MB * 1024.0;
    const TB: f64 = GB * 1024.0;
    let b = bytes as f64;
    if b >= TB {
        format!("{:.2} TB", b / TB)
    } else if b >= GB {
        format!("{:.2} GB", b / GB)
    } else if b >= MB {
        format!("{:.1} MB", b / MB)
    } else if b >= KB {
        format!("{:.0} KB", b / KB)
    } else {
        format!("{bytes} B")
    }
}

pub fn validate_proxy_url(raw: &str) -> Result<String, String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err("请输入自定义代理地址".into());
    }
    let parsed = url::Url::parse(trimmed).map_err(|_| {
        "代理地址格式无效，示例：http://127.0.0.1:7897 或 socks5://127.0.0.1:7897".to_string()
    })?;
    if !matches!(
        parsed.scheme(),
        "http" | "https" | "socks5" | "socks5h" | "socks4"
    ) || parsed.host_str().is_none()
    {
        return Err("仅支持 http / https / socks5 / socks5h 代理地址".into());
    }
    Ok(trimmed.to_string())
}

impl SwiftSubApp {
    /// Build `FetchOptions` honoring the user's configured proxy mode.
    pub fn fetch_options(&self) -> FetchOptions {
        let proxy = match self.p1.fetch_proxy_mode.as_str() {
            "direct" => ProxyMode::Direct,
            "custom" if !self.p1.fetch_proxy_url_value.trim().is_empty() => {
                ProxyMode::Custom(self.p1.fetch_proxy_url_value.trim().to_string())
            }
            _ => ProxyMode::System,
        };
        FetchOptions {
            proxy,
            ..FetchOptions::default()
        }
    }

    pub fn set_fetch_proxy_mode(&mut self, mode: &str, cx: &mut Context<Self>) {
        if !matches!(mode, "system" | "direct" | "custom") {
            return;
        }
        self.p1.fetch_proxy_mode = mode.to_string();
        let _ = lock_store(&self.store).set_setting("fetch_proxy_mode", mode);
        let label = match mode {
            "direct" => "直连（不使用代理）",
            "custom" => "自定义代理",
            _ => "跟随系统代理",
        };
        self.show_toast(format!("网络抓取代理已切换为：{label}"), cx);
        cx.notify();
    }

    pub fn save_fetch_proxy_url(&mut self, cx: &mut Context<Self>) {
        let raw = self.p1.fetch_proxy_url.read(cx).value().to_string();
        match validate_proxy_url(&raw) {
            Ok(valid) => {
                self.p1.fetch_proxy_url_value = valid.clone();
                self.p1.fetch_proxy_mode = "custom".to_string();
                {
                    let store = lock_store(&self.store);
                    let _ = store.set_setting("fetch_proxy_url", &valid);
                    let _ = store.set_setting("fetch_proxy_mode", "custom");
                }
                self.show_toast(format!("已启用自定义抓取代理：{valid}"), cx);
            }
            Err(error) => self.show_toast(error, cx),
        }
        cx.notify();
    }

    pub fn apply_server_fixed_port(&mut self, cx: &mut Context<Self>) {
        let raw = self
            .p1
            .server_fixed_port_input
            .read(cx)
            .value()
            .trim()
            .to_string();
        let next_port = if raw.is_empty() {
            None
        } else {
            match raw.parse::<u16>().ok().filter(|&p| p > 0) {
                Some(port) => Some(port),
                None => {
                    self.show_toast("端口须为 1–65535 的整数（留空为自动选择）".into(), cx);
                    return;
                }
            }
        };
        let previous = self.p1.server_fixed_port;
        self.p1.server_fixed_port = next_port;
        if let Some(mut server) = self.server.take() {
            server.stop();
        }
        self.start_server();
        if self.server.is_none() {
            let err = self
                .server_error
                .clone()
                .unwrap_or_else(|| "端口已被其他程序占用".into());
            self.p1.server_fixed_port = previous;
            self.start_server();
            self.show_toast(format!("绑定固定端口失败（已恢复原端口）：{err}"), cx);
            cx.notify();
            return;
        }
        let persisted = next_port.map(|p| p.to_string()).unwrap_or_default();
        let _ = lock_store(&self.store).set_setting("server_fixed_port", &persisted);
        let bound = self
            .server
            .as_ref()
            .map(|s| s.local_addr().port())
            .unwrap_or(17899);
        let message = if let Some(port) = next_port {
            format!("订阅服务已固定监听端口 {port}")
        } else {
            format!("已切换为自动选择端口（当前 {bound}）")
        };
        self.show_toast(message, cx);
        cx.notify();
    }

    pub fn set_server_lan_private_only(&mut self, enabled: bool, cx: &mut Context<Self>) {
        if self.p1.server_lan_private_only == enabled {
            return;
        }
        self.p1.server_lan_private_only = enabled;
        let _ = lock_store(&self.store).set_setting(
            "server_lan_private_only",
            if enabled { "true" } else { "false" },
        );
        if self.server_bind_lan {
            if let Some(mut server) = self.server.take() {
                server.stop();
            }
            self.start_server();
        }
        self.show_toast(
            if enabled {
                "局域网订阅已限制为仅私有网段（RFC1918 / Tailscale）可访问".into()
            } else {
                "已允许任意来源访问局域网监听端口（请仅在受信任网络下使用）".into()
            },
            cx,
        );
        cx.notify();
    }

    pub fn copy_lan_firewall_command(&mut self, cx: &mut Context<Self>) {
        let port = self
            .server
            .as_ref()
            .map(|s| s.local_addr().port())
            .or(self.p1.server_fixed_port)
            .unwrap_or(17899);
        let cmd = crate::desktop::lan_firewall_command(port);
        cx.write_to_clipboard(ClipboardItem::new_string(cmd));
        self.show_toast(
            format!("已复制防火墙放行命令（端口 {port}），请以管理员身份在终端运行"),
            cx,
        );
    }

    pub fn rotate_saved_config_token(
        &mut self,
        id: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.saved_config_id.as_deref() == Some(id.as_str())
            && let Err(error) = self.autosave_saved_editor(window, cx)
        {
            self.show_toast(format!("请先修正未保存的编辑再重置地址：{error}"), cx);
            return;
        }
        let result = lock_store(&self.store)
            .rotate_saved_config_id(&self.data_dir.join("configs"), &id)
            .map_err(|error| error.to_string());
        match result {
            Ok(rotated) => {
                if self.saved_config_id.as_deref() == Some(id.as_str()) {
                    self.saved_config_id = Some(rotated.id.clone());
                }
                if self
                    .generated_saved_config
                    .as_ref()
                    .is_some_and(|config| config.id == id)
                {
                    self.generated_saved_config = Some(rotated.clone());
                }
                self.reload_saved_configs();
                self.show_toast(
                    format!("「{}」订阅地址已重置，旧链接已失效", rotated.name),
                    cx,
                );
            }
            Err(error) => self.show_toast(format!("重置订阅地址失败：{error}"), cx),
        }
        cx.notify();
    }

    pub fn open_qr_modal(
        &mut self,
        title: String,
        subtitle: String,
        content: String,
        sensitive: bool,
        cx: &mut Context<Self>,
    ) {
        match crate::qr::encode_qr(&content) {
            Ok(encoded) => {
                self.p1.qr_modal = Some(QrModal {
                    title,
                    subtitle,
                    content,
                    svg: encoded.svg.into_bytes(),
                    size: encoded.size,
                    runs: encoded.runs,
                    sensitive,
                });
                cx.notify();
            }
            Err(error) => self.show_toast(error, cx),
        }
    }

    pub fn open_config_lan_qr(&mut self, id: String, cx: &mut Context<Self>) {
        let name = self
            .saved_configs
            .iter()
            .find(|c| c.id == id)
            .map(|c| c.name.clone())
            .unwrap_or_else(|| "配置订阅".into());
        let Some(url) = self.config_lan_subscription_url(&id) else {
            self.show_toast("请先开启「局域网导入」并确保检测到局域网 IP".into(), cx);
            return;
        };
        self.open_qr_modal(
            format!("扫码订阅 · {name}"),
            "同局域网的手机 / 平板 Clash 客户端可直接扫码导入".into(),
            url,
            false,
            cx,
        );
    }

    pub fn open_node_share_qr(&mut self, source_key: String, cx: &mut Context<Self>) {
        let Some(node) = self.nodes.iter().find(|n| n.source_key == source_key) else {
            return;
        };
        let Some(link) = node.proxy.to_share_link() else {
            self.show_toast(
                format!("协议 {} 暂不支持导出标准分享链接", node.proxy.kind()),
                cx,
            );
            return;
        };
        let title = format!("节点分享 · {}", node.name());
        self.open_qr_modal(
            title,
            "包含节点连接凭据，请仅在可信设备间扫描或复制".into(),
            link,
            true,
            cx,
        );
    }

    pub fn export_qr_svg(&mut self, cx: &mut Context<Self>) {
        let Some(modal) = &self.p1.qr_modal else {
            return;
        };
        let filename = format!(
            "{}.svg",
            modal
                .title
                .replace(['/', '\\', ':', '*', '?', '"', '<', '>', '|', ' '], "_")
        );
        let Some(path) = rfd::FileDialog::new()
            .set_file_name(filename)
            .add_filter("SVG Image", &["svg"])
            .save_file()
        else {
            return;
        };
        match std::fs::write(&path, &modal.svg) {
            Ok(()) => self.show_toast("二维码 SVG 已导出".into(), cx),
            Err(error) => self.show_toast(format!("导出失败：{error}"), cx),
        }
    }

    pub fn set_auto_start(&mut self, enabled: bool, cx: &mut Context<Self>) {
        match crate::desktop::set_auto_start(enabled) {
            Ok(()) => {
                self.p1.auto_start = enabled;
                self.show_toast(
                    if enabled {
                        "已开启开机静默自启（启动后自动最小化到托盘）".into()
                    } else {
                        "已关闭开机自启".into()
                    },
                    cx,
                );
            }
            Err(error) => self.show_toast(error, cx),
        }
        cx.notify();
    }

    pub fn export_full_backup(&mut self, cx: &mut Context<Self>) {
        let timestamp = chrono::Local::now().format("%Y%m%d-%H%M%S");
        let Some(path) = rfd::FileDialog::new()
            .set_file_name(format!("swiftsub-backup-{timestamp}.db"))
            .add_filter("SwiftSub Backup", &["db"])
            .save_file()
        else {
            return;
        };
        if path.exists() {
            let _ = std::fs::remove_file(&path);
        }
        let path_str = path.to_string_lossy().to_string();
        let result = lock_store(&self.store)
            .backup_to(&path_str)
            .map_err(|error| error.to_string());
        match result {
            Ok(()) => self.show_toast("完整数据备份已导出（含订阅、配置、模板与历史）".into(), cx),
            Err(error) => self.show_toast(format!("导出备份失败：{error}"), cx),
        }
    }

    pub fn pick_backup_to_restore(&mut self, cx: &mut Context<Self>) {
        let Some(path) = rfd::FileDialog::new()
            .add_filter("SwiftSub Backup", &["db"])
            .pick_file()
        else {
            return;
        };
        let preview = (|| -> Result<BackupPreview, String> {
            let store = Store::open(&path).map_err(|e| format!("无法打开备份文件：{e}"))?;
            store
                .validate_database()
                .map_err(|e| format!("备份文件完整性校验失败：{e}"))?;
            let subscriptions = store.list_subscriptions().map_err(|e| e.to_string())?.len();
            let saved_configs = store.list_saved_configs().map_err(|e| e.to_string())?.len();
            let templates = store
                .list_custom_templates()
                .map_err(|e| e.to_string())?
                .len();
            Ok(BackupPreview {
                path,
                subscriptions,
                saved_configs,
                templates,
            })
        })();
        match preview {
            Ok(info) => {
                self.p1.backup_preview = Some(info);
                cx.notify();
            }
            Err(error) => self.show_toast(error, cx),
        }
    }

    pub fn confirm_backup_restore(&mut self, cx: &mut Context<Self>) {
        let Some(preview) = self.p1.backup_preview.take() else {
            return;
        };
        let target = self.data_dir.join("restore-pending.db");
        match std::fs::copy(&preview.path, &target) {
            Ok(_) => {
                self.p1.backup_restore_pending = true;
                self.show_toast(
                    "恢复任务已就绪，请退出并重新启动 SwiftSub 以生效（原数据自动保留为 .bak）"
                        .into(),
                    cx,
                );
            }
            Err(error) => self.show_toast(format!("写入待恢复文件失败：{error}"), cx),
        }
        cx.notify();
    }

    pub fn open_config_history(&mut self, config_id: String, cx: &mut Context<Self>) {
        let config_name = self
            .saved_configs
            .iter()
            .find(|c| c.id == config_id)
            .map(|c| c.name.clone())
            .unwrap_or_else(|| "配置".into());
        let history_result = lock_store(&self.store).config_history(&config_id);
        let revisions = match history_result {
            Ok(list) => list
                .into_iter()
                .map(|revision| {
                    let (proxies, groups, rules) = summarize_yaml_counts(&revision.yaml);
                    RevisionSummary {
                        revision,
                        proxies,
                        groups,
                        rules,
                    }
                })
                .collect(),
            Err(error) => {
                self.show_toast(format!("读取历史版本失败：{error}"), cx);
                return;
            }
        };
        self.p1.history_modal = Some(HistoryModal {
            config_id,
            config_name,
            revisions,
        });
        cx.notify();
    }

    pub fn restore_config_revision(
        &mut self,
        revision_id: i64,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some((config_id, revision)) = self.p1.history_modal.as_ref().and_then(|modal| {
            modal
                .revisions
                .iter()
                .find(|item| item.revision.id == revision_id)
                .map(|item| (modal.config_id.clone(), item.revision.clone()))
        }) else {
            return;
        };
        let result = lock_store(&self.store)
            .save_config_batch(
                &self.data_dir.join("configs"),
                &[SavedConfigWrite {
                    id: Some(&config_id),
                    name: &revision.name,
                    yaml: &revision.yaml,
                    source_json: revision.source_json.as_deref(),
                    reason: "恢复历史版本",
                    file_baseline: None,
                }],
            )
            .map_err(|error| error.to_string());
        match result {
            Ok(_) => {
                {
                    let store = lock_store(&self.store);
                    for prefix in ["visual-meta", "visual-form", "yaml-editor"] {
                        let _ = store.set_setting(&format!("{prefix}:{config_id}"), "");
                    }
                }
                self.p1.history_modal = None;
                let was_open = self.saved_config_id.as_deref() == Some(config_id.as_str());
                self.reload_saved_configs();
                if was_open {
                    self.saved_config_id = None;
                    self.open_saved_config(config_id, window, cx);
                }
                self.show_toast("已恢复所选历史版本（原订阅地址保持不变）".into(), cx);
            }
            Err(error) => self.show_toast(format!("恢复历史版本失败：{error}"), cx),
        }
        cx.notify();
    }

    fn filtered_workspace_node_keys(&self, cx: &Context<Self>) -> Vec<String> {
        let query = self.search.read(cx).value().trim().to_lowercase();
        self.nodes
            .iter()
            .filter(|node| {
                query.is_empty()
                    || node.name().to_lowercase().contains(&query)
                    || node.proxy.kind().to_lowercase().contains(&query)
                    || node.source_name.to_lowercase().contains(&query)
            })
            .map(|node| node.source_key.clone())
            .collect()
    }

    /// Batch enable (`Some(true)`), disable (`Some(false)`), or invert (`None`) filtered nodes.
    pub fn batch_set_filtered_nodes_enabled(
        &mut self,
        target: Option<bool>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let keys: HashSet<String> = self.filtered_workspace_node_keys(cx).into_iter().collect();
        if keys.is_empty() {
            self.show_toast("当前没有匹配的节点".into(), cx);
            return;
        }
        let snapshot = match self.draft_template_snapshot(cx) {
            Ok(snapshot) => snapshot,
            Err(error) => {
                self.show_toast(format!("批量操作失败：{error}"), cx);
                return;
            }
        };
        let previous_nodes = self.nodes.clone();
        let previous_disabled = self.disabled_nodes.clone();
        let mut changed = Vec::new();
        for node in &mut self.nodes {
            if !keys.contains(&node.source_key) {
                continue;
            }
            let next = target.unwrap_or(!node.enabled);
            if node.enabled != next {
                node.enabled = next;
                if next {
                    self.disabled_nodes.remove(&node.source_key);
                } else {
                    self.disabled_nodes.insert(node.source_key.clone());
                }
                changed.push(node.clone());
            }
        }
        if changed.is_empty() {
            self.show_toast("所选节点状态无需变更".into(), cx);
            return;
        }
        let overrides = match changed
            .iter()
            .map(|node| self.workspace_node_override(node))
            .collect::<Result<Vec<_>, _>>()
        {
            Ok(list) => list,
            Err(error) => {
                self.nodes = previous_nodes;
                self.disabled_nodes = previous_disabled;
                self.show_toast(format!("批量操作失败：{error}"), cx);
                return;
            }
        };
        let count = changed.len();
        if let Err(error) = self.persist_template_with_node_overrides(
            snapshot.clone(),
            None,
            &overrides,
            window,
            cx,
        ) {
            self.nodes = previous_nodes;
            self.disabled_nodes = previous_disabled;
            self.show_toast(format!("批量保存失败：{error}"), cx);
            cx.notify();
            return;
        }
        self.sync_workspace_snapshot(snapshot, window, cx);
        let action = match target {
            Some(true) => "启用",
            Some(false) => "停用",
            None => "反转",
        };
        self.show_toast(format!("已批量{action} {count} 个节点"), cx);
        cx.notify();
    }

    pub fn open_batch_rename(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.p1.batch_rename_error = None;
        self.p1
            .batch_rename_prefix
            .update(cx, |s, cx| s.set_value("", window, cx));
        self.p1
            .batch_rename_find
            .update(cx, |s, cx| s.set_value("", window, cx));
        self.p1
            .batch_rename_replace
            .update(cx, |s, cx| s.set_value("", window, cx));
        self.p1.batch_rename_open = true;
        cx.notify();
    }

    pub fn apply_batch_rename(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let prefix = self.p1.batch_rename_prefix.read(cx).value().to_string();
        let find = self.p1.batch_rename_find.read(cx).value().to_string();
        let replace = self.p1.batch_rename_replace.read(cx).value().to_string();
        if prefix.is_empty() && find.is_empty() {
            self.p1.batch_rename_error = Some("请填写统一前缀或查找文本".into());
            cx.notify();
            return;
        }
        let keys: HashSet<String> = self.filtered_workspace_node_keys(cx).into_iter().collect();
        if keys.is_empty() {
            self.p1.batch_rename_error = Some("当前没有匹配的节点".into());
            cx.notify();
            return;
        }
        let mut snapshot = match self.draft_template_snapshot(cx) {
            Ok(snapshot) => snapshot,
            Err(error) => {
                self.p1.batch_rename_error = Some(error);
                cx.notify();
                return;
            }
        };
        let previous_nodes = self.nodes.clone();
        let previous_visibility = self.workspace.rule_visibility.clone();
        let mut used_names: HashSet<String> = self
            .nodes
            .iter()
            .filter(|n| !keys.contains(&n.source_key))
            .map(|n| n.name().to_string())
            .collect();
        let mut changed = Vec::new();
        for node in &mut self.nodes {
            if !keys.contains(&node.source_key) {
                continue;
            }
            let old = node.name().to_string();
            let mut next = if find.is_empty() {
                old.clone()
            } else {
                old.replace(&find, &replace)
            };
            if !prefix.is_empty() && !next.starts_with(&prefix) {
                next = format!("{prefix}{next}");
            }
            let next = next.trim().to_string();
            if next.is_empty() {
                self.nodes = previous_nodes;
                self.workspace.rule_visibility = previous_visibility;
                self.p1.batch_rename_error = Some(format!("节点「{old}」替换后名称为空"));
                cx.notify();
                return;
            }
            let mut unique = next.clone();
            let mut suffix = 2;
            while !used_names.insert(unique.clone()) {
                unique = format!("{next} #{suffix}");
                suffix += 1;
            }
            if unique != old {
                node.set_name(unique.clone());
                node.region = detect_region(&unique);
                super::layout::retarget_snapshot(&mut snapshot, &old, &unique);
                workspace::retarget_rule_lines(
                    &mut self.workspace.rule_visibility.disabled_manual,
                    &old,
                    &unique,
                );
                changed.push(node.clone());
            }
        }
        if changed.is_empty() {
            self.p1.batch_rename_open = false;
            self.show_toast("没有节点名称发生变化".into(), cx);
            cx.notify();
            return;
        }
        let overrides = match changed
            .iter()
            .map(|node| self.workspace_node_override(node))
            .collect::<Result<Vec<_>, _>>()
        {
            Ok(list) => list,
            Err(error) => {
                self.nodes = previous_nodes;
                self.workspace.rule_visibility = previous_visibility;
                self.p1.batch_rename_error = Some(error);
                cx.notify();
                return;
            }
        };
        let count = changed.len();
        if let Err(error) = self.persist_template_with_node_overrides(
            snapshot.clone(),
            None,
            &overrides,
            window,
            cx,
        ) {
            self.nodes = previous_nodes;
            self.workspace.rule_visibility = previous_visibility;
            self.p1.batch_rename_error = Some(error);
            cx.notify();
            return;
        }
        self.sync_workspace_snapshot(snapshot, window, cx);
        self.p1.batch_rename_open = false;
        self.p1.batch_rename_error = None;
        self.show_toast(format!("已批量重命名 {count} 个节点"), cx);
        cx.notify();
    }

    pub fn delete_subscription_source(&mut self, sub_id: i64, cx: &mut Context<Self>) {
        let Some(sub) = self
            .subscriptions
            .iter()
            .find(|item| item.id == sub_id)
            .cloned()
        else {
            return;
        };
        let other_sub_keys: HashSet<String> = self
            .subscriptions
            .iter()
            .filter(|other| other.id != sub_id)
            .filter_map(|other| {
                other
                    .payload
                    .as_deref()
                    .and_then(|text| self.parsed_sources.parse(other.id, text).ok())
            })
            .flat_map(|other| other.proxies.into_iter().map(|p| p.identity_key()))
            .collect();
        let keys_to_clean: Vec<String> = sub
            .payload
            .as_deref()
            .and_then(|payload| self.parsed_sources.parse(sub_id, payload).ok())
            .map(|parsed| {
                parsed
                    .proxies
                    .iter()
                    .map(Proxy::identity_key)
                    .filter(|key| !other_sub_keys.contains(key))
                    .collect()
            })
            .unwrap_or_default();
        let delete_result = lock_store(&self.store)
            .delete_subscription(sub_id)
            .map_err(|error| error.to_string());
        if let Err(error) = delete_result {
            self.show_toast(format!("删除订阅失败：{error}"), cx);
            return;
        }
        self.restore_deleted_node_keys(&keys_to_clean);
        for key in &keys_to_clean {
            self.disabled_nodes.remove(key);
            self.node_order.retain(|item| item != key);
        }
        if let Some(ids) = &mut self.profile_membership {
            ids.remove(&sub_id);
        }
        self.failure_counts.remove(&sub_id);
        self.retry_after.remove(&sub_id);
        self.refresh_versions.remove(&sub_id);
        self.reload_subscriptions(cx);
        self.show_toast(format!("已删除订阅「{}」", sub.name), cx);
        cx.notify();
    }

    pub fn open_sub_logs(&mut self, cx: &mut Context<Self>) {
        self.p1.refresh_logs = lock_store(&self.store)
            .recent_refresh_logs(40)
            .unwrap_or_default();
        self.p1.sub_logs_open = true;
        cx.notify();
    }

    pub fn set_theme_mode(
        &mut self,
        mode: crate::ui::AppThemeMode,
        window: &Window,
        cx: &mut Context<Self>,
    ) {
        self.p1.theme_mode = mode;
        let _ = lock_store(&self.store).set_setting("theme", mode.as_str());
        let system_is_light = matches!(
            window.appearance(),
            WindowAppearance::Light | WindowAppearance::VibrantLight
        );
        let light = mode.resolve_light(system_is_light);
        self.light_theme = light;
        Theme::change(
            if light {
                ThemeMode::Light
            } else {
                ThemeMode::Dark
            },
            None,
            cx,
        );
        crate::configure_theme(cx, light);
        crate::ui::set_light_theme(light);
        cx.set_window_appearance(Some(if light {
            WindowAppearance::Light
        } else {
            WindowAppearance::Dark
        }));
        cx.notify();
    }

    /// Export active workspace nodes (or the currently opened saved config) to sing-box JSON.
    pub fn export_singbox_json_file(&mut self, cx: &mut Context<Self>) {
        let (nodes, suggested_name) =
            if self.page == Page::SavedConfigs || self.workspace.saved_editor_open {
                let yaml = self.saved_config_yaml.read(cx).value().to_string();
                let parsed = match parse_content(&yaml) {
                    Ok(parsed) => parsed,
                    Err(error) => {
                        self.show_toast(format!("无法提取节点导出 sing-box：{error}"), cx);
                        return;
                    }
                };
                let name = self.saved_config_name.read(cx).value().trim().to_string();
                (
                    build_nodes(parsed.proxies, "saved", "saved"),
                    if name.is_empty() {
                        "sing-box".to_string()
                    } else {
                        name
                    },
                )
            } else {
                (self.nodes.clone(), "swiftsub-singbox".to_string())
            };
        let json = match swiftsub_core::generator::generate_singbox_json(&nodes, &self.options) {
            Ok(json) => json,
            Err(error) => {
                self.show_toast(format!("导出 sing-box 失败：{error}"), cx);
                return;
            }
        };
        let safe_name = suggested_name.replace(['/', '\\', ':', '*', '?', '"', '<', '>', '|'], "_");
        let Some(path) = rfd::FileDialog::new()
            .set_file_name(format!("{safe_name}.json"))
            .add_filter("sing-box JSON", &["json"])
            .save_file()
        else {
            return;
        };
        match std::fs::write(&path, json) {
            Ok(()) => self.show_toast("sing-box JSON 配置已导出".into(), cx),
            Err(error) => self.show_toast(format!("保存文件失败：{error}"), cx),
        }
    }

    pub fn pick_mihomo_binary(&mut self, cx: &mut Context<Self>) {
        let Some(path) = rfd::FileDialog::new()
            .add_filter("Mihomo Core", &["exe"])
            .pick_file()
        else {
            return;
        };
        let path_str = path.display().to_string();
        let _ = lock_store(&self.store).set_setting("mihomo_binary_path", &path_str);
        self.p1.mihomo_binary_path = Some(path);
        self.show_toast(format!("已配置本地 Mihomo 内核：{path_str}"), cx);
        cx.notify();
    }

    pub fn verify_saved_with_mihomo(&mut self, cx: &mut Context<Self>) {
        if self.p1.mihomo_checking {
            return;
        }
        let binary = match &self.p1.mihomo_binary_path {
            Some(path) if path.is_file() => path.clone(),
            _ => match crate::desktop::detect_mihomo_binary() {
                Some(path) => {
                    self.p1.mihomo_binary_path = Some(path.clone());
                    path
                }
                None => {
                    self.show_toast(
                        "未自动检测到本地 Mihomo / Clash Verge 内核，请在「设置 → 应用与数据」指定 mihomo.exe"
                            .into(),
                        cx,
                    );
                    return;
                }
            },
        };
        let yaml = self.saved_config_yaml.read(cx).value().to_string();
        if yaml.trim().is_empty() {
            self.show_toast("当前没有可校验的 YAML 内容".into(), cx);
            return;
        }
        self.p1.mihomo_checking = true;
        cx.notify();
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move { crate::desktop::run_mihomo_check(&binary, &yaml) })
                .await;
            let Some(this) = this.upgrade() else {
                return;
            };
            this.update(cx, |this, cx| {
                this.p1.mihomo_checking = false;
                match result {
                    Ok(msg) => this.show_toast(format!("内核校验通过：{msg}"), cx),
                    Err(err) => this.show_toast(err, cx),
                }
                cx.notify();
            });
        })
        .detach();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validates_fetch_proxy_urls() {
        assert!(validate_proxy_url("http://127.0.0.1:7897").is_ok());
        assert!(validate_proxy_url("socks5://127.0.0.1:7897").is_ok());
        assert!(validate_proxy_url("socks5h://127.0.0.1:1080").is_ok());
        assert!(validate_proxy_url("").is_err());
        assert!(validate_proxy_url("ftp://example.com:21").is_err());
        assert!(validate_proxy_url("not-a-url").is_err());
    }

    #[test]
    fn applies_pending_backup_restore_and_preserves_bak() {
        let root = std::env::temp_dir().join(format!(
            "swiftsub-restore-{}",
            uuid::Uuid::new_v4().simple()
        ));
        std::fs::create_dir_all(&root).unwrap();
        let original = Store::open(root.join("swiftsub.db")).unwrap();
        original.set_setting("marker", "old").unwrap();
        drop(original);

        let backup = Store::open(root.join("restore-pending.db")).unwrap();
        backup.set_setting("marker", "restored").unwrap();
        drop(backup);

        let notice = apply_pending_backup_restore(&root);
        assert!(notice.is_some());
        assert!(!root.join("restore-pending.db").exists());
        assert!(root.join("swiftsub.db.bak").exists());

        let active = Store::open(root.join("swiftsub.db")).unwrap();
        assert_eq!(
            active.get_setting("marker").unwrap().as_deref(),
            Some("restored")
        );
        drop(active);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn formats_traffic_bytes_readably() {
        assert_eq!(format_bytes_human(512), "512 B");
        assert_eq!(format_bytes_human(2048), "2 KB");
        assert_eq!(format_bytes_human(5 * 1024 * 1024), "5.0 MB");
        assert_eq!(format_bytes_human(10 * 1024 * 1024 * 1024), "10.00 GB");
    }
}
