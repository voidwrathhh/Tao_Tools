# toolhub 开发计划

> 状态：进行中　最后更新：2026-11-22　版本基线：v1.0.2

## 1. 目标

工具中心入口与注册表：列表展示已注册工具、启动、直达各工具设置、排序过滤。`registry.json` 是唯一数据源。

## 2. 非目标与分工边界

- **不代管各工具的业务设置**：只提供 `settings` 直达入口，具体设置界面属于各工具。
- **不静默写注册表**：自动发现未注册工具只做提示，补登记仍由人确认（注册表是约定，不能被自动改写）。
- 不做插件/脚本市场之类的扩展体系。
- 进程管理只负责"自己拉起的实例"（记录 pid、结束该 pid），不按名字杀进程，不接管各工具自身单例逻辑。

## 3. 候选功能

### P0 注册表加载期校验 + 加载错误在界面可见 · 本轮执行

- 证据：`src/registry.rs:93-97` 的 `RegistryFile` 只声明 `tools`，`registry.json:2` 的 `"schema": 1` 被静默忽略；`ToolEntry`（`registry.rs:14-40`）无 `deny_unknown_fields`，字段名拼错（如 `"lauch"`）不报错只退化为空；重复 id、空 id、kind 白名单均不检查，未知 kind 在 `gui.rs:538-542` 一律显示成 CLI；读取失败仅 `registry.rs:120` eprintln，GUI 里看不到，用户只看到「工具少了」。
- 做法：`Registry` 增加 `problems: Vec<Problem>`，加载时校验 `schema==1`、id 非空不重复、`kind ∈ {gui,tui,cli}`、`readme` 文件存在；问题随 Registry 带出，在状态栏与对应行标注（对照 README 字段表）；软提示，不阻塞其余工具启动。
- 验收：造三份坏注册表（`schema:2` / 重复 id / `"lauch"` 拼错）分别启动，顶部出现可读错误且其余工具仍能启动；`cargo test -p toolhub` 新增三例覆盖。
- 版本：minor（v1.1.0）

### P0 可执行文件解析结果按需缓存（现在每帧读盘）· 本轮执行

- 证据：`gui.rs:380` 在 `tool_row` 内调 `launcher::describe`，而 `tool_row` 每帧对每个可见工具执行；`describe`→`resolve`（`launcher.rs:17-49`）每次做 `config.paths` 判存、两次 `workspace_root()`（`launcher.rs:26,48`）、`~/.cargo/bin` 与整条 PATH 的 `is_file` 探测；`workspace_root`（`registry.rs:163-203`）逐级向上，每层读两次 `Cargo.toml`（`registry.rs:199-203`）。
- 做法：`HubApp` 内缓存 id→解析结果，仅在 new/reload、`config.paths` 变更、启动失败时失效；`entries` 不再每帧 `tool.clone()`（`gui.rs:254-260`），改借用 + 行内取 `&ToolEntry`。
- 验收：加渲染计数开关，连续 300 帧后 `resolve` 调用次数为常数（≈工具数）；滚动时 CPU 明显下降；`toolhub list` 输出不变。
- 版本：minor（v1.1.0）
- 待决：缓存会让「刚 build 完未刷新」看到旧 exe（现行为实时）；复用现有刷新按钮（`gui.rs:351-352`）失效，需确认可接受。

### P0 自身行禁止启动（README 承诺的「不会递归启动」并未实现）· 本轮执行

- 证据：README 写「列表里会显示自己这一行，不会递归启动」，但 `src` 全量检索 `toolhub` 只命中路径与文案，没有任何针对自身 id 的判断；`launcher::launch` 只要 spawn 成功就返回「已启动」（`launcher.rs:88-91`、`108-113`）。
- 做法：`id == "toolhub"` 的行禁用启动按钮并给 tooltip；启动前若同 id 已在运行则提示而非再开（记录 `HashMap<id, Child>`，用 `try_wait` 清理）；结束进程只允许杀自己 spawn 的 pid。
- 验收：toolhub 自身行启动按钮禁用并有说明；连点两次 sertool，第二次在状态栏提示「已在运行」；任务管理器 kill 后刷新即消失「运行中」。
- 版本：minor（v1.1.0）
- 待决：GUI 工具多开是合理需求，「禁止重复」做成开关还是硬规则需拍板（建议默认提示、可在设置里允许）。

### P1 控制台工具启动参数不要拼成一整行交给 cmd

- 证据：`launcher.rs:72-86` 的 console 分支把 exe 与 args 经 `quote_arg` 拼成单行塞给 `cmd /K`，参数含 `&` `|` `^` `%VAR%` 会被 cmd 二次解释；`quote_arg`（`launcher.rs:143-152`）用 `\"` 转义，与 cmd 的 `""` 规则不符，带引号的路径会启动失败。
- 做法：Exe 分支统一 `Command::new(&path).args(args)` + `creation_flags(CREATE_NEW_CONSOLE)`（`attach_console` 已在 `launcher.rs:168-176` 提供）；确需保留窗口现场改用 `cmd /C "<exe>" ... & pause`，参数仍逐个 arg 传。
- 验收：临时给 sertool 的 launch 加 `["a&b","c d","e\"f"]`，用回显 argv 的程序验证三项原样到达。
- 版本：patch
- 待决：去掉 `cmd /K` 后 sertool/friend 退出即关窗；需在「保留现场」与「参数绝对安全」间拍板（或接受 `pause` 方案的歧义）。

### P1 工具健康检查（版本不一致 / 可执行文件缺失）

- 验收：列表里标出「registry 版本 ≠ Cargo.toml 版本」「可执行文件解析失败」两类问题，给出原因与修复提示；不阻塞正常启动。补：注册表字段级校验（见 P0）与之分开登记，不混为一谈。
- 备注：Cargo.toml 版本可用 `cargo metadata` 的 JSON 输出解析（已有 `serde_json`），避免手写 TOML 解析。

### P1 从 `cargo metadata` 自动发现未注册工具并提示

- 验收：读取 `Tools/Cargo.toml` 的 workspace 成员，列出未在 `registry.json` 中的包，给出一键跳转到「注册新工具」说明；不自动写入。

### P2 收藏 / 置顶分组

- 验收：可置顶/分组，显示在列表顶部；状态写本地 `~/.tao_tools/toolhub/config.json`，不改 `registry.json`。

### P2 启动参数模板

- 验收：同一工具可保存多套启动参数，启动时选择；只存本地配置。
- 与 [copilotkey](copilotkey.md) 的「多套配置切换」区分：那边切键位配置，这边切命令行参数。

## 4. 版本规划

| 版本 | 内容 |
|---|---|
| v1.1.0 | P0 注册表校验 + 解析缓存 + 自身禁启动 |
| v1.2.0 | P1 启动参数安全 + 健康检查 |
| v1.3.0 | P1 自动发现未注册工具 |
| v1.4.0 | P2 收藏/置顶 + 参数模板 |

## 5. 风险与未决问题

- 版本比对需要读各工具 `Cargo.toml` 或 `cargo metadata`；后者启动开销大（可能数秒），要缓存并异步刷新。
- 自动发现要求 `cargo` 在 PATH 中，`cargo install` 后离开仓库的场景会失效，需降级为「仅提示不可用」。
- 收藏与排序（现有 `order`）语义要合并清楚，避免两套顺序机制冲突。
- 过滤态下的「上移/下移」与绝对下标错位（见 `gui.rs:442/449/143`），若本轮不动排序，先在计划里保留，等真要做时按可见序列重排。
