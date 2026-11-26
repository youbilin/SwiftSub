use super::*;
use gpui_kit::Focusable;

pub(super) struct RuleBodyChange {
    pub binding: RuleSetBinding,
    pub key: String,
    pub payload: Vec<String>,
}

pub(super) struct RuleBodyError {
    pub binding: RuleSetBinding,
    pub text: String,
    pub message: String,
}

// Only bindings still participating in this output can consume a matching source draft.
pub(super) fn collect_rule_body_changes(
    choices: Vec<rules::WorkspaceRuleChoice>,
    drafts: &HashMap<String, String>,
) -> Result<Vec<RuleBodyChange>, Box<RuleBodyError>> {
    let mut changes = Vec::new();
    for choice in choices.into_iter().filter(|choice| choice.active) {
        let binding = choice.binding;
        let key = SwiftSubApp::workspace_rule_cache_key(&binding);
        let Some(text) = drafts.get(&key) else {
            continue;
        };
        let payload = swiftsub_core::generator::parse_rule_set_payload(text, &binding.behavior)
            .map_err(|error| {
                Box::new(RuleBodyError {
                    message: format!("规则集「{}」：{error}", binding.name),
                    binding: binding.clone(),
                    text: text.clone(),
                })
            })?;
        changes.push(RuleBodyChange {
            binding,
            key,
            payload,
        });
    }
    Ok(changes)
}

impl SwiftSubApp {
    pub(crate) fn workspace_projected_options(
        &self,
        cx: &Context<Self>,
    ) -> Result<GenerateOptions, String> {
        let mut options = self.draft_generate_options(cx)?;
        let mut drafts = self.workspace_read_rule_content_drafts()?;
        let choices = self.workspace_rule_choices();
        if self.workspace.rule_content_id.is_some()
            && self.workspace.rule_content_editing
            && !self.workspace.rule_content_loading
            && let Some(binding) = &self.workspace.rule_content_binding
            && choices
                .iter()
                .any(|choice| choice.active && choice.binding == *binding)
        {
            let text = self.workspace.rule_content.read(cx).value().to_string();
            if text != self.workspace.rule_content_original {
                drafts.insert(Self::workspace_rule_cache_key(binding), text);
            } else {
                drafts.remove(&Self::workspace_rule_cache_key(binding));
            }
        }
        for change in collect_rule_body_changes(choices, &drafts).map_err(|error| error.message)? {
            options
                .rule_set_overrides
                .insert(change.binding.id, change.payload);
        }
        Ok(options)
    }

    pub(crate) fn workspace_apply_rule_drafts(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Result<(), String> {
        self.workspace_checkpoint_rule_content(cx)?;
        let mut drafts = self.workspace_read_rule_content_drafts()?;
        let changes = match collect_rule_body_changes(self.workspace_rule_choices(), &drafts) {
            Ok(changes) => changes,
            Err(error) => {
                self.workspace_rules_for(error.binding.target.clone(), window, cx);
                if let Some(module) = catalog::MODULES
                    .iter()
                    .find(|m| m.name == error.binding.target)
                {
                    self.workspace
                        .categories_closed
                        .remove(workspace_category(module.id));
                } else {
                    self.workspace.categories_closed.remove("自定义策略组");
                }
                self.workspace.rule_picker_open = false;
                self.workspace.rule_content_request += 1;
                self.workspace.rule_content_id = Some(error.binding.id.clone());
                self.workspace.rule_content_group = error.binding.target.clone();
                self.workspace
                    .reveal_group
                    .replace(Some(error.binding.target.clone()));
                self.workspace.rule_content_original = self
                    .template_base_options
                    .as_ref()
                    .unwrap_or(&self.options)
                    .rule_set_overrides
                    .get(&error.binding.id)
                    .map(|p| p.join("\n"))
                    .unwrap_or_default();
                self.workspace.rule_content_binding = Some(error.binding.clone());
                self.workspace.rule_content_editing = true;
                self.workspace.rule_content_loading = false;
                self.workspace.rule_content_error = Some(error.message.clone());
                self.workspace.rule_content.update(cx, |input, cx| {
                    input.set_value(error.text.clone(), window, cx)
                });
                self.workspace
                    .rule_content
                    .read(cx)
                    .focus_handle(cx)
                    .focus(window, cx);
                cx.notify();
                return Err(error.message);
            }
        };
        if changes.is_empty() {
            return Ok(());
        }
        let previous = self.template_base_options.clone();
        let base = self
            .template_base_options
            .get_or_insert_with(|| self.options.clone());
        for change in &changes {
            base.rule_set_overrides
                .insert(change.binding.id.clone(), change.payload.clone());
        }
        // Validate the resulting output before committing any body changes.
        let result = self
            .draft_generate_options(cx)
            .and_then(|options| saved_configs::generate_yaml_output(&self.nodes, &options))
            .and_then(|_| self.autosave_routing(window, cx));
        if let Err(error) = result {
            self.template_base_options = previous;
            return Err(error);
        }
        for change in changes {
            drafts.remove(&change.key);
            if self.workspace.rule_content_id.as_deref() == Some(&change.binding.id) {
                let normalized = change.payload.join("\n");
                self.workspace.rule_content_original = normalized.clone();
                self.workspace
                    .rule_content
                    .update(cx, |input, cx| input.set_value(normalized, window, cx));
                self.workspace.rule_content_error = None;
            }
        }
        self.store
            .lock()
            .map_err(|e| e.to_string())?
            .set_setting(
                &format!("rule-content-drafts:{}", self.profile_id),
                &serde_json::to_string(&drafts).map_err(|e| e.to_string())?,
            )
            .map_err(|e| e.to_string())?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn choice(id: &str, active: bool) -> rules::WorkspaceRuleChoice {
        rules::WorkspaceRuleChoice {
            binding: RuleSetBinding {
                id: id.into(),
                name: id.into(),
                url: format!("https://example.org/{id}.yaml"),
                behavior: "domain".into(),
                format: "yaml".into(),
                target: "节点选择".into(),
                no_resolve: false,
            },
            source: "test".into(),
            active,
        }
    }

    #[test]
    fn save_collects_closed_editors_but_leaves_dormant_and_foreign_source_drafts() {
        let first = choice("first", true);
        let second = choice("second", true);
        let disabled = choice("disabled", false);
        let mut drafts = HashMap::new();
        drafts.insert(
            SwiftSubApp::workspace_rule_cache_key(&first.binding),
            "+.example.org".into(),
        );
        drafts.insert(
            SwiftSubApp::workspace_rule_cache_key(&second.binding),
            "example.net".into(),
        );
        drafts.insert(
            SwiftSubApp::workspace_rule_cache_key(&disabled.binding),
            "bad,rule".into(),
        );
        drafts.insert(
            "first|https://old.example/first.yaml|domain".into(),
            "bad,rule".into(),
        );
        drafts.insert(
            "removed|https://example.org/removed.yaml|domain".into(),
            "bad,rule".into(),
        );
        let changes = collect_rule_body_changes(vec![first, second, disabled], &drafts)
            .ok()
            .unwrap();
        assert_eq!(changes.len(), 2);
        assert_eq!(drafts.len(), 5);
    }

    #[test]
    fn invalid_active_body_reports_exact_owner_and_preserves_every_draft() {
        let first = choice("first", true);
        let bad = choice("bad", true);
        let drafts = HashMap::from([
            (
                SwiftSubApp::workspace_rule_cache_key(&first.binding),
                "example.org".into(),
            ),
            (
                SwiftSubApp::workspace_rule_cache_key(&bad.binding),
                "DOMAIN,example.org,PROXY".into(),
            ),
        ]);
        let error = collect_rule_body_changes(vec![first, bad], &drafts)
            .err()
            .unwrap();
        assert_eq!(error.binding.id, "bad");
        assert!(error.message.contains("bad"));
        assert_eq!(error.text, "DOMAIN,example.org,PROXY");
        assert_eq!(drafts.len(), 2);
    }
}
