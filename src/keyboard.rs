// SPDX-License-Identifier: GPL-2.0-or-later
// Copyright (C) 2026 SampoSoft - Francesco Sampoli
// The matrix solver (`KeyMatrix::connected`) follows VICE 3.10
// c64/c64cia1.c, Copyright (C) Andre Fachat, Ettore Perazzoli, Andreas
// Boose, Marco van den Heuvel. Ported to Rust and modified by SampoSoft in
// 2026; see CREDITS.md.

use std::collections::VecDeque;

/// Non-printable host keys relevant to the C64, independent of the GUI
/// toolkit. Printable characters go through `push_char` instead.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum HostKey {
    Backspace, Delete, Return, Tab, Escape, Space, Home,
    Left, Right, Up, Down,
    F1, F2, F3, F4, F5, F6, F7, F8, F9, F10, F11, F12,
    LShift, RShift, LCtrl, RCtrl, LAlt, RAlt,
}

/// C64 8×8 keyboard matrix.
///
/// A C64 key can be held down by several sources at once: RSHIFT by the
/// host's ← and ↑ (which on the C64 are SHIFT+CRSR), by the right SHIFT, by
/// the debugger's `key` command. For each position we count how many sources
/// hold it down, and the key is released only when none is left. On top of
/// the counts sit the characters of host keys held down (`char_down` /
/// `char_up`: pressed for as long as the host key, at least `CHAR_MIN_FRAMES`,
/// with the SHIFT state the character needs) and the character being typed
/// from the queue (`push_char`: pasted text, the debugger), pressed for one
/// frame.
pub struct KeyMatrix {
    /// State seen by the C64: `matrix[col]` = row bitmask, bit 0 = pressed.
    /// Derived from `holds`, `typed` and `typed_shift` on every change.
    pub matrix: [u8; 8],

    /// Sources holding down each key, `holds[col][row]`.
    holds: [[u8; 8]; 8],

    /// Host keys pressed (bit = `HostKey as u32`): a second press of the
    /// same key, or a release without a press, does not change the counts.
    host_down: u32,

    /// Keys of the character being typed, pressed for one frame.
    typed: Vec<(usize, usize)>,

    /// SHIFT forced by the character being typed: `Some(true)` LSHIFT down,
    /// `Some(false)` both SHIFTs up, `None` as the counts say.
    typed_shift: Option<bool>,

    // Queue of characters to "type" into the C64 (one char per frame)
    type_queue: VecDeque<char>,

    /// Characters of host keys held down, oldest first.
    held: Vec<HeldChar>,

    /// Frames since the last press or release of a host key.
    idle_frames: u16,
}

/// A printable character whose host key is held down.
struct HeldChar {
    /// Host key that produced it (released by `char_up` with the same id).
    id: u64,
    keys: Vec<(usize, usize)>,
    shift: bool,
    /// Frames it has been pressed for, and whether the host key is up.
    frames: u8,
    released: bool,
}

/// A character typed on the host stays pressed at least this many frames,
/// even if the host key is released sooner: programs that debounce the
/// keyboard, such as GEOS, want to see it in more than one scan.
pub const CHAR_MIN_FRAMES: u8 = 3;

impl KeyMatrix {
    pub fn new() -> Self {
        Self {
            matrix: [0xFF; 8],
            holds: [[0; 8]; 8],
            host_down: 0,
            typed: Vec::new(),
            typed_shift: None,
            type_queue: VecDeque::new(),
            held: Vec::new(),
            idle_frames: u16::MAX,
        }
    }

    /// One more source holds the key down.
    pub fn press(&mut self, col: usize, row: usize) {
        self.holds[col][row] = self.holds[col][row].saturating_add(1);
        self.rebuild();
    }

    /// One fewer source holds the key down.
    pub fn release(&mut self, col: usize, row: usize) {
        self.holds[col][row] = self.holds[col][row].saturating_sub(1);
        self.rebuild();
    }

    fn rebuild(&mut self) {
        for (col, rows) in self.holds.iter().enumerate() {
            self.matrix[col] = rows.iter().enumerate()
                .fold(0xFF, |m, (row, &n)| if n > 0 { m & !(1u8 << row) } else { m });
        }
        for h in &self.held {
            for &(col, row) in &h.keys {
                self.matrix[col] &= !(1u8 << row);
            }
        }
        // SHIFT as the last character needs, or as the queued one needs
        let shift = self.typed_shift.or(self.held.last().map(|h| h.shift));
        match shift {
            Some(true) => self.matrix[1] &= !(1 << 7),
            Some(false) => {
                self.matrix[1] |= 1 << 7;
                self.matrix[6] |= 1 << 4;
            }
            None => {}
        }
        for &(col, row) in &self.typed {
            self.matrix[col] &= !(1u8 << row);
        }
    }

    /// Some key is down.
    #[inline]
    pub fn any_pressed(&self) -> bool {
        self.matrix.iter().any(|&m| m != 0xFF)
    }

    /// PA lines with a key pressed on PB line `pb` (VICE: rev_keyarr).
    pub fn pa_of_pb(&self, pb: usize) -> u8 {
        (0..8).filter(|&c| self.matrix[c] & (1 << pb) == 0).fold(0, |m, c| m | 1 << c)
    }

    /// All the lines connected, through the keys pressed, to the PA lines
    /// `pa` and the PB lines `pb` (bit masks), as (PA, PB): a key joins its
    /// PA and PB line, and several keys join lines that no single key
    /// does (the ghost keys of the real matrix). VICE: matrix_activate_row
    /// and matrix_activate_column.
    pub fn connected(&self, pa: u8, pb: u8) -> (u8, u8) {
        let (mut a, mut b) = (pa, pb);
        loop {
            let mut nb = b;
            for (c, &m) in self.matrix.iter().enumerate() {
                if a & (1 << c) != 0 {
                    nb |= !m;
                }
            }
            let mut na = a;
            for (c, &m) in self.matrix.iter().enumerate() {
                if !m & nb != 0 {
                    na |= 1 << c;
                }
            }
            if (na, nb) == (a, b) {
                return (a, b);
            }
            (a, b) = (na, nb);
        }
    }

    // ── TextInput ─────────────────────────────────────────────────────────────

    /// True if the typing queue still has keys to press or release.
    pub fn typing(&self) -> bool {
        !self.type_queue.is_empty() || !self.typed.is_empty()
    }

    /// Queues a character to type, one per frame (pasted text, debugger).
    pub fn push_char(&mut self, c: char) {
        self.type_queue.push_back(c);
    }

    /// Host key `id` pressed, producing the printable character `c`: its
    /// C64 keys stay down until `char_up(id)`. A second press of a key
    /// already down (auto-repeat) changes nothing: the C64 repeats by itself.
    /// Unlike the typing queue, letters keep their case: a capital is
    /// SHIFT + letter, as on the real keyboard (a capital in GEOS and in
    /// lowercase mode, a graphic character in BASIC's uppercase mode).
    pub fn char_down(&mut self, id: u64, c: char) {
        self.idle_frames = 0;
        if self.held.iter().any(|h| h.id == id && !h.released) {
            return;
        }
        if let Some((keys, mut shift)) = char_to_c64(c) {
            if c.is_ascii_alphabetic() {
                shift = c.is_ascii_uppercase();
            }
            self.held.push(HeldChar { id, keys, shift, frames: 0, released: false });
            self.rebuild();
        }
    }

    /// Host key `id` released: its character is released too, once it has
    /// been down for `CHAR_MIN_FRAMES`.
    pub fn char_up(&mut self, id: u64) {
        self.idle_frames = 0;
        for h in self.held.iter_mut().filter(|h| h.id == id) {
            h.released = true;
        }
        self.drop_released();
    }

    /// Releases every character held by host keys (the window lost the
    /// focus: their key-up events will not arrive).
    pub fn release_chars(&mut self) {
        if !self.held.is_empty() {
            self.held.clear();
            self.rebuild();
        }
    }

    fn drop_released(&mut self) {
        let before = self.held.len();
        self.held.retain(|h| !(h.released && h.frames >= CHAR_MIN_FRAMES));
        if self.held.len() != before {
            self.rebuild();
        }
    }

    /// Called once per frame: releases the previous frame's keys and
    /// presses those of the next queued character.
    /// Someone is typing on the host: a key is held down, or one was
    /// pressed or released less than `frames` frames ago.
    pub fn host_typing(&self, frames: u16) -> bool {
        !self.held.is_empty() || self.host_down != 0 || self.idle_frames < frames
    }

    pub fn advance_typing(&mut self) {
        self.idle_frames = self.idle_frames.saturating_add(1);
        for h in &mut self.held {
            h.frames = h.frames.saturating_add(1);
        }
        self.drop_released();
        let released = std::mem::take(&mut self.typed);
        self.typed_shift = None;

        if let Some(&c) = self.type_queue.front() {
            let mapped = char_to_c64(c);
            // A key just released stays up for one frame: pressed again right
            // away, the keyboard scan would not see the release
            // ("LL" would become "L")
            let again = matches!(&mapped, Some((keys, _)) if keys.iter().any(|k| released.contains(k)));
            if !again {
                self.type_queue.pop_front();
                if let Some((keys, need_shift)) = mapped {
                    self.typed = keys;
                    self.typed_shift = Some(need_shift);
                }
            }
        }
        self.rebuild();
    }

    // ── KeyDown / KeyUp ───────────────────────────────────────────────────────

    /// Handles non-printable keys (control, cursors, function, shift, etc.).
    /// Printable characters go through `push_char`.
    pub fn update(&mut self, key: HostKey, pressed: bool) -> bool {
        let positions: &[(usize, usize)] = match key {
            // ── Control keys ──────────────────────────────────────────────────
            HostKey::Backspace | HostKey::Delete => &[(0, 0)],
            HostKey::Return                      => &[(0, 1)],
            HostKey::Right                       => &[(0, 2)],
            HostKey::F7                          => &[(0, 3)],
            HostKey::F1                          => &[(0, 4)],
            HostKey::F3                          => &[(0, 5)],
            // F5 and F9 used for save/load: not forwarded here
            HostKey::Down                        => &[(0, 7)],
            HostKey::Left  => &[(6, 4), (0, 2)], // RShift + Right = Left
            HostKey::Up    => &[(6, 4), (0, 7)], // RShift + Down  = Up
            HostKey::Home  => &[(6, 3)],
            HostKey::LShift => &[(1, 7)],
            HostKey::RShift => &[(6, 4)],
            HostKey::RCtrl => &[(7, 2)],                 // C64 Control (LCtrl = joy2 fire)
            HostKey::RAlt  => &[(7, 5)],                 // Commodore key (LAlt = joy2 fire)
            // RUN/STOP
            HostKey::Escape => &[(7, 7)],

            // Space: handled directly (held down for games)
            HostKey::Space => &[(7, 4)],

            // ── Keys mapped directly to the C64 ───────────────────────────────
            HostKey::F2 => &[(0, 4)], // F2 = equivalent to Shift+F1
            HostKey::F4 => &[(0, 5)],
            HostKey::F6 => &[(0, 6)],
            HostKey::F8 => &[(0, 3)],

            _ => return false,
        };

        let bit = 1u32 << key as u32;
        if pressed == (self.host_down & bit != 0) {
            return true;                // already pressed, or release without a press
        }
        self.host_down ^= bit;
        self.idle_frames = 0;
        for &(col, row) in positions {
            if pressed { self.press(col, row); } else { self.release(col, row); }
        }
        true
    }
}

// ── Named keys ───────────────────────────────────────────────────────────────

/// Keys by name, for the debugger's `key` and the `{name}` codes of `keys`:
/// matrix positions as (col, row).
pub const KEY_NAMES: &[(&str, &[(usize, usize)])] = &[
    ("return",    &[(0, 1)]),
    ("space",     &[(7, 4)]),
    ("runstop",   &[(7, 7)]),
    ("f1",        &[(0, 4)]),
    ("f3",        &[(0, 5)]),
    ("f5",        &[(0, 6)]),
    ("f7",        &[(0, 3)]),
    ("f2",        &[(1, 7), (0, 4)]),
    ("f4",        &[(1, 7), (0, 5)]),
    ("f6",        &[(1, 7), (0, 6)]),
    ("f8",        &[(1, 7), (0, 3)]),
    ("home",      &[(6, 3)]),
    ("clr",       &[(1, 7), (6, 3)]),
    ("del",       &[(0, 0)]),
    ("inst",      &[(1, 7), (0, 0)]),
    ("right",     &[(0, 2)]),
    ("down",      &[(0, 7)]),
    ("left",      &[(6, 4), (0, 2)]),
    ("up",        &[(6, 4), (0, 7)]),
    ("lshift",    &[(1, 7)]),
    ("rshift",    &[(6, 4)]),
    ("ctrl",      &[(7, 2)]),
    ("commodore", &[(7, 5)]),
];

/// In the typing queue the named keys are characters of the Unicode private
/// use area: `NAMED_KEY_BASE + i` presses `KEY_NAMES[i]`.
const NAMED_KEY_BASE: u32 = 0xE000;

/// Character that queues the named key `name` with `push_char`.
pub fn named_key_char(name: &str) -> Option<char> {
    let i = KEY_NAMES.iter().position(|(n, _)| n.eq_ignore_ascii_case(name))?;
    char::from_u32(NAMED_KEY_BASE + i as u32)
}

/// Matrix positions of a named key (`KEY_NAMES`) or of a single character,
/// with the SHIFT it needs.
pub fn key_positions(name: &str) -> Option<Vec<(usize, usize)>> {
    let lname = name.to_ascii_lowercase();
    if let Some((_, pos)) = KEY_NAMES.iter().find(|(n, _)| *n == lname) {
        return Some(pos.to_vec());
    }
    let mut chars = name.chars();
    let c = chars.next()?;
    if chars.next().is_some() { return None; }
    let (mut keys, shift) = char_to_c64(c)?;
    if shift { keys.insert(0, (1, 7)); }
    Some(keys)
}

// ── Character → C64 position map ─────────────────────────────────────────────
// Returns (list of (col,row), need_shift)

pub fn char_to_c64(c: char) -> Option<(Vec<(usize, usize)>, bool)> {
    // Named key from the typing queue: its SHIFT, if any, is among its keys
    // (pressed after the forced release of both SHIFTs)
    if let Some(&(_, keys)) = (c as u32).checked_sub(NAMED_KEY_BASE).and_then(|i| KEY_NAMES.get(i as usize)) {
        return Some((keys.to_vec(), false));
    }
    // Letters: the C64 matrix has only uppercase letters; the KERNAL handles case
    if c.is_ascii_alphabetic() {
        let l = c.to_ascii_lowercase();
        return Some((vec![letter_pos(l)?], false));
    }

    let (keys, shift) = match c {
        // ── Digits ────────────────────────────────────────────────────────────
        '0' => (vec![(4, 3)], false),
        '1' => (vec![(7, 0)], false),
        '2' => (vec![(7, 3)], false),
        '3' => (vec![(1, 0)], false),
        '4' => (vec![(1, 3)], false),
        '5' => (vec![(2, 0)], false),
        '6' => (vec![(2, 3)], false),
        '7' => (vec![(3, 0)], false),
        '8' => (vec![(3, 3)], false),
        '9' => (vec![(4, 0)], false),

        // ── Shifted digits (PETSCII shifted numbers) ─────────────────────────
        '!' => (vec![(7, 0)], true),  // Shift+1
        '"' => (vec![(7, 3)], true),  // Shift+2
        '#' => (vec![(1, 0)], true),  // Shift+3
        '$' => (vec![(1, 3)], true),  // Shift+4
        '%' => (vec![(2, 0)], true),  // Shift+5
        '&' => (vec![(2, 3)], true),  // Shift+6
        '\'' => (vec![(3, 0)], true), // Shift+7
        '(' => (vec![(3, 3)], true),  // Shift+8
        ')' => (vec![(4, 0)], true),  // Shift+9

        // ── C64 special keys (no shift needed in the matrix) ─────────────────
        // Space: in the GUI it arrives via KeyDown/KeyUp (main.rs filters TextInput),
        // here it serves the debugger (`keys "..."`) and the typing queue.
        ' '  => (vec![(7, 4)], false),
        '\r' | '\n' => (vec![(0, 1)], false),
        '*'  => (vec![(6, 1)], false),
        '+'  => (vec![(5, 0)], false),
        '-'  => (vec![(5, 3)], false),
        '='  => (vec![(6, 5)], false),
        '@'  => (vec![(5, 6)], false),
        ','  => (vec![(5, 7)], false),
        '.'  => (vec![(5, 4)], false),
        ':'  => (vec![(5, 5)], false),
        ';'  => (vec![(6, 2)], false),
        '/'  => (vec![(6, 7)], false),

        // ── C64 shifted keys ──────────────────────────────────────────────────
        '?'  => (vec![(6, 7)], true),  // Shift+/
        '<'  => (vec![(5, 7)], true),  // Shift+,
        '>'  => (vec![(5, 4)], true),  // Shift+.
        '['  => (vec![(6, 5)], true),  // Shift+= (PETSCII [)
        ']'  => (vec![(6, 2)], true),  // Shift+; (PETSCII ])

        _ => return None,
    };

    Some((keys, shift))
}

fn letter_pos(c: char) -> Option<(usize, usize)> {
    Some(match c {
        'a' => (1, 2), 'b' => (3, 4), 'c' => (2, 4), 'd' => (2, 2),
        'e' => (1, 6), 'f' => (2, 5), 'g' => (3, 2), 'h' => (3, 5),
        'i' => (4, 1), 'j' => (4, 2), 'k' => (4, 5), 'l' => (5, 2),
        'm' => (4, 4), 'n' => (4, 7), 'o' => (4, 6), 'p' => (5, 1),
        'q' => (7, 6), 'r' => (2, 1), 's' => (1, 5), 't' => (2, 6),
        'u' => (3, 6), 'v' => (3, 7), 'w' => (1, 1), 'x' => (2, 7),
        'y' => (3, 1), 'z' => (1, 4),
        _ => return None,
    })
}

// ── Saveable state ───────────────────────────────────────────────────────────

// Keys held down on the host are not part of the state: a reloaded state
// must not keep them pressed
crate::snapshot::impl_state!(KeyMatrix { matrix, holds, host_down, typed, typed_shift, type_queue } skip { held, idle_frames });

#[cfg(test)]
mod tests {
    use super::*;

    fn down(k: &KeyMatrix, col: usize, row: usize) -> bool {
        k.matrix[col] & (1 << row) == 0
    }

    #[test]
    fn held_arrow_stays_shifted() {
        let mut k = KeyMatrix::new();
        k.update(HostKey::Left, true);
        for _ in 0..3 {
            k.advance_typing();
            assert!(down(&k, 6, 4) && down(&k, 0, 2), "← = RSHIFT + CRSR →");
        }
        k.update(HostKey::Left, false);
        assert_eq!(k.matrix, [0xFF; 8]);
    }

    #[test]
    fn shift_shared_between_arrows() {
        let mut k = KeyMatrix::new();
        k.update(HostKey::Up, true);
        k.update(HostKey::Left, true);
        k.update(HostKey::Left, false);
        assert!(down(&k, 6, 4) && down(&k, 0, 7), "↑ must stay RSHIFT + CRSR ↓");
        assert!(!down(&k, 0, 2));
        k.update(HostKey::Up, true);            // double press: ignored
        k.update(HostKey::Up, false);
        assert_eq!(k.matrix, [0xFF; 8]);
        k.update(HostKey::Up, false);           // release without a press
        assert_eq!(k.matrix, [0xFF; 8]);
    }

    #[test]
    fn typing_forces_shift_for_one_frame() {
        let mut k = KeyMatrix::new();
        k.update(HostKey::RShift, true);
        k.push_char('a');
        k.push_char('!');
        k.advance_typing();                     // 'a': SHIFT up
        assert!(down(&k, 1, 2) && !down(&k, 6, 4) && !down(&k, 1, 7));
        k.advance_typing();                     // '!': LSHIFT + 1
        assert!(down(&k, 7, 0) && down(&k, 1, 7) && !down(&k, 1, 2));
        k.advance_typing();                     // queue empty: the held SHIFT is back
        assert!(down(&k, 6, 4) && !down(&k, 1, 7) && !down(&k, 7, 0));
        assert!(!k.typing());
    }

    #[test]
    fn held_char_stays_down_with_its_host_key() {
        let mut k = KeyMatrix::new();
        k.char_down(1, 'a');
        for _ in 0..10 {
            assert!(down(&k, 1, 2), "'a' held");
            k.advance_typing();
        }
        k.char_down(1, 'a');                    // auto-repeat: nothing changes
        k.char_up(1);
        assert!(!down(&k, 1, 2), "released with its host key");
    }

    #[test]
    fn quick_tap_lasts_min_frames() {
        let mut k = KeyMatrix::new();
        k.char_down(7, 'z');
        k.char_up(7);                           // released within the frame
        for _ in 0..CHAR_MIN_FRAMES {
            assert!(down(&k, 1, 4), "'z' still down");
            k.advance_typing();
        }
        assert!(!down(&k, 1, 4));
    }

    #[test]
    fn host_typing_lasts_after_the_last_key() {
        let mut k = KeyMatrix::new();
        assert!(!k.host_typing(25));
        k.char_down(1, 'a');
        assert!(k.host_typing(25));
        k.char_up(1);
        for _ in 0..24 { k.advance_typing(); }
        assert!(k.host_typing(25));
        k.advance_typing();
        assert!(!k.host_typing(25));
        k.update(HostKey::Return, true);
        assert!(k.host_typing(25), "held non-printable key");
    }

    #[test]
    fn held_capital_is_shift_letter() {
        let mut k = KeyMatrix::new();
        k.char_down(1, 'B');
        assert!(down(&k, 3, 4) && down(&k, 1, 7), "SHIFT + B");
        k.char_up(1);
        for _ in 0..CHAR_MIN_FRAMES { k.advance_typing(); }
        k.char_down(2, 'b');
        assert!(down(&k, 3, 4) && !down(&k, 1, 7) && !down(&k, 6, 4), "B alone");
    }

    #[test]
    fn held_char_forces_shift() {
        let mut k = KeyMatrix::new();
        k.char_down(2, '!');                    // SHIFT + 1
        assert!(down(&k, 7, 0) && down(&k, 1, 7));
        k.char_down(3, 'b');                    // the last one decides SHIFT
        assert!(down(&k, 3, 4) && !down(&k, 1, 7));
        k.char_up(3);
        for _ in 0..CHAR_MIN_FRAMES { k.advance_typing(); }
        assert!(down(&k, 1, 7) && !down(&k, 3, 4), "back to the SHIFT of '!'");
    }

    #[test]
    fn named_keys_in_the_typing_queue() {
        let mut k = KeyMatrix::new();
        k.update(HostKey::LShift, true);
        k.push_char(named_key_char("left").unwrap());
        k.push_char(named_key_char("F2").unwrap());
        k.push_char(named_key_char("return").unwrap());
        k.advance_typing();                     // CRSR left: RSHIFT + CRSR right, LSHIFT up
        assert!(down(&k, 6, 4) && down(&k, 0, 2) && !down(&k, 1, 7));
        k.advance_typing();                     // F2: LSHIFT + F1
        assert!(down(&k, 1, 7) && down(&k, 0, 4) && !down(&k, 6, 4) && !down(&k, 0, 2));
        k.advance_typing();                     // RETURN, without SHIFT
        assert!(down(&k, 0, 1) && !down(&k, 1, 7) && !down(&k, 0, 4));
        k.advance_typing();
        assert!(!k.typing() && down(&k, 1, 7), "the held SHIFT is back");
        assert_eq!(named_key_char("nokey"), None);
    }
}
