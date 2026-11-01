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
    default_params, DeviceParam, ParamValue, PlotChannel, CHART_COLORS, MAX_SAMPLES_PER_CHANNEL,
};
use eframe::egui::{self, Color32, RichText, ScrollArea, Vec2};
use egui_plot::{Legend, Line, Plot, PlotPoints};
use std::time::Instant;

// ─── 主应用状态 ────────────────────────────────────────────

pub struct BlueInfoApp {
    comm: Option<CommState>,
    config: AppConfig,

    available_ports: Vec<(String, String)>,
    selected_port: String,
    selected_baud: u32,
    reconnect_countdown: f64,

    params: Vec<ParamValue>,

    text_input: String,
    log_lines: Vec<String>,
    log_auto_scroll: bool,

    plot_channels: Vec<PlotChannel>,
    axis_auto_scale: bool,
    plot_time_start: f64,

    latest_raw: Vec<(String, f64)>,

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
}

impl BlueInfoApp {
    pub fn new() -> Self {
        let config = AppConfig::load();
        let ports: Vec<(String, String)> = CommState::available_ports();

        let params: Vec<ParamValue> = config.params.iter().map(ParamValue::from).collect();

        let selected_port =
            if !config.last_port.is_empty() && ports.iter().any(|(n, _)| n == &config.last_port) {
                config.last_port.clone()
            } else {
                ports.first().map(|(n, _)| n.clone()).unwrap_or_default()
            };

        Self {
            comm: None,
            config,
            available_ports: ports,
            selected_port,
            selected_baud: 115200,
            reconnect_countdown: 0.0,
            params,
            text_input: String::new(),
            log_lines: Vec::with_capacity(1000),
            log_auto_scroll: true,
            plot_channels: Vec::new(),
            axis_auto_scale: true,
            plot_time_start: 0.0,
            latest_raw: Vec::new(),
            frame_times: std::collections::VecDeque::with_capacity(120),
            fps: 60.0,
            packet_count: 0,
            last_stats_update: Instant::now(),
            auto_reconnect: true,
            user_disconnected: false,
            text_parse_enabled: true,
            style_initialized: false,
            custom_commands: AppConfig::load().quick_commands.clone(),
            new_command_input: String::new(),
            selected_tab: "左电机".into(),
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
                self.plot_time_start = 0.0;
                self.user_disconnected = false;
                self.plot_channels.clear();
                self.comm = Some(comm);
            }
            Err(e) => {
                self.log_lines.push(format!("[错误] {}", e));
                if self.log_lines.len() > 1000 {
                    self.log_lines.drain(0..500);
                }
            }
        }
    }

    fn disconnect(&mut self) {
        if let Some(mut comm) = self.comm.take() {
            let _ = comm.disconnect();
        }
        self.user_disconnected = true;
    }

    fn send_text(&mut self) {
        let text = self.text_input.trim().to_string();
        if text.is_empty() {
            return;
        }
        if let Some(ref comm) = self.comm {
            if let Err(e) = comm.send_text(&text) {
                self.log_lines.push(format!("[发送失败] {}", e));
                if self.log_lines.len() > 1000 {
                    self.log_lines.drain(0..500);
                }
            }
        } else {
            self.log_lines.push("[错误] 串口未连接".into());
        }
        self.text_input.clear();
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
            if let Err(e) = comm.send_params(&device_params) {
                self.log_lines.push(format!("[参数下发失败] {}", e));
            }
        } else {
            self.log_lines
                .push("[错误] 串口未连接，无法下发参数".into());
        }
    }

    fn reset_params(&mut self) {
        self.params = default_params().into_iter().map(ParamValue::from).collect();
        self.log_lines.push("[参数] 已恢复默认值".into());
    }

    fn save_config(&mut self) {
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
        };
        if let Some(ref comm) = self.comm {
            cfg.last_port = comm.port_name.clone();
            cfg.last_baud = comm.baud_rate;
        }
        cfg.save();
        self.config = cfg;
        self.log_lines
            .push("[配置] 已保存到 ~/.tao_tools/blueinfo/config.json".into());
    }

    fn export_log(&mut self) {
        let path = format!("blueinfo_log_{}.txt", chrono_now());
        if let Err(e) = std::fs::write(&path, self.log_lines.join("\n")) {
            self.log_lines.push(format!("[导出失败] {}: {}", path, e));
        } else {
            self.log_lines.push(format!("[导出] 日志已保存到 {}", path));
        }
    }

    fn clear_log(&mut self) {
        self.log_lines.clear();
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

// ??? ??????????

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

// ??? ??????? ??????????????????????????????????????
//
// ? MCU ????????? "KEY: value" ? "KEY = value"
// ?????????/?????????????

fn parse_text_value(line: &str) -> Option<(&str, f64)> {
    if let Some(idx) = line.find(':') {
        let key = line[..idx].trim();
        let val_str = line[idx + 1..].trim();
        let val_str = val_str.split_whitespace().next().unwrap_or(val_str);
        if let Ok(val) = val_str.parse::<f64>() {
            if !key.is_empty() && key.len() < 20 {
                return Some((key, val));
            }
        }
    }
    if let Some(idx) = line.find('=') {
        let key = line[..idx].trim();
        let val_str = line[idx + 1..].trim();
        let val_str = val_str.split_whitespace().next().unwrap_or(val_str);
        if let Ok(val) = val_str.parse::<f64>() {
            if !key.is_empty() && key.len() < 20 {
                return Some((key, val));
            }
        }
    }
    None
}

// ─── eframe App trait ──────────────────────────────────────

impl eframe::App for BlueInfoApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        if !self.style_initialized {
            setup_dark_theme(ctx);
            setup_chinese_fonts(ctx);
            self.style_initialized = true;
        }

        // ── 自动重连 ──
        if self.auto_reconnect
            && !self.user_disconnected
            && self.comm.is_some()
            && !self.comm.as_ref().unwrap().is_connected()
        {
            self.reconnect_countdown += ctx.input(|i| i.unstable_dt) as f64;
            if self.reconnect_countdown >= 3.0 {
                self.reconnect_countdown = 0.0;
                self.log_lines.push("[重连] 尝试重新连接...".into());
                let mut new_comm = CommState::new(self.selected_port.clone(), self.selected_baud);
                match new_comm.connect() {
                    Ok(()) => {
                        self.comm = Some(new_comm);
                        self.log_lines.push("[重连] 连接成功".into());
                    }
                    Err(e) => {
                        self.log_lines.push(format!("[重连失败] {}", e));
                        self.comm = None;
                    }
                }
                if self.log_lines.len() > 1000 {
                    self.log_lines.drain(0..500);
                }
            }
        }

        // ── 从后台线程拉取数据 ──
        if let Some(ref comm) = self.comm {
            if comm.is_connected() {
                let new_lines = comm.drain_lines();
                for line in &new_lines {
                    if self.log_lines.len() >= 1000 {
                        self.log_lines.drain(0..100);
                    }
                    self.log_lines.push(line.clone());

                    // ????????? -> ????
                    if self.text_parse_enabled {
                        if let Some((key, val)) = parse_text_value(line) {
                            let rel_time = if self.plot_channels.is_empty() {
                                self.plot_time_start as f64
                            } else {
                                self.plot_channels[0].len() as f64 * 0.01
                            };

                            // ????/????
                            let ch_idx = self.plot_channels.iter().position(|c| c.name == key);
                            let idx = match ch_idx {
                                Some(i) => i,
                                None => {
                                    let color =
                                        CHART_COLORS[self.plot_channels.len() % CHART_COLORS.len()];
                                    self.plot_channels.push(PlotChannel::new(key, color));
                                    self.plot_channels.len() - 1
                                }
                            };
                            self.plot_channels[idx].push(rel_time, val);
                            self.packet_count += 1;
                        }
                    }
                }

                let packets = comm.drain_packets();
                for pkt in &packets {
                    self.packet_count += 1;
                    if self.plot_time_start == 0.0 {
                        self.plot_time_start = pkt.timestamp;
                    }
                    let rel_time = pkt.timestamp - self.plot_time_start;

                    while self.plot_channels.len() < pkt.channel_count as usize {
                        let idx = self.plot_channels.len();
                        let color = CHART_COLORS[idx % CHART_COLORS.len()];
                        let name = format!("CH{}", idx + 1);
                        self.plot_channels.push(PlotChannel::new(&name, color));
                    }

                    for (i, val) in pkt.channel_data.iter().enumerate() {
                        if i < self.plot_channels.len() {
                            self.plot_channels[i].push(rel_time, *val);
                        }
                    }
                }

                let vals = comm.latest_values.lock().unwrap();
                self.latest_raw = vals.iter().map(|(k, v)| (k.clone(), *v)).collect();
            }
        }

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

// ─── 左侧面板 ──────────────────────────────────────────────

fn left_panel(ui: &mut egui::Ui, app: &mut BlueInfoApp) {
    // ── 1. 连接配置 ──
    egui::CollapsingHeader::new("蓝牙设备")
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
    egui::CollapsingHeader::new("蓝牙设备")
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
            let (range, prefix) = if app.selected_tab == "左电机" {
                (0..5, "L")
            } else {
                (5..10, "R")
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
                                app.log_lines.push(format!(">>> [{}] {}", prefix, cmd));
                            }
                        }
                    } else {
                        app.log_lines.push("[错误] 蓝牙未连接，无法下发参数".into());
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
                                ui.label(
                                    RichText::new(line)
                                        .size(12.0)
                                        .color(Color32::from_rgb(180, 190, 200)),
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
            let ch_name = channel.name.clone();
            let ch_len = channel.len();
            let line = Line::new(PlotPoints::new(pts_data))
                .name(format!("{} ({}pts)", ch_name, ch_len))
                .color(ch_color)
                .width(1.5);
            plot_ui.line(line);
        }
    });
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
        ui.separator();
        ui.label(format!("采样: {}", app.packet_count));
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
    // ?? Segoe UI Emoji ??? emoji ??
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
