# deskman 开发计划

> 状态：进行中　最后更新：2026-11-22　版本基线：v4.3.1

## 1. 目标

桌面接管式整理器：桌面图标收纳进分类面板（真实图标）、Dock 滑轨、编辑模式，退出/崩溃后必须原位还原。

## 2. 非目标与分工边界

- **通用窗口管理器**不独立成产品：现有 `window *` 命令仅作保留兼容，不再扩展新能力。
- 仅 Windows，不做跨平台适配。
- **不接管启动/注册表自启的其它职责**：开机自启只做本工具自己的 Run 项，与 [copilotkey](copilotkey.md) 各写各的，互不调用。
- Dock 只做"本工具收集到的项目"的展示，不做全系统启动器；与 [toolhub](toolhub.md) 的启动器定位不合并。

## 3. 候选功能

### P0 桌面图标显隐幂等化，统一三处入口 · 本轮执行

- 证据：`src/gui.rs:1929-1938` 设置页调用 `desktop::toggle_desktop_icons()`（`desktop.rs:532-538` 用 WM_COMMAND 0x7402 发给 SysListView32），这条既不更新 `app.state.view_toggled` 也不走与托盘相同的路径；正确实现却在 `desktop_list.rs:308-318`，仅托盘（`gui.rs:2413-2426`）用到。README v4.0.0 已记录 Win11 必须发给 `SHELLDLL_DefView` 子窗口。
- 做法：新增 `desktop_list::desktop_icons_visible()` 与 `set_desktop_icons_visible(bool)`（仅在目标态不同时发 0x7402，并同步 `state.view_toggled`）；托盘、设置页、`startup_desktop_flow`（`gui.rs:279-298`）、`Drop`（`gui.rs:483-493`）全部改走它；删除 `desktop.rs` 的重复实现。
- 验收：Win11 设置页开关可用；启动前桌面图标已隐藏时启动不再反显；运行中重启 explorer.exe 后退出 deskman，桌面图标保持可见；连点两次开关不漂移。
- 版本：patch（v4.3.2）
- 待决：`0x7402` 后 `IsWindowVisible(SysListView32)` 是否可靠需实测，不可靠则改读 `HKCU\...\Explorer\Advanced\HideIcons`（与现有 IconSize 同法）；`state.taken` 与 `main.rs:231-285` 的 restore-positions/move-icon 已无写入方，是否删除需拍板。

### P0 桌面状态与配置原子落盘 · 本轮执行

- 证据：`config.rs:206-222` 与 `config.rs:353-366` 均 `fs::write` 直写目标文件；写入中被杀留半截 JSON，`config.rs:344-350` 的 load 静默 `unwrap_or_default()`，`DesktopState` 变空 → 退出/下次启动不再还原，桌面停在隐藏态。
- 做法：写 `<name>.tmp` 并 flush，再用 `fs::rename` 原子替换；保留 `.bak`；load 依次尝试主文件/`.bak`，都失败时 `view_toggled` 取保守值（启动先显示一次）。
- 验收：保存瞬间强杀进程 10 次，重启读到的 state 始终可解析且 `view_toggled` 为旧值或新值、绝不为空；手工写入半截文件后启动能显示桌面图标。
- 版本：patch（v4.3.2）

### P0 Dock 放大动画接上（`dock_scales` 目前是死代码）· 本轮执行

- 证据：`gui.rs:634-655` 与 `gui.rs:698-708` 用未平滑的 scale 构造 `places`；`gui.rs:712-723` 只把插值写进 `app.dock_scales`；`gui.rs:738-739` 绘制仍读 `places` → 悬停放大瞬间跳变，16ms 重绘白跑。
- 做法：平滑后按 `app.dock_scales[i]` 重建 `places` 的 rect（回环的 alpha/depth 保留）；插值系数按帧时间归一（现 0.3 与帧率绑定）；`animating` 仅在 `|diff|>0.005` 时 request_repaint。
- 验收：鼠标横向扫过 Dock，图标尺寸连续渐变约 100-200ms、移开平滑回落；静止后 16ms 重绘计数为 0（仅保留 100ms 兜底）。
- 版本：patch（v4.3.2）

### P1 图标纹理后台提取 + 有界缓存

- 证据：`gui.rs:310-331` 的 `icon_texture()` 在 UI 线程同步调 `desktop::extract_icon_rgba`（`desktop.rs:236-297`，SHGetFileInfoW + IImageList）；`gui.rs:1408-1412` 每帧 clone 全组并逐个取纹理，Dock（`gui.rs:747`）同样；缓存只在关闭真实图标时整体 clear（`gui.rs:1918-1920`），删项目/换路径不回收。
- 做法：loader 线程 + mpsc，key=path，UI 只查 Pending/Loaded/Failed，先画占位/矢量图标，回包后 `load_texture` 并 `request_repaint`；限制在途请求（约 32），每帧限时消费；扫描/删除后按现存 path 集合淘汰纹理。
- 验收：100 项首次进入面板时拖动不卡（加 >16ms 帧日志）；图标 1-2s 内逐个出现；反复增删 50 项后 `icon_textures.len()` 不超过当前项目数。
- 版本：minor
- 待决：P2 favicon 应复用该 loader，是否提前做（联网/隐私）需拍板。

### P1 桌面目录变更实时同步 + 忽略清单

- 证据：`gui.rs:40-44` 仅启动时扫描一次，之后只能点托盘「扫描桌面」（`tray.rs:179`）或工具条（`gui.rs:832-834`）；`desktop::scan_into_config`（`desktop.rs:7-29`）只插入新路径、从不核对删除/改名；去重集只取现存面板项（`desktop.rs:14-27`），用户从面板移除的项下次扫描会复活。
- 做法：后台线程 `ReadDirectoryChangesW` 监视用户/公共桌面（30-60s 轮询兜底），事件经 mpsc 进 UI；新文件去重后进「未分类」；路径不存在的项标 missing（灰显+右键移除），不自动删已分类项；被移除的 path 写忽略清单，扫描时跳过。
- 验收：运行中桌面新建 `test.txt`，2s 内进未分类且不重复；删除后 5s 内该 tile 显示「缺失」；已归入「工作」的同名项不被自动删；从面板移除后再次扫描不复活；`config.json` 无重复 path。
- 版本：minor
- 待决：删除策略（标记 vs 移除）、是否监视公共桌面、忽略清单能否一键清空。

### P1 拖拽桌面文件到分类抽屉

- 验收：从资源管理器拖入面板，落到鼠标所在分类（或用拖入位置命中测试），路径去重后写入配置；拖拽失败/非文件类型给气泡提示。补验收：DPI≠100%、面板缩放后落点仍命中。
- 备注：`eframe` 的 `dropped_files` 只能拿到整窗拖入，无法直接知道落在哪个分类，需要自绘命中区域。

### P2 提取 `.url` 网站 favicon 作为网页图标

- 验收：`.url` 项目优先显示站点 favicon，失败回退现有矢量图标；不联网阻塞 UI（后台线程取，缓存 24h）；断网 3s 内回退且不报错。

### P2 保存 / 恢复窗口布局

- 验收：可保存当前受管窗口的位置与尺寸集合，一键恢复；与桌面接管状态分开落盘，退出不自动还原窗口。

### P2 开机自启动配置

- 验收：设置窗口开关写/删 `HKCU\...\Run` 项，重启验证；与 `copilotkey` 使用相同 Run 机制但彼此独立。

### P2 多显示器识别与抽屉布局规则

- 验收：识别多屏，分类面板与 Dock 可按屏幕分配；单屏行为不变。

### P2 默认分组坐标不再硬编码

- 证据：`config.rs:266` 默认分组 `x=1360`，1366×768 或 150% 缩放下首启面板可能出屏。
- 做法：按工作区宽度布局，启动时把分组 rect 夹取到可见区域。
- 验收：1366×768 @100% 与 1920×1080 @150% 首次启动，面板完整可见。

## 4. 版本规划

| 版本 | 内容 |
|---|---|
| v4.3.2 | P0 显隐幂等 + 原子落盘 + Dock 动画 |
| v4.4.0 | P1 图标后台加载 + 目录实时同步 |
| v4.5.0 | P1 拖拽到分类；P2 favicon |
| 待定 | P2 窗口布局 / 自启 / 多屏 / 坐标夹取 |

## 5. 风险与未决问题

- 拖放命中测试需要处理 DPI 缩放与面板坐标变换，容易错位。
- favicon 抓取涉及网络与隐私（会访问站点），需默认关闭或明确提示。
- 多显示器改动会碰核心布局逻辑（`gui.rs` 最大文件），必须在「退出原位还原」路径上回归验证。
- 显隐统一后，Explorer 重启/多开是最容易被打破的场景，验收必须覆盖「隐藏态启动 → 退出」与「运行中重启 explorer」。
