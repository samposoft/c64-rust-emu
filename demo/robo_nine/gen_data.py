#!/usr/bin/env python3
"""Generate data.asm for robo_nine.asm: digit sprites, the ghost digit's
per-line patterns, the robot's multicolour characters and screen map, orbit
and easing tables, and the music (instruments, patterns, order lists).

Tune: "Nine Bolts" - A minor, SPEED 6 (a sixteenth every 6 frames, 125 BPM),
two bars of intro, then a 16-bar loop.
  V1: pulse lead with PWM and delayed vibrato
  V2: filtered saw bass + kick/snare on the same voice
  V3: arpeggiated chords at 50 Hz
"""

import math

OUT = 'data.asm'

# --- digits -----------------------------------------------------------------
# 6x16. The sprites use them in bits 7-2 of their first column; the ghost
# digit (the one that is not a sprite) uses the rows shifted right by two
# (bits 5-0) and inverted as idle bytes: the two pixels of the next idle byte
# that also get the digit's colour then fall on its empty columns 0-1.
DIGITS = {
    1: ["..##..", ".###..", "####..", "..##..", "..##..", "..##..", "..##..", "..##..",
        "..##..", "..##..", "..##..", "..##..", "..##..", "..##..", "######", "######"],
    2: [".####.", "######", "##..##", "....##", "....##", "....##", "...##.", "..###.",
        ".###..", ".##...", "##....", "##....", "##....", "##....", "######", "######"],
    3: [".####.", "######", "##..##", "....##", "....##", "....##", "..###.", "..###.",
        "....##", "....##", "....##", "....##", "##..##", "##..##", "######", ".####."],
    4: ["...##.", "..###.", "..###.", ".####.", ".#.##.", "##.##.", "#..##.", "#..##.",
        "######", "######", "...##.", "...##.", "...##.", "...##.", "...##.", "...##."],
    5: ["######", "######", "##....", "##....", "##....", "#####.", "######", "....##",
        "....##", "....##", "....##", "....##", "##..##", "##..##", "######", ".####."],
    6: ["..###.", ".####.", "##....", "##....", "##....", "##....", "#####.", "######",
        "##..##", "##..##", "##..##", "##..##", "##..##", "##..##", "######", ".####."],
    7: ["######", "######", "....##", "....##", "...##.", "...##.", "...##.", "..##..",
        "..##..", "..##..", ".##...", ".##...", ".##...", ".##...", ".##...", ".##..."],
    8: [".####.", "######", "##..##", "##..##", "##..##", "##..##", ".####.", ".####.",
        "##..##", "##..##", "##..##", "##..##", "##..##", "##..##", "######", ".####."],
    9: [".####.", "######", "##..##", "##..##", "##..##", "##..##", "##..##", "######",
        ".#####", "....##", "....##", "....##", "....##", "...###", ".####.", ".###.."],
}

GHOST = 7               # the digit drawn with the idle byte instead of a sprite


def glyph_bytes(d):
    rows = DIGITS[d]
    assert len(rows) == 16 and all(len(r) == 6 for r in rows), d
    return [sum(0x80 >> i for i, c in enumerate(r) if c == '#') for r in rows]


# --- robot ------------------------------------------------------------------
# Multicolour: 24 fat pixels x 80 lines = 6x10 characters.
# '.' = 00 background, 'a' = 01 ($D022 light grey), 'b' = 10 ($D023 grey),
# 'c' = 11 (the cell's colour RAM, black unless CELLCOL says otherwise).
ROBOT = [
    "...........cc...........",   # 0  antenna bulb
    "..........cccc..........",
    "..........cccc..........",
    "...........cc...........",
    "...........ab...........",
    "...........ab...........",
    "........aaaabbbb........",
    "........bbbbbbbb........",
    "...aaaaaaaaaaaaaaaaaa...",   # 8  head
    "...abbbbbbbbbbbbbbbbbc..",
    "...abbbbbbbbbbbbbbbbbc..",
    "...abbbbbbbbbbbbbbbbbc..",
    ".aaabbbbbbbbbbbbbbbbbccc",
    ".abbbbbbbbbbbbbbbbbbbbbc",
    ".abbbbbbbbbbbbbbbbbbbbbc",
    ".abbbbbbbbbbbbbbbbbbbbbc",
    ".abbbccbbbbbbbbbbbccbbbc",   # 16 eyes (cells 1 and 4)
    ".abbccccbbbbbbbbbccccbbc",
    ".abbccccbbbbbbbbbccccbbc",
    ".abbccccbbbbbbbbbccccbbc",
    ".abbccccbbbbbbbbbccccbbc",
    ".abbbccbbbbbbbbbbbccbbbc",
    ".abbbbbbbbbbbbbbbbbbbbbc",
    ".cccbbbbbbbbbbbbbbbbbccc",
    "...abbbbbbbbbbbbbbbbbc..",   # 24 mouth
    "...abbbccccccccccbbbbc..",
    "...abbbcbcbcbcbcbbbbbc..",
    "...abbbcbcbcbcbcbbbbbc..",
    "...abbbccccccccccbbbbc..",
    "...abbbbbbbbbbbbbbbbbc..",
    "...cccccccccccccccccccc.",
    ".........abbbc..........",
    ".........abbbc..........",   # 32 neck and shoulders
    ".........abbbc..........",
    "..aaaaaaaaaaaaaaaaaaaa..",
    ".aabbbbbbbbbbbbbbbbbbbc.",
    "aabbbbbbbbbbbbbbbbbbbbbc",
    "abbbabbbbbbbbbbbbbbbbbbc",
    "abbbabbbbbbbbbbbbbbbcbbc",
    "abbbabbbbbbbbbbbbbbbcbbc",
    "abbbabbbcccccccccbbbcbbc",   # 40 chest hatch (cells 2 and 3)
    "abbbabbbcccccccccbbbcbbc",
    "abbbabbbcccccccccbbbcbbc",
    "abbbabbbcccccccccbbbcbbc",
    "abbbabbbcccccccccbbbcbbc",
    "abbbabbbcccccccccbbbcbbc",
    "abbbabbbcccccccccbbbcbbc",
    "abbbabbbbbbbbbbbbbbbcbbc",
    "abbbabbbbbbbbbbbbbbbcbbc",   # 48 lights (cells 1-4)
    "abbbaccbbccbbccbbccbcbbc",
    "abbbaccbbccbbccbbccbcbbc",
    "abbbabbbbbbbbbbbbbbbcbbc",
    "abbbabbbbbbbbbbbbbbbcbbc",
    "abbbaaaaaaaaaaaaaaaacbbc",
    "abbbabbbbbbbbbbbbbbbcbbc",
    "abbb.abbbbbbbbbbbbbb.bbc",
    "abbb.abbbbbbbbbbbbbc.bbc",   # 56 hands
    "cbbc.abbbbbbbbbbbbbc.cbc",
    "c..c.accccccccccccc..c.c",
    "cc.c..abbc....abbc...ccc",
    ".cc...abbc....abbc......",
    "......abbc....abbc......",
    "......abbc....abbc......",
    "......abbc....abbc......",
    "......abbc....abbc......",   # 64 legs
    "......abbc....abbc......",
    "......abbc....abbc......",
    "......aabc....aabc......",
    "......abbc....abbc......",
    "......abbc....abbc......",
    "......abbc....abbc......",
    "......abbc....abbc......",
    "....aaaabc....aaaabb....",   # 72 feet
    "...abbbbbc....abbbbbb...",
    "...abbbbbc....abbbbbbc..",
    "...abbbbbc....abbbbbbc..",
    "...cccccccc..cccccccccc.",
    "........................",
    "........................",
    "........................",
]
assert len(ROBOT) == 80 and all(len(r) == 24 for r in ROBOT)

# colour of the '11' pixels of some cells (col, row) -> colour 0-7
CELLCOL = {(2, 0): 2, (3, 0): 2,                        # antenna bulb: red
           (1, 2): 3, (4, 2): 3,                        # eyes: cyan
           (2, 5): 0, (3, 5): 0,                        # hatch (black when closed)
           (1, 6): 2, (2, 6): 7, (3, 6): 5, (4, 6): 3}  # chest lights

ROBOT_COL, ROBOT_ROW = 17, 9                            # top left on the screen
CHAR0 = 64                                              # first robot character

# eye cells: 4 fat pixels wide, lines 16-23 of the robot. 'a' is the pupil.
EYES = {
    'centre': [".cc.", "cccc", "cacc", "cacc", "cccc", ".cc.", "....", "...."],
    'left':   [".cc.", "cccc", "accc", "accc", "cccc", ".cc.", "....", "...."],
    'right':  [".cc.", "cccc", "ccca", "ccca", "cccc", ".cc.", "....", "...."],
    'closed': ["....", "....", "....", "cccc", "cccc", "....", "....", "...."],
}


def cell_bytes(rows):
    val = {'.': 0, 'a': 1, 'b': 2, 'c': 3}
    return [sum(val[ch] << (6 - 2 * i) for i, ch in enumerate(r)) for r in rows]


def robot_cells():
    """Cell (col, row) -> 8 bytes, with the eyes looking straight ahead."""
    cells = {}
    for row in range(10):
        for col in range(6):
            lines = [ROBOT[row * 8 + y][col * 4:col * 4 + 4] for y in range(8)]
            cells[(col, row)] = cell_bytes(lines)
    return cells


def eye_cell(style, col):
    # the eye art replaces the 'c' pixels of the cell; the frame stays grey
    base = [ROBOT[16 + y][col * 4:col * 4 + 4] for y in range(8)]
    out = []
    for y in range(8):
        e = EYES[style][y]
        out.append(''.join(e[i] if e[i] != '.' else ('b' if base[y][i] in 'bc' else base[y][i])
                           for i in range(4)))
    return cell_bytes(out)


# --- tables -----------------------------------------------------------------
CX, CY = 181, 155          # orbit centre (sprite coordinates of the digit)
RX, RY = 112, 70
CHEST = (181, 150)


def orbit():
    xs, ys = [], []
    for a in range(256):
        t = 2 * math.pi * a / 256
        xs.append(round(CX + RX * math.cos(t)))
        ys.append(round(CY + RY * math.sin(t)))
    return xs, ys


def ease():
    # 65 steps of smoothstep, 0..255
    return [min(255, round(255 * (3 * (i / 64) ** 2 - 2 * (i / 64) ** 3))) for i in range(65)]


def hop():
    # a 32-frame hop for the wave in the finale: 0 on the ground, 12 at the top
    return [round(12 * math.sin(math.pi * i / 32)) if i < 32 else 0 for i in range(64)]


# --- music ------------------------------------------------------------------
SPEED = 6

SEMI = {'C': 0, 'C#': 1, 'Db': 1, 'D': 2, 'D#': 3, 'Eb': 3, 'E': 4, 'F': 5,
        'F#': 6, 'Gb': 6, 'G': 7, 'G#': 8, 'Ab': 8, 'A': 9, 'A#': 10, 'Bb': 10, 'B': 11}


def nn(s):
    """'D5' -> MIDI note (D4 = 62)."""
    return 12 * (int(s[-1]) + 1) + SEMI[s[:-1]]


# wt: list of (waveform, note) per frame; an int note is relative to the
# played note, ('a', n) is absolute. HOLD stops the table, LOOP restarts it.
HOLD, LOOP = 'hold', 'loop'
INSTR = {
    'lead':   dict(ad=0x08, sr=0x9A, wt=[(0x41, 12), (0x41, 0), HOLD],
                   pw=0x06, pws=0x14, vdel=14, vsh=3),
    'bass':   dict(ad=0x05, sr=0xA9, wt=[(0x41, 12), (0x21, 0), HOLD],
                   pw=0x04, flt=1, fst=0xA0, fdec=0x08),
    'kick':   dict(ad=0x00, sr=0xF6, wt=[(0x81, ('a', 60)), (0x41, ('a', 45)), (0x41, ('a', 38)),
                                         (0x41, ('a', 33)), (0x41, ('a', 30)), (0x40, ('a', 28)), HOLD],
                   pw=0x08, beat=1),
    'snare':  dict(ad=0x00, sr=0xF7, wt=[(0x81, ('a', 84)), (0x41, ('a', 56)), (0x81, ('a', 80)),
                                         (0x81, ('a', 78)), (0x81, ('a', 76)), (0x80, ('a', 74)), HOLD],
                   pw=0x08, beat=2),
    'arpmin': dict(ad=0x06, sr=0x6A, wt=[(0x41, 0), (0x41, 3), (0x41, 7), (0x41, 12), LOOP],
                   pw=0x04, pws=0x0C),
    'arpmaj': dict(ad=0x06, sr=0x6A, wt=[(0x41, 0), (0x41, 4), (0x41, 7), (0x41, 12), LOOP],
                   pw=0x04, pws=0x0C),
}
IDX = {name: i + 1 for i, name in enumerate(INSTR)}

BASS = {'Am': 33, 'F': 29, 'C': 36, 'G': 31, 'Dm': 38, 'E': 28, 'Em': 28}
ARP = {'Am': (57, 'arpmin'), 'F': (53, 'arpmaj'), 'C': (60, 'arpmaj'), 'G': (55, 'arpmaj'),
       'Dm': (62, 'arpmin'), 'E': (52, 'arpmaj'), 'Em': (52, 'arpmin')}

INTRO = ['Am', 'E']
SEC_A = ['Am', 'F', 'C', 'G', 'Am', 'F', 'Dm', 'E']
SEC_B = ['F', 'G', 'Em', 'Am', 'F', 'G', 'E', 'E']

PAD = 'x.......x.......'
SWING = 'x..x..x.x..x..x.'
DRIVE = 'x.x.x.x.x.x.x.x.'


def phrase(*items):
    """'A4:2' = a note of 2 sixteenths, 'r:4' = rest (gate off)."""
    rows = []
    for it in items:
        name, d = it.split(':')
        rows.append('off' if name == 'r' else ('lead', nn(name)))
        rows += [None] * (int(d) - 1)
    return rows


def bassbar(ch, kind='full'):
    lo, hi = ('bass', BASS[ch]), ('bass', BASS[ch] + 12)
    K, S = ('kick', 60), ('snare', 60)
    rows = [K, None, lo, hi, S, None, lo, hi, K, None, lo, hi, S, None, lo, hi]
    if kind == 'intro':
        rows = [lo, None, None, None, None, None, None, None, hi, None, None, None, lo, None, None, None]
    elif kind == 'fill':
        rows[12:16] = [S, S, S, S]
    return rows


def arpbar(ch, rhythm):
    note, ins = ARP[ch]
    return [(ins, note) if c == 'x' else None for c in rhythm]


lead = phrase('r:16', 'r:8', 'E5:2', 'G#5:2', 'B5:4')
lead += phrase(                                           # section A
    'A5:2', 'C6:2', 'E6:4', 'A5:6', 'G5:2',
    'F5:4', 'E5:2', 'D5:2', 'C5:4', 'A4:4',
    'G4:2', 'C5:2', 'E5:4', 'G5:6', 'F5:2',
    'E5:4', 'D5:2', 'C5:2', 'D5:8',
    'A4:2', 'C5:2', 'E5:4', 'A5:6', 'B5:2',
    'C6:4', 'B5:2', 'A5:2', 'F5:4', 'A5:4',
    'D5:2', 'F5:2', 'A5:4', 'G5:4', 'F5:4',
    'E5:4', 'G#5:4', 'B5:4', 'E6:4')
lead += phrase(                                           # section B
    'C6:6', 'A5:2', 'F5:4', 'A5:4',
    'B5:6', 'G5:2', 'D5:4', 'G5:4',
    'E5:4', 'G5:2', 'B5:2', 'E6:6', 'D6:2',
    'C6:4', 'B5:2', 'A5:2', 'E5:8',
    'F5:2', 'A5:2', 'C6:4', 'A5:2', 'F5:2', 'C5:4',
    'D5:2', 'G5:2', 'B5:4', 'G5:2', 'D5:2', 'B4:4',
    'G#4:2', 'B4:2', 'E5:4', 'G#5:4', 'B5:4',
    'E6:8', 'D6:2', 'B5:2', 'G#5:2', 'E5:2')

bass = []
for ch in INTRO:
    bass += bassbar(ch, 'intro')
for i, ch in enumerate(SEC_A):
    bass += bassbar(ch, 'fill' if i == 7 else 'full')
for i, ch in enumerate(SEC_B):
    bass += bassbar(ch, 'fill' if i in (3, 7) else 'full')

arps = []
arps += sum((arpbar(ch, PAD) for ch in INTRO), [])
arps += sum((arpbar(ch, SWING) for ch in SEC_A), [])
arps += sum((arpbar(ch, DRIVE) for ch in SEC_B), [])

voices = [lead, bass, arps]
assert all(len(v) == len(lead) and len(v) % 16 == 0 for v in voices), [len(v) for v in voices]
LOOP_BAR = len(INTRO)


def encode(bar):
    out = []
    for ev in bar:
        if ev is None:
            out.append(0)
        elif ev == 'off':
            out.append(0x60)
        else:
            ins, note = ev
            assert 1 <= note <= 95
            out += [0xE0 | IDX[ins], note]
    return tuple(out)


def music_lines():
    pats, orders = {}, []
    for v in voices:
        order = []
        for b in range(0, len(v), 16):
            p = encode(v[b:b + 16])
            order.append(pats.setdefault(p, len(pats)))
        orders.append(order + [0xFF, LOOP_BAR])

    wtw, wtn, wstart = [], [], {}
    for name, ins in INSTR.items():
        start = wstart[name] = len(wtw)
        for e in ins['wt']:
            if e == HOLD:
                wtw.append(0xFE); wtn.append(0)
            elif e == LOOP:
                wtw.append(0xFF); wtn.append(start)
            else:
                w, v = e
                wtw.append(w)
                wtn.append(0x80 | v[1] if isinstance(v, tuple) else v)

    def field(key):
        return [0] + [ins.get(key, 0) for ins in INSTR.values()]

    L = ['; --- music ---', '', 'SPEED   = %d' % SPEED, '']
    for label, key in [('i_ad', 'ad'), ('i_sr', 'sr'), ('i_pw', 'pw'), ('i_pws', 'pws'),
                       ('i_vdel', 'vdel'), ('i_vsh', 'vsh'), ('i_flt', 'flt'), ('i_fst', 'fst'),
                       ('i_fdec', 'fdec'), ('i_beat', 'beat')]:
        L.append('%-7s .byte %s' % (label, hx(field(key))))
    L.append('%-7s .byte %s' % ('i_wt', hx([0] + [wstart[n] for n in INSTR])))
    L += ['', 'wtw     .byte ' + hx(wtw), 'wtn     .byte ' + hx(wtn), '']
    L.append('patlo   .byte ' + ', '.join('<pat%d' % i for i in range(len(pats))))
    L.append('pathi   .byte ' + ', '.join('>pat%d' % i for i in range(len(pats))))
    for p, i in pats.items():
        L.append('pat%-4d .byte %s' % (i, hx(p)))
    L.append('')
    for i, o in enumerate(orders):
        L.append('ord%d    .byte %s' % (i, hx(o)))
    L.append('ordlo   .byte <ord0, <ord1, <ord2')
    L.append('ordhi   .byte >ord0, >ord1, >ord2')
    bars = len(lead) // 16
    print('music: %d bars, %d patterns, %.1f s' % (bars, len(pats), bars * 16 * SPEED / 50))
    return L


# --- output -----------------------------------------------------------------
def hx(vals):
    return ', '.join('$%02x' % v for v in vals)


def rows_of(label, vals, per=16):
    out = []
    for i in range(0, len(vals), per):
        out.append('%-7s .byte %s' % (label if i == 0 else '', hx(vals[i:i + per])))
    return out


def main():
    L = ['; generated by gen_data.py - do not edit by hand', '']

    # digit sprites: block 0 empty, blocks 1-9 = digits
    L.append('; sprite blocks: empty, then digits 1-9 (7x16 in the first column)')
    L.append('sprites')
    L += rows_of('', [0] * 64)
    for d in range(1, 10):
        g = glyph_bytes(d)
        blk = []
        for r in range(21):
            blk += [g[r] if r < 16 else 0, 0, 0]
        blk.append(0)
        L += rows_of('', blk)
    L.append('')

    # ghost patterns: idle byte per line, 1 = black pixel
    g = glyph_bytes(GHOST)
    L.append('GHOST   = %d' % GHOST)
    L.append('; idle bytes of the ghost digit, one per raster line (inverted glyph)')
    L.append('GPAT    = (%s)' % hx([0xFF ^ (b >> 2) for b in g] + [0xFF]))
    L.append('')

    # robot charset and screen map
    cells = robot_cells()
    chars, scr, col = [], [], []
    for row in range(10):
        for c in range(6):
            chars.append(cells[(c, row)])
            scr.append(CHAR0 + len(chars) - 1)
            col.append(8 | CELLCOL.get((c, row), 0))
    eye_chars = {}
    for style in ('centre', 'left', 'right', 'closed'):
        for c in (1, 4):
            eye_chars[(style, c)] = CHAR0 + len(chars)
            chars.append(eye_cell(style, c))
    # the robot's own eye cells start looking ahead
    scr[2 * 6 + 1] = eye_chars[('centre', 1)]
    scr[2 * 6 + 4] = eye_chars[('centre', 4)]
    assert CHAR0 + len(chars) <= 256
    L.append('ROBOT_COL = %d' % ROBOT_COL)
    L.append('ROBOT_ROW = %d' % ROBOT_ROW)
    L.append('ROBOT_CHARS = %d' % len(chars))
    L.append('CHAR0   = %d' % CHAR0)
    L.append('; eye characters: centre, left, right, closed (left eye, right eye)')
    L.append('eyel    .byte ' + hx([eye_chars[(s, 1)] for s in ('centre', 'left', 'right', 'closed')]))
    L.append('eyer    .byte ' + hx([eye_chars[(s, 4)] for s in ('centre', 'left', 'right', 'closed')]))
    L.append('robchars')
    L += rows_of('', sum(chars, []))
    L += rows_of('robscr', scr, 6)
    L += rows_of('robcol', col, 6)
    L.append('')

    # orbit, easing, hop
    xs, ys = orbit()
    L.append('; orbit: centre (%d,%d), radii %d x %d' % (CX, CY, RX, RY))
    L += rows_of('orbxl', [x & 0xFF for x in xs])
    L += rows_of('orbxh', [x >> 8 for x in xs])
    L += rows_of('orby', ys)
    L.append('CHESTX  = %d' % CHEST[0])
    L.append('CHESTY  = %d' % CHEST[1])
    L += rows_of('ease', ease())
    L += rows_of('hop', hop())
    L.append('')

    L += music_lines()

    with open(OUT, 'w') as f:
        f.write('\n'.join(L) + '\n')


if __name__ == '__main__':
    main()
