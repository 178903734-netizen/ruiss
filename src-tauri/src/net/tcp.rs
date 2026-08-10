// net/tcp.rs：TCP 通道 —— 按键/点击/滚轮/控制消息（可靠、有序）。
//
// 帧格式：4 字节大端长度前缀 + serde_json（M2 先求通，M4 换紧凑二进制）。
// 直连方式（对称直连）：双方都监听又都尝试连接会建两条连接；
// 约定 (本机IP, 端口) < (对端IP, 端口) 的一侧当 Server（只听），另一侧当 Client（只连）。

use std::net::IpAddr;

use anyhow::Result;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

use crate::core::protocol::Message;

/// 单帧最大长度（16MB：剪贴板图片 PNG 可能几 MB；大文件走 FileChunk 分块，
/// 每块 256KB 不会超限。再大就拒绝，防异常帧撑爆内存）。
const MAX_FRAME: usize = 16 * 1024 * 1024;

/// 写一帧：4 字节大端长度 + JSON。
pub async fn write_frame<W: AsyncWrite + Unpin>(stream: &mut W, msg: &Message) -> Result<()> {
    let body = serde_json::to_vec(msg)?;
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
    let msg: Message = serde_json::from_slice(&buf)?;
    Ok(msg)
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
        assert!(is_server(ip([192, 168, 1, 5]), 5200, ip([192, 168, 1, 9]), 5200));
        assert!(!is_server(ip([192, 168, 1, 9]), 5200, ip([192, 168, 1, 5]), 5200));
        // 同 IP（本机回环测试）：端口小者当 Server
        assert!(is_server(ip([127, 0, 0, 1]), 5200, ip([127, 0, 0, 1]), 5201));
        assert!(!is_server(ip([127, 0, 0, 1]), 5201, ip([127, 0, 0, 1]), 5200));
    }
}
