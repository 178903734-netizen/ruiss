// net/udp.rs：UDP 通道 —— 鼠标移动（高频、丢帧无所谓，追求低延迟）。
//
// 只承载 MouseMove 载荷（Fire-and-forget）；发送前合并积压的移动，
// 一次只发最新位置（丢中间帧没问题）。对端插值平滑留到 M4。

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use tokio::net::UdpSocket;
use tokio::sync::mpsc;

use crate::core::protocol::{Message, Payload};

/// UDP 读循环：收到对端移动消息 → 转发给上层。错误记日志并继续。
pub async fn read_loop(udp: Arc<UdpSocket>, incoming: mpsc::Sender<Message>, received: Arc<AtomicU64>) {
    let mut buf = vec![0u8; 2048];
    loop {
        match udp.recv(&mut buf).await {
            Ok(n) => match serde_json::from_slice::<Message>(&buf[..n]) {
                Ok(msg) => {
                    received.fetch_add(1, Ordering::Relaxed);
                    if incoming.send(msg).await.is_err() {
                        break; // 上层已关闭
                    }
                }
                Err(_) => log::debug!("UDP 收到无法解析的包（{} 字节）", n),
            },
            Err(e) => {
                log::debug!("UDP 读错误: {e}，稍后重试");
                tokio::time::sleep(std::time::Duration::from_millis(100)).await;
            }
        }
    }
}

/// UDP 写循环：把本机鼠标移动发往对端（合并积压，只发最新）。
pub async fn write_loop(
    udp: Arc<UdpSocket>,
    mut moves: mpsc::Receiver<(i32, i32, u32, u32)>,
    name: String,
    sent: Arc<AtomicU64>,
) {
    while let Some((x, y, src_w, src_h)) = moves.recv().await {
        // 积压合并：发最新位置
        let mut last = (x, y, src_w, src_h);
        while let Ok(next) = moves.try_recv() {
            last = next;
        }
        let msg = Message::event(
            &name,
            Payload::MouseMove { x: last.0, y: last.1, src_w: last.2, src_h: last.3 },
        );
        match serde_json::to_vec(&msg) {
            Ok(bytes) => match udp.send(&bytes).await {
                Ok(n) if n > 0 => {
                    sent.fetch_add(1, Ordering::Relaxed);
                }
                Ok(_) => log::debug!("UDP send 返回 0"),
                Err(e) => log::debug!("UDP 发送失败: {e}"),
            },
            Err(e) => log::debug!("UDP 序列化失败: {e}"),
        }
    }
}
