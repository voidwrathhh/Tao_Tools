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

const APP_NAME: &str = "sertool";
const VERSION: &str = env!("CARGO_PKG_VERSION");
const MAX_LINES: usize = 2000;
const MAX_INPUT_LEN: usize = 4096;
const COMMON_BAUD_RATES: &[u32] = &[9600, 19200, 38400, 57600, 115200, 230400, 460800, 921600];

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

/// 过滤控制字符但保留 ANSI 转义码
fn clean_text(data: &[u8]) -> String {
    let s = String::from_utf8_lossy(data);
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
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
        "connect" => {
            if args.len() < 3 {
                eprintln!("错误：缺少端口名");
                eprintln!("用法：{} connect <端口> [波特率]", APP_NAME);
                std::process::exit(1);
            }
            let baud: u32 = args.get(3).and_then(|s| s.parse().ok()).unwrap_or(115200);
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
        "  {} connect <端口> [波特率]     连接串口（默认 115200）",
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

/// 将字节数据格式化为十六进制字符串（每行 16 字节）
fn format_as_hex(data: &[u8]) -> String {
    let mut s = String::new();
    for (i, byte) in data.iter().enumerate() {
        if i > 0 && i % 16 == 0 {
            s.push('\n');
        } else if i > 0 {
            s.push(' ');
        }
        s.push_str(&format!("{:02X}", byte));
    }
    s
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
//  TUI 连接
// ---------------------------------------------------------------------------

fn connect(port_name: &str, baud_rate: u32) {
    let _ = crossterm::execute!(std::io::stdout(), crossterm::event::EnableMouseCapture);
    let mut terminal = ratatui::init();
    let mut app = App::new(port_name.to_string(), baud_rate);
    let res = run_tui(&mut app, &mut terminal);
    ratatui::restore();
    let _ = crossterm::execute!(std::io::stdout(), crossterm::event::DisableMouseCapture);

    if let Err(e) = res {
        eprintln!("TUI 错误：{}", e);
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
    pending: String,
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
    fn new(port_name: String, baud_rate: u32) -> Self {
        let info = Self::open_port(&port_name, baud_rate);
        App {
            port_name,
            baud_rate,
            port: info.0,
            connected: info.1,
            output: VecDeque::with_capacity(MAX_LINES),
            pending: String::new(),
            input: String::new(),
            scroll: 0,
            status: info.2,
            running: true,
            hex_mode: false,
            hex_send: false,
            manual_disconnect: false,
            enter_sends: true,
            append_crlf: true,
        }
    }

    fn open_port(name: &str, baud: u32) -> (Option<Box<dyn serialport::SerialPort>>, bool, String) {
        let mut port = match serialport::new(name, baud)
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

    /// 读取串口数据，解析 ANSI 颜色或 HEX 显示，返回 true 表示有新数据
    fn read_serial(&mut self) -> bool {
        let port = match self.port.as_mut() {
            Some(p) => p,
            None => return false,
        };
        let mut buf = [0u8; 1024];
        match port.read(&mut buf) {
            Ok(0) => false,
            Ok(n) => {
                if self.hex_mode {
                    // HEX 模式：直接格式化为十六进制，不经过 ANSI 解析
                    let hex = format_as_hex(&buf[..n]);
                    self.output.push_back(vec![(hex, Style::default())]);
                    while self.output.len() > MAX_LINES {
                        self.output.pop_front();
                    }
                } else {
                    let text = clean_text(&buf[..n]);
                    self.pending.push_str(&text);

                    if let Some(pos) = self.pending.rfind('\n') {
                        let complete = self.pending[..=pos].to_string();
                        for chunk in complete.split_inclusive('\n') {
                            self.output.push_back(parse_ansi(chunk));
                        }
                        while self.output.len() > MAX_LINES {
                            self.output.pop_front();
                        }
                        self.pending = self.pending[pos + 1..].to_string();
                    }
                }
                true
            }
            Err(ref e) if e.kind() == io::ErrorKind::TimedOut => false,
            Err(e) => {
                self.connected = false;
                self.port = None;
                self.status = format!("✗ {} 已断开 ({})", self.port_name, e);
                false
            }
        }
    }

    fn try_reconnect(&mut self) {
        let (port, ok, msg) = Self::open_port(&self.port_name, self.baud_rate);
        self.port = port;
        self.connected = ok;
        self.status = msg;
        if ok {
            self.output
                .push_back(parse_ansi("\n\x1b[1;33m--- 已重新连接 ---\x1b[0m\n"));
            self.scroll = 0;
        }
    }

    fn send_line(&mut self) {
        if self.input.is_empty() || !self.connected {
            return;
        }
        let port = match self.port.as_mut() {
            Some(p) => p,
            None => return,
        };

        if self.hex_send {
            // HEX 发送模式：解析输入为十六进制字节
            match parse_hex_input(&self.input) {
                Ok(bytes) => {
                    if port.write_all(&bytes).is_ok() {
                        let _ = port.flush();
                        let hex = format_as_hex(&bytes);
                        self.output
                            .push_back(parse_ansi(&format!("\n<<< {}\n", hex)));
                    }
                }
                Err(e) => {
                    self.output
                        .push_back(parse_ansi(&format!("\n<<< HEX 解析错误: {} >>>\n", e)));
                }
            }
        } else {
            let suffix = if self.append_crlf { "\r\n" } else { "" };
            let data = format!("{}{}", self.input, suffix);
            if port.write_all(data.as_bytes()).is_ok() {
                let _ = port.flush();
                self.output
                    .push_back(parse_ansi(&format!("\n<<< {}\n", self.input)));
            }
        }
        self.input.clear();
    }

    fn toggle_connection(&mut self) {
        if self.connected {
            // 用户主动断开
            self.port = None;
            self.connected = false;
            self.manual_disconnect = true;
            self.status = format!("✗ {} 已断开 (用户)", self.port_name);
            self.output
                .push_back(parse_ansi("\n\x1b[1;33m--- 已断开连接 ---\x1b[0m\n"));
        } else {
            // 重新打开
            self.manual_disconnect = false;
            let (port, ok, msg) = Self::open_port(&self.port_name, self.baud_rate);
            self.port = port;
            self.connected = ok;
            self.status = msg;
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
                // 切换模式后清空 pending，避免切换回文本时输出混乱
                app.pending.clear();
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

        let total = self.output.len();

        // 钳位 scroll，确保可视区域始终填满，不会因 scroll 过大导致窗口收缩
        let max_scroll = total.saturating_sub(visible_lines);
        if self.scroll > max_scroll {
            self.scroll = max_scroll;
        }

        let end = total.saturating_sub(self.scroll);
        let start = end.saturating_sub(visible_lines);

        // 将每一行的 StyledSegments 转换为 Line
        let lines: Vec<Line> = self
            .output
            .range(start..end)
            .map(|segments| {
                let spans: Vec<Span> = segments
                    .iter()
                    .map(|(text, style)| Span::styled(text.as_str(), *style))
                    .collect();
                Line::from(spans)
            })
            .collect();

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
        let line = Line::from(vec![
            Span::styled(&self.status, Style::default().fg(color)),
            Span::raw("  "),
            Span::styled(help, Style::default().fg(Color::DarkGray)),
        ]);
        let status = Paragraph::new(line);
        frame.render_widget(status, area);
    }
}
