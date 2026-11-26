use indexmap::IndexMap;
use serde_yaml_ng::{Mapping, Value};
use sha2::{Digest, Sha256};

use crate::error::{Error, Result};

pub type Fields = IndexMap<String, Value>;

#[derive(Clone, Debug, PartialEq)]
pub struct Proxy {
    fields: Fields,
}

impl Proxy {
    pub fn new(
        kind: impl Into<String>,
        name: impl Into<String>,
        server: impl Into<String>,
        port: u16,
    ) -> Self {
        let mut fields = Fields::new();
        fields.insert("name".to_string(), Value::String(name.into()));
        fields.insert("type".to_string(), Value::String(kind.into()));
        fields.insert("server".to_string(), Value::String(server.into()));
        fields.insert(
            "port".to_string(),
            Value::Number(serde_yaml_ng::Number::from(port as u64)),
        );
        Self { fields }
    }

    pub fn from_value(value: Value) -> Result<Self> {
        let Value::Mapping(map) = value else {
            return Err(Error::InvalidLink("proxy entry must be a mapping".into()));
        };

        let mut fields = Fields::new();
        for (key, value) in map {
            if let Value::String(key) = key {
                fields.insert(key, value);
            }
        }

        let kind = fields
            .get("type")
            .and_then(Value::as_str)
            .filter(|value| !value.is_empty())
            .ok_or(Error::MissingField("type"))?
            .to_lowercase();
        fields.insert("type".to_string(), Value::String(kind.clone()));

        if matches!(kind.as_str(), "direct" | "dns" | "reject" | "rematch")
            || kind == "wireguard" && fields.contains_key("peers")
        {
            fields
                .entry("name".into())
                .or_insert_with(|| Value::String(kind.clone()));
            return Ok(Self { fields });
        }

        let server = fields
            .get("server")
            .and_then(Value::as_str)
            .filter(|value| !value.is_empty())
            .ok_or(Error::MissingField("server"))?
            .to_string();

        let port = parse_yaml_port(fields.get("port")).ok_or(Error::MissingField("port"))?;
        fields.insert(
            "port".to_string(),
            Value::Number(serde_yaml_ng::Number::from(port as u64)),
        );

        if !fields.contains_key("name") {
            let default_name = format!("{kind}-{server}:{port}");
            fields.insert("name".to_string(), Value::String(default_name));
        }

        let proxy = Self { fields };
        Ok(proxy)
    }

    pub fn from_yaml_text(input: &str) -> Result<Self> {
        let value: Value = serde_yaml_ng::from_str(input)?;
        Self::from_value(value)
    }

    pub fn to_yaml_text(&self) -> Result<String> {
        Ok(serde_yaml_ng::to_string(&self.to_value())?)
    }

    pub fn fields(&self) -> &Fields {
        &self.fields
    }

    pub fn fields_mut(&mut self) -> &mut Fields {
        &mut self.fields
    }

    pub fn name(&self) -> &str {
        self.get_str("name").unwrap_or_default()
    }

    pub fn kind(&self) -> &str {
        self.get_str("type").unwrap_or_default()
    }

    pub fn server(&self) -> &str {
        self.get_str("server").unwrap_or_default()
    }

    pub fn port(&self) -> u16 {
        self.fields
            .get("port")
            .and_then(|value| match value {
                Value::Number(number) => number.as_u64(),
                Value::String(text) => text.parse::<u64>().ok(),
                _ => None,
            })
            .and_then(|port| u16::try_from(port).ok())
            .unwrap_or_default()
    }

    pub fn uses_server_port(&self) -> bool {
        !matches!(self.kind(), "direct" | "dns" | "reject" | "rematch")
            && !(self.kind() == "wireguard" && self.contains("peers"))
    }

    pub fn get_str(&self, key: &str) -> Option<&str> {
        self.fields.get(key).and_then(Value::as_str)
    }

    pub fn get_bool(&self, key: &str) -> Option<bool> {
        self.fields.get(key).and_then(Value::as_bool)
    }

    pub fn get_u64(&self, key: &str) -> Option<u64> {
        self.fields.get(key).and_then(|value| match value {
            Value::Number(number) => number.as_u64(),
            Value::String(text) => text.parse().ok(),
            _ => None,
        })
    }

    pub fn contains(&self, key: &str) -> bool {
        self.fields.contains_key(key)
    }

    pub fn insert(&mut self, key: impl Into<String>, value: impl Into<Value>) -> &mut Self {
        self.fields.insert(key.into(), value.into());
        self
    }

    pub fn set_name(&mut self, name: impl Into<String>) {
        self.fields
            .insert("name".to_string(), Value::String(name.into()));
    }

    pub fn set_dialer_proxy(&mut self, target: impl Into<String>) {
        self.fields
            .insert("dialer-proxy".to_string(), Value::String(target.into()));
    }

    pub fn remove(&mut self, key: &str) -> Option<Value> {
        self.fields.shift_remove(key)
    }

    pub fn to_value(&self) -> Value {
        let mut map = Mapping::new();
        for key in ["name", "type", "server", "port"] {
            if let Some(value) = self.fields.get(key) {
                map.insert(Value::String(key.to_string()), value.clone());
            }
        }
        for (key, value) in &self.fields {
            if matches!(key.as_str(), "name" | "type" | "server" | "port") {
                continue;
            }
            map.insert(Value::String(key.clone()), value.clone());
        }
        Value::Mapping(map)
    }

    pub fn identity_key(&self) -> String {
        let mut pairs: Vec<(&String, &Value)> = self
            .fields
            .iter()
            .filter(|(key, _)| !matches!(key.as_str(), "name" | "udp" | "tfo" | "dialer-proxy"))
            .collect();
        pairs.sort_by(|left, right| left.0.cmp(right.0));

        let mut hasher = Sha256::new();
        for (key, value) in pairs {
            hasher.update(key.as_bytes());
            hasher.update(b"=");
            hasher.update(render_value(value).as_bytes());
            hasher.update(b"\0");
        }
        hex::encode(&hasher.finalize()[..16])
    }

    pub fn warnings(&self) -> Vec<String> {
        let mut warnings = Vec::new();
        if !self.uses_server_port() {
            if self.name().trim().is_empty() {
                warnings.push("缺少节点名称".into());
            }
            if let Some(issue) = self.fatal_issue() {
                warnings.push(issue);
            }
            return warnings;
        }
        if self.kind().is_empty() {
            warnings.push("缺少代理类型".to_string());
        }
        if self.server().is_empty() {
            warnings.push("缺少服务器地址".to_string());
        }
        if self.port() == 0 {
            warnings.push("缺少有效端口".to_string());
        }
        if self.name().trim().is_empty() {
            warnings.push("缺少节点名称".to_string());
        }
        warnings
    }

    pub fn fatal_issue(&self) -> Option<String> {
        if matches!(self.kind(), "direct" | "dns" | "reject" | "rematch") {
            return None;
        }
        if self.kind() == "wireguard" && self.contains("peers") {
            if self.get_str("private-key").unwrap_or_default().is_empty() {
                return Some("WireGuard 缺少 private-key".into());
            }
            let Some(peers) = self
                .fields
                .get("peers")
                .and_then(Value::as_sequence)
                .filter(|peers| !peers.is_empty())
            else {
                return Some("WireGuard peers 必须是非空节点列表".into());
            };
            for (index, peer) in peers.iter().enumerate() {
                if peer
                    .get("server")
                    .and_then(Value::as_str)
                    .is_none_or(str::is_empty)
                    || parse_yaml_port(peer.get("port")).is_none()
                    || peer
                        .get("public-key")
                        .and_then(Value::as_str)
                        .is_none_or(str::is_empty)
                {
                    return Some(format!(
                        "WireGuard 第 {} 个 peer 缺少服务器、有效端口或公钥",
                        index + 1
                    ));
                }
            }
            return None;
        }
        if self.kind().is_empty() {
            return Some("缺少代理类型".to_string());
        }
        if self.server().is_empty() {
            return Some("缺少服务器地址".to_string());
        }
        if self.port() == 0 {
            return Some("缺少有效端口".to_string());
        }

        match self.kind() {
            "vless" => {
                if self.get_str("uuid").unwrap_or_default().is_empty() {
                    return Some("VLESS 缺少 uuid".to_string());
                }
                if let Some(Value::Mapping(reality)) = self.fields.get("reality-opts") {
                    let public_key = mapping_get(reality, "public-key")
                        .and_then(Value::as_str)
                        .unwrap_or_default();
                    if !is_valid_reality_public_key(public_key) {
                        return Some("REALITY 公钥必须是 32 字节 base64url".to_string());
                    }
                    if let Some(short_id) = mapping_get(reality, "short-id").and_then(Value::as_str)
                        && !is_valid_reality_short_id(short_id)
                    {
                        return Some("REALITY short-id 必须是 0-16 位十六进制".to_string());
                    }
                }
            }
            "vmess" => {
                if self.get_str("uuid").unwrap_or_default().is_empty() {
                    return Some("VMess 缺少 uuid".to_string());
                }
            }
            "trojan" | "hysteria2" | "anytls" => {
                if self.get_str("password").unwrap_or_default().is_empty() {
                    return Some(format!("{} 缺少 password", self.kind().to_uppercase()));
                }
            }
            "snell" if self.get_str("psk").unwrap_or_default().is_empty() => {
                return Some("Snell 缺少 psk".into());
            }
            "ssh"
                if self.get_str("username").unwrap_or_default().is_empty()
                    || (self.get_str("password").unwrap_or_default().is_empty()
                        && self.get_str("private-key").unwrap_or_default().is_empty()) =>
            {
                return Some("SSH 缺少 username 或认证字段".into());
            }
            "wireguard"
                if self.get_str("private-key").unwrap_or_default().is_empty()
                    || self.get_str("public-key").unwrap_or_default().is_empty() =>
            {
                return Some("WireGuard 缺少密钥".into());
            }
            "ssr"
                if ["cipher", "password", "protocol", "obfs"]
                    .iter()
                    .any(|k| self.get_str(k).unwrap_or_default().is_empty()) =>
            {
                return Some("SSR 缺少必填字段".into());
            }
            "tuic" => {
                if self.get_str("uuid").unwrap_or_default().is_empty()
                    && self.get_str("token").unwrap_or_default().is_empty()
                {
                    return Some("TUIC 缺少 uuid 或 token".to_string());
                }
            }
            "ss" if (self.get_str("cipher").unwrap_or_default().is_empty()
                || self.get_str("password").unwrap_or_default().is_empty()) =>
            {
                return Some("Shadowsocks 缺少 cipher 或 password".to_string());
            }
            _ => {}
        }

        None
    }
}

fn mapping_get<'a>(mapping: &'a Mapping, key: &str) -> Option<&'a Value> {
    mapping
        .iter()
        .find_map(|(candidate, value)| (candidate.as_str() == Some(key)).then_some(value))
}

pub fn is_valid_reality_public_key(value: &str) -> bool {
    use base64::Engine;

    let value = value.trim().trim_end_matches('=');
    if value.is_empty() {
        return false;
    }

    for engine in [
        &base64::engine::general_purpose::URL_SAFE_NO_PAD,
        &base64::engine::general_purpose::STANDARD_NO_PAD,
    ] {
        if let Ok(bytes) = engine.decode(value)
            && bytes.len() == 32
        {
            return true;
        }
    }
    false
}

pub fn is_valid_reality_short_id(value: &str) -> bool {
    value.len() <= 16 && value.chars().all(|character| character.is_ascii_hexdigit())
}

fn parse_yaml_port(value: Option<&Value>) -> Option<u16> {
    let port = match value? {
        Value::Number(number) => number.as_u64()?,
        Value::String(text) => text.trim().parse::<u64>().ok()?,
        _ => return None,
    };
    u16::try_from(port).ok().filter(|port| *port > 0)
}

fn render_value(value: &Value) -> String {
    match value {
        Value::Null => "null".to_string(),
        Value::Bool(value) => value.to_string(),
        Value::Number(value) => value.to_string(),
        Value::String(value) => value.clone(),
        Value::Sequence(values) => {
            let values: Vec<String> = values.iter().map(render_value).collect();
            format!("[{}]", values.join(","))
        }
        Value::Mapping(mapping) => {
            let mut entries: Vec<(&Value, &Value)> = mapping.iter().collect();
            entries.sort_by_key(|(key, _)| render_value(key));
            let entries: Vec<String> = entries
                .into_iter()
                .map(|(key, value)| format!("{}:{}", render_value(key), render_value(value)))
                .collect();
            format!("{{{}}}", entries.join(","))
        }
        Value::Tagged(tagged) => render_value(&tagged.value),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn invalid_ports_cannot_wrap_and_local_and_peer_proxies_roundtrip() {
        for value in [Value::from(65537), Value::from("65537")] {
            let mut proxy = Proxy::new("http", "test", "example.org", 1);
            proxy.insert("port", value);
            assert_eq!(proxy.port(), 0);
            assert!(proxy.fatal_issue().is_some());
            assert!(Proxy::from_value(proxy.to_value()).is_err());
        }
        for kind in ["direct", "dns", "reject", "rematch"] {
            let value: Value = serde_yaml_ng::from_str(&format!(
                "name: named-{kind}\ntype: {kind}\ninterface-name: Ethernet\n"
            ))
            .unwrap();
            let proxy = Proxy::from_value(value.clone()).unwrap();
            assert!(!proxy.uses_server_port());
            assert!(proxy.fatal_issue().is_none());
            assert_eq!(proxy.to_value(), value);
        }
        let value: Value = serde_yaml_ng::from_str("name: multi-wg\ntype: wireguard\nprivate-key: private\nip: 172.16.0.2\npeers:\n  - {server: example.org, port: 2480, public-key: public, allowed-ips: ['0.0.0.0/0']}\n").unwrap();
        let proxy = Proxy::from_value(value.clone()).unwrap();
        assert!(proxy.fatal_issue().is_none());
        assert_eq!(proxy.to_value(), value);
        let mut invalid = value;
        invalid["peers"][0]["port"] = Value::from(65537);
        assert!(
            Proxy::from_value(invalid)
                .unwrap()
                .fatal_issue()
                .unwrap()
                .contains("peer")
        );
    }

    #[test]
    fn validates_reality_keys() {
        assert!(is_valid_reality_public_key(
            "jNXHt1yRo0vDuchQlIP6Z0ZvjT3KtzVI-T4E7RoLJS0"
        ));
        assert!(!is_valid_reality_public_key("abc"));
        assert!(is_valid_reality_short_id(""));
        assert!(is_valid_reality_short_id("abcd0123"));
        assert!(!is_valid_reality_short_id("not-hex"));
    }

    #[test]
    fn identity_ignores_nested_key_order_and_name() {
        let first: Value = serde_yaml_ng::from_str(
            "name: A\ntype: vmess\nserver: example.com\nport: 443\nuuid: u\nws-opts:\n  path: /ws\n  headers:\n    Host: a.com\n    X-Test: yes\n",
        )
        .unwrap();
        let second: Value = serde_yaml_ng::from_str(
            "name: B\ntype: vmess\nserver: example.com\nport: 443\nuuid: u\nws-opts:\n  headers:\n    X-Test: yes\n    Host: a.com\n  path: /ws\n",
        )
        .unwrap();

        let first = Proxy::from_value(first).unwrap();
        let second = Proxy::from_value(second).unwrap();
        assert_eq!(first.identity_key(), second.identity_key());
    }
}
