pub mod icons;
mod layout;
mod phase2;
mod saved_configs;
pub mod theme;
mod workspace;

use gpui_kit::base::Disableable as _;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::input::{Input, Textarea};
use gpui_kit::component::scroll::ScrollableElement as _;
use gpui_kit::component::switch::Switch;
use gpui_kit::component::{Sizable as _, StyledExt as _, TitleBar, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use swiftsub_core::catalog::{self, ModuleKind, Preset};
use swiftsub_core::model::{GroupKind, Node};
use swiftsub_core::parser::{looks_like_clash_yaml, parse_content};
use swiftsub_core::region::Region;

use crate::app::{
    ChainSide, CustomDialog, Page, SettingsTab, SwiftSubApp, TemplateTab, format_timestamp,
};
use crate::rule_catalog;
use icons::{close_icon, delete_icon, edit_icon, plus_icon};
use layout::{danger_action, object_action, object_row};
use std::collections::HashSet;
pub use theme::{AppThemeMode, set_light_theme};
use theme::{rgb, rgba, tokens::*};

impl SwiftSubApp {
    fn has_blocking_modal(&self) -> bool {
        self.p1.has_modal()
            || self.add_open
            || self.node_rename_key.is_some()
            || self.custom_dialog != CustomDialog::None
            || self.generated_save_open
            || self.workspace.rule_picker_open
            || self.workspace.rule_content_id.is_some()
            || self.workspace.confirm_delete_config.is_some()
            || (self.workspace.saved_editor_open
                && (self.saved_editor.editing_node
                    || self.saved_editor.node_import
                    || self.saved_editor.picking_rules
                    || self.saved_editor.more
                    || self.saved_editor.copy_open))
    }
}

impl Render for SwiftSubApp {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if ((self.page == Page::Generator && !self.workspace.saved_editor_open)
            || (self.page == Page::Settings
                && matches!(
                    self.settings_tab,
                    SettingsTab::Generation | SettingsTab::RuleSources | SettingsTab::Templates
                )))
            && !self.template_dialog_open
        {
            self.open_template_dialog(window, cx);
        }
        if self.template_dialog_open
            && self.page == Page::Settings
            && self.settings_tab == SettingsTab::Generation
            && !self.workspace.settings_loaded
        {
            self.workspace_load_settings(window, cx);
        }
        div()
            .id("swiftsub-root")
            .on_mouse_move(cx.listener(|this, event: &MouseMoveEvent, window, cx| {
                if this.workspace.resizing_sidebar {
                    let max_w =
                        (f32::from(window.viewport_size().width) * 0.42).clamp(280.0, 440.0);
                    let next = (f32::from(event.position.x) - 12.0).clamp(260.0, max_w);
                    if (next - this.workspace.sidebar_width).abs() > 0.5 {
                        this.workspace.sidebar_width = next;
                        cx.notify();
                    }
                }
            }))
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|this, _, _, cx| {
                    if this.workspace.resizing_sidebar {
                        this.workspace.resizing_sidebar = false;
                        cx.notify();
                    }
                }),
            )
            .capture_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
                if this.add_open
                    && event.keystroke.modifiers.control
                    && event.keystroke.key == "enter"
                {
                    this.submit_add(window, cx);
                    cx.stop_propagation();
                } else if event.keystroke.modifiers.control
                    && event.keystroke.key == "s"
                    && !this.has_blocking_modal()
                {
                    match this.page {
                        Page::Generator => this.request_save_generated(window, cx),
                        Page::SavedConfigs if this.workspace.saved_editor_open => {
                            this.save_yaml_editor(false, window, cx);
                        }
                        Page::Settings
                            if this.settings_tab == SettingsTab::Templates
                                && this.workspace.template_editor.open =>
                        {
                            this.save_template_editor(cx);
                        }
                        _ => return,
                    }
                    cx.stop_propagation();
                } else if event.keystroke.modifiers.control
                    && matches!(event.keystroke.key.as_str(), "z" | "y")
                    && this.page == Page::Generator
                    && this.workspace.saved_editor_open
                    && !this.has_blocking_modal()
                    && !this.saved_editor.editing_group
                    && !this
                        .saved_config_name
                        .read(cx)
                        .focus_handle(cx)
                        .is_focused(window)
                {
                    this.saved_undo(
                        event.keystroke.key == "y" || event.keystroke.modifiers.shift,
                        window,
                        cx,
                    );
                    cx.stop_propagation();
                } else if event.keystroke.modifiers.control
                    && event.keystroke.key == "z"
                    && !this.workspace.saved_editor_open
                    && this.page == Page::Generator
                    && !this.has_blocking_modal()
                    && ![
                        this.template_custom_rules.read(cx).focus_handle(cx),
                        this.template_policy_rename.read(cx).focus_handle(cx),
                        this.template_policy_name.read(cx).focus_handle(cx),
                        this.chain_name.read(cx).focus_handle(cx),
                        this.workspace.description.read(cx).focus_handle(cx),
                        this.template_member_search.read(cx).focus_handle(cx),
                        this.search.read(cx).focus_handle(cx),
                    ]
                    .iter()
                    .any(|handle| handle.is_focused(window))
                {
                    this.workspace_undo(window, cx);
                    cx.stop_propagation();
                } else if event.keystroke.modifiers.control && event.keystroke.key == "f" {
                    if this.workspace.saved_editor_open && this.saved_editor.more {
                        this.saved_config_yaml
                            .read(cx)
                            .focus_handle(cx)
                            .focus(window, cx);
                        this.saved_config_yaml
                            .update(cx, |input, cx| input.open_search(false, cx));
                        cx.stop_propagation();
                        return;
                    }
                    if this.workspace.rule_content_id.is_some() {
                        if this.workspace.rule_content_editing {
                            this.workspace
                                .rule_content
                                .read(cx)
                                .focus_handle(cx)
                                .focus(window, cx);
                            this.workspace
                                .rule_content
                                .update(cx, |input, cx| input.open_search(false, cx));
                        } else {
                            let handle =
                                this.workspace.rule_content_search.read(cx).focus_handle(cx);
                            window.focus(&handle, cx);
                        }
                        cx.stop_propagation();
                        return;
                    }
                    if this.workspace.rule_picker_open {
                        let handle = this.template_rule_search.read(cx).focus_handle(cx);
                        window.focus(&handle, cx);
                        cx.stop_propagation();
                        return;
                    }
                    if this.page == Page::Generator && !this.workspace.saved_editor_open {
                        let input = if this.template_policy_edit.is_some()
                            && this.nodes.len() + this.chains.len() > 12
                        {
                            &this.template_member_search
                        } else {
                            &this.search
                        };
                        let handle = input.read(cx).focus_handle(cx);
                        window.focus(&handle, cx);
                        cx.stop_propagation();
                    }
                } else if event.keystroke.key == "escape" {
                    if this.workspace.rule_content_id.is_some()
                        && this.workspace.rule_content.read(cx).search_session().open
                    {
                        this.workspace
                            .rule_content
                            .update(cx, |input, cx| input.close_search(cx));
                        cx.stop_propagation();
                        return;
                    }
                    if this.workspace.saved_editor_open
                        && this.saved_editor.more
                        && this.saved_config_yaml.read(cx).search_session().open
                    {
                        this.saved_config_yaml
                            .update(cx, |input, cx| input.close_search(cx));
                        cx.stop_propagation();
                        return;
                    }
                    if this.p1.qr_modal.take().is_some()
                        || this.p1.history_modal.take().is_some()
                        || this.p1.backup_preview.take().is_some()
                    {
                        cx.notify();
                        cx.stop_propagation();
                    } else if this.p1.batch_rename_open {
                        this.p1.batch_rename_open = false;
                        cx.notify();
                        cx.stop_propagation();
                    } else if this.p1.sub_logs_open {
                        this.p1.sub_logs_open = false;
                        cx.notify();
                        cx.stop_propagation();
                    } else if this.workspace.confirm_delete_config.take().is_some() {
                        cx.notify();
                        cx.stop_propagation();
                    } else if this.workspace.rule_content_id.is_some() {
                        this.workspace_close_rule_content(cx);
                        cx.stop_propagation();
                    } else if this.workspace.rule_picker_open {
                        this.workspace.rule_picker_open = false;
                        cx.notify();
                        cx.stop_propagation();
                    } else if this.add_open {
                        this.close_add_dialog(window, cx);
                        cx.stop_propagation();
                    } else if this.node_rename_key.is_some() {
                        this.close_node_rename(window, cx);
                        cx.stop_propagation();
                    } else if this.custom_dialog != CustomDialog::None {
                        this.close_custom_dialog(window, cx);
                        cx.stop_propagation();
                    } else if this.generated_save_open {
                        this.cancel_generated_name(window, cx);
                        cx.stop_propagation();
                    } else if this.workspace.saved_editor_open && this.saved_editor.copy_open {
                        this.saved_editor.copy_open = false;
                        cx.notify();
                        cx.stop_propagation();
                    } else if this.workspace.saved_editor_open && this.saved_editor.editing_node {
                        if let Err(error) = this.flush_saved_fields(window, cx) {
                            this.saved_config_error = Some(error);
                        } else {
                            this.saved_editor.editing_node = false;
                        }
                        cx.notify();
                        cx.stop_propagation();
                    } else if this.workspace.saved_editor_open && this.saved_editor.node_import {
                        this.saved_editor.node_import = false;
                        cx.notify();
                        cx.stop_propagation();
                    } else if this.workspace.saved_editor_open && this.saved_editor.picking_rules {
                        this.saved_editor.picking_rules = false;
                        cx.notify();
                        cx.stop_propagation();
                    } else if this.workspace.saved_editor_open && this.saved_editor.more {
                        this.set_saved_tab(crate::app::SavedEditorTab::Groups, window, cx);
                        if this.saved_config_error.is_none() {
                            this.saved_editor.more = false;
                        }
                        cx.notify();
                        cx.stop_propagation();
                    } else if this.workspace.group_settings_open {
                        this.workspace.group_settings_open = false;
                        cx.notify();
                        cx.stop_propagation();
                    } else if this.page == Page::Settings
                        && this.settings_tab == SettingsTab::Templates
                        && this.workspace.template_editor.open
                    {
                        if this.template_editor_handle_escape(cx) {
                            cx.stop_propagation();
                        }
                    } else if this.page == Page::Generator && !this.workspace.saved_editor_open {
                        if this.finish_workspace_fields(window, cx) {
                            this.template_policy_edit = None;
                            this.workspace.save_template_open = false;
                        }
                        cx.notify();
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
            .when(self.custom_dialog == CustomDialog::Filter, |this| {
                this.child(self.filter_dialog(cx))
            })
            .when(
                self.generated_save_open && self.page == Page::Generator,
                |this| this.child(self.generated_name_dialog(cx)),
            )
            .when(
                self.workspace.rule_picker_open && self.page == Page::Generator,
                |this| this.child(self.workspace_rule_picker_dialog(cx)),
            )
            .when(self.node_rename_key.is_some(), |this| {
                this.child(self.node_rename_dialog(cx))
            })
            .when(self.workspace.rule_content_id.is_some(), |this| {
                this.child(self.workspace_rule_content_dialog(cx))
            })
            .when_some(
                self.workspace.confirm_delete_config.clone(),
                |this, config_id| this.child(self.confirm_delete_config_dialog(config_id, cx)),
            )
            .children(self.render_p1_modals(cx))
            .when_some(self.toast.clone(), |this, toast| {
                this.child(
                    div()
                        .absolute()
                        .bottom(px(18.))
                        .right(px(18.))
                        .occlude()
                        .px_3()
                        .py_2()
                        .rounded(px(8.))
                        .bg(rgb(SURFACE_3))
                        .border_1()
                        .border_color(rgb(BORDER_STRONG))
                        .text_size(px(13.))
                        .text_color(rgb(TEXT))
                        .child(toast),
                )
            })
    }
}

impl SwiftSubApp {
    fn page_view(&mut self, width: Pixels, cx: &mut Context<Self>) -> AnyElement {
        match self.page {
            Page::Subscriptions | Page::Generator => self.clean_generator_page(width, cx),
            Page::Templates => self.clean_template_settings(cx),
            Page::SavedConfigs => self.clean_config_library(width, cx),
            Page::Settings => self.settings_page(width, cx),
        }
    }

    fn header(&self) -> TitleBar {
        let logo = gpui_kit::component::Icon::default().data(
            br#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.3" stroke-linecap="round" stroke-linejoin="round"><path d="M16 6H9.5a3 3 0 0 0 0 6h5a3 3 0 0 1 0 6H8"/><circle cx="17.5" cy="6" r="1.6" fill="currentColor" stroke="none"/><circle cx="6.5" cy="18" r="1.6" fill="currentColor" stroke="none"/></svg>"#,
        );
        TitleBar::new().child(
            h_flex()
                .h_full()
                .w_full()
                .gap_2()
                .child(
                    div()
                        .w(px(20.))
                        .h(px(20.))
                        .rounded(px(5.))
                        .bg(rgb(ACCENT))
                        .text_color(gpui_kit::rgb(0xffffff))
                        .flex()
                        .items_center()
                        .justify_center()
                        .child(logo.with_size(px(14.))),
                )
                .child(
                    div()
                        .text_size(px(14.))
                        .font_weight(FontWeight::SEMIBOLD)
                        .child("SwiftSub"),
                )
                .child(micro_badge(
                    format!("v{}", env!("CARGO_PKG_VERSION")),
                    false,
                )),
        )
    }

    fn nav_bar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        h_flex()
            .h(px(52.))
            .px_4()
            .gap_3()
            .bg(rgb(BG))
            .border_b_1()
            .border_color(rgb(BORDER))
            .child(self.nav_item("生成", Page::Generator, cx))
            .child(self.nav_item("配置", Page::SavedConfigs, cx))
            .child(self.nav_item("设置", Page::Settings, cx))
    }

    fn nav_item(&self, label: &'static str, page: Page, cx: &mut Context<Self>) -> AnyElement {
        let active = self.page == page
            || (page == Page::Generator
                && matches!(self.page, Page::Subscriptions | Page::Templates));
        div()
            .id(format!("nav-{label}"))
            .cursor_pointer()
            .h(px(36.))
            .px(px(18.))
            .rounded(px(8.))
            .flex()
            .items_center()
            .border_1()
            .border_color(rgb(if active { ACCENT } else { BORDER }))
            .bg(rgb(if active { ACCENT_SOFT } else { SURFACE }))
            .text_size(px(14.))
            .font_weight(if active {
                FontWeight::SEMIBOLD
            } else {
                FontWeight::MEDIUM
            })
            .text_color(rgb(if active { TEXT } else { MUTED }))
            .hover(|style| style.bg(rgb(SURFACE_2)).text_color(rgb(TEXT)))
            .child(label)
            .on_click(cx.listener(move |this, _, window, cx| this.set_page(page, window, cx)))
            .into_any_element()
    }

    fn page_title(&self, title: &'static str, actions: impl IntoElement) -> Div {
        h_flex()
            .min_w_0()
            .flex_wrap()
            .justify_between()
            .gap_4()
            .child(
                div()
                    .text_size(px(20.))
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(title),
            )
            .child(actions)
    }

    fn add_dialog(&self, cx: &mut Context<Self>) -> AnyElement {
        let input = self.add_url.read(cx).value().trim().to_string();
        let detection = if input.is_empty() {
            "自动识别订阅地址或节点链接 · Ctrl+Enter 添加".into()
        } else if input.lines().count() == 1
            && (input.starts_with("https://") || input.starts_with("http://"))
        {
            "订阅地址".into()
        } else {
            match parse_content(&input) {
                Ok(parsed) => format!(
                    "识别 {} 个节点{}",
                    parsed.proxies.len(),
                    if looks_like_clash_yaml(&input) {
                        " · 仅提取节点，分流配置保留"
                    } else {
                        ""
                    }
                ),
                Err(_) => "尚未识别，请检查输入内容".into(),
            }
        };

        let panel =
            v_flex()
                .w(px(540.))
                .max_w(relative(0.92))
                .max_h(relative(0.88))
                .min_h_0()
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
                                .child("添加订阅或节点"),
                        )
                        .child(object_action(
                            "add-dialog-close",
                            close_icon(),
                            "关闭",
                            cx,
                            |this, window, cx| this.close_add_dialog(window, cx),
                        )),
                )
                .child(Textarea::new(&self.add_url).h(px(150.)))
                .child(
                    div()
                        .text_size(px(11.5))
                        .text_color(rgb(MUTED))
                        .child(detection),
                )
                .child(Input::new(&self.add_name).cleanable(true))
                .child(
                    h_flex()
                        .gap_2()
                        .child(text_action(
                            "import-clipboard",
                            "粘贴剪贴板",
                            ACCENT,
                            cx,
                            |this, window, cx| this.paste_add_input(window, cx),
                        ))
                        .child(text_action(
                            "import-file",
                            "从 YAML 提取节点",
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
                .child(
                    h_flex()
                        .gap_2()
                        .justify_end()
                        .child(Button::new("add-cancel").ghost().label("取消").on_click(
                            cx.listener(|this, _, window, cx| this.close_add_dialog(window, cx)),
                        ))
                        .child(Button::new("add-confirm").primary().label("添加").on_click(
                            cx.listener(|this, _, window, cx| this.submit_add(window, cx)),
                        )),
                );
        modal_with_backdrop("add-dialog-backdrop", panel, cx, |this, window, cx| {
            this.close_add_dialog(window, cx)
        })
    }

    fn node_rename_dialog(&self, cx: &mut Context<Self>) -> AnyElement {
        let panel =
            v_flex()
                .w(px(620.))
                .max_w(relative(0.92))
                .max_h(relative(0.88))
                .min_h_0()
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
                                .child("编辑节点"),
                        )
                        .child(object_action(
                            "node-edit-close",
                            close_icon(),
                            "关闭",
                            cx,
                            |this, window, cx| this.close_node_rename(window, cx),
                        )),
                )
                .child(
                    div()
                        .id("node-edit-body")
                        .flex_1()
                        .min_h_0()
                        .overflow_y_scrollbar()
                        .child(
                            v_flex()
                                .gap_3()
                                .child(field_label("名称"))
                                .child(Input::new(&self.node_rename_input).cleanable(true))
                                .child(field_label("节点 YAML"))
                                .child(Textarea::new(&self.node_edit_textarea).h(px(260.)))
                                .child(div().text_size(px(12.)).text_color(rgb(MUTED)).child(
                                    "可直接修改协议、服务器、端口、密码等字段，保存后立即生效。",
                                )),
                        ),
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
        modal_with_backdrop("node-edit-backdrop", panel, cx, |this, window, cx| {
            this.close_node_rename(window, cx)
        })
    }

    fn confirm_delete_config_dialog(&self, id: String, cx: &mut Context<Self>) -> AnyElement {
        let name = self
            .saved_configs
            .iter()
            .find(|config| config.id == id)
            .map(|config| config.name.clone())
            .unwrap_or_else(|| "此配置".into());
        let panel = v_flex()
            .w(px(400.))
            .max_w(relative(0.92))
            .p_4()
            .gap_3()
            .rounded(px(12.))
            .bg(rgb(SURFACE))
            .border_1()
            .border_color(rgb(BORDER))
            .child(
                div()
                    .text_size(px(16.))
                    .font_weight(FontWeight::SEMIBOLD)
                    .child("删除配置"),
            )
            .child(
                div()
                    .text_size(px(13.))
                    .text_color(rgb(MUTED))
                    .child(format!(
                        "确定删除「{name}」吗？对应的本地 YAML 文件和订阅地址会同时失效。"
                    )),
            )
            .child(
                h_flex()
                    .gap_2()
                    .justify_end()
                    .child(
                        Button::new("confirm-delete-config-cancel")
                            .ghost()
                            .label("取消")
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.workspace.confirm_delete_config = None;
                                cx.notify();
                            })),
                    )
                    .child(
                        Button::new("confirm-delete-config-ok")
                            .danger()
                            .label("删除")
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.workspace.confirm_delete_config = None;
                                this.delete_library_config(id.clone(), cx);
                            })),
                    ),
            );
        modal_with_backdrop("delete-config-backdrop", panel, cx, |this, _, cx| {
            this.workspace.confirm_delete_config = None;
            cx.notify();
        })
    }

    fn template_rule_source_choices(&self, cx: &mut Context<Self>) -> AnyElement {
        let mut options = vec!["全部来源".into(), "内置规则".into(), "自定义链接".into()];
        options.extend(rule_catalog::SOURCES.iter().map(|s| s.name.to_string()));
        let label = match self.workspace.rule_source.as_str() {
            "builtin" => "内置规则",
            "url" => "自定义链接",
            id => rule_catalog::SOURCES
                .iter()
                .find(|s| s.id == id)
                .map(|s| s.name)
                .unwrap_or("全部来源"),
        };
        self.saved_choice(
            "workspace-rule-source",
            label.into(),
            options,
            cx,
            |this, label, _, cx| {
                this.workspace.rule_source = match label.as_str() {
                    "内置规则" => "builtin".into(),
                    "自定义链接" => "url".into(),
                    name => rule_catalog::SOURCES
                        .iter()
                        .find(|s| s.name == name)
                        .map(|s| s.id.to_string())
                        .unwrap_or_default(),
                };
                this.template_rule_sources_collapsed
                    .remove(&this.workspace.rule_source);
                cx.notify();
            },
        )
    }

    fn region_chip(&self, region: Region, selected: bool, cx: &mut Context<Self>) -> AnyElement {
        div()
            .id(format!("region-{:?}", region))
            .cursor_pointer()
            .h(px(32.))
            .px(px(12.))
            .rounded(px(6.))
            .border_1()
            .flex()
            .items_center()
            .text_size(px(13.))
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

    fn filter_dialog(&self, cx: &mut Context<Self>) -> AnyElement {
        let mut regions = div().flex().flex_wrap().gap_2();
        for region in Region::ALL {
            regions =
                regions.child(self.region_chip(region, self.filter_regions.contains(&region), cx));
        }

        let panel =
            v_flex()
                .w(px(620.))
                .max_w(relative(0.92))
                .max_h(relative(0.88))
                .min_h_0()
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
                                .child(if self.workspace.edit_filter.is_some() {
                                    "编辑筛选代理组"
                                } else {
                                    "新建筛选代理组"
                                }),
                        )
                        .child(object_action(
                            "filter-dialog-close",
                            close_icon(),
                            "关闭",
                            cx,
                            |this, window, cx| this.close_custom_dialog(window, cx),
                        )),
                )
                .child(
                    div()
                        .id("filter-dialog-body")
                        .flex_1()
                        .min_h_0()
                        .overflow_y_scrollbar()
                        .child(
                            v_flex()
                                .gap_3()
                                .child(field_label("名称（可选）"))
                                .child(Input::new(&self.filter_name).cleanable(true))
                                .child(field_label("地区（可多选）"))
                                .child(regions)
                                .child(field_label("包含关键词"))
                                .child(Input::new(&self.filter_include).cleanable(true))
                                .child(field_label("排除关键词"))
                                .child(Input::new(&self.filter_exclude).cleanable(true))
                                .child(field_label("来源（可选，名称或 ID，逗号分隔）"))
                                .child(Input::new(&self.workspace.filter_sources).cleanable(true)),
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
                        .child(Button::new("filter-cancel").ghost().label("取消").on_click(
                            cx.listener(|this, _, window, cx| this.close_custom_dialog(window, cx)),
                        ))
                        .child(
                            Button::new("filter-add")
                                .primary()
                                .label(if self.workspace.edit_filter.is_some() {
                                    "保存修改"
                                } else {
                                    "添加筛选组"
                                })
                                .on_click(cx.listener(|this, _, window, cx| {
                                    this.add_filter_group(window, cx)
                                })),
                        ),
                );
        modal_with_backdrop("filter-dialog-backdrop", panel, cx, |this, window, cx| {
            this.close_custom_dialog(window, cx)
        })
    }

    fn settings_page(&mut self, _width: Pixels, cx: &mut Context<Self>) -> AnyElement {
        let section = match self.settings_tab {
            SettingsTab::Data => self.config_subscription_settings(cx),
            SettingsTab::Generation => self.workspace_settings_panel(cx),
            SettingsTab::RuleSources => self.generation_rule_source_settings(cx),
            SettingsTab::Templates => self.clean_template_settings(cx),
        };
        let mut navigation = h_flex().w_full().gap_2();
        for (tab, label) in [
            (SettingsTab::Generation, "基础与DNS"),
            (SettingsTab::RuleSources, "规则来源"),
            (SettingsTab::Templates, "模板"),
            (SettingsTab::Data, "应用与更新"),
        ] {
            let active = tab == self.settings_tab;
            navigation = navigation.child(
                Button::new(format!("settings-tab-{tab:?}"))
                    .outline()
                    .label(label)
                    .when(active, |button| button.primary())
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.settings_tab = tab;
                        this.workspace.settings_error_section = None;
                        cx.notify();
                    })),
            );
        }
        navigation = navigation.child(div().flex_1()).child(
            Button::new("settings-theme")
                .ghost()
                .icon(if self.light_theme {
                    gpui_kit::component::IconName::Moon
                } else {
                    gpui_kit::component::IconName::Sun
                })
                .tooltip(if self.light_theme {
                    "切换深色外观"
                } else {
                    "切换浅色外观"
                })
                .accessibility_label("切换外观")
                .on_click(cx.listener(|this, _, _, cx| this.set_theme(!this.light_theme, cx))),
        );
        v_flex()
            .size_full()
            .min_h_0()
            .p_4()
            .gap_3()
            .child(navigation)
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .min_h_0()
                    .rounded(px(12.))
                    .border_1()
                    .border_color(rgb(BORDER))
                    .bg(rgb(SURFACE))
                    .overflow_hidden()
                    .child(
                        v_flex()
                            .id(format!("settings-scroll-{:?}", self.settings_tab))
                            .size_full()
                            .min_w_0()
                            .p_4()
                            .gap_4()
                            .overflow_y_scrollbar()
                            .when_some(self.store_error.clone(), |panel, error| {
                                panel.child(banner(error, WARNING))
                            })
                            .child(section),
                    ),
            )
            .into_any_element()
    }
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

fn modal_with_backdrop(
    id: impl Into<ElementId>,
    panel: Div,
    cx: &mut Context<SwiftSubApp>,
    on_close: impl Fn(&mut SwiftSubApp, &mut Window, &mut Context<SwiftSubApp>) + 'static,
) -> AnyElement {
    div()
        .id(id)
        .absolute()
        .top_0()
        .left_0()
        .size_full()
        .occlude()
        .bg(rgba(0x05070bcc))
        .flex()
        .items_center()
        .justify_center()
        .on_click(cx.listener(move |this, _, window, cx| on_close(this, window, cx)))
        .child(panel.on_mouse_down(MouseButton::Left, |_, _, cx| {
            cx.stop_propagation();
        }))
        .into_any_element()
}

pub(crate) fn rule_set_label(id: &str) -> String {
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
        .h(px(32.))
        .px(px(12.))
        .rounded(px(6.))
        .flex()
        .items_center()
        .text_size(px(13.))
        .text_color(rgb(color))
        .hover(|style| style.bg(rgb(SURFACE_2)))
        .child(label)
        .on_click(cx.listener(move |this, _, window, cx| handler(this, window, cx)))
        .into_any_element()
}

fn micro_badge(label: impl Into<String>, accent: bool) -> Div {
    div()
        .flex_shrink_0()
        .px(px(6.))
        .py(px(1.))
        .rounded(px(4.))
        .bg(rgb(if accent { ACCENT_SOFT } else { SURFACE_2 }))
        .text_size(px(10.5))
        .text_color(rgb(if accent { ACCENT } else { MUTED }))
        .child(label.into())
}

fn field_label(label: &'static str) -> Div {
    div().text_size(px(12.)).text_color(rgb(MUTED)).child(label)
}

fn banner(message: String, color: u32) -> AnyElement {
    h_flex()
        .min_w_0()
        .flex_shrink_0()
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
                .flex_1()
                .min_w_0()
                .text_size(px(12.))
                .text_color(rgb(color))
                .child(message),
        )
        .into_any_element()
}

fn mode_tabs(
    id: &str,
    selected: &str,
    values: &[&str],
    cx: &mut Context<SwiftSubApp>,
    handler: impl Fn(&mut SwiftSubApp, String, &mut Window, &mut Context<SwiftSubApp>) + 'static,
) -> AnyElement {
    let handler = std::rc::Rc::new(handler);
    let mut tabs = h_flex().gap_1().flex_wrap().min_w_0();
    for value in values {
        let active = *value == selected;
        let value = value.to_string();
        let handler = handler.clone();
        tabs = tabs.child(
            Button::new(format!("{id}-{value}"))
                .ghost()
                .label(value.clone())
                .when(active, |button| button.primary())
                .on_click(cx.listener(move |this, _, window, cx| {
                    handler(this, value.clone(), window, cx);
                })),
        );
    }
    tabs.into_any_element()
}

fn settings_section(
    title: &'static str,
    rows: Vec<AnyElement>,
    actions: impl IntoElement,
) -> AnyElement {
    let mut panel = v_flex().w_full().min_w_0().bg(rgb(SURFACE)).pb_3();

    panel = panel.child(
        h_flex()
            .min_w_0()
            .gap_2()
            .flex_wrap()
            .px_4()
            .py_3()
            .justify_between()
            .border_b_1()
            .border_color(rgb(BORDER))
            .child(
                div()
                    .text_size(px(14.5))
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(title),
            )
            .child(actions),
    );

    let mut body = v_flex().min_w_0().p_4().gap_3();
    for row in rows {
        body = body.child(row);
    }
    panel.child(body).into_any_element()
}

fn info_row(label: &'static str, value: String, mono: bool) -> AnyElement {
    h_flex()
        .py(px(4.))
        .gap_4()
        .child(
            div()
                .w(px(110.))
                .flex_shrink_0()
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
