// 键位映射表。
// 第一版规则就三条（规划里定死的）：
//   1. Ctrl ↔ Command 硬映射（Mac 上 Command 才是 Ctrl 的位）
//   2. Windows 键 → Mac 的 Command
//   3. Shift / Alt 不动
// 键码用平台无关的抽象码（Qw 虚拟键位），映射后由平台层转成
// Windows VK / Mac CGKeyCode。

/// 抽象键位枚举（覆盖第一版需要的键）。
/// 跨机器传输的就是它（协议 Payload::Key.key）——平台无关。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub enum Key {
    A, B, C, D, E, F, G, H, I, J, K, L, M,
    N, O, P, Q, R, S, T, U, V, W, X, Y, Z,
    Digit0, Digit1, Digit2, Digit3, Digit4,
    Digit5, Digit6, Digit7, Digit8, Digit9,
    Ctrl, Alt, Shift, Super, // Super = Win / Command
    Enter, Space, Backspace, Tab, Esc,
    ArrowUp, ArrowDown, ArrowLeft, ArrowRight,
    F1, F2, F3, F4, F5, F6, F7, F8, F9, F10, F11, F12,
    // 导航键
    Delete, Home, End, PageUp, PageDown, Insert, CapsLock,
    // 标点符号（中英文输入必备）
    Comma, Period, Slash, Semicolon, Quote,
    LBracket, RBracket, Backslash, Minus, Equals, Backtick,
    Other(u32), // 未覆盖键：透传原始码
}

/// 将"来源端"的键转换为"目标端"应该注入的键。
/// 规则（M2 双 Windows 联调时恒等透传；Mac 目标端才做映射）：
///   - 目标 Windows：原样透传（Win↔Win 没有键位差异）；
///   - 目标 Mac：Ctrl ↔ Command(Super) 互换（Mac 上 Command 才是 Ctrl 的位），
///     Win 键 → Command，Shift / Alt 不动。
/// 参数 `target_is_mac`：目标端是否为 Mac。
pub fn map_key(target_is_mac: bool, key: Key) -> Key {
    if !target_is_mac {
        return key;
    }
    match key {
        Key::Ctrl => Key::Super,
        Key::Super => Key::Ctrl,
        other => other,
    }
}

/// 字符 → 抽象键码（注入测试用；仅支持小写字母/数字/空格）。
pub fn char_to_key(c: char) -> Option<Key> {
    const LETTERS: [Key; 26] = [
        Key::A, Key::B, Key::C, Key::D, Key::E, Key::F, Key::G, Key::H, Key::I, Key::J, Key::K,
        Key::L, Key::M, Key::N, Key::O, Key::P, Key::Q, Key::R, Key::S, Key::T, Key::U, Key::V,
        Key::W, Key::X, Key::Y, Key::Z,
    ];
    const DIGITS: [Key; 10] = [
        Key::Digit0, Key::Digit1, Key::Digit2, Key::Digit3, Key::Digit4, Key::Digit5, Key::Digit6,
        Key::Digit7, Key::Digit8, Key::Digit9,
    ];
    match c {
        'a'..='z' => Some(LETTERS[(c as u32 - 'a' as u32) as usize]),
        '0'..='9' => Some(DIGITS[(c as u32 - '0' as u32) as usize]),
        ' ' => Some(Key::Space),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn win_to_win_is_identity() {
        // 双 Windows 联调（M2 主场景）：所有键原样
        for key in [Key::Ctrl, Key::Super, Key::Alt, Key::Shift, Key::A, Key::Enter, Key::Other(0x5B)] {
            assert_eq!(map_key(false, key), key);
        }
    }

    #[test]
    fn win_to_mac_ctrl_becomes_super() {
        // 来源是 Windows（Ctrl 在 Ctrl 位），目标 Mac：Ctrl 应映射为 Command
        assert_eq!(map_key(true, Key::Ctrl), Key::Super);
        // Win 键 → Command
        assert_eq!(map_key(true, Key::Super), Key::Ctrl); // 注：第一版规则里 Super 互换为 Ctrl
    }

    #[test]
    fn mac_to_win_identity() {
        // 目标 Windows：即使来源是 Mac，也原样透传（映射在 Mac 端捕获时已完成）
        assert_eq!(map_key(false, Key::Super), Key::Super);
    }
}
