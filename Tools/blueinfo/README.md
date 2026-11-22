# blueinfo - 通用桌面蓝牙调试工具

版本: v1.2.0

## 简介

`blueinfo` 是 tao_tools 工具链中的桌面蓝牙调试工具，基于 eframe + egui 提供原生桌面图形界面，支持串口蓝牙（HC05/HC06）通信、实时数据曲线绘制和设备参数调试。

## 功能

- [x] 串口扫描、连接/断开，在线状态指示
- [x] 双模式通信：bincode 二进制结构化数据 + ASCII 文本日志
- [x] 实时曲线绘制（多条通道，60 FPS，定长环形队列）
- [x] 参数调试面板（滑块 + 精确数值输入 + 下发/恢复/保存/加载）
- [x] 文本指令收发面板 + 多行日志框
- [x] 鼠标框选放大、悬停数值、坐标轴自动缩放
- [x] 断线自动重连（3 秒周期状态机：失败持续重试并保留端口信息，手动「断开」后停止）
- [x] 文本行与二进制帧共用单调相对时间轴（连接时记 Instant 原点，重连后时间轴连续）
- [x] 日志分类着色（RX / TX / ERR / 信息），只解析 Rx 且非回显前缀的文本行，避免回显伪造通道
- [x] 读写线程分离：读线程独占串口句柄，发送只投递 mpsc 写队列（UI 热路径无串口锁，100 次连发不阻塞）
- [x] 协议加固：定长布局校验 + 帧长度上限 64KB + 坏帧丢弃计数 + 残帧 500ms 超时 + 逐字节重同步
- [x] 快捷指令条：设置里配置的命令在主界面渲染为按钮，点击即发送
- [x] 通道显隐（复选框，重启保持）+ 双击重命名（仅前端显示）
- [x] 「蓝牙设备」与「参数调试」面板独立折叠（修复同名标题导致的折叠状态串扰）
- [x] 深色工控护眼主题
- [x] 配置持久化（JSON 本地保存）
- [x] `blueinfo settings` 独立设置窗口：默认波特率、自动重连、显示选项、快捷指令（供 toolhub 设置直达）
- [x] 日志导出到文件

## 使用方法

### 启动图形界面

```bash
# 在 Tools/ 目录下启动
cargo run -p blueinfo -- bluetooth-info

# 安装到全局（在 Tools/ 目录下执行）
cargo install --path blueinfo
blueinfo bluetooth-info
```

### 设置界面

不想启动完整主界面时，可以直接只打开设置窗口（不连接设备、不绘图）：

```bash
cargo run -p blueinfo -- settings
# 安装后
blueinfo settings
```

设置项：默认波特率、断线自动重连、日志自动滚动、曲线纵轴自动缩放、快捷指令列表（每行一条）。
保存后写入 `~/.tao_tools/blueinfo/config.json`，下次启动主界面生效。

### 界面布局

- **左侧面板**：串口连接配置 / 参数调试 / 文本指令收发 / 实时数据监控
- **右侧绘图区**：顶部通道栏（勾选显隐、双击改名）+ 数据曲线、框选放大、悬停显示坐标
- **底部状态栏**：连接状态、采样统计、坏帧计数、FPS

### 快捷指令与通道显隐

- 在设置窗口的「快捷指令」里每行写一条命令，保存后主界面「[指令] 串口」区出现对应按钮，点击即发送（日志里记为 `TX`）；
- 绘图区顶部「通道」栏：勾选/取消勾选控制曲线显隐，隐藏名单写入配置的 `hidden_channels`，重启后保持；
- 双击通道名可重命名，仅影响界面显示（图例、状态栏），不改协议键名与持久化键。

### 参数配置保存位置

`~/.tao_tools/blueinfo/config.json`

## 二进制协议

串口上同时存在两类数据：**二进制帧**（结构化数值，进曲线）与**普通文本行**（日志，形如 `KEY: value` 的行会进文本通道）。接收端按魔数 `BLUE` 切分，帧外字节一律按文本处理。

### 帧结构

```
[4B 魔数 "BLUE"] [4B 载荷长度 N（u32 小端）] [N B bincode 载荷]
```

载荷使用 bincode 1 的**定长（fixint）编码**、小端、无填充，布局固定如下：

| 偏移 | 类型 | 字段 |
|---|---|---|
| 0 | f64（8B） | `timestamp` 时间戳（秒，当前不参与曲线 x 轴） |
| 8 | u8（1B） | `channel_count` 通道数（上限 32） |
| 9 | u64（8B） | `Vec` 长度前缀，必须等于 `channel_count` |
| 17 | f64 × count | 各通道数值 |

因此 `N = 17 + 8 × count`（count=3 时 N=41）。**没有 padding，也没有 checksum 字段**。

### 坏帧处理（v1.2.0 起）

| 情形 | 行为 |
|---|---|
| `N > 64 KB` | 丢弃这 8 字节帧头，继续扫描下一个魔数 |
| `N` 与通道数不符 / `Vec` 前缀 ≠ 通道数 / 通道数 > 32 | 整帧丢弃，记 1 个坏帧 |
| 帧头已到但载荷 500ms 内没凑齐 | 丢弃残帧并计数，回到文本模式 |
| 累积缓冲 > 256 KB | 整块丢弃并计数，内存不会无限增长 |
| 文本里出现半个魔数 | 保留末尾 1~3 字节，跨读边界拼成完整魔数 |

坏帧一律**丢弃 + 计数**（状态栏显示「坏帧: N」，日志里是 `[协议]` 开头的 ERR 行），不会被当成文本解析，也不会清空已有曲线；坏帧之后从后续字节重新扫描魔数，截断/乱序流无需重连即可恢复。当前协议没有校验和/CRC（加校验属改协议，需设备端同批升级）。

### C 语言发送示例

```c
#include <stdint.h>
#include <string.h>
#include <unistd.h>   /* write() */

/*
 * 线上格式（全部小端、无填充）：
 *   [4B "BLUE"] [4B payload_len(u32)] [f64 timestamp] [u8 count] [u64 vec_len] [f64 * count]
 *   payload_len = 17 + 8 * count
 *
 * 注意：Vec 的长度前缀是 u64（bincode 定长编码），不是 u8/u32；
 *       载荷里没有 padding，也没有 checksum，请逐字节写。
 */
static int send_blue_packet(int fd, double timestamp, const double *data, int count) {
    if (count < 0 || count > 32) return -1;

    uint8_t buf[8 + 17 + 32 * 8];
    size_t n = 0;
    uint32_t payload_len = (uint32_t)(17 + 8 * count);
    uint64_t vec_len = (uint64_t)count;

    memcpy(buf + n, "BLUE", 4);
    n += 4;
    buf[n++] = (uint8_t)(payload_len >> 0);
    buf[n++] = (uint8_t)(payload_len >> 8);
    buf[n++] = (uint8_t)(payload_len >> 16);
    buf[n++] = (uint8_t)(payload_len >> 24);

    memcpy(buf + n, &timestamp, 8);          /* 主机需为 IEEE754 小端 */
    n += 8;
    buf[n++] = (uint8_t)count;               /* u8 通道数 */
    for (int i = 0; i < 8; i++) {            /* u64 Vec 长度前缀 */
        buf[n++] = (uint8_t)(vec_len >> (8 * i));
    }
    memcpy(buf + n, data, sizeof(double) * (size_t)count);
    n += sizeof(double) * (size_t)count;

    size_t sent = 0;
    while (sent < n) {
        ssize_t w = write(fd, buf + sent, n - sent);
        if (w <= 0) return -1;
        sent += (size_t)w;
    }
    return 0;
}
```

发送文本日志直接输出 `printf("logger: hello\n");` 即可，程序自动解析为文本行。形如 `KEY: value` / `KEY = value` 的行会进文本通道（键名不含空白、长度 < 20）；以 `>>>` 或 `[` 开头的行视为回显/程序提示，不参与解析。

## 环境依赖

- Rust 2021 edition+
- 操作系统：Windows / Linux / macOS
- 串口硬件驱动（系统自带，USB 蓝牙模块自动虚拟串口）

## 开发规范

通用规范见根目录 [AGENTS.md](../../AGENTS.md)，本工具特有约定：

- **模块划分**：`main.rs` 入口与 CLI → `gui.rs` 界面（eframe/egui + egui_plot 绘图）→ `comm.rs` 串口读写线程 → `protocol.rs` 帧切分 / 结构校验 / 重同步（纯逻辑，可单测）→ `data_struct.rs` 数据结构与 bincode 协议 → `config.rs` 参数持久化
- **线程模型**：读线程独占串口句柄（读超时 20ms）；UI 发送只投递 mpsc 写队列（`try_send`，满载报错不阻塞），UI 热路径不持有任何串口锁；断开只置停止标志并等待读线程退出（有上界），随后残余数据仍会由 UI 取走，不丢日志
- **协议即契约**：帧结构、定长载荷布局、坏帧行为在 README「二进制协议」章节有 C 语言示例；改协议必须同步改 README、`data_struct.rs` 与 `protocol.rs`（含单测）
- **配置位置**：按 README「参数配置保存位置」落盘，不写到程序目录
- **依赖**：`eframe/egui/egui_plot` 0.31 固定同版本号，升级要三者一起升

## 开发进度 / 开发计划

- 已完成：v1.2.0 —— 图形界面、参数配置、串口收发、数据曲线、独立设置窗口、日志导出全部可用；本版完成读写线程分离、二进制协议加固（坏帧丢弃计数 / 重同步 / 残帧超时）、快捷指令条与通道显隐（版本演进见「更新历史」）
- 下一步：P2 串口参数配置（数据位/停止位/校验位，对齐 sertool 模型）→ BLE 低功耗拓展（btleplug）
- 计划、优先级与验收标准见 [plans/blueinfo.md](../../plans/blueinfo.md)

## 项目日志

每次修改本工具都会在同一次提交里更新 [项目日志.md](项目日志.md)。
