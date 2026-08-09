// macOS 平台实现（M2）：CGEventTap 捕获 + CGEventPost 注入。
//
// 前提：需要"辅助功能"权限（系统设置 → 隐私与安全性 → 辅助功能），
//       未授权时 CGEventTap 收不到任何事件。权限检查用 AXIsProcessTrusted()。
//
// 防回环（与 Windows 的 LLKHF_INJECTED 对应）：注入事件的事件源进程 ID
// 为本进程（非 0），真实硬件事件的进程 ID 为 0 —— 按 pid 过滤注入事件，
// 本机回环不互相循环，对端（Windows）注入到本机的事件也不会再转发。
//
// 注意：本文件无法在 Windows 上编译验证；API 以 core-graphics 0.24 为准，
// 如编译报错请把错误信息发回迭代。

use std::cell::RefCell;
use std::ffi::c_void;
use std::sync::atomic::{AtomicBool, AtomicI32, AtomicUsize, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::Mutex;
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use anyhow::{anyhow, Result};
use core_foundation::base::TCFType;
use core_foundation::boolean::CFBoolean;
use core_foundation::runloop::{kCFRunLoopCommonModes, CFRunLoop};
use core_foundation::string::CFString;
use core_graphics::event::{
    CGEvent, CGEventFlags, CGEventTap, CGEventTapLocation, CGEventTapOptions, CGEventTapPlacement,
    CGEventType, CGMouseButton, EventField, ScrollEventUnit,
};
use core_graphics::event_source::{CGEventSource, CGEventSourceStateID};
use core_graphics::geometry::CGPoint;
use objc::{sel, sel_impl};

use crate::core::keys::{map_key, Key};
use crate::core::protocol::Payload;

/// 平台标记：来源端是否 Mac（用于键位映射方向）。
pub const TARGET_IS_MAC: bool = true;

/// iPhone 镜像（macOS Sequoia 15+）的 bundle identifier（进程名 ScreenContinuity）。
/// 侧键映射快捷键只在该 App 前台时生效；若实测不匹配（bundle id 变动）改这里即可。
const IPHONE_MIRRORING_BUNDLE_ID: &str = "com.apple.ScreenContinuity";

#[link(name = "ApplicationServices", kind = "framework")]
extern "C" {
    fn AXIsProcessTrusted() -> i32;
}

#[link(name = "CoreFoundation", kind = "framework")]
extern "C" {
    fn CFRunLoopStop(rl: *const c_void);
}

// 系统级光标隐藏（CoreGraphics）：与 Windows 的 SetSystemCursor 对称。
// NSCursor hide 是 app 级——只在光标位于本应用窗口时生效，跨屏时光标在
// 任意窗口上 hide 直接失效（mac→win 双鼠标根因之一）。CGDisplayHideCursor 是
// 系统级，不挑窗口，且鼠标移动不会自动重显光标（无需"移动补藏"）。
//
// 但 CGDisplayHideCursor 只对【前台应用】生效（Apple 官方文档原话：
// "To use these functions, your application must be in the foreground"）。
// 跨屏瞬间本进程必在后台（焦点在桌面/其他窗口）→ hide 静默无效
// （mac→win 双鼠标根因之二）。解法照抄 Synergy/InputLeap 的
// OSXScreen::hideCursor()：先经私有 CGS API 设 "SetsCursorInBackground"=true
// 允许后台进程控制光标，再调 CGDisplayHideCursor，最后补一句
// CGAssociateMouseAndMouseCursorPosition(true)（InputLeap 注释：修
// "mouse randomly not hiding" 玄学 bug）。私有符号由 CoreGraphics 导出，
// 自用工具不上 App Store，无审核风险。
// CGDirectDisplayID = u32，CGError = i32，CGSConnectionID = i32，boolean_t = i32。
#[link(name = "CoreGraphics", kind = "framework")]
extern "C" {
    fn CGMainDisplayID() -> u32;
    fn CGDisplayHideCursor(display: u32) -> i32;
    fn CGDisplayShowCursor(display: u32) -> i32;
    fn CGAssociateMouseAndMouseCursorPosition(connected: i32) -> i32;
    fn _CGSDefaultConnection() -> i32;
    fn CGSSetConnectionProperty(
        cid: i32,
        target_cid: i32,
        key: *const c_void,
        value: *const c_void,
    ) -> i32;
}

/// 允许后台进程控制光标（Synergy/InputLeap 同款前置步骤）。
/// 不设它，CGDisplayHideCursor/ShowCursor 在本进程非前台时静默无效。
/// hide/show 前各调一次（与 InputLeap 一致；属性是幂等设置，重复调无副作用）。
fn allow_background_cursor_control() {
    unsafe {
        let key = CFString::from_static_string("SetsCursorInBackground");
        let val = CFBoolean::true_value();
        let cid = _CGSDefaultConnection();
        let err = CGSSetConnectionProperty(
            cid,
            cid,
            key.as_concrete_TypeRef() as *const c_void,
            val.as_concrete_TypeRef() as *const c_void,
        );
        if err != 0 {
            log::warn!("[MAC-CURSOR] CGSSetConnectionProperty(SetsCursorInBackground) err={err}");
        }
    }
}

/// 辅助功能是否已授权（捕获的前提）。
pub fn accessibility_trusted() -> bool {
    unsafe { AXIsProcessTrusted() != 0 }
}

/// 捕获线程 runloop 指针（stop 时通知退出）。
static RUNLOOP_PTR: AtomicUsize = AtomicUsize::new(0);

/// 跨屏期间是否拦截本机键盘/点击/滚轮（只转发对端，本机不生效；移动放行）。
static BLOCK_LOCAL_INPUT: AtomicBool = AtomicBool::new(false);

/// 本机是否处于"被控端"（Sink）：此时吞掉本机 MouseMoved——光标只跟对端注入走，
/// 否则本机触控板一动光标就被抢走，与对端注入"打架" → 双鼠标/乱跳。
static SINK_ACTIVE: AtomicBool = AtomicBool::new(false);

/// 跨屏期间是否应保持光标隐藏（Source 跨屏时 true）。
/// 仅作状态标记（tap 回调判断/日志用）；实际隐藏由 CURSOR_HIDDEN 幂等驱动。
static CURSOR_SUPPRESS: AtomicBool = AtomicBool::new(false);

/// 光标是否已实际隐藏（幂等标志，防 CGDisplayHideCursor/ShowCursor 重复调用失衡）。
/// hide_cursor 在 false→true 时调一次 CGDisplayHideCursor；show_cursor 在 true→false
/// 时调一次 CGDisplayShowCursor，严格配对一次，绝不叠加（CG 系列内部也有计数语义）。
static CURSOR_HIDDEN: AtomicBool = AtomicBool::new(false);

pub fn set_local_input_blocked(blocked: bool) {
    BLOCK_LOCAL_INPUT.store(blocked, Ordering::Relaxed);
}

/// 被控端（Sink）：本机鼠标的虚拟位置（由相对位移 delta 累积）。
/// 被控时光标不跟随本机鼠标（防双鼠标），但本机鼠标物理移动的
/// delta 累积成虚拟位置——推到出口边即可反向夺回控制权（自由切换）。
static LOCAL_VIRTUAL_POS: Mutex<Option<(f64, f64)>> = Mutex::new(None);

/// 设置本机是否为被控端（Sink）。被控期间吞掉本机 MouseMoved（防双鼠标）。
pub fn set_sink_active(active: bool) {
    SINK_ACTIVE.store(active, Ordering::Relaxed);
    if active {
        // 新一輪被控：虚拟位置清空，等待首次移动用当前光标位置初始化
        if let Ok(mut g) = LOCAL_VIRTUAL_POS.lock() {
            *g = None;
        }
    }
}

/// tap 回调往消费线程送事件的通道（线程局部：捕获线程装填）。
thread_local! {
    static HOOK_SENDER: RefCell<Option<Sender<Payload>>> = const { RefCell::new(None) };
}

/// 注入侧当前按住的鼠标按钮（-1 = 未按住）。
/// macOS 拖选/拖拽要求按住按钮期间的移动事件类型为 *MouseDragged
/// （MouseMoved 不会更新选区）；注入移动时按此状态选择事件类型。
static HELD_BUTTON: AtomicI32 = AtomicI32::new(-1);

/// 注入侧最后已知光标位置（点击/滚轮注入用；Sink 侧返回判定用）。
static LAST_POS: Mutex<Option<(f64, f64)>> = Mutex::new(None);

/// 最近一次注入到本机的光标位置（None = 还没注入过移动）。
pub fn last_injected_pos() -> Option<(i32, i32)> {
    match LAST_POS.lock() {
        Ok(g) => g.map(|(x, y)| (x as i32, y as i32)),
        Err(_) => None,
    }
}

/// 隐藏本机光标（跨屏期间源端光标"离开"本机屏幕，避免双光标）。
/// 用 CGDisplayHideCursor 系统级隐藏——不挑光标在哪个窗口，且鼠标移动
/// 不会自动重显（无需"移动补藏"，与 Windows 的 SetSystemCursor 对称）。
/// 幂等：仅在 false→true 时调一次 CGDisplayHideCursor，重复调用不叠加。
pub fn hide_cursor() {
    CURSOR_SUPPRESS.store(true, Ordering::SeqCst);
    // swap 返回旧值；原本未隐藏才真正调一次，避免计数叠加
    if !CURSOR_HIDDEN.swap(true, Ordering::SeqCst) {
        allow_background_cursor_control(); // 后台进程也能控光标（关键！不设则 hide 静默无效）
        unsafe {
            let disp = CGMainDisplayID();
            let err = CGDisplayHideCursor(disp);
            // InputLeap：补一句重新关联鼠标与光标位置，修 "randomly not hiding" 玄学 bug
            let _ = CGAssociateMouseAndMouseCursorPosition(1);
            log::info!("[MAC-CURSOR] hide_cursor: CGDisplayHideCursor err={err}");
        }
    }
}

/// 恢复显示本机光标（幂等：仅 true→false 时调一次 CGDisplayShowCursor，
/// 与 hide_cursor 严格配对一次）。
pub fn show_cursor() {
    CURSOR_SUPPRESS.store(false, Ordering::SeqCst);
    if CURSOR_HIDDEN.swap(false, Ordering::SeqCst) {
        allow_background_cursor_control(); // show 同样要求前台，先开后台权限
        unsafe {
            let disp = CGMainDisplayID();
            let err = CGDisplayShowCursor(disp);
            let _ = CGAssociateMouseAndMouseCursorPosition(1);
            log::info!("[MAC-CURSOR] show_cursor: CGDisplayShowCursor err={err}");
        }
    }
}

/// 跨屏期间补藏光标。CGDisplayHideCursor 是系统级、鼠标移动不重显，无需补藏，
/// 此处保留空实现供 lib.rs tick 统一调用（与 win.rs 同名函数对称）。
pub fn enforce_cursor_hidden() {}

/// 捕获器：CGEventTap + 捕获线程 + 消费线程。
pub struct InputCapturer {
    hook_thread: JoinHandle<()>,
    consume_thread: JoinHandle<()>,
}

impl InputCapturer {
    /// 启动捕获。`on_event` 在独立消费线程里被调用。
    pub fn start(on_event: impl Fn(Payload) + Send + 'static) -> Result<Self> {
        if !accessibility_trusted() {
            return Err(anyhow!(
                "未授予辅助功能权限：请到 系统设置 → 隐私与安全性 → 辅助功能 勾选本应用后重启"
            ));
        }
        let (tx, rx): (Sender<Payload>, Receiver<Payload>) = mpsc::channel();
        let (ready_tx, ready_rx) = mpsc::channel::<Result<()>>();

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
            Ok(Ok(())) => Ok(Self { hook_thread, consume_thread }),
            Ok(Err(e)) => Err(e),
            Err(_) => Err(anyhow!("事件 tap 线程异常退出")),
        }
    }

    /// 停止捕获：通知 runloop 退出，线程 join。
    pub fn stop(self) {
        let ptr = RUNLOOP_PTR.load(Ordering::Relaxed) as *const c_void;
        if !ptr.is_null() {
            unsafe { CFRunLoopStop(ptr) };
        }
        let _ = self.hook_thread.join();
        let _ = self.consume_thread.join();
    }
}

/// 捕获线程主体：创建 tap → 加入 runloop → 泵事件。
fn run_hook_loop(tx: Sender<Payload>, ready: Sender<Result<()>>) {
    let event_types = vec![
        CGEventType::MouseMoved,
        CGEventType::LeftMouseDragged,
        CGEventType::RightMouseDragged,
        CGEventType::OtherMouseDragged,
        CGEventType::LeftMouseDown,
        CGEventType::LeftMouseUp,
        CGEventType::RightMouseDown,
        CGEventType::RightMouseUp,
        CGEventType::OtherMouseDown,
        CGEventType::OtherMouseUp,
        CGEventType::ScrollWheel,
        CGEventType::KeyDown,
        CGEventType::KeyUp,
        CGEventType::FlagsChanged,
    ];
    let tap = match CGEventTap::new(
        CGEventTapLocation::HID,
        CGEventTapPlacement::HeadInsertEventTap,
        CGEventTapOptions::Default,
        event_types,
        move |_proxy, event_type, event| {
            // 防回环：注入事件的来源进程 ID 非 0（真实硬件事件为 0）
            let pid = event.get_integer_value_field(EventField::EVENT_SOURCE_UNIX_PROCESS_ID);
            if pid != 0 {
                // 本进程注入的事件（如 warp_cursor）。CGDisplayHideCursor 系统级、
                // 鼠标移动不重显，注入移动不会再把光标拉回显示，无需补藏。
                return Some(event.clone());
            }
            // 被控端（Sink）：本机 MouseMoved 吞掉（光标不跟随本机，防双鼠标），
            // 但相对位移 delta 累积成虚拟位置发送给仲裁器——本机鼠标推到
            // 出口边即可反向夺回控制权（自由切换）。
            if SINK_ACTIVE.load(Ordering::Relaxed)
                && matches!(event_type, CGEventType::MouseMoved)
            {
                let dx = event.get_double_value_field(EventField::MOUSE_EVENT_DELTA_X);
                let dy = event.get_double_value_field(EventField::MOUSE_EVENT_DELTA_Y);
                let (vx, vy) = {
                    let mut g = match LOCAL_VIRTUAL_POS.lock() {
                        Ok(g) => g,
                        Err(p) => p.into_inner(),
                    };
                    match *g {
                        Some((x, y)) => {
                            let nx = x + dx;
                            let ny = y + dy;
                            *g = Some((nx, ny));
                            (nx, ny)
                        }
                        None => {
                            // 首次移动：以当前光标位置（对端注入位置）为起点
                            let loc = event.location();
                            *g = Some((loc.x, loc.y));
                            (loc.x, loc.y)
                        }
                    }
                };
                let (w, h) = screen_size();
                let _ = tx.send(Payload::MouseMove {
                    x: vx as i32,
                    y: vy as i32,
                    src_w: w as u32,
                    src_h: h as u32,
                });
                // 吞掉真实事件：光标不跟随本机鼠标（防双鼠标）
                return None;
            }
            if let Some(p) = event_to_payload(event_type, event) {
                let _ = tx.send(p.clone());
                // CGDisplayHideCursor 系统级、鼠标移动不重显，无需"移动补藏"。
                // 跨屏期间（主控或被控都算）：键盘/点击/滚轮吞掉（只转发对端，本机不生效）；移动放行
                // 注意：不用 CURSOR_SUPPRESS 兜底——它表示"光标隐藏中"，若 show_cursor 未配对执行
                // 会卡 true 导致空闲状态也吞输入（右击/滚动全废）；BLOCK||SINK 已完整覆盖跨屏链路。
                let blocked = BLOCK_LOCAL_INPUT.load(Ordering::Relaxed);
                let sink = SINK_ACTIVE.load(Ordering::Relaxed);
                let is_scroll = matches!(p, Payload::MouseWheel { .. });
                if is_scroll {
                    log::info!(
                        "[MAC-SCROLL-DIAG] delta={p:?} blocked={blocked} sink={sink} suppress={}",
                        CURSOR_SUPPRESS.load(Ordering::SeqCst),
                    );
                }
                if (blocked || sink)
                    && matches!(
                        p,
                        Payload::Key { .. } | Payload::MouseButton { .. } | Payload::MouseWheel { .. }
                    )
                {
                    if is_scroll {
                        log::info!("[MAC-SCROLL-DIAG] 滚轮被吞掉（blocked={blocked} sink={sink}）");
                    }
                    return None;
                }
                if is_scroll {
                    log::info!("[MAC-SCROLL-DIAG] 滚轮放行（本地生效）");
                }
            }
            // 监听模式：原样放行，绝不吞事件
            Some(event.clone())
        },
    ) {
        Ok(t) => t,
        Err(_) => {
            log::error!("CGEventTap 创建失败");
            let _ = ready.send(Err(anyhow!("CGEventTap 创建失败")));
            return;
        }
    };

    // tap 通过 mach_port 创建 runloop source 并加入当前 runloop
    let runloop = CFRunLoop::get_current();
    let loop_source = tap
        .mach_port
        .create_runloop_source(0)
        .expect("create_runloop_source 失败");
    // kCFRunLoopCommonModes 是 extern static，读取需 unsafe
    unsafe {
        runloop.add_source(&loop_source, kCFRunLoopCommonModes);
    }
    tap.enable();
    RUNLOOP_PTR.store(runloop.as_concrete_TypeRef() as usize, Ordering::Relaxed);

    let _ = ready.send(Ok(()));
    log::info!("CGEventTap 已启动");
    CFRunLoop::run_current();

    // runloop 退出：清理
    RUNLOOP_PTR.store(0, Ordering::Relaxed);
    HOOK_SENDER.with(|s| *s.borrow_mut() = None);
    log::info!("事件 tap 线程已退出");
}

fn event_to_payload(event_type: CGEventType, event: &CGEvent) -> Option<Payload> {
    match event_type {
        // 拖动（按住按键期间移动）与普通移动统一转 MouseMove 转发：
        // 对端注入时会根据自身的按住状态还原为对应 Dragged 类型
        CGEventType::MouseMoved
        | CGEventType::LeftMouseDragged
        | CGEventType::RightMouseDragged
        | CGEventType::OtherMouseDragged => {
            let p = event.location();
            let (w, h) = screen_size();
            Some(Payload::MouseMove { x: p.x as i32, y: p.y as i32, src_w: w as u32, src_h: h as u32 })
        }
        CGEventType::LeftMouseDown => Some(Payload::MouseButton { button: 0, down: true }),
        CGEventType::LeftMouseUp => Some(Payload::MouseButton { button: 0, down: false }),
        CGEventType::RightMouseDown => Some(Payload::MouseButton { button: 1, down: true }),
        CGEventType::RightMouseUp => Some(Payload::MouseButton { button: 1, down: false }),
        CGEventType::OtherMouseDown => Some(Payload::MouseButton { button: 2, down: true }),
        CGEventType::OtherMouseUp => Some(Payload::MouseButton { button: 2, down: false }),
        CGEventType::ScrollWheel => {
            let dy = event.get_integer_value_field(EventField::SCROLL_WHEEL_EVENT_POINT_DELTA_AXIS_1);
            let dx = event.get_integer_value_field(EventField::SCROLL_WHEEL_EVENT_POINT_DELTA_AXIS_2);
            Some(Payload::MouseWheel { dx: dx as i32, dy: dy as i32 })
        }
        CGEventType::KeyDown | CGEventType::KeyUp => {
            // 过滤自动重复（按住不放的重复帧不需要转发，目标端自己会重复）
            if matches!(event_type, CGEventType::KeyDown) {
                let repeat = event.get_integer_value_field(EventField::KEYBOARD_EVENT_AUTOREPEAT);
                if repeat != 0 {
                    return None;
                }
            }
            let code = event.get_integer_value_field(EventField::KEYBOARD_EVENT_KEYCODE) as u16;
            Some(Payload::Key {
                key: cg_to_key(code),
                scan: 0,
                extended: false,
                down: matches!(event_type, CGEventType::KeyDown),
            })
        }
        CGEventType::FlagsChanged => {
            let code = event.get_integer_value_field(EventField::KEYBOARD_EVENT_KEYCODE) as u16;
            let key = cg_to_key(code);
            let down = modifier_down(&key, event);
            Some(Payload::Key { key, scan: 0, extended: false, down })
        }
        _ => None,
    }
}

/// 修饰键当前是否按下（FlagsChanged 事件的 down/up 判断）。
fn modifier_down(key: &Key, event: &CGEvent) -> bool {
    let f = event.get_flags();
    match key {
        Key::Shift => f.contains(CGEventFlags::CGEventFlagShift),
        Key::Ctrl => f.contains(CGEventFlags::CGEventFlagControl),
        Key::Alt => f.contains(CGEventFlags::CGEventFlagAlternate),
        Key::Super => f.contains(CGEventFlags::CGEventFlagCommand),
        _ => true,
    }
}

/// 双击识别窗口：两次点击间隔超过 500ms 不算双击（对齐 macOS 系统默认阈值）。
const DOUBLE_CLICK_WINDOW: Duration = Duration::from_millis(500);
/// 双击位置容差：两次点击落点偏差超过 4px 不算双击
/// （略大于跨屏抖动容差 JITTER_TOLERANCE=3，给注入坐标取整留余量）。
const DOUBLE_CLICK_DISTANCE: f64 = 4.0;

/// 点击计数状态：注入侧维护，识别"连续两次点击同一位置"= 双击。
/// macOS 双击识别依赖注入事件的 kCGMouseEventClickState 字段，
/// 不设置时系统把两次点击当两次单击 → Finder 双击打开失效。
struct ClickState {
    last_time: Instant,
    last_pos: (f64, f64),
    count: u32,
}

impl ClickState {
    fn new() -> Self {
        Self {
            last_time: Instant::now(),
            last_pos: (f64::NAN, f64::NAN),
            count: 0,
        }
    }
}

/// 注入器：CGEventPost 回放事件。
pub struct InputInjector {
    /// 点击计数状态（跨线程：注入可能在消费线程/主线程被调用）。
    click: Mutex<ClickState>,
}

impl InputInjector {
    pub fn new() -> Self {
        Self {
            click: Mutex::new(ClickState::new()),
        }
    }

    /// 注入一条事件，返回 1 表示已投递（0 = 失败/跳过）。
    pub fn inject(&self, event: Payload) -> u32 {
        // 记录最后光标位置（点击/滚轮注入落点；Sink 侧返回判定用）
        if let Payload::MouseMove { x, y, .. } = &event {
            if let Ok(mut p) = LAST_POS.lock() {
                *p = Some((*x as f64, *y as f64));
            }
        }
        let source = match CGEventSource::new(CGEventSourceStateID::CombinedSessionState) {
            Ok(s) => s,
            Err(_) => return 0,
        };
        let cg = match &event {
            Payload::MouseMove { x, y, .. } => {
                // 按住左键拖动期间必须注入 LeftMouseDragged（MouseMoved 不会更新选区
                // → 拖选文字失效）；右键/中键同理。未按住时正常 MouseMoved。
                let held = HELD_BUTTON.load(Ordering::Relaxed);
                let ty = match held {
                    0 => CGEventType::LeftMouseDragged,
                    1 => CGEventType::RightMouseDragged,
                    2 => CGEventType::OtherMouseDragged,
                    _ => CGEventType::MouseMoved,
                };
                let btn = match held {
                    1 => CGMouseButton::Right,
                    2 => CGMouseButton::Center,
                    _ => CGMouseButton::Left,
                };
                CGEvent::new_mouse_event(source, ty, CGPoint::new(*x as f64, *y as f64), btn)
            }
            Payload::MouseButton { button, down } => {
                // 侧键（罗技 XButton1/XButton2 → 协议 button 3/4）：
                // iPhone 镜像前台时映射为快捷键（后退→Cmd+1 主屏幕 / 前进→Cmd+2 App 切换），
                // 其他 App 注入原生侧键事件，保持浏览器前进/后退等原语义。
                if *button == 3 || *button == 4 {
                    return self.inject_side_button(*button, *down);
                }
                // 记录当前按住的按钮：拖动期间的移动要注入为 *Dragged 类型
                // （up 后清空；macOS 拖选依赖这个状态还原事件类型）
                HELD_BUTTON.store(if *down { *button as i32 } else { -1 }, Ordering::Relaxed);
                let (ty, btn) = match (button, down) {
                    (0, true) => (CGEventType::LeftMouseDown, CGMouseButton::Left),
                    (0, false) => (CGEventType::LeftMouseUp, CGMouseButton::Left),
                    (1, true) => (CGEventType::RightMouseDown, CGMouseButton::Right),
                    (1, false) => (CGEventType::RightMouseUp, CGMouseButton::Right),
                    (2, true) => (CGEventType::OtherMouseDown, CGMouseButton::Center),
                    (2, false) => (CGEventType::OtherMouseUp, CGMouseButton::Center),
                    _ => return 0,
                };
                let (x, y) = match LAST_POS.lock() {
                    Ok(g) => g.unwrap_or((0.0, 0.0)),
                    Err(_) => (0.0, 0.0),
                };
                let ev = CGEvent::new_mouse_event(source, ty, CGPoint::new(x, y), btn);
                // 双击识别：macOS 靠注入事件的 kCGMouseEventClickState 字段识别双击，
                // 不设置时系统把两次注入点击当成两次单击 → Finder 双击打开失效。
                // 关键：down 和 up 都必须携带 click state，且抬起与按下用同一个计数
                // （cliclick 同款：down1/up1 → down2/up2）。只给 down 设置、up 保持
                // 默认值时，系统把第二次 up 当第二次单击，双击永远凑不成。
                if let Ok(ev) = &ev {
                    let mut st = match self.click.lock() {
                        Ok(g) => g,
                        Err(p) => p.into_inner(),
                    };
                    if *down {
                        let now = Instant::now();
                        let same_spot = (st.last_pos.0 - x).abs() <= DOUBLE_CLICK_DISTANCE
                            && (st.last_pos.1 - y).abs() <= DOUBLE_CLICK_DISTANCE;
                        st.count = if now.duration_since(st.last_time) <= DOUBLE_CLICK_WINDOW
                            && same_spot
                        {
                            st.count + 1
                        } else {
                            1
                        };
                        st.last_time = now;
                        st.last_pos = (x, y);
                    }
                    // down 计算/更新计数，up 沿用当前计数（不重置、不再判定）
                    ev.set_integer_value_field(EventField::MOUSE_EVENT_CLICK_STATE, st.count as i64);
                }
                ev
            }
            Payload::MouseWheel { dx, dy } => {
                // Win 端 wheel_delta 是格数（±1），LINE 单位注入 macOS 响应极弱（等同没滚）；
                // 改 PIXEL 单位放大到 ~100px/格，保证对端滚轮在 Mac 上可感知。
                let scale = 100;
                CGEvent::new_scroll_event(
                    source,
                    ScrollEventUnit::PIXEL,
                    1,
                    (*dy as i32) * scale,
                    (*dx as i32) * scale,
                    0,
                )
            }
            Payload::Key { key, down, .. } => {
                let key = map_key(true, *key); // 目标为 Mac：Ctrl ↔ Command 映射
                CGEvent::new_keyboard_event(source, key_to_cg(key), *down)
            }
            other => {
                log::debug!("注入跳过（非输入事件）: {other:?}");
                return 0;
            }
        };
        let Ok(cg) = cg else { return 0 };
        cg.post(CGEventTapLocation::HID);
        1
    }

    /// 侧键（协议 button 3/4）注入：
    /// - iPhone 镜像前台 → 注入 Cmd+1 / Cmd+2 键盘快捷键（带 Command flag，不碰修饰键状态）；
    /// - 其他 App → 注入原生侧键鼠标事件（OtherMouseDown/Up + button number 3/4）。
    /// 返回 1 表示已投递。
    fn inject_side_button(&self, button: u8, down: bool) -> u32 {
        if frontmost_bundle_id().as_deref() == Some(IPHONE_MIRRORING_BUNDLE_ID) {
            // 后退侧键(3) → Cmd+1 主屏幕；前进侧键(4) → Cmd+2 App 切换
            let key = if button == 3 { Key::Digit1 } else { Key::Digit2 };
            let Some(source) = CGEventSource::new(CGEventSourceStateID::CombinedSessionState).ok()
            else {
                return 0;
            };
            if let Ok(ev) = CGEvent::new_keyboard_event(source, key_to_cg(key), down) {
                ev.set_flags(CGEventFlags::CGEventFlagCommand);
                ev.post(CGEventTapLocation::HID);
            }
            return 1;
        }
        // 非 iPhone 镜像：注入原生侧键事件（不改变系统前进/后退语义）
        let Some(source) = CGEventSource::new(CGEventSourceStateID::CombinedSessionState).ok()
        else {
            return 0;
        };
        let (x, y) = match LAST_POS.lock() {
            Ok(g) => g.unwrap_or((0.0, 0.0)),
            Err(_) => (0.0, 0.0),
        };
        let ty = if down {
            CGEventType::OtherMouseDown
        } else {
            CGEventType::OtherMouseUp
        };
        if let Ok(ev) = CGEvent::new_mouse_event(source, ty, CGPoint::new(x, y), CGMouseButton::Center)
        {
            ev.set_integer_value_field(EventField::MOUSE_EVENT_BUTTON_NUMBER, button as i64);
            ev.post(CGEventTapLocation::HID);
        }
        1
    }
}

/// 前台 App 的 bundle identifier（NSWorkspace.frontmostApplication）。
/// 失败（无权限/异常）返回 None —— 侧键按"非 iPhone 镜像"处理，注入原生事件。
fn frontmost_bundle_id() -> Option<String> {
    unsafe {
        let cls = objc::runtime::Class::get("NSWorkspace")?;
        let ws: *mut objc::runtime::Object = objc::msg_send![cls, sharedWorkspace];
        if ws.is_null() {
            return None;
        }
        let app: *mut objc::runtime::Object = objc::msg_send![ws, frontmostApplication];
        if app.is_null() {
            return None;
        }
        let nsid: *mut objc::runtime::Object = objc::msg_send![app, bundleIdentifier];
        if nsid.is_null() {
            return None;
        }
        Some(CFString::wrap_under_get_rule(nsid as *const _).to_string())
    }
}

/// 主屏逻辑尺寸（跨屏判定用；Retina 下 NSScreen 尺寸就是逻辑点）。
pub fn screen_size() -> (i32, i32) {
    unsafe {
        let cls = match objc::runtime::Class::get("NSScreen") {
            Some(c) => c,
            None => return (1920, 1080),
        };
        let screen: *mut objc::runtime::Object = objc::msg_send![cls, mainScreen];
        if screen.is_null() {
            return (1920, 1080);
        }
        let frame: core_graphics::geometry::CGRect = objc::msg_send![screen, frame];
        (frame.size.width as i32, frame.size.height as i32)
    }
}

/// 光标跳转（跨屏回绕用）：注入移动事件代替 CGWarpMouseCursorPosition。
pub fn warp_cursor(x: i32, y: i32) {
    if let Ok(source) = CGEventSource::new(CGEventSourceStateID::CombinedSessionState) {
        if let Ok(ev) = CGEvent::new_mouse_event(
            source,
            CGEventType::MouseMoved,
            CGPoint::new(x as f64, y as f64),
            CGMouseButton::Left,
        ) {
            ev.post(CGEventTapLocation::HID);
        }
    }
}

// ---- Mac CGKeyCode ↔ 抽象键码（keys::Key）映射（标准 ANSI 键位）----

const CG_MAP: &[(u16, Key)] = &[
    // 字母
    (0, Key::A), (11, Key::B), (8, Key::C), (2, Key::D), (14, Key::E), (3, Key::F),
    (5, Key::G), (4, Key::H), (34, Key::I), (38, Key::J), (40, Key::K), (37, Key::L),
    (46, Key::M), (45, Key::N), (31, Key::O), (35, Key::P), (12, Key::Q), (15, Key::R),
    (1, Key::S), (17, Key::T), (32, Key::U), (9, Key::V), (13, Key::W), (7, Key::X),
    (16, Key::Y), (6, Key::Z),
    // 数字（主键盘行）
    (29, Key::Digit0), (18, Key::Digit1), (19, Key::Digit2), (20, Key::Digit3),
    (21, Key::Digit4), (23, Key::Digit5), (22, Key::Digit6), (26, Key::Digit7),
    (28, Key::Digit8), (25, Key::Digit9),
    // 修饰键（左右各一组）
    (56, Key::Shift), (59, Key::Ctrl), (58, Key::Alt), (55, Key::Super),
    (60, Key::Shift), (62, Key::Ctrl), (61, Key::Alt), (54, Key::Super),
    // 功能键
    (36, Key::Enter), (49, Key::Space), (51, Key::Backspace), (48, Key::Tab), (53, Key::Esc),
    (123, Key::ArrowLeft), (126, Key::ArrowUp), (124, Key::ArrowRight), (125, Key::ArrowDown),
    // 导航/编辑键
    (117, Key::Delete), // ForwardDelete（Fn+Delete 的效果）
    (115, Key::Home), (119, Key::End),
    (116, Key::PageUp), (121, Key::PageDown),
    (114, Key::Insert), // Mac 全键盘 Insert 位是 Help 键，这里映射为 Help
    (57, Key::CapsLock),
    // 标点符号（中英文输入必备）
    (43, Key::Comma), (47, Key::Period), (44, Key::Slash),
    (41, Key::Semicolon), (39, Key::Quote),
    (33, Key::LBracket), (30, Key::RBracket), (42, Key::Backslash),
    (27, Key::Minus), (24, Key::Equals), (50, Key::Backtick),
    (122, Key::F1), (120, Key::F2), (99, Key::F3), (118, Key::F4), (96, Key::F5),
    (97, Key::F6), (98, Key::F7), (100, Key::F8), (101, Key::F9), (109, Key::F10),
    (103, Key::F11), (111, Key::F12),
];

/// CGKeyCode → 抽象键码（未覆盖的键透传为 Other）。
pub fn cg_to_key(code: u16) -> Key {
    CG_MAP
        .iter()
        .find(|(c, _)| *c == code)
        .map(|(_, k)| *k)
        .unwrap_or(Key::Other(code as u32))
}

/// 抽象键码 → CGKeyCode（未覆盖的 Other 透传原始码，其余返回 0 表示丢键）。
pub fn key_to_cg(key: Key) -> u16 {
    match key {
        Key::Other(n) => n as u16,
        k => CG_MAP.iter().find(|(_, m)| *m == k).map(|(c, _)| *c).unwrap_or(0),
    }
}
