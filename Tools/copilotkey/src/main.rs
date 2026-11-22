//! copilotkey：把 Copilot 键改造成全局自定义功能键。
//!
//! 原理：用 WH_KEYBOARD_LL 全局低级键盘钩子截获 Copilot 键（Windows 上报为
//! Win+Shift+F23），吞掉这个按键，改为启动用户配置的目标程序——系统不再收到
//! F23，也就不会再弹出 Copilot / 搜索面板。

// release 构建不弹控制台窗口（开机自启时用），需要输出时再附加到父控制台。
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

macro_rules! out {
    ($($arg:tt)*) => {{
        crate::util::say(&format!($($arg)*));
    }};
}

mod autostart;
mod config;
mod hook;
mod keys;
mod launch;
mod state;
mod util;

use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use config::Config;
use state::State;

fn main() {
    util::ensure_console();
    let args: Vec<String> = std::env::args().skip(1).collect();
    let code = dispatch(&args);
    std::process::exit(code);
}

fn dispatch(args: &[String]) -> i32 {
    let Some(command) = args.first() else {
        print_help();
        return 0;
    };
    let rest = &args[1..];
    match command.as_str() {
        "learn" => cmd_learn(rest),
        "trace" => cmd_trace(rest),
        "set" => cmd_set(rest),
        "target" => cmd_target(rest),
        "run" => cmd_run(),
        "install" => cmd_install(),
        "uninstall" => cmd_uninstall(),
        "status" => cmd_status(),
        "stop" => cmd_stop(),
        "test" => cmd_test(),
        "help" | "-h" | "--help" => {
            print_help();
            0
        }
        "-V" | "--version" | "version" => {
            out!("copilotkey v{}\n", env!("CARGO_PKG_VERSION"));
            0
        }
        other => {
            out!("未知命令：{}\n\n", other);
            print_help();
            2
        }
    }
}

fn print_help() {
    out!("copilotkey v{} —— 把 Copilot 键变成你的自定义功能键\n", env!("CARGO_PKG_VERSION"));
    out!("\n用法：copilotkey <命令> [参数]\n\n");
    out!("命令：\n");
    out!("  trace [--seconds N]          调试：打印接下来收到的所有按键事件（看 Copilot 键到底上报了什么）\n");
    out!("  learn [--save] [--seconds N] 学习：按一次 Copilot 键，捕获它的键码（加 --save 直接保存）\n");
    out!("  set <组合键>                 手动设置触发键，例如 set win+shift+f23 或 set f23\n");
    out!("  target <程序> [参数...]      设置按下后要启动的目标\n");
    out!("       target --list           列出当前可用（已编译）的 tao_tools 工具\n");
    out!("       target --clear          清除目标\n");
    out!("  run                          前台运行守护进程（一般由 install 自动拉起）\n");
    out!("  install                      注册开机自启并启动守护进程（会轮询确认真的生效）\n");
    out!("  uninstall                    取消开机自启并停止守护进程\n");
    out!("  status                       查看配置、上次触发结果与运行状态\n");
    out!("  stop                         停止守护进程\n");
    out!("  test                         校验目标并注入一次按键，靠 state.json 确认真实触发\n");
    out!("\n示例：\n");
    out!("  copilotkey run target --list\n");
    out!("  copilotkey target deskman\n");
    out!("  copilotkey target \"C:\\Windows\\System32\\calc.exe\"\n");
    out!("  copilotkey install\n");
}

fn parse_seconds(args: &[String], default: u64) -> u64 {
    let mut index = 0;
    while index < args.len() {
        if args[index] == "--seconds" || args[index] == "--timeout" || args[index] == "-s" {
            if let Some(value) = args.get(index + 1).and_then(|item| item.parse().ok()) {
                return value;
            }
        }
        index += 1;
    }
    default
}

fn cmd_trace(args: &[String]) -> i32 {
    let seconds = parse_seconds(args, 15);
    out!("调试模式：接下来 {} 秒内打印所有按键事件，请按一次 Copilot 键。\n\n", seconds);
    let mut count = 0usize;
    hook::trace(Duration::from_secs(seconds), |event| {
        count += 1;
        let mods = keys::mods_label(event.mods);
        out!(
            "{} vk=0x{:02X} {:<14} scan=0x{:02X} flags=0x{:02X} 修饰={}{}\n",
            if event.down { "按下" } else { "抬起" },
            event.vk,
            keys::key_name(event.vk),
            event.scan,
            event.flags,
            if mods.is_empty() { "无" } else { mods.as_str() },
            if event.flags & 0x10 != 0 { " [注入]" } else { "" }
        );
    });
    out!("\n共收到 {} 个按键事件。\n", count);
    if count == 0 {
        out!("一个事件都没收到，说明钩子没生效（可能被别的程序抢占）。\n");
    }
    0
}

fn cmd_learn(args: &[String]) -> i32 {
    let save = args.iter().any(|item| item == "--save");
    let seconds = parse_seconds(args, 15);
    out!("请在 {} 秒内按下要绑定的那个键（也就是 Copilot 键）……\n\n", seconds);
    match hook::learn(Duration::from_secs(seconds)) {
        Some(key) => {
            let label = keys::combo_label(key.vk, key.mods);
            out!("\n捕获到按键：{}\n", label);
            out!("  虚拟键码 vk    = 0x{:02X}\n", key.vk);
            out!("  扫描码   scan  = 0x{:02X}\n", key.scan);
            out!("  修饰键   mods  = {}\n", {
                let mods = keys::mods_label(key.mods);
                if mods.is_empty() { "无".to_string() } else { mods }
            });
            out!("  标志     flags = 0x{:02X}{}\n", key.flags, if key.flags & 0x10 != 0 { "（注入事件）" } else { "" });
            if save {
                let mut config = config::load_or_exit();
                config.trigger_vk = key.vk;
                config.trigger_mods = key.mods;
                config.trigger_label = label.clone();
                // 学到的修饰键可能包含按 Copilot 键时顺手按住的其它键，默认放宽成只看主键。
                config.strict_mods = false;
                if let Err(err) = config.save() {
                    out!("\n保存失败：{}\n", err);
                    return 1;
                }
                out!("\n已保存，触发键 = {}\n", label);
                out!("（已设为只看主键：多按/少按修饰键也会触发；需要精确匹配请把 config.json 的 strict_mods 设为 true）\n");
                if hook::is_running() {
                    out!("守护进程会在 1 秒内热加载新键位，无需重启。\n");
                }
            } else {
                out!("\n（没有保存。确认无误后加 --save 再执行一次即可写入配置。）\n");
            }
            0
        }
        None => {
            out!("\n没有捕获到任何按键。\n");
            out!("可以先执行 copilotkey trace 再按一次，看看系统到底有没有收到这个键。\n");
            1
        }
    }
}

fn cmd_set(args: &[String]) -> i32 {
    let Some(raw) = args.first() else {
        out!("用法：copilotkey set win+shift+f23\n");
        return 2;
    };
    let (vk, mods) = match keys::parse_combo(raw) {
        Ok(value) => value,
        Err(err) => {
            out!("{}\n", err);
            return 2;
        }
    };
    let mut config = config::load_or_exit();
    config.trigger_vk = vk;
    config.trigger_mods = mods;
    config.trigger_label = keys::combo_label(vk, mods);
    if let Err(err) = config.save() {
        out!("保存失败：{}\n", err);
        return 1;
    }
    out!("触发键已设置为：{}\n", config.trigger_label);
    if config.strict_mods {
        out!("严格修饰：是（修饰键必须完全一致，多按/少按都不触发）\n");
    } else {
        out!("严格修饰：否 —— 只看主键，多按/少按修饰键都会触发（含 Ctrl+F23 这类误触）。\n");
        out!("要精确匹配，把配置文件里的 strict_mods 改成 true 即可（同样无需重启守护进程）。\n");
    }
    if hook::is_running() {
        out!("守护进程会在 1 秒内热加载新键位，无需重启。\n");
    }
    0
}

fn cmd_target(args: &[String]) -> i32 {
    let mut config = config::load_or_exit();

    if args.is_empty() {
        if config.target.is_empty() {
            out!("当前目标：（未设置）\n");
        } else {
            out!("当前目标：{}\n", config.target);
            if !config.arguments.is_empty() {
                out!("参    数：{}\n", config.arguments);
            }
        }
        return 0;
    }

    if args[0] == "--list" {
        return list_tools();
    }

    if args[0] == "--clear" {
        config.target.clear();
        config.arguments.clear();
        config.workdir.clear();
        if let Err(err) = config.save() {
            out!("保存失败：{}\n", err);
            return 1;
        }
        out!("已清除目标。\n");
        return 0;
    }

    let raw_target = args[0].clone();
    let mut rest: Vec<String> = Vec::new();
    let mut index = 1;
    while index < args.len() {
        if args[index] == "--args" {
            index += 1;
            continue;
        }
        rest.push(args[index].clone());
        index += 1;
    }
    let resolved = resolve_target(&raw_target);
    config.target = resolved.clone();
    config.arguments = rest.join(" ");
    if let Err(err) = config.save() {
        out!("保存失败：{}\n", err);
        return 1;
    }
    out!("目标已设置为：{}\n", resolved);
    if !config.arguments.is_empty() {
        out!("参        数：{}\n", config.arguments);
    }
    if resolved != raw_target {
        out!("（“{}” 已解析为上面的完整路径）\n", raw_target);
    } else {
        // 看着像工具名却没解析出来（没有路径分隔符、不是 .exe / URL），提醒一下。
        let looks_like_tool = !raw_target.contains('\\')
            && !raw_target.contains('/')
            && !raw_target.contains(':')
            && !raw_target.to_lowercase().ends_with(".exe");
        if looks_like_tool {
            out!(
                "注意：“{}” 没能解析成已编译的可执行文件，按下按键时会启动失败。\n用 copilotkey target --list 看看可用的工具，或直接写完整路径。\n",
                raw_target
            );
        }
    }
    if hook::is_running() {
        out!("守护进程会立即使用新目标，无需重启。\n");
    }
    0
}

fn cmd_run() -> i32 {
    hook::run(config::load_or_exit())
}

fn cmd_install() -> i32 {
    let exe = match std::env::current_exe() {
        Ok(path) => path,
        Err(err) => {
            out!("无法获取自身路径：{}\n", err);
            return 1;
        }
    };
    let exe_text = exe.display().to_string();
    if exe_text.to_lowercase().contains(r"\target\debug\") {
        out!("注意：当前是 debug 构建，自启动会指向这个调试版本。\n");
        out!("      正式使用建议：cargo build --release -p copilotkey\n\n");
    }

    // 只在配置文件不存在时写入：install 绝不能把默认值覆盖到用户已有的配置上。
    let config = config::load_or_exit();
    if !util::config_path().exists() {
        if let Err(err) = config.save() {
            out!("写入初始配置失败：{}\n", err);
            return 1;
        }
    }

    // 1) 注册表：写完立刻回读确认，不假设 RegSetValueExW 返回 0 就等于值是对的。
    let expected_value = format!("\"{}\" run", exe_text);
    if let Err(err) = autostart::install(&exe_text) {
        out!("注册开机自启失败：{}\n", err);
        return 1;
    }
    let confirmed = poll_until(Duration::from_secs(2), Duration::from_millis(50), || {
        autostart::query().as_deref() == Some(expected_value.as_str())
    });
    if !confirmed {
        out!(
            "开机自启写入后回读不一致：期望 {}，实际 {}。\n",
            expected_value,
            autostart::query().unwrap_or_else(|| "（未设置）".to_string())
        );
        return 1;
    }
    out!("已注册开机自启（已回读确认）：HKCU\\Software\\Microsoft\\Windows\\CurrentVersion\\Run\\TaoToolsCopilotKey\n");

    // 记下“上一次”的状态：后面用它确认新进程真的换了 pid / 启动时间。
    let previous = state::load().ok().flatten();

    // 2) 停旧进程：轮询等它真的退出，不能 sleep 完就当它退了。
    if hook::is_running() {
        if !hook::stop() {
            out!("通知旧守护进程退出失败。\n");
            return 1;
        }
        if !poll_until(Duration::from_secs(3), Duration::from_millis(100), || {
            !hook::is_running()
        }) {
            out!("旧守护进程 3 秒内没有退出，为免两个进程抢钩子已中止。\n");
            out!("请手动结束它后重试：taskkill /F /IM copilotkey.exe\n");
            return 1;
        }
        out!("旧守护进程已退出。\n");
    }

    // 3) 拉起新进程并保留 Child 句柄：能立刻发现它提前退出（旧实现把句柄丢了）。
    let mut child = match spawn_daemon(&exe) {
        Ok(child) => child,
        Err(err) => {
            out!("启动守护进程失败：{}\n", err);
            return 1;
        }
    };
    let child_pid = child.id();

    // 4) 双重确认：进程还在跑（is_running）+ state.json 显示“就是这个 pid、钩子已就绪”。
    match wait_daemon_ready(&mut child, child_pid, previous.as_ref()) {
        Ok(started) => {
            out!("守护进程已启动，全局钩子生效。\n");
            out!("  pid    : {}\n", started.pid);
            out!("  启动于 : {}\n", started.started_at);
            out!("现在按下 {} 就会启动目标程序，不再弹出搜索面板。\n", config.label());
        }
        Err(err) => {
            out!("{}\n", err);
            let tail = util::tail_log(15);
            if !tail.is_empty() {
                out!("\n最近的日志：\n{}\n", tail);
            }
            return 1;
        }
    }

    if config.target.is_empty() {
        out!("\n提示：还没有设置目标程序。执行 copilotkey target --list 看看有哪些可用工具。\n");
    }
    0
}

fn cmd_uninstall() -> i32 {
    match autostart::remove() {
        Ok(()) => out!("已取消开机自启。\n"),
        Err(err) => out!("取消开机自启失败：{}\n", err),
    }
    if hook::stop() {
        out!("已通知守护进程退出。\n");
    } else {
        out!("守护进程本来就没在运行。\n");
    }
    0
}

fn cmd_stop() -> i32 {
    if hook::stop() {
        out!("已通知守护进程退出。\n");
        0
    } else {
        out!("守护进程没有在运行。\n");
        1
    }
}

fn print_config_fields(config: &Config) {
    out!("  结构版本：v{}\n", config.version);
    out!("  触发键　：{}\n", config.label());
    out!(
        "  目标　　：{}\n",
        if config.target.is_empty() {
            "（未设置）".to_string()
        } else {
            config.target.clone()
        }
    );
    if !config.arguments.is_empty() {
        out!("  参数　　：{}\n", config.arguments);
    }
    out!(
        "  拦截按键：{}\n",
        if config.swallow {
            "是（不会再呼出搜索面板）"
        } else {
            "否"
        }
    );
    out!(
        "  严格修饰：{}\n",
        if config.strict_mods {
            "是（修饰键必须完全一致，多按/少按都不触发）"
        } else {
            "否（只看主键，多按/少按修饰键都会触发）"
        }
    );
}

/// 打印 state.json 里的运行状态：pid / 启动时间 / 累计触发 / 上次触发结果。
///
/// 这是 `status` 的第二块信息源：即使 `log = false` 关掉日志、release 构建没有
/// 控制台，也能看到真实的运行结果 —— 因为 state.json 由守护进程无条件原子写入。
fn print_runtime_state(running: bool) {
    match State::load() {
        Ok(Some(state)) => {
            if running {
                out!("  运行状态：pid {}，启动于 {}\n", state.pid, state.started_at);
            } else {
                out!(
                    "  运行状态：已停止（记录来自上次运行：pid {}，启动于 {}）\n",
                    state.pid, state.started_at
                );
            }
            out!(
                "  钩子就绪：{}\n",
                if state.hook_ready {
                    "是"
                } else {
                    "否（本次启动没装成钩子）"
                }
            );
            out!("  累计触发：{} 次\n", state.trigger_count);
            let at = state
                .last_trigger_at
                .clone()
                .unwrap_or_else(|| "时间未知".to_string());
            let target = if state.last_target.is_empty() {
                "（空）".to_string()
            } else {
                state.last_target.clone()
            };
            if state.last_outcome == state::OUTCOME_FAILED {
                out!(
                    "  上次触发：失败：{}\n",
                    if state.last_error.is_empty() {
                        "（原因未知）"
                    } else {
                        state.last_error.as_str()
                    }
                );
                out!("            时间 {}，目标 {}\n", at, target);
            } else if let Some(label) = state.outcome_label() {
                out!("  上次触发：{}（{}，目标 {}）\n", label, at, target);
            } else {
                out!("  上次触发：（还没有触发过）\n");
            }
        }
        Ok(None) => out!("  运行状态：（没有记录，守护进程从未成功启动过）\n"),
        Err(err) => out!("  运行状态：读取失败（{}）\n", err),
    }
}

fn cmd_status() -> i32 {
    let path = util::config_path();
    let running = hook::is_running();
    let mut logging = true;
    out!("copilotkey v{}\n", env!("CARGO_PKG_VERSION"));
    match Config::load() {
        Ok(Some(config)) => {
            logging = config.log;
            out!("  配置文件：{}\n", path.display());
            print_config_fields(&config);
        }
        Ok(None) => {
            out!("  配置文件：{}（不存在，正在用默认值）\n", path.display());
            print_config_fields(&Config::default());
        }
        Err(err) => {
            // 只报告、不动文件：备份与退出交给 target / install 这类会写配置的命令。
            out!("  配置文件：{}\n", path.display());
            out!("  警告　　：{}\n", err);
            if running {
                out!("  配置已损坏，正在使用上次有效配置（守护进程内存里的 last-good）。\n");
            } else {
                out!("  配置已损坏，守护进程未运行，当前没有生效的配置。\n");
            }
            out!("  执行 copilotkey target / install 会先把原文件备份成 config.json.broken-<时间戳>，再以非零码退出。\n");
        }
    }
    if !logging {
        out!("  日志开关：已关闭（log = false）—— 运行结果仍记录在 state.json，下面照常可见\n");
    }
    match autostart::query() {
        Some(value) => out!("  开机自启：已启用 → {}\n", value),
        None => out!("  开机自启：未启用\n"),
    }
    out!("  守护进程：{}\n", if running { "运行中" } else { "未运行" });
    print_runtime_state(running);
    let tail = util::tail_log(5);
    if !tail.is_empty() {
        out!("\n最近的日志：\n{}\n", tail);
    }
    0
}

fn cmd_test() -> i32 {
    let config = config::load_or_exit();
    if config.trigger_vk == 0 {
        out!("还没有设置触发键。\n");
        return 1;
    }

    // 1) 先静态校验目标（不启动任何东西）：目标无效就没必要注入按键。
    if let Err(err) = check_target(&config) {
        out!("目标校验失败：{}\n", err);
        return 1;
    }
    out!("目标校验：通过（{}）\n", config.target.trim());

    if !hook::is_running() {
        out!("守护进程没在运行，注入的按键不会有任何反应。\n");
        out!("先执行 copilotkey install，再执行 copilotkey test。\n");
        return 1;
    }

    // 注入被忽略时注入本身毫无意义（还会把按键放行给系统），直接说明清楚。
    if config.ignore_injected {
        out!("\n注意：配置里 ignore_injected = true，守护进程按设计会忽略程序注入的按键。\n");
        out!("      所以注入**不能**验证钩子通路；真实按下 {} 仍然有效。\n", config.label());
        out!("结论：目标可启动，但钩子通路未验证 —— 本次测试不算通过（也不是失败）。\n");
        return 1;
    }

    // 2) 注入前记下基线：靠 state.json 的计数变化判断“真的触发了”，而不是假定成功。
    let baseline = state::load().ok().flatten();
    let before_count = baseline.as_ref().map(|state| state.trigger_count).unwrap_or(0);
    let baseline_pid = baseline.as_ref().map(|state| state.pid);

    keys::send_combo(config.trigger_vk, config.trigger_mods);
    let confirmed = poll_until(Duration::from_secs(3), Duration::from_millis(100), || {
        State::load()
            .ok()
            .flatten()
            .map(|state| state.trigger_count > before_count)
            .unwrap_or(false)
    });

    if !confirmed {
        out!("\n未确认到真实触发：3 秒内 state.json 的触发计数没有增加。\n");
        out!("可能原因：\n");
        out!("  - 守护进程不是 v1.1.0 及以上（旧版本不写 state.json）；\n");
        out!("  - 钩子被其它程序抢占，或触发键/修饰键与注入的组合不一致；\n");
        out!("  - 守护进程在测试期间重启了。\n");
        let tail = util::tail_log(5);
        if !tail.is_empty() {
            out!("\n最近的日志：\n{}\n", tail);
        }
        return 1;
    }

    let latest = match State::load() {
        Ok(Some(state)) => state,
        _ => {
            out!("已确认触发计数增加，但随后读不到 state.json（可能被删除）。\n");
            return 1;
        }
    };
    if latest.pid != baseline_pid.unwrap_or(latest.pid) {
        out!("已确认触发，但守护进程在测试期间重启过（pid 变了），结果不可靠。\n");
        return 1;
    }

    out!("\n已确认真实触发：\n");
    out!(
        "  累计触发：{} 次（测试前 {} 次）\n",
        latest.trigger_count, before_count
    );
    out!(
        "  触发时间：{}\n",
        latest
            .last_trigger_at
            .clone()
            .unwrap_or_else(|| "时间未知".to_string())
    );
    out!(
        "  目标　　：{}\n",
        if latest.last_target.is_empty() {
            "（空）"
        } else {
            latest.last_target.as_str()
        }
    );
    if latest.last_outcome == state::OUTCOME_FAILED {
        out!("  结果　　：失败：{}\n", latest.last_error);
        out!("\n钩子通路正常，但目标启动失败 —— 先修好目标再试。\n");
        return 1;
    }
    out!("  结果　　：成功\n");
    out!("钩子通路与目标启动都正常。\n");
    0
}

/// 反复调用 `condition`，直到它为真或超时；返回是否在超时前为真。
///
/// 抽成独立函数是为了能单测“轮询确认”这套逻辑本身（install / test 都用它）。
fn poll_until(timeout: Duration, interval: Duration, mut condition: impl FnMut() -> bool) -> bool {
    let deadline = Instant::now() + timeout;
    loop {
        if condition() {
            return true;
        }
        if Instant::now() >= deadline {
            return false;
        }
        thread::sleep(interval);
    }
}

/// 判断 state.json 是否属于「刚拉起的那个守护进程，且钩子已就绪」。
///
/// 两个条件缺一不可：
/// 1. `pid` 等于子进程 pid —— 排除“旧进程还持有互斥量、新进程静默退出”的假成功；
/// 2. `started_at` 与上一次记录不同（或此前没有记录）—— 排除 pid 被复用。
fn daemon_ready(state: Option<&State>, child_pid: u32, previous: Option<&State>) -> bool {
    let Some(state) = state else {
        return false;
    };
    if !state.hook_ready || state.pid != child_pid {
        return false;
    }
    match previous {
        Some(previous) => previous.pid != state.pid || previous.started_at != state.started_at,
        None => true,
    }
}

/// 等新拉起的守护进程完成启动：`child.try_wait()` 提前发现退出，state.json 确认钩子就绪。
fn wait_daemon_ready(
    child: &mut Child,
    child_pid: u32,
    previous: Option<&State>,
) -> Result<State, String> {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                let code = status
                    .code()
                    .map(|code| code.to_string())
                    .unwrap_or_else(|| "未知".to_string());
                return Err(format!("守护进程启动后立即退出（退出码 {}）。", code));
            }
            Ok(None) => {}
            Err(err) => return Err(format!("无法查询守护进程状态：{}", err)),
        }
        if let Ok(Some(state)) = State::load() {
            if daemon_ready(Some(&state), child_pid, previous) && hook::is_running() {
                return Ok(state);
            }
        }
        if Instant::now() >= deadline {
            return Err(
                "守护进程 5 秒内没有完成启动（state.json 未确认钩子就绪）。\n\
                 请检查是否残留旧进程：taskkill /F /IM copilotkey.exe 后重试。"
                    .to_string(),
            );
        }
        thread::sleep(Duration::from_millis(100));
    }
}

/// 校验目标“确实有东西可以启动”：空目标、不存在的路径、没编译的工具名都算失败。
///
/// 只做静态检查，不启动任何程序（`test` 用它避免“注入成功但目标根本不存在”）。
fn check_target(config: &Config) -> Result<(), String> {
    let target = config.target.trim();
    if target.is_empty() {
        return Err("还没有设置目标（用 copilotkey target <程序> 设置）".to_string());
    }
    let lower = target.to_lowercase();
    if lower.starts_with("http://") || lower.starts_with("https://") || lower.starts_with("ms-") {
        return Ok(());
    }
    if target.contains('\\') || target.contains('/') || lower.ends_with(".exe") {
        if Path::new(target).exists() {
            return Ok(());
        }
        return Err(format!("目标路径不存在：{}", target));
    }
    if let Some(repo) = repo_root() {
        if tool_exe(&repo, target).is_some() {
            return Ok(());
        }
    }
    Err(format!(
        "“{}” 不是已编译的 tao_tools 工具，也不是有效路径或 URL；用 copilotkey target --list 看看可用工具",
        target
    ))
}

fn spawn_daemon(exe: &Path) -> std::io::Result<Child> {
    use std::os::windows::process::CommandExt;
    const DETACHED_PROCESS: u32 = 0x0000_0008;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    Command::new(exe)
        .arg("run")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .creation_flags(DETACHED_PROCESS | CREATE_NO_WINDOW)
        .spawn()
}

/// 判断某个目录是不是 cargo workspace 根（Cargo.toml 里带 `[workspace]`）。
fn is_workspace_root(dir: &Path) -> bool {
    matches!(
        std::fs::read_to_string(dir.join("Cargo.toml")),
        Ok(text) if text.contains("[workspace]")
    )
}

/// 找出 tao_tools 的 workspace 目录（工具都在它的子目录里）。
///
/// 依次尝试：可执行文件所在目录往上（`<workspace>\target\<profile>\exe`）、
/// 当前目录往上（含旧布局 `<仓库>\Tools\Cargo.toml`）、最后是常见安装路径。
fn repo_root() -> Option<PathBuf> {
    if let Ok(exe) = std::env::current_exe() {
        for dir in exe.ancestors().skip(1).take(4) {
            if is_workspace_root(dir) {
                return Some(dir.to_path_buf());
            }
        }
    }
    if let Ok(cwd) = std::env::current_dir() {
        for dir in cwd.ancestors().take(6) {
            if is_workspace_root(dir) {
                return Some(dir.to_path_buf());
            }
            if is_workspace_root(&dir.join("Tools")) {
                return Some(dir.join("Tools"));
            }
        }
    }
    for candidate in [
        r"D:\File_Data\Tao_Tools\Tools",
        r"D:\File_Data\Tao_Tools",
        r"E:\File_Data\Tao_Tools\Tools",
        r"E:\File_Data\Tao_Tools",
    ] {
        let path = PathBuf::from(candidate);
        if is_workspace_root(&path) {
            return Some(path);
        }
    }
    None
}

/// 在仓库里找某个工具的已编译可执行文件（workspace 共享 target，也兼容独立 target）。
fn tool_exe(repo: &Path, name: &str) -> Option<PathBuf> {
    let file = format!("{}.exe", name);
    let candidates = [
        repo.join("target").join("release").join(&file),
        repo.join("target").join("debug").join(&file),
        repo.join(name).join("target").join("release").join(&file),
        repo.join(name).join("target").join("debug").join(&file),
    ];
    candidates.into_iter().find(|path| path.exists())
}

/// 把“工具名”解析成完整路径；已经是路径或 URL 就原样返回。
fn resolve_target(raw: &str) -> String {
    let text = raw.trim();
    if text.contains('\\')
        || text.contains('/')
        || text.contains(':')
        || text.to_lowercase().ends_with(".exe")
        || text.to_lowercase().starts_with("http")
        || text.to_lowercase().starts_with("ms-")
    {
        return text.to_string();
    }
    if text.eq_ignore_ascii_case("calc") || text.eq_ignore_ascii_case("calculator") {
        return r"C:\Windows\System32\calc.exe".to_string();
    }
    if let Some(repo) = repo_root() {
        if let Some(path) = tool_exe(&repo, text) {
            return path.display().to_string();
        }
    }
    text.to_string()
}

fn list_tools() -> i32 {
    let Some(repo) = repo_root() else {
        out!("找不到 tao_tools 仓库目录，请直接用完整路径设置目标。\n");
        return 1;
    };
    out!("仓库目录：{}\n\n", repo.display());
    let mut names: Vec<String> = Vec::new();
    if let Ok(entries) = std::fs::read_dir(&repo) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() && path.join("Cargo.toml").exists() {
                if let Some(name) = entry.file_name().to_str() {
                    names.push(name.to_string());
                }
            }
        }
    }
    names.sort();
    if names.is_empty() {
        out!("仓库里没有找到任何工具。\n");
        return 1;
    }
    out!("{:<12} {:<10} {}\n", "工具", "状态", "可执行文件");
    for name in &names {
        match tool_exe(&repo, name) {
            Some(path) => out!("{:<12} {:<10} {}\n", name, "可用", path.display()),
            None => out!("{:<12} {:<10} {}\n", name, "未编译", "-"),
        }
    }
    out!("\n用 copilotkey target <工具名> 即可绑定，例如 copilotkey target deskman\n");
    0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn poll_until_returns_immediately_when_condition_holds() {
        let mut calls = 0;
        assert!(poll_until(
            Duration::from_secs(1),
            Duration::from_millis(10),
            || {
                calls += 1;
                true
            }
        ));
        assert_eq!(calls, 1, "条件一开始就成立，不该多轮询");
    }

    #[test]
    fn poll_until_waits_for_eventual_condition() {
        let mut calls = 0;
        assert!(poll_until(
            Duration::from_secs(2),
            Duration::from_millis(10),
            || {
                calls += 1;
                calls >= 3
            }
        ));
        assert_eq!(calls, 3);
    }

    #[test]
    fn poll_until_times_out() {
        let started = Instant::now();
        assert!(!poll_until(
            Duration::from_millis(120),
            Duration::from_millis(20),
            || false
        ));
        assert!(started.elapsed() >= Duration::from_millis(120));
    }

    fn sample_state(pid: u32, started_at: &str, hook_ready: bool) -> State {
        State {
            pid,
            started_at: started_at.to_string(),
            hook_ready,
            ..State::default()
        }
    }

    #[test]
    fn daemon_ready_requires_matching_pid_and_ready_hook() {
        let current = sample_state(4242, "t1", true);
        assert!(daemon_ready(Some(&current), 4242, None));

        // pid 对不上：不是我们刚拉起的那个进程。
        assert!(!daemon_ready(Some(&current), 4241, None));
        // 钩子没就绪：进程起来了但钩子没装上，不能算启动成功。
        let not_ready = sample_state(4242, "t1", false);
        assert!(!daemon_ready(Some(&not_ready), 4242, None));
        // 根本没有 state.json。
        assert!(!daemon_ready(None, 4242, None));

        // 与上一次记录完全相同：旧进程还持有互斥量，新进程已静默退出。
        let previous = sample_state(4242, "t1", true);
        assert!(!daemon_ready(Some(&current), 4242, Some(&previous)));

        // pid 被复用但启动时间变了：确实是一次新的启动。
        let restarted = sample_state(4242, "t2", true);
        assert!(daemon_ready(Some(&restarted), 4242, Some(&previous)));

        // pid 变了、启动时间也变了：新进程。
        let replaced = sample_state(5000, "t2", true);
        assert!(daemon_ready(Some(&replaced), 5000, Some(&previous)));
    }

    #[test]
    fn check_target_rejects_empty_missing_and_unknown_targets() {
        let mut config = Config::default();
        assert!(check_target(&config)
            .unwrap_err()
            .contains("还没有设置目标"));

        config.target = "https://example.com".to_string();
        assert!(check_target(&config).is_ok());

        config.target = "ms-settings:".to_string();
        assert!(check_target(&config).is_ok());

        config.target = r"C:\definitely\not\here\copilotkey-missing.exe".to_string();
        let err = check_target(&config).unwrap_err();
        assert!(err.contains("不存在"), "错误信息不对：{}", err);

        config.target = "no-such-tool-xyz".to_string();
        assert!(check_target(&config).is_err());
    }
}
