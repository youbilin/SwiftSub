use crate::error::{Error, Result};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Rule {
    pub kind: String,
    pub payload: Option<String>,
    pub target: String,
    pub no_resolve: bool,
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
        }
    }

    pub fn rule_set(name: impl Into<String>, target: impl Into<String>, no_resolve: bool) -> Self {
        Self {
            kind: "RULE-SET".to_string(),
            payload: Some(name.into()),
            target: target.into(),
            no_resolve,
        }
    }

    pub fn match_all(target: impl Into<String>) -> Self {
        Self {
            kind: "MATCH".to_string(),
            payload: None,
            target: target.into(),
            no_resolve: false,
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

        let payload = self.payload.as_deref().unwrap_or_default();
        if self.no_resolve {
            format!("{},{},{},no-resolve", self.kind, payload, self.target)
        } else {
            format!("{},{},{}", self.kind, payload, self.target)
        }
    }

    pub fn parse(line: &str) -> Result<Self> {
        let parts: Vec<&str> = line.split(',').map(str::trim).collect();
        if parts.len() < 2 {
            return Err(Error::InvalidLink(format!("invalid rule: {line}")));
        }

        let kind = parts[0].to_uppercase();
        if kind == "MATCH" {
            if parts.len() != 2 || parts[1].is_empty() {
                return Err(Error::InvalidLink(format!("invalid MATCH rule: {line}")));
            }
            return Ok(Self::match_all(parts[1]));
        }

        if parts.len() < 3 || parts[1].is_empty() || parts[2].is_empty() {
            return Err(Error::InvalidLink(format!("invalid rule: {line}")));
        }

        let no_resolve = parts.get(3).is_some_and(|value| *value == "no-resolve");
        Ok(Self::new(parts[0], parts[1], parts[2]).with_no_resolve(no_resolve))
    }
}
