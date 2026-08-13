# 懒传剪贴板文件(粘贴时才传输)+ 精简

版本 bump 0.1.1 → 0.2.0(Cargo.toml + tauri.conf.json)。核心思路:复制只发"文件清单",对端剪贴板挂虚拟文件;用户真正粘贴那一刻才回发请求、开始传输。

## 1. 协议层 `src-tauri/src/core/protocol.rs`

- `ClipboardFiles` 增加 `#[serde(default)] names: Vec<String>`(文件名清单,旧版收到忽略未知字段,不会断连)
- 新增 `ClipboardFileRequest { id: String }`(接收端粘贴时发出;旧版收到会断连——接受,靠版本号手工对齐)
- 删除 `ClipboardFileOffer` legacy variant(0.1.1 已不发此消息)
- `Heartbeat` 删除 `protocol_version` 字段(只留 `app_version`)
- 更新单测(round-trip、兼容测试)

## 2. 发送端 `clipboard/mod.rs` + `file_transfer/mod.rs`

**修掉多触发根因**:`ClipboardSync` 回调加内容去重——记录上次 (内容键, 时间),1 秒内相同内容(文件路径列表/文本/图片字节)直接跳过。同时删除 win.rs 无效的 `LAST_HANDLED_CLIPBOARD_SEQUENCE`。

`FileSender` 改动:
- 删 `send_clipboard_paths` / `cancel_clipboard_transfer` / `active_clipboard`
- 新增 `offer_clipboard_paths(paths) -> id`:路径存入 `pending_offers: Mutex<HashMap<String, Vec<PathBuf>>>`(新复制覆盖旧 offer),不传输
- 新增 `request_clipboard_files(id)`:查 pending,命中 → `enqueue_paths(paths, clipboard_id=id)` 走现有管道传输;失效 → 返回错误
- 剪贴板变成文本/图片时清空 pending(回调里调 `clear_pending_offers`)

回调行为:复制文件 → 去重 → `offer_clipboard_paths` → 发 `ClipboardFiles { id, paths: [], names }`。**不再发 ClipboardClear、不再自动传输**。

## 3. 接收端协调 `file_transfer/mod.rs`

`FileReceiver` 加"懒粘贴等待者"(复用 win.rs `RemoteDragDataObject` 的 Condvar 模式):
- `attach_clipboard_waiter(id, ready)`:注册共享状态 `Arc<(Mutex<Option<Result<Vec<String>,String>>>, Condvar)>`
- `on_batch_end`:clipboard_id 存在时——**有 waiter → notify(路径)不写剪贴板**;无 waiter(旧版对端 eager 直传)→ 维持现有写剪贴板行为
- `on_batch_start` 拒绝(FileBatchReady=false)、失败、superseded、cleanup_stale 超时 → 都 notify 错误,别让 render 线程永久阻塞
- 删除全部 `notify_transfer` 系统通知调用及 `LARGE_TRANSFER_NOTICE_BYTES`

## 4. Windows 平台层 `platform/win.rs`(全新 delay-render)

- 新增 `set_clipboard_file_promise(id, names, callback)`:
  - `OpenClipboard(watcher窗口hwnd)` → `EmptyClipboard` → `SetClipboardData(CF_HDROP, NULL)`(延迟渲染)→ 再塞一个立即的 `PreferredDropEffect=COPY` → `remember_local_clipboard_sequence`(防回环)
  - 状态存 `PENDING_LAZY_HDROP: Mutex<Option<LazyOffer{id, callback}>>`
- `clip_wnd_proc` 加三个分支:
  - `WM_RENDERFORMAT`:发 `ClipboardPasteEvent::Requested(id, ready)` 回调 → **阻塞 Condvar 等传输完成**(带超时)→ 构建 HDROP HGLOBAL(复用现有 `RemoteDragDataObject::hdrop`)→ `SetClipboardData`;失败则放空 HDROP。渲染后剪贴板变真实路径,重复粘贴不再触发传输
  - `WM_RENDERALLFORMATS` / `WM_DESTROYCLIPBOARD`:清理占位状态

## 5. macOS 平台层 `platform/mac.rs`(复用现有 promise 机制)

- 新增 `set_clipboard_file_promise(id, names, callback)`:复用 `remote_promise_delegate_class`(dragId ivar 存 offer id、fileName 存文件名),创建 `NSFilePromiseProvider` → `pasteboard clearContents + writeObjects` → `remember_local_clipboard_change`
- 粘贴时系统回调现有 `promise_write`:发 Requested 事件 → 线程轮询结果(复用 REMOTE_DRAG_RESULTS 模式,新增 offerId → 本地路径缓存,重复粘贴直接 copy 不再请求)→ copy 到目标目录 → completionHandler

## 6. 桥接与路由 `lib.rs` / `net/mod.rs`

- 注册剪贴板 promise 回调:平台 `Requested(id, ready)` → ① `net.send_ctrl(ClipboardFileRequest{id})`(非阻塞,任意线程可发)② `file_receiver.attach_clipboard_waiter(id, ready)`
- `run_incoming_router` 新增 `ClipboardFileRequest` 分支(发送端收到)→ `file_sender.request_clipboard_files(id)`;删除 clipboard 分支里的 `cancel_clipboard_transfer` 调用
- 删除 `CLIPBOARD_PROTOCOL_VERSION`、`peer_protocol_version`、`versions_match` 里的协议比较;版本显示只留 `app_version`(对不上 → GUI 红色提示)
- 心跳只发 `app_version`
- 移除 `tauri-plugin-notification`(Cargo.toml 依赖 + plugin 注册)

## 7. GUI `gui/main.js`(小改)

版本状态行去掉"协议 x"字样,只显示 `本机 v0.2.0 / 对端 vX · 已同步/不一致`。

## 行为变化(预期效果)

| 场景 | 旧行为 | 新行为 |
|---|---|---|
| 复制图片文件 | 立即传输+偶发"已被更新的复制内容替代" | 只发文件名清单,0 流量 |
| 到对端 Ctrl+V | 文件已在,秒粘 | 粘贴瞬间开始传输(Explorer/访达等待下载完成) |
| 复制后一直不粘贴 | 白白传输 | 不传输 |
| 传输完成后再粘贴一次 | 正常 | Win:剪贴板已变真实路径;Mac:本地缓存直接 copy |
| 复制了新文件再粘贴 | 各种取消消息 | 旧 offer 被覆盖,request 失效报错,粘贴失败但不误粘旧文件 |

## 风险与边界

- Windows 第一版用 delay-render **CF_HDROP**:粘贴大文件夹时目标应用(资源管理器)会卡在下载期间(同步渲染),这是与 CFSTR_FILECONTENTS+IStream 流式方案(工作量大 2 倍)的取舍,先做可用版
- 旧版对端收到 `ClipboardFileRequest` 会断开重连——所以版本号必须两端对齐(GUI 已显示)
- 剪贴板在 offer 后、粘贴前被第三方应用覆盖 → 粘贴失败(正常现象,不误粘)

## 验证

- `cargo check` + `cargo test`(更新/新增协议与 file_transfer 单测);两端手工测试:复制图片→粘贴、复制文件夹→粘贴、复制后不粘贴、连续复制两次
