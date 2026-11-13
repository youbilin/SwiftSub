#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod desktop;
mod rule_catalog;
mod ui;

use app::SwiftSubApp;
use gpui_kit::component::{Theme, ThemeMode, TitleBar};
use gpui_kit::*;

fn main() {
    application().with_assets(assets::Assets).run(|cx| {
        init(cx);
        cx.set_window_appearance(Some(WindowAppearance::Dark));
        Theme::change(ThemeMode::Dark, None, cx);
        configure_theme(cx);

        let bounds = Bounds::centered(None, size(px(1180.0), px(760.0)), cx);
        let mut options = TitleBar::window_options();
        options.window_bounds = Some(WindowBounds::Windowed(bounds));
        options.window_min_size = Some(size(px(1024.0), px(680.0)));
        options.titlebar = Some(TitlebarOptions {
            title: Some("SwiftSub".into()),
            appears_transparent: true,
            ..Default::default()
        });

        open_window(options, cx, |window, cx| {
            cx.new(|cx| SwiftSubApp::new(window, cx))
        })
        .expect("failed to open SwiftSub window");
    });
}

pub(crate) fn configure_theme(cx: &mut App) {
    Theme::update(cx, |theme| {
        theme.font_family = "Microsoft YaHei UI".into();
        theme.font_size = px(14.0);
        theme.mono_font_family = "Consolas".into();
        theme.mono_font_size = px(13.0);
        theme.primary = Hsla::from(rgb(0x5b8cff));
        theme.primary_foreground = Hsla::from(rgb(0xffffff));
        theme.primary_hover = Hsla::from(rgb(0x4a7bf0));
        theme.primary_active = Hsla::from(rgb(0x3d6de0));
        theme.button_primary = Hsla::from(rgb(0x5b8cff));
        theme.button_primary_foreground = Hsla::from(rgb(0xffffff));
        theme.button_primary_hover = Hsla::from(rgb(0x4a7bf0));
        theme.button_primary_active = Hsla::from(rgb(0x3d6de0));
        theme.tokens.primary = Hsla::from(rgb(0x5b8cff)).into();
    });
}
