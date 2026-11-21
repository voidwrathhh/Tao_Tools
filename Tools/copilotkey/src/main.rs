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
mod util;

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use config::Config;

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
    out!("  install                      注册开机自启并立即启动守护进程\n");
    out!("  uninstall                    取消开机自启并停止守护进程\n");
    out!("  status                       查看当前配置与运行状态\n");
    out!("  stop                         停止守护进程\n");
    out!("  test                         模拟按一次触发键，验证端到端效果\n");
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
                let mut config = Config::load();
                config.trigger_vk = key.vk;
                config.trigger_mods = key.mods;
                config.trigger_label = label.clone();
                config.strict_mods = false;
                if let Err(err) = config.save() {
                    out!("\n保存失败：{}\n", err);
                    return 1;
                }
                out!("\n已保存，触发键 = {}\n", label);
                if hook::is_running() {
                    out!("守护进程还在用旧配置，执行 copilotkey install 重启后生效。\n");
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
    let mut config = Config::load();
    config.trigger_vk = vk;
    config.trigger_mods = mods;
    config.trigger_label = keys::combo_label(vk, mods);
    if let Err(err) = config.save() {
        out!("保存失败：{}\n", err);
        return 1;
    }
    out!("触发键已设置为：{}\n", config.trigger_label);
    if hook::is_running() {
        out!("守护进程需要重启才能生效：执行 copilotkey install。\n");
    }
    0
}

fn cmd_target(args: &[String]) -> i32 {
    let mut config = Config::load();

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
    hook::run(Config::load())
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

    let config = Config::load();
    if let Err(err) = config.save() {
        out!("写入默认配置失败：{}\n", err);
        return 1;
    }
    match autostart::install(&exe_text) {
        Ok(()) => out!("已注册开机自启：HKCU\\Software\\Microsoft\\Windows\\CurrentVersion\\Run\\TaoToolsCopilotKey\n"),
        Err(err) => {
            out!("注册开机自启失败：{}\n", err);
            return 1;
        }
    }

    if hook::is_running() {
        hook::stop();
        thread::sleep(Duration::from_millis(400));
        out!("已通知旧进程退出。\n");
    }

    if let Err(err) = spawn_daemon(&exe) {
        out!("启动守护进程失败：{}\n", err);
        return 1;
    }
    thread::sleep(Duration::from_millis(900));
    if hook::is_running() {
        out!("守护进程已启动，全局钩子生效。\n");
        out!("现在按下 {} 就会启动目标程序，不再弹出搜索面板。\n", config.label());
    } else {
        out!("守护进程没有起来，最近日志：\n{}\n", util::tail_log(15));
        return 1;
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

fn cmd_status() -> i32 {
    let config = Config::load();
    let path = util::config_path();
    out!("copilotkey v{}\n", env!("CARGO_PKG_VERSION"));
    out!("  配置文件：{}{}\n", path.display(), if path.exists() { "" } else { "（不存在，正在用默认值）" });
    out!("  触发键　：{}\n", config.label());
    out!(
        "  目标　　：{}\n",
        if config.target.is_empty() { "（未设置）".to_string() } else { config.target.clone() }
    );
    if !config.arguments.is_empty() {
        out!("  参数　　：{}\n", config.arguments);
    }
    out!("  拦截按键：{}\n", if config.swallow { "是（不会再呼出搜索面板）" } else { "否" });
    out!("  严格修饰：{}\n", if config.strict_mods { "是" } else { "否（只看主键）" });
    match autostart::query() {
        Some(value) => out!("  开机自启：已启用 → {}\n", value),
        None => out!("  开机自启：未启用\n"),
    }
    out!("  守护进程：{}\n", if hook::is_running() { "运行中" } else { "未运行" });
    let tail = util::tail_log(5);
    if !tail.is_empty() {
        out!("\n最近的日志：\n{}\n", tail);
    }
    0
}

fn cmd_test() -> i32 {
    let config = Config::load();
    if config.trigger_vk == 0 {
        out!("还没有设置触发键。\n");
        return 1;
    }
    if !hook::is_running() {
        out!("提示：守护进程没在运行，注入的按键不会有任何反应。先执行 copilotkey install。\n");
    }
    keys::send_combo(config.trigger_vk, config.trigger_mods);
    let started = Instant::now();
    thread::sleep(Duration::from_millis(600));
    out!(
        "已模拟按下一次 {}（耗时 {} 毫秒）。\n",
        keys::combo_label(config.trigger_vk, config.trigger_mods),
        started.elapsed().as_millis()
    );
    if config.target.is_empty() {
        out!("当前没有设置目标程序，只会记录一条日志。\n");
    }
    let tail = util::tail_log(3);
    if !tail.is_empty() {
        out!("\n最近的日志：\n{}\n", tail);
    }
    0
}

fn spawn_daemon(exe: &Path) -> std::io::Result<()> {
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
        .map(|_| ())
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
