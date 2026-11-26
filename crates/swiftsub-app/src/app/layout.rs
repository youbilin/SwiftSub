use super::*;

#[derive(Clone, Serialize, Deserialize)]
struct GenerationDraft {
    source_json: String,
    workspace_json: Option<String>,
    rule_content_json: Option<String>,
    yaml: Option<String>,
    #[serde(default)]
    deleted_nodes: HashSet<String>,
    #[serde(default)]
    node_order: Vec<String>,
    #[serde(default)]
    rule_visibility: RuleVisibility,
    #[serde(default)]
    nodes: Vec<DraftNode>,
}

#[derive(Clone, Serialize, Deserialize)]
struct DraftNode {
    source_id: i64,
    source_key: String,
    yaml: String,
    enabled: bool,
    listen_port: Option<u16>,
}

fn draft_node_overrides(
    store: &Store,
    nodes: &[DraftNode],
    subscriptions: &[StoredSubscription],
) -> Result<Vec<swiftsub_store::NodeOverride>, String> {
    let mut overrides = Vec::new();
    let mut previous = HashMap::new();
    for node in nodes.iter().filter(|node| {
        subscriptions
            .iter()
            .any(|subscription| subscription.id == node.source_id)
    }) {
        if let std::collections::hash_map::Entry::Vacant(entry) = previous.entry(node.source_id) {
            entry.insert(
                store
                    .list_node_overrides(node.source_id)
                    .map_err(|error| error.to_string())?,
            );
        }
        let proxy = Proxy::from_yaml_text(&node.yaml).map_err(|error| error.to_string())?;
        let existing = previous
            .get(&node.source_id)
            .and_then(|items| items.get(&node.source_key));
        overrides.push(swiftsub_store::NodeOverride {
            sub_id: node.source_id,
            node_key: node.source_key.clone(),
            alias: Some(proxy.name().into()),
            enabled: Some(node.enabled),
            listen_port: node.listen_port,
            sort_order: existing.and_then(|item| item.sort_order),
            extras: Some(node.yaml.clone()),
        });
    }
    Ok(overrides)
}

#[derive(Clone, Default, Serialize, Deserialize)]
pub(crate) struct RuleVisibility {
    pub disabled_sets: HashSet<String>,
    pub disabled_manual: Vec<String>,
}

pub(super) fn retarget_snapshot(snapshot: &mut TemplateSnapshot, old: &str, new: &str) {
    for name in &mut snapshot.group_order {
        if name == old {
            *name = new.into();
        }
    }
    for group in &mut snapshot.policy_groups {
        for member in &mut group.members {
            if member == old {
                *member = new.into();
            }
        }
        let mut seen = HashSet::new();
        group.members.retain(|member| seen.insert(member.clone()));
    }
    for binding in &mut snapshot.custom_rule_sets {
        if binding.target == old {
            binding.target = new.into();
        }
    }
    for target in snapshot.rule_set_targets.values_mut() {
        if target == old {
            *target = new.into();
        }
    }
    workspace::retarget_rule_lines(&mut snapshot.custom_rule_lines, old, new);
    workspace::retarget_rule_lines(&mut snapshot.rule_order, old, new);
    for listener in &mut snapshot.listeners {
        if listener.proxy == old {
            listener.proxy = new.into();
        }
    }
}

fn chain_uses_node(chain: &ChainSpec, node: &Node) -> bool {
    let selectors = if chain.hops.is_empty() {
        vec![chain.entry_node.as_str(), chain.landing_node.as_str()]
    } else {
        chain.hops.iter().map(String::as_str).collect()
    };
    selectors
        .iter()
        .any(|selector| *selector == node.source_key || *selector == node.name())
}

fn rename_chain_node(chain: &mut ChainSpec, node: &Node, new: &str) {
    if chain.entry_node == node.source_key || chain.entry_node == node.name() {
        chain.entry_node = node.source_key.clone();
        chain.entry_name = new.into();
    }
    if chain.landing_node == node.source_key || chain.landing_node == node.name() {
        chain.landing_node = node.source_key.clone();
        chain.landing_name = new.into();
    }
    for hop in &mut chain.hops {
        if hop == node.name() {
            *hop = node.source_key.clone();
        }
    }
}

fn save_temporary_config(
    store: &Store,
    directory: &std::path::Path,
    previous: Option<&swiftsub_store::SavedConfig>,
    name: &str,
    draft: &GenerationDraft,
) -> Result<swiftsub_store::SavedConfig, String> {
    let json = serde_json::to_string(draft).map_err(|error| error.to_string())?;
    let existing = if let Some(previous) = previous {
        let current = store
            .list_saved_configs()
            .map_err(|error| error.to_string())?
            .into_iter()
            .find(|config| config.id == previous.id)
            .ok_or("配置已删除，请存为新配置")?;
        if current.yaml != previous.yaml
            || current.name != previous.name
            || current.source_json != previous.source_json
        {
            return Err("此配置已被其他操作修改，草稿已保留，请重新打开或存为新配置".into());
        }
        Some(current)
    } else {
        None
    };
    let config = match (draft.yaml.as_deref(), existing) {
        (None, Some(config)) => config,
        (yaml, previous) => {
            let yaml = yaml.unwrap_or(
                "mixed-port: 7890\nproxies: []\nproxy-groups: []\nrules:\n  - MATCH,DIRECT\n",
            );
            saved_configs::save_generated_yaml(
                store,
                directory,
                previous.as_ref(),
                name,
                yaml,
                &draft.source_json,
            )?
        }
    };
    store
        .set_setting(&format!("generation-draft:{}", config.id), &json)
        .map_err(|error| error.to_string())?;
    if !config.path(directory).exists() {
        std::fs::create_dir_all(directory).map_err(|error| error.to_string())?;
        std::fs::write(config.path(directory), &config.yaml).map_err(|error| error.to_string())?;
    }
    Ok(config)
}

impl SwiftSubApp {
    pub(super) fn checkpoint_rule_visibility(&self) -> Result<(), String> {
        let json = serde_json::to_string(&self.workspace.rule_visibility)
            .map_err(|error| error.to_string())?;
        let store = self.store.lock().map_err(|error| error.to_string())?;
        let key = format!("rule-visibility:{}", self.profile_id);
        if store
            .get_setting(&key)
            .map_err(|error| error.to_string())?
            .as_deref()
            != Some(&json)
        {
            store
                .set_setting(&key, &json)
                .map_err(|error| error.to_string())?;
        }
        Ok(())
    }

    pub(super) fn restore_rule_visibility(&mut self) {
        let result = self
            .store
            .lock()
            .map_err(|error| error.to_string())
            .and_then(|store| {
                store
                    .get_setting(&format!("rule-visibility:{}", self.profile_id))
                    .map_err(|error| error.to_string())
            });
        match result {
            Ok(Some(json)) if !json.is_empty() => match serde_json::from_str(&json) {
                Ok(visibility) => self.workspace.rule_visibility = visibility,
                Err(error) => self.store_error = Some(format!("规则状态读取失败：{error}")),
            },
            Ok(_) => self.workspace.rule_visibility = Default::default(),
            Err(error) => self.store_error = Some(error),
        }
    }

    pub(crate) fn workspace_owned_rules(
        &self,
        target: &str,
    ) -> Vec<workspace::rules::WorkspaceRuleChoice> {
        self.workspace_rule_choices()
            .into_iter()
            .filter(|choice| {
                let binding = &choice.binding;
                binding.target == target
                    && (!self.template_disabled_rule_sets.contains(&binding.id)
                        || self
                            .workspace
                            .rule_visibility
                            .disabled_sets
                            .contains(&binding.id))
                    && (self
                        .template_custom_rule_sets
                        .iter()
                        .any(|item| item.id == binding.id)
                        || self.template_rule_set_targets.contains_key(&binding.id)
                        || catalog::MODULES.iter().any(|module| {
                            module.name == self.workspace_builtin_name(target)
                                && self.template_enabled_modules.contains(module.id)
                                && module.rule_sets.iter().any(|spec| spec.id == binding.id)
                        }))
            })
            .collect()
    }

    pub fn toggle_workspace_rule(
        &mut self,
        id: String,
        enabled: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let previous = self.template_disabled_rule_sets.clone();
        let previous_visibility = self.workspace.rule_visibility.clone();
        if enabled {
            self.template_disabled_rule_sets.remove(&id);
            self.workspace.rule_visibility.disabled_sets.remove(&id);
        } else {
            self.template_disabled_rule_sets.insert(id.clone());
            self.workspace.rule_visibility.disabled_sets.insert(id);
        }
        let result = self.draft_template_snapshot(cx).and_then(|snapshot| {
            self.persist_and_commit_template(snapshot.clone(), None, window, cx)?;
            self.sync_workspace_snapshot(snapshot, window, cx);
            Ok(())
        });
        if let Err(error) = result {
            self.template_disabled_rule_sets = previous;
            self.workspace.rule_visibility = previous_visibility;
            self.template_error = Some(error);
        }
        cx.notify();
    }

    pub fn remove_workspace_rule(
        &mut self,
        id: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.finish_workspace_fields(window, cx) {
            return;
        }
        let previous = self.workspace.rule_visibility.clone();
        self.workspace.rule_visibility.disabled_sets.remove(&id);
        let result = self.draft_template_snapshot(cx).and_then(|mut snapshot| {
            if !snapshot.disabled_rule_sets.contains(&id) {
                snapshot.disabled_rule_sets.push(id.clone());
            }
            self.persist_and_commit_template(snapshot.clone(), None, window, cx)?;
            self.sync_workspace_snapshot(snapshot, window, cx);
            if self.workspace.rule_content_id.as_deref() == Some(&id) {
                self.workspace_close_rule_content(cx);
            }
            Ok(())
        });
        if let Err(error) = result {
            self.workspace.rule_visibility = previous;
            self.template_error = Some(error);
        }
        cx.notify();
    }

    pub fn toggle_workspace_manual(
        &mut self,
        line: String,
        enabled: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let original = self.template_custom_rules.read(cx).value().to_string();
        let previous = self.workspace.rule_visibility.clone();
        let mut lines: Vec<String> = original
            .lines()
            .filter(|raw| !Rule::parse(raw).is_ok_and(|rule| rule.to_line() == line))
            .map(str::to_string)
            .collect();
        self.workspace
            .rule_visibility
            .disabled_manual
            .retain(|raw| raw != &line);
        if enabled {
            lines.push(line);
        } else {
            self.workspace.rule_visibility.disabled_manual.push(line);
        }
        self.template_custom_rules.update(cx, |input, cx| {
            input.set_value(lines.join("\n"), window, cx)
        });
        let result = self.draft_template_snapshot(cx).and_then(|snapshot| {
            self.persist_and_commit_template(snapshot.clone(), None, window, cx)?;
            self.sync_workspace_snapshot(snapshot, window, cx);
            Ok(())
        });
        if let Err(error) = result {
            self.workspace.rule_visibility = previous;
            self.template_custom_rules
                .update(cx, |input, cx| input.set_value(original, window, cx));
            self.template_error = Some(error);
        }
        cx.notify();
    }

    pub fn remove_workspace_manual(
        &mut self,
        line: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.finish_workspace_fields(window, cx) {
            return;
        }
        let original = self.template_custom_rules.read(cx).value().to_string();
        let previous = self.workspace.rule_visibility.clone();
        let lines = original
            .lines()
            .filter(|raw| !Rule::parse(raw).is_ok_and(|rule| rule.to_line() == line))
            .collect::<Vec<_>>()
            .join("\n");
        self.template_custom_rules
            .update(cx, |input, cx| input.set_value(lines, window, cx));
        self.workspace
            .rule_visibility
            .disabled_manual
            .retain(|raw| raw != &line);
        let result = self.draft_template_snapshot(cx).and_then(|snapshot| {
            self.persist_and_commit_template(snapshot.clone(), None, window, cx)?;
            self.sync_workspace_snapshot(snapshot, window, cx);
            Ok(())
        });
        if let Err(error) = result {
            self.workspace.rule_visibility = previous;
            self.template_custom_rules
                .update(cx, |input, cx| input.set_value(original, window, cx));
            self.template_error = Some(error);
        }
        cx.notify();
    }

    pub fn edit_workspace_manual(
        &mut self,
        line: String,
        index: Option<usize>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Ok(rule) = Rule::parse(&line) else {
            return;
        };
        self.workspace.rules_target = Some(rule.target.clone());
        self.workspace.manual_edit = index;
        self.workspace.manual_disabled_edit = index.is_none().then_some(line);
        self.workspace.rule_picker_kind = "手写规则".into();
        self.workspace.rule_picker_open = true;
        self.workspace
            .manual_rule
            .update(cx, |input, cx| input.set_value(rule.to_body(), window, cx));
        cx.notify();
    }

    pub(super) fn save_disabled_manual(
        &mut self,
        original: String,
        target: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let previous = self.workspace.rule_visibility.clone();
        let result = (|| {
            let text = self
                .workspace
                .manual_rule
                .read(cx)
                .value()
                .trim()
                .to_string();
            let mut rule = Rule::parse_body(&text).map_err(|error| error.to_string())?;
            rule.target = target;
            let Some(index) = self
                .workspace
                .rule_visibility
                .disabled_manual
                .iter()
                .position(|line| line == &original)
            else {
                return Err("这条规则已被移除".into());
            };
            self.workspace.rule_visibility.disabled_manual[index] = rule.to_line();
            self.checkpoint_rule_visibility()?;
            self.workspace.manual_disabled_edit = None;
            self.workspace.rule_picker_open = false;
            self.workspace
                .manual_rule
                .update(cx, |input, cx| input.set_value("", window, cx));
            Ok::<_, String>(())
        })();
        if let Err(error) = result {
            self.workspace.rule_visibility = previous;
            self.template_error = Some(error);
        }
        cx.notify();
    }

    pub(super) fn save_workspace_chain(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let previous_visibility = self.workspace.rule_visibility.clone();
        let previous_order = self.node_order.clone();
        let result = (|| {
            if !self.finish_workspace_fields(window, cx) {
                return Err(self
                    .template_error
                    .clone()
                    .unwrap_or("请完成策略组编辑".into()));
            }
            let entry = self
                .nodes
                .iter()
                .find(|node| {
                    node.enabled && Some(&node.source_key) == self.chain_entry_key.as_ref()
                })
                .cloned()
                .ok_or("请选择入口节点")?;
            let exit = self
                .nodes
                .iter()
                .find(|node| {
                    node.enabled && Some(&node.source_key) == self.chain_landing_key.as_ref()
                })
                .cloned()
                .ok_or("请选择出口节点")?;
            if entry.source_key == exit.source_key {
                return Err("入口和出口不能是同一个节点".into());
            }
            let editing = self.workspace.edit_chain;
            let previous = editing.and_then(|index| self.chains.get(index)).cloned();
            let typed = self.chain_name.read(cx).value().trim().to_string();
            let default_name = format!("{} | {}", entry.name(), exit.name());
            let automatic = typed.is_empty() || typed == default_name;
            let mut name = if typed.is_empty() {
                default_name
            } else {
                typed
            };
            let base_name = name.clone();
            let occupied = |name: &str| {
                self.nodes.iter().any(|node| node.name() == name)
                    || (self
                        .template_target_options()
                        .iter()
                        .any(|target| target == name)
                        && !previous.as_ref().is_some_and(|chain| chain.name == name))
            };
            let mut suffix = 2;
            while occupied(&name) {
                if !automatic {
                    return Err("名称已被其他节点或策略组使用".into());
                }
                name = format!("{base_name} {suffix}");
                suffix += 1;
            }
            let mut snapshot = self.draft_template_snapshot(cx)?;
            let mut chain = ChainSpec::new(
                name.clone(),
                entry.source_key.clone(),
                entry.name(),
                exit.source_key.clone(),
                exit.name(),
            );
            if let Some(previous) = previous {
                chain.disabled = previous.disabled;
                chain.hops = previous.hops;
                if !chain.hops.is_empty() {
                    chain.hops[0] = entry.source_key;
                    let last = chain.hops.len() - 1;
                    chain.hops[last] = exit.source_key;
                }
                if previous.name != name {
                    let old_key = chain_order_key(&previous.name);
                    let new_key = chain_order_key(&name);
                    for key in &mut self.node_order {
                        if key == &old_key {
                            *key = new_key.clone();
                        }
                    }
                    retarget_snapshot(&mut snapshot, &previous.name, &name);
                    workspace::retarget_rule_lines(
                        &mut self.workspace.rule_visibility.disabled_manual,
                        &previous.name,
                        &name,
                    );
                }
                if let Some(index) = editing.filter(|&index| index < snapshot.chains.len()) {
                    snapshot.chains[index] = chain;
                } else {
                    snapshot.chains.push(chain);
                }
            } else {
                snapshot.chains.push(chain);
            }
            self.persist_and_commit_template(snapshot.clone(), None, window, cx)?;
            self.template_baseline = Some(snapshot.clone());
            self.load_template_draft(snapshot, String::new(), self.template_edit_id, window, cx);
            self.reset_chain_editor(window, cx);
            self.template_tab = TemplateTab::Tools;
            self.custom_dialog = CustomDialog::None;
            if let Err(error) = self.checkpoint_draft(cx) {
                self.store_error = Some(error);
            }
            Ok::<_, String>(())
        })();
        match result {
            Ok(()) => self.show_toast("链式节点已保存".into(), cx),
            Err(error) => {
                self.workspace.rule_visibility = previous_visibility;
                self.node_order = previous_order;
                self.custom_error = Some(error);
                cx.notify();
            }
        }
    }

    fn generation_draft(&self, cx: &Context<Self>) -> Result<GenerationDraft, String> {
        let source_json = self.profile_snapshot_json(self.active_template_snapshot())?;
        let store = self.store.lock().map_err(|error| error.to_string())?;
        let workspace_json = store
            .get_setting(&format!("draft:{}", self.profile_id))
            .map_err(|error| error.to_string())?;
        let rule_content_json = store
            .get_setting(&format!("rule-content-drafts:{}", self.profile_id))
            .map_err(|error| error.to_string())?;
        drop(store);
        let yaml = self.projected_profile(cx).ok().map(|profile| profile.yaml);
        let nodes = self
            .nodes
            .iter()
            .filter_map(|node| {
                node.source_id
                    .parse::<i64>()
                    .ok()
                    .map(|source_id| (source_id, node))
            })
            .map(|(source_id, node)| {
                Ok(DraftNode {
                    source_id,
                    source_key: node.source_key.clone(),
                    yaml: node
                        .proxy
                        .to_yaml_text()
                        .map_err(|error| error.to_string())?,
                    enabled: node.enabled,
                    listen_port: node.listen_port,
                })
            })
            .collect::<Result<Vec<_>, String>>()?;
        Ok(GenerationDraft {
            source_json,
            workspace_json,
            rule_content_json,
            yaml,
            deleted_nodes: self.deleted_nodes.clone(),
            node_order: self.node_order.clone(),
            rule_visibility: self.workspace.rule_visibility.clone(),
            nodes,
        })
    }

    pub fn save_generation_draft(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.workspace.saved_editor_open {
            match self.autosave_saved_editor(window, cx) {
                Ok(()) => self.show_toast("已保存，当前配置的本地订阅已更新".into(), cx),
                Err(error) => {
                    self.saved_config_error = Some(error);
                    cx.notify();
                }
            }
            return;
        }
        let result = (|| {
            if self.projected_profile(cx).is_ok() {
                self.autosave_routing(window, cx)?;
                self.workspace_apply_rule_drafts(window, cx)?;
            }
            self.checkpoint_draft(cx)?;
            let draft = self.generation_draft(cx)?;
            let name = self
                .generated_saved_config
                .as_ref()
                .map(|config| config.name.clone())
                .unwrap_or_else(|| format!("配置 {}", self.saved_configs.len() + 1));
            let config = save_temporary_config(
                &*self.store.lock().map_err(|error| error.to_string())?,
                &self.data_dir.join("configs"),
                self.generated_saved_config.as_ref(),
                &name,
                &draft,
            )?;
            self.remember_generated_config(&config)?;
            Ok::<_, String>(config)
        })();
        match result {
            Ok(config) => {
                let published = self.projected_profile(cx).is_ok();
                self.generated_saved_config = Some(config.clone());
                self.saved_configs
                    .retain(|existing| existing.id != config.id);
                self.saved_configs.insert(0, config);
                self.refresh_config_summaries();
                self.generated_save_error = None;
                self.reset_generated_name(true, window, cx);
                self.publish_profiles();
                self.show_toast(
                    if published {
                        "已保存，本地订阅已更新"
                    } else {
                        "草稿已保存（存在错误，订阅保留上次有效配置）"
                    }
                    .into(),
                    cx,
                );
            }
            Err(error) => {
                self.generated_save_error = Some(error);
                cx.notify();
            }
        }
    }

    pub(super) fn remember_generated_config(
        &self,
        config: &swiftsub_store::SavedConfig,
    ) -> Result<(), String> {
        let json = saved_configs::association_json(config)?;
        self.store
            .lock()
            .map_err(|error| error.to_string())?
            .set_setting(&format!("generated-target:{}", self.profile_id), &json)
            .map_err(|error| error.to_string())
    }

    pub(super) fn checkpoint_config_generation(
        &self,
        config: &swiftsub_store::SavedConfig,
        cx: &Context<Self>,
    ) -> Result<(), String> {
        if self.workspace.saved_editor_open {
            return Ok(());
        }
        self.checkpoint_draft(cx)?;
        let json = serde_json::to_string(&self.generation_draft(cx)?)
            .map_err(|error| error.to_string())?;
        self.store
            .lock()
            .map_err(|error| error.to_string())?
            .set_setting(&format!("generation-draft:{}", config.id), &json)
            .map_err(|error| error.to_string())
    }

    pub fn edit_library_config(&mut self, id: String, window: &mut Window, cx: &mut Context<Self>) {
        let Some(config) = self
            .saved_configs
            .iter()
            .find(|config| config.id == id)
            .cloned()
        else {
            return;
        };
        let saved_draft = self
            .store
            .lock()
            .map_err(|error| error.to_string())
            .and_then(|store| {
                store
                    .get_setting(&format!("generation-draft:{id}"))
                    .map_err(|error| error.to_string())
            });
        let saved_draft = match saved_draft {
            Ok(value) => value,
            Err(error) => {
                self.saved_config_error = Some(error);
                cx.notify();
                return;
            }
        };
        let draft = saved_draft
            .filter(|json| !json.is_empty())
            .map(|json| serde_json::from_str::<GenerationDraft>(&json));
        let draft = match draft.transpose() {
            Ok(draft) => draft,
            Err(error) => {
                self.saved_config_error = Some(format!("临时配置读取失败：{error}"));
                cx.notify();
                return;
            }
        };
        let source = draft
            .as_ref()
            .map(|draft| draft.source_json.as_str())
            .or(config.source_json.as_deref());
        let recipe =
            source.and_then(|json| serde_json::from_str::<profiles::ProfileConfig>(json).ok());
        let Some(recipe) = recipe else {
            if let Some(previous) = &self.generated_saved_config
                && let Err(error) = self.checkpoint_config_generation(previous, cx)
            {
                self.saved_config_error = Some(error);
                cx.notify();
                return;
            }
            self.open_saved_config(id, window, cx);
            if self.saved_config_id.as_deref() != Some(&config.id) {
                return;
            }
            self.workspace.saved_editor_open = true;
            self.saved_editor.list_collapsed = true;
            self.saved_editor.more = false;
            self.saved_editor.tab = SavedEditorTab::Groups;
            self.page = Page::Generator;
            cx.notify();
            return;
        };
        if self
            .generated_saved_config
            .as_ref()
            .is_some_and(|target| target.id == id)
            && !self.workspace.saved_editor_open
        {
            self.set_page(Page::Generator, window, cx);
            return;
        }
        if let Some(previous) = &self.generated_saved_config
            && let Err(error) = self.checkpoint_config_generation(previous, cx)
        {
            self.saved_config_error = Some(error);
            cx.notify();
            return;
        }
        if let Err(error) = self.checkpoint_yaml_editor(cx) {
            self.saved_config_error = Some(error);
            cx.notify();
            return;
        }
        let options = match recipe.routing.generate_options(&self.options) {
            Ok(options) => options,
            Err(error) => {
                self.saved_config_error = Some(error);
                cx.notify();
                return;
            }
        };
        let deleted_nodes = draft
            .as_ref()
            .map(|draft| draft.deleted_nodes.clone())
            .unwrap_or_default();
        let node_order = draft
            .as_ref()
            .map(|draft| draft.node_order.clone())
            .unwrap_or_else(|| recipe.node_order.clone().unwrap_or_default());
        let rule_visibility = draft
            .as_ref()
            .map(|draft| draft.rule_visibility.clone())
            .unwrap_or_default();
        let restore = (|| {
            let mut settings = HashMap::new();
            settings.insert(
                "deleted_nodes".into(),
                serde_json::to_string(&deleted_nodes).map_err(|error| error.to_string())?,
            );
            settings.insert(
                "node_order".into(),
                serde_json::to_string(&node_order).map_err(|error| error.to_string())?,
            );
            settings.insert(
                format!("rule-visibility:{}", self.profile_id),
                serde_json::to_string(&rule_visibility).map_err(|error| error.to_string())?,
            );
            settings.insert(
                format!("draft:{}", self.profile_id),
                draft
                    .as_ref()
                    .and_then(|draft| draft.workspace_json.clone())
                    .unwrap_or_default(),
            );
            settings.insert(
                format!("rule-content-drafts:{}", self.profile_id),
                draft
                    .as_ref()
                    .and_then(|draft| draft.rule_content_json.clone())
                    .unwrap_or_default(),
            );
            settings.insert(
                format!("generated-target:{}", self.profile_id),
                saved_configs::association_json(&config)?,
            );
            let store = self.store.lock().map_err(|error| error.to_string())?;
            let overrides = draft_node_overrides(
                &store,
                draft
                    .as_ref()
                    .map(|draft| draft.nodes.as_slice())
                    .unwrap_or_default(),
                &self.subscriptions,
            )?;
            store
                .save_routing_profile_with_node_overrides(
                    &settings,
                    None,
                    Some((self.profile_id, source.unwrap_or_default())),
                    &overrides,
                )
                .map_err(|error| error.to_string())?;
            Ok::<_, String>(())
        })();
        if let Err(error) = restore {
            self.saved_config_error = Some(error);
            cx.notify();
            return;
        }
        self.options = options;
        self.profile_membership = recipe.membership;
        self.template_baseline = Some(recipe.routing.clone());
        self.load_template_draft(recipe.routing, String::new(), None, window, cx);
        self.deleted_nodes = deleted_nodes;
        self.disabled_nodes.clear();
        self.node_order = node_order;
        self.workspace.rule_visibility = rule_visibility;
        if draft.is_some() {
            self.restore_draft(window, cx);
        }
        self.generated_saved_config = Some(config.clone());
        self.generated_config_name.update(cx, |input, cx| {
            input.set_value(config.name.clone(), window, cx)
        });
        self.generated_save_open = false;
        self.generated_save_error = None;
        self.workspace.saved_editor_open = false;
        self.workspace.edit_chain = None;
        self.template_policy_edit = None;
        self.template_tab = TemplateTab::PolicyGroups;
        self.workspace.group_rules_tab = false;
        self.page = Page::Generator;
        if let Err(error) = self.remember_generated_config(&config) {
            self.generated_save_error = Some(error);
        }
        self.rebuild_nodes(cx);
        cx.notify();
    }

    pub fn import_library_config(
        &mut self,
        id: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.clash_import_busy {
            return;
        }
        self.saved_config_error = None;
        if !self.workspace.saved_editor_open
            && self
                .generated_saved_config
                .as_ref()
                .is_some_and(|config| config.id == id)
        {
            self.import_generated_to_clash(window, cx);
            if let Some(error) = &self.generated_save_error {
                self.saved_config_error = Some(error.clone());
            }
            cx.notify();
            return;
        }
        if self.workspace.saved_editor_open && self.saved_config_id.as_deref() == Some(&id) {
            self.import_saved_to_clash(window, cx);
            cx.notify();
            return;
        }
        let Some(config) = self
            .saved_configs
            .iter()
            .find(|config| config.id == id)
            .cloned()
        else {
            return;
        };
        self.publish_profiles();
        self.begin_clash_import(config, false, window, cx);
        cx.notify();
    }

    pub fn open_node_edit_by_key(
        &mut self,
        key: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(node) = self
            .nodes
            .iter()
            .find(|node| node.source_key == key)
            .cloned()
        else {
            return;
        };
        let yaml = node.proxy.to_yaml_text().unwrap_or_default();
        self.open_node_rename(
            node.source_key.clone(),
            node.source_id.clone(),
            node.name().to_string(),
            yaml,
            window,
            cx,
        );
    }

    pub fn swap_chain_endpoints(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let previous_name = self
            .chain_entry_name
            .as_ref()
            .zip(self.chain_landing_name.as_ref())
            .map(|(entry, exit)| format!("{entry} | {exit}"));
        let current_name = self.chain_name.read(cx).value().trim().to_string();
        std::mem::swap(&mut self.chain_entry_key, &mut self.chain_landing_key);
        std::mem::swap(&mut self.chain_entry_name, &mut self.chain_landing_name);
        if (current_name.is_empty() || previous_name.as_deref() == Some(&current_name))
            && let (Some(entry), Some(exit)) = (&self.chain_entry_name, &self.chain_landing_name)
        {
            let name = format!("{entry} | {exit}");
            self.chain_name
                .update(cx, |input, cx| input.set_value(name, window, cx));
        }
        cx.notify();
    }

    pub fn reset_chain_editor(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.workspace.edit_chain = None;
        self.chain_entry_key = None;
        self.chain_entry_name = None;
        self.chain_landing_key = None;
        self.chain_landing_name = None;
        self.custom_error = None;
        self.chain_name
            .update(cx, |input, cx| input.set_value("", window, cx));
        cx.notify();
    }

    pub fn choose_chain_endpoint(
        &mut self,
        side: ChainSide,
        key: String,
        name: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let previous_name = self
            .chain_entry_name
            .as_ref()
            .zip(self.chain_landing_name.as_ref())
            .map(|(entry, exit)| format!("{entry} | {exit}"));
        let current_name = self.chain_name.read(cx).value().trim().to_string();
        match side {
            ChainSide::Entry => {
                self.chain_entry_key = Some(key);
                self.chain_entry_name = Some(name);
            }
            ChainSide::Landing => {
                self.chain_landing_key = Some(key);
                self.chain_landing_name = Some(name);
            }
        }
        if (current_name.is_empty() || previous_name.as_deref() == Some(&current_name))
            && let (Some(entry), Some(exit)) = (&self.chain_entry_name, &self.chain_landing_name)
        {
            let name = format!("{entry} | {exit}");
            self.chain_name
                .update(cx, |input, cx| input.set_value(name, window, cx));
        }
        cx.notify();
    }

    pub fn toggle_workspace_chain(
        &mut self,
        index: usize,
        enabled: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.finish_workspace_fields(window, cx) {
            return;
        }
        let result = self.draft_template_snapshot(cx).and_then(|mut snapshot| {
            snapshot
                .chains
                .get_mut(index)
                .ok_or("链式节点已删除")?
                .disabled = !enabled;
            self.persist_and_commit_template(snapshot.clone(), None, window, cx)?;
            self.sync_workspace_snapshot(snapshot, window, cx);
            Ok(())
        });
        if let Err(error) = result {
            self.custom_error = Some(error);
        } else {
            self.custom_error = None;
        }
        cx.notify();
    }

    pub fn toggle_workspace_group(
        &mut self,
        name: String,
        enabled: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.finish_workspace_fields(window, cx) {
            return;
        }
        let result = self.draft_template_snapshot(cx).and_then(|mut snapshot| {
            if let Some(index) = snapshot
                .policy_groups
                .iter()
                .position(|group| group.name == name && group.removed)
            {
                let identity = snapshot.policy_groups[index]
                    .builtin_name()
                    .unwrap_or(&name)
                    .to_string();
                let mut restored = self.workspace_builtin_spec(&identity)?;
                restored.name = name.clone();
                snapshot.policy_groups[index] = restored;
            } else if !snapshot
                .policy_groups
                .iter()
                .any(|group| group.name == name)
            {
                snapshot
                    .policy_groups
                    .push(self.workspace_builtin_spec(&name)?);
            }
            let group = snapshot
                .policy_groups
                .iter_mut()
                .find(|group| group.name == name)
                .ok_or("策略组已删除")?;
            group.disabled = !enabled;
            group.removed = false;
            let identity = group.builtin_name().unwrap_or(&name).to_string();
            if enabled
                && let Some(module) = catalog::MODULES
                    .iter()
                    .find(|module| module.name == identity)
                && !snapshot.enabled_modules.iter().any(|id| id == module.id)
            {
                snapshot.enabled_modules.push(module.id.into());
            }
            self.persist_and_commit_template(snapshot.clone(), None, window, cx)?;
            self.sync_workspace_snapshot(snapshot, window, cx);
            Ok(())
        });
        if let Err(error) = result {
            self.template_error = Some(error);
        }
        cx.notify();
    }

    pub fn edit_workspace_group(
        &mut self,
        name: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.workspace_group_settings(name, window, cx);
    }

    pub(super) fn rename_workspace_policy_group(
        &mut self,
        index: usize,
        name: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Result<(), String> {
        let mut snapshot = self.draft_template_snapshot(cx)?;
        let group = snapshot
            .policy_groups
            .get_mut(index)
            .ok_or("策略组已删除")?;
        let old = group.name.clone();
        group.ensure_id();
        group.name = name.clone();
        retarget_snapshot(&mut snapshot, &old, &name);
        let previous_nodes = self.nodes.clone();
        let previous_visibility = self.workspace.rule_visibility.clone();
        workspace::retarget_rule_lines(
            &mut self.workspace.rule_visibility.disabled_manual,
            &old,
            &name,
        );
        let result = (|| {
            let mut changed = Vec::new();
            for node in &mut self.nodes {
                if node.proxy.get_str("dialer-proxy") == Some(&old) {
                    node.proxy.set_dialer_proxy(&name);
                    changed.push(node.clone());
                }
            }
            let overrides = changed
                .iter()
                .map(|node| self.workspace_node_override(node))
                .collect::<Result<Vec<_>, _>>()?;
            self.persist_template_with_node_overrides(
                snapshot.clone(),
                None,
                &overrides,
                window,
                cx,
            )
        })();
        if let Err(error) = result {
            self.nodes = previous_nodes;
            self.workspace.rule_visibility = previous_visibility;
            return Err(error);
        }
        if self.template_rules_target_filter == old {
            self.template_rules_target_filter = name.clone();
        }
        if self.workspace.rules_target.as_deref() == Some(&old) {
            self.workspace.rules_target = Some(name.clone());
        }
        if self.workspace.rule_content_group == old {
            self.workspace.rule_content_group = name.clone();
        }
        if let Some(binding) = &mut self.workspace.rule_content_binding
            && binding.target == old
        {
            binding.target = name.clone();
        }
        if self.workspace.reveal_group.borrow().as_deref() == Some(&old) {
            self.workspace.reveal_group.replace(Some(name.clone()));
        }
        if let Some(line) = &mut self.workspace.manual_disabled_edit {
            workspace::retarget_rule_lines(std::slice::from_mut(line), &old, &name);
        }
        self.template_policy_groups[index].name = name;
        self.sync_workspace_snapshot(snapshot, window, cx);
        Ok(())
    }

    pub fn set_workspace_member(
        &mut self,
        index: usize,
        name: String,
        enabled: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.finish_workspace_fields(window, cx) {
            return;
        }
        let inherited: Vec<_> = self
            .nodes
            .iter()
            .filter(|node| node.enabled)
            .map(|node| node.name().to_string())
            .chain(
                self.chains
                    .iter()
                    .filter(|chain| !chain.disabled)
                    .map(|chain| chain.name.clone()),
            )
            .collect();
        let result = self.draft_template_snapshot(cx).and_then(|mut snapshot| {
            let group = snapshot
                .policy_groups
                .get_mut(index)
                .ok_or("策略组已删除")?;
            if group.include_all_nodes && !enabled {
                group.include_all_nodes = false;
                group.members.extend(inherited);
            }
            group.members.retain(|member| member != &name);
            if enabled {
                group.members.push(name);
            }
            let mut seen = HashSet::new();
            group.members.retain(|name| seen.insert(name.clone()));
            self.persist_and_commit_template(snapshot.clone(), None, window, cx)?;
            self.sync_workspace_snapshot(snapshot, window, cx);
            Ok(())
        });
        if let Err(error) = result {
            self.template_error = Some(error);
        }
        cx.notify();
    }

    pub fn edit_workspace_member(
        &mut self,
        name: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(node) = self.nodes.iter().find(|node| node.name() == name).cloned() {
            self.open_node_rename(
                node.source_key,
                node.source_id,
                name,
                node.proxy.to_yaml_text().unwrap_or_default(),
                window,
                cx,
            );
        } else if let Some(index) = self.chains.iter().position(|chain| chain.name == name) {
            self.workspace_edit_chain(index, window, cx);
        } else if !matches!(name.as_str(), "DIRECT" | "REJECT" | "REJECT-DROP") {
            self.workspace_edit_group(name, window, cx);
        }
    }

    pub fn remove_workspace_node(
        &mut self,
        key: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(node) = self
            .nodes
            .iter()
            .find(|node| node.source_key == key)
            .cloned()
        else {
            return;
        };
        if !self.finish_workspace_fields(window, cx) {
            return;
        }
        let previous_nodes = self.nodes.clone();
        let previous_deleted = self.deleted_nodes.clone();
        let previous_order = self.node_order.clone();
        let previous_visibility = self.workspace.rule_visibility.clone();
        let result = (|| {
            let mut snapshot = self.draft_template_snapshot(cx)?;
            let fallback = if snapshot
                .policy_groups
                .iter()
                .any(|group| group.name == "节点选择" && (group.disabled || group.removed))
            {
                "DIRECT"
            } else {
                "节点选择"
            };
            retarget_snapshot(&mut snapshot, node.name(), fallback);
            workspace::retarget_rule_lines(
                &mut self.workspace.rule_visibility.disabled_manual,
                node.name(),
                fallback,
            );
            for chain in &mut snapshot.chains {
                if chain_uses_node(chain, &node) {
                    chain.disabled = true;
                }
            }
            self.deleted_nodes.insert(key.clone());
            self.node_order.retain(|item| item != &key);
            self.nodes.retain(|item| item.source_key != key);
            let mut overrides = Vec::new();
            for dependent in &mut self.nodes {
                if dependent.proxy.get_str("dialer-proxy") == Some(node.name()) {
                    dependent.proxy.set_dialer_proxy("DIRECT");
                    overrides.push(dependent.clone());
                }
            }
            let overrides = overrides
                .iter()
                .map(|node| self.workspace_node_override(node))
                .collect::<Result<Vec<_>, _>>()?;
            self.persist_template_with_node_overrides(
                snapshot.clone(),
                None,
                &overrides,
                window,
                cx,
            )?;
            self.sync_workspace_snapshot(snapshot, window, cx);
            Ok::<_, String>(())
        })();
        if let Err(error) = result {
            self.nodes = previous_nodes;
            self.deleted_nodes = previous_deleted;
            self.node_order = previous_order;
            self.workspace.rule_visibility = previous_visibility;
            self.template_error = Some(error);
        }
        cx.notify();
    }

    pub(super) fn workspace_node_override(
        &self,
        node: &Node,
    ) -> Result<swiftsub_store::NodeOverride, String> {
        let sub_id = node.source_id.parse().map_err(|_| "该节点不支持编辑")?;
        let store = self.store.lock().map_err(|error| error.to_string())?;
        let existing = store
            .list_node_overrides(sub_id)
            .map_err(|error| error.to_string())?
            .remove(&node.source_key);
        Ok(swiftsub_store::NodeOverride {
            sub_id,
            node_key: node.source_key.clone(),
            alias: Some(node.name().into()),
            enabled: Some(node.enabled),
            listen_port: node.listen_port,
            sort_order: existing.and_then(|value| value.sort_order),
            extras: Some(
                node.proxy
                    .to_yaml_text()
                    .map_err(|error| error.to_string())?,
            ),
        })
    }

    pub(super) fn sync_workspace_snapshot(
        &mut self,
        snapshot: TemplateSnapshot,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let edit_name = self
            .template_policy_edit
            .and_then(|index| self.template_policy_groups.get(index))
            .map(|group| group.name.clone());
        let settings_open = self.workspace.group_settings_open;
        let member_search = self.template_member_search.read(cx).value().to_string();
        let page = self.page;
        self.template_baseline = Some(snapshot.clone());
        let name = self.template_name.read(cx).value().to_string();
        self.load_template_draft(snapshot, name, self.template_edit_id, window, cx);
        self.page = page;
        if let Some(index) = edit_name.and_then(|name| {
            self.template_policy_groups
                .iter()
                .position(|group| group.name == name)
        }) {
            self.edit_template_policy_group(index, window, cx);
            self.workspace.group_settings_open = settings_open;
            self.template_member_search
                .update(cx, |input, cx| input.set_value(member_search, window, cx));
        }
        if let Err(error) = self.checkpoint_draft(cx) {
            self.store_error = Some(error);
        }
    }

    pub(super) fn workspace_builtin_spec(&self, name: &str) -> Result<PolicyGroupSpec, String> {
        let options = workspace::builtin_group_options(
            self.template_base_options.as_ref().unwrap_or(&self.options),
            &self.template_enabled_modules,
            name,
        );
        let mut nodes = self.nodes.clone();
        if !nodes.iter().any(|node| node.enabled) {
            nodes.extend(build_nodes(
                vec![Proxy::new("direct", "模板校验节点", "127.0.0.1", 1)],
                "validation",
                "validation",
            ));
        }
        let generated = generate_profile(&nodes, &options).map_err(|error| error.to_string())?;
        let group = generated
            .groups
            .into_iter()
            .find(|group| group.name == name)
            .ok_or("没有匹配的策略组")?;
        let all_nodes = matches!(name, "节点选择" | "自动选择");
        let members = group
            .proxies
            .into_iter()
            .filter(|member| !all_nodes || !nodes.iter().any(|node| node.name() == member))
            .map(|member| self.workspace_resolved_group_name(&member).to_string())
            .collect();
        Ok(PolicyGroupSpec {
            id: format!("builtin:{name}"),
            name: name.into(),
            kind: group.kind,
            members,
            include_all_nodes: all_nodes,
            builtin: true,
            ..Default::default()
        })
    }

    pub fn toggle_workspace_filter(
        &mut self,
        index: usize,
        enabled: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.finish_workspace_fields(window, cx) {
            return;
        }
        let result = self.draft_template_snapshot(cx).and_then(|mut snapshot| {
            snapshot
                .custom_groups
                .get_mut(index)
                .ok_or("策略组已删除")?
                .disabled = !enabled;
            self.persist_and_commit_template(snapshot.clone(), None, window, cx)?;
            self.sync_workspace_snapshot(snapshot, window, cx);
            Ok(())
        });
        if let Err(error) = result {
            self.template_error = Some(error);
        }
        cx.notify();
    }

    pub(super) fn save_workspace_filter(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.finish_workspace_fields(window, cx) {
            return;
        }
        let previous_nodes = self.nodes.clone();
        let previous_visibility = self.workspace.rule_visibility.clone();
        let result = (|| {
            let mut snapshot = self.draft_template_snapshot(cx)?;
            let editing = self.workspace.edit_filter;
            let previous = editing
                .and_then(|index| snapshot.custom_groups.get(index))
                .cloned();
            if editing.is_some() && previous.is_none() {
                return Err("策略组已删除".into());
            }
            let mut regions: Vec<_> = self.filter_regions.iter().copied().collect();
            regions.sort();
            let filter = NodeFilter {
                regions,
                sources: split_keywords(&self.workspace.filter_sources.read(cx).value()),
                include_keywords: split_keywords(&self.filter_include.read(cx).value()),
                exclude_keywords: split_keywords(&self.filter_exclude.read(cx).value()),
            };
            if filter.regions.is_empty()
                && filter.sources.is_empty()
                && filter.include_keywords.is_empty()
                && filter.exclude_keywords.is_empty()
            {
                return Err("请选择地区或填写筛选条件".into());
            }
            let typed = self.filter_name.read(cx).value().trim().to_string();
            let automatic = typed.is_empty();
            let mut name = if automatic {
                previous
                    .as_ref()
                    .map(|group| group.name.clone())
                    .unwrap_or_else(|| format!("筛选组 {}", snapshot.custom_groups.len() + 1))
            } else {
                typed
            };
            let base_name = name.clone();
            let mut suffix = 2;
            while (self.template_target_options().contains(&name)
                && !previous.as_ref().is_some_and(|group| group.name == name))
                || self.nodes.iter().any(|node| node.name() == name)
            {
                if !automatic {
                    return Err("名称已被其他节点或策略组使用".into());
                }
                name = format!("{base_name} {suffix}");
                suffix += 1;
            }
            let group = CustomGroupSpec {
                name: name.clone(),
                disabled: previous.as_ref().is_some_and(|group| group.disabled),
                kind: previous
                    .as_ref()
                    .map(|group| group.kind)
                    .unwrap_or(GroupKind::Select),
                filter,
            };
            let mut changed_nodes = Vec::new();
            if let Some(previous) = previous {
                retarget_snapshot(&mut snapshot, &previous.name, &name);
                workspace::retarget_rule_lines(
                    &mut self.workspace.rule_visibility.disabled_manual,
                    &previous.name,
                    &name,
                );
                for node in &mut self.nodes {
                    if node.proxy.get_str("dialer-proxy") == Some(&previous.name) {
                        node.proxy.set_dialer_proxy(&name);
                        changed_nodes.push(node.clone());
                    }
                }
                if let Some(index) = editing.filter(|&index| index < snapshot.custom_groups.len()) {
                    snapshot.custom_groups[index] = group;
                } else {
                    snapshot.custom_groups.push(group);
                }
            } else {
                snapshot.custom_groups.push(group);
            }
            let overrides = changed_nodes
                .iter()
                .map(|node| self.workspace_node_override(node))
                .collect::<Result<Vec<_>, _>>()?;
            self.persist_template_with_node_overrides(
                snapshot.clone(),
                None,
                &overrides,
                window,
                cx,
            )?;
            self.sync_workspace_snapshot(snapshot, window, cx);
            self.close_custom_dialog(window, cx);
            self.workspace.new_group_open = false;
            Ok::<_, String>(())
        })();
        match result {
            Ok(()) => self.show_toast("策略组已保存".into(), cx),
            Err(error) => {
                self.nodes = previous_nodes;
                self.workspace.rule_visibility = previous_visibility;
                self.custom_error = Some(error);
                cx.notify();
            }
        }
    }

    pub(super) fn save_workspace_node_edit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(key) = self.node_rename_key.clone() else {
            return;
        };
        if !self.finish_workspace_fields(window, cx) {
            return;
        }
        let previous_nodes = self.nodes.clone();
        let previous_visibility = self.workspace.rule_visibility.clone();
        let result = (|| {
            let index = self
                .nodes
                .iter()
                .position(|node| node.source_key == key)
                .ok_or("节点已被移除")?;
            let old = self.nodes[index].clone();
            let name = self.node_rename_input.read(cx).value().trim().to_string();
            if name.is_empty() || name.contains([',', '\n', '\r']) {
                return Err("请输入有效的节点名称".into());
            }
            if name != old.name()
                && (self
                    .nodes
                    .iter()
                    .any(|node| node.source_key != key && node.name() == name)
                    || self.template_target_options().contains(&name))
            {
                return Err("名称已被其他节点或策略组使用".into());
            }
            let mut proxy = Proxy::from_yaml_text(&self.node_edit_textarea.read(cx).value())
                .map_err(|error| format!("节点 YAML 无效：{error}"))?;
            if let Some(error) = proxy.fatal_issue() {
                return Err(error);
            }
            proxy.set_name(name.clone());
            let mut snapshot = self.draft_template_snapshot(cx)?;
            retarget_snapshot(&mut snapshot, old.name(), &name);
            workspace::retarget_rule_lines(
                &mut self.workspace.rule_visibility.disabled_manual,
                old.name(),
                &name,
            );
            for chain in &mut snapshot.chains {
                rename_chain_node(chain, &old, &name);
            }
            self.nodes[index].proxy = proxy;
            self.nodes[index].region = detect_region(&name);
            let mut edited = vec![self.nodes[index].clone()];
            for dependent in &mut self.nodes {
                if dependent.source_key != key
                    && dependent.proxy.get_str("dialer-proxy") == Some(old.name())
                {
                    dependent.proxy.set_dialer_proxy(&name);
                    edited.push(dependent.clone());
                }
            }
            let overrides = edited
                .iter()
                .map(|node| self.workspace_node_override(node))
                .collect::<Result<Vec<_>, _>>()?;
            self.persist_template_with_node_overrides(
                snapshot.clone(),
                None,
                &overrides,
                window,
                cx,
            )?;
            self.sync_workspace_snapshot(snapshot, window, cx);
            Ok::<_, String>(())
        })();
        match result {
            Ok(()) => {
                self.close_node_rename(window, cx);
                self.show_toast("节点已保存".into(), cx);
            }
            Err(error) => {
                self.nodes = previous_nodes;
                self.workspace.rule_visibility = previous_visibility;
                self.node_rename_error = Some(error);
                cx.notify();
            }
        }
    }

    pub(super) fn toggle_workspace_node(
        &mut self,
        key: String,
        enabled: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(index) = self.nodes.iter().position(|node| node.source_key == key) else {
            return;
        };
        if !self.finish_workspace_fields(window, cx) {
            return;
        }
        let previous = self.nodes[index].enabled;
        let previous_disabled = self.disabled_nodes.clone();
        let result = (|| {
            let mut snapshot = self.draft_template_snapshot(cx)?;
            if !enabled {
                for chain in &mut snapshot.chains {
                    if chain_uses_node(chain, &self.nodes[index]) {
                        chain.disabled = true;
                    }
                }
            }
            self.nodes[index].enabled = enabled;
            if enabled {
                self.disabled_nodes.remove(&key);
            } else {
                self.disabled_nodes.insert(key);
            }
            let value = self.workspace_node_override(&self.nodes[index])?;
            self.persist_template_with_node_overrides(
                snapshot.clone(),
                None,
                &[value],
                window,
                cx,
            )?;
            self.sync_workspace_snapshot(snapshot, window, cx);
            Ok::<_, String>(())
        })();
        if let Err(error) = result {
            self.nodes[index].enabled = previous;
            self.disabled_nodes = previous_disabled;
            self.template_error = Some(error);
        }
        cx.notify();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renaming_and_editing_a_chain_endpoint_keeps_stable_keys_and_repairs_references() {
        let mut nodes = build_nodes(
            vec![
                Proxy::new("direct", "Entry", "entry.example.org", 1),
                Proxy::new("direct", "Exit", "exit.example.org", 2),
            ],
            "fixture",
            "fixture",
        );
        let old = nodes[0].clone();
        let mut chain = ChainSpec::new(
            "Chain",
            old.source_key.clone(),
            old.name(),
            nodes[1].source_key.clone(),
            nodes[1].name(),
        );
        chain.hops = vec![old.name().into(), nodes[1].source_key.clone()];
        let mut snapshot: TemplateSnapshot = serde_json::from_value(serde_json::json!({ "preset":"minimal", "enabled_modules":[], "test_url":"https://example.org", "test_interval":300, "mixed_port":7890, "allow_lan":false })).unwrap();
        snapshot.policy_groups.push(PolicyGroupSpec {
            name: "Explicit".into(),
            members: vec![old.name().into(), "Chain".into()],
            ..Default::default()
        });
        snapshot
            .custom_rule_lines
            .push("DOMAIN,example.org,Entry".into());
        snapshot.rule_order.push("DOMAIN,example.org,Entry".into());
        snapshot.chains.push(chain);
        retarget_snapshot(&mut snapshot, old.name(), "Edited");
        for chain in &mut snapshot.chains {
            rename_chain_node(chain, &old, "Edited");
        }
        nodes[0].set_name("Edited");
        nodes[0].proxy.insert("server", "edited.example.org");
        assert_ne!(nodes[0].proxy.identity_key(), old.proxy.identity_key());
        assert_eq!(nodes[0].source_key, old.source_key);
        assert_eq!(snapshot.chains[0].hops[0], old.source_key);
        assert_eq!(snapshot.rule_order[0], "DOMAIN,example.org,Edited");
        let generated = generate_profile(
            &nodes,
            &snapshot
                .generate_options(&GenerateOptions::default())
                .unwrap(),
        )
        .unwrap();
        assert!(
            generated
                .groups
                .iter()
                .find(|group| group.name == "Explicit")
                .unwrap()
                .proxies
                .iter()
                .any(|member| member == "Edited")
        );
        assert!(generated.yaml.contains("dialer-proxy: Edited"));
        saved_configs::validate_yaml(&generated.yaml).unwrap();
    }

    #[test]
    fn configuration_drafts_restore_their_own_node_edits_and_switches() {
        let store = Store::open_in_memory().unwrap();
        let source_id = store
            .add_subscription(&NewSubscription::url("fixture", "https://example.org/sub"))
            .unwrap();
        store
            .set_node_override_alias(source_id, "stable", "Other configuration")
            .unwrap();
        let node = DraftNode {
            source_id,
            source_key: "stable".into(),
            yaml: Proxy::new("direct", "Saved name", "saved.example.org", 8443)
                .to_yaml_text()
                .unwrap(),
            enabled: false,
            listen_port: Some(12000),
        };
        let json = serde_json::to_string(&vec![node]).unwrap();
        let nodes: Vec<DraftNode> = serde_json::from_str(&json).unwrap();
        let overrides =
            draft_node_overrides(&store, &nodes, &store.list_subscriptions().unwrap()).unwrap();
        store
            .save_routing_profile_with_node_overrides(&HashMap::new(), None, None, &overrides)
            .unwrap();
        let restored = store
            .list_node_overrides(source_id)
            .unwrap()
            .remove("stable")
            .unwrap();
        assert_eq!(restored.alias.as_deref(), Some("Saved name"));
        assert_eq!(restored.enabled, Some(false));
        assert_eq!(restored.listen_port, Some(12000));
        assert_eq!(
            Proxy::from_yaml_text(restored.extras.as_deref().unwrap())
                .unwrap()
                .server(),
            "saved.example.org"
        );
        assert!(
            draft_node_overrides(&store, &nodes, &[])
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn temporary_save_updates_its_own_file_and_retains_incomplete_draft() {
        let store = Store::open_in_memory().unwrap();
        let directory =
            std::env::temp_dir().join(format!("swiftsub-temporary-save-{}", uuid::Uuid::new_v4()));
        let mut draft = GenerationDraft {
            source_json: "{}".into(),
            workspace_json: Some("unfinished input".into()),
            rule_content_json: None,
            yaml: None,
            deleted_nodes: HashSet::new(),
            node_order: Vec::new(),
            rule_visibility: Default::default(),
            nodes: Vec::new(),
        };
        let first = save_temporary_config(&store, &directory, None, "配置 1", &draft).unwrap();
        assert!(first.path(&directory).exists());
        saved_configs::validate_yaml(&first.yaml).unwrap();
        draft.workspace_json = Some("still unfinished".into());
        draft.yaml = Some("mixed-port: 7891\nproxies: []\nrules: ['MATCH,DIRECT']\n".into());
        let second =
            save_temporary_config(&store, &directory, Some(&first), "配置 1", &draft).unwrap();
        assert_eq!(first.id, second.id);
        assert_eq!(store.list_saved_configs().unwrap().len(), 1);
        assert_eq!(
            std::fs::read_to_string(first.path(&directory)).unwrap(),
            second.yaml
        );
        assert_ne!(first.yaml, second.yaml);
        draft.yaml = None;
        let third =
            save_temporary_config(&store, &directory, Some(&second), "配置 1", &draft).unwrap();
        assert_eq!(third.yaml, second.yaml);
        assert_eq!(
            std::fs::read_to_string(third.path(&directory)).unwrap(),
            second.yaml
        );
        let saved: GenerationDraft = serde_json::from_str(
            &store
                .get_setting(&format!("generation-draft:{}", first.id))
                .unwrap()
                .unwrap(),
        )
        .unwrap();
        assert_eq!(saved.workspace_json.as_deref(), Some("still unfinished"));
        std::fs::remove_dir_all(directory).unwrap();
    }
}
