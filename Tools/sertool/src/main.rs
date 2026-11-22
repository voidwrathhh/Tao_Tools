use std::collections::VecDeque;
use std::io::{self, Read, Write};
use std::time::Duration;

use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use ratatui::{
    layout::Layout,
    style::{Color, Modifier, Style},
    text::{Line, Span, Text},
    widgets::{Block, Borders, Paragraph, Wrap},
    DefaultTerminal,
};

mod settings;

use settings::Settings;

const APP_NAME: &str = "sertool";
const VERSION: &str = env!("CARGO_PKG_VERSION");
const MAX_LINES: usize = 2000;
const MAX_INPUT_LEN: usize = 4096;
const COMMON_BAUD_RATES: &[u32] = &[9600, 19200, 38400, 57600, 115200, 230400, 460800, 921600];

/// HEX 显示每行的字节数
const HEX_BYTES_PER_ROW: usize = 16;
/// 文本 pending 上限；超过即强制断行，避免无换行设备把内存撑爆
const MAX_PENDING_BYTES: usize = 8 * 1024;
/// 串口读超时。它同时是「自然 flush 点」的静默阈值：
/// 太短会把慢速设备的多段输出切成多行，太长则无换行设备看不到提示符。
const READ_TIMEOUT: Duration = Duration::from_millis(50);

// ---------------------------------------------------------------------------
//  ANSI 解析
// ---------------------------------------------------------------------------

#[derive(Clone, Copy)]
struct AnsiState {
    fg: Option<Color>,
    bold: bool,
}

impl AnsiState {
    fn reset() -> Self {
        Self {
            fg: None,
            bold: false,
        }
    }

    fn to_style(&self) -> Style {
        let mut s = Style::default();
        if let Some(c) = self.fg {
            s = s.fg(c);
        }
        if self.bold {
            s = s.add_modifier(Modifier::BOLD);
        }
        s
    }
}

/// 将 ANSI 转义码解析为 (文本, 样式) 片段列表
fn parse_ansi(text: &str) -> Vec<(String, Style)> {
    let mut segments: Vec<(String, Style)> = Vec::new();
    let mut state = AnsiState::reset();
    let mut buf = String::new();
    let mut chars = text.chars().peekable();

    while let Some(c) = chars.next() {
        if c == '\x1b' && chars.peek() == Some(&'[') {
            if !buf.is_empty() {
                segments.push((buf.clone(), state.to_style()));
                buf.clear();
            }
            chars.next(); // skip '['
            let mut params = String::new();
            while let Some(&n) = chars.peek() {
                if n >= '\x40' && n <= '\x7e' {
                    if n == 'm' {
                        apply_sgr(&mut state, &params);
                    }
                    chars.next();
                    break;
                }
                params.push(n);
                chars.next();
            }
        } else {
            buf.push(c);
        }
    }
    if !buf.is_empty() {
        segments.push((buf, state.to_style()));
    }
    segments
}

fn apply_sgr(state: &mut AnsiState, params: &str) {
    if params.is_empty() || params == "0" {
        *state = AnsiState::reset();
        return;
    }
    for p in params.split(';') {
        match p {
            "0" => *state = AnsiState::reset(),
            "1" => state.bold = true,
            "22" => state.bold = false,
            "30" => state.fg = Some(Color::Black),
            "31" => state.fg = Some(Color::Red),
            "32" => state.fg = Some(Color::Green),
            "33" => state.fg = Some(Color::Yellow),
            "34" => state.fg = Some(Color::Blue),
            "35" => state.fg = Some(Color::Magenta),
            "36" => state.fg = Some(Color::Cyan),
            "37" => state.fg = Some(Color::White),
            "90" => state.fg = Some(Color::DarkGray),
            "91" => state.fg = Some(Color::LightRed),
            "92" => state.fg = Some(Color::LightGreen),
            "93" => state.fg = Some(Color::LightYellow),
            "94" => state.fg = Some(Color::LightBlue),
            "95" => state.fg = Some(Color::LightMagenta),
            "96" => state.fg = Some(Color::LightCyan),
            "97" => state.fg = Some(Color::White),
            _ => {}
        }
    }
}

/// 过滤控制字符但保留 ANSI 转义码（入参必须是已解码的合法 UTF-8 文本）
fn clean_text(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '\x1b' => {
                out.push(c); // 保留 ESC（ANSI 转义序列的起始）
            }
            '\n' | '\t' => out.push(c),
            '\r' => {}                      // 移除回车符
            c if c.is_ascii_control() => {} // 移除其他控制字符
            c => out.push(c),
        }
    }
    out
}

// ---------------------------------------------------------------------------
//  CLI 命令
// ---------------------------------------------------------------------------

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 2 {
        print_usage();
        return;
    }
    match args[1].as_str() {
        "list" => list_ports(),
        "settings" => run_settings(),
        "connect" => {
            if args.len() < 3 {
                eprintln!("错误：缺少端口名");
                eprintln!("用法：{} connect <端口> [波特率]", APP_NAME);
                std::process::exit(1);
            }
            let baud: u32 = args
                .get(3)
                .and_then(|s| s.parse().ok())
                .unwrap_or_else(|| Settings::load().baud);
            connect(&args[2], baud);
        }
        "--help" | "-h" => print_usage(),
        "--version" | "-v" => println!("{} v{}", APP_NAME, VERSION),
        _ => {
            eprintln!("未知命令：{}", args[1]);
            print_usage();
        }
    }
}

fn print_usage() {
    println!("{} v{} - 串口终端工具", APP_NAME, VERSION);
    println!();
    println!("用法：");
    println!("  {} list                       列出可用串口", APP_NAME);
    println!(
        "  {} settings                   打开设置界面（默认波特率 / HEX / 发送选项）",
        APP_NAME
    );
    println!(
        "  {} connect <端口> [波特率]     连接串口（波特率缺省取配置，默认 115200）",
        APP_NAME
    );
    println!("  {} --version, -v               显示版本", APP_NAME);
    println!("  {} --help, -h                  显示帮助", APP_NAME);
    println!();
    println!("示例：");
    println!("  {} list", APP_NAME);
    println!("  {} connect COM3", APP_NAME);
    println!("  {} connect COM3 9600", APP_NAME);
    println!("  {} connect /dev/ttyUSB0", APP_NAME);
    println!("  {} connect /dev/ttyUSB0 115200", APP_NAME);
    println!();
    println!("TUI 连接后操作：");
    println!("  - 上方区域显示设备串口数据（可滚动，支持 ANSI 颜色 / HEX 切换）");
    println!("  - 下方输入框打字，回车发送");
    println!("  - ↑↓/滚轮 滚动历史数据，PgUp/PgDn 翻页");
    println!("  - Ctrl+D 断开/重连串口，Ctrl+B 切换波特率");
    println!("  - Ctrl+H 切换 HEX/文本显示，Ctrl+S 切换 HEX/文本发送");
    println!("  - Ctrl+E 切换回车发送/换行，Ctrl+L 切换追加 \\r\\n");
    println!("  - Ctrl+Enter 强制发送，Esc 或 Ctrl+C 退出");
}

fn list_ports() {
    match serialport::available_ports() {
        Ok(ports) => {
            if ports.is_empty() {
                println!("未检测到串口。");
                return;
            }
            println!("检测到 {} 个串口：\n", ports.len());
            for (i, port) in ports.iter().enumerate() {
                let (desc, vid, pid) = match &port.port_type {
                    serialport::SerialPortType::UsbPort(info) => (
                        info.product.clone().unwrap_or_default(),
                        format!("{:04X}", info.vid),
                        format!("{:04X}", info.pid),
                    ),
                    serialport::SerialPortType::BluetoothPort => {
                        ("蓝牙设备".into(), String::new(), String::new())
                    }
                    _ => (String::new(), String::new(), String::new()),
                };
                println!("  {}. {}", i + 1, port.port_name);
                if !desc.is_empty() {
                    println!("     描述        : {}", desc);
                }
                if !vid.is_empty() && !pid.is_empty() {
                    println!("     USB VID:PID : {}:{}", vid, pid);
                }
                println!();
            }
        }
        Err(e) => eprintln!("扫描串口失败：{}", e),
    }
}

// ---------------------------------------------------------------------------
//  解析后的一行输出：多个带样式的文本片段
// ---------------------------------------------------------------------------

type StyledSegments = Vec<(String, Style)>;

// ---------------------------------------------------------------------------
//  HEX 格式化
// ---------------------------------------------------------------------------

/// 将一行字节格式化为 "00 01 02 ..." 形式的十六进制文本
fn hex_row(chunk: &[u8]) -> String {
    let mut s = String::with_capacity(chunk.len() * 3);
    for (i, byte) in chunk.iter().enumerate() {
        if i > 0 {
            s.push(' ');
        }
        s.push_str(&format!("{:02X}", byte));
    }
    s
}

/// 将字节数据格式化为十六进制字符串（每行 16 字节，用于发送回显）
fn format_as_hex(data: &[u8]) -> String {
    data.chunks(HEX_BYTES_PER_ROW)
        .map(hex_row)
        .collect::<Vec<_>>()
        .join("\n")
}

/// 将字节数据格式化为十六进制显示行：每行 16 字节，首列带 `0x%04x` 地址偏移。
///
/// `offset` 是本块之前已显示的字节数，因此多块数据拼起来后地址连续。
fn format_as_hex_lines(data: &[u8], offset: u64) -> Vec<String> {
    data.chunks(HEX_BYTES_PER_ROW)
        .enumerate()
        .map(|(row, chunk)| {
            let addr = offset + (row * HEX_BYTES_PER_ROW) as u64;
            format!("0x{:04x}  {}", addr, hex_row(chunk))
        })
        .collect()
}

/// 将十六进制字符串解析为字节数组（支持 "FF A0 01" 或 "FFA001" 格式）
fn parse_hex_input(s: &str) -> Result<Vec<u8>, String> {
    let hex: String = s.chars().filter(|c| !c.is_whitespace()).collect();
    if hex.len() % 2 != 0 {
        return Err("十六进制字符串长度必须为偶数".to_string());
    }
    let bytes: Result<Vec<u8>, _> = (0..hex.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&hex[i..i + 2], 16))
        .collect();
    bytes.map_err(|_| "包含无效的十六进制字符".to_string())
}

// ---------------------------------------------------------------------------
//  接收管线：跨块增量 UTF-8 解码 + 文本 pending + HEX 地址偏移
// ---------------------------------------------------------------------------

/// 跨块增量 UTF-8 解码器。
///
/// 串口按 1KB 分块读取，一个多字节字符可能被切在两次 read 之间。这里保留
/// 末尾不足 4 字节的不完整序列等下一块拼接；真正的非法字节替换为 `�` 后
/// 继续解码，保证既不丢数据也不把整个块判死。
#[derive(Default)]
struct Utf8Decoder {
    /// 上一块末尾未凑成完整字符的字节（至多 3 字节）
    tail: Vec<u8>,
}

impl Utf8Decoder {
    fn new() -> Self {
        Self::default()
    }

    /// 接受一块新数据，返回已能完整解码的文本（不含保留下来的残尾）
    fn push(&mut self, data: &[u8]) -> String {
        let mut bytes = std::mem::take(&mut self.tail);
        bytes.extend_from_slice(data);

        let mut out = String::with_capacity(bytes.len());
        let mut rest = bytes.as_slice();
        loop {
            match std::str::from_utf8(rest) {
                Ok(s) => {
                    out.push_str(s);
                    break;
                }
                Err(err) => {
                    let valid = err.valid_up_to();
                    // valid_up_to 之前必定是合法 UTF-8
                    if let Ok(s) = std::str::from_utf8(&rest[..valid]) {
                        out.push_str(s);
                    }
                    match err.error_len() {
                        // 真正的非法字节：只消费到 valid_up_to，跳过坏字节继续
                        Some(len) => {
                            out.push('\u{FFFD}');
                            rest = &rest[valid + len..];
                        }
                        // 块尾是多字节字符的前缀：保留下来等下一块
                        None => {
                            self.tail.extend_from_slice(&rest[valid..]);
                            break;
                        }
                    }
                }
            }
        }
        out
    }

    /// 丢弃残尾（断开 / 重连 / 切换显示模式时调用）
    fn reset(&mut self) {
        self.tail.clear();
    }

    #[cfg(test)]
    fn tail_len(&self) -> usize {
        self.tail.len()
    }
}

/// 返回 `s` 中不超过 `max` 的最大字符边界下标
fn char_boundary_at_most(s: &str, max: usize) -> usize {
    let mut i = max.min(s.len());
    while i > 0 && !s.is_char_boundary(i) {
        i -= 1;
    }
    i
}

/// 接收显示缓冲：增量解码 + 未成行的文本 pending + HEX 地址偏移。
///
/// 抽成独立结构是为了让「跨块解码 / 断行 / 偏移累计」这些纯逻辑可以脱离
/// 串口和 TUI 单元测试。
#[derive(Default)]
struct RxBuffer {
    decoder: Utf8Decoder,
    /// 已解码但还没凑到换行的文本
    pending: String,
    /// HEX 模式下已输出的字节数，用作下一行的起始地址
    hex_offset: u64,
}

impl RxBuffer {
    fn new() -> Self {
        Self {
            decoder: Utf8Decoder::new(),
            pending: String::new(),
            hex_offset: 0,
        }
    }

    /// 清空接收侧状态（地址偏移归零、丢弃 pending 与解码残尾）
    fn reset(&mut self) {
        self.decoder.reset();
        self.pending.clear();
        self.hex_offset = 0;
    }

    /// 处理一块收到的数据，返回应追加到输出区的显示行（每个元素 = 一条 Line）。
    ///
    /// `flush` 为 true 时表示到了自然 flush 点（读超时），把 pending 整块吐出，
    /// 让只发 `>` / 进度条这类没有换行符的设备也能立刻看到内容。
    fn push(&mut self, data: &[u8], hex: bool, flush: bool) -> Vec<String> {
        if hex {
            let lines = format_as_hex_lines(data, self.hex_offset);
            self.hex_offset += data.len() as u64;
            return lines;
        }

        let text = clean_text(&self.decoder.push(data));
        self.pending.push_str(&text);

        let mut lines = Vec::new();
        // 1) 先把所有完整行（以 \n 结尾）拆出来
        if let Some(pos) = self.pending.rfind('\n') {
            let complete: String = self.pending.drain(..=pos).collect();
            lines.extend(complete.split_inclusive('\n').map(str::to_string));
        }
        // 2) flush 点：不完整的最后一行也整块输出
        if flush && !self.pending.is_empty() {
            lines.push(std::mem::take(&mut self.pending));
        }
        // 3) pending 上限：超限时在字符边界处强制断行，输出区可继续滚动查看
        while self.pending.len() > MAX_PENDING_BYTES {
            let cut = char_boundary_at_most(&self.pending, MAX_PENDING_BYTES);
            if cut == 0 {
                // 理论上不会发生（下标 0 必定是字符边界），防御性退出避免死循环
                break;
            }
            let head: String = self.pending.drain(..cut).collect();
            lines.push(head);
        }
        lines
    }
}

// ---------------------------------------------------------------------------
//  TUI 连接
// ---------------------------------------------------------------------------

fn connect(port_name: &str, baud_rate: u32) {
    // 记住本次端口与波特率，下次 `sertool settings` 里能看到
    let mut settings = Settings::load();
    settings.last_port = port_name.to_string();
    settings.baud = baud_rate;
    let _ = settings.save();

    let _ = crossterm::execute!(std::io::stdout(), crossterm::event::EnableMouseCapture);
    let mut terminal = ratatui::init();
    let mut app = App::new(port_name.to_string(), baud_rate, &settings);
    let res = run_tui(&mut app, &mut terminal);
    ratatui::restore();
    let _ = crossterm::execute!(std::io::stdout(), crossterm::event::DisableMouseCapture);

    if let Err(e) = res {
        eprintln!("TUI 错误：{}", e);
    }
}

// ---------------------------------------------------------------------------
//  设置界面
// ---------------------------------------------------------------------------

const SETTING_ROWS: usize = 5;

fn run_settings() {
    let mut settings = Settings::load();
    let mut terminal = ratatui::init();
    let result = settings_loop(&mut settings, &mut terminal);
    ratatui::restore();

    match result {
        Ok(msg) => println!("{}", msg),
        Err(e) => eprintln!("TUI 错误：{}", e),
    }
}

fn settings_loop(settings: &mut Settings, terminal: &mut DefaultTerminal) -> io::Result<String> {
    let mut selected = 0usize;

    loop {
        terminal.draw(|frame| {
            let chunks = Layout::vertical([
                ratatui::layout::Constraint::Min(9),
                ratatui::layout::Constraint::Length(4),
            ])
            .split(frame.area());

            let rows = [
                format!("默认波特率        : {}  （← → 切换）", settings.baud),
                format!("接收显示模式      : {}", if settings.hex_mode { "HEX" } else { "文本" }),
                format!("发送模式          : {}", if settings.hex_send { "HEX" } else { "文本" }),
                format!("回车即发送        : {}", yes_no(settings.enter_sends)),
                format!("发送追加 \\r\\n     : {}", yes_no(settings.append_crlf)),
            ];

            let mut lines: Vec<Line> = Vec::new();
            lines.push(Line::from(""));
            for (i, text) in rows.iter().enumerate() {
                let marker = if i == selected { " ▶ " } else { "   " };
                let style = if i == selected {
                    Style::default()
                        .fg(Color::Cyan)
                        .add_modifier(Modifier::BOLD)
                } else {
                    Style::default()
                };
                lines.push(Line::from(Span::styled(
                    format!("{}{}", marker, text),
                    style,
                )));
            }
            lines.push(Line::from(""));
            lines.push(Line::from(Span::styled(
                format!(
                    "上次端口          : {}",
                    if settings.last_port.is_empty() {
                        "（无）"
                    } else {
                        settings.last_port.as_str()
                    }
                ),
                Style::default().fg(Color::DarkGray),
            )));
            lines.push(Line::from(Span::styled(
                format!("配置文件          : {}", Settings::path().display()),
                Style::default().fg(Color::DarkGray),
            )));

            let block = Block::default()
                .borders(Borders::ALL)
                .title(format!(" {} v{} 设置 ", APP_NAME, VERSION));
            frame.render_widget(Paragraph::new(Text::from(lines)).block(block), chunks[0]);

            let hint = "↑↓/jk 选择    ←→/Enter 修改    s 保存并退出    q/Esc 退出不保存";
            frame.render_widget(
                Paragraph::new(hint)
                    .wrap(Wrap { trim: true })
                    .block(Block::default().borders(Borders::ALL)),
                chunks[1],
            );
        })?;

        if !event::poll(Duration::from_millis(120))? {
            continue;
        }
        let Event::Key(key) = event::read()? else {
            continue;
        };
        if key.kind != KeyEventKind::Press {
            continue;
        }

        match key.code {
            KeyCode::Esc | KeyCode::Char('q') => {
                return Ok("已退出设置（未保存）".to_string());
            }
            KeyCode::Char('s') => {
                return match settings.save() {
                    Ok(()) => Ok(format!("设置已保存到 {}", Settings::path().display())),
                    Err(e) => Err(io::Error::other(format!("保存设置失败：{}", e))),
                };
            }
            KeyCode::Up | KeyCode::Char('k') => {
                selected = (selected + SETTING_ROWS - 1) % SETTING_ROWS;
            }
            KeyCode::Down | KeyCode::Char('j') => {
                selected = (selected + 1) % SETTING_ROWS;
            }
            KeyCode::Left | KeyCode::Right | KeyCode::Enter | KeyCode::Char(' ') => {
                let forward = key.code != KeyCode::Left;
                match selected {
                    0 => {
                        let current = COMMON_BAUD_RATES
                            .iter()
                            .position(|&b| b == settings.baud)
                            .unwrap_or(5);
                        let next = if forward {
                            (current + 1) % COMMON_BAUD_RATES.len()
                        } else {
                            (current + COMMON_BAUD_RATES.len() - 1) % COMMON_BAUD_RATES.len()
                        };
                        settings.baud = COMMON_BAUD_RATES[next];
                    }
                    1 => settings.hex_mode = !settings.hex_mode,
                    2 => settings.hex_send = !settings.hex_send,
                    3 => settings.enter_sends = !settings.enter_sends,
                    4 => settings.append_crlf = !settings.append_crlf,
                    _ => {}
                }
            }
            _ => {}
        }
    }
}

fn yes_no(value: bool) -> &'static str {
    if value {
        "是"
    } else {
        "否"
    }
}

// ---------------------------------------------------------------------------
//  应用状态
// ---------------------------------------------------------------------------

struct App {
    port_name: String,
    baud_rate: u32,
    port: Option<Box<dyn serialport::SerialPort>>,
    connected: bool,
    output: VecDeque<StyledSegments>,
    /// 接收侧状态：增量解码、文本 pending、HEX 地址偏移
    rx: RxBuffer,
    /// 自上次重置（连接 / 清屏）以来累计收到的字节数
    rx_bytes: u64,
    input: String,
    scroll: usize,
    status: String,
    running: bool,
    hex_mode: bool,
    hex_send: bool,
    manual_disconnect: bool,
    enter_sends: bool,
    append_crlf: bool,
}

impl App {
    fn new(port_name: String, baud_rate: u32, settings: &Settings) -> Self {
        let info = Self::open_port(&port_name, baud_rate);
        App {
            port_name,
            baud_rate,
            port: info.0,
            connected: info.1,
            output: VecDeque::with_capacity(MAX_LINES),
            rx: RxBuffer::new(),
            rx_bytes: 0,
            input: String::new(),
            scroll: 0,
            status: info.2,
            running: true,
            hex_mode: settings.hex_mode,
            hex_send: settings.hex_send,
            manual_disconnect: false,
            enter_sends: settings.enter_sends,
            append_crlf: settings.append_crlf,
        }
    }

    fn open_port(name: &str, baud: u32) -> (Option<Box<dyn serialport::SerialPort>>, bool, String) {
        // 读取超时同时充当 flush 阈值：没有数据时 read 返回 TimedOut，
        // 主循环借此把 pending 整块刷到输出区（见 READ_TIMEOUT 注释）
        let mut port = match serialport::new(name, baud)
            .timeout(READ_TIMEOUT)
            .flow_control(serialport::FlowControl::None)
            .open()
        {
            Ok(p) => p,
            Err(e) => return (None, false, format!("✗ 无法打开: {}", e)),
        };
        let _ = port.write_data_terminal_ready(true);
        let _ = port.write_request_to_send(false);
        (Some(port), true, format!("✓ 已连接 {} @ {}bps", name, baud))
    }

    /// 追加若干显示行，并裁剪到行数上限
    fn extend_output(&mut self, lines: Vec<String>) {
        if lines.is_empty() {
            return;
        }
        for line in lines {
            self.output.push_back(parse_ansi(&line));
        }
        while self.output.len() > MAX_LINES {
            self.output.pop_front();
        }
    }

    /// 清空接收侧残留状态：HEX 地址偏移、文本 pending、解码残尾与 RX 计数。
    /// 断开 / 重连 / 切换显示模式（等同于清屏）时调用。
    fn reset_rx(&mut self) {
        self.rx.reset();
        self.rx_bytes = 0;
    }

    /// 在输出区追加一条红色错误提示
    fn push_error(&mut self, msg: &str) {
        self.output
            .push_back(parse_ansi(&format!("\n\x1b[1;31m! {}\x1b[0m\n", msg)));
    }

    /// 读取串口数据，解析 ANSI 颜色或 HEX 显示，返回 true 表示有新数据（或刷出了 pending）
    fn read_serial(&mut self) -> bool {
        let port = match self.port.as_mut() {
            Some(p) => p,
            None => return false,
        };
        let mut buf = [0u8; 1024];
        let read = port.read(&mut buf);
        let hex_mode = self.hex_mode;
        match read {
            Ok(0) => false,
            Ok(n) => {
                self.rx_bytes = self.rx_bytes.saturating_add(n as u64);
                let lines = self.rx.push(&buf[..n], hex_mode, false);
                self.extend_output(lines);
                true
            }
            Err(ref e) if e.kind() == io::ErrorKind::TimedOut => {
                // 读超时 = 自然 flush 点：把 pending 整块吐出，
                // 让只输出 `>` 或进度条这类无换行数据的设备也能立刻看到内容
                let lines = self.rx.push(&[], hex_mode, true);
                let flushed = !lines.is_empty();
                self.extend_output(lines);
                flushed
            }
            Err(e) => {
                self.connected = false;
                self.port = None;
                self.status = format!("✗ {} 已断开 ({})", self.port_name, e);
                self.reset_rx();
                false
            }
        }
    }

    fn try_reconnect(&mut self) {
        let (port, ok, msg) = Self::open_port(&self.port_name, self.baud_rate);
        self.port = port;
        self.connected = ok;
        self.status = msg;
        self.reset_rx();
        if ok {
            self.output
                .push_back(parse_ansi("\n\x1b[1;33m--- 已重新连接 ---\x1b[0m\n"));
            self.scroll = 0;
        }
    }

    fn send_line(&mut self) {
        if self.input.is_empty() {
            return;
        }
        if !self.connected || self.port.is_none() {
            self.push_error("未连接，输入内容未发送（已保留）");
            return;
        }

        let hex_send = self.hex_send;
        let bytes = if hex_send {
            // HEX 发送模式：解析输入为十六进制字节
            match parse_hex_input(&self.input) {
                Ok(bytes) if bytes.is_empty() => {
                    self.push_error("HEX 输入为空，没有可发送的字节");
                    return;
                }
                Ok(bytes) => bytes,
                Err(e) => {
                    self.output
                        .push_back(parse_ansi(&format!("\n<<< HEX 解析错误: {} >>>\n", e)));
                    return;
                }
            }
        } else {
            let suffix = if self.append_crlf { "\r\n" } else { "" };
            format!("{}{}", self.input, suffix).into_bytes()
        };

        let port = match self.port.as_mut() {
            Some(p) => p,
            None => return,
        };
        let write_result = port.write_all(&bytes).and_then(|_| port.flush());
        match write_result {
            Ok(()) => {
                if hex_send {
                    // HEX 回显按 16 字节一行拆分，与接收区的行结构保持一致
                    for (i, row) in format_as_hex(&bytes).split('\n').enumerate() {
                        if i == 0 {
                            self.output
                                .push_back(parse_ansi(&format!("\n<<< {}\n", row)));
                        } else {
                            self.output
                                .push_back(parse_ansi(&format!("    {}\n", row)));
                        }
                    }
                } else {
                    let echo = self.input.clone();
                    self.output
                        .push_back(parse_ansi(&format!("\n<<< {}\n", echo)));
                }
                // 写成功才清空输入；失败时保留内容让用户重发
                self.input.clear();
            }
            Err(e) => {
                self.push_error(&format!("发送失败: {}（输入已保留）", e));
            }
        }
    }

    fn toggle_connection(&mut self) {
        if self.connected {
            // 用户主动断开
            self.port = None;
            self.connected = false;
            self.manual_disconnect = true;
            self.status = format!("✗ {} 已断开 (用户)", self.port_name);
            self.reset_rx();
            self.output
                .push_back(parse_ansi("\n\x1b[1;33m--- 已断开连接 ---\x1b[0m\n"));
        } else {
            // 重新打开
            self.manual_disconnect = false;
            let (port, ok, msg) = Self::open_port(&self.port_name, self.baud_rate);
            self.port = port;
            self.connected = ok;
            self.status = msg;
            self.reset_rx();
            if ok {
                self.output
                    .push_back(parse_ansi("\n\x1b[1;33m--- 已重新连接 ---\x1b[0m\n"));
                self.scroll = 0;
            }
        }
    }

    fn cycle_baud_rate(&mut self) {
        if self.connected {
            self.output.push_back(parse_ansi(
                "\n\x1b[1;33m! 请先断开连接再切换波特率 (Ctrl+D)\x1b[0m\n",
            ));
            return;
        }
        // 在当前常见波特率列表中找下一个
        let next = COMMON_BAUD_RATES
            .iter()
            .position(|&r| r == self.baud_rate)
            .map(|i| (i + 1) % COMMON_BAUD_RATES.len())
            .unwrap_or(0);
        self.baud_rate = COMMON_BAUD_RATES[next];
        self.status = format!("波特率: {}bps (未连接)", self.baud_rate);
        self.output.push_back(parse_ansi(&format!(
            "\n\x1b[1;33m--- 切换波特率至 {} ---\x1b[0m\n",
            self.baud_rate
        )));
        self.scroll = 0;
    }
}

// ---------------------------------------------------------------------------
//  TUI 事件循环
// ---------------------------------------------------------------------------

fn run_tui(app: &mut App, terminal: &mut DefaultTerminal) -> io::Result<()> {
    let mut last_reconnect = std::time::Instant::now();
    let reconnect_interval = Duration::from_secs(2);
    let mut need_render = true;

    while app.running {
        if app.connected {
            if app.read_serial() {
                need_render = true;
            }
        } else if !app.manual_disconnect && last_reconnect.elapsed() >= reconnect_interval {
            app.try_reconnect();
            last_reconnect = std::time::Instant::now();
            need_render = true;
        }

        while event::poll(Duration::from_millis(1))? {
            match event::read()? {
                Event::Key(key) if key.kind == KeyEventKind::Press => {
                    handle_key(app, key);
                    need_render = true;
                }
                Event::Mouse(mouse) => {
                    use crossterm::event::MouseEventKind;
                    match mouse.kind {
                        MouseEventKind::ScrollUp => {
                            let max_scroll = app.output.len().saturating_sub(1);
                            if app.scroll < max_scroll {
                                app.scroll += 1;
                            }
                            need_render = true;
                        }
                        MouseEventKind::ScrollDown => {
                            app.scroll = app.scroll.saturating_sub(1);
                            need_render = true;
                        }
                        _ => {}
                    }
                }
                _ => {}
            }
        }

        if need_render {
            terminal.draw(|f| app.render(f))?;
            need_render = false;
        }
    }
    Ok(())
}

fn handle_key(app: &mut App, key: KeyEvent) {
    if key.modifiers == KeyModifiers::CONTROL {
        match key.code {
            KeyCode::Char('c') => {
                app.running = false;
                return;
            }
            KeyCode::Char('d') => {
                app.toggle_connection();
                return;
            }
            KeyCode::Char('b') => {
                app.cycle_baud_rate();
                return;
            }
            KeyCode::Char('h') => {
                app.hex_mode = !app.hex_mode;
                // 切换显示模式 = 重置显示流：清 pending、HEX 地址偏移归零、
                // RX 计数归零，避免切换回文本时输出混乱
                app.reset_rx();
                let mode = if app.hex_mode { "HEX" } else { "文本" };
                app.output.push_back(parse_ansi(&format!(
                    "\n\x1b[1;33m--- 切换到 {} 显示模式 ---\x1b[0m\n",
                    mode
                )));
                app.scroll = 0;
                return;
            }
            KeyCode::Char('s') => {
                app.hex_send = !app.hex_send;
                let mode = if app.hex_send { "HEX" } else { "文本" };
                app.output.push_back(parse_ansi(&format!(
                    "\n\x1b[1;33m--- 切换到 {} 发送模式 ---\x1b[0m\n",
                    mode
                )));
                app.scroll = 0;
                return;
            }
            KeyCode::Char('e') => {
                app.enter_sends = !app.enter_sends;
                let mode = if app.enter_sends {
                    "回车发送"
                } else {
                    "回车换行"
                };
                app.output.push_back(parse_ansi(&format!(
                    "\n\x1b[1;33m--- 切换到 {} 模式 ---\x1b[0m\n",
                    mode
                )));
                app.scroll = 0;
                return;
            }
            KeyCode::Char('l') => {
                app.append_crlf = !app.append_crlf;
                let mode = if app.append_crlf {
                    "追加 \\r\\n"
                } else {
                    "不追加"
                };
                app.output.push_back(parse_ansi(&format!(
                    "\n\x1b[1;33m--- 发送 {} ---\x1b[0m\n",
                    mode
                )));
                app.scroll = 0;
                return;
            }
            KeyCode::Enter => {
                // Ctrl+Enter 强制发送
                app.send_line();
                return;
            }
            _ => {}
        }
    }
    match key.code {
        KeyCode::Char(c) => {
            if app.input.len() < MAX_INPUT_LEN {
                app.input.push(c);
            }
        }
        KeyCode::Enter => {
            if app.enter_sends {
                app.send_line();
            } else {
                app.input.push('\n');
            }
        }
        KeyCode::Backspace => {
            app.input.pop();
        }
        KeyCode::Up => {
            let max_scroll = app.output.len().saturating_sub(1);
            if app.scroll < max_scroll {
                app.scroll += 1;
            }
        }
        KeyCode::Down => {
            app.scroll = app.scroll.saturating_sub(1);
        }
        KeyCode::PageUp => {
            app.scroll = (app.scroll + 20).min(app.output.len().saturating_sub(1));
        }
        KeyCode::PageDown => {
            app.scroll = app.scroll.saturating_sub(20);
        }
        KeyCode::Esc => {
            app.running = false;
        }
        KeyCode::Tab => {
            app.input.push('\t');
        }
        _ => {}
    }
}

// ---------------------------------------------------------------------------
//  TUI 渲染
// ---------------------------------------------------------------------------

impl App {
    fn render(&mut self, frame: &mut ratatui::Frame) {
        let area = frame.area();
        let [output_area, input_area, status_area] = Layout::vertical([
            ratatui::layout::Constraint::Fill(1),
            ratatui::layout::Constraint::Length(3),
            ratatui::layout::Constraint::Length(1),
        ])
        .areas(area);

        self.render_output(frame, output_area);
        self.render_input(frame, input_area);
        self.render_status(frame, status_area);
    }

    fn render_output(&mut self, frame: &mut ratatui::Frame, area: ratatui::prelude::Rect) {
        let border_color = if self.connected {
            Color::Green
        } else {
            Color::Red
        };
        let suffix = if !self.append_crlf { " RAW" } else { "" };
        let title = match (self.hex_mode, self.hex_send) {
            (true, true) => format!(
                " {} @ {}bps [RX:HEX TX:HEX{}] ",
                self.port_name, self.baud_rate, suffix
            ),
            (true, false) => format!(
                " {} @ {}bps [RX:HEX{}] ",
                self.port_name, self.baud_rate, suffix
            ),
            (false, true) => format!(
                " {} @ {}bps [TX:HEX{}] ",
                self.port_name, self.baud_rate, suffix
            ),
            (false, false) => format!(" {} @ {}bps{} ", self.port_name, self.baud_rate, suffix),
        };
        let block = Block::default()
            .title(title)
            .borders(Borders::ALL)
            .border_style(Style::default().fg(border_color));

        let inner = block.inner(area);
        let visible_lines = inner.height.max(1) as usize;

        // 最后一个还没等到换行的 pending 也作为一行显示，
        // 这样设备只发 `>` 之类的无换行数据时不用等 flush 就能看到
        let pending_segments = if !self.hex_mode && !self.rx.pending.is_empty() {
            Some(parse_ansi(&self.rx.pending))
        } else {
            None
        };
        let total = self.output.len() + usize::from(pending_segments.is_some());

        // 钳位 scroll，确保可视区域始终填满，不会因 scroll 过大导致窗口收缩
        let max_scroll = total.saturating_sub(visible_lines);
        if self.scroll > max_scroll {
            self.scroll = max_scroll;
        }

        let end = total.saturating_sub(self.scroll);
        let start = end.saturating_sub(visible_lines);

        // 每一行（含待 flush 的 pending 行）单独转换成一条 Line
        let mut lines: Vec<Line> = Vec::with_capacity(end - start);
        for index in start..end {
            let segments = match self.output.get(index) {
                Some(segments) => segments,
                // 索引越过 output 尾部时只剩 pending 这一行
                None => match pending_segments.as_ref() {
                    Some(segments) => segments,
                    None => continue,
                },
            };
            let spans: Vec<Span> = segments
                .iter()
                .map(|(text, style)| Span::styled(text.as_str(), *style))
                .collect();
            lines.push(Line::from(spans));
        }

        let text = Text::from(lines);
        let paragraph = Paragraph::new(text).block(block).wrap(Wrap { trim: false });
        frame.render_widget(paragraph, area);
    }

    fn render_input(&self, frame: &mut ratatui::Frame, area: ratatui::prelude::Rect) {
        let input_title = match (self.hex_send, self.enter_sends) {
            (true, _) => "发送 [HEX]",
            (false, true) => "发送 [Enter]",
            (false, false) => "发送 [Ctrl+Enter]",
        };
        let block = Block::default().title(input_title).borders(Borders::ALL);
        let input = Paragraph::new(self.input.as_str()).block(block);
        frame.render_widget(input, area);

        let x = area.x + 1 + self.input.len() as u16;
        let y = area.y + 1;
        if x < area.x + area.width.saturating_sub(1) {
            frame.set_cursor_position((x, y));
        }
    }

    fn render_status(&self, frame: &mut ratatui::Frame, area: ratatui::prelude::Rect) {
        let color = if self.connected {
            Color::Green
        } else {
            Color::Red
        };
        let help = " ↑↓滚动  Ctrl+D开/关 Ctrl+B波特率 Ctrl+H显示 Ctrl+S发送 Ctrl+E回车 Ctrl+L换行 Esc退出 ";
        let rx = format!("RX {} B", self.rx_bytes);
        let line = Line::from(vec![
            Span::styled(&self.status, Style::default().fg(color)),
            Span::raw("  "),
            Span::styled(rx, Style::default().fg(Color::Cyan)),
            Span::raw("  "),
            Span::styled(help, Style::default().fg(Color::DarkGray)),
        ]);
        let status = Paragraph::new(line);
        frame.render_widget(status, area);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ---------------- 增量 UTF-8 解码 ----------------

    /// 逐字节喂入多字节字符，拼起来必须与原文完全一致且不出现替换符
    #[test]
    fn decoder_splits_multibyte_across_chunks() {
        let text = "温度=25℃";
        let bytes = text.as_bytes();
        let mut decoder = Utf8Decoder::new();
        let mut out = String::new();
        for byte in bytes {
            out.push_str(&decoder.push(std::slice::from_ref(byte)));
        }
        assert_eq!(out, text);
        assert!(!out.contains('\u{FFFD}'));
        assert_eq!(decoder.tail_len(), 0);
    }

    /// 块尾的多字节前缀要保留到下一块，而不是当非法字节替换掉
    #[test]
    fn decoder_keeps_incomplete_tail() {
        let mut decoder = Utf8Decoder::new();
        assert_eq!(decoder.push(&[0xE4]), "");
        assert_eq!(decoder.tail_len(), 1);
        assert_eq!(decoder.push(&[0xB8, 0xAD]), "中");
        assert_eq!(decoder.tail_len(), 0);
    }

    /// 真正的非法字节替换为 U+FFFD 后继续解码，不丢后续内容
    #[test]
    fn decoder_replaces_invalid_byte_and_continues() {
        let mut decoder = Utf8Decoder::new();
        assert_eq!(decoder.push(b"a\xFFb"), "a\u{FFFD}b");
        assert_eq!(decoder.tail_len(), 0);
    }

    /// 残尾 + 后续非法字节：残尾不能吞掉后面的数据
    #[test]
    fn decoder_tail_then_invalid() {
        let mut decoder = Utf8Decoder::new();
        assert_eq!(decoder.push(&[0xE4]), "");
        let out = decoder.push(&[0xFF, b'x']);
        assert!(out.contains('\u{FFFD}'), "非法字节应替换为 U+FFFD: {:?}", out);
        assert!(out.ends_with('x'), "非法字节之后的字符不能丢: {:?}", out);
        assert_eq!(decoder.tail_len(), 0);
    }

    /// reset 之后残尾清零
    #[test]
    fn decoder_reset_drops_tail() {
        let mut decoder = Utf8Decoder::new();
        decoder.push(&[0xE4]);
        assert_eq!(decoder.tail_len(), 1);
        decoder.reset();
        assert_eq!(decoder.tail_len(), 0);
    }

    // ---------------- 控制字符过滤 ----------------

    #[test]
    fn clean_text_drops_cr_and_control_keeps_ansi() {
        let input = "a\rb\x01c\x1b[31md\n\te";
        assert_eq!(clean_text(input), "abc\x1b[31md\n\te");
    }

    // ---------------- HEX 行拆分与地址偏移 ----------------

    fn hex_payload(start: u8, len: usize) -> Vec<u8> {
        (0..len).map(|i| start.wrapping_add(i as u8)).collect()
    }

    /// 40 字节 = 3 行（16 + 16 + 8），首列偏移 0000 / 0010 / 0020
    #[test]
    fn hex_lines_40_bytes_three_rows() {
        let data = hex_payload(0x00, 40);
        let lines = format_as_hex_lines(&data, 0);
        assert_eq!(lines.len(), 3);
        assert!(lines[0].starts_with("0x0000  "), "{}", lines[0]);
        assert!(lines[1].starts_with("0x0010  "), "{}", lines[1]);
        assert!(lines[2].starts_with("0x0020  "), "{}", lines[2]);
        // 完整行 16 字节 = 16*2 + 15 个空格
        assert_eq!(lines[0].len(), "0x0000  ".len() + 16 * 2 + 15);
        // 最后一行只有 8 字节
        assert_eq!(lines[2].len(), "0x0020  ".len() + 8 * 2 + 7);
        assert!(lines[0].ends_with("0F"), "{}", lines[0]);
        assert!(lines[1].starts_with("0x0010  10 11"), "{}", lines[1]);
    }

    /// 空 / 恰好一行 / 多一行
    #[test]
    fn hex_lines_row_counts() {
        assert!(format_as_hex_lines(&[], 0).is_empty());
        assert_eq!(format_as_hex_lines(&hex_payload(0, 16), 0).len(), 1);
        assert_eq!(format_as_hex_lines(&hex_payload(0, 17), 0).len(), 2);
    }

    /// 跨块累计偏移：20 + 4 字节接续成 0x0000 / 0x0010 / 0x0014
    #[test]
    fn hex_offset_accumulates_across_chunks() {
        let mut rx = RxBuffer::new();
        let first = rx.push(&hex_payload(0, 20), true, false);
        assert_eq!(first.len(), 2);
        assert!(first[0].starts_with("0x0000  "));
        assert!(first[1].starts_with("0x0010  "));

        let second = rx.push(&hex_payload(0x20, 4), true, false);
        assert_eq!(second.len(), 1);
        assert!(second[0].starts_with("0x0014  "), "{}", second[0]);

        rx.reset();
        let after = rx.push(&hex_payload(0, 1), true, false);
        assert!(after[0].starts_with("0x0000  "), "重置后偏移应归零: {}", after[0]);
    }

    // ---------------- 文本 pending / flush / 上限 ----------------

    /// 没有换行时不输出，来一个 flush 点整块吐出
    #[test]
    fn text_pending_flushes_at_timeout() {
        let mut rx = RxBuffer::new();
        assert!(rx.push(b">", false, false).is_empty());
        assert_eq!(rx.pending, ">");
        let lines = rx.push(&[], false, true);
        assert_eq!(lines, vec![">".to_string()]);
        assert!(rx.pending.is_empty());
    }

    /// 完整行按 \n 拆开，最后不完整的一段留在 pending
    #[test]
    fn text_splits_complete_lines() {
        let mut rx = RxBuffer::new();
        let lines = rx.push(b"a\nb\nc", false, false);
        assert_eq!(lines, vec!["a\n".to_string(), "b\n".to_string()]);
        assert_eq!(rx.pending, "c");
    }

    /// 超过 8KB 的无换行数据必须强制断行，不能无限增长
    #[test]
    fn text_pending_forced_break_over_limit() {
        let mut rx = RxBuffer::new();
        let data = vec![b'a'; MAX_PENDING_BYTES + 100];
        let lines = rx.push(&data, false, false);
        assert_eq!(lines.len(), 1);
        assert_eq!(lines[0].len(), MAX_PENDING_BYTES);
        assert_eq!(rx.pending.len(), 100);
    }

    /// 强制断行落在多字节字符中间时要回退到字符边界
    #[test]
    fn text_forced_break_respects_char_boundary() {
        let mut rx = RxBuffer::new();
        let mut data = vec![b'a'; MAX_PENDING_BYTES - 1];
        data.extend_from_slice("中".as_bytes());
        assert_eq!(data.len(), MAX_PENDING_BYTES + 2);
        let lines = rx.push(&data, false, false);
        assert_eq!(lines.len(), 1);
        assert_eq!(lines[0].len(), MAX_PENDING_BYTES - 1);
        assert_eq!(rx.pending, "中");
    }

    /// 1KB 分块喂入超长中文多行输出：无替换符、拼回原文
    #[test]
    fn text_long_cjk_stream_round_trip() {
        let text = "温度=25℃\n".repeat(300);
        let bytes = text.as_bytes();
        let mut rx = RxBuffer::new();
        let mut out = String::new();
        for chunk in bytes.chunks(1024) {
            out.push_str(&rx.push(chunk, false, false).concat());
        }
        out.push_str(&rx.push(&[], false, true).concat());
        assert!(!out.contains('\u{FFFD}'), "跨块解码不应产生替换符");
        assert_eq!(out, text);
    }
}

