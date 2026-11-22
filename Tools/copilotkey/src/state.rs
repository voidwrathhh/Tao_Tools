//! 运行状态：%APPDATA%\.tao_tools\copilotkey\state.json
//!
//! 守护进程把「我是谁、钩子是否就绪、触发了几次、上次触发是否成功」写进这个文件，
//! 命令行（`status` / `test` / `install`）只读它。这样即使 `log = false`、
//! release 构建没有控制台，也能看到真实的运行情况，不必靠日志猜测。
//!
//! 写入复用 [`util::write_atomic`]（临时文件 + `fsync` + 重命名）：
//! 读的一方要么读到旧的完整内容、要么读到新的完整内容，不会读到写了一半的 JSON。

use std::path::Path;
use std::sync::{Mutex, OnceLock};

use serde::{Deserialize, Serialize};

use crate::util;

/// 状态结构版本；字段增删时 +1，旧文件缺字段由 serde 默认值补齐。
pub const STATE_VERSION: u32 = 1;

/// `last_outcome` 取值：成功启动目标。
pub const OUTCOME_OK: &str = "ok";
/// `last_outcome` 取值：启动目标失败（详情在 `last_error`）。
pub const OUTCOME_FAILED: &str = "failed";

/// 落盘的运行状态。字段全部带默认值，旧版本文件缺字段也能读。
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct State {
    /// 状态结构版本。
    #[serde(default = "def_version")]
    pub version: u32,
    /// 守护进程 pid（命令行用它确认“当前运行的是不是我刚拉起的那个进程”）。
    #[serde(default)]
    pub pid: u32,
    /// 守护进程启动时间（本地时间戳，形如 2026-11-22 14:00:00.123）。
    #[serde(default)]
    pub started_at: String,
    /// 全局键盘钩子是否已安装成功（`run` 在 `install_hook` 成功后才置真）。
    #[serde(default)]
    pub hook_ready: bool,
    /// 累计触发次数（每次识别到触发键 +1，不论目标启动成功与否）。
    #[serde(default)]
    pub trigger_count: u64,
    /// 上次触发的时间；从未触发过为 `None`。
    #[serde(default)]
    pub last_trigger_at: Option<String>,
    /// 上次触发时的目标（配置里的原始写法，便于对照）。
    #[serde(default)]
    pub last_target: String,
    /// 上次触发的结果：`ok` / `failed`；空串表示还没触发过。
    #[serde(default)]
    pub last_outcome: String,
    /// 上次触发失败的原因（成功时为空）。
    #[serde(default)]
    pub last_error: String,
}

fn def_version() -> u32 {
    STATE_VERSION
}

impl Default for State {
    fn default() -> Self {
        Self {
            version: STATE_VERSION,
            pid: 0,
            started_at: String::new(),
            hook_ready: false,
            trigger_count: 0,
            last_trigger_at: None,
            last_target: String::new(),
            last_outcome: String::new(),
            last_error: String::new(),
        }
    }
}

impl State {
    /// 上次触发结果的中文说明；还没触发过返回 `None`。
    pub fn outcome_label(&self) -> Option<&'static str> {
        match self.last_outcome.as_str() {
            OUTCOME_OK => Some("成功"),
            OUTCOME_FAILED => Some("失败"),
            _ => None,
        }
    }

    /// 读取运行状态，**无副作用**。
    ///
    /// - `Ok(None)`：文件不存在（守护进程从未启动过，或状态文件被删了）
    /// - `Ok(Some(_))`：解析成功，且已按 `version` 完成迁移
    /// - `Err(_)`：读取失败 / JSON 损坏 / 版本高于本程序
    pub fn load() -> Result<Option<State>, String> {
        Self::load_from(&util::state_path())
    }

    fn load_from(path: &Path) -> Result<Option<State>, String> {
        let text = match std::fs::read_to_string(path) {
            Ok(text) => text,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(err) => return Err(format!("读取运行状态失败：{}（{}）", err, path.display())),
        };
        let mut state: State = serde_json::from_str(&text)
            .map_err(|err| format!("运行状态解析失败：{}（{}）", err, path.display()))?;
        state
            .migrate()
            .map_err(|err| format!("{}（{}）", err, path.display()))?;
        Ok(Some(state))
    }

    /// 原子写：临时文件 → `fsync` → 重命名。
    pub fn save(&self) -> Result<(), String> {
        self.save_to(&util::state_path())
    }

    fn save_to(&self, path: &Path) -> Result<(), String> {
        util::ensure_dirs().map_err(|err| format!("创建状态目录失败：{}", err))?;
        let mut normalized = self.clone();
        normalized.version = STATE_VERSION;
        let text = serde_json::to_string_pretty(&normalized)
            .map_err(|err| format!("序列化运行状态失败：{}", err))?;
        util::write_atomic(path, &text)
    }

    /// 版本比本程序新时拒绝加载（旧版本不该把新结构覆盖掉）。
    fn migrate(&mut self) -> Result<(), String> {
        if self.version > STATE_VERSION {
            return Err(format!(
                "运行状态版本 v{} 高于本程序支持的 v{}，请升级 copilotkey",
                self.version, STATE_VERSION
            ));
        }
        self.version = STATE_VERSION;
        Ok(())
    }
}

/// 记录一次触发（纯逻辑，便于单测）：
/// 计数 +1、写入时间/目标、成功时清空错误、失败时留下原因。
pub fn record_trigger_into(state: &mut State, target: &str, result: &Result<(), String>) {
    state.trigger_count = state.trigger_count.saturating_add(1);
    state.last_trigger_at = Some(util::timestamp());
    state.last_target = target.to_string();
    match result {
        Ok(()) => {
            state.last_outcome = OUTCOME_OK.to_string();
            state.last_error.clear();
        }
        Err(err) => {
            state.last_outcome = OUTCOME_FAILED.to_string();
            state.last_error = err.clone();
        }
    }
}

/// 守护进程内存里的当前状态（只有守护进程会写这个文件）。
static LIVE: OnceLock<Mutex<State>> = OnceLock::new();

fn mutate(mut change: impl FnMut(&mut State)) -> Result<(), String> {
    let Some(lock) = LIVE.get() else {
        return Err("运行状态未初始化（只有守护进程会写 state.json）".to_string());
    };
    let mut guard = lock.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    change(&mut guard);
    guard.save()
}

/// 守护进程启动：记录 pid 与启动时间并落盘。此时钩子还没装好，`hook_ready = false`。
pub fn begin() -> Result<(), String> {
    let state = State {
        pid: std::process::id(),
        started_at: util::timestamp(),
        ..State::default()
    };
    state.save()?;
    let _ = LIVE.set(Mutex::new(state));
    Ok(())
}

/// 钩子安装成功后置真；`status` / `install` 用它确认“钩子真的生效了”。
pub fn set_hook_ready(ready: bool) -> Result<(), String> {
    mutate(|state| state.hook_ready = ready)
}

/// 记录一次触发结果并立即落盘。
pub fn record_trigger(target: &str, result: &Result<(), String>) -> Result<(), String> {
    mutate(|state| record_trigger_into(state, target, result))
}

/// 读取本进程内的当前状态（单测与诊断用）。
#[cfg(test)]
pub fn current() -> Option<State> {
    LIVE.get()
        .map(|lock| lock.lock().unwrap_or_else(|poisoned| poisoned.into_inner()).clone())
}

/// 读取磁盘上的运行状态（`status` / `install` / `test` 用）。
pub fn load() -> Result<Option<State>, String> {
    State::load()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU32, Ordering};

    static SEQ: AtomicU32 = AtomicU32::new(0);

    fn temp_dir(tag: &str) -> PathBuf {
        let seq = SEQ.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!(
            "copilotkey-state-{}-{}-{}",
            std::process::id(),
            tag,
            seq
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).expect("创建测试目录失败");
        dir
    }

    #[test]
    fn save_and_load_roundtrip_leaves_no_tmp_file() {
        let dir = temp_dir("roundtrip");
        let path = dir.join("state.json");
        let mut state = State {
            pid: 4321,
            started_at: "2026-11-22 14:00:00.123".to_string(),
            hook_ready: true,
            ..State::default()
        };
        state.version = 99; // save 时应归一到 STATE_VERSION
        state.save_to(&path).expect("保存失败");

        let loaded = State::load_from(&path).unwrap().unwrap();
        assert_eq!(loaded.pid, 4321);
        assert_eq!(loaded.started_at, "2026-11-22 14:00:00.123");
        assert!(loaded.hook_ready);
        assert_eq!(loaded.version, STATE_VERSION);
        let leftovers: Vec<String> = fs::read_dir(&dir)
            .map(|entries| {
                entries
                    .flatten()
                    .map(|entry| entry.file_name().to_string_lossy().into_owned())
                    .filter(|name| name.contains(".tmp-"))
                    .collect()
            })
            .unwrap_or_default();
        assert!(leftovers.is_empty(), "临时文件没清掉：{:?}", leftovers);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn missing_file_is_not_an_error() {
        let dir = temp_dir("missing");
        assert!(State::load_from(&dir.join("state.json")).unwrap().is_none());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn old_state_without_new_fields_still_loads() {
        // 模拟“上一版程序写的 state.json”：只有 pid 一个字段。
        let dir = temp_dir("compat");
        let path = dir.join("state.json");
        fs::write(&path, r#"{"pid": 77}"#).expect("写入失败");

        let state = State::load_from(&path).unwrap().expect("旧状态应能读");
        assert_eq!(state.pid, 77);
        assert_eq!(state.version, STATE_VERSION);
        assert!(!state.hook_ready);
        assert_eq!(state.trigger_count, 0);
        assert!(state.last_trigger_at.is_none());
        assert!(state.outcome_label().is_none());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn corrupt_state_is_reported() {
        let dir = temp_dir("corrupt");
        let path = dir.join("state.json");
        fs::write(&path, "{").expect("写入失败");
        let err = State::load_from(&path).expect_err("损坏状态必须报错");
        assert!(err.contains("解析失败"), "错误信息不对：{}", err);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn newer_state_version_is_refused() {
        let dir = temp_dir("future");
        let path = dir.join("state.json");
        let mut state = State::default();
        state.version = STATE_VERSION + 1;
        fs::write(&path, serde_json::to_string(&state).unwrap()).expect("写入失败");
        let err = State::load_from(&path).expect_err("更高版本必须拒绝");
        assert!(err.contains("高于"), "错误信息不对：{}", err);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn record_trigger_counts_and_keeps_failure_reason() {
        let mut state = State {
            pid: 100,
            ..State::default()
        };

        record_trigger_into(&mut state, r"C:\Windows\System32\calc.exe", &Ok(()));
        assert_eq!(state.trigger_count, 1);
        assert_eq!(state.last_target, r"C:\Windows\System32\calc.exe");
        assert_eq!(state.last_outcome, OUTCOME_OK);
        assert_eq!(state.outcome_label(), Some("成功"));
        assert!(state.last_trigger_at.is_some());
        assert!(state.last_error.is_empty());

        let failed = Err("ShellExecuteEx 调用失败，错误码 2".to_string());
        record_trigger_into(&mut state, r"C:\nope.exe", &failed);
        assert_eq!(state.trigger_count, 2);
        assert_eq!(state.last_outcome, OUTCOME_FAILED);
        assert_eq!(state.outcome_label(), Some("失败"));
        assert_eq!(state.last_error, "ShellExecuteEx 调用失败，错误码 2");
        assert_eq!(state.last_target, r"C:\nope.exe");

        // 之后成功一次：计数继续加，错误被清空，结果回到 ok。
        record_trigger_into(&mut state, "calc", &Ok(()));
        assert_eq!(state.trigger_count, 3);
        assert_eq!(state.last_outcome, OUTCOME_OK);
        assert!(state.last_error.is_empty());
    }
}
