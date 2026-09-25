// SPDX-License-Identifier: GPL-2.0-or-later
// Copyright (C) 2026 SampoSoft - Francesco Sampoli

//! Terminal input parser: keys with the Kitty keyboard protocol (press,
//! repeat, release, bare modifiers), legacy sequences of terminals without
//! the protocol (xterm, Terminal.app), pasted text and replies to the
//! startup queries.
//!
//! Reference: <https://sw.kovidgoyal.net/kitty/keyboard-protocol/>.

use crate::keyboard::HostKey;

/// Recognized key.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Key {
    /// Key with a non-printable counterpart on the C64.
    Host(HostKey),
    /// Character key (unshifted: 'a' even with SHIFT held).
    Char(char),
    /// Key the C64 does not use (Cmd, PgUp, Caps Lock...).
    Other,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum KeyKind { Press, Repeat, Release }

/// Modifiers active during the event.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct Mods(u8);

impl Mods {
    pub fn shift(self) -> bool { self.0 & 1 != 0 }
    pub fn alt(self) -> bool { self.0 & 2 != 0 }
    pub fn ctrl(self) -> bool { self.0 & 4 != 0 }
    pub fn super_(self) -> bool { self.0 & 8 != 0 }
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub struct KeyEvent {
    pub key: Key,
    pub kind: KeyKind,
    pub mods: Mods,
    /// Text produced by the key, with the OS keyboard layout (Kitty protocol
    /// only, on press and repeat).
    pub text: String,
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Event {
    Key(KeyEvent),
    /// Character received as plain text: typed in a terminal without the
    /// Kitty protocol, or pasted.
    Text(char),
    FocusIn,
    FocusOut,
    /// Reply to `CSI ? u`: the Kitty keyboard protocol is supported.
    KeyboardFlags(u32),
    /// Reply to `CSI c` (Primary Device Attributes): ends the queries.
    DeviceAttributes,
    /// Graphics protocol reply: `ok` if the message is "OK".
    Graphics { id: u32, ok: bool },
    /// Reply to `CSI 6n`: cursor position, from 1.
    CursorPosition { row: usize, col: usize },
}

/// Stateful parser: buffers the bytes of a sequence split across two
/// reads.
#[derive(Default)]
pub struct Parser {
    buf: Vec<u8>,
    /// Expecting the reply to `CSI 6n`.
    expect_cursor: bool,
}

impl Parser {
    pub fn new() -> Self { Self::default() }

    /// The next `CSI row;column R` is the cursor position and not F3 with a
    /// modifier, which has the same form in legacy terminals.
    pub fn expect_cursor_report(&mut self) {
        self.expect_cursor = true;
    }

    /// Appends the bytes read and returns the complete events.
    pub fn feed(&mut self, bytes: &[u8]) -> Vec<Event> {
        self.buf.extend_from_slice(bytes);
        let mut events = Vec::new();
        let mut i = 0;
        while i < self.buf.len() {
            match parse_one(&self.buf[i..], &mut events, &mut self.expect_cursor) {
                Some(n) => i += n,
                None => break, // incomplete sequence: wait for the rest
            }
        }
        self.buf.drain(..i);
        events
    }

    /// A lone ESC at the end of the buffer: either the ESC key or the start
    /// of a sequence not yet arrived.
    pub fn pending_escape(&self) -> bool {
        self.buf == [0x1B]
    }

    /// To be called when nothing follows an ESC for a while: it was the ESC
    /// key. The rest of an incomplete sequence is discarded.
    pub fn flush(&mut self) -> Vec<Event> {
        let lone_esc = self.pending_escape();
        self.buf.clear();
        if lone_esc { vec![press(Key::Host(HostKey::Escape))] } else { Vec::new() }
    }
}

fn press(key: Key) -> Event {
    Event::Key(KeyEvent { key, kind: KeyKind::Press, mods: Mods::default(), text: String::new() })
}

/// Parses one event at the start of `b`; returns the bytes consumed, or
/// `None` if the sequence is incomplete.
fn parse_one(b: &[u8], events: &mut Vec<Event>, expect_cursor: &mut bool) -> Option<usize> {
    match b[0] {
        0x1B => parse_escape(b, events, expect_cursor),
        // Control keys of terminals without the Kitty protocol
        0x7F | 0x08 => { events.push(press(Key::Host(HostKey::Backspace))); Some(1) }
        b'\t' => { events.push(press(Key::Host(HostKey::Tab))); Some(1) }
        b'\r' | b'\n' => { events.push(Event::Text(b[0] as char)); Some(1) }
        c if c < 0x20 => Some(1), // other Ctrl+letter: not needed
        _ => parse_utf8(b, events),
    }
}

fn parse_utf8(b: &[u8], events: &mut Vec<Event>) -> Option<usize> {
    let len = match b[0] {
        0x00..=0x7F => 1,
        0xC0..=0xDF => 2,
        0xE0..=0xEF => 3,
        0xF0..=0xF7 => 4,
        _ => return Some(1), // invalid byte
    };
    if b.len() < len { return None; }
    match std::str::from_utf8(&b[..len]) {
        Ok(s) => events.extend(s.chars().map(Event::Text)),
        Err(_) => return Some(1),
    }
    Some(len)
}

fn parse_escape(b: &[u8], events: &mut Vec<Event>, expect_cursor: &mut bool) -> Option<usize> {
    let &kind = b.get(1)?;
    match kind {
        b'[' => parse_csi(b, events, expect_cursor),
        b'O' => {
            // SS3: F1-F4, arrows and Home/End in "application mode"
            let &f = b.get(2)?;
            if let Some(key) = legacy_final(f) { events.push(press(key)); }
            Some(3)
        }
        // APC (graphics), DCS, OSC: up to the String Terminator (ESC \ or BEL)
        b'_' | b'P' | b']' => {
            let end = find_st(&b[2..])?;
            if kind == b'_' {
                if let Some(ev) = parse_graphics_reply(&b[2..2 + end.0]) { events.push(ev); }
            }
            Some(2 + end.0 + end.1)
        }
        // ESC ESC or Alt+key in legacy terminals: counts as ESC
        _ => { events.push(press(Key::Host(HostKey::Escape))); Some(1) }
    }
}

/// Finds the String Terminator: (content length, terminator length).
fn find_st(b: &[u8]) -> Option<(usize, usize)> {
    for i in 0..b.len() {
        if b[i] == 0x07 { return Some((i, 1)); }
        if b[i] == 0x1B && b.get(i + 1) == Some(&b'\\') { return Some((i, 2)); }
    }
    None
}

/// `Gi=31;OK` or `Gi=31;ENOENT:...`.
fn parse_graphics_reply(b: &[u8]) -> Option<Event> {
    let s = std::str::from_utf8(b).ok()?.strip_prefix('G')?;
    let (keys, msg) = s.split_once(';')?;
    let id = keys.split(',').find_map(|kv| kv.strip_prefix("i="))?.parse().ok()?;
    Some(Event::Graphics { id, ok: msg == "OK" })
}

fn parse_csi(b: &[u8], events: &mut Vec<Event>, expect_cursor: &mut bool) -> Option<usize> {
    // Parameters 0x30-0x3F, intermediates 0x20-0x2F, final 0x40-0x7E
    let end = b[2..].iter().position(|c| (0x40..=0x7E).contains(c))? + 2;
    let len = end + 1;
    let fin = b[end];
    let mut params = &b[2..end];
    let private = match params.first() {
        Some(&p @ (b'?' | b'>' | b'<' | b'=')) => { params = &params[1..]; Some(p) }
        _ => None,
    };
    let Ok(params) = std::str::from_utf8(params) else { return Some(len) };
    // Parameters separated by ';', subparameters by ':'; empty = 0
    let p: Vec<Vec<u32>> = params.split(';')
        .map(|f| f.split(':').map(|v| v.parse().unwrap_or(0)).collect())
        .collect();
    let get = |i: usize, j: usize| p.get(i).and_then(|f| f.get(j)).copied().unwrap_or(0);

    match (private, fin) {
        (Some(b'?'), b'u') => events.push(Event::KeyboardFlags(get(0, 0))),
        (Some(b'?'), b'c') => events.push(Event::DeviceAttributes),
        (Some(_), _) => {}
        (None, b'R') if *expect_cursor && p.len() == 2 => {
            *expect_cursor = false;
            events.push(Event::CursorPosition { row: get(0, 0) as usize, col: get(1, 0) as usize });
        }
        (None, b'I') => events.push(Event::FocusIn),
        (None, b'O') => events.push(Event::FocusOut),
        (None, _) => {
            let key = match fin {
                b'u' => kitty_key(get(0, 0)),
                b'~' => match tilde_key(get(0, 0)) {
                    Some(k) => k,
                    None => return Some(len), // bracketed paste and the like
                },
                f => match legacy_final(f) {
                    Some(k) => k,
                    None => return Some(len),
                },
            };
            let mods = Mods(get(1, 0).saturating_sub(1) as u8);
            let kind = match get(1, 1) {
                2 => KeyKind::Repeat,
                3 => KeyKind::Release,
                _ => KeyKind::Press,
            };
            let text = p.get(2).map_or_else(String::new, |f| {
                f.iter().filter_map(|&c| char::from_u32(c)).collect()
            });
            events.push(Event::Key(KeyEvent { key, kind, mods, text }));
        }
    }
    Some(len)
}

/// Final letter of the `CSI 1;mod X` and `SS3 X` sequences.
fn legacy_final(f: u8) -> Option<Key> {
    Some(Key::Host(match f {
        b'A' => HostKey::Up,
        b'B' => HostKey::Down,
        b'C' => HostKey::Right,
        b'D' => HostKey::Left,
        b'H' => HostKey::Home,
        b'P' => HostKey::F1,
        b'Q' => HostKey::F2,
        b'R' => HostKey::F3,
        b'S' => HostKey::F4,
        b'F' | b'E' => return Some(Key::Other), // End, keypad center
        _ => return None,
    }))
}

/// Number of the `CSI n ~` sequences.
fn tilde_key(n: u32) -> Option<Key> {
    Some(Key::Host(match n {
        1 | 7 => HostKey::Home,
        3 => HostKey::Delete,
        11 => HostKey::F1,
        12 => HostKey::F2,
        13 => HostKey::F3,
        14 => HostKey::F4,
        15 => HostKey::F5,
        17 => HostKey::F6,
        18 => HostKey::F7,
        19 => HostKey::F8,
        20 => HostKey::F9,
        21 => HostKey::F10,
        23 => HostKey::F11,
        24 => HostKey::F12,
        2 | 4 | 5 | 6 | 8 => return Some(Key::Other), // Ins, End, PgUp, PgDn
        _ => return None,
    }))
}

/// Key codes of `CSI code u`: Unicode for characters, private use area
/// from 57344 for keypad and modifiers.
fn kitty_key(code: u32) -> Key {
    Key::Host(match code {
        27 => HostKey::Escape,
        13 | 57414 => HostKey::Return, // Enter and keypad Enter
        9 => HostKey::Tab,
        127 | 8 => HostKey::Backspace,
        32 => HostKey::Space,
        57441 => HostKey::LShift,
        57442 => HostKey::LCtrl,
        57443 => HostKey::LAlt,
        57447 => HostKey::RShift,
        57448 => HostKey::RCtrl,
        57449 => HostKey::RAlt,
        // Keypad: digits, point, operators
        57399..=57408 => return Key::Char(char::from(b'0' + (code - 57399) as u8)),
        57409 => return Key::Char('.'),
        57410 => return Key::Char('/'),
        57411 => return Key::Char('*'),
        57412 => return Key::Char('-'),
        57413 => return Key::Char('+'),
        57415 => return Key::Char('='),
        57344..=63743 => return Key::Other,
        c => match char::from_u32(c) {
            Some(ch) if !ch.is_control() => return Key::Char(ch),
            _ => return Key::Other,
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(key: Key, kind: KeyKind, mods: u8, text: &str) -> Event {
        Event::Key(KeyEvent { key, kind, mods: Mods(mods), text: text.into() })
    }

    #[test]
    fn kitty_press_repeat_release() {
        let mut p = Parser::new();
        // 'a' pressed (text "a"), repeated, released
        let ev = p.feed(b"\x1b[97;;97u\x1b[97;1:2;97u\x1b[97;1:3u");
        assert_eq!(ev, vec![
            key(Key::Char('a'), KeyKind::Press, 0, "a"),
            key(Key::Char('a'), KeyKind::Repeat, 0, "a"),
            key(Key::Char('a'), KeyKind::Release, 0, ""),
        ]);
    }

    #[test]
    fn kitty_shift_text_and_modifier_keys() {
        let mut p = Parser::new();
        // Left SHIFT pressed, then SHIFT+1 = "!", then SHIFT released
        let ev = p.feed(b"\x1b[57441;2u\x1b[49;2;33u\x1b[57441;1:3u");
        assert_eq!(ev, vec![
            key(Key::Host(HostKey::LShift), KeyKind::Press, 1, ""),
            key(Key::Char('1'), KeyKind::Press, 1, "!"),
            key(Key::Host(HostKey::LShift), KeyKind::Release, 0, ""),
        ]);
        let Event::Key(k) = &ev[1] else { unreachable!() };
        assert!(k.mods.shift() && !k.mods.ctrl());
    }

    #[test]
    fn kitty_functional_keys() {
        let mut p = Parser::new();
        let ev = p.feed(b"\x1b[13u\x1b[1;1:3A\x1b[24~\x1b[13~\x1b[1;5P\x1b[57414u\x1b[57400u");
        let keys: Vec<(Key, KeyKind)> = ev.iter().map(|e| match e {
            Event::Key(k) => (k.key, k.kind),
            _ => panic!("{e:?}"),
        }).collect();
        assert_eq!(keys, vec![
            (Key::Host(HostKey::Return), KeyKind::Press),
            (Key::Host(HostKey::Up), KeyKind::Release),
            (Key::Host(HostKey::F12), KeyKind::Press),
            (Key::Host(HostKey::F3), KeyKind::Press),
            (Key::Host(HostKey::F1), KeyKind::Press),
            (Key::Host(HostKey::Return), KeyKind::Press),
            (Key::Char('1'), KeyKind::Press),
        ]);
    }

    #[test]
    fn legacy_keys_and_text() {
        let mut p = Parser::new();
        let ev = p.feed(b"ab\r\x7f\t\x1bOP\x1b[15~\x1b[D\xc3\xa8");
        assert_eq!(ev, vec![
            Event::Text('a'), Event::Text('b'), Event::Text('\r'),
            key(Key::Host(HostKey::Backspace), KeyKind::Press, 0, ""),
            key(Key::Host(HostKey::Tab), KeyKind::Press, 0, ""),
            key(Key::Host(HostKey::F1), KeyKind::Press, 0, ""),
            key(Key::Host(HostKey::F5), KeyKind::Press, 0, ""),
            key(Key::Host(HostKey::Left), KeyKind::Press, 0, ""),
            Event::Text('è'),
        ]);
    }

    #[test]
    fn split_sequences_wait_for_the_rest() {
        let mut p = Parser::new();
        assert!(p.feed(b"\x1b[57441").is_empty());
        assert_eq!(p.feed(b";2u"), vec![key(Key::Host(HostKey::LShift), KeyKind::Press, 1, "")]);
        assert!(p.feed(b"\xc3").is_empty());
        assert_eq!(p.feed(b"\xa8"), vec![Event::Text('è')]);
    }

    #[test]
    fn lone_escape_is_the_escape_key() {
        let mut p = Parser::new();
        assert!(p.feed(b"\x1b").is_empty());
        assert!(p.pending_escape());
        assert_eq!(p.flush(), vec![key(Key::Host(HostKey::Escape), KeyKind::Press, 0, "")]);
        assert!(!p.pending_escape());
    }

    #[test]
    fn cursor_report_only_when_expected() {
        let mut p = Parser::new();
        // Without a request it is Ctrl+F3 from a legacy terminal
        assert_eq!(p.feed(b"\x1b[1;5R"), vec![key(Key::Host(HostKey::F3), KeyKind::Press, 4, "")]);
        p.expect_cursor_report();
        assert_eq!(p.feed(b"\x1b[1;5R\x1b[1;5R"), vec![
            Event::CursorPosition { row: 1, col: 5 },
            key(Key::Host(HostKey::F3), KeyKind::Press, 4, ""),
        ]);
    }

    #[test]
    fn startup_replies() {
        let mut p = Parser::new();
        let ev = p.feed(b"\x1b_Gi=31;OK\x1b\\\x1b_Gi=32;ENOENT:no such file\x1b\\\x1b[?0u\x1b[?62;22c\x1b[I\x1b[O");
        assert_eq!(ev, vec![
            Event::Graphics { id: 31, ok: true },
            Event::Graphics { id: 32, ok: false },
            Event::KeyboardFlags(0),
            Event::DeviceAttributes,
            Event::FocusIn,
            Event::FocusOut,
        ]);
    }
}
