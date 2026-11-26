use super::*;

#[derive(Clone)]
pub(crate) struct WorkspaceGroupItem {
    pub name: String,
    pub index: Option<usize>,
    pub module: Option<&'static str>,
    pub filter: Option<usize>,
    pub enabled: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn keys(names: &[&str]) -> Vec<String> {
        names.iter().map(|name| (*name).into()).collect()
    }

    #[test]
    fn adjacent_and_long_moves_work_in_both_directions() {
        let visible = keys(&["a", "b", "c", "d"]);
        assert_eq!(
            moved_order(&visible, &visible, "a", "b"),
            Some(keys(&["b", "a", "c", "d"]))
        );
        assert_eq!(
            moved_order(&visible, &visible, "b", "a"),
            Some(keys(&["b", "a", "c", "d"]))
        );
        assert_eq!(
            moved_order(&visible, &visible, "a", "d"),
            Some(keys(&["b", "c", "d", "a"]))
        );
        assert_eq!(
            moved_order(&visible, &visible, "d", "a"),
            Some(keys(&["d", "a", "b", "c"]))
        );
        assert_eq!(moved_order(&visible, &visible, "a", "a"), None);
        assert_eq!(moved_order(&visible, &visible, "a", "missing"), None);
        assert_eq!(moved_order(&visible, &visible, "missing", "a"), None);
    }

    #[test]
    fn hidden_source_slots_survive_reordering_and_new_items_are_retained() {
        let saved = keys(&["a", "hidden", "a", "b", "missing"]);
        let visible = keys(&["a", "b", "new"]);
        let order = moved_order(&saved, &visible, "a", "b").unwrap();
        assert_eq!(order, keys(&["b", "hidden", "a", "missing", "new"]));
        let json = serde_json::to_string(&order).unwrap();
        assert_eq!(serde_json::from_str::<Vec<String>>(&json).unwrap(), order);
    }

    #[test]
    fn renamed_and_disabled_groups_share_one_order_with_filters() {
        let groups = vec![
            PolicyGroupSpec {
                id: "builtin:节点选择".into(),
                name: "手动出口".into(),
                builtin: true,
                ..Default::default()
            },
            PolicyGroupSpec {
                name: "工作".into(),
                disabled: true,
                ..Default::default()
            },
            PolicyGroupSpec {
                id: "builtin:自动选择".into(),
                name: "测速".into(),
                builtin: true,
                removed: true,
                disabled: true,
                ..Default::default()
            },
        ];
        let filters = vec![CustomGroupSpec::new("筛选", NodeFilter::default())];
        let items = group_items(
            &groups,
            &filters,
            &HashSet::new(),
            &keys(&["筛选", "工作", "手动出口", "漏网之鱼"]),
        );
        assert_eq!(
            items
                .iter()
                .map(|item| item.name.as_str())
                .collect::<Vec<_>>(),
            ["筛选", "工作", "手动出口", "漏网之鱼"]
        );
        assert_eq!(items[0].filter, Some(0));
        assert!(!items[1].enabled);
        assert_eq!(items[2].index, Some(0));
        assert_eq!(items[2].module, Some("select"));
    }

    #[test]
    fn node_and_chain_order_keeps_source_identity_and_disabled_nodes() {
        let mut nodes = build_nodes(
            vec![
                Proxy::new("direct", "A", "127.0.0.1", 1),
                Proxy::new("direct", "B", "127.0.0.2", 1),
            ],
            "test",
            "test",
        );
        nodes[1].enabled = false;
        let chain = ChainSpec::new(
            "Chain",
            &nodes[0].source_key,
            "A",
            &nodes[1].source_key,
            "B",
        );
        let order = vec![
            nodes[1].source_key.clone(),
            chain_order_key(&chain.name),
            nodes[0].source_key.clone(),
        ];
        let items = node_items(&nodes, &[chain], &order);
        assert_eq!(
            items.iter().map(|item| &item.key).collect::<Vec<_>>(),
            order.iter().collect::<Vec<_>>()
        );
        assert!(matches!(items[0].kind, WorkspaceNodeKind::Node(1)));
        assert!(matches!(items[1].kind, WorkspaceNodeKind::Chain(0)));
        assert!(matches!(items[2].kind, WorkspaceNodeKind::Node(0)));
    }
}

#[derive(Clone, Copy)]
pub(crate) enum WorkspaceNodeKind {
    Node(usize),
    Chain(usize),
}

pub(crate) struct WorkspaceNodeItem {
    pub key: String,
    pub kind: WorkspaceNodeKind,
}

pub(crate) fn chain_order_key(name: &str) -> String {
    format!("chain:{name}")
}

fn positions(order: &[String]) -> HashMap<&str, usize> {
    order
        .iter()
        .enumerate()
        .map(|(index, key)| (key.as_str(), index))
        .collect()
}

fn group_items(
    groups: &[PolicyGroupSpec],
    filters: &[CustomGroupSpec],
    modules: &HashSet<String>,
    order: &[String],
) -> Vec<WorkspaceGroupItem> {
    let mut items = Vec::new();
    for (index, group) in groups.iter().enumerate() {
        if !group.builtin && !group.removed {
            items.push(WorkspaceGroupItem {
                name: group.name.clone(),
                index: Some(index),
                module: None,
                filter: None,
                enabled: !group.disabled,
            });
        }
    }
    for module in catalog::MODULES
        .iter()
        .filter(|module| !matches!(module.kind, catalog::ModuleKind::Region(_)))
    {
        let index = groups
            .iter()
            .position(|group| group.builtin_name() == Some(module.name));
        let spec = index.map(|index| &groups[index]);
        if spec.is_some_and(|group| group.removed)
            || (!modules.contains(module.id)
                && !matches!(module.id, "select" | "auto" | "final")
                && spec.is_none())
        {
            continue;
        }
        items.push(WorkspaceGroupItem {
            name: spec.map_or(module.name, |group| &group.name).to_string(),
            index,
            module: Some(module.id),
            filter: None,
            enabled: !spec.is_some_and(|group| group.disabled),
        });
    }
    for (index, filter) in filters.iter().enumerate() {
        items.push(WorkspaceGroupItem {
            name: filter.name.clone(),
            index: None,
            module: None,
            filter: Some(index),
            enabled: !filter.disabled,
        });
    }
    let ranks = positions(order);
    items.sort_by_key(|item| ranks.get(item.name.as_str()).copied().unwrap_or(usize::MAX));
    items
}

fn node_items(nodes: &[Node], chains: &[ChainSpec], order: &[String]) -> Vec<WorkspaceNodeItem> {
    let mut items: Vec<_> = nodes
        .iter()
        .enumerate()
        .map(|(index, node)| WorkspaceNodeItem {
            key: node.source_key.clone(),
            kind: WorkspaceNodeKind::Node(index),
        })
        .chain(
            chains
                .iter()
                .enumerate()
                .map(|(index, chain)| WorkspaceNodeItem {
                    key: chain_order_key(&chain.name),
                    kind: WorkspaceNodeKind::Chain(index),
                }),
        )
        .collect();
    let ranks = positions(order);
    items.sort_by_key(|item| ranks.get(item.key.as_str()).copied().unwrap_or(usize::MAX));
    items
}

/// Moving down uses the target's original position, including the adjacent case.
/// Hidden items retain their slots until they become available again.
fn moved_order(
    saved: &[String],
    visible: &[String],
    dragged: &str,
    target: &str,
) -> Option<Vec<String>> {
    let from = visible.iter().position(|key| key == dragged)?;
    let to = visible.iter().position(|key| key == target)?;
    if from == to {
        return None;
    }
    let mut moved = visible.to_vec();
    let key = moved.remove(from);
    moved.insert(to, key);
    let visible: HashSet<_> = visible.iter().collect();
    let mut replacement = moved.into_iter();
    let mut order = Vec::new();
    let mut seen = HashSet::new();
    for key in saved {
        if !seen.insert(key) {
            continue;
        }
        if visible.contains(key) {
            if let Some(key) = replacement.next() {
                order.push(key);
            }
        } else {
            order.push(key.clone());
        }
    }
    order.extend(replacement);
    Some(order)
}

impl SwiftSubApp {
    pub(crate) fn workspace_group_items(&self) -> Vec<WorkspaceGroupItem> {
        group_items(
            &self.template_policy_groups,
            &self.custom_groups,
            &self.template_enabled_modules,
            &self.template_group_order,
        )
    }

    pub(crate) fn workspace_node_items(&self) -> Vec<WorkspaceNodeItem> {
        node_items(&self.nodes, &self.chains, &self.node_order)
    }

    pub(crate) fn workspace_resolved_group_name<'a>(&'a self, name: &'a str) -> &'a str {
        self.template_policy_groups
            .iter()
            .find(|group| group.builtin_name() == Some(name))
            .map(|group| group.name.as_str())
            .unwrap_or(name)
    }

    pub(crate) fn workspace_builtin_name<'a>(&'a self, name: &'a str) -> &'a str {
        self.template_policy_groups
            .iter()
            .find(|group| group.name == name)
            .and_then(PolicyGroupSpec::builtin_name)
            .unwrap_or(name)
    }

    pub fn move_workspace_group(
        &mut self,
        dragged: String,
        target: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Err(error) = self.autosave_routing(window, cx) {
            self.template_error = Some(error);
            cx.notify();
            return;
        }
        let visible: Vec<_> = self
            .workspace_group_items()
            .into_iter()
            .map(|item| item.name)
            .collect();
        let Some(order) = moved_order(&self.template_group_order, &visible, &dragged, &target)
        else {
            return;
        };
        let previous = std::mem::replace(&mut self.template_group_order, order);
        let result = self.autosave_routing(window, cx);
        if let Err(error) = result {
            self.template_group_order = previous;
            self.template_error = Some(error);
        } else if let Err(error) = self.checkpoint_draft(cx) {
            self.store_error = Some(error);
        }
        cx.notify();
    }

    pub(crate) fn workspace_selected_members(&self, index: usize) -> Vec<String> {
        let Some(group) = self.template_policy_groups.get(index) else {
            return Vec::new();
        };
        let mut members = group.members.clone();
        if group.include_all_nodes {
            members.extend(
                self.nodes
                    .iter()
                    .filter(|node| node.enabled)
                    .map(|node| node.name().to_string()),
            );
            members.extend(
                self.chains
                    .iter()
                    .filter(|chain| !chain.disabled)
                    .map(|chain| chain.name.clone()),
            );
        }
        let mut seen = HashSet::new();
        members.retain(|member| seen.insert(member.clone()));
        members
    }

    pub fn move_workspace_member(
        &mut self,
        index: usize,
        dragged: String,
        target: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Err(error) = self.autosave_routing(window, cx) {
            self.template_error = Some(error);
            cx.notify();
            return;
        }
        let members = self.workspace_selected_members(index);
        let Some(order) = moved_order(&members, &members, &dragged, &target) else {
            return;
        };
        let result = self.draft_template_snapshot(cx).and_then(|mut snapshot| {
            snapshot
                .policy_groups
                .get_mut(index)
                .ok_or("策略组已删除")?
                .members = order;
            self.persist_and_commit_template(snapshot.clone(), None, window, cx)?;
            self.sync_workspace_snapshot(snapshot, window, cx);
            Ok(())
        });
        if let Err(error) = result {
            self.template_error = Some(error);
        }
        cx.notify();
    }

    pub(super) fn apply_workspace_node_order(&mut self) {
        let ranks = positions(&self.node_order);
        self.nodes.sort_by_key(|node| {
            ranks
                .get(node.source_key.as_str())
                .copied()
                .unwrap_or(usize::MAX)
        });
        let editing = self
            .workspace
            .edit_chain
            .and_then(|index| self.chains.get(index))
            .map(|chain| chain.name.clone());
        self.chains.sort_by_key(|chain| {
            ranks
                .get(chain_order_key(&chain.name).as_str())
                .copied()
                .unwrap_or(usize::MAX)
        });
        self.workspace.edit_chain =
            editing.and_then(|name| self.chains.iter().position(|chain| chain.name == name));
    }

    pub fn move_node(
        &mut self,
        dragged: String,
        target: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Err(error) = self.autosave_routing(window, cx) {
            self.template_error = Some(error);
            cx.notify();
            return;
        }
        let visible: Vec<_> = self
            .workspace_node_items()
            .into_iter()
            .map(|item| item.key)
            .collect();
        let Some(order) = moved_order(&self.node_order, &visible, &dragged, &target) else {
            return;
        };
        let previous = self.node_order.clone();
        let previous_nodes = self.nodes.clone();
        let previous_chains = self.chains.clone();
        let previous_edit = self.workspace.edit_chain;
        self.node_order = order;
        self.apply_workspace_node_order();
        let result = self.draft_template_snapshot(cx).and_then(|snapshot| {
            self.persist_and_commit_template(snapshot.clone(), None, window, cx)?;
            self.template_baseline = Some(snapshot);
            self.template_base_options = Some(self.options.clone());
            self.refresh_advanced_draft(true, window, cx);
            Ok(())
        });
        if let Err(error) = result {
            self.node_order = previous;
            self.nodes = previous_nodes;
            self.chains = previous_chains;
            self.workspace.edit_chain = previous_edit;
            self.template_error = Some(error);
        } else if let Err(error) = self.checkpoint_draft(cx) {
            self.store_error = Some(error);
        }
        cx.notify();
    }
}
