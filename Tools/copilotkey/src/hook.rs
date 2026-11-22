//! 全局低级键盘钩子（WH_KEYBOARD_LL）与守护进程消息循环。
//!
//! 运行时状态全部放在静态原子变量里，[`apply_config`] 是刷新它们的唯一入口：
//! 启动时调用一次，之后由轮询线程每 [`CONFIG_POLL_MS`] 毫秒重新读配置再调用，
//! 所以 `set` / `learn --save` 这类改动无需重启守护进程（1 秒内生效）。

use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::mpsc::{self, Sender};
use std::sync::{Mutex, OnceLock};
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

/// 配置轮询间隔：必须小于 1 秒，保证 `set` / `learn --save` 后 1 秒内生效。
const CONFIG_POLL_MS: u64 = 300;
/// 同一条配置错误最多每 30 秒记一次日志，避免把日志刷爆。
const CONFIG_ERROR_LOG_GAP: Duration = Duration::from_secs(30);

static TRIGGER_VK: AtomicU32 = AtomicU32::new(0);
static REQUIRED_MODS: AtomicU32 = AtomicU32::new(0);
static STRICT_MODS: AtomicBool = AtomicBool::new(false);
static SWALLOW: AtomicBool = AtomicBool::new(true);
static IGNORE_INJECTED: AtomicBool = AtomicBool::new(false);
static DEBOUNCE_MS: AtomicU64 = AtomicU64::new(600);
static LAST_FIRE: AtomicU64 = AtomicU64::new(0);
static STOPPING: AtomicBool = AtomicBool::new(false);
static LAUNCH_TX: OnceLock<Sender<()>> = OnceLock::new();

/// 守护进程内存里的 last-good 配置：解析失败时保留它，绝不回退默认值。
/// 触发时也从这里取目标，不再每次重新读文件（写入中途读不到半个文件）。
static CONFIG: OnceLock<Mutex<Config>> = OnceLock::new();

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

        // 修饰键不再累积到全局状态：判定时用 keys::live_mods() 现算。

        if LEARN.load(Ordering::Relaxed) {
            if is_down && keys::modifier_bit(vk).is_none() {
                LEARN_VK.store(vk, Ordering::Relaxed);
                LEARN_SCAN.store(data.scanCode, Ordering::Relaxed);
                LEARN_FLAGS.store(data.flags, Ordering::Relaxed);
                // 捕获瞬间现算修饰键，和真实按住的键一致（不受 UAC/安全桌面残留影响）。
                LEARN_MODS.store(keys::live_mods(), Ordering::Relaxed);
                LEARN_HIT.store(true, Ordering::Relaxed);
            }
        } else if is_down && vk == TRIGGER_VK.load(Ordering::Relaxed) {
            let injected = (data.flags & LLKHF_INJECTED) != 0;
            let skip = injected && IGNORE_INJECTED.load(Ordering::Relaxed);
            let strict = STRICT_MODS.load(Ordering::Relaxed);
            let required = REQUIRED_MODS.load(Ordering::Relaxed);
            // 精确匹配：判定瞬间现算修饰键，多按一个 Ctrl 也不触发、也不吞键。
            let live = if strict { keys::live_mods() } else { 0 };
            if !skip && keys::mods_match(strict, live, required) {
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
                    mods: keys::live_mods(),
                });
            }
        }
    }
    CallNextHookEx(std::ptr::null_mut(), code, wparam, lparam)
}

/// 用一份配置刷新钩子的全部运行时状态（键位、吞键、防抖、日志开关）。
///
/// 返回「键位相关字段是否发生变化」，供轮询线程决定要不要写一行“配置已热更新”。
/// 启动流程与配置轮询线程都必须走这里：这是唯一一处把配置写进静态变量的地方。
pub fn apply_config(config: &Config) -> bool {
    let changed = [
        TRIGGER_VK.swap(config.trigger_vk, Ordering::Relaxed) != config.trigger_vk,
        REQUIRED_MODS.swap(config.trigger_mods, Ordering::Relaxed) != config.trigger_mods,
        STRICT_MODS.swap(config.strict_mods, Ordering::Relaxed) != config.strict_mods,
        SWALLOW.swap(config.swallow, Ordering::Relaxed) != config.swallow,
        IGNORE_INJECTED.swap(config.ignore_injected, Ordering::Relaxed) != config.ignore_injected,
        DEBOUNCE_MS.swap(config.debounce_ms, Ordering::Relaxed) != config.debounce_ms,
    ]
    .into_iter()
    .any(|item| item);
    util::set_logging(config.log);
    changed
}

/// 把新配置装进运行时（更新内存 last-good + 刷新静态变量）。返回键位相关字段是否变化。
pub fn update_config(config: &Config) -> bool {
    if let Some(slot) = CONFIG.get() {
        let mut guard = slot.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        *guard = config.clone();
    }
    apply_config(config)
}

/// 守护进程内存里的 last-good 配置快照。
pub fn current_config() -> Option<Config> {
    CONFIG.get().map(|slot| {
        slot.lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
    })
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

/// 配置轮询线程：每 [`CONFIG_POLL_MS`] 毫秒读一次配置并应用，实现真正的键位热重载。
fn spawn_config_watcher() {
    thread::spawn(|| {
        let mut last_error: Option<String> = None;
        let mut last_error_at = Instant::now();
        loop {
            thread::sleep(Duration::from_millis(CONFIG_POLL_MS));
            if STOPPING.load(Ordering::Relaxed) {
                break;
            }
            match Config::load() {
                Ok(Some(fresh)) => {
                    last_error = None;
                    if update_config(&fresh) {
                        util::log(&format!(
                            "配置已热更新：触发键 {}（vk=0x{:02X}，修饰键=0x{:X}），严格修饰键={}，拦截按键={}，防抖={}ms",
                            fresh.label(),
                            fresh.trigger_vk,
                            fresh.trigger_mods,
                            fresh.strict_mods,
                            fresh.swallow,
                            fresh.debounce_ms
                        ));
                    }
                }
                // 文件被删除：保留上一次有效配置，等用户重新写入。
                Ok(None) => last_error = None,
                Err(err) => {
                    // 解析失败：保留 last-good，绝不回退默认值；同一错误限流记录。
                    let same = last_error.as_deref() == Some(err.as_str());
                    if !same || last_error_at.elapsed() >= CONFIG_ERROR_LOG_GAP {
                        util::log(&format!("配置已损坏，正在使用上次有效配置：{}", err));
                        last_error = Some(err);
                        last_error_at = Instant::now();
                    }
                }
            }
        }
    });
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

    STOPPING.store(false, Ordering::Relaxed);
    let _ = CONFIG.set(Mutex::new(config.clone()));
    apply_config(&config);

    let event_name = util::wide(STOP_EVENT);
    let stop_event = unsafe { CreateEventW(std::ptr::null(), 1, 0, event_name.as_ptr()) };
    if stop_event.is_null() {
        util::say(&format!("创建停止事件失败，错误码 {}\n", unsafe { GetLastError() }));
        return 1;
    }

    // 启动线程：钩子回调只负责投递信号，真正的启动动作在这里做。
    let (launch_tx, launch_rx) = mpsc::channel::<()>();
    let _ = LAUNCH_TX.set(launch_tx);
    thread::spawn(move || {
        for _ in launch_rx {
            // 用内存里的 last-good 配置：轮询线程负责刷新，触发瞬间不读文件，
            // 不会出现“配置写到一半读到空 target”的情况。
            let Some(snapshot) = current_config() else {
                continue;
            };
            thread::spawn(move || {
                let target = snapshot.target.clone();
                let args = snapshot.arguments.clone();
                match launch::launch(&snapshot) {
                    Ok(()) => util::log(&format!(
                        "触发 → 启动 {}{}",
                        target,
                        if args.is_empty() {
                            String::new()
                        } else {
                            format!("（参数：{}）", args)
                        }
                    )),
                    Err(err) => util::log(&format!("触发 → 失败：{}", err)),
                }
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

    // 钩子装好后再开轮询：启动失败就直接返回，不留后台线程。
    spawn_config_watcher();

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

    STOPPING.store(true, Ordering::Relaxed);
    unsafe { UnhookWindowsHookEx(hook) };
    util::log("守护进程退出");
    result
}

/// 捕获一次按键：返回按下的主键及其修饰键。
pub fn learn(timeout: Duration) -> Option<LearnedKey> {
    LEARN_HIT.store(false, Ordering::Relaxed);
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

#[cfg(test)]
mod tests {
    use super::*;

    /// 合成一条测试：apply_config / update_config 改的是进程级静态变量，
    /// 拆成多个 #[test] 并行跑会互相覆盖，故意放同一个测试里顺序执行。
    #[test]
    fn apply_config_refreshes_every_runtime_field_and_keeps_last_good() {
        let mut config = Config::default();
        config.trigger_vk = 0x71;
        config.trigger_mods = 0b0110;
        config.strict_mods = true;
        config.swallow = false;
        config.ignore_injected = true;
        config.debounce_ms = 123;
        config.log = false;
        config.target = "first.exe".to_string();

        assert!(apply_config(&config), "首次应用应报告发生变化");
        assert_eq!(TRIGGER_VK.load(Ordering::Relaxed), 0x71);
        assert_eq!(REQUIRED_MODS.load(Ordering::Relaxed), 0b0110);
        assert!(STRICT_MODS.load(Ordering::Relaxed));
        assert!(!SWALLOW.load(Ordering::Relaxed));
        assert!(IGNORE_INJECTED.load(Ordering::Relaxed));
        assert_eq!(DEBOUNCE_MS.load(Ordering::Relaxed), 123);

        // 同一份配置再应用一次：没有变化，不该刷“配置已热更新”日志。
        assert!(!apply_config(&config), "相同配置不应报告变化");

        // 换键位 → 报告变化，静态变量跟着变。
        config.trigger_vk = 0x86;
        config.trigger_mods = 0b0011;
        assert!(apply_config(&config));
        assert_eq!(TRIGGER_VK.load(Ordering::Relaxed), 0x86);
        assert_eq!(REQUIRED_MODS.load(Ordering::Relaxed), 0b0011);

        // 内存 last-good：目标这类“只在触发时用”的字段也要跟着更新。
        let _ = CONFIG.set(Mutex::new(config.clone()));
        update_config(&config);
        assert_eq!(current_config().unwrap().target, "first.exe");
        config.target = "second.exe".to_string();
        update_config(&config);
        assert_eq!(current_config().unwrap().target, "second.exe");
        assert_eq!(TRIGGER_VK.load(Ordering::Relaxed), config.trigger_vk);
    }

    #[test]
    fn poll_interval_is_fast_enough_for_the_one_second_promise() {
        assert!(CONFIG_POLL_MS <= 1000, "轮询间隔必须小于 1 秒");
        assert!(CONFIG_POLL_MS <= 500, "轮询间隔应远小于 1 秒，留出读写延迟余量");
    }
}
