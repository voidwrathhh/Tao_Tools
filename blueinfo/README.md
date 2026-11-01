# blueinfo - 通用桌面蓝牙调试工具

版本: v1.0.0

## 简介

`blueinfo` 是 tao_tools 工具链中的桌面蓝牙调试工具，基于 eframe + egui 提供原生桌面图形界面，支持串口蓝牙（HC05/HC06）通信、实时数据曲线绘制和设备参数调试。

## 功能

- [x] 串口扫描、连接/断开，在线状态指示
- [x] 双模式通信：bincode 二进制结构化数据 + ASCII 文本日志
- [x] 实时曲线绘制（多条通道，60 FPS，定长环形队列）
- [x] 参数调试面板（滑块 + 精确数值输入 + 下发/恢复/保存/加载）
- [x] 文本指令收发面板 + 多行日志框
- [x] 鼠标框选放大、悬停数值、坐标轴自动缩放
- [x] 断线自动重连（3 秒周期）
- [x] 深色工控护眼主题
- [x] 配置持久化（JSON 本地保存）
- [x] 日志导出到文件
- [ ] BLE 低功耗蓝牙拓展（btleplug 接口预留）
- [ ] 串口参数配置（数据位、停止位、校验位）
- [ ] 截图保存

## 使用方法

### 启动图形界面

```bash
# 从项目根目录启动
cargo run -- bluetooth-info

# 或指定 blueinfo 包
cargo run -p blueinfo -- bluetooth-info

# 安装到全局后
cargo install --path blueinfo
blueinfo bluetooth-info
```

### 界面布局

- **左侧面板**：串口连接配置 / 参数调试 / 文本指令收发 / 实时数据监控
- **右侧绘图区**：数据曲线、框选放大、悬停显示坐标
- **底部状态栏**：连接状态、采样统计、FPS

### 参数配置保存位置

`~/.tao_tools/blueinfo/config.json`

## 二进制协议

设备端发送的二进制帧格式（配合 bincode 序列化）：

```
[4B 魔数 "BLUE"] [4B 载荷长度(小端 u32)] [N B bincode 数据]
```

载荷结构（Rust 侧 BinaryPacket）：
```rust
struct BinaryPacket {
    timestamp: f64,        // 时间戳（秒）
    channel_count: u8,     // 通道数量
    channel_data: Vec<f64>,// 各通道数值
}
```

### C 语言发送示例

```c
#include <stdint.h>
#include <string.h>

#pragma pack(1)
typedef struct {
    uint8_t  magic[4];
    uint32_t length;
    double   timestamp;
    uint8_t  channel_count;
    double   channels[8];
    uint32_t checksum;  // 简单 XOR 校验
} BluePacket;
#pragma pack()

void send_blue_packet(int fd, double timestamp, double* data, int count) {
    BluePacket pkt;
    memcpy(pkt.magic, "BLUE", 4);
    pkt.length = 12 + count * 8; // timestamp(8) + count(1) + padding(3) + data
    pkt.timestamp = timestamp;
    pkt.channel_count = count;
    for (int i = 0; i < count && i < 8; i++) {
        pkt.channels[i] = data[i];
    }
    // checksum = XOR of all payload bytes
    uint32_t cs = 0;
    uint8_t* payload = (uint8_t*)&pkt.timestamp;
    for (uint32_t i = 0; i < pkt.length; i++) cs ^= payload[i];
    pkt.checksum = cs;

    uint8_t header[8] = { 'B','L','U','E',
        (pkt.length>>0)&0xFF, (pkt.length>>8)&0xFF,
        (pkt.length>>16)&0xFF, (pkt.length>>24)&0xFF };
    write(fd, header, 8);
    write(fd, &pkt.timestamp, pkt.length);
}
```

发送文本日志直接输出 `printf("logger: hello\n");` 即可，程序自动解析为文本行。

## 环境依赖

- Rust 2021 edition+
- 操作系统：Windows / Linux / macOS
- 串口硬件驱动（系统自带，USB 蓝牙模块自动虚拟串口）

## 开发计划

- [ ] BLE 低功耗蓝牙接口（btleplug 集成）
- [ ] 串口参数配置面板（数据/停止/校验位）
- [ ] 曲线截图保存到本地
- [ ] 多语言界面切换
- [ ] 历史配置导入导出（跨设备）
- [ ] 数据回放功能
