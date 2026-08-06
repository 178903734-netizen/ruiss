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
use std::ffi::c_void;
use std::mem::size_of;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Mutex, OnceLock};
use std::thread::{self, JoinHandle};

use anyhow::{anyhow, Result};
use windows::core::{w, PCWSTR};
use windows::Win32::Foundation::{COLORREF, HINSTANCE, HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::Graphics::Gdi::HBRUSH;
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Threading::GetCurrentThreadId;
use windows::Win32::UI::Input::KeyboardAndMouse::{
    INPUT, INPUT_0, INPUT_KEYBOARD, INPUT_MOUSE, KEYBDINPUT, KEYBD_EVENT_FLAGS,
    KEYEVENTF_EXTENDEDKEY, KEYEVENTF_KEYUP, MOUSEINPUT, MOUSEEVENTF_ABSOLUTE, MOUSEEVENTF_HWHEEL,
    MOUSEEVENTF_LEFTDOWN, MOUSEEVENTF_LEFTUP, MOUSEEVENTF_MIDDLEDOWN, MOUSEEVENTF_MIDDLEUP,
    MOUSEEVENTF_MOVE, MOUSEEVENTF_RIGHTDOWN, MOUSEEVENTF_RIGHTUP, MOUSEEVENTF_WHEEL, SendInput,
    VIRTUAL_KEY,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CallNextHookEx, CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW,
    GetMessageW, GetSystemMetrics, KBDLLHOOKSTRUCT, LWA_ALPHA, LLKHF_EXTENDED, LLKHF_INJECTED,
    LLMHF_INJECTED, MSLLHOOKSTRUCT, PostThreadMessageW, RegisterClassW, SetCursorPos,
    SetLayeredWindowAttributes, SetWindowsHookExW, ShowWindow, SystemParametersInfoW,
    TranslateMessage, UnhookWindowsHookEx, UnregisterClassW, WNDCLASSW,
    WH_KEYBOARD_LL, WH_MOUSE_LL, WM_KEYDOWN, WM_KEYUP, WM_LBUTTONDOWN, WM_LBUTTONUP,
    WM_MBUTTONDOWN, WM_MBUTTONUP, WM_MOUSEMOVE, WM_MOUSEWHEEL, WM_QUIT, WM_RBUTTONDOWN,
    WM_RBUTTONUP, WM_SETCURSOR, WM_SYSKEYDOWN, WM_SYSKEYUP, MSG, SM_CXSCREEN, SM_CYSCREEN,
    CreateCursor, SetSystemCursor,
    SPI_SETCURSORS, SYSTEM_CURSOR_ID, SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS, HCURSOR,
    SW_HIDE, SW_SHOWNA, WNDCLASS_STYLES, WS_EX_LAYERED, WS_EX_NOACTIVATE, WS_EX_TOPMOST, WS_POPUP,
    HICON,
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

/// 本机是否处于"被控端"（Sink）：此时吞掉本机 MouseMove——光标只跟对端注入走，
/// 否则本机鼠标一动光标就被抢走，与对端注入"打架" → 双鼠标/乱跳。
static SINK_ACTIVE: AtomicBool = AtomicBool::new(false);

/// 全屏透明"罩子"窗口句柄（存 isize：HWND 是裸指针不 Send，不能进 static Mutex）。
/// 跨屏期间盖住整个桌面，鼠标移动消息打在罩子上（桌面收不到）→ 桌面文件零 hover；
/// 低层钩子仍在罩子之前拿到事件，转发数据链完全不受影响。
static SHIELD_HWND: Mutex<Option<isize>> = Mutex::new(None);

pub fn set_local_input_blocked(blocked: bool) {
    BLOCK_LOCAL_INPUT.store(blocked, Ordering::Relaxed);
    // 罩子窗口开合与跨屏状态联动：显示罩子（不抢焦点）或收起
    if let Ok(g) = SHIELD_HWND.lock() {
        if let Some(raw) = *g {
            let hwnd = HWND(raw as *mut c_void);
            unsafe {
                let _ = ShowWindow(hwnd, if blocked { SW_SHOWNA } else { SW_HIDE });
            }
        }
    }
}

/// 设置本机是否为被控端（Sink）。被控期间吞掉本机 MouseMove（防双鼠标）。
pub fn set_sink_active(active: bool) {
    SINK_ACTIVE.store(active, Ordering::Relaxed);
}

/// 罩子窗口的窗口过程：什么都不做，默认处理即可（窗口只是"接住"鼠标消息）。
/// WM_SETCURSOR 返回 TRUE：拦截系统自动设置光标——否则鼠标移到罩子上时
/// Windows 可能显示忙碌转圈/箭头（SetSystemCursor 替换不覆盖的场景）。
unsafe extern "system" fn shield_wnd_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    if msg == WM_SETCURSOR {
        return LRESULT(1); // TRUE：光标已处理，系统不要切换
    }
    DefWindowProcW(hwnd, msg, wparam, lparam)
}

/// 创建全屏透明罩子窗口（WS_EX_LAYERED 全透明 + 置顶 + 不激活）。
/// 创建后默认隐藏，跨屏开始由 set_local_input_blocked(true) 显示。
fn create_shield_window() -> Option<HWND> {
    unsafe {
        let class = w!("RuissShieldWindow");
        let hinst: HINSTANCE = GetModuleHandleW(None).ok()?.into();
        let wc = WNDCLASSW {
            style: WNDCLASS_STYLES(0),
            lpfnWndProc: Some(shield_wnd_proc),
            cbClsExtra: 0,
            cbWndExtra: 0,
            hInstance: hinst,
            hIcon: HICON::default(),
            hCursor: HCURSOR::default(),
            hbrBackground: HBRUSH::default(),
            lpszMenuName: PCWSTR::null(),
            lpszClassName: class,
        };
        // 类可能已被本进程注册（重复 start），忽略 ERR_ALREADY_EXISTS 即可
        let _ = RegisterClassW(&wc);
        let (w, h) = screen_size();
        let hwnd = CreateWindowExW(
            WS_EX_LAYERED | WS_EX_TOPMOST | WS_EX_NOACTIVATE,
            class,
            w!("RuissShield"),
            WS_POPUP,
            0,
            0,
            w,
            h,
            None,
            None,
            hinst,
            None,
        )
        .ok()?;
        // alpha=1：肉眼不可见但 hit-test 有效（alpha=0 可能被系统当点击穿透）
        let _ = SetLayeredWindowAttributes(hwnd, COLORREF(0), 1, LWA_ALPHA);
        Some(hwnd)
    }
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
    // 创建全屏透明罩子窗口（默认隐藏）：跨屏期间盖住桌面 → 桌面文件零 hover。
    // 窗口在钩子线程创建，消息循环泵它的消息；ShowWindow 由 set_local_input_blocked 从外部控制。
    match create_shield_window() {
        Some(hwnd) => {
            if let Ok(mut g) = SHIELD_HWND.lock() {
                *g = Some(hwnd.0 as isize);
            }
            log::info!("罩子窗口已创建（透明全屏，默认隐藏）");
        }
        None => log::warn!("罩子窗口创建失败（桌面 hover 副作用可能回来）"),
    }
    let _ = ready.send(Ok(tid));

    // 消息循环：GetMessageW 收到 WM_QUIT 返回 FALSE，循环结束
    let mut msg = MSG::default();
    unsafe {
        while GetMessageW(&mut msg, None, 0, 0).as_bool() {
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }

    // 清理：销毁罩子窗口 + 卸钩子 + 释放通道（Sender 全部 drop 后消费线程自然退出）
    if let Ok(mut g) = SHIELD_HWND.lock() {
        if let Some(raw) = g.take() {
            let hwnd = HWND(raw as *mut c_void);
            unsafe {
                let _ = DestroyWindow(hwnd);
            }
        }
    }
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
                // 跨屏期间：点击/滚轮吞掉（只转发对端，本机不生效）；移动放行（本机光标还要动）。
                // 被控端（Sink）：本机 MouseMove 也吞掉——光标只跟对端注入走，
                // 否则本机鼠标一动光标就被抢走，与对端注入"打架" → 双鼠标/乱跳。
                let swallow = BLOCK_LOCAL_INPUT.load(Ordering::Relaxed)
                    && matches!(p, Payload::MouseButton { .. } | Payload::MouseWheel { .. });
                let sink_swallow = SINK_ACTIVE.load(Ordering::Relaxed)
                    && matches!(p, Payload::MouseMove { .. });
                HOOK_SENDER.with(|s| {
                    if let Some(tx) = s.borrow().as_ref() {
                        let _ = tx.send(p);
                    }
                });
                if swallow || sink_swallow {
                    return LRESULT(1);
                }
                // 移动补藏：跨屏期间每个真实鼠标移动事件后补一次隐藏
                // （对抗 tao 等外部 ShowCursor(TRUE) 把计数拉回 0）
                if wparam.0 as u32 == WM_MOUSEMOVE {
                    enforce_cursor_hidden();
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
/// 使用 SetSystemCursor 替换系统光标资源为透明图标——内核级替换，
/// 不受 per-thread ShowCursor 计数器或其他窗口 SetCursor 影响。
static CURSOR_SUPPRESS: AtomicBool = AtomicBool::new(false);

/// 创建 1x1 全透明光标。
/// AND 掩码全 1（不改变桌面像素），XOR 掩码全 0（不绘制任何像素）→ 完全透明。
fn create_transparent_cursor() -> Option<HCURSOR> {
    unsafe {
        let and_mask: [u8; 4] = [0xFF, 0xFF, 0xFF, 0xFF];
        let xor_mask: [u8; 4] = [0x00, 0x00, 0x00, 0x00];
        CreateCursor(
            None,
            0, 0, 1, 1,
            and_mask.as_ptr() as *const c_void,
            xor_mask.as_ptr() as *const c_void,
        )
        .ok()
    }
}

/// 隐藏本机光标 — 用 SetSystemCursor 把系统光标资源替换为透明图标。
/// 不管哪个窗口通过 LoadCursor/SetCursor 显示光标，拿到的都是透明图。
pub fn hide_cursor() {
    if CURSOR_SUPPRESS.swap(true, Ordering::Relaxed) {
        return; // 已在隐藏状态
    }

    // 需要替换的系统光标类型（覆盖最常用的；32514 = OCR_WAIT 纯转圈也要盖，
    // 否则跨屏时 Windows 切到忙碌光标会显示转圈）
    let cursor_ids: [SYSTEM_CURSOR_ID; 5] = [
        SYSTEM_CURSOR_ID(32512), // OCR_NORMAL      — 箭头
        SYSTEM_CURSOR_ID(32513), // OCR_IBEAM       — 文本选择
        SYSTEM_CURSOR_ID(32514), // OCR_WAIT        — 忙碌（纯转圈）
        SYSTEM_CURSOR_ID(32649), // OCR_HAND        — 手型（链接悬停）
        SYSTEM_CURSOR_ID(32650), // OCR_APPSTARTING — 后台忙（转圈+箭头）
    ];

    for &id in &cursor_ids {
        // SetSystemCursor 会销毁传入的光标句柄，所以每次新建一个
        if let Some(c) = create_transparent_cursor() {
            let _ = unsafe { SetSystemCursor(c, id) };
        }
    }

    log::info!("已隐藏光标（SetSystemCursor 替换为透明图标）");
}

/// 恢复显示本机光标 — SPI_SETCURSORS 从注册表重新加载所有系统光标。
/// 无条件执行（不检查 CURSOR_SUPPRESS）：即使上次运行崩溃/被强杀导致
/// 系统光标停留在透明状态，本次启动调用也能恢复。
pub fn show_cursor() {
    CURSOR_SUPPRESS.store(false, Ordering::Relaxed);

    // SPI_SETCURSORS：通知系统从 HKCU\Control Panel\Cursors 重新加载所有光标
    unsafe {
        let _ = SystemParametersInfoW(
            SPI_SETCURSORS,
            0,
            None,
            SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS(0),
        );
    }

    log::info!("已恢复光标（SPI_SETCURSORS 从注册表重载）");
}

/// SetSystemCursor 是系统级替换，无需补藏。保留空实现兼容外部调用。
pub fn enforce_cursor_hidden() {}

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
