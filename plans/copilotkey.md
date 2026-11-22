# copilotkey 开发计划

> 状态：进行中　最后更新：2026-11-22　版本基线：v1.0.2

## 1. 目标

把 Windows Copilot 键（`Win+Shift+F23`）改成自定义全局功能键：低级键盘钩子吞键 + 启动目标 + 开机自启，无驱动、无管理员权限。

## 2. 非目标与分工边界

- 只做键盘钩子与目标启动；不做通用快捷键管理器（不注册任意热键）。
- **「多套配置快速切换」与 toolhub 的「启动参数模板」是两件事**：前者切键位/目标配置（本工具），后者切某工具的命令行参数（[toolhub](toolhub.md)），不要混做。
- 图形化配置界面是否做未决（见 P2），若做优先独立窗口；不因 GUI 引入常驻重依赖。

## 3. 候选功能

### P1 `install` 不再靠固定 sleep 假装成功

- 证据：`src/main.rs:305-322` 是 `stop()` → sleep 400ms → `spawn_daemon` → sleep 900ms → `is_running()`；旧进程没退完时新进程在 `hook.rs:178-181` 撞 `ERROR_ALREADY_EXISTS` 直接 `return 0` 静默退出，而 `is_running()`（`hook.rs:144-153`）因旧进程仍持锁返回 true → 打印「守护进程已启动」，随后钩子彻底消失；`spawn_daemon`（`main.rs:407-419`）用 `.map(|_| ())` 丢掉子进程句柄，启动失败无从察觉。
- 做法：stop 后轮询 `!is_running()`（≤3s，超时提示 taskkill）；spawn 保留 `Child`，轮询 `is_running()`（≤5s）+ `child.try_wait()` 双重判定，提前退出则打印退出码与 `tail_log`；`run` 撞已存在时返回非零码。
- 验收：连续执行 20 次 `install` 均真正生效且每次 pid/启动时间都变；手动 kill 守护进程后 `install` 非零码退出并给出原因。
- 版本：patch

### P1 运行态可观测（state.json + `test` 不再假阳性）

- 证据：`src/hook.rs:217` 失败只 `util::log`，而 `util.rs:131-135` 在 `log=false` 时连日志都不写，release 构建是 `windows_subsystem = "windows"` 无控制台；`src/main.rs:389,392-396` 的 `test` 只 `SendInput` 一次就打印「已模拟按下」，但 `hook.rs:96-97` 在 `ignore_injected` 时丢弃注入键 → 什么都没发生却报成功；`status`（`main.rs:353-378`）只有 `tail_log(5)`。
- 做法：守护进程把 pid/启动时间/上次触发时间/目标/结果/累计次数原子写进 `state.json`，`status` 读它；`test` 先同步验证目标可启动，再注入验证钩子通路，并在 `ignore_injected=true` 时明确警告。
- 验收：target 设为不存在路径后按触发键 → `status` 显示「上次触发 失败：ShellExecuteEx 错误码 2」；`log=false` 时 `status` 仍可见上次结果；`test` 在注入被忽略时给出警告但仍验证目标启动。
- 版本：minor

### P2 长按 / 双击与单次按下区分

- 验收：配置新增 `long_press_ms`、`double_press_ms`；单次、长按、双击可分别绑定不同 target；未触发时按键放行；长按判定总延迟有可测上限（≤120ms），不吞短按。

### P2 一键多动作（宏）

- 验收：`target` 支持按顺序启动多个目标，动作间隔可配；任一动作失败不阻断后续，日志记录每步结果与退出码。

### P2 多套配置快速切换

- 验收：`copilotkey profile use <名字>` 在多份 profile 间切换；`profile list` 列出；切换不要求重启守护进程（依赖 v1.0.2 的键位热重载）。

### P2 图形化配置界面（未决）

- 未决：独立窗口（推荐，钩子常驻态需要） vs 由 toolhub 直达一个 `settings` 窗口。**先定方案再动工**，避免两处各做一半。
- 反向参考：`eframe` 会显著增大体积，而本工具当前是轻量 CLI。

### P2 不同品牌笔记本 Copilot 键上报差异

- 验收：用 `trace` 在至少两种机型上确认上报（`F23` / `Win+Shift+F23` / 其它），在 README 记录结论；`learn --save` 对两种都能用。

## 4. 版本规划

| 版本 | 内容 |
|---|---|
| v1.1.0 | P1 install 可靠性 + 运行态可观测 |
| v1.2.0 | P2 长按/双击 + 宏 |
| v1.3.0 | P2 多套配置；图形界面（若确认做） |

## 5. 风险与未决问题

- 长按/双击判定天然引入延迟，可能吞掉短按；需明确「延迟上限」并在文档说明。
- 宏并发启动的顺序、工作目录、失败回滚都要先定义清楚。
- 图形界面若引入 GUI 框架，需要评估对「无管理员权限、开机自启轻量」的影响。
- 精确修饰键依赖 `GetAsyncKeyState` 的实时性；安全桌面/提权前台是已知盲区（UAC 期间低级钩子收不到事件）。v1.0.2 已改成判定瞬间现算、不再累积状态，「修饰键残留导致之后一直不触发」的问题消除；但「提权前台时能否读到修饰键」仍需真机实测，不稳则退回「钩子空闲 N 秒自动清零」。
- v1.0.2 起配置损坏会生成 `config.json.broken-<时间戳>` 且不自动清理；保留份数与是否提供 `config --repair` / `--restore` 待决。
