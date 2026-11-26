use super::layout::{
    configuration_icon, disclosure_icon, grip_icon, lan_icon, link_icon, swap_icon,
};
use super::phase2::{history_icon, qr_icon};
use super::*;
use crate::app::SavedEditorTab;
use crate::saved_document::{SavedDocument, rule_target, text};
use gpui_kit::component::menu::{DropdownMenu as _, PopupMenuItem};
use gpui_kit::component::tooltip::Tooltip;
use serde_yaml_ng::Value;
use std::rc::Rc;
use swiftsub_core::generator::RuleSetBinding;

#[derive(Clone)]
struct SavedOrderDrag {
    config: String,
    key: &'static str,
    group: String,
    index: usize,
    name: String,
}

fn saved_objects(doc: &SavedDocument, key: &str) -> Vec<(Value, bool, Option<usize>)> {
    let mut rows: Vec<_> = doc
        .items(key)
        .into_iter()
        .enumerate()
        .map(|(i, value)| (value, true, Some(i)))
        .collect();
    let mut disabled: Vec<_> = match key {
        "proxies" => doc
            .metadata
            .disabled_nodes
            .iter()
            .map(|item| (item.index, item.value.clone()))
            .collect(),
        "proxy-groups" => doc
            .metadata
            .disabled_groups
            .iter()
            .map(|item| (item.index, item.value.clone()))
            .collect(),
        _ => Vec::new(),
    };
    disabled.sort_by_key(|item| item.0);
    for (index, value) in disabled {
        rows.insert(index.min(rows.len()), (value, false, None));
    }
    rows
}

fn saved_panel() -> Div {
    v_flex()
        .size_full()
        .min_h_0()
        .min_w_0()
        .p_3()
        .gap_2()
        .rounded(px(12.))
        .border_1()
        .border_color(rgb(BORDER))
        .bg(rgb(SURFACE))
}

fn saved_group_kind_label(kind: &str) -> &'static str {
    match kind {
        "url-test" => "自动选择",
        "fallback" => "故障转移",
        "load-balance" => "负载均衡",
        _ => "手动选择",
    }
}

impl SwiftSubApp {
    pub(super) fn saved_configs_editor(&self, width: Pixels, cx: &mut Context<Self>) -> AnyElement {
        self.clean_saved_generator_page(width, cx)
    }

    pub(super) fn clean_saved_generator_page(
        &self,
        width: Pixels,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let sidebar_width = px(self.workspace.sidebar_width)
            .max(px(270.))
            .min((width * 0.36).max(px(280.)).min(px(420.)));

        let mut page = v_flex().size_full().min_h_0().p_3().gap_3().child(
            h_flex()
                .min_h(px(50.))
                .px_3()
                .py_2()
                .gap_3()
                .rounded(px(10.))
                .border_1()
                .border_color(rgb(BORDER))
                .bg(rgb(SURFACE))
                .child(
                    div()
                        .w(px(200.))
                        .flex_shrink_0()
                        .child(Input::new(&self.saved_config_name)),
                )
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .text_size(px(11.5))
                        .text_color(rgb(MUTED))
                        .truncate()
                        .child(self.saved_output_status(cx)),
                )
                .when_some(self.saved_config_id.clone(), |row, id| {
                    let lan_id = id.clone();
                    let qr_id = id.clone();
                    let hist_id = id.clone();
                    row.child(object_action(
                        "saved-history",
                        history_icon(),
                        "历史版本与回滚",
                        cx,
                        move |this, _, cx| this.open_config_history(hist_id.clone(), cx),
                    ))
                    .child(object_action(
                        "saved-copy-subscription",
                        link_icon(),
                        "复制本机订阅地址",
                        cx,
                        move |this, _, cx| this.copy_config_subscription(id.clone(), cx),
                    ))
                    .when(self.server_bind_lan, |row| {
                        row.child(
                            Button::new("saved-copy-lan-subscription")
                                .outline()
                                .icon(lan_icon())
                                .label("局域网地址")
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.copy_config_lan_subscription(lan_id.clone(), cx)
                                })),
                        )
                        .child(object_action(
                            "saved-lan-qr",
                            qr_icon(),
                            "扫码导入（局域网二维码）",
                            cx,
                            move |this, _, cx| this.open_config_lan_qr(qr_id.clone(), cx),
                        ))
                    })
                })
                .child(
                    Button::new("save-generated-copy")
                        .outline()
                        .label("存为新配置")
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.request_saved_copy(window, cx);
                        })),
                )
                .child(
                    Button::new("save-generated-config")
                        .primary()
                        .label("保存")
                        .tooltip("保存并更新此配置的本地订阅 · Ctrl+S")
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.save_generation_draft(window, cx)
                        })),
                )
                .child(
                    Button::new("generator-import-clash")
                        .outline()
                        .label(if self.clash_import_busy {
                            "正在导入…"
                        } else {
                            "导入到 Clash"
                        })
                        .disabled(self.clash_import_busy)
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.import_saved_to_clash(window, cx)
                        })),
                ),
        );
        if let Some(error) = &self.saved_config_error {
            page = page.child(banner(error.clone(), DANGER));
        }
        if self.saved_external_change {
            page = page.child(
                h_flex().gap_2().child(
                    Button::new("saved-load-external")
                        .outline()
                        .label("载入外部文件修改")
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.load_saved_external_file(window, cx)
                        })),
                ),
            );
        }
        if self.saved_draft_corrupt {
            page = page.child(
                h_flex().gap_2().child(
                    Button::new("saved-recover-draft")
                        .outline()
                        .label("恢复已保存配置")
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.recover_saved_yaml_draft(window, cx)
                        })),
                ),
            );
        }
        page = page.child(
            h_flex()
                .flex_1()
                .min_h_0()
                .items_stretch()
                .gap_1()
                .child(
                    div()
                        .w(sidebar_width)
                        .flex_shrink_0()
                        .min_h_0()
                        .child(self.clean_saved_nodes(cx)),
                )
                .child(self.sidebar_resize_handle(cx))
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .min_h_0()
                        .child(self.clean_saved_routing(cx)),
                ),
        );
        if self.saved_editor.editing_node {
            page = page.child(self.clean_saved_node_dialog(cx));
        }
        if self.saved_editor.node_import {
            page = page.child(self.clean_saved_source_dialog(cx));
        }
        if self.saved_editor.picking_rules {
            page = page.child(self.clean_saved_rule_dialog(cx));
        }
        if self.saved_editor.more || self.saved_editor.document.is_none() {
            page = page.child(self.clean_saved_yaml_dialog(cx));
        }
        if self.saved_editor.copy_open {
            page = page.child(self.saved_copy_dialog(cx));
        }
        page.into_any_element()
    }

    fn saved_copy_dialog(&self, cx: &mut Context<Self>) -> AnyElement {
        let panel = v_flex()
            .w(px(420.))
            .max_w(relative(0.92))
            .p_4()
            .gap_3()
            .rounded(px(12.))
            .bg(rgb(SURFACE))
            .border_1()
            .border_color(rgb(BORDER))
            .child(
                h_flex()
                    .gap_2()
                    .child(
                        div()
                            .flex_1()
                            .text_size(px(16.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .child("存为新配置"),
                    )
                    .child(object_action(
                        "close-saved-copy",
                        close_icon(),
                        "关闭",
                        cx,
                        |this, _, cx| {
                            this.saved_editor.copy_open = false;
                            cx.notify();
                        },
                    )),
            )
            .child(Input::new(&self.saved_editor.copy_name))
            .when_some(self.saved_config_error.clone(), |panel, error| {
                panel.child(div().text_sm().text_color(rgb(DANGER)).child(error))
            })
            .child(
                h_flex()
                    .justify_end()
                    .gap_2()
                    .child(
                        Button::new("cancel-saved-copy")
                            .ghost()
                            .label("取消")
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.saved_editor.copy_open = false;
                                cx.notify();
                            })),
                    )
                    .child(
                        Button::new("confirm-saved-copy")
                            .primary()
                            .label("保存")
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.save_yaml_editor(true, window, cx);
                            })),
                    ),
            );
        modal_with_backdrop("saved-copy-backdrop", panel, cx, |this, _, cx| {
            this.saved_editor.copy_open = false;
            cx.notify();
        })
    }

    fn saved_drag_handle(
        &self,
        key: &'static str,
        group: String,
        index: usize,
        name: String,
    ) -> AnyElement {
        let drag = SavedOrderDrag {
            config: self.saved_config_id.clone().unwrap_or_default(),
            key,
            group: group.clone(),
            index,
            name,
        };
        div()
            .id(format!("saved-grip-{key}-{group}-{index}"))
            .w(px(18.))
            .h(px(32.))
            .flex_shrink_0()
            .flex()
            .items_center()
            .justify_center()
            .cursor(CursorStyle::OpenHand)
            .text_color(rgb(MUTED))
            .hover(|style| {
                style
                    .text_color(rgb(ACCENT))
                    .bg(rgb(ACCENT_SOFT))
                    .rounded(px(4.))
            })
            .tooltip(|window, cx| Tooltip::new("拖动调整顺序").build(window, cx))
            .child(grip_icon().with_size(px(14.)))
            .on_drag(drag, |drag, _, _, cx| {
                cx.new(|_| NodeDragPreview {
                    name: drag.name.clone(),
                })
            })
            .into_any_element()
    }

    fn saved_order_row(
        &self,
        row: Div,
        key: &'static str,
        group: String,
        index: usize,
        cx: &mut Context<Self>,
    ) -> Stateful<Div> {
        let config = self.saved_config_id.clone().unwrap_or_default();
        let drag_group = group.clone();
        row.id(format!("saved-drop-{key}-{group}-{index}"))
            .can_drop(move |value, _, _| {
                value.downcast_ref::<SavedOrderDrag>().is_some_and(|drag| {
                    drag.config == config
                        && drag.key == key
                        && drag.group == drag_group
                        && drag.index != index
                })
            })
            .drag_over::<SavedOrderDrag>(move |style, drag, _, _| {
                let style = style.border_color(rgb(ACCENT)).bg(rgb(ACCENT_SOFT));
                if drag.index < index {
                    style.border_b_2()
                } else if drag.index > index {
                    style.border_t_2()
                } else {
                    style
                }
            })
            .on_drop(cx.listener(move |this, drag: &SavedOrderDrag, window, cx| {
                if this.saved_config_id.as_deref() != Some(&drag.config)
                    || drag.key != key
                    || drag.group != group
                {
                    return;
                }
                let from = drag.index;
                this.edit_saved_document(window, cx, |doc| {
                    if key == "member" {
                        doc.move_member(&group, from, index)
                    } else {
                        doc.move_item(key, from, index)
                    }
                });
            }))
    }

    fn clean_saved_nodes(&self, cx: &mut Context<Self>) -> AnyElement {
        let mut rows = v_flex().gap_2();
        let mut total = 0usize;
        if let Some(doc) = &self.saved_editor.document {
            let objects = saved_objects(doc, "proxies");
            total = objects.len();
            for (node, enabled, position) in objects {
                let name = text(&node, "name");
                let kind = text(&node, "type").to_uppercase();
                let delete = name.clone();
                let edit = name.clone();
                let toggle = name.clone();
                let chain = !text(&node, "dialer-proxy").is_empty();
                let group_id = SharedString::from(format!("saved-node-group-{name}"));
                let mut row = object_row()
                    .group(group_id.clone())
                    .when(!enabled, |row| row.opacity(0.6))
                    .hover(|style| style.bg(rgb(SURFACE_2)));
                if let Some(index) = position {
                    row = row.child(self.saved_drag_handle(
                        "proxies",
                        String::new(),
                        index,
                        name.clone(),
                    ));
                } else {
                    row = row.child(div().w(px(18.)).flex_shrink_0());
                }
                row = row
                    .child(
                        h_flex()
                            .relative()
                            .flex_1()
                            .min_w_0()
                            .gap_2()
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .text_sm()
                                    .truncate()
                                    .text_color(rgb(if enabled { TEXT } else { MUTED }))
                                    .child(name.clone()),
                            )
                            .child(if chain {
                                micro_badge("链式", true)
                            } else if !kind.is_empty() {
                                micro_badge(kind, false)
                            } else {
                                div()
                            })
                            .child(
                                h_flex()
                                    .absolute()
                                    .right_0()
                                    .top_0()
                                    .bottom_0()
                                    .pl_2()
                                    .gap_1()
                                    .bg(rgb(SURFACE_2))
                                    .rounded(px(6.))
                                    .invisible()
                                    .group_hover(group_id, |style| style.visible())
                                    .child(object_action(
                                        format!("saved-edit-node-{name}"),
                                        edit_icon(),
                                        "编辑节点",
                                        cx,
                                        move |this, window, cx| {
                                            if chain {
                                                this.saved_begin_chain(
                                                    Some(edit.clone()),
                                                    window,
                                                    cx,
                                                );
                                            } else {
                                                this.saved_open_node(edit.clone(), window, cx);
                                            }
                                        },
                                    ))
                                    .child(danger_action(
                                        format!("saved-delete-node-{name}"),
                                        delete_icon(),
                                        "删除节点",
                                        cx,
                                        move |this, window, cx| {
                                            this.edit_saved_document(window, cx, |doc| {
                                                doc.remove_node(&delete)
                                            });
                                        },
                                    )),
                            ),
                    )
                    .child(
                        Switch::new(format!("saved-toggle-node-{name}"))
                            .small()
                            .checked(enabled)
                            .accessibility_label("启用节点")
                            .on_change(cx.listener(move |this, checked, window, cx| {
                                this.edit_saved_document(window, cx, |doc| {
                                    doc.toggle_node(&toggle, *checked)
                                })
                            })),
                    );
                rows = if let Some(index) = position {
                    rows.child(self.saved_order_row(row, "proxies", String::new(), index, cx))
                } else {
                    rows.child(row)
                };
            }
            if total == 0 {
                rows = rows.child(
                    div()
                        .py_6()
                        .flex()
                        .justify_center()
                        .text_sm()
                        .text_color(rgb(MUTED))
                        .child("暂无节点"),
                );
            }
        }
        saved_panel()
            .child(
                h_flex()
                    .gap_2()
                    .child(
                        Button::new("editor-add-source")
                            .outline()
                            .flex_1()
                            .icon(plus_icon())
                            .label("添加节点")
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.saved_editor.node_import = true;
                                cx.notify();
                            })),
                    )
                    .when(total > 0, |row| {
                        row.child(
                            div()
                                .text_size(px(11.5))
                                .text_color(rgb(MUTED))
                                .child(format!("{total}")),
                        )
                    }),
            )
            .child(
                div()
                    .id("saved-generator-nodes")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scrollbar()
                    .child(rows),
            )
            .into_any_element()
    }

    fn clean_saved_routing(&self, cx: &mut Context<Self>) -> AnyElement {
        let chains = self.saved_editor.chains_tab;
        saved_panel()
            .child(
                h_flex()
                    .gap_2()
                    .child(
                        Button::new("workspace-tab-groups")
                            .outline()
                            .label("策略组")
                            .when(!chains, |button| button.primary())
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.saved_editor.chains_tab = false;
                                cx.notify();
                            })),
                    )
                    .child(
                        Button::new("workspace-tab-chains")
                            .outline()
                            .label("链式")
                            .when(chains, |button| button.primary())
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.saved_begin_chain(None, window, cx)
                            })),
                    )
                    .when(!chains, |row| {
                        row.child(object_action(
                            "saved-add-group",
                            plus_icon(),
                            "新建策略组",
                            cx,
                            |this, window, cx| {
                                this.saved_add_group(window, cx);
                                this.saved_editor.group_expanded = true;
                                this.saved_editor.editing_group = true;
                            },
                        ))
                    })
                    .child(div().flex_1())
                    .child(object_action(
                        "saved-full-yaml",
                        configuration_icon(),
                        "编辑完整 YAML",
                        cx,
                        |this, window, cx| {
                            this.set_saved_tab(SavedEditorTab::Yaml, window, cx);
                            this.saved_editor.more = true;
                            cx.notify();
                        },
                    )),
            )
            .child(if chains {
                self.clean_saved_chain_editor(cx)
            } else {
                self.clean_saved_groups(cx)
            })
            .into_any_element()
    }

    fn clean_saved_groups(&self, cx: &mut Context<Self>) -> AnyElement {
        let mut rows = v_flex().gap_2();
        if let Some(doc) = &self.saved_editor.document {
            for (group, enabled, position) in saved_objects(doc, "proxy-groups") {
                let name = text(&group, "name");
                let kind = saved_group_kind_label(&text(&group, "type"));
                let member_count = doc.members(&name).len();
                let rule_count = doc
                    .rules()
                    .iter()
                    .filter(|line| rule_target(line) == Some(name.as_str()))
                    .count();
                let summary = if rule_count > 0 {
                    format!("{kind} · {member_count} 节点 · {rule_count} 规则")
                } else {
                    format!("{kind} · {member_count} 节点")
                };
                let open = self.saved_editor.group_expanded
                    && self.saved_editor.selected_group.as_ref() == Some(&name);
                let select = name.clone();
                let edit = name.clone();
                let delete = name.clone();
                let toggle = name.clone();
                let mut header = h_flex()
                    .min_h(px(48.))
                    .min_w_0()
                    .gap_1()
                    .px_2()
                    .bg(rgb(if open { ACCENT_SOFT } else { SURFACE }));
                if let Some(index) = position {
                    header = header.child(self.saved_drag_handle(
                        "proxy-groups",
                        String::new(),
                        index,
                        name.clone(),
                    ));
                } else {
                    header = header.child(div().w(px(18.)).flex_shrink_0());
                }
                header = header
                    .child(
                        h_flex()
                            .id(format!("saved-expand-group-{name}"))
                            .flex_1()
                            .min_w_0()
                            .cursor_pointer()
                            .gap_2()
                            .child(
                                disclosure_icon(open)
                                    .with_size(px(14.))
                                    .text_color(rgb(MUTED)),
                            )
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .text_sm()
                                    .truncate()
                                    .font_weight(if open {
                                        FontWeight::SEMIBOLD
                                    } else {
                                        FontWeight::MEDIUM
                                    })
                                    .text_color(rgb(if enabled { TEXT } else { MUTED }))
                                    .child(name.clone()),
                            )
                            .child(
                                div()
                                    .flex_shrink_0()
                                    .text_size(px(11.))
                                    .text_color(rgb(MUTED))
                                    .child(summary),
                            )
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.saved_open_group(select.clone(), false, window, cx)
                            })),
                    )
                    .child(object_action(
                        format!("saved-edit-group-{name}"),
                        edit_icon(),
                        "编辑策略组",
                        cx,
                        move |this, window, cx| {
                            this.saved_open_group(edit.clone(), true, window, cx)
                        },
                    ))
                    .child(danger_action(
                        format!("saved-delete-group-{name}"),
                        delete_icon(),
                        "删除策略组",
                        cx,
                        move |this, window, cx| this.saved_delete_group(delete.clone(), window, cx),
                    ))
                    .child(
                        Switch::new(format!("saved-toggle-group-{name}"))
                            .small()
                            .checked(enabled)
                            .accessibility_label("启用策略组")
                            .on_change(cx.listener(move |this, checked, window, cx| {
                                this.saved_toggle_group(toggle.clone(), *checked, window, cx)
                            })),
                    );
                let card = v_flex()
                    .flex_shrink_0()
                    .min_w_0()
                    .rounded(px(8.))
                    .overflow_hidden()
                    .border_1()
                    .border_color(rgb(if open { ACCENT } else { BORDER_ITEM }))
                    .bg(rgb(SURFACE))
                    .when(!enabled && !open, |card| card.opacity(0.65))
                    .child(header)
                    .when(open, |card| {
                        card.child(self.clean_saved_group_detail(&name, cx))
                    });
                rows = if let Some(index) = position {
                    rows.child(self.saved_order_row(card, "proxy-groups", String::new(), index, cx))
                } else {
                    rows.child(card)
                };
            }
        }
        div()
            .id("saved-generator-groups")
            .flex_1()
            .min_h_0()
            .overflow_y_scrollbar()
            .child(rows)
            .into_any_element()
    }

    fn clean_saved_group_detail(&self, name: &str, cx: &mut Context<Self>) -> AnyElement {
        let Some(doc) = &self.saved_editor.document else {
            return div().into_any_element();
        };
        let members = doc.members(name);
        let mut panel = v_flex()
            .p_3()
            .gap_3()
            .border_t_1()
            .border_color(rgb(BORDER_STRONG))
            .bg(rgb(SURFACE_2));
        if self.saved_editor.editing_group {
            panel = panel.child(
                v_flex()
                    .gap_2()
                    .p_3()
                    .rounded(px(8.))
                    .border_1()
                    .border_color(rgb(BORDER))
                    .bg(rgb(SURFACE))
                    .child(
                        h_flex()
                            .gap_2()
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .child(Input::new(&self.saved_editor.group_name)),
                            )
                            .child(self.saved_choice(
                                "saved-group-kind",
                                saved_group_kind_label(&self.saved_editor.group_kind).into(),
                                vec![
                                    "手动选择".into(),
                                    "自动选择".into(),
                                    "故障转移".into(),
                                    "负载均衡".into(),
                                ],
                                cx,
                                |this, label, _, cx| {
                                    this.saved_editor.group_kind = match label.as_str() {
                                        "自动选择" => "url-test",
                                        "故障转移" => "fallback",
                                        "负载均衡" => "load-balance",
                                        _ => "select",
                                    }
                                    .into();
                                    cx.notify();
                                },
                            ))
                            .child(
                                Button::new("saved-save-group")
                                    .primary()
                                    .label("保存")
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        match this.autosave_saved_editor(window, cx) {
                                            Ok(()) => {
                                                this.saved_editor.editing_group = false;
                                                cx.notify();
                                            }
                                            Err(error) => {
                                                this.saved_config_error = Some(error);
                                                cx.notify();
                                            }
                                        }
                                    })),
                            ),
                    )
                    .when(self.saved_editor.group_kind != "select", |box_panel| {
                        box_panel.child(
                            h_flex()
                                .gap_2()
                                .child(
                                    div()
                                        .flex_1()
                                        .min_w_0()
                                        .child(Input::new(&self.saved_editor.group_url)),
                                )
                                .child(
                                    div()
                                        .w(px(110.))
                                        .child(Input::new(&self.saved_editor.group_interval)),
                                ),
                        )
                    }),
            );
        }
        let target = name.to_string();
        panel = panel.child(
            h_flex()
                .gap_2()
                .child(
                    Button::new(format!("saved-group-nodes-{name}"))
                        .outline()
                        .label("节点")
                        .when(!self.saved_editor.group_rules, |button| button.primary())
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.saved_editor.group_rules = false;
                            cx.notify();
                        })),
                )
                .child(
                    Button::new(format!("saved-group-rules-{name}"))
                        .outline()
                        .label("规则")
                        .when(self.saved_editor.group_rules, |button| button.primary())
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.saved_editor.group_rules = true;
                            cx.notify();
                        })),
                )
                .child(
                    Button::new(format!("saved-group-add-{name}"))
                        .outline()
                        .label(if self.saved_editor.group_rules {
                            "新增规则"
                        } else if self.saved_editor.member_picker {
                            "收起候选"
                        } else {
                            "添加节点"
                        })
                        .on_click(cx.listener(move |this, _, window, cx| {
                            if this.saved_editor.group_rules {
                                this.saved_begin_rule(target.clone(), None, window, cx);
                            } else {
                                this.saved_editor.member_picker = !this.saved_editor.member_picker;
                                cx.notify();
                            }
                        })),
                ),
        );
        let mut inner = v_flex().gap_2().min_w_0();
        if self.saved_editor.group_rules {
            let mut has_rules = false;
            for (index, line) in doc
                .rules()
                .iter()
                .enumerate()
                .filter(|(_, line)| rule_target(line) == Some(name))
            {
                has_rules = true;
                let target = name.to_string();
                let row = object_row()
                    .child(self.saved_drag_handle("rules", String::new(), index, line.clone()))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .text_sm()
                            .truncate()
                            .child(line.clone()),
                    )
                    .child(object_action(
                        format!("saved-edit-rule-{index}"),
                        edit_icon(),
                        "编辑规则",
                        cx,
                        move |this, window, cx| {
                            this.saved_begin_rule(target.clone(), Some(index), window, cx)
                        },
                    ))
                    .child(danger_action(
                        format!("saved-delete-rule-{index}"),
                        delete_icon(),
                        "删除规则",
                        cx,
                        move |this, window, cx| {
                            this.edit_saved_document(window, cx, |doc| {
                                let mut rules = doc.items("rules");
                                rules.remove(index);
                                doc.set_items("rules", rules);
                                Ok(())
                            })
                        },
                    ))
                    .child(
                        Switch::new(format!("saved-toggle-rule-{index}"))
                            .small()
                            .checked(true)
                            .on_change(cx.listener(move |this, checked, window, cx| {
                                this.edit_saved_document(window, cx, |doc| {
                                    doc.set_rule_enabled(index, *checked)
                                })
                            })),
                    );
                inner = inner.child(self.saved_order_row(row, "rules", String::new(), index, cx));
            }
            for (index, rule) in doc
                .metadata
                .disabled_rules
                .iter()
                .enumerate()
                .filter(|(_, rule)| rule_target(&rule.line) == Some(name))
            {
                has_rules = true;
                let target = name.to_string();
                inner = inner.child(
                    object_row()
                        .opacity(0.6)
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .text_sm()
                                .text_color(rgb(MUTED))
                                .truncate()
                                .child(rule.line.clone()),
                        )
                        .child(object_action(
                            format!("saved-edit-disabled-rule-{index}"),
                            edit_icon(),
                            "编辑规则",
                            cx,
                            move |this, window, cx| {
                                this.saved_begin_disabled_rule(target.clone(), index, window, cx)
                            },
                        ))
                        .child(danger_action(
                            format!("saved-delete-disabled-rule-{index}"),
                            delete_icon(),
                            "删除规则",
                            cx,
                            move |this, window, cx| {
                                this.edit_saved_document(window, cx, |doc| {
                                    doc.metadata.disabled_rules.remove(index);
                                    Ok(())
                                })
                            },
                        ))
                        .child(
                            Switch::new(format!("saved-toggle-disabled-rule-{index}"))
                                .small()
                                .checked(false)
                                .on_change(cx.listener(move |this, checked, window, cx| {
                                    this.edit_saved_document(window, cx, |doc| {
                                        doc.set_rule_enabled(index, *checked)
                                    })
                                })),
                        ),
                );
            }
            if !has_rules {
                inner = inner.child(
                    div()
                        .py_3()
                        .text_sm()
                        .text_color(rgb(MUTED))
                        .child("暂无规则"),
                );
            }
        } else {
            for (index, member) in members.iter().enumerate() {
                let group = name.to_string();
                let remove = member.clone();
                let edit = member.clone();
                let toggle_group = name.to_string();
                let toggle = member.clone();
                let row = object_row()
                    .child(self.saved_drag_handle(
                        "member",
                        name.to_string(),
                        index,
                        member.clone(),
                    ))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .text_sm()
                            .truncate()
                            .child(member.clone()),
                    )
                    .when(
                        doc.editable_node(member).is_some() || doc.editable_group(member).is_some(),
                        |row| {
                            row.child(object_action(
                                format!("saved-edit-member-{name}-{index}"),
                                edit_icon(),
                                "编辑成员",
                                cx,
                                move |this, window, cx| {
                                    if this
                                        .saved_editor
                                        .document
                                        .as_ref()
                                        .is_some_and(|doc| doc.editable_node(&edit).is_some())
                                    {
                                        this.saved_open_node(edit.clone(), window, cx);
                                    } else {
                                        this.saved_open_group(edit.clone(), true, window, cx);
                                    }
                                },
                            ))
                        },
                    )
                    .child(danger_action(
                        format!("saved-remove-member-{name}-{index}"),
                        delete_icon(),
                        "移除成员",
                        cx,
                        move |this, window, cx| {
                            this.edit_saved_document(window, cx, |doc| {
                                doc.member(&group, &remove, false)
                            })
                        },
                    ))
                    .child(
                        Switch::new(format!("saved-member-toggle-{name}-{index}"))
                            .small()
                            .checked(true)
                            .on_change(cx.listener(move |this, checked, window, cx| {
                                this.edit_saved_document(window, cx, |doc| {
                                    doc.member(&toggle_group, &toggle, *checked)
                                })
                            })),
                    );
                inner =
                    inner.child(self.saved_order_row(row, "member", name.to_string(), index, cx));
            }
            if self.saved_editor.member_picker {
                inner = inner.child(Input::new(&self.saved_editor.member_search).cleanable(true));
                let query = self
                    .saved_editor
                    .member_search
                    .read(cx)
                    .value()
                    .to_lowercase();
                for member in doc.targets().into_iter().filter(|target| {
                    target != name
                        && !members.contains(target)
                        && target.to_lowercase().contains(&query)
                }) {
                    let target = member.clone();
                    let group = name.to_string();
                    inner = inner.child(
                        object_row()
                            .opacity(0.7)
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .text_sm()
                                    .truncate()
                                    .text_color(rgb(MUTED))
                                    .child(member.clone()),
                            )
                            .child(
                                Switch::new(format!("saved-pick-member-{member}"))
                                    .small()
                                    .checked(false)
                                    .on_change(cx.listener(move |this, checked, window, cx| {
                                        this.edit_saved_document(window, cx, |doc| {
                                            doc.member(&group, &target, *checked)
                                        })
                                    })),
                            ),
                    );
                }
            }
            if let Some(providers) = doc
                .editable_group(name)
                .and_then(|group| group.get("use").cloned())
                .and_then(|value| value.as_sequence().cloned())
            {
                inner = inner.child(div().text_size(px(12.)).text_color(rgb(MUTED)).child(
                    format!(
                        "节点提供者：{}",
                        providers
                            .iter()
                            .filter_map(Value::as_str)
                            .collect::<Vec<_>>()
                            .join(" · ")
                    ),
                ));
            }
        }
        panel
            .child(
                div()
                    .id(format!("saved-group-inner-{name}"))
                    .max_h(px(300.))
                    .p_2()
                    .rounded(px(8.))
                    .border_1()
                    .border_color(rgb(BORDER_ITEM))
                    .bg(rgb(SURFACE_INSET))
                    .overflow_y_scrollbar()
                    .child(inner),
            )
            .into_any_element()
    }

    fn clean_saved_chain_editor(&self, cx: &mut Context<Self>) -> AnyElement {
        let nodes = self
            .saved_editor
            .document
            .as_ref()
            .map(|doc| doc.names("proxies"))
            .unwrap_or_default();
        let entry_choices: Vec<String> = nodes
            .iter()
            .filter(|name| Some(*name) != self.saved_editor.chain_edit.as_ref())
            .cloned()
            .collect();

        let mut rows = v_flex().gap_2().min_w_0();
        let mut chain_count = 0usize;
        if let Some(doc) = &self.saved_editor.document {
            for (node, enabled, _) in saved_objects(doc, "proxies") {
                let dialer = text(&node, "dialer-proxy");
                if dialer.is_empty() {
                    continue;
                }
                chain_count += 1;
                let name = text(&node, "name");
                let exit = doc
                    .chain_exit(&name)
                    .unwrap_or_else(|| text(&node, "server"));
                let edit = name.clone();
                let delete = name.clone();
                let toggle = name.clone();
                let group_id = SharedString::from(format!("saved-chain-row-{name}"));
                rows = rows.child(
                    object_row()
                        .id(format!("saved-chain-item-{name}"))
                        .group(group_id.clone())
                        .when(!enabled, |row| row.opacity(0.6))
                        .hover(|style| style.bg(rgb(SURFACE_2)))
                        .child(
                            h_flex()
                                .relative()
                                .flex_1()
                                .min_w_0()
                                .gap_2()
                                .child(
                                    v_flex()
                                        .flex_1()
                                        .min_w_0()
                                        .py_1()
                                        .child(
                                            h_flex()
                                                .gap_2()
                                                .min_w_0()
                                                .child(
                                                    div()
                                                        .flex_1()
                                                        .min_w_0()
                                                        .text_sm()
                                                        .truncate()
                                                        .text_color(rgb(if enabled {
                                                            TEXT
                                                        } else {
                                                            MUTED
                                                        }))
                                                        .child(name.clone()),
                                                )
                                                .child(micro_badge("链式", true)),
                                        )
                                        .child(
                                            div()
                                                .text_size(px(11.))
                                                .text_color(rgb(MUTED))
                                                .truncate()
                                                .child(format!("{dialer} → {exit}")),
                                        ),
                                )
                                .child(
                                    h_flex()
                                        .absolute()
                                        .right_0()
                                        .top_0()
                                        .bottom_0()
                                        .pl_2()
                                        .gap_1()
                                        .bg(rgb(SURFACE_2))
                                        .rounded(px(6.))
                                        .invisible()
                                        .group_hover(group_id, |style| style.visible())
                                        .child(object_action(
                                            format!("saved-chain-edit-{name}"),
                                            edit_icon(),
                                            "编辑链式节点",
                                            cx,
                                            move |this, window, cx| {
                                                this.saved_begin_chain(
                                                    Some(edit.clone()),
                                                    window,
                                                    cx,
                                                )
                                            },
                                        ))
                                        .child(danger_action(
                                            format!("saved-chain-delete-{name}"),
                                            delete_icon(),
                                            "删除链式节点",
                                            cx,
                                            move |this, window, cx| {
                                                this.edit_saved_document(window, cx, |doc| {
                                                    doc.remove_node(&delete)
                                                });
                                            },
                                        )),
                                ),
                        )
                        .child(
                            Switch::new(format!("saved-chain-switch-{name}"))
                                .small()
                                .checked(enabled)
                                .accessibility_label("启用链式节点")
                                .on_change(cx.listener(move |this, checked, window, cx| {
                                    this.edit_saved_document(window, cx, |doc| {
                                        doc.toggle_node(&toggle, *checked)
                                    })
                                })),
                        ),
                );
            }
        }
        if chain_count == 0 {
            rows = rows.child(
                div()
                    .py_6()
                    .flex()
                    .justify_center()
                    .text_sm()
                    .text_color(rgb(MUTED))
                    .child("在上方选择入口与出口节点，点击「保存」创建链式节点"),
            );
        }

        v_flex()
            .flex_1()
            .min_h_0()
            .min_w_0()
            .gap_3()
            .child(
                h_flex()
                    .gap_2()
                    .child(
                        div().flex_1().min_w_0().child(
                            self.saved_choice(
                                "saved-chain-entry",
                                self.saved_editor
                                    .chain_entry
                                    .clone()
                                    .unwrap_or_else(|| "入口选择".into()),
                                entry_choices,
                                cx,
                                |this, name, _, cx| {
                                    this.saved_editor.chain_entry = Some(name);
                                    cx.notify();
                                },
                            ),
                        ),
                    )
                    .child(object_action(
                        "saved-chain-swap",
                        swap_icon(),
                        "互换入口与出口",
                        cx,
                        |this, _, cx| {
                            std::mem::swap(
                                &mut this.saved_editor.chain_entry,
                                &mut this.saved_editor.chain_exit,
                            );
                            cx.notify();
                        },
                    ))
                    .child(
                        div().flex_1().min_w_0().child(
                            self.saved_choice(
                                "saved-chain-exit",
                                self.saved_editor
                                    .chain_exit
                                    .clone()
                                    .unwrap_or_else(|| "出口选择".into()),
                                nodes,
                                cx,
                                |this, name, _, cx| {
                                    this.saved_editor.chain_exit = Some(name);
                                    cx.notify();
                                },
                            ),
                        ),
                    ),
            )
            .child(
                div()
                    .id("saved-chain-list")
                    .flex_1()
                    .min_h_0()
                    .p_2()
                    .rounded(px(8.))
                    .border_1()
                    .border_color(rgb(BORDER))
                    .bg(rgb(SURFACE_INSET))
                    .overflow_y_scrollbar()
                    .child(rows),
            )
            .child(
                h_flex()
                    .gap_2()
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .child(Input::new(&self.saved_editor.chain_name).cleanable(true)),
                    )
                    .when(self.saved_editor.chain_edit.is_some(), |row| {
                        row.child(object_action(
                            "saved-chain-cancel",
                            close_icon(),
                            "取消编辑",
                            cx,
                            |this, window, cx| this.saved_begin_chain(None, window, cx),
                        ))
                    })
                    .child(
                        Button::new("saved-save-chain")
                            .primary()
                            .label(if self.saved_editor.chain_edit.is_some() {
                                "更新链式"
                            } else {
                                "保存"
                            })
                            .on_click(
                                cx.listener(|this, _, window, cx| {
                                    this.saved_save_chain(window, cx)
                                }),
                            ),
                    ),
            )
            .into_any_element()
    }

    fn clean_saved_node_dialog(&self, cx: &mut Context<Self>) -> AnyElement {
        let panel = v_flex()
            .w(px(440.))
            .max_w(relative(0.92))
            .max_h(relative(0.88))
            .p_4()
            .gap_3()
            .rounded(px(12.))
            .border_1()
            .border_color(rgb(BORDER))
            .bg(rgb(SURFACE))
            .child(
                h_flex()
                    .gap_2()
                    .child(
                        div()
                            .flex_1()
                            .text_size(px(16.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .child("编辑节点"),
                    )
                    .child(object_action(
                        "saved-node-close",
                        close_icon(),
                        "关闭",
                        cx,
                        |this, window, cx| {
                            if let Err(error) = this.flush_saved_fields(window, cx) {
                                this.saved_config_error = Some(error);
                            } else {
                                this.saved_editor.editing_node = false;
                            }
                            cx.notify();
                        },
                    )),
            )
            .child(field_label("名称"))
            .child(Input::new(&self.saved_editor.node_name))
            .child(field_label("服务器"))
            .child(Input::new(&self.saved_editor.node_server))
            .child(field_label("端口"))
            .child(Input::new(&self.saved_editor.node_port))
            .when_some(self.saved_config_error.clone(), |panel, error| {
                panel.child(div().text_sm().text_color(rgb(DANGER)).child(error))
            })
            .child(
                h_flex()
                    .justify_end()
                    .gap_2()
                    .child(
                        Button::new("saved-node-cancel")
                            .ghost()
                            .label("取消")
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.saved_editor.editing_node = false;
                                cx.notify();
                            })),
                    )
                    .child(
                        Button::new("saved-node-save")
                            .primary()
                            .label("保存")
                            .on_click(cx.listener(|this, _, window, cx| {
                                match this.autosave_saved_editor(window, cx) {
                                    Ok(()) => this.saved_editor.editing_node = false,
                                    Err(error) => this.saved_config_error = Some(error),
                                };
                                cx.notify();
                            })),
                    ),
            );
        modal_with_backdrop("saved-node-backdrop", panel, cx, |this, window, cx| {
            if let Err(error) = this.flush_saved_fields(window, cx) {
                this.saved_config_error = Some(error);
            } else {
                this.saved_editor.editing_node = false;
            }
            cx.notify();
        })
    }

    fn clean_saved_source_dialog(&self, cx: &mut Context<Self>) -> AnyElement {
        let mut rows = v_flex().gap_2();
        let sources = self.saved_library_nodes();
        for (id, name, nodes) in sources {
            let values = nodes
                .iter()
                .map(swiftsub_core::model::Proxy::to_value)
                .collect::<Vec<_>>();
            rows = rows.child(
                Button::new(format!("saved-import-source-{id}"))
                    .outline()
                    .w_full()
                    .label(format!("{name} · {} 个节点", nodes.len()))
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.saved_import_source(id, values.clone(), false, window, cx);
                        if this.saved_config_error.is_none() {
                            this.saved_editor.node_import = false;
                        }
                        cx.notify();
                    })),
            );
        }
        let panel = v_flex()
            .w(px(460.))
            .max_w(relative(0.92))
            .max_h(relative(0.85))
            .min_h_0()
            .p_4()
            .gap_3()
            .rounded(px(12.))
            .border_1()
            .border_color(rgb(BORDER))
            .bg(rgb(SURFACE))
            .child(
                h_flex()
                    .gap_2()
                    .child(
                        div()
                            .flex_1()
                            .text_size(px(16.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .child("从已导入来源添加节点"),
                    )
                    .child(object_action(
                        "saved-source-close",
                        close_icon(),
                        "关闭",
                        cx,
                        |this, _, cx| {
                            this.saved_editor.node_import = false;
                            cx.notify();
                        },
                    )),
            )
            .child(
                div()
                    .id("saved-source-list")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scrollbar()
                    .child(rows),
            )
            .child(
                Button::new("saved-import-yaml-file")
                    .ghost()
                    .label("导入新 YAML 配置文件")
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.import_yaml_config(window, cx);
                        this.saved_editor.node_import = false;
                    })),
            );
        modal_with_backdrop("saved-source-backdrop", panel, cx, |this, _, cx| {
            this.saved_editor.node_import = false;
            cx.notify();
        })
    }

    fn clean_saved_rule_dialog(&self, cx: &mut Context<Self>) -> AnyElement {
        let mut body = v_flex().gap_2().min_h_0();
        let target = self
            .saved_editor
            .rule_scope
            .clone()
            .unwrap_or("DIRECT".into());
        if self.saved_editor.rule_picker_kind == "自定义" {
            let targets = self
                .saved_editor
                .document
                .as_ref()
                .map(SavedDocument::targets)
                .unwrap_or_default();
            body = body
                .child(
                    h_flex()
                        .gap_2()
                        .child(field_label("目标"))
                        .child(self.saved_choice(
                            "saved-custom-rule-target",
                            target.clone(),
                            targets,
                            cx,
                            |this, name, window, cx| {
                                this.saved_editor.rule_scope = Some(name.clone());
                                if let Some(index) = this.saved_editor.rule_edit {
                                    this.saved_set_rule_target(index, name, window, cx);
                                    if this.saved_config_error.is_none()
                                        && let Some(line) = this
                                            .saved_editor
                                            .document
                                            .as_ref()
                                            .and_then(|doc| doc.rules().get(index).cloned())
                                    {
                                        this.saved_editor.new_rule.update(cx, |input, cx| {
                                            input.set_value(line, window, cx)
                                        });
                                    }
                                }
                                cx.notify();
                            },
                        )),
                )
                .child(Input::new(&self.saved_editor.new_rule).cleanable(true))
                .child(
                    div()
                        .text_size(px(12.))
                        .text_color(rgb(MUTED))
                        .child("输入规则或规则集链接。例如 DOMAIN-SUFFIX,example.com"),
                )
                .child(
                    h_flex().justify_end().child(
                        Button::new("saved-custom-rule-save")
                            .primary()
                            .label("保存")
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.saved_save_custom_rule(window, cx)
                            })),
                    ),
                );
        } else {
            let query = self.saved_editor.search.read(cx).value().to_lowercase();
            let mut choices = v_flex().gap_2();
            let mut bindings = Vec::new();
            for (_, source, items) in self.saved_rule_bindings() {
                for mut binding in items {
                    binding.target = target.clone();
                    if format!("{} {} {}", source, binding.name, binding.id)
                        .to_lowercase()
                        .contains(&query)
                    {
                        let id = binding.id.clone();
                        let active = self
                            .saved_editor
                            .document
                            .as_ref()
                            .and_then(|doc| doc.catalog_rule_target(&id))
                            .as_ref()
                            == Some(&target);
                        choices = choices.child(
                            object_row()
                                .child(
                                    div()
                                        .flex_1()
                                        .min_w_0()
                                        .text_sm()
                                        .truncate()
                                        .child(format!("{source} · {}", binding.name)),
                                )
                                .child(
                                    Switch::new(format!("saved-rule-pick-{id}"))
                                        .small()
                                        .checked(
                                            active || self.saved_editor.rule_picks.contains(&id),
                                        )
                                        .disabled(active)
                                        .on_change(cx.listener(move |this, checked, _, cx| {
                                            if *checked {
                                                this.saved_editor.rule_picks.insert(id.clone());
                                            } else {
                                                this.saved_editor.rule_picks.remove(&id);
                                            }
                                            cx.notify();
                                        })),
                                ),
                        );
                    }
                    bindings.push(binding);
                }
            }
            body = body
                .child(Input::new(&self.saved_editor.search).cleanable(true))
                .child(
                    div()
                        .id("saved-rule-catalog-list")
                        .h(px(300.))
                        .min_h_0()
                        .p_2()
                        .rounded(px(8.))
                        .border_1()
                        .border_color(rgb(BORDER))
                        .bg(rgb(SURFACE_INSET))
                        .overflow_y_scrollbar()
                        .child(choices),
                )
                .child(
                    h_flex().justify_end().child(
                        Button::new("saved-rule-picks-save")
                            .primary()
                            .label("添加所选规则")
                            .disabled(self.saved_editor.rule_picks.is_empty())
                            .on_click(cx.listener(move |this, _, window, cx| {
                                let picks = this.saved_editor.rule_picks.clone();
                                this.edit_saved_document(window, cx, |doc| {
                                    for binding in bindings
                                        .iter()
                                        .filter(|binding| picks.contains(&binding.id))
                                    {
                                        doc.set_scoped_binding(binding, true, Some(&target))?;
                                    }
                                    Ok(())
                                });
                                if this.saved_config_error.is_none() {
                                    this.saved_editor.picking_rules = false;
                                }
                                cx.notify();
                            })),
                    ),
                );
        }
        let panel = v_flex()
            .w(px(680.))
            .max_w(relative(0.92))
            .max_h(relative(0.88))
            .min_h_0()
            .p_4()
            .gap_3()
            .rounded(px(12.))
            .border_1()
            .border_color(rgb(BORDER))
            .bg(rgb(SURFACE))
            .child(
                h_flex()
                    .gap_2()
                    .child(
                        div()
                            .flex_1()
                            .text_size(px(16.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .child("新增 / 编辑规则"),
                    )
                    .child(object_action(
                        "saved-rule-picker-close",
                        close_icon(),
                        "关闭",
                        cx,
                        |this, _, cx| {
                            this.saved_editor.picking_rules = false;
                            cx.notify();
                        },
                    )),
            )
            .child(mode_tabs(
                "saved-rule-kind",
                &self.saved_editor.rule_picker_kind,
                &["规则库", "自定义"],
                cx,
                |this, kind, _, cx| {
                    this.saved_editor.rule_picker_kind = kind;
                    cx.notify();
                },
            ))
            .child(body)
            .when_some(self.saved_config_error.clone(), |panel, error| {
                panel.child(div().text_sm().text_color(rgb(DANGER)).child(error))
            });
        modal_with_backdrop("saved-rule-backdrop", panel, cx, |this, _, cx| {
            this.saved_editor.picking_rules = false;
            cx.notify();
        })
    }

    fn clean_saved_yaml_dialog(&self, cx: &mut Context<Self>) -> AnyElement {
        let panel = v_flex()
            .w(px(800.))
            .max_w(relative(0.92))
            .h(relative(0.88))
            .min_h_0()
            .p_4()
            .gap_3()
            .rounded(px(12.))
            .border_1()
            .border_color(rgb(BORDER))
            .bg(rgb(SURFACE))
            .child(
                h_flex()
                    .gap_2()
                    .child(
                        div()
                            .flex_1()
                            .text_size(px(16.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .child("完整 YAML"),
                    )
                    .child(object_action(
                        "saved-yaml-close",
                        close_icon(),
                        "关闭",
                        cx,
                        |this, window, cx| {
                            this.set_saved_tab(SavedEditorTab::Groups, window, cx);
                            if this.saved_config_error.is_none() {
                                this.saved_editor.more = false;
                            }
                            cx.notify();
                        },
                    )),
            )
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .child(Textarea::new(&self.saved_config_yaml).h_full()),
            )
            .when_some(self.saved_config_error.clone(), |panel, error| {
                panel.child(div().text_sm().text_color(rgb(DANGER)).child(error))
            })
            .child(
                h_flex()
                    .justify_end()
                    .gap_2()
                    .child(
                        Button::new("saved-yaml-export")
                            .outline()
                            .label("导出")
                            .on_click(cx.listener(|this, _, _, cx| this.export_saved_yaml(cx))),
                    )
                    .child(
                        Button::new("saved-yaml-save")
                            .primary()
                            .label("保存")
                            .on_click(cx.listener(|this, _, window, cx| {
                                if this.save_yaml_editor(false, window, cx) {
                                    this.set_saved_tab(SavedEditorTab::Groups, window, cx);
                                    this.saved_editor.more = false;
                                }
                                cx.notify();
                            })),
                    ),
            );
        modal_with_backdrop("saved-yaml-backdrop", panel, cx, |this, window, cx| {
            this.set_saved_tab(SavedEditorTab::Groups, window, cx);
            if this.saved_config_error.is_none() {
                this.saved_editor.more = false;
            }
            cx.notify();
        })
    }

    pub(super) fn saved_choice(
        &self,
        id: impl Into<ElementId>,
        label: String,
        options: Vec<String>,
        cx: &mut Context<Self>,
        handler: impl Fn(&mut Self, String, &mut Window, &mut Context<Self>) + 'static,
    ) -> AnyElement {
        let owner = cx.entity().downgrade();
        let handler = Rc::new(handler);
        Button::new(id)
            .outline()
            .min_w(px(116.))
            .label(format!("{label} ▾"))
            .dropdown_menu(move |mut menu, _, _| {
                menu = menu.max_h(px(320.)).scrollable(true);
                for value in &options {
                    let value = value.clone();
                    let owner = owner.clone();
                    let handler = handler.clone();
                    menu = menu.item(
                        PopupMenuItem::new(value.clone())
                            .checked(value == label)
                            .on_click(move |_, window, cx| {
                                let _ = owner.update(cx, |this, cx| {
                                    handler(this, value.clone(), window, cx)
                                });
                            }),
                    );
                }
                menu
            })
            .into_any_element()
    }

    fn saved_rule_bindings(&self) -> Vec<(String, String, Vec<RuleSetBinding>)> {
        let base = &self.options.rule_provider_base_url;
        let mut builtin = Vec::new();
        for module in catalog::MODULES
            .iter()
            .filter(|m| !matches!(m.kind, ModuleKind::Region(_)))
        {
            for spec in module.rule_sets {
                builtin.push(RuleSetBinding {
                    id: spec.id.into(),
                    name: format!("{} · {}", module.name, rule_set_label(spec.id)),
                    url: format!("{}/{}", base.trim_end_matches('/'), spec.path),
                    behavior: spec.behavior.as_str().into(),
                    format: "mrs".into(),
                    no_resolve: spec.no_resolve,
                    target: module.name.into(),
                });
            }
        }
        let mut result = vec![("builtin".into(), "内置 MetaCubeX".into(), builtin)];
        for source in rule_catalog::SOURCES {
            result.push((
                source.id.into(),
                source.name.into(),
                source
                    .entries
                    .iter()
                    .map(|entry| {
                        rule_catalog::binding_for(
                            source,
                            entry,
                            rule_catalog::mirror_from_base(base),
                            base,
                            &std::collections::HashSet::new(),
                        )
                    })
                    .collect(),
            ));
        }
        result
    }
}
