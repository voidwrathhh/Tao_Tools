// ======================== 数据结构定义 ========================
//
// 本模块定义 blueinfo 使用的所有数据结构，包括：
//   1. BinaryPacket — bincode 二进制协议帧（高频结构化数据）
//   2. PlotChannel  — 绘图曲线通道（含定长环形缓冲区）
//   3. DeviceParam  — 设备参数项
//   4. ParamValue   — UI 参数编辑状态
//
// ─── C 语言结构体对齐参考 ───
// 若在嵌入式设备（C 语言）侧发送二进制帧，建议定义如下结构体：
//
//   #pragma pack(1)  // 字节对齐，取消 padding
//   typedef struct {
//       uint8_t  magic[4];      // "BLUE" 魔数
//       uint32_t length;        // 载荷长度（小端序）
//       double   timestamp;     // 时间戳（秒）
//       uint8_t  channel_count; // 通道数量
//       double   channels[8];   // 各通道数值
//       uint32_t checksum;      // 简单 XOR 校验和
//   } BluePacket;
//   #pragma pack()
//
// Rust 侧 bincode 序列化默认使用小端序 + 紧凑格式，
// 与 C 层 pack(1) 对齐后二进制布局完全一致。

use serde::{Deserialize, Serialize};
use std::collections::VecDeque;

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
    pub name: String,
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
