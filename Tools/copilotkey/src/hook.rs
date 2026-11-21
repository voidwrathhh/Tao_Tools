//! 全局低级键盘钩子（WH_KEYBOARD_LL）与守护进程消息循环。

use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::mpsc::{self, Sender};
use std::sync::OnceLock;
use std::thread;
use std::time::{Duration, Instant};

use windows_sys::Win32::Foundation::{
    CloseHandle, GetLastError, ERROR_ALREADY_EXISTS, HANDLE, LPARAM, LRESULT, WPARAM, WAIT_FAILED,
    WAIT_OBJECT_0,
};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::System::SystemInformation::GetTickCount64;
use windows_sys::Win32::System::Threading::{
    CreateEventW, CreateMutexW, OpenEventW, OpenMutexW, SetEvent, EVENT_MODIFY_STATE,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    CallNextHookEx, DispatchMessageW, MsgWaitForMultipleObjectsEx, PeekMessageW, SetWindowsHookExW,
    TranslateMessage, UnhookWindowsHookEx, HHOOK, KBDLLHOOKSTRUCT, LLKHF_INJECTED, MSG,
    MWMO_INPUTAVAILABLE, PM_REMOVE, QS_ALLINPUT, WH_KEYBOARD_LL, WM_KEYDOWN, WM_QUIT, WM_SYSKEYDOWN,
};

use crate::config::Config;
use crate::keys;
use crate::launch;
use crate::util;

const MUTEX_NAME: &str = r"Local\TaoToolsCopilotKeyMutex";
const STOP_EVENT: &str = r"Local\TaoToolsCopilotKeyStop";
const SYNCHRONIZE: u32 = 0x0010_0000;
const INFINITE: u32 = 0xFFFF_FFFF;

static TRIGGER_VK: AtomicU32 = AtomicU32::new(0);
static REQUIRED_MODS: AtomicU32 = AtomicU32::new(0);
static STRICT_MODS: AtomicBool = AtomicBool::new(false);
static SWALLOW: AtomicBool = AtomicBool::new(true);
static IGNORE_INJECTED: AtomicBool = AtomicBool::new(false);
static DEBOUNCE_MS: AtomicU64 = AtomicU64::new(600);
static LAST_FIRE: AtomicU64 = AtomicU64::new(0);
static MOD_STATE: AtomicU32 = AtomicU32::new(0);
static LAUNCH_TX: OnceLock<Sender<()>> = OnceLock::new();

// learn / trace 模式
static LEARN: AtomicBool = AtomicBool::new(false);
static LEARN_HIT: AtomicBool = AtomicBool::new(false);
static LEARN_VK: AtomicU32 = AtomicU32::new(0);
static LEARN_SCAN: AtomicU32 = AtomicU32::new(0);
static LEARN_FLAGS: AtomicU32 = AtomicU32::new(0);
static LEARN_MODS: AtomicU32 = AtomicU32::new(0);
static TRACE: AtomicBool = AtomicBool::new(false);
static TRACE_TX: OnceLock<Sender<TraceEvent>> = OnceLock::new();

/// learn 捕获到的按键信息。
pub struct LearnedKey {
    pub vk: u32,
    pub scan: u32,
    pub mods: u32,
    pub flags: u32,
}

/// trace 模式记录的一次按键事件。
#[derive(Clone, Debug)]
pub struct TraceEvent {
    pub down: bool,
    pub vk: u32,
    pub scan: u32,
    pub flags: u32,
    pub mods: u32,
}

unsafe extern "system" fn hook_proc(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if code >= 0 && lparam != 0 {
        let data = &*(lparam as *const KBDLLHOOKSTRUCT);
        let vk = data.vkCode;
        let message = wparam as u32;
        let is_down = message == WM_KEYDOWN || message == WM_SYSKEYDOWN;

        if let Some(bit) = keys::modifier_bit(vk) {
            if is_down {
                MOD_STATE.fetch_or(bit, Ordering::Relaxed);
            } else {
                MOD_STATE.fetch_and(!bit, Ordering::Relaxed);
            }
        }

        if LEARN.load(Ordering::Relaxed) {
            if is_down && keys::modifier_bit(vk).is_none() {
                LEARN_VK.store(vk, Ordering::Relaxed);
                LEARN_SCAN.store(data.scanCode, Ordering::Relaxed);
                LEARN_FLAGS.store(data.flags, Ordering::Relaxed);
                LEARN_MODS.store(MOD_STATE.load(Ordering::Relaxed), Ordering::Relaxed);
                LEARN_HIT.store(true, Ordering::Relaxed);
            }
        } else if is_down && vk == TRIGGER_VK.load(Ordering::Relaxed) {
            let injected = (data.flags & LLKHF_INJECTED) != 0;
            let skip = injected && IGNORE_INJECTED.load(Ordering::Relaxed);
            let required = REQUIRED_MODS.load(Ordering::Relaxed);
            let mods_ok = !STRICT_MODS.load(Ordering::Relaxed)
                || (MOD_STATE.load(Ordering::Relaxed) & required) == required;
            if !skip && mods_ok {
                let now = GetTickCount64();
                let fresh = now.saturating_sub(LAST_FIRE.load(Ordering::Relaxed))
                    >= DEBOUNCE_MS.load(Ordering::Relaxed);
                if fresh {
                    LAST_FIRE.store(now, Ordering::Relaxed);
                    if let Some(sender) = LAUNCH_TX.get() {
                        let _ = sender.send(());
                    }
                }
                if SWALLOW.load(Ordering::Relaxed) {
                    // 返回 1 表示吞掉这个按键，系统不会再收到 F23，搜索面板也就不会弹出来。
                    return 1;
                }
            }
        }

        if TRACE.load(Ordering::Relaxed) {
            if let Some(sender) = TRACE_TX.get() {
                let _ = sender.send(TraceEvent {
                    down: is_down,
                    vk,
                    scan: data.scanCode,
                    flags: data.flags,
                    mods: MOD_STATE.load(Ordering::Relaxed),
                });
            }
        }
    }
    CallNextHookEx(std::ptr::null_mut(), code, wparam, lparam)
}

fn install_hook() -> Option<HHOOK> {
    let module = unsafe { GetModuleHandleW(std::ptr::null()) };
    let hook = unsafe { SetWindowsHookExW(WH_KEYBOARD_LL, Some(hook_proc), module, 0) };
    if hook.is_null() {
        None
    } else {
        Some(hook)
    }
}

/// 守护进程是否在运行。
pub fn is_running() -> bool {
    let name = util::wide(MUTEX_NAME);
    let handle = unsafe { OpenMutexW(SYNCHRONIZE, 0, name.as_ptr()) };
    if handle.is_null() {
        false
    } else {
        unsafe { CloseHandle(handle) };
        true
    }
}

/// 通知正在运行的守护进程退出。返回是否真的通知到了。
pub fn stop() -> bool {
    let name = util::wide(STOP_EVENT);
    let handle: HANDLE = unsafe { OpenEventW(EVENT_MODIFY_STATE, 0, name.as_ptr()) };
    if handle.is_null() {
        return false;
    }
    let ok = unsafe { SetEvent(handle) };
    unsafe { CloseHandle(handle) };
    ok != 0
}

/// 启动守护进程（阻塞在消息循环里，直到收到 stop 或 WM_QUIT）。
pub fn run(config: Config) -> i32 {
    let _ = util::ensure_dirs();
    util::set_logging(config.log);

    let mutex_name = util::wide(MUTEX_NAME);
    let mutex = unsafe { CreateMutexW(std::ptr::null(), 0, mutex_name.as_ptr()) };
    if mutex.is_null() {
        util::say(&format!("创建互斥量失败，错误码 {}\n", unsafe { GetLastError() }));
        return 1;
    }
    if unsafe { GetLastError() } == ERROR_ALREADY_EXISTS {
        util::say("已经有一个 copilotkey 守护进程在运行了。\n");
        return 0;
    }

    TRIGGER_VK.store(config.trigger_vk, Ordering::Relaxed);
    REQUIRED_MODS.store(config.trigger_mods, Ordering::Relaxed);
    STRICT_MODS.store(config.strict_mods, Ordering::Relaxed);
    SWALLOW.store(config.swallow, Ordering::Relaxed);
    IGNORE_INJECTED.store(config.ignore_injected, Ordering::Relaxed);
    DEBOUNCE_MS.store(config.debounce_ms, Ordering::Relaxed);

    let event_name = util::wide(STOP_EVENT);
    let stop_event = unsafe { CreateEventW(std::ptr::null(), 1, 0, event_name.as_ptr()) };
    if stop_event.is_null() {
        util::say(&format!("创建停止事件失败，错误码 {}\n", unsafe { GetLastError() }));
        return 1;
    }

    // 启动线程：钩子回调只负责投递信号，真正的启动动作在这里做。
    let (launch_tx, launch_rx) = mpsc::channel::<()>();
    let _ = LAUNCH_TX.set(launch_tx);
    let worker_config = config.clone();
    thread::spawn(move || {
        for _ in launch_rx {
            let snapshot = worker_config.clone();
            thread::spawn(move || match launch::launch(&snapshot) {
                Ok(()) => util::log(&format!(
                    "触发 → 启动 {}{}",
                    snapshot.target,
                    if snapshot.arguments.is_empty() {
                        String::new()
                    } else {
                        format!("（参数：{}）", snapshot.arguments)
                    }
                )),
                Err(err) => util::log(&format!("触发 → 失败：{}", err)),
            });
        }
    });

    let hook = match install_hook() {
        Some(hook) => hook,
        None => {
            util::say(&format!(
                "安装全局键盘钩子失败，错误码 {}\n",
                unsafe { GetLastError() }
            ));
            return 1;
        }
    };

    util::log(&format!(
        "守护进程启动：触发键 {}（vk=0x{:02X}），目标 {:?}，参数 {:?}，严格修饰键={}，拦截按键={}",
        config.label(),
        config.trigger_vk,
        config.target,
        config.arguments,
        config.strict_mods,
        config.swallow
    ));
    util::say("copilotkey 守护进程已启动。\n");
    util::say(&format!("  触发键 : {}\n", config.label()));
    util::say(&format!(
        "  目标   : {}\n",
        if config.target.is_empty() {
            "（未设置，可以用 copilotkey target 设置）"
        } else {
            config.target.as_str()
        }
    ));
    util::say(&format!("  日志   : {}\n", util::log_path().display()));
    util::say("按 Ctrl+C 或执行 copilotkey stop 可退出。\n");

    let handles = [stop_event];
    let mut message: MSG = unsafe { std::mem::zeroed() };
    let mut result = 0;
    'outer: loop {
        let waited = unsafe {
            MsgWaitForMultipleObjectsEx(1, handles.as_ptr(), INFINITE, QS_ALLINPUT, MWMO_INPUTAVAILABLE)
        };
        if waited == WAIT_OBJECT_0 {
            util::say("\n收到停止信号，正在退出……\n");
            break;
        }
        if waited == WAIT_OBJECT_0 + 1 {
            loop {
                let got = unsafe { PeekMessageW(&mut message, std::ptr::null_mut(), 0, 0, PM_REMOVE) };
                if got == 0 {
                    break;
                }
                if message.message == WM_QUIT {
                    break 'outer;
                }
                unsafe {
                    TranslateMessage(&message);
                    DispatchMessageW(&message);
                }
            }
            continue;
        }
        if waited == WAIT_FAILED {
            util::log("消息循环等待失败，退出");
            result = 1;
            break;
        }
    }

    unsafe { UnhookWindowsHookEx(hook) };
    util::log("守护进程退出");
    result
}

/// 捕获一次按键：返回按下的主键及其修饰键。
pub fn learn(timeout: Duration) -> Option<LearnedKey> {
    LEARN_HIT.store(false, Ordering::Relaxed);
    MOD_STATE.store(0, Ordering::Relaxed);
    LEARN.store(true, Ordering::Relaxed);
    let hook = match install_hook() {
        Some(hook) => hook,
        None => {
            LEARN.store(false, Ordering::Relaxed);
            return None;
        }
    };

    let started = Instant::now();
    let mut message: MSG = unsafe { std::mem::zeroed() };
    while started.elapsed() < timeout {
        unsafe {
            while PeekMessageW(&mut message, std::ptr::null_mut(), 0, 0, PM_REMOVE) != 0 {
                TranslateMessage(&message);
                DispatchMessageW(&message);
            }
        }
        if LEARN_HIT.load(Ordering::Relaxed) {
            break;
        }
        thread::sleep(Duration::from_millis(15));
    }

    LEARN.store(false, Ordering::Relaxed);
    unsafe { UnhookWindowsHookEx(hook) };

    if LEARN_HIT.load(Ordering::Relaxed) {
        Some(LearnedKey {
            vk: LEARN_VK.load(Ordering::Relaxed),
            scan: LEARN_SCAN.load(Ordering::Relaxed),
            mods: LEARN_MODS.load(Ordering::Relaxed),
            flags: LEARN_FLAGS.load(Ordering::Relaxed),
        })
    } else {
        None
    }
}

/// 调试模式：把接下来一段时间内的所有按键事件都打印出来。
pub fn trace(timeout: Duration, mut on_event: impl FnMut(&TraceEvent)) {
    let (sender, receiver) = mpsc::channel::<TraceEvent>();
    let _ = TRACE_TX.set(sender);
    MOD_STATE.store(0, Ordering::Relaxed);
    TRACE.store(true, Ordering::Relaxed);

    let hook = match install_hook() {
        Some(hook) => hook,
        None => {
            TRACE.store(false, Ordering::Relaxed);
            util::say("安装全局键盘钩子失败，无法进入调试模式。\n");
            return;
        }
    };

    let started = Instant::now();
    let mut message: MSG = unsafe { std::mem::zeroed() };
    while started.elapsed() < timeout {
        unsafe {
            while PeekMessageW(&mut message, std::ptr::null_mut(), 0, 0, PM_REMOVE) != 0 {
                TranslateMessage(&message);
                DispatchMessageW(&message);
            }
        }
        while let Ok(event) = receiver.try_recv() {
            on_event(&event);
        }
        thread::sleep(Duration::from_millis(10));
    }

    while let Ok(event) = receiver.try_recv() {
        on_event(&event);
    }
    TRACE.store(false, Ordering::Relaxed);
    unsafe { UnhookWindowsHookEx(hook) };
}
