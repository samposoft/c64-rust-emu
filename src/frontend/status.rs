// SPDX-License-Identifier: GPL-2.0-or-later
// Copyright (C) 2026 SampoSoft - Francesco Sampoli

//! Frontend status bar, like VICE's: Datasette (clickable buttons,
//! counter, motor), drives 8 and 9 (LED, track, motor), joysticks, paddles
//! and mouse,
//! configuration, speed, inserted media and the last message.
//!
//! `Status` collects the state from the machine and the frontend; `Bar`
//! draws it in a strip as wide as the VIC screen and `BAR_HEIGHT` tall,
//! with the C64 font (the character ROM, lowercase set) and icons drawn
//! here. The window puts it under the screen at the same scale; `c64term`
//! uses its text version (`compact`).

use std::time::{Duration, Instant};

use super::JoyPort;
use crate::c64::C64;
use crate::ctrlport::Device;
use crate::tape::Button;
use crate::vic::WIDTH;

/// Bar height in C64 pixels: three rows.
pub const BAR_HEIGHT: usize = 32;

/// State shown by the bar.
#[derive(Clone)]
pub struct Status {
    pub tape_button: Button,
    pub tape_present: bool,
    pub tape_motor: bool,
    pub tape_moving: bool,
    pub tape_counter: u32,
    pub tape_name: String,
    /// Recordings not yet written to the file.
    pub tape_changed: bool,
    /// Emulated drives 8 and 9 (drive 8 None: disks through the KERNAL
    /// trap).
    pub drives: [Option<DriveStatus>; crate::drive::UNITS],
    /// Disk of each drive.
    pub disk_names: [String; crate::drive::UNITS],
    /// Drive writes not yet saved to the disk file.
    pub disk_changed: [bool; crate::drive::UNITS],
    pub cart_name: String,
    /// SID, REU: one word each.
    pub config: String,
    /// Joystick ports as the C64 sees them (bit at 0 = pressed).
    pub joy: [u8; 2],
    /// Devices in the control ports.
    pub ports: [Device; 2],
    /// Buttons of the paddles and mice (bit 0 left, bit 1 right).
    pub analog_buttons: u8,
    /// Host mouse captured by the window for the paddles and the mouse.
    pub mouse_captured: bool,
    /// Port driven by the keyboard.
    pub keyboard_port: JoyPort,
    pub gamepad: bool,
    /// Emulated frames per second (50 PAL, 60 NTSC at real speed).
    pub fps: f32,
    /// Frames per second at real speed: 50 PAL, 60 NTSC.
    pub nominal_fps: f32,
    /// Load turbo (or maximum speed in the debugger).
    pub turbo: bool,
    /// Emulation paused (debugger).
    pub paused: bool,
    /// Current medium and number of media from the command line (F8).
    pub media: Option<(usize, usize)>,
    /// Last message and when it arrived.
    pub message: Option<(String, Instant)>,
    /// Key reminders, in alternating pages.
    pub help: &'static [&'static str],
    /// Emulated frames, for the animations.
    pub frame: u64,
}

#[derive(Clone, Copy)]
pub struct DriveStatus {
    pub led: bool,
    pub motor: bool,
    /// Half-track under the head, counted on both sides as in a D71: side
    /// 1 of a 1571 starts at track 36.
    pub half_track: usize,
    pub disk: bool,
}

impl Status {
    /// Machine state; keyboard port, gamepad, speed, media, message and
    /// reminders are filled in by the frontend.
    pub fn of(c64: &C64) -> Status {
        let tape = &c64.bus.tape;
        let drives = c64.bus.drives.each_ref().map(|d| d.as_ref().map(|d| DriveStatus {
            led: d.bus.mech.led,
            motor: d.bus.mech.motor,
            half_track: d.bus.mech.half_track + d.bus.mech.side as usize * 70,
            disk: d.bus.mech.disk.is_some(),
        }));
        let disk_changed = c64.bus.drives.each_ref()
            .map(|d| d.as_ref().and_then(|d| d.bus.mech.disk.as_ref()).is_some_and(|d| d.dirty));
        let disk_names = std::array::from_fn(|i| {
            let unit = crate::drive::FIRST_UNIT + i as u8;
            // Without drive 8 the trap's disk
            let shown = if i == 0 { drives[0].is_none_or(|d| d.disk) } else { drives[i].is_some_and(|d| d.disk) };
            if shown { file_name(c64.disk_path(unit)) } else { String::new() }
        });
        let sid = &c64.bus.sid;
        let mut config = if sid.digiboost() { "8580D".to_string() } else { sid.model().name().to_string() };
        if c64.bus.sid2.is_some() {
            config += "x2";
        }
        use crate::vic::Chip;
        config += match c64.chip() {
            Chip::Mos6569 => "",
            Chip::Mos6569R1 => " 6569R1",
            Chip::Mos8565 => " 8565",
            Chip::Mos6567R8 => " NTSC",
            Chip::Mos8562 => " NTSC 8562",
            Chip::Mos6567R56A => " NTSC R56A",
            Chip::Mos6572 => " PAL-N",
        };
        if let Some(reu) = &c64.bus.reu {
            config += &format!(" REU{}", if reu.kb() >= 1024 { format!("{}M", reu.kb() / 1024) } else { reu.kb().to_string() });
        }
        Status {
            tape_button: tape.button(),
            tape_present: tape.image.is_some() || c64.t64().is_some(),
            tape_motor: tape.motor(),
            tape_moving: tape.moving(),
            tape_counter: tape.counter(),
            tape_name: file_name(c64.tape_path()),
            tape_changed: tape.image.as_ref().is_some_and(|t| t.changed),
            drives,
            disk_names,
            disk_changed,
            cart_name: c64.bus.cart.as_ref().map_or(String::new(), |c| c.name.trim().to_string()),
            config,
            joy: [c64.bus.joy1 & c64.bus.ctrl.lines(0, c64.bus.cycle), c64.bus.joy2 & c64.bus.ctrl.lines(1, c64.bus.cycle)],
            ports: c64.bus.ctrl.devices,
            analog_buttons: c64.bus.ctrl.buttons,
            mouse_captured: false,
            keyboard_port: JoyPort::Two,
            gamepad: false,
            fps: c64.standard().mains_hz() as f32,
            nominal_fps: c64.standard().mains_hz() as f32,
            turbo: false,
            paused: false,
            media: None,
            message: None,
            help: &[],
            frame: c64.frame_count,
        }
    }

    /// Text version for the terminal: Datasette button and counter, drive
    /// LED and track, joystick port, speed.
    pub fn compact(&self) -> String {
        let mut parts = Vec::new();
        if self.tape_present || self.tape_button != Button::Stop {
            let icon = match self.tape_button {
                Button::Stop => "■",
                Button::Play => "▶",
                Button::Forward => "▶▶",
                Button::Rewind => "◀◀",
                Button::Record => "●",
            };
            parts.push(format!("{icon} {:03}{}", self.tape_counter, if self.tape_motor { " ⟳" } else { "" }));
        }
        for (i, d) in self.drives.iter().enumerate() {
            if let Some(d) = d {
                parts.push(format!("{}:{} {}", 8 + i, if d.led { "●" } else { "○" }, track(d.half_track)));
            }
        }
        parts.push(match self.keyboard_port {
            JoyPort::One => "J1".into(),
            JoyPort::Two => "J2".into(),
            JoyPort::None => "J-".into(),
        });
        for (i, d) in self.ports.iter().enumerate() {
            match d {
                Device::Joystick => {}
                Device::Paddles => parts.push(format!("P{}", i + 1)),
                Device::Mouse => parts.push(format!("M{}", i + 1)),
                Device::JoyMouse => parts.push(format!("MJ{}", i + 1)),
                _ => parts.push(format!("L{}", i + 1)),
            }
        }
        parts.push(self.speed_text());
        parts.join(" · ")
    }

    fn speed_text(&self) -> String {
        let pct = (self.fps * 100.0 / self.nominal_fps).round() as u32;
        if self.paused {
            "PAUSED".into()
        } else if self.turbo {
            format!("TURBO {pct}%")
        } else {
            format!("{pct}% {:.0}fps", self.fps)
        }
    }

    /// Names of the inserted media (second row of the bar), shortened to
    /// fit in `width` characters.
    fn media_items(&self) -> Vec<(char, String)> {
        let mut items = Vec::new();
        if !self.tape_name.is_empty() {
            items.push(('T', format!("{}{}", self.tape_name, if self.tape_changed { "*" } else { "" })));
        }
        // With drive 9 the disks are labelled with their unit
        let two = self.drives[1].is_some();
        for (i, name) in self.disk_names.iter().enumerate() {
            if !name.is_empty() {
                let label = if two { (b'8' + i as u8) as char } else { 'D' };
                items.push((label, format!("{name}{}", if self.disk_changed[i] { "*" } else { "" })));
            }
        }
        if !self.cart_name.is_empty() {
            items.push(('C', self.cart_name.clone()));
        }
        items
    }
}

/// Drive track: "18.0", "18.5" for half tracks.
fn track(half_track: usize) -> String {
    format!("{:2}.{}", half_track / 2, if half_track % 2 == 1 { 5 } else { 0 })
}

/// File name without the directory.
fn file_name(path: &str) -> String {
    std::path::Path::new(path).file_name().map_or(String::new(), |n| n.to_string_lossy().into_owned())
}

/// Shortens `s` to `max` characters by cutting out the middle, so the
/// start of the name and the extension remain.
fn shorten(s: &str, max: usize) -> String {
    let chars: Vec<char> = s.chars().collect();
    if chars.len() <= max {
        return s.to_string();
    }
    if max < 5 {
        return chars[..max].iter().collect();
    }
    let tail = (max - 2) / 3;
    let head = max - 2 - tail;
    chars[..head].iter().chain(['.', '.'].iter()).chain(chars[chars.len() - tail..].iter()).collect()
}

// ── Speed ────────────────────────────────────────────────────────────────────

/// Emulated frames per second, measured over half a second.
pub struct SpeedMeter {
    since: Instant,
    frames: u32,
    /// None until the first measure.
    fps: Option<f32>,
}

impl SpeedMeter {
    pub fn new() -> Self {
        Self { since: Instant::now(), frames: 0, fps: None }
    }

    /// To be called on every emulated frame.
    pub fn frame(&mut self) {
        self.frames += 1;
        let t = self.since.elapsed();
        if t >= Duration::from_millis(500) {
            self.fps = Some(self.frames as f32 / t.as_secs_f32());
            self.frames = 0;
            self.since = Instant::now();
        }
    }

    /// Frames per second measured, None before the first half second.
    pub fn fps(&self) -> Option<f32> {
        self.fps
    }
}

impl Default for SpeedMeter {
    fn default() -> Self {
        Self::new()
    }
}

// ── Clicks ───────────────────────────────────────────────────────────────────

/// What can be clicked in the bar.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Click {
    /// Datasette button.
    Tape(Button),
    /// Counter: back to 000.
    ResetCounter,
}

/// Datasette buttons in the order they have on the Datasette.
const TAPE_BUTTONS: [Button; 5] = [Button::Record, Button::Play, Button::Rewind, Button::Forward, Button::Stop];

/// Element under the point (`x`, `y`) of the bar, in C64 pixels.
pub fn click_at(x: usize, y: usize) -> Option<Click> {
    if !(ROW1..ROW1 + 9).contains(&y) {
        return None;
    }
    if x >= TAPE_X && x < TAPE_X + TAPE_BUTTONS.len() * BUTTON_STEP && (x - TAPE_X) % BUTTON_STEP < 9 {
        return Some(Click::Tape(TAPE_BUTTONS[(x - TAPE_X) / BUTTON_STEP]));
    }
    if (COUNTER_X..COUNTER_X + 24).contains(&x) {
        return Some(Click::ResetCounter);
    }
    None
}

/// Performs a click on the bar.
pub fn apply_click(c64: &mut C64, click: Click) {
    match click {
        Click::Tape(Button::Record) if c64.bus.tape.image.is_none() => {
            crate::notice!("Datasette: RECORD cannot be pressed without a tape");
        }
        Click::Tape(b) => {
            let clk = c64.bus.cycle;
            c64.bus.tape.press(b, clk);
            crate::notice!("Datasette: {} pressed", b.name());
        }
        Click::ResetCounter => {
            c64.bus.tape.reset_counter();
            crate::notice!("Datasette: counter reset");
        }
    }
}

// ── Drawing ──────────────────────────────────────────────────────────────────

// Rows and columns of the bar, in C64 pixels
const ROW1: usize = 2;
const ROW2: usize = 13;
const ROW3: usize = 23;
const TAPE_X: usize = 3;
const BUTTON_STEP: usize = 10;
const COUNTER_X: usize = 56;
const REEL_X: usize = 83;
const DRIVE_X: usize = 99;
const JOY_X: usize = 168;
const SPEED_SEPARATOR_X: usize = 226;
const TEXT_X: usize = 4;
/// Characters that fit in a text row.
const TEXT_COLS: usize = (WIDTH - 2 * TEXT_X) / 8;

const BG: u32 = 0x1C1C1C;
const EDGE: u32 = 0x3C3C3C;
const TEXT: u32 = 0xC8C8C8;
const DIM: u32 = 0x686868;
const BRIGHT: u32 = 0xFFFFFF;
const BUTTON: u32 = 0x363636;
const BUTTON_DOWN: u32 = 0x7A7A7A;
const RED: u32 = 0xE83C3C;
const RED_DIM: u32 = 0x5A2020;
const LED_OFF: u32 = 0x3A1414;
const LED_ON: u32 = 0xFF2A2A;
const GREEN: u32 = 0x4CD44C;
const AMBER: u32 = 0xF0B030;

/// Messages: still for this long before scrolling, then one pixel every
/// `MS_PER_PIXEL`, and visible for `MESSAGE_SHOW_MS` plus the scrolling.
const MESSAGE_PAUSE_MS: u128 = 1000;
const MESSAGE_SHOW_MS: u64 = 4000;
const MS_PER_PIXEL: u128 = 12;

/// How long a message stays on the third row.
fn message_duration(msg: &str) -> Duration {
    let extra = msg.chars().count().saturating_sub(TEXT_COLS) as u64 * 8;
    Duration::from_millis(MESSAGE_SHOW_MS + extra * MS_PER_PIXEL as u64)
}

/// Rendered status bar: 0RGB pixels, WIDTH × BAR_HEIGHT.
pub struct Bar {
    /// Lowercase set of the character ROM (256 characters of 8 bytes).
    font: Vec<u8>,
    pixels: Vec<u32>,
}

impl Bar {
    /// `char_rom`: the C64 character ROM (4 KB); if it is missing the text
    /// is not shown, the icons are.
    pub fn new(char_rom: &[u8]) -> Self {
        let font = if char_rom.len() >= 0x1000 { char_rom[0x800..0x1000].to_vec() } else { vec![0; 0x800] };
        Self { font, pixels: vec![BG; WIDTH * BAR_HEIGHT] }
    }

    pub fn pixels(&self) -> &[u32] {
        &self.pixels
    }

    /// Redraws the bar.
    pub fn render(&mut self, s: &Status) {
        self.pixels.fill(BG);
        self.rect(0, 0, WIDTH, 1, EDGE);
        self.render_tape(s);
        self.rect(DRIVE_X - 5, ROW1, 1, 9, EDGE);
        self.render_drive(s);
        self.rect(JOY_X - 5, ROW1, 1, 9, EDGE);
        self.render_joysticks(s);
        self.rect(SPEED_SEPARATOR_X, ROW1, 1, 9, EDGE);
        let speed = s.speed_text();
        let color = if s.paused { AMBER } else if s.turbo { GREEN } else { TEXT };
        self.text(WIDTH - 3 - 8 * speed.chars().count(), ROW1 + 1, &speed, color, (0, WIDTH));
        self.render_media(s);
        self.render_message(s);
    }

    fn render_tape(&mut self, s: &Status) {
        for (i, &b) in TAPE_BUTTONS.iter().enumerate() {
            let x = TAPE_X + i * BUTTON_STEP;
            let down = s.tape_button == b && b != Button::Stop;
            self.rect(x, ROW1, 9, 9, if down { BUTTON_DOWN } else { BUTTON });
            let color = match (b, down) {
                (Button::Record, _) if !s.tape_present => RED_DIM,
                (Button::Record, _) => RED,
                (_, true) => BRIGHT,
                _ if s.tape_present => TEXT,
                _ => DIM,
            };
            let icon: &[&str] = match b {
                Button::Record => &[".xxx.", "xxxxx", "xxxxx", "xxxxx", ".xxx."],
                Button::Play => &["x....", "xxx..", "xxxxx", "xxx..", "x...."],
                Button::Rewind => &["..x..x", ".xx.xx", "xxxxxx", ".xx.xx", "..x..x"],
                Button::Forward => &["x..x..", "xx.xx.", "xxxxxx", "xx.xx.", "x..x.."],
                Button::Stop => &["xxxxx", "xxxxx", "xxxxx", "xxxxx", "xxxxx"],
            };
            let w = icon[0].len();
            self.icon(x + (9 - w) / 2, ROW1 + 2, icon, color);
        }
        let counter = format!("{:03}", s.tape_counter);
        self.text(COUNTER_X, ROW1 + 1, &counter, if s.tape_present { BRIGHT } else { DIM }, (0, WIDTH));
        // Reel: spins while the tape moves, green with the motor on
        let color = if s.tape_motor { GREEN } else if s.tape_present { TEXT } else { DIM };
        self.reel(REEL_X, ROW1 + 1, color, s.tape_moving.then_some(s.frame / 4));
    }

    fn render_drive(&mut self, s: &Status) {
        if s.drives[1].is_some() {
            // Two drives: number, LED and track of each, the track green
            // while the motor turns
            for (i, d) in s.drives.iter().enumerate() {
                let x = DRIVE_X - 3 + i * 33;
                let Some(d) = d else {
                    self.text(x, ROW1 + 1, "8", DIM, (0, WIDTH));
                    continue;
                };
                self.text(x, ROW1 + 1, if i == 0 { "8" } else { "9" }, TEXT, (0, WIDTH));
                self.rect(x + 8, ROW1 + 3, 5, 4, if d.led { LED_ON } else { LED_OFF });
                let color = if d.motor { GREEN } else if d.disk { TEXT } else { DIM };
                self.text(x + 15, ROW1 + 1, &format!("{:02}", d.half_track / 2), color, (0, WIDTH));
            }
            return;
        }
        let Some(d) = s.drives[0] else {
            self.text(DRIVE_X, ROW1 + 1, "8", DIM, (0, WIDTH));
            self.text(DRIVE_X + 18, ROW1 + 1, "trap", DIM, (0, WIDTH));
            return;
        };
        self.text(DRIVE_X, ROW1 + 1, "8", TEXT, (0, WIDTH));
        // Red drive LED, as on the 1541 and 1571: lit during accesses,
        // blinking on errors (the DOS does that)
        self.rect(DRIVE_X + 9, ROW1 + 3, 6, 4, if d.led { LED_ON } else { LED_OFF });
        let t = track(d.half_track);
        self.text(DRIVE_X + 18, ROW1 + 1, &t, if d.disk { TEXT } else { DIM }, (0, WIDTH));
        let color = if d.motor { GREEN } else { DIM };
        self.reel(DRIVE_X + 52, ROW1 + 1, color, d.motor.then_some(s.frame / 2));
    }

    fn render_joysticks(&mut self, s: &Status) {
        for (i, &port) in [JoyPort::One, JoyPort::Two].iter().enumerate() {
            let x = JOY_X + i * 22;
            let label_color = if s.keyboard_port == port { BRIGHT } else { DIM };
            self.text(x, ROW1 + 1, if i == 0 { "1" } else { "2" }, label_color, (0, WIDTH));
            let joy = s.joy[i];
            let on = |bit: u8| if joy & bit == 0 { GREEN } else { BUTTON };
            let cx = x + 9;
            let body = if s.mouse_captured { TEXT } else { BUTTON };
            match s.ports[i] {
                Device::Joystick => {}
                Device::Mouse | Device::JoyMouse => {
                    // Two buttons over the body; in joystick mode the body
                    // lights green while a direction is held
                    let pressed = |bit: u8| if s.analog_buttons & bit != 0 { RED } else { BUTTON };
                    self.rect(cx, ROW1, 4, 3, pressed(1));
                    self.rect(cx + 5, ROW1, 4, 3, pressed(2));
                    let moving = s.ports[i] == Device::JoyMouse && joy & 0x0F != 0x0F;
                    self.rect(cx, ROW1 + 4, 9, 5, if moving { GREEN } else { body });
                    continue;
                }
                Device::Paddles => {
                    // Two knobs with their fire buttons (LEFT, RIGHT) above
                    for (k, bit) in [(0, 0x04), (5, 0x08)] {
                        self.rect(cx + k + 1, ROW1, 2, 2, if joy & bit == 0 { RED } else { BUTTON });
                        self.rect(cx + k, ROW1 + 3, 4, 6, TEXT);
                    }
                    continue;
                }
                d => {
                    // Light pen or gun: the barrel, its tip green while it
                    // sees the screen, the buttons red when pressed
                    let sees = d.pen().is_some_and(|p| !p.touch || s.analog_buttons & 2 != 0);
                    self.rect(cx + 3, ROW1, 3, 7, TEXT);
                    self.rect(cx + 3, ROW1 + 7, 3, 2, if sees { GREEN } else { BUTTON });
                    self.rect(cx + 7, ROW1 + 1, 2, 2, if s.analog_buttons & 1 != 0 { RED } else { BUTTON });
                    continue;
                }
            }
            // Direction cross with fire in the middle, lit when pressed
            self.rect(cx + 3, ROW1, 3, 3, on(0x01));
            self.rect(cx + 3, ROW1 + 6, 3, 3, on(0x02));
            self.rect(cx, ROW1 + 3, 3, 3, on(0x04));
            self.rect(cx + 6, ROW1 + 3, 3, 3, on(0x08));
            self.rect(cx + 3, ROW1 + 3, 3, 3, if joy & 0x10 == 0 { RED } else { BUTTON });
        }
        if s.gamepad {
            self.icon(JOY_X + 44, ROW1 + 2, &[
                ".xxxxxxx.",
                "xx.xxx.xx",
                "x...x.x.x",
                "xx.xxx.xx",
                "xxxx.xxxx",
            ], TEXT);
        }
    }

    /// Second row: inserted tape, disk and cartridge (with * if there are
    /// changes to save); on the right the configuration (SID, REU) and the
    /// position among the media for F8.
    fn render_media(&mut self, s: &Status) {
        let mut right = s.config.clone();
        if let Some((i, n)) = s.media {
            right += &format!("  F8 {i}/{n}");
        }
        let right_cols = right.chars().count();
        self.text(WIDTH - 3 - 8 * right_cols, ROW2, &right, DIM, (0, WIDTH));
        let mut items = s.media_items();
        if items.is_empty() {
            return;
        }
        // Each item: letter, space, name; two spaces between items and
        // before the right part
        let room = TEXT_COLS.saturating_sub(right_cols + 2 + 4 * items.len());
        let each = (room / items.len()).max(6);
        for it in &mut items {
            it.1 = shorten(&it.1, each);
        }
        let clip = (0, WIDTH - 3 - 8 * (right_cols + 1));
        let mut x = TEXT_X;
        for (label, name) in &items {
            x = self.text(x, ROW2, &label.to_string(), DIM, clip) + 8;
            x = self.text(x, ROW2, name, TEXT, clip) + 16;
        }
    }

    /// Third row: the last message for a few seconds (scrolling if it is
    /// long), otherwise the key reminders.
    fn render_message(&mut self, s: &Status) {
        if let Some((msg, at)) = &s.message {
            if Instant::now() < *at + message_duration(msg) {
                self.scrolled_message(msg, *at);
                return;
            }
        }
        if !s.help.is_empty() {
            let page = (Instant::now().duration_since(*START).as_secs() / 5) as usize % s.help.len();
            self.text(TEXT_X, ROW3, s.help[page], DIM, (TEXT_X, WIDTH - TEXT_X));
        }
    }

    /// Message on the third row since `start`: still for a second, then it
    /// scrolls if it is longer than the row.
    fn scrolled_message(&mut self, msg: &str, start: Instant) {
        let age = Instant::now().saturating_duration_since(start).as_millis();
        let extra = msg.chars().count().saturating_sub(TEXT_COLS) * 8;
        let offset = (age.saturating_sub(MESSAGE_PAUSE_MS) / MS_PER_PIXEL).min(extra as u128) as usize;
        self.text_scrolled(TEXT_X, offset, ROW3, msg, BRIGHT, (TEXT_X, WIDTH - TEXT_X));
    }

    // ── Primitives ───────────────────────────────────────────────────────────

    fn rect(&mut self, x: usize, y: usize, w: usize, h: usize, color: u32) {
        for yy in y..(y + h).min(BAR_HEIGHT) {
            let row = &mut self.pixels[yy * WIDTH..(yy + 1) * WIDTH];
            for px in &mut row[x.min(WIDTH)..(x + w).min(WIDTH)] {
                *px = color;
            }
        }
    }

    fn icon(&mut self, x: usize, y: usize, rows: &[&str], color: u32) {
        for (dy, row) in rows.iter().enumerate() {
            for (dx, c) in row.chars().enumerate() {
                if c == 'x' {
                    self.rect(x + dx, y + dy, 1, 1, color);
                }
            }
        }
    }

    /// 7×7 pixel reel (or disk) with a spoke; `phase` makes it spin.
    fn reel(&mut self, x: usize, y: usize, color: u32, phase: Option<u64>) {
        self.icon(x, y, &["..xxx..", ".x...x.", "x.....x", "x.....x", "x.....x", ".x...x.", "..xxx.."], color);
        let spoke: &[&str] = match phase.unwrap_or(0) % 4 {
            0 => &["...x...", "...x...", "...x..."],
            1 => &["....x..", "...x...", "..x...."],
            2 => &[".......", ".xxxxx.", "......."],
            _ => &["..x....", "...x...", "....x.."],
        };
        self.icon(x, y + 2, spoke, color);
    }

    /// Text in the C64 font from `x`, clipped to the `clip` column range;
    /// returns the x after the last character.
    fn text(&mut self, x: usize, y: usize, s: &str, color: u32, clip: (usize, usize)) -> usize {
        self.text_scrolled(x, 0, y, s, color, clip)
    }

    /// Like `text`, shifted left by `offset` pixels.
    fn text_scrolled(&mut self, x: usize, offset: usize, y: usize, s: &str, color: u32, clip: (usize, usize)) -> usize {
        let mut cx = x as isize - offset as isize;
        for c in s.chars() {
            let glyph = screen_code(c) as usize * 8;
            for row in 0..8 {
                let bits = self.font[glyph + row];
                for col in 0..8 {
                    let px = cx + col;
                    if bits & (0x80 >> col) != 0 && px >= clip.0 as isize && px < clip.1 as isize && y + row < BAR_HEIGHT {
                        self.pixels[(y + row) * WIDTH + px as usize] = color;
                    }
                }
            }
            cx += 8;
        }
        cx.max(0) as usize
    }
}

/// Reference time for alternating the reminder pages.
static START: std::sync::LazyLock<Instant> = std::sync::LazyLock::new(Instant::now);

/// Lowercase-set screen code for a character: accented letters lose the
/// accent, what the C64 lacks becomes the closest lookalike.
fn screen_code(c: char) -> u8 {
    let c = match c {
        'à' | 'á' | 'â' => 'a',
        'è' | 'é' | 'ê' => 'e',
        'ì' | 'í' => 'i',
        'ò' | 'ó' => 'o',
        'ù' | 'ú' => 'u',
        'À' => 'A',
        'È' | 'É' => 'E',
        'Ì' => 'I',
        'Ò' => 'O',
        'Ù' => 'U',
        '{' => '(',
        '}' => ')',
        '\\' => '/',
        '`' | '‘' | '’' => '\'',
        '“' | '”' => '"',
        '·' | '•' | '…' => '.',
        '—' | '–' | '~' => '-',
        '→' => '>',
        '±' => '+',
        '×' => 'x',
        c => c,
    };
    match c {
        '@' => 0,
        'a'..='z' => c as u8 - b'a' + 1,
        '[' => 27,
        '£' => 28,
        ']' => 29,
        '↑' => 30,
        '←' => 31,
        ' '..='?' => c as u8,
        'A'..='Z' => c as u8 - b'A' + 65,
        '|' | '│' => 93,
        '_' => 100,
        _ => b'?',
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shorten_keeps_start_and_extension() {
        assert_eq!(shorten("game.tap", 20), "game.tap");
        assert_eq!(shorten("Double Dragon III (side A).tap", 16), "Double Dra...tap");
    }

    #[test]
    fn clicks_hit_tape_buttons_and_counter() {
        assert_eq!(click_at(TAPE_X + 1, ROW1 + 4), Some(Click::Tape(Button::Record)));
        assert_eq!(click_at(TAPE_X + BUTTON_STEP + 4, ROW1), Some(Click::Tape(Button::Play)));
        assert_eq!(click_at(TAPE_X + 4 * BUTTON_STEP + 8, ROW1 + 8), Some(Click::Tape(Button::Stop)));
        assert_eq!(click_at(TAPE_X + 9, ROW1 + 4), None); // between two buttons
        assert_eq!(click_at(COUNTER_X + 10, ROW1 + 3), Some(Click::ResetCounter));
        assert_eq!(click_at(TAPE_X + 1, ROW2), None);
    }

    /// Colours on the third row of a rendered bar.
    fn row3_colors(bar: &Bar) -> Vec<u32> {
        bar.pixels()[(ROW3 + 3) * WIDTH..(ROW3 + 4) * WIDTH].to_vec()
    }

    #[test]
    fn message_then_key_reminders() {
        // A font of solid blocks: every character draws pixels
        let mut rom = vec![0u8; 0x1000];
        rom[0x800..].fill(0xFF);
        let mut bar = Bar::new(&rom);
        let c64 = C64::new();
        let mut s = Status::of(&c64);
        s.help = &["help"];
        // A fresh message: bright text
        s.message = Some(("a message".into(), Instant::now()));
        bar.render(&s);
        assert!(row3_colors(&bar).contains(&BRIGHT));
        // An old one: the key reminders, dim
        s.message = Some(("a message".into(), Instant::now() - Duration::from_secs(60)));
        bar.render(&s);
        let row = row3_colors(&bar);
        assert!(!row.contains(&BRIGHT) && row.contains(&DIM));
    }

    #[test]
    fn screen_codes_of_the_lowercase_set() {
        assert_eq!(screen_code('a'), 1);
        assert_eq!(screen_code('A'), 65);
        assert_eq!(screen_code('è'), 5);
        assert_eq!(screen_code('0'), 48);
        assert_eq!(screen_code('%'), 37);
    }
}
