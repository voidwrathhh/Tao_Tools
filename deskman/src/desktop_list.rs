//! 桌面图标列表（SysListView32）的跨进程读取与操作。
//!
//! 桌面图标列表属于 Explorer 进程，文本/位置读取需要远程内存；
//! 移动图标用 LVM_SETITEMPOSITION 直接携带坐标，无需远程缓冲。

#[derive(Debug, Clone)]
pub struct DesktopIcon {
    pub index: i32,
    pub name: String,
    pub x: i32,
    pub y: i32,
}

// ---------- Windows 实现 ----------

#[cfg(windows)]
mod platform {
    use super::DesktopIcon;
    use windows_sys::Win32::Foundation::CloseHandle;
    use windows_sys::Win32::System::Diagnostics::Debug::{ReadProcessMemory, WriteProcessMemory};
    use windows_sys::Win32::System::Memory::{
        VirtualAllocEx, VirtualFreeEx, MEM_COMMIT, MEM_RELEASE, MEM_RESERVE, PAGE_READWRITE,
    };
    use windows_sys::Win32::System::Threading::{
        OpenProcess, PROCESS_VM_OPERATION, PROCESS_VM_READ, PROCESS_VM_WRITE,
    };
    use windows_sys::Win32::Foundation::HWND;
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        FindWindowExW, FindWindowW, GetWindowLongPtrW, GetWindowThreadProcessId,
        SendMessageW,
    };

    const LVM_FIRST: u32 = 0x1000;
    const LVM_GETITEMCOUNT: u32 = LVM_FIRST + 4;
    const LVM_GETITEMPOSITION: u32 = LVM_FIRST + 16;
    const LVM_SETITEMPOSITION: u32 = LVM_FIRST + 15;
    const LVM_GETITEMTEXTW: u32 = LVM_FIRST + 115;
    const LVIF_TEXT: u32 = 0x0000_0001;

    /// SHELLDLL_DefView（桌面视图容器，视图命令发给它）
    pub fn desktop_defview() -> Option<HWND> {
        unsafe {
            let progman_class = wide("Progman");
            let progman = FindWindowW(progman_class.as_ptr(), std::ptr::null());
            let defview_class = wide("SHELLDLL_DefView");
            let mut defview = if progman.is_null() {
                std::ptr::null_mut()
            } else {
                FindWindowExW(progman, std::ptr::null_mut(), defview_class.as_ptr(), std::ptr::null())
            };
            if defview.is_null() {
                let worker_class = wide("WorkerW");
                let mut worker = FindWindowExW(
                    std::ptr::null_mut(),
                    std::ptr::null_mut(),
                    worker_class.as_ptr(),
                    std::ptr::null(),
                );
                while !worker.is_null() && defview.is_null() {
                    defview =
                        FindWindowExW(worker, std::ptr::null_mut(), defview_class.as_ptr(), std::ptr::null());
                    worker = FindWindowExW(
                        std::ptr::null_mut(),
                        worker,
                        worker_class.as_ptr(),
                        std::ptr::null(),
                    );
                }
            }
            if defview.is_null() {
                None
            } else {
                Some(defview)
            }
        }
    }

    pub fn desktop_listview() -> Option<HWND> {
        let defview = desktop_defview()?;
        unsafe {
            let list_class = wide("SysListView32");
            let list_name = wide("FolderView");
            let list = FindWindowExW(defview, std::ptr::null_mut(), list_class.as_ptr(), list_name.as_ptr());
            if list.is_null() {
                None
            } else {
                Some(list)
            }
        }
    }

    fn wide(text: &str) -> Vec<u16> {
        text.encode_utf16().chain(std::iter::once(0)).collect()
    }

    /// 与列表窗口所在进程建立的远程内存通道
    struct Remote {
        handle: *mut core::ffi::c_void,
    }

    impl Remote {
        fn open(list: HWND) -> Option<Remote> {
            unsafe {
                let mut pid = 0u32;
                GetWindowThreadProcessId(list, &mut pid);
                if pid == 0 {
                    return None;
                }
                let handle = OpenProcess(
                    PROCESS_VM_OPERATION | PROCESS_VM_READ | PROCESS_VM_WRITE,
                    0,
                    pid,
                );
                if handle.is_null() {
                    return None;
                }
                Some(Remote { handle })
            }
        }

        unsafe fn alloc(&self, size: usize) -> *mut u8 {
            VirtualAllocEx(
                self.handle,
                std::ptr::null(),
                size,
                MEM_COMMIT | MEM_RESERVE,
                PAGE_READWRITE,
            ) as *mut u8
        }

        unsafe fn write(&self, remote: *mut u8, data: &[u8]) -> bool {
            let mut written = 0usize;
            WriteProcessMemory(self.handle, remote as *const _, data.as_ptr() as *const _, data.len(), &mut written) != 0
        }

        unsafe fn read(&self, remote: *const u8, out: &mut [u8]) -> bool {
            let mut read = 0usize;
            ReadProcessMemory(self.handle, remote as *const _, out.as_mut_ptr() as *mut _, out.len(), &mut read) != 0
        }

        unsafe fn free(&self, remote: *mut u8) {
            if !remote.is_null() {
                VirtualFreeEx(self.handle, remote as *mut _, 0, MEM_RELEASE);
            }
        }
    }

    impl Drop for Remote {
        fn drop(&mut self) {
            unsafe { CloseHandle(self.handle) };
        }
    }


    /// 读取全部桌面图标（名称 + 当前位置）
    pub fn read_desktop_icons() -> Vec<DesktopIcon> {
        let Some(list) = desktop_listview() else {
            return Vec::new();
        };
        unsafe {
            let count = SendMessageW(list, LVM_GETITEMCOUNT, 0, 0) as i32;
            let Some(remote) = Remote::open(list) else {
                return Vec::new();
            };
            let text_buf: *mut u8 = remote.alloc(1024);
            let lvi_buf: *mut u8 = remote.alloc(128);
            let pos_buf: *mut u8 = remote.alloc(8);
            if text_buf.is_null() || lvi_buf.is_null() || pos_buf.is_null() {
                remote.free(text_buf);
                remote.free(lvi_buf);
                remote.free(pos_buf);
                return Vec::new();
            }

            let mut icons = Vec::new();
            for index in 0..count {
                // 名称
                let mut lvi = [0u8; 128];
                let text_ptr = text_buf as usize as u64;
                lvi[0..4].copy_from_slice(&LVIF_TEXT.to_ne_bytes()); // mask
                lvi[4..8].copy_from_slice(&index.to_ne_bytes()); // iItem
                lvi[8..12].copy_from_slice(&0i32.to_ne_bytes()); // iSubItem
                // state/stateMask 保持 0
                lvi[24..32].copy_from_slice(&text_ptr.to_ne_bytes()); // pszText
                lvi[32..36].copy_from_slice(&511i32.to_ne_bytes()); // cchTextMax
                if !remote.write(lvi_buf, &lvi) {
                    continue;
                }
                SendMessageW(list, LVM_GETITEMTEXTW, index as usize, lvi_buf as usize as isize);
                let mut text_raw = [0u8; 1024];
                if !remote.read(text_buf, &mut text_raw) {
                    continue;
                }
                let wide_text: Vec<u16> = text_raw
                    .chunks_exact(2)
                    .take_while(|p| *p != [0, 0])
                    .map(|p| u16::from_ne_bytes([p[0], p[1]]))
                    .collect();
                let name = String::from_utf16_lossy(&wide_text);

                // 位置
                SendMessageW(list, LVM_GETITEMPOSITION, index as usize, pos_buf as usize as isize);
                let mut pos_raw = [0u8; 8];
                if !remote.read(pos_buf, &mut pos_raw) {
                    continue;
                }
                let x = i32::from_ne_bytes([pos_raw[0], pos_raw[1], pos_raw[2], pos_raw[3]]);
                let y = i32::from_ne_bytes([pos_raw[4], pos_raw[5], pos_raw[6], pos_raw[7]]);

                icons.push(DesktopIcon { index, name, x, y });
            }

            remote.free(text_buf);
            remote.free(lvi_buf);
            remote.free(pos_buf);
            icons
        }
    }

    /// 移动桌面图标到指定位置
    pub fn set_icon_pos(index: i32, x: i32, y: i32) -> bool {
        let Some(list) = desktop_listview() else {
            return false;
        };
        unsafe {
            let lparam = ((y as u16 as usize) << 16) | (x as u16 as usize);
            SendMessageW(list, LVM_SETITEMPOSITION, index as usize, lparam as isize) != 0
        }
    }

    /// LVM_SETITEMPOSITION32（指针传参版本，用于诊断）
    pub fn set_icon_pos32(index: i32, x: i32, y: i32) -> bool {
        use windows_sys::Win32::Foundation::POINT;
        let Some(list) = desktop_listview() else {
            return false;
        };
        unsafe {
            let Some(remote) = Remote::open(list) else {
                return false;
            };
            let buf: *mut u8 = remote.alloc(8);
            if buf.is_null() {
                return false;
            }
            let mut pt = POINT { x, y };
            let ok = remote.write(
                buf,
                std::slice::from_raw_parts(
                    &mut pt as *mut POINT as *const u8,
                    8,
                ),
            );
            let sent = SendMessageW(list, LVM_FIRST + 17, index as usize, buf as usize as isize);
            remote.free(buf);
            ok && sent != 0
        }
    }

    /// 系统桌面图标设置尺寸（96-DPI 参考值，即逻辑像素）。
    /// 来源：HKCU\...\Bags\Desktop\IconSize，系统按 DPI 缩放后绘制。
    /// 注意：不要用 LVM_GETITEMRECT 读——矩形是含间距的单元格，数值虚高。
    pub fn system_icon_size_logical() -> Option<f32> {
        use windows_sys::Win32::System::Registry::{
            RegGetValueW, HKEY_CURRENT_USER, RRF_RT_REG_DWORD,
        };
        let subkey: Vec<u16> = "Software\\Microsoft\\Windows\\Shell\\Bags\\1\\Desktop"
            .encode_utf16()
            .chain(std::iter::once(0))
            .collect();
        let value: Vec<u16> = "IconSize".encode_utf16().chain(std::iter::once(0)).collect();
        let mut data: u32 = 0;
        let mut size = std::mem::size_of::<u32>() as u32;
        let ok = unsafe {
            RegGetValueW(
                HKEY_CURRENT_USER,
                subkey.as_ptr(),
                value.as_ptr(),
                RRF_RT_REG_DWORD,
                std::ptr::null_mut(),
                &mut data as *mut u32 as *mut _,
                &mut size,
            )
        };
        if ok == 0 && (24..=256).contains(&data) {
            Some(data as f32)
        } else {
            None
        }
    }

    use windows_sys::Win32::UI::WindowsAndMessaging::WINDOW_LONG_PTR_INDEX;
    const LVS_AUTOARRANGE: isize = 0x0100;
    const GWL_STYLE: i32 = -16;

    /// 桌面是否开启了“自动排列图标”
    pub fn auto_arrange_enabled() -> bool {
        let Some(list) = desktop_listview() else {
            return false;
        };
        unsafe {
            let style = GetWindowLongPtrW(list, GWL_STYLE as WINDOW_LONG_PTR_INDEX);
            (style & LVS_AUTOARRANGE) != 0
        }
    }

    /// 切换桌面图标的显示/隐藏。
    /// Win11 新版本必须发给 SHELLDLL_DefView（发给 SysListView32 无效）。
    pub fn toggle_view() -> Result<(), String> {
        use windows_sys::Win32::UI::WindowsAndMessaging::WM_COMMAND;
        let Some(defview) = desktop_defview() else {
            return Err("未找到桌面视图窗口".to_string());
        };
        unsafe {
            SendMessageW(defview, WM_COMMAND, 0x7402, 0);
            eprintln!("[toggle] defview={:?}", defview);
        }
        Ok(())
    }

    /// 把窗口排到普通层级的最底部（类似桌面图标层）
    pub fn send_to_bottom(hwnd: isize) {
        use windows_sys::Win32::UI::WindowsAndMessaging::{
            SetWindowPos, HWND_BOTTOM, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE,
        };
        unsafe {
            SetWindowPos(
                hwnd as HWND,
                HWND_BOTTOM,
                0,
                0,
                0,
                0,
                SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE,
            );
        }
    }
}

#[cfg(windows)]
pub use platform::*;

// ---------- 非 Windows 占位 ----------

#[cfg(not(windows))]
#[derive(Debug, Clone)]
pub struct DesktopIcon {
    pub index: i32,
    pub name: String,
    pub x: i32,
    pub y: i32,
}

#[cfg(not(windows))]
#[cfg(not(windows))]
pub fn read_desktop_icons() -> Vec<DesktopIcon> {
    Vec::new()
}

#[cfg(not(windows))]
pub fn set_icon_pos(_index: i32, _x: i32, _y: i32) -> bool {
    false
}

#[cfg(not(windows))]
pub fn icon_size_px() -> Option<f32> {
    None
}

#[cfg(not(windows))]
pub fn toggle_view() -> Result<(), String> {
    Err("该功能仅支持 Windows".to_string())
}

#[cfg(not(windows))]
pub fn send_to_bottom(_hwnd: isize) {}
