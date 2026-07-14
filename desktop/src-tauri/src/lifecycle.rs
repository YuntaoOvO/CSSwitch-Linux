//! 生命周期串行器（spec §8.1，与 native-entry §5.3 共用最小核心）。
//! 把所有会改 AppState/config 的操作（建/连接编辑/清 key/删/切/一键/停/ensure_proxy）
//! 串行化；探活刻意在锁外，用 generation token 防「被清除/取代后又拿旧 key 复活代理」。
//!
//! 三把锁分层，严格避免自死锁：
//!   1. 本串行器锁（`Mutex<()>`）= 最外层，命令级操作整段持有，**绝不重入**；
//!   2. `AppState` 锁 = 内层，读写运行态时短暂持有，探活期间释放；
//!   3. `config::update` 锁 = 最内层，仅盖 load-modify-save。
//!
//! ensure_proxy/start_proxy_for **绝不**取本串行器锁（其调用方命令才取），故不自锁。

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;

pub struct Lifecycle {
    lock: Mutex<()>,
    generation: AtomicU64,
}

impl Default for Lifecycle {
    fn default() -> Self {
        Self::new()
    }
}

impl Lifecycle {
    pub fn new() -> Self {
        Lifecycle {
            lock: Mutex::new(()),
            generation: AtomicU64::new(1),
        }
    }

    /// 在 app 级互斥下跑 f（读 config→spawn 等复合操作原子化）。poison 也照常恢复继续。
    pub fn with_serialized<T>(&self, f: impl FnOnce() -> T) -> T {
        let _g = self.lock.lock().unwrap_or_else(|e| e.into_inner());
        f()
    }

    /// 清 key / 停 / 切换时调用：使正在锁外探活的旧启动作废。返回新 generation。
    pub fn bump_generation(&self) -> u64 {
        self.generation.fetch_add(1, Ordering::SeqCst) + 1
    }

    #[allow(dead_code)]
    pub fn current_generation(&self) -> u64 {
        self.generation.load(Ordering::SeqCst)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicU64;
    use std::sync::Arc;

    #[test]
    fn with_serialized_runs_exclusively() {
        let lc = Arc::new(Lifecycle::new());
        let counter = Arc::new(AtomicU64::new(0));
        let max_seen = Arc::new(AtomicU64::new(0));
        let mut hs = vec![];
        for _ in 0..8 {
            let lc = lc.clone();
            let c = counter.clone();
            let m = max_seen.clone();
            hs.push(std::thread::spawn(move || {
                lc.with_serialized(|| {
                    let n = c.fetch_add(1, Ordering::SeqCst) + 1;
                    m.fetch_max(n, Ordering::SeqCst);
                    std::thread::sleep(std::time::Duration::from_millis(2));
                    c.fetch_sub(1, Ordering::SeqCst);
                });
            }));
        }
        for h in hs {
            h.join().unwrap();
        }
        assert_eq!(max_seen.load(Ordering::SeqCst), 1, "串行器内同时最多一个");
    }

    #[test]
    fn generation_bumps_monotonically() {
        let lc = Lifecycle::new();
        let g0 = lc.current_generation();
        let g1 = lc.bump_generation();
        assert!(g1 > g0);
        assert_eq!(lc.current_generation(), g1);
        let g2 = lc.bump_generation();
        assert!(g2 > g1);
    }
}
