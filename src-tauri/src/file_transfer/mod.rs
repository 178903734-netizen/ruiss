// 文件传输（M3）：跨屏发文件。
//
// 发送端（FileSender）：把文件分块（256KB/块）通过 TCP 发出
//   FileStart → FileChunk* → FileEnd
// 触发场景：
//   1. 剪贴板监听到本机复制了文件路径（clipboard 模块回调 on_files）
//   2. GUI 手动选文件发送（send_file 命令）
//   3. 跨屏拖拽（drag 模块触发）
//
// 接收端（FileReceiver）：状态机处理 FileStart/Chunk/End，写入下载目录，
//   完成后路径写入本机剪贴板（用户可直接 Ctrl+V 粘贴文件）+ 通知前端。
//
// 块大小 256KB：单帧 16MB 上限内富余，TCP 吞吐与帧数平衡。
// 乱序/丢块 → 整文件作废取消（TCP 可靠有序，正常不会丢；防御性处理）。

use std::collections::HashMap;
use std::io::Write;
use std::path::PathBuf;
use std::sync::Mutex;

use tauri::{AppHandle, Emitter};

use crate::core::protocol::{Message, Payload};
use crate::net::NetHandle;
use crate::platform;

const CHUNK_SIZE: usize = 256 * 1024;

/// 文件发送器：无状态，每次发送 spawn 一个异步任务。
#[derive(Clone)]
pub struct FileSender {
    net: NetHandle,
    name: String,
}

impl FileSender {
    pub fn new(net: NetHandle, name: String) -> Self {
        Self { net, name }
    }

    /// 异步发送一个文件（不阻塞调用方）。
    pub fn send_file(&self, path: PathBuf) {
        let net = self.net.clone();
        let name = self.name.clone();
        tauri::async_runtime::spawn(async move {
            let meta = match tokio::fs::metadata(&path).await {
                Ok(m) => m,
                Err(e) => {
                    log::error!("[FILE] 读文件信息失败 {path:?}: {e}");
                    return;
                }
            };
            if !meta.is_file() {
                log::warn!("[FILE] 不是普通文件，跳过: {path:?}");
                return;
            }
            let size = meta.len();
            let file_name = path
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_else(|| "file".into());
            let id = uuid::Uuid::new_v4().to_string();

            log::info!("[FILE] 开始发送: {file_name} ({size} 字节) id={id}");
            net.send(Message::clipboard(
                &name,
                Payload::FileStart { id: id.clone(), name: file_name.clone(), size },
            ));

            use tokio::io::AsyncReadExt;
            let mut file = match tokio::fs::File::open(&path).await {
                Ok(f) => f,
                Err(e) => {
                    log::error!("[FILE] 打开文件失败: {e}");
                    net.send(Message::clipboard(&name, Payload::FileCancel { id }));
                    return;
                }
            };
            let mut buf = vec![0u8; CHUNK_SIZE];
            let mut seq = 0u32;
            loop {
                let n = match file.read(&mut buf).await {
                    Ok(0) => break,
                    Ok(n) => n,
                    Err(e) => {
                        log::error!("[FILE] 读块失败: {e}");
                        net.send(Message::clipboard(&name, Payload::FileCancel { id }));
                        return;
                    }
                };
                net.send(Message::clipboard(
                    &name,
                    Payload::FileChunk { id: id.clone(), seq, data: buf[..n].to_vec() },
                ));
                seq += 1;
            }
            net.send(Message::clipboard(&name, Payload::FileEnd { id: id.clone() }));
            log::info!("[FILE] 发送完成: {file_name} ({seq} 块)");
        });
    }
}

/// 接收中的单个文件状态。
struct ReceiveState {
    file: Option<std::fs::File>,
    name: String,
    expected: u64,
    written: u64,
    next_seq: u32,
    path: PathBuf,
}

/// 文件接收器：处理对端 FileStart/Chunk/End，写盘 + 完成写剪贴板 + 通知前端。
pub struct FileReceiver {
    state: Mutex<HashMap<String, ReceiveState>>,
    download_dir: PathBuf,
    app: AppHandle,
}

impl FileReceiver {
    pub fn new(app: AppHandle) -> Self {
        let download_dir = dirs::download_dir()
            .or_else(|| dirs::home_dir())
            .unwrap_or_else(|| std::env::temp_dir());
        log::info!("[FILE] 下载目录: {}", download_dir.display());
        Self {
            state: Mutex::new(HashMap::new()),
            download_dir,
            app,
        }
    }

    /// 处理对端文件消息。返回 true 表示已处理（属于文件传输消息）。
    pub fn handle(&self, payload: &Payload) -> bool {
        match payload {
            Payload::FileStart { id, name, size } => {
                self.on_start(id, name, *size);
                true
            }
            Payload::FileChunk { id, seq, data } => {
                self.on_chunk(id, *seq, data);
                true
            }
            Payload::FileEnd { id } => {
                self.on_end(id);
                true
            }
            Payload::FileCancel { id } => {
                let mut st = self.state.lock().unwrap();
                if st.remove(id).is_some() {
                    log::warn!("[FILE] 对端取消传输 id={id}");
                }
                true
            }
            _ => false,
        }
    }

    fn on_start(&self, id: &str, name: &str, size: u64) {
        // 重名处理：foo.txt → foo (1).txt
        let path = unique_path(&self.download_dir, name);
        let file = std::fs::File::create(&path).ok();
        if file.is_none() {
            log::error!("[FILE] 无法创建文件: {}", path.display());
            return;
        }
        log::info!("[FILE] 开始接收: {name} ({size} 字节) → {}", path.display());
        self.state.lock().unwrap().insert(
            id.to_string(),
            ReceiveState {
                file,
                name: name.to_string(),
                expected: size,
                written: 0,
                next_seq: 0,
                path,
            },
        );
    }

    fn on_chunk(&self, id: &str, seq: u32, data: &[u8]) {
        let mut st = self.state.lock().unwrap();
        let Some(s) = st.get_mut(id) else { return };
        if seq != s.next_seq {
            log::warn!(
                "[FILE] 块乱序，丢弃整文件: id={id} 期望 {} 收到 {seq}",
                s.next_seq
            );
            let name = s.name.clone();
            drop(st);
            self.cancel(id, &name);
            return;
        }
        if let Some(f) = s.file.as_mut() {
            if f.write_all(data).is_ok() {
                s.written += data.len() as u64;
                s.next_seq += 1;
            }
        }
    }

    fn on_end(&self, id: &str) {
        let removed = self.state.lock().unwrap().remove(id);
        let Some(mut s) = removed else { return };
        if let Some(f) = s.file.as_mut() {
            let _ = f.flush();
        }
        s.file = None; // 关闭文件
        if s.written == s.expected {
            log::info!("[FILE] 接收完成: {} ({} 字节)", s.name, s.written);
            self.complete(&s.name, s.path.clone());
        } else {
            log::warn!(
                "[FILE] 大小不符: {} 期望 {} 实际 {}",
                s.name, s.expected, s.written
            );
            // 仍写入剪贴板，让用户能用部分文件（可选）
            self.complete(&s.name, s.path.clone());
        }
    }

    fn cancel(&self, id: &str, _name: &str) {
        self.state.lock().unwrap().remove(id);
        // 文件已部分写入，保留在下载目录（用户可手动清理）
    }

    fn complete(&self, name: &str, path: PathBuf) {
        // 路径写入本机剪贴板：用户可直接 Ctrl+V 粘贴文件
        let path_str = path.to_string_lossy().into_owned();
        platform::clipboard_write_files(&[path_str.clone()]);
        // 通知前端显示传输记录
        let _ = self.app.emit(
            "file-received",
            serde_json::json!({ "name": name, "path": path_str }),
        );
    }
}

/// 在 dir 下找一个不冲突的文件名：foo.txt → foo (1).txt → foo (2).txt ...
fn unique_path(dir: &PathBuf, name: &str) -> PathBuf {
    let candidate = dir.join(name);
    if !candidate.exists() {
        return candidate;
    }
    let (stem, ext) = match name.rfind('.') {
        Some(i) if i > 0 => (&name[..i], &name[i..]),
        _ => (name, ""),
    };
    for i in 1..1000 {
        let new_name = format!("{stem} ({i}){ext}");
        let p = dir.join(&new_name);
        if !p.exists() {
            return p;
        }
    }
    candidate
}
