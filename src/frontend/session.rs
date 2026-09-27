// SPDX-License-Identifier: GPL-2.0-or-later
// Copyright (C) 2026 SampoSoft - Francesco Sampoli

//! Machine configuration and real-time frame loop, shared by the frontends
//! (`c64`, `c64term`), with the remote monitor when asked for. `c64dbg` only
//! uses the configuration: there the frames are driven by the debugger.

use std::path::PathBuf;
use std::time::Instant;

use crate::c64::{LoadDetector, C64};
use crate::ctrlport::Device;
use crate::keyboard::HostKey;
use crate::sid::Model;

use super::remote::RemoteMonitor;
use super::status::{Click, SpeedMeter, Status};
use super::{apply_host_key, set_joy_port, AudioOut, Gamepad, JoyPort};

// ── Configuration ────────────────────────────────────────────────────────────

/// Machine options shared by all frontends.
#[derive(Default)]
pub struct MachineOptions {
    /// Files: the first is loaded at startup (.prg, .d64, .g64, .tap, .t64,
    /// .crt); the others (tapes and disks) are the next media, for side B
    /// or the following disks.
    pub input_files: Vec<String>,
    /// ROM directory (if missing, `C64::default_roms_dir`).
    pub roms_dir: Option<PathBuf>,
    /// REU to attach, in KB.
    pub reu: Option<u32>,
    /// No 1541 drive: D64s are loaded through the KERNAL trap.
    pub no_drive: bool,
    /// SID model and digiboost (default 6581).
    pub sid: Option<(Model, bool)>,
    /// Address of the second SID.
    pub sid2: Option<u16>,
    /// Sound of the running tape (VICE's default volume).
    pub tape_sound: bool,
    /// Datasette azimuth error, in thousandths of a cycle.
    pub tape_azimuth: Option<u32>,
    /// Devices in control ports 1 and 2 (joystick by default).
    pub ports: [Device; 2],
    /// Socket of the remote monitor (`--remote`, `--remote-socket`).
    pub remote: Option<PathBuf>,
    /// Frame blending (`--blend`, `C64::set_blend`).
    pub blend: bool,
    /// CRT monitor emulation in the window: the arguments of `--crt`,
    /// `--composite` and `--rf`, in order (`crt`: resolved with the
    /// machine's standard, which may come later on the command line).
    pub crt_args: Vec<String>,
    /// Video standard (`--ntsc`; PAL by default).
    pub standard: crate::timing::Standard,
}

impl MachineOptions {
    /// Syntax of the common arguments, for usage messages.
    pub const USAGE: &'static str =
        "[--version] [--ntsc] [--roms DIR] [--reu KB] [--no-drive] [--sid 6581|8580|8580d] [--sid2 ADDR] [--tape-sound] [--tape-azimuth CYCLES] [--port1 DEV] [--port2 DEV] [--blend] [--crt MONITOR] [--composite] [--rf] [--remote] [--remote-socket PATH] [file.prg | .d64 | .g64 | .tap | .t64 | .crt] [more .tap/.d64/.g64...]";

    /// Explanation of the common arguments, for `--help`.
    pub const HELP: &'static str = "  --version    print version, copyright and license
  --ntsc       an NTSC C64 (VIC-II 6567R8, 1.023 MHz, 60 Hz, 263 lines) instead of PAL
  --roms DIR   directory of kernal.rom, basic.rom, chargen.rom and dos1541.rom (default:
               roms/ next to the executable or in a directory above it, else ./roms)
  --reu KB     attach a REU (128, 256, 512 ... 16384 KB)
  --no-drive   no 1541 drive: D64s are loaded through the KERNAL trap
  --sid M      SID model: 6581 (default), 8580, 8580d (8580 with digiboost)
  --sid2 ADDR  second SID (stereo) at ADDR: d420-d7e0 or de00-dfe0, in steps of 20
  --tape-sound sound of the tape running in PLAY (as VICE's -datasettesound)
  --tape-azimuth CYCLES  Datasette azimuth error: every pulse is off by a random
               amount up to CYCLES (0.001-10, as VICE's -dstapeerror in thousandths)
  --port1 DEV, --port2 DEV  device in control port 1 or 2: joystick (default),
               paddles (follow the pointer), mouse (1351, captured with a click) or
               joymouse (1351 in joystick mode, as the 1350), in the window only
  --blend      show every frame mixed with the previous one, as the eye sees a 50 Hz
               CRT: for pictures that alternate two frames (interlace, IFLI)
  --crt MONITOR  show the screen as a real monitor does, on the GPU: the PAL signal of
               the VIC-II and the picture tube of MONITOR, with the luma/chroma input:
               1084s-p1 (or 1084s: Commodore 1084S-P1, Philips), 1084s-d1 (Daewoo),
               1901 (Thomson), or the TV cp90 (or tv: Philips 15CE1510) through the RF
               modulator; with --ntsc the 1702; off by default, window only
  --composite  connect the monitor through composite video (implies --crt 1084s, or 1702)
  --rf         connect a TV to the RF output, channel 36 (implies --crt tv)
  --remote     remote monitor: debugger commands from other programs (c64mcp, the
               MCP server for Claude) on a Unix socket, macOS and Linux only
  --remote-socket PATH  remote monitor on the socket PATH instead of the default one
  With several files the first is loaded; the others (tapes, disks) are the next
  media, for side B or disk 2: F8 switches to the next one (in the debugger: swap).
  A .tap that does not exist is a blank tape for SAVE, written on exit.";

    /// Parses a common argument, taking from `rest` the value of the
    /// options that have one. Anything that is not a common option is a
    /// file to load. Exits with code 2 if a value is missing or invalid.
    pub fn parse_arg(&mut self, arg: &str, rest: &mut impl Iterator<Item = String>) {
        match arg {
            "--roms" => match rest.next() {
                Some(dir) => self.roms_dir = Some(PathBuf::from(dir)),
                None => { eprintln!("--roms needs the ROM directory"); std::process::exit(2); }
            },
            "--reu" => match rest.next().and_then(|s| s.parse().ok()) {
                Some(kb) => self.reu = Some(kb),
                None => { eprintln!("--reu needs the size in KB (128, 256, 512 ... 16384)"); std::process::exit(2); }
            },
            "--no-drive" => self.no_drive = true,
            "--ntsc" => self.standard = crate::timing::Standard::Ntsc,
            "--pal" => self.standard = crate::timing::Standard::Pal,
            "--sid" => match rest.next().as_deref().and_then(parse_sid_model) {
                Some(m) => self.sid = Some(m),
                None => { eprintln!("--sid needs the model: 6581, 8580 or 8580d (8580 with digiboost)"); std::process::exit(2); }
            },
            "--sid2" => match rest.next().as_deref().and_then(parse_address) {
                Some(a) => self.sid2 = Some(a),
                None => { eprintln!("--sid2 needs the address in hex, e.g. d420 or de00"); std::process::exit(2); }
            },
            "--version" => {
                println!("{}", version_text());
                std::process::exit(0);
            }
            "--tape-sound" => self.tape_sound = true,
            "--blend" => self.blend = true,
            "--crt" | "--composite" | "--rf" => {
                let value = match arg {
                    "--composite" => Some("composite".to_string()),
                    "--rf" => Some("rf".to_string()),
                    _ => rest.next(),
                };
                match value {
                    Some(v) => self.crt_args.push(v.to_ascii_lowercase()),
                    None => { eprintln!("--crt needs the monitor: {} (or off)", crate::crt::Model::NAMES); std::process::exit(2); }
                }
            }
            "--remote" => self.remote = Some(super::remote::default_path()),
            "--remote-socket" => match rest.next() {
                Some(path) => self.remote = Some(PathBuf::from(path)),
                None => { eprintln!("--remote-socket needs the path of the socket"); std::process::exit(2); }
            },
            "--port1" | "--port2" => match rest.next().as_deref().and_then(Device::parse) {
                Some(d) => self.ports[if arg == "--port1" { 0 } else { 1 }] = d,
                None => { eprintln!("{arg} needs the device: joystick, paddles, mouse or joymouse"); std::process::exit(2); }
            },
            "--tape-azimuth" => match rest.next().as_deref().and_then(parse_azimuth) {
                Some(e) => self.tape_azimuth = Some(e),
                None => { eprintln!("--tape-azimuth needs the error in cycles, from 0.001 to 10 (e.g. 1.5)"); std::process::exit(2); }
            },
            _ => self.input_files.push(arg.to_string()),
        }
    }

    /// CRT emulation asked for on the command line, for the machine's
    /// standard.
    pub fn crt(&self) -> Result<Option<crate::crt::Crt>, String> {
        let mut crt = None;
        for a in &self.crt_args {
            crt = crate::crt::Crt::apply(crt, a, self.standard).map_err(|e| format!("--crt: {e}"))?;
        }
        if let Some(c) = crt {
            c.check(self.standard).map_err(|e| format!("--crt: {e}"))?;
        }
        Ok(crt)
    }

    /// The files after the first must be tapes or disks: they are swapped
    /// while the machine is running.
    pub fn check_media(&self) -> Result<(), String> {
        for f in self.input_files.iter().skip(1) {
            let ext = std::path::Path::new(f).extension().and_then(|e| e.to_str()).unwrap_or("").to_lowercase();
            if !matches!(ext.as_str(), "tap" | "d64" | "g64") {
                return Err(format!("{f}: after the first file only tapes (.tap) and disks (.d64, .g64) can be given"));
            }
        }
        Ok(())
    }

    /// Creates the C64 with ROMs, drive and REU and resets it; loading the
    /// file is left to the caller. Missing ROMs and drive are only warnings,
    /// the error is an invalid REU size.
    pub fn build(&self) -> Result<C64, String> {
        let mut c64 = C64::new();
        c64.set_standard(self.standard)?;
        let roms = self.roms_dir.clone().unwrap_or_else(C64::default_roms_dir);
        for w in c64.load_roms_from_dir(&roms) {
            eprintln!("WARN: {w}");
        }
        c64.set_reu(self.reu)?;
        if let Some((model, digiboost)) = self.sid {
            c64.set_sid_model(model, digiboost);
        }
        c64.set_sid2(self.sid2)?;
        c64.set_blend(self.blend);
        c64.set_crt(self.crt()?);
        if self.tape_sound {
            c64.bus.tape.set_sound(Some(crate::tape::SOUND_VOLUME_DEFAULT));
        }
        if let Some(e) = self.tape_azimuth {
            c64.bus.tape.set_azimuth_error(e);
        }
        for (i, dev) in self.ports.iter().enumerate() {
            c64.set_control_port(i + 1, *dev);
        }
        if !self.no_drive {
            if let Err(e) = c64.attach_drive_from_dir(&roms) {
                eprintln!("WARN: {e}; D64s are loaded through the KERNAL trap");
            }
        }
        c64.reset();
        Ok(c64)
    }
}

/// SID model from the command line or the debugger: 6581, 8580, 8580d
/// (8580 with digiboost).
pub fn parse_sid_model(s: &str) -> Option<(Model, bool)> {
    match s.to_ascii_lowercase().as_str() {
        "6581" => Some((Model::Mos6581, false)),
        "8580" => Some((Model::Mos8580, false)),
        "8580d" => Some((Model::Mos8580, true)),
        _ => None,
    }
}

/// Azimuth error in cycles (0-10, decimals allowed), in thousandths of a cycle.
pub fn parse_azimuth(s: &str) -> Option<u32> {
    let c: f64 = s.parse().ok()?;
    (0.0..=10.0).contains(&c).then(|| (c * 1000.0).round() as u32)
}

/// Hex address, with optional `$` or `0x`.
pub fn parse_address(s: &str) -> Option<u16> {
    let h = s.trim_start_matches('$').trim_start_matches("0x").trim_start_matches("0X");
    u16::from_str_radix(h, 16).ok()
}

/// On exit: cartridge saves (EEPROM) to the .nvram file, drive writes to the
/// D64, recordings to the TAP.
pub fn save_on_exit(c64: &mut C64) {
    match c64.save_tape_changes() {
        Ok(Some(path)) => eprintln!("Tape recordings written to {path}"),
        Ok(None) => {}
        Err(e) => eprintln!("{e}"),
    }
    match c64.save_cart_nvram() {
        Ok(Some(path)) => eprintln!("Cartridge saves written to {path}"),
        Ok(None) => {}
        Err(e) => eprintln!("{e}"),
    }
    match c64.save_disk_changes() {
        Ok(Some(path)) => eprintln!("Disk changes written to {path}"),
        Ok(None) => {}
        Err(e) => eprintln!("{e}"),
    }
}

// ── Real-time session ────────────────────────────────────────────────────────

/// Service keys handled by the frontend.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Hotkey {
    /// F12: quit.
    Quit,
    /// F10: fullscreen.
    Fullscreen,
}

/// Emulation at 50 frames per second with audio, gamepad, keyboard joystick
/// and turbo while loading. The frontend provides the input, calls
/// `run_due_frames` and draws `c64.framebuffer`.
/// Application name, at the start of the window title.
pub const APP_NAME: &str = "SampoSoft C64 Emulator";

/// Copyright and license notice printed when an interactive session starts,
/// as the GPL asks (section 2c) and as VICE does: on stderr for `c64` and
/// `c64term`, before the prompt of `c64dbg`.
pub const NOTICE: &str = "SampoSoft C64 Emulator - Copyright (C) 2026 SampoSoft - Francesco Sampoli. \
    Free software under the GNU GPL v2 or later, with ABSOLUTELY NO WARRANTY; see LICENSE and CREDITS.md.";

/// Text of `--version`: version, copyright, license and credits.
pub fn version_text() -> String {
    format!(
        "{APP_NAME} {}\n\
         Copyright (C) 2026 SampoSoft - Francesco Sampoli\n\
         Parts derived from VICE (Copyright (C) the VICE Team) and reSID (Copyright (C) 2010 Dag Lem): see CREDITS.md.\n\
         This program is free software, licensed under the GNU General Public License,\n\
         version 2 or (at your option) any later version: see LICENSE.\n\
         It comes with ABSOLUTELY NO WARRANTY.",
        env!("CARGO_PKG_VERSION")
    )
}

pub struct Session {
    pub c64: C64,
    audio: Option<AudioOut>,
    gamepad: Gamepad,
    joy_port: JoyPort,
    /// Media from the command line and the one inserted now (F8 switches to
    /// the next).
    media: Vec<String>,
    media_pos: usize,
    /// Turbo while loading from disk and tape (disabled by --no-turbo).
    turbo: Option<LoadDetector>,
    /// Loading in progress: frames run without waiting and without audio.
    loading: bool,
    next_frame: Instant,
    speed: SpeedMeter,
    /// Host mouse movement for the next frame, in C64 pixels.
    mouse: (i32, i32),
    /// Host pointer over the screen for the paddles, for the next frame.
    pointer: Option<(i32, i32)>,
    /// Remote monitor (`--remote`).
    remote: Option<RemoteMonitor>,
}

impl Session {
    /// Creates the machine, loads the given file and opens audio and gamepad.
    /// `turbo`: maximum speed while loading from disk and tape.
    pub fn new(opts: &MachineOptions, turbo: bool) -> Result<Self, String> {
        opts.check_media()?;
        // On the console only, as VICE does: not in the status bar
        eprintln!("{NOTICE}");
        let mut c64 = opts.build()?;
        if let Some(path) = opts.input_files.first() {
            match c64.load_file(path) {
                Ok(msg) => {
                    crate::notice!("{msg}");
                    if let Some(disk) = &c64.disk {
                        disk.print_directory();
                    }
                    if opts.input_files.len() > 1 {
                        eprintln!("F8 = next medium ({})", opts.input_files[1]);
                    } else if c64.disk.is_some() {
                        eprintln!("F8 = print disk directory");
                    }
                }
                Err(e) => crate::notice!("{e}"),
            }
        }

        let remote = match &opts.remote {
            Some(path) => {
                let r = RemoteMonitor::start(path)?;
                eprintln!("Remote monitor on {}", r.path().display());
                Some(r)
            }
            None => None,
        };

        let audio = AudioOut::open();
        match &audio {
            Some(a) => c64.set_audio(Some(a.rate()))?,
            None => crate::notice!("WARN: audio not available, no sound."),
        }

        Ok(Self {
            c64,
            audio,
            gamepad: Gamepad::new(),
            joy_port: JoyPort::Two,
            media: opts.input_files.clone(),
            media_pos: 0,
            turbo: turbo.then(LoadDetector::new),
            loading: false,
            next_frame: Instant::now(),
            speed: SpeedMeter::new(),
            mouse: (0, 0),
            pointer: None,
            remote,
        })
    }

    /// Loading in progress (turbo active).
    pub fn loading(&self) -> bool { self.loading && !self.paused() }

    /// Emulation paused by the remote monitor.
    pub fn paused(&self) -> bool {
        self.remote.as_ref().is_some_and(|r| r.paused())
    }

    /// Screen to show: the last frame, or while paused the one being built
    /// (up to the beam, as in `c64dbg`).
    pub fn screen(&self) -> &[u32] {
        if self.paused() { self.c64.live_framebuffer() } else { &self.c64.framebuffer }
    }

    /// Screen for the window and its CRT emulation, if on: then the frame
    /// as the VIC drew it (the GPU does the blending).
    pub fn display(&self) -> (&[u32], Option<super::window::CrtView>) {
        match self.c64.crt() {
            None => (self.screen(), None),
            Some(crt) => {
                let fb = if self.paused() { self.c64.live_framebuffer() } else { self.c64.last_frame() };
                (fb, Some(super::window::CrtView { crt, blend: self.c64.blend(), frame: self.c64.frame_count }))
            }
        }
    }

    /// Window title: application name, name of the medium in use (the file
    /// given on the command line, or the one F8 switched to) and the load
    /// turbo while it is on.
    pub fn title(&self) -> String {
        let mut title = APP_NAME.to_string();
        if let Some(path) = self.media.get(self.media_pos) {
            if let Some(name) = std::path::Path::new(path).file_name() {
                title += &format!(" — {}", name.to_string_lossy());
            }
        }
        if self.paused() {
            title += " — paused (remote monitor)";
        } else if self.loading {
            title += " — loading (turbo)";
        }
        title
    }

    /// State for the bar: machine, joystick port, gamepad, speed, media and
    /// last message; the key reminders are `help`.
    pub fn status(&self, help: &'static [&'static str]) -> Status {
        let mut s = Status::of(&self.c64);
        s.keyboard_port = self.joy_port;
        s.gamepad = self.gamepad.connected();
        if let Some(fps) = self.speed.fps() { s.fps = fps; }
        s.turbo = self.loading;
        s.paused = self.paused();
        s.media = (self.media.len() > 1).then(|| (self.media_pos + 1, self.media.len()));
        s.message = crate::notice::latest();
        s.help = help;
        s
    }

    /// Click on the status bar.
    pub fn click(&mut self, click: Click) {
        super::status::apply_click(&mut self.c64, click);
    }

    /// Time of the next frame at real speed.
    pub fn next_frame(&self) -> Instant { self.next_frame }

    /// Restarts from now, without catching up on lost frames.
    pub fn restart_clock(&mut self) { self.next_frame = Instant::now(); }

    /// Host key pressed or released. Auto-repeats are ignored. Handles F5/F9
    /// (state), F8 (next medium, or disk directory), F11 (reset) and TAB
    /// (joystick port) by itself; returns the keys that belong to the
    /// frontend.
    pub fn host_key(&mut self, hk: HostKey, pressed: bool, repeat: bool) -> Option<Hotkey> {
        if pressed && repeat { return None; }
        if pressed {
            match hk {
                HostKey::F12 => return Some(Hotkey::Quit),
                HostKey::F10 => return Some(Hotkey::Fullscreen),
                HostKey::F11 => {
                    self.c64.reset();
                    if let Some(r) = &mut self.remote { r.resume(); }
                    crate::notice!("Hardware reset.");
                    return None;
                }
                HostKey::F8 => {
                    if self.media.len() > 1 {
                        self.media_pos = (self.media_pos + 1) % self.media.len();
                        match self.c64.change_medium(&self.media[self.media_pos]) {
                            Ok(msg) => crate::notice!("{msg}"),
                            Err(e) => crate::notice!("{e}"),
                        }
                    }
                    if let Some(disk) = &self.c64.disk { disk.print_directory(); }
                    else if self.media.len() <= 1 { crate::notice!("No disk mounted."); }
                    return None;
                }
                HostKey::F5 => {
                    match self.c64.save_state("c64_state.sav") {
                        Ok(_)  => crate::notice!("State saved to c64_state.sav"),
                        Err(e) => crate::notice!("Save error: {e}"),
                    }
                    return None;
                }
                HostKey::F9 => {
                    match self.c64.load_state("c64_state.sav") {
                        Ok(_)  => crate::notice!("State loaded from c64_state.sav"),
                        Err(e) => crate::notice!("Load error: {e}"),
                    }
                    return None;
                }
                HostKey::Tab => {
                    self.joy_port = self.joy_port.next();
                    set_joy_port(&mut self.c64, self.joy_port);
                    return None;
                }
                _ => {}
            }
        }
        apply_host_key(&mut self.c64, self.joy_port, hk, pressed);
        None
    }

    /// Paddles and 1351 mouse in the control ports.
    pub fn analog(&self) -> super::window::Analog {
        super::window::Analog { paddles: self.c64.bus.ctrl.has_paddles(), mouse: self.c64.bus.ctrl.has_mouse() }
    }

    /// Host pointer at `(x, y)` in the framebuffer, for the paddles: applied
    /// at the start of the next frame.
    pub fn point_paddles(&mut self, x: i32, y: i32) {
        self.pointer = Some((x, y));
    }

    /// Host mouse movement in C64 pixels (Y downwards), applied at the start
    /// of the next frame.
    pub fn mouse_move(&mut self, dx: i32, dy: i32) {
        self.mouse.0 += dx;
        self.mouse.1 += dy;
    }

    /// Host mouse button, for the paddles and the 1351 mouse.
    pub fn mouse_button(&mut self, b: crate::ctrlport::Button, pressed: bool) {
        self.c64.mouse_button(b, pressed);
    }

    /// Description of the analog devices, for the capture message.
    pub fn mouse_devices(&self) -> String {
        let d = self.c64.bus.ctrl.devices;
        (0..2).filter(|&i| d[i].is_analog())
            .map(|i| format!("{} in port {}", d[i].name(), i + 1))
            .collect::<Vec<_>>().join(", ")
    }

    /// Printable character to type, one per frame (pasted text, the
    /// terminal).
    pub fn type_char(&mut self, c: char) {
        self.c64.bus.keyboard.push_char(c);
    }

    /// Host key `id` pressed with the printable character `c` (operating
    /// system layout): the C64 keys stay down until `char_up(id)`.
    pub fn char_down(&mut self, id: u64, c: char) {
        self.c64.bus.keyboard.char_down(id, c);
    }

    pub fn char_up(&mut self, id: u64) {
        self.c64.bus.keyboard.char_up(id);
    }

    /// The window lost the focus: no key-up will arrive.
    pub fn release_chars(&mut self) {
        self.c64.bus.keyboard.release_chars();
    }

    /// Runs the frames due at this moment; true if there is something new
    /// to draw. While loading it runs frames back to back for one frame of
    /// real time, then returns to the frontend for input and drawing. The
    /// remote monitor's commands are executed before each frame, and every
    /// frame while paused.
    pub fn run_due_frames(&mut self) -> bool {
        let polled = self.poll_remote();
        let frame = super::frame(&self.c64);
        if self.paused() {
            // Awake every frame, to serve the remote monitor
            self.next_frame = Instant::now() + frame;
            return polled;
        }
        if self.loading {
            let until = Instant::now() + frame;
            while self.loading && Instant::now() < until {
                if !self.step_frame() { break; }
            }
            self.next_frame = Instant::now();
            return true;
        }
        let now = Instant::now();
        // If we are far behind (hidden window, sleep) restart from now
        if now.duration_since(self.next_frame) > frame * 10 {
            self.next_frame = now;
        }
        let mut ran = 0;
        while Instant::now() >= self.next_frame && ran < 3 && !self.loading {
            let done = self.step_frame();
            self.next_frame += frame;
            ran += 1;
            if !done { break; }
        }
        ran > 0 || polled
    }

    fn poll_remote(&mut self) -> bool {
        match &mut self.remote {
            Some(r) => r.poll(&mut self.c64),
            None => false,
        }
    }

    /// One emulation frame; updates the turbo state. False if the remote
    /// monitor paused the machine before the end of the frame.
    fn step_frame(&mut self) -> bool {
        self.poll_remote();
        // During the turbo the SID computes only the digital part
        if let Some(audio) = &self.audio {
            let _ = self.c64.set_audio((!self.loading).then(|| audio.rate()));
        }
        self.gamepad.poll(&mut self.c64.bus.joy2);
        if self.mouse != (0, 0) {
            self.c64.mouse_move(self.mouse.0, self.mouse.1);
            self.mouse = (0, 0);
        }
        if let Some((x, y)) = self.pointer.take() {
            self.c64.point_paddles(x, y);
        }
        let done = match &mut self.remote {
            Some(r) => r.run_frame(&mut self.c64),
            None => { self.c64.run_frame(); true }
        };
        if !done {
            return false;
        }
        self.c64.bus.keyboard.advance_typing();
        if !self.loading {
            if let Some(audio) = &mut self.audio {
                audio.push(&self.c64.audio_buf, self.c64.audio_channels());
            }
        }
        self.loading = self.turbo.as_mut().is_some_and(|t| t.update(&self.c64));
        self.speed.frame();
        true
    }
}
