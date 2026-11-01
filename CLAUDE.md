# tao_tools 开发规范

## 项目结构
- 每个工具位于根目录下的独立文件夹，文件夹名即工具名
- 每个工具有独立的 Cargo.toml 和 README.md
- 根目录 Cargo.toml 以 workspace 管理所有工具成员

## 版本规范
每个工具独立版本号，格式 `v{major}.{minor}.{patch}`
- **major**: 加入重大新功能或逻辑重构时 +1
- **minor**: 功能优化/增强时 +1，满10进一（即 minor 到10时 major+1, minor 归零）
- **patch**: Bug 修复

## 开发语言
- Rust 2021 edition

## 代码风格
- 命令行参数解析：手动 match 匹配，不引入 clap 等 CLI 框架（除非必要）
- 错误处理：用户可见错误用 `unwrap_or_else` + `eprintln!` + `process::exit(1)`
- 代码标识符使用英文，面向用户的文本（帮助信息、README 等）使用中文

## 文档规范
- 每个工具 README.md 必须包含：简介、版本、功能列表（标注 [x] 完成状态）、使用方法、环境依赖、开发计划
- README 使用中文

## 交互模式约定
- 数据接收线程独立运行，实时输出到 stdout
- 主线程负责用户输入和串口写入
- Ctrl+C 用于安全断开连接
