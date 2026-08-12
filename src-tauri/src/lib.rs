// Ruiss 核心：Tauri 启动、托盘菜单、设置窗口、命令。
//
// M0：托盘 + 设置窗口。
// M2：双机打通 —— 常驻捕获 → 仲裁（边缘停留/令牌，core/arbiter）→
//     网络（TCP 按键点击控制 + UDP 鼠标移动，net/）→ 对端注入。
//
// 事件流：
//   本机捕获(platform) → Router 闭包 → Arbiter → 网络
//   网络 incoming → 路由任务 → Arbiter（令牌）→ 注入(platform)

mod clipboard;
mod core;
mod file_transfer;
mod net;
mod platform;

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use tauri::menu::{Menu, MenuItem};
use tauri::tray::TrayIconBuilder;
use tauri::{AppHandle, Manager, State};

use crate::core::arbiter::{Action, Arbiter, Layout, Mode};
use crate::core::keys::{
    translate_windows_shortcut_to_mac, Key, ModifierState, NativeShortcut,
};
use crate::core::protocol::{Message, Payload};

/// Tauri 入口。
#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    // 初始化日志（debug 输出到控制台）
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();

    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .setup(|app| {
            setup_tray(app)?;
            // 启动兜底：恢复上次可能残留的光标状态（Ruiss 被强杀后光标计数可能仍为负）
            platform::show_cursor();

            let injector = Arc::new(platform::InputInjector::new());
            let router = Arc::new(Mutex::new(RouterState::new(
                load_settings(app.handle()),
                injector.clone(),
            )));

            app.manage(router.clone());

            // 常驻捕获（自测/跨屏都要用；失败则 app 照常可用，仅无输入功能）
            match start_capturer(router.clone()) {
                Ok(c) => {
                    router.lock().unwrap().capture_ok = true;
                    app.manage(CapturerHandle(c));
                }
                Err(e) => {
                    log::error!("输入捕获启动失败（自测/跨屏不可用）: {e}");
                    // Mac 上常见原因：未授予辅助功能权限
                    log::error!("若在 Mac 上：请到 系统设置 → 隐私与安全性 → 辅助功能 授权后重启应用");
                }
            }

            // 空闲心跳：光标停在边缘不动也能触发跨屏判定
            spawn_tick_task(router.clone());

            // 文件接收器（监听对端 FileStart/Chunk/End，写下载目录 + 写剪贴板 + 通知前端）
            let file_receiver = Arc::new(file_transfer::FileReceiver::new(app.handle().clone()));
            {
                let mut r = router.lock().unwrap();
                r.file_receiver = Some(file_receiver);
            }

            // 若已配置对端 IP，启动网络
            if !router.lock().unwrap().settings.peer_ip.trim().is_empty() {
                let r = router.clone();
                let app_handle = app.handle().clone();
                tauri::async_runtime::spawn(async move {
                    if let Err(e) = apply_link_config(&r, &app_handle).await {
                        log::error!("网络启动失败: {e}");
                    }
                });
            }
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            get_settings,
            save_settings,
            get_net_status,
            send_file,
            pick_and_send_file,
        ])
        .run(tauri::generate_context!())
        .expect("error while running Ruiss");
    // 退出兜底：还原系统光标（SetSystemCursor 替换不会随进程退出自动还原，
    // 正常退出（托盘退出/关窗）必须走这里恢复）
    platform::show_cursor();
}

/// 托盘：图标 + 右键菜单（显示设置窗口 / 退出）。
fn setup_tray(app: &tauri::App) -> tauri::Result<()> {
    let show = MenuItem::with_id(app, "show", "显示设置窗口", true, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", "退出", true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&show, &quit])?;

    let tray = TrayIconBuilder::with_id("ruiss-tray")
        .icon(app.default_window_icon().unwrap().clone())
        .tooltip("Ruiss — 双机键鼠共享")
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| match event.id.as_ref() {
            "show" => {
                if let Some(win) = app.get_webview_window("settings") {
                    let _ = win.show();
                    let _ = win.set_focus();
                }
            }
            "quit" => app.exit(0),
            _ => {}
        })
        .build(app)?;

    // 托盘对象需保活：挂在 app state 上（Tauri 2 的 tray 由 runtime 持有，此处仅防优化）
    app.manage(TrayHandle(tray));
    Ok(())
}

/// 保活句柄。
struct TrayHandle(tauri::tray::TrayIcon);

/// 捕获器保活句柄。
struct CapturerHandle(platform::InputCapturer);

// ======================== 共享路由状态 ========================

/// 共享路由状态（一把锁，短临界区）。
struct RouterState {
    /// 最近一次保存的设置（name/layout/peerIp 驱动网络与仲裁）
    settings: Settings,
    /// 输入捕获是否可用（Mac 上未授权辅助功能时为 false）
    capture_ok: bool,
    /// 网络已配置时的仲裁器
    arbiter: Option<Arbiter>,
    /// 无锁网络发送句柄
    net: Option<net::NetHandle>,
    /// 网络引擎（持有任务，Drop 即停止）
    engine: Option<net::NetEngine>,
    injector: Arc<platform::InputInjector>,
    /// 剪贴板同步器（Drop 即停止监听）
    clipboard: Option<clipboard::ClipboardSync>,
    /// 文件发送器（无状态，可克隆）
    file_sender: Option<file_transfer::FileSender>,
    /// 文件接收器（处理对端 FileStart/Chunk/End）
    file_receiver: Option<Arc<file_transfer::FileReceiver>>,
    /// 本机作为 Source 时的跨屏轮次。收到同轮次 ControlReady 前只缓存移动。
    tx_pointer_session: u64,
    tx_pointer_ready: bool,
    tx_pointer_seq: u64,
    pending_pointer_move: Option<Payload>,
    /// 本机作为 Sink 时当前接受的跨屏轮次及最后 UDP 序号。
    rx_pointer_session: u64,
    rx_pointer_seq: u64,
}

impl RouterState {
    fn new(settings: Settings, injector: Arc<platform::InputInjector>) -> Self {
        Self {
            settings,
            capture_ok: false,
            arbiter: None,
            net: None,
            engine: None,
            injector,
            clipboard: None,
            file_sender: None,
            file_receiver: None,
            tx_pointer_session: 0,
            tx_pointer_ready: false,
            tx_pointer_seq: 0,
            pending_pointer_move: None,
            rx_pointer_session: 0,
            rx_pointer_seq: 0,
        }
    }
}

fn next_pointer_session() -> u64 {
    static SEED: OnceLock<u64> = OnceLock::new();
    static COUNTER: AtomicU64 = AtomicU64::new(1);
    let seed = *SEED.get_or_init(|| {
        let time = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos() as u64;
        time ^ ((std::process::id() as u64) << 32)
    });
    let session = seed.wrapping_add(COUNTER.fetch_add(1, Ordering::Relaxed));
    if session == 0 { u64::MAX } else { session }
}

fn merge_pending_pointer(slot: &mut Option<Payload>, next: Payload) {
    if let (
        Some(Payload::MouseMoveRelative { dx, dy }),
        Payload::MouseMoveRelative {
            dx: next_dx,
            dy: next_dy,
        },
    ) = (slot.as_mut(), &next)
    {
        *dx = dx.saturating_add(*next_dx);
        *dy = dy.saturating_add(*next_dy);
        return;
    }
    *slot = Some(next);
}

fn accept_pointer_sequence(
    active_session: u64,
    last_seq: &mut u64,
    session: u64,
    seq: u64,
) -> bool {
    if active_session == 0 || session != active_session || seq <= *last_seq {
        return false;
    }
    *last_seq = seq;
    true
}

fn make_pointer_frame(session: u64, seq: u64, payload: Payload) -> Option<Payload> {
    match payload {
        Payload::MouseMove {
            x,
            y,
            src_w,
            src_h,
        } => Some(Payload::PointerMove {
            session,
            seq,
            x,
            y,
            src_w,
            src_h,
        }),
        Payload::MouseMoveRelative { dx, dy } => Some(Payload::PointerMoveRelative {
            session,
            seq,
            dx,
            dy,
        }),
        _ => None,
    }
}

#[cfg(test)]
mod pointer_session_tests {
    use super::*;

    #[test]
    fn pending_relative_moves_accumulate_before_ready() {
        let mut pending = Some(Payload::MouseMoveRelative { dx: 4, dy: -2 });
        merge_pending_pointer(&mut pending, Payload::MouseMoveRelative { dx: 3, dy: 5 });
        assert_eq!(pending, Some(Payload::MouseMoveRelative { dx: 7, dy: 3 }));
    }

    #[test]
    fn pending_absolute_move_keeps_latest_position() {
        let mut pending = Some(Payload::MouseMove {
            x: 10,
            y: 20,
            src_w: 100,
            src_h: 100,
        });
        merge_pending_pointer(
            &mut pending,
            Payload::MouseMove {
                x: 80,
                y: 90,
                src_w: 100,
                src_h: 100,
            },
        );
        assert_eq!(
            pending,
            Some(Payload::MouseMove {
                x: 80,
                y: 90,
                src_w: 100,
                src_h: 100,
            })
        );
    }

    #[test]
    fn receiver_accepts_only_current_increasing_sequence() {
        let mut last = 0;
        assert!(accept_pointer_sequence(42, &mut last, 42, 1));
        assert!(!accept_pointer_sequence(42, &mut last, 42, 1));
        assert!(!accept_pointer_sequence(42, &mut last, 42, 0));
        assert!(!accept_pointer_sequence(42, &mut last, 41, 2));
        assert!(accept_pointer_sequence(42, &mut last, 42, 3));
        assert_eq!(last, 3);
    }
}

/// 只在对端确认 TakeControl 后发送 UDP；确认前保留最新绝对位置或累计相对 delta。
fn forward_pointer_move(router: &Mutex<RouterState>, move_payload: Payload) {
    let outgoing = {
        let mut r = match router.lock() {
            Ok(g) => g,
            Err(e) => e.into_inner(),
        };
        if r.tx_pointer_session == 0 {
            return;
        }
        if !r.tx_pointer_ready {
            merge_pending_pointer(&mut r.pending_pointer_move, move_payload);
            return;
        }
        r.tx_pointer_seq = r.tx_pointer_seq.saturating_add(1);
        let session = r.tx_pointer_session;
        let seq = r.tx_pointer_seq;
        let net = r.net.clone();
        let Some(wire) = make_pointer_frame(session, seq, move_payload) else {
            return;
        };
        (net, wire)
    };
    if let (Some(net), payload) = outgoing {
        net.send_pointer(payload);
    }
}

/// 常驻捕获：路由到仲裁/转发。
fn start_capturer(router: Arc<Mutex<RouterState>>) -> Result<platform::InputCapturer, String> {
    let router_cb = router.clone();
    platform::InputCapturer::start(move |payload| {
        let actions: Vec<Action> = {
            let mut r = match router_cb.lock() {
                Ok(g) => g,
                Err(e) => e.into_inner(),
            };
            let connected = r.net.as_ref().map(|n| n.connected()).unwrap_or(false);
            if !connected {
                return;
            }
            // Windows mouse bindings are target-native Mac shortcuts. Consume both
            // button transitions while linked; emit one atomic shortcut on press.
            if !platform::TARGET_IS_MAC {
                let linked_source = r
                    .arbiter
                    .as_ref()
                    .map(|a| a.mode == Mode::Source && a.linked)
                    .unwrap_or(false);
                if linked_source {
                    if let Some(actions) = mouse_binding_actions(&r.settings, &payload) {
                        actions
                    } else {
                        route_captured_payload(&mut r, payload)
                    }
                } else {
                    route_captured_payload(&mut r, payload)
                }
            } else {
                route_captured_payload(&mut r, payload)
            }
        };
        for action in actions {
            execute_action(&router_cb, action);
        }
    })
    .map_err(|e| e.to_string())
}

fn route_captured_payload(r: &mut RouterState, payload: Payload) -> Vec<Action> {
    match &payload {
        Payload::MouseMove { x, y, .. } => match r.arbiter.as_mut() {
            Some(arb) => {
                let (w, h) = platform::screen_size();
                arb.on_cursor(*x, *y, w, h, Instant::now())
            }
            None => Vec::new(),
        },
        _ => match r.arbiter.as_mut() {
            Some(arb) => match arb.on_input(payload) {
                Action::None => Vec::new(),
                action => vec![action],
            },
            None => Vec::new(),
        },
    }
}

/// Some(Vec) means the physical mouse button is handled (an empty Vec disables it).
/// Bindings are recorded with Windows keyboard semantics so clicking a mouse
/// button behaves exactly like pressing the same shortcut on the source keyboard.
fn mouse_binding_actions(settings: &Settings, payload: &Payload) -> Option<Vec<Action>> {
    let Payload::MouseButton { button, down } = payload else {
        return None;
    };
    let binding = match button {
        2 => settings.mouse_middle_shortcut,
        3 => settings.mouse_back_shortcut,
        4 => settings.mouse_forward_shortcut,
        _ => return None,
    };
    if !down {
        return Some(Vec::new());
    }
    Some(match binding {
        Some(source) => {
            let target = translate_windows_shortcut_to_mac(source.modifiers, source.key);
            vec![Action::Forward(Payload::Shortcut {
                shortcut: NativeShortcut {
                    key: target.key,
                    modifiers: target.modifiers,
                },
            })]
        }
        None => Vec::new(),
    })
}

/// 空闲心跳：每 25ms 推进仲裁器停留判定（光标停在边缘不动时也能触发）。
/// Source 侧：发起跨屏；Sink 侧：注入光标停在入口边 → 返回；断线时复位。
fn spawn_tick_task(router: Arc<Mutex<RouterState>>) {
    tauri::async_runtime::spawn(async move {
        loop {
            tokio::time::sleep(Duration::from_millis(25)).await;
            let actions = {
                let mut r = match router.lock() {
                    Ok(g) => g,
                    Err(e) => e.into_inner(),
                };
                let mut acts = Vec::new();
                let connected = r.net.as_ref().map(|n| n.connected()).unwrap_or(false);
                if let Some(a) = r.arbiter.as_mut() {
                    let (w, h) = platform::screen_size();
                    if !connected {
                        let was_active = a.linked || a.mode == Mode::Sink;
                        a.on_peer_release();
                        acts.clear();
                        if was_active {
                            log::warn!("网络断开，复位跨屏状态");
                            r.injector.reset_keyboard_state();
                            platform::show_cursor();
                            platform::set_local_input_blocked(false);
                            platform::set_sink_active(false);
                        }
                        r.tx_pointer_session = 0;
                        r.tx_pointer_ready = false;
                        r.tx_pointer_seq = 0;
                        r.pending_pointer_move = None;
                        r.rx_pointer_session = 0;
                        r.rx_pointer_seq = 0;
                        continue;
                    }
                    // 主动补藏：Source 跨屏期间每 25ms 确认一次光标隐藏。
                    // 对抗 tao ShowCursor(TRUE) / macOS 移动自动重显，
                    // 停手不动时也持续压制（不依赖移动事件）。
                    if a.linked {
                        platform::enforce_cursor_hidden();
                    }
                    if a.mode == Mode::Sink {
                        // Sink 侧：注入光标停在入口边（=自己的出口边）→ 返回
                        acts.extend(a.on_sink_tick(platform::last_injected_pos(), w, h, Instant::now()));
                    } else {
                        acts.extend(a.on_tick(Instant::now()));
                    }
                }
                acts
            };
            for action in actions {
                execute_action(&router, action);
            }
        }
    });
}

/// 执行仲裁器动作：发令牌 / 回绕光标 / 转发事件。
fn execute_action(router: &Mutex<RouterState>, action: Action) {
    let (name, net, injector) = {
        let r = match router.lock() {
            Ok(g) => g,
            Err(e) => e.into_inner(),
        };
        (r.settings.name.clone(), r.net.clone(), r.injector.clone())
    };
    let Some(net) = net else {
        return;
    };
    match action {
        Action::TakeControl { x, y, src_w, src_h } => {
            let session = next_pointer_session();
            let was_sink = {
                let mut r = match router.lock() {
                    Ok(g) => g,
                    Err(e) => e.into_inner(),
                };
                let was_sink = r.rx_pointer_session != 0;
                r.tx_pointer_session = session;
                r.tx_pointer_ready = false;
                r.tx_pointer_seq = 0;
                r.pending_pointer_move = None;
                // 从 Sink 反向夺回时，旧接收轮次立即作废。
                r.rx_pointer_session = 0;
                r.rx_pointer_seq = 0;
                was_sink
            };
            // Normal Source crossover has no local injected keyboard state. Only a
            // reverse takeover from Sink needs local target keys released here.
            if was_sink {
                injector.reset_keyboard_state();
            }
            log::info!("发起跨屏 session={session} → TakeControl({x}, {y})");
            // 本机光标"离开"（隐藏），避免双光标；键盘/点击/滚轮被拦截，只转发对端
            platform::hide_cursor();
            platform::set_local_input_blocked(true);
            // 若是从被控（Sink）状态夺回控制权：必须清除 Sink 标记——否则 blocked 与
            // sink 同时为 true，tap 里两个吞移动分支叠加、Source 虚拟位置不生效
            // （2026-08-11 日志实测出现 blocked=true sink=true 组合）。
            platform::set_sink_active(false);
            net.send_ctrl(Message::ctrl(
                &name,
                Payload::TakeControl {
                    session,
                    x,
                    y,
                    src_w,
                    src_h,
                },
            ));
            // 拖拽跨屏：左键按下时把本机剪贴板内容带过去 + 通知对端注入粘贴
            if platform::is_left_button_down() {
                let content = platform::clipboard_read();
                let (has_text, has_image, has_files) = match &content {
                    platform::ClipboardContent::Text(_) => (true, false, false),
                    platform::ClipboardContent::Image(_) => (false, true, false),
                    platform::ClipboardContent::Files(_) => (false, false, true),
                    platform::ClipboardContent::Empty => (false, false, false),
                };
                if has_text || has_image || has_files {
                    log::info!("拖拽跨屏：携带剪贴板内容 text={has_text} image={has_image} files={has_files}");
                    net.send(Message::ctrl(&name, Payload::DragOffer {
                        drag: true, has_text, has_image, has_files,
                    }));
                    match content {
                        platform::ClipboardContent::Text(t) => {
                            net.send(Message::clipboard(&name, Payload::ClipboardText { text: t }));
                        }
                        platform::ClipboardContent::Image(png) => {
                            net.send(Message::clipboard(&name, Payload::ClipboardImage { png }));
                        }
                        platform::ClipboardContent::Files(paths) => {
                            // 每个文件走分块传输
                            let sender = {
                                let r = match router.lock() {
                                    Ok(g) => g,
                                    Err(e) => e.into_inner(),
                                };
                                r.file_sender.clone()
                            };
                            if let Some(sender) = sender {
                                for p in paths {
                                    sender.send_file(std::path::PathBuf::from(p));
                                }
                            }
                        }
                        platform::ClipboardContent::Empty => {}
                    }
                }
            }
        }
        Action::ReleaseControl => {
            injector.reset_keyboard_state();
            let session = {
                let mut r = match router.lock() {
                    Ok(g) => g,
                    Err(e) => e.into_inner(),
                };
                let session = if r.tx_pointer_session != 0 {
                    r.tx_pointer_session
                } else {
                    r.rx_pointer_session
                };
                r.tx_pointer_session = 0;
                r.tx_pointer_ready = false;
                r.tx_pointer_seq = 0;
                r.pending_pointer_move = None;
                r.rx_pointer_session = 0;
                r.rx_pointer_seq = 0;
                session
            };
            log::info!("返回本机 session={session} → ReleaseControl");
            platform::show_cursor();
            platform::set_local_input_blocked(false);
            // 本机作为 Sink 主动返回后，仲裁器已切回 Source；平台输入所有权也必须
            // 同步清零，否则本机点击/滚轮仍会被 Sink 钩子吞掉，直到下一次跨屏。
            platform::set_sink_active(false);
            if session != 0 {
                net.send_ctrl(Message::ctrl(&name, Payload::ReleaseControl { session }));
            }
        }
        Action::Warp { x, y } => {
            log::debug!("光标回绕 → ({x}, {y})");
            // Mac 端回绕到避开 Dock/菜单栏的安全点；Windows 端为 no-op，
            // 隐藏光标留在出口边并改走 Raw Input 相对移动。
            platform::warp_cursor_cross(x, y);
        }
        Action::Forward(payload) => match payload {
            Payload::MouseMove { x, y, .. } => {
                let (w, h) = platform::screen_size();
                forward_pointer_move(
                    router,
                    Payload::MouseMove {
                        x,
                        y,
                        src_w: w as u32,
                        src_h: h as u32,
                    },
                );
            }
            Payload::MouseMoveRelative { dx, dy } => {
                forward_pointer_move(router, Payload::MouseMoveRelative { dx, dy });
            }
            // Key/button up must never be silently dropped, otherwise the target can be
            // left with a stuck modifier or mouse button. Use the reliable control lane.
            event @ (Payload::Key { .. }
            | Payload::MouseButton { .. }
            | Payload::Shortcut { .. }) => {
                net.send_ctrl(Message::event(&name, event));
            }
            other => net.send(Message::event(&name, other)),
        },
        Action::None => {}
    }
}

/// 网络路由任务：对端消息 → 令牌处理 / 注入。
async fn run_incoming_router(
    mut incoming: tokio::sync::mpsc::Receiver<Message>,
    router: Arc<Mutex<RouterState>>,
    injector: Arc<platform::InputInjector>,
) {
    while let Some(msg) = incoming.recv().await {
        match &msg.payload {
            Payload::Heartbeat { .. } => {} // 保活，无需处理
            Payload::TakeControl {
                session,
                x,
                y,
                src_w,
                src_h,
            } => {
                injector.reset_keyboard_state();
                // 被接管：本机系统光标保持可见——对端注入的事件移动的就是
                // 这个系统光标，隐藏它会导致"被控但找不到鼠标"；本机输入恢复原样
                platform::show_cursor();
                platform::set_local_input_blocked(false);
                // 标记本机为被控端（Sink）：吞掉本机 MouseMove，光标只跟对端注入走（防双鼠标）
                platform::set_sink_active(true);
                let (entry, ack_name, ack_net) = {
                    let mut r = match router.lock() {
                        Ok(g) => g,
                        Err(e) => e.into_inner(),
                    };
                    // 新轮次生效时，任何旧 Source/UDP 状态立即作废。
                    r.tx_pointer_session = 0;
                    r.tx_pointer_ready = false;
                    r.tx_pointer_seq = 0;
                    r.pending_pointer_move = None;
                    r.rx_pointer_session = *session;
                    r.rx_pointer_seq = 0;
                    let entry = r.arbiter.as_mut().and_then(|a| a.on_peer_take(*x, *y));
                    (entry, r.settings.name.clone(), r.net.clone())
                };
                if let Some((ix, iy)) = entry {
                    log::info!("对端接管 session={session} → 注入入口 ({ix}, {iy})");
                    let (tw, th) = platform::screen_size();
                    let (mx, my) = crate::core::geometry::map_coords(
                        ix, iy, *src_w, *src_h, tw as u32, th as u32,
                    );
                    injector.inject(Payload::MouseMove {
                        x: mx,
                        y: my,
                        src_w: tw as u32,
                        src_h: th as u32,
                    });
                    // 入口注入和 Sink 状态已经完成，之后才允许 Source 发本轮 UDP。
                    if let Some(net) = ack_net {
                        net.send_ctrl(Message::ctrl(
                            &ack_name,
                            Payload::ControlReady { session: *session },
                        ));
                    }
                }
            }
            Payload::ControlReady { session } => {
                {
                    let mut r = match router.lock() {
                        Ok(g) => g,
                        Err(e) => e.into_inner(),
                    };
                    if r.tx_pointer_session == *session {
                        log::info!("对端已准备好 session={session}，开始发送 UDP 指针帧");
                        // 必须先把确认前缓存的帧排进 UDP 队列，再开放新帧；否则
                        // 捕获线程可能抢先发送 seq=1，随后旧缓存以 seq=2 覆盖新位置。
                        if let Some(pending) = r.pending_pointer_move.take() {
                            r.tx_pointer_seq = r.tx_pointer_seq.saturating_add(1);
                            if let (Some(net), Some(frame)) = (
                                r.net.clone(),
                                make_pointer_frame(
                                    r.tx_pointer_session,
                                    r.tx_pointer_seq,
                                    pending,
                                ),
                            ) {
                                net.send_pointer(frame);
                            }
                        }
                        r.tx_pointer_ready = true;
                    }
                }
            }
            Payload::ReleaseControl { session } => {
                let accepted = {
                    let mut r = match router.lock() {
                        Ok(g) => g,
                        Err(e) => e.into_inner(),
                    };
                    let accepted = r.tx_pointer_session == *session
                        || r.rx_pointer_session == *session;
                    if accepted {
                        r.tx_pointer_session = 0;
                        r.tx_pointer_ready = false;
                        r.tx_pointer_seq = 0;
                        r.pending_pointer_move = None;
                        r.rx_pointer_session = 0;
                        r.rx_pointer_seq = 0;
                    }
                    accepted
                };
                if !accepted {
                    log::debug!("忽略旧轮次 ReleaseControl session={session}");
                    continue;
                }
                log::info!("对端归还控制 session={session}");
                injector.reset_keyboard_state();
                platform::show_cursor();
                platform::set_local_input_blocked(false);
                platform::set_sink_active(false);
                let mut r = match router.lock() {
                    Ok(g) => g,
                    Err(e) => e.into_inner(),
                };
                if let Some(a) = r.arbiter.as_mut() {
                    a.on_peer_release();
                    // 返回后光标回到出口边内侧（跨屏时被 warp 到了对侧，
                    // 不 warp 回来鼠标会从屏幕另一头出现）
                    let (w, h) = platform::screen_size();
                    if let Some((_, exit_y)) = a.exit_pos() {
                        let (wx, wy) = match a.exit_edge() {
                            crate::core::geometry::Edge::Right => (w - 2, exit_y),
                            crate::core::geometry::Edge::Left => (1, exit_y),
                            _ => (w / 2, h / 2),
                        };
                        platform::warp_cursor(wx, wy.clamp(0, h - 1));
                    }
                }
            }
            Payload::PointerMove {
                session,
                seq,
                x,
                y,
                src_w,
                src_h,
            } => {
                let accepted = {
                    let mut r = match router.lock() {
                        Ok(g) => g,
                        Err(e) => e.into_inner(),
                    };
                    let sink = r
                        .arbiter
                        .as_ref()
                        .map(|a| a.mode == Mode::Sink)
                        .unwrap_or(false);
                    let active_session = r.rx_pointer_session;
                    sink && accept_pointer_sequence(
                        active_session,
                        &mut r.rx_pointer_seq,
                        *session,
                        *seq,
                    )
                };
                if accepted {
                    let (tw, th) = platform::screen_size();
                    let (mx, my) = crate::core::geometry::map_coords(
                        *x,
                        *y,
                        *src_w,
                        *src_h,
                        tw as u32,
                        th as u32,
                    );
                    injector.inject(Payload::MouseMove {
                        x: mx,
                        y: my,
                        src_w: tw as u32,
                        src_h: th as u32,
                    });
                } else {
                    log::debug!(
                        "丢弃非当前/乱序绝对指针帧 session={session} seq={seq}"
                    );
                }
            }
            Payload::PointerMoveRelative {
                session,
                seq,
                dx,
                dy,
            } => {
                let accepted = {
                    let mut r = match router.lock() {
                        Ok(g) => g,
                        Err(e) => e.into_inner(),
                    };
                    let sink = r
                        .arbiter
                        .as_ref()
                        .map(|a| a.mode == Mode::Sink)
                        .unwrap_or(false);
                    let active_session = r.rx_pointer_session;
                    sink && accept_pointer_sequence(
                        active_session,
                        &mut r.rx_pointer_seq,
                        *session,
                        *seq,
                    )
                };
                if accepted {
                    injector.inject(Payload::MouseMoveRelative { dx: *dx, dy: *dy });
                } else {
                    log::debug!(
                        "丢弃非当前/乱序相对指针帧 session={session} seq={seq}"
                    );
                }
            }
            Payload::ClipboardText { .. } | Payload::ClipboardImage { .. } | Payload::ClipboardFiles { .. } => {
                clipboard::handle_remote(&msg.payload);
            }
            Payload::FileStart { .. } | Payload::FileChunk { .. } | Payload::FileEnd { .. } | Payload::FileCancel { .. } => {
                let fr = {
                    let r = match router.lock() {
                        Ok(g) => g,
                        Err(e) => e.into_inner(),
                    };
                    r.file_receiver.clone()
                };
                if let Some(fr) = fr {
                    fr.handle(&msg.payload);
                }
            }
            Payload::DragOffer { drag, has_text, has_image, has_files: _ } => {
                // 文字/图片拖拽：剪贴板内容随后到达写入，延迟注入粘贴
                // 文件拖拽：文件传完路径已入剪贴板，用户手动 Ctrl+V（传输耗时不确定）
                if *drag && (*has_text || *has_image) {
                    let inj = injector.clone();
                    tauri::async_runtime::spawn(async move {
                        tokio::time::sleep(Duration::from_millis(200)).await;
                        inject_paste(&inj);
                    });
                }
            }
            Payload::MouseMove { .. } | Payload::MouseMoveRelative { .. } => {
                // 网络移动必须带 session/seq；拒绝无轮次帧，避免旧版本或残留包
                // 绕过握手参与当前接管。
                log::debug!("丢弃无跨屏轮次的指针帧: {:?}", msg.payload);
            }
            _ => {
                let sink = {
                    let r = match router.lock() {
                        Ok(g) => g,
                        Err(e) => e.into_inner(),
                    };
                    r.arbiter.as_ref().map(|a| a.mode == Mode::Sink).unwrap_or(false)
                };
                if sink {
                    injector.inject(msg.payload.clone());
                } else {
                    log::debug!("忽略对端事件（本机 Source）: {:?}", msg.payload);
                }
            }
        }
    }
    log::info!("网络路由任务结束");
}

/// 注入"粘贴"组合键：本机是 Mac 用 Command+V，Windows 用 Ctrl+V。
/// 跨屏拖拽文字/图片到对端后，对端在光标处自动粘贴。
fn inject_paste(injector: &platform::InputInjector) {
    // InputInjector consumes the peer/source-side semantic key and performs the
    // Ctrl↔Command mapping. Feed the opposite key to obtain the native target modifier.
    let mod_key = if platform::TARGET_IS_MAC { Key::Ctrl } else { Key::Super };
    injector.inject(Payload::Key { key: mod_key, scan: 0, extended: false, down: true });
    injector.inject(Payload::Key { key: Key::V, scan: 0, extended: false, down: true });
    injector.inject(Payload::Key { key: Key::V, scan: 0, extended: false, down: false });
    injector.inject(Payload::Key { key: mod_key, scan: 0, extended: false, down: false });
}

/// 按当前设置（重）配置网络与仲裁器。
async fn apply_link_config(router: &Arc<Mutex<RouterState>>, _app: &AppHandle) -> Result<(), String> {
    let (peer_ip, layout, name, cross_enabled, clip_enabled) = {
        let r = match router.lock() {
            Ok(g) => g,
            Err(e) => e.into_inner(),
        };
        (
            r.settings.peer_ip.trim().to_string(),
            r.settings.layout.clone(),
            r.settings.name.trim().to_string(),
            r.settings.cross_screen_enabled,
            r.settings.clipboard_enabled,
        )
    };
    let injector = {
        let r = match router.lock() {
            Ok(g) => g,
            Err(e) => e.into_inner(),
        };
        r.injector.clone()
    };
    injector.reset_keyboard_state();
    platform::show_cursor();
    platform::set_local_input_blocked(false);
    platform::set_sink_active(false);
    // 停旧引擎
    {
        let mut r = match router.lock() {
            Ok(g) => g,
            Err(e) => e.into_inner(),
        };
        r.engine = None; // Drop → 任务中止
        r.net = None;
        r.arbiter = None;
        r.clipboard = None; // Drop → 停止剪贴板监听
        r.file_sender = None;
        r.tx_pointer_session = 0;
        r.tx_pointer_ready = false;
        r.tx_pointer_seq = 0;
        r.pending_pointer_move = None;
        r.rx_pointer_session = 0;
        r.rx_pointer_seq = 0;
    }
    if peer_ip.is_empty() {
        log::info!("未配置对端 IP，网络关闭");
        return Ok(());
    }

    let layout = if layout == "left" { Layout::PeerLeft } else { Layout::PeerRight };
    let name = if name.is_empty() { "ruiss".to_string() } else { name };
    let cfg = net::PeerConfig::new(peer_ip.clone());
    let start = net::NetEngine::start(name.clone(), cfg)
        .await
        .map_err(|e| format!("网络启动失败: {e}"))?;
    let handle = start.handle.clone();
    let injector = {
        let r = match router.lock() {
            Ok(g) => g,
            Err(e) => e.into_inner(),
        };
        r.injector.clone()
    };
    {
        let mut r = match router.lock() {
            Ok(g) => g,
            Err(e) => e.into_inner(),
        };
        r.engine = Some(start.engine);
        r.net = Some(handle.clone());
        // 跨屏开关关闭时不建仲裁器（事件不再触发跨屏），网络保持连接
        r.arbiter = if cross_enabled { Some(Arbiter::new(layout)) } else { None };
        // 文件发送器（剪贴板文件 + GUI 手动发送 + 拖拽，都用它）
        let file_sender = file_transfer::FileSender::new(handle.clone(), name.clone());
        r.file_sender = Some(file_sender.clone());
        // 剪贴板同步：监听本机剪贴板变化发对端，剪贴板开关关闭时不启动
        if clip_enabled {
            let clip = clipboard::ClipboardSync::start(
                name.clone(),
                handle.clone(),
                std::sync::Arc::new(move |paths: Vec<String>| {
                    // 本机剪贴板出现文件路径 → 逐个发送
                    for p in paths {
                        file_sender.send_file(std::path::PathBuf::from(p));
                    }
                }),
            );
            r.clipboard = Some(clip);
        }
    }
    tauri::async_runtime::spawn(run_incoming_router(start.incoming, router.clone(), injector));
    log::info!(
        "网络已启动：对端 {peer_ip}（跨屏{}，剪贴板{}）",
        if cross_enabled { "开启" } else { "已关闭" },
        if clip_enabled { "开启" } else { "已关闭" }
    );
    Ok(())
}

// ======================== 设置（持久化到 app_data_dir/settings.json）=======================

#[derive(serde::Serialize, serde::Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
struct Settings {
    /// 本机名字
    name: String,
    /// 对方 IP
    peer_ip: String,
    /// 屏幕布局：left / right（对方在我哪边）
    layout: String,
    /// 跨屏共享开关（默认开启；关闭后不再触发跨屏，网络保持连接）
    #[serde(default = "default_true")]
    cross_screen_enabled: bool,
    /// 剪贴板共享开关
    #[serde(default)]
    clipboard_enabled: bool,
    /// 开机自启
    autostart: bool,
    /// Windows 鼠标按键绑定，按 Windows 源键盘语义记录；发送前使用与普通键盘
    /// 相同的规则翻译到 Mac。None 表示禁用。
    #[serde(default = "default_mouse_back_shortcut")]
    mouse_back_shortcut: Option<NativeShortcut>,
    #[serde(default = "default_mouse_forward_shortcut")]
    mouse_forward_shortcut: Option<NativeShortcut>,
    #[serde(default = "default_mouse_middle_shortcut")]
    mouse_middle_shortcut: Option<NativeShortcut>,
}

fn default_true() -> bool {
    true
}

fn control_shortcut(key: Key) -> Option<NativeShortcut> {
    Some(NativeShortcut {
        key,
        modifiers: ModifierState {
            ctrl: true,
            ..Default::default()
        },
    })
}

fn default_mouse_back_shortcut() -> Option<NativeShortcut> {
    control_shortcut(Key::Digit1)
}

fn default_mouse_forward_shortcut() -> Option<NativeShortcut> {
    control_shortcut(Key::Digit2)
}

fn default_mouse_middle_shortcut() -> Option<NativeShortcut> {
    control_shortcut(Key::Digit3)
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            name: String::new(),
            peer_ip: String::new(),
            layout: "right".into(),
            cross_screen_enabled: true,
            clipboard_enabled: false,
            autostart: false,
            mouse_back_shortcut: default_mouse_back_shortcut(),
            mouse_forward_shortcut: default_mouse_forward_shortcut(),
            mouse_middle_shortcut: default_mouse_middle_shortcut(),
        }
    }
}

fn settings_path(app: &tauri::AppHandle) -> Result<std::path::PathBuf, String> {
    // 单机双实例自测：RUISS_CONFIG_FILE 指定独立配置文件，避免两实例互相覆盖设置
    if let Ok(p) = std::env::var("RUISS_CONFIG_FILE") {
        return Ok(std::path::PathBuf::from(p));
    }
    let dir = app.path().app_data_dir().map_err(|e| e.to_string())?;
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    Ok(dir.join("settings.json"))
}

fn load_settings(app: &tauri::AppHandle) -> Settings {
    let Ok(path) = settings_path(app) else {
        return Settings::default();
    };
    std::fs::read_to_string(path)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

#[tauri::command]
fn get_settings(app: tauri::AppHandle) -> Settings {
    load_settings(&app)
}

#[tauri::command]
async fn save_settings(
    app: tauri::AppHandle,
    state: State<'_, Arc<Mutex<RouterState>>>,
    settings: Settings,
) -> Result<(), String> {
    let path = settings_path(&app)?;
    let json = serde_json::to_string_pretty(&settings).map_err(|e| e.to_string())?;
    std::fs::write(&path, json).map_err(|e| format!("保存设置失败: {e}"))?;

    {
        let mut r = state.inner().lock().map_err(|e| e.to_string())?;
        r.settings = settings.clone();
    }
    apply_link_config(state.inner(), &app).await?;
    log::info!("设置已保存: {}", settings.peer_ip);
    Ok(())
}

// ======================== 网络状态 ========================

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct NetStatusView {
    /// 是否已配置对端 IP（网络层在跑）
    configured: bool,
    /// 跨屏共享是否开启（开关决定是否建仲裁器）
    cross_screen: bool,
    connected: bool,
    capture_ok: bool,
    mode: &'static str,
    linked: bool,
    sent: u64,
    received: u64,
}

#[tauri::command]
fn get_net_status(state: State<'_, Arc<Mutex<RouterState>>>) -> NetStatusView {
    let r = state.inner().lock().unwrap_or_else(|e| e.into_inner());
    let (connected, sent, received) = match &r.net {
        Some(n) => {
            let s = n.status();
            (s.connected, s.sent, s.received)
        }
        None => (false, 0, 0),
    };
    let (mode, linked) = match &r.arbiter {
        Some(a) => {
            let mode = if a.mode == Mode::Sink { "sink" } else { "source" };
            (mode, a.linked)
        }
        None => ("source", false),
    };
    NetStatusView {
        configured: r.net.is_some(),
        cross_screen: r.arbiter.is_some(),
        connected,
        capture_ok: r.capture_ok,
        mode,
        linked,
        sent,
        received,
    }
}

/// 发送指定路径的文件到对端（前端传完整路径）。
#[tauri::command]
fn send_file(
    state: State<'_, Arc<Mutex<RouterState>>>,
    path: String,
) -> Result<(), String> {
    let sender = {
        let r = state.inner().lock().map_err(|e| e.to_string())?;
        r.file_sender.clone()
    };
    let sender = sender.ok_or_else(|| "网络未连接，无法发送文件".to_string())?;
    sender.send_file(std::path::PathBuf::from(path));
    Ok(())
}

/// 弹出文件选择框，选完即发送（用 tauri-plugin-dialog）。
#[tauri::command]
async fn pick_and_send_file(
    app: AppHandle,
    state: State<'_, Arc<Mutex<RouterState>>>,
) -> Result<(), String> {
    use tauri_plugin_dialog::DialogExt;
    let picked = app
        .dialog()
        .file()
        .add_filter("所有文件", &["*"])
        .blocking_pick_file();
    let Some(picked) = picked else { return Ok(()); };
    let path = picked.into_path().map_err(|e| e.to_string())?;
    let sender = {
        let r = state.inner().lock().map_err(|e| e.to_string())?;
        r.file_sender.clone()
    };
    let sender = sender.ok_or_else(|| "网络未连接，无法发送文件".to_string())?;
    sender.send_file(path);
    Ok(())
}

#[cfg(test)]
mod mouse_binding_tests {
    use super::*;

    #[test]
    fn default_mouse_buttons_match_windows_ctrl_keyboard_translation() {
        let settings = Settings::default();
        for (button, key) in [(3, Key::Digit1), (4, Key::Digit2), (2, Key::Digit3)] {
            let actions = mouse_binding_actions(
                &settings,
                &Payload::MouseButton { button, down: true },
            )
            .expect("configured mouse button");
            assert_eq!(
                actions,
                vec![Action::Forward(Payload::Shortcut {
                    shortcut: NativeShortcut {
                        key,
                        modifiers: ModifierState {
                            super_key: true,
                            ..Default::default()
                        },
                    },
                })]
            );
        }
    }

    #[test]
    fn mouse_binding_release_is_consumed_without_second_shortcut() {
        let actions = mouse_binding_actions(
            &Settings::default(),
            &Payload::MouseButton {
                button: 3,
                down: false,
            },
        );
        assert_eq!(actions, Some(Vec::new()));
    }

    #[test]
    fn cleared_binding_consumes_button_without_action() {
        let mut settings = Settings::default();
        settings.mouse_back_shortcut = None;
        let actions = mouse_binding_actions(
            &settings,
            &Payload::MouseButton {
                button: 3,
                down: true,
            },
        );
        assert_eq!(actions, Some(Vec::new()));
    }
}
