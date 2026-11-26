use serde_yaml_ng::{Mapping, Value};

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct DnsOptions {
    pub ipv6: bool,
    pub fake_ip_range: String,
    pub domestic: Vec<String>,
    pub foreign: Vec<String>,
    /// Complete Mihomo DNS object. Older recipes keep using the four legacy fields.
    pub config: Option<Mapping>,
    pub hosts: Mapping,
}

impl Default for DnsOptions {
    fn default() -> Self {
        Self {
            ipv6: false,
            fake_ip_range: "198.18.0.1/16".to_string(),
            domestic: vec![
                "https://dns.alidns.com/dns-query".to_string(),
                "https://doh.pub/dns-query".to_string(),
            ],
            foreign: vec![
                "https://cloudflare-dns.com/dns-query".to_string(),
                "https://dns.google/dns-query".to_string(),
            ],
            config: None,
            hosts: Mapping::new(),
        }
    }
}

impl DnsOptions {
    pub fn from_yaml(yaml: &str) -> crate::Result<Self> {
        let value: Value = serde_yaml_ng::from_str(yaml)?;
        let root = value
            .as_mapping()
            .ok_or_else(|| crate::Error::InvalidConfig("DNS 配置必须是 YAML 对象".into()))?;
        let wrapped = root.contains_key("dns");
        let dns = if wrapped {
            root.get("dns")
                .and_then(Value::as_mapping)
                .ok_or_else(|| crate::Error::InvalidConfig("dns 必须是 YAML 对象".into()))?
        } else {
            root
        };
        if wrapped
            && root
                .keys()
                .any(|key| !matches!(key.as_str(), Some("dns" | "hosts")))
        {
            return Err(crate::Error::InvalidConfig(
                "这里只填写 dns 和 hosts；其他配置请在配置页编辑".into(),
            ));
        }
        let hosts = if wrapped {
            match root.get("hosts") {
                None => Mapping::new(),
                Some(Value::Mapping(hosts)) => hosts.clone(),
                _ => return Err(crate::Error::InvalidConfig("hosts 必须是 YAML 对象".into())),
            }
        } else {
            Mapping::new()
        };
        let mut options = Self {
            config: Some(dns.clone()),
            hosts,
            ..Self::default()
        };
        options.ipv6 = dns.get("ipv6").and_then(Value::as_bool).unwrap_or(false);
        if let Some(range) = dns.get("fake-ip-range").and_then(Value::as_str) {
            options.fake_ip_range = range.into();
        }
        options.validate()?;
        Ok(options)
    }

    pub fn to_value(&self) -> Value {
        let mut root = Mapping::new();
        root.insert("dns".into(), build_dns(self));
        root.insert("hosts".into(), Value::Mapping(self.hosts.clone()));
        Value::Mapping(root)
    }

    pub fn to_yaml(&self) -> crate::Result<String> {
        Ok(serde_yaml_ng::to_string(&self.to_value())?)
    }

    pub fn validate(&self) -> crate::Result<()> {
        if let Some(dns) = &self.config {
            return validate_dns(dns, &self.hosts);
        }
        let valid_range = self
            .fake_ip_range
            .split_once('/')
            .is_some_and(|(ip, prefix)| {
                ip.parse::<std::net::Ipv4Addr>().is_ok()
                    && prefix.parse::<u8>().is_ok_and(|bits| bits <= 32)
            });
        if !valid_range {
            return Err(crate::Error::InvalidConfig(
                "Fake IP 地址段须为有效的 IPv4 CIDR，例如 198.18.0.1/16".into(),
            ));
        }
        for (name, entries) in [("国内 DNS", &self.domestic), ("国外 DNS", &self.foreign)] {
            if entries.is_empty()
                || entries
                    .iter()
                    .any(|entry| entry.trim().is_empty() || entry.contains(['\n', '\r', ' ', '\t']))
            {
                return Err(crate::Error::InvalidConfig(format!(
                    "{name} 需要非空 DNS 地址列表，单个地址不能含空格或换行"
                )));
            }
        }
        Ok(())
    }
}

pub fn build_dns(options: &DnsOptions) -> Value {
    if let Some(config) = &options.config {
        return Value::Mapping(config.clone());
    }
    let mut dns = Mapping::new();
    dns.insert(Value::String("enable".into()), Value::Bool(true));
    dns.insert("listen".into(), "127.0.0.1:1053".into());
    dns.insert(Value::String("ipv6".into()), Value::Bool(options.ipv6));
    dns.insert(
        Value::String("enhanced-mode".into()),
        Value::String("fake-ip".into()),
    );
    dns.insert(
        Value::String("fake-ip-range".into()),
        Value::String(options.fake_ip_range.clone()),
    );
    dns.insert("fake-ip-range6".into(), "fdfe:dcba:9876::1/64".into());
    dns.insert("fake-ip-filter-mode".into(), "blacklist".into());
    for (key, value) in [
        ("prefer-h3", false),
        ("respect-rules", false),
        ("use-hosts", true),
        ("use-system-hosts", false),
        ("direct-nameserver-follow-policy", false),
    ] {
        dns.insert(key.into(), Value::Bool(value));
    }
    dns.insert("direct-nameserver".into(), Value::Sequence(Vec::new()));
    dns.insert("fallback".into(), Value::Sequence(Vec::new()));
    dns.insert(
        Value::String("fake-ip-filter".into()),
        string_sequence(&[
            "*.lan",
            "*.local",
            "*.arpa",
            "localhost.ptlogin2.qq.com",
            "time.*.com",
            "time.*.*.com",
            "ntp.*.com",
            "*.ntp.org.cn",
            "*.msftconnecttest.com",
            "*.msftncsi.com",
            "+.pool.ntp.org",
            "+.market.xiaomi.com",
            "geosite:private",
            "geosite:connectivity-check",
            "stun.*.*",
            "stun.*.*.*",
        ]),
    );
    dns.insert(
        Value::String("default-nameserver".into()),
        string_sequence(&["223.5.5.5", "119.29.29.29"]),
    );

    let mut nameservers = options.domestic.clone();
    nameservers.extend(options.foreign.iter().cloned());
    dns.insert(
        Value::String("nameserver".into()),
        string_sequence(&nameservers.iter().map(String::as_str).collect::<Vec<_>>()),
    );
    dns.insert(
        Value::String("proxy-server-nameserver".into()),
        string_sequence(
            &options
                .domestic
                .iter()
                .map(String::as_str)
                .collect::<Vec<_>>(),
        ),
    );

    let mut policy = Mapping::new();
    policy.insert(
        Value::String("geosite:cn,private".into()),
        string_sequence(
            &options
                .domestic
                .iter()
                .map(String::as_str)
                .collect::<Vec<_>>(),
        ),
    );
    policy.insert(
        Value::String("geosite:geolocation-!cn".into()),
        string_sequence(
            &options
                .foreign
                .iter()
                .map(String::as_str)
                .collect::<Vec<_>>(),
        ),
    );
    dns.insert("nameserver-policy".into(), Value::Mapping(policy));

    Value::Mapping(dns)
}

fn string_sequence(values: &[&str]) -> Value {
    Value::Sequence(
        values
            .iter()
            .map(|value| Value::String((*value).to_string()))
            .collect(),
    )
}

fn validate_dns(dns: &Mapping, hosts: &Mapping) -> crate::Result<()> {
    let invalid = |message: String| crate::Error::InvalidConfig(message);
    for key in [
        "enable",
        "ipv6",
        "prefer-h3",
        "respect-rules",
        "use-hosts",
        "use-system-hosts",
        "direct-nameserver-follow-policy",
        "fallback-lazy-query",
    ] {
        if dns.get(key).is_some_and(|value| !value.is_bool()) {
            return Err(invalid(format!("DNS {key} 须为 true 或 false")));
        }
    }
    if let Some(value) = dns.get("listen") {
        let listen = value
            .as_str()
            .ok_or_else(|| invalid("DNS listen 须为监听地址".into()))?;
        if !listen.is_empty()
            && !listen
                .parse::<std::net::SocketAddr>()
                .is_ok_and(|addr| addr.port() > 0)
        {
            return Err(invalid(
                "DNS listen 须为 IP:端口，例如 127.0.0.1:1053；IPv6 地址请加方括号".into(),
            ));
        }
    }
    for (key, ipv6) in [("fake-ip-range", false), ("fake-ip-range6", true)] {
        if let Some(range) = dns.get(key) {
            let valid = range
                .as_str()
                .and_then(|range| range.split_once('/'))
                .is_some_and(|(ip, prefix)| {
                    prefix
                        .parse::<u8>()
                        .is_ok_and(|bits| bits <= if ipv6 { 128 } else { 32 })
                        && if ipv6 {
                            ip.parse::<std::net::Ipv6Addr>().is_ok()
                        } else {
                            ip.parse::<std::net::Ipv4Addr>().is_ok()
                        }
                });
            if !valid {
                return Err(invalid(format!(
                    "DNS {key} 须为有效的 {} CIDR 地址段",
                    if ipv6 { "IPv6" } else { "IPv4" }
                )));
            }
        }
    }
    for (key, values) in [
        ("enhanced-mode", &["fake-ip", "redir-host"][..]),
        (
            "fake-ip-filter-mode",
            &["blacklist", "whitelist", "rule"][..],
        ),
        ("cache-algorithm", &["lru", "arc"][..]),
    ] {
        if dns
            .get(key)
            .is_some_and(|value| !value.as_str().is_some_and(|s| values.contains(&s)))
        {
            return Err(invalid(format!("DNS {key} 可选 {}", values.join(" / "))));
        }
    }
    for key in [
        "default-nameserver",
        "proxy-server-nameserver",
        "direct-nameserver",
        "nameserver",
        "fallback",
        "fake-ip-filter",
    ] {
        if let Some(value) = dns.get(key) {
            let entries = value
                .as_sequence()
                .ok_or_else(|| invalid(format!("DNS {key} 须为列表")))?;
            if entries
                .iter()
                .any(|entry| !entry.as_str().is_some_and(|s| !s.trim().is_empty()))
            {
                return Err(invalid(format!("DNS {key} 的每项须为非空字符串")));
            }
            if key != "fake-ip-filter"
                && entries
                    .iter()
                    .any(|entry| entry.as_str().unwrap().contains([' ', '\t', '\n', '\r']))
            {
                return Err(invalid(format!("DNS {key} 的地址不能包含空白字符")));
            }
            if key == "nameserver"
                && entries.is_empty()
                && dns.get("enable").and_then(Value::as_bool) != Some(false)
            {
                return Err(invalid("启用 DNS 时 nameserver 不能为空".into()));
            }
        }
    }
    for key in ["nameserver-policy", "proxy-server-nameserver-policy"] {
        if let Some(value) = dns.get(key) {
            let policy = value
                .as_mapping()
                .ok_or_else(|| invalid(format!("DNS {key} 须为对象")))?;
            for (name, value) in policy {
                let valid_value = value.as_str().is_some_and(|s| !s.trim().is_empty())
                    || value.as_sequence().is_some_and(|list| {
                        !list.is_empty()
                            && list
                                .iter()
                                .all(|value| value.as_str().is_some_and(|s| !s.trim().is_empty()))
                    });
                if !name.as_str().is_some_and(|s| !s.trim().is_empty()) || !valid_value {
                    return Err(invalid(format!(
                        "DNS {key} 须使用域名或规则作为键、DNS 地址或地址列表作为值"
                    )));
                }
            }
        }
    }
    if dns
        .get("fallback-filter")
        .is_some_and(|value| !value.is_mapping())
    {
        return Err(invalid("DNS fallback-filter 须为对象".into()));
    }
    if dns.get("respect-rules").and_then(Value::as_bool) == Some(true)
        && !dns
            .get("proxy-server-nameserver")
            .and_then(Value::as_sequence)
            .is_some_and(|values| !values.is_empty())
    {
        return Err(invalid(
            "启用 respect-rules 时，请填写 proxy-server-nameserver，避免节点域名解析循环".into(),
        ));
    }
    if hosts
        .keys()
        .any(|key| !key.as_str().is_some_and(|key| !key.trim().is_empty()))
    {
        return Err(invalid("hosts 的键须为非空域名".into()));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn complete_dns_round_trips_with_hosts_and_extensions() {
        let yaml = include_str!("../../tests/fixtures/full-dns.yaml");
        let options = DnsOptions::from_yaml(yaml).unwrap();
        assert_eq!(
            options.to_value(),
            serde_yaml_ng::from_str::<Value>(yaml).unwrap()
        );
        let json = serde_json::to_string(&options).unwrap();
        let restored: DnsOptions = serde_json::from_str(&json).unwrap();
        assert_eq!(restored.to_value(), options.to_value());
        let legacy: DnsOptions = serde_json::from_str(r#"{"ipv6":true,"fake_ip_range":"198.18.0.1/16","domestic":["223.5.5.5"],"foreign":["1.1.1.1"]}"#).unwrap();
        assert_eq!(
            build_dns(&legacy)["nameserver"],
            string_sequence(&["223.5.5.5", "1.1.1.1"])
        );
    }

    #[test]
    fn invalid_dns_never_becomes_publishable() {
        for yaml in [
            "dns: {enable: yes}",
            "dns: {listen: '127.0.0.1:0'}",
            "dns: {fake-ip-range6: '198.18.0.1/64'}",
            "dns: {fake-ip-filter-mode: wrong}",
            "dns: {respect-rules: true, proxy-server-nameserver: []}",
            "dns: {nameserver-policy: {'+.example.com': []}}",
            "dns: {nameserver: []}",
            "dns: {}\nhosts: []",
        ] {
            assert!(DnsOptions::from_yaml(yaml).is_err(), "{yaml}");
        }
    }
}
