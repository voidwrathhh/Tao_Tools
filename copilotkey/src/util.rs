//! 通用工具：路径、控制台输出、日志、字符串转换。
//!
//! 输出统一走 `say()`，不用 `println!`：
//! 1. 不 panic（GUI 子系统进程没有控制台时 `println!` 会直接 panic）；
//! 2. 句柄是控制台时用 `WriteConsoleW` 写 UTF-16，避免中文在 GBK 代码页下乱码。

use std::ffi::OsStr;
use std::fs::OpenOptions;
use std::io::Write;
use std::os::windows::ffi::OsStrExt;
use std::os::windows::io::AsRawHandle;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};

use windows_sys::Win32::Foundation::{HANDLE, INVALID_HANDLE_VALUE};
use windows_sys::Win32::Storage::FileSystem::WriteFile;
use windows_sys::Win32::System::Console::{
    AttachConsole, GetConsoleMode, GetStdHandle, SetStdHandle, WriteConsoleW, ATTACH_PARENT_PROCESS,
    STD_ERROR_HANDLE, STD_INPUT_HANDLE, STD_OUTPUT_HANDLE,
};
use windows_sys::Win32::System::SystemInformation::GetLocalTime;
use windows_sys::Win32::Foundation::SYSTEMTIME;

/// 转成以 0 结尾的 UTF-16，供 Win32 宽字符 API 使用。
pub fn wide(s: &str) -> Vec<u16> {
    OsStr::new(s).encode_wide().chain(std::iter::once(0)).collect()
}

fn handle_valid(h: HANDLE) -> bool {
    !h.is_null() && h != INVALID_HANDLE_VALUE
}

/// GUI 子系统的进程被终端拉起时，让它也能输出文字。
pub fn ensure_console() {
    unsafe {
        if handle_valid(GetStdHandle(STD_OUTPUT_HANDLE)) {
            return;
        }
        if AttachConsole(ATTACH_PARENT_PROCESS) == 0 {
            return;
        }
        attach_handle(STD_OUTPUT_HANDLE, "CONOUT$");
        attach_handle(STD_ERROR_HANDLE, "CONOUT$");
        attach_handle(STD_INPUT_HANDLE, "CONIN$");
    }
}

unsafe fn attach_handle(std_handle: u32, device: &str) {
    if let Ok(file) = OpenOptions::new().read(true).write(true).open(device) {
        SetStdHandle(std_handle, file.as_raw_handle() as HANDLE);
        // 句柄交给标准输入输出，故意泄漏，随进程结束回收。
        std::mem::forget(file);
    }
}

/// 安全地向标准输出写一段文本（控制台用宽字符，管道用 UTF-8 字节）。
pub fn say(text: &str) {
    if text.is_empty() {
        return;
    }
    unsafe {
        let handle = GetStdHandle(STD_OUTPUT_HANDLE);
        if !handle_valid(handle) {
            return;
        }
        let mut mode: u32 = 0;
        if GetConsoleMode(handle, &mut mode) != 0 {
            let utf16: Vec<u16> = text.encode_utf16().collect();
            let mut written: u32 = 0;
            WriteConsoleW(
                handle,
                utf16.as_ptr(),
                utf16.len() as u32,
                &mut written,
                std::ptr::null(),
            );
        } else {
            let bytes = text.as_bytes();
            let mut written: u32 = 0;
            WriteFile(
                handle,
                bytes.as_ptr(),
                bytes.len() as u32,
                &mut written,
                std::ptr::null_mut(),
            );
        }
    }
}

/// 配置、日志所在目录：%APPDATA%\.tao_tools\copilotkey
pub fn app_dir() -> PathBuf {
    let base = std::env::var_os("APPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."));
    base.join(".tao_tools").join("copilotkey")
}

pub fn ensure_dirs() -> std::io::Result<()> {
    std::fs::create_dir_all(app_dir())
}

pub fn config_path() -> PathBuf {
    app_dir().join("config.json")
}

pub fn log_path() -> PathBuf {
    app_dir().join("copilotkey.log")
}

/// 本地时间戳，形如 2026-11-21 22:30:01.123
pub fn timestamp() -> String {
    unsafe {
        let mut st: SYSTEMTIME = std::mem::zeroed();
        GetLocalTime(&mut st);
        format!(
            "{:04}-{:02}-{:02} {:02}:{:02}:{:02}.{:03}",
            st.wYear, st.wMonth, st.wDay, st.wHour, st.wMinute, st.wSecond, st.wMilliseconds
        )
    }
}

/// 是否写日志（守护进程按配置开关）。
static LOGGING: AtomicBool = AtomicBool::new(true);

pub fn set_logging(enabled: bool) {
    LOGGING.store(enabled, Ordering::Relaxed);
}

/// 追加一行日志（超过 2MB 自动清空重来）。
pub fn log(message: &str) {
    if !LOGGING.load(Ordering::Relaxed) {
        return;
    }
    let _ = ensure_dirs();
    let path = log_path();
    if let Ok(meta) = std::fs::metadata(&path) {
        if meta.len() > 2 * 1024 * 1024 {
            let _ = std::fs::remove_file(&path);
        }
    }
    if let Ok(mut file) = OpenOptions::new().create(true).append(true).open(&path) {
        let _ = writeln!(file, "[{}] {}", timestamp(), message);
    }
}

/// 读取日志最后 n 行。
pub fn tail_log(n: usize) -> String {
    match std::fs::read_to_string(log_path()) {
        Ok(text) => {
            let lines: Vec<&str> = text.lines().collect();
            let start = lines.len().saturating_sub(n);
            lines[start..].join("\n")
        }
        Err(_) => String::new(),
    }
}
