mod generation;
mod layout;
mod lifecycle;
pub(crate) mod ordering;
pub(crate) use ordering::{WorkspaceNodeKind, chain_order_key};
pub mod phase2;
mod profiles;
mod publication;
mod refresh;
mod saved_configs;
mod saved_editor;
pub use saved_editor::{SavedEditor, SavedEditorTab};
pub mod upgrades;
pub mod workspace;
use std::collections::{HashMap, HashSet, VecDeque};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use chrono::{Local, TimeZone};
use gpui_kit::component::input::{InputEvent, InputState, TextareaState};
use gpui_kit::component::{Theme, ThemeMode};
use gpui_kit::{
    AppContext as _, ClipboardItem, Context, Entity, Focusable, ScrollHandle, Window,
    WindowAppearance, point, px,
};
use serde::{Deserialize, Serialize};
use swiftsub_core::catalog::{self, Preset};
use swiftsub_core::generator::RuleSetBinding;
use swiftsub_core::model::{
    ChainSpec, CustomGroupSpec, GroupKind, Node, NodeFilter, PolicyGroupSpec, Proxy, Rule,
};
use swiftsub_core::normalize::{build_nodes, dedupe_proxies, proxies_to_clash_yaml};
use swiftsub_core::parser::{looks_like_clash_yaml, parse_content};
use swiftsub_core::region::{Region, detect_region};
use swiftsub_core::{GenerateOptions, ProfileStats, generate_profile};
use swiftsub_fetch::{
    CacheValidators, FetchOptions, FetchOutcome, SubscriptionInfo, fetch_subscription,
};
use swiftsub_server::{LocalServer, ProfileSnapshot, ServerConfig};
use swiftsub_store::{
    CustomTemplate, NewSubscription, Store, Subscription as StoredSubscription, TrafficSnapshot,
};

use crate::rule_catalog;

pub(crate) fn lock_store(store: &Mutex<Store>) -> std::sync::MutexGuard<'_, Store> {
    store.lock().unwrap_or_else(|poison| poison.into_inner())
}

fn default_test_tolerance() -> u32 {
    50
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Page {
    Subscriptions,
    Generator,
    SavedConfigs,
    Templates,
    Settings,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SettingsTab {
    Generation,
    RuleSources,
    Templates,
    Data,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TemplateTab {
    PolicyGroups,
    Tools,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct TemplateSnapshot {
    preset: Preset,
    enabled_modules: Vec<String>,
    #[serde(default)]
    disabled_rule_sets: Vec<String>,
    #[serde(default)]
    custom_rule_sets: Vec<RuleSetBinding>,
    #[serde(default)]
    custom_rule_lines: Vec<String>,
    #[serde(default)]
    rule_order: Vec<String>,
    #[serde(default)]
    dns: swiftsub_core::generator::DnsOptions,
    #[serde(default)]
    dns_override: bool,
    #[serde(default)]
    rule_set_overrides: HashMap<String, Vec<String>>,
    #[serde(default)]
    listeners: Vec<swiftsub_core::generator::ListenerSpec>,
    #[serde(default)]
    extra_yaml: String,
    #[serde(default)]
    rule_provider_base_url: String,
    #[serde(default)]
    policy_groups: Vec<PolicyGroupSpec>,
    #[serde(default)]
    group_order: Vec<String>,
    #[serde(default)]
    rule_set_targets: HashMap<String, String>,
    #[serde(default)]
    custom_groups: Vec<CustomGroupSpec>,
    #[serde(default)]
    chains: Vec<ChainSpec>,
    test_url: String,
    test_interval: u32,
    #[serde(default = "default_test_tolerance")]
    test_tolerance: u32,
    mixed_port: u16,
    allow_lan: bool,
}

impl TemplateSnapshot {
    fn generate_options(&self, base: &GenerateOptions) -> Result<GenerateOptions, String> {
        let mut options = base.clone();
        options.preset = self.preset;
        options.enabled_modules = Some(self.enabled_modules.clone());
        options.disabled_rule_sets = self.disabled_rule_sets.clone();
        options.custom_rule_sets = self.custom_rule_sets.clone();
        options.custom_rules = parse_custom_rule_lines(&self.custom_rule_lines.join("\n"))?;
        options.custom_groups = self.custom_groups.clone();
        options.chains = self.chains.clone();
        options.policy_groups = self.policy_groups.clone();
        options.group_order = self.group_order.clone();
        options.rule_order = self.rule_order.clone();
        options.dns = self.dns.clone();
        options.dns_override = self.dns_override;
        options.rule_set_overrides = self.rule_set_overrides.clone();
        options.listeners = self.listeners.clone();
        options.extra_yaml = self.extra_yaml.clone();
        options.rule_set_targets = self.rule_set_targets.clone();
        options.test_url = self.test_url.clone();
        options.test_interval = self.test_interval;
        options.test_tolerance = self.test_tolerance;
        options.mixed_port = self.mixed_port;
        options.allow_lan = self.allow_lan;
        if !self.rule_provider_base_url.is_empty() {
            options.rule_provider_base_url = self.rule_provider_base_url.clone();
        }
        options.remove_legacy_region_groups();
        Ok(options)
    }

    fn remove_legacy_regions(&mut self) {
        if let Ok(options) = self.generate_options(&GenerateOptions::default()) {
            self.enabled_modules = options.enabled_modules.unwrap_or_default();
            self.policy_groups = options.policy_groups;
            self.custom_rule_sets = options.custom_rule_sets;
            self.custom_rule_lines = options.custom_rules.iter().map(Rule::to_line).collect();
            self.rule_set_targets = options.rule_set_targets;
            self.listeners = options.listeners;
        }
    }
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct AdvancedDraft {
    dns: swiftsub_core::generator::DnsOptions,
    #[serde(default)]
    listeners: Vec<swiftsub_core::generator::ListenerSpec>,
    #[serde(default)]
    chains: Vec<ChainSpec>,
    #[serde(default)]
    custom_groups: Vec<CustomGroupSpec>,
    #[serde(default)]
    extra_yaml: String,
}

#[cfg(test)]
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct TemplateFile {
    format: String,
    version: u32,
    name: String,
    config: TemplateSnapshot,
}

#[cfg(test)]
impl TemplateFile {
    fn decode(text: &str) -> Result<Self, String> {
        let file: Self = serde_json::from_str(text).map_err(|e| format!("模板文件无效：{e}"))?;
        if file.format != "swiftsub-routing-template" || file.version != 1 {
            return Err(format!(
                "不支持的模板格式 / 版本：{} v{}",
                file.format, file.version
            ));
        }
        if file.name.trim().is_empty() {
            return Err("模板名称不能为空".into());
        }
        Ok(file)
    }
}

pub(crate) fn rule_set_binding_from_url(url: &str) -> Result<RuleSetBinding, String> {
    let url = url.trim();
    if !(url.starts_with("http://") || url.starts_with("https://")) {
        return Err("规则集链接必须以 http:// 或 https:// 开头".to_string());
    }
    if url.chars().any(char::is_whitespace) {
        return Err("规则集链接不能包含空格".to_string());
    }
    let path = url.split(['?', '#']).next().unwrap_or(url);
    let filename = path.rsplit('/').next().unwrap_or_default();
    let (stem, extension) = filename
        .rsplit_once('.')
        .map(|(stem, extension)| (stem, extension.to_lowercase()))
        .unwrap_or((filename, String::new()));
    let format = match extension.as_str() {
        "mrs" => "mrs",
        "yaml" | "yml" => "yaml",
        "txt" | "list" => "text",
        _ => return Err("仅支持 .mrs、.yaml、.yml、.txt 或 .list 规则集".to_string()),
    };
    let behavior = if format == "mrs" {
        if url.to_lowercase().contains("/geoip/") {
            "ipcidr"
        } else {
            "domain"
        }
    } else {
        "classical"
    };
    let mut id: String = stem
        .to_lowercase()
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || matches!(character, '-' | '_' | '.') {
                character
            } else {
                '-'
            }
        })
        .collect();
    while id.starts_with('-') || id.starts_with('.') {
        id.remove(0);
    }
    if id.is_empty() {
        id = "ruleset".to_string();
    }
    let name = if stem.is_empty() {
        "自定义规则集".to_string()
    } else {
        stem.to_string()
    };
    Ok(RuleSetBinding {
        id: format!("url-{id}"),
        name,
        url: url.to_string(),
        behavior: behavior.to_string(),
        format: format.to_string(),
        target: rule_catalog::SELECT_GROUP.to_string(),
        no_resolve: false,
    })
}

pub(crate) fn parse_custom_rule_lines(text: &str) -> Result<Vec<Rule>, String> {
    let mut rules = Vec::new();
    for (index, line) in text.lines().enumerate() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        match Rule::parse(line) {
            Ok(rule) => rules.push(rule),
            Err(error) => {
                return Err(format!(
                    "自定义规则第 {} 行格式无效：{line}。{error}",
                    index + 1
                ));
            }
        }
    }
    Ok(rules)
}

pub(crate) fn template_summary_counts(config_json: &str) -> (usize, usize, usize) {
    let Ok(snapshot) = serde_json::from_str::<TemplateSnapshot>(config_json) else {
        return (0, 0, 0);
    };
    let disabled: HashSet<&str> = snapshot
        .disabled_rule_sets
        .iter()
        .map(String::as_str)
        .collect();
    let custom_ids: HashSet<&str> = snapshot
        .custom_rule_sets
        .iter()
        .map(|b| b.id.as_str())
        .collect();
    let builtin_rule_sets = snapshot
        .enabled_modules
        .iter()
        .filter_map(|id| catalog::module(id))
        .filter(|m| !matches!(m.kind, catalog::ModuleKind::Region(_)))
        .flat_map(|module| module.rule_sets.iter())
        .filter(|spec| !disabled.contains(spec.id) && !custom_ids.contains(spec.id))
        .count();
    let custom_rule_sets = snapshot
        .custom_rule_sets
        .iter()
        .filter(|b| !disabled.contains(b.id.as_str()))
        .count();
    let active_modules = snapshot
        .enabled_modules
        .iter()
        .filter_map(|id| catalog::module(id))
        .filter(|m| !matches!(m.kind, catalog::ModuleKind::Region(_)))
        .count();
    let custom_policy_groups = snapshot
        .policy_groups
        .iter()
        .filter(|g| !g.builtin && !g.removed && !g.disabled)
        .count();
    let custom_groups = snapshot
        .custom_groups
        .iter()
        .filter(|g| !g.disabled)
        .count();
    let chains = snapshot.chains.iter().filter(|c| !c.disabled).count();
    let manual = snapshot
        .custom_rule_lines
        .iter()
        .filter(|line| {
            let trimmed = line.trim();
            !trimmed.is_empty() && !trimmed.starts_with('#')
        })
        .count();
    (
        active_modules + custom_policy_groups + custom_groups + chains,
        builtin_rule_sets + custom_rule_sets,
        manual,
    )
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CustomDialog {
    None,
    Filter,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ChainSide {
    Entry,
    Landing,
}

pub struct SwiftSubApp {
    pub workspace: workspace::Workspace,
    pub p1: phase2::Phase2State,
    pub upgrades: upgrades::Upgrades,
    pub desktop: crate::desktop::DesktopIntegration,
    pub close_to_tray: bool,
    pub recovered_draft: bool,
    pub(super) last_draft_checkpoint: std::cell::RefCell<Option<(String, String)>>,
    pub page: Page,
    pub light_theme: bool,
    pub store: Arc<Mutex<Store>>,
    pub store_error: Option<String>,
    pub data_dir: PathBuf,
    pub subscriptions: Vec<StoredSubscription>,
    pub nodes: Vec<Node>,
    pub node_order: Vec<String>,
    pub parse_warnings: Vec<String>,
    pub import_report: Vec<swiftsub_core::parser::ImportReportItem>,
    pub source_import_report: Vec<(String, swiftsub_core::parser::ImportReportItem)>,
    pub generation_report: String,
    pub search: Entity<InputState>,
    pub add_open: bool,
    pub add_name: Entity<InputState>,
    pub add_url: Entity<TextareaState>,
    pub add_error: Option<String>,
    pub custom_dialog: CustomDialog,
    pub filter_name: Entity<InputState>,
    pub filter_include: Entity<InputState>,
    pub filter_exclude: Entity<InputState>,
    pub chain_name: Entity<InputState>,
    pub chain_search: Entity<InputState>,
    pub filter_regions: HashSet<Region>,
    pub chain_entry_key: Option<String>,
    pub chain_entry_name: Option<String>,
    pub chain_landing_key: Option<String>,
    pub chain_landing_name: Option<String>,
    pub custom_error: Option<String>,
    pub custom_groups: Vec<CustomGroupSpec>,
    pub chains: Vec<ChainSpec>,
    pub custom_templates: Vec<CustomTemplate>,
    pub saved_configs: Vec<swiftsub_store::SavedConfig>,
    pub saved_config_id: Option<String>,
    pub saved_config_name: Entity<InputState>,
    pub saved_editor: SavedEditor,
    pub generated_config_name: Entity<InputState>,
    pub generated_save_open: bool,
    pub generated_save_update: bool,
    pub generated_save_error: Option<String>,
    pub clash_import_busy: bool,
    pub saved_config_yaml: Entity<TextareaState>,
    pub saved_config_error: Option<String>,
    pub saved_file_baseline: Option<String>,
    pub saved_external_change: bool,
    pub saved_draft_corrupt: bool,
    pub saved_config_delete_armed: bool,
    pub generated_saved_config: Option<swiftsub_store::SavedConfig>,
    generation_cache: generation::GenerationCache,
    pub template_name: Entity<InputState>,
    pub template_error: Option<String>,
    pub template_dialog_open: bool,
    pub template_edit_id: Option<i64>,
    pub template_session_active: bool,
    template_advanced_saved: String,
    template_baseline: Option<TemplateSnapshot>,
    pub(crate) template_base_options: Option<GenerateOptions>,
    pub refreshing: HashSet<i64>,
    refresh_queue: VecDeque<refresh::FetchJob>,
    refresh_versions: HashMap<i64, u64>,
    refresh_reload_pending: bool,
    parsed_sources: generation::ParseCache,
    publication: publication::PublicationWorker,
    pub failure_counts: HashMap<i64, u32>,
    pub retry_after: HashMap<i64, i64>,
    pub options: GenerateOptions,
    pub subscription_filename: String,
    pub subscription_filename_input: Entity<InputState>,
    pub rule_provider_input: Entity<InputState>,
    pub enabled_modules: HashSet<String>,
    pub disabled_rule_sets: HashSet<String>,
    pub custom_rule_sets: Vec<RuleSetBinding>,
    pub template_disabled_rule_sets: HashSet<String>,
    pub template_custom_rule_sets: Vec<RuleSetBinding>,
    pub template_enabled_modules: HashSet<String>,
    pub template_policy_groups: Vec<PolicyGroupSpec>,
    pub template_group_order: Vec<String>,
    pub template_rule_set_targets: HashMap<String, String>,
    pub template_policy_name: Entity<InputState>,
    pub template_policy_rename: Entity<InputState>,
    pub template_health_url: Entity<InputState>,
    pub template_health_interval: Entity<InputState>,
    pub template_member_search: Entity<InputState>,
    pub template_rule_order: Vec<String>,
    pub template_policy_edit: Option<usize>,
    pub template_rules_target_filter: String,
    pub template_rule_search: Entity<InputState>,
    pub template_rule_url: Entity<InputState>,
    pub template_custom_rules: Entity<TextareaState>,
    pub advanced_draft: Entity<TextareaState>,
    pub template_tab: TemplateTab,
    pub settings_tab: SettingsTab,
    pub template_rule_sources_collapsed: HashSet<String>,
    pub profiles: Vec<swiftsub_store::Profile>,
    pub profile_name_input: Entity<InputState>,
    pub profile_membership: Option<HashSet<i64>>,
    pub profile_id: i64,
    pub profile_token: String,
    pub server: Option<LocalServer>,
    pub server_bind_lan: bool,
    pub server_lan_host: Option<String>,
    pub lan_interfaces: Vec<swiftsub_server::LanInterface>,
    pub lan_interfaces_refreshing: bool,
    pub server_url: Option<String>,
    pub server_error: Option<String>,
    pub update_repo: String,
    pub update_checking: bool,
    pub update_result: Option<swiftsub_fetch::ReleaseInfo>,
    pub update_error: Option<String>,
    pub preview: String,
    pub node_rename_key: Option<String>,
    pub node_rename_sub_id: Option<i64>,
    pub node_rename_input: Entity<InputState>,
    pub node_edit_textarea: Entity<TextareaState>,
    pub node_rename_error: Option<String>,
    pub stats: ProfileStats,
    pub gen_warnings: Vec<String>,
    pub toast: Option<String>,
    pub toast_epoch: u64,
    pub disabled_nodes: HashSet<String>,
    pub deleted_nodes: HashSet<String>,
    pub preview_scroll: ScrollHandle,
    pub _subscriptions: Vec<gpui_kit::Subscription>,
}

impl SwiftSubApp {
    pub fn new(
        window: &mut Window,
        instance: Option<crate::desktop::SingleInstanceGuard>,
        cx: &mut Context<Self>,
    ) -> Self {
        let data_dir = data_dir();
        let restore_notice = phase2::apply_pending_backup_restore(&data_dir);
        let db_path = data_dir.join("swiftsub.db");
        let (store, store_error) = match Store::open(&db_path) {
            Ok(store) => (store, None),
            Err(error) => (
                Store::open_in_memory().expect("in-memory store should open"),
                Some(format!("数据目录不可用，当前使用临时存储：{error}")),
            ),
        };
        let store = Arc::new(Mutex::new(store));
        let (theme_mode, light_theme) = {
            let guard = lock_store(&store);
            let mode = crate::ui::AppThemeMode::from_setting(
                guard.get_setting("theme").ok().flatten().as_deref(),
            );
            let system_is_light = crate::desktop::system_prefers_light().unwrap_or_else(|| {
                matches!(
                    window.appearance(),
                    WindowAppearance::Light | WindowAppearance::VibrantLight
                )
            });
            (mode, mode.resolve_light(system_is_light))
        };
        let (server_bind_lan, server_lan_host, update_repo) = {
            let guard = lock_store(&store);
            let bind_lan = guard
                .get_setting("server_bind_lan")
                .ok()
                .flatten()
                .as_deref()
                == Some("true");
            let lan_host = guard
                .get_setting("server_lan_host")
                .ok()
                .flatten()
                .map(|v| v.trim().to_string())
                .filter(|v| !v.is_empty());
            let repo = guard
                .get_setting("update_repo")
                .ok()
                .flatten()
                .map(|v| v.trim().to_string())
                .filter(|v| !v.is_empty())
                .unwrap_or_else(|| "youbilin/SwiftSub".to_string());
            (bind_lan, lan_host, repo)
        };
        let deleted_nodes = store
            .lock()
            .ok()
            .and_then(|store| store.get_setting("deleted_nodes").ok().flatten())
            .and_then(|value| serde_json::from_str(&value).ok())
            .unwrap_or_default();

        let (profile_id, profile_token) = {
            let guard = lock_store(&store);
            let active = guard
                .get_setting("active_profile")
                .ok()
                .flatten()
                .and_then(|id| id.parse::<i64>().ok());
            let existing = guard.list_profiles().unwrap_or_default();
            let selected = existing
                .iter()
                .find(|p| Some(p.id) == active)
                .or_else(|| existing.first())
                .cloned();
            match selected
                .map(Ok)
                .unwrap_or_else(|| guard.get_or_create_profile("默认配置"))
            {
                Ok(profile) => (profile.id, profile.token),
                Err(_) => (0, "swiftsub-local".to_string()),
            }
        };
        let (subscription_filename, rule_provider_base_url) = {
            let guard = lock_store(&store);
            let filename = guard
                .get_setting("subscription_filename")
                .ok()
                .flatten()
                .filter(|value| !value.trim().is_empty())
                .unwrap_or_else(|| "swiftsub.yaml".to_string());
            let base = guard
                .get_setting("rule_provider_base_url")
                .ok()
                .flatten()
                .filter(|value| value.starts_with("http"))
                .unwrap_or_else(|| GenerateOptions::default().rule_provider_base_url);
            (filename, base)
        };

        let search = cx.new(|cx| InputState::new(window, cx).placeholder("搜索节点…"));
        let add_name = cx.new(|cx| InputState::new(window, cx).placeholder("订阅名称（可选）"));
        let add_url = cx.new(|cx| {
            TextareaState::new(window, cx).placeholder("粘贴订阅地址或节点链接，支持多行")
        });
        let node_rename_input = cx.new(|cx| InputState::new(window, cx).placeholder("节点名称"));
        let node_edit_textarea =
            cx.new(|cx| TextareaState::new(window, cx).placeholder("节点配置 YAML"));
        let template_name =
            cx.new(|cx| InputState::new(window, cx).placeholder("模板名称（保存模板时填写）"));
        let template_rule_search =
            cx.new(|cx| InputState::new(window, cx).placeholder("搜索规则集…"));
        let template_rule_url = cx
            .new(|cx| InputState::new(window, cx).placeholder("规则集链接（.mrs / .yaml / .txt）"));
        let template_policy_name = cx
            .new(|cx| InputState::new(window, cx).placeholder("策略组名称，例如 流媒体 / AI 专线"));
        let template_policy_rename =
            cx.new(|cx| InputState::new(window, cx).placeholder("策略组名称"));
        let template_health_url =
            cx.new(|cx| InputState::new(window, cx).placeholder("测速 URL，留空继承全局"));
        let template_health_interval =
            cx.new(|cx| InputState::new(window, cx).placeholder("测速间隔秒数，留空继承"));
        let template_member_search =
            cx.new(|cx| InputState::new(window, cx).placeholder("搜索策略组 / 节点成员"));
        let advanced_draft = cx.new(|cx| TextareaState::new(window, cx));
        let template_custom_rules = cx.new(|cx| {
            TextareaState::new(window, cx).placeholder(
                "每行一条规则，例如 DOMAIN-SUFFIX,example.com,节点选择\nIP-CIDR,1.1.1.0/24,DIRECT,no-resolve",
            )
        });
        let filter_name =
            cx.new(|cx| InputState::new(window, cx).placeholder("筛选组名称（可选）"));
        let filter_include =
            cx.new(|cx| InputState::new(window, cx).placeholder("包含关键词，逗号分隔"));
        let filter_exclude =
            cx.new(|cx| InputState::new(window, cx).placeholder("排除关键词，逗号分隔"));
        let chain_name =
            cx.new(|cx| InputState::new(window, cx).placeholder("链式代理名称（可选）"));
        let chain_search = cx.new(|cx| InputState::new(window, cx).placeholder("搜索节点"));
        let subscription_filename_input =
            cx.new(|cx| InputState::new(window, cx).default_value(subscription_filename.clone()));
        let rule_provider_input =
            cx.new(|cx| InputState::new(window, cx).default_value(rule_provider_base_url.clone()));

        let enabled_modules: HashSet<String> = catalog::modules_for_preset(Preset::Minimal)
            .into_iter()
            .map(|module| module.id.to_string())
            .collect();
        let node_order = {
            let guard = lock_store(&store);
            guard
                .get_setting("node_order")
                .ok()
                .flatten()
                .and_then(|value| serde_json::from_str::<Vec<String>>(&value).ok())
                .unwrap_or_default()
        };
        let disabled_rule_sets = {
            let guard = lock_store(&store);
            guard
                .get_setting("disabled_rule_sets")
                .ok()
                .flatten()
                .and_then(|value| serde_json::from_str::<Vec<String>>(&value).ok())
                .unwrap_or_default()
                .into_iter()
                .collect()
        };
        let (custom_rule_sets, custom_rule_lines) = {
            let guard = lock_store(&store);
            let rule_sets = guard
                .get_setting("custom_rule_sets")
                .ok()
                .flatten()
                .and_then(|value| serde_json::from_str::<Vec<RuleSetBinding>>(&value).ok())
                .unwrap_or_default();
            let rule_lines = guard
                .get_setting("custom_rule_lines")
                .ok()
                .flatten()
                .and_then(|value| serde_json::from_str::<Vec<String>>(&value).ok())
                .unwrap_or_default();
            (rule_sets, rule_lines)
        };
        let custom_rules =
            parse_custom_rule_lines(&custom_rule_lines.join("\n")).unwrap_or_default();
        let (policy_groups, rule_set_targets) = {
            let guard = lock_store(&store);
            (
                guard
                    .get_setting("policy_groups")
                    .ok()
                    .flatten()
                    .and_then(|value| serde_json::from_str::<Vec<PolicyGroupSpec>>(&value).ok())
                    .unwrap_or_default(),
                guard
                    .get_setting("rule_set_targets")
                    .ok()
                    .flatten()
                    .and_then(|value| serde_json::from_str::<HashMap<String, String>>(&value).ok())
                    .unwrap_or_default(),
            )
        };
        let rule_order = lock_store(&store)
            .get_setting("rule_order")
            .ok()
            .flatten()
            .and_then(|v| serde_json::from_str(&v).ok())
            .unwrap_or_default();
        let options = GenerateOptions {
            preset: Preset::Minimal,
            rule_order,
            group_order: lock_store(&store)
                .get_setting("group_order")
                .ok()
                .flatten()
                .and_then(|value| serde_json::from_str(&value).ok())
                .unwrap_or_default(),
            policy_groups,
            rule_set_targets,
            rule_provider_base_url,
            custom_rule_sets: custom_rule_sets.clone(),
            custom_rules,
            ..Default::default()
        };

        let profiles = lock_store(&store).list_profiles().unwrap_or_default();
        let profile_name_input = cx.new(|cx| InputState::new(window, cx).placeholder("配置名称"));
        let generated_config_name =
            cx.new(|cx| InputState::new(window, cx).placeholder("配置名称"));
        let saved_editor = SavedEditor::new(window, cx);
        let saved_config_name = cx.new(|cx| InputState::new(window, cx).placeholder("配置名称"));
        let saved_config_yaml = cx.new(|cx| {
            TextareaState::new(window, cx)
                .searchable(true)
                .placeholder("选择或保存一份 YAML 配置")
        });
        let p1 = phase2::Phase2State::new(&store, window, cx);
        let mut app = Self {
            p1,
            upgrades: upgrades::Upgrades::new(window, cx),
            desktop: crate::desktop::DesktopIntegration::new(window, instance),
            close_to_tray: false,
            recovered_draft: false,
            last_draft_checkpoint: std::cell::RefCell::new(None),
            page: Page::Generator,
            light_theme,
            store,
            store_error,
            data_dir,
            subscriptions: Vec::new(),
            nodes: Vec::new(),
            node_order,
            parse_warnings: Vec::new(),
            import_report: Vec::new(),
            source_import_report: Vec::new(),
            generation_report: String::new(),
            search,
            add_open: false,
            add_name,
            add_url,
            add_error: None,
            custom_dialog: CustomDialog::None,
            filter_name,
            filter_include,
            filter_exclude,
            chain_name,
            chain_search,
            filter_regions: HashSet::new(),
            chain_entry_key: None,
            chain_entry_name: None,
            chain_landing_key: None,
            chain_landing_name: None,
            custom_error: None,
            custom_groups: Vec::new(),
            chains: Vec::new(),
            custom_templates: Vec::new(),
            saved_configs: Vec::new(),
            saved_config_id: None,
            saved_config_name,
            saved_editor,
            generated_config_name,
            generated_save_open: false,
            generated_save_update: false,
            generated_save_error: None,
            clash_import_busy: false,
            saved_config_yaml,
            saved_config_error: None,
            saved_file_baseline: None,
            saved_external_change: false,
            saved_draft_corrupt: false,
            saved_config_delete_armed: false,
            generated_saved_config: None,
            generation_cache: generation::GenerationCache::default(),
            template_name,
            template_error: None,
            template_dialog_open: false,
            template_edit_id: None,
            template_session_active: false,
            template_advanced_saved: String::new(),
            template_baseline: None,
            template_base_options: None,
            refreshing: HashSet::new(),
            refresh_queue: VecDeque::new(),
            refresh_versions: HashMap::new(),
            refresh_reload_pending: false,
            parsed_sources: generation::ParseCache::default(),
            publication: publication::PublicationWorker::default(),
            failure_counts: HashMap::new(),
            retry_after: HashMap::new(),
            options,
            subscription_filename,
            subscription_filename_input,
            rule_provider_input,
            enabled_modules,
            disabled_rule_sets,
            custom_rule_sets,
            template_disabled_rule_sets: HashSet::new(),
            template_custom_rule_sets: Vec::new(),
            template_enabled_modules: HashSet::new(),
            template_policy_groups: Vec::new(),
            template_group_order: Vec::new(),
            template_rule_set_targets: HashMap::new(),
            template_policy_name,
            template_policy_rename,
            template_health_url,
            template_health_interval,
            template_member_search,
            template_rule_order: Vec::new(),
            template_policy_edit: None,
            template_rules_target_filter: String::new(),
            template_rule_search,
            template_rule_url,
            template_custom_rules,
            advanced_draft,
            template_tab: TemplateTab::PolicyGroups,
            workspace: workspace::Workspace::new(window, cx),
            settings_tab: SettingsTab::Generation,
            template_rule_sources_collapsed: rule_catalog::SOURCES
                .iter()
                .map(|source| source.id.to_string())
                .chain(std::iter::once("builtin".to_string()))
                .collect(),
            profiles,
            profile_name_input,
            profile_membership: None,
            profile_id,
            profile_token,
            server: None,
            server_bind_lan,
            server_lan_host,
            lan_interfaces: Vec::new(),
            lan_interfaces_refreshing: false,
            server_url: None,
            server_error: None,
            update_repo,
            update_checking: false,
            update_result: None,
            update_error: None,
            preview: String::new(),
            node_rename_key: None,
            node_rename_sub_id: None,
            node_rename_input,
            node_edit_textarea,
            node_rename_error: None,
            stats: ProfileStats::default(),
            gen_warnings: Vec::new(),
            toast: None,
            toast_epoch: 0,
            disabled_nodes: HashSet::new(),
            deleted_nodes,
            preview_scroll: ScrollHandle::default(),
            _subscriptions: Vec::new(),
        };

        let mode = if app.light_theme {
            ThemeMode::Light
        } else {
            ThemeMode::Dark
        };
        Theme::change(mode, None, cx);
        crate::configure_theme(cx, app.light_theme);
        crate::ui::set_light_theme(app.light_theme);
        cx.set_window_appearance(match theme_mode {
            crate::ui::AppThemeMode::System => None,
            crate::ui::AppThemeMode::Light => Some(WindowAppearance::Light),
            crate::ui::AppThemeMode::Dark => Some(WindowAppearance::Dark),
        });

        let add_url_sub = cx.subscribe_in(
            &app.add_url,
            window,
            |this, _, event: &InputEvent, _, cx| {
                if matches!(event, InputEvent::Change) {
                    this.add_error = None;
                    cx.notify();
                }
            },
        );
        let add_name_sub = cx.subscribe_in(
            &app.add_name,
            window,
            |this, _, event: &InputEvent, window, cx| {
                if matches!(event, InputEvent::PressEnter { .. }) {
                    this.submit_add(window, cx);
                }
            },
        );
        let search_obs = cx.observe_in(&app.search, window, |_, _, _, cx| cx.notify());
        let chain_search_obs = cx.observe_in(&app.chain_search, window, |_, _, _, cx| cx.notify());
        let member_search_obs =
            cx.observe_in(&app.template_member_search, window, |_, _, _, cx| {
                cx.notify()
            });
        let saved_yaml_obs =
            cx.observe_in(&app.saved_config_yaml, window, |_, _, _, cx| cx.notify());
        let generated_name_obs =
            cx.observe_in(&app.generated_config_name, window, |_, _, _, cx| {
                cx.notify()
            });
        let generated_name_sub = cx.subscribe_in(
            &app.generated_config_name,
            window,
            |this, _, event: &InputEvent, window, cx| {
                if this.page == Page::Generator && this.generated_save_open {
                    match event {
                        InputEvent::PressEnter { .. } => {
                            this.save_generated_config(this.generated_save_update, window, cx);
                        }
                        InputEvent::Change => {
                            this.generated_save_error = None;
                            cx.notify();
                        }
                        _ => {}
                    }
                }
            },
        );
        let saved_name_obs =
            cx.observe_in(&app.saved_config_name, window, |_, _, _, cx| cx.notify());
        let template_rule_search_obs =
            cx.observe_in(&app.template_rule_search, window, |_, _, _, cx| cx.notify());
        let rule_content_obs = cx.observe_in(&app.workspace.rule_content, window, |_, _, _, cx| {
            cx.notify()
        });
        let rule_content_search_obs =
            cx.observe_in(&app.workspace.rule_content_search, window, |_, _, _, cx| {
                cx.notify()
            });
        let tpl_builtin_search_obs = cx.observe_in(
            &app.workspace.template_editor.builtin_search,
            window,
            |_, _, _, cx| cx.notify(),
        );
        let tpl_rule_search_obs = cx.observe_in(
            &app.workspace.template_editor.rule_search,
            window,
            |_, _, _, cx| cx.notify(),
        );
        let visual_inputs = [
            app.saved_editor.search.clone(),
            app.saved_editor.member_search.clone(),
            app.saved_editor.group_name.clone(),
            app.saved_editor.group_url.clone(),
            app.saved_editor.group_interval.clone(),
            app.saved_editor.node_name.clone(),
            app.saved_editor.node_server.clone(),
            app.saved_editor.node_port.clone(),
            app.saved_editor.new_rule.clone(),
            app.saved_editor.mixed_port.clone(),
        ];
        let visual_observers: Vec<_> = visual_inputs
            .iter()
            .map(|input| cx.observe_in(input, window, |_, _, _, cx| cx.notify()))
            .collect();
        app._subscriptions = vec![
            add_url_sub,
            add_name_sub,
            search_obs,
            chain_search_obs,
            template_rule_search_obs,
            rule_content_obs,
            rule_content_search_obs,
            tpl_builtin_search_obs,
            tpl_rule_search_obs,
            member_search_obs,
            saved_yaml_obs,
            saved_name_obs,
            generated_name_obs,
            generated_name_sub,
        ];
        app._subscriptions.extend(visual_observers);

        let args: Vec<String> = std::env::args().collect();
        let demo_mode = args.iter().any(|arg| arg == "--demo");
        let page_override = args
            .iter()
            .find_map(|arg| arg.strip_prefix("--page=").map(str::to_string));
        let dialog_override = args
            .iter()
            .find_map(|arg| arg.strip_prefix("--dialog=").map(str::to_string));
        let settings_tab_override = args
            .iter()
            .find_map(|arg| arg.strip_prefix("--settings-tab=").map(str::to_string));
        let open_template_override = args
            .iter()
            .find_map(|arg| arg.strip_prefix("--open-template=").map(str::to_string));

        app.load_custom_config();
        let recovered = lock_store(&app.store)
            .get_setting("routing_snapshot")
            .ok()
            .flatten()
            .and_then(|v| serde_json::from_str::<TemplateSnapshot>(&v).ok());
        if let Some(mut snapshot) = recovered {
            snapshot.remove_legacy_regions();
            // Existing individual settings remain authoritative for legacy UI actions.
            app.options.test_url = snapshot.test_url;
            app.options.test_interval = snapshot.test_interval;
            app.options.test_tolerance = snapshot.test_tolerance;
            app.options.mixed_port = snapshot.mixed_port;
            app.options.allow_lan = snapshot.allow_lan;
            app.options.dns = snapshot.dns;
            app.options.listeners = snapshot.listeners;
            app.options.extra_yaml = snapshot.extra_yaml;
        }
        app.load_module_config();
        app.load_templates();
        app.reload_saved_configs();
        app.load_profile_pull_history();
        app.restore_selected_profile(window, cx);
        app.workspace_load_global_defaults();
        app.reload_subscriptions(cx);
        if demo_mode {
            if app.subscriptions.is_empty() {
                app.load_demo(cx);
            } else {
                app.page = Page::Generator;
            }
        }
        app.recount_manual_nodes();
        app.start_server();
        app.start_scheduler(cx);
        app.start_desktop_lifecycle(window, cx);
        app.refresh_lan_interfaces(cx);
        if let Some(notice) = restore_notice {
            app.show_toast(notice, cx);
        }
        app.restore_draft(window, cx);
        if let Some(page) = page_override {
            app.page = match page.as_str() {
                "generator" => Page::Generator,
                "templates" => Page::Templates,
                "configs" => Page::SavedConfigs,
                "settings" => Page::Settings,
                _ => Page::Subscriptions,
            };
        }
        if let Some(dialog) = dialog_override {
            match dialog.as_str() {
                "add" => {
                    app.page = Page::Generator;
                    app.add_open = true;
                }
                "filter" => {
                    app.page = Page::Generator;
                    app.custom_dialog = CustomDialog::Filter;
                }
                "chain" => {
                    app.page = Page::Generator;
                    app.open_template_dialog(window, cx);
                    app.template_tab = TemplateTab::Tools;
                }
                "rules" => {
                    app.page = Page::Generator;
                    app.open_template_dialog(window, cx);
                }
                _ => {}
            }
        }
        if let Some(tab) = settings_tab_override {
            app.page = Page::Settings;
            app.settings_tab = match tab.as_str() {
                "rules" => SettingsTab::RuleSources,
                "templates" => SettingsTab::Templates,
                "data" => SettingsTab::Data,
                _ => SettingsTab::Generation,
            };
        }
        if let Some(target_tpl) = open_template_override
            && let Some(template) = app.custom_templates.first().cloned()
        {
            app.page = Page::Settings;
            app.settings_tab = SettingsTab::Templates;
            app.open_edit_custom_template(template.id, window, cx);
            if !target_tpl.is_empty() && target_tpl != "0" {
                app.workspace.template_editor.expanded_group = Some(target_tpl);
            }
        }
        if app.page == Page::SavedConfigs
            && app.saved_config_id.is_none()
            && let Some(config) = app.saved_configs.first()
        {
            app.open_saved_config(config.id.clone(), window, cx);
        }
        if app.page == Page::Generator {
            app.open_template_dialog(window, cx);
        }
        app
    }

    fn recount_manual_nodes(&mut self) {
        let mut updates = Vec::new();
        for subscription in &self.subscriptions {
            if subscription.kind == swiftsub_store::SubscriptionKind::Url
                || subscription.node_count > 0
            {
                continue;
            }
            let Some(payload) = &subscription.payload else {
                continue;
            };
            if let Ok(parsed) = parse_content(payload) {
                updates.push((subscription.id, parsed.proxies.len()));
            }
        }
        if updates.is_empty() {
            return;
        }

        {
            let store = lock_store(&self.store);
            for (id, count) in &updates {
                let _ = store.set_subscription_node_count(*id, *count);
            }
        }
        for subscription in &mut self.subscriptions {
            if let Some((_, count)) = updates.iter().find(|(id, _)| *id == subscription.id) {
                subscription.node_count = *count;
            }
        }
    }

    pub fn set_theme(&mut self, light: bool, cx: &mut Context<Self>) {
        if self.light_theme == light {
            return;
        }
        self.light_theme = light;
        self.p1.theme_mode = if light {
            crate::ui::AppThemeMode::Light
        } else {
            crate::ui::AppThemeMode::Dark
        };
        {
            let store = lock_store(&self.store);
            let _ = store.set_setting("theme", if light { "light" } else { "dark" });
        }
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

    pub fn set_close_to_tray(&mut self, enabled: bool, cx: &mut Context<Self>) {
        self.close_to_tray = enabled;
        if let Ok(store) = self.store.lock() {
            let _ = store.set_setting("close_to_tray", if enabled { "true" } else { "false" });
        }
        cx.notify();
    }

    pub fn set_page(&mut self, page: Page, window: &mut Window, cx: &mut Context<Self>) {
        if let Err(error) = self.workspace_checkpoint_rule_content(cx) {
            self.workspace.rule_content_error = Some(error);
            cx.notify();
            return;
        }
        if let Err(e) = self.autosave_saved_editor(window, cx) {
            self.saved_config_error = Some(e);
        }
        if let Err(e) = self.checkpoint_yaml_editor(cx) {
            self.saved_config_error = Some(e);
            return;
        }
        if self.template_dialog_open && page != Page::Generator {
            self.request_close_template(window, cx);
        }
        if page != self.page {
            match page {
                Page::Generator => self.preview_scroll.set_offset(point(px(0.0), px(0.0))),
                Page::Settings => {
                    self.workspace.settings_error_section = None;
                }
                Page::Subscriptions | Page::Templates | Page::SavedConfigs => {}
            }
        }
        self.page = page;
        if page == Page::Generator && !self.workspace.saved_editor_open {
            self.open_template_dialog(window, cx);
        }
        if page == Page::SavedConfigs {
            self.workspace.saved_editor_open = false;
        }
        cx.notify();
    }

    pub fn open_add_dialog(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.add_open = true;
        self.add_error = None;
        self.add_url.read(cx).focus_handle(cx).focus(window, cx);
        cx.notify();
    }

    fn append_manual_proxies(
        &mut self,
        proxies: Vec<Proxy>,
    ) -> Result<(usize, usize, usize), String> {
        let store = lock_store(&self.store);

        let existing_id = store
            .get_setting("manual_subscription_id")
            .ok()
            .flatten()
            .and_then(|value| value.parse::<i64>().ok())
            .filter(|id| store.get_subscription(*id).ok().flatten().is_some())
            .or_else(|| {
                store.list_subscriptions().ok().and_then(|list| {
                    list.into_iter()
                        .find(|subscription| {
                            subscription.name == "手动节点" && subscription.url.is_none()
                        })
                        .map(|subscription| subscription.id)
                })
            });

        let id = match existing_id {
            Some(id) => id,
            None => {
                let id = store
                    .add_subscription(&NewSubscription::manual(
                        "手动节点",
                        swiftsub_store::SubscriptionKind::Yaml,
                        "proxies: []",
                    ))
                    .map_err(|error| error.to_string())?;
                let _ = store.set_setting("manual_subscription_id", &id.to_string());
                id
            }
        };

        let existing = store
            .get_subscription(id)
            .map_err(|error| error.to_string())?;
        let existing_payload = existing.as_ref().and_then(|value| value.payload.as_deref());
        let visible_keys: HashSet<String> = self
            .nodes
            .iter()
            .map(|node| node.source_key.clone())
            .collect();
        let (payload, added, duplicates, total) = merge_manual_proxies_restoring_deleted(
            existing_payload,
            proxies,
            &mut self.deleted_nodes,
            &visible_keys,
        )?;
        store
            .save_manual_payload(id, &payload, total)
            .map_err(|error| error.to_string())?;
        let deleted_json =
            serde_json::to_string(&self.deleted_nodes).map_err(|error| error.to_string())?;
        store
            .set_setting("deleted_nodes", &deleted_json)
            .map_err(|error| error.to_string())?;
        if let Some(ids) = &mut self.profile_membership {
            ids.insert(id);
        }
        Ok((added, duplicates, total))
    }

    pub fn open_custom_dialog(&mut self, dialog: CustomDialog, cx: &mut Context<Self>) {
        self.custom_dialog = dialog;
        self.workspace.edit_filter = None;
        self.workspace.edit_chain = None;
        self.custom_error = None;
        cx.notify();
    }

    pub fn close_custom_dialog(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.custom_dialog = CustomDialog::None;
        self.custom_error = None;
        self.filter_regions.clear();
        self.chain_entry_key = None;
        self.chain_entry_name = None;
        self.chain_landing_key = None;
        self.chain_landing_name = None;
        self.chain_search
            .update(cx, |state, cx| state.set_value("", window, cx));
        self.filter_name
            .update(cx, |state, cx| state.set_value("", window, cx));
        self.filter_include
            .update(cx, |state, cx| state.set_value("", window, cx));
        self.filter_exclude
            .update(cx, |state, cx| state.set_value("", window, cx));
        self.chain_name
            .update(cx, |state, cx| state.set_value("", window, cx));
        self.workspace
            .filter_sources
            .update(cx, |state, cx| state.set_value("", window, cx));
        self.workspace
            .chain_hops
            .update(cx, |state, cx| state.set_value("", window, cx));
        self.workspace.edit_filter = None;
        self.workspace.edit_chain = None;
        cx.notify();
    }

    pub fn toggle_filter_region(&mut self, region: Region, cx: &mut Context<Self>) {
        if !self.filter_regions.insert(region) {
            self.filter_regions.remove(&region);
        }
        cx.notify();
    }

    pub fn add_filter_group(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.save_workspace_filter(window, cx);
    }

    pub fn add_chain(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.save_workspace_chain(window, cx);
    }

    fn load_custom_config(&mut self) {
        let store = lock_store(&self.store);
        if let Some(value) = store.get_setting("custom_groups").ok().flatten()
            && let Ok(items) = serde_json::from_str::<Vec<CustomGroupSpec>>(&value)
        {
            self.custom_groups = items;
        }
        if let Some(value) = store.get_setting("chains").ok().flatten()
            && let Ok(items) = serde_json::from_str::<Vec<ChainSpec>>(&value)
        {
            self.chains = items;
        }
    }

    pub fn close_add_dialog(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.add_open = false;
        self.add_error = None;
        self.add_name
            .update(cx, |state, cx| state.set_value("", window, cx));
        self.add_url
            .update(cx, |state, cx| state.set_value("", window, cx));
        cx.notify();
    }

    pub(crate) fn submit_add(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.add_open {
            return;
        }
        let input = self.add_url.read(cx).value().trim().to_string();
        if input.is_empty() {
            self.add_error = Some("请粘贴订阅地址或节点链接".into());
            cx.notify();
            return;
        }

        let typed_name = self.add_name.read(cx).value().trim().to_string();
        if input.lines().count() > 1
            || is_node_link(&input)
            || looks_like_clash_yaml(&input)
            || !(input.starts_with("http://") || input.starts_with("https://"))
        {
            self.add_single_node(input, typed_name, window, cx);
            return;
        }

        if url::Url::parse(&input).is_err() {
            self.add_error = Some("请输入有效的 HTTP/HTTPS 订阅链接".to_string());
            cx.notify();
            return;
        }
        let url = input;

        if let Some(existing) = self
            .subscriptions
            .iter()
            .find(|sub| sub.url.as_deref() == Some(&url))
            .cloned()
        {
            if let Some(ids) = &mut self.profile_membership {
                ids.insert(existing.id);
            }
            if !existing.enabled {
                let _ = lock_store(&self.store).set_subscription_enabled(existing.id, true);
            }
            let restored = self.restore_deleted_nodes_for_subscription(&existing);
            self.close_add_dialog(window, cx);
            self.reload_subscriptions(cx);
            self.fetch_in_background(
                refresh::FetchRequest {
                    id: existing.id,
                    url,
                    name: existing.name,
                    cache: CacheValidators::default(),
                    options: self.fetch_options(),
                    restore_deleted: true,
                },
                cx,
            );
            self.show_toast(
                if restored > 0 {
                    format!("订阅已存在，已恢复 {restored} 个已删节点并正在刷新")
                } else {
                    "订阅已存在，正在刷新原订阅".into()
                },
                cx,
            );
            return;
        }

        let name = if typed_name.is_empty() {
            default_subscription_name(&url)
        } else {
            typed_name
        };

        let result = {
            let store = lock_store(&self.store);
            store.add_subscription(&NewSubscription::url(name.clone(), url.clone()))
        };

        match result {
            Ok(id) => {
                if let Some(ids) = &mut self.profile_membership {
                    ids.insert(id);
                }
                self.add_open = false;
                self.add_error = None;
                self.add_name
                    .update(cx, |state, cx| state.set_value("", window, cx));
                self.add_url
                    .update(cx, |state, cx| state.set_value("", window, cx));
                self.reload_subscriptions(cx);
                self.fetch_in_background(
                    refresh::FetchRequest {
                        id,
                        url,
                        name,
                        cache: CacheValidators::default(),
                        options: self.fetch_options(),
                        restore_deleted: true,
                    },
                    cx,
                );
            }
            Err(error) => {
                self.add_error = Some(format!("保存订阅失败：{error}"));
                cx.notify();
            }
        }
    }

    fn add_single_node(
        &mut self,
        input: String,
        typed_name: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let mut parsed = match parse_content(&input) {
            Ok(parsed) if !parsed.proxies.is_empty() => {
                self.import_report = parsed.report.clone();
                parsed
            }
            Ok(_) => {
                self.add_error = Some("没有从链接中解析出节点".to_string());
                cx.notify();
                return;
            }
            Err(error) => {
                self.add_error = Some(format!("节点链接解析失败：{error}"));
                cx.notify();
                return;
            }
        };

        let errors: Vec<_> = parsed
            .report
            .iter()
            .filter(|item| item.status == swiftsub_core::parser::ImportStatus::Rejected)
            .map(|item| format!("第 {} 行：{}", item.index, item.messages.join("；")))
            .collect();
        if !errors.is_empty() {
            self.add_error = Some(format!(
                "识别到 {} 个有效节点，请修正以下内容后添加：\n{}",
                parsed.proxies.len(),
                errors.join("\n")
            ));
            cx.notify();
            return;
        }
        if parsed.proxies.len() == 1 && !typed_name.is_empty() {
            parsed.proxies[0].set_name(typed_name);
        }
        let previous_keys: HashSet<_> = self
            .nodes
            .iter()
            .map(|node| node.source_key.clone())
            .collect();
        match self.append_manual_proxies(parsed.proxies) {
            Ok((added, duplicates, total)) => {
                self.close_add_dialog(window, cx);
                self.reload_subscriptions(cx);
                self.workspace.node_source.clear();
                self.search
                    .update(cx, |input, cx| input.set_value("", window, cx));
                if let Some(index) = self
                    .nodes
                    .iter()
                    .position(|node| !previous_keys.contains(&node.source_key))
                {
                    self.workspace
                        .node_scroll
                        .scroll_to_item_strict(index, gpui_kit::ScrollStrategy::Top);
                }
                self.show_toast(
                    format!(
                        "新增 {added} 个节点，{duplicates} 个已存在或重复，手动节点共 {total} 个"
                    ),
                    cx,
                );
            }
            Err(error) => {
                self.add_error = Some(format!("保存节点失败：{error}"));
                cx.notify();
            }
        }
    }

    fn restore_deleted_nodes_for_subscription(
        &mut self,
        subscription: &StoredSubscription,
    ) -> usize {
        let Some(payload) = subscription.payload.as_deref() else {
            return 0;
        };
        let Ok(parsed) = self.parsed_sources.parse(subscription.id, payload) else {
            return 0;
        };
        let keys: Vec<String> = parsed.proxies.iter().map(Proxy::identity_key).collect();
        self.restore_deleted_node_keys(&keys)
    }

    pub(super) fn restore_deleted_node_keys(&mut self, keys: &[String]) -> usize {
        let mut restored = 0;
        for key in keys {
            if self.deleted_nodes.remove(key) {
                restored += 1;
            }
        }
        if restored > 0
            && let Ok(json) = serde_json::to_string(&self.deleted_nodes)
        {
            let _ = lock_store(&self.store).set_setting("deleted_nodes", &json);
        }
        restored
    }

    pub fn paste_add_input(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(text) = cx.read_from_clipboard().and_then(|item| item.text()) {
            self.add_url
                .update(cx, |input, cx| input.set_value(text, window, cx));
            self.add_error = None;
            cx.notify();
        }
    }

    pub fn load_demo(&mut self, cx: &mut Context<Self>) {
        if let Some(existing) = self
            .subscriptions
            .iter()
            .find(|subscription| subscription.name == "示例订阅（可删除）")
            .cloned()
        {
            let restored = self.restore_deleted_nodes_for_subscription(&existing);
            if restored > 0 {
                self.reload_subscriptions(cx);
            }
            self.page = Page::Generator;
            self.show_toast(
                if restored > 0 {
                    format!("示例订阅已恢复 {restored} 个节点")
                } else {
                    "示例订阅已存在".to_string()
                },
                cx,
            );
            return;
        }
        const DEMO: &str = include_str!("../assets/demo.yaml");
        let parsed_demo = parse_content(DEMO).ok();
        let node_count = parsed_demo
            .as_ref()
            .map(|parsed| parsed.proxies.len())
            .unwrap_or_default();
        if let Some(parsed) = &parsed_demo {
            let keys: Vec<String> = parsed.proxies.iter().map(Proxy::identity_key).collect();
            self.restore_deleted_node_keys(&keys);
        }
        let result = {
            let store = lock_store(&self.store);
            store
                .add_subscription(&NewSubscription::manual(
                    "示例订阅（可删除）",
                    swiftsub_store::SubscriptionKind::Yaml,
                    DEMO,
                ))
                .inspect(|&id| {
                    let _ = store.set_subscription_node_count(id, node_count);
                })
        };
        match result {
            Ok(_) => {
                self.reload_subscriptions(cx);
                self.show_toast("已添加示例订阅".to_string(), cx);
                self.page = Page::Generator;
            }
            Err(error) => self.show_toast(format!("添加示例失败：{error}"), cx),
        }
    }

    pub fn import_file(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(path) = rfd::FileDialog::new()
            .add_filter("订阅或配置", &["yaml", "yml", "txt"])
            .pick_file()
        else {
            return;
        };

        let content = match std::fs::read_to_string(&path) {
            Ok(content) => content,
            Err(error) => {
                self.add_error = Some(format!("读取文件失败：{error}"));
                cx.notify();
                return;
            }
        };

        match parse_content(&content) {
            Ok(parsed) if !parsed.proxies.is_empty() => {
                let name = path
                    .file_stem()
                    .and_then(|name| name.to_str())
                    .filter(|name| !name.is_empty())
                    .unwrap_or("本地文件")
                    .to_string();
                let kind = if looks_like_clash_yaml(&content) {
                    swiftsub_store::SubscriptionKind::Yaml
                } else {
                    swiftsub_store::SubscriptionKind::Links
                };
                let result = {
                    let store = lock_store(&self.store);
                    store.add_subscription(&NewSubscription::manual(name, kind, content))
                };
                match result {
                    Ok(id) => {
                        let keys: Vec<String> =
                            parsed.proxies.iter().map(Proxy::identity_key).collect();
                        self.restore_deleted_node_keys(&keys);
                        let count = parsed.proxies.len();
                        {
                            let store = lock_store(&self.store);
                            let _ = store.set_subscription_node_count(id, count);
                        }
                        self.close_add_dialog(window, cx);
                        self.reload_subscriptions(cx);
                        self.show_toast(format!("已导入 {count} 个节点"), cx);
                    }
                    Err(error) => {
                        self.add_error = Some(format!("保存失败：{error}"));
                        cx.notify();
                    }
                }
            }
            Ok(_) => {
                self.add_error = Some("文件中没有可用节点".to_string());
                cx.notify();
            }
            Err(error) => {
                self.add_error = Some(format!("无法识别文件内容：{error}"));
                cx.notify();
            }
        }
    }

    pub fn reload_subscriptions(&mut self, cx: &mut Context<Self>) {
        let result = {
            let store = lock_store(&self.store);
            store.list_subscriptions()
        };
        match result {
            Ok(subscriptions) => self.subscriptions = subscriptions,
            Err(error) => self.store_error = Some(error.to_string()),
        }
        self.parsed_sources.retain(&self.subscriptions);
        self.rebuild_nodes(cx);
    }

    fn start_server(&mut self) {
        let mut candidates = Vec::new();
        if let Some(fixed_port) = self.p1.server_fixed_port {
            candidates.push(fixed_port);
        } else {
            let preferred_port = {
                let store = lock_store(&self.store);
                store
                    .get_setting("server_port")
                    .ok()
                    .flatten()
                    .and_then(|value| value.parse::<u16>().ok())
            };
            if let Some(port) = preferred_port.filter(|port| *port > 0) {
                candidates.push(port);
            }
            // Always include fallback ports outside Clash's 7890-7899 range, plus OS-assigned port 0.
            for port in (17899..=17920).chain(7899..=7910).chain(std::iter::once(0)) {
                if !candidates.contains(&port) {
                    candidates.push(port);
                }
            }
        }

        let mut last_error = None;
        for port in candidates {
            let config = ServerConfig {
                port,
                token: self.profile_token.clone(),
                bind_lan: self.server_bind_lan,
                lan_private_only: self.p1.server_lan_private_only,
                update_interval_hours: 24,
            };
            let snapshot = ProfileSnapshot {
                yaml: self.preview.clone(),
                userinfo_header: self.aggregate_userinfo(),
                update_interval_hours: 24,
            };
            match LocalServer::start(config, snapshot) {
                Ok(server) => {
                    let bound_port = server.local_addr().port();
                    {
                        let store = lock_store(&self.store);
                        let _ = store.set_setting("server_port", &bound_port.to_string());
                    }
                    self.server_url =
                        Some(server.profile_url(&self.profile_token, &self.subscription_filename));
                    self.server = Some(server);
                    self.publish_profiles();
                    self.server_error = None;
                    return;
                }
                Err(error) => last_error = Some(error),
            }
        }
        self.server_error = last_error.map(|error| error.to_string());
    }

    pub fn set_server_bind_lan(&mut self, enabled: bool, cx: &mut Context<Self>) {
        if self.server_bind_lan == enabled && self.server.is_some() {
            return;
        }
        let previous = self.server_bind_lan;
        self.server_bind_lan = enabled;
        if let Some(mut server) = self.server.take() {
            server.stop();
        }
        self.start_server();
        if self.server.is_none() && enabled {
            let err = self
                .server_error
                .clone()
                .unwrap_or_else(|| "端口绑定失败".into());
            self.server_bind_lan = previous;
            self.start_server();
            self.show_toast(format!("开启局域网导入失败：{err}"), cx);
            cx.notify();
            return;
        }
        {
            let store = lock_store(&self.store);
            let _ = store.set_setting(
                "server_bind_lan",
                if self.server_bind_lan {
                    "true"
                } else {
                    "false"
                },
            );
        }
        let message = if let Some(error) = &self.server_error {
            format!("重启本地订阅服务失败：{error}")
        } else if self.server_bind_lan {
            if self.lan_interfaces.is_empty() {
                self.lan_interfaces = swiftsub_server::list_lan_interfaces();
            } else {
                self.refresh_lan_interfaces(cx);
            }
            let port = self
                .server
                .as_ref()
                .map(|server| server.local_addr().port())
                .unwrap_or(17899);
            if let Some(ip) = self.active_lan_host() {
                format!("已开启局域网导入（{ip}:{port}）")
            } else {
                format!("已开启局域网监听（0.0.0.0:{port}），暂未检测到局域网 IP")
            }
        } else {
            "已关闭局域网导入（仅本机 127.0.0.1 可用）".into()
        };
        self.show_toast(message, cx);
        cx.notify();
    }

    pub fn check_for_updates(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.update_checking {
            return;
        }
        self.update_checking = true;
        self.update_error = None;
        let repo = self.update_repo.clone();
        let current = env!("CARGO_PKG_VERSION").to_string();
        let fetch_opts = self.fetch_options();
        cx.notify();
        cx.spawn_in(window, async move |this, cx| {
            let result =
                cx.background_executor()
                    .spawn(async move {
                        swiftsub_fetch::check_github_release(&repo, &current, &fetch_opts)
                    })
                    .await;
            let _ = cx.update(|_, cx| {
                let _ = this.update(cx, |this, cx| {
                    this.update_checking = false;
                    match result {
                        Ok(info) => {
                            let msg = if info.has_update {
                                format!("发现新版本 v{}", info.version)
                            } else {
                                format!("当前已是最新版本（v{}）", env!("CARGO_PKG_VERSION"))
                            };
                            this.update_result = Some(info);
                            this.update_error = None;
                            this.show_toast(msg, cx);
                        }
                        Err(error) => {
                            this.update_error = Some(error.clone());
                            this.show_toast(format!("检查更新失败：{error}"), cx);
                        }
                    }
                    cx.notify();
                });
            });
        })
        .detach();
    }

    pub fn open_update_download(&mut self, cx: &mut Context<Self>) {
        let Some(info) = &self.update_result else {
            return;
        };
        let target = info
            .download_url
            .as_deref()
            .unwrap_or(info.html_url.as_str());
        match crate::desktop::open_external_url(target) {
            Ok(()) => self.show_toast("已在浏览器中打开下载页面".into(), cx),
            Err(error) => self.show_toast(error, cx),
        }
    }

    pub fn open_data_dir(&mut self, cx: &mut Context<Self>) {
        match crate::desktop::open_folder(&self.data_dir) {
            Ok(()) => {}
            Err(error) => self.show_toast(format!("无法打开数据目录：{error}"), cx),
        }
    }

    pub fn refresh_lan_interfaces(&mut self, cx: &mut Context<Self>) {
        if self.lan_interfaces_refreshing {
            return;
        }
        self.lan_interfaces_refreshing = true;
        cx.spawn(async move |this, cx| {
            let interfaces = cx
                .background_executor()
                .spawn(async move { swiftsub_server::list_lan_interfaces() })
                .await;
            let Some(this) = this.upgrade() else {
                return;
            };
            this.update(cx, |this, cx| {
                this.lan_interfaces_refreshing = false;
                if this.lan_interfaces != interfaces {
                    this.lan_interfaces = interfaces;
                    cx.notify();
                }
            });
        })
        .detach();
    }

    pub fn active_lan_host(&self) -> Option<String> {
        if let Some(host) = self
            .server_lan_host
            .as_deref()
            .map(str::trim)
            .filter(|h| !h.is_empty())
        {
            return Some(host.to_string());
        }
        self.lan_interfaces
            .first()
            .map(|iface| iface.ip.to_string())
    }

    pub fn set_server_lan_host(&mut self, host: Option<String>, cx: &mut Context<Self>) {
        let host = host.map(|h| h.trim().to_string()).filter(|h| !h.is_empty());
        self.server_lan_host = host.clone();
        {
            let store = lock_store(&self.store);
            let _ = store.set_setting("server_lan_host", host.as_deref().unwrap_or(""));
        }
        let label = host.unwrap_or_else(|| {
            self.lan_interfaces
                .first()
                .map(|iface| format!("自动（{}）", iface.ip))
                .unwrap_or_else(|| "自动".into())
        });
        self.show_toast(format!("局域网订阅地址已切换为 {label}"), cx);
        cx.notify();
    }

    fn load_templates(&mut self) {
        let templates = {
            let store = lock_store(&self.store);
            store.list_custom_templates().unwrap_or_default()
        };
        self.custom_templates = templates;
    }

    pub fn open_template_dialog(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.template_dialog_open {
            return;
        }
        if self.template_session_active {
            self.template_dialog_open = true;
            cx.notify();
            return;
        }
        self.template_base_options = Some(self.options.clone());
        self.template_baseline = Some(self.active_template_snapshot());
        self.template_edit_id = self.workspace.selected_template_id;
        self.template_session_active = true;
        self.template_dialog_open = true;
        self.reset_advanced_draft(window, cx);
        self.template_error = None;
        self.template_tab = TemplateTab::PolicyGroups;
        self.template_disabled_rule_sets = self.disabled_rule_sets.clone();
        self.template_custom_rule_sets = self.custom_rule_sets.clone();
        self.template_enabled_modules = self.enabled_modules.clone();
        self.template_policy_groups = self.options.policy_groups.clone();
        self.template_group_order = self.options.group_order.clone();
        for group in &mut self.template_policy_groups {
            group.ensure_id();
            if group.id.is_empty() {
                group.id = uuid::Uuid::new_v4().to_string();
            }
        }
        self.template_rule_order = self.options.rule_order.clone();
        self.template_rule_set_targets = self.options.rule_set_targets.clone();
        self.template_policy_edit = None;
        self.workspace.settings_loaded = false;
        self.template_policy_name
            .update(cx, |state, cx| state.set_value("", window, cx));
        let lines = self
            .options
            .custom_rules
            .iter()
            .map(Rule::to_line)
            .collect::<Vec<_>>()
            .join("\n");
        self.template_name
            .update(cx, |state, cx| state.set_value("", window, cx));
        self.template_rule_search
            .update(cx, |state, cx| state.set_value("", window, cx));
        self.template_rule_url
            .update(cx, |state, cx| state.set_value("", window, cx));
        self.template_custom_rules
            .update(cx, |state, cx| state.set_value(lines, window, cx));
        cx.notify();
    }

    pub fn close_template_dialog(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.workspace_close_rule_content(cx);
        if self.workspace.rule_content_id.is_some() {
            return;
        }
        self.workspace.rule_picker_open = false;
        self.workspace.rule_metadata_id = None;
        self.workspace.group_settings_open = false;
        self.workspace.rules_target = None;
        self.clear_saved_draft();
        self.recovered_draft = false;
        if let Some(baseline) = self.template_baseline.take() {
            self.custom_groups = baseline.custom_groups;
            self.chains = baseline.chains;
        }
        self.template_base_options = None;
        self.template_edit_id = None;
        self.template_session_active = false;
        self.template_dialog_open = false;
        self.template_error = None;
        self.template_tab = TemplateTab::PolicyGroups;
        self.template_disabled_rule_sets = self.disabled_rule_sets.clone();
        self.template_custom_rule_sets = self.custom_rule_sets.clone();
        self.template_enabled_modules = self.enabled_modules.clone();
        self.template_policy_groups = self.options.policy_groups.clone();
        self.template_group_order = self.options.group_order.clone();
        self.template_rule_order = self.options.rule_order.clone();
        self.template_rule_set_targets = self.options.rule_set_targets.clone();
        self.template_policy_edit = None;
        self.template_policy_name
            .update(cx, |state, cx| state.set_value("", window, cx));
        self.template_name
            .update(cx, |state, cx| state.set_value("", window, cx));
        self.template_rule_search
            .update(cx, |state, cx| state.set_value("", window, cx));
        self.template_rule_url
            .update(cx, |state, cx| state.set_value("", window, cx));
        self.template_custom_rules
            .update(cx, |state, cx| state.set_value("", window, cx));
        cx.notify();
    }

    pub fn reset_advanced_draft(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.refresh_advanced_draft(false, window, cx);
    }

    fn refresh_advanced_draft(
        &mut self,
        preserve_dirty: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let dirty = preserve_dirty
            && self.advanced_draft.read(cx).value().as_ref() != self.template_advanced_saved;
        let base = self.template_base_options.as_ref().unwrap_or(&self.options);
        let draft = AdvancedDraft {
            dns: base.dns.clone(),
            listeners: base.listeners.clone(),
            chains: self.chains.clone(),
            custom_groups: self.custom_groups.clone(),
            extra_yaml: base.extra_yaml.clone(),
        };
        if let Ok(text) = serde_json::to_string_pretty(&draft) {
            self.template_advanced_saved = text.clone();
            if !dirty {
                self.advanced_draft
                    .update(cx, |s, cx| s.set_value(text, window, cx));
            }
        }
    }

    pub fn update_advanced_draft(&mut self, cx: &mut Context<Self>) {
        let result = serde_json::from_str::<AdvancedDraft>(&self.advanced_draft.read(cx).value())
            .map_err(|e| e.to_string());
        let draft = match result {
            Ok(d) => d,
            Err(e) => {
                self.template_error = Some(format!("高级配置格式错误：{e}"));
                cx.notify();
                return;
            }
        };
        let mut options = match self.draft_generate_options(cx) {
            Ok(o) => o,
            Err(e) => {
                self.template_error = Some(e);
                cx.notify();
                return;
            }
        };
        options.dns = draft.dns.clone();
        options.listeners = draft.listeners.clone();
        options.extra_yaml = draft.extra_yaml.clone();
        options.chains = draft.chains.clone();
        options.custom_groups = draft.custom_groups.clone();
        let validation_nodes = if self.nodes.iter().any(|n| n.enabled) {
            self.nodes.clone()
        } else {
            build_nodes(
                vec![Proxy::new("direct", "模板校验节点", "127.0.0.1", 1)],
                "validation",
                "validation",
            )
        };
        let result = generate_profile(&validation_nodes, &options);
        if let Err(e) = result {
            self.template_error = Some(e.to_string());
            cx.notify();
            return;
        }
        let base = self
            .template_base_options
            .get_or_insert_with(|| self.options.clone());
        base.dns = draft.dns;
        base.listeners = draft.listeners;
        base.extra_yaml = draft.extra_yaml;
        self.chains = draft.chains;
        self.custom_groups = draft.custom_groups;
        self.template_advanced_saved = self.advanced_draft.read(cx).value().to_string();
        self.template_error = None;
        cx.notify();
    }

    pub fn add_template_rule_set_url(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let url = self.template_rule_url.read(cx).value().trim().to_string();
        if url.is_empty() {
            self.template_error = Some("请输入规则集链接".to_string());
            cx.notify();
            return;
        }
        match rule_set_binding_from_url(&url) {
            Ok(mut binding) => {
                if self
                    .template_custom_rule_sets
                    .iter()
                    .any(|item| item.url == binding.url)
                {
                    self.show_toast("此规则集链接已添加，可在列表中调整目标".into(), cx);
                    return;
                }
                if !self.template_rules_target_filter.is_empty() {
                    binding.target = self.template_rules_target_filter.clone();
                }
                let base_id = binding.id.clone();
                let mut suffix = 2;
                while self
                    .template_custom_rule_sets
                    .iter()
                    .any(|item| item.id == binding.id)
                {
                    binding.id = format!("{base_id}-{suffix}");
                    suffix += 1;
                }
                self.template_custom_rule_sets.push(binding);
                self.template_error = None;
                self.template_rule_url
                    .update(cx, |state, cx| state.set_value("", window, cx));
            }
            Err(error) => self.template_error = Some(error),
        }
        cx.notify();
    }

    fn normalize_rule_set_bindings(&self, bindings: &[RuleSetBinding]) -> Vec<RuleSetBinding> {
        let mut seen = HashSet::new();
        bindings
            .iter()
            .filter(|binding| seen.insert(binding.id.clone()))
            .map(|binding| {
                let mut binding = binding.clone();
                rule_catalog::refresh_binding_url(
                    &mut binding,
                    &self.options.rule_provider_base_url,
                );
                binding
            })
            .collect()
    }

    pub fn template_target_options(&self) -> Vec<String> {
        let mut names = vec!["DIRECT".into(), "REJECT".into(), "REJECT-DROP".into()];
        names.extend(
            catalog::MODULES
                .iter()
                .filter(|module| !matches!(module.kind, catalog::ModuleKind::Region(_)))
                .map(|module| module.name.to_string()),
        );
        names.extend(
            self.template_policy_groups
                .iter()
                .map(|group| group.name.clone()),
        );
        names.extend(self.custom_groups.iter().map(|group| group.name.clone()));
        names.extend(self.chains.iter().map(|chain| chain.name.clone()));
        let mut seen = HashSet::new();
        names.retain(|name| seen.insert(name.clone()));
        names
    }

    pub fn add_template_policy_group(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let name = self
            .template_policy_name
            .read(cx)
            .value()
            .trim()
            .to_string();
        let reserved = [
            "DIRECT",
            "REJECT",
            "REJECT-DROP",
            "PASS",
            "COMPATIBLE",
            "GLOBAL",
        ];
        if name.is_empty()
            || name.contains([',', '\n', '\r'])
            || reserved
                .iter()
                .any(|reserved| reserved.eq_ignore_ascii_case(&name))
            || self.template_target_options().contains(&name)
            || self.nodes.iter().any(|node| node.name() == name)
        {
            self.template_error = Some(
                "策略组名称不能为空或包含逗号、换行，也不能与现有组、节点或保留策略重名".into(),
            );
        } else {
            self.template_policy_groups.push(PolicyGroupSpec {
                id: uuid::Uuid::new_v4().to_string(),
                name,
                kind: GroupKind::Select,
                members: vec![
                    self.workspace_resolved_group_name("节点选择").into(),
                    self.workspace_resolved_group_name("自动选择").into(),
                    "DIRECT".into(),
                ],
                include_all_nodes: true,
                ..Default::default()
            });
            self.edit_template_policy_group(self.template_policy_groups.len() - 1, window, cx);
            self.template_error = None;
            self.template_policy_name
                .update(cx, |state, cx| state.set_value("", window, cx));
        }
        cx.notify();
    }

    pub fn edit_template_policy_group(
        &mut self,
        index: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(group) = self.template_policy_groups.get(index) {
            self.template_policy_edit = Some(index);
            self.workspace.member_source.clear();
            self.workspace.member_region.clear();
            self.template_member_search
                .update(cx, |input, cx| input.set_value("", window, cx));
            self.template_policy_rename
                .update(cx, |s, cx| s.set_value(group.name.clone(), window, cx));
            self.template_health_url.update(cx, |s, cx| {
                s.set_value(group.test_url.clone().unwrap_or_default(), window, cx)
            });
            self.template_health_interval.update(cx, |s, cx| {
                s.set_value(
                    group
                        .test_interval
                        .map(|n| n.to_string())
                        .unwrap_or_default(),
                    window,
                    cx,
                )
            });
            self.workspace.description.update(cx, |input, cx| {
                input.set_value(group.description.clone(), window, cx)
            });
            self.workspace.tolerance.update(cx, |input, cx| {
                input.set_value(
                    group.tolerance.map(|n| n.to_string()).unwrap_or_default(),
                    window,
                    cx,
                )
            });
        }
        cx.notify();
    }

    pub fn draft_generate_options(&self, cx: &Context<Self>) -> Result<GenerateOptions, String> {
        let mut options = self
            .template_base_options
            .clone()
            .unwrap_or_else(|| self.options.clone());
        options.enabled_modules = Some(self.template_enabled_modules.iter().cloned().collect());
        options.disabled_rule_sets = self.template_disabled_rule_sets.iter().cloned().collect();
        options.custom_rule_sets = self.template_custom_rule_sets.clone();
        options.custom_rules =
            parse_custom_rule_lines(&self.template_custom_rules.read(cx).value())?;
        options.policy_groups = self.template_policy_groups.clone();
        options.group_order = self.template_group_order.clone();
        options.rule_set_targets = self.template_rule_set_targets.clone();
        options.rule_order = self.template_rule_order.clone();
        options.custom_groups = self.custom_groups.clone();
        options.chains = self.chains.clone();
        if let Some(defaults) = &self.workspace.global_defaults {
            defaults.apply(&mut options);
        }
        Ok(options)
    }

    pub fn edit_builtin_policy_group(
        &mut self,
        name: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(index) = self
            .template_policy_groups
            .iter()
            .position(|g| g.name == name)
        {
            self.edit_template_policy_group(index, window, cx);
            return;
        }
        // Defaults are generated with the draft's modules but without any overrides.
        // Opening a built-in group must not parse unrelated, unfinished rule text.
        let options = workspace::builtin_group_options(
            self.template_base_options.as_ref().unwrap_or(&self.options),
            &self.template_enabled_modules,
            &name,
        );
        let nodes = if self.nodes.iter().any(|n| n.enabled) {
            self.nodes.clone()
        } else {
            build_nodes(
                vec![Proxy::new("direct", "模板校验节点", "127.0.0.1", 1)],
                "validation",
                "validation",
            )
        };
        let profile = match generate_profile(&nodes, &options) {
            Ok(profile) => profile,
            Err(error) => {
                self.template_error = Some(format!("策略组「{name}」无法打开：{error}"));
                cx.notify();
                return;
            }
        };
        if let Some(group) = profile.groups.iter().find(|g| g.name == name) {
            let all_nodes =
                catalog::module_by_name(&name).is_some_and(catalog::default_includes_all_nodes);
            self.template_policy_groups.push(PolicyGroupSpec {
                id: format!("builtin:{name}"),
                name,
                kind: group.kind,
                members: if all_nodes {
                    group
                        .proxies
                        .iter()
                        .filter(|member| {
                            !nodes.iter().any(|node| node.name() == *member)
                                && !self.chains.iter().any(|chain| chain.name == **member)
                        })
                        .map(|member| self.workspace_resolved_group_name(member).to_string())
                        .collect()
                } else {
                    group
                        .proxies
                        .iter()
                        .map(|member| self.workspace_resolved_group_name(member).to_string())
                        .collect()
                },
                include_all_nodes: all_nodes,
                builtin: true,
                ..Default::default()
            });
            self.edit_template_policy_group(self.template_policy_groups.len() - 1, window, cx);
        } else {
            self.template_error = Some(format!("策略组「{name}」没有匹配节点，请先启用节点或模块"));
            cx.notify();
        }
    }

    pub fn save_policy_details(
        &mut self,
        index: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(group) = self.template_policy_groups.get(index) else {
            return;
        };
        let old_name = group.name.clone();
        let name = self
            .template_policy_rename
            .read(cx)
            .value()
            .trim()
            .to_string();
        let url = self.template_health_url.read(cx).value().trim().to_string();
        let interval_text = self
            .template_health_interval
            .read(cx)
            .value()
            .trim()
            .to_string();
        let interval = if interval_text.is_empty() {
            None
        } else {
            match interval_text.parse::<u32>() {
                Ok(n) if n > 0 => Some(n),
                _ => {
                    self.template_error =
                        Some(format!("策略组「{old_name}」的测速间隔必须是正整数"));
                    cx.notify();
                    return;
                }
            }
        };
        let tolerance_text = self.workspace.tolerance.read(cx).value().trim().to_string();
        let tolerance = if tolerance_text.is_empty() {
            None
        } else {
            match tolerance_text.parse::<u32>() {
                Ok(value) => Some(value),
                Err(_) => {
                    self.template_error =
                        Some(format!("策略组「{old_name}」的测速容差必须是非负整数"));
                    cx.notify();
                    return;
                }
            }
        };
        if !url.is_empty()
            && !url::Url::parse(&url).is_ok_and(|url| {
                matches!(url.scheme(), "https" | "http") && url.host_str().is_some()
            })
        {
            self.template_error = Some(format!(
                "策略组「{old_name}」的测速地址无效，请填写 HTTP / HTTPS 地址"
            ));
            cx.notify();
            return;
        }
        if name.is_empty()
            || name.contains([',', '\n', '\r'])
            || (name != old_name
                && ((self.template_target_options().contains(&name)
                    && group.builtin_name() != Some(name.as_str()))
                    || [
                        "DIRECT",
                        "REJECT",
                        "REJECT-DROP",
                        "PASS",
                        "COMPATIBLE",
                        "GLOBAL",
                    ]
                    .iter()
                    .any(|reserved| reserved.eq_ignore_ascii_case(&name))
                    || self.nodes.iter().any(|n| n.name() == name)))
        {
            self.template_error = Some(format!(
                "策略组「{old_name}」的名称“{name}”重复或无效：名称不能为空、包含逗号或换行，或与现有组、节点及保留策略重名"
            ));
            cx.notify();
            return;
        }
        let group = &mut self.template_policy_groups[index];
        group.tolerance = tolerance;
        group.description = self.workspace.description.read(cx).value().to_string();
        group.test_url = (!url.is_empty()).then_some(url);
        group.test_interval = interval;
        if old_name != name
            && let Err(error) = self.rename_workspace_policy_group(index, name, window, cx)
        {
            self.template_error = Some(error);
            cx.notify();
            return;
        }
        self.template_error = None;
        cx.notify();
    }

    pub fn set_template_policy_kind(
        &mut self,
        index: usize,
        kind: GroupKind,
        cx: &mut Context<Self>,
    ) {
        if let Some(group) = self.template_policy_groups.get_mut(index) {
            group.kind = kind;
        }
        cx.notify();
    }

    pub fn remove_template_policy_group(&mut self, index: usize, cx: &mut Context<Self>) {
        if let Some(group) = self.template_policy_groups.get(index)
            && let Some(identity) = group.builtin_name()
        {
            let name = group.name.clone();
            match self.workspace_builtin_spec(identity) {
                Ok(mut restored) => {
                    restored.name = name;
                    self.template_policy_groups[index] = restored;
                    self.template_policy_edit = None;
                    self.workspace.group_settings_open = false;
                    self.template_error = None;
                }
                Err(error) => self.template_error = Some(error),
            }
            cx.notify();
            return;
        }
        if index < self.template_policy_groups.len() {
            let name = &self.template_policy_groups[index].name;
            let referenced = self
                .template_custom_rule_sets
                .iter()
                .any(|binding| &binding.target == name)
                || self
                    .template_rule_set_targets
                    .values()
                    .any(|target| target == name)
                || self
                    .template_policy_groups
                    .iter()
                    .enumerate()
                    .any(|(i, group)| i != index && group.members.contains(name))
                || parse_custom_rule_lines(&self.template_custom_rules.read(cx).value())
                    .is_ok_and(|rules| rules.iter().any(|rule| &rule.target == name));
            if referenced && !self.template_policy_groups[index].builtin {
                self.template_error = Some(format!(
                    "策略组「{name}」仍被规则或其他组引用，请先更改目标"
                ));
            } else {
                self.template_policy_groups.remove(index);
                self.template_policy_edit = None;
                self.template_error = None;
            }
        }
        cx.notify();
    }

    fn draft_template_snapshot(&self, cx: &Context<Self>) -> Result<TemplateSnapshot, String> {
        let base = self.template_base_options.as_ref().unwrap_or(&self.options);
        let rules = parse_custom_rule_lines(&self.template_custom_rules.read(cx).value())?;
        let mut enabled_modules: Vec<String> =
            self.template_enabled_modules.iter().cloned().collect();
        for id in ["select", "auto", "final"] {
            if !enabled_modules.iter().any(|module| module == id) {
                enabled_modules.push(id.into());
            }
        }
        enabled_modules.sort();
        let mut disabled_rule_sets: Vec<String> =
            self.template_disabled_rule_sets.iter().cloned().collect();
        disabled_rule_sets.sort();
        let snapshot = TemplateSnapshot {
            preset: base.preset,
            enabled_modules,
            disabled_rule_sets,
            custom_rule_sets: self.template_custom_rule_sets.clone(),
            custom_rule_lines: rules.iter().map(Rule::to_line).collect(),
            policy_groups: self.template_policy_groups.clone(),
            group_order: self.template_group_order.clone(),
            rule_order: self.template_rule_order.clone(),
            dns: base.dns.clone(),
            dns_override: base.dns_override,
            rule_set_overrides: base.rule_set_overrides.clone(),
            listeners: base.listeners.clone(),
            extra_yaml: base.extra_yaml.clone(),
            rule_set_targets: self.template_rule_set_targets.clone(),
            rule_provider_base_url: base.rule_provider_base_url.clone(),
            custom_groups: self.custom_groups.clone(),
            chains: self.chains.clone(),
            test_url: base.test_url.clone(),
            test_interval: base.test_interval,
            test_tolerance: base.test_tolerance,
            mixed_port: base.mixed_port,
            allow_lan: base.allow_lan,
        };
        self.validate_template_snapshot(&snapshot)?;
        Ok(snapshot)
    }

    fn validate_template_snapshot(&self, snapshot: &TemplateSnapshot) -> Result<(), String> {
        let rules = parse_custom_rule_lines(&snapshot.custom_rule_lines.join("\n"))?;
        let mut targets: HashSet<String> = [
            "DIRECT",
            "REJECT",
            "REJECT-DROP",
            "PASS",
            "COMPATIBLE",
            "GLOBAL",
            "节点选择",
            "自动选择",
            "漏网之鱼",
        ]
        .into_iter()
        .map(str::to_string)
        .collect();
        targets.extend(
            snapshot
                .enabled_modules
                .iter()
                .filter_map(|id| catalog::module(id))
                .filter(|module| !matches!(module.kind, catalog::ModuleKind::Region(_)))
                .map(|module| module.name.to_string()),
        );
        targets.extend(
            snapshot
                .policy_groups
                .iter()
                .map(|group| group.name.clone()),
        );
        targets.extend(
            snapshot
                .custom_groups
                .iter()
                .map(|group| group.name.clone()),
        );
        targets.extend(snapshot.chains.iter().map(|chain| chain.name.clone()));
        targets.extend(self.nodes.iter().map(|node| node.name().to_string()));
        for (label, target) in snapshot
            .custom_rule_sets
            .iter()
            .map(|binding| (binding.name.as_str(), binding.target.as_str()))
            .chain(
                rules
                    .iter()
                    .map(|rule| ("自定义规则", rule.target.as_str())),
            )
            .chain(
                snapshot
                    .rule_set_targets
                    .iter()
                    .filter(|(id, _)| !snapshot.disabled_rule_sets.contains(id))
                    .filter(|(id, _)| {
                        snapshot
                            .enabled_modules
                            .iter()
                            .filter_map(|id| catalog::module(id))
                            .any(|module| module.rule_sets.iter().any(|spec| spec.id == *id))
                    })
                    .map(|(id, target)| (id.as_str(), target.as_str())),
            )
        {
            if !targets.contains(target) {
                return Err(format!(
                    "「{label}」的策略组「{target}」未启用或不存在，请先调整目标"
                ));
            }
        }
        let options = snapshot.generate_options(&self.options)?;
        // Validate structure even when a template is edited before importing subscriptions.
        let mut nodes = self.nodes.clone();
        if !nodes.iter().any(|node| node.enabled) {
            nodes.extend(swiftsub_core::normalize::build_nodes(
                vec![Proxy::new("direct", "模板校验节点", "127.0.0.1", 1)],
                "validation",
                "validation",
            ));
        }
        self.generation_cache.generate(&nodes, &options)?;
        Ok(())
    }

    fn active_template_snapshot(&self) -> TemplateSnapshot {
        let mut enabled_modules: Vec<_> = self.enabled_modules.iter().cloned().collect();
        enabled_modules.sort();
        let mut disabled_rule_sets: Vec<_> = self.disabled_rule_sets.iter().cloned().collect();
        disabled_rule_sets.sort();
        TemplateSnapshot {
            preset: self.options.preset,
            enabled_modules,
            disabled_rule_sets,
            custom_rule_sets: self.custom_rule_sets.clone(),
            custom_rule_lines: self
                .options
                .custom_rules
                .iter()
                .map(Rule::to_line)
                .collect(),
            rule_provider_base_url: self.options.rule_provider_base_url.clone(),
            policy_groups: self.options.policy_groups.clone(),
            group_order: self.options.group_order.clone(),
            rule_set_targets: self.options.rule_set_targets.clone(),
            rule_order: self.options.rule_order.clone(),
            dns: self.options.dns.clone(),
            dns_override: self.options.dns_override,
            rule_set_overrides: self.options.rule_set_overrides.clone(),
            listeners: self.options.listeners.clone(),
            extra_yaml: self.options.extra_yaml.clone(),
            custom_groups: self.custom_groups.clone(),
            chains: self.chains.clone(),
            test_url: self.options.test_url.clone(),
            test_interval: self.options.test_interval,
            test_tolerance: self.options.test_tolerance,
            mixed_port: self.options.mixed_port,
            allow_lan: self.options.allow_lan,
        }
    }

    pub fn request_close_template(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        match self.autosave_routing(window, cx) {
            Ok(()) => {
                self.close_template_dialog(window, cx);
                return;
            }
            Err(e) => self.template_error = Some(e),
        }
        if let Err(error) = self.checkpoint_draft(cx) {
            self.show_toast(format!("编辑内容保存失败：{error}"), cx);
            return;
        }
        self.template_dialog_open = false;
        cx.notify();
    }

    pub(super) fn autosave_routing(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Result<(), String> {
        if !self.template_session_active {
            return Ok(());
        }
        if !self.finish_policy_details(window, cx) {
            return Err(self
                .template_error
                .clone()
                .unwrap_or_else(|| "策略组尚未填写完整".into()));
        }
        if self.advanced_draft.read(cx).value().as_ref() != self.template_advanced_saved {
            self.update_advanced_draft(cx);
            if let Some(error) = &self.template_error {
                return Err(error.clone());
            }
        }
        let snapshot = self.draft_template_snapshot(cx)?;
        if serde_json::to_value(&snapshot).ok()
            != serde_json::to_value(
                self.template_baseline
                    .clone()
                    .unwrap_or_else(|| self.active_template_snapshot()),
            )
            .ok()
        {
            self.persist_and_commit_template(snapshot, None, window, cx)?;
            self.template_baseline = Some(self.active_template_snapshot());
            self.template_base_options = Some(self.options.clone());
        }
        self.template_error = None;
        Ok(())
    }

    fn persist_and_commit_template(
        &mut self,
        snapshot: TemplateSnapshot,
        save: Option<(Option<i64>, String)>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Result<(), String> {
        self.persist_template_with_node_overrides(snapshot, save, &[], window, cx)
    }

    fn persist_template_with_node_overrides(
        &mut self,
        snapshot: TemplateSnapshot,
        save: Option<(Option<i64>, String)>,
        overrides: &[swiftsub_store::NodeOverride],
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Result<(), String> {
        self.validate_template_snapshot(&snapshot)?;
        let options = snapshot.generate_options(&self.options)?;
        let json = serde_json::to_string(&snapshot).map_err(|e| e.to_string())?;
        let value = serde_json::to_value(&snapshot).map_err(|e| e.to_string())?;
        let mut settings = HashMap::new();
        let Some(object) = value.as_object() else {
            return Err("模板快照序列化格式无效".into());
        };
        for (key, value) in object {
            settings.insert(
                key.clone(),
                if key == "rule_provider_base_url" || key == "test_url" {
                    value.as_str().unwrap_or_default().to_string()
                } else {
                    value.to_string()
                },
            );
        }
        // Keep the whole snapshot for deterministic startup recovery as fields evolve.
        settings.insert("routing_snapshot".into(), json.clone());
        settings.insert(
            "deleted_nodes".into(),
            serde_json::to_string(&self.deleted_nodes).map_err(|error| error.to_string())?,
        );
        settings.insert(
            "node_order".into(),
            serde_json::to_string(&self.node_order).map_err(|error| error.to_string())?,
        );
        settings.insert(
            format!("rule-visibility:{}", self.profile_id),
            serde_json::to_string(&self.workspace.rule_visibility)
                .map_err(|error| error.to_string())?,
        );
        let profile_json = self.profile_snapshot_json(snapshot.clone())?;
        let saved_template_id = lock_store(&self.store)
            .save_routing_profile_with_node_overrides(
                &settings,
                save.as_ref()
                    .map(|(id, name)| (*id, name.as_str(), json.as_str())),
                Some((self.profile_id, profile_json.as_str())),
                overrides,
            )
            .map_err(|e| format!("保存失败，配置未应用：{e}"))?;
        if save.is_some() {
            self.workspace.selected_template_id = saved_template_id;
            self.template_edit_id = saved_template_id;
        }
        self.options = options;
        self.enabled_modules = snapshot.enabled_modules.into_iter().collect();
        self.disabled_rule_sets = snapshot.disabled_rule_sets.into_iter().collect();
        self.custom_rule_sets = snapshot.custom_rule_sets;
        self.custom_groups = snapshot.custom_groups;
        self.chains = snapshot.chains;
        self.template_baseline = None;
        self.rule_provider_input.update(cx, |s, cx| {
            s.set_value(self.options.rule_provider_base_url.clone(), window, cx)
        });
        self.regenerate(cx);
        Ok(())
    }

    fn finish_policy_details(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        if let Some(index) = self.template_policy_edit {
            self.save_policy_details(index, window, cx);
            self.template_error.is_none()
        } else {
            true
        }
    }

    fn load_template_draft(
        &mut self,
        mut snapshot: TemplateSnapshot,
        name: String,
        id: Option<i64>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        snapshot.remove_legacy_regions();
        self.open_template_dialog(window, cx);
        let mut base_snapshot = snapshot.clone();
        if parse_custom_rule_lines(&base_snapshot.custom_rule_lines.join("\n")).is_err() {
            base_snapshot.custom_rule_lines.clear();
        }
        self.template_base_options = base_snapshot.generate_options(&self.options).ok();
        self.workspace.settings_loaded = false;
        self.template_edit_id = id;
        self.workspace.selected_template_id = id;
        self.template_enabled_modules = snapshot.enabled_modules.into_iter().collect();
        self.template_disabled_rule_sets = snapshot.disabled_rule_sets.into_iter().collect();
        self.template_custom_rule_sets = snapshot.custom_rule_sets;
        self.template_policy_groups = snapshot.policy_groups;
        self.template_group_order = snapshot.group_order;
        for group in &mut self.template_policy_groups {
            group.ensure_id();
            if group.id.is_empty() {
                group.id = uuid::Uuid::new_v4().to_string();
            }
        }
        self.template_policy_edit = None;
        self.template_rule_set_targets = snapshot.rule_set_targets;
        self.template_rule_order = snapshot.rule_order;
        self.custom_groups = snapshot.custom_groups;
        self.chains = snapshot.chains;
        self.template_name
            .update(cx, |s, cx| s.set_value(name, window, cx));
        self.template_custom_rules.update(cx, |s, cx| {
            s.set_value(snapshot.custom_rule_lines.join("\n"), window, cx)
        });
        self.workspace.rule_repair_open =
            parse_custom_rule_lines(&self.template_custom_rules.read(cx).value()).is_err();
        self.reset_advanced_draft(window, cx);
        self.page = Page::Generator;
        cx.notify();
    }

    pub fn delete_custom_template(&mut self, id: i64, cx: &mut Context<Self>) {
        let result = lock_store(&self.store).delete_custom_template(id);
        match result {
            Ok(()) => {
                self.load_templates();
                self.show_toast("模板已删除".into(), cx)
            }
            Err(e) => self.show_toast(format!("删除失败：{e}"), cx),
        }
    }

    pub fn set_preset(&mut self, preset: Preset, cx: &mut Context<Self>) {
        self.options.preset = preset;
        self.enabled_modules = catalog::modules_for_preset(preset)
            .into_iter()
            .map(|module| module.id.to_string())
            .collect();
        self.disabled_rule_sets.clear();
        self.custom_rule_sets.clear();
        self.options.rule_order.clear();
        self.options.custom_rules.clear();
        self.options.policy_groups.clear();
        self.options.group_order.clear();
        self.options.rule_set_targets.clear();
        self.save_module_config();
        self.save_rule_set_config();
        self.regenerate(cx);
        self.show_toast(format!("已切换为{}模板", preset.label()), cx);
    }

    fn save_module_config(&self) {
        let store = lock_store(&self.store);
        if let Ok(value) =
            serde_json::to_string(&self.enabled_modules.iter().cloned().collect::<Vec<_>>())
        {
            let _ = store.set_setting("enabled_modules", &value);
        }
        if let Ok(value) = serde_json::to_string(&self.options.preset) {
            let _ = store.set_setting("preset", &value);
        }
    }

    fn save_rule_set_config(&self) {
        let store = lock_store(&self.store);
        if let Ok(value) = serde_json::to_string(&self.options.group_order) {
            let _ = store.set_setting("group_order", &value);
        }
        if let Ok(value) = serde_json::to_string(&self.options.rule_order) {
            let _ = store.set_setting("rule_order", &value);
        }
        if let Ok(value) = serde_json::to_string(&self.options.policy_groups) {
            let _ = store.set_setting("policy_groups", &value);
        }
        if let Ok(value) = serde_json::to_string(&self.options.rule_set_targets) {
            let _ = store.set_setting("rule_set_targets", &value);
        }
        if let Ok(value) =
            serde_json::to_string(&self.disabled_rule_sets.iter().cloned().collect::<Vec<_>>())
        {
            let _ = store.set_setting("disabled_rule_sets", &value);
        }
        if let Ok(value) = serde_json::to_string(&self.custom_rule_sets) {
            let _ = store.set_setting("custom_rule_sets", &value);
        }
        let lines: Vec<String> = self
            .options
            .custom_rules
            .iter()
            .map(Rule::to_line)
            .collect();
        if let Ok(value) = serde_json::to_string(&lines) {
            let _ = store.set_setting("custom_rule_lines", &value);
        }
    }

    fn load_module_config(&mut self) {
        let store = lock_store(&self.store);
        if let Some(value) = store.get_setting("preset").ok().flatten()
            && let Ok(preset) = serde_json::from_str::<Preset>(&value)
        {
            self.options.preset = preset;
            self.enabled_modules = catalog::modules_for_preset(preset)
                .into_iter()
                .map(|module| module.id.to_string())
                .collect();
        }
        if let Some(value) = store.get_setting("enabled_modules").ok().flatten()
            && let Ok(items) = serde_json::from_str::<Vec<String>>(&value)
            && !items.is_empty()
        {
            self.enabled_modules = items.into_iter().collect();
        }
    }

    pub fn open_node_rename(
        &mut self,
        key: String,
        sub_id: String,
        name: String,
        yaml: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Ok(sub_id) = sub_id.parse::<i64>() else {
            self.show_toast("该节点不支持编辑".to_string(), cx);
            return;
        };
        self.node_rename_key = Some(key);
        self.node_rename_sub_id = Some(sub_id);
        self.node_rename_error = None;
        self.node_rename_input.update(cx, |state, cx| {
            state.set_value(name, window, cx);
        });
        self.node_edit_textarea.update(cx, |state, cx| {
            state.set_value(yaml, window, cx);
        });
        cx.notify();
    }

    pub fn close_node_rename(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.node_rename_key = None;
        self.node_rename_sub_id = None;
        self.node_rename_error = None;
        self.node_rename_input
            .update(cx, |state, cx| state.set_value("", window, cx));
        self.node_edit_textarea
            .update(cx, |state, cx| state.set_value("", window, cx));
        cx.notify();
    }

    pub fn save_node_rename(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.save_workspace_node_edit(window, cx);
    }

    pub fn toggle_node(
        &mut self,
        key: String,
        enabled: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.toggle_workspace_node(key, enabled, window, cx);
    }

    pub fn copy_server_url(&mut self, cx: &mut Context<Self>) {
        let id = if self.workspace.saved_editor_open {
            self.saved_config_id.clone()
        } else {
            self.generated_saved_config
                .as_ref()
                .map(|config| config.id.clone())
        };
        if let Some(id) = id {
            self.copy_config_subscription(id, cx);
        } else {
            self.show_toast("请先在生成页保存配置".into(), cx);
        }
    }

    fn prepare_yaml_output(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Result<String, String> {
        if !self.nodes.iter().any(|node| node.enabled) {
            return Err("请先添加并启用节点".into());
        }
        if let Err(error) = self.autosave_routing(window, cx) {
            if parse_custom_rule_lines(&self.template_custom_rules.read(cx).value()).is_err() {
                self.workspace.rule_repair_open = true;
                self.template_custom_rules
                    .read(cx)
                    .focus_handle(cx)
                    .focus(window, cx);
            } else if let Some(group) = self
                .template_policy_edit
                .and_then(|i| self.template_policy_groups.get(i))
            {
                self.workspace.group_settings_open = true;
                self.workspace
                    .reveal_group
                    .replace(Some(group.name.clone()));
                self.template_policy_rename
                    .read(cx)
                    .focus_handle(cx)
                    .focus(window, cx);
            }
            self.template_error = Some(error.clone());
            cx.notify();
            return Err(error);
        }
        self.workspace_apply_rule_drafts(window, cx)?;
        saved_configs::generate_yaml_output(&self.nodes, &self.draft_generate_options(cx)?)
    }

    pub fn show_toast(&mut self, message: String, cx: &mut Context<Self>) {
        self.toast = Some(message);
        self.toast_epoch = self.toast_epoch.wrapping_add(1);
        let epoch = self.toast_epoch;
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(Duration::from_secs(3)).await;
            if let Some(this) = this.upgrade() {
                this.update(cx, |this, cx| {
                    if this.toast_epoch == epoch {
                        this.toast = None;
                        cx.notify();
                    }
                });
            }
        })
        .detach();
        cx.notify();
    }

    fn refresh_server_url(&mut self) {
        if let Some(server) = &self.server {
            self.server_url =
                Some(server.profile_url(&self.profile_token, &self.subscription_filename));
        }
    }

    pub fn save_rule_provider(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let base = self
            .rule_provider_input
            .read(cx)
            .value()
            .trim()
            .trim_end_matches('/')
            .to_string();
        if !(base.starts_with("http://") || base.starts_with("https://")) {
            self.show_toast("规则集来源必须是 HTTP/HTTPS 地址".to_string(), cx);
            return;
        }
        self.options.rule_provider_base_url = base.clone();
        {
            let store = lock_store(&self.store);
            let _ = store.set_setting("rule_provider_base_url", &base);
        }
        self.rule_provider_input
            .update(cx, |state, cx| state.set_value(base, window, cx));
        self.regenerate(cx);
        self.show_toast("规则集来源已更新".to_string(), cx);
    }

    pub fn apply_rule_provider_preset(
        &mut self,
        base: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.rule_provider_input.update(cx, |state, cx| {
            state.set_value(base.to_string(), window, cx)
        });
        self.save_rule_provider(window, cx);
    }

    pub fn aggregate_userinfo(&self) -> Option<String> {
        let mut info = SubscriptionInfo::default();
        let mut has_data = false;
        let mut has_expire = false;

        for subscription in self
            .subscriptions
            .iter()
            .filter(|subscription| subscription.enabled && self.profile_includes(subscription.id))
        {
            if subscription.traffic_upload > 0
                || subscription.traffic_download > 0
                || subscription.traffic_total > 0
            {
                info.upload = info.upload.saturating_add(subscription.traffic_upload);
                info.download = info.download.saturating_add(subscription.traffic_download);
                info.total = info.total.saturating_add(subscription.traffic_total);
                has_data = true;
            }
            if let Some(expire) = subscription.expire_at.filter(|expire| *expire > 0) {
                info.expire = Some(match info.expire {
                    Some(current) => current.min(expire),
                    None => expire,
                });
                has_expire = true;
            }
        }

        (has_data || has_expire).then(|| info.to_header_value())
    }
}

pub fn data_dir() -> PathBuf {
    if let Ok(explicit) = std::env::var("SWIFTSUB_DATA_DIR") {
        let explicit = explicit.trim();
        if !explicit.is_empty() {
            return PathBuf::from(explicit);
        }
    }
    let portable = portable_data_dir();
    if let Some(dir) = portable {
        if let Ok(location) = std::fs::read_to_string(dir.join("location.txt")) {
            let location = location.trim();
            if !location.is_empty() {
                return PathBuf::from(location);
            }
        }
        return dir;
    }
    let fallback = fallback_appdata_dir();
    if let Ok(location) = std::fs::read_to_string(fallback.join("location.txt")) {
        let location = location.trim();
        if !location.is_empty() {
            return PathBuf::from(location);
        }
    }
    fallback
}

fn portable_data_dir() -> Option<PathBuf> {
    let exe_dir = std::env::current_exe().ok()?.parent()?.to_path_buf();
    if exe_dir.join(".installed").exists() {
        return None;
    }
    let candidate = exe_dir.join("data");
    if std::fs::create_dir_all(&candidate).is_ok() {
        let probe = candidate.join(".write-probe");
        if std::fs::write(&probe, b"").is_ok() {
            let _ = std::fs::remove_file(&probe);
            return Some(candidate);
        }
    }
    None
}

fn fallback_appdata_dir() -> PathBuf {
    if let Ok(appdata) = std::env::var("APPDATA") {
        PathBuf::from(appdata).join("SwiftSub")
    } else {
        PathBuf::from(".swiftsub")
    }
}

fn split_keywords(value: &str) -> Vec<String> {
    value
        .split([',', '，', ';', '；'])
        .map(str::trim)
        .filter(|keyword| !keyword.is_empty())
        .map(str::to_string)
        .collect()
}

fn is_node_link(text: &str) -> bool {
    text.split_once("://")
        .is_some_and(|(scheme, _)| !matches!(scheme.to_lowercase().as_str(), "http" | "https"))
}

fn merge_manual_proxies_restoring_deleted(
    existing_payload: Option<&str>,
    new_proxies: Vec<Proxy>,
    deleted_nodes: &mut HashSet<String>,
    visible_keys: &HashSet<String>,
) -> Result<(String, usize, usize, usize), String> {
    let mut existing = Vec::new();
    if let Some(payload) = existing_payload.filter(|payload| !payload.trim().is_empty()) {
        existing.extend(
            parse_content(payload)
                .map_err(|error| error.to_string())?
                .proxies,
        );
    }
    // Filter out previously deleted entries from the manual subscription payload so they
    // no longer cause false "already exists" collisions or inflate manual node counts.
    existing.retain(|proxy| !deleted_nodes.contains(&proxy.identity_key()));
    let (mut kept, _) = dedupe_proxies(existing);
    let mut active_keys: HashSet<String> = kept
        .iter()
        .map(Proxy::identity_key)
        .chain(visible_keys.iter().cloned())
        .collect();
    let mut added = 0usize;
    let mut duplicates = 0usize;
    for proxy in new_proxies {
        let key = proxy.identity_key();
        deleted_nodes.remove(&key);
        if active_keys.insert(key) {
            kept.push(proxy);
            added += 1;
        } else {
            duplicates += 1;
        }
    }
    let payload = proxies_to_clash_yaml(&kept).map_err(|error| error.to_string())?;
    Ok((payload, added, duplicates, kept.len()))
}

fn remove_manual_proxy_from_payload(
    existing_payload: Option<&str>,
    key: &str,
    deleted_nodes: &HashSet<String>,
) -> Result<Option<(String, usize)>, String> {
    let Some(payload) = existing_payload.filter(|payload| !payload.trim().is_empty()) else {
        return Ok(None);
    };
    let mut proxies = parse_content(payload)
        .map_err(|error| error.to_string())?
        .proxies;
    let before = proxies.len();
    proxies.retain(|proxy| {
        let id = proxy.identity_key();
        id != key && !deleted_nodes.contains(&id)
    });
    if proxies.len() == before {
        return Ok(None);
    }
    let updated = proxies_to_clash_yaml(&proxies).map_err(|error| error.to_string())?;
    Ok(Some((updated, proxies.len())))
}

pub fn default_subscription_name(url: &str) -> String {
    let host = url
        .split_once("://")
        .map(|(_, rest)| rest)
        .unwrap_or(url)
        .split('/')
        .next()
        .unwrap_or("订阅")
        .trim_start_matches("www.");
    if host.is_empty() {
        "新订阅".to_string()
    } else {
        host.to_string()
    }
}

pub fn format_timestamp(timestamp: Option<i64>) -> String {
    let Some(timestamp) = timestamp else {
        return "从未更新".to_string();
    };
    let Some(datetime) = Local.timestamp_opt(timestamp, 0).single() else {
        return "—".to_string();
    };
    datetime.format("%m-%d %H:%M").to_string()
}

#[cfg(test)]
mod routing_template_tests {
    use super::*;
    use serde_json::json;

    fn legacy_snapshot() -> serde_json::Value {
        json!({
            "preset":"standard", "enabled_modules":["select","auto","final","netflix"],
            "custom_groups":[], "chains":[], "test_url":"https://example.com/204",
            "test_interval":123, "mixed_port":7897, "allow_lan":false
        })
    }

    #[test]
    fn old_templates_load_without_new_policy_fields() {
        let snapshot: TemplateSnapshot = serde_json::from_value(legacy_snapshot()).unwrap();
        assert!(snapshot.policy_groups.is_empty());
        assert!(snapshot.group_order.is_empty());
        assert!(snapshot.rule_set_targets.is_empty());
        assert!(
            snapshot
                .generate_options(&GenerateOptions::default())
                .is_ok()
        );
    }

    #[test]
    fn template_roundtrip_preserves_policy_groups_members_and_all_rule_targets() {
        let mut json = legacy_snapshot();
        json["policy_groups"] = json!([
            {"name":"流媒体", "kind":"select", "members":["自动选择","DIRECT"], "include_all_nodes":false},
            {"name":"测速专线", "kind":"url-test", "members":[], "include_all_nodes":true}
        ]);
        json["rule_set_targets"] = json!({"netflix":"流媒体", "netflix-ip":"测速专线"});
        json["custom_rule_sets"] = json!([{
            "id":"bm-netflix", "name":"Netflix", "url":"https://example.com/Netflix.yaml",
            "format":"yaml", "behavior":"classical", "target":"流媒体", "no_resolve":false
        }]);
        json["custom_rule_lines"] = json!(["DOMAIN-SUFFIX,example.org,测速专线"]);
        json["group_order"] = json!(["流媒体", "测速专线", "漏网之鱼"]);
        let snapshot: TemplateSnapshot = serde_json::from_value(json).unwrap();
        let restored: TemplateSnapshot =
            serde_json::from_str(&serde_json::to_string(&snapshot).unwrap()).unwrap();
        assert_eq!(snapshot.policy_groups, restored.policy_groups);
        assert_eq!(snapshot.group_order, restored.group_order);
        assert_eq!(snapshot.rule_set_targets, restored.rule_set_targets);
        let parsed = parse_content("trojan://pass@example.com:443#test").unwrap();
        let nodes = build_nodes(parsed.proxies, "fixture", "fixture");
        let profile = generate_profile(
            &nodes,
            &restored
                .generate_options(&GenerateOptions::default())
                .unwrap(),
        )
        .unwrap();
        assert_eq!(
            profile
                .groups
                .iter()
                .take(3)
                .map(|group| group.name.as_str())
                .collect::<Vec<_>>(),
            ["流媒体", "测速专线", "漏网之鱼"]
        );
        for rule in [
            "RULE-SET,netflix,流媒体",
            "RULE-SET,netflix-ip,测速专线,no-resolve",
            "RULE-SET,bm-netflix,流媒体",
            "DOMAIN-SUFFIX,example.org,测速专线",
        ] {
            assert!(profile.yaml.contains(rule), "{rule}");
        }
        assert_eq!(
            template_summary_counts(&serde_json::to_string(&restored).unwrap()),
            (6, 3, 1)
        );
        assert!(profile.yaml.contains("interval: 123"));
    }

    #[test]
    fn stale_template_target_is_rejected_without_fallback_or_mutating_source_options() {
        let mut json = legacy_snapshot();
        json["rule_set_targets"] = json!({"netflix":"已删除策略组"});
        let snapshot: TemplateSnapshot = serde_json::from_value(json).unwrap();
        let base = GenerateOptions::default();
        let parsed = parse_content("trojan://pass@example.com:443#test").unwrap();
        let nodes = build_nodes(parsed.proxies, "fixture", "fixture");
        let error =
            generate_profile(&nodes, &snapshot.generate_options(&base).unwrap()).unwrap_err();
        assert!(error.to_string().contains("已删除策略组"));
        assert!(base.rule_set_targets.is_empty());
        assert!(base.policy_groups.is_empty());
    }
    #[test]
    fn template_file_roundtrip_and_version_rejection() {
        let config: TemplateSnapshot = serde_json::from_value(legacy_snapshot()).unwrap();
        let file = TemplateFile {
            format: "swiftsub-routing-template".into(),
            version: 1,
            name: "测试模板".into(),
            config,
        };
        let text = serde_json::to_string(&file).unwrap();
        let restored = TemplateFile::decode(&text).unwrap();
        assert_eq!(restored.name, "测试模板");
        assert_eq!(restored.config.test_interval, 123);
        let mut invalid: serde_json::Value = serde_json::from_str(&text).unwrap();
        invalid["version"] = json!(2);
        assert!(TemplateFile::decode(&invalid.to_string()).is_err());
        invalid["version"] = json!(1);
        invalid["format"] = json!("other");
        assert!(TemplateFile::decode(&invalid.to_string()).is_err());
        assert!(TemplateFile::decode("{}").is_err());
    }

    #[test]
    fn manual_node_delete_and_readd_does_not_report_duplicate_or_stay_hidden() {
        let link = "trojan://pass@node-a.example.org:443#HK-01";
        let first_proxies = parse_content(link).unwrap().proxies;
        let key = first_proxies[0].identity_key();
        let mut deleted = HashSet::new();
        let visible = HashSet::new();

        // First add: 1 added, 0 duplicates, total 1.
        let (payload, added, duplicates, total) = merge_manual_proxies_restoring_deleted(
            None,
            first_proxies.clone(),
            &mut deleted,
            &visible,
        )
        .unwrap();
        assert_eq!((added, duplicates, total), (1, 0, 1));

        // Re-adding while still active reports duplicate.
        let (_, added_dup, dup_count, total_dup) = merge_manual_proxies_restoring_deleted(
            Some(&payload),
            first_proxies.clone(),
            &mut deleted,
            &HashSet::from([key.clone()]),
        )
        .unwrap();
        assert_eq!((added_dup, dup_count, total_dup), (0, 1, 1));

        // Delete the manual node from the manual payload.
        let (after_delete, remaining) =
            remove_manual_proxy_from_payload(Some(&payload), &key, &deleted)
                .unwrap()
                .unwrap();
        assert_eq!(remaining, 0);

        // Re-adding the same node succeeds as 1 added, 0 duplicates.
        let (readded_payload, added_again, dup_again, total_again) =
            merge_manual_proxies_restoring_deleted(
                Some(&after_delete),
                first_proxies.clone(),
                &mut deleted,
                &HashSet::new(),
            )
            .unwrap();
        assert_eq!((added_again, dup_again, total_again), (1, 0, 1));
        assert_eq!(parse_content(&readded_payload).unwrap().proxies.len(), 1);
        assert!(!deleted.contains(&key));
    }

    #[test]
    fn legacy_deleted_manual_node_in_deleted_set_can_be_readded_cleanly() {
        // Simulate state produced before this fix: node still present in manual payload
        // AND recorded in deleted_nodes.
        let link = "trojan://pass@legacy.example.org:443#Legacy-01";
        let proxies = parse_content(link).unwrap().proxies;
        let key = proxies[0].identity_key();
        let payload = proxies_to_clash_yaml(&proxies).unwrap();
        let mut deleted = HashSet::from([key.clone()]);

        let (merged, added, duplicates, total) = merge_manual_proxies_restoring_deleted(
            Some(&payload),
            proxies,
            &mut deleted,
            &HashSet::new(),
        )
        .unwrap();
        assert_eq!((added, duplicates, total), (1, 0, 1));
        assert!(!deleted.contains(&key));
        assert_eq!(parse_content(&merged).unwrap().proxies.len(), 1);
    }
}
