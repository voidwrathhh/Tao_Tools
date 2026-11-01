use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;

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
pub struct ScanConfig {
    pub include_user_desktop: bool,
    pub include_public_desktop: bool,
    pub show_unclassified: bool,
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

#[derive(Debug, Clone, Serialize, Deserialize)]
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
            scan: ScanConfig {
                include_user_desktop: true,
                include_public_desktop: true,
                show_unclassified: true,
            },
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
        if !path.exists() {
            let mut config = Self::default();
            config.normalize_layouts(false);
            return config;
        }
        let mut config: Self = fs::read_to_string(&path)
            .ok()
            .and_then(|content| serde_json::from_str(&content).ok())
            .unwrap_or_default();
        config.normalize_layouts(false);
        config
    }

    pub fn save(&self) {
        let path = config_path();
        if let Some(parent) = path.parent() {
            if let Err(e) = fs::create_dir_all(parent) {
                eprintln!("警告：无法创建配置目录 {} - {}", parent.display(), e);
                return;
            }
        }
        match serde_json::to_string_pretty(self) {
            Ok(content) => {
                if let Err(e) = fs::write(&path, content) {
                    eprintln!("警告：无法保存配置 {} - {}", path.display(), e);
                }
            }
            Err(e) => eprintln!("警告：配置序列化失败 - {}", e),
        }
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
        fs::read_to_string(&path)
            .ok()
            .and_then(|content| serde_json::from_str(&content).ok())
            .unwrap_or_default()
    }

    pub fn save(&self) {
        let path = desktop_state_path();
        if let Some(parent) = path.parent() {
            let _ = fs::create_dir_all(parent);
        }
        match serde_json::to_string_pretty(self) {
            Ok(content) => {
                if let Err(e) = fs::write(&path, content) {
                    eprintln!("警告：无法保存桌面状态 {} - {}", path.display(), e);
                }
            }
            Err(e) => eprintln!("警告：桌面状态序列化失败 - {}", e),
        }
    }
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
