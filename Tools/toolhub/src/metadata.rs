// ======================== cargo metadata 只读查询 ========================
//
// 用 `cargo metadata --no-deps` 读取 workspace 里的包名与版本：
//   · 健康检查：对比 registry.json 的 version 与各工具 Cargo.toml 的版本
//   · 自动发现：列出 workspace 成员里还没注册到 registry.json 的包
//
// 只读：不修改任何 Cargo.toml / registry.json，发现结果只作提示。
// `cargo metadata` 可能要几百毫秒到数秒，GUI 在后台线程里跑，不阻塞界面。

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Command;

use serde::Deserialize;

/// workspace 里的包（按 `cargo metadata` 的输出解析）
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct WorkspacePackages {
    /// 包名 → 版本
    pub versions: HashMap<String, String>,
    /// 包名（字典序），即 workspace 成员
    pub members: Vec<String>,
}

#[derive(Deserialize)]
struct Metadata {
    packages: Vec<Package>,
}

#[derive(Deserialize)]
struct Package {
    name: String,
    version: String,
}

/// 解析 `cargo metadata --no-deps --format-version 1` 的 JSON 输出
pub fn parse_metadata(json: &str) -> Result<WorkspacePackages, String> {
    let metadata: Metadata =
        serde_json::from_str(json).map_err(|e| format!("解析 cargo metadata 输出失败：{}", e))?;

    let mut versions = HashMap::new();
    let mut members = Vec::new();
    for package in metadata.packages {
        members.push(package.name.clone());
        versions.insert(package.name, package.version);
    }
    members.sort();
    members.dedup();

    Ok(WorkspacePackages { versions, members })
}

/// 找到仓库 workspace 清单（`<workspace>/Cargo.toml`）
pub fn manifest_path() -> Option<PathBuf> {
    crate::registry::workspace_root().map(|root| root.join("Cargo.toml"))
}

/// 运行 `cargo metadata`（只读）并解析结果
pub fn run(manifest: &Path) -> Result<WorkspacePackages, String> {
    let output = Command::new("cargo")
        .arg("metadata")
        .arg("--no-deps")
        .arg("--format-version")
        .arg("1")
        .arg("--manifest-path")
        .arg(manifest)
        .output()
        .map_err(|e| format!("无法运行 cargo（不在 PATH 中？）：{}", e))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!(
            "cargo metadata 失败：{}",
            stderr.trim().lines().next().unwrap_or("未知错误")
        ));
    }

    parse_metadata(&String::from_utf8_lossy(&output.stdout))
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"{
        "packages": [
            {"name": "toolhub", "version": "1.2.0"},
            {"name": "sertool", "version": "2.4.0"},
            {"name": "friend", "version": "1.0.0"}
        ],
        "workspace_members": ["toolhub", "sertool", "friend"]
    }"#;

    #[test]
    fn metadata_parses_versions_and_members() {
        let packages = parse_metadata(SAMPLE).expect("示例 metadata 应能解析");
        assert_eq!(packages.versions.get("toolhub"), Some(&"1.2.0".to_string()));
        assert_eq!(packages.versions.get("sertool"), Some(&"2.4.0".to_string()));
        assert_eq!(
            packages.members,
            vec!["friend".to_string(), "sertool".to_string(), "toolhub".to_string()]
        );
    }

    #[test]
    fn broken_metadata_is_an_error() {
        assert!(parse_metadata("not json").is_err());
        assert!(parse_metadata(r#"{"packages": [{"name": "x"}]}"#).is_err());
    }

    /// 真实环境验证用：只读当前仓库，不写任何文件
    #[test]
    fn workspace_metadata_contains_toolhub() {
        let Some(manifest) = manifest_path() else {
            return; // 不在仓库里（例如 cargo install 后的测试环境）就跳过
        };
        let Ok(packages) = run(&manifest) else {
            return; // cargo 不可用时跳过，避免依赖外部环境
        };
        assert!(
            packages.versions.contains_key("toolhub"),
            "workspace 里应有 toolhub 包，实际：{:?}",
            packages.members
        );
    }
}
