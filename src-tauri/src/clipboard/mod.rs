// 剪贴板同步。
//
// 职责：
//   1. 监听本机剪贴板变化（platform::start_clipboard_watcher）
//      - 文字直接发送；图片以 256 KiB 二进制块传输。
//      - 文件复制发送清单；接收端立即请求后台传输。
//   2. 收到对端剪贴板消息 → 写入本机剪贴板（handle_remote）
//      - 文字/图片 → 直接写
//      - 文件校验完成后发布真实本地路径，避免系统粘贴回调超时。
//
// 监听去重：Windows 一次复制会产生多条 WM_CLIPBOARDUPDATE（EmptyClipboard、
// CF_HDROP、PreferredDropEffect 各一次），内容相同、间隔毫秒级。按"内容 + 1 秒
// 时间窗"去重，防止一次复制被当成多次。
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
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

pub mod image;

/// 剪贴板同步器。Drop 时停止监听。
pub struct ClipboardSync {
    _watcher: ClipboardWatcherHandle,
}

/// 去重键：上次已发送的剪贴板内容摘要。
fn content_key(content: &ClipboardContent) -> Option<(u8, Vec<u8>)> {
    match content {
        ClipboardContent::Text(text) => Some((1, text.as_bytes().to_vec())),
        ClipboardContent::Image(png) => Some((2, png.clone())),
        ClipboardContent::Files(paths) => Some((3, paths.join("\0").into_bytes())),
        ClipboardContent::Empty => None,
    }
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
        let last_content: Arc<Mutex<Option<(Instant, u8, Vec<u8>)>>> = Arc::new(Mutex::new(None));
        let watcher = platform::start_clipboard_watcher(Box::new(move |content| {
            // 内容去重：同一次复制的多条通知（或毫秒级重复复制）只处理一次。
            let Some((kind, key)) = content_key(&content) else {
                // 剪贴板被清空：旧的文件 offer 随之失效。
                file_sender.clear_pending_offers();
                file_receiver.invalidate_clipboard_revision();
                *last_content.lock().unwrap_or_else(|e| e.into_inner()) = None;
                net_cb.send_ctrl(Message::clipboard(&name_cb, Payload::ClipboardClear));
                return;
            };
            {
                let mut last = last_content.lock().unwrap_or_else(|e| e.into_inner());
                if let Some((at, last_kind, last_key)) = last.as_ref() {
                    if *last_kind == kind
                        && last_key == &key
                        && at.elapsed() < Duration::from_secs(1)
                    {
                        return;
                    }
                }
                *last = Some((Instant::now(), kind, key));
            }
            // 分类日志：剪贴板跨屏出问题时，这行是判断"本机这次复制被识别成什么"的唯一线索
            // （文本/图片原来完全静默；文件在下面的 arm 里有更详细的日志）。
            // 注意：本文件是两端共用代码，Mac 端同样会打这行。
            match &content {
                ClipboardContent::Text(text) => {
                    log::info!(
                        "[CLIPBOARD] 本机复制文本 {} 字符 → 发送对端",
                        text.chars().count()
                    )
                }
                ClipboardContent::Image(png) => {
                    log::info!("[CLIPBOARD] 本机复制图片 {} 字节 PNG → 发送对端", png.len())
                }
                _ => {}
            }
            // A physical copy on this machine wins over an unfinished incoming copy.
            file_receiver.invalidate_clipboard_revision();
            match content {
                ClipboardContent::Text(text) => {
                    file_sender.clear_pending_offers();
                    if !text.is_empty() {
                        let id = uuid::Uuid::new_v4().to_string();
                        net_cb.send_ctrl(Message::clipboard(
                            &name_cb,
                            Payload::ClipboardText { id, text },
                        ));
                    }
                }
                ClipboardContent::Image(png) => {
                    file_sender.clear_pending_offers();
                    if !png.is_empty() {
                        let id = uuid::Uuid::new_v4().to_string();
                        image::send(&net_cb, &name_cb, id, png);
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
                    // 懒传：只发文件名清单，对端粘贴时才真正传输。
                    let names = paths
                        .iter()
                        .filter_map(|p| p.file_name().map(|n| n.to_string_lossy().into_owned()))
                        .collect::<Vec<_>>();
                    let id = file_sender.offer_clipboard_paths(paths);
                    log::info!("[CLIPBOARD] 本机复制文件 → 发出懒传 offer id={id} names={names:?}");
                    net_cb.send_ctrl(Message::clipboard(
                        &name_cb,
                        Payload::ClipboardFiles {
                            id,
                            paths: Vec::new(),
                            names,
                        },
                    ));
                }
                ClipboardContent::Empty => {}
            }
        }));
        Self { _watcher: watcher }
    }
}

/// 处理对端发来的剪贴板消息（文字/图片），写入本机剪贴板。
/// 文件传输消息（FileStart/Chunk/End）不经过这里，由 file_transfer 模块处理。
/// 文件清单到达即请求下载，用户可在传输完成后粘贴。
pub fn handle_remote(payload: &Payload, file_receiver: &FileReceiver, net: &NetHandle, name: &str) {
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
        Payload::ClipboardClear => {
            file_receiver.invalidate_clipboard_revision();
            platform::clipboard_clear();
        }
        Payload::ClipboardFiles { id, paths, names } => {
            if paths.is_empty() && !id.is_empty() && !names.is_empty() {
                // Download before publishing real local file URLs / CF_HDROP. Finder's
                // ordinary Paste does not consume drag promises, and Windows delayed
                // clipboard rendering has a system timeout independent of our timeout.
                log::info!(
                    "[CLIPBOARD] 收到文件清单 id={id} names={names:?} → 后台接收，完成后可粘贴"
                );
                if file_receiver.begin_clipboard_revision(id) {
                    platform::clipboard_clear();
                    net.send_ctrl(Message::ctrl(
                        name,
                        Payload::ClipboardFileRequest { id: id.clone() },
                    ));
                } else {
                    log::warn!("[CLIPBOARD] offer id={id} 已被更新的剪贴板内容取代，忽略");
                }
                return;
            }
            // 轻量场景：对端只发路径（共享盘/同机双实例）。写入本机剪贴板文件列表。
            if !paths.is_empty() && (id.is_empty() || file_receiver.begin_clipboard_revision(id)) {
                if id.is_empty() {
                    file_receiver.invalidate_clipboard_revision();
                }
                if let Err(error) = platform::clipboard_write_files(paths) {
                    log::error!("[CLIPBOARD] {error}");
                }
            }
        }
        _ => {}
    }
}
