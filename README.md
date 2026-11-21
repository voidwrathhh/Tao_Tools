# tao_tools

自定义终端工具链。安装后所有工具可直接在任意终端中调用，无需 `cargo run`。

## 目录结构

```
Tao_Tools/
├── Tools/                 # 工具源码与 cargo 工作区（一个工具一个文件夹）
│   ├── Cargo.toml         # workspace 清单：统一管理下面所有工具成员
│   ├── Cargo.lock
│   ├── target/            # 构建产物（已 gitignore）
│   ├── sertool/
│   ├── friend/
│   ├── blueinfo/
│   ├── deskman/
│   ├── copilotkey/
│   └── toolhub/
├── plans/                 # 各工具开发计划（未完成功能、验收标准、分工边界）
├── AGENTS.md              # 开发规范（唯一规范文件）
├── README.md              # 本文件：项目总览
└── 项目日志.md            # 项目级操作与里程碑
```

仓库根目录只放文档与仓库级文件，工具源码和 cargo 清单都在 `Tools/` 下，避免和构建产物、临时文件混在一起。

> cargo 命令在 `Tools/` 里执行（如 `cd Tools && cargo build -p toolhub`）；在仓库根目录执行时加 `--manifest-path Tools/Cargo.toml`。

## 快速开始

```bash
# 安装 sertool 到全局
cargo install --path Tools/sertool

# 任意终端直接使用
sertool list
sertool connect COM3
```

## 工具列表

| 工具 | 版本 | 描述 |
|------|------|------|
| [sertool](Tools/sertool/) | v2.4.0 | 串口终端工具（TUI 界面，含设置界面与配置记忆） |
| [friend](Tools/friend/) | v1.0.0 | AI 聊天助手（自定义 API、人格训练） |
| [blueinfo](Tools/blueinfo/) | v1.1.0 | 通用桌面蓝牙调试工具（GUI 图形界面，含设置界面） |
| [deskman](Tools/deskman/) | v4.3.0 | 桌面接管式图标整理器（真实图标 + 原位还原，含设置界面） |
| [copilotkey](Tools/copilotkey/) | v1.0.1 | Copilot 键改造工具（Windows，把 Copilot 键变成全局功能键） |
| [toolhub](Tools/toolhub/) | v1.0.2 | 工具中心（图形入口：启动工具、调整顺序、直达各工具设置） |

> **注册约定（强制）**：每新增一个工具，都必须在 [Tools/toolhub/registry.json](Tools/toolhub/registry.json) 中登记，否则工具中心里看不到它。字段与流程见 [Tools/toolhub/README.md](Tools/toolhub/README.md)。

## 项目日志

- 各工具开发计划：[plans/](plans/README.md)（未完成功能与分工边界，动手前先读）
- 项目级操作与里程碑：[项目日志.md](项目日志.md)
- 各工具的逐条修改记录：`Tools/<工具名>/项目日志.md`（如 [Tools/sertool/项目日志.md](Tools/sertool/项目日志.md)）
- 开发规范见 [AGENTS.md](AGENTS.md)；每次修改都会同步更新对应日志并推送

## 环境要求

- Rust 2021 edition+
- `~/.cargo/bin/` 已加入系统 PATH（cargo 安装时通常自动配置）

## 安装

```bash
# 安装指定工具
cargo install --path Tools/sertool

# 验证安装
sertool --version
```

安装后对应工具可直接在终端中调用。

### 卸载

```bash
cargo uninstall sertool
```
