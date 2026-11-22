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
use std::path::{Path, PathBuf};
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
    unsafe { write_handle(GetStdHandle(STD_OUTPUT_HANDLE), text) }
}

/// 向标准错误写一段文本。错误信息走 stderr，但同样用宽字符输出，避免中文乱码。
pub fn err_say(text: &str) {
    unsafe { write_handle(GetStdHandle(STD_ERROR_HANDLE), text) }
}

unsafe fn write_handle(handle: HANDLE, text: &str) {
    if text.is_empty() || !handle_valid(handle) {
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

/// 运行状态文件：守护进程写的 pid / 启动时间 / 上次触发结果。
pub fn state_path() -> PathBuf {
    app_dir().join("state.json")
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

/// 文件名安全的时间戳，形如 20261122-123001-123（用于 `.broken-<时间戳>` 备份名）。
pub fn file_stamp() -> String {
    unsafe {
        let mut st: SYSTEMTIME = std::mem::zeroed();
        GetLocalTime(&mut st);
        format!(
            "{:04}{:02}{:02}-{:02}{:02}{:02}-{:03}",
            st.wYear, st.wMonth, st.wDay, st.wHour, st.wMinute, st.wSecond, st.wMilliseconds
        )
    }
}

/// 临时文件路径，带 pid 后缀：两个进程同时保存时不会互相写坏同一个临时文件。
pub fn tmp_path(path: &Path) -> PathBuf {
    let mut name = path.file_name().unwrap_or_default().to_os_string();
    name.push(format!(".tmp-{}", std::process::id()));
    path.with_file_name(name)
}

/// 原子写：写临时文件 → `fsync` → `rename` 覆盖正式文件。
///
/// 任何时刻读到的要么是旧的完整内容、要么是新的完整内容，不会读到写了一半的文件。
/// 配置（`config.rs`）与运行状态（`state.rs`）共用这一条写路径。
pub fn write_atomic(path: &Path, text: &str) -> Result<(), String> {
    let tmp = tmp_path(path);
    let written = (|| -> std::io::Result<()> {
        let mut file = std::fs::File::create(&tmp)?;
        file.write_all(text.as_bytes())?;
        // 落盘后再 rename：断电/崩溃时不会留下“内容还没写完”的正式文件。
        file.sync_all()?;
        Ok(())
    })();
    if let Err(err) = written {
        let _ = std::fs::remove_file(&tmp);
        return Err(format!("写入临时文件失败：{}（{}）", err, tmp.display()));
    }
    if let Err(err) = rename_with_retry(&tmp, path) {
        let _ = std::fs::remove_file(&tmp);
        return Err(format!(
            "替换文件失败：{}（临时文件：{}）",
            err,
            tmp.display()
        ));
    }
    Ok(())
}

/// rename 偶尔会撞上别的进程短暂持有句柄（杀毒软件、索引器），短暂重试几次。
fn rename_with_retry(from: &Path, to: &Path) -> std::io::Result<()> {
    let mut last: Option<std::io::Error> = None;
    for _ in 0..5 {
        match std::fs::rename(from, to) {
            Ok(()) => return Ok(()),
            Err(err) => {
                last = Some(err);
                std::thread::sleep(std::time::Duration::from_millis(20));
            }
        }
    }
    Err(last.unwrap_or_else(|| std::io::Error::new(std::io::ErrorKind::Other, "重命名文件失败")))
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
