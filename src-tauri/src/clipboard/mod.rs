// 剪贴板同步：监听本机剪贴板变化，发给对端；对端写入本地。
// 防回环：收到自己发出的消息不转发（靠 Message.from 比对 + 本机标记位）。

// TODO(M3):
//   - Windows: AddClipboardFormatListener（WM_CLIPBOARDUPDATE）
//   - Mac: NSPasteboard changeCount 轮询 / 通知
//   - 文本 + PNG 图片（arboard crate）
//   - 写入本机剪贴板时挂"本机来源"标记，监听回调里跳过

/// 剪贴板同步器（骨架占位）。
pub struct ClipboardSync;

impl ClipboardSync {
    pub fn start(&self) {
        // TODO(M3)
    }

    pub fn set_enabled(&self, _enabled: bool) {
        // TODO(M3)
    }
}
