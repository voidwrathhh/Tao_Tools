// ======================== 本地配置 ========================
//
// 保存默认波特率、串口参数（数据位 / 停止位 / 校验位）与 TUI 的初始显示 /
// 发送选项，供 `sertool settings` 编辑、`sertool connect` 启动时读取。
//
// 文件位置：~/.tao_tools/sertool/config.ini
// 格式：每行 `key=value`，`#` 开头为注释；不引入额外依赖。
// 新增字段一律有默认值：旧配置文件缺字段时按 8-N-1 补齐，不会启动失败。

use std::fs;
use std::path::PathBuf;

use serialport::{DataBits, Parity, StopBits};

const DEFAULT_BAUD: u32 = 115200;

// ---------------------------------------------------------------------------
//  串口参数（数据位 / 停止位 / 校验位）
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DataBitsSel {
    Five,
    Six,
    Seven,
    Eight,
}

impl DataBitsSel {
    pub const ALL: [DataBitsSel; 4] = [Self::Five, Self::Six, Self::Seven, Self::Eight];

    pub fn value(self) -> u8 {
        match self {
            Self::Five => 5,
            Self::Six => 6,
            Self::Seven => 7,
            Self::Eight => 8,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Five => "5",
            Self::Six => "6",
            Self::Seven => "7",
            Self::Eight => "8",
        }
    }

    pub fn from_value(value: u8) -> Option<Self> {
        Self::ALL.iter().copied().find(|d| d.value() == value)
    }

    pub fn to_serialport(self) -> DataBits {
        match self {
            Self::Five => DataBits::Five,
            Self::Six => DataBits::Six,
            Self::Seven => DataBits::Seven,
            Self::Eight => DataBits::Eight,
        }
    }

    /// 面板 ←→ 切换（`forward` 为 false 时反向）
    pub fn cycle(self, forward: bool) -> Self {
        let index = Self::ALL.iter().position(|d| *d == self).unwrap_or(3);
        let len = Self::ALL.len();
        let next = if forward {
            (index + 1) % len
        } else {
            (index + len - 1) % len
        };
        Self::ALL[next]
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StopBitsSel {
    One,
    OnePointFive,
    Two,
}

impl StopBitsSel {
    pub const ALL: [StopBitsSel; 3] = [Self::One, Self::OnePointFive, Self::Two];

    pub fn label(self) -> &'static str {
        match self {
            Self::One => "1",
            Self::OnePointFive => "1.5",
            Self::Two => "2",
        }
    }

    /// 写入 config.ini 的取值（1 / 1.5 / 2）
    pub fn to_ini(self) -> &'static str {
        self.label()
    }

    pub fn from_ini(value: &str) -> Option<Self> {
        Self::ALL.iter().copied().find(|s| s.to_ini() == value)
    }

    /// serialport 4 只提供 1 / 2 两种停止位；1.5 无法表达，返回 None
    pub fn to_serialport(self) -> Option<StopBits> {
        match self {
            Self::One => Some(StopBits::One),
            Self::OnePointFive => None,
            Self::Two => Some(StopBits::Two),
        }
    }

    pub fn cycle(self, forward: bool) -> Self {
        let index = Self::ALL.iter().position(|s| *s == self).unwrap_or(0);
        let len = Self::ALL.len();
        let next = if forward {
            (index + 1) % len
        } else {
            (index + len - 1) % len
        };
        Self::ALL[next]
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ParitySel {
    None,
    Even,
    Odd,
}

impl ParitySel {
    pub const ALL: [ParitySel; 3] = [Self::None, Self::Even, Self::Odd];

    pub fn label(self) -> &'static str {
        match self {
            Self::None => "None",
            Self::Even => "Even",
            Self::Odd => "Odd",
        }
    }

    pub fn to_ini(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Even => "even",
            Self::Odd => "odd",
        }
    }

    pub fn from_ini(value: &str) -> Option<Self> {
        Self::ALL
            .iter()
            .copied()
            .find(|p| p.to_ini() == value.to_ascii_lowercase())
    }

    pub fn to_serialport(self) -> Parity {
        match self {
            Self::None => Parity::None,
            Self::Even => Parity::Even,
            Self::Odd => Parity::Odd,
        }
    }

    pub fn cycle(self, forward: bool) -> Self {
        let index = Self::ALL.iter().position(|p| *p == self).unwrap_or(0);
        let len = Self::ALL.len();
        let next = if forward {
            (index + 1) % len
        } else {
            (index + len - 1) % len
        };
        Self::ALL[next]
    }
}

/// 一条串口帧格式，例如 8-N-1
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SerialParams {
    pub data_bits: DataBitsSel,
    pub stop_bits: StopBitsSel,
    pub parity: ParitySel,
}

impl Default for SerialParams {
    /// 8-N-1：与旧版本行为一致，也是所有平台的通用回退值
    fn default() -> Self {
        Self {
            data_bits: DataBitsSel::Eight,
            stop_bits: StopBitsSel::One,
            parity: ParitySel::None,
        }
    }
}

impl SerialParams {
    /// 人类可读的帧格式，如 `8-N-1`
    pub fn display(&self) -> String {
        format!(
            "{}-{}-{}",
            self.data_bits.label(),
            match self.parity {
                ParitySel::None => "N",
                ParitySel::Even => "E",
                ParitySel::Odd => "O",
            },
            self.stop_bits.label()
        )
    }

    /// 标题栏用的紧凑格式，如 `8N1`
    pub fn compact(&self) -> String {
        format!(
            "{}{}{}",
            self.data_bits.value(),
            match self.parity {
                ParitySel::None => "N",
                ParitySel::Even => "E",
                ParitySel::Odd => "O",
            },
            self.stop_bits.label()
        )
    }

    /// 校验参数组合能否真正应用到串口。
    ///
    /// 目前唯一无法表达的是停止位 1.5：`serialport` 4 的 `StopBits` 只有
    /// 1 和 2，且多数平台的驱动也不支持。其余组合由驱动决定能否生效。
    pub fn validate(&self) -> Result<(), String> {
        if self.stop_bits == StopBitsSel::OnePointFive {
            return Err("停止位 1.5 无法应用到串口（serialport 只支持 1 / 2）".to_string());
        }
        Ok(())
    }

    /// 求实际生效的参数：非法组合给出中文提示并整体回退到 8-N-1
    pub fn resolve(&self) -> (Self, Option<String>) {
        match self.validate() {
            Ok(()) => (*self, None),
            Err(msg) => (
                Self::default(),
                Some(format!("{}；已回退到 8-N-1", msg)),
            ),
        }
    }
}

// ---------------------------------------------------------------------------
//  配置
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct Settings {
    /// 上次使用的端口（仅作提示，不自动连接）
    pub last_port: String,
    /// 默认波特率
    pub baud: u32,
    /// 默认串口帧格式（数据位 / 停止位 / 校验位）
    pub params: SerialParams,
    /// 接收显示默认为 HEX
    pub hex_mode: bool,
    /// 发送默认为 HEX
    pub hex_send: bool,
    /// 回车即发送
    pub enter_sends: bool,
    /// 发送时追加 \r\n
    pub append_crlf: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            last_port: String::new(),
            baud: DEFAULT_BAUD,
            params: SerialParams::default(),
            hex_mode: false,
            hex_send: false,
            enter_sends: true,
            append_crlf: true,
        }
    }
}

impl Settings {
    pub fn path() -> PathBuf {
        let base = std::env::var_os("USERPROFILE")
            .or_else(|| std::env::var_os("HOME"))
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("."));
        base.join(".tao_tools").join("sertool").join("config.ini")
    }

    /// 读取配置；文件不存在或损坏时返回默认值
    pub fn load() -> Self {
        match fs::read_to_string(Self::path()) {
            Ok(content) => Self::parse(&content),
            Err(_) => Self::default(),
        }
    }

    /// 解析配置文件内容（纯函数，未知 / 非法取值一律落到默认值）
    pub fn parse(content: &str) -> Self {
        let mut s = Self::default();
        for line in content.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let Some((key, value)) = line.split_once('=') else {
                continue;
            };
            let key = key.trim();
            let value = value.trim();
            match key {
                "last_port" => s.last_port = value.to_string(),
                "baud" => {
                    if let Ok(v) = value.parse::<u32>() {
                        if v > 0 {
                            s.baud = v;
                        }
                    }
                }
                "data_bits" => {
                    if let Ok(v) = value.parse::<u8>() {
                        if let Some(bits) = DataBitsSel::from_value(v) {
                            s.params.data_bits = bits;
                        }
                    }
                }
                "stop_bits" => {
                    if let Some(stop) = StopBitsSel::from_ini(value) {
                        s.params.stop_bits = stop;
                    }
                }
                "parity" => {
                    if let Some(parity) = ParitySel::from_ini(value) {
                        s.params.parity = parity;
                    }
                }
                "hex_mode" => s.hex_mode = parse_bool(value, s.hex_mode),
                "hex_send" => s.hex_send = parse_bool(value, s.hex_send),
                "enter_sends" => s.enter_sends = parse_bool(value, s.enter_sends),
                "append_crlf" => s.append_crlf = parse_bool(value, s.append_crlf),
                _ => {}
            }
        }
        s
    }

    /// 序列化为配置文件内容（纯函数）
    pub fn render(&self) -> String {
        format!(
            "# sertool 配置（由 `sertool settings` 生成）\n\
             last_port={}\n\
             baud={}\n\
             data_bits={}\n\
             stop_bits={}\n\
             parity={}\n\
             hex_mode={}\n\
             hex_send={}\n\
             enter_sends={}\n\
             append_crlf={}\n",
            self.last_port,
            self.baud,
            self.params.data_bits.value(),
            self.params.stop_bits.to_ini(),
            self.params.parity.to_ini(),
            self.hex_mode,
            self.hex_send,
            self.enter_sends,
            self.append_crlf
        )
    }

    /// 写入配置，返回错误信息供调用方提示
    pub fn save(&self) -> Result<(), String> {
        let path = Self::path();
        if let Some(dir) = path.parent() {
            fs::create_dir_all(dir).map_err(|e| format!("创建目录失败 ({}): {}", dir.display(), e))?;
        }
        fs::write(&path, self.render()).map_err(|e| format!("写入失败 ({}): {}", path.display(), e))
    }
}

fn parse_bool(value: &str, fallback: bool) -> bool {
    match value.to_ascii_lowercase().as_str() {
        "1" | "true" | "yes" | "on" => true,
        "0" | "false" | "no" | "off" => false,
        _ => fallback,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 旧配置文件（没有 data_bits / stop_bits / parity）必须读成 8-N-1
    #[test]
    fn legacy_config_defaults_to_8n1() {
        let old = "last_port=COM7\nbaud=9600\nhex_mode=true\nhex_send=false\nenter_sends=1\nappend_crlf=0\n";
        let s = Settings::parse(old);
        assert_eq!(s.last_port, "COM7");
        assert_eq!(s.baud, 9600);
        assert!(s.hex_mode);
        assert!(!s.hex_send);
        assert!(s.enter_sends);
        assert!(!s.append_crlf);
        assert_eq!(s.params, SerialParams::default());
        assert_eq!(s.params.display(), "8-N-1");
    }

    /// 非法取值（数据位 9、停止位 3、校验 Mark）一律忽略，保留 8-N-1
    #[test]
    fn invalid_config_values_fall_back() {
        let bad = "data_bits=9\nstop_bits=3\nparity=mark\n";
        let s = Settings::parse(bad);
        assert_eq!(s.params.display(), "8-N-1");
    }

    /// 合法参数可解析，并能原样写回
    #[test]
    fn params_round_trip_through_config() {
        let s = Settings {
            params: SerialParams {
                data_bits: DataBitsSel::Seven,
                stop_bits: StopBitsSel::Two,
                parity: ParitySel::Even,
            },
            ..Settings::default()
        };
        let rendered = s.render();
        assert!(rendered.contains("data_bits=7"));
        assert!(rendered.contains("stop_bits=2"));
        assert!(rendered.contains("parity=even"));
        let back = Settings::parse(&rendered);
        assert_eq!(back.params, s.params);
        assert_eq!(back.params.display(), "7-E-2");
    }

    /// 1.5 停止位无法应用到 serialport，resolve 回退 8-N-1 并给出中文提示
    #[test]
    fn one_point_five_stop_bits_falls_back_to_8n1() {
        let params = SerialParams {
            stop_bits: StopBitsSel::OnePointFive,
            ..SerialParams::default()
        };
        assert!(params.validate().is_err());
        let (effective, warning) = params.resolve();
        assert_eq!(effective, SerialParams::default());
        let warning = warning.expect("应给出回退提示");
        assert!(warning.contains("1.5"), "{}", warning);
        assert!(warning.contains("8-N-1"), "{}", warning);
        assert!(StopBitsSel::OnePointFive.to_serialport().is_none());
    }

    /// 合法组合 resolve 不提示
    #[test]
    fn valid_params_resolve_without_warning() {
        let params = SerialParams {
            data_bits: DataBitsSel::Seven,
            stop_bits: StopBitsSel::Two,
            parity: ParitySel::Odd,
        };
        assert_eq!(params.resolve(), (params, None));
        assert_eq!(params.compact(), "7O2");
    }

    /// 面板循环选择：正向 / 反向都能回绕
    #[test]
    fn selectors_cycle_and_wrap() {
        assert_eq!(DataBitsSel::Eight.cycle(true), DataBitsSel::Five);
        assert_eq!(DataBitsSel::Five.cycle(false), DataBitsSel::Eight);
        assert_eq!(StopBitsSel::Two.cycle(true), StopBitsSel::One);
        assert_eq!(StopBitsSel::One.cycle(false), StopBitsSel::Two);
        assert_eq!(ParitySel::Odd.cycle(true), ParitySel::None);
        assert_eq!(ParitySel::None.cycle(false), ParitySel::Odd);
        // 5 数据位 + 1.5 停止位在 serialport 下同样无法应用
        assert_eq!(
            SerialParams {
                data_bits: DataBitsSel::Five,
                stop_bits: StopBitsSel::OnePointFive,
                parity: ParitySel::Even,
            }
            .resolve()
            .0,
            SerialParams::default()
        );
    }
}
