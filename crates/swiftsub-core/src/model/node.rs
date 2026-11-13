use crate::model::Proxy;
use crate::region::{Region, detect_region};

#[derive(Clone, Debug)]
pub struct Node {
    pub proxy: Proxy,
    pub source_key: String,
    pub original_name: String,
    pub source_id: String,
    pub source_name: String,
    pub region: Region,
    pub enabled: bool,
    pub listen_port: Option<u16>,
}

impl Node {
    pub fn new(proxy: Proxy, source_id: impl Into<String>, source_name: impl Into<String>) -> Self {
        let source_key = proxy.identity_key();
        let original_name = proxy.name().to_string();
        let region = detect_region(&original_name);
        Self {
            proxy,
            source_key,
            original_name,
            source_id: source_id.into(),
            source_name: source_name.into(),
            region,
            enabled: true,
            listen_port: None,
        }
    }

    pub fn name(&self) -> &str {
        self.proxy.name()
    }

    pub fn set_name(&mut self, name: impl Into<String>) {
        self.proxy.set_name(name);
    }
}
