mod extended;
mod helpers;
mod protocols;
#[cfg(test)]
mod tests;

use extended::{
    parse_extended_proxy, parse_hysteria_v1, parse_juicity, parse_simple_proxy, parse_ssr,
};
use helpers::{link_compatibility_warnings, truncate};
use protocols::{parse_hysteria2, parse_ss, parse_trojan, parse_tuic, parse_vless, parse_vmess};

use crate::error::{Error, Result};
use crate::model::Proxy;

use super::{ImportReportItem, ImportStatus, ParseWarning};

pub struct LinkOutput {
    pub proxies: Vec<Proxy>,
    pub warnings: Vec<ParseWarning>,
    pub report: Vec<ImportReportItem>,
}

pub fn parse_links(input: &str) -> LinkOutput {
    let mut proxies = Vec::new();
    let mut warnings = Vec::new();
    let mut report = Vec::new();

    for (index, raw_line) in input.lines().enumerate() {
        let line = raw_line.trim();
        if line.is_empty() || line.starts_with("//") || line.starts_with(';') {
            continue;
        }
        if !line.contains("://") {
            report.push(ImportReportItem {
                index: index + 1,
                name: None,
                protocol: None,
                status: ImportStatus::Rejected,
                messages: vec!["无法识别的节点链接".into()],
            });
            warnings.push(ParseWarning::new(
                Some(index + 1),
                format!("无法识别的节点链接: {}", truncate(line, 80)),
            ));
            continue;
        }

        match parse_link(line) {
            Ok(proxy) => {
                let messages = link_compatibility_warnings(line);
                report.push(ImportReportItem {
                    index: index + 1,
                    name: Some(proxy.name().to_string()),
                    protocol: Some(proxy.kind().to_string()),
                    status: if messages.is_empty() {
                        ImportStatus::Accepted
                    } else {
                        ImportStatus::Warning
                    },
                    messages: messages.clone(),
                });
                for message in messages {
                    warnings.push(ParseWarning::new(Some(index + 1), message));
                }
                proxies.push(proxy);
            }
            Err(error) => {
                let message = error.to_string();
                report.push(ImportReportItem {
                    index: index + 1,
                    name: None,
                    protocol: line.split_once("://").map(|(s, _)| s.to_string()),
                    status: ImportStatus::Rejected,
                    messages: vec![message.clone()],
                });
                warnings.push(ParseWarning::new(Some(index + 1), message));
            }
        }
    }

    LinkOutput {
        proxies,
        warnings,
        report,
    }
}

fn parse_link(line: &str) -> Result<Proxy> {
    let (scheme, _) = line
        .split_once("://")
        .ok_or_else(|| Error::InvalidLink("missing scheme".into()))?;
    let rest = &line[scheme.len() + 3..];

    match scheme.to_lowercase().as_str() {
        "ss" => parse_ss(rest),
        "ssr" => parse_ssr(rest),
        "anytls" | "snell" | "wireguard" | "wg" | "ssh" => parse_extended_proxy(scheme, rest),
        "vmess" => parse_vmess(rest),
        "vless" => parse_vless(rest),
        "trojan" => parse_trojan(rest),
        "hysteria" => parse_hysteria_v1(rest),
        "hysteria2" | "hy2" => parse_hysteria2(rest),
        "juicity" => parse_juicity(rest),
        "tuic" => parse_tuic(rest),
        "http" | "https" | "socks" | "socks5" | "socks4" => parse_simple_proxy(scheme, rest),
        other => Err(Error::UnsupportedProtocol(other.to_string())),
    }
}
