# toolhub — Tao Tools 工具中心

## 简介

toolhub 是 tao_tools 的统一入口：用一个列表界面集中展示所有已注册工具的快捷入口，
支持启动工具、直达各工具自己的设置界面，并可按需调整列表显示顺序。

**它同时是这套工具链的“登记处”**：toolhub 不会自动发现新工具，
每个新增的 tao_tools 工具都**必须注册到 `toolhub/registry.json`**，
否则不会出现在工具中心里。详见下文「注册新工具」。

## 版本

v1.1.0

## 功能列表

- [x] 列表式工具快捷入口（图标、名称、版本、类型、说明、可执行文件解析结果）
- [x] 一键启动工具（GUI / TUI / CLI 均支持，终端类工具自动开新控制台）
- [x] 一键直达工具设置界面（调用各工具自带的 `settings` 入口）
- [x] 显示顺序调整：`↑` / `↓` 上移下移，顺序立即保存
- [x] 按名称 / 说明过滤工具
- [x] 双击条目直接启动；右键打开 README、工具目录或直接启动
- [x] 可执行文件解析：用户配置路径 → `target/debug` / `target/release` → `~/.cargo/bin` → `PATH` → `cargo run -p <工具>`
- [x] 解析结果按 id 缓存：只在启动 / 刷新 / 启动失败时重算，滚动不再每帧读盘
- [x] 注册表来源提示：仓库注册表 / 用户注册表 / 内置注册表
- [x] 注册表加载期校验：schema、id 非空且不重复、kind 白名单、readme 是否存在；问题显示在状态栏、列表顶部横幅与对应行，只做软提示
- [x] 自身行禁止启动（`toolhub` 行的「启动」禁用并给出说明），避免递归打开工具中心
- [x] 运行状态提示：同一工具已在运行时只提示「已在运行」，不再重复拉起；运行中的行显示「结束」按钮
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

### 注册表加载期校验

加载时按本文「registry.json 字段」表做事后校验，问题收集进 `Registry::problems`：

- `schema` 必须等于 `1`（不符只提示，条目照常加载）；
- `id` 不能为空、不能重复（空 id 丢弃、重复 id 只保留首次出现）；
- `kind` 必须属于 `gui` / `tui` / `cli`（未知值按 CLI 显示）；
- 声明了 `readme` 时文件必须存在（路径相对 workspace 根目录）；
- 出现未知字段（如把 `launch` 拼成 `"lauch"`）会点名报出，但不丢弃该条目。

所有问题都是**软提示**：状态栏、列表顶部横幅、对应行的「注意：…」三处展示，**不阻塞**其余工具启动。
`toolhub list` 会把问题打到 stderr，stdout 仍是可直接解析的表格，退出码不受影响。

### 可执行文件解析结果缓存

`launcher::ResolveCache` 按 id 缓存解析结果，只在以下时机失效：

- 启动界面（`HubApp::new`）与点「刷新」（重新读注册表与 `config.json`）；
- `config.paths` 变更（同样经由刷新）；
- 某个工具启动失败（下一次使用它时重新解析）。

已知取舍：刚 `cargo build` 完但没有点「刷新」时，行里仍显示旧的 exe 路径。
调试构建下底部会显示「解析调用 N 次」，300 帧后应等于工具数，可用来确认缓存生效。

### 运行中与结束进程

- 启动成功后按 id 记录本进程 spawn 的 `Child`，每帧用 `try_wait` 清理已退出的条目；
- 同一 id 已在运行时，点「启动」只在状态栏提示「已在运行」，不会重复拉起；
- 「结束」只对本进程自己 spawn 的 pid 调 `kill`，不按进程名杀、也不接管各工具自身的单例逻辑。

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
| copilotkey | CLI | `copilotkey status` | Copilot 键改造 |
| toolhub | GUI | —（本工具自身） | 工具中心入口 |

toolhub 自身也登记在注册表里，便于其它入口互相跳转；列表里会显示自己这一行，但这一行的「启动」是禁用的（tooltip 会说明原因），不会递归启动自己。

## 开发进度 / 开发计划

已完成 v1.1.0：注册表加载期校验（问题在状态栏 / 顶部横幅 / 对应行可见）、解析结果按 id 缓存（不再每帧读盘）、自身行禁启动与运行中提示 / 结束进程。

下一步：控制台工具启动参数安全、工具健康检查（版本不一致 / 可执行文件缺失）、`cargo metadata` 自动发现未注册工具。

计划与验收标准见 [plans/toolhub.md](../../plans/toolhub.md)。

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

- 「启动」按钮或双击条目：运行工具（控制台工具会在新窗口中运行，退出后窗口保留现场；GUI 工具直接开窗）
- 同一工具已在运行时，再点「启动」只在状态栏提示「已在运行」，不会重复拉起；运行中的行右侧出现「结束」（只结束 toolhub 自己拉起的 pid）
- toolhub 自身这一行的「启动」禁用，悬停可看到原因
- 「设置」按钮：直接打开该工具自带的设置界面
- `↑` / `↓`：调整显示顺序，自动保存
- 右键条目：打开 README / 工具目录 / 直接启动 / 结束进程
- 顶部「刷新」：重新读取注册表与配置（并重算可执行文件解析结果）；「注册表」：打开注册表文件；「说明」：打开本 README
- 注册表有校验问题时，列表顶部会出现黄色横幅，对应行显示「注意：…」，状态栏也会提示；这些都不影响其它工具启动

## 环境依赖

- Rust 2021 edition（建议 1.75+）
- Windows 10/11（GUI 使用 wgpu 后端；路径解析与 `CREATE_NEW_CONSOLE` 针对 Windows）
- 依赖：`eframe` / `egui` 0.31（wgpu 后端）、`serde` / `serde_json`、`dirs`
- 中文显示依赖系统字体：`msyh.ttc` → `simsun.ttc` → `simhei.ttf`

## 更新历史

- v1.1.0：注册表加载期校验（schema / id 非空不重复 / kind 白名单 / readme 存在性 / 未知字段），问题在状态栏、顶部横幅与对应行软提示；可执行文件解析结果按 id 缓存，刷新与启动失败时失效；自身行禁止启动、同 id 已在运行只提示不重复拉起、运行中的行可「结束」（仅限自己 spawn 的 pid）
- v1.0.2：控制台工具（如 sertool）改用 `cmd /K` 启动，退出后窗口保留、输出可读；注册表同步 blueinfo v1.1.0（新增 `settings` 入口）与 sertool v2.4.0（`list` 启动 + `settings` 入口）
- v1.0.1：workspace 清单迁到 `Tools/`，`workspace_root()` 同时识别 `Cargo.toml` 与 `Tools/Cargo.toml` 两种布局
- v1.0.0：首个版本，实现列表入口、启动、设置直达、排序、过滤与注册机制
