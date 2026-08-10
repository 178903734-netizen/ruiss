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
