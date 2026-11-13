use serde_yaml_ng::{Mapping, Value};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum GroupKind {
    #[default]
    Select,
    UrlTest,
    Fallback,
    LoadBalance,
    Relay,
}

impl GroupKind {
    pub fn as_str(self) -> &'static str {
        match self {
            GroupKind::Select => "select",
            GroupKind::UrlTest => "url-test",
            GroupKind::Fallback => "fallback",
            GroupKind::LoadBalance => "load-balance",
            GroupKind::Relay => "relay",
        }
    }
}

#[derive(Clone, Debug)]
pub struct ProxyGroup {
    pub name: String,
    pub kind: GroupKind,
    pub proxies: Vec<String>,
    pub url: Option<String>,
    pub interval: Option<u32>,
    pub lazy: bool,
}

impl ProxyGroup {
    pub fn new(name: impl Into<String>, kind: GroupKind, proxies: Vec<String>) -> Self {
        Self {
            name: name.into(),
            kind,
            proxies,
            url: None,
            interval: None,
            lazy: false,
        }
    }

    pub fn with_health_check(mut self, url: impl Into<String>, interval: u32) -> Self {
        self.url = Some(url.into());
        self.interval = Some(interval);
        self
    }

    pub fn to_value(&self) -> Value {
        let mut map = Mapping::new();
        map.insert(
            Value::String("name".to_string()),
            Value::String(self.name.clone()),
        );
        map.insert(
            Value::String("type".to_string()),
            Value::String(self.kind.as_str().to_string()),
        );
        map.insert(
            Value::String("proxies".to_string()),
            Value::Sequence(
                self.proxies
                    .iter()
                    .map(|proxy| Value::String(proxy.clone()))
                    .collect(),
            ),
        );
        if let Some(url) = &self.url {
            map.insert(Value::String("url".to_string()), Value::String(url.clone()));
        }
        if let Some(interval) = self.interval {
            map.insert(
                Value::String("interval".to_string()),
                Value::Number(serde_yaml_ng::Number::from(interval as u64)),
            );
        }
        if self.lazy {
            map.insert(Value::String("lazy".to_string()), Value::Bool(true));
        }
        Value::Mapping(map)
    }
}
