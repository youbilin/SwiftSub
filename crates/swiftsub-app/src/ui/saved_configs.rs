use super::*;
use crate::app::SavedEditorTab;
use crate::saved_document::{rule_parts, rule_target, text};
use gpui_kit::component::menu::{DropdownMenu as _, PopupMenuItem};
use std::rc::Rc;
use swiftsub_core::generator::RuleSetBinding;

#[derive(Clone)]
struct SavedDrag {
    kind: &'static str,
    index: usize,
    name: String,
    group: String,
}

impl SwiftSubApp {
    fn saved_choice(
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
            .ghost()
            .small()
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
    fn saved_tab(
        &self,
        label: &'static str,
        tab: SavedEditorTab,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        Button::new(format!("saved-tab-{label}"))
            .ghost()
            .small()
            .label(label)
            .when(self.saved_editor.tab == tab, |b| b.primary())
            .on_click(cx.listener(move |this, _, w, cx| this.set_saved_tab(tab, w, cx)))
            .into_any_element()
    }
    pub(super) fn saved_configs_page(&self, cx: &mut Context<Self>) -> AnyElement {
        let config_query = self
            .saved_editor
            .config_search
            .read(cx)
            .value()
            .to_lowercase();
        let visible_count = self
            .saved_configs
            .iter()
            .filter(|config| {
                config_query.is_empty() || config.name.to_lowercase().contains(&config_query)
            })
            .count();
        let mut list = v_flex()
            .gap_2()
            .p_2()
            .child(Input::new(&self.saved_editor.config_search).cleanable(true));
        for config in self.saved_configs.iter().filter(|config| {
            config_query.is_empty() || config.name.to_lowercase().contains(&config_query)
        }) {
            let id = config.id.clone();
            let active = self.saved_config_id.as_ref() == Some(&id);
            list =
                list.child(
                    v_flex()
                        .id(format!("saved-{id}"))
                        .cursor_pointer()
                        .gap_1()
                        .p_3()
                        .rounded(px(8.))
                        .border_1()
                        .border_color(rgb(if active { ACCENT } else { BORDER }))
                        .bg(rgb(if active { ACCENT_SOFT } else { SURFACE }))
                        .child(
                            div()
                                .text_size(px(13.))
                                .truncate()
                                .child(config.name.clone()),
                        )
                        .child(
                            div()
                                .text_size(px(10.5))
                                .text_color(rgb(MUTED))
                                .child(format_timestamp(Some(config.updated_at))),
                        )
                        .on_click(cx.listener(move |this, _, w, cx| {
                            this.open_saved_config(id.clone(), w, cx)
                        })),
                );
        }
        if visible_count == 0 {
            list = list.child(
                div()
                    .p_2()
                    .text_size(px(11.))
                    .text_color(rgb(MUTED))
                    .child("没有匹配的配置"),
            );
        }
        let selected = self
            .saved_config_id
            .as_ref()
            .and_then(|id| self.saved_configs.iter().find(|c| &c.id == id));
        let editor =
            if selected.is_some() {
                let content = match self.saved_editor.tab {
                    SavedEditorTab::Groups => self.saved_groups_view(cx),
                    SavedEditorTab::Nodes => self.saved_nodes_view(cx),
                    SavedEditorTab::Rules => self.saved_rules_view(cx),
                    SavedEditorTab::Settings => self.saved_settings_view(cx),
                    SavedEditorTab::Yaml => v_flex()
                        .size_full()
                        .min_h_0()
                        .gap_2()
                        .child(
                            h_flex()
                                .justify_between()
                                .child(field_label("高级 YAML"))
                                .child(
                                    Button::new("saved-yaml-copy")
                                        .ghost()
                                        .small()
                                        .label("复制 YAML")
                                        .on_click(cx.listener(|this, _, _, cx| {
                                            cx.write_to_clipboard(ClipboardItem::new_string(
                                                this.saved_config_yaml.read(cx).value().to_string(),
                                            ));
                                            this.show_toast("已复制 YAML".into(), cx);
                                        })),
                                ),
                        )
                        .child(
                            div()
                                .flex_1()
                                .min_h_0()
                                .child(Textarea::new(&self.saved_config_yaml).h_full()),
                        )
                        .into_any_element(),
                };
                v_flex()
                    .size_full()
                    .min_h_0()
                    .min_w_0()
                    .p_3()
                    .gap_2()
                    .child(
                        h_flex()
                            .gap_2()
                            .flex_wrap()
                            .flex_shrink_0()
                            .child(
                                div()
                                    .flex_1()
                                    .min_w(px(140.))
                                    .child(Input::new(&self.saved_config_name).cleanable(true)),
                            )
                            .child(
                                div()
                                    .text_size(px(10.5))
                                    .text_color(rgb(if self.saved_config_error.is_some() {
                                        DANGER
                                    } else {
                                        MUTED
                                    }))
                                    .child(if self.saved_config_error.is_some() {
                                        "待修正"
                                    } else if self.saved_config_dirty(cx)
                                        || self.saved_editor.pending
                                    {
                                        "保存中…"
                                    } else {
                                        "已自动保存"
                                    }),
                            )
                            .child(
                                Button::new("saved-clash-verge")
                                    .primary()
                                    .small()
                                    .label("导入 Clash Verge")
                                    .on_click(cx.listener(|this, _, w, cx| {
                                        this.import_saved_to_clash(w, cx)
                                    })),
                            )
                            .child(
                                Button::new("saved-more")
                                    .ghost()
                                    .small()
                                    .label("更多")
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.saved_editor.more = !this.saved_editor.more;
                                        cx.notify();
                                    })),
                            ),
                    )
                    .when(self.saved_editor.more, |col| {
                        col.child(
                            h_flex()
                                .gap_1()
                                .flex_wrap()
                                .flex_shrink_0()
                                .child(
                                    Button::new("saved-copy")
                                        .ghost()
                                        .small()
                                        .label("另存一份")
                                        .on_click(cx.listener(|this, _, w, cx| {
                                            this.save_yaml_editor(true, w, cx);
                                        })),
                                )
                                .child(
                                    Button::new("saved-url")
                                        .ghost()
                                        .small()
                                        .label("复制订阅地址")
                                        .on_click(cx.listener(|this, _, _, cx| {
                                            this.copy_saved_config_url(cx)
                                        })),
                                )
                                .child(
                                    Button::new("saved-export")
                                        .ghost()
                                        .small()
                                        .label("导出 YAML")
                                        .on_click(cx.listener(|this, _, w, cx| {
                                            if let Err(e) = this.autosave_saved_editor(w, cx) {
                                                this.saved_config_error = Some(e);
                                                cx.notify();
                                                return;
                                            }
                                            this.export_saved_yaml(cx);
                                        })),
                                )
                                .child(
                                    Button::new("saved-delete")
                                        .ghost()
                                        .small()
                                        .label(if self.saved_config_delete_armed {
                                            "确认删除"
                                        } else {
                                            "删除配置"
                                        })
                                        .on_click(cx.listener(|this, _, w, cx| {
                                            this.delete_yaml_config(w, cx)
                                        })),
                                ),
                        )
                    })
                    .child(
                        h_flex()
                            .gap_1()
                            .flex_wrap()
                            .flex_shrink_0()
                            .child(self.saved_tab("节点", SavedEditorTab::Nodes, cx))
                            .child(self.saved_tab("策略组", SavedEditorTab::Groups, cx))
                            .child(self.saved_tab("分流规则", SavedEditorTab::Rules, cx))
                            .child(self.saved_tab("Mihomo", SavedEditorTab::Settings, cx))
                            .child(self.saved_tab("YAML", SavedEditorTab::Yaml, cx))
                            .child(div().flex_1())
                            .when(self.saved_editor.tab != SavedEditorTab::Yaml, |bar| {
                                bar.child(
                                    Button::new("saved-undo")
                                        .ghost()
                                        .small()
                                        .label("撤销")
                                        .disabled(self.saved_editor.undo.is_empty())
                                        .on_click(cx.listener(|this, _, w, cx| {
                                            this.saved_undo(false, w, cx)
                                        })),
                                )
                                .child(
                                    Button::new("saved-redo")
                                        .ghost()
                                        .small()
                                        .label("重做")
                                        .disabled(self.saved_editor.redo.is_empty())
                                        .on_click(cx.listener(|this, _, w, cx| {
                                            this.saved_undo(true, w, cx)
                                        })),
                                )
                            }),
                    )
                    .when_some(self.saved_config_error.clone(), |col, error| {
                        col.child(
                            div()
                                .text_size(px(11.))
                                .text_color(rgb(DANGER))
                                .child(error),
                        )
                    })
                    .child(div().flex_1().min_h_0().min_w_0().child(content))
                    .into_any_element()
            } else {
                v_flex()
                    .size_full()
                    .justify_center()
                    .items_center()
                    .gap_3()
                    .child(div().text_size(px(18.)).child("暂无配置"))
                    .child(
                        Button::new("saved-empty-generate")
                            .primary()
                            .label("生成配置")
                            .on_click(
                                cx.listener(|this, _, w, cx| this.set_page(Page::Generator, w, cx)),
                            ),
                    )
                    .child(
                        Button::new("saved-empty-import")
                            .ghost()
                            .label("导入 YAML")
                            .on_click(cx.listener(|this, _, w, cx| this.import_yaml_config(w, cx))),
                    )
                    .into_any_element()
            };
        let body = if self.saved_editor.list_collapsed {
            editor
        } else {
            h_resizable("saved-config-layout")
                .child(
                    resizable_panel()
                        .size(px(220.))
                        .size_range(px(170.)..px(360.))
                        .child(
                            div()
                                .id("saved-config-list")
                                .size_full()
                                .overflow_y_scrollbar()
                                .child(list),
                        ),
                )
                .child(
                    resizable_panel()
                        .size_range(px(450.)..px(3000.))
                        .child(editor),
                )
                .into_any_element()
        };
        v_flex()
            .size_full()
            .min_h_0()
            .p_3()
            .gap_2()
            .child(
                h_flex()
                    .gap_2()
                    .flex_shrink_0()
                    .child(
                        Button::new("saved-list-toggle")
                            .ghost()
                            .small()
                            .label(if self.saved_editor.list_collapsed {
                                "展开列表"
                            } else {
                                "收起列表"
                            })
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.saved_editor.list_collapsed =
                                    !this.saved_editor.list_collapsed;
                                cx.notify();
                            })),
                    )
                    .child(div().text_size(px(18.)).child(format!(
                        "我的配置 · {visible_count}/{}",
                        self.saved_configs.len()
                    )))
                    .child(div().flex_1())
                    .child(
                        Button::new("library-new")
                            .ghost()
                            .small()
                            .label("生成配置")
                            .on_click(
                                cx.listener(|this, _, w, cx| this.set_page(Page::Generator, w, cx)),
                            ),
                    )
                    .child(
                        Button::new("library-import-yaml")
                            .ghost()
                            .small()
                            .label("导入 YAML")
                            .on_click(cx.listener(|this, _, w, cx| this.import_yaml_config(w, cx))),
                    ),
            )
            .child(div().flex_1().min_h_0().child(body))
            .into_any_element()
    }
    fn saved_groups_view(&self, cx: &mut Context<Self>) -> AnyElement {
        let Some(doc) = &self.saved_editor.document else {
            return div().child("请先修正 YAML").into_any_element();
        };
        let query = self.saved_editor.search.read(cx).value().to_lowercase();
        let mut groups = doc.items("proxy-groups");
        groups.sort_by_key(|g| match text(g, "name").as_str() {
            "节点选择" => 0,
            "自动选择" => 1,
            "漏网之鱼" => 2,
            _ => 3,
        });
        let mut list = v_flex().gap_1();
        for group in groups {
            let name = text(&group, "name");
            if !name.to_lowercase().contains(&query) {
                continue;
            }
            let active = self.saved_editor.selected_group.as_ref() == Some(&name);
            let selected = name.clone();
            let toggle_name = name.clone();
            let required = ["节点选择", "自动选择", "漏网之鱼"].contains(&name.as_str())
                || doc.rules().iter().any(|r| {
                    matches!(rule_parts(r).first(), Some(&"MATCH" | &"FINAL"))
                        && rule_target(r) == Some(name.as_str())
                });
            let members = doc.members(&name).len();
            let rules = doc
                .rules()
                .iter()
                .filter(|r| rule_target(r) == Some(name.as_str()))
                .count();
            list = list.child(
                v_flex()
                    .id(format!("saved-group-{name}"))
                    .p_2()
                    .gap_1()
                    .rounded(px(7.))
                    .cursor_pointer()
                    .bg(rgb(if active { ACCENT_SOFT } else { SURFACE }))
                    .hover(|s| s.bg(rgb(SURFACE_2)))
                    .child(
                        h_flex()
                            .gap_1()
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .text_size(px(12.5))
                                    .truncate()
                                    .child(name.clone()),
                            )
                            .child(
                                Switch::new(format!("saved-group-enabled-{name}"))
                                    .small()
                                    .checked(true)
                                    .disabled(required)
                                    .on_change(cx.listener(move |this, checked, w, cx| {
                                        this.saved_toggle_group(
                                            toggle_name.clone(),
                                            *checked,
                                            w,
                                            cx,
                                        )
                                    })),
                            ),
                    )
                    .child(
                        div()
                            .text_size(px(10.5))
                            .text_color(rgb(MUTED))
                            .child(format!(
                                "{} · {members} 成员 · {rules} 规则",
                                text(&group, "type")
                            )),
                    )
                    .on_click(cx.listener(move |this, _, w, cx| {
                        this.select_saved_group(selected.clone(), w, cx)
                    })),
            );
        }
        for disabled in &doc.metadata.disabled_groups {
            let name = text(&disabled.value, "name");
            if !name.to_lowercase().contains(&query) {
                continue;
            }
            let target = name.clone();
            list = list.child(
                h_flex()
                    .gap_1()
                    .p_2()
                    .rounded(px(7.))
                    .bg(rgb(SURFACE))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .text_size(px(12.))
                            .text_color(rgb(MUTED))
                            .truncate()
                            .child(name.clone()),
                    )
                    .child(
                        Switch::new(format!("saved-group-disabled-{name}"))
                            .small()
                            .checked(false)
                            .on_change(cx.listener(move |this, checked, w, cx| {
                                this.saved_toggle_group(target.clone(), *checked, w, cx)
                            })),
                    ),
            );
        }
        let list = v_flex()
            .size_full()
            .min_h_0()
            .gap_2()
            .child(
                h_flex()
                    .gap_1()
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .child(Input::new(&self.saved_editor.search).cleanable(true)),
                    )
                    .child(
                        Button::new("saved-add-group")
                            .ghost()
                            .small()
                            .label("+")
                            .tooltip("新建策略组")
                            .on_click(
                                cx.listener(|this, _, w, cx| this.saved_add_group(false, w, cx)),
                            ),
                    ),
            )
            .child(
                div()
                    .id("saved-groups-list")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scrollbar()
                    .child(list),
            );
        let detail = self.saved_group_detail(cx);
        h_resizable("saved-groups-layout")
            .child(
                resizable_panel()
                    .size(px(190.))
                    .size_range(px(150.)..px(320.))
                    .child(list),
            )
            .child(
                resizable_panel()
                    .size_range(px(260.)..px(2600.))
                    .child(detail),
            )
            .into_any_element()
    }
    fn saved_group_detail(&self, cx: &mut Context<Self>) -> AnyElement {
        let Some(doc) = &self.saved_editor.document else {
            return div().into_any_element();
        };
        let Some(name) = self
            .saved_editor
            .selected_group
            .clone()
            .filter(|n| doc.group(n).is_some())
        else {
            return div().p_3().child("选择或新建策略组").into_any_element();
        };
        let group = doc.group(&name).unwrap();
        let members = doc.members(&name);
        let rule_count = doc
            .rules()
            .iter()
            .filter(|r| rule_target(r) == Some(name.as_str()))
            .count();
        let current_kind = self.saved_editor.group_kind.clone();
        let group_kind = self.saved_choice(
            "saved-group-kind",
            current_kind.clone(),
            ["select", "url-test", "fallback", "load-balance"]
                .map(str::to_string)
                .to_vec(),
            cx,
            |this, kind, _, cx| {
                this.saved_editor.group_kind = kind;
                cx.notify();
            },
        );
        let mut selected = v_flex().gap_1();
        for (i, member) in members.iter().enumerate() {
            let member = member.clone();
            let drag_name = member.clone();
            let group = name.clone();
            let group_move = name.clone();
            let group_remove = name.clone();
            let remove = member.clone();
            selected = selected.child(
                div()
                    .id(format!("saved-member-{name}-{i}"))
                    .cursor_pointer()
                    .child(
                        h_flex()
                            .gap_1()
                            .px_2()
                            .py_1()
                            .rounded(px(5.))
                            .bg(rgb(SURFACE_2))
                            .child(div().text_size(px(11.)).text_color(rgb(MUTED)).child("≡"))
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .text_size(px(12.))
                                    .truncate()
                                    .child(member),
                            )
                            .child(
                                Button::new(format!("saved-member-up-{i}"))
                                    .ghost()
                                    .small()
                                    .label("↑")
                                    .disabled(i == 0)
                                    .on_click(cx.listener(move |this, _, w, cx| {
                                        let group = group.clone();
                                        this.edit_saved_document(w, cx, |d| {
                                            d.move_member(&group, i, i.saturating_sub(1))
                                        });
                                    })),
                            )
                            .child(
                                Button::new(format!("saved-member-remove-{i}"))
                                    .ghost()
                                    .small()
                                    .label("×")
                                    .on_click(cx.listener(move |this, _, w, cx| {
                                        let group = group_remove.clone();
                                        let member = remove.clone();
                                        this.edit_saved_document(w, cx, |d| {
                                            d.member(&group, &member, false)
                                        });
                                    })),
                            ),
                    )
                    .on_drag(
                        SavedDrag {
                            kind: "member",
                            index: i,
                            name: drag_name,
                            group: name.clone(),
                        },
                        |drag, _, _, cx| {
                            cx.new(|_| NodeDragPreview {
                                name: drag.name.clone(),
                            })
                        },
                    )
                    .on_drop(cx.listener(move |this, drag: &SavedDrag, w, cx| {
                        if drag.kind == "member" && drag.group == group_move {
                            let group = group_move.clone();
                            let from = drag.index;
                            this.edit_saved_document(w, cx, |d| d.move_member(&group, from, i));
                        }
                    })),
            );
        }
        let query = self
            .saved_editor
            .member_search
            .read(cx)
            .value()
            .to_lowercase();
        let mut available = v_flex().gap_1();
        for target in doc
            .targets()
            .into_iter()
            .filter(|t| t != &name && !members.contains(t) && t.to_lowercase().contains(&query))
        {
            let group = name.clone();
            let member = target.clone();
            available = available.child(
                h_flex()
                    .gap_2()
                    .min_h(px(29.))
                    .child(
                        Switch::new(format!("saved-add-member-{target}"))
                            .small()
                            .checked(false)
                            .on_change(cx.listener(move |this, checked, w, cx| {
                                let group = group.clone();
                                let member = member.clone();
                                this.edit_saved_document(w, cx, |d| {
                                    d.member(&group, &member, *checked)
                                });
                            })),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .text_size(px(12.))
                            .truncate()
                            .child(target),
                    ),
            );
        }
        let bulk_group = name.clone();
        let clear_group = name.clone();
        let reset = name.clone();
        v_flex()
            .size_full()
            .min_h_0()
            .pl_3()
            .gap_2()
            .child(
                h_flex()
                    .gap_1()
                    .flex_wrap()
                    .flex_shrink_0()
                    .child(
                        Button::new("saved-group-rules")
                            .primary()
                            .small()
                            .label(format!("分流规则 {rule_count}"))
                            .on_click(cx.listener(|this, _, w, cx| this.saved_group_rules(w, cx))),
                    )
                    .child(
                        Button::new("saved-duplicate-group")
                            .ghost()
                            .small()
                            .label("复制组")
                            .on_click(
                                cx.listener(|this, _, w, cx| this.saved_add_group(true, w, cx)),
                            ),
                    )
                    .when(catalog::MODULES.iter().any(|m| m.name == name), |row| {
                        row.child(
                            Button::new("saved-reset-group")
                                .ghost()
                                .small()
                                .label("恢复默认")
                                .on_click(cx.listener(move |this, _, w, cx| {
                                    let name = reset.clone();
                                    this.saved_reset_group(name, w, cx);
                                })),
                        )
                    })
                    .when(
                        !["节点选择", "自动选择", "漏网之鱼"].contains(&name.as_str()),
                        |row| {
                            row.child(
                                Button::new("saved-remove-group")
                                    .ghost()
                                    .small()
                                    .label("删除组")
                                    .on_click(
                                        cx.listener(|this, _, w, cx| {
                                            this.saved_delete_group(w, cx)
                                        }),
                                    ),
                            )
                        },
                    ),
            )
            .child(
                h_flex()
                    .gap_2()
                    .flex_wrap()
                    .flex_shrink_0()
                    .child(
                        div()
                            .flex_1()
                            .min_w(px(130.))
                            .child(Input::new(&self.saved_editor.group_name).cleanable(true)),
                    )
                    .child(group_kind),
            )
            .when(current_kind != "select", |col| {
                col.child(
                    h_flex()
                        .gap_2()
                        .flex_shrink_0()
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .child(Input::new(&self.saved_editor.group_url)),
                        )
                        .child(
                            div().w(px(100.)).child(
                                Input::new(&self.saved_editor.group_interval).cleanable(true),
                            ),
                        ),
                )
            })
            .when_some(
                group
                    .get("use")
                    .and_then(serde_yaml_ng::Value::as_sequence)
                    .filter(|v| !v.is_empty()),
                |col, providers| {
                    col.child(
                        div()
                            .text_size(px(11.))
                            .text_color(rgb(MUTED))
                            .child(format!(
                                "节点提供者：{}",
                                providers
                                    .iter()
                                    .filter_map(serde_yaml_ng::Value::as_str)
                                    .collect::<Vec<_>>()
                                    .join(" · ")
                            )),
                    )
                },
            )
            .child(
                div()
                    .id("saved-group-detail-scroll")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scrollbar()
                    .child(
                        v_flex()
                            .gap_2()
                            .child(
                                h_flex()
                                    .gap_1()
                                    .child(
                                        div()
                                            .text_size(px(12.))
                                            .child(format!("成员 · {}", members.len())),
                                    )
                                    .child(div().flex_1())
                                    .child(
                                        Button::new("saved-members-all")
                                            .ghost()
                                            .small()
                                            .label("全部节点")
                                            .on_click(cx.listener(move |this, _, w, cx| {
                                                let name = bulk_group.clone();
                                                this.edit_saved_document(w, cx, |doc| {
                                                    for member in doc.names("proxies") {
                                                        doc.member(&name, &member, true)?;
                                                    }
                                                    Ok(())
                                                });
                                            })),
                                    )
                                    .child(
                                        Button::new("saved-members-direct")
                                            .ghost()
                                            .small()
                                            .label("仅直连")
                                            .on_click(cx.listener(move |this, _, w, cx| {
                                                let name = clear_group.clone();
                                                this.edit_saved_document(w, cx, |d| {
                                                    d.member(&name, "DIRECT", true)?;
                                                    for member in d.members(&name) {
                                                        if member != "DIRECT" {
                                                            d.member(&name, &member, false)?;
                                                        }
                                                    }
                                                    Ok(())
                                                });
                                            })),
                                    ),
                            )
                            .child(selected)
                            .child(Input::new(&self.saved_editor.member_search).cleanable(true))
                            .child(available),
                    ),
            )
            .into_any_element()
    }
    fn saved_nodes_view(&self, cx: &mut Context<Self>) -> AnyElement {
        let Some(doc) = &self.saved_editor.document else {
            return div().into_any_element();
        };
        let query = self.saved_editor.search.read(cx).value().to_lowercase();
        let mut rows = v_flex().gap_2();
        if self.saved_editor.node_import {
            for (source_id, source_name, nodes) in self.saved_library_nodes() {
                let batch = nodes.iter().map(|p| p.to_value()).collect::<Vec<_>>();
                let linked = doc
                    .metadata
                    .node_sources
                    .iter()
                    .any(|n| n.source_id == source_id);
                let open = self.saved_editor.node_sources_open.contains(&source_id);
                rows = rows.child(
                    h_flex()
                        .gap_2()
                        .child(
                            Button::new(format!("saved-node-source-expand-{source_id}"))
                                .ghost()
                                .small()
                                .label(format!("{} {}", if open { "▾" } else { "▸" }, source_name))
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    if !this.saved_editor.node_sources_open.insert(source_id) {
                                        this.saved_editor.node_sources_open.remove(&source_id);
                                    }
                                    cx.notify();
                                })),
                        )
                        .child(div().flex_1())
                        .child(
                            Button::new(format!("saved-source-refresh-{source_id}"))
                                .ghost()
                                .small()
                                .label(if self.refreshing.contains(&source_id) {
                                    "刷新中…"
                                } else {
                                    "刷新订阅"
                                })
                                .disabled(self.refreshing.contains(&source_id))
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.refresh_subscription(source_id, cx)
                                })),
                        )
                        .child(
                            Button::new(format!("saved-import-source-{source_id}"))
                                .ghost()
                                .small()
                                .label(if linked {
                                    "同步到此配置"
                                } else {
                                    "加入此订阅"
                                })
                                .disabled(batch.is_empty())
                                .on_click(cx.listener(move |this, _, w, cx| {
                                    this.saved_import_source(source_id, batch.clone(), true, w, cx)
                                })),
                        ),
                );
                if open {
                    for (i, node) in nodes.into_iter().enumerate() {
                        if !node.name().to_lowercase().contains(&query) {
                            continue;
                        }
                        let present =
                            doc.metadata.node_sources.iter().any(|n| {
                                n.source_id == source_id && n.original_name == node.name()
                            });
                        let value = node.to_value();
                        rows = rows.child(
                            h_flex()
                                .gap_2()
                                .min_h(px(30.))
                                .child(
                                    Button::new(format!("saved-import-node-{source_id}-{i}"))
                                        .ghost()
                                        .small()
                                        .label(if present { "已加入" } else { "加入" })
                                        .disabled(present)
                                        .on_click(cx.listener(move |this, _, w, cx| {
                                            this.saved_import_source(
                                                source_id,
                                                vec![value.clone()],
                                                false,
                                                w,
                                                cx,
                                            )
                                        })),
                                )
                                .child(
                                    div()
                                        .flex_1()
                                        .text_size(px(12.))
                                        .truncate()
                                        .child(node.name().to_string()),
                                ),
                        );
                    }
                }
            }
        } else {
            for (i, node) in doc.items("proxies").iter().enumerate() {
                let name = text(node, "name");
                if !name.to_lowercase().contains(&query) {
                    continue;
                }
                let selected = self.saved_editor.selected_node.as_ref() == Some(&name);
                let edit_name = name.clone();
                let enabled_name = name.clone();
                let title = name.clone();
                let count = doc.items("proxies").len();
                rows = rows.child(
                    div()
                        .id(format!("saved-node-drag-{i}"))
                        .cursor_pointer()
                        .child(
                            h_flex()
                                .gap_2()
                                .px_2()
                                .py_2()
                                .rounded(px(7.))
                                .bg(rgb(if selected { ACCENT_SOFT } else { SURFACE }))
                                .child(div().text_color(rgb(MUTED)).child("≡"))
                                .child(
                                    v_flex()
                                        .flex_1()
                                        .min_w_0()
                                        .gap_1()
                                        .child(
                                            div()
                                                .text_size(px(12.5))
                                                .truncate()
                                                .child(name.clone()),
                                        )
                                        .child(
                                            div()
                                                .text_size(px(10.5))
                                                .text_color(rgb(MUTED))
                                                .child(text(node, "type")),
                                        ),
                                )
                                .child(
                                    Button::new(format!("saved-node-edit-{i}"))
                                        .ghost()
                                        .small()
                                        .label("编辑")
                                        .on_click(cx.listener(move |this, _, w, cx| {
                                            this.select_saved_node(edit_name.clone(), w, cx)
                                        })),
                                )
                                .child(
                                    Button::new(format!("saved-node-up-{i}"))
                                        .ghost()
                                        .small()
                                        .label("↑")
                                        .disabled(i == 0)
                                        .on_click(cx.listener(move |this, _, w, cx| {
                                            this.edit_saved_document(w, cx, |d| {
                                                d.move_item("proxies", i, i.saturating_sub(1))
                                            })
                                        })),
                                )
                                .child(
                                    Button::new(format!("saved-node-down-{i}"))
                                        .ghost()
                                        .small()
                                        .label("↓")
                                        .disabled(i + 1 == count)
                                        .on_click(cx.listener(move |this, _, w, cx| {
                                            this.edit_saved_document(w, cx, |d| {
                                                d.move_item("proxies", i, i + 1)
                                            })
                                        })),
                                )
                                .child(
                                    Switch::new(format!("saved-node-enabled-{i}"))
                                        .small()
                                        .checked(true)
                                        .on_change(cx.listener(move |this, checked, w, cx| {
                                            let name = enabled_name.clone();
                                            this.edit_saved_document(w, cx, |d| {
                                                d.toggle_node(&name, *checked)
                                            });
                                        })),
                                ),
                        )
                        .on_drag(
                            SavedDrag {
                                kind: "node",
                                index: i,
                                name: title,
                                group: String::new(),
                            },
                            |drag, _, _, cx| {
                                cx.new(|_| NodeDragPreview {
                                    name: drag.name.clone(),
                                })
                            },
                        )
                        .on_drop(cx.listener(move |this, drag: &SavedDrag, w, cx| {
                            if drag.kind == "node" {
                                let from = drag.index;
                                this.edit_saved_document(w, cx, |d| {
                                    d.move_item("proxies", from, i)
                                });
                            }
                        })),
                );
            }
            for (i, node) in doc.metadata.disabled_nodes.iter().enumerate() {
                let name = text(&node.value, "name");
                if !name.to_lowercase().contains(&query) {
                    continue;
                }
                let target = name.clone();
                rows = rows.child(
                    h_flex()
                        .gap_2()
                        .px_2()
                        .py_2()
                        .child(
                            div()
                                .flex_1()
                                .text_size(px(12.))
                                .text_color(rgb(MUTED))
                                .child(name),
                        )
                        .child(
                            Switch::new(format!("saved-disabled-node-{i}"))
                                .small()
                                .checked(false)
                                .on_change(cx.listener(move |this, checked, w, cx| {
                                    let name = target.clone();
                                    this.edit_saved_document(w, cx, |d| {
                                        d.toggle_node(&name, *checked)
                                    });
                                })),
                        ),
                );
            }
            if let Some(providers) = doc
                .root
                .get("proxy-providers")
                .and_then(serde_yaml_ng::Value::as_mapping)
            {
                for name in providers.keys().filter_map(serde_yaml_ng::Value::as_str) {
                    rows = rows.child(
                        h_flex()
                            .gap_2()
                            .p_2()
                            .child(tag("提供者", ACCENT))
                            .child(div().text_size(px(12.)).child(name.to_string())),
                    );
                }
            }
            if doc.items("proxies").is_empty() {
                rows = rows.child(
                    div()
                        .p_3()
                        .text_size(px(12.))
                        .text_color(rgb(MUTED))
                        .child("从订阅加入节点，或使用配置中的节点提供者"),
                );
            }
        }
        let import = self.saved_editor.node_import;
        v_flex()
            .size_full()
            .min_h_0()
            .gap_2()
            .child(
                h_flex()
                    .gap_2()
                    .flex_wrap()
                    .flex_shrink_0()
                    .child(
                        div()
                            .flex_1()
                            .min_w(px(130.))
                            .child(Input::new(&self.saved_editor.search).cleanable(true)),
                    )
                    .child(
                        Button::new("saved-node-import-toggle")
                            .ghost()
                            .small()
                            .label(if import {
                                "返回节点"
                            } else {
                                "从订阅加入"
                            })
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.saved_editor.node_import = !this.saved_editor.node_import;
                                cx.notify();
                            })),
                    )
                    .when(!import, |bar| {
                        bar.child(
                            Button::new("saved-enable-all-nodes")
                                .ghost()
                                .small()
                                .label("全部启用")
                                .on_click(cx.listener(|this, _, w, cx| {
                                    this.edit_saved_document(w, cx, |d| {
                                        let names = d
                                            .metadata
                                            .disabled_nodes
                                            .iter()
                                            .map(|n| text(&n.value, "name"))
                                            .collect::<Vec<_>>();
                                        for name in names {
                                            d.toggle_node(&name, true)?;
                                        }
                                        Ok(())
                                    })
                                })),
                        )
                        .child(
                            Button::new("saved-disable-all-nodes")
                                .ghost()
                                .small()
                                .label("全部停用")
                                .on_click(cx.listener(|this, _, w, cx| {
                                    this.edit_saved_document(w, cx, |d| {
                                        for name in d.names("proxies") {
                                            d.toggle_node(&name, false)?;
                                        }
                                        Ok(())
                                    })
                                })),
                        )
                    }),
            )
            .when(
                !import && self.saved_editor.selected_node.is_some(),
                |col| {
                    col.child(
                        v_flex()
                            .gap_2()
                            .p_2()
                            .bg(rgb(SURFACE_2))
                            .rounded(px(7.))
                            .flex_shrink_0()
                            .child(
                                h_flex()
                                    .gap_2()
                                    .flex_wrap()
                                    .child(div().flex_1().min_w(px(120.)).child(
                                        Input::new(&self.saved_editor.node_name).cleanable(true),
                                    ))
                                    .child(div().flex_1().min_w(px(120.)).child(
                                        Input::new(&self.saved_editor.node_server).cleanable(true),
                                    ))
                                    .child(div().w(px(85.)).child(
                                        Input::new(&self.saved_editor.node_port).cleanable(true),
                                    ))
                                    .child(
                                        Button::new("saved-node-close-detail")
                                            .ghost()
                                            .small()
                                            .label("收起")
                                            .on_click(cx.listener(|this, _, w, cx| {
                                                if let Err(e) = this.flush_saved_fields(w, cx) {
                                                    this.saved_config_error = Some(e);
                                                } else {
                                                    this.saved_editor.selected_node = None;
                                                }
                                                cx.notify();
                                            })),
                                    ),
                            ),
                    )
                },
            )
            .child(
                div()
                    .id("saved-nodes-scroll")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scrollbar()
                    .child(rows),
            )
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
    fn saved_rules_view(&self, cx: &mut Context<Self>) -> AnyElement {
        let Some(doc) = &self.saved_editor.document else {
            return div().into_any_element();
        };
        let query = self.saved_editor.search.read(cx).value().to_lowercase();
        let scope = self.saved_editor.rule_scope.clone();
        let targets = doc.targets();
        let mut scopes = vec!["全部策略组".into()];
        scopes.extend(doc.names("proxy-groups"));
        scopes.extend(["DIRECT".into(), "REJECT".into()]);
        let scope_picker = self.saved_choice(
            "saved-rule-scope",
            scope.clone().unwrap_or("全部策略组".into()),
            scopes,
            cx,
            |this, target, _, cx| {
                this.saved_editor.rule_scope = if target == "全部策略组" {
                    None
                } else {
                    Some(target)
                };
                this.saved_editor.rule_current_group_only = false;
                cx.notify();
            },
        );
        let current_group = self.saved_editor.selected_group.clone();
        let current_group_only = self.saved_editor.rule_current_group_only;
        let current_group_button = Button::new("saved-rule-current-group")
            .ghost()
            .small()
            .label(if current_group_only {
                "当前组 ✓"
            } else {
                "当前组"
            })
            .disabled(current_group.is_none())
            .on_click(cx.listener(move |this, _, _, cx| {
                if current_group_only {
                    this.saved_editor.rule_scope = None;
                    this.saved_editor.rule_current_group_only = false;
                } else if let Some(group) = this.saved_editor.selected_group.clone() {
                    this.saved_editor.rule_scope = Some(group);
                    this.saved_editor.rule_current_group_only = true;
                }
                cx.notify();
            }));
        let only_enabled = self.saved_editor.rule_only_enabled;
        let enabled_filter = h_flex()
            .gap_1()
            .items_center()
            .child(
                Switch::new("saved-rule-only-enabled")
                    .small()
                    .checked(only_enabled)
                    .on_change(cx.listener(|this, checked, _, cx| {
                        this.saved_editor.rule_only_enabled = *checked;
                        cx.notify();
                    })),
            )
            .child(div().text_size(px(11.)).child("只看已启用"));
        let mut rows = v_flex().gap_2();
        for (source_id, label, bindings) in self.saved_rule_bindings() {
            let count = bindings
                .iter()
                .filter(|b| {
                    doc.catalog_rule_target(&b.id)
                        .is_some_and(|t| scope.as_ref().is_none_or(|s| s == &t))
                })
                .count();
            let total = bindings.len();
            let forced_open = !query.is_empty();
            let open = forced_open || self.saved_editor.sources_open.contains(&source_id);
            let expand_id = source_id.clone();
            let bindings = bindings
                .into_iter()
                .map(|mut b| {
                    if let Some(scope) = &scope {
                        b.target = scope.clone();
                    }
                    b
                })
                .collect::<Vec<_>>();
            let all = bindings.clone();
            rows = rows.child(
                h_flex()
                    .gap_2()
                    .pt_2()
                    .child(
                        Button::new(format!("saved-source-expand-{source_id}"))
                            .ghost()
                            .small()
                            .label(format!(
                                "{} {label} {count}/{total}",
                                if open { "▾" } else { "▸" }
                            ))
                            .on_click(cx.listener(move |this, _, _, cx| {
                                if !this.saved_editor.sources_open.insert(expand_id.clone()) {
                                    this.saved_editor.sources_open.remove(&expand_id);
                                }
                                cx.notify();
                            })),
                    )
                    .child(div().flex_1())
                    .child(
                        Switch::new(format!("saved-source-toggle-{source_id}"))
                            .small()
                            .checked(count > 0)
                            .on_change(cx.listener(move |this, checked, w, cx| {
                                let bindings = all.clone();
                                let scope = this.saved_editor.rule_scope.clone();
                                this.edit_saved_document(w, cx, |doc| {
                                    for b in &bindings {
                                        doc.set_scoped_binding(b, *checked, scope.as_deref())?;
                                    }
                                    Ok(())
                                });
                            })),
                    ),
            );
            if open {
                for binding in bindings {
                    if !format!("{} {} {}", binding.name, binding.id, binding.target)
                        .to_lowercase()
                        .contains(&query)
                    {
                        continue;
                    }
                    let actual_target = doc.catalog_rule_target(&binding.id);
                    let active = actual_target
                        .as_ref()
                        .is_some_and(|t| scope.as_ref().is_none_or(|s| s == t));
                    if only_enabled && !active {
                        continue;
                    }
                    let chosen = binding.clone();
                    let name = binding.name.clone();
                    rows = rows.child(
                        h_flex()
                            .gap_2()
                            .px_2()
                            .py_1()
                            .rounded(px(5.))
                            .bg(rgb(SURFACE))
                            .child(
                                Switch::new(format!("saved-catalog-{}", binding.id))
                                    .small()
                                    .checked(active)
                                    .on_change(cx.listener(move |this, checked, w, cx| {
                                        this.saved_catalog_binding(chosen.clone(), *checked, w, cx)
                                    })),
                            )
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .text_size(px(12.))
                                    .truncate()
                                    .child(name),
                            )
                            .child(
                                div()
                                    .text_size(px(10.5))
                                    .text_color(rgb(MUTED))
                                    .child(actual_target.unwrap_or(binding.target)),
                            ),
                    );
                }
            }
        }
        rows = rows.child(
            h_flex()
                .justify_between()
                .pt_3()
                .child(section_label("匹配顺序"))
                .child(
                    div()
                        .text_size(px(11.))
                        .text_color(rgb(MUTED))
                        .child(format!("{} 条", doc.rules().len())),
                ),
        );
        for (index, line) in doc.rules().iter().enumerate() {
            let target = rule_target(line).map(str::to_string);
            if scope.as_ref().is_some_and(|s| target.as_ref() != Some(s))
                || !line.to_lowercase().contains(&query)
            {
                continue;
            }
            let parts = rule_parts(line);
            let final_rule = matches!(parts.first(), Some(&"MATCH" | &"FINAL"));
            let label = if final_rule {
                "最终兜底".into()
            } else {
                parts[..parts
                    .len()
                    .saturating_sub(1 + usize::from(parts.last() == Some(&"no-resolve")))]
                    .join(",")
            };
            let target_picker = if let Some(target) = target {
                self.saved_choice(
                    format!("saved-rule-target-{index}"),
                    target,
                    targets.clone(),
                    cx,
                    move |this, target, w, cx| this.saved_set_rule_target(index, target, w, cx),
                )
            } else {
                div()
                    .text_size(px(11.))
                    .child("高级规则")
                    .into_any_element()
            };
            let total = doc.rules().len();
            let drag_name = line.clone();
            rows = rows.child(
                div()
                    .id(format!("saved-rule-row-{index}"))
                    .cursor_pointer()
                    .child(
                        h_flex()
                            .gap_1()
                            .px_2()
                            .py_1()
                            .rounded(px(6.))
                            .bg(rgb(if final_rule { ACCENT_SOFT } else { SURFACE }))
                            .child(div().text_size(px(10.)).text_color(rgb(MUTED)).child(
                                if final_rule {
                                    "兜底".into()
                                } else {
                                    format!("≡ {}", index + 1)
                                },
                            ))
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .text_size(px(11.5))
                                    .truncate()
                                    .child(label),
                            )
                            .child(target_picker)
                            .when(!final_rule, |row| {
                                row.child(
                                    Button::new(format!("saved-rule-up-{index}"))
                                        .ghost()
                                        .small()
                                        .label("↑")
                                        .disabled(index == 0)
                                        .on_click(cx.listener(move |this, _, w, cx| {
                                            this.edit_saved_document(w, cx, |d| {
                                                d.move_item("rules", index, index.saturating_sub(1))
                                            })
                                        })),
                                )
                                .child(
                                    Button::new(format!("saved-rule-down-{index}"))
                                        .ghost()
                                        .small()
                                        .label("↓")
                                        .disabled(index + 2 >= total)
                                        .on_click(cx.listener(move |this, _, w, cx| {
                                            this.edit_saved_document(w, cx, |d| {
                                                d.move_item("rules", index, index + 1)
                                            })
                                        })),
                                )
                            })
                            .child(
                                Switch::new(format!("saved-rule-enabled-{index}"))
                                    .small()
                                    .checked(true)
                                    .on_change(cx.listener(move |this, checked, w, cx| {
                                        this.edit_saved_document(w, cx, |d| {
                                            d.set_rule_enabled(index, *checked)
                                        })
                                    })),
                            ),
                    )
                    .on_drag(
                        SavedDrag {
                            kind: "rule",
                            index,
                            name: drag_name,
                            group: String::new(),
                        },
                        |drag, _, _, cx| {
                            cx.new(|_| NodeDragPreview {
                                name: drag.name.clone(),
                            })
                        },
                    )
                    .on_drop(cx.listener(move |this, drag: &SavedDrag, w, cx| {
                        if drag.kind == "rule" && !final_rule {
                            let from = drag.index;
                            this.edit_saved_document(w, cx, |d| d.move_item("rules", from, index));
                        }
                    })),
            );
        }
        for (index, rule) in doc.metadata.disabled_rules.iter().enumerate() {
            if only_enabled {
                continue;
            }
            if scope
                .as_ref()
                .is_some_and(|s| rule_target(&rule.line) != Some(s.as_str()))
                || !rule.line.to_lowercase().contains(&query)
            {
                continue;
            }
            rows = rows.child(
                h_flex()
                    .gap_2()
                    .px_2()
                    .py_1()
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .text_size(px(11.5))
                            .text_color(rgb(MUTED))
                            .truncate()
                            .child(rule.line.clone()),
                    )
                    .child(
                        Button::new(format!("saved-disabled-rule-delete-{index}"))
                            .ghost()
                            .small()
                            .label("删除")
                            .on_click(cx.listener(move |this, _, w, cx| {
                                this.edit_saved_document(w, cx, |d| {
                                    d.metadata.disabled_rules.remove(index);
                                    Ok(())
                                })
                            })),
                    )
                    .child(
                        Switch::new(format!("saved-disabled-rule-{index}"))
                            .small()
                            .checked(false)
                            .on_change(cx.listener(move |this, checked, w, cx| {
                                this.edit_saved_document(w, cx, |d| {
                                    d.set_rule_enabled(index, *checked)
                                })
                            })),
                    ),
            );
        }
        v_flex()
            .size_full()
            .min_h_0()
            .gap_2()
            .child(
                h_flex()
                    .gap_2()
                    .flex_wrap()
                    .flex_shrink_0()
                    .child(
                        div()
                            .flex_1()
                            .min_w(px(150.))
                            .child(Input::new(&self.saved_editor.search).cleanable(true)),
                    )
                    .child(current_group_button)
                    .child(enabled_filter)
                    .child(scope_picker),
            )
            .child(
                div()
                    .id("saved-rules-scroll")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scrollbar()
                    .child(rows),
            )
            .child(
                h_flex()
                    .gap_2()
                    .flex_shrink_0()
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .child(Input::new(&self.saved_editor.new_rule).cleanable(true)),
                    )
                    .child(
                        Button::new("saved-add-rule")
                            .primary()
                            .small()
                            .label("添加规则")
                            .on_click(cx.listener(|this, _, w, cx| this.saved_add_rule(w, cx))),
                    ),
            )
            .into_any_element()
    }
    fn saved_settings_view(&self, cx: &mut Context<Self>) -> AnyElement {
        v_flex()
            .child(field_label("Mihomo 运行参数"))
            .size_full()
            .min_h_0()
            .gap_3()
            .child(field_label("Mihomo 混合监听端口"))
            .child(
                div()
                    .w(px(180.))
                    .child(Input::new(&self.saved_editor.mixed_port).cleanable(true)),
            )
            .child(
                h_flex()
                    .gap_2()
                    .child(field_label("Mihomo 路由模式"))
                    .child(self.saved_choice(
                        "saved-mode",
                        self.saved_editor.mode.clone(),
                        ["rule", "global", "direct"].map(str::to_string).to_vec(),
                        cx,
                        |this, value, _, cx| {
                            this.saved_editor.mode = value;
                            cx.notify();
                        },
                    )),
            )
            .child(
                h_flex()
                    .gap_2()
                    .child(
                        Switch::new("saved-allow-lan")
                            .small()
                            .checked(self.saved_editor.allow_lan)
                            .on_change(cx.listener(|this, checked, _, cx| {
                                this.saved_editor.allow_lan = *checked;
                                cx.notify();
                            })),
                    )
                    .child(field_label("允许局域网访问")),
            )
            .child(
                h_flex()
                    .gap_2()
                    .child(
                        Switch::new("saved-dns")
                            .small()
                            .checked(self.saved_editor.dns)
                            .on_change(cx.listener(|this, checked, _, cx| {
                                this.saved_editor.dns = *checked;
                                cx.notify();
                            })),
                    )
                    .child(field_label("DNS 开关")),
            )
            .into_any_element()
    }
}
