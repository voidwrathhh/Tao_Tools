use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

#[derive(Debug, Clone)]
pub struct ToolInfo {
    pub name: String,
    pub version: String,
    pub description: String,
}

pub fn list_tools() -> Result<Vec<ToolInfo>, String> {
    let root = workspace_root()?;
    let mut tools = Vec::new();
    for entry in fs::read_dir(&root).map_err(|e| format!("读取工作区失败: {}", e))? {
        let entry = entry.map_err(|e| format!("读取目录项失败: {}", e))?;
        let path = entry.path();
        if !path.is_dir() || path.file_name().and_then(|n| n.to_str()) == Some("target") {
            continue;
        }
        let manifest = path.join("Cargo.toml");
        let readme = path.join("README.md");
        if manifest.exists() && readme.exists() {
            if let Some(info) = read_tool_manifest(&manifest)? {
                tools.push(info);
            }
        }
    }
    tools.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(tools)
}

pub fn run_cargo_for_tool(action: &str, name: &str) -> Result<(), String> {
    let root = workspace_root()?;
    ensure_tool_exists(&root, name)?;
    let status = Command::new("cargo")
        .arg(action)
        .arg("-p")
        .arg(name)
        .current_dir(&root)
        .stdin(Stdio::inherit())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .status()
        .map_err(|e| format!("无法启动 cargo {}: {}", action, e))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("cargo {} -p {} 执行失败", action, name))
    }
}

pub fn install_tool(name: &str) -> Result<(), String> {
    let root = workspace_root()?;
    let path = ensure_tool_exists(&root, name)?;
    let status = Command::new("cargo")
        .arg("install")
        .arg("--path")
        .arg(&path)
        .current_dir(&root)
        .stdin(Stdio::inherit())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .status()
        .map_err(|e| format!("无法启动 cargo install: {}", e))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("cargo install --path {} 执行失败", path.display()))
    }
}

pub fn tool_readme_path(name: &str) -> Result<PathBuf, String> {
    let root = workspace_root()?;
    let path = ensure_tool_exists(&root, name)?;
    Ok(path.join("README.md"))
}

pub fn workspace_root() -> Result<PathBuf, String> {
    let mut dir = std::env::current_dir().map_err(|e| format!("获取当前目录失败: {}", e))?;
    loop {
        if dir.join("Cargo.toml").exists() && has_workspace_manifest(&dir.join("Cargo.toml")) {
            return Ok(dir);
        }
        if !dir.pop() {
            return Err("未找到 tao_tools workspace 根目录".to_string());
        }
    }
}

fn has_workspace_manifest(path: &Path) -> bool {
    fs::read_to_string(path)
        .map(|content| content.contains("[workspace]"))
        .unwrap_or(false)
}

fn ensure_tool_exists(root: &Path, name: &str) -> Result<PathBuf, String> {
    let path = root.join(name);
    if path.join("Cargo.toml").exists() {
        Ok(path)
    } else {
        Err(format!("未找到工具 '{}'", name))
    }
}

fn read_tool_manifest(manifest: &Path) -> Result<Option<ToolInfo>, String> {
    let content = fs::read_to_string(manifest)
        .map_err(|e| format!("读取 {} 失败: {}", manifest.display(), e))?;
    let name = find_toml_string(&content, "name");
    let version = find_toml_string(&content, "version");
    let description = find_toml_string(&content, "description").unwrap_or_default();
    Ok(match (name, version) {
        (Some(name), Some(version)) => Some(ToolInfo {
            name,
            version,
            description,
        }),
        _ => None,
    })
}

fn find_toml_string(content: &str, key: &str) -> Option<String> {
    let prefix = format!("{} =", key);
    content.lines().find_map(|line| {
        let line = line.trim();
        if !line.starts_with(&prefix) {
            return None;
        }
        let value = line[prefix.len()..].trim();
        if value.starts_with('"') && value.ends_with('"') && value.len() >= 2 {
            Some(value[1..value.len() - 1].to_string())
        } else {
            None
        }
    })
}
