use super::*;
use gpui_kit::component::checkbox::Checkbox;

impl SwiftSubApp {
    pub(in crate::ui) fn workspace_rule_picker_dialog(&self, cx: &mut Context<Self>) -> AnyElement {
        let target = self.workspace.rules_target.as_deref().unwrap_or("节点选择");
        let title = if self.workspace.manual_edit.is_some()
            || self.workspace.manual_disabled_edit.is_some()
        {
            "编辑规则"
        } else {
            "新增规则"
        };
        let panel = v_flex()
            .w(px(700.))
            .max_w(relative(0.92))
            .max_h(relative(0.88))
            .min_h_0()
            .rounded(px(12.))
            .border_1()
            .border_color(rgb(BORDER))
            .bg(rgb(SURFACE))
            .p_4()
            .gap_3()
            .child(
                h_flex()
                    .gap_2()
                    .child(
                        div()
                            .flex_1()
                            .text_size(px(16.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(format!("{title} · {target}")),
                    )
                    .child(object_action(
                        "rule-picker-dismiss",
                        close_icon(),
                        "关闭",
                        cx,
                        |this, _, cx| {
                            this.workspace.rule_picker_open = false;
                            cx.notify();
                        },
                    )),
            )
            .child(
                div()
                    .id("rule-picker-body")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scrollbar()
                    .child(
                        v_flex()
                            .min_w_0()
                            .gap_3()
                            .child(self.workspace_inline_rule_picker(target, cx))
                            .when_some(self.template_error.clone(), |panel, error| {
                                panel.child(div().text_sm().text_color(rgb(DANGER)).child(error))
                            }),
                    ),
            );
        modal_with_backdrop("rule-picker-backdrop", panel, cx, |this, _, cx| {
            this.workspace.rule_picker_open = false;
            cx.notify();
        })
    }

    pub(in crate::ui) fn workspace_group_rules_panel(
        &self,
        target: &str,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let choices = self.workspace_owned_rules(target);
        let mut rows = v_flex().gap_2().min_w_0();
        for choice in &choices {
            let id = choice.binding.id.clone();
            let view_id = id.clone();
            let edit_id = id.clone();
            let remove_id = id.clone();
            let checked = !self.template_disabled_rule_sets.contains(&id);
            rows = rows.child(
                object_row()
                    .when(!checked, |row| row.opacity(0.6))
                    .child(
                        div()
                            .id(format!("rule-view-{id}"))
                            .flex_1()
                            .min_w_0()
                            .cursor_pointer()
                            .text_sm()
                            .truncate()
                            .text_color(rgb(if checked { TEXT } else { MUTED }))
                            .child(choice.binding.name.clone())
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.workspace_view_rule_content(view_id.clone(), window, cx)
                            })),
                    )
                    .child(object_action(
                        format!("rule-edit-{id}"),
                        edit_icon(),
                        "编辑规则",
                        cx,
                        move |this, window, cx| {
                            this.workspace_edit_rule_metadata(edit_id.clone(), window, cx)
                        },
                    ))
                    .child(danger_action(
                        format!("rule-delete-{id}"),
                        delete_icon(),
                        "删除规则",
                        cx,
                        move |this, window, cx| {
                            this.remove_workspace_rule(remove_id.clone(), window, cx)
                        },
                    ))
                    .child(
                        Switch::new(format!("rule-switch-{id}"))
                            .small()
                            .checked(checked)
                            .accessibility_label("启用规则")
                            .on_change(cx.listener(move |this, checked, window, cx| {
                                this.toggle_workspace_rule(id.clone(), *checked, window, cx)
                            })),
                    ),
            );
        }
        let mut manual: Vec<_> =
            crate::app::parse_custom_rule_lines(&self.template_custom_rules.read(cx).value())
                .unwrap_or_default()
                .into_iter()
                .enumerate()
                .filter(|(_, rule)| rule.target == target)
                .map(|(index, rule)| (Some(index), rule))
                .collect();
        manual.extend(
            self.workspace
                .rule_visibility
                .disabled_manual
                .iter()
                .filter_map(|line| swiftsub_core::Rule::parse(line).ok())
                .filter(|rule| rule.target == target)
                .map(|rule| (None, rule)),
        );
        for (row_index, (index, rule)) in manual.iter().enumerate() {
            let line = rule.to_line();
            let edit_line = line.clone();
            let remove_line = line.clone();
            let index = *index;
            let enabled = index.is_some();
            rows = rows.child(
                object_row()
                    .when(!enabled, |row| row.opacity(0.6))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .text_sm()
                            .truncate()
                            .text_color(rgb(if enabled { TEXT } else { MUTED }))
                            .child(rule.to_body()),
                    )
                    .child(object_action(
                        format!("manual-edit-{row_index}"),
                        edit_icon(),
                        "编辑规则",
                        cx,
                        move |this, window, cx| {
                            this.edit_workspace_manual(edit_line.clone(), index, window, cx)
                        },
                    ))
                    .child(danger_action(
                        format!("manual-delete-{row_index}"),
                        delete_icon(),
                        "删除规则",
                        cx,
                        move |this, window, cx| {
                            this.remove_workspace_manual(remove_line.clone(), window, cx)
                        },
                    ))
                    .child(
                        Switch::new(format!("manual-switch-{row_index}"))
                            .small()
                            .checked(enabled)
                            .accessibility_label("启用规则")
                            .on_change(cx.listener(move |this, checked, window, cx| {
                                this.toggle_workspace_manual(line.clone(), *checked, window, cx)
                            })),
                    ),
            );
        }
        if choices.is_empty() && manual.is_empty() {
            rows = rows.child(
                div()
                    .py_3()
                    .text_sm()
                    .text_color(rgb(MUTED))
                    .child("暂无规则"),
            );
        }
        v_flex()
            .min_w_0()
            .gap_3()
            .child(
                div()
                    .id(format!("workspace-owned-rules-{target}"))
                    .max_h(px(300.))
                    .p_2()
                    .rounded(px(8.))
                    .border_1()
                    .border_color(rgb(BORDER_ITEM))
                    .bg(rgb(SURFACE_INSET))
                    .overflow_y_scrollbar()
                    .child(rows),
            )
            .when(
                self.workspace
                    .rule_metadata_id
                    .as_ref()
                    .is_some_and(|id| choices.iter().any(|choice| &choice.binding.id == id)),
                |panel| panel.child(self.workspace_inline_rule_metadata(cx)),
            )
            .into_any_element()
    }

    fn workspace_inline_rule_picker(&self, target: &str, cx: &mut Context<Self>) -> AnyElement {
        let mut panel = v_flex().min_w_0().gap_2().bg(rgb(SURFACE)).child(
            h_flex()
                .flex_wrap()
                .gap_2()
                .child(mode_tabs(
                    "workspace-rule-add-kind",
                    &self.workspace.rule_picker_kind,
                    &["规则库", "自定义链接", "手写规则"],
                    cx,
                    |this, kind, _, cx| {
                        this.workspace.rule_picker_kind = kind;
                        this.workspace.manual_edit = None;
                        this.workspace.manual_disabled_edit = None;
                        cx.notify();
                    },
                ))
                .child(div().flex_1()),
        );
        match self.workspace.rule_picker_kind.as_str() {
            "自定义链接" => {
                let owner = target.to_string();
                panel = panel.child(Input::new(&self.template_rule_url)).child(
                    h_flex().justify_end().child(
                        Button::new("workspace-inline-add-url")
                            .primary()
                            .label(format!("添加到 {target}"))
                            .on_click(cx.listener(move |this, _, window, cx| {
                                let previous = this.template_custom_rule_sets.clone();
                                let previous_url =
                                    this.template_rule_url.read(cx).value().to_string();
                                this.template_rules_target_filter = owner.clone();
                                this.add_template_rule_set_url(window, cx);
                                if this.template_error.is_none()
                                    && this.template_custom_rule_sets.len() != previous.len()
                                {
                                    if let Err(error) = this.autosave_routing(window, cx) {
                                        this.template_custom_rule_sets = previous;
                                        this.template_rule_url.update(cx, |input, cx| {
                                            input.set_value(previous_url, window, cx)
                                        });
                                        this.template_error = Some(error);
                                    } else {
                                        this.workspace.rule_picker_open = false;
                                    }
                                }
                                cx.notify();
                            })),
                    ),
                );
            }
            "手写规则" => {
                let owner = target.to_string();
                panel = panel
                    .child(Input::new(&self.workspace.manual_rule))
                    .child(
                        div()
                            .text_size(px(11.5))
                            .text_color(rgb(MUTED))
                            .child(format!("目标：{target}")),
                    )
                    .child(
                        h_flex().justify_end().child(
                            Button::new("workspace-inline-add-manual")
                                .primary()
                                .label(
                                    if self.workspace.manual_edit.is_some()
                                        || self.workspace.manual_disabled_edit.is_some()
                                    {
                                        "保存修改".into()
                                    } else {
                                        format!("添加到 {target}")
                                    },
                                )
                                .on_click(cx.listener(move |this, _, window, cx| {
                                    this.workspace_add_manual(owner.clone(), window, cx)
                                })),
                        ),
                    );
            }
            _ => {
                panel = panel.child(
                    h_flex()
                        .gap_2()
                        .min_w_0()
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .child(Input::new(&self.template_rule_search).cleanable(true)),
                        )
                        .child(self.template_rule_source_choices(cx)),
                );
                let query = self
                    .template_rule_search
                    .read(cx)
                    .value()
                    .trim()
                    .to_lowercase();
                let source = match self.workspace.rule_source.as_str() {
                    "builtin" => "内置规则",
                    "url" => "自定义链接",
                    id => rule_catalog::SOURCES
                        .iter()
                        .find(|source| source.id == id)
                        .map(|source| source.name)
                        .unwrap_or(""),
                };
                let mut choices: Vec<_> = self
                    .workspace_rule_choices()
                    .into_iter()
                    .filter(|choice| {
                        (source.is_empty() || choice.source == source)
                            && (query.is_empty()
                                || format!(
                                    "{} {} {}",
                                    choice.binding.name, choice.binding.id, choice.source
                                )
                                .to_lowercase()
                                .contains(&query))
                    })
                    .collect();
                if query.is_empty() {
                    let common = [
                        "openai",
                        "netflix",
                        "youtube",
                        "telegram",
                        "google",
                        "google-gemini",
                    ];
                    choices.sort_by_key(|choice| {
                        common
                            .iter()
                            .position(|id| *id == choice.binding.id)
                            .unwrap_or(common.len())
                    });
                }
                let count = choices.len();
                let mut rows = v_flex().min_w_0().gap_1();
                if choices.is_empty() {
                    rows = rows.child(
                        div()
                            .py_2()
                            .text_size(px(12.))
                            .text_color(rgb(MUTED))
                            .child("没有匹配的规则，试试其他关键词或来源"),
                    );
                }
                for choice in choices {
                    let binding = choice.binding;
                    let added = choice.active && binding.target == target;
                    let elsewhere = choice.active && !added;
                    let selected = self.workspace.rule_picks.contains(&binding.id);
                    let id = binding.id;
                    let view_id = id.clone();
                    let add_id = id.clone();
                    let label = if added {
                        String::new()
                    } else if elsewhere {
                        format!("归属：{}", binding.target)
                    } else {
                        String::new()
                    };
                    rows = rows.child(
                        h_flex()
                            .min_w_0()
                            .gap_2()
                            .px_2()
                            .py_1()
                            .rounded(px(6.))
                            .hover(|style| style.bg(rgb(SURFACE_2)))
                            .child(
                                v_flex()
                                    .id(format!("workspace-preview-rule-{id}"))
                                    .cursor_pointer()
                                    .flex_1()
                                    .min_w_0()
                                    .gap_1()
                                    .child(div().text_sm().truncate().child(binding.name))
                                    .child(
                                        div()
                                            .text_size(px(10.5))
                                            .text_color(rgb(MUTED))
                                            .truncate()
                                            .child(choice.source),
                                    )
                                    .on_click(cx.listener(move |this, _, window, cx| {
                                        this.workspace_view_rule_content(
                                            view_id.clone(),
                                            window,
                                            cx,
                                        )
                                    })),
                            )
                            .when(!label.is_empty(), |row| {
                                row.child(
                                    div()
                                        .text_size(px(10.5))
                                        .text_color(rgb(MUTED))
                                        .truncate()
                                        .max_w(px(140.))
                                        .child(label),
                                )
                            })
                            .child(
                                Button::new(format!("workspace-direct-add-{add_id}"))
                                    .ghost()
                                    .disabled(added)
                                    .label(if added {
                                        "已添加"
                                    } else if elsewhere {
                                        "移入此组"
                                    } else {
                                        "添加"
                                    })
                                    .on_click(cx.listener(move |this, _, window, cx| {
                                        this.workspace_add_rule(add_id.clone(), window, cx)
                                    })),
                            )
                            .when(!added, |row| {
                                row.child(
                                    Checkbox::new(format!("workspace-pick-rule-{id}"))
                                        .small()
                                        .checked(selected)
                                        .tooltip("选择多个规则后统一添加")
                                        .accessibility_label("批量选择规则")
                                        .on_change(cx.listener(move |this, checked, _, cx| {
                                            if *checked {
                                                this.workspace.rule_picks.insert(id.clone());
                                            } else {
                                                this.workspace.rule_picks.remove(&id);
                                            }
                                            cx.notify();
                                        })),
                                )
                            }),
                    );
                }
                panel = panel
                    .child(
                        div()
                            .text_size(px(11.5))
                            .text_color(rgb(MUTED))
                            .child(format!("{count} 个规则集 · 点击名称查看内容")),
                    )
                    .child(
                        div()
                            .id("workspace-rule-candidates")
                            .h(px(300.))
                            .p_2()
                            .rounded(px(8.))
                            .border_1()
                            .border_color(rgb(BORDER))
                            .bg(rgb(SURFACE_INSET))
                            .overflow_y_scrollbar()
                            .child(rows),
                    )
                    .when(!self.workspace.rule_picks.is_empty(), |panel| {
                        panel.child(
                            h_flex()
                                .gap_2()
                                .child(
                                    div()
                                        .flex_1()
                                        .text_size(px(11.5))
                                        .text_color(rgb(MUTED))
                                        .child(format!(
                                            "已选 {} 项",
                                            self.workspace.rule_picks.len()
                                        )),
                                )
                                .child(
                                    Button::new("workspace-confirm-add-rules")
                                        .primary()
                                        .disabled(self.workspace.rule_picks.is_empty())
                                        .label(format!("添加到 {target}"))
                                        .on_click(cx.listener(|this, _, window, cx| {
                                            this.workspace_add_selected_rules(window, cx)
                                        })),
                                ),
                        )
                    });
            }
        }
        panel.into_any_element()
    }

    fn workspace_inline_rule_metadata(&self, cx: &mut Context<Self>) -> AnyElement {
        v_flex()
            .min_w_0()
            .gap_2()
            .p_3()
            .rounded(px(8.))
            .border_1()
            .border_color(rgb(BORDER))
            .bg(rgb(SURFACE))
            .child(
                div()
                    .text_sm()
                    .font_weight(FontWeight::MEDIUM)
                    .child("规则集信息"),
            )
            .child(Input::new(&self.workspace.rule_metadata_name))
            .child(Input::new(&self.workspace.rule_metadata_url))
            .child(
                h_flex()
                    .gap_2()
                    .flex_wrap()
                    .child(self.saved_choice(
                        "workspace-rule-behavior",
                        self.workspace.rule_metadata_behavior.clone(),
                        vec!["domain".into(), "ipcidr".into(), "classical".into()],
                        cx,
                        |this, value, _, cx| {
                            this.workspace.rule_metadata_behavior = value;
                            cx.notify();
                        },
                    ))
                    .child(self.saved_choice(
                        "workspace-rule-format",
                        self.workspace.rule_metadata_format.clone(),
                        vec!["mrs".into(), "yaml".into(), "text".into()],
                        cx,
                        |this, value, _, cx| {
                            this.workspace.rule_metadata_format = value;
                            cx.notify();
                        },
                    ))
                    .child(
                        div()
                            .text_size(px(12.))
                            .text_color(rgb(MUTED))
                            .child("no-resolve"),
                    )
                    .child(
                        Switch::new("workspace-rule-no-resolve")
                            .small()
                            .checked(self.workspace.rule_metadata_no_resolve)
                            .on_change(cx.listener(|this, checked, _, cx| {
                                this.workspace.rule_metadata_no_resolve = *checked;
                                cx.notify();
                            })),
                    ),
            )
            .child(
                h_flex()
                    .justify_end()
                    .gap_2()
                    .child(
                        Button::new("workspace-rule-metadata-cancel")
                            .ghost()
                            .label("取消")
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.workspace.rule_metadata_id = None;
                                cx.notify();
                            })),
                    )
                    .child(
                        Button::new("workspace-rule-metadata-save")
                            .primary()
                            .label("保存")
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.workspace_save_rule_metadata(window, cx)
                            })),
                    ),
            )
            .into_any_element()
    }

    pub(super) fn workspace_inline_rule_content(&self, cx: &mut Context<Self>) -> AnyElement {
        let Some(binding) = &self.workspace.rule_content_binding else {
            return div().into_any_element();
        };
        let text = self.workspace.rule_content.read(cx).value().to_string();
        let count = text.lines().filter(|line| !line.trim().is_empty()).count();
        let assigned = self
            .workspace_group_rules(&self.workspace.rule_content_group)
            .iter()
            .any(|choice| choice.binding.id == binding.id);
        let modified = text != self.workspace.rule_content_original;
        let overridden = self
            .template_base_options
            .as_ref()
            .unwrap_or(&self.options)
            .rule_set_overrides
            .contains_key(&binding.id);
        let mut panel = v_flex()
            .min_w_0()
            .gap_3()
            .bg(rgb(SURFACE))
            .child(
                h_flex()
                    .min_w_0()
                    .gap_2()
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .text_size(px(15.5))
                            .font_weight(FontWeight::SEMIBOLD)
                            .truncate()
                            .child(binding.name.clone()),
                    )
                    .child(div().text_size(px(11.5)).text_color(rgb(MUTED)).child(
                        if self.workspace.rule_content_loading {
                            "读取中…".into()
                        } else {
                            format!("{} · {count} 条", binding.behavior)
                        },
                    ))
                    .child(object_action(
                        "workspace-inline-content-close",
                        close_icon(),
                        "关闭",
                        cx,
                        |this, _, cx| this.workspace_close_rule_content(cx),
                    )),
            )
            .child(
                div()
                    .text_size(px(11.))
                    .text_color(rgb(MUTED))
                    .truncate()
                    .child(binding.url.clone()),
            )
            .child(div().text_size(px(11.5)).text_color(rgb(MUTED)).child(
                if modified && assigned {
                    "正文有修改 · 保存配置时一并应用"
                } else if overridden {
                    "本地覆写 · 修改不会影响远端来源"
                } else if assigned {
                    "来源规则 · 可直接编辑，本地保存"
                } else {
                    "添加前预览 · 不改变当前配置"
                },
            ));
        if self.workspace.rule_content_editing {
            panel = panel
                .child(
                    Textarea::new(&self.workspace.rule_content)
                        .h(px(320.))
                        .disabled(self.workspace.rule_content_loading),
                )
                .child(
                    h_flex()
                        .gap_2()
                        .justify_end()
                        .child(
                            Button::new("workspace-inline-content-cancel")
                                .ghost()
                                .label("撤回正文修改")
                                .on_click(cx.listener(|this, _, window, cx| {
                                    this.workspace_cancel_rule_content(window, cx)
                                })),
                        )
                        .child(
                            Button::new("workspace-inline-content-save")
                                .primary()
                                .label("应用修改")
                                .disabled(self.workspace.rule_content_loading)
                                .on_click(cx.listener(|this, _, window, cx| {
                                    this.workspace_save_rule_content(window, cx)
                                })),
                        ),
                );
        } else {
            let query = self
                .workspace
                .rule_content_search
                .read(cx)
                .value()
                .trim()
                .to_lowercase();
            let lines: Vec<_> = text
                .lines()
                .enumerate()
                .filter(|(_, line)| query.is_empty() || line.to_lowercase().contains(&query))
                .map(|(index, line)| (index + 1, line.to_string()))
                .collect();
            panel = panel
                .child(Input::new(&self.workspace.rule_content_search).cleanable(true))
                .child(
                    div()
                        .p_2()
                        .rounded(px(8.))
                        .border_1()
                        .border_color(rgb(BORDER))
                        .bg(rgb(SURFACE_INSET))
                        .child(
                            gpui_kit::uniform_list(
                                "workspace-rule-content-lines",
                                lines.len(),
                                move |range, _, _| {
                                    range
                                        .map(|index| {
                                            let (number, line) = &lines[index];
                                            h_flex()
                                                .h(px(24.))
                                                .gap_2()
                                                .min_w_0()
                                                .child(
                                                    div()
                                                        .w(px(42.))
                                                        .text_size(px(10.5))
                                                        .text_color(rgb(MUTED))
                                                        .child(number.to_string()),
                                                )
                                                .child(
                                                    div()
                                                        .flex_1()
                                                        .min_w_0()
                                                        .text_size(px(12.))
                                                        .truncate()
                                                        .font_family("Consolas")
                                                        .child(line.clone()),
                                                )
                                        })
                                        .collect::<Vec<_>>()
                                },
                            )
                            .h(px(300.))
                            .w_full(),
                        ),
                )
                .child(
                    h_flex()
                        .gap_2()
                        .flex_wrap()
                        .child(self.saved_choice(
                            "workspace-inline-content-actions",
                            "来源".into(),
                            vec!["更新来源".into(), "恢复来源".into()],
                            cx,
                            |this, action, window, cx| {
                                if this.workspace.rule_content_loading {
                                    return;
                                }
                                if action == "恢复来源" {
                                    this.workspace_restore_rule_content(window, cx);
                                } else {
                                    this.workspace_refresh_rule_content(window, cx);
                                }
                            },
                        ))
                        .child(div().flex_1())
                        .when(!assigned, |row| {
                            row.child(
                                Button::new("workspace-inline-content-add")
                                    .primary()
                                    .label("添加到当前组")
                                    .disabled(self.workspace.rule_content_loading)
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        if let Some(id) = this.workspace.rule_content_id.clone() {
                                            this.workspace_add_rule(id, window, cx);
                                        }
                                    })),
                            )
                        })
                        .when(assigned, |row| {
                            row.child(
                                Button::new("workspace-inline-content-edit")
                                    .outline()
                                    .label("编辑正文")
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.workspace.rule_content_editing = true;
                                        cx.notify();
                                    })),
                            )
                        }),
                );
        }
        panel
            .when(
                self.workspace.rule_content_error.is_some() && !self.workspace.rule_content_loading,
                |panel| {
                    panel.child(
                        Button::new("workspace-rule-retry")
                            .ghost()
                            .label("重试加载")
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.workspace_refresh_rule_content(window, cx);
                            })),
                    )
                },
            )
            .when_some(self.workspace.rule_content_error.clone(), |panel, error| {
                panel.child(
                    div()
                        .text_size(px(12.))
                        .text_color(rgb(DANGER))
                        .child(error),
                )
            })
            .into_any_element()
    }

    pub(in crate::ui) fn workspace_inline_group_settings(
        &self,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let Some(index) = self.template_policy_edit else {
            return div().into_any_element();
        };
        let Some(group) = self.template_policy_groups.get(index) else {
            return div().into_any_element();
        };
        let mut panel = v_flex()
            .min_w_0()
            .gap_2()
            .p_3()
            .rounded(px(8.))
            .border_1()
            .border_color(rgb(BORDER))
            .bg(rgb(SURFACE))
            .child(field_label("组名称"))
            .child(Input::new(&self.template_policy_rename))
            .child(field_label("说明（可选）"))
            .child(Input::new(&self.workspace.description));
        if group.kind != GroupKind::Select {
            panel = panel
                .child(field_label("测速地址（留空使用全局默认）"))
                .child(Input::new(&self.template_health_url).cleanable(true))
                .child(
                    h_flex()
                        .gap_2()
                        .child(
                            v_flex()
                                .flex_1()
                                .min_w_0()
                                .gap_1()
                                .child(field_label("测速周期（秒）"))
                                .child(Input::new(&self.template_health_interval).cleanable(true)),
                        )
                        .when(group.kind == GroupKind::UrlTest, |row| {
                            row.child(
                                v_flex()
                                    .flex_1()
                                    .min_w_0()
                                    .gap_1()
                                    .child(field_label("测速容差（毫秒）"))
                                    .child(Input::new(&self.workspace.tolerance).cleanable(true)),
                            )
                        }),
                );
        }
        if group.kind == GroupKind::LoadBalance {
            panel = panel.child(field_label("负载均衡策略")).child(
                self.saved_choice(
                    "workspace-inline-lb-strategy",
                    group
                        .strategy
                        .clone()
                        .unwrap_or_else(|| "consistent-hashing".into()),
                    vec![
                        "consistent-hashing".into(),
                        "round-robin".into(),
                        "sticky-sessions".into(),
                    ],
                    cx,
                    move |this, value, _, cx| {
                        this.template_policy_groups[index].strategy = Some(value);
                        cx.notify();
                    },
                ),
            );
        }
        panel
            .child(
                h_flex()
                    .justify_end()
                    .gap_2()
                    .when(group.builtin, |row| {
                        row.child(
                            Button::new("workspace-group-settings-reset")
                                .ghost()
                                .label("恢复默认")
                                .on_click(cx.listener(move |this, _, window, cx| {
                                    this.remove_template_policy_group(index, cx);
                                    if this.template_error.is_none()
                                        && let Err(error) = this.autosave_routing(window, cx)
                                    {
                                        this.template_error = Some(error);
                                    }
                                })),
                        )
                    })
                    .child(
                        Button::new("workspace-group-settings-close")
                            .ghost()
                            .label("收起")
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.workspace.group_settings_open = false;
                                cx.notify();
                            })),
                    )
                    .child(
                        Button::new("workspace-group-settings-save")
                            .primary()
                            .label("保存")
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.workspace_save_group_settings(index, window, cx);
                            })),
                    ),
            )
            .into_any_element()
    }
}
