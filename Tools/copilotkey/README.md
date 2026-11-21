# copilotkey - Copilot 键改造工具

## 简介

把笔记本/键盘上的 **Copilot 键**变成自己的全局功能键：按一下即执行配置的目标（启动某个 tao_tools 工具、任意 exe、URL 或脚本），
系统不再收到这个按键，也就不会再弹出 Copilot / 搜索面板 —— 相当于凭空多出一个可自定义的快捷键。

## 版本

v1.0.1（与 `Cargo.toml` 保持一致）

## 原理

Windows 把 Copilot 键上报成 `Win + Shift + F23`。程序装一个 `WH_KEYBOARD_LL` 全局低级键盘钩子：

1. 识别到 `Win+Shift+F23`（或你 `set` 的任意组合）→ **吞掉**这个按键，不让它传给系统；
2. 同一时刻启动配置好的目标程序；
3. 其它按键原样放行，不影响正常输入。

因为钩子是在用户态做过滤，不需要驱动、不需要管理员权限。

## 功能列表

- [x] `trace` —— 打印接下来收到的所有按键事件（键码、扫描码、修饰键、是否注入），用来确认 Copilot 键到底上报了什么
- [x] `learn` —— 按一次 Copilot 键自动捕获键码，`--save` 直接保存为触发键
- [x] `set <组合键>` —— 手动指定触发键，如 `win+shift+f23`、`f23`
- [x] `target <程序> [参数...]` —— 设置按下后要启动的目标：`--list` 列出已编译的 tao_tools 工具，`--clear` 清除
- [x] `run` —— 前台运行守护进程（正常由 `install` 拉起）
- [x] `install` / `uninstall` —— 注册 / 移除开机自启（`HKCU\...\CurrentVersion\Run` 的 `TaoToolsCopilotKey`），并启停守护进程
- [x] `status` —— 查看当前配置与运行状态
- [x] `stop` —— 停止守护进程
- [x] `test` —— 模拟触发一次，验证端到端效果
- [ ] 一个键映射多组动作（宏）
- [ ] 长按 / 双击与单次按下区分
- [ ] 图形化配置界面
- [ ] 多套配置快速切换

## 使用方法

```bash
# 1) 先确认 Copilot 键上报了什么（打印 15 秒内的按键事件）
copilotkey trace --seconds 15

# 2) 学习并保存触发键（按一次 Copilot 键）
copilotkey learn --save

# 3) 设置按下后启动什么
copilotkey target --list              # 看有哪些已编译的 tao_tools 工具可用
copilotkey target deskman             # 启动 deskman
copilotkey target "C:\Windows\System32\calc.exe"

# 4) 自检 + 注册开机自启（会立即拉起守护进程）
copilotkey test
copilotkey install

# 日常查看与停止
copilotkey status
copilotkey stop
copilotkey uninstall
```

## 配置

配置文件：`%APPDATA%\.tao_tools\copilotkey\config.json`（首次运行时自动生成默认值）

| 字段 | 含义 |
|---|---|
| `trigger_vk` / `trigger_mods` / `trigger_label` | 触发键的虚拟键码、修饰键位掩码、展示名 |
| `strict_mods` | 是否要求修饰键完全匹配（多按/少按修饰键都不触发） |
| `swallow` | 触发时是否吞掉该按键（关闭后系统仍会收到） |
| `ignore_injected` | 是否忽略其它程序注入的按键（防止误触发） |
| `debounce_ms` | 防连击间隔（毫秒） |
| `target` / `arguments` / `workdir` | 目标程序（或 tao_tools 工具名）、参数、工作目录 |
| `log` | 是否把启动记录写日志 |

## 开发规范

通用规范见根目录 [AGENTS.md](../../AGENTS.md)，本工具特有约定：

- **平台**：仅面向 Windows 10/11（键盘钩子、注册表、控制台 API 依赖 `windows-sys`，只在 Windows 上构建与验证）
- **模块划分**：`main.rs` 命令分发与各子命令、`hook.rs` 低级键盘钩子与事件回调、`keys.rs` 键码/修饰键名称、`config.rs` 配置读写、`autostart.rs` 注册表开机自启、`launch.rs` 启动目标、`util.rs` 控制台与宽字符等工具函数
- **不做常驻服务**：靠注册表 Run 项在登录时以当前用户身份拉起 `run`，不写系统服务、不需要管理员权限
- **release 构建不弹控制台**（`windows_subsystem = "windows"`），需要输出的地方统一用 `out!` / `util::say` 附加到父控制台
- **参数解析手写 `match`**，不引入 clap（遵循根规范）
- 配置只写 `%APPDATA%\.tao_tools\copilotkey\config.json`，不落任何密钥

## 开发进度

- [x] v1.0.0：钩子截获 + 目标启动 + 开机自启 + trace/learn/set/target/status/test 全套命令
- [x] v1.0.1：适配 `Tools/` 新布局（按 `[workspace]` 标记自动探测仓库根）、`target` 解析失败显式告警、守护进程热重载配置（改目标不用重启）
- [x] 真机验证（ASUS TX Gaming FA608PM）：Copilot 键确实上报 `Win+Shift+F23`，按下后启动目标程序、搜索面板不再出现
- [x] 本机已配置：目标为 `toolhub.exe`（工具中心），开机自启已注册，守护进程常驻
- [ ] 待做：多动作映射、长按/双击区分、图形化配置（见上面功能列表未勾选项）
- [ ] 待验证：不同品牌笔记本 Copilot 键上报差异（部分机型是 `F23`，部分是 `Win+Shift+F23`，用 `trace` 确认）

## 环境依赖

- Rust 2021 edition+
- Windows 10 / 11（x64）
- 无需管理员权限；无需额外系统组件

## 更新历史

### v1.0.1

- 修复：适配仓库 `Tools/` 新布局 —— 仓库根改为按 `[workspace]` 标记向上探测（exe 路径 → 当前目录 → 已知兜底路径），仓库整体搬家后 `target <工具名>` 不再失效
- 修复：守护进程热重载配置 —— 每次触发重新读取 `config.json`，改完目标 / 参数立即生效，无需重启守护进程
- 改进：`target` 未能把名字解析成已编译 exe 时打印告警，不再静默存下裸名字（按下按键才发现启动失败）

### v1.0.0

- 首个版本：`WH_KEYBOARD_LL` 全局钩子截获 Copilot 键（Win+Shift+F23），吞键并启动自定义目标
- 命令：`trace` / `learn` / `set` / `target` / `run` / `install` / `uninstall` / `status` / `stop` / `test` / `help`
- 注册表 `HKCU\Software\Microsoft\Windows\CurrentVersion\Run` 注册开机自启
