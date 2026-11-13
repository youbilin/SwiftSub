mod base64_utils;
mod clash_yaml;
mod link;

use crate::model::Proxy;

pub use base64_utils::{decode_base64, decode_base64_text, try_decode_base64_text};
pub use clash_yaml::{is_known_protocol, looks_like_clash_yaml, parse_clash_yaml};

use crate::error::{Error, Result};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SourceKind {
    ClashYaml,
    Links,
    Base64Links,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ParseWarning {
    pub line: Option<usize>,
    pub message: String,
}

impl ParseWarning {
    pub fn new(line: Option<usize>, message: impl Into<String>) -> Self {
        Self {
            line,
            message: message.into(),
        }
    }
}

#[derive(Clone, Debug)]
pub struct ParseOutput {
    pub proxies: Vec<Proxy>,
    pub warnings: Vec<ParseWarning>,
    pub source_kind: SourceKind,
}

pub fn parse_content(content: &str) -> Result<ParseOutput> {
    parse_content_inner(content, 0)
}

fn parse_content_inner(content: &str, depth: usize) -> Result<ParseOutput> {
    if depth > 2 {
        return Err(Error::InvalidLink(
            "subscription nesting is too deep".into(),
        ));
    }

    let content = content.trim_start_matches('\u{feff}').trim();
    if content.is_empty() {
        return Err(Error::InvalidLink("empty subscription content".into()));
    }

    if looks_like_clash_yaml(content) {
        let output = parse_clash_yaml(content)?;
        return Ok(ParseOutput {
            proxies: output.proxies,
            warnings: output.warnings,
            source_kind: SourceKind::ClashYaml,
        });
    }

    if !content.contains("://")
        && let Some(decoded) = try_decode_base64_text(content)
    {
        let mut output = parse_content_inner(&decoded, depth + 1)?;
        if output.source_kind == SourceKind::Links {
            output.source_kind = SourceKind::Base64Links;
        }
        return Ok(output);
    }

    let output = link::parse_links(content);
    if output.proxies.is_empty() && output.warnings.is_empty() {
        return Err(Error::InvalidLink("no nodes found in subscription".into()));
    }
    Ok(ParseOutput {
        proxies: output.proxies,
        warnings: output.warnings,
        source_kind: SourceKind::Links,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_plain_link_list() {
        let content = "trojan://pass@example.com:443#HK-01\n\
                       ss://YWVzLTI1Ni1nY206cGFzcw==@example.com:8388#JP-01";
        let output = parse_content(content).unwrap();
        assert_eq!(output.proxies.len(), 2);
        assert_eq!(output.proxies[0].kind(), "trojan");
        assert_eq!(output.proxies[1].kind(), "ss");
    }

    #[test]
    fn parses_base64_link_list() {
        use base64::Engine;
        let raw = "trojan://pass@example.com:443#HK-01";
        let encoded = base64::engine::general_purpose::STANDARD.encode(raw);
        let output = parse_content(&encoded).unwrap();
        assert_eq!(output.proxies.len(), 1);
        assert_eq!(output.source_kind, SourceKind::Base64Links);
    }
}
