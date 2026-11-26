use super::*;
mod inline_rules;
mod settings;

impl SwiftSubApp {
    pub(super) fn workspace_routing_panel(&self, cx: &mut Context<Self>) -> AnyElement {
        let base = self.template_base_options.as_ref().unwrap_or(&self.options);
        let chain = self.template_tab == TemplateTab::Tools;
        let error = self.template_error.clone().or_else(|| {
            crate::app::parse_custom_rule_lines(&self.template_custom_rules.read(cx).value()).err()
        });
        v_flex()
            .size_full()
            .min_h_0()
            .min_w_0()
            .p_3()
            .gap_3()
            .rounded(px(12.))
            .border_1()
            .border_color(rgb(BORDER))
            .bg(rgb(SURFACE))
            .child(
                h_flex()
                    .gap_2()
                    .child(
                        Button::new("workspace-tab-groups")
                            .outline()
                            .label("策略组")
                            .when(!chain, |button| button.primary())
                            .on_click(cx.listener(|this, _, window, cx| {
                                if this.finish_workspace_fields(window, cx) {
                                    this.template_tab = TemplateTab::PolicyGroups;
                                    cx.notify();
                                }
                            })),
                    )
                    .child(
                        Button::new("workspace-tab-chains")
                            .outline()
                            .label("链式")
                            .when(chain, |button| button.primary())
                            .on_click(cx.listener(|this, _, window, cx| {
                                if this.finish_workspace_fields(window, cx) {
                                    this.template_tab = TemplateTab::Tools;
                                    cx.notify();
                                }
                            })),
                    )
                    .when(!chain, |row| {
                        row.child(object_action(
                            "workspace-new-group",
                            plus_icon(),
                            "新增策略组",
                            cx,
                            |this, _, cx| {
                                this.workspace.new_group_open = !this.workspace.new_group_open;
                                cx.notify();
                            },
                        ))
                    })
                    .child(div().flex_1())
                    .child(div().text_sm().text_color(rgb(MUTED)).child("DNS 覆写"))
                    .child(
                        Switch::new("workspace-dns-override")
                            .small()
                            .checked(base.dns_override)
                            .accessibility_label("DNS 覆写")
                            .on_change(cx.listener(|this, checked, window, cx| {
                                this.workspace_toggle_dns(*checked, window, cx)
                            })),
                    ),
            )
            .when_some(error, |panel, error| {
                panel.child(div().text_sm().text_color(rgb(DANGER)).child(error))
            })
            .when(self.workspace.rule_repair_open, |panel| {
                panel.child(self.workspace_rule_repair_panel(cx))
            })
            .child(if chain {
                self.clean_chain_panel(cx)
            } else {
                self.workspace_groups_panel(cx)
            })
            .into_any_element()
    }

    pub(super) fn workspace_rule_content_dialog(&self, cx: &mut Context<Self>) -> AnyElement {
        let panel = v_flex()
            .w(px(760.))
            .max_w(relative(0.92))
            .max_h(relative(0.88))
            .min_h_0()
            .bg(rgb(SURFACE))
            .border_1()
            .border_color(rgb(BORDER))
            .rounded(px(12.))
            .p_4()
            .child(self.workspace_inline_rule_content(cx));
        modal_with_backdrop("rule-content-backdrop", panel, cx, |this, _, cx| {
            this.workspace_close_rule_content(cx)
        })
    }

    pub(super) fn workspace_groups_panel(&self, cx: &mut Context<Self>) -> AnyElement {
        let mut rows = v_flex().min_w_0().gap_2();
        if self.workspace.new_group_open {
            let inactive: Vec<String> = catalog::MODULES
                .iter()
                .filter(|module| !matches!(module.kind, ModuleKind::Region(_)))
                .filter(|module| {
                    self.template_policy_groups
                        .iter()
                        .any(|group| group.builtin_name() == Some(module.name) && group.removed)
                        || (!self.template_enabled_modules.contains(module.id)
                            && !matches!(module.id, "select" | "auto" | "final")
                            && !self
                                .template_policy_groups
                                .iter()
                                .any(|group| group.builtin_name() == Some(module.name)))
                })
                .map(|module| self.workspace_resolved_group_name(module.name).to_string())
                .collect();
            rows = rows.child(
                v_flex()
                    .flex_shrink_0()
                    .gap_2()
                    .p_3()
                    .rounded(px(8.))
                    .border_1()
                    .border_color(rgb(BORDER_ITEM))
                    .bg(rgb(SURFACE_2))
                    .child(
                        h_flex()
                            .gap_2()
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .child(Input::new(&self.template_policy_name)),
                            )
                            .child(
                                Button::new("workspace-add-group")
                                    .primary()
                                    .label("创建")
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        let count = this.template_policy_groups.len();
                                        this.add_template_policy_group(window, cx);
                                        if this.template_policy_groups.len() > count {
                                            this.workspace.new_group_open = false;
                                        }
                                    })),
                            ),
                    )
                    .child(
                        h_flex()
                            .gap_2()
                            .when(!inactive.is_empty(), |row| {
                                row.child(self.saved_choice(
                                    "workspace-add-builtin",
                                    "内置策略组".into(),
                                    inactive,
                                    cx,
                                    |this, name, window, cx| {
                                        this.toggle_workspace_group(name, true, window, cx);
                                        if this.template_error.is_none() {
                                            this.workspace.new_group_open = false;
                                        }
                                        cx.notify();
                                    },
                                ))
                            })
                            .child(
                                Button::new("workspace-add-filter")
                                    .outline()
                                    .label("按条件筛选节点")
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.open_custom_dialog(CustomDialog::Filter, cx)
                                    })),
                            ),
                    ),
            );
        }
        let reveal = self.workspace.reveal_group.borrow_mut().take();
        let offset = usize::from(self.workspace.new_group_open);
        let group_items = self.workspace_group_items();
        for (index, item) in group_items.into_iter().enumerate() {
            if reveal.as_deref() == Some(&item.name) {
                self.workspace
                    .group_scroll
                    .scroll_to_top_of_item(index + offset);
            }
            rows = rows.child(if let Some(filter_index) = item.filter {
                self.clean_filter_group_card(filter_index, index, cx)
            } else {
                self.clean_group_card(&item.name, item.index, item.module, item.enabled, index, cx)
            });
        }
        for target in ["DIRECT", "REJECT", "REJECT-DROP"] {
            let aux_rules = self.workspace_group_rules(target);
            if aux_rules.is_empty() {
                continue;
            }
            let target_name = target.to_string();
            let expanded = self.workspace.rules_target.as_deref() == Some(target);
            rows = rows.child(
                v_flex()
                    .flex_shrink_0()
                    .rounded(px(8.))
                    .overflow_hidden()
                    .border_1()
                    .border_color(rgb(if expanded { ACCENT } else { BORDER_ITEM }))
                    .bg(rgb(SURFACE))
                    .child(
                        h_flex()
                            .id(format!("workspace-aux-rules-{target}"))
                            .min_h(px(48.))
                            .px_3()
                            .gap_2()
                            .cursor_pointer()
                            .bg(rgb(if expanded { ACCENT_SOFT } else { SURFACE }))
                            .child(
                                layout::disclosure_icon(expanded)
                                    .with_size(px(14.))
                                    .text_color(rgb(MUTED)),
                            )
                            .child(
                                div()
                                    .flex_1()
                                    .text_sm()
                                    .font_weight(FontWeight::MEDIUM)
                                    .child(target_name.clone()),
                            )
                            .child(
                                div()
                                    .text_size(px(11.5))
                                    .text_color(rgb(MUTED))
                                    .child(format!("{} 条规则", aux_rules.len())),
                            )
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.workspace_rules_for(target_name.clone(), window, cx);
                                this.workspace.group_rules_tab = true;
                            })),
                    )
                    .when(expanded, |card| {
                        card.child(
                            div()
                                .p_3()
                                .border_t_1()
                                .border_color(rgb(BORDER))
                                .bg(rgb(SURFACE_2))
                                .child(self.workspace_group_rules_panel(target, cx)),
                        )
                    }),
            );
        }
        div()
            .id("workspace-group-cards")
            .flex_1()
            .min_h_0()
            .overflow_hidden()
            .flex()
            .child(
                rows.id("workspace-group-scroll")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .track_scroll(&self.workspace.group_scroll)
                    .vertical_scrollbar(&self.workspace.group_scroll),
            )
            .into_any_element()
    }

    fn workspace_rule_repair_panel(&self, cx: &mut Context<Self>) -> AnyElement {
        v_flex()
            .min_w_0()
            .gap_2()
            .p_3()
            .border_1()
            .border_color(rgb(BORDER))
            .rounded(px(8.))
            .bg(rgb(SURFACE_2))
            .child(
                h_flex()
                    .gap_2()
                    .child(div().flex_1().text_sm().child("自定义规则草稿"))
                    .child(object_action(
                        "workspace-rule-repair-close",
                        close_icon(),
                        "收起草稿",
                        cx,
                        |this, _, cx| {
                            this.workspace.rule_repair_open = false;
                            cx.notify();
                        },
                    )),
            )
            .child(Textarea::new(&self.template_custom_rules).h(px(120.)))
            .child(
                h_flex()
                    .gap_2()
                    .justify_end()
                    .child(
                        Button::new("workspace-rule-repair-restore")
                            .ghost()
                            .label("恢复上次有效规则")
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.workspace_restore_valid_rules(window, cx)
                            })),
                    )
                    .child(
                        Button::new("workspace-rule-repair-apply")
                            .primary()
                            .label("应用修正")
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.workspace_repair_rules(window, cx)
                            })),
                    ),
            )
            .into_any_element()
    }
}
