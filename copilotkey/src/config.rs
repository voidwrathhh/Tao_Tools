//! 配置读写：%APPDATA%\.tao_tools\copilotkey\config.json

use serde::{Deserialize, Serialize};

use crate::util;

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Config {
    /// 配置结构版本，便于以后升级迁移。
    #[serde(default = "def_version")]
    pub version: u32,
    /// 触发键的虚拟键码，默认 0x86 = F23（Copilot 键）。
    #[serde(default = "def_vk")]
    pub trigger_vk: u32,
    /// 触发键自带的修饰键（位掩码：1=Win 2=Shift 4=Ctrl 8=Alt）。
    #[serde(default)]
    pub trigger_mods: u32,
    /// 组合键的可读名称，仅用于展示。
    #[serde(default = "def_label")]
    pub trigger_label: String,
    /// 是否严格要求修饰键完全匹配；false 时只看主键。
    #[serde(default)]
    pub strict_mods: bool,
    /// 是否拦截按键，不让它传给系统（即“不再呼出搜索面板”）。
    #[serde(default = "def_true")]
    pub swallow: bool,
    /// 是否忽略程序注入的按键（自测时会用到，默认不忽略）。
    #[serde(default)]
    pub ignore_injected: bool,
    /// 防抖毫秒数，避免按住不放时连续启动。
    #[serde(default = "def_debounce")]
    pub debounce_ms: u64,
    /// 目标：程序路径、URL 或 tao_tools 里的工具名。
    #[serde(default)]
    pub target: String,
    /// 传给目标的命令行参数。
    #[serde(default)]
    pub arguments: String,
    /// 工作目录（可空）。
    #[serde(default)]
    pub workdir: String,
    /// 是否写日志。
    #[serde(default = "def_true")]
    pub log: bool,
}

fn def_version() -> u32 {
    1
}
fn def_vk() -> u32 {
    0x86
}
fn def_label() -> String {
    "Win+Shift+F23".to_string()
}
fn def_true() -> bool {
    true
}
fn def_debounce() -> u64 {
    600
}

impl Default for Config {
    fn default() -> Self {
        Self {
            version: def_version(),
            trigger_vk: def_vk(),
            trigger_mods: 0b0011,
            trigger_label: def_label(),
            strict_mods: false,
            swallow: def_true(),
            ignore_injected: false,
            debounce_ms: def_debounce(),
            target: String::new(),
            arguments: String::new(),
            workdir: String::new(),
            log: def_true(),
        }
    }
}

impl Config {
    pub fn load() -> Config {
        let path = util::config_path();
        match std::fs::read_to_string(&path) {
            Ok(text) => serde_json::from_str(&text).unwrap_or_else(|err| {
                util::say(&format!(
                    "配置文件解析失败，暂时使用默认值：{}\n文件：{}\n",
                    err,
                    path.display()
                ));
                Config::default()
            }),
            Err(_) => Config::default(),
        }
    }

    pub fn save(&self) -> Result<(), String> {
        util::ensure_dirs().map_err(|err| format!("创建配置目录失败：{}", err))?;
        let text = serde_json::to_string_pretty(self).map_err(|err| format!("序列化配置失败：{}", err))?;
        std::fs::write(util::config_path(), text).map_err(|err| format!("写入配置失败：{}", err))
    }

    pub fn label(&self) -> String {
        if self.trigger_label.is_empty() {
            crate::keys::combo_label(self.trigger_vk, self.trigger_mods)
        } else {
            self.trigger_label.clone()
        }
    }
}
