// SPDX-License-Identifier: GPL-2.0-or-later
// Copyright (C) 2026 SampoSoft - Francesco Sampoli

use crate::{cpu::{Cpu, Flags}, mem::Bus, disk::D64, cart, reu::DmaCycle};
use crate::snapshot::{impl_state, impl_state_enum, Reader, State, Writer};
use std::path::{Path, PathBuf};

use crate::timing::Standard;
use crate::vic::Chip;

/// Kind of queued PRG.
#[derive(Clone, Copy, PartialEq)]
enum PrgKind {
    Basic,   // loads at $0801, the user types RUN
    Machine, // ML load, auto-run via SYS
}

/// Hooks used by the headless debugger. All off by default: no cost
/// during normal execution.
#[derive(Default)]
pub struct DebugHooks {
    /// Raster line to stop at (at cycle 0 of the line).
    pub raster_break: Option<u16>,
    /// Set by the core when `raster_break` has been reached.
    pub raster_hit: bool,
}

/// Result of `step_instruction`.
#[derive(Clone, Copy)]
pub struct StepResult {
    /// CPU cycles of the instruction (0 if a KERNAL trap was served).
    pub cycles: u8,
    /// Machine cycles elapsed, including those stolen by the VIC.
    pub elapsed: u32,
    /// True if this instruction completed a frame (housekeeping done).
    pub frame_done: bool,
}

pub struct C64 {
    /// VIC-II of the machine, which sets its video standard (`set_chip`).
    chip: Chip,
    pub cpu: Cpu,
    pub bus: Bus,
    /// Last complete frame (background + sprites): the one to present.
    pub framebuffer: Vec<u32>,
    /// Frame being built, line by line.
    work_fb: Vec<u32>,
    /// Frame blending (`set_blend`): the previous frame as the VIC drew it.
    blend: Option<Vec<u32>>,
    /// HDR output of the CRT emulation (`set_hdr`).
    hdr: bool,
    /// CRT monitor emulation of the window frontends (`set_crt`).
    crt: Option<crate::crt::Crt>,

    // Direct PRG (no disk)
    pending_prg: Option<Vec<u8>>,
    prg_kind: PrgKind,
    injected: bool,
    inject_countdown: Option<u8>,

    // Mounted D64 image
    pub disk: Option<D64>,
    /// File of the mounted disk, to write back the changes made by the drive.
    disk_path: String,
    disk_autoload: bool,    // true = still have to inject the LOAD"*",8,1 + RUN command
    /// File of the inserted tape, to write the recordings back to (or of
    /// the T64 image, which is never written).
    tape_path: String,
    /// T64 image in place of the tape: the KERNAL tape routines read its
    /// files (`t64_trap`).
    t64: Option<crate::tape::T64>,
    /// Inserted tape to load: LOAD, PLAY and then RUN.
    tape_autoload: bool,
    /// Datasette keys pressed automatically when the KERNAL asks for
    /// PRESS PLAY ON TAPE or PRESS RECORD & PLAY ON TAPE.
    pub tape_auto_buttons: bool,
    /// Tape boot in progress: frames left to see "FOUND" and press C=,
    /// which skips the KERNAL pause (0: nothing to do).
    tape_found_wait: u16,
    /// Frames during which C= stays pressed (0: released).
    tape_cbm_key: u8,
    /// Frames to wait, once LOAD has been typed, before putting RUN in the
    /// KERNAL keyboard buffer (0 = nothing to do).
    autoload_run: u8,
    /// Address to start with SYS instead of RUN after the autoload (0:
    /// RUN), for T64 programs that are not BASIC.
    autoload_sys: u16,

    /// Audio of the last frame at the rate chosen with `set_audio`: mono,
    /// or interleaved stereo (SID 1 left, SID 2 right) with the second
    /// SID, see `audio_channels`.
    pub audio_buf: Vec<i16>,
    /// Samples of the second SID to interleave.
    audio_buf2: Vec<i16>,


    // Cycles accumulated in the current frame (for 50 Hz frame pacing)
    frame_elapsed: u32,
    /// VIC frames rendered since start-up.
    pub frame_count: u64,

    pub dbg: DebugHooks,
}

impl C64 {
    pub fn new() -> Self {
        use crate::vic::{WIDTH, HEIGHT};
        Self {
            cpu: Cpu::new(),
            bus: Bus::new(),
            chip: Chip::Mos6569,
            framebuffer: vec![0xFF000000; WIDTH * HEIGHT],
            work_fb: vec![0xFF000000; WIDTH * HEIGHT],
            blend: None,
            hdr: false,
            crt: None,
            pending_prg: None,
            prg_kind: PrgKind::Basic,
            injected: false,
            inject_countdown: None,
            disk: None,
            disk_path: String::new(),
            disk_autoload: false,
            tape_path: String::new(),
            t64: None,
            tape_autoload: false,
            tape_auto_buttons: true,
            tape_found_wait: 0,
            tape_cbm_key: 0,
            autoload_run: 0,
            autoload_sys: 0,
            audio_buf: Vec::new(),
            audio_buf2: Vec::new(),
            frame_elapsed: 0,
            frame_count: 0,
            dbg: DebugHooks::default(),
        }
    }

    /// Frame blending: `framebuffer` becomes the average, in linear light,
    /// of the last two frames. It shows what the eye sees on a 50 Hz CRT
    /// when a picture alternates two frames (interlace, color mixing),
    /// which a 60 or 120 Hz monitor would show as uneven flicker instead;
    /// moving objects leave a half-bright trail. Off by default.
    pub fn set_blend(&mut self, on: bool) {
        self.blend = on.then(|| self.work_fb.clone());
    }

    pub fn blend(&self) -> bool {
        self.blend.is_some()
    }

    /// HDR output of the CRT emulation, a setting of the window frontends:
    /// on a display with headroom above white (Apple EDR) the slot mask is
    /// shown at its full depth, its stripes brighter than white. Off by
    /// default.
    pub fn set_hdr(&mut self, on: bool) {
        self.hdr = on;
    }

    pub fn hdr(&self) -> bool {
        self.hdr
    }

    pub fn standard(&self) -> Standard {
        self.chip.standard()
    }

    pub fn chip(&self) -> Chip {
        self.chip
    }

    /// The machine's usual VIC-II for a video standard (`set_chip`): the
    /// 6569 for PAL, the 6567R8 for NTSC.
    pub fn set_standard(&mut self, standard: Standard) -> Result<(), String> {
        self.set_chip(Chip::of(standard))
    }

    /// VIC-II of the machine (6569 by default), with the video standard,
    /// clock, raster and framebuffer size (`fb_height`) that go with it; the
    /// CIAs' TOD inputs (50 or 60 Hz mains), the SID, the drive and the
    /// Datasette follow the clock. The machine is reset: as changing the
    /// crystal and the VIC-II of a real C64.
    pub fn set_chip(&mut self, chip: Chip) -> Result<(), String> {
        self.chip = chip;
        self.bus.vic.chip = chip;
        self.bus.vic.standard = chip.standard();
        let len = chip.standard().fb_len();
        self.framebuffer = vec![0xFF000000; len];
        self.work_fb = vec![0xFF000000; len];
        if let Some(prev) = &mut self.blend {
            *prev = vec![0xFF000000; len];
        }
        self.frame_elapsed = 0;
        self.apply_clocks()?;
        self.reset();
        Ok(())
    }

    /// Framebuffer height (the width is `vic::WIDTH`): 284 PAL, 253 NTSC.
    pub fn fb_height(&self) -> usize {
        self.standard().fb_height()
    }

    /// Gives the clock of the standard to the parts timed by it.
    fn apply_clocks(&mut self) -> Result<(), String> {
        let s = self.standard();
        let hz = s.clock_hz();
        self.bus.cia1.set_tod_input(hz, s.mains_hz());
        self.bus.cia2.set_tod_input(hz, s.mains_hz());
        self.bus.sid.set_clock(hz)?;
        if let Some(sid2) = &mut self.bus.sid2 {
            sid2.set_clock(hz)?;
        }
        if let Some(drive) = &mut self.bus.drive {
            drive.set_c64_clock(hz);
        }
        self.bus.tape.set_clock(hz);
        Ok(())
    }

    /// The last complete frame as the VIC drew it, also with frame
    /// blending on (for the CRT emulation, which blends on its own).
    pub fn last_frame(&self) -> &[u32] {
        self.blend.as_deref().unwrap_or(&self.framebuffer)
    }

    /// CRT monitor emulation (`crate::crt`): a setting of the window
    /// frontends, which draw the screen through it on the GPU. Off by
    /// default.
    pub fn set_crt(&mut self, crt: Option<crate::crt::Crt>) {
        self.crt = crt;
    }

    pub fn crt(&self) -> Option<crate::crt::Crt> {
        self.crt
    }

    /// Framebuffer being built: the current frame up to the VIC beam, over
    /// the previous frame (as VICE shows it when paused).
    pub fn live_framebuffer(&self) -> &[u32] {
        &self.work_fb
    }

    pub fn load_roms(&mut self, kernal: &[u8], basic: &[u8], char_rom: &[u8]) {
        self.bus.load_kernal(kernal);
        self.bus.load_basic(basic);
        self.bus.load_char(char_rom);
    }

    /// ROM directory: `<executable dir>/roms` if it exists, otherwise `./roms`.
    /// `roms` next to the executable or in one of its parent directories
    /// (target/release → the project), otherwise `roms` in the current
    /// directory: the emulator also finds its ROMs when started from
    /// elsewhere, as `c64mcp` does.
    pub fn default_roms_dir() -> PathBuf {
        let exe = std::env::current_exe().ok().and_then(|p| p.canonicalize().ok());
        exe.iter()
            .flat_map(|p| p.ancestors().skip(1))
            .map(|d| d.join("roms"))
            .find(|p| p.is_dir())
            .unwrap_or_else(|| PathBuf::from("roms"))
    }

    /// Loads kernal.rom, basic.rom and chargen.rom from `dir`.
    /// Returns the warnings (missing files or unexpected size).
    pub fn load_roms_from_dir(&mut self, dir: &Path) -> Vec<String> {
        let mut warnings = Vec::new();
        let mut load = |name: &str, expected: usize| -> Vec<u8> {
            let path = dir.join(name);
            match std::fs::read(&path) {
                Ok(data) => {
                    if data.len() != expected {
                        warnings.push(format!("{}: expected {expected} bytes, found {}", path.display(), data.len()));
                    }
                    data
                }
                Err(_) => {
                    warnings.push(format!("ROM not found: {}", path.display()));
                    vec![0u8; expected]
                }
            }
        };
        let kernal  = load("kernal.rom",  0x2000);
        let basic   = load("basic.rom",   0x2000);
        let charrom = load("chargen.rom", 0x1000);
        self.load_roms(&kernal, &basic, &charrom);
        warnings
    }

    pub fn reset(&mut self) {
        // The reset reaches the cartridge too: registers cleared, bank 0
        if let Some(cart) = &mut self.bus.cart {
            cart.reset();
        }
        if let Some(reu) = &mut self.bus.reu {
            reu.reset();
        }
        if let Some(drive) = &mut self.bus.drive {
            drive.reset();
        }
        // The Datasette stops and rewinds (VICE's DatasetteResetWithCPU)
        self.bus.tape.reset();
        if let Some(t64) = &mut self.t64 {
            t64.rewind();
        }
        // The RES line reaches both CIAs and the SIDs; the VIC, which has no
        // reset pin, restarts from line 0 as in VICE (last, as VICE does)
        self.bus.cia1.reset();
        self.bus.cia2.reset();
        self.bus.pot_mask_changed();
        self.bus.sid.reset();
        if let Some(sid2) = &mut self.bus.sid2 {
            sid2.reset();
        }
        self.bus.vic.reset();
        // Processor port: all inputs, so the reset vector comes from the
        // KERNAL whatever the program had banked in
        self.bus.reset_cpu_port();
        self.cpu.reset(&mut self.bus);
        self.injected = false;
        self.inject_countdown = None;
        // 6510 reset sequence: the clock advances 6 cycles before the first
        // fetch (as in VICE). The VIC starts one cycle behind the CPU clock:
        // that is the phase at which, in VICE, line 0 arrives in the KERNAL
        // wait loop (calibrated on the trace).
        for i in 0..6 {
            if i > 0 { self.tick_vic(); }
            self.bus.cia1.tick(1);
            self.bus.cia2.tick(1);
            self.tick_drive();
            self.cpu.total_cycles += 1;
        }
    }

    /// Loads a file according to its extension (.crt, .d64, .g64, .tap, .t64,
    /// .prg or anything else = prg).
    /// Returns a descriptive message.
    pub fn load_file(&mut self, path: &str) -> Result<String, String> {
        let ext = Path::new(path)
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("")
            .to_lowercase();
        self.autoload_sys = 0;
        if ext == "tap" {
            return if self.insert_tape_file(path)? {
                Ok(format!("Blank tape: {path} (the file is created on the first SAVE)"))
            } else {
                self.tape_autoload = true;
                Ok(format!("Tape inserted: {path} (auto-load LOAD + PLAY + RUN)"))
            };
        }
        let data = std::fs::read(path).map_err(|e| format!("Cannot read {path}: {e}"))?;
        match ext.as_str() {
            "crt" => {
                self.load_cartridge(&data).map_err(|e| format!("CRT error: {e}"))?;
                let cart = self.bus.cart.as_mut().unwrap();
                let mut msg = format!("Cartridge loaded: {path} — \"{}\", type {} ({}), mode {}",
                    cart.name, cart.kind.crt_type(), cart.kind.name(), cart.mode.name());
                // EEPROM and non-volatile memories: in the .nvram file next to the CRT
                if cart.has_nvram() {
                    let nv = Path::new(path).with_extension("nvram");
                    cart.nvram_path = nv.to_string_lossy().into_owned();
                    match std::fs::read(&nv) {
                        Ok(data) => match cart.load_nvram(&data) {
                            Ok(()) => msg += &format!("; saves from {}", nv.display()),
                            Err(e) => msg += &format!("; {} ignored: {e}", nv.display()),
                        },
                        Err(_) => msg += &format!("; saves in {}", nv.display()),
                    }
                }
                Ok(msg)
            }
            "d64" => {
                self.mount_disk(data).map_err(|e| format!("D64 error: {e}"))?;
                self.disk_path = path.to_string();
                let how = if self.bus.drive.is_some() { "drive 1541" } else { "fast load" };
                Ok(format!("Disk mounted: {path} ({how}, auto-load LOAD\"*\",8,1 + RUN)"))
            }
            "g64" => {
                let drive = self.bus.drive.as_mut()
                    .ok_or("G64 images require the 1541 drive (the dos1541.rom ROM is missing)")?;
                drive.insert(crate::drive::gcr::Disk::from_g64(&data)?);
                self.disk = None;
                self.disk_path = path.to_string();
                self.disk_autoload = true;
                Ok(format!("G64 disk inserted: {path} (auto-load LOAD\"*\",8,1 + RUN)"))
            }
            "t64" => {
                let t64 = crate::tape::T64::from_bytes(&data).map_err(|e| format!("T64 error: {e}"))?;
                let first = &t64.files[0];
                // BASIC programs start with RUN, the others with SYS at
                // their address (as a PRG)
                self.autoload_sys = if first.start == 0x0801 { 0 } else { first.start };
                let msg = format!("T64 inserted: {path} — {} program(s), the first \"{}\" ${:04X}–${:04X} (auto-load LOAD + PLAY + {})",
                    t64.files.len(), first.display_name(), first.start, first.end,
                    if self.autoload_sys == 0 { "RUN".to_string() } else { format!("SYS{}", first.start) });
                self.insert_t64(t64, path);
                self.tape_autoload = true;
                Ok(msg)
            }
            _ => {
                let (lo, hi) = self.queue_prg(data).map_err(|e| format!("PRG error: {e}"))?;
                Ok(format!("PRG queued: {path} — ${lo:04X}–${hi:04X}"))
            }
        }
    }

    /// Inserts a TAP tape (Datasette stopped, tape at the start),
    /// with no file to save the recordings to.
    pub fn insert_tape(&mut self, data: &[u8]) -> Result<(), String> {
        let tap = crate::tape::Tap::from_bytes(data).map_err(|e| format!("TAP error: {e}"))?;
        self.replace_tape(tap, String::new());
        Ok(())
    }

    /// Inserts the tape from file `path`, or a blank tape if the file does
    /// not exist (true in that case): the recordings end up in the file
    /// with `save_tape_changes`.
    pub fn insert_tape_file(&mut self, path: &str) -> Result<bool, String> {
        let (tap, blank) = match std::fs::read(path) {
            Ok(data) => (crate::tape::Tap::from_bytes(&data).map_err(|e| format!("TAP error: {e}"))?, false),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                // As VICE, the header says the machine's video standard
                let mut tap = crate::tape::Tap::blank();
                tap.video = match self.standard() {
                    Standard::Pal => 0,
                    Standard::Ntsc => 1,
                    Standard::NtscOld => 2,
                    Standard::PalN => 3,
                };
                (tap, true)
            }
            Err(e) => return Err(format!("Cannot read {path}: {e}")),
        };
        self.replace_tape(tap, path.to_string());
        Ok(blank)
    }

    /// File of the inserted disk ("" if none).
    pub fn disk_path(&self) -> &str {
        &self.disk_path
    }

    /// File of the inserted tape or T64 ("" if it does not come from a
    /// file).
    pub fn tape_path(&self) -> &str {
        &self.tape_path
    }

    /// Inserted T64 image.
    pub fn t64(&self) -> Option<&crate::tape::T64> {
        self.t64.as_ref()
    }

    /// Changes tape after saving the recordings of the inserted one.
    fn replace_tape(&mut self, tap: crate::tape::Tap, path: String) {
        match self.save_tape_changes() {
            Ok(Some(saved)) => crate::notice!("Recordings written to {saved}"),
            Ok(None) => {}
            Err(e) => eprintln!("{e}"),
        }
        self.bus.tape.insert(tap);
        self.t64 = None;
        self.tape_path = path;
    }

    /// Inserts a T64 image in place of the tape, rewound: the Datasette
    /// is left empty (as in VICE) and the KERNAL tape routines read the
    /// image's files.
    pub fn insert_t64(&mut self, t64: crate::tape::T64, path: &str) {
        match self.eject_tape() {
            Ok(Some(saved)) => crate::notice!("Recordings written to {saved}"),
            Ok(None) => {}
            Err(e) => eprintln!("{e}"),
        }
        self.t64 = Some(t64);
        self.tape_path = path.to_string();
    }

    /// Reads the T64 image `path` and inserts it (`insert_t64`).
    pub fn insert_t64_file(&mut self, path: &str) -> Result<(), String> {
        let data = std::fs::read(path).map_err(|e| format!("Cannot read {path}: {e}"))?;
        let t64 = crate::tape::T64::from_bytes(&data).map_err(|e| format!("T64 error: {e}"))?;
        self.insert_t64(t64, path);
        Ok(())
    }

    /// Ejects the tape or the T64, after saving the tape's recordings.
    pub fn eject_tape(&mut self) -> Result<Option<String>, String> {
        let saved = self.save_tape_changes()?;
        self.bus.tape.eject();
        self.t64 = None;
        self.tape_path.clear();
        Ok(saved)
    }

    /// Rewinds the tape, or the T64, to the start instantly.
    pub fn rewind_tape(&mut self) {
        self.bus.tape.rewind();
        if let Some(t64) = &mut self.t64 {
            t64.rewind();
        }
    }

    /// Writes what was recorded on the tape to its file, if anything.
    /// Returns the file written.
    pub fn save_tape_changes(&mut self) -> Result<Option<String>, String> {
        let Some(tap) = self.bus.tape.image.as_mut() else { return Ok(None) };
        if !tap.changed || self.tape_path.is_empty() {
            return Ok(None);
        }
        std::fs::write(&self.tape_path, tap.to_bytes())
            .map_err(|e| format!("Cannot save {}: {e}", self.tape_path))?;
        tap.changed = false;
        Ok(Some(self.tape_path.clone()))
    }

    /// Loads a cartridge from CRT data. Returns Err if the format is
    /// invalid or the type is not supported.
    pub fn load_cartridge(&mut self, data: &[u8]) -> Result<(), String> {
        // Saves of the previous cartridge
        if let Err(e) = self.save_cart_nvram() {
            eprintln!("{e}");
        }
        self.bus.cart = Some(cart::parse_crt(data)?);
        // Full reset to start the cartridge, as when it is inserted with
        // the machine off: CIAs, VIC and processor port too, so the RESET
        // vector is read from the ROM whatever was banked in
        self.reset();
        self.injected = true; // do not inject a PRG if there is a cart
        Ok(())
    }

    /// Writes the cartridge EEPROM and non-volatile memories to their file,
    /// if they changed. Returns the file written.
    pub fn save_cart_nvram(&mut self) -> Result<Option<String>, String> {
        let Some(cart) = self.bus.cart.as_mut() else { return Ok(None) };
        if !cart.has_nvram() || !cart.nvram_dirty() || cart.nvram_path.is_empty() {
            return Ok(None);
        }
        std::fs::write(&cart.nvram_path, cart.nvram())
            .map_err(|e| format!("Cannot save {}: {e}", cart.nvram_path))?;
        cart.clear_nvram_dirty();
        Ok(Some(cart.nvram_path.clone()))
    }

    /// Mounts a D64 image. Returns Err if the format is invalid.
    pub fn mount_disk(&mut self, data: Vec<u8>) -> Result<(), String> {
        if let Some(drive) = self.bus.drive.as_mut() {
            drive.insert(crate::drive::gcr::Disk::from_d64(&data)?);
        }
        match D64::from_bytes(data) {
            Some(d) => { self.disk = Some(d); self.disk_autoload = true; Ok(()) }
            None    => Err("invalid or too short D64 file".into()),
        }
    }

    /// Inserts a D64 or G64 disk into the drive without auto-loading.
    pub fn insert_disk(&mut self, path: &str) -> Result<String, String> {
        let data = std::fs::read(path).map_err(|e| format!("Cannot read {path}: {e}"))?;
        let drive = self.bus.drive.as_mut().ok_or("1541 drive not emulated")?;
        let g64 = data.starts_with(b"GCR-1541");
        let disk = if g64 { crate::drive::gcr::Disk::from_g64(&data)? } else { crate::drive::gcr::Disk::from_d64(&data)? };
        drive.insert(disk);
        self.disk = if g64 { None } else { D64::from_bytes(data) };
        self.disk_path = path.to_string();
        Ok(format!("Disk inserted into the drive: {path}"))
    }

    /// Medium change with the machine on, as a person would do it: a tape
    /// (.tap) goes in rewound with PLAY pressed (side B, next tape); a disk
    /// (.d64, .g64) replaces the one in the drive after saving its changes,
    /// without auto-loading.
    pub fn change_medium(&mut self, path: &str) -> Result<String, String> {
        let ext = Path::new(path).extension().and_then(|e| e.to_str()).unwrap_or("").to_lowercase();
        match ext.as_str() {
            "tap" => {
                if self.insert_tape_file(path)? {
                    return Ok(format!("Blank tape inserted: {path}"));
                }
                self.bus.tape.play(self.bus.cycle);
                Ok(format!("Tape inserted: {path} (PLAY pressed)"))
            }
            "t64" => {
                self.insert_t64_file(path)?;
                self.bus.tape.play(self.bus.cycle);
                Ok(format!("T64 inserted: {path} (PLAY pressed)"))
            }
            "d64" | "g64" => {
                if let Some(saved) = self.save_disk_changes()? {
                    crate::notice!("Disk changes written to {saved}");
                }
                if self.bus.drive.is_some() {
                    self.insert_disk(path)
                } else {
                    // Without a drive, D64 images are read through the KERNAL trap
                    let data = std::fs::read(path).map_err(|e| format!("Cannot read {path}: {e}"))?;
                    self.disk = Some(D64::from_bytes(data).ok_or("invalid D64")?);
                    self.disk_path = path.to_string();
                    Ok(format!("Disk inserted: {path}"))
                }
            }
            _ => Err(format!("{path}: only tapes (.tap, .t64) and disks (.d64, .g64) can be changed")),
        }
    }

    /// Attaches the 1541 drive with the DOS ROM in `dir` (dos1541.rom, or with
    /// the VICE file names). Without it, D64 images load through the KERNAL trap.
    pub fn attach_drive_from_dir(&mut self, dir: &Path) -> Result<(), String> {
        const NAMES: [&str; 3] = ["dos1541.rom", "1541.rom", "dos1541-325302-01+901229-05.bin"];
        let rom = NAMES.iter().find_map(|n| std::fs::read(dir.join(n)).ok())
            .ok_or_else(|| format!("1541 ROM not found in {} ({})", dir.display(), NAMES.join(", ")))?;
        let mut drive = Box::new(crate::drive::Drive::new(&rom, 8)?);
        drive.set_c64_clock(self.standard().clock_hz());
        self.bus.drive = Some(drive);
        Ok(())
    }

    /// Writes the changes made by the drive (game saves, SAVE) to the disk
    /// file, if there are any. Returns the file written.
    pub fn save_disk_changes(&mut self) -> Result<Option<String>, String> {
        let Some(drive) = self.bus.drive.as_mut() else { return Ok(None) };
        let Some(disk) = drive.bus.mech.disk.as_mut() else { return Ok(None) };
        if !disk.dirty || self.disk_path.is_empty() {
            return Ok(None);
        }
        // A D64 stays a D64 (sectors decoded from GCR, the rest of the
        // original file unchanged), a G64 is rewritten track by track
        let data = match (disk.from_d64, self.disk.as_ref()) {
            (true, Some(d64)) => disk.to_d64(d64.bytes()),
            (true, None) => return Ok(None),
            (false, _) => disk.to_g64(),
        };
        std::fs::write(&self.disk_path, &data)
            .map_err(|e| format!("Cannot save {}: {e}", self.disk_path))?;
        disk.dirty = false;
        if disk.from_d64 {
            self.disk = D64::from_bytes(data);
        }
        Ok(Some(self.disk_path.clone()))
    }

    /// Queues a .prg to be loaded automatically after the KERNAL boot.
    pub fn queue_prg(&mut self, data: Vec<u8>) -> Result<(u16, u16), &'static str> {
        if data.len() < 3 {
            return Err(".prg file too short");
        }
        let load_addr = u16::from_le_bytes([data[0], data[1]]);
        let end_usize = (load_addr as usize).saturating_add(data.len().saturating_sub(2));
        let end_addr = end_usize.min(0x10000) as u16;

        self.prg_kind = if load_addr == 0x0801 { PrgKind::Basic } else { PrgKind::Machine };
        self.pending_prg = Some(data);
        self.injected = false;
        self.inject_countdown = None;
        Ok((load_addr, end_addr))
    }

    /// True if a PRG is waiting to be injected after boot.
    pub fn prg_pending(&self) -> bool {
        !self.injected && self.pending_prg.is_some()
    }

    /// Loads the PRG into RAM and updates the pointers / auto-run.
    fn inject_prg(&mut self) {
        let data = match self.pending_prg.take() {
            Some(d) => d,
            None => return,
        };
        let load_addr = u16::from_le_bytes([data[0], data[1]]);
        let body = &data[2..];

        let start = load_addr as usize;
        let available = 0x10000usize.saturating_sub(start);
        let to_copy = body.len().min(available);
        self.bus.ram[start..start + to_copy].copy_from_slice(&body[..to_copy]);
        if to_copy < body.len() {
            eprintln!("WARN: PRG truncated, {} bytes past $FFFF", body.len() - to_copy);
        }
        let end_addr = (start + to_copy).min(0x10000) as u16;

        match self.prg_kind {
            PrgKind::Basic => {
                let lo = (load_addr & 0xFF) as u8;
                let hi = (load_addr >> 8) as u8;
                let elo = (end_addr & 0xFF) as u8;
                let ehi = (end_addr >> 8) as u8;
                self.bus.ram[0x2B] = lo;  self.bus.ram[0x2C] = hi;
                self.bus.ram[0x2D] = elo; self.bus.ram[0x2E] = ehi;
                self.bus.ram[0x2F] = elo; self.bus.ram[0x30] = ehi;
                self.bus.ram[0x31] = elo; self.bus.ram[0x32] = ehi;
            }
            PrgKind::Machine => {
                let cmd = format!("SYS{}\r", load_addr);
                let bytes: Vec<u8> = cmd.bytes().take(10).collect();
                for (i, &b) in bytes.iter().enumerate() {
                    self.bus.ram[0x0277 + i] = b;
                }
                self.bus.ram[0x00C6] = bytes.len() as u8;
                eprintln!("Auto-run: {}", cmd.trim_end_matches('\r'));
            }
        }

        self.injected = true;
    }

    // ── KERNAL trap ────────────────────────────────────────────────────────────

    /// Intercepts `JSR $FFD5` (KERNAL LOAD) and serves the file from the mounted D64.
    /// Returns `true` if the trap was handled (the CPU must not execute).
    fn handle_kernal_load(&mut self) -> bool {
        let disk = match &self.disk {
            Some(d) => d,
            None    => return false,
        };

        // Read KERNAL variables from zero page (written by SETNAM/SETLFS)
        let fname_len  = self.bus.ram[0xB7] as usize;
        let fname_lo   = self.bus.ram[0xBB] as usize;
        let fname_hi   = self.bus.ram[0xBC] as usize;
        let fname_ptr  = fname_lo | (fname_hi << 8);
        let device     = self.bus.ram[0xBA];
        let sec_addr   = self.bus.ram[0xB9];
        let load_mode  = self.cpu.a; // 0=load 1=verify

        // Device 8 (disk) only
        if device != 8 {
            return false;
        }

        // Verify mode (A=1): there is no real drive, report success and return
        if load_mode != 0 {
            self.cpu.p.set(Flags::C, false);
            Self::kernal_rts(&mut self.cpu, &self.bus.ram);
            return true;
        }

        // Extract the file name from RAM (raw PETSCII)
        let raw_name: Vec<u8> = (0..fname_len)
            .map(|i| self.bus.ram[(fname_ptr + i) & 0xFFFF])
            .collect();
        let display_name: String = raw_name.iter()
            .map(|&b| if b.is_ascii_graphic() { b as char } else { '?' })
            .collect();

        crate::notice!("KERNAL LOAD: \"{}\" from device {}", display_name, device);

        match disk.find_file(&raw_name) {
            None => {
                eprintln!("  → file not found in the D64");
                self.cpu.p.set(Flags::C, true);
                self.cpu.a = 4; // FILE NOT FOUND
                Self::kernal_rts(&mut self.cpu, &self.bus.ram);
                true
            }
            Some(entry) => {
                let file_data = disk.read_file_data(&entry);

                // PRG files start with the 2-byte load address
                let (load_addr, body) = if file_data.len() >= 2 {
                    let addr = u16::from_le_bytes([file_data[0], file_data[1]]);
                    (addr, &file_data[2..])
                } else {
                    eprintln!("  → empty file");
                    self.cpu.p.set(Flags::C, true);
                    self.cpu.a = 5;
                    Self::kernal_rts(&mut self.cpu, &self.bus.ram);
                    return true;
                };

                // If secondary address == 0: use the address in X/Y (CPU registers)
                // If secondary address != 0: use the address in the file
                let dest = if sec_addr == 0 {
                    u16::from_le_bytes([self.cpu.x, self.cpu.y])
                } else {
                    load_addr
                };

                let start = dest as usize;
                let avail = 0x10000usize.saturating_sub(start);
                let to_copy = body.len().min(avail);
                self.bus.ram[start..start + to_copy].copy_from_slice(&body[..to_copy]);

                let end_addr = (start + to_copy) as u16;
                self.cpu.x = (end_addr & 0xFF) as u8;
                self.cpu.y = (end_addr >> 8) as u8;
                self.cpu.p.set(Flags::C, false); // success

                eprintln!(
                    "  → loaded ${:04X}–${:04X} ({} bytes, {} blocks)",
                    dest, end_addr, to_copy, entry.blocks
                );

                // Update BASIC pointers if loaded at $0801
                if dest == 0x0801 {
                    let elo = (end_addr & 0xFF) as u8;
                    let ehi = (end_addr >> 8) as u8;
                    self.bus.ram[0x2B] = 0x01; self.bus.ram[0x2C] = 0x08;
                    self.bus.ram[0x2D] = elo;  self.bus.ram[0x2E] = ehi;
                    self.bus.ram[0x2F] = elo;  self.bus.ram[0x30] = ehi;
                    self.bus.ram[0x31] = elo;  self.bus.ram[0x32] = ehi;
                }

                Self::kernal_rts(&mut self.cpu, &self.bus.ram);
                true
            }
        }
    }

    /// Simulates RTS: pops the return address from the stack and updates PC.
    fn kernal_rts(cpu: &mut Cpu, ram: &[u8; 0x10000]) {
        let sp1 = cpu.sp.wrapping_add(1) as usize;
        let sp2 = cpu.sp.wrapping_add(2) as usize;
        let lo = ram[0x0100 + sp1];
        let hi = ram[0x0100 + sp2];
        cpu.sp = cpu.sp.wrapping_add(2);
        cpu.pc = u16::from_le_bytes([lo, hi]).wrapping_add(1);
    }

    // ── Save/Load state ────────────────────────────────────────────────────────

    /// Complete machine state in memory (see `crate::snapshot`):
    /// CPU with the current micro-cycle, RAM, VIC, SID, CIA, cartridge,
    /// mounted disk, framebuffer. ROMs are not included.
    pub fn snapshot(&self) -> Vec<u8> {
        let mut w = Writer::new();
        w.put(STATE_MAGIC);
        STATE_VERSION.save(&mut w);
        self.save(&mut w);
        w.put(STATE_END);
        w.buf
    }

    /// Restores a state produced by `snapshot`. On error the machine stays
    /// as it was: the state is rebuilt on a fresh C64, which takes the ROMs
    /// and debugger hooks of this one.
    pub fn restore(&mut self, data: &[u8]) -> Result<(), String> {
        let mut r = Reader::new(data);
        let magic = r.take(STATE_MAGIC.len()).map_err(|_| "not a snapshot file".to_string())?;
        if magic != STATE_MAGIC {
            return Err(if data.starts_with(b"C64S") && !data.starts_with(STATE_MAGIC) {
                "snapshot in the old format (incomplete), no longer supported".into()
            } else {
                "not a snapshot file".into()
            });
        }
        let mut version = 0u32;
        version.load(&mut r)?;
        if version != STATE_VERSION {
            return Err(format!("snapshot version {version}, this emulator reads version {STATE_VERSION}"));
        }
        let mut fresh = C64::new();
        fresh.load(&mut r)?;
        if r.take(STATE_END.len())? != STATE_END || !r.is_empty() {
            return Err("corrupt snapshot: end of data misaligned".into());
        }
        fresh.bus.kernal_rom = self.bus.kernal_rom;
        fresh.bus.basic_rom = self.bus.basic_rom;
        fresh.bus.char_rom = self.bus.char_rom;
        fresh.bus.dbg_watch = std::mem::take(&mut self.bus.dbg_watch);
        fresh.dbg = std::mem::take(&mut self.dbg);
        // Audio belongs to the frontend: it stays as it was, also for the
        // second SID of the state
        fresh.apply_clocks()?;
        let rate = self.bus.sid.audio_rate();
        fresh.bus.sid.inherit_audio(rate)?;
        if let Some(sid2) = &mut fresh.bus.sid2 {
            sid2.inherit_audio(rate)?;
        }
        *self = fresh;
        Ok(())
    }

    pub fn save_state(&self, path: &str) -> std::io::Result<()> {
        std::fs::write(path, self.snapshot())
    }

    pub fn load_state(&mut self, path: &str) -> std::io::Result<()> {
        let data = std::fs::read(path)?;
        self.restore(&data)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))
    }

    // ── Main loop ──────────────────────────────────────────────────────────────

    /// One VIC cycle (fetch, drawing 8 pixels, raster, sprites, bad line) and
    /// debugger hooks. Returns BA/AEC for the CPU access in this cycle.
    /// Interrupt sources do not touch the CPU here: the IRQ line is
    /// recomputed as a level in `step_instruction` from $D019 bit 7 and
    /// the CIA1 ICR.
    fn tick_vic(&mut self) -> crate::vic::Tick {
        let t = crate::vic::cycle(&mut self.bus, &mut self.work_fb, self.cpu.pc);
        if t.frame_done {
            match &mut self.blend {
                None => self.framebuffer.copy_from_slice(&self.work_fb),
                Some(prev) => {
                    blend_frames(&mut self.framebuffer, prev, &self.work_fb);
                    prev.copy_from_slice(&self.work_fb);
                }
            }
            self.frame_count += 1;
        }
        if let Some(target) = self.dbg.raster_break {
            if self.bus.vic.raster_line == target && self.bus.vic.line_start() {
                self.dbg.raster_hit = true;
            }
        }
        t
    }

    /// One clock cycle for VIC and CIA.
    fn tick_all(&mut self) -> crate::vic::Tick {
        self.bus.cycle += 1;
        let t = self.tick_vic();
        self.bus.cia1.tick(1);
        self.bus.cia2.tick(1);
        self.bus.sid.tick();
        if let Some(sid2) = &mut self.bus.sid2 {
            sid2.tick();
        }
        t
    }

    /// The drive advances to the end of the C64 cycle, after the CPU access:
    /// it sees this cycle's writes to $DD00, and the C64 reads in this cycle
    /// saw the drive up to the start of the cycle (as in VICE, which brings
    /// the drive up to the clock of the access).
    #[inline]
    fn tick_drive(&mut self) {
        if self.bus.drive.is_some() {
            let (atn, clk, data) = self.bus.iec_c64_lines();
            self.bus.drive.as_deref_mut().unwrap().run_c64_cycle(atn, clk, data);
        }
        // Datasette: like VICE alarms, after the CPU access
        if self.bus.tape.tick(self.bus.cycle) {
            self.bus.cia1.flag_edge();
        }
    }

    /// IRQ line as the CPU sees it in this cycle: VIC ($D019 bit 7),
    /// CIA1 (the CIA raises it one cycle after the flag, see `CiaState::ifr_cycle`)
    /// or REU.
    fn irq_line(&self) -> bool {
        self.bus.vic.regs[0x19] & 0x80 != 0
            || self.bus.cia1.irq_active()
            || self.bus.reu.as_ref().is_some_and(|r| r.irq())
    }

    /// REU cycle: true if the bus is the REU's in this cycle (the CPU is
    /// stalled). The REU is taken off the bus during the cycle, so its
    /// accesses go through the bus like the CPU's.
    fn reu_cycle(&mut self, ba_low: bool) -> DmaCycle {
        let cpu_writes = self.cpu.next_is_write();
        let mut reu = self.bus.reu.take().unwrap();
        let r = if reu.wants_bus(cpu_writes) { reu.dma_cycle(&mut self.bus, ba_low) } else { DmaCycle::Free };
        self.bus.reu = Some(reu);
        r
    }

    /// Attaches a REU of `kb` KB (128, 256, 512, ... 16384), or removes it
    /// with `None`. The RAM starts with its power-on contents.
    pub fn set_reu(&mut self, kb: Option<u32>) -> Result<(), String> {
        match kb {
            Some(kb) if !crate::reu::SIZES.contains(&kb) => {
                Err(format!("invalid REU size: {kb} KB (valid: {:?})", crate::reu::SIZES))
            }
            Some(kb) => {
                self.bus.reu = Some(crate::reu::Reu::new(kb));
                Ok(())
            }
            None => {
                self.bus.reu = None;
                Ok(())
            }
        }
    }

    /// Executes a single CPU instruction and advances VIC, CIA and IRQ logic
    /// by the corresponding cycles. When the frame cycle budget is exceeded
    /// it runs the end-of-frame housekeeping (PRG injection, D64 autoload, audio).
    pub fn step_instruction(&mut self) -> StepResult {
        // Intercept KERNAL LOAD before the CPU executes the instruction
        if self.cpu.pc == 0xFFD5 && self.disk.is_some() && self.bus.drive.is_none() && self.handle_kernal_load() {
            return StepResult { cycles: 0, elapsed: 0, frame_done: false };
        }
        if self.cpu.pc == 0xF81E && self.tape_auto_buttons {
            self.tape_prompt();
        }
        if self.t64.is_some() && matches!(self.cpu.pc, 0xF72F | 0xF8A1) && self.t64_trap() {
            return StepResult { cycles: 0, elapsed: 0, frame_done: false };
        }

        // One machine cycle at a time until the instruction (or the interrupt
        // sequence) is complete. In every cycle: VIC and CIA advance, the VIC
        // says whether the CPU may use the bus (BA/AEC for bad lines and sprite
        // DMA), the CPU performs its access, then samples the interrupt lines.
        let mut elapsed = 0u32;
        let mut cycles = 0u8;
        loop {
            let crate::vic::Tick { ba_low, aec_low, .. } = self.tick_all();
            // REU DMA: takes the bus at the first CPU read after the
            // command and keeps it until the end of the transfer
            let mut ba_low = ba_low;
            if self.bus.reu.is_some() {
                match self.reu_cycle(ba_low) {
                    DmaCycle::Busy => {
                        self.cpu.dma_stall();
                        self.tick_drive();
                        let irq = self.irq_line();
                        let nmi = self.bus.cia2.irq_active();
                        self.cpu.sample_lines(irq, nmi, true);
                        self.cpu.total_cycles += 1;
                        elapsed += 1;
                        continue;
                    }
                    DmaCycle::FreeIgnoreBa => ba_low = false,
                    DmaCycle::Free => {}
                }
            }
            let r = self.cpu.cycle(&mut self.bus, ba_low, aec_low);
            self.tick_drive();
            let irq = self.irq_line();
            let nmi = self.bus.cia2.irq_active();
            self.cpu.sample_lines(irq, nmi, r.stalled);
            self.cpu.total_cycles += 1;
            elapsed += 1;
            if !r.stalled { cycles = cycles.wrapping_add(1); }
            if r.done { break; }
        }

        self.frame_elapsed += elapsed;
        let frame_done = self.frame_elapsed >= self.standard().slice_cycles();
        if frame_done {
            self.frame_elapsed = 0;
            self.end_of_frame();
        }
        StepResult { cycles, elapsed, frame_done }
    }

    /// End-of-frame housekeeping: PRG injection, D64 autoload, audio samples.
    fn end_of_frame(&mut self) {
        // PRG injection after KERNAL boot
        if !self.injected && self.pending_prg.is_some() {
            match self.inject_countdown {
                None => {
                    if self.bus.ram[0x00CF] != 0 {
                        let wait = match self.prg_kind {
                            PrgKind::Basic   => 50,
                            PrgKind::Machine => 20,
                        };
                        self.inject_countdown = Some(wait);
                    }
                }
                Some(0) => {
                    if self.bus.ram[0x00C6] == 0 {
                        self.inject_prg();
                    }
                }
                Some(ref mut n) => { *n -= 1; }
            }
        }

        // D64 and tape auto-load: when the KERNAL is ready and the keyboard is
        // idle, type LOAD"*",8,1 (or LOAD and press PLAY) + RETURN. RUN then
        // goes into the KERNAL keyboard buffer ($0277, $C6), which is read
        // once loading is done: during a load the keyboard is not
        // scanned.
        if self.disk_autoload || self.tape_autoload {
            // Wait until the KERNAL has shown the prompt (0x00CF = CR received)
            if self.bus.ram[0x00CF] != 0 && self.bus.ram[0x00C6] == 0 {
                let cmd = if self.tape_autoload { "LOAD" } else { r#"LOAD"*",8,1"# };
                for c in cmd.chars() {
                    self.bus.keyboard.push_char(c);
                }
                self.bus.keyboard.push_char('\r');
                if self.tape_autoload {
                    self.bus.tape.play(self.bus.cycle);
                    self.tape_found_wait = 3000;
                }
                self.disk_autoload = false;
                self.tape_autoload = false;
                self.autoload_run = 10;
                crate::notice!("auto-load: injected {cmd} + RUN");
            }
        } else if self.autoload_run > 0 && !self.bus.keyboard.typing() {
            self.autoload_run -= 1;
            if self.autoload_run == 0 {
                let cmd = match std::mem::take(&mut self.autoload_sys) {
                    0 => "RUN\r".to_string(),
                    a => format!("SYS{a}\r"),
                };
                self.bus.ram[0x0277..0x0277 + cmd.len()].copy_from_slice(cmd.as_bytes());
                self.bus.ram[0x00C6] = cmd.len() as u8;
            }
        }

        self.tape_autoload_found();

        // Audio samples produced in this frame (≈882 at 44100 Hz)
        self.audio_buf.clear();
        self.bus.sid.take_samples(&mut self.audio_buf);
        if let Some(sid2) = &mut self.bus.sid2 {
            // Stereo: the two SIDs have the same sampling phase and yield
            // the same number of samples
            self.audio_buf2.clear();
            sid2.take_samples(&mut self.audio_buf2);
            let left = std::mem::take(&mut self.audio_buf);
            let n = left.len().max(self.audio_buf2.len());
            let at = |v: &[i16], i: usize| v.get(i).or(v.last()).copied().unwrap_or(0);
            self.audio_buf.reserve(2 * n);
            for i in 0..n {
                self.audio_buf.push(at(&left, i));
                self.audio_buf.push(at(&self.audio_buf2, i));
            }
        }
        // Datasette sound, the same on both channels (as in VICE)
        let channels = self.audio_channels();
        self.bus.tape.mix_sound(&mut self.audio_buf, channels, self.bus.cycle);
    }

    /// The KERNAL is about to ask for a Datasette key to be pressed
    /// ($F81E: prints the message in Y, then waits for SENSE): with a tape
    /// inserted it is pressed right away, PLAY for PRESS PLAY ON TAPE
    /// (Y = $1B), RECORD for PRESS RECORD & PLAY ON TAPE (Y = $2E).
    fn tape_prompt(&mut self) {
        use crate::tape::Button;
        let b = &self.bus;
        if (b.tape.image.is_none() && self.t64.is_none()) || b.tape.button() != Button::Stop
            || [b.peek(0xF81C), b.peek(0xF81E), b.peek(0xF81F), b.peek(0xF820)] != [0xA0, 0x20, 0x2F, 0xF1]
        {
            return;
        }
        let button = match self.cpu.y {
            0x1B => Button::Play,
            0x2E => Button::Record,
            _ => return,
        };
        self.bus.tape.press(button, self.bus.cycle);
        crate::notice!("Datasette: {} pressed", button.name());
    }

    /// Tape boot: after "FOUND name" the KERNAL pauses for about a dozen
    /// seconds with the motor off, unless C= is pressed. As a user loading
    /// would do, C= is pressed for a few frames as soon as FOUND appears.
    fn tape_autoload_found(&mut self) {
        const CBM_KEY: (usize, usize) = (7, 5);
        if self.tape_cbm_key > 0 {
            self.tape_cbm_key -= 1;
            if self.tape_cbm_key == 0 {
                self.bus.keyboard.release(CBM_KEY.0, CBM_KEY.1);
            }
        }
        if self.tape_found_wait == 0 {
            return;
        }
        self.tape_found_wait -= 1;
        // "FOUND" in screen codes, in the KERNAL screen page
        const FOUND: [u8; 5] = [6, 15, 21, 14, 4];
        let base = (self.bus.ram[0x0288] as usize) << 8;
        if self.bus.ram[base..base + 1000].windows(5).any(|w| w == FOUND) {
            self.bus.keyboard.press(CBM_KEY.0, CBM_KEY.1);
            self.tape_cbm_key = 3;
            self.tape_found_wait = 0;
        }
    }

    /// T64 tape traps (VICE's c64_tape_traps): with the KERNAL ROM visible,
    /// the JSR that reads a header block ($F72F) or the data ($F8A1) from
    /// the tape is replaced by reading the T64 image, and the KERNAL goes
    /// on from where the tape read would have returned. Returns true if
    /// the trap was handled.
    fn t64_trap(&mut self) -> bool {
        let pc = self.cpu.pc;
        let (check, resume) = if pc == 0xF72F { ([0x20, 0x41, 0xF8], 0xF732) } else { ([0x20, 0xBD, 0xFC], 0xFC93) };
        if (0..3).any(|i| self.bus.peek(pc + i) != check[i as usize]) {
            return false;
        }
        let Some(t64) = self.t64.as_mut() else { return false };
        if pc == 0xF72F {
            let stop = t64.find_header(&mut self.bus.ram);
            self.cpu.p.set(Flags::C, stop);
            self.cpu.p.set(Flags::Z, true);
        } else {
            t64.receive(&mut self.bus.ram, self.cpu.x);
            self.cpu.p.set(Flags::C, false);
            self.cpu.p.set(Flags::I, false);
        }
        self.cpu.pc = resume;
        true
    }

    /// Channels of `audio_buf`: 2 with the second SID.
    pub fn audio_channels(&self) -> usize {
        if self.bus.sid2.is_some() { 2 } else { 1 }
    }

    /// Audio on at rate `rate` or off, for all SIDs.
    pub fn set_audio(&mut self, rate: Option<u32>) -> Result<(), String> {
        self.bus.sid.set_audio(rate)?;
        if let Some(sid2) = &mut self.bus.sid2 {
            sid2.set_audio(rate)?;
        }
        Ok(())
    }

    /// SID model (the same for both, as in VICE).
    /// Model of both CIAs: the 6526 (default) or the 6526A of the C64C.
    pub fn set_cia_model(&mut self, model: crate::cia::Model) {
        self.bus.cia1.model = model;
        self.bus.cia2.model = model;
    }

    pub fn cia_model(&self) -> crate::cia::Model {
        self.bus.cia1.model
    }

    pub fn set_sid_model(&mut self, model: crate::sid::Model, digiboost: bool) {
        self.bus.sid.set_model(model, digiboost);
        if let Some(sid2) = &mut self.bus.sid2 {
            sid2.set_model(model, digiboost);
        }
    }

    /// Attaches the second SID at address `base` ($D420-$D7E0 or
    /// $DE00-$DFE0, in steps of $20), or removes it. It starts as after
    /// power-on, with the model and audio of the first one.
    pub fn set_sid2(&mut self, base: Option<u16>) -> Result<(), String> {
        let Some(base) = base else {
            self.bus.sid2 = None;
            return Ok(());
        };
        if !crate::sid::valid_sid2_address(base) {
            return Err(format!("invalid second SID address: ${base:04X} ($D420-$D7E0 or $DE00-$DFE0, in steps of $20)"));
        }
        let mut sid2 = Box::new(crate::sid::Sid::new());
        sid2.set_clock(self.standard().clock_hz())?;
        sid2.set_model(self.bus.sid.model(), self.bus.sid.digiboost());
        sid2.set_audio(self.bus.sid.audio_rate())?;
        sid2.sync_sampling(&self.bus.sid);
        self.bus.sid2 = Some(sid2);
        self.bus.sid2_base = base;
        Ok(())
    }

    /// Plugs `dev` into control port `port` (1 or 2).
    pub fn set_control_port(&mut self, port: usize, dev: crate::ctrlport::Device) {
        self.bus.ctrl.set_device(port - 1, dev, self.bus.cycle);
    }

    /// Host mouse movement in C64 pixels (Y positive downwards), for the
    /// paddles and the 1351 mouse in the control ports.
    pub fn mouse_move(&mut self, dx: i32, dy: i32) {
        // While someone types on the host, the 1351 in joystick mode stays
        // still: its lines share the keyboard rows (port 1) and a finger
        // brushing the trackpad would add phantom keys, as on a real C64
        let typing = self.bus.keyboard.host_typing(TYPING_GUARD_FRAMES);
        self.bus.ctrl.move_by(dx, dy, self.bus.cycle, typing);
    }

    /// Host mouse button, for the paddles and the 1351 mouse.
    pub fn mouse_button(&mut self, b: crate::ctrlport::Button, pressed: bool) {
        self.bus.ctrl.set_button(b, pressed, self.bus.cycle);
    }

    /// Host pointer at `(x, y)` in the framebuffer: sets the paddles from its
    /// position over the display area (see `ControlPorts::point_at`).
    pub fn point_paddles(&mut self, x: i32, y: i32) {
        self.bus.ctrl.point_at(x, y, self.bus.cycle);
    }

    /// Paddle readings (0-255) of control port `port` (1 or 2).
    pub fn set_paddles(&mut self, port: usize, x: u8, y: u8) {
        self.bus.ctrl.set_paddles(port - 1, x, y, self.bus.cycle);
    }

    /// Runs one PAL frame (≈19,705 cycles). Returns true if the VIC
    /// completed rendering at least one frame during the call.
    pub fn run_frame(&mut self) -> bool {
        let before = self.frame_count;
        loop {
            if self.step_instruction().frame_done {
                break;
            }
        }
        self.frame_count != before
    }

    /// Drive activity counter (0 without a drive), see `DriveBus::activity`.
    pub fn drive_activity(&self) -> u64 {
        self.bus.drive.as_ref().map_or(0, |d| d.bus.activity)
    }

    /// DOS job pending with the motor on, see `Drive::job_pending`.
    pub fn drive_job_pending(&self) -> bool {
        self.bus.drive.as_ref().is_some_and(|d| d.job_pending())
    }

    /// The tape is moving (a key pressed and the motor on).
    pub fn tape_moving(&self) -> bool {
        self.bus.tape.moving()
    }

    /// Tape load: the tape has been moving without stopping for at least
    /// a second. Many games turn the motor on for a few frames when they
    /// set $01 to $00 to read the RAM under the I/O: the tape moves in
    /// bursts but it is not a load; a load keeps the motor on for tens of
    /// seconds.
    pub fn tape_loading(&self) -> bool {
        let second = self.standard().clock_hz();
        self.bus.tape.moving_since().is_some_and(|t| self.bus.cycle - t >= second)
    }
}

/// Detects a load for the frontends' load turbo: active while the drive
/// reads or writes the disk, the serial bus is busy, the DOS has a job
/// queued or the tape has been moving for at least one second, and for
/// `LOAD_HOLD_FRAMES` frames afterwards. The drive motor alone is not enough: the DOS keeps it
/// spinning about 3 seconds after the load has finished.
pub struct LoadDetector {
    last: u64,
    quiet: u32,
}

/// Frames of drive silence before returning to real speed: covers the
/// short pauses of a load, such as track changes.
pub const LOAD_HOLD_FRAMES: u32 = 25;

impl LoadDetector {
    pub fn new() -> Self {
        Self { last: 0, quiet: LOAD_HOLD_FRAMES }
    }

    /// To be called every frame: true if a load is in progress. A PRG
    /// waiting to be injected counts too, without the hold: the boot runs
    /// at full speed and the program starts at real speed.
    pub fn update(&mut self, c64: &C64) -> bool {
        let a = c64.drive_activity();
        if a != self.last || c64.drive_job_pending() || c64.tape_loading() {
            self.last = a;
            self.quiet = 0;
        } else {
            self.quiet = self.quiet.saturating_add(1);
        }
        self.quiet < LOAD_HOLD_FRAMES || c64.prg_pending()
    }
}

// ── Saveable state ───────────────────────────────────────────────────────────

/// Frames after the last host key during which the 1351 in joystick mode
/// ignores the host mouse (half a second).
const TYPING_GUARD_FRAMES: u16 = 25;

/// Snapshot file header; the version changes on every layout change.
const STATE_MAGIC: &[u8] = b"C64SNAP\x1a";
const STATE_VERSION: u32 = 23;
const STATE_END: &[u8] = b"END.";

impl_state_enum!(PrgKind { Basic, Machine });

/// `out` = average of the ARGB frames `a` and `b` in linear light, per
/// channel (sRGB curve).
fn blend_frames(out: &mut [u32], a: &[u32], b: &[u32]) {
    use std::sync::OnceLock;
    // sRGB byte -> linear 0-65535; linear >> 4 (0-4095) -> sRGB byte
    static LUT: OnceLock<([u32; 256], Vec<u8>)> = OnceLock::new();
    let (to_lin, to_srgb) = LUT.get_or_init(|| {
        let lin = |c: f64| if c <= 0.04045 { c / 12.92 } else { ((c + 0.055) / 1.055).powf(2.4) };
        let srgb = |l: f64| if l <= 0.0031308 { l * 12.92 } else { 1.055 * l.powf(1.0 / 2.4) - 0.055 };
        let to_lin = std::array::from_fn(|i| (lin(i as f64 / 255.0) * 65535.0).round() as u32);
        let to_srgb = (0..4096).map(|i| (srgb((i as f64 + 0.5) / 4096.0) * 255.0).round() as u8).collect();
        (to_lin, to_srgb)
    });
    for ((o, &pa), &pb) in out.iter_mut().zip(a).zip(b) {
        if pa == pb {
            *o = pa;
            continue;
        }
        let mix = |shift: u32| {
            let l = (to_lin[(pa >> shift & 0xFF) as usize] + to_lin[(pb >> shift & 0xFF) as usize]) / 2;
            (to_srgb[(l >> 4) as usize] as u32) << shift
        };
        *o = 0xFF00_0000 | mix(16) | mix(8) | mix(0);
    }
}

// Excluded: the audio buffer (rewritten every frame), the debugger hooks,
// frame blending and the CRT emulation (display settings).
impl_state!(C64 {
    chip, cpu, bus, framebuffer, work_fb, pending_prg, prg_kind, injected, inject_countdown,
    disk, disk_path, disk_autoload, tape_path, t64, tape_autoload, tape_auto_buttons, tape_found_wait, tape_cbm_key,
    autoload_run, autoload_sys,
    frame_elapsed, frame_count,
} skip { audio_buf, audio_buf2, dbg, blend, hdr, crt });
