// ======================== egui 图形界面 ========================
//
// 列表式工具中心：
//   · 每行展示图标、名称、版本、类型、说明与可执行文件解析结果
//   · “启动”运行工具，“设置”直达工具自带的设置界面
//   · ↑ / ↓ 调整显示顺序，顺序保存在 ~/.tao_tools/toolhub/config.json
//   · 右键条目可打开 README / 工具目录
//
// 界面只依赖 registry.json 的注册数据，新增工具无需改代码。

use crate::config::HubConfig;
use crate::launcher::{self, Target};
use crate::registry::{Registry, ToolEntry};
use eframe::egui::{
    self, Align, Align2, Button, Color32, CornerRadius, FontId, Frame, Layout, Margin, RichText,
    ScrollArea, Sense, Stroke, TextEdit, Ui, Vec2,
};
use std::time::Instant;

// ─── 配色 ──────────────────────────────────────────────────

const BG: Color32 = Color32::from_rgb(16, 18, 24);
const CARD: Color32 = Color32::from_rgb(25, 28, 36);
const STROKE: Color32 = Color32::from_rgb(42, 47, 59);
const STROKE_HOVER: Color32 = Color32::from_rgb(74, 96, 140);
const TEXT: Color32 = Color32::from_rgb(233, 236, 243);
const TEXT_DIM: Color32 = Color32::from_rgb(150, 158, 174);
const TEXT_FAINT: Color32 = Color32::from_rgb(112, 120, 136);
const ACCENT: Color32 = Color32::from_rgb(80, 146, 255);
const ACCENT_DIM: Color32 = Color32::from_rgb(44, 92, 176);
const BADGE_COLORS: [Color32; 6] = [
    Color32::from_rgb(66, 120, 232),
    Color32::from_rgb(62, 168, 128),
    Color32::from_rgb(214, 132, 62),
    Color32::from_rgb(148, 102, 216),
    Color32::from_rgb(210, 92, 124),
    Color32::from_rgb(70, 158, 198),
];

// ─── 启动 ──────────────────────────────────────────────────

pub fn run() {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([960.0, 700.0])
            .with_min_inner_size([720.0, 480.0])
            .with_title("Tao Tools 工具中心"),
        renderer: eframe::Renderer::Wgpu,
        ..Default::default()
    };

    let result = eframe::run_native(
        "Tao Tools 工具中心",
        options,
        Box::new(|_cc| Ok(Box::new(HubApp::new()))),
    );

    if let Err(e) = result {
        eprintln!("错误：GUI 启动失败：{}", e);
        std::process::exit(1);
    }
}

// ─── 应用状态 ──────────────────────────────────────────────

struct HubApp {
    registry: Registry,
    config: HubConfig,
    tools: Vec<ToolEntry>,
    filter: String,
    status: String,
    status_error: bool,
    status_at: Option<Instant>,
    style_initialized: bool,
}

enum Action {
    Launch(usize),
    Settings(usize),
    Move(usize, isize),
    Readme(usize),
    Folder(usize),
}

impl HubApp {
    fn new() -> Self {
        let registry = Registry::load();
        let config = HubConfig::load();
        let tools = config.apply_order(&registry.tools);
        Self {
            registry,
            config,
            tools,
            filter: String::new(),
            status: "就绪：双击工具直接启动，右键可打开 README 或目录".to_string(),
            status_error: false,
            status_at: None,
            style_initialized: false,
        }
    }

    fn reload(&mut self) {
        self.registry = Registry::load();
        self.config = HubConfig::load();
        self.tools = self.config.apply_order(&self.registry.tools);
        self.set_status(format!("已刷新注册表，共 {} 个工具", self.tools.len()));
    }

    fn set_status(&mut self, text: impl Into<String>) {
        self.status = text.into();
        self.status_error = false;
        self.status_at = Some(Instant::now());
    }

    fn set_error(&mut self, text: impl Into<String>) {
        self.status = text.into();
        self.status_error = true;
        self.status_at = Some(Instant::now());
    }

    fn handle(&mut self, action: Action) {
        match action {
            Action::Launch(index) => {
                let tool = self.tools[index].clone();
                match launcher::launch(&tool, &tool.launch, &self.config) {
                    Ok(message) => self.set_status(message),
                    Err(e) => self.set_error(e),
                }
            }
            Action::Settings(index) => {
                let tool = self.tools[index].clone();
                match launcher::launch(&tool, &tool.settings, &self.config) {
                    Ok(message) => self.set_status(format!("{} · 设置：{}", tool.id, message)),
                    Err(e) => self.set_error(e),
                }
            }
            Action::Move(index, delta) => {
                let target = index as isize + delta;
                if target < 0 || target >= self.tools.len() as isize {
                    return;
                }
                let target = target as usize;
                self.tools.swap(index, target);
                self.config.order = self.tools.iter().map(|tool| tool.id.clone()).collect();
                self.config.save();
                self.set_status(format!("已调整 {} 的显示顺序", self.tools[target].id));
            }
            Action::Readme(index) => {
                let tool = self.tools[index].clone();
                match tool.readme_path() {
                    Some(path) => match launcher::open_path(&path) {
                        Ok(()) => self.set_status(format!("已打开 {}/README.md", tool.id)),
                        Err(e) => self.set_error(e),
                    },
                    None => self.set_error(format!("{} 未注册 README 路径", tool.id)),
                }
            }
            Action::Folder(index) => {
                let tool = self.tools[index].clone();
                let path = match launcher::resolve(&tool, &self.config) {
                    Some(Target::Exe(path)) => path.parent().map(|p| p.to_path_buf()),
                    _ => crate::registry::workspace_root().map(|root| root.join(&tool.id)),
                };
                match path {
                    Some(path) => match launcher::open_folder(&path) {
                        Ok(()) => self.set_status(format!("已打开 {}", path.display())),
                        Err(e) => self.set_error(e),
                    },
                    None => self.set_error(format!("找不到 {} 的目录", tool.id)),
                }
            }
        }
    }

    fn open_registry(&mut self) {
        match self.registry.source.file_path() {
            Some(path) => match launcher::open_path(path) {
                Ok(()) => self.set_status("已打开注册表文件"),
                Err(e) => self.set_error(e),
            },
            None => self.set_error("当前使用内置注册表，没有可编辑的文件"),
        }
    }

    fn open_readme(&mut self) {
        let path = crate::registry::workspace_root()
            .map(|root| root.join("toolhub").join("README.md"));
        match path {
            Some(path) if path.is_file() => match launcher::open_path(&path) {
                Ok(()) => self.set_status("已打开 toolhub/README.md"),
                Err(e) => self.set_error(e),
            },
            _ => self.set_error("找不到 toolhub/README.md"),
        }
    }
}

impl eframe::App for HubApp {
    fn clear_color(&self, _visuals: &egui::Visuals) -> [f32; 4] {
        [BG.r() as f32 / 255.0, BG.g() as f32 / 255.0, BG.b() as f32 / 255.0, 1.0]
    }

    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        if !self.style_initialized {
            setup_style(ctx);
            setup_chinese_fonts(ctx);
            self.style_initialized = true;
        }

        self.header(ctx);

        egui::TopBottomPanel::bottom("status_bar")
            .frame(Frame::new().fill(BG).inner_margin(Margin::symmetric(20, 9)))
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.label(
                        RichText::new("↑ ↓ 调整顺序 · 顺序自动保存 · 右键条目打开 README / 目录")
                            .size(10.5)
                            .color(TEXT_FAINT),
                    );
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        let color = if self.status_error { Color32::from_rgb(235, 110, 110) } else { TEXT_DIM };
                        ui.add(egui::Label::new(RichText::new(&self.status).size(11.0).color(color)).truncate());
                    });
                });
            });

        let mut pending: Option<Action> = None;

        egui::CentralPanel::default()
            .frame(Frame::new().fill(BG).inner_margin(Margin::symmetric(20, 12)))
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.add(
                        TextEdit::singleline(&mut self.filter)
                            .hint_text("按名称 / 说明过滤工具…")
                            .desired_width(260.0)
                            .margin(Margin::symmetric(10, 6)),
                    );
                    if !self.filter.is_empty() && ui.button("清除").clicked() {
                        self.filter.clear();
                    }
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        let visible = self.visible_count();
                        ui.label(
                            RichText::new(format!("显示 {} / {} 个工具", visible, self.tools.len()))
                                .size(10.5)
                                .color(TEXT_FAINT),
                        );
                    });
                });
                ui.add_space(10.0);

                let entries: Vec<(usize, ToolEntry)> = self
                    .tools
                    .iter()
                    .enumerate()
                    .filter(|(_, tool)| matches_filter(tool, &self.filter))
                    .map(|(index, tool)| (index, tool.clone()))
                    .collect();

                if entries.is_empty() {
                    ui.add_space(40.0);
                    ui.vertical_centered(|ui| {
                        ui.label(RichText::new("没有匹配的工具").size(15.0).color(TEXT_DIM));
                        ui.add_space(6.0);
                        ui.label(
                            RichText::new("新工具必须注册到 toolhub/registry.json 才会出现在这里")
                                .size(11.5)
                                .color(TEXT_FAINT),
                        );
                    });
                    return;
                }

                ScrollArea::vertical()
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        for (index, tool) in &entries {
                            if let Some(action) =
                                self.tool_row(ui, tool, *index, entries.len())
                            {
                                pending = Some(action);
                            }
                            ui.add_space(8.0);
                        }
                    });
            });

        if let Some(action) = pending {
            self.handle(action);
        }
    }
}

impl HubApp {
    fn visible_count(&self) -> usize {
        self.tools
            .iter()
            .filter(|tool| matches_filter(tool, &self.filter))
            .count()
    }

    fn header(&mut self, ctx: &egui::Context) {
        let mut refresh = false;
        let mut registry = false;
        let mut readme = false;
        let mut quit = false;

        egui::TopBottomPanel::top("header")
            .frame(Frame::new().fill(BG).inner_margin(Margin::symmetric(20, 16)))
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    let (rect, _) = ui.allocate_exact_size(Vec2::splat(42.0), Sense::hover());
                    ui.painter().rect_filled(rect, CornerRadius::same(12), ACCENT_DIM);
                    ui.painter().text(
                        rect.center(),
                        Align2::CENTER_CENTER,
                        "T",
                        FontId::proportional(23.0),
                        Color32::WHITE,
                    );
                    ui.add_space(4.0);

                    ui.vertical(|ui| {
                        ui.label(RichText::new("Tao Tools 工具中心").size(19.0).strong().color(TEXT));
                        ui.add(
                            egui::Label::new(
                                RichText::new(format!(
                                    "已注册 {} 个工具 · {}",
                                    self.tools.len(),
                                    self.registry.source.label()
                                ))
                                .size(10.5)
                                .color(TEXT_FAINT),
                            )
                            .truncate(),
                        );
                    });

                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if ui.add(header_button("退出")).clicked() {
                            quit = true;
                        }
                        if ui.add(header_button("说明")).clicked() {
                            readme = true;
                        }
                        if ui.add(header_button("注册表")).clicked() {
                            registry = true;
                        }
                        if ui.add(header_button("刷新")).clicked() {
                            refresh = true;
                        }
                    });
                });
            });

        if refresh {
            self.reload();
        }
        if registry {
            self.open_registry();
        }
        if readme {
            self.open_readme();
        }
        if quit {
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
    }

    fn tool_row(
        &mut self,
        ui: &mut Ui,
        tool: &ToolEntry,
        index: usize,
        total_visible: usize,
    ) -> Option<Action> {
        let width = ui.available_width();
        let exe_hint = launcher::describe(tool, &self.config);
        let badge = badge_color(&tool.id);
        let initial = tool
            .id
            .chars()
            .next()
            .unwrap_or('?')
            .to_ascii_uppercase()
            .to_string();

        let mut action: Option<Action> = None;

        let frame = Frame::new()
            .fill(CARD)
            .stroke(Stroke::new(1.0, STROKE))
            .corner_radius(CornerRadius::same(12))
            .inner_margin(Margin::symmetric(14, 12));

        let inner = frame.show(ui, |ui| {
            ui.set_min_width(width - 30.0);
            ui.horizontal(|ui| {
                let (rect, _) = ui.allocate_exact_size(Vec2::splat(38.0), Sense::hover());
                ui.painter().rect_filled(rect, CornerRadius::same(10), badge);
                ui.painter().text(
                    rect.center(),
                    Align2::CENTER_CENTER,
                    &initial,
                    FontId::proportional(17.0),
                    Color32::WHITE,
                );
                ui.add_space(6.0);

                ui.vertical(|ui| {
                    ui.horizontal(|ui| {
                        ui.label(
                            RichText::new(tool.display_name())
                                .size(14.5)
                                .strong()
                                .color(TEXT),
                        );
                        ui.label(
                            RichText::new(format!("v{}", tool.version))
                                .size(10.0)
                                .color(TEXT_FAINT),
                        );
                        kind_tag(ui, &tool.kind);
                    });
                    ui.add_space(2.0);
                    ui.label(RichText::new(&tool.description).size(11.5).color(TEXT_DIM));
                    ui.add_space(1.0);
                    ui.add(
                        egui::Label::new(
                            RichText::new(format!("{} · {}", tool.id, exe_hint))
                                .size(10.0)
                                .color(TEXT_FAINT),
                        )
                        .truncate(),
                    );
                });

                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    let down = ui
                        .add_enabled(index + 1 < self.tools.len(), small_button("↓"))
                        .on_hover_text("下移");
                    if down.clicked() {
                        action = Some(Action::Move(index, 1));
                    }

                    let up = ui
                        .add_enabled(index > 0, small_button("↑"))
                        .on_hover_text("上移");
                    if up.clicked() {
                        action = Some(Action::Move(index, -1));
                    }

                    let settings = ui.add_enabled(
                        !tool.settings.is_empty(),
                        pill_button("设置", Color32::from_rgb(38, 43, 54), TEXT),
                    );
                    let settings = if tool.settings.is_empty() {
                        settings.on_disabled_hover_text("该工具未提供设置入口")
                    } else {
                        settings.on_hover_text("直接打开工具自带的设置界面")
                    };
                    if settings.clicked() {
                        action = Some(Action::Settings(index));
                    }

                    if ui
                        .add(pill_button("启动", ACCENT_DIM, Color32::WHITE))
                        .on_hover_text("启动工具")
                        .clicked()
                    {
                        action = Some(Action::Launch(index));
                    }
                });
            });
        });

        let response = inner.response.interact(Sense::click());
        if response.hovered() {
            ui.painter().rect_stroke(
                inner.response.rect,
                CornerRadius::same(12),
                Stroke::new(1.0, STROKE_HOVER),
                egui::StrokeKind::Inside,
            );
        }
        if response.double_clicked() {
            action = Some(Action::Launch(index));
        }

        response.context_menu(|ui| {
            ui.label(RichText::new(tool.id.as_str()).size(11.5).color(TEXT_FAINT));
            ui.separator();
            if ui.button("打开 README").clicked() {
                action = Some(Action::Readme(index));
                ui.close_menu();
            }
            if ui.button("打开工具目录").clicked() {
                action = Some(Action::Folder(index));
                ui.close_menu();
            }
            if ui.button("直接启动").clicked() {
                action = Some(Action::Launch(index));
                ui.close_menu();
            }
        });

        let _ = total_visible;
        action
    }
}

// ─── 小部件 ────────────────────────────────────────────────

fn header_button(text: &str) -> Button<'static> {
    Button::new(RichText::new(text.to_string()).size(12.0).color(TEXT))
        .fill(Color32::from_rgb(30, 34, 43))
        .corner_radius(CornerRadius::same(8))
        .min_size(Vec2::new(0.0, 28.0))
}

fn pill_button(text: &str, fill: Color32, color: Color32) -> Button<'static> {
    Button::new(RichText::new(text.to_string()).size(12.0).color(color))
        .fill(fill)
        .corner_radius(CornerRadius::same(8))
        .min_size(Vec2::new(56.0, 28.0))
}

fn small_button(text: &str) -> Button<'static> {
    Button::new(RichText::new(text.to_string()).size(12.0).color(TEXT_DIM))
        .fill(Color32::from_rgb(32, 36, 45))
        .corner_radius(CornerRadius::same(7))
        .min_size(Vec2::new(28.0, 28.0))
}

fn kind_tag(ui: &mut Ui, kind: &str) {
    let (label, color) = match kind {
        "gui" => ("GUI", Color32::from_rgb(58, 100, 178)),
        "tui" => ("TUI", Color32::from_rgb(160, 108, 54)),
        _ => ("CLI", Color32::from_rgb(58, 130, 104)),
    };
    let (rect, _) = ui.allocate_exact_size(Vec2::new(34.0, 16.0), Sense::hover());
    ui.painter().rect_filled(rect, CornerRadius::same(5), color);
    ui.painter().text(
        rect.center(),
        Align2::CENTER_CENTER,
        label,
        FontId::proportional(9.0),
        Color32::WHITE,
    );
}

fn matches_filter(tool: &ToolEntry, filter: &str) -> bool {
    let filter = filter.trim().to_lowercase();
    if filter.is_empty() {
        return true;
    }
    let haystack = format!(
        "{} {} {} {}",
        tool.id,
        tool.title,
        tool.description,
        tool.kind
    )
    .to_lowercase();
    haystack.contains(&filter)
}

fn badge_color(id: &str) -> Color32 {
    let sum = id.bytes().fold(0usize, |acc, byte| acc + byte as usize);
    BADGE_COLORS[sum % BADGE_COLORS.len()]
}

// ─── 主题与字体 ────────────────────────────────────────────

fn setup_style(ctx: &egui::Context) {
    let mut style = (*ctx.style()).clone();
    style.visuals = egui::Visuals::dark();
    style.visuals.panel_fill = BG;
    style.visuals.window_fill = CARD;
    style.visuals.extreme_bg_color = Color32::from_rgb(12, 14, 19);
    style.visuals.window_stroke = Stroke::new(1.0, STROKE);
    style.visuals.widgets.inactive.bg_fill = Color32::from_rgb(32, 36, 45);
    style.visuals.widgets.inactive.weak_bg_fill = Color32::from_rgb(32, 36, 45);
    style.visuals.widgets.inactive.fg_stroke = Stroke::new(1.0, TEXT);
    style.visuals.widgets.hovered.bg_fill = Color32::from_rgb(45, 52, 66);
    style.visuals.widgets.hovered.weak_bg_fill = Color32::from_rgb(45, 52, 66);
    style.visuals.widgets.hovered.fg_stroke = Stroke::new(1.0, Color32::WHITE);
    style.visuals.widgets.active.bg_fill = Color32::from_rgb(56, 64, 82);
    style.visuals.widgets.active.fg_stroke = Stroke::new(1.0, Color32::WHITE);
    style.visuals.selection.bg_fill = ACCENT_DIM;
    style.visuals.selection.stroke = Stroke::new(1.0, ACCENT);
    style.visuals.widgets.inactive.corner_radius = CornerRadius::same(8);
    style.visuals.widgets.hovered.corner_radius = CornerRadius::same(8);
    style.visuals.widgets.active.corner_radius = CornerRadius::same(8);
    style.visuals.widgets.open.corner_radius = CornerRadius::same(8);
    style.visuals.widgets.noninteractive.corner_radius = CornerRadius::same(8);
    style.visuals.menu_corner_radius = CornerRadius::same(10);
    style.spacing.item_spacing = Vec2::new(8.0, 6.0);
    style.spacing.button_padding = Vec2::new(10.0, 4.0);
    ctx.set_style(style);
}

fn setup_chinese_fonts(ctx: &egui::Context) {
    use std::sync::Arc;
    let mut fonts = egui::FontDefinitions::default();
    let candidates = [
        ("C:/Windows/Fonts/msyh.ttc", "msyh"),
        ("C:/Windows/Fonts/simsun.ttc", "simsun"),
        ("C:/Windows/Fonts/simhei.ttf", "simhei"),
    ];
    for (path, name) in candidates {
        if let Ok(data) = std::fs::read(path) {
            fonts
                .font_data
                .insert(name.to_string(), Arc::new(egui::FontData::from_owned(data)));
            if let Some(list) = fonts.families.get_mut(&egui::FontFamily::Proportional) {
                list.insert(0, name.to_string());
            }
            if let Some(list) = fonts.families.get_mut(&egui::FontFamily::Monospace) {
                list.insert(0, name.to_string());
            }
            break;
        }
    }
    ctx.set_fonts(fonts);
}
