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
    // macOS 把 Shift 并进标点送来：Ctrl+Shift+- 是 key「_」、不带 Shift（gpui-pre-macos 的 parse_keystroke）。
    // 还原成物理键 + Shift，打出的字符当文字给编码器，Ctrl+_ 才编成 ^_（同 Ghostty）
    let (key, unshifted, shifted) = match physical_key(&keystroke.key) {
        Some((key, unshifted)) => (key, unshifted, None),
        None => {
            let ch = single_char(&keystroke.key)?;
            let base = unshifted_punctuation(ch)?;
            (char_key(base)?, base, Some(ch))
        }
    };
    let shift = m.shift || shifted.is_some();
    let text = keystroke.key_char.clone().filter(|t| !t.is_empty());
    let printable = unshifted != '\0';
    // 普通字符（含 Shift、Option 打出来的字符）走输入法，才能用中文输入法、死键组字
    if printable && !m.control && text.is_some() {
        return None;
    }
    let mut mods = Mods::empty();
    if shift {
        mods |= Mods::SHIFT;
    }
    if m.alt {
        mods |= Mods::ALT;
    }
    if m.control {
        mods |= Mods::CTRL;
    }
    let mut consumed = Mods::empty();
    if printable && shift {
        consumed |= Mods::SHIFT;
    }
    Some(KeyInput {
        key,
        mods,
        consumed_mods: consumed,
        // Ctrl 组合的文字（如 ^C 的 "\u{3}"）由编码器自己算，不带平台给的文字；Shift 标点给出打出的字符
        text: if m.control { shifted.map(String::from) } else { text },
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
            let ch = single_char(name)?.to_ascii_lowercase();
            return char_key(ch).map(|key| (key, ch));
        }
    };
    Some((named, '\0'))
}

fn single_char(name: &str) -> Option<char> {
    let mut chars = name.chars();
    match (chars.next(), chars.next()) {
        (Some(ch), None) => Some(ch),
        _ => None,
    }
}

/// 美式键盘上按 Shift 打出的标点 → 同一个键不按 Shift 的字符（物理键按美式布局，同 char_key）。
fn unshifted_punctuation(ch: char) -> Option<char> {
    Some(match ch {
        '!' => '1',
        '@' => '2',
        '#' => '3',
        '$' => '4',
        '%' => '5',
        '^' => '6',
        '&' => '7',
        '*' => '8',
        '(' => '9',
        ')' => '0',
        '_' => '-',
        '+' => '=',
        '{' => '[',
        '}' => ']',
        '|' => '\\',
        ':' => ';',
        '"' => '\'',
        '<' => ',',
        '>' => '.',
        '?' => '/',
        '~' => '`',
        _ => return None,
    })
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

    /// 经 libghostty 编成发给服务器的字节（默认模式，非 Kitty 键盘协议）。
    fn bytes(keystroke: Keystroke) -> Vec<u8> {
        let input = encode(&keystroke, false).expect("应由终端编码");
        let mut event = libghostty_vt::key::Event::new().unwrap();
        event
            .set_action(libghostty_vt::key::Action::Press)
            .set_key(input.key)
            .set_mods(input.mods)
            .set_consumed_mods(input.consumed_mods)
            .set_unshifted_codepoint(input.unshifted)
            .set_utf8(input.text);
        let mut out = Vec::new();
        libghostty_vt::key::Encoder::new().unwrap().encode_to_vec(&event, &mut out).unwrap();
        out
    }

    /// macOS 把 Shift 并进标点：Ctrl+Shift+- 送来的是 key「_」、不带 Shift、没有文字（gpui-pre-macos 的 parse_keystroke）。
    #[test]
    fn control_with_shifted_punctuation_reaches_the_shell() {
        assert_eq!(bytes(stroke("c", None, Modifiers::control())), [0x03]);
        assert_eq!(bytes(stroke("]", None, Modifiers::control())), [0x1d]);
        assert_eq!(bytes(stroke("_", None, Modifiers::control())), [0x1f], "Ctrl+_：readline / zsh 撤销");
        // Ctrl+@ 同 Ghostty 按 fixterms 编成 CSI u；要 NUL 用 Ctrl+2 / Ctrl+Space
        assert_eq!(bytes(stroke("@", None, Modifiers::control())), b"\x1b[64;5u");
        assert_eq!(bytes(stroke("2", None, Modifiers::control())), [0x00]);
        assert_eq!(bytes(stroke("^", None, Modifiers::control())), [0x1e]);
        assert_eq!(bytes(stroke("?", None, Modifiers::control())), [0x7f]);
        // 不按 Ctrl 的 Shift 标点还是文字，走输入法
        assert!(encode(&stroke("_", Some("_"), Modifiers::none()), false).is_none());
    }
}
