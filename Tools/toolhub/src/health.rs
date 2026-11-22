// ======================== 工具健康检查 ========================
//
// 两类软问题，复用注册表校验的 `Problem` 展示通道（状态栏 / 列表行 / 「体检」命令）：
//   1. 版本不一致：registry.json 的 version ≠ 该工具 Cargo.toml 的 version
//   2. 可执行文件：解析不到可执行文件（只能回退 cargo run，或连回退条件都没有）
//
// 检查结果只提示、不阻塞启动；版本数据来自 `cargo metadata`（见 metadata.rs），
// 解析结果复用 `ResolveCache`，不额外读盘。

use crate::config::HubConfig;
use crate::launcher::{ResolveCache, Target};
use crate::metadata::WorkspacePackages;
use crate::registry::{Problem, Registry, ToolEntry};

/// 去掉版本号可能带的 `v` 前缀再比较
fn normalize_version(version: &str) -> &str {
    version.trim().trim_start_matches(['v', 'V'])
}

/// 版本比对：registry 记录 vs workspace 包版本
pub fn version_problem(tool: &ToolEntry, packages: &WorkspacePackages) -> Option<Problem> {
    let Some(expected) = packages.versions.get(&tool.id) else {
        return Some(Problem::tool(
            Some(tool.id.clone()),
            format!(
                "版本：Cargo.toml 中找不到名为 {} 的包，无法比对 registry 记录的 v{}",
                tool.id, tool.version
            ),
        ));
    };
    if normalize_version(expected) == normalize_version(&tool.version) {
        return None;
    }
    Some(Problem::tool(
        Some(tool.id.clone()),
        format!(
            "版本不一致：registry 记录 v{}，Cargo.toml 记录 v{}（同步其一后点「刷新」重新体检）",
            normalize_version(&tool.version),
            normalize_version(expected)
        ),
    ))
}

/// 可执行文件问题：None 表示解析失败，CargoRun 表示只能回退开发模式
pub fn exe_problem(tool: &ToolEntry, target: Option<&Target>) -> Option<Problem> {
    match target {
        Some(Target::Exe(_)) => None,
        Some(Target::CargoRun(_)) => Some(Problem::tool(
            Some(tool.id.clone()),
            format!(
                "可执行文件：未找到已构建/已安装的 {}，启动时会回退 cargo run -p {}（开发模式，首次编译较慢）",
                tool.id, tool.id
            ),
        )),
        None => Some(Problem::tool(
            Some(tool.id.clone()),
            format!(
                "可执行文件：解析失败，请先 cargo build -p {} 或 cargo install --path {}",
                tool.id, tool.id
            ),
        )),
    }
}

/// 跑一遍健康检查；`packages` 为 None 时跳过版本比对（cargo metadata 还没回来）
pub fn collect(
    registry: &Registry,
    packages: Option<&WorkspacePackages>,
    config: &HubConfig,
    resolve: &mut ResolveCache,
) -> Vec<Problem> {
    let mut problems = Vec::new();
    for tool in &registry.tools {
        if let Some(packages) = packages {
            if let Some(problem) = version_problem(tool, packages) {
                problems.push(problem);
            }
        }
        let target = resolve.get(tool, config);
        if let Some(problem) = exe_problem(tool, target.as_ref()) {
            problems.push(problem);
        }
    }
    problems
}

/// workspace 成员里尚未出现在注册表中的包（只提示，不自动写入）
pub fn unregistered(registry: &Registry, packages: &WorkspacePackages) -> Vec<String> {
    packages
        .members
        .iter()
        .filter(|name| !registry.tools.iter().any(|tool| &tool.id == *name))
        .cloned()
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::registry::RegistrySource;
    use std::collections::HashMap;

    fn tool(id: &str, version: &str) -> ToolEntry {
        ToolEntry {
            id: id.to_string(),
            title: String::new(),
            description: String::new(),
            version: version.to_string(),
            kind: "cli".to_string(),
            launch: Vec::new(),
            settings: Vec::new(),
            console: false,
            readme: String::new(),
        }
    }

    fn registry(tools: Vec<ToolEntry>) -> Registry {
        Registry {
            tools,
            source: RegistrySource::Embedded,
            problems: Vec::new(),
        }
    }

    fn packages(pairs: &[(&str, &str)]) -> WorkspacePackages {
        let mut versions = HashMap::new();
        let mut members = Vec::new();
        for (name, version) in pairs {
            versions.insert(name.to_string(), version.to_string());
            members.push(name.to_string());
        }
        WorkspacePackages { versions, members }
    }

    #[test]
    fn version_mismatch_is_reported() {
        let tool = tool("deskman", "4.3.2");
        let packages = packages(&[("deskman", "4.3.1")]);
        let problem = version_problem(&tool, &packages).expect("版本不同应报问题");
        assert_eq!(problem.tool_id.as_deref(), Some("deskman"));
        assert!(problem.message.contains("4.3.2"), "{}", problem.message);
        assert!(problem.message.contains("4.3.1"), "{}", problem.message);
    }

    #[test]
    fn matching_version_is_clean_and_v_prefix_is_ignored() {
        assert!(version_problem(&tool("deskman", "4.3.2"), &packages(&[("deskman", "4.3.2")]))
            .is_none());
        // registry 里偶尔会带 v 前缀，比较前去掉
        assert!(version_problem(&tool("deskman", "v4.3.2"), &packages(&[("deskman", "4.3.2")]))
            .is_none());
    }

    #[test]
    fn missing_package_is_reported() {
        let problem =
            version_problem(&tool("ghost", "1.0.0"), &packages(&[])).expect("缺包应报问题");
        assert!(problem.message.contains("ghost"), "{}", problem.message);
    }

    #[test]
    fn exe_problems_only_when_not_a_built_exe() {
        let tool = tool("sertool", "2.4.0");
        assert!(exe_problem(&tool, Some(&Target::Exe("x".into()))).is_none());
        assert!(exe_problem(&tool, Some(&Target::CargoRun("x".into()))).is_some());
        assert!(exe_problem(&tool, None).is_some());
    }

    #[test]
    fn unregistered_lists_workspace_members_not_in_registry() {
        let registry = registry(vec![tool("toolhub", "1.2.0"), tool("sertool", "2.4.0")]);
        let packages = packages(&[
            ("toolhub", "1.2.0"),
            ("sertool", "2.4.0"),
            ("newtool", "0.1.0"),
        ]);
        assert_eq!(unregistered(&registry, &packages), vec!["newtool".to_string()]);
    }

    #[test]
    fn collect_skips_version_check_without_metadata() {
        let registry = registry(vec![tool("sertool", "2.4.0")]);
        let exe = std::env::current_exe().expect("current_exe 可用");
        let mut config = HubConfig::default();
        config
            .paths
            .insert("sertool".to_string(), exe.display().to_string());
        let mut cache = ResolveCache::default();

        // 没有 metadata 时只剩可执行文件检查，而它指向真实存在的文件 → 没有问题
        let problems = collect(&registry, None, &config, &mut cache);
        assert!(problems.is_empty(), "{:?}", problems);
        assert_eq!(cache.resolve_calls(), 1, "体检应复用解析缓存，每个工具只解析一次");
    }
}
