// net：双通道网络引擎（M2）。
//
// TCP（按键/点击/滚轮/控制消息，可靠有序）+ UDP（鼠标移动，高频可丢）。
// 心跳 2s 保活，读超时 8s 判死，断线自动重连（2s 间隔）。
// 对称直连：地址 (IP, 端口) 小者当 Server（见 tcp::is_server）。
//
// 结构：
//   NetEngine::start() → NetStart { engine, handle, incoming }
//     - engine：生命周期管理（Drop 中止全部任务）
//     - handle：无锁发送句柄（消费者线程/路由用，可克隆）
//     - incoming：对端消息接收端（路由任务消费）

pub mod tcp;
pub mod udp;

use std::net::IpAddr;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use tokio::net::UdpSocket;
use tokio::sync::mpsc;

use crate::core::protocol::{Message, Payload};

/// 默认端口（M3 再开放设置项）。
pub const TCP_PORT: u16 = 5200;
pub const UDP_PORT: u16 = 5300;

const HEARTBEAT_INTERVAL: Duration = Duration::from_secs(2);
const READ_TIMEOUT: Duration = Duration::from_secs(8);
const RECONNECT_INTERVAL: Duration = Duration::from_secs(2);

/// 对端连接配置（由设置窗口下发）。
#[derive(Debug, Clone)]
pub struct PeerConfig {
    pub ip: String,
    /// 本机监听端口
    pub tcp_port: u16,
    pub udp_port: u16,
    /// 对端端口（双机场景 = 本机端口；单机双实例自测时可指到对方实例）
    pub peer_tcp_port: u16,
    pub peer_udp_port: u16,
}

impl PeerConfig {
    /// 默认端口；可用环境变量覆盖（单机双实例自测用）：
    ///   RUISS_TCP_PORT / RUISS_UDP_PORT      本机端口
    ///   RUISS_PEER_TCP_PORT / RUISS_PEER_UDP_PORT  对端端口（默认=本机端口）
    pub fn new(ip: String) -> Self {
        let env_or = |name: &str, default: u16| {
            std::env::var(name).ok().and_then(|s| s.parse().ok()).unwrap_or(default)
        };
        let tcp_port = env_or("RUISS_TCP_PORT", TCP_PORT);
        let udp_port = env_or("RUISS_UDP_PORT", UDP_PORT);
        let peer_tcp_port = env_or("RUISS_PEER_TCP_PORT", tcp_port);
        let peer_udp_port = env_or("RUISS_PEER_UDP_PORT", udp_port);
        Self { ip, tcp_port, udp_port, peer_tcp_port, peer_udp_port }
    }
}

/// 网络状态（GUI 轮询）。
#[derive(Debug, Clone, Copy, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NetStatus {
    pub connected: bool,
    pub sent: u64,
    pub received: u64,
}

#[derive(Default)]
struct NetStatusInner {
    connected: AtomicBool,
    sent: Arc<AtomicU64>,
    received: Arc<AtomicU64>,
}

/// 轻量发送句柄：无锁、可克隆。
#[derive(Clone)]
pub struct NetHandle {
    out: mpsc::Sender<Message>,
    moves: mpsc::Sender<(i32, i32, u32, u32)>,
    status: Arc<NetStatusInner>,
}

impl NetHandle {
    /// 发送可靠消息（TCP；断线时静默丢弃，重连后恢复）。
    pub fn send(&self, msg: Message) {
        let _ = self.out.try_send(msg);
    }

    /// 发送鼠标移动（UDP，fire-and-forget）。src_w/src_h 为发送端屏幕尺寸，
    /// 接收端据此做等比坐标映射。
    pub fn send_move(&self, x: i32, y: i32, src_w: u32, src_h: u32) {
        let _ = self.moves.try_send((x, y, src_w, src_h));
    }

    pub fn connected(&self) -> bool {
        self.status.connected.load(Ordering::Relaxed)
    }

    /// 收发统计 + 连接状态（GUI 轮询用）。
    pub fn status(&self) -> NetStatus {
        NetStatus {
            connected: self.status.connected.load(Ordering::Relaxed),
            sent: self.status.sent.load(Ordering::Relaxed),
            received: self.status.received.load(Ordering::Relaxed),
        }
    }
}

/// 网络引擎：任务挂在 tokio runtime 上，Drop 时中止全部任务。
pub struct NetEngine {
    handle: NetHandle,
    shutdown: Arc<AtomicBool>,
    tasks: Vec<tokio::task::JoinHandle<()>>,
}

/// start() 返回值。
pub struct NetStart {
    pub engine: NetEngine,
    pub handle: NetHandle,
    /// 对端消息接收端（路由任务消费）
    pub incoming: mpsc::Receiver<Message>,
}

impl NetEngine {
    /// 启动：绑 UDP（拿路由 IP）→ 起 TCP 重连循环 + UDP 收发循环。
    pub async fn start(name: String, cfg: PeerConfig) -> Result<NetStart> {
        let (out_tx, out_rx) = mpsc::channel::<Message>(256);
        let (in_tx, incoming) = mpsc::channel::<Message>(256);
        let (move_tx, move_rx) = mpsc::channel::<(i32, i32, u32, u32)>(256);
        let status = Arc::new(NetStatusInner {
            connected: AtomicBool::new(false),
            sent: Arc::new(AtomicU64::new(0)),
            received: Arc::new(AtomicU64::new(0)),
        });
        let shutdown = Arc::new(AtomicBool::new(false));

        let udp = UdpSocket::bind(("0.0.0.0", cfg.udp_port)).await?;
        // connect 到对端 UDP：只收对端包，并拿到去对端的本机路由 IP
        let _ = udp.connect((cfg.ip.as_str(), cfg.peer_udp_port)).await;
        let my_ip = match udp.local_addr().map(|a| a.ip()) {
            Ok(ip) if !ip.is_unspecified() => ip,
            _ => IpAddr::from([127, 0, 0, 1]),
        };

        // 读/写循环共用同一个 UDP socket（&self 并发安全）
        let udp = Arc::new(udp);
        let mut tasks = Vec::new();
        tasks.push(tokio::spawn(udp::read_loop(
            udp.clone(),
            in_tx.clone(),
            status.received.clone(),
        )));
        tasks.push(tokio::spawn(udp::write_loop(
            udp,
            move_rx,
            name.clone(),
            status.sent.clone(),
        )));
        tasks.push(tokio::spawn(
            Connector {
                name,
                cfg: cfg.clone(),
                my_ip,
                out_rx,
                incoming: in_tx,
                status: status.clone(),
                shutdown: shutdown.clone(),
            }
            .run(),
        ));

        let handle = NetHandle { out: out_tx, moves: move_tx, status: status.clone() };
        let engine = NetEngine { handle: handle.clone(), shutdown, tasks };
        Ok(NetStart { engine, handle, incoming })
    }

    pub fn handle(&self) -> NetHandle {
        self.handle.clone()
    }

    pub fn status(&self) -> NetStatus {
        NetStatus {
            connected: self.handle.connected(),
            sent: self.handle.status.sent.load(Ordering::Relaxed),
            received: self.handle.status.received.load(Ordering::Relaxed),
        }
    }
}

impl Drop for NetEngine {
    fn drop(&mut self) {
        self.shutdown.store(true, Ordering::Relaxed);
        for t in self.tasks.drain(..) {
            t.abort();
        }
    }
}

/// TCP 连接循环：重连 → 存活期（读任务 + 本任务负责写与心跳）→ 断开重来。
struct Connector {
    name: String,
    cfg: PeerConfig,
    my_ip: IpAddr,
    out_rx: mpsc::Receiver<Message>,
    incoming: mpsc::Sender<Message>,
    status: Arc<NetStatusInner>,
    shutdown: Arc<AtomicBool>,
}

impl Connector {
    async fn run(mut self) {
        loop {
            if self.shutdown.load(Ordering::Relaxed) {
                return;
            }
            match tcp::establish(
                self.my_ip,
                self.cfg.tcp_port,
                &self.cfg.ip,
                self.cfg.peer_tcp_port,
            )
            .await
            {
                Ok(stream) => {
                    log::info!("TCP 已连接 {}", self.cfg.ip);
                    self.status.connected.store(true, Ordering::Relaxed);
                    self.serve(stream).await;
                    self.status.connected.store(false, Ordering::Relaxed);
                    log::info!("TCP 断开，{}s 后重连", RECONNECT_INTERVAL.as_secs());
                }
                Err(e) => {
                    log::debug!("TCP 连接失败: {e}");
                }
            }
            if self.shutdown.load(Ordering::Relaxed) {
                return;
            }
            tokio::time::sleep(RECONNECT_INTERVAL).await;
        }
    }

    /// 连接存活期：读循环（独立任务）+ 写与心跳。
    async fn serve(&mut self, stream: tokio::net::TcpStream) {
        let (mut rd, mut wr) = stream.into_split();
        let (dead_tx, mut dead_rx) = tokio::sync::oneshot::channel();
        let incoming = self.incoming.clone();
        let status = self.status.clone();
        let reader = tokio::spawn(async move {
            loop {
                let res = tokio::time::timeout(READ_TIMEOUT, tcp::read_frame(&mut rd)).await;
                match res {
                    Ok(Ok(msg)) => {
                        status.received.fetch_add(1, Ordering::Relaxed);
                        if incoming.send(msg).await.is_err() {
                            break; // 上层已关闭
                        }
                    }
                    Ok(Err(e)) => {
                        log::debug!("TCP 读失败: {e}");
                        break;
                    }
                    Err(_) => {
                        log::debug!("TCP 读超时（心跳丢失，判定断线）");
                        break;
                    }
                }
            }
            let _ = dead_tx.send(());
        });

        let mut seq: u64 = 0;
        loop {
            tokio::select! {
                _ = &mut dead_rx => break, // 读端断了
                msg = self.out_rx.recv() => match msg {
                    Some(m) => {
                        if tcp::write_frame(&mut wr, &m).await.is_err() { break; }
                    }
                    None => break, // 上层已关闭
                },
                _ = tokio::time::sleep(HEARTBEAT_INTERVAL) => {
                    seq += 1;
                    let m = Message::ctrl(&self.name, Payload::Heartbeat { seq });
                    if tcp::write_frame(&mut wr, &m).await.is_err() { break; }
                }
            }
        }
        reader.abort();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::protocol::{Message, MsgKind, Payload};

    /// 从接收端取消息直到命中预期类型（心跳/其他消息会先到，需过滤）。
    async fn recv_until(
        rx: &mut mpsc::Receiver<Message>,
        pred: impl Fn(&Message) -> bool,
    ) -> Message {
        loop {
            let m = tokio::time::timeout(Duration::from_secs(5), rx.recv())
                .await
                .expect("等消息超时")
                .expect("通道关闭");
            if pred(&m) {
                return m;
            }
        }
    }

    #[tokio::test]
    async fn two_engines_exchange_messages() {
        // 同机双引擎：端口错开，对端端口互指（A 地址小 → A 当 Server）
        let cfg_a = PeerConfig {
            ip: "127.0.0.1".into(),
            tcp_port: 5200,
            udp_port: 5300,
            peer_tcp_port: 5201,
            peer_udp_port: 5301,
        };
        let cfg_b = PeerConfig {
            ip: "127.0.0.1".into(),
            tcp_port: 5201,
            udp_port: 5301,
            peer_tcp_port: 5200,
            peer_udp_port: 5300,
        };

        let sa = NetEngine::start("A".into(), cfg_a).await.expect("A 启动失败");
        let sb = NetEngine::start("B".into(), cfg_b).await.expect("B 启动失败");
        let (_engine_a, handle_a, mut in_a) = (sa.engine, sa.handle, sa.incoming);
        let (_engine_b, handle_b, mut in_b) = (sb.engine, sb.handle, sb.incoming);

        // 等连接建立
        let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
        while !handle_a.connected() || !handle_b.connected() {
            assert!(tokio::time::Instant::now() < deadline, "双引擎连接超时");
            tokio::time::sleep(Duration::from_millis(50)).await;
        }

        // A → B（TCP 事件）
        handle_a.send(Message::event(
            "A",
            Payload::Key { key: crate::core::keys::Key::A, scan: 0x1E, extended: false, down: true },
        ));
        let m = recv_until(&mut in_b, |m| m.from == "A" && m.kind == MsgKind::Event).await;
        assert!(matches!(m.payload, Payload::Key { key: crate::core::keys::Key::A, .. }));

        // B → A（TCP 事件）
        handle_b.send(Message::event("B", Payload::MouseButton { button: 0, down: true }));
        let m = recv_until(&mut in_a, |m| m.from == "B" && m.kind == MsgKind::Event).await;
        assert!(matches!(m.payload, Payload::MouseButton { button: 0, down: true }));

        // A → B（UDP 移动）
        handle_a.send_move(10, 20, 1280, 800);
        let m = recv_until(&mut in_b, |m| matches!(m.payload, Payload::MouseMove { .. })).await;
        assert!(matches!(m.payload, Payload::MouseMove { x: 10, y: 20, src_w: 1280, src_h: 800 }));

        // 控制消息（Ctrl）
        handle_a.send(Message::ctrl(
            "A",
            Payload::TakeControl { x: 0, y: 100, src_w: 1280, src_h: 800 },
        ));
        let m = recv_until(&mut in_b, |m| m.kind == MsgKind::Ctrl).await;
        assert!(matches!(m.payload, Payload::TakeControl { x: 0, y: 100, .. }));
    }
}
