use crate::error::{Error, Result};

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize)]
pub struct Rule {
    pub kind: String,
    pub payload: Option<String>,
    pub target: String,
    pub no_resolve: bool,
    pub source_ip: bool,
}

impl Rule {
    pub fn new(
        kind: impl Into<String>,
        payload: impl Into<String>,
        target: impl Into<String>,
    ) -> Self {
        Self {
            kind: kind.into().to_uppercase(),
            payload: Some(payload.into()),
            target: target.into(),
            no_resolve: false,
            source_ip: false,
        }
    }

    pub fn rule_set(name: impl Into<String>, target: impl Into<String>, no_resolve: bool) -> Self {
        Self {
            kind: "RULE-SET".to_string(),
            payload: Some(name.into()),
            target: target.into(),
            no_resolve,
            source_ip: false,
        }
    }

    pub fn match_all(target: impl Into<String>) -> Self {
        Self {
            kind: "MATCH".to_string(),
            payload: None,
            target: target.into(),
            no_resolve: false,
            source_ip: false,
        }
    }

    pub fn with_no_resolve(mut self, no_resolve: bool) -> Self {
        self.no_resolve = no_resolve;
        self
    }

    pub fn to_line(&self) -> String {
        if self.kind == "MATCH" {
            return format!("MATCH,{}", self.target);
        }

        let mut line = format!(
            "{},{},{}",
            self.kind,
            self.payload.as_deref().unwrap_or_default(),
            self.target
        );
        if self.source_ip {
            line.push_str(",src");
        }
        if self.no_resolve {
            line.push_str(",no-resolve");
        }
        line
    }

    pub fn parse(line: &str) -> Result<Self> {
        parse(line, true, 0)
    }

    /// Rule-provider and logical child entries have no outbound target.
    pub fn parse_body(line: &str) -> Result<Self> {
        parse(line, false, 0)
    }

    pub fn to_body(&self) -> String {
        let mut body = self.kind.clone();
        if let Some(payload) = &self.payload {
            body.push(',');
            body.push_str(payload);
        }
        if self.source_ip {
            body.push_str(",src");
        }
        if self.no_resolve {
            body.push_str(",no-resolve");
        }
        body
    }

    pub fn parts(line: &str) -> Vec<&str> {
        split_fields(line, true).unwrap_or_else(|_| vec![line.trim()])
    }

    pub fn referenced_rule_sets(&self) -> Vec<String> {
        let mut references = Vec::new();
        if self.kind == "RULE-SET" {
            references.extend(self.payload.clone());
        } else if matches!(self.kind.as_str(), "AND" | "OR" | "NOT" | "SUB-RULE")
            && let Some(payload) = &self.payload
            && let Ok(children) = logic_children(payload, self.kind == "SUB-RULE")
        {
            for child in children {
                if let Ok(rule) = Self::parse_body(child) {
                    references.extend(rule.referenced_rule_sets());
                }
            }
        }
        references
    }
}

fn invalid(message: impl Into<String>) -> Error {
    Error::InvalidLink(message.into())
}

fn split_fields(line: &str, with_target: bool) -> Result<Vec<&str>> {
    let kind = line
        .split(',')
        .next()
        .unwrap_or_default()
        .trim()
        .to_uppercase();
    if matches!(
        kind.as_str(),
        "DOMAIN-REGEX" | "PROCESS-NAME-REGEX" | "PROCESS-PATH-REGEX"
    ) {
        let Some((kind, body)) = line.split_once(',') else {
            return Ok(vec![line.trim()]);
        };
        if with_target {
            let Some((payload, target)) = body.rsplit_once(',') else {
                return Ok(vec![kind.trim(), body.trim()]);
            };
            return Ok(vec![kind.trim(), payload.trim(), target.trim()]);
        }
        return Ok(vec![kind.trim(), body.trim()]);
    }
    let logical = matches!(kind.as_str(), "AND" | "OR" | "NOT" | "SUB-RULE");
    if !logical {
        return Ok(line.split(',').map(str::trim).collect());
    }
    let mut depth = 0usize;
    let mut start = 0;
    let mut fields = Vec::new();
    for (index, character) in line.char_indices() {
        match character {
            '(' => {
                depth += 1;
                if depth > 64 {
                    return Err(invalid("逻辑规则嵌套过深"));
                }
            }
            ')' => {
                depth = depth
                    .checked_sub(1)
                    .ok_or_else(|| invalid("逻辑规则括号不匹配"))?
            }
            ',' if depth == 0 => {
                fields.push(line[start..index].trim());
                start = index + 1;
            }
            _ => {}
        }
    }
    if depth != 0 {
        return Err(invalid("逻辑规则缺少右括号"));
    }
    fields.push(line[start..].trim());
    Ok(fields)
}

fn logic_children(payload: &str, single: bool) -> Result<Vec<&str>> {
    let payload = payload
        .trim()
        .strip_prefix('(')
        .and_then(|body| body.strip_suffix(')'))
        .ok_or_else(|| invalid("逻辑规则匹配内容须放在括号内"))?;
    if single {
        return Ok(vec![payload]);
    }
    let mut depth = 0usize;
    let mut start = 0;
    let mut children = Vec::new();
    let mut needs_child = true;
    for (index, character) in payload.char_indices() {
        match character {
            '(' => {
                if depth == 0 {
                    if !needs_child {
                        return Err(invalid("逻辑子规则之间需要逗号"));
                    }
                    start = index + 1;
                }
                depth += 1;
            }
            ')' => {
                depth = depth
                    .checked_sub(1)
                    .ok_or_else(|| invalid("逻辑规则括号不匹配"))?;
                if depth == 0 {
                    children.push(payload[start..index].trim());
                    needs_child = false;
                }
            }
            ',' if depth == 0 => {
                if needs_child {
                    return Err(invalid("逻辑规则中存在空子规则"));
                }
                needs_child = true;
            }
            ' ' | '\t' if depth == 0 => {}
            _ if depth == 0 => return Err(invalid("逻辑子规则须各自放在括号内")),
            _ => {}
        }
    }
    if depth != 0 || children.is_empty() || needs_child {
        return Err(invalid("逻辑规则缺少有效子规则"));
    }
    Ok(children)
}

fn parse(line: &str, with_target: bool, depth: usize) -> Result<Rule> {
    if depth > 32 {
        return Err(invalid("逻辑规则嵌套过深"));
    }
    if line.contains(['\n', '\r']) {
        return Err(invalid("一条规则不能包含换行"));
    }
    let parts = split_fields(line.trim(), with_target)?;
    let kind = parts.first().unwrap_or(&"").to_uppercase();
    if matches!(kind.as_str(), "MATCH" | "FINAL") {
        if parts.len() != 1 + usize::from(with_target) || with_target && parts[1].is_empty() {
            return Err(invalid("MATCH 规则只需填写目标策略"));
        }
        return Ok(Rule::match_all(if with_target {
            parts[1]
        } else {
            "DIRECT"
        }));
    }
    let needed = 2 + usize::from(with_target);
    if parts.len() < needed || parts[1].is_empty() || with_target && parts[2].is_empty() {
        return Err(invalid("规则需要有效的类型、匹配内容与目标策略"));
    }
    let mut rule = Rule::new(
        kind,
        parts[1],
        if with_target { parts[2] } else { "DIRECT" },
    );
    if matches!(rule.kind.as_str(), "AND" | "OR" | "NOT" | "SUB-RULE") && parts.len() != needed {
        return Err(invalid(
            "逻辑规则目标须在最后，不能附加 src / no-resolve 参数",
        ));
    }
    for parameter in &parts[needed..] {
        match *parameter {
            "no-resolve" if !rule.no_resolve => rule.no_resolve = true,
            "src" if !rule.source_ip => rule.source_ip = true,
            _ => return Err(invalid(format!("不支持或重复的规则参数：{parameter}"))),
        }
    }
    validate_payload(&rule, depth)?;
    Ok(rule)
}

fn validate_payload(rule: &Rule, depth: usize) -> Result<()> {
    let payload = rule.payload.as_deref().unwrap_or_default();
    let valid = match rule.kind.as_str() {
        "DOMAIN" | "DOMAIN-SUFFIX" | "DOMAIN-KEYWORD" | "DOMAIN-WILDCARD" => {
            !payload.contains([' ', '\t', ':'])
        }
        "IP-CIDR" | "IP-CIDR6" | "SRC-IP-CIDR" | "IP-SUFFIX" | "SRC-IP-SUFFIX" => {
            payload.split_once('/').is_some_and(|(address, prefix)| {
                address
                    .parse::<std::net::IpAddr>()
                    .ok()
                    .zip(prefix.parse::<u8>().ok())
                    .is_some_and(|(address, prefix)| {
                        prefix <= if address.is_ipv4() { 32 } else { 128 }
                    })
            })
        }
        "IP-ASN" | "SRC-IP-ASN" => payload.parse::<u32>().is_ok(),
        "UID" => unsigned_ranges(payload, u32::MAX.into()),
        "DSCP" => unsigned_ranges(payload, 63),
        "DST-PORT" | "SRC-PORT" | "IN-PORT" => unsigned_ranges(payload, u16::MAX.into()),
        "NETWORK" => matches!(payload.to_uppercase().as_str(), "TCP" | "UDP"),
        "DOMAIN-REGEX" | "PROCESS-PATH-REGEX" | "PROCESS-NAME-REGEX" => {
            regex::Regex::new(payload).is_ok()
        }
        "GEOSITE"
        | "GEOIP"
        | "SRC-GEOIP"
        | "PROCESS-NAME"
        | "PROCESS-PATH"
        | "PROCESS-PATH-WILDCARD"
        | "PROCESS-NAME-WILDCARD"
        | "IN-TYPE"
        | "IN-USER"
        | "IN-NAME"
        | "REMATCH-NAME"
        | "RULE-SET" => true,
        "AND" | "OR" | "NOT" | "SUB-RULE" => {
            let children = logic_children(payload, rule.kind == "SUB-RULE")?;
            if matches!(rule.kind.as_str(), "NOT" | "SUB-RULE") && children.len() != 1 {
                return Err(invalid("NOT / SUB-RULE 必须只包含一条子规则"));
            }
            for child in children {
                let child = parse(child, false, depth + 1)?;
                if matches!(child.kind.as_str(), "MATCH" | "SUB-RULE") {
                    return Err(invalid("逻辑子规则不支持 MATCH / SUB-RULE"));
                }
            }
            true
        }
        _ => return Err(invalid(format!("不支持的规则类型：{}", rule.kind))),
    };
    if !valid {
        return Err(invalid(format!("{} 匹配内容无效：{payload}", rule.kind)));
    }
    Ok(())
}

fn unsigned_ranges(payload: &str, maximum: u64) -> bool {
    payload.split('/').all(|part| {
        let (first, last) = part.split_once('-').unwrap_or((part, part));
        first
            .trim()
            .parse::<u64>()
            .ok()
            .zip(last.trim().parse::<u64>().ok())
            .is_some_and(|(first, last)| first <= last && last <= maximum)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn logical_rules_and_source_flags_preserve_payload_and_targets() {
        let line = "AND,((DOMAIN,example.org),(NOT,((NETWORK,UDP)))),节点选择";
        let rule = Rule::parse(line).unwrap();
        assert_eq!(rule.to_line(), line);
        assert_eq!(rule.target, "节点选择");
        let rule = Rule::parse("IP-CIDR,192.0.2.0/24,DIRECT,src,no-resolve").unwrap();
        assert!(rule.source_ip && rule.no_resolve);
        assert_eq!(
            Rule::parse_body(&rule.to_body()).unwrap().to_body(),
            rule.to_body()
        );
        let rule = Rule::parse("OR,((RULE-SET,domains),(NETWORK,TCP)),DIRECT").unwrap();
        assert_eq!(rule.referenced_rule_sets(), ["domains"]);
        assert!(Rule::parse("PROCESS-PATH,C:\\Program Files (x86)\\app.exe,DIRECT").is_ok());
        let regex = "DOMAIN-REGEX,^test[a-z]{1,3}\\.example\\.org$,DIRECT";
        let rule = Rule::parse(regex).unwrap();
        assert_eq!(rule.to_line(), regex);
        assert_eq!(
            Rule::parse_body(&rule.to_body()).unwrap().to_body(),
            rule.to_body()
        );
        assert_eq!(Rule::parts(regex)[2], "DIRECT");
        assert!(Rule::parse("UID,1000-1003/2000,DIRECT").is_ok());
        assert!(Rule::parse("DSCP,18/20-30,DIRECT").is_ok());
    }
    #[test]
    fn invalid_rule_types_payloads_and_logic_fail_before_output() {
        for line in [
            "aaa,example.org,DIRECT",
            "IP-CIDR,192.0.2.0/99,DIRECT",
            "DST-PORT,65536,DIRECT",
            "NETWORK,quic,DIRECT",
            "DOMAIN-REGEX,[,DIRECT",
            "NOT,((NETWORK,TCP),(DOMAIN,example.org)),DIRECT",
            "AND,((DOMAIN,example.org),DIRECT",
            "AND,(),DIRECT",
            "AND,((NETWORK,TCP),,(DOMAIN,example.org)),DIRECT",
            "AND,((NETWORK,TCP)(DOMAIN,example.org)),DIRECT",
            "AND,((NETWORK,TCP),),DIRECT",
            "AND,((NETWORK,TCP)),DIRECT,no-resolve",
            "DOMAIN,example.org,DIRECT,no-resolve,no-resolve",
        ] {
            assert!(Rule::parse(line).is_err(), "{line}");
        }
    }
}
