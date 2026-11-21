# friend 开发计划

> 状态：规划中　最后更新：2026-11-21　版本基线：v1.0.0

## 1. 目标

终端 AI 聊天助手：自定义 API、人格设定、对话训练与历史管理，保持单文件实现与同步 HTTP（`ureq`，不引入 tokio）。

## 2. 非目标与分工边界

- **不做 GUI 外壳**：图形入口统一由 [toolhub](toolhub.md) 提供（`friend --edit-config` 直达配置）。
- **不做本地模型推理**：若将来要接本地 LLM，作为独立 sidecar 单独立项，且需用户同意后才动。
- **不托管密钥**：API Key 只从 `~/.tao_tools/friend/config.toml` 读，不写进代码、不落库、不提交仓库。

## 3. 候选功能

### P0 支持非 OpenAI 格式 API（Anthropic）

- 验收：`config.toml` 的 `[api]` 增加 `provider = "openai" | "anthropic"`（缺省 openai）；Anthropic 走 `/v1/messages`，带 `x-api-key` 与 `anthropic-version`，流式 SSE 解析独立分支；鉴权失败/限流给出中文提示，不 panic。
- 备注：不要把 Anthropic 的 SSE 事件格式硬塞进 OpenAI 解析器；两条分支各自解析，共用「拼装上下文 / 落盘」部分。

### P1 对话导出（TXT / Markdown）

- 验收：`friend --export <会话id|latest> [--format md|txt]`，导出到 `~/.tao_tools/friend/exports/`，文件带人格名与时间；不改变现有对话历史格式。

### P1 多人格配置快速切换

- 验收：`config.toml` 支持 `[personalities.<名字>]` 多段 + `--persona <名字>` 启动指定人格、`/persona <名字>` 会话内切换；未指定时沿用现有 `[personality]` 行为（向后兼容）。

### P2 更精细的训练参数调节

- 验收：训练反馈可配置权重 / 单次注入条数上限 / 温度偏移，避免 prompt 体积失控；默认值与现行为一致。

## 4. 版本规划

| 版本 | 内容 |
|---|---|
| v1.1.0 | P0 Anthropic API |
| v1.2.0 | P1 对话导出 |
| v1.3.0 | P1 多人格配置 |
| v1.4.0 | P2 训练参数细化 |

## 5. 风险与未决问题

- Anthropic 与 OpenAI 的流式事件差异较大（`content_block_delta` vs `choices[].delta`），解析分支要独立测试，避免同步阻塞。
- 训练反馈靠系统提示词拼装，条数增多会挤占上下文（现 `max_context = 30`），导出与多人格都不能放大 prompt。
- 多人格改配置结构会动到现有 `[personality]` 解析，必须做旧配置兼容。
