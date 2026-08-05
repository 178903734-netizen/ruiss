// platform：事件捕获与注入的平台实现。
// 骨架阶段仅声明模块接口；M1 里程碑填入真实实现。
// Windows 侧：SetWindowsHookEx（捕获） + SendInput（注入）
// Mac 侧：CGEventTap（捕获） + CGEventPost（注入）

#[cfg(target_os = "windows")]
mod win;
#[cfg(target_os = "windows")]
pub use win::*;

#[cfg(target_os = "macos")]
mod mac;
#[cfg(target_os = "macos")]
pub use mac::*;
