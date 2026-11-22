// ======================== 数据结构定义 ========================
//
// 本模块定义 blueinfo 使用的所有数据结构，包括：
//   1. BinaryPacket — bincode 二进制协议帧（高频结构化数据）
//   2. PlotChannel  — 绘图曲线通道（含定长环形缓冲区）
//   3. DeviceParam  — 设备参数项
//   4. ParamValue   — UI 参数编辑状态
//
// ─── 线上格式（协议契约）───
// 帧 = [4B "BLUE"] [4B 载荷长度(u32 LE)] [bincode 载荷]
// bincode 1 定长（fixint）编码的载荷布局固定为：
//   f64 timestamp(8) | u8 channel_count(1) | u64 Vec 长度前缀(8) | f64 × count
//   即载荷长度 = 17 + 8 * count
// 没有 padding、没有 checksum 字段。C 侧发帧请按下表逐字节写，
// 不要直接发送 #pragma pack 的结构体（Vec 前缀是 u64，不是 u8/u32）。
// 解析与坏帧处理见 protocol.rs，完整 C 示例见 README「二进制协议」。

use serde::{Deserialize, Serialize};
use std::collections::VecDeque;
use std::time::Instant;

// ─── 常量 ───────────────────────────────────────────────────

/// 二进制帧魔数
pub const MAGIC: [u8; 4] = [b'B', b'L', b'U', b'E'];

/// 单条曲线最大采样点数（环形队列，循环覆盖旧数据）
pub const MAX_SAMPLES_PER_CHANNEL: usize = 2000;

/// 文本日志最大行数
pub const MAX_LOG_LINES: usize = 1000;

/// 缓存区内最大未解析二进制帧数
pub const MAX_CACHED_PACKETS: usize = 500;

/// 曲线画板颜色（深色背景可见）
pub const CHART_COLORS: &[&str] = &[
    "#00BFFF", "#FF6347", "#7CFC00", "#FFD700", "#FF69B4", "#00FA9A", "#FFA500", "#BA55D3",
];

// ─── 二进制协议帧 ────────────────────────────────────────────

/// 来自设备的二进制结构化数据包（bincode 序列化）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BinaryPacket {
    pub timestamp: f64,
    pub channel_count: u8,
    pub channel_data: Vec<f64>,
}

/// 带本地接收时刻的二进制帧。
///
/// 曲线 x 轴统一使用本地单调时钟（连接成功时记录的 `Instant` 原点），
/// 不再依赖设备端的 `timestamp` 字段，因此文本行与二进制帧在同一坐标系内。
#[derive(Debug, Clone)]
pub struct TimedPacket {
    pub packet: BinaryPacket,
    pub at: Instant,
}

// ─── 日志行与分类 ────────────────────────────────────────────

/// 日志行来源分类。
///
/// 只有「设备发来（Rx）且不是回显/提示前缀」的行才允许参与文本→曲线解析，
/// 避免本地回显（如 `>>> SET_L_TARGET: 500`）被当成设备数据新建通道。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LogKind {
    /// 从设备收到的原始文本行
    Rx,
    /// 本地发送/回显（`>>>` 前缀）
    Tx,
    /// 错误提示
    Err,
    /// 程序内部信息
    Info,
}

impl LogKind {
    pub fn label(self) -> &'static str {
        match self {
            LogKind::Rx => "RX",
            LogKind::Tx => "TX",
            LogKind::Err => "ERR",
            LogKind::Info => "信息",
        }
    }
}

/// 带分类与产生时刻的一行日志
#[derive(Debug, Clone)]
pub struct LogLine {
    pub kind: LogKind,
    pub text: String,
    /// 行产生（接收/发送）时的本地单调时钟，用于文本曲线的时间轴
    pub at: Instant,
}

impl LogLine {
    pub fn new(kind: LogKind, text: impl Into<String>) -> Self {
        Self {
            kind,
            text: text.into(),
            at: Instant::now(),
        }
    }

    /// 生产代码中 Rx 行由 `flush_text` 批量构造（整块共享同一个 `at`），
    /// 此处保留对称构造器供测试与未来单独使用。
    #[allow(dead_code)]
    pub fn rx(text: impl Into<String>) -> Self {
        Self::new(LogKind::Rx, text)
    }

    pub fn tx(text: impl Into<String>) -> Self {
        Self::new(LogKind::Tx, text)
    }

    pub fn err(text: impl Into<String>) -> Self {
        Self::new(LogKind::Err, text)
    }

    pub fn info(text: impl Into<String>) -> Self {
        Self::new(LogKind::Info, text)
    }

    /// 是否可以参与「文本→曲线」解析：只解析设备发来的、非回显/提示前缀的行
    pub fn is_plottable(&self) -> bool {
        self.kind == LogKind::Rx && !is_echo_prefix(&self.text)
    }
}

/// `>>>` 为本地回显，`[..]` 为程序提示，均不作为设备数据解析
pub fn is_echo_prefix(text: &str) -> bool {
    let trimmed = text.trim_start();
    trimmed.starts_with(">>>") || trimmed.starts_with('[')
}

// ─── 设备参数 ────────────────────────────────────────────────

/// 设备参数描述（用于序列化保存 / 下发）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeviceParam {
    pub name: String,
    pub value: f64,
    pub min: f64,
    pub max: f64,
    pub step: f64,
}

// ─── 用户界面参数状态 ───────────────────────────────────────

/// GUI 中单个参数的编辑状态
#[derive(Debug, Clone)]
pub struct ParamValue {
    pub name: String,
    pub value: f64,
    pub min: f64,
    pub max: f64,
    pub step: f64,
}

impl From<DeviceParam> for ParamValue {
    fn from(p: DeviceParam) -> Self {
        Self {
            name: p.name,
            value: p.value,
            min: p.min,
            max: p.max,
            step: p.step,
        }
    }
}

impl From<&DeviceParam> for ParamValue {
    fn from(p: &DeviceParam) -> Self {
        Self {
            name: p.name.clone(),
            value: p.value,
            min: p.min,
            max: p.max,
            step: p.step,
        }
    }
}

impl ParamValue {
    pub fn to_device_param(&self) -> DeviceParam {
        DeviceParam {
            name: self.name.clone(),
            value: self.value,
            min: self.min,
            max: self.max,
            step: self.step,
        }
    }
}

// ─── 绘图曲线通道 ────────────────────────────────────────────

/// 单条曲线通道，内部使用定长 `VecDeque` 环形队列
pub struct PlotChannel {
    /// 通道原始名（来自协议/文本键名，用于持久化隐藏名单与查找）
    pub name: String,
    /// 仅前端显示用的重命名（双击通道名修改，不影响协议与持久化键）
    pub display_name: Option<String>,
    pub color: String,
    pub visible: bool,
    data: VecDeque<[f64; 2]>,
    max_len: usize,
    pub cached_points: Vec<[f64; 2]>,
    dirty: bool,
}

impl PlotChannel {
    pub fn new(name: &str, color: &str) -> Self {
        Self {
            name: name.to_string(),
            display_name: None,
            color: color.to_string(),
            visible: true,
            data: VecDeque::with_capacity(MAX_SAMPLES_PER_CHANNEL),
            cached_points: Vec::with_capacity(MAX_SAMPLES_PER_CHANNEL),
            dirty: false,
            max_len: MAX_SAMPLES_PER_CHANNEL,
        }
    }

    /// 向环形队列追加一个采样点，超限时循环覆盖
    pub fn push(&mut self, x: f64, y: f64) {
        if self.data.len() >= self.max_len {
            self.data.pop_front();
        }
        self.data.push_back([x, y]);
        self.dirty = true;
    }

    /// 获取所有数据点引用
    pub fn points(&mut self) -> &[[f64; 2]] {
        if self.dirty {
            self.cached_points.clear();
            self.cached_points.extend(self.data.iter().copied());
            self.dirty = false;
        }
        self.cached_points.as_slice()
    }

    /// 清空数据
    pub fn clear(&mut self) {
        self.data.clear();
    }

    /// 当前采样数
    pub fn len(&self) -> usize {
        self.data.len()
    }

    /// 界面显示名：重命名优先，否则用通道原始名
    pub fn label(&self) -> &str {
        self.display_name.as_deref().unwrap_or(&self.name)
    }

    /// 最近一个采样值（无数据时返回 None），用于按通道顺序展示实时数值
    pub fn last_value(&self) -> Option<f64> {
        self.data.back().map(|point| point[1])
    }
}

// ─── 默认出厂参数 ────────────────────────────────────────────

pub fn default_params() -> Vec<DeviceParam> {
    vec![
        // 左电机 PID
        DeviceParam {
            name: "左轮 目标转速".into(),
            value: 200.0,
            min: 0.0,
            max: 5000.0,
            step: 10.0,
        },
        DeviceParam {
            name: "左轮 Kp".into(),
            value: 1.5,
            min: 0.0,
            max: 50.0,
            step: 0.1,
        },
        DeviceParam {
            name: "左轮 Ki".into(),
            value: 0.1,
            min: 0.0,
            max: 10.0,
            step: 0.01,
        },
        DeviceParam {
            name: "左轮 Kd".into(),
            value: 0.05,
            min: 0.0,
            max: 10.0,
            step: 0.01,
        },
        DeviceParam {
            name: "左轮 输出限幅".into(),
            value: 8399.0,
            min: 0.0,
            max: 8399.0,
            step: 100.0,
        },
        // 右电机 PID
        DeviceParam {
            name: "右轮 目标转速".into(),
            value: 200.0,
            min: 0.0,
            max: 5000.0,
            step: 10.0,
        },
        DeviceParam {
            name: "右轮 Kp".into(),
            value: 1.5,
            min: 0.0,
            max: 50.0,
            step: 0.1,
        },
        DeviceParam {
            name: "右轮 Ki".into(),
            value: 0.1,
            min: 0.0,
            max: 10.0,
            step: 0.01,
        },
        DeviceParam {
            name: "右轮 Kd".into(),
            value: 0.05,
            min: 0.0,
            max: 10.0,
            step: 0.01,
        },
        DeviceParam {
            name: "右轮 输出限幅".into(),
            value: 8399.0,
            min: 0.0,
            max: 8399.0,
            step: 100.0,
        },
    ]
}
