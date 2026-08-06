// Windows 平台实现（M1）：低层钩子捕获 + SendInput 注入。
//
// 捕获：WH_MOUSE_LL / WH_KEYBOARD_LL 低层钩子，回调跑在专用钩子线程的消息循环里
//      （不注入 DLL，hMod 传 None）。
// 注入：SendInput，VK + 扫描码 + 扩展位原样回放，保证左右 Ctrl/方向键等"同码键"不走样。
// 防回环：钩子结构体自带 INJECTED 标记（LLMHF_INJECTED / LLKHF_INJECTED），
//       注入的事件直接丢弃 —— M1 本机回环靠它防死循环，M2 对端注入防再转发。
//
// 链路：钩子回调 → 无界通道 → 消费线程 → on_event 回调（M1 里即注入器）。
// TODO(M2)：多显示器坐标（虚拟屏幕归一化）、X 键鼠标按键、键位映射接入 keys.rs。

use std::cell::RefCell;
use std::mem::size_of;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Mutex, OnceLock};
use std::thread::{self, JoinHandle};

use anyhow::{anyhow, Result};
use windows::Win32::Foundation::{FALSE, LPARAM, LRESULT, TRUE, WPARAM};
use windows::Win32::System::Threading::GetCurrentThreadId;
use windows::Win32::UI::Input::KeyboardAndMouse::{
    INPUT, INPUT_0, INPUT_KEYBOARD, INPUT_MOUSE, KEYBDINPUT, KEYBD_EVENT_FLAGS,
    KEYEVENTF_EXTENDEDKEY, KEYEVENTF_KEYUP, MOUSEINPUT, MOUSEEVENTF_ABSOLUTE, MOUSEEVENTF_HWHEEL,
    MOUSEEVENTF_LEFTDOWN, MOUSEEVENTF_LEFTUP, MOUSEEVENTF_MIDDLEDOWN, MOUSEEVENTF_MIDDLEUP,
    MOUSEEVENTF_MOVE, MOUSEEVENTF_RIGHTDOWN, MOUSEEVENTF_RIGHTUP, MOUSEEVENTF_WHEEL, SendInput,
    VIRTUAL_KEY,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CallNextHookEx, DispatchMessageW, GetMessageW, GetSystemMetrics, KBDLLHOOKSTRUCT,
    LLKHF_EXTENDED, LLKHF_INJECTED, LLMHF_INJECTED, MSLLHOOKSTRUCT, PostThreadMessageW,
    SetCursorPos, SetWindowsHookExW, ShowCursor, TranslateMessage, UnhookWindowsHookEx,
    WH_KEYBOARD_LL, WH_MOUSE_LL, WM_KEYDOWN, WM_KEYUP, WM_LBUTTONDOWN, WM_LBUTTONUP,
    WM_MBUTTONDOWN, WM_MBUTTONUP, WM_MOUSEMOVE, WM_MOUSEWHEEL, WM_QUIT, WM_RBUTTONDOWN,
    WM_RBUTTONUP, WM_SYSKEYDOWN, WM_SYSKEYUP, MSG, SM_CXSCREEN, SM_CYSCREEN,
};

use crate::core::keys::Key;
use crate::core::protocol::Payload;

// 钩子回调往消费线程送事件的通道（线程局部：钩子线程装填）
thread_local! {
    static HOOK_SENDER: RefCell<Option<Sender<Payload>>> = const { RefCell::new(None) };
}

/// 是否过滤注入事件。RUISS_NO_SUPPRESS=1 时不过滤——仅供自动化验证捕获链路
/// （用 SendInput 造"伪真实"事件喂给钩子），正常使用永远过滤。
fn suppress_injected() -> bool {
    static V: OnceLock<bool> = OnceLock::new();
    *V.get_or_init(|| std::env::var("RUISS_NO_SUPPRESS").as_deref() != Ok("1"))
}

/// 跨屏期间是否拦截本机键盘/点击/滚轮（只转发对端，本机不生效；移动放行）。
static BLOCK_LOCAL_INPUT: AtomicBool = AtomicBool::new(false);

pub fn set_local_input_blocked(blocked: bool) {
    BLOCK_LOCAL_INPUT.store(blocked, Ordering::Relaxed);
}

/// 捕获器：两个低层钩子 + 钩子线程 + 消费线程。
pub struct InputCapturer {
    /// 钩子线程 ID（stop 时 PostThreadMessage 让它退出消息循环）
    thread_id: u32,
    hook_thread: JoinHandle<()>,
    consume_thread: JoinHandle<()>,
}

impl InputCapturer {
    /// 启动捕获。`on_event` 在独立消费线程里被调用，不阻塞钩子回调。
    pub fn start(on_event: impl Fn(Payload) + Send + 'static) -> Result<Self> {
        let (tx, rx): (Sender<Payload>, Receiver<Payload>) = mpsc::channel();
        // 钩子线程安装完成后回传线程 ID（成功）或错误（失败）
        let (ready_tx, ready_rx) = mpsc::channel::<Result<u32>>();

        let consume_thread = thread::Builder::new()
            .name("ruiss-input-consumer".into())
            .spawn(move || {
                while let Ok(p) = rx.recv() {
                    on_event(p);
                }
            })?;

        let hook_thread = thread::Builder::new()
            .name("ruiss-hook".into())
            .spawn(move || run_hook_loop(tx, ready_tx))?;

        match ready_rx.recv() {
            Ok(Ok(tid)) => Ok(Self { thread_id: tid, hook_thread, consume_thread }),
            Ok(Err(e)) => Err(e),
            Err(_) => Err(anyhow!("钩子线程异常退出")),
        }
    }

    /// 停止捕获：通知消息循环退出，钩子卸载，两个线程 join。
    pub fn stop(self) {
        unsafe {
            let _ = PostThreadMessageW(self.thread_id, WM_QUIT, WPARAM(0), LPARAM(0));
        }
        let _ = self.hook_thread.join();
        let _ = self.consume_thread.join();
    }
}

/// 钩子线程主体：装填通道 → 装钩子 → 泵消息循环。
/// 低层钩子回调必须由本线程的消息循环驱动。
fn run_hook_loop(tx: Sender<Payload>, ready: Sender<Result<u32>>) {
    let tid = unsafe { GetCurrentThreadId() };
    HOOK_SENDER.with(|s| *s.borrow_mut() = Some(tx.clone()));

    let mouse_hook = match unsafe { SetWindowsHookExW(WH_MOUSE_LL, Some(mouse_proc), None, 0) } {
        Ok(h) => h,
        Err(e) => return finish_hook_setup_failed(ready, format!("WH_MOUSE_LL 安装失败: {e}")),
    };
    let keyboard_hook =
        match unsafe { SetWindowsHookExW(WH_KEYBOARD_LL, Some(keyboard_proc), None, 0) } {
            Ok(h) => h,
            Err(e) => {
                unsafe { let _ = UnhookWindowsHookEx(mouse_hook); }
                return finish_hook_setup_failed(ready, format!("WH_KEYBOARD_LL 安装失败: {e}"));
            }
        };

    log::info!("低层钩子已安装（鼠标 + 键盘）");
    let _ = ready.send(Ok(tid));

    // 消息循环：GetMessageW 收到 WM_QUIT 返回 FALSE，循环结束
    let mut msg = MSG::default();
    unsafe {
        while GetMessageW(&mut msg, None, 0, 0).as_bool() {
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }

    // 清理：卸钩子 + 释放通道（Sender 全部 drop 后消费线程自然退出）
    unsafe {
        let _ = UnhookWindowsHookEx(keyboard_hook);
        let _ = UnhookWindowsHookEx(mouse_hook);
    }
    HOOK_SENDER.with(|s| *s.borrow_mut() = None);
    log::info!("钩子线程已退出");
}

/// 钩子安装失败：清通道引用、回传错误。
fn finish_hook_setup_failed(ready: Sender<Result<u32>>, msg: String) {
    log::error!("{msg}");
    HOOK_SENDER.with(|s| *s.borrow_mut() = None);
    let _ = ready.send(Err(anyhow!(msg)));
}

unsafe extern "system" fn mouse_proc(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if code >= 0 {
        let ms = &*(lparam.0 as *const MSLLHOOKSTRUCT);
        // 注入事件（含本机回环的回声）直接丢弃，防死循环
        if !suppress_injected() || ms.flags & LLMHF_INJECTED == 0 {
            if let Some(p) = mouse_to_payload(wparam.0 as u32, ms) {
                log::debug!("捕获: {p:?}");
                // 跨屏期间：点击/滚轮吞掉（只转发对端，本机不生效）；移动放行（本机光标还要动）
                let swallow = BLOCK_LOCAL_INPUT.load(Ordering::Relaxed)
                    && matches!(p, Payload::MouseButton { .. } | Payload::MouseWheel { .. });
                HOOK_SENDER.with(|s| {
                    if let Some(tx) = s.borrow().as_ref() {
                        let _ = tx.send(p);
                    }
                });
                if swallow {
                    return LRESULT(1);
                }
                // 移动补藏：跨屏期间每个真实鼠标移动事件后补一次隐藏
                // （对抗 tao 等外部 ShowCursor(TRUE) 把计数拉回 0）
                if wparam.0 as u32 == WM_MOUSEMOVE {
                    enforce_cursor_hidden_on_move();
                }
            }
        }
    }
    CallNextHookEx(None, code, wparam, lparam)
}

unsafe extern "system" fn keyboard_proc(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if code >= 0 {
        let ks = &*(lparam.0 as *const KBDLLHOOKSTRUCT);
        if !suppress_injected() || ks.flags.0 & LLKHF_INJECTED.0 == 0 {
            if let Some(p) = keyboard_to_payload(wparam.0 as u32, ks) {
                log::debug!("捕获: {p:?}");
                // 跨屏期间：键盘事件吞掉（只转发对端，本机不生效）
                let swallow = BLOCK_LOCAL_INPUT.load(Ordering::Relaxed);
                HOOK_SENDER.with(|s| {
                    if let Some(tx) = s.borrow().as_ref() {
                        let _ = tx.send(p);
                    }
                });
                if swallow {
                    return LRESULT(1);
                }
            }
        }
    }
    CallNextHookEx(None, code, wparam, lparam)
}

fn mouse_to_payload(wparam: u32, ms: &MSLLHOOKSTRUCT) -> Option<Payload> {
    match wparam {
        WM_MOUSEMOVE => {
            let (w, h) = screen_size();
            Some(Payload::MouseMove { x: ms.pt.x, y: ms.pt.y, src_w: w as u32, src_h: h as u32 })
        }
        WM_LBUTTONDOWN => Some(Payload::MouseButton { button: 0, down: true }),
        WM_LBUTTONUP => Some(Payload::MouseButton { button: 0, down: false }),
        WM_RBUTTONDOWN => Some(Payload::MouseButton { button: 1, down: true }),
        WM_RBUTTONUP => Some(Payload::MouseButton { button: 1, down: false }),
        WM_MBUTTONDOWN => Some(Payload::MouseButton { button: 2, down: true }),
        WM_MBUTTONUP => Some(Payload::MouseButton { button: 2, down: false }),
        WM_MOUSEWHEEL => Some(Payload::MouseWheel { dx: 0, dy: wheel_delta(ms) }),
        0x020E => Some(Payload::MouseWheel { dx: wheel_delta(ms), dy: 0 }), // WM_MOUSEHWHEEL（0.58 未导出该常量）
        _ => None, // WM_XBUTTON* 等 M2 再支持
    }
}

/// 注入侧最后已知光标位置（Sink 侧返回判定用）。
static LAST_INJECTED: Mutex<Option<(i32, i32)>> = Mutex::new(None);

/// 最近一次注入到本机的光标位置（None = 还没注入过移动）。
pub fn last_injected_pos() -> Option<(i32, i32)> {
    *LAST_INJECTED.lock().unwrap_or_else(|e| e.into_inner())
}

/// 跨屏期间是否抑制本机光标显示（Source linked 或 Sink 都为 true）。
/// 移动补藏：鼠标移动事件时检查，发现显示计数被外部（tao 等）拉回就再藏。
static CURSOR_SUPPRESS: AtomicBool = AtomicBool::new(false);

/// 隐藏本机光标（跨屏期间源端光标"离开"本机屏幕，避免双光标）。
/// ShowCursor 计数是全局的，tao/其他程序会频繁 ShowCursor(TRUE) 抵消，
/// 循环压到负计数 + 移动补藏（mouse_proc 里）双重保障。
pub fn hide_cursor() {
    CURSOR_SUPPRESS.store(true, Ordering::Relaxed);
    let mut n = unsafe { ShowCursor(FALSE) };
    while n >= 0 {
        n = unsafe { ShowCursor(FALSE) };
    }
    log::info!("隐藏光标（计数 {n}）");
}

/// 恢复显示本机光标（循环归正计数）。
pub fn show_cursor() {
    CURSOR_SUPPRESS.store(false, Ordering::Relaxed);
    let mut n = unsafe { ShowCursor(TRUE) };
    while n < 0 {
        n = unsafe { ShowCursor(TRUE) };
    }
    log::info!("恢复光标（计数 {n}）");
}

/// 移动补藏：跨屏期间每个真实鼠标移动事件后调用一次，
/// 对抗外部 ShowCursor(TRUE) 把计数拉回 0（光标重新显示）。
/// 移动事件频率（~125Hz）远高于外部 +1 频率（实测 ~2.6Hz），能完全压住。
fn enforce_cursor_hidden_on_move() {
    if CURSOR_SUPPRESS.load(Ordering::Relaxed) {
        let _ = unsafe { ShowCursor(FALSE) };
    }
}

/// 滚轮增量：高 16 位为有符号 delta，换算为"格"（Windows 一格 = 120）。
fn wheel_delta(ms: &MSLLHOOKSTRUCT) -> i32 {
    let raw = (ms.mouseData >> 16) as i16 as i32;
    raw / 120 // WHEEL_DELTA：协议统一按"格"传
}

fn keyboard_to_payload(wparam: u32, ks: &KBDLLHOOKSTRUCT) -> Option<Payload> {
    let down = match wparam {
        WM_KEYDOWN | WM_SYSKEYDOWN => true,
        WM_KEYUP | WM_SYSKEYUP => false,
        _ => return None,
    };
    Some(Payload::Key {
        key: vk_to_key(ks.vkCode),
        scan: ks.scanCode as u16,
        extended: ks.flags.0 & LLKHF_EXTENDED.0 != 0,
        down,
    })
}

// ---- Windows VK ↔ 抽象键码（keys::Key）映射 ----

const VK_MAP: &[(u32, Key)] = &[
    // 字母 A-Z（0x41..=0x5A）
    (0x41, Key::A), (0x42, Key::B), (0x43, Key::C), (0x44, Key::D), (0x45, Key::E),
    (0x46, Key::F), (0x47, Key::G), (0x48, Key::H), (0x49, Key::I), (0x4A, Key::J),
    (0x4B, Key::K), (0x4C, Key::L), (0x4D, Key::M), (0x4E, Key::N), (0x4F, Key::O),
    (0x50, Key::P), (0x51, Key::Q), (0x52, Key::R), (0x53, Key::S), (0x54, Key::T),
    (0x55, Key::U), (0x56, Key::V), (0x57, Key::W), (0x58, Key::X), (0x59, Key::Y),
    (0x5A, Key::Z),
    // 数字 0-9（0x30..=0x39）
    (0x30, Key::Digit0), (0x31, Key::Digit1), (0x32, Key::Digit2), (0x33, Key::Digit3),
    (0x34, Key::Digit4), (0x35, Key::Digit5), (0x36, Key::Digit6), (0x37, Key::Digit7),
    (0x38, Key::Digit8), (0x39, Key::Digit9),
    // 修饰键
    (0x10, Key::Shift), (0x11, Key::Ctrl), (0x12, Key::Alt),
    (0x5B, Key::Super), (0x5C, Key::Super), // 左/右 Win 键
    // 功能键
    (0x0D, Key::Enter), (0x20, Key::Space), (0x08, Key::Backspace),
    (0x09, Key::Tab), (0x1B, Key::Esc),
    (0x25, Key::ArrowLeft), (0x26, Key::ArrowUp), (0x27, Key::ArrowRight),
    (0x28, Key::ArrowDown),
    (0x70, Key::F1), (0x71, Key::F2), (0x72, Key::F3), (0x73, Key::F4),
    (0x74, Key::F5), (0x75, Key::F6), (0x76, Key::F7), (0x77, Key::F8),
    (0x78, Key::F9), (0x79, Key::F10), (0x7A, Key::F11), (0x7B, Key::F12),
];

/// Windows VK 虚拟键码 → 抽象键码（未覆盖的键透传为 Other）。
pub fn vk_to_key(vk: u32) -> Key {
    VK_MAP
        .iter()
        .find(|(v, _)| *v == vk)
        .map(|(_, k)| *k)
        .unwrap_or(Key::Other(vk))
}

/// 抽象键码 → Windows VK（未覆盖的 Other 透传原始码，其余返回 0 表示丢键）。
pub fn key_to_vk(key: Key) -> u32 {
    match key {
        Key::Other(n) => n,
        k => VK_MAP.iter().find(|(_, m)| *m == k).map(|(v, _)| *v).unwrap_or(0),
    }
}

/// 注入器：把协议事件用 SendInput 回放到本机。
pub struct InputInjector;

impl InputInjector {
    pub fn new() -> Self {
        Self
    }

    /// 注入一条事件，返回 SendInput 实际注入的条数（0 = 失败/跳过）。
    /// M1：鼠标坐标按主屏物理像素归一化（多显示器 M2/M4 处理）。
    pub fn inject(&self, event: Payload) -> u32 {
        // 记录最后注入位置（Sink 侧返回判定用；已是本机坐标系）
        if let Payload::MouseMove { x, y, .. } = &event {
            if let Ok(mut p) = LAST_INJECTED.lock() {
                *p = Some((*x, *y));
            }
        }
        let inputs: Vec<INPUT> = match &event {
            Payload::MouseMove { x, y, .. } => mouse_move_inputs(*x, *y),
            Payload::MouseButton { button, down } => mouse_button_inputs(*button, *down),
            Payload::MouseWheel { dx, dy } => mouse_wheel_inputs(*dx, *dy),
            Payload::Key { key, scan, extended, down } => {
                key_inputs(*key, *scan, *extended, *down)
            }
            other => {
                log::debug!("注入跳过（非输入事件）: {other:?}");
                return 0;
            }
        };
        if inputs.is_empty() {
            return 0;
        }
        let n = unsafe { SendInput(&inputs, size_of::<INPUT>() as i32) };
        log::debug!("注入: {event:?} → SendInput 返回 {n}");
        n
    }
}

fn mouse_move_inputs(x: i32, y: i32) -> Vec<INPUT> {
    // 钩子给的 pt 是物理像素坐标；SendInput ABSOLUTE 要求归一化到 0~65535
    let (w, h) = screen_size();
    if w == 0 || h == 0 {
        return Vec::new();
    }
    let dx = (x.clamp(0, w - 1) as f64 * 65535.0 / w as f64).round() as i32;
    let dy = (y.clamp(0, h - 1) as f64 * 65535.0 / h as f64).round() as i32;
    vec![INPUT {
        r#type: INPUT_MOUSE,
        Anonymous: INPUT_0 {
            mi: MOUSEINPUT {
                dx,
                dy,
                mouseData: 0,
                dwFlags: MOUSEEVENTF_MOVE | MOUSEEVENTF_ABSOLUTE,
                time: 0,
                dwExtraInfo: 0,
            },
        },
    }]
}

fn mouse_button_inputs(button: u8, down: bool) -> Vec<INPUT> {
    let flag = match (button, down) {
        (0, true) => MOUSEEVENTF_LEFTDOWN,
        (0, false) => MOUSEEVENTF_LEFTUP,
        (1, true) => MOUSEEVENTF_RIGHTDOWN,
        (1, false) => MOUSEEVENTF_RIGHTUP,
        (2, true) => MOUSEEVENTF_MIDDLEDOWN,
        (2, false) => MOUSEEVENTF_MIDDLEUP,
        _ => {
            log::warn!("未知鼠标键: {button}");
            return Vec::new();
        }
    };
    vec![INPUT {
        r#type: INPUT_MOUSE,
        Anonymous: INPUT_0 {
            mi: MOUSEINPUT {
                dx: 0,
                dy: 0,
                mouseData: 0,
                dwFlags: flag,
                time: 0,
                dwExtraInfo: 0,
            },
        },
    }]
}

fn mouse_wheel_inputs(dx: i32, dy: i32) -> Vec<INPUT> {
    // 协议按"格"传，注入时还原为 Windows delta（一格 = 120，放高 16 位）
    let to_delta = |clicks: i32| ((clicks * 120) as u32) << 16;
    let mut out = Vec::new();
    if dy != 0 {
        out.push(INPUT {
            r#type: INPUT_MOUSE,
            Anonymous: INPUT_0 {
                mi: MOUSEINPUT {
                    dx: 0,
                    dy: 0,
                    mouseData: to_delta(dy),
                    dwFlags: MOUSEEVENTF_WHEEL,
                    time: 0,
                    dwExtraInfo: 0,
                },
            },
        });
    }
    if dx != 0 {
        out.push(INPUT {
            r#type: INPUT_MOUSE,
            Anonymous: INPUT_0 {
                mi: MOUSEINPUT {
                    dx: 0,
                    dy: 0,
                    mouseData: to_delta(dx),
                    dwFlags: MOUSEEVENTF_HWHEEL,
                    time: 0,
                    dwExtraInfo: 0,
                },
            },
        });
    }
    out
}

fn key_inputs(key: Key, scan: u16, extended: bool, down: bool) -> Vec<INPUT> {
    let vk = key_to_vk(key);
    if vk == 0 {
        log::warn!("忽略无法映射到 VK 的键: {key:?}");
        return Vec::new();
    }
    let mut flags = KEYBD_EVENT_FLAGS(0);
    if extended {
        flags |= KEYEVENTF_EXTENDEDKEY;
    }
    if !down {
        flags |= KEYEVENTF_KEYUP;
    }
    vec![INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT {
                wVk: VIRTUAL_KEY(vk as u16),
                wScan: scan,
                time: 0,
                dwFlags: flags,
                dwExtraInfo: 0,
            },
        },
    }]
}

/// 主屏物理像素尺寸（跨屏判定用；多显示器 M4 处理）。
pub fn screen_size() -> (i32, i32) {
    unsafe { (GetSystemMetrics(SM_CXSCREEN), GetSystemMetrics(SM_CYSCREEN)) }
}

/// 光标跳转（跨屏回绕用）。物理像素坐标。
pub fn warp_cursor(x: i32, y: i32) {
    unsafe {
        let _ = SetCursorPos(x, y);
    }
}
