use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

#[derive(Debug, Clone)]
pub enum TrayCommand {
    ShowMainWindow,
    HideMainWindow,
    ToggleMainWindow,
    OpenSettings,
    ScanDesktop,
    ToggleLock,
    ToggleDesktopIcons,
    RestoreDesktopIcons,
    Exit,
}

#[derive(Clone, Default)]
pub struct TrayController {
    queue: Arc<Mutex<VecDeque<TrayCommand>>>,
}

impl TrayController {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn push(&self, command: TrayCommand) {
        if let Ok(mut queue) = self.queue.lock() {
            queue.push_back(command);
        }
    }

    pub fn drain(&self) -> Vec<TrayCommand> {
        if let Ok(mut queue) = self.queue.lock() {
            queue.drain(..).collect()
        } else {
            Vec::new()
        }
    }
}

#[cfg(windows)]
mod platform {
    use super::{TrayCommand, TrayController};
    use std::mem::size_of;
    use std::ptr::null_mut;
    use std::sync::OnceLock;
    use std::thread;
    use windows_sys::Win32::Foundation::{HINSTANCE, HWND, LPARAM, LRESULT, POINT, WPARAM};
    use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
    use windows_sys::Win32::UI::Shell::{
        Shell_NotifyIconW, NIF_ICON, NIF_MESSAGE, NIF_TIP, NIM_ADD, NIM_DELETE, NIM_SETVERSION,
        NOTIFYICONDATAW, NOTIFYICON_VERSION_4,
    };
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::RegisterHotKey;
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        AppendMenuW, CreatePopupMenu, CreateWindowExW, DefWindowProcW, DestroyMenu,
        DispatchMessageW, GetCursorPos, GetMessageW, LoadIconW, PostMessageW, PostQuitMessage,
        RegisterClassW, SetForegroundWindow, TrackPopupMenu, TranslateMessage, HWND_MESSAGE,
        IDI_APPLICATION, MF_SEPARATOR, MF_STRING, MSG, TPM_BOTTOMALIGN, TPM_LEFTALIGN,
        TPM_NONOTIFY, TPM_RETURNCMD, WM_APP, WM_COMMAND, WM_CONTEXTMENU, WM_DESTROY,
        WM_LBUTTONDBLCLK, WM_LBUTTONUP, WM_NULL, WM_RBUTTONUP, WNDCLASSW,
    };

    const TRAY_CALLBACK_MESSAGE: u32 = WM_APP + 1;
    const TRAY_ICON_ID: u32 = 1;
    const MENU_SHOW: u16 = 1001;
    const MENU_HIDE: u16 = 1002;
    const MENU_SCAN: u16 = 1003;
    const MENU_SETTINGS: u16 = 1004;
    const MENU_TOGGLE_LOCK: u16 = 1005;
    const MENU_TOGGLE_DESKTOP_ICONS: u16 = 1006;
    const MENU_RESTORE_ICONS: u16 = 1007;
    const MENU_EXIT: u16 = 1008;
    const WM_HOTKEY: u32 = 0x0312;
    const HOTKEY_TOGGLE_EDIT: u16 = 0xE001;
    const MOD_ALT: u32 = 0x0001;
    const MOD_CONTROL: u32 = 0x0002;
    const VK_E: u32 = 0x45;
    const WINDOW_CLASS_NAME: &str = "deskman_tray_window";

    static TRAY_CONTROLLER: OnceLock<TrayController> = OnceLock::new();

    pub fn start(controller: TrayController) -> Result<(), String> {
        TRAY_CONTROLLER
            .set(controller)
            .map_err(|_| "托盘控制器已初始化".to_string())?;
        thread::spawn(|| {
            if let Err(err) = run_tray_message_loop() {
                eprintln!("警告：托盘初始化失败 - {}", err);
            }
        });
        Ok(())
    }

    fn run_tray_message_loop() -> Result<(), String> {
        unsafe {
            let instance = GetModuleHandleW(std::ptr::null());
            if instance.is_null() {
                return Err("无法获取模块句柄".to_string());
            }

            let class_name = wide(WINDOW_CLASS_NAME);
            let wnd_class = WNDCLASSW {
                style: 0,
                lpfnWndProc: Some(tray_window_proc),
                cbClsExtra: 0,
                cbWndExtra: 0,
                hInstance: instance as HINSTANCE,
                hIcon: std::ptr::null_mut(),
                hCursor: std::ptr::null_mut(),
                hbrBackground: std::ptr::null_mut(),
                lpszMenuName: std::ptr::null(),
                lpszClassName: class_name.as_ptr(),
            };
            if RegisterClassW(&wnd_class) == 0 {
                return Err("注册托盘窗口类失败".to_string());
            }

            let hwnd = CreateWindowExW(
                0,
                class_name.as_ptr(),
                wide("deskman-tray").as_ptr(),
                0,
                0,
                0,
                0,
                0,
                HWND_MESSAGE,
                std::ptr::null_mut(),
                instance as HINSTANCE,
                null_mut(),
            );
            if hwnd.is_null() {
                return Err("创建托盘消息窗口失败".to_string());
            }

            let mut nid = create_notify_icon_data(hwnd);
            if Shell_NotifyIconW(NIM_ADD, &mut nid) == 0 {
                return Err("添加系统托盘图标失败".to_string());
            }
            nid.Anonymous.uVersion = NOTIFYICON_VERSION_4;
            Shell_NotifyIconW(NIM_SETVERSION, &mut nid);

            // 全局热键 Ctrl+Alt+E：切换编辑模式
            RegisterHotKey(hwnd, HOTKEY_TOGGLE_EDIT as i32, MOD_CONTROL | MOD_ALT, VK_E);

            let mut message: MSG = std::mem::zeroed();
            while GetMessageW(&mut message, std::ptr::null_mut(), 0, 0) > 0 {
                TranslateMessage(&message);
                DispatchMessageW(&message);
            }

            Shell_NotifyIconW(NIM_DELETE, &mut nid);
            Ok(())
        }
    }

    unsafe extern "system" fn tray_window_proc(
        hwnd: HWND,
        msg: u32,
        wparam: WPARAM,
        lparam: LPARAM,
    ) -> LRESULT {
        match msg {
            TRAY_CALLBACK_MESSAGE => {
                match lparam as u32 {
                    WM_LBUTTONUP => push(TrayCommand::ToggleMainWindow),
                    WM_LBUTTONDBLCLK => push(TrayCommand::ShowMainWindow),
                    WM_RBUTTONUP | WM_CONTEXTMENU => show_tray_menu(hwnd),
                    _ => {}
                }
                0
            }
            WM_COMMAND => {
                match (wparam & 0xFFFF) as u16 {
                    MENU_SHOW => push(TrayCommand::ShowMainWindow),
                    MENU_HIDE => push(TrayCommand::HideMainWindow),
                    MENU_SCAN => push(TrayCommand::ScanDesktop),
                    MENU_SETTINGS => push(TrayCommand::OpenSettings),
                    MENU_TOGGLE_LOCK => push(TrayCommand::ToggleLock),
                    MENU_TOGGLE_DESKTOP_ICONS => push(TrayCommand::ToggleDesktopIcons),
                    MENU_RESTORE_ICONS => push(TrayCommand::RestoreDesktopIcons),
                    MENU_EXIT => push(TrayCommand::Exit),
                    _ => {}
                }
                0
            }
            WM_HOTKEY => {
                // Ctrl+Alt+E：切换编辑模式
                if (wparam & 0xFFFF) as u16 == HOTKEY_TOGGLE_EDIT {
                    push(TrayCommand::ToggleLock);
                }
                0
            }
            WM_DESTROY => {
                PostQuitMessage(0);
                0
            }
            _ => DefWindowProcW(hwnd, msg, wparam, lparam),
        }
    }

    unsafe fn show_tray_menu(hwnd: HWND) {
        let menu = CreatePopupMenu();
        if menu.is_null() {
            return;
        }

        let _ = AppendMenuW(
            menu,
            MF_STRING,
            MENU_SHOW as usize,
            wide("显示主界面").as_ptr(),
        );
        let _ = AppendMenuW(
            menu,
            MF_STRING,
            MENU_HIDE as usize,
            wide("隐藏主界面").as_ptr(),
        );
        let _ = AppendMenuW(menu, MF_SEPARATOR, 0, std::ptr::null());
        let _ = AppendMenuW(
            menu,
            MF_STRING,
            MENU_SCAN as usize,
            wide("扫描桌面").as_ptr(),
        );
        let _ = AppendMenuW(
            menu,
            MF_STRING,
            MENU_SETTINGS as usize,
            wide("打开设置").as_ptr(),
        );
        let _ = AppendMenuW(
            menu,
            MF_STRING,
            MENU_TOGGLE_LOCK as usize,
            wide("切换锁定").as_ptr(),
        );
        let _ = AppendMenuW(
            menu,
            MF_STRING,
            MENU_TOGGLE_DESKTOP_ICONS as usize,
            wide("切换桌面图标显示").as_ptr(),
        );
        let _ = AppendMenuW(
            menu,
            MF_STRING,
            MENU_RESTORE_ICONS as usize,
            wide("恢复桌面图标(原位)").as_ptr(),
        );
        let _ = AppendMenuW(menu, MF_SEPARATOR, 0, std::ptr::null());
        let _ = AppendMenuW(
            menu,
            MF_STRING,
            MENU_EXIT as usize,
            wide("退出 deskman").as_ptr(),
        );

        let mut point = POINT { x: 0, y: 0 };
        GetCursorPos(&mut point);
        SetForegroundWindow(hwnd);
        let selected = TrackPopupMenu(
            menu,
            TPM_BOTTOMALIGN | TPM_LEFTALIGN | TPM_NONOTIFY | TPM_RETURNCMD,
            point.x,
            point.y,
            0,
            hwnd,
            std::ptr::null(),
        );
        if selected != 0 {
            let _ = PostMessageW(hwnd, WM_COMMAND, selected as usize, 0);
        }
        DestroyMenu(menu);
        let _ = PostMessageW(hwnd, WM_NULL, 0, 0);
    }

    fn create_notify_icon_data(hwnd: HWND) -> NOTIFYICONDATAW {
        let mut data = NOTIFYICONDATAW {
            cbSize: size_of::<NOTIFYICONDATAW>() as u32,
            hWnd: hwnd,
            uID: TRAY_ICON_ID,
            uFlags: NIF_MESSAGE | NIF_ICON | NIF_TIP,
            uCallbackMessage: TRAY_CALLBACK_MESSAGE,
            hIcon: unsafe { LoadIconW(std::ptr::null_mut(), IDI_APPLICATION) },
            ..unsafe { std::mem::zeroed() }
        };
        copy_tooltip(&mut data.szTip, "deskman - 桌面分类整理器");
        data
    }

    fn copy_tooltip(buf: &mut [u16], text: &str) {
        let mut encoded: Vec<u16> = text.encode_utf16().collect();
        if buf.is_empty() {
            return;
        }
        let max_len = buf.len().saturating_sub(1);
        if encoded.len() > max_len {
            encoded.truncate(max_len);
        }
        for (index, value) in encoded.iter().enumerate() {
            buf[index] = *value;
        }
        buf[encoded.len()] = 0;
    }

    fn wide(text: &str) -> Vec<u16> {
        text.encode_utf16().chain(std::iter::once(0)).collect()
    }

    fn push(command: TrayCommand) {
        if let Some(controller) = TRAY_CONTROLLER.get() {
            controller.push(command);
        }
    }
}

#[cfg(windows)]
pub use platform::start;

#[cfg(not(windows))]
pub fn start(_controller: TrayController) -> Result<(), String> {
    Ok(())
}
