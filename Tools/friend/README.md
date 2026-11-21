# friend - AI 聊天助手

版本: v1.0.0

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

## 安装

```bash
cargo install --path friend
```

## 开发规范

通用规范见根目录 [AGENTS.md](../../AGENTS.md)，本工具特有约定：

- **单文件实现**：逻辑在 `src/main.rs`，按「配置加载 / 请求构造 / 回复处理 / 人格训练 / 命令分发」分段
- **依赖克制**：HTTP 用 `ureq`（同步、不引入 tokio），JSON 用 `serde_json`，配置用 `toml`，路径用 `dirs`
- **配置路径固定**：`~/.tao_tools/friend/config.toml`（首次运行生成模板），对话历史落在同目录；路径必须走 `dirs`，不硬编码盘符
- **密钥不入库**：API Key 只从配置文件读，不写进代码，不提交到仓库（根 `.gitignore` 已排除 `API.txt`）
- **训练机制是本地拼装**：靠系统提示词 + 历史修正实现人格，不发额外训练请求，改动它时要注意 prompt 体积

## 开发进度 / 开发计划

- 已完成：v1.0.0 —— 自定义 API 接入、人格训练、终端对话、历史会话管理全部可用
- 下一步：支持 Anthropic API → 对话导出 → 多人格切换
- 计划、优先级与验收标准见 [plans/friend.md](../../plans/friend.md)

## 项目日志

每次修改本工具都会在同一次提交里更新 [项目日志.md](项目日志.md)。
