mod saved_configs;
use gpui_kit::base::{Disableable as _, InteractiveElementExt as _};
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::input::{Input, Textarea};
use gpui_kit::component::resizable::{h_resizable, resizable_panel, v_resizable};
use gpui_kit::component::scroll::ScrollableElement as _;
use gpui_kit::component::switch::Switch;
use gpui_kit::component::{Sizable as _, StyledExt as _, TitleBar, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use swiftsub_core::catalog::{self, Module, ModuleKind, Preset};
use swiftsub_core::model::{GroupKind, Node};
use swiftsub_core::region::Region;
use swiftsub_store::Subscription as StoredSubscription;

use crate::app::{
    AddMode, ChainSide, CustomDialog, Page, SettingsTab, SwiftSubApp, TemplateTab, format_bytes,
    format_expire, format_timestamp, template_summary_counts,
};
use crate::rule_catalog;

mod colors {
    pub const BG: u32 = 0x0e1014;
    pub const SURFACE: u32 = 0x14171d;
    pub const SURFACE_2: u32 = 0x191d25;
    pub const SURFACE_3: u32 = 0x202633;
    pub const BORDER: u32 = 0x262b36;
    pub const TEXT: u32 = 0xe6e8ee;
    pub const MUTED: u32 = 0x8a93a6;
    pub const ACCENT: u32 = 0x5b8cff;
    pub const ACCENT_SOFT: u32 = 0x17233d;
    pub const SUCCESS: u32 = 0x3ddc97;
    pub const WARNING: u32 = 0xffb020;
    pub const DANGER: u32 = 0xff5c5c;
    pub const DANGER_SOFT: u32 = 0x2a171a;
}

use colors::*;

use std::sync::atomic::{AtomicBool, Ordering};

static LIGHT_THEME: AtomicBool = AtomicBool::new(false);

pub fn set_light_theme(light: bool) {
    LIGHT_THEME.store(light, Ordering::Relaxed);
}

fn theme_color(color: u32) -> u32 {
    if !LIGHT_THEME.load(Ordering::Relaxed) {
        return color;
    }
    match color {
        BG => 0xf3f5fa,
        SURFACE => 0xffffff,
        SURFACE_2 => 0xf1f4f9,
        SURFACE_3 => 0xe8edf5,
        BORDER => 0xdbe2ec,
        TEXT => 0x1f2937,
        MUTED => 0x5f6c82,
        ACCENT => 0x3b6ef5,
        ACCENT_SOFT => 0xe8efff,
        SUCCESS => 0x0f9d63,
        WARNING => 0xb45309,
        DANGER => 0xd92d2d,
        DANGER_SOFT => 0xfdecec,
        0x05070bcc => 0x1f29374d,
        0x11151b => 0xf5f8fd,
        0x2a2110 => 0xfff7e6,
        0x5b4316 => 0xf0d49a,
        0x5b2328 => 0xf3c6c6,
        0xc9d1e0 => 0x25324a,
        other => other,
    }
}

fn rgb(color: u32) -> Rgba {
    gpui_kit::rgb(theme_color(color))
}

fn rgba(color: u32) -> Rgba {
    gpui_kit::rgba(theme_color(color))
}

impl Render for SwiftSubApp {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .id("swiftsub-root")
            .capture_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
                if event.keystroke.modifiers.control && event.keystroke.key == "f" {
                    if this.template_dialog_open {
                        this.set_template_tab(TemplateTab::RuleSets, cx);
                    } else if this.page != Page::Generator {
                        return;
                    }
                    let input = if this.template_dialog_open {
                        &this.template_rule_search
                    } else {
                        &this.search
                    };
                    let handle = input.read(cx).focus_handle(cx);
                    window.focus(&handle, cx);
                    cx.stop_propagation();
                } else if event.keystroke.key == "escape" {
                    if this.qr_content.take().is_some() || this.diagnostics_open {
                        this.diagnostics_open = false;
                        cx.notify();
                        cx.stop_propagation();
                    } else if this.template_target_picker.take().is_some() {
                        cx.notify();
                        cx.stop_propagation();
                    } else if this.template_dialog_open {
                        this.request_close_template(window, cx);
                        cx.stop_propagation();
                    }
                }
            }))
            .relative()
            .size_full()
            .bg(rgb(BG))
            .text_color(rgb(TEXT))
            .v_flex()
            .child(self.header())
            .child(self.nav_bar(cx))
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .child(self.page_view(window.viewport_size().width, cx)),
            )
            .when(self.add_open, |this| this.child(self.add_dialog(cx)))
            .when_some(self.confirm_delete, |this, id| {
                this.child(self.confirm_delete_dialog(id, cx))
            })
            .when(self.custom_dialog != CustomDialog::None, |this| {
                this.child(self.custom_dialog_view(cx))
            })
            .when(self.batch_import_open, |this| {
                this.child(self.batch_import_dialog(cx))
            })
            .when_some(self.edit_subscription, |this, id| {
                this.child(self.edit_subscription_dialog(id, cx))
            })
            .when(self.batch_rename_open, |this| {
                this.child(self.batch_rename_dialog(cx))
            })
            .when(self.node_rename_key.is_some(), |this| {
                this.child(self.node_rename_dialog(cx))
            })
            .when(
                self.template_dialog_open && self.page != Page::Generator,
                |this| this.child(self.template_dialog(cx)),
            )
            .when(self.qr_content.is_some(), |this| {
                this.child(self.qr_dialog(cx))
            })
            .when(self.diagnostics_open, |this| {
                this.child(self.diagnostics_dialog(cx))
            })
            .when_some(self.toast.clone(), |this, toast| {
                this.child(
                    div()
                        .absolute()
                        .bottom(px(20.))
                        .right(px(20.))
                        .occlude()
                        .px_3()
                        .py_2()
                        .rounded(px(10.))
                        .bg(rgb(SURFACE_3))
                        .border_1()
                        .border_color(rgb(BORDER))
                        .text_size(px(13.5))
                        .text_color(rgb(TEXT))
                        .child(toast),
                )
            })
    }
}

impl SwiftSubApp {
    fn page_view(&mut self, width: Pixels, cx: &mut Context<Self>) -> AnyElement {
        match self.page {
            Page::Subscriptions => self.subscriptions_page(cx),
            Page::Generator => self.generator_page(width, cx),
            Page::Templates => self.templates_page(cx),
            Page::SavedConfigs => self.saved_configs_page(cx),
            Page::Settings => self.settings_page(width, cx),
        }
    }

    fn header(&self) -> TitleBar {
        TitleBar::new().child(
            h_flex().h_full().w_full().gap_3().child(
                div()
                    .text_size(px(14.5))
                    .font_weight(FontWeight::SEMIBOLD)
                    .child("SwiftSub"),
            ),
        )
    }

    fn nav_bar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let (server_ready, port) = match &self.server {
            Some(server) => (true, Some(server.local_addr().port())),
            None => (false, None),
        };
        let status_text = match port {
            Some(port) => format!("本地 · 127.0.0.1:{port}"),
            None => "本地服务未启动".to_string(),
        };

        h_flex()
            .h(px(50.))
            .px(px(24.))
            .gap_2()
            .bg(rgb(SURFACE))
            .border_b_1()
            .border_color(rgb(BORDER))
            .child(self.nav_item("订阅源", Page::Subscriptions, cx))
            .child(self.nav_item("配置生成", Page::Generator, cx))
            .child(self.nav_item("我的配置", Page::SavedConfigs, cx))
            .child(self.nav_item("模板", Page::Templates, cx))
            .child(self.nav_item("设置", Page::Settings, cx))
            .child(div().flex_1())
            .child(
                h_flex()
                    .gap_2()
                    .child(
                        h_flex()
                            .gap_2()
                            .px_3()
                            .h(px(26.))
                            .rounded_full()
                            .bg(rgb(SURFACE_2))
                            .border_1()
                            .border_color(rgb(BORDER))
                            .child(
                                div()
                                    .w(px(6.))
                                    .h(px(6.))
                                    .rounded_full()
                                    .bg(rgb(if server_ready { SUCCESS } else { DANGER })),
                            )
                            .child(
                                div()
                                    .text_size(px(11.5))
                                    .text_color(rgb(MUTED))
                                    .child(status_text),
                            ),
                    )
                    .child(icon_action(
                        "theme-toggle",
                        if self.light_theme { "☾" } else { "☀" },
                        MUTED,
                        cx,
                        |this, _, cx| this.set_theme(!this.light_theme, cx),
                    ))
                    .when(self.server_url.is_some(), |this| {
                        this.child(text_action(
                            "copy-server-header",
                            "复制地址",
                            ACCENT,
                            cx,
                            |this, _, cx| this.copy_server_url(cx),
                        ))
                    }),
            )
    }

    fn nav_item(&self, label: &'static str, page: Page, cx: &mut Context<Self>) -> AnyElement {
        let active = self.page == page;
        div()
            .id(format!("nav-{label}"))
            .cursor_pointer()
            .h(px(32.))
            .px(px(14.))
            .rounded_full()
            .flex()
            .items_center()
            .border_1()
            .border_color(rgb(if active { ACCENT } else { SURFACE }))
            .bg(rgb(if active { ACCENT_SOFT } else { SURFACE }))
            .text_size(px(13.))
            .text_color(rgb(if active { TEXT } else { MUTED }))
            .hover(|style| style.bg(rgb(SURFACE_2)).text_color(rgb(TEXT)))
            .child(label)
            .on_click(cx.listener(move |this, _, window, cx| this.set_page(page, window, cx)))
            .into_any_element()
    }

    fn page_title(&self, title: &'static str, actions: impl IntoElement) -> Div {
        h_flex()
            .justify_between()
            .gap_4()
            .child(
                div()
                    .text_size(px(22.))
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(title),
            )
            .child(actions)
    }

    fn metric_card(&self, label: &str, value: String) -> Div {
        v_flex()
            .flex_1()
            .h(px(72.))
            .px_3()
            .py_2()
            .gap_2()
            .rounded(px(10.))
            .bg(rgb(SURFACE))
            .border_1()
            .border_color(rgb(BORDER))
            .child(
                div()
                    .text_size(px(11.5))
                    .text_color(rgb(MUTED))
                    .child(label.to_string()),
            )
            .child(
                div()
                    .text_size(px(21.))
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(value),
            )
    }

    fn subscriptions_page(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let (enabled, total) = self.subscriptions_summary();
        let (used, total_traffic) = self.aggregate_traffic();
        let traffic_value = if total_traffic > 0 {
            format!("{} / {}", format_bytes(used), format_bytes(total_traffic))
        } else {
            format_bytes(used)
        };

        let mut actions = h_flex().gap_2();
        if self
            .subscriptions
            .iter()
            .any(|subscription| subscription.url.is_some())
        {
            actions = actions.child(
                Button::new("refresh-all")
                    .ghost()
                    .label("刷新全部")
                    .on_click(cx.listener(|this, _, _, cx| this.refresh_all(cx))),
            );
        }
        actions = actions
            .child(
                Button::new("batch-import-open")
                    .ghost()
                    .label("批量导入")
                    .on_click(cx.listener(|this, _, _, cx| this.open_batch_import(cx))),
            )
            .child(
                Button::new("demo-subscription")
                    .ghost()
                    .label("加载示例")
                    .on_click(cx.listener(|this, _, _, cx| this.load_demo(cx))),
            )
            .child(
                Button::new("add-subscription")
                    .primary()
                    .label("添加订阅")
                    .on_click(cx.listener(|this, _, _, cx| this.open_add_dialog(cx))),
            );

        let mut page = v_flex()
            .size_full()
            .min_h_0()
            .p(px(24.))
            .gap_4()
            .child(self.page_title("订阅源", actions))
            .child(
                h_flex()
                    .gap_3()
                    .child(self.metric_card("订阅", format!("{enabled} / {total}")))
                    .child(self.metric_card("节点", self.nodes.len().to_string()))
                    .child(self.metric_card("已用流量", traffic_value))
                    .child(
                        self.metric_card(
                            "本地服务",
                            if self.server.is_some() {
                                "运行中"
                            } else {
                                "未启动"
                            }
                            .to_string(),
                        ),
                    ),
            );

        if let Some(error) = &self.store_error {
            page = page.child(banner(error.clone(), WARNING));
        }
        if let Some(error) = &self.server_error {
            page = page.child(banner(format!("本地服务启动失败：{error}"), DANGER));
        }

        if self.subscriptions.is_empty() {
            page = page.child(self.empty_subscriptions(cx));
        } else {
            page = page.child(self.subscription_table(cx));
        }

        page.into_any_element()
    }

    fn empty_subscriptions(&self, cx: &mut Context<Self>) -> AnyElement {
        div()
            .flex_1()
            .min_h_0()
            .rounded(px(12.))
            .border_1()
            .border_color(rgb(BORDER))
            .bg(rgb(SURFACE))
            .flex()
            .items_center()
            .justify_center()
            .child(
                v_flex()
                    .items_center()
                    .gap_3()
                    .child(
                        div()
                            .w(px(46.))
                            .h(px(46.))
                            .rounded(px(12.))
                            .bg(rgb(ACCENT_SOFT))
                            .border_1()
                            .border_color(rgb(ACCENT))
                            .flex()
                            .items_center()
                            .justify_center()
                            .text_size(px(18.))
                            .font_weight(FontWeight::BOLD)
                            .text_color(rgb(ACCENT))
                            .child("+"),
                    )
                    .child(
                        div()
                            .text_size(px(16.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .child("还没有订阅"),
                    )
                    .child(
                        div()
                            .text_size(px(13.))
                            .text_color(rgb(MUTED))
                            .child("添加机场订阅链接，或先加载示例数据体验完整流程"),
                    )
                    .child(
                        h_flex()
                            .gap_2()
                            .child(
                                Button::new("empty-add")
                                    .primary()
                                    .label("添加订阅")
                                    .on_click(
                                        cx.listener(|this, _, _, cx| this.open_add_dialog(cx)),
                                    ),
                            )
                            .child(
                                Button::new("empty-batch")
                                    .ghost()
                                    .label("批量导入节点")
                                    .on_click(
                                        cx.listener(|this, _, _, cx| this.open_batch_import(cx)),
                                    ),
                            )
                            .child(
                                Button::new("empty-demo")
                                    .ghost()
                                    .label("加载示例")
                                    .on_click(cx.listener(|this, _, _, cx| this.load_demo(cx))),
                            ),
                    ),
            )
            .into_any_element()
    }

    fn subscription_table(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let mut panel = v_flex()
            .flex_1()
            .min_h_0()
            .rounded(px(12.))
            .border_1()
            .border_color(rgb(BORDER))
            .bg(rgb(SURFACE));

        panel = panel.child(
            h_flex()
                .h(px(42.))
                .px_4()
                .gap_3()
                .border_b_1()
                .border_color(rgb(BORDER))
                .text_size(px(11.5))
                .text_color(rgb(MUTED))
                .child(div().flex_1().min_w_0().child("订阅"))
                .child(div().w(px(66.)).child("节点"))
                .child(div().w(px(160.)).child("流量"))
                .child(div().w(px(100.)).child("最近更新"))
                .child(div().w(px(58.)).child("状态"))
                .child(div().w(px(152.)).child("操作")),
        );

        let mut list = div()
            .id("subscription-list")
            .flex_1()
            .min_h_0()
            .overflow_y_scrollbar();

        let subscriptions = self.subscriptions.clone();
        for subscription in &subscriptions {
            list = list.child(self.subscription_row(subscription, cx));
        }

        panel.child(list).into_any_element()
    }

    fn subscription_row(
        &self,
        subscription: &StoredSubscription,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let id = subscription.id;
        let enabled = subscription.enabled;
        let refreshing = self.refreshing.contains(&id);
        let kind = match subscription.kind {
            swiftsub_store::SubscriptionKind::Url => "订阅链接",
            swiftsub_store::SubscriptionKind::Yaml => "YAML",
            swiftsub_store::SubscriptionKind::Links => "节点链接",
            swiftsub_store::SubscriptionKind::File => "本地文件",
        };
        let used = subscription
            .traffic_upload
            .saturating_add(subscription.traffic_download);
        let traffic = if subscription.traffic_total > 0 {
            format!(
                "{} / {}",
                format_bytes(used),
                format_bytes(subscription.traffic_total)
            )
        } else {
            "—".to_string()
        };
        let expire = subscription
            .expire_at
            .map(|value| format!("到期 {}", format_expire(Some(value))))
            .unwrap_or_else(|| "到期 —".to_string());
        let detail = subscription.last_error.clone().unwrap_or_else(|| {
            subscription
                .url
                .clone()
                .unwrap_or_else(|| "手动导入".to_string())
        });

        let mut actions = h_flex().gap_1();
        actions = actions.child(icon_action(
            format!("edit-{id}"),
            "✎",
            ACCENT,
            cx,
            move |this, window, cx| this.open_edit_subscription(id, window, cx),
        ));
        if subscription.url.is_some() {
            if refreshing {
                actions = actions.child(
                    div()
                        .px_2()
                        .py_1()
                        .text_size(px(11.5))
                        .text_color(rgb(ACCENT))
                        .child("刷新中"),
                );
            } else {
                actions = actions.child(text_action(
                    format!("refresh-{id}"),
                    "刷新",
                    ACCENT,
                    cx,
                    move |this, _, cx| this.refresh_subscription(id, cx),
                ));
            }
        }
        actions = actions.child(icon_action(
            format!("delete-{id}"),
            "×",
            DANGER,
            cx,
            move |this, _, cx| this.ask_delete(id, cx),
        ));

        h_flex()
            .min_h(px(64.))
            .px_4()
            .py_2()
            .gap_3()
            .border_b_1()
            .border_color(rgb(BORDER))
            .hover(|style| style.bg(rgb(0x11151b)))
            .child(
                v_flex()
                    .flex_1()
                    .min_w_0()
                    .gap_1()
                    .child(
                        h_flex()
                            .gap_2()
                            .min_w_0()
                            .child(
                                div()
                                    .text_size(px(13.5))
                                    .font_weight(FontWeight::MEDIUM)
                                    .truncate()
                                    .child(subscription.name.clone()),
                            )
                            .child(tag(kind, MUTED)),
                    )
                    .child(
                        div()
                            .text_size(px(11.5))
                            .text_color(if subscription.last_error.is_some() {
                                rgb(DANGER)
                            } else {
                                rgb(MUTED)
                            })
                            .truncate()
                            .child(detail),
                    ),
            )
            .child(
                div()
                    .w(px(66.))
                    .text_size(px(12.5))
                    .text_color(rgb(MUTED))
                    .child(subscription.node_count.to_string()),
            )
            .child(
                v_flex()
                    .w(px(160.))
                    .gap_1()
                    .child(
                        div()
                            .text_size(px(12.5))
                            .text_color(rgb(TEXT))
                            .truncate()
                            .child(traffic),
                    )
                    .child(
                        div()
                            .text_size(px(11.))
                            .text_color(rgb(MUTED))
                            .truncate()
                            .child(expire),
                    ),
            )
            .child(
                div()
                    .w(px(100.))
                    .text_size(px(11.5))
                    .text_color(rgb(MUTED))
                    .child(format_timestamp(subscription.last_ok_at)),
            )
            .child(
                div().w(px(58.)).child(
                    Switch::new(format!("sub-switch-{id}"))
                        .small()
                        .checked(enabled)
                        .on_change(cx.listener(move |this, checked, _, cx| {
                            this.toggle_subscription(id, *checked, cx);
                        })),
                ),
            )
            .child(div().w(px(152.)).child(actions))
            .into_any_element()
    }

    fn add_dialog(&self, cx: &mut Context<Self>) -> AnyElement {
        let node_mode = self.add_mode == AddMode::Node;
        let title = if node_mode {
            "添加单个节点"
        } else {
            "添加订阅"
        };
        let name_label = if node_mode {
            "节点名称（可选）"
        } else {
            "订阅名称（可选）"
        };
        let input_label = if node_mode {
            "节点链接"
        } else {
            "订阅地址"
        };
        let confirm_label = if node_mode {
            "添加节点"
        } else {
            "添加订阅"
        };

        let mut buttons = h_flex().gap_2().justify_end();
        buttons = buttons
            .child(
                Button::new("add-cancel")
                    .ghost()
                    .label("取消")
                    .on_click(cx.listener(|this, _, window, cx| this.close_add_dialog(window, cx))),
            )
            .child(
                Button::new("add-confirm")
                    .primary()
                    .label(confirm_label)
                    .on_click(cx.listener(|this, _, window, cx| this.submit_add(window, cx))),
            );

        div()
            .absolute()
            .top_0()
            .left_0()
            .size_full()
            .occlude()
            .bg(rgba(0x05070bcc))
            .flex()
            .items_center()
            .justify_center()
            .child(
                v_flex()
                    .w(px(540.))
                    .p(px(22.))
                    .gap_3()
                    .rounded(px(14.))
                    .bg(rgb(SURFACE))
                    .border_1()
                    .border_color(rgb(BORDER))
                    .child(
                        div()
                            .text_size(px(17.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(title),
                    )
                    .child(
                        h_flex()
                            .gap_2()
                            .child(self.add_mode_pill("订阅链接", AddMode::Subscription, cx))
                            .child(self.add_mode_pill("单个节点", AddMode::Node, cx)),
                    )
                    .when(!node_mode, |this| {
                        this.child(field_label(name_label))
                            .child(Input::new(&self.add_name).cleanable(true))
                    })
                    .child(field_label(input_label))
                    .child(Input::new(&self.add_url).cleanable(true))
                    .child(
                        h_flex()
                            .gap_2()
                            .child(text_action(
                                "import-clipboard",
                                "从剪贴板导入",
                                ACCENT,
                                cx,
                                |this, window, cx| this.import_from_clipboard(window, cx),
                            ))
                            .child(text_action(
                                "import-file",
                                "导入 YAML 文件",
                                MUTED,
                                cx,
                                |this, window, cx| this.import_file(window, cx),
                            )),
                    )
                    .when_some(self.add_error.clone(), |this, error| {
                        this.child(
                            div()
                                .text_size(px(12.))
                                .text_color(rgb(DANGER))
                                .child(error),
                        )
                    })
                    .child(buttons),
            )
            .into_any_element()
    }

    fn node_rename_dialog(&self, cx: &mut Context<Self>) -> AnyElement {
        let panel =
            v_flex()
                .w(px(640.))
                .p(px(22.))
                .gap_3()
                .rounded(px(14.))
                .bg(rgb(SURFACE))
                .border_1()
                .border_color(rgb(BORDER))
                .child(
                    div()
                        .text_size(px(17.))
                        .font_weight(FontWeight::SEMIBOLD)
                        .child("编辑节点"),
                )
                .child(field_label("名称"))
                .child(Input::new(&self.node_rename_input).cleanable(true))
                .child(field_label("节点 YAML"))
                .child(Textarea::new(&self.node_edit_textarea).h(px(300.)))
                .child(
                    div()
                        .text_size(px(12.))
                        .text_color(rgb(MUTED))
                        .child("可直接修改协议、服务器、端口、密码等全部字段，保存后立即生效。"),
                )
                .when_some(self.node_rename_error.clone(), |this, error| {
                    this.child(
                        div()
                            .text_size(px(12.))
                            .text_color(rgb(DANGER))
                            .child(error),
                    )
                })
                .child(
                    h_flex()
                        .gap_2()
                        .justify_end()
                        .child(
                            Button::new("node-rename-cancel")
                                .ghost()
                                .label("取消")
                                .on_click(cx.listener(|this, _, window, cx| {
                                    this.close_node_rename(window, cx)
                                })),
                        )
                        .child(
                            Button::new("node-rename-save")
                                .primary()
                                .label("保存")
                                .on_click(cx.listener(|this, _, window, cx| {
                                    this.save_node_rename(window, cx)
                                })),
                        ),
                );

        modal(panel).into_any_element()
    }

    fn template_tab_pill(
        &self,
        label: &'static str,
        tab: TemplateTab,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let active = self.template_tab == tab;
        div()
            .id(format!("template-tab-{label}"))
            .cursor_pointer()
            .h(px(28.))
            .px_3()
            .rounded(px(7.))
            .border_1()
            .border_color(rgb(if active { ACCENT } else { BORDER }))
            .bg(rgb(if active { ACCENT_SOFT } else { SURFACE_2 }))
            .text_size(px(12.5))
            .text_color(rgb(if active { TEXT } else { MUTED }))
            .hover(|style| style.text_color(rgb(TEXT)))
            .flex()
            .items_center()
            .child(label)
            .on_click(cx.listener(move |this, _, _, cx| this.set_template_tab(tab, cx)))
            .into_any_element()
    }

    fn template_rule_source_header(
        &self,
        source_id: &str,
        name: &str,
        selected: usize,
        total: usize,
        entry_ids: Vec<String>,
        searching: bool,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let id = source_id.to_string();
        let select_id = id.clone();
        let clear_id = id.clone();
        let clear_entries = entry_ids.clone();
        let open = searching || !self.template_rule_sources_collapsed.contains(source_id);
        h_flex()
            .gap_2()
            .pt(px(10.))
            .pb(px(4.))
            .child(
                h_flex()
                    .id(format!("rule-source-{source_id}"))
                    .flex_1()
                    .min_w_0()
                    .gap_2()
                    .cursor_pointer()
                    .text_size(px(12.))
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(rgb(TEXT))
                    .child(if open { "▾" } else { "▸" })
                    .child(div().truncate().child(name.to_string()))
                    .child(
                        div()
                            .text_size(px(11.))
                            .text_color(rgb(MUTED))
                            .child(format!("已选 {selected}/{total}")),
                    )
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.toggle_template_rule_source(id.clone(), cx);
                    })),
            )
            .child(
                div()
                    .id(format!("rule-source-select-{source_id}"))
                    .cursor_pointer()
                    .text_size(px(11.))
                    .text_color(rgb(ACCENT))
                    .child(if searching { "全选匹配" } else { "全选" })
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.set_template_source_rules(
                            select_id.clone(),
                            entry_ids.clone(),
                            true,
                            cx,
                        );
                    })),
            )
            .child(
                div()
                    .id(format!("rule-source-clear-{source_id}"))
                    .cursor_pointer()
                    .text_size(px(11.))
                    .text_color(rgb(MUTED))
                    .child(if searching { "清空匹配" } else { "清空" })
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.set_template_source_rules(
                            clear_id.clone(),
                            clear_entries.clone(),
                            false,
                            cx,
                        );
                    })),
            )
            .into_any_element()
    }

    fn template_rule_target_button(
        &self,
        id: &str,
        target: &str,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let id = id.to_string();
        div()
            .id(format!("rule-target-{id}"))
            .cursor_pointer()
            .px_2()
            .py_1()
            .rounded(px(5.))
            .bg(rgb(ACCENT_SOFT))
            .text_size(px(11.))
            .text_color(rgb(ACCENT))
            .child(format!("{target} ▾"))
            .on_click(
                cx.listener(move |this, _, _, cx| this.open_template_target_picker(id.clone(), cx)),
            )
            .into_any_element()
    }

    fn template_target_picker_panel(&self, id: &str, cx: &mut Context<Self>) -> AnyElement {
        let mut rows = v_flex().gap_1();
        let filtering = id == "__target_filter__";
        if filtering {
            rows = rows.child(
                Button::new("filter-all-targets")
                    .ghost()
                    .label("全部目标")
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.set_template_rule_target("__target_filter__".into(), String::new(), cx)
                    })),
            );
        }

        for target in self.template_target_options() {
            let rule_id = id.to_string();
            let module = catalog::MODULES.iter().find(|module| module.name == target);
            let inactive = module.is_some_and(|module| {
                !self.template_enabled_modules.contains(module.id)
                    && !matches!(module.id, "select" | "auto" | "final")
            });
            let selected_target = target.clone();
            rows = rows.child(
                h_flex()
                    .id(format!("target-option-{target}"))
                    .h(px(32.))
                    .px_2()
                    .rounded(px(6.))
                    .cursor_pointer()
                    .hover(|style| style.bg(rgb(SURFACE_2)))
                    .child(div().flex_1().text_size(px(12.5)).child(target))
                    .when(inactive && !filtering, |row| {
                        row.child(tag("选择后启用", MUTED))
                    })
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.set_template_rule_target(rule_id.clone(), selected_target.clone(), cx);
                    })),
            );
        }
        v_flex()
            .flex_1()
            .min_h_0()
            .gap_2()
            .child(
                h_flex()
                    .justify_between()
                    .child(field_label(if filtering {
                        "按策略组筛选规则集"
                    } else {
                        "选择此规则集的策略组 / 策略"
                    }))
                    .child(
                        Button::new("target-picker-back")
                            .ghost()
                            .label("返回规则集")
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.template_target_picker = None;
                                cx.notify();
                            })),
                    ),
            )
            .child(
                div()
                    .text_size(px(11.5))
                    .text_color(rgb(MUTED))
                    .child(if filtering {
                        "只筛选显示的规则集，不改变任何规则目标或启用状态。"
                    } else {
                        "可以绑定内置组或自定义策略组；选择未启用的内置组会同时启用该组。"
                    }),
            )
            .child(
                div()
                    .id("template-target-picker-list")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scrollbar()
                    .rounded(px(8.))
                    .border_1()
                    .border_color(rgb(BORDER))
                    .p_2()
                    .child(rows),
            )
            .into_any_element()
    }

    fn template_policy_groups_panel(&self, cx: &mut Context<Self>) -> AnyElement {
        let mut list = v_flex().gap_1();
        for (index, group) in self.template_policy_groups.iter().enumerate() {
            list = list.child(
                Button::new(format!("edit-policy-{index}"))
                    .ghost()
                    .label(format!(
                        "{} {}",
                        if self.template_policy_edit == Some(index) {
                            "▸"
                        } else {
                            ""
                        },
                        group.name
                    ))
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.edit_template_policy_group(index, window, cx)
                    })),
            );
        }
        list = list.child(field_label("内置组"));
        for module in catalog::MODULES
            .iter()
            .filter(|m| !matches!(m.kind, ModuleKind::Region(_)))
        {
            let id = module.id.to_string();
            let name = module.name.to_string();
            let enabled = self.template_enabled_modules.contains(module.id)
                || matches!(module.id, "select" | "auto" | "final");
            list = list.child(
                h_flex()
                    .gap_1()
                    .child(
                        Switch::new(format!("module-{id}"))
                            .small()
                            .checked(enabled)
                            .on_change(cx.listener(move |this, checked, _, cx| {
                                this.toggle_template_module(id.clone(), *checked, cx)
                            })),
                    )
                    .child(
                        Button::new(format!("builtin-{}", module.id))
                            .ghost()
                            .label(name.clone())
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.edit_builtin_policy_group(name.clone(), window, cx)
                            })),
                    ),
            );
        }
        let mut detail = v_flex().gap_2().min_w_0();
        if let Some(index) = self.template_policy_edit {
            if let Some(group) = self.template_policy_groups.get(index) {
                detail = detail
                    .child(field_label(if group.builtin {
                        "内置组覆盖（删除覆盖可恢复默认）"
                    } else {
                        "策略组详情"
                    }))
                    .child(Input::new(&self.template_policy_rename));
                let mut kinds = h_flex().flex_wrap().gap_1();
                for (kind, label) in [
                    (GroupKind::Select, "手动"),
                    (GroupKind::UrlTest, "测速"),
                    (GroupKind::Fallback, "故障转移"),
                    (GroupKind::LoadBalance, "负载均衡"),
                ] {
                    kinds = kinds.child(
                        Button::new(format!("kind-{}", kind.as_str()))
                            .ghost()
                            .label(format!(
                                "{}{}",
                                if group.kind == kind { "✓ " } else { "" },
                                label
                            ))
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.set_template_policy_kind(index, kind, cx)
                            })),
                    );
                }
                detail = detail
                    .child(kinds)
                    .child(Input::new(&self.template_health_url))
                    .child(Input::new(&self.template_health_interval))
                    .child(
                        h_flex()
                            .flex_wrap()
                            .gap_1()
                            .child(
                                Button::new("save-policy-details")
                                    .primary()
                                    .label("更新名称 / 测速参数")
                                    .on_click(cx.listener(move |this, _, window, cx| {
                                        this.save_policy_details(index, window, cx)
                                    })),
                            )
                            .child(
                                Button::new("remove-policy")
                                    .ghost()
                                    .label(if group.builtin {
                                        "恢复默认"
                                    } else {
                                        "删除组"
                                    })
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        this.remove_template_policy_group(index, cx)
                                    })),
                            ),
                    )
                    .child(field_label("已选成员（拖动或 ↑↓ 调整优先级）"));
                for (member_index, name) in group.members.iter().enumerate() {
                    let drag_name = name.clone();
                    let mut row = h_flex().gap_1().child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .text_size(px(11.))
                            .truncate()
                            .child(name.clone()),
                    );
                    if member_index > 0 {
                        row = row.child(
                            Button::new(format!("member-up-{member_index}"))
                                .ghost()
                                .label("↑")
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.move_policy_member(
                                        index,
                                        member_index,
                                        member_index - 1,
                                        cx,
                                    )
                                })),
                        );
                    }
                    if member_index + 1 < group.members.len() {
                        row = row.child(
                            Button::new(format!("member-down-{member_index}"))
                                .ghost()
                                .label("↓")
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.move_policy_member(
                                        index,
                                        member_index,
                                        member_index + 1,
                                        cx,
                                    )
                                })),
                        );
                    }
                    let name = name.clone();
                    row = row.child(
                        Button::new(format!("member-remove-{member_index}"))
                            .ghost()
                            .label("×")
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.toggle_template_policy_member(index, name.clone(), cx)
                            })),
                    );
                    detail = detail.child(
                        div()
                            .id(format!("member-drag-{member_index}"))
                            .cursor_pointer()
                            .child(row)
                            .on_drag(
                                MemberDrag {
                                    group: index,
                                    member: member_index,
                                    name: drag_name,
                                },
                                |drag, _, _, cx| {
                                    cx.new(|_| NodeDragPreview {
                                        name: drag.name.clone(),
                                    })
                                },
                            )
                            .on_drop(cx.listener(move |this, drag: &MemberDrag, _, cx| {
                                if drag.group == index {
                                    this.move_policy_member(index, drag.member, member_index, cx)
                                }
                            })),
                    );
                }
                detail = detail
                    .child(
                        Button::new("all-nodes")
                            .ghost()
                            .label(if group.include_all_nodes {
                                "✓ 动态包含全部节点"
                            } else {
                                "+ 动态包含全部节点"
                            })
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.toggle_template_policy_member(index, "__all_nodes".into(), cx)
                            })),
                    )
                    .child(Input::new(&self.template_member_search).cleanable(true));
                let query = self.template_member_search.read(cx).value().to_lowercase();
                let mut candidates = self.template_target_options();
                candidates.extend(
                    self.nodes
                        .iter()
                        .filter(|n| n.enabled)
                        .map(|n| n.name().to_string()),
                );
                let mut seen = std::collections::HashSet::new();
                candidates.retain(|n| {
                    n != &group.name
                        && !group.members.contains(n)
                        && n.to_lowercase().contains(query.as_str())
                        && seen.insert(n.clone())
                });
                let selected_candidates = candidates.clone();
                detail = detail.child(
                    Button::new("bulk-members")
                        .ghost()
                        .label(format!("加入搜索结果（{}）", candidates.len()))
                        .on_click(cx.listener(move |this, _, _, cx| {
                            for name in &selected_candidates {
                                this.toggle_template_policy_member(index, name.clone(), cx)
                            }
                        })),
                );
                let mut choices = h_flex().flex_wrap().gap_1();
                for (i, name) in candidates.into_iter().take(200).enumerate() {
                    choices = choices.child(
                        Button::new(format!("candidate-{i}"))
                            .ghost()
                            .label(format!("+ {name}"))
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.toggle_template_policy_member(index, name.clone(), cx)
                            })),
                    );
                }
                detail = detail.child(choices).child(
                    div().text_size(px(10.)).text_color(rgb(MUTED)).child(
                        "候选显示前 200 项；搜索或批量加入可处理全部结果。空测速参数继承全局。",
                    ),
                );
            }
        } else {
            detail = detail.child(field_label("选择策略组编辑成员、名称和测速参数"));
        }
        v_flex()
            .flex_1()
            .min_h_0()
            .gap_2()
            .child(
                h_flex()
                    .gap_1()
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .child(Input::new(&self.template_policy_name).cleanable(true)),
                    )
                    .child(
                        Button::new("add-policy")
                            .primary()
                            .label("新建组")
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.add_template_policy_group(window, cx)
                            })),
                    ),
            )
            .child(
                h_resizable("policy-editor")
                    .child(
                        resizable_panel()
                            .size(px(185.))
                            .size_range(px(140.)..px(350.))
                            .child(
                                div()
                                    .id("policy-list")
                                    .size_full()
                                    .overflow_y_scrollbar()
                                    .child(list),
                            ),
                    )
                    .child(
                        resizable_panel().child(
                            div()
                                .id("policy-detail")
                                .size_full()
                                .min_w_0()
                                .overflow_y_scrollbar()
                                .child(detail),
                        ),
                    ),
            )
            .into_any_element()
    }

    fn template_rule_order_panel(&self, cx: &mut Context<Self>) -> AnyElement {
        let profile = match self.draft_generated(cx) {
            Ok(p) => p,
            Err(e) => return div().text_color(rgb(DANGER)).child(e).into_any_element(),
        };
        let mut rows = v_flex().gap_1();
        for warning in profile.warnings {
            rows = rows.child(
                div()
                    .text_size(px(11.))
                    .text_color(rgb(WARNING))
                    .child(warning),
            );
        }
        for (index, rule) in profile.rules.iter().enumerate() {
            let name = rule.to_line();
            let total = profile.rules.len();
            let mut row = h_flex().gap_1().child(
                div()
                    .flex_1()
                    .min_w_0()
                    .text_size(px(11.))
                    .child(format!("{}. {name}", index + 1)),
            );
            if index > 0 {
                row = row.child(
                    Button::new(format!("rule-up-{index}"))
                        .ghost()
                        .label("↑")
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.move_template_rule(index, index - 1, cx)
                        })),
                );
            }
            if index + 1 < total {
                row = row.child(
                    Button::new(format!("rule-down-{index}"))
                        .ghost()
                        .label("↓")
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.move_template_rule(index, index + 1, cx)
                        })),
                );
            }
            rows = rows.child(
                div()
                    .id(format!("rule-drag-{index}"))
                    .p_1()
                    .border_b_1()
                    .border_color(rgb(BORDER))
                    .cursor_pointer()
                    .child(row)
                    .on_drag(RuleDrag { index, name }, |drag, _, _, cx| {
                        cx.new(|_| NodeDragPreview {
                            name: drag.name.clone(),
                        })
                    })
                    .on_drop(cx.listener(move |this, drag: &RuleDrag, _, cx| {
                        this.move_template_rule(drag.index, index, cx)
                    })),
            );
        }
        v_flex()
            .flex_1()
            .min_h_0()
            .gap_2()
            .child(field_label("最终规则顺序 · 从上到下首次命中生效"))
            .child(
                Button::new("reset-rule-order")
                    .ghost()
                    .label("恢复默认顺序")
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.template_rule_order.clear();
                        cx.notify();
                    })),
            )
            .child(
                div()
                    .id("rule-order-scroll")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scrollbar()
                    .child(rows),
            )
            .into_any_element()
    }

    fn template_dialog(&self, cx: &mut Context<Self>) -> AnyElement {
        let query = self
            .template_rule_search
            .read(cx)
            .value()
            .trim()
            .to_lowercase();
        let mut rule_rows = v_flex().gap_1();
        let mut selected_count = 0usize;
        let mut total_count = 0usize;
        let mut visible_count = 0usize;

        let is_catalog_binding = |id: &str| {
            rule_catalog::SOURCES.iter().any(|source| {
                source
                    .entries
                    .iter()
                    .any(|entry| format!("{}-{}", source.id, entry.id) == id)
            })
        };
        let mut url_rows = Vec::new();
        for binding in self
            .template_custom_rule_sets
            .iter()
            .filter(|binding| !is_catalog_binding(&binding.id))
        {
            total_count += 1;
            selected_count += 1;
            let haystack =
                format!("{} {} {}", binding.name, binding.url, binding.target).to_lowercase();
            if (query.is_empty() || haystack.contains(&query))
                && (self.template_rules_target_filter.is_empty()
                    || self.template_rules_target_filter == binding.target)
            {
                visible_count += 1;
                url_rows.push(binding);
            }
        }
        if !url_rows.is_empty() {
            rule_rows = rule_rows.child(
                h_flex()
                    .justify_between()
                    .pt(px(8.))
                    .pb(px(2.))
                    .child(
                        div()
                            .text_size(px(12.))
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(rgb(TEXT))
                            .child("自定义链接"),
                    )
                    .child(
                        div()
                            .text_size(px(11.))
                            .text_color(rgb(MUTED))
                            .child(format!("{} 条", url_rows.len())),
                    ),
            );
            for binding in url_rows {
                let id = binding.id.clone();
                rule_rows = rule_rows.child(
                    h_flex()
                        .h(px(42.))
                        .px_2()
                        .gap_2()
                        .rounded(px(7.))
                        .hover(|style| style.bg(rgb(SURFACE_2)))
                        .child(
                            v_flex()
                                .flex_1()
                                .min_w_0()
                                .gap_1()
                                .child(
                                    div()
                                        .text_size(px(12.5))
                                        .truncate()
                                        .child(binding.name.clone()),
                                )
                                .child(
                                    div()
                                        .text_size(px(10.5))
                                        .text_color(rgb(MUTED))
                                        .truncate()
                                        .child(binding.url.clone()),
                                ),
                        )
                        .child(self.template_rule_target_button(&binding.id, &binding.target, cx))
                        .child(icon_action(
                            format!("remove-rule-url-{}", binding.id),
                            "×",
                            DANGER,
                            cx,
                            move |this, _, cx| this.remove_template_custom_binding(id.clone(), cx),
                        )),
                );
            }
        }

        let builtin_ids: Vec<String> = catalog::MODULES
            .iter()
            .filter(|module| {
                self.template_enabled_modules.contains(module.id)
                    || matches!(module.id, "select" | "auto" | "final")
            })
            .flat_map(|module| module.rule_sets.iter().map(move |spec| (module, spec)))
            .filter(|(module, spec)| {
                let enabled = !self.template_disabled_rule_sets.contains(spec.id);
                let target = self
                    .template_rule_set_targets
                    .get(spec.id)
                    .map(String::as_str)
                    .unwrap_or(module.name);
                (query.is_empty()
                    || format!(
                        "内置 MetaCubeX {} {} {} {}",
                        module.name,
                        rule_set_label(spec.id),
                        spec.id,
                        target
                    )
                    .to_lowercase()
                    .contains(&query))
                    && (!self.template_rules_selected_only || enabled)
                    && (self.template_rules_target_filter.is_empty()
                        || self.template_rules_target_filter == target)
            })
            .map(|(_, spec)| spec.id.to_string())
            .collect();
        let builtin_selected = catalog::MODULES
            .iter()
            .filter(|module| {
                self.template_enabled_modules.contains(module.id)
                    || matches!(module.id, "select" | "auto" | "final")
            })
            .flat_map(|module| module.rule_sets)
            .filter(|spec| !self.template_disabled_rule_sets.contains(spec.id))
            .count();
        let builtin_total = catalog::MODULES
            .iter()
            .filter(|module| {
                self.template_enabled_modules.contains(module.id)
                    || matches!(module.id, "select" | "auto" | "final")
            })
            .map(|module| module.rule_sets.len())
            .sum();
        if !builtin_ids.is_empty() {
            rule_rows = rule_rows.child(self.template_rule_source_header(
                "builtin",
                "内置规则（MetaCubeX）",
                builtin_selected,
                builtin_total,
                builtin_ids,
                !query.is_empty()
                    || self.template_rules_selected_only
                    || !self.template_rules_target_filter.is_empty(),
                cx,
            ));
        }
        let filtered =
            self.template_rules_selected_only || !self.template_rules_target_filter.is_empty();
        let builtin_open = filtered
            || !query.is_empty()
            || !self.template_rule_sources_collapsed.contains("builtin");

        for module in catalog::MODULES.iter().filter(|module| {
            !module.rule_sets.is_empty()
                && (self.template_enabled_modules.contains(module.id)
                    || matches!(module.id, "select" | "auto" | "final"))
        }) {
            let mut visible = Vec::new();
            for spec in module.rule_sets {
                total_count += 1;
                let enabled = !self.template_disabled_rule_sets.contains(spec.id);
                if enabled {
                    selected_count += 1;
                }
                let haystack = format!(
                    "内置 MetaCubeX {} {} {}",
                    module.name,
                    rule_set_label(spec.id),
                    spec.id
                )
                .to_lowercase();
                let target = self
                    .template_rule_set_targets
                    .get(spec.id)
                    .map(String::as_str)
                    .unwrap_or(module.name);
                if (query.is_empty()
                    || haystack.contains(&query)
                    || target.to_lowercase().contains(&query))
                    && (!self.template_rules_selected_only || enabled)
                    && (self.template_rules_target_filter.is_empty()
                        || self.template_rules_target_filter == target)
                {
                    visible_count += 1;
                    visible.push((*spec, enabled));
                }
            }
            if visible.is_empty() || !builtin_open {
                continue;
            }

            rule_rows = rule_rows.child(
                h_flex()
                    .justify_between()
                    .pt(px(8.))
                    .pb(px(2.))
                    .child(
                        div()
                            .text_size(px(12.))
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(rgb(TEXT))
                            .child(module.name),
                    )
                    .child(
                        div()
                            .text_size(px(11.))
                            .text_color(rgb(MUTED))
                            .child(format!("内置 · {} 条", visible.len())),
                    ),
            );

            for (spec, enabled) in visible {
                let id = spec.id.to_string();
                rule_rows = rule_rows.child(
                    h_flex()
                        .h(px(34.))
                        .px_2()
                        .gap_2()
                        .rounded(px(7.))
                        .hover(|style| style.bg(rgb(SURFACE_2)))
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .truncate()
                                .text_size(px(12.5))
                                .child(rule_set_label(spec.id)),
                        )
                        .child(
                            div()
                                .w(px(140.))
                                .truncate()
                                .text_size(px(11.))
                                .text_color(rgb(MUTED))
                                .child(spec.id.to_string()),
                        )
                        .child(
                            self.template_rule_target_button(
                                spec.id,
                                self.template_rule_set_targets
                                    .get(spec.id)
                                    .map(String::as_str)
                                    .unwrap_or(module.name),
                                cx,
                            ),
                        )
                        .child(tag(
                            if spec.behavior == catalog::RuleSetBehavior::Domain {
                                "域名"
                            } else {
                                "IP"
                            },
                            MUTED,
                        ))
                        .child(
                            Switch::new(format!("template-rule-set-{}", spec.id))
                                .small()
                                .checked(enabled)
                                .on_change(cx.listener(move |this, checked, _, cx| {
                                    this.toggle_template_rule_set(id.clone(), *checked, cx)
                                })),
                        ),
                );
            }
        }

        for source in rule_catalog::SOURCES {
            let mut entries = Vec::new();
            let mut source_selected = 0usize;
            for entry in source.entries {
                total_count += 1;
                let binding_id = format!("{}-{}", source.id, entry.id);
                let selected = self
                    .template_custom_rule_sets
                    .iter()
                    .any(|binding| binding.id == binding_id);
                if selected {
                    selected_count += 1;
                    source_selected += 1;
                }
                let target = self
                    .template_custom_rule_sets
                    .iter()
                    .find(|binding| binding.id == binding_id)
                    .map(|binding| binding.target.clone())
                    .unwrap_or_else(|| {
                        rule_catalog::resolve_target(entry.target, &self.template_enabled_modules)
                    });
                let haystack = format!(
                    "{} {} {} {} {}",
                    source.name, entry.name, entry.id, entry.path, target
                )
                .to_lowercase();
                if (query.is_empty() || haystack.contains(&query))
                    && (!self.template_rules_selected_only || selected)
                    && (self.template_rules_target_filter.is_empty()
                        || self.template_rules_target_filter == target)
                {
                    visible_count += 1;
                    entries.push((entry, selected, target, binding_id));
                }
            }
            if entries.is_empty() {
                continue;
            }

            rule_rows = rule_rows.child(
                self.template_rule_source_header(
                    source.id,
                    source.name,
                    source_selected,
                    source.entries.len(),
                    entries
                        .iter()
                        .map(|(entry, _, _, _)| entry.id.to_string())
                        .collect(),
                    !query.is_empty() || filtered,
                    cx,
                ),
            );
            if query.is_empty()
                && !filtered
                && self.template_rule_sources_collapsed.contains(source.id)
            {
                continue;
            }
            rule_rows = rule_rows.child(
                div()
                    .pb(px(2.))
                    .text_size(px(10.5))
                    .text_color(rgb(MUTED))
                    .child(source.description),
            );

            for (entry, selected, target, binding_id) in entries {
                let source_id = source.id.to_string();
                let entry_id = entry.id.to_string();
                rule_rows = rule_rows.child(
                    h_flex()
                        .h(px(34.))
                        .px_2()
                        .gap_2()
                        .rounded(px(7.))
                        .hover(|style| style.bg(rgb(SURFACE_2)))
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .truncate()
                                .text_size(px(12.5))
                                .child(entry.name),
                        )
                        .child(self.template_rule_target_button(&binding_id, &target, cx))
                        .child(
                            div()
                                .w(px(38.))
                                .text_size(px(10.5))
                                .text_color(rgb(MUTED))
                                .child(entry.format.to_uppercase()),
                        )
                        .child(
                            Switch::new(format!("template-source-{binding_id}"))
                                .small()
                                .checked(selected)
                                .on_change(cx.listener(move |this, checked, _, cx| {
                                    this.toggle_template_source_rule_set(
                                        source_id.clone(),
                                        entry_id.clone(),
                                        *checked,
                                        cx,
                                    )
                                })),
                        ),
                );
            }
        }

        if visible_count == 0 {
            rule_rows = rule_rows.child(
                div()
                    .p_4()
                    .text_size(px(12.5))
                    .text_color(rgb(MUTED))
                    .child(if total_count == 0 {
                        "当前没有可配置的规则集，请先在配置生成页启用服务分组。"
                    } else {
                        "没有匹配的规则集"
                    }),
            );
        }

        let source = rule_provider_source_label(&self.options.rule_provider_base_url);
        let custom_rule_count = self
            .template_custom_rules
            .read(cx)
            .value()
            .lines()
            .filter(|line| {
                let line = line.trim();
                !line.is_empty() && !line.starts_with('#')
            })
            .count();
        let selected_sources = rule_catalog::SOURCES
            .iter()
            .filter(|source| {
                (source.id == "mcx" && builtin_selected > 0)
                    || source.entries.iter().any(|entry| {
                        self.template_custom_rule_sets
                            .iter()
                            .any(|binding| binding.id == format!("{}-{}", source.id, entry.id))
                    })
            })
            .count();
        let header_count = match self.template_tab {
            TemplateTab::PolicyGroups => format!(
                "{} 个启用模块 · {} 个组配置",
                self.template_enabled_modules.len(),
                self.template_policy_groups.len()
            ),
            TemplateTab::RuleSets => {
                format!("已选 {selected_count} / {total_count} 条 · {selected_sources} 个来源")
            }
            TemplateTab::Advanced => "DNS / 多跳 / 独立监听 / 扩展 YAML".into(),
            TemplateTab::Changes => "应用前配置差异".into(),
            TemplateTab::RuleOrder => "最终规则排序与检查".into(),
            TemplateTab::CustomRules => format!("{custom_rule_count} 条自定义规则"),
            TemplateTab::Tools => "筛选代理组与链式代理".into(),
        };

        let tab_content = if let Some(id) = &self.template_target_picker {
            self.template_target_picker_panel(id, cx)
        } else {
            match self.template_tab {
            TemplateTab::Advanced => v_flex().flex_1().min_h_0().gap_2()
                .child(div().text_size(px(11.)).text_color(rgb(MUTED)).child("编辑 DNS、监听或链路 JSON 后，先校验配置，自动保存。链路按入口到落地填写 2–16 个节点。"))
                .child(div().flex_1().min_h_0().child(Textarea::new(&self.advanced_draft).h_full()))
                .child(h_flex().gap_2().child(Button::new("advanced-validate").primary().label("校验配置").on_click(cx.listener(|this,_,_,cx|this.update_advanced_draft(cx))))
                    .child(Button::new("advanced-reset").ghost().label("重载已保存配置").on_click(cx.listener(|this,_,window,cx|this.reset_advanced_draft(window,cx)))))
                .into_any_element(),
            TemplateTab::Changes => {
                let mut rows=v_flex().gap_2();
                match self.draft_generated(cx) {
                    Ok(profile)=> {
                        for warning in profile.warnings { rows=rows.child(div().text_color(rgb(WARNING)).text_size(px(11.)).child(warning)); }
                        let old:std::collections::HashSet<_>=self.preview.lines().collect();
                        let new:std::collections::HashSet<_>=profile.yaml.lines().collect();
                        rows=rows.child(field_label("移除的配置行（最多 100 行）"));
                        for line in self.preview.lines().filter(|line|!new.contains(line)).take(100) {rows=rows.child(div().font_family("Consolas").text_size(px(11.)).text_color(rgb(DANGER)).child(format!("− {line}")));}
                        rows=rows.child(field_label("新增的配置行（最多 100 行）"));
                        for line in profile.yaml.lines().filter(|line|!old.contains(line)).take(100) {rows=rows.child(div().font_family("Consolas").text_size(px(11.)).text_color(rgb(SUCCESS)).child(format!("+ {line}")));}
                        rows=rows.child(div().text_size(px(11.)).text_color(rgb(MUTED)).child("此处按配置行比较；顺序变更请在最终规则顺序页检查。"));
                    }, Err(error)=>{rows=rows.child(div().text_color(rgb(DANGER)).child(error));}
                }
                div().id("template-changes").flex_1().min_h_0().overflow_y_scrollbar().child(rows).into_any_element()
            },
            TemplateTab::RuleOrder => self.template_rule_order_panel(cx),
            TemplateTab::Tools => div().id("workspace-tools").flex_1().min_h_0().overflow_y_scrollbar().child(v_flex().gap_4().child(self.custom_groups_section(cx)).child(self.chains_section(cx))).into_any_element(),
            TemplateTab::PolicyGroups => self.template_policy_groups_panel(cx),
            TemplateTab::RuleSets => v_flex()
                .flex_1().min_h_0().gap_2()
                .child(h_flex().gap_2().flex_wrap().flex_shrink_0()
                    .child(div().flex_1().min_w(px(160.)).child(Input::new(&self.template_rule_search).cleanable(true)))
                    .child(Button::new("rules-selected-only").ghost().small()
                        .label(if self.template_rules_selected_only {"仅已选 ✓"} else {"仅已选"})
                        .on_click(cx.listener(|this,_,_,cx| {this.template_rules_selected_only = !this.template_rules_selected_only; cx.notify();})))
                    .child(Button::new("rules-target-filter").ghost().small()
                        .label(if self.template_rules_target_filter.is_empty() {"全部目标 ▾".to_string()} else {format!("{} ▾", self.template_rules_target_filter)})
                        .on_click(cx.listener(|this,_,_,cx| this.open_template_target_picker("__target_filter__".into(),cx))))
                    .child(Button::new("rules-link-toggle").ghost().small()
                        .label(if self.template_rule_link_open {"收起链接"} else {"添加链接"})
                        .on_click(cx.listener(|this,_,_,cx| {this.template_rule_link_open = !this.template_rule_link_open; cx.notify();}))))
                .when(!query.is_empty() || self.template_rules_selected_only || !self.template_rules_target_filter.is_empty(), |column| {
                    column.child(h_flex().justify_between().flex_shrink_0()
                        .child(div().text_size(px(11.)).text_color(rgb(MUTED)).child(format!("匹配 {visible_count} 条规则集")))
                        .child(Button::new("clear-rule-filters").ghost().small().label("清除筛选")
                            .on_click(cx.listener(|this,_,window,cx| {
                                this.template_rules_selected_only = false;
                                this.template_rules_target_filter.clear();
                                this.template_rule_search.update(cx, |input,cx| input.set_value("",window,cx));
                                cx.notify();
                            }))))
                })
                .when(self.template_rule_link_open, |column| column.child(h_flex().gap_2().flex_shrink_0()
                    .child(div().flex_1().min_w_0().child(Input::new(&self.template_rule_url).cleanable(true)))
                    .child(Button::new("add-rule-set-url").primary().small().label("加入规则集")
                        .on_click(cx.listener(|this,_,window,cx| this.add_template_rule_set_url(window,cx))))))
                .child(
                    div()
                        .id("template-rule-set-list")
                        .flex_1()
                        .min_h_0()
                        .p_2()
                        .rounded(px(9.))
                        .border_1()
                        .border_color(rgb(BORDER))
                        .bg(rgb(BG))
                        .overflow_y_scrollbar()
                        .child(rule_rows),
                )
                .child(
                    div()
                        .text_size(px(11.5))
                        .text_color(rgb(MUTED))
                        .child(format!("可同时勾选多个来源 · 点击蓝色目标绑定策略组 · MetaCubeX 镜像：{source}")),
                )
                .into_any_element(),
            TemplateTab::CustomRules => v_flex()
                .flex_1()
                .min_h_0()
                .gap_2()
                .child(field_label("自定义规则（Mihomo 语法）"))
                .child(div().flex_1().min_h_0().child(Textarea::new(&self.template_custom_rules).h_full()))
                .child(
                    div()
                        .text_size(px(11.5))
                        .text_color(rgb(MUTED))
                        .child("每行一条规则，例如 DOMAIN-SUFFIX,example.com,节点选择；IP-CIDR,1.1.1.0/24,DIRECT,no-resolve；以 # 开头为注释。"),
                )
                .child(
                    div()
                        .text_size(px(11.5))
                        .text_color(rgb(MUTED))
                        .child("自定义规则会插入到内置分流规则之间，并随模板保存。"),
                )
                .into_any_element(),
        }
        };

        let embedded = self.page == Page::Generator;
        let panel = v_flex()
            .when(embedded, |this| this.size_full().min_w_0().min_h_0())
            .when(!embedded, |this| this.w(px(760.)).h(px(600.)))
            .p(px(14.))
            .gap_2()
            .bg(rgb(SURFACE))
            .when(!embedded, |this| {
                this.rounded(px(14.)).border_1().border_color(rgb(BORDER))
            })
            .child(
                h_flex()
                    .gap_3()
                    .flex_shrink_0()
                    .child(
                        div()
                            .text_size(px(16.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .child("分流编辑"),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .text_size(px(11.))
                            .text_color(rgb(MUTED))
                            .child(header_count),
                    ),
            )
            .child(
                h_flex()
                    .gap_1()
                    .flex_wrap()
                    .flex_shrink_0()
                    .child(self.template_tab_pill("策略组", TemplateTab::PolicyGroups, cx))
                    .child(self.template_tab_pill("规则集", TemplateTab::RuleSets, cx))
                    .child(self.template_tab_pill("自定义规则", TemplateTab::CustomRules, cx))
                    .child(self.template_tab_pill("规则顺序", TemplateTab::RuleOrder, cx))
                    .child(self.template_tab_pill("筛选与链式", TemplateTab::Tools, cx))
                    .child(self.template_tab_pill("高级配置", TemplateTab::Advanced, cx))
                    .child(self.template_tab_pill("变更预览", TemplateTab::Changes, cx)),
            )
            .child(
                h_flex()
                    .gap_2()
                    .flex_shrink_0()
                    .min_w_0()
                    .when(self.recovered_draft, |row| {
                        row.child(tag("已恢复编辑", ACCENT))
                    })
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .text_size(px(10.5))
                            .text_color(rgb(MUTED))
                            .child(if self.template_error.is_some() {
                                "编辑内容已保留，修正后自动更新输出".to_string()
                            } else {
                                "修改自动保存 · 可直接收起或切换页面".to_string()
                            }),
                    ),
            )
            .child(tab_content)
            .when_some(self.template_error.clone(), |this, error| {
                this.child(
                    div()
                        .id("template-error")
                        .max_h(px(64.))
                        .overflow_y_scrollbar()
                        .text_size(px(12.))
                        .text_color(rgb(DANGER))
                        .child(error),
                )
            })
            .child(
                h_flex()
                    .gap_2()
                    .flex_wrap()
                    .flex_shrink_0()
                    .pt_2()
                    .border_t_1()
                    .border_color(rgb(BORDER))
                    .child(
                        div()
                            .min_w(px(140.))
                            .flex_1()
                            .child(Input::new(&self.template_name).cleanable(true)),
                    )
                    .child(
                        Button::new("template-cancel")
                            .ghost()
                            .label("收起编辑")
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.request_close_template(window, cx)
                            })),
                    )
                    .child(
                        Button::new("template-save")
                            .ghost()
                            .label(if self.template_edit_id.is_some() {
                                "更新模板"
                            } else {
                                "存为模板"
                            })
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.save_current_template(window, cx)
                            })),
                    ),
            );

        if embedded {
            panel.into_any_element()
        } else {
            modal(panel).into_any_element()
        }
    }

    fn batch_rename_dialog(&self, cx: &mut Context<Self>) -> AnyElement {
        let variables = [
            "{region}",
            "{region_code}",
            "{index}",
            "{index:02}",
            "{protocol}",
            "{source}",
            "{name}",
        ];
        let mut chips = div().flex().flex_wrap().gap_2();
        for variable in variables {
            chips = chips.child(static_chip(variable));
        }

        let panel = v_flex()
            .w(px(620.))
            .p(px(22.))
            .gap_3()
            .rounded(px(14.))
            .bg(rgb(SURFACE))
            .border_1()
            .border_color(rgb(BORDER))
            .child(
                div()
                    .text_size(px(17.))
                    .font_weight(FontWeight::SEMIBOLD)
                    .child("批量改名"),
            )
            .child(field_label("命名模板"))
            .child(Input::new(&self.rename_template).cleanable(true))
            .child(field_label("可用变量"))
            .child(chips)
            .when_some(self.rename_error.clone(), |this, error| {
                this.child(
                    div()
                        .text_size(px(12.))
                        .text_color(rgb(DANGER))
                        .child(error),
                )
            })
            .child(
                h_flex()
                    .gap_2()
                    .justify_end()
                    .child(Button::new("rename-cancel").ghost().label("取消").on_click(
                        cx.listener(|this, _, window, cx| this.close_batch_rename(window, cx)),
                    ))
                    .child(
                        Button::new("rename-apply")
                            .primary()
                            .label("应用")
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.apply_batch_rename(window, cx)
                            })),
                    ),
            );

        modal(panel).into_any_element()
    }

    fn diagnostics_dialog(&self, cx: &mut Context<Self>) -> AnyElement {
        use swiftsub_core::parser::ImportStatus;
        let mut rows = v_flex().gap_2().child(field_label("最近一次手动导入"));
        let mut displayed = 0;
        for (source, items) in [("最近导入".to_string(), self.import_report.clone())]
            .into_iter()
            .chain(
                self.source_import_report
                    .iter()
                    .map(|(source, item)| (source.clone(), vec![item.clone()])),
            )
        {
            for item in items {
                if displayed >= 500 {
                    break;
                }
                displayed += 1;
                let label = match item.status {
                    ImportStatus::Accepted => "成功",
                    ImportStatus::Warning => "需检查",
                    ImportStatus::Rejected => "失败 / 跳过",
                };
                let color = match item.status {
                    ImportStatus::Accepted => SUCCESS,
                    ImportStatus::Warning => WARNING,
                    ImportStatus::Rejected => DANGER,
                };
                rows = rows.child(
                    v_flex()
                        .gap_1()
                        .p_2()
                        .border_b_1()
                        .border_color(rgb(BORDER))
                        .child(
                            div()
                                .text_size(px(12.))
                                .text_color(rgb(color))
                                .child(format!(
                                    "{source} · #{} · {label} · {} · {}",
                                    item.index,
                                    item.name.unwrap_or_default(),
                                    item.protocol.unwrap_or_default()
                                )),
                        )
                        .child(div().text_size(px(11.)).child(item.messages.join("；"))),
                );
            }
        }
        rows = rows.child(
            div()
                .text_size(px(11.))
                .text_color(rgb(MUTED))
                .child("界面最多显示 500 项，导出包含全部记录。"),
        );
        for warning in &self.gen_warnings {
            rows = rows.child(
                div()
                    .text_size(px(11.))
                    .text_color(rgb(WARNING))
                    .child(warning.clone()),
            );
        }
        modal(v_flex().w(px(740.)).h(px(600.)).p_4().gap_3().rounded(px(12.)).bg(rgb(SURFACE))
            .child(field_label("导入诊断与配置追溯"))
            .child(div().text_size(px(11.)).text_color(rgb(MUTED)).child("链接报告的序号为原行号，YAML 为 proxies 项序号。成功表示解析成功；连接能力需另行测试。JSON 包含完整报告和生成配置摘要。"))
            .child(div().id("diagnostics-scroll").flex_1().min_h_0().overflow_y_scrollbar().child(rows))
            .child(h_flex().justify_end().gap_2()
                .child(Button::new("export-diagnostics").primary().label("导出完整报告").on_click(cx.listener(|this,_,_,cx|this.export_diagnostics(cx))))
                .child(Button::new("close-diagnostics").ghost().label("关闭").on_click(cx.listener(|this,_,_,cx|{this.diagnostics_open=false;cx.notify();})))))
            .into_any_element()
    }

    fn batch_import_dialog(&self, cx: &mut Context<Self>) -> AnyElement {
        let panel = v_flex()
            .w(px(720.))
            .p(px(22.))
            .gap_3()
            .rounded(px(14.))
            .bg(rgb(SURFACE))
            .border_1()
            .border_color(rgb(BORDER))
            .child(
                div()
                    .text_size(px(17.))
                    .font_weight(FontWeight::SEMIBOLD)
                    .child("批量导入节点"),
            )
            .child(Textarea::new(&self.batch_import).h(px(300.)))
            .when_some(self.batch_error.clone(), |this, error| {
                this.child(
                    div()
                        .text_size(px(12.))
                        .text_color(rgb(DANGER))
                        .child(error),
                )
            })
            .child(
                h_flex()
                    .gap_2()
                    .child(text_action(
                        "batch-paste-clipboard",
                        "从剪贴板填入",
                        ACCENT,
                        cx,
                        |this, window, cx| this.paste_batch_clipboard(window, cx),
                    ))
                    .child(div().flex_1())
                    .child(Button::new("batch-close").ghost().label("关闭").on_click(
                        cx.listener(|this, _, window, cx| this.close_batch_import(window, cx)),
                    ))
                    .child(
                        Button::new("batch-confirm")
                            .primary()
                            .label("导入")
                            .on_click(
                                cx.listener(|this, _, window, cx| this.import_batch(window, cx)),
                            ),
                    ),
            );

        modal(panel).into_any_element()
    }

    fn add_mode_pill(
        &self,
        label: &'static str,
        mode: AddMode,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let active = self.add_mode == mode;
        div()
            .id(format!("add-mode-{label}"))
            .cursor_pointer()
            .h(px(30.))
            .px(px(12.))
            .rounded(px(7.))
            .border_1()
            .flex()
            .items_center()
            .text_size(px(12.))
            .when(active, |this| {
                this.bg(rgb(ACCENT_SOFT))
                    .border_color(rgb(ACCENT))
                    .text_color(rgb(TEXT))
            })
            .when(!active, |this| {
                this.bg(rgb(SURFACE))
                    .border_color(rgb(BORDER))
                    .text_color(rgb(MUTED))
                    .hover(|style| style.bg(rgb(SURFACE_2)).text_color(rgb(TEXT)))
            })
            .child(label)
            .on_click(cx.listener(move |this, _, _, cx| this.set_add_mode(mode, cx)))
            .into_any_element()
    }

    fn edit_subscription_dialog(&self, _id: i64, cx: &mut Context<Self>) -> AnyElement {
        let Some(data) = &self.edit_data else {
            return div().into_any_element();
        };
        let is_url = data.kind == swiftsub_store::SubscriptionKind::Url;
        let kind_label = match data.kind {
            swiftsub_store::SubscriptionKind::Url => "订阅链接",
            swiftsub_store::SubscriptionKind::Yaml => "YAML",
            swiftsub_store::SubscriptionKind::Links => "节点链接",
            swiftsub_store::SubscriptionKind::File => "本地文件",
        };
        let used = data.traffic_upload.saturating_add(data.traffic_download);
        let traffic = if data.traffic_total > 0 {
            format!(
                "{} / {}",
                format_bytes(used),
                format_bytes(data.traffic_total)
            )
        } else {
            "—".to_string()
        };

        let mut node_rows = v_flex();
        for (name, region, protocol) in self.edit_nodes.iter().take(200) {
            node_rows = node_rows.child(
                h_flex()
                    .h(px(34.))
                    .px_3()
                    .gap_2()
                    .border_b_1()
                    .border_color(rgb(BORDER))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .text_size(px(12.5))
                            .truncate()
                            .child(name.clone()),
                    )
                    .child(tag(region, MUTED))
                    .child(
                        div()
                            .w(px(64.))
                            .text_size(px(11.))
                            .text_color(rgb(MUTED))
                            .child(protocol.clone()),
                    ),
            );
        }

        let node_list: AnyElement = if self.edit_nodes.is_empty() {
            div()
                .h(px(120.))
                .flex()
                .items_center()
                .justify_center()
                .text_size(px(12.5))
                .text_color(rgb(MUTED))
                .child("暂无缓存节点，刷新订阅后可查看")
                .into_any_element()
        } else {
            div()
                .id("edit-node-list")
                .h(px(170.))
                .rounded(px(8.))
                .border_1()
                .border_color(rgb(BORDER))
                .bg(rgb(SURFACE_2))
                .overflow_y_scrollbar()
                .child(node_rows)
                .into_any_element()
        };

        let panel = v_flex()
            .w(px(780.))
            .p(px(22.))
            .gap_3()
            .rounded(px(14.))
            .bg(rgb(SURFACE))
            .border_1()
            .border_color(rgb(BORDER))
            .child(
                div()
                    .text_size(px(17.))
                    .font_weight(FontWeight::SEMIBOLD)
                    .child("编辑订阅"),
            )
            .child(
                h_flex()
                    .gap_3()
                    .child(
                        v_flex()
                            .flex_1()
                            .gap_1()
                            .child(field_label("名称"))
                            .child(Input::new(&self.edit_name).cleanable(true)),
                    )
                    .when(is_url, |this| {
                        this.child(
                            v_flex()
                                .flex_1()
                                .gap_1()
                                .child(field_label("订阅链接"))
                                .child(Input::new(&self.edit_url).cleanable(true)),
                        )
                    }),
            )
            .child(
                h_flex()
                    .gap_3()
                    .child(
                        v_flex()
                            .flex_1()
                            .gap_1()
                            .child(field_label("刷新间隔（分钟）"))
                            .child(Input::new(&self.edit_interval).cleanable(true)),
                    )
                    .child(
                        v_flex()
                            .flex_1()
                            .gap_1()
                            .child(field_label("User-Agent"))
                            .child(Input::new(&self.edit_ua).cleanable(true)),
                    ),
            )
            .child(
                h_flex()
                    .gap_2()
                    .child(div().text_size(px(12.5)).child("启用订阅"))
                    .child(
                        Switch::new("edit-enabled")
                            .small()
                            .checked(self.edit_enabled)
                            .on_change(cx.listener(|this, checked, _, cx| {
                                this.set_edit_enabled(*checked, cx)
                            })),
                    ),
            )
            .child(
                h_flex()
                    .gap_4()
                    .child(
                        v_flex()
                            .flex_1()
                            .child(info_row("类型", kind_label.to_string(), false))
                            .child(info_row("节点数", data.node_count.to_string(), false))
                            .child(info_row(
                                "最近更新",
                                format_timestamp(data.last_ok_at),
                                false,
                            )),
                    )
                    .child(
                        v_flex()
                            .flex_1()
                            .child(info_row("流量", traffic, false))
                            .child(info_row("到期", format_expire(data.expire_at), false))
                            .child(info_row(
                                "状态",
                                if data.last_error.is_some() {
                                    "刷新失败"
                                } else {
                                    "正常"
                                }
                                .to_string(),
                                false,
                            )),
                    ),
            )
            .when_some(data.last_error.clone(), |this, error| {
                this.child(
                    div()
                        .text_size(px(12.))
                        .text_color(rgb(DANGER))
                        .child(error),
                )
            })
            .child(
                h_flex()
                    .justify_between()
                    .child(div().text_size(px(12.5)).text_color(rgb(MUTED)).child(
                        if self.edit_nodes.len() > 200 {
                            format!("节点列表（前 200 / 共 {}）", self.edit_nodes.len())
                        } else {
                            format!("节点列表（{}）", self.edit_nodes.len())
                        },
                    ))
                    .child(div()),
            )
            .child(node_list)
            .when_some(self.edit_error.clone(), |this, error| {
                this.child(
                    div()
                        .text_size(px(12.))
                        .text_color(rgb(DANGER))
                        .child(error),
                )
            })
            .child(
                h_flex()
                    .gap_2()
                    .justify_end()
                    .child(Button::new("edit-cancel").ghost().label("取消").on_click(
                        cx.listener(|this, _, window, cx| this.close_edit_subscription(window, cx)),
                    ))
                    .child(Button::new("edit-save").primary().label("保存").on_click(
                        cx.listener(|this, _, window, cx| this.save_edit_subscription(window, cx)),
                    )),
            );

        modal(panel).into_any_element()
    }

    fn confirm_delete_dialog(&self, id: i64, cx: &mut Context<Self>) -> AnyElement {
        let name = self
            .subscriptions
            .iter()
            .find(|subscription| subscription.id == id)
            .map(|subscription| subscription.name.clone())
            .unwrap_or_else(|| "该订阅".to_string());

        div()
            .absolute()
            .top_0()
            .left_0()
            .size_full()
            .occlude()
            .bg(rgba(0x05070bcc))
            .flex()
            .items_center()
            .justify_center()
            .child(
                v_flex()
                    .w(px(380.))
                    .p(px(20.))
                    .gap_3()
                    .rounded(px(14.))
                    .bg(rgb(SURFACE))
                    .border_1()
                    .border_color(rgb(BORDER))
                    .child(
                        div()
                            .text_size(px(15.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .child("删除订阅"),
                    )
                    .child(
                        div()
                            .text_size(px(12.))
                            .text_color(rgb(MUTED))
                            .child(format!(
                                "将移除「{name}」及其缓存节点，已生成的本地配置会立即重建。"
                            )),
                    )
                    .child(
                        h_flex()
                            .gap_2()
                            .justify_end()
                            .child(
                                Button::new("delete-cancel")
                                    .ghost()
                                    .label("取消")
                                    .on_click(cx.listener(|this, _, _, cx| this.cancel_delete(cx))),
                            )
                            .child(
                                Button::new("delete-confirm")
                                    .danger()
                                    .label("删除")
                                    .on_click(
                                        cx.listener(|this, _, _, cx| this.perform_delete(cx)),
                                    ),
                            ),
                    ),
            )
            .into_any_element()
    }

    fn generator_page(&mut self, width: Pixels, cx: &mut Context<Self>) -> AnyElement {
        let stacked_preview = self.preview_open && width < px(1380.);
        let layout_id = match (self.template_dialog_open, stacked_preview) {
            (true, true) => "workspace-edit-stacked",
            (true, false) => "workspace-edit-wide",
            (false, true) => "workspace-browse-stacked",
            (false, false) => "workspace-browse-wide",
        };
        let mut panels = h_resizable(layout_id);
        if self.nodes_panel_open {
            panels = panels.child(
                resizable_panel()
                    .size(px(if self.template_dialog_open {
                        250.
                    } else {
                        300.
                    }))
                    .size_range(px(230.)..px(460.))
                    .child(self.generator_nodes_panel(cx)),
            );
        }
        let editor = if self.template_dialog_open {
            self.template_dialog(cx)
        } else {
            self.generator_config_panel(cx)
        };
        let editor = if stacked_preview {
            v_resizable(if self.template_dialog_open {
                "edit-preview-stack"
            } else {
                "browse-preview-stack"
            })
            .child(resizable_panel().child(editor))
            .child(
                resizable_panel()
                    .size(px(190.))
                    .size_range(px(140.)..px(420.))
                    .child(self.generator_preview(false, cx)),
            )
            .into_any_element()
        } else {
            editor
        };
        panels = panels.child(
            resizable_panel()
                .size_range(px(420.)..px(3000.))
                .child(editor),
        );
        if self.preview_open && !stacked_preview {
            panels = panels.child(
                resizable_panel()
                    .size(px(340.))
                    .size_range(px(280.)..px(1000.))
                    .child(self.generator_preview(true, cx)),
            );
        }
        let profile_name = self
            .editing_saved_config
            .as_ref()
            .and_then(|id| self.saved_configs.iter().find(|c| &c.id == id))
            .map(|c| c.name.as_str())
            .or_else(|| {
                self.profiles
                    .iter()
                    .find(|p| p.id == self.profile_id)
                    .map(|p| p.name.as_str())
            })
            .unwrap_or("当前配置");
        v_flex()
            .size_full()
            .min_h_0()
            .child(
                h_flex()
                    .px_3()
                    .py_2()
                    .gap_2()
                    .flex_wrap()
                    .border_b_1()
                    .border_color(rgb(BORDER))
                    .child(
                        Button::new("workspace-nodes")
                            .ghost()
                            .label(if self.nodes_panel_open {
                                "收起节点"
                            } else {
                                "显示节点"
                            })
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.nodes_panel_open = !this.nodes_panel_open;
                                cx.notify();
                            })),
                    )
                    .child(
                        div()
                            .flex_1()
                            .text_size(px(12.))
                            .text_color(rgb(MUTED))
                            .min_w_0()
                            .truncate()
                            .child(format!(
                                "{} · {}",
                                profile_name,
                                if self.template_dialog_open {
                                    "自动保存"
                                } else {
                                    "当前输出"
                                }
                            )),
                    )
                    .child(
                        Button::new("workspace-edit")
                            .ghost()
                            .label(if self.template_dialog_open {
                                "收起分流"
                            } else {
                                "编辑分流"
                            })
                            .on_click(cx.listener(|this, _, window, cx| {
                                if this.template_dialog_open {
                                    this.request_close_template(window, cx);
                                } else {
                                    this.open_template_dialog(window, cx);
                                }
                            })),
                    )
                    .child(
                        Button::new("workspace-preview")
                            .ghost()
                            .label(if self.preview_open {
                                "收起预览"
                            } else {
                                "显示预览"
                            })
                            .on_click(cx.listener(|this, _, _, cx| this.toggle_preview(cx))),
                    )
                    .child(
                        Button::new("workspace-diagnostics")
                            .ghost()
                            .label("诊断")
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.diagnostics_open = true;
                                cx.notify();
                            })),
                    )
                    .when(self.editing_saved_config.is_some(), |bar| {
                        bar.child(
                            Button::new("update-saved-generated")
                                .ghost()
                                .label("更新原配置")
                                .on_click(cx.listener(|this, _, window, cx| {
                                    this.save_generated_config(true, window, cx)
                                })),
                        )
                    })
                    .child(
                        Button::new("save-generated-new")
                            .primary()
                            .label("保存新配置")
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.save_generated_config(false, window, cx)
                            })),
                    )
                    .child(
                        Button::new("workspace-export")
                            .ghost()
                            .label("导出")
                            .on_click(cx.listener(|this, _, _, cx| this.export_preview(cx))),
                    ),
            )
            .child(div().flex_1().min_h_0().child(panels))
            .into_any_element()
    }

    fn generator_nodes_panel(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let enabled_count = self.nodes.iter().filter(|node| node.enabled).count();
        let query = self.search.read(cx).value().trim().to_lowercase();
        let visible_count = if query.is_empty() {
            self.nodes.len()
        } else {
            self.nodes
                .iter()
                .filter(|node| node.name().to_lowercase().contains(&query))
                .count()
        };
        let node_summary = if query.is_empty() {
            format!("{} / {} 启用", enabled_count, self.nodes.len())
        } else {
            format!("显示 {visible_count} / {}", self.nodes.len())
        };
        let mut rows = div()
            .id("node-list")
            .flex_1()
            .min_h_0()
            .rounded(px(10.))
            .border_1()
            .border_color(rgb(BORDER))
            .bg(rgb(SURFACE))
            .overflow_y_scrollbar()
            .v_flex();

        if self.nodes.is_empty() {
            rows = rows.child(
                div()
                    .p_4()
                    .text_size(px(12.5))
                    .text_color(rgb(MUTED))
                    .child("暂无节点，先在订阅源添加订阅"),
            );
        } else {
            let filtered: Vec<&Node> = self
                .nodes
                .iter()
                .filter(|node| query.is_empty() || node.name().to_lowercase().contains(&query))
                .take(160)
                .collect();
            if filtered.is_empty() {
                rows = rows.child(
                    div()
                        .p_4()
                        .text_size(px(12.5))
                        .text_color(rgb(MUTED))
                        .child("没有匹配的节点"),
                );
            }
            for node in filtered {
                rows = rows.child(self.node_row(node, cx));
            }
        }

        v_flex()
            .w_full()
            .h_full()
            .min_h_0()
            .border_r_1()
            .border_color(rgb(BORDER))
            .child(
                v_flex()
                    .p(px(16.))
                    .pb(px(10.))
                    .gap_2()
                    .child(
                        h_flex()
                            .gap_2()
                            .child(
                                div()
                                    .flex_1()
                                    .text_size(px(15.))
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .child("节点管理"),
                            )
                            .child(text_action(
                                "batch-rename-open",
                                "批量改名",
                                ACCENT,
                                cx,
                                |this, _, cx| this.open_batch_rename(cx),
                            )),
                    )
                    .child(
                        div()
                            .text_size(px(11.))
                            .text_color(rgb(MUTED))
                            .child(node_summary),
                    )
                    .child(Input::new(&self.search).cleanable(true)),
            )
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .px(px(16.))
                    .pb(px(16.))
                    .v_flex()
                    .child(rows),
            )
            .into_any_element()
    }

    fn generator_config_panel(&mut self, cx: &mut Context<Self>) -> AnyElement {
        v_flex()
            .flex_1()
            .min_w_0()
            .h_full()
            .min_h_0()
            .child(
                h_flex()
                    .justify_between()
                    .px(px(20.))
                    .pt(px(16.))
                    .pb(px(10.))
                    .child(
                        div()
                            .text_size(px(16.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .child("分流配置"),
                    )
                    .child(
                        h_flex()
                            .gap_2()
                            .child(text_action(
                                "open-rule-sets",
                                "规则集",
                                ACCENT,
                                cx,
                                |this, window, cx| this.open_template_dialog(window, cx),
                            ))
                            .child(text_action(
                                "open-template-dialog",
                                "存为模板",
                                ACCENT,
                                cx,
                                |this, window, cx| this.open_template_dialog(window, cx),
                            )),
                    ),
            )
            .child(
                div()
                    .id("generator-config-scroll")
                    .flex_1()
                    .min_h_0()
                    .px(px(20.))
                    .pb(px(16.))
                    .overflow_y_scrollbar()
                    .v_flex()
                    .gap_3()
                    .child(self.custom_groups_section(cx))
                    .child(self.chains_section(cx))
                    .child(self.modules_section(cx)),
            )
            .into_any_element()
    }

    fn node_row(&self, node: &Node, cx: &mut Context<Self>) -> AnyElement {
        let key = node.source_key.clone();
        let name = node.name().to_string();
        let source_id = node.source_id.clone();
        let yaml = node.proxy.to_yaml_text().unwrap_or_default();
        let switch = Switch::new(format!("node-switch-{key}"))
            .small()
            .checked(node.enabled)
            .on_change(cx.listener({
                let key = key.clone();
                move |this, checked, _, cx| this.toggle_node(key.clone(), *checked, cx)
            }));
        let name_cell = div()
            .id(format!("node-name-{key}"))
            .flex_1()
            .min_w_0()
            .text_size(px(12.5))
            .text_color(rgb(if node.enabled { TEXT } else { MUTED }))
            .truncate()
            .cursor_pointer()
            .child(name.clone())
            .on_double_click(cx.listener({
                let key = key.clone();
                let name = name.clone();
                let source_id = source_id.clone();
                let yaml = yaml.clone();
                move |this, _, window, cx| {
                    this.open_node_rename(
                        key.clone(),
                        source_id.clone(),
                        name.clone(),
                        yaml.clone(),
                        window,
                        cx,
                    )
                }
            }));
        v_flex()
            .id(format!("node-row-{key}"))
            .min_h(px(64.))
            .px_2()
            .py_1()
            .gap_1()
            .justify_center()
            .border_b_1()
            .border_color(rgb(BORDER))
            .hover(|style| style.bg(rgb(SURFACE_2)))
            .child(
                h_flex()
                    .gap_1()
                    .child(
                        div()
                            .id(format!("node-drag-{key}"))
                            .w(px(18.))
                            .h(px(24.))
                            .flex_shrink_0()
                            .flex()
                            .items_center()
                            .justify_center()
                            .rounded(px(5.))
                            .text_size(px(12.))
                            .text_color(rgb(MUTED))
                            .cursor_pointer()
                            .hover(|style| style.bg(rgb(SURFACE_3)))
                            .child("≡")
                            .on_drag(
                                NodeDrag {
                                    key: key.clone(),
                                    name: name.clone(),
                                },
                                |drag, _, _, cx| {
                                    cx.new(|_| NodeDragPreview {
                                        name: drag.name.clone(),
                                    })
                                },
                            ),
                    )
                    .child(name_cell)
                    .child(switch),
            )
            .child(
                h_flex()
                    .pl(px(22.))
                    .gap_1()
                    .child(tag(node.region.label(), MUTED))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .text_size(px(10.))
                            .text_color(rgb(MUTED))
                            .child(node.proxy.kind().to_uppercase()),
                    )
                    .child(icon_action(format!("share-node-{key}"), "▦", MUTED, cx, {
                        let key = key.clone();
                        move |this, _, cx| this.share_node_qr(&key, cx)
                    }))
                    .child(icon_action(
                        format!("rename-node-{key}"),
                        "✎",
                        MUTED,
                        cx,
                        {
                            let key = key.clone();
                            let name = name.clone();
                            let source_id = source_id.clone();
                            let yaml = yaml.clone();
                            move |this, window, cx| {
                                this.open_node_rename(
                                    key.clone(),
                                    source_id.clone(),
                                    name.clone(),
                                    yaml.clone(),
                                    window,
                                    cx,
                                )
                            }
                        },
                    )),
            )
            .on_drop(cx.listener(move |this, drag: &NodeDrag, _, cx| {
                this.move_node(drag.key.clone(), key.clone(), cx)
            }))
            .into_any_element()
    }

    fn modules_section(&self, cx: &mut Context<Self>) -> AnyElement {
        let core: Vec<&Module> = catalog::MODULES
            .iter()
            .filter(|module| matches!(module.id, "ad" | "private" | "cn" | "global"))
            .collect();
        let services: Vec<&Module> = catalog::MODULES
            .iter()
            .filter(|module| matches!(module.kind, ModuleKind::Service) && module.id != "global")
            .collect();
        let total = core.len() + services.len();
        let enabled = core
            .iter()
            .chain(services.iter())
            .filter(|module| self.is_module_enabled(module.id))
            .count();

        v_flex()
            .gap_3()
            .child(
                v_flex().gap_2().child(section_label("模板")).child(
                    h_flex()
                        .gap_2()
                        .child(self.preset_pill("精简", Preset::Minimal, cx))
                        .child(self.preset_pill("标准", Preset::Standard, cx))
                        .child(self.preset_pill("完整", Preset::Full, cx)),
                ),
            )
            .child(
                h_flex().gap_2().child(section_label("常开组")).child(
                    div()
                        .text_size(px(12.))
                        .text_color(rgb(MUTED))
                        .child("节点选择 · 自动选择 · 漏网之鱼"),
                ),
            )
            .child(
                h_flex()
                    .justify_between()
                    .child(section_label("分流代理组"))
                    .child(
                        div()
                            .text_size(px(11.5))
                            .text_color(rgb(MUTED))
                            .child(format!("{enabled}/{total}")),
                    ),
            )
            .when(!self.options.policy_groups.is_empty(), |this| {
                this.child(
                    v_flex()
                        .gap_1()
                        .child(section_label("自定义策略组"))
                        .child(
                            div().text_size(px(12.)).text_color(rgb(MUTED)).child(
                                self.options
                                    .policy_groups
                                    .iter()
                                    .map(|group| group.name.as_str())
                                    .collect::<Vec<_>>()
                                    .join(" · "),
                            ),
                        )
                        .child(text_action(
                            "edit-active-policy-groups",
                            "配置策略组与规则目标",
                            ACCENT,
                            cx,
                            |this, window, cx| this.open_template_dialog(window, cx),
                        )),
                )
            })
            .child(self.module_category("core", "核心", &core, cx))
            .child(self.module_category("service", "服务", &services, cx))
            .into_any_element()
    }

    fn preset_pill(
        &self,
        label: &'static str,
        preset: Preset,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let active = self.options.preset == preset;
        div()
            .id(format!("preset-{label}"))
            .cursor_pointer()
            .h(px(30.))
            .px(px(12.))
            .rounded(px(7.))
            .border_1()
            .flex()
            .items_center()
            .text_size(px(12.))
            .when(active, |this| {
                this.bg(rgb(ACCENT_SOFT))
                    .border_color(rgb(ACCENT))
                    .text_color(rgb(TEXT))
            })
            .when(!active, |this| {
                this.bg(rgb(SURFACE))
                    .border_color(rgb(BORDER))
                    .text_color(rgb(MUTED))
                    .hover(|style| style.bg(rgb(SURFACE_2)).text_color(rgb(TEXT)))
            })
            .child(label)
            .on_click(cx.listener(move |this, _, _, cx| this.set_preset(preset, cx)))
            .into_any_element()
    }

    fn module_category(
        &self,
        id: &'static str,
        label: &'static str,
        modules: &[&'static Module],
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let open = self.is_module_category_open(id);
        let enabled = modules
            .iter()
            .filter(|module| self.is_module_enabled(module.id))
            .count();
        let header = h_flex()
            .id(format!("module-category-{id}"))
            .cursor_pointer()
            .h(px(34.))
            .px_2()
            .gap_2()
            .rounded(px(7.))
            .hover(|style| style.bg(rgb(SURFACE_2)))
            .child(
                div()
                    .w(px(14.))
                    .text_size(px(12.))
                    .text_color(rgb(MUTED))
                    .child(if open { "▾" } else { "▸" }),
            )
            .child(
                div()
                    .text_size(px(12.5))
                    .font_weight(FontWeight::MEDIUM)
                    .child(label),
            )
            .child(div().flex_1())
            .child(
                div()
                    .text_size(px(11.5))
                    .text_color(rgb(MUTED))
                    .child(format!("{enabled}/{}", modules.len())),
            )
            .on_click(cx.listener(move |this, _, _, cx| this.toggle_module_category(id, cx)));

        let mut rows = v_flex()
            .rounded(px(8.))
            .border_1()
            .border_color(rgb(BORDER))
            .bg(rgb(SURFACE))
            .overflow_hidden();
        for module in modules {
            rows = rows.child(self.module_row(module, cx));
        }

        v_flex()
            .gap_1()
            .child(header)
            .when(open, |this| this.child(rows))
            .into_any_element()
    }

    fn module_row(&self, module: &'static Module, cx: &mut Context<Self>) -> AnyElement {
        let active = self.is_module_enabled(module.id);
        let meta = match module.kind {
            ModuleKind::Region(region) => format!(
                "{} 节点",
                self.nodes
                    .iter()
                    .filter(|node| node.enabled && node.region == region)
                    .count()
            ),
            _ => format!("{} 规则", module.rule_sets.len()),
        };
        let module_id = module.id.to_string();
        h_flex()
            .h(px(44.))
            .px_3()
            .gap_3()
            .border_b_1()
            .border_color(rgb(BORDER))
            .hover(|style| style.bg(rgb(0x11151b)))
            .child(
                v_flex()
                    .flex_1()
                    .min_w_0()
                    .gap_1()
                    .child(
                        div()
                            .text_size(px(12.5))
                            .font_weight(FontWeight::MEDIUM)
                            .truncate()
                            .child(module.name),
                    )
                    .child(
                        div()
                            .text_size(px(11.5))
                            .text_color(rgb(MUTED))
                            .truncate()
                            .child(module.description),
                    ),
            )
            .child(div().text_size(px(11.5)).text_color(rgb(MUTED)).child(meta))
            .child(
                Switch::new(format!("module-switch-{}", module.id))
                    .small()
                    .checked(active)
                    .on_change(cx.listener(move |this, checked, _, cx| {
                        this.toggle_module(module_id.clone(), *checked, cx)
                    })),
            )
            .into_any_element()
    }

    fn custom_groups_section(&self, cx: &mut Context<Self>) -> AnyElement {
        let mut rows = v_flex().gap_1();
        for (index, group) in self.custom_groups.iter().enumerate() {
            let count = self
                .nodes
                .iter()
                .filter(|node| group.filter.matches(node))
                .count();
            rows = rows.child(
                h_flex()
                    .min_h(px(42.))
                    .px_3()
                    .py_2()
                    .gap_2()
                    .rounded(px(8.))
                    .bg(rgb(SURFACE))
                    .border_1()
                    .border_color(rgb(BORDER))
                    .child(
                        v_flex()
                            .flex_1()
                            .min_w_0()
                            .gap_1()
                            .child(
                                div()
                                    .text_size(px(12.5))
                                    .font_weight(FontWeight::MEDIUM)
                                    .truncate()
                                    .child(group.name.clone()),
                            )
                            .child(
                                div()
                                    .text_size(px(11.5))
                                    .text_color(rgb(MUTED))
                                    .truncate()
                                    .child(format!("{} · {count} 节点", group.filter.summary())),
                            ),
                    )
                    .child(icon_action(
                        format!("delete-filter-{index}"),
                        "×",
                        DANGER,
                        cx,
                        move |this, _, cx| this.delete_filter_group(index, cx),
                    )),
            );
        }

        v_flex()
            .gap_2()
            .child(
                h_flex()
                    .justify_between()
                    .child(section_label("筛选代理组"))
                    .child(icon_action(
                        "add-filter-group",
                        "+",
                        ACCENT,
                        cx,
                        |this, _, cx| this.open_custom_dialog(CustomDialog::Filter, cx),
                    )),
            )
            .when(!self.custom_groups.is_empty(), |this| this.child(rows))
            .into_any_element()
    }

    fn chains_section(&self, cx: &mut Context<Self>) -> AnyElement {
        let mut rows = v_flex().gap_1();
        for (index, chain) in self.chains.iter().enumerate() {
            let entry_name = self
                .nodes
                .iter()
                .find(|node| node.proxy.identity_key() == chain.entry_node)
                .map(|node| node.name().to_string())
                .unwrap_or_else(|| chain.entry_name.clone());
            let landing_name = self
                .nodes
                .iter()
                .find(|node| node.proxy.identity_key() == chain.landing_node)
                .map(|node| node.name().to_string())
                .unwrap_or_else(|| chain.landing_name.clone());
            rows = rows.child(
                h_flex()
                    .min_h(px(42.))
                    .px_3()
                    .py_2()
                    .gap_2()
                    .rounded(px(8.))
                    .bg(rgb(SURFACE))
                    .border_1()
                    .border_color(rgb(BORDER))
                    .child(
                        v_flex()
                            .flex_1()
                            .min_w_0()
                            .gap_1()
                            .child(
                                div()
                                    .text_size(px(12.5))
                                    .font_weight(FontWeight::MEDIUM)
                                    .truncate()
                                    .child(chain.name.clone()),
                            )
                            .child(
                                div()
                                    .text_size(px(11.5))
                                    .text_color(rgb(MUTED))
                                    .truncate()
                                    .child(format!("{entry_name} → {landing_name}")),
                            ),
                    )
                    .child(icon_action(
                        format!("delete-chain-{index}"),
                        "×",
                        DANGER,
                        cx,
                        move |this, _, cx| this.delete_chain(index, cx),
                    )),
            );
        }

        v_flex()
            .gap_2()
            .child(
                h_flex()
                    .justify_between()
                    .child(section_label("链式代理"))
                    .child(icon_action("add-chain", "+", ACCENT, cx, |this, _, cx| {
                        this.open_custom_dialog(CustomDialog::Chain, cx)
                    })),
            )
            .when(!self.chains.is_empty(), |this| this.child(rows))
            .into_any_element()
    }

    fn region_chip(&self, region: Region, selected: bool, cx: &mut Context<Self>) -> AnyElement {
        div()
            .id(format!("region-{:?}", region))
            .cursor_pointer()
            .h(px(30.))
            .px(px(10.))
            .rounded(px(7.))
            .border_1()
            .flex()
            .items_center()
            .text_size(px(12.))
            .when(selected, |this| {
                this.bg(rgb(ACCENT_SOFT))
                    .border_color(rgb(ACCENT))
                    .text_color(rgb(TEXT))
            })
            .when(!selected, |this| {
                this.bg(rgb(SURFACE))
                    .border_color(rgb(BORDER))
                    .text_color(rgb(MUTED))
                    .hover(|style| style.bg(rgb(SURFACE_2)).text_color(rgb(TEXT)))
            })
            .child(region.label())
            .on_click(cx.listener(move |this, _, _, cx| this.toggle_filter_region(region, cx)))
            .into_any_element()
    }

    fn custom_dialog_view(&self, cx: &mut Context<Self>) -> AnyElement {
        match self.custom_dialog {
            CustomDialog::Filter => self.filter_dialog(cx),
            CustomDialog::Chain => self.chain_dialog(cx),
            CustomDialog::None => div().into_any_element(),
        }
    }

    fn filter_dialog(&self, cx: &mut Context<Self>) -> AnyElement {
        let mut regions = div().flex().flex_wrap().gap_2();
        for region in Region::ALL {
            regions =
                regions.child(self.region_chip(region, self.filter_regions.contains(&region), cx));
        }

        let panel =
            v_flex()
                .w(px(640.))
                .p(px(22.))
                .gap_3()
                .rounded(px(14.))
                .bg(rgb(SURFACE))
                .border_1()
                .border_color(rgb(BORDER))
                .child(
                    div()
                        .text_size(px(17.))
                        .font_weight(FontWeight::SEMIBOLD)
                        .child("新建筛选代理组"),
                )
                .child(field_label("名称（可选）"))
                .child(Input::new(&self.filter_name).cleanable(true))
                .child(field_label("地区（可多选）"))
                .child(regions)
                .child(field_label("包含关键词"))
                .child(Input::new(&self.filter_include).cleanable(true))
                .child(field_label("排除关键词"))
                .child(Input::new(&self.filter_exclude).cleanable(true))
                .when_some(self.custom_error.clone(), |this, error| {
                    this.child(
                        div()
                            .text_size(px(12.))
                            .text_color(rgb(DANGER))
                            .child(error),
                    )
                })
                .child(
                    h_flex()
                        .gap_2()
                        .justify_end()
                        .child(Button::new("filter-cancel").ghost().label("取消").on_click(
                            cx.listener(|this, _, window, cx| this.close_custom_dialog(window, cx)),
                        ))
                        .child(
                            Button::new("filter-add")
                                .primary()
                                .label("添加筛选组")
                                .on_click(cx.listener(|this, _, window, cx| {
                                    this.add_filter_group(window, cx)
                                })),
                        ),
                );

        modal(panel).into_any_element()
    }

    fn chain_dialog(&self, cx: &mut Context<Self>) -> AnyElement {
        let query = self.chain_search.read(cx).value().trim().to_lowercase();
        let visible_nodes: Vec<&Node> = self
            .nodes
            .iter()
            .filter(|node| {
                node.enabled && (query.is_empty() || node.name().to_lowercase().contains(&query))
            })
            .take(120)
            .collect();

        let entry_key = self.chain_entry_key.as_deref();
        let landing_key = self.chain_landing_key.as_deref();
        let mut entry_rows = v_flex();
        let mut landing_rows = v_flex();
        for node in &visible_nodes {
            let key = node.proxy.identity_key();
            entry_rows = entry_rows.child(self.chain_node_row(
                node,
                ChainSide::Entry,
                entry_key == Some(key.as_str()),
                cx,
            ));
            landing_rows = landing_rows.child(self.chain_node_row(
                node,
                ChainSide::Landing,
                landing_key == Some(key.as_str()),
                cx,
            ));
        }

        let build_list = |rows: AnyElement, id: &'static str| -> AnyElement {
            div()
                .id(id)
                .h(px(220.))
                .rounded(px(8.))
                .border_1()
                .border_color(rgb(BORDER))
                .bg(rgb(SURFACE_2))
                .overflow_y_scrollbar()
                .child(rows)
                .into_any_element()
        };
        let entry_list = if visible_nodes.is_empty() {
            div()
                .h(px(220.))
                .flex()
                .items_center()
                .justify_center()
                .text_size(px(12.5))
                .text_color(rgb(MUTED))
                .child("没有可用节点")
                .into_any_element()
        } else {
            build_list(entry_rows.into_any_element(), "chain-entry-list")
        };
        let landing_list = if visible_nodes.is_empty() {
            div()
                .h(px(220.))
                .flex()
                .items_center()
                .justify_center()
                .text_size(px(12.5))
                .text_color(rgb(MUTED))
                .child("没有可用节点")
                .into_any_element()
        } else {
            build_list(landing_rows.into_any_element(), "chain-landing-list")
        };

        let entry_selected = self
            .chain_entry_name
            .clone()
            .unwrap_or_else(|| "未选择".to_string());
        let landing_selected = self
            .chain_landing_name
            .clone()
            .unwrap_or_else(|| "未选择".to_string());

        let panel = v_flex()
            .w(px(840.))
            .p(px(22.))
            .gap_3()
            .rounded(px(14.))
            .bg(rgb(SURFACE))
            .border_1()
            .border_color(rgb(BORDER))
            .child(
                div()
                    .text_size(px(17.))
                    .font_weight(FontWeight::SEMIBOLD)
                    .child("新建链式代理"),
            )
            .child(field_label("名称（可选）"))
            .child(Input::new(&self.chain_name).cleanable(true))
            .child(Input::new(&self.chain_search).cleanable(true))
            .child(
                h_flex()
                    .items_start()
                    .gap_3()
                    .child(
                        v_flex()
                            .flex_1()
                            .min_w_0()
                            .gap_2()
                            .child(
                                h_flex()
                                    .justify_between()
                                    .gap_2()
                                    .child(section_label("中转节点"))
                                    .child(
                                        div()
                                            .min_w_0()
                                            .truncate()
                                            .text_size(px(11.5))
                                            .text_color(rgb(MUTED))
                                            .child(entry_selected),
                                    ),
                            )
                            .child(entry_list),
                    )
                    .child(
                        v_flex()
                            .flex_1()
                            .min_w_0()
                            .gap_2()
                            .child(
                                h_flex()
                                    .justify_between()
                                    .gap_2()
                                    .child(section_label("落地节点"))
                                    .child(
                                        div()
                                            .min_w_0()
                                            .truncate()
                                            .text_size(px(11.5))
                                            .text_color(rgb(MUTED))
                                            .child(landing_selected),
                                    ),
                            )
                            .child(landing_list),
                    ),
            )
            .when_some(self.custom_error.clone(), |this, error| {
                this.child(
                    div()
                        .text_size(px(12.))
                        .text_color(rgb(DANGER))
                        .child(error),
                )
            })
            .child(
                h_flex()
                    .gap_2()
                    .justify_end()
                    .child(Button::new("chain-cancel").ghost().label("取消").on_click(
                        cx.listener(|this, _, window, cx| this.close_custom_dialog(window, cx)),
                    ))
                    .child(
                        Button::new("chain-add")
                            .primary()
                            .label("添加链式代理")
                            .on_click(
                                cx.listener(|this, _, window, cx| this.add_chain(window, cx)),
                            ),
                    ),
            );

        modal(panel).into_any_element()
    }

    fn chain_node_row(
        &self,
        node: &Node,
        side: ChainSide,
        selected: bool,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let key = node.proxy.identity_key();
        let name = node.name().to_string();
        div()
            .id(format!("chain-node-{:?}-{key}", side))
            .cursor_pointer()
            .h(px(38.))
            .px_3()
            .gap_2()
            .border_b_1()
            .border_color(rgb(BORDER))
            .flex()
            .items_center()
            .when(selected, |this| this.bg(rgb(ACCENT_SOFT)))
            .hover(|style| style.bg(rgb(SURFACE_2)))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .text_size(px(12.5))
                    .truncate()
                    .child(name.clone()),
            )
            .child(tag(node.region.label(), MUTED))
            .child(
                div()
                    .w(px(78.))
                    .text_size(px(11.))
                    .text_color(rgb(MUTED))
                    .child(node.proxy.kind().to_uppercase()),
            )
            .on_click(cx.listener(move |this, _, _, cx| {
                this.select_chain_node(side, key.clone(), name.clone(), cx)
            }))
            .into_any_element()
    }

    fn generator_preview(&mut self, show_url: bool, cx: &mut Context<Self>) -> AnyElement {
        let stats = || {
            h_flex()
                .flex_wrap()
                .gap_2()
                .child(stat_chip("节点", self.stats.proxies))
                .child(stat_chip("代理组", self.stats.groups))
                .child(stat_chip("规则", self.stats.rules))
                .child(stat_chip("规则集", self.stats.rule_sets))
        };
        let header = h_flex()
            .gap_2()
            .flex_wrap()
            .min_w_0()
            .child(
                div()
                    .flex_1()
                    .min_w(px(85.))
                    .flex_shrink_0()
                    .text_size(px(16.))
                    .font_weight(FontWeight::SEMIBOLD)
                    .child("当前输出"),
            )
            .when(!show_url, |row| row.child(stats()))
            .child(
                Button::new("copy-preview")
                    .primary()
                    .label("复制配置")
                    .on_click(cx.listener(|this, _, _, cx| this.copy_preview(cx))),
            )
            .child(icon_action(
                "close-preview",
                "×",
                MUTED,
                cx,
                |this, _, cx| this.toggle_preview(cx),
            ));
        let header = v_flex()
            .px(px(16.))
            .py(px(10.))
            .gap_2()
            .flex_shrink_0()
            .border_b_1()
            .border_color(rgb(BORDER))
            .child(header)
            .when(show_url, |column| column.child(stats()));

        let mut body = div()
            .id("preview-scroll")
            .flex_1()
            .min_h_0()
            .p(px(16.))
            .overflow_y_scroll()
            .track_scroll(&self.preview_scroll);

        if self.preview.is_empty() {
            body = body.child(
                v_flex()
                    .size_full()
                    .items_center()
                    .justify_center()
                    .gap_2()
                    .child(
                        div()
                            .text_size(px(15.))
                            .font_weight(FontWeight::MEDIUM)
                            .child("还没有可预览的配置"),
                    )
                    .child(
                        div()
                            .text_size(px(12.5))
                            .text_color(rgb(MUTED))
                            .child("添加订阅并启用节点后，SwiftSub 会自动生成 YAML"),
                    )
                    .child(
                        Button::new("preview-add")
                            .primary()
                            .label("添加订阅")
                            .on_click(cx.listener(|this, _, _, cx| this.open_add_dialog(cx))),
                    ),
            );
        } else {
            let total_lines = self.preview.lines().count();
            let mut text = self
                .preview
                .lines()
                .take(400)
                .collect::<Vec<_>>()
                .join("\n");
            if total_lines > 400 {
                text.push_str(&format!(
                    "\n\n# ... 仅显示前 400 行，共 {total_lines} 行。复制可获取完整配置。"
                ));
            }
            body = body.child(
                div()
                    .font_family("Consolas")
                    .text_size(px(12.5))
                    .line_height(px(19.))
                    .text_color(rgb(0xc9d1e0))
                    .child(text),
            );
        }

        let mut warnings: Vec<String> = self.gen_warnings.clone();
        warnings.extend(self.parse_warnings.clone());
        warnings.dedup();
        let warning_banner = if warnings.is_empty() {
            None
        } else {
            let first = warnings.first().cloned().unwrap_or_default();
            let extra = warnings.len().saturating_sub(1);
            Some(
                h_flex()
                    .mx(px(16.))
                    .my(px(10.))
                    .px_3()
                    .py_2()
                    .gap_2()
                    .rounded(px(8.))
                    .bg(rgb(0x2a2110))
                    .border_1()
                    .border_color(rgb(0x5b4316))
                    .child(
                        div()
                            .text_size(px(12.))
                            .text_color(rgb(WARNING))
                            .truncate()
                            .child(if extra > 0 {
                                format!("{first}（另有 {extra} 条提示）")
                            } else {
                                first
                            }),
                    ),
            )
        };

        let mut panel = v_flex()
            .flex_1()
            .min_w_0()
            .h_full()
            .min_h_0()
            .bg(rgb(BG))
            .child(header);

        if let Some(banner) = warning_banner {
            panel = panel.child(banner);
        }

        if let Some(url) = self.server_url.as_ref().filter(|_| show_url) {
            panel = panel.child(
                h_flex()
                    .min_w_0()
                    .mx(px(16.))
                    .mt(px(10.))
                    .px_3()
                    .py_2()
                    .gap_2()
                    .rounded(px(8.))
                    .bg(rgb(SURFACE))
                    .border_1()
                    .border_color(rgb(BORDER))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .font_family("Consolas")
                            .text_size(px(11.5))
                            .text_color(rgb(MUTED))
                            .child(url.clone()),
                    )
                    .child(text_action(
                        "copy-url-inline",
                        "复制",
                        ACCENT,
                        cx,
                        |this, _, cx| this.copy_server_url(cx),
                    )),
            );
        }

        panel
            .child(body.vertical_scrollbar(&self.preview_scroll))
            .into_any_element()
    }

    fn templates_page(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let mut builtin_rows = v_flex()
            .rounded(px(10.))
            .border_1()
            .border_color(rgb(BORDER))
            .bg(rgb(SURFACE));
        builtin_rows = builtin_rows
            .child(self.builtin_template_row(
                Preset::Minimal,
                "精简",
                "基础代理组与国内外分流，适合轻度使用",
                cx,
            ))
            .child(self.builtin_template_row(
                Preset::Standard,
                "标准",
                "常用服务分流，满足多数场景",
                cx,
            ))
            .child(self.builtin_template_row(Preset::Full, "完整", "启用全部服务与扩展规则集", cx));

        let mut custom_rows = v_flex().gap_1();
        for template in &self.custom_templates {
            let (module_count, rule_set_count, custom_rule_count) =
                template_summary_counts(&template.config_json);
            custom_rows = custom_rows.child(
                h_flex()
                    .min_h(px(72.))
                    .flex_wrap()
                    .px_4()
                    .gap_3()
                    .rounded(px(8.))
                    .bg(rgb(SURFACE))
                    .border_1()
                    .border_color(rgb(BORDER))
                    .child(
                        v_flex()
                            .flex_1()
                            .min_w_0()
                            .gap_1()
                            .child(
                                div()
                                    .text_size(px(13.))
                                    .font_weight(FontWeight::MEDIUM)
                                    .truncate()
                                    .child(template.name.clone()),
                            )
                            .child(
                                div()
                                    .text_size(px(11.5))
                                    .text_color(rgb(MUTED))
                                    .truncate()
                                    .child(format!(
                                        "{} 策略组 · {} 规则集 · {} 自定义规则 · 创建于 {}",
                                        module_count,
                                        rule_set_count,
                                        custom_rule_count,
                                        format_timestamp(Some(template.created_at))
                                    )),
                            ),
                    )
                    .child(
                        Button::new(format!("apply-template-{}", template.id))
                            .ghost()
                            .label("编辑 / 预览")
                            .on_click(cx.listener({
                                let id = template.id;
                                move |this, _, window, cx| {
                                    this.apply_custom_template(id, window, cx)
                                }
                            })),
                    )
                    .child(
                        Button::new(format!("copy-template-{}", template.id))
                            .ghost()
                            .label("复制")
                            .on_click(cx.listener({
                                let id = template.id;
                                move |this, _, _, cx| this.copy_custom_template(id, cx)
                            })),
                    )
                    .child(
                        Button::new(format!("export-template-{}", template.id))
                            .ghost()
                            .label("导出")
                            .on_click(cx.listener({
                                let id = template.id;
                                move |this, _, _, cx| this.export_custom_template(id, cx)
                            })),
                    )
                    .child(icon_action(
                        format!("delete-template-{}", template.id),
                        "×",
                        DANGER,
                        cx,
                        {
                            let id = template.id;
                            move |this, _, cx| this.delete_custom_template(id, cx)
                        },
                    )),
            );
        }

        let my_section = v_flex()
            .gap_2()
            .child(
                h_flex()
                    .justify_between()
                    .flex_wrap()
                    .child(section_label("我的分流模板"))
                    .child(
                        h_flex()
                            .gap_2()
                            .child(
                                div()
                                    .text_size(px(11.5))
                                    .text_color(rgb(MUTED))
                                    .child(format!("{} 个", self.custom_templates.len())),
                            )
                            .child(
                                Button::new("import-template")
                                    .ghost()
                                    .label("导入 JSON")
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.import_custom_template(window, cx)
                                    })),
                            )
                            .child(
                                Button::new("new-template")
                                    .primary()
                                    .label("新建模板")
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.open_template_dialog(window, cx)
                                    })),
                            ),
                    ),
            )
            .when(self.custom_templates.is_empty(), |this| {
                this.child(
                    div()
                        .h(px(110.))
                        .rounded(px(10.))
                        .border_1()
                        .border_color(rgb(BORDER))
                        .flex()
                        .items_center()
                        .justify_center()
                        .text_size(px(12.5))
                        .text_color(rgb(MUTED))
                        .child("还没有自定义模板"),
                )
            })
            .when(!self.custom_templates.is_empty(), |this| {
                this.child(custom_rows)
            });

        let columns = h_flex()
            .items_start()
            .gap_4()
            .child(
                v_flex()
                    .flex_1()
                    .min_w_0()
                    .gap_2()
                    .child(section_label("内置模板"))
                    .child(builtin_rows),
            )
            .child(v_flex().flex_1().min_w_0().gap_2().child(my_section));

        v_flex()
            .size_full()
            .min_h_0()
            .p(px(24.))
            .gap_4()
            .child(self.page_title("分流配置模板", div()))
            .child(
                div()
                    .id("templates-scroll")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scrollbar()
                    .child(columns),
            )
            .into_any_element()
    }

    fn builtin_template_row(
        &self,
        preset: Preset,
        name: &'static str,
        description: &'static str,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let active = self.options.preset == preset;
        let modules = catalog::modules_for_preset(preset);
        let rule_sets: usize = modules.iter().map(|module| module.rule_sets.len()).sum();
        h_flex()
            .h(px(66.))
            .px_4()
            .gap_3()
            .border_b_1()
            .border_color(rgb(BORDER))
            .child(
                v_flex()
                    .flex_1()
                    .min_w_0()
                    .gap_1()
                    .child(
                        div()
                            .text_size(px(13.5))
                            .font_weight(FontWeight::MEDIUM)
                            .child(name),
                    )
                    .child(
                        div()
                            .text_size(px(11.5))
                            .text_color(rgb(MUTED))
                            .truncate()
                            .child(description),
                    ),
            )
            .child(
                div()
                    .text_size(px(11.5))
                    .text_color(rgb(MUTED))
                    .child(format!("{} 模块 · {} 规则集", modules.len(), rule_sets)),
            )
            .when(active, |this| this.child(tag("当前", ACCENT)))
            .child(
                Button::new(format!("apply-builtin-{name}"))
                    .ghost()
                    .label("应用")
                    .on_click(
                        cx.listener(move |this, _, _, cx| this.apply_builtin_template(preset, cx)),
                    ),
            )
            .into_any_element()
    }

    fn qr_dialog(&self, cx: &mut Context<Self>) -> AnyElement {
        let (title, content) = self.qr_content.as_ref().expect("QR open");
        let mut matrix = v_flex().bg(gpui_kit::rgb(0xffffff)).p(px(24.));
        if let Ok(code) = qrcode::QrCode::new(content.as_bytes()) {
            let cell = 320.0 / code.width() as f32;
            for y in 0..code.width() {
                let mut row = h_flex();
                for x in 0..code.width() {
                    row = row.child(div().w(px(cell)).h(px(cell)).flex_shrink_0().bg(
                        gpui_kit::rgb(if code[(x, y)] == qrcode::Color::Dark {
                            0
                        } else {
                            0xffffff
                        }),
                    ));
                }
                matrix = matrix.child(row);
            }
        }
        modal(
            v_flex()
                .w(px(450.))
                .p_5()
                .gap_3()
                .rounded(px(12.))
                .bg(rgb(SURFACE))
                .child(div().text_lg().child(title.clone()))
                .child(div().flex().justify_center().child(matrix))
                .child(div().text_sm().text_color(rgb(MUTED)).child(
                    "节点二维码包含凭据；YAML 可导入 SwiftSub。订阅地址仅供本机客户端使用。",
                ))
                .child(
                    h_flex()
                        .gap_2()
                        .child(
                            Button::new("copy-qr")
                                .label("复制内容")
                                .on_click(cx.listener(|this, _, _, cx| this.copy_qr_content(cx))),
                        )
                        .child(
                            Button::new("export-qr")
                                .label("导出 SVG")
                                .on_click(cx.listener(|this, _, _, cx| this.export_qr(cx))),
                        )
                        .child(Button::new("close-qr").ghost().label("关闭").on_click(
                            cx.listener(|this, _, _, cx| {
                                this.qr_content = None;
                                cx.notify();
                            }),
                        )),
                ),
        )
    }

    fn desktop_settings(&self, cx: &mut Context<Self>) -> AnyElement {
        settings_section("桌面与编辑",vec![
            info_row("系统托盘",if self.desktop.available{"运行中"}else{"不可用"}.into(),false),
            h_flex().gap_3().child(Switch::new("close-to-tray").checked(self.close_to_tray).on_change(cx.listener(|this,value,_,cx|this.set_close_to_tray(*value,cx)))).child("关闭窗口时隐藏到托盘").into_any_element(),
            h_flex().gap_3().child(Switch::new("autostart").checked(self.autostart).on_change(cx.listener(|this,value,_,cx|this.set_autostart(*value,cx)))).child("开机自启（后台运行）").into_any_element(),
            div().text_sm().text_color(rgb(MUTED)).child("分流修改每两秒自动保存；未完成的文本保留供下次继续。托盘右键可退出。移动程序后请重新设置自启。").into_any_element(),
        ],div())
    }

    fn profile_settings(&self, cx: &mut Context<Self>) -> AnyElement {
        let mut rows = v_flex().gap_2();
        for profile in &self.profiles {
            let id = profile.id;
            rows = rows.child(
                h_flex()
                    .gap_2()
                    .px_3()
                    .py_1()
                    .rounded(px(8.))
                    .bg(rgb(if id == self.profile_id {
                        ACCENT_SOFT
                    } else {
                        SURFACE_2
                    }))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .child(profile.name.clone()),
                    )
                    .child(
                        Button::new(format!("profile-{id}"))
                            .ghost()
                            .disabled(id == self.profile_id)
                            .label(if id == self.profile_id {
                                "当前配置"
                            } else {
                                "切换"
                            })
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.switch_profile(id, window, cx)
                            })),
                    ),
            );
        }
        rows = rows
            .child(div().pt_2().child(field_label("当前配置名称")))
            .child(Input::new(&self.profile_name_input).cleanable(true))
            .child(
                h_flex()
                    .gap_2()
                    .flex_wrap()
                    .child(
                        Button::new("create-profile")
                            .label("复制为新配置")
                            .on_click(cx.listener(|this, _, w, cx| this.create_profile(w, cx))),
                    )
                    .child(
                        Button::new("rename-profile")
                            .ghost()
                            .label("重命名")
                            .on_click(cx.listener(|this, _, _, cx| this.rename_profile(cx))),
                    )
                    .child(
                        Button::new("delete-profile")
                            .ghost()
                            .label(if self.profile_delete_armed {
                                "确认删除"
                            } else {
                                "删除"
                            })
                            .on_click(
                                cx.listener(|this, _, w, cx| this.delete_active_profile(w, cx)),
                            ),
                    ),
            );
        rows =
            rows.child(div().text_sm().text_color(rgb(MUTED)).child(
                "规则与订阅选择独立保存；所有配置地址同时可用。订阅内容缓存与节点编辑共享。",
            ));
        rows = rows.child(div().pt_2().child(field_label("此配置包含的订阅")));
        rows = rows.child(
            h_flex()
                .gap_2()
                .flex_wrap()
                .child(
                    Button::new("profile-all-sources")
                        .ghost()
                        .label(if self.profile_membership.is_none() {
                            "✓ 全部订阅（含以后新增）"
                        } else {
                            "改为包含全部订阅（含以后新增）"
                        })
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.profile_membership = None;
                            this.refresh_profile_nodes(cx);
                        })),
                )
                .child(
                    div()
                        .text_size(px(11.))
                        .text_color(rgb(MUTED))
                        .child(format!(
                            "已包含 {} / {} 个订阅",
                            self.subscriptions
                                .iter()
                                .filter(|sub| self.profile_includes(sub.id))
                                .count(),
                            self.subscriptions.len()
                        )),
                ),
        );
        for sub in &self.subscriptions {
            let id = sub.id;
            rows = rows.child(
                h_flex()
                    .gap_2()
                    .flex_wrap()
                    .child(
                        Switch::new(format!("profile-sub-{id}"))
                            .checked(self.profile_includes(id))
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.toggle_profile_subscription(id, cx)
                            })),
                    )
                    .child(sub.name.clone()),
            );
        }
        settings_section(
            "多配置管理",
            vec![rows.into_any_element()],
            Button::new("subscription-qr")
                .ghost()
                .label("订阅二维码")
                .on_click(cx.listener(|this, _, _, cx| this.share_subscription_qr(cx))),
        )
    }

    fn settings_page(&mut self, width: Pixels, cx: &mut Context<Self>) -> AnyElement {
        let service_url = self
            .server_url
            .clone()
            .unwrap_or_else(|| "本地服务未启动".to_string());
        let token_display = if self.profile_token.len() > 12 {
            format!(
                "{}…{}",
                &self.profile_token[..6],
                &self.profile_token[self.profile_token.len() - 4..]
            )
        } else {
            self.profile_token.clone()
        };
        let service_section = settings_section(
            "本地订阅服务",
            vec![
                info_row(
                    "状态",
                    if self.server.is_some() {
                        "运行中"
                    } else {
                        "未启动"
                    }
                    .to_string(),
                    false,
                ),
                info_row("订阅地址", service_url, true),
                info_row("访问令牌", token_display, true),
                info_row("监听范围", "仅本机（127.0.0.1）".to_string(), false),
                info_row("自动刷新", "每 24 小时 / 应用运行时".to_string(), false),
                h_flex()
                    .py(px(5.))
                    .gap_4()
                    .child(
                        div()
                            .w(px(110.))
                            .text_size(px(12.))
                            .text_color(rgb(MUTED))
                            .child("订阅文件名"),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .child(Input::new(&self.subscription_filename_input).cleanable(true)),
                    )
                    .child(
                        Button::new("save-subscription-filename")
                            .ghost()
                            .label("保存")
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.save_subscription_filename(window, cx)
                            })),
                    )
                    .into_any_element(),
            ],
            h_flex().gap_2().child(
                Button::new("settings-copy-url")
                    .primary()
                    .label("复制订阅地址")
                    .on_click(cx.listener(|this, _, _, cx| this.copy_server_url(cx))),
            ),
        );
        let generation_section = settings_section(
            "生成设置",
            vec![
                v_flex()
                    .gap_2()
                    .child(field_label("MetaCubeX 规则集镜像"))
                    .child(Input::new(&self.rule_provider_input).cleanable(true))
                    .child(
                        div()
                            .text_size(px(11.5))
                            .text_color(rgb(MUTED))
                            .child("模板中还可选择 blackmatrix7、Loyalsoldier、ACL4SSR 等来源。"),
                    )
                    .child(
                        h_flex()
                            .gap_2()
                            .child(text_action(
                                "rule-source-github",
                                "GitHub",
                                ACCENT,
                                cx,
                                |this, window, cx| {
                                    this.apply_rule_provider_preset(
                                        "https://github.com/MetaCubeX/meta-rules-dat/raw/refs/heads/meta/geo",
                                        window,
                                        cx,
                                    )
                                },
                            ))
                            .child(text_action(
                                "rule-source-jsdelivr",
                                "jsDelivr",
                                ACCENT,
                                cx,
                                |this, window, cx| {
                                    this.apply_rule_provider_preset(
                                        "https://cdn.jsdelivr.net/gh/MetaCubeX/meta-rules-dat@meta/geo",
                                        window,
                                        cx,
                                    )
                                },
                            ))
                            .child(text_action(
                                "rule-source-fastly",
                                "Fastly",
                                ACCENT,
                                cx,
                                |this, window, cx| {
                                    this.apply_rule_provider_preset(
                                        "https://fastly.jsdelivr.net/gh/MetaCubeX/meta-rules-dat@meta/geo",
                                        window,
                                        cx,
                                    )
                                },
                            ))
                            .child(text_action(
                                "rule-source-testingcf",
                                "TestingCF",
                                ACCENT,
                                cx,
                                |this, window, cx| {
                                    this.apply_rule_provider_preset(
                                        "https://testingcf.jsdelivr.net/gh/MetaCubeX/meta-rules-dat@meta/geo",
                                        window,
                                        cx,
                                    )
                                },
                            ))
                            .child(div().flex_1())
                            .child(
                                Button::new("save-rule-provider")
                                    .primary()
                                    .label("保存")
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.save_rule_provider(window, cx)
                                    })),
                            ),
                    )
                    .into_any_element(),
            ],
            div(),
        );
        let data_section = settings_section(
            "数据与关于",
            vec![
                info_row("数据目录", self.data_dir.display().to_string(), true),
                info_row("本地存储", "SQLite · 仅保存在本机".to_string(), false),
                info_row("版本", format!("v{}", env!("CARGO_PKG_VERSION")), false),
                info_row("许可证", "Apache-2.0".to_string(), false),
                info_row("隐私", "无账号、无云端、无遥测".to_string(), false),
            ],
            h_flex().gap_2().child(
                Button::new("change-data-dir")
                    .ghost()
                    .label("更改目录")
                    .on_click(cx.listener(|this, _, _, cx| this.change_data_dir(cx))),
            ),
        );

        let section = match self.settings_tab {
            SettingsTab::Profiles => self.profile_settings(cx),
            SettingsTab::Service => service_section,
            SettingsTab::Generation => generation_section,
            SettingsTab::Desktop => self.desktop_settings(cx),
            SettingsTab::Data => data_section,
        };
        let compact = width < px(1100.);
        let mut navigation = div()
            .flex()
            .gap_2()
            .when(compact, |nav| nav.flex_row().flex_wrap())
            .when(!compact, |nav| nav.flex_col().w(px(160.)).flex_shrink_0());
        for (tab, label) in [
            (SettingsTab::Profiles, "配置管理"),
            (SettingsTab::Service, "本地订阅"),
            (SettingsTab::Generation, "规则与生成"),
            (SettingsTab::Desktop, "桌面与编辑"),
            (SettingsTab::Data, "数据与关于"),
        ] {
            let active = tab == self.settings_tab;
            navigation = navigation.child(
                div()
                    .id(format!("settings-tab-{tab:?}"))
                    .cursor_pointer()
                    .px_3()
                    .py_2()
                    .rounded(px(8.))
                    .border_1()
                    .border_color(rgb(if active { ACCENT } else { BORDER }))
                    .bg(rgb(if active { ACCENT_SOFT } else { SURFACE }))
                    .text_size(px(13.))
                    .text_color(rgb(if active { TEXT } else { MUTED }))
                    .hover(|s| s.bg(rgb(SURFACE_2)))
                    .child(label)
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.settings_tab = tab;
                        this.settings_scroll.set_offset(point(px(0.), px(0.)));
                        cx.notify();
                    })),
            );
        }
        let mut content = v_flex().gap_3().w_full().min_w_0();
        if let Some(error) = &self.store_error {
            content = content.child(banner(error.clone(), WARNING));
        }
        content = content.child(section);
        v_flex()
            .size_full()
            .min_h_0()
            .p(px(20.))
            .gap_3()
            .child(self.page_title("设置", div()))
            .child(
                div()
                    .flex()
                    .flex_1()
                    .min_h_0()
                    .gap_4()
                    .when(compact, |body| body.flex_col())
                    .when(!compact, |body| body.flex_row())
                    .child(navigation)
                    .child(
                        div()
                            .id("settings-scroll")
                            .flex_1()
                            .min_w_0()
                            .min_h_0()
                            .overflow_y_scroll()
                            .track_scroll(&self.settings_scroll)
                            .child(content)
                            .vertical_scrollbar(&self.settings_scroll),
                    ),
            )
            .into_any_element()
    }
}

#[derive(Clone)]
struct NodeDrag {
    key: String,
    name: String,
}

struct NodeDragPreview {
    name: String,
}

impl Render for NodeDragPreview {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
            .px_3()
            .py_2()
            .rounded(px(8.))
            .bg(rgb(SURFACE))
            .border_1()
            .border_color(rgb(ACCENT))
            .text_size(px(12.5))
            .child(self.name.clone())
    }
}

fn modal(panel: impl IntoElement) -> AnyElement {
    div()
        .absolute()
        .top_0()
        .left_0()
        .size_full()
        .occlude()
        .bg(rgba(0x05070bcc))
        .flex()
        .items_center()
        .justify_center()
        .child(panel)
        .into_any_element()
}

fn rule_set_label(id: &str) -> String {
    let label = match id {
        "category-ads-all" => "广告拦截（全）",
        "private" => "私有域名",
        "private-ip" => "私有 IP",
        "geolocation-cn" => "国内域名",
        "cn-ip" => "国内 IP",
        "geolocation-!cn" => "非中国大陆域名",
        "openai" => "OpenAI",
        "anthropic" => "Anthropic",
        "category-ai-chat-!cn" => "AI 聊天（非中国）",
        "google" => "Google",
        "google-ip" => "Google IP",
        "youtube" => "YouTube",
        "telegram" => "Telegram",
        "telegram-ip" => "Telegram IP",
        "github" => "GitHub",
        "microsoft" => "Microsoft",
        "apple" => "Apple",
        "netflix" => "Netflix",
        "netflix-ip" => "Netflix IP",
        "spotify" => "Spotify",
        "discord" => "Discord",
        "twitter" => "X (Twitter)",
        "disney" => "Disney+",
        "tiktok" => "TikTok",
        "facebook" => "Facebook",
        "instagram" => "Instagram",
        "steam" => "Steam",
        "steam-ip" => "Steam IP",
        "paypal" => "PayPal",
        _ => return id.to_string(),
    };
    label.to_string()
}

fn rule_provider_source_label(base: &str) -> &'static str {
    if base.contains("jsdelivr") {
        "jsDelivr"
    } else if base.contains("github.com") {
        "GitHub"
    } else {
        "自定义"
    }
}

fn icon_action(
    id: impl Into<String>,
    icon: &'static str,
    color: u32,
    cx: &mut Context<SwiftSubApp>,
    handler: impl Fn(&mut SwiftSubApp, &mut Window, &mut Context<SwiftSubApp>) + 'static,
) -> AnyElement {
    let id: String = id.into();
    div()
        .id(id)
        .cursor_pointer()
        .w(px(26.))
        .h(px(26.))
        .rounded(px(6.))
        .flex()
        .items_center()
        .justify_center()
        .text_size(px(16.))
        .text_color(rgb(color))
        .hover(|style| style.bg(rgb(SURFACE_2)))
        .child(icon)
        .on_click(cx.listener(move |this, _, window, cx| handler(this, window, cx)))
        .into_any_element()
}

fn text_action(
    id: impl Into<String>,
    label: &'static str,
    color: u32,
    cx: &mut Context<SwiftSubApp>,
    handler: impl Fn(&mut SwiftSubApp, &mut Window, &mut Context<SwiftSubApp>) + 'static,
) -> AnyElement {
    let id: String = id.into();
    div()
        .id(id)
        .cursor_pointer()
        .h(px(24.))
        .px(px(8.))
        .rounded(px(6.))
        .flex()
        .items_center()
        .text_size(px(12.))
        .text_color(rgb(color))
        .hover(|style| style.bg(rgb(SURFACE_2)))
        .child(label)
        .on_click(cx.listener(move |this, _, window, cx| handler(this, window, cx)))
        .into_any_element()
}

fn tag(label: &str, color: u32) -> Div {
    div()
        .px(px(7.))
        .py(px(2.))
        .rounded(px(4.))
        .bg(rgb(SURFACE_2))
        .text_size(px(11.))
        .text_color(rgb(color))
        .child(label.to_string())
}

fn static_chip(label: &'static str) -> AnyElement {
    div()
        .h(px(30.))
        .px(px(10.))
        .rounded(px(7.))
        .border_1()
        .border_color(rgb(BORDER))
        .bg(rgb(SURFACE))
        .flex()
        .items_center()
        .text_size(px(12.))
        .text_color(rgb(MUTED))
        .child(label)
        .into_any_element()
}

fn section_label(label: &'static str) -> Div {
    div()
        .text_size(px(12.5))
        .font_weight(FontWeight::MEDIUM)
        .text_color(rgb(MUTED))
        .child(label)
}

fn field_label(label: &'static str) -> Div {
    div().text_size(px(12.)).text_color(rgb(MUTED)).child(label)
}

fn stat_chip(label: &'static str, value: usize) -> AnyElement {
    h_flex()
        .gap_1()
        .h(px(26.))
        .px(px(8.))
        .rounded(px(6.))
        .bg(rgb(SURFACE))
        .border_1()
        .border_color(rgb(BORDER))
        .child(
            div()
                .text_size(px(12.5))
                .font_weight(FontWeight::SEMIBOLD)
                .child(value.to_string()),
        )
        .child(div().text_size(px(11.)).text_color(rgb(MUTED)).child(label))
        .into_any_element()
}

fn banner(message: String, color: u32) -> AnyElement {
    h_flex()
        .px_3()
        .py_2()
        .gap_2()
        .rounded(px(8.))
        .bg(rgb(if color == DANGER {
            DANGER_SOFT
        } else {
            0x2a2110
        }))
        .border_1()
        .border_color(rgb(if color == DANGER { 0x5b2328 } else { 0x5b4316 }))
        .child(
            div()
                .text_size(px(12.))
                .text_color(rgb(color))
                .truncate()
                .child(message),
        )
        .into_any_element()
}

fn settings_section(
    title: &'static str,
    rows: Vec<AnyElement>,
    actions: impl IntoElement,
) -> AnyElement {
    let mut panel = v_flex()
        .rounded(px(12.))
        .bg(rgb(SURFACE))
        .border_1()
        .border_color(rgb(BORDER));

    panel = panel.child(
        h_flex()
            .px(px(18.))
            .py(px(14.))
            .justify_between()
            .border_b_1()
            .border_color(rgb(BORDER))
            .child(
                div()
                    .text_size(px(14.))
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(title),
            )
            .child(actions),
    );

    let mut body = v_flex().p(px(18.)).gap(px(6.));
    for row in rows {
        body = body.child(row);
    }
    panel.child(body).into_any_element()
}

fn info_row(label: &'static str, value: String, mono: bool) -> AnyElement {
    h_flex()
        .py(px(5.))
        .gap_4()
        .child(
            div()
                .w(px(110.))
                .text_size(px(12.))
                .text_color(rgb(MUTED))
                .child(label),
        )
        .child(
            div()
                .flex_1()
                .min_w_0()
                .when(mono, |this| this.font_family("Consolas"))
                .text_size(px(if mono { 11.5 } else { 12.5 }))
                .text_color(rgb(TEXT))
                .truncate()
                .child(value),
        )
        .into_any_element()
}

#[derive(Clone)]
struct RuleDrag {
    index: usize,
    name: String,
}
#[derive(Clone)]
struct MemberDrag {
    group: usize,
    member: usize,
    name: String,
}
