// SPDX-License-Identifier: GPL-2.0-or-later
// Copyright (C) 2026 SampoSoft - Francesco Sampoli

// Window drawing and CRT monitor emulation (model and constants in
// src/crt.rs). The constants W, H, NS, PH, LUMA_TAPS, LUMA_FIRST, FIR_HALF,
// SEP_HALF, FIRST_LINE, LINES, PAL, LINE_FLIP, the offsets U_* into the parameters and
// the gammas are prepended by frontend::gpu, for the machine's standard.
// Signal passes run on NS samples per line (PH every 4 pixels: 9 PAL, 7
// NTSC).
//
// Passes of the CRT emulation, all at signal resolution (NS samples by H
// lines) except the last:
//   vic      palette indices → VIC-II output: luma, chroma U/V, modulated chroma
//   cable    composite and RF: the C64's composite signal (modulator luma network)
//   tuner    RF: the TV's IF filter
//   separate input stage of the set: comb filter or luma trap, peaking,
//            chroma band-pass (composite signal)
//   decode   color decoder of the set → light emitted by the tube (linear)
//   glow_h/v halation in the glass (separable gaussian)
//   crt      per window pixel: beams of the lines, halation, slot mask

// Parameters (floats, see the U_* offsets in gpu.rs):
//   0 composite signal (composite or RF input), blend, mask strength, glow share
//   4 screen origin x, y, size x, y (window pixels)
//   8 triad pitch (window pixels), slot pitch (window pixels), stripe, bridge
//  12 beam sigma dark, bright (lines), glow sigma (samples), glow sigma (lines)
//  16 light white may lose to the mask, white point R, G, B
//  20 parity of the frame (NTSC: the subcarrier phase alternates), knobs:
//     brightness (drive offset, 0-255), contrast (gain), color (gain)
//  24 tint: cos, sin of the hue rotation; HDR headroom (the brightest value
//     the display takes, 1 = white in SDR), linear output (HDR)
//  28 comb filter on, its shift of the previous line (samples), shift of the
//     PAL delay line, monochrome (white point R, G, B = the phosphor)
//  32 phase of the chroma path at the subcarrier (the decoder locks to the
//     burst, which it turns as well)
//  33 tube errors: half the useful screen, window pixels (0: none), pincushion
//     east-west and north-south, misconvergence at the centre, the middle of
//     the edges and the corners (window pixels)
//  then the tables at U_PAL_Y, U_PAL_UV, U_LUMA, U_CHROMA, U_LUMA_LPF (FIR_HALF
//  taps each side), U_MOD, U_LUMA_IN, U_CHROMA_IN, U_IF, U_COMB (SEP_HALF taps
//  each side) and U_BARS (jail bars, 8 pixels)
@group(0) @binding(0) var<uniform> P: array<vec4<f32>, NV>;
@group(0) @binding(1) var index_tex: texture_2d<u32>;
@group(0) @binding(2) var sig_tex: texture_2d<f32>;
@group(0) @binding(3) var lin_tex: texture_2d<f32>;
@group(0) @binding(4) var prev_tex: texture_2d<f32>;
@group(0) @binding(5) var glow_tex: texture_2d<f32>;
@group(0) @binding(6) var lin_samp: sampler;
@group(0) @binding(7) var image_tex: texture_2d<f32>;

const PI: f32 = 3.14159265;

fn pf(i: u32) -> f32 {
    return P[i >> 2u][i & 3u];
}

struct VOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) uv: vec2<f32>,
}

// One triangle over the viewport; uv 0-1 from the top-left corner
@vertex
fn vs(@builtin(vertex_index) i: u32) -> VOut {
    let x = f32(i32(i & 1u) * 4 - 1);
    let y = f32(i32(i >> 1u) * 4 - 1);
    var o: VOut;
    o.pos = vec4<f32>(x, y, 0.0, 1.0);
    o.uv = vec2<f32>(x * 0.5 + 0.5, 0.5 - y * 0.5);
    return o;
}

// ── Plain image (no CRT, status bar): nearest pixel ─────────────────────────

@fragment
fn blit(v: VOut) -> @location(0) vec4<f32> {
    let size = vec2<f32>(textureDimensions(image_tex));
    let p = vec2<i32>(clamp(floor(v.uv * size), vec2<f32>(0.0), size - 1.0));
    return vec4<f32>(textureLoad(image_tex, p, 0).rgb, 1.0);
}

// sRGB-encoded values → linear light
fn srgb_to_linear(c: vec3<f32>) -> vec3<f32> {
    return select(pow((c + 0.055) / 1.055, vec3<f32>(2.4)), c / 12.92, c <= vec3<f32>(0.04045));
}

// The same into an HDR target (extended linear sRGB)
@fragment
fn blit_linear(v: VOut) -> @location(0) vec4<f32> {
    let size = vec2<f32>(textureDimensions(image_tex));
    let p = vec2<i32>(clamp(floor(v.uv * size), vec2<f32>(0.0), size - 1.0));
    return vec4<f32>(srgb_to_linear(textureLoad(image_tex, p, 0).rgb), 1.0);
}

// ── VIC-II ──────────────────────────────────────────────────────────────────

// Sample n of line y of the input image of a signal pass. Beyond the ends
// of the line (the border goes on) the nearest samples of the same
// subcarrier phase (a whole number of cycles, 4 samples, away)
fn sig(n: i32, y: i32) -> vec4<f32> {
    var m = n;
    if (m < 0) {
        m = ((m % 4) + 4) % 4;
    } else if (m >= NS) {
        m = NS - 4 + (m - NS) % 4;
    }
    return textureLoad(sig_tex, vec2<i32>(m, y), 0);
}

fn color_at(x: i32, y: i32) -> u32 {
    return textureLoad(index_tex, vec2<i32>(clamp(x, 0, W - 1), y), 0).r;
}

// Odd raster line: framebuffer line y is raster line y + FIRST_LINE (NTSC:
// the last lines are lines 0-11 of the next frame)
fn is_odd(y: i32) -> bool {
    return (((y + FIRST_LINE) % LINES) & 1) == 1;
}

// sin and cos of the subcarrier at sample n: 4 samples per cycle, and a
// line of 283.5 (PAL) or 227.5 (NTSC, PAL-N) cycles flips the phase from
// line to line (LINE_FLIP; not the 224 of the 6567R56A). Counting the lines
// from the start of the frame, an odd number of lines per frame (NTSC, 263)
// makes the phase alternate between frames too
fn carrier(n: i32, y: i32) -> vec2<f32> {
    return carrier_at(n, y, 0.0);
}

// The same turned by `phase`: the decoder's reference
fn carrier_at(n: i32, y: i32, phase: f32) -> vec2<f32> {
    let lines = i32(pf(20u)) * LINES + y + FIRST_LINE;
    let t = 0.5 * PI * (f32(n) + 0.5) + select(0.0, PI, LINE_FLIP && (lines & 1) == 1) + phase;
    return vec2<f32>(sin(t), cos(t));
}

// V switch of PAL (inverted on odd lines); none in NTSC
fn v_switch(y: i32) -> f32 {
    return select(1.0, -1.0, PAL && is_odd(y));
}

@fragment
fn vic(v: VOut) -> @location(0) vec4<f32> {
    let n = i32(v.pos.x);
    let y = i32(v.pos.y);
    // Pixel of the sample, floor((n + 0.5) × 4 / PH), and phase of the
    // sample in it (PH samples every 4 pixels)
    let p = (8 * n + 4) / (2 * PH);
    let phase = n % PH;
    var luma = 0.0;
    for (var j = 0; j < LUMA_TAPS; j++) {
        let c = color_at(p + LUMA_FIRST + j, y);
        luma += pf(U_LUMA + u32(phase * LUMA_TAPS + j)) * pf(U_PAL_Y + c);
    }
    // Jail bars: the same on every line, one PHI0 cycle long (the
    // character cells start at framebuffer x 41)
    luma += pf(U_BARS + u32((p + 7) & 7));
    let odd = is_odd(y);
    let pal = P[U_PAL_UV / 4u + color_at(p, y)];
    let uv = select(pal.xy, pal.zw, odd);
    // Modulated chroma: U sin + V cos, V inverted on odd lines (PAL)
    let sc = carrier(n, y);
    return vec4<f32>(luma, uv, uv.x * sc.x + v_switch(y) * uv.y * sc.y);
}

// The C64's composite signal: the luma through the network of the RF
// modulator, plus the modulated chroma
@fragment
fn cable(v: VOut) -> @location(0) vec4<f32> {
    let n = i32(v.pos.x);
    let y = i32(v.pos.y);
    var luma = 0.0;
    for (var k = -SEP_HALF; k <= SEP_HALF; k++) {
        luma += pf(U_MOD + u32(k + SEP_HALF)) * sig(n - k, y).x;
    }
    let s = sig(n, y);
    return vec4<f32>(s.xyz, luma + s.w);
}

// The TV's IF filter on the composite signal (RF input)
@fragment
fn tuner(v: VOut) -> @location(0) vec4<f32> {
    let n = i32(v.pos.x);
    let y = i32(v.pos.y);
    var w = 0.0;
    for (var k = -SEP_HALF; k <= SEP_HALF; k++) {
        w += pf(U_IF + u32(k + SEP_HALF)) * sig(n - k, y).w;
    }
    let s = sig(n, y);
    return vec4<f32>(s.xyz, w);
}

// ── Color decoder of the set ────────────────────────────────────────────────

// Input stage of the set. Luma: trap (composite signal), peaking, as the set
// has them. Chroma: with luma/chroma the U/V of the VIC as they are; with a
// composite signal the band-pass around the subcarrier, with whatever luma
// falls in its band (cross-color). With a comb filter the chroma is the
// band-pass of half the difference between the line and the previous one
// through the 1H delay line (shifted by pf(29u) samples when the C64's line
// is not as long as the delay), and the luma the signal without it
@fragment
fn separate(v: VOut) -> @location(0) vec4<f32> {
    let n = i32(v.pos.x);
    let y = i32(v.pos.y);
    let composite = pf(0u) > 0.5;
    var luma = 0.0;
    var chroma = 0.0;
    if (composite && pf(28u) > 0.5) {
        let off = i32(pf(29u));
        let prev = max(y - 1, 0);
        for (var k = -SEP_HALF; k <= SEP_HALF; k++) {
            let i = u32(k + SEP_HALF);
            let w = sig(n - k, y).w;
            let d = 0.5 * (w - sig(n - k + off, prev).w);
            luma += pf(U_LUMA_IN + i) * w - pf(U_COMB + i) * d;
            chroma += pf(U_CHROMA_IN + i) * d;
        }
        return vec4<f32>(luma, chroma, 0.0, 0.0);
    }
    for (var k = -SEP_HALF; k <= SEP_HALF; k++) {
        let s = sig(n - k, y);
        let i = u32(k + SEP_HALF);
        luma += pf(U_LUMA_IN + i) * select(s.x, s.w, composite);
        chroma += pf(U_CHROMA_IN + i) * s.w;
    }
    if (composite) {
        return vec4<f32>(luma, chroma, 0.0, 0.0);
    }
    return vec4<f32>(luma, sig(n, y).yz, 0.0);
}

// Luma and demodulated chroma of line y at sample n; the input (sig_tex)
// is the output of `separate`.
fn decode(n: i32, y: i32) -> vec3<f32> {
    // Luma amplifier of the monitor
    var yuv = vec3<f32>(0.0);
    for (var k = -FIR_HALF; k <= FIR_HALF; k++) {
        yuv.x += pf(U_LUMA_LPF + u32(k + FIR_HALF)) * sig(n - k, y).x;
    }
    if (pf(0u) > 0.5) {
        for (var k = -FIR_HALF; k <= FIR_HALF; k++) {
            let h = pf(U_CHROMA + u32(k + FIR_HALF));
            let c = sig(n - k, y).y;
            let r = carrier_at(n - k, y, pf(32u));
            yuv.y += h * 2.0 * c * r.x;
            yuv.z += h * 2.0 * c * r.y;
        }
        yuv.z *= v_switch(y);
    } else {
        for (var k = -FIR_HALF; k <= FIR_HALF; k++) {
            yuv.y += pf(U_CHROMA + u32(k + FIR_HALF)) * sig(n - k, y).y;
            yuv.z += pf(U_CHROMA + u32(k + FIR_HALF)) * sig(n - k, y).z;
        }
    }
    return yuv;
}

@fragment
fn decode_fs(v: VOut) -> @location(0) vec4<f32> {
    let n = i32(v.pos.x);
    let y = i32(v.pos.y);
    let a = decode(n, y);
    // PAL delay line: the chroma averaged with the previous line (NTSC has
    // none), shifted by pf(30u) samples when the C64's line is not as long
    // as the delay line (the Drean on a PAL-N set)
    var b = a;
    if (PAL) {
        b = decode(n + i32(pf(30u)), max(y - 1, 0));
    }
    let white = vec3<f32>(pf(17u), pf(18u), pf(19u));
    if (pf(31u) > 0.5) {
        // Monochrome: the luma of the composite signal (subcarrier
        // included) on the phosphor
        let level = clamp(a.x * CONTRAST * pf(22u) + pf(21u), 0.0, 255.0);
        return vec4<f32>(white * pow(max(level / 255.0, 1e-6), TUBE_GAMMA), 1.0);
    }
    // Front-panel knobs: contrast on the whole video, color on the chroma,
    // tint turning it, brightness on the black level
    let gain = CONTRAST * pf(22u);
    let luma = a.x * gain;
    let u0 = (a.y + b.y) * 0.5 * gain * pf(23u);
    let w0 = (a.z + b.z) * 0.5 * gain * pf(23u);
    let tc = pf(24u);
    let ts = pf(25u);
    let u = u0 * tc - w0 * ts;
    let w = u0 * ts + w0 * tc;
    let rgb = clamp(vec3<f32>(luma + 1.140 * w, luma - 0.396 * u - 0.581 * w, luma + 2.029 * u) + pf(21u),
                    vec3<f32>(0.0), vec3<f32>(255.0));
    // Light of the three guns, balanced to the monitor's white point
    return vec4<f32>(white * pow(max(rgb / 255.0, vec3<f32>(1e-6)), vec3<f32>(TUBE_GAMMA)), 1.0);
}

// ── Halation ────────────────────────────────────────────────────────────────

@fragment
fn glow_h(v: VOut) -> @location(0) vec4<f32> {
    let n = i32(v.pos.x);
    let y = i32(v.pos.y);
    let s = max(pf(14u), 0.1);
    let r = min(i32(ceil(3.0 * s)), 48);
    var acc = vec3<f32>(0.0);
    var sum = 0.0;
    for (var k = -r; k <= r; k++) {
        let w = exp(-0.5 * f32(k * k) / (s * s));
        acc += w * textureLoad(lin_tex, vec2<i32>(clamp(n + k, 0, NS - 1), y), 0).rgb;
        sum += w;
    }
    return vec4<f32>(acc / sum, 1.0);
}

@fragment
fn glow_v(v: VOut) -> @location(0) vec4<f32> {
    let n = i32(v.pos.x);
    let y = i32(v.pos.y);
    let s = max(pf(15u), 0.1);
    let r = min(i32(ceil(3.0 * s)), 48);
    var acc = vec3<f32>(0.0);
    var sum = 0.0;
    for (var k = -r; k <= r; k++) {
        let w = exp(-0.5 * f32(k * k) / (s * s));
        acc += w * textureLoad(glow_tex, vec2<i32>(n, clamp(y + k, 0, H - 1)), 0).rgb;
        sum += w;
    }
    return vec4<f32>(acc / sum, 1.0);
}

// ── Picture tube ────────────────────────────────────────────────────────────

// Error function (Abramowitz and Stegun 7.1.26, error < 1.5e-7)
fn erf3(x: vec3<f32>) -> vec3<f32> {
    let a = abs(x);
    let t = 1.0 / (1.0 + 0.3275911 * a);
    let p = t * (0.254829592 + t * (-0.284496736 + t * (1.421413741 + t * (-1.453152027 + t * 1.061405429))));
    return sign(x) * (1.0 - p * exp(-a * a));
}

fn sinc(x: f32) -> f32 {
    if (abs(x) < 1e-4) {
        return 1.0;
    }
    return sin(PI * x) / (PI * x);
}

// Light of the window pixel spanning lines v0-v1 at sample xs: every line
// is a gaussian beam (wider when brighter, per gun) with its energy
// conserved, integrated over the pixel
fn beam(t: texture_2d<f32>, xs: f32, v0: f32, v1: f32) -> vec3<f32> {
    let l0 = i32(floor((v0 + v1) * 0.5));
    var acc = vec3<f32>(0.0);
    for (var l = l0 - 2; l <= l0 + 2; l++) {
        if (l < 0 || l >= H) {
            continue;
        }
        let c = textureSampleLevel(t, lin_samp, vec2<f32>(xs / f32(NS), (f32(l) + 0.5) / f32(H)), 0.0).rgb;
        let sigma = mix(vec3<f32>(pf(12u)), vec3<f32>(pf(13u)), sqrt(c)) * 1.41421356;
        let center = f32(l) + 0.5;
        acc += c * (erf3((v1 - center) / sigma) - erf3((v0 - center) / sigma)) * 0.5;
    }
    return acc / (v1 - v0);
}

// Slot mask seen through the window pixel at `px` (window pixels from the
// screen corner): vertical R, G, B stripes and bridges between the slots,
// alternately offset by half a slot. Fourier series box-filtered over the
// pixel and cut below its Nyquist frequency: average 1, no moiré. The
// fourth component is the highest value the series can reach.
fn mask(px: vec2<f32>) -> vec4<f32> {
    let triad = pf(8u);
    let d = 1.0 / triad;
    let u = px.x * d;
    let stripe = pf(10u);
    let centers = vec3<f32>(1.0 / 6.0, 0.5, 5.0 / 6.0);
    var m = vec3<f32>(1.0);
    var peak = 1.0;
    for (var k = 1; k <= 12; k++) {
        let kd = f32(k) * d;
        let fade = smoothstep(0.5, 0.35, kd);
        if (fade <= 0.0) {
            break;
        }
        let a = 2.0 * sinc(f32(k) * stripe) * sinc(kd) * fade;
        m += a * cos(2.0 * PI * f32(k) * (u - centers));
        peak += abs(a);
    }
    // Bridges: only when the triads are large enough to show them
    let show = smoothstep(3.0, 5.0, triad);
    if (show > 0.0) {
        let beta = pf(11u);
        let dv = 1.0 / pf(9u);
        let col = floor(u);
        let vv = px.y * dv + 0.5 * (col - 2.0 * floor(col * 0.5));
        var b = 1.0;
        var bpeak = 1.0;
        for (var k = 1; k <= 12; k++) {
            let kd = f32(k) * dv;
            let fade = smoothstep(0.5, 0.35, kd);
            if (fade <= 0.0) {
                break;
            }
            let a = 2.0 * beta / (1.0 - beta) * sinc(f32(k) * beta) * sinc(kd) * fade;
            b -= a * cos(2.0 * PI * f32(k) * vv);
            bpeak += abs(a);
        }
        m *= mix(1.0, b, show);
        peak *= mix(1.0, bpeak, show);
    }
    return vec4<f32>(m, peak);
}

// Light of the lines at `p` (window pixels from the picture's corner), of
// the frame in `t`
fn lines(t: texture_2d<f32>, p: vec2<f32>, size: vec2<f32>) -> vec3<f32> {
    let yl = p.y / size.y * f32(H);
    let half = 0.5 * f32(H) / size.y;
    return beam(t, p.x / size.x * f32(NS), yl - half, yl + half);
}

// The same, with the frame blending: the eye averages the light of the
// two frames
fn light(p: vec2<f32>, size: vec2<f32>) -> vec3<f32> {
    let c = lines(lin_tex, p, size);
    if (pf(1u) > 0.5) {
        return 0.5 * (c + lines(prev_tex, p, size));
    }
    return c;
}

@fragment
fn crt(v: VOut) -> @location(0) vec4<f32> {
    let origin = P[1].xy;
    let size = P[1].zw;
    let rel = v.pos.xy - origin;
    // Tube errors: the raster bowed as a pincushion (the point of the
    // raster that lands here), and the red and blue beams landing apart,
    // more towards the edges and the corners
    var src = rel;
    var apart = 0.0;
    if (pf(33u) > 0.0) {
        let hs = vec2<f32>(pf(33u), pf(34u));
        let n = (rel - 0.5 * size) / hs;
        let r = vec2<f32>(n.x / (1.0 + pf(35u) * n.y * n.y), n.y / (1.0 + pf(36u) * n.x * n.x));
        src = 0.5 * size + r * hs;
        let rho = dot(n, n);
        apart = select(mix(pf(38u), pf(39u), clamp(rho - 1.0, 0.0, 1.0)), mix(pf(37u), pf(38u), rho), rho < 1.0);
    }
    var c = vec3<f32>(0.0);
    if (all(src >= vec2<f32>(0.0)) && all(src <= size)) {
        c = light(src, size);
        if (apart > 0.0) {
            c.r = light(src - vec2<f32>(0.5 * apart, 0.0), size).r;
            c.b = light(src + vec2<f32>(0.5 * apart, 0.0), size).b;
        }
        let g = pf(3u);
        c = (1.0 - g) * c + g * textureSampleLevel(glow_tex, lin_samp, src / size, 0.0).rgb;
    }
    // The mask, as deep as the display allows: where its brightest
    // stripes would go beyond the brightest value of the display (white, or
    // the HDR headroom pf(26u)) it is made shallower, so that the average
    // light (the color) stays right; but not below the depth that costs
    // white a share pf(16u) of its light (about depth·peak/π for a clipped
    // cosine), because on the real tube the mask shows on white too
    let top = pf(26u);
    if (pf(2u) <= 0.0) {
        // No mask (monochrome tube)
        c = clamp(c, vec3<f32>(0.0), vec3<f32>(top));
        let e = pow(min(c, vec3<f32>(1.0)), vec3<f32>(1.0 / DISPLAY_GAMMA));
        if (pf(27u) > 0.5) {
            return vec4<f32>(select(srgb_to_linear(e), c, c > vec3<f32>(1.0)), 1.0);
        }
        return vec4<f32>(e, 1.0);
    }
    let m = mask(rel);
    let room = max(top - c, vec3<f32>(0.0)) / max(c * (m.w - 1.0), vec3<f32>(1e-5));
    let least = min(pf(2u), PI * pf(16u) / max(m.w - 1.0, 1e-5));
    let depth = min(vec3<f32>(pf(2u)), max(room, vec3<f32>(least)));
    c *= vec3<f32>(1.0) + depth * (m.rgb - vec3<f32>(1.0));
    c = clamp(c, vec3<f32>(0.0), vec3<f32>(top));
    let encoded = pow(min(c, vec3<f32>(1.0)), vec3<f32>(1.0 / DISPLAY_GAMMA));
    if (pf(27u) > 0.5) {
        // HDR: the same picture up to white (the display gamma of the model,
        // as linear sRGB light), and the light beyond it as it is
        return vec4<f32>(select(srgb_to_linear(encoded), c, c > vec3<f32>(1.0)), 1.0);
    }
    return vec4<f32>(encoded, 1.0);
}
