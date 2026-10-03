#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-2.0-or-later
# Copyright (C) 2026 SampoSoft - Francesco Sampoli
"""Writes gfx.asm for sidwave.asm: character sets, screen, colour RAM,
sprites, tables and the raster kernel.

The picture (PAL raster lines):

    0- 50  upper border, opened: the logo in sprites
   51-154  sky, multicolour characters: the gradient is $d021 changed on
           every line, the sun is the colour %10 ($d023, also changed on
           every line: its gradient and its stripes), the stars and the
           windows are %01 ($d022), the buildings %11 (colour RAM, black)
  155-250  ground, hires characters (a second character set): the lines
           towards the vanishing point are characters, the horizontal ones
           are $d021 changed on the lines where they fall (by the kernel
           down to line 192, then by interrupts: at most two lines)
  251-311  lower border, opened: the scroller in sprites

Run with --preview to also write preview.png (the first frame, as the
kernel draws it, without sprites).
"""

import os
import sys

HERE = os.path.dirname(os.path.abspath(__file__))

CLOCK = 985248
TOP = 51                    # first display line
HORIZON = 155               # first ground line (row 13)
BOTTOM = 251                # first lower border line
KEND = 193                  # the kernel colours lines TOP..KEND-1
NEAR = 196                  # grid lines from here on are drawn by interrupts
SKY_ROWS = 13
GROUND_ROWS = 12

# --- the sky ---------------------------------------------------------------
def dither(c1, c2, y0, y1):
    """Alternating lines c1/c2 on y0..y1 (inclusive), weighted towards c2."""
    out = {}
    for y in range(y0, y1 + 1):
        t = (y - y0 + 0.5) / (y1 - y0 + 1)
        # 1/4, 1/2, 3/4 of c2 in three bands
        pattern = (0, 0, 0, 1) if t < 1 / 3 else (0, 1) if t < 2 / 3 else (0, 1, 1, 1)
        out[y] = c2 if pattern[y % len(pattern)] else c1
    return out


def bands(spec):
    """[(colour, y0, y1) or ('d', c1, c2, y0, y1)] -> {line: colour}."""
    out = {}
    for s in spec:
        if s[0] == 'd':
            out.update(dither(*s[1:]))
        else:
            c, y0, y1 = s
            out.update({y: c for y in range(y0, y1 + 1)})
    return out


SKY = bands([(0, 51, 80), ('d', 0, 6, 81, 92), (6, 93, 110), ('d', 6, 4, 111, 124),
             (4, 125, 150), ('d', 4, 10, 151, 154)])
SUN_CX, SUN_CY, SUN_RX, SUN_RY = 160, 118, 48, 44
SUN_TOP = SUN_CY - SUN_RY
SUN = bands([(7, SUN_TOP, 92), ('d', 7, 10, 93, 108), (10, 109, 154)])
# the sun rises in the intro through these colour ramps (full to gone);
# None is the sky behind
FADE = [{7: 7, 10: 10}, {7: 10, 10: 2}, {7: 2, 10: 9}, {7: 9, 10: None}, {7: None, 10: None}]
STRIPE_TOP = 104            # the stripes start here
STRIPE_PERIOD = 10
STRIPE_PHASES = 16

GROUND_BG = bands([(10, 155, 155), (4, 156, 156), (6, 157, 172), ('d', 6, 0, 173, 188),
                   (0, 189, 250)])
VPY, GRID_G, GRID_N = 150, 0.4, 14
LINE_COLOURS = bands([(4, 155, 175), (10, 176, 250)])
FLASH_COLOURS = bands([(10, 155, 175), (1, 176, 250)])
GRID_ROW_COLOURS = [6, 6, 4, 4, 10, 10, 10, 10, 10, 10, 10, 10]   # vertical lines, per row
GROUND_PHASES = 64
GROUND_DEPTH = 100          # y - VPY = GROUND_DEPTH / z

# buildings: 14 bars of 2 columns, rows 6-12, heights 0-56
BAR_ROW = 6
BAR_ROWS = 7
BAR_COLS = [0, 2, 4, 6, 8, 10, 12, 26, 28, 30, 32, 34, 36, 38]
BAR_MAX = 8 * BAR_ROWS


def hexes(seq):
    return ', '.join(f'${x:02x}' for x in seq)


def mc_char(rows):
    """8 strings of 4 MC pixels ('.', '1', '2', '3') -> 8 bytes."""
    out = []
    for r in rows:
        b = 0
        for ch in r:
            b = b << 2 | '.123'.index(ch)
        out.append(b)
    return bytes(out)


class Charset:
    def __init__(self):
        self.chars = [bytes(8)]
        self.index = {bytes(8): 0}

    def add(self, data):
        data = bytes(data)
        if data not in self.index:
            self.index[data] = len(self.chars)
            self.chars.append(data)
        assert len(self.chars) <= 256
        return self.index[data]


def build_sky():
    cs = Charset()
    screen = [[0] * 40 for _ in range(SKY_ROWS)]
    # the sun: MC pixels (2 hires pixels wide) inside the ellipse
    for row in range(SKY_ROWS):
        for col in range(14, 26):
            rows = []
            for line in range(8):
                y = TOP + row * 8 + line
                s = ''
                for px in range(4):
                    x = col * 8 + px * 2 + 1
                    inside = ((x - SUN_CX) / SUN_RX) ** 2 + ((y + 0.5 - SUN_CY) / SUN_RY) ** 2 <= 1
                    s += '2' if inside and y < HORIZON else '.'
                rows.append(s)
            screen[row][col] = cs.add(mc_char(rows))
    # stars
    import random
    rnd = random.Random(1983)
    stars = []
    star_chars = []
    for pattern in (['....', '....', '..1.', '....', '....', '....', '....', '....'],
                    ['....', '....', '....', '....', '....', '.1..', '....', '....'],
                    ['....', '..1.', '.111', '..1.', '....', '....', '....', '....']):
        star_chars.append(cs.add(mc_char(pattern)))
    while len(stars) < 26:
        row, col = rnd.randrange(0, BAR_ROW), rnd.randrange(40)
        if 13 <= col <= 26 and row >= 1:
            continue
        if any(abs(r - row) + abs(c - col) < 3 for r, c in stars):
            continue
        stars.append((row, col))
        screen[row][col] = star_chars[rnd.randrange(2)]
    # buildings: for each variant and side, k = 1..8 lines filled from the bottom
    windows = {('A', 'L'): (1, 3), ('A', 'R'): (0, 2), ('B', 'L'): (1,), ('B', 'R'): (2,)}
    lit_rows = (1, 2, 5, 6)
    body = {}
    for (var, side), wcols in windows.items():
        for k in range(1, 9):
            rows = []
            for line in range(8):
                filled = line >= 8 - k
                roof = line == 8 - k
                s = ''
                for px in range(4):
                    if not filled:
                        s += '.'
                    elif not roof and px in wcols and line in lit_rows and (var == 'A' or line < 4):
                        s += '1'
                    else:
                        s += '3'
                rows.append(s)
            body[var, side, k] = cs.add(mc_char(rows))
    # per bar and row: the character for every height
    bar_tables = []
    for var in 'AB':
        for side in 'LR':
            for r in range(BAR_ROWS):
                t = []
                for h in range(BAR_MAX + 1):
                    top = BAR_MAX - h                   # first filled pixel line
                    k = max(0, min(8, 8 * r + 8 - top))
                    t.append(body[var, side, k] if k else 0)
                bar_tables.append(t)
    return cs, screen, stars, star_chars, bar_tables


def ground_x(i, y):
    return 160 + i * GRID_G * (y - VPY)


def build_ground():
    import numpy as np
    bm = np.zeros((GROUND_ROWS * 8, 320), bool)
    for i in range(-GRID_N, GRID_N + 1):
        for y in range(HORIZON, BOTTOM):
            lo, hi = sorted((ground_x(i, y), ground_x(i, y + 1)))
            for x in range(int(round(lo)), int(round(hi)) + 1):
                if 0 <= x < 320:
                    bm[y - HORIZON, x] = True
    cs = Charset()
    screen = [[0] * 40 for _ in range(GROUND_ROWS)]
    for r in range(GROUND_ROWS):
        for c in range(40):
            cell = bm[r * 8:r * 8 + 8, c * 8:c * 8 + 8]
            data = [int(''.join('1' if p else '0' for p in row), 2) for row in cell]
            screen[r][c] = cs.add(data)
    return cs, screen, bm


def ground_lines():
    """Per phase: the lines of the horizontal grid lines that the kernel
    colours, and the near ones (line, thickness) for the interrupts."""
    phases = []
    for p in range(GROUND_PHASES):
        ys, near = [], []
        n = 1
        while True:
            z = n - p / GROUND_PHASES
            y = VPY + round(GROUND_DEPTH / z)
            if y < 160:
                break
            if y < KEND:
                ys.append(y)
            elif y < BOTTOM:
                y = max(y, NEAR)
                near.append((y, min(2 if y >= 214 else 1, BOTTOM - y)))
            n += 1
        phases.append((ys, sorted(near)))
    return phases


def stripes(p):
    """Sun lines that are gaps at stripe phase p."""
    gaps = []
    for y in range(STRIPE_TOP, HORIZON):
        u = (y - STRIPE_TOP) / (HORIZON - STRIPE_TOP)
        thick = 1 + 3.2 * u
        pos = (y - STRIPE_TOP - p * STRIPE_PERIOD / STRIPE_PHASES) % STRIPE_PERIOD
        if pos < thick:
            gaps.append(y)
    return gaps


# --- the logo --------------------------------------------------------------
# 10 MC pixels wide, 21 lines, bold and upright.
GLYPHS = {
    'S': ['.#########', '##########', '##########', '###.......', '###.......', '###.......', '###.......', '###.......', '#########.', '##########', '.#########', '.......###', '.......###', '.......###', '.......###', '.......###', '.......###', '.......###', '##########', '##########', '#########.'],
    'I': ['##########', '##########', '##########', '...####...', '...####...', '...####...', '...####...', '...####...', '...####...', '...####...', '...####...', '...####...', '...####...', '...####...', '...####...', '...####...', '...####...', '...####...', '##########', '##########', '##########'],
    'D': ['########..', '#########.', '##########', '###....###', '###....###', '###....###', '###....###', '###....###', '###....###', '###....###', '###....###', '###....###', '###....###', '###....###', '###....###', '###....###', '###....###', '###....###', '##########', '#########.', '########..'],
    'W': ['###....###', '###....###', '###....###', '###....###', '###....###', '###....###', '###....###', '###....###', '###....###', '###....###', '###....###', '###....###', '###.##.###', '###.##.###', '###.##.###', '###.##.###', '###.##.###', '###.##.###', '##########', '####..####', '###....###'],
    'A': ['..######..', '.########.', '##########', '###....###', '###....###', '###....###', '###....###', '###....###', '###....###', '##########', '##########', '##########', '###....###', '###....###', '###....###', '###....###', '###....###', '###....###', '###....###', '###....###', '###....###'],
    'V': ['###....###', '###....###', '###....###', '###....###', '###....###', '###....###', '###....###', '###....###', '###....###', '###....###', '###....###', '###....###', '.###..###.', '.###..###.', '.###..###.', '..######..', '..######..', '..######..', '...####...', '...####...', '...####...'],
    'E': ['##########', '##########', '##########', '###.......', '###.......', '###.......', '###.......', '###.......', '########..', '########..', '########..', '###.......', '###.......', '###.......', '###.......', '###.......', '###.......', '###.......', '##########', '##########', '##########'],
}
LOGO = 'SIDWAVE'


def logo_sprite(ch):
    """A multicolour sprite in chrome bands: light %01 on top, the letter's
    colour %10 in the middle, %11 below."""
    g = GLYPHS[ch]
    data = []
    for y in range(21):
        row = ['.'] * 12
        for x, p in enumerate(g[y]):
            if p == '#':
                row[x + 1] = '1' if y < 5 else '2' if y < 11 else '3'
        bits = 0
        for p in row:
            bits = bits << 2 | '.123'.index(p) if p != '.' else bits << 2
        data += [bits >> 16 & 0xFF, bits >> 8 & 0xFF, bits & 0xFF]
    return data + [0]


# --- the raster kernel -----------------------------------------------------
# Lines 51-251. Block L stores the colours of line L in the cycles that
# are not drawn (62 of line L-1, 3 and 7 of line L), then loads those of
# line L+1, then waits until cycle 59. On a bad line the VIC takes cycles
# 12-54: the loads must be done by cycle 11, then a NOP stalls until 55.

class Sim:
    """Cycle count of a block, with the bad line stall (reads at 12-54
    wait until 55)."""

    def __init__(self, badline, start=59):
        self.c = start              # cycle in the line, 59..63 then 64.. = 1.. of the next line
        self.badline = badline
        self.writes = []

    def cyc(self, kind):
        lc = self.c - 63            # cycle in the block's own line (<=0: previous line)
        if self.badline and kind == 'r' and 12 <= lc <= 54:
            self.c += 55 - lc
        if kind == 'w':
            self.writes.append(self.c - 63)
        self.c += 1

    def ins(self, pattern):
        for k in pattern:
            self.cyc(k)


TIMING = {'sta': 'rrrw', 'stx': 'rrrw', 'sty': 'rrrw', 'lda': 'rrrr', 'ldx': 'rrrr', 'ldy': 'rrrr',
          'lda#': 'rr', 'ldx#': 'rr', 'ldy#': 'rr', 'nop': 'rr', 'bit': 'rrr'}


def kernel_writes():
    """For each line L: [(register, source)], source = ('tab', name) or ('imm', value)."""
    W = {}
    for L in range(TOP, KEND):
        w = [('$d021', ('tab', 'kd021'))]
        if L < HORIZON:
            w.append(('$d023', ('tab', 'kd023')))
        W[L] = w
    W[TOP + 8 * BAR_ROW + 1].append(('$d022', ('imm', 7)))       # windows: yellow
    W[HORIZON] += [('$d016', ('imm', 0x08)), ('$d018', ('imm', 0x04))]  # hires, ground characters
    return W


def assign(w):
    """Registers for the writes of a line: tables in A then X, immediates in Y then X."""
    regs = {}
    free = ['a', 'x', 'y']
    for reg, src in w:
        if src[0] == 'tab':
            r = free.pop(0)
        else:
            r = 'y' if 'y' in free else free[-1]
            free.remove(r)
        regs[reg] = r
    return regs


def kernel():
    W = kernel_writes()
    out = [f'; --- raster kernel, generated by gfx.py: lines {TOP}-{KEND - 1} ---',
           'kernel']
    regs_next = assign(W[TOP])
    # the loads for line 51 come before the kernel
    pre = []
    for reg, src in W[TOP]:
        r = regs_next[reg]
        pre.append(f'        ld{r} ' + (f'{src[1]}+0' if src[0] == 'tab' else f'#${src[1]:02x}'))
    out[1:1] = ['kernel_pre'] + pre + ['        rts']
    total_bytes = 0
    for L in range(TOP, KEND):
        badline = (L & 7) == 3 and L < 248
        sim = Sim(badline)
        regs = regs_next
        code = [f'; line {L}' + (' (bad line)' if badline else '')]
        for reg, src in W[L]:
            r = regs[reg]
            code.append(f'        st{r} {reg}')
            sim.ins(TIMING['sta'])
        if L < KEND - 1:
            regs_next = assign(W[L + 1])
            for reg, src in W[L + 1]:
                r = regs_next[reg]
                if src[0] == 'tab':
                    code.append(f'        ld{r} {src[1]}+{L + 1 - TOP}')
                    sim.ins(TIMING['lda'])
                else:
                    code.append(f'        ld{r} #${src[1]:02x}')
                    sim.ins(TIMING['lda#'])
            # wait until cycle 59 of this line (= 122 in block counting)
            end = 63 + 59
            if badline:
                while sim.c < end:
                    code.append('        nop')
                    sim.ins(TIMING['nop'])
            else:
                left = end - sim.c
                assert left >= 2, (L, left)
                if left % 2:
                    code.append('        bit $ea')
                    sim.ins(TIMING['bit'])
                    left -= 3
                if left >= 14:
                    k = (left - 12) // 2               # jsr + rts = 12, then k nops
                    code.append(f'        jsr wait{k}')
                    sim.c += 12 + 2 * k
                    left -= 12 + 2 * k
                while left > 0:
                    code.append('        nop')
                    sim.ins(TIMING['nop'])
                    left -= 2
            assert sim.c == end, (L, sim.c)
        assert all(-4 <= w <= 11 for w in sim.writes), (L, sim.writes)
        out += code
    out.append('        rts')
    out.append('; jsr waitK takes 12 + 2K cycles: K nops, then the rts')
    for k in range(40, 0, -1):
        out.append(f'{"wait" + str(k):<8}nop')
    out.append('wait0   rts')
    return '\n'.join(out)


def drawbars():
    """Redraws the buildings whose height (barh) differs from the drawn one (bard)."""
    out = ['; --- buildings, generated by gfx.py: drawbars_a the left half, _b the right ---']
    for b, col in enumerate(BAR_COLS):
        if b == 0:
            out.append('drawbars_a')
        if b == len(BAR_COLS) // 2:
            out += ['        rts', 'drawbars_b']
        out += [f'        ldx barh+{b}', f'        cpx bard+{b}', f'        beq _skip{b}',
                f'        stx bard+{b}']
        for side in range(2):
            for r in range(BAR_ROWS):
                out += [f'        lda bartab{((b % 2) * 2 + side) * BAR_ROWS + r},x',
                        f'        sta SCREEN+{(BAR_ROW + r) * 40 + col + side}']
        out.append(f'_skip{b}')
    out.append('        rts')
    return '\n'.join(out)


# --- the scroller text -----------------------------------------------------
TEXT = (
    "      SIDWAVE ... A SUNSET ON THE GRID, WITH A TUNE MADE THE WAY THE TRACKERS TAUGHT US.   "
    "THE MUSIC: HARD RESTART (THE GATE GOES OFF AND ADSR TO ZERO TWO FRAMES BEFORE EVERY NOTE, "
    "SO EVERY ATTACK IS ON TIME) ... WAVE TABLES: THE DRUMS LIVE INSIDE THE BASS AND THE CHORDS, "
    "A KICK FALLS INTO A BASS NOTE AND A SNARE INTO AN ARPEGGIO ON THE SAME VOICE ... "
    "50 HZ ARPEGGIOS FOR THE CHORDS ... SLOW PULSE WIDTH SWEEPS ... A FILTER ENVELOPE ON EVERY "
    "BASS NOTE ... DELAYED VIBRATO ON THE LEAD ... AN ECHO FROM THE THIRD VOICE IN THE BREAK ... "
    "AND A KEY CHANGE FOR THE LAST CHORUS.   "
    "THE PICTURE: THE SKY AND THE SUN ARE COLOURS CHANGED ON EVERY RASTER LINE, THE CITY IS A "
    "SPECTRUM ANALYSER OF THE THREE VOICES, THE BORDERS ARE OPEN FOR THE LOGO AND THIS TEXT.   "
    "CODE, MUSIC AND GRAPHICS BY SAMPOSOFT 2026 ... PRESS SPACE TO EXIT ...        ")


def screen_code(ch):
    if 'A' <= ch <= 'Z':
        return ord(ch) - 64
    if ch == '@':
        return 0
    return ord(ch) if ' ' <= ch <= '?' else 32


# --- output ----------------------------------------------------------------
def main():
    sky_cs, sky_screen, stars, star_chars, bar_tables = build_sky()
    gnd_cs, gnd_screen, bm = build_ground()
    files = {}
    L = None

    def start(name):
        nonlocal L
        L = files[name] = ['; Generated by gfx.py. Do not edit.', '']

    def table(name, data, per_line=32):
        data = list(data)
        L.append(name)
        for i in range(0, len(data), per_line):
            L.append('        .byte ' + hexes(data[i:i + per_line]))

    # placed by sidwave.asm in the VIC bank
    start('gfx_sky.asm')
    table('skychars', b''.join(sky_cs.chars))
    start('gfx_ground.asm')
    table('gndchars', b''.join(gnd_cs.chars))
    start('gfx_screen.asm')
    screen = [c for row in sky_screen for c in row] + [c for row in gnd_screen for c in row]
    table('screendata', screen, 40)
    start('gfx_sprites.asm')
    sprites = []
    for ch in LOGO:
        sprites += logo_sprite(ch)
    table('logosprites', sprites, 32)
    start('gfx_data.asm')
    colours = [0x08] * (SKY_ROWS * 40)
    for r in range(GROUND_ROWS):
        colours += [GRID_ROW_COLOURS[r]] * 40
    table('colourdata', colours, 40)
    # kernel tables: initial values
    kd021 = [SKY.get(y, GROUND_BG.get(y, 0)) for y in range(TOP, KEND)]
    L.append(f'KLINES = {KEND - TOP}')
    table('kd021_init', kd021)
    table('skycol', [SKY[y] for y in range(TOP, HORIZON)])
    table('gndbg', [GROUND_BG[y] for y in range(HORIZON, KEND)])
    table('linecol', [LINE_COLOURS[y] for y in range(HORIZON, BOTTOM)])
    table('flashcol', [FLASH_COLOURS[y] for y in range(HORIZON, BOTTOM)])
    # the sun gradient for the fade levels (lines SUN_TOP..154), and the sky behind it
    for lvl, ramp in enumerate(FADE):
        table(f'sungrad{lvl}', [SKY[y] if ramp[SUN[y]] is None else ramp[SUN[y]]
                                for y in range(SUN_TOP, HORIZON)])
    L.append(f'SUN_TOP = {SUN_TOP}')
    L.append(f'SUN_LINES = {HORIZON - SUN_TOP}')
    L.append(f'FADE_LEVELS = {len(FADE)}')
    L.append('sungrad_lo .byte ' + ', '.join(f'<sungrad{i}' for i in range(len(FADE))))
    L.append('sungrad_hi .byte ' + ', '.join(f'>sungrad{i}' for i in range(len(FADE))))
    # stripes: per phase, the gap lines as kd023 indices, $ff ends
    for p in range(STRIPE_PHASES):
        table(f'stripe{p}', [y - TOP for y in stripes(p)] + [0xFF])
    L.append('stripe_lo .byte ' + ', '.join(f'<stripe{p}' for p in range(STRIPE_PHASES)))
    L.append('stripe_hi .byte ' + ', '.join(f'>stripe{p}' for p in range(STRIPE_PHASES)))
    # ground lines: per phase, kd021 indices, $ff ends
    phases = ground_lines()
    for p, (ys, near) in enumerate(phases):
        table(f'gline{p}', [y - TOP for y in ys] + [0xFF])
        table(f'gnear{p}', [v for yt in near for v in yt] + [0xFF])
    L.append('gline_lo .byte ' + ', '.join(f'<gline{p}' for p in range(GROUND_PHASES)))
    L.append('gline_hi .byte ' + ', '.join(f'>gline{p}' for p in range(GROUND_PHASES)))
    L.append('gnear_lo .byte ' + ', '.join(f'<gnear{p}' for p in range(GROUND_PHASES)))
    L.append('gnear_hi .byte ' + ', '.join(f'>gnear{p}' for p in range(GROUND_PHASES)))
    L.append(f'MAX_GLINES = {max(len(ys) for ys, _ in phases)}')
    L.append(f'MAX_NEAR = {max(len(n) for _, n in phases)}')
    # buildings
    for i, t in enumerate(bar_tables):
        table(f'bartab{i}', t, 57)
    L.append(f'BARS = {len(BAR_COLS)}')
    L.append(f'BAR_ROW = {BAR_ROW}')
    L.append(f'BAR_ROWS = {BAR_ROWS}')
    L.append(f'BAR_MAX = {BAR_MAX}')
    L.append('barcol  .byte ' + hexes(BAR_COLS))
    # stars: screen offsets and the two characters (dim, bright)
    L.append('starlo  .byte ' + ', '.join(f'<(SCREEN+{r * 40 + c})' for r, c in stars))
    L.append('starhi  .byte ' + ', '.join(f'>(SCREEN+{r * 40 + c})' for r, c in stars))
    L.append(f'STARS = {len(stars)}')
    L.append(f'STAR_DIM = {star_chars[0]}')
    L.append(f'STAR_DIM2 = {star_chars[1]}')
    L.append(f'STAR_BRIGHT = {star_chars[2]}')
    table('scrolltext', [screen_code(c) for c in TEXT] + [0xFF])
    L.append(f'LOGO_LETTERS = {len(LOGO)}')
    import math
    table('wave8', [round(4 + 4 * math.sin(2 * math.pi * i / 256)) for i in range(256)])
    table('wave6', [round(3 + 3 * math.sin(2 * math.pi * i / 256)) for i in range(256)])
    fbar = []
    for fhi in range(256):
        hz = (fhi * 256 + 128) * CLOCK / (1 << 24)
        note = 12 * math.log2(hz / 440) + 57
        fbar.append(max(0, min(len(BAR_COLS) - 1, round((note - 28) / 4))))
    table('fbar', fbar)
    start('gfx_code.asm')
    L.append(drawbars())
    L.append(kernel())
    for name, lines in files.items():
        with open(os.path.join(HERE, name), 'w') as f:
            f.write('\n'.join(lines) + '\n')
    print(f'gfx: sky {len(sky_cs.chars)} chars, ground {len(gnd_cs.chars)} chars, '
          f'{max(len(ys) for ys, _ in phases)} + {max(len(n) for _, n in phases)} grid lines max')
    if '--preview' in sys.argv:
        preview(sky_cs, sky_screen, gnd_cs, gnd_screen, kd021, phases)


PALETTE = [(0, 0, 0), (255, 255, 255), (129, 51, 56), (117, 206, 200), (142, 60, 151),
           (86, 172, 77), (46, 44, 155), (237, 241, 113), (142, 80, 41), (85, 56, 0),
           (196, 108, 113), (74, 74, 74), (123, 123, 123), (169, 255, 159), (112, 109, 235),
           (178, 178, 178)]


def preview(sky_cs, sky_screen, gnd_cs, gnd_screen, kd021, phases):
    from PIL import Image
    img = Image.new('RGB', (320, 200))
    px = img.load()
    k = list(kd021) + [0] * (BOTTOM - KEND)
    for y in phases[0][0]:
        k[y - TOP] = LINE_COLOURS[y]
    for y, t in phases[0][1]:
        for i in range(t):
            k[y + i - TOP] = LINE_COLOURS[y]
    for y in range(200):
        line = TOP + y
        row, sub = divmod(y, 8)
        bg = k[y]
        if line < HORIZON:
            sun = FADE[0][SUN[line]] if line in SUN else bg
            if line in stripes(0):
                sun = bg
            star = 1 if row < BAR_ROW else 7
            for col in range(40):
                byte = sky_cs.chars[sky_screen[row][col]][sub]
                for p in range(4):
                    v = byte >> (6 - 2 * p) & 3
                    c = (bg, star, sun, 0)[v]
                    px[col * 8 + 2 * p, y] = px[col * 8 + 2 * p + 1, y] = PALETTE[c]
        else:
            gr = row - SKY_ROWS
            for col in range(40):
                byte = gnd_cs.chars[gnd_screen[gr][col]][sub]
                for p in range(8):
                    c = GRID_ROW_COLOURS[gr] if byte >> (7 - p) & 1 else bg
                    px[col * 8 + p, y] = PALETTE[c]
    img.resize((640, 400), Image.NEAREST).save(os.path.join(HERE, 'build', 'preview.png'))


if __name__ == '__main__':
    main()
