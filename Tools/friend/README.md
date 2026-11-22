# friend - AI 聊天助手

版本: v1.0.1

## 简介

`friend` 是一个终端 AI 聊天助手，你可以自定义接入任意兼容 OpenAI 格式的 AI API（DeepSeek、OpenAI、通义千问等），通过配置人格设定和对话训练，让 AI 越来越像真实的人类朋友。

## 功能

- [x] 自定义 AI API 接入（兼容 OpenAI Chat API 格式）
- [x] 人格设定（系统提示词 + 温度参数）
- [x] 流式输出（实时显示回复）
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
temperature = 0.8         # 回复随机性 (0.0-1.0)

[chat]
auto_save = true           # 自动保存对话
max_context = 30           # 保留的上下文轮数
```

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
- **训练机制是本地拼装**：靠系统提示词 + 历史修正实现人格，不发额外训练请求，改动它时要注意 prompt 体积

## 开发进度 / 开发计划

- 已完成：v1.0.1 —— 自定义 API 接入、人格训练、终端对话、历史会话管理；一次会话只落一个文件，`--list`/`--continue` 预览按字符截断不再 panic，续聊也会带上训练的 `[经验]`
- 下一步：配置缺省值与启动校验（v1.0.2）→ 流式超时/中断（v1.1.0）→ Anthropic API
- 计划、优先级与验收标准见 [plans/friend.md](../../plans/friend.md)

## 项目日志

每次修改本工具都会在同一次提交里更新 [项目日志.md](项目日志.md)。
