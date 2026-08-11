// net/udp.rs：UDP 通道 —— 鼠标移动（高频、丢帧无所谓，追求低延迟）。
//
// 只承载鼠标移动载荷（Fire-and-forget）。绝对移动合并时只保留最新位置；
// 相对移动必须累加积压 delta，否则触控板快速移动会丢失路程。

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
    mut moves: mpsc::UnboundedReceiver<Payload>,
    name: String,
    sent: Arc<AtomicU64>,
) {
    while let Some(first) = moves.recv().await {
        let mut merged = first;
        while let Ok(next) = moves.try_recv() {
            merge_move(&mut merged, next);
        }
        let msg = Message::event(&name, merged);
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

fn merge_move(current: &mut Payload, next: Payload) {
    match (current, next) {
        (
            Payload::PointerMoveRelative { session, seq, dx, dy },
            Payload::PointerMoveRelative {
                session: next_session,
                seq: next_seq,
                dx: next_dx,
                dy: next_dy,
            },
        ) if *session == next_session => {
            *dx = dx.saturating_add(next_dx);
            *dy = dy.saturating_add(next_dy);
            *seq = next_seq;
        }
        (slot, next) => *slot = next,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn relative_moves_are_accumulated() {
        let mut current = Payload::PointerMoveRelative { session: 9, seq: 1, dx: 4, dy: -2 };
        merge_move(
            &mut current,
            Payload::PointerMoveRelative { session: 9, seq: 2, dx: 3, dy: 5 },
        );
        assert_eq!(
            current,
            Payload::PointerMoveRelative { session: 9, seq: 2, dx: 7, dy: 3 }
        );
    }

    #[test]
    fn relative_moves_from_different_sessions_are_not_merged() {
        let mut current = Payload::PointerMoveRelative {
            session: 9,
            seq: 4,
            dx: 20,
            dy: 10,
        };
        merge_move(
            &mut current,
            Payload::PointerMoveRelative {
                session: 10,
                seq: 1,
                dx: 2,
                dy: 3,
            },
        );
        assert_eq!(
            current,
            Payload::PointerMoveRelative {
                session: 10,
                seq: 1,
                dx: 2,
                dy: 3,
            }
        );
    }

    #[test]
    fn absolute_moves_keep_latest_position() {
        let mut current = Payload::PointerMove {
            session: 9,
            seq: 1,
            x: 10,
            y: 20,
            src_w: 100,
            src_h: 100,
        };
        merge_move(
            &mut current,
            Payload::PointerMove {
                session: 9,
                seq: 2,
                x: 80,
                y: 90,
                src_w: 100,
                src_h: 100,
            },
        );
        assert_eq!(
            current,
            Payload::PointerMove {
                session: 9,
                seq: 2,
                x: 80,
                y: 90,
                src_w: 100,
                src_h: 100,
            }
        );
    }
}
