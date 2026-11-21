use super::TermMode;
use gpui_kit::Keystroke;

pub(crate) fn key_bytes_for_platform(
    stroke: &Keystroke,
    mode: TermMode,
    mac: bool,
) -> Option<Vec<u8>> {
    let key = stroke.key.as_str();
    let m = stroke.modifiers;
    if m.platform {
        return None;
    }
    let modifier = 1 + u8::from(m.shift) + 2 * u8::from(m.alt) + 4 * u8::from(m.control);
    let modified = modifier != 1;
    let prefix_alt = |byte| if m.alt { vec![0x1b, byte] } else { vec![byte] };
    let sequence = match key {
        "backspace" => return Some(prefix_alt(if m.control { 8 } else { 127 })),
        "enter" => return Some(prefix_alt(b'\r')),
        "escape" => return Some(prefix_alt(0x1b)),
        "tab" if m.shift => "\x1b[Z".into(),
        "tab" => "\t".into(),
        "up" | "down" | "right" | "left" | "home" | "end" | "f1" | "f2" | "f3" | "f4" => {
            let last = match key {
                "up" => 'A',
                "down" => 'B',
                "right" => 'C',
                "left" => 'D',
                "home" => 'H',
                "end" => 'F',
                "f1" => 'P',
                "f2" => 'Q',
                "f3" => 'R',
                _ => 'S',
            };
            if modified {
                format!("\x1b[1;{modifier}{last}")
            } else if key.starts_with('f') || mode.contains(TermMode::APP_CURSOR) {
                format!("\x1bO{last}")
            } else {
                format!("\x1b[{last}")
            }
        }
        "insert" | "delete" | "pageup" | "pagedown" | "f5" | "f6" | "f7" | "f8" | "f9" | "f10"
        | "f11" | "f12" => {
            let number = match key {
                "insert" if m.shift || m.control => return None,
                "insert" => 2,
                "delete" => 3,
                "pageup" => 5,
                "pagedown" => 6,
                "f5" => 15,
                "f6" => 17,
                "f7" => 18,
                "f8" => 19,
                "f9" => 20,
                "f10" => 21,
                "f11" => 23,
                _ => 24,
            };
            if modified && key != "insert" && (!key.starts_with("page") || m.control) {
                format!("\x1b[{number};{modifier}~")
            } else {
                format!("\x1b[{number}~")
            }
        }
        "add" | "subtract" | "multiply" | "divide" | "decimal" | "separator" | "equal" => {
            let (literal, application) = match key {
                "add" => ('+', 'k'),
                "subtract" => ('-', 'm'),
                "multiply" => ('*', 'j'),
                "divide" => ('/', 'o'),
                "decimal" => ('.', 'n'),
                "equal" => ('=', 'X'),
                _ => (',', 'l'),
            };
            if mode.contains(TermMode::APP_KEYPAD) {
                format!("\x1bO{application}")
            } else {
                literal.to_string()
            }
        }
        _ if m.control
            && m.alt
            && !mac
            && key.len() == 1
            && !key.as_bytes()[0].is_ascii_alphabetic() =>
        {
            let c = key.as_bytes()[0];
            let literal = if m.shift {
                let plain = b"0123456789-=[]\\;',./\x60";
                let shifted = b")!@#$%^&*(_+{}|:\"<>?~";
                plain.iter().position(|&k| k == c).map_or(c, |i| shifted[i])
            } else {
                c
            };
            return Some(vec![0x1b, literal]);
        }
        _ if m.control && (!m.alt || !mac) => {
            let byte = match key.as_bytes() {
                [b'a'..=b'z'] if !m.shift => key.as_bytes()[0] - b'a' + 1,
                b"space" | b"@" => 0,
                b"3" | b"[" => 27,
                b"4" | b"\\" => 28,
                b"5" | b"]" => 29,
                b"6" | b"^" => 30,
                b"7" | b"_" => 31,
                b"8" | b"?" => 127,
                _ => return None,
            };
            return Some(prefix_alt(byte));
        }
        _ if !mac && m.alt => {
            let text = stroke.key_char.as_deref().unwrap_or(key);
            if key == "space" {
                "\x1b ".into()
            } else if text.chars().count() == 1 {
                format!("\x1b{text}")
            } else {
                return None;
            }
        }
        _ => return None,
    };
    Some(sequence.into_bytes())
}

pub(crate) fn paste_bytes(text: &[u8], bracketed: bool) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(text.len() + if bracketed { 12 } else { 0 });
    if bracketed {
        bytes.extend_from_slice(b"\x1b[200~");
    }
    let mut previous = None;
    for &byte in text {
        if byte == b'\n' {
            if previous != Some(b'\r') {
                bytes.push(b'\r');
            }
        } else {
            bytes.push(byte);
        }
        previous = Some(byte);
    }
    if bracketed {
        bytes.extend_from_slice(b"\x1b[201~");
    }
    bytes
}
