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
use crate::file_transfer::{FileReceiver, FileSender};
use crate::net::NetHandle;
use crate::platform::{self, ClipboardContent, ClipboardWatcherHandle};
use std::path::PathBuf;
use std::sync::Arc;

/// 剪贴板同步器。Drop 时停止监听。
pub struct ClipboardSync {
    _watcher: ClipboardWatcherHandle,
}

impl ClipboardSync {
    /// 启动剪贴板同步。
    /// - name：本机名（消息 from 字段）
    /// - net：网络发送句柄
    pub fn start(
        name: String,
        net: NetHandle,
        file_sender: FileSender,
        file_receiver: Arc<FileReceiver>,
    ) -> Self {
        let net_cb = net.clone();
        let name_cb = name.clone();
        let watcher = platform::start_clipboard_watcher(Box::new(move |content| {
            // A physical copy on this machine wins over an unfinished incoming copy.
            file_receiver.invalidate_clipboard_revision();
            file_sender.cancel_clipboard_transfer();
            match content {
                ClipboardContent::Text(text) => {
                    if !text.is_empty() {
                        let id = uuid::Uuid::new_v4().to_string();
                        net_cb.send_ctrl(Message::clipboard(
                            &name_cb,
                            Payload::ClipboardText { id, text },
                        ));
                    }
                }
                ClipboardContent::Image(png) => {
                    if !png.is_empty() {
                        let id = uuid::Uuid::new_v4().to_string();
                        net_cb.send_ctrl(Message::clipboard(
                            &name_cb,
                            Payload::ClipboardImage { id, png },
                        ));
                    }
                }
                ClipboardContent::Files(paths) => {
                    let paths = paths
                        .into_iter()
                        .map(PathBuf::from)
                        .filter(|path| path.exists())
                        .collect::<Vec<_>>();
                    if paths.is_empty() {
                        return;
                    }
                    let id = uuid::Uuid::new_v4().to_string();
                    net_cb.send_ctrl(Message::clipboard(
                        &name_cb,
                        Payload::ClipboardFileOffer { id: id.clone() },
                    ));
                    if let Err(error) = file_sender.send_clipboard_paths(id, paths) {
                        log::error!("[CLIPBOARD] unable to start file copy: {error}");
                    }
                }
                ClipboardContent::Empty => {}
            }
        }));
        Self { _watcher: watcher }
    }
}

/// 处理对端发来的剪贴板消息（文字/图片），写入本机剪贴板。
/// 文件传输消息（FileStart/Chunk/End）不经过这里，由 file_transfer 模块处理。
pub fn handle_remote(payload: &Payload, file_receiver: &FileReceiver) {
    match payload {
        Payload::ClipboardText { id, text } => {
            if id.is_empty() {
                file_receiver.invalidate_clipboard_revision();
                platform::clipboard_write_text(text);
            } else if file_receiver.begin_clipboard_revision(id) {
                platform::clipboard_write_text(text);
            }
        }
        Payload::ClipboardImage { id, png } => {
            if id.is_empty() {
                file_receiver.invalidate_clipboard_revision();
                platform::clipboard_write_image(png);
            } else if file_receiver.begin_clipboard_revision(id) {
                platform::clipboard_write_image(png);
            }
        }
        Payload::ClipboardFileOffer { id } => {
            if file_receiver.begin_clipboard_revision(id) {
                platform::clipboard_clear();
            }
        }
        Payload::ClipboardClear => {
            file_receiver.invalidate_clipboard_revision();
            platform::clipboard_clear();
        }
        Payload::ClipboardFiles { id, paths } => {
            // 轻量场景：对端只发路径（共享盘/同机双实例）。写入本机剪贴板文件列表。
            if !paths.is_empty() && (id.is_empty() || file_receiver.begin_clipboard_revision(id)) {
                if id.is_empty() {
                    file_receiver.invalidate_clipboard_revision();
                }
                platform::clipboard_write_files(paths);
            }
        }
        _ => {}
    }
}
