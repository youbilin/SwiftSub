use serde_yaml_ng::{Mapping, Value};

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct DnsOptions {
    pub ipv6: bool,
    pub fake_ip_range: String,
    pub domestic: Vec<String>,
    pub foreign: Vec<String>,
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
        }
    }
}

pub fn build_dns(options: &DnsOptions) -> Value {
    let mut dns = Mapping::new();
    dns.insert(Value::String("enable".into()), Value::Bool(true));
    dns.insert(Value::String("ipv6".into()), Value::Bool(options.ipv6));
    dns.insert(
        Value::String("enhanced-mode".into()),
        Value::String("fake-ip".into()),
    );
    dns.insert(
        Value::String("fake-ip-range".into()),
        Value::String(options.fake_ip_range.clone()),
    );
    dns.insert(Value::String("use-hosts".into()), Value::Bool(false));
    dns.insert(
        Value::String("fake-ip-filter".into()),
        string_sequence(&[
            "*.lan",
            "*.local",
            "localhost.ptlogin2.qq.com",
            "time.*.com",
            "time.*.*.com",
            "ntp.*.com",
            "*.ntp.org.cn",
            "*.msftconnecttest.com",
            "*.msftncsi.com",
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
