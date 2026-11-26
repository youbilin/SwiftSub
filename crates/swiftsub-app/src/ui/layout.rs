use super::*;
use crate::app::{WorkspaceNodeKind, chain_order_key};
use gpui_kit::component::menu::{DropdownMenu as _, PopupMenuItem};
use gpui_kit::component::tooltip::Tooltip;

#[derive(Clone, Copy, PartialEq, Eq)]
enum OrderScope {
    Nodes,
    Groups,
    Members(usize),
}

#[derive(Clone)]
struct ObjectOrderDrag {
    scope: OrderScope,
    key: String,
    name: String,
    position: usize,
}

pub(super) fn grip_icon() -> gpui_kit::component::Icon {
    gpui_kit::component::Icon::default().data(br#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="currentColor"><circle cx="9" cy="6" r="1.5"/><circle cx="15" cy="6" r="1.5"/><circle cx="9" cy="12" r="1.5"/><circle cx="15" cy="12" r="1.5"/><circle cx="9" cy="18" r="1.5"/><circle cx="15" cy="18" r="1.5"/></svg>"#)
}

pub(super) fn disclosure_icon(open: bool) -> gpui_kit::component::Icon {
    gpui_kit::component::Icon::default().data(if open {
        br#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round"><path d="m6 9 6 6 6-6"/></svg>"#
    } else {
        br#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round"><path d="m9 6 6 6-6 6"/></svg>"#
    })
}

pub(super) fn configuration_icon() -> gpui_kit::component::Icon {
    gpui_kit::component::Icon::default().data(br#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.7" stroke-linecap="round" stroke-linejoin="round"><path d="M14 3H6a2 2 0 0 0-2 2v14a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V9Z"/><path d="M14 3v6h6M8 13h8M8 17h5"/></svg>"#)
}

pub(super) fn link_icon() -> gpui_kit::component::Icon {
    gpui_kit::component::Icon::default().data(br#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round"><path d="m10 13 4-4M8 15l-1 1a4 4 0 0 1-6-6l4-4a4 4 0 0 1 6 0M16 9l1-1a4 4 0 0 1 6 6l-4 4a4 4 0 0 1-6 0"/></svg>"#)
}

pub(super) fn lan_icon() -> gpui_kit::component::Icon {
    gpui_kit::component::Icon::default().data(br#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round"><circle cx="12" cy="12" r="9"/><path d="M3 12h18M12 3a14 14 0 0 1 0 18M12 3a14 14 0 0 0 0 18"/></svg>"#)
}

pub(super) fn swap_icon() -> gpui_kit::component::Icon {
    gpui_kit::component::Icon::default().data(br#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round"><path d="M5 12h14m-4-4 4 4-4 4"/></svg>"#)
}

pub(super) fn edit_icon() -> gpui_kit::component::Icon {
    gpui_kit::component::Icon::default().data(br#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.7" stroke-linecap="round" stroke-linejoin="round"><path d="m16 3 5 5-12 12-6 1 1-6Z"/><path d="m13 6 5 5"/></svg>"#)
}

pub(super) fn plus_icon() -> gpui_kit::component::Icon {
    gpui_kit::component::Icon::default().data(br#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round"><path d="M12 5v14M5 12h14"/></svg>"#)
}

pub(super) fn close_icon() -> gpui_kit::component::Icon {
    gpui_kit::component::Icon::default().data(br#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round"><path d="m6 6 12 12M6 18 18 6"/></svg>"#)
}

fn order_drop_target<E: InteractiveElement>(
    row: E,
    scope: OrderScope,
    key: String,
    position: usize,
    cx: &mut Context<SwiftSubApp>,
) -> E {
    let drop_key = key.clone();
    row.can_drop(move |value, _, _| {
        value
            .downcast_ref::<ObjectOrderDrag>()
            .is_some_and(|drag| drag.scope == scope && drag.key != key)
    })
    .drag_over::<ObjectOrderDrag>(move |style, drag, _, _| {
        if drag.scope != scope {
            return style;
        }
        let style = style.bg(rgb(ACCENT_SOFT)).border_color(rgb(ACCENT));
        if drag.position < position {
            style.border_b_2()
        } else if drag.position > position {
            style.border_t_2()
        } else {
            style
        }
    })
    .on_drop(cx.listener(
        move |this, drag: &ObjectOrderDrag, window, cx| match scope {
            OrderScope::Nodes => this.move_node(drag.key.clone(), drop_key.clone(), window, cx),
            OrderScope::Groups => {
                this.move_workspace_group(drag.key.clone(), drop_key.clone(), window, cx)
            }
            OrderScope::Members(index) => {
                this.move_workspace_member(index, drag.key.clone(), drop_key.clone(), window, cx)
            }
        },
    ))
}

pub(super) fn object_action(
    id: impl Into<ElementId>,
    icon: gpui_kit::component::Icon,
    label: &'static str,
    cx: &mut Context<SwiftSubApp>,
    handler: impl Fn(&mut SwiftSubApp, &mut Window, &mut Context<SwiftSubApp>) + 'static,
) -> AnyElement {
    div()
        .id(id)
        .cursor_pointer()
        .w(px(32.))
        .h(px(32.))
        .flex_shrink_0()
        .rounded(px(6.))
        .flex()
        .items_center()
        .justify_center()
        .text_color(rgb(MUTED))
        .hover(|style| style.text_color(rgb(TEXT)).bg(rgb(SURFACE_3)))
        .tooltip(move |window, cx| Tooltip::new(label).build(window, cx))
        .child(icon.with_size(px(16.)))
        .on_click(cx.listener(move |this, _, window, cx| {
            cx.stop_propagation();
            handler(this, window, cx);
        }))
        .into_any_element()
}

pub(super) fn danger_action(
    id: impl Into<ElementId>,
    icon: gpui_kit::component::Icon,
    label: &'static str,
    cx: &mut Context<SwiftSubApp>,
    handler: impl Fn(&mut SwiftSubApp, &mut Window, &mut Context<SwiftSubApp>) + 'static,
) -> AnyElement {
    div()
        .id(id)
        .cursor_pointer()
        .w(px(32.))
        .h(px(32.))
        .flex_shrink_0()
        .rounded(px(6.))
        .flex()
        .items_center()
        .justify_center()
        .text_color(rgb(MUTED))
        .hover(|style| style.text_color(rgb(DANGER)).bg(rgb(DANGER_SOFT)))
        .tooltip(move |window, cx| Tooltip::new(label).build(window, cx))
        .child(icon.with_size(px(16.)))
        .on_click(cx.listener(move |this, _, window, cx| {
            cx.stop_propagation();
            handler(this, window, cx);
        }))
        .into_any_element()
}

pub(super) fn object_row() -> Div {
    h_flex()
        .min_w_0()
        .flex_shrink_0()
        .min_h(px(48.))
        .px_3()
        .gap_2()
        .rounded(px(8.))
        .border_1()
        .border_color(rgb(BORDER_ITEM))
        .bg(rgb(SURFACE))
}

fn short_protocol(kind: &str) -> String {
    match kind.to_lowercase().as_str() {
        "shadowsocks" => "SS".into(),
        "shadowsocksr" => "SSR".into(),
        "hysteria2" => "HY2".into(),
        "hysteria" => "HY".into(),
        "wireguard" => "WG".into(),
        other => other.to_uppercase(),
    }
}

fn group_kind_label(kind: GroupKind) -> &'static str {
    match kind {
        GroupKind::UrlTest => "自动选择",
        GroupKind::Fallback => "故障转移",
        GroupKind::LoadBalance => "负载均衡",
        _ => "手动选择",
    }
}

impl SwiftSubApp {
    pub(super) fn config_subscription_settings(&self, cx: &mut Context<Self>) -> AnyElement {
        let interfaces = swiftsub_server::list_lan_interfaces();
        let auto_iface = interfaces.first().cloned();
        let active_ip = self.active_lan_host();
        let active_iface_label = active_ip.as_deref().map(|ip| {
            interfaces
                .iter()
                .find(|iface| iface.ip.to_string() == ip)
                .map(|iface| format!("{} · {}", iface.name, iface.ip))
                .unwrap_or_else(|| ip.to_string())
        });
        let status_label = match &self.server {
            Some(server) if self.server_bind_lan => {
                let port = server.local_addr().port();
                if let Some(label) = &active_iface_label {
                    format!("运行中 · 局域网可用 ({label}:{port})")
                } else {
                    format!("运行中 · 局域网已监听 (0.0.0.0:{port}，暂未检测到局域网 IP)")
                }
            }
            Some(server) => format!("运行中 · 仅本机 (127.0.0.1:{})", server.local_addr().port()),
            None => "未启动".into(),
        };
        let mut rows = vec![
            info_row("服务状态", status_label, false),
            h_flex()
                .min_w_0()
                .py(px(6.))
                .gap_4()
                .child(
                    div()
                        .w(px(110.))
                        .flex_shrink_0()
                        .text_size(px(12.))
                        .text_color(rgb(MUTED))
                        .child("局域网导入"),
                )
                .child(
                    h_flex()
                        .flex_1()
                        .min_w_0()
                        .gap_3()
                        .child(
                            Switch::new("about-server-bind-lan")
                                .checked(self.server_bind_lan)
                                .accessibility_label("允许局域网导入")
                                .on_change(cx.listener(|this, checked, _, cx| {
                                    this.set_server_bind_lan(*checked, cx)
                                })),
                        )
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .text_size(px(12.5))
                                .text_color(rgb(TEXT))
                                .child(if self.server_bind_lan {
                                    "已开启 · 同局域网的手机、平板、路由器等设备可直接订阅"
                                } else {
                                    "关闭中 · 开启后监听 0.0.0.0，供局域网其他设备导入"
                                }),
                        ),
                )
                .into_any_element(),
        ];
        if self.server_bind_lan && !interfaces.is_empty() {
            let auto_label = auto_iface
                .as_ref()
                .map(|iface| format!("自动（{} · {}）", iface.name, iface.ip))
                .unwrap_or_else(|| "自动".into());
            let current_label = match &self.server_lan_host {
                None => auto_label.clone(),
                Some(host) => interfaces
                    .iter()
                    .find(|iface| iface.ip.to_string() == *host)
                    .map(|iface| format!("{} · {}", iface.name, iface.ip))
                    .unwrap_or_else(|| host.clone()),
            };
            let mut options = vec![auto_label.clone()];
            let mut label_to_ip: std::collections::HashMap<String, Option<String>> =
                std::collections::HashMap::from([(auto_label, None)]);
            for iface in &interfaces {
                let label = format!("{} · {}", iface.name, iface.ip);
                label_to_ip.insert(label.clone(), Some(iface.ip.to_string()));
                options.push(label);
            }
            rows.push(
                h_flex()
                    .min_w_0()
                    .py(px(4.))
                    .gap_4()
                    .child(
                        div()
                            .w(px(110.))
                            .flex_shrink_0()
                            .text_size(px(12.))
                            .text_color(rgb(MUTED))
                            .child("局域网网卡"),
                    )
                    .child(self.saved_choice(
                        "about-server-lan-host",
                        current_label,
                        options,
                        cx,
                        move |this, selected, _, cx| {
                            let host = label_to_ip.get(&selected).cloned().flatten();
                            this.set_server_lan_host(host, cx);
                        },
                    ))
                    .into_any_element(),
            );
        }
        rows.extend([
            info_row(
                "更新方式",
                "保存后立即发布；客户端可手动更新或按订阅周期自动拉取".into(),
                false,
            ),
            info_row(
                "订阅文件",
                "每份配置一个独立 YAML 文件和固定地址".into(),
                false,
            ),
        ]);
        for config in &self.saved_configs {
            let id = config.id.clone();
            let lan_id = config.id.clone();
            let lan_url = self.config_lan_subscription_url(&config.id);
            rows.push(
                h_flex()
                    .min_w_0()
                    .gap_3()
                    .py_2()
                    .border_t_1()
                    .border_color(rgb(BORDER))
                    .child(
                        v_flex()
                            .flex_1()
                            .min_w_0()
                            .gap_1()
                            .child(
                                div()
                                    .text_sm()
                                    .font_weight(FontWeight::MEDIUM)
                                    .text_color(rgb(TEXT))
                                    .child(config.name.clone()),
                            )
                            .child(
                                div()
                                    .text_size(px(11.5))
                                    .text_color(rgb(MUTED))
                                    .font_family("Consolas")
                                    .truncate()
                                    .child(format!(
                                        "本机：{}",
                                        self.config_subscription_url(&config.id)
                                            .unwrap_or_else(|| "服务启动后可使用".into())
                                    )),
                            )
                            .when(self.server_bind_lan, |col| {
                                col.child(
                                    div()
                                        .text_size(px(11.5))
                                        .text_color(rgb(ACCENT))
                                        .font_family("Consolas")
                                        .truncate()
                                        .child(format!(
                                            "局域网：{}",
                                            lan_url.clone().unwrap_or_else(|| {
                                                "未检测到局域网 IPv4 地址".into()
                                            })
                                        )),
                                )
                            })
                            .child(
                                div()
                                    .text_size(px(11.5))
                                    .text_color(rgb(MUTED))
                                    .child(self.profile_pull_status(&config.id, &config.yaml)),
                            ),
                    )
                    .child(
                        Button::new(format!("settings-config-subscription-{id}"))
                            .outline()
                            .icon(link_icon())
                            .label("复制本机")
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.copy_config_subscription(id.clone(), cx)
                            })),
                    )
                    .when(self.server_bind_lan, |row| {
                        row.child(
                            Button::new(format!("settings-config-lan-{lan_id}"))
                                .primary()
                                .icon(lan_icon())
                                .label("复制局域网")
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.copy_config_lan_subscription(lan_id.clone(), cx)
                                })),
                        )
                    })
                    .into_any_element(),
            );
        }
        if self.saved_configs.is_empty() {
            rows.push(
                div()
                    .text_sm()
                    .text_color(rgb(MUTED))
                    .child("在生成页保存配置后，会自动创建对应的本地订阅。")
                    .into_any_element(),
            );
        }
        if let Some(error) = &self.server_error {
            rows.push(
                div()
                    .text_sm()
                    .text_color(rgb(DANGER))
                    .child(error.clone())
                    .into_any_element(),
            );
        }
        rows.push(
            div()
                .text_size(px(12.))
                .text_color(rgb(MUTED))
                .child(if self.server_bind_lan {
                    "程序运行时提供订阅；关闭到托盘可继续更新。若其他设备无法访问，请在 Windows 防火墙放行 SwiftSub。"
                } else {
                    "程序运行时提供订阅；关闭到托盘可继续更新。"
                })
                .into_any_element(),
        );

        let app_behavior_rows = vec![
            h_flex()
                .min_w_0()
                .py(px(6.))
                .gap_4()
                .child(
                    div()
                        .w(px(110.))
                        .flex_shrink_0()
                        .text_size(px(12.))
                        .text_color(rgb(MUTED))
                        .child("托盘保活"),
                )
                .child(
                    h_flex()
                        .flex_1()
                        .min_w_0()
                        .gap_3()
                        .child(
                            Switch::new("about-close-to-tray")
                                .checked(self.close_to_tray)
                                .accessibility_label("关闭窗口时最小化到托盘")
                                .on_change(cx.listener(|this, checked, _, cx| {
                                    this.set_close_to_tray(*checked, cx)
                                })),
                        )
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .text_size(px(12.5))
                                .text_color(rgb(TEXT))
                                .child("关闭主窗口时隐藏到系统托盘，保持本机与局域网订阅服务在线"),
                        ),
                )
                .into_any_element(),
        ];

        let mut update_rows = vec![
            info_row("当前版本", format!("v{}", env!("CARGO_PKG_VERSION")), true),
            info_row("更新仓库", self.update_repo.clone(), true),
        ];
        if let Some(info) = &self.update_result {
            if info.has_update {
                update_rows.push(
                    v_flex()
                        .gap_2()
                        .p_3()
                        .rounded(px(8.))
                        .border_1()
                        .border_color(rgb(ACCENT))
                        .bg(rgb(ACCENT_SOFT))
                        .child(
                            h_flex()
                                .justify_between()
                                .gap_3()
                                .child(
                                    div()
                                        .text_sm()
                                        .font_weight(FontWeight::SEMIBOLD)
                                        .text_color(rgb(TEXT))
                                        .child(format!(
                                            "发现新版本 v{} · {}",
                                            info.version, info.title
                                        )),
                                )
                                .child(
                                    Button::new("update-open-download")
                                        .primary()
                                        .label("前往下载")
                                        .on_click(cx.listener(|this, _, _, cx| {
                                            this.open_update_download(cx)
                                        })),
                                ),
                        )
                        .when(!info.notes.is_empty(), |box_el| {
                            box_el.child(
                                div()
                                    .text_size(px(12.))
                                    .text_color(rgb(MUTED))
                                    .child(info.notes.chars().take(280).collect::<String>()),
                            )
                        })
                        .into_any_element(),
                );
            } else {
                update_rows.push(
                    div()
                        .text_size(px(12.5))
                        .text_color(rgb(MUTED))
                        .child(format!("已是最新版本（v{}）", info.version))
                        .into_any_element(),
                );
            }
        }
        if let Some(error) = &self.update_error {
            update_rows.push(
                div()
                    .text_size(px(12.5))
                    .text_color(rgb(DANGER))
                    .child(error.clone())
                    .into_any_element(),
            );
        }

        v_flex()
            .w_full()
            .min_w_0()
            .gap_4()
            .child(settings_section("应用行为", app_behavior_rows, div()))
            .child(settings_section("本地与局域网订阅", rows, div()))
            .child(settings_section(
                "版本与更新",
                update_rows,
                Button::new("settings-check-update")
                    .primary()
                    .disabled(self.update_checking)
                    .label(if self.update_checking {
                        "检查中…"
                    } else {
                        "检查更新"
                    })
                    .on_click(
                        cx.listener(|this, _, window, cx| this.check_for_updates(window, cx)),
                    ),
            ))
            .into_any_element()
    }

    fn drag_handle(
        &self,
        scope: OrderScope,
        key: String,
        name: String,
        position: usize,
    ) -> AnyElement {
        let prefix = match scope {
            OrderScope::Nodes => "node".to_string(),
            OrderScope::Groups => "group".to_string(),
            OrderScope::Members(index) => format!("member-{index}"),
        };
        div()
            .id(format!("{prefix}-order-grip-{key}"))
            .w(px(18.))
            .h(px(32.))
            .flex_shrink_0()
            .flex()
            .items_center()
            .justify_center()
            .text_color(rgb(MUTED))
            .cursor(CursorStyle::OpenHand)
            .hover(|style| {
                style
                    .text_color(rgb(ACCENT))
                    .bg(rgb(ACCENT_SOFT))
                    .rounded(px(4.))
            })
            .tooltip(|window, cx| Tooltip::new("拖动调整顺序").build(window, cx))
            .child(grip_icon().with_size(px(14.)))
            .on_drag(
                ObjectOrderDrag {
                    scope,
                    key,
                    name,
                    position,
                },
                |drag, _, _, cx| {
                    cx.new(|_| NodeDragPreview {
                        name: drag.name.clone(),
                    })
                },
            )
            .into_any_element()
    }

    pub(super) fn sidebar_resize_handle(&self, cx: &mut Context<Self>) -> AnyElement {
        let active = self.workspace.resizing_sidebar;
        div()
            .id("sidebar-resize-handle")
            .w(px(6.))
            .flex_shrink_0()
            .cursor_col_resize()
            .flex()
            .items_center()
            .justify_center()
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _, _, cx| {
                    this.workspace.resizing_sidebar = true;
                    cx.notify();
                }),
            )
            .child(
                div()
                    .w(px(2.))
                    .h(px(40.))
                    .rounded(px(2.))
                    .bg(rgb(if active { ACCENT } else { BORDER_ITEM })),
            )
            .into_any_element()
    }

    pub(super) fn clean_generator_page(
        &mut self,
        width: Pixels,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        if self.workspace.saved_editor_open {
            return self.clean_saved_generator_page(width, cx);
        }
        let sidebar_width = px(self.workspace.sidebar_width)
            .max(px(270.))
            .min((width * 0.36).max(px(280.)).min(px(420.)));
        let current_name = self
            .generated_saved_config
            .as_ref()
            .map(|config| config.name.clone())
            .unwrap_or_else(|| "新建配置".into());
        let status_text = self
            .generated_saved_config
            .as_ref()
            .map(|config| self.profile_pull_status(&config.id, &config.yaml))
            .unwrap_or_else(|| "未保存 · 点击「保存」生成本地订阅".into());
        let saved_id = self
            .generated_saved_config
            .as_ref()
            .map(|config| config.id.clone());

        v_flex()
            .size_full()
            .min_h_0()
            .p_3()
            .gap_3()
            .child(
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
                        h_flex()
                            .flex_1()
                            .min_w_0()
                            .gap_2()
                            .child(
                                div()
                                    .text_sm()
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .text_color(rgb(TEXT))
                                    .truncate()
                                    .child(current_name),
                            )
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .text_size(px(11.5))
                                    .text_color(rgb(MUTED))
                                    .truncate()
                                    .child(status_text),
                            ),
                    )
                    .when_some(saved_id, |row, id| {
                        let lan_id = id.clone();
                        row.child(object_action(
                            "generator-config-subscription",
                            link_icon(),
                            "复制本机订阅地址",
                            cx,
                            move |this, _, cx| this.copy_config_subscription(id.clone(), cx),
                        ))
                        .when(self.server_bind_lan, |row| {
                            row.child(
                                Button::new("generator-config-lan-subscription")
                                    .outline()
                                    .icon(lan_icon())
                                    .label("局域网地址")
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        this.copy_config_lan_subscription(lan_id.clone(), cx)
                                    })),
                            )
                        })
                    })
                    .child(
                        Button::new("save-generated-copy")
                            .outline()
                            .label("存为新配置")
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.begin_save_generated(false, window, cx)
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
                                this.import_generated_to_clash(window, cx)
                            })),
                    ),
            )
            .when_some(self.generated_save_error.clone(), |page, error| {
                page.child(banner(error, DANGER))
            })
            .child(
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
                            .child(self.clean_nodes_panel(cx)),
                    )
                    .child(self.sidebar_resize_handle(cx))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .min_h_0()
                            .child(self.workspace_routing_panel(cx)),
                    ),
            )
            .into_any_element()
    }

    fn clean_nodes_panel(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let total = self.nodes.len() + self.chains.len();
        let query = self.search.read(cx).value().trim().to_lowercase();
        let show_search = total > 10 || !query.is_empty();

        let all_items = self.workspace_node_items();
        let filtered: Vec<_> = all_items
            .into_iter()
            .enumerate()
            .filter(|(_, item)| {
                if query.is_empty() {
                    return true;
                }
                match item.kind {
                    WorkspaceNodeKind::Node(i) => {
                        let node = &self.nodes[i];
                        node.name().to_lowercase().contains(&query)
                            || node.proxy.kind().to_lowercase().contains(&query)
                    }
                    WorkspaceNodeKind::Chain(i) => {
                        let chain = &self.chains[i];
                        chain.name.to_lowercase().contains(&query) || "链式".contains(&query)
                    }
                }
            })
            .collect();
        let count = filtered.len();
        let owner = cx.entity().downgrade();

        let nodes = if total == 0 {
            div()
                .flex_1()
                .flex()
                .items_center()
                .justify_center()
                .text_sm()
                .text_color(rgb(MUTED))
                .child("点击上方「添加」导入订阅或节点")
                .into_any_element()
        } else if count == 0 {
            div()
                .flex_1()
                .flex()
                .items_center()
                .justify_center()
                .text_sm()
                .text_color(rgb(MUTED))
                .child("没有匹配的节点")
                .into_any_element()
        } else {
            uniform_list("generator-node-rows", count, move |range, _, cx| {
                owner
                    .update(cx, |this, cx| {
                        range
                            .map(|idx| {
                                let (position, item) = &filtered[idx];
                                let row = match item.kind {
                                    WorkspaceNodeKind::Node(node_index) => {
                                        this.clean_node_row(&this.nodes[node_index], *position, cx)
                                    }
                                    WorkspaceNodeKind::Chain(chain_index) => {
                                        this.clean_chain_row(chain_index, *position, "sidebar", cx)
                                    }
                                };
                                div().w_full().pb_2().child(row).into_any_element()
                            })
                            .collect::<Vec<_>>()
                    })
                    .unwrap_or_default()
            })
            .flex_1()
            .min_h_0()
            .w_full()
            .track_scroll(&self.workspace.node_scroll)
            .into_any_element()
        };

        v_flex()
            .size_full()
            .min_h_0()
            .p_3()
            .gap_2()
            .rounded(px(12.))
            .border_1()
            .border_color(rgb(BORDER))
            .bg(rgb(SURFACE))
            .child(
                h_flex()
                    .gap_2()
                    .child(
                        Button::new("editor-add-source")
                            .outline()
                            .flex_1()
                            .icon(plus_icon())
                            .label("添加节点 / 订阅")
                            .on_click(
                                cx.listener(|this, _, window, cx| this.open_add_dialog(window, cx)),
                            ),
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
            .when(show_search, |panel| {
                panel.child(Input::new(&self.search).cleanable(true))
            })
            .child(nodes)
            .into_any_element()
    }

    pub(super) fn clean_node_row(
        &self,
        node: &Node,
        position: usize,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let key = node.source_key.clone();
        let edit_key = key.clone();
        let delete_key = key.clone();
        let name = node.name().to_string();
        let proto = short_protocol(node.proxy.kind());
        let group_id = SharedString::from(format!("node-row-group-{key}"));

        let row = object_row()
            .id(format!("node-row-{key}"))
            .group(group_id.clone())
            .when(!node.enabled, |row| row.opacity(0.6))
            .hover(|style| style.bg(rgb(SURFACE_2)))
            .child(self.drag_handle(OrderScope::Nodes, key.clone(), name.clone(), position))
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
                            .text_color(rgb(if node.enabled { TEXT } else { MUTED }))
                            .child(name),
                    )
                    .child(micro_badge(proto, false))
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
                                format!("rename-node-{key}"),
                                edit_icon(),
                                "编辑节点",
                                cx,
                                move |this, window, cx| {
                                    this.open_node_edit_by_key(edit_key.clone(), window, cx)
                                },
                            ))
                            .child(danger_action(
                                format!("delete-node-{key}"),
                                delete_icon(),
                                "删除节点",
                                cx,
                                move |this, window, cx| {
                                    this.remove_workspace_node(delete_key.clone(), window, cx)
                                },
                            )),
                    ),
            )
            .child(
                Switch::new(format!("node-switch-{key}"))
                    .small()
                    .checked(node.enabled)
                    .accessibility_label("启用节点")
                    .on_change(cx.listener({
                        let key = key.clone();
                        move |this, checked, window, cx| {
                            this.toggle_node(key.clone(), *checked, window, cx)
                        }
                    })),
            );
        order_drop_target(row, OrderScope::Nodes, key, position, cx).into_any_element()
    }

    fn chain_route_summary(&self, index: usize) -> (String, bool) {
        let chain = &self.chains[index];
        let resolve =
            |key: &str, fallback: &str| -> (String, bool) {
                if let Some(node) = self.nodes.iter().find(|n| {
                    n.source_key == key || n.proxy.identity_key() == key || n.name() == key
                }) {
                    (node.name().to_string(), node.enabled)
                } else {
                    (fallback.to_string(), false)
                }
            };
        let (entry, entry_ok) = resolve(&chain.entry_node, &chain.entry_name);
        let (exit, exit_ok) = resolve(&chain.landing_node, &chain.landing_name);
        (format!("{entry} → {exit}"), !entry_ok || !exit_ok)
    }

    fn clean_chain_row(
        &self,
        index: usize,
        position: usize,
        scope: &str,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let chain = &self.chains[index];
        let name = chain.name.clone();
        let key = chain_order_key(&name);
        let show_route = scope == "editor";
        let (route, route_warn) = if show_route {
            self.chain_route_summary(index)
        } else {
            (String::new(), false)
        };
        let group_id = SharedString::from(format!("{scope}-chain-group-{index}"));

        let row = object_row()
            .id(format!("{scope}-chain-row-{index}"))
            .group(group_id.clone())
            .when(chain.disabled, |row| row.opacity(0.6))
            .hover(|style| style.bg(rgb(SURFACE_2)))
            .child(self.drag_handle(OrderScope::Nodes, key.clone(), name.clone(), position))
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
                                            .text_color(rgb(if chain.disabled {
                                                MUTED
                                            } else {
                                                TEXT
                                            }))
                                            .child(chain.name.clone()),
                                    )
                                    .child(micro_badge("链式", true)),
                            )
                            .when(show_route, |col| {
                                col.child(
                                    div()
                                        .text_size(px(11.))
                                        .truncate()
                                        .text_color(rgb(if route_warn { WARNING } else { MUTED }))
                                        .child(route),
                                )
                            }),
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
                                format!("{scope}-edit-chain-{index}"),
                                edit_icon(),
                                "编辑链式节点",
                                cx,
                                move |this, window, cx| {
                                    this.workspace_edit_chain(index, window, cx)
                                },
                            ))
                            .child(danger_action(
                                format!("{scope}-delete-chain-{index}"),
                                delete_icon(),
                                "删除链式节点",
                                cx,
                                move |this, window, cx| {
                                    this.workspace_remove_group(name.clone(), window, cx)
                                },
                            )),
                    ),
            )
            .child(
                Switch::new(format!("{scope}-chain-switch-{index}"))
                    .small()
                    .checked(!chain.disabled)
                    .accessibility_label("启用链式节点")
                    .on_change(cx.listener(move |this, checked, window, cx| {
                        this.toggle_workspace_chain(index, *checked, window, cx)
                    })),
            );
        order_drop_target(row, OrderScope::Nodes, key, position, cx).into_any_element()
    }

    fn chain_dropdown(&self, side: ChainSide, cx: &mut Context<Self>) -> AnyElement {
        let selected = match side {
            ChainSide::Entry => self.chain_entry_key.as_ref(),
            ChainSide::Landing => self.chain_landing_key.as_ref(),
        };
        let label = self
            .nodes
            .iter()
            .find(|node| Some(&node.source_key) == selected)
            .map(|node| node.name().to_string())
            .or_else(|| match side {
                ChainSide::Entry => self.chain_entry_name.clone(),
                ChainSide::Landing => self.chain_landing_name.clone(),
            })
            .unwrap_or_else(|| match side {
                ChainSide::Entry => "入口选择".into(),
                ChainSide::Landing => "出口选择".into(),
            });
        let choices: Vec<_> = self
            .nodes
            .iter()
            .filter(|node| node.enabled)
            .map(|node| (node.source_key.clone(), node.name().to_string()))
            .collect();
        let selected = selected.cloned();
        let owner = cx.entity().downgrade();
        Button::new(format!("chain-select-{side:?}"))
            .outline()
            .w_full()
            .label(format!("{label} ▾"))
            .dropdown_menu(move |mut menu, _, _| {
                menu = menu.max_h(px(300.)).scrollable(true);
                for (key, name) in &choices {
                    let owner = owner.clone();
                    let key = key.clone();
                    let name = name.clone();
                    menu = menu.item(
                        PopupMenuItem::new(name.clone())
                            .checked(selected.as_ref() == Some(&key))
                            .on_click(move |_, window, cx| {
                                let _ = owner.update(cx, |this, cx| {
                                    this.choose_chain_endpoint(
                                        side,
                                        key.clone(),
                                        name.clone(),
                                        window,
                                        cx,
                                    )
                                });
                            }),
                    );
                }
                menu
            })
            .into_any_element()
    }

    pub(super) fn clean_chain_panel(&self, cx: &mut Context<Self>) -> AnyElement {
        let mut rows = v_flex().gap_2().min_w_0();
        let node_items = self.workspace_node_items();
        for index in 0..self.chains.len() {
            let key = chain_order_key(&self.chains[index].name);
            let pos = node_items
                .iter()
                .position(|item| item.key == key)
                .unwrap_or(index);
            rows = rows.child(self.clean_chain_row(index, pos, "editor", cx));
        }
        if self.chains.is_empty() {
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
                        div()
                            .flex_1()
                            .min_w_0()
                            .child(self.chain_dropdown(ChainSide::Entry, cx)),
                    )
                    .child(object_action(
                        "chain-swap-endpoints",
                        swap_icon(),
                        "互换入口与出口",
                        cx,
                        |this, window, cx| this.swap_chain_endpoints(window, cx),
                    ))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .child(self.chain_dropdown(ChainSide::Landing, cx)),
                    ),
            )
            .child(
                div()
                    .id("chain-editor-list")
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
            .when_some(self.custom_error.clone(), |panel, error| {
                panel.child(div().text_sm().text_color(rgb(DANGER)).child(error))
            })
            .child(
                h_flex()
                    .gap_2()
                    .child(div().flex_1().min_w_0().child(Input::new(&self.chain_name)))
                    .when(self.workspace.edit_chain.is_some(), |row| {
                        row.child(object_action(
                            "chain-cancel-edit",
                            close_icon(),
                            "取消编辑",
                            cx,
                            |this, window, cx| this.reset_chain_editor(window, cx),
                        ))
                    })
                    .child(
                        Button::new("chain-save")
                            .primary()
                            .label(if self.workspace.edit_chain.is_some() {
                                "更新链式"
                            } else {
                                "保存"
                            })
                            .on_click(
                                cx.listener(|this, _, window, cx| this.add_chain(window, cx)),
                            ),
                    ),
            )
            .into_any_element()
    }

    fn group_summary_text(&self, name: &str, index: Option<usize>, module: Option<&str>) -> String {
        let kind_label = index
            .and_then(|i| self.template_policy_groups.get(i))
            .map(|group| group_kind_label(group.kind))
            .unwrap_or(if module == Some("auto") {
                "自动选择"
            } else {
                "手动选择"
            });
        let member_count = index
            .and_then(|i| self.template_policy_groups.get(i))
            .map(|group| {
                if group.include_all_nodes {
                    self.nodes.iter().filter(|n| n.enabled).count() + group.members.len()
                } else {
                    group.members.len()
                }
            });
        let rule_count = self.workspace_owned_rules(name).len();
        match (member_count, rule_count) {
            (Some(m), r) if r > 0 => format!("{kind_label} · {m} 节点 · {r} 规则"),
            (Some(m), _) => format!("{kind_label} · {m} 节点"),
            (None, r) if r > 0 => format!("{kind_label} · {r} 规则"),
            (None, _) => kind_label.into(),
        }
    }

    pub(super) fn clean_group_card(
        &self,
        name: &str,
        index: Option<usize>,
        module: Option<&str>,
        enabled: bool,
        position: usize,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let expanded = index.is_some() && self.template_policy_edit == index;
        let edit_name = name.to_string();
        let settings_name = name.to_string();
        let delete_name = name.to_string();
        let toggle_name = name.to_string();
        let summary = self.group_summary_text(name, index, module);

        let mut card = v_flex()
            .min_w_0()
            .flex_shrink_0()
            .rounded(px(8.))
            .overflow_hidden()
            .border_1()
            .border_color(rgb(if expanded { ACCENT } else { BORDER_ITEM }))
            .bg(rgb(SURFACE))
            .when(!enabled && !expanded, |card| card.opacity(0.65))
            .child(
                h_flex()
                    .min_h(px(48.))
                    .gap_1()
                    .px_2()
                    .bg(rgb(if expanded { ACCENT_SOFT } else { SURFACE }))
                    .child(self.drag_handle(
                        OrderScope::Groups,
                        name.to_string(),
                        name.to_string(),
                        position,
                    ))
                    .child(
                        h_flex()
                            .id(format!("workspace-group-{name}"))
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
                                    .text_sm()
                                    .font_weight(if expanded {
                                        FontWeight::SEMIBOLD
                                    } else {
                                        FontWeight::MEDIUM
                                    })
                                    .text_color(rgb(if enabled { TEXT } else { MUTED }))
                                    .truncate()
                                    .child(name.to_string()),
                            )
                            .child(
                                div()
                                    .flex_shrink_0()
                                    .text_size(px(11.))
                                    .text_color(rgb(MUTED))
                                    .child(summary),
                            )
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.workspace_edit_group(edit_name.clone(), window, cx)
                            })),
                    )
                    .child(object_action(
                        format!("workspace-group-settings-{name}"),
                        edit_icon(),
                        "编辑策略组",
                        cx,
                        move |this, window, cx| {
                            this.edit_workspace_group(settings_name.clone(), window, cx)
                        },
                    ))
                    .child(danger_action(
                        format!("workspace-delete-group-{name}"),
                        delete_icon(),
                        "删除策略组",
                        cx,
                        move |this, window, cx| {
                            this.workspace_remove_group(delete_name.clone(), window, cx)
                        },
                    ))
                    .child(
                        Switch::new(format!("workspace-group-enabled-{name}"))
                            .small()
                            .checked(enabled)
                            .accessibility_label("启用策略组")
                            .on_change(cx.listener(move |this, checked, window, cx| {
                                this.toggle_workspace_group(
                                    toggle_name.clone(),
                                    *checked,
                                    window,
                                    cx,
                                )
                            })),
                    ),
            );
        if expanded {
            let target = name.to_string();
            let kind_label = index
                .and_then(|index| self.template_policy_groups.get(index))
                .map(|group| group_kind_label(group.kind))
                .unwrap_or(if module == Some("auto") {
                    "自动选择"
                } else {
                    "手动选择"
                });
            let mut tabs = h_flex()
                .gap_2()
                .child(
                    Button::new(format!("group-nodes-{name}"))
                        .outline()
                        .label("节点")
                        .when(!self.workspace.group_rules_tab, |button| button.primary())
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.workspace.group_rules_tab = false;
                            cx.notify();
                        })),
                )
                .child(
                    Button::new(format!("group-rules-{name}"))
                        .outline()
                        .label("规则")
                        .when(self.workspace.group_rules_tab, |button| button.primary())
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.workspace.group_rules_tab = true;
                            cx.notify();
                        })),
                )
                .child(
                    Button::new(format!("workspace-add-rules-{name}"))
                        .outline()
                        .label("新增规则")
                        .on_click(cx.listener(move |this, _, window, cx| {
                            this.workspace.group_rules_tab = true;
                            this.workspace_begin_rule_picker(target.clone(), window, cx);
                        })),
                );
            if self.workspace.group_settings_open {
                tabs = tabs.child(div().flex_1()).child(self.saved_choice(
                    format!("workspace-group-kind-{name}"),
                    kind_label.into(),
                    vec![
                        "手动选择".into(),
                        "自动选择".into(),
                        "故障转移".into(),
                        "负载均衡".into(),
                    ],
                    cx,
                    move |this, value, _, cx| {
                        if let Some(index) = index {
                            this.set_template_policy_kind(
                                index,
                                match value.as_str() {
                                    "自动选择" => GroupKind::UrlTest,
                                    "故障转移" => GroupKind::Fallback,
                                    "负载均衡" => GroupKind::LoadBalance,
                                    _ => GroupKind::Select,
                                },
                                cx,
                            );
                        }
                    },
                ));
            }
            card = card.child(
                v_flex()
                    .p_3()
                    .gap_3()
                    .min_w_0()
                    .border_t_1()
                    .border_color(rgb(BORDER_STRONG))
                    .bg(rgb(SURFACE_2))
                    .child(tabs)
                    .when(self.workspace.group_settings_open, |body| {
                        body.child(self.workspace_inline_group_settings(cx))
                    })
                    .child(if self.workspace.group_rules_tab {
                        self.workspace_group_rules_panel(name, cx)
                    } else {
                        self.clean_group_members(cx)
                    }),
            );
        }
        order_drop_target(
            card.id(format!("group-card-{name}")),
            OrderScope::Groups,
            name.to_string(),
            position,
            cx,
        )
        .into_any_element()
    }

    fn clean_group_members(&self, cx: &mut Context<Self>) -> AnyElement {
        let Some(index) = self.template_policy_edit else {
            return div().into_any_element();
        };
        let Some(group) = self.template_policy_groups.get(index) else {
            return div().into_any_element();
        };
        let selected_members = self.workspace_selected_members(index);
        let query = self
            .template_member_search
            .read(cx)
            .value()
            .trim()
            .to_lowercase();
        let mut candidates: Vec<String> = self
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
            .chain(group.members.iter().cloned())
            .collect();
        let mut seen = HashSet::new();
        candidates.retain(|name| {
            name != &group.name && seen.insert(name.clone()) && name.to_lowercase().contains(&query)
        });
        candidates.sort_by_key(|name| {
            selected_members
                .iter()
                .position(|member| member == name)
                .unwrap_or(usize::MAX)
        });
        let mut rows = v_flex().gap_2().min_w_0();
        for name in candidates {
            let is_node = self
                .nodes
                .iter()
                .any(|node| node.enabled && node.name() == name)
                || self
                    .chains
                    .iter()
                    .any(|chain| !chain.disabled && chain.name == name);
            let selected = group.members.contains(&name) || (group.include_all_nodes && is_node);
            let remove_name = name.clone();
            let remove_key = self
                .nodes
                .iter()
                .find(|node| node.name() == name)
                .map(|node| node.source_key.clone());
            let remove_chain = self.chains.iter().any(|chain| chain.name == name);
            let edit_name = name.clone();
            let toggle_name = name.clone();
            let member_position = selected_members
                .iter()
                .position(|member| member == &name)
                .unwrap_or_default();
            let row = object_row()
                .id(format!("workspace-member-row-{index}-{name}"))
                .when(!selected, |row| row.opacity(0.65))
                .child(if selected {
                    self.drag_handle(
                        OrderScope::Members(index),
                        name.clone(),
                        name.clone(),
                        member_position,
                    )
                } else {
                    div().w(px(18.)).flex_shrink_0().into_any_element()
                })
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .truncate()
                        .text_sm()
                        .text_color(rgb(if selected { TEXT } else { MUTED }))
                        .child(name.clone()),
                )
                .when(
                    !matches!(name.as_str(), "DIRECT" | "REJECT" | "REJECT-DROP"),
                    |row| {
                        row.child(object_action(
                            format!("member-edit-{name}"),
                            edit_icon(),
                            "编辑",
                            cx,
                            move |this, window, cx| {
                                this.edit_workspace_member(edit_name.clone(), window, cx)
                            },
                        ))
                    },
                )
                .child(danger_action(
                    format!("member-remove-{name}"),
                    delete_icon(),
                    if remove_key.is_some() {
                        "删除节点"
                    } else if remove_chain {
                        "删除链式节点"
                    } else {
                        "从策略组移除"
                    },
                    cx,
                    move |this, window, cx| {
                        if let Some(key) = &remove_key {
                            this.remove_workspace_node(key.clone(), window, cx);
                        } else if remove_chain {
                            this.workspace_remove_group(remove_name.clone(), window, cx);
                        } else {
                            this.set_workspace_member(
                                index,
                                remove_name.clone(),
                                false,
                                window,
                                cx,
                            );
                        }
                    },
                ))
                .child(
                    Switch::new(format!("member-switch-{name}"))
                        .small()
                        .checked(selected)
                        .accessibility_label("加入策略组")
                        .on_change(cx.listener(move |this, checked, window, cx| {
                            this.set_workspace_member(
                                index,
                                toggle_name.clone(),
                                *checked,
                                window,
                                cx,
                            )
                        })),
                );
            rows = rows.child(if selected {
                order_drop_target(row, OrderScope::Members(index), name, member_position, cx)
                    .into_any_element()
            } else {
                row.into_any_element()
            });
        }
        v_flex()
            .min_w_0()
            .gap_2()
            .when(self.nodes.len() + self.chains.len() > 12, |panel| {
                panel.child(Input::new(&self.template_member_search).cleanable(true))
            })
            .child(
                div()
                    .id("workspace-member-list")
                    .max_h(px(300.))
                    .p_2()
                    .rounded(px(8.))
                    .border_1()
                    .border_color(rgb(BORDER_ITEM))
                    .bg(rgb(SURFACE_INSET))
                    .overflow_y_scrollbar()
                    .child(rows),
            )
            .into_any_element()
    }

    pub(super) fn clean_config_library(&self, width: Pixels, cx: &mut Context<Self>) -> AnyElement {
        if self.workspace.saved_editor_open {
            return v_flex()
                .size_full()
                .min_h_0()
                .gap_2()
                .p_3()
                .child(
                    Button::new("saved-back-library")
                        .ghost()
                        .label("返回配置")
                        .on_click(cx.listener(|this, _, window, cx| {
                            match this.autosave_saved_editor(window, cx) {
                                Ok(()) => this.workspace.saved_editor_open = false,
                                Err(error) => this.saved_config_error = Some(error),
                            };
                            cx.notify();
                        })),
                )
                .child(
                    div()
                        .flex_1()
                        .min_h_0()
                        .child(self.saved_configs_editor(width, cx)),
                )
                .into_any_element();
        }
        let mut rows = v_flex().gap_3().min_w_0();
        for config in &self.saved_configs {
            let edit_id = config.id.clone();
            let import_id = config.id.clone();
            let delete_id = config.id.clone();
            let copy_id = config.id.clone();
            let copy_lan_id = config.id.clone();
            let current = self
                .generated_saved_config
                .as_ref()
                .map(|saved| saved.id == config.id)
                .unwrap_or_else(|| self.saved_config_id.as_deref() == Some(config.id.as_str()));
            let summary = self
                .workspace
                .config_summaries
                .get(&config.id)
                .cloned()
                .unwrap_or_else(|| crate::app::workspace::ConfigSummary::from_config(config));
            rows = rows.child(
                v_flex()
                    .id(format!("library-card-{}", config.id))
                    .min_h(px(112.))
                    .px_4()
                    .py_3()
                    .gap_3()
                    .rounded(px(12.))
                    .border_1()
                    .border_color(rgb(if current { ACCENT } else { BORDER_ITEM }))
                    .bg(rgb(SURFACE))
                    .child(
                        h_flex()
                            .gap_3()
                            .child(
                                div()
                                    .w(px(40.))
                                    .h(px(40.))
                                    .flex_shrink_0()
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .rounded(px(8.))
                                    .bg(rgb(ACCENT_SOFT))
                                    .text_color(rgb(ACCENT))
                                    .child(configuration_icon().with_size(px(20.))),
                            )
                            .child(
                                v_flex()
                                    .flex_1()
                                    .min_w_0()
                                    .gap_1()
                                    .child(
                                        div()
                                            .truncate()
                                            .text_size(px(15.))
                                            .font_weight(FontWeight::SEMIBOLD)
                                            .child(config.name.clone()),
                                    )
                                    .child(
                                        h_flex()
                                            .gap_2()
                                            .text_size(px(11.5))
                                            .text_color(rgb(MUTED))
                                            .child(if summary.generated {
                                                "生成配置"
                                            } else {
                                                "YAML 配置"
                                            })
                                            .when(current, |row| {
                                                row.child(micro_badge("当前", true))
                                            }),
                                    ),
                            )
                            .child(
                                Button::new(format!("library-edit-{edit_id}"))
                                    .outline()
                                    .icon(edit_icon())
                                    .label("编辑")
                                    .on_click(cx.listener(move |this, _, window, cx| {
                                        this.edit_library_config(edit_id.clone(), window, cx)
                                    })),
                            )
                            .child(
                                Button::new(format!("library-import-{import_id}"))
                                    .primary()
                                    .label(if self.clash_import_busy {
                                        "导入中…"
                                    } else {
                                        "导入到 Clash"
                                    })
                                    .disabled(self.clash_import_busy)
                                    .on_click(cx.listener(move |this, _, window, cx| {
                                        this.import_library_config(import_id.clone(), window, cx)
                                    })),
                            )
                            .child(danger_action(
                                format!("library-delete-{delete_id}"),
                                delete_icon(),
                                "删除配置",
                                cx,
                                move |this, _, cx| {
                                    this.workspace.confirm_delete_config = Some(delete_id.clone());
                                    cx.notify();
                                },
                            )),
                    )
                    .child(
                        h_flex()
                            .flex_wrap()
                            .justify_between()
                            .gap_2()
                            .pt_2()
                            .border_t_1()
                            .border_color(rgb(BORDER))
                            .text_size(px(11.5))
                            .text_color(rgb(MUTED))
                            .child(summary.text())
                            .child(
                                h_flex()
                                    .gap_2()
                                    .child(format!(
                                        "更新于 {}",
                                        format_timestamp(Some(config.updated_at))
                                    ))
                                    .child(object_action(
                                        format!("library-link-{copy_id}"),
                                        link_icon(),
                                        "复制本机订阅地址",
                                        cx,
                                        move |this, _, cx| {
                                            this.copy_config_subscription(copy_id.clone(), cx)
                                        },
                                    ))
                                    .when(self.server_bind_lan, |row| {
                                        row.child(
                                            Button::new(format!("library-lan-{copy_lan_id}"))
                                                .outline()
                                                .icon(lan_icon())
                                                .label("局域网地址")
                                                .on_click(cx.listener(move |this, _, _, cx| {
                                                    this.copy_config_lan_subscription(
                                                        copy_lan_id.clone(),
                                                        cx,
                                                    )
                                                })),
                                        )
                                    }),
                            ),
                    ),
            );
        }
        if self.saved_configs.is_empty() {
            rows = rows.child(
                v_flex()
                    .h(px(220.))
                    .items_center()
                    .justify_center()
                    .gap_3()
                    .rounded(px(12.))
                    .border_1()
                    .border_color(rgb(BORDER_ITEM))
                    .bg(rgb(SURFACE))
                    .text_sm()
                    .text_color(rgb(MUTED))
                    .child(configuration_icon().with_size(px(30.)))
                    .child(
                        div()
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(rgb(TEXT))
                            .child("暂无配置"),
                    )
                    .child("在生成页保存配置，或导入已有 YAML 文件"),
            );
        }
        v_flex()
            .size_full()
            .min_h_0()
            .p_4()
            .child(
                v_flex()
                    .w_full()
                    .flex_1()
                    .min_h_0()
                    .gap_3()
                    .child(
                        self.page_title(
                            "我的配置",
                            h_flex()
                                .gap_3()
                                .child(
                                    div()
                                        .text_sm()
                                        .text_color(rgb(MUTED))
                                        .child(format!("共 {} 份", self.saved_configs.len())),
                                )
                                .child(
                                    h_flex()
                                        .gap_2()
                                        .px_3()
                                        .h(px(34.))
                                        .rounded(px(8.))
                                        .border_1()
                                        .border_color(rgb(if self.server_bind_lan {
                                            ACCENT
                                        } else {
                                            BORDER
                                        }))
                                        .bg(rgb(if self.server_bind_lan {
                                            ACCENT_SOFT
                                        } else {
                                            SURFACE
                                        }))
                                        .child(
                                            div()
                                                .text_size(px(12.5))
                                                .text_color(rgb(if self.server_bind_lan {
                                                    TEXT
                                                } else {
                                                    MUTED
                                                }))
                                                .child("局域网导入"),
                                        )
                                        .child(
                                            Switch::new("library-server-bind-lan")
                                                .small()
                                                .checked(self.server_bind_lan)
                                                .accessibility_label("允许局域网导入")
                                                .on_change(cx.listener(|this, checked, _, cx| {
                                                    this.set_server_bind_lan(*checked, cx)
                                                })),
                                        ),
                                )
                                .child(
                                    Button::new("library-import-yaml")
                                        .outline()
                                        .icon(plus_icon())
                                        .label("导入 YAML")
                                        .on_click(cx.listener(|this, _, window, cx| {
                                            this.import_yaml_config(window, cx)
                                        })),
                                ),
                        ),
                    )
                    .when_some(self.saved_config_error.clone(), |page, error| {
                        page.child(banner(error, DANGER))
                    })
                    .child(
                        div()
                            .id("config-library-scroll")
                            .flex_1()
                            .min_h_0()
                            .overflow_y_scrollbar()
                            .child(rows),
                    ),
            )
            .into_any_element()
    }

    pub(super) fn generated_name_dialog(&self, cx: &mut Context<Self>) -> AnyElement {
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
                        "close-generated-name",
                        close_icon(),
                        "关闭",
                        cx,
                        |this, window, cx| this.cancel_generated_name(window, cx),
                    )),
            )
            .child(Input::new(&self.generated_config_name))
            .when_some(self.generated_save_error.clone(), |panel, error| {
                panel.child(div().text_sm().text_color(rgb(DANGER)).child(error))
            })
            .child(
                h_flex()
                    .justify_end()
                    .gap_2()
                    .child(
                        Button::new("cancel-generated-name")
                            .ghost()
                            .label("取消")
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.cancel_generated_name(window, cx)
                            })),
                    )
                    .child(
                        Button::new("confirm-generated-name")
                            .primary()
                            .label("保存")
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.save_generated_config(false, window, cx);
                            })),
                    ),
            );
        modal_with_backdrop("generated-name-backdrop", panel, cx, |this, window, cx| {
            this.cancel_generated_name(window, cx)
        })
    }

    pub(super) fn clean_filter_group_card(
        &self,
        index: usize,
        position: usize,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let group = &self.custom_groups[index];
        let name = group.name.clone();
        let delete_name = name.clone();
        let expanded = self.workspace.rules_target.as_deref() == Some(&name)
            && self.template_policy_edit.is_none();
        let match_count = self
            .nodes
            .iter()
            .filter(|node| node.enabled && group.filter.matches(node))
            .count();
        let rule_count = self.workspace_owned_rules(&name).len();
        let summary = if rule_count > 0 {
            format!("筛选组 · {match_count} 节点 · {rule_count} 规则")
        } else {
            format!("筛选组 · {match_count} 节点")
        };

        let mut card = v_flex()
            .min_w_0()
            .flex_shrink_0()
            .rounded(px(8.))
            .overflow_hidden()
            .border_1()
            .border_color(rgb(if expanded { ACCENT } else { BORDER_ITEM }))
            .bg(rgb(SURFACE))
            .when(group.disabled && !expanded, |card| card.opacity(0.65))
            .child(
                h_flex()
                    .min_h(px(48.))
                    .px_2()
                    .gap_1()
                    .bg(rgb(if expanded { ACCENT_SOFT } else { SURFACE }))
                    .child(self.drag_handle(
                        OrderScope::Groups,
                        name.clone(),
                        name.clone(),
                        position,
                    ))
                    .child(
                        h_flex()
                            .id(format!("workspace-filter-{index}"))
                            .flex_1()
                            .min_w_0()
                            .gap_2()
                            .cursor_pointer()
                            .child(
                                disclosure_icon(expanded)
                                    .with_size(px(14.))
                                    .text_color(rgb(MUTED)),
                            )
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .text_sm()
                                    .font_weight(if expanded {
                                        FontWeight::SEMIBOLD
                                    } else {
                                        FontWeight::MEDIUM
                                    })
                                    .text_color(rgb(if group.disabled { MUTED } else { TEXT }))
                                    .truncate()
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
                                this.workspace_edit_group(name.clone(), window, cx)
                            })),
                    )
                    .child(object_action(
                        format!("filter-edit-{index}"),
                        edit_icon(),
                        "编辑策略组",
                        cx,
                        move |this, window, cx| this.workspace_edit_filter(index, window, cx),
                    ))
                    .child(danger_action(
                        format!("filter-delete-{index}"),
                        delete_icon(),
                        "删除策略组",
                        cx,
                        move |this, window, cx| {
                            this.workspace_remove_group(delete_name.clone(), window, cx)
                        },
                    ))
                    .child(
                        Switch::new(format!("filter-switch-{index}"))
                            .small()
                            .checked(!group.disabled)
                            .accessibility_label("启用策略组")
                            .on_change(cx.listener(move |this, checked: &bool, window, cx| {
                                this.toggle_workspace_filter(index, *checked, window, cx)
                            })),
                    ),
            );
        if expanded {
            let target = group.name.clone();
            let mut nodes = v_flex().gap_2();
            for (pos, node) in self
                .nodes
                .iter()
                .filter(|node| group.filter.matches(node))
                .enumerate()
            {
                nodes = nodes.child(self.clean_node_row(node, pos, cx));
            }
            card = card.child(
                v_flex()
                    .min_w_0()
                    .p_3()
                    .gap_3()
                    .border_t_1()
                    .border_color(rgb(BORDER_STRONG))
                    .bg(rgb(SURFACE_2))
                    .child(
                        h_flex()
                            .gap_2()
                            .child(
                                Button::new("filter-nodes-tab")
                                    .outline()
                                    .label("节点")
                                    .when(!self.workspace.group_rules_tab, |button| {
                                        button.primary()
                                    })
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.workspace.group_rules_tab = false;
                                        cx.notify();
                                    })),
                            )
                            .child(
                                Button::new("filter-rules-tab")
                                    .outline()
                                    .label("规则")
                                    .when(self.workspace.group_rules_tab, |button| button.primary())
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.workspace.group_rules_tab = true;
                                        cx.notify();
                                    })),
                            )
                            .child(
                                Button::new("filter-add-rule")
                                    .outline()
                                    .label("新增规则")
                                    .on_click(cx.listener(move |this, _, window, cx| {
                                        this.workspace.group_rules_tab = true;
                                        this.workspace_begin_rule_picker(
                                            target.clone(),
                                            window,
                                            cx,
                                        );
                                    })),
                            ),
                    )
                    .child(if self.workspace.group_rules_tab {
                        self.workspace_group_rules_panel(&group.name, cx)
                    } else {
                        div()
                            .id("filter-node-list")
                            .max_h(px(300.))
                            .p_2()
                            .rounded(px(8.))
                            .border_1()
                            .border_color(rgb(BORDER_ITEM))
                            .bg(rgb(SURFACE_INSET))
                            .overflow_y_scrollbar()
                            .child(nodes)
                            .into_any_element()
                    }),
            );
        }
        order_drop_target(
            card.id(format!("filter-group-card-{index}")),
            OrderScope::Groups,
            group.name.clone(),
            position,
            cx,
        )
        .into_any_element()
    }

    pub(super) fn clean_template_settings(&self, cx: &mut Context<Self>) -> AnyElement {
        let preset = self
            .template_base_options
            .as_ref()
            .unwrap_or(&self.options)
            .preset;
        let mut rows = v_flex().gap_2().min_w_0();
        for (value, label, desc) in [
            (
                Preset::Minimal,
                "精简",
                "节点选择、自动选择、国内直连与漏网之鱼",
            ),
            (Preset::Standard, "标准", "常用流媒体、AI、开发与社交分流组"),
            (Preset::Full, "完整", "全量 46 个细分服务分流策略组"),
        ] {
            let active = preset == value && self.workspace.selected_template_id.is_none();
            rows = rows.child(
                object_row()
                    .px_3()
                    .when(active, |row| row.border_color(rgb(ACCENT)))
                    .child(
                        v_flex()
                            .flex_1()
                            .min_w_0()
                            .py_1()
                            .child(div().text_sm().font_weight(FontWeight::MEDIUM).child(label))
                            .child(div().text_size(px(11.5)).text_color(rgb(MUTED)).child(desc)),
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
                        Button::new(format!("template-customize-{label}"))
                            .ghost()
                            .label("去生成页自定义")
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.workspace_preset(value, window, cx);
                                this.page = Page::Generator;
                                cx.notify();
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
                format!(
                    "{groups_count} 个策略组 · {rule_sets_count} 个规则集 · {manual_count} 条手写规则"
                )
            } else {
                format!("{groups_count} 个策略组 · {rule_sets_count} 个规则集")
            };
            rows = rows.child(
                object_row()
                    .px_3()
                    .when(active, |row| row.border_color(rgb(ACCENT)))
                    .child(
                        v_flex()
                            .flex_1()
                            .min_w_0()
                            .py_1()
                            .child(
                                div()
                                    .truncate()
                                    .text_sm()
                                    .font_weight(FontWeight::MEDIUM)
                                    .child(template.name.clone()),
                            )
                            .child(
                                div()
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
                            .label("编辑")
                            .tooltip("载入此模板并跳转到「生成」页修改策略组和规则")
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.workspace_load_template(id, window, cx);
                                this.page = Page::Generator;
                                cx.notify();
                            })),
                    )
                    .child(
                        Button::new(format!("template-overwrite-{id}"))
                            .ghost()
                            .label("用当前覆盖")
                            .tooltip("用「生成」页当前的策略组与规则覆盖更新此模板")
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.update_custom_template_with_current(id, window, cx);
                            })),
                    )
                    .child(danger_action(
                        format!("template-delete-{id}"),
                        delete_icon(),
                        "删除模板",
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
                        v_flex()
                            .flex_1()
                            .min_w_0()
                            .gap_1()
                            .child(
                                div()
                                    .text_size(px(14.5))
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .child("分流模板"),
                            )
                            .child(
                                div()
                                    .text_size(px(12.))
                                    .text_color(rgb(MUTED))
                                    .child("模板保存的是「生成」页的策略组、规则集与手写规则结构（不含节点）。在「生成」页调整好分流后，可在此另存为新模板或覆盖已有自定义模板。"),
                            ),
                    )
                    .child(
                        Button::new("template-save-current")
                            .primary()
                            .label("将当前分流存为新模板")
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.template_edit_id = None;
                                this.template_name.update(cx, |input, cx| {
                                    input.set_value("", window, cx);
                                });
                                this.workspace.save_template_open = true;
                                cx.notify();
                            })),
                    ),
            )
            .when(self.workspace.save_template_open, |panel| {
                panel.child(
                    h_flex()
                        .gap_2()
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .child(Input::new(&self.template_name)),
                        )
                        .child(
                            Button::new("workspace-save-template")
                                .primary()
                                .label("保存")
                                .on_click(cx.listener(|this, _, window, cx| {
                                    this.save_current_template(window, cx);
                                    this.page = Page::Settings;
                                    this.settings_tab = SettingsTab::Templates;
                                    cx.notify();
                                })),
                        )
                        .child(object_action(
                            "template-save-close",
                            close_icon(),
                            "取消",
                            cx,
                            |this, _, cx| {
                                this.workspace.save_template_open = false;
                                cx.notify();
                            },
                        )),
                )
            })
            .child(rows)
            .into_any_element()
    }
}
