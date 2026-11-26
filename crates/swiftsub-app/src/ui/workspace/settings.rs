use super::*;
use crate::app::workspace::dns::{DNS_FIELDS, DNS_FLAGS, DNS_LISTS, DNS_MAPS};
use crate::ui::layout::disclosure_icon;

const RULE_MIRRORS: [(&str, &str); 4] = [
    (
        "GitHub",
        "https://github.com/MetaCubeX/meta-rules-dat/raw/refs/heads/meta/geo",
    ),
    (
        "jsDelivr",
        "https://cdn.jsdelivr.net/gh/MetaCubeX/meta-rules-dat@meta/geo",
    ),
    (
        "Fastly",
        "https://fastly.jsdelivr.net/gh/MetaCubeX/meta-rules-dat@meta/geo",
    ),
    (
        "TestingCF",
        "https://testingcf.jsdelivr.net/gh/MetaCubeX/meta-rules-dat@meta/geo",
    ),
];

fn setting_row(label: &'static str, control: impl IntoElement) -> AnyElement {
    h_flex()
        .min_w_0()
        .gap_3()
        .child(
            div()
                .w(px(140.))
                .flex_shrink_0()
                .text_sm()
                .text_color(rgb(MUTED))
                .child(label),
        )
        .child(div().flex_1().min_w_0().child(control))
        .into_any_element()
}

fn compact_switch_row(
    id: impl Into<ElementId>,
    label: &'static str,
    checked: bool,
    on_change: impl Fn(&bool, &mut Window, &mut App) + 'static,
) -> AnyElement {
    h_flex()
        .min_w_0()
        .gap_2()
        .py_1()
        .child(
            Switch::new(id)
                .small()
                .checked(checked)
                .accessibility_label(label)
                .on_change(on_change),
        )
        .child(
            div()
                .flex_1()
                .min_w_0()
                .text_sm()
                .text_color(rgb(TEXT))
                .child(label),
        )
        .into_any_element()
}

fn settings_note(text: &'static str) -> AnyElement {
    div()
        .text_size(px(12.))
        .text_color(rgb(MUTED))
        .child(text)
        .into_any_element()
}

impl SwiftSubApp {
    fn generation_settings_error(&self, section: &'static str) -> AnyElement {
        div()
            .when(
                self.workspace.settings_error_section == Some(section),
                |row| {
                    row.when_some(self.template_error.clone(), |row, error| {
                        row.text_sm().text_color(rgb(DANGER)).child(error)
                    })
                },
            )
            .into_any_element()
    }

    pub(in crate::ui) fn workspace_settings_panel(&self, cx: &mut Context<Self>) -> AnyElement {
        v_flex()
            .w_full()
            .min_w_0()
            .gap_4()
            .when(self.workspace.settings_error_section.is_none(), |panel| {
                panel.when_some(self.template_error.clone(), |panel, error| {
                    panel.child(banner(error, DANGER))
                })
            })
            .child(self.generation_dns_settings(cx))
            .child(self.generation_health_settings(cx))
            .into_any_element()
    }

    fn generation_dns_settings(&self, cx: &mut Context<Self>) -> AnyElement {
        let editor = &self.workspace.dns_editor;
        let mut rows = vec![
            h_flex()
                .gap_6()
                .min_w_0()
                .child(
                    div().flex_1().min_w_0().child(setting_row(
                        "混合端口",
                        div()
                            .max_w(px(180.))
                            .child(Input::new(&self.workspace.mixed_port)),
                    )),
                )
                .child(div().flex_1().min_w_0().child(compact_switch_row(
                    "workspace-allow-lan",
                    "生成配置允许局域网代理（allow-lan）",
                    self.workspace.settings_allow_lan,
                    cx.listener(|this, checked, _, cx| {
                        this.workspace.settings_allow_lan = *checked;
                        cx.notify();
                    }),
                )))
                .into_any_element(),
        ];
        if editor.yaml_mode {
            rows.push(settings_note(
                "可直接粘贴完整的 dns: 和 hosts: 配置。其他 DNS 扩展字段也会保留。",
            ));
            rows.push(Textarea::new(&editor.yaml).h(px(460.)).into_any_element());
        } else {
            let dns_enabled = editor.flags.get("enable").copied().unwrap_or(false);
            rows.push(compact_switch_row(
                "workspace-dns-enable",
                "启用 DNS",
                dns_enabled,
                cx.listener(|this, checked, _, cx| {
                    this.workspace.dns_editor.flags.insert("enable", *checked);
                    cx.notify();
                }),
            ));
            rows.push(
                h_flex()
                    .gap_4()
                    .min_w_0()
                    .child(div().flex_1().min_w_0().child(setting_row(
                        "处理模式",
                        self.saved_choice(
                            "workspace-dns-mode",
                            editor.mode.clone(),
                            vec!["fake-ip".into(), "redir-host".into()],
                            cx,
                            |this, mode, _, cx| {
                                this.workspace.dns_editor.mode = mode;
                                cx.notify();
                            },
                        ),
                    )))
                    .child(div().flex_1().min_w_0().child(setting_row(
                        "过滤模式",
                        self.saved_choice(
                            "workspace-dns-filter-mode",
                            editor.filter_mode.clone(),
                            vec!["blacklist".into(), "whitelist".into(), "rule".into()],
                            cx,
                            |this, mode, _, cx| {
                                this.workspace.dns_editor.filter_mode = mode;
                                cx.notify();
                            },
                        ),
                    )))
                    .into_any_element(),
            );
            rows.push(
                h_flex()
                    .items_start()
                    .gap_4()
                    .min_w_0()
                    .child(
                        v_flex()
                            .flex_1()
                            .min_w_0()
                            .gap_2()
                            .child(field_label("解析 DNS（每行一项）"))
                            .child(Textarea::new(&editor.blocks["nameserver"]).h(px(92.))),
                    )
                    .child(
                        v_flex()
                            .flex_1()
                            .min_w_0()
                            .gap_2()
                            .child(field_label("默认 DNS（每行一项）"))
                            .child(Textarea::new(&editor.blocks["default-nameserver"]).h(px(92.))),
                    )
                    .into_any_element(),
            );

            let open = self.workspace.dns_advanced_open;
            rows.push(
                h_flex()
                    .id("toggle-dns-advanced")
                    .cursor_pointer()
                    .pt_2()
                    .pb_1()
                    .gap_2()
                    .border_t_1()
                    .border_color(rgb(BORDER))
                    .text_sm()
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(rgb(MUTED))
                    .hover(|style| style.text_color(rgb(TEXT)))
                    .child(disclosure_icon(open).with_size(px(14.)))
                    .child(if open {
                        "收起高级 DNS 与 Hosts"
                    } else {
                        "展开高级 DNS 与 Hosts（监听地址、Fake IP、策略与 Hosts）"
                    })
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.workspace.dns_advanced_open = !this.workspace.dns_advanced_open;
                        cx.notify();
                    }))
                    .into_any_element(),
            );

            if open {
                let extra_flags: Vec<_> = DNS_FLAGS
                    .iter()
                    .copied()
                    .filter(|(key, _)| *key != "enable")
                    .collect();
                for pair in extra_flags.chunks(2) {
                    let mut row = h_flex().gap_6().min_w_0();
                    for &(key, label) in pair {
                        row = row.child(div().flex_1().min_w_0().child(compact_switch_row(
                            format!("workspace-dns-{key}"),
                            label,
                            editor.flags.get(key).copied().unwrap_or(false),
                            cx.listener(move |this, checked, _, cx| {
                                this.workspace.dns_editor.flags.insert(key, *checked);
                                cx.notify();
                            }),
                        )));
                    }
                    if pair.len() == 1 {
                        row = row.child(div().flex_1());
                    }
                    rows.push(row.into_any_element());
                }
                for (key, label) in DNS_FIELDS {
                    rows.push(setting_row(
                        label,
                        Input::new(&editor.fields[key]).cleanable(true),
                    ));
                }
                rows.push(settings_note(
                    "DNS 地址和过滤项每行一项；留空表示空列表。解析策略与 Hosts 使用 YAML 对象。",
                ));
                let extra_lists: Vec<_> = DNS_LISTS
                    .iter()
                    .copied()
                    .filter(|(key, _)| !matches!(*key, "nameserver" | "default-nameserver"))
                    .collect();
                for pair in extra_lists.chunks(2) {
                    let mut row = h_flex().items_start().gap_4().min_w_0();
                    for &(key, label) in pair {
                        row = row.child(
                            v_flex()
                                .flex_1()
                                .min_w_0()
                                .gap_2()
                                .child(field_label(label))
                                .child(
                                    Textarea::new(&editor.blocks[key])
                                        .h(px(if key == "fake-ip-filter" { 132. } else { 84. })),
                                ),
                        );
                    }
                    rows.push(row.into_any_element());
                }
                for pair in DNS_MAPS.chunks(2) {
                    let mut row = h_flex().items_start().gap_4().min_w_0();
                    for &(key, label) in pair {
                        row = row.child(
                            v_flex()
                                .flex_1()
                                .min_w_0()
                                .gap_2()
                                .child(field_label(label))
                                .child(Textarea::new(&editor.blocks[key]).h(px(120.))),
                        );
                    }
                    rows.push(row.into_any_element());
                }
            }
        }
        rows.push(settings_note(
            "保存为 DNS 默认值；生成页启用 DNS 覆写后使用。",
        ));
        if self.workspace.settings_error_section == Some("基础与 DNS")
            && self.template_error.is_some()
        {
            rows.insert(0, self.generation_settings_error("基础与 DNS"));
        }
        settings_section(
            "基础与 DNS",
            rows,
            h_flex()
                .gap_2()
                .child(
                    Button::new("workspace-dns-form")
                        .ghost()
                        .label("表单")
                        .when(!editor.yaml_mode, |button| button.primary())
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.workspace_set_dns_editor(false, window, cx)
                        })),
                )
                .child(
                    Button::new("workspace-dns-yaml")
                        .ghost()
                        .label("YAML")
                        .when(editor.yaml_mode, |button| button.primary())
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.workspace_set_dns_editor(true, window, cx)
                        })),
                )
                .child(
                    Button::new("workspace-save-settings")
                        .primary()
                        .label("保存")
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.workspace.settings_error_section = Some("基础与 DNS");
                            this.workspace_save_settings(window, cx);
                        })),
                ),
        )
    }

    fn generation_health_settings(&self, cx: &mut Context<Self>) -> AnyElement {
        settings_section(
            "全局测速",
            vec![
                settings_note("自动选择、故障转移和负载均衡组默认使用这些参数。"),
                setting_row("测速地址", Input::new(&self.workspace.global_test_url)),
                setting_row(
                    "周期（秒）",
                    div()
                        .max_w(px(180.))
                        .child(Input::new(&self.workspace.global_test_interval)),
                ),
                setting_row(
                    "容差（毫秒）",
                    div()
                        .max_w(px(180.))
                        .child(Input::new(&self.workspace.global_test_tolerance)),
                ),
                self.generation_settings_error("全局测速"),
            ],
            Button::new("workspace-save-health")
                .primary()
                .label("保存")
                .on_click(cx.listener(|this, _, window, cx| {
                    this.workspace.settings_error_section = Some("全局测速");
                    this.workspace_save_health(window, cx);
                })),
        )
    }

    pub(in crate::ui) fn generation_rule_source_settings(
        &self,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let value = self.rule_provider_input.read(cx).value();
        let mirror = RULE_MIRRORS
            .iter()
            .find(|(_, url)| value.trim_end_matches('/') == *url)
            .map(|(label, _)| *label)
            .unwrap_or("自定义");
        let mut mirrors: Vec<_> = RULE_MIRRORS
            .iter()
            .map(|(label, _)| label.to_string())
            .collect();
        mirrors.push("自定义".into());
        div()
            .w_full()
            .child(settings_section(
                "规则来源",
                vec![
                    setting_row(
                        "下载镜像",
                        self.saved_choice(
                            "settings-rule-mirror",
                            mirror.into(),
                            mirrors,
                            cx,
                            |this, label, window, cx| {
                                this.workspace.settings_error_section = Some("规则来源");
                                if let Some((_, url)) =
                                    RULE_MIRRORS.iter().find(|(name, _)| *name == label)
                                {
                                    this.apply_rule_provider_preset(url, window, cx);
                                } else {
                                    this.rule_provider_input
                                        .read(cx)
                                        .focus_handle(cx)
                                        .focus(window, cx);
                                }
                            },
                        ),
                    ),
                    setting_row(
                        "基础地址",
                        Input::new(&self.rule_provider_input).cleanable(true),
                    ),
                    settings_note(
                        "选择镜像会立即保存。自定义地址填写后点击保存；当前组绑定哪些规则仍在生成页编辑。",
                    ),
                    div()
                        .text_size(px(12.))
                        .text_color(rgb(MUTED))
                        .child(
                            self.workspace
                                .rules_updated_at
                                .map(|at| format!("内置规则更新于 {}", format_timestamp(Some(at))))
                                .unwrap_or_else(|| {
                                    "可更新已使用的内置规则；自定义正文会保留。".into()
                                }),
                        )
                        .into_any_element(),
                    div()
                        .when_some(self.workspace.rule_updates_error.clone(), |row, error| {
                            row.text_sm().text_color(rgb(DANGER)).child(error)
                        })
                        .into_any_element(),
                    self.generation_settings_error("规则来源"),
                ],
                h_flex()
                    .gap_2()
                    .child(
                        Button::new("update-builtin-rules")
                            .outline()
                            .label(if self.workspace.rule_updates_busy {
                                "更新中…"
                            } else {
                                "更新内置规则"
                            })
                            .disabled(self.workspace.rule_updates_busy)
                            .on_click(cx.listener(|this, _, _, cx| this.refresh_builtin_rules(cx))),
                    )
                    .child(
                        Button::new("save-rule-provider")
                            .primary()
                            .label("保存")
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.workspace.settings_error_section = Some("规则来源");
                                this.save_rule_provider(window, cx);
                            })),
                    ),
            ))
            .into_any_element()
    }
}
