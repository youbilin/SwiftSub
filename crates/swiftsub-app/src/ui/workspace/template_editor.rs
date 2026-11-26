use super::*;
use crate::app::workspace::template_editor::TemplateGroupRow;
use crate::ui::layout::{disclosure_icon, group_kind_label};

fn short_kind_tag(kind: GroupKind) -> Option<&'static str> {
    match kind {
        GroupKind::UrlTest => Some("自动"),
        GroupKind::Fallback => Some("故障"),
        GroupKind::LoadBalance => Some("均衡"),
        _ => None,
    }
}

fn short_format_tag(format: &str) -> &'static str {
    match format {
        "mrs" => "MRS",
        "yaml" => "YAML",
        _ => "TXT",
    }
}

impl SwiftSubApp {
    pub(in crate::ui) fn clean_template_settings(&self, cx: &mut Context<Self>) -> AnyElement {
        if self.workspace.template_editor.open {
            return self.standalone_template_editor(cx);
        }
        let preset = self
            .template_base_options
            .as_ref()
            .unwrap_or(&self.options)
            .preset;
        let mut rows = v_flex().gap_2().min_w_0();

        for (value, label) in [
            (Preset::Minimal, "精简"),
            (Preset::Standard, "标准"),
            (Preset::Full, "完整"),
        ] {
            let active = preset == value && self.workspace.selected_template_id.is_none();
            let copy_name = format!("{label}自定义");
            let mods = catalog::modules_for_preset(value);
            let group_count = mods.len();
            let rule_set_count: usize = mods.iter().map(|m| m.rule_sets.len()).sum();
            rows = rows.child(
                object_row()
                    .px_3()
                    .when(active, |row| row.border_color(rgb(ACCENT)))
                    .child(
                        h_flex()
                            .flex_1()
                            .min_w_0()
                            .gap_2()
                            .child(div().text_sm().font_weight(FontWeight::MEDIUM).child(label))
                            .child(micro_badge("内置", false))
                            .child(
                                div()
                                    .text_size(px(11.5))
                                    .text_color(rgb(MUTED))
                                    .child(format!("{group_count} 组 · {rule_set_count} 规则集")),
                            ),
                    )
                    .child(
                        Button::new(format!("template-use-{label}"))
                            .outline()
                            .when(active, |button| button.primary())
                            .label(if active { "使用中" } else { "应用" })
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.workspace_preset(value, window, cx);
                                this.page = Page::Settings;
                                this.settings_tab = SettingsTab::Templates;
                                cx.notify();
                            })),
                    )
                    .child(
                        Button::new(format!("template-fork-{label}"))
                            .outline()
                            .label("复制")
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.open_template_editor_from_preset(
                                    value, &copy_name, window, cx,
                                );
                            })),
                    ),
            );
        }

        for template in &self.custom_templates {
            let id = template.id;
            let active = self.workspace.selected_template_id == Some(id);
            let (groups_count, rule_sets_count, manual_count) =
                crate::app::template_summary_counts(&template.config_json);
            let summary = if manual_count > 0 {
                format!("{groups_count} 组 · {rule_sets_count} 规则集 · {manual_count} 手写")
            } else {
                format!("{groups_count} 组 · {rule_sets_count} 规则集")
            };
            rows = rows.child(
                object_row()
                    .px_3()
                    .when(active, |row| row.border_color(rgb(ACCENT)))
                    .child(
                        h_flex()
                            .flex_1()
                            .min_w_0()
                            .gap_2()
                            .child(
                                div()
                                    .truncate()
                                    .text_sm()
                                    .font_weight(FontWeight::MEDIUM)
                                    .child(template.name.clone()),
                            )
                            .child(micro_badge("自定义", true))
                            .child(
                                div()
                                    .flex_shrink_0()
                                    .text_size(px(11.5))
                                    .text_color(rgb(MUTED))
                                    .child(summary),
                            ),
                    )
                    .child(
                        Button::new(format!("template-apply-{id}"))
                            .outline()
                            .when(active, |button| button.primary())
                            .label(if active { "使用中" } else { "应用" })
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.workspace_load_template(id, window, cx);
                                this.page = Page::Settings;
                                this.settings_tab = SettingsTab::Templates;
                                cx.notify();
                            })),
                    )
                    .child(
                        Button::new(format!("template-edit-{id}"))
                            .outline()
                            .icon(edit_icon())
                            .label("编辑")
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.open_edit_custom_template(id, window, cx);
                            })),
                    )
                    .child(danger_action(
                        format!("template-delete-{id}"),
                        delete_icon(),
                        "删除",
                        cx,
                        move |this, _, cx| this.delete_custom_template(id, cx),
                    )),
            );
        }

        v_flex()
            .w_full()
            .min_w_0()
            .gap_3()
            .child(
                h_flex()
                    .gap_2()
                    .child(
                        div()
                            .flex_1()
                            .text_size(px(14.5))
                            .font_weight(FontWeight::SEMIBOLD)
                            .child("分流模板"),
                    )
                    .child(
                        Button::new("template-new-from-current")
                            .outline()
                            .label("导入当前")
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.open_template_editor_from_current(window, cx);
                            })),
                    )
                    .child(
                        Button::new("template-new-standalone")
                            .primary()
                            .icon(plus_icon())
                            .label("新建")
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.open_template_editor_from_preset(
                                    Preset::Minimal,
                                    "",
                                    window,
                                    cx,
                                );
                            })),
                    ),
            )
            .child(rows)
            .into_any_element()
    }

    fn standalone_template_editor(&self, cx: &mut Context<Self>) -> AnyElement {
        let editor = &self.workspace.template_editor;
        let group_rows = self.template_editor_group_rows();
        let proxy_group_count = group_rows.iter().filter(|g| !g.is_aux).count();

        let mut cards = v_flex().gap_2().min_w_0();
        for row in group_rows {
            cards = cards.child(self.template_editor_group_card(row, cx));
        }

        v_flex()
            .w_full()
            .min_w_0()
            .gap_3()
            .child(
                h_flex()
                    .gap_2()
                    .child(
                        Button::new("tpl-editor-back")
                            .outline()
                            .label("← 返回")
                            .on_click(cx.listener(|this, _, _, cx| this.close_template_editor(cx))),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .child(Input::new(&editor.name).cleanable(true)),
                    )
                    .child(
                        Button::new("tpl-editor-save")
                            .primary()
                            .label("保存")
                            .on_click(cx.listener(|this, _, _, cx| this.save_template_editor(cx))),
                    ),
            )
            .when_some(editor.error.clone(), |panel, error| {
                panel.child(banner(error, DANGER))
            })
            .child(
                h_flex()
                    .gap_2()
                    .child(
                        div()
                            .text_size(px(13.))
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(rgb(MUTED))
                            .child(format!("策略组 · {proxy_group_count}")),
                    )
                    .child(div().flex_1())
                    .child(
                        Button::new("tpl-toggle-new-group")
                            .outline()
                            .when(editor.new_group_open, |b| b.primary())
                            .icon(plus_icon())
                            .label("新建组")
                            .on_click(cx.listener(|this, _, _, cx| {
                                let ed = &mut this.workspace.template_editor;
                                ed.new_group_open = !ed.new_group_open;
                                if ed.new_group_open {
                                    ed.builtin_picker_open = false;
                                }
                                cx.notify();
                            })),
                    )
                    .child(
                        Button::new("tpl-toggle-builtin-picker")
                            .outline()
                            .when(editor.builtin_picker_open, |b| b.primary())
                            .label("内置组")
                            .on_click(cx.listener(|this, _, _, cx| {
                                let ed = &mut this.workspace.template_editor;
                                ed.builtin_picker_open = !ed.builtin_picker_open;
                                if ed.builtin_picker_open {
                                    ed.new_group_open = false;
                                }
                                cx.notify();
                            })),
                    ),
            )
            .when(editor.new_group_open, |panel| {
                panel.child(
                    h_flex()
                        .gap_2()
                        .p_2()
                        .rounded(px(8.))
                        .border_1()
                        .border_color(rgb(BORDER_ITEM))
                        .bg(rgb(SURFACE_2))
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .child(Input::new(&editor.new_group_name).cleanable(true)),
                        )
                        .child(self.saved_choice(
                            "tpl-new-group-kind",
                            group_kind_label(editor.new_group_kind).into(),
                            vec![
                                "手动选择".into(),
                                "自动选择".into(),
                                "故障转移".into(),
                                "负载均衡".into(),
                            ],
                            cx,
                            |this, value, _, cx| {
                                this.workspace.template_editor.new_group_kind = match value.as_str()
                                {
                                    "自动选择" => GroupKind::UrlTest,
                                    "故障转移" => GroupKind::Fallback,
                                    "负载均衡" => GroupKind::LoadBalance,
                                    _ => GroupKind::Select,
                                };
                                cx.notify();
                            },
                        ))
                        .child(
                            Button::new("tpl-add-group-btn")
                                .primary()
                                .label("创建")
                                .on_click(cx.listener(|this, _, window, cx| {
                                    this.template_editor_add_group(window, cx)
                                })),
                        ),
                )
            })
            .when(editor.builtin_picker_open, |panel| {
                panel.child(self.template_editor_builtin_picker(cx))
            })
            .child(cards)
            .into_any_element()
    }

    fn template_editor_builtin_picker(&self, cx: &mut Context<Self>) -> AnyElement {
        let editor = &self.workspace.template_editor;
        let query = editor.builtin_search.read(cx).value().trim().to_lowercase();
        let mut chips = div().flex().flex_wrap().gap_2();
        for module in catalog::MODULES.iter().filter(|m| {
            !matches!(m.kind, ModuleKind::Region(_))
                && !matches!(m.id, "select" | "auto" | "final")
                && (query.is_empty()
                    || m.name.to_lowercase().contains(&query)
                    || m.id.to_lowercase().contains(&query))
        }) {
            let id = module.id.to_string();
            let active = editor.enabled_modules.contains(module.id);
            chips = chips.child(
                div()
                    .id(format!("tpl-mod-{}", module.id))
                    .cursor_pointer()
                    .h(px(30.))
                    .px(px(10.))
                    .rounded(px(6.))
                    .border_1()
                    .flex()
                    .items_center()
                    .text_size(px(12.))
                    .when(active, |el| {
                        el.bg(rgb(ACCENT_SOFT))
                            .border_color(rgb(ACCENT))
                            .text_color(rgb(TEXT))
                            .font_weight(FontWeight::MEDIUM)
                    })
                    .when(!active, |el| {
                        el.bg(rgb(SURFACE))
                            .border_color(rgb(BORDER))
                            .text_color(rgb(MUTED))
                            .hover(|s| s.bg(rgb(SURFACE_2)).text_color(rgb(TEXT)))
                    })
                    .child(module.name)
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.template_editor_toggle_module(id.clone(), cx);
                    })),
            );
        }
        v_flex()
            .min_w_0()
            .gap_2()
            .p_3()
            .rounded(px(8.))
            .border_1()
            .border_color(rgb(BORDER_ITEM))
            .bg(rgb(SURFACE_2))
            .child(Input::new(&editor.builtin_search).cleanable(true))
            .child(chips)
            .into_any_element()
    }

    fn template_editor_group_card(
        &self,
        row: TemplateGroupRow,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let editor = &self.workspace.template_editor;
        let name = row.name.clone();
        let expand_name = name.clone();
        let expanded = editor.expanded_group.as_deref() == Some(&name);

        let mut header_left = h_flex()
            .id(format!("tpl-group-row-{name}"))
            .flex_1()
            .min_w_0()
            .cursor_pointer()
            .gap_2()
            .child(
                disclosure_icon(expanded)
                    .with_size(px(14.))
                    .text_color(rgb(MUTED)),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .text_sm()
                    .font_weight(if expanded {
                        FontWeight::SEMIBOLD
                    } else {
                        FontWeight::MEDIUM
                    })
                    .child(name.clone()),
            );
        if row.is_aux {
            header_left = header_left.child(micro_badge("动作", false));
        } else if let Some(tag) = short_kind_tag(row.kind) {
            header_left = header_left.child(micro_badge(tag, false));
        }
        header_left = header_left
            .child(
                div()
                    .flex_shrink_0()
                    .text_size(px(11.5))
                    .text_color(rgb(MUTED))
                    .child(format!("{} 规则", row.rule_count)),
            )
            .on_click(cx.listener(move |this, _, _, cx| {
                this.template_editor_toggle_expand(expand_name.clone(), cx);
            }));

        let mut header = h_flex()
            .min_h(px(44.))
            .px_3()
            .gap_2()
            .bg(rgb(if expanded { ACCENT_SOFT } else { SURFACE }))
            .child(header_left);

        if let Some(idx) = row.custom_index {
            header = header.child(danger_action(
                format!("tpl-group-del-{idx}"),
                delete_icon(),
                "删除策略组",
                cx,
                move |this, _, cx| this.template_editor_remove_custom_group(idx, cx),
            ));
        } else if let Some(mod_id) = row.builtin_module_id
            && !row.is_core
        {
            let mod_id_owned = mod_id.to_string();
            header = header.child(danger_action(
                format!("tpl-mod-del-{mod_id}"),
                delete_icon(),
                "移除策略组",
                cx,
                move |this, _, cx| this.template_editor_toggle_module(mod_id_owned.clone(), cx),
            ));
        }

        let mut card = v_flex()
            .min_w_0()
            .flex_shrink_0()
            .rounded(px(8.))
            .overflow_hidden()
            .border_1()
            .border_color(rgb(if expanded { ACCENT } else { BORDER_ITEM }))
            .bg(rgb(SURFACE))
            .child(header);

        if expanded {
            card = card.child(self.template_editor_group_body(&row, cx));
        }
        card.into_any_element()
    }

    fn template_editor_group_body(
        &self,
        row: &TemplateGroupRow,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let editor = &self.workspace.template_editor;
        let target = &row.name;
        let owned_sets = self.template_editor_group_rule_sets(target);
        let manual_items: Vec<(usize, &swiftsub_core::Rule)> = editor
            .manual_rules
            .iter()
            .enumerate()
            .filter(|(_, r)| r.target == *target)
            .collect();

        // Compact group toolbar
        let mut toolbar = h_flex().gap_2().min_w_0().flex_wrap();
        if let Some(idx) = row.custom_index {
            if editor.renaming_group == Some(idx) {
                toolbar =
                    toolbar
                        .child(
                            div()
                                .flex_1()
                                .min_w(px(140.))
                                .child(Input::new(&editor.rename_input).cleanable(true)),
                        )
                        .child(
                            Button::new("tpl-rename-ok")
                                .primary()
                                .label("确定")
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.template_editor_commit_rename(cx)
                                })),
                        )
                        .child(
                            Button::new("tpl-rename-cancel")
                                .ghost()
                                .label("取消")
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.workspace.template_editor.renaming_group = None;
                                    cx.notify();
                                })),
                        );
            } else {
                toolbar = toolbar
                    .child(object_action(
                        format!("tpl-group-rename-{idx}"),
                        edit_icon(),
                        "重命名",
                        cx,
                        move |this, window, cx| this.template_editor_start_rename(idx, window, cx),
                    ))
                    .child(self.saved_choice(
                        format!("tpl-group-kind-{idx}"),
                        group_kind_label(row.kind).into(),
                        vec![
                            "手动选择".into(),
                            "自动选择".into(),
                            "故障转移".into(),
                            "负载均衡".into(),
                        ],
                        cx,
                        move |this, value, _, cx| {
                            if let Some(g) =
                                this.workspace.template_editor.policy_groups.get_mut(idx)
                            {
                                g.kind = match value.as_str() {
                                    "自动选择" => GroupKind::UrlTest,
                                    "故障转移" => GroupKind::Fallback,
                                    "负载均衡" => GroupKind::LoadBalance,
                                    _ => GroupKind::Select,
                                };
                                cx.notify();
                            }
                        },
                    ));
            }
        }
        toolbar = toolbar.child(div().flex_1()).child(
            Button::new(format!("tpl-add-rule-btn-{target}"))
                .outline()
                .when(editor.rule_add_open, |b| b.primary())
                .icon(plus_icon())
                .label("添加规则")
                .on_click(cx.listener(|this, _, _, cx| {
                    let ed = &mut this.workspace.template_editor;
                    ed.rule_add_open = !ed.rule_add_open;
                    ed.editing_rule_set_id = None;
                    if !ed.rule_add_open {
                        ed.editing_manual_index = None;
                    }
                    cx.notify();
                })),
        );

        // Owned rules list
        let mut rule_rows = v_flex().gap_1().min_w_0();
        for choice in &owned_sets {
            let id = choice.binding.id.clone();
            let edit_id = id.clone();
            let del_id = id.clone();
            let toggle_id = id.clone();
            let enabled = choice.enabled;
            rule_rows = rule_rows.child(
                h_flex()
                    .min_w_0()
                    .min_h(px(38.))
                    .px_2()
                    .gap_2()
                    .rounded(px(6.))
                    .border_1()
                    .border_color(rgb(BORDER_ITEM))
                    .bg(rgb(SURFACE))
                    .when(!enabled, |r| r.opacity(0.55))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .text_sm()
                            .text_color(rgb(if enabled { TEXT } else { MUTED }))
                            .child(choice.binding.name.clone()),
                    )
                    .child(micro_badge(short_format_tag(&choice.binding.format), false))
                    .child(object_action(
                        format!("tpl-rs-edit-{id}"),
                        edit_icon(),
                        "编辑规则集",
                        cx,
                        move |this, window, cx| {
                            this.template_editor_start_edit_rule_set(edit_id.clone(), window, cx)
                        },
                    ))
                    .child(danger_action(
                        format!("tpl-rs-del-{id}"),
                        delete_icon(),
                        "移除",
                        cx,
                        move |this, _, cx| this.template_editor_remove_rule_set(del_id.clone(), cx),
                    ))
                    .child(
                        Switch::new(format!("tpl-rs-sw-{id}"))
                            .small()
                            .checked(enabled)
                            .accessibility_label("启用规则集")
                            .on_change(cx.listener(move |this, checked, _, cx| {
                                this.template_editor_toggle_rule_set(
                                    toggle_id.clone(),
                                    *checked,
                                    cx,
                                );
                            })),
                    ),
            );
        }

        for (manual_idx, rule) in &manual_items {
            let idx = *manual_idx;
            rule_rows = rule_rows.child(
                h_flex()
                    .min_w_0()
                    .min_h(px(38.))
                    .px_2()
                    .gap_2()
                    .rounded(px(6.))
                    .border_1()
                    .border_color(rgb(BORDER_ITEM))
                    .bg(rgb(SURFACE))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .font_family("Consolas")
                            .text_size(px(12.))
                            .child(rule.to_body()),
                    )
                    .child(micro_badge("手写", true))
                    .child(object_action(
                        format!("tpl-man-edit-{idx}"),
                        edit_icon(),
                        "编辑规则",
                        cx,
                        move |this, window, cx| {
                            this.template_editor_start_edit_manual(idx, window, cx)
                        },
                    ))
                    .child(danger_action(
                        format!("tpl-man-del-{idx}"),
                        delete_icon(),
                        "删除规则",
                        cx,
                        move |this, _, cx| this.template_editor_remove_manual(idx, cx),
                    )),
            );
        }

        if owned_sets.is_empty() && manual_items.is_empty() {
            rule_rows = rule_rows.child(
                div()
                    .py_2()
                    .text_size(px(12.))
                    .text_color(rgb(MUTED))
                    .child("暂无规则"),
            );
        }

        let editing_rs_here = editor
            .editing_rule_set_id
            .as_ref()
            .is_some_and(|id| owned_sets.iter().any(|c| &c.binding.id == id));

        v_flex()
            .min_w_0()
            .p_3()
            .gap_2()
            .border_t_1()
            .border_color(rgb(BORDER_STRONG))
            .bg(rgb(SURFACE_2))
            .child(toolbar)
            .when(editor.rule_add_open, |body| {
                body.child(self.template_editor_rule_picker(target, cx))
            })
            .when(editing_rs_here, |body| {
                body.child(self.template_editor_rule_set_form(cx))
            })
            .child(rule_rows)
            .into_any_element()
    }

    fn template_editor_rule_picker(&self, target: &str, cx: &mut Context<Self>) -> AnyElement {
        let editor = &self.workspace.template_editor;
        let owner = target.to_string();

        let mut panel = v_flex()
            .min_w_0()
            .gap_2()
            .p_2()
            .rounded(px(8.))
            .border_1()
            .border_color(rgb(BORDER))
            .bg(rgb(SURFACE))
            .child(
                h_flex()
                    .gap_2()
                    .child(mode_tabs(
                        "tpl-rule-add-tab",
                        &editor.rule_add_tab,
                        &["规则库", "链接", "手写"],
                        cx,
                        |this, tab, _, cx| {
                            let ed = &mut this.workspace.template_editor;
                            ed.rule_add_tab = tab;
                            ed.editing_manual_index = None;
                            ed.error = None;
                            cx.notify();
                        },
                    ))
                    .child(div().flex_1())
                    .child(object_action(
                        "tpl-rule-add-close",
                        close_icon(),
                        "收起",
                        cx,
                        |this, _, cx| {
                            let ed = &mut this.workspace.template_editor;
                            ed.rule_add_open = false;
                            ed.editing_manual_index = None;
                            cx.notify();
                        },
                    )),
            );

        match editor.rule_add_tab.as_str() {
            "链接" => {
                panel = panel.child(
                    h_flex()
                        .gap_2()
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .child(Input::new(&editor.new_rule_set_url).cleanable(true)),
                        )
                        .child(
                            Button::new("tpl-add-url-submit")
                                .primary()
                                .label("添加")
                                .on_click(cx.listener(move |this, _, window, cx| {
                                    this.template_editor_add_url_rule(owner.clone(), window, cx)
                                })),
                        ),
                );
            }
            "手写" => {
                let is_editing = editor.editing_manual_index.is_some();
                panel = panel.child(
                    h_flex()
                        .gap_2()
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .child(Input::new(&editor.manual_rule_input).cleanable(true)),
                        )
                        .child(
                            Button::new("tpl-add-manual-submit")
                                .primary()
                                .label(if is_editing { "保存" } else { "添加" })
                                .on_click(cx.listener(move |this, _, window, cx| {
                                    this.template_editor_save_manual_rule(owner.clone(), window, cx)
                                })),
                        ),
                );
            }
            _ => {
                let mut source_options = vec!["全部来源".into(), "内置".into()];
                source_options.extend(rule_catalog::SOURCES.iter().map(|s| s.name.to_string()));
                let current_source_label = match editor.rule_source.as_str() {
                    "builtin" => "内置",
                    id => rule_catalog::SOURCES
                        .iter()
                        .find(|s| s.id == id)
                        .map(|s| s.name)
                        .unwrap_or("全部来源"),
                };
                let query = editor.rule_search.read(cx).value().trim().to_lowercase();
                let source_filter = match editor.rule_source.as_str() {
                    "builtin" => "内置",
                    id => rule_catalog::SOURCES
                        .iter()
                        .find(|s| s.id == id)
                        .map(|s| s.name)
                        .unwrap_or(""),
                };
                let mut choices: Vec<_> = self
                    .template_editor_rule_choices()
                    .into_iter()
                    .filter(|c| {
                        (source_filter.is_empty() || c.source == source_filter)
                            && (query.is_empty()
                                || format!("{} {} {}", c.binding.name, c.binding.id, c.source)
                                    .to_lowercase()
                                    .contains(&query))
                    })
                    .collect();
                if query.is_empty() {
                    let common = [
                        "openai",
                        "google",
                        "youtube",
                        "netflix",
                        "telegram",
                        "github",
                        "category-ads-all",
                    ];
                    choices.sort_by_key(|c| {
                        common
                            .iter()
                            .position(|id| *id == c.binding.id)
                            .unwrap_or(common.len())
                    });
                }

                let mut list = v_flex().gap_1().min_w_0();
                if choices.is_empty() {
                    list = list.child(
                        div()
                            .py_2()
                            .text_size(px(12.))
                            .text_color(rgb(MUTED))
                            .child("无匹配规则"),
                    );
                }
                for choice in choices.into_iter().take(80) {
                    let added = choice.active && choice.binding.target == target;
                    let elsewhere = choice.active && !added;
                    let id = choice.binding.id.clone();
                    let target_for_add = target.to_string();
                    list = list.child(
                        h_flex()
                            .min_w_0()
                            .gap_2()
                            .px_2()
                            .py_1()
                            .rounded(px(6.))
                            .hover(|s| s.bg(rgb(SURFACE_2)))
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .truncate()
                                    .text_sm()
                                    .child(choice.binding.name),
                            )
                            .child(
                                div()
                                    .flex_shrink_0()
                                    .text_size(px(11.))
                                    .text_color(rgb(MUTED))
                                    .child(if elsewhere {
                                        format!("→ {}", choice.binding.target)
                                    } else {
                                        choice.source
                                    }),
                            )
                            .child(
                                Button::new(format!("tpl-pick-rs-{id}"))
                                    .ghost()
                                    .disabled(added)
                                    .label(if added {
                                        "已添加"
                                    } else if elsewhere {
                                        "移入"
                                    } else {
                                        "添加"
                                    })
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        this.template_editor_add_catalog_rule(
                                            id.clone(),
                                            target_for_add.clone(),
                                            cx,
                                        );
                                    })),
                            ),
                    );
                }

                panel = panel
                    .child(
                        h_flex()
                            .gap_2()
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .child(Input::new(&editor.rule_search).cleanable(true)),
                            )
                            .child(self.saved_choice(
                                "tpl-rule-source",
                                current_source_label.into(),
                                source_options,
                                cx,
                                |this, label, _, cx| {
                                    this.workspace.template_editor.rule_source =
                                        match label.as_str() {
                                            "内置" => "builtin".into(),
                                            name => rule_catalog::SOURCES
                                                .iter()
                                                .find(|s| s.name == name)
                                                .map(|s| s.id.to_string())
                                                .unwrap_or_default(),
                                        };
                                    cx.notify();
                                },
                            )),
                    )
                    .child(
                        div()
                            .id("tpl-rule-catalog-list")
                            .max_h(px(210.))
                            .p_1()
                            .rounded(px(6.))
                            .border_1()
                            .border_color(rgb(BORDER_ITEM))
                            .bg(rgb(SURFACE_INSET))
                            .overflow_y_scrollbar()
                            .child(list),
                    );
            }
        }

        panel.into_any_element()
    }

    fn template_editor_rule_set_form(&self, cx: &mut Context<Self>) -> AnyElement {
        let editor = &self.workspace.template_editor;
        let targets = self.template_editor_targets();
        v_flex()
            .min_w_0()
            .gap_2()
            .p_2()
            .rounded(px(8.))
            .border_1()
            .border_color(rgb(ACCENT))
            .bg(rgb(SURFACE))
            .child(
                h_flex()
                    .gap_2()
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .child(Input::new(&editor.edit_rs_name).cleanable(true)),
                    )
                    .child(self.saved_choice(
                        "tpl-edit-rs-target",
                        editor.edit_rs_target.clone(),
                        targets,
                        cx,
                        |this, value, _, cx| {
                            this.workspace.template_editor.edit_rs_target = value;
                            cx.notify();
                        },
                    )),
            )
            .child(Input::new(&editor.edit_rs_url).cleanable(true))
            .child(
                h_flex()
                    .gap_2()
                    .flex_wrap()
                    .child(self.saved_choice(
                        "tpl-edit-rs-behavior",
                        editor.edit_rs_behavior.clone(),
                        vec!["domain".into(), "ipcidr".into(), "classical".into()],
                        cx,
                        |this, value, _, cx| {
                            this.workspace.template_editor.edit_rs_behavior = value;
                            cx.notify();
                        },
                    ))
                    .child(self.saved_choice(
                        "tpl-edit-rs-format",
                        editor.edit_rs_format.clone(),
                        vec!["mrs".into(), "yaml".into(), "text".into()],
                        cx,
                        |this, value, _, cx| {
                            this.workspace.template_editor.edit_rs_format = value;
                            cx.notify();
                        },
                    ))
                    .child(
                        div()
                            .text_size(px(11.5))
                            .text_color(rgb(MUTED))
                            .child("no-resolve"),
                    )
                    .child(
                        Switch::new("tpl-edit-rs-no-resolve")
                            .small()
                            .checked(editor.edit_rs_no_resolve)
                            .on_change(cx.listener(|this, checked, _, cx| {
                                this.workspace.template_editor.edit_rs_no_resolve = *checked;
                                cx.notify();
                            })),
                    )
                    .child(div().flex_1())
                    .child(
                        Button::new("tpl-edit-rs-cancel")
                            .ghost()
                            .label("取消")
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.workspace.template_editor.editing_rule_set_id = None;
                                cx.notify();
                            })),
                    )
                    .child(
                        Button::new("tpl-edit-rs-save")
                            .primary()
                            .label("确定")
                            .on_click(
                                cx.listener(|this, _, _, cx| {
                                    this.template_editor_save_rule_set(cx)
                                }),
                            ),
                    ),
            )
            .into_any_element()
    }
}
