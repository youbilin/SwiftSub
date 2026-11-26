use super::*;
use serde_yaml_ng::{Mapping, Value};
use swiftsub_core::generator::DnsOptions;

pub const DNS_FLAGS: [(&str, &str); 8] = [
    ("enable", "启用 DNS"),
    ("ipv6", "IPv6"),
    ("prefer-h3", "优先 HTTP/3"),
    ("respect-rules", "DNS 连接遵循分流规则"),
    ("use-hosts", "使用 Hosts"),
    ("use-system-hosts", "使用系统 Hosts"),
    ("direct-nameserver-follow-policy", "直连 DNS 遵循解析策略"),
    ("fallback-lazy-query", "延迟查询备用 DNS"),
];
pub const DNS_FIELDS: [(&str, &str); 3] = [
    ("listen", "监听地址"),
    ("fake-ip-range", "Fake IP · IPv4"),
    ("fake-ip-range6", "Fake IP · IPv6"),
];
pub const DNS_LISTS: [(&str, &str); 6] = [
    ("nameserver", "解析 DNS"),
    ("default-nameserver", "默认 DNS"),
    ("proxy-server-nameserver", "节点域名 DNS"),
    ("direct-nameserver", "直连 DNS"),
    ("fallback", "备用 DNS"),
    ("fake-ip-filter", "Fake IP 过滤"),
];
pub const DNS_MAPS: [(&str, &str); 4] = [
    ("nameserver-policy", "域名解析策略"),
    ("proxy-server-nameserver-policy", "节点域名解析策略"),
    ("fallback-filter", "备用 DNS 筛选"),
    ("hosts", "Hosts"),
];

pub struct DnsEditor {
    pub yaml_mode: bool,
    pub yaml: Entity<TextareaState>,
    pub fields: HashMap<&'static str, Entity<InputState>>,
    pub blocks: HashMap<&'static str, Entity<TextareaState>>,
    pub flags: HashMap<&'static str, bool>,
    pub mode: String,
    pub filter_mode: String,
    baseline: Value,
}

impl DnsEditor {
    pub fn new(window: &mut Window, cx: &mut Context<SwiftSubApp>) -> Self {
        Self {
            yaml_mode: false,
            yaml: cx.new(|cx| TextareaState::new(window, cx).searchable(true)),
            fields: DNS_FIELDS
                .into_iter()
                .map(|(key, label)| {
                    (
                        key,
                        cx.new(|cx| InputState::new(window, cx).placeholder(label)),
                    )
                })
                .collect(),
            blocks: DNS_LISTS
                .into_iter()
                .chain(DNS_MAPS)
                .map(|(key, _)| (key, cx.new(|cx| TextareaState::new(window, cx))))
                .collect(),
            flags: HashMap::new(),
            mode: "fake-ip".into(),
            filter_mode: "blacklist".into(),
            baseline: DnsOptions::default().to_value(),
        }
    }

    pub fn load(
        &mut self,
        options: &DnsOptions,
        window: &mut Window,
        cx: &mut Context<SwiftSubApp>,
    ) {
        self.baseline = options.to_value();
        let dns = &self.baseline["dns"];
        self.flags = DNS_FLAGS
            .into_iter()
            .map(|(key, _)| (key, dns.get(key).and_then(Value::as_bool).unwrap_or(false)))
            .collect();
        self.mode = dns
            .get("enhanced-mode")
            .and_then(Value::as_str)
            .unwrap_or("redir-host")
            .into();
        self.filter_mode = dns
            .get("fake-ip-filter-mode")
            .and_then(Value::as_str)
            .unwrap_or("blacklist")
            .into();
        for (key, input) in &self.fields {
            let value = dns
                .get(*key)
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string();
            input.update(cx, |input, cx| input.set_value(value, window, cx));
        }
        for (key, _) in DNS_LISTS {
            let text = dns
                .get(key)
                .and_then(Value::as_sequence)
                .map(|entries| {
                    entries
                        .iter()
                        .filter_map(Value::as_str)
                        .collect::<Vec<_>>()
                        .join("\n")
                })
                .unwrap_or_default();
            self.blocks[key].update(cx, |input, cx| input.set_value(text, window, cx));
        }
        for (key, _) in DNS_MAPS {
            let value = if key == "hosts" {
                self.baseline.get(key)
            } else {
                dns.get(key)
            };
            let text = value
                .map(serde_yaml_ng::to_string)
                .transpose()
                .ok()
                .flatten()
                .unwrap_or_else(|| "{}\n".into());
            self.blocks[key].update(cx, |input, cx| input.set_value(text, window, cx));
        }
        self.yaml.update(cx, |input, cx| {
            input.set_value(options.to_yaml().unwrap_or_default(), window, cx)
        });
    }

    fn form_value(&self, cx: &Context<SwiftSubApp>) -> Result<Value, String> {
        let mut value = self.baseline.clone();
        let dns = value
            .get_mut("dns")
            .and_then(Value::as_mapping_mut)
            .ok_or("DNS 必须是对象")?;
        for (key, flag) in &self.flags {
            // Leave optional fields absent unless the user enables them.
            if dns.contains_key(*key) || *flag || *key != "fallback-lazy-query" {
                dns.insert((*key).into(), Value::Bool(*flag));
            }
        }
        for (key, input) in &self.fields {
            let text = input.read(cx).value().trim().to_string();
            if text.is_empty() {
                dns.remove(*key);
            } else {
                dns.insert((*key).into(), text.into());
            }
        }
        dns.insert("enhanced-mode".into(), self.mode.clone().into());
        dns.insert(
            "fake-ip-filter-mode".into(),
            self.filter_mode.clone().into(),
        );
        for (key, _) in DNS_LISTS {
            let entries = self.blocks[key]
                .read(cx)
                .value()
                .lines()
                .map(str::trim)
                .filter(|line| !line.is_empty())
                .map(|line| Value::String(line.into()))
                .collect();
            dns.insert(key.into(), Value::Sequence(entries));
        }
        let mut hosts = Mapping::new();
        for (key, _) in DNS_MAPS {
            let text = self.blocks[key].read(cx).value();
            let parsed: Value = if text.trim().is_empty() {
                Value::Mapping(Mapping::new())
            } else {
                serde_yaml_ng::from_str(&text)
                    .map_err(|error| format!("{key} YAML 格式错误：{error}"))?
            };
            let map = parsed
                .as_mapping()
                .ok_or_else(|| format!("{key} 必须是 YAML 对象"))?;
            if key == "hosts" {
                hosts = map.clone();
            } else if dns.contains_key(key) || !map.is_empty() {
                dns.insert(key.into(), parsed);
            }
        }
        if let Some(map) = value.as_mapping_mut() {
            map.insert("hosts".into(), Value::Mapping(hosts));
        }
        Ok(value)
    }

    pub fn read(&self, cx: &Context<SwiftSubApp>) -> Result<DnsOptions, String> {
        let yaml = if self.yaml_mode {
            self.yaml.read(cx).value().to_string()
        } else {
            serde_yaml_ng::to_string(&self.form_value(cx)?).map_err(|error| error.to_string())?
        };
        DnsOptions::from_yaml(&yaml).map_err(|error| error.to_string())
    }
}

impl SwiftSubApp {
    pub fn workspace_set_dns_editor(
        &mut self,
        yaml_mode: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match self.workspace.dns_editor.read(cx) {
            Ok(options) => {
                self.workspace.dns_editor.load(&options, window, cx);
                self.workspace.dns_editor.yaml_mode = yaml_mode;
                self.template_error = None;
            }
            Err(error) => {
                self.workspace.settings_error_section = Some("基础与 DNS");
                self.template_error = Some(error);
            }
        }
        cx.notify();
    }
}
