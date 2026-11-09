//! GPUI 按键 → libghostty 按键事件。编码（应用光标键、Kitty 键盘协议等）交给 libghostty。

use gpui::Keystroke;
use libghostty_vt::key::{Key, Mods};

use super::engine::KeyInput;

/// 这个按键要不要由终端直接编码。返回 None 的交给输入法（普通字符、组字）或应用快捷键（⌘ 组合）。
pub fn encode(keystroke: &Keystroke, repeat: bool) -> Option<KeyInput> {
    let m = &keystroke.modifiers;
    if m.platform {
        // ⌘ 组合归应用（复制、粘贴、关标签……）
        return None;
    }
    let (key, unshifted) = physical_key(&keystroke.key)?;
    let text = keystroke.key_char.clone().filter(|t| !t.is_empty());
    let printable = unshifted != '\0';
    // 普通字符（含 Shift、Option 打出来的字符）走输入法，才能用中文输入法、死键组字
    if printable && !m.control && text.is_some() {
        return None;
    }
    let mut mods = Mods::empty();
    if m.shift {
        mods |= Mods::SHIFT;
    }
    if m.alt {
        mods |= Mods::ALT;
    }
    if m.control {
        mods |= Mods::CTRL;
    }
    let mut consumed = Mods::empty();
    if printable && m.shift {
        consumed |= Mods::SHIFT;
    }
    Some(KeyInput {
        key,
        mods,
        consumed_mods: consumed,
        // Ctrl 组合的文字（如 ^C 的 "\u{3}"）由编码器自己算，不带平台给的文字
        text: if m.control { None } else { text },
        unshifted,
        repeat,
    })
}

/// GPUI 的按键名 → (物理键, 不按 Shift 时这个键打出的字符；非字符键为 '\0')。
fn physical_key(name: &str) -> Option<(Key, char)> {
    let named = match name {
        "enter" => Key::Enter,
        "tab" => Key::Tab,
        "backspace" => Key::Backspace,
        "escape" => Key::Escape,
        "space" => return Some((Key::Space, ' ')),
        "delete" => Key::Delete,
        "insert" => Key::Insert,
        "home" => Key::Home,
        "end" => Key::End,
        "pageup" => Key::PageUp,
        "pagedown" => Key::PageDown,
        "up" => Key::ArrowUp,
        "down" => Key::ArrowDown,
        "left" => Key::ArrowLeft,
        "right" => Key::ArrowRight,
        "f1" => Key::F1,
        "f2" => Key::F2,
        "f3" => Key::F3,
        "f4" => Key::F4,
        "f5" => Key::F5,
        "f6" => Key::F6,
        "f7" => Key::F7,
        "f8" => Key::F8,
        "f9" => Key::F9,
        "f10" => Key::F10,
        "f11" => Key::F11,
        "f12" => Key::F12,
        "f13" => Key::F13,
        "f14" => Key::F14,
        "f15" => Key::F15,
        "f16" => Key::F16,
        "f17" => Key::F17,
        "f18" => Key::F18,
        "f19" => Key::F19,
        _ => {
            let mut chars = name.chars();
            let (Some(ch), None) = (chars.next(), chars.next()) else {
                return None;
            };
            return char_key(ch.to_ascii_lowercase()).map(|key| (key, ch.to_ascii_lowercase()));
        }
    };
    Some((named, '\0'))
}

fn char_key(ch: char) -> Option<Key> {
    Some(match ch {
        'a' => Key::A,
        'b' => Key::B,
        'c' => Key::C,
        'd' => Key::D,
        'e' => Key::E,
        'f' => Key::F,
        'g' => Key::G,
        'h' => Key::H,
        'i' => Key::I,
        'j' => Key::J,
        'k' => Key::K,
        'l' => Key::L,
        'm' => Key::M,
        'n' => Key::N,
        'o' => Key::O,
        'p' => Key::P,
        'q' => Key::Q,
        'r' => Key::R,
        's' => Key::S,
        't' => Key::T,
        'u' => Key::U,
        'v' => Key::V,
        'w' => Key::W,
        'x' => Key::X,
        'y' => Key::Y,
        'z' => Key::Z,
        '0' => Key::Digit0,
        '1' => Key::Digit1,
        '2' => Key::Digit2,
        '3' => Key::Digit3,
        '4' => Key::Digit4,
        '5' => Key::Digit5,
        '6' => Key::Digit6,
        '7' => Key::Digit7,
        '8' => Key::Digit8,
        '9' => Key::Digit9,
        '-' => Key::Minus,
        '=' => Key::Equal,
        '[' => Key::BracketLeft,
        ']' => Key::BracketRight,
        '\\' => Key::Backslash,
        ';' => Key::Semicolon,
        '\'' => Key::Quote,
        ',' => Key::Comma,
        '.' => Key::Period,
        '/' => Key::Slash,
        '`' => Key::Backquote,
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::Modifiers;

    fn stroke(key: &str, key_char: Option<&str>, modifiers: Modifiers) -> Keystroke {
        Keystroke { modifiers, key: key.into(), key_char: key_char.map(Into::into) }
    }

    #[test]
    fn plain_characters_go_through_the_input_method() {
        assert!(encode(&stroke("a", Some("a"), Modifiers::none()), false).is_none());
        assert!(encode(&stroke("a", Some("A"), Modifiers::shift()), false).is_none());
        // Option 打出的特殊字符也是文字（不把 Option 当 Meta，同 Ghostty / xterm 默认）
        let option = Modifiers { alt: true, ..Modifiers::none() };
        assert!(encode(&stroke("f", Some("ƒ"), option), false).is_none());
    }

    #[test]
    fn control_and_special_keys_are_encoded_by_the_terminal() {
        let ctrl_c = encode(&stroke("c", None, Modifiers::control()), false).unwrap();
        assert_eq!(ctrl_c.key, Key::C);
        assert!(ctrl_c.mods.contains(Mods::CTRL));
        assert_eq!(ctrl_c.text, None);
        let enter = encode(&stroke("enter", None, Modifiers::none()), false).unwrap();
        assert_eq!(enter.key, Key::Enter);
        assert_eq!(enter.unshifted, '\0');
        let up = encode(&stroke("up", None, Modifiers::none()), true).unwrap();
        assert!(up.repeat);
    }

    #[test]
    fn command_shortcuts_belong_to_the_app() {
        assert!(encode(&stroke("c", Some("c"), Modifiers::command()), false).is_none());
        assert!(encode(&stroke("w", None, Modifiers::command()), false).is_none());
    }
}
