use super::*;

pub(crate) struct RoutingUndo {
    before: TemplateSnapshot,
    after: TemplateSnapshot,
    name: String,
    edit_id: Option<i64>,
    pub label: String,
}

fn snapshots_match(first: &TemplateSnapshot, second: &TemplateSnapshot) -> bool {
    serde_json::to_value(first).ok() == serde_json::to_value(second).ok()
}

impl SwiftSubApp {
    pub(super) fn workspace_remember_undo(
        &mut self,
        before: TemplateSnapshot,
        name: String,
        edit_id: Option<i64>,
        label: String,
    ) {
        self.workspace.routing_undo = Some(RoutingUndo {
            before,
            after: self.active_template_snapshot(),
            name,
            edit_id,
            label,
        });
    }

    pub(crate) fn workspace_undo_available(&self, cx: &Context<Self>) -> bool {
        self.workspace.routing_undo.as_ref().is_some_and(|undo| {
            self.draft_template_snapshot(cx)
                .is_ok_and(|current| snapshots_match(&current, &undo.after))
        })
    }

    pub fn workspace_undo(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.finish_workspace_fields(window, cx) {
            return;
        }
        if !self.workspace_undo_available(cx) {
            self.show_toast("已有后续修改，当前内容已保留".into(), cx);
            return;
        }
        if let Err(error) = self.workspace_checkpoint_rule_content(cx) {
            self.template_error = Some(error);
            cx.notify();
            return;
        }
        let Some(undo) = self.workspace.routing_undo.take() else {
            return;
        };
        if let Err(error) = self.persist_and_commit_template(undo.before.clone(), None, window, cx)
        {
            self.template_error = Some(error);
            self.workspace.routing_undo = Some(undo);
            cx.notify();
            return;
        }
        self.workspace_close_rule_content(cx);
        self.load_template_draft(undo.before, undo.name, undo.edit_id, window, cx);
        self.show_toast(format!("已撤销：{}", undo.label), cx);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn undo_never_overwrites_following_member_or_rule_changes() {
        let snapshot: TemplateSnapshot = serde_json::from_value(serde_json::json!({ "preset":"minimal", "enabled_modules":[], "rule_provider_base_url":"https://example.org", "test_url":"https://example.org", "test_interval":300, "mixed_port":7890, "allow_lan":false })).unwrap();
        assert!(snapshots_match(&snapshot, &snapshot.clone()));
        let mut changed = snapshot.clone();
        changed
            .custom_rule_lines
            .push("DOMAIN,example.org,DIRECT".into());
        assert!(!snapshots_match(&snapshot, &changed));
        changed = snapshot.clone();
        changed.policy_groups.push(PolicyGroupSpec {
            name: "new".into(),
            ..Default::default()
        });
        assert!(!snapshots_match(&snapshot, &changed));
    }
}
