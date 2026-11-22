// ======================== egui 图形界面 ========================
//
// 布局：左侧功能面板（连接 + 参数 + 指令 + 状态）| 右侧绘图区
// 底部：状态栏
//
// 线程模型：UI 主线程仅负责渲染和交互，
// 所有串口读写操作通过 CommState 后台线程进行。

use crate::comm::CommState;
use crate::config::AppConfig;
use crate::data_struct::{
    default_params, is_echo_prefix, DeviceParam, LogKind, LogLine, ParamValue, PlotChannel,
    CHART_COLORS, MAX_LOG_LINES, MAX_SAMPLES_PER_CHANNEL,
};
use crate::protocol::FrameStats;
use eframe::egui::{self, Color32, RichText, ScrollArea, Vec2};
use egui_plot::{Legend, Line, Plot, PlotPoints};
use std::time::{Duration, Instant};

/// 断线重连周期（验收要求约 3 秒一次）
const RECONNECT_INTERVAL: Duration = Duration::from_secs(3);

// ─── 主应用状态 ────────────────────────────────────────────

pub struct BlueInfoApp {
    comm: Option<CommState>,
    config: AppConfig,

    available_ports: Vec<(String, String)>,
    selected_port: String,
    selected_baud: u32,

    // ── 断线重连状态机（与 comm 的生命周期解耦）──
    /// 自本次掉线以来已尝试的重连次数
    reconnect_attempts: u32,
    /// 下一次允许重试的时间点；None 表示尚未安排
    next_retry_at: Option<Instant>,

    params: Vec<ParamValue>,

    text_input: String,
    log_lines: Vec<LogLine>,
    log_auto_scroll: bool,

    plot_channels: Vec<PlotChannel>,
    axis_auto_scale: bool,
    /// 本次连接的时间原点：连接成功时统一重置，文本行与二进制帧共用
    plot_time_start: Option<Instant>,
    /// 已推入曲线的最大相对时间；重连时据此把原点前移，保证时间轴连续
    plot_last_time: f64,
    /// 被用户隐藏的通道原始名（与 config.json 同步，重启后保持）
    hidden_channels: Vec<String>,
    /// 正在重命名的通道：(通道索引, 编辑中的文本)；仅前端显示，不改协议键
    channel_name_edit: Option<(usize, String)>,

    latest_raw: Vec<(String, f64)>,
    /// 协议坏帧统计（来自串口线程，状态栏展示）
    frame_stats: FrameStats,

    frame_times: std::collections::VecDeque<f64>,
    fps: f64,
    packet_count: usize,
    last_stats_update: Instant,

    auto_reconnect: bool,
    user_disconnected: bool,
    text_parse_enabled: bool,
    style_initialized: bool,
    custom_commands: Vec<String>,
    new_command_input: String,
    selected_tab: String,
    /// true：只渲染设置界面（`blueinfo settings` 启动）
    settings_only: bool,
    /// 设置界面里快捷指令的多行文本缓存
    settings_commands_text: String,
}

impl BlueInfoApp {
    pub fn new(settings_only: bool) -> Self {
        let config = AppConfig::load();
        let ports: Vec<(String, String)> = CommState::available_ports();

        let params: Vec<ParamValue> = config.params.iter().map(ParamValue::from).collect();

        let selected_port =
            if !config.last_port.is_empty() && ports.iter().any(|(n, _)| n == &config.last_port) {
                config.last_port.clone()
            } else {
                ports.first().map(|(n, _)| n.clone()).unwrap_or_default()
            };

        let selected_baud = if config.last_baud == 0 {
            115200
        } else {
            config.last_baud
        };
        let quick_commands = config.quick_commands.clone();
        let hidden_channels = config.hidden_channels.clone();

        Self {
            comm: None,
            config,
            available_ports: ports,
            selected_port,
            selected_baud,
            reconnect_attempts: 0,
            next_retry_at: None,
            params,
            text_input: String::new(),
            log_lines: Vec::with_capacity(MAX_LOG_LINES),
            log_auto_scroll: true,
            plot_channels: Vec::new(),
            axis_auto_scale: true,
            plot_time_start: None,
            plot_last_time: 0.0,
            hidden_channels,
            channel_name_edit: None,
            latest_raw: Vec::new(),
            frame_stats: FrameStats::default(),
            frame_times: std::collections::VecDeque::with_capacity(120),
            fps: 60.0,
            packet_count: 0,
            last_stats_update: Instant::now(),
            auto_reconnect: true,
            user_disconnected: false,
            text_parse_enabled: true,
            style_initialized: false,
            custom_commands: quick_commands.clone(),
            new_command_input: String::new(),
            selected_tab: "左电机".into(),
            settings_only,
            settings_commands_text: quick_commands.join("\n"),
        }
    }

    fn refresh_ports(&mut self) {
        self.available_ports = CommState::available_ports();
        if !self
            .available_ports
            .iter()
            .any(|(n, _)| n == &self.selected_port)
        {
            self.selected_port = self
                .available_ports
                .first()
                .map(|(n, _)| n.clone())
                .unwrap_or_default();
        }
    }

    fn try_connect(&mut self) {
        if self.selected_port.is_empty() {
            return;
        }
        let mut comm = CommState::new(self.selected_port.clone(), self.selected_baud);
        match comm.connect() {
            Ok(()) => {
                self.comm = Some(comm);
                self.on_connected(false);
                self.log_info(format!(
                    "[连接] 已连接 {} @ {}",
                    self.selected_port, self.selected_baud
                ));
            }
            Err(e) => {
                self.log_err(format!("[错误] {}", e));
            }
        }
    }

    /// 连接成功后的统一收尾：重置时间原点与重连状态
    ///
    /// `is_reconnect = true` 时保留已有曲线，把时间原点前移到「上一段最后
    /// 一个采样点」，使重连后的数据紧接上一段，时间轴保持连续。
    fn on_connected(&mut self, is_reconnect: bool) {
        let now = Instant::now();
        if is_reconnect {
            self.plot_time_start = Some(continuation_origin(now, self.plot_last_time));
        } else {
            self.plot_channels.clear();
            self.latest_raw.clear();
            self.plot_last_time = 0.0;
            self.plot_time_start = Some(now);
        }
        self.reconnect_attempts = 0;
        self.next_retry_at = None;
        self.user_disconnected = false;
    }

    fn disconnect(&mut self) {
        if let Some(mut comm) = self.comm.take() {
            let _ = comm.disconnect();
            // 断开瞬间后台可能刚写入残余数据：先取走再释放，避免丢日志
            self.comm = Some(comm);
            self.poll_serial();
            self.comm = None;
        }
        self.user_disconnected = true;
        self.reconnect_attempts = 0;
        self.next_retry_at = None;
        self.log_info("[连接] 已断开（不再自动重连）");
    }

    fn send_text(&mut self) {
        let text = self.text_input.trim().to_string();
        if text.is_empty() {
            return;
        }
        if let Some(ref comm) = self.comm {
            if let Err(e) = comm.send_text(&text) {
                self.log_err(format!("[发送失败] {}", e));
            }
        } else {
            self.log_err("[错误] 串口未连接");
        }
        self.text_input.clear();
    }

    /// 快捷指令条：点击按钮立即发送预设文本
    fn send_command(&mut self, cmd: &str) {
        if let Some(ref comm) = self.comm {
            if let Err(e) = comm.send_text(cmd) {
                self.log_err(format!("[发送失败] {}", e));
            }
        } else {
            self.log_err("[错误] 串口未连接");
        }
    }

    fn send_params(&mut self) {
        let device_params: Vec<DeviceParam> = self
            .params
            .iter()
            .map(|p| DeviceParam {
                name: p.name.clone(),
                value: p.value,
                min: p.min,
                max: p.max,
                step: p.step,
            })
            .collect();
        if let Some(ref comm) = self.comm {
            // 回显由 comm.send_params 记录为 Tx 行，不参与文本→曲线解析
            if let Err(e) = comm.send_params(&device_params) {
                self.log_err(format!("[参数下发失败] {}", e));
            }
        } else {
            self.log_err("[错误] 串口未连接，无法下发参数");
        }
    }

    fn reset_params(&mut self) {
        self.params = default_params().into_iter().map(ParamValue::from).collect();
        self.log_info("[参数] 已恢复默认值");
    }

    /// 由当前界面状态构造待落盘配置（保存与通道显隐共用，保证同源）
    fn build_config(&self) -> AppConfig {
        let params: Vec<DeviceParam> = self
            .params
            .iter()
            .map(|p| DeviceParam {
                name: p.name.clone(),
                value: p.value,
                min: p.min,
                max: p.max,
                step: p.step,
            })
            .collect();
        let mut cfg = AppConfig {
            last_port: self.selected_port.clone(),
            last_baud: self.selected_baud,
            params,
            max_samples: MAX_SAMPLES_PER_CHANNEL,
            auto_reconnect: self.auto_reconnect,
            log_auto_scroll: self.log_auto_scroll,
            axis_auto_scale: self.axis_auto_scale,
            quick_commands: self.custom_commands.clone(),
            hidden_channels: self.hidden_channels.clone(),
        };
        if let Some(ref comm) = self.comm {
            cfg.last_port = comm.port_name.clone();
            cfg.last_baud = comm.baud_rate;
        }
        cfg
    }

    fn save_config(&mut self) {
        let cfg = self.build_config();
        cfg.save();
        self.config = cfg;
        // 设置窗口里刚改的快捷指令立即生效，不必重启主界面
        self.custom_commands = self.config.quick_commands.clone();
        self.log_info("[配置] 已保存到 ~/.tao_tools/blueinfo/config.json");
    }

    /// 通道勾选变化：更新可见性、同步隐藏名单并立即落盘（重启后保持）
    fn set_channel_visible(&mut self, idx: usize, visible: bool) {
        if let Some(ch) = self.plot_channels.get_mut(idx) {
            ch.visible = visible;
            let name = ch.name.clone();
            update_hidden(&mut self.hidden_channels, &name, visible);
        }
        let cfg = self.build_config();
        cfg.save();
        self.config = cfg;
    }

    fn export_log(&mut self) {
        let path = format!("blueinfo_log_{}.txt", chrono_now());
        let body = self
            .log_lines
            .iter()
            .map(|l| l.text.as_str())
            .collect::<Vec<_>>()
            .join("\n");
        if let Err(e) = std::fs::write(&path, body) {
            self.log_err(format!("[导出失败] {}: {}", path, e));
        } else {
            self.log_info(format!("[导出] 日志已保存到 {}", path));
        }
    }

    fn clear_log(&mut self) {
        self.log_lines.clear();
    }

    // ── 日志与时间轴辅助 ──────────────────────────────────

    fn push_log(&mut self, line: LogLine) {
        if self.log_lines.len() >= MAX_LOG_LINES {
            self.log_lines.drain(0..MAX_LOG_LINES / 2);
        }
        self.log_lines.push(line);
    }

    fn log_info(&mut self, text: impl Into<String>) {
        self.push_log(LogLine::info(text));
    }

    fn log_err(&mut self, text: impl Into<String>) {
        self.push_log(LogLine::err(text));
    }

    /// 计算相对时间（秒）。文本行与二进制帧共用同一 Instant 原点。
    fn rel_time(&mut self, at: Instant) -> f64 {
        let origin = match self.plot_time_start {
            Some(o) => o,
            None => {
                self.plot_time_start = Some(at);
                at
            }
        };
        rel_secs(origin, at)
    }

    fn note_sample_time(&mut self, t: f64) {
        if t > self.plot_last_time {
            self.plot_last_time = t;
        }
        self.latest_raw = self
            .plot_channels
            .iter()
            .map(|c| (c.label().to_string(), c.last_value().unwrap_or(0.0)))
            .collect();
    }

    fn push_text_sample(&mut self, key: &str, val: f64, t: f64) {
        let idx = match self.plot_channels.iter().position(|c| c.name == key) {
            Some(i) => i,
            None => {
                let idx = self.plot_channels.len();
                let color = CHART_COLORS[idx % CHART_COLORS.len()];
                let mut channel = PlotChannel::new(key, color);
                channel.visible = !self.hidden_channels.iter().any(|n| n == key);
                self.plot_channels.push(channel);
                idx
            }
        };
        self.plot_channels[idx].push(t, val);
        self.packet_count += 1;
        self.note_sample_time(t);
    }

    /// 拉取串口缓冲：日志分类入列；只有 Rx 且非回显前缀的文本行参与文本→曲线
    fn poll_serial(&mut self) {
        let Some(comm) = self.comm.take() else {
            return;
        };
        let lines = comm.drain_lines();
        let packets = comm.drain_packets();
        self.frame_stats = comm.frame_stats();
        self.comm = Some(comm);

        for line in lines {
            if self.text_parse_enabled && line.is_plottable() && !is_echo_prefix(&line.text) {
                if let Some((key, val)) = parse_text_value(&line.text) {
                    let t = self.rel_time(line.at);
                    self.push_text_sample(key, val, t);
                }
            }
            self.push_log(line);
        }

        for tp in packets {
            self.packet_count += 1;
            let t = self.rel_time(tp.at);
            while self.plot_channels.len() < tp.packet.channel_count as usize {
                let idx = self.plot_channels.len();
                let color = CHART_COLORS[idx % CHART_COLORS.len()];
                let name = format!("CH{}", idx + 1);
                let mut channel = PlotChannel::new(&name, color);
                channel.visible = !self.hidden_channels.iter().any(|n| n == &name);
                self.plot_channels.push(channel);
            }
            for (i, val) in tp.packet.channel_data.iter().enumerate() {
                if i < self.plot_channels.len() {
                    self.plot_channels[i].push(t, *val);
                }
            }
            self.note_sample_time(t);
        }
    }

    /// 掉线检测 + 固定周期重试的状态机
    ///
    /// 重连复用同一个 `CommState`（缓冲与端口信息都保留），失败只记日志，
    /// 不把 `self.comm` 置空；用户点「断开」后 `user_disconnected` 阻止重试。
    fn maybe_reconnect(&mut self) {
        if !self.auto_reconnect || self.user_disconnected {
            return;
        }
        if self.comm.as_ref().map(|c| c.is_connected()).unwrap_or(true) {
            return;
        }

        let now = Instant::now();
        match self.next_retry_at {
            None => {
                self.log_err("[连接] 串口已断开，进入自动重连");
                self.reconnect_attempts = 0;
                self.next_retry_at = Some(now);
            }
            Some(due) if now >= due => {}
            Some(_) => return,
        }

        // 到达重试时间点，执行一次连接尝试
        self.reconnect_attempts += 1;
        self.next_retry_at = Some(now + RECONNECT_INTERVAL);
        let Some(mut comm) = self.comm.take() else {
            self.next_retry_at = None;
            return;
        };
        let attempt = self.reconnect_attempts;
        let port = comm.port_name.clone();
        let baud = comm.baud_rate;
        self.log_info(format!("[重连] 第 {} 次尝试：{} @ {} ..", attempt, port, baud));
        match comm.connect() {
            Ok(()) => {
                self.comm = Some(comm);
                self.on_connected(true);
                self.log_info("[重连] 成功，曲线继续追加");
            }
            Err(e) => {
                self.comm = Some(comm);
                self.log_err(format!("[重连] 第 {} 次失败：{}", attempt, e));
            }
        }
    }
}

fn chrono_now() -> String {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default();
    let secs = now.as_secs();
    let days = secs / 86400;
    let time = secs % 86400;
    let h = time / 3600;
    let m = (time % 3600) / 60;
    let s = time % 60;
    let y = 2020 + (days as f64 / 365.25) as u64;
    format!("{:04}{:02}{:02}_{:02}{:02}{:02}", y, 1, 1, h, m, s)
}

// ─── 串口显示标签 ──────────────────────────────────────────

fn current_port_label(app: &BlueInfoApp) -> String {
    app.available_ports
        .iter()
        .find(|(n, _)| n == &app.selected_port)
        .map(|(_, l)| l.clone())
        .unwrap_or_else(|| {
            if app.selected_port.is_empty() {
                "无蓝牙设备".to_string()
            } else {
                app.selected_port.clone()
            }
        })
}

// ─── 文本行 → 通道数值 ─────────────────────────────────────
//
// 只有 Rx 且非回显前缀（">>> " / "[..]"）的行才会进入本函数，见 poll_serial。
// 支持的格式："KEY: value" 与 "KEY = value"，键名不含空白、长度 < 20。

/// 维护「持久化的隐藏通道名单」：按通道原始名增删，重复操作幂等
fn update_hidden(hidden: &mut Vec<String>, name: &str, visible: bool) {
    let pos = hidden.iter().position(|n| n == name);
    match (visible, pos) {
        (true, Some(i)) => {
            hidden.remove(i);
        }
        (false, None) => hidden.push(name.to_string()),
        _ => {}
    }
}

fn parse_text_value(line: &str) -> Option<(&str, f64)> {
    let valid_key = |key: &str| !key.is_empty() && key.len() < 20 && !key.contains(char::is_whitespace);
    if let Some(idx) = line.find(':') {
        let key = line[..idx].trim();
        let val_str = line[idx + 1..].trim();
        let val_str = val_str.split_whitespace().next().unwrap_or(val_str);
        if let Ok(val) = val_str.parse::<f64>() {
            if valid_key(key) {
                return Some((key, val));
            }
        }
    }
    if let Some(idx) = line.find('=') {
        let key = line[..idx].trim();
        let val_str = line[idx + 1..].trim();
        let val_str = val_str.split_whitespace().next().unwrap_or(val_str);
        if let Ok(val) = val_str.parse::<f64>() {
            if valid_key(key) {
                return Some((key, val));
            }
        }
    }
    None
}

/// 相对时间：`at` 距 `origin` 的秒数（单调非负）
fn rel_secs(origin: Instant, at: Instant) -> f64 {
    at.saturating_duration_since(origin).as_secs_f64()
}

/// 重连后的新原点：让 `now` 时刻的采样恰好接在上一段最后采样 `last_secs` 之后，
/// 从而在不清空曲线的前提下保持时间轴连续。
fn continuation_origin(now: Instant, last_secs: f64) -> Instant {
    now - Duration::from_secs_f64(last_secs.max(0.0))
}

// ─── eframe App trait ──────────────────────────────────────

impl eframe::App for BlueInfoApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        if !self.style_initialized {
            setup_dark_theme(ctx);
            setup_chinese_fonts(ctx);
            self.style_initialized = true;
        }

        // 设置模式：不连接设备、不绘图，只渲染设置界面
        if self.settings_only {
            settings_panel(ctx, self);
            return;
        }

        // ── 断线重连状态机 + 拉取后台数据 ──
        self.maybe_reconnect();
        self.poll_serial();

        // ── FPS ──
        let now = Instant::now();
        let dt = now.duration_since(self.last_stats_update).as_secs_f64();
        if dt > 0.0 {
            self.frame_times.push_back(1.0 / dt);
            if self.frame_times.len() > 60 {
                self.frame_times.pop_front();
            }
            self.fps = self.frame_times.iter().sum::<f64>() / self.frame_times.len().max(1) as f64;
        }
        self.last_stats_update = now;

        // ── 布局 ──
        egui::TopBottomPanel::bottom("status_bar")
            .min_height(24.0)
            .show(ctx, |ui| status_bar(ui, self));

        egui::SidePanel::left("left_panel")
            .resizable(true)
            .default_width(380.0)
            .min_width(300.0)
            .show(ctx, |ui| left_panel(ui, self));

        egui::CentralPanel::default().show(ctx, |ui| plot_panel(ui, self));

        ctx.request_repaint();
    }
}

// ─── 深色工控主题 ──────────────────────────────────────────

fn setup_dark_theme(ctx: &egui::Context) {
    let mut style = (*ctx.style()).clone();
    style.visuals.dark_mode = true;
    style.visuals.window_fill = Color32::from_rgb(20, 22, 26);
    style.visuals.panel_fill = Color32::from_rgb(26, 28, 33);
    style.visuals.window_stroke = egui::Stroke::new(1.0, Color32::from_rgb(60, 70, 85));
    style.visuals.widgets.inactive.bg_fill = Color32::from_rgb(38, 42, 50);
    style.visuals.widgets.inactive.fg_stroke =
        egui::Stroke::new(1.0, Color32::from_rgb(190, 195, 200));
    style.visuals.widgets.hovered.bg_fill = Color32::from_rgb(50, 55, 65);
    style.visuals.widgets.active.bg_fill = Color32::from_rgb(55, 65, 80);
    style.visuals.selection.bg_fill = Color32::from_rgb(0, 110, 170);
    style.visuals.hyperlink_color = Color32::from_rgb(70, 170, 240);
    style.spacing.item_spacing = Vec2::new(8.0, 6.0);
    ctx.set_style(style);
}

// ─── 设置面板（blueinfo settings）──────────────────────────

fn settings_panel(ctx: &egui::Context, app: &mut BlueInfoApp) {
    egui::CentralPanel::default().show(ctx, |ui| {
        ui.add_space(6.0);
        ui.heading(RichText::new("BlueInfo 设置").size(20.0));
        ui.label(
            RichText::new("配置保存在 %USERPROFILE%\\.tao_tools\\blueinfo\\config.json")
                .small()
                .weak(),
        );
        ui.separator();

        egui::ScrollArea::vertical().show(ui, |ui| {
            ui.label(RichText::new("串口").strong());
            ui.horizontal(|ui| {
                ui.label("默认波特率:");
                egui::ComboBox::from_id_salt("settings_baud")
                    .selected_text(app.selected_baud.to_string())
                    .show_ui(ui, |ui| {
                        for baud in [
                            9600u32, 19200, 38400, 57600, 115200, 230400, 460800, 921600,
                        ] {
                            ui.selectable_value(
                                &mut app.selected_baud,
                                baud,
                                baud.to_string(),
                            );
                        }
                    });
            });
            ui.checkbox(&mut app.auto_reconnect, "断线后自动重连（每 3 秒重试一次）");

            ui.add_space(8.0);
            ui.label(RichText::new("显示").strong());
            ui.checkbox(&mut app.log_auto_scroll, "日志自动滚动到底部");
            ui.checkbox(&mut app.axis_auto_scale, "曲线纵轴自动缩放");

            ui.add_space(8.0);
            ui.label(RichText::new("快捷指令").strong());
            ui.label(RichText::new("每行一条，显示在左侧指令面板").small().weak());
            ui.add(
                egui::TextEdit::multiline(&mut app.settings_commands_text)
                    .desired_rows(10)
                    .desired_width(f32::INFINITY)
                    .hint_text("STATUS"),
            );
        });

        ui.separator();
        ui.horizontal(|ui| {
            if ui
                .button(RichText::new("保存设置").strong())
                .clicked()
            {
                app.custom_commands = app
                    .settings_commands_text
                    .lines()
                    .map(|line| line.trim().to_string())
                    .filter(|line| !line.is_empty())
                    .collect();
                app.save_config();
            }
            if ui.button("关闭").clicked() {
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            }
            ui.label(
                RichText::new("保存后重新打开主界面生效")
                    .small()
                    .weak(),
            );
        });
    });
}

// ─── 左侧面板 ──────────────────────────────────────────────

fn left_panel(ui: &mut egui::Ui, app: &mut BlueInfoApp) {
    // ── 1. 连接配置 ──
    egui::CollapsingHeader::new("蓝牙设备")
        .id_salt("device_conn")
        .default_open(true)
        .show(ui, |ui| {
            ui.add_space(4.0);

            ui.horizontal(|ui| {
                ui.label("蓝牙设备:");
                let width = ui.available_width() - 60.0;
                egui::ComboBox::from_id_salt("port_select")
                    .width(width.max(120.0))
                    .selected_text(current_port_label(&app))
                    .show_ui(ui, |ui| {
                        for (port_name, label) in &app.available_ports {
                            ui.selectable_value(&mut app.selected_port, port_name.clone(), label);
                        }
                    });
                if ui.button("刷新").on_hover_text("刷新设备列表").clicked() {
                    app.refresh_ports();
                }
            });

            ui.horizontal(|ui| {
                let is_connected = app.comm.as_ref().map(|c| c.is_connected()).unwrap_or(false);

                if is_connected {
                    if ui
                        .button(RichText::new("断开").color(Color32::from_rgb(255, 120, 100)))
                        .clicked()
                    {
                        app.disconnect();
                    }
                    ui.label(RichText::new("● 已连接").color(Color32::GREEN).size(14.0));
                } else {
                    if ui
                        .button(RichText::new("连接").color(Color32::from_rgb(100, 200, 100)))
                        .clicked()
                    {
                        app.try_connect();
                    }
                    ui.label(
                        RichText::new("● 未连接")
                            .color(Color32::from_rgb(160, 80, 80))
                            .size(14.0),
                    );
                }
                ui.checkbox(&mut app.auto_reconnect, "自动重连");
            });
            ui.add_space(4.0);
        });

    ui.separator();

    // ── 2. 参数调试 ──
    egui::CollapsingHeader::new("参数调试")
        .id_salt("param_debug")
        .default_open(true)
        .show(ui, |ui| {
            ui.add_space(4.0);
            let tabs = ["左电机", "右电机"];
            ui.horizontal(|ui| {
                for tab in &tabs {
                    let selected = app.selected_tab == *tab;
                    let btn = egui::Button::new(
                        RichText::new(*tab)
                            .color(if selected {
                                Color32::WHITE
                            } else {
                                Color32::from_rgb(150, 160, 170)
                            })
                            .size(14.0),
                    )
                    .fill(if selected {
                        Color32::from_rgb(40, 44, 52)
                    } else {
                        Color32::from_rgb(28, 31, 38)
                    })
                    .stroke(egui::Stroke::new(
                        1.0,
                        if selected {
                            Color32::from_rgb(70, 150, 220)
                        } else {
                            Color32::from_rgb(50, 55, 62)
                        },
                    ))
                    .min_size(Vec2::new(100.0, 32.0));
                    if ui.add(btn).clicked() {
                        app.selected_tab = tab.to_string();
                    }
                }
            });
            ui.add_space(6.0);
            let range = if app.selected_tab == "左电机" {
                0..5
            } else {
                5..10
            };
            for i in range {
                if i >= app.params.len() {
                    break;
                }
                let param = &mut app.params[i];
                ui.horizontal(|ui| {
                    ui.label(RichText::new(&param.name).size(13.0));
                    let slider_width = ui.available_width() - 110.0;
                    ui.add_sized(
                        [slider_width, 0.0],
                        egui::Slider::new(&mut param.value, param.min..=param.max)
                            .step_by(param.step),
                    );
                    ui.add_sized(
                        [80.0, 0.0],
                        egui::DragValue::new(&mut param.value)
                            .speed(param.step * 2.0)
                            .range(param.min..=param.max),
                    );
                });
                param.value = param.value.clamp(param.min, param.max);
                ui.add_space(2.0);
            }
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                if ui.button(RichText::new("下发参数").size(13.0)).clicked() {
                    let prefix = if app.selected_tab == "左电机" {
                        "L"
                    } else {
                        "R"
                    };
                    let base = if prefix == "L" { 0 } else { 5 };
                    if let Some(ref comm) = app.comm {
                        if base + 4 < app.params.len() {
                            let cmds = vec![
                                format!("SET_{}_TARGET: {}", prefix, app.params[base].value as i32),
                                format!("SET_{}_KP: {:.3}", prefix, app.params[base + 1].value),
                                format!("SET_{}_KI: {:.3}", prefix, app.params[base + 2].value),
                                format!("SET_{}_KD: {:.3}", prefix, app.params[base + 3].value),
                            ];
                            for cmd in &cmds {
                                let _ = comm.send_text(cmd);
                            }
                        }
                    } else {
                        app.log_err("[错误] 蓝牙未连接，无法下发参数");
                    }
                }
                if ui.button("恢复默认").clicked() {
                    app.reset_params();
                }
                if ui.button("保存配置").clicked() {
                    app.save_config();
                }
            });
            ui.add_space(4.0);
        });

    ui.separator();

    // ── 3. 文本指令 ──
    egui::CollapsingHeader::new("[指令] 串口")
        .default_open(true)
        .show(ui, |ui| {
            ui.add_space(4.0);

            ui.horizontal(|ui| {
                let resp = ui.add(
                    egui::TextEdit::singleline(&mut app.text_input)
                        .hint_text("输入指令...")
                        .desired_width(ui.available_width() - 60.0),
                );
                if resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                    app.send_text();
                }
                if ui.button("发送").clicked() {
                    app.send_text();
                }
            });

            // 快捷指令条：设置里配置的命令直接渲染成按钮，点击即发送
            if !app.custom_commands.is_empty() {
                ui.add_space(4.0);
                ui.label(RichText::new("快捷指令").small().weak());
                let cmds = app.custom_commands.clone();
                ui.horizontal_wrapped(|ui| {
                    for cmd in &cmds {
                        if ui
                            .button(RichText::new(cmd).size(12.0))
                            .on_hover_text("点击发送该指令")
                            .clicked()
                        {
                            app.send_command(cmd);
                        }
                    }
                });
            }

            ui.add_space(4.0);

            let log_height = ui.available_height().max(80.0);
            egui::Frame::group(ui.style())
                .fill(Color32::from_rgb(16, 18, 22))
                .stroke(egui::Stroke::new(1.0, Color32::from_rgb(50, 55, 65)))
                .show(ui, |ui| {
                    ui.set_min_height(log_height);
                    ScrollArea::vertical()
                        .id_salt("log_scroll")
                        .stick_to_bottom(app.log_auto_scroll)
                        .show(ui, |ui| {
                            ui.set_min_width(ui.available_width());
                            for line in app.log_lines.iter().rev().take(200).rev() {
                                let color = match line.kind {
                                    LogKind::Rx => Color32::from_rgb(180, 190, 200),
                                    LogKind::Tx => Color32::from_rgb(120, 200, 255),
                                    LogKind::Err => Color32::from_rgb(255, 130, 130),
                                    LogKind::Info => Color32::from_rgb(150, 220, 160),
                                };
                                ui.label(
                                    RichText::new(format!("[{}] {}", line.kind.label(), line.text))
                                        .size(12.0)
                                        .color(color),
                                );
                            }
                        });
                });

            ui.horizontal(|ui| {
                if ui.button("清空").clicked() {
                    app.clear_log();
                }
                if ui.button("导出").clicked() {
                    app.export_log();
                }
                ui.checkbox(&mut app.log_auto_scroll, "自动滚动");
            });
            ui.add_space(4.0);
        });

    ui.separator();

    // ── 4. 实时数据 ──
    egui::CollapsingHeader::new("[数据] 实时")
        .default_open(false)
        .show(ui, |ui| {
            ui.add_space(4.0);
            for (name, val) in &app.latest_raw {
                ui.horizontal(|ui| {
                    ui.label(RichText::new(format!("{}:", name)).size(13.0));
                    ui.colored_label(Color32::from_rgb(100, 220, 140), format!("{:.4}", val));
                });
            }
            if app.latest_raw.is_empty() {
                ui.label("等待数据...");
            }
            ui.add_space(4.0);
        });

    ui.allocate_space(Vec2::new(0.0, 8.0));
}

// ─── 右侧绘图区 ────────────────────────────────────────────

fn plot_panel(ui: &mut egui::Ui, app: &mut BlueInfoApp) {
    channel_bar(ui, app);
    ui.add_space(2.0);

    let plot = Plot::new("main_plot")
        .legend(Legend::default())
        .show_grid(true)
        .show_axes([true, true])
        .allow_boxed_zoom(true)
        .allow_double_click_reset(true)
        .allow_scroll(true)
        .set_margin_fraction(egui::Vec2::splat(0.05))
        .x_axis_label("时间 (s)")
        .y_axis_label("数值");

    plot.show(ui, |plot_ui| {
        for channel in &mut app.plot_channels {
            if !channel.visible {
                continue;
            }
            let pts_data = channel.points().to_vec();
            if pts_data.is_empty() {
                continue;
            }
            let ch_color = parse_color(&channel.color);
            let ch_name = channel.label().to_string();
            let ch_len = channel.len();
            let line = Line::new(PlotPoints::new(pts_data))
                .name(format!("{} ({}pts)", ch_name, ch_len))
                .color(ch_color)
                .width(1.5);
            plot_ui.line(line);
        }
    });
}

// ─── 绘图区顶部：通道显隐 / 重命名 ─────────────────────────

fn channel_bar(ui: &mut egui::Ui, app: &mut BlueInfoApp) {
    if app.plot_channels.is_empty() {
        return;
    }

    let mut toggled: Option<(usize, bool)> = None;
    let mut rename_request: Option<(usize, String)> = None;
    let editing = app.channel_name_edit.clone();

    ui.horizontal_wrapped(|ui| {
        ui.label(RichText::new("通道").small().weak());
        for idx in 0..app.plot_channels.len() {
            let label = app.plot_channels[idx].label().to_string();
            let mut visible = app.plot_channels[idx].visible;
            let is_editing = matches!(&editing, Some((i, _)) if *i == idx);
            if ui
                .checkbox(&mut visible, "")
                .on_hover_text("勾选显示/隐藏该通道曲线")
                .changed()
            {
                toggled = Some((idx, visible));
            }
            if is_editing {
                ui.label(RichText::new("编辑中…").size(12.0).weak());
            } else if ui
                .selectable_label(false, RichText::new(&label).size(12.0))
                .on_hover_text("双击重命名（仅前端显示，不改协议键）")
                .double_clicked()
            {
                rename_request = Some((idx, label));
            }
        }
    });

    if let Some((idx, visible)) = toggled {
        app.set_channel_visible(idx, visible);
    }
    if let Some(pair) = rename_request {
        app.channel_name_edit = Some(pair);
    }

    // 重命名编辑行：回车/确定提交，Esc/取消放弃
    if let Some((idx, mut text)) = app.channel_name_edit.take() {
        if idx >= app.plot_channels.len() {
            return;
        }
        let mut commit = false;
        let mut cancel = false;
        ui.horizontal(|ui| {
            ui.label(
                RichText::new(format!("重命名 {}:", app.plot_channels[idx].name))
                    .small()
                    .weak(),
            );
            let resp = ui.add(
                egui::TextEdit::singleline(&mut text)
                    .desired_width(180.0)
                    .hint_text("仅前端显示"),
            );
            if ui.button("确定").clicked()
                || (resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)))
            {
                commit = true;
            }
            if ui.button("取消").clicked() || ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                cancel = true;
            }
        });
        if commit {
            let trimmed = text.trim();
            if !trimmed.is_empty() {
                app.plot_channels[idx].display_name = Some(trimmed.to_string());
            }
        } else if !cancel {
            app.channel_name_edit = Some((idx, text));
        }
    }
}

// ─── 底部状态栏 ───────────────────────────────────────────
fn status_bar(ui: &mut egui::Ui, app: &mut BlueInfoApp) {
    ui.horizontal(|ui| {
        let is_connected = app.comm.as_ref().map(|c| c.is_connected()).unwrap_or(false);
        let (status_color, status_text) = if is_connected {
            (
                Color32::GREEN,
                format!(
                    "● {} @ {}bps",
                    app.comm.as_ref().unwrap().port_name,
                    app.comm.as_ref().unwrap().baud_rate
                ),
            )
        } else {
            (Color32::RED, "● 未连接".into())
        };
        ui.colored_label(status_color, status_text);
        if !is_connected && app.comm.is_some() && !app.user_disconnected && app.auto_reconnect {
            let hint = match app.next_retry_at {
                Some(due) => {
                    let remain = due
                        .saturating_duration_since(Instant::now())
                        .as_secs_f64();
                    format!(
                        "重连中：第 {} 次失败，{:.1}s 后重试",
                        app.reconnect_attempts, remain
                    )
                }
                None => "等待重连".to_string(),
            };
            ui.colored_label(Color32::from_rgb(255, 190, 90), hint);
        }
        ui.separator();
        ui.label(format!("采样: {}", app.packet_count));
        if app.frame_stats.bad_frames > 0 {
            ui.separator();
            ui.colored_label(
                Color32::from_rgb(255, 170, 90),
                format!("坏帧: {}", app.frame_stats.bad_frames),
            )
            .on_hover_text("协议解析失败并重同步的帧数（详见日志 [协议] 行）");
        }
        ui.separator();
        for (name, val) in &app.latest_raw {
            ui.label(
                RichText::new(format!("{}: {:.2}", name, val))
                    .size(12.0)
                    .color(Color32::from_rgb(100, 200, 140)),
            );
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.label(
                RichText::new(format!("FPS: {:.0}", app.fps))
                    .size(12.0)
                    .color(Color32::from_rgb(140, 160, 180)),
            );
        });
    });
}
// ─── 中文字体加载 ──────────────────────────────────────────
fn setup_chinese_fonts(ctx: &egui::Context) {
    use std::sync::Arc;
    let mut fonts = egui::FontDefinitions::default();
    let font_candidates = [
        ("C:/Windows/Fonts/msyh.ttc", "msyh"),
        ("C:/Windows/Fonts/msyhbd.ttc", "msyhbd"),
        ("C:/Windows/Fonts/simsun.ttc", "simsun"),
        ("C:/Windows/Fonts/simhei.ttf", "simhei"),
    ];
    for (font_path, font_name) in &font_candidates {
        if let Ok(data) = std::fs::read(font_path) {
            fonts.font_data.insert(
                font_name.to_string(),
                Arc::new(egui::FontData::from_owned(data)),
            );
            if let Some(list) = fonts.families.get_mut(&egui::FontFamily::Proportional) {
                list.insert(0, font_name.to_string());
            }
            if let Some(list) = fonts.families.get_mut(&egui::FontFamily::Monospace) {
                list.insert(0, font_name.to_string());
            }
            break;
        }
    }
    // 补充 Segoe UI Emoji，覆盖界面里的 emoji 字符
    let emoji_path = "C:/Windows/Fonts/seguiemj.ttf";
    if let Ok(data) = std::fs::read(emoji_path) {
        fonts
            .font_data
            .insert("emoji".into(), Arc::new(egui::FontData::from_owned(data)));
        if let Some(list) = fonts.families.get_mut(&egui::FontFamily::Proportional) {
            list.push("emoji".into());
        }
        if let Some(list) = fonts.families.get_mut(&egui::FontFamily::Monospace) {
            list.push("emoji".into());
        }
    }
    ctx.set_fonts(fonts);
}
// ─── 颜色工具 ──────────────────────────────────────────────
fn parse_color(hex: &str) -> Color32 {
    let hex = hex.trim_start_matches('#');
    if hex.len() == 6 {
        if let Ok(val) = u32::from_str_radix(hex, 16) {
            let r = ((val >> 16) & 0xFF) as u8;
            let g = ((val >> 8) & 0xFF) as u8;
            let b = (val & 0xFF) as u8;
            return Color32::from_rgb(r, g, b);
        }
    }
    Color32::LIGHT_BLUE
}

// ─── 单元测试（纯逻辑部分，不依赖串口设备）──────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hidden_channel_list_is_idempotent() {
        let mut hidden: Vec<String> = Vec::new();
        update_hidden(&mut hidden, "CH1", false);
        update_hidden(&mut hidden, "CH1", false);
        assert_eq!(hidden, vec!["CH1".to_string()]);
        update_hidden(&mut hidden, "CH2", false);
        assert_eq!(hidden.len(), 2);
        update_hidden(&mut hidden, "CH1", true);
        assert_eq!(hidden, vec!["CH2".to_string()]);
        // 重复恢复可见不会产生重复项或 panic
        update_hidden(&mut hidden, "CH1", true);
        assert_eq!(hidden, vec!["CH2".to_string()]);
        update_hidden(&mut hidden, "CH2", true);
        assert!(hidden.is_empty());
    }

    #[test]
    fn hidden_channels_stay_hidden_when_recreated() {
        let mut app = BlueInfoApp::new(false);
        app.hidden_channels = vec!["CH1".to_string()];
        app.plot_channels.clear();
        app.push_text_sample("CH1", 1.0, 0.0);
        app.push_text_sample("CH2", 2.0, 0.1);
        assert!(!app.plot_channels[0].visible, "隐藏通道不应因重建而复活");
        assert!(app.plot_channels[1].visible);
    }

    #[test]
    fn echo_and_log_lines_never_become_channels() {
        // 回显前缀识别
        assert!(is_echo_prefix(">>> SET_L_TARGET: 500"));
        assert!(is_echo_prefix("   [参数下发] @PARAM:[]"));
        assert!(!is_echo_prefix("CH1: 1"));

        // 只有 Rx 才可参与文本→曲线
        assert!(LogLine::rx("CH1: 1").is_plottable());
        assert!(!LogLine::tx(">>> CH1: 1").is_plottable());
        assert!(!LogLine::err("CH1: 1").is_plottable());
        assert!(!LogLine::info("[连接] 已连接").is_plottable());

        // 回显内容即使形如 KEY: value 也不会被解析成新通道
        assert!(parse_text_value(">>> SET_L_TARGET: 500").is_none());
        assert!(parse_text_value("[参数下发] CH1: 500").is_none());
        assert_eq!(parse_text_value("CH1: 1"), Some(("CH1", 1.0)));
        assert_eq!(parse_text_value("RPM = -3.5"), Some(("RPM", -3.5)));
    }

    #[test]
    fn ten_hz_text_stream_spans_one_second() {
        // 10 个采样、间隔 100ms：首尾跨度 0.9s（验收容差 ±0.1s，即 ≈1.0s±0.1s）
        let origin = Instant::now();
        let times: Vec<f64> = (0..10)
            .map(|i| rel_secs(origin, origin + Duration::from_millis(100 * i)))
            .collect();
        assert!(times.windows(2).all(|w| w[1] >= w[0]), "时间轴必须单调");
        let span = times[9] - times[0];
        assert!((span - 0.9).abs() <= 0.1, "跨度异常: {span}");
    }

    #[test]
    fn reconnect_keeps_timeline_continuous() {
        let now = Instant::now();
        // 上一段最后一个采样在相对时间 5.0s
        let origin = continuation_origin(now, 5.0);
        assert!((rel_secs(origin, now) - 5.0).abs() < 1e-9);
        // 重连后 100ms 的采样紧接上一段，不跳变、不重置
        assert!((rel_secs(origin, now + Duration::from_millis(100)) - 5.1).abs() < 1e-9);
        // last_secs 为负时不应产生未来原点
        let origin_zero = continuation_origin(now, -1.0);
        assert!((rel_secs(origin_zero, now) - 0.0).abs() < 1e-9);
    }

    #[test]
    fn reconnect_state_machine_retries_and_stops_on_disconnect() {
        let mut app = BlueInfoApp::new(false);
        app.auto_reconnect = true;
        app.user_disconnected = false;
        // 故意使用不存在的端口：connect() 必然失败，用于验证失败路径
        app.comm = Some(CommState::new(
            "__blueinfo_test_missing_port__".into(),
            115200,
        ));

        // 首次检测到掉线：立即第 1 次尝试，失败后保留 comm 并安排下一次
        app.maybe_reconnect();
        assert_eq!(app.reconnect_attempts, 1);
        assert!(app.comm.is_some(), "重连失败不能丢弃端口信息");
        assert!(app.next_retry_at.is_some());

        // 未到重试时间点：不重复尝试
        app.maybe_reconnect();
        assert_eq!(app.reconnect_attempts, 1);

        // 到点后继续按周期重试
        app.next_retry_at = Some(Instant::now() - Duration::from_millis(1));
        app.maybe_reconnect();
        assert_eq!(app.reconnect_attempts, 2);
        assert!(app.comm.is_some());

        // 用户手动「断开」后不再自动重试
        app.user_disconnected = true;
        app.next_retry_at = Some(Instant::now() - Duration::from_millis(1));
        app.maybe_reconnect();
        assert_eq!(app.reconnect_attempts, 2);
    }
}
