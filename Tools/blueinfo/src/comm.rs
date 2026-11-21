// ======================== 串口通信与后台线程 ========================
//
// 架构说明：
//   - 后台独立线程负责串口数据读取、帧解析、缓存
//   - UI 主线程通过 Arc<Mutex<...>> 共享状态读取解析后的数据
//   - 写入操作（发送文本/参数）由 UI 线程直接执行，加锁保护串口对象
//   - 数据读取永不阻塞 UI 渲染
//
// 帧协议：
//   [4B magic "BLUE"] [4B payload_len(u32 LE)] [N B bincode payload]
//   文本行以 \n 或 \r\n 分隔，在帧外独立解析。

use crate::data_struct::{BinaryPacket, DeviceParam, MAGIC, MAX_CACHED_PACKETS, MAX_LOG_LINES};
use serialport::SerialPort;
use std::collections::{HashMap, VecDeque};
use std::io::{self, Read, Write};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

// ─── 共享通信状态 ────────────────────────────────────────────

pub struct CommState {
    port: Arc<Mutex<Option<Box<dyn SerialPort>>>>,
    read_handle: Option<thread::JoinHandle<()>>,
    pub port_name: String,
    pub baud_rate: u32,
    pub running: Arc<AtomicBool>,
    pub connected: Arc<AtomicBool>,

    // 共享数据缓冲区（后台线程写入，UI 线程读取）
    pub binary_packets: Arc<Mutex<VecDeque<BinaryPacket>>>,
    pub text_lines: Arc<Mutex<VecDeque<String>>>,
    pub latest_values: Arc<Mutex<HashMap<String, f64>>>,
    pub channel_names: Arc<Mutex<Vec<String>>>,
}

impl CommState {
    pub fn new(port_name: String, baud_rate: u32) -> Self {
        Self {
            port: Arc::new(Mutex::new(None)),
            read_handle: None,
            port_name,
            baud_rate,
            running: Arc::new(AtomicBool::new(false)),
            connected: Arc::new(AtomicBool::new(false)),
            binary_packets: Arc::new(Mutex::new(VecDeque::with_capacity(MAX_CACHED_PACKETS))),
            text_lines: Arc::new(Mutex::new(VecDeque::with_capacity(MAX_LOG_LINES))),
            latest_values: Arc::new(Mutex::new(HashMap::new())),
            channel_names: Arc::new(Mutex::new(Vec::new())),
        }
    }

    /// 尝试连接串口并启动后台读取线程
    pub fn connect(&mut self) -> Result<(), String> {
        if self.connected.load(Ordering::SeqCst) {
            let _ = self.disconnect();
            thread::sleep(Duration::from_millis(200));
        }

        let port = serialport::new(&self.port_name, self.baud_rate)
            .timeout(Duration::from_millis(50))
            .open()
            .map_err(|e| format!("打开串口 {} 失败: {}", self.port_name, e))?;

        self.connected.store(true, Ordering::SeqCst);
        self.running.store(true, Ordering::SeqCst);

        let port = Arc::new(Mutex::new(Some(port)));
        self.port = port.clone();

        // ── 启动后台读取线程 ──
        let running = self.running.clone();
        let connected = self.connected.clone();
        let packets = self.binary_packets.clone();
        let lines = self.text_lines.clone();
        let values = self.latest_values.clone();
        let ch_names = self.channel_names.clone();

        let handle = thread::Builder::new()
            .name("serial-reader".into())
            .spawn(move || read_loop(port, running, connected, packets, lines, values, ch_names))
            .map_err(|e| format!("创建读取线程失败: {}", e))?;

        self.read_handle = Some(handle);
        Ok(())
    }

    /// 断开串口连接，停止后台线程
    pub fn disconnect(&mut self) -> io::Result<()> {
        self.running.store(false, Ordering::SeqCst);
        self.connected.store(false, Ordering::SeqCst);

        // 关闭串口以唤醒阻塞的 read()
        if let Ok(mut guard) = self.port.lock() {
            drop(guard.take());
        }

        // 等待后台线程结束
        if let Some(handle) = self.read_handle.take() {
            let _ = handle.join();
        }
        Ok(())
    }

    /// 发送文本字符串（追加 \r\n）
    pub fn send_text(&self, text: &str) -> Result<(), String> {
        let mut guard = self.port.lock().unwrap();
        if let Some(ref mut port) = *guard {
            let data = format!("{}\r\n", text);
            port.write_all(data.as_bytes())
                .map_err(|e| format!("发送失败: {}", e))?;
            // 本地回显到日志
            let mut lines = self.text_lines.lock().unwrap();
            if lines.len() >= MAX_LOG_LINES {
                lines.pop_front();
            }
            lines.push_back(format!(">>> {}", text));
            Ok(())
        } else {
            Err("串口未连接".into())
        }
    }

    /// 下发参数（JSON 格式文本行）
    pub fn send_params(&self, params: &[DeviceParam]) -> Result<(), String> {
        let json = serde_json::to_string(params).map_err(|e| format!("序列化参数失败: {}", e))?;
        let mut guard = self.port.lock().unwrap();
        if let Some(ref mut port) = *guard {
            let data = format!("@PARAM:{}\r\n", json);
            port.write_all(data.as_bytes())
                .map_err(|e| format!("参数下发失败: {}", e))?;
            let mut lines = self.text_lines.lock().unwrap();
            if lines.len() >= MAX_LOG_LINES {
                lines.pop_front();
            }
            lines.push_back(format!(">>> [参数下发] {}", json));
            Ok(())
        } else {
            Err("串口未连接".into())
        }
    }

    /// 刷新可用串口列表
    pub fn available_ports() -> Vec<(String, String)> {
        serialport::available_ports()
            .unwrap_or_default()
            .into_iter()
            .filter(|p| matches!(&p.port_type, serialport::SerialPortType::BluetoothPort))
            .map(|p| (p.port_name.clone(), p.port_name.clone()))
            .collect()
    }

    /// 判断当前是否处于连接状态
    pub fn is_connected(&self) -> bool {
        self.connected.load(Ordering::SeqCst)
    }

    /// 消耗缓存的二进制帧（UI 线程调用）
    pub fn drain_packets(&self) -> Vec<BinaryPacket> {
        let mut guard = self.binary_packets.lock().unwrap();
        guard.drain(..).collect()
    }

    /// 消耗缓存的文本行（UI 线程调用）
    pub fn drain_lines(&self) -> Vec<String> {
        let mut guard = self.text_lines.lock().unwrap();
        guard.drain(..).collect()
    }
}

// ─── Drop 确保关窗口时后台线程退出 ────────────────────────

impl Drop for CommState {
    fn drop(&mut self) {
        self.running.store(false, Ordering::SeqCst);
        if let Ok(mut guard) = self.port.lock() {
            drop(guard.take());
        }
        if let Some(handle) = self.read_handle.take() {
            let _ = handle.join();
        }
    }
}

// ─── 后台读取循环 ────────────────────────────────────────────

fn read_loop(
    port: Arc<Mutex<Option<Box<dyn SerialPort>>>>,
    running: Arc<AtomicBool>,
    connected: Arc<AtomicBool>,
    packets: Arc<Mutex<VecDeque<BinaryPacket>>>,
    lines: Arc<Mutex<VecDeque<String>>>,
    values: Arc<Mutex<HashMap<String, f64>>>,
    ch_names: Arc<Mutex<Vec<String>>>,
) {
    let mut buf = [0u8; 4096];
    let mut accum: Vec<u8> = Vec::with_capacity(8192);

    while running.load(Ordering::SeqCst) {
        let n = {
            let mut guard = port.lock().unwrap();
            match guard.as_mut() {
                Some(p) => match p.read(&mut buf) {
                    Ok(n) => n,
                    Err(e) if e.kind() == io::ErrorKind::TimedOut => 0,
                    Err(_) => {
                        connected.store(false, Ordering::SeqCst);
                        break;
                    }
                },
                None => {
                    thread::sleep(Duration::from_millis(50));
                    continue;
                }
            }
        };

        if n == 0 {
            thread::sleep(Duration::from_millis(10));
            continue;
        }

        accum.extend_from_slice(&buf[..n]);
        parse_accumulator(&mut accum, &packets, &lines, &values, &ch_names);
    }

    connected.store(false, Ordering::SeqCst);
}

// ─── 二进制帧解析器 ──────────────────────────────────────────

fn parse_accumulator(
    accum: &mut Vec<u8>,
    packets: &Arc<Mutex<VecDeque<BinaryPacket>>>,
    lines: &Arc<Mutex<VecDeque<String>>>,
    values: &Arc<Mutex<HashMap<String, f64>>>,
    ch_names: &Arc<Mutex<Vec<String>>>,
) {
    loop {
        if accum.len() < 8 {
            break; // 不足最小帧头
        }

        // 查找魔数位置
        let magic_pos = accum.windows(4).position(|w| w == MAGIC);

        match magic_pos {
            Some(pos) => {
                // pos 之前的数据当作文本处理
                if pos > 0 {
                    flush_text(&accum[..pos], lines);
                    accum.drain(..pos);
                }

                // 尝试提取二进制帧（从 pos=0 开始，因为前面的文本已清除）
                if let Some(packet) = try_extract_binary_frame(accum) {
                    store_packet(packet, packets, values, ch_names);
                } else {
                    // 数据不足，等待下次
                    break;
                }
            }
            None => {
                // 没有魔数，全部当作文本
                flush_text(accum, lines);
                accum.clear();
                break;
            }
        }
    }
}

/// 尝试从累积缓冲区头部提取一个完整的二进制帧
fn try_extract_binary_frame(accum: &mut Vec<u8>) -> Option<BinaryPacket> {
    // 帧结构: [4B MAGIC] [4B LE length] [N B bincode payload]
    if accum.len() < 8 {
        return None;
    }
    let payload_len = u32::from_le_bytes([accum[4], accum[5], accum[6], accum[7]]) as usize;
    let frame_len = 8 + payload_len;

    if accum.len() < frame_len {
        return None; // 等待更多数据
    }

    let payload = &accum[8..frame_len];
    match bincode::deserialize::<BinaryPacket>(payload) {
        Ok(packet) => {
            accum.drain(..frame_len);
            Some(packet)
        }
        Err(_) => {
            // 无效帧，跳过魔数作为文本
            accum.drain(..4);
            None
        }
    }
}

/// 将字节切片作为 UTF-8 文本刷新到日志缓冲区
fn flush_text(data: &[u8], lines: &Arc<Mutex<VecDeque<String>>>) {
    if data.is_empty() {
        return;
    }
    let s = String::from_utf8_lossy(data).to_string();
    let mut guard = lines.lock().unwrap();
    for line in s.split('\n') {
        let trimmed = line.trim_end_matches('\r').to_string();
        if !trimmed.is_empty() {
            if guard.len() >= MAX_LOG_LINES {
                guard.pop_front();
            }
            guard.push_back(trimmed);
        }
    }
}

/// 存储解析后的数据包，更新通道数值和曲线数据
fn store_packet(
    packet: BinaryPacket,
    packets: &Arc<Mutex<VecDeque<BinaryPacket>>>,
    values: &Arc<Mutex<HashMap<String, f64>>>,
    ch_names: &Arc<Mutex<Vec<String>>>,
) {
    // 缓存原始包
    {
        let mut guard = packets.lock().unwrap();
        if guard.len() >= MAX_CACHED_PACKETS {
            guard.pop_front();
        }
        guard.push_back(packet.clone());
    }

    // 更新通道名称（自动发现）
    {
        let n = packet.channel_count as usize;
        let mut cn = ch_names.lock().unwrap();
        if cn.len() < n {
            for i in cn.len()..n {
                cn.push(format!("CH{}", i + 1));
            }
        }
    }

    // 更新最新数值
    {
        let mut vals = values.lock().unwrap();
        for (i, val) in packet.channel_data.iter().enumerate() {
            let key = format!("CH{}", i + 1);
            vals.insert(key, *val);
        }
    }
}
