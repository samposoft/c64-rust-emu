// SPDX-License-Identifier: GPL-2.0-or-later
// Copyright (C) 2026 SampoSoft - Francesco Sampoli

//! CRT monitor emulation: model of the analog path from the VIC-II to the
//! picture tube of a Commodore 1084S-P1, with the constants and filter
//! kernels that the GPU shaders use (`frontend::gpu`, feature `gpu`).
//!
//! The chain, stage by stage:
//!
//! 1. VIC-II: every color is a luma level and a chroma vector (Pepto's
//!    "colodore" model, <https://www.pepto.de/projects/colorvic/>, the one
//!    `vic::C64_PALETTE` comes from). The luma output is slow and
//!    underdamped: measured on a real C64 it takes about 1.5 pixels to rise
//!    and overshoots by about 12% (<https://jamontoads.net/p/lumachroma.html>).
//!    The chroma phase differs between even and odd raster lines, 11-16°
//!    on the 6569R5 (Tobias' measurements in VICE's `vicii-color.c`).
//! 2. Cable: separate luma and chroma (the 1084S "LCA" inputs, as S-Video),
//!    or composite, their sum. The signal is sampled at 4 times the PAL
//!    subcarrier (17.734475 MHz, the C64 crystal): 9 samples every 4
//!    pixels. A line is 283.5 subcarrier cycles, so the carrier phase flips
//!    from line to line and repeats every frame (no dot crawl).
//! 3. PAL decoder (TDA4510 in the 1084S-P1): in composite mode a trap at
//!    4.43 MHz takes the chroma out of the luma; the chroma is demodulated
//!    and low-passed (about 1.3 MHz), and the 64 µs delay line averages
//!    it with the previous line, which cancels the odd-line phase error.
//! 4. Picture tube: Pepto's YUV to RGB and gamma (2.8 for the tube, 2.2
//!    for the host display) give exactly the colodore colors on flat
//!    areas; each line is a gaussian beam that widens with brightness, and
//!    the light goes through a slot mask of 0.42 mm triads.
//!
//! Figures from the service manuals and measurements are marked with their
//! source; the others are estimates, chosen by eye (beam width, mask slots,
//! glass halation, trap Q).

use std::f64::consts::PI;

use crate::vic::{C64_PALETTE, WIDTH};

/// PAL color subcarrier, Hz.
pub const FSC: f64 = 4_433_618.75;
/// Sampling rate of the video signal: 4 samples per subcarrier cycle.
pub const SAMPLE_RATE: f64 = 4.0 * FSC;
/// Pixel clock of the PAL C64: the crystal (4 × FSC) × 4/9.
pub const DOT_CLOCK: f64 = SAMPLE_RATE * 4.0 / 9.0;
/// Signal samples per framebuffer line: 9 every 4 pixels.
pub const SAMPLES: usize = (WIDTH * 9 + 3) / 4;
/// Width over height of a C64 pixel on a PAL screen: the square-pixel rate
/// of PAL (7.375 MHz for 384 pixels in 52 µs) over the dot clock.
pub const PIXEL_ASPECT: f64 = 7_375_000.0 / DOT_CLOCK;

/// Luma overshoot of the VIC-II on a black to white step (measured, Jam on
/// Toads: 1.05 V white, peak about 1.14 V over 0.32 V black).
const VIC_LUMA_OVERSHOOT: f64 = 0.12;
/// Luma rise time of the VIC-II, 10-90%, in pixels (same measurement:
/// "almost 2 pixels" to white; 10-90% about 1.5).
const VIC_LUMA_RISE_PX: f64 = 1.5;
/// Chroma phase difference between odd and even raster lines, degrees
/// (mean of Tobias' measurements on the 6569R5: 11-16°).
const ODD_LINE_PHASE: f64 = 13.0;

/// Luma levels (0-32) of the 16 colors in the colodore model, later
/// VIC-II revisions (6569R3 on, 8565).
const LUMA: [u8; 16] = [0, 32, 10, 20, 12, 16, 8, 24, 12, 8, 16, 10, 15, 24, 15, 20];
/// Hue of the colors: angle 11.25° + a × 22.5°; None without chroma.
const HUE: [Option<u8>; 16] = [
    None, None, Some(4), Some(12), Some(2), Some(10), Some(15), Some(7),
    Some(5), Some(6), Some(4), None, None, Some(10), Some(15), None,
];
/// Colodore defaults: brightness 50, contrast 100, saturation 50, with
/// Pepto's "screen" term of 1/5.
const SCREEN: f64 = 0.2;
const SATURATION: f64 = 50.0 * (1.0 - SCREEN);
pub const CONTRAST: f64 = 1.0 + SCREEN;
/// Gamma of the picture tube and of the host display, as in colodore.
pub const TUBE_GAMMA: f64 = 2.8;
pub const DISPLAY_GAMMA: f64 = 2.2;

// ── Settings ─────────────────────────────────────────────────────────────────

/// How the C64 is connected to the monitor.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Input {
    /// Separate luma and chroma (the 1084S rear LCA sockets, as S-Video).
    LumaChroma,
    /// Composite video (the front CVBS socket, or SCART).
    Composite,
    /// The antenna socket of a TV, from the C64's RF modulator.
    Rf,
}

impl Input {
    pub fn name(self) -> &'static str {
        match self {
            Input::LumaChroma => "luma/chroma",
            Input::Composite => "composite",
            Input::Rf => "RF (UHF channel 36)",
        }
    }

    pub fn parse(s: &str) -> Option<Input> {
        match s {
            "lc" => Some(Input::LumaChroma),
            "composite" => Some(Input::Composite),
            "rf" => Some(Input::Rf),
            _ => None,
        }
    }
}

/// Emulated monitors.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Model {
    /// Commodore 1084S-P1 (Philips chassis, PAL, 1990).
    C1084SP1,
    /// Commodore 1084S-D1 (Daewoo chassis, PAL).
    C1084SD1,
    /// Commodore 1901 (Thomson chassis, PAL, 1986).
    C1901,
    /// Philips 15CE1510 color TV (CP90 chassis, 1988).
    PhilipsCp90,
}

impl Model {
    /// Names accepted by `parse`, for help and error messages.
    pub const NAMES: &'static str = "1084s-p1 (or 1084s), 1084s-d1, 1901, cp90 (or tv)";

    pub fn parse(s: &str) -> Option<Model> {
        match s.to_ascii_lowercase().as_str() {
            "1084s" | "1084s-p1" | "1084sp1" | "1084" => Some(Model::C1084SP1),
            "1084s-d1" | "1084sd1" => Some(Model::C1084SD1),
            "1901" => Some(Model::C1901),
            "cp90" | "tv" | "15ce1510" => Some(Model::PhilipsCp90),
            _ => None,
        }
    }

    /// Short name, as `parse` takes it.
    pub fn name(self) -> &'static str {
        match self {
            Model::C1084SP1 => "1084S-P1",
            Model::C1084SD1 => "1084S-D1",
            Model::C1901 => "1901",
            Model::PhilipsCp90 => "CP90",
        }
    }

    /// Full name, with the brand.
    pub fn full_name(self) -> &'static str {
        match self {
            Model::C1084SP1 => "Commodore 1084S-P1",
            Model::C1084SD1 => "Commodore 1084S-D1",
            Model::C1901 => "Commodore 1901",
            Model::PhilipsCp90 => "Philips 15CE1510 TV (CP90 chassis)",
        }
    }

    pub fn monitor(self) -> &'static Monitor {
        match self {
            Model::C1084SP1 => &C1084S_P1,
            Model::C1084SD1 => &C1084S_D1,
            Model::C1901 => &C1901,
            Model::PhilipsCp90 => &PHILIPS_CP90,
        }
    }
}

/// CRT emulation settings: monitor and connection.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Crt {
    pub model: Model,
    pub input: Input,
}

impl Crt {
    /// The model with its first input: luma/chroma for the monitors, RF
    /// for the TV.
    pub fn new(model: Model) -> Crt {
        Crt { model, input: model.monitor().inputs[0] }
    }

    /// The settings `crt` (None: off) changed by `arg`: `off`, a model
    /// (keeping the input if the model has it, otherwise its default one) or
    /// an input (`lc`, `composite`, `rf`; turning the emulation on with the
    /// 1084S-P1, or the TV for RF).
    pub fn apply(crt: Option<Crt>, arg: &str) -> Result<Option<Crt>, String> {
        if arg == "off" {
            return Ok(None);
        }
        if let Some(input) = Input::parse(arg) {
            let default = if input == Input::Rf { Model::PhilipsCp90 } else { Model::C1084SP1 };
            return Ok(Some(Crt { input, ..crt.unwrap_or(Crt::new(default)) }));
        }
        let model = Model::parse(arg)
            .ok_or_else(|| format!("unknown monitor or input '{arg}': monitors {}; inputs lc, composite, rf", Model::NAMES))?;
        let input = crt.map(|c| c.input).filter(|i| model.monitor().inputs.contains(i));
        Ok(Some(Crt { model, input: input.unwrap_or(Crt::new(model).input) }))
    }

    /// Error if the model does not have the input.
    pub fn check(&self) -> Result<(), String> {
        let inputs = self.model.monitor().inputs;
        if inputs.contains(&self.input) {
            return Ok(());
        }
        let names: Vec<&str> = inputs.iter().map(|i| i.name()).collect();
        Err(format!("the {} has no {} input (it has: {})", self.model.full_name(), self.input.name(), names.join(", ")))
    }

    pub fn describe(&self) -> String {
        format!("{}, {} input", self.model.full_name(), self.input.name())
    }
}

// ── Monitors ─────────────────────────────────────────────────────────────────

/// Physical data of a monitor.
#[derive(Debug)]
pub struct Monitor {
    /// Picture on the screen (the active part of the PAL signal), mm.
    pub screen_width: f64,
    pub screen_height: f64,
    /// Horizontal pitch of the phosphor triads, mm.
    pub triad_pitch: f64,
    /// Width of a phosphor stripe, as a fraction of the triad.
    pub stripe: f64,
    /// Vertical period of the mask slots, mm, and height of the bridges
    /// between them as a fraction of it.
    pub slot_pitch: f64,
    pub bridge: f64,
    /// Standard deviation of the beam, in line pitches: at black and at
    /// full brightness.
    pub beam_sigma: (f64, f64),
    /// Halation in the glass: share of the light and spread (sigma), mm.
    pub glow: (f64, f64),
    /// Inputs of the set; the first is the default.
    pub inputs: &'static [Input],
    /// Response of the IF filter of the TV tuner (RF input): attenuation,
    /// dB, at IF frequencies, MHz, with the vision carrier at 38.9 MHz.
    pub if_filter: Option<&'static [(f64, f64)]>,
    /// Bandwidth (-3 dB) of the luma amplifier, Hz, with separate luma and
    /// with a composite signal (composite or RF input).
    pub luma_bandwidth: [f64; 2],
    /// Luma peaking: time constant, s, of a high shelf of +6 dB (zero at
    /// 1/(2πτ), pole at twice that).
    pub luma_peaking: Option<f64>,
    /// Luma trap at 4.43 MHz with a composite signal (without one the
    /// subcarrier stays in the luma).
    pub luma_trap: Option<Trap>,
    /// Q of the chroma band-pass tuned to 4.43 MHz, which takes the chroma
    /// out of a composite signal.
    pub chroma_q: f64,
    /// The chroma input goes through that band-pass also with separate
    /// luma and chroma.
    pub chroma_bandpass: bool,
    /// Bandwidth (-3 dB) of the low-pass after the chroma demodulators, Hz.
    pub chroma_bandwidth: f64,
    /// White point, K; None: D65, the white of the colodore palette.
    pub white_point: Option<f64>,
}

/// Luma trap: a second-order notch at the subcarrier.
#[derive(Debug, Clone, Copy)]
pub struct Trap {
    pub q: f64,
    /// Gain at the subcarrier: 0 for a full notch.
    pub depth: f64,
}

/// Commodore 1084S-P1 (service manual, Philips chassis): tube M34EAQ10X,
/// "slot triplet pitch 0.42 mm", 14", 90°; video bandwidth 8 MHz; TDA4510
/// PAL decoder with a 64 µs delay line and an adjustable 4.43 MHz
/// "chrominance suppression" trap, used in composite mode only.
pub static C1084S_P1: Monitor = Monitor {
    // M34: 34 cm visible diagonal, 4:3
    screen_width: 272.0,
    screen_height: 204.0,
    triad_pitch: 0.42,
    // Estimates: stripes with black guard bands, slots of the usual
    // shape for TV slot masks
    stripe: 0.25,
    slot_pitch: 0.8,
    bridge: 0.12,
    beam_sigma: (0.26, 0.42),
    glow: (0.05, 2.0),
    inputs: &[Input::LumaChroma, Input::Composite],
    if_filter: None,
    luma_bandwidth: [8_000_000.0, 8_000_000.0],
    luma_peaking: None,
    luma_trap: Some(Trap { q: 2.0, depth: 0.0 }),
    chroma_q: 2.0,
    chroma_bandpass: false,
    // U/V bandwidth of PAL
    chroma_bandwidth: 1_300_000.0,
    white_point: None,
};

/// Commodore 1084S-D1 (service manual, Daewoo chassis): 13" visible tube,
/// 90°, in-line guns, "dot pitch 0.41 or 0.42 mm", "black strips screen";
/// display size 260 × 186 mm; video bandwidth 5.2 MHz with separate luma
/// and chroma, 4.4 MHz composite; TDA4510 PAL decoder (as the P1) with the
/// TDA3507. The estimates are those of the P1: the manual gives nothing
/// about the beam, the slots or the trap.
pub static C1084S_D1: Monitor = Monitor {
    screen_width: 260.0,
    screen_height: 186.0,
    triad_pitch: 0.41,
    stripe: 0.25,
    slot_pitch: 0.8,
    bridge: 0.12,
    beam_sigma: (0.26, 0.42),
    glow: (0.05, 2.0),
    inputs: &[Input::LumaChroma, Input::Composite],
    if_filter: None,
    luma_bandwidth: [5_200_000.0, 4_400_000.0],
    luma_peaking: None,
    luma_trap: Some(Trap { q: 2.0, depth: 0.0 }),
    chroma_q: 2.0,
    chroma_bandpass: false,
    chroma_bandwidth: 1_300_000.0,
    white_point: None,
};

/// Commodore 1901 (service manual and schematics, Thomson chassis): 14"
/// tube, 90°, in-line guns, M34JGT60 with 0.43 mm pitch (a version with
/// the 0.39 mm M34JAW13 also existed); white point 7500 K. Luma: no trap;
/// the emitter follower before the TDA3506 drives 560 Ω ∥ 470 pF into
/// 560 Ω, a fixed peaking of +6 dB above about 1.2 MHz. Chroma: LC band-pass
/// (10 µH, damped by 1 kΩ: Q about 3.6 when tuned to 4.43 MHz) into the
/// AN5620X PAL decoder with a 64 µs delay line. Composite needs an internal
/// jumper; without a trap the subcarrier stays in the luma as a fine dot
/// pattern. The manual gives neither the picture size nor the bandwidth:
/// those are the P1's, as the estimates.
pub static C1901: Monitor = Monitor {
    screen_width: 272.0,
    screen_height: 204.0,
    triad_pitch: 0.43,
    stripe: 0.25,
    slot_pitch: 0.8,
    bridge: 0.12,
    beam_sigma: (0.26, 0.42),
    glow: (0.05, 2.0),
    inputs: &[Input::LumaChroma, Input::Composite],
    if_filter: None,
    luma_bandwidth: [8_000_000.0, 8_000_000.0],
    luma_peaking: Some(560.0 * 470e-12),
    luma_trap: None,
    chroma_q: 3.6,
    chroma_bandpass: true,
    chroma_bandwidth: 1_300_000.0,
    white_point: Some(7500.0),
};

/// Philips 15CE1510 color TV (CP90 chassis service manual; made by Philips
/// Italy, 1987-90; the 15CE1210 is the same without teletext): tube
/// A36JAR40X02M or A36EAM01X16, 36 cm flat square, in-line guns, slotted
/// mask with vertical stripes of 0.52 mm at the centre, useful screen
/// 284.5 × 213.4 mm (Philips data handbook T08, 1986); tuner, SAW IF filter
/// and TDA2541 synchronous demodulator; TDA3561A PAL decoder with the
/// DL701 64 µs delay line; luma trap S5261, which "attenuates the 4.43 MHz
/// chroma approx. 6 dB" (a shallow notch: fine luma detail stays, and so
/// does some dot pattern); chroma band-pass S5259. Inputs: antenna and
/// SCART (composite; its RGB is not used by the C64). The SAW is not named
/// in the manual: the response is that of a B/G intercarrier SAW of the
/// time (EPCOS K2966M). Q of the trap and of the band-pass, bandwidth and
/// white point are not in the manual: the band-pass has the Q of the
/// TDA356x application circuit (10.7 µH ∥ 120 pF from 1 kΩ: about 3).
pub static PHILIPS_CP90: Monitor = Monitor {
    screen_width: 284.5,
    screen_height: 213.4,
    triad_pitch: 0.52,
    stripe: 0.25,
    slot_pitch: 0.8,
    bridge: 0.12,
    beam_sigma: (0.26, 0.42),
    glow: (0.05, 2.0),
    inputs: &[Input::Rf, Input::Composite],
    if_filter: Some(&SAW_BG),
    luma_bandwidth: [5_000_000.0, 5_000_000.0],
    luma_peaking: None,
    luma_trap: Some(Trap { q: 2.0, depth: 0.5 }),
    chroma_q: 3.0,
    chroma_bandpass: false,
    chroma_bandwidth: 1_300_000.0,
    white_point: None,
};

/// IF response of a PAL B/G intercarrier SAW filter, EPCOS K2966M
/// (datasheet: picture carrier 38.9 MHz on the Nyquist slope, -5.6 dB;
/// color carrier 34.47 MHz, -3.1 dB; sound shelf 33.4-32.4 MHz, about -20
/// dB; adjacent carriers 31.9 and 40.4 MHz, -58 dB; the other points read
/// from its curve).
pub static SAW_BG: [(f64, f64); 13] = [
    (30.0, -60.0), (31.9, -58.0), (32.4, -20.4), (33.4, -19.3), (34.2, -8.0),
    (34.47, -3.1), (35.0, 0.0), (38.0, 0.0), (38.5, -2.3), (38.9, -5.6),
    (39.1, -8.0), (40.2, -40.0), (45.0, -42.0),
];

impl Monitor {
    /// Width of a C64 pixel on the screen, mm: the 52 µs of active line
    /// fill the width of the picture.
    pub fn pixel_width(&self) -> f64 {
        self.screen_width / (52e-6 * DOT_CLOCK)
    }

    /// Distance between two lines, mm: 288 lines (half of the 576 active
    /// lines of PAL) fill the height of the picture.
    pub fn line_pitch(&self) -> f64 {
        self.screen_height / 288.0
    }
}

// ── Colors ───────────────────────────────────────────────────────────────────

/// Luma and chroma of the 16 colors, in Pepto's units (luma 0-256), on
/// even or odd raster lines. The two lines have the chroma rotated by
/// ∓ODD_LINE_PHASE/2, with the amplitude raised so that their average (the
/// PAL delay line) is exactly the colodore vector.
pub fn palette_yuv(odd_line: bool) -> [[f64; 3]; 16] {
    let half = (ODD_LINE_PHASE / 2.0).to_radians();
    let turn = if odd_line { half } else { -half };
    std::array::from_fn(|i| {
        let y = 8.0 * LUMA[i] as f64;
        match HUE[i] {
            None => [y, 0.0, 0.0],
            Some(a) => {
                let angle = (11.25 + a as f64 * 22.5).to_radians() + turn;
                let s = SATURATION / half.cos();
                [y, angle.cos() * s, angle.sin() * s]
            }
        }
    })
}

/// Pepto's YUV → R'G'B' (0-255, clamped), with the colodore contrast.
pub fn yuv_to_rgb(y: f64, u: f64, v: f64) -> [f64; 3] {
    let (y, u, v) = (y * CONTRAST, u * CONTRAST, v * CONTRAST);
    [y + 1.140 * v, y - 0.396 * u - 0.581 * v, y + 2.029 * u].map(|c| c.clamp(0.0, 255.0))
}

/// R'G'B' value (0-255) → light emitted by the tube (0-1).
pub fn to_light(c: f64) -> f64 {
    (c / 255.0).powf(TUBE_GAMMA)
}

/// Light (0-1) → value for the host display (0-255).
pub fn to_display(l: f64) -> f64 {
    255.0 * l.max(0.0).powf(1.0 / DISPLAY_GAMMA)
}

/// Gains of the three guns (linear light) that make the white of the
/// monitor on a D65 display, the brightest at 1: its white point on the CIE
/// daylight locus, to sRGB.
pub fn white_balance(m: &Monitor) -> [f64; 3] {
    let Some(t) = m.white_point else { return [1.0; 3] };
    let x = if t <= 7000.0 {
        -4.6070e9 / t.powi(3) + 2.9678e6 / t.powi(2) + 0.09911e3 / t + 0.244063
    } else {
        -2.0064e9 / t.powi(3) + 1.9018e6 / t.powi(2) + 0.24748e3 / t + 0.237040
    };
    let y = -3.0 * x * x + 2.870 * x - 0.275;
    let (cx, cy, cz) = (x / y, 1.0, (1.0 - x - y) / y);
    let rgb = [
        3.2406 * cx - 1.5372 * cy - 0.4986 * cz,
        -0.9689 * cx + 1.8758 * cy + 0.0415 * cz,
        0.0557 * cx - 0.2040 * cy + 1.0570 * cz,
    ];
    let max = rgb.iter().cloned().fold(0.0, f64::max);
    rgb.map(|c| c / max)
}

/// Palette index of an ARGB pixel: the VIC color it is, or the nearest
/// one (frames drawn by the VIC only have palette colors).
pub fn palette_index(argb: u32) -> usize {
    let rgb = |c: u32| [(c >> 16 & 0xFF) as i32, (c >> 8 & 0xFF) as i32, (c & 0xFF) as i32];
    let p = rgb(argb);
    (0..16)
        .min_by_key(|&i| rgb(C64_PALETTE[i]).iter().zip(&p).map(|(a, b)| (a - b) * (a - b)).sum::<i32>())
        .unwrap_or(0)
}

/// Share of the slot mask shown on the host display, 0-1. The mask of a
/// real tube is fully dark between the phosphor stripes, with the stripes
/// much brighter than the average; a display that cannot go beyond white
/// would clip them and dim the picture, so the mask is only partly
/// applied (a choice for the display, not a property of the monitor).
pub const MASK_STRENGTH: f64 = 0.5;
/// On the brightest colors the mask is made shallower, but not below the
/// depth that costs white this share of its light: the mask still shows
/// on white, whatever the size of the triads (white comes out as about 250
/// instead of 255).
pub const MASK_WHITE_LOSS: f64 = 0.02;

// ── VIC-II luma output ───────────────────────────────────────────────────────

/// Second-order low-pass system, time in pixels.
struct SecondOrder {
    zeta: f64,
    wn: f64,
}

impl SecondOrder {
    /// The VIC-II luma output, from the measured overshoot and rise time.
    fn vic_luma() -> SecondOrder {
        let ln = VIC_LUMA_OVERSHOOT.ln();
        let zeta = -ln / (PI * PI + ln * ln).sqrt();
        let unit = SecondOrder { zeta, wn: 1.0 };
        let rise = unit.time_to(0.9) - unit.time_to(0.1);
        SecondOrder { zeta, wn: rise / VIC_LUMA_RISE_PX }
    }

    /// Response to a unit step at t = 0.
    fn step(&self, t: f64) -> f64 {
        if t <= 0.0 {
            return 0.0;
        }
        let wd = (1.0 - self.zeta * self.zeta).sqrt();
        let x = self.wn * t;
        1.0 - (-self.zeta * x).exp() * ((wd * x).cos() + self.zeta / wd * (wd * x).sin())
    }

    /// First time the step response reaches `level`.
    fn time_to(&self, level: f64) -> f64 {
        let mut t = 0.0;
        let dt = 1e-3 / self.wn;
        while self.step(t + dt) < level {
            t += dt;
        }
        let (mut lo, mut hi) = (t, t + dt);
        for _ in 0..60 {
            let mid = (lo + hi) / 2.0;
            if self.step(mid) < level { lo = mid } else { hi = mid }
        }
        lo
    }
}

/// Phases of the samples within the pixels: 9 samples every 4 pixels.
pub const LUMA_PHASES: usize = 9;
/// Pixels that contribute to a luma sample: from LUMA_FIRST (the past) to
/// LUMA_FIRST + LUMA_TAPS - 1 (the future, through the delay compensation).
pub const LUMA_TAPS: usize = 12;
pub const LUMA_FIRST: i32 = -9;

/// Weights of the pixels around a sample in the luma output of the VIC-II,
/// for each of the 9 sample phases. Sample n lies in pixel
/// floor((n + 0.5) × 4/9) and its phase is n mod 9; weight j multiplies
/// pixel p + LUMA_FIRST + j. The response is shifted so that a step
/// reaches half height at the pixel boundary (the monitor's luma delay line
/// aligns luma and chroma).
pub fn luma_kernels() -> [[f32; LUMA_TAPS]; LUMA_PHASES] {
    let r = SecondOrder::vic_luma();
    let t50 = r.time_to(0.5);
    let s = |x: f64| r.step(x + t50);
    std::array::from_fn(|phase| {
        let pos = (phase as f64 + 0.5) * 4.0 / 9.0;
        let tau = pos - pos.floor();
        let w: [f64; LUMA_TAPS] = std::array::from_fn(|i| {
            let j = (LUMA_FIRST + i as i32) as f64;
            s(tau - j) - s(tau - j - 1.0)
        });
        let sum: f64 = w.iter().sum();
        w.map(|x| (x / sum) as f32)
    })
}

// ── Monitor filters ──────────────────────────────────────────────────────────

/// Taps on each side of the filters after the demodulation, at SAMPLE_RATE.
pub const FIR_HALF: usize = 24;
pub const FIR_LEN: usize = 2 * FIR_HALF + 1;
/// Taps on each side of the filters on the composite signal (C64 modulator,
/// TV IF, luma trap and chroma band-pass of the set, combined).
pub const SEP_HALF: usize = 48;
pub const SEP_LEN: usize = 2 * SEP_HALF + 1;

/// Symmetric FIR (zero phase) of 2·`half` + 1 taps whose response is the
/// real part of `resp(f)` e^(j2πfτ), `f` in Hz: with a magnitude, zero phase;
/// with a complex response and its low-frequency delay `tau`, the response
/// with that delay taken away. Unity gain at DC.
fn fir_from_response(half: usize, resp: impl Fn(f64) -> (f64, f64), tau: f64) -> Vec<f64> {
    const M: usize = 4096;
    let taps = |k: i64| -> f64 {
        let sum: f64 = (0..M)
            .map(|i| {
                let f = (i as f64 + 0.5) * SAMPLE_RATE / 2.0 / M as f64;
                let (re, im) = resp(f);
                let w = 2.0 * PI * f * (k as f64 / SAMPLE_RATE + tau);
                re * w.cos() - im * w.sin()
            })
            .sum();
        // Tukey window: flat, tapered over the last quarter, where the
        // response has died out
        let x = k.unsigned_abs() as f64 / (half + 1) as f64;
        let window = if x < 0.75 { 1.0 } else { 0.5 * (1.0 + (PI * (x - 0.75) / 0.25).cos()) };
        sum / M as f64 * window
    };
    let h: Vec<f64> = (-(half as i64)..=half as i64).map(taps).collect();
    let dc: f64 = h.iter().sum();
    h.into_iter().map(|x| x / dc).collect()
}

/// Symmetric FIR (zero phase) of FIR_LEN taps with magnitude `mag(f)`.
fn zero_phase_fir(mag: impl Fn(f64) -> f64) -> [f32; FIR_LEN] {
    let h = fir_from_response(FIR_HALF, |f| (mag(f), 0.0), 0.0);
    std::array::from_fn(|i| h[i] as f32)
}

/// Luma amplifier of the monitor: second-order Butterworth magnitude at its
/// bandwidth for the input.
pub fn luma_fir(m: &Monitor, input: Input) -> [f32; FIR_LEN] {
    let fc = m.luma_bandwidth[(input != Input::LumaChroma) as usize];
    zero_phase_fir(|f| 1.0 / (1.0 + (f / fc).powi(4)).sqrt())
}

/// Low-pass of the demodulated chroma: second-order Butterworth magnitude
/// at the monitor's chroma bandwidth. When the separate chroma input goes
/// through the tuned band-pass its effect on the demodulated chroma is here
/// too (with a composite signal the band-pass is done on the signal): a
/// circuit of Q tuned to FSC passes FSC ± Δ with 1 / sqrt(1 + (2QΔ/FSC)²).
pub fn chroma_fir(m: &Monitor, input: Input) -> [f32; FIR_LEN] {
    let fc = m.chroma_bandwidth;
    let band = m.chroma_bandpass && input == Input::LumaChroma;
    let q = m.chroma_q;
    zero_phase_fir(|f| {
        let lp = 1.0 / (1.0 + (f / fc).powi(4)).sqrt();
        if band { lp / (1.0 + (2.0 * q * f / FSC).powi(2)).sqrt() } else { lp }
    })
}

/// Kernel (SEP_LEN taps around the sample) of the analog second-order
/// section (s'² + b1·s' + 1) / (s'² + a1·s' + 1), s' = s/ω0 with ω0 at the
/// subcarrier, from its complex response: gain and phase of the circuit,
/// without the warping of a bilinear transform (the subcarrier is only a
/// quarter of the sampling rate).
fn notch_section(b1: f64, a1: f64) -> Vec<f64> {
    fir_from_response(SEP_HALF, move |f| {
        let x = f / FSC;
        let (nr, ni) = (1.0 - x * x, b1 * x);
        let (dr, di) = (1.0 - x * x, a1 * x);
        let d = dr * dr + di * di;
        ((nr * dr + ni * di) / d, (ni * dr - nr * di) / d)
    }, 0.0)
}

/// Luma trap: notch of Q with gain `depth` at the subcarrier.
fn trap(t: Trap) -> Vec<f64> {
    notch_section(t.depth / t.q, 1.0 / t.q)
}

/// Chroma band-pass of Q at the subcarrier: the signal minus a full notch
/// of the same Q (for a second-order circuit the two are complementary).
fn band_pass(q: f64) -> Vec<f64> {
    let mut h = notch_section(0.0, 1.0 / q);
    for x in h.iter_mut() {
        *x = -*x;
    }
    h[SEP_HALF] += 1.0;
    h
}

/// Luma peaking of time constant `tau`: 2(1 + sτ) / (2 + sτ), unity gain at
/// DC and 2 at high frequencies, from its complex response.
fn peaking(tau: f64) -> Vec<f64> {
    fir_from_response(SEP_HALF, move |f| {
        let wt = 2.0 * PI * f * tau;
        // 2(1 + jwt) / (2 + jwt)
        let d = 4.0 + wt * wt;
        (2.0 * (2.0 + wt * wt) / d, 2.0 * wt / d)
    }, 0.0)
}

/// Video response of the TV's IF filter: the C64's modulator sends both
/// sidebands (no vestigial filter), which reach the IF at 38.9 MHz ∓ f;
/// the synchronous demodulator adds them, and the Nyquist slope makes the
/// sum flat at low frequencies.
fn if_response(points: &[(f64, f64)]) -> Vec<f64> {
    let at = |mhz: f64| -> f64 {
        let (first, last) = (points[0], points[points.len() - 1]);
        let db = if mhz <= first.0 {
            first.1
        } else if mhz >= last.0 {
            last.1
        } else {
            let i = points.windows(2).position(|w| mhz <= w[1].0).unwrap_or(0);
            let (a, b) = (points[i], points[i + 1]);
            a.1 + (b.1 - a.1) * (mhz - a.0) / (b.0 - a.0)
        };
        10f64.powf(db / 20.0)
    };
    let video = |f: f64| at(38.9 - f / 1e6) + at(38.9 + f / 1e6);
    fir_from_response(SEP_HALF, |f| (video(f), 0.0), 0.0)
}

/// Luma of the C64's composite and RF output: the luma network in the RF
/// modulator (Service Manual, modulator 251696): the emitter follower drives
/// L2 ∥ C3 220 pF in series to node X, with C4 330 pF to ground, then R8
/// 150 Ω to the output, loaded by R11 150 Ω, and R9 1 kΩ / R10 120 Ω to the
/// RF modulator. L2 is adjustable: here tuned to the subcarrier (5.8 µH
/// with C3), as it has to be to keep the chroma band clean (the drawn
/// 10 µH would put the notch at 3.4 MHz). Complex response from the nodal
/// equations; the delay at low frequencies is taken away.
fn modulator_luma() -> Vec<f64> {
    let l2 = 1.0 / ((2.0 * PI * FSC).powi(2) * 220e-12);
    // Complex arithmetic on (re, im)
    type C = (f64, f64);
    let add = |a: C, b: C| (a.0 + b.0, a.1 + b.1);
    let mul = |a: C, b: C| (a.0 * b.0 - a.1 * b.1, a.0 * b.1 + a.1 * b.0);
    let div = |a: C, b: C| {
        let d = b.0 * b.0 + b.1 * b.1;
        ((a.0 * b.0 + a.1 * b.1) / d, (a.1 * b.0 - a.0 * b.1) / d)
    };
    let g = |r: f64| (1.0 / r, 0.0);
    let resp = move |f: f64| -> C {
        let w = 2.0 * PI * f.max(1.0);
        // Tank admittance: 1/(jωL) + jωC
        let yt = add(div((1.0, 0.0), (0.0, w * l2)), (0.0, w * 220e-12));
        // Node Y (RF drive): R9 from X, R10 and R20 to ground: Y = X·k
        let ky = div(g(1e3), add(add(g(1e3), g(120.0)), g(6.8e3)));
        // Node N (output): R8 from X, R11 to ground: N = X·kn
        let kn = div(g(150.0), add(g(150.0), g(150.0)));
        // Node X: yt·(E - X) = X·(jωC4 + 1/R8 + 1/R9) - N/R8 - Y/R9
        let load = add(add((0.0, w * 330e-12), g(150.0)), g(1e3));
        let back = add(mul(kn, g(150.0)), mul(ky, g(1e3)));
        let x = div(yt, add(yt, (load.0 - back.0, load.1 - back.1)));
        mul(x, kn)
    };
    let dc = resp(1.0);
    let norm = move |f: f64| div(resp(f), dc);
    // Delay at low frequencies: from the phase at 100 kHz
    let p = norm(1e5);
    let tau = -p.1.atan2(p.0) / (2.0 * PI * 1e5);
    fir_from_response(SEP_HALF, norm, tau)
}

/// Convolution of two kernels whose tap 0 is at index `a0` and `b0`, cut to
/// SEP_LEN around SEP_HALF.
fn convolve(a: &[f64], a0: usize, b: &[f64], b0: usize) -> Vec<f64> {
    let mut out = vec![0f64; SEP_LEN];
    for (i, &x) in a.iter().enumerate() {
        for (j, &y) in b.iter().enumerate() {
            let k = i as i64 - a0 as i64 + j as i64 - b0 as i64 + SEP_HALF as i64;
            if (0..SEP_LEN as i64).contains(&k) {
                out[k as usize] += x * y;
            }
        }
    }
    out
}

/// The C64's luma for a composite signal (composite and RF output), as a
/// kernel of SEP_LEN taps around the sample.
pub fn modulator_fir() -> [f32; SEP_LEN] {
    let h = modulator_luma();
    std::array::from_fn(|i| h[i] as f32)
}

/// Luma input of the monitor, SEP_LEN taps around the sample: the IF
/// filter (RF), the trap (composite signal), the peaking.
pub fn luma_input_fir(m: &Monitor, input: Input) -> [f32; SEP_LEN] {
    let mut h = vec![0f64; SEP_LEN];
    h[SEP_HALF] = 1.0;
    if input == Input::Rf {
        if let Some(points) = m.if_filter {
            h = convolve(&h, SEP_HALF, &if_response(points), SEP_HALF);
        }
    }
    if input != Input::LumaChroma {
        if let Some(t) = m.luma_trap {
            h = convolve(&h, SEP_HALF, &trap(t), SEP_HALF);
        }
    }
    if let Some(tau) = m.luma_peaking {
        h = convolve(&h, SEP_HALF, &peaking(tau), SEP_HALF);
    }
    let dc: f64 = h.iter().sum();
    std::array::from_fn(|i| (h[i] / dc) as f32)
}

/// Chroma out of a composite signal, SEP_LEN taps around the sample: the
/// IF filter (RF) and the band-pass, with unity gain at the subcarrier (the
/// burst sets the chroma gain: ACC).
pub fn chroma_input_fir(m: &Monitor, input: Input) -> [f32; SEP_LEN] {
    let mut h = band_pass(m.chroma_q);
    if input == Input::Rf {
        if let Some(points) = m.if_filter {
            h = convolve(&h, SEP_HALF, &if_response(points), SEP_HALF);
        }
    }
    // Gain at FSC = SAMPLE_RATE/4, where the taps weigh 1, 0, -1, 0... and
    // 0, 1, 0, -1...
    let (mut re, mut im) = (0.0, 0.0);
    for (i, &x) in h.iter().enumerate() {
        let k = (i as i64 - SEP_HALF as i64).rem_euclid(4);
        re += x * [1.0, 0.0, -1.0, 0.0][k as usize];
        im -= x * [0.0, 1.0, 0.0, -1.0][k as usize];
    }
    let g = (re * re + im * im).sqrt();
    std::array::from_fn(|i| (h[i] / g) as f32)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Gain of a FIR centered on its middle tap at `f` Hz (magnitude).
    fn gain(h: &[f32], f: f64) -> f64 {
        let mid = (h.len() / 2) as f64;
        let (re, im) = h.iter().enumerate().fold((0.0, 0.0), |(re, im), (i, &c)| {
            let w = 2.0 * PI * f * (i as f64 - mid) / SAMPLE_RATE;
            (re + c as f64 * w.cos(), im + c as f64 * w.sin())
        });
        (re * re + im * im).sqrt()
    }

    #[test]
    fn flat_colors_are_colodore() {
        // Through the delay line (average of an even and an odd line) the
        // model gives back exactly the palette of the VIC
        let (even, odd) = (palette_yuv(false), palette_yuv(true));
        for i in 0..16 {
            let avg: [f64; 3] = std::array::from_fn(|k| (even[i][k] + odd[i][k]) / 2.0);
            let rgb = yuv_to_rgb(avg[0], avg[1], avg[2]).map(|c| to_display(to_light(c)).round() as u32);
            let argb = 0xFF00_0000 | rgb[0] << 16 | rgb[1] << 8 | rgb[2];
            assert_eq!(argb, C64_PALETTE[i], "color {i}");
        }
    }

    #[test]
    fn odd_lines_turn_the_hue() {
        let (even, odd) = (palette_yuv(false), palette_yuv(true));
        let angle = |c: [f64; 3]| c[2].atan2(c[1]).to_degrees();
        for i in [2, 3, 4, 5, 6, 7, 8, 9, 10, 13, 14] {
            let d = (angle(odd[i]) - angle(even[i]) + 360.0) % 360.0;
            assert!((d - ODD_LINE_PHASE).abs() < 1e-9, "color {i}: {d}");
        }
    }

    #[test]
    fn signal_timing() {
        assert!((DOT_CLOCK - 7_881_988.9).abs() < 1.0);
        assert_eq!(SAMPLES, 907);
        // 504 pixels per line = 283.5 subcarrier cycles
        assert!((504.0 * FSC / DOT_CLOCK - 283.5).abs() < 1e-9);
        assert!((PIXEL_ASPECT - 0.9357).abs() < 1e-4);
        // The monitor geometry agrees with the PAL pixel aspect
        let m = &C1084S_P1;
        assert!((m.pixel_width() / m.line_pitch() - PIXEL_ASPECT).abs() < 0.005);
    }

    #[test]
    fn vic_luma_step() {
        let r = SecondOrder::vic_luma();
        let rise = r.time_to(0.9) - r.time_to(0.1);
        assert!((rise - VIC_LUMA_RISE_PX).abs() < 1e-6);
        // Peak of the step response: the measured overshoot
        let peak = (0..10_000).map(|i| r.step(i as f64 * 1e-3)).fold(0.0, f64::max);
        assert!((peak - 1.0 - VIC_LUMA_OVERSHOOT).abs() < 1e-4, "{peak}");
    }

    #[test]
    fn luma_kernels_step_through_the_pixels() {
        let k = luma_kernels();
        for phase in &k {
            let sum: f32 = phase.iter().sum();
            assert!((sum - 1.0).abs() < 1e-5);
        }
        // A black-white edge: half height at the boundary, overshoot
        // afterwards, black before
        let level = |n: usize| {
            let pos = (n as f64 + 0.5) * 4.0 / 9.0;
            let p = pos.floor() as i32;
            k[n % 9].iter().enumerate()
                .map(|(i, &w)| if p + LUMA_FIRST + i as i32 >= 40 { w as f64 } else { 0.0 })
                .sum::<f64>()
        };
        // Pixel 40 starts at sample 90
        assert!(level(85) < 0.01);
        assert!((level(89) + level(90)) / 2.0 > 0.3 && (level(89) + level(90)) / 2.0 < 0.7);
        let peak = (90..110).map(level).fold(0.0, f64::max);
        assert!(peak > 1.08 && peak < 1.13, "{peak}");
    }

    #[test]
    fn chroma_bandwidth() {
        let h = chroma_fir(&C1084S_P1, Input::LumaChroma);
        assert!((gain(&h, 0.0) - 1.0).abs() < 1e-6);
        let g = gain(&h, 1_300_000.0);
        assert!((g - 0.707).abs() < 0.03, "{g}");
        // Twice the subcarrier (the demodulation product): what a
        // second-order filter leaves, about 2%
        assert!(gain(&h, 2.0 * FSC).abs() < 0.03);
    }

    #[test]
    fn luma_bandwidth() {
        for (m, input, bw) in [
            (&C1084S_P1, Input::LumaChroma, 8_000_000.0),
            (&C1084S_D1, Input::LumaChroma, 5_200_000.0),
            (&C1084S_D1, Input::Composite, 4_400_000.0),
        ] {
            let h = luma_fir(m, input);
            assert!((gain(&h, 0.0) - 1.0).abs() < 1e-6);
            let g = gain(&h, bw);
            assert!((g - 0.707).abs() < 0.03, "{bw}: {g}");
        }
    }

    #[test]
    fn peaking_1901() {
        // +6 dB shelf: gain 1 at DC, about 2 well above 1.2 MHz
        let h = luma_input_fir(&C1901, Input::LumaChroma);
        assert!((gain(&h, 0.0) - 1.0).abs() < 1e-3);
        let g = gain(&h, 4_000_000.0);
        assert!(g > 1.8 && g < 2.05, "{g}");
        // No trap: the subcarrier stays in the luma with composite
        assert!(gain(&luma_input_fir(&C1901, Input::Composite), FSC) > 1.8);
        // The 1084S has neither: luma/chroma passes as it is
        let flat = luma_input_fir(&C1084S_P1, Input::LumaChroma);
        assert!((gain(&flat, 3_000_000.0) - 1.0).abs() < 1e-6);
    }

    #[test]
    fn chroma_band_pass_1901() {
        // Q 3.6 at 4.43 MHz: ±0.62 MHz, narrower than the 1.3 MHz of PAL
        let h = chroma_fir(&C1901, Input::LumaChroma);
        let g = gain(&h, FSC / (2.0 * 3.6));
        assert!(g > 0.6 && g < 0.72, "{g}");
        // In composite the band-pass is done on the signal instead
        let c = chroma_fir(&C1901, Input::Composite);
        assert!((gain(&c, 1_300_000.0) - 0.707).abs() < 0.03);
    }

    #[test]
    fn white_points() {
        assert_eq!(white_balance(&C1084S_P1), [1.0; 3]);
        // The daylight locus at 6504 K is D65: no change
        let d65 = Monitor { white_point: Some(6504.0), ..C1901 };
        for c in white_balance(&d65) {
            assert!((c - 1.0).abs() < 0.01, "{c}");
        }
        // 7500 K: bluer, blue at full
        let [r, g, b] = white_balance(&C1901);
        assert_eq!(b, 1.0);
        assert!(r < g && g < 1.0 && r > 0.75, "{r} {g}");
    }

    #[test]
    fn models() {
        assert_eq!(Model::parse("1084S"), Some(Model::C1084SP1));
        assert_eq!(Model::parse("1084s-d1"), Some(Model::C1084SD1));
        assert_eq!(Model::parse("1702"), None);
        for model in [Model::C1084SP1, Model::C1084SD1, Model::C1901, Model::PhilipsCp90] {
            assert_eq!(Model::parse(model.name()), Some(model));
        }
    }

    #[test]
    fn luma_trap() {
        let h = luma_input_fir(&C1084S_P1, Input::Composite);
        assert!((gain(&h, 0.0) - 1.0).abs() < 1e-6);
        assert!(gain(&h, FSC) < 0.005);
        assert!(gain(&h, 3_000_000.0) > 0.8);
        assert!(gain(&h, 1_000_000.0) > 0.9);
    }

    #[test]
    fn chroma_band_pass() {
        // Composite chroma: unity at the subcarrier, nothing at DC, the
        // band of a Q 2 circuit on the 1084S
        let h = chroma_input_fir(&C1084S_P1, Input::Composite);
        assert!((gain(&h, FSC) - 1.0).abs() < 1e-6);
        assert!(gain(&h, 0.0) < 1e-3);
        // The analog response: 1 / sqrt(1 + Q²(x - 1/x)²) at x = f/FSC
        let x: f64 = 0.75;
        let want = 1.0 / (1.0 + 4.0 * (x - 1.0 / x).powi(2)).sqrt();
        let g = gain(&h, x * FSC);
        assert!((g - want).abs() < 0.01, "{g} {want}");
    }

    #[test]
    fn tv_shallow_trap() {
        // CP90 with SCART composite: the trap takes 6 dB at the subcarrier
        let h = luma_input_fir(&PHILIPS_CP90, Input::Composite);
        let g = gain(&h, FSC);
        assert!((g - 0.5).abs() < 0.02, "{g}");
    }

    #[test]
    fn if_filter() {
        // Both sidebands through the Nyquist slope: flat at low frequencies
        let h: Vec<f32> = if_response(&SAW_BG).iter().map(|&x| x as f32).collect();
        for f in [100e3, 500e3, 1e6, 2e6, 3e6] {
            let g = gain(&h, f);
            assert!((g - 1.0).abs() < 0.1, "{f}: {g}");
        }
        // The color carrier 3 dB down, the sound carrier (5.5 MHz) 20 dB
        let g = gain(&h, FSC);
        assert!((g - 0.7).abs() < 0.08, "{g}");
        assert!(gain(&h, 5.5e6) < 0.13);
        // RF on the TV: luma through the IF and the shallow trap, well
        // below the monitors above 3.5 MHz
        let rf = luma_input_fir(&PHILIPS_CP90, Input::Rf);
        assert!(gain(&rf, 4e6) < 0.6 && gain(&rf, 2e6) > 0.9);
    }

    #[test]
    fn modulator() {
        // The C64's luma in its composite output: flat at low frequencies,
        // a notch at the subcarrier, a bump near 2 MHz (nodal analysis:
        // +2.9 dB)
        let h = modulator_fir();
        assert!((gain(&h, 100e3) - 1.0).abs() < 0.02);
        assert!(gain(&h, FSC) < 0.05);
        let bump = gain(&h, 2e6);
        assert!(bump > 1.2 && bump < 1.5, "{bump}");
    }

    #[test]
    fn settings() {
        let tv = Crt::apply(None, "rf").unwrap().unwrap();
        assert_eq!((tv.model, tv.input), (Model::PhilipsCp90, Input::Rf));
        // A monitor after the TV: the RF input goes, luma/chroma comes
        let m = Crt::apply(Some(tv), "1901").unwrap().unwrap();
        assert_eq!((m.model, m.input), (Model::C1901, Input::LumaChroma));
        // Composite is kept across models
        let c = Crt::apply(Crt::apply(None, "composite").unwrap(), "tv").unwrap().unwrap();
        assert_eq!((c.model, c.input), (Model::PhilipsCp90, Input::Composite));
        assert_eq!(Crt::apply(Some(c), "off").unwrap(), None);
        assert!(Crt::apply(None, "1702").is_err());
        // RF on a monitor is caught by check
        let bad = Crt::apply(Crt::apply(None, "1084s").unwrap(), "rf").unwrap().unwrap();
        assert!(bad.check().is_err());
    }

    #[test]
    fn inputs() {
        assert!(Crt::new(Model::PhilipsCp90).input == Input::Rf);
        assert!(Crt::new(Model::C1901).input == Input::LumaChroma);
        assert!(Crt { model: Model::C1084SP1, input: Input::Rf }.check().is_err());
        assert!(Crt { model: Model::PhilipsCp90, input: Input::LumaChroma }.check().is_err());
        assert!(Crt { model: Model::PhilipsCp90, input: Input::Composite }.check().is_ok());
    }

    #[test]
    fn palette_lookup() {
        for (i, &c) in C64_PALETTE.iter().enumerate() {
            assert_eq!(palette_index(c), i);
        }
        // Not a palette color: the nearest
        assert_eq!(palette_index(0xFF7A7C7B), 12);
    }
}
