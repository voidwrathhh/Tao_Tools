// ======================== 启动与路径解析 ========================
//
// 优先使用已构建/已安装的可执行文件；都找不到时回退到 `cargo run`，
// 方便在仓库里直接开发（无需先 cargo install）。
//
// `resolve` 会读盘（workspace 逐级向上、`~/.cargo/bin`、整条 PATH），属于重操作，
// GUI 里通过 `ResolveCache` 按 id 缓存结果，只在 new/刷新、paths 变更或启动失败时失效。
//
// `RunningTools` 只记录“本进程自己 spawn 的子进程”，结束进程也只针对这些 pid，
// 不按名字杀进程，也不接管各工具自身的单例逻辑。

use crate::config::HubConfig;
use crate::registry::{workspace_root, ToolEntry};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::process::{Child, Command};

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

fn format_target(tool: &ToolEntry, target: Option<&Target>) -> String {
    match target {
        Some(Target::Exe(path)) => path.display().to_string(),
        Some(Target::CargoRun(root)) => format!("cargo run -p {}（{}）", tool.id, root.display()),
        None => format!(
            "未找到可执行文件，请先安装：cargo install --path {}",
            tool.id
        ),
    }
}

pub fn describe(tool: &ToolEntry, config: &HubConfig) -> String {
    format_target(tool, resolve(tool, config).as_ref())
}

/// 按 id 缓存 `resolve` 结果
///
/// 失效时机：`HubApp::new` / 刷新（重新读注册表与 config，paths 可能已变）、
/// 启动失败（解析结果可能已过期，例如刚 build 完路径变了）。
/// 因此“刚 build 完不刷新就看到旧结果”是已知取舍，点「刷新」即可重算。
#[derive(Debug, Default)]
pub struct ResolveCache {
    entries: HashMap<String, Option<Target>>,
    /// 实际执行 `resolve` 的次数：用于验证 300 帧后不再增长
    resolve_calls: usize,
}

impl ResolveCache {
    pub fn clear(&mut self) {
        self.entries.clear();
    }

    pub fn invalidate(&mut self, id: &str) {
        self.entries.remove(id);
    }

    /// 已执行 `resolve` 的次数（缓存命中不计）
    pub fn resolve_calls(&self) -> usize {
        self.resolve_calls
    }

    pub fn get(&mut self, tool: &ToolEntry, config: &HubConfig) -> Option<Target> {
        if let Some(cached) = self.entries.get(tool.id.as_str()) {
            return cached.clone();
        }
        self.resolve_calls += 1;
        let target = resolve(tool, config);
        self.entries.insert(tool.id.clone(), target.clone());
        target
    }

    pub fn describe(&mut self, tool: &ToolEntry, config: &HubConfig) -> String {
        let target = self.get(tool, config);
        format_target(tool, target.as_ref())
    }
}

/// 启动结果：带上子进程句柄，交给 `RunningTools` 跟踪
pub struct Launched {
    pub message: String,
    pub child: Child,
}

pub fn launch(tool: &ToolEntry, args: &[String], config: &HubConfig) -> Result<Launched, String> {
    let target = resolve(tool, config).ok_or_else(|| {
        format!(
            "未找到 {} 的可执行文件；请先执行 cargo install --path {}",
            tool.id, tool.id
        )
    })?;

    match target {
        Target::Exe(path) => {
            let mut command = if tool.console {
                // 控制台工具用 cmd /K 包一层：工具退出后窗口保留，输出不会一闪而过
                let mut line = quote_arg(&path.display().to_string());
                for arg in args {
                    line.push(' ');
                    line.push_str(&quote_arg(arg));
                }
                let mut command = Command::new("cmd");
                command.arg("/K").arg(line);
                attach_console(&mut command);
                command
            } else {
                let mut command = Command::new(&path);
                command.args(args);
                command
            };
            let child = command
                .spawn()
                .map_err(|e| format!("启动 {} 失败：{}", path.display(), e))?;
            Ok(Launched {
                message: format!("已启动 {}（{}）", tool.id, path.display()),
                child,
            })
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
            let child = command
                .spawn()
                .map_err(|e| format!("启动 cargo run -p {} 失败：{}", tool.id, e))?;
            Ok(Launched {
                message: format!(
                    "正在通过 cargo run 启动 {}（首次编译可能较慢）",
                    tool.id
                ),
                child,
            })
        }
    }
}

/// 本进程拉起、尚未确认退出的子进程
///
/// 只按 id 记录，`poll_running` / `running_pid` 用 `try_wait` 清理已退出的条目，
/// 这样任务管理器里 kill 掉工具后，界面上的「运行中」下一帧就会消失。
#[derive(Debug, Default)]
pub struct RunningTools {
    children: HashMap<String, Child>,
}

impl RunningTools {
    pub fn insert(&mut self, id: &str, child: Child) {
        self.children.insert(id.to_string(), child);
    }

    /// 清理已退出的子进程，返回当前仍在运行的 id 集合
    pub fn poll_running(&mut self) -> HashSet<String> {
        let mut running = HashSet::new();
        self.children.retain(|id, child| match child.try_wait() {
            Ok(None) => {
                running.insert(id.clone());
                true
            }
            Ok(Some(_)) | Err(_) => false,
        });
        running
    }

    /// 该工具是否仍在运行（顺带清理已退出的子进程）
    pub fn running_pid(&mut self, id: &str) -> Option<u32> {
        let (pid, status) = match self.children.get_mut(id) {
            Some(child) => (child.id(), child.try_wait()),
            None => return None,
        };
        match status {
            Ok(None) => Some(pid),
            _ => {
                self.children.remove(id);
                None
            }
        }
    }

    /// 结束本进程拉起的工具；返回被结束的 pid（已自行退出返回 None）
    pub fn stop(&mut self, id: &str) -> Result<Option<u32>, String> {
        let Some(mut child) = self.children.remove(id) else {
            return Ok(None);
        };
        let pid = child.id();
        if let Ok(Some(_)) = child.try_wait() {
            return Ok(None);
        }
        match child.kill() {
            Ok(()) => {
                // 回收句柄，避免留下未 wait 的子进程记录
                let _ = child.wait();
                Ok(Some(pid))
            }
            Err(e) => {
                // 结束失败就继续跟踪，下一次还能再试
                self.children.insert(id.to_string(), child);
                Err(format!("结束 pid {} 失败：{}", pid, e))
            }
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

/// 给 cmd 的命令行片段加引号（路径或参数含空格时必需）
fn quote_arg(value: &str) -> String {
    if value.is_empty() {
        return "\"\"".to_string();
    }
    if value.contains([' ', '\t', '"']) {
        format!("\"{}\"", value.replace('"', "\\\""))
    } else {
        value.to_string()
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture_tool(id: &str) -> ToolEntry {
        ToolEntry {
            id: id.to_string(),
            title: String::new(),
            description: String::new(),
            version: String::new(),
            kind: "cli".to_string(),
            launch: Vec::new(),
            settings: Vec::new(),
            console: false,
            readme: String::new(),
        }
    }

    /// 300 帧渲染等价于对每个工具调用一次 `describe`；解析次数应恒等于工具数
    #[test]
    fn resolve_cache_is_constant_across_frames() {
        let exe = std::env::current_exe().expect("current_exe 可用");
        let mut config = HubConfig::default();
        let mut tools = Vec::new();
        for index in 0..5 {
            let id = format!("fixture-tool-{}", index);
            // 把 paths 指向一个确实存在的文件，避免测试依赖真实工作区布局
            config.paths.insert(id.clone(), exe.display().to_string());
            tools.push(fixture_tool(&id));
        }

        let mut cache = ResolveCache::default();
        for _frame in 0..300 {
            for tool in &tools {
                let hint = cache.describe(tool, &config);
                assert!(!hint.is_empty(), "每行都应拿到解析结果文本");
            }
        }
        assert_eq!(
            cache.resolve_calls(),
            tools.len(),
            "300 帧后每个工具只应解析一次"
        );

        // 单个工具失效只重算它自己
        cache.invalidate(&tools[0].id);
        let _ = cache.get(&tools[0], &config);
        assert_eq!(cache.resolve_calls(), tools.len() + 1);

        // 整体失效（刷新 / paths 变更）后重新解析
        cache.clear();
        let _ = cache.get(&tools[0], &config);
        assert_eq!(cache.resolve_calls(), tools.len() + 2);
    }

    #[test]
    fn running_tools_reports_only_spawned_children() {
        let mut running = RunningTools::default();
        assert!(running.poll_running().is_empty());
        assert_eq!(running.running_pid("anything"), None);
        assert_eq!(running.stop("anything"), Ok(None));
    }
}
