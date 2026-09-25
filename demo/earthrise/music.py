#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-2.0-or-later
# Copyright (C) 2026 SampoSoft - Francesco Sampoli
"""Writes music.asm for ifli.asm (player in player.asm).

Tune: "Earthrise" - D minor, SPEED 8 (a sixteenth every 8 frames, 94 BPM),
four bars of intro, then a 16-bar loop.
  V1: filtered pulse pad with slow PWM; the cutoff follows a slow LFO
  V2: arpeggio in eighths, each note echoed three sixteenths later by a
      softer triangle
  V3: filtered noise wind in the intro, then a triangle bass (section A)
      and a pulse lead with delayed vibrato (section B)

The meters imitate the SID envelope: for each instrument, the steps per
frame of attack, decay and release and the sustain level come from its
ADSR, with the SID's times, scaled to its loudness (`vu`: a triangle is
quieter than a pulse).
"""

import os

SPEED = 8

SEMI = {'C': 0, 'C#': 1, 'Db': 1, 'D': 2, 'D#': 3, 'Eb': 3, 'E': 4, 'F': 5,
        'F#': 6, 'Gb': 6, 'G': 7, 'G#': 8, 'Ab': 8, 'A': 9, 'A#': 10, 'Bb': 10, 'B': 11}


def nn(s):
    """'D5' -> MIDI note (D4 = 62)."""
    return 12 * (int(s[-1]) + 1) + SEMI[s[:-1]]


# wt: list of (waveform, note) per frame; an int note is relative to the
# played note, ('a', n) is absolute. HOLD stops the table, LOOP restarts it.
HOLD, LOOP = 'hold', 'loop'
INSTR = {
    'pad':   dict(ad=0xA9, sr=0xBB, wt=[(0x41, 0), HOLD], pw=0x04, pws=0x06, flt=1, vu=0.75),
    'ping':  dict(ad=0x09, sr=0x00, wt=[(0x41, 12), (0x41, 0), HOLD], pw=0x02, pws=0x10, vu=1.0),
    'echo':  dict(ad=0x08, sr=0x00, wt=[(0x11, 0), HOLD], vu=0.55),
    'wind':  dict(ad=0xD0, sr=0xFC, wt=[(0x81, 0), HOLD], flt=1, vu=0.8),
    'bass':  dict(ad=0x2A, sr=0xA9, wt=[(0x11, 0), HOLD], vu=0.85),
    'lead':  dict(ad=0x59, sr=0xAA, wt=[(0x41, 12), (0x41, 0), HOLD],
                  pw=0x07, pws=0x0C, vdel=24, vsh=4, vu=0.95),
}
IDX = {name: i + 1 for i, name in enumerate(INSTR)}

# SID envelope times in ms
ATTACK = [2, 8, 16, 24, 38, 56, 68, 80, 100, 250, 500, 800, 1000, 3000, 5000, 8000]
DECAY = [6, 24, 48, 72, 114, 168, 204, 240, 300, 750, 1500, 2400, 3000, 9000, 15000, 24000]


def vu_rates(ins):
    """Meter envelope (0-255 scale, peak = the instrument's loudness `vu`):
    steps per frame of attack, decay and release, sustain level, peak."""
    peak = round(255 * ins['vu'])
    step = lambda ms: max(1, min(255, round(peak * 20 / ms)))
    a, d = ins['ad'] >> 4, ins['ad'] & 15
    s, r = ins['sr'] >> 4, ins['sr'] & 15
    return step(ATTACK[a]), step(DECAY[d]), round(s * peak / 15), step(DECAY[r]), peak


# --- the tune ---------------------------------------------------------------

INTRO = ['Dm', 'Dm', 'Bb', 'Bb']
SEC_A = ['Dm', 'Bb', 'F', 'C', 'Dm', 'Bb', 'Gm', 'A']
SEC_B = ['Bb', 'C', 'Dm', 'Dm', 'Bb', 'C', 'A', 'A']

ROOT = {'Dm': 'D', 'Bb': 'Bb', 'F': 'F', 'C': 'C', 'Gm': 'G', 'A': 'A'}
# chord tones for the arpeggio, in semitones from the root (with 7th and 9th)
TONES = {'Dm': [0, 3, 7, 10, 12, 14, 15, 19], 'Gm': [0, 3, 7, 10, 12, 14, 15, 19],
         'Bb': [0, 4, 7, 11, 12, 14, 16, 19], 'F': [0, 4, 7, 11, 12, 14, 16, 19],
         'C': [0, 4, 7, 11, 12, 14, 16, 19], 'A': [0, 4, 7, 10, 12, 13, 16, 19]}
# pad: a line that moves by steps (A Bb A G A Bb Bb A)
PAD = {'Dm': 'A3', 'Bb': 'Bb3', 'F': 'A3', 'C': 'G3', 'Gm': 'Bb3', 'A': 'A3'}


def arp_root(ch):
    """Root of the arpeggio between A3 and G#4."""
    n = nn(ROOT[ch] + '4')
    return n - 12 if n > nn('G#4') else n


def padbar(ch):
    return [('pad', nn(PAD[ch]))] + [None] * 15


def arpbar(ch, kind='full'):
    """Eighths up and down the chord; on the off sixteenths the echo of the
    note three sixteenths before, softer."""
    order = [0, 2, 4, 5, 6, 5, 4, 2] if kind != 'up' else [0, 1, 2, 4, 5, 6, 7, 6]
    notes = [arp_root(ch) + TONES[ch][i] for i in order]
    rows = [None] * 16
    for i, n in enumerate(notes):
        rows[2 * i] = ('ping', n)
        if i > 0:
            rows[2 * i + 1] = ('echo', notes[i - 1])
    if kind == 'sparse':
        rows = [('ping', notes[i // 2]) if i % 4 == 0 else None for i in range(16)]
    return rows


def bassbar(ch):
    r = nn(ROOT[ch] + '2')
    r = r + 12 if r < nn('A1') else r
    return [('bass', r)] + [None] * 9 + [('bass', r + 12), None, ('bass', r)] + [None] * 3


def phrase(*items):
    """'A4:2' = a note of 2 sixteenths, 'r:4' = rest (gate off)."""
    rows = []
    for it in items:
        name, d = it.split(':')
        rows.append('off' if name == 'r' else ('lead', nn(name)))
        rows += [None] * (int(d) - 1)
    return rows


pad = sum((padbar(ch) for ch in INTRO + SEC_A + SEC_B), [])
pad[len(INTRO) * 16 - 4] = 'off'              # breath before section A

arps = [None] * 32 + arpbar('Bb', 'sparse') + arpbar('Bb', 'up')
arps += sum((arpbar(ch) for ch in SEC_A), [])
arps += sum((arpbar(ch, 'up' if i in (3, 7) else 'full') for i, ch in enumerate(SEC_B)), [])

third = [('wind', 84)] + [None] * 39 + ['off'] + [None] * 23
third += sum((bassbar(ch) for ch in SEC_A), [])
third += phrase(
    'F5:8', 'D5:4', 'F5:4',
    'G5:8', 'E5:4', 'C5:4',
    'A5:12', 'G5:2', 'F5:2',
    'E5:4', 'F5:4', 'D5:8',
    'D6:8', 'C6:4', 'Bb5:4',
    'C6:8', 'G5:4', 'E5:4',
    'C#6:8', 'A5:4', 'E5:4',
    'A5:12', 'r:4')

voices = [pad, arps, third]
assert all(len(v) == len(pad) and len(v) % 16 == 0 for v in voices), [len(v) for v in voices]
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


def hx(vals):
    return ', '.join('$%02x' % v for v in vals)


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

    rates = [(0, 0, 0, 0, 0)] + [vu_rates(ins) for ins in INSTR.values()]
    L = ['; Generated by music.py - do not edit by hand.', '',
         'SPEED   = %d' % SPEED, '']
    for label, key in [('i_ad', 'ad'), ('i_sr', 'sr'), ('i_pw', 'pw'), ('i_pws', 'pws'),
                       ('i_vdel', 'vdel'), ('i_vsh', 'vsh'), ('i_flt', 'flt')]:
        L.append('%-7s .byte %s' % (label, hx(field(key))))
    L.append('%-7s .byte %s' % ('i_wt', hx([0] + [wstart[n] for n in INSTR])))
    for i, label in enumerate(['i_va', 'i_vd', 'i_vs', 'i_vr', 'i_vmax']):
        L.append('%-7s .byte %s' % (label, hx([r[i] for r in rates])))
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
    bars = len(pad) // 16
    print('music: %d bars, %d patterns, %.1f s (loop %.1f s)' % (
        bars, len(pats), bars * 16 * SPEED / 50, (bars - LOOP_BAR) * 16 * SPEED / 50))
    return L


def main():
    here = os.path.dirname(os.path.abspath(__file__))
    with open(os.path.join(here, 'music.asm'), 'w') as f:
        f.write('\n'.join(music_lines()) + '\n')


if __name__ == '__main__':
    main()
