// macOS 平台实现（M2）：CGEventTap 捕获 + CGEventPost 注入。
//
// 前提：需要"辅助功能"权限（系统设置 → 隐私与安全性 → 辅助功能），
//       未授权时 CGEventTap 收不到任何事件。权限检查用 AXIsProcessTrusted()。
//
// 防回环（与 Windows 的 LLKHF_INJECTED 对应）：所有 Ruiss 注入事件都写入专用
// EVENT_SOURCE_USER_DATA 标记；tap 只按该标记放行自身注入。真实硬件事件和
// WindowServer 合成的惯性滚动均不带标记，跨屏时会被转发并从本机派发链吞掉。
//
// 注意：本文件无法在 Windows 上原生编译验证；API 以 core-graphics 0.25 为准，
// 如编译报错请把错误信息发回迭代。

use std::collections::HashMap;
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
    CallbackResult, CGEvent, CGEventFlags, CGEventTap, CGEventTapLocation, CGEventTapOptions,
    CGEventTapPlacement, CGEventType, CGMouseButton, EventField, ScrollEventUnit,
};
use core_graphics::event_source::{CGEventSource, CGEventSourceStateID};
use core_graphics::geometry::CGPoint;
use objc::{sel, sel_impl};

use crate::core::keys::{
    map_modifiers, translate_windows_shortcut_to_mac, Key, ModifierState, ShortcutStroke,
};
use crate::core::protocol::Payload;

/// 平台标记：来源端是否 Mac（用于键位映射方向）。
pub const TARGET_IS_MAC: bool = true;

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
// （mac→win 双鼠标根因之二）。这里沿用 Synergy/InputLeap 的后台控制方案：
// 先经私有 CGS API 设 "SetsCursorInBackground"=true，再调
// CGDisplayHideCursor。新版不再在跨屏期间断开光标与物理设备的关联；HID tap
// 会在 WindowServer 更新指针前直接吞掉本机移动。
// 私有符号由 CoreGraphics 导出，
// 自用工具不上 App Store，无审核风险。
// CGDirectDisplayID = u32，CGError = i32，CGSConnectionID = i32，boolean_t = i32。
#[link(name = "CoreGraphics", kind = "framework")]
extern "C" {
    fn CGMainDisplayID() -> u32;
    fn CGDisplayHideCursor(display: u32) -> i32;
    fn CGDisplayShowCursor(display: u32) -> i32;
    fn CGAssociateMouseAndMouseCursorPosition(connected: i32) -> i32;
    fn CGEventTapEnable(tap: *const c_void, enable: bool);
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

/// 两个事件 tap 的 Mach port。HID tap 在 WindowServer 处理前抢占物理点击；
/// Session tap 保留现有移动/键盘和完整惯性滚动处理。tap 被系统禁用时，回调
/// 通过对应指针立即重新启用。
static HID_TAP_PORT_PTR: AtomicUsize = AtomicUsize::new(0);
static SESSION_TAP_PORT_PTR: AtomicUsize = AtomicUsize::new(0);

/// Ruiss 自己注入的 CGEvent 标记。不能只用 source pid 防回环：触控板惯性段由
/// WindowServer 合成，source pid 语义并不等价于“是不是 Ruiss 注入”。
const RUISS_EVENT_MARKER: i64 = 0x5255_4953_535F_4556;

/// Source 跨屏期间是否独占本机输入：点击由 HID tap 提前截获，其余输入由
/// Session tap 转发后删除，Mac 本机不再接收。
static BLOCK_LOCAL_INPUT: AtomicBool = AtomicBool::new(false);

/// Dock 热区高度（屏幕底部，逻辑像素）：隐藏光标不能停在里面——macOS 在
/// Dock/菜单栏等系统 UI 区域不约束 CGDisplayHideCursor（光标会被强制重显），
/// 且 Dock 悬停放大动画只看光标位置、不检查可见性，进热区必触发。
const DOCK_HOT_ZONE: i32 = 20;
/// 菜单栏高度（屏幕顶部，逻辑像素）：同上，隐藏光标不能停在里面。
const MENUBAR_HOT_ZONE: i32 = 25;

/// 本机是否处于"被控端"（Sink）：此时吞掉本机 MouseMoved——光标只跟对端注入走，
/// 否则本机触控板一动光标就被抢走，与对端注入"打架" → 双鼠标/乱跳。
static SINK_ACTIVE: AtomicBool = AtomicBool::new(false);

/// 光标是否已实际隐藏（幂等标志，防 CGDisplayHideCursor/ShowCursor 重复调用失衡）。
/// hide_cursor 在 false→true 时调一次 CGDisplayHideCursor；show_cursor 在 true→false
/// 时调一次 CGDisplayShowCursor，严格配对一次，绝不叠加（CG 系列内部也有计数语义）。
static CURSOR_HIDDEN: AtomicBool = AtomicBool::new(false);
/// 仅在进程启动后的首次 show 中执行旧版本崩溃恢复，避免每次接管都调用
/// CGAssociateMouseAndMouseCursorPosition 给跨屏入口增加同步停顿。
static CURSOR_STARTUP_REASSOCIATED: AtomicBool = AtomicBool::new(false);

pub fn set_local_input_blocked(blocked: bool) {
    BLOCK_LOCAL_INPUT.store(blocked, Ordering::SeqCst);
}

/// 被控端（Sink）：本机鼠标的虚拟位置（由相对位移 delta 累积）。
/// 被控时光标不跟随本机鼠标（防双鼠标），但本机鼠标物理移动的
/// delta 累积成虚拟位置——推到出口边即可反向夺回控制权（自由切换）。
static LOCAL_VIRTUAL_POS: Mutex<Option<(f64, f64)>> = Mutex::new(None);

/// 设置本机是否为被控端（Sink）。HID tap 在 WindowServer 处理前吞掉本机移动。
pub fn set_sink_active(active: bool) {
    let was = SINK_ACTIVE.swap(active, Ordering::Relaxed);
    if active && !was {
        // 新一輪被控：虚拟位置清空，等待首次移动用当前光标位置初始化
        if let Ok(mut g) = LOCAL_VIRTUAL_POS.lock() {
            *g = None;
        }
    }
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
    // swap 返回旧值；原本未隐藏才真正调一次，避免计数叠加
    if !CURSOR_HIDDEN.swap(true, Ordering::SeqCst) {
        allow_background_cursor_control(); // 后台进程也能控光标（关键！不设则 hide 静默无效）
        unsafe {
            let disp = CGMainDisplayID();
            let err = CGDisplayHideCursor(disp);
            log::info!("[MAC-CURSOR] hide_cursor: CGDisplayHideCursor err={err}");
        }
    }
}

/// 恢复显示本机光标（幂等：仅 true→false 时调一次 CGDisplayShowCursor，
/// 与 hide_cursor 严格配对一次）。
pub fn show_cursor() {
    // 兼容从旧版本升级：旧进程若在断开光标关联时异常退出，首次启动时恢复一次。
    if !CURSOR_STARTUP_REASSOCIATED.swap(true, Ordering::SeqCst) {
        allow_background_cursor_control();
        let _ = unsafe { CGAssociateMouseAndMouseCursorPosition(1) };
    }
    if CURSOR_HIDDEN.swap(false, Ordering::SeqCst) {
        allow_background_cursor_control(); // show 同样要求前台，先开后台权限
        unsafe {
            let disp = CGMainDisplayID();
            let err = CGDisplayShowCursor(disp);
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
    let hid_tx = tx.clone();
    // SAFETY：两个 tap 及其回调都只安装在当前捕获线程的 runloop，且在
    // CFRunLoop::run_current 返回前始终存活。
    let hid_tap = match unsafe {
        CGEventTap::new_unchecked(
            // 移动、点击和键盘在进入 WindowServer 前截获；滚动留给 Session tap，
            // 以覆盖 WindowServer 合成的惯性段。
            CGEventTapLocation::HID,
            CGEventTapPlacement::HeadInsertEventTap,
            CGEventTapOptions::Default,
            vec![
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
                CGEventType::KeyDown,
                CGEventType::KeyUp,
                CGEventType::FlagsChanged,
            ],
            move |_proxy, event_type, event| {
                if matches!(
                    event_type,
                    CGEventType::TapDisabledByTimeout | CGEventType::TapDisabledByUserInput
                ) {
                    let tap_ptr = HID_TAP_PORT_PTR.load(Ordering::SeqCst) as *const c_void;
                    if !tap_ptr.is_null() {
                        unsafe { CGEventTapEnable(tap_ptr, true) };
                    }
                    log::warn!(
                        "[MAC-TAP] HID tap 被系统禁用，已自动重新启用: {event_type:?}"
                    );
                    return CallbackResult::Drop;
                }

                let is_move = matches!(
                    event_type,
                    CGEventType::MouseMoved
                        | CGEventType::LeftMouseDragged
                        | CGEventType::RightMouseDragged
                        | CGEventType::OtherMouseDragged
                );
                let blocked = BLOCK_LOCAL_INPUT.load(Ordering::SeqCst);
                let sink = SINK_ACTIVE.load(Ordering::SeqCst);
                let marked = event
                    .get_integer_value_field(EventField::EVENT_SOURCE_USER_DATA)
                    == RUISS_EVENT_MARKER;

                if marked {
                    // Sink 必须放行对端注入；Source 只允许入口/返回 warp 的移动通过。
                    return if blocked && !is_move {
                        CallbackResult::Drop
                    } else {
                        CallbackResult::Keep
                    };
                }

                if sink && is_move {
                    // Sink 本机鼠标不移动系统指针，但保留虚拟绝对位置用于本机夺回。
                    let dx = event.get_double_value_field(EventField::MOUSE_EVENT_DELTA_X);
                    let dy = event.get_double_value_field(EventField::MOUSE_EVENT_DELTA_Y);
                    let (vx, vy) = {
                        let mut pos = match LOCAL_VIRTUAL_POS.lock() {
                            Ok(g) => g,
                            Err(p) => p.into_inner(),
                        };
                        let (x, y) = (*pos).unwrap_or_else(|| {
                            let loc = event.location();
                            (loc.x, loc.y)
                        });
                        let next = (x + dx, y + dy);
                        *pos = Some(next);
                        next
                    };
                    let (w, h) = screen_size();
                    let _ = hid_tx.send(Payload::MouseMove {
                        x: vx as i32,
                        y: vy as i32,
                        src_w: w as u32,
                        src_h: h as u32,
                    });
                    return CallbackResult::Drop;
                }

                if blocked && is_move {
                    // Source 直接转发 HID 相对 delta；不再虚构绝对坐标或逐帧 warp。
                    let dx = event.get_integer_value_field(EventField::MOUSE_EVENT_DELTA_X) as i32;
                    let dy = event.get_integer_value_field(EventField::MOUSE_EVENT_DELTA_Y) as i32;
                    if dx != 0 || dy != 0 {
                        let _ = hid_tx.send(Payload::MouseMoveRelative { dx, dy });
                    }
                    return CallbackResult::Drop;
                }

                if blocked {
                    // Source 的点击/键盘只发给对端。
                    if let Some(payload) = event_to_payload(event_type, event) {
                        let _ = hid_tx.send(payload);
                    }
                    return CallbackResult::Drop;
                }
                if sink {
                    // Sink 的本机点击/键盘不作用于任何一端。
                    return CallbackResult::Drop;
                }

                // 正常状态只有移动在 HID 层发送；点击/键盘交给 Session fallback，
                // 防止同一个事件被发送两次。
                if is_move {
                    if let Some(payload) = event_to_payload(event_type, event) {
                        let _ = hid_tx.send(payload);
                    }
                }
                CallbackResult::Keep
            },
        )
    } {
        Ok(t) => t,
        Err(_) => {
            log::error!("HID CGEventTap 创建失败");
            let _ = ready.send(Err(anyhow!("HID CGEventTap 创建失败")));
            return;
        }
    };

    let event_types = vec![
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
    // SAFETY：同上，Session tap 只由当前线程的 runloop 调用，生命周期覆盖 runloop。
    let tap = match unsafe {
        CGEventTap::new_unchecked(
            // 捕获层：Session（WindowServer 对外派发层），不用 HID。关键：触控板惯性
            // 滚动（momentum 段）由 WindowServer 内部合成派发，HID 层 Drop
            // 拦不住（实测 Mac 应用照样滚，见 2026-08-11 实验 6618b71），只有 Session
            // 层才能拦截——跨屏双滚根因就在这。
            CGEventTapLocation::Session,
            CGEventTapPlacement::HeadInsertEventTap,
            CGEventTapOptions::Default,
            event_types,
            move |_proxy, event_type, event| {
            // macOS 会在 tap 超时或用户请求后禁用它；不重新启用时，本机输入将完全
            // 绕过 Ruiss。特殊通知不属于用户输入，恢复 tap 后直接丢弃通知本身。
            if matches!(
                event_type,
                CGEventType::TapDisabledByTimeout | CGEventType::TapDisabledByUserInput
            ) {
                let tap_ptr = SESSION_TAP_PORT_PTR.load(Ordering::SeqCst) as *const c_void;
                if !tap_ptr.is_null() {
                    unsafe { CGEventTapEnable(tap_ptr, true) };
                }
                log::warn!("[MAC-TAP] 事件 tap 被系统禁用，已自动重新启用: {event_type:?}");
                return CallbackResult::Drop;
            }

            // 防回环只认显式 marker，不再猜 source pid。对端输入及本机 warp 都会在
            // 注入前写入该标记；物理触控板和 WindowServer 惯性事件不会携带它。
            let marker = event.get_integer_value_field(EventField::EVENT_SOURCE_USER_DATA);
            if marker == RUISS_EVENT_MARKER {
                // Source 不应接收远程按键/滚动；即便合成事件意外继承 marker，也不能
                // 绕过本机隔离。Sink 上的对端注入则必须放行。
                return if BLOCK_LOCAL_INPUT.load(Ordering::SeqCst) {
                    CallbackResult::Drop
                } else {
                    CallbackResult::Keep
                };
            }
            if let Some(p) = event_to_payload(event_type, event) {
                let _ = tx.send(p);
            }

            // 输入所有权判断必须基于原始 CGEventType，不能依赖 event_to_payload 是否
            // 认识/成功转换该事件。跨屏期间所有真实键盘、按键和滚动都只允许发往
            // 对端，本机一律吞掉；移动已在上面的 Source/Sink 分支单独处理。
            let owns_remote = BLOCK_LOCAL_INPUT.load(Ordering::Relaxed)
                || SINK_ACTIVE.load(Ordering::Relaxed);
            if owns_remote
                && matches!(
                    event_type,
                    CGEventType::ScrollWheel
                        | CGEventType::KeyDown
                        | CGEventType::KeyUp
                        | CGEventType::FlagsChanged
                        | CGEventType::LeftMouseDown
                        | CGEventType::LeftMouseUp
                        | CGEventType::RightMouseDown
                        | CGEventType::RightMouseUp
                        | CGEventType::OtherMouseDown
                        | CGEventType::OtherMouseUp
                )
            {
                return CallbackResult::Drop;
            }
            // 监听模式：原样放行，绝不吞事件
                CallbackResult::Keep
            },
        )
    } {
        Ok(t) => t,
        Err(_) => {
            log::error!("CGEventTap 创建失败");
            let _ = ready.send(Err(anyhow!("CGEventTap 创建失败")));
            return;
        }
    };

    // 两个 tap 通过 mach_port 创建 runloop source 并加入同一个捕获线程 runloop。
    let runloop = CFRunLoop::get_current();
    let hid_loop_source = hid_tap
        .mach_port()
        .create_runloop_source(0)
        .expect("HID create_runloop_source 失败");
    let loop_source = tap
        .mach_port()
        .create_runloop_source(0)
        .expect("create_runloop_source 失败");
    // kCFRunLoopCommonModes 是 extern static，读取需 unsafe
    unsafe {
        runloop.add_source(&hid_loop_source, kCFRunLoopCommonModes);
        runloop.add_source(&loop_source, kCFRunLoopCommonModes);
    }
    HID_TAP_PORT_PTR.store(
        hid_tap.mach_port().as_concrete_TypeRef() as usize,
        Ordering::SeqCst,
    );
    SESSION_TAP_PORT_PTR.store(
        tap.mach_port().as_concrete_TypeRef() as usize,
        Ordering::SeqCst,
    );
    hid_tap.enable();
    tap.enable();
    RUNLOOP_PTR.store(runloop.as_concrete_TypeRef() as usize, Ordering::Relaxed);

    let _ = ready.send(Ok(()));
    log::info!(
        "[MAC-TAP] CGEventTap 已启动 input=HID scroll/fallback=Session marker={RUISS_EVENT_MARKER:#x}"
    );
    CFRunLoop::run_current();

    // runloop 退出：清理
    HID_TAP_PORT_PTR.store(0, Ordering::SeqCst);
    SESSION_TAP_PORT_PTR.store(0, Ordering::SeqCst);
    RUNLOOP_PTR.store(0, Ordering::Relaxed);
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
        Key::CapsLock => f.contains(CGEventFlags::CGEventFlagAlphaShift),
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

#[derive(Default)]
struct KeyboardState {
    /// 收到的 Windows 源端修饰键状态。
    source_modifiers: ModifierState,
    /// 已经在 macOS 目标端同步的修饰键状态。
    applied_modifiers: ModifierState,
    /// key-up 沿用 key-down 时的翻译，防止修饰键提前释放导致按键卡住。
    active: HashMap<Key, ShortcutStroke>,
}

impl KeyboardState {
    fn prepare_key(&mut self, key: Key, down: bool) -> ShortcutStroke {
        if down {
            let stroke = translate_windows_shortcut_to_mac(self.source_modifiers, key);
            self.active.insert(key, stroke);
            stroke
        } else {
            self.active
                .remove(&key)
                .unwrap_or_else(|| translate_windows_shortcut_to_mac(self.source_modifiers, key))
        }
    }
}

fn mac_event_flags(modifiers: ModifierState) -> CGEventFlags {
    let mut flags = CGEventFlags::empty();
    if modifiers.ctrl {
        flags |= CGEventFlags::CGEventFlagControl;
    }
    if modifiers.alt {
        flags |= CGEventFlags::CGEventFlagAlternate;
    }
    if modifiers.shift {
        flags |= CGEventFlags::CGEventFlagShift;
    }
    if modifiers.super_key {
        flags |= CGEventFlags::CGEventFlagCommand;
    }
    flags
}

fn post_mac_modifier(key: Key, down: bool, modifiers: ModifierState) {
    let Ok(source) = CGEventSource::new(CGEventSourceStateID::CombinedSessionState) else {
        return;
    };
    let Ok(event) = CGEvent::new_keyboard_event(source, key_to_cg(key), down) else {
        return;
    };
    // macOS represents modifier transitions as FlagsChanged, not ordinary KeyDown/KeyUp.
    // Application shortcuts may work from flags alone, while system shortcuts such as
    // Mission Control require the modifier state transition to have the correct type.
    event.set_type(CGEventType::FlagsChanged);
    event.set_flags(mac_event_flags(modifiers));
    event.set_integer_value_field(EventField::EVENT_SOURCE_USER_DATA, RUISS_EVENT_MARKER);
    event.post(CGEventTapLocation::HID);
}

fn sync_mac_modifiers(keyboard: &mut KeyboardState, desired: ModifierState) {
    let current = keyboard.applied_modifiers;
    let mut next = current;
    for (key, was_down, should_down) in [
        (Key::Super, current.super_key, desired.super_key),
        (Key::Alt, current.alt, desired.alt),
        (Key::Ctrl, current.ctrl, desired.ctrl),
        (Key::Shift, current.shift, desired.shift),
    ] {
        if was_down && !should_down {
            next.set(key, false);
            post_mac_modifier(key, false, next);
        }
    }
    for (key, was_down, should_down) in [
        (Key::Ctrl, current.ctrl, desired.ctrl),
        (Key::Alt, current.alt, desired.alt),
        (Key::Shift, current.shift, desired.shift),
        (Key::Super, current.super_key, desired.super_key),
    ] {
        if !was_down && should_down {
            next.set(key, true);
            post_mac_modifier(key, true, next);
        }
    }
    keyboard.applied_modifiers = desired;
}

/// 注入器：CGEventPost 回放事件。
pub struct InputInjector {
    /// 点击计数状态（跨线程：注入可能在消费线程/主线程被调用）。
    click: Mutex<ClickState>,
    keyboard: Mutex<KeyboardState>,
}

impl InputInjector {
    pub fn new() -> Self {
        Self {
            click: Mutex::new(ClickState::new()),
            keyboard: Mutex::new(KeyboardState::default()),
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
        if matches!(event, Payload::MouseButton { .. } | Payload::MouseWheel { .. }) {
            let mut keyboard = match self.keyboard.lock() {
                Ok(g) => g,
                Err(p) => p.into_inner(),
            };
            let desired = map_modifiers(true, keyboard.source_modifiers);
            sync_mac_modifiers(&mut keyboard, desired);
        }
        let mut event_flags = None;
        let mut restore_modifiers = None;
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
            Payload::MouseMoveRelative { dx, dy } => {
                let (w, h) = screen_size();
                let (x, y) = {
                    let mut pos = match LAST_POS.lock() {
                        Ok(g) => g,
                        Err(p) => p.into_inner(),
                    };
                    let (x, y) = (*pos).unwrap_or((0.0, 0.0));
                    let next = (
                        (x + *dx as f64).clamp(0.0, (w - 1).max(0) as f64),
                        (y + *dy as f64).clamp(0.0, (h - 1).max(0) as f64),
                    );
                    *pos = Some(next);
                    next
                };
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
                CGEvent::new_mouse_event(source, ty, CGPoint::new(x, y), btn)
            }
            Payload::MouseButton { button, down } => {
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
                let stroke = {
                    let mut keyboard = match self.keyboard.lock() {
                        Ok(g) => g,
                        Err(p) => p.into_inner(),
                    };
                    if keyboard.source_modifiers.set(*key, *down) {
                        let desired = map_modifiers(true, keyboard.source_modifiers);
                        sync_mac_modifiers(&mut keyboard, desired);
                        return 1;
                    }
                    let stroke = keyboard.prepare_key(*key, *down);
                    if *down {
                        let generic = ShortcutStroke {
                            key: *key,
                            modifiers: map_modifiers(true, keyboard.source_modifiers),
                        };
                        if stroke != generic {
                            log::info!(
                                "[MAC-KEY] shortcut source={:?}+{:?} -> target={:?}+{:?}",
                                keyboard.source_modifiers,
                                key,
                                stroke.modifiers,
                                stroke.key,
                            );
                        }
                    }
                    sync_mac_modifiers(&mut keyboard, stroke.modifiers);
                    if !*down
                        && translate_windows_shortcut_to_mac(
                            keyboard.source_modifiers,
                            *key,
                        ) != stroke
                    {
                        restore_modifiers = Some(map_modifiers(
                            true,
                            keyboard.source_modifiers,
                        ));
                    }
                    stroke
                };
                event_flags = Some(mac_event_flags(stroke.modifiers));
                CGEvent::new_keyboard_event(source, key_to_cg(stroke.key), *down)
            }
            other => {
                log::debug!("注入跳过（非输入事件）: {other:?}");
                return 0;
            }
        };
        let Ok(cg) = cg else { return 0 };
        let flags = event_flags.unwrap_or_else(|| {
            let keyboard = match self.keyboard.lock() {
                Ok(g) => g,
                Err(p) => p.into_inner(),
            };
            mac_event_flags(keyboard.applied_modifiers)
        });
        cg.set_flags(flags);
        cg.set_integer_value_field(EventField::EVENT_SOURCE_USER_DATA, RUISS_EVENT_MARKER);
        cg.post(CGEventTapLocation::HID);
        if let Some(desired) = restore_modifiers {
            let mut keyboard = match self.keyboard.lock() {
                Ok(g) => g,
                Err(p) => p.into_inner(),
            };
            sync_mac_modifiers(&mut keyboard, desired);
        }
        1
    }

    /// Release remote keys/buttons that might still be down after a handoff or disconnect.
    pub fn reset_keyboard_state(&self) {
        let active_keys = {
            let mut keyboard = match self.keyboard.lock() {
                Ok(g) => g,
                Err(p) => p.into_inner(),
            };
            let keys = keyboard
                .active
                .values()
                .map(|stroke| stroke.key)
                .collect::<Vec<_>>();
            sync_mac_modifiers(&mut keyboard, ModifierState::default());
            *keyboard = KeyboardState::default();
            keys
        };
        let held_button = HELD_BUTTON.swap(-1, Ordering::Relaxed);
        if (0..=2).contains(&held_button) {
            let (event_type, button) = match held_button {
                0 => (CGEventType::LeftMouseUp, CGMouseButton::Left),
                1 => (CGEventType::RightMouseUp, CGMouseButton::Right),
                _ => (CGEventType::OtherMouseUp, CGMouseButton::Center),
            };
            let (x, y) = LAST_POS
                .lock()
                .map(|pos| pos.unwrap_or((0.0, 0.0)))
                .unwrap_or((0.0, 0.0));
            if let Ok(source) = CGEventSource::new(CGEventSourceStateID::CombinedSessionState) {
                if let Ok(event) = CGEvent::new_mouse_event(
                    source,
                    event_type,
                    CGPoint::new(x, y),
                    button,
                ) {
                    event.set_flags(CGEventFlags::empty());
                    event.set_integer_value_field(
                        EventField::EVENT_SOURCE_USER_DATA,
                        RUISS_EVENT_MARKER,
                    );
                    event.post(CGEventTapLocation::HID);
                }
            }
        }

        for key in active_keys {
            let Ok(source) = CGEventSource::new(CGEventSourceStateID::CombinedSessionState) else {
                continue;
            };
            let Ok(event) = CGEvent::new_keyboard_event(source, key_to_cg(key), false) else {
                continue;
            };
            event.set_flags(CGEventFlags::empty());
            event.set_integer_value_field(EventField::EVENT_SOURCE_USER_DATA, RUISS_EVENT_MARKER);
            event.post(CGEventTapLocation::HID);
        }
        for key in [Key::Shift, Key::Ctrl, Key::Alt, Key::Super] {
            post_mac_modifier(key, false, ModifierState::default());
        }
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

/// 光标跳转（跨屏入口/返回定位用）。
pub fn warp_cursor(x: i32, y: i32) {
    if let Ok(source) = CGEventSource::new(CGEventSourceStateID::CombinedSessionState) {
        if let Ok(ev) = CGEvent::new_mouse_event(
            source,
            CGEventType::MouseMoved,
            CGPoint::new(x as f64, y as f64),
            CGMouseButton::Left,
        ) {
            ev.set_integer_value_field(EventField::EVENT_SOURCE_USER_DATA, RUISS_EVENT_MARKER);
            ev.post(CGEventTapLocation::HID);
        }
    }
}

/// 跨屏触发时的光标回绕（Action::Warp 专用）：与 warp_cursor 相同，但落点
/// y 避开屏幕底部 Dock / 顶部菜单栏热区——隐藏光标停进热区会被 macOS 强制重显
/// 并触发悬停动画。后续移动直接走 HID 相对 delta，不再维护 Source 虚拟坐标，
/// 也不再逐帧把系统指针 warp 回来。
pub fn warp_cursor_cross(x: i32, y: i32) {
    let (_, h) = screen_size();
    let ty = if h > MENUBAR_HOT_ZONE + DOCK_HOT_ZONE + 1 {
        y.clamp(MENUBAR_HOT_ZONE, h - 1 - DOCK_HOT_ZONE)
    } else {
        y
    };
    warp_cursor(x, ty);
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

// ======================== M3：剪贴板 + 拖拽检测 ========================
//
// 注意：本段为 Mac 实现，Windows 上不编译（cfg gate）。API 用 objc 0.2 调
// NSPasteboard / NSEvent。CFString 与 NSString toll-free bridged，用作类型参数。
// 首次 Mac 编译若有 API 签名差异，把错误信息发回迭代。

use crate::platform::{ClipboardContent, ClipboardWatcherHandle};
use objc::runtime::{Class, Object, NO, YES};
use objc::{class, msg_send};

use core_foundation::base::CFTypeRef;
use core_foundation::string::CFStringRef;

/// 本机写入标志：本机主动写剪贴板时置 true，监听器跳过本次变化（防回环）。
static LOCAL_WRITE: AtomicBool = AtomicBool::new(false);

/// 取全局 NSPasteboard。
unsafe fn general_pasteboard() -> *mut Object {
    msg_send![class!(NSPasteboard), generalPasteboard]
}

/// CFString → NSString（toll-free bridge）作为 NSPasteboard 类型参数。
unsafe fn ns_type(name: &str) -> *mut Object {
    let cf = CFString::new(name);
    cf.as_concrete_TypeRef() as *mut Object
}

/// NSString → Rust String。
unsafe fn nsstring_to_string(nsstr: *mut Object) -> Option<String> {
    if nsstr.is_null() {
        return None;
    }
    let utf8: *const std::os::raw::c_char = msg_send![nsstr, UTF8String];
    if utf8.is_null() {
        return None;
    }
    std::ffi::CStr::from_ptr(utf8)
        .to_str()
        .ok()
        .map(|s| s.to_string())
}

/// NSData（字节）→ Vec<u8>。
unsafe fn nsdata_to_vec(data: *mut Object) -> Option<Vec<u8>> {
    if data.is_null() {
        return None;
    }
    let len: usize = msg_send![data, length];
    if len == 0 {
        return Some(Vec::new());
    }
    let bytes: *const c_void = msg_send![data, bytes];
    let slice = std::slice::from_raw_parts(bytes as *const u8, len);
    Some(slice.to_vec())
}

/// Rust String → autoreleased NSString。
unsafe fn string_to_nsstring(s: &str) -> *mut Object {
    let bytes = s.as_bytes();
    let ns: *mut Object = msg_send![class!(NSString), alloc];
    msg_send![ns, initWithBytes: bytes.as_ptr() length: bytes.len() encoding: 4 /* NSUTF8StringEncoding */]
}

/// 读当前剪贴板（优先级 files > image > text）。
pub fn clipboard_read() -> ClipboardContent {
    unsafe {
        let pb = general_pasteboard();
        if pb.is_null() {
            return ClipboardContent::Empty;
        }
        // 文件优先
        if let Some(files) = read_files(pb) {
            if !files.is_empty() {
                return ClipboardContent::Files(files);
            }
        }
        // 图片：PNG 优先，否则 TIFF→PNG
        if let Some(png) = read_image(pb) {
            return ClipboardContent::Image(png);
        }
        // 文本
        if let Some(text) = read_text(pb) {
            return ClipboardContent::Text(text);
        }
        ClipboardContent::Empty
    }
}

unsafe fn read_text(pb: *mut Object) -> Option<String> {
    let t = ns_type("public.utf8-plain-text");
    let nsstr: *mut Object = msg_send![pb, stringForType: t];
    nsstring_to_string(nsstr)
}

unsafe fn read_image(pb: *mut Object) -> Option<Vec<u8>> {
    // 1. PNG 直接取
    let t = ns_type("public.png");
    let data: *mut Object = msg_send![pb, dataForType: t];
    if !data.is_null() {
        if let Some(v) = nsdata_to_vec(data) {
            return Some(v);
        }
    }
    // 2. TIFF → PNG（截图默认 TIFF）
    let t = ns_type("public.tiff");
    let tiff: *mut Object = msg_send![pb, dataForType: t];
    if tiff.is_null() {
        return None;
    }
    let tiff_bytes = nsdata_to_vec(tiff)?;
    tiff_to_png(&tiff_bytes)
}

/// TIFF 字节 → PNG 字节（用 NSBitmapImageRep）。
fn tiff_to_png(tiff: &[u8]) -> Option<Vec<u8>> {
    unsafe {
        let nsdata: *mut Object = msg_send![
            class!(NSData),
            dataWithBytes: tiff.as_ptr()
            length: tiff.len()
        ];
        if nsdata.is_null() {
            return None;
        }
        let rep: *mut Object = msg_send![class!(NSBitmapImageRep), imageRepWithData: nsdata];
        if rep.is_null() {
            return None;
        }
        let props: *mut Object = msg_send![class!(NSDictionary), dictionary];
        // NSBitmapImageRep representationUsingType:properties: 4 = NSPNGFileType
        let png: *mut Object = msg_send![rep, representationUsingType: 4 properties: props];
        nsdata_to_vec(png)
    }
}

unsafe fn read_files(pb: *mut Object) -> Option<Vec<String>> {
    let classes: *mut Object = msg_send![class!(NSArray), arrayWithObject: class!(NSURL)];
    let options: *mut Object = msg_send![class!(NSDictionary), dictionary];
    let urls: *mut Object = msg_send![pb, readObjectsForClasses: classes options: options];
    if urls.is_null() {
        return Some(Vec::new());
    }
    let count: usize = msg_send![urls, count];
    let mut files = Vec::with_capacity(count);
    for i in 0..count {
        let url: *mut Object = msg_send![urls, objectAtIndex: i];
        let path: *mut Object = msg_send![url, path];
        if let Some(s) = nsstring_to_string(path) {
            files.push(s);
        }
    }
    Some(files)
}

/// 写文本到剪贴板。
pub fn clipboard_write_text(text: &str) {
    LOCAL_WRITE.store(true, std::sync::atomic::Ordering::Relaxed);
    unsafe {
        let pb = general_pasteboard();
        let _: () = msg_send![pb, clearContents];
        let nsstr = string_to_nsstring(text);
        let t = ns_type("public.utf8-plain-text");
        let _: () = msg_send![pb, setString: nsstr forType: t];
    }
}

/// 写 PNG 图片到剪贴板（同时写 PNG 和 TIFF 类型，兼容性最好）。
pub fn clipboard_write_image(png_bytes: &[u8]) {
    LOCAL_WRITE.store(true, std::sync::atomic::Ordering::Relaxed);
    unsafe {
        let pb = general_pasteboard();
        let _: () = msg_send![pb, clearContents];
        let data: *mut Object = msg_send![
            class!(NSData),
            dataWithBytes: png_bytes.as_ptr()
            length: png_bytes.len()
        ];
        let t = ns_type("public.png");
        let _: () = msg_send![pb, setData: data forType: t];
        // 同时写 TIFF（部分应用只认 TIFF）
        if let Some(tiff) = png_to_tiff(png_bytes) {
            let t2 = ns_type("public.tiff");
            let tdata: *mut Object = msg_send![
                class!(NSData),
                dataWithBytes: tiff.as_ptr()
                length: tiff.len()
            ];
            let _: () = msg_send![pb, setData: tdata forType: t2];
        }
    }
}

/// PNG → TIFF（NSBitmapImageRep 中转）。
fn png_to_tiff(png: &[u8]) -> Option<Vec<u8>> {
    unsafe {
        let nsdata: *mut Object = msg_send![
            class!(NSData),
            dataWithBytes: png.as_ptr()
            length: png.len()
        ];
        let rep: *mut Object = msg_send![class!(NSBitmapImageRep), imageRepWithData: nsdata];
        if rep.is_null() {
            return None;
        }
        let props: *mut Object = msg_send![class!(NSDictionary), dictionary];
        // NSTIFFFileType = 0
        let tiff: *mut Object = msg_send![rep, representationUsingType: 0 properties: props];
        nsdata_to_vec(tiff)
    }
}

/// 写文件路径列表到剪贴板（NSURL 数组 writeObjects）。
pub fn clipboard_write_files(paths: &[String]) {
    if paths.is_empty() {
        return;
    }
    LOCAL_WRITE.store(true, std::sync::atomic::Ordering::Relaxed);
    unsafe {
        let pb = general_pasteboard();
        let _: () = msg_send![pb, clearContents];
        // 构造 NSURL 数组
        let n = paths.len();
        let mut urls: Vec<*mut Object> = Vec::with_capacity(n);
        for p in paths {
            let path_ns = string_to_nsstring(p);
            let url: *mut Object = msg_send![
                class!(NSURL),
                fileURLWithPath: path_ns
            ];
            urls.push(url);
        }
        let arr: *mut Object = msg_send![
            class!(NSArray),
            arrayWithObjects: urls.as_ptr()
            count: n
        ];
        let _: bool = msg_send![pb, writeObjects: arr];
    }
}

/// 鼠标左键是否按下（拖拽跨屏检测用）。NSEvent pressedMouseButtons 位掩码 bit0=左键。
pub fn is_left_button_down() -> bool {
    unsafe {
        let pressed: isize = msg_send![class!(NSEvent), pressedMouseButtons];
        (pressed & 1) != 0
    }
}

/// 启动剪贴板监听：1s 轮询 NSPasteboard changeCount，变化时读 + 回调。
/// Mac 没有像 Windows AddClipboardFormatListener 那样的通知机制，轮询最稳。
pub fn start_clipboard_watcher(cb: Box<dyn Fn(ClipboardContent) + Send + 'static>) -> ClipboardWatcherHandle {
    use std::sync::atomic::AtomicBool;
    use std::sync::Arc;
    let stop = Arc::new(AtomicBool::new(false));
    let stop_clone = stop.clone();
    std::thread::spawn(move || unsafe {
        let pb = general_pasteboard();
        let mut last: isize = msg_send![pb, changeCount];
        while !stop_clone.load(std::sync::atomic::Ordering::Relaxed) {
            std::thread::sleep(std::time::Duration::from_millis(800));
            let cur: isize = msg_send![pb, changeCount];
            if cur == last {
                continue;
            }
            last = cur;
            // 本机写入触发跳过（防回环）
            if LOCAL_WRITE
                .compare_exchange(true, false, std::sync::atomic::Ordering::Relaxed, std::sync::atomic::Ordering::Relaxed)
                .is_ok()
            {
                continue;
            }
            let content = clipboard_read();
            if !content.is_empty() {
                cb(content);
            }
        }
    });
    ClipboardWatcherHandle {
        stop: Some(Box::new(move || {
            stop.store(true, std::sync::atomic::Ordering::Relaxed);
        })),
    }
}
