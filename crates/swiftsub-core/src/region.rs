#[derive(
    Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, serde::Serialize, serde::Deserialize,
)]
#[serde(rename_all = "lowercase")]
pub enum Region {
    Hk,
    Tw,
    Jp,
    Kr,
    Sg,
    Us,
    Uk,
    De,
    Fr,
    Nl,
    Ca,
    Au,
    Ru,
    In,
    Tr,
    Br,
    Ar,
    Other,
}

impl Region {
    pub const ALL: [Region; 18] = [
        Region::Hk,
        Region::Tw,
        Region::Jp,
        Region::Kr,
        Region::Sg,
        Region::Us,
        Region::Uk,
        Region::De,
        Region::Fr,
        Region::Nl,
        Region::Ca,
        Region::Au,
        Region::Ru,
        Region::In,
        Region::Tr,
        Region::Br,
        Region::Ar,
        Region::Other,
    ];

    pub fn code(self) -> &'static str {
        match self {
            Region::Hk => "hk",
            Region::Tw => "tw",
            Region::Jp => "jp",
            Region::Kr => "kr",
            Region::Sg => "sg",
            Region::Us => "us",
            Region::Uk => "uk",
            Region::De => "de",
            Region::Fr => "fr",
            Region::Nl => "nl",
            Region::Ca => "ca",
            Region::Au => "au",
            Region::Ru => "ru",
            Region::In => "in",
            Region::Tr => "tr",
            Region::Br => "br",
            Region::Ar => "ar",
            Region::Other => "other",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Region::Hk => "香港",
            Region::Tw => "台湾",
            Region::Jp => "日本",
            Region::Kr => "韩国",
            Region::Sg => "新加坡",
            Region::Us => "美国",
            Region::Uk => "英国",
            Region::De => "德国",
            Region::Fr => "法国",
            Region::Nl => "荷兰",
            Region::Ca => "加拿大",
            Region::Au => "澳大利亚",
            Region::Ru => "俄罗斯",
            Region::In => "印度",
            Region::Tr => "土耳其",
            Region::Br => "巴西",
            Region::Ar => "阿根廷",
            Region::Other => "其他地区",
        }
    }
}

struct RegionHints {
    region: Region,
    words: &'static [&'static str],
    tokens: &'static [&'static str],
}

const HINTS: &[RegionHints] = &[
    RegionHints {
        region: Region::Hk,
        words: &["香港", "深港", "沪港", "港区", "hong kong", "hongkong"],
        tokens: &["hk", "hkg", "hkbn", "pccw"],
    },
    RegionHints {
        region: Region::Tw,
        words: &["台湾", "台灣", "台北", "新北", "彰化", "taiwan", "taipei"],
        tokens: &["tw", "tpe", "hinet"],
    },
    RegionHints {
        region: Region::Jp,
        words: &[
            "日本", "东京", "東京", "大阪", "埼玉", "沪日", "japan", "tokyo", "osaka",
        ],
        tokens: &["jp", "nrt", "hnd", "kix", "tyo"],
    },
    RegionHints {
        region: Region::Kr,
        words: &["韩国", "韓國", "首尔", "首爾", "korea", "seoul"],
        tokens: &["kr", "icn", "kix"],
    },
    RegionHints {
        region: Region::Sg,
        words: &["新加坡", "狮城", "獅城", "singapore", "singtel"],
        tokens: &["sg", "sin"],
    },
    RegionHints {
        region: Region::Us,
        words: &[
            "美国",
            "美國",
            "洛杉矶",
            "圣何塞",
            "硅谷",
            "西雅图",
            "纽约",
            "达拉斯",
            "芝加哥",
            "凤凰城",
            "united states",
            "los angeles",
            "san jose",
            "seattle",
            "new york",
            "dallas",
            "chicago",
            "phoenix",
            "ashburn",
        ],
        tokens: &[
            "us", "usa", "lax", "sjc", "sfo", "sea", "jfk", "ewr", "iad", "dfw", "ord",
        ],
    },
    RegionHints {
        region: Region::Uk,
        words: &[
            "英国",
            "英國",
            "伦敦",
            "倫敦",
            "britain",
            "united kingdom",
            "london",
        ],
        tokens: &["uk", "gb", "lhr"],
    },
    RegionHints {
        region: Region::De,
        words: &["德国", "德國", "法兰克福", "germany", "frankfurt"],
        tokens: &["de", "fra"],
    },
    RegionHints {
        region: Region::Fr,
        words: &["法国", "法國", "巴黎", "france", "paris"],
        tokens: &["fr", "cdg"],
    },
    RegionHints {
        region: Region::Nl,
        words: &["荷兰", "荷蘭", "阿姆斯特丹", "netherlands", "amsterdam"],
        tokens: &["nl", "ams"],
    },
    RegionHints {
        region: Region::Ca,
        words: &[
            "加拿大",
            "多伦多",
            "温哥华",
            "canada",
            "toronto",
            "vancouver",
        ],
        tokens: &["ca", "yyz", "yvr"],
    },
    RegionHints {
        region: Region::Au,
        words: &[
            "澳大利亚",
            "澳洲",
            "悉尼",
            "墨尔本",
            "australia",
            "sydney",
            "melbourne",
        ],
        tokens: &["au", "syd", "mel"],
    },
    RegionHints {
        region: Region::Ru,
        words: &["俄罗斯", "俄羅斯", "莫斯科", "russia", "moscow"],
        tokens: &["ru"],
    },
    RegionHints {
        region: Region::In,
        words: &["印度", "孟买", "india", "mumbai"],
        tokens: &["in", "bom"],
    },
    RegionHints {
        region: Region::Tr,
        words: &["土耳其", "伊斯坦布尔", "turkey", "istanbul"],
        tokens: &["tr"],
    },
    RegionHints {
        region: Region::Br,
        words: &["巴西", "圣保罗", "brazil", "sao paulo"],
        tokens: &["br"],
    },
    RegionHints {
        region: Region::Ar,
        words: &["阿根廷", "布宜诺斯艾利斯", "argentina"],
        tokens: &["ar"],
    },
];

pub fn detect_region(name: &str) -> Region {
    let lower = name.to_lowercase();

    for hint in HINTS {
        if hint.words.iter().any(|word| lower.contains(word)) {
            return hint.region;
        }
    }

    let tokens: Vec<&str> = lower
        .split(|c: char| !c.is_ascii_alphanumeric())
        .filter(|token| !token.is_empty())
        .collect();

    for hint in HINTS {
        if hint
            .tokens
            .iter()
            .any(|code| tokens.iter().any(|token| token == code))
        {
            return hint.region;
        }
    }

    Region::Other
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_chinese_names() {
        assert_eq!(detect_region("香港 01 | 1x"), Region::Hk);
        assert_eq!(detect_region("日本东京 IPLC"), Region::Jp);
        assert_eq!(detect_region("美国 洛杉矶 02"), Region::Us);
        assert_eq!(detect_region("新加坡-狮城"), Region::Sg);
    }

    #[test]
    fn detects_codes_and_airports() {
        assert_eq!(detect_region("US-LAX-01"), Region::Us);
        assert_eq!(detect_region("jp_tokyo_01"), Region::Jp);
        assert_eq!(detect_region("HKG 01"), Region::Hk);
        assert_eq!(detect_region("ss-德国-01"), Region::De);
    }

    #[test]
    fn avoids_false_positives() {
        assert_eq!(detect_region("premium-node"), Region::Other);
        assert_eq!(detect_region("direct"), Region::Other);
    }
}
