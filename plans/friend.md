# friend 开发计划

> 状态：进行中　最后更新：2026-11-22　版本基线：v1.0.0

## 1. 目标

终端 AI 聊天助手：自定义 API、人格设定、对话训练与历史管理，保持单文件实现与同步 HTTP（`ureq`，不引入 tokio）。

## 2. 非目标与分工边界

- **不做 GUI 外壳**：图形入口统一由 [toolhub](toolhub.md) 提供（`friend --edit-config` 直达配置）。
- **不做本地模型推理**：若将来要接本地 LLM，作为独立 sidecar 单独立项，且需用户同意后才动。
- **不托管密钥**：API Key 只从 `~/.tao_tools/friend/config.toml` 读，不写进代码、不落库、不提交仓库。

## 3. 候选功能

### P0 会话身份：autosave 不再把一次聊天拆成 N 个文件 · 本轮执行

- 证据：`src/main.rs:242` 用秒级时间戳作文件名，而 `main.rs:706-708` 每轮回复后都会 `auto_save`；`Conversation`（`main.rs:51-57`）没有 id 字段，`get_latest_conversation`（`main.rs:297-300`）只能凭 `updated_at` 猜。
- 做法：`Conversation` 增加 `#[serde(default)] id: String`；`run_chat` 入口固定一个文件名（created_at/id），autosave 始终覆盖写同一文件；`list_conversations` 按 id 去重取 updated_at 最新；旧格式文件补出 id。
- 验收：连续聊 3 轮后 `friend --list` 只多 1 条；`conversations/` 文件数等于会话数；v1.0.0 旧文件仍能 `--continue` 且内容不丢。
- 版本：patch（v1.0.1）

### P0 预览截断按字符，不再 panic · 本轮执行

- 证据：`main.rs:888-890` 用 `&content[..30]`、`main.rs:977-979` 用 `&content[..60]` 按字节切 UTF-8，首条消息中英混排时切在字符中间 → `byte index 30 is not a char boundary`，`--list`/`--continue` 直接崩。
- 做法：抽 `fn preview(s: &str, n_chars: usize) -> String`（`s.chars().take(n).collect()`），替换两处；不为显示宽度引入新依赖。
- 验收：首条消息为中英混排长文本时 `--list`/`--continue` 退出码 0、截断处无乱码；补单元测试覆盖纯中文/纯英文/中英混排三例。
- 版本：patch（v1.0.1）

### P0 `/train` 纠正在 `--continue` 里失效 · 本轮执行

- 证据：训练注入只写在新会话分支 `main.rs:1011-1027`；`--continue` 分支（`main.rs:943-1003`）只把 `messages[0]` 换成 system prompt，从不读 `training.json`，续聊时训练记录等于白记。另 `handle_command` 形参是 `&mut [Message]`（`main.rs:749`），结构上无法 pop，挡住 `/undo`。
- 做法：抽 `fn build_system_prompt(config) -> String`（人格 + 最近 N 条 `[经验]`），新会话与 `--continue` 共用；`handle_command` 改收 `&mut Vec<Message>`，为 `/undo` 留入口（本轮可不实现 `/undo`）。
- 验收：`/train 应该更温柔` → `/exit` → `--continue` 发问，抓请求体可见该条 `[经验]` 在 system 内；新会话行为不变。
- 版本：patch（v1.0.1）

### P1 配置缺省值与启动校验

- 证据：`Config`/`ApiConfig`/`PersonalityConfig`/`ChatConfig`（`main.rs:17-43`）全字段必填、无 `#[serde(default)]`；`main.rs:188-192` 解析失败只打一行 toml 报错就退出；`key` 为空仍带 `Bearer ` 发请求，直到 401 才暴露。
- 做法：给 `greeting`/`temperature`/`auto_save`/`max_context` 等加 `#[serde(default = "...")]`；解析失败提示具体缺失项；启动前校验 key 非空、endpoint 以 `http(s)://` 开头；配置写入改「临时文件 + 原子替换」。
- 验收：删掉 `[chat]` 整段仍能启动并落默认值；key 为空时首条消息发出前就提示；写坏配置后退出码 1 且原文件字节不变。
- 版本：patch（v1.0.2）

### P1 流式请求超时、可中断、SSE 容错

- 证据：`main.rs:540` 直接 `ureq::post` 无 Agent 级 timeout；`main.rs:576-599` 只取 `choices[0].delta.content`，丢弃 `reasoning_content`/`finish_reason`，只匹配 `"data: "`（带空格）；首行不匹配就报「未知的响应格式」（`main.rs:624`）；流中 Ctrl+C 直接杀进程，半截回复不落盘。
- 做法：复用 `ureq::AgentBuilder`，`timeout_connect(10s)` / `timeout_read(60s)`；SSE 解析跳过空行与 `:` 注释、`data:` 后空格可选；`reasoning_content` 用暗色单独输出；Ctrl+C 置 `AtomicBool` 收尾保存（引入 `ctrlc` 需同意）。
- 验收：endpoint 指向只连不回的端口，60s 内给中文超时提示；`data:{...}`（无空格）可解析；流中 Ctrl+C 后 `--continue` 能看到已收部分。
- 版本：minor（v1.1.0）

### P2 支持非 OpenAI 格式 API（Anthropic）

- 验收：`[api]` 加 `provider = "openai" | "anthropic"`（缺省 openai）；Anthropic 走 `/v1/messages`，带 `x-api-key` 与 `anthropic-version`，SSE 解析独立分支；鉴权失败/限流给中文提示。
- 备注：不要把 Anthropic 的 SSE 硬塞进 OpenAI 解析器，两条分支各自解析，共用「拼装上下文 / 落盘」。
- 版本：minor

### P2 对话导出（TXT / Markdown）

- 验收：`friend --export <会话id|latest> [--format md|txt]`，导出到 `~/.tao_tools/friend/exports/`，文件带人格名与时间；导出前后 `messages` 与 `training.json` 字节不变。

### P2 多人格配置快速切换

- 验收：`[personalities.<名字>]` 多段 + `--persona <名字>` 与 `/persona <名字>`；未指定时沿用 `[personality]`（向后兼容）。

### P2 更精细的训练参数调节

- 验收：训练反馈可配权重 / 单次注入条数上限 / 温度偏移；默认值与现行为一致。

## 4. 版本规划

| 版本 | 内容 |
|---|---|
| v1.0.1 | P0 会话身份 + 预览 panic + `/train` 续聊 |
| v1.0.2 | P1 配置缺省与校验 |
| v1.1.0 | P1 流式超时/中断/SSE 容错 |
| v1.2.0 | P2 Anthropic API |
| v1.3.0 | P2 对话导出 / 多人格 |
| v1.4.0 | P2 训练参数细化 |

## 5. 风险与未决问题

- Anthropic 与 OpenAI 的流式事件差异较大（`content_block_delta` vs `choices[].delta`），解析分支要独立测试。
- 训练反馈靠系统提示词拼装，条数增多会挤占上下文（现 `max_context = 30`），导出与多人格都不能放大 prompt。
- `#[serde(default)]` 会让"字段名写错"从报错变成静默用默认值，需权衡是否保留严格模式。
- 中断保存要处理：只保存已完整收到的内容，半截 JSON 行不能写进历史。
