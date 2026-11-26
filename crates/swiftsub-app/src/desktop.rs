//! Windows lifecycle integration, owned by the GPUI thread.
use std::path::Path;

#[cfg(windows)]
use sha2::{Digest, Sha256};
#[cfg(windows)]
use tray_icon::{
    Icon, MouseButton, MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent,
    menu::{Menu, MenuEvent, MenuItem, PredefinedMenuItem},
};

#[cfg(windows)]
mod win32 {
    use std::ffi::c_void;

    pub const ERROR_ALREADY_EXISTS: u32 = 183;
    pub const WAIT_OBJECT_0: u32 = 0;
    pub const EVENT_MODIFY_STATE: u32 = 0x0002;
    pub const IMAGE_ICON: u32 = 1;
    pub const LR_DEFAULTSIZE: u32 = 0x0040;
    pub const WM_SETICON: u32 = 0x0080;
    pub const ICON_SMALL: usize = 0;
    pub const ICON_BIG: usize = 1;

    #[link(name = "kernel32")]
    unsafe extern "system" {
        pub fn GetModuleHandleW(module_name: *const u16) -> *mut c_void;
        pub fn GetLastError() -> u32;
        pub fn CreateMutexW(
            attributes: *const c_void,
            initial_owner: i32,
            name: *const u16,
        ) -> *mut c_void;
        pub fn CreateEventW(
            attributes: *const c_void,
            manual_reset: i32,
            initial_state: i32,
            name: *const u16,
        ) -> *mut c_void;
        pub fn OpenEventW(
            desired_access: u32,
            inherit_handle: i32,
            name: *const u16,
        ) -> *mut c_void;
        pub fn SetEvent(handle: *mut c_void) -> i32;
        pub fn WaitForSingleObject(handle: *mut c_void, milliseconds: u32) -> u32;
        pub fn CloseHandle(handle: *mut c_void) -> i32;
    }

    #[link(name = "user32")]
    unsafe extern "system" {
        pub fn LoadImageW(
            hinst: *mut c_void,
            name: *const u16,
            r#type: u32,
            cx: i32,
            cy: i32,
            fu_load: u32,
        ) -> *mut c_void;
        pub fn SendMessageW(hwnd: *mut c_void, msg: u32, wparam: usize, lparam: isize) -> isize;
        pub fn ShowWindow(hwnd: *mut c_void, command: i32) -> i32;
        pub fn SetForegroundWindow(hwnd: *mut c_void) -> i32;
    }

    #[link(name = "shell32")]
    unsafe extern "system" {
        pub fn ShellExecuteW(
            hwnd: *mut c_void,
            operation: *const u16,
            file: *const u16,
            params: *const u16,
            directory: *const u16,
            show: i32,
        ) -> isize;
    }

    pub fn to_wide(text: &str) -> Vec<u16> {
        text.encode_utf16().chain(std::iter::once(0)).collect()
    }
}

/// Process-level single-instance lock scoped to a data directory.
///
/// Separate data directories (such as isolated smoke-test fixtures) can run side by side,
/// while launching a second process against the same data directory wakes the existing
/// window and exits cleanly before touching SQLite or local ports.
pub struct SingleInstanceGuard {
    #[cfg(windows)]
    mutex: isize,
    #[cfg(windows)]
    wakeup: isize,
}

impl SingleInstanceGuard {
    pub fn acquire(data_dir: &Path) -> Option<Self> {
        #[cfg(windows)]
        {
            let key = instance_key(data_dir);
            let mutex_name = win32::to_wide(&format!("Local\\SwiftSub-Instance-{key}"));
            let event_name = win32::to_wide(&format!("Local\\SwiftSub-Wakeup-{key}"));
            let mutex = unsafe { win32::CreateMutexW(std::ptr::null(), 0, mutex_name.as_ptr()) };
            if mutex.is_null() {
                return Some(Self {
                    mutex: 0,
                    wakeup: 0,
                });
            }
            if unsafe { win32::GetLastError() } == win32::ERROR_ALREADY_EXISTS {
                let existing =
                    unsafe { win32::OpenEventW(win32::EVENT_MODIFY_STATE, 0, event_name.as_ptr()) };
                if !existing.is_null() {
                    unsafe {
                        win32::SetEvent(existing);
                        win32::CloseHandle(existing);
                    }
                }
                unsafe {
                    win32::CloseHandle(mutex);
                }
                return None;
            }
            let wakeup =
                unsafe { win32::CreateEventW(std::ptr::null(), 0, 0, event_name.as_ptr()) };
            Some(Self {
                mutex: mutex as isize,
                wakeup: wakeup as isize,
            })
        }
        #[cfg(not(windows))]
        {
            let _ = data_dir;
            Some(Self {})
        }
    }

    pub fn poll_wakeup(&self) -> bool {
        #[cfg(windows)]
        {
            self.wakeup != 0
                && unsafe { win32::WaitForSingleObject(self.wakeup as *mut _, 0) }
                    == win32::WAIT_OBJECT_0
        }
        #[cfg(not(windows))]
        {
            false
        }
    }
}

impl Drop for SingleInstanceGuard {
    fn drop(&mut self) {
        #[cfg(windows)]
        unsafe {
            if self.wakeup != 0 {
                win32::CloseHandle(self.wakeup as *mut _);
            }
            if self.mutex != 0 {
                win32::CloseHandle(self.mutex as *mut _);
            }
        }
    }
}

#[cfg(windows)]
fn instance_key(data_dir: &Path) -> String {
    let canonical = std::fs::canonicalize(data_dir).unwrap_or_else(|_| data_dir.to_path_buf());
    let normalized = canonical
        .to_string_lossy()
        .replace('/', "\\")
        .trim_end_matches('\\')
        .to_ascii_lowercase();
    let digest = Sha256::digest(normalized.as_bytes());
    let mut hex = String::with_capacity(16);
    for byte in &digest[..8] {
        use std::fmt::Write as _;
        let _ = write!(&mut hex, "{byte:02x}");
    }
    hex
}

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
    instance: Option<SingleInstanceGuard>,
    pub available: bool,
}

impl DesktopIntegration {
    pub fn new(window: &gpui_kit::Window, instance: Option<SingleInstanceGuard>) -> Self {
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
                instance,
                available,
            }
        }
        #[cfg(not(windows))]
        {
            let _ = window;
            Self {
                instance,
                available: false,
            }
        }
    }
    pub fn actions(&self) -> Vec<DesktopAction> {
        let mut actions = Vec::new();
        if self
            .instance
            .as_ref()
            .is_some_and(SingleInstanceGuard::poll_wakeup)
        {
            actions.push(DesktopAction::Show);
        }
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
                win32::ShowWindow(self.hwnd as *mut _, 0);
            }
        }
    }
    pub fn show(&self) {
        #[cfg(windows)]
        if self.hwnd != 0 {
            unsafe {
                win32::ShowWindow(self.hwnd as *mut _, 9);
                win32::SetForegroundWindow(self.hwnd as *mut _);
            }
        }
    }
}

#[cfg(windows)]
fn apply_window_icon(hwnd: isize) {
    unsafe {
        let hinst = win32::GetModuleHandleW(std::ptr::null());
        let res_id = std::ptr::without_provenance::<u16>(1);
        let big = win32::LoadImageW(
            hinst,
            res_id,
            win32::IMAGE_ICON,
            0,
            0,
            win32::LR_DEFAULTSIZE,
        );
        if !big.is_null() {
            win32::SendMessageW(
                hwnd as *mut _,
                win32::WM_SETICON,
                win32::ICON_BIG,
                big as isize,
            );
        }
        let small = win32::LoadImageW(hinst, res_id, win32::IMAGE_ICON, 16, 16, 0);
        if !small.is_null() {
            win32::SendMessageW(
                hwnd as *mut _,
                win32::WM_SETICON,
                win32::ICON_SMALL,
                small as isize,
            );
        }
    }
}

#[cfg(windows)]
fn registry_command() -> std::process::Command {
    use std::os::windows::process::CommandExt;
    let mut cmd = std::process::Command::new("reg.exe");
    cmd.creation_flags(0x08000000);
    cmd
}

#[cfg(windows)]
const AUTO_START_RUN_KEY: &str = r"HKCU\Software\Microsoft\Windows\CurrentVersion\Run";
#[cfg(windows)]
const AUTO_START_VALUE_NAME: &str = "SwiftSub";

/// Returns whether Windows user-level background auto-start is currently registered.
pub fn is_auto_start_enabled() -> bool {
    #[cfg(windows)]
    {
        registry_command()
            .args(["query", AUTO_START_RUN_KEY, "/v", AUTO_START_VALUE_NAME])
            .output()
            .is_ok_and(|output| output.status.success())
    }
    #[cfg(not(windows))]
    {
        false
    }
}

/// Enable or disable Windows login auto-start (`--background`, minimized to tray).
pub fn set_auto_start(enabled: bool) -> Result<(), String> {
    #[cfg(windows)]
    {
        if enabled {
            let exe = std::env::current_exe()
                .map_err(|error| format!("无法获取当前程序路径：{error}"))?;
            let command = format!("\"{}\" --background", exe.display());
            let output = registry_command()
                .args([
                    "add",
                    AUTO_START_RUN_KEY,
                    "/v",
                    AUTO_START_VALUE_NAME,
                    "/t",
                    "REG_SZ",
                    "/d",
                    &command,
                    "/f",
                ])
                .output()
                .map_err(|error| format!("调用注册表失败：{error}"))?;
            if output.status.success() {
                Ok(())
            } else {
                Err("写入开机自启注册表项失败".into())
            }
        } else {
            let output = registry_command()
                .args([
                    "delete",
                    AUTO_START_RUN_KEY,
                    "/v",
                    AUTO_START_VALUE_NAME,
                    "/f",
                ])
                .output()
                .map_err(|error| format!("调用注册表失败：{error}"))?;
            if output.status.success() || !is_auto_start_enabled() {
                Ok(())
            } else {
                Err("移除开机自启注册表项失败".into())
            }
        }
    }
    #[cfg(not(windows))]
    {
        let _ = enabled;
        Err("当前平台暂不支持开机自启".into())
    }
}

/// Build a ready-to-run Windows Firewall inbound allow command for the current executable and port.
pub fn lan_firewall_command(port: u16) -> String {
    let exe = std::env::current_exe()
        .map(|path| path.display().to_string())
        .unwrap_or_else(|_| "swiftsub.exe".to_string());
    format!(
        "netsh advfirewall firewall add rule name=\"SwiftSub LAN ({port})\" dir=in action=allow program=\"{exe}\" protocol=TCP localport={port} profile=private"
    )
}

/// Auto-detect an installed Mihomo / Clash Verge core binary on the local machine.
pub fn detect_mihomo_binary() -> Option<std::path::PathBuf> {
    let mut candidates = Vec::new();
    if let Ok(path_var) = std::env::var("PATH") {
        for dir in std::env::split_paths(&path_var) {
            for name in ["mihomo.exe", "verge-mihomo.exe", "mihomo"] {
                candidates.push(dir.join(name));
            }
        }
    }
    #[cfg(windows)]
    {
        for env_key in ["ProgramFiles", "LOCALAPPDATA"] {
            if let Ok(base) = std::env::var(env_key) {
                let base = std::path::PathBuf::from(base);
                for rel in [
                    "Clash Verge/verge-mihomo.exe",
                    "Clash Verge/verge-mihomo-alpha.exe",
                    "Clash Verge/mihomo.exe",
                    "Programs/Clash Verge/verge-mihomo.exe",
                    "Mihomo Party/resources/sidecar/mihomo.exe",
                ] {
                    candidates.push(base.join(rel));
                }
            }
        }
    }
    candidates.into_iter().find(|path| path.is_file())
}

/// Query the OS preference for light application theme (`AppsUseLightTheme` on Windows).
pub fn system_prefers_light() -> Option<bool> {
    #[cfg(windows)]
    {
        let output = registry_command()
            .args([
                "query",
                r"HKCU\Software\Microsoft\Windows\CurrentVersion\Themes\Personalize",
                "/v",
                "AppsUseLightTheme",
            ])
            .output()
            .ok()?;
        if !output.status.success() {
            return None;
        }
        let stdout = String::from_utf8_lossy(&output.stdout);
        for line in stdout.lines() {
            if line.contains("AppsUseLightTheme") {
                let value = line.split_whitespace().last()?;
                return match value {
                    "0x1" | "1" => Some(true),
                    "0x0" | "0" => Some(false),
                    _ => None,
                };
            }
        }
        None
    }
    #[cfg(not(windows))]
    {
        None
    }
}

/// Validate a generated Clash/Mihomo YAML using a local `mihomo` binary (`mihomo -t`).
pub fn run_mihomo_check(mihomo_path: &Path, yaml: &str) -> Result<String, String> {
    if !mihomo_path.is_file() {
        return Err(format!("找不到 Mihomo 内核文件：{}", mihomo_path.display()));
    }
    let work_dir = std::env::temp_dir().join(format!(
        "swiftsub-mihomo-check-{}",
        uuid::Uuid::new_v4().simple()
    ));
    std::fs::create_dir_all(&work_dir).map_err(|e| format!("创建临时目录失败：{e}"))?;
    let cfg_path = work_dir.join("config.yaml");
    if let Err(e) = std::fs::write(&cfg_path, yaml) {
        let _ = std::fs::remove_dir_all(&work_dir);
        return Err(format!("写入临时配置失败：{e}"));
    }

    let mut cmd = std::process::Command::new(mihomo_path);
    cmd.args(["-t", "-d"])
        .arg(&work_dir)
        .arg("-f")
        .arg(&cfg_path)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x08000000);
    }
    let mut child = match cmd.spawn() {
        Ok(child) => child,
        Err(e) => {
            let _ = std::fs::remove_dir_all(&work_dir);
            return Err(format!("无法执行 Mihomo 内核：{e}"));
        }
    };
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(15);
    let timed_out = loop {
        match child.try_wait() {
            Ok(Some(_)) => break false,
            Ok(None) => {
                if std::time::Instant::now() >= deadline {
                    let _ = child.kill();
                    break true;
                }
                std::thread::sleep(std::time::Duration::from_millis(40));
            }
            Err(_) => break false,
        }
    };
    let output = child.wait_with_output();
    let _ = std::fs::remove_dir_all(&work_dir);

    if timed_out {
        return Err("Mihomo 内核校验超时（超过 15 秒）".into());
    }
    let output = output.map_err(|e| format!("读取 Mihomo 输出失败：{e}"))?;
    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
    let detail = if !stderr.is_empty() { stderr } else { stdout };
    if output.status.success() {
        Ok(if detail.is_empty() {
            "Mihomo 内核语法校验通过".into()
        } else {
            detail
                .lines()
                .last()
                .unwrap_or("Mihomo 内核校验通过")
                .to_string()
        })
    } else {
        let excerpt: String = detail.chars().take(280).collect();
        Err(format!("Mihomo 内核校验未通过：{excerpt}"))
    }
}

/// Open the registered Verge handler, with support for its legacy Clash scheme.
pub fn open_clash_verge(uri: &str) -> Result<(), String> {
    #[cfg(windows)]
    {
        let operation = win32::to_wide("open");
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
            let file = win32::to_wide(&candidate);
            let result = unsafe {
                win32::ShellExecuteW(
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
        let operation = win32::to_wide("open");
        let file = win32::to_wide(url);
        let result = unsafe {
            win32::ShellExecuteW(
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
        let operation = win32::to_wide("open");
        let file = win32::to_wide(&path.to_string_lossy());
        let result = unsafe {
            win32::ShellExecuteW(
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

#[cfg(all(test, windows))]
mod tests {
    use super::*;

    #[test]
    fn single_instance_blocks_duplicate_and_signals_wakeup() {
        let dir = std::env::temp_dir().join(format!("swiftsub-lock-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let other = std::env::temp_dir().join(format!("swiftsub-lock-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&other).unwrap();

        let primary = SingleInstanceGuard::acquire(&dir).expect("primary lock should succeed");
        assert!(!primary.poll_wakeup());

        // Different data directory remains isolated and can run in parallel.
        let isolated = SingleInstanceGuard::acquire(&other).expect("other dir lock should succeed");
        assert!(!isolated.poll_wakeup());

        // Duplicate instance for the same data directory is rejected and wakes primary.
        assert!(SingleInstanceGuard::acquire(&dir).is_none());
        assert!(primary.poll_wakeup());
        // Wakeup event auto-resets after consumption.
        assert!(!primary.poll_wakeup());

        drop(primary);
        let reacquired = SingleInstanceGuard::acquire(&dir)
            .expect("lock should be acquirable after primary exits");
        drop(reacquired);
        drop(isolated);
        let _ = std::fs::remove_dir_all(&dir);
        let _ = std::fs::remove_dir_all(&other);
    }
}
