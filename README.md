# tao_tools

自定义终端工具链。安装后所有工具可直接在任意终端中调用，无需 `cargo run`。

## 快速开始

```bash
# 安装 sertool 到全局
cargo install --path sertool

# 任意终端直接使用
sertool list
sertool connect COM3
```

## 工具列表

| 工具 | 版本 | 描述 |
|------|------|------|
| [sertool](sertool/) | v2.3.0 | 串口终端工具（TUI 界面） |
| [friend](friend/) | v1.0.0 | AI 聊天助手（自定义 API、人格训练） |
| [blueinfo](blueinfo/) | v1.0.0 | 通用桌面蓝牙调试工具（GUI 图形界面） |
| [deskman](deskman/) | v4.3.0 | 桌面接管式图标整理器（真实图标 + 原位还原） |
| [copilotkey](copilotkey/) | v1.0.0 | Copilot 键改造工具（Windows，把 Copilot 键变成全局功能键） |
| [toolhub](toolhub/) | v1.0.0 | 工具中心（图形入口：启动工具、调整顺序、直达 README） |

## 项目日志

- 项目级操作与里程碑：[项目日志.md](项目日志.md)
- 各工具的逐条修改记录：`<工具名>/项目日志.md`（如 [sertool/项目日志.md](sertool/项目日志.md)）
- 开发规范见 [AGENTS.md](AGENTS.md)；每次修改都会同步更新对应日志并推送

## 环境要求

- Rust 2021 edition+
- `~/.cargo/bin/` 已加入系统 PATH（cargo 安装时通常自动配置）

## 安装

```bash
# 安装指定工具
cargo install --path sertool

# 验证安装
sertool --version
```

安装后对应工具可直接在终端中调用。

### 卸载

```bash
cargo uninstall sertool
```
