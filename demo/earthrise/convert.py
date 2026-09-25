#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-2.0-or-later
# Copyright (C) 2026 SampoSoft - Francesco Sampoli
"""Converts a picture to C64 Koala and IFLI data, with previews.

Usage: python3 convert.py [source image] [options]; see --help.

IFLI (interlaced multicolor FLI) shows two multicolor FLI pictures on
alternate frames. The eye mixes them, so every pixel can be one of up to
16x16 averages of two palette colors. Frame B is also shifted right by one
hires pixel ($D016 = 1), which gives back 320 pixels of horizontal detail:
hires pixel x shows A[x // 2] and B[(x - 1) // 2].

Per frame, the constraints of multicolor FLI with the displayer in ifli.asm:
- background ($D021) black;
- two screen colors per 4x1 cell (screen RAM changes on every line), except
  lines 196-199, which share the screen line fetched at line 196: bad lines
  cannot be forced below raster line $F7;
- one color RAM color per 4x8 cell: the displayer rewrites color RAM between
  frames, so each frame has its own;
- the first three character columns show the FLI bug and stay black.

The conversion is an exhaustive search per cell: for each frame, with the
other one fixed, the cell colors are chosen among all 136 screen pairs and
16 color RAM values to minimize the error of the mixed result in CIELAB
(chroma weighted by --chroma), plus a flicker penalty on the difference in
lightness between the two frames; then every pixel takes the best of the
four colors of its cell. The frames alternate until the error stops
improving. Dithering is ordered and folded into the search: the target
lightness is offset by a 4x4 Bayer pattern, so the search picks colors that
alternate around it. Before that, the stars (points of 1-2 source pixels
that scaling would average away) are max-pooled back onto the dark sky.

Koala (multicolor bitmap, 3 colors + background per 4x8 cell) and a single
FLI frame go through the same search, for comparison.

Outputs in build/: previews (target, koala, fli, ifli = the two frames
mixed as the eye sees them, ifli_a, ifli_b), koala.bin, ifli_a.bin and
ifli_b.bin (16 KB VIC banks: screens 0-7, bitmap at $2000),
ifli_colors.asm (color RAM code for ifli.asm) and expect_*.png (the
emulator's 403x284 framebuffer of each frame, to check the viewers pixel
by pixel).
"""

import argparse
import os

import numpy as np
from PIL import Image

# The emulator palette (src/vic/mod.rs: C64_PALETTE, Colodore).
PALETTE = [0x000000, 0xFFFFFF, 0x813338, 0x75CEC8, 0x8E3C97, 0x56AC4D,
           0x2E2C9B, 0xEDF171, 0x8E5029, 0x553800, 0xC46C71, 0x4A4A4A,
           0x7B7B7B, 0xA9FF9F, 0x706DEB, 0xB2B2B2]

W, H = 320, 200       # hires pixels
MC = 160              # multicolor pixels per line
BUG_COLS = 3          # character columns hidden by the FLI bug
PAL_ASPECT = 0.9365   # width / height of a PAL C64 pixel
IFLI_SHIFT = 1        # frame B $D016 X scroll


def srgb_to_lin(c):
    c = np.asarray(c, float) / 255
    return np.where(c <= 0.04045, c / 12.92, ((c + 0.055) / 1.055) ** 2.4)


def lin_to_srgb8(l):
    l = np.clip(l, 0, 1)
    s = np.where(l <= 0.0031308, l * 12.92, 1.055 * l ** (1 / 2.4) - 0.055)
    return np.round(s * 255).astype(np.uint8)


_M = np.array([[0.4124564, 0.3575761, 0.1804375],
               [0.2126729, 0.7151522, 0.0721750],
               [0.0193339, 0.1191920, 0.9503041]])
_WHITE = _M.sum(1)


def lin_to_lab(rgb):
    xyz = np.asarray(rgb) @ _M.T / _WHITE
    f = np.where(xyz > 216 / 24389, np.cbrt(xyz), (24389 / 27 * xyz + 16) / 116)
    return np.stack([116 * f[..., 1] - 16,
                     500 * (f[..., 0] - f[..., 1]),
                     200 * (f[..., 1] - f[..., 2])], -1)


PAL_LIN = srgb_to_lin([[(c >> 16) & 255, (c >> 8) & 255, c & 255] for c in PALETTE])
MIX_LIN = (PAL_LIN[:, None] + PAL_LIN[None, :]) / 2        # [a, b] -> linear RGB
PAL_L = lin_to_lab(PAL_LIN)[:, 0]
FLICKER = (PAL_L[:, None] - PAL_L[None, :]) ** 2           # [a, b] -> dL^2
CHROMA = 1.0
MIX_P = None                                               # [a, b] -> perceptual


def perceptual(lin):
    """CIELAB with the chroma axes weighted by CHROMA."""
    return lin_to_lab(lin) * np.array([1, CHROMA, CHROMA])


def set_metric(chroma):
    global CHROMA, MIX_P
    CHROMA = chroma
    MIX_P = perceptual(MIX_LIN)


set_metric(1.0)

PAIRS = [(h, l) for h in range(16) for l in range(h, 16)]
PAIR_H = np.array([p[0] for p in PAIRS])
PAIR_L = np.array([p[1] for p in PAIRS])

BAYER4 = (np.array([[0, 8, 2, 10], [12, 4, 14, 6], [3, 11, 1, 9], [15, 7, 13, 5]]) + 0.5) / 16 - 0.5


def resize_lin(lin, size):
    return np.stack([np.asarray(Image.fromarray(lin[..., c].astype(np.float32), 'F')
                                .resize(size, Image.LANCZOS)) for c in range(3)], -1)


def blur(a, sigma):
    """Separable Gaussian blur of a 2D array (edges repeated)."""
    r = int(3 * sigma)
    k = np.exp(-0.5 * (np.arange(-r, r + 1) / sigma) ** 2)
    k /= k.sum()
    for axis in (0, 1):
        p = np.pad(a, [(r, r) if i == axis else (0, 0) for i in (0, 1)], mode='edge')
        a = sum(w * np.take(p, np.arange(a.shape[axis]) + i, axis) for i, w in enumerate(k))
    return a


def load_target(path, x0, width, adjust, stars):
    """Crops the source at full height from column x0 and scales it to the
    picture area (hires x 25-319: the FLI bug covers x 0-24 in one of the
    two frames), in linear light. Returns (H, W, 3) linear RGB.

    Scaling averages away the stars, points of 1-2 source pixels: on the
    dark sky, the positive detail (luminance minus its blur) is max-pooled
    into the target pixel instead, and added back scaled by `stars`."""
    im = Image.open(path).convert('RGB')
    if width is None:
        width = round(im.height * (W - 25) * PAL_ASPECT / H)
    im = im.crop((x0, 0, x0 + width, im.height))
    lin = srgb_to_lin(np.asarray(im))
    out = np.clip(resize_lin(lin, (W - 25, H)), 0, 1)
    if stars > 0:
        lum = lin @ np.array([0.2126, 0.7152, 0.0722])
        detail = np.clip(lum - blur(lum, 3), 0, None) * (blur(lum, 12) < 0.04)
        ty = np.arange(lum.shape[0]) * H // lum.shape[0]
        tx = np.arange(lum.shape[1]) * (W - 25) // lum.shape[1]
        pooled = np.zeros((H, W - 25))
        np.maximum.at(pooled, (ty[:, None], tx[None, :]), detail)
        pooled = np.where(pooled > 0.04, pooled, 0)
        out = np.clip(out + stars * pooled[..., None], 0, 1)
    sat, gamma, gain = adjust
    luma = out @ np.array([0.2126, 0.7152, 0.0722])
    out = np.clip(luma[..., None] + (out - luma[..., None]) * sat, 0, 1)
    out = np.clip(gain * out ** gamma, 0, 1)
    target = np.zeros((H, W, 3))
    target[:, 25:] = out
    return target


def ordered(target_p, strength, cell):
    """Ordered dithering folded into the fitting: the target lightness is
    offset by a 4x4 Bayer pattern of `cell` hires pixels per step, so the
    exhaustive search picks colors that alternate around it."""
    y = np.arange(H)[:, None] % 4
    x = (np.arange(W)[None, :] // cell) % 4
    t = target_p.copy()
    t[..., 0] += strength * BAYER4[y, x]
    return t


# ── Frame model ──────────────────────────────────────────────────────────────

def to_hires(mc, shift):
    """Hires color map (H, W) of a frame from its multicolor map (H, MC),
    with the frame shifted right by `shift` pixels (background black)."""
    x = np.arange(W) - shift
    idx = np.clip(x // 2, 0, MC - 1)
    return np.where(x >= 0, mc[:, idx], 0)


def mc_costs(target_p, other_hires, shift, lam):
    """(H, MC, 16): error of each multicolor pixel of a frame for each
    color, with the other frame's hires colors fixed (None: the frame is
    shown alone, no mixing and no flicker)."""
    if other_hires is None:
        d = ((MIX_P[np.arange(16), np.arange(16)][None, None] - target_p[:, :, None]) ** 2).sum(-1)
    else:
        d = ((MIX_P[:, other_hires] - target_p[None]) ** 2).sum(-1)       # (16, H, W)
        d = (d + lam * FLICKER[:, other_hires]).transpose(1, 2, 0)
    pad = np.zeros((H, W + 2, 16))           # pixel i covers x = 2i + shift, 2i + 1 + shift
    pad[:, :W - shift] = d[:, shift:]
    return pad[:, 0:2 * MC:2] + pad[:, 1:2 * MC:2]


def line_groups(kind):
    """Groups of lines of each character row that share the screen colors."""
    if kind == 'koala':
        return [[list(range(8))]] * 25
    # FLI: every line, except 196-199: no forced bad line after raster $F7
    return [[[y] for y in range(8)]] * 24 + [[[0], [1], [2], [3], [4, 5, 6, 7]]]


def fit_frame(cost, kind, bug):
    """Chooses the cell colors minimizing `cost` (H, MC, 16).
    Returns sets (H, MC, 4): the colors of bit pairs 00, 01, 10, 11 for each
    multicolor pixel, plus screen hi/lo (H, 40) and color RAM (25, 40)."""
    hi = np.zeros((H, 40), int)
    lo = np.zeros((H, 40), int)
    cram = np.zeros((25, 40), int)
    for r, groups in enumerate(line_groups(kind)):
        c = cost[8 * r:8 * r + 8]
        base = np.minimum(c[..., :1], c)                            # bg or color RAM m
        pm = np.minimum(c[..., PAIR_H], c[..., PAIR_L])             # screen pair p
        tot = np.minimum(base[..., :, None], pm[..., None, :])      # (8, MC, 16, 136)
        tot = tot.reshape(8, 40, 4, 16, len(PAIRS)).sum(2)          # (8, 40, 16, 136)
        rowcost = np.zeros((40, 16))
        choices = []
        for g in groups:
            gt = tot[g].sum(0)
            rowcost += gt.min(-1)
            choices.append((g, gt.argmin(-1)))
        m = rowcost.argmin(-1)
        cram[r] = m
        for g, best in choices:
            p = best[np.arange(40), m]
            for y in g:
                hi[8 * r + y] = PAIR_H[p]
                lo[8 * r + y] = PAIR_L[p]
    if bug:
        hi[:, :BUG_COLS] = lo[:, :BUG_COLS] = cram[:, :BUG_COLS] = 0
    sets = np.stack([np.zeros((H, 40), int), hi, lo, np.repeat(cram, 8, 0)], -1)
    return np.repeat(sets, 4, 1), hi, lo, cram


def assign(cost, sets):
    """Best color of each multicolor pixel among the four of its cell."""
    c = np.take_along_axis(cost, sets, -1)
    return np.take_along_axis(sets, c.argmin(-1)[..., None], -1)[..., 0]


def error(target_p, a_hires, b_hires):
    """Mean squared perceptual error of the (mixed) result."""
    return ((MIX_P[a_hires, b_hires] - target_p) ** 2).sum(-1).mean()


# ── Conversions ──────────────────────────────────────────────────────────────

def convert_ifli(target_p, lam, passes, shift=IFLI_SHIFT):
    """Returns ((mc, sets, (hi, lo, cram)) for frame A, the same for B)."""
    # Start: each multicolor pixel split into the best pair of colors, the
    # two halves alternating between the frames in a checkerboard.
    pair_t = (target_p[:, 0::2] + target_p[:, 1::2]) / 2
    cost = ((MIX_P[None, None] - pair_t[:, :, None, None]) ** 2).sum(-1) + lam * FLICKER
    best = cost.reshape(H, MC, 256).argmin(-1)
    ka, kb = best // 16, best % 16
    checker = (np.arange(MC)[None] + np.arange(H)[:, None]) % 2 == 1
    a_map = np.where(checker, kb, ka)
    b_map = np.where(checker, ka, kb)
    for n in range(passes):
        ca = mc_costs(target_p, to_hires(b_map, shift), 0, lam)
        sets_a, *frame_a = fit_frame(ca, 'fli', True)
        a_map = assign(ca, sets_a)
        cb = mc_costs(target_p, to_hires(a_map, 0), shift, lam)
        sets_b, *frame_b = fit_frame(cb, 'fli', True)
        b_map = assign(cb, sets_b)
        a_h, b_h = to_hires(a_map, 0), to_hires(b_map, shift)
        print(f'  pass {n + 1}: error {error(target_p, a_h, b_h):.1f}, '
              f'flicker {(lam * FLICKER[a_h, b_h]).mean():.1f}')
    return (a_map, sets_a, frame_a), (b_map, sets_b, frame_b)


def convert_single(target_p, kind):
    cost = mc_costs(target_p, None, 0, 0)
    sets, *frame = fit_frame(cost, kind, kind == 'fli')
    return assign(cost, sets), sets, frame


# ── Output ───────────────────────────────────────────────────────────────────

def bitmap_bytes(mc, sets):
    """8000 bytes of multicolor bitmap in the VIC layout."""
    bits = (sets == mc[..., None]).argmax(-1)                  # first match
    b = bits.reshape(H, 40, 4)
    cells = (b[..., 0] << 6) | (b[..., 1] << 4) | (b[..., 2] << 2) | b[..., 3]   # (H, 40)
    return cells.reshape(25, 8, 40).transpose(0, 2, 1).astype(np.uint8).ravel()


def fli_bank(mc, sets, hi, lo):
    """16 KB VIC bank: screens 0-7 at $0000-$1FFF (line y uses screen
    y & 7, lines 196-199 screen 4), bitmap at $2000."""
    bank = np.zeros(0x4000, np.uint8)
    for y in range(H):
        s, row = min(y, 196) & 7, y // 8
        bank[s * 0x400 + row * 40:s * 0x400 + row * 40 + 40] = (hi[y] << 4) | lo[y]
    bank[0x2000:0x2000 + 8000] = bitmap_bytes(mc, sets)
    return bank


def render(a_hires, b_hires=None):
    return lin_to_srgb8(MIX_LIN[a_hires, a_hires if b_hires is None else b_hires])


def save_png(path, rgb, scale=2):
    im = Image.fromarray(rgb)
    im.resize((im.width * scale, im.height * scale), Image.NEAREST).save(path)


def emulator_frame(hires):
    """403x284 framebuffer as the emulator shows a frame (DISPLAY_X 41,
    DISPLAY_Y 42), to compare with its screenshots pixel by pixel."""
    fb = np.zeros((284, 403), int)
    fb[42:242, 41:361] = hires
    return render(fb)


def main():
    here = os.path.dirname(os.path.abspath(__file__))
    ap = argparse.ArgumentParser(description=__doc__.split('\n')[0])
    ap.add_argument('source', nargs='?', default=os.path.join(here, 'source.webp'))
    ap.add_argument('--x0', type=int, default=118, help='first source column (default 118)')
    ap.add_argument('--width', type=int, help='source columns (default: PAL aspect at full height)')
    ap.add_argument('--sat', type=float, default=1.5, help='saturation (default 1.5)')
    ap.add_argument('--gamma', type=float, default=1.0, help='gamma on linear light (default 1)')
    ap.add_argument('--gain', type=float, default=1.0, help='brightness gain (default 1)')
    ap.add_argument('--stars', type=float, default=3, help='star recovery (default 3, 0 = off)')
    ap.add_argument('--chroma', type=float, default=1.6, help='weight of chroma in the error (default 1.6)')
    ap.add_argument('--flicker', type=float, default=0.05,
                    help='penalty on the lightness difference between the IFLI frames (default 0.05)')
    ap.add_argument('--dither', type=float, default=8,
                    help='ordered dithering amplitude in L* units (default 8, 0 = off)')
    ap.add_argument('--passes', type=int, default=6, help='IFLI fitting passes (default 6)')
    ap.add_argument('--out', default=os.path.join(here, 'build'), help='output directory (default build/)')
    args = ap.parse_args()
    os.makedirs(args.out, exist_ok=True)
    out = lambda name: os.path.join(args.out, name)
    set_metric(args.chroma)

    target = load_target(args.source, args.x0, args.width, (args.sat, args.gamma, args.gain), args.stars)
    save_png(out('target.png'), lin_to_srgb8(target))
    tp = perceptual(target)

    print('Koala')
    k_mc, k_sets, (k_hi, k_lo, k_cram) = convert_single(ordered(tp, args.dither, 2), 'koala')
    k_h = to_hires(k_mc, 0)
    print(f'  error {error(tp, k_h, k_h):.1f}')
    save_png(out('koala.png'), render(k_h))
    koala = np.zeros(10001, np.uint8)                 # Koala layout, background black
    koala[:8000] = bitmap_bytes(k_mc, k_sets)
    koala[8000:9000] = ((k_hi[::8] << 4) | k_lo[::8]).ravel()
    koala[9000:10000] = k_cram.ravel()
    koala.tofile(out('koala.bin'))
    Image.fromarray(emulator_frame(k_h)).save(out('expect_koala.png'))

    print('FLI (one frame)')
    f_mc, _, _ = convert_single(ordered(tp, args.dither, 2), 'fli')
    f_h = to_hires(f_mc, 0)
    print(f'  error {error(tp, f_h, f_h):.1f}')
    save_png(out('fli.png'), render(f_h))

    print('IFLI')
    (a_mc, a_sets, (a_hi, a_lo, a_cram)), (b_mc, b_sets, (b_hi, b_lo, b_cram)) = \
        convert_ifli(ordered(tp, args.dither, 1), args.flicker, args.passes)
    a_h, b_h = to_hires(a_mc, 0), to_hires(b_mc, IFLI_SHIFT)
    print(f'  error {error(tp, a_h, b_h):.1f}')
    save_png(out('ifli.png'), render(a_h, b_h))
    save_png(out('ifli_a.png'), render(a_h))
    save_png(out('ifli_b.png'), render(b_h))
    fli_bank(a_mc, a_sets, a_hi, a_lo).tofile(out('ifli_a.bin'))
    fli_bank(b_mc, b_sets, b_hi, b_lo).tofile(out('ifli_b.bin'))
    np.stack([a_cram.ravel(), b_cram.ravel()]).astype(np.uint8).tofile(out('ifli_cram.bin'))
    Image.fromarray(emulator_frame(a_h)).save(out('expect_a.png'))
    Image.fromarray(emulator_frame(b_h)).save(out('expect_b.png'))
    write_colors_asm(out('ifli_colors.asm'), a_cram.ravel(), b_cram.ravel())
    print(f'  color RAM: {int((a_cram != b_cram).sum())} cells differ between the frames')


def write_colors_asm(path, cram_a, cram_b):
    """Color RAM code for ifli.asm: `copy_a`/`copy_b` rewrite, with
    LDA #/STA (6 cycles each), the cells whose color differs between the
    frames; `cram_a` is the whole color RAM of frame A for the start."""
    diff = np.nonzero(cram_a != cram_b)[0]
    lines = ['; Generated by convert.py: color RAM of the IFLI frames.', '',
             f'NDIFF = {len(diff)}          ; cells that differ between the frames', '']
    for name, cram in (('copy_a', cram_a), ('copy_b', cram_b)):
        lines.append(f'{name}:')
        lines += [f'        lda #{cram[i]}\n        sta $d800+{i}' for i in diff]
        lines += ['        rts', '']
    lines.append('cram_a:')
    lines += ['        .byte ' + ', '.join(str(v) for v in cram_a[i:i + 20]) for i in range(0, 1000, 20)]
    with open(path, 'w') as f:
        f.write('\n'.join(lines) + '\n')


if __name__ == '__main__':
    main()
