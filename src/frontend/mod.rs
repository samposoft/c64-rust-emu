// SPDX-License-Identifier: GPL-2.0-or-later
// Copyright (C) 2026 SampoSoft - Francesco Sampoli

//! Parts shared by the frontends (`c64`, `c64term` and `c64dbg --window`): audio
//! with cpal, keyboard joystick, gamepad with gilrs (feature `gamepad`). The
//! winit-specific part is in [`window`], the terminal one in `term` (Unix only),
//! configuration and frame loop in [`session`], the remote monitor in [`remote`].

pub mod remote;
#[cfg(feature = "gpu")]
pub mod gpu;
pub mod session;
pub mod status;
#[cfg(unix)]
pub mod term;
pub mod window;

use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};

use crate::c64::{C64, CLOCK_HZ, CYCLES_PER_FRAME};
use crate::keyboard::HostKey;
use crate::ring;

/// Duration of an emulation frame (`CYCLES_PER_FRAME` cycles at the PAL
/// clock): slightly less than 20 ms, so the machine runs at exactly
/// 985,248 Hz and the audio comes out at the right rate.
pub const FRAME: Duration = Duration::from_nanos(1_000_000_000 * CYCLES_PER_FRAME as u64 / CLOCK_HZ);

// ── Audio ────────────────────────────────────────────────────────────────────

/// Audio output: cpal stream fed by a lock-free ring buffer of i16 samples
/// already in the device format (one sample per channel). The SID produces
/// the samples directly at the device rate (`rate`); `push` spreads them
/// over the channels and the real-time callback copies them, without locks
/// and without printing: device overruns (xruns) are only counted and
/// summarized by `push`, on the emulation thread.
pub struct AudioOut {
    /// Keeps the cpal stream alive.
    _stream: cpal::Stream,
    producer: ring::Producer,
    rate: u32,
    channels: usize,
    /// Maximum queue before dropping a frame: 3 frames (60 ms).
    max_queued: usize,
    /// Samples converted to the device format.
    scratch: Vec<i16>,
    xruns: Arc<AtomicU32>,
    reported: u32,
    opened: Instant,
    last_report: Instant,
}

/// With the queue empty the last sample decays by this factor per frame:
/// at 48 kHz it drops below 1% in about 20 ms.
const UNDERRUN_DECAY: f32 = 0.995;
/// Right after opening, overruns are normal (the device is starting up).
const XRUN_GRACE: Duration = Duration::from_secs(1);
/// Minimum interval between two overrun summaries.
const XRUN_REPORT_INTERVAL: Duration = Duration::from_secs(10);

impl AudioOut {
    /// Opens the default output device at its own rate. With the queue
    /// empty the callback repeats the last sample.
    pub fn open() -> Option<Self> {
        let device = cpal::default_host().default_output_device()?;
        let default = device.default_output_config().ok()?;
        let channels = default.channels() as usize;
        let rate = default.sample_rate();
        let config = cpal::StreamConfig {
            channels: default.channels(),
            sample_rate: default.sample_rate(),
            buffer_size: cpal::BufferSize::Default,
        };

        // Room for max_queued plus two whole frames
        let frame = rate as usize / 50 * channels;
        let max_queued = frame * 3;
        let (producer, mut consumer) = ring::channel(max_queued + 2 * frame);
        let xruns = Arc::new(AtomicU32::new(0));
        let cb_xruns = xruns.clone();
        let mut last = vec![0.0f32; channels];

        let stream = device.build_output_stream(
            config,
            move |data: &mut [f32], _| {
                // The queue holds whole frames: if the first sample is there,
                // so are the others. With the queue empty (turbo, pause,
                // delays) the last frame decays to zero in a few milliseconds
                // instead of holding: resuming does not make a step.
                for frame in data.chunks_mut(channels) {
                    if let Some(s) = consumer.pop() {
                        last[0] = s as f32 / 32768.0;
                        for l in &mut last[1..] {
                            *l = consumer.pop().map_or(*l, |s| s as f32 / 32768.0);
                        }
                    } else {
                        for l in last.iter_mut() {
                            *l *= UNDERRUN_DECAY;
                        }
                    }
                    frame.copy_from_slice(&last);
                }
            },
            move |err| {
                if err.kind() == cpal::ErrorKind::Xrun {
                    cb_xruns.fetch_add(1, Ordering::Relaxed);
                } else {
                    eprintln!("WARN audio: {err}");
                }
            },
            None,
        ).ok()?;
        stream.play().ok()?;
        eprintln!("Audio: {rate} Hz, {channels} channels");
        let now = Instant::now();
        Some(Self {
            _stream: stream, producer, rate, channels, max_queued, scratch: Vec::new(),
            xruns, reported: 0, opened: now, last_report: now,
        })
    }

    /// Sample rate of the device.
    pub fn rate(&self) -> u32 {
        self.rate
    }

    /// Queues the samples of a frame, mono (`src_channels` 1) or interleaved
    /// stereo (2): on a mono device the two channels are mixed as in VICE, on
    /// a stereo one the mono goes to both; beyond the second, the channels
    /// repeat the mono or stay silent. The frame is dropped if the queue
    /// already holds 3 (the emulation is ahead of the device). Summarizes
    /// the device overruns at most every 10 seconds.
    pub fn push(&mut self, samples: &[i16], src_channels: usize) {
        let out = &mut self.scratch;
        out.clear();
        let ch = self.channels;
        if src_channels == 2 {
            for f in samples.chunks_exact(2) {
                if ch == 1 {
                    out.push(crate::sid::audio_mix(f[0], f[1]));
                } else {
                    out.extend_from_slice(f);
                    out.extend(std::iter::repeat(0).take(ch - 2));
                }
            }
        } else {
            for &s in samples {
                out.extend(std::iter::repeat(s).take(ch));
            }
        }
        // Whole frames or nothing, so the channels do not get out of step
        let free = self.producer.capacity() - self.producer.len();
        if self.producer.len() < self.max_queued && out.len() <= free {
            self.producer.push_slice(out);
        }

        let xruns = self.xruns.load(Ordering::Relaxed);
        if xruns == self.reported {
            return;
        }
        if self.opened.elapsed() < XRUN_GRACE {
            self.reported = xruns;
        } else if self.last_report.elapsed() >= XRUN_REPORT_INTERVAL || self.reported == 0 {
            eprintln!("WARN audio: {} device overruns (possible clicks)", xruns - self.reported);
            self.reported = xruns;
            self.last_report = Instant::now();
        }
    }
}

// ── Keyboard joystick ────────────────────────────────────────────────────────

// Bits of the joystick byte (active low: 0=pressed)
const JOY_UP:    u8 = 0x01;
const JOY_DOWN:  u8 = 0x02;
const JOY_LEFT:  u8 = 0x04;
const JOY_RIGHT: u8 = 0x08;
const JOY_FIRE:  u8 = 0x10;

fn joy_set(joy: &mut u8, bit: u8, pressed: bool) {
    if pressed { *joy &= !bit; } else { *joy |= bit; }
}

/// Arrows + LCtrl/LAlt/Space as FIRE on the active joystick port.
fn joy_key(joy: &mut u8, k: HostKey, pressed: bool) {
    match k {
        HostKey::Up    => joy_set(joy, JOY_UP,    pressed),
        HostKey::Down  => joy_set(joy, JOY_DOWN,  pressed),
        HostKey::Left  => joy_set(joy, JOY_LEFT,  pressed),
        HostKey::Right => joy_set(joy, JOY_RIGHT, pressed),
        HostKey::LCtrl | HostKey::LAlt | HostKey::Space => joy_set(joy, JOY_FIRE, pressed),
        _ => {}
    }
}

/// Joystick port driven by arrows and fire (TAB cycles through them).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum JoyPort { Two, One, None }

impl JoyPort {
    pub fn next(self) -> Self {
        match self { JoyPort::Two => JoyPort::One, JoyPort::One => JoyPort::None, JoyPort::None => JoyPort::Two }
    }
    pub fn label(self) -> &'static str {
        match self {
            JoyPort::Two  => "Joy2 active, Joy1 unplugged",
            JoyPort::One  => "Joy1 active, Joy2 unplugged",
            JoyPort::None => "both unplugged",
        }
    }
}

/// Applies a (non-printable) host key to the C64: joystick on the active port
/// and keyboard matrix. While a joystick port is active the arrows drive
/// only the joystick, as VICE does with the keys of a joystick keyset: as
/// cursor keys too they would close keys in column PA0 of the matrix, the
/// line that joystick 2 up pulls low, and the program would see ghost keys
/// (Giana Sisters pauses: RUN/STOP). With no port they are the cursor keys.
/// Releases always reach the keyboard, for a key held while TAB changed
/// the port.
pub fn apply_host_key(c64: &mut C64, port: JoyPort, hk: HostKey, pressed: bool) {
    match port {
        JoyPort::Two  => joy_key(&mut c64.bus.joy2, hk, pressed),
        JoyPort::One  => joy_key(&mut c64.bus.joy1, hk, pressed),
        JoyPort::None => {}
    }
    let arrow = matches!(hk, HostKey::Up | HostKey::Down | HostKey::Left | HostKey::Right);
    if !(arrow && pressed && port != JoyPort::None) {
        c64.bus.keyboard.update(hk, pressed);
    }
}

/// Changes the active joystick port, releasing both.
pub fn set_joy_port(c64: &mut C64, port: JoyPort) {
    c64.bus.joy1 = 0xFF;
    c64.bus.joy2 = 0xFF;
    crate::notice!("Joystick → {}", port.label());
}

/// Printable characters of a keyboard event (operating system layout);
/// space goes through as a host key.
pub fn printable_chars(text: &str) -> impl Iterator<Item = char> + '_ {
    text.chars().filter(|c| *c != ' ' && !c.is_control())
}

// ── Gamepad ──────────────────────────────────────────────────────────────────

/// Physical gamepad → joystick port 2. With the `gamepad` feature disabled it
/// is a no-op.
#[cfg(feature = "gamepad")]
pub struct Gamepad(Option<gilrs::Gilrs>);

#[cfg(feature = "gamepad")]
impl Gamepad {
    pub fn new() -> Self {
        match gilrs::Gilrs::new() {
            Ok(g) => {
                for (_, gp) in g.gamepads() {
                    eprintln!("Gamepad connected: {}", gp.name());
                }
                Gamepad(Some(g))
            }
            Err(e) => { eprintln!("WARN gamepad: {e}"); Gamepad(None) }
        }
    }

    /// At least one gamepad connected.
    pub fn connected(&self) -> bool {
        self.0.as_ref().is_some_and(|g| g.gamepads().next().is_some())
    }

    /// Consumes the queued events and updates the joystick byte.
    pub fn poll(&mut self, joy: &mut u8) {
        use gilrs::{Axis, Button, EventType};
        const THRESHOLD: f32 = 0.5;
        let Some(g) = self.0.as_mut() else { return };
        while let Some(ev) = g.next_event() {
            match ev.event {
                EventType::ButtonPressed(b, _) | EventType::ButtonReleased(b, _) => {
                    let pressed = matches!(ev.event, EventType::ButtonPressed(..));
                    match b {
                        Button::South | Button::East | Button::West | Button::North |
                        Button::LeftTrigger | Button::RightTrigger |
                        Button::LeftTrigger2 | Button::RightTrigger2 => joy_set(joy, JOY_FIRE, pressed),
                        Button::DPadUp    => joy_set(joy, JOY_UP,    pressed),
                        Button::DPadDown  => joy_set(joy, JOY_DOWN,  pressed),
                        Button::DPadLeft  => joy_set(joy, JOY_LEFT,  pressed),
                        Button::DPadRight => joy_set(joy, JOY_RIGHT, pressed),
                        _ => {}
                    }
                }
                EventType::AxisChanged(axis, v, _) => match axis {
                    Axis::LeftStickX => {
                        joy_set(joy, JOY_LEFT,  v < -THRESHOLD);
                        joy_set(joy, JOY_RIGHT, v >  THRESHOLD);
                    }
                    Axis::LeftStickY => {
                        // gilrs: +1 = up
                        joy_set(joy, JOY_UP,   v >  THRESHOLD);
                        joy_set(joy, JOY_DOWN, v < -THRESHOLD);
                    }
                    _ => {}
                },
                EventType::Connected => {
                    if let Some(gp) = g.connected_gamepad(ev.id) {
                        crate::notice!("Gamepad connected: {}", gp.name());
                    }
                }
                EventType::Disconnected => { *joy = 0xFF; crate::notice!("Gamepad disconnected"); }
                _ => {}
            }
        }
    }
}

#[cfg(not(feature = "gamepad"))]
pub struct Gamepad;

#[cfg(not(feature = "gamepad"))]
impl Gamepad {
    pub fn new() -> Self { Gamepad }
    pub fn poll(&mut self, _joy: &mut u8) {}
    pub fn connected(&self) -> bool { false }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn arrows_are_joystick_or_cursor_keys() {
        let mut c64 = C64::new();
        // Joystick port 2 active: up moves the joystick, no key closes
        apply_host_key(&mut c64, JoyPort::Two, HostKey::Up, true);
        assert_eq!(c64.bus.joy2 & JOY_UP, 0);
        assert!(!c64.bus.keyboard.any_pressed());
        apply_host_key(&mut c64, JoyPort::Two, HostKey::Up, false);
        assert_eq!(c64.bus.joy2, 0xFF);
        // Space is fire and the space key
        apply_host_key(&mut c64, JoyPort::Two, HostKey::Space, true);
        assert_eq!(c64.bus.joy2 & JOY_FIRE, 0);
        assert!(c64.bus.keyboard.any_pressed());
        apply_host_key(&mut c64, JoyPort::Two, HostKey::Space, false);
        // No port: the arrows are the cursor keys
        apply_host_key(&mut c64, JoyPort::None, HostKey::Down, true);
        assert!(c64.bus.keyboard.any_pressed());
        // Released after TAB changed the port: the key does not stay down
        apply_host_key(&mut c64, JoyPort::Two, HostKey::Down, false);
        assert!(!c64.bus.keyboard.any_pressed());
    }
}
