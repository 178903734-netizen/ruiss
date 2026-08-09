// 跨机事件消息协议。
// 序列化用 serde_json（骨架阶段可读性好；M4 优化为紧凑二进制编码）。

use serde::{Deserialize, Serialize};

use crate::core::keys::Key;

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
    MouseMove { x: i32, y: i32, src_w: u32, src_h: u32 },
    /// 鼠标按键：0 左键 / 1 右键 / 2 中键 / 3 XButton1（后退侧键）/ 4 XButton2（前进侧键），
    /// down=true 按下，false 抬起
    MouseButton { button: u8, down: bool },
    /// 滚轮：dy>0 向上滚，dx 为横向滚轮。单位统一为"格"：
    /// Windows 一格=120 delta（捕获 ÷120、注入 ×120），Mac 一格=1 行（原样）
    MouseWheel { dx: i32, dy: i32 },
    /// 键盘：key 为平台无关抽象键码（见 keys::Key，Win↔Mac 通用），
    /// down=true 按下，false 抬起。
    /// scan/extended 是 Windows 专用（精确回放同码键），Mac 端恒为 0/false 并忽略。
    Key { key: Key, scan: u16, extended: bool, down: bool },
    /// 剪贴板文本
    ClipboardText { text: String },
    /// 剪贴板图片（PNG 字节）
    ClipboardImage { png: Vec<u8> },
    /// 心跳（保活 + 角色仲裁）
    Heartbeat { seq: u64 },
    /// 令牌：成为主控（对端进入 Sink），携带对端坐标系入口位置 (x, y)
    /// 及源端屏幕尺寸（对端注入入口时按比例映射）
    TakeControl { x: i32, y: i32, src_w: u32, src_h: u32 },
    /// 释放令牌（本机鼠标回到出口边，对端恢复自主）
    ReleaseControl,
}

/// 便捷构造。
impl Message {
    pub fn event(from: &str, payload: Payload) -> Self {
        Self { kind: MsgKind::Event, from: from.into(), payload }
    }
    pub fn clipboard(from: &str, payload: Payload) -> Self {
        Self { kind: MsgKind::Clipboard, from: from.into(), payload }
    }
    pub fn ctrl(from: &str, payload: Payload) -> Self {
        Self { kind: MsgKind::Ctrl, from: from.into(), payload }
    }
}
