use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use chrono::{Local, TimeZone};
use gpui_kit::component::input::{InputEvent, InputState, TextareaState};
use gpui_kit::component::{Theme, ThemeMode};
use gpui_kit::{
    AppContext as _, ClipboardItem, Context, Entity, ScrollHandle, Window, WindowAppearance, point,
    px,
};
use serde::{Deserialize, Serialize};
use swiftsub_core::catalog::{self, Preset};
use swiftsub_core::generator::RuleSetBinding;
use swiftsub_core::model::{
    ChainSpec, CustomGroupSpec, GroupKind, Node, NodeFilter, PolicyGroupSpec, Proxy, Rule,
};
use swiftsub_core::normalize::{apply_name_template, build_nodes, merge_manual_proxies_yaml};
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Page {
    Subscriptions,
    Generator,
    Templates,
    Settings,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TemplateTab {
    PolicyGroups,
    RuleSets,
    CustomRules,
    RuleOrder,
    Changes,
    Advanced,
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
    listeners: Vec<swiftsub_core::generator::ListenerSpec>,
    #[serde(default)]
    extra_yaml: String,
    #[serde(default)]
    rule_provider_base_url: String,
    #[serde(default)]
    policy_groups: Vec<PolicyGroupSpec>,
    #[serde(default)]
    rule_set_targets: HashMap<String, String>,
    #[serde(default)]
    custom_groups: Vec<CustomGroupSpec>,
    #[serde(default)]
    chains: Vec<ChainSpec>,
    test_url: String,
    test_interval: u32,
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
        options.rule_order = self.rule_order.clone();
        options.dns = self.dns.clone();
        options.listeners = self.listeners.clone();
        options.extra_yaml = self.extra_yaml.clone();
        options.rule_set_targets = self.rule_set_targets.clone();
        options.test_url = self.test_url.clone();
        options.test_interval = self.test_interval;
        options.mixed_port = self.mixed_port;
        options.allow_lan = self.allow_lan;
        if !self.rule_provider_base_url.is_empty() {
            options.rule_provider_base_url = self.rule_provider_base_url.clone();
        }
        Ok(options)
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

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct TemplateFile {
    format: String,
    version: u32,
    name: String,
    config: TemplateSnapshot,
}

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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AddMode {
    Subscription,
    Node,
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
            Err(_) => {
                return Err(format!("第 {} 行规则格式无效：{line}", index + 1));
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
    let rule_sets = snapshot
        .enabled_modules
        .iter()
        .filter_map(|id| catalog::module(id))
        .flat_map(|module| module.rule_sets.iter())
        .filter(|spec| !disabled.contains(spec.id))
        .count()
        + snapshot.custom_rule_sets.len();
    (
        snapshot.enabled_modules.len()
            + snapshot.custom_groups.len()
            + snapshot.policy_groups.len()
            + snapshot.chains.len(),
        rule_sets,
        snapshot.custom_rule_lines.len(),
    )
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CustomDialog {
    None,
    Filter,
    Chain,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ChainSide {
    Entry,
    Landing,
}

pub struct SwiftSubApp {
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
    pub diagnostics_open: bool,
    pub generation_report: String,
    pub search: Entity<InputState>,
    pub add_open: bool,
    pub add_mode: AddMode,
    pub add_name: Entity<InputState>,
    pub add_url: Entity<InputState>,
    pub add_error: Option<String>,
    pub batch_import_open: bool,
    pub batch_import: Entity<TextareaState>,
    pub batch_error: Option<String>,
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
    pub template_name: Entity<InputState>,
    pub template_error: Option<String>,
    pub template_dialog_open: bool,
    pub template_edit_id: Option<i64>,
    pub template_discard_armed: bool,
    template_baseline: Option<TemplateSnapshot>,
    template_base_options: Option<GenerateOptions>,
    pub batch_rename_open: bool,
    pub rename_template: Entity<InputState>,
    pub rename_error: Option<String>,
    pub confirm_delete: Option<i64>,
    pub edit_subscription: Option<i64>,
    pub edit_data: Option<StoredSubscription>,
    pub edit_nodes: Vec<(String, String, String)>,
    pub edit_name: Entity<InputState>,
    pub edit_url: Entity<InputState>,
    pub edit_interval: Entity<InputState>,
    pub edit_ua: Entity<InputState>,
    pub edit_enabled: bool,
    pub edit_error: Option<String>,
    pub refreshing: HashSet<i64>,
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
    pub template_rule_set_targets: HashMap<String, String>,
    pub template_policy_name: Entity<InputState>,
    pub template_policy_rename: Entity<InputState>,
    pub template_health_url: Entity<InputState>,
    pub template_health_interval: Entity<InputState>,
    pub template_member_search: Entity<InputState>,
    pub template_rule_order: Vec<String>,
    pub template_policy_edit: Option<usize>,
    pub template_target_picker: Option<String>,
    pub template_rule_search: Entity<InputState>,
    pub template_rule_url: Entity<InputState>,
    pub template_custom_rules: Entity<TextareaState>,
    pub advanced_draft: Entity<TextareaState>,
    pub template_tab: TemplateTab,
    pub template_rule_sources_collapsed: HashSet<String>,
    pub module_categories_collapsed: HashSet<String>,
    pub profile_id: i64,
    pub profile_token: String,
    pub server: Option<LocalServer>,
    pub server_url: Option<String>,
    pub server_error: Option<String>,
    pub preview: String,
    pub preview_open: bool,
    pub nodes_panel_open: bool,
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
    pub preview_scroll: ScrollHandle,
    pub settings_scroll: ScrollHandle,
    pub _subscriptions: Vec<gpui_kit::Subscription>,
}

impl SwiftSubApp {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let data_dir = data_dir();
        let db_path = data_dir.join("swiftsub.db");
        let (store, store_error) = match Store::open(&db_path) {
            Ok(store) => (store, None),
            Err(error) => (
                Store::open_in_memory().expect("in-memory store should open"),
                Some(format!("数据目录不可用，当前使用临时存储：{error}")),
            ),
        };
        let store = Arc::new(Mutex::new(store));
        let light_theme = {
            let guard = store.lock().expect("store mutex poisoned");
            guard.get_setting("theme").ok().flatten().as_deref() == Some("light")
        };

        let (profile_id, profile_token) = {
            let guard = store.lock().expect("store mutex poisoned");
            match guard.get_or_create_profile("默认配置") {
                Ok(profile) => (profile.id, profile.token),
                Err(_) => (0, "swiftsub-local".to_string()),
            }
        };
        let (subscription_filename, rule_provider_base_url) = {
            let guard = store.lock().expect("store mutex poisoned");
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
            InputState::new(window, cx)
                .placeholder("https://... 或 ss:// / vmess:// / vless:// ...")
        });
        let batch_import = cx.new(|cx| {
            TextareaState::new(window, cx)
                .placeholder("每行一个节点链接，也可直接粘贴 Clash YAML 或 Base64 订阅内容")
        });
        let edit_name = cx.new(|cx| InputState::new(window, cx).placeholder("订阅名称"));
        let edit_url = cx.new(|cx| InputState::new(window, cx).placeholder("订阅链接"));
        let edit_interval =
            cx.new(|cx| InputState::new(window, cx).placeholder("刷新间隔（分钟）"));
        let edit_ua = cx.new(|cx| InputState::new(window, cx).placeholder("User-Agent（可选）"));
        let node_rename_input = cx.new(|cx| InputState::new(window, cx).placeholder("节点名称"));
        let node_edit_textarea =
            cx.new(|cx| TextareaState::new(window, cx).placeholder("节点配置 YAML"));
        let template_name = cx.new(|cx| InputState::new(window, cx).placeholder("规则集模板名称"));
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
        let rename_template =
            cx.new(|cx| InputState::new(window, cx).placeholder("{region} {index:02}"));
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

        let enabled_modules: HashSet<String> = catalog::modules_for_preset(Preset::Standard)
            .into_iter()
            .map(|module| module.id.to_string())
            .collect();
        let node_order = {
            let guard = store.lock().expect("store mutex poisoned");
            guard
                .get_setting("node_order")
                .ok()
                .flatten()
                .and_then(|value| serde_json::from_str::<Vec<String>>(&value).ok())
                .unwrap_or_default()
        };
        let disabled_rule_sets = {
            let guard = store.lock().expect("store mutex poisoned");
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
            let guard = store.lock().expect("store mutex poisoned");
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
            let guard = store.lock().expect("store mutex poisoned");
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
        let rule_order = store
            .lock()
            .expect("store mutex poisoned")
            .get_setting("rule_order")
            .ok()
            .flatten()
            .and_then(|v| serde_json::from_str(&v).ok())
            .unwrap_or_default();
        let options = GenerateOptions {
            rule_order,
            policy_groups,
            rule_set_targets,
            rule_provider_base_url,
            custom_rule_sets: custom_rule_sets.clone(),
            custom_rules,
            ..Default::default()
        };

        let mut app = Self {
            page: Page::Subscriptions,
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
            diagnostics_open: false,
            generation_report: String::new(),
            search,
            add_open: false,
            add_mode: AddMode::Subscription,
            add_name,
            add_url,
            add_error: None,
            batch_import_open: false,
            batch_import,
            batch_error: None,
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
            template_name,
            template_error: None,
            template_dialog_open: false,
            template_edit_id: None,
            template_discard_armed: false,
            template_baseline: None,
            template_base_options: None,
            batch_rename_open: false,
            rename_template,
            rename_error: None,
            confirm_delete: None,
            edit_subscription: None,
            edit_data: None,
            edit_nodes: Vec::new(),
            edit_name,
            edit_url,
            edit_interval,
            edit_ua,
            edit_enabled: true,
            edit_error: None,
            refreshing: HashSet::new(),
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
            template_rule_set_targets: HashMap::new(),
            template_policy_name,
            template_policy_rename,
            template_health_url,
            template_health_interval,
            template_member_search,
            template_rule_order: Vec::new(),
            template_policy_edit: None,
            template_target_picker: None,
            template_rule_search,
            template_rule_url,
            template_custom_rules,
            advanced_draft,
            template_tab: TemplateTab::RuleSets,
            template_rule_sources_collapsed: rule_catalog::SOURCES
                .iter()
                .map(|source| source.id.to_string())
                .chain(std::iter::once("builtin".to_string()))
                .collect(),
            module_categories_collapsed: HashSet::from([
                "region".to_string(),
                "service".to_string(),
            ]),
            profile_id,
            profile_token,
            server: None,
            server_url: None,
            server_error: None,
            preview: String::new(),
            preview_open: false,
            nodes_panel_open: true,
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
            preview_scroll: ScrollHandle::default(),
            settings_scroll: ScrollHandle::default(),
            _subscriptions: Vec::new(),
        };

        let mode = if app.light_theme {
            ThemeMode::Light
        } else {
            ThemeMode::Dark
        };
        Theme::change(mode, None, cx);
        crate::configure_theme(cx);
        crate::ui::set_light_theme(app.light_theme);
        cx.set_window_appearance(Some(if app.light_theme {
            WindowAppearance::Light
        } else {
            WindowAppearance::Dark
        }));

        let add_url_sub = cx.subscribe_in(
            &app.add_url,
            window,
            |this, _, event: &InputEvent, window, cx| {
                if matches!(event, InputEvent::PressEnter { .. }) {
                    this.submit_add(window, cx);
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
        let template_rule_search_obs =
            cx.observe_in(&app.template_rule_search, window, |_, _, _, cx| cx.notify());
        app._subscriptions = vec![
            add_url_sub,
            add_name_sub,
            search_obs,
            chain_search_obs,
            template_rule_search_obs,
            member_search_obs,
        ];

        let args: Vec<String> = std::env::args().collect();
        let demo_mode = args.iter().any(|arg| arg == "--demo");
        let page_override = args
            .iter()
            .find_map(|arg| arg.strip_prefix("--page=").map(str::to_string));
        let dialog_override = args
            .iter()
            .find_map(|arg| arg.strip_prefix("--dialog=").map(str::to_string));

        app.load_custom_config();
        let recovered = app
            .store
            .lock()
            .expect("store mutex poisoned")
            .get_setting("routing_snapshot")
            .ok()
            .flatten()
            .and_then(|v| serde_json::from_str::<TemplateSnapshot>(&v).ok());
        if let Some(snapshot) = recovered {
            // Existing individual settings remain authoritative for legacy UI actions.
            app.options.test_url = snapshot.test_url;
            app.options.test_interval = snapshot.test_interval;
            app.options.mixed_port = snapshot.mixed_port;
            app.options.allow_lan = snapshot.allow_lan;
            app.options.dns = snapshot.dns;
            app.options.listeners = snapshot.listeners;
            app.options.extra_yaml = snapshot.extra_yaml;
        }
        app.load_module_config();
        app.load_templates();
        app.reload_subscriptions(cx);
        if demo_mode {
            if app.subscriptions.is_empty() {
                app.load_demo(cx);
            } else {
                app.page = Page::Generator;
            }
        }
        if let Some(page) = page_override {
            app.page = match page.as_str() {
                "generator" => Page::Generator,
                "templates" => Page::Templates,
                "settings" => Page::Settings,
                _ => Page::Subscriptions,
            };
        }
        if let Some(dialog) = dialog_override {
            match dialog.as_str() {
                "add" => {
                    app.page = Page::Subscriptions;
                    app.add_open = true;
                }
                "filter" => {
                    app.page = Page::Generator;
                    app.custom_dialog = CustomDialog::Filter;
                }
                "chain" => {
                    app.page = Page::Generator;
                    app.custom_dialog = CustomDialog::Chain;
                }
                "batch" => {
                    app.page = Page::Subscriptions;
                    app.batch_import_open = true;
                }
                "edit" => {
                    app.page = Page::Subscriptions;
                    if let Some(id) = app
                        .subscriptions
                        .first()
                        .map(|subscription| subscription.id)
                    {
                        app.open_edit_subscription(id, window, cx);
                    }
                }
                "rename" => {
                    app.page = Page::Generator;
                    app.batch_rename_open = true;
                }
                "rules" => {
                    app.page = Page::Generator;
                    app.open_template_dialog(window, cx);
                }
                _ => {}
            }
        }
        app.recount_manual_nodes();
        app.start_server();
        app.start_scheduler(cx);
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
            let store = self.store.lock().expect("store mutex poisoned");
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

    fn start_scheduler(&mut self, cx: &mut Context<Self>) {
        cx.spawn(async move |this, cx| {
            let mut first_tick = true;
            loop {
                let delay = if first_tick {
                    Duration::from_secs(3)
                } else {
                    Duration::from_secs(60)
                };
                first_tick = false;
                cx.background_executor().timer(delay).await;
                let Some(this) = this.upgrade() else {
                    break;
                };
                let due = this.update(cx, |this, _| this.due_subscription_ids());
                for id in due {
                    this.update(cx, |this, cx| this.refresh_subscription(id, cx));
                }
            }
        })
        .detach();
    }

    fn due_subscription_ids(&self) -> Vec<i64> {
        let now = swiftsub_store::now_ts();
        self.subscriptions
            .iter()
            .filter(|subscription| {
                subscription.enabled
                    && subscription.url.is_some()
                    && !self.refreshing.contains(&subscription.id)
            })
            .filter(|subscription| {
                if self
                    .retry_after
                    .get(&subscription.id)
                    .is_some_and(|until| now < *until)
                {
                    return false;
                }
                subscription.last_ok_at.is_none_or(|last| {
                    now.saturating_sub(last) >= i64::from(subscription.refresh_minutes.max(15)) * 60
                })
            })
            .map(|subscription| subscription.id)
            .take(2)
            .collect()
    }

    pub fn set_theme(&mut self, light: bool, cx: &mut Context<Self>) {
        if self.light_theme == light {
            return;
        }
        self.light_theme = light;
        {
            let store = self.store.lock().expect("store mutex poisoned");
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
        crate::configure_theme(cx);
        crate::ui::set_light_theme(light);
        cx.set_window_appearance(Some(if light {
            WindowAppearance::Light
        } else {
            WindowAppearance::Dark
        }));
        cx.notify();
    }

    pub fn set_page(&mut self, page: Page, cx: &mut Context<Self>) {
        if self.template_dialog_open && page != Page::Generator {
            self.show_toast("请先应用或取消当前分流草稿".into(), cx);
            return;
        }
        if page != self.page {
            match page {
                Page::Generator => self.preview_scroll.set_offset(point(px(0.0), px(0.0))),
                Page::Settings => self.settings_scroll.set_offset(point(px(0.0), px(0.0))),
                Page::Subscriptions | Page::Templates => {}
            }
        }
        self.page = page;
        cx.notify();
    }

    pub fn open_add_dialog(&mut self, cx: &mut Context<Self>) {
        self.add_open = true;
        self.add_error = None;
        cx.notify();
    }

    pub fn set_add_mode(&mut self, mode: AddMode, cx: &mut Context<Self>) {
        self.add_mode = mode;
        self.add_error = None;
        cx.notify();
    }

    pub fn open_batch_rename(&mut self, cx: &mut Context<Self>) {
        self.batch_rename_open = true;
        self.rename_error = None;
        cx.notify();
    }

    pub fn close_batch_rename(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.batch_rename_open = false;
        self.rename_error = None;
        self.rename_template
            .update(cx, |state, cx| state.set_value("", window, cx));
        cx.notify();
    }

    pub fn apply_batch_rename(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let template = self.rename_template.read(cx).value().trim().to_string();
        if template.is_empty() {
            self.rename_error = Some("请输入命名模板".to_string());
            cx.notify();
            return;
        }
        if self.nodes.is_empty() {
            self.rename_error = Some("没有可重命名的节点".to_string());
            cx.notify();
            return;
        }

        let old_names: Vec<String> = self
            .nodes
            .iter()
            .map(|node| node.name().to_string())
            .collect();
        let mut renamed = self.nodes.clone();
        apply_name_template(&mut renamed, &template);

        let changes: Vec<(i64, String, String)> = renamed
            .iter()
            .zip(old_names.iter())
            .filter(|(node, old_name)| node.name() != old_name.as_str())
            .filter_map(|(node, _)| {
                node.source_id
                    .parse::<i64>()
                    .ok()
                    .map(|sub_id| (sub_id, node.source_key.clone(), node.name().to_string()))
            })
            .collect();

        if changes.is_empty() {
            self.rename_error = Some("名称没有变化".to_string());
            cx.notify();
            return;
        }

        let count = changes.len();
        {
            let store = self.store.lock().expect("store mutex poisoned");
            for (sub_id, key, alias) in &changes {
                let _ = store.set_node_override_alias(*sub_id, key, alias);
            }
        }
        self.close_batch_rename(window, cx);
        self.reload_subscriptions(cx);
        self.show_toast(format!("已重命名 {count} 个节点"), cx);
    }

    pub fn open_batch_import(&mut self, cx: &mut Context<Self>) {
        self.batch_import_open = true;
        self.batch_error = None;
        self.page = Page::Subscriptions;
        cx.notify();
    }

    pub fn close_batch_import(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.batch_import_open = false;
        self.batch_error = None;
        self.batch_import
            .update(cx, |state, cx| state.set_value("", window, cx));
        cx.notify();
    }

    pub fn paste_batch_clipboard(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(text) = cx
            .read_from_clipboard()
            .and_then(|item| item.text())
            .filter(|text| !text.trim().is_empty())
        else {
            self.batch_error = Some("剪贴板为空或没有可读取的文本".to_string());
            cx.notify();
            return;
        };
        self.batch_import
            .update(cx, |state, cx| state.set_value(text, window, cx));
        self.batch_error = None;
        cx.notify();
    }

    pub fn import_batch(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let input = self.batch_import.read(cx).value().trim().to_string();
        if input.is_empty() {
            self.batch_error = Some("请粘贴节点链接或订阅内容".to_string());
            cx.notify();
            return;
        }

        let parsed = match parse_content(&input) {
            Ok(parsed) if !parsed.proxies.is_empty() => {
                self.import_report = parsed.report.clone();
                parsed
            }
            Ok(parsed) => {
                self.import_report = parsed.report;
                self.batch_error = Some("没有识别到可用节点".to_string());
                cx.notify();
                return;
            }
            Err(error) => {
                self.batch_error = Some(format!("内容解析失败：{error}"));
                cx.notify();
                return;
            }
        };

        let imported = parsed.proxies.len();
        match self.append_manual_proxies(parsed.proxies) {
            Ok(total) => {
                self.batch_import
                    .update(cx, |state, cx| state.set_value("", window, cx));
                self.batch_error = None;
                self.reload_subscriptions(cx);
                self.show_toast(
                    format!("本次导入 {imported} 个节点，手动节点共 {total} 个"),
                    cx,
                );
            }
            Err(error) => {
                self.batch_error = Some(error);
                cx.notify();
            }
        }
    }

    fn append_manual_proxies(&mut self, proxies: Vec<Proxy>) -> Result<usize, String> {
        let store = self.store.lock().expect("store mutex poisoned");

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
        let (payload, total) = merge_manual_proxies_yaml(existing_payload, proxies)
            .map_err(|error| error.to_string())?;
        store
            .save_manual_payload(id, &payload, total)
            .map_err(|error| error.to_string())?;
        Ok(total)
    }

    pub fn open_custom_dialog(&mut self, dialog: CustomDialog, cx: &mut Context<Self>) {
        self.custom_dialog = dialog;
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
        cx.notify();
    }

    pub fn toggle_filter_region(&mut self, region: Region, cx: &mut Context<Self>) {
        if !self.filter_regions.insert(region) {
            self.filter_regions.remove(&region);
        }
        cx.notify();
    }

    pub fn select_chain_node(
        &mut self,
        side: ChainSide,
        key: String,
        name: String,
        cx: &mut Context<Self>,
    ) {
        match side {
            ChainSide::Entry => {
                if self.chain_entry_key.as_deref() == Some(key.as_str()) {
                    self.chain_entry_key = None;
                    self.chain_entry_name = None;
                } else {
                    self.chain_entry_key = Some(key);
                    self.chain_entry_name = Some(name);
                }
            }
            ChainSide::Landing => {
                if self.chain_landing_key.as_deref() == Some(key.as_str()) {
                    self.chain_landing_key = None;
                    self.chain_landing_name = None;
                } else {
                    self.chain_landing_key = Some(key);
                    self.chain_landing_name = Some(name);
                }
            }
        }
        cx.notify();
    }

    pub fn add_filter_group(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let mut regions: Vec<Region> = self.filter_regions.iter().copied().collect();
        regions.sort();
        let include_keywords = split_keywords(&self.filter_include.read(cx).value());
        let exclude_keywords = split_keywords(&self.filter_exclude.read(cx).value());

        if regions.is_empty() && include_keywords.is_empty() && exclude_keywords.is_empty() {
            self.custom_error = Some("请选择地区或填写关键词".to_string());
            cx.notify();
            return;
        }

        let typed_name = self.filter_name.read(cx).value().trim().to_string();
        let name = if typed_name.is_empty() {
            format!("筛选组 {}", self.custom_groups.len() + 1)
        } else {
            typed_name
        };

        self.custom_groups.push(CustomGroupSpec {
            name,
            kind: GroupKind::Select,
            filter: NodeFilter {
                regions,
                sources: Vec::new(),
                include_keywords,
                exclude_keywords,
            },
        });
        self.save_custom_config();
        self.close_custom_dialog(window, cx);
        self.regenerate(cx);
        self.show_toast("已添加筛选代理组".to_string(), cx);
    }

    pub fn add_chain(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let (Some(entry_key), Some(entry_name)) =
            (self.chain_entry_key.clone(), self.chain_entry_name.clone())
        else {
            self.custom_error = Some("请选择中转节点".to_string());
            cx.notify();
            return;
        };
        let (Some(landing_key), Some(landing_name)) = (
            self.chain_landing_key.clone(),
            self.chain_landing_name.clone(),
        ) else {
            self.custom_error = Some("请选择落地节点".to_string());
            cx.notify();
            return;
        };
        if entry_key == landing_key {
            self.custom_error = Some("中转和落地不能是同一个节点".to_string());
            cx.notify();
            return;
        }

        let typed_name = self.chain_name.read(cx).value().trim().to_string();
        let name = if typed_name.is_empty() {
            format!("链式代理 {}", self.chains.len() + 1)
        } else {
            typed_name
        };

        self.chains.push(ChainSpec::new(
            name,
            entry_key,
            entry_name,
            landing_key,
            landing_name,
        ));
        self.save_custom_config();
        self.close_custom_dialog(window, cx);
        self.regenerate(cx);
        self.show_toast("已添加链式代理".to_string(), cx);
    }

    pub fn delete_filter_group(&mut self, index: usize, cx: &mut Context<Self>) {
        if index < self.custom_groups.len() {
            self.custom_groups.remove(index);
            self.save_custom_config();
            self.regenerate(cx);
            self.show_toast("已删除筛选代理组".to_string(), cx);
        }
    }

    pub fn delete_chain(&mut self, index: usize, cx: &mut Context<Self>) {
        if index < self.chains.len() {
            self.chains.remove(index);
            self.save_custom_config();
            self.regenerate(cx);
            self.show_toast("已删除链式代理".to_string(), cx);
        }
    }

    fn load_custom_config(&mut self) {
        let store = self.store.lock().expect("store mutex poisoned");
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

    fn save_custom_config(&self) {
        if self.template_dialog_open {
            return;
        }
        let store = self.store.lock().expect("store mutex poisoned");
        if let Ok(value) = serde_json::to_string(&self.custom_groups) {
            let _ = store.set_setting("custom_groups", &value);
        }
        if let Ok(value) = serde_json::to_string(&self.chains) {
            let _ = store.set_setting("chains", &value);
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
            self.add_error = Some(match self.add_mode {
                AddMode::Subscription => "请输入订阅链接".to_string(),
                AddMode::Node => "请粘贴节点链接".to_string(),
            });
            cx.notify();
            return;
        }

        let typed_name = self.add_name.read(cx).value().trim().to_string();
        if self.add_mode == AddMode::Node || is_node_link(&input) {
            self.add_single_node(input, typed_name, window, cx);
            return;
        }

        if !(input.starts_with("http://") || input.starts_with("https://")) {
            self.add_error = Some("请输入有效的 HTTP/HTTPS 订阅链接".to_string());
            cx.notify();
            return;
        }
        let url = input;

        let name = if typed_name.is_empty() {
            default_subscription_name(&url)
        } else {
            typed_name
        };

        let result = {
            let store = self.store.lock().expect("store mutex poisoned");
            store.add_subscription(&NewSubscription::url(name.clone(), url.clone()))
        };

        match result {
            Ok(id) => {
                self.add_open = false;
                self.add_error = None;
                self.add_name
                    .update(cx, |state, cx| state.set_value("", window, cx));
                self.add_url
                    .update(cx, |state, cx| state.set_value("", window, cx));
                self.reload_subscriptions(cx);
                self.fetch_in_background(
                    id,
                    url,
                    name,
                    CacheValidators::default(),
                    FetchOptions::default(),
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
        _typed_name: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let parsed = match parse_content(&input) {
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

        let imported = parsed.proxies.len();
        match self.append_manual_proxies(parsed.proxies) {
            Ok(total) => {
                self.close_add_dialog(window, cx);
                self.reload_subscriptions(cx);
                self.show_toast(
                    format!("本次导入 {imported} 个节点，手动节点共 {total} 个"),
                    cx,
                );
            }
            Err(error) => {
                self.add_error = Some(format!("保存节点失败：{error}"));
                cx.notify();
            }
        }
    }

    pub fn load_demo(&mut self, cx: &mut Context<Self>) {
        if self
            .subscriptions
            .iter()
            .any(|subscription| subscription.name == "示例订阅（可删除）")
        {
            self.page = Page::Generator;
            self.show_toast("示例订阅已存在".to_string(), cx);
            return;
        }
        const DEMO: &str = include_str!("../assets/demo.yaml");
        let node_count = parse_content(DEMO)
            .map(|parsed| parsed.proxies.len())
            .unwrap_or_default();
        let result = {
            let store = self.store.lock().expect("store mutex poisoned");
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

    pub fn import_from_clipboard(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(text) = cx
            .read_from_clipboard()
            .and_then(|item| item.text())
            .map(|text| text.trim().to_string())
        else {
            self.add_error = Some("剪贴板为空或没有可读取的文本".to_string());
            cx.notify();
            return;
        };
        if text.is_empty() {
            self.add_error = Some("剪贴板内容为空".to_string());
            cx.notify();
            return;
        }

        if text.starts_with("http://") || text.starts_with("https://") {
            let name = default_subscription_name(&text);
            let result = {
                let store = self.store.lock().expect("store mutex poisoned");
                store.add_subscription(&NewSubscription::url(name.clone(), text.clone()))
            };
            match result {
                Ok(id) => {
                    self.close_add_dialog(window, cx);
                    self.reload_subscriptions(cx);
                    self.fetch_in_background(
                        id,
                        text,
                        name,
                        CacheValidators::default(),
                        FetchOptions::default(),
                        cx,
                    );
                }
                Err(error) => {
                    self.add_error = Some(format!("保存订阅失败：{error}"));
                    cx.notify();
                }
            }
            return;
        }

        match parse_content(&text) {
            Ok(parsed) if !parsed.proxies.is_empty() => {
                self.import_report = parsed.report.clone();
                let imported = parsed.proxies.len();
                match self.append_manual_proxies(parsed.proxies) {
                    Ok(total) => {
                        self.close_add_dialog(window, cx);
                        self.reload_subscriptions(cx);
                        self.show_toast(
                            format!("本次导入 {imported} 个节点，手动节点共 {total} 个"),
                            cx,
                        );
                    }
                    Err(error) => {
                        self.add_error = Some(format!("保存节点失败：{error}"));
                        cx.notify();
                    }
                }
            }
            Ok(_) => {
                self.add_error = Some("剪贴板中没有可用节点".to_string());
                cx.notify();
            }
            Err(error) => {
                self.add_error = Some(format!("无法识别剪贴板内容：{error}"));
                cx.notify();
            }
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
                    let store = self.store.lock().expect("store mutex poisoned");
                    store.add_subscription(&NewSubscription::manual(name, kind, content))
                };
                match result {
                    Ok(id) => {
                        let count = parsed.proxies.len();
                        {
                            let store = self.store.lock().expect("store mutex poisoned");
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

    pub fn export_diagnostics(&mut self, cx: &mut Context<Self>) {
        let report = serde_json::json!({"format":"swiftsub-diagnostics","version":1,
            "last_import":self.import_report,"subscriptions":self.source_import_report,
            "generation":serde_json::from_str::<serde_json::Value>(&self.generation_report).ok(),
            "warnings":self.gen_warnings});
        let Some(path) = rfd::FileDialog::new()
            .add_filter("诊断 JSON", &["json"])
            .set_file_name("swiftsub-report.json")
            .save_file()
        else {
            return;
        };
        let result = serde_json::to_string_pretty(&report)
            .map_err(|e| e.to_string())
            .and_then(|text| std::fs::write(path, text).map_err(|e| e.to_string()));
        match result {
            Ok(()) => self.show_toast("诊断报告已导出".into(), cx),
            Err(e) => self.show_toast(format!("导出失败：{e}"), cx),
        }
    }

    pub fn export_preview(&mut self, cx: &mut Context<Self>) {
        if self.preview.is_empty() {
            self.show_toast("暂无可导出的配置".to_string(), cx);
            return;
        }
        let Some(path) = rfd::FileDialog::new()
            .set_file_name("swiftsub.yaml")
            .add_filter("Mihomo YAML", &["yaml", "yml"])
            .save_file()
        else {
            return;
        };
        match std::fs::write(&path, &self.preview) {
            Ok(()) => {
                let name = path
                    .file_name()
                    .and_then(|name| name.to_str())
                    .unwrap_or("swiftsub.yaml");
                self.show_toast(format!("已导出到 {name}"), cx);
            }
            Err(error) => self.show_toast(format!("导出失败：{error}"), cx),
        }
    }

    pub fn reload_subscriptions(&mut self, cx: &mut Context<Self>) {
        let result = {
            let store = self.store.lock().expect("store mutex poisoned");
            store.list_subscriptions()
        };
        match result {
            Ok(subscriptions) => self.subscriptions = subscriptions,
            Err(error) => self.store_error = Some(error.to_string()),
        }
        self.rebuild_nodes(cx);
    }

    fn rebuild_nodes(&mut self, cx: &mut Context<Self>) {
        let mut reports = Vec::new();
        let mut all_nodes = Vec::new();
        let mut seen = HashSet::new();
        let mut warnings = Vec::new();
        let subscriptions = self.subscriptions.clone();

        {
            let store = self.store.lock().expect("store mutex poisoned");
            for subscription in subscriptions
                .iter()
                .filter(|subscription| subscription.enabled)
            {
                let Some(payload) = subscription.payload.as_deref() else {
                    continue;
                };
                match parse_content(payload) {
                    Ok(parsed) => {
                        reports.extend(
                            parsed
                                .report
                                .iter()
                                .cloned()
                                .map(|item| (subscription.name.clone(), item)),
                        );
                        for warning in parsed.warnings {
                            warnings.push(format!("{} · {}", subscription.name, warning.message));
                        }
                        let mut nodes = build_nodes(
                            parsed.proxies,
                            subscription.id.to_string(),
                            subscription.name.clone(),
                        );
                        let overrides = store
                            .list_node_overrides(subscription.id)
                            .unwrap_or_default();
                        for node in &mut nodes {
                            let source_key = node.source_key.clone();
                            if let Some(value) = overrides.get(&source_key) {
                                if let Some(extras) = &value.extras
                                    && let Ok(edited) = Proxy::from_yaml_text(extras)
                                {
                                    node.proxy = edited;
                                }
                                if let Some(alias) = &value.alias {
                                    node.set_name(alias.clone());
                                }
                                if let Some(enabled) = value.enabled {
                                    node.enabled = enabled;
                                }
                                node.listen_port = value.listen_port;
                            }
                            if self.disabled_nodes.contains(&source_key) {
                                node.enabled = false;
                            }
                            node.region = detect_region(node.name());
                            if seen.insert(node.proxy.identity_key()) {
                                all_nodes.push(node.clone());
                            }
                        }
                    }
                    Err(error) => warnings.push(format!("{}：{error}", subscription.name)),
                }
            }
        }

        if !self.node_order.is_empty() {
            let positions: HashMap<&str, usize> = self
                .node_order
                .iter()
                .enumerate()
                .map(|(index, key)| (key.as_str(), index))
                .collect();
            all_nodes.sort_by_key(|node| {
                positions
                    .get(node.source_key.as_str())
                    .copied()
                    .unwrap_or(usize::MAX)
            });
        }

        self.nodes = all_nodes;
        self.source_import_report = reports;
        self.parse_warnings = warnings;
        self.regenerate(cx);
    }

    pub fn regenerate(&mut self, cx: &mut Context<Self>) {
        if self.nodes.is_empty() {
            self.preview.clear();
            self.stats = ProfileStats::default();
            self.gen_warnings = self.parse_warnings.clone();
            if let Some(server) = &self.server {
                server.update_profile(ProfileSnapshot::default());
            }
            cx.notify();
            return;
        }

        let mut options = self.options.clone();
        options.enabled_modules = Some(self.enabled_modules.iter().cloned().collect());
        options.disabled_rule_sets = self.disabled_rule_sets.iter().cloned().collect();
        options.custom_rule_sets = self.normalize_rule_set_bindings(&self.custom_rule_sets);
        options.custom_groups = self
            .template_baseline
            .as_ref()
            .map(|b| b.custom_groups.clone())
            .unwrap_or_else(|| self.custom_groups.clone());
        options.chains = self
            .template_baseline
            .as_ref()
            .map(|b| b.chains.clone())
            .unwrap_or_else(|| self.chains.clone());

        let preview_was_empty = self.preview.is_empty();
        match generate_profile(&self.nodes, &options) {
            Ok(profile) => {
                self.generation_report =
                    serde_json::to_string_pretty(&profile.trace).unwrap_or_default();
                self.preview = profile.yaml;
                if preview_was_empty {
                    self.preview_scroll.set_offset(point(px(0.0), px(0.0)));
                }
                self.stats = profile.stats;
                self.gen_warnings = profile.warnings;

                if self.profile_id > 0 {
                    let store = self.store.lock().expect("store mutex poisoned");
                    let _ = store.save_profile_yaml(self.profile_id, &self.preview);
                }
                if let Some(server) = &self.server {
                    server.update_profile(ProfileSnapshot {
                        yaml: self.preview.clone(),
                        userinfo_header: self.aggregate_userinfo(),
                        update_interval_hours: 24,
                    });
                }
            }
            Err(error) => {
                self.gen_warnings = vec![format!("生成失败：{error}")];
            }
        }
        cx.notify();
    }

    fn start_server(&mut self) {
        let preferred_port = {
            let store = self.store.lock().expect("store mutex poisoned");
            store
                .get_setting("server_port")
                .ok()
                .flatten()
                .and_then(|value| value.parse::<u16>().ok())
        };
        let mut candidates = Vec::new();
        if let Some(port) = preferred_port.filter(|port| *port > 0) {
            candidates.push(port);
        }
        for port in 7899..=7905 {
            if !candidates.contains(&port) {
                candidates.push(port);
            }
        }

        let mut last_error = None;
        for port in candidates {
            let config = ServerConfig {
                port,
                token: self.profile_token.clone(),
                bind_lan: false,
                update_interval_hours: 24,
            };
            let snapshot = ProfileSnapshot {
                yaml: self.preview.clone(),
                userinfo_header: self.aggregate_userinfo(),
                update_interval_hours: 24,
            };
            match LocalServer::start(config, snapshot) {
                Ok(server) => {
                    {
                        let store = self.store.lock().expect("store mutex poisoned");
                        let _ = store.set_setting("server_port", &port.to_string());
                    }
                    self.server_url = Some(server.url_for(&self.subscription_filename));
                    self.server = Some(server);
                    self.server_error = None;
                    return;
                }
                Err(error) => last_error = Some(error),
            }
        }
        self.server_error = last_error.map(|error| error.to_string());
    }

    pub fn refresh_subscription(&mut self, id: i64, cx: &mut Context<Self>) {
        if self.refreshing.contains(&id) {
            return;
        }
        let Some(subscription) = self
            .subscriptions
            .iter()
            .find(|subscription| subscription.id == id)
            .cloned()
        else {
            return;
        };
        let Some(url) = subscription.url.clone() else {
            self.show_toast("该订阅没有远程地址，无需刷新".to_string(), cx);
            return;
        };
        let cache = CacheValidators {
            etag: subscription.etag.clone(),
            last_modified: subscription.last_modified.clone(),
        };
        let mut options = FetchOptions::default();
        if let Some(user_agent) = subscription.user_agent {
            options.user_agent = user_agent;
        }
        self.fetch_in_background(id, url, subscription.name, cache, options, cx);
    }

    pub fn refresh_all(&mut self, cx: &mut Context<Self>) {
        let ids: Vec<i64> = self
            .subscriptions
            .iter()
            .filter(|subscription| subscription.enabled && subscription.url.is_some())
            .map(|subscription| subscription.id)
            .collect();
        if ids.is_empty() {
            self.show_toast("没有可刷新的远程订阅".to_string(), cx);
            return;
        }
        for id in ids {
            self.refresh_subscription(id, cx);
        }
    }

    fn fetch_in_background(
        &mut self,
        id: i64,
        url: String,
        name: String,
        cache: CacheValidators,
        options: FetchOptions,
        cx: &mut Context<Self>,
    ) {
        self.refreshing.insert(id);
        cx.notify();

        let store = Arc::clone(&self.store);
        cx.spawn(async move |this, cx| {
            let (outcome, node_count) = cx
                .background_executor()
                .spawn(async move {
                    let outcome =
                        fetch_subscription(&url, &options, &cache).map_err(|e| e.to_string());
                    let (outcome, node_count) = match outcome {
                        Ok(FetchOutcome::Fetched(result)) => match parse_content(&result.content) {
                            Ok(parsed)
                                if parsed.proxies.iter().any(|p| p.fatal_issue().is_none()) =>
                            {
                                let count = parsed.proxies.len();
                                (Ok(FetchOutcome::Fetched(result)), count)
                            }
                            Ok(_) => (Err("订阅没有可用节点，保留上次成功内容".to_string()), 0),
                            Err(e) => (Err(format!("订阅解析失败，保留上次成功内容：{e}")), 0),
                        },
                        other => (other, 0),
                    };
                    (outcome, node_count)
                })
                .await;

            let Some(this) = this.upgrade() else {
                return;
            };
            this.update(cx, |this, cx| {
                this.refreshing.remove(&id);
                match outcome {
                    Ok(FetchOutcome::Fetched(result)) => {
                        let info = result.info.clone().unwrap_or_default();
                        let traffic = TrafficSnapshot {
                            upload: info.upload,
                            download: info.download,
                            total: info.total,
                            expire_at: info.expire,
                        };
                        {
                            let store = store.lock().expect("store mutex poisoned");
                            if let Err(error) = store.save_fetch_success(
                                id,
                                &result.content,
                                result.etag.as_deref(),
                                result.last_modified.as_deref(),
                                &traffic,
                                node_count,
                            ) {
                                this.show_toast(format!("订阅保存失败，保留上次内容：{error}"), cx);
                                return;
                            }
                            if let Some(hours) = result.update_interval_hours {
                                let minutes = hours.saturating_mul(60).clamp(15, 10_080);
                                let _ = store.set_subscription_refresh_minutes(id, minutes);
                            }
                            let _ = store.log_refresh(Some(id), true, None);
                        }
                        this.failure_counts.remove(&id);
                        this.retry_after.remove(&id);
                        this.show_toast(format!("{name} 已更新，共 {node_count} 个节点"), cx);
                    }
                    Ok(FetchOutcome::NotModified { info, .. }) => {
                        let traffic = info.map(|info| TrafficSnapshot {
                            upload: info.upload,
                            download: info.download,
                            total: info.total,
                            expire_at: info.expire,
                        });
                        {
                            let store = store.lock().expect("store mutex poisoned");
                            let _ = store.save_fetch_not_modified(id, traffic.as_ref());
                            let _ = store.log_refresh(Some(id), true, Some("not modified"));
                        }
                        this.failure_counts.remove(&id);
                        this.retry_after.remove(&id);
                        this.show_toast(format!("{name} 已是最新版本"), cx);
                    }
                    Err(error) => {
                        let message = error.to_string();
                        {
                            let store = store.lock().expect("store mutex poisoned");
                            let _ = store.save_fetch_error(id, &message);
                            let _ = store.log_refresh(Some(id), false, Some(&message));
                        }
                        let failures = this.failure_counts.entry(id).or_insert(0);
                        *failures = failures.saturating_add(1);
                        let delay_seconds = 3600_i64
                            .saturating_mul(1_i64 << (*failures - 1).min(5))
                            .min(86_400);
                        this.retry_after
                            .insert(id, swiftsub_store::now_ts() + delay_seconds);
                        this.show_toast(format!("{name} 更新失败：{message}"), cx);
                    }
                }
                this.reload_subscriptions(cx);
            });
        })
        .detach();
    }

    pub fn toggle_subscription(&mut self, id: i64, enabled: bool, cx: &mut Context<Self>) {
        {
            let store = self.store.lock().expect("store mutex poisoned");
            let _ = store.set_subscription_enabled(id, enabled);
        }
        self.reload_subscriptions(cx);
    }

    pub fn open_edit_subscription(&mut self, id: i64, window: &mut Window, cx: &mut Context<Self>) {
        let Some(subscription) = self
            .subscriptions
            .iter()
            .find(|subscription| subscription.id == id)
            .cloned()
        else {
            return;
        };

        self.edit_name.update(cx, |state, cx| {
            state.set_value(subscription.name.clone(), window, cx)
        });
        self.edit_url.update(cx, |state, cx| {
            state.set_value(subscription.url.clone().unwrap_or_default(), window, cx)
        });
        self.edit_interval.update(cx, |state, cx| {
            state.set_value(subscription.refresh_minutes.to_string(), window, cx)
        });
        self.edit_ua.update(cx, |state, cx| {
            state.set_value(
                subscription.user_agent.clone().unwrap_or_default(),
                window,
                cx,
            )
        });
        self.edit_enabled = subscription.enabled;
        self.edit_error = None;
        self.edit_nodes = subscription
            .payload
            .as_deref()
            .and_then(|payload| parse_content(payload).ok())
            .map(|parsed| {
                parsed
                    .proxies
                    .iter()
                    .map(|proxy| {
                        (
                            proxy.name().to_string(),
                            detect_region(proxy.name()).label().to_string(),
                            proxy.kind().to_uppercase(),
                        )
                    })
                    .collect()
            })
            .unwrap_or_default();
        self.edit_data = Some(subscription);
        self.edit_subscription = Some(id);
        cx.notify();
    }

    pub fn close_edit_subscription(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.edit_subscription = None;
        self.edit_data = None;
        self.edit_nodes.clear();
        self.edit_error = None;
        for input in [
            &self.edit_name,
            &self.edit_url,
            &self.edit_interval,
            &self.edit_ua,
        ] {
            input.update(cx, |state, cx| state.set_value("", window, cx));
        }
        cx.notify();
    }

    pub fn set_edit_enabled(&mut self, enabled: bool, cx: &mut Context<Self>) {
        self.edit_enabled = enabled;
        cx.notify();
    }

    pub fn save_edit_subscription(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(id) = self.edit_subscription else {
            return;
        };
        let Some(old) = self.edit_data.clone() else {
            return;
        };

        let name = self.edit_name.read(cx).value().trim().to_string();
        if name.is_empty() {
            self.edit_error = Some("订阅名称不能为空".to_string());
            cx.notify();
            return;
        }

        let interval = match self.edit_interval.read(cx).value().trim().parse::<u32>() {
            Ok(interval) if interval > 0 => interval.clamp(15, 10_080),
            _ => {
                self.edit_error = Some("刷新间隔必须是 15-10080 之间的分钟数".to_string());
                cx.notify();
                return;
            }
        };

        let url = if old.kind == swiftsub_store::SubscriptionKind::Url {
            let url = self.edit_url.read(cx).value().trim().to_string();
            if !(url.starts_with("http://") || url.starts_with("https://")) {
                self.edit_error = Some("订阅链接必须是 HTTP/HTTPS 地址".to_string());
                cx.notify();
                return;
            }
            Some(url)
        } else {
            old.url.clone()
        };
        let url_changed = old.url != url;

        let user_agent = self.edit_ua.read(cx).value().trim().to_string();
        let user_agent = (!user_agent.is_empty()).then_some(user_agent.as_str());

        let result = {
            let store = self.store.lock().expect("store mutex poisoned");
            store.update_subscription_meta(
                id,
                &name,
                url.as_deref(),
                self.edit_enabled,
                interval,
                user_agent,
            )
        };

        match result {
            Ok(()) => {
                self.close_edit_subscription(window, cx);
                self.reload_subscriptions(cx);
                if url_changed
                    && old.kind == swiftsub_store::SubscriptionKind::Url
                    && let Some(url) = url
                {
                    self.fetch_in_background(
                        id,
                        url,
                        name,
                        CacheValidators::default(),
                        FetchOptions::default(),
                        cx,
                    );
                }
                self.show_toast("订阅已保存".to_string(), cx);
            }
            Err(error) => {
                self.edit_error = Some(format!("保存失败：{error}"));
                cx.notify();
            }
        }
    }

    pub fn ask_delete(&mut self, id: i64, cx: &mut Context<Self>) {
        self.confirm_delete = Some(id);
        cx.notify();
    }

    pub fn cancel_delete(&mut self, cx: &mut Context<Self>) {
        self.confirm_delete = None;
        cx.notify();
    }

    pub fn perform_delete(&mut self, cx: &mut Context<Self>) {
        let Some(id) = self.confirm_delete.take() else {
            return;
        };
        {
            let store = self.store.lock().expect("store mutex poisoned");
            let _ = store.delete_subscription(id);
        }
        self.reload_subscriptions(cx);
        self.show_toast("订阅已删除".to_string(), cx);
    }

    pub fn toggle_module(&mut self, id: String, enabled: bool, cx: &mut Context<Self>) {
        if matches!(id.as_str(), "select" | "auto" | "final") {
            return;
        }
        if enabled {
            self.enabled_modules.insert(id);
        } else {
            self.enabled_modules.remove(&id);
        }
        self.save_module_config();
        self.regenerate(cx);
    }

    fn load_templates(&mut self) {
        let templates = {
            let store = self.store.lock().expect("store mutex poisoned");
            store.list_custom_templates().unwrap_or_default()
        };
        self.custom_templates = templates;
    }

    pub fn open_template_dialog(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.template_dialog_open {
            return;
        }
        self.template_base_options = Some(self.options.clone());
        self.template_baseline = Some(self.active_template_snapshot());
        self.template_edit_id = None;
        self.template_discard_armed = false;
        self.template_dialog_open = true;
        self.reset_advanced_draft(window, cx);
        self.template_error = None;
        self.template_tab = TemplateTab::PolicyGroups;
        self.template_disabled_rule_sets = self.disabled_rule_sets.clone();
        self.template_custom_rule_sets = self.custom_rule_sets.clone();
        self.template_enabled_modules = self.enabled_modules.clone();
        self.template_policy_groups = self.options.policy_groups.clone();
        self.template_rule_order = self.options.rule_order.clone();
        self.template_rule_set_targets = self.options.rule_set_targets.clone();
        self.template_policy_edit = None;
        self.template_target_picker = None;
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
        if let Some(baseline) = self.template_baseline.take() {
            self.custom_groups = baseline.custom_groups;
            self.chains = baseline.chains;
        }
        self.template_base_options = None;
        self.template_edit_id = None;
        self.template_discard_armed = false;
        self.template_dialog_open = false;
        self.template_error = None;
        self.template_tab = TemplateTab::PolicyGroups;
        self.template_disabled_rule_sets = self.disabled_rule_sets.clone();
        self.template_custom_rule_sets = self.custom_rule_sets.clone();
        self.template_enabled_modules = self.enabled_modules.clone();
        self.template_policy_groups = self.options.policy_groups.clone();
        self.template_rule_order = self.options.rule_order.clone();
        self.template_rule_set_targets = self.options.rule_set_targets.clone();
        self.template_policy_edit = None;
        self.template_target_picker = None;
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
        let base = self.template_base_options.as_ref().unwrap_or(&self.options);
        let draft = AdvancedDraft {
            dns: base.dns.clone(),
            listeners: base.listeners.clone(),
            chains: self.chains.clone(),
            custom_groups: self.custom_groups.clone(),
            extra_yaml: base.extra_yaml.clone(),
        };
        if let Ok(text) = serde_json::to_string_pretty(&draft) {
            self.advanced_draft
                .update(cx, |s, cx| s.set_value(text, window, cx));
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
        self.template_error = None;
        cx.notify();
    }

    pub fn set_template_tab(&mut self, tab: TemplateTab, cx: &mut Context<Self>) {
        self.template_tab = tab;
        self.template_target_picker = None;
        cx.notify();
    }

    pub fn toggle_template_rule_set(&mut self, id: String, enabled: bool, cx: &mut Context<Self>) {
        if enabled {
            self.template_disabled_rule_sets.remove(&id);
        } else {
            self.template_disabled_rule_sets.insert(id);
        }
        cx.notify();
    }

    pub fn toggle_template_source_rule_set(
        &mut self,
        source_id: String,
        entry_id: String,
        enabled: bool,
        cx: &mut Context<Self>,
    ) {
        self.set_template_source_rules(source_id, vec![entry_id], enabled, cx);
    }

    pub fn toggle_template_rule_source(&mut self, id: String, cx: &mut Context<Self>) {
        if !self.template_rule_sources_collapsed.insert(id.clone()) {
            self.template_rule_sources_collapsed.remove(&id);
        }
        cx.notify();
    }

    pub fn set_template_source_rules(
        &mut self,
        source_id: String,
        entry_ids: Vec<String>,
        enabled: bool,
        cx: &mut Context<Self>,
    ) {
        if source_id == "builtin" {
            for id in entry_ids {
                if enabled {
                    self.template_disabled_rule_sets.remove(&id);
                } else {
                    self.template_disabled_rule_sets.insert(id);
                }
            }
        } else if let Some(source) = rule_catalog::SOURCES
            .iter()
            .find(|source| source.id == source_id)
        {
            rule_catalog::set_source_selection(
                &mut self.template_custom_rule_sets,
                source,
                &entry_ids,
                enabled,
                &self.options.rule_provider_base_url,
                &self.template_enabled_modules,
            );
            if enabled {
                for binding in &self.template_custom_rule_sets {
                    if let Some(module) = catalog::MODULES
                        .iter()
                        .find(|module| module.name == binding.target)
                    {
                        self.template_enabled_modules.insert(module.id.to_string());
                    }
                }
            }
        }
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

    pub fn remove_template_custom_binding(&mut self, id: String, cx: &mut Context<Self>) {
        self.template_custom_rule_sets
            .retain(|binding| binding.id != id);
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

    pub fn open_template_target_picker(&mut self, id: String, cx: &mut Context<Self>) {
        self.template_target_picker = Some(id);
        self.template_error = None;
        cx.notify();
    }

    pub fn set_template_rule_target(&mut self, id: String, target: String, cx: &mut Context<Self>) {
        if let Some(module) = catalog::MODULES.iter().find(|module| module.name == target) {
            self.template_enabled_modules.insert(module.id.to_string());
        }
        if let Some(binding) = self
            .template_custom_rule_sets
            .iter_mut()
            .find(|binding| binding.id == id)
        {
            binding.target = target;
        } else if catalog::MODULES
            .iter()
            .any(|module| module.rule_sets.iter().any(|spec| spec.id == id))
        {
            self.template_rule_set_targets.insert(id, target);
        } else {
            for source in rule_catalog::SOURCES {
                if let Some(entry) = source
                    .entries
                    .iter()
                    .find(|entry| format!("{}-{}", source.id, entry.id) == id)
                {
                    let mut binding = rule_catalog::binding_for(
                        source,
                        entry,
                        rule_catalog::mirror_from_base(&self.options.rule_provider_base_url),
                        &self.options.rule_provider_base_url,
                        &self.template_enabled_modules,
                    );
                    binding.target = target;
                    self.template_custom_rule_sets.push(binding);
                    break;
                }
            }
        }
        self.template_target_picker = None;
        cx.notify();
    }

    pub fn toggle_template_module(&mut self, id: String, enabled: bool, cx: &mut Context<Self>) {
        if !matches!(id.as_str(), "select" | "auto" | "final") {
            if enabled {
                self.template_enabled_modules.insert(id);
            } else {
                self.template_enabled_modules.remove(&id);
            }
        }
        self.template_error = None;
        cx.notify();
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
                name,
                kind: GroupKind::Select,
                members: vec!["节点选择".into(), "自动选择".into(), "DIRECT".into()],
                include_all_nodes: false,
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
        options.rule_set_targets = self.template_rule_set_targets.clone();
        options.rule_order = self.template_rule_order.clone();
        options.custom_groups = self.custom_groups.clone();
        options.chains = self.chains.clone();
        Ok(options)
    }

    pub fn draft_generated(
        &self,
        cx: &Context<Self>,
    ) -> Result<swiftsub_core::generator::GeneratedProfile, String> {
        let options = self.draft_generate_options(cx)?;
        let nodes = if self.nodes.iter().any(|n| n.enabled) {
            self.nodes.clone()
        } else {
            build_nodes(
                vec![Proxy::new("direct", "模板校验节点", "127.0.0.1", 1)],
                "validation",
                "validation",
            )
        };
        generate_profile(&nodes, &options).map_err(|e| e.to_string())
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
        let Ok(mut options) = self.draft_generate_options(cx) else {
            return;
        };
        options.policy_groups.clear();
        let nodes = if self.nodes.iter().any(|n| n.enabled) {
            self.nodes.clone()
        } else {
            build_nodes(
                vec![Proxy::new("direct", "模板校验节点", "127.0.0.1", 1)],
                "validation",
                "validation",
            )
        };
        if let Ok(profile) = generate_profile(&nodes, &options) {
            if let Some(group) = profile.groups.iter().find(|g| g.name == name) {
                let all_nodes = matches!(name.as_str(), "节点选择" | "自动选择");
                self.template_policy_groups.push(PolicyGroupSpec {
                    name,
                    kind: group.kind,
                    members: if all_nodes {
                        Vec::new()
                    } else {
                        group.proxies.clone()
                    },
                    include_all_nodes: all_nodes,
                    builtin: true,
                    ..Default::default()
                });
                self.edit_template_policy_group(self.template_policy_groups.len() - 1, window, cx);
            } else {
                self.template_error = Some("该组没有匹配节点，请先启用节点或模块".into());
                cx.notify();
            }
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
                    self.template_error = Some("测速间隔必须是正整数".into());
                    cx.notify();
                    return;
                }
            }
        };
        if (!url.is_empty() && !(url.starts_with("https://") || url.starts_with("http://")))
            || name.is_empty()
            || name.contains([',', '\n', '\r'])
            || (name != old_name
                && (group.builtin
                    || self.template_target_options().contains(&name)
                    || ["PASS", "COMPATIBLE", "GLOBAL"].contains(&name.as_str())
                    || self.nodes.iter().any(|n| n.name() == name)))
        {
            self.template_error = Some("名称重复或无效，或测速 URL 无效；内置组名称固定".into());
            cx.notify();
            return;
        }
        // Parse first: invalid rule text must not result in a partially renamed draft.
        let mut rules = match parse_custom_rule_lines(&self.template_custom_rules.read(cx).value())
        {
            Ok(rules) => rules,
            Err(e) => {
                self.template_error = Some(e);
                cx.notify();
                return;
            }
        };
        if old_name != name {
            for g in &mut self.template_policy_groups {
                for member in &mut g.members {
                    if *member == old_name {
                        *member = name.clone();
                    }
                }
            }
            for b in &mut self.template_custom_rule_sets {
                if b.target == old_name {
                    b.target = name.clone();
                }
            }
            for t in self.template_rule_set_targets.values_mut() {
                if *t == old_name {
                    *t = name.clone();
                }
            }
            for rule in &mut rules {
                if rule.target == old_name {
                    rule.target = name.clone();
                }
            }
            self.template_custom_rules.update(cx, |s, cx| {
                s.set_value(
                    rules
                        .iter()
                        .map(Rule::to_line)
                        .collect::<Vec<_>>()
                        .join("\n"),
                    window,
                    cx,
                )
            });
        }
        let group = &mut self.template_policy_groups[index];
        group.name = name;
        group.test_url = (!url.is_empty()).then_some(url);
        group.test_interval = interval;
        self.template_error = None;
        cx.notify();
    }

    pub fn move_policy_member(
        &mut self,
        index: usize,
        from: usize,
        to: usize,
        cx: &mut Context<Self>,
    ) {
        if let Some(g) = self.template_policy_groups.get_mut(index) {
            if from < g.members.len() && to < g.members.len() {
                let member = g.members.remove(from);
                g.members.insert(to, member);
            }
        }
        cx.notify();
    }

    pub fn move_template_rule(&mut self, from: usize, to: usize, cx: &mut Context<Self>) {
        if let Ok(profile) = self.draft_generated(cx) {
            let mut keys: Vec<_> = profile
                .rules
                .iter()
                .map(swiftsub_core::generator::rule_identity)
                .collect();
            if from < keys.len() && to < keys.len() {
                let key = keys.remove(from);
                keys.insert(to, key);
                self.template_rule_order = keys;
            }
        }
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

    pub fn toggle_template_policy_member(
        &mut self,
        index: usize,
        member: String,
        cx: &mut Context<Self>,
    ) {
        if let Some(group) = self.template_policy_groups.get_mut(index) {
            if member == "__all_nodes" {
                group.include_all_nodes = !group.include_all_nodes;
            } else if group.members.contains(&member) {
                group.members.retain(|name| name != &member);
            } else {
                if let Some(module) = catalog::MODULES.iter().find(|module| module.name == member) {
                    self.template_enabled_modules.insert(module.id.to_string());
                }
                group.members.push(member);
            }
        }
        cx.notify();
    }

    pub fn remove_template_policy_group(&mut self, index: usize, cx: &mut Context<Self>) {
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
            rule_order: self.template_rule_order.clone(),
            dns: base.dns.clone(),
            listeners: base.listeners.clone(),
            extra_yaml: base.extra_yaml.clone(),
            rule_set_targets: self.template_rule_set_targets.clone(),
            rule_provider_base_url: base.rule_provider_base_url.clone(),
            custom_groups: self.custom_groups.clone(),
            chains: self.chains.clone(),
            test_url: base.test_url.clone(),
            test_interval: base.test_interval,
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
        let nodes = if self.nodes.iter().any(|node| node.enabled) {
            self.nodes.clone()
        } else {
            swiftsub_core::normalize::build_nodes(
                vec![Proxy::new("direct", "模板校验节点", "127.0.0.1", 1)],
                "validation",
                "validation",
            )
        };
        generate_profile(&nodes, &options).map_err(|error| error.to_string())?;
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
            rule_set_targets: self.options.rule_set_targets.clone(),
            rule_order: self.options.rule_order.clone(),
            dns: self.options.dns.clone(),
            listeners: self.options.listeners.clone(),
            extra_yaml: self.options.extra_yaml.clone(),
            custom_groups: self.custom_groups.clone(),
            chains: self.chains.clone(),
            test_url: self.options.test_url.clone(),
            test_interval: self.options.test_interval,
            mixed_port: self.options.mixed_port,
            allow_lan: self.options.allow_lan,
        }
    }

    pub fn template_change_summary(&self, cx: &Context<Self>) -> String {
        match self.draft_generate_options(cx).and_then(|o| {
            self.template_baseline
                .as_ref()
                .unwrap_or(&self.active_template_snapshot())
                .generate_options(&self.options)
                .map(|b| (o, b))
        }) {
            Ok((draft, base)) => {
                let mut changes = Vec::new();
                if draft
                    .enabled_modules
                    .as_ref()
                    .map(|v| v.iter().collect::<HashSet<_>>())
                    != base
                        .enabled_modules
                        .as_ref()
                        .map(|v| v.iter().collect::<HashSet<_>>())
                {
                    changes.push("模块");
                }
                if draft.disabled_rule_sets != base.disabled_rule_sets
                    || draft.custom_rule_sets != base.custom_rule_sets
                {
                    changes.push("规则集");
                }
                if draft.policy_groups != base.policy_groups {
                    changes.push("策略组");
                }
                if draft.rule_set_targets != base.rule_set_targets {
                    changes.push("规则目标");
                }
                if draft.custom_rules != base.custom_rules {
                    changes.push("自定义规则");
                }
                if draft.rule_order != base.rule_order {
                    changes.push("规则顺序");
                }
                if serde_json::to_string(&draft.custom_groups).ok()
                    != serde_json::to_string(&base.custom_groups).ok()
                {
                    changes.push("筛选组");
                }
                if serde_json::to_string(&draft.chains).ok()
                    != serde_json::to_string(&base.chains).ok()
                {
                    changes.push("链式代理");
                }
                if draft.dns != base.dns {
                    changes.push("DNS");
                }
                if draft.listeners != base.listeners {
                    changes.push("独立监听");
                }
                if draft.extra_yaml != base.extra_yaml {
                    changes.push("扩展 YAML");
                }
                if draft.test_url != base.test_url || draft.test_interval != base.test_interval {
                    changes.push("全局测速");
                }
                if draft.mixed_port != base.mixed_port || draft.allow_lan != base.allow_lan {
                    changes.push("监听设置");
                }
                if draft.rule_provider_base_url != base.rule_provider_base_url {
                    changes.push("规则镜像");
                }
                if changes.is_empty() {
                    "配置与当前输出一致；名称及未更新的组详情不计入比较".into()
                } else {
                    format!("待应用变更：{}", changes.join("、"))
                }
            }
            Err(e) => e,
        }
    }

    pub fn request_close_template(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.template_discard_armed {
            self.template_discard_armed = true;
            self.template_error =
                Some("再点「放弃草稿」将丢弃本次编辑，也可继续编辑或应用。".into());
            cx.notify();
            return;
        }
        self.close_template_dialog(window, cx);
    }

    fn persist_and_commit_template(
        &mut self,
        snapshot: TemplateSnapshot,
        save: Option<(Option<i64>, String)>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Result<(), String> {
        self.validate_template_snapshot(&snapshot)?;
        let options = snapshot.generate_options(&self.options)?;
        let json = serde_json::to_string(&snapshot).map_err(|e| e.to_string())?;
        let value = serde_json::to_value(&snapshot).map_err(|e| e.to_string())?;
        let mut settings = HashMap::new();
        for (key, value) in value.as_object().expect("snapshot is object") {
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
        self.store
            .lock()
            .expect("store mutex poisoned")
            .save_routing_snapshot(
                &settings,
                save.as_ref()
                    .map(|(id, name)| (*id, name.as_str(), json.as_str())),
            )
            .map_err(|e| format!("保存失败，配置未应用：{e}"))?;
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

    pub fn apply_template_rule_sets(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.finish_policy_details(window, cx) {
            return;
        }
        let result = self
            .draft_template_snapshot(cx)
            .and_then(|snapshot| self.persist_and_commit_template(snapshot, None, window, cx));
        match result {
            Ok(()) => {
                self.close_template_dialog(window, cx);
                self.show_toast("分流配置已应用".into(), cx);
            }
            Err(e) => {
                self.template_error = Some(e);
                cx.notify();
            }
        }
    }

    pub fn save_current_template(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.finish_policy_details(window, cx) {
            return;
        }
        let name = self.template_name.read(cx).value().trim().to_string();
        if name.is_empty()
            || self
                .custom_templates
                .iter()
                .any(|t| t.name == name && Some(t.id) != self.template_edit_id)
        {
            self.template_error = Some("请输入未被其他模板使用的名称".into());
            cx.notify();
            return;
        }
        let save = Some((self.template_edit_id, name));
        let result = self
            .draft_template_snapshot(cx)
            .and_then(|snapshot| self.persist_and_commit_template(snapshot, save, window, cx));
        match result {
            Ok(()) => {
                self.close_template_dialog(window, cx);
                self.load_templates();
                self.show_toast("模板已保存并应用".into(), cx);
            }
            Err(e) => {
                self.template_error = Some(e);
                cx.notify();
            }
        }
    }

    fn load_template_draft(
        &mut self,
        snapshot: TemplateSnapshot,
        name: String,
        id: Option<i64>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.open_template_dialog(window, cx);
        self.template_base_options = snapshot.generate_options(&self.options).ok();
        self.template_edit_id = id;
        self.template_enabled_modules = snapshot.enabled_modules.into_iter().collect();
        self.template_disabled_rule_sets = snapshot.disabled_rule_sets.into_iter().collect();
        self.template_custom_rule_sets = snapshot.custom_rule_sets;
        self.template_policy_groups = snapshot.policy_groups;
        self.template_rule_set_targets = snapshot.rule_set_targets;
        self.template_rule_order = snapshot.rule_order;
        self.custom_groups = snapshot.custom_groups;
        self.chains = snapshot.chains;
        self.template_name
            .update(cx, |s, cx| s.set_value(name, window, cx));
        self.template_custom_rules.update(cx, |s, cx| {
            s.set_value(snapshot.custom_rule_lines.join("\n"), window, cx)
        });
        self.reset_advanced_draft(window, cx);
        self.page = Page::Generator;
        cx.notify();
    }

    pub fn apply_custom_template(&mut self, id: i64, window: &mut Window, cx: &mut Context<Self>) {
        if self.template_dialog_open {
            self.show_toast("请先处理当前草稿".into(), cx);
            return;
        }
        if let Some(t) = self.custom_templates.iter().find(|t| t.id == id).cloned() {
            match serde_json::from_str(&t.config_json)
                .map_err(|e| e.to_string())
                .and_then(|s| self.validate_template_snapshot(&s).map(|_| s))
            {
                Ok(s) => self.load_template_draft(s, t.name, Some(id), window, cx),
                Err(e) => self.show_toast(format!("模板无效：{e}"), cx),
            }
        }
    }

    pub fn copy_custom_template(&mut self, id: i64, cx: &mut Context<Self>) {
        if let Some(t) = self.custom_templates.iter().find(|t| t.id == id).cloned() {
            let mut name = format!("{} 副本", t.name);
            let mut i = 2;
            while self.custom_templates.iter().any(|t| t.name == name) {
                name = format!("{} 副本 {i}", t.name);
                i += 1;
            }
            let result = self
                .store
                .lock()
                .expect("store mutex poisoned")
                .add_custom_template(&name, &t.config_json);
            match result {
                Ok(_) => {
                    self.load_templates();
                    self.show_toast("模板已复制".into(), cx)
                }
                Err(e) => self.show_toast(format!("复制失败：{e}"), cx),
            }
        }
    }

    pub fn export_custom_template(&mut self, id: i64, cx: &mut Context<Self>) {
        let Some(t) = self.custom_templates.iter().find(|t| t.id == id).cloned() else {
            return;
        };
        let result = serde_json::from_str(&t.config_json)
            .map_err(|e| e.to_string())
            .and_then(|config| {
                serde_json::to_string_pretty(&TemplateFile {
                    format: "swiftsub-routing-template".into(),
                    version: 1,
                    name: t.name,
                    config,
                })
                .map_err(|e| e.to_string())
            });
        let text = match result {
            Ok(s) => s,
            Err(e) => {
                self.show_toast(e, cx);
                return;
            }
        };
        if let Some(path) = rfd::FileDialog::new()
            .add_filter("SwiftSub 模板", &["json"])
            .set_file_name("routing-template.json")
            .save_file()
        {
            match std::fs::write(path, text) {
                Ok(()) => self.show_toast("模板已导出".into(), cx),
                Err(e) => self.show_toast(format!("导出失败：{e}"), cx),
            }
        }
    }

    pub fn import_custom_template(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.template_dialog_open {
            self.show_toast("请先处理当前草稿".into(), cx);
            return;
        }
        let Some(path) = rfd::FileDialog::new()
            .add_filter("SwiftSub 模板", &["json"])
            .pick_file()
        else {
            return;
        };
        let result = std::fs::read_to_string(path)
            .map_err(|e| e.to_string())
            .and_then(|s| TemplateFile::decode(&s))
            .and_then(|file| self.validate_template_snapshot(&file.config).map(|_| file));
        match result {
            Ok(file) => self.load_template_draft(file.config, file.name, None, window, cx),
            Err(e) => self.show_toast(format!("导入失败：{e}"), cx),
        }
    }

    pub fn delete_custom_template(&mut self, id: i64, cx: &mut Context<Self>) {
        let result = self
            .store
            .lock()
            .expect("store mutex poisoned")
            .delete_custom_template(id);
        match result {
            Ok(()) => {
                self.load_templates();
                self.show_toast("模板已删除".into(), cx)
            }
            Err(e) => self.show_toast(format!("删除失败：{e}"), cx),
        }
    }

    pub fn apply_builtin_template(&mut self, preset: Preset, cx: &mut Context<Self>) {
        self.set_preset(preset, cx);
        self.page = Page::Generator;
        cx.notify();
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
        self.options.rule_set_targets.clear();
        self.save_module_config();
        self.save_rule_set_config();
        self.regenerate(cx);
        self.show_toast(format!("已切换为{}模板", preset.label()), cx);
    }

    pub fn toggle_module_category(&mut self, category: &str, cx: &mut Context<Self>) {
        if !self
            .module_categories_collapsed
            .insert(category.to_string())
        {
            self.module_categories_collapsed.remove(category);
        }
        cx.notify();
    }

    pub fn is_module_category_open(&self, category: &str) -> bool {
        !self.module_categories_collapsed.contains(category)
    }

    fn save_module_config(&self) {
        let store = self.store.lock().expect("store mutex poisoned");
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
        let store = self.store.lock().expect("store mutex poisoned");
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
        let store = self.store.lock().expect("store mutex poisoned");
        if let Some(value) = store.get_setting("preset").ok().flatten()
            && let Ok(preset) = serde_json::from_str::<Preset>(&value)
        {
            self.options.preset = preset;
        }
        if let Some(value) = store.get_setting("enabled_modules").ok().flatten()
            && let Ok(items) = serde_json::from_str::<Vec<String>>(&value)
            && !items.is_empty()
        {
            self.enabled_modules = items.into_iter().collect();
        }
    }

    pub fn is_module_enabled(&self, id: &str) -> bool {
        matches!(id, "select" | "auto" | "final") || self.enabled_modules.contains(id)
    }

    pub fn toggle_preview(&mut self, cx: &mut Context<Self>) {
        self.preview_open = !self.preview_open;
        cx.notify();
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
        let (Some(key), Some(sub_id)) = (self.node_rename_key.clone(), self.node_rename_sub_id)
        else {
            return;
        };
        let name = self.node_rename_input.read(cx).value().trim().to_string();
        if name.is_empty() {
            self.node_rename_error = Some("节点名称不能为空".to_string());
            cx.notify();
            return;
        }

        let yaml = self.node_edit_textarea.read(cx).value().trim().to_string();
        let mut edited = match Proxy::from_yaml_text(&yaml) {
            Ok(proxy) => proxy,
            Err(error) => {
                self.node_rename_error = Some(format!("节点 YAML 无效：{error}"));
                cx.notify();
                return;
            }
        };
        if let Some(issue) = edited.fatal_issue() {
            self.node_rename_error = Some(issue);
            cx.notify();
            return;
        }
        edited.set_name(name.clone());
        let edited_yaml = match edited.to_yaml_text() {
            Ok(yaml) => yaml,
            Err(error) => {
                self.node_rename_error = Some(format!("序列化节点失败：{error}"));
                cx.notify();
                return;
            }
        };

        {
            let store = self.store.lock().expect("store mutex poisoned");
            let _ = store.set_node_override_alias(sub_id, &key, &name);
            let _ = store.set_node_override_fields(sub_id, &key, &edited_yaml);
        }
        self.close_node_rename(window, cx);
        self.reload_subscriptions(cx);
        self.show_toast("节点已保存".to_string(), cx);
    }

    pub fn toggle_node(&mut self, key: String, enabled: bool, cx: &mut Context<Self>) {
        if let Some(node) = self.nodes.iter().find(|node| node.source_key == key)
            && let Ok(sub_id) = node.source_id.parse::<i64>()
        {
            let store = self.store.lock().expect("store mutex poisoned");
            let _ = store.set_node_override_enabled(sub_id, &key, enabled);
        }
        if enabled {
            self.disabled_nodes.remove(&key);
        } else {
            self.disabled_nodes.insert(key);
        }
        self.rebuild_nodes(cx);
    }

    pub fn move_node(&mut self, dragged: String, target: String, cx: &mut Context<Self>) {
        if dragged == target {
            return;
        }
        let Some(from) = self
            .nodes
            .iter()
            .position(|node| node.source_key == dragged)
        else {
            return;
        };
        let node = self.nodes.remove(from);
        let to = self
            .nodes
            .iter()
            .position(|node| node.source_key == target)
            .unwrap_or(self.nodes.len());
        self.nodes.insert(to, node);
        self.node_order = self
            .nodes
            .iter()
            .map(|node| node.source_key.clone())
            .collect();
        if let Ok(value) = serde_json::to_string(&self.node_order) {
            let store = self.store.lock().expect("store mutex poisoned");
            let _ = store.set_setting("node_order", &value);
        }
        self.regenerate(cx);
    }

    pub fn copy_text(&mut self, text: String, cx: &mut Context<Self>) {
        cx.write_to_clipboard(ClipboardItem::new_string(text));
        self.show_toast("已复制到剪贴板".to_string(), cx);
    }

    pub fn copy_server_url(&mut self, cx: &mut Context<Self>) {
        if let Some(url) = self.server_url.clone() {
            self.copy_text(url, cx);
        } else {
            self.show_toast("本地服务未启动".to_string(), cx);
        }
    }

    pub fn copy_preview(&mut self, cx: &mut Context<Self>) {
        if self.preview.is_empty() {
            self.show_toast("暂无可复制的配置".to_string(), cx);
        } else {
            self.copy_text(self.preview.clone(), cx);
        }
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
            self.server_url = Some(server.url_for(&self.subscription_filename));
        }
    }

    pub fn save_subscription_filename(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let input = self
            .subscription_filename_input
            .read(cx)
            .value()
            .trim()
            .to_string();
        match normalize_profile_filename(&input) {
            Ok(filename) => {
                self.subscription_filename = filename.clone();
                {
                    let store = self.store.lock().expect("store mutex poisoned");
                    let _ = store.set_setting("subscription_filename", &filename);
                }
                self.subscription_filename_input
                    .update(cx, |state, cx| state.set_value(filename, window, cx));
                self.refresh_server_url();
                self.show_toast("订阅文件名已更新".to_string(), cx);
            }
            Err(error) => self.show_toast(error, cx),
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
            let store = self.store.lock().expect("store mutex poisoned");
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

    pub fn change_data_dir(&mut self, cx: &mut Context<Self>) {
        let Some(selected) = rfd::FileDialog::new().pick_folder() else {
            return;
        };
        if selected == self.data_dir {
            self.show_toast("已经是当前数据目录".to_string(), cx);
            return;
        }
        if let Err(error) = std::fs::create_dir_all(&selected) {
            self.show_toast(format!("创建目录失败：{error}"), cx);
            return;
        }
        let target = selected.join("swiftsub.db");
        if target.exists() {
            self.show_toast("目标目录已存在 swiftsub.db，请换一个目录".to_string(), cx);
            return;
        }

        let result = {
            let store = self.store.lock().expect("store mutex poisoned");
            store.backup_to(target.to_string_lossy().as_ref())
        };
        match result {
            Ok(()) => {
                let bootstrap = bootstrap_dir();
                if let Err(error) = std::fs::create_dir_all(&bootstrap) {
                    self.show_toast(format!("写入数据目录配置失败：{error}"), cx);
                    return;
                }
                if let Err(error) = std::fs::write(
                    bootstrap.join("location.txt"),
                    selected.display().to_string(),
                ) {
                    self.show_toast(format!("写入数据目录配置失败：{error}"), cx);
                    return;
                }
                self.data_dir = selected;
                self.show_toast("数据目录已更改，重启应用后生效".to_string(), cx);
            }
            Err(error) => self.show_toast(format!("复制数据库失败：{error}"), cx),
        }
    }

    pub fn aggregate_userinfo(&self) -> Option<String> {
        let mut info = SubscriptionInfo::default();
        let mut has_data = false;
        let mut has_expire = false;

        for subscription in self
            .subscriptions
            .iter()
            .filter(|subscription| subscription.enabled)
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

    pub fn subscriptions_summary(&self) -> (usize, usize) {
        let enabled = self
            .subscriptions
            .iter()
            .filter(|subscription| subscription.enabled)
            .count();
        (enabled, self.subscriptions.len())
    }

    pub fn aggregate_traffic(&self) -> (u64, u64) {
        let mut used: u64 = 0;
        let mut total: u64 = 0;
        for subscription in self
            .subscriptions
            .iter()
            .filter(|subscription| subscription.enabled)
        {
            used = used.saturating_add(
                subscription
                    .traffic_upload
                    .saturating_add(subscription.traffic_download),
            );
            total = total.saturating_add(subscription.traffic_total);
        }
        (used, total)
    }
}

pub fn data_dir() -> PathBuf {
    let base = bootstrap_dir();
    if let Ok(location) = std::fs::read_to_string(base.join("location.txt")) {
        let location = location.trim();
        if !location.is_empty() {
            return PathBuf::from(location);
        }
    }
    base
}

pub fn bootstrap_dir() -> PathBuf {
    if let Ok(appdata) = std::env::var("APPDATA") {
        PathBuf::from(appdata).join("SwiftSub")
    } else {
        PathBuf::from(".swiftsub")
    }
}

fn normalize_profile_filename(input: &str) -> Result<String, String> {
    let mut name = input.trim().trim_start_matches('/').to_string();
    if name.is_empty() {
        return Err("订阅文件名不能为空".to_string());
    }
    if name.contains(['/', '\\', '?', '#', '%', ' ']) {
        return Err("文件名不能包含路径分隔符或空格".to_string());
    }

    let lower = name.to_lowercase();
    if lower.ends_with(".yaml") {
        name = format!("{}.yaml", &name[..name.len() - 5]);
    } else if lower.ends_with(".yml") {
        name = format!("{}.yml", &name[..name.len() - 4]);
    } else {
        name.push_str(".yaml");
    }

    let stem = name.trim_end_matches(".yaml").trim_end_matches(".yml");
    if stem.is_empty() {
        return Err("订阅文件名不能为空".to_string());
    }
    if !name
        .chars()
        .all(|character| character.is_ascii_alphanumeric() || matches!(character, '-' | '_' | '.'))
    {
        return Err("文件名只能包含字母、数字、-、_ 和 .".to_string());
    }
    Ok(name)
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

pub fn format_bytes(value: u64) -> String {
    if value == 0 {
        return "—".to_string();
    }
    const UNITS: &[&str] = &["B", "KB", "MB", "GB", "TB"];
    let mut size = value as f64;
    let mut unit = 0;
    while size >= 1024.0 && unit < UNITS.len() - 1 {
        size /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{} {}", value, UNITS[unit])
    } else {
        format!("{size:.1} {}", UNITS[unit])
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

pub fn format_expire(timestamp: Option<i64>) -> String {
    let Some(timestamp) = timestamp else {
        return "—".to_string();
    };
    let Some(datetime) = Local.timestamp_opt(timestamp, 0).single() else {
        return "—".to_string();
    };
    datetime.format("%Y-%m-%d").to_string()
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
        let snapshot: TemplateSnapshot = serde_json::from_value(json).unwrap();
        let restored: TemplateSnapshot =
            serde_json::from_str(&serde_json::to_string(&snapshot).unwrap()).unwrap();
        assert_eq!(snapshot.policy_groups, restored.policy_groups);
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
}
