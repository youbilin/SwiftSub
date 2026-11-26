use serde_yaml_ng::{Mapping, Value};

use crate::error::{Error, Result};
use crate::model::Proxy;

use super::{ImportReportItem, ImportStatus, ParseWarning};

pub struct ClashYamlOutput {
    pub proxies: Vec<Proxy>,
    pub warnings: Vec<ParseWarning>,
    pub report: Vec<ImportReportItem>,
}

pub fn parse_clash_yaml(input: &str) -> Result<ClashYamlOutput> {
    let value: Value = serde_yaml_ng::from_str(input)?;
    let mapping = value
        .as_mapping()
        .ok_or_else(|| Error::InvalidLink("Clash YAML root must be a mapping".into()))?;

    let proxies_value = mapping_get(mapping, "proxies").ok_or(Error::MissingField("proxies"))?;
    let sequence = proxies_value
        .as_sequence()
        .ok_or_else(|| Error::InvalidLink("'proxies' must be a sequence".into()))?;

    let mut proxies = Vec::with_capacity(sequence.len());
    let mut warnings = Vec::new();
    let mut report = Vec::new();

    for (index, item) in sequence.iter().enumerate() {
        match Proxy::from_value(item.clone()) {
            Ok(proxy) => {
                if !is_known_protocol(proxy.kind()) {
                    warnings.push(ParseWarning::new(
                        Some(index + 1),
                        format!(
                            "节点 '{}' 的协议 '{}' 未验证，将原样透传",
                            proxy.name(),
                            proxy.kind()
                        ),
                    ));
                }
                let mut messages: Vec<_> = warnings
                    .iter()
                    .filter(|w| w.line == Some(index + 1))
                    .map(|w| w.message.clone())
                    .collect();
                let status = if let Some(issue) = proxy.fatal_issue() {
                    messages.push(issue);
                    ImportStatus::Rejected
                } else if messages.is_empty() {
                    ImportStatus::Accepted
                } else {
                    ImportStatus::Warning
                };
                report.push(ImportReportItem {
                    index: index + 1,
                    name: Some(proxy.name().to_string()),
                    protocol: Some(proxy.kind().to_string()),
                    status,
                    messages,
                });
                proxies.push(proxy);
            }
            Err(error) => {
                let message = format!("跳过无效节点: {error}");
                report.push(ImportReportItem {
                    index: index + 1,
                    name: None,
                    protocol: None,
                    status: ImportStatus::Rejected,
                    messages: vec![message.clone()],
                });
                warnings.push(ParseWarning::new(Some(index + 1), message));
            }
        }
    }

    Ok(ClashYamlOutput {
        proxies,
        warnings,
        report,
    })
}

pub fn looks_like_clash_yaml(input: &str) -> bool {
    let trimmed = input.trim_start();
    if trimmed.starts_with('{') && trimmed.contains("\"proxies\"") {
        return true;
    }

    for line in input.lines().take(40) {
        let line = line.trim_start();
        if line.starts_with('#') || line.is_empty() {
            continue;
        }
        if line.starts_with("proxies:")
            || line.starts_with("proxy-groups:")
            || line.starts_with("proxy-providers:")
            || line.starts_with("mixed-port:")
            || line.starts_with("mode:")
            || line.starts_with("rule-providers:")
        {
            return true;
        }
    }
    false
}

fn mapping_get<'a>(mapping: &'a Mapping, key: &str) -> Option<&'a Value> {
    mapping
        .iter()
        .find_map(|(candidate, value)| (candidate.as_str() == Some(key)).then_some(value))
}

pub fn is_known_protocol(kind: &str) -> bool {
    matches!(
        kind,
        "ss" | "ssr"
            | "vmess"
            | "vless"
            | "trojan"
            | "hysteria"
            | "hysteria2"
            | "tuic"
            | "anytls"
            | "snell"
            | "wireguard"
            | "http"
            | "socks5"
            | "socks4"
            | "ssh"
            | "mieru"
            | "juicity"
            | "masque"
            | "direct"
            | "dns"
            | "reject"
    )
}
