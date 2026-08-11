// 屏幕坐标换算。
// 关键约定：全项目统一用【逻辑像素】（Windows 缩放 125%/150%、Mac Retina 下
// 系统 API 上报的坐标就是逻辑像素，物理像素换算留给平台层）。

/// 一条屏幕边的判定结果。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Edge {
    Left,
    Right,
    Top,
    Bottom,
    None,
}

/// 判断坐标 (x, y) 落在虚拟屏幕边界上的哪一侧。
/// virtual_w / virtual_h：本机逻辑分辨率（骨架阶段单屏；M4 支持多屏时改为屏幕并集）。
///
/// edge_margin：触发边缘切换的像素余量（比如 2px），避免边界浮点抖动。
pub fn hit_edge(x: i32, y: i32, virtual_w: i32, virtual_h: i32, edge_margin: i32) -> Edge {
    if x < edge_margin {
        Edge::Left
    } else if x >= virtual_w - edge_margin {
        Edge::Right
    } else if y < edge_margin {
        Edge::Top
    } else if y >= virtual_h - edge_margin {
        Edge::Bottom
    } else {
        Edge::None
    }
}

/// 从"对端屏幕边"进入时，对端注入的落点坐标。
/// 例：本机从右边缘滑出 → 对端从 (0, y) 进入。
pub fn enter_position(edge: Edge, cursor_y: i32, cursor_x: i32, virtual_w: i32, virtual_h: i32) -> (i32, i32) {
    // macOS 的箭头热点位于左上角；落在最外侧像素时大部分图形会在屏幕外，
    // 看起来像在边缘闪烁。入口放到屏内几像素，同时仍保留贴边的跨屏手感。
    const ENTRY_INSET: i32 = 4;
    let left = ENTRY_INSET.min((virtual_w - 1).max(0));
    let right = (virtual_w - 1 - ENTRY_INSET).max(0);
    let top = ENTRY_INSET.min((virtual_h - 1).max(0));
    let bottom = (virtual_h - 1 - ENTRY_INSET).max(0);
    match edge {
        Edge::Left => (right, cursor_y),
        Edge::Right => (left, cursor_y),
        Edge::Top => (cursor_x, bottom),
        Edge::Bottom => (cursor_x, top),
        Edge::None => (cursor_x, cursor_y),
    }
}

/// 跨屏坐标等比映射：源屏 (src_w×src_h) 上的 (x, y) → 目标屏 (dst_w×dst_h) 坐标。
/// 两端分辨率不同时保证对端光标能覆盖全屏；尺寸为 0 或完全相同时原样返回（防除零）。
pub fn map_coords(x: i32, y: i32, src_w: u32, src_h: u32, dst_w: u32, dst_h: u32) -> (i32, i32) {
    if src_w == 0 || src_h == 0 || dst_w == 0 || dst_h == 0 || (src_w == dst_w && src_h == dst_h)
    {
        return (x, y);
    }
    let nx = (x as i64 * dst_w as i64 / src_w as i64) as i32;
    let ny = (y as i64 * dst_h as i64 / src_h as i64) as i32;
    (nx, ny)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hit_right_edge() {
        assert_eq!(hit_edge(1279, 500, 1280, 800, 2), Edge::Right);
        assert_eq!(hit_edge(1270, 500, 1280, 800, 2), Edge::None);
    }

    #[test]
    fn enter_from_right() {
        assert_eq!(enter_position(Edge::Right, 300, 0, 1280, 800), (4, 300));
    }

    #[test]
    fn map_small_to_large() {
        // 小屏(1280×800) → 大屏(1920×1080)：等比放大，右边缘映射到右边缘
        assert_eq!(map_coords(1280, 800, 1280, 800, 1920, 1080), (1920, 1080));
        assert_eq!(map_coords(640, 400, 1280, 800, 1920, 1080), (960, 540));
    }

    #[test]
    fn map_large_to_small() {
        // 大屏(2560×1440) → 小屏(1280×800)：等比缩小
        assert_eq!(map_coords(2560, 1440, 2560, 1440, 1280, 800), (1280, 800));
        assert_eq!(map_coords(1280, 720, 2560, 1440, 1280, 800), (640, 400));
    }

    #[test]
    fn map_same_or_zero_passthrough() {
        assert_eq!(map_coords(100, 200, 1920, 1080, 1920, 1080), (100, 200));
        assert_eq!(map_coords(100, 200, 0, 0, 1920, 1080), (100, 200));
        assert_eq!(map_coords(100, 200, 1920, 1080, 0, 0), (100, 200));
    }
}
