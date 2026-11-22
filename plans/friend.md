# friend 开发计划

> 状态：进行中　最后更新：2026-11-22　版本基线：v1.0.1

## 1. 目标

终端 AI 聊天助手：自定义 API、人格设定、对话训练与历史管理，保持单文件实现与同步 HTTP（`ureq`，不引入 tokio）。

## 2. 非目标与分工边界

- **不做 GUI 外壳**：图形入口统一由 [toolhub](toolhub.md) 提供（`friend --edit-config` 直达配置）。
- **不做本地模型推理**：若将来要接本地 LLM，作为独立 sidecar 单独立项，且需用户同意后才动。
- **不托管密钥**：API Key 只从 `~/.tao_tools/friend/config.toml` 读，不写进代码、不落库、不提交仓库。

## 3. 候选功能

### P1 配置缺省值与启动校验

- 证据：`Config`/`ApiConfig`/`PersonalityConfig`/`ChatConfig`（`main.rs:19-44`）全字段必填、无 `#[serde(default)]`；`main.rs:192-197` 解析失败只打一行 toml 报错就退出；`key` 为空仍带 `Bearer ` 发请求，直到 401 才暴露。
- 做法：给 `greeting`/`temperature`/`auto_save`/`max_context` 等加 `#[serde(default = "...")]`；解析失败提示具体缺失项；启动前校验 key 非空、endpoint 以 `http(s)://` 开头；配置写入改「临时文件 + 原子替换」。
- 验收：删掉 `[chat]` 整段仍能启动并落默认值；key 为空时首条消息发出前就提示；写坏配置后退出码 1 且原文件字节不变。
- 版本：patch（v1.0.2）

### P1 流式请求超时、可中断、SSE 容错

- 证据：`main.rs:615` 直接 `ureq::post` 无 Agent 级 timeout；`main.rs:650-676` 只取 `choices[0].delta.content`，丢弃 `reasoning_content`/`finish_reason`，只匹配 `"data: "`（带空格）；首行不匹配就报「未知的响应格式」（`main.rs:699`）；流中 Ctrl+C 直接杀进程，半截回复不落盘。
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
