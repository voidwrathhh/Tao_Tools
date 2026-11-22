# copilotkey - Copilot 键改造工具

## 简介

把笔记本/键盘上的 **Copilot 键**变成自己的全局功能键：按一下即执行配置的目标（启动某个 tao_tools 工具、任意 exe、URL 或脚本），
系统不再收到这个按键，也就不会再弹出 Copilot / 搜索面板 —— 相当于凭空多出一个可自定义的快捷键。

## 版本

v1.0.2（与 `Cargo.toml` 保持一致）

## 原理

Windows 把 Copilot 键上报成 `Win + Shift + F23`。程序装一个 `WH_KEYBOARD_LL` 全局低级键盘钩子：

1. 识别到 `Win+Shift+F23`（或你 `set` 的任意组合）→ **吞掉**这个按键，不让它传给系统；
2. 同一时刻启动配置好的目标程序；
3. 其它按键原样放行，不影响正常输入；
4. 修饰键在**判定瞬间**用 `GetAsyncKeyState` 现算，不依赖钩子累计的历史状态 —— UAC / 安全桌面期间钩子收不到事件也不会留下“修饰键一直按着”的脏状态。

因为钩子是在用户态做过滤，不需要驱动、不需要管理员权限。

## 功能列表

- [x] `trace` —— 打印接下来收到的所有按键事件（键码、扫描码、修饰键、是否注入），用来确认 Copilot 键到底上报了什么
- [x] `learn` —— 按一次 Copilot 键自动捕获键码，`--save` 直接保存为触发键
- [x] `set <组合键>` —— 手动指定触发键，如 `win+shift+f23`、`f23`
- [x] 键位热重载 —— `set` / `learn --save` 改完配置后，运行中的守护进程 **1 秒内**自动换键，不需要重启（守护进程每 300ms 轮询一次配置）
- [x] 配置原子写与损坏保护 —— 写配置走「临时文件 + `fsync` + 重命名」，并发读写不会读到半个文件；解析失败时把原文件备份成 `config.json.broken-<时间戳>` 并以非零码退出，绝不静默回退默认值
- [x] `strict_mods` 精确匹配 —— 判定瞬间用 `GetAsyncKeyState` 现算修饰键，`(mods & 0xF) == (required & 0xF)`；多按一个 Ctrl 也不触发、不吞键
- [x] `target <程序> [参数...]` —— 设置按下后要启动的目标：`--list` 列出已编译的 tao_tools 工具，`--clear` 清除
- [x] `run` —— 前台运行守护进程（正常由 `install` 拉起）
- [x] `install` / `uninstall` —— 注册 / 移除开机自启（`HKCU\...\CurrentVersion\Run` 的 `TaoToolsCopilotKey`），并启停守护进程
- [x] `status` —— 查看当前配置与运行状态（配置文件损坏时明确提示“正在使用上次有效配置”，不会假装一切正常）
- [x] `stop` —— 停止守护进程
- [x] `test` —— 模拟触发一次，验证端到端效果

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

配置文件：`%APPDATA%\.tao_tools\copilotkey\config.json`（首次写配置时自动生成）

| 字段 | 含义 |
|---|---|
| `version` | 配置结构版本（当前 `1`）：旧文件自动迁移；版本高于本程序时拒绝加载，不会用旧版本覆盖新配置 |
| `trigger_vk` / `trigger_mods` / `trigger_label` | 触发键的虚拟键码、修饰键位掩码、展示名 |
| `strict_mods` | `true` = 修饰键必须**完全一致**：`Win+Shift+F23` 时多按一个 Ctrl 不触发、也不吞键；`false` = 只看主键，多按/少按修饰键都会触发（`set` 时会提示） |
| `swallow` | 触发时是否吞掉该按键（关闭后系统仍会收到） |
| `ignore_injected` | 是否忽略其它程序注入的按键（防止误触发） |
| `debounce_ms` | 防连击间隔（毫秒） |
| `target` / `arguments` / `workdir` | 目标程序（或 tao_tools 工具名）、参数、工作目录 |
| `log` | 是否把启动记录写日志 |

写入与损坏处理：

- **原子写**：`set` / `target` / `learn --save` 都是先写临时文件（`config.json.tmp-<pid>`）并落盘，再重命名覆盖正式文件 —— 并发读写不会读到半个 JSON，也不会读到“写了一半目标为空”的配置；
- **不静默回退**：配置文件解析失败时，`target` / `install` / `set` / `learn` / `test` 会把原文件备份成 `config.json.broken-<时间戳>`，然后**以非零码退出**，绝不用默认值覆盖你的配置；守护进程则保留内存里的上一次有效配置（last-good），并往日志写一行“配置已损坏，正在使用上次有效配置”；
- `status` 只读、不改文件：发现损坏时给出警告与恢复提示；
- `install` 只在配置文件**不存在**时写入初始配置，不会把默认值覆盖到已有配置上；
- `learn --save` 会把 `strict_mods` 复位为 `false`（学到的修饰键可能包含按 Copilot 键时顺手按住的其它键）。

## 开发规范

通用规范见根目录 [AGENTS.md](../../AGENTS.md)，本工具特有约定：

- **平台**：仅面向 Windows 10/11（键盘钩子、注册表、控制台 API 依赖 `windows-sys`，只在 Windows 上构建与验证）
- **模块划分**：`main.rs` 命令分发与各子命令、`hook.rs` 低级键盘钩子与事件回调、`keys.rs` 键码/修饰键名称、`config.rs` 配置读写、`autostart.rs` 注册表开机自启、`launch.rs` 启动目标、`util.rs` 控制台与宽字符等工具函数
- **不做常驻服务**：靠注册表 Run 项在登录时以当前用户身份拉起 `run`，不写系统服务、不需要管理员权限
- **release 构建不弹控制台**（`windows_subsystem = "windows"`），需要输出的地方统一用 `out!` / `util::say` 附加到父控制台
- **参数解析手写 `match`**，不引入 clap（遵循根规范）
- 配置只写 `%APPDATA%\.tao_tools\copilotkey\config.json`，不落任何密钥
- **配置读写只有一条路**：写一律走 `Config::save()`（临时文件 + `sync_all` + 重命名），命令行入口读一律走 `config::load_or_exit()`（损坏即备份并非零码退出），守护进程轮询用 `Config::load()` 保留内存里的 last-good；不要直接 `fs::write` 配置，也不要让任何路径在解析失败时静默用默认值
- **运行时状态单一入口**：`hook.rs` 的静态原子变量只能通过 `hook::apply_config(&Config)` 刷新（启动时一次 + 配置轮询线程每 `CONFIG_POLL_MS`=300ms 一次），新增配置字段时必须同步加进 `apply_config`，否则热重载对它无效
- **修饰键不累积**：没有全局 `MOD_STATE`，触发判定与 `trace` / `learn` 展示都用 `keys::live_mods()`（`GetAsyncKeyState` 在判定瞬间现算）
- 守护进程触发时用内存里的配置快照（`hook::current_config()`），不在热路径上重新读文件

## 开发进度 / 开发计划

- 已完成：v1.0.2 —— 配置原子写与损坏保护（不静默回退默认值）、键位热重载（`set` 后 1 秒内生效）、`strict_mods` 精确匹配（`GetAsyncKeyState` 现算，删掉累积状态）
- 下一步：`install` 可靠性（不再靠固定 sleep）→ 运行状态可观测（state.json / `status --json`）→ 长按 / 双击区分与一键多动作
- 待验证：提权窗口（UAC / 管理员程序）前台时 `GetAsyncKeyState` 能否读到修饰键；真机 `F23` 触发链路本轮未复测（本机守护进程仍是 v1.0.1）
- 计划、优先级与验收标准见 [plans/copilotkey.md](../../plans/copilotkey.md)

## 环境依赖

- Rust 2021 edition+
- Windows 10 / 11（x64）
- 无需管理员权限；无需额外系统组件

## 更新历史

### v1.0.2

- 改进：配置原子写 —— `save()` 改为「临时文件 + `sync_all` + 重命名」，并发读写不会读到半个 JSON，写失败也不会留下残缺配置
- 修复：配置解析失败不再静默回退默认值 —— 命令行操作（`target` / `install` / `set` / `learn` / `test`）先把原文件备份成 `config.json.broken-<时间戳>`，再以非零码退出；守护进程保留内存里的 last-good 并往日志告警
- 改进：启用此前闲置的 `version` 字段 —— 旧配置自动迁移到当前结构版本，版本高于本程序时拒绝加载（避免旧版本把新配置覆盖成旧结构）
- 改进：键位真热重载 —— 抽出 `hook::apply_config()` 作为刷新运行时状态的唯一入口，启动与 300ms 配置轮询线程都调用；`set` / `learn --save` 改完 1 秒内生效，不再提示“需要重启”
- 修复：`strict_mods` 恢复真正的精确匹配 —— 判定瞬间用 `GetAsyncKeyState` 现算修饰键、`(mods & 0xF) == (required & 0xF)`，删掉全局 `MOD_STATE`；UAC / 安全桌面期间钩子收不到事件也不会留下脏状态导致之后一直不触发
- 修复：`install` 不再用默认值覆盖已有配置，只在配置文件不存在时写入初始配置
- 文档：README 写清热重载与 `strict_mods` 的实际语义；`status` 在配置损坏时明确提示“正在使用上次有效配置”
- 未真机验证：守护进程内轮询热重载 + `F23` 真实触发链路、提权窗口前台时的修饰键读取

### v1.0.1

- 修复：适配仓库 `Tools/` 新布局 —— 仓库根改为按 `[workspace]` 标记向上探测（exe 路径 → 当前目录 → 已知兜底路径），仓库整体搬家后 `target <工具名>` 不再失效
- 修复：守护进程热重载配置 —— 每次触发重新读取 `config.json`，改完目标 / 参数立即生效，无需重启守护进程
- 改进：`target` 未能把名字解析成已编译 exe 时打印告警，不再静默存下裸名字（按下按键才发现启动失败）

### v1.0.0

- 首个版本：`WH_KEYBOARD_LL` 全局钩子截获 Copilot 键（Win+Shift+F23），吞键并启动自定义目标
- 命令：`trace` / `learn` / `set` / `target` / `run` / `install` / `uninstall` / `status` / `stop` / `test` / `help`
- 注册表 `HKCU\Software\Microsoft\Windows\CurrentVersion\Run` 注册开机自启
