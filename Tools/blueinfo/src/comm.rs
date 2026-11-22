// ======================== 串口通信与后台线程 ========================
//
// 线程/锁模型（v1.2.0 起）：
//   - UI 线程只做两件事：把待发送文本投进 mpsc 写队列（try_send，不阻塞），
//     以及每帧 drain 已解析好的数据 —— 热路径上没有任何串口锁；
//   - 后台「serial-reader」线程独占串口句柄：先排空写队列再读，读超时
//     READ_TIMEOUT，因此发送最多被一次读等待延迟（≤20ms）；
//   - 断开只置停止标志 + 关闭写队列 + join（≤ 一个读超时），不再与读争锁；
//   - 解析出的帧/日志放进独立缓冲队列，断开时不清空，残余数据仍会被 UI 取走。
//
// 帧协议与坏帧处理见 protocol.rs 与 README「二进制协议」章节。

use crate::data_struct::{
    BinaryPacket, DeviceParam, LogKind, LogLine, TimedPacket, MAX_CACHED_PACKETS, MAX_LOG_LINES,
};
use crate::protocol::{FeedOutput, FrameParser, FrameStats};
use serialport::SerialPort;
use std::collections::VecDeque;
use std::io::{self, Write};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, SyncSender, TryRecvError, TrySendError};
use std::sync::{Arc, Mutex, MutexGuard};
use std::thread;
use std::time::{Duration, Instant};

/// 读线程单次读等待上限。也是「发送被读延迟」与「断开 join 耗时」的上界。
const READ_TIMEOUT: Duration = Duration::from_millis(20);

/// 写队列容量：UI 侧 try_send 满载时报错而不是阻塞界面
const WRITE_QUEUE_LEN: usize = 256;

/// 投递给读线程的写指令（顺序即发出顺序）
enum WriteCmd {
    Line(String),
}

// ─── 加锁辅助：poison 时继续使用内部数据，避免一次 panic 拖垮整个工具 ──

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(|e| e.into_inner())
}

// ─── 共享通信状态 ────────────────────────────────────────────

pub struct CommState {
    read_handle: Option<thread::JoinHandle<()>>,
    /// 写队列入口；断开/重连时置 None 关闭队列
    write_tx: Option<SyncSender<WriteCmd>>,
    pub port_name: String,
    pub baud_rate: u32,
    pub running: Arc<AtomicBool>,
    pub connected: Arc<AtomicBool>,

    // 共享数据缓冲区（后台线程写入，UI 线程读取）
    pub binary_packets: Arc<Mutex<VecDeque<TimedPacket>>>,
    pub text_lines: Arc<Mutex<VecDeque<LogLine>>>,
    pub channel_names: Arc<Mutex<Vec<String>>>,
    stats: Arc<Mutex<FrameStats>>,
}

impl CommState {
    pub fn new(port_name: String, baud_rate: u32) -> Self {
        Self {
            read_handle: None,
            write_tx: None,
            port_name,
            baud_rate,
            running: Arc::new(AtomicBool::new(false)),
            connected: Arc::new(AtomicBool::new(false)),
            binary_packets: Arc::new(Mutex::new(VecDeque::with_capacity(MAX_CACHED_PACKETS))),
            text_lines: Arc::new(Mutex::new(VecDeque::with_capacity(MAX_LOG_LINES))),
            channel_names: Arc::new(Mutex::new(Vec::new())),
            stats: Arc::new(Mutex::new(FrameStats::default())),
        }
    }

    /// 尝试连接串口并启动后台读取线程。
    ///
    /// 断线重连直接复用本对象调用本方法即可：缓冲区与端口参数都保留，
    /// 失败时对象仍然可用，只是 `connected` 保持 false。
    pub fn connect(&mut self) -> Result<(), String> {
        if self.connected.load(Ordering::SeqCst) {
            let _ = self.disconnect();
            // 给驱动一点时间释放串口句柄
            thread::sleep(Duration::from_millis(50));
        }
        self.open_port()
    }

    /// 打开串口并启动读取线程（保留已有缓冲，供重连后继续追加数据）
    fn open_port(&mut self) -> Result<(), String> {
        // 回收上一轮已结束的读取线程（它独占串口句柄，必须先退出再重新打开）
        if let Some(handle) = self.read_handle.take() {
            let _ = handle.join();
        }
        // 旧的写队列一并关闭
        self.write_tx = None;

        let port = serialport::new(&self.port_name, self.baud_rate)
            .timeout(READ_TIMEOUT)
            .open()
            .map_err(|e| format!("打开串口 {} 失败: {}", self.port_name, e))?;

        // 端口参数已生效，避免读到上一轮残留数据
        let _ = port.clear(serialport::ClearBuffer::All);

        self.connected.store(true, Ordering::SeqCst);
        self.running.store(true, Ordering::SeqCst);

        let (tx, rx) = mpsc::sync_channel::<WriteCmd>(WRITE_QUEUE_LEN);

        let running = self.running.clone();
        let connected = self.connected.clone();
        let packets = self.binary_packets.clone();
        let lines = self.text_lines.clone();
        let ch_names = self.channel_names.clone();
        let stats = self.stats.clone();

        let spawn = thread::Builder::new()
            .name("serial-reader".into())
            .spawn(move || read_loop(port, rx, running, connected, packets, lines, ch_names, stats));

        match spawn {
            Ok(handle) => {
                self.read_handle = Some(handle);
                self.write_tx = Some(tx);
                Ok(())
            }
            Err(e) => {
                // 线程没起来就不算连接成功；port 已随闭包释放，句柄自动关闭
                self.connected.store(false, Ordering::SeqCst);
                self.running.store(false, Ordering::SeqCst);
                Err(format!("创建读取线程失败: {}", e))
            }
        }
    }

    /// 断开串口连接，停止后台线程。
    ///
    /// 只置停止标志并关闭写队列，读线程会在一个读超时内自行退出，因此本调用
    /// 的耗时有上界（≈READ_TIMEOUT），不会因为设备持续发数据而卡住 UI。
    pub fn disconnect(&mut self) -> io::Result<()> {
        self.running.store(false, Ordering::SeqCst);
        self.connected.store(false, Ordering::SeqCst);
        self.write_tx = None;

        if let Some(handle) = self.read_handle.take() {
            let _ = handle.join();
        }
        Ok(())
    }

    /// 发送文本字符串（追加 \r\n）：只投递写队列，不触碰串口锁
    pub fn send_text(&self, text: &str) -> Result<(), String> {
        self.enqueue(text.to_string())?;
        push_line(&self.text_lines, LogLine::tx(format!(">>> {}", text)));
        Ok(())
    }

    /// 下发参数（JSON 格式文本行）
    pub fn send_params(&self, params: &[DeviceParam]) -> Result<(), String> {
        let json = serde_json::to_string(params).map_err(|e| format!("序列化参数失败: {}", e))?;
        self.enqueue(format!("@PARAM:{}", json))?;
        push_line(
            &self.text_lines,
            LogLine::tx(format!(">>> [参数下发] {}", json)),
        );
        Ok(())
    }

    /// 把一整行文本投进写队列（由读线程写出，避免与读争用串口句柄）
    fn enqueue(&self, text: String) -> Result<(), String> {
        if !self.connected.load(Ordering::SeqCst) {
            return Err("串口未连接".into());
        }
        let Some(tx) = self.write_tx.as_ref() else {
            return Err("串口未连接".into());
        };
        tx.try_send(WriteCmd::Line(text)).map_err(|e| match e {
            TrySendError::Full(_) => "发送队列已满，请降低发送频率".to_string(),
            TrySendError::Disconnected(_) => "串口未连接".to_string(),
        })
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

    /// 协议坏帧统计（累计值）
    pub fn frame_stats(&self) -> FrameStats {
        *lock(&self.stats)
    }

    /// 消耗缓存的二进制帧（UI 线程调用）
    pub fn drain_packets(&self) -> Vec<TimedPacket> {
        lock(&self.binary_packets).drain(..).collect()
    }

    /// 消耗缓存的文本行（UI 线程调用）
    pub fn drain_lines(&self) -> Vec<LogLine> {
        lock(&self.text_lines).drain(..).collect()
    }
}

// ─── Drop 确保关窗口时后台线程退出 ────────────────────────

impl Drop for CommState {
    fn drop(&mut self) {
        self.running.store(false, Ordering::SeqCst);
        self.write_tx = None;
        if let Some(handle) = self.read_handle.take() {
            let _ = handle.join();
        }
    }
}

// ─── 后台读写循环（独占串口句柄）──────────────────────────

fn read_loop(
    mut port: Box<dyn SerialPort>,
    rx: Receiver<WriteCmd>,
    running: Arc<AtomicBool>,
    connected: Arc<AtomicBool>,
    packets: Arc<Mutex<VecDeque<TimedPacket>>>,
    lines: Arc<Mutex<VecDeque<LogLine>>>,
    ch_names: Arc<Mutex<Vec<String>>>,
    stats: Arc<Mutex<FrameStats>>,
) {
    let mut buf = [0u8; 4096];
    let mut parser = FrameParser::new();

    while running.load(Ordering::SeqCst) {
        // 1) 先把写队列排空（顺序发送，最多被上一次读等待延迟一个 READ_TIMEOUT）
        let mut write_failed = false;
        loop {
            match rx.try_recv() {
                Ok(WriteCmd::Line(text)) => {
                    let data = format!("{}\r\n", text);
                    if let Err(e) = port.write_all(data.as_bytes()) {
                        push_line(
                            &lines,
                            LogLine::err(format!("[发送失败] 串口写入错误，连接中断: {}", e)),
                        );
                        write_failed = true;
                        break;
                    }
                }
                Err(TryRecvError::Empty) => break,
                // UI 侧已丢弃发送端：继续读，直到 running 被置 false
                Err(TryRecvError::Disconnected) => break,
            }
        }
        if write_failed {
            break;
        }

        // 2) 读一段（带超时，保证能周期性检查 running）
        match port.read(&mut buf) {
            Ok(n) if n > 0 => {
                let out = parser.feed(&buf[..n], Instant::now());
                apply_output(out, &packets, &lines, &ch_names);
                *lock(&stats) = parser.stats();
            }
            Ok(_) => {}
            Err(ref e) if e.kind() == io::ErrorKind::TimedOut => {}
            Err(e) => {
                push_line(
                    &lines,
                    LogLine::err(format!("[串口] 读取失败，连接中断: {}", e)),
                );
                break;
            }
        }

        // 3) 残帧超时检查（坏帧行为：丢弃 + 计数 + 回到文本模式）
        let notes = parser.check_timeout(Instant::now());
        if !notes.is_empty() {
            for note in notes {
                push_line(&lines, LogLine::err(note));
            }
            *lock(&stats) = parser.stats();
        }
    }

    connected.store(false, Ordering::SeqCst);
}

/// 把解析产物落到共享缓冲：帧带本地接收时刻，文本按行切分，提示进 Err 行
fn apply_output(
    out: FeedOutput,
    packets: &Arc<Mutex<VecDeque<TimedPacket>>>,
    lines: &Arc<Mutex<VecDeque<LogLine>>>,
    ch_names: &Arc<Mutex<Vec<String>>>,
) {
    for packet in out.packets {
        store_packet(packet, packets, ch_names);
    }
    if !out.text.is_empty() {
        flush_text(&out.text, lines);
    }
    for note in out.notes {
        push_line(lines, LogLine::err(note));
    }
}

/// 向日志队列追加一行（超上限丢最旧的）
fn push_line(lines: &Arc<Mutex<VecDeque<LogLine>>>, line: LogLine) {
    let mut guard = lock(lines);
    if guard.len() >= MAX_LOG_LINES {
        guard.pop_front();
    }
    guard.push_back(line);
}

/// 将字节切片作为 UTF-8 文本刷新到日志缓冲区（Rx 分类）
fn flush_text(data: &[u8], lines: &Arc<Mutex<VecDeque<LogLine>>>) {
    if data.is_empty() {
        return;
    }
    let s = String::from_utf8_lossy(data).to_string();
    let at = Instant::now();
    let mut guard = lock(lines);
    for line in s.split('\n') {
        let trimmed = line.trim_end_matches('\r').to_string();
        if !trimmed.is_empty() {
            if guard.len() >= MAX_LOG_LINES {
                guard.pop_front();
            }
            guard.push_back(LogLine {
                kind: LogKind::Rx,
                text: trimmed,
                at,
            });
        }
    }
}

/// 存储解析后的数据包，更新通道名称，并缓存带接收时刻的帧
fn store_packet(
    packet: BinaryPacket,
    packets: &Arc<Mutex<VecDeque<TimedPacket>>>,
    ch_names: &Arc<Mutex<Vec<String>>>,
) {
    // 缓存原始包（带上本地接收时刻，供曲线使用统一单调时间轴）
    {
        let mut guard = lock(packets);
        if guard.len() >= MAX_CACHED_PACKETS {
            guard.pop_front();
        }
        guard.push_back(TimedPacket {
            packet: packet.clone(),
            at: Instant::now(),
        });
    }

    // 更新通道名称（自动发现）
    {
        let n = packet.channel_count as usize;
        let mut cn = lock(ch_names);
        if cn.len() < n {
            for i in cn.len()..n {
                cn.push(format!("CH{}", i + 1));
            }
        }
    }
}

// ─── 单元测试（写队列行为，不依赖真实串口）──────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn comm_with_queue() -> (CommState, Receiver<WriteCmd>) {
        let mut comm = CommState::new("__blueinfo_test_port__".into(), 115200);
        let (tx, rx) = mpsc::sync_channel::<WriteCmd>(WRITE_QUEUE_LEN);
        comm.write_tx = Some(tx);
        comm.connected.store(true, Ordering::SeqCst);
        (comm, rx)
    }

    #[test]
    fn send_text_is_non_blocking_and_keeps_order() {
        let (comm, rx) = comm_with_queue();

        let start = Instant::now();
        for i in 0..100 {
            comm.send_text(&format!("CMD{}", i)).unwrap();
        }
        let elapsed = start.elapsed();
        assert!(
            elapsed < Duration::from_millis(30),
            "100 次连发应远快于 30ms（UI 不阻塞），实际 {:?}",
            elapsed
        );

        let mut got = Vec::new();
        while let Ok(WriteCmd::Line(text)) = rx.try_recv() {
            got.push(text);
        }
        assert_eq!(got.len(), 100);
        assert_eq!(got[0], "CMD0");
        assert_eq!(got[99], "CMD99");

        // 回显按 Tx 分类，不会参与文本→曲线解析
        let lines = comm.drain_lines();
        assert_eq!(lines.len(), 100);
        assert!(lines.iter().all(|l| l.kind == LogKind::Tx));
    }

    #[test]
    fn full_write_queue_returns_error_instead_of_blocking() {
        let (comm, rx) = comm_with_queue();
        for i in 0..WRITE_QUEUE_LEN {
            comm.send_text(&format!("CMD{}", i)).unwrap();
        }

        let start = Instant::now();
        let err = comm.send_text("OVERFLOW").unwrap_err();
        assert!(err.contains("队列已满"), "错误信息应说明队列已满: {err}");
        assert!(start.elapsed() < Duration::from_millis(30));
        drop(rx);

        // 未连接时立即返回错误，不排队
        comm.connected.store(false, Ordering::SeqCst);
        assert_eq!(comm.send_text("x").unwrap_err(), "串口未连接");
    }

    #[test]
    fn disconnect_without_reader_returns_immediately() {
        let (mut comm, _rx) = comm_with_queue();
        let start = Instant::now();
        comm.disconnect().unwrap();
        assert!(start.elapsed() < Duration::from_millis(30));
        assert!(!comm.is_connected());
        // 断开发送端后不再接受新指令
        assert!(comm.send_text("x").is_err());
    }
}
