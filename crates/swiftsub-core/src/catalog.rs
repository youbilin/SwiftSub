use crate::region::Region;

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Preset {
    Minimal,
    Standard,
    Full,
}

impl Preset {
    pub fn label(self) -> &'static str {
        match self {
            Preset::Minimal => "精简",
            Preset::Standard => "标准",
            Preset::Full => "完整",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ModuleKind {
    Select,
    Auto,
    Region(Region),
    Service,
    DirectFirst,
    RejectFirst,
    Final,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RuleSetBehavior {
    Domain,
    IpCidr,
    Classical,
}

impl RuleSetBehavior {
    pub fn as_str(self) -> &'static str {
        match self {
            RuleSetBehavior::Domain => "domain",
            RuleSetBehavior::IpCidr => "ipcidr",
            RuleSetBehavior::Classical => "classical",
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct RuleSetSpec {
    pub id: &'static str,
    pub behavior: RuleSetBehavior,
    pub path: &'static str,
    pub no_resolve: bool,
}

#[derive(Clone, Copy, Debug)]
pub struct Module {
    pub id: &'static str,
    pub name: &'static str,
    pub description: &'static str,
    pub kind: ModuleKind,
    pub rule_sets: &'static [RuleSetSpec],
    pub presets: &'static [Preset],
}

const fn domain(id: &'static str, path: &'static str) -> RuleSetSpec {
    RuleSetSpec {
        id,
        behavior: RuleSetBehavior::Domain,
        path,
        no_resolve: false,
    }
}

const fn ipcidr(id: &'static str, path: &'static str) -> RuleSetSpec {
    RuleSetSpec {
        id,
        behavior: RuleSetBehavior::IpCidr,
        path,
        no_resolve: true,
    }
}

const fn make_module(
    id: &'static str,
    name: &'static str,
    description: &'static str,
    kind: ModuleKind,
    rule_sets: &'static [RuleSetSpec],
    presets: &'static [Preset],
) -> Module {
    Module {
        id,
        name,
        description,
        kind,
        rule_sets,
        presets,
    }
}

const ALL_PRESETS: &[Preset] = &[Preset::Minimal, Preset::Standard, Preset::Full];
const STANDARD_PRESETS: &[Preset] = &[Preset::Standard, Preset::Full];
const FULL_PRESETS: &[Preset] = &[Preset::Full];

const AD_RULES: &[RuleSetSpec] = &[domain("category-ads-all", "geosite/category-ads-all.mrs")];
const PRIVATE_RULES: &[RuleSetSpec] = &[
    domain("private", "geosite/private.mrs"),
    ipcidr("private-ip", "geoip/private.mrs"),
];
const CN_RULES: &[RuleSetSpec] = &[
    domain("geolocation-cn", "geosite/geolocation-cn.mrs"),
    ipcidr("cn-ip", "geoip/cn.mrs"),
];
const GLOBAL_RULES: &[RuleSetSpec] = &[domain("geolocation-!cn", "geosite/geolocation-!cn.mrs")];
const AI_RULES: &[RuleSetSpec] = &[
    domain("openai", "geosite/openai.mrs"),
    domain("anthropic", "geosite/anthropic.mrs"),
    domain("category-ai-chat-!cn", "geosite/category-ai-chat-!cn.mrs"),
];
const GOOGLE_RULES: &[RuleSetSpec] = &[
    domain("google", "geosite/google.mrs"),
    ipcidr("google-ip", "geoip/google.mrs"),
];
const YOUTUBE_RULES: &[RuleSetSpec] = &[domain("youtube", "geosite/youtube.mrs")];
const TELEGRAM_RULES: &[RuleSetSpec] = &[
    domain("telegram", "geosite/telegram.mrs"),
    ipcidr("telegram-ip", "geoip/telegram.mrs"),
];
const GITHUB_RULES: &[RuleSetSpec] = &[domain("github", "geosite/github.mrs")];
const MICROSOFT_RULES: &[RuleSetSpec] = &[domain("microsoft", "geosite/microsoft.mrs")];
const APPLE_RULES: &[RuleSetSpec] = &[domain("apple", "geosite/apple.mrs")];
const NETFLIX_RULES: &[RuleSetSpec] = &[
    domain("netflix", "geosite/netflix.mrs"),
    ipcidr("netflix-ip", "geoip/netflix.mrs"),
];
const SPOTIFY_RULES: &[RuleSetSpec] = &[domain("spotify", "geosite/spotify.mrs")];
const DISCORD_RULES: &[RuleSetSpec] = &[domain("discord", "geosite/discord.mrs")];
const X_RULES: &[RuleSetSpec] = &[domain("twitter", "geosite/twitter.mrs")];
const DISNEY_RULES: &[RuleSetSpec] = &[domain("disney", "geosite/disney.mrs")];
const TIKTOK_RULES: &[RuleSetSpec] = &[domain("tiktok", "geosite/tiktok.mrs")];
const META_RULES: &[RuleSetSpec] = &[
    domain("facebook", "geosite/facebook.mrs"),
    domain("instagram", "geosite/instagram.mrs"),
];
const STEAM_RULES: &[RuleSetSpec] = &[
    domain("steam", "geosite/steam.mrs"),
    ipcidr("steam-ip", "geoip/steam.mrs"),
];
const PAYPAL_RULES: &[RuleSetSpec] = &[domain("paypal", "geosite/paypal.mrs")];

pub static MODULES: &[Module] = &[
    make_module(
        "select",
        "节点选择",
        "手动选择代理节点",
        ModuleKind::Select,
        &[],
        ALL_PRESETS,
    ),
    make_module(
        "auto",
        "自动选择",
        "自动选择延迟最低的节点",
        ModuleKind::Auto,
        &[],
        ALL_PRESETS,
    ),
    make_module(
        "ad",
        "广告拦截",
        "拦截广告与追踪域名",
        ModuleKind::RejectFirst,
        AD_RULES,
        ALL_PRESETS,
    ),
    make_module(
        "private",
        "私有网络",
        "局域网与私有 IP 直连",
        ModuleKind::DirectFirst,
        PRIVATE_RULES,
        ALL_PRESETS,
    ),
    make_module(
        "cn",
        "国内直连",
        "国内域名与 IP 直连",
        ModuleKind::DirectFirst,
        CN_RULES,
        ALL_PRESETS,
    ),
    make_module(
        "global",
        "全球代理",
        "未命中其他规则的非中国域名",
        ModuleKind::Service,
        GLOBAL_RULES,
        STANDARD_PRESETS,
    ),
    make_module(
        "final",
        "漏网之鱼",
        "未匹配任何规则的流量",
        ModuleKind::Final,
        &[],
        ALL_PRESETS,
    ),
    make_module(
        "region-hk",
        "香港",
        "香港节点",
        ModuleKind::Region(Region::Hk),
        &[],
        STANDARD_PRESETS,
    ),
    make_module(
        "region-tw",
        "台湾",
        "台湾节点",
        ModuleKind::Region(Region::Tw),
        &[],
        STANDARD_PRESETS,
    ),
    make_module(
        "region-jp",
        "日本",
        "日本节点",
        ModuleKind::Region(Region::Jp),
        &[],
        STANDARD_PRESETS,
    ),
    make_module(
        "region-kr",
        "韩国",
        "韩国节点",
        ModuleKind::Region(Region::Kr),
        &[],
        STANDARD_PRESETS,
    ),
    make_module(
        "region-sg",
        "新加坡",
        "新加坡节点",
        ModuleKind::Region(Region::Sg),
        &[],
        STANDARD_PRESETS,
    ),
    make_module(
        "region-us",
        "美国",
        "美国节点",
        ModuleKind::Region(Region::Us),
        &[],
        STANDARD_PRESETS,
    ),
    make_module(
        "region-uk",
        "英国",
        "英国节点",
        ModuleKind::Region(Region::Uk),
        &[],
        STANDARD_PRESETS,
    ),
    make_module(
        "region-de",
        "德国",
        "德国节点",
        ModuleKind::Region(Region::De),
        &[],
        STANDARD_PRESETS,
    ),
    make_module(
        "region-fr",
        "法国",
        "法国节点",
        ModuleKind::Region(Region::Fr),
        &[],
        STANDARD_PRESETS,
    ),
    make_module(
        "region-nl",
        "荷兰",
        "荷兰节点",
        ModuleKind::Region(Region::Nl),
        &[],
        STANDARD_PRESETS,
    ),
    make_module(
        "region-ca",
        "加拿大",
        "加拿大节点",
        ModuleKind::Region(Region::Ca),
        &[],
        STANDARD_PRESETS,
    ),
    make_module(
        "region-au",
        "澳大利亚",
        "澳大利亚节点",
        ModuleKind::Region(Region::Au),
        &[],
        STANDARD_PRESETS,
    ),
    make_module(
        "region-other",
        "其他地区",
        "未识别地区的节点",
        ModuleKind::Region(Region::Other),
        &[],
        STANDARD_PRESETS,
    ),
    make_module(
        "ai",
        "AI 服务",
        "OpenAI、Claude、Gemini 等",
        ModuleKind::Service,
        AI_RULES,
        STANDARD_PRESETS,
    ),
    make_module(
        "google",
        "谷歌服务",
        "Google 搜索与相关服务",
        ModuleKind::Service,
        GOOGLE_RULES,
        STANDARD_PRESETS,
    ),
    make_module(
        "youtube",
        "YouTube",
        "YouTube 视频与音乐",
        ModuleKind::Service,
        YOUTUBE_RULES,
        STANDARD_PRESETS,
    ),
    make_module(
        "telegram",
        "Telegram",
        "Telegram 消息与语音",
        ModuleKind::Service,
        TELEGRAM_RULES,
        STANDARD_PRESETS,
    ),
    make_module(
        "github",
        "GitHub",
        "GitHub 与开发者服务",
        ModuleKind::Service,
        GITHUB_RULES,
        STANDARD_PRESETS,
    ),
    make_module(
        "microsoft",
        "微软服务",
        "Microsoft、Office、OneDrive",
        ModuleKind::Service,
        MICROSOFT_RULES,
        STANDARD_PRESETS,
    ),
    make_module(
        "apple",
        "苹果服务",
        "Apple 与 iCloud",
        ModuleKind::Service,
        APPLE_RULES,
        STANDARD_PRESETS,
    ),
    make_module(
        "netflix",
        "Netflix",
        "Netflix 流媒体",
        ModuleKind::Service,
        NETFLIX_RULES,
        STANDARD_PRESETS,
    ),
    make_module(
        "spotify",
        "Spotify",
        "Spotify 音乐",
        ModuleKind::Service,
        SPOTIFY_RULES,
        STANDARD_PRESETS,
    ),
    make_module(
        "discord",
        "Discord",
        "Discord 语音与社区",
        ModuleKind::Service,
        DISCORD_RULES,
        STANDARD_PRESETS,
    ),
    make_module(
        "x",
        "X (Twitter)",
        "X / Twitter",
        ModuleKind::Service,
        X_RULES,
        STANDARD_PRESETS,
    ),
    make_module(
        "disney",
        "Disney+",
        "Disney+ 流媒体",
        ModuleKind::Service,
        DISNEY_RULES,
        FULL_PRESETS,
    ),
    make_module(
        "tiktok",
        "TikTok",
        "TikTok 短视频",
        ModuleKind::Service,
        TIKTOK_RULES,
        FULL_PRESETS,
    ),
    make_module(
        "meta",
        "Meta",
        "Facebook、Instagram",
        ModuleKind::Service,
        META_RULES,
        FULL_PRESETS,
    ),
    make_module(
        "steam",
        "Steam",
        "Steam 游戏平台",
        ModuleKind::Service,
        STEAM_RULES,
        FULL_PRESETS,
    ),
    make_module(
        "paypal",
        "PayPal",
        "PayPal 支付服务",
        ModuleKind::Service,
        PAYPAL_RULES,
        FULL_PRESETS,
    ),
];

pub fn module(id: &str) -> Option<&'static Module> {
    MODULES.iter().find(|module| module.id == id)
}

pub fn modules_for_preset(preset: Preset) -> Vec<&'static Module> {
    MODULES
        .iter()
        .filter(|module| {
            module.presets.contains(&preset) && !matches!(module.kind, ModuleKind::Region(_))
        })
        .collect()
}

pub fn rule_priority(module: &Module) -> u16 {
    match module.id {
        "ad" => 10,
        "private" => 20,
        "final" => 1000,
        "cn" => 800,
        "global" => 850,
        _ => 400,
    }
}
