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

// ===== 懒传剪贴板文件（粘贴时才传输）=====

use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

/// 一次懒粘贴请求的共享结果槽。平台层在 WM_RENDERFORMAT / NSFilePromiseProvider
/// 回调里阻塞等传输完成；网络层（FileReceiver）在文件落地后填入结果并唤醒。
pub type PasteReady = Arc<(Mutex<Option<Result<Vec<String>, String>>>, Condvar)>;

/// 用户真的粘贴了懒传 offer：平台回调带着 offer id 与结果槽请求上层传输。
#[derive(Debug, Clone)]
pub enum ClipboardPasteEvent {
    Requested { id: String, ready: PasteReady },
}

pub type ClipboardPasteCallback = Arc<dyn Fn(ClipboardPasteEvent) + Send + Sync>;

pub fn new_paste_ready() -> PasteReady {
    Arc::new((Mutex::new(None), Condvar::new()))
}

/// 阻塞等待传输完成（平台回调线程调用）。超时兜底防止目标应用永久卡死。
pub fn wait_paste_ready(ready: &PasteReady, timeout: Duration) -> Result<Vec<String>, String> {
    let (lock, cv) = &**ready;
    let mut state = lock.lock().unwrap_or_else(|e| e.into_inner());
    let deadline = Instant::now() + timeout;
    loop {
        if let Some(result) = state.take() {
            return result;
        }
        let (next, _) = cv
            .wait_timeout(state, Duration::from_millis(500))
            .unwrap_or_else(|e| e.into_inner());
        state = next;
        if Instant::now() >= deadline {
            return Err("等待对端文件传输超时".to_string());
        }
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
