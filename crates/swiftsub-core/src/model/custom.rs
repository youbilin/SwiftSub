use serde::{Deserialize, Serialize};

use crate::model::{GroupKind, Node};
use crate::region::Region;

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct NodeFilter {
    #[serde(default)]
    pub regions: Vec<Region>,
    #[serde(default)]
    pub sources: Vec<String>,
    #[serde(default)]
    pub include_keywords: Vec<String>,
    #[serde(default)]
    pub exclude_keywords: Vec<String>,
}

impl NodeFilter {
    pub fn new(regions: Vec<Region>) -> Self {
        Self {
            regions,
            ..Default::default()
        }
    }

    pub fn matches(&self, node: &Node) -> bool {
        if !self.regions.is_empty() && !self.regions.contains(&node.region) {
            return false;
        }

        if !self.sources.is_empty() {
            let source_id = node.source_id.to_lowercase();
            let source_name = node.source_name.to_lowercase();
            let matched = self.sources.iter().any(|source| {
                let source = source.to_lowercase();
                source == source_id || source == source_name
            });
            if !matched {
                return false;
            }
        }

        let name = node.name().to_lowercase();
        if !self.include_keywords.is_empty()
            && !self
                .include_keywords
                .iter()
                .any(|keyword| name.contains(&keyword.to_lowercase()))
        {
            return false;
        }

        !self
            .exclude_keywords
            .iter()
            .any(|keyword| name.contains(&keyword.to_lowercase()))
    }

    pub fn is_empty(&self) -> bool {
        self.regions.is_empty()
            && self.sources.is_empty()
            && self.include_keywords.is_empty()
            && self.exclude_keywords.is_empty()
    }

    pub fn summary(&self) -> String {
        let mut parts = Vec::new();
        if !self.regions.is_empty() {
            parts.push(
                self.regions
                    .iter()
                    .map(|region| region.label())
                    .collect::<Vec<_>>()
                    .join("/"),
            );
        }
        if !self.include_keywords.is_empty() {
            parts.push(format!("含 {}", self.include_keywords.join(", ")));
        }
        if !self.exclude_keywords.is_empty() {
            parts.push(format!("排除 {}", self.exclude_keywords.join(", ")));
        }
        if parts.is_empty() {
            "全部节点".to_string()
        } else {
            parts.join(" · ")
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CustomGroupSpec {
    pub name: String,
    #[serde(default)]
    pub disabled: bool,
    #[serde(default)]
    pub kind: GroupKind,
    pub filter: NodeFilter,
}

impl CustomGroupSpec {
    pub fn new(name: impl Into<String>, filter: NodeFilter) -> Self {
        Self {
            name: name.into(),
            disabled: false,
            kind: GroupKind::Select,
            filter,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ChainSpec {
    pub name: String,
    #[serde(default)]
    pub disabled: bool,
    #[serde(default)]
    pub entry_node: String,
    #[serde(default)]
    pub entry_name: String,
    #[serde(default)]
    pub landing_node: String,
    #[serde(default)]
    pub hops: Vec<String>,
    #[serde(default)]
    pub landing_name: String,
}

impl ChainSpec {
    pub fn new(
        name: impl Into<String>,
        entry_node: impl Into<String>,
        entry_name: impl Into<String>,
        landing_node: impl Into<String>,
        landing_name: impl Into<String>,
    ) -> Self {
        Self {
            name: name.into(),
            disabled: false,
            entry_node: entry_node.into(),
            entry_name: entry_name.into(),
            landing_node: landing_node.into(),
            hops: Vec::new(),
            landing_name: landing_name.into(),
        }
    }
}

/// A routing policy group with explicit group/policy members and optional all-node membership.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PolicyGroupSpec {
    #[serde(default)]
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub disabled: bool,
    /// Preserve a deleted catalog group's identity so it is not regenerated.
    #[serde(default)]
    pub removed: bool,
    #[serde(default)]
    pub kind: GroupKind,
    #[serde(default)]
    pub members: Vec<String>,
    #[serde(default)]
    pub include_all_nodes: bool,
    /// Replace a catalog group, retaining its original name in `id` as `builtin:<name>`.
    #[serde(default)]
    pub builtin: bool,
    #[serde(default)]
    pub test_url: Option<String>,
    #[serde(default)]
    pub test_interval: Option<u32>,
    #[serde(default)]
    pub tolerance: Option<u32>,
    #[serde(default)]
    pub strategy: Option<String>,
}

impl PolicyGroupSpec {
    /// The catalog identity stays stable when the visible name changes.
    /// Older configurations without an identity still use their original name.
    pub fn builtin_name(&self) -> Option<&str> {
        self.builtin
            .then(|| self.id.strip_prefix("builtin:").unwrap_or(&self.name))
    }

    pub fn ensure_id(&mut self) {
        if self.builtin && !self.id.starts_with("builtin:") {
            self.id = format!("builtin:{}", self.name);
        }
    }
}
