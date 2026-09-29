#!/usr/bin/env python3
"""STARFALL data generator: writes data.asm (collision maps, table picture,
lamps, sprites, font, maths tables) and consts.asm.

    python3 gen_data.py [--preview table.png]

The table is drawn from the shapes in table.py, the same ones that give the
collision maps, so what the player sees is what the ball hits.
"""

import math
import os
import sys

import numpy as np
from PIL import Image

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)
import table as T  # noqa: E402
import sound  # noqa: E402

# emulator palette (src/vic/mod.rs, C64_PALETTE)
PALETTE = [
    0x000000, 0xFFFFFF, 0x813338, 0x75CEC8, 0x8E3C97, 0x56AC4D, 0x2E2C9B, 0xEDF171,
    0x8E5029, 0x553800, 0xC46C71, 0x4A4A4A, 0x7B7B7B, 0xA9FF9F, 0x706DEB, 0xB2B2B2,
]
RGB = np.array([[(c >> 16) & 255, (c >> 8) & 255, c & 255] for c in PALETTE], float)

BLACK, WHITE, RED, CYAN, PURPLE, GREEN, BLUE, YELLOW = range(8)
ORANGE, BROWN, LRED, DGREY, GREY, LGREEN, LBLUE, LGREY = range(8, 16)

BG = BLUE                 # $D021: the playfield
TABLE_FX = 52             # the table covers fat pixels 0-51 (x 0-207)... cells 0-25
PANEL_C0 = 26             # the panel: cells 26-39

# collision map: 2x2-pixel cells, x 16-199, y 0-199
MAP_X0 = 16
MAP_W = 92
MAP_H = 100

MAT_WALL, MAT_RUBBER, MAT_SPECIAL = T.M_WALL, T.M_RUBBER, T.M_SPECIAL


def out(*a):
    print(*a, file=sys.stderr)


# ================================================================ font

FONT3 = {
    '0': "###|#.#|#.#|#.#|###", '1': ".#.|##.|.#.|.#.|###", '2': "###|..#|###|#..|###",
    '3': "###|..#|.##|..#|###", '4': "#.#|#.#|###|..#|..#", '5': "###|#..|###|..#|###",
    '6': "###|#..|###|#.#|###", '7': "###|..#|..#|.#.|.#.", '8': "###|#.#|###|#.#|###",
    '9': "###|#.#|###|..#|###",
    'A': "###|#.#|###|#.#|#.#", 'B': "##.|#.#|##.|#.#|##.", 'C': ".##|#..|#..|#..|.##",
    'D': "##.|#.#|#.#|#.#|##.", 'E': "###|#..|##.|#..|###", 'F': "###|#..|##.|#..|#..",
    'G': ".##|#..|#.#|#.#|.##", 'H': "#.#|#.#|###|#.#|#.#", 'I': "###|.#.|.#.|.#.|###",
    'J': "..#|..#|..#|#.#|.#.", 'K': "#.#|#.#|##.|#.#|#.#", 'L': "#..|#..|#..|#..|###",
    'M': "#.#|###|###|#.#|#.#", 'N': "##.|#.#|#.#|#.#|#.#", 'O': ".#.|#.#|#.#|#.#|.#.",
    'P': "##.|#.#|##.|#..|#..", 'Q': ".#.|#.#|#.#|##.|.##", 'R': "##.|#.#|##.|#.#|#.#",
    'S': ".##|#..|.#.|..#|##.", 'T': "###|.#.|.#.|.#.|.#.", 'U': "#.#|#.#|#.#|#.#|###",
    'V': "#.#|#.#|#.#|#.#|.#.", 'W': "#.#|#.#|###|###|#.#", 'X': "#.#|#.#|.#.|#.#|#.#",
    'Y': "#.#|#.#|.#.|.#.|.#.", 'Z': "###|..#|.#.|#..|###",
    '!': ".#.|.#.|.#.|...|.#.", '-': "...|...|###|...|...", '.': "...|...|...|...|.#.",
    ':': "...|.#.|...|.#.|...", ' ': "...|...|...|...|...", 'x': "...|#.#|.#.|#.#|...",
    '/': "..#|..#|.#.|#..|#..", "'": ".#.|.#.|...|...|...", '?': "##.|..#|.#.|...|.#.",
    '=': "...|###|...|###|...",
}
FONT_CHARS = " 0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZ!-.:x/'?="


def glyph(ch):
    return [[c == '#' for c in row] for row in FONT3[ch].split('|')]


def glyph7(ch):
    """3x5 glyph stretched to 3x7 (rows 1 and 3 doubled)."""
    g = glyph(ch)
    return [g[i] for i in (0, 1, 1, 2, 3, 3, 4)]


def glyph14(ch):
    g = glyph(ch)
    return [g[i] for i in (0, 0, 1, 1, 1, 1, 2, 2, 3, 3, 3, 3, 4, 4)]


LOGO5 = {
    'S': ".####|#....|#....|.###.|....#|....#|####.",
    'T': "#####|..#..|..#..|..#..|..#..|..#..|..#..",
    'A': ".###.|#...#|#...#|#####|#...#|#...#|#...#",
    'R': "####.|#...#|#...#|####.|#.#..|#..#.|#...#",
    'F': "#####|#....|#....|####.|#....|#....|#....",
    'L': "#....|#....|#....|#....|#....|#....|#####",
}

# ================================================================ picture


class Picture:
    """Multicolour bitmap built at fat-pixel resolution (160x200).

    `col` holds a C64 colour per fat pixel; `lamp` the lamp id (or -1) of
    pixels that must use the colour RAM slot."""

    def __init__(self):
        self.col = np.full((200, 160), BG, np.int16)
        self.lamp = np.full((200, 160), -1, np.int16)

    def put(self, fx, fy, c, lamp=-1):
        if 0 <= fx < 160 and 0 <= fy < 200:
            self.col[fy, fx] = c
            self.lamp[fy, fx] = lamp

    def text(self, fx, fy, s, c, lamp=-1, rows=5, clear=True):
        if clear:                       # no stars inside the letters
            self.col[max(0, fy - 1):fy + rows + 1, max(0, fx - 1):fx + 4 * len(s)] = BG
        for ch in s:
            g = glyph(ch) if rows == 5 else glyph7(ch)
            for r, row in enumerate(g):
                for i, on in enumerate(row):
                    if on:
                        self.put(fx + i, fy + r, c, lamp)
            fx += 4

    def encode(self, lamps_off):
        """Bitmap, screen RAM and colour RAM with at most 4 colours per cell.
        Cells over the limit keep their most used colours; the others become
        the nearest kept colour (reported)."""
        bitmap = bytearray(8000)
        screen = bytearray(1000)
        colram = bytearray(1000)
        merged = 0
        for cy in range(25):
            for cx in range(40):
                blk = self.col[cy * 8:cy * 8 + 8, cx * 4:cx * 4 + 4]
                lmp = self.lamp[cy * 8:cy * 8 + 8, cx * 4:cx * 4 + 4]
                lamp_ids = set(int(v) for v in lmp.flat if v >= 0)
                if len(lamp_ids) > 1:
                    raise SystemExit(f"cell {cx},{cy}: lamps {lamp_ids}")
                counts = {}
                for v, l in zip(blk.flat, lmp.flat):
                    if l < 0 and v != BG:
                        counts[int(v)] = counts.get(int(v), 0) + 1
                room = 2 if lamp_ids else 3
                keep = sorted(counts, key=lambda c: -counts[c])[:room]
                if len(counts) > room:
                    merged += 1
                    out(f"  cell {cx},{cy}: colours {counts} -> {keep}")
                slots = {BG: 0}
                for i, c in enumerate(keep):
                    slots[c] = i + 1
                scr = [0, 0, 0]          # colours of %01, %10, %11
                for c, s in slots.items():
                    if s:
                        scr[s - 1] = c
                if lamp_ids:
                    lid = lamp_ids.pop()
                    scr[2] = lamps_off[lid]
                screen[cy * 40 + cx] = (scr[0] << 4) | scr[1]
                colram[cy * 40 + cx] = scr[2]
                for r in range(8):
                    b = 0
                    for i in range(4):
                        v, l = int(blk[r, i]), int(lmp[r, i])
                        if l >= 0:
                            s = 3
                        elif v in slots:
                            s = slots[v]
                        else:
                            s = min(slots, key=lambda c: np.sum((RGB[c] - RGB[v]) ** 2) if c != BG
                                    else 1e9 if keep else 0)
                            s = slots[s]
                        b = (b << 2) | s
                    bitmap[cy * 320 + cx * 8 + r] = b
        if merged:
            out(f"{merged} cells over the colour limit")
        return bitmap, screen, colram

    def rgb(self, lamps_on=None):
        img = RGB[self.col].astype(np.uint8)
        return np.repeat(img, 2, axis=1)


def render_rgb(bitmap, screen, colram):
    """What the VIC shows (320x200 RGB) for a multicolour bitmap."""
    img = np.zeros((200, 320, 3), np.uint8)
    for cy in range(25):
        for cx in range(40):
            sc = screen[cy * 40 + cx]
            cols = [BG, sc >> 4, sc & 15, colram[cy * 40 + cx]]
            for r in range(8):
                b = bitmap[cy * 320 + cx * 8 + r]
                for i in range(4):
                    s = (b >> (6 - 2 * i)) & 3
                    img[cy * 8 + r, cx * 8 + 2 * i:cx * 8 + 2 * i + 2] = RGB[cols[s]]
    return img


# ================================================================ lamps

LAMPS = []                # (name, off colour, on colour)


def lamp(name, off, on):
    LAMPS.append((name, off, on))
    return len(LAMPS) - 1


L_LANE = [lamp(f"lane{i}", BROWN, YELLOW) for i in range(3)]
L_BUMP = [lamp(f"bump{i}", ORANGE, YELLOW) for i in range(3)]
L_SLING = [lamp(f"sling{i}", PURPLE, LRED) for i in range(2)]
L_STAR = [lamp(f"star{i}", DGREY, YELLOW) for i in range(4)]
L_MULT = [lamp(f"mult{i}", DGREY, LGREEN) for i in range(4)]
L_AGAIN = lamp("again", DGREY, LRED)
L_EXTRA = lamp("extra", DGREY, LGREEN)
L_SAVE = lamp("save", DGREY, CYAN)


# ================================================================ table art


def fat_grid():
    fy, fx = np.mgrid[0:200, 0:160]
    return fx, fy, fx * 2 + 1.0, fy + 0.5


def draw_table(pic, targets_up=(True, True, True)):
    fx, fy, x, y = fat_grid()
    d, idx = T.sdf_all(x, y)
    nx, ny = T.normal_at(x, y)
    mats = np.array([s.mat for s in T.SHAPES])[idx]
    objs = np.array([-1 if s.obj is None else s.obj for s in T.SHAPES])[idx]
    kinds = np.array([s.kind for s in T.SHAPES])[idx]
    table = x < 208
    lit = (-nx * 0.6 - ny * 0.8)          # light from the top left
    col = pic.col
    lampm = pic.lamp

    # stars on the playfield (deterministic)
    rng = np.random.RandomState(7)
    for _ in range(90):
        sx, sy = rng.randint(4, 102), rng.randint(2, 198)
        c = [WHITE, LBLUE, LGREY][rng.randint(3)]
        if d[sy, sx] > 6:
            col[sy, sx] = c

    # outside of the table: black cabinet with a light blue rim
    outside = (kinds == "outside") & (d < 0) & table
    col[outside] = BLACK
    rim = outside & (d > -2.6)
    col[rim] = LBLUE
    col[rim & (d > -1.3) & (lit > 0.25)] = WHITE

    # the upper-left block is cabinet too
    block = (kinds == "poly") & (d < 0) & (y < 100) & table
    col[block] = BLACK
    brim = block & (d > -2.6)
    col[brim] = LBLUE
    col[brim & (d > -1.3) & (lit > 0.25)] = WHITE
    outside |= block

    solid = (d < 0) & table & ~outside
    walls = solid & (mats == MAT_WALL)
    col[walls] = LBLUE
    col[walls & (d > -1.1) & (lit > 0.25)] = WHITE
    rub = solid & (mats == MAT_RUBBER)
    col[rub] = WHITE

    # slingshots: body lamp, white rubber on the kicking face
    for side in (0, 1):
        body = solid & (kinds == "poly") & (x > 100 if side else x < 100) & (y > 120)
        inner = body & (d < -1.2)
        col[body & ~inner] = LBLUE
        lampm[inner] = L_SLING[side]
        col[inner] = PURPLE
        face = solid & (objs == 3 + side)
        col[face] = WHITE
        lampm[face] = -1

    # bumpers: white skirt, red ring, lamp cap
    for i, (bx, by, br) in enumerate(T.BUMPERS):
        r = np.hypot(x - bx, y - by)
        col[(r < br) & table] = WHITE
        col[(r < br - 1.6) & table] = RED
        cap = (r < br - 4.3) & table
        col[cap] = ORANGE
        lampm[cap] = L_BUMP[i]

    # S-T-A-R stand-up targets and their letters
    fxv, fyv = T.BLOCK_BOT[0] - T.BLOCK_TOP[0], T.BLOCK_BOT[1] - T.BLOCK_TOP[1]
    fl = math.hypot(fxv, fyv)
    ux, uy = fxv / fl, fyv / fl
    nxf, nyf = uy, -ux
    for i, t in enumerate(T.STAR_T):
        tg = solid & (objs == 5 + i)
        col[tg] = ORANGE
        cx = T.BLOCK_TOP[0] + fxv * t + nxf * 13
        cy = T.BLOCK_TOP[1] + fyv * t + nyf * 13
        cellx, celly = int(cx) // 8, int(cy) // 8
        pic.text(cellx * 4 + 0, celly * 8 + 1, "STAR"[i], DGREY, L_STAR[i], rows=7)

    # drop targets on the left wall
    for i in range(3):
        y0, y1 = T.TARGET_Y[i] + 1, T.TARGET_Y[i + 1] - 1
        box = (y >= y0) & (y < y1) & (x >= 16) & (x < T.TARGET_X)
        if targets_up[i]:
            col[box] = YELLOW
        else:
            col[box] = BG
            col[box & (x < 18)] = BLACK

    # top lane lamps
    for i, lx in enumerate((84, 104, 124)):
        m = (np.hypot(x - lx, (y - 33) * 1.0) < 3.2) & table
        col[m] = BROWN
        lampm[m] = L_LANE[i]
    # arrows over the lanes
    for lx in (84, 104, 124):
        for k in range(3):
            pic.put((lx - 1) // 2 - 1 + k, 45 - (1 if k == 1 else 0), LGREY)
            pic.put((lx - 1) // 2 - 1 + k, 46 - (1 if k == 1 else 0), LGREY)

    # saucer: dark hole with a grey ring
    sx, sy = T.SAUCER
    r = np.hypot(x - sx, y - sy)
    col[(r < 6.5) & table] = LGREY
    col[(r < 6.5) & (r > 4.5) & (y - sy + (x - sx) * 0.4 > 1.5) & table] = GREY
    col[(r < 4.6) & table] = BLACK
    pic.text(68, 81, "EXTRA", DGREY, L_EXTRA, rows=7)

    # the planet between the bumpers and the multiplier lamps
    px_, py_ = 99.5, 106
    body = (np.hypot((x - px_) / 1.0, y - py_) < 7.5) & table
    col[body] = ORANGE
    col[body & (x - px_ + (y - py_) * 0.6 < -3)] = YELLOW
    ring = ((((x - px_) / 16) ** 2 + ((y - py_ - (x - px_) * 0.18) / 3.2) ** 2) < 1) & \
        ((((x - px_) / 12) ** 2 + ((y - py_ - (x - px_) * 0.18) / 1.6) ** 2) > 1) & table
    front = ring & ((y - py_ - (x - px_) * 0.18) > -0.2)
    col[(ring & ~body) | front] = LGREY

    # bonus multipliers
    for i in range(4):
        cx = 7 + i * 3
        pic.text(cx * 4 + 1, 121, f"{i + 2}x", DGREY, L_MULT[i], rows=7)
    # shoot again
    pic.text(40, 145, "SHOOT", DGREY, L_AGAIN, rows=7)
    pic.text(40, 153, "AGAIN", DGREY, L_AGAIN, rows=7)
    # ball save lamp between the flippers
    m = (np.hypot(x - 99.5, y - 196) < 2.6) & table
    col[m] = DGREY
    lampm[m] = L_SAVE

    # plunger lane: arrows
    for k in range(3):
        ay = 150 - k * 14
        for j in range(3):
            pic.put(94 + j, ay + (1 if j == 1 else 0) - 0, LBLUE)
        pic.put(95, ay - 1, LBLUE)

    # left of the table
    col[(x < 16) & ~outside & (fx < TABLE_FX)] = BLACK


def draw_panel(pic):
    col = pic.col
    x0 = PANEL_C0 * 4
    col[:, x0:] = BLACK
    # frame
    col[:, x0] = DGREY
    col[:, 159] = DGREY
    col[0, x0:] = DGREY
    col[199, x0:] = DGREY
    # logo: STARFALL, 5x7 letters with doubled rows, yellow to red
    grad = [YELLOW, YELLOW, YELLOW, YELLOW, ORANGE, ORANGE, ORANGE, ORANGE, ORANGE, ORANGE,
            RED, RED, RED, RED]         # rows 8-15 in cell row 1, 16-21 in row 2
    lx = x0 + 4
    for ch in "STARFALL":
        rows = LOGO5[ch].split('|')
        for r in range(14):
            for i, c in enumerate(rows[r // 2]):
                if c == '#':
                    pic.put(lx + i, 8 + r, grad[r])
        lx += 6
    pic.text(x0 + 14, 25, "PINBALL", LBLUE, rows=5, clear=False)
    # separators
    for yy in (31, 150):
        col[yy, x0 + 2:158] = DGREY


def panel_text_cells():
    """Static labels drawn by the game go through the runtime font."""
    return


# ================================================================ maps


def collision_maps():
    ys, xs = np.mgrid[0:MAP_H, 0:MAP_W]
    cx = MAP_X0 + xs * 2 + 1.0
    cy = ys * 2 + 1.0
    d, idx = T.sdf_all(cx, cy)
    nx, ny = T.normal_at(cx, cy)
    mats = np.array([s.mat for s in T.SHAPES])[idx]
    near = d < T.BALL_R + 1.6
    ang = np.round(np.arctan2(ny, nx) / (2 * math.pi) * 64).astype(int) & 63
    A = np.where(near, (mats << 6) | ang, 0).astype(np.uint8)
    D = np.clip(np.round((T.BALL_R - d) * 16), -127, 127).astype(int)
    D = np.where(near, D, 0) & 255
    return A, D.astype(np.uint8), d


def rle(data):
    """Control byte c: c < $80 copies c+1 literal bytes, c >= $80 repeats the
    next byte c-$7D times (3-130); $00 after a repeat never occurs... the
    stream ends with a zero-length marker $FF $00?  No: the decoder knows
    the size."""
    outb = bytearray()
    i = 0
    n = len(data)
    lit = bytearray()

    def flush():
        nonlocal lit
        while lit:
            chunk = lit[:128]
            outb.append(len(chunk) - 1)
            outb.extend(chunk)
            lit = lit[128:]

    while i < n:
        j = i
        while j < n and data[j] == data[i] and j - i < 130:
            j += 1
        if j - i >= 3:
            flush()
            outb.append(0x80 + (j - i) - 3)
            outb.append(data[i])
            i = j
        else:
            lit.append(data[i])
            i += 1
    flush()
    return bytes(outb)


def unrle(src, size):
    outb = bytearray()
    i = 0
    while len(outb) < size:
        c = src[i]
        i += 1
        if c < 0x80:
            outb.extend(src[i:i + c + 1])
            i += c + 1
        else:
            outb.extend([src[i]] * (c - 0x80 + 3))
            i += 1
    return bytes(outb)


def reach_check():
    """Ball centres reachable from the plunger (flippers down, targets up)."""
    ys, xs = np.mgrid[0:400, 0:420]
    x = xs * 0.5 + 0.25
    y = ys * 0.5 + 0.25
    d = T.sdf_all(x, y)[0]
    for mir, p in ((False, T.LFLIP), (True, T.RFLIP)):
        d = np.minimum(d, T.flipper_poly(T.FLIP_REST, mir)(x, y))
    free = d >= T.BALL_R
    from collections import deque
    start = (int(T.PLUNGER_Y * 2), int(T.LANE_CX * 2))
    seen = np.zeros_like(free)
    q = deque([start])
    seen[start] = True
    while q:
        a, b = q.popleft()
        for da, db in ((1, 0), (-1, 0), (0, 1), (0, -1)):
            u, v = a + da, b + db
            if 0 <= u < 400 and 0 <= v < 420 and free[u, v] and not seen[u, v]:
                seen[u, v] = True
                q.append((u, v))
    # places that must be reachable
    for name, (px, py) in {"top lane 1": (84, 32), "top lane 3": (124, 32),
                           "left inlane": (37, 140), "right outlane": (175, 150),
                           "left outlane": (22, 150),
                           "saucer": T.SAUCER, "drop targets": (26, 112),
                           "between flippers": (98.5, 198)}.items():
        if not seen[int(py * 2), int(px * 2)]:
            raise SystemExit(f"reach check: {name} not reachable")
    out("reach check: ok")


# ================================================================ sprites


def sprite_bytes(pix, mc=False):
    """pix: 21 rows x 24 (hires) or 12 (mc: values 0-3) -> 64 bytes."""
    b = bytearray(64)
    for r in range(21):
        row = pix[r]
        for k in range(3):
            v = 0
            if mc:
                for i in range(4):
                    v = (v << 2) | row[k * 4 + i]
            else:
                for i in range(8):
                    v = (v << 1) | (1 if row[k * 8 + i] else 0)
            b[r * 3 + k] = v
    return b


def ball_sprites():
    body = [[0] * 24 for _ in range(21)]
    shine = [[0] * 24 for _ in range(21)]
    for r in range(7):
        for c in range(7):
            dx, dy = c - 3, r - 3
            if dx * dx + dy * dy <= 10:
                body[r][c] = 1
    for (r, c) in ((1, 2), (1, 3), (2, 1), (2, 2)):
        shine[r][c] = 1
    return sprite_bytes(shine), sprite_bytes(body)


FLIP_FRAMES = list(range(T.FLIP_UP, T.FLIP_REST + 1, 4))    # 11 angles


def flipper_sprites():
    """Two multicolour sprites per angle: %01 = red rubber ($D025), %11 =
    white body ($D026), %10 = grey shade (sprite colour). Returns the sprite
    blocks and the top-left corner (screen x, y) of each frame for both
    sides."""
    frames = {False: [], True: []}
    for mirror in (False, True):
        px, py = T.RFLIP if mirror else T.LFLIP
        for a in FLIP_FRAMES:
            f = T.flipper_poly(a, mirror)
            # sample fat pixels on a 48x40 window around the pivot
            ox = int(px) - 6 if not mirror else int(px) - 42
            ox &= ~1
            ys, xs = np.mgrid[0:40, 0:24]
            X = ox + xs * 2 + 1.0
            Y = py - 20 + ys + 0.5
            d = f(X, Y)
            rows = np.where((d < 0).any(axis=1))[0]
            top = rows.min()
            top = min(top, 40 - 21)
            pix = np.zeros((21, 24), int)
            a_rad = a * 2 * math.pi / 256
            for r in range(21):
                for c in range(24):
                    dd = d[top + r, c]
                    if dd < 0:
                        v = 3
                        if dd > -1.0:
                            v = 1
                        else:
                            # shade the lower half of the body
                            ux, uy = math.cos(a_rad), math.sin(a_rad)
                            if mirror:
                                ux = -ux
                            rx, ry = X[top + r, c] - px, Y[top + r, c] - py
                            perp = rx * uy - ry * ux if not mirror else -(rx * uy - ry * ux)
                            if perp < -1.2:
                                v = 2
                        pix[r, c] = v
            if (d[:top] < 0).any() or (d[top + 21:] < 0).any():
                raise SystemExit(f"flipper frame {a} does not fit")
            left = sprite_bytes(pix[:, :12].tolist(), mc=True)
            right = sprite_bytes(pix[:, 12:].tolist(), mc=True)
            frames[mirror].append((left, right, ox, int(py - 20 + top)))
    return frames


def plunger_sprite():
    pix = [[0] * 12 for _ in range(21)]
    for r in range(21):
        if r < 2:
            for c in range(1, 5):
                pix[r][c] = 1           # red tip ($D025)
        elif r < 4:
            pix[r][2] = pix[r][3] = 3   # white rod ($D026)
        else:
            for c in range(0, 6):       # grey spring (sprite colour) round the rod
                pix[r][c] = 2 if (r + c // 2) % 3 == 0 else (3 if c in (2, 3) else 0)
    return sprite_bytes(pix, mc=True)


# ================================================================ tables


def asm_bytes(name, data, per=16):
    lines = [f"{name}"]
    for i in range(0, len(data), per):
        lines.append("        .byte " + ",".join(f"${b:02x}" for b in data[i:i + per]))
    return "\n".join(lines) + "\n"


def maths_tables():
    s = []
    cos = [round(127 * math.cos(2 * math.pi * i / 256)) & 255 for i in range(256)]
    sin = [round(127 * math.sin(2 * math.pi * i / 256)) & 255 for i in range(256)]
    s.append(asm_bytes("costab", cos))
    s.append(asm_bytes("sintab", sin))
    # quarter squares for the multiplication (page aligned):
    # a * b = sq1[a + b] - sq2[b - a + 255]
    sq1 = [n * n // 4 for n in range(512)]
    sq2 = [(n - 255) ** 2 // 4 for n in range(512)]
    s.append("        .align 256\n")
    s.append(asm_bytes("sq1lo", [v & 255 for v in sq1]))
    s.append(asm_bytes("sq1hi", [v >> 8 for v in sq1]))
    s.append(asm_bytes("sq2lo", [v & 255 for v in sq2]))
    s.append(asm_bytes("sq2hi", [v >> 8 for v in sq2]))
    # flipper: contact radius (1/16 px) by position along the flipper
    rt = [round((T.BALL_R + T.FLIP_R1 + (T.FLIP_R2 - T.FLIP_R1) * a / T.FLIP_L) * 16)
          for a in range(T.FLIP_L + 1)]
    s.append(asm_bytes("fliprad", rt))
    # surface speed (1/256 px per substep) at distance r for 1 unit/substep
    sp = [round(2 * math.pi * r) for r in range(T.FLIP_L + 2)]
    s.append(asm_bytes("flipsurf", sp))
    # tip circle: local offset (along, perp) in half pixels, -12..12
    ang, dist = [], []
    for ia in range(25):
        for ip in range(25):
            ta = (ia - 12 + 0.5) * 0.5
            tp = (ip - 12 + 0.5) * 0.5
            ang.append(round(math.atan2(-tp, ta) / (2 * math.pi) * 256) & 255)
            dist.append(min(255, round(math.hypot(ta, tp) * 16)))
    s.append(asm_bytes("tiprowlo", [((k + 12) * 25) & 255 for k in range(13)]))
    s.append(asm_bytes("tiprowhi", [((k + 12) * 25) >> 8 for k in range(13)]))
    s.append(asm_bytes("tipang", ang))
    s.append(asm_bytes("tipdist", dist))
    # row offsets of the collision maps
    rows = [r * MAP_W for r in range(MAP_H)]
    s.append(asm_bytes("maprowlo", [v & 255 for v in rows]))
    s.append(asm_bytes("maprowhi", [v >> 8 for v in rows]))
    # bitmap row addresses (cell rows)
    s.append(asm_bytes("bmrowlo", [(r * 320) & 255 for r in range(25)]))
    s.append(asm_bytes("bmrowhi", [(r * 320) >> 8 for r in range(25)]))
    s.append(asm_bytes("scrrowlo", [(r * 40) & 255 for r in range(25)]))
    s.append(asm_bytes("scrrowhi", [(r * 40) >> 8 for r in range(25)]))
    return "".join(s)


def font_tables():
    """Runtime font: 8 bytes per character, ink = %11 on %01 (black)."""
    small, big = [], []
    for ch in FONT_CHARS:
        g = glyph7(ch)
        rows = [[False] * 3] + g
        for r in range(8):
            b = 0
            for i in range(4):
                on = i < 3 and rows[r][i]
                b = (b << 2) | (3 if on else 1)
            small.append(b)
    for ch in "0123456789 ":
        g = glyph14(ch)
        rows = [[False] * 3] + g + [[False] * 3]
        for r in range(16):
            b = 0
            for i in range(4):
                on = i < 3 and rows[r][i]
                b = (b << 2) | (3 if on else (2 if (i < 3 and False) else 1))
            big.append(b)
    return asm_bytes("font", small) + asm_bytes("bigfont", big)


# ================================================================ main


def main():
    preview = None
    if "--preview" in sys.argv:
        preview = sys.argv[sys.argv.index("--preview") + 1]

    reach_check()

    # collision maps
    A, D, _ = collision_maps()
    ca, cd = rle(A.tobytes()), rle(D.tobytes())
    assert unrle(ca, A.size) == A.tobytes() and unrle(cd, D.size) == D.tobytes()
    out(f"maps: {A.size} bytes each, packed {len(ca)} + {len(cd)}")

    # picture with the targets up, then the drop target patches
    lamps_off = [l[1] for l in LAMPS]
    pic = Picture()
    draw_table(pic)
    draw_panel(pic)
    bitmap, screen, colram = pic.encode(lamps_off)
    patches = []
    for i in range(3):
        up = [True] * 3
        up[i] = False
        p2 = Picture()
        draw_table(p2, up)
        draw_panel(p2)
        b2, s2, c2 = p2.encode(lamps_off)
        cells = [c for c in range(1000)
                 if b2[(c // 40) * 320 + (c % 40) * 8:(c // 40) * 320 + (c % 40) * 8 + 8] !=
                 bitmap[(c // 40) * 320 + (c % 40) * 8:(c // 40) * 320 + (c % 40) * 8 + 8]
                 or s2[c] != screen[c] or c2[c] != colram[c]]
        patches.append((cells, b2, s2, c2))

    # lamp cells
    lamp_cells = [[] for _ in LAMPS]
    for cy in range(25):
        for cx in range(40):
            ids = set(int(v) for v in pic.lamp[cy * 8:cy * 8 + 8, cx * 4:cx * 4 + 4].flat if v >= 0)
            for i in ids:
                lamp_cells[i].append(cy * 40 + cx)

    if preview:
        img = render_rgb(bitmap, screen, colram)
        Image.fromarray(np.repeat(np.repeat(img, 3, 0), 3, 1)).save(preview)
        # with every lamp on
        c_on = bytearray(colram)
        for i, cells in enumerate(lamp_cells):
            for c in cells:
                c_on[c] = LAMPS[i][2]
        img = render_rgb(bitmap, screen, c_on)
        base, ext = os.path.splitext(preview)
        Image.fromarray(np.repeat(np.repeat(img, 3, 0), 3, 1)).save(base + "_lit" + ext)
        out(f"preview: {preview}")

    # sprites
    shine, body = ball_sprites()
    flips = flipper_sprites()
    plunger = plunger_sprite()

    s = ["; generated by gen_data.py - do not edit\n\n"]
    s.append(asm_bytes("mapa_rle", ca))
    s.append(asm_bytes("mapd_rle", cd))
    s.append(maths_tables())
    s.append(font_tables())
    # lamps
    s.append(asm_bytes("lampoff", [l[1] for l in LAMPS]))
    s.append(asm_bytes("lampon", [l[2] for l in LAMPS]))
    idx = []
    flat = []
    for cells in lamp_cells:
        idx.append(len(flat))
        flat.extend(cells)
    idx.append(len(flat))
    s.append(asm_bytes("lampfirst", idx))
    s.append(asm_bytes("lampcelllo", [c & 255 for c in flat]))
    s.append(asm_bytes("lampcellhi", [c >> 8 for c in flat]))
    # drop target patches: cells, then for each cell 8 bitmap bytes, screen, colour
    for state, name in ((False, "down"), (True, "up")):
        for i, (cells, b2, s2, c2) in enumerate(patches):
            data = bytearray()
            for c in cells:
                src_b, src_s, src_c = (b2, s2, c2) if not state else (bitmap, screen, colram)
                off = (c // 40) * 320 + (c % 40) * 8
                data += bytes(src_b[off:off + 8]) + bytes([src_s[c], src_c[c]])
            s.append(asm_bytes(f"tgt{name}{i}", data))
    for i, (cells, *_r) in enumerate(patches):
        s.append(asm_bytes(f"tgtcells{i}", [len(cells)] +
                           sum([[c & 255, c >> 8] for c in cells], [])))
    # sprite frame positions
    for mirror, name in ((False, "l"), (True, "r")):
        s.append(asm_bytes(f"flip{name}x", [f[2] + 24 for f in flips[mirror]]))
        s.append(asm_bytes(f"flip{name}y", [f[3] + 50 for f in flips[mirror]]))
    # font lookup: ASCII -> glyph index
    fidx = [0] * 128
    for i, ch in enumerate(FONT_CHARS):
        fidx[ord(ch)] = i
    s.append(asm_bytes("fontidx", fidx))
    # S-T-A-R targets: along-face coordinate a = y - x + 40 (pixels * 1.414)
    fxv, fyv = T.BLOCK_BOT[0] - T.BLOCK_TOP[0], T.BLOCK_BOT[1] - T.BLOCK_TOP[1]
    fl = math.hypot(fxv, fyv)
    lo, hi = [], []
    for t in T.STAR_T:
        cx = T.BLOCK_TOP[0] + fxv * t
        cy = T.BLOCK_TOP[1] + fyv * t
        a = cy - cx + 40
        half = (T.STAR_W / 2 + 1.5) * math.sqrt(2)
        lo.append(round(a - half))
        hi.append(round(a + half))
    s.append(asm_bytes("starlo", lo))
    s.append(asm_bytes("starhi", hi))
    s.append(sound.asm())
    with open(os.path.join(HERE, "data.asm"), "w") as f:
        f.write("".join(s))

    # init-only data: bitmap, screen, colour RAM, sprites (copied at start,
    # then overwritten by the collision maps)
    spr = bytearray()
    spr += shine + body + plunger + bytes(64)
    for mirror in (False, True):
        for f_ in flips[mirror]:
            spr += f_[0] + f_[1]
    s = ["; generated by gen_data.py - do not edit\n\n"]
    s.append(asm_bytes("src_bitmap", bitmap))
    s.append(asm_bytes("src_screen", screen))
    s.append(asm_bytes("src_colram", colram))
    s.append(asm_bytes("src_sprites", spr))
    with open(os.path.join(HERE, "init.asm"), "w") as f:
        f.write("".join(s))

    c = ["; generated by gen_data.py - do not edit\n\n"]
    consts = {
        "MAP_X0": MAP_X0, "MAP_W": MAP_W, "MAP_H": MAP_H, "MAP_SIZE": MAP_W * MAP_H,
        "BG": BG, "NLAMPS": len(LAMPS), "NSPRITES": len(spr) // 64,
        "FLIP_L": T.FLIP_L, "FLIP_REST": T.FLIP_REST, "FLIP_UP": T.FLIP_UP,
        "LPIVX": int(T.LFLIP[0]), "RPIVX": int(T.RFLIP[0]), "PIVY": int(T.LFLIP[1]),
        "RPIVXF": round((T.RFLIP[0] % 1) * 256),
        "LANE_CX": T.LANE_CX, "PLUNGER_Y": T.PLUNGER_Y, "LANE_X0": T.LANE_X0,
        "GATE_X": T.GATE_X, "GATE_Y0": T.GATE_Y0, "GATE_Y1": T.GATE_Y1,
        "TARGET_X": T.TARGET_X, "TARGET_Y0": T.TARGET_Y[0], "TARGET_Y1": T.TARGET_Y[-1],
        "SAUCER_X": T.SAUCER[0], "SAUCER_Y": T.SAUCER[1],
        "TIPR16": round((T.BALL_R + T.FLIP_R2) * 16),
        "PANEL_C0": PANEL_C0, "SPR_BYTES": len(spr),
        "SLING_Y0": 122, "SLING_Y1": 165, "LSLING_X0": 40, "RSLING_X0": 197 - 66, "SLING_XW": 26,
    }
    for i, (bx, by, br) in enumerate(T.BUMPERS):
        consts[f"BUMP{i}X"], consts[f"BUMP{i}Y"] = bx, by
    for name, v in [("L_LANE", L_LANE[0]), ("L_BUMP", L_BUMP[0]), ("L_SLING", L_SLING[0]),
                    ("L_STAR", L_STAR[0]), ("L_MULT", L_MULT[0]), ("L_AGAIN", L_AGAIN),
                    ("L_EXTRA", L_EXTRA), ("L_SAVE", L_SAVE)]:
        consts[name] = v
    for k, v in consts.items():
        c.append(f"{k:<12}= {v}\n")
    with open(os.path.join(HERE, "consts.asm"), "w") as f:
        f.write("".join(c))
    out(f"sprites: {len(spr) // 64} blocks, lamps: {len(LAMPS)}")


if __name__ == "__main__":
    main()
