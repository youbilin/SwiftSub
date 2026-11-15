use super::*;

impl SwiftSubApp {
    pub(super) fn saved_configs_page(&self, cx: &mut Context<Self>) -> AnyElement {
        let mut list = v_flex().gap_2().p_2();
        for config in &self.saved_configs {
            let id = config.id.clone();
            let active = self.saved_config_id.as_ref() == Some(&id);
            list = list.child(
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
                            .child(format!(
                                "{} · {}",
                                format_timestamp(Some(config.updated_at)),
                                format_bytes(config.yaml.len() as u64)
                            )),
                    )
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.open_saved_config(id.clone(), window, cx)
                    })),
            );
        }
        if self.saved_configs.is_empty() {
            list = list.child(
                div()
                    .p_3()
                    .text_size(px(12.))
                    .text_color(rgb(MUTED))
                    .child("还没有保存的配置。生成后点击「保存新配置」，每次都会新增一份。"),
            );
        }
        let selected = self
            .saved_config_id
            .as_ref()
            .and_then(|id| self.saved_configs.iter().find(|c| &c.id == id));
        let editor = if let Some(config) = selected {
            let id = config.id.clone();
            v_flex()
                .size_full()
                .min_h_0()
                .gap_2()
                .p_3()
                .child(
                    h_flex()
                        .gap_2()
                        .flex_wrap()
                        .flex_shrink_0()
                        .child(
                            div()
                                .flex_1()
                                .min_w(px(180.))
                                .child(Input::new(&self.saved_config_name).cleanable(true)),
                        )
                        .child(
                            Button::new("saved-update")
                                .primary()
                                .label("更新此配置")
                                .on_click(cx.listener(|this, _, window, cx| {
                                    this.save_yaml_editor(false, window, cx)
                                })),
                        )
                        .child(
                            Button::new("saved-copy")
                                .ghost()
                                .label("另存一份")
                                .on_click(cx.listener(|this, _, window, cx| {
                                    this.save_yaml_editor(true, window, cx)
                                })),
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
                                .min_w_0()
                                .text_size(px(11.))
                                .text_color(rgb(if self.saved_config_dirty(cx) {
                                    WARNING
                                } else {
                                    MUTED
                                }))
                                .child(if self.saved_config_dirty(cx) {
                                    "有修改 · 点击更新写入当前文件，或另存一份"
                                } else {
                                    "已保存 · 重新生成不会覆盖这份 YAML"
                                }),
                        )
                        .when(config.source_json.is_some(), |row| {
                            row.child(
                                Button::new("saved-generator")
                                    .ghost()
                                    .small()
                                    .label("原生成设置")
                                    .on_click(cx.listener(move |this, _, window, cx| {
                                        this.load_saved_generator(id.clone(), window, cx)
                                    })),
                            )
                        })
                        .child(
                            Button::new("saved-url")
                                .ghost()
                                .small()
                                .label("复制订阅地址")
                                .on_click(
                                    cx.listener(|this, _, _, cx| this.copy_saved_config_url(cx)),
                                ),
                        )
                        .child(
                            Button::new("saved-copy-yaml")
                                .ghost()
                                .small()
                                .label("复制 YAML")
                                .on_click(cx.listener(|this, _, _, cx| {
                                    cx.write_to_clipboard(ClipboardItem::new_string(
                                        this.saved_config_yaml.read(cx).value().to_string(),
                                    ));
                                    this.show_toast("已复制 YAML".into(), cx);
                                })),
                        )
                        .child(
                            Button::new("saved-export")
                                .ghost()
                                .small()
                                .label("导出")
                                .on_click(cx.listener(|this, _, _, cx| this.export_saved_yaml(cx))),
                        )
                        .child(
                            Button::new("saved-delete")
                                .ghost()
                                .small()
                                .label(if self.saved_config_delete_armed {
                                    "确认删除此配置"
                                } else {
                                    "删除"
                                })
                                .on_click(cx.listener(|this, _, window, cx| {
                                    this.delete_yaml_config(window, cx)
                                })),
                        ),
                )
                .when_some(self.saved_config_error.clone(), |col, error| {
                    col.child(
                        div()
                            .text_size(px(12.))
                            .text_color(rgb(DANGER))
                            .child(error),
                    )
                })
                .child(
                    div()
                        .flex_1()
                        .min_h_0()
                        .child(Textarea::new(&self.saved_config_yaml).h_full()),
                )
                .child(
                    div()
                        .text_size(px(10.5))
                        .text_color(rgb(MUTED))
                        .truncate()
                        .child(
                            config
                                .path(&self.data_dir.join("configs"))
                                .display()
                                .to_string(),
                        ),
                )
                .into_any_element()
        } else {
            v_flex()
                .size_full()
                .justify_center()
                .items_center()
                .gap_3()
                .p_4()
                .child(div().text_size(px(18.)).child("我的配置"))
                .child(
                    div()
                        .text_size(px(12.))
                        .text_color(rgb(MUTED))
                        .child("保存生成的 YAML 后，在这里独立命名、编辑、更新和导出。"),
                )
                .child(
                    Button::new("saved-go-generator")
                        .primary()
                        .label("去生成配置")
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.set_page(Page::Generator, window, cx)
                        })),
                )
                .when_some(self.saved_config_error.clone(), |col, error| {
                    col.child(div().text_color(rgb(DANGER)).child(error))
                })
                .into_any_element()
        };
        v_flex()
            .size_full()
            .min_h_0()
            .p_3()
            .gap_2()
            .child(
                h_flex()
                    .justify_between()
                    .gap_2()
                    .child(
                        div()
                            .text_size(px(20.))
                            .child(format!("我的配置 · {}", self.saved_configs.len())),
                    )
                    .child(
                        div()
                            .text_size(px(11.))
                            .text_color(rgb(MUTED))
                            .child("每份配置独立保存为 YAML 文件"),
                    ),
            )
            .child(
                h_resizable("saved-config-layout")
                    .child(
                        resizable_panel()
                            .size(px(260.))
                            .size_range(px(220.)..px(420.))
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
                            .size_range(px(420.)..px(3000.))
                            .child(editor),
                    ),
            )
            .into_any_element()
    }
}
