// 输入探针（M1/M2 开发工具，非产品代码）。
//
// 两种用法：
//   1. 独立验证注入：cargo run --example input_probe full
//      - 注入 'a' 键并用 GetAsyncKeyState 校验按键状态真的变了
//      - 注入鼠标左键点击（光标处）、相对移动、滚轮
//   2. 验证 Ruiss 捕获链路：Ruiss 以 RUISS_NO_SUPPRESS=1 运行时，
//      cargo run --example input_probe move 注入一次相对移动，
//      Ruiss 日志里应出现 "捕获: MouseMove {...}"。
//
// 注意：full 模式会把 'a' 打到你当前聚焦的窗口、在光标处点击一次，请先切到安全窗口。

use std::env;
use std::mem::size_of;
use std::thread;
use std::time::Duration;

use windows::Win32::Foundation::{LPARAM, LRESULT, WPARAM};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    GetAsyncKeyState, INPUT, INPUT_0, INPUT_KEYBOARD, INPUT_MOUSE, KEYBDINPUT, KEYBD_EVENT_FLAGS,
    KEYEVENTF_KEYUP, MOUSEINPUT, MOUSEEVENTF_ABSOLUTE, MOUSEEVENTF_LEFTDOWN, MOUSEEVENTF_LEFTUP,
    MOUSEEVENTF_MOVE, MOUSEEVENTF_WHEEL, SendInput, VIRTUAL_KEY,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CallNextHookEx, DispatchMessageW, KBDLLHOOKSTRUCT, MSLLHOOKSTRUCT, PeekMessageW,
    SetWindowsHookExW, TranslateMessage, UnhookWindowsHookEx, WH_KEYBOARD_LL, WH_MOUSE_LL,
    WM_QUIT, MSG, PM_REMOVE,
};

fn send(i: INPUT) -> u32 {
    unsafe { SendInput(&[i], size_of::<INPUT>() as i32) }
}

fn key(vk: u16, up: bool) -> u32 {
    send(INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT {
                wVk: VIRTUAL_KEY(vk),
                wScan: 0,
                time: 0,
                dwFlags: if up { KEYEVENTF_KEYUP } else { KEYBD_EVENT_FLAGS(0) },
                dwExtraInfo: 0,
            },
        },
    })
}

fn mouse(flags: windows::Win32::UI::Input::KeyboardAndMouse::MOUSE_EVENT_FLAGS, mouse_data: u32) -> u32 {
    send(INPUT {
        r#type: INPUT_MOUSE,
        Anonymous: INPUT_0 {
            mi: MOUSEINPUT {
                dx: 0,
                dy: 0,
                mouseData: mouse_data,
                dwFlags: flags,
                time: 0,
                dwExtraInfo: 0,
            },
        },
    })
}

fn main() {
    let args: Vec<String> = env::args().collect();
    let mode = args.get(1).map(|s| s.as_str()).unwrap_or("move");

    match mode {
        "move" => {
            // 绝对移动（可靠产生事件）；配合 RUISS_NO_SUPPRESS=1 的 Ruiss 验证捕获链路
            println!("== 探针: 注入绝对移动到 (300,300)，共 3 次 ==");
            for i in 0..3 {
                let mi = INPUT {
                    r#type: INPUT_MOUSE,
                    Anonymous: INPUT_0 {
                        mi: MOUSEINPUT {
                            dx: (300.0 * 65535.0 / 1920.0) as i32,
                            dy: (300.0 * 65535.0 / 1080.0) as i32,
                            mouseData: 0,
                            dwFlags: MOUSEEVENTF_MOVE | MOUSEEVENTF_ABSOLUTE,
                            time: 0,
                            dwExtraInfo: 0,
                        },
                    },
                };
                println!("absmove({i}) 返回 {}", send(mi));
                thread::sleep(Duration::from_millis(200));
            }
        }
        "hook" => {
            // 决定性实验：LL 钩子能不能看到注入的事件？（键盘 + 绝对坐标移动）
            println!("== 探针 hook: 装 WH_MOUSE_LL + WH_KEYBOARD_LL，注入按键和移动 ==");
            thread_local! {
                static SEEN: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
            }
            unsafe extern "system" fn mouse_hook(code: i32, w: WPARAM, l: LPARAM) -> LRESULT {
                if code >= 0 {
                    let ms = &*(l.0 as *const MSLLHOOKSTRUCT);
                    println!(
                        "MOUSE-HOOK: wparam={:#x} flags={:#x} pt=({},{})",
                        w.0, ms.flags, ms.pt.x, ms.pt.y
                    );
                    SEEN.with(|s| s.set(true));
                }
                CallNextHookEx(None, code, w, l)
            }
            unsafe extern "system" fn key_hook(code: i32, w: WPARAM, l: LPARAM) -> LRESULT {
                if code >= 0 {
                    let ks = &*(l.0 as *const KBDLLHOOKSTRUCT);
                    println!(
                        "KEY-HOOK: wparam={:#x} vk={:#x} scan={:#x} flags={:#x}",
                        w.0, ks.vkCode, ks.scanCode, ks.flags.0
                    );
                    SEEN.with(|s| s.set(true));
                }
                CallNextHookEx(None, code, w, l)
            }
            let h_m = unsafe { SetWindowsHookExW(WH_MOUSE_LL, Some(mouse_hook), None, 0) }
                .expect("装鼠标钩子失败");
            let h_k = unsafe { SetWindowsHookExW(WH_KEYBOARD_LL, Some(key_hook), None, 0) }
                .expect("装键盘钩子失败");
            let mut msg = MSG::default();
            let mut injected = false;
            let start = std::time::Instant::now();
            loop {
                let elapsed = start.elapsed().as_millis() as i64;
                if elapsed > 2500 {
                    break;
                }
                if !injected && elapsed > 300 {
                    println!("注入: 绝对移动(200,200) + 'a' 按下/松开...");
                    let mi = INPUT {
                        r#type: INPUT_MOUSE,
                        Anonymous: INPUT_0 {
                            mi: MOUSEINPUT {
                                dx: (200.0 * 65535.0 / 1920.0) as i32,
                                dy: (200.0 * 65535.0 / 1080.0) as i32,
                                mouseData: 0,
                                dwFlags: MOUSEEVENTF_MOVE | MOUSEEVENTF_ABSOLUTE,
                                time: 0,
                                dwExtraInfo: 0,
                            },
                        },
                    };
                    println!("absmove 返回 {}", send(mi));
                    println!("keydown 返回 {}", key(0x41, false));
                    println!("keyup  返回 {}", key(0x41, true));
                    injected = true;
                }
                if unsafe { PeekMessageW(&mut msg, None, 0, 0, PM_REMOVE) }.as_bool() {
                    if msg.message == WM_QUIT {
                        break;
                    }
                    unsafe {
                        let _ = TranslateMessage(&msg);
                        DispatchMessageW(&msg);
                    }
                } else {
                    thread::sleep(Duration::from_millis(5));
                }
            }
            let seen = SEEN.with(|s| s.get());
            println!("钩子是否看到注入事件: {seen}");
            let _ = unsafe {
                UnhookWindowsHookEx(h_k);
                UnhookWindowsHookEx(h_m);
            };
        }
        "full" => {
            println!("== 探针 full: 按键 / 点击 / 移动 / 滚轮 ==");
            println!("注入 'a' 按下... 返回 {}", key(0x41, false));
            thread::sleep(Duration::from_millis(80));
            let down = (unsafe { GetAsyncKeyState(0x41) } as u16 & 0x8000) != 0;
            println!("GetAsyncKeyState('a') 应为按下中: {down}");
            println!("注入 'a' 松开... 返回 {}", key(0x41, true));
            thread::sleep(Duration::from_millis(80));
            let down = (unsafe { GetAsyncKeyState(0x41) } as u16 & 0x8000) != 0;
            println!("GetAsyncKeyState('a') 应已松开: {down}");

            println!("注入左键按下/松开（光标处）...");
            println!("leftdown 返回 {}", mouse(MOUSEEVENTF_LEFTDOWN, 0));
            thread::sleep(Duration::from_millis(30));
            println!("leftup  返回 {}", mouse(MOUSEEVENTF_LEFTUP, 0));

            println!("注入滚轮 -1 格...");
            println!("wheel 返回 {}", mouse(MOUSEEVENTF_WHEEL, ((-120i32) as u32) << 16));
        }
        other => {
            println!("未知模式 {other:?}，用 full 或 move");
        }
    }
    println!("== 探针完成 ==");
}
