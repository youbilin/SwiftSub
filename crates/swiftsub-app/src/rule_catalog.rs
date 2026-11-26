use std::collections::HashSet;

use swiftsub_core::catalog;
use swiftsub_core::generator::RuleSetBinding;

pub const SELECT_GROUP: &str = "节点选择";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SourceKind {
    MetaCube,
    GitHub,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mirror {
    GitHub,
    JsDelivr,
}

#[derive(Clone, Copy, Debug)]
pub enum TargetHint {
    Module(&'static str),
    Policy(&'static str),
    Select,
}

#[derive(Clone, Copy, Debug)]
pub struct SourceRuleSet {
    pub id: &'static str,
    pub name: &'static str,
    pub path: &'static str,
    pub behavior: &'static str,
    pub format: &'static str,
    pub no_resolve: bool,
    pub target: TargetHint,
}

#[derive(Clone, Copy, Debug)]
pub struct RuleSetSource {
    pub id: &'static str,
    pub name: &'static str,
    pub kind: SourceKind,
    pub owner: &'static str,
    pub repo: &'static str,
    pub branch: &'static str,
    pub entries: &'static [SourceRuleSet],
}

const fn set(
    id: &'static str,
    name: &'static str,
    path: &'static str,
    behavior: &'static str,
    format: &'static str,
    no_resolve: bool,
    target: TargetHint,
) -> SourceRuleSet {
    SourceRuleSet {
        id,
        name,
        path,
        behavior,
        format,
        no_resolve,
        target,
    }
}

const MCX_ENTRIES: &[SourceRuleSet] = &[
    set(
        "ads-extra",
        "广告域名合集",
        "geosite/category-ads.mrs",
        "domain",
        "mrs",
        false,
        TargetHint::Module("ad"),
    ),
    set(
        "games-all",
        "游戏合集",
        "geosite/category-games.mrs",
        "domain",
        "mrs",
        false,
        TargetHint::Select,
    ),
    set(
        "media-all",
        "流媒体合集",
        "geosite/category-media.mrs",
        "domain",
        "mrs",
        false,
        TargetHint::Select,
    ),
    set(
        "hbo",
        "HBO",
        "geosite/hbo.mrs",
        "domain",
        "mrs",
        false,
        TargetHint::Select,
    ),
    set(
        "hulu",
        "Hulu",
        "geosite/hulu.mrs",
        "domain",
        "mrs",
        false,
        TargetHint::Select,
    ),
    set(
        "primevideo",
        "Prime Video",
        "geosite/primevideo.mrs",
        "domain",
        "mrs",
        false,
        TargetHint::Select,
    ),
    set(
        "twitch",
        "Twitch",
        "geosite/twitch.mrs",
        "domain",
        "mrs",
        false,
        TargetHint::Select,
    ),
    set(
        "bilibili",
        "哔哩哔哩",
        "geosite/bilibili.mrs",
        "domain",
        "mrs",
        false,
        TargetHint::Policy("DIRECT"),
    ),
    set(
        "iqiyi",
        "爱奇艺",
        "geosite/iqiyi.mrs",
        "domain",
        "mrs",
        false,
        TargetHint::Policy("DIRECT"),
    ),
    set(
        "bahamut",
        "巴哈姆特",
        "geosite/bahamut.mrs",
        "domain",
        "mrs",
        false,
        TargetHint::Select,
    ),
    set(
        "zhihu",
        "知乎",
        "geosite/zhihu.mrs",
        "domain",
        "mrs",
        false,
        TargetHint::Policy("DIRECT"),
    ),
    set(
        "xiaohongshu",
        "小红书",
        "geosite/xiaohongshu.mrs",
        "domain",
        "mrs",
        false,
        TargetHint::Policy("DIRECT"),
    ),
    set(
        "douyin",
        "抖音",
        "geosite/douyin.mrs",
        "domain",
        "mrs",
        false,
        TargetHint::Policy("DIRECT"),
    ),
    set(
        "kuaishou",
        "快手",
        "geosite/kuaishou.mrs",
        "domain",
        "mrs",
        false,
        TargetHint::Policy("DIRECT"),
    ),
    set(
        "tencent",
        "腾讯（微信/QQ）",
        "geosite/tencent.mrs",
        "domain",
        "mrs",
        false,
        TargetHint::Policy("DIRECT"),
    ),
    set(
        "alibaba",
        "阿里巴巴（支付宝/淘宝）",
        "geosite/alibaba.mrs",
        "domain",
        "mrs",
        false,
        TargetHint::Policy("DIRECT"),
    ),
    set(
        "jd",
        "京东",
        "geosite/jd.mrs",
        "domain",
        "mrs",
        false,
        TargetHint::Policy("DIRECT"),
    ),
    set(
        "baidu",
        "百度",
        "geosite/baidu.mrs",
        "domain",
        "mrs",
        false,
        TargetHint::Policy("DIRECT"),
    ),
    set(
        "binance",
        "Binance",
        "geosite/binance.mrs",
        "domain",
        "mrs",
        false,
        TargetHint::Select,
    ),
    set(
        "okx",
        "OKX",
        "geosite/okx.mrs",
        "domain",
        "mrs",
        false,
        TargetHint::Select,
    ),
    set(
        "bybit",
        "Bybit",
        "geosite/bybit.mrs",
        "domain",
        "mrs",
        false,
        TargetHint::Select,
    ),
    set(
        "perplexity",
        "Perplexity",
        "geosite/perplexity.mrs",
        "domain",
        "mrs",
        false,
        TargetHint::Module("ai"),
    ),
    set(
        "gemini",
        "Google Gemini",
        "geosite/google-gemini.mrs",
        "domain",
        "mrs",
        false,
        TargetHint::Module("ai"),
    ),
    set(
        "copilot",
        "GitHub Copilot",
        "geosite/github-copilot.mrs",
        "domain",
        "mrs",
        false,
        TargetHint::Module("ai"),
    ),
    set(
        "twitter-ip",
        "X (Twitter) IP",
        "geoip/twitter.mrs",
        "ipcidr",
        "mrs",
        true,
        TargetHint::Module("x"),
    ),
    set(
        "facebook-ip",
        "Facebook IP",
        "geoip/facebook.mrs",
        "ipcidr",
        "mrs",
        true,
        TargetHint::Module("meta"),
    ),
];

const BM_ENTRIES: &[SourceRuleSet] = &[
    set(
        "netflix",
        "Netflix",
        "rule/Clash/Netflix/Netflix.yaml",
        "classical",
        "yaml",
        false,
        TargetHint::Module("netflix"),
    ),
    set(
        "youtube",
        "YouTube",
        "rule/Clash/YouTube/YouTube.yaml",
        "classical",
        "yaml",
        false,
        TargetHint::Module("youtube"),
    ),
    set(
        "spotify",
        "Spotify",
        "rule/Clash/Spotify/Spotify.yaml",
        "classical",
        "yaml",
        false,
        TargetHint::Module("spotify"),
    ),
    set(
        "telegram",
        "Telegram",
        "rule/Clash/Telegram/Telegram.yaml",
        "classical",
        "yaml",
        false,
        TargetHint::Module("telegram"),
    ),
    set(
        "github",
        "GitHub",
        "rule/Clash/GitHub/GitHub.yaml",
        "classical",
        "yaml",
        false,
        TargetHint::Module("github"),
    ),
    set(
        "microsoft",
        "Microsoft",
        "rule/Clash/Microsoft/Microsoft.yaml",
        "classical",
        "yaml",
        false,
        TargetHint::Module("microsoft"),
    ),
    set(
        "apple",
        "Apple",
        "rule/Clash/Apple/Apple.yaml",
        "classical",
        "yaml",
        false,
        TargetHint::Module("apple"),
    ),
    set(
        "steam",
        "Steam",
        "rule/Clash/Steam/Steam.yaml",
        "classical",
        "yaml",
        false,
        TargetHint::Module("steam"),
    ),
    set(
        "paypal",
        "PayPal",
        "rule/Clash/PayPal/PayPal.yaml",
        "classical",
        "yaml",
        false,
        TargetHint::Module("paypal"),
    ),
    set(
        "tiktok",
        "TikTok",
        "rule/Clash/TikTok/TikTok.yaml",
        "classical",
        "yaml",
        false,
        TargetHint::Module("tiktok"),
    ),
    set(
        "discord",
        "Discord",
        "rule/Clash/Discord/Discord.yaml",
        "classical",
        "yaml",
        false,
        TargetHint::Module("discord"),
    ),
    set(
        "disney",
        "Disney+",
        "rule/Clash/Disney/Disney.yaml",
        "classical",
        "yaml",
        false,
        TargetHint::Module("disney"),
    ),
    set(
        "openai",
        "OpenAI",
        "rule/Clash/OpenAI/OpenAI.yaml",
        "classical",
        "yaml",
        false,
        TargetHint::Module("ai"),
    ),
    set(
        "amazon",
        "Amazon",
        "rule/Clash/Amazon/Amazon.yaml",
        "classical",
        "yaml",
        false,
        TargetHint::Select,
    ),
    set(
        "hbo",
        "HBO",
        "rule/Clash/HBO/HBO.yaml",
        "classical",
        "yaml",
        false,
        TargetHint::Select,
    ),
    set(
        "hulu",
        "Hulu",
        "rule/Clash/Hulu/Hulu.yaml",
        "classical",
        "yaml",
        false,
        TargetHint::Select,
    ),
    set(
        "primevideo",
        "Prime Video",
        "rule/Clash/PrimeVideo/PrimeVideo.yaml",
        "classical",
        "yaml",
        false,
        TargetHint::Select,
    ),
    set(
        "twitch",
        "Twitch",
        "rule/Clash/Twitch/Twitch.yaml",
        "classical",
        "yaml",
        false,
        TargetHint::Select,
    ),
    set(
        "playstation",
        "PlayStation",
        "rule/Clash/PlayStation/PlayStation.yaml",
        "classical",
        "yaml",
        false,
        TargetHint::Select,
    ),
    set(
        "xbox",
        "Xbox",
        "rule/Clash/Xbox/Xbox.yaml",
        "classical",
        "yaml",
        false,
        TargetHint::Select,
    ),
    set(
        "nintendo",
        "Nintendo",
        "rule/Clash/Nintendo/Nintendo.yaml",
        "classical",
        "yaml",
        false,
        TargetHint::Select,
    ),
];

const LOY_ENTRIES: &[SourceRuleSet] = &[
    set(
        "reject",
        "广告与追踪拦截",
        "reject.txt",
        "domain",
        "yaml",
        false,
        TargetHint::Policy("REJECT"),
    ),
    set(
        "proxy",
        "常被墙域名",
        "proxy.txt",
        "domain",
        "yaml",
        false,
        TargetHint::Select,
    ),
    set(
        "direct",
        "直连域名",
        "direct.txt",
        "domain",
        "yaml",
        false,
        TargetHint::Policy("DIRECT"),
    ),
    set(
        "gfw",
        "GFW 域名",
        "gfw.txt",
        "domain",
        "yaml",
        false,
        TargetHint::Select,
    ),
    set(
        "tld",
        "非中国 TLD",
        "tld-not-cn.txt",
        "domain",
        "yaml",
        false,
        TargetHint::Select,
    ),
    set(
        "cncidr",
        "国内 IP",
        "cncidr.txt",
        "ipcidr",
        "yaml",
        true,
        TargetHint::Policy("DIRECT"),
    ),
    set(
        "telegramcidr",
        "Telegram IP",
        "telegramcidr.txt",
        "ipcidr",
        "yaml",
        true,
        TargetHint::Module("telegram"),
    ),
    set(
        "lancidr",
        "局域网 IP",
        "lancidr.txt",
        "ipcidr",
        "yaml",
        true,
        TargetHint::Policy("DIRECT"),
    ),
    set(
        "private",
        "私有域名",
        "private.txt",
        "domain",
        "yaml",
        false,
        TargetHint::Policy("DIRECT"),
    ),
    set(
        "apple",
        "Apple 域名",
        "apple.txt",
        "domain",
        "yaml",
        false,
        TargetHint::Module("apple"),
    ),
    set(
        "google",
        "Google 域名",
        "google.txt",
        "domain",
        "yaml",
        false,
        TargetHint::Module("google"),
    ),
    set(
        "icloud",
        "iCloud 域名",
        "icloud.txt",
        "domain",
        "yaml",
        false,
        TargetHint::Module("apple"),
    ),
    set(
        "applications",
        "常用应用",
        "applications.txt",
        "domain",
        "yaml",
        false,
        TargetHint::Select,
    ),
    set(
        "greatfire",
        "GreatFire 测试域名",
        "greatfire.txt",
        "domain",
        "yaml",
        false,
        TargetHint::Select,
    ),
];

const ACL_ENTRIES: &[SourceRuleSet] = &[
    set(
        "netflix",
        "Netflix",
        "Clash/Netflix.list",
        "classical",
        "text",
        false,
        TargetHint::Module("netflix"),
    ),
    set(
        "telegram",
        "Telegram",
        "Clash/Telegram.list",
        "classical",
        "text",
        false,
        TargetHint::Module("telegram"),
    ),
    set(
        "apple",
        "Apple",
        "Clash/Apple.list",
        "classical",
        "text",
        false,
        TargetHint::Module("apple"),
    ),
    set(
        "microsoft",
        "Microsoft",
        "Clash/Microsoft.list",
        "classical",
        "text",
        false,
        TargetHint::Module("microsoft"),
    ),
    set(
        "banad",
        "广告拦截",
        "Clash/BanAD.list",
        "classical",
        "text",
        false,
        TargetHint::Policy("REJECT"),
    ),
    set(
        "banprogramad",
        "程序广告拦截",
        "Clash/BanProgramAD.list",
        "classical",
        "text",
        false,
        TargetHint::Policy("REJECT"),
    ),
    set(
        "proxylite",
        "代理域名合集",
        "Clash/ProxyLite.list",
        "classical",
        "text",
        false,
        TargetHint::Select,
    ),
    set(
        "chinadomain",
        "国内域名",
        "Clash/ChinaDomain.list",
        "classical",
        "text",
        false,
        TargetHint::Policy("DIRECT"),
    ),
    set(
        "chinacompanyip",
        "国内企业 IP",
        "Clash/ChinaCompanyIp.list",
        "classical",
        "text",
        false,
        TargetHint::Policy("DIRECT"),
    ),
    set(
        "chinaip",
        "国内 IP 合集",
        "Clash/ChinaIp.list",
        "classical",
        "text",
        false,
        TargetHint::Policy("DIRECT"),
    ),
];

pub static SOURCES: &[RuleSetSource] = &[
    RuleSetSource {
        id: "mcx",
        name: "MetaCubeX meta-rules-dat",
        kind: SourceKind::MetaCube,
        owner: "MetaCubeX",
        repo: "meta-rules-dat",
        branch: "meta",
        entries: MCX_ENTRIES,
    },
    RuleSetSource {
        id: "bm",
        name: "blackmatrix7 ios_rule_script",
        kind: SourceKind::GitHub,
        owner: "blackmatrix7",
        repo: "ios_rule_script",
        branch: "master",
        entries: BM_ENTRIES,
    },
    RuleSetSource {
        id: "loy",
        name: "Loyalsoldier clash-rules",
        kind: SourceKind::GitHub,
        owner: "Loyalsoldier",
        repo: "clash-rules",
        branch: "release",
        entries: LOY_ENTRIES,
    },
    RuleSetSource {
        id: "acl",
        name: "ACL4SSR Rules",
        kind: SourceKind::GitHub,
        owner: "ACL4SSR",
        repo: "ACL4SSR",
        branch: "master",
        entries: ACL_ENTRIES,
    },
];

pub fn mirror_from_base(base: &str) -> Mirror {
    if base.contains("jsdelivr") {
        Mirror::JsDelivr
    } else {
        Mirror::GitHub
    }
}

pub fn resolve_url(
    source: &RuleSetSource,
    entry: &SourceRuleSet,
    mirror: Mirror,
    metacube_base: &str,
) -> String {
    match source.kind {
        SourceKind::MetaCube => {
            format!("{}/{}", metacube_base.trim_end_matches('/'), entry.path)
        }
        SourceKind::GitHub => match mirror {
            Mirror::GitHub => format!(
                "https://raw.githubusercontent.com/{}/{}/{}/{}",
                source.owner, source.repo, source.branch, entry.path
            ),
            Mirror::JsDelivr => format!(
                "https://cdn.jsdelivr.net/gh/{}/{}@{}/{}",
                source.owner, source.repo, source.branch, entry.path
            ),
        },
    }
}

pub fn resolve_target(hint: TargetHint, _enabled_modules: &HashSet<String>) -> String {
    match hint {
        TargetHint::Module(id) => catalog::module(id)
            .map(|module| module.name.to_string())
            .unwrap_or_else(|| SELECT_GROUP.to_string()),
        TargetHint::Policy(policy) => policy.to_string(),
        TargetHint::Select => SELECT_GROUP.to_string(),
    }
}

pub fn binding_for(
    source: &RuleSetSource,
    entry: &SourceRuleSet,
    mirror: Mirror,
    metacube_base: &str,
    enabled_modules: &HashSet<String>,
) -> RuleSetBinding {
    RuleSetBinding {
        id: format!("{}-{}", source.id, entry.id),
        name: entry.name.to_string(),
        url: resolve_url(source, entry, mirror, metacube_base),
        behavior: entry.behavior.to_string(),
        format: entry.format.to_string(),
        target: resolve_target(entry.target, enabled_modules),
        no_resolve: entry.no_resolve,
    }
}

/// Update only the requested entries of one source; keep other sources and manual URLs.
#[cfg(test)]
pub fn set_source_selection(
    bindings: &mut Vec<RuleSetBinding>,
    source: &RuleSetSource,
    entry_ids: &[String],
    enabled: bool,
    metacube_base: &str,
    enabled_modules: &HashSet<String>,
) {
    let mirror = mirror_from_base(metacube_base);
    for entry in source
        .entries
        .iter()
        .filter(|entry| entry_ids.iter().any(|id| id == entry.id))
    {
        let id = format!("{}-{}", source.id, entry.id);
        if enabled {
            if !bindings.iter().any(|binding| binding.id == id) {
                bindings.push(binding_for(
                    source,
                    entry,
                    mirror,
                    metacube_base,
                    enabled_modules,
                ));
            }
        } else {
            bindings.retain(|binding| binding.id != id);
        }
    }
}

/// Re-resolve catalog URLs when the mirror changes, preserving manual URLs and routing.
pub fn refresh_binding_url(binding: &mut RuleSetBinding, metacube_base: &str) {
    for source in SOURCES {
        if let Some(entry) = source
            .entries
            .iter()
            .find(|entry| binding.id == format!("{}-{}", source.id, entry.id))
        {
            let managed = url::Url::parse(&binding.url).is_ok_and(|url| {
                if url.scheme() != "https" || url.query().is_some() || url.fragment().is_some() {
                    return false;
                }
                let raw_path = format!(
                    "/{}/{}/{}/{}",
                    source.owner, source.repo, source.branch, entry.path
                );
                let cdn_path = format!(
                    "/gh/{}/{}@{}/{}",
                    source.owner, source.repo, source.branch, entry.path
                );
                let github_path = format!(
                    "/{}/{}/raw/refs/heads/{}/{}",
                    source.owner, source.repo, source.branch, entry.path
                );
                let with_geo = |path: &str| {
                    source.kind == SourceKind::MetaCube
                        && url.path()
                            == format!("{}geo/{}", path.trim_end_matches(entry.path), entry.path)
                };
                match url.host_str() {
                    Some("github.com") => url.path() == github_path || with_geo(&github_path),
                    Some("raw.githubusercontent.com") => {
                        url.path() == raw_path || with_geo(&raw_path)
                    }
                    Some("cdn.jsdelivr.net" | "fastly.jsdelivr.net" | "testingcf.jsdelivr.net") => {
                        url.path() == cdn_path || with_geo(&cdn_path)
                    }
                    _ => false,
                }
            });
            if !managed {
                return;
            }
            binding.url = resolve_url(
                source,
                entry,
                mirror_from_base(metacube_base),
                metacube_base,
            );
            return;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn disabled_service_keeps_its_intended_target_for_explicit_enablement() {
        assert_eq!(
            resolve_target(TargetHint::Module("netflix"), &HashSet::new()),
            "Netflix"
        );
    }

    #[test]
    fn source_selection_accumulates_and_clears_only_requested_entries() {
        let base = "https://raw.githubusercontent.com/MetaCubeX/meta-rules-dat/meta";
        let modules = HashSet::new();
        let mut bindings = Vec::new();
        for source in SOURCES {
            let ids = vec![
                source.entries[0].id.to_string(),
                source.entries[1].id.to_string(),
            ];
            set_source_selection(&mut bindings, source, &ids, true, base, &modules);
            set_source_selection(&mut bindings, source, &ids, true, base, &modules);
        }
        assert_eq!(bindings.len(), SOURCES.len() * 2);
        let selected = bindings.clone();
        let ids = vec![SOURCES[0].entries[0].id.to_string()];
        set_source_selection(&mut bindings, &SOURCES[0], &ids, false, base, &modules);
        assert_eq!(bindings.len(), selected.len() - 1);
        for binding in &selected[1..] {
            assert!(bindings.iter().any(|other| other.id == binding.id));
        }
        // Template serialization preserves every source without another selection field.
        let json = serde_json::to_string(&bindings).unwrap();
        let restored: Vec<RuleSetBinding> = serde_json::from_str(&json).unwrap();
        assert_eq!(restored.len(), bindings.len());
        for (left, right) in bindings.iter().zip(&restored) {
            assert_eq!(left.id, right.id);
            assert_eq!(left.url, right.url);
        }
    }

    #[test]
    fn mirror_changes_refresh_all_catalog_sources_without_changing_targets() {
        let modules = HashSet::new();
        for source in SOURCES {
            let mut binding = binding_for(
                source,
                &source.entries[0],
                Mirror::GitHub,
                "https://raw.githubusercontent.com/MetaCubeX/meta-rules-dat/meta",
                &modules,
            );
            binding.target = "DIRECT".into();
            refresh_binding_url(
                &mut binding,
                "https://cdn.jsdelivr.net/gh/MetaCubeX/meta-rules-dat@meta",
            );
            assert!(binding.url.starts_with("https://cdn.jsdelivr.net/gh/"));
            assert_eq!(binding.target, "DIRECT");
            refresh_binding_url(
                &mut binding,
                "https://raw.githubusercontent.com/MetaCubeX/meta-rules-dat/meta",
            );
            assert!(
                binding
                    .url
                    .starts_with("https://raw.githubusercontent.com/")
            );
        }
        let mut manual = binding_for(
            &SOURCES[0],
            &SOURCES[0].entries[0],
            Mirror::GitHub,
            "https://example.com",
            &modules,
        );
        manual.id = "manual-custom-url".into();
        let original = manual.url.clone();
        refresh_binding_url(
            &mut manual,
            "https://cdn.jsdelivr.net/gh/MetaCubeX/meta-rules-dat@meta",
        );
        assert_eq!(manual.url, original);

        for base in [
            "https://github.com/MetaCubeX/meta-rules-dat/raw/refs/heads/meta/geo",
            "https://raw.githubusercontent.com/MetaCubeX/meta-rules-dat/meta/geo",
            "https://fastly.jsdelivr.net/gh/MetaCubeX/meta-rules-dat@meta/geo",
            "https://testingcf.jsdelivr.net/gh/MetaCubeX/meta-rules-dat@meta/geo",
        ] {
            let mut binding = binding_for(
                &SOURCES[0],
                &SOURCES[0].entries[0],
                Mirror::GitHub,
                base,
                &modules,
            );
            refresh_binding_url(
                &mut binding,
                "https://cdn.jsdelivr.net/gh/MetaCubeX/meta-rules-dat@meta/geo",
            );
            assert!(binding.url.starts_with("https://cdn.jsdelivr.net/gh/"));
        }
    }

    #[test]
    fn mirror_changes_preserve_edited_catalog_urls() {
        for source in SOURCES {
            let mut binding = binding_for(
                source,
                &source.entries[0],
                Mirror::GitHub,
                "https://raw.githubusercontent.com/MetaCubeX/meta-rules-dat/meta",
                &HashSet::new(),
            );
            let original = binding.url.clone();
            for edited in [
                "https://example.com/custom.yaml".to_string(),
                format!("{original}?version=custom"),
                original.replace(source.branch, "custom-branch"),
            ] {
                binding.url = edited.clone();
                refresh_binding_url(
                    &mut binding,
                    "https://cdn.jsdelivr.net/gh/MetaCubeX/meta-rules-dat@meta",
                );
                assert_eq!(binding.url, edited);
            }
        }
    }
}
