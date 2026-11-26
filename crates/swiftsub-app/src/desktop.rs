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
            if hwnd != 0 {
                apply_window_icon(hwnd);
            }
            let create = || -> Result<TrayIcon, Box<dyn std::error::Error>> {
                let menu = Menu::new();
                menu.append_items(&[
                    &MenuItem::with_id("show", "打开 SwiftSub", true, None),
                    &MenuItem::with_id("copy", "复制当前订阅地址", true, None),
                    &MenuItem::with_id("refresh", "刷新全部订阅", true, None),
                    &PredefinedMenuItem::separator(),
                    &MenuItem::with_id("exit", "退出", true, None),
                ])?;
                let pixels = include_bytes!("../../../assets/swiftsub-32.rgba").to_vec();
                Ok(TrayIconBuilder::new()
                    .with_tooltip("SwiftSub · 本地与局域网订阅服务")
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
fn apply_window_icon(hwnd: isize) {
    const IMAGE_ICON: u32 = 1;
    const LR_DEFAULTSIZE: u32 = 0x0040;
    const WM_SETICON: u32 = 0x0080;
    const ICON_SMALL: usize = 0;
    const ICON_BIG: usize = 1;

    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn GetModuleHandleW(module_name: *const u16) -> *mut std::ffi::c_void;
    }
    #[link(name = "user32")]
    unsafe extern "system" {
        fn LoadImageW(
            hinst: *mut std::ffi::c_void,
            name: *const u16,
            r#type: u32,
            cx: i32,
            cy: i32,
            fu_load: u32,
        ) -> *mut std::ffi::c_void;
        fn SendMessageW(
            hwnd: *mut std::ffi::c_void,
            msg: u32,
            wparam: usize,
            lparam: isize,
        ) -> isize;
    }

    unsafe {
        let hinst = GetModuleHandleW(std::ptr::null());
        let res_id = std::ptr::without_provenance::<u16>(1);
        let big = LoadImageW(hinst, res_id, IMAGE_ICON, 0, 0, LR_DEFAULTSIZE);
        if !big.is_null() {
            SendMessageW(hwnd as *mut _, WM_SETICON, ICON_BIG, big as isize);
        }
        let small = LoadImageW(hinst, res_id, IMAGE_ICON, 16, 16, 0);
        if !small.is_null() {
            SendMessageW(hwnd as *mut _, WM_SETICON, ICON_SMALL, small as isize);
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
/// Open the registered Verge handler, with support for its legacy Clash scheme.
pub fn open_clash_verge(uri: &str) -> Result<(), String> {
    #[cfg(windows)]
    {
        #[link(name = "shell32")]
        unsafe extern "system" {
            fn ShellExecuteW(
                hwnd: *mut std::ffi::c_void,
                operation: *const u16,
                file: *const u16,
                params: *const u16,
                directory: *const u16,
                show: i32,
            ) -> isize;
        }
        let operation: Vec<u16> = "open\0".encode_utf16().collect();
        // Older versions forward only clash: arguments to their existing instance.
        for candidate in [
            uri.to_string(),
            uri.replacen("clash://", "clash-verge://", 1),
        ] {
            let scheme = candidate.split_once("://").map(|p| p.0).unwrap_or("clash");
            if !registry_command()
                .args([
                    "query",
                    &format!(r"HKCR\{scheme}\shell\open\command"),
                    "/ve",
                ])
                .output()
                .is_ok_and(|output| output.status.success())
            {
                continue;
            }
            let file: Vec<u16> = candidate.encode_utf16().chain(std::iter::once(0)).collect();
            let result = unsafe {
                ShellExecuteW(
                    std::ptr::null_mut(),
                    operation.as_ptr(),
                    file.as_ptr(),
                    std::ptr::null(),
                    std::ptr::null(),
                    1,
                )
            };
            if result > 32 {
                return Ok(());
            }
        }
        Err("未能打开 Clash Verge，请先启动客户端并启用链接关联".into())
    }
    #[cfg(not(windows))]
    {
        let _ = uri;
        Err("当前平台暂不支持直接导入，请复制订阅地址".into())
    }
}

/// Open an external http/https URL in the system default browser.
pub fn open_external_url(url: &str) -> Result<(), String> {
    if !(url.starts_with("https://") || url.starts_with("http://")) {
        return Err("仅支持打开 http/https 链接".into());
    }
    #[cfg(windows)]
    {
        #[link(name = "shell32")]
        unsafe extern "system" {
            fn ShellExecuteW(
                hwnd: *mut std::ffi::c_void,
                operation: *const u16,
                file: *const u16,
                params: *const u16,
                directory: *const u16,
                show: i32,
            ) -> isize;
        }
        let operation: Vec<u16> = "open\0".encode_utf16().collect();
        let file: Vec<u16> = url.encode_utf16().chain(std::iter::once(0)).collect();
        let result = unsafe {
            ShellExecuteW(
                std::ptr::null_mut(),
                operation.as_ptr(),
                file.as_ptr(),
                std::ptr::null(),
                std::ptr::null(),
                1,
            )
        };
        if result > 32 {
            Ok(())
        } else {
            Err("无法调用系统浏览器打开链接".into())
        }
    }
    #[cfg(not(windows))]
    {
        let _ = url;
        Err("当前平台暂不支持自动打开浏览器".into())
    }
}

/// Open a local directory in the system file manager.
pub fn open_folder(path: &std::path::Path) -> Result<(), String> {
    let _ = std::fs::create_dir_all(path);
    #[cfg(windows)]
    {
        #[link(name = "shell32")]
        unsafe extern "system" {
            fn ShellExecuteW(
                hwnd: *mut std::ffi::c_void,
                operation: *const u16,
                file: *const u16,
                params: *const u16,
                directory: *const u16,
                show: i32,
            ) -> isize;
        }
        let operation: Vec<u16> = "open\0".encode_utf16().collect();
        let file: Vec<u16> = path
            .to_string_lossy()
            .encode_utf16()
            .chain(std::iter::once(0))
            .collect();
        let result = unsafe {
            ShellExecuteW(
                std::ptr::null_mut(),
                operation.as_ptr(),
                file.as_ptr(),
                std::ptr::null(),
                std::ptr::null(),
                1,
            )
        };
        if result > 32 {
            Ok(())
        } else {
            Err("无法打开目录".into())
        }
    }
    #[cfg(not(windows))]
    {
        let _ = path;
        Err("当前平台暂不支持打开目录".into())
    }
}
