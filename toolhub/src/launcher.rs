// ======================== 启动与路径解析 ========================
//
// 优先使用已构建/已安装的可执行文件；都找不到时回退到 `cargo run`，
// 方便在仓库里直接开发（无需先 cargo install）。

use crate::config::HubConfig;
use crate::registry::{workspace_root, ToolEntry};
use std::path::{Path, PathBuf};
use std::process::Command;

#[derive(Debug, Clone)]
pub enum Target {
    Exe(PathBuf),
    CargoRun(PathBuf),
}

pub fn resolve(tool: &ToolEntry, config: &HubConfig) -> Option<Target> {
    if let Some(path) = config.paths.get(&tool.id) {
        let path = PathBuf::from(path);
        if path.is_file() {
            return Some(Target::Exe(path));
        }
    }

    let exe_name = format!("{}.exe", tool.id);
    if let Some(root) = workspace_root() {
        for dir in ["target/debug", "target/release"] {
            let candidate = root.join(dir).join(&exe_name);
            if candidate.is_file() {
                return Some(Target::Exe(candidate));
            }
        }
    }

    if let Some(home) = dirs::home_dir() {
        for name in [&exe_name, &tool.id] {
            let candidate = home.join(".cargo").join("bin").join(name);
            if candidate.is_file() {
                return Some(Target::Exe(candidate));
            }
        }
    }

    if let Some(candidate) = find_in_path(&tool.id) {
        return Some(Target::Exe(candidate));
    }

    workspace_root().map(Target::CargoRun)
}

pub fn describe(tool: &ToolEntry, config: &HubConfig) -> String {
    match resolve(tool, config) {
        Some(Target::Exe(path)) => path.display().to_string(),
        Some(Target::CargoRun(root)) => format!("cargo run -p {}（{}）", tool.id, root.display()),
        None => format!(
            "未找到可执行文件，请先安装：cargo install --path {}",
            tool.id
        ),
    }
}

pub fn launch(tool: &ToolEntry, args: &[String], config: &HubConfig) -> Result<String, String> {
    let target = resolve(tool, config).ok_or_else(|| {
        format!(
            "未找到 {} 的可执行文件；请先执行 cargo install --path {}",
            tool.id, tool.id
        )
    })?;

    match target {
        Target::Exe(path) => {
            let mut command = Command::new(&path);
            command.args(args);
            if tool.console {
                attach_console(&mut command);
            }
            command
                .spawn()
                .map_err(|e| format!("启动 {} 失败：{}", path.display(), e))?;
            Ok(format!("已启动 {}（{}）", tool.id, path.display()))
        }
        Target::CargoRun(root) => {
            let mut command = Command::new("cmd");
            command
                .current_dir(&root)
                .arg("/C")
                .arg("start")
                .arg("")
                .arg("cargo")
                .arg("run")
                .arg("-p")
                .arg(&tool.id)
                .arg("--")
                .args(args);
            detach_window(&mut command);
            command
                .spawn()
                .map_err(|e| format!("启动 cargo run -p {} 失败：{}", tool.id, e))?;
            Ok(format!(
                "正在通过 cargo run 启动 {}（首次编译可能较慢）",
                tool.id
            ))
        }
    }
}

pub fn open_path(path: &Path) -> Result<(), String> {
    if !path.exists() {
        return Err(format!("路径不存在：{}", path.display()));
    }
    let mut command = Command::new("cmd");
    command.arg("/C").arg("start").arg("").arg(path);
    detach_window(&mut command);
    command
        .spawn()
        .map_err(|e| format!("打开 {} 失败：{}", path.display(), e))?;
    Ok(())
}

pub fn open_folder(path: &Path) -> Result<(), String> {
    if !path.exists() {
        return Err(format!("路径不存在：{}", path.display()));
    }
    Command::new("explorer")
        .arg(path)
        .spawn()
        .map_err(|e| format!("打开目录 {} 失败：{}", path.display(), e))?;
    Ok(())
}

fn find_in_path(name: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    let separator = if cfg!(windows) { ';' } else { ':' };
    for dir in std::env::split_paths(&path) {
        for candidate in [dir.join(name), dir.join(format!("{}.exe", name))] {
            if candidate.is_file() {
                return Some(candidate);
            }
        }
    }
    let _ = separator;
    None
}

#[cfg(windows)]
fn attach_console(command: &mut Command) {
    use std::os::windows::process::CommandExt;
    const CREATE_NEW_CONSOLE: u32 = 0x0000_0010;
    command.creation_flags(CREATE_NEW_CONSOLE);
}

#[cfg(not(windows))]
fn attach_console(_command: &mut Command) {}

#[cfg(windows)]
fn detach_window(command: &mut Command) {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    command.creation_flags(CREATE_NO_WINDOW);
}

#[cfg(not(windows))]
fn detach_window(_command: &mut Command) {}
