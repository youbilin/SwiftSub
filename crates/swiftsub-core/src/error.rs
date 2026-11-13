use thiserror::Error;

#[derive(Debug, Error)]
pub enum Error {
    #[error("invalid configuration: {0}")]
    InvalidConfig(String),
    #[error("invalid node link: {0}")]
    InvalidLink(String),
    #[error("unsupported protocol: {0}")]
    UnsupportedProtocol(String),
    #[error("missing field: {0}")]
    MissingField(&'static str),
    #[error("invalid port: {0}")]
    InvalidPort(String),
    #[error("invalid URL: {0}")]
    Url(#[from] url::ParseError),
    #[error("invalid base64: {0}")]
    Base64(#[from] base64::DecodeError),
    #[error("invalid JSON: {0}")]
    Json(#[from] serde_json::Error),
    #[error("invalid YAML: {0}")]
    Yaml(#[from] serde_yaml_ng::Error),
    #[error("subscription contains no usable nodes")]
    NoNodes,
    #[error("rule target does not exist: {0}")]
    UnknownRuleTarget(String),
}

pub type Result<T, E = Error> = std::result::Result<T, E>;
