# sertool 开发计划

> 状态：进行中　最后更新：2026-11-22　版本基线：v2.4.1

## 1. 目标

终端串口调试工具：扫描/连接串口、TUI 实时收发、设置项持久化。保持纯终端（无 GUI 依赖）。

## 2. 非目标与分工边界

- **BLE / 蓝牙调试不做** → 归 [blueinfo](blueinfo.md)。
- **图形曲线、截图不做** → 归 [blueinfo](blueinfo.md)。
- 不引入 GUI 框架（eframe/egui 等），只依赖 `ratatui` + `crossterm`。
- 不做跨工具共享代码；与 blueinfo 的同类功能各自实现，参数模型保持一致即可（见 P1 串口参数）。
- **RX 解码只在显示层做**，不改设备协议；与 blueinfo 的协议解析互不依赖。

## 3. 候选功能

### P1 串口参数配置（数据位 / 停止位 / 校验位）

- 验收：`sertool settings` 面板可选 5/6/7/8 数据位、1/1.5/2 停止位、None/Even/Odd 校验；`connect` 按配置应用；非法组合给出中文错误并回退 8-N-1。
- 备注：`serialport` 0.4 对应 `DataBits` / `StopBits` / `Parity`；`config.ini` 新增字段，缺省 8-N-1 以兼容旧配置。
- 与 blueinfo：**本工具的字段名与取值作为基准**，blueinfo 的同类计划对齐本模型。

### P1 滚动锚点改绝对行 + 暂停跟随

- 证据：`main.rs:860-869` 用 `end = total - scroll`，"距底行数"在新数据到达时整体下移，与 README「已滚动位置不被打扰」矛盾。
- 做法：改存绝对 top 行号（或 `bottom_anchor`），push/pop_front 时同步修正；新增暂停/恢复跟随键（F）与 `End` 回底，状态栏提示「N 行新数据 ↓」。
- 验收：上滚 10 行后持续输出 100 行，屏幕文本不动；`End` 或 `F` 恢复自动跟随；新键位与 Ctrl+S/Ctrl+E 不冲突。
- 版本：minor

### P1 日志保存到文件

- 验收：`connect --log <路径>`（或在 settings 里配置开关）；接收数据按原始字节落盘，不受 HEX/文本显示模式影响；退出与 Ctrl+C 都要 flush 收尾。
- 备注：日志文件命名默认 `sertool-<端口>-<时间>.log`，落盘位置由 `--log` 决定，不隐式写程序目录。

### P2 输入区修饰键 / 粘贴 / 光标宽度

- 证据：`main.rs:698` 只判 `key.modifiers == KeyModifiers::CONTROL`，未识别组合在 `main.rs:767` 不 return，落到 `main.rs:770-775` 的 `Char` 分支，Ctrl+V/Ctrl+X 会往输入框塞 `v`/`x`；未启用 bracketed paste；`main.rs:899` 用字节长度算光标，中文输入时光标偏右。
- 做法：判定改 `contains(CONTROL)`，match 末尾统一 `return` 吞掉未识别组合；启用 `EnableBracketedPaste` 并处理 `Event::Paste`（定 `MAX_INPUT_LEN` 截断策略）；光标 x 用显示宽度（`unicode-width`，需同意加依赖，否则按字符数近似）。
- 验收：按 Ctrl+V/Ctrl+Q/Ctrl+Shift+S 后输入框内容不变；粘贴「温度=25℃」完整且光标在末尾；输入中文光标不偏。
- 版本：patch（粘贴部分算 minor）

### P2 发送历史 / 快捷指令（待确认）

- 待确认：是否与 blueinfo 的「快捷指令」重复；若做，只做 TUI 内 `↑`/`↓` 回溯本次会话已发送内容，不做持久化快捷指令列表。

### P2 RX 编码选择（GBK 等）

- 说明：设备若发 GBK，当前显示层必然乱码。做之前先确认是否值得引入 `encoding_rs`（与"轻依赖"取向冲突），且与 blueinfo 的解码口径保持一致。

## 4. 版本规划

| 版本 | 内容 |
|---|---|
| v2.5.0 | P1 串口参数配置 |
| v2.6.0 | P1 日志落盘 + 滚动锚点/暂停跟随 |
| 待定 | P2 输入区修复 / 发送历史 / GBK |

## 5. 风险与未决问题

- 部分 USB 转串口驱动会忽略校验位设置，`serialport` 设置成功不代表设备端生效，需真实硬件实测并如实报错。
- 新增配置字段后必须保证旧 `config.ini` 仍可解析（缺省值补齐），不能让老用户启动失败。
- 停止位 1.5 在多数平台不支持，需在 UI 上标注或在设置时拒绝该组合。
- `READ_TIMEOUT`（当前 50ms）的 flush 阈值未在真机上实测：太快会把慢速设备的多段输出切碎，太慢则无换行设备看到提示符有延迟；需要接真实设备后按设备输出节奏复核。
