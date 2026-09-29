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

use crate::timing::Standard;
use crate::vic::{Chip, WIDTH};

/// Color system of a signal, and of the decoder of a color set.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Color {
    /// PAL B/G: 4.43 MHz subcarrier, 625 lines.
    Pal,
    /// NTSC M: 3.58 MHz subcarrier, 525 lines.
    Ntsc,
    /// PAL N (Argentina): PAL with a 3.58 MHz subcarrier, 625 lines.
    PalN,
}

impl Color {
    pub fn of(standard: Standard) -> Color {
        match standard {
            Standard::Pal => Color::Pal,
            Standard::Ntsc | Standard::NtscOld => Color::Ntsc,
            Standard::PalN => Color::PalN,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Color::Pal => "PAL",
            Color::Ntsc => "NTSC",
            Color::PalN => "PAL-N",
        }
    }

    /// The usual machine standard that sends it.
    pub fn standard(self) -> Standard {
        match self {
            Color::Pal => Standard::Pal,
            Color::Ntsc => Standard::Ntsc,
            Color::PalN => Standard::PalN,
        }
    }

    /// 625-line system (PAL, PAL-N), else 525 (NTSC).
    fn lines_625(self) -> bool {
        self != Color::Ntsc
    }
}

/// The video signal of a C64: its color subcarrier and the sampling, at 4
/// samples per subcarrier cycle, that is at the frequency of the C64's
/// crystal, from which the pixel clock is divided.
///
/// | | PAL | NTSC | Old NTSC | PAL-N |
/// |---|---|---|---|---|
/// | Subcarrier | 4.43361875 MHz | 3.579545 MHz | 3.579545 MHz | 3.58205625 MHz |
/// | Pixel clock | crystal × 4/9 | crystal × 4/7 | crystal × 4/7 | crystal × 4/7 |
/// | Samples every 4 pixels | 9 | 7 | 7 | 7 |
/// | Pixels per line | 504 | 520 | 512 | 520 |
/// | Subcarrier cycles per line | 283.5 | 227.5 | 224 | 227.5 |
///
/// With half a cycle more than a whole number per line the phase of the
/// subcarrier flips from line to line, and the color artifacts of fine
/// detail alternate and partly cancel; the 6567R56A has a whole number, so
/// they line up in vertical stripes. A PAL or PAL-N frame (312 lines) and
/// an old NTSC one are a whole number of cycles, so the artifacts stand
/// still, while an NTSC frame (263 lines) ends half a cycle off, so they
/// alternate from frame to frame (the dot crawl of NTSC).
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Signal {
    /// The VIC-II that makes it (its lumas: `Chip::luma`).
    pub chip: Chip,
    pub standard: Standard,
    /// Color subcarrier, Hz.
    pub fsc: f64,
    /// Sampling rate, Hz: 4 × fsc.
    pub sample_rate: f64,
    /// Samples every 4 pixels: 9 PAL, 7 the others.
    pub phases: usize,
    /// Samples per framebuffer line.
    pub samples: usize,
}

impl Signal {
    pub fn of(chip: Chip) -> Signal {
        let standard = chip.standard();
        let (fsc, phases) = match standard {
            Standard::Pal => (4_433_618.75, 9),
            Standard::Ntsc | Standard::NtscOld => (315e6 / 88.0, 7),
            // 4 × fsc is the Drean's 14.328225 MHz crystal
            Standard::PalN => (3_582_056.25, 7),
        };
        Signal { chip, standard, fsc, sample_rate: 4.0 * fsc, phases, samples: (WIDTH * phases + 3) / 4 }
    }

    pub fn color(&self) -> Color {
        Color::of(self.standard)
    }

    /// PAL and PAL-N: V switched on alternate lines and averaged by the
    /// delay line.
    pub fn pal(&self) -> bool {
        self.color() != Color::Ntsc
    }

    /// Samples in a raster line.
    pub fn line_samples(&self) -> usize {
        8 * self.standard.cycles_per_line() as usize * self.phases / 4
    }

    /// The subcarrier phase flips from a raster line to the next (half a
    /// cycle over a whole number per line).
    pub fn line_flip(&self) -> bool {
        self.line_samples() % 4 == 2
    }

    /// Pixel clock, Hz.
    pub fn dot_clock(&self) -> f64 {
        self.sample_rate * 4.0 / self.phases as f64
    }

    /// Width over height of a C64 pixel on the screen: the square-pixel
    /// rate of the line system (625 lines 7.375 MHz, 525 lines 6.136 MHz,
    /// for 320-pixel lines) over the pixel clock: 0.936 PAL, 0.75 NTSC, 0.90
    /// PAL-N.
    pub fn pixel_aspect(&self) -> f64 {
        let square = if self.color().lines_625() { 7_375_000.0 } else { 135e6 / 22.0 };
        square / self.dot_clock()
    }

    /// Active part of a line, seconds: 52 µs in 625-line sets, 52.66 µs in
    /// 525-line ones.
    pub fn active_line(&self) -> f64 {
        if self.color().lines_625() { 52e-6 } else { 52.66e-6 }
    }

    /// Lines of a field that fill the picture height: 288 in 625-line sets
    /// (576 active lines), 242.5 in 525-line ones (485).
    pub fn picture_lines(&self) -> f64 {
        if self.color().lines_625() { 288.0 } else { 242.5 }
    }
}

/// Luma overshoot of the VIC-II on a black to white step (measured, Jam on
/// Toads: 1.05 V white, peak about 1.14 V over 0.32 V black).
const VIC_LUMA_OVERSHOOT: f64 = 0.12;
/// Luma rise time of the VIC-II, 10-90%, in pixels (same measurement:
/// "almost 2 pixels" to white; 10-90% about 1.5).
const VIC_LUMA_RISE_PX: f64 = 1.5;
/// Chroma phase difference between odd and even raster lines of the PAL
/// chips, degrees (mean of Tobias' measurements on the 6569R5: 11-16°; on
/// the 8565R2 most colors measure 12-19°, but yellow and brown the other
/// way, so the same mean is kept; the 6572 is assumed to be as the 6569).
const ODD_LINE_PHASE: f64 = 13.0;
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
            Input::Rf => "RF",
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

/// Emulated monitors and TVs.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Model {
    /// Commodore 1084S-P1 (Philips chassis, PAL, 1990).
    C1084SP1,
    /// Commodore 1084S-D1 (Daewoo chassis, PAL).
    C1084SD1,
    /// Commodore 1901 (Thomson chassis, PAL, 1986).
    C1901,
    /// Philips 15CE1510 color TV (CP90 chassis, PAL, 1988).
    PhilipsCp90,
    /// Philips 16CT2216 color TV (CTX-E chassis, PAL, 1983).
    PhilipsCtxe,
    /// Commodore 1701 for PAL countries (JVC chassis, as the PAL 1702).
    C1701,
    /// Commodore 1802 for PAL countries (Daewoo CM-146 chassis, 1987).
    C1802,
    /// Commodore 1702 (JVC chassis, NTSC, 1984).
    C1702,
    /// Commodore 1084S-P for NTSC countries (Philips/Magnavox, 1988).
    C1084SPNtsc,
    /// Sony Trinitron KV-1311CR color TV (NTSC, about 1985).
    SonyKv1311,
    /// Sontec CNT-4442 B color TV (Argentina, PAL-N).
    SontecCnt4442,
    /// Commodore 1900 M green monochrome monitor (Philips BM7502).
    C1900,
}

impl Model {
    /// Names accepted by `parse`, for help and error messages.
    pub const NAMES: &'static str = "1084s (the 1084S-P1 on PAL, the NTSC 1084S-P on NTSC), 1084s-p1, 1084s-d1, 1901, \
        cp90, ctxe, 1701 (also 1702 on PAL), 1802 for PAL; 1702, 1084s-p, kv1311 for NTSC; cnt4442 for PAL-N; tv (the TV of the standard); 1900 (monochrome)";

    pub const ALL: [Model; 12] = [
        Model::C1084SP1, Model::C1084SD1, Model::C1901, Model::PhilipsCp90, Model::PhilipsCtxe, Model::C1701,
        Model::C1802, Model::C1702,
        Model::C1084SPNtsc, Model::SonyKv1311, Model::SontecCnt4442, Model::C1900,
    ];

    /// The model named `s`; `1084s` and `tv` are the ones of the machine's
    /// `standard`.
    pub fn parse(s: &str, standard: Standard) -> Option<Model> {
        let color = Color::of(standard);
        match s.to_ascii_lowercase().as_str() {
            "1084s" | "1084" if color == Color::Ntsc => Some(Model::C1084SPNtsc),
            "1084s" | "1084" | "1084s-p1" | "1084sp1" => Some(Model::C1084SP1),
            "1084s-p" | "1084sp" => Some(Model::C1084SPNtsc),
            "1084s-d1" | "1084sd1" => Some(Model::C1084SD1),
            "1901" => Some(Model::C1901),
            "tv" => Some(match color {
                Color::Pal => Model::PhilipsCp90,
                Color::Ntsc => Model::SonyKv1311,
                Color::PalN => Model::SontecCnt4442,
            }),
            "cp90" | "15ce1510" => Some(Model::PhilipsCp90),
            "ctxe" | "ctx-e" | "16ct2216" => Some(Model::PhilipsCtxe),
            "1701" | "1702-pal" => Some(Model::C1701),
            "1702" if color == Color::Pal => Some(Model::C1701),
            "1702" => Some(Model::C1702),
            "1802" => Some(Model::C1802),
            "kv1311" | "kv-1311cr" | "kv1311cr" | "sony" => Some(Model::SonyKv1311),
            "cnt4442" | "cnt-4442" | "sontec" => Some(Model::SontecCnt4442),
            "1900" | "1900m" | "dm602" => Some(Model::C1900),
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
            Model::PhilipsCtxe => "CTX-E",
            Model::C1701 => "1701",
            Model::C1802 => "1802",
            Model::C1702 => "1702",
            Model::C1084SPNtsc => "1084S-P",
            Model::SonyKv1311 => "KV1311",
            Model::SontecCnt4442 => "CNT4442",
            Model::C1900 => "1900",
        }
    }

    /// Full name, with the brand.
    pub fn full_name(self) -> &'static str {
        match self {
            Model::C1084SP1 => "Commodore 1084S-P1",
            Model::C1084SD1 => "Commodore 1084S-D1",
            Model::C1901 => "Commodore 1901",
            Model::PhilipsCp90 => "Philips 15CE1510 TV (CP90 chassis)",
            Model::PhilipsCtxe => "Philips 16CT2216 TV (CTX-E chassis)",
            Model::C1701 => "Commodore 1701 (PAL)",
            Model::C1802 => "Commodore 1802 (PAL)",
            Model::C1702 => "Commodore 1702",
            Model::C1084SPNtsc => "Commodore 1084S-P (NTSC)",
            Model::SonyKv1311 => "Sony Trinitron KV-1311CR TV",
            Model::SontecCnt4442 => "Sontec CNT-4442 B TV",
            Model::C1900 => "Commodore 1900 M",
        }
    }

    pub fn monitor(self) -> &'static Monitor {
        match self {
            Model::C1084SP1 => &C1084S_P1,
            Model::C1084SD1 => &C1084S_D1,
            Model::C1901 => &C1901,
            Model::PhilipsCp90 => &PHILIPS_CP90,
            Model::PhilipsCtxe => &PHILIPS_CTXE,
            Model::C1701 => &C1701_PAL,
            Model::C1802 => &C1802_PAL,
            Model::C1702 => &C1702,
            Model::C1084SPNtsc => &C1084S_P_NTSC,
            Model::SonyKv1311 => &SONY_KV1311CR,
            Model::SontecCnt4442 => &SONTEC_CNT4442,
            Model::C1900 => &C1900,
        }
    }
}

/// A front-panel control of a set. The manuals say only which way each one
/// turns and that the normal setting is the centre click-stop position
/// (knob 0, where the model gives the colodore colors): the ranges are
/// estimates.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Knob {
    /// Black level: at ±100 the drive of the guns moves by ±BRIGHTNESS_RANGE
    /// of white (before the tube gamma).
    Brightness,
    /// Gain of the video amplifier (luma and chroma): ×2^(knob/100), half to
    /// twice.
    Contrast,
    /// Chroma gain ("colour", "intensity"): ×(1 + knob/100), from none to
    /// twice.
    Color,
    /// NTSC hue ("tint"): the demodulation axes turn by ±TINT_RANGE, to
    /// the left towards red (purple), to the right towards green.
    Tint,
    /// Luma peaking of the sets that have the control (`Monitor::peak`):
    /// its gain ×(1 + knob/100), from none to twice.
    Sharpness,
}

/// Brightness range, share of the white drive.
pub const BRIGHTNESS_RANGE: f64 = 0.25;
/// Tint range, degrees.
pub const TINT_RANGE: f64 = 45.0;

impl Knob {
    pub const ALL: [Knob; 5] = [Knob::Brightness, Knob::Contrast, Knob::Color, Knob::Tint, Knob::Sharpness];

    pub fn name(self) -> &'static str {
        match self {
            Knob::Brightness => "brightness",
            Knob::Contrast => "contrast",
            Knob::Color => "color",
            Knob::Tint => "tint",
            Knob::Sharpness => "sharpness",
        }
    }

    pub fn parse(s: &str) -> Option<Knob> {
        match s {
            "brightness" | "bright" => Some(Knob::Brightness),
            "contrast" => Some(Knob::Contrast),
            "color" | "colour" | "saturation" => Some(Knob::Color),
            "tint" | "hue" => Some(Knob::Tint),
            "sharpness" => Some(Knob::Sharpness),
            _ => None,
        }
    }
}

/// CRT emulation settings: monitor, connection, front-panel controls and
/// switches.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Crt {
    pub model: Model,
    pub input: Input,
    /// Positions of the knobs (`Knob` order), -100 to 100, 0 at the centre.
    pub knobs: [i8; 5],
    /// The comb filter of the sets that have one is on (the "comb defeat"
    /// switch of the 1084S-P turns it off).
    pub comb: bool,
    /// The VIC-II's jail bars (`jail_bars`) in its luma.
    pub bars: bool,
    /// Geometry and convergence errors of the picture tube, as a share
    /// (0-100%) of the largest its data book allows (`Tolerance`): 0, a
    /// perfect tube, by default.
    pub geometry: u8,
}

impl Crt {
    /// The model with its first input (luma/chroma for the monitors, RF
    /// for the TVs), the knobs at the centre, comb filter and jail bars on.
    pub fn new(model: Model) -> Crt {
        Crt { model, input: model.monitor().inputs[0], knobs: [0; 5], comb: true, bars: true, geometry: 0 }
    }

    /// Gain of the peaking of the set, with its sharpness knob.
    pub fn peak_gain(&self) -> f64 {
        let Some(p) = self.model.monitor().peak else { return 0.0 };
        p.gain * (1.0 + self.knob(Knob::Sharpness) as f64 / 100.0)
    }

    /// The comb filter separates luma and chroma: the set has one, it is on,
    /// and the input is composite or RF.
    pub fn comb_active(&self) -> bool {
        self.comb && self.input != Input::LumaChroma && self.model.monitor().comb.is_some()
    }

    pub fn knob(&self, k: Knob) -> i8 {
        self.knobs[k as usize]
    }

    /// Brightness as an offset of the gun drive, 0-255 units.
    pub fn brightness(&self) -> f64 {
        255.0 * BRIGHTNESS_RANGE * self.knob(Knob::Brightness) as f64 / 100.0
    }

    /// Gain of the video amplifier.
    pub fn contrast(&self) -> f64 {
        2f64.powf(self.knob(Knob::Contrast) as f64 / 100.0)
    }

    /// Chroma gain.
    pub fn color(&self) -> f64 {
        1.0 + self.knob(Knob::Color) as f64 / 100.0
    }

    /// Hue rotation, radians.
    pub fn tint(&self) -> f64 {
        (TINT_RANGE * self.knob(Knob::Tint) as f64 / 100.0).to_radians()
    }

    /// The set shown by default for a machine of `standard`, or connected
    /// by `input` when the emulation is turned on with it: the 1084S-P1 for
    /// PAL, the 1702 for NTSC, the TV of the standard for RF.
    pub fn default_for(standard: Standard, input: Option<Input>) -> Crt {
        let model = match (Color::of(standard), input) {
            (Color::Pal, Some(Input::Rf)) => Model::PhilipsCp90,
            (Color::Pal, _) => Model::C1084SP1,
            (Color::Ntsc, Some(Input::Rf)) => Model::SonyKv1311,
            (Color::Ntsc, _) => Model::C1702,
            (Color::PalN, _) => Model::SontecCnt4442,
        };
        let crt = Crt::new(model);
        match input {
            Some(input) if model.monitor().inputs.contains(&input) => Crt { input, ..crt },
            _ => crt,
        }
    }

    /// The settings `crt` (None: off) changed by `arg`: `off`, a model
    /// (keeping the input if the model has it, otherwise its default one,
    /// and the knobs it has), an input (`lc`, `composite`, `rf`; turning the
    /// emulation on with the set of the machine's `standard`,
    /// `default_for`), a knob (`tint=-20`: -100 to 100, 0 the centre), a
    /// switch (`comb=on|off`, `bars=on|off`), or several of them separated
    /// by commas.
    pub fn apply(crt: Option<Crt>, arg: &str, standard: Standard) -> Result<Option<Crt>, String> {
        if arg.contains(',') {
            let mut crt = crt;
            for a in arg.split(',').filter(|a| !a.is_empty()) {
                crt = Crt::apply(crt, a, standard)?;
            }
            return Ok(crt);
        }
        if arg == "off" {
            return Ok(None);
        }
        if let Some((name, value @ ("on" | "off"))) = arg.split_once('=') {
            let on = value == "on";
            let mut c = crt.unwrap_or(Crt::default_for(standard, None));
            match name {
                "comb" if c.model.monitor().comb.is_some() => c.comb = on,
                "comb" => return Err(format!("the {} has no comb filter", c.model.full_name())),
                "bars" | "jailbars" => c.bars = on,
                "geometry" => c.geometry = if on { 100 } else { 0 },
                _ => return Err(format!("unknown switch '{name}': comb, bars, geometry")),
            }
            return Ok(Some(c));
        }
        if let Some(("geometry", value)) = arg.split_once('=') {
            let share: u8 = value.parse().ok().filter(|v| *v <= 100)
                .ok_or_else(|| format!("geometry: a share from 0 to 100 of the tube's tolerances, not '{value}'"))?;
            let mut c = crt.unwrap_or(Crt::default_for(standard, None));
            c.geometry = share;
            return Ok(Some(c));
        }
        if let Some((name, value)) = arg.split_once('=') {
            let knob = Knob::parse(name).ok_or_else(|| {
                let names: Vec<&str> = Knob::ALL.iter().map(|k| k.name()).collect();
                format!("unknown control '{name}': {}", names.join(", "))
            })?;
            let value: i8 = value.parse().ok().filter(|v: &i8| (-100..=100).contains(v))
                .ok_or_else(|| format!("{}: a position from -100 to 100 (0 the centre), not '{value}'", knob.name()))?;
            let mut c = crt.unwrap_or(Crt::default_for(standard, None));
            if !c.model.monitor().knobs.contains(&knob) {
                return Err(format!("the {} has no {} control", c.model.full_name(), knob.name()));
            }
            c.knobs[knob as usize] = value;
            return Ok(Some(c));
        }
        if let Some(input) = Input::parse(arg) {
            return Ok(Some(match crt {
                Some(c) => Crt { input, ..c },
                None => Crt { input, ..Crt::default_for(standard, Some(input)) },
            }));
        }
        let model = Model::parse(arg, standard)
            .ok_or_else(|| format!("unknown monitor or input '{arg}': sets {}; inputs lc, composite, rf", Model::NAMES))?;
        let m = model.monitor();
        let input = crt.map(|c| c.input).filter(|i| m.inputs.contains(i));
        let mut knobs = crt.map(|c| c.knobs).unwrap_or([0; 5]);
        for k in Knob::ALL {
            if !m.knobs.contains(&k) {
                knobs[k as usize] = 0;
            }
        }
        let fresh = Crt::new(model);
        Ok(Some(Crt {
            model,
            input: input.unwrap_or(fresh.input),
            knobs,
            comb: crt.map_or(true, |c| c.comb),
            bars: crt.map_or(true, |c| c.bars),
            geometry: crt.map_or(0, |c| c.geometry),
        }))
    }

    /// Error if the model does not have the input, or does not decode the
    /// color of the machine's `standard` (a PAL monitor shows an NTSC C64 in
    /// black and white, and vice versa).
    pub fn check(&self, standard: Standard) -> Result<(), String> {
        let m = self.model.monitor();
        let color = Color::of(standard);
        if let Some(mc) = m.color.filter(|&c| c != color) {
            let sets: Vec<&str> = Model::ALL.iter().filter(|x| x.monitor().color == Some(color)).map(|x| x.name()).collect();
            let a = |c: Color| if c == Color::Ntsc { "an NTSC" } else { "a" };
            let set = if m.if_filter.is_some() { "set" } else { "monitor" };
            return Err(format!("the {} is {} {} {set}: with {} {} C64 use {}",
                self.model.full_name(), a(mc), mc.name(), a(color), color.name(), sets.join(", ")));
        }
        let inputs = m.inputs;
        if inputs.contains(&self.input) {
            return Ok(());
        }
        let names: Vec<&str> = inputs.iter().map(|i| i.name()).collect();
        Err(format!("the {} has no {} input (it has: {})", self.model.full_name(), self.input.name(), names.join(", ")))
    }

    pub fn describe(&self) -> String {
        let mut s = format!("{}, {} input", self.model.full_name(), self.input.name());
        for k in Knob::ALL.into_iter().filter(|&k| self.knob(k) != 0) {
            s += &format!(", {} {:+}", k.name(), self.knob(k));
        }
        if !self.comb && self.model.monitor().comb.is_some() {
            s += ", comb filter off";
        }
        if !self.bars {
            s += ", no jail bars";
        }
        if self.geometry > 0 {
            s += &format!(", geometry and convergence errors {}% of the tolerances", self.geometry);
        }
        s
    }
}

// ── Monitors ─────────────────────────────────────────────────────────────────

/// Physical data of a monitor.
#[derive(Debug)]
pub struct Monitor {
    /// Color system of its decoder: its C64 must send that one. None: a
    /// monochrome monitor, which shows every standard, without color.
    pub color: Option<Color>,
    /// Color of the phosphor of a monochrome tube, CIE xy.
    pub phosphor: Option<(f64, f64)>,
    /// Visible screen, mm.
    pub screen_width: f64,
    pub screen_height: f64,
    /// Share of the picture scanned beyond the edges of the screen, on
    /// each axis: a TV hides part of the border (a monitor is set to show
    /// all of it).
    pub overscan: f64,
    /// Horizontal pitch of the phosphor triads, mm; 0: no mask (monochrome).
    pub triad_pitch: f64,
    /// Width of a phosphor stripe, as a fraction of the triad.
    pub stripe: f64,
    /// Vertical period of the mask slots, mm, and height of the bridges
    /// between them as a fraction of it (0: an aperture grille).
    pub slot_pitch: f64,
    pub bridge: f64,
    /// Standard deviation of the beam, in line pitches: at black and at
    /// full brightness.
    pub beam_sigma: (f64, f64),
    /// Halation in the glass: share of the light and spread (sigma), mm.
    pub glow: (f64, f64),
    /// Inputs of the set; the first is the default.
    pub inputs: &'static [Input],
    /// Front-panel controls of the picture.
    pub knobs: &'static [Knob],
    /// IF filter of the TV tuner (RF input).
    pub if_filter: Option<&'static IfFilter>,
    /// Bandwidth (-3 dB) of the luma amplifier, Hz, with separate luma and
    /// with a composite signal (composite or RF input).
    pub luma_bandwidth: [f64; 2],
    /// Luma peaking: time constant, s, of a high shelf of +6 dB (zero at
    /// 1/(2πτ), pole at twice that).
    pub luma_peaking: Option<f64>,
    /// Luma peaking around a frequency (with the Sharpness knob, if the set
    /// has it, from none at -100 to twice the gain at +100).
    pub peak: Option<Peak>,
    /// Luma trap at the subcarrier with a composite signal (without one,
    /// or with a comb filter, the subcarrier stays in the luma).
    pub luma_trap: Option<Trap>,
    /// Comb filter for a composite signal: luma and chroma separated with
    /// the previous line through a 1H delay line of this many subcarrier
    /// cycles (NTSC: 227.5).
    pub comb: Option<f64>,
    /// Band-pass that takes the chroma out of a composite signal.
    pub chroma_filter: ChromaFilter,
    /// The chroma input goes through that band-pass also with separate
    /// luma and chroma.
    pub chroma_bandpass: bool,
    /// Bandwidth (-3 dB) of the low-pass after the chroma demodulators, Hz.
    pub chroma_bandwidth: f64,
    /// Chroma delay line of the PAL decoder, subcarrier cycles: 283.5
    /// (63.943 µs) in PAL B/G sets, 229 (63.930 µs) in PAL-N ones.
    pub delay_line: f64,
    /// White point, K; None: D65, the white of the colodore palette.
    pub white_point: Option<f64>,
    /// Largest geometry and convergence errors of the picture tube.
    pub tolerance: &'static Tolerance,
}

/// Tolerances of a picture tube with its deflection unit, from the maker's
/// data book: the largest misconvergence between any two colors, mm, at
/// the centre, at the middle of the edges and in the corners; and the
/// pattern distortion without east-west and north-south correction, mm,
/// over a raster of `reference` mm (the bow of its vertical and of its
/// horizontal edges).
#[derive(Debug)]
pub struct Tolerance {
    pub converge: [f64; 3],
    pub pattern: (f64, f64),
    pub reference: (f64, f64),
}

/// Philips M34EAQ series, the 1084S's 14" tube with its deflection unit, a
/// "pre-aligned, self-converging and raster correction free" assembly
/// (data handbook T08, 1985): misconvergence 0.3 mm at the centre, 0.5 mm
/// in the area of 244 × 186 mm, 0.8 mm beyond; pattern distortion 3.0 mm
/// east-west, 2.1 mm north-south over 244 × 186 mm. Taken for the other
/// monitors too, whose tubes have no such data.
pub static TOLERANCE_MONITOR: Tolerance = Tolerance {
    converge: [0.3, 0.5, 0.8],
    pattern: (3.0, 2.1),
    reference: (244.0, 186.0),
};

/// Philips A37-598X0620, a 37 cm TV tube assembly of 1985 (data handbook
/// T08): misconvergence 0.3 mm at the centre, 0.7 mm at the middle of the
/// edges, 0.9 mm in the corners; pattern distortion 3 mm east-west over
/// 264 mm, 2.3 mm north-south over 190 mm. Taken for the TVs, whose tubes
/// have no such data.
pub static TOLERANCE_TV: Tolerance = Tolerance {
    converge: [0.3, 0.7, 0.9],
    pattern: (3.0, 2.3),
    reference: (264.0, 190.0),
};

/// Luma trap: a second-order notch, at the subcarrier unless the set's
/// components put it elsewhere.
#[derive(Debug, Clone, Copy)]
pub struct Trap {
    pub q: f64,
    /// Gain at the notch: 0 for a full notch.
    pub depth: f64,
    /// Frequency of the notch, Hz; None: the subcarrier.
    pub f0: Option<f64>,
}

/// Luma peaking: 1 + gain · (second-order band-pass of Q at f0, 1 at f0).
#[derive(Debug, Clone, Copy)]
pub struct Peak {
    pub f0: f64,
    pub q: f64,
    pub gain: f64,
}

/// Chroma band-pass of a set.
#[derive(Debug, Clone, Copy)]
pub enum ChromaFilter {
    /// Second-order band-pass of this Q tuned to the subcarrier.
    Tuned(f64),
    /// A series capacitor into a shunt of a coil, a capacitor and a
    /// resistor, from a low-impedance source (farads, henries, ohms).
    Network { series_c: f64, l: f64, shunt_c: f64, r: f64 },
    /// From a source resistance `r`, a series capacitor into a series coil
    /// and capacitor to ground, loaded by the decoder's input (`load_r` ∥
    /// `load_c`): a band-pass near where the coil resonates with the two
    /// capacitors in series, and a zero where it resonates with the shunt
    /// one alone, which keeps the low frequencies of the luma out (farads,
    /// henries, ohms).
    SeriesTrap { series_c: f64, shunt_c: f64, l: f64, r: f64, load_r: f64, load_c: f64 },
}

/// IF filter of a TV tuner: attenuation, dB, at IF frequencies, MHz, with
/// the picture carrier at `picture` MHz; and the network after the video
/// detector, if the set shapes the video there.
#[derive(Debug)]
pub struct IfFilter {
    pub picture: f64,
    pub points: &'static [(f64, f64)],
    pub detector: Option<Detector>,
}

/// Network at the video detector's output: a series coil into a shunt
/// resistor and capacitor in series, loaded by a resistor (henries, ohms,
/// farads), from a low-impedance source.
#[derive(Debug, Clone, Copy)]
pub struct Detector {
    pub l: f64,
    pub r: f64,
    pub c: f64,
    pub load: f64,
}

/// Controls of a PAL set: brightness, contrast, color (a PAL decoder needs
/// no tint).
const KNOBS_PAL: &[Knob] = &[Knob::Brightness, Knob::Contrast, Knob::Color];
/// Controls of an NTSC set, with the tint.
const KNOBS_NTSC: &[Knob] = &[Knob::Brightness, Knob::Contrast, Knob::Color, Knob::Tint];
/// Controls of a monochrome monitor.
const KNOBS_MONO: &[Knob] = &[Knob::Brightness, Knob::Contrast];

/// Commodore 1084S-P1 (service manual, Philips chassis): tube M34EAQ10X,
/// "slot triplet pitch 0.42 mm", 14", 90°; useful screen 280.8 × 210.6 mm
/// (Philips data handbook T08, 1986: "slotted shadow mask", in-line gun);
/// video bandwidth 8 MHz; TDA4510 PAL decoder with a 64 µs delay line and
/// an adjustable 4.43 MHz "chrominance suppression" trap, used in composite
/// mode only (transistor 7603 grounds it). The luma then goes through a
/// 330 ns delay line straight into the TDA3505: no peaking; the RGB output
/// stages have 47 µH shunt-peaking coils, for the flat 8 MHz.
pub static C1084S_P1: Monitor = Monitor {
    color: Some(Color::Pal),
    phosphor: None,
    screen_width: 280.8,
    screen_height: 210.6,
    overscan: 0.0,
    triad_pitch: 0.42,
    // Estimates: stripes with black guard bands, slots of the usual
    // shape for TV slot masks
    stripe: 0.25,
    slot_pitch: 0.8,
    bridge: 0.12,
    beam_sigma: (0.26, 0.42),
    glow: (0.05, 2.0),
    inputs: &[Input::LumaChroma, Input::Composite],
    knobs: KNOBS_PAL,
    if_filter: None,
    luma_bandwidth: [8_000_000.0, 8_000_000.0],
    luma_peaking: None,
    peak: None,
    luma_trap: Some(Trap { q: 2.0, depth: 0.0, f0: None }),
    comb: None,
    chroma_filter: ChromaFilter::Tuned(2.0),
    chroma_bandpass: false,
    // U/V bandwidth of PAL
    chroma_bandwidth: 1_300_000.0,
    delay_line: 283.5,
    white_point: None,
    tolerance: &TOLERANCE_MONITOR,
};

/// Commodore 1084S-D1 (service manual, Daewoo chassis): 13" visible tube,
/// 90°, in-line guns, "dot pitch 0.41 or 0.42 mm", "black strips screen";
/// display size 260 × 186 mm; video bandwidth 5.2 MHz with separate luma
/// and chroma, 4.4 MHz composite; TDA4510 PAL decoder (as the P1) with the
/// TDA3507. The estimates are those of the P1: the manual gives nothing
/// about the beam, the slots or the trap.
pub static C1084S_D1: Monitor = Monitor {
    color: Some(Color::Pal),
    phosphor: None,
    screen_width: 260.0,
    screen_height: 186.0,
    overscan: 0.0,
    triad_pitch: 0.41,
    stripe: 0.25,
    slot_pitch: 0.8,
    bridge: 0.12,
    beam_sigma: (0.26, 0.42),
    glow: (0.05, 2.0),
    inputs: &[Input::LumaChroma, Input::Composite],
    knobs: KNOBS_PAL,
    if_filter: None,
    luma_bandwidth: [5_200_000.0, 4_400_000.0],
    luma_peaking: None,
    peak: None,
    luma_trap: Some(Trap { q: 2.0, depth: 0.0, f0: None }),
    comb: None,
    chroma_filter: ChromaFilter::Tuned(2.0),
    chroma_bandpass: false,
    chroma_bandwidth: 1_300_000.0,
    delay_line: 283.5,
    white_point: None,
    tolerance: &TOLERANCE_MONITOR,
};

/// Commodore 1901 (service manual and schematics, Thomson chassis): 14"
/// tube, 90°, in-line guns, M34JGT60 with 0.43 mm pitch (a version with
/// the 0.39 mm M34JAW13 also existed); white point 7500 K. Luma: no trap;
/// the emitter follower before the TDA3506 drives 560 Ω ∥ 470 pF into
/// 560 Ω, a fixed peaking of +6 dB above about 1.2 MHz. Chroma: LC band-pass
/// (10 µH, damped by 1 kΩ: Q about 3.6 when tuned to 4.43 MHz) into the
/// AN5620X PAL decoder with a 64 µs delay line. Composite needs an internal
/// jumper; without a trap the subcarrier stays in the luma as a fine dot
/// pattern. The video output stages are shunt-feedback amplifiers (47 kΩ
/// feedback, 1.8 kΩ ∥ 68 pF input: the capacitor compensates the feedback
/// resistor's own capacitance), taken as flat. The manual gives neither the
/// picture size nor the bandwidth: those are the P1's, as the estimates.
pub static C1901: Monitor = Monitor {
    color: Some(Color::Pal),
    phosphor: None,
    screen_width: 280.8,
    screen_height: 210.6,
    overscan: 0.0,
    triad_pitch: 0.43,
    stripe: 0.25,
    slot_pitch: 0.8,
    bridge: 0.12,
    beam_sigma: (0.26, 0.42),
    glow: (0.05, 2.0),
    inputs: &[Input::LumaChroma, Input::Composite],
    knobs: KNOBS_PAL,
    if_filter: None,
    luma_bandwidth: [8_000_000.0, 8_000_000.0],
    luma_peaking: Some(560.0 * 470e-12),
    peak: None,
    luma_trap: None,
    comb: None,
    chroma_filter: ChromaFilter::Tuned(3.6),
    chroma_bandpass: true,
    chroma_bandwidth: 1_300_000.0,
    delay_line: 283.5,
    white_point: Some(7500.0),
    tolerance: &TOLERANCE_MONITOR,
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
/// time (EPCOS K2966M). Q of the trap and of the band-pass, bandwidth,
/// overscan and white point are not in the manual: the band-pass has the Q
/// of the TDA356x application circuit (10.7 µH ∥ 120 pF from 1 kΩ: about
/// 3), the overscan the 7% "normal scan" of Sony's monitors.
pub static PHILIPS_CP90: Monitor = Monitor {
    color: Some(Color::Pal),
    phosphor: None,
    screen_width: 284.5,
    screen_height: 213.4,
    overscan: TV_OVERSCAN,
    triad_pitch: 0.52,
    stripe: 0.25,
    slot_pitch: 0.8,
    bridge: 0.12,
    beam_sigma: (0.26, 0.42),
    glow: (0.05, 2.0),
    inputs: &[Input::Rf, Input::Composite],
    knobs: KNOBS_PAL,
    if_filter: Some(&SAW_BG),
    luma_bandwidth: [5_000_000.0, 5_000_000.0],
    luma_peaking: None,
    peak: None,
    luma_trap: Some(Trap { q: 2.0, depth: 0.5, f0: None }),
    comb: None,
    chroma_filter: ChromaFilter::Tuned(3.0),
    chroma_bandpass: false,
    chroma_bandwidth: 1_300_000.0,
    delay_line: 283.5,
    white_point: None,
    tolerance: &TOLERANCE_TV,
};

/// Philips 16CT2216 color TV (1983; CTX-E chassis, service manual CS 83
/// 862, also in 14" and 20" sets): tube A42-570X (radiomuseum.org, for
/// the 16CT2214), 42 cm, 90°, slotted mask with vertical stripes of 0.70 mm
/// at the centre, useful screen 322.1 × 241.6 mm (Philips data handbook
/// T08); antenna input only (user's manual: a VCR or a TV game go to the
/// aerial socket). TDA2541 IF amplifier and synchronous demodulator behind an LC IF filter
/// (`IF_CTXE`) and a network at the detector's output that lifts the
/// chroma band again (`Detector`); then 5.5 and 5.74 MHz sound traps (their
/// coils are not given: left out, the IF filter already takes the sound
/// carrier away), the luma through a delay line and the adjustable 4.43
/// MHz trap 5220 ("minimum chrominance") into the TDA3560 PAL decoder, with
/// its 64 µs delay line; the chroma through 15 pF from 1.5 kΩ into 120 pF
/// and the coil 5194 in series to ground, whose value the manual does not
/// give: 82.5 µH puts the peak at the subcarrier with the decoder's input
/// of 10 kΩ ∥ 6.5 pF (TDA3562A datasheet, of the same family); the zero is
/// then at 1.6 MHz. Q of the trap, luma bandwidth, overscan and white point
/// are not in the manual: those of the CP90.
pub static PHILIPS_CTXE: Monitor = Monitor {
    color: Some(Color::Pal),
    phosphor: None,
    screen_width: 322.1,
    screen_height: 241.6,
    overscan: TV_OVERSCAN,
    triad_pitch: 0.70,
    stripe: 0.25,
    slot_pitch: 0.8,
    bridge: 0.12,
    beam_sigma: (0.26, 0.42),
    glow: (0.05, 2.0),
    inputs: &[Input::Rf],
    knobs: KNOBS_PAL,
    if_filter: Some(&IF_CTXE),
    luma_bandwidth: [5_000_000.0, 5_000_000.0],
    luma_peaking: None,
    peak: None,
    luma_trap: Some(Trap { q: 2.0, depth: 0.0, f0: None }),
    comb: None,
    chroma_filter: ChromaFilter::SeriesTrap {
        series_c: 15e-12, shunt_c: 120e-12, l: 82.5e-6, r: 1.5e3, load_r: 10e3, load_c: 6.5e-12,
    },
    chroma_bandpass: false,
    chroma_bandwidth: 1_300_000.0,
    delay_line: 283.5,
    white_point: None,
    tolerance: &TOLERANCE_TV,
};

/// Commodore 1701 for PAL countries (schematic "Color video monitor
/// 1701-VDC", JVC board GE-1001A, 220/240 V; the PAL 1702, JVC CM20649 and
/// CM20716, is the same monitor): tube 370FVB22, vertical stripes of 0.64
/// mm (PAL user's guide); inputs luminance and PAL chroma (rear), composite
/// (front). M51393AP PAL decoder with a 1H delay line. The luma trap is L202
/// 27 µH in series with 33 + 9 pF, switched to ground by Q206 in composite
/// mode only, at a node fed through 560 Ω and loaded by 1 kΩ, the luma delay
/// line and 1.5 kΩ: those values put it at 4.73 MHz with a Q of 1.75 (the
/// subcarrier comes out 13 dB down). The chroma goes through 1000 pF and
/// 560 Ω into a double-tuned band-pass transformer damped by 680 Ω, whose
/// inner values are not printed (its Q, estimated, is the 1702's). The
/// decoder's "video tone" (double differentiation, pins 9, 10 and 13) and the
/// peaking of the CRT board are not given as responses: none. The Tint
/// control acts on the burst phase, which the delay line turns into a small
/// loss of saturation: not modelled. Picture size, bandwidth and white point
/// are not in the documents: the 1702's estimates, with the 5 MHz of PAL
/// video.
pub static C1701_PAL: Monitor = Monitor {
    color: Some(Color::Pal),
    phosphor: None,
    // 13" (330 mm) viewable, 4:3
    screen_width: 264.0,
    screen_height: 198.0,
    overscan: 0.0,
    triad_pitch: 0.64,
    stripe: 0.25,
    slot_pitch: 0.8,
    bridge: 0.12,
    beam_sigma: (0.26, 0.42),
    glow: (0.05, 2.0),
    inputs: &[Input::LumaChroma, Input::Composite],
    knobs: KNOBS_PAL,
    if_filter: None,
    luma_bandwidth: [5_000_000.0, 5_000_000.0],
    luma_peaking: None,
    peak: None,
    luma_trap: Some(Trap { q: 1.75, depth: 0.0, f0: Some(4_730_000.0) }),
    comb: None,
    chroma_filter: ChromaFilter::Tuned(3.0),
    chroma_bandpass: true,
    chroma_bandwidth: 1_300_000.0,
    delay_line: 283.5,
    white_point: None,
    tolerance: &TOLERANCE_MONITOR,
};

/// Commodore 1802 for PAL countries (Commodore 1802 technical manual, 5/87,
/// PAL section; schematic of the Daewoo CM-146 chassis): 13" 90° tube,
/// in-line guns, vertical stripes of 0.62 mm, grey face (370GGB22 or
/// 370EGB22); inputs composite, luma and chroma, monochrome; video bandwidth
/// 3.5 MHz composite, 4.2 MHz with separate luma and chroma; TA7698AP
/// decoder with a 1H delay line. The picture is set larger than the screen:
/// the 40 × 25 characters fill 247 × 160 mm ("display size"), 0.8 mm per
/// line, which on a 13" screen is 14% of overscan, as on a TV. Luma trap for
/// composite: L701 33 µH with 27 + 10 pF, switched by Q701, at 4.55 MHz (its
/// Q, and the trap inside the luma delay line DL201, are not legible on the
/// photocopy: the usual estimate). The Tint control is not modelled (see
/// the 1701); picture size of the tube, chroma band-pass and white point
/// are estimates, those of the 1701.
pub static C1802_PAL: Monitor = Monitor {
    color: Some(Color::Pal),
    phosphor: None,
    // 13" (330 mm) viewable, 4:3
    screen_width: 264.0,
    screen_height: 198.0,
    overscan: 0.14,
    triad_pitch: 0.62,
    stripe: 0.25,
    slot_pitch: 0.8,
    bridge: 0.12,
    beam_sigma: (0.26, 0.42),
    glow: (0.05, 2.0),
    inputs: &[Input::LumaChroma, Input::Composite],
    knobs: KNOBS_PAL,
    if_filter: None,
    luma_bandwidth: [4_200_000.0, 3_500_000.0],
    luma_peaking: None,
    peak: None,
    luma_trap: Some(Trap { q: 2.0, depth: 0.0, f0: Some(4_550_000.0) }),
    comb: None,
    chroma_filter: ChromaFilter::Tuned(3.0),
    chroma_bandpass: true,
    chroma_bandwidth: 1_300_000.0,
    delay_line: 283.5,
    white_point: None,
    tolerance: &TOLERANCE_MONITOR,
};

/// Commodore 1702 (service manual 314004-01, JVC chassis, NTSC; the 1701 is
/// the same monitor): 13" viewable, in-line guns (tube 370FVB22), "vertical
/// stripe, 0.64 mm pitch" (1701/1702 user's guides); inputs luminance,
/// chrominance (rear) and composite (front). The luma has a 3.58 MHz trap
/// (T201, adjusted from the composite input for the least subcarrier: used
/// in composite mode only, as the 1802 manual says) and fixed peaking coils
/// whose response the manual does not give; the chroma goes through the
/// band-pass amplifier of the HA11247 decoder (no delay line: NTSC). Picture
/// size, bandwidth, the Q of the trap and of the band-pass and the white
/// point are not in the manual: estimates (NTSC video bandwidth 4.2 MHz, Q
/// of the TDA356x application circuits).
pub static C1702: Monitor = Monitor {
    color: Some(Color::Ntsc),
    phosphor: None,
    // 13" (330 mm) viewable, 4:3
    screen_width: 264.0,
    screen_height: 198.0,
    overscan: 0.0,
    triad_pitch: 0.64,
    stripe: 0.25,
    slot_pitch: 0.8,
    bridge: 0.12,
    beam_sigma: (0.26, 0.42),
    glow: (0.05, 2.0),
    inputs: &[Input::LumaChroma, Input::Composite],
    knobs: KNOBS_NTSC,
    if_filter: None,
    luma_bandwidth: [4_200_000.0, 4_200_000.0],
    luma_peaking: None,
    peak: None,
    luma_trap: Some(Trap { q: 2.0, depth: 0.0, f0: None }),
    comb: None,
    chroma_filter: ChromaFilter::Tuned(3.0),
    chroma_bandpass: true,
    chroma_bandwidth: 1_300_000.0,
    delay_line: 0.0,
    white_point: None,
    tolerance: &TOLERANCE_MONITOR,
};

/// Commodore 1084S-P for NTSC countries (Commodore service manual 1084S-P,
/// July 1988, PN-314890-01: Philips/Magnavox CM8500 family, 115 V, 60 Hz):
/// 13", 90°, tube of the M34EAQ series (slotted mask, 0.42 mm, useful
/// screen 280.8 × 210.6 mm: Philips data handbook T08); RGB amplifier of 8
/// MHz; a 1H delay-line comb filter for the composite input, with a "COMB
/// DEFEAT" switch on the back, and a 3.58 MHz trap (S533) that takes over
/// when the comb is off; front controls brightness, contrast, color, hue and
/// sharpness, which sets the peaking stage TS544 (6.8 µH, 680 pF, 220 Ω:
/// 2.34 MHz, Q about 0.45). Not in the manual (estimates): the gain of the
/// peaking at the centre of the knob (+3 dB), the Q of the chroma
/// band-pass and of the trap, the composite bandwidth, the white point.
pub static C1084S_P_NTSC: Monitor = Monitor {
    color: Some(Color::Ntsc),
    phosphor: None,
    screen_width: 280.8,
    screen_height: 210.6,
    overscan: 0.0,
    triad_pitch: 0.42,
    stripe: 0.25,
    slot_pitch: 0.8,
    bridge: 0.12,
    beam_sigma: (0.26, 0.42),
    glow: (0.05, 2.0),
    inputs: &[Input::LumaChroma, Input::Composite],
    knobs: &[Knob::Brightness, Knob::Contrast, Knob::Color, Knob::Tint, Knob::Sharpness],
    if_filter: None,
    luma_bandwidth: [8_000_000.0, 8_000_000.0],
    luma_peaking: None,
    peak: Some(Peak { f0: 2_340_000.0, q: 0.45, gain: 0.41 }),
    luma_trap: Some(Trap { q: 2.0, depth: 0.0, f0: None }),
    comb: Some(227.5),
    chroma_filter: ChromaFilter::Tuned(2.0),
    chroma_bandpass: false,
    chroma_bandwidth: 1_300_000.0,
    delay_line: 0.0,
    white_point: None,
    tolerance: &TOLERANCE_MONITOR,
};

/// Sony Trinitron KV-1311CR, a 13" NTSC TV of about 1985 (Sams Photofact
/// 2393-2, 3-86; owner's manual): tube A34JHS10X, an aperture grille of 0.37
/// mm (the Sony PVM-1390 with the same tube); tuner, one SAW and the
/// CX20014A detector; a 1H glass delay-line comb filter (T351, DL351), then
/// the chroma band-pass T352 into the CX848 decoder, whose "sharp Y amp"
/// has a fixed peaking network (100 µH, 39 pF: about 2.5 MHz); controls
/// picture, bright, color and hue, no sharpness; inputs antenna and video.
/// The SAW is known only by Sony's part number: the response is that of an
/// NTSC intercarrier SAW (EPCOS M1967M). White point 9300 K and 5 MHz of
/// luma bandwidth are those of the PVM-1390; the picture size (13" at
/// 4:3), the overscan ("slight overscan" in the service procedure: 7%, the
/// "normal scan" of Sony's PVM-1341), the gain and Q of the peaking and the
/// Q of the band-pass are estimates.
pub static SONY_KV1311CR: Monitor = Monitor {
    color: Some(Color::Ntsc),
    phosphor: None,
    screen_width: 264.0,
    screen_height: 198.0,
    overscan: TV_OVERSCAN,
    triad_pitch: 0.37,
    stripe: 0.25,
    slot_pitch: 0.8,
    bridge: 0.0,
    beam_sigma: (0.26, 0.42),
    glow: (0.05, 2.0),
    inputs: &[Input::Rf, Input::Composite],
    knobs: KNOBS_NTSC,
    if_filter: Some(&SAW_M),
    luma_bandwidth: [5_000_000.0, 5_000_000.0],
    luma_peaking: None,
    peak: Some(Peak { f0: 2_550_000.0, q: 1.0, gain: 0.41 }),
    luma_trap: None,
    comb: Some(227.5),
    chroma_filter: ChromaFilter::Tuned(3.0),
    chroma_bandpass: false,
    chroma_bandwidth: 1_300_000.0,
    delay_line: 0.0,
    white_point: Some(9300.0),
    tolerance: &TOLERANCE_TV,
};

/// Sontec CNT-4442 B, an Argentine PAL-N TV (single standard; the luma and
/// chroma schematic in the DsTecnologia repair course, lesson 14): LA7520
/// IF and detector, a ceramic 3.58 MHz luma trap (the Murata TPS3.58 type:
/// about 27 dB, very narrow), TDA3562A PAL decoder with a glass chroma
/// delay line. The chroma band-pass before the decoder is C207 82 pF in
/// series into L201 5.6 µH ∥ C209 100 pF ∥ R210 1 kΩ: tuned to 5 MHz, not
/// to 3.58 (the course's author thinks the maker kept the PAL B/G values),
/// so the subcarrier comes in on its slope, with its upper sideband
/// stronger than the lower one. The PAL-N delay line is 63.930 µs (the
/// only figure found, in Thomson's patent US 5,374,962), 229 subcarrier
/// cycles: 0.42 µs longer than a line of the Drean C64 (227.5 cycles), so
/// the decoder averages each line with the chroma of the previous one 3.4
/// pixels to the left. Tube, screen, overscan and SAW are not documented:
/// the tube of the CP90 (36 cm) and the NTSC/N SAW of the Sony are
/// estimates; the white point is BT.470's for N/PAL (illuminant C, about
/// 6774 K), the luma bandwidth the 4.2 MHz of system N.
pub static SONTEC_CNT4442: Monitor = Monitor {
    color: Some(Color::PalN),
    phosphor: None,
    screen_width: 284.5,
    screen_height: 213.4,
    overscan: TV_OVERSCAN,
    triad_pitch: 0.52,
    stripe: 0.25,
    slot_pitch: 0.8,
    bridge: 0.12,
    beam_sigma: (0.26, 0.42),
    glow: (0.05, 2.0),
    inputs: &[Input::Rf],
    knobs: KNOBS_PAL,
    if_filter: Some(&SAW_M),
    luma_bandwidth: [4_200_000.0, 4_200_000.0],
    luma_peaking: None,
    peak: None,
    luma_trap: Some(Trap { q: 20.0, depth: 0.045, f0: None }),
    comb: None,
    chroma_filter: ChromaFilter::Network { series_c: 82e-12, l: 5.6e-6, shunt_c: 100e-12, r: 1e3 },
    chroma_bandpass: false,
    chroma_bandwidth: 1_000_000.0,
    delay_line: 229.0,
    white_point: Some(6774.0),
    tolerance: &TOLERANCE_TV,
};

/// Commodore 1900 M, a green monochrome monitor (the Commodore DM602, a
/// Philips BM7502 "Computer Monitor 80": user manual): 12", 90°, composite
/// input, video bandwidth "> 20 MHz", "> 850 lines in centre", brightness
/// and contrast knobs. Tube Philips M31-344GH/PD (photographed in one):
/// phosphor GH, P31 green, medium-short persistence (10 µs to 1 ms: no
/// trail from frame to frame), x 0.265 y 0.550; useful screen at least 257
/// × 195 mm (Philips data handbook T16, 1987). No trap and no decoder: the
/// C64's subcarrier shows as a fine pattern over the colored areas. The
/// thin beam (about 0.3 mm for 850 lines) is an estimate.
pub static C1900: Monitor = Monitor {
    color: None,
    phosphor: Some((0.265, 0.550)),
    screen_width: 257.0,
    screen_height: 195.0,
    overscan: 0.0,
    triad_pitch: 0.0,
    stripe: 0.25,
    slot_pitch: 0.8,
    bridge: 0.0,
    beam_sigma: (0.16, 0.26),
    glow: (0.05, 2.0),
    inputs: &[Input::Composite],
    knobs: KNOBS_MONO,
    if_filter: None,
    luma_bandwidth: [20_000_000.0, 20_000_000.0],
    luma_peaking: None,
    peak: None,
    luma_trap: None,
    comb: None,
    chroma_filter: ChromaFilter::Tuned(2.0),
    chroma_bandpass: false,
    chroma_bandwidth: 1_300_000.0,
    delay_line: 0.0,
    white_point: None,
    tolerance: &TOLERANCE_MONITOR,
};

/// Overscan of the TVs, on each axis: 7%, the "normal scan" of Sony's
/// PVM-1341 monitor (the TV service manuals only say "slight overscan").
pub const TV_OVERSCAN: f64 = 0.07;

/// IF response of a PAL B/G intercarrier SAW filter, EPCOS K2966M
/// (datasheet: picture carrier 38.9 MHz on the Nyquist slope, -5.6 dB;
/// color carrier 34.47 MHz, -3.1 dB; sound shelf 33.4-32.4 MHz, about -20
/// dB; adjacent carriers 31.9 and 40.4 MHz, -58 dB; the other points read
/// from its curve).
pub static SAW_BG: IfFilter = IfFilter {
    picture: 38.9,
    points: &[
        (30.0, -60.0), (31.9, -58.0), (32.4, -20.4), (33.4, -19.3), (34.2, -8.0),
        (34.47, -3.1), (35.0, 0.0), (38.0, 0.0), (38.5, -2.3), (38.9, -5.6),
        (39.1, -8.0), (40.2, -40.0), (45.0, -42.0),
    ],
    detector: None,
};

/// IF response of the CTX-E: its LC filter as the service manual draws it
/// (fig. 3, the curve the alignment aims at: traps at 31.9, 33.4, 40.4 and
/// 41.9 MHz, the top at 36.5 MHz, the picture carrier 38.9 MHz at about half
/// of it), read off the drawing every 0.2 MHz; the traps, drawn at zero,
/// are taken as -40 dB. Then the network at the detector's output (5156 12
/// µH, 3159 180 Ω with 2157 82 pF, into 3149 2.2 kΩ): about +5 dB at the
/// subcarrier, where the IF filter has taken 6 dB off the chroma.
pub static IF_CTXE: IfFilter = IfFilter {
    picture: 38.9,
    points: &[
        (30.0, -40.0), (31.9, -40.0), (32.9, -28.2), (33.4, -40.0), (33.6, -28.2),
        (33.8, -20.6), (34.0, -13.0), (34.2, -8.9), (34.4, -6.3), (34.6, -4.7),
        (34.8, -3.3), (35.0, -2.3), (35.2, -1.5), (35.4, -0.9), (35.6, -0.4),
        (35.8, -0.2), (36.0, -0.1), (36.5, 0.0), (37.0, -0.1), (37.2, -0.1),
        (37.4, -0.3), (37.6, -0.6), (37.8, -1.1), (38.0, -1.6), (38.2, -2.2),
        (38.4, -2.9), (38.6, -4.0), (38.8, -5.5), (39.0, -7.1), (39.2, -9.4),
        (39.4, -12.3), (39.6, -16.1), (39.8, -22.5), (40.0, -30.8), (40.4, -40.0),
        (41.1, -32.0), (41.9, -40.0), (45.0, -40.0),
    ],
    detector: Some(Detector { l: 12e-6, r: 180.0, c: 82e-12, load: 2.2e3 }),
};

/// IF response of an NTSC (M/N) intercarrier SAW filter, EPCOS M1967M
/// (datasheet, relative to 44 MHz: picture carrier 45.75 MHz -6 dB, color
/// carrier 42.17 MHz -1 dB, 41.92 MHz -3 dB, 41.67 MHz -7.4 dB, sound
/// carrier 41.25 MHz -19.1 dB, adjacent picture 39.75 MHz -62 dB, adjacent
/// sound 47.25 MHz -56 dB, sidelobes -46 and -47 dB). The Nyquist slope
/// between them is drawn linear in amplitude, from 45.0 to 46.5 MHz (0.75
/// MHz of vestigial sideband), as the Nyquist condition wants.
pub static SAW_M: IfFilter = IfFilter {
    picture: 45.75,
    points: &[
        (35.0, -46.0), (39.75, -62.0), (40.5, -46.0), (41.25, -19.1), (41.67, -7.4),
        (41.92, -3.0), (42.17, -1.0), (44.0, 0.0), (45.0, 0.0), (45.375, -2.5),
        (45.75, -6.0), (46.125, -12.0), (46.5, -40.0), (47.25, -56.0), (55.0, -47.0),
    ],
    detector: None,
};

impl Monitor {
    /// The picture on the screen, mm: the visible screen and the overscan.
    fn picture(&self) -> (f64, f64) {
        (self.screen_width / (1.0 - self.overscan), self.screen_height / (1.0 - self.overscan))
    }

    /// Width of a C64 pixel of signal `sig` on the screen, mm: the active
    /// line fills the width of the picture.
    pub fn pixel_width(&self, sig: &Signal) -> f64 {
        self.picture().0 / (sig.active_line() * sig.dot_clock())
    }

    /// Distance between two lines of signal `sig`, mm: the lines of a field
    /// (288 in 625-line sets, 242.5 in 525-line ones) fill the height of the
    /// picture.
    pub fn line_pitch(&self, sig: &Signal) -> f64 {
        self.picture().1 / sig.picture_lines()
    }

    /// The part of the framebuffer (`WIDTH` × `rows` pixels, centred) on the
    /// screen, as fractions of its width and height: all of it on a
    /// monitor, less on an overscanning TV.
    pub fn visible(&self, sig: &Signal, rows: usize) -> (f64, f64) {
        let w = self.screen_width / self.pixel_width(sig) / WIDTH as f64;
        let h = self.screen_height / self.line_pitch(sig) / rows as f64;
        (w.min(1.0), h.min(1.0))
    }

    /// Accepts the signal of a machine of `standard`.
    pub fn shows(&self, standard: Standard) -> bool {
        self.color.is_none_or(|c| c == Color::of(standard))
    }
}

// ── Colors ───────────────────────────────────────────────────────────────────

/// Luma and chroma of the 16 colors, in Pepto's units (luma 0-256), on
/// even or odd raster lines, with the lumas of the chip (`Chip::luma`: 5
/// levels on the first revisions). PAL and PAL-N: the two lines have the
/// chroma rotated by ∓ODD_LINE_PHASE/2, with the amplitude raised so that
/// their average (the PAL delay line) is exactly the colodore vector.
/// NTSC: the colodore vector on every line (the 6567 is assumed to have the
/// chroma of the 6569: no measurements of its phases were found, and NTSC
/// monitors have a tint control).
pub fn palette_yuv(odd_line: bool, sig: &Signal) -> [[f64; 3]; 16] {
    let half = if sig.pal() { (ODD_LINE_PHASE / 2.0).to_radians() } else { 0.0 };
    let turn = if odd_line { half } else { -half };
    let luma = sig.chip.luma();
    std::array::from_fn(|i| {
        let y = 8.0 * luma[i] as f64;
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

/// Palette index of an ARGB pixel: the color of `palette` it is, or the
/// nearest one (frames drawn by the VIC only have palette colors).
pub fn palette_index(argb: u32, palette: &[u32; 16]) -> usize {
    let rgb = |c: u32| [(c >> 16 & 0xFF) as i32, (c >> 8 & 0xFF) as i32, (c & 0xFF) as i32];
    let p = rgb(argb);
    (0..16)
        .min_by_key(|&i| rgb(palette[i]).iter().zip(&p).map(|(a, b)| (a - b) * (a - b)).sum::<i32>())
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

/// Largest number of sample phases within 4 pixels (PAL: 9).
pub const LUMA_PHASES: usize = 9;
/// Pixels that contribute to a luma sample: from LUMA_FIRST (the past) to
/// LUMA_FIRST + LUMA_TAPS - 1 (the future, through the delay compensation).
pub const LUMA_TAPS: usize = 12;
pub const LUMA_FIRST: i32 = -9;

/// Weights of the pixels around a sample in the luma output of the VIC-II,
/// for each of the sample phases (9 PAL, 7 NTSC). Sample n lies in pixel
/// floor((n + 0.5) × 4/phases) and its phase is n mod phases; weight j multiplies
/// pixel p + LUMA_FIRST + j. The response is shifted so that a step
/// reaches half height at the pixel boundary (the monitor's luma delay line
/// aligns luma and chroma).
pub fn luma_kernels(sig: &Signal) -> Vec<[f32; LUMA_TAPS]> {
    let r = SecondOrder::vic_luma();
    let t50 = r.time_to(0.5);
    let s = |x: f64| r.step(x + t50);
    (0..sig.phases).map(|phase| {
        let pos = (phase as f64 + 0.5) * 4.0 / sig.phases as f64;
        let tau = pos - pos.floor();
        let w: [f64; LUMA_TAPS] = std::array::from_fn(|i| {
            let j = (LUMA_FIRST + i as i32) as f64;
            s(tau - j) - s(tau - j - 1.0)
        });
        let sum: f64 = w.iter().sum();
        w.map(|x| (x / sum) as f32)
    }).collect()
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
fn fir_from_response(sig: &Signal, half: usize, resp: impl Fn(f64) -> (f64, f64), tau: f64) -> Vec<f64> {
    let fs = sig.sample_rate;
    const M: usize = 4096;
    let taps = |k: i64| -> f64 {
        let sum: f64 = (0..M)
            .map(|i| {
                let f = (i as f64 + 0.5) * fs / 2.0 / M as f64;
                let (re, im) = resp(f);
                let w = 2.0 * PI * f * (k as f64 / fs + tau);
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
fn zero_phase_fir(sig: &Signal, mag: impl Fn(f64) -> f64) -> [f32; FIR_LEN] {
    let h = fir_from_response(sig, FIR_HALF, |f| (mag(f), 0.0), 0.0);
    std::array::from_fn(|i| h[i] as f32)
}

/// Luma amplifier of the monitor: second-order Butterworth magnitude at its
/// bandwidth for the input.
pub fn luma_fir(sig: &Signal, m: &Monitor, input: Input) -> [f32; FIR_LEN] {
    let fc = m.luma_bandwidth[(input != Input::LumaChroma) as usize];
    zero_phase_fir(sig, |f| 1.0 / (1.0 + (f / fc).powi(4)).sqrt())
}

/// Low-pass of the demodulated chroma: second-order Butterworth magnitude
/// at the monitor's chroma bandwidth. When the separate chroma input goes
/// through the tuned band-pass its effect on the demodulated chroma is here
/// too (with a composite signal the band-pass is done on the signal): a
/// circuit of Q tuned to FSC passes FSC ± Δ with 1 / sqrt(1 + (2QΔ/FSC)²).
pub fn chroma_fir(sig: &Signal, m: &Monitor, input: Input) -> [f32; FIR_LEN] {
    let fc = m.chroma_bandwidth;
    let band = m.chroma_bandpass && input == Input::LumaChroma;
    let q = match m.chroma_filter {
        ChromaFilter::Tuned(q) => q,
        // Only the tuned band-passes are used on a separate chroma input
        ChromaFilter::Network { .. } | ChromaFilter::SeriesTrap { .. } => 1.0,
    };
    zero_phase_fir(sig, |f| {
        let lp = 1.0 / (1.0 + (f / fc).powi(4)).sqrt();
        if band { lp / (1.0 + (2.0 * q * f / sig.fsc).powi(2)).sqrt() } else { lp }
    })
}

/// Kernel (SEP_LEN taps around the sample) of the analog second-order
/// section (s'² + b1·s' + 1) / (s'² + a1·s' + 1), s' = s/ω0 with ω0 at `f0`
/// Hz, from its complex response: gain and phase of the circuit, without
/// the warping of a bilinear transform (the subcarrier is only a quarter of
/// the sampling rate).
fn notch_section(sig: &Signal, f0: f64, b1: f64, a1: f64) -> Vec<f64> {
    fir_from_response(sig, SEP_HALF, move |f| {
        let x = f / f0;
        let (nr, ni) = (1.0 - x * x, b1 * x);
        let (dr, di) = (1.0 - x * x, a1 * x);
        let d = dr * dr + di * di;
        ((nr * dr + ni * di) / d, (ni * dr - nr * di) / d)
    }, 0.0)
}

/// Luma trap: notch of Q with gain `depth` at `f0` (the subcarrier).
fn trap(sig: &Signal, t: Trap) -> Vec<f64> {
    notch_section(sig, t.f0.unwrap_or(sig.fsc), t.depth / t.q, 1.0 / t.q)
}

/// Luma peaking around `p.f0` of `gain`: 1 + gain · jωω0/Q / (ω0² - ω² +
/// jωω0/Q), from its complex response.
fn peak(sig: &Signal, p: Peak, gain: f64) -> Vec<f64> {
    fir_from_response(sig, SEP_HALF, move |f| {
        let x = f / p.f0;
        // Band-pass x/Q j / (1 - x² + x/Q j)
        let (dr, di) = (1.0 - x * x, x / p.q);
        let d = dr * dr + di * di;
        let (br, bi) = (di * di / d, di * dr / d);
        (1.0 + gain * br, gain * bi)
    }, 0.0)
}

/// Complex response of a set's chroma filter at `f` Hz, for a signal of
/// subcarrier `fsc`.
fn chroma_filter_response(filter: ChromaFilter, fsc: f64, f: f64) -> (f64, f64) {
    match filter {
        ChromaFilter::Tuned(q) => {
            // x/Q j / (1 - x² + x/Q j), 1 at the subcarrier
            let x = f / fsc;
            let (dr, di) = (1.0 - x * x, x / q);
            let d = dr * dr + di * di;
            (di * di / d, di * dr / d)
        }
        ChromaFilter::Network { series_c, l, shunt_c, r } => {
            // Z / (Z + 1/(jωC)), Z the shunt L ∥ C ∥ R: with admittances,
            // jωC / (Y + jωC), Y = 1/R + jωC_s - j/(ωL)
            let w = 2.0 * PI * f.max(1.0);
            let (yr, yi) = (1.0 / r, w * shunt_c - 1.0 / (w * l) + w * series_c);
            let (nr, ni) = (0.0, w * series_c);
            let d = yr * yr + yi * yi;
            ((nr * yr + ni * yi) / d, (ni * yr - nr * yi) / d)
        }
        ChromaFilter::SeriesTrap { series_c, shunt_c, l, r, load_r, load_c } => {
            // Z / (r + 1/(jωC) + Z), Z the shunt: the branch jωL + 1/(jωC_s)
            // in parallel with the load. With admittances: Y = 1/branch +
            // 1/load_r + jω·load_c, Z = 1/Y
            let w = 2.0 * PI * f.max(1.0);
            let xb = w * l - 1.0 / (w * shunt_c);
            let (yr, yi) = (1.0 / load_r, w * load_c - 1.0 / xb);
            let yd = yr * yr + yi * yi;
            let (zr, zi) = (yr / yd, -yi / yd);
            let (dr, di) = (r + zr, zi - 1.0 / (w * series_c));
            let d = dr * dr + di * di;
            ((zr * dr + zi * di) / d, (zi * dr - zr * di) / d)
        }
    }
}

/// Luma peaking of time constant `tau`: 2(1 + sτ) / (2 + sτ), unity gain at
/// DC and 2 at high frequencies, from its complex response.
fn peaking(sig: &Signal, tau: f64) -> Vec<f64> {
    fir_from_response(sig, SEP_HALF, move |f| {
        let wt = 2.0 * PI * f * tau;
        // 2(1 + jwt) / (2 + jwt)
        let d = 4.0 + wt * wt;
        (2.0 * (2.0 + wt * wt) / d, 2.0 * wt / d)
    }, 0.0)
}

/// Video response of the TV's IF filter: the C64's modulator sends both
/// sidebands (no vestigial filter), which reach the IF at the picture
/// carrier ∓ f; the synchronous demodulator adds them, and the Nyquist
/// slope makes the sum flat at low frequencies.
fn if_response(sig: &Signal, filter: &IfFilter) -> Vec<f64> {
    let points = filter.points;
    let picture = filter.picture;
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
    let video = |f: f64| at(picture - f / 1e6) + at(picture + f / 1e6);
    let Some(net) = filter.detector else {
        return fir_from_response(sig, SEP_HALF, |f| (video(f), 0.0), 0.0);
    };
    // The detector's network: Z / (jωL + Z), Z = (r + 1/(jωc)) ∥ load
    let det = move |f: f64| -> (f64, f64) {
        let w = 2.0 * PI * f.max(1.0);
        // Admittance of the shunt: 1/(r - j/(ωc)) + 1/load
        let (sr, si) = (net.r, -1.0 / (w * net.c));
        let s2 = sr * sr + si * si;
        let (yr, yi) = (sr / s2 + 1.0 / net.load, -si / s2);
        // Z = 1/Y; H = 1 / (1 + jωL·Y)
        let (dr, di) = (1.0 - w * net.l * yi, w * net.l * yr);
        let d = dr * dr + di * di;
        (dr / d, -di / d)
    };
    // Delay at low frequencies: from the phase at 100 kHz
    let p = det(1e5);
    let tau = -p.1.atan2(p.0) / (2.0 * PI * 1e5);
    fir_from_response(sig, SEP_HALF, move |f| {
        let (re, im) = det(f);
        (re * video(f), im * video(f))
    }, tau)
}

/// Luma of the C64's composite and RF output: the luma network in the RF
/// modulator (Service Manual, modulator 251696): the emitter follower drives
/// L2 ∥ C3 220 pF in series to node X, with C4 330 pF to ground, then R8
/// 150 Ω to the output, loaded by R11 150 Ω, and R9 1 kΩ / R10 120 Ω to the
/// RF modulator. L2 is adjustable: here tuned to the subcarrier (5.8 µH
/// with C3), as it has to be to keep the chroma band clean (the drawn
/// 10 µH would put the notch at 3.4 MHz). Complex response from the nodal
/// equations; the delay at low frequencies is taken away.
fn modulator_luma(sig: &Signal) -> Vec<f64> {
    let l2 = 1.0 / ((2.0 * PI * sig.fsc).powi(2) * 220e-12);
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
    fir_from_response(sig, SEP_HALF, norm, tau)
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
pub fn modulator_fir(sig: &Signal) -> [f32; SEP_LEN] {
    let h = modulator_luma(sig);
    std::array::from_fn(|i| h[i] as f32)
}

/// The TV's IF filter (RF input), SEP_LEN taps around the sample; None
/// without one.
pub fn if_fir(sig: &Signal, crt: &Crt) -> Option<[f32; SEP_LEN]> {
    let filter = crt.model.monitor().if_filter.filter(|_| crt.input == Input::Rf)?;
    let h = if_response(sig, filter);
    Some(std::array::from_fn(|i| h[i] as f32))
}

/// The set's luma peaking (shelf and band), SEP_LEN taps around the sample.
fn peakings(sig: &Signal, crt: &Crt) -> Vec<f64> {
    let m = crt.model.monitor();
    let mut h = vec![0f64; SEP_LEN];
    h[SEP_HALF] = 1.0;
    if let Some(tau) = m.luma_peaking {
        h = convolve(&h, SEP_HALF, &peaking(sig, tau), SEP_HALF);
    }
    if let Some(p) = m.peak {
        h = convolve(&h, SEP_HALF, &peak(sig, p, crt.peak_gain()), SEP_HALF);
    }
    h
}

/// Luma input of the set, after the IF filter (RF), SEP_LEN taps around
/// the sample: the trap (composite signal, unless the comb filter does the
/// separation), the peaking.
pub fn luma_input_fir(sig: &Signal, crt: &Crt) -> [f32; SEP_LEN] {
    let m = crt.model.monitor();
    let mut h = peakings(sig, crt);
    if crt.input != Input::LumaChroma && !crt.comb_active() {
        if let Some(t) = m.luma_trap {
            h = convolve(&h, SEP_HALF, &trap(sig, t), SEP_HALF);
        }
    }
    let dc: f64 = h.iter().sum();
    std::array::from_fn(|i| (h[i] / dc) as f32)
}

/// Kernel of the chroma filter, SEP_LEN taps around the sample, and its
/// gain at the subcarrier (magnitude).
fn chroma_band(sig: &Signal, filter: ChromaFilter) -> (Vec<f64>, f64) {
    let fsc = sig.fsc;
    let resp = move |f: f64| chroma_filter_response(filter, fsc, f);
    // fir_from_response takes the DC gain as 1: without DC, the raw sum
    let fs = sig.sample_rate;
    const M: usize = 4096;
    let h: Vec<f64> = (-(SEP_HALF as i64)..=SEP_HALF as i64).map(|k| {
        let sum: f64 = (0..M).map(|i| {
            let f = (i as f64 + 0.5) * fs / 2.0 / M as f64;
            let (re, im) = resp(f);
            let w = 2.0 * PI * f * k as f64 / fs;
            re * w.cos() - im * w.sin()
        }).sum();
        let x = k.unsigned_abs() as f64 / (SEP_HALF + 1) as f64;
        let window = if x < 0.75 { 1.0 } else { 0.5 * (1.0 + (PI * (x - 0.75) / 0.25).cos()) };
        sum / M as f64 * window
    }).collect();
    let (re, im) = response_at_fsc(&h);
    (h, (re * re + im * im).sqrt())
}

/// Complex response at the subcarrier, SAMPLE_RATE/4, of a kernel of
/// SEP_LEN taps: there the taps weigh 1, 0, -1, 0... and 0, 1, 0, -1...
fn response_at_fsc(h: &[f64]) -> (f64, f64) {
    let (mut re, mut im) = (0.0, 0.0);
    for (i, &x) in h.iter().enumerate() {
        let k = (i as i64 - SEP_HALF as i64).rem_euclid(4);
        re += x * [1.0, 0.0, -1.0, 0.0][k as usize];
        im -= x * [0.0, 1.0, 0.0, -1.0][k as usize];
    }
    (re, im)
}

/// The chroma path of a composite signal: IF filter (RF) and band-pass.
fn chroma_path(sig: &Signal, crt: &Crt) -> (Vec<f64>, Vec<f64>) {
    let (h, _) = chroma_band(sig, crt.model.monitor().chroma_filter);
    let path = match if_fir(sig, crt) {
        Some(f) => {
            let f: Vec<f64> = f.iter().map(|&x| x as f64).collect();
            convolve(&h, SEP_HALF, &f, SEP_HALF)
        }
        None => h.clone(),
    };
    (h, path)
}

/// Phase that the chroma path (`chroma_path`) gives the subcarrier,
/// radians: the burst goes through it too, and the decoder's oscillator
/// locks to it, so the demodulation turns by as much (0 with a band-pass
/// tuned to the subcarrier; 165° on the CNT-4442's, tuned to 5 MHz).
pub fn chroma_phase(sig: &Signal, crt: &Crt) -> f64 {
    if crt.input == Input::LumaChroma {
        return 0.0;
    }
    let (re, im) = response_at_fsc(&chroma_path(sig, crt).1);
    im.atan2(re)
}

/// Chroma out of a composite signal (after the IF filter with RF), SEP_LEN
/// taps around the sample: the band-pass, with the gain that makes the
/// whole path (IF filter and band-pass) unity at the subcarrier (the burst
/// sets the chroma gain: ACC).
pub fn chroma_input_fir(sig: &Signal, crt: &Crt) -> [f32; SEP_LEN] {
    let (h, path) = chroma_path(sig, crt);
    let (re, im) = response_at_fsc(&path);
    let g = (re * re + im * im).sqrt();
    std::array::from_fn(|i| (h[i] / g) as f32)
}

/// Comb filter: the chroma it takes out of the luma, SEP_LEN taps around the
/// sample, applied to half the difference of a line and the previous one:
/// the chroma band (at unity gain at the subcarrier, where the comb is
/// balanced), through the set's peaking.
pub fn comb_luma_fir(sig: &Signal, crt: &Crt) -> [f32; SEP_LEN] {
    let m = crt.model.monitor();
    let (band, g) = chroma_band(sig, m.chroma_filter);
    let unit: Vec<f64> = band.iter().map(|x| x / g).collect();
    let h = convolve(&peakings(sig, crt), SEP_HALF, &unit, SEP_HALF);
    std::array::from_fn(|i| h[i] as f32)
}

// ── Line delays ──────────────────────────────────────────────────────────────

/// A delay line of `cycles` subcarrier cycles, as a shift of the previous
/// line: its sample n + offset is the one delayed to sample n (the delay is
/// the line minus the offset). 0 when the delay is exactly a line.
fn line_delay_offset(sig: &Signal, cycles: f64) -> i32 {
    sig.line_samples() as i32 - (4.0 * cycles).round() as i32
}

/// Shift of the previous line in the set's comb filter.
pub fn comb_offset(sig: &Signal, m: &Monitor) -> i32 {
    m.comb.map_or(0, |c| line_delay_offset(sig, c))
}

/// Shift of the previous line in the set's PAL delay line: 0 for the PAL
/// C64 on a PAL B/G set (283.5 cycles, 63.943 µs, also the C64's line),
/// -6 samples (3.4 pixels) for the Drean on a PAL-N set (229 cycles,
/// 63.930 µs, against its 227.5).
pub fn delay_offset(sig: &Signal, m: &Monitor) -> i32 {
    if sig.pal() { line_delay_offset(sig, m.delay_line) } else { 0 }
}

// ── VIC-II jail bars ─────────────────────────────────────────────────────────

/// Jail bars: interference of the VIC-II's AEC and PHI0 on its own luma
/// output, the same on every line and in the border too (forum64.de,
/// "Streifenfix", and the LumaFix64 documentation: the edges of AEC make a
/// dark and a bright dot). Added to the luma, 0-256 units, for each pixel
/// of a character cell (8 pixels, one PHI0 cycle), measured on frame grabs
/// of real C64s (ikari's captures, before the fix): on an 8565 a dark dip
/// of one pixel on the first pixel of the cell and a bright peak on pixel
/// 4, about 3% of black to white peak to peak; on the NMOS chips (6569R5)
/// a broader pattern, about 5 pixels darker and 3 brighter, 2% peak to
/// peak, whose phase in the cell is assumed to be the same.
pub fn jail_bars(chip: Chip) -> [f64; 8] {
    let bars = if chip.hmos() {
        [-3.5, 0.0, 0.0, 0.0, 4.4, 0.0, 0.0, 0.0]
    } else {
        [-1.8, -1.8, -1.8, -1.8, -1.8, 3.0, 3.0, 3.0]
    };
    // No change of the average level
    let mean = bars.iter().sum::<f64>() / 8.0;
    bars.map(|b| b - mean)
}

// ── Monochrome ───────────────────────────────────────────────────────────────

/// The color of a phosphor (CIE xy) on the host display: linear sRGB, the
/// brightest channel at 1 (the part outside the sRGB gamut is clipped).
pub fn phosphor_rgb((x, y): (f64, f64)) -> [f64; 3] {
    let (cx, cy, cz) = (x / y, 1.0, (1.0 - x - y) / y);
    let rgb = [
        3.2406 * cx - 1.5372 * cy - 0.4986 * cz,
        -0.9689 * cx + 1.8758 * cy + 0.0415 * cz,
        0.0557 * cx - 0.2040 * cy + 1.0570 * cz,
    ].map(|c: f64| c.max(0.0));
    let max = rgb.iter().cloned().fold(0.0, f64::max);
    rgb.map(|c| c / max)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vic::C64_PALETTE;

    /// The PAL signal, for the tests of the PAL monitors.
    const FSC: f64 = 4_433_618.75;
    const SAMPLE_RATE: f64 = 4.0 * FSC;
    fn pal() -> Signal {
        Signal::of(Chip::Mos6569)
    }

    fn with(model: Model, input: Input) -> Crt {
        Crt { input, ..Crt::new(model) }
    }

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
        // model gives back exactly the palette of the VIC, for every chip
        // (the first revisions with their 5 lumas)
        for chip in Chip::ALL {
            let sig = Signal::of(chip);
            let (even, odd) = (palette_yuv(false, &sig), palette_yuv(true, &sig));
            for i in 0..16 {
                let avg: [f64; 3] = std::array::from_fn(|k| (even[i][k] + odd[i][k]) / 2.0);
                let rgb = yuv_to_rgb(avg[0], avg[1], avg[2]).map(|c| to_display(to_light(c)).round() as u32);
                let argb = 0xFF00_0000 | rgb[0] << 16 | rgb[1] << 8 | rgb[2];
                assert_eq!(argb, chip.palette()[i], "{chip:?} color {i}");
            }
        }
    }

    #[test]
    fn odd_lines_turn_the_hue() {
        let (even, odd) = (palette_yuv(false, &pal()), palette_yuv(true, &pal()));
        let angle = |c: [f64; 3]| c[2].atan2(c[1]).to_degrees();
        for i in [2, 3, 4, 5, 6, 7, 8, 9, 10, 13, 14] {
            let d = (angle(odd[i]) - angle(even[i]) + 360.0) % 360.0;
            assert!((d - ODD_LINE_PHASE).abs() < 1e-9, "color {i}: {d}");
        }
    }

    #[test]
    fn signal_timing() {
        let p = pal();
        assert!((p.dot_clock() - 7_881_988.9).abs() < 1.0);
        assert_eq!(p.samples, 907);
        // 504 pixels per line = 283.5 subcarrier cycles
        assert!((504.0 * p.fsc / p.dot_clock() - 283.5).abs() < 1e-9);
        assert!((p.pixel_aspect() - 0.9357).abs() < 1e-4);
        // NTSC: 8.18 MHz, 7 samples every 4 pixels, 520 pixels per line =
        // 227.5 subcarrier cycles, pixel aspect 0.75
        let n = Signal::of(Chip::Mos6567R8);
        assert!((n.dot_clock() - 8_181_818.2).abs() < 1.0);
        assert_eq!(n.samples, 706);
        assert!((520.0 * n.fsc / n.dot_clock() - 227.5).abs() < 1e-9);
        assert!((n.pixel_aspect() - 0.75).abs() < 1e-6);
        // Old NTSC: 512 pixels = 224 cycles, no flip from line to line
        let o = Signal::of(Chip::Mos6567R56A);
        assert!((512.0 * o.fsc / o.dot_clock() - 224.0).abs() < 1e-9);
        assert!(!o.line_flip() && n.line_flip() && p.line_flip());
        // PAL-N: the NTSC pixel timing with the PAL-N subcarrier, 227.5
        // cycles per line, and the 625-line geometry
        let pn = Signal::of(Chip::Mos6572);
        assert!(pn.pal() && pn.line_flip() && !n.pal());
        assert!((520.0 * pn.fsc / pn.dot_clock() - 227.5).abs() < 1e-9);
        // CPU clock = pixel clock / 8: 1,023,445 Hz (VICE: 1,023,440)
        assert!((pn.dot_clock() / 8.0 - 1_023_440.0).abs() < 10.0);
        assert!((pn.pixel_aspect() - 0.9008).abs() < 1e-4);
        // The monitor geometry agrees with the PAL pixel aspect
        let m = &C1084S_P1;
        assert!((m.pixel_width(&p) / m.line_pitch(&p) - p.pixel_aspect()).abs() < 0.005);
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
        let k = luma_kernels(&pal());
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
        let h = chroma_fir(&pal(), &C1084S_P1, Input::LumaChroma);
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
            let h = luma_fir(&pal(), m, input);
            assert!((gain(&h, 0.0) - 1.0).abs() < 1e-6);
            let g = gain(&h, bw);
            assert!((g - 0.707).abs() < 0.03, "{bw}: {g}");
        }
    }

    #[test]
    fn peaking_1901() {
        // +6 dB shelf: gain 1 at DC, about 2 well above 1.2 MHz
        let h = luma_input_fir(&pal(), &with(Model::C1901, Input::LumaChroma));
        assert!((gain(&h, 0.0) - 1.0).abs() < 1e-3);
        let g = gain(&h, 4_000_000.0);
        assert!(g > 1.8 && g < 2.05, "{g}");
        // No trap: the subcarrier stays in the luma with composite
        assert!(gain(&luma_input_fir(&pal(), &with(Model::C1901, Input::Composite)), FSC) > 1.8);
        // The 1084S has neither: luma/chroma passes as it is
        let flat = luma_input_fir(&pal(), &with(Model::C1084SP1, Input::LumaChroma));
        assert!((gain(&flat, 3_000_000.0) - 1.0).abs() < 1e-6);
    }

    #[test]
    fn chroma_band_pass_1901() {
        // Q 3.6 at 4.43 MHz: ±0.62 MHz, narrower than the 1.3 MHz of PAL
        let h = chroma_fir(&pal(), &C1901, Input::LumaChroma);
        let g = gain(&h, FSC / (2.0 * 3.6));
        assert!(g > 0.6 && g < 0.72, "{g}");
        // In composite the band-pass is done on the signal instead
        let c = chroma_fir(&pal(), &C1901, Input::Composite);
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
        let pal = Standard::Pal;
        assert_eq!(Model::parse("1084S", pal), Some(Model::C1084SP1));
        assert_eq!(Model::parse("1084s-d1", pal), Some(Model::C1084SD1));
        // The 1702 of the machine's color system: the PAL one is the 1701
        assert_eq!(Model::parse("1702", pal), Some(Model::C1701));
        assert_eq!(Model::parse("1702", Standard::Ntsc), Some(Model::C1702));
        assert_eq!(Model::parse("1701", Standard::Ntsc), Some(Model::C1701));
        assert_eq!(Model::parse("1703", pal), None);
        for model in Model::ALL {
            let standard = model.monitor().color.map_or(pal, |c| c.standard());
            assert_eq!(Model::parse(model.name(), standard), Some(model));
        }
        // 1084s and tv are the sets of the standard
        assert_eq!(Model::parse("1084s", Standard::Ntsc), Some(Model::C1084SPNtsc));
        assert_eq!(Model::parse("tv", pal), Some(Model::PhilipsCp90));
        assert_eq!(Model::parse("tv", Standard::NtscOld), Some(Model::SonyKv1311));
        assert_eq!(Model::parse("tv", Standard::PalN), Some(Model::SontecCnt4442));
    }

    #[test]
    fn luma_trap() {
        let h = luma_input_fir(&pal(), &with(Model::C1084SP1, Input::Composite));
        assert!((gain(&h, 0.0) - 1.0).abs() < 1e-6);
        assert!(gain(&h, FSC) < 0.005);
        assert!(gain(&h, 3_000_000.0) > 0.8);
        assert!(gain(&h, 1_000_000.0) > 0.9);
    }

    #[test]
    fn chroma_band_pass() {
        // Composite chroma: unity at the subcarrier, nothing at DC, the
        // band of a Q 2 circuit on the 1084S
        let h = chroma_input_fir(&pal(), &with(Model::C1084SP1, Input::Composite));
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
        let h = luma_input_fir(&pal(), &with(Model::PhilipsCp90, Input::Composite));
        let g = gain(&h, FSC);
        assert!((g - 0.5).abs() < 0.02, "{g}");
    }

    #[test]
    fn if_filter() {
        // Both sidebands through the Nyquist slope: flat at low frequencies
        let h: Vec<f32> = if_response(&pal(), &SAW_BG).iter().map(|&x| x as f32).collect();
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
        let rf = luma_input_fir(&pal(), &with(Model::PhilipsCp90, Input::Rf));
        assert!(gain(&rf, 4e6) < 0.6 && gain(&rf, 2e6) > 0.9);
    }

    #[test]
    fn modulator() {
        // The C64's luma in its composite output: flat at low frequencies,
        // a notch at the subcarrier, a bump near 2 MHz (nodal analysis:
        // +2.9 dB)
        let h = modulator_fir(&pal());
        assert!((gain(&h, 100e3) - 1.0).abs() < 0.02);
        assert!(gain(&h, FSC) < 0.05);
        let bump = gain(&h, 2e6);
        assert!(bump > 1.2 && bump < 1.5, "{bump}");
    }

    #[test]
    fn settings() {
        let tv = Crt::apply(None, "rf", Standard::Pal).unwrap().unwrap();
        assert_eq!((tv.model, tv.input), (Model::PhilipsCp90, Input::Rf));
        // A monitor after the TV: the RF input goes, luma/chroma comes
        let m = Crt::apply(Some(tv), "1901", Standard::Pal).unwrap().unwrap();
        assert_eq!((m.model, m.input), (Model::C1901, Input::LumaChroma));
        // Composite is kept across models
        let c = Crt::apply(Crt::apply(None, "composite", Standard::Pal).unwrap(), "tv", Standard::Pal).unwrap().unwrap();
        assert_eq!((c.model, c.input), (Model::PhilipsCp90, Input::Composite));
        assert_eq!(Crt::apply(Some(c), "off", Standard::Pal).unwrap(), None);
        assert!(Crt::apply(None, "1703", Standard::Pal).is_err());
        // RF on a monitor is caught by check
        let bad = Crt::apply(Crt::apply(None, "1084s", Standard::Pal).unwrap(), "rf", Standard::Pal).unwrap().unwrap();
        assert!(bad.check(Standard::Pal).is_err());
        // The monitor follows the machine: 1702 on NTSC, and a PAL monitor
        // on an NTSC C64 is refused
        let ntsc = Crt::apply(None, "lc", Standard::Ntsc).unwrap().unwrap();
        assert_eq!(ntsc.model, Model::C1702);
        assert!(ntsc.check(Standard::Ntsc).is_ok());
        assert!(Crt::new(Model::C1084SP1).check(Standard::Ntsc).is_err());
        assert!(ntsc.check(Standard::Pal).is_err());
    }

    #[test]
    fn geometry_setting() {
        // A share of the tolerances, kept across sets; on is all of them
        let c = Crt::apply(None, "1084s,geometry=40", Standard::Pal).unwrap().unwrap();
        assert_eq!(c.geometry, 40);
        assert!(c.describe().ends_with("geometry and convergence errors 40% of the tolerances"));
        let c = Crt::apply(Some(c), "cp90", Standard::Pal).unwrap().unwrap();
        assert_eq!(c.geometry, 40);
        assert_eq!(Crt::apply(Some(c), "geometry=on", Standard::Pal).unwrap().unwrap().geometry, 100);
        assert_eq!(Crt::apply(Some(c), "geometry=off", Standard::Pal).unwrap().unwrap().geometry, 0);
        assert!(Crt::apply(Some(c), "geometry=101", Standard::Pal).is_err());
        assert_eq!(Crt::new(Model::C1901).geometry, 0);
    }

    #[test]
    fn knobs() {
        let c = Crt::apply(None, "1702,composite,tint=-20,color=15", Standard::Ntsc).unwrap().unwrap();
        assert_eq!((c.model, c.input, c.knob(Knob::Tint), c.knob(Knob::Color)), (Model::C1702, Input::Composite, -20, 15));
        assert!((c.tint() + (0.2 * TINT_RANGE).to_radians()).abs() < 1e-12);
        assert!((c.color() - 1.15).abs() < 1e-12);
        assert_eq!(c.describe(), "Commodore 1702, composite input, color +15, tint -20");
        // Centre: no change
        let n = Crt::new(Model::C1084SP1);
        assert_eq!((n.brightness(), n.contrast(), n.color(), n.tint()), (0.0, 1.0, 1.0, 0.0));
        // PAL sets have no tint; out of range and unknown controls
        assert!(Crt::apply(Some(n), "tint=10", Standard::Pal).is_err());
        assert!(Crt::apply(Some(n), "contrast=101", Standard::Pal).is_err());
        assert!(Crt::apply(Some(n), "sharpness=10", Standard::Pal).is_err());
        // A model without the knob drops it, the others are kept
        let p = Crt::apply(Some(c), "contrast=30", Standard::Ntsc).unwrap().unwrap();
        let q = Crt::apply(Some(p), "1084s-p1", Standard::Ntsc).unwrap().unwrap();
        assert_eq!(q.knobs, [0, 30, 15, 0, 0]);
        // The NTSC 1084S keeps the tint and has sharpness
        let r = Crt::apply(Some(p), "1084s,sharpness=50", Standard::Ntsc).unwrap().unwrap();
        assert_eq!((r.model, r.knobs), (Model::C1084SPNtsc, [0, 30, 15, -20, 50]));
        assert!((r.peak_gain() - 1.5 * C1084S_P_NTSC.peak.unwrap().gain).abs() < 1e-12);
        // Switches
        let s = Crt::apply(Some(r), "comb=off,bars=off", Standard::Ntsc).unwrap().unwrap();
        assert!(!s.comb && !s.bars);
        assert!(Crt::apply(Some(q), "comb=off", Standard::Ntsc).is_err());
    }

    #[test]
    fn line_delays() {
        let sig = |c: Chip| Signal::of(c);
        // PAL C64 on a PAL B/G set: its line is the delay line
        assert_eq!(delay_offset(&sig(Chip::Mos6569), &C1084S_P1), 0);
        // Drean on the PAL-N set: 227.5 against 229 cycles, 6 samples
        assert_eq!(delay_offset(&sig(Chip::Mos6572), &SONTEC_CNT4442), -6);
        // NTSC comb: the 6567R8's line is 1H; the 6567R56A's is 8 pixels
        // shorter
        assert_eq!(comb_offset(&sig(Chip::Mos6567R8), &SONY_KV1311CR), 0);
        assert_eq!(comb_offset(&sig(Chip::Mos6567R56A), &SONY_KV1311CR), -14);
        assert_eq!(delay_offset(&sig(Chip::Mos6567R8), &SONY_KV1311CR), 0);
    }

    #[test]
    fn sontec_chroma_network() {
        // As built it resonates near 5 MHz: the subcarrier comes in on the
        // slope, lower than at 5 MHz, and turned
        let f = SONTEC_CNT4442.chroma_filter;
        let fsc = 3_582_056.25;
        let mag = |f0: f64| {
            let (re, im) = chroma_filter_response(f, fsc, f0);
            (re * re + im * im).sqrt()
        };
        assert!(mag(4.99e6) > 0.95 && mag(fsc) < 0.6, "{} {}", mag(4.99e6), mag(fsc));
        let (re, im) = chroma_filter_response(f, fsc, fsc);
        assert!(im.atan2(re).abs() > 0.5);
        // The decoder follows the phase: the tuned band-passes turn nothing
        let n = Signal::of(Chip::Mos6572);
        let tv = Crt::new(Model::SontecCnt4442);
        assert!((chroma_phase(&n, &tv) - im.atan2(re)).abs() < 0.3);
        assert!(chroma_phase(&pal(), &with(Model::C1084SP1, Input::Composite)).abs() < 1e-3);
    }

    #[test]
    fn display_size_1802() {
        // The 40 × 25 characters fill 247 × 160 mm (technical manual)
        let sig = pal();
        let (w, h) = (320.0 * C1802_PAL.pixel_width(&sig), 200.0 * C1802_PAL.line_pitch(&sig));
        assert!((w - 247.0).abs() < 10.0 && (h - 160.0).abs() < 2.0, "{w} x {h}");
    }

    #[test]
    fn trap_1701() {
        // 27 µH with 42 pF: the notch at 4.73 MHz, the subcarrier 13 dB down
        let h: Vec<f32> = trap(&pal(), C1701_PAL.luma_trap.unwrap()).iter().map(|&x| x as f32).collect();
        assert!(gain(&h, 4.73e6) < 0.02, "{}", gain(&h, 4.73e6));
        assert!((20.0 * gain(&h, FSC).log10() + 13.0).abs() < 1.5, "{}", gain(&h, FSC));
        assert!(gain(&h, 1e6) > 0.95);
    }

    #[test]
    fn ctxe_if_filter() {
        // The LC filter alone takes the chroma about 5 dB down (34.47 MHz at
        // 0.52 of the top, against 0.97 for the two sidebands at DC)
        let bare = IfFilter { detector: None, ..IF_CTXE };
        let h: Vec<f32> = if_response(&pal(), &bare).iter().map(|&x| x as f32).collect();
        assert!((gain(&h, FSC) - 0.54).abs() < 0.05, "{}", gain(&h, FSC));
        // With the detector's network: flat at low frequencies, a lift of
        // about 3 dB from 2.5 to 4 MHz, the chroma back to unity, and the
        // sound carrier (5.5 MHz) gone
        let h: Vec<f32> = if_response(&pal(), &IF_CTXE).iter().map(|&x| x as f32).collect();
        let db = |f: f64| 20.0 * gain(&h, f).log10();
        for f in [100e3, 500e3, 1e6] {
            assert!(db(f).abs() < 0.5, "{f}: {}", db(f));
        }
        for f in [2.5e6, 3e6, 3.5e6, 4e6] {
            assert!((1.5..4.5).contains(&db(f)), "{f}: {}", db(f));
        }
        assert!(db(FSC).abs() < 1.0, "{}", db(FSC));
        assert!(gain(&h, 5.5e6) < 0.1);
    }

    #[test]
    fn series_trap_chroma_filter() {
        // The CTX-E's chroma input: the peak at the subcarrier, the zero
        // where the coil resonates with 120 pF (1.6 MHz)
        let filter = PHILIPS_CTXE.chroma_filter;
        let mag = |f: f64| {
            let (re, im) = chroma_filter_response(filter, FSC, f);
            (re * re + im * im).sqrt()
        };
        let top = mag(FSC);
        for f in [4.2e6, 4.3e6, 4.55e6, 4.65e6] {
            assert!(mag(f) < top, "{f}");
        }
        let zero = 1.0 / (2.0 * PI * (82.5e-6f64 * 120e-12).sqrt());
        assert!(mag(zero) < 1e-3 * top);
        assert!(mag(1e6) < 0.1 * top && mag(3e6) < 0.5 * top);
    }

    #[test]
    fn ntsc_if_filter() {
        // NTSC M SAW: flat low frequencies through the Nyquist slope, the
        // color carrier 1 dB down in the IF, the sound carrier (4.5 MHz)
        // 19 dB
        let n = Signal::of(Chip::Mos6567R8);
        let h = if_response(&n, &SAW_M);
        let gain = |f: f64| {
            let (re, im) = h.iter().enumerate().fold((0.0, 0.0), |(re, im), (i, &c)| {
                let w = 2.0 * PI * f * (i as f64 - SEP_HALF as f64) / n.sample_rate;
                (re + c * w.cos(), im + c * w.sin())
            });
            (re * re + im * im).sqrt()
        };
        for f in [100e3, 500e3, 1e6, 2e6] {
            assert!((gain(f) - 1.0).abs() < 0.1, "{f}: {}", gain(f));
        }
        assert!((gain(n.fsc) - 0.89).abs() < 0.1, "{}", gain(n.fsc));
        assert!(gain(4.5e6) < 0.15);
    }

    #[test]
    fn overscan() {
        // Monitors show the whole screen, TVs about 93% of it
        assert_eq!(C1084S_P1.visible(&pal(), 284), (1.0, 1.0));
        let (w, h) = PHILIPS_CP90.visible(&pal(), 284);
        assert!(w > 0.85 && w < 0.99 && h > 0.85 && h < 0.99, "{w} {h}");
    }

    #[test]
    fn jail_bars_and_phosphor() {
        for chip in Chip::ALL {
            assert!(jail_bars(chip).iter().sum::<f64>().abs() < 1e-9);
        }
        // HMOS: a dip on the first pixel of the cell, a peak on pixel 4
        let b = jail_bars(Chip::Mos8565);
        assert!(b[0] < 0.0 && b[4] > 0.0);
        // P31 green: no red, some blue
        let [r, g, bl] = phosphor_rgb(C1900.phosphor.unwrap());
        assert!(r == 0.0 && g == 1.0 && bl > 0.05 && bl < 0.3, "{r} {g} {bl}");
    }

    #[test]
    fn monochrome_shows_every_standard() {
        let m = Crt::new(Model::C1900);
        for s in [Standard::Pal, Standard::Ntsc, Standard::NtscOld, Standard::PalN] {
            assert!(m.check(s).is_ok());
        }
        assert!(Crt::new(Model::SontecCnt4442).check(Standard::Pal).is_err());
        assert!(Crt::new(Model::SontecCnt4442).check(Standard::PalN).is_ok());
    }

    #[test]
    fn inputs() {
        assert!(Crt::new(Model::PhilipsCp90).input == Input::Rf);
        assert!(Crt::new(Model::C1901).input == Input::LumaChroma);
        assert!(Crt { input: Input::Rf, ..Crt::new(Model::C1084SP1) }.check(Standard::Pal).is_err());
        assert!(Crt { input: Input::LumaChroma, ..Crt::new(Model::PhilipsCp90) }.check(Standard::Pal).is_err());
        assert!(Crt { input: Input::Composite, ..Crt::new(Model::PhilipsCp90) }.check(Standard::Pal).is_ok());
    }

    #[test]
    fn palette_lookup() {
        for palette in [&C64_PALETTE, &crate::vic::C64_PALETTE_OLD] {
            for (i, &c) in palette.iter().enumerate() {
                assert_eq!(palette_index(c, palette), i);
            }
        }
        // Not a palette color: the nearest
        assert_eq!(palette_index(0xFF7A7C7B, &C64_PALETTE), 12);
    }
}
