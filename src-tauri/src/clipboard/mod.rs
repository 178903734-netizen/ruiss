// 剪贴板同步（M3）。
//
// 职责：
//   1. 监听本机剪贴板变化（platform::start_clipboard_watcher）
//      - 文字/图片变化 → 直接走网络发对端（ClipboardText / ClipboardImage）
//      - 文件路径变化 → 当前不传输；等待真正的跨机延迟粘贴 provider
//   2. 收到对端剪贴板消息 → 写入本机剪贴板（handle_remote）
//      - 文字 → clipboard_write_text
//      - 图片 → clipboard_write_image
//      （文件传输只由手动发送或原生拖放提交触发）
//
// 防回环：
//   - platform 层记录本机写入后的精确剪贴板版本，只跳过该版本
//   - 双重保险：对端消息携带 from，与本地 name 比对（lib.rs 路由层已过滤）
//
// 注意：剪贴板读写都在平台层，本模块只做协调，不直接碰 arboard/Win32。

use crate::core::protocol::{Message, Payload};
use crate::net::NetHandle;
use crate::platform::{self, ClipboardContent, ClipboardWatcherHandle};

/// 剪贴板同步器。Drop 时停止监听。
pub struct ClipboardSync {
    _watcher: ClipboardWatcherHandle,
}

impl ClipboardSync {
    /// 启动剪贴板同步。
    /// - name：本机名（消息 from 字段）
    /// - net：网络发送句柄
    pub fn start(name: String, net: NetHandle) -> Self {
        let net_cb = net.clone();
        let name_cb = name.clone();
        let watcher = platform::start_clipboard_watcher(Box::new(move |content| {
            match content {
                ClipboardContent::Text(text) => {
                    if !text.is_empty() {
                        net_cb.send(Message::clipboard(&name_cb, Payload::ClipboardText { text }));
                    }
                }
                ClipboardContent::Image(png) => {
                    if !png.is_empty() {
                        net_cb.send(Message::clipboard(&name_cb, Payload::ClipboardImage { png }));
                    }
                }
                ClipboardContent::Files(_) => {
                    // A file copy is only an offer, not permission to transfer its bytes.
                    // Proper cross-machine file paste needs a lazy virtual-file provider so
                    // transmission starts when the peer actually pastes. Until that provider
                    // exists, ignore file clipboard changes instead of downloading immediately.
                    net_cb.send(Message::clipboard(&name_cb, Payload::ClipboardClear));
                    log::debug!("[CLIPBOARD] 文件复制不自动传输，已让对端废弃旧剪贴板");
                }
                ClipboardContent::Empty => {}
            }
        }));
        Self { _watcher: watcher }
    }
}

/// 处理对端发来的剪贴板消息（文字/图片），写入本机剪贴板。
/// 文件传输消息（FileStart/Chunk/End）不经过这里，由 file_transfer 模块处理。
pub fn handle_remote(payload: &Payload) {
    match payload {
        Payload::ClipboardText { text } => {
            platform::clipboard_write_text(text);
        }
        Payload::ClipboardImage { png } => {
            platform::clipboard_write_image(png);
        }
        Payload::ClipboardClear => {
            platform::clipboard_clear();
        }
        Payload::ClipboardFiles { paths } => {
            // 轻量场景：对端只发路径（共享盘/同机双实例）。写入本机剪贴板文件列表。
            if !paths.is_empty() {
                platform::clipboard_write_files(paths);
            }
        }
        _ => {}
    }
}
