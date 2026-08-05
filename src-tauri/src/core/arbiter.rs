// 控制仲裁状态机（M2）。
//
// 跨屏模型（绝对坐标镜像 + 光标回绕）：
//   1. 本机光标在"出口边"（对端所在的那条边）停留 150ms → 发起 TakeControl：
//      - 对端收到后进入 Sink（注入模式），光标落到入口位置；
//      - 本机光标回绕（warp）到对侧边缘 —— Windows 会把光标钳制在屏幕边缘，
//        不回绕的话继续往外的移动在系统层面消失，钩子看不到意图。
//   2. 跨屏期间（linked）：本机鼠标移动/按键/点击/滚轮全部转发，对端注入；
//      两台光标按同一绝对坐标镜像（M2 假设分辨率一致，DPI 适配是 M4）。
//   3. 回到出口边停留 150ms → ReleaseControl，控制归还。
//   4. 对端（Sink）用户在自己出口边停留 150ms → 发 TakeControl 夺回，完全对称。
//
// 已知限制（M2 标注，M4 打磨）：双方同时抢控时可能两边都变 Sink（僵住），
// 任一方在出口边停留即可夺回；Sink 期间对端持续注入会与本机真实鼠标"打架"。

use std::time::{Duration, Instant};

use crate::core::geometry::{self, Edge};
use crate::core::protocol::Payload;

/// 跨屏布局：对端在我哪边。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Layout {
    /// 对端在右边：从本机右边缘滑出/返回
    PeerRight,
    /// 对端在左边：从本机左边缘滑出/返回
    PeerLeft,
}

/// 控制模式：Source=本机主控（转发本机事件）；Sink=对端主控（注入对端事件）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Source,
    Sink,
}

/// 状态机产出的动作（由上层执行）。
#[derive(Debug, Clone, PartialEq)]
pub enum Action {
    /// 请对端接管：对端进入 Sink 并把光标落到 (x, y)（对端坐标系入口位置），
    /// src_w/src_h 为源端屏幕尺寸（对端注入入口时按比例映射）
    TakeControl { x: i32, y: i32, src_w: u32, src_h: u32 },
    /// 归还控制：对端恢复 Source
    ReleaseControl,
    /// 本机光标回绕到 (x, y)（跨屏后继续移动才可见）
    Warp { x: i32, y: i32 },
    /// 转发给对端的事件
    Forward(Payload),
    /// 无动作
    None,
}

/// 边界像素余量（与 geometry::hit_edge 默认一致）
const EDGE_MARGIN: i32 = 2;
/// 边缘停留防误触时长（规划 100~200ms）
const DWELL: Duration = Duration::from_millis(150);

/// 仲裁器：每次光标事件调用 on_cursor，空闲时上层每 100ms 调 on_tick。
pub struct Arbiter {
    layout: Layout,
    exit_edge: Edge,
    pub mode: Mode,
    /// 本机是否处于跨屏中（Source 侧为真）
    pub linked: bool,
    /// 光标进入出口边的时间与位置（停留计时）
    dwell: Option<(Instant, i32, i32)>,
    /// 最近一次光标位置（on_tick 用）
    last: Option<(i32, i32, i32, i32)>,
}

impl Arbiter {
    pub fn new(layout: Layout) -> Self {
        let exit_edge = match layout {
            Layout::PeerRight => Edge::Right,
            Layout::PeerLeft => Edge::Left,
        };
        Self { layout, exit_edge, mode: Mode::Source, linked: false, dwell: None, last: None }
    }

    /// 本机光标事件。`now` 可注入（单测用），生产传 Instant::now()。
    pub fn on_cursor(&mut self, x: i32, y: i32, w: i32, h: i32, now: Instant) -> Vec<Action> {
        self.last = Some((x, y, w, h));
        let at_exit = geometry::hit_edge(x, y, w, h, EDGE_MARGIN) == self.exit_edge;

        // 出口边停留计时（离开即清零）
        match (at_exit, self.dwell) {
            (true, None) => self.dwell = Some((now, x, y)),
            (false, Some(_)) => self.dwell = None,
            _ => {}
        }

        if at_exit {
            let actions = self.check_dwell(now, w, h);
            if !actions.is_empty() {
                return actions;
            }
        }

        // Sink（被对端控制）：本机事件不转发、不触发边缘
        if self.mode == Mode::Sink {
            return Vec::new();
        }
        if self.linked {
            vec![Action::Forward(Payload::MouseMove {
                x,
                y,
                src_w: w as u32,
                src_h: h as u32,
            })]
        } else {
            Vec::new()
        }
    }

    /// 本机按键/点击/滚轮事件。
    pub fn on_input(&mut self, payload: Payload) -> Action {
        if self.mode == Mode::Sink || !self.linked {
            return Action::None;
        }
        Action::Forward(payload)
    }

    /// 空闲心跳（上层每 ~100ms 调用）：光标停在边缘不动时也能触发停留判定。
    pub fn on_tick(&mut self, now: Instant) -> Vec<Action> {
        match self.last {
            Some((_x, _y, w, h)) => self.check_dwell(now, w, h),
            None => Vec::new(),
        }
    }

    /// 对端发来 TakeControl：接受，进入 Sink，返回对端给的入口位置（需注入）。
    /// 已知限制：双方同时抢控时都接受可能双双变 Sink，任一方出口边停留即可夺回。
    pub fn on_peer_take(&mut self, x: i32, y: i32) -> Option<(i32, i32)> {
        self.mode = Mode::Sink;
        self.linked = false;
        Some((x, y))
    }

    /// 对端发来 ReleaseControl：恢复 Source。
    pub fn on_peer_release(&mut self) {
        self.mode = Mode::Source;
        self.linked = false;
    }

    /// 停留判定：dwell 到期 → 归还（已跨屏）或发起（未跨屏/夺回）。
    fn check_dwell(&mut self, now: Instant, w: i32, h: i32) -> Vec<Action> {
        let (start, x, y) = match self.dwell {
            Some(d) => d,
            None => return Vec::new(),
        };
        if now.duration_since(start) < DWELL {
            return Vec::new();
        }
        self.dwell = None; // 防重复触发（触发后本机光标仍停在边缘，避免反复触发）

        if self.linked {
            // 跨屏中回到出口边 → 归还
            self.linked = false;
            return vec![Action::ReleaseControl];
        }

        // 发起/夺回：成为 Source 并跨屏
        self.mode = Mode::Source;
        self.linked = true;
        let (peer_x, peer_y) = geometry::enter_position(self.exit_edge, y, x, w, h);
        let warp_x = match self.exit_edge {
            Edge::Right => 1,   // 回绕到左边缘内侧
            Edge::Left => w - 2, // 回绕到右边缘内侧
            _ => x,
        };
        vec![
            Action::TakeControl { x: peer_x, y: peer_y, src_w: w as u32, src_h: h as u32 },
            Action::Warp { x: warp_x, y },
        ]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::keys::Key;
    use std::time::Duration;

    const W: i32 = 1280;
    const H: i32 = 800;
    const EDGE_X: i32 = W - 1; // 右边缘（PeerRight 的出口边）
    const ENTRY_X: i32 = 0; // 对端入口 x（PeerRight → 对端左边缘）

    fn t(ms: u64) -> Instant {
        Instant::now() + Duration::from_millis(ms)
    }

    #[test]
    fn take_control_after_dwell() {
        let mut a = Arbiter::new(Layout::PeerRight);
        // 刚到边缘：开始计时，无动作
        assert!(a.on_cursor(EDGE_X, 100, W, H, t(0)).is_empty());
        // 100ms 未到
        assert!(a.on_cursor(EDGE_X, 100, W, H, t(100)).is_empty());
        // 160ms 到期：TakeControl + Warp
        let acts = a.on_cursor(EDGE_X, 100, W, H, t(160));
        assert_eq!(
            acts,
            vec![
                Action::TakeControl { x: ENTRY_X, y: 100, src_w: W as u32, src_h: H as u32 },
                Action::Warp { x: 1, y: 100 },
            ]
        );
        assert!(a.linked);
        assert_eq!(a.mode, Mode::Source);
    }

    #[test]
    fn forward_moves_while_linked() {
        let mut a = Arbiter::new(Layout::PeerRight);
        a.on_cursor(EDGE_X, 100, W, H, t(0));
        a.on_cursor(EDGE_X, 100, W, H, t(160)); // 触发跨屏
        // 跨屏后普通移动 → 转发（带源端尺寸）
        let acts = a.on_cursor(100, 200, W, H, t(200));
        assert_eq!(
            acts,
            vec![Action::Forward(Payload::MouseMove { x: 100, y: 200, src_w: W as u32, src_h: H as u32 })]
        );
        // 按键 → 转发
        let act = a.on_input(Payload::Key { key: Key::A, scan: 0x1E, extended: false, down: true });
        assert_eq!(
            act,
            Action::Forward(Payload::Key { key: Key::A, scan: 0x1E, extended: false, down: true })
        );
    }

    #[test]
    fn release_when_back_at_exit_edge() {
        let mut a = Arbiter::new(Layout::PeerRight);
        a.on_cursor(EDGE_X, 100, W, H, t(0));
        a.on_cursor(EDGE_X, 100, W, H, t(160)); // 跨屏
        // 回到出口边停留 → Release
        assert!(a.on_cursor(EDGE_X, 300, W, H, t(200)).is_empty()); // 开始计时
        let acts = a.on_cursor(EDGE_X, 300, W, H, t(360));
        assert_eq!(acts, vec![Action::ReleaseControl]);
        assert!(!a.linked);
        // 归还后普通移动不再转发
        assert!(a.on_cursor(100, 200, W, H, t(400)).is_empty());
    }

    #[test]
    fn sink_side_retake() {
        let mut a = Arbiter::new(Layout::PeerRight);
        // 对端接管
        assert_eq!(a.on_peer_take(ENTRY_X, 100), Some((ENTRY_X, 100)));
        assert_eq!(a.mode, Mode::Sink);
        // Sink 期间本机移动不转发
        assert!(a.on_cursor(500, 400, W, H, t(100)).is_empty());
        // 本机在出口边停留 → 夺回（需要 PeerLeft 布局才与出口边一致？不——Sink 侧出口边不变）
        // PeerRight 的出口边是右边缘；被对端接管后入口在左边缘。夺回边 = 出口边（右）。
        a.on_cursor(EDGE_X, 300, W, H, t(200));
        let acts = a.on_cursor(EDGE_X, 300, W, H, t(360));
        assert_eq!(
            acts,
            vec![
                Action::TakeControl { x: ENTRY_X, y: 300, src_w: W as u32, src_h: H as u32 },
                Action::Warp { x: 1, y: 300 },
            ]
        );
        assert_eq!(a.mode, Mode::Source);
        assert!(a.linked);
    }

    #[test]
    fn tick_triggers_when_cursor_stops_at_edge() {
        let mut a = Arbiter::new(Layout::PeerRight);
        // 光标停在边缘，之后不再有移动事件
        a.on_cursor(EDGE_X, 100, W, H, t(0));
        // 只有 on_tick 驱动
        assert!(a.on_tick(t(100)).is_empty());
        let acts = a.on_tick(t(160));
        assert_eq!(acts.len(), 2); // TakeControl + Warp
        assert!(a.linked);
    }

    #[test]
    fn peer_left_mirrors() {
        let mut a = Arbiter::new(Layout::PeerLeft);
        a.on_cursor(0, 100, W, H, t(0));
        let acts = a.on_cursor(0, 100, W, H, t(160));
        // PeerLeft：对端入口在 (W-1, y)，回绕到本机右边缘内侧
        assert_eq!(
            acts,
            vec![
                Action::TakeControl { x: W - 1, y: 100, src_w: W as u32, src_h: H as u32 },
                Action::Warp { x: W - 2, y: 100 },
            ]
        );
    }

    #[test]
    fn release_peer() {
        let mut a = Arbiter::new(Layout::PeerRight);
        a.on_peer_take(0, 0);
        a.on_peer_release();
        assert_eq!(a.mode, Mode::Source);
        assert!(!a.linked);
    }
}
