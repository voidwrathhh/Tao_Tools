use crate::config::{AppConfig, DesktopItem, DesktopItemKind, DesktopState, GroupLayout};
use crate::desktop;
use crate::desktop_list;
use crate::tray::{self, TrayCommand, TrayController};
use eframe::egui::{
    self, Align, Align2, Area, Color32, CornerRadius, CursorIcon, FontId, Frame, Id, Margin, Pos2,
    Rect, RichText, ScrollArea, Sense, Stroke, Vec2,
};
use std::time::Instant;

// ---------- 主题色板（参考 DeskBox 深色玻璃风格） ----------
const ACCENT: Color32 = Color32::from_rgb(186, 132, 255);
const ACCENT_DIM: Color32 = Color32::from_rgb(146, 92, 214);
const DANGER: Color32 = Color32::from_rgb(243, 122, 131);
// egui Color32 以预乘 alpha 存储，以下常量均为预乘值
const TEXT_PRIMARY: Color32 = Color32::from_rgba_premultiplied(231, 232, 236, 245);
const TEXT_SECONDARY: Color32 = Color32::from_rgba_premultiplied(136, 142, 154, 220);
const PANEL_STROKE: Color32 = Color32::from_rgba_premultiplied(18, 18, 18, 18);
const CARD_BG: Color32 = Color32::from_rgba_premultiplied(14, 14, 14, 14);
const WINDOW_BG: Color32 = Color32::from_rgba_premultiplied(24, 26, 34, 250);
const SHADOW: egui::epaint::Shadow = egui::epaint::Shadow {
    offset: [0, 10],
    blur: 30,
    spread: 0,
    color: Color32::from_rgba_premultiplied(0, 0, 0, 120),
};

pub fn run() {
    let mut config = AppConfig::load();
    if config.scan.show_unclassified {
        if desktop::scan_into_config(&mut config).unwrap_or(0) > 0 {
            config.save();
        }
    }

    let viewport = egui::ViewportBuilder::default()
        .with_inner_size([config.desktop.width, config.desktop.height])
        .with_decorations(false)
        .with_transparent(true)
        .with_resizable(false)
        .with_title_shown(false)
        .with_titlebar_shown(false)
        .with_close_button(false)
        .with_minimize_button(false)
        .with_maximize_button(false)
        .with_taskbar(false)
        .with_title("Deskman");

    let options = eframe::NativeOptions {
        viewport,
        // wgpu 后端在 Windows 上才支持正确的 per-pixel alpha（glow 会整窗压暗）
        renderer: eframe::Renderer::Wgpu,
        ..Default::default()
    };

    let tray = TrayController::new();
    if let Err(e) = tray::start(tray.clone()) {
        eprintln!("警告：托盘初始化失败: {}", e);
    }

    let result = eframe::run_native(
        "Deskman",
        options,
        Box::new(move |_cc| Ok(Box::new(DeskmanApp::new(config, tray)))),
    );

    if let Err(e) = result {
        eprintln!("错误：GUI 启动失败: {}", e);
        std::process::exit(1);
    }
}

struct DeskmanApp {
    config: AppConfig,
    tray: TrayController,
    workspace_visible: bool,
    status: String,
    status_at: Option<Instant>,
    style_initialized: bool,
    editing_group: Option<usize>,
    settings_open: bool,
    new_item_name: String,
    new_item_path: String,
    new_group_name: String,
    request_exit: bool,
    interactive_rects: Vec<Rect>,
    applied_topmost: Option<bool>,
    applied_fullscreen: Option<bool>,
    applied_size: [f32; 2],
    delete_confirm: Option<(usize, Instant)>,
    icon_textures: std::collections::HashMap<String, Option<egui::TextureHandle>>,
    state: DesktopState,
    system_icon_size: Option<f32>,
    dock_scroll: f32,
    dock_scroll_target: f32,
    dock_spin: f32,
    dock_scales: Vec<f32>,
    dock_menu: Option<(egui::Response, usize)>,
    tick: u64,
    applied_bottom: bool,
}

impl DeskmanApp {
    fn new(config: AppConfig, tray: TrayController) -> Self {
        let mut config = config;
        // 默认锁定：编辑模式只在会话内通过 Ctrl+Alt+E 进入
        config.locked = true;
        let size = [config.desktop.width, config.desktop.height];
        Self {
            config,
            tray,
            workspace_visible: true,
            status: "就绪".to_string(),
            status_at: None,
            style_initialized: false,
            editing_group: None,
            settings_open: false,
            new_item_name: String::new(),
            new_item_path: String::new(),
            new_group_name: String::new(),
            request_exit: false,
            interactive_rects: Vec::new(),
            applied_topmost: None,
            applied_fullscreen: None,
            applied_size: size,
            delete_confirm: None,
            icon_textures: std::collections::HashMap::new(),
            state: DesktopState::load(),
            system_icon_size: None,
            dock_scroll: 0.0,
            dock_scroll_target: 0.0,
            dock_spin: 0.0,
            dock_scales: Vec::new(),
            dock_menu: None,
            tick: 0,
            applied_bottom: false,
        }
    }

    fn set_status(&mut self, text: impl Into<String>) {
        self.status = text.into();
        self.status_at = Some(Instant::now());
    }

    fn save_config(&mut self) {
        self.config.save();
    }

    fn scan_desktop(&mut self) {
        match desktop::scan_into_config(&mut self.config) {
            Ok(count) => {
                self.config.save();
                self.set_status(format!("已导入 {} 个桌面项目", count));
            }
            Err(e) => self.set_status(format!("扫描失败：{}", e)),
        }
    }

    fn handle_tray_commands(&mut self, ctx: &egui::Context) {
        for command in self.tray.drain() {
            match command {
                TrayCommand::ShowMainWindow => {
                    self.workspace_visible = true;
                    ctx.send_viewport_cmd(egui::ViewportCommand::Visible(true));
                    self.set_status("已显示桌面面板");
                }
                TrayCommand::HideMainWindow => {
                    self.workspace_visible = false;
                    ctx.send_viewport_cmd(egui::ViewportCommand::Visible(false));
                }
                TrayCommand::ToggleMainWindow => {
                    self.workspace_visible = !self.workspace_visible;
                    ctx.send_viewport_cmd(egui::ViewportCommand::Visible(self.workspace_visible));
                }
                TrayCommand::OpenSettings => {
                    self.settings_open = true;
                }
                TrayCommand::ScanDesktop => {
                    self.scan_desktop();
                }
                TrayCommand::ToggleLock => {
                    // 隐藏状态下：先恢复显示并进入编辑模式（工具条可用于解除隐藏）
                    if !self.workspace_visible {
                        self.workspace_visible = true;
                        self.config.locked = false;
                        self.save_config();
                        ctx.send_viewport_cmd(egui::ViewportCommand::Visible(true));
                        self.set_status("已显示面板（编辑模式，Ctrl+Alt+E 锁定）");
                    } else if self.config.locked {
                        self.config.locked = false;
                        self.save_config();
                        self.set_status("已进入编辑模式");
                    } else {
                        self.config.locked = true;
                        self.save_config();
                        self.set_status("已锁定布局");
                    }
                }
                TrayCommand::ToggleDesktopIcons => toggle_desktop_view(self),
                TrayCommand::RestoreDesktopIcons => {
                    restore_taken(&mut self.state);
                    self.set_status("已恢复桌面图标原位");
                }
                TrayCommand::Exit => {
                    exit_restore(&mut self.state);
                    self.request_exit = true;
                    self.save_config();
                    ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                }
            }
        }
    }

    fn sync_viewport(&mut self, ctx: &egui::Context) {
        let topmost = self.config.desktop.always_on_top;
        if self.applied_topmost != Some(topmost) {
            ctx.send_viewport_cmd(egui::ViewportCommand::WindowLevel(if topmost {
                egui::WindowLevel::AlwaysOnTop
            } else {
                egui::WindowLevel::Normal
            }));
            self.applied_topmost = Some(topmost);
        }

        let fullscreen = self.config.desktop.fullscreen;
        if self.applied_fullscreen != Some(fullscreen) {
            let size = if fullscreen {
                click_through::work_area_logical_size(ctx).unwrap_or(Vec2::new(
                    self.config.desktop.width,
                    self.config.desktop.height,
                ))
            } else {
                Vec2::new(self.config.desktop.width, self.config.desktop.height)
            };
            ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(size));
            ctx.send_viewport_cmd(egui::ViewportCommand::OuterPosition(Pos2::new(0.0, 0.0)));
            self.applied_fullscreen = Some(fullscreen);
            self.applied_size = [size.x, size.y];
        }

        if !fullscreen {
            let size = [self.config.desktop.width, self.config.desktop.height];
            if (self.applied_size[0] - size[0]).abs() > 1.0
                || (self.applied_size[1] - size[1]).abs() > 1.0
            {
                ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(Vec2::new(
                    size[0], size[1],
                )));
                self.applied_size = size;
            }
        }
    }

    /// 启动桌面流程：先恢复上次遗留状态（崩溃/强杀保护），再扫描并接管
    fn startup_desktop_flow(&mut self) {
        eprintln!("[flow] startup: taken={} view={}", self.state.taken.len(), self.state.view_toggled);
        if !self.state.taken.is_empty() || self.state.view_toggled {
            restore_taken(&mut self.state);
            self.set_status("已恢复上次的桌面图标布局");
            eprintln!("[flow] restored");
        }
        if self.config.desktop.takeover_icons {
            let count = apply_takeover(&mut self.state, &self.config);
            eprintln!("[flow] takeover count={}", count);
            if count > 0 {
                self.set_status(format!("已收纳 {} 个桌面图标", count));
            }
        }
        // Dock 为空时自动收纳全部桌面快捷方式
        if self.config.dock.enabled && self.config.dock.items.is_empty() {
            let count = fill_dock_from_desktop(self);
            eprintln!("[flow] dock autofill count={}", count);
        }
    }

    /// 面板实际使用的图标尺寸（跟随系统或手动）
    fn effective_icon_size(&self, group: &crate::config::DesktopGroup) -> f32 {
        if self.config.desktop.follow_system_icon_size {
            self.system_icon_size.unwrap_or(group.icon_size)
        } else {
            group.icon_size
        }
    }

    /// 获取（或提取并缓存）项目的真实图标纹理
    fn icon_texture(&mut self, ctx: &egui::Context, item: &DesktopItem) -> Option<egui::TextureHandle> {
        if !self.config.desktop.real_icons {
            return None;
        }
        match item.kind {
            DesktopItemKind::Command | DesktopItemKind::Url => return None,
            _ => {}
        }
        let key = item.path.to_lowercase();
        if let Some(cached) = self.icon_textures.get(&key) {
            return cached.clone();
        }
        let loaded = desktop::extract_icon_rgba(&item.path).and_then(|(rgba, w, h)| {
            if w == 0 || h == 0 {
                return None;
            }
            let image = egui::ColorImage::from_rgba_unmultiplied([w, h], &rgba);
            Some(ctx.load_texture(format!("deskman_icon_{}", key), image, egui::TextureOptions::LINEAR))
        });
        self.icon_textures.insert(key, loaded.clone());
        loaded
    }
}

impl eframe::App for DeskmanApp {
    fn clear_color(&self, _visuals: &egui::Visuals) -> [f32; 4] {
        // 透明窗口必须清除为全透明，否则整个窗口区域会有暗色蒙层
        [0.0, 0.0, 0.0, 0.0]
    }

    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        if !self.style_initialized {
            setup_style(ctx);
            setup_chinese_fonts(ctx);
            self.style_initialized = true;
            self.startup_desktop_flow();
        }
        click_through::enable_true_transparency();

        // 普通模式沉到普通层级最底部，任何窗口都能盖住它
        if !self.applied_bottom {
            let hwnd = click_through::current_hwnd();
            if hwnd != 0 {
                if !self.config.desktop.always_on_top {
                    desktop_list::send_to_bottom(hwnd);
                }
                self.applied_bottom = true;
            }
        }

        // 轮询系统桌面图标尺寸（约 3 秒一次）
        self.tick += 1;
        if self.config.desktop.follow_system_icon_size && self.tick % 30 == 0 {
            self.system_icon_size = desktop_list::system_icon_size_logical();
            // 同步到各分组并按新尺寸重排网格（旧坐标是按旧尺寸算的）
            if let Some(size) = self.system_icon_size {
                let changed = self
                    .config
                    .groups
                    .iter()
                    .any(|g| (g.icon_size - size).abs() > 1.0);
                if changed {
                    for group in &mut self.config.groups {
                        group.icon_size = size;
                    }
                    self.config.normalize_layouts(true);
                    self.save_config();
                }
            }
        }

        self.sync_viewport(ctx);
        self.handle_tray_commands(ctx);

        // "隐藏"只隐藏分类面板；工具条与 Dock 始终可用
        render_workspace(ctx, self);
        render_dock(ctx, self);

        // Dock 右键菜单（跨帧保持，动作见 render_dock 的 secondary_clicked 记录）
        if let Some((response, index)) = self.dock_menu.clone() {
            if index < self.config.dock.items.len() {
                let item = self.config.dock.items[index].clone();
                let name = item.name.clone();
                response.context_menu(|ui| {
                    ui.label(RichText::new(&name).size(12.0).color(TEXT_SECONDARY));
                    ui.separator();
                    if ui.button("打开").clicked() {
                        match desktop::open_item(&item) {
                            Ok(()) => self.set_status(format!("已打开：{}", item.name)),
                            Err(e) => self.set_status(format!("打开失败：{}", e)),
                        }
                        self.dock_menu = None;
                        ui.close_menu();
                    }
                    if ui.button("从 Dock 移除").clicked() {
                        self.config.dock.items.remove(index);
                        self.dock_menu = None;
                        self.save_config();
                        self.set_status("已从 Dock 移除");
                        ui.close_menu();
                    }
                });
            } else {
                self.dock_menu = None;
            }
        }

        update_click_through(ctx, self);

        draw_status_toast(ctx, self);
        ctx.request_repaint_after(std::time::Duration::from_millis(100));
    }

    fn on_exit(&mut self) {
        exit_restore(&mut self.state);
    }
}

// ---------- 桌面工作区 ----------

fn render_workspace(ctx: &egui::Context, app: &mut DeskmanApp) {
    app.interactive_rects.clear();

    egui::CentralPanel::default()
        .frame(Frame::new().fill(Color32::TRANSPARENT).inner_margin(0.0))
        .show(ctx, |_ui| {});

    workspace_toolbar(ctx, app);

    // 面板隐藏时跳过分类渲染，工具条保持可用
    if !app.workspace_visible {
        return;
    }

    for index in 0..app.config.groups.len() {
        desktop_group(ctx, app, index);
    }

    if let Some(index) = app.editing_group {
        edit_group_window(ctx, app, index);
    }
    if app.settings_open {
        settings_window(ctx, app);
    }
}

/// 底部 Dock 滑轨：回环轮盘（环形）与经典条状两种形态
fn render_dock(ctx: &egui::Context, app: &mut DeskmanApp) {
    if !app.config.dock.enabled || app.config.dock.items.is_empty() {
        return;
    }
    // 借全屏 Area 拿 Ui 做 interact（Context 没有直接 interact）
    let screen = ctx.screen_rect();
    egui::Area::new(Id::new("deskman_dock_layer"))
        .fixed_pos(screen.min)
        .order(egui::Order::Foreground)
        .show(ctx, |ui| {
            ui.set_min_size(screen.size());
            dock_inner(ui, app);
        });
}

#[allow(clippy::too_many_arguments)]
fn dock_inner(ui: &mut egui::Ui, app: &mut DeskmanApp) {
    let theme = app.panel_theme();
    let icon_size = app.config.dock.icon_size.clamp(28.0, 96.0);
    let magnify = app.config.dock.magnify.clamp(1.0, 3.0);
    let gap = icon_size * 0.22;
    let pad = gap.max(8.0);
    let n = app.config.dock.items.len();

    // 确保 dock_scales 与图标数量同步
    if app.dock_scales.len() != n {
        app.dock_scales = vec![1.0; n];
    }
    let screen = ui.ctx().screen_rect();
    let items = app.config.dock.items.clone();
    let loop_mode = app.config.dock.loop_mode;
    let pointer = ui.ctx().input(|i| i.pointer.hover_pos());

    let (bar_rect, max_scroll) = if loop_mode {
        let a = ((n as f32) * (icon_size + gap) * 0.5)
            .min(screen.width() * 0.46)
            .max(240.0);
        let base_y = screen.bottom() - icon_size * 0.9 - 12.0;
        (
            Rect::from_min_size(
                Pos2::new(screen.center().x - a, base_y - icon_size * 2.4),
                Vec2::new(a * 2.0, icon_size * 2.8),
            ),
            f32::INFINITY,
        )
    } else {
        let content_w = pad * 2.0 + n as f32 * icon_size + (n as f32 - 1.0).max(0.0) * gap;
        let max_w = screen.width() - 120.0;
        let bar_w = content_w.min(max_w).max(120.0);
        let max_scroll = (content_w - bar_w).max(0.0);
        let reflect_h = if app.config.dock.reflection {
            icon_size * 0.45
        } else {
            0.0
        };
        let bar_h = icon_size + reflect_h + pad * 2.0;
        (
            Rect::from_min_size(
                Pos2::new(screen.center().x - bar_w / 2.0, screen.bottom() - bar_h - 10.0),
                Vec2::new(bar_w, bar_h),
            ),
            max_scroll,
        )
    };

    // 滚动输入：回环模式写入旋转量（可无限环绕），条状模式写入平移目标
    let dock_hit = ui.interact(
        bar_rect,
        Id::new("dock_hit_zone"),
        Sense::click_and_drag(),
    );
    let wheel = ui.ctx().input(|i| i.raw_scroll_delta);
    if dock_hit.hovered() {
        let d = (wheel.x + wheel.y) * 1.2;
        if loop_mode {
            app.dock_spin += d;
        } else {
            app.dock_scroll_target = (app.dock_scroll_target - d).clamp(0.0, max_scroll);
        }
    }
    if dock_hit.dragged() {
        let dx = ui.ctx().input(|i| i.pointer.delta().x);
        if loop_mode {
            app.dock_spin -= dx;
        } else {
            app.dock_scroll_target = (app.dock_scroll_target - dx).clamp(0.0, max_scroll);
            app.dock_scroll = app.dock_scroll_target;
        }
    }

    // 逐图标目标缩放
    let mut targets = vec![1.0_f32; n];
    let mut places: Vec<(egui::Rect, f32, f32)> = vec![
        (Rect::from_min_size(Pos2::ZERO, Vec2::new(1.0, 1.0)), 1.0, 1.0);
        n
    ];

    if loop_mode {
        // ---- 回环轮盘：椭圆轨道，滚动即旋转，无缝环绕 ----
        let a = ((n as f32) * (icon_size + gap) * 0.5)
            .min(screen.width() * 0.46)
            .max(240.0);
        let b = icon_size * 0.95;
        let cx = screen.center().x;
        let base_y = screen.bottom() - icon_size * 0.9 - 12.0;
        let step = std::f32::consts::TAU / n as f32;
        let offset = app.dock_spin / (icon_size + gap) * step;
        let px = pointer.unwrap_or(Pos2::new(-9999.0, -9999.0)).x;
        let sigma = icon_size * 2.2;

        for i in 0..n {
            let theta = offset + i as f32 * step;
            let (sin_t, cos_t) = theta.sin_cos();
            let depth = (sin_t + 1.0) * 0.5; // 0 最远 1 最近
            let x = cx + a * cos_t;
            let y = base_y - (1.0 - depth) * b;
            let mut scale = 0.45 + 0.85 * depth;
            if depth > 0.55 {
                let d = (px - x).abs();
                scale += (magnify - 1.0) * (-(d * d) / (sigma * sigma)).exp() * depth;
            }
            targets[i] = scale;
            let alpha = 0.30 + 0.70 * depth;
            places[i] = (
                Rect::from_center_size(
                    Pos2::new(x, y - icon_size * scale / 2.0),
                    Vec2::new(icon_size * scale, icon_size * scale),
                ),
                alpha,
                depth,
            );
        }

        // 底部托板
        let painter = ui.ctx().layer_painter(egui::LayerId::new(
            egui::Order::Foreground,
            Id::new("dock_loop_shelf"),
        ));
        let shelf_w = (a * 1.9).min(screen.width() - 60.0);
        let shelf_h = icon_size * 0.52;
        let shelf_rect = Rect::from_center_size(
            Pos2::new(cx, base_y - shelf_h / 2.0 + 2.0),
            Vec2::new(shelf_w, shelf_h),
        );
        let opacity = app.config.dock.opacity.clamp(0.0, 1.0);
        if opacity > 0.01 {
            let d_alpha = (opacity * 255.0) as u8;
            let (r, g, b) = theme.panel_rgb;
            painter.rect_filled(
                shelf_rect,
                CornerRadius::same((shelf_h / 2.0) as u8),
                Color32::from_rgba_premultiplied(
                    (r as u32 * d_alpha as u32 / 255) as u8,
                    (g as u32 * d_alpha as u32 / 255) as u8,
                    (b as u32 * d_alpha as u32 / 255) as u8,
                    d_alpha,
                ),
            );
            painter.rect_stroke(
                shelf_rect,
                CornerRadius::same((shelf_h / 2.0) as u8),
                Stroke::new(1.0, theme.stroke),
                egui::epaint::StrokeKind::Inside,
            );
        }
    } else {
        // ---- 经典条状：线性排布 + 指针波浪 ----
        let scroll = app.dock_scroll;
        let px = pointer.unwrap_or(Pos2::new(-9999.0, -9999.0)).x - bar_rect.left();
        let sigma = icon_size * 1.5;
        let mut x = pad - scroll;
        for i in 0..n {
            let center_base = pad + i as f32 * (icon_size + gap) + icon_size / 2.0 - scroll;
            let d = (px - center_base).abs();
            targets[i] = 1.0 + (magnify - 1.0) * (-(d * d) / (sigma * sigma)).exp();
            let w = icon_size * targets[i];
            places[i] = (
                Rect::from_min_size(
                    Pos2::new(bar_rect.left() + x, bar_rect.top() + pad),
                    Vec2::new(w, icon_size),
                ),
                1.0,
                1.0,
            );
            x += w + gap;
        }
    }

    // 平滑插值 + 动画检测（修复移开鼠标后的恢复卡顿）
    let mut animating = false;
    for i in 0..n {
        let diff = targets[i] - app.dock_scales[i];
        if diff.abs() > 0.005 {
            animating = true;
        }
        app.dock_scales[i] += diff * 0.3;
    }
    if animating {
        ui.ctx().request_repaint_after(std::time::Duration::from_millis(16));
    }

    // 绘制：按深度从远到近
    let painter = ui.ctx().layer_painter(egui::LayerId::new(
        egui::Order::Foreground,
        Id::new("deskman_dock_icons"),
    ));
    let mut order: Vec<usize> = (0..n).collect();
    order.sort_by(|a, b| {
        places[*a]
            .2
            .partial_cmp(&places[*b].2)
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    for i in order {
        let (icon_rect, alpha, depth) = places[i];
        let alpha_byte = (alpha * 255.0) as u8;
        let item = &items[i];
        let icon_response = ui
            .interact(icon_rect, Id::new(("dock_icon", i)), Sense::click())
            .on_hover_cursor(CursorIcon::PointingHand)
            .on_hover_text(&item.name);

        let real_icon = app.icon_texture(ui.ctx(), item);
        if let Some(texture) = real_icon {
            painter.image(
                texture.id(),
                icon_rect,
                Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)),
                Color32::from_rgba_premultiplied(255, 255, 255, alpha_byte),
            );
            if app.config.dock.reflection && depth > 0.5 {
                let refl_h = icon_rect.height() * 0.5;
                let refl = Rect::from_min_max(
                    Pos2::new(
                        icon_rect.left() + icon_rect.width() * 0.06,
                        icon_rect.bottom() + 2.0,
                    ),
                    Pos2::new(
                        icon_rect.right() - icon_rect.width() * 0.06,
                        icon_rect.bottom() + 2.0 + refl_h,
                    ),
                );
                painter.image(
                    texture.id(),
                    refl,
                    Rect::from_min_max(Pos2::new(0.0, 1.0), Pos2::new(1.0, 0.0)),
                    Color32::from_rgba_premultiplied(
                        255,
                        255,
                        255,
                        (alpha_byte as u32 * 70 / 255) as u8,
                    ),
                );
            }
        } else {
            let (_, kind_color) = desktop::item_badge(&item.kind);
            let base = Color32::from_rgba_premultiplied(
                (kind_color.0 as u32 * alpha_byte as u32 / 255) as u8,
                (kind_color.1 as u32 * alpha_byte as u32 / 255) as u8,
                (kind_color.2 as u32 * alpha_byte as u32 / 255) as u8,
                alpha_byte,
            );
            painter.rect_filled(
                icon_rect,
                CornerRadius::same((icon_rect.width() * 0.24) as u8),
                base,
            );
        }

        if icon_response.clicked() {
            match desktop::open_item(item) {
                Ok(()) => app.set_status(format!("已打开：{}", item.name)),
                Err(e) => app.set_status(format!("打开失败：{}", e)),
            }
        }
        if icon_response.secondary_clicked() {
            app.dock_menu = Some((icon_response.clone(), i));
        }
    }

    app.interactive_rects.push(bar_rect);
}

/// 底部中央浮动工具条：只在编辑模式显示；锁定模式完全交给 Ctrl+Alt+E 和托盘
fn workspace_toolbar(ctx: &egui::Context, app: &mut DeskmanApp) {
    if app.config.locked {
        return;
    }
    let editing = true;

    let response = Area::new(Id::new("deskman_toolbar"))
        .anchor(Align2::CENTER_BOTTOM, [0.0, -14.0])
        .order(egui::Order::Foreground)
        .show(ctx, |ui| {
            Frame::new()
                .fill(Color32::from_rgba_unmultiplied(
                    18,
                    20,
                    26,
                    if editing { 220 } else { 140 },
                ))
                .stroke(Stroke::new(1.0, PANEL_STROKE))
                .corner_radius(CornerRadius::same(if editing { 14 } else { 12 }))
                .inner_margin(Margin::symmetric(10, 7))
                .shadow(SHADOW)
                .show(ui, |ui| {
                    ui.horizontal(|ui| {
                        if pill_button(ui, "重新扫描", ButtonStyle::Ghost).clicked() {
                            app.scan_desktop();
                        }
                        if pill_button(ui, "新建分类", ButtonStyle::Ghost).clicked() {
                            app.settings_open = true;
                        }
                        if pill_button(ui, "锁定", ButtonStyle::Ghost).clicked() {
                            app.config.locked = true;
                            app.save_config();
                            app.set_status("已锁定布局");
                        }
                        if pill_button(ui, "设置", ButtonStyle::Ghost).clicked() {
                            app.settings_open = true;
                        }
                        let panel_label = if app.workspace_visible {
                            "隐藏面板"
                        } else {
                            "显示面板"
                        };
                        if pill_button(ui, panel_label, ButtonStyle::Ghost).clicked() {
                            app.workspace_visible = !app.workspace_visible;
                            app.set_status(if app.workspace_visible {
                                "已显示面板"
                            } else {
                                "已隐藏面板（Ctrl+Alt+E 恢复）"
                            });
                        }
                        if pill_button(ui, "退出", ButtonStyle::DangerGhost).clicked() {
                            exit_restore(&mut app.state);
                            app.request_exit = true;
                            app.save_config();
                            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                        }
                    });
                })
                .response
        })
        .inner;

    app.interactive_rects.push(response.rect);
}

fn draw_status_toast(ctx: &egui::Context, app: &DeskmanApp) {
    if !app.workspace_visible {
        return;
    }
    let Some(at) = app.status_at else { return };
    let elapsed = at.elapsed().as_secs_f32();
    if elapsed > 2.2 {
        return;
    }
    let alpha = ((2.2 - elapsed) / 0.5).clamp(0.0, 1.0);
    let screen = ctx.screen_rect();
    let editing = !app.config.locked;
    let y = if editing {
        screen.bottom() - 88.0
    } else {
        screen.bottom() - 64.0
    };
    let painter = ctx.layer_painter(egui::LayerId::new(egui::Order::Tooltip, Id::new("toast")));
    let galley = painter.layout_no_wrap(
        app.status.clone(),
        FontId::proportional(12.0),
        Color32::from_rgba_unmultiplied(238, 241, 246, (235.0 * alpha) as u8),
    );
    let pad_x = 14.0;
    let size = galley.size() + Vec2::new(pad_x * 2.0, 12.0);
    let rect = Rect::from_center_size(Pos2::new(screen.center().x, y), size);
    painter.rect_filled(
        rect,
        CornerRadius::same(10),
        Color32::from_rgba_unmultiplied(18, 20, 26, (220.0 * alpha) as u8),
    );
    painter.rect_stroke(
        rect,
        CornerRadius::same(10),
        Stroke::new(1.0, PANEL_STROKE),
        egui::epaint::StrokeKind::Inside,
    );
    painter.galley(rect.left_top() + Vec2::new(pad_x, 6.0), galley, TEXT_PRIMARY);
}

// ---------- 面板样式主题 ----------

#[derive(Clone, Copy)]
struct PanelTheme {
    /// 面板底色 RGB（透明度按配置的不透明度套用）
    panel_rgb: (u8, u8, u8),
    stroke: Color32,
    corner: u8,
    shadow: egui::epaint::Shadow,
    text: Color32,
    text_dim: Color32,
    hover: Color32,
    label: Color32,
    label_hover: Color32,
    btn_text: Color32,
}

impl PanelTheme {
    fn fill(&self, opacity: f32) -> Color32 {
        let a = (opacity.clamp(0.35, 1.0) * 240.0) as u8;
        let (r, g, b) = self.panel_rgb;
        Color32::from_rgba_premultiplied(
            (r as u32 * a as u32 / 255) as u8,
            (g as u32 * a as u32 / 255) as u8,
            (b as u32 * a as u32 / 255) as u8,
            a,
        )
    }
}

const THEME_DARK_GLASS: PanelTheme = PanelTheme {
    panel_rgb: (23, 25, 32),
    stroke: Color32::from_rgba_premultiplied(18, 18, 18, 18),
    corner: 14,
    shadow: SHADOW,
    text: TEXT_PRIMARY,
    text_dim: TEXT_SECONDARY,
    hover: Color32::from_rgba_premultiplied(24, 24, 24, 16),
    label: Color32::from_rgba_premultiplied(213, 216, 222, 220),
    label_hover: Color32::from_rgba_premultiplied(231, 232, 236, 245),
    btn_text: TEXT_SECONDARY,
};

const THEME_IOS_LIGHT: PanelTheme = PanelTheme {
    panel_rgb: (250, 250, 252),
    stroke: Color32::from_rgba_premultiplied(10, 10, 12, 26),
    corner: 20,
    shadow: egui::epaint::Shadow {
        offset: [0, 8],
        blur: 36,
        spread: 0,
        color: Color32::from_rgba_premultiplied(0, 0, 0, 46),
    },
    text: Color32::from_rgba_premultiplied(8, 8, 10, 248),
    text_dim: Color32::from_rgba_premultiplied(66, 66, 70, 190),
    hover: Color32::from_rgba_premultiplied(6, 6, 8, 16),
    label: Color32::from_rgba_premultiplied(48, 48, 52, 238),
    label_hover: Color32::from_rgba_premultiplied(10, 10, 12, 250),
    btn_text: Color32::from_rgba_premultiplied(66, 66, 70, 210),
};

const THEME_IOS_DARK: PanelTheme = PanelTheme {
    panel_rgb: (28, 28, 30),
    stroke: Color32::from_rgba_premultiplied(255, 255, 255, 26),
    corner: 20,
    shadow: egui::epaint::Shadow {
        offset: [0, 10],
        blur: 32,
        spread: 0,
        color: Color32::from_rgba_premultiplied(0, 0, 0, 96),
    },
    text: Color32::from_rgba_premultiplied(250, 250, 252, 245),
    text_dim: Color32::from_rgba_premultiplied(160, 160, 168, 200),
    hover: Color32::from_rgba_premultiplied(255, 255, 255, 22),
    label: Color32::from_rgba_premultiplied(214, 214, 220, 226),
    label_hover: Color32::from_rgba_premultiplied(248, 248, 250, 248),
    btn_text: Color32::from_rgba_premultiplied(170, 170, 178, 210),
};

impl DeskmanApp {
    fn panel_theme(&self) -> PanelTheme {
        match self.config.desktop.style.as_str() {
            "ios-light" => THEME_IOS_LIGHT,
            "ios-dark" => THEME_IOS_DARK,
            _ => THEME_DARK_GLASS,
        }
    }
}

fn desktop_group(ctx: &egui::Context, app: &mut DeskmanApp, index: usize) {
    let group_id = app.config.groups[index].id.clone();
    let pos = Pos2::new(app.config.groups[index].x, app.config.groups[index].y);
    let width = app.config.groups[index].width;
    let height = app.config.groups[index].height;
    let movable = !app.config.locked && !app.config.groups[index].position_locked;
    let opacity = app.config.desktop.opacity.clamp(0.35, 1.0);
    let collapsed = app.config.groups[index].collapsed;
    let theme = app.panel_theme();
    Area::new(Id::new(format!("desktop_group_{}", group_id)))
        .fixed_pos(pos)
        .movable(movable)
        .show(ctx, |ui| {
            let capsule = collapsed;
            let radius = if capsule {
                (height.min(40.0) / 2.0) as u8
            } else {
                theme.corner
            };
            let response = Frame::new()
                .fill(theme.fill(opacity))
                .stroke(Stroke::new(1.0, theme.stroke))
                .corner_radius(CornerRadius::same(radius))
                .inner_margin(Margin::ZERO)
                .shadow(theme.shadow)
                .show(ui, |ui| {
                    ui.set_width(width);
                    if capsule {
                        group_capsule(ui, app, index, width, height, &theme);
                    } else {
                        group_header(ui, app, index, width, &theme);
                        group_body(ui, app, index, width, height, &theme);
                    }
                })
                .response;

            if movable {
                let rect = response.rect;
                app.config.groups[index].x = rect.min.x;
                app.config.groups[index].y = rect.min.y;
            }
            app.interactive_rects.push(response.rect);
        });
}

fn group_header(
    ui: &mut egui::Ui,
    app: &mut DeskmanApp,
    index: usize,
    width: f32,
    theme: &PanelTheme,
) {
    let header_height = 44.0;
    let (header_rect, header_response) =
        ui.allocate_exact_size(Vec2::new(width, header_height), Sense::click());
    let editing = !app.config.locked;
    let hovered = ui.rect_contains_pointer(header_rect) || editing;

    let mut header_ui = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(header_rect.shrink2(Vec2::new(14.0, 0.0)))
            .layout(egui::Layout::left_to_right(Align::Center)),
    );

    header_ui.horizontal(|ui| {
        ui.label(
            RichText::new(app.config.groups[index].name.clone())
                .strong()
                .size(13.5)
                .color(theme.text),
        );
    });

    // 面板头右键菜单（平时不显示任何编辑选项）
    header_response.context_menu(|ui| {
        let collapsed = app.config.groups[index].collapsed;
        if ui
            .button(if collapsed { "展开面板" } else { "收起面板" })
            .clicked()
        {
            app.config.groups[index].collapsed = !collapsed;
            app.save_config();
            ui.close_menu();
        }
        if ui.button("面板设置").clicked() {
            app.editing_group = Some(index);
            ui.close_menu();
        }
        if app.config.locked && ui.button("进入编辑模式 (Ctrl+Alt+E)").clicked() {
            app.config.locked = false;
            app.save_config();
            app.set_status("已进入编辑模式");
            ui.close_menu();
        }
    });

    // 右侧动作：编辑模式才显示"设置"；收起始终只做收纳相关操作
    let mut actions_ui = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(header_rect.shrink2(Vec2::new(12.0, 0.0)))
            .layout(egui::Layout::right_to_left(Align::Center)),
    );
    actions_ui.horizontal(|ui| {
        if editing {
            if ghost_small_button_painted(ui, "收起", theme.btn_text, theme.hover).clicked() {
                app.config.groups[index].collapsed = true;
                app.save_config();
            }
            if ghost_small_button_painted(ui, "设置", theme.btn_text, theme.hover).clicked() {
                app.editing_group = Some(index);
            }
        } else if hovered
            && ghost_small_button_painted(ui, "收起", theme.btn_text, theme.hover).clicked()
        {
            app.config.groups[index].collapsed = true;
            app.save_config();
        }
    });
}

fn group_capsule(
    ui: &mut egui::Ui,
    app: &mut DeskmanApp,
    index: usize,
    width: f32,
    height: f32,
    theme: &PanelTheme,
) {
    let capsule_h = height.min(40.0);
    let (rect, _) = ui.allocate_exact_size(Vec2::new(width, capsule_h), Sense::hover());
    let mut inner = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(rect.shrink2(Vec2::new(14.0, 0.0)))
            .layout(egui::Layout::left_to_right(Align::Center)),
    );
    inner.horizontal(|ui| {
        ui.label(
            RichText::new(app.config.groups[index].name.clone())
                .strong()
                .size(13.0)
                .color(theme.text),
        );
    });
    let mut actions = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(rect.shrink2(Vec2::new(12.0, 0.0)))
            .layout(egui::Layout::right_to_left(Align::Center)),
    );
    actions.horizontal(|ui| {
        if ghost_small_button_painted(ui, "展开", theme.btn_text, theme.hover).clicked() {
            app.config.groups[index].collapsed = false;
            app.save_config();
        }
    });
}

fn group_body(
    ui: &mut egui::Ui,
    app: &mut DeskmanApp,
    index: usize,
    width: f32,
    height: f32,
    theme: &PanelTheme,
) {
    let body_height = (height - 44.0).max(60.0);
    // 显式子区域：左右各留 12px，避免 ScrollArea 在嵌套布局中测量异常
    let body_rect = Rect::from_min_size(
        Pos2::new(ui.cursor().left() + 12.0, ui.cursor().top() + 2.0),
        Vec2::new((width - 24.0).max(1.0), body_height),
    );
    let layout = app.config.groups[index].layout.clone();
    let items = app.config.groups[index].items.clone();
    let group_index = index;
    ui.allocate_new_ui(
        egui::UiBuilder::new()
            .max_rect(body_rect)
            .layout(egui::Layout::top_down(Align::Min)),
        |body_ui| match layout {
            GroupLayout::Free => {
                free_icon_canvas(body_ui, app, group_index, width, body_height, theme);
            }
            GroupLayout::Horizontal => {
                if items.is_empty() {
                    empty_hint(body_ui, body_height, theme);
                }
                ScrollArea::horizontal()
                    .max_height(body_height)
                    .show(body_ui, |ui| {
                        ui.horizontal(|ui| {
                            for (item_index, item) in items.iter().enumerate() {
                                icon_tile_list(ui, app, group_index, item_index, item, theme);
                            }
                        });
                    });
            }
            GroupLayout::Grid => {
                if items.is_empty() {
                    empty_hint(body_ui, body_height, theme);
                }
                ScrollArea::vertical()
                    .max_height(body_height)
                    .show(body_ui, |ui| {
                        ui.set_width(width - 24.0);
                        ui.horizontal_wrapped(|ui| {
                            for (item_index, item) in items.iter().enumerate() {
                                icon_tile_list(ui, app, group_index, item_index, item, theme);
                            }
                        });
                    });
            }
        },
    );
}

fn empty_hint(ui: &mut egui::Ui, height: f32, theme: &PanelTheme) {
    ui.allocate_ui_with_layout(
        Vec2::new(ui.available_width().max(160.0), height.max(60.0)),
        egui::Layout::centered_and_justified(egui::Direction::TopDown),
        |ui| {
            ui.add_space((height.max(60.0) / 2.0 - 26.0).max(0.0));
            ui.label(
                RichText::new("还没有项目")
                    .size(12.5)
                    .color(theme.text_dim.linear_multiply(0.85)),
            );
            ui.label(
                RichText::new("在面板设置中添加，或扫描桌面导入")
                    .size(11.0)
                    .color(theme.text_dim.linear_multiply(0.55)),
            );
        },
    );
}

// ---------- 图标 ----------

fn icon_tile_list(
    ui: &mut egui::Ui,
    app: &mut DeskmanApp,
    group_index: usize,
    item_index: usize,
    item: &DesktopItem,
    theme: &PanelTheme,
) {
    let icon_size = app.effective_icon_size(&app.config.groups[group_index]);
    let real_icon = app.icon_texture(ui.ctx(), item);
    let tile = Vec2::new(icon_size + 12.0, icon_size + 30.0);
    let (rect, response) = ui.allocate_exact_size(tile, Sense::click());
    let response = response.on_hover_cursor(CursorIcon::PointingHand);
    paint_icon_tile(
        ui,
        rect,
        item,
        response.hovered(),
        icon_size,
        real_icon.as_ref(),
        theme,
        app.config.desktop.show_labels,
    );
    if response.clicked() {
        match desktop::open_item(item) {
            Ok(()) => app.set_status(format!("已打开：{}", item.name)),
            Err(e) => app.set_status(format!("打开失败：{}", e)),
        }
    }
    tile_context_menu(ui, &response, app, group_index, item_index, item);
}

/// 图标右键菜单：打开 / 移动到分类 / 从面板移除
fn tile_context_menu(
    _ui: &mut egui::Ui,
    response: &egui::Response,
    app: &mut DeskmanApp,
    group_index: usize,
    item_index: usize,
    item: &DesktopItem,
) {
    let name = item.name.clone();
    response.context_menu(|ui| {
        ui.label(RichText::new(&name).size(12.0).color(TEXT_SECONDARY));
        ui.separator();
        if ui.button("打开").clicked() {
            match desktop::open_item(item) {
                Ok(()) => app.set_status(format!("已打开：{}", name)),
                Err(e) => app.set_status(format!("打开失败：{}", e)),
            }
            ui.close_menu();
        }
        ui.menu_button("移动到分类", |ui| {
            let targets: Vec<(usize, String)> = app
                .config
                .groups
                .iter()
                .enumerate()
                .filter(|(i, _)| *i != group_index)
                .map(|(i, g)| (i, g.name.clone()))
                .collect();
            if targets.is_empty() {
                ui.label(RichText::new("没有其他分类").size(11.5).color(TEXT_SECONDARY));
            }
            for (target, target_name) in targets {
                if ui.button(target_name).clicked() {
                    move_item_between_groups(app, group_index, item_index, target);
                    ui.close_menu();
                }
            }
        });
        ui.separator();
        let in_dock = app
            .config
            .dock
            .items
            .iter()
            .any(|d| d.path == item.path);
        if ui
            .button(if in_dock { "已在 Dock 中" } else { "添加到 Dock" })
            .clicked()
        {
            if !in_dock {
                let mut dock_item = item.clone();
                dock_item.x = 0.0;
                dock_item.y = 0.0;
                app.config.dock.items.push(dock_item);
                app.save_config();
                app.set_status("已添加到 Dock");
            }
            ui.close_menu();
        }
        ui.separator();
        if ui.button("从面板移除").clicked() {
            remove_item_at(app, group_index, item_index);
            ui.close_menu();
        }
    });
}

fn move_item_between_groups(app: &mut DeskmanApp, from: usize, item_index: usize, to: usize) {
    if from == to
        || from >= app.config.groups.len()
        || to >= app.config.groups.len()
        || item_index >= app.config.groups[from].items.len()
    {
        return;
    }
    let mut item = app.config.groups[from].items.remove(item_index);
    item.x = 0.0;
    item.y = 0.0;
    let target_name = app.config.groups[to].name.clone();
    app.config.groups[to].items.push(item);
    desktop::place_item_in_group(&mut app.config.groups[to]);
    app.save_config();
    app.set_status(format!("已移动到「{}」", target_name));
}

fn remove_item_at(app: &mut DeskmanApp, group_index: usize, item_index: usize) {
    if group_index >= app.config.groups.len()
        || item_index >= app.config.groups[group_index].items.len()
    {
        return;
    }
    let item = app.config.groups[group_index].items.remove(item_index);
    app.save_config();
    app.set_status(format!("已移除「{}」", item.name));
}

fn free_icon_canvas(
    ui: &mut egui::Ui,
    app: &mut DeskmanApp,
    group_index: usize,
    width: f32,
    height: f32,
    theme: &PanelTheme,
) {
    let locked = app.config.locked;
    let icon_size = app.effective_icon_size(&app.config.groups[group_index]);
    let tile_size = Vec2::new(icon_size + 12.0, icon_size + 30.0);
    let canvas_size = Vec2::new((width - 24.0).max(200.0), height.max(80.0));
    let (canvas_rect, _) = ui.allocate_exact_size(canvas_size, Sense::hover());

    if app.config.groups[group_index].items.is_empty() {
        let mut hint_ui = ui.new_child(
            egui::UiBuilder::new()
                .max_rect(canvas_rect)
                .layout(egui::Layout::centered_and_justified(egui::Direction::TopDown)),
        );
        hint_ui.add_space((canvas_rect.height() / 2.0 - 26.0).max(0.0));
        hint_ui.label(
            RichText::new("还没有项目")
                .size(12.5)
                .color(theme.text_dim.linear_multiply(0.85)),
        );
        hint_ui.label(
            RichText::new("在面板设置中添加，或扫描桌面导入")
                .size(11.0)
                .color(theme.text_dim.linear_multiply(0.55)),
        );
    }

    let origin = canvas_rect.min + Vec2::new(8.0, 4.0);
    let mut dirty = false;
    let mut status_update: Option<String> = None;
    let mut open_update: Option<(String, Result<(), String>)> = None;
    let mut pending_menu: Option<(egui::Response, usize)> = None;

    // 先取真实图标（避免与可变借用冲突）
    let items_snapshot = app.config.groups[group_index].items.clone();
    let textures: Vec<Option<egui::TextureHandle>> = items_snapshot
        .iter()
        .map(|it| app.icon_texture(ui.ctx(), it))
        .collect();

    {
        let group = &mut app.config.groups[group_index];
        for (item_index, item) in group.items.iter_mut().enumerate() {
            let pos = origin + Vec2::new(item.x, item.y);
            let rect = Rect::from_min_size(pos, tile_size);
            let id = ui.id().with(("deskman_icon", group_index, item_index));
            let response = ui.interact(
                rect,
                id,
                if locked {
                    Sense::click()
                } else {
                    Sense::click_and_drag()
                },
            );
            let response = response.on_hover_cursor(if locked {
                CursorIcon::PointingHand
            } else {
                CursorIcon::Grab
            });

            paint_icon_tile(
                ui,
                rect,
                item,
                response.hovered(),
                icon_size,
                textures.get(item_index).and_then(|t| t.as_ref()),
                theme,
                app.config.desktop.show_labels,
            );

            if response.clicked() {
                open_update = Some((item.name.clone(), desktop::open_item(item)));
            }

            if response.secondary_clicked() {
                pending_menu = Some((response.clone(), item_index));
            }

            if !locked && response.dragged() {
                let delta = ui.ctx().input(|i| i.pointer.delta());
                if delta != Vec2::ZERO {
                    item.x = (item.x + delta.x).max(0.0);
                    item.y = (item.y + delta.y).max(0.0);
                    dirty = true;
                    ui.ctx().request_repaint();
                }
            }
        }
    }

    if let Some((response, item_index)) = pending_menu {
        if let Some(item) = items_snapshot.get(item_index) {
            tile_context_menu(ui, &response, app, group_index, item_index, item);
        }
    }

    if dirty && ui.ctx().input(|i| i.pointer.any_released()) {
        app.save_config();
        status_update = Some("图标位置已保存".to_string());
    }

    if let Some((name, result)) = open_update {
        match result {
            Ok(()) => status_update = Some(format!("已打开：{}", name)),
            Err(e) => status_update = Some(format!("打开失败：{}", e)),
        }
    }

    if let Some(status) = status_update {
        app.set_status(status);
    }
}

fn paint_icon_tile(
    ui: &mut egui::Ui,
    rect: Rect,
    item: &DesktopItem,
    hovered: bool,
    icon_size: f32,
    real_icon: Option<&egui::TextureHandle>,
    theme: &PanelTheme,
    show_labels: bool,
) {
    let painter = ui.painter_at(rect.expand(30.0));
    let (_, kind_color) = desktop::item_badge(&item.kind);

    if hovered {
        painter.rect_filled(rect, CornerRadius::same(10), theme.hover);
    }

    let badge_size = icon_size.min(rect.width() - 8.0);
    let badge_rect = Rect::from_center_size(
        Pos2::new(rect.center().x, rect.top() + 5.0 + badge_size / 2.0),
        Vec2::new(badge_size, badge_size),
    );
    let badge_radius = ((badge_size * 0.27).max(6.0)) as u8;

    if let Some(texture) = real_icon {
        // 真实应用图标：按原始比例居中绘制
        let size = texture.size_vec2();
        let draw_side = badge_size;
        let scale = (draw_side / size.x).min(draw_side / size.y);
        let draw_size = Vec2::new(size.x * scale, size.y * scale);
        let draw_rect = Rect::from_center_size(badge_rect.center(), draw_size);
        painter.image(
            texture.id(),
            draw_rect,
            Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)),
            Color32::WHITE,
        );
    } else {
        let base = if hovered {
            lerp_color(
                Color32::from_rgb(kind_color.0, kind_color.1, kind_color.2),
                Color32::WHITE,
                0.14,
            )
        } else {
            Color32::from_rgb(kind_color.0, kind_color.1, kind_color.2)
        };
        painter.rect_filled(badge_rect, CornerRadius::same(badge_radius), base);
        // 底部轻微压暗，模拟纵向渐变
        painter.rect_filled(
            Rect::from_min_max(
                Pos2::new(badge_rect.left(), badge_rect.center().y),
                badge_rect.max,
            ),
            CornerRadius {
                nw: 0,
                ne: 0,
                sw: badge_radius,
                se: badge_radius,
            },
            Color32::from_rgba_unmultiplied(0, 0, 0, 26),
        );
        paint_kind_glyph(ui, badge_rect, &item.kind);
    }

    if show_labels {
        let label_color = if hovered {
            theme.label_hover
        } else {
            theme.label
        };
        let font_size = (icon_size * 0.2).clamp(10.0, 12.5);
        let label_rect = Rect::from_min_max(
            Pos2::new(rect.left() + 2.0, badge_rect.max.y + 3.0),
            Pos2::new(rect.right() - 2.0, rect.bottom() - 1.0),
        );
        // 用 new_child 绘制标签：不向父级回报尺寸，避免破坏 wrapped 布局状态
        let mut label_ui = ui.new_child(
            egui::UiBuilder::new()
                .max_rect(label_rect)
                .layout(egui::Layout::centered_and_justified(egui::Direction::TopDown)),
        );
        label_ui.add(
            egui::Label::new(RichText::new(&item.name).size(font_size).color(label_color))
                .truncate(),
        );
    }
}

/// 用矢量图形绘制项目类型图标（避免字符缺字）
fn paint_kind_glyph(ui: &mut egui::Ui, badge: Rect, kind: &DesktopItemKind) {
    let painter = ui.painter_at(badge.expand(2.0));
    let c = badge.center();
    let d = badge.width() * 0.26;
    let white = Color32::from_rgba_unmultiplied(255, 255, 255, 242);
    let stroke = Stroke::new((badge.width() * 0.075).max(1.5), white);

    match kind {
        DesktopItemKind::Shortcut => {
            let a = Pos2::new(c.x - d * 0.85, c.y + d * 0.85);
            let b = Pos2::new(c.x + d * 0.7, c.y - d * 0.7);
            painter.line_segment([a, b], stroke);
            painter.line_segment([b, Pos2::new(b.x - d * 0.62, b.y)], stroke);
            painter.line_segment([b, Pos2::new(b.x, b.y + d * 0.62)], stroke);
        }
        DesktopItemKind::Url => {
            painter.circle_stroke(c, d * 0.95, stroke);
            painter.line_segment(
                [Pos2::new(c.x - d * 0.95, c.y), Pos2::new(c.x + d * 0.95, c.y)],
                Stroke::new(stroke.width * 0.8, white),
            );
            painter.line_segment(
                [Pos2::new(c.x, c.y - d * 0.95), Pos2::new(c.x, c.y + d * 0.95)],
                Stroke::new(stroke.width * 0.8, white),
            );
        }
        DesktopItemKind::Folder => {
            painter.rect_filled(
                Rect::from_min_size(Pos2::new(c.x - d, c.y - d * 0.75), Vec2::new(d * 1.1, d * 0.5)),
                CornerRadius::same(2),
                white,
            );
            painter.rect_filled(
                Rect::from_min_size(
                    Pos2::new(c.x - d, c.y - d * 0.4),
                    Vec2::new(d * 2.0, d * 1.15),
                ),
                CornerRadius::same(3),
                white,
            );
        }
        DesktopItemKind::File => {
            // 描边文档页 + 文本行
            let w = d * 1.35;
            let h = d * 1.75;
            let page = Rect::from_min_size(Pos2::new(c.x - w / 2.0, c.y - h / 2.0), Vec2::new(w, h));
            painter.rect_stroke(
                page,
                CornerRadius::same(3),
                stroke,
                egui::epaint::StrokeKind::Inside,
            );
            let line = Stroke::new(stroke.width * 0.75, white);
            painter.line_segment(
                [
                    Pos2::new(page.left() + w * 0.2, c.y - h * 0.16),
                    Pos2::new(page.right() - w * 0.2, c.y - h * 0.16),
                ],
                line,
            );
            painter.line_segment(
                [
                    Pos2::new(page.left() + w * 0.2, c.y + h * 0.18),
                    Pos2::new(page.right() - w * 0.38, c.y + h * 0.18),
                ],
                line,
            );
        }
        DesktopItemKind::Command => {
            painter.line_segment(
                [
                    Pos2::new(c.x - d * 0.8, c.y - d * 0.55),
                    Pos2::new(c.x - d * 0.2, c.y),
                ],
                stroke,
            );
            painter.line_segment(
                [
                    Pos2::new(c.x - d * 0.2, c.y),
                    Pos2::new(c.x - d * 0.8, c.y + d * 0.55),
                ],
                stroke,
            );
            painter.line_segment(
                [
                    Pos2::new(c.x + d * 0.05, c.y + d * 0.55),
                    Pos2::new(c.x + d * 0.8, c.y + d * 0.55),
                ],
                stroke,
            );
        }
    }
}


// ---------- 通用控件 ----------

#[derive(Clone, Copy, PartialEq)]
enum ButtonStyle {
    Primary,
    Ghost,
    DangerGhost,
}

fn pill_button(ui: &mut egui::Ui, text: &str, style: ButtonStyle) -> egui::Response {
    let text_color = match style {
        ButtonStyle::Primary => Color32::WHITE,
        ButtonStyle::Ghost => TEXT_PRIMARY,
        ButtonStyle::DangerGhost => DANGER,
    };
    let fill = match style {
        ButtonStyle::Primary => ACCENT_DIM,
        _ => Color32::TRANSPARENT,
    };
    let button = egui::Button::new(RichText::new(text).size(12.0).color(text_color))
        .fill(fill)
        .stroke(Stroke::NONE)
        .corner_radius(CornerRadius::same(8))
        .min_size(Vec2::new(0.0, 24.0));
    let response = ui.add(button);
    if response.hovered() {
        let hover_bg = match style {
            ButtonStyle::Primary => ACCENT,
            ButtonStyle::Ghost => Color32::from_rgba_unmultiplied(255, 255, 255, 24),
            ButtonStyle::DangerGhost => Color32::from_rgba_unmultiplied(243, 122, 131, 40),
        };
        ui.painter()
            .rect_filled(response.rect, CornerRadius::same(8), hover_bg);
    }
    response.on_hover_cursor(CursorIcon::PointingHand)
}

fn ghost_small_button_painted(
    ui: &mut egui::Ui,
    text: &str,
    text_color: Color32,
    hover_fill: Color32,
) -> egui::Response {
    let button = egui::Button::new(
        RichText::new(text.to_string()).size(11.0).color(text_color),
    )
    .fill(Color32::TRANSPARENT)
    .stroke(Stroke::NONE)
    .corner_radius(CornerRadius::same(6));
    let response = ui.add(button);
    if response.hovered() {
        ui.painter()
            .rect_filled(response.rect, CornerRadius::same(6), hover_fill);
    }
    response.on_hover_cursor(CursorIcon::PointingHand)
}

fn toggle_switch(ui: &mut egui::Ui, on: &mut bool) -> egui::Response {
    let size = Vec2::new(36.0, 19.0);
    let (rect, _) = ui.allocate_exact_size(size, Sense::hover());
    let id = ui.id().with(("toggle", rect.left() as u64, rect.top() as u64));
    let response = ui.interact(rect, id, Sense::click());
    let response = response.on_hover_cursor(CursorIcon::PointingHand);
    if response.clicked() {
        *on = !*on;
    }
    let t = ui
        .ctx()
        .animate_value_with_time(id, if *on { 1.0 } else { 0.0 }, 0.14);

    let off_bg = Color32::from_rgba_unmultiplied(255, 255, 255, 34);
    let bg = lerp_color(off_bg, ACCENT, t);
    ui.painter().rect_filled(rect, CornerRadius::same(10), bg);
    let knob_r = size.y / 2.0 - 3.0;
    let knob_x = rect.left() + 3.0 + knob_r + t * (size.x - 6.0 - knob_r * 2.0);
    ui.painter()
        .circle_filled(Pos2::new(knob_x, rect.center().y), knob_r + 0.5, Color32::WHITE);
    response
}

fn lerp_color(a: Color32, b: Color32, t: f32) -> Color32 {
    let t = t.clamp(0.0, 1.0);
    Color32::from_rgba_unmultiplied(
        (a.r() as f32 + (b.r() as f32 - a.r() as f32) * t) as u8,
        (a.g() as f32 + (b.g() as f32 - a.g() as f32) * t) as u8,
        (a.b() as f32 + (b.b() as f32 - a.b() as f32) * t) as u8,
        (a.a() as f32 + (b.a() as f32 - a.a() as f32) * t) as u8,
    )
}

fn section_title(ui: &mut egui::Ui, text: &str) {
    ui.add_space(4.0);
    ui.label(
        RichText::new(text)
            .strong()
            .size(12.5)
            .color(TEXT_SECONDARY),
    );
    ui.add_space(2.0);
}

/// WinUI 风格的设置行卡片
fn settings_row(ui: &mut egui::Ui, row_height: f32, content: impl FnOnce(&mut egui::Ui)) {
    Frame::new()
        .fill(CARD_BG)
        .corner_radius(CornerRadius::same(10))
        .inner_margin(Margin::symmetric(14, 9))
        .show(ui, |ui| {
            // 行内容限制在固定高度内，避免右侧控件被垂直"居中"到剩余空间
            let row_rect = Rect::from_min_size(
                ui.cursor().left_top(),
                Vec2::new(ui.available_width(), row_height),
            );
            let mut row_ui = ui.new_child(
                egui::UiBuilder::new()
                    .max_rect(row_rect)
                    .layout(egui::Layout::left_to_right(Align::Center)),
            );
            content(&mut row_ui);
            ui.advance_cursor_after_rect(row_rect);
        });
    ui.add_space(5.0);
}

fn row_texts(ui: &mut egui::Ui, title: &str, desc: &str) {
    ui.vertical(|ui| {
        ui.label(RichText::new(title).size(13.0).color(TEXT_PRIMARY));
        if !desc.is_empty() {
            ui.label(RichText::new(desc).size(11.0).color(TEXT_SECONDARY));
        }
    });
}

// ---------- 设置窗口 ----------

fn window_frame() -> Frame {
    Frame::new()
        .fill(WINDOW_BG)
        .stroke(Stroke::new(1.0, PANEL_STROKE))
        .corner_radius(CornerRadius::same(14))
        .inner_margin(Margin::same(16))
        .shadow(SHADOW)
}

fn settings_window(ctx: &egui::Context, app: &mut DeskmanApp) {
    let mut open = app.settings_open;
    let max_content_height = (ctx.screen_rect().height() - 150.0).clamp(280.0, 640.0);
    let settings_window_response = egui::Window::new(RichText::new("设 置").strong().size(14.0))
        .open(&mut open)
        .collapsible(false)
        .resizable(false)
        .default_width(500.0)
        .default_pos(Pos2::new(160.0, 40.0))
        .frame(window_frame())
        .show(ctx, |ui| {
            ScrollArea::vertical()
                .max_height(max_content_height)
                .show(ui, |ui| {
                    settings_window_body(ui, app);
                });
        });
    app.settings_open = open;
    if let Some(win) = settings_window_response {
        app.interactive_rects.push(win.response.rect);
    }
}

fn settings_window_body(ui: &mut egui::Ui, app: &mut DeskmanApp) {
    section_title(ui, "扫 描");
    settings_row(ui, 40.0, |ui| {
        row_texts(ui, "扫描用户桌面", "导入当前用户的桌面项目");
        ui.with_layout(egui::Layout::right_to_left(Align::Center), |ui| {
            toggle_switch(ui, &mut app.config.scan.include_user_desktop);
        });
    });
    settings_row(ui, 40.0, |ui| {
        row_texts(ui, "扫描公共桌面", "导入所有用户的公共桌面项目");
        ui.with_layout(egui::Layout::right_to_left(Align::Center), |ui| {
            toggle_switch(ui, &mut app.config.scan.include_public_desktop);
        });
    });
    settings_row(ui, 40.0, |ui| {
        row_texts(ui, "启动时显示未分类", "启动时自动扫描并导入未分类项目");
        ui.with_layout(egui::Layout::right_to_left(Align::Center), |ui| {
            toggle_switch(ui, &mut app.config.scan.show_unclassified);
        });
    });
    settings_row(ui, 34.0, |ui| {
        if pill_button(ui, "立即扫描桌面", ButtonStyle::Primary).clicked() {
            app.scan_desktop();
        }
    });

    section_title(ui, "面 板");
    settings_row(ui, 34.0, |ui| {
        row_texts(ui, "面板样式", "深色玻璃 / iOS 亮色 / iOS 暗色");
        ui.with_layout(egui::Layout::right_to_left(Align::Center), |ui| {
            let style = app.config.desktop.style.clone();
            for (id, label) in [
                ("dark-glass", "深色玻璃"),
                ("ios-light", "iOS 亮色"),
                ("ios-dark", "iOS 暗色"),
            ] {
                let selected = style == id;
                let button = egui::Button::new(
                    RichText::new(label)
                        .size(11.5)
                        .color(if selected { Color32::WHITE } else { TEXT_SECONDARY }),
                )
                .fill(if selected {
                    ACCENT_DIM
                } else {
                    Color32::from_rgba_unmultiplied(255, 255, 255, 18)
                })
                .corner_radius(CornerRadius::same(7))
                .min_size(Vec2::new(0.0, 22.0));
                if ui.add(button).clicked() {
                    app.config.desktop.style = id.to_string();
                    app.save_config();
                    app.set_status(format!("面板样式：{}", label));
                }
            }
        });
    });
    settings_row(ui, 40.0, |ui| {
        row_texts(ui, "显示图标名称", "图标下方显示文件名称");
        ui.with_layout(egui::Layout::right_to_left(Align::Center), |ui| {
            toggle_switch(ui, &mut app.config.desktop.show_labels);
        });
    });
    settings_row(ui, 40.0, |ui| {
        row_texts(ui, "面板不透明度", "调整桌面面板的透明程度");
        ui.with_layout(egui::Layout::right_to_left(Align::Center), |ui| {
            ui.add_sized(
                [130.0, 18.0],
                egui::Slider::new(&mut app.config.desktop.opacity, 0.35..=1.0).show_value(false),
            );
        });
    });
    settings_row(ui, 40.0, |ui| {
        row_texts(ui, "显示真实图标", "提取文件和快捷方式的原始图标");
        ui.with_layout(egui::Layout::right_to_left(Align::Center), |ui| {
            let before = app.config.desktop.real_icons;
            toggle_switch(ui, &mut app.config.desktop.real_icons);
            if before && !app.config.desktop.real_icons {
                app.icon_textures.clear();
            }
        });
    });
    settings_row(ui, 40.0, |ui| {
        row_texts(ui, "全屏工作区", "面板可摆放到屏幕任意位置");
        ui.with_layout(egui::Layout::right_to_left(Align::Center), |ui| {
            toggle_switch(ui, &mut app.config.desktop.fullscreen);
        });
    });
    settings_row(ui, 40.0, |ui| {
        row_texts(ui, "桌面图标显示", "切换系统桌面图标的显示/隐藏");
        ui.with_layout(egui::Layout::right_to_left(Align::Center), |ui| {
            if pill_button(ui, "切换显示", ButtonStyle::Ghost).clicked() {
                match desktop::toggle_desktop_icons() {
                    Ok(()) => app.set_status("已切换桌面图标显示"),
                    Err(e) => app.set_status(format!("切换失败：{}", e)),
                }
            }
        });
    });
    settings_row(ui, 40.0, |ui| {
        row_texts(ui, "面板置顶", "开启后面板显示在其他窗口之上");
        ui.with_layout(egui::Layout::right_to_left(Align::Center), |ui| {
            toggle_switch(ui, &mut app.config.desktop.always_on_top);
        });
    });
    settings_row(ui, 40.0, |ui| {
        row_texts(ui, "工作区大小", "面板可摆放的桌面区域尺寸");
        ui.with_layout(egui::Layout::right_to_left(Align::Center), |ui| {
            ui.add(
                egui::DragValue::new(&mut app.config.desktop.width)
                    .range(640.0..=2560.0)
                    .suffix(" 宽"),
            );
            ui.add(
                egui::DragValue::new(&mut app.config.desktop.height)
                    .range(400.0..=1600.0)
                    .suffix(" 高"),
            );
        });
    });

    section_title(ui, "Dock 滑轨");
    settings_row(ui, 40.0, |ui| {
        row_texts(ui, "启用 Dock", "底部横置图标滑轨（右键面板图标可添加）");
        ui.with_layout(egui::Layout::right_to_left(Align::Center), |ui| {
            toggle_switch(ui, &mut app.config.dock.enabled);
        });
    });
    settings_row(ui, 40.0, |ui| {
        row_texts(ui, "回环模式", "环形轮盘布局，无限环绕滚动");
        ui.with_layout(egui::Layout::right_to_left(Align::Center), |ui| {
            toggle_switch(ui, &mut app.config.dock.loop_mode);
        });
    });
    if app.config.dock.enabled {
        settings_row(ui, 40.0, |ui| {
            row_texts(ui, "图标大小", format!("{:.0} px", app.config.dock.icon_size).as_str());
            ui.with_layout(egui::Layout::right_to_left(Align::Center), |ui| {
                ui.add_sized(
                    [130.0, 18.0],
                    egui::Slider::new(&mut app.config.dock.icon_size, 32.0..=96.0).show_value(false),
                );
            });
        });
        settings_row(ui, 40.0, |ui| {
            row_texts(ui, "悬停放大倍数", format!("{:.1}x", app.config.dock.magnify).as_str());
            ui.with_layout(egui::Layout::right_to_left(Align::Center), |ui| {
                ui.add_sized([130.0, 18.0], egui::Slider::new(&mut app.config.dock.magnify, 1.0..=2.5));
            });
        });
        settings_row(ui, 40.0, |ui| {
            row_texts(ui, "底板不透明度", "0 为完全透明，仅显示图标");
            ui.with_layout(egui::Layout::right_to_left(Align::Center), |ui| {
                ui.add_sized(
                    [130.0, 18.0],
                    egui::Slider::new(&mut app.config.dock.opacity, 0.0..=1.0).show_value(false),
                );
            });
        });
        settings_row(ui, 40.0, |ui| {
            row_texts(ui, "底板圆角", format!("{:.0} px", app.config.dock.corner_radius).as_str());
            ui.with_layout(egui::Layout::right_to_left(Align::Center), |ui| {
                ui.add_sized(
                    [130.0, 18.0],
                    egui::Slider::new(&mut app.config.dock.corner_radius, 0.0..=32.0).show_value(false),
                );
            });
        });
        settings_row(ui, 40.0, |ui| {
            row_texts(ui, "图标倒影", "图标下方的镜像渐隐效果");
            ui.with_layout(egui::Layout::right_to_left(Align::Center), |ui| {
                toggle_switch(ui, &mut app.config.dock.reflection);
            });
        });
        settings_row(ui, 34.0, |ui| {
            ui.label(
                RichText::new(format!("Dock 内 {} 个图标，右键面板图标可添加", app.config.dock.items.len()))
                    .size(11.0)
                    .color(TEXT_SECONDARY),
            );
            ui.with_layout(egui::Layout::right_to_left(Align::Center), |ui| {
                if pill_button(ui, "导入全部", ButtonStyle::Primary).clicked() {
                    let added = fill_dock_from_desktop(app);
                    app.set_status(format!("已导入 {} 个快捷方式到 Dock", added));
                }
                if pill_button(ui, "清空", ButtonStyle::DangerGhost).clicked() {
                    app.config.dock.items.clear();
                    app.save_config();
                    app.set_status("Dock 已清空");
                }
            });
        });
    }

    section_title(ui, "分 类");
    settings_row(ui, 34.0, |ui| {
        ui.add_sized(
            [170.0, 24.0],
            egui::TextEdit::singleline(&mut app.new_group_name).hint_text("新分类名称"),
        );
        if pill_button(ui, "新建", ButtonStyle::Primary).clicked() {
            add_group(app);
        }
    });
    let group_count = app.config.groups.len();
    for index in 0..group_count {
        let (name, collapsed, item_count) = {
            let g = &app.config.groups[index];
            (g.name.clone(), g.collapsed, g.items.len())
        };
        settings_row(ui, 34.0, |ui| {
            ui.label(RichText::new(name.clone()).size(12.5).color(TEXT_PRIMARY));
            ui.label(
                RichText::new(format!("{} 项", item_count))
                    .size(11.0)
                    .color(TEXT_SECONDARY),
            );
            ui.with_layout(egui::Layout::right_to_left(Align::Center), |ui| {
                if ghost_small_button_painted(
                    ui,
                    if collapsed { "展开" } else { "收起" }, TEXT_SECONDARY, Color32::from_rgba_unmultiplied(255, 255, 255, 24),
                )
                .clicked()
                {
                    app.config.groups[index].collapsed = !app.config.groups[index].collapsed;
                    app.save_config();
                }
                if ghost_small_button_painted(ui, "设置", TEXT_SECONDARY, Color32::from_rgba_unmultiplied(255, 255, 255, 24)).clicked() {
                    app.editing_group = Some(index);
                }
            });
        });
    }

    ui.add_space(6.0);
    ui.separator();
    ui.horizontal(|ui| {
        ui.label(
            RichText::new(format!(
                "deskman v{} · Tao_Tools 工具链",
                env!("CARGO_PKG_VERSION")
            ))
            .size(11.0)
            .color(TEXT_SECONDARY.linear_multiply(0.7)),
        );
        ui.with_layout(egui::Layout::right_to_left(Align::Center), |ui| {
            if pill_button(ui, "保存设置", ButtonStyle::Primary).clicked() {
                app.save_config();
                app.set_status("设置已保存");
            }
        });
    });
}

fn edit_group_window(ctx: &egui::Context, app: &mut DeskmanApp, index: usize) {
    if index >= app.config.groups.len() {
        app.editing_group = None;
        return;
    }

    let mut open = true;
    let max_edit_height = (ctx.screen_rect().height() - 150.0).clamp(280.0, 600.0);
    // 面板尺寸边界：最小恰好容纳一个图标，最大铺满屏幕
    let min_panel = app.effective_icon_size(&app.config.groups[index]) + 44.0;
    let max_w = ctx.screen_rect().width().max(200.0);
    let max_h = ctx.screen_rect().height().max(200.0);
    let edit_window_response = egui::Window::new(RichText::new("面板设置").strong().size(14.0))
        .open(&mut open)
        .collapsible(false)
        .resizable(false)
        .default_width(430.0)
        .default_pos(Pos2::new(240.0, 70.0))
        .frame(window_frame())
        .show(ctx, |ui| {
            ScrollArea::vertical()
                .max_height(max_edit_height)
                .show(ui, |ui| {
                    settings_row(ui, 34.0, |ui| {
                        let name_edit = egui::TextEdit::singleline(&mut app.config.groups[index].name)
                            .desired_width(ui.available_width() - 20.0);
                        ui.add(name_edit);
                    });

            settings_row(ui, 34.0, |ui| {
                row_texts(ui, "面板宽度", "");
                ui.with_layout(egui::Layout::right_to_left(Align::Center), |ui| {
                    ui.add_sized(
                        [110.0, 18.0],
                        egui::Slider::new(&mut app.config.groups[index].width, min_panel..=max_w)
                            .show_value(false),
                    );
                });
            });
            settings_row(ui, 34.0, |ui| {
                row_texts(ui, "面板高度", "");
                ui.with_layout(egui::Layout::right_to_left(Align::Center), |ui| {
                    ui.add_sized(
                        [110.0, 18.0],
                        egui::Slider::new(&mut app.config.groups[index].height, min_panel..=max_h)
                            .show_value(false),
                    );
                });
            });
            settings_row(ui, 34.0, |ui| {
                row_texts(ui, "图标尺寸", "");
                ui.with_layout(egui::Layout::right_to_left(Align::Center), |ui| {
                    ui.add_sized(
                        [110.0, 18.0],
                        egui::Slider::new(&mut app.config.groups[index].icon_size, 36.0..=88.0)
                            .show_value(false),
                    );
                });
            });

            settings_row(ui, 34.0, |ui| {
                row_texts(ui, "排列方式", "");
                ui.with_layout(egui::Layout::right_to_left(Align::Center), |ui| {
                    segmented_layout(ui, app, index);
                });
            });

            settings_row(ui, 34.0, |ui| {
                row_texts(ui, "锁定位置", "编辑模式下不可拖动该面板");
                ui.with_layout(egui::Layout::right_to_left(Align::Center), |ui| {
                    toggle_switch(ui, &mut app.config.groups[index].position_locked);
                });
            });
            settings_row(ui, 34.0, |ui| {
                row_texts(ui, "锁定尺寸", "不可调整宽高和图标大小");
                ui.with_layout(egui::Layout::right_to_left(Align::Center), |ui| {
                    toggle_switch(ui, &mut app.config.groups[index].size_locked);
                });
            });

            ui.add_space(8.0);
            section_title(ui, "添加项目");
            settings_row(ui, 34.0, |ui| {
                ui.add_sized(
                    [100.0, 24.0],
                    egui::TextEdit::singleline(&mut app.new_item_name).hint_text("名称"),
                );
                let path_edit = egui::TextEdit::singleline(&mut app.new_item_path)
                    .hint_text("路径、网址或命令")
                    .desired_width(ui.available_width() - 10.0);
                ui.add(path_edit);
            });
            settings_row(ui, 34.0, |ui| {
                if pill_button(ui, "文件", ButtonStyle::Ghost).clicked() {
                    add_item(app, index, DesktopItemKind::File);
                }
                if pill_button(ui, "文件夹", ButtonStyle::Ghost).clicked() {
                    add_item(app, index, DesktopItemKind::Folder);
                }
                if pill_button(ui, "网页", ButtonStyle::Ghost).clicked() {
                    add_item(app, index, DesktopItemKind::Url);
                }
                if pill_button(ui, "命令", ButtonStyle::Ghost).clicked() {
                    add_item(app, index, DesktopItemKind::Command);
                }
            });

            ui.add_space(8.0);
            section_title(
                ui,
                format!("已有项目（{}）", app.config.groups[index].items.len()).as_str(),
            );
            let mut remove_index = None;
            ScrollArea::vertical().max_height(150.0).show(ui, |ui| {
                for (item_index, item) in app.config.groups[index].items.iter_mut().enumerate() {
                    settings_row(ui, 30.0, |ui| {
                        ui.label(
                            RichText::new(format!("{}", item_index + 1))
                                .size(11.0)
                                .color(TEXT_SECONDARY),
                        );
                        let name_edit = egui::TextEdit::singleline(&mut item.name)
                            .desired_width(ui.available_width() - 56.0);
                        ui.add(name_edit);
                        ui.with_layout(egui::Layout::right_to_left(Align::Center), |ui| {
                            if ghost_small_button_painted(ui, "删除", TEXT_SECONDARY, Color32::from_rgba_unmultiplied(255, 255, 255, 24)).clicked() {
                                remove_index = Some(item_index);
                            }
                        });
                    });
                }
            });
            if let Some(item_index) = remove_index {
                app.config.groups[index].items.remove(item_index);
                app.save_config();
            }

            ui.add_space(6.0);
            ui.separator();
            ui.horizontal(|ui| {
                let confirming = match &app.delete_confirm {
                    Some((i, at)) => *i == index && at.elapsed().as_secs_f32() < 3.0,
                    None => false,
                };
                if pill_button(
                    ui,
                    if confirming { "确认删除？" } else { "删除分类" },
                    ButtonStyle::DangerGhost,
                )
                .clicked()
                {
                    if confirming {
                        app.config.groups.remove(index);
                        app.delete_confirm = None;
                        app.editing_group = None;
                        app.save_config();
                        app.set_status("分类已删除");
                        return;
                    }
                    app.delete_confirm = Some((index, Instant::now()));
                }
                ui.with_layout(egui::Layout::right_to_left(Align::Center), |ui| {
                    if pill_button(ui, "保存并关闭", ButtonStyle::Primary).clicked() {
                        app.save_config();
                        app.editing_group = None;
                    }
                });
            });
                });
        });

    if !open {
        app.editing_group = None;
        app.delete_confirm = None;
    }
    if let Some(win) = edit_window_response {
        app.interactive_rects.push(win.response.rect);
    }
}

fn segmented_layout(ui: &mut egui::Ui, app: &mut DeskmanApp, index: usize) {
    let options = [
        (GroupLayout::Free, "自由"),
        (GroupLayout::Grid, "网格"),
        (GroupLayout::Horizontal, "横向"),
    ];
    let current = app.config.groups[index].layout.clone();
    for (layout, label) in options {
        let selected = current == layout;
        let text_color = if selected {
            Color32::WHITE
        } else {
            TEXT_SECONDARY
        };
        let button = egui::Button::new(RichText::new(label).size(11.5).color(text_color))
            .fill(if selected {
                ACCENT_DIM
            } else {
                Color32::from_rgba_unmultiplied(255, 255, 255, 18)
            })
            .corner_radius(CornerRadius::same(7))
            .min_size(Vec2::new(44.0, 22.0));
        if ui.add(button).clicked() {
            app.config.groups[index].layout = layout;
            app.save_config();
        }
    }
}

// ---------- 数据操作 ----------

fn add_item(app: &mut DeskmanApp, group_index: usize, kind: DesktopItemKind) {
    let name = app.new_item_name.trim();
    let path = app.new_item_path.trim();
    if name.is_empty() || path.is_empty() {
        app.set_status("名称和路径不能为空");
        return;
    }
    let next_index = app.config.groups[group_index].items.len() + 1;
    app.config.groups[group_index].items.push(DesktopItem {
        id: format!("manual-{}", next_index),
        name: name.to_string(),
        path: path.to_string(),
        kind,
        args: Vec::new(),
        pinned: false,
        x: 0.0,
        y: 0.0,
    });
    desktop::place_item_in_group(&mut app.config.groups[group_index]);
    app.new_item_name.clear();
    app.new_item_path.clear();
    app.save_config();
    app.set_status("已添加项目");
}

fn add_group(app: &mut DeskmanApp) {
    let name = app.new_group_name.trim();
    if name.is_empty() {
        app.set_status("分类名称不能为空");
        return;
    }
    let index = app.config.groups.len();
    app.config.groups.push(crate::config::DesktopGroup {
        id: format!("group-{}", index + 1),
        name: name.to_string(),
        x: 28.0 + (index as f32 * 32.0),
        y: 96.0 + (index as f32 * 24.0),
        width: 300.0,
        height: 220.0,
        collapsed: false,
        position_locked: false,
        size_locked: false,
        layout: GroupLayout::Free,
        icon_size: 48.0,
        items: Vec::new(),
    });
    app.new_group_name.clear();
    app.save_config();
    app.set_status("已新建分类");
}

/// 把桌面扫描到的全部图标导入 Dock（按名称排序，按路径去重）
fn fill_dock_from_desktop(app: &mut DeskmanApp) -> usize {
    let scanned = desktop::scan_desktop_items(&app.config).unwrap_or_default();
    let mut existing: std::collections::HashSet<String> = app
        .config
        .dock
        .items
        .iter()
        .map(|i| i.path.to_lowercase())
        .collect();
    let mut items: Vec<DesktopItem> = scanned
        .into_iter()
        .filter(|i| existing.insert(i.path.to_lowercase()))
        .collect();
    items.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
    let mut added = 0;
    for mut item in items {
        let seq = app.config.dock.items.len() + 1;
        item.id = format!("dock-auto-{}", seq);
        item.x = 0.0;
        item.y = 0.0;
        app.config.dock.items.push(item);
        added += 1;
    }
    if added > 0 {
        app.config.save();
    }
    added
}

/// 接管桌面：整体隐藏系统桌面图标层（退出时恢复，位置零漂移）
fn apply_takeover(state: &mut DesktopState, _config: &AppConfig) -> usize {
    if !state.view_toggled && desktop_list::toggle_view().is_ok() {
        state.view_toggled = true;
        state.save();
    }
    0
}

/// 恢复桌面图标显示（视图开关是系统原语，位置天然不丢）
fn restore_taken(state: &mut DesktopState) {
    if state.view_toggled {
        let _ = desktop_list::toggle_view();
        state.view_toggled = false;
        state.save();
    }
}

/// 退出前的兜底还原
fn exit_restore(state: &mut DesktopState) {
    if !state.taken.is_empty() || state.view_toggled {
        restore_taken(state);
    }
}

/// 切换系统桌面图标显示（记录奇偶，退出时恢复）
fn toggle_desktop_view(app: &mut DeskmanApp) {
    match desktop_list::toggle_view() {
        Ok(()) => {
            app.state.view_toggled = !app.state.view_toggled;
            app.state.save();
            app.set_status(if app.state.view_toggled {
                "已隐藏桌面图标（面板保留）"
            } else {
                "已显示桌面图标"
            });
        }
        Err(e) => app.set_status(format!("切换失败：{}", e)),
    }
}



// ---------- 点击穿透：面板外区域放行鼠标 ----------

#[cfg(windows)]
mod click_through {
    use std::sync::atomic::{AtomicIsize, Ordering};
    use windows_sys::Win32::Foundation::{HWND, POINT};
    use windows_sys::Win32::Graphics::Gdi::ScreenToClient;
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        FindWindowW, GetCursorPos, GetWindowLongPtrW, SetLayeredWindowAttributes,
        SetWindowLongPtrW, GWL_EXSTYLE, LWA_ALPHA, WS_EX_LAYERED, WS_EX_TRANSPARENT,
    };

    static CACHED_HWND: AtomicIsize = AtomicIsize::new(0);

    pub fn current_hwnd() -> isize {
        window_handle()
    }

    fn window_handle() -> isize {
        let cached = CACHED_HWND.load(Ordering::Relaxed);
        if cached != 0 {
            return cached;
        }
        let title: Vec<u16> = "Deskman\0".encode_utf16().collect();
        let hwnd = unsafe { FindWindowW(std::ptr::null(), title.as_ptr()) };
        if hwnd.is_null() {
            0
        } else {
            let value = hwnd as isize;
            CACHED_HWND.store(value, Ordering::Relaxed);
            value
        }
    }

    pub fn set_pass_through(pass_through: bool) {
        let hwnd = window_handle();
        if hwnd == 0 {
            return;
        }
        let hwnd = hwnd as HWND;
        unsafe {
            let style = GetWindowLongPtrW(hwnd, GWL_EXSTYLE);
            // LAYERED+TRANSPARENT 是鼠标穿透的标准组合；配合 clear_color 全透明不会产生蒙层
            let new_style = if pass_through {
                style | WS_EX_TRANSPARENT as isize | WS_EX_LAYERED as isize
            } else {
                style & !(WS_EX_TRANSPARENT as isize)
            };
            if new_style != style {
                SetWindowLongPtrW(hwnd, GWL_EXSTYLE, new_style);
                SetLayeredWindowAttributes(hwnd, 0, 255, LWA_ALPHA);
            }
        }
    }

    /// 鼠标在窗口客户区中的物理像素坐标
    pub fn cursor_client_pos() -> Option<(i32, i32)> {
        let hwnd = window_handle();
        if hwnd == 0 {
            return None;
        }
        let hwnd = hwnd as HWND;
        let mut pt = POINT { x: 0, y: 0 };
        unsafe {
            if GetCursorPos(&mut pt) == 0 {
                return None;
            }
            if ScreenToClient(hwnd, &mut pt) == 0 {
                return None;
            }
        }
        Some((pt.x, pt.y))
    }

    /// 窗口屏幕矩形（物理像素）：x, y, w, h
    #[allow(dead_code)]
    pub fn window_rect_physical() -> Option<(i32, i32, i32, i32)> {
        use windows_sys::Win32::UI::WindowsAndMessaging::GetWindowRect;
        use windows_sys::Win32::Foundation::RECT;
        let hwnd = window_handle();
        if hwnd == 0 {
            return None;
        }
        let mut rect = RECT {
            left: 0,
            top: 0,
            right: 0,
            bottom: 0,
        };
        if unsafe { GetWindowRect(hwnd as HWND, &mut rect) } == 0 {
            return None;
        }
        Some((
            rect.left,
            rect.top,
            rect.right - rect.left,
            rect.bottom - rect.top,
        ))
    }

    /// 用 DWM AccentPolicy 启用真透明（去掉 winit with_transparent 带来的暗色玻璃蒙层）
    pub fn enable_true_transparency() {
        use windows_sys::Win32::Graphics::Dwm::DwmSetWindowAttribute;

        #[repr(C)]
        struct AccentPolicy {
            accent_state: u32,
            accent_flags: u32,
            gradient_color: u32,
            animation_id: u32,
        }
        let hwnd = window_handle();
        if hwnd == 0 {
            return;
        }
        let accent = AccentPolicy {
            accent_state: 2, // ACCENT_ENABLE_TRANSPARENTGRADIENT
            accent_flags: 2,
            gradient_color: 0,
            animation_id: 0,
        };
        unsafe {
            DwmSetWindowAttribute(
                hwnd as HWND,
                19, // DWMWA_ACCENT_POLICY
                &accent as *const AccentPolicy as *const _,
                std::mem::size_of::<AccentPolicy>() as u32,
            );
        }
    }

    /// 主显示器工作区（去掉任务栏）的物理像素尺寸
    pub fn work_area_physical_size() -> Option<(f32, f32)> {
        use windows_sys::Win32::Foundation::RECT;
        use windows_sys::Win32::UI::WindowsAndMessaging::{
            SystemParametersInfoW, SPI_GETWORKAREA,
        };
        let mut rect = RECT {
            left: 0,
            top: 0,
            right: 0,
            bottom: 0,
        };
        let ok = unsafe {
            SystemParametersInfoW(
                SPI_GETWORKAREA,
                0,
                &mut rect as *mut RECT as *mut _,
                0,
            )
        };
        if ok != 0 {
            Some(((rect.right - rect.left) as f32, (rect.bottom - rect.top) as f32))
        } else {
            None
        }
    }

    /// 主显示器工作区的 egui 逻辑尺寸
    pub fn work_area_logical_size(ctx: &egui::Context) -> Option<egui::Vec2> {
        let (w, h) = work_area_physical_size()?;
        let scale = ctx.pixels_per_point();
        Some(egui::Vec2::new(w / scale, h / scale))
    }
}

#[cfg(not(windows))]
mod click_through {
    pub fn set_pass_through(_pass_through: bool) {}
    pub fn cursor_client_pos() -> Option<(i32, i32)> {
        None
    }
    pub fn work_area_logical_size(_ctx: &egui::Context) -> Option<egui::Vec2> {
        None
    }
    pub fn window_rect_physical() -> Option<(i32, i32, i32, i32)> {
        None
    }
    pub fn enable_true_transparency() {}
    pub fn current_hwnd() -> isize {
        0
    }
}

/// 指针是否悬停在任意弹层上（右键菜单/子菜单/工具提示/设置窗口等 Foreground 区域）
fn pointer_over_popup(ctx: &egui::Context, p: Pos2) -> bool {
    ctx.memory(|m| {
        for layer_id in m.areas().visible_layer_ids() {
            if !matches!(
                layer_id.order,
                egui::Order::Foreground | egui::Order::Tooltip
            ) {
                continue;
            }
            if m.area_rect(layer_id.id).is_some_and(|r| r.contains(p)) {
                return true;
            }
        }
        false
    })
}

fn update_click_through(ctx: &egui::Context, app: &mut DeskmanApp) {
    let Some((cx, cy)) = click_through::cursor_client_pos() else {
        return;
    };
    let scale = ctx.pixels_per_point();
    let p = Pos2::new(cx as f32 / scale, cy as f32 / scale);
    let over_panel = app
        .interactive_rects
        .iter()
        .any(|r| r.expand(6.0).contains(p));
    let interacting = ctx.input(|i| i.pointer.any_down()) || pointer_over_popup(ctx, p);
    click_through::set_pass_through(!over_panel && !interacting);
}

// ---------- 全局样式 ----------

fn setup_style(ctx: &egui::Context) {
    let mut style = (*ctx.style()).clone();
    style.visuals = egui::Visuals::dark();
    style.visuals.panel_fill = Color32::TRANSPARENT;
    style.visuals.window_fill = WINDOW_BG;
    style.visuals.extreme_bg_color = Color32::from_rgb(16, 18, 24);
    style.visuals.window_stroke = Stroke::new(1.0, PANEL_STROKE);
    style.visuals.widgets.inactive.bg_fill = Color32::from_rgba_unmultiplied(255, 255, 255, 18);
    style.visuals.widgets.inactive.fg_stroke = Stroke::new(1.0, TEXT_PRIMARY);
    style.visuals.widgets.hovered.bg_fill = Color32::from_rgba_unmultiplied(255, 255, 255, 30);
    style.visuals.widgets.active.bg_fill = Color32::from_rgba_unmultiplied(255, 255, 255, 42);
    style.visuals.selection.bg_fill = ACCENT_DIM;
    style.visuals.selection.stroke = Stroke::new(1.0, ACCENT);
    style.visuals.widgets.inactive.corner_radius = CornerRadius::same(8);
    style.visuals.widgets.hovered.corner_radius = CornerRadius::same(8);
    style.visuals.widgets.active.corner_radius = CornerRadius::same(8);
    style.visuals.widgets.open.corner_radius = CornerRadius::same(8);
    style.visuals.widgets.noninteractive.corner_radius = CornerRadius::same(8);
    style.visuals.menu_corner_radius = CornerRadius::same(10);
    style.spacing.item_spacing = Vec2::new(8.0, 7.0);
    style.spacing.button_padding = Vec2::new(10.0, 5.0);
    ctx.set_style(style);
}

fn setup_chinese_fonts(ctx: &egui::Context) {
    use std::sync::Arc;
    let mut fonts = egui::FontDefinitions::default();
    let font_candidates = [
        ("C:/Windows/Fonts/msyh.ttc", "msyh"),
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
    ctx.set_fonts(fonts);
}
