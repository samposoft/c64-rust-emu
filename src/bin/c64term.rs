// SPDX-License-Identifier: GPL-2.0-or-later
// Copyright (C) 2026 SampoSoft - Francesco Sampoli

//! c64term — the emulator inside the terminal, without a window.
//!
//! Usage: c64term [--fullscreen] [--no-turbo] [--halfblock] [--roms DIR] [--reu KB] [--no-drive] [file]
//!
//! By default it runs inline: it reserves below the prompt the rows the
//! screen needs (plus a status line) and on exit the last frame stays in the
//! terminal scrollback, with the prompt below it. With `--fullscreen` it uses
//! the whole window in the alternate screen, which disappears on exit.
//!
//! In terminals with the Kitty graphics protocol (Kitty, Ghostty, WezTerm)
//! the screen is at full resolution; in the others it is drawn with half
//! blocks. With the Kitty keyboard protocol every key press and release is
//! seen, as in the window; without it, a key stays pressed for a moment on
//! each character received (no held keys for games and joystick). Audio and
//! gamepads as in the window.

#[cfg(unix)]
fn main() {
    app::main();
}

#[cfg(not(unix))]
fn main() {
    eprintln!("c64term only works on macOS and Linux.");
    std::process::exit(1);
}

#[cfg(unix)]
mod app {
    use std::io::Write;
    use std::time::{Duration, Instant};

    use c64::frontend::printable_chars;
    use c64::frontend::session::{save_on_exit, Hotkey, MachineOptions, Session};
    use c64::frontend::term::input::{Event, Key, KeyKind, Parser};
    use c64::frontend::term::screen::{Area, BlockScreen, KittyScreen, Transfer};
    use c64::frontend::term::{self, Caps, Mode, TermSize, Terminal};
    use c64::keyboard::HostKey;

    const HELP: &str = "F12 quit · F11 reset · TAB joystick port · F5/F9 save/load state";
    /// How long a message stays visible in the status line.
    const MESSAGE_TIME: Duration = Duration::from_secs(5);
    /// An ESC with nothing following within this time was the ESC key.
    const ESC_TIMEOUT: Duration = Duration::from_millis(30);
    /// Without the Kitty protocol: how long a key stays pressed on each
    /// character received. It covers the auto-repeat interval, so a key held
    /// down stays pressed, but is below the KERNAL repeat delay.
    const LEGACY_HOLD: Duration = Duration::from_millis(150);

    enum Screen {
        Kitty(KittyScreen),
        Blocks(BlockScreen),
    }

    struct App {
        session: Session,
        term: Terminal,
        caps: Caps,
        mode: Mode,
        size: TermSize,
        /// Rows of the C64 screen; the status line is the one below.
        area: Area,
        /// The inline area has already been reserved.
        reserved: bool,
        screen: Screen,
        parser: Parser,
        /// Lone ESC received: when.
        esc_since: Option<Instant>,
        /// Host keys held (Kitty protocol), to be released if the terminal
        /// loses focus.
        held: Vec<HostKey>,
        /// Without the Kitty protocol: keys pressed and when to release them.
        timed: Vec<(HostKey, Instant)>,
        /// Status line on screen.
        status: String,
        /// Current terminal title (file in use, load turbo).
        title: String,
        quit: bool,
        out: Vec<u8>,
    }

    pub fn main() {
        let mut opts = MachineOptions::default();
        let mut turbo = true;
        let mut graphics = true;
        let mut mode = Mode::Inline;
        let mut args = std::env::args().skip(1);
        while let Some(a) = args.next() {
            match a.as_str() {
                "--no-turbo" => turbo = false,
                "--halfblock" => graphics = false,
                "--fullscreen" => mode = Mode::Fullscreen,
                "-h" | "--help" => {
                    println!("Usage: c64term [--fullscreen] [--no-turbo] [--halfblock] {}", MachineOptions::USAGE);
                    println!("  --fullscreen the whole terminal window (default: inline, below the prompt)");
                    println!("  --no-turbo   disk and tape loads at real speed (default: at maximum speed)");
                    println!("  --halfblock  draw with half blocks even in terminals with Kitty graphics");
                    println!("{}", MachineOptions::HELP);
                    println!("{HELP}");
                    return;
                }
                other => opts.parse_arg(other, &mut args),
            }
        }

        let session = match Session::new(&opts, turbo) {
            Ok(s) => s,
            Err(e) => { eprintln!("{e}"); std::process::exit(2); }
        };
        let mut term = match Terminal::open() {
            Ok(t) => t,
            Err(e) => { eprintln!("c64term needs a terminal: {e}"); std::process::exit(2); }
        };
        let caps = term.detect(graphics);
        let size = term.size();
        let area = Area::fullscreen(size);
        let screen = match caps.graphics {
            Some(t) => Screen::Kitty(KittyScreen::new(t, area)),
            None => Screen::Blocks(BlockScreen::new(caps.truecolor, area)),
        };
        let image_id = match &screen { Screen::Kitty(s) => s.id(), Screen::Blocks(_) => 0 };
        term.enter(&caps, mode, image_id);
        eprintln!("{}", describe(&caps));

        let mut app = App {
            session, term, caps, mode, size, area,
            reserved: false,
            screen,
            parser: Parser::new(),
            esc_since: None,
            held: Vec::new(),
            timed: Vec::new(),
            status: String::new(),
            title: String::new(),
            quit: false,
            out: Vec::new(),
        };
        app.run();

        if mode == Mode::Inline {
            // The last frame stays; the prompt returns in place of the status line
            let _ = write!(app.out, "\x1b[{};1H\x1b[0m\x1b[2K", app.status_row() + 1);
            let out = std::mem::take(&mut app.out);
            app.term.write(&out);
        }
        if let Screen::Kitty(s) = &app.screen { s.cleanup(); }
        for line in app.term.close() {
            eprintln!("{line}");
        }
        save_on_exit(&mut app.session.c64);
    }

    fn describe(caps: &Caps) -> String {
        let gfx = match caps.graphics {
            Some(Transfer::Shm) => "Kitty graphics (shared memory)",
            Some(Transfer::Direct) => "Kitty graphics (data in the sequence)",
            None if caps.truecolor => "24-bit half blocks",
            None => "256-color half blocks",
        };
        let keys = if caps.kitty_keyboard {
            "Kitty keyboard"
        } else {
            "legacy keyboard: no held keys, try Kitty or Ghostty"
        };
        format!("Terminal: {gfx}, {keys}")
    }

    impl App {
        /// Row of the status line (from 0), right below the area.
        fn status_row(&self) -> usize {
            self.area.row + self.area.rows
        }

        fn run(&mut self) {
            self.title = self.session.title();
            term::set_title(&mut self.out, &self.title);
            self.relayout();
            self.session.restart_clock();

            while !self.quit && !term::quit_requested() {
                if term::take_resized() {
                    self.relayout();
                }
                self.input();
                self.release_timed();
                if self.session.run_due_frames() {
                    let fb = self.session.screen();
                    match &mut self.screen {
                        Screen::Kitty(s) => s.draw(fb, &mut self.out),
                        Screen::Blocks(s) => s.draw(fb, &mut self.out),
                    }
                }
                self.update_status();
                if self.mode == Mode::Inline && !self.out.is_empty() {
                    // Cursor parked on the status line: if the terminal is
                    // resized and reflows its rows, the cursor follows and
                    // the area is found again from there
                    let _ = write!(self.out, "\x1b[{};1H", self.status_row() + 1);
                }
                let out = std::mem::take(&mut self.out);
                self.term.write(&out);
                self.out = out;
                self.out.clear();
            }
        }

        /// New (or initial) size: clear and redraw everything.
        fn relayout(&mut self) {
            self.size = self.term.size();
            self.area = match self.mode {
                Mode::Fullscreen => {
                    term::clear(&mut self.out, self.caps.truecolor);
                    Area::fullscreen(self.size)
                }
                Mode::Inline => self.reserve_inline(),
            };
            let area = self.area;
            match &mut self.screen {
                Screen::Kitty(s) => s.resize(area),
                Screen::Blocks(s) => s.resize(area),
            }
            self.status.clear();
            self.update_status_forced();
            let fb = self.session.screen();
            match &mut self.screen {
                Screen::Kitty(s) => s.draw(fb, &mut self.out),
                Screen::Blocks(s) => s.draw(fb, &mut self.out),
            }
        }

        /// Inline mode: reserves below the cursor the screen rows plus the
        /// status line, scrolling the terminal if there are not enough, and
        /// clears them with the terminal colors. After a resize the area is
        /// found again from the cursor, parked on the status line.
        fn reserve_inline(&mut self) -> Area {
            let mut area = Area::inline(self.size, self.caps.graphics.is_some(), 0);
            let total = area.rows + 1;

            let out = std::mem::take(&mut self.out);
            self.term.write(&out);
            let (cursor, events) = self.term.cursor_row(&mut self.parser);
            self.apply(events);
            let mut top = match cursor {
                Some(r) if self.reserved => r.saturating_sub(self.area.rows),
                Some(r) => r,
                // No reply: the area at the bottom, after making room
                None => {
                    self.out.extend_from_slice("\r\n".repeat(total).as_bytes());
                    self.size.rows.saturating_sub(total)
                }
            };
            self.reserved = true;

            // Clear the old content (including the frame at the old position)
            let _ = write!(self.out, "\x1b[{};1H\x1b[0m\x1b[J", top + 1);
            if self.caps.graphics.is_some() {
                // Only our image: the ones further up in the scrollback stay
                let id = match &self.screen { Screen::Kitty(s) => s.id(), Screen::Blocks(_) => 0 };
                let _ = write!(self.out, "\x1b_Ga=d,d=i,i={id},q=2\x1b\\");
            }
            // If the area does not fit below, scroll the terminal up
            let overflow = (top + total).saturating_sub(self.size.rows);
            if overflow > 0 {
                let _ = write!(self.out, "\x1b[{};1H{}", self.size.rows, "\n".repeat(overflow));
                top -= overflow.min(top);
            }
            area.row = top;
            term::clear_rows(&mut self.out, "\x1b[0m", top, total);
            area
        }

        /// Waits for input until the next frame (or the next release, or an
        /// ESC timeout) and applies it.
        fn input(&mut self) {
            let now = Instant::now();
            let mut until = if self.session.loading() { now } else { self.session.next_frame() };
            if let Some(t) = self.timed.iter().map(|&(_, t)| t).min() { until = until.min(t); }
            if let Some(t) = self.esc_since { until = until.min(t + ESC_TIMEOUT); }
            let bytes = self.term.read(Some(until.saturating_duration_since(now)));

            let events = if !bytes.is_empty() {
                self.parser.feed(&bytes)
            } else if self.esc_since.is_some_and(|t| t.elapsed() >= ESC_TIMEOUT) {
                self.parser.flush()
            } else {
                Vec::new()
            };
            self.esc_since = match self.esc_since {
                Some(t) if self.parser.pending_escape() => Some(t),
                _ if self.parser.pending_escape() => Some(Instant::now()),
                _ => None,
            };
            self.apply(events);
        }

        fn apply(&mut self, events: Vec<Event>) {
            // Without the Kitty protocol keys arrive as text: a single
            // space is the key (held, for joystick fire), several
            // characters together are pasted text, to be typed in order
            let texts = events.iter().filter(|e| matches!(e, Event::Text(_))).count();
            for ev in events {
                match ev {
                    Event::Text(' ') if texts == 1 && !self.caps.kitty_keyboard => {
                        self.legacy_key(HostKey::Space);
                    }
                    Event::Text(c) => self.session.type_char(c),
                    Event::Key(k) => match k.key {
                        Key::Host(hk) if !self.caps.kitty_keyboard => self.legacy_key(hk),
                        Key::Host(hk) => {
                            let pressed = k.kind != KeyKind::Release;
                            if pressed {
                                if !self.held.contains(&hk) { self.held.push(hk); }
                            } else {
                                self.held.retain(|&h| h != hk);
                            }
                            self.host_key(hk, pressed, k.kind == KeyKind::Repeat);
                        }
                        Key::Char(c) if k.kind != KeyKind::Release => {
                            if !k.text.is_empty() {
                                for ch in printable_chars(&k.text) { self.session.type_char(ch); }
                            } else if !k.mods.super_() {
                                // With Ctrl there is no text: the key's
                                // character, for colors with CTRL+digit
                                self.session.type_char(c);
                            }
                        }
                        Key::Char(_) | Key::Other => {}
                    },
                    Event::FocusOut => {
                        // The release would go to another window
                        for hk in std::mem::take(&mut self.held) {
                            self.host_key(hk, false, false);
                        }
                    }
                    _ => {}
                }
            }
        }

        fn host_key(&mut self, hk: HostKey, pressed: bool, repeat: bool) {
            match self.session.host_key(hk, pressed, repeat) {
                Some(Hotkey::Quit) => self.quit = true,
                // Fullscreen is up to the terminal
                Some(Hotkey::Fullscreen) | None => {}
            }
        }

        /// Key without release: pressed for `LEGACY_HOLD`, extended if it
        /// arrives again (auto-repeat).
        fn legacy_key(&mut self, hk: HostKey) {
            let release = Instant::now() + LEGACY_HOLD;
            if let Some(t) = self.timed.iter_mut().find(|(h, _)| *h == hk) {
                t.1 = release;
                return;
            }
            self.timed.push((hk, release));
            self.host_key(hk, true, false);
        }

        fn release_timed(&mut self) {
            let now = Instant::now();
            let due: Vec<HostKey> = self.timed.iter().filter(|&&(_, t)| t <= now).map(|&(h, _)| h).collect();
            self.timed.retain(|&(_, t)| t > now);
            for hk in due {
                self.host_key(hk, false, false);
            }
        }

        /// Message (or key reminder) on the left, Datasette, drive, joystick
        /// and speed status on the right.
        fn status_text(&self) -> String {
            let left = if self.session.loading() {
                "Loading (turbo)…".to_string()
            } else {
                match self.term.latest_message() {
                    Some((msg, at)) if at.elapsed() < MESSAGE_TIME => msg,
                    _ => HELP.into(),
                }
            };
            let right = self.session.status(&[]).compact();
            let width = self.size.cols.saturating_sub(1);
            let room = width.saturating_sub(right.chars().count() + 2);
            let left: String = left.chars().take(room).collect();
            let pad = width.saturating_sub(left.chars().count() + right.chars().count());
            format!("{left}{}{right}", " ".repeat(pad))
        }

        fn update_status(&mut self) {
            let title = self.session.title();
            if title != self.title {
                term::set_title(&mut self.out, &title);
                self.title = title;
            }
            if self.status_text() != self.status {
                self.update_status_forced();
            }
        }

        fn update_status_forced(&mut self) {
            self.status = self.status_text();
            let row = self.status_row();
            // Fullscreen: gray on black like the borders; inline: the
            // terminal colors, like the surrounding text
            let sgr = match self.mode {
                Mode::Fullscreen => format!("{}\x1b[37m", term::black_background(self.caps.truecolor)),
                Mode::Inline => "\x1b[0m".to_string(),
            };
            term::status_line(&mut self.out, row, self.size.cols, &sgr, &self.status);
        }
    }
}
