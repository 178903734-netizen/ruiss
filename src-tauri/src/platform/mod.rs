// platform：事件捕获与注入的平台实现。
// 骨架阶段仅声明模块接口；M1 里程碑填入真实实现。
// Windows 侧：SetWindowsHookEx（捕获） + SendInput（注入）
// Mac 侧：CGEventTap（捕获） + CGEventPost（注入）

#[cfg(target_os = "windows")]
mod win;
#[cfg(target_os = "windows")]
pub use win::*;

#[cfg(target_os = "macos")]
mod mac;
#[cfg(target_os = "macos")]
pub use mac::*;

#[derive(Debug, Clone)]
pub enum RemoteFileDragEvent {
    DataRequested(String),
    Cancelled(String),
}

// ===== Native file-promise completion shared by all roots in a drag =====

use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

/// Native drag promise result. Multiple roots share one completed download.
pub type PasteReady = Arc<(Mutex<Option<Result<Vec<String>, String>>>, Condvar)>;

pub fn new_paste_ready() -> PasteReady {
    Arc::new((Mutex::new(None), Condvar::new()))
}

/// 阻塞等待传输完成（平台回调线程调用）。超时兜底防止目标应用永久卡死。
pub fn wait_paste_ready(ready: &PasteReady, timeout: Duration) -> Result<Vec<String>, String> {
    let (lock, cv) = &**ready;
    let mut state = lock.lock().unwrap_or_else(|e| e.into_inner());
    let deadline = Instant::now() + timeout;
    loop {
        if let Some(result) = state.as_ref() {
            // One native drag has a promise per root; all share the same download.
            return result.clone();
        }
        let (next, _) = cv
            .wait_timeout(
                state,
                deadline
                    .saturating_duration_since(Instant::now())
                    .min(Duration::from_millis(500)),
            )
            .unwrap_or_else(|e| e.into_inner());
        state = next;
        if Instant::now() >= deadline {
            return Err("等待对端文件传输超时".to_string());
        }
    }
}

#[cfg(test)]
mod promise_tests {
    use super::*;

    #[test]
    fn multiple_promises_share_one_completed_transfer() {
        let ready = new_paste_ready();
        complete_paste_ready(&ready, Ok(vec!["one.txt".into(), "two.txt".into()]));
        let first = wait_paste_ready(&ready, Duration::ZERO).unwrap();
        let second = wait_paste_ready(&ready, Duration::ZERO).unwrap();
        assert_eq!(first, second);
        assert_eq!(first.len(), 2);
    }

    #[test]
    fn promise_failure_is_shared_with_all_waiters() {
        let ready = new_paste_ready();
        complete_paste_ready(&ready, Err("cancelled".into()));
        assert_eq!(
            wait_paste_ready(&ready, Duration::ZERO),
            Err("cancelled".into())
        );
        assert_eq!(
            wait_paste_ready(&ready, Duration::ZERO),
            Err("cancelled".into())
        );
    }
}

/// 网络层在文件传输结束时填入结果并唤醒等待的平台回调线程。
pub fn complete_paste_ready(ready: &PasteReady, result: Result<Vec<String>, String>) {
    let (lock, cv) = &**ready;
    *lock.lock().unwrap_or_else(|e| e.into_inner()) = Some(result);
    cv.notify_all();
}

/// 剪贴板内容快照（平台无关）。
/// 读取时按 files > image > text 优先级返回当前剪贴板里"最高级"的那一种；
/// 写入时由调用方指定类型。监听器变化回调也用这个类型。
#[derive(Debug, Clone)]
pub enum ClipboardContent {
    Empty,
    Text(String),
    /// PNG 编码字节
    Image(Vec<u8>),
    /// 文件绝对路径列表
    Files(Vec<String>),
}

impl ClipboardContent {
    pub fn is_empty(&self) -> bool {
        matches!(self, ClipboardContent::Empty)
    }
}

/// 剪贴板监听器句柄，Drop 时停止监听线程。
pub struct ClipboardWatcherHandle {
    stop: Option<Box<dyn FnOnce() + Send>>,
}

impl ClipboardWatcherHandle {
    pub fn stop(mut self) {
        if let Some(f) = self.stop.take() {
            f();
        }
    }
}

impl Drop for ClipboardWatcherHandle {
    fn drop(&mut self) {
        if let Some(f) = self.stop.take() {
            f();
        }
    }
}
