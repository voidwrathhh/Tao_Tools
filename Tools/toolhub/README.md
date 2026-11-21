# toolhub — Tao Tools 工具中心

## 简介

toolhub 是 tao_tools 的统一入口：用一个列表界面集中展示所有已注册工具的快捷入口，
支持启动工具、直达各工具自己的设置界面，并可按需调整列表显示顺序。

**它同时是这套工具链的“登记处”**：toolhub 不会自动发现新工具，
每个新增的 tao_tools 工具都**必须注册到 `toolhub/registry.json`**，
否则不会出现在工具中心里。详见下文「注册新工具」。

## 版本

v1.0.1

## 功能列表

- [x] 列表式工具快捷入口（图标、名称、版本、类型、说明、可执行文件解析结果）
- [x] 一键启动工具（GUI / TUI / CLI 均支持，终端类工具自动开新控制台）
- [x] 一键直达工具设置界面（调用各工具自带的 `settings` 入口）
- [x] 显示顺序调整：`↑` / `↓` 上移下移，顺序立即保存
- [x] 按名称 / 说明过滤工具
- [x] 双击条目直接启动；右键打开 README、工具目录或直接启动
- [x] 可执行文件解析：用户配置路径 → `target/debug` / `target/release` → `~/.cargo/bin` → `PATH` → `cargo run -p <工具>`
- [x] 注册表来源提示：仓库注册表 / 用户注册表 / 内置注册表
- [x] 命令行辅助：`toolhub list`、`toolhub registry`、`toolhub open-registry`

## 开发规范

通用规范见根目录 [`AGENTS.md`](../../AGENTS.md)，本工具特有约定如下：

### 注册表是唯一数据源

- `registry.json` 决定界面上有哪些工具，界面代码中**不允许**写死任何工具名。
- 新增工具只改 `registry.json`，不需要改 toolhub 源码。

### 注册表加载优先级

1. `<workspace>/toolhub/registry.json` —— 仓库内注册表（开发时以它为准）
2. `~/.tao_tools/toolhub/registry.json` —— 用户自定义覆盖（可选）
3. 编译期内置的 `registry.json` —— `cargo install` 后离开仓库时使用

workspace 根目录通过向上查找包含 `[workspace]` 的 `Cargo.toml` 得到，
也可用环境变量 `TAO_TOOLS_ROOT` 显式指定。

### 用户本地配置

`~/.tao_tools/toolhub/config.json`：

```json
{
  "order": ["deskman", "blueinfo", "sertool", "friend"],
  "paths": { "sertool": "D:\\Tools\\sertool.exe" }
}
```

- `order`：界面显示顺序，调整顺序时自动写入；注册表里新增但 `order` 中缺失的工具会自动追加到末尾。
- `paths`：手动指定可执行文件路径，优先于自动探测。
- 排序只影响本机显示，不改动 `registry.json`，不会污染仓库。

### registry.json 字段

| 字段 | 必填 | 说明 |
| --- | --- | --- |
| `id` | 是 | 工具目录名，同时是默认可执行文件名（如 `sertool`） |
| `title` | 否 | 界面显示名称，缺省用 `id` |
| `description` | 否 | 一句话说明，显示在名称下方 |
| `version` | 否 | 当前版本，显示为 `vX.Y.Z` |
| `kind` | 否 | `gui` / `tui` / `cli`，仅用于界面上的类型标签 |
| `launch` | 否 | 点击“启动”时传给可执行文件的参数数组，如 `["gui"]` |
| `settings` | 否 | 点击“设置”时传给可执行文件的参数数组；为空则“设置”按钮禁用 |
| `console` | 否 | `true` 时在独立控制台窗口运行（终端类工具） |
| `readme` | 否 | README 相对 workspace 根目录的路径 |

## 注册新工具（强制）

> **约定：每一个新增的 tao_tools 工具都必须在本工具中注册。**
> 未注册的工具不会出现在工具中心，视为“未接入工具链”。

步骤：

1. 在新工具目录下实现源码，并按需提供设置入口：
   - GUI 工具：提供 `settings` 子命令，直接打开设置窗口（推荐）
   - TUI 工具：提供 `settings` 子命令，进入设置界面
   - CLI 工具：提供 `--edit-config` 或等价参数，直接打开配置文件
2. 在 `toolhub/registry.json` 的 `tools` 数组中追加一项，例如：

   ```json
   {
     "id": "my_tool",
     "title": "我的工具",
     "description": "一句话说明这个工具做什么",
     "version": "1.0.0",
     "kind": "gui",
     "launch": [],
     "settings": ["settings"],
     "console": false,
     "readme": "my_tool/README.md"
   }
   ```

3. 同步更新本 README 的「已注册工具」表格与各工具自己的 README / 项目日志。
4. 执行 `cargo build -p my_tool`，再运行 `toolhub list` 确认能解析到可执行文件。

### 已注册工具

| 工具 | 类型 | 设置入口 | 说明 |
| --- | --- | --- | --- |
| deskman | GUI | `deskman settings` | 桌面快捷方式分类整理 |
| blueinfo | GUI | `blueinfo settings` | 蓝牙调试与数据可视化 |
| sertool | TUI | `sertool settings` | 串口终端 |
| friend | CLI | `friend --edit-config` | AI 聊天助手 |

toolhub 自身作为入口不列入注册表，避免自引用。

## 开发进度 / 开发计划

已完成 v1.0.1：列表 UI、启动、设置直达、排序、过滤、右键菜单、注册表来源提示、命令行辅助。

后续计划：

- [ ] 收藏 / 置顶分组
- [ ] 启动参数模板（同一工具多套参数）
- [ ] 工具健康检查（版本不一致、可执行文件缺失提示）
- [ ] 从 `cargo metadata` 自动发现未注册工具并提示补登记

## 使用方法

```powershell
# 打开工具中心
toolhub
# 或
cargo run -p toolhub

# 列出已注册工具及可执行文件解析结果
toolhub list

# 查看当前生效的注册表路径
toolhub registry

# 用系统默认程序打开注册表文件
toolhub open-registry

# 版本 / 帮助
toolhub --version
toolhub --help
```

界面操作：

- 「启动」按钮或双击条目：运行工具
- 「设置」按钮：直接打开该工具自带的设置界面
- `↑` / `↓`：调整显示顺序，自动保存
- 右键条目：打开 README / 工具目录 / 直接启动
- 顶部「刷新」：重新读取注册表与配置；「注册表」：打开注册表文件；「说明」：打开本 README

## 环境依赖

- Rust 2021 edition（建议 1.75+）
- Windows 10/11（GUI 使用 wgpu 后端；路径解析与 `CREATE_NEW_CONSOLE` 针对 Windows）
- 依赖：`eframe` / `egui` 0.31（wgpu 后端）、`serde` / `serde_json`、`dirs`
- 中文显示依赖系统字体：`msyh.ttc` → `simsun.ttc` → `simhei.ttf`

## 更新历史

- v1.0.1：workspace 清单迁到 `Tools/`，`workspace_root()` 同时识别 `Cargo.toml` 与 `Tools/Cargo.toml` 两种布局
- v1.0.0：首个版本，实现列表入口、启动、设置直达、排序、过滤与注册机制
