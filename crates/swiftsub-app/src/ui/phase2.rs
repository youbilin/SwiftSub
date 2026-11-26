//! Phase 2 (P1) UI views and modals:
//! - QR code share modal (LAN subscription & single-node share link)
//! - Configuration revision history & rollback modal
//! - Backup restore confirmation modal
//! - Batch node rename modal
//! - Subscription traffic/expiry health & refresh logs modal
//! - Settings sections: auto-start, backup/restore, fixed port, LAN private-only, firewall helper, fetch proxy

use gpui_kit::component::menu::{DropdownMenu as _, PopupMenuItem};
use gpui_kit::component::tooltip::Tooltip;

use super::icons::{batch_icon, close_icon, qr_icon};
use super::layout::object_action;
use super::*;
use crate::app::phase2::format_bytes_human;

impl SwiftSubApp {
    /// Extra rows for Settings -> Application Behavior: appearance mode, silent auto-start, backup/restore.
    pub(super) fn app_behavior_p1_rows(&self, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let theme_mode = self.p1.theme_mode;
        let mut rows = vec![
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
                        .child("外观主题"),
                )
                .child(
                    h_flex()
                        .flex_1()
                        .min_w_0()
                        .flex_wrap()
                        .gap_2()
                        .child(
                            Button::new("theme-mode-system")
                                .ghost()
                                .label("跟随系统")
                                .when(theme_mode == AppThemeMode::System, |b| b.primary())
                                .on_click(cx.listener(|this, _, window, cx| {
                                    this.set_theme_mode(AppThemeMode::System, window, cx)
                                })),
                        )
                        .child(
                            Button::new("theme-mode-light")
                                .ghost()
                                .label("浅色")
                                .when(theme_mode == AppThemeMode::Light, |b| b.primary())
                                .on_click(cx.listener(|this, _, window, cx| {
                                    this.set_theme_mode(AppThemeMode::Light, window, cx)
                                })),
                        )
                        .child(
                            Button::new("theme-mode-dark")
                                .ghost()
                                .label("深色")
                                .when(theme_mode == AppThemeMode::Dark, |b| b.primary())
                                .on_click(cx.listener(|this, _, window, cx| {
                                    this.set_theme_mode(AppThemeMode::Dark, window, cx)
                                })),
                        ),
                )
                .into_any_element(),
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
                        .child("开机自启"),
                )
                .child(
                    h_flex()
                        .flex_1()
                        .min_w_0()
                        .gap_3()
                        .child(
                            Switch::new("about-auto-start")
                                .checked(self.p1.auto_start)
                                .accessibility_label("开机静默自启动到系统托盘")
                                .on_change(cx.listener(|this, checked, _, cx| {
                                    this.set_auto_start(*checked, cx)
                                })),
                        )
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .text_size(px(12.5))
                                .text_color(rgb(TEXT))
                                .child(
                                    "登录 Windows 时自动静默启动并隐藏至托盘，保持局域网订阅在线",
                                ),
                        ),
                )
                .into_any_element(),
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
                        .child("数据备份"),
                )
                .child(
                    h_flex()
                        .flex_1()
                        .min_w_0()
                        .flex_wrap()
                        .gap_2()
                        .child(
                            Button::new("about-export-backup")
                                .outline()
                                .label("导出完整备份")
                                .on_click(
                                    cx.listener(|this, _, _, cx| this.export_full_backup(cx)),
                                ),
                        )
                        .child(
                            Button::new("about-restore-backup")
                                .outline()
                                .label("从备份恢复")
                                .on_click(
                                    cx.listener(|this, _, _, cx| this.pick_backup_to_restore(cx)),
                                ),
                        ),
                )
                .into_any_element(),
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
                        .child("Mihomo 内核"),
                )
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .font_family("Consolas")
                        .text_size(px(11.5))
                        .text_color(rgb(if self.p1.mihomo_binary_path.is_some() {
                            TEXT
                        } else {
                            MUTED
                        }))
                        .truncate()
                        .child(
                            self.p1
                                .mihomo_binary_path
                                .as_ref()
                                .map(|p| p.display().to_string())
                                .unwrap_or_else(|| {
                                    "未检测到（可选，用于本机内核级配置预检）".into()
                                }),
                        ),
                )
                .child(
                    Button::new("about-pick-mihomo")
                        .outline()
                        .label("选择内核")
                        .on_click(cx.listener(|this, _, _, cx| this.pick_mihomo_binary(cx))),
                )
                .into_any_element(),
        ];
        if self.p1.backup_restore_pending {
            rows.push(banner(
                "已准备从备份恢复：请退出（托盘右键退出）并重新打开 SwiftSub 生效，原数据会自动保留为 swiftsub.db.bak。".into(),
                WARNING,
            ));
        }
        rows
    }

    /// Extra rows for Settings -> Local & LAN subscription: fixed port, private-only guard, firewall command.
    pub(super) fn lan_p1_settings_rows(&self, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let mut rows =
            vec![
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
                            .child("固定监听端口"),
                    )
                    .child(
                        h_flex()
                            .flex_1()
                            .min_w_0()
                            .gap_2()
                            .child(div().w(px(200.)).child(
                                Input::new(&self.p1.server_fixed_port_input).cleanable(true),
                            ))
                            .child(
                                Button::new("about-apply-fixed-port")
                                    .outline()
                                    .label("应用端口")
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.apply_server_fixed_port(cx)
                                    })),
                            ),
                    )
                    .into_any_element(),
            ];
        if self.server_bind_lan {
            rows.push(
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
                            .child("访问安全限制"),
                    )
                    .child(
                        h_flex()
                            .flex_1()
                            .min_w_0()
                            .gap_3()
                            .child(
                                Switch::new("about-lan-private-only")
                                    .checked(self.p1.server_lan_private_only)
                                    .accessibility_label("仅允许私网地址访问局域网订阅")
                                    .on_change(cx.listener(|this, checked, _, cx| {
                                        this.set_server_lan_private_only(*checked, cx)
                                    })),
                            )
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .text_size(px(12.5))
                                    .text_color(rgb(TEXT))
                                    .child("仅允许私网与虚拟组网访问（RFC1918 / Tailscale），拦截公网来源"),
                            ),
                    )
                    .into_any_element(),
            );
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
                            .child("防火墙放行"),
                    )
                    .child(
                        h_flex()
                            .flex_1()
                            .min_w_0()
                            .gap_2()
                            .child(
                                Button::new("about-copy-firewall-cmd")
                                    .outline()
                                    .label("复制管理员放行命令")
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.copy_lan_firewall_command(cx)
                                    })),
                            )
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .text_size(px(11.5))
                                    .text_color(rgb(MUTED))
                                    .child("手机或软路由无法连接时，在管理员终端粘贴运行即可"),
                            ),
                    )
                    .into_any_element(),
            );
        }
        rows
    }

    /// Settings section for global fetch proxy (System / Direct / Custom HTTP or SOCKS5).
    pub(super) fn fetch_proxy_settings_section(&self, cx: &mut Context<Self>) -> AnyElement {
        let mode = self.p1.fetch_proxy_mode.as_str();
        let mut rows = vec![
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
                        .child("抓取代理模式"),
                )
                .child(
                    h_flex()
                        .flex_1()
                        .min_w_0()
                        .flex_wrap()
                        .gap_2()
                        .child(
                            Button::new("proxy-mode-system")
                                .ghost()
                                .label("跟随系统代理")
                                .when(mode == "system", |b| b.primary())
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.set_fetch_proxy_mode("system", cx)
                                })),
                        )
                        .child(
                            Button::new("proxy-mode-direct")
                                .ghost()
                                .label("直连（不走代理）")
                                .when(mode == "direct", |b| b.primary())
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.set_fetch_proxy_mode("direct", cx)
                                })),
                        )
                        .child(
                            Button::new("proxy-mode-custom")
                                .ghost()
                                .label("自定义代理")
                                .when(mode == "custom", |b| b.primary())
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.set_fetch_proxy_mode("custom", cx)
                                })),
                        ),
                )
                .into_any_element(),
        ];
        if mode == "custom" {
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
                            .child("代理服务器"),
                    )
                    .child(
                        h_flex()
                            .flex_1()
                            .min_w_0()
                            .gap_2()
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .child(Input::new(&self.p1.fetch_proxy_url).cleanable(true)),
                            )
                            .child(
                                Button::new("save-fetch-proxy-url")
                                    .primary()
                                    .label("保存代理")
                                    .on_click(
                                        cx.listener(|this, _, _, cx| this.save_fetch_proxy_url(cx)),
                                    ),
                            ),
                    )
                    .into_any_element(),
            );
        }
        rows.push(
            div()
                .text_size(px(12.))
                .text_color(rgb(MUTED))
                .child("用于订阅刷新、规则集内容读取、内置规则更新以及 GitHub 版本检查。")
                .into_any_element(),
        );
        settings_section("网络抓取代理", rows, div())
    }

    /// Compact batch-action dropdown for the generator node sidebar.
    pub(super) fn node_batch_menu(&self, cx: &mut Context<Self>) -> AnyElement {
        let view = cx.entity().clone();
        Button::new("node-batch-actions")
            .outline()
            .icon(batch_icon())
            .tooltip("批量操作当前筛选的节点")
            .dropdown_menu(move |menu, _, _| {
                let enable_view = view.clone();
                let disable_view = view.clone();
                let invert_view = view.clone();
                let rename_view = view.clone();
                menu.item(
                    PopupMenuItem::new("启用筛选节点").on_click(move |_, window, cx| {
                        enable_view.update(cx, |this, cx| {
                            this.batch_set_filtered_nodes_enabled(Some(true), window, cx)
                        });
                    }),
                )
                .item(
                    PopupMenuItem::new("停用筛选节点").on_click(move |_, window, cx| {
                        disable_view.update(cx, |this, cx| {
                            this.batch_set_filtered_nodes_enabled(Some(false), window, cx)
                        });
                    }),
                )
                .item(
                    PopupMenuItem::new("反转启停状态").on_click(move |_, window, cx| {
                        invert_view.update(cx, |this, cx| {
                            this.batch_set_filtered_nodes_enabled(None, window, cx)
                        });
                    }),
                )
                .separator()
                .item(
                    PopupMenuItem::new("批量重命名…").on_click(move |_, window, cx| {
                        rename_view.update(cx, |this, cx| this.open_batch_rename(window, cx));
                    }),
                )
            })
            .into_any_element()
    }

    /// Render all active Phase 2 modals in overlay order.
    pub(super) fn render_p1_modals(&self, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let mut overlays = Vec::new();
        if let Some(modal) = &self.p1.qr_modal {
            overlays.push(self.render_qr_modal(modal, cx));
        }
        if let Some(modal) = &self.p1.history_modal {
            overlays.push(self.render_history_modal(modal, cx));
        }
        if let Some(preview) = &self.p1.backup_preview {
            overlays.push(self.render_backup_preview_modal(preview, cx));
        }
        if self.p1.batch_rename_open {
            overlays.push(self.render_batch_rename_modal(cx));
        }
        if self.p1.sub_logs_open {
            overlays.push(self.render_sub_logs_modal(cx));
        }
        overlays
    }

    fn render_qr_modal(
        &self,
        modal: &crate::app::phase2::QrModal,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let canvas_px = 208.0_f32;
        let cell = canvas_px / (modal.size.max(1) as f32);
        let mut matrix = div().relative().w(px(canvas_px)).h(px(canvas_px));
        for &(y, x, len) in &modal.runs {
            matrix = matrix.child(
                div()
                    .absolute()
                    .left(px(x as f32 * cell))
                    .top(px(y as f32 * cell))
                    .w(px(len as f32 * cell + 0.35))
                    .h(px(cell + 0.35))
                    .bg(gpui_kit::rgb(0x111827)),
            );
        }
        let content_copy = modal.content.clone();
        let panel = v_flex()
            .w(px(440.))
            .max_w(relative(0.92))
            .max_h(relative(0.90))
            .p_4()
            .gap_3()
            .rounded(px(12.))
            .bg(rgb(SURFACE))
            .border_1()
            .border_color(rgb(BORDER))
            .child(
                h_flex()
                    .justify_between()
                    .gap_2()
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .text_size(px(15.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .truncate()
                            .child(modal.title.clone()),
                    )
                    .child(object_action(
                        "close-qr-modal",
                        close_icon(),
                        "关闭",
                        cx,
                        |this, _, cx| {
                            this.p1.qr_modal = None;
                            cx.notify();
                        },
                    )),
            )
            .when(modal.sensitive, |panel| {
                panel.child(banner(modal.subtitle.clone(), WARNING))
            })
            .when(!modal.sensitive, |panel| {
                panel.child(
                    div()
                        .text_size(px(12.))
                        .text_color(rgb(MUTED))
                        .child(modal.subtitle.clone()),
                )
            })
            .child(
                h_flex().justify_center().py_2().child(
                    div()
                        .p(px(14.))
                        .rounded(px(10.))
                        .bg(gpui_kit::rgb(0xffffff))
                        .border_1()
                        .border_color(rgb(BORDER_ITEM))
                        .child(matrix),
                ),
            )
            .child(
                div()
                    .w_full()
                    .p_2()
                    .rounded(px(6.))
                    .bg(rgb(SURFACE_INSET))
                    .border_1()
                    .border_color(rgb(BORDER))
                    .font_family("Consolas")
                    .text_size(px(11.5))
                    .text_color(rgb(MUTED))
                    .truncate()
                    .child(modal.content.clone()),
            )
            .child(
                h_flex()
                    .justify_end()
                    .gap_2()
                    .child(
                        Button::new("qr-export-svg")
                            .outline()
                            .label("导出 SVG")
                            .on_click(cx.listener(|this, _, _, cx| this.export_qr_svg(cx))),
                    )
                    .child(
                        Button::new("qr-copy-content")
                            .primary()
                            .label("复制链接")
                            .on_click(cx.listener(move |this, _, _, cx| {
                                cx.write_to_clipboard(ClipboardItem::new_string(
                                    content_copy.clone(),
                                ));
                                this.show_toast("链接已复制到剪贴板".into(), cx);
                            })),
                    ),
            );
        modal_with_backdrop("qr-modal-backdrop", panel, cx, |this, _, cx| {
            this.p1.qr_modal = None;
            cx.notify();
        })
    }

    fn render_history_modal(
        &self,
        modal: &crate::app::phase2::HistoryModal,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let mut list = v_flex().gap_2();
        for (idx, item) in modal.revisions.iter().enumerate() {
            let rev_id = item.revision.id;
            let is_latest = idx == 0;
            list = list.child(
                h_flex()
                    .min_w_0()
                    .p_3()
                    .gap_3()
                    .rounded(px(8.))
                    .border_1()
                    .border_color(rgb(if is_latest { ACCENT } else { BORDER }))
                    .bg(rgb(SURFACE_2))
                    .child(
                        v_flex()
                            .flex_1()
                            .min_w_0()
                            .gap_1()
                            .child(
                                h_flex()
                                    .gap_2()
                                    .child(
                                        div()
                                            .text_sm()
                                            .font_weight(FontWeight::MEDIUM)
                                            .text_color(rgb(TEXT))
                                            .child(item.revision.reason.clone()),
                                    )
                                    .when(is_latest, |row| row.child(micro_badge("最新", true))),
                            )
                            .child(div().text_size(px(11.5)).text_color(rgb(MUTED)).child(
                                format!(
                                    "{} · {} 节点 · {} 策略组 · {} 规则",
                                    format_timestamp(Some(item.revision.created_at)),
                                    item.proxies,
                                    item.groups,
                                    item.rules
                                ),
                            )),
                    )
                    .child(
                        Button::new(format!("restore-rev-{rev_id}"))
                            .outline()
                            .disabled(is_latest)
                            .label(if is_latest {
                                "当前版本"
                            } else {
                                "恢复此版本"
                            })
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.restore_config_revision(rev_id, window, cx)
                            })),
                    ),
            );
        }
        if modal.revisions.is_empty() {
            list = list.child(
                div()
                    .py_6()
                    .text_center()
                    .text_sm()
                    .text_color(rgb(MUTED))
                    .child("暂无历史版本记录"),
            );
        }
        let panel = v_flex()
            .w(px(560.))
            .max_w(relative(0.92))
            .max_h(relative(0.86))
            .p_4()
            .gap_3()
            .rounded(px(12.))
            .bg(rgb(SURFACE))
            .border_1()
            .border_color(rgb(BORDER))
            .child(
                h_flex()
                    .justify_between()
                    .gap_2()
                    .child(
                        v_flex()
                            .flex_1()
                            .min_w_0()
                            .child(
                                div()
                                    .text_size(px(15.))
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .truncate()
                                    .child(format!("历史版本 · {}", modal.config_name)),
                            )
                            .child(div().text_size(px(11.5)).text_color(rgb(MUTED)).child(
                                "保留最近 20 个版本；恢复后沿用原订阅地址，恢复前状态自动留存",
                            )),
                    )
                    .child(object_action(
                        "close-history-modal",
                        close_icon(),
                        "关闭",
                        cx,
                        |this, _, cx| {
                            this.p1.history_modal = None;
                            cx.notify();
                        },
                    )),
            )
            .child(
                div()
                    .id("history-modal-scroll")
                    .flex_1()
                    .min_h(px(180.))
                    .max_h(px(440.))
                    .overflow_y_scrollbar()
                    .child(list),
            );
        modal_with_backdrop("history-modal-backdrop", panel, cx, |this, _, cx| {
            this.p1.history_modal = None;
            cx.notify();
        })
    }

    fn render_backup_preview_modal(
        &self,
        preview: &crate::app::phase2::BackupPreview,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let panel = v_flex()
            .w(px(460.))
            .max_w(relative(0.92))
            .p_4()
            .gap_3()
            .rounded(px(12.))
            .bg(rgb(SURFACE))
            .border_1()
            .border_color(rgb(BORDER))
            .child(
                div()
                    .text_size(px(15.))
                    .font_weight(FontWeight::SEMIBOLD)
                    .child("确认从备份恢复"),
            )
            .child(info_row(
                "备份文件",
                preview.path.display().to_string(),
                true,
            ))
            .child(info_row(
                "包含内容",
                format!(
                    "{} 个订阅源 · {} 份配置 · {} 个自定义模板",
                    preview.subscriptions, preview.saved_configs, preview.templates
                ),
                false,
            ))
            .child(banner(
                "确认后将在下次启动 SwiftSub 时替换当前数据库，并自动保留原数据为 swiftsub.db.bak。".into(),
                WARNING,
            ))
            .child(
                h_flex()
                    .justify_end()
                    .gap_2()
                    .child(
                        Button::new("cancel-backup-restore")
                            .outline()
                            .label("取消")
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.p1.backup_preview = None;
                                cx.notify();
                            })),
                    )
                    .child(
                        Button::new("confirm-backup-restore")
                            .primary()
                            .label("确认并准备恢复")
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.confirm_backup_restore(cx)
                            })),
                    ),
            );
        modal_with_backdrop("backup-preview-backdrop", panel, cx, |this, _, cx| {
            this.p1.backup_preview = None;
            cx.notify();
        })
    }

    fn render_batch_rename_modal(&self, cx: &mut Context<Self>) -> AnyElement {
        let query = self.search.read(cx).value().trim().to_lowercase();
        let matched = self
            .nodes
            .iter()
            .filter(|node| {
                query.is_empty()
                    || node.name().to_lowercase().contains(&query)
                    || node.proxy.kind().to_lowercase().contains(&query)
                    || node.source_name.to_lowercase().contains(&query)
            })
            .count();
        let panel = v_flex()
            .w(px(440.))
            .max_w(relative(0.92))
            .p_4()
            .gap_3()
            .rounded(px(12.))
            .bg(rgb(SURFACE))
            .border_1()
            .border_color(rgb(BORDER))
            .child(
                h_flex()
                    .justify_between()
                    .child(
                        div()
                            .text_size(px(15.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(format!("批量重命名（当前匹配 {matched} 个节点）")),
                    )
                    .child(object_action(
                        "close-batch-rename",
                        close_icon(),
                        "关闭",
                        cx,
                        |this, _, cx| {
                            this.p1.batch_rename_open = false;
                            cx.notify();
                        },
                    )),
            )
            .when_some(self.p1.batch_rename_error.clone(), |panel, error| {
                panel.child(banner(error, DANGER))
            })
            .child(
                v_flex()
                    .gap_1()
                    .child(field_label("统一添加前缀（可选）"))
                    .child(Input::new(&self.p1.batch_rename_prefix).cleanable(true)),
            )
            .child(
                v_flex()
                    .gap_1()
                    .child(field_label("查找文本（可选）"))
                    .child(Input::new(&self.p1.batch_rename_find).cleanable(true)),
            )
            .child(
                v_flex()
                    .gap_1()
                    .child(field_label("替换为（留空则删除匹配文本）"))
                    .child(Input::new(&self.p1.batch_rename_replace).cleanable(true)),
            )
            .child(
                div()
                    .text_size(px(11.5))
                    .text_color(rgb(MUTED))
                    .child("重命名后会自动同步策略组与规则引用，并保留原有节点顺序。"),
            )
            .child(
                h_flex()
                    .justify_end()
                    .gap_2()
                    .child(
                        Button::new("cancel-batch-rename")
                            .outline()
                            .label("取消")
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.p1.batch_rename_open = false;
                                cx.notify();
                            })),
                    )
                    .child(
                        Button::new("apply-batch-rename")
                            .primary()
                            .label("应用重命名")
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.apply_batch_rename(window, cx)
                            })),
                    ),
            );
        modal_with_backdrop("batch-rename-backdrop", panel, cx, |this, _, cx| {
            this.p1.batch_rename_open = false;
            cx.notify();
        })
    }

    fn render_sub_logs_modal(&self, cx: &mut Context<Self>) -> AnyElement {
        let now = swiftsub_store::now_ts();
        let mut subs_list = v_flex().gap_2();
        let remote_subs: Vec<_> = self
            .subscriptions
            .iter()
            .filter(|sub| sub.url.is_some())
            .collect();
        for sub in &remote_subs {
            let sub_id = sub.id;
            let refreshing = self.refreshing.contains(&sub_id);
            let used = sub.traffic_upload.saturating_add(sub.traffic_download);
            let total = sub.traffic_total;
            let usage_pct = if total > 0 {
                ((used as f64 / total as f64) * 100.0).min(100.0)
            } else {
                0.0
            };
            let traffic_warn = total > 0 && usage_pct >= 90.0;
            let days_left = sub.expire_at.map(|exp| (exp - now) / 86_400);
            let expire_warn = days_left.is_some_and(|days| days <= 7);
            let expire_label = match (sub.expire_at, days_left) {
                (Some(ts), Some(days)) if days < 0 => {
                    format!("已到期（{}）", format_timestamp(Some(ts)))
                }
                (Some(ts), Some(days)) => {
                    format!("剩余 {days} 天（{}）", format_timestamp(Some(ts)))
                }
                _ => "无到期信息".into(),
            };
            subs_list = subs_list.child(
                v_flex()
                    .p_3()
                    .gap_2()
                    .rounded(px(8.))
                    .border_1()
                    .border_color(rgb(if sub.last_error.is_some() || traffic_warn {
                        DANGER
                    } else if expire_warn {
                        WARNING
                    } else {
                        BORDER
                    }))
                    .bg(rgb(SURFACE_2))
                    .child(
                        h_flex()
                            .justify_between()
                            .gap_2()
                            .child(
                                h_flex()
                                    .flex_1()
                                    .min_w_0()
                                    .gap_2()
                                    .child(
                                        div()
                                            .text_sm()
                                            .font_weight(FontWeight::MEDIUM)
                                            .truncate()
                                            .child(sub.name.clone()),
                                    )
                                    .child(micro_badge(format!("{} 节点", sub.node_count), false))
                                    .when(traffic_warn, |row| {
                                        row.child(micro_badge("流量告警", true))
                                    })
                                    .when(expire_warn, |row| {
                                        row.child(micro_badge("临近到期", true))
                                    }),
                            )
                            .child(
                                Button::new(format!("sub-log-refresh-{sub_id}"))
                                    .outline()
                                    .disabled(refreshing)
                                    .label(if refreshing { "刷新中…" } else { "刷新" })
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        this.refresh_subscription(sub_id, cx)
                                    })),
                            ),
                    )
                    .when(total > 0, |card| {
                        card.child(
                            v_flex()
                                .gap_1()
                                .child(
                                    h_flex()
                                        .justify_between()
                                        .text_size(px(11.5))
                                        .text_color(rgb(if traffic_warn { DANGER } else { MUTED }))
                                        .child(format!(
                                            "流量：{} / {} ({:.1}%)",
                                            format_bytes_human(used),
                                            format_bytes_human(total),
                                            usage_pct
                                        ))
                                        .child(expire_label.clone()),
                                )
                                .child(
                                    div()
                                        .w_full()
                                        .h(px(5.))
                                        .rounded(px(3.))
                                        .bg(rgb(SURFACE_3))
                                        .child(
                                            div()
                                                .h_full()
                                                .w(relative((usage_pct / 100.0) as f32))
                                                .rounded(px(3.))
                                                .bg(rgb(if traffic_warn {
                                                    DANGER
                                                } else {
                                                    ACCENT
                                                })),
                                        ),
                                ),
                        )
                    })
                    .when(total == 0, |card| {
                        card.child(
                            div()
                                .text_size(px(11.5))
                                .text_color(rgb(if expire_warn { WARNING } else { MUTED }))
                                .child(format!(
                                    "上次成功：{} · {}",
                                    format_timestamp(sub.last_ok_at),
                                    expire_label
                                )),
                        )
                    })
                    .when_some(sub.last_error.clone(), |card, err| {
                        card.child(
                            div()
                                .text_size(px(11.5))
                                .text_color(rgb(DANGER))
                                .child(format!("最近错误：{err}")),
                        )
                    }),
            );
        }
        if remote_subs.is_empty() {
            subs_list = subs_list.child(
                div()
                    .py_3()
                    .text_size(px(12.5))
                    .text_color(rgb(MUTED))
                    .child("暂无远程订阅源（手动导入的节点无需远程刷新）"),
            );
        }

        let mut logs_list = v_flex().gap_1();
        for log in &self.p1.refresh_logs {
            let sub_name = log
                .sub_id
                .and_then(|id| self.subscriptions.iter().find(|s| s.id == id))
                .map(|s| s.name.as_str())
                .unwrap_or("已删除订阅");
            let msg = log
                .message
                .as_deref()
                .filter(|m| !m.is_empty())
                .unwrap_or(if log.ok {
                    "更新成功"
                } else {
                    "更新失败"
                });
            logs_list = logs_list.child(
                h_flex()
                    .gap_2()
                    .py_1()
                    .border_b_1()
                    .border_color(rgb(BORDER))
                    .text_size(px(11.5))
                    .child(
                        div()
                            .w(px(118.))
                            .flex_shrink_0()
                            .font_family("Consolas")
                            .text_color(rgb(MUTED))
                            .child(format_timestamp(Some(log.at))),
                    )
                    .child(
                        div()
                            .w(px(36.))
                            .flex_shrink_0()
                            .text_color(rgb(if log.ok { ACCENT } else { DANGER }))
                            .child(if log.ok { "成功" } else { "失败" }),
                    )
                    .child(
                        div()
                            .w(px(110.))
                            .flex_shrink_0()
                            .truncate()
                            .text_color(rgb(TEXT))
                            .child(sub_name.to_string()),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .text_color(rgb(MUTED))
                            .child(msg.to_string()),
                    ),
            );
        }
        if self.p1.refresh_logs.is_empty() {
            logs_list = logs_list.child(
                div()
                    .py_3()
                    .text_size(px(12.))
                    .text_color(rgb(MUTED))
                    .child("暂无刷新日志"),
            );
        }

        let panel = v_flex()
            .w(px(640.))
            .max_w(relative(0.94))
            .max_h(relative(0.88))
            .p_4()
            .gap_3()
            .rounded(px(12.))
            .bg(rgb(SURFACE))
            .border_1()
            .border_color(rgb(BORDER))
            .child(
                h_flex()
                    .justify_between()
                    .gap_2()
                    .child(
                        div()
                            .text_size(px(15.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .child("订阅状态与刷新日志"),
                    )
                    .child(
                        h_flex()
                            .gap_2()
                            .child(
                                Button::new("sub-logs-refresh-all")
                                    .outline()
                                    .label("全部刷新")
                                    .on_click(cx.listener(|this, _, _, cx| this.refresh_all(cx))),
                            )
                            .child(object_action(
                                "close-sub-logs",
                                close_icon(),
                                "关闭",
                                cx,
                                |this, _, cx| {
                                    this.p1.sub_logs_open = false;
                                    cx.notify();
                                },
                            )),
                    ),
            )
            .child(
                div()
                    .id("sub-logs-scroll")
                    .flex_1()
                    .min_h(px(220.))
                    .max_h(px(480.))
                    .overflow_y_scrollbar()
                    .child(
                        v_flex()
                            .gap_3()
                            .child(field_label("远程订阅流量与到期状态"))
                            .child(subs_list)
                            .child(field_label("最近刷新日志"))
                            .child(logs_list),
                    ),
            );
        modal_with_backdrop("sub-logs-backdrop", panel, cx, |this, _, cx| {
            this.p1.sub_logs_open = false;
            cx.notify();
        })
    }

    pub(super) fn node_share_action(&self, key: String, cx: &mut Context<Self>) -> AnyElement {
        div()
            .id(format!("share-node-{key}"))
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
            .tooltip(|window, cx| Tooltip::new("分享节点链接 / 二维码").build(window, cx))
            .child(qr_icon().with_size(px(15.)))
            .on_click(cx.listener(move |this, _, _, cx| {
                this.open_node_share_qr(key.clone(), cx);
            }))
            .into_any_element()
    }
}
