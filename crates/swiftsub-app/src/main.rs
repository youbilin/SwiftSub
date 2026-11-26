#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod desktop;
mod qr;
mod rule_catalog;
mod saved_document;
mod ui;

use app::SwiftSubApp;
use gpui_kit::component::{Theme, ThemeMode, TitleBar};
use gpui_kit::*;

fn main() {
    let Some(instance_guard) = desktop::SingleInstanceGuard::acquire(&app::data_dir()) else {
        return;
    };
    let mut instance_guard = Some(instance_guard);

    application().with_assets(assets::Assets).run(move |cx| {
        init(cx);
        cx.set_window_appearance(Some(WindowAppearance::Dark));
        Theme::change(ThemeMode::Dark, None, cx);
        configure_theme(cx, false);

        let bounds = Bounds::centered(None, size(px(1180.0), px(760.0)), cx);
        let mut options = TitleBar::window_options();
        options.window_bounds = Some(WindowBounds::Windowed(bounds));
        options.window_min_size = Some(size(px(1024.0), px(680.0)));
        options.titlebar = Some(TitlebarOptions {
            title: Some("SwiftSub".into()),
            appears_transparent: true,
            ..Default::default()
        });

        let guard = instance_guard.take();
        open_window(options, cx, move |window, cx| {
            cx.new(|cx| SwiftSubApp::new(window, guard, cx))
        })
        .expect("failed to open SwiftSub window");
    });
}

fn platform_ui_font() -> &'static str {
    #[cfg(target_os = "windows")]
    {
        "Microsoft YaHei UI"
    }
    #[cfg(target_os = "macos")]
    {
        ".AppleSystemUIFont"
    }
    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    {
        "Noto Sans CJK SC"
    }
}

fn platform_mono_font() -> &'static str {
    #[cfg(target_os = "windows")]
    {
        "Consolas"
    }
    #[cfg(target_os = "macos")]
    {
        "Menlo"
    }
    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    {
        "JetBrains Mono"
    }
}

pub(crate) fn configure_theme(cx: &mut App, light: bool) {
    let (primary, hover, active) = if light {
        (rgb(0x3b6ef5), rgb(0x2f5ee0), rgb(0x254fc7))
    } else {
        (rgb(0x5b8cff), rgb(0x4a7bf0), rgb(0x3d6de0))
    };
    Theme::update(cx, |theme| {
        theme.font_family = platform_ui_font().into();
        theme.font_size = px(14.0);
        theme.mono_font_family = platform_mono_font().into();
        theme.mono_font_size = px(13.0);
        theme.primary = Hsla::from(primary);
        theme.primary_foreground = Hsla::from(rgb(0xffffff));
        theme.primary_hover = Hsla::from(hover);
        theme.primary_active = Hsla::from(active);
        theme.button_primary = Hsla::from(primary);
        theme.button_primary_foreground = Hsla::from(rgb(0xffffff));
        theme.button_primary_hover = Hsla::from(hover);
        theme.button_primary_active = Hsla::from(active);
        theme.tokens.primary = Hsla::from(primary).into();
    });
}
