// ======================== 工具注册表 ========================
//
// registry.json 是本工具中心的唯一数据源：每个 tao_tools 工具都必须在这里注册，
// 才会出现在快捷入口列表中（详见 README.md）。
//
// 加载优先级：
//   1. workspace/toolhub/registry.json（源码仓库内，开发时以它为准）
//   2. ~/.tao_tools/toolhub/registry.json（用户自定义覆盖）
//   3. 编译期内置的 registry.json（cargo install 后离开仓库时仍可用）

use serde::Deserialize;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Deserialize)]
pub struct ToolEntry {
    /// 工具在 workspace 中的目录名，同时也是默认可执行文件名
    pub id: String,
    /// 界面显示名称
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub version: String,
    /// gui / tui / cli，仅用于界面展示
    #[serde(default = "default_kind")]
    pub kind: String,
    /// 点击“启动”时传给可执行文件的参数
    #[serde(default)]
    pub launch: Vec<String>,
    /// 点击“设置”时传给可执行文件的参数（直达工具自带设置）
    #[serde(default)]
    pub settings: Vec<String>,
    /// 是否需要在独立控制台窗口中运行（终端类工具为 true）
    #[serde(default)]
    pub console: bool,
    /// README 相对 workspace 根目录的路径
    #[serde(default)]
    pub readme: String,
}

fn default_kind() -> String {
    "gui".to_string()
}

impl ToolEntry {
    pub fn display_name(&self) -> &str {
        if self.title.is_empty() {
            &self.id
        } else {
            &self.title
        }
    }

    pub fn readme_path(&self) -> Option<PathBuf> {
        if self.readme.is_empty() {
            return None;
        }
        workspace_root().map(|root| root.join(&self.readme))
    }
}

#[derive(Debug, Clone)]
pub enum RegistrySource {
    Workspace(PathBuf),
    User(PathBuf),
    Embedded,
}

impl RegistrySource {
    pub fn label(&self) -> String {
        match self {
            RegistrySource::Workspace(path) => format!("仓库注册表 {}", path.display()),
            RegistrySource::User(path) => format!("用户注册表 {}", path.display()),
            RegistrySource::Embedded => "内置注册表（cargo install 版本）".to_string(),
        }
    }

    pub fn file_path(&self) -> Option<&Path> {
        match self {
            RegistrySource::Workspace(path) | RegistrySource::User(path) => Some(path),
            RegistrySource::Embedded => None,
        }
    }
}

#[derive(Debug, Clone)]
pub struct Registry {
    pub tools: Vec<ToolEntry>,
    pub source: RegistrySource,
}

#[derive(Deserialize)]
struct RegistryFile {
    #[serde(default)]
    tools: Vec<ToolEntry>,
}

impl Registry {
    pub fn load() -> Self {
        let mut candidates: Vec<PathBuf> = Vec::new();
        if let Some(root) = workspace_root() {
            candidates.push(root.join("toolhub").join("registry.json"));
        }
        candidates.push(hub_dir().join("registry.json"));

        for path in candidates {
            if !path.is_file() {
                continue;
            }
            match read_registry(&path) {
                Ok(tools) => {
                    let source = if path.starts_with(hub_dir()) {
                        RegistrySource::User(path)
                    } else {
                        RegistrySource::Workspace(path)
                    };
                    return Self { tools, source };
                }
                Err(e) => eprintln!("警告：读取注册表 {} 失败：{}", path.display(), e),
            }
        }

        match read_registry_str(include_str!("../registry.json")) {
            Ok(tools) => Self {
                tools,
                source: RegistrySource::Embedded,
            },
            Err(e) => {
                eprintln!("错误：内置注册表损坏：{}", e);
                Self {
                    tools: Vec::new(),
                    source: RegistrySource::Embedded,
                }
            }
        }
    }
}

fn read_registry(path: &Path) -> Result<Vec<ToolEntry>, String> {
    let text = std::fs::read_to_string(path).map_err(|e| e.to_string())?;
    read_registry_str(&text)
}

fn read_registry_str(text: &str) -> Result<Vec<ToolEntry>, String> {
    // 有些 Windows 编辑器会给 UTF-8 文件加 BOM，serde_json 不认，先去掉再解析。
    let text = text.trim_start_matches('\u{feff}');
    let file: RegistryFile = serde_json::from_str(text).map_err(|e| e.to_string())?;
    Ok(file.tools)
}

pub fn hub_dir() -> PathBuf {
    let base = dirs::home_dir()
        .or_else(|| dirs::data_dir())
        .unwrap_or_else(|| PathBuf::from("."));
    base.join(".tao_tools").join("toolhub")
}

/// 从当前目录或可执行文件所在目录向上寻找带 [workspace] 的 Cargo.toml
///
/// 当前布局把 Cargo.toml 放在 `Tools/` 下（仓库根目录只放文档），所以每层目录
/// 既检查 `<dir>/Cargo.toml`，也检查 `<dir>/Tools/Cargo.toml`，两种布局都能定位。
pub fn workspace_root() -> Option<PathBuf> {
    if let Some(root) = std::env::var_os("TAO_TOOLS_ROOT") {
        let root = PathBuf::from(root);
        if root.join("Cargo.toml").is_file() {
            return Some(root);
        }
    }

    let mut starts: Vec<PathBuf> = Vec::new();
    if let Ok(cwd) = std::env::current_dir() {
        starts.push(cwd);
    }
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            starts.push(dir.to_path_buf());
        }
    }

    for start in starts {
        let mut dir = start;
        loop {
            if is_workspace_manifest(&dir.join("Cargo.toml")) {
                return Some(dir);
            }
            let nested = dir.join("Tools");
            if is_workspace_manifest(&nested.join("Cargo.toml")) {
                return Some(nested);
            }
            if !dir.pop() {
                break;
            }
        }
    }
    None
}

fn is_workspace_manifest(path: &Path) -> bool {
    std::fs::read_to_string(path)
        .map(|text| text.contains("[workspace]"))
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn embedded_registry_is_valid() {
        let tools = read_registry_str(include_str!("../registry.json")).unwrap();
        assert!(tools.len() >= 4);
        assert!(tools.iter().all(|t| !t.id.is_empty()));
        assert!(tools.iter().any(|t| t.id == "deskman"));
    }
}
