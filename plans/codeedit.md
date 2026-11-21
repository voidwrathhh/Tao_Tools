# codeedit 开发计划

> 状态：已决策，待动工　最后更新：2026-11-21　版本基线：v0.0.0（未发布）
> 工具名：codeedit（包名 `makepad-codeedit`），**暂未注册到 `Tools/`**；落地时按根 `AGENTS.md` 规范接入（workspace 成员、`registry.json`、根 README、项目日志）。
> 开发位置：`D:\File_Data\Tao_Tools\参考项目\makepad-dev\makepad-dev\apps\codeedit`（Makepad 源码副本内，不进本仓库）。

## 1. 目标

以 Makepad Director 源码为基础改造一个独立代码编辑器工具（方案 C：在 Makepad 仓库内原地开发，框架走同 workspace 的 `path` 依赖，零 vendoring），并按 tao_tools 规范改造：手写参数解析、统一错误处理、中文 README、独立版本号。

## 2. 非目标与分工边界

- **不做 AI 功能**：去掉 F10 AI 助手、`aichat` 的 `engine` feature、`ai-cuda` / `ai-metal` / `ai-llm` 等依赖（已决策）。
- 不把 Makepad 框架复制进本仓库（会撑爆仓库体积）。
- 不引入 clap 等 CLI 框架；错误处理沿用 AGENTS.md 的 `unwrap_or_else` + `eprintln!` + `exit(1)`。

## 3. 候选功能（未完成项）

### 阶段 1（收尾）
- [ ] `--hot` 验证 Splash 热重载可用

### 阶段 2：复制 Director 源码
- [ ] 复制 `apps/director/src/` 全部 52 个文件到新 crate（43,768 行）
- [ ] 复制 `apps/director/Cargo.toml` 依赖段（全部 `path` 依赖，同 workspace 直接可用）
- [ ] 处理 `write_widget` / 模块路径引用，逐个消解编译错误
- [ ] 剔除 AI 相关依赖与 F10 入口、`ai_slot` 挂载点、usage 面板里的 provider 查询
- [ ] 验证：`cargo tree -p <包名> | grep -c ai-` 应为 0
- [ ] **决定三个辅助 bin 去留**：`director-git-guard` / `director-rustc-guard` / `director-flow`（原本服务于 Director 的 agent 流程）
- [ ] 构建 `makepad-screen.exe`（实测必需：`cargo build --release -p makepad-screen -p <包名>`，仅编 lib 不会产出该 bin）

### 阶段 3：按规范改造
- [ ] 参数解析改手写 `match`（现参数：`--cwd` / `--size` / `--state-dir` / `--remote`）
- [ ] `--state-dir` 默认从 `~/.makepad/studio` 改为本工具目录，避免与 Director 状态互相污染
- [ ] 用户可见错误统一 `unwrap_or_else` + `eprintln!` + `process::exit(1)`
- [ ] 中文 README（六项齐全）+ 中文用户文本
- [ ] 打版本 `v0.1.0`

### 阶段 4：验收
- [ ] `cargo build -p makepad-codeedit --release` 零 error 零 warning
- [ ] 完整 IDE 界面可启动（Structured / Tasks / Architecture / Disk 四模式）
- [ ] Architecture 模式能渲染 `arch/<crate>/crate.toml` 代码地图（hover / click / 拖拽 / 缩放 / F 键）
- [ ] 能打开、编辑、保存文件；中文路径与中文内容正常显示（依赖已验证的字体修复）
- [ ] `--hot` 下改 Splash 源码即时生效
- [ ] 中文 README 六项齐全；`cargo test -p makepad-codeedit` 通过

## 4. 版本规划

| 版本 | 内容 |
|---|---|
| v0.1.0 | 完整 Director 功能移植 + 规范改造完成 |
| v0.2.0 | 参数与状态目录独立，去掉 Director 专属 agent flow |
| v0.3.0 | 针对 tao_tools 工具链 / 项目结构做定制 |
| v1.0.0 | 功能冻结，文档与错误处理完备 |

> 遵循 AGENTS.md：minor 满 10 进一；patch 用于 bug 修复。

## 5. 风险与未决问题

| 风险 / 未决 | 说明与对策 |
|---|---|
| 与 Director 共存同一 workspace | 依赖树相同，首次构建时间翻倍；增量编译共享，影响有限；`--state-dir` 必须隔离 |
| Makepad 副本无法 `git pull` | 解压 zip 无独立远端；需手工记录来源，更新靠重新下载覆盖 |
| 拿不到 Scope（交互式代码地图） | `apps/scope` 是私有仓库，公开渠道无 clone；只能用 Director 自带 Architecture 模式替代 |
| 仅 Windows 走 D3D11 | `platform/src/os/windows/windows.rs:44` 硬编码 `D3d11Cx`；排错时 Metal/GL 资料不适用 |
| `D3D11 CreateBuffer failed 0x80070057` | 实测 `device still alive`，非致命；图形异常再深查 `d3d11.rs:2624` |
| exe 被运行中进程锁定 | 构建报 `os error 5`，先关掉运行实例 |

**未决问题**

1. 三个辅助 bin（`director-git-guard` / `director-rustc-guard` / `director-flow`）是否保留？
2. 图形/字体之外的其余定制范围（例如是否接入 toolhub 的 `settings` 直达）尚未讨论。

## 附录：已定决策与已验证结论（历史档案，不再追加）

- **路线**：方案 C —— 在 Makepad 仓库 `apps/codeedit/` 原地开发。三个方案对比结论：A（完整 vendoring，+148 万行）会撑爆仓库；B（只复制 app 层 + 外部 path）路径一变即断；C 零复制、工作量最小。
- **crates.io 路线已否决**：`makepad-director` / `makepad-screen` / `makepad-wm-theme` / `makepad-code-arch` / `makepad-diskmap` 均未发布；`makepad-widgets` 等已发布的只是 1.0.0，而仓库是 2.0.0（API 不兼容）。Director 的 14 个直接依赖里 5 个无法从 crates.io 获取。
- **字体（中文豆腐块）已修复并验证**：根因是 `install_theme_fonts()` 用 `policy.monospace`（International 下只有 `LIBERATION_MONO`）重建代码样式字体，桌面端无 CJK 回退。修复：新增 `CODE_CHAIN = [LIBERATION_MONO, SYSTEM_CJK_FONT_ASSET]`（`platform/src/font_policy.rs`），`install_theme_fonts()` 代码样式改用 `CODE_CHAIN`（`widgets/src/font_policy.rs`），抽出 `register_abs_resource_path()` 共用（`platform/src/script/res.rs`）。
- **验证证据**：`font miss` 从 3 条（U+FF0C/U+FF08/U+FF09）降到 0 条；`cargo test -p makepad-platform --lib font_policy` → 9 passed；`cargo build -p makepad-codeedit --release` 零告警（17MB exe）。
- **日志陷阱**：CJK 区间缺失是静默的，只有全角标点会打印 `font miss`；判断修复是否生效**必须看这两个全角标点是否消失**。
- **字体限制**：Windows 专属（硬编码 `C:/Windows/Fonts/simhei.ttf`），无兜底；体积零增长（运行时读盘，无 `include_bytes!`）；换字体只改 `SYSTEM_CJK_FONT_ASSET` 一处。
