// net/tcp.rs：TCP 通道 —— 按键/点击/滚轮/控制消息（可靠、有序）。
//
// 帧格式：4 字节大端长度前缀 + serde_json（M2 先求通，M4 换紧凑二进制）。
// 直连方式（对称直连）：双方都监听又都尝试连接会建两条连接；
// 约定 (本机IP, 端口) < (对端IP, 端口) 的一侧当 Server（只听），另一侧当 Client（只连）。

use std::net::IpAddr;

use anyhow::Result;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

use crate::core::protocol::{Message, MsgKind, Payload};

/// 单帧最大长度（16MB：剪贴板图片 PNG 可能几 MB；大文件走 FileChunk 分块，
/// 每块 256KB 不会超限。再大就拒绝，防异常帧撑爆内存）。
const MAX_FRAME: usize = 16 * 1024 * 1024;

/// 写一帧：4 字节大端长度 + JSON。
pub async fn write_frame<W: AsyncWrite + Unpin>(stream: &mut W, msg: &Message) -> Result<()> {
    let body = encode_body(msg)?;
    stream.write_all(&(body.len() as u32).to_be_bytes()).await?;
    stream.write_all(&body).await?;
    Ok(())
}

/// 读一帧。对端关闭 / 帧损坏 / 超长帧 → Err。
pub async fn read_frame<R: AsyncRead + Unpin>(stream: &mut R) -> Result<Message> {
    let mut len = [0u8; 4];
    stream.read_exact(&mut len).await?;
    let n = u32::from_be_bytes(len) as usize;
    if n == 0 || n > MAX_FRAME {
        return Err(anyhow::anyhow!("非法帧长度 {n}"));
    }
    let mut buf = vec![0u8; n];
    stream.read_exact(&mut buf).await?;
    decode_body(&buf)
}

// File chunks dominate transfer CPU and bandwidth. A tagged binary frame avoids
// Base64's 33% expansion and the extra encode/decode copies while keeping JSON for
// every control/clipboard message (and therefore keeping diagnostics readable).
const FRAME_JSON: u8 = 0;
const FRAME_FILE_CHUNK: u8 = 1;

fn encode_body(msg: &Message) -> Result<Vec<u8>> {
    if let Payload::FileChunk { id, seq, data } = &msg.payload {
        let from = msg.from.as_bytes();
        let id = id.as_bytes();
        if from.len() > u16::MAX as usize || id.len() > u16::MAX as usize {
            return Err(anyhow::anyhow!("file chunk identifier is too long"));
        }
        let mut body = Vec::with_capacity(9 + from.len() + id.len() + data.len());
        body.push(FRAME_FILE_CHUNK);
        body.extend_from_slice(&(from.len() as u16).to_be_bytes());
        body.extend_from_slice(&(id.len() as u16).to_be_bytes());
        body.extend_from_slice(&seq.to_be_bytes());
        body.extend_from_slice(from);
        body.extend_from_slice(id);
        body.extend_from_slice(data);
        Ok(body)
    } else {
        let json = serde_json::to_vec(msg)?;
        let mut body = Vec::with_capacity(json.len() + 1);
        body.push(FRAME_JSON);
        body.extend_from_slice(&json);
        Ok(body)
    }
}

fn decode_body(body: &[u8]) -> Result<Message> {
    let Some((&tag, rest)) = body.split_first() else {
        return Err(anyhow::anyhow!("empty frame"));
    };
    match tag {
        FRAME_JSON => Ok(serde_json::from_slice(rest)?),
        FRAME_FILE_CHUNK => {
            if rest.len() < 8 {
                return Err(anyhow::anyhow!("truncated file chunk header"));
            }
            let from_len = u16::from_be_bytes([rest[0], rest[1]]) as usize;
            let id_len = u16::from_be_bytes([rest[2], rest[3]]) as usize;
            let seq = u32::from_be_bytes([rest[4], rest[5], rest[6], rest[7]]);
            let names_end = 8usize
                .checked_add(from_len)
                .and_then(|n| n.checked_add(id_len))
                .ok_or_else(|| anyhow::anyhow!("file chunk header overflow"))?;
            if names_end > rest.len() {
                return Err(anyhow::anyhow!("truncated file chunk identifiers"));
            }
            let from = std::str::from_utf8(&rest[8..8 + from_len])?.to_owned();
            let id = std::str::from_utf8(&rest[8 + from_len..names_end])?.to_owned();
            Ok(Message {
                kind: MsgKind::Clipboard,
                from,
                payload: Payload::FileChunk {
                    id,
                    seq,
                    data: rest[names_end..].to_vec(),
                },
            })
        }
        _ => Err(anyhow::anyhow!("unknown frame tag {tag}")),
    }
}

/// 判定本机角色：地址（IP, 端口）小者当 Server。
pub fn is_server(my_ip: IpAddr, my_port: u16, peer_ip: IpAddr, peer_port: u16) -> bool {
    (my_ip, my_port) < (peer_ip, peer_port)
}

/// 建立 TCP 连接（按角色监听或连接）。
pub async fn establish(
    my_ip: IpAddr,
    my_port: u16,
    peer_ip: &str,
    peer_port: u16,
) -> Result<TcpStream> {
    let peer_ip: IpAddr = peer_ip.parse()?;
    if is_server(my_ip, my_port, peer_ip, peer_port) {
        let listener = TcpListener::bind(("0.0.0.0", my_port)).await?;
        let (stream, _peer) = listener.accept().await?;
        Ok(stream)
    } else {
        let stream = TcpStream::connect((peer_ip, peer_port)).await?;
        Ok(stream)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::{IpAddr, Ipv4Addr};

    fn ip(octets: [u8; 4]) -> IpAddr {
        IpAddr::V4(Ipv4Addr::from(octets))
    }

    #[test]
    fn server_is_lower_address() {
        // 不同 IP：小 IP 当 Server
        assert!(is_server(
            ip([192, 168, 1, 5]),
            5200,
            ip([192, 168, 1, 9]),
            5200
        ));
        assert!(!is_server(
            ip([192, 168, 1, 9]),
            5200,
            ip([192, 168, 1, 5]),
            5200
        ));
        // 同 IP（本机回环测试）：端口小者当 Server
        assert!(is_server(
            ip([127, 0, 0, 1]),
            5200,
            ip([127, 0, 0, 1]),
            5201
        ));
        assert!(!is_server(
            ip([127, 0, 0, 1]),
            5201,
            ip([127, 0, 0, 1]),
            5200
        ));
    }

    #[test]
    fn binary_file_chunk_round_trip() {
        let original = Message::clipboard(
            "peer",
            Payload::FileChunk {
                id: "file-1".into(),
                seq: 42,
                data: vec![0, 1, 2, 254, 255],
            },
        );
        let body = encode_body(&original).unwrap();
        assert_eq!(body[0], FRAME_FILE_CHUNK);
        let decoded = decode_body(&body).unwrap();
        assert_eq!(decoded.from, original.from);
        assert_eq!(decoded.payload, original.payload);
    }

    #[test]
    fn json_control_frame_round_trip() {
        let original = Message::ctrl("peer", Payload::DragCancel { id: "drag-1".into() });
        let body = encode_body(&original).unwrap();
        assert_eq!(body[0], FRAME_JSON);
        let decoded = decode_body(&body).unwrap();
        assert_eq!(decoded.payload, original.payload);
    }
}
