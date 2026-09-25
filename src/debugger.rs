// SPDX-License-Identifier: GPL-2.0-or-later
// Copyright (C) 2026 SampoSoft - Francesco Sampoli

//! Headless debugger: breakpoints, watchpoints, stepping, tracing, state dumps.
//!
//! Driven by text commands (see `help`), meant to be used from scripts and
//! pipes: every command produces deterministic, greppable output.
//! Number convention: addresses and values in hex (`$` optional), counters
//! (frames, instructions, cycles, raster lines) in decimal.

use crate::c64::{LoadDetector, C64, StepResult};
use crate::cart::Mode;
use crate::cpu::Flags;
use crate::disasm;
use crate::mem::{Watch, WatchHit};
use crate::vic::{HEIGHT, WIDTH};
use std::collections::BTreeSet;
use std::fmt::Write as FmtWrite;
use std::io::Write as IoWrite;
use std::time::Instant;

use crate::frontend::FRAME;

/// Hook for a frontend (the `c64dbg --window` window): receives every frame
/// completed while running, and the state after every command.
pub trait FrameHook {
    /// Called at the end of each frame while emulation runs; `realtime` is
    /// false at maximum speed (including during the load turbo). Returns
    /// true to stop execution (pause requested by the user).
    fn frame(&mut self, c64: &mut C64, realtime: bool) -> bool;
    /// Called after every command, with emulation stopped.
    fn idle(&mut self, c64: &mut C64);
    /// The frontend asks to quit (window closed).
    fn quit_requested(&self) -> bool { false }
}

/// Why execution stopped.
#[derive(Debug)]
pub enum Stop {
    Breakpoint(u16),
    Temp(u16),
    Watch(WatchHit),
    Raster(u16),
    Irq(u16),
    Nmi(u16),
    Kil(u16),
    Limit(String),
    /// Pause requested by the frontend (key in the window).
    User,
}

impl Stop {
    pub fn describe(&self) -> String {
        match self {
            Stop::Breakpoint(a) => format!("breakpoint ${a:04X}"),
            Stop::Temp(a)       => format!("reached ${a:04X}"),
            Stop::Watch(h) => if h.write {
                format!("watch write ${:04X} <- ${:02X} (PC ${:04X})", h.addr, h.val, h.pc)
            } else {
                format!("watch read ${:04X} = ${:02X} (PC ${:04X})", h.addr, h.val, h.pc)
            },
            Stop::Raster(l)     => format!("raster line {l}"),
            Stop::Irq(v)        => format!("IRQ taken -> ${v:04X}"),
            Stop::Nmi(v)        => format!("NMI taken -> ${v:04X}"),
            Stop::Kil(a)        => format!("KIL opcode at ${a:04X} (CPU jammed)"),
            Stop::Limit(s)      => format!("limit {s}"),
            Stop::User          => "paused from the window".to_string(),
        }
    }
    pub fn is_limit(&self) -> bool { matches!(self, Stop::Limit(_)) }
}

/// Execution limit for `run`.
#[derive(Clone, Copy)]
pub enum Limit {
    Instructions(u64),
    Frames(u64),
    Cycles(u64),
}

struct Trace {
    out: Box<dyn IoWrite>,
    from: u16,
    to: u16,
    lines: u64,
}

pub struct Debugger {
    pub breakpoints: BTreeSet<u16>,
    pub temp_break: Option<u16>,
    pub break_irq: bool,
    pub break_nmi: bool,
    pub break_kil: bool,
    trace: Option<Trace>,
    pub instr_count: u64,
    pub quit: bool,
    /// Run at 50 frames/s instead of at maximum speed.
    pub realtime: bool,
    /// Real speed, but maximum while loading from the drive (`speed auto`).
    pub load_turbo: bool,
    loads: LoadDetector,
    loading: bool,
    pace_next: Option<Instant>,
    pub hook: Option<Box<dyn FrameHook>>,
    /// Audio recording in progress (`audio file`).
    audio_rec: Option<AudioRec>,
    /// File for the raw output being captured (`audio raw file`).
    audio_raw: Option<String>,
    /// Media from the command line (tapes, disks) and the one inserted now,
    /// for `swap`.
    pub media: Vec<String>,
    media_pos: usize,
}

struct AudioRec {
    path: String,
    rate: u32,
    /// 1, or 2 with the second SID (channels chosen when recording starts).
    channels: usize,
    samples: Vec<i16>,
}

impl AudioRec {
    /// Appends one frame of audio, converting its channels.
    fn add(&mut self, buf: &[i16], channels: usize) {
        match (channels, self.channels) {
            (1, 2) => self.samples.extend(buf.iter().flat_map(|&s| [s, s])),
            (2, 1) => self.samples.extend(buf.chunks_exact(2).map(|f| crate::sid::audio_mix(f[0], f[1]))),
            _ => self.samples.extend_from_slice(buf),
        }
    }
}

/// 16-bit PCM WAV, mono or interleaved stereo.
fn write_wav(path: &str, rate: u32, channels: usize, samples: &[i16]) -> std::io::Result<()> {
    let channels = channels as u16;
    let data_len = (samples.len() * 2) as u32;
    let mut b = Vec::with_capacity(44 + data_len as usize);
    b.extend_from_slice(b"RIFF");
    b.extend_from_slice(&(36 + data_len).to_le_bytes());
    b.extend_from_slice(b"WAVEfmt ");
    b.extend_from_slice(&16u32.to_le_bytes());
    b.extend_from_slice(&1u16.to_le_bytes()); // PCM
    b.extend_from_slice(&channels.to_le_bytes());
    b.extend_from_slice(&rate.to_le_bytes());
    b.extend_from_slice(&(rate * 2 * channels as u32).to_le_bytes());
    b.extend_from_slice(&(2 * channels).to_le_bytes());
    b.extend_from_slice(&16u16.to_le_bytes());
    b.extend_from_slice(b"data");
    b.extend_from_slice(&data_len.to_le_bytes());
    for s in samples {
        b.extend_from_slice(&s.to_le_bytes());
    }
    std::fs::write(path, b)
}

impl Default for Debugger {
    fn default() -> Self { Self::new() }
}

// ── Formatting ────────────────────────────────────────────────────────────────

/// VICE-style flags: "NV-BDIZC", '.' for clear bits.
pub fn flags_str(p: Flags) -> String {
    let names = ['N', 'V', '-', 'B', 'D', 'I', 'Z', 'C'];
    let mut s = String::with_capacity(8);
    for (i, n) in names.iter().enumerate() {
        let bit = 0x80 >> i;
        if i == 2 { s.push('-'); }
        else if p.0 & bit != 0 { s.push(*n); }
        else { s.push('.'); }
    }
    s
}

/// Register line in VICE monitor style:
/// `.C:e5cd  A5 C6       LDA $C6        - A:00 X:00 Y:0A SP:f6 ..-..IZ.  3364019`
pub fn reg_line(c64: &C64) -> String {
    let pc = c64.cpu.pc;
    let ins = disasm::disasm(&|a| c64.bus.peek(a), pc);
    format!(
        ".C:{:04x}  {:<11} {:<15}- A:{:02X} X:{:02X} Y:{:02X} SP:{:02x} {}  {}",
        pc, ins.bytes_hex(), ins.text(),
        c64.cpu.a, c64.cpu.x, c64.cpu.y, c64.cpu.sp,
        flags_str(c64.cpu.p), c64.cpu.total_cycles
    )
}

fn parse_hex(s: &str) -> Option<u32> {
    let s = s.trim();
    if let Some(d) = s.strip_prefix('#') { return d.parse().ok(); }
    let h = s.strip_prefix('$').or_else(|| s.strip_prefix("0x")).unwrap_or(s);
    u32::from_str_radix(h, 16).ok()
}

pub(crate) fn parse_dec(s: &str) -> Option<u64> {
    let s = s.trim();
    if let Some(h) = s.strip_prefix('$') { return u64::from_str_radix(h, 16).ok(); }
    s.parse().ok()
}

pub(crate) fn parse_addr(s: &str) -> Result<u16, String> {
    parse_hex(s).filter(|v| *v <= 0xFFFF).map(|v| v as u16)
        .ok_or_else(|| format!("invalid address: {s}"))
}

fn parse_byte(s: &str) -> Result<u8, String> {
    parse_hex(s).filter(|v| *v <= 0xFF).map(|v| v as u8)
        .ok_or_else(|| format!("invalid byte: {s}"))
}

fn screencode_to_char(code: u8, lower: bool) -> char {
    let c = code & 0x7F; // ignore reverse
    match c {
        0 => '@',
        1..=26 => if lower { (b'a' + c - 1) as char } else { (b'A' + c - 1) as char },
        27 => '[', 28 => '\\', 29 => ']', 30 => '^', 31 => '<',
        32..=63 => c as char,
        64 => '-',
        65..=90 => if lower { (b'A' + c - 65) as char } else { '#' },
        _ => '#',
    }
}

fn petscii_printable(b: u8) -> char {
    match b {
        0x20..=0x5F => b as char,
        0x61..=0x7A => (b - 0x20) as char, // PETSCII lowercase → ASCII uppercase
        0xC1..=0xDA => (b - 0x80) as char,
        _ => '.',
    }
}

const JOY_BITS: &[(&str, u8)] = &[
    ("up", 0x01), ("down", 0x02), ("left", 0x04), ("right", 0x08), ("fire", 0x10),
];

// ── Debugger ──────────────────────────────────────────────────────────────────

impl Debugger {
    pub fn new() -> Self {
        Self {
            breakpoints: BTreeSet::new(),
            temp_break: None,
            break_irq: false,
            break_nmi: false,
            break_kil: true,
            trace: None,
            instr_count: 0,
            quit: false,
            realtime: false,
            load_turbo: false,
            loads: LoadDetector::new(),
            loading: false,
            pace_next: None,
            hook: None,
            audio_rec: None,
            audio_raw: None,
            media: Vec::new(),
            media_pos: 0,
        }
    }

    /// Executes one instruction, applying tracing and stop checks.
    /// Returns `Some(stop)` if execution must stop after this instruction.
    pub fn step_once(&mut self, c64: &mut C64) -> (StepResult, Option<Stop>) {
        let res = self.step_traced(c64);
        let mut user_stop = false;
        if res.frame_done {
            self.record_frame(c64);
            c64.bus.keyboard.advance_typing();
            self.loading = self.loads.update(c64);
            let paced = self.realtime && !(self.load_turbo && self.loading);
            if paced {
                self.pace();
            } else {
                self.pace_next = None;
            }
            if let Some(h) = self.hook.as_mut() {
                user_stop = h.frame(c64, paced);
            }
        }
        let stop = self.check_stop(c64).or(user_stop.then_some(Stop::User));
        (res, stop)
    }

    /// One instruction with the trace, and the watchpoint armed; the stop
    /// checks are `check_stop`. Frame housekeeping (typing, audio
    /// recording, pacing) is left to the caller: `step_once` here, the
    /// real-time session for the remote monitor.
    pub fn step_traced(&mut self, c64: &mut C64) -> StepResult {
        if let Some(t) = self.trace.as_mut() {
            let pc = c64.cpu.pc;
            // As in VICE: the interrupt sequence produces no line, the next
            // line is the first instruction of the ISR.
            if pc >= t.from && pc <= t.to && !c64.cpu.interrupt_imminent() {
                let _ = writeln!(t.out, "{}", reg_line(c64));
                t.lines += 1;
            }
        }
        c64.bus.dbg_watch_hit = None;
        let res = c64.step_instruction();
        self.instr_count += 1;
        res
    }

    /// Why execution must stop after the instruction just executed, if it
    /// must: watchpoint, raster line, interrupt, breakpoint, KIL.
    pub fn check_stop(&mut self, c64: &mut C64) -> Option<Stop> {
        if let Some(hit) = c64.bus.dbg_watch_hit.take() {
            return Some(Stop::Watch(hit));
        }
        if c64.dbg.raster_hit {
            c64.dbg.raster_hit = false;
            return Some(Stop::Raster(c64.bus.vic.raster_line));
        }
        let pc = c64.cpu.pc;
        match c64.cpu.last_interrupt {
            1 if self.break_irq => return Some(Stop::Irq(pc)),
            2 if self.break_nmi => return Some(Stop::Nmi(pc)),
            _ => {}
        }
        if self.temp_break == Some(pc) {
            self.temp_break = None;
            return Some(Stop::Temp(pc));
        }
        if self.breakpoints.contains(&pc) {
            return Some(Stop::Breakpoint(pc));
        }
        if self.break_kil && disasm::OPCODES[c64.bus.peek(pc) as usize].0 == "KIL" {
            return Some(Stop::Kil(pc));
        }
        None
    }

    /// Something to check after every instruction: breakpoints, watchpoints,
    /// raster and interrupt stops, trace. Without them a real-time frontend
    /// can run whole frames at full speed (KIL stops are not checked then).
    pub fn needs_checks(&self, c64: &C64) -> bool {
        !self.breakpoints.is_empty() || self.temp_break.is_some() || self.break_irq
            || self.break_nmi || self.trace.is_some() || !c64.bus.dbg_watch.is_empty()
            || c64.dbg.raster_break.is_some()
    }

    /// End of a frame: the audio being recorded (`audio file`) takes its
    /// samples.
    pub fn record_frame(&mut self, c64: &C64) {
        if let Some(rec) = self.audio_rec.as_mut() {
            rec.add(&c64.audio_buf, c64.audio_channels());
        }
    }

    /// Waits for the frame deadline at real speed. If far behind (execution
    /// just resumed after a pause) it restarts from now.
    fn pace(&mut self) {
        let now = Instant::now();
        let target = match self.pace_next {
            Some(t) if now < t + FRAME * 10 => t + FRAME,
            _ => now + FRAME,
        };
        if target > now {
            std::thread::sleep(target - now);
        }
        self.pace_next = Some(target);
    }

    /// Runs until a stop or the limit.
    pub fn run(&mut self, c64: &mut C64, limit: Limit) -> Stop {
        let (mut instr, mut frames, mut cycles) = (0u64, 0u64, 0u64);
        loop {
            let (res, stop) = self.step_once(c64);
            instr += 1;
            cycles += res.elapsed as u64;
            if res.frame_done { frames += 1; }
            if let Some(s) = stop { return s; }
            match limit {
                Limit::Instructions(n) if instr >= n => return Stop::Limit(format!("{n} instructions")),
                Limit::Frames(n) if frames >= n => return Stop::Limit(format!("{n} frames")),
                Limit::Cycles(n) if cycles >= n => return Stop::Limit(format!("{n} cycles")),
                _ => {}
            }
        }
    }

    // ── State dumps ───────────────────────────────────────────────────────────

    pub fn dump_mem(c64: &C64, from: u16, to: u16) -> String {
        let mut s = String::new();
        let mut a = from as u32;
        while a <= to as u32 {
            let _ = write!(s, ">C:{a:04x} ");
            let mut ascii = String::new();
            for i in 0..16u32 {
                let addr = a + i;
                if addr <= to as u32 {
                    let b = c64.bus.peek(addr as u16);
                    let _ = write!(s, " {b:02x}");
                    ascii.push(petscii_printable(b));
                } else {
                    s.push_str("   ");
                    ascii.push(' ');
                }
            }
            let _ = writeln!(s, "  {ascii}");
            a += 16;
        }
        s
    }

    pub fn disassemble(c64: &C64, from: u16, count: usize) -> String {
        let mut s = String::new();
        let mut pc = from;
        for _ in 0..count {
            let ins = disasm::disasm(&|a| c64.bus.peek(a), pc);
            let mark = if pc == c64.cpu.pc { '*' } else { ' ' };
            let _ = writeln!(s, "{mark}.C:{:04x}  {:<11} {}", pc, ins.bytes_hex(), ins.text());
            pc = pc.wrapping_add(ins.len());
        }
        s
    }

    pub fn regs(c64: &C64) -> String {
        let mut s = reg_line(c64);
        s.push('\n');
        let _ = writeln!(s,
            "  P=${:02X} raster={} cycle={} frame={} irq_pending={} nmi_pending={} $01=${:02X}",
            c64.cpu.p.0, c64.bus.vic.raster_line, c64.bus.vic.cycle, c64.frame_count,
            c64.cpu.irq_pending as u8, c64.cpu.nmi_pending as u8, c64.bus.cpu_port_data);
        s
    }

    pub fn stack(c64: &C64) -> String {
        let sp = c64.cpu.sp;
        let mut s = format!("SP=${sp:02x}  (stack $01{:02x}-$01ff, {} bytes in use)\n",
            sp.wrapping_add(1), 0xFFu8.wrapping_sub(sp));
        let mut a = 0x0100u16 + sp as u16 + 1;
        let mut items = Vec::new();
        while a <= 0x01FF {
            items.push(format!("{:02x}", c64.bus.peek(a)));
            if a == 0x01FF { break; }
            a += 1;
        }
        let _ = writeln!(s, "  {}", items.join(" "));
        // Possible return addresses (JSR pushes PC+2, hence +1 for the next instruction)
        let mut a = 0x0100u16 + sp as u16 + 1;
        let mut guesses = Vec::new();
        while a < 0x01FF {
            let lo = c64.bus.peek(a) as u16;
            let hi = c64.bus.peek(a + 1) as u16;
            let ret = ((hi << 8) | lo).wrapping_add(1);
            // Plausible if there is a JSR 3 bytes earlier
            let jsr_at = ret.wrapping_sub(3);
            if c64.bus.peek(jsr_at) == 0x20 {
                guesses.push(format!("${:04x}: ret ${ret:04X} (JSR ${:04X})", a, jsr_at));
            }
            a += 1;
        }
        if !guesses.is_empty() {
            let _ = writeln!(s, "  plausible return addresses:");
            for g in guesses { let _ = writeln!(s, "    {g}"); }
        }
        s
    }

    pub fn screen(c64: &C64) -> String {
        let regs = &c64.bus.vic.regs;
        let screen_base = (((regs[0x18] >> 4) & 0x0F) as u16) * 0x0400;
        let lower = (regs[0x18] >> 1) & 0x07 == 3;
        let mut s = format!("screen ${:04X} (VIC bank {}, {})\n",
            c64.bus.vic_bank() * 0x4000 + screen_base, c64.bus.vic_bank(),
            if lower { "lowercase" } else { "uppercase/graphics" });
        s.push_str("+----------------------------------------+\n");
        for row in 0..25u16 {
            s.push('|');
            for col in 0..40u16 {
                let code = c64.bus.vic_read(screen_base + row * 40 + col);
                s.push(screencode_to_char(code, lower));
            }
            s.push_str("|\n");
        }
        s.push_str("+----------------------------------------+\n");
        s
    }

    pub fn bank(c64: &C64) -> String {
        let b = &c64.bus;
        let mut s = String::new();
        let _ = writeln!(s, "$00=${:02X} $01=${:02X} eff=${:02X}  LORAM={} HIRAM={} CHAREN={}",
            b.cpu_port_dir, b.cpu_port_data, b.effective_port(),
            b.loram() as u8, b.hiram() as u8, b.charen() as u8);
        let mode = b.cart_mode();
        if b.ultimax() {
            let _ = writeln!(s, "  ULTIMAX: RAM $0000-$0FFF, ROML $8000-$9FFF, I/O $D000-$DFFF, ROMH $E000-$FFFF, rest open bus");
        } else {
            let _ = writeln!(s, "  $8000-$9FFF: {}",
                if matches!(mode, Mode::Game8K | Mode::Game16K) && b.loram() && b.hiram() { "cart ROML" }
                else { "RAM" });
            let _ = writeln!(s, "  $A000-$BFFF: {}",
                if mode == Mode::Game16K && b.hiram() { "cart ROMH" }
                else if b.loram() && b.hiram() { "BASIC ROM" } else { "RAM" });
            let _ = writeln!(s, "  $D000-$DFFF: {}",
                if b.io_active() { "I/O" }
                else if !b.charen() && (b.hiram() || (b.loram() && mode != Mode::Game16K)) { "CHAR ROM" }
                else { "RAM" });
            let _ = writeln!(s, "  $E000-$FFFF: {}",
                if b.hiram() { "KERNAL ROM" } else { "RAM" });
        }
        let vb = b.vic_bank();
        let regs = &b.vic.regs;
        let screen = vb * 0x4000 + (((regs[0x18] >> 4) & 0x0F) as u16) * 0x0400;
        let charset = vb * 0x4000 + (((regs[0x18] >> 1) & 0x07) as u16) * 0x0800;
        let bitmap = vb * 0x4000 + if regs[0x18] & 0x08 != 0 { 0x2000 } else { 0 };
        let _ = writeln!(s, "  VIC bank {} (${:04X}-${:04X})  screen ${:04X}  charset ${:04X}{}  bitmap ${:04X}",
            vb, vb * 0x4000, vb * 0x4000 + 0x3FFF, screen, charset,
            if (vb == 0 || vb == 2) && (charset & 0x3FFF) >= 0x1000 && (charset & 0x3FFF) < 0x2000 { " (ROM)" } else { "" },
            bitmap);
        match &b.cart {
            None => { let _ = writeln!(s, "  cart: none"); }
            Some(c) => {
                let _ = writeln!(s, "  cart: \"{}\" type {} ({}), mode {}, bank ROML {} ROMH {}, registers {:02X?}",
                    c.name, c.kind.crt_type(), c.kind.name(), c.mode.name(), c.roml_bank, c.romh_bank, c.regs);
            }
        }
        s
    }

    pub fn vic(c64: &C64) -> String {
        let v = &c64.bus.vic;
        let r = &v.regs;
        let mut s = String::new();
        let _ = writeln!(s, "raster line {} cycle {}  IRQ target line {}  frame {}",
            v.raster_line, v.cycle, v.raster_target(), c64.frame_count);
        let _ = writeln!(s, "$D011=${:02X}  RST8={} ECM={} BMM={} DEN={} RSEL={} YSCROLL={}",
            r[0x11], (r[0x11] >> 7) & 1, (r[0x11] >> 6) & 1, (r[0x11] >> 5) & 1,
            (r[0x11] >> 4) & 1, (r[0x11] >> 3) & 1, r[0x11] & 7);
        let _ = writeln!(s, "$D016=${:02X}  MCM={} CSEL={} XSCROLL={}",
            r[0x16], (r[0x16] >> 4) & 1, (r[0x16] >> 3) & 1, r[0x16] & 7);
        let _ = writeln!(s, "$D018=${:02X}  VM=${:04X} CB=${:04X}",
            r[0x18], ((r[0x18] >> 4) as u16 & 0x0F) * 0x400, ((r[0x18] >> 1) as u16 & 7) * 0x800);
        let _ = writeln!(s, "$D019=${:02X} (flags: any={} lp={} ss={} sb={} raster={})  $D01A=${:02X} (enable: lp={} ss={} sb={} raster={})",
            r[0x19], (r[0x19] >> 7) & 1, (r[0x19] >> 3) & 1, (r[0x19] >> 2) & 1, (r[0x19] >> 1) & 1, r[0x19] & 1,
            r[0x1A], (r[0x1A] >> 3) & 1, (r[0x1A] >> 2) & 1, (r[0x1A] >> 1) & 1, r[0x1A] & 1);
        let mode = match ((r[0x11] >> 5) & 3, (r[0x16] >> 4) & 1) {
            (0, 0) => "text standard", (0, 1) => "text multicolor",
            (1, 0) => "bitmap standard", (1, 1) => "bitmap multicolor",
            (2, 0) => "text ECM", _ => "invalid",
        };
        let _ = writeln!(s, "mode: {mode}  display={} bad_line={} rc={} vc={} vcbase={} vborder={} border={}",
            v.display_state() as u8, v.bad_line as u8, v.rc, v.vc, v.vcbase, v.vborder as u8,
            v.main_border as u8);
        let _ = writeln!(s, "line buffer (c-access): {}",
            v.vbuf.iter().map(|c| format!("{c:02x}")).collect::<Vec<_>>().join(" "));
        let _ = writeln!(s, "border ${:02X}={}  bg0 ${:02X}={} bg1 ${:02X}={} bg2 ${:02X}={} bg3 ${:02X}={}",
            r[0x20], r[0x20] & 15, r[0x21], r[0x21] & 15, r[0x22], r[0x22] & 15,
            r[0x23], r[0x23] & 15, r[0x24], r[0x24] & 15);
        let _ = writeln!(s, "sprites: enable=${:02X} mc=${:02X} xexp=${:02X} yexp=${:02X} behind=${:02X} mc0={} mc1={}",
            r[0x15], r[0x1C], r[0x1D], r[0x17], r[0x1B], r[0x25] & 15, r[0x26] & 15);
        for row in 0..3 {
            let _ = write!(s, "regs {:02X}:", row * 16);
            for i in 0..16 {
                let idx = row * 16 + i;
                if idx < 0x2F { let _ = write!(s, " {:02x}", r[idx]); }
            }
            s.push('\n');
        }
        s
    }

    pub fn sprites(c64: &C64) -> String {
        let r = &c64.bus.vic.regs;
        let vb = c64.bus.vic_bank();
        let screen_base = (((r[0x18] >> 4) & 0x0F) as u16) * 0x0400;
        let mut s = String::from("#  en  x    y    ptr  addr   col mc xexp yexp behind  dma disp row\n");
        for i in 0..8usize {
            let x = r[i * 2] as u16 | (((r[0x10] >> i) as u16 & 1) << 8);
            let y = r[i * 2 + 1];
            let ptr = c64.bus.vic_read(screen_base + 0x3F8 + i as u16);
            let bit = |reg: usize| (r[reg] >> i) & 1;
            let v = &c64.bus.vic;
            let _ = writeln!(s, "{i}  {}   {:<4} {:<4} ${ptr:02X}  ${:04X}  {:<2}  {}  {}    {}    {}       {}   {}    {}",
                bit(0x15), x, y, vb * 0x4000 + ptr as u16 * 64,
                r[0x27 + i] & 15, bit(0x1C), bit(0x1D), bit(0x17), bit(0x1B),
                (v.sprite_dma >> i) & 1, (v.sprite_display_bits >> i) & 1, v.sprites[i].mcbase / 3);
        }
        s
    }

    fn cia_common(name: &str, base: u16, c: &crate::cia::CiaState, pa: u8, pb: u8) -> String {
        let mut s = String::new();
        let _ = writeln!(s, "{name} ${base:04X}: PA=${pa:02X} DDRA=${:02X} PB=${pb:02X} DDRB=${:02X}",
            c.regs[2], c.regs[3]);
        let cra = c.regs[0x0E];
        let crb = c.regs[0x0F];
        let _ = writeln!(s, "  timer A: ${:04X} latch ${:04X}  CRA=${cra:02X} start={} oneshot={} pbon={} inmode={}",
            c.ta.cnt, c.ta.latch, c.ta.running() as u8, (cra >> 3) & 1, (cra >> 1) & 1,
            if cra & 0x20 != 0 { "CNT" } else { "phi2" });
        let _ = writeln!(s, "  timer B: ${:04X} latch ${:04X}  CRB=${crb:02X} start={} oneshot={} pbon={} inmode={}",
            c.tb.cnt, c.tb.latch, c.tb.running() as u8, (crb >> 3) & 1, (crb >> 1) & 1,
            match (crb >> 5) & 3 { 0 => "phi2", 1 => "CNT", 2 => "timerA", _ => "timerA+CNT" });
        let _ = writeln!(s, "  ICR flags=${:02X} mask=${:02X} irq_active={}  (bit0=TA bit1=TB bit2=TOD bit3=SDR bit4=FLAG)",
            c.icr_flags as u8, c.icr_mask, c.irq_active() as u8);
        let _ = writeln!(s, "  TOD {:02X}:{:02X}:{:02X}.{:X} {}{}  alarm {:02X}:{:02X}:{:02X}.{:X}  SDR=${:02X} SP={}",
            c.tod[3] & 0x1F, c.tod[2], c.tod[1], c.tod[0],
            if c.tod[3] & 0x80 != 0 { "PM" } else { "AM" },
            if c.tod_stopped { " (stopped)" } else { "" },
            c.tod_alarm[3] & 0x1F, c.tod_alarm[2], c.tod_alarm[1], c.tod_alarm[0],
            c.sdr, if c.regs[0x0E] & 0x40 != 0 { "out" } else { "in" });
        let _ = write!(s, "  regs:");
        for i in 0..16 { let _ = write!(s, " {:02x}", c.regs[i]); }
        s.push('\n');
        s
    }

    pub fn cia1(c64: &C64) -> String {
        let mut s = Self::cia_common("CIA1", 0xDC00, &c64.bus.cia1, c64.bus.cia1_port_a(), c64.bus.cia1_port_b());
        let _ = writeln!(s, "  joy1=${:02X} joy2=${:02X} keyboard matrix cols: {}",
            c64.bus.joy1, c64.bus.joy2,
            c64.bus.keyboard.matrix.iter().map(|m| format!("{m:02x}")).collect::<Vec<_>>().join(" "));
        s
    }

    pub fn cia2(c64: &C64) -> String {
        let c = &c64.bus.cia2;
        let mut s = Self::cia_common("CIA2", 0xDD00, c, c.regs[0], c.regs[1]);
        let _ = writeln!(s, "  VIC bank {} (PA bit0-1)", c64.bus.vic_bank());
        s
    }

    /// 1541 drive: its CPU's registers and instructions, mechanics, VIAs,
    /// serial bus lines; with `mem a [b]`, its memory.
    pub fn drive(c64: &C64, args: &[&str]) -> Result<String, String> {
        let Some(d) = c64.bus.drive.as_ref() else { return Ok("  1541 drive: not emulated\n".into()) };
        let mut s = String::new();
        if args.first() == Some(&"trace") {
            // The drive must be modified: handled in exec_inner()
            return Err("usage: drive trace on file | off".into());
        }
        if args.first() == Some(&"g64") {
            let path = args.get(1).ok_or("usage: drive g64 file")?;
            let disk = d.bus.mech.disk.as_ref().ok_or("no disk in the drive")?;
            std::fs::write(path, disk.to_g64()).map_err(|e| format!("{path}: {e}"))?;
            return Ok(format!("disk saved to {path}\n"));
        }
        if args.first() == Some(&"mem") {
            let a = parse_addr(args.get(1).ok_or("usage: drive mem a [b]")?)?;
            let b = args.get(2).map(|x| parse_addr(x)).transpose()?.unwrap_or(a.wrapping_add(0x7F));
            let mut addr = a;
            while addr <= b {
                let _ = write!(s, ">8:{addr:04x} ");
                for i in 0..16u16 {
                    let _ = write!(s, " {:02x}", d.bus.peek(addr.wrapping_add(i)));
                }
                s.push('\n');
                if addr.checked_add(16).is_none() { break; }
                addr += 16;
            }
            return Ok(s);
        }
        let c = &d.cpu;
        let _ = writeln!(s, "  8: PC ${:04X} A ${:02X} X ${:02X} Y ${:02X} SP ${:02X} P ${:02X}",
            c.pc, c.a, c.x, c.y, c.sp, c.p.0);
        let mut pc = c.pc;
        for _ in 0..4 {
            let ins = disasm::disasm(&|a| d.bus.peek(a), pc);
            let _ = writeln!(s, "  .8:{:04x}  {:<11} {}", pc, ins.bytes_hex(), ins.text());
            pc = pc.wrapping_add(ins.len());
        }
        s += &d.describe();
        let (atn, clk, data) = c64.bus.iec_c64_lines();
        let (dclk, ddata) = d.pulls(atn);
        let _ = writeln!(s, "  serial bus: ATN {}  CLK {}  DATA {}  (C64 pulls low{}{}{}; drive{}{})",
            if atn { "low" } else { "high" }, if clk || dclk { "low" } else { "high" },
            if data || ddata { "low" } else { "high" },
            if atn { " ATN" } else { "" }, if clk { " CLK" } else { "" }, if data { " DATA" } else { "" },
            if dclk { " CLK" } else { "" }, if ddata { " DATA" } else { "" });
        Ok(s)
    }

    pub fn reu(c64: &C64) -> String {
        let Some(r) = c64.bus.reu.as_ref() else { return "  REU: none ('reu 512' to attach one)\n".into() };
        let p = |o: u8| r.peek(o);
        format!(
            "  REU {} KB  status ${:02X}  command ${:02X}  C64 ${:02X}{:02X}  REU ${:02X}{:02X}{:02X}  length ${:02X}{:02X}  IRQ ${:02X}  addrctl ${:02X}\n",
            r.size_kb(), p(0), p(1), p(3), p(2), p(6), p(5), p(4), p(8), p(7), p(9), p(10),
        )
    }

    pub fn sid(c64: &C64) -> String {
        Self::sid_chip(&c64.bus.sid)
    }

    /// Second SID: address and state, or how to attach one.
    pub fn sid2(c64: &C64) -> String {
        match &c64.bus.sid2 {
            Some(sid) => format!("second SID at ${:04X}\n{}", c64.bus.sid2_base, Self::sid_chip(sid)),
            None => "second SID: none ('sid2 d420' to attach one)\n".into(),
        }
    }

    fn sid_chip(sid: &crate::sid::Sid) -> String {
        let r = &sid.registers;
        let mut s = String::new();
        for v in 0..3usize {
            let b = v * 7;
            let freq = r[b] as u16 | ((r[b + 1] as u16) << 8);
            let pw = (r[b + 2] as u16 | ((r[b + 3] as u16) << 8)) & 0x0FFF;
            let ctrl = r[b + 4];
            let hz = freq as f64 * 985248.0 / 16777216.0;
            let mut wave = Vec::new();
            if ctrl & 0x10 != 0 { wave.push("tri"); }
            if ctrl & 0x20 != 0 { wave.push("saw"); }
            if ctrl & 0x40 != 0 { wave.push("pulse"); }
            if ctrl & 0x80 != 0 { wave.push("noise"); }
            let _ = writeln!(s, "voice {}: freq ${freq:04X} ({hz:.1} Hz) pw ${pw:03X} ctrl ${ctrl:02X} [gate={} sync={} ring={} test={} {}] A={} D={} S={} R={}",
                v + 1, ctrl & 1, (ctrl >> 1) & 1, (ctrl >> 2) & 1, (ctrl >> 3) & 1, wave.join("+"),
                r[b + 5] >> 4, r[b + 5] & 15, r[b + 6] >> 4, r[b + 6] & 15);
        }
        let cutoff = (r[0x15] as u16 & 7) | ((r[0x16] as u16) << 3);
        let _ = writeln!(s, "filter: cutoff ${cutoff:03X} res {} voices=${:02X} mode=${:02X} (lp={} bp={} hp={} 3off={}) volume {}",
            r[0x17] >> 4, r[0x17] & 7, r[0x18] >> 4, (r[0x18] >> 4) & 1, (r[0x18] >> 5) & 1,
            (r[0x18] >> 6) & 1, (r[0x18] >> 7) & 1, r[0x18] & 15);
        let _ = write!(s, "regs:");
        for i in 0..0x19 { let _ = write!(s, " {:02x}", r[i]); }
        let (osc3, env3) = sid.osc3_env3();
        let model = if sid.digiboost() { "8580 (digiboost)" } else { sid.model().name() };
        let audio = sid.audio_rate().map_or("off".to_string(), |r| format!("{r} Hz"));
        let _ = writeln!(s, "\nmodel {model}  OSC3 ${osc3:02X}  ENV3 ${env3:02X}  bus ${:02X}  audio {audio}", sid.read(0));
        s
    }

    /// Datasette: button, motor, counter, position on the tape.
    pub fn tape(c64: &C64) -> String {
        let t = &c64.bus.tape;
        let (amp, freq) = t.wobble();
        let wobble = if amp == 0 || freq == 0 {
            "off".to_string()
        } else {
            format!("±{}% at {} Hz", amp as f64 / 1000.0, freq as f64 / 1000.0)
        };
        let auto = if c64.tape_auto_buttons { "automatic" } else { "manual" };
        let azimuth = match t.azimuth_error() {
            0 => "off".to_string(),
            e => format!("±{} cycles", e as f64 / 1000.0),
        };
        let sound = t.sound().map_or("off".to_string(), |v| format!("volume {v}"));
        let extra = format!("buttons {auto}  wobble {wobble}  azimuth {azimuth}  sound {sound}");
        let Some(tap) = &t.image else {
            return format!("tape: none ('tape insert file.tap')  {extra}\n");
        };
        let (pos, len) = t.position();
        let file = match c64.tape_path() {
            "" => String::new(),
            p => format!("  file {p}{}", if tap.changed { " (unsaved recordings)" } else { "" }),
        };
        format!("tape: {}  motor {}  counter {:03}  position {pos}/{len} ({:.0}%)  pulses read {}{file}\n{extra}\n",
            t.button().name(), if t.motor() { "on" } else { "off" }, t.counter(),
            100.0 * pos as f64 / len.max(1) as f64, t.pulses)
    }

    pub fn help() -> &'static str {
        "Commands (addresses/values in hex, '$' optional; counters in decimal):
  run [n]            run until a stop (max n frames, default 50000)
  frames n           run n frames
  step|s [n]         execute n instructions (default 1), print the registers
  cycles n           run at least n cycles
  next|n             step over: skips JSRs
  until addr         run until PC=addr
  break|bp addr      breakpoint     delete addr|all     breaks
  watch a [b]        r/w watchpoint on [a,b]   watchr / watchw   unwatch a|all   watches
  rbreak line|off    stop at cycle 0 of the raster line
  irqbreak on|off    stop on entry to every IRQ     nmibreak on|off
  kilbreak on|off    stop on a KIL opcode (default on)
  regs|r             registers, raster, frame     set a|x|y|sp|pc|p val
  mem|m a [b]        hex dump (default 128 bytes, no side effects)
  read a [a..]       read via the bus, with side effects (ICR, $D01E/$D01F)
  poke a v [v..]     write via the bus   fill a b v   find a b v [v..]
  dis|d [addr] [n]   disassemble n instructions (default PC, 16)
  stack              stack and plausible return addresses
  screen             screen RAM as text
  bank | vic | sprites | cia1 | cia2
  sid [6581|8580|8580d]  SID registers and state; a model switches every SID to it (8580d: with digiboost)
  sid2 [addr|off]    second SID: state; an address (d420-d7e0, de00-dfe0) attaches it, off removes it
  audio file.wav [Hz] | audio off   record the SID output from the next frames (default 44100 Hz; stereo with the second SID)
  audio raw file     capture the first SID's filter output every cycle, 16-bit LE (like VICE's -residrawoutput)
  reu [KB|off]       REU registers; KB (128..16384) attaches it, off removes it
  drive [mem a [b]]  1541 drive: CPU, track, motor, VIA, serial bus; mem = its memory
  drive g64 file     save the disk in the drive as a G64 image
  tape [play|record|stop|ff|rew]   Datasette: state and buttons
  tape insert file|eject|save      .tap tape (missing file: blank tape); save writes the recordings
  tape rewind|counter              rewind instantly / reset the counter
  tape auto on|off                 buttons pressed automatically on PRESS PLAY / RECORD & PLAY ON TAPE
  tape wobble [off | % Hz]         tape speed wobble (default ±0.5% at 3 Hz, as in VICE)
  tape azimuth [cycles | off]      azimuth error: each pulse is off by a random amount up to that many cycles (0.001-10)
  tape sound [on [vol] | off]      tape sound during PLAY (volume 1-4096, default 1024)
  swap               next medium among the command-line files (tape with PLAY, disk)
  drive insert file  insert a D64/G64 without autoloading it
  drive trace on file | off   trace of the drive's instructions (PC, registers, cycle)
  screenshot file [bar]  save the framebuffer as PNG (bar: with the window's status bar; file - = the PNG on the output)
  trace on [file] [from to] | trace off
  keys text          type text on the keyboard (\\n = RETURN, {name} = a key named as for key: {f1} {clr} {left}...)
  key name down|up|press   (return space runstop f1..f8 home clr del inst left right up down lshift rshift ctrl commodore restore, or a character)
  joy1|joy2 [+|-]up|down|left|right|fire ... | none
  port [1|2 joystick|paddles|mouse|joymouse]   control ports: devices, paddles, mouse, SID POT registers; plugs a device
  mouse dx dy        move the host mouse by dx, dy C64 pixels (y down): 1351 mouse (both modes) and paddles
  mouse left|right down|up|press   mouse button (1351: left = fire, right = up, in joystick mode POTX; paddles: fire of X / Y)
  paddle 1|2 x y     paddle readings of a port (0-255)
  load file          load .prg/.d64/.g64/.tap/.t64/.crt      savestate file | loadstate file   reset
  speed [auto|real|max]  auto: real speed, maximum while loading from disk and tape (default with --window); real: always 50 frames/s; max: maximum
  info | echo text | help | quit"
    }

    // ── Command execution ─────────────────────────────────────────────────────

    /// Executes one command line, writing the output to `out`; an error is
    /// written as `ERROR: ...`.
    pub fn exec(&mut self, c64: &mut C64, line: &str, out: &mut dyn IoWrite) -> std::io::Result<()> {
        match self.exec_result(c64, line, out) {
            Ok(()) => Ok(()),
            Err(e) => writeln!(out, "ERROR: {e}"),
        }
    }

    /// Executes one command line, writing the output to `out` and returning
    /// the error, if any, instead of writing it (the remote monitor sends it
    /// back as such).
    pub fn exec_result(&mut self, c64: &mut C64, line: &str, out: &mut dyn IoWrite) -> Result<(), String> {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') { return Ok(()); }
        let (cmd, rest) = match line.split_once(char::is_whitespace) {
            Some((c, r)) => (c, r.trim()),
            None => (line, ""),
        };
        let args: Vec<&str> = rest.split_whitespace().collect();
        let r = self.exec_inner(c64, &cmd.to_ascii_lowercase(), rest, &args, out);
        // The file trace is buffered: flushed after every command, so that
        // quitting from the prompt loses no lines
        if let Some(t) = self.trace.as_mut() {
            let _ = t.out.flush();
        }
        if let Some(h) = self.hook.as_mut() {
            h.idle(c64);
            if h.quit_requested() {
                self.quit = true;
            }
        }
        r
    }

    fn report_stop(&self, c64: &C64, stop: &Stop, out: &mut dyn IoWrite) -> std::io::Result<()> {
        writeln!(out, "STOP: {}", stop.describe())?;
        writeln!(out, "{}", reg_line(c64))
    }

    fn exec_inner(&mut self, c64: &mut C64, cmd: &str, rest: &str, args: &[&str], out: &mut dyn IoWrite)
        -> Result<(), String>
    {
        let io = |e: std::io::Error| e.to_string();
        match cmd {
            "help" | "h" | "?" => writeln!(out, "{}", Self::help()).map_err(io)?,
            "quit" | "q" | "exit" => self.quit = true,
            "echo" => writeln!(out, "{rest}").map_err(io)?,
            "speed" => {
                match args.first().map(|a| a.to_ascii_lowercase()).as_deref() {
                    Some("auto") => (self.realtime, self.load_turbo) = (true, true),
                    Some("real") => (self.realtime, self.load_turbo) = (true, false),
                    Some("max") => self.realtime = false,
                    None => {}
                    _ => return Err("usage: speed [auto|real|max]".into()),
                }
                let mode = match (self.realtime, self.load_turbo) {
                    (false, _) => "max",
                    (true, false) => "real (50 frames/s)",
                    (true, true) => "auto (50 frames/s, maximum while loading from the drive)",
                };
                writeln!(out, "speed {mode}").map_err(io)?;
            }

            "run" | "frames" | "go" => {
                let n = args.first().map(|a| parse_dec(a).ok_or("invalid frame count")).transpose()?
                    .unwrap_or(50_000);
                let stop = self.run(c64, Limit::Frames(n));
                self.report_stop(c64, &stop, out).map_err(io)?;
            }
            "step" | "s" => {
                let n = args.first().map(|a| parse_dec(a).ok_or("invalid instruction count")).transpose()?
                    .unwrap_or(1);
                let verbose = n <= 64;
                for i in 0..n {
                    let (_, stop) = self.step_once(c64);
                    if verbose || i + 1 == n { writeln!(out, "{}", reg_line(c64)).map_err(io)?; }
                    if let Some(s) = stop {
                        writeln!(out, "STOP: {}", s.describe()).map_err(io)?;
                        break;
                    }
                }
            }
            "cycles" => {
                let n = args.first().and_then(|a| parse_dec(a)).ok_or("usage: cycles n")?;
                let stop = self.run(c64, Limit::Cycles(n));
                self.report_stop(c64, &stop, out).map_err(io)?;
            }
            "next" | "n" => {
                let pc = c64.cpu.pc;
                if c64.bus.peek(pc) == 0x20 {
                    self.temp_break = Some(pc.wrapping_add(3));
                    let stop = self.run(c64, Limit::Instructions(50_000_000));
                    self.report_stop(c64, &stop, out).map_err(io)?;
                } else {
                    let (_, stop) = self.step_once(c64);
                    writeln!(out, "{}", reg_line(c64)).map_err(io)?;
                    if let Some(s) = stop { writeln!(out, "STOP: {}", s.describe()).map_err(io)?; }
                }
            }
            "until" => {
                let a = parse_addr(args.first().ok_or("usage: until addr")?)?;
                self.temp_break = Some(a);
                let stop = self.run(c64, Limit::Instructions(50_000_000));
                self.report_stop(c64, &stop, out).map_err(io)?;
            }

            "break" | "bp" | "b" => {
                let a = parse_addr(args.first().ok_or("usage: break addr")?)?;
                self.breakpoints.insert(a);
                writeln!(out, "breakpoint ${a:04X}").map_err(io)?;
            }
            "delete" | "del" => {
                match args.first() {
                    Some(&"all") => { self.breakpoints.clear(); }
                    Some(a) => { let a = parse_addr(a)?; self.breakpoints.remove(&a); }
                    None => return Err("usage: delete addr|all".into()),
                }
                writeln!(out, "ok").map_err(io)?;
            }
            "breaks" => {
                let list: Vec<String> = self.breakpoints.iter().map(|a| format!("${a:04X}")).collect();
                writeln!(out, "breakpoints: {}", if list.is_empty() { "none".into() } else { list.join(" ") }).map_err(io)?;
            }
            "watch" | "watchr" | "watchw" => {
                let a = parse_addr(args.first().ok_or("usage: watch addr [end]")?)?;
                let b = args.get(1).map(|s| parse_addr(s)).transpose()?.unwrap_or(a);
                c64.bus.dbg_watch.push(Watch {
                    start: a.min(b), end: a.max(b),
                    on_read: cmd != "watchw", on_write: cmd != "watchr",
                });
                writeln!(out, "watch ${:04X}-${:04X} {}", a.min(b), a.max(b),
                    match cmd { "watchr" => "read", "watchw" => "write", _ => "read/write" }).map_err(io)?;
            }
            "unwatch" => {
                match args.first() {
                    Some(&"all") => c64.bus.dbg_watch.clear(),
                    Some(a) => { let a = parse_addr(a)?; c64.bus.dbg_watch.retain(|w| w.start != a); }
                    None => return Err("usage: unwatch addr|all".into()),
                }
                writeln!(out, "ok").map_err(io)?;
            }
            "watches" => {
                if c64.bus.dbg_watch.is_empty() { writeln!(out, "watches: none").map_err(io)?; }
                for w in &c64.bus.dbg_watch {
                    writeln!(out, "watch ${:04X}-${:04X} r={} w={}", w.start, w.end, w.on_read as u8, w.on_write as u8).map_err(io)?;
                }
            }
            "rbreak" => {
                match args.first() {
                    Some(&"off") => { c64.dbg.raster_break = None; }
                    Some(l) => {
                        let l = parse_dec(l).filter(|v| *v < 312).ok_or("raster line 0-311")?;
                        c64.dbg.raster_break = Some(l as u16);
                    }
                    None => return Err("usage: rbreak line|off".into()),
                }
                c64.dbg.raster_hit = false;
                writeln!(out, "ok").map_err(io)?;
            }
            "irqbreak" | "nmibreak" | "kilbreak" => {
                let on = match args.first() { Some(&"on") => true, Some(&"off") => false, _ => return Err(format!("usage: {cmd} on|off")) };
                match cmd { "irqbreak" => self.break_irq = on, "nmibreak" => self.break_nmi = on, _ => self.break_kil = on }
                writeln!(out, "ok").map_err(io)?;
            }

            "regs" | "r" => write!(out, "{}", Self::regs(c64)).map_err(io)?,
            "set" => {
                let (reg, val) = (args.first().ok_or("usage: set reg val")?, args.get(1).ok_or("usage: set reg val")?);
                match reg.to_ascii_lowercase().as_str() {
                    "a"  => c64.cpu.a = parse_byte(val)?,
                    "x"  => c64.cpu.x = parse_byte(val)?,
                    "y"  => c64.cpu.y = parse_byte(val)?,
                    "sp" => c64.cpu.sp = parse_byte(val)?,
                    "p"  => c64.cpu.p = Flags(parse_byte(val)?),
                    "pc" => c64.cpu.pc = parse_addr(val)?,
                    r => return Err(format!("unknown register: {r}")),
                }
                writeln!(out, "{}", reg_line(c64)).map_err(io)?;
            }
            "mem" | "m" => {
                let a = parse_addr(args.first().ok_or("usage: mem addr [end]")?)?;
                let b = args.get(1).map(|s| parse_addr(s)).transpose()?
                    .unwrap_or_else(|| a.saturating_add(0x7F));
                write!(out, "{}", Self::dump_mem(c64, a, b.max(a))).map_err(io)?;
            }
            "read" => {
                // Read via the bus, with side effects (CIA ICR, $D01E/$D01F)
                if args.is_empty() { return Err("usage: read addr [addr..]".into()); }
                let mut out_s = String::new();
                for a in args {
                    let addr = parse_addr(a)?;
                    let v = c64.bus.read(addr);
                    let _ = write!(out_s, "${addr:04X}=${v:02X} ");
                }
                c64.bus.dbg_watch_hit = None;
                writeln!(out, "{}", out_s.trim_end()).map_err(io)?;
            }
            "poke" => {
                let a = parse_addr(args.first().ok_or("usage: poke addr val [val..]")?)?;
                if args.len() < 2 { return Err("usage: poke addr val [val..]".into()); }
                for (i, v) in args[1..].iter().enumerate() {
                    c64.bus.write(a.wrapping_add(i as u16), parse_byte(v)?);
                }
                c64.bus.dbg_watch_hit = None;
                writeln!(out, "ok").map_err(io)?;
            }
            "fill" => {
                if args.len() < 3 { return Err("usage: fill from to val".into()); }
                let (a, b, v) = (parse_addr(args[0])?, parse_addr(args[1])?, parse_byte(args[2])?);
                for addr in a..=b { c64.bus.write(addr, v); }
                c64.bus.dbg_watch_hit = None;
                writeln!(out, "ok").map_err(io)?;
            }
            "find" => {
                if args.len() < 3 { return Err("usage: find from to byte [byte..]".into()); }
                let (a, b) = (parse_addr(args[0])?, parse_addr(args[1])?);
                let pat: Vec<u8> = args[2..].iter().map(|s| parse_byte(s)).collect::<Result<_, _>>()?;
                let mut found = 0;
                let mut addr = a as u32;
                while addr + pat.len() as u32 - 1 <= b as u32 {
                    if pat.iter().enumerate().all(|(i, p)| c64.bus.peek((addr + i as u32) as u16) == *p) {
                        writeln!(out, "found ${addr:04X}").map_err(io)?;
                        found += 1;
                        if found >= 50 { writeln!(out, "(truncated at 50 results)").map_err(io)?; break; }
                    }
                    addr += 1;
                }
                writeln!(out, "{found} match").map_err(io)?;
            }
            "dis" | "d" => {
                let a = args.first().map(|s| parse_addr(s)).transpose()?.unwrap_or(c64.cpu.pc);
                let n = args.get(1).and_then(|s| parse_dec(s)).unwrap_or(16) as usize;
                write!(out, "{}", Self::disassemble(c64, a, n)).map_err(io)?;
            }
            "stack" => write!(out, "{}", Self::stack(c64)).map_err(io)?,
            "screen" => write!(out, "{}", Self::screen(c64)).map_err(io)?,
            "bank" => write!(out, "{}", Self::bank(c64)).map_err(io)?,
            "vic" => write!(out, "{}", Self::vic(c64)).map_err(io)?,
            "sprites" => write!(out, "{}", Self::sprites(c64)).map_err(io)?,
            "cia1" => write!(out, "{}", Self::cia1(c64)).map_err(io)?,
            "cia2" => write!(out, "{}", Self::cia2(c64)).map_err(io)?,
            "sid" => {
                if let Some(m) = args.first() {
                    let (model, digiboost) = crate::frontend::session::parse_sid_model(m)
                        .ok_or("usage: sid [6581|8580|8580d]")?;
                    c64.set_sid_model(model, digiboost);
                }
                write!(out, "{}", Self::sid(c64)).map_err(io)?
            }
            "sid2" => {
                match args.first().copied() {
                    Some("off") => c64.set_sid2(None)?,
                    Some(a) => c64.set_sid2(Some(parse_addr(a)?))?,
                    None => {}
                }
                write!(out, "{}", Self::sid2(c64)).map_err(io)?
            }
            "audio" => match args.first().copied() {
                Some("raw") => {
                    let path = args.get(1).ok_or("usage: audio raw file")?;
                    if c64.bus.sid.audio_rate().is_none() {
                        c64.set_audio(Some(44_100))?;
                    }
                    c64.bus.sid.raw_capture = Some(Vec::new());
                    self.audio_raw = Some(path.to_string());
                    writeln!(out, "raw SID output (16 bits every cycle) to {path} ('audio off' to save it)").map_err(io)?;
                }
                Some("off") => {
                    if let (Some(path), Some(raw)) = (self.audio_raw.take(), c64.bus.sid.raw_capture.take()) {
                        let bytes: Vec<u8> = raw.iter().flat_map(|s| s.to_le_bytes()).collect();
                        std::fs::write(&path, bytes).map_err(|e| format!("{path}: {e}"))?;
                        writeln!(out, "raw output saved: {path} ({} cycles)", raw.len()).map_err(io)?;
                        if self.audio_rec.is_none() {
                            return Ok(());
                        }
                    }
                    let rec = self.audio_rec.take().ok_or("no recording in progress")?;
                    write_wav(&rec.path, rec.rate, rec.channels, &rec.samples).map_err(|e| format!("{}: {e}", rec.path))?;
                    if self.hook.is_none() {
                        c64.set_audio(None)?;
                    }
                    let frames = rec.samples.len() / rec.channels;
                    writeln!(out, "audio saved: {} ({frames} samples, {:.2} s at {} Hz, {})", rec.path,
                        frames as f64 / rec.rate as f64, rec.rate,
                        if rec.channels == 2 { "stereo" } else { "mono" }).map_err(io)?;
                }
                Some(path) => {
                    let rate = match args.get(1) {
                        Some(r) => r.parse().map_err(|_| "usage: audio file.wav [Hz] | audio off")?,
                        None => c64.bus.sid.audio_rate().unwrap_or(44_100),
                    };
                    c64.set_audio(Some(rate))?;
                    let channels = c64.audio_channels();
                    self.audio_rec = Some(AudioRec { path: path.to_string(), rate, channels, samples: Vec::new() });
                    writeln!(out, "recording audio at {rate} Hz to {path} ('audio off' to save it)").map_err(io)?;
                }
                None => return Err("usage: audio file.wav [Hz] | audio off".into()),
            },
            "swap" => {
                if self.media.len() < 2 {
                    return Err("swap: needs several files on the command line (e.g. side_a.tap side_b.tap)".into());
                }
                self.media_pos = (self.media_pos + 1) % self.media.len();
                writeln!(out, "{}", c64.change_medium(&self.media[self.media_pos])?).map_err(io)?;
            }
            "tape" => {
                use crate::tape::Button;
                const USAGE: &str = "usage: tape [play|record|stop|ff|rew|insert file|eject|save|rewind|counter|auto on|off|wobble [off|% Hz]|azimuth [cycles|off]|sound [on [vol]|off]]";
                let clk = c64.bus.cycle;
                match args.first().copied() {
                    Some("insert") => {
                        let path = args.get(1).ok_or("usage: tape insert file.tap")?;
                        if c64.insert_tape_file(path)? {
                            writeln!(out, "blank tape: {path} will be created on the first save").map_err(io)?;
                        }
                    }
                    Some("play") => c64.bus.tape.press(Button::Play, clk),
                    Some("record") => {
                        if c64.bus.tape.image.is_none() {
                            return Err("tape record: no tape ('tape insert file.tap')".into());
                        }
                        c64.bus.tape.press(Button::Record, clk)
                    }
                    Some("stop") => c64.bus.tape.press(Button::Stop, clk),
                    Some("ff") => c64.bus.tape.press(Button::Forward, clk),
                    Some("rew") => c64.bus.tape.press(Button::Rewind, clk),
                    Some("rewind") => c64.bus.tape.rewind(),
                    Some("counter") => c64.bus.tape.reset_counter(),
                    Some("eject") => {
                        if let Some(path) = c64.eject_tape()? {
                            writeln!(out, "recordings written to {path}").map_err(io)?;
                        }
                    }
                    Some("save") => match c64.save_tape_changes()? {
                        Some(path) => writeln!(out, "recordings written to {path}").map_err(io)?,
                        None => writeln!(out, "nothing to save").map_err(io)?,
                    },
                    Some("auto") => match args.get(1).copied() {
                        Some("on") => c64.tape_auto_buttons = true,
                        Some("off") => c64.tape_auto_buttons = false,
                        _ => return Err("usage: tape auto on|off".into()),
                    },
                    Some("wobble") => match (args.get(1).copied(), args.get(2)) {
                        (None, _) => {}
                        (Some("off"), _) => c64.bus.tape.set_wobble(0, 0),
                        (Some(pct), Some(hz)) => {
                            let num = |s: &str| s.parse::<f64>().ok().filter(|v| (0.0..=50.0).contains(v));
                            let (Some(pct), Some(hz)) = (num(pct), num(hz)) else {
                                return Err("usage: tape wobble PERCENT HZ (e.g. 0.5 3, max 50)".into());
                            };
                            c64.bus.tape.set_wobble((pct * 1000.0).round() as u32, (hz * 1000.0).round() as u32);
                        }
                        _ => return Err("usage: tape wobble [off | PERCENT HZ]".into()),
                    },
                    Some("azimuth") => match args.get(1).copied() {
                        None => {}
                        Some("off") => c64.bus.tape.set_azimuth_error(0),
                        Some(c) => match crate::frontend::session::parse_azimuth(c) {
                            Some(e) => c64.bus.tape.set_azimuth_error(e),
                            None => return Err("usage: tape azimuth CYCLES (0.001 to 10, e.g. 1.5) | off".into()),
                        },
                    },
                    Some("sound") => match (args.get(1).copied(), args.get(2)) {
                        (None, _) => {}
                        (Some("off"), _) => c64.bus.tape.set_sound(None),
                        (Some("on"), None) => c64.bus.tape.set_sound(Some(crate::tape::SOUND_VOLUME_DEFAULT)),
                        (Some("on"), Some(v)) => match v.parse::<u16>() {
                            Ok(v) if (1..=crate::tape::SOUND_VOLUME_MAX).contains(&v) => c64.bus.tape.set_sound(Some(v)),
                            _ => return Err("usage: tape sound on VOLUME (1-4096)".into()),
                        },
                        _ => return Err("usage: tape sound [on [volume] | off]".into()),
                    },
                    None => {}
                    _ => return Err(USAGE.into()),
                }
                write!(out, "{}", Self::tape(c64)).map_err(io)?
            }
            "drive" if args.first() == Some(&"insert") => {
                let path = args.get(1).ok_or("usage: drive insert file")?;
                writeln!(out, "{}", c64.insert_disk(path)?).map_err(io)?;
            }
            "drive" if args.first() == Some(&"trace") => {
                let d = c64.bus.drive.as_mut().ok_or("1541 drive: not emulated")?;
                match (args.get(1).copied(), args.get(2)) {
                    (Some("on"), Some(path)) => {
                        let f = std::fs::File::create(path).map_err(|e| format!("{path}: {e}"))?;
                        let mut w = std::io::BufWriter::new(f);
                        d.trace = Some(Box::new(move |cpu, bus, clk| {
                            let ins = disasm::disasm(&|a| bus.peek(a), cpu.pc);
                            let _ = writeln!(w, ".8:{:04x}  {:<11} {:<15}- A:{:02X} X:{:02X} Y:{:02X} SP:{:02x} {}  {}",
                                cpu.pc, ins.bytes_hex(), ins.text(), cpu.a, cpu.x, cpu.y, cpu.sp, flags_str(cpu.p), clk);
                        }));
                        writeln!(out, "drive trace to {path}").map_err(io)?;
                    }
                    (Some("off"), _) => {
                        d.trace = None;
                        writeln!(out, "drive trace off").map_err(io)?;
                    }
                    _ => return Err("usage: drive trace on file | off".into()),
                }
            }
            "drive" => write!(out, "{}", Self::drive(c64, &args)?).map_err(io)?,
            "reu" => {
                match args.first().copied() {
                    None => {}
                    Some("off") => c64.set_reu(None)?,
                    Some(kb) => c64.set_reu(Some(parse_dec(kb).and_then(|k| u32::try_from(k).ok()).ok_or("usage: reu [KB|off]")?))?,
                }
                write!(out, "{}", Self::reu(c64)).map_err(io)?;
            }
            "info" => {
                writeln!(out, "frame {} instr {} cycles {} raster {}/{} prg_pending={} typing={} disk={} cart={} breakpoints={} watches={} trace={}",
                    c64.frame_count, self.instr_count, c64.cpu.total_cycles,
                    c64.bus.vic.raster_line, c64.bus.vic.cycle,
                    c64.prg_pending() as u8, c64.bus.keyboard.typing() as u8, c64.disk.is_some() as u8, c64.bus.cart.is_some() as u8,
                    self.breakpoints.len(), c64.bus.dbg_watch.len(),
                    self.trace.as_ref().map(|t| t.lines.to_string()).unwrap_or_else(|| "off".into())).map_err(io)?;
            }

            "screenshot" => {
                let path = args.first().ok_or("usage: screenshot file.png [bar]")?;
                match args.get(1).copied() {
                    // The PNG itself on the output: for the remote monitor
                    None if *path == "-" => out.write_all(&crate::png::encode_argb(&c64.framebuffer, WIDTH, HEIGHT)).map_err(io)?,
                    None => {
                        crate::png::write_argb(path, &c64.framebuffer, WIDTH, HEIGHT).map_err(io)?;
                        writeln!(out, "screenshot saved: {path} ({WIDTH}x{HEIGHT})").map_err(io)?;
                    }
                    Some("bar") => {
                        // With the status bar below, as in the window
                        use crate::frontend::status::{Bar, Status, BAR_HEIGHT};
                        let mut s = Status::of(c64);
                        s.media = (self.media.len() > 1).then(|| (self.media_pos + 1, self.media.len()));
                        s.message = crate::notice::latest();
                        let mut bar = Bar::new(&c64.bus.char_rom);
                        bar.render(&s);
                        let mut img = c64.framebuffer.clone();
                        img.extend_from_slice(bar.pixels());
                        let h = HEIGHT + BAR_HEIGHT;
                        crate::png::write_argb(path, &img, WIDTH, h).map_err(io)?;
                        writeln!(out, "screenshot saved: {path} ({WIDTH}x{h}, with the status bar)").map_err(io)?;
                    }
                    Some(_) => return Err("usage: screenshot file.png [bar]".into()),
                }
            }
            "trace" => {
                match args.first() {
                    Some(&"on") => {
                        // trace on [file] [from to]
                        let mut rest_args = &args[1..];
                        let file = rest_args.first().filter(|f| parse_hex(f).is_none() || f.contains('.') || f.contains('/'));
                        let sink: Box<dyn IoWrite> = match file {
                            Some(f) => {
                                rest_args = &rest_args[1..];
                                Box::new(std::io::BufWriter::new(std::fs::File::create(f).map_err(io)?))
                            }
                            None => Box::new(std::io::stdout()),
                        };
                        let from = rest_args.first().map(|s| parse_addr(s)).transpose()?.unwrap_or(0);
                        let to = rest_args.get(1).map(|s| parse_addr(s)).transpose()?.unwrap_or(0xFFFF);
                        self.trace = Some(Trace { out: sink, from, to, lines: 0 });
                        writeln!(out, "trace on ${from:04X}-${to:04X} -> {}", file.unwrap_or(&"stdout")).map_err(io)?;
                    }
                    Some(&"off") => {
                        if let Some(mut t) = self.trace.take() {
                            t.out.flush().map_err(io)?;
                            writeln!(out, "trace off ({} lines)", t.lines).map_err(io)?;
                        } else {
                            writeln!(out, "trace already off").map_err(io)?;
                        }
                    }
                    _ => return Err("usage: trace on [file] [from to] | trace off".into()),
                }
            }

            "keys" => {
                // One pair of quotes around the text: the text may end with a quote
                let text = rest.strip_prefix('"').and_then(|t| t.strip_suffix('"')).unwrap_or(rest);
                let n = queue_keys(c64, text)?;
                writeln!(out, "{n} keys queued (one per frame)").map_err(io)?;
            }
            "key" => {
                let name = args.first().ok_or("usage: key name down|up|press")?;
                let action = args.get(1).copied().unwrap_or("press");
                if name.eq_ignore_ascii_case("restore") {
                    c64.cpu.nmi_pending = true;
                    writeln!(out, "RESTORE: NMI pending").map_err(io)?;
                    return Ok(());
                }
                let pos = crate::keyboard::key_positions(name).ok_or_else(|| format!("unknown key: {name}"))?;
                let press = |c64: &mut C64| for &(c, r) in &pos { c64.bus.keyboard.press(c, r); };
                let release = |c64: &mut C64| for &(c, r) in &pos { c64.bus.keyboard.release(c, r); };
                match action {
                    "down" => press(c64),
                    "up" => release(c64),
                    "press" => {
                        press(c64);
                        let stop = self.run(c64, Limit::Frames(4));
                        release(c64);
                        if !stop.is_limit() { self.report_stop(c64, &stop, out).map_err(io)?; return Ok(()); }
                        let stop = self.run(c64, Limit::Frames(2));
                        if !stop.is_limit() { self.report_stop(c64, &stop, out).map_err(io)?; return Ok(()); }
                    }
                    _ => return Err("usage: key name down|up|press".into()),
                }
                writeln!(out, "ok").map_err(io)?;
            }
            "joy1" | "joy2" => {
                if args.is_empty() { return Err("usage: joy2 [+|-]up|down|left|right|fire ... | none".into()); }
                let mut val = if cmd == "joy1" { c64.bus.joy1 } else { c64.bus.joy2 };
                for a in args {
                    if *a == "none" { val = 0xFF; continue; }
                    let (release, name) = match a.strip_prefix('-') {
                        Some(n) => (true, n),
                        None => (false, a.strip_prefix('+').unwrap_or(a)),
                    };
                    let bit = JOY_BITS.iter().find(|(n, _)| n.eq_ignore_ascii_case(name))
                        .map(|(_, b)| *b).ok_or_else(|| format!("unknown direction: {name}"))?;
                    if release { val |= bit; } else { val &= !bit; }
                }
                if cmd == "joy1" { c64.bus.joy1 = val; } else { c64.bus.joy2 = val; }
                writeln!(out, "{cmd}=${val:02X}").map_err(io)?;
            }

            "port" => {
                if let [n, dev] = args[..] {
                    let port = parse_port(n)?;
                    let dev = crate::ctrlport::Device::parse(dev)
                        .ok_or("usage: port 1|2 joystick|paddles|mouse|joymouse")?;
                    c64.set_control_port(port, dev);
                } else if !args.is_empty() {
                    return Err("usage: port [1|2 joystick|paddles|mouse|joymouse]".into());
                }
                let now = c64.bus.cycle;
                c64.bus.ctrl.sync(now);
                write!(out, "{}", c64.bus.ctrl.summary()).map_err(io)?;
            }
            "mouse" => {
                use crate::ctrlport::Button;
                const USAGE: &str = "usage: mouse dx dy | mouse left|right down|up|press";
                match args[..] {
                    [b @ ("left" | "right"), action] => {
                        let b = if b == "left" { Button::Left } else { Button::Right };
                        match action {
                            "down" => c64.mouse_button(b, true),
                            "up" => c64.mouse_button(b, false),
                            "press" => {
                                c64.mouse_button(b, true);
                                let stop = self.run(c64, Limit::Frames(4));
                                c64.mouse_button(b, false);
                                if !stop.is_limit() { self.report_stop(c64, &stop, out).map_err(io)?; return Ok(()); }
                            }
                            _ => return Err(USAGE.into()),
                        }
                    }
                    [dx, dy] => {
                        let parse = |v: &str| v.parse::<i32>().map_err(|_| USAGE.to_string());
                        c64.mouse_move(parse(dx)?, parse(dy)?);
                    }
                    _ => return Err(USAGE.into()),
                }
                if !c64.bus.ctrl.wants_mouse() {
                    writeln!(out, "(no paddles or mouse in the control ports: see port)").map_err(io)?;
                }
                writeln!(out, "ok").map_err(io)?;
            }
            "paddle" => {
                const USAGE: &str = "usage: paddle 1|2 x y (0-255)";
                let [n, x, y] = args[..] else { return Err(USAGE.into()) };
                let port = parse_port(n)?;
                let parse = |v: &str| v.parse::<u8>().map_err(|_| USAGE.to_string());
                c64.set_paddles(port, parse(x)?, parse(y)?);
                writeln!(out, "ok").map_err(io)?;
            }

            "load" => {
                let path = rest.trim_matches('"');
                if path.is_empty() { return Err("usage: load file".into()); }
                let msg = c64.load_file(path)?;
                writeln!(out, "{msg}").map_err(io)?;
            }
            "savestate" => {
                let path = rest.trim_matches('"');
                c64.save_state(path).map_err(io)?;
                writeln!(out, "state saved: {path}").map_err(io)?;
            }
            "loadstate" => {
                let path = rest.trim_matches('"');
                c64.load_state(path).map_err(io)?;
                writeln!(out, "state loaded: {path}").map_err(io)?;
                writeln!(out, "{}", reg_line(c64)).map_err(io)?;
            }
            "reset" => {
                c64.reset();
                writeln!(out, "{}", reg_line(c64)).map_err(io)?;
            }
            _ => return Err(format!("unknown command: {cmd} (help for the list)")),
        }
        Ok(())
    }
}

/// Queues `text` on the keyboard, one key per frame: `\n` is RETURN,
/// `{name}` a named key (`{f1}`, `{runstop}`, `{clr}`, `{left}`...). Returns
/// how many keys were queued.
pub fn queue_keys(c64: &mut C64, text: &str) -> Result<usize, String> {
    let mut keys = Vec::new();
    let mut chars = text.chars();
    while let Some(c) = chars.next() {
        let key = match c {
            '\\' => match chars.next() {
                Some('n') | Some('r') => '\r',
                Some(o) => o,
                None => break,
            },
            '{' => {
                let name: String = chars.by_ref().take_while(|&c| c != '}').collect();
                crate::keyboard::named_key_char(name.trim()).ok_or_else(|| {
                    let names: Vec<&str> = crate::keyboard::KEY_NAMES.iter().map(|(n, _)| *n).collect();
                    format!("unknown key {{{name}}}: {}", names.join(" "))
                })?
            }
            c => c,
        };
        keys.push(key);
    }
    for &k in &keys {
        c64.bus.keyboard.push_char(k);
    }
    Ok(keys.len())
}

/// Control port number, 1 or 2.
fn parse_port(s: &str) -> Result<usize, String> {
    match s {
        "1" => Ok(1),
        "2" => Ok(2),
        _ => Err(format!("control port 1 or 2, not {s}")),
    }
}
