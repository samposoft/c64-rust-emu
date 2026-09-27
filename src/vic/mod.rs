// SPDX-License-Identifier: GPL-2.0-or-later
// Copyright (C) 2026 SampoSoft - Francesco Sampoli
// Derived from VICE 3.10 (src/viciisc), Copyright (C) Hannu Nuotio,
// Daniel Kahlin, Ettore Perazzoli, Andreas Boose.
// Ported to Rust and modified by SampoSoft in 2026; see CREDITS.md.

//! Cycle-exact VIC-II 6569 (PAL) and 6567R8 (NTSC): a port of the x64sc
//! core of VICE 3.10 (`src/viciisc`: vicii-cycle.c, vicii-fetch.c,
//! vicii-draw-cycle.c, vicii-mem.c, vicii-chip-model.c). The two chips
//! differ in cycles per line (63, 65), raster lines (312, 263) and the
//! cycle table (`tables.rs`); see `crate::timing`.
//!
//! `cycle` does what VICE's `vicii_cycle()` does: it completes the phi2
//! accesses of the cycle just ended (after the CPU access), moves to the next
//! cycle, performs the phi1 access and the border checks, draws 8 pixels,
//! updates raster, sprites, bad line, VC/RC and returns BA for the CPU.
//!
//! Drawing is a pipeline as on the chip: graphics come out two cycles after
//! the g-access, sprites start when the X of the previous cycle matches the
//! register's (latched one cycle earlier), the main border uses the state of
//! the previous cycle and colors are resolved one cycle after drawing. So
//! every register takes effect on the same pixel as in VICE: for example a
//! write to $D016 at cycle 56 opens the right border, one to $D021 at cycle W
//! changes color from X = 8W - 111.

use crate::mem::Bus;
use crate::timing::Standard;

mod chip;
mod tables;

pub use chip::Chip;

/// Framebuffer width, the same for all standards.
pub const WIDTH: usize = 403;
/// Framebuffer height of PAL, the largest (NTSC: `Standard::fb_height`).
pub const HEIGHT: usize = 284;

// The framebuffer is centred horizontally on the 320×200 display area: VIC X
// coordinate (0-503) becomes fb_x = X + 17 (X=24, left border in 40 columns,
// lands at fb_x 41), with X ≥ 400 wrapped to the left (end of previous line).
/// First PAL framebuffer line of the 320×200 display area.
pub const DISPLAY_Y: usize = (HEIGHT - 200) / 2; // 42
/// First framebuffer column of the 320×200 display area.
pub const DISPLAY_X: usize = 41;

/// First PAL raster line mapped into the framebuffer: line 51 (start of the
/// 25-row display) lands at fb_y = DISPLAY_Y.
pub const FIRST_FB_LINE: u16 = 51 - DISPLAY_Y as u16; // 9

pub const C64_PALETTE: [u32; 16] = [
    0xFF000000, // 0  black
    0xFFFFFFFF, // 1  white
    0xFF813338, // 2  red
    0xFF75CEC8, // 3  cyan
    0xFF8E3C97, // 4  purple
    0xFF56AC4D, // 5  green
    0xFF2E2C9B, // 6  blue
    0xFFEDF171, // 7  yellow
    0xFF8E5029, // 8  orange
    0xFF553800, // 9  brown
    0xFFC46C71, // 10 pink
    0xFF4A4A4A, // 11 dark grey
    0xFF7B7B7B, // 12 medium grey
    0xFFA9FF9F, // 13 light green
    0xFF706DEB, // 14 light blue
    0xFFB2B2B2, // 15 light grey
];

/// Palette of the first revisions (6569R1, 6567R56A), with 5 lumas: the
/// colodore model with Pepto's levels for them (`vic::Chip::luma`).
pub const C64_PALETTE_OLD: [u32; 16] = [
    0xFF000000, // 0  black
    0xFFFFFFFF, // 1  white
    0xFF6D2327, // 2  red
    0xFFA0FEF8, // 3  cyan
    0xFFBA62C4, // 4  purple
    0xFF56AC4D, // 5  green
    0xFF2E2C9B, // 6  blue
    0xFFEDF171, // 7  yellow
    0xFFBA784D, // 8  orange
    0xFF553800, // 9  brown
    0xFFC46C71, // 10 pink
    0xFF373737, // 11 dark grey
    0xFF868686, // 12 medium grey
    0xFFA9FF9F, // 13 light green
    0xFF7A77F8, // 14 light blue
    0xFFE0E0E0, // 15 light grey
];

// ── Cycle table (vicii-chip-model.c) ─────────────────────────────────────────
//
// Index: our cycle k (0 to cycles per line - 1). k = Bauer cycle 1 onwards,
// k = 0 is the last cycle (63 PAL, 65 NTSC). RASTER advances at cycle 1, as
// in VICE (raster_cycle 0).

/// Cycles 15-54: c-access in phi2; drawing in the next cycle latches graphics.
const VISIBLE: u16 = 1 << 0;
/// Cycles 12-54: BA low on bad lines.
const FETCH_BA: u16 = 1 << 1;
const UPDATE_VC: u16 = 1 << 2; // 14
const UPDATE_RC: u16 = 1 << 3; // 58
const SPR_CRUNCH: u16 = 1 << 4; // 15
const UPDATE_MCBASE: u16 = 1 << 5; // 16
const SPR_DMA: u16 = 1 << 6; // 55, 56
const SPR_EXP: u16 = 1 << 7; // 56
const SPR_DISP: u16 = 1 << 8; // 58
const BRD_L1: u16 = 1 << 9; // 17: end of left border with CSEL=1
const BRD_L0: u16 = 1 << 10; // 18: with CSEL=0
const BRD_R0: u16 = 1 << 11; // 56: start of right border with CSEL=0
const BRD_R1: u16 = 1 << 12; // 57: with CSEL=1

/// VIC access in phi1. Sprites: pointer (phi1) + DMA 0 (phi2) in the first
/// cycle, DMA 1 (phi1) + DMA 2 (phi2) in the second.
#[derive(Clone, Copy, PartialEq, Debug)]
enum Phi1 {
    Idle,
    Refresh,
    FetchG,
    SprPtr(u8),
    SprDma1(u8),
}

#[derive(Clone, Copy, PartialEq, Debug)]
struct Cyc {
    flags: u16,
    phi1: Phi1,
    /// X of the cycle (phi1) rounded to 8, like VICE's table
    xpos: u16,
    /// Sprites that, if their DMA is active, hold BA low in this cycle
    spr_ba: u8,
}

/// Cycle table of the chip.
fn table(standard: Standard) -> &'static [Cyc] {
    match standard {
        Standard::Pal => &tables::PAL,
        Standard::Ntsc | Standard::PalN => &tables::NTSC,
        Standard::NtscOld => &tables::NTSC_OLD,
    }
}


// ── Drawing color codes (vicii-draw-cycle.c) ─────────────────────────────────
//
// Drawing writes codes: 0-15 fixed color, $20-$2E color register resolved
// one cycle later (so a write in the middle of a cycle changes the following
// pixels), $10-$15 sources that depend on c-access data.

const COL_NONE: u8 = 0x10;
const COL_VBUF_L: u8 = 0x11;
const COL_VBUF_H: u8 = 0x12;
const COL_CBUF: u8 = 0x13;
const COL_CBUF_MC: u8 = 0x14;
const COL_D02X_EXT: u8 = 0x15;
const COL_D020: u8 = 0x20;
const COL_D021: u8 = 0x21;
const COL_D022: u8 = 0x22;
const COL_D023: u8 = 0x23;
const COL_D025: u8 = 0x25;
const COL_D026: u8 = 0x26;
const COL_D027: u8 = 0x27;

/// Color per mode (ECM·16 + BMM·8 + MCM·4) and pixel bit pair.
const COLORS: [u8; 32] = [
    COL_D021, COL_D021, COL_CBUF, COL_CBUF, // text
    COL_D021, COL_D022, COL_D023, COL_CBUF_MC, // multicolor text
    COL_VBUF_L, COL_VBUF_L, COL_VBUF_H, COL_VBUF_H, // bitmap
    COL_D021, COL_VBUF_H, COL_VBUF_L, COL_CBUF, // multicolor bitmap
    COL_D02X_EXT, COL_D02X_EXT, COL_CBUF, COL_CBUF, // ECM
    COL_NONE, COL_NONE, COL_NONE, COL_NONE, // invalid modes: black
    COL_NONE, COL_NONE, COL_NONE, COL_NONE,
    COL_NONE, COL_NONE, COL_NONE, COL_NONE,
];

/// Sprite state (vicii_sprite_t).
#[derive(Clone, Copy, Default)]
pub struct Sprite {
    /// X coordinate (9 bits) from $D000-$D00F and $D010
    pub x: u16,
    /// 24 bits read by the line's DMA (also without DMA: bus and idle)
    pub data: u32,
    pub mc: u8,
    pub mcbase: u8,
    pub pointer: u8,
    pub exp_flop: bool,
}

/// Drawing pipeline (the static variables of vicii-draw-cycle.c).
struct Draw {
    gbuf_pipe0: u8,
    cbuf_pipe0: u8,
    vbuf_pipe0: u8,
    gbuf_pipe1: u8,
    cbuf_pipe1: u8,
    vbuf_pipe1: u8,
    xscroll_pipe: u8,
    vmode11_pipe: u8,
    vmode16_pipe: u8,
    vmode16_pipe2: u8,
    gbuf_reg: u8,
    gbuf_mc_flop: bool,
    gbuf_pixel_reg: u8,
    cbuf_reg: u8,
    vbuf_reg: u8,
    dmli: u8,
    sprite_x_pipe: [u16; 8],
    sprite_pri_bits: u8,
    sprite_mc_bits: u8,
    sprite_expx_bits: u8,
    sprite_pending_bits: u8,
    sprite_active_bits: u8,
    sprite_halt_bits: u8,
    sbuf_reg: [u32; 8],
    sbuf_pixel_reg: [u8; 8],
    sbuf_expx_flops: u8,
    sbuf_mc_flops: u8,
    border_state: bool,
    render_buffer: [u8; 8],
    pri_buffer: [u8; 8],
    pixel_buffer: [u8; 8],
    /// Color registers as seen by drawing (0-15 identity, $20-$2E registers)
    cregs: [u8; 0x2F],
    last_color_reg: u8,
    last_color_value: u8,
}

impl Draw {
    fn new() -> Self {
        let mut cregs = [0u8; 0x2F];
        for (i, c) in cregs.iter_mut().enumerate().take(16) {
            *c = i as u8;
        }
        Self {
            gbuf_pipe0: 0, cbuf_pipe0: 0, vbuf_pipe0: 0,
            gbuf_pipe1: 0, cbuf_pipe1: 0, vbuf_pipe1: 0,
            xscroll_pipe: 0, vmode11_pipe: 0, vmode16_pipe: 0, vmode16_pipe2: 0,
            gbuf_reg: 0, gbuf_mc_flop: false, gbuf_pixel_reg: 0, cbuf_reg: 0, vbuf_reg: 0,
            dmli: 0,
            sprite_x_pipe: [0; 8], sprite_pri_bits: 0, sprite_mc_bits: 0,
            sprite_expx_bits: 0, sprite_pending_bits: 0, sprite_active_bits: 0,
            sprite_halt_bits: 0, sbuf_reg: [0; 8], sbuf_pixel_reg: [0; 8],
            sbuf_expx_flops: 0, sbuf_mc_flops: 0,
            border_state: false,
            render_buffer: [0; 8], pri_buffer: [0; 8], pixel_buffer: [0; 8],
            cregs, last_color_reg: 0xFF, last_color_value: 0,
        }
    }
}

/// Output of one VIC cycle to the CPU.
#[derive(Clone, Copy, Default)]
pub struct Tick {
    /// BA low: the CPU only completes writes
    pub ba_low: bool,
    /// The VIC uses the bus in phi2 (c-access or sprite DMA): CPU stalled
    pub aec_low: bool,
    /// The picture is complete (end of `Standard::frame_end_line`)
    pub frame_done: bool,
}

/// Internal VIC-II state (the needed subset of `vicii_t`).
pub struct VicState {
    /// The chip (`C64::set_chip`) and its standard
    pub chip: Chip,
    pub standard: Standard,
    /// Registers; $D019 holds the IRQ status (VICE's irq_status)
    pub regs: [u8; 64],
    pub raster_line: u16,
    /// Current cycle, our numbering (see the cycle table)
    pub cycle: u16,
    raster_irq_triggered: bool,
    start_of_frame: bool,
    allow_bad_lines: bool,
    pub bad_line: bool,
    pub idle_state: bool,
    pub vc: u16,
    pub vcbase: u16,
    pub rc: u8,
    vmli: u8,
    /// c-access line buffer: codes and color nibbles
    pub vbuf: [u8; 40],
    pub cbuf: [u8; 40],
    gbuf: u8,
    refresh_counter: u8,
    prefetch_cycles: u8,
    last_bus_phi2: u8,
    /// Byte read by the VIC in the phi1 half of the current cycle: the CPU
    /// sees it when it reads an unconnected address (open bus).
    pub last_read_phi1: u8,
    reg11_delay: u8,
    pub main_border: bool,
    pub vborder: bool,
    set_vborder: bool,
    pub sprites: [Sprite; 8],
    pub sprite_dma: u8,
    pub sprite_display_bits: u8,
    sprite_sprite_collisions: u8,
    sprite_background_collisions: u8,
    clear_collisions: u8,
    /// Color register write in the CPU cycle, for drawing
    last_color_reg: u8,
    last_color_value: u8,
    /// Framebuffer line being drawn (changes at cycle 2)
    dbuf_line: u16,
    d: Draw,
}

impl VicState {
    pub fn new() -> Self {
        // At reset all VIC registers are 0 (DEN=0: border-colored screen, no
        // bad lines until the KERNAL programs them in CINT).
        Self {
            chip: Chip::Mos6569,
            standard: Standard::Pal,
            regs: [0u8; 64],
            raster_line: 0,
            cycle: 1,
            raster_irq_triggered: false,
            start_of_frame: false,
            allow_bad_lines: false,
            bad_line: false,
            idle_state: true,
            vc: 0,
            vcbase: 0,
            rc: 0,
            vmli: 0,
            vbuf: [0; 40],
            cbuf: [0; 40],
            gbuf: 0,
            refresh_counter: 0xFF,
            prefetch_cycles: 0,
            last_bus_phi2: 0xFF,
            last_read_phi1: 0,
            reg11_delay: 0,
            main_border: true,
            vborder: true,
            set_vborder: true,
            sprites: [Sprite::default(); 8],
            sprite_dma: 0,
            sprite_display_bits: 0,
            sprite_sprite_collisions: 0,
            sprite_background_collisions: 0,
            clear_collisions: 0,
            last_color_reg: 0xFF,
            last_color_value: 0,
            dbuf_line: 0,
            d: Draw::new(),
        }
    }

    /// Register read without side effects (debugger, peek).
    pub fn read(&self, reg: u8) -> u8 {
        let r = (reg & 0x3F) as usize;
        match r {
            // Bit 7 = bit 8 of the current raster
            0x11 => (self.regs[0x11] & 0x7F) | (((self.raster_line >> 1) & 0x80) as u8),
            0x12 => (self.raster_line & 0xFF) as u8,
            0x13 | 0x14 => 0, // light pen
            // Unimplemented bits read as 1 (as on hardware and in VICE)
            0x16 => self.regs[r] | 0xC0,
            0x18 => self.regs[r] | 0x01,
            0x19 => self.regs[r] | 0x70,
            0x1A => self.regs[r] | 0xF0,
            0x1E => self.sprite_sprite_collisions,
            0x1F => self.sprite_background_collisions,
            0x20..=0x2E => self.regs[r] | 0xF0,
            0x2F..=0x3F => 0xFF,
            _ => self.regs[r],
        }
    }

    /// CPU read: collisions are cleared by the next VIC cycle, and the value
    /// stays on the VIC's internal bus (seen by the phi2 accesses of sprites
    /// without DMA).
    pub fn read_mut(&mut self, reg: u8) -> u8 {
        let r = reg & 0x3F;
        let v = self.read(r);
        if r == 0x1E || r == 0x1F {
            self.regs[r as usize] = v;
            self.clear_collisions = r;
        }
        self.last_bus_phi2 = v;
        v
    }

    pub fn write(&mut self, reg: u8, val: u8) {
        let r = (reg & 0x3F) as usize;
        self.last_bus_phi2 = val;
        match r {
            0x00 | 0x02 | 0x04 | 0x06 | 0x08 | 0x0A | 0x0C | 0x0E => {
                self.regs[r] = val;
                let n = r >> 1;
                self.sprites[n].x = val as u16 | (((self.regs[0x10] >> n) as u16 & 1) << 8);
            }
            0x10 => {
                self.regs[0x10] = val;
                for n in 0..8 {
                    self.sprites[n].x = self.regs[n * 2] as u16 | (((val >> n) as u16 & 1) << 8);
                }
            }
            0x13 | 0x14 | 0x1E | 0x1F => {}
            // $D017: with the bit at 0 the expansion flip-flop goes back to 1; if
            // it was 0 at cycle 15, MC gets "stretched" (sprite crunch)
            0x17 => {
                for n in 0..8 {
                    let b = 1u8 << n;
                    if val & b == 0 && !self.sprites[n].exp_flop {
                        if table(self.standard)[self.cycle as usize].flags & SPR_CRUNCH != 0 {
                            let s = &mut self.sprites[n];
                            s.mc = (0x2A & (s.mcbase & s.mc)) | (0x15 & (s.mcbase | s.mc));
                        }
                        self.sprites[n].exp_flop = true;
                    }
                }
                self.regs[0x17] = val;
            }
            // $D019: writing 1 to a bit clears that IRQ flag
            0x19 => {
                self.regs[0x19] &= !((val & 0x0F) | 0x80);
                self.update_irq_flag();
            }
            0x1A => {
                self.regs[0x1A] = val & 0x0F;
                self.update_irq_flag();
            }
            0x20..=0x2E => {
                self.regs[r] = val & 0x0F;
                self.last_color_reg = r as u8;
                self.last_color_value = val & 0x0F;
            }
            0x2F..=0x3F => {}
            _ => self.regs[r] = val,
        }
    }

    /// Bit 7 of $D019 (IRQ line) from flags and enables.
    /// C64 reset. The VIC-II has no reset pin, but VICE (vicii_reset)
    /// restarts the raster at line 0, clears the interrupt mask ($D01A) and
    /// closes the borders, keeping the other registers; we do the same, so
    /// the machine after a reset runs as in VICE. Like VICE it does not
    /// recompute $D019 bit 7: an IRQ already signalled stays until it is
    /// acknowledged.
    pub fn reset(&mut self) {
        self.raster_line = 0;
        self.cycle = 1;
        self.start_of_frame = false;
        self.raster_irq_triggered = false;
        self.regs[0x1A] = 0;
        self.vborder = true;
        self.set_vborder = true;
        self.main_border = true;
    }

    pub(crate) fn update_irq_flag(&mut self) {
        if self.regs[0x19] & self.regs[0x1A] & 0x0F != 0 {
            self.regs[0x19] |= 0x80;
        } else {
            self.regs[0x19] &= 0x7F;
        }
    }

    /// Raster IRQ line (from $D011 bit 7 and $D012).
    pub fn raster_target(&self) -> u16 {
        (((self.regs[0x11] & 0x80) as u16) << 1) | self.regs[0x12] as u16
    }

    /// True in the cycle where RASTER moves to a new line: cycle 1, but cycle
    /// 2 for line 0 (in VICE the last line lasts one extra cycle).
    pub fn line_start(&self) -> bool {
        self.cycle == if self.raster_line == 0 { 2 } else { 1 }
    }

    /// The VIC shows the screen (display state) rather than idle graphics.
    pub fn display_state(&self) -> bool {
        !self.idle_state
    }

    fn check_hborder(&mut self, flags: u16) {
        let csel = self.regs[0x16] & 0x08 != 0;
        let left = (flags & BRD_L1 != 0 && csel) || (flags & BRD_L0 != 0 && !csel);
        if left {
            self.check_vborder_bottom();
            self.vborder = self.set_vborder;
            if !self.vborder {
                self.main_border = false;
            }
        }
        let right = (flags & BRD_R0 != 0 && !csel) || (flags & BRD_R1 != 0 && csel);
        if right {
            self.main_border = true;
        }
    }

    fn check_vborder_top(&mut self) {
        let top = if self.regs[0x11] & 0x08 != 0 { 0x33 } else { 0x37 };
        if self.raster_line == top && self.regs[0x11] & 0x10 != 0 {
            self.vborder = false;
            self.set_vborder = false;
        }
    }

    fn check_vborder_bottom(&mut self) {
        let bottom = if self.regs[0x11] & 0x08 != 0 { 0xFB } else { 0xF7 };
        if self.raster_line == bottom {
            self.set_vborder = true;
        }
    }

    fn sprite_logic(&mut self, flags: u16) {
        // Cycle 16: MCBASE advances (if the expansion flip-flop is set)
        if flags & UPDATE_MCBASE != 0 {
            for n in 0..8 {
                let s = &mut self.sprites[n];
                if s.exp_flop {
                    s.mcbase = s.mc;
                    if s.mcbase == 63 {
                        self.sprite_dma &= !(1 << n);
                    }
                }
            }
        }
        // Cycles 55 and 56: DMA turns on if Y matches
        if flags & SPR_DMA != 0 {
            let enable = self.regs[0x15];
            for n in 0..8 {
                let b = 1u8 << n;
                if enable & b != 0 && self.regs[n * 2 + 1] == self.raster_line as u8
                    && self.sprite_dma & b == 0
                {
                    self.sprite_dma |= b;
                    self.sprites[n].mcbase = 0;
                    self.sprites[n].exp_flop = true;
                }
            }
        }
        // Cycle 56: the Y expansion flip-flop toggles
        if flags & SPR_EXP != 0 {
            let yexp = self.regs[0x17];
            for n in 0..8 {
                let b = 1u8 << n;
                if self.sprite_dma & b != 0 && yexp & b != 0 {
                    self.sprites[n].exp_flop = !self.sprites[n].exp_flop;
                }
            }
        }
        // Cycle 58: MC restarts from MCBASE, decide which sprites are displayed
        if flags & SPR_DISP != 0 {
            let enable = self.regs[0x15];
            for n in 0..8 {
                let b = 1u8 << n;
                self.sprites[n].mc = self.sprites[n].mcbase;
                if self.sprite_dma & b != 0 {
                    if enable & b != 0 && self.regs[n * 2 + 1] == self.raster_line as u8 {
                        self.sprite_display_bits |= b;
                    }
                } else {
                    self.sprite_display_bits &= !b;
                }
            }
        }
    }
}

impl Default for VicState {
    fn default() -> Self {
        Self::new()
    }
}

// ── Memory accesses (vicii-fetch.c) ──────────────────────────────────────────

fn is_char_rom(bus: &Bus, addr: u16) -> bool {
    let bank = bus.vic_bank();
    (bank == 0 || bank == 2) && addr & 0x3000 == 0x1000
}

fn g_fetch_addr(v: &VicState, mode: u8) -> u16 {
    let mut a = if mode & 0x20 != 0 {
        (v.vc << 3) | v.rc as u16 | (((v.regs[0x18] & 0x08) as u16) << 10)
    } else {
        ((v.vbuf[(v.vmli as usize).min(39)] as u16) << 3) | v.rc as u16
            | (((v.regs[0x18] & 0x0E) as u16) << 10)
    };
    if mode & 0x40 != 0 {
        a &= 0x39FF;
    }
    a
}

/// g-access in display state. NMOS: the previous cycle's mode for BMM;
/// HMOS: the previous cycle's $D011 for all bits.
fn fetch_graphics(bus: &mut Bus) -> u8 {
    let v = &bus.vic;
    let r11 = v.regs[0x11];
    let delay = v.reg11_delay;
    let mut addr = if v.chip.hmos() { g_fetch_addr(v, delay) } else { g_fetch_addr(v, r11 | (delay & 0x20)) };
    if !v.chip.hmos() && (r11 ^ delay) & 0x20 != 0 {
        // Switch from RAM to character ROM: the low address bits stay those
        // of the previous mode (6569)
        let from = g_fetch_addr(v, delay);
        let to = g_fetch_addr(v, r11);
        if !is_char_rom(bus, from) && is_char_rom(bus, to) {
            addr = (from & 0xFF) | (to & 0x3F00);
        }
    }
    let data = bus.vic_read(addr);
    let v = &mut bus.vic;
    v.gbuf = data;
    v.vmli += 1;
    v.vc = (v.vc + 1) & 0x3FF;
    data
}

/// g-access in idle state: $3FFF, or $39FF with ECM (HMOS: ECM of the
/// previous cycle).
fn fetch_idle_gfx(bus: &mut Bus) -> u8 {
    let r11 = if bus.vic.chip.hmos() { bus.vic.reg11_delay } else { bus.vic.regs[0x11] };
    let addr = if r11 & 0x40 != 0 { 0x39FF } else { 0x3FFF };
    bus.vic.gbuf = bus.vic_read(addr);
    bus.vic.gbuf
}

fn sprite_addr(v: &VicState, n: usize) -> u16 {
    ((v.sprites[n].pointer as u16) << 6) + v.sprites[n].mc as u16
}

/// DMA 0 (phi2 of the pointer cycle) and DMA 2 (phi2 of the next cycle).
/// Without DMA the sprite takes the byte on the VIC's internal bus.
fn sprite_dma_phi2(bus: &mut Bus, n: usize, shift: u32) {
    let mut data = bus.vic.last_bus_phi2;
    if bus.vic.sprite_dma & (1 << n) != 0 {
        if bus.vic.prefetch_cycles == 0 {
            data = bus.vic_read(sprite_addr(&bus.vic, n));
        }
        let s = &mut bus.vic.sprites[n];
        s.mc = (s.mc + 1) & 0x3F;
    }
    let s = &mut bus.vic.sprites[n];
    s.data = (s.data & !(0xFF << shift)) | ((data as u32) << shift);
}

/// DMA 1 (phi1): without DMA it is an idle read of $3FFF.
fn sprite_dma_1(bus: &mut Bus, n: usize) -> u8 {
    let data = if bus.vic.sprite_dma & (1 << n) != 0 {
        let d = bus.vic_read(sprite_addr(&bus.vic, n));
        let s = &mut bus.vic.sprites[n];
        s.mc = (s.mc + 1) & 0x3F;
        d
    } else {
        bus.vic_read(0x3FFF)
    };
    let s = &mut bus.vic.sprites[n];
    s.data = (s.data & 0xFF00FF) | ((data as u32) << 8);
    data
}

// ── VIC cycle (vicii-cycle.c) ────────────────────────────────────────────────

/// One VIC cycle, to be called before the CPU's bus access in that cycle.
/// `cpu_pc` is used for the c-access in the first three BA cycles, when the
/// bus still belongs to the CPU (the VIC reads $FF and color nibble RAM[PC]).
pub fn cycle(bus: &mut Bus, fb: &mut [u32], cpu_pc: u16) -> Tick {
    let standard = bus.vic.standard;
    let tab = table(standard);
    let n = standard.cycles_per_line();
    // Phi2 of the cycle just ended, after the CPU access: sprite DMA
    match tab[bus.vic.cycle as usize].phi1 {
        Phi1::SprPtr(n) => sprite_dma_phi2(bus, n as usize, 16),
        Phi1::SprDma1(n) => sprite_dma_phi2(bus, n as usize, 0),
        _ => {}
    }

    let k = (bus.vic.cycle + 1) % n;
    bus.vic.cycle = k;
    let c = tab[k as usize];

    // Phi1. Every access reads memory: the CPU sees the byte on the open bus.
    bus.vic.last_read_phi1 = match c.phi1 {
        Phi1::FetchG => {
            if !bus.vic.idle_state { fetch_graphics(bus) } else { fetch_idle_gfx(bus) }
        }
        Phi1::SprPtr(n) => {
            let vm = ((bus.vic.regs[0x18] & 0xF0) as u16) << 6;
            let p = bus.vic_read(vm + 0x3F8 + n as u16);
            bus.vic.sprites[n as usize].pointer = p;
            p
        }
        Phi1::SprDma1(n) => sprite_dma_1(bus, n as usize),
        Phi1::Refresh => {
            let r = bus.vic.refresh_counter;
            bus.vic.refresh_counter = r.wrapping_sub(1);
            bus.vic_read(0x3F00 | r as u16)
        }
        Phi1::Idle => bus.vic_read(0x3FFF),
    };

    let v = &mut bus.vic;
    v.check_hborder(c.flags);

    let can_ss = v.sprite_sprite_collisions == 0;
    let can_sb = v.sprite_background_collisions == 0;
    draw_cycle(v, fb, tab);
    match v.clear_collisions {
        0x1E => {
            v.sprite_sprite_collisions = 0;
            v.clear_collisions = 0;
        }
        0x1F => {
            v.sprite_background_collisions = 0;
            v.clear_collisions = 0;
        }
        _ => {}
    }
    if can_ss && v.sprite_sprite_collisions != 0 {
        v.regs[0x19] |= 0x04;
        v.update_irq_flag();
    }
    if can_sb && v.sprite_background_collisions != 0 {
        v.regs[0x19] |= 0x02;
        v.update_irq_flag();
    }

    // End of line and start of the next
    let mut frame_done = false;
    if k == 1 {
        if v.raster_line == standard.raster_lines() - 1 {
            v.start_of_frame = true;
        }
        if v.raster_line == standard.frame_end_line() {
            frame_done = true;
        }
        // DEN in the first cycle of the line after $30; no bad lines from $F7
        if v.raster_line == 0x30 && !v.allow_bad_lines && v.regs[0x11] & 0x10 != 0 {
            v.allow_bad_lines = true;
        }
        if v.raster_line == 0xF7 {
            v.allow_bad_lines = false;
        }
        v.bad_line = false;
    }
    if v.start_of_frame {
        // Line 0 starts at cycle 2
        if k == 2 {
            v.start_of_frame = false;
            v.raster_line = 0;
            v.refresh_counter = 0xFF;
            v.allow_bad_lines = false;
            v.vcbase = 0;
            v.vc = 0;
        }
    } else if k == 1 {
        v.raster_line += 1;
    }

    // Raster IRQ: on the transition from not-equal to equal, in any cycle
    if v.raster_line == v.raster_target() {
        if !v.raster_irq_triggered {
            if v.regs[0x19] & 0x01 == 0 {
                v.regs[0x19] |= 0x01;
                v.update_irq_flag();
            }
            v.raster_irq_triggered = true;
        }
    } else {
        v.raster_irq_triggered = false;
    }

    // Vertical border
    v.check_vborder_top();
    v.check_vborder_bottom();
    if k == 1 {
        v.vborder = v.set_vborder;
    }

    v.sprite_logic(c.flags);

    // Bad line, VC and RC
    if v.raster_line == 0x30 && !v.allow_bad_lines {
        v.allow_bad_lines = v.regs[0x11] & 0x10 != 0;
    }
    if v.allow_bad_lines {
        if (v.raster_line & 7) as u8 == v.regs[0x11] & 7 {
            v.bad_line = true;
            v.idle_state = false;
        } else {
            v.bad_line = false;
        }
    }
    if c.flags & UPDATE_VC != 0 {
        v.vc = v.vcbase;
        v.vmli = 0;
        if v.bad_line {
            v.rc = 0;
        }
    }
    if c.flags & UPDATE_RC != 0 {
        if v.rc == 7 {
            v.idle_state = true;
            v.vcbase = v.vc;
        }
        if !v.idle_state || v.bad_line {
            v.rc = (v.rc + 1) & 7;
            v.idle_state = false;
        }
    }

    // BA: the CPU keeps the bus for 3 cycles after BA goes low
    let ba_low = (v.bad_line && c.flags & FETCH_BA != 0) || v.sprite_dma & c.spr_ba != 0;
    if ba_low {
        v.prefetch_cycles = v.prefetch_cycles.saturating_sub(1);
    } else {
        v.prefetch_cycles = 4;
    }

    // c-access (phi2)
    let mut aec_low = false;
    if v.bad_line && c.flags & VISIBLE != 0 {
        let i = (v.vmli as usize).min(39);
        if v.prefetch_cycles != 0 {
            v.vbuf[i] = 0xFF;
            v.cbuf[i] = bus.ram[cpu_pc as usize] & 0x0F;
        } else {
            let vm = ((v.regs[0x18] & 0xF0) as u16) << 6;
            let vc = v.vc;
            let code = bus.vic_read(vm + vc);
            let v = &mut bus.vic;
            v.vbuf[i] = code;
            v.cbuf[i] = bus.color_ram[vc as usize & 0x3FF] & 0x0F;
            aec_low = true;
        }
    }
    let v = &mut bus.vic;
    // Sprite DMA in phi2 of this cycle (performed at the start of the next)
    if let Phi1::SprPtr(n) | Phi1::SprDma1(n) = c.phi1 {
        if v.sprite_dma & (1 << n) != 0 && v.prefetch_cycles == 0 {
            aec_low = true;
        }
    }

    v.last_bus_phi2 = 0xFF;
    v.reg11_delay = v.regs[0x11];
    Tick { ba_low, aec_low, frame_done }
}

// ── Drawing (vicii-draw-cycle.c) ─────────────────────────────────────────────
//
// The NMOS chips (6569, 6567, 6572: VICE's color_latency) and the HMOS ones
// (8565, 8562) differ in when they latch the mode bits of $D011 and the
// sprite multicolor bits, and in how they resolve colors (`draw_colors8`).

fn draw_cycle(v: &mut VicState, fb: &mut [u32], tab: &[Cyc]) {
    let k = v.cycle;
    if k == 2 {
        v.dbuf_line = if v.start_of_frame { 0 } else { v.raster_line };
    }
    // Drawing uses the flags of the previous cycle (cycle_flags_pipe)
    let n = tab.len() as u16;
    let pc = tab[((k + n - 1) % n) as usize];
    // Fast path: 8 pixels all border, no sprite to draw and an empty graphics
    // pipeline. The result is the same as full drawing, which here would only
    // update the pipeline state.
    let d = &v.d;
    let covered = d.border_state && v.main_border;
    if covered && d.gbuf_reg == 0 && d.gbuf_pipe1 == 0 && d.gbuf_pixel_reg == 0
        && sprites_quiet(v, &pc)
    {
        skip_graphics8(v, &pc);
    } else {
        draw_graphics8(v, &pc);
    }
    draw_sprites8(v, &pc);
    draw_border8(v);
    draw_colors8(v, fb);
}

/// No sprite can produce pixels in this cycle: none active and none that
/// could start (no DMA and no display check).
#[inline]
fn sprites_quiet(v: &VicState, pc: &Cyc) -> bool {
    let d = &v.d;
    if d.sprite_active_bits != 0 || pc.flags & SPR_DISP != 0 {
        return false;
    }
    if d.sprite_pending_bits == 0 {
        return true;
    }
    (0..8).all(|s| d.sprite_pending_bits & (1 << s) == 0
        || pc.xpos & 0x1F8 != d.sprite_x_pipe[s] & 0x1F8)
}

/// Like `draw_graphics8` when the graphics pipeline holds only zeros and the
/// cycle is covered by the border: pixels stay 0, only registers change.
fn skip_graphics8(v: &mut VicState, pc: &Cyc) {
    let r11 = v.regs[0x11];
    let r16 = v.regs[0x16];
    let d = &mut v.d;
    let new16 = (r16 & 0x10) >> 2;
    let mut mc = d.gbuf_mc_flop;
    for i in 0..8u8 {
        if i == 7 && new16 != 0 && d.vmode16_pipe2 == 0 {
            mc = false;
        }
        if i == d.xscroll_pipe {
            mc = true;
        }
        mc = !mc;
    }
    d.gbuf_mc_flop = mc;
    d.vbuf_reg = d.vbuf_pipe1;
    d.cbuf_reg = d.cbuf_pipe1;
    d.vmode16_pipe = new16;
    d.vmode16_pipe2 = new16;
    d.vmode11_pipe = (r11 & 0x60) >> 2;
    d.pri_buffer = [0; 8];
    advance_pipes(v, pc);
}

fn draw_graphics8(v: &mut VicState, pc: &Cyc) {
    let r11 = v.regs[0x11];
    let r16 = v.regs[0x16];
    let hmos = v.chip.hmos();
    let d = &mut v.d;
    let new11 = (r11 & 0x60) >> 2;
    let new16 = (r16 & 0x10) >> 2;
    if d.vmode11_pipe == new11 && d.vmode16_pipe == new16 && d.vmode16_pipe2 == new16 {
        // Mode unchanged: the updates at pixels 4, 6 and 7 change nothing and
        // the cycle splits into two runs around the load
        let xs = d.xscroll_pipe;
        draw_graphics_run(d, 0, xs.min(8));
        if xs < 8 {
            d.vbuf_reg = d.vbuf_pipe1;
            d.cbuf_reg = d.cbuf_pipe1;
            d.gbuf_reg = d.gbuf_pipe1;
            d.gbuf_mc_flop = true;
            draw_graphics_run(d, xs, 8);
        }
        advance_pipes(v, pc);
        return;
    }
    for i in 0..8u8 {
        match i {
            4 => {
                d.vmode16_pipe = (r16 & 0x10) >> 2;
                if !hmos {
                    d.vmode11_pipe |= (r11 & 0x60) >> 2; // rising edge
                }
            }
            6 if !hmos => d.vmode11_pipe &= (r11 & 0x60) >> 2, // falling edge
            7 => {
                if d.vmode16_pipe != 0 && d.vmode16_pipe2 == 0 {
                    d.gbuf_mc_flop = false;
                }
                d.vmode16_pipe2 = d.vmode16_pipe;
            }
            _ => {}
        }
        draw_graphics(d, i);
    }
    if hmos {
        // The HMOS chips take the $D011 mode bits at the end of the cycle
        d.vmode11_pipe = new11;
    }
    advance_pipes(v, pc);
}

/// End of the cycle's graphics drawing: g-access and c-access data advance
/// one stage in the pipeline.
#[inline]
fn advance_pipes(v: &mut VicState, pc: &Cyc) {
    let r16 = v.regs[0x16];
    let d = &mut v.d;
    d.vbuf_pipe1 = d.vbuf_pipe0;
    d.cbuf_pipe1 = d.cbuf_pipe0;
    d.gbuf_pipe1 = d.gbuf_pipe0;
    let vis_en = pc.flags & VISIBLE != 0 && !v.vborder;
    if vis_en {
        d.gbuf_pipe0 = v.gbuf;
        d.xscroll_pipe = r16 & 0x07;
        if !v.idle_state {
            let i = (d.dmli as usize).min(39);
            d.vbuf_pipe0 = v.vbuf[i];
            d.cbuf_pipe0 = v.cbuf[i];
            d.dmli += 1;
        } else {
            d.vbuf_pipe0 = 0;
            d.cbuf_pipe0 = 0;
        }
    } else {
        d.gbuf_pipe0 = 0;
        d.dmli = 0;
    }
}

/// Pixels `from..to` of `draw_graphics` with no loads or mode changes: mode
/// and colors computed once.
#[inline]
fn draw_graphics_run(d: &mut Draw, from: u8, to: u8) {
    let vmode = d.vmode11_pipe | d.vmode16_pipe;
    let mut codes = [0u8; 4];
    for (px, c) in codes.iter_mut().enumerate() {
        *c = match COLORS[vmode as usize | px] {
            COL_NONE => 0,
            COL_VBUF_L => d.vbuf_reg & 0x0F,
            COL_VBUF_H => d.vbuf_reg >> 4,
            COL_CBUF => d.cbuf_reg,
            COL_CBUF_MC => d.cbuf_reg & 0x07,
            COL_D02X_EXT => COL_D021 + (d.vbuf_reg >> 6),
            c => c,
        };
    }
    let mc_char = d.vmode11_pipe & 0x08 != 0 || d.cbuf_reg & 0x08 != 0;
    let multi = d.vmode16_pipe2 != 0 && mc_char;
    let hi = if d.vmode16_pipe2 == 0 && mc_char { 2 } else { 3 };
    for i in from as usize..to as usize {
        if multi {
            if d.gbuf_mc_flop {
                d.gbuf_pixel_reg = d.gbuf_reg >> 6;
            }
        } else {
            d.gbuf_pixel_reg = if d.gbuf_reg & 0x80 != 0 { hi } else { 0 };
        }
        let px = d.gbuf_pixel_reg;
        d.gbuf_reg <<= 1;
        d.gbuf_mc_flop = !d.gbuf_mc_flop;
        d.render_buffer[i] = codes[px as usize];
        d.pri_buffer[i] = px & 0x02;
    }
}

#[inline]
fn draw_graphics(d: &mut Draw, i: u8) {
    if i == d.xscroll_pipe {
        d.vbuf_reg = d.vbuf_pipe1;
        d.cbuf_reg = d.cbuf_pipe1;
        d.gbuf_reg = d.gbuf_pipe1;
        d.gbuf_mc_flop = true;
    }
    let mc_char = d.vmode11_pipe & 0x08 != 0 || d.cbuf_reg & 0x08 != 0;
    if d.vmode16_pipe2 != 0 {
        if mc_char {
            if d.gbuf_mc_flop {
                d.gbuf_pixel_reg = d.gbuf_reg >> 6;
            }
        } else {
            d.gbuf_pixel_reg = if d.gbuf_reg & 0x80 != 0 { 3 } else { 0 };
        }
    } else if mc_char {
        d.gbuf_pixel_reg = if d.gbuf_reg & 0x80 != 0 { 2 } else { 0 };
    } else {
        d.gbuf_pixel_reg = if d.gbuf_reg & 0x80 != 0 { 3 } else { 0 };
    }
    let px = d.gbuf_pixel_reg;
    d.gbuf_reg <<= 1;
    d.gbuf_mc_flop = !d.gbuf_mc_flop;

    let vmode = d.vmode11_pipe | d.vmode16_pipe;
    let cc = match COLORS[(vmode | px) as usize] {
        COL_NONE => 0,
        COL_VBUF_L => d.vbuf_reg & 0x0F,
        COL_VBUF_H => d.vbuf_reg >> 4,
        COL_CBUF => d.cbuf_reg,
        COL_CBUF_MC => d.cbuf_reg & 0x07,
        COL_D02X_EXT => COL_D021 + (d.vbuf_reg >> 6),
        c => c,
    };
    d.render_buffer[i as usize] = cc;
    d.pri_buffer[i as usize] = px & 0x02;
}

fn draw_sprites8(v: &mut VicState, pc: &Cyc) {
    let hmos = v.chip.hmos();
    let d = &mut v.d;
    let xpos = pc.xpos;
    let (dma0, dma2, dma2_num) = match pc.phi1 {
        Phi1::SprPtr(n) => (1u8 << n, 0u8, 8usize),
        Phi1::SprDma1(n) => (0, 1u8 << n, n as usize),
        _ => (0, 0, 8),
    };
    // A sprite can produce pixels only if already active or if it can start
    // (pending, with its X in this cycle); otherwise just update the state
    let spr_disp = pc.flags & SPR_DISP != 0;
    let pending_after = if spr_disp { v.sprite_display_bits } else { d.sprite_pending_bits };
    let waiting = d.sprite_pending_bits | pending_after;
    let mut candidates = 0u8;
    if waiting != 0 {
        for s in 0..8 {
            if xpos & 0x1F8 == d.sprite_x_pipe[s] & 0x1F8 {
                candidates |= 1 << s;
            }
        }
    }
    if d.sprite_active_bits != 0 || candidates & waiting != 0 {
        for i in 0..8u8 {
            match i {
                2 => d.sprite_active_bits &= !dma2,
                3 => d.sprite_halt_bits |= dma0,
                4 => {
                    if spr_disp {
                        d.sprite_pending_bits = v.sprite_display_bits;
                    }
                    if dma2_num < 8 {
                        d.sbuf_reg[dma2_num] = v.sprites[dma2_num].data;
                    }
                }
                6 => {
                    if hmos {
                        update_sprite_mc_bits_hmos(d, v.regs[0x1C]);
                    }
                    d.sprite_pri_bits = v.regs[0x1B];
                    d.sprite_expx_bits = v.regs[0x1D];
                }
                7 => {
                    if !hmos {
                        update_sprite_mc_bits(d, v.regs[0x1C]);
                    }
                    d.sprite_halt_bits &= !dma2;
                }
                _ => {}
            }
            trigger_sprites(d, xpos + i as u16, candidates);
            draw_sprites(d, i as usize, hmos, &mut v.sprite_sprite_collisions,
                         &mut v.sprite_background_collisions);
        }
    } else {
        d.sprite_halt_bits = (d.sprite_halt_bits | dma0) & !dma2;
        d.sprite_pending_bits = pending_after;
        if dma2_num < 8 {
            d.sbuf_reg[dma2_num] = v.sprites[dma2_num].data;
        }
        if hmos {
            update_sprite_mc_bits_hmos(d, v.regs[0x1C]);
        } else {
            update_sprite_mc_bits(d, v.regs[0x1C]);
        }
        d.sprite_pri_bits = v.regs[0x1B];
        d.sprite_expx_bits = v.regs[0x1D];
    }
    for s in 0..8 {
        d.sprite_x_pipe[s] = v.sprites[s].x;
    }
}

/// New sprite multicolor bits ($D01C), NMOS chips (at pixel 7): a sprite
/// whose mode changed restarts its pair of bits.
#[inline]
fn update_sprite_mc_bits(d: &mut Draw, next: u8) {
    d.sbuf_mc_flops &= !(next ^ d.sprite_mc_bits);
    d.sprite_mc_bits = next;
}

/// HMOS chips (at pixel 6): the flop of a changed sprite toggles, unless
/// between the two halves of an X-expanded pixel (VICE's
/// update_sprite_mc_bits_8565 with SPRITESPLITPATCH).
#[inline]
fn update_sprite_mc_bits_hmos(d: &mut Draw, next: u8) {
    let toggled = (next ^ d.sprite_mc_bits) & !d.sbuf_expx_flops;
    d.sbuf_mc_flops ^= toggled;
    d.sbuf_mc_flops |= toggled & !next;
    d.sprite_mc_bits = next;
}

#[inline]
fn trigger_sprites(d: &mut Draw, xpos: u16, candidates: u8) {
    if candidates == 0 || d.sprite_pending_bits == 0 {
        return;
    }
    for s in 0..8 {
        let m = 1u8 << s;
        if candidates & m != 0 && d.sprite_pending_bits & m != 0
            && d.sprite_active_bits & m == 0 && d.sprite_halt_bits & m == 0
            && xpos == d.sprite_x_pipe[s]
        {
            d.sbuf_expx_flops |= m;
            d.sbuf_mc_flops |= m;
            d.sprite_active_bits |= m;
        }
    }
}

#[inline]
fn draw_sprites(d: &mut Draw, i: usize, hmos: bool, ss: &mut u8, sb: &mut u8) {
    if d.sprite_active_bits == 0 {
        return;
    }
    let mut active_sprite = 0usize;
    let mut collision = 0u8;
    for s in (0..8).rev() {
        let m = 1u8 << s;
        if d.sprite_active_bits & m == 0 {
            continue;
        }
        if d.sbuf_reg[s] != 0 || d.sbuf_pixel_reg[s] != 0 {
            if d.sprite_halt_bits & m == 0 {
                if d.sbuf_expx_flops & m != 0 {
                    if d.sprite_mc_bits & m != 0 {
                        if d.sbuf_mc_flops & m != 0 {
                            d.sbuf_pixel_reg[s] = ((d.sbuf_reg[s] >> 22) & 0x03) as u8;
                        }
                        d.sbuf_mc_flops ^= m;
                    } else if !hmos || d.sbuf_mc_flops & m != 0 {
                        // NMOS: the hires pixel is always read; HMOS: not
                        // right after a switch from multicolor
                        d.sbuf_pixel_reg[s] = (((d.sbuf_reg[s] >> 23) & 0x01) << 1) as u8;
                    } else {
                        d.sbuf_mc_flops |= m;
                    }
                }
                if d.sbuf_expx_flops & m != 0 {
                    d.sbuf_reg[s] <<= 1;
                }
                if d.sprite_expx_bits & m != 0 {
                    d.sbuf_expx_flops ^= m;
                } else {
                    d.sbuf_expx_flops |= m;
                }
            }
            if d.sbuf_pixel_reg[s] != 0 {
                active_sprite = s;
                collision |= m;
            }
        } else {
            d.sprite_active_bits &= !m;
        }
    }
    if collision != 0 {
        let pri = d.pri_buffer[i];
        if !(pri != 0 && d.sprite_pri_bits & (1 << active_sprite) != 0) {
            match d.sbuf_pixel_reg[active_sprite] {
                1 => d.render_buffer[i] = COL_D025,
                2 => d.render_buffer[i] = COL_D027 + active_sprite as u8,
                3 => d.render_buffer[i] = COL_D026,
                _ => {}
            }
        }
        if pri != 0 {
            *sb |= collision;
        }
    }
    if collision & collision.wrapping_sub(1) != 0 {
        *ss |= collision;
    }
}

fn draw_border8(v: &mut VicState) {
    let d = &mut v.d;
    if !(d.border_state || v.main_border) {
        return;
    }
    if d.border_state && v.main_border {
        d.render_buffer = [COL_D020; 8];
        return;
    }
    if v.regs[0x16] & 0x08 != 0 {
        if d.border_state {
            d.render_buffer = [COL_D020; 8];
        }
        d.border_state = v.main_border;
    } else {
        if d.border_state {
            d.render_buffer[..7].fill(COL_D020);
        }
        d.border_state = v.main_border;
        if d.border_state {
            d.render_buffer[7] = COL_D020;
        }
    }
}

/// Resolves colors (one cycle after drawing) and writes the 8 pixels of the
/// previous drawing to the framebuffer. The NMOS chips resolve each pixel
/// while drawing the one before, so the first pixel after a color register
/// write still has the old color; the HMOS chips resolve it in place, and
/// the first pixel of the register being written is light grey (the grey
/// dot).
fn draw_colors8(v: &mut VicState, fb: &mut [u32]) {
    let d = &mut v.d;
    if d.last_color_reg != 0xFF {
        d.cregs[d.last_color_reg as usize] = d.last_color_value;
    }
    let mut out = [0u8; 8];
    if v.chip.hmos() {
        for i in 0..8 {
            let c = d.pixel_buffer[i];
            out[i] = if i == 0 && c == d.last_color_reg { 0x0F } else { d.cregs[c as usize] };
            d.pixel_buffer[i] = d.render_buffer[i];
        }
    } else {
        for i in 0..8 {
            let li = (i + 1) & 7;
            d.pixel_buffer[li] = d.cregs[d.pixel_buffer[li] as usize];
            out[i] = d.pixel_buffer[i];
            d.pixel_buffer[i] = d.render_buffer[i];
        }
    }
    d.last_color_reg = v.last_color_reg;
    d.last_color_value = v.last_color_value;
    v.last_color_reg = 0xFF;

    let Some(row) = v.standard.fb_row(v.dbuf_line) else { return };
    // Position in VICE's line buffer (reset at cycle 2), mapped to the VIC X
    // of the pixels: X = 8·(drawing cycle) - 120 + i
    let n = v.standard.cycles_per_line() as i32;
    let offs = 8 * ((v.cycle as i32 + n - 2) % n);
    let row = row * WIDTH;
    let fx = offs - 95;
    let palette = v.chip.palette();
    if fx >= 0 && fx as usize + 8 <= WIDTH {
        let dst = &mut fb[row + fx as usize..row + fx as usize + 8];
        for (p, &c) in dst.iter_mut().zip(out.iter()) {
            *p = palette[(c & 0x0F) as usize];
        }
    } else {
        for (i, &c) in out.iter().enumerate() {
            let x = fx + i as i32;
            if (0..WIDTH as i32).contains(&x) {
                fb[row + x as usize] = palette[(c & 0x0F) as usize];
            }
        }
    }
}

// ── Saveable state ───────────────────────────────────────────────────────────

use crate::snapshot::impl_state;

impl_state!(Sprite { x, data, mc, mcbase, pointer, exp_flop });

impl_state!(Draw {
    gbuf_pipe0, cbuf_pipe0, vbuf_pipe0, gbuf_pipe1, cbuf_pipe1, vbuf_pipe1,
    xscroll_pipe, vmode11_pipe, vmode16_pipe, vmode16_pipe2, gbuf_reg,
    gbuf_mc_flop, gbuf_pixel_reg, cbuf_reg, vbuf_reg, dmli, sprite_x_pipe,
    sprite_pri_bits, sprite_mc_bits, sprite_expx_bits, sprite_pending_bits,
    sprite_active_bits, sprite_halt_bits, sbuf_reg, sbuf_pixel_reg,
    sbuf_expx_flops, sbuf_mc_flops, border_state, render_buffer, pri_buffer,
    pixel_buffer, cregs, last_color_reg, last_color_value,
});

impl_state!(VicState {
    chip, standard, regs, raster_line, cycle, raster_irq_triggered, start_of_frame,
    allow_bad_lines, bad_line, idle_state, vc, vcbase, rc, vmli, vbuf, cbuf,
    gbuf, refresh_counter, prefetch_cycles, last_bus_phi2, last_read_phi1,
    reg11_delay, main_border, vborder, set_vborder, sprites, sprite_dma,
    sprite_display_bits, sprite_sprite_collisions, sprite_background_collisions,
    clear_collisions, last_color_reg, last_color_value, dbuf_line, d,
});
