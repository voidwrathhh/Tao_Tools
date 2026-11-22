//! 桌面目录的实时变更监视。
//!
//! 实现方式是后台线程定时（1 秒）读取桌面目录快照并与上次比较，变化时把最新
//! 快照经通道发给 UI 线程；UI 侧只做「新增补进未分类」。没有用
//! `ReadDirectoryChangesW`：轮询实现不依赖 overlapped IO、线程退出不会阻塞在
//! 未完成的读操作上，1 秒间隔足以满足「新建文件 2 秒内出现」的验收，
//! 且目录本身很小（几十项），开销可忽略。

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{channel, Receiver};
use std::sync::{Arc, Mutex};
use std::time::Duration;

const POLL_INTERVAL: Duration = Duration::from_millis(1000);

pub struct DesktopWatcher {
    rx: Receiver<Vec<PathBuf>>,
    roots: Arc<Mutex<Vec<PathBuf>>>,
    stop: Arc<AtomicBool>,
}

impl DesktopWatcher {
    /// 启动后台监视线程；`roots` 可在运行中通过 `set_roots` 更新
    pub fn spawn(roots: Vec<PathBuf>) -> Self {
        let (tx, rx) = channel();
        let roots = Arc::new(Mutex::new(roots));
        let stop = Arc::new(AtomicBool::new(false));

        let thread_roots = Arc::clone(&roots);
        let thread_stop = Arc::clone(&stop);
        let _ = std::thread::Builder::new()
            .name("deskman-desktop-watch".to_string())
            .spawn(move || {
                let mut last: Option<Vec<PathBuf>> = None;
                while !thread_stop.load(Ordering::Relaxed) {
                    std::thread::sleep(POLL_INTERVAL);
                    if thread_stop.load(Ordering::Relaxed) {
                        break;
                    }
                    let current_roots = match thread_roots.lock() {
                        Ok(guard) => guard.clone(),
                        Err(_) => Vec::new(),
                    };
                    let snapshot = crate::desktop::snapshot_paths(&current_roots);
                    if last.as_deref() != Some(snapshot.as_slice()) {
                        last = Some(snapshot.clone());
                        if tx.send(snapshot).is_err() {
                            break;
                        }
                    }
                }
            });

        Self { rx, roots, stop }
    }

    /// 更新监视目录（配置里切换用户/公共桌面后生效）
    pub fn set_roots(&self, roots: Vec<PathBuf>) {
        if let Ok(mut guard) = self.roots.lock() {
            if *guard != roots {
                *guard = roots;
            }
        }
    }

    /// 取一次待处理快照（没有变化时返回 None）
    pub fn try_recv(&self) -> Option<Vec<PathBuf>> {
        self.rx.try_recv().ok()
    }
}

impl Drop for DesktopWatcher {
    fn drop(&mut self) {
        // 线程最多再睡 1 秒后自然退出；这里不 join，避免关闭窗口被阻塞
        self.stop.store(true, Ordering::Relaxed);
    }
}
