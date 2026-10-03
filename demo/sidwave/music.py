#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-2.0-or-later
# Copyright (C) 2026 SampoSoft - Francesco Sampoli
"""Writes music.asm for player.asm: the tune "Neon Horizon".

A minor, 125 BPM (6 frames per sixteenth), 76 bars (about 2'26"), then it
starts again:

  bars  1-8   intro: a pad through a resonant low-pass that opens slowly,
              then kick on the beats and bass on the off-beats
  bars  9-24  verse: pulse lead, plucked bass with its own filter envelope,
              gated chords with the hi-hats and the snare inside them
  bars 25-40  chorus: sawtooth lead, rolling sixteenth bass
  bars 41-48  break: no drums, a slow filter sweep on the bass, the lead
              echoed by the third voice three sixteenths later
  bars 49-52  build: four-on-the-floor kick, snare roll, a whole tone up
  bars 53-68  chorus in B minor
  bars 69-76  outro: the pad closes again and the volume fades

Voice 1 plays bass and kick, voice 2 the lead (and the snare fills), voice
3 the chords: the drums share the voices with the melodic parts, as in the
1980s, through the wave table (an instrument can start with a drum and
continue as a chord or a bass note).

Formats (indices into the tables start at 1, 0 means "none"):

  wave table   L = waveform register ($00: keep the previous one),
               R = note: $00-$5f up, $60-$7f down 32..1 semitones from the
               played note, $80+n absolute note n.
               L = $ff: jump to R ($00: stop).
  pulse table  L = $8h, R = ll: pulse width $hll.
               L = 1-127: L frames adding R (signed) per frame.
               L = $ff: jump to R ($00: stop).
  filter table L = $80 + mode ($10 LP, $20 BP, $40 HP), R = $d417
               (resonance, voices). L = $00: cutoff R.
               L = 1-127: L frames adding R/16 (signed) per frame.
               L = $ff: jump to R ($00: stop).
  patterns     $00-$5f note, $60 rest (gate off), $61 tie, then prefixes:
               $80-$bf instrument, $c0-$df duration 1-32 rows (sticky),
               $e0 legato, $e1 ss portamento, $e2 pp filter program,
               $e3 ss fade out (0 = full volume), $e4 vv sync for the demo.
               $ff end of the pattern.
  order lists  $00-$7f pattern, $80-$bf transposition -32..+31,
               $ff nn go to position nn, $fe end.
"""

import os

SPEEDS = (6, 6)
CLOCK = 985248                     # PAL

SEMI = {'C': 0, 'C#': 1, 'Db': 1, 'D': 2, 'D#': 3, 'Eb': 3, 'E': 4, 'F': 5,
        'F#': 6, 'Gb': 6, 'G': 7, 'G#': 8, 'Ab': 8, 'A': 9, 'A#': 10, 'Bb': 10, 'B': 11}


def nn(s):
    """'A4' -> note index (C0 = 0, A4 = 57)."""
    return 12 * int(s[-1]) + SEMI[s[:-1]]


def freq_table():
    regs = []
    for n in range(97):
        hz = 440.0 * 2 ** ((n - 57) / 12)
        regs.append(min(0xFFFF, round(hz * (1 << 24) / CLOCK)))
    return regs


# --- tables ----------------------------------------------------------------
class Table:
    """A two-column program table; entry 0 is unused (0 = none)."""

    def __init__(self, name):
        self.name = name
        self.rows = [(0, 0)]
        self.named = {}

    def add(self, entries, name=None):
        start = len(self.rows)
        for l, r in entries:
            if l == 'loop':                    # jump back to entry r of this program
                l, r = 0xFF, start + r
            self.rows.append((l, r & 0xFF))
        assert len(self.rows) <= 256, self.name
        if name:
            self.named[name] = start
        return start


def rel(k):
    assert -32 <= k < 0x60
    return k & 0x7F if k >= 0 else k + 0x80


def ab(note):
    return 0x80 | note


STOP = (0xFF, 0)
wt, pt, ft = Table('wave'), Table('pulse'), Table('filter')

# Chord shapes for the arpeggios (semitones above the played note).
SHAPES = {
    'm': (0, 3, 7), 'M': (0, 4, 7), 'M2': (0, 5, 9), 'M1': (0, 3, 8),
    'M7': (0, 4, 7, 11), 'M8': (0, 4, 7, 12), 'm7': (0, 3, 7, 10),
    'm9': (0, 3, 7, 14), 'm1': (0, 4, 9), 'm2': (0, 5, 8),
}
for shape, offs in SHAPES.items():
    wt.add([(0x41, rel(o)) for o in offs] + [('loop', 0)], 'chord_' + shape)

# Drum heads (one entry per frame), from the recipes of the trackers:
# snare = noise, then a low pulse body, then noise again; hi-hat = one
# frame of high noise; kick = a click, then a pulse falling fast in pitch.
SNARE = [(0x81, 0xD0), (0x41, 0xA6), (0x41, 0xA1), (0x81, 0xCA), (0x81, 0xC6), (0x81, 0xC3)]
HAT = [(0x81, ab(94))]
KICK = [(0x81, 0xC8), (0x41, 0xA4), (0x41, 0x9C), (0x41, 0x97), (0x11, 0x93)]
HEADS = {'plain': [(0x09, rel(0))], 'hat': HAT, 'snare': SNARE}

# Gated chords: a drum head, then a jump into the shape's loop.
for head, frames in HEADS.items():
    for shape in SHAPES:
        wt.add(frames + [(0xFF, wt.named['chord_' + shape])], f'{head}_{shape}')

wt.add(KICK + [(0x10, 0x90), STOP], 'kick')
wt.add(SNARE[:5] + [(0x80, 0xC0), STOP], 'snare')
wt.add(KICK + [(0x21, rel(0)), STOP], 'kickbass')
wt.add([(0x21, rel(0)), STOP], 'saw')
wt.add([(0x41, rel(0)), STOP], 'pulse')
wt.add([(0x11, rel(0)), STOP], 'tri')
wt.add([(0x41, rel(12)), (0x41, rel(0)), STOP], 'lead')     # an octave blip on the first frame
wt.add([(0x21, rel(12)), (0x21, rel(0)), STOP], 'sawlead')

# Pulse programs.
pt.add([(0x82, 0x00), (96, 16), (96, -16), ('loop', 1)], 'arp')      # $200-$800 and back
pt.add([(0x83, 0x00), (100, 8), (100, -8), ('loop', 1)], 'lead')
pt.add([(0x88, 0x00), STOP], 'square')
pt.add([(0x84, 0x00), (127, 4), (127, -4), ('loop', 1)], 'bass')

# Filter programs. On the 6581 the useful cutoff range is roughly $10-$80.
LP, BP = 0x10, 0x20
V1, V3 = 1, 4
ft.add([(0x80 | LP, 0xE0 | V3), (0x00, 0x06)] + [(127, 2)] * 6 + [STOP], 'intro')
ft.add([(0x80 | LP, 0xB0 | V1), (0x00, 0x58), (6, -40), (10, -12), STOP], 'pluck')
ft.add([(0x80 | LP, 0xB0 | V1), (0x00, 0x68), (6, -40), (12, -12), STOP], 'pluck2')
ft.add([(0x80 | LP | BP, 0xD0 | V1), (0x00, 0x12)] + [(127, 2)] * 3 + [(127, -2)] * 3 + [STOP], 'sweep')
ft.add([(0x80 | LP, 0xE0 | V3), (0x00, 0x64)] + [(127, -2)] * 6 + [STOP], 'outro')
ft.add([(0x80 | LP, 0x00), STOP], 'off')

# --- instruments -----------------------------------------------------------
# vis: what the demo shows for the instrument: 1 kick, 2 snare, 3 hi-hat,
# 4 bass, 5 lead, 6 chord.
INSTR = {}


def instr(name, ad, sr, wave, pulse=None, filt=None, vdel=0, vibs=0, vibd=0,
          gate=0, hr=True, vis=0):
    INSTR[name] = dict(ad=ad, sr=sr, wave=wt.named[wave],
                       pulse=pt.named[pulse] if pulse else 0,
                       filt=ft.named[filt] if filt else 0,
                       vdel=vdel, vibs=vibs, vibd=vibd, gate=gate,
                       flags=0 if hr else 0x80, vis=vis)


VIS_HEAD = {'plain': 6, 'hat': 3, 'snare': 2}
for head in HEADS:
    for shape in SHAPES:
        instr(f'{head}_{shape}', 0x04, 0x96, f'{head}_{shape}', vis=VIS_HEAD[head])
for shape in SHAPES:
    instr(f'pad_{shape}', 0x6A, 0xAB, f'chord_{shape}', pulse='arp', hr=False, vis=6)
instr('kick', 0x08, 0x05, 'kick', pulse='square', vis=1)
instr('snare', 0x08, 0x07, 'snare', pulse='square', vis=2)
instr('kickbass', 0x06, 0xB8, 'kickbass', pulse='square', filt='pluck', vis=1)
instr('bass', 0x06, 0xB8, 'saw', filt='pluck', vis=4)
instr('kickbass2', 0x06, 0xB8, 'kickbass', pulse='square', filt='pluck2', vis=1)
instr('bass2', 0x05, 0xA8, 'saw', filt='pluck2', vis=4)
instr('obass', 0x05, 0x98, 'pulse', pulse='bass', vis=4)
instr('longbass', 0x0A, 0xED, 'saw', vis=4)
instr('lead', 0x08, 0xC9, 'lead', pulse='lead', vdel=16, vibs=4, vibd=2, vis=5)
instr('sawlead', 0x08, 0xB9, 'sawlead', vdel=16, vibs=4, vibd=2, vis=5)
instr('soft', 0x3A, 0xBA, 'tri', vdel=10, vibs=5, vibd=1, vis=5)
instr('echo', 0x3A, 0x6A, 'tri', vdel=10, vibs=5, vibd=1, vis=0)
NAMES = []                         # the instruments the tune uses (set by build())

# --- writing the tune ------------------------------------------------------
BAR = 16


class Cell:
    """An event on a row: a note (instrument, note, legato/portamento), a rest
    or a tie, with global commands."""

    def __init__(self, kind, inst=None, note=None, legato=False, porta=0, cmds=()):
        self.kind, self.inst, self.note = kind, inst, note
        self.legato, self.porta, self.cmds = legato, porta, list(cmds)


def N(inst, note, **kw):
    return Cell('note', inst, nn(note) if isinstance(note, str) else note, **kw)


def R(**kw):
    return Cell('rest', **kw)


def track(bars):
    return [None] * (bars * BAR)


def melody(rows, start, text, inst):
    """'E5:4 A4:2 r:2 ~C5:2 ^8D5:4': note:sixteenths, r = rest,
    ~ = legato, ^ss = portamento with speed ss."""
    pos = start
    for tok in text.split():
        name, d = tok.split(':')
        legato, porta = False, 0
        if name.startswith('~'):
            legato, name = True, name[1:]
        elif name.startswith('^'):
            i = 1
            while name[i].isdigit():
                i += 1
            porta, name = int(name[1:i]), name[i:]
        rows[pos] = R() if name == 'r' else N(inst, name, legato=legato, porta=porta)
        pos += int(d)
    assert (pos - start) % BAR == 0, (text, pos - start)
    return pos - start


def cmd(rows, pos, *cmds):
    """Global commands on a row (a tie is added if the row has no event)."""
    if rows[pos] is None:
        rows[pos] = Cell('tie')
    rows[pos].cmds += cmds


# Chords: arpeggio note and shape for the verse, the chorus and the pads;
# bass root.
ARP = {'Am': ('A3', 'm'), 'F': ('F3', 'M'), 'C': ('G3', 'M2'), 'G': ('G3', 'M'),
       'E': ('G#3', 'M1'), 'Em': ('G3', 'm1'), 'Dm': ('A3', 'm2')}
ARP2 = {'F': ('F3', 'M7'), 'G': ('G3', 'M8'), 'Em': ('E3', 'm7'), 'Am': ('A3', 'm9'),
        'E': ('G#3', 'M1'), 'C': ('G3', 'M2'), 'Dm': ('A3', 'm2')}
ROOT = {'Am': 'A1', 'F': 'F1', 'C': 'C2', 'G': 'G1', 'E': 'E1', 'Em': 'E1', 'Dm': 'D2'}


def pads(rows, start, chords):
    for i, ch in enumerate(chords):
        note, shape = ARP[ch]
        rows[start + i * BAR] = N(f'pad_{shape}', note)


def gated(rows, start, chords, table, fill_every=4, snare=True):
    """Chords on the eighths, with hi-hats on the off-beats and the snare
    on 2 and 4; a sixteenth snare fill at the end of every few bars."""
    for i, ch in enumerate(chords):
        note, shape = table[ch]
        b = start + i * BAR
        heads = ['plain', 'hat', 'snare' if snare else 'plain', 'hat'] * 2
        for k, head in enumerate(heads):
            rows[b + 2 * k] = N(f'{head}_{shape}', note)
        if snare and fill_every and i % fill_every == fill_every - 1:
            for r in (13, 14, 15):
                rows[b + r] = N(f'snare_{shape}', note)


def bass8(rows, start, chords, kb='kickbass', b='bass'):
    """Kick + bass on 1 and 3, bass on the eighths with octave jumps."""
    for i, ch in enumerate(chords):
        r = nn(ROOT[ch])
        s = start + i * BAR
        for k, (ins, n) in enumerate([(kb, r), (b, r), (b, r + 12), (b, r),
                                      (kb, r), (b, r), (b, r + 12), (b, r + 12)]):
            rows[s + 2 * k] = N(ins, n)


def bass16(rows, start, chords):
    """Rolling sixteenths: kick + bass on 1 and 3, octave on the fourth one."""
    for i, ch in enumerate(chords):
        r = nn(ROOT[ch])
        s = start + i * BAR
        for k in range(16):
            beat = k % 4
            ins = 'kickbass2' if k in (0, 8) else 'bass2'
            rows[s + k] = N(ins, r + (12 if beat == 3 else 0))


def offbeat(rows, start, chords, kick=True):
    """Four on the floor and bass on the off-beats."""
    for i, ch in enumerate(chords):
        r = nn(ROOT[ch]) + 12
        s = start + i * BAR
        for beat in range(4):
            rows[s + 4 * beat] = N('kick', 'C3') if kick else R()
            rows[s + 4 * beat + 2] = N('obass', r)


# --- the sections ----------------------------------------------------------
VERSE = ['Am', 'F', 'C', 'G'] * 3 + ['Am', 'F', 'C', 'E']
VERSE_LEAD = """
E5:4 A4:2 C5:2 E5:2 D5:4 C5:2      C5:6 A4:2 F4:4 A4:2 C5:2
G5:4 E5:2 G5:2 E5:2 D5:2 C5:2 E5:2 D5:10 B4:2 G4:2 B4:2
E5:4 A4:2 C5:2 E5:2 D5:4 C5:2      A5:6 G5:2 F5:4 E5:2 C5:2
E5:4 G5:4 C6:4 B5:2 G5:2           A5:8 G5:4 D5:4
E5:4 A4:2 C5:2 E5:2 D5:4 C5:2      C5:6 A4:2 F4:4 A4:2 C5:2
G5:4 E5:2 G5:2 C6:2 B5:2 G5:2 E5:2 D5:8 E5:2 F5:2 G5:4
A5:6 G5:2 E5:4 C5:4                F5:4 E5:2 C5:2 A4:4 C5:4
E5:6 D5:2 C5:4 E5:4                B4:4 E5:4 G#5:4 B5:4
"""

CHORUS = ['F', 'G', 'Em', 'Am', 'F', 'G', 'Am', 'E', 'F', 'G', 'Em', 'Am', 'F', 'G', 'E', 'Am']
CHORUS_LEAD = """
C6:3 A5:3 F5:2 G5:2 A5:2 C6:4      D6:3 B5:3 G5:2 A5:2 B5:2 D6:4
E6:3 B5:3 G5:2 E6:2 D6:2 B5:4      C6:6 B5:2 A5:8
C6:3 A5:3 F5:2 G5:2 A5:2 C6:4      D6:3 B5:3 G5:2 D6:2 E6:2 F6:4
E6:6 D6:2 C6:4 A5:4                B5:8 G#5:4 B5:4
C6:3 A5:3 F5:2 G5:2 A5:2 C6:4      D6:3 B5:3 G5:2 A5:2 B5:2 D6:4
E6:3 B5:3 G5:2 E6:2 D6:2 B5:4      C6:6 B5:2 A5:8
C6:3 A5:3 F5:2 G5:2 A5:2 C6:4      D6:3 B5:3 G5:2 D6:2 E6:2 F6:4
E6:6 D6:2 B5:4 G#5:4               A5:12 r:4
"""

BREAK = ['Dm', 'F', 'Am', 'G', 'Dm', 'F', 'E', 'E']
BREAK_LEAD = """
A5:8 F5:4 D5:4      C6:8 A5:4 F5:4      E5:8 C5:4 A4:4      B4:8 D5:4 G5:4
A5:8 F5:4 D5:4      C6:6 D6:2 C6:4 A5:4 B5:8 G#5:4 E5:4     E5:12 r:4
"""
BREAK_BASS = ['D2', 'F1', 'A1', 'G1', 'D2', 'F1', 'E1', 'E1']

BUILD = ['Dm', 'F', 'G', 'E']
BUILD_LEAD = "A4:16 C5:16 D5:16 E5:8 ^6G#5:8"

OUTRO = ['F', 'G', 'Am', 'Am'] * 2
OUTRO_LEAD = "C6:3 A5:3 F5:2 G5:2 A5:2 C6:4 D6:3 B5:3 G5:2 A5:2 B5:2 D6:4 C6:16 r:16"

# Sync values for the demo, one per section.
SYNC = {'intro': 1, 'verse': 2, 'chorus': 3, 'break': 4, 'build': 5, 'chorus2': 6, 'outro': 7}


def section_intro():
    v1, v2, v3 = track(8), track(8), track(8)
    pads(v3, 0, ['Am', 'F', 'C', 'G'])
    gated(v3, 4 * BAR, ['Am', 'F', 'C', 'G'], ARP, snare=False)
    cmd(v3, 0, (0xE2, ft.named['intro']))
    v1[0] = R(cmds=[(0xE3, 0), (0xE4, SYNC['intro'])])
    offbeat(v1, 4 * BAR, ['Am', 'F', 'C', 'G'])
    v2[0] = R()
    for r in range(8, 16):                      # snare fill into the verse
        v2[7 * BAR + r] = N('snare', 'C3')
    return v1, v2, v3


def section_verse():
    v1, v2, v3 = track(16), track(16), track(16)
    bass8(v1, 0, VERSE)
    gated(v3, 0, VERSE, ARP)
    melody(v2, 0, VERSE_LEAD, 'lead')
    cmd(v1, 0, (0xE4, SYNC['verse']))
    return v1, v2, v3


def section_chorus(sync):
    v1, v2, v3 = track(16), track(16), track(16)
    bass16(v1, 0, CHORUS)
    gated(v3, 0, CHORUS, ARP2, fill_every=8)
    melody(v2, 0, CHORUS_LEAD, 'sawlead')
    cmd(v1, 0, (0xE4, SYNC[sync]))
    return v1, v2, v3


def section_break():
    v1, v2, v3 = track(8), track(8), track(8)
    for i, n in enumerate(BREAK_BASS):
        v1[i * BAR] = N('longbass', n)
    cmd(v1, 0, (0xE2, ft.named['sweep']), (0xE4, SYNC['break']))
    melody(v2, 0, BREAK_LEAD, 'soft')
    v3[0] = R()
    for i, c in enumerate(v2[:-3]):             # the echo: three sixteenths later
        if c is not None:
            v3[i + 3] = R() if c.kind == 'rest' else N('echo', c.note)
    return v1, v2, v3


def section_build():
    v1, v2, v3 = track(4), track(4), track(4)
    offbeat(v1, 0, BUILD)
    cmd(v1, 0, (0xE2, ft.named['off']), (0xE4, SYNC['build']))
    melody(v2, 0, BUILD_LEAD, 'sawlead')
    for bar, step in enumerate((4, 2, 1, 1)):  # the roll gets denser
        for r in range(0, BAR, step):
            v3[bar * BAR + r] = N('snare', 'C3')
    return v1, v2, v3


def section_outro():
    v1, v2, v3 = track(8), track(8), track(8)
    pads(v3, 0, OUTRO)
    cmd(v3, 0, (0xE2, ft.named['outro']))
    offbeat(v1, 0, OUTRO[:4])
    for i, ch in enumerate(OUTRO[4:]):
        v1[(4 + i) * BAR] = N('obass', nn(ROOT[ch]) + 12)
    cmd(v1, 0, (0xE4, SYNC['outro']))
    cmd(v1, 4 * BAR, (0xE3, 25))               # fade out over the last 4 bars
    melody(v2, 0, OUTRO_LEAD, 'soft')
    return v1, v2, v3


# (section, transposition)
SONG = [(section_intro(), 0), (section_verse(), 0), (section_chorus('chorus'), 0),
        (section_break(), 0), (section_build(), 2), (section_chorus('chorus2'), 2),
        (section_outro(), 2)]


# --- packing ---------------------------------------------------------------
def pack_bar(cells, inst_state):
    """One bar of one voice -> pattern bytes. inst_state = [instrument, duration]
    at the start of the pattern is unknown, so the first event sets both."""
    out = []
    events = [(i, c) for i, c in enumerate(cells) if c is not None]
    if not events or events[0][0] != 0:
        events.insert(0, (0, Cell('tie')))
    cur_inst, cur_dur = None, None
    for k, (pos, c) in enumerate(events):
        end = events[k + 1][0] if k + 1 < len(events) else len(cells)
        d = end - pos
        for code, val in c.cmds:
            out += [code, val]
        if c.kind == 'note':
            ins = NAMES.index(c.inst)
            if ins != cur_inst:
                out.append(0x80 | ins)
                cur_inst = ins
        if d != cur_dur:
            out.append(0xC0 | (d - 1))
            cur_dur = d
        if c.kind == 'note':
            if c.porta:
                out += [0xE1, c.porta]
            elif c.legato:
                out.append(0xE0)
            assert 0 <= c.note < 0x60
            out.append(c.note)
        else:
            out.append(0x60 if c.kind == 'rest' else 0x61)
    out.append(0xFF)
    return bytes(out)


def build():
    used = {c.inst for voices, _ in SONG for rows in voices for c in rows
            if c is not None and c.kind == 'note'}
    NAMES[:] = [n for n in INSTR if n in used]
    assert len(NAMES) <= 64
    patterns, index = [], {}
    orders = [[], [], []]
    for (voices, tr) in SONG:
        for v, rows in enumerate(voices):
            orders[v].append(0xA0 + tr)
            for b in range(0, len(rows), BAR):
                data = pack_bar(rows[b:b + BAR], None)
                if data not in index:
                    index[data] = len(patterns)
                    patterns.append(data)
                orders[v].append(index[data])
    for o in orders:
        # drop repeated transpositions
        clean, last = [], None
        for x in o:
            if 0x80 <= x < 0xC0:
                if x == last:
                    continue
                last = x
            clean.append(x)
        o[:] = clean + [0xFF, 0]
    assert len(patterns) < 0x80
    return patterns, orders


def hexes(seq):
    return ', '.join(f'${x:02x}' for x in seq)


def emit(path):
    patterns, orders = build()
    freqs = freq_table()
    L = ['; Generated by music.py: "Neon Horizon". Do not edit.', '']
    L.append(f'speedtab .byte {SPEEDS[0]}, {SPEEDS[1]}')
    L.append('freqlo  .byte ' + hexes(f & 0xFF for f in freqs))
    L.append('freqhi  .byte ' + hexes(f >> 8 for f in freqs))
    for key in ('ad', 'sr', 'wave', 'pulse', 'filt', 'vdel', 'vibs', 'vibd', 'gate', 'flags', 'vis'):
        L.append(f'i_{key:<6}.byte ' + hexes(INSTR[n][key] for n in NAMES))
    for t, p in ((wt, 'wt'), (pt, 'pt'), (ft, 'ft')):
        L.append(f'{p}_l    .byte ' + hexes(l for l, r in t.rows))
        L.append(f'{p}_r    .byte ' + hexes(r for l, r in t.rows))
    L.append('ordlo   .byte <ord1, 0, 0, 0, 0, 0, 0, <ord2, 0, 0, 0, 0, 0, 0, <ord3')
    L.append('ordhi   .byte >ord1, 0, 0, 0, 0, 0, 0, >ord2, 0, 0, 0, 0, 0, 0, >ord3')
    for v, o in enumerate(orders):
        L.append(f'ord{v + 1}    .byte ' + hexes(o))
    L.append('pat_lo  .byte ' + ', '.join(f'<pat{i}' for i in range(len(patterns))))
    L.append('pat_hi  .byte ' + ', '.join(f'>pat{i}' for i in range(len(patterns))))
    for i, p in enumerate(patterns):
        L.append(f'pat{i:<4} .byte ' + hexes(p))
    L.append('')
    # the instrument names, for reading the patterns
    L.append('; instruments: ' + ', '.join(f'{i}={n}' for i, n in enumerate(NAMES)))
    for k, v in SYNC.items():
        L.append(f'SYNC_{k.upper()} = {v}')
    total = sum(len(p) for p in patterns) + sum(len(o) for o in orders)
    with open(path, 'w') as f:
        f.write('\n'.join(L) + '\n')
    bars = sum(len(v[0]) for v, _ in SONG) // BAR
    print(f'{path}: {bars} bars, {len(patterns)} patterns, {total} bytes of song, '
          f'{len(NAMES)} instruments, wave {len(wt.rows)}, pulse {len(pt.rows)}, filter {len(ft.rows)}')


if __name__ == '__main__':
    emit(os.path.join(os.path.dirname(os.path.abspath(__file__)), 'music.asm'))
