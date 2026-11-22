// ======================== egui 图形界面 ========================
//
// 列表式工具中心：
//   · 每行展示图标、名称、版本、类型、说明与可执行文件解析结果
//   · “启动”运行工具，“设置”直达工具自带的设置界面
//   · ↑ / ↓ 调整显示顺序，顺序保存在 ~/.tao_tools/toolhub/config.json
//   · 右键条目可打开 README / 工具目录
//
// 界面只依赖 registry.json 的注册数据，新增工具无需改代码。
//
// 性能与安全约定：
//   · 可执行文件解析结果按 id 缓存（`ResolveCache`），行渲染不再每帧读盘；
//   · 自身 id 的行禁用「启动」，同一 id 已在运行只提示不重复拉起；
//   · 「结束」只针对本进程自己 spawn 的子进程 pid。

use crate::config::HubConfig;
use crate::launcher::{self, ResolveCache, RunningTools, Target};
use crate::registry::{Problem, Registry, ToolEntry};
use eframe::egui::{
    self, Align, Align2, Button, Color32, CornerRadius, FontId, Frame, Layout, Margin, RichText,
    ScrollArea, Sense, Stroke, TextEdit, Ui, Vec2,
};
use std::collections::HashSet;
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
const WARN: Color32 = Color32::from_rgb(226, 168, 82);
const RUNNING: Color32 = Color32::from_rgb(52, 110, 84);
const BADGE_COLORS: [Color32; 6] = [
    Color32::from_rgb(66, 120, 232),
    Color32::from_rgb(62, 168, 128),
    Color32::from_rgb(214, 132, 62),
    Color32::from_rgb(148, 102, 216),
    Color32::from_rgb(210, 92, 124),
    Color32::from_rgb(70, 158, 198),
];

/// 本工具自身的 id：禁止从列表里启动自己（避免递归打开工具中心）
/// 用包名而不是写死 "toolhub"，界面代码里仍然没有硬编码的工具名。
const SELF_ID: &str = env!("CARGO_PKG_NAME");

/// 点击「启动」前的拦截判断（纯逻辑，便于单测）
#[derive(Debug, PartialEq, Eq)]
enum LaunchGuard {
    /// 可以启动
    Ready,
    /// 是自己的行：硬禁用
    SelfTool,
    /// 同一 id 已在运行：只提示，不重复拉起
    AlreadyRunning(u32),
}

fn launch_guard(id: &str, running_pid: Option<u32>) -> LaunchGuard {
    if id == SELF_ID {
        return LaunchGuard::SelfTool;
    }
    if let Some(pid) = running_pid {
        return LaunchGuard::AlreadyRunning(pid);
    }
    LaunchGuard::Ready
}

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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum StatusKind {
    Info,
    Warning,
    Error,
}

struct HubApp {
    registry: Registry,
    config: HubConfig,
    tools: Vec<ToolEntry>,
    /// 可执行文件解析结果（id → Target），避免行渲染每帧读盘
    resolve_cache: ResolveCache,
    /// 本进程拉起的子进程，用于「运行中」提示与「结束」
    processes: RunningTools,
    filter: String,
    status: String,
    status_kind: StatusKind,
    status_at: Option<Instant>,
    style_initialized: bool,
}

enum Action {
    Launch(usize),
    Settings(usize),
    Stop(usize),
    Move(usize, isize),
    Readme(usize),
    Folder(usize),
}

impl HubApp {
    fn new() -> Self {
        let registry = Registry::load();
        let config = HubConfig::load();
        let tools = config.apply_order(&registry.tools);
        let mut app = Self {
            registry,
            config,
            tools,
            resolve_cache: ResolveCache::default(),
            processes: RunningTools::default(),
            filter: String::new(),
            status: String::new(),
            status_kind: StatusKind::Info,
            status_at: None,
            style_initialized: false,
        };
        app.announce_registry_state();
        app
    }

    fn reload(&mut self) {
        self.registry = Registry::load();
        self.config = HubConfig::load();
        self.tools = self.config.apply_order(&self.registry.tools);
        // config.paths / 注册表都可能变了，解析结果一律丢弃重算
        self.resolve_cache.clear();

        let count = self.tools.len();
        if self.registry.problems.is_empty() {
            self.set_status(format!("已刷新注册表，共 {} 个工具", count));
        } else {
            let problems = self.registry.problems.len();
            self.set_warning(format!(
                "已刷新注册表，共 {} 个工具；注册表有 {} 条问题（见列表顶部）",
                count, problems
            ));
        }
    }

    /// 启动时把注册表校验结果显示到状态栏（问题同时会在列表顶部与对应行标注）
    fn announce_registry_state(&mut self) {
        let Some(first) = self.registry.problems.first() else {
            self.set_status("就绪：双击工具直接启动，右键可打开 README 或目录");
            return;
        };
        let count = self.registry.problems.len();
        let first = first.display();
        let more = if count > 1 {
            format!("（另有 {} 条）", count - 1)
        } else {
            String::new()
        };
        self.set_warning(format!(
            "注册表有 {} 条问题：{}{}；其余工具仍可启动",
            count, first, more
        ));
    }

    fn set_status(&mut self, text: impl Into<String>) {
        self.status = text.into();
        self.status_kind = StatusKind::Info;
        self.status_at = Some(Instant::now());
    }

    fn set_warning(&mut self, text: impl Into<String>) {
        self.status = text.into();
        self.status_kind = StatusKind::Warning;
        self.status_at = Some(Instant::now());
    }

    fn set_error(&mut self, text: impl Into<String>) {
        self.status = text.into();
        self.status_kind = StatusKind::Error;
        self.status_at = Some(Instant::now());
    }

    fn status_color(&self) -> Color32 {
        match self.status_kind {
            StatusKind::Info => TEXT_DIM,
            StatusKind::Warning => WARN,
            StatusKind::Error => Color32::from_rgb(235, 110, 110),
        }
    }

    fn footer_hint(&self) -> String {
        let mut hint = String::from("↑ ↓ 调整顺序 · 顺序自动保存 · 右键条目打开 README / 目录");
        if !self.registry.problems.is_empty() {
            hint.push_str(&format!(
                " · 注册表 {} 条问题（见列表顶部）",
                self.registry.problems.len()
            ));
        }
        // 调试构建下显示解析次数：300 帧后应恒等于工具数，可用来验证缓存生效
        if cfg!(debug_assertions) {
            hint.push_str(&format!(
                " · 解析调用 {} 次（调试）",
                self.resolve_cache.resolve_calls()
            ));
        }
        hint
    }

    fn handle(&mut self, action: Action) {
        match action {
            Action::Launch(index) => self.launch_tool(index, false),
            Action::Settings(index) => self.launch_tool(index, true),
            Action::Stop(index) => {
                let id = self.tools[index].id.clone();
                match self.processes.stop(&id) {
                    Ok(Some(pid)) => {
                        self.set_status(format!("已结束 {}（pid {}，只结束本工具拉起的进程）", id, pid))
                    }
                    Ok(None) => self.set_status(format!(
                        "{} 没有由 toolhub 启动且仍在运行的进程，未做任何操作",
                        id
                    )),
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
                let tool = &self.tools[index];
                let path = match self.resolve_cache.get(tool, &self.config) {
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

    /// 启动（或直达设置）
    ///
    /// `settings == true` 时不纳入“运行中”跟踪：设置窗口只是工具的附属入口，
    /// 用它代表主进程是否在运行会误判。
    fn launch_tool(&mut self, index: usize, settings: bool) {
        let id = self.tools[index].id.clone();
        if !settings {
            let running = self.processes.running_pid(&id);
            match launch_guard(&id, running) {
                LaunchGuard::SelfTool => {
                    self.set_error("toolhub 自身不能从这里启动：会递归打开工具中心");
                    return;
                }
                LaunchGuard::AlreadyRunning(pid) => {
                    self.set_status(format!(
                        "{} 已在运行（pid {}），不会重复启动；如需重开请先结束进程",
                        id, pid
                    ));
                    return;
                }
                LaunchGuard::Ready => {}
            }
        }

        let args = if settings {
            &self.tools[index].settings
        } else {
            &self.tools[index].launch
        };
        let result = launcher::launch(&self.tools[index], args, &self.config);
        match result {
            Ok(launched) => {
                let message = launched.message;
                if settings {
                    // 只丢弃句柄，不结束进程
                    drop(launched.child);
                    self.set_status(format!("{} · 设置：{}", id, message));
                } else {
                    self.processes.insert(&id, launched.child);
                    self.set_status(message);
                }
            }
            Err(e) => {
                // 解析结果可能已经过期（例如刚 build 完路径变了），下次重算
                self.resolve_cache.invalidate(&id);
                self.set_error(e);
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

        let footer = self.footer_hint();
        let status_text = self.status.clone();
        let status_color = self.status_color();

        egui::TopBottomPanel::bottom("status_bar")
            .frame(Frame::new().fill(BG).inner_margin(Margin::symmetric(20, 9)))
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.add(
                        egui::Label::new(RichText::new(&footer).size(10.5).color(TEXT_FAINT))
                            .truncate(),
                    );
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        ui.add(
                            egui::Label::new(
                                RichText::new(&status_text).size(11.0).color(status_color),
                            )
                            .truncate(),
                        );
                    });
                });
            });

        // 每帧只对已拉起的子进程做一次 try_wait，已退出的条目在这里被清掉
        let running: HashSet<String> = self.processes.poll_running();
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
                        let visible = self
                            .tools
                            .iter()
                            .filter(|tool| matches_filter(tool, &self.filter))
                            .count();
                        ui.label(
                            RichText::new(format!("显示 {} / {} 个工具", visible, self.tools.len()))
                                .size(10.5)
                                .color(TEXT_FAINT),
                        );
                    });
                });
                ui.add_space(10.0);

                problems_banner(ui, &self.registry.problems);

                // 只收集可见行的下标，条目本身按引用取，不再每帧 clone
                let visible: Vec<usize> = self
                    .tools
                    .iter()
                    .enumerate()
                    .filter(|(_, tool)| matches_filter(tool, &self.filter))
                    .map(|(index, _)| index)
                    .collect();

                if visible.is_empty() {
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
                        for &index in &visible {
                            let tool = &self.tools[index];
                            // 缓存命中时不再读盘；首次（或失效后）才真正解析
                            let exe_hint = self.resolve_cache.describe(tool, &self.config);
                            let row = RowCtx {
                                tool,
                                exe_hint: &exe_hint,
                                problems: &self.registry.problems,
                                index,
                                total: self.tools.len(),
                                running: running.contains(&tool.id),
                                is_self: tool.id == SELF_ID,
                            };
                            if let Some(action) = tool_row(ui, row) {
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
                        let mut summary = format!(
                            "已注册 {} 个工具 · {}",
                            self.tools.len(),
                            self.registry.source.label()
                        );
                        if !self.registry.problems.is_empty() {
                            summary.push_str(&format!(
                                " · {} 条问题",
                                self.registry.problems.len()
                            ));
                        }
                        ui.add(
                            egui::Label::new(RichText::new(summary).size(10.5).color(TEXT_FAINT))
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
}

// ─── 列表行 ────────────────────────────────────────────────

/// 渲染单行所需的数据（都按引用传入，避免 clone）
struct RowCtx<'a> {
    tool: &'a ToolEntry,
    exe_hint: &'a str,
    /// 注册表全部问题：行内自行筛出与该工具相关的
    problems: &'a [Problem],
    index: usize,
    total: usize,
    running: bool,
    is_self: bool,
}

fn tool_row(ui: &mut Ui, row: RowCtx<'_>) -> Option<Action> {
    let RowCtx {
        tool,
        exe_hint,
        problems,
        index,
        total,
        running,
        is_self,
    } = row;
    let width = ui.available_width();
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
                    if running {
                        tag(ui, "运行中", RUNNING, 46.0);
                    }
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
                // 注册表校验问题：只标在相关行上（软提示，不影响启动）
                for problem in problems
                    .iter()
                    .filter(|p| p.tool_id.as_deref() == Some(tool.id.as_str()))
                {
                    ui.label(
                        RichText::new(format!("注意：{}", problem.message))
                            .size(10.0)
                            .color(WARN),
                    )
                    .on_hover_text(problem.message.as_str());
                }
            });

            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                let down = ui
                    .add_enabled(index + 1 < total, small_button("↓"))
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

                if running {
                    let stop = ui
                        .add(pill_button("结束", Color32::from_rgb(92, 42, 48), Color32::WHITE))
                        .on_hover_text("结束本工具拉起的进程（只影响 toolhub 自己 spawn 的 pid）");
                    if stop.clicked() {
                        action = Some(Action::Stop(index));
                    }
                }

                let launch = ui.add_enabled(!is_self, pill_button("启动", ACCENT_DIM, Color32::WHITE));
                let launch = if is_self {
                    launch.on_disabled_hover_text(
                        "toolhub 自身不能从这里启动：会递归打开工具中心",
                    )
                } else if running {
                    launch.on_hover_text("已在运行；再点只会在状态栏提示，不会重复启动")
                } else {
                    launch.on_hover_text("启动工具")
                };
                if launch.clicked() {
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
        if running && ui.button("结束进程").clicked() {
            action = Some(Action::Stop(index));
            ui.close_menu();
        }
    });

    action
}

/// 注册表问题横幅：顶部可读错误，软提示不阻塞其它工具
fn problems_banner(ui: &mut Ui, problems: &[Problem]) {
    if problems.is_empty() {
        return;
    }
    let frame = Frame::new()
        .fill(Color32::from_rgb(44, 37, 24))
        .stroke(Stroke::new(1.0, Color32::from_rgb(112, 88, 40)))
        .corner_radius(CornerRadius::same(10))
        .inner_margin(Margin::symmetric(14, 10));
    frame.show(ui, |ui| {
        ui.label(
            RichText::new(format!(
                "注册表校验发现 {} 条问题（软提示，不影响其它工具启动）",
                problems.len()
            ))
            .size(11.5)
            .strong()
            .color(WARN),
        );
        for problem in problems.iter().take(5) {
            ui.add(
                egui::Label::new(
                    RichText::new(format!("· {}", problem.display()))
                        .size(10.5)
                        .color(WARN),
                )
                .truncate(),
            )
            .on_hover_text(problem.message.as_str());
        }
        if problems.len() > 5 {
            ui.label(
                RichText::new(format!(
                    "…另有 {} 条，修正 registry.json 后点「刷新」重新校验",
                    problems.len() - 5
                ))
                .size(10.0)
                .color(TEXT_FAINT),
            );
        }
    });
    ui.add_space(8.0);
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
    tag(ui, label, color, 34.0);
}

fn tag(ui: &mut Ui, label: &str, color: Color32, width: f32) {
    let (rect, _) = ui.allocate_exact_size(Vec2::new(width, 16.0), Sense::hover());
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn self_row_is_never_launchable() {
        // 即使记录里恰好有同 id 的子进程，自身行也优先判为“禁用”
        assert_eq!(launch_guard("toolhub", None), LaunchGuard::SelfTool);
        assert_eq!(launch_guard("toolhub", Some(4242)), LaunchGuard::SelfTool);
        assert_eq!(launch_guard(SELF_ID, None), LaunchGuard::SelfTool);
    }

    #[test]
    fn running_tool_is_prompted_not_duplicated() {
        assert_eq!(
            launch_guard("deskman", Some(1234)),
            LaunchGuard::AlreadyRunning(1234)
        );
        assert_eq!(launch_guard("deskman", None), LaunchGuard::Ready);
    }
}
