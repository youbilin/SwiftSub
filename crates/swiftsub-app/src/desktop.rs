//! Windows lifecycle integration, owned by the GPUI thread.
#[cfg(windows)]
use tray_icon::{
    Icon, MouseButton, MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent,
    menu::{Menu, MenuEvent, MenuItem, PredefinedMenuItem},
};

#[derive(Clone, Copy)]
pub enum DesktopAction {
    Show,
    Copy,
    Refresh,
    Exit,
}

pub struct DesktopIntegration {
    #[cfg(windows)]
    _tray: Option<TrayIcon>,
    #[cfg(windows)]
    hwnd: isize,
    pub available: bool,
}

impl DesktopIntegration {
    pub fn new(window: &gpui_kit::Window) -> Self {
        #[cfg(windows)]
        {
            use raw_window_handle::{HasWindowHandle, RawWindowHandle};
            let hwnd = match HasWindowHandle::window_handle(window).map(|h| h.as_raw()) {
                Ok(RawWindowHandle::Win32(h)) => h.hwnd.get(),
                _ => 0,
            };
            let create = || -> Result<TrayIcon, Box<dyn std::error::Error>> {
                let menu = Menu::new();
                menu.append_items(&[
                    &MenuItem::with_id("show", "打开 SwiftSub", true, None),
                    &MenuItem::with_id("copy", "复制当前订阅地址", true, None),
                    &MenuItem::with_id("refresh", "刷新全部订阅", true, None),
                    &PredefinedMenuItem::separator(),
                    &MenuItem::with_id("exit", "退出", true, None),
                ])?;
                let mut pixels = vec![0u8; 32 * 32 * 4];
                for y in 0..32 {
                    for x in 0..32 {
                        let i = (y * 32 + x) * 4;
                        let stripe =
                            (8..12).contains(&y) || (14..18).contains(&y) || (20..24).contains(&y);
                        let white = stripe && (8..24).contains(&x);
                        pixels[i..i + 4].copy_from_slice(if white {
                            &[255, 255, 255, 255]
                        } else {
                            &[91, 140, 255, 255]
                        });
                    }
                }
                Ok(TrayIconBuilder::new()
                    .with_tooltip("SwiftSub · 本地订阅服务")
                    .with_menu(Box::new(menu))
                    .with_menu_on_left_click(false)
                    .with_icon(Icon::from_rgba(pixels, 32, 32)?)
                    .build()?)
            };
            let tray = create().ok();
            let available = tray.is_some() && hwnd != 0;
            Self {
                _tray: tray,
                hwnd,
                available,
            }
        }
        #[cfg(not(windows))]
        {
            let _ = window;
            Self { available: false }
        }
    }
    pub fn actions(&self) -> Vec<DesktopAction> {
        let mut actions = Vec::new();
        #[cfg(windows)]
        {
            while let Ok(e) = MenuEvent::receiver().try_recv() {
                if let Some(a) = match e.id.as_ref() {
                    "show" => Some(DesktopAction::Show),
                    "copy" => Some(DesktopAction::Copy),
                    "refresh" => Some(DesktopAction::Refresh),
                    "exit" => Some(DesktopAction::Exit),
                    _ => None,
                } {
                    actions.push(a);
                }
            }
            while let Ok(e) = TrayIconEvent::receiver().try_recv() {
                if matches!(
                    e,
                    TrayIconEvent::DoubleClick {
                        button: MouseButton::Left,
                        ..
                    } | TrayIconEvent::Click {
                        button: MouseButton::Left,
                        button_state: MouseButtonState::Up,
                        ..
                    }
                ) {
                    actions.push(DesktopAction::Show);
                }
            }
        }
        actions
    }
    pub fn hide(&self) {
        #[cfg(windows)]
        if self.available {
            unsafe {
                ShowWindow(self.hwnd as *mut _, 0);
            }
        }
    }
    pub fn show(&self) {
        #[cfg(windows)]
        if self.hwnd != 0 {
            unsafe {
                ShowWindow(self.hwnd as *mut _, 9);
                SetForegroundWindow(self.hwnd as *mut _);
            }
        }
    }
}
#[cfg(windows)]
#[link(name = "user32")]
unsafe extern "system" {
    fn ShowWindow(hwnd: *mut std::ffi::c_void, command: i32) -> i32;
    fn SetForegroundWindow(hwnd: *mut std::ffi::c_void) -> i32;
}

#[cfg(windows)]
fn registry_command() -> std::process::Command {
    use std::os::windows::process::CommandExt;
    let mut cmd = std::process::Command::new("reg.exe");
    cmd.creation_flags(0x08000000);
    cmd
}
pub fn autostart_enabled() -> bool {
    #[cfg(windows)]
    {
        registry_command()
            .args([
                "query",
                r"HKCU\Software\Microsoft\Windows\CurrentVersion\Run",
                "/v",
                "SwiftSub",
            ])
            .output()
            .is_ok_and(|o| o.status.success())
    }
    #[cfg(not(windows))]
    {
        false
    }
}
pub fn set_autostart(enabled: bool) -> Result<(), String> {
    #[cfg(windows)]
    {
        let mut cmd = registry_command();
        let key = r"HKCU\Software\Microsoft\Windows\CurrentVersion\Run";
        if enabled {
            let exe = std::env::current_exe().map_err(|e| e.to_string())?;
            cmd.args([
                "add",
                key,
                "/v",
                "SwiftSub",
                "/t",
                "REG_SZ",
                "/d",
                &format!("\"{}\" --background", exe.display()),
                "/f",
            ]);
        } else {
            if !autostart_enabled() {
                return Ok(());
            }
            cmd.args(["delete", key, "/v", "SwiftSub", "/f"]);
        }
        let result = cmd.output().map_err(|e| e.to_string())?;
        if result.status.success() {
            Ok(())
        } else {
            Err(String::from_utf8_lossy(&result.stderr).into_owned())
        }
    }
    #[cfg(not(windows))]
    {
        let _ = enabled;
        Err("当前平台暂不支持开机自启".into())
    }
}
