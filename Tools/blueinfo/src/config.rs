// ======================== 本地持久化配置 ========================
//
// 程序启动自动加载上次关闭时的串口参数、参数配置等。
// 配置文件保存于用户目录: ~/.tao_tools/blueinfo/config.json

use crate::data_struct::DeviceParam;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;

/// 应用全局持久化配置
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppConfig {
    pub last_port: String,
    pub last_baud: u32,
    pub params: Vec<DeviceParam>,
    pub max_samples: usize,
    pub auto_reconnect: bool,
    pub log_auto_scroll: bool,
    pub axis_auto_scale: bool,
    pub quick_commands: Vec<String>,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            last_port: String::new(),
            last_baud: 115200,
            params: crate::data_struct::default_params(),
            max_samples: crate::data_struct::MAX_SAMPLES_PER_CHANNEL,
            auto_reconnect: true,
            log_auto_scroll: true,
            axis_auto_scale: true,
            quick_commands: vec![
                "STATUS".into(),
                "STOP".into(),
                "SET_L_TARGET: 500".into(),
                "SET_R_TARGET: 500".into(),
                "SET_L_KP: 1.5".into(),
                "SET_L_KI: 0.1".into(),
                "SET_L_KD: 0.05".into(),
                "SET_R_KP: 1.5".into(),
                "SET_R_KI: 0.1".into(),
                "SET_R_KD: 0.05".into(),
            ],
        }
    }
}

impl AppConfig {
    fn config_dir() -> PathBuf {
        let base = dirs_data_dir();
        base.join("blueinfo")
    }

    fn config_path() -> PathBuf {
        Self::config_dir().join("config.json")
    }

    /// 从磁盘加载配置，失败时返回默认值
    pub fn load() -> Self {
        let path = Self::config_path();
        if !path.exists() {
            return Self::default();
        }
        match fs::read_to_string(&path) {
            Ok(content) => serde_json::from_str(&content).unwrap_or_else(|e| {
                eprintln!("警告：配置解析失败 ({})，已恢复默认配置", e);
                Self::default()
            }),
            Err(e) => {
                eprintln!("警告：读取配置文件失败 ({}), 已恢复默认配置", e);
                Self::default()
            }
        }
    }

    /// 保存配置到磁盘
    pub fn save(&self) {
        let dir = Self::config_dir();
        if let Err(e) = fs::create_dir_all(&dir) {
            eprintln!("警告：无法创建配置目录 ({}): {}", dir.display(), e);
            return;
        }
        let path = Self::config_path();
        match serde_json::to_string_pretty(self) {
            Ok(json) => {
                if let Err(e) = fs::write(&path, &json) {
                    eprintln!("警告：写入配置文件失败 ({}): {}", path.display(), e);
                }
            }
            Err(e) => eprintln!("警告：序列化配置失败: {}", e),
        }
    }
}

/// 获取用户数据目录（跨平台）
fn dirs_data_dir() -> PathBuf {
    if let Some(base) = dirs::data_dir() {
        base.join(".tao_tools")
    } else {
        PathBuf::from(".").join(".tao_tools")
    }
}
