// ======================== 本地配置 ========================
//
// 保存默认波特率与 TUI 的初始显示/发送选项，供 `sertool settings`
// 编辑、`sertool connect` 启动时读取。
//
// 文件位置：~/.tao_tools/sertool/config.ini
// 格式：每行 `key=value`，`#` 开头为注释；不引入额外依赖。

use std::fs;
use std::path::PathBuf;

const DEFAULT_BAUD: u32 = 115200;

#[derive(Debug, Clone)]
pub struct Settings {
    /// 上次使用的端口（仅作提示，不自动连接）
    pub last_port: String,
    /// 默认波特率
    pub baud: u32,
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
        let path = Self::path();
        let content = match fs::read_to_string(&path) {
            Ok(c) => c,
            Err(_) => return Self::default(),
        };

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
                "hex_mode" => s.hex_mode = parse_bool(value, s.hex_mode),
                "hex_send" => s.hex_send = parse_bool(value, s.hex_send),
                "enter_sends" => s.enter_sends = parse_bool(value, s.enter_sends),
                "append_crlf" => s.append_crlf = parse_bool(value, s.append_crlf),
                _ => {}
            }
        }
        s
    }

    /// 写入配置，返回错误信息供调用方提示
    pub fn save(&self) -> Result<(), String> {
        let path = Self::path();
        if let Some(dir) = path.parent() {
            fs::create_dir_all(dir).map_err(|e| format!("创建目录失败 ({}): {}", dir.display(), e))?;
        }
        let content = format!(
            "# sertool 配置（由 `sertool settings` 生成）\n\
             last_port={}\n\
             baud={}\n\
             hex_mode={}\n\
             hex_send={}\n\
             enter_sends={}\n\
             append_crlf={}\n",
            self.last_port,
            self.baud,
            self.hex_mode,
            self.hex_send,
            self.enter_sends,
            self.append_crlf
        );
        fs::write(&path, content).map_err(|e| format!("写入失败 ({}): {}", path.display(), e))
    }
}

fn parse_bool(value: &str, fallback: bool) -> bool {
    match value.to_ascii_lowercase().as_str() {
        "1" | "true" | "yes" | "on" => true,
        "0" | "false" | "no" | "off" => false,
        _ => fallback,
    }
}
