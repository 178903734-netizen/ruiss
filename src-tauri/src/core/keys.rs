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
    // 导航/编辑键
    Delete, Home, End, PageUp, PageDown, Insert, CapsLock,
    // 标点符号（中英文输入必备）
    Comma, Period, Slash, Semicolon, Quote,
    LBracket, RBracket, Backslash, Minus, Equals, Backtick,
    Other(u32), // 未覆盖键：透传原始码
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ModifierState {
    pub ctrl: bool,
    pub alt: bool,
    pub shift: bool,
    pub super_key: bool,
}

impl ModifierState {
    pub fn set(&mut self, key: Key, down: bool) -> bool {
        match key {
            Key::Ctrl => self.ctrl = down,
            Key::Alt => self.alt = down,
            Key::Shift => self.shift = down,
            Key::Super => self.super_key = down,
            _ => return false,
        }
        true
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ShortcutStroke {
    pub key: Key,
    pub modifiers: ModifierState,
}

/// 将"来源端"的键转换为"目标端"应该注入的键。
/// Windows/macOS 跨平台时 Ctrl ↔ Command(Super) 互换，Shift / Alt 不动。
/// 这样 Windows Ctrl+C → Mac Command+C，Mac Command+C → Windows Ctrl+C。
/// 参数仅保留目标端方向语义；当前 Windows↔macOS 的互换规则是对称的。
pub fn map_key(_target_is_mac: bool, key: Key) -> Key {
    match key {
        Key::Ctrl => Key::Super,
        Key::Super => Key::Ctrl,
        other => other,
    }
}

/// 跨 Windows/macOS 时，Ctrl 与 Command(Super) 互换；Alt/Option、Shift 保持。
pub fn map_modifiers(target_is_mac: bool, source: ModifierState) -> ModifierState {
    let mut target = ModifierState::default();
    for (key, down) in [
        (Key::Ctrl, source.ctrl),
        (Key::Alt, source.alt),
        (Key::Shift, source.shift),
        (Key::Super, source.super_key),
    ] {
        if down {
            target.set(map_key(target_is_mac, key), true);
        }
    }
    target
}

/// Windows 键盘控制 Mac 时，将两套系统语义明显不同的常用快捷键翻译为 Mac 语义。
/// 未命中的组合走 Ctrl→Command、Win→Control 的通用映射。
pub fn translate_windows_shortcut_to_mac(
    source: ModifierState,
    key: Key,
) -> ShortcutStroke {
    let mut mapped = ShortcutStroke {
        key,
        modifiers: map_modifiers(true, source),
    };
    let shift = source.shift;

    match key {
        // Windows Alt+Tab → macOS Command+Tab（Shift 反向切换保留）。
        Key::Tab if source.alt && !source.ctrl && !source.super_key => {
            mapped.modifiers = ModifierState { super_key: true, shift, ..Default::default() };
        }
        // Windows Ctrl+Tab → macOS Control+Tab（浏览器/标签页切换）。
        Key::Tab if source.ctrl && !source.alt && !source.super_key => {
            mapped.modifiers = ModifierState { ctrl: true, shift, ..Default::default() };
        }
        // Windows Win+Ctrl+←/→ → macOS Control+←/→（切换桌面空间）。
        Key::ArrowLeft | Key::ArrowRight
            if source.super_key && source.ctrl && !source.alt =>
        {
            mapped.modifiers = ModifierState { ctrl: true, shift, ..Default::default() };
        }
        // Windows Win+Tab → macOS Control+↑（Mission Control）。
        Key::Tab if source.super_key && !source.ctrl && !source.alt => {
            mapped.key = Key::ArrowUp;
            mapped.modifiers = ModifierState { ctrl: true, ..Default::default() };
        }
        // Windows Alt+F4 → macOS Command+Q。
        Key::F4 if source.alt && !source.ctrl && !source.super_key => {
            mapped.key = Key::Q;
            mapped.modifiers = ModifierState { super_key: true, ..Default::default() };
        }
        // Windows Win+L → macOS Command+Control+Q（锁定屏幕）。
        Key::L if source.super_key && !source.ctrl && !source.alt && !source.shift => {
            mapped.key = Key::Q;
            mapped.modifiers = ModifierState {
                ctrl: true,
                super_key: true,
                ..Default::default()
            };
        }
        // Windows Win+Shift+S → macOS Command+Shift+4（区域截图）。
        Key::S if source.super_key && source.shift && !source.ctrl && !source.alt => {
            mapped.key = Key::Digit4;
            mapped.modifiers = ModifierState {
                shift: true,
                super_key: true,
                ..Default::default()
            };
        }
        // Windows Win+D → macOS Command+F3（显示桌面）。
        Key::D if source.super_key && !source.ctrl && !source.alt && !source.shift => {
            mapped.key = Key::F3;
            mapped.modifiers = ModifierState { super_key: true, ..Default::default() };
        }
        // Windows Win+E → macOS Command+Option+Space（Finder 窗口）。
        Key::E if source.super_key && !source.ctrl && !source.alt && !source.shift => {
            mapped.key = Key::Space;
            mapped.modifiers = ModifierState {
                alt: true,
                super_key: true,
                ..Default::default()
            };
        }
        // Windows Win+Space → macOS Control+Space（切换输入法）。
        Key::Space if source.super_key && !source.ctrl && !source.alt => {
            mapped.modifiers = ModifierState { ctrl: true, shift, ..Default::default() };
        }
        // Windows Win+R → macOS Command+Space（打开 Spotlight）。
        Key::R if source.super_key && !source.ctrl && !source.alt && !source.shift => {
            mapped.key = Key::Space;
            mapped.modifiers = ModifierState { super_key: true, ..Default::default() };
        }
        // Windows Win+S → macOS Command+Space（搜索/Spotlight）。
        Key::S if source.super_key && !source.ctrl && !source.alt && !source.shift => {
            mapped.key = Key::Space;
            mapped.modifiers = ModifierState { super_key: true, ..Default::default() };
        }
        // Windows Ctrl+Shift+Esc → macOS Command+Option+Esc（强制退出）。
        Key::Esc if source.ctrl && source.shift && !source.super_key && !source.alt => {
            mapped.modifiers = ModifierState {
                alt: true,
                super_key: true,
                ..Default::default()
            };
        }
        // Windows Ctrl+方向键 → macOS Control+方向键（空间切换/Mission Control/App Exposé）。
        Key::ArrowLeft | Key::ArrowRight | Key::ArrowUp | Key::ArrowDown
            if source.ctrl && !source.alt && !source.super_key =>
        {
            mapped.modifiers = ModifierState { ctrl: true, shift, ..Default::default() };
        }
        // Windows Ctrl+退格/删除 → macOS Option 语义（按词删除）。
        Key::Backspace | Key::Delete
            if source.ctrl && !source.alt && !source.super_key =>
        {
            mapped.modifiers = ModifierState { alt: true, shift, ..Default::default() };
        }
        // Windows Ctrl+Home/End → macOS Command+↑/↓（文档开头/结尾）。
        Key::Home if source.ctrl && !source.alt && !source.super_key => {
            mapped.key = Key::ArrowUp;
            mapped.modifiers = ModifierState { super_key: true, shift, ..Default::default() };
        }
        Key::End if source.ctrl && !source.alt && !source.super_key => {
            mapped.key = Key::ArrowDown;
            mapped.modifiers = ModifierState { super_key: true, shift, ..Default::default() };
        }
        // Windows Home/End → macOS Command+←/→（行首/行尾）。
        Key::Home if !source.ctrl && !source.alt && !source.super_key => {
            mapped.key = Key::ArrowLeft;
            mapped.modifiers = ModifierState { super_key: true, shift, ..Default::default() };
        }
        Key::End if !source.ctrl && !source.alt && !source.super_key => {
            mapped.key = Key::ArrowRight;
            mapped.modifiers = ModifierState { super_key: true, shift, ..Default::default() };
        }
        // Windows Alt+←/→ → macOS Command+[/]（浏览器前进/后退）。
        Key::ArrowLeft if source.alt && !source.ctrl && !source.super_key => {
            mapped.key = Key::LBracket;
            mapped.modifiers = ModifierState { super_key: true, ..Default::default() };
        }
        Key::ArrowRight if source.alt && !source.ctrl && !source.super_key => {
            mapped.key = Key::RBracket;
            mapped.modifiers = ModifierState { super_key: true, ..Default::default() };
        }
        _ => {}
    }
    mapped
}

/// macOS 键盘控制 Windows 时，将系统级快捷键翻译为 Windows 语义。
/// 未命中的组合走 Command→Ctrl、Control→Win 的通用映射。
pub fn translate_macos_shortcut_to_windows(
    source: ModifierState,
    key: Key,
) -> ShortcutStroke {
    let mut mapped = ShortcutStroke {
        key,
        modifiers: map_modifiers(false, source),
    };
    let shift = source.shift;

    match key {
        // macOS Command+Tab → Windows Alt+Tab。
        Key::Tab if source.super_key && !source.ctrl && !source.alt => {
            mapped.modifiers = ModifierState { alt: true, shift, ..Default::default() };
        }
        // macOS Control+Tab → Windows Ctrl+Tab（浏览器/标签页切换）。
        Key::Tab if source.ctrl && !source.super_key && !source.alt => {
            mapped.modifiers = ModifierState { ctrl: true, shift, ..Default::default() };
        }
        // macOS Control+←/→ → Windows Win+Ctrl+←/→（切换虚拟桌面）。
        Key::ArrowLeft | Key::ArrowRight
            if source.ctrl && !source.super_key && !source.alt =>
        {
            mapped.modifiers = ModifierState {
                ctrl: true,
                super_key: true,
                shift,
                ..Default::default()
            };
        }
        // macOS Control+↑ → Windows Win+Tab（任务视图）。
        Key::ArrowUp if source.ctrl && !source.super_key && !source.alt => {
            mapped.key = Key::Tab;
            mapped.modifiers = ModifierState { super_key: true, ..Default::default() };
        }
        // macOS Command+Q → Windows Alt+F4。
        Key::Q if source.super_key && !source.ctrl && !source.alt => {
            mapped.key = Key::F4;
            mapped.modifiers = ModifierState { alt: true, ..Default::default() };
        }
        // macOS Command+Control+Q → Windows Win+L。
        Key::Q if source.super_key && source.ctrl && !source.alt && !source.shift => {
            mapped.key = Key::L;
            mapped.modifiers = ModifierState { super_key: true, ..Default::default() };
        }
        // macOS Command+Shift+4/5 → Windows Win+Shift+S（截图工具）。
        Key::Digit4 | Key::Digit5
            if source.super_key && source.shift && !source.ctrl && !source.alt =>
        {
            mapped.key = Key::S;
            mapped.modifiers = ModifierState {
                shift: true,
                super_key: true,
                ..Default::default()
            };
        }
        // macOS Command+F3 → Windows Win+D（显示桌面）。
        Key::F3 if source.super_key && !source.ctrl && !source.alt && !source.shift => {
            mapped.key = Key::D;
            mapped.modifiers = ModifierState { super_key: true, ..Default::default() };
        }
        // macOS Command+Option+Space → Windows Win+E（文件资源管理器）。
        Key::Space if source.super_key && source.alt && !source.ctrl && !source.shift => {
            mapped.key = Key::E;
            mapped.modifiers = ModifierState { super_key: true, ..Default::default() };
        }
        // macOS Control+Space → Windows Win+Space（切换输入法）。
        Key::Space if source.ctrl && !source.super_key && !source.alt => {
            mapped.modifiers = ModifierState { super_key: true, shift, ..Default::default() };
        }
        // macOS Command+Space → Windows Win+S（搜索）。
        Key::Space if source.super_key && !source.ctrl && !source.alt => {
            mapped.key = Key::S;
            mapped.modifiers = ModifierState { super_key: true, ..Default::default() };
        }
        // macOS Option+方向键/退格/删除 → Windows Ctrl 语义（按词移动/删除）。
        Key::ArrowLeft | Key::ArrowRight | Key::ArrowUp | Key::ArrowDown
        | Key::Backspace | Key::Delete
            if source.alt && !source.ctrl && !source.super_key =>
        {
            mapped.modifiers = ModifierState { ctrl: true, shift, ..Default::default() };
        }
        // macOS Command+↑/↓ → Windows Ctrl+Home/End。
        Key::ArrowUp if source.super_key && !source.ctrl && !source.alt => {
            mapped.key = Key::Home;
            mapped.modifiers = ModifierState { ctrl: true, shift, ..Default::default() };
        }
        Key::ArrowDown if source.super_key && !source.ctrl && !source.alt => {
            mapped.key = Key::End;
            mapped.modifiers = ModifierState { ctrl: true, shift, ..Default::default() };
        }
        // macOS Command+←/→ → Windows Home/End（行首/行尾）。
        Key::ArrowLeft if source.super_key && !source.ctrl && !source.alt => {
            mapped.key = Key::Home;
            mapped.modifiers = ModifierState { shift, ..Default::default() };
        }
        Key::ArrowRight if source.super_key && !source.ctrl && !source.alt => {
            mapped.key = Key::End;
            mapped.modifiers = ModifierState { shift, ..Default::default() };
        }
        // macOS Command+[/] → Windows Alt+←/→（浏览器后退/前进）。
        Key::LBracket if source.super_key && !source.ctrl && !source.alt => {
            mapped.key = Key::ArrowLeft;
            mapped.modifiers = ModifierState { alt: true, ..Default::default() };
        }
        Key::RBracket if source.super_key && !source.ctrl && !source.alt => {
            mapped.key = Key::ArrowRight;
            mapped.modifiers = ModifierState { alt: true, ..Default::default() };
        }
        // macOS Command+Option+Esc → Windows Ctrl+Shift+Esc（任务管理器）。
        Key::Esc if source.super_key && source.alt && !source.ctrl => {
            mapped.modifiers = ModifierState {
                ctrl: true,
                shift: true,
                ..Default::default()
            };
        }
        _ => {}
    }
    mapped
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
    fn mac_to_windows_command_becomes_ctrl() {
        assert_eq!(map_key(false, Key::Super), Key::Ctrl);
        assert_eq!(map_key(false, Key::Ctrl), Key::Super);
        assert_eq!(map_key(false, Key::Alt), Key::Alt);
    }

    #[test]
    fn win_to_mac_ctrl_becomes_super() {
        // 来源是 Windows（Ctrl 在 Ctrl 位），目标 Mac：Ctrl 应映射为 Command
        assert_eq!(map_key(true, Key::Ctrl), Key::Super);
        // Win 键 → Command
        assert_eq!(map_key(true, Key::Super), Key::Ctrl); // 注：第一版规则里 Super 互换为 Ctrl
    }

    #[test]
    fn windows_common_shortcuts_translate_to_mac() {
        let ctrl = ModifierState { ctrl: true, ..Default::default() };
        assert_eq!(
            translate_windows_shortcut_to_mac(ctrl, Key::C),
            ShortcutStroke {
                key: Key::C,
                modifiers: ModifierState { super_key: true, ..Default::default() },
            }
        );

        let alt = ModifierState { alt: true, ..Default::default() };
        assert_eq!(
            translate_windows_shortcut_to_mac(alt, Key::Tab),
            ShortcutStroke {
                key: Key::Tab,
                modifiers: ModifierState { super_key: true, ..Default::default() },
            }
        );
    }

    #[test]
    fn windows_desktop_and_navigation_shortcuts_translate_to_mac() {
        let desktop = ModifierState { ctrl: true, super_key: true, ..Default::default() };
        assert_eq!(
            translate_windows_shortcut_to_mac(desktop, Key::ArrowRight),
            ShortcutStroke {
                key: Key::ArrowRight,
                modifiers: ModifierState { ctrl: true, ..Default::default() },
            }
        );

        let ctrl_shift = ModifierState { ctrl: true, shift: true, ..Default::default() };
        assert_eq!(
            translate_windows_shortcut_to_mac(ctrl_shift, Key::ArrowLeft),
            ShortcutStroke {
                key: Key::ArrowLeft,
                modifiers: ModifierState { ctrl: true, shift: true, ..Default::default() },
            }
        );
        assert_eq!(
            translate_windows_shortcut_to_mac(
                ModifierState { ctrl: true, ..Default::default() },
                Key::ArrowUp,
            ),
            ShortcutStroke {
                key: Key::ArrowUp,
                modifiers: ModifierState { ctrl: true, ..Default::default() },
            }
        );
    }

    #[test]
    fn macos_common_shortcuts_translate_to_windows() {
        let command = ModifierState { super_key: true, ..Default::default() };
        assert_eq!(
            translate_macos_shortcut_to_windows(command, Key::C),
            ShortcutStroke {
                key: Key::C,
                modifiers: ModifierState { ctrl: true, ..Default::default() },
            }
        );
        assert_eq!(
            translate_macos_shortcut_to_windows(command, Key::Tab),
            ShortcutStroke {
                key: Key::Tab,
                modifiers: ModifierState { alt: true, ..Default::default() },
            }
        );
    }

    #[test]
    fn macos_desktop_shortcuts_translate_to_windows() {
        let control = ModifierState { ctrl: true, ..Default::default() };
        assert_eq!(
            translate_macos_shortcut_to_windows(control, Key::ArrowRight),
            ShortcutStroke {
                key: Key::ArrowRight,
                modifiers: ModifierState {
                    ctrl: true,
                    super_key: true,
                    ..Default::default()
                },
            }
        );
        assert_eq!(
            translate_macos_shortcut_to_windows(control, Key::ArrowUp),
            ShortcutStroke {
                key: Key::Tab,
                modifiers: ModifierState { super_key: true, ..Default::default() },
            }
        );
    }
}
