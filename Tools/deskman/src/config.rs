use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppConfig {
    pub locked: bool,
    pub desktop: DesktopConfig,
    pub scan: ScanConfig,
    pub groups: Vec<DesktopGroup>,
    pub dock: DockConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct DesktopConfig {
    pub width: f32,
    pub height: f32,
    pub opacity: f32,
    pub always_on_top: bool,
    pub fullscreen: bool,
    pub real_icons: bool,
    /// 收纳进面板的图标自动从桌面移除（退出时按原位恢复）
    pub takeover_icons: bool,
    /// 图标尺寸跟随系统桌面设置变化
    pub follow_system_icon_size: bool,
    /// 面板样式：dark-glass 深色玻璃 / ios-light iOS 亮色 / ios-dark iOS 暗色
    pub style: String,
    /// 图标下方是否显示名称
    pub show_labels: bool,
    /// 底部 Dock 滑轨
    pub dock: DockConfig,
}

impl Default for DesktopConfig {
    fn default() -> Self {
        Self {
            width: 920.0,
            height: 560.0,
            opacity: 0.92,
            always_on_top: false,
            fullscreen: true,
            real_icons: true,
            takeover_icons: true,
            follow_system_icon_size: true,
            style: "dark-glass".to_string(),
            show_labels: true,
            dock: DockConfig::default(),
        }
    }
}

/// 底部 Dock 滑轨配置
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct DockConfig {
    pub enabled: bool,
    pub icon_size: f32,
    /// 悬停放大倍数（1.0 = 不放大）
    pub magnify: f32,
    /// 底板不透明度（0 = 完全透明，仅图标）
    pub opacity: f32,
    pub corner_radius: f32,
    pub reflection: bool,
    /// 回环轮盘模式（环形布局，滚动无限环绕）
    pub loop_mode: bool,
    pub items: Vec<DesktopItem>,
}

impl Default for DockConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            icon_size: 48.0,
            magnify: 1.8,
            opacity: 0.55,
            corner_radius: 18.0,
            reflection: true,
            loop_mode: true,
            items: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct ScanConfig {
    pub include_user_desktop: bool,
    pub include_public_desktop: bool,
    pub show_unclassified: bool,
    /// 桌面目录变更实时同步（后台线程轮询，约 1 秒一次）
    pub live_sync: bool,
    /// 忽略清单：小写完整路径；扫描与实时同步都会跳过，从面板移除的项目写在这里
    pub ignore: Vec<String>,
}

impl Default for ScanConfig {
    fn default() -> Self {
        Self {
            include_user_desktop: true,
            include_public_desktop: true,
            show_unclassified: true,
            live_sync: true,
            ignore: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct DesktopGroup {
    pub id: String,
    pub name: String,
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
    pub collapsed: bool,
    pub position_locked: bool,
    pub size_locked: bool,
    pub layout: GroupLayout,
    pub icon_size: f32,
    pub items: Vec<DesktopItem>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum GroupLayout {
    Free,
    Grid,
    Horizontal,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct DesktopItem {
    pub id: String,
    pub name: String,
    pub path: String,
    pub kind: DesktopItemKind,
    pub args: Vec<String>,
    pub pinned: bool,
    pub x: f32,
    pub y: f32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DesktopItemKind {
    Shortcut,
    Url,
    Folder,
    File,
    Command,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            locked: true,
            desktop: DesktopConfig::default(),
            scan: ScanConfig::default(),
            groups: default_groups(),
            dock: DockConfig::default(),
        }
    }
}

impl Default for DesktopItem {
    fn default() -> Self {
        Self {
            id: String::new(),
            name: String::new(),
            path: String::new(),
            kind: DesktopItemKind::File,
            args: Vec::new(),
            pinned: false,
            x: 0.0,
            y: 0.0,
        }
    }
}

impl Default for DesktopGroup {
    fn default() -> Self {
        Self {
            id: String::new(),
            name: String::new(),
            x: 0.0,
            y: 0.0,
            width: 300.0,
            height: 220.0,
            collapsed: false,
            position_locked: false,
            size_locked: false,
            layout: GroupLayout::Free,
            icon_size: 48.0,
            items: Vec::new(),
        }
    }
}

impl AppConfig {
    pub fn load() -> Self {
        let path = config_path();
        let mut config: Self = match read_json_file(&path) {
            Some(config) => config,
            None => match read_json_file(&sibling_path(&path, ".bak")) {
                Some(config) => {
                    eprintln!("提示：主配置不可用，已从备份恢复 {}", path.display());
                    config
                }
                None => {
                    if path.exists() {
                        eprintln!("警告：配置文件损坏，已使用默认配置 {}", path.display());
                    }
                    Self::default()
                }
            },
        };
        config.normalize_layouts(false);
        config
    }

    pub fn save(&self) {
        if let Err(e) = self.save_result() {
            eprintln!("警告：无法保存配置 - {}", e);
        }
    }

    /// 与 `save` 相同，但把失败原因返回给调用方（拖拽添加等需要回滚的场景）
    pub fn save_result(&self) -> Result<(), String> {
        let path = config_path();
        let content = serde_json::to_string_pretty(self)
            .map_err(|e| format!("配置序列化失败：{}", e))?;
        write_json_atomic(&path, &content)
            .map_err(|e| format!("写入 {} 失败：{}", path.display(), e))
    }
}

impl AppConfig {
    pub fn normalize_layouts(&mut self, force: bool) {
        for group in &mut self.groups {
            normalize_group_layout(group, force);
        }
    }
}

fn normalize_group_layout(group: &mut DesktopGroup, force: bool) {
    if group.items.is_empty() {
        return;
    }

    if !force {
        let unpositioned = group
            .items
            .iter()
            .all(|item| item.x == 0.0 && item.y == 0.0);
        if !unpositioned {
            return;
        }
    }

    let columns = if group.width >= 420.0 {
        4
    } else if group.width >= 300.0 {
        3
    } else {
        2
    };
    let step_x = group.icon_size + 12.0;
    let step_y = group.icon_size + 30.0;

    for (index, item) in group.items.iter_mut().enumerate() {
        item.x = (index % columns) as f32 * step_x;
        item.y = (index / columns) as f32 * step_y;
    }
}

fn default_groups() -> Vec<DesktopGroup> {
    // 默认收纳到屏幕右侧竖排（全屏工作区下贴右边距），可自由拖动自定义
    let right = 1360.0;
    vec![
        DesktopGroup {
            id: "unclassified".to_string(),
            name: "未分类".to_string(),
            x: right,
            y: 24.0,
            width: 300.0,
            height: 220.0,
            collapsed: false,
            position_locked: false,
            size_locked: false,
            layout: GroupLayout::Free,
            icon_size: 48.0,
            items: vec![
                DesktopItem {
                    id: "terminal".to_string(),
                    name: "终端".to_string(),
                    path: "powershell".to_string(),
                    kind: DesktopItemKind::Command,
                    args: Vec::new(),
                    pinned: false,
                    x: 0.0,
                    y: 0.0,
                },
                DesktopItem {
                    id: "tao-tools-folder".to_string(),
                    name: "项目目录".to_string(),
                    path: "D:\\File_Data\\Tao_Tools".to_string(),
                    kind: DesktopItemKind::Folder,
                    args: Vec::new(),
                    pinned: false,
                    x: 0.0,
                    y: 0.0,
                },
            ],
        },
        DesktopGroup {
            id: "work".to_string(),
            name: "工作".to_string(),
            x: right + 40.0,
            y: 280.0,
            width: 300.0,
            height: 220.0,
            collapsed: false,
            position_locked: false,
            size_locked: false,
            layout: GroupLayout::Free,
            icon_size: 48.0,
            items: vec![DesktopItem {
                id: "deskman-readme".to_string(),
                name: "deskman README".to_string(),
                path: "D:\\File_Data\\Tao_Tools\\deskman\\README.md".to_string(),
                kind: DesktopItemKind::File,
                args: Vec::new(),
                pinned: false,
                x: 0.0,
                y: 0.0,
            }],
        },
    ]
}

/// 已从桌面移除的图标记录（用于退出/崩溃后按原位恢复）
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct TakenIcon {
    pub name: String,
    pub x: i32,
    pub y: i32,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct DesktopState {
    pub taken: Vec<TakenIcon>,
    /// 视图开关被本工具切换过（退出时恢复）
    pub view_toggled: bool,
}

impl DesktopState {
    pub fn load() -> Self {
        let path = desktop_state_path();
        let bak = sibling_path(&path, ".bak");
        if let Some(state) = read_json_file(&path) {
            return state;
        }
        if let Some(state) = read_json_file(&bak) {
            eprintln!("提示：桌面状态主文件不可用，已从备份恢复 {}", bak.display());
            return state;
        }
        if !path.exists() && !bak.exists() {
            // 首次启动没有任何历史，不做“恢复”动作，避免把可见的图标切走
            return Self::default();
        }
        // 主文件与备份都损坏：保守认为上次可能隐藏过图标，启动时先恢复显示
        eprintln!("警告：桌面状态文件损坏，按保守值启动（先恢复桌面图标显示）");
        Self {
            taken: Vec::new(),
            view_toggled: true,
        }
    }

    pub fn save(&self) {
        let path = desktop_state_path();
        match serde_json::to_string_pretty(self) {
            Ok(content) => {
                if let Err(e) = write_json_atomic(&path, &content) {
                    eprintln!("警告：无法保存桌面状态 {} - {}", path.display(), e);
                }
            }
            Err(e) => eprintln!("警告：桌面状态序列化失败 - {}", e),
        }
    }
}

/// 同目录的伴生文件路径（`config.json` -> `config.json.bak` / `.tmp`）
fn sibling_path(path: &Path, suffix: &str) -> PathBuf {
    let mut name = path.as_os_str().to_os_string();
    name.push(suffix);
    PathBuf::from(name)
}

/// 读取并解析 JSON；文件不存在或内容损坏都返回 None
fn read_json_file<T: serde::de::DeserializeOwned>(path: &Path) -> Option<T> {
    let content = fs::read_to_string(path).ok()?;
    serde_json::from_str(&content).ok()
}

/// 原子写 JSON：先写同目录 `.tmp` 并落盘，再用 rename 替换主文件；
/// 替换前把上一份主文件留成 `.bak`。
/// 中途崩溃最多留下一个残片 `.tmp`，主文件要么是旧的完整内容、要么是新的完整内容。
fn write_json_atomic(path: &Path, content: &str) -> std::io::Result<()> {
    use std::io::Write;

    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let tmp = sibling_path(path, ".tmp");
    let bak = sibling_path(path, ".bak");

    let write_tmp = || -> std::io::Result<()> {
        let mut file = fs::File::create(&tmp)?;
        file.write_all(content.as_bytes())?;
        file.sync_all()?;
        Ok(())
    };
    if let Err(e) = write_tmp() {
        let _ = fs::remove_file(&tmp);
        return Err(e);
    }
    if path.exists() {
        if let Err(e) = fs::copy(path, &bak) {
            let _ = fs::remove_file(&tmp);
            return Err(e);
        }
    }
    if let Err(e) = fs::rename(&tmp, path) {
        let _ = fs::remove_file(&tmp);
        return Err(e);
    }
    Ok(())
}

pub fn desktop_state_path() -> PathBuf {
    if let Some(base) = dirs::data_dir() {
        base.join(".tao_tools").join("deskman").join("desktop_state.json")
    } else {
        PathBuf::from(".").join(".tao_tools").join("deskman").join("desktop_state.json")
    }
}

pub fn config_path() -> PathBuf {
    if let Some(base) = dirs::data_dir() {
        base.join(".tao_tools").join("deskman").join("config.json")
    } else {
        PathBuf::from(".")
            .join(".tao_tools")
            .join("deskman")
            .join("config.json")
    }
}
