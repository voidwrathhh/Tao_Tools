// ======================== 用户本地配置 ========================
//
// 保存界面顺序与自定义可执行文件路径，位置：
//   ~/.tao_tools/toolhub/config.json
//
// 排序只影响本机显示顺序，不改动 registry.json。

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct HubConfig {
    /// 工具 id 的显示顺序；新注册的工具会自动追加到末尾
    #[serde(default)]
    pub order: Vec<String>,
    /// 手动指定的可执行文件路径（可选），优先于自动探测
    #[serde(default)]
    pub paths: BTreeMap<String, String>,
}

impl HubConfig {
    pub fn config_path() -> PathBuf {
        crate::registry::hub_dir().join("config.json")
    }

    pub fn load() -> Self {
        let path = Self::config_path();
        if !path.is_file() {
            return Self::default();
        }
        match std::fs::read_to_string(&path) {
            Ok(text) => serde_json::from_str(&text).unwrap_or_else(|e| {
                eprintln!("警告：配置解析失败（{}），已使用默认顺序", e);
                Self::default()
            }),
            Err(e) => {
                eprintln!("警告：读取配置失败（{}），已使用默认顺序", e);
                Self::default()
            }
        }
    }

    pub fn save(&self) {
        let path = Self::config_path();
        if let Some(dir) = path.parent() {
            if let Err(e) = std::fs::create_dir_all(dir) {
                eprintln!("警告：无法创建配置目录（{}）：{}", dir.display(), e);
                return;
            }
        }
        match serde_json::to_string_pretty(self) {
            Ok(text) => {
                if let Err(e) = std::fs::write(&path, text) {
                    eprintln!("警告：写入配置失败（{}）：{}", path.display(), e);
                }
            }
            Err(e) => eprintln!("警告：序列化配置失败：{}", e),
        }
    }

    /// 按保存的顺序重排注册表工具；order 中缺失的工具追加到末尾
    pub fn apply_order(&self, tools: &[crate::registry::ToolEntry]) -> Vec<crate::registry::ToolEntry> {
        let mut ordered: Vec<crate::registry::ToolEntry> = Vec::with_capacity(tools.len());
        for id in &self.order {
            if let Some(tool) = tools.iter().find(|t| &t.id == id) {
                if !ordered.iter().any(|t| &t.id == id) {
                    ordered.push(tool.clone());
                }
            }
        }
        for tool in tools {
            if !ordered.iter().any(|t| t.id == tool.id) {
                ordered.push(tool.clone());
            }
        }
        ordered
    }
}
