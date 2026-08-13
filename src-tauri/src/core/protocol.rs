// 跨机事件消息协议。
// 序列化用 serde_json（骨架阶段可读性好；M4 优化为紧凑二进制编码）。

use serde::{Deserialize, Serialize};

use crate::core::keys::{Key, NativeShortcut};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TransferRoot {
    pub name: String,
    pub is_dir: bool,
}

/// 消息类型标记：区分"事件流"与"控制流"。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MsgKind {
    /// 按键 / 鼠标事件（对端注入）
    Event,
    /// 剪贴板内容同步
    Clipboard,
    /// 心跳 / 令牌仲裁（角色判定）
    Ctrl,
}

/// 一条跨机消息。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Message {
    pub kind: MsgKind,
    /// 发送方机器名（防回环判定用）
    pub from: String,
    pub payload: Payload,
}

/// 具体载荷。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Payload {
    /// 鼠标移动到源端逻辑坐标 (x, y)；src_w/src_h 为源端屏幕尺寸
    /// （接收端按自己的屏幕等比映射，支持两端分辨率不同）
    MouseMove {
        x: i32,
        y: i32,
        src_w: u32,
        src_h: u32,
    },
    /// 相对鼠标移动。Mac 转发 HID delta，Windows 转发 Raw Input delta；接收端
    /// 使用相对移动注入，避免虚拟绝对坐标、边缘钳制和逐帧 warp 破坏连续性。
    MouseMoveRelative {
        dx: i32,
        dy: i32,
    },
    /// UDP 线上的绝对移动帧。session 标识本次跨屏接管，seq 在该轮次内递增；
    /// 接收端据此拒绝旧轮次和乱序帧。
    PointerMove {
        session: u64,
        seq: u64,
        x: i32,
        y: i32,
        src_w: u32,
        src_h: u32,
    },
    /// UDP 线上的相对移动帧，同一轮次内可在发送队列中累加 delta。
    PointerMoveRelative {
        session: u64,
        seq: u64,
        dx: i32,
        dy: i32,
    },
    /// 鼠标按键：0 左键 / 1 右键 / 2 中键，down=true 按下，false 抬起
    MouseButton {
        button: u8,
        down: bool,
    },
    /// 滚轮：dy>0 向上滚，dx 为横向滚轮。单位统一为"格"：
    /// Windows 一格=120 delta（捕获 ÷120、注入 ×120），Mac 一格=1 行（原样）
    MouseWheel {
        dx: i32,
        dy: i32,
    },
    /// 键盘：key 为平台无关抽象键码（见 keys::Key，Win↔Mac 通用），
    /// down=true 按下，false 抬起。
    /// scan/extended 是 Windows 专用（精确回放同码键），Mac 端恒为 0/false 并忽略。
    Key {
        key: Key,
        scan: u16,
        extended: bool,
        down: bool,
    },
    /// 目标端原生快捷键。修饰键不做 Ctrl/Command 跨平台翻译，一次完整注入。
    Shortcut {
        shortcut: NativeShortcut,
    },
    /// 剪贴板文本
    ClipboardText {
        #[serde(default)]
        id: String,
        text: String,
    },
    /// 剪贴板图片（PNG 字节）
    ClipboardImage {
        #[serde(default)]
        id: String,
        png: Vec<u8>,
    },
    /// Source clipboard changed to content that is not yet available lazily on the peer.
    /// Invalidate the previous synchronized value so paste cannot reuse stale content.
    ClipboardClear,
    /// A file/folder copy started. Clear stale peer data and make this revision current
    /// until its verified FileBatch finishes.
    ClipboardFileOffer {
        id: String,
    },
    /// 剪贴板文件（路径列表）。接收端把这些路径写入本机剪贴板的文件类型
    /// （Win: CF_HDROP；Mac: NSFilenamesPboardType），用户可直接 Ctrl+V 粘贴文件。
    ClipboardFiles {
        #[serde(default)]
        id: String,
        paths: Vec<String>,
    },
    /// 心跳（保活 + 角色仲裁）
    Heartbeat {
        seq: u64,
    },
    /// 令牌：成为主控（对端进入 Sink），携带对端坐标系入口位置 (x, y)
    /// 及源端屏幕尺寸（对端注入入口时按比例映射）
    TakeControl {
        session: u64,
        x: i32,
        y: i32,
        src_w: u32,
        src_h: u32,
    },
    /// 接收端已经完成本轮接管。Source 收到前只缓存移动，不发 UDP，保证
    /// TakeControl 一定先于本轮所有移动生效。
    ControlReady {
        session: u64,
    },
    /// 释放令牌（本机鼠标回到出口边，对端恢复自主）
    ReleaseControl {
        session: u64,
    },

    // ===== M3：文件传输（分块流，支持任意大小文件）=====
    /// 一批文件/文件夹的清单与总大小；接收端预留安全且不重名的根路径后确认。
    FileBatchStart {
        id: String,
        roots: Vec<TransferRoot>,
        total_files: u32,
        total_bytes: u64,
        /// 来自原生跨屏拖拽；完成后接收端在当前光标应用自动粘贴。
        #[serde(default)]
        drop_at_cursor: bool,
        /// Native cross-screen drag session. When present, the receiver stages the
        /// payload for the OS drag provider instead of pasting it into the active app.
        #[serde(default)]
        drag_id: Option<String>,
        /// Clipboard copy revision. Only the newest revision may own the target clipboard.
        #[serde(default)]
        clipboard_id: Option<String>,
    },
    FileBatchReady {
        id: String,
        ok: bool,
        error: Option<String>,
    },
    FileDirectory {
        batch_id: String,
        path: String,
    },
    FileStart {
        id: String,
        batch_id: String,
        path: String,
        size: u64,
    },
    FileReady {
        id: String,
        ok: bool,
        error: Option<String>,
    },
    /// 文件数据块：id 对应 FileStart，seq 从 0 递增（接收端按序写盘，
    /// 丢块则整文件作废），data 为原始字节（建议 256KB/块）。
    /// Base64 数据避免 serde_json 把每个字节展开成十进制数组，显著降低网络帧体积。
    FileChunk {
        id: String,
        seq: u32,
        data: Vec<u8>,
    },
    /// 单文件结束；接收端同时校验大小与 SHA-256，成功后原子替换临时文件。
    FileEnd {
        id: String,
        sha256: String,
    },
    /// 取消文件传输（发送方主动取消或接收方拒绝）。
    FileCancel {
        id: String,
    },
    FileBatchEnd {
        id: String,
    },
    FileBatchCancel {
        id: String,
    },
    FileResult {
        id: String,
        ok: bool,
        error: Option<String>,
    },
    FileBatchResult {
        id: String,
        ok: bool,
        error: Option<String>,
        #[serde(default)]
        drop_at_cursor: bool,
    },

    // ===== M3：跨屏拖拽（拖动文件/图片跨屏到对端放下）=====
    /// 拖拽跨屏通告：Source 侧鼠标拖着东西滑到对端时随 TakeControl 一并发送，
    /// kinds 告知对端"拖拽里有什么"（text/image/files），对端收到后：
    /// 1) 等待后续 Clipboard*/File* 消息把内容补齐到本机剪贴板；
    /// 2) 在光标位置注入 Ctrl+V（Mac: Cmd+V）模拟"放下"。
    /// 若 drag=false 表示只是普通跨屏（不注入粘贴）。
    DragOffer {
        drag: bool,
        has_text: bool,
        has_image: bool,
        has_files: bool,
    },
    /// A file drag crossed the screen edge. This only starts the target OS drag
    /// session; file bytes are not sent until the target requests the data.
    DragStart {
        id: String,
        roots: Vec<TransferRoot>,
    },
    /// The target drop provider requested the promised files.
    DragCommit {
        id: String,
    },
    /// The native drag was cancelled or returned to the source screen.
    DragCancel {
        id: String,
    },
}

/// 便捷构造。
impl Message {
    pub fn event(from: &str, payload: Payload) -> Self {
        Self {
            kind: MsgKind::Event,
            from: from.into(),
            payload,
        }
    }
    pub fn clipboard(from: &str, payload: Payload) -> Self {
        Self {
            kind: MsgKind::Clipboard,
            from: from.into(),
            payload,
        }
    }
    pub fn ctrl(from: &str, payload: Payload) -> Self {
        Self {
            kind: MsgKind::Ctrl,
            from: from.into(),
            payload,
        }
    }
}

#[cfg(test)]
mod file_protocol_tests {
    use super::*;

    #[test]
    fn file_batch_messages_round_trip_json() {
        let messages = [
            Payload::ClipboardFileOffer {
                id: "clipboard".into(),
            },
            Payload::FileBatchStart {
                id: "batch".into(),
                roots: vec![TransferRoot {
                    name: "folder".into(),
                    is_dir: true,
                }],
                total_files: 1,
                total_bytes: 5,
                drop_at_cursor: false,
                drag_id: None,
                clipboard_id: Some("clipboard".into()),
            },
            Payload::FileStart {
                id: "file".into(),
                batch_id: "batch".into(),
                path: "folder/a.txt".into(),
                size: 5,
            },
            Payload::FileEnd {
                id: "file".into(),
                sha256: "hash".into(),
            },
            Payload::FileBatchResult {
                id: "batch".into(),
                ok: true,
                error: None,
                drop_at_cursor: false,
            },
        ];
        for payload in messages {
            let json = serde_json::to_string(&payload).unwrap();
            let decoded: Payload = serde_json::from_str(&json).unwrap();
            assert_eq!(decoded, payload);
        }
    }
}
