//! 开机自启动：HKCU\Software\Microsoft\Windows\CurrentVersion\Run

use std::ptr;

use windows_sys::Win32::Foundation::ERROR_FILE_NOT_FOUND;
use windows_sys::Win32::System::Registry::{
    RegCloseKey, RegCreateKeyExW, RegDeleteValueW, RegOpenKeyExW, RegQueryValueExW, RegSetValueExW,
    HKEY, HKEY_CURRENT_USER, KEY_QUERY_VALUE, KEY_SET_VALUE, REG_SZ,
};

use crate::util;

const RUN_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
const VALUE_NAME: &str = "TaoToolsCopilotKey";

fn create_key(access: u32) -> Result<HKEY, String> {
    let sub_key = util::wide(RUN_KEY);
    let mut key: HKEY = ptr::null_mut();
    let mut disposition: u32 = 0;
    let code = unsafe {
        RegCreateKeyExW(
            HKEY_CURRENT_USER,
            sub_key.as_ptr(),
            0,
            ptr::null(),
            0,
            access,
            ptr::null(),
            &mut key,
            &mut disposition,
        )
    };
    if code != 0 {
        return Err(format!("打开注册表 Run 项失败，错误码 {}", code));
    }
    Ok(key)
}

/// 写入自启动项，值为 `"<exe>" run`。
pub fn install(exe_path: &str) -> Result<(), String> {
    let key = create_key(KEY_SET_VALUE)?;
    let value_text = format!("\"{}\" run", exe_path);
    let mut value: Vec<u16> = value_text.encode_utf16().collect();
    value.push(0);
    let bytes = unsafe {
        std::slice::from_raw_parts(value.as_ptr() as *const u8, value.len() * std::mem::size_of::<u16>())
    };
    let name = util::wide(VALUE_NAME);
    let code = unsafe {
        RegSetValueExW(
            key,
            name.as_ptr(),
            0,
            REG_SZ,
            bytes.as_ptr(),
            bytes.len() as u32,
        )
    };
    unsafe { RegCloseKey(key) };
    if code != 0 {
        return Err(format!("写入自启动项失败，错误码 {}", code));
    }
    Ok(())
}

/// 删除自启动项；本来就没有也算成功。
pub fn remove() -> Result<(), String> {
    let key = match open_for_read() {
        Ok(key) => key,
        Err(_) => return Ok(()),
    };
    let name = util::wide(VALUE_NAME);
    let code = unsafe { RegDeleteValueW(key, name.as_ptr()) };
    unsafe { RegCloseKey(key) };
    if code != 0 && code != ERROR_FILE_NOT_FOUND {
        return Err(format!("删除自启动项失败，错误码 {}", code));
    }
    Ok(())
}

/// 读取自启动项内容，未设置返回 None。
pub fn query() -> Option<String> {
    let key = open_for_read().ok()?;
    let name = util::wide(VALUE_NAME);
    let mut value_type: u32 = 0;
    let mut size: u32 = 0;
    let mut text = String::new();
    unsafe {
        let code = RegQueryValueExW(
            key,
            name.as_ptr(),
            ptr::null(),
            &mut value_type,
            ptr::null_mut(),
            &mut size,
        );
        if code == 0 && size > 0 {
            let mut buffer: Vec<u8> = vec![0u8; size as usize];
            let code = RegQueryValueExW(
                key,
                name.as_ptr(),
                ptr::null(),
                &mut value_type,
                buffer.as_mut_ptr(),
                &mut size,
            );
            if code == 0 {
                let wide: Vec<u16> = buffer
                    .chunks_exact(2)
                    .map(|pair| u16::from_le_bytes([pair[0], pair[1]]))
                    .take_while(|unit| *unit != 0)
                    .collect();
                text = String::from_utf16_lossy(&wide);
            }
        }
        RegCloseKey(key);
    }
    if text.is_empty() {
        None
    } else {
        Some(text)
    }
}

fn open_for_read() -> Result<HKEY, String> {
    let sub_key = util::wide(RUN_KEY);
    let mut key: HKEY = ptr::null_mut();
    let code = unsafe {
        RegOpenKeyExW(
            HKEY_CURRENT_USER,
            sub_key.as_ptr(),
            0,
            KEY_QUERY_VALUE,
            &mut key,
        )
    };
    if code != 0 {
        return Err(format!("打开注册表 Run 项失败，错误码 {}", code));
    }
    Ok(key)
}
