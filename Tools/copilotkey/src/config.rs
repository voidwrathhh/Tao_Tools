//! 配置读写：%APPDATA%\.tao_tools\copilotkey\config.json
//!
//! 两条硬规则（v1.0.2 起）：
//!
//! 1. **原子写**：先写临时文件并 `fsync`，再 `rename` 覆盖正式文件。任何时刻读到的
//!    要么是旧的完整文件、要么是新的完整文件，不会读到写了一半的 JSON；
//! 2. **不静默回退**：解析失败时绝不拿默认值覆盖用户配置。命令行把原文件备份成
//!    `config.json.broken-<时间戳>` 后以非零码退出；守护进程保留内存里的 last-good。

use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::util;

/// 当前配置结构版本。字段增删或语义变化时 +1，并在 [`Config::migrate`] 里补迁移分支。
pub const CONFIG_VERSION: u32 = 1;

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
    CONFIG_VERSION
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
    /// 读取配置，**无副作用**（守护进程轮询与 `status` 都用它）。
    ///
    /// - `Ok(None)`：文件不存在（首次运行）
    /// - `Ok(Some(_))`：解析成功，且已按 `version` 完成迁移
    /// - `Err(_)`：读取失败 / JSON 损坏 / 版本高于本程序
    pub fn load() -> Result<Option<Config>, String> {
        Self::load_from(&util::config_path())
    }

    fn load_from(path: &Path) -> Result<Option<Config>, String> {
        let text = match fs::read_to_string(path) {
            Ok(text) => text,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(err) => return Err(format!("读取配置文件失败：{}（{}）", err, path.display())),
        };
        let mut config: Config = serde_json::from_str(&text)
            .map_err(|err| format!("配置文件解析失败：{}（{}）", err, path.display()))?;
        config
            .migrate()
            .map_err(|err| format!("{}（{}）", err, path.display()))?;
        Ok(Some(config))
    }

    /// 按 `version` 迁移；版本比本程序新时拒绝加载，避免把不认识的字段写丢。
    fn migrate(&mut self) -> Result<(), String> {
        if self.version > CONFIG_VERSION {
            return Err(format!(
                "配置结构版本 v{} 高于本程序支持的 v{}，请升级 copilotkey（不会用旧版本覆盖它）",
                self.version, CONFIG_VERSION
            ));
        }
        if self.version < CONFIG_VERSION {
            // v1 之前的文件没有 version 字段时 serde 会补默认值 1；这里统一归一到当前版本。
            self.version = CONFIG_VERSION;
        }
        Ok(())
    }

    /// 原子写入正式配置：写临时文件 → `fsync` → `rename` 覆盖。
    pub fn save(&self) -> Result<(), String> {
        util::ensure_dirs().map_err(|err| format!("创建配置目录失败：{}", err))?;
        self.save_to(&util::config_path())
    }

    fn save_to(&self, path: &Path) -> Result<(), String> {
        let mut normalized = self.clone();
        normalized.version = CONFIG_VERSION;
        let text = serde_json::to_string_pretty(&normalized)
            .map_err(|err| format!("序列化配置失败：{}", err))?;
        util::write_atomic(path, &text)
    }

    pub fn label(&self) -> String {
        if self.trigger_label.is_empty() {
            crate::keys::combo_label(self.trigger_vk, self.trigger_mods)
        } else {
            self.trigger_label.clone()
        }
    }
}

/// 命令行入口：读取配置。损坏时备份原文件并**以非零码退出**，绝不静默回退默认值。
pub fn load_or_exit() -> Config {
    match Config::load() {
        Ok(Some(config)) => config,
        Ok(None) => Config::default(),
        Err(err) => {
            util::err_say(&format!("{}\n", err));
            match backup_broken() {
                Some(path) => util::err_say(&format!("原文件已备份为：{}\n", path.display())),
                None => util::err_say(&format!(
                    "备份原文件失败，请手动检查：{}\n",
                    util::config_path().display()
                )),
            }
            util::err_say("已停止：不会用默认值覆盖你的配置。修好或删除该文件后重新执行命令。\n");
            std::process::exit(1);
        }
    }
}

/// 把损坏的配置改名成 `config.json.broken-<时间戳>`，返回备份路径。
pub fn backup_broken() -> Option<PathBuf> {
    backup_file(&util::config_path())
}

fn backup_file(path: &Path) -> Option<PathBuf> {
    if !path.exists() {
        return None;
    }
    let backup = path.with_file_name(format!(
        "{}.broken-{}",
        path.file_name()?.to_string_lossy(),
        util::file_stamp()
    ));
    fs::rename(path, &backup).ok().map(|_| backup)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU32, Ordering};

    static SEQ: AtomicU32 = AtomicU32::new(0);

    fn temp_dir(tag: &str) -> PathBuf {
        let seq = SEQ.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!(
            "copilotkey-test-{}-{}-{}",
            std::process::id(),
            tag,
            seq
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).expect("创建测试目录失败");
        dir
    }

    fn leftovers(dir: &Path) -> Vec<String> {
        fs::read_dir(dir)
            .map(|entries| {
                entries
                    .flatten()
                    .map(|entry| entry.file_name().to_string_lossy().into_owned())
                    .filter(|name| name.contains(".tmp-"))
                    .collect()
            })
            .unwrap_or_default()
    }

    #[test]
    fn save_is_atomic_and_leaves_no_tmp_file() {
        let dir = temp_dir("save");
        let path = dir.join("config.json");
        let mut config = Config::default();
        config.target = "x".repeat(4000);
        config.version = 7; // save 时应归一到 CONFIG_VERSION

        config.save_to(&path).expect("保存失败");

        assert!(path.exists());
        assert!(leftovers(&dir).is_empty(), "临时文件没清掉");
        let loaded = Config::load_from(&path).unwrap().unwrap();
        assert_eq!(loaded.target.len(), 4000);
        assert_eq!(loaded.version, CONFIG_VERSION);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn concurrent_write_and_read_never_sees_partial_json() {
        let dir = temp_dir("race");
        let path = dir.join("config.json");
        let mut base = Config::default();
        base.target = "t".repeat(4000);
        base.save_to(&path).expect("初始保存失败");

        let writer_path = path.clone();
        let writer = std::thread::spawn(move || {
            for index in 0..200 {
                let mut config = Config::default();
                config.target = format!("{}#{index}", "x".repeat(4000));
                config.save_to(&writer_path).expect("并发写入失败");
            }
        });

        let mut reads = 0usize;
        for _ in 0..2000 {
            match Config::load_from(&path) {
                Ok(Some(config)) => {
                    assert!(!config.target.is_empty(), "读到了空的 target");
                    reads += 1;
                }
                Ok(None) => panic!("配置文件在并发写入期间消失了"),
                Err(err) => panic!("读到了不完整的配置：{}", err),
            }
        }
        writer.join().expect("写线程 panic");
        assert!(reads > 0);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn broken_file_is_reported_and_backed_up() {
        let dir = temp_dir("broken");
        let path = dir.join("config.json");
        fs::write(&path, "{").expect("写入损坏配置失败");

        let err = Config::load_from(&path).expect_err("损坏配置必须报错");
        assert!(err.contains("解析失败"), "错误信息不对：{}", err);

        let backup = backup_file(&path).expect("应生成备份");
        assert!(backup.exists(), "备份文件不存在");
        assert!(!path.exists(), "原文件应已改名");
        assert!(
            backup
                .file_name()
                .unwrap()
                .to_string_lossy()
                .contains(".broken-"),
            "备份文件名不带时间戳：{}",
            backup.display()
        );
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn missing_file_is_not_an_error() {
        let dir = temp_dir("missing");
        let path = dir.join("config.json");
        assert!(Config::load_from(&path).unwrap().is_none());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn newer_version_is_refused_instead_of_downgraded() {
        let dir = temp_dir("future");
        let path = dir.join("config.json");
        let mut config = Config::default();
        config.version = CONFIG_VERSION + 1;
        fs::write(&path, serde_json::to_string(&config).unwrap()).expect("写入失败");

        let err = Config::load_from(&path).expect_err("更高版本必须拒绝加载");
        assert!(err.contains("高于"), "错误信息不对：{}", err);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn old_version_is_migrated_to_current() {
        let dir = temp_dir("migrate");
        let path = dir.join("config.json");
        let mut config = Config::default();
        config.version = 0;
        fs::write(&path, serde_json::to_string(&config).unwrap()).expect("写入失败");

        let loaded = Config::load_from(&path).unwrap().unwrap();
        assert_eq!(loaded.version, CONFIG_VERSION);
        let _ = fs::remove_dir_all(&dir);
    }
}
