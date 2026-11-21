#[derive(Debug, Clone)]
pub struct ManagedWindow {
    pub handle: String,
    pub hwnd: isize,
    pub pid: u32,
    pub title: String,
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
    pub minimized: bool,
}

impl ManagedWindow {
    pub fn rect_label(&self) -> String {
        format!("{},{},{}x{}", self.x, self.y, self.width, self.height)
    }
}

#[cfg(windows)]
mod platform {
    use super::ManagedWindow;
    use windows_sys::Win32::Foundation::{BOOL, HWND, LPARAM, RECT};
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        EnumWindows, GetWindowRect, GetWindowTextLengthW, GetWindowTextW, GetWindowThreadProcessId,
        IsIconic, IsWindowVisible, SetForegroundWindow, SetWindowPos, ShowWindow, HWND_NOTOPMOST,
        HWND_TOPMOST, SWP_NOMOVE, SWP_NOSIZE, SW_MINIMIZE, SW_RESTORE,
    };

    pub fn list_windows() -> Result<Vec<ManagedWindow>, String> {
        let mut windows: Vec<ManagedWindow> = Vec::new();
        let ok = unsafe { EnumWindows(Some(enum_windows_proc), &mut windows as *mut _ as LPARAM) };
        if ok == 0 && !windows.is_empty() {
            return Err("枚举窗口失败".to_string());
        }
        windows.sort_by(|a, b| a.title.to_lowercase().cmp(&b.title.to_lowercase()));
        Ok(windows)
    }

    pub fn find_windows(keyword: &str) -> Result<Vec<ManagedWindow>, String> {
        let keyword = keyword.to_lowercase();
        Ok(list_windows()?
            .into_iter()
            .filter(|w| {
                w.title.to_lowercase().contains(&keyword)
                    || w.pid.to_string() == keyword
                    || w.handle.to_lowercase() == keyword
            })
            .collect())
    }

    pub fn focus_window(keyword: &str) -> Result<ManagedWindow, String> {
        let item = first_match(keyword)?;
        unsafe {
            ShowWindow(item.hwnd as HWND, SW_RESTORE);
            if SetForegroundWindow(item.hwnd as HWND) == 0 {
                return Err(format!("无法聚焦窗口 '{}'", item.title));
            }
        }
        Ok(item)
    }

    pub fn move_window(
        keyword: &str,
        x: i32,
        y: i32,
        width: i32,
        height: i32,
    ) -> Result<ManagedWindow, String> {
        if width <= 0 || height <= 0 {
            return Err("窗口宽高必须大于 0".to_string());
        }
        let item = first_match(keyword)?;
        unsafe {
            let ok = SetWindowPos(
                item.hwnd as HWND,
                std::ptr::null_mut(),
                x,
                y,
                width,
                height,
                0,
            );
            if ok == 0 {
                return Err(format!("无法移动窗口 '{}'", item.title));
            }
        }
        Ok(item)
    }

    pub fn set_topmost(keyword: &str, enable: bool) -> Result<ManagedWindow, String> {
        let item = first_match(keyword)?;
        let insert_after = if enable { HWND_TOPMOST } else { HWND_NOTOPMOST };
        unsafe {
            let ok = SetWindowPos(
                item.hwnd as HWND,
                insert_after,
                0,
                0,
                0,
                0,
                SWP_NOMOVE | SWP_NOSIZE,
            );
            if ok == 0 {
                return Err(format!("无法设置窗口置顶状态 '{}'", item.title));
            }
        }
        Ok(item)
    }

    pub fn minimize_window(keyword: &str) -> Result<ManagedWindow, String> {
        show_window(keyword, SW_MINIMIZE)
    }

    pub fn restore_window(keyword: &str) -> Result<ManagedWindow, String> {
        show_window(keyword, SW_RESTORE)
    }

    fn show_window(keyword: &str, cmd: i32) -> Result<ManagedWindow, String> {
        let item = first_match(keyword)?;
        unsafe {
            ShowWindow(item.hwnd as HWND, cmd);
        }
        Ok(item)
    }

    fn first_match(keyword: &str) -> Result<ManagedWindow, String> {
        find_windows(keyword)?
            .into_iter()
            .next()
            .ok_or_else(|| format!("未找到匹配窗口 '{}'", keyword))
    }

    unsafe extern "system" fn enum_windows_proc(hwnd: HWND, lparam: LPARAM) -> BOOL {
        if IsWindowVisible(hwnd) == 0 {
            return 1;
        }

        let title_len = GetWindowTextLengthW(hwnd);
        if title_len <= 0 {
            return 1;
        }

        let mut title_buf = vec![0u16; title_len as usize + 1];
        let copied = GetWindowTextW(hwnd, title_buf.as_mut_ptr(), title_buf.len() as i32);
        if copied <= 0 {
            return 1;
        }
        let title = String::from_utf16_lossy(&title_buf[..copied as usize])
            .trim()
            .to_string();
        if title.is_empty() {
            return 1;
        }

        let mut rect = RECT {
            left: 0,
            top: 0,
            right: 0,
            bottom: 0,
        };
        if GetWindowRect(hwnd, &mut rect as *mut RECT) == 0 {
            return 1;
        }

        let mut pid = 0u32;
        GetWindowThreadProcessId(hwnd, &mut pid as *mut u32);
        let windows = &mut *(lparam as *mut Vec<ManagedWindow>);
        windows.push(ManagedWindow {
            handle: format!("0x{:X}", hwnd as usize),
            hwnd: hwnd as isize,
            pid,
            title,
            x: rect.left,
            y: rect.top,
            width: rect.right - rect.left,
            height: rect.bottom - rect.top,
            minimized: IsIconic(hwnd) != 0,
        });

        1
    }
}

#[cfg(not(windows))]
mod platform {
    use super::ManagedWindow;

    pub fn list_windows() -> Result<Vec<ManagedWindow>, String> {
        Err("窗口管理功能当前仅支持 Windows".to_string())
    }

    pub fn find_windows(_keyword: &str) -> Result<Vec<ManagedWindow>, String> {
        Err("窗口管理功能当前仅支持 Windows".to_string())
    }

    pub fn focus_window(_keyword: &str) -> Result<ManagedWindow, String> {
        Err("窗口管理功能当前仅支持 Windows".to_string())
    }

    pub fn move_window(
        _keyword: &str,
        _x: i32,
        _y: i32,
        _width: i32,
        _height: i32,
    ) -> Result<ManagedWindow, String> {
        Err("窗口管理功能当前仅支持 Windows".to_string())
    }

    pub fn set_topmost(_keyword: &str, _enable: bool) -> Result<ManagedWindow, String> {
        Err("窗口管理功能当前仅支持 Windows".to_string())
    }

    pub fn minimize_window(_keyword: &str) -> Result<ManagedWindow, String> {
        Err("窗口管理功能当前仅支持 Windows".to_string())
    }

    pub fn restore_window(_keyword: &str) -> Result<ManagedWindow, String> {
        Err("窗口管理功能当前仅支持 Windows".to_string())
    }
}

pub use platform::*;
