//! 图标纹理的后台提取与有界缓存。
//!
//! UI 线程只做「查缓存 / 入队」两件轻活：真正的 `SHGetFileInfoW` 提取在独立的
//! 后台线程里跑，结果经 `std::sync::mpsc` 回传，UI 线程每帧限时消费并在拿到
//! 新纹理后 `request_repaint`。缓存有容量上限，并可按「当前面板里仍存在的路径」
//! 精确淘汰，因此反复增删项目不会让内存无限增长。

use crate::desktop;
use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::time::{Duration, Instant};

/// 同时在途的提取请求上限，避免把后台线程压垮、也避免一次涌入上百个请求
const MAX_IN_FLIGHT: usize = 32;
/// 纹理缓存条目数硬上限（超出后按插入顺序淘汰已完成的条目）
const MAX_ENTRIES: usize = 256;

// ---------- 有界 LRU 缓存（与 egui 解耦，便于单测） ----------

/// 带容量上限的 LRU 缓存：`get` 会刷新热度，插入超过容量时淘汰最久未用的条目。
pub struct LruStore<T> {
    capacity: usize,
    clock: u64,
    map: HashMap<String, (T, u64)>,
}

impl<T> LruStore<T> {
    pub fn new(capacity: usize) -> Self {
        Self {
            capacity: capacity.max(1),
            clock: 0,
            map: HashMap::new(),
        }
    }

    pub fn len(&self) -> usize {
        self.map.len()
    }

    #[cfg_attr(not(test), allow(dead_code))]
    pub fn contains(&self, key: &str) -> bool {
        self.map.contains_key(key)
    }

    /// 取一份克隆并刷新该条目的最近使用时间
    pub fn get_cloned(&mut self, key: &str) -> Option<T>
    where
        T: Clone,
    {
        self.clock += 1;
        let clock = self.clock;
        let entry = self.map.get_mut(key)?;
        entry.1 = clock;
        Some(entry.0.clone())
    }

    pub fn insert(&mut self, key: String, value: T) {
        self.clock += 1;
        self.map.insert(key, (value, self.clock));
        while self.map.len() > self.capacity {
            self.evict_lru();
        }
    }

    /// 只保留 `keep` 里的条目（面板项目删除/扫描后调用）
    pub fn prune_not_in(&mut self, keep: &HashSet<String>) {
        self.map.retain(|key, _| keep.contains(key));
    }

    pub fn clear(&mut self) {
        self.map.clear();
    }

    fn evict_lru(&mut self) {
        if let Some(key) = self
            .map
            .iter()
            .min_by_key(|(_, (_, stamp))| *stamp)
            .map(|(key, _)| key.clone())
        {
            self.map.remove(&key);
        }
    }
}

// ---------- 后台提取 ----------

struct ExtractRequest {
    key: String,
    path: String,
}

struct ExtractResponse {
    key: String,
    rgba: Option<(Vec<u8>, usize, usize)>,
}

pub struct IconStore {
    tx: Sender<ExtractRequest>,
    rx: Receiver<ExtractResponse>,
    /// 已入队或在途的 key
    pending: HashSet<String>,
    /// 提取失败（没有可用图标）的 key，避免每帧重试
    failed: HashSet<String>,
    /// 已加载的纹理，LRU 上限 MAX_ENTRIES
    ready: LruStore<egui::TextureHandle>,
    queue: VecDeque<ExtractRequest>,
    in_flight: usize,
}

impl IconStore {
    pub fn new() -> Self {
        let (request_tx, request_rx) = channel::<ExtractRequest>();
        let (result_tx, result_rx) = channel::<ExtractResponse>();

        let _ = std::thread::Builder::new()
            .name("deskman-icon-loader".to_string())
            .spawn(move || {
                init_worker_com();
                while let Ok(request) = request_rx.recv() {
                    let rgba = desktop::extract_icon_rgba(&request.path);
                    if result_tx
                        .send(ExtractResponse {
                            key: request.key,
                            rgba,
                        })
                        .is_err()
                    {
                        break;
                    }
                }
                finish_worker_com();
            });

        Self {
            tx: request_tx,
            rx: result_rx,
            pending: HashSet::new(),
            failed: HashSet::new(),
            ready: LruStore::new(MAX_ENTRIES),
            queue: VecDeque::new(),
            in_flight: 0,
        }
    }

    /// 取纹理；未加载时返回 None 并把请求排队（同一路径只排一次）
    pub fn texture(&mut self, ctx: &egui::Context, key: &str, path: &str) -> Option<egui::TextureHandle> {
        if let Some(texture) = self.ready.get_cloned(key) {
            return Some(texture);
        }
        if self.pending.contains(key) || self.failed.contains(key) {
            return None;
        }
        self.pending.insert(key.to_string());
        self.queue.push_back(ExtractRequest {
            key: key.to_string(),
            path: path.to_string(),
        });
        ctx.request_repaint();
        None
    }

    /// 每帧调用：限时消费回包、补发排队请求
    pub fn pump(&mut self, ctx: &egui::Context, budget: Duration) -> usize {
        let started = Instant::now();
        let mut loaded = 0usize;

        while started.elapsed() < budget {
            match self.rx.try_recv() {
                Ok(response) => {
                    self.in_flight = self.in_flight.saturating_sub(1);
                    // 请求期间被 prune 掉的条目直接丢弃结果，避免「复活」已删除项目
                    if !self.pending.remove(&response.key) {
                        continue;
                    }
                    match response.rgba {
                        Some((rgba, w, h)) if w > 0 && h > 0 && rgba.len() >= w * h * 4 => {
                            let image = egui::ColorImage::from_rgba_unmultiplied([w, h], &rgba);
                            let texture = ctx.load_texture(
                                format!("deskman_icon_{}", response.key),
                                image,
                                egui::TextureOptions::LINEAR,
                            );
                            self.ready.insert(response.key, texture);
                            loaded += 1;
                        }
                        _ => {
                            self.failed.insert(response.key);
                        }
                    }
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => break,
                Err(std::sync::mpsc::TryRecvError::Disconnected) => break,
            }
        }

        // 补发排队请求：只发仍在 pending 的，避免为已删除项目白跑提取
        while self.in_flight < MAX_IN_FLIGHT {
            let Some(request) = self.queue.pop_front() else {
                break;
            };
            if !self.pending.contains(&request.key) {
                continue;
            }
            if self.tx.send(request).is_err() {
                break;
            }
            self.in_flight += 1;
        }

        if loaded > 0 || self.in_flight > 0 || !self.queue.is_empty() {
            ctx.request_repaint_after(Duration::from_millis(16));
        }
        loaded
    }

    /// 与当前面板项目集合对齐：不在集合里的缓存/排队都丢掉
    pub fn prune_to(&mut self, keep: &HashSet<String>) {
        self.ready.prune_not_in(keep);
        self.failed.retain(|key| keep.contains(key));
        self.pending.retain(|key| keep.contains(key));
        self.queue.retain(|request| keep.contains(&request.key));
        // 仍在途的请求无法撤回，交给回包时按 pending 判定丢弃
    }

    pub fn clear(&mut self) {
        self.ready.clear();
        self.pending.clear();
        self.failed.clear();
        self.queue.clear();
    }

    /// 已缓存的纹理数（用于判断是否需要按当前项目集合裁剪）
    pub fn len(&self) -> usize {
        self.ready.len()
    }
}

#[cfg(windows)]
fn init_worker_com() {
    use windows_sys::Win32::System::Com::{CoInitializeEx, COINIT_APARTMENTTHREADED};
    // 图标提取走 Shell API（SHGetImageList），后台线程需要先初始化 COM；
    // 已初始化会返回 S_FALSE，忽略即可。
    unsafe {
        CoInitializeEx(std::ptr::null(), COINIT_APARTMENTTHREADED as u32);
    }
}

#[cfg(windows)]
fn finish_worker_com() {
    use windows_sys::Win32::System::Com::CoUninitialize;
    unsafe { CoUninitialize() };
}

#[cfg(not(windows))]
fn init_worker_com() {}

#[cfg(not(windows))]
fn finish_worker_com() {}

#[cfg(test)]
mod tests {
    use super::LruStore;
    use std::collections::HashSet;

    #[test]
    fn lru_evicts_least_recently_used() {
        let mut store = LruStore::new(2);
        store.insert("a".to_string(), 1);
        store.insert("b".to_string(), 2);
        // 访问 a 后，b 变成最久未用
        assert_eq!(store.get_cloned("a"), Some(1));
        store.insert("c".to_string(), 3);
        assert_eq!(store.len(), 2);
        assert!(store.contains("a"));
        assert!(!store.contains("b"));
        assert!(store.contains("c"));
    }

    #[test]
    fn lru_prune_keeps_only_listed_keys() {
        let mut store = LruStore::new(8);
        store.insert("a".to_string(), 1);
        store.insert("b".to_string(), 2);
        store.insert("c".to_string(), 3);
        let keep: HashSet<String> = ["a".to_string(), "c".to_string()].into_iter().collect();
        store.prune_not_in(&keep);
        assert_eq!(store.len(), 2);
        assert!(!store.contains("b"));
    }

    #[test]
    fn lru_capacity_is_at_least_one() {
        let mut store = LruStore::new(0);
        store.insert("a".to_string(), 1);
        store.insert("b".to_string(), 2);
        assert_eq!(store.len(), 1);
        assert!(store.contains("b"));
    }
}
