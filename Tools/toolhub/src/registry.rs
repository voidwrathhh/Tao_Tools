// ======================== 工具注册表 ========================
//
// registry.json 是本工具中心的唯一数据源：每个 tao_tools 工具都必须在这里注册，
// 才会出现在快捷入口列表中（详见 README.md）。
//
// 加载优先级：
//   1. workspace/toolhub/registry.json（源码仓库内，开发时以它为准）
//   2. ~/.tao_tools/toolhub/registry.json（用户自定义覆盖）
//   3. 编译期内置的 registry.json（cargo install 后离开仓库时仍可用）
//
// 加载时顺带做字段级校验（schema / id / kind / readme / 未知字段），问题收集到
// `Registry::problems` 交给界面展示：只做软提示，不阻塞其余工具启动。

use serde::Deserialize;
use serde_json::Value;
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

/// 当前支持的注册表 schema 版本
pub const SCHEMA_VERSION: i64 = 1;

/// ToolEntry 的已知字段：用于发现 "lauch" 这类拼写错误（否则只会静默退化为空）
const KNOWN_FIELDS: [&str; 9] = [
    "id",
    "title",
    "description",
    "version",
    "kind",
    "launch",
    "settings",
    "console",
    "readme",
];

/// kind 白名单
const VALID_KINDS: [&str; 3] = ["gui", "tui", "cli"];

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

/// 注册表校验发现的一条问题
///
/// `tool_id` 为 `None` 时表示注册表级问题（如 schema 不符）；否则可据此在列表
/// 对应行上标注。问题只做软提示，不影响其它条目加载与启动。
#[derive(Debug, Clone)]
pub struct Problem {
    pub tool_id: Option<String>,
    pub message: String,
}

impl Problem {
    fn registry(message: impl Into<String>) -> Self {
        Self {
            tool_id: None,
            message: message.into(),
        }
    }

    fn tool(tool_id: Option<String>, message: impl Into<String>) -> Self {
        Self {
            tool_id,
            message: message.into(),
        }
    }

    /// 供状态栏 / 命令行输出的一行文本
    pub fn display(&self) -> String {
        match self.tool_id.as_deref() {
            Some(id) if !id.is_empty() => format!("{}：{}", id, self.message),
            _ => self.message.clone(),
        }
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
    /// 加载期校验发现的问题（软提示，不阻塞启动）
    pub problems: Vec<Problem>,
}

/// 解析 + 校验的结果
#[derive(Debug, Default)]
struct ParsedRegistry {
    tools: Vec<ToolEntry>,
    problems: Vec<Problem>,
}

impl Registry {
    pub fn load() -> Self {
        let root = workspace_root();
        let mut problems: Vec<Problem> = Vec::new();

        let hub = hub_dir();
        let mut candidates: Vec<PathBuf> = Vec::new();
        if let Some(root) = &root {
            candidates.push(root.join("toolhub").join("registry.json"));
        }
        candidates.push(hub.join("registry.json"));

        for path in candidates {
            if !path.is_file() {
                continue;
            }
            let is_user = path.starts_with(&hub);
            match read_registry(&path, root.as_deref()) {
                Ok(parsed) => {
                    problems.extend(parsed.problems);
                    let source = if is_user {
                        RegistrySource::User(path)
                    } else {
                        RegistrySource::Workspace(path)
                    };
                    return Self {
                        tools: parsed.tools,
                        source,
                        problems,
                    };
                }
                Err(e) => {
                    let message = format!("读取注册表 {} 失败：{}", path.display(), e);
                    eprintln!("警告：{}", message);
                    problems.push(Problem::registry(message));
                }
            }
        }

        match parse_registry_str(include_str!("../registry.json"), root.as_deref()) {
            Ok(parsed) => {
                problems.extend(parsed.problems);
                Self {
                    tools: parsed.tools,
                    source: RegistrySource::Embedded,
                    problems,
                }
            }
            Err(e) => {
                let message = format!("内置注册表损坏：{}", e);
                eprintln!("错误：{}", message);
                problems.push(Problem::registry(message));
                Self {
                    tools: Vec::new(),
                    source: RegistrySource::Embedded,
                    problems,
                }
            }
        }
    }
}

fn read_registry(path: &Path, root: Option<&Path>) -> Result<ParsedRegistry, String> {
    let text = std::fs::read_to_string(path).map_err(|e| e.to_string())?;
    parse_registry_str(&text, root)
}

/// 解析并校验注册表文本
///
/// `root` 用于检查 `readme` 文件是否存在（`None` 时跳过该检查，便于单测）。
fn parse_registry_str(text: &str, root: Option<&Path>) -> Result<ParsedRegistry, String> {
    // 有些 Windows 编辑器会给 UTF-8 文件加 BOM，serde_json 不认，先去掉再解析。
    let text = text.trim_start_matches('\u{feff}');
    let value: Value = serde_json::from_str(text).map_err(|e| format!("JSON 解析失败：{}", e))?;
    let Some(object) = value.as_object() else {
        return Err("注册表根节点必须是 JSON 对象".to_string());
    };

    let mut parsed = ParsedRegistry::default();

    match object.get("schema") {
        Some(Value::Number(n)) if n.as_i64() == Some(SCHEMA_VERSION) => {}
        Some(other) => parsed.problems.push(Problem::registry(format!(
            "注册表 schema 应为 {}，实际为 {}（请按 README 的字段说明修正）",
            SCHEMA_VERSION, other
        ))),
        None => parsed.problems.push(Problem::registry(format!(
            "注册表缺少 schema 字段（应为 {}）",
            SCHEMA_VERSION
        ))),
    }

    let Some(Value::Array(items)) = object.get("tools") else {
        let message = if object.contains_key("tools") {
            "注册表的 tools 字段必须是数组".to_string()
        } else {
            "注册表缺少 tools 数组".to_string()
        };
        parsed.problems.push(Problem::registry(message));
        return Ok(parsed);
    };

    let mut seen: BTreeSet<String> = BTreeSet::new();
    for (index, item) in items.iter().enumerate() {
        let Some(map) = item.as_object() else {
            parsed
                .problems
                .push(Problem::registry(format!("tools[{}] 不是 JSON 对象，已忽略", index)));
            continue;
        };

        let raw_id = map
            .get("id")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();
        let label = if raw_id.is_empty() {
            format!("tools[{}]", index)
        } else {
            format!("tools[{}]（id={}）", index, raw_id)
        };
        let tool_id = if raw_id.is_empty() {
            None
        } else {
            Some(raw_id.clone())
        };

        for key in map.keys() {
            if !KNOWN_FIELDS.contains(&key.as_str()) {
                parsed.problems.push(Problem::tool(
                    tool_id.clone(),
                    format!(
                        "{} 含未知字段“{}”，可能是拼写错误（已知字段：{}）",
                        label,
                        key,
                        KNOWN_FIELDS.join(" / ")
                    ),
                ));
            }
        }

        let tool: ToolEntry = match serde_json::from_value(item.clone()) {
            Ok(tool) => tool,
            Err(e) => {
                parsed.problems.push(Problem::tool(
                    tool_id,
                    format!("{} 解析失败，已忽略该条目：{}", label, e),
                ));
                continue;
            }
        };

        if tool.id.trim().is_empty() {
            parsed.problems.push(Problem::tool(
                None,
                format!("{} 的 id 为空，已忽略该条目", label),
            ));
            continue;
        }
        if !seen.insert(tool.id.clone()) {
            parsed.problems.push(Problem::tool(
                Some(tool.id.clone()),
                format!("id“{}”重复，已忽略该文件中后出现的条目", tool.id),
            ));
            continue;
        }
        if !VALID_KINDS.contains(&tool.kind.as_str()) {
            parsed.problems.push(Problem::tool(
                Some(tool.id.clone()),
                format!(
                    "id“{}”的 kind“{}”不在 gui / tui / cli 中（界面会按 CLI 显示）",
                    tool.id, tool.kind
                ),
            ));
        }
        if let Some(root) = root {
            if !tool.readme.is_empty() && !root.join(&tool.readme).is_file() {
                parsed.problems.push(Problem::tool(
                    Some(tool.id.clone()),
                    format!(
                        "id“{}”的 readme 文件不存在：{}（路径相对 workspace 根目录）",
                        tool.id, tool.readme
                    ),
                ));
            }
        }

        parsed.tools.push(tool);
    }

    Ok(parsed)
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

    /// `Tools/` 目录：registry.json 里的 readme 路径相对它
    fn tools_root() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .expect("toolhub 目录应有父目录")
            .to_path_buf()
    }

    fn parse_ok(text: &str) -> ParsedRegistry {
        parse_registry_str(text, None).expect("顶层 JSON 应能解析")
    }

    #[test]
    fn embedded_registry_is_valid() {
        let root = tools_root();
        let parsed = parse_registry_str(include_str!("../registry.json"), Some(&root))
            .expect("内置注册表应能解析");
        assert!(parsed.tools.len() >= 4);
        assert!(parsed.tools.iter().all(|t| !t.id.is_empty()));
        assert!(parsed.tools.iter().any(|t| t.id == "deskman"));
        assert!(
            parsed.problems.is_empty(),
            "内置注册表不应有校验问题：{:?}",
            parsed.problems
        );
    }

    #[test]
    fn schema_mismatch_is_reported_and_tools_still_load() {
        let parsed = parse_ok(r#"{"schema":2,"tools":[{"id":"alpha"},{"id":"beta"}]}"#);
        assert_eq!(parsed.tools.len(), 2, "schema 不符不应丢弃工具");
        assert!(parsed
            .problems
            .iter()
            .any(|p| p.tool_id.is_none() && p.message.contains("schema")));
    }

    #[test]
    fn duplicate_id_is_reported_and_deduplicated() {
        let parsed = parse_ok(r#"{"schema":1,"tools":[{"id":"alpha"},{"id":"alpha"}]}"#);
        assert_eq!(parsed.tools.len(), 1);
        let problem = parsed
            .problems
            .iter()
            .find(|p| p.message.contains("重复"))
            .expect("重复 id 应产生问题");
        assert_eq!(problem.tool_id.as_deref(), Some("alpha"));
    }

    #[test]
    fn misspelled_field_is_reported_without_dropping_tool() {
        let parsed = parse_ok(r#"{"schema":1,"tools":[{"id":"alpha","lauch":["gui"]}]}"#);
        assert_eq!(parsed.tools.len(), 1, "拼错字段不应丢工具");
        let problem = parsed
            .problems
            .iter()
            .find(|p| p.message.contains("lauch"))
            .expect("未知字段应产生问题");
        assert_eq!(problem.tool_id.as_deref(), Some("alpha"));
        assert!(
            parsed.tools[0].launch.is_empty(),
            "拼错的字段不会生效，保持默认空参数"
        );
    }

    #[test]
    fn unknown_kind_and_missing_readme_are_reported() {
        let root = tools_root();
        let parsed = parse_registry_str(
            r#"{"schema":1,"tools":[{"id":"alpha","kind":"service","readme":"alpha/README.md"}]}"#,
            Some(&root),
        )
        .expect("应能解析");
        assert_eq!(parsed.tools.len(), 1);
        assert!(parsed
            .problems
            .iter()
            .any(|p| p.message.contains("kind") && p.tool_id.as_deref() == Some("alpha")));
        assert!(parsed
            .problems
            .iter()
            .any(|p| p.message.contains("readme") && p.tool_id.as_deref() == Some("alpha")));
    }

    #[test]
    fn empty_id_is_reported_and_dropped() {
        let parsed = parse_ok(r#"{"schema":1,"tools":[{"id":""},{"id":"beta"}]}"#);
        assert_eq!(parsed.tools.len(), 1);
        assert_eq!(parsed.tools[0].id, "beta");
        assert!(parsed.problems.iter().any(|p| p.message.contains("id 为空")));
    }

    #[test]
    fn broken_json_is_a_hard_error() {
        assert!(parse_registry_str("{ not json", None).is_err());
        assert!(parse_registry_str("[]", None).is_err());
    }
}
