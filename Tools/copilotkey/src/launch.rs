//! 启动目标：用 ShellExecuteExW 打开程序、文档或 URL。

use windows_sys::Win32::Foundation::{CloseHandle, GetLastError};
use windows_sys::Win32::System::Com::{CoInitializeEx, COINIT_APARTMENTTHREADED};
use windows_sys::Win32::UI::Shell::{
    ShellExecuteExW, SEE_MASK_FLAG_NO_UI, SEE_MASK_NOASYNC, SHELLEXECUTEINFOW,
};
use windows_sys::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;

use crate::config::Config;
use crate::util;

pub fn launch(config: &Config) -> Result<(), String> {
    let target = config.target.trim();
    if target.is_empty() {
        return Err("还没有设置目标（用 copilotkey target <程序> 设置）".to_string());
    }

    let verb = util::wide("open");
    let file = util::wide(target);
    let parameters = if config.arguments.trim().is_empty() {
        None
    } else {
        Some(util::wide(&config.arguments))
    };
    let directory = if config.workdir.trim().is_empty() {
        None
    } else {
        Some(util::wide(&config.workdir))
    };

    let mut info: SHELLEXECUTEINFOW = unsafe { std::mem::zeroed() };
    info.cbSize = std::mem::size_of::<SHELLEXECUTEINFOW>() as u32;
    info.fMask = SEE_MASK_FLAG_NO_UI | SEE_MASK_NOASYNC;
    info.lpVerb = verb.as_ptr();
    info.lpFile = file.as_ptr();
    info.lpParameters = parameters
        .as_ref()
        .map_or(std::ptr::null(), |value| value.as_ptr());
    info.lpDirectory = directory
        .as_ref()
        .map_or(std::ptr::null(), |value| value.as_ptr());
    info.nShow = SW_SHOWNORMAL;

    unsafe {
        // ShellExecuteExW 在非 UI 线程上要求 COM 已初始化；重复初始化返回失败无所谓。
        let _ = CoInitializeEx(std::ptr::null(), COINIT_APARTMENTTHREADED as u32);
        if ShellExecuteExW(&mut info) == 0 {
            return Err(format!(
                "ShellExecuteEx 调用失败，错误码 {}",
                GetLastError()
            ));
        }
        if !info.hProcess.is_null() {
            CloseHandle(info.hProcess);
        }
    }
    Ok(())
}
