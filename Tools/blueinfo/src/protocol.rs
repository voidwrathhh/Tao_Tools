// ======================== 二进制协议解析器 ========================
//
// 帧格式（与 README「二进制协议」章节一致）：
//   [4B 魔数 "BLUE"] [4B 载荷长度(u32 LE)] [N B bincode 载荷]
//
// bincode 1 定长（fixint）编码下的载荷布局是固定的：
//   f64 timestamp(8) + u8 channel_count(1) + Vec 长度前缀 u64(8) + f64 * count
//   即 N = 17 + 8 * count
//
// 加固策略（纯逻辑，便于单测；串口线程只负责喂字节）：
//   1. 载荷长度上限 MAX_PAYLOAD_LEN，超限即判定坏帧 —— 防止伪造长度撑爆累积缓冲；
//   2. 解码前先按上面的定长布局做结构校验（长度、通道数上限、Vec 前缀与通道数一致），
//      校验通过才交给 bincode —— 防止 Vec 前缀伪造出天文数字导致的巨量分配；
//   3. 坏帧整帧丢弃并计数（不把二进制垃圾当文本解析），随后继续向前扫描魔数重同步；
//   4. 残帧超时：等待完整的帧超过 FRAME_TIMEOUT 即丢弃并计数，回到文本模式；
//   5. 累积缓冲硬上限 MAX_ACCUM_BYTES，超限整块丢弃并计数，绝不无限增长。

use crate::data_struct::{BinaryPacket, MAGIC};
use std::time::{Duration, Instant};

/// 单个载荷的最大字节数（超过即判定为坏帧）
pub const MAX_PAYLOAD_LEN: usize = 64 * 1024;

/// 累积缓冲的硬上限，超过即丢弃并重同步（正常情况下远达不到）
pub const MAX_ACCUM_BYTES: usize = 256 * 1024;

/// 残帧（帧头声明了长度但载荷迟迟凑不齐）的最长等待时间
pub const FRAME_TIMEOUT: Duration = Duration::from_millis(500);

/// 单帧允许的最大通道数
pub const MAX_CHANNELS: usize = 32;

/// bincode 定长载荷的固定头部长度：f64(8) + u8(1) + Vec 长度前缀 u64(8)
pub const PAYLOAD_HEADER_LEN: usize = 17;

/// 帧头长度：魔数(4) + 载荷长度(4)
pub const FRAME_HEADER_LEN: usize = 8;

/// 坏帧/重同步统计（累计值，供状态栏展示与排障）
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct FrameStats {
    /// 被判定为坏帧并丢弃的帧数（长度超限、结构不符、bincode 解码失败）
    pub bad_frames: u64,
    /// 触发的重同步次数
    pub resyncs: u64,
    /// 因缓冲上限/残帧超时被丢弃的字节数
    pub dropped_bytes: u64,
    /// 残帧超时次数
    pub timeouts: u64,
}

/// 一次喂数据后的解析产物
#[derive(Debug, Default)]
pub struct FeedOutput {
    /// 解出的二进制帧
    pub packets: Vec<BinaryPacket>,
    /// 帧外文本的原始字节（由调用方按行切分并记录接收时刻）
    pub text: Vec<u8>,
    /// 需要写进日志的协议提示（坏帧、丢弃、超时等）
    pub notes: Vec<String>,
}

/// 二进制帧解析器：把串口字节流切成「二进制帧」与「帧外文本」
pub struct FrameParser {
    accum: Vec<u8>,
    /// 已看到可信帧头、正在等载荷凑齐的起始时刻
    waiting_since: Option<Instant>,
    stats: FrameStats,
}

impl Default for FrameParser {
    fn default() -> Self {
        Self::new()
    }
}

impl FrameParser {
    pub fn new() -> Self {
        Self {
            accum: Vec::with_capacity(8192),
            waiting_since: None,
            stats: FrameStats::default(),
        }
    }

    pub fn stats(&self) -> FrameStats {
        self.stats
    }

    /// 当前累积缓冲字节数（测试用，验证不会无限增长）
    #[cfg(test)]
    pub fn buffered(&self) -> usize {
        self.accum.len()
    }

    /// 喂入一段串口数据，返回本次解出的帧、文本与提示
    pub fn feed(&mut self, data: &[u8], now: Instant) -> FeedOutput {
        let mut out = FeedOutput::default();
        self.accum.extend_from_slice(data);

        // 兜底：累积缓冲硬上限，防止任何路径把它撑爆
        if self.accum.len() > MAX_ACCUM_BYTES {
            let dropped = self.accum.len();
            self.stats.dropped_bytes += dropped as u64;
            self.stats.bad_frames += 1;
            out.notes.push(format!(
                "[协议] 累积缓冲超过 {} KB 上限，丢弃 {} 字节并重同步",
                MAX_ACCUM_BYTES / 1024,
                dropped
            ));
            self.accum.clear();
            self.waiting_since = None;
            return out;
        }

        loop {
            if self.accum.len() < MAGIC.len() {
                break;
            }

            match find_magic(&self.accum) {
                Some(pos) => {
                    // 魔数之前的字节是帧外文本
                    if pos > 0 {
                        out.text.extend_from_slice(&self.accum[..pos]);
                        self.accum.drain(..pos);
                    }

                    if self.accum.len() < FRAME_HEADER_LEN {
                        // 只收到部分帧头，等下次数据
                        break;
                    }

                    let payload_len = u32::from_le_bytes([
                        self.accum[4],
                        self.accum[5],
                        self.accum[6],
                        self.accum[7],
                    ]) as usize;

                    if payload_len > MAX_PAYLOAD_LEN {
                        // 长度不可信：丢弃这 8 字节帧头，从后续字节重新扫魔数
                        self.stats.bad_frames += 1;
                        self.stats.resyncs += 1;
                        out.notes.push(format!(
                            "[协议] 帧长度 {} 字节超过上限 {} 字节，丢弃帧头并重同步",
                            payload_len, MAX_PAYLOAD_LEN
                        ));
                        self.accum.drain(..FRAME_HEADER_LEN);
                        self.waiting_since = None;
                        continue;
                    }

                    let frame_len = FRAME_HEADER_LEN + payload_len;
                    if self.accum.len() < frame_len {
                        // 帧头可信但载荷未到齐：开始等，超时由 check_timeout 处理
                        if self.waiting_since.is_none() {
                            self.waiting_since = Some(now);
                        }
                        break;
                    }

                    let payload = &self.accum[FRAME_HEADER_LEN..frame_len];
                    match decode_payload(payload) {
                        Ok(packet) => {
                            self.accum.drain(..frame_len);
                            self.waiting_since = None;
                            out.packets.push(packet);
                        }
                        Err(reason) => {
                            // 结构校验失败：整帧丢弃（长度可信，绝不把二进制垃圾灌进日志）
                            self.stats.bad_frames += 1;
                            self.stats.resyncs += 1;
                            out.notes.push(format!(
                                "[协议] 帧载荷无效（{}），丢弃 {} 字节并重同步",
                                reason, frame_len
                            ));
                            self.accum.drain(..frame_len);
                            self.waiting_since = None;
                        }
                    }
                }
                None => {
                    // 没有完整魔数：除末尾可能是魔数前缀的字节外，全部按文本处理
                    let keep = magic_prefix_suffix_len(&self.accum);
                    let split = self.accum.len() - keep;
                    if split > 0 {
                        out.text.extend_from_slice(&self.accum[..split]);
                    }
                    self.accum.drain(..split);
                    self.waiting_since = None;
                    break;
                }
            }
        }

        out
    }

    /// 残帧超时检查：等待超过 `FRAME_TIMEOUT` 仍不完整则丢弃并回到文本模式。
    /// 串口线程每轮读取后调用一次。
    pub fn check_timeout(&mut self, now: Instant) -> Vec<String> {
        let Some(since) = self.waiting_since else {
            return Vec::new();
        };
        if now.saturating_duration_since(since) < FRAME_TIMEOUT {
            return Vec::new();
        }
        let dropped = self.accum.len();
        self.stats.timeouts += 1;
        self.stats.dropped_bytes += dropped as u64;
        self.accum.clear();
        self.waiting_since = None;
        vec![format!(
            "[协议] 残帧超过 {} ms 未凑齐，丢弃 {} 字节并回到文本模式",
            FRAME_TIMEOUT.as_millis(),
            dropped
        )]
    }
}

/// 在字节流中查找魔数位置
fn find_magic(buf: &[u8]) -> Option<usize> {
    buf.windows(MAGIC.len()).position(|w| w == MAGIC)
}

/// buf 末尾与魔数前缀匹配的长度（0..=3），用于跨读边界保留半个魔数
fn magic_prefix_suffix_len(buf: &[u8]) -> usize {
    let max = (MAGIC.len() - 1).min(buf.len());
    for k in (1..=max).rev() {
        if buf.ends_with(&MAGIC[..k]) {
            return k;
        }
    }
    0
}

/// 按定长布局校验并解码载荷。
///
/// 先做结构校验（长度、通道数、Vec 前缀与通道数一致），确认 `Vec<f64>` 的长度
/// 只可能来自真实字段，再交给 bincode 解码 —— 避免伪造的 u64 前缀触发巨量分配。
fn decode_payload(payload: &[u8]) -> Result<BinaryPacket, String> {
    if payload.len() < PAYLOAD_HEADER_LEN {
        return Err(format!(
            "载荷 {} 字节短于定长头部 {} 字节",
            payload.len(),
            PAYLOAD_HEADER_LEN
        ));
    }

    let count = payload[8] as usize;
    if count > MAX_CHANNELS {
        return Err(format!("通道数 {} 超过上限 {}", count, MAX_CHANNELS));
    }

    let vec_len = u64::from_le_bytes(payload[9..17].try_into().unwrap());
    if vec_len != count as u64 {
        return Err(format!(
            "Vec 长度前缀 {} 与通道数 {} 不一致",
            vec_len, count
        ));
    }

    let expect = PAYLOAD_HEADER_LEN + count * 8;
    if payload.len() != expect {
        return Err(format!(
            "载荷 {} 字节与通道数 {} 不符（应为 {} 字节）",
            payload.len(),
            count,
            expect
        ));
    }

    let packet: BinaryPacket =
        bincode::deserialize(payload).map_err(|e| format!("bincode 解码失败: {}", e))?;
    if packet.channel_data.len() != count || packet.channel_count as usize != count {
        return Err("解码结果与定长布局不符".into());
    }
    Ok(packet)
}

// ─── 单元测试（纯逻辑，不依赖串口设备）──────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn build_frame(timestamp: f64, data: &[f64]) -> Vec<u8> {
        let packet = BinaryPacket {
            timestamp,
            channel_count: data.len() as u8,
            channel_data: data.to_vec(),
        };
        let payload = bincode::serialize(&packet).unwrap();
        let mut frame = Vec::with_capacity(8 + payload.len());
        frame.extend_from_slice(&MAGIC);
        frame.extend_from_slice(&(payload.len() as u32).to_le_bytes());
        frame.extend_from_slice(&payload);
        frame
    }

    #[test]
    fn documented_c_layout_decodes_exactly() {
        // 完全按 README「二进制协议」里修正后的 C 示例手写字节（不经过 bincode）：
        // f64 timestamp + u8 count + u64 Vec 前缀 + f64*count，全部小端、无填充
        let ts: f64 = 1234.5;
        let values: [f64; 3] = [10.0, -20.5, 0.125];
        let mut payload = Vec::new();
        payload.extend_from_slice(&ts.to_le_bytes());
        payload.push(values.len() as u8);
        payload.extend_from_slice(&(values.len() as u64).to_le_bytes());
        for v in values {
            payload.extend_from_slice(&v.to_le_bytes());
        }
        assert_eq!(payload.len(), PAYLOAD_HEADER_LEN + 8 * values.len());

        let mut frame = Vec::new();
        frame.extend_from_slice(&MAGIC);
        frame.extend_from_slice(&(payload.len() as u32).to_le_bytes());
        frame.extend_from_slice(&payload);

        let mut p = FrameParser::new();
        let out = p.feed(&frame, Instant::now());
        assert_eq!(out.packets.len(), 1);
        assert_eq!(out.packets[0].timestamp, ts);
        assert_eq!(out.packets[0].channel_count, 3);
        assert_eq!(out.packets[0].channel_data, values.to_vec());
        assert!(out.notes.is_empty());
        assert_eq!(p.stats(), FrameStats::default());
    }

    #[test]
    fn valid_frame_decodes_exactly() {
        let frame = build_frame(1.5, &[1.0, -2.5, 3.25]);
        let mut p = FrameParser::new();
        let out = p.feed(&frame, Instant::now());
        assert_eq!(out.packets.len(), 1);
        assert_eq!(out.packets[0].timestamp, 1.5);
        assert_eq!(out.packets[0].channel_count, 3);
        assert_eq!(out.packets[0].channel_data, vec![1.0, -2.5, 3.25]);
        assert!(out.text.is_empty());
        assert!(out.notes.is_empty());
        assert_eq!(p.stats(), FrameStats::default());
        assert_eq!(p.buffered(), 0);
    }

    #[test]
    fn frame_split_across_reads_is_reassembled() {
        let frame = build_frame(2.0, &[7.0, 8.0]);
        let split = 11;
        let mut p = FrameParser::new();
        let out1 = p.feed(&frame[..split], Instant::now());
        assert!(out1.packets.is_empty());
        let out2 = p.feed(&frame[split..], Instant::now());
        assert_eq!(out2.packets.len(), 1);
        assert_eq!(out2.packets[0].channel_data, vec![7.0, 8.0]);
        assert_eq!(p.buffered(), 0);
    }

    #[test]
    fn text_with_magic_prefix_suffix_is_preserved() {
        // "abcBL" 末尾的 BL 是魔数前缀，必须先留着，等下一段补成完整魔数
        let mut p = FrameParser::new();
        let out1 = p.feed(b"abcBL", Instant::now());
        assert_eq!(out1.text, b"abc");
        assert_eq!(p.buffered(), 2);

        let frame = build_frame(3.0, &[1.0]);
        let mut rest = b"UE".to_vec();
        rest.extend_from_slice(&frame[4..]); // 补上帧头剩余部分
        let out2 = p.feed(&rest, Instant::now());
        assert_eq!(out2.packets.len(), 1);
        assert_eq!(out2.packets[0].channel_data, vec![1.0]);
    }

    #[test]
    fn oversized_length_is_dropped_and_resyncs_to_text() {
        let mut p = FrameParser::new();
        let mut data = Vec::new();
        data.extend_from_slice(&MAGIC);
        data.extend_from_slice(&0x7FFF_FFFFu32.to_le_bytes());
        data.extend_from_slice(b"hello\n");
        let out = p.feed(&data, Instant::now());

        assert!(out.packets.is_empty());
        assert_eq!(p.stats().bad_frames, 1);
        assert_eq!(p.stats().resyncs, 1);
        assert!(!out.notes.is_empty());
        // 帧头被丢弃后，后面的文本照常进文本流
        assert_eq!(String::from_utf8_lossy(&out.text), "hello\n");
        // 缓冲不会因伪造长度而膨胀
        assert!(p.buffered() < MAX_ACCUM_BYTES);
    }

    #[test]
    fn truncated_frame_times_out_and_returns_to_text() {
        let mut p = FrameParser::new();
        let frame = build_frame(4.0, &[1.0, 2.0]);
        let now = Instant::now();
        // 只喂帧头和一半载荷
        let out1 = p.feed(&frame[..frame.len() - 4], now);
        assert!(out1.packets.is_empty());
        assert!(p.buffered() > 0);

        // 未到超时时间：不丢弃
        assert!(p.check_timeout(now + Duration::from_millis(100)).is_empty());
        assert!(p.buffered() > 0);

        // 超时：丢弃残帧并回到文本模式
        let notes = p.check_timeout(now + FRAME_TIMEOUT + Duration::from_millis(1));
        assert_eq!(notes.len(), 1);
        assert_eq!(p.stats().timeouts, 1);
        assert_eq!(p.buffered(), 0);

        let out2 = p.feed(b"CH1: 1\n", Instant::now());
        assert_eq!(String::from_utf8_lossy(&out2.text), "CH1: 1\n");
        assert!(out2.packets.is_empty());
    }

    #[test]
    fn structurally_invalid_payload_is_dropped_and_next_frame_parsed() {
        // 长度字段正确，但通道数与 Vec 前缀不一致 → 结构校验失败
        let mut payload = Vec::new();
        payload.extend_from_slice(&5.0f64.to_le_bytes());
        payload.push(1u8); // channel_count = 1
        payload.extend_from_slice(&99u64.to_le_bytes()); // Vec 前缀撒谎
        payload.extend_from_slice(&1.0f64.to_le_bytes());

        let mut data = Vec::new();
        data.extend_from_slice(&MAGIC);
        data.extend_from_slice(&(payload.len() as u32).to_le_bytes());
        data.extend_from_slice(&payload);
        let good = build_frame(6.0, &[42.0]);
        data.extend_from_slice(&good);
        data.extend_from_slice(b"tail\n");

        let mut p = FrameParser::new();
        let out = p.feed(&data, Instant::now());
        assert_eq!(p.stats().bad_frames, 1);
        assert_eq!(out.packets.len(), 1);
        assert_eq!(out.packets[0].channel_data, vec![42.0]);
        assert_eq!(String::from_utf8_lossy(&out.text), "tail\n");
    }

    #[test]
    fn huge_junk_feed_is_bounded_and_counted() {
        let mut p = FrameParser::new();
        let junk = vec![b'x'; MAX_ACCUM_BYTES + 1024];
        let out = p.feed(&junk, Instant::now());
        assert!(out.packets.is_empty());
        assert!(!out.notes.is_empty());
        assert!(p.stats().dropped_bytes >= MAX_ACCUM_BYTES as u64);
        assert_eq!(p.buffered(), 0);
        // 之后仍能正常解析文本与帧
        let frame = build_frame(7.0, &[9.0]);
        let mut next = b"ok\n".to_vec();
        next.extend_from_slice(&frame);
        let out2 = p.feed(&next, Instant::now());
        assert_eq!(out2.packets.len(), 1);
        assert_eq!(String::from_utf8_lossy(&out2.text), "ok\n");
    }

    #[test]
    fn channel_count_over_limit_is_rejected_without_allocating() {
        let mut payload = Vec::new();
        payload.extend_from_slice(&0.0f64.to_le_bytes());
        payload.push(200u8); // 超过 MAX_CHANNELS
        payload.extend_from_slice(&200u64.to_le_bytes());
        payload.extend_from_slice(&[0u8; 64]); // 随机尾巴

        let mut data = Vec::new();
        data.extend_from_slice(&MAGIC);
        data.extend_from_slice(&(payload.len() as u32).to_le_bytes());
        data.extend_from_slice(&payload);

        let mut p = FrameParser::new();
        let out = p.feed(&data, Instant::now());
        assert!(out.packets.is_empty());
        assert_eq!(p.stats().bad_frames, 1);
        assert!(p.buffered() < 64 * 1024);
    }
}
