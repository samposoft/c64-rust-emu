// SPDX-License-Identifier: GPL-2.0-or-later
// Copyright (C) 2026 SampoSoft - Francesco Sampoli
// The 1351 reading, the movement limit and the port selection follow VICE 3.10
// (joyport/mouse_1351.c, mouse.c, joyport.c), Copyright (C) Marco van den
// Heuvel, Andreas Boose, Hannu Nuotio; the joystick mode follows
// joyport/mouse_digital.c, Copyright (C) the VICE Team; the SID measurement
// model is new.
// Ported to Rust and modified by SampoSoft in 2026; see CREDITS.md.

//! Control ports: what is plugged into the two joystick ports besides a
//! joystick (paddles, 1351 mouse) and the SID's POTX/POTY measurement.
//!
//! The SID has one POTX and one POTY pin; a 4066 analog switch on the C64
//! board connects them to control port 1 when CIA1 PA6 is high and to port
//! 2 when PA7 is high (both: the two ports in parallel; neither: nothing,
//! the pins float). Every 512 cycles the SID discharges the capacitor on
//! each pin for 256 cycles, then counts the cycles until the voltage,
//! charging through the resistance plugged into the port, crosses the
//! threshold: the count (255 if it never gets there) is latched into
//! $D419/$D41A at the end of the period. Here the charge is integrated
//! cycle by cycle segment, so switching the port during the count gives the
//! mixed value a real SID reads; a reading `v` corresponds to a charging
//! time of `v` cycles, and two ports in parallel add their conductances.
//!
//! The 1351 in joystick mode (right button held at power-on; the same as
//! the older 1350 mouse, VICE's "digital" mouse) instead moves the joystick
//! lines: each step of movement pulls the line of its direction low for a
//! frame and a half, so a moving mouse holds the direction as a joystick
//! would; left button = FIRE, right button = POTX pulled to 0.
//!
//! Paddles are two potentiometers per port, with the fire buttons on the
//! LEFT (paddle X) and RIGHT (paddle Y) lines. The window points them with
//! the host pointer (`point_at`: its position over the display area is the
//! position of the knobs); the debugger and a captured mouse turn them by
//! relative steps (`move_by`). The 1351 mouse in
//! proportional mode puts its position modulo 64 in bits 1-6 of the reading
//! (`(pos & $7F) + $40`, as VICE), the left button on FIRE and the right on
//! UP. VICE also adds random noise to the readings; here they are exact, so
//! runs stay reproducible.

use crate::snapshot::{impl_state, impl_state_enum};

/// Device in a control port (the joystick lines are always read as well).
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Device {
    #[default]
    Joystick,
    Paddles,
    Mouse,
    /// 1351 in joystick mode (1350).
    JoyMouse,
}

impl Device {
    pub fn parse(s: &str) -> Option<Self> {
        match s.to_ascii_lowercase().as_str() {
            "joystick" | "joy" => Some(Device::Joystick),
            "paddles" | "paddle" => Some(Device::Paddles),
            "mouse" | "1351" => Some(Device::Mouse),
            "joymouse" | "1350" => Some(Device::JoyMouse),
            _ => None,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Device::Joystick => "joystick",
            Device::Paddles => "paddles",
            Device::Mouse => "1351 mouse",
            Device::JoyMouse => "1351 mouse in joystick mode",
        }
    }

    /// Driven by the host mouse.
    pub fn is_analog(self) -> bool {
        self != Device::Joystick
    }

    /// A mouse, in either mode: it moves by relative steps, so the window
    /// captures the host mouse for it.
    pub fn is_mouse(self) -> bool {
        matches!(self, Device::Mouse | Device::JoyMouse)
    }
}

/// Buttons of the host mouse, as they map on the analog devices.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Button {
    /// 1351: left button (FIRE); paddles: fire of paddle X (LEFT line).
    Left,
    /// 1351: right button (UP); paddles: fire of paddle Y (RIGHT line).
    Right,
}

/// Reading of a pot input with nothing connected (the pin never charges).
const OPEN: u8 = 0xFF;
/// Largest movement of the 1351 in one step, in POT units: the drivers read
/// the difference modulo 128 as a signed 7-bit value (VICE: MOUSE_MAX_DIFF).
pub const MOUSE_MAX_STEP: i32 = 63;
/// Joystick mode: C64 pixels of movement for one direction pulse (one, so a
/// mouse moving even slowly keeps the direction held and the program's
/// joystick acceleration builds up), and how long the pulse holds the line
/// low (a frame and a half, as VICE's MOVE_CLOCK_DELAY).
const JOY_STEP: i32 = 1;
const JOY_HOLD: u64 = 312 * 63 * 3 / 2;

/// One pot pin (X or Y) of the SID.
#[derive(Clone, Copy, Default)]
struct Pin {
    /// Charge of the capacitor in the current count window (1.0 = threshold).
    charge: f64,
    /// Count at which the threshold was crossed in the current window.
    reached: Option<u8>,
    /// Value of the register ($D419/$D41A).
    latched: u8,
}

pub struct ControlPorts {
    pub devices: [Device; 2],
    /// Paddle readings per port (X, Y): 0 = minimum resistance, 255 = maximum.
    pub paddles: [[u8; 2]; 2],
    /// 1351 position (X, Y) in POT units; Y grows upwards.
    pub mouse: [i32; 2],
    /// Buttons of the analog devices (bit 0 = left, bit 1 = right).
    pub buttons: u8,
    /// Joystick mode: movement not yet turned into pulses (X, Y; C64 pixels,
    /// Y downwards), and the direction held low with the cycle it ends at.
    joy_acc: [i32; 2],
    joy_dir: [u8; 2],
    joy_until: [u64; 2],
    /// Ports selected by CIA1 PA6/PA7 (bit 0 = port 1, bit 1 = port 2).
    mask: u8,
    /// Cycle up to which the measurement has been computed.
    pos: u64,
    pins: [Pin; 2],
}

impl ControlPorts {
    pub fn new() -> Self {
        Self {
            devices: [Device::Joystick; 2],
            paddles: [[0x80; 2]; 2],
            mouse: [0; 2],
            buttons: 0,
            joy_acc: [0; 2],
            joy_dir: [0; 2],
            joy_until: [0; 2],
            // CIA1 after reset: PA all inputs, read as 1 (both ports)
            mask: 3,
            pos: 0,
            pins: [Pin { latched: OPEN, ..Pin::default() }; 2],
        }
    }

    /// Joystick lines pulled low by the device in `port` (0 or 1) at cycle
    /// `now`, active low like `Bus::joy1`/`joy2`.
    #[inline]
    pub fn lines(&self, port: usize, now: u64) -> u8 {
        let (left, right) = match self.devices[port] {
            Device::Joystick => return 0xFF,
            Device::JoyMouse => return self.joy_lines(now),
            _ if self.buttons == 0 => return 0xFF,
            Device::Mouse => (0x10, 0x01),   // FIRE, UP
            Device::Paddles => (0x04, 0x08), // LEFT, RIGHT
        };
        let mut v = 0xFF;
        if self.buttons & 1 != 0 { v &= !left; }
        if self.buttons & 2 != 0 { v &= !right; }
        v
    }

    /// Joystick mode: FIRE from the left button, the directions of the
    /// pulses still running.
    fn joy_lines(&self, now: u64) -> u8 {
        let mut v = 0xFF;
        if self.buttons & 1 != 0 {
            v &= !0x10;
        }
        for axis in 0..2 {
            if now < self.joy_until[axis] {
                v &= !self.joy_dir[axis];
            }
        }
        v
    }

    /// Resistance on `axis` (0 = X, 1 = Y) of `port`, as a reading.
    fn source(&self, port: usize, axis: usize) -> u8 {
        match self.devices[port] {
            Device::Joystick => OPEN,
            // Right button: POTX to ground
            Device::JoyMouse => if axis == 0 && self.buttons & 2 != 0 { 0 } else { OPEN },
            Device::Paddles => self.paddles[port][axis],
            Device::Mouse => ((self.mouse[axis] & 0x7F) + 0x40) as u8,
        }
    }

    /// Conductance seen by the pin of `axis` with the ports in `mask`, as
    /// 1/cycles to the threshold; infinite for a reading of 0.
    fn rate(&self, axis: usize) -> f64 {
        let mut rate = 0.0;
        for port in 0..2 {
            if self.mask & (1 << port) != 0 {
                match self.source(port, axis) {
                    OPEN => {}
                    0 => return f64::INFINITY,
                    v => rate += 1.0 / v as f64,
                }
            }
        }
        rate
    }

    /// Brings the measurement up to cycle `now`, with the current mask and
    /// devices: to be called before any of them changes and before reading.
    pub fn sync(&mut self, now: u64) {
        let entry = self.pos;
        while self.pos < now {
            let start = self.pos & !511;
            let window = start + 256;
            let end = (start + 512).min(now);
            if end > window {
                let from = self.pos.max(window);
                let len = (end - from) as f64;
                let offset = from - window;
                for axis in 0..2 {
                    let rate = self.rate(axis);
                    let pin = &mut self.pins[axis];
                    if pin.reached.is_some() || rate == 0.0 {
                        continue;
                    }
                    // Cycles still needed at this rate
                    let need = if rate.is_infinite() { 0.0 } else { ((1.0 - pin.charge) / rate - 1e-9).ceil() };
                    if need <= len {
                        pin.reached = Some((offset + need as u64).min(255) as u8);
                    } else {
                        pin.charge += len * rate;
                    }
                }
            }
            self.pos = end;
            if end == start + 512 {
                // End of the period: latch, discharge
                for pin in &mut self.pins {
                    pin.latched = pin.reached.unwrap_or(OPEN);
                    pin.charge = 0.0;
                    pin.reached = None;
                }
                // A whole period measured in this call (nothing changed)
                // gives the same values as all the following ones
                if start >= entry && now - self.pos >= 512 {
                    self.pos = now & !511;
                }
            }
        }
    }

    /// $D419 (reg $19) or $D41A (reg $1A) at cycle `now`.
    pub fn read(&mut self, reg: u8, now: u64) -> u8 {
        self.sync(now);
        self.peek(reg)
    }

    /// Register value as of the last `sync`.
    pub fn peek(&self, reg: u8) -> u8 {
        self.pins[(reg & 1 ^ 1) as usize].latched
    }

    /// CIA1 port A changed: `mask` = PA6/PA7 as seen by the 4066.
    pub fn set_mask(&mut self, mask: u8, now: u64) {
        if mask != self.mask {
            self.sync(now);
            self.mask = mask;
        }
    }

    /// Host mouse movement, in C64 pixels (Y positive downwards), applied
    /// to every analog device. The 1351 moves 2 POT units per pixel, at
    /// most `MOUSE_MAX_STEP` per call (the excess is dropped, as in VICE);
    /// paddles turn one step per pixel, X with the horizontal movement and Y
    /// with the vertical one, right and up lowering the reading. With
    /// `typing` the 1351 in joystick mode ignores the movement (see
    /// `C64::mouse_move`).
    pub fn move_by(&mut self, dx: i32, dy: i32, now: u64, typing: bool) {
        self.sync(now);
        for port in 0..2 {
            match self.devices[port] {
                Device::Joystick => {}
                Device::Paddles => {
                    let p = &mut self.paddles[port];
                    p[0] = (p[0] as i32 - dx).clamp(0, 255) as u8;
                    p[1] = (p[1] as i32 + dy).clamp(0, 255) as u8;
                }
                Device::Mouse | Device::JoyMouse => {}
            }
        }
        if self.devices.contains(&Device::JoyMouse) && !typing {
            // Right/left (bits 3/2), down/up (bits 1/0)
            for (axis, d, bits) in [(0, dx, (0x08, 0x04)), (1, dy, (0x02, 0x01))] {
                self.joy_acc[axis] += d;
                let acc = self.joy_acc[axis];
                if acc.abs() >= JOY_STEP {
                    self.joy_dir[axis] = if acc > 0 { bits.0 } else { bits.1 };
                    self.joy_until[axis] = now + JOY_HOLD;
                    self.joy_acc[axis] = 0;
                }
            }
        }
        if self.devices.contains(&Device::Mouse) {
            let (mut mx, mut my) = (2 * dx, -2 * dy);
            let big = mx.abs().max(my.abs());
            if big > MOUSE_MAX_STEP {
                // Keep the direction, as VICE's mouse_move_apply_limit
                mx = mx * MOUSE_MAX_STEP / big;
                my = my * MOUSE_MAX_STEP / big;
            }
            self.mouse[0] += mx;
            self.mouse[1] += my;
        }
    }

    /// Host pointer at `(x, y)` in the framebuffer (403×284): its position
    /// over the 320×200 display area sets the paddles of every port, X
    /// horizontally (left edge = fully left, the highest reading, as when
    /// the mouse moves left) and Y vertically (top edge = lowest reading).
    pub fn point_at(&mut self, x: i32, y: i32, now: u64) {
        use crate::vic::{DISPLAY_X, DISPLAY_Y};
        self.sync(now);
        let px = ((x - DISPLAY_X as i32) * 255 / 319).clamp(0, 255) as u8;
        let py = ((y - DISPLAY_Y as i32) * 255 / 199).clamp(0, 255) as u8;
        for port in 0..2 {
            if self.devices[port] == Device::Paddles {
                self.paddles[port] = [255 - px, py];
            }
        }
    }

    /// Button of the analog devices pressed or released at cycle `now`.
    pub fn set_button(&mut self, b: Button, pressed: bool, now: u64) {
        self.sync(now);
        let bit = match b { Button::Left => 1, Button::Right => 2 };
        if pressed { self.buttons |= bit } else { self.buttons &= !bit }
    }

    /// Paddle readings of `port` (0 or 1).
    pub fn set_paddles(&mut self, port: usize, x: u8, y: u8, now: u64) {
        self.sync(now);
        self.paddles[port] = [x, y];
    }

    /// Plugs `dev` into `port` (0 or 1).
    pub fn set_device(&mut self, port: usize, dev: Device, now: u64) {
        self.sync(now);
        self.devices[port] = dev;
        if !self.devices.iter().any(|d| d.is_analog()) {
            self.buttons = 0;
        }
    }

    /// State of both ports, for the debugger.
    pub fn summary(&self) -> String {
        let mut s = String::new();
        for port in 0..2 {
            s += &format!("  port {}: {}", port + 1, self.devices[port].name());
            match self.devices[port] {
                Device::Joystick => {}
                Device::Paddles => s += &format!("  X={} Y={}", self.paddles[port][0], self.paddles[port][1]),
                Device::Mouse => s += &format!("  position {},{} (POT ${:02X} ${:02X})",
                    self.mouse[0], self.mouse[1], self.source(port, 0), self.source(port, 1)),
                Device::JoyMouse => {
                    let dir = |b: u8| match b { 0x01 => "up", 0x02 => "down", 0x04 => "left", 0x08 => "right", _ => "-" };
                    s += &format!("  last pulses {} {}", dir(self.joy_dir[0]), dir(self.joy_dir[1]));
                }
            }
            s.push('\n');
        }
        let sel = match self.mask { 0 => "none", 1 => "port 1", 2 => "port 2", _ => "ports 1 and 2" };
        s += &format!("  POT pins on {sel} (CIA1 PA6/PA7); $D419=${:02X} $D41A=${:02X}; buttons: {}{}\n",
            self.peek(0x19), self.peek(0x1A),
            if self.buttons & 1 != 0 { "left " } else { "" },
            if self.buttons & 2 != 0 { "right" } else if self.buttons == 0 { "none" } else { "" });
        s
    }

    /// Some port has an analog device, driven by the host mouse.
    pub fn wants_mouse(&self) -> bool {
        self.devices.iter().any(|d| d.is_analog())
    }

    /// Some port has paddles.
    pub fn has_paddles(&self) -> bool {
        self.devices.contains(&Device::Paddles)
    }

    /// Some port has a mouse (either mode), which needs the host mouse
    /// captured.
    pub fn has_mouse(&self) -> bool {
        self.devices.iter().any(|d| d.is_mouse())
    }
}

impl Default for ControlPorts {
    fn default() -> Self {
        Self::new()
    }
}

impl_state_enum!(Device { Joystick, Paddles, Mouse, JoyMouse });
impl_state!(Pin { charge, reached, latched });
impl_state!(ControlPorts { devices, paddles, mouse, buttons, joy_acc, joy_dir, joy_until, mask, pos, pins });

#[cfg(test)]
mod tests {
    use super::*;

    fn ports(dev: Device) -> ControlPorts {
        let mut c = ControlPorts::new();
        c.set_device(0, dev, 0);
        c.set_mask(1, 0);
        c
    }

    #[test]
    fn open_pins_read_ff() {
        let mut c = ControlPorts::new();
        assert_eq!(c.read(0x19, 5000), 0xFF);
        assert_eq!(c.read(0x1A, 5000), 0xFF);
    }

    #[test]
    fn paddle_value_latched_at_end_of_period() {
        let mut c = ports(Device::Paddles);
        c.set_paddles(0, 100, 7, 0);
        assert_eq!(c.read(0x19, 511), 0xFF, "the first period is not over");
        assert_eq!(c.read(0x19, 512), 100);
        assert_eq!(c.read(0x1A, 512), 7);
        c.set_paddles(0, 0, 255, 600);
        assert_eq!(c.read(0x19, 1023), 100, "the register holds the last measurement");
        assert_eq!(c.read(0x19, 1024), 0);
        assert_eq!(c.read(0x1A, 1024), 255);
    }

    #[test]
    fn long_gaps_skip_identical_periods() {
        let mut c = ports(Device::Paddles);
        c.set_paddles(0, 100, 7, 0);
        assert_eq!(c.read(0x19, 1_000_000_000), 100);
        c.set_paddles(0, 60, 7, 1_000_000_000);
        assert_eq!(c.read(0x19, 1_000_000_000 + 511), 100);
        assert_eq!(c.read(0x19, 1_000_000_000 + 1024), 60);
    }

    #[test]
    fn deselected_port_floats() {
        let mut c = ports(Device::Paddles);
        c.set_paddles(0, 50, 50, 0);
        c.set_mask(2, 0);
        assert_eq!(c.read(0x19, 1024), 0xFF);
    }

    #[test]
    fn two_ports_in_parallel() {
        let mut c = ControlPorts::new();
        c.set_device(0, Device::Paddles, 0);
        c.set_device(1, Device::Paddles, 0);
        c.set_paddles(0, 100, 0xFF, 0);
        c.set_paddles(1, 100, 40, 0);
        c.set_mask(3, 0);
        assert_eq!(c.read(0x19, 512), 50);
        assert_eq!(c.read(0x1A, 512), 40);
    }

    #[test]
    fn switch_during_the_count_mixes_the_ports() {
        let mut c = ControlPorts::new();
        c.set_device(0, Device::Paddles, 0);
        c.set_device(1, Device::Paddles, 0);
        c.set_paddles(0, 200, 200, 0);
        c.set_paddles(1, 20, 20, 0);
        c.set_mask(1, 0);
        // 100 cycles of the count at 1/200, then port 2 at 1/20: half of
        // the charge left, 10 more cycles
        c.set_mask(2, 256 + 100);
        assert_eq!(c.read(0x19, 512), 110);
        assert_eq!(c.read(0x19, 1024), 20, "the next period is clean");
    }

    #[test]
    fn switch_during_the_discharge_is_clean() {
        let mut c = ControlPorts::new();
        c.set_device(1, Device::Paddles, 0);
        c.set_paddles(1, 30, 30, 0);
        c.set_mask(1, 0);
        c.set_mask(2, 200);
        assert_eq!(c.read(0x19, 512), 30);
    }

    #[test]
    fn mouse_position_in_bits_1_to_6() {
        let mut c = ports(Device::Mouse);
        assert_eq!(c.read(0x19, 512), 0x40);
        c.move_by(5, -3, 600, false);   // right 5, up 3
        assert_eq!(c.read(0x19, 1024), 0x4A);
        assert_eq!(c.read(0x1A, 1024), 0x46);
        c.move_by(-40, 0, 1100, false); // 80 units: limited to 63
        assert_eq!(c.read(0x19, 1536), (0x0A - 63 & 0x7F) as u8 + 0x40);
    }

    #[test]
    fn pointer_sets_the_paddles() {
        let mut c = ports(Device::Paddles);
        c.point_at(41, 42, 0);
        assert_eq!(c.paddles[0], [255, 0], "top left corner of the display area");
        c.point_at(360, 241, 0);
        assert_eq!(c.paddles[0], [0, 255], "bottom right corner");
        c.point_at(0, 283, 0);
        assert_eq!(c.paddles[0], [255, 255], "on the border: clamped");
    }

    #[test]
    fn buttons_on_the_joystick_lines() {
        let mut c = ports(Device::Mouse);
        c.set_button(Button::Left, true, 0);
        c.set_button(Button::Right, true, 0);
        assert_eq!(c.lines(0, 0), 0xFF & !0x10 & !0x01);
        assert_eq!(c.lines(1, 0), 0xFF);
        c.set_device(0, Device::Paddles, 0);
        c.set_button(Button::Left, true, 0);
        assert_eq!(c.lines(0, 0), 0xFF & !0x04 & !0x08);
    }

    #[test]
    fn joystick_mode_pulses_the_directions() {
        let mut c = ports(Device::JoyMouse);
        c.move_by(0, 0, 100, false);
        assert_eq!(c.lines(0, 100), 0xFF, "no movement: nothing");
        c.move_by(1, -3, 100, false);
        assert_eq!(c.lines(0, 101), 0xFF & !0x08 & !0x01, "right and up");
        assert_eq!(c.lines(0, 100 + JOY_HOLD - 1), 0xFF & !0x08 & !0x01);
        assert_eq!(c.lines(0, 100 + JOY_HOLD), 0xFF, "the pulse is over");
        c.move_by(-5, 4, 50_000, false);
        assert_eq!(c.lines(0, 50_000), 0xFF & !0x04 & !0x02, "left and down");
    }

    #[test]
    fn joystick_mode_ignores_movement_while_typing() {
        let mut c = ports(Device::JoyMouse);
        c.move_by(5, 5, 100, true);
        assert_eq!(c.lines(0, 100), 0xFF);
        c.move_by(5, 0, 200, false);
        assert_eq!(c.lines(0, 200), 0xFF & !0x08, "only the movement after typing");
    }

    #[test]
    fn joystick_mode_buttons() {
        let mut c = ports(Device::JoyMouse);
        c.set_button(Button::Left, true, 0);
        assert_eq!(c.lines(0, 0), 0xFF & !0x10, "left = FIRE");
        c.set_button(Button::Right, true, 0);
        assert_eq!(c.read(0x19, 512), 0, "right = POTX to 0");
        assert_eq!(c.read(0x1A, 512), 0xFF);
        c.set_button(Button::Right, false, 600);
        assert_eq!(c.read(0x19, 1024), 0xFF);
    }
}
