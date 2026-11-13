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
            .unwrap_or_default() as u16
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
            "trojan" | "hysteria2" => {
                if self.get_str("password").unwrap_or_default().is_empty() {
                    return Some(format!("{} 缺少 password", self.kind().to_uppercase()));
                }
            }
            "tuic" => {
                if self.get_str("uuid").unwrap_or_default().is_empty() {
                    return Some("TUIC 缺少 uuid".to_string());
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
