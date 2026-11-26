//! Semantic design-token theme palette and light/dark/system appearance helper.

use std::sync::atomic::{AtomicBool, Ordering};

use gpui_kit::Rgba;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AppThemeMode {
    System,
    Light,
    Dark,
}

impl AppThemeMode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::System => "system",
            Self::Light => "light",
            Self::Dark => "dark",
        }
    }

    pub fn from_setting(raw: Option<&str>) -> Self {
        match raw {
            Some("dark") => Self::Dark,
            Some("system") => Self::System,
            _ => Self::Light,
        }
    }

    pub fn resolve_light(self, system_is_light: bool) -> bool {
        match self {
            Self::System => system_is_light,
            Self::Light => true,
            Self::Dark => false,
        }
    }
}

/// Complete semantic color palette for one appearance mode.
#[derive(Clone, Copy, Debug)]
pub struct ThemePalette {
    pub bg: u32,
    pub surface: u32,
    pub surface_2: u32,
    pub surface_3: u32,
    pub border: u32,
    pub border_item: u32,
    pub border_strong: u32,
    pub surface_inset: u32,
    pub text: u32,
    pub muted: u32,
    pub accent: u32,
    pub accent_soft: u32,
    pub warning: u32,
    pub danger: u32,
    pub danger_soft: u32,
    pub backdrop: u32,
    pub warn_bg: u32,
    pub warn_border: u32,
    pub danger_border: u32,
}

impl ThemePalette {
    pub const DARK: Self = Self {
        bg: 0x0e1014,
        surface: 0x14171d,
        surface_2: 0x191d25,
        surface_3: 0x202633,
        border: 0x262b36,
        border_item: 0x354052,
        border_strong: 0x45536b,
        surface_inset: 0x10141b,
        text: 0xe6e8ee,
        muted: 0x8a93a6,
        accent: 0x5b8cff,
        accent_soft: 0x17233d,
        warning: 0xffb020,
        danger: 0xff5c5c,
        danger_soft: 0x2a171a,
        backdrop: 0x05070bcc,
        warn_bg: 0x2a2110,
        warn_border: 0x5b4316,
        danger_border: 0x5b2328,
    };

    pub const LIGHT: Self = Self {
        bg: 0xf7f8fa,
        surface: 0xffffff,
        surface_2: 0xf7f8fa,
        surface_3: 0xe8edf5,
        border: 0xe1e4ea,
        border_item: 0xcdd5e1,
        border_strong: 0xb4c1d5,
        surface_inset: 0xf0f3f8,
        text: 0x1f2937,
        muted: 0x5f6c82,
        accent: 0x3b6ef5,
        accent_soft: 0xe8efff,
        warning: 0xb45309,
        danger: 0xd92d2d,
        danger_soft: 0xfdecec,
        backdrop: 0x1f29374d,
        warn_bg: 0xfff7e6,
        warn_border: 0xf0d49a,
        danger_border: 0xf3c6c6,
    };
}

/// Token identifiers aligned with `ThemePalette::DARK` for ergonomic color lookup.
pub mod tokens {
    use super::ThemePalette;

    pub const BG: u32 = ThemePalette::DARK.bg;
    pub const SURFACE: u32 = ThemePalette::DARK.surface;
    pub const SURFACE_2: u32 = ThemePalette::DARK.surface_2;
    pub const SURFACE_3: u32 = ThemePalette::DARK.surface_3;
    pub const BORDER: u32 = ThemePalette::DARK.border;
    pub const BORDER_ITEM: u32 = ThemePalette::DARK.border_item;
    pub const BORDER_STRONG: u32 = ThemePalette::DARK.border_strong;
    pub const SURFACE_INSET: u32 = ThemePalette::DARK.surface_inset;
    pub const TEXT: u32 = ThemePalette::DARK.text;
    pub const MUTED: u32 = ThemePalette::DARK.muted;
    pub const ACCENT: u32 = ThemePalette::DARK.accent;
    pub const ACCENT_SOFT: u32 = ThemePalette::DARK.accent_soft;
    pub const WARNING: u32 = ThemePalette::DARK.warning;
    pub const DANGER: u32 = ThemePalette::DARK.danger;
    pub const DANGER_SOFT: u32 = ThemePalette::DARK.danger_soft;
}

static LIGHT_THEME: AtomicBool = AtomicBool::new(false);

pub fn set_light_theme(light: bool) {
    LIGHT_THEME.store(light, Ordering::Relaxed);
}

pub fn palette() -> &'static ThemePalette {
    if LIGHT_THEME.load(Ordering::Relaxed) {
        &ThemePalette::LIGHT
    } else {
        &ThemePalette::DARK
    }
}

fn resolve_token(color: u32) -> u32 {
    let p = palette();
    let d = &ThemePalette::DARK;
    match color {
        c if c == d.bg => p.bg,
        c if c == d.surface => p.surface,
        c if c == d.surface_2 => p.surface_2,
        c if c == d.surface_3 => p.surface_3,
        c if c == d.border => p.border,
        c if c == d.border_item => p.border_item,
        c if c == d.border_strong => p.border_strong,
        c if c == d.surface_inset => p.surface_inset,
        c if c == d.text => p.text,
        c if c == d.muted => p.muted,
        c if c == d.accent => p.accent,
        c if c == d.accent_soft => p.accent_soft,
        c if c == d.warning => p.warning,
        c if c == d.danger => p.danger,
        c if c == d.danger_soft => p.danger_soft,
        c if c == d.backdrop => p.backdrop,
        c if c == d.warn_bg => p.warn_bg,
        c if c == d.warn_border => p.warn_border,
        c if c == d.danger_border => p.danger_border,
        other => other,
    }
}

pub fn rgb(color: u32) -> Rgba {
    gpui_kit::rgb(resolve_token(color))
}

pub fn rgba(color: u32) -> Rgba {
    gpui_kit::rgba(resolve_token(color))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolves_theme_modes_consistently() {
        assert_eq!(AppThemeMode::from_setting(Some("dark")), AppThemeMode::Dark);
        assert_eq!(
            AppThemeMode::from_setting(Some("system")),
            AppThemeMode::System
        );
        assert_eq!(
            AppThemeMode::from_setting(Some("light")),
            AppThemeMode::Light
        );
        assert_eq!(AppThemeMode::from_setting(None), AppThemeMode::Light);
        assert!(AppThemeMode::System.resolve_light(true));
        assert!(!AppThemeMode::System.resolve_light(false));
        assert!(AppThemeMode::Light.resolve_light(false));
        assert!(!AppThemeMode::Dark.resolve_light(true));
    }
}
