// SPDX-License-Identifier: GPL-2.0-or-later
// Copyright (C) 2026 SampoSoft - Francesco Sampoli
// Derived from reSID 1.0 as shipped with VICE 3.10,
// Copyright (C) 2010 Dag Lem <resid@nimrod.no>.
// Ported to Rust and modified by SampoSoft in 2026; see CREDITS.md.

//! SID filter, mixer and output amplifier at transistor level: a port of
//! reSID's `filter8580new`, the filter VICE 3.10 uses for both models.
//!
//! The chip's stages are inverting op-amps modelled with the transfer
//! function measured on real chips. Integrators, summer, mixer and volume
//! amplifier are solved in 16-bit fixed point with precomputed tables
//! (Newton-Raphson plus bisection on the transistor current):
//! - 6581: integrators with the "snake" resistor and the VCR (EKV model)
//!   driven by the cutoff DAC, which is not linear; resonance 1/Q ~ ~res/8;
//!   each voice reaches the mixer with its DC component, so the volume
//!   shifts the output level and samples played via $D418 are audible;
//! - 8580: integrators with a ladder of parallel NMOS driven by the
//!   cutoff, resonance 1/Q = 2^((4 - res)/8).
//!
//! The tables (about ten MB per model) are built on first request and
//! kept for the whole process.

use std::sync::OnceLock;

use super::dac::build_dac_table;
use super::Model;

// ── Model parameters ─────────────────────────────────────────────────────────

/// 6581 op-amp transfer function (vi, vo), measured on
/// CAP1B/CAP1A of a 6581R4AR 0687 14.
const OPAMP_VOLTAGE_6581: [[f64; 2]; 35] = [
    [0.81, 10.31], // start of range
    [0.81, 10.31], // repeated point
    [2.40, 10.31],
    [2.60, 10.30],
    [2.70, 10.29],
    [2.80, 10.26],
    [2.90, 10.17],
    [3.00, 10.04],
    [3.10, 9.83],
    [3.20, 9.58],
    [3.30, 9.32],
    [3.50, 8.69],
    [3.70, 8.00],
    [4.00, 6.89],
    [4.40, 5.21],
    [4.54, 4.54], // operating point (vi = vo)
    [4.60, 4.19],
    [4.80, 3.00],
    [4.90, 2.30], // change of curvature
    [4.95, 2.03],
    [5.00, 1.88],
    [5.05, 1.77],
    [5.10, 1.69],
    [5.20, 1.58],
    [5.40, 1.44],
    [5.60, 1.33],
    [5.80, 1.26],
    [6.00, 1.21],
    [6.40, 1.12],
    [7.00, 1.02],
    [7.50, 0.97],
    [8.50, 0.89],
    [10.00, 0.81],
    [10.31, 0.81], // end of range
    [10.31, 0.81], // repeated point
];

/// 8580 op-amp transfer function, measured on a CSG
/// 8580R5 1690 25.
const OPAMP_VOLTAGE_8580: [[f64; 2]; 23] = [
    [1.30, 8.91],
    [1.30, 8.91],
    [4.76, 8.91],
    [4.77, 8.90],
    [4.78, 8.88],
    [4.785, 8.86],
    [4.79, 8.80],
    [4.795, 8.60],
    [4.80, 8.25],
    [4.805, 7.50],
    [4.81, 6.10],
    [4.815, 4.05],
    [4.82, 2.27],
    [4.825, 1.65],
    [4.83, 1.55],
    [4.84, 1.47],
    [4.85, 1.43],
    [4.87, 1.37],
    [4.90, 1.34],
    [5.00, 1.30],
    [5.10, 1.30],
    [8.91, 1.30],
    [8.91, 1.30],
];

/// 8580 resonance gain: feedback and input resistors selected by
/// bits 4-7 of $D417 (Rf|Rn / Ri, R4, R8, RC).
const RES_GAIN_8580: [f64; 16] = [
    1.4 / 1.0,
    ((1.4 * 15.3) / (1.4 + 15.3)) / 1.0,
    ((1.4 * 7.3) / (1.4 + 7.3)) / 1.0,
    ((1.4 * 4.7) / (1.4 + 4.7)) / 1.0,
    1.4 / 1.4,
    ((1.4 * 15.3) / (1.4 + 15.3)) / 1.4,
    ((1.4 * 7.3) / (1.4 + 7.3)) / 1.4,
    ((1.4 * 4.7) / (1.4 + 4.7)) / 1.4,
    1.4 / 2.0,
    ((1.4 * 15.3) / (1.4 + 15.3)) / 2.0,
    ((1.4 * 7.3) / (1.4 + 7.3)) / 2.0,
    ((1.4 * 4.7) / (1.4 + 4.7)) / 2.0,
    1.4 / 2.8,
    ((1.4 * 15.3) / (1.4 + 15.3)) / 2.8,
    ((1.4 * 7.3) / (1.4 + 7.3)) / 2.8,
    ((1.4 * 4.7) / (1.4 + 4.7)) / 2.8,
];

struct ModelInit {
    opamp_voltage: &'static [[f64; 2]],
    /// Analog swing of a voice and its DC level.
    voice_voltage_range: f64,
    voice_dc_voltage: f64,
    /// Filter capacitors.
    c: f64,
    // Transistor parameters
    vdd: f64,
    vth: f64,
    ut: f64,
    k: f64,
    u_cox: f64,
    wl_vcr: f64,
    wl_snake: f64,
    // Cutoff DAC (6581)
    dac_zero: f64,
    dac_scale: f64,
    dac_2r_div_r: f64,
    dac_term: bool,
}

const INIT_6581: ModelInit = ModelInit {
    opamp_voltage: &OPAMP_VOLTAGE_6581,
    voice_voltage_range: 1.5,
    voice_dc_voltage: 5.075, // 5 V + 1.5%
    c: 470e-12,
    vdd: 12.18, // 12 V + 1.5%
    vth: 1.31,
    ut: 26.0e-3,
    k: 1.0,
    u_cox: 20e-6,
    wl_vcr: 9.0 / 1.0,
    wl_snake: 1.0 / 115.0,
    dac_zero: 6.65,
    dac_scale: 2.63,
    dac_2r_div_r: 2.20,
    dac_term: false,
};

const INIT_8580: ModelInit = ModelInit {
    opamp_voltage: &OPAMP_VOLTAGE_8580,
    voice_voltage_range: 0.24,
    voice_dc_voltage: 4.7975, // 4.75 V + 1%
    c: 22e-9,
    vdd: 9.09, // 9 V + 1%
    vth: 0.80,
    ut: 26.0e-3,
    k: 1.0,
    u_cox: 100e-6,
    wl_vcr: 0.0,
    wl_snake: 0.0,
    dac_zero: 0.0,
    dac_scale: 0.0,
    dac_2r_div_r: 2.00,
    dac_term: true,
};

/// 8580 virtual ground (resistive divider): 4.75 V + 1%.
const VREF_8580: f64 = 4.7975;

// Offset into the summer (2-6 inputs) and mixer (0-7 inputs) tables
// by number of active inputs
const SUMMER_OFFSET: [usize; 5] = [0, 2 << 16, 5 << 16, 9 << 16, 14 << 16];
const SUMMER_SIZE: usize = 20 << 16;
const MIXER_OFFSET: [usize; 8] = [
    0, 1, 1 + (1 << 16), 1 + (3 << 16), 1 + (6 << 16), 1 + (10 << 16), 1 + (15 << 16), 1 + (21 << 16),
];
const MIXER_SIZE: usize = 1 + (28 << 16);

// ── Tables ───────────────────────────────────────────────────────────────────

#[derive(Clone, Copy, Default)]
struct Opamp {
    vx: u16,
    dvx: i16,
}

/// Tables and constants of a model.
pub struct ModelFilter {
    k_vddt: i32,
    voice_scale_s14: i32,
    voice_dc: i32,
    filter_gain: i32,
    vo_n16: f64,
    /// Inverse op-amp function: capacitor voltage -> vx.
    opamp_rev: Vec<u16>,
    summer: Vec<u16>,
    /// 16 volume tables, 16 resonance tables, indexed [n << 16 | vi].
    gain: Vec<u16>,
    resonance: Vec<u16>,
    mixer: Vec<u16>,
    /// Cutoff DAC output (11 bits).
    f0_dac: Vec<u16>,
    // 6581: VCR and "snake"
    vcr_kvg: Vec<u16>,
    vcr_n_ids_term: Vec<u16>,
    n_snake: i32,
    // 8580: cutoff NMOS ladder
    n_param: i32,
}

static TABLES: [OnceLock<ModelFilter>; 2] = [OnceLock::new(), OnceLock::new()];

/// Model tables, built on first request.
pub fn tables(model: Model) -> &'static ModelFilter {
    TABLES[model as usize].get_or_init(|| build(model))
}

/// Piecewise cubic spline interpolation (reSID, spline.h): each segment
/// between p1 and p2 uses the slopes given by p0 and p3; the repeated
/// endpoints make the curve pass through the first and last point.
fn interpolate(p: &[[f64; 2]], plot: &mut impl FnMut(f64, f64), res: f64) {
    let pn = p.len() - 1;
    let mut i = 0;
    while i + 2 != pn {
        let (p0, p1, p2, p3) = (p[i], p[i + 1], p[i + 2], p[i + 3]);
        i += 1;
        if p1[0] == p2[0] {
            continue;
        }
        let (k1, k2);
        if p0[0] == p1[0] && p2[0] == p3[0] {
            k1 = (p2[1] - p1[1]) / (p2[0] - p1[0]);
            k2 = k1;
        } else if p0[0] == p1[0] {
            k2 = (p3[1] - p1[1]) / (p3[0] - p1[0]);
            k1 = (3.0 * (p2[1] - p1[1]) / (p2[0] - p1[0]) - k2) / 2.0;
        } else if p2[0] == p3[0] {
            k1 = (p2[1] - p0[1]) / (p2[0] - p0[0]);
            k2 = (3.0 * (p2[1] - p1[1]) / (p2[0] - p1[0]) - k1) / 2.0;
        } else {
            k1 = (p2[1] - p0[1]) / (p2[0] - p0[0]);
            k2 = (p3[1] - p1[1]) / (p3[0] - p1[0]);
        }

        // Polynomial coefficients and forward differences
        let (x1, y1, x2, y2) = (p1[0], p1[1], p2[0], p2[1]);
        let dx = x2 - x1;
        let dy = y2 - y1;
        let a = ((k1 + k2) - 2.0 * dy / dx) / (dx * dx);
        let b = ((k2 - k1) / dx - 3.0 * (x1 + x2) * a) / 2.0;
        let c = k1 - (3.0 * x1 * a + 2.0 * b) * x1;
        let d = y1 - ((x1 * a + b) * x1 + c) * x1;
        let mut y = ((a * x1 + b) * x1 + c) * x1 + d;
        let mut dy = (3.0 * a * (x1 + res) + 2.0 * b) * x1 * res + ((a * res + b) * res + c) * res;
        let mut d2y = (6.0 * a * (x1 + res) + 2.0 * b) * res * res;
        let d3y = 6.0 * a * res * res * res;
        let mut x = x1;
        while x <= x2 {
            plot(x, y);
            y += dy;
            dy += d2y;
            d2y += d3y;
            x += res;
        }
    }
}

/// Output of an inverting op-amp with gain `n` (amplifier or summer)
/// for input `vi`, solving Kirchhoff's current law for the transistors
/// in triode: Newton-Raphson with bisection as fallback. `x` is the
/// initial guess and receives the solution.
fn solve_gain_d(opamp: &[Opamp], n: f64, vi: i32, x: &mut i32, ak0: i32, bk0: i32, k_vddt: i32) -> u16 {
    let (mut ak, mut bk) = (ak0, bk0);
    let a = n + 1.0;
    let b = k_vddt;
    let b_vi = if b > vi { (b - vi) as f64 } else { 0.0 };
    let c = n * (b_vi * b_vi);

    loop {
        let xk = *x;
        let vx = opamp[*x as usize].vx as i32;
        let dvx = opamp[*x as usize].dvx as i32;

        // f = a*(b - vx)^2 - c - (b - vo)^2
        // df = 2*((b - vo) - a*(b - vx))*dvx
        let mut vo = vx + (*x << 1) - (1 << 16);
        if vo > (1 << 16) - 1 {
            vo = (1 << 16) - 1;
        } else if vo < 0 {
            vo = 0;
        }
        let b_vx = if b > vx { (b - vx) as f64 } else { 0.0 };
        let b_vo = if b > vo { (b - vo) as f64 } else { 0.0 };
        let f = a * (b_vx * b_vx) - c - (b_vo * b_vo);
        let df = 2.0 * (b_vo - a * b_vx) * dvx as f64;

        if df != 0.0 {
            *x = x.wrapping_sub((2048.0 * f / df) as i32);
        }
        if *x == xk {
            return vo as u16;
        }
        // Narrow the root interval
        if f < 0.0 {
            ak = xk;
        } else {
            bk = xk;
        }
        if *x <= ak || *x >= bk {
            // Bisection (Dekker's method)
            *x = (ak + bk) >> 1;
            if *x == ak {
                return vo as u16;
            }
        }
    }
}

fn build(model: Model) -> ModelFilter {
    let fi = match model {
        Model::Mos6581 => &INIT_6581,
        Model::Mos8580 => &INIT_8580,
    };
    let is6581 = model == Model::Mos6581;
    let size = fi.opamp_voltage.len();

    // Op-amp voltages converted to 16 bits
    let vmin = fi.opamp_voltage[0][0];
    let opamp_max = fi.opamp_voltage[0][1];
    let k_vddt = fi.k * (fi.vdd - fi.vth);
    let vmax = if k_vddt < opamp_max { opamp_max } else { k_vddt };
    let denorm = vmax - vmin;
    let norm = 1.0 / denorm;
    let n16 = norm * ((1u32 << 16) - 1) as f64;
    let n31 = norm * ((1u32 << 31) - 1) as f64;

    // On the 6581 the mixer input resistors for the filter outputs are
    // slightly larger than those for the voices
    let filter_gain = ((if is6581 { 0.93 } else { 1.0 }) * (1 << 12) as f64) as i32;
    let n14 = norm * (1u32 << 14) as f64;
    let voice_scale_s14 = (n14 * fi.voice_voltage_range) as i32;
    let voice_dc = (n16 * (fi.voice_dc_voltage - vmin)) as i32;
    let k_vddt_i = (n16 * (k_vddt - vmin) + 0.5) as i32;
    let tmp_n_param = denorm * (1 << 13) as f64 * ((fi.u_cox / 2.0) * 1.0e-6 / fi.c);

    // Table vo - vx -> vx; the y axis is temporarily 31 bits for the
    // precision of the derivative
    let mut scaled = vec![[0.0f64; 2]; size];
    for i in 0..size {
        let [vi, vo] = fi.opamp_voltage[i];
        scaled[size - 1 - i][0] = n16 * (vo - vi) / 2.0 + (1 << 15) as f64;
        scaled[size - 1 - i][1] = n31 * (vi - vmin);
    }
    if scaled[size - 1][0] > 65535.0 {
        scaled[size - 1][0] = 65535.0;
        scaled[size - 2][0] = 65535.0;
    }
    let mut voltages = vec![0u32; 1 << 16];
    interpolate(&scaled, &mut |x, y| {
        voltages[x as i32 as usize] = (if y < 0.0 { 0.0 } else { y } + 0.5) as u32;
    }, 1.0);

    // Function and derivative in the same table
    let ak = (scaled[0][0] + 0.5) as i32;
    let bk = (scaled[size - 1][0] + 0.5) as i32;
    let mut opamp = vec![Opamp::default(); 1 << 16];
    let mut f = voltages[ak as usize];
    for j in ak as usize..bk as usize {
        let fp = f;
        f = voltages[j]; // scale m*2^31
        let df = f.wrapping_sub(fp) as i32; // scale 2^15
        opamp[j].vx = if f > (0xFFFF << 15) { 0xFFFF } else { (f >> 15) as u16 };
        opamp[j].dvx = (df >> (15 - 11)) as i16;
    }
    // The first point has no derivative: use that of the second
    opamp[ak as usize].dvx = opamp[ak as usize + 1].dvx;

    let solve = |n: f64, vi: i32, x: &mut i32| solve_gain_d(&opamp, n, vi, x, ak, bk, k_vddt_i);

    // Filter summer: gain ~1, 2-6 inputs
    let mut summer = vec![0u16; SUMMER_SIZE];
    let mut offset = 0;
    for k in 0..5 {
        let idiv = 2 + k as i32;
        let size = (idiv as usize) << 16;
        let mut x = ak;
        for vi in 0..size {
            summer[offset + vi] = solve(idiv as f64, vi as i32 / idiv, &mut x);
        }
        offset += size;
    }

    // Mixer: gain ~8/6 (6581) or 8/5 (8580), 0-7 inputs
    let divider = if is6581 { 6.0 } else { 5.0 };
    let mut mixer = vec![0u16; MIXER_SIZE];
    let mut offset = 0;
    let mut size = 1;
    for l in 0..8 {
        let mut idiv = l;
        let n_idiv = (idiv << 3) as f64 / divider;
        if idiv == 0 {
            idiv = 1; // n_idiv = 0: the result is correct anyway
        }
        let mut x = ak;
        for vi in 0..size {
            mixer[offset + vi] = solve(n_idiv, vi as i32 / idiv, &mut x);
        }
        offset += size;
        size = (l as usize + 1) << 16;
    }

    // Volume: gain ~vol/12 (6581) or vol/16 (8580)
    let divider = if is6581 { 12.0 } else { 16.0 };
    let mut gain = vec![0u16; 16 << 16];
    for n8 in 0..16 {
        let n = n8 as f64 / divider;
        let mut x = ak;
        for vi in 0..1 << 16 {
            gain[(n8 << 16) | vi] = solve(n, vi as i32, &mut x);
        }
    }

    let opamp_rev: Vec<u16> = opamp.iter().map(|o| o.vx).collect();

    // Resonance: 6581 1/Q ~ ~res/8, 8580 from the resistor table
    let mut resonance = vec![0u16; 16 << 16];
    for n8 in 0..16 {
        let n = if is6581 { (!n8 & 0xF) as f64 / 8.0 } else { RES_GAIN_8580[n8] };
        let mut x = ak;
        for vi in 0..1 << 16 {
            resonance[(n8 << 16) | vi] = solve(n, vi as i32, &mut x);
        }
    }

    let mut f0_dac = vec![0u16; 1 << 11];
    let mut vcr_kvg = Vec::new();
    let mut vcr_n_ids_term = Vec::new();
    let (mut n_snake, mut n_param) = (0, 0);
    if is6581 {
        // "Snake" current for one cycle at 1 MHz
        n_snake = (fi.wl_snake * tmp_n_param + 0.5) as i32;

        // Cutoff DAC: 6581 R-2R ladder converted to voltage
        let dac: [u16; 2048] = build_dac_table(11, fi.dac_2r_div_r, fi.dac_term);
        for n in 0..1 << 11 {
            f0_dac[n] = (n16 * (fi.dac_zero + dac[n] as f64 * fi.dac_scale / (1 << 11) as f64 - vmin) + 0.5) as u16;
        }

        // VCR: gate voltage Vg = Vddt - sqrt(((Vddt - Vw)^2 + Vgdt^2)/2),
        // indexed by the square root argument divided by 2^16
        let k = fi.k;
        let k_vddt_n = n16 * (k * (fi.vdd - fi.vth));
        let vmin_n = vmin * n16;
        vcr_kvg = (0..1 << 16)
            .map(|i| {
                let vg = k_vddt_n - ((i as f64) * (1 << 16) as f64).sqrt();
                (k * vg - vmin_n + 0.5) as u16
            })
            .collect();

        // VCR current with the EKV model:
        // Ids = Is*(if - ir), if/ir = ln^2(1 + e^((k*(Vg - Vt) - Vs|d)/(2*Ut)))
        let k_vt = fi.k * fi.vth;
        let ut = fi.ut;
        let is = ((2.0 * fi.u_cox * ut * ut) / fi.k) * fi.wl_vcr;
        let n15 = n16 / 2.0;
        let n_is = n15 * 1.0e-6 / fi.c * is;
        vcr_n_ids_term = (0..1 << 16)
            .map(|i| {
                let kvg_vx = i - (1 << 15);
                let log_term = ((kvg_vx as f64 / n16 - k_vt) / (2.0 * ut)).exp().ln_1p();
                (n_is * log_term * log_term) as u16
            })
            .collect();
    } else {
        n_param = (tmp_n_param * 32.0 + 0.5) as i32;

        // Cutoff DAC: W/L ratios of the parallel NMOS, proportional
        // to the bits (with 5 fractional bits of fixed point)
        let dac_wl: u32 = 806;
        f0_dac[0] = (dac_wl >> 8) as u16;
        for n in 1..1u32 << 11 {
            let mut wl = 0;
            for i in 0..11 {
                let bitmask = 1 << i;
                if n & bitmask != 0 {
                    wl += dac_wl * (bitmask << 1);
                }
            }
            f0_dac[n as usize] = (wl >> 8) as u16;
        }
    }

    ModelFilter {
        k_vddt: k_vddt_i,
        voice_scale_s14,
        voice_dc,
        filter_gain,
        vo_n16: n16,
        opamp_rev,
        summer,
        gain,
        resonance,
        mixer,
        f0_dac,
        vcr_kvg,
        vcr_n_ids_term,
        n_snake,
        n_param,
    }
}

// ── Noise ────────────────────────────────────────────────────────────────────

/// Noise added to the inputs before reduction to 16 bits (dither), as
/// in reSID; here from a fixed generator instead of rand().
static NOISE: [i32; 1024] = {
    let mut t = [0i32; 1024];
    let mut s: u32 = 1;
    let mut i = 0;
    while i < 1024 {
        s = s.wrapping_mul(1_103_515_245).wrapping_add(12_345);
        t[i] = ((s >> 8) & ((1 << 19) - 1)) as i32;
        i += 1;
    }
    t
};

// ── Filter ───────────────────────────────────────────────────────────────────

pub struct Filter {
    model: Model,
    // Registers
    fc: u16,
    res: u8,
    filt: u8,
    mode: u8,
    vol: u8,
    /// Connected inputs: voices 1-3 and, with digiboost, EXT IN.
    voice_mask: u8,
    /// Inputs of the filter summer and of the mixer, from filt, mode and
    /// voice_mask.
    sum: u8,
    mix: u8,
    // State: high-pass, band-pass, low-pass with the integrator
    // voltages
    vhp: i32,
    vbp: i32,
    vbp_x: i32,
    vbp_vc: i32,
    vlp: i32,
    vlp_x: i32,
    vlp_vc: i32,
    // Inputs
    ve: i32,
    v1: i32,
    v2: i32,
    v3: i32,
    ext_in: i16,
    // Quantities derived from the registers (require the tables)
    vddt_vw_2: i32,
    vw_bias: i32,
    n_dac: i32,
    n_vgt: i32,
    /// Cutoff DAC bias voltage in V (VICE: 0.5 for the
    /// 6581, 0 for the 8580).
    dac_bias: f64,
    noise_index: u16,
}

impl Filter {
    pub fn new() -> Self {
        let mut f = Self {
            model: Model::Mos6581,
            fc: 0, res: 0, filt: 0, mode: 0, vol: 0,
            voice_mask: 0x07,
            sum: 0, mix: 0,
            vhp: 0, vbp: 0, vbp_x: 0, vbp_vc: 0, vlp: 0, vlp_x: 0, vlp_vc: 0,
            ve: 0, v1: 0, v2: 0, v3: 0,
            ext_in: 0,
            vddt_vw_2: 0, vw_bias: 0, n_dac: 0, n_vgt: 0,
            dac_bias: 0.0,
            noise_index: 0,
        };
        f.reset();
        f
    }

    /// Model change: the state restarts from zero.
    pub fn set_model(&mut self, model: Model, voice_mask: u8, ext_in: i16, dac_bias: f64) {
        self.model = model;
        self.voice_mask = 0xF0 | (voice_mask & 0x0F);
        self.ext_in = ext_in;
        self.dac_bias = dac_bias;
        self.set_sum_mix();
        self.clear_state();
    }

    fn clear_state(&mut self) {
        self.vhp = 0;
        self.vbp = 0;
        self.vbp_x = 0;
        self.vbp_vc = 0;
        self.vlp = 0;
        self.vlp_x = 0;
        self.vlp_vc = 0;
    }

    pub fn reset(&mut self) {
        self.fc = 0;
        self.res = 0;
        self.filt = 0;
        self.mode = 0;
        self.vol = 0;
        self.clear_state();
        self.set_sum_mix();
    }

    /// Recomputes the quantities that depend on the model tables:
    /// DAC bias, cutoff, EXT IN.
    pub fn refresh(&mut self, t: &ModelFilter) {
        match self.model {
            Model::Mos6581 => self.vw_bias = (self.dac_bias * t.vo_n16) as i32,
            Model::Mos8580 => {
                // The gate voltage comes from a switched-capacitor
                // divider: Ua = Ue * v, 1 < v < 2
                let vg = VREF_8580 * (self.dac_bias * 6.0 / 100.0 + 1.6);
                let vgt = vg - INIT_8580.vth;
                let vmin = INIT_8580.opamp_voltage[0][0];
                self.n_vgt = (t.vo_n16 * (vgt - vmin) + 0.5) as i32;
            }
        }
        self.set_w0(t);
        self.ve = ((self.ext_in as i32).wrapping_mul(t.voice_scale_s14).wrapping_mul(3) >> 14) + t.mixer[0] as i32;
    }

    fn set_w0(&mut self, t: &ModelFilter) {
        let dac = t.f0_dac[self.fc as usize] as i32;
        match self.model {
            Model::Mos6581 => {
                let d = (t.k_vddt - (self.vw_bias + dac)) as u32;
                self.vddt_vw_2 = (d.wrapping_mul(d) >> 1) as i32;
            }
            // 8580 cutoff: 0 - 12.5 kHz
            Model::Mos8580 => self.n_dac = (t.n_param * dac) >> 11,
        }
    }

    fn set_sum_mix(&mut self) {
        // The 3OFF bit affects voice 3 only if it goes directly to the mixer
        self.sum = self.filt & self.voice_mask;
        self.mix = ((self.mode & 0x70) | (!(self.filt | (self.mode & 0x80) >> 5) & 0x0F)) & self.voice_mask;
    }

    pub fn write_fc_lo(&mut self, v: u8, t: Option<&ModelFilter>) {
        self.fc = (self.fc & 0x7F8) | (v as u16 & 0x007);
        if let Some(t) = t { self.set_w0(t); }
    }

    pub fn write_fc_hi(&mut self, v: u8, t: Option<&ModelFilter>) {
        self.fc = ((v as u16) << 3 & 0x7F8) | (self.fc & 0x007);
        if let Some(t) = t { self.set_w0(t); }
    }

    pub fn write_res_filt(&mut self, v: u8) {
        self.res = (v >> 4) & 0x0F;
        self.filt = v & 0x0F;
        self.set_sum_mix();
    }

    pub fn write_mode_vol(&mut self, v: u8) {
        self.mode = v & 0xF0;
        self.set_sum_mix();
        self.vol = v & 0x0F;
    }

    #[inline]
    fn noise(&mut self) -> i32 {
        self.noise_index = (self.noise_index + 1) & 0x3FF;
        NOISE[self.noise_index as usize]
    }

    /// One filter cycle with the outputs of the three voices.
    #[inline]
    pub fn clock(&mut self, t: &ModelFilter, voice1: i32, voice2: i32, voice3: i32) {
        let n1 = self.noise();
        self.v1 = (voice1.wrapping_mul(t.voice_scale_s14).wrapping_add(n1) >> 18) + t.voice_dc;
        let n2 = self.noise();
        self.v2 = (voice2.wrapping_mul(t.voice_scale_s14).wrapping_add(n2) >> 18) + t.voice_dc;
        let n3 = self.noise();
        self.v3 = (voice3.wrapping_mul(t.voice_scale_s14).wrapping_add(n3) >> 18) + t.voice_dc;

        // Summer inputs
        let sum = self.sum;
        let mut vi = 0;
        let mut n = 0;
        if sum & 0x1 != 0 { vi += self.v1; n += 1; }
        if sum & 0x2 != 0 { vi += self.v2; n += 1; }
        if sum & 0x4 != 0 { vi += self.v3; n += 1; }
        if sum & 0x8 != 0 { vi += self.ve; n += 1; }

        match self.model {
            Model::Mos6581 => {
                self.vlp = integrate_6581(t, self.vddt_vw_2, self.vbp, &mut self.vlp_x, &mut self.vlp_vc);
                self.vbp = integrate_6581(t, self.vddt_vw_2, self.vhp, &mut self.vbp_x, &mut self.vbp_vc);
            }
            Model::Mos8580 => {
                self.vlp = integrate_8580(t, self.n_vgt, self.n_dac, self.vbp, &mut self.vlp_x, &mut self.vlp_vc);
                self.vbp = integrate_8580(t, self.n_vgt, self.n_dac, self.vhp, &mut self.vbp_x, &mut self.vbp_vc);
            }
        }
        let bp = self.vbp.clamp(0, 0xFFFF) as usize;
        let idx = SUMMER_OFFSET[n] as i32 + t.resonance[(self.res as usize) << 16 | bp] as i32 + self.vlp + vi;
        self.vhp = t.summer[idx.clamp(0, SUMMER_SIZE as i32 - 1) as usize] as i32;
    }

    /// 16-bit output: mixer and volume amplifier.
    #[inline]
    pub fn output(&self, t: &ModelFilter) -> i16 {
        let mix = self.mix;
        let mut vi = 0;
        let mut n = 0;
        if mix & 0x01 != 0 { vi += self.v1; n += 1; }
        if mix & 0x02 != 0 { vi += self.v2; n += 1; }
        if mix & 0x04 != 0 { vi += self.v3; n += 1; }
        if mix & 0x08 != 0 { vi += self.ve; n += 1; }
        if mix & 0x70 != 0 {
            let mut filt = 0;
            if mix & 0x10 != 0 { filt += self.vlp; n += 1; }
            if mix & 0x20 != 0 { filt += self.vbp; n += 1; }
            if mix & 0x40 != 0 { filt += self.vhp; n += 1; }
            let dc_offset = 32767 * ((1 << 12) - t.filter_gain);
            vi += (filt.wrapping_mul(t.filter_gain) + dc_offset) >> 12;
        }
        let idx1 = (MIXER_OFFSET[n] as i32 + vi).clamp(0, MIXER_SIZE as i32 - 1) as usize;
        let idx2 = t.mixer[idx1] as usize;
        (t.gain[(self.vol as usize) << 16 | idx2] as i32 - (1 << 15)) as i16
    }
}

/// 6581 integrator: the capacitor is charged by the current of the
/// "snake" resistor (always in triode) and of the cutoff-driven VCR.
/// One fixed-point step per cycle; returns vo.
#[inline]
fn integrate_6581(t: &ModelFilter, vddt_vw_2: i32, vi: i32, vx: &mut i32, vc: &mut i32) -> i32 {
    let k_vddt = t.k_vddt;

    // "Snake" voltages in triode
    let vgst = (k_vddt - *vx) as u32;
    let vgdt = (k_vddt - vi) as u32;
    let vgdt_2 = vgdt.wrapping_mul(vgdt);

    // "Snake" current, scale m*2^30
    let n_i_snake = t.n_snake.wrapping_mul(vgst.wrapping_mul(vgst).wrapping_sub(vgdt_2) as i32 >> 15);

    // VCR gate voltage
    let kvg = t.vcr_kvg[((vddt_vw_2 as u32).wrapping_add(vgdt_2 >> 1) >> 16) as usize] as i32;

    // VCR current (EKV model), scale m*2^30
    let vgs = (kvg - *vx + (1 << 15)).clamp(0, 0xFFFF) as usize;
    let vgd = (kvg - vi + (1 << 15)).clamp(0, 0xFFFF) as usize;
    let n_i_vcr = (t.vcr_n_ids_term[vgs] as i32 - t.vcr_n_ids_term[vgd] as i32).wrapping_shl(15);

    // Capacitor charge
    *vc = vc.wrapping_sub(n_i_snake.wrapping_add(n_i_vcr));

    // vx = g(vc)
    *vx = t.opamp_rev[((*vc >> 15) + (1 << 15)).clamp(0, 0xFFFF) as usize] as i32;
    *vx + (*vc >> 14)
}

/// 8580 integrator: the resistor is a ladder of parallel NMOS
/// selected by the cutoff bits, with gate voltage `n_vgt`.
#[inline]
fn integrate_8580(t: &ModelFilter, n_vgt: i32, n_dac: i32, vi: i32, vx: &mut i32, vc: &mut i32) -> i32 {
    let vgst = (n_vgt - *vx) as u32;
    let vgdt = if vi < n_vgt { (n_vgt - vi) as u32 } else { 0 }; // triode/saturation

    // DAC current, scale m*2^30
    let n_i_rfc = n_dac.wrapping_mul(vgst.wrapping_mul(vgst).wrapping_sub(vgdt.wrapping_mul(vgdt)) as i32 >> 15) >> 4;

    *vc = vc.wrapping_sub(n_i_rfc);
    *vx = t.opamp_rev[((*vc >> 15) + (1 << 15)).clamp(0, 0xFFFF) as usize] as i32;
    *vx + (*vc >> 14)
}

use crate::snapshot::impl_state;

impl_state!(Filter {
    model, fc, res, filt, mode, vol, voice_mask, sum, mix, vhp, vbp, vbp_x, vbp_vc, vlp, vlp_x,
    vlp_vc, ve, v1, v2, v3, ext_in, vddt_vw_2, vw_bias, n_dac, n_vgt, dac_bias, noise_index,
});

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn opamp_tables_are_sane() {
        let t = tables(Model::Mos6581);
        // The op-amp inverse is monotonic where defined
        let r: Vec<u16> = t.opamp_rev.iter().copied().filter(|&v| v != 0).collect();
        assert!(r.len() > 30_000);
        assert!(r.windows(2).all(|w| w[1] <= w[0]) || r.windows(2).all(|w| w[1] >= w[0]));
        // Volume 0 gives a constant output, volume 15 does not
        let g0 = &t.gain[0..1 << 16];
        assert!(g0.iter().all(|&v| v == g0[0]));
        let g15 = &t.gain[15 << 16..16 << 16];
        assert!(g15[1000] != g15[60000]);
    }
}
