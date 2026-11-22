# friend - AI 聊天助手

版本: v1.1.0

## 简介

`friend` 是一个终端 AI 聊天助手，你可以自定义接入任意兼容 OpenAI 格式的 AI API（DeepSeek、OpenAI、通义千问等），通过配置人格设定和对话训练，让 AI 越来越像真实的人类朋友。

## 功能

- [x] 自定义 AI API 接入（兼容 OpenAI Chat API 格式）
- [x] 人格设定（系统提示词 + 温度参数）
- [x] 流式输出（实时显示回复）
- [x] SSE 容错：`data:{...}` 无空格可解析，空行 / `:` 注释 / `event:` 等字段跳过，坏 JSON 块跳过，`reasoning_content` 暗色单独显示
- [x] 请求超时（连接 10 秒 / 读取 60 秒）与中文超时提示
- [x] 回复中 Ctrl+C 中断，已收到的内容保留并落盘
- [x] 配置容错：`[personality]` / `[chat]` 整段可省（自动落默认值），key / endpoint 有问题在首条消息发出前就提示，配置按「临时文件 + 改名替换」原子写入
- [x] 对话历史保存/继续
- [x] 训练反馈（记录纠正数据，逐步改进回复风格）
- [x] 命令行交互（快捷键和命令）

## 使用方法

### 首次配置

```bash
# 运行一次以生成配置模板
friend

# 编辑配置文件，填入 API 密钥和人格设定
# 配置文件路径: ~/.tao_tools/friend/config.toml
friend --edit-config
```

### 配置说明

```toml
[api]
endpoint = "https://api.deepseek.com/v1/chat/completions"  # API 地址
key = "sk-xxxxxxxxxxxx"                                     # API 密钥
model = "deepseek-chat"                                     # 模型名称

[personality]
name = "小美"              # AI 角色名称
greeting = "你好呀～"       # 开场问候语（可留空）
system_prompt = """
你是一个温暖真诚的朋友...
"""
temperature = 0.8         # 回复随机性 (0.0-2.0)

[chat]
auto_save = true           # 自动保存对话
max_context = 30           # 保留的上下文轮数
```

`[personality]`、`[chat]` 两段可以整段删掉，缺的字段自动落默认值：`name = "朋友"`、`greeting = ""`、`temperature = 0.8`、`auto_save = true`、`max_context = 30`、`model = "gpt-4o-mini"`。`[api] key` 留空或 `endpoint` 不是 `http(s)://` 开头时，启动就会给中文提示并退出（不会等到 401 才发现）；配置解析失败会打印出错的行号、列号与原文，并且原配置文件字节不变。

### 运行

```bash
# 开始新对话
friend

# 继续上次对话
friend --continue

# 列出历史对话
friend --list

# 编辑配置
friend --edit-config
```

### 聊天命令

| 命令 | 说明 |
|------|------|
| `/exit` | 退出对话 |
| `/save` | 手动保存 |
| `/clear` | 清屏 |
| `/train <反馈>` | 纠正上一条回复，记录训练数据 |
| `/help` | 显示帮助 |

### 交互说明

- **Ctrl+C 中断**：在 AI 回复过程中按 Ctrl+C，会中止本次回复，已经收到的字符会保留并立即保存（即使 `auto_save = false`），随后回到「你:」提示符；之后 `friend --continue` 能看到这段半截回复。不在回复期间时，Ctrl+C 仍是系统默认行为（直接退出进程）。
- **中断生效时机**：流式读取每收到一行数据前都会检查中断标志，所以正常逐字输出时按下立即生效；若服务端中途长时间不发数据，最多要等到下一块数据或 60 秒读取超时才生效（同步 HTTP 无法打断阻塞中的 socket 读，这是刻意取舍，不引入线程池/tokio）。
- **超时默认值**：连接 10 秒、单次读取 60 秒（`CONNECT_TIMEOUT_SECS` / `READ_TIMEOUT_SECS`），服务端 60 秒没有新数据会打印中文超时提示并回到提示符，不会卡住整个进程。
- **SSE 容错**：`data:{...}`（无空格）也能识别，空行 / `:` 开头的注释 / `event:`、`id:` 等字段跳过，单个坏 JSON 块跳过不影响整段回复，`finish_reason` 与 `data: [DONE]` 都作为结束信号；`reasoning_content` 用暗灰色单独输出。

### 训练机制

使用 `/train` 命令可以让 AI 从反馈中学习：

```
你: 今天心情不太好
AI: 哦，发生什么事了？

你: /train 应该更关心一点，先问问怎么了再回应
💡 已记录这条反馈，我会记住的！
```

下次对话时，AI 会参考这些训练记录调整回复风格。

## 环境依赖

- Rust 2021 edition+
- 一个兼容 OpenAI Chat API 的 AI 服务（DeepSeek、OpenAI、Groq 等）
- Windows + MSVC 工具链下，`ring`（`ureq` 的 TLS 依赖）**首次从零编译**时要求 cl.exe 能写 `%TEMP%`

### Windows/MSVC：ring 编译报 D8050

从零编译 `ring` 时可能失败，cl.exe 报：

```
cl : Command line error D8050 : cannot execute '...\c1.dll': failed to get command line into debug records
```

原因：cc-rs 给 cl 传 `/Z7`（debuginfo）并配合 `/Brepro`，`/Z7` 下 cl 需要往 `%TEMP%` 写临时文件；一旦构建子进程写不了 `%TEMP%`（受限令牌 / 低完整性沙箱、`TEMP` 指向不存在或只读目录、路径含非 ASCII 等），cl 就以 D8050 报出来。**这不是 MSVC 组件损坏、ring 版本或本工具源码的问题**——同一条 cl 命令行在普通终端里能正常编过，只有由 cargo/cc-rs 拉起时才失败。

实测有效的绕开方式（任选其一，都不改源码、不动 workspace 清单）：

| 方式 | 命令（在 `Tools/` 下） | 说明 |
|---|---|---|
| release 构建 | `cargo build -p friend --release` | 无 debuginfo，不触发 `/Z7`，最省事 |
| 关掉 dev profile 的 debuginfo | `CARGO_PROFILE_DEV_DEBUG=0 cargo build -p friend` | dev 构建仍可用，只是无调试信息 |
| 把临时目录指到可写位置 | `mkdir .tmp-build; $env:TEMP=$env:TMP=(Resolve-Path .tmp-build).Path; cargo build -p friend` | 想保留 dev + debuginfo 时用；目录要**先建好**且位于仓库内（如 `Tools/.tmp-build`）才一定可写 |

注意：`RUSTFLAGS='-C debuginfo=0'` **无效**。ring 的 C 编译看的是 cargo 传给构建脚本的 `DEBUG` 环境变量（profile 级），与 rustc 的 debuginfo 选项无关，`/Z7` 照样会加上，仍报 D8050（且它作用于全部 crate，不是好开关）。

普通（Medium 完整性）终端一般不会遇到此问题，多见于 IDE / CI / 受限令牌的沙箱子进程（例如 DSH 给子会话的沙箱策略）下调起的构建。

## 安装

```bash
# 在 Tools/ 目录下执行
cargo install --path friend
```

## 开发规范

通用规范见根目录 [AGENTS.md](../../AGENTS.md)，本工具特有约定：

- **单文件实现**：逻辑在 `src/main.rs`，按「配置加载 / 请求构造 / 回复处理 / 人格训练 / 命令分发」分段
- **依赖克制**：HTTP 用 `ureq`（同步、不引入 tokio），JSON 用 `serde_json`，配置用 `toml`，路径用 `dirs`
- **配置路径固定**：`~/.tao_tools/friend/config.toml`（首次运行生成模板），对话历史落在同目录；路径必须走 `dirs`，不硬编码盘符
- **会话一文件**：会话落盘为 `conversations/<id>.json`，`id` = 创建时刻 + 进程号；自动保存始终覆盖同一个文件，`--list` 按 `id` 去重，v1.0.0 的旧文件在加载时用文件名补齐 `id`
- **密钥不入库**：API Key 只从配置文件读，不写进代码，不提交到仓库（根 `.gitignore` 已排除 `API.txt`）
- **超时与中断**：请求统一走 `chat_agent()`（`timeout_connect(10s)` / `timeout_read(60s)`）；Ctrl+C 只在流式读取期间用 kernel32 `SetConsoleCtrlHandler` 接管（`AtomicBool` 置位，流式循环每读一行前 `interrupt::take()` 检查），退出流式立即卸载 handler，平时保持系统默认
- **配置写入是原子替换**：写 `config.toml.tmp` 再 rename；缺段/缺字段由 `#[serde(default = ...)]` 补默认值，启动校验集中在 `config_problems()`（key 空、endpoint 非 http(s)、model 空、temperature 越界、max_context 为 0）
- **SSE 解析纯手写**：`sse_data` 抽行、`stream_sse` 逐行消费，坏块跳过；为了不引新 crate，没有用事件流库或 tokio
- **训练机制是本地拼装**：靠系统提示词 + 历史修正实现人格，不发额外训练请求，改动它时要注意 prompt 体积

## 开发进度 / 开发计划

- 已完成：v1.1.0 —— 在 v1.0.1 基础上补齐配置缺省值与启动校验（含配置原子写入）、流式请求超时 / 可中断 / SSE 容错
- 下一步：Anthropic API（v1.2.0）→ 对话导出 / 多人格（v1.3.0）→ 训练参数细化（v1.4.0）
- 计划、优先级与验收标准见 [plans/friend.md](../../plans/friend.md)

## 项目日志

每次修改本工具都会在同一次提交里更新 [项目日志.md](项目日志.md)。
