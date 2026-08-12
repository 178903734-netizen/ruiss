use std::collections::{HashMap, HashSet};
use std::io::Write;
use std::path::{Component, Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use sha2::{Digest, Sha256};
use tauri::{AppHandle, Emitter};
use tokio::sync::{oneshot, Semaphore};

use crate::core::protocol::{Message, Payload, TransferRoot};
use crate::net::NetHandle;
use crate::platform;

const CHUNK_SIZE: usize = 1024 * 1024;
const RESULT_TIMEOUT: Duration = Duration::from_secs(120);

#[derive(Clone)]
pub struct FileSender {
    inner: Arc<SenderInner>,
}

struct SenderInner {
    net: NetHandle,
    name: String,
    app: AppHandle,
    waiters: Mutex<HashMap<String, oneshot::Sender<Result<(), String>>>>,
    cancelled: Mutex<HashSet<String>>,
    retries: Mutex<HashMap<String, Vec<PathBuf>>>,
    pending_drags: Mutex<HashMap<String, Vec<PathBuf>>>,
    gate: Semaphore,
}

struct SendPlan {
    roots: Vec<TransferRoot>,
    directories: Vec<String>,
    files: Vec<SendFile>,
    total_bytes: u64,
}

struct SendFile {
    source: PathBuf,
    relative: String,
    size: u64,
}

impl FileSender {
    pub fn new(net: NetHandle, name: String, app: AppHandle) -> Self {
        Self {
            inner: Arc::new(SenderInner {
                net,
                name,
                app,
                waiters: Mutex::new(HashMap::new()),
                cancelled: Mutex::new(HashSet::new()),
                retries: Mutex::new(HashMap::new()),
                pending_drags: Mutex::new(HashMap::new()),
                gate: Semaphore::new(1),
            }),
        }
    }

    pub fn send_paths(&self, paths: Vec<PathBuf>) -> Result<String, String> {
        self.enqueue_paths(paths, false, None)
    }

    pub fn send_drag_session(
        &self,
        drag_id: String,
        paths: Vec<PathBuf>,
    ) -> Result<String, String> {
        self.enqueue_paths(paths, false, Some(drag_id))
    }

    pub fn offer_drag_paths(&self, paths: Vec<PathBuf>) -> Result<String, String> {
        if paths.is_empty() || !self.inner.net.connected() {
            return Err("no connected file drag".into());
        }
        let id = uuid::Uuid::new_v4().to_string();
        self.inner
            .pending_drags
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(id.clone(), paths.clone());
        let sender = self.clone();
        let task_id = id.clone();
        tauri::async_runtime::spawn(async move {
            let result = tokio::task::spawn_blocking(move || build_plan(paths)).await;
            match result {
                Ok(Ok(plan)) => {
                    sender.inner.net.send_ctrl(Message::ctrl(
                        &sender.inner.name,
                        Payload::DragStart {
                            id: task_id.clone(),
                            roots: plan.roots,
                        },
                    ));
                }
                Ok(Err(error)) => {
                    log::error!("[DRAG] unable to prepare drag {task_id}: {error}");
                    sender.cancel_drag(&task_id);
                }
                Err(error) => {
                    log::error!("[DRAG] drag preparation task failed {task_id}: {error}");
                    sender.cancel_drag(&task_id);
                }
            }
        });
        Ok(id)
    }

    pub fn commit_drag(&self, id: &str) -> Result<String, String> {
        let paths = self
            .inner
            .pending_drags
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(id)
            .ok_or_else(|| "drag session is no longer available".to_string())?;
        self.send_drag_session(id.to_string(), paths)
    }

    pub fn cancel_drag(&self, id: &str) {
        self.inner
            .pending_drags
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(id);
    }

    fn enqueue_paths(
        &self,
        paths: Vec<PathBuf>,
        drop_at_cursor: bool,
        drag_id: Option<String>,
    ) -> Result<String, String> {
        if paths.is_empty() {
            return Err("没有选择文件或文件夹".into());
        }
        if !self.inner.net.connected() {
            return Err("对端未连接，无法发送".into());
        }
        let id = uuid::Uuid::new_v4().to_string();
        self.inner
            .retries
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(id.clone(), paths.clone());
        emit_update(
            &self.inner.app,
            &id,
            "send",
            "准备传输",
            "preparing",
            0,
            0,
            0,
            0,
            None,
            None,
        );
        let sender = self.clone();
        let task_id = id.clone();
        tauri::async_runtime::spawn(async move {
            if let Err(error) = sender
                .run_batch(task_id.clone(), paths, drop_at_cursor, drag_id)
                .await
            {
                log::error!("[FILE] 发送失败 batch={task_id}: {error}");
                sender.inner.net.send_ctrl(Message::clipboard(
                    &sender.inner.name,
                    Payload::FileBatchCancel {
                        id: task_id.clone(),
                    },
                ));
                let status = if error == "传输已取消" {
                    "cancelled"
                } else {
                    "failed"
                };
                emit_update(
                    &sender.inner.app,
                    &task_id,
                    "send",
                    "文件传输",
                    status,
                    0,
                    0,
                    0,
                    0,
                    Some(error),
                    None,
                );
            }
            sender
                .inner
                .cancelled
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .remove(&task_id);
            sender
                .inner
                .waiters
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .retain(|_, waiter| !waiter.is_closed());
        });
        Ok(id)
    }

    pub fn retry(&self, id: &str) -> Result<String, String> {
        let paths = self
            .inner
            .retries
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(id)
            .cloned()
            .ok_or_else(|| "找不到可重试的传输任务".to_string())?;
        let new_id = self.send_paths(paths)?;
        self.inner
            .retries
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(id);
        Ok(new_id)
    }

    pub fn cancel(&self, id: &str) {
        self.inner
            .cancelled
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(id.to_string());
        self.inner.net.send_ctrl(Message::clipboard(
            &self.inner.name,
            Payload::FileBatchCancel { id: id.to_string() },
        ));
    }

    pub fn handle_result(&self, payload: &Payload) -> bool {
        let (id, result) = match payload {
            Payload::FileReady { id, ok, error }
            | Payload::FileResult { id, ok, error }
            | Payload::FileBatchReady { id, ok, error }
            | Payload::FileBatchResult { id, ok, error, .. } => (
                id,
                if *ok {
                    Ok(())
                } else {
                    Err(error.clone().unwrap_or_else(|| "对端接收失败".into()))
                },
            ),
            _ => return false,
        };
        if let Some(tx) = self
            .inner
            .waiters
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(id)
        {
            let _ = tx.send(result);
        }
        true
    }

    async fn run_batch(
        &self,
        batch_id: String,
        paths: Vec<PathBuf>,
        drop_at_cursor: bool,
        drag_id: Option<String>,
    ) -> Result<(), String> {
        let _permit = self
            .inner
            .gate
            .acquire()
            .await
            .map_err(|_| "文件传输队列已关闭".to_string())?;
        self.ensure_not_cancelled(&batch_id)?;
        let plan = tokio::task::spawn_blocking(move || build_plan(paths))
            .await
            .map_err(|e| format!("整理文件失败: {e}"))??;
        self.ensure_not_cancelled(&batch_id)?;
        let title = plan
            .roots
            .iter()
            .map(|r| r.name.as_str())
            .collect::<Vec<_>>()
            .join("、");
        let total_files = u32::try_from(plan.files.len()).unwrap_or(u32::MAX);

        let ready = self.register_waiter(&batch_id);
        self.send_payload(Payload::FileBatchStart {
            id: batch_id.clone(),
            roots: plan.roots.clone(),
            total_files,
            total_bytes: plan.total_bytes,
            drop_at_cursor,
            drag_id,
        })
        .await?;
        self.wait_result(ready, &batch_id, &batch_id).await?;
        for path in &plan.directories {
            self.ensure_not_cancelled(&batch_id)?;
            self.send_payload(Payload::FileDirectory {
                batch_id: batch_id.clone(),
                path: path.clone(),
            })
            .await?;
        }

        let mut transferred = 0u64;
        let mut files_done = 0u32;
        for entry in plan.files {
            self.ensure_not_cancelled(&batch_id)?;
            let file_id = uuid::Uuid::new_v4().to_string();
            let ready = self.register_waiter(&file_id);
            self.send_payload(Payload::FileStart {
                id: file_id.clone(),
                batch_id: batch_id.clone(),
                path: entry.relative.clone(),
                size: entry.size,
            })
            .await?;
            self.wait_result(ready, &file_id, &batch_id).await?;

            let completion = self.register_waiter(&file_id);
            let mut file = tokio::fs::File::open(&entry.source)
                .await
                .map_err(|e| format!("无法打开 {}: {e}", entry.source.display()))?;
            let mut hasher = Sha256::new();
            let mut seq = 0u32;
            let mut buf = vec![0u8; CHUNK_SIZE];
            let mut last_emit = Instant::now() - Duration::from_secs(1);
            use tokio::io::AsyncReadExt;
            loop {
                self.ensure_not_cancelled(&batch_id)?;
                let n = file
                    .read(&mut buf)
                    .await
                    .map_err(|e| format!("读取 {} 失败: {e}", entry.source.display()))?;
                if n == 0 {
                    break;
                }
                hasher.update(&buf[..n]);
                self.send_payload(Payload::FileChunk {
                    id: file_id.clone(),
                    seq,
                    data: buf[..n].to_vec(),
                })
                .await?;
                seq = seq.saturating_add(1);
                transferred = transferred.saturating_add(n as u64);
                if last_emit.elapsed() >= Duration::from_millis(100) {
                    emit_update(
                        &self.inner.app,
                        &batch_id,
                        "send",
                        &title,
                        "transferring",
                        transferred,
                        plan.total_bytes,
                        files_done,
                        total_files,
                        None,
                        None,
                    );
                    last_emit = Instant::now();
                }
            }
            self.send_payload(Payload::FileEnd {
                id: file_id.clone(),
                sha256: format!("{:x}", hasher.finalize()),
            })
            .await?;
            self.wait_result(completion, &file_id, &batch_id).await?;
            files_done = files_done.saturating_add(1);
            emit_update(
                &self.inner.app,
                &batch_id,
                "send",
                &title,
                "transferring",
                transferred,
                plan.total_bytes,
                files_done,
                total_files,
                None,
                None,
            );
        }

        let rx = self.register_waiter(&batch_id);
        self.send_payload(Payload::FileBatchEnd {
            id: batch_id.clone(),
        })
        .await?;
        self.wait_result(rx, &batch_id, &batch_id).await?;
        emit_update(
            &self.inner.app,
            &batch_id,
            "send",
            &title,
            "completed",
            plan.total_bytes,
            plan.total_bytes,
            total_files,
            total_files,
            None,
            None,
        );
        self.inner
            .cancelled
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(&batch_id);
        self.inner
            .retries
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(&batch_id);
        Ok(())
    }

    fn register_waiter(&self, id: &str) -> oneshot::Receiver<Result<(), String>> {
        let (tx, rx) = oneshot::channel();
        self.inner
            .waiters
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(id.to_string(), tx);
        rx
    }

    fn ensure_not_cancelled(&self, id: &str) -> Result<(), String> {
        if self
            .inner
            .cancelled
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .contains(id)
        {
            Err("传输已取消".into())
        } else if !self.inner.net.connected() {
            Err("网络连接已断开".into())
        } else {
            Ok(())
        }
    }

    async fn send_payload(&self, payload: Payload) -> Result<(), String> {
        self.inner
            .net
            .send_file(Message::clipboard(&self.inner.name, payload))
            .await
    }

    async fn wait_result(
        &self,
        mut rx: oneshot::Receiver<Result<(), String>>,
        waiter_id: &str,
        batch_id: &str,
    ) -> Result<(), String> {
        let deadline = tokio::time::Instant::now() + RESULT_TIMEOUT;
        loop {
            tokio::select! {
                result = &mut rx => {
                    return match result {
                        Ok(result) => result,
                        Err(_) => Err(format!("传输确认通道已关闭: {waiter_id}")),
                    };
                }
                _ = tokio::time::sleep(Duration::from_millis(100)) => {
                    if let Err(error) = self.ensure_not_cancelled(batch_id) {
                        self.inner.waiters.lock().unwrap_or_else(|e| e.into_inner()).remove(waiter_id);
                        return Err(error);
                    }
                    if tokio::time::Instant::now() >= deadline {
                        self.inner.waiters.lock().unwrap_or_else(|e| e.into_inner()).remove(waiter_id);
                        return Err(format!("等待对端确认超时: {waiter_id}"));
                    }
                }
            }
        }
    }
}

fn build_plan(paths: Vec<PathBuf>) -> Result<SendPlan, String> {
    let mut roots = Vec::new();
    let mut directories = Vec::new();
    let mut files = Vec::new();
    let mut used = HashSet::new();
    for source in paths {
        let metadata = std::fs::symlink_metadata(&source)
            .map_err(|e| format!("无法读取 {}: {e}", source.display()))?;
        if metadata.file_type().is_symlink() {
            return Err(format!("暂不传输符号链接: {}", source.display()));
        }
        let original = source
            .file_name()
            .and_then(|s| s.to_str())
            .filter(|s| !s.is_empty())
            .ok_or_else(|| format!("无效路径: {}", source.display()))?;
        let root_name = unique_logical_name(original, &mut used);
        if metadata.is_dir() {
            roots.push(TransferRoot {
                name: root_name.clone(),
                is_dir: true,
            });
            directories.push(root_name.clone());
            walk_directory(&source, &root_name, &mut directories, &mut files)?;
        } else if metadata.is_file() {
            roots.push(TransferRoot {
                name: root_name.clone(),
                is_dir: false,
            });
            files.push(SendFile {
                source,
                relative: root_name,
                size: metadata.len(),
            });
        } else {
            return Err("只支持普通文件和文件夹".into());
        }
    }
    let total_bytes = files.iter().map(|f| f.size).sum();
    Ok(SendPlan {
        roots,
        directories,
        files,
        total_bytes,
    })
}

fn walk_directory(
    dir: &Path,
    relative: &str,
    directories: &mut Vec<String>,
    files: &mut Vec<SendFile>,
) -> Result<(), String> {
    let mut entries = std::fs::read_dir(dir)
        .map_err(|e| format!("无法读取文件夹 {}: {e}", dir.display()))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| format!("读取文件夹失败: {e}"))?;
    entries.sort_by_key(|e| e.file_name());
    for entry in entries {
        let path = entry.path();
        let metadata = std::fs::symlink_metadata(&path)
            .map_err(|e| format!("无法读取 {}: {e}", path.display()))?;
        if metadata.file_type().is_symlink() {
            continue;
        }
        let name = entry.file_name().to_string_lossy().into_owned();
        let child = format!("{relative}/{name}");
        if metadata.is_dir() {
            directories.push(child.clone());
            walk_directory(&path, &child, directories, files)?;
        } else if metadata.is_file() {
            files.push(SendFile {
                source: path,
                relative: child,
                size: metadata.len(),
            });
        }
    }
    Ok(())
}

fn unique_logical_name(name: &str, used: &mut HashSet<String>) -> String {
    if used.insert(name.to_string()) {
        return name.to_string();
    }
    let (stem, ext) = split_name(name);
    for i in 1u64.. {
        let candidate = format!("{stem} ({i}){ext}");
        if used.insert(candidate.clone()) {
            return candidate;
        }
    }
    unreachable!()
}

struct ReceiveFile {
    file: Option<std::fs::File>,
    batch_id: String,
    expected: u64,
    written: u64,
    next_seq: u32,
    hasher: Sha256,
    temp_path: PathBuf,
    final_path: PathBuf,
}

struct ReceiveBatch {
    title: String,
    roots: HashMap<String, PathBuf>,
    root_paths: Vec<PathBuf>,
    expected_files: u32,
    completed_files: u32,
    total_bytes: u64,
    received_bytes: u64,
    failed: Option<String>,
    last_emit: Instant,
    last_activity: Instant,
    drop_at_cursor: bool,
    drag_id: Option<String>,
    drag_stage_dir: Option<PathBuf>,
}

#[derive(Default)]
struct ReceiverState {
    files: HashMap<String, ReceiveFile>,
    batches: HashMap<String, ReceiveBatch>,
}

pub struct FileReceiver {
    state: Mutex<ReceiverState>,
    download_dir: Mutex<PathBuf>,
    app: AppHandle,
}

impl FileReceiver {
    pub fn new(app: AppHandle, configured_dir: &str) -> Self {
        let download_dir = resolve_download_dir(configured_dir);
        if let Err(e) = std::fs::create_dir_all(&download_dir) {
            log::error!("[FILE] 无法创建接收目录 {}: {e}", download_dir.display());
        }
        log::info!("[FILE] 接收目录: {}", download_dir.display());
        Self {
            state: Mutex::new(ReceiverState::default()),
            download_dir: Mutex::new(download_dir),
            app,
        }
    }

    pub fn set_download_dir(&self, configured_dir: &str) -> Result<PathBuf, String> {
        let dir = resolve_download_dir(configured_dir);
        std::fs::create_dir_all(&dir)
            .map_err(|e| format!("无法创建接收目录 {}: {e}", dir.display()))?;
        *self.download_dir.lock().unwrap_or_else(|e| e.into_inner()) = dir.clone();
        Ok(dir)
    }

    pub fn download_dir(&self) -> PathBuf {
        self.download_dir
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }

    pub fn handle(&self, payload: &Payload) -> Vec<Payload> {
        match payload {
            Payload::FileBatchStart {
                id,
                roots,
                total_files,
                total_bytes,
                drop_at_cursor,
                drag_id,
            } => {
                let result = self.on_batch_start(
                    id,
                    roots,
                    *total_files,
                    *total_bytes,
                    *drop_at_cursor,
                    drag_id.clone(),
                );
                vec![Payload::FileBatchReady {
                    id: id.clone(),
                    ok: result.is_ok(),
                    error: result.err(),
                }]
            }
            Payload::FileDirectory { batch_id, path } => {
                self.on_directory(batch_id, path);
                Vec::new()
            }
            Payload::FileStart {
                id,
                batch_id,
                path,
                size,
            } => {
                if let Err(error) = self.on_start(id, batch_id, path, *size) {
                    vec![Payload::FileReady {
                        id: id.clone(),
                        ok: false,
                        error: Some(error),
                    }]
                } else {
                    vec![Payload::FileReady {
                        id: id.clone(),
                        ok: true,
                        error: None,
                    }]
                }
            }
            Payload::FileChunk { id, seq, data } => {
                if let Err(error) = self.on_chunk(id, *seq, data) {
                    self.fail_file(id, &error);
                    vec![Payload::FileResult {
                        id: id.clone(),
                        ok: false,
                        error: Some(error),
                    }]
                } else {
                    Vec::new()
                }
            }
            Payload::FileEnd { id, sha256 } => {
                let result = self.on_end(id, sha256);
                vec![Payload::FileResult {
                    id: id.clone(),
                    ok: result.is_ok(),
                    error: result.err(),
                }]
            }
            Payload::FileCancel { id } => {
                self.fail_file(id, "发送端已取消");
                Vec::new()
            }
            Payload::FileBatchEnd { id } => {
                let (result, drop_at_cursor) = self.on_batch_end(id);
                vec![Payload::FileBatchResult {
                    id: id.clone(),
                    ok: result.is_ok(),
                    error: result.err(),
                    drop_at_cursor,
                }]
            }
            Payload::FileBatchCancel { id } => {
                self.cancel_batch(id);
                vec![Payload::FileBatchResult {
                    id: id.clone(),
                    ok: false,
                    error: Some("发送端已取消".into()),
                    drop_at_cursor: false,
                }]
            }
            _ => Vec::new(),
        }
    }

    fn on_batch_start(
        &self,
        id: &str,
        roots: &[TransferRoot],
        files: u32,
        bytes: u64,
        drop_at_cursor: bool,
        drag_id: Option<String>,
    ) -> Result<(), String> {
        let drag_stage_dir = drag_id
            .as_ref()
            .map(|drag_id| std::env::temp_dir().join("ruiss-drag").join(drag_id));
        let base = if let Some(path) = &drag_stage_dir {
            if path.exists() {
                let _ = std::fs::remove_dir_all(path);
            }
            std::fs::create_dir_all(path)
                .map_err(|e| format!("unable to create drag staging directory: {e}"))?;
            path.clone()
        } else {
            self.download_dir()
        };
        let mut root_map = HashMap::new();
        let mut root_paths = Vec::new();
        let mut failure = None;
        for root in roots {
            let Some(name) = safe_single_name(&root.name) else {
                failure = Some(format!("对端发送了不安全的名称: {}", root.name));
                continue;
            };
            let target = unique_path(&base, &name);
            if root.is_dir {
                if let Err(e) = std::fs::create_dir_all(&target) {
                    failure = Some(format!("无法创建文件夹 {}: {e}", target.display()));
                    continue;
                }
            } else if let Err(e) = std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&target)
            {
                failure = Some(format!("无法预留文件 {}: {e}", target.display()));
                continue;
            }
            root_map.insert(root.name.clone(), target.clone());
            root_paths.push(target);
        }
        let title = roots
            .iter()
            .map(|r| r.name.as_str())
            .collect::<Vec<_>>()
            .join("、");
        self.state
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .batches
            .insert(
                id.to_string(),
                ReceiveBatch {
                    title: title.clone(),
                    roots: root_map,
                    root_paths,
                    expected_files: files,
                    completed_files: 0,
                    total_bytes: bytes,
                    received_bytes: 0,
                    failed: failure,
                    last_emit: Instant::now() - Duration::from_secs(1),
                    last_activity: Instant::now(),
                    drop_at_cursor,
                    drag_id,
                    drag_stage_dir,
                },
            );
        emit_update(
            &self.app,
            id,
            "receive",
            &title,
            "transferring",
            0,
            bytes,
            0,
            files,
            None,
            None,
        );
        if let Some(error) = self
            .state
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .batches
            .get(id)
            .and_then(|batch| batch.failed.clone())
        {
            let batch = self
                .state
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .batches
                .remove(id);
            if let Some(batch) = batch {
                cleanup_paths(&batch.root_paths);
                if let Some(stage) = &batch.drag_stage_dir {
                    let _ = std::fs::remove_dir_all(stage);
                }
            }
            Err(error)
        } else {
            Ok(())
        }
    }

    fn on_directory(&self, batch_id: &str, relative: &str) {
        let result = self.resolve_target(batch_id, relative).and_then(|path| {
            std::fs::create_dir_all(&path)
                .map_err(|e| format!("无法创建文件夹 {}: {e}", path.display()))?;
            Ok(())
        });
        if let Err(error) = result {
            self.mark_batch_failed(batch_id, error);
        } else {
            self.touch_batch(batch_id);
        }
    }

    fn on_start(&self, id: &str, batch_id: &str, relative: &str, size: u64) -> Result<(), String> {
        let final_path = self.resolve_target(batch_id, relative)?;
        if let Some(parent) = final_path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| format!("无法创建目录 {}: {e}", parent.display()))?;
        }
        let file_name = final_path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("file");
        let temp_path = final_path.with_file_name(format!(".{file_name}.ruiss-{id}.part"));
        let file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temp_path)
            .map_err(|e| format!("无法创建临时文件 {}: {e}", temp_path.display()))?;
        self.state
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .files
            .insert(
                id.to_string(),
                ReceiveFile {
                    file: Some(file),
                    batch_id: batch_id.to_string(),
                    expected: size,
                    written: 0,
                    next_seq: 0,
                    hasher: Sha256::new(),
                    temp_path,
                    final_path,
                },
            );
        self.touch_batch(batch_id);
        Ok(())
    }

    fn on_chunk(&self, id: &str, seq: u32, data: &[u8]) -> Result<(), String> {
        let (batch_id, written) = {
            let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
            let file = state
                .files
                .get_mut(id)
                .ok_or_else(|| "文件任务不存在或已失败".to_string())?;
            if seq != file.next_seq {
                return Err(format!(
                    "数据块序号错误，期望 {}，收到 {seq}",
                    file.next_seq
                ));
            }
            if file.written.saturating_add(data.len() as u64) > file.expected {
                return Err("收到的数据超过声明大小".into());
            }
            file.file
                .as_mut()
                .ok_or_else(|| "临时文件已关闭".to_string())?
                .write_all(&data)
                .map_err(|e| format!("写入磁盘失败: {e}"))?;
            file.hasher.update(&data);
            file.written += data.len() as u64;
            file.next_seq = file.next_seq.saturating_add(1);
            let batch_id = file.batch_id.clone();
            if let Some(batch) = state.batches.get_mut(&batch_id) {
                batch.received_bytes = batch.received_bytes.saturating_add(data.len() as u64);
                batch.last_activity = Instant::now();
            }
            (batch_id, data.len() as u64)
        };
        let _ = written;
        self.emit_receive_progress(&batch_id, false);
        Ok(())
    }

    fn on_end(&self, id: &str, expected_hash: &str) -> Result<(), String> {
        let mut file = self
            .state
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .files
            .remove(id)
            .ok_or_else(|| "文件任务不存在或已失败".to_string())?;
        let result = (|| {
            if file.written != file.expected {
                return Err(format!(
                    "文件大小不符：期望 {}，实际 {}",
                    file.expected, file.written
                ));
            }
            let actual_hash = format!("{:x}", file.hasher.finalize());
            if actual_hash != expected_hash {
                return Err("文件校验失败，内容不完整".into());
            }
            if let Some(mut output) = file.file.take() {
                output.flush().map_err(|e| format!("刷新文件失败: {e}"))?;
                output
                    .sync_all()
                    .map_err(|e| format!("文件落盘失败: {e}"))?;
            }
            if file.final_path.exists() {
                std::fs::remove_file(&file.final_path)
                    .map_err(|e| format!("无法替换预留文件: {e}"))?;
            }
            std::fs::rename(&file.temp_path, &file.final_path)
                .map_err(|e| format!("完成文件失败: {e}"))?;
            Ok(())
        })();
        if let Err(error) = &result {
            let _ = std::fs::remove_file(&file.temp_path);
            self.mark_batch_failed(&file.batch_id, error.clone());
        } else {
            let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
            if let Some(batch) = state.batches.get_mut(&file.batch_id) {
                batch.completed_files = batch.completed_files.saturating_add(1);
                batch.last_activity = Instant::now();
            }
        }
        self.emit_receive_progress(&file.batch_id, true);
        result
    }

    fn on_batch_end(&self, id: &str) -> (Result<(), String>, bool) {
        let batch = self
            .state
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .batches
            .remove(id)
            .ok_or_else(|| "接收任务不存在".to_string());
        let batch = match batch {
            Ok(batch) => batch,
            Err(error) => return (Err(error), false),
        };
        let drop_at_cursor = batch.drop_at_cursor;
        let result = if let Some(error) = batch.failed.clone() {
            Err(error)
        } else if batch.completed_files != batch.expected_files {
            Err(format!(
                "文件数量不符：期望 {}，完成 {}",
                batch.expected_files, batch.completed_files
            ))
        } else if batch.received_bytes != batch.total_bytes {
            Err(format!(
                "总大小不符：期望 {}，收到 {}",
                batch.total_bytes, batch.received_bytes
            ))
        } else {
            Ok(())
        };
        if let Err(error) = &result {
            emit_update(
                &self.app,
                id,
                "receive",
                &batch.title,
                "failed",
                batch.received_bytes,
                batch.total_bytes,
                batch.completed_files,
                batch.expected_files,
                Some(error.clone()),
                None,
            );
            cleanup_paths(&batch.root_paths);
            if let Some(stage) = &batch.drag_stage_dir {
                let _ = std::fs::remove_dir_all(stage);
            }
            return (result, drop_at_cursor);
        }
        let paths = batch
            .root_paths
            .iter()
            .map(|p| p.to_string_lossy().into_owned())
            .collect::<Vec<_>>();
        if let Some(drag_id) = &batch.drag_id {
            platform::complete_remote_file_drag(drag_id, &paths, None);
        } else {
            platform::clipboard_write_files(&paths);
        }
        let display_path = paths.first().cloned();
        emit_update(
            &self.app,
            id,
            "receive",
            &batch.title,
            "completed",
            batch.total_bytes,
            batch.total_bytes,
            batch.expected_files,
            batch.expected_files,
            None,
            display_path.clone(),
        );
        let _ = self.app.emit(
            "file-received",
            serde_json::json!({
                "name": batch.title,
                "path": display_path.unwrap_or_default(),
                "paths": paths,
            }),
        );
        if let Some(stage) = batch.drag_stage_dir {
            std::thread::spawn(move || {
                // The OS drop target may still be copying a very large staged file
                // into its final destination after the network batch completes.
                std::thread::sleep(Duration::from_secs(60 * 60));
                let _ = std::fs::remove_dir_all(stage);
            });
        }
        (Ok(()), drop_at_cursor)
    }

    fn resolve_target(&self, batch_id: &str, relative: &str) -> Result<PathBuf, String> {
        let components = safe_relative_components(relative)?;
        let state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        let batch = state
            .batches
            .get(batch_id)
            .ok_or_else(|| "接收批次不存在".to_string())?;
        let root = batch
            .roots
            .get(&components[0])
            .ok_or_else(|| "文件不属于声明的根目录".to_string())?;
        let mut result = root.clone();
        for component in components.iter().skip(1) {
            result.push(component);
        }
        Ok(result)
    }

    fn fail_file(&self, id: &str, error: &str) {
        let removed = self
            .state
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .files
            .remove(id);
        if let Some(file) = removed {
            drop(file.file);
            let _ = std::fs::remove_file(file.temp_path);
            self.mark_batch_failed(&file.batch_id, error.to_string());
        }
    }

    fn mark_batch_failed(&self, id: &str, error: String) {
        if let Some(batch) = self
            .state
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .batches
            .get_mut(id)
        {
            if batch.failed.is_none() {
                batch.failed = Some(error);
            }
        }
    }

    fn touch_batch(&self, id: &str) {
        if let Some(batch) = self
            .state
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .batches
            .get_mut(id)
        {
            batch.last_activity = Instant::now();
        }
    }

    pub fn cleanup_stale(&self, max_age: Duration) {
        let stale = {
            let state = self.state.lock().unwrap_or_else(|e| e.into_inner());
            state
                .batches
                .iter()
                .filter_map(|(id, batch)| {
                    (batch.last_activity.elapsed() >= max_age).then(|| id.clone())
                })
                .collect::<Vec<_>>()
        };
        for id in stale {
            log::warn!("[FILE] 接收任务长时间无数据，清理 batch={id}");
            self.cancel_batch_with_reason(&id, "传输超时，已清理临时文件");
        }
    }

    fn cancel_batch(&self, id: &str) {
        self.cancel_batch_with_reason(id, "发送端已取消");
    }

    fn cancel_batch_with_reason(&self, id: &str, reason: &str) {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        let file_ids = state
            .files
            .iter()
            .filter_map(|(file_id, file)| (file.batch_id == id).then(|| file_id.clone()))
            .collect::<Vec<_>>();
        for file_id in file_ids {
            if let Some(file) = state.files.remove(&file_id) {
                drop(file.file);
                let _ = std::fs::remove_file(file.temp_path);
            }
        }
        if let Some(batch) = state.batches.remove(id) {
            if let Some(drag_id) = &batch.drag_id {
                platform::complete_remote_file_drag(drag_id, &[], Some(reason.to_string()));
            }
            emit_update(
                &self.app,
                id,
                "receive",
                &batch.title,
                "cancelled",
                batch.received_bytes,
                batch.total_bytes,
                batch.completed_files,
                batch.expected_files,
                Some(reason.into()),
                None,
            );
            cleanup_paths(&batch.root_paths);
            if let Some(stage) = &batch.drag_stage_dir {
                let _ = std::fs::remove_dir_all(stage);
            }
        }
    }

    fn emit_receive_progress(&self, id: &str, force: bool) {
        let snapshot = {
            let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
            let Some(batch) = state.batches.get_mut(id) else {
                return;
            };
            if !force && batch.last_emit.elapsed() < Duration::from_millis(100) {
                return;
            }
            batch.last_emit = Instant::now();
            (
                batch.title.clone(),
                batch.received_bytes,
                batch.total_bytes,
                batch.completed_files,
                batch.expected_files,
            )
        };
        emit_update(
            &self.app,
            id,
            "receive",
            &snapshot.0,
            "transferring",
            snapshot.1,
            snapshot.2,
            snapshot.3,
            snapshot.4,
            None,
            None,
        );
    }
}

fn resolve_download_dir(configured: &str) -> PathBuf {
    if !configured.trim().is_empty() {
        return PathBuf::from(configured.trim());
    }
    dirs::download_dir()
        .or_else(dirs::home_dir)
        .unwrap_or_else(std::env::temp_dir)
}

fn safe_single_name(name: &str) -> Option<String> {
    let mut components = Path::new(name).components();
    match (components.next(), components.next()) {
        (Some(Component::Normal(value)), None) => {
            value.to_str().filter(|s| !s.is_empty()).map(str::to_string)
        }
        _ => None,
    }
}

fn safe_relative_components(path: &str) -> Result<Vec<String>, String> {
    let normalized = path.replace('\\', "/");
    let mut result = Vec::new();
    for component in Path::new(&normalized).components() {
        match component {
            Component::Normal(value) => {
                let value = value.to_str().ok_or_else(|| "文件名编码无效".to_string())?;
                if value.is_empty() {
                    return Err("文件路径包含空名称".into());
                }
                result.push(value.to_string());
            }
            _ => return Err("对端发送了不安全的文件路径".into()),
        }
    }
    if result.is_empty() {
        Err("文件路径为空".into())
    } else {
        Ok(result)
    }
}

fn unique_path(dir: &Path, name: &str) -> PathBuf {
    let candidate = dir.join(name);
    if !candidate.exists() {
        return candidate;
    }
    let (stem, ext) = split_name(name);
    for i in 1u64.. {
        let path = dir.join(format!("{stem} ({i}){ext}"));
        if !path.exists() {
            return path;
        }
    }
    unreachable!()
}

fn split_name(name: &str) -> (&str, &str) {
    match name.rfind('.') {
        Some(index) if index > 0 => (&name[..index], &name[index..]),
        _ => (name, ""),
    }
}

fn cleanup_paths(paths: &[PathBuf]) {
    for path in paths {
        if path.is_dir() {
            let _ = std::fs::remove_dir_all(path);
        } else if path.exists() {
            let _ = std::fs::remove_file(path);
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn emit_update(
    app: &AppHandle,
    id: &str,
    direction: &str,
    name: &str,
    status: &str,
    transferred: u64,
    total: u64,
    files_done: u32,
    files_total: u32,
    error: Option<String>,
    path: Option<String>,
) {
    let _ = app.emit(
        "file-transfer-update",
        serde_json::json!({
            "id": id,
            "direction": direction,
            "name": name,
            "status": status,
            "transferred": transferred,
            "total": total,
            "filesDone": files_done,
            "filesTotal": files_total,
            "error": error,
            "path": path,
        }),
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_unsafe_remote_paths() {
        assert!(safe_relative_components("folder/file.txt").is_ok());
        assert!(safe_relative_components("../secret").is_err());
        assert!(safe_relative_components("C:\\secret").is_err());
        assert!(safe_relative_components("/secret").is_err());
    }

    #[test]
    fn logical_names_do_not_collide() {
        let mut used = HashSet::new();
        assert_eq!(unique_logical_name("a.txt", &mut used), "a.txt");
        assert_eq!(unique_logical_name("a.txt", &mut used), "a (1).txt");
    }

    #[test]
    fn folder_plan_preserves_tree_and_empty_directories() {
        let root = std::env::temp_dir().join(format!("ruiss-plan-{}", uuid::Uuid::new_v4()));
        let empty = root.join("empty");
        let nested = root.join("nested");
        std::fs::create_dir_all(&empty).unwrap();
        std::fs::create_dir_all(&nested).unwrap();
        std::fs::write(nested.join("hello.txt"), b"hello").unwrap();

        let plan = build_plan(vec![root.clone()]).unwrap();
        let root_name = root.file_name().unwrap().to_string_lossy();
        assert_eq!(plan.roots.len(), 1);
        assert_eq!(plan.total_bytes, 5);
        assert!(plan.directories.contains(&format!("{root_name}/empty")));
        assert!(plan
            .files
            .iter()
            .any(|file| file.relative == format!("{root_name}/nested/hello.txt")));

        std::fs::remove_dir_all(root).unwrap();
    }
}
