use crate::error::{Error, Result};
use crate::model::Rule;
use serde_yaml_ng::Value;

pub fn parse_rule_set_payload(text: &str, behavior: &str) -> Result<Vec<String>> {
    if text.len() > 10 * 1024 * 1024 {
        return Err(Error::InvalidLink("规则集内容超过 10 MB".into()));
    }
    let raw = text.trim_start_matches('\u{feff}').trim();
    // Published YAML rule sets often start with generated metadata comments.
    let raw = raw
        .lines()
        .find(|line| !line.trim().is_empty() && !line.trim_start().starts_with('#'))
        .and_then(|line| raw.find(line).map(|offset| &raw[offset..]))
        .unwrap_or("")
        .trim_start();
    let payload = if raw.starts_with("payload:")
        || raw.starts_with("---")
        || raw.starts_with("- ")
        || raw.starts_with(['[', '{'])
        || raw
            .lines()
            .any(|line| line.trim_start().starts_with("payload:"))
    {
        let value: Value = serde_yaml_ng::from_str(raw)?;
        let sequence = value
            .as_sequence()
            .or_else(|| value.get("payload").and_then(Value::as_sequence))
            .ok_or_else(|| Error::InvalidLink("YAML 规则集必须包含 payload 列表".into()))?;
        sequence
            .iter()
            .map(|v| {
                v.as_str()
                    .map(str::to_string)
                    .ok_or_else(|| Error::InvalidLink("规则条目必须是文本".into()))
            })
            .collect::<Result<Vec<_>>>()?
    } else {
        raw.lines()
            .map(str::trim)
            .filter(|s| !s.is_empty() && !s.starts_with('#') && !s.starts_with("//"))
            .map(str::to_string)
            .collect()
    };
    if payload.is_empty() {
        return Err(Error::InvalidLink(
            "规则集不能为空；可停用不需要的规则集".into(),
        ));
    }
    for (index, entry) in payload.iter().enumerate() {
        let valid = match behavior {
            "domain" => !entry.is_empty() && !entry.contains([',', '\n', '\r', ' ', '\t', ':']),
            "ipcidr" => entry.split_once('/').is_some_and(|(ip, bits)| {
                ip.parse::<std::net::IpAddr>()
                    .ok()
                    .zip(bits.parse::<u8>().ok())
                    .is_some_and(|(ip, bits)| bits <= if ip.is_ipv4() { 32 } else { 128 })
            }),
            "classical" => Rule::parse_body(entry)
                .is_ok_and(|rule| !matches!(rule.kind.as_str(), "MATCH" | "SUB-RULE" | "RULE-SET")),
            _ => false,
        };
        if !valid {
            return Err(Error::InvalidLink(format!(
                "第 {} 条规则不符合 {behavior} 格式：{entry}",
                index + 1
            )));
        }
    }
    Ok(payload)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn validates_real_payloads_and_rejects_wrong_behavior() {
        assert_eq!(
            parse_rule_set_payload(
                "# Generated rules\n# Updated today\npayload:\n  - '+.example.org'\n  - api.example.net",
                "domain"
            )
            .unwrap()
            .len(),
            2
        );
        assert!(parse_rule_set_payload("192.0.2.0/24\n2001:db8::/32", "ipcidr").is_ok());
        assert!(
            parse_rule_set_payload(
                "DOMAIN-SUFFIX,example.org\nIP-CIDR,192.0.2.0/24,no-resolve",
                "classical"
            )
            .is_ok()
        );
        assert!(parse_rule_set_payload("192.0.2.0/99", "ipcidr").is_err());
        assert!(parse_rule_set_payload("DOMAIN-SUFFIX,example.org,REJECT", "classical").is_err());
        assert!(parse_rule_set_payload("payload: [123]", "domain").is_err());
    }
}
