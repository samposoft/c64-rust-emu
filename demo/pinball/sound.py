"""STARFALL sound: effects (one SID voice each, frame by frame) and the
attract-mode tune, as 64tass data."""

PAL_CLOCK = 985248

TRI, SAW, PULSE, NOISE = 0x10, 0x20, 0x40, 0x80
GATE = 1


def freq(hz):
    return max(0, min(0xFFFF, round(hz * 16777216 / PAL_CLOCK)))


def note_hz(n):
    """MIDI note number to Hz."""
    return 440.0 * 2 ** ((n - 69) / 12)


class Effect:
    def __init__(self, name, prio, ad, sr, pw=0x08):
        self.name, self.prio, self.ad, self.sr, self.pw = name, prio, ad, sr, pw
        self.frames = []

    def f(self, wave, hz, gate=True):
        self.frames.append((wave | (GATE if gate else 0), freq(hz)))
        return self

    def sweep(self, wave, hz0, hz1, n, gate=True):
        for i in range(n):
            t = i / max(1, n - 1)
            self.f(wave, hz0 * (hz1 / hz0) ** t, gate)
        return self

    def data(self):
        b = [self.prio, self.ad, self.sr, self.pw]
        for ctrl, fq in self.frames:
            b += [ctrl, fq & 255, fq >> 8]
        b.append(0)
        return b


def effects():
    E = []
    # voice 1: mechanics
    E.append(Effect("flip", 1, 0x00, 0x00).f(NOISE, 3000).f(NOISE, 1400).f(NOISE, 700, False))
    E.append(Effect("launch", 2, 0x02, 0x08).sweep(NOISE, 400, 3200, 10).f(NOISE, 3200, False))
    E.append(Effect("pull", 1, 0x00, 0x00).f(PULSE, 90).f(PULSE, 90, False))
    # voice 2: impacts
    E.append(Effect("bumper", 2, 0x00, 0x09, 0x06).sweep(PULSE, 900, 260, 6).f(PULSE, 200, False))
    E.append(Effect("sling", 2, 0x00, 0x07).f(NOISE, 2600).sweep(PULSE, 500, 250, 3).f(PULSE, 200, False))
    E.append(Effect("target", 2, 0x00, 0x08).f(NOISE, 1200).sweep(TRI, 300, 120, 4).f(TRI, 100, False))
    E.append(Effect("thud", 1, 0x00, 0x05).f(NOISE, 700).f(TRI, 150, False))
    E.append(Effect("star", 2, 0x00, 0x0a).f(TRI, 1318).f(TRI, 1760).f(TRI, 2637, False))
    # voice 3: lamps, jingles
    E.append(Effect("ding", 2, 0x00, 0x0a).f(TRI, 2093).f(TRI, 2093, False))
    E.append(Effect("lanes", 3, 0x00, 0x09, 0x04)
             .f(PULSE, 523).f(PULSE, 523).f(PULSE, 659).f(PULSE, 659)
             .f(PULSE, 784).f(PULSE, 784).f(PULSE, 1047).f(PULSE, 1047).f(PULSE, 1047, False))
    E.append(Effect("saucer", 3, 0x00, 0x0a).sweep(TRI, 1600, 150, 14).f(TRI, 150, False))
    E.append(Effect("eject", 3, 0x00, 0x06).f(NOISE, 900).sweep(PULSE, 200, 600, 4).f(PULSE, 600, False))
    E.append(Effect("drain", 4, 0x08, 0x0a, 0x08).sweep(PULSE, 440, 70, 40).f(PULSE, 70, False))
    e = Effect("jingle", 4, 0x00, 0x0a, 0x04)
    for n in (72, 76, 79, 84, 79, 84, 88, 91):
        e.f(PULSE, note_hz(n)).f(PULSE, note_hz(n)).f(PULSE, note_hz(n))
    e.f(PULSE, note_hz(96)).f(PULSE, note_hz(96)).f(PULSE, note_hz(96), False)
    E.append(e)
    E.append(Effect("tick", 2, 0x00, 0x04, 0x04).f(PULSE, 1760).f(PULSE, 1760, False))
    E.append(Effect("tilt", 5, 0x00, 0xf8).f(SAW, 60).f(SAW, 62).f(SAW, 60).f(SAW, 62)
             .f(SAW, 60).f(SAW, 62).f(SAW, 60).f(SAW, 62).f(SAW, 60).f(SAW, 62).f(SAW, 60, False))
    e = Effect("start", 4, 0x00, 0x09, 0x04)
    for n in (60, 64, 67, 72, 67, 72, 76, 79):
        e.f(PULSE, note_hz(n)).f(PULSE, note_hz(n))
    e.f(PULSE, note_hz(84)).f(PULSE, note_hz(84)).f(PULSE, note_hz(84), False)
    E.append(e)
    e = Effect("over", 4, 0x00, 0x0b, 0x06)
    for n in (67, 64, 60, 55):
        for _ in range(6):
            e.f(TRI, note_hz(n))
    e.f(TRI, note_hz(48)).f(TRI, note_hz(48), False)
    E.append(e)
    e = Effect("extra", 5, 0x00, 0x0a, 0x04)
    for _ in range(3):
        for n in (79, 84, 88, 91):
            e.f(PULSE, note_hz(n)).f(PULSE, note_hz(n))
    e.f(PULSE, note_hz(96), False)
    E.append(e)
    return E


# ---------------------------------------------------------------- tune
# A minor, 4 bars looped twice with a variation; 6 frames per sixteenth.

TEMPO = 6


def tune():
    """Three voices of (midi note or 0 for rest, sixteenths)."""
    A, C, D, E, F, G = 57, 60, 62, 64, 65, 67
    # voice 1: bass, eighths
    bass = []
    for root in (45, 45, 41, 43, 45, 45, 41, 40):          # Am Am F G | Am Am F E
        for k in range(8):
            bass.append((root + (12 if k % 2 else 0), 2))
    # voice 2: melody
    mel = [
        (76, 4), (74, 2), (72, 2), (71, 4), (72, 4),
        (69, 6), (72, 2), (76, 8),
        (77, 4), (76, 2), (74, 2), (72, 4), (74, 4),
        (74, 6), (71, 2), (67, 8),
        (76, 4), (74, 2), (72, 2), (71, 4), (72, 4),
        (69, 6), (72, 2), (81, 8),
        (79, 4), (77, 2), (76, 2), (74, 4), (72, 4),
        (71, 6), (68, 2), (69, 8),
    ]
    # voice 3: arpeggios, sixteenths
    chords = [(57, 60, 64), (57, 60, 64), (53, 57, 60), (55, 59, 62),
              (57, 60, 64), (57, 60, 64), (53, 57, 60), (52, 56, 59)]
    arp = []
    for ch in chords:
        for k in range(16):
            arp.append((ch[(0, 1, 2, 1)[k % 4]] + 12, 1))
    return bass, mel, arp


def asm():
    s = []
    E = effects()
    s.append("; sound effects: prio, AD, SR, pulse width hi, frames (ctrl, freq lo, hi), 0\n")
    for i, e in enumerate(E):
        s.append(f"SFX_{e.name.upper():<8}= {i}\n")
    s.append("sfxlo   .byte " + ",".join(f"<sfx_{e.name}" for e in E) + "\n")
    s.append("sfxhi   .byte " + ",".join(f">sfx_{e.name}" for e in E) + "\n")
    for e in E:
        d = e.data()
        s.append(f"sfx_{e.name}\n")
        for i in range(0, len(d), 16):
            s.append("        .byte " + ",".join(f"${b:02x}" for b in d[i:i + 16]) + "\n")
    # tune: per voice, pairs (note index 0-95 or $ff rest, frames), 0 = loop
    s.append("\n; tune: note (0 = rest, 1-95 = midi-23), frames; $00 ends (loops)\n")
    for i, v in enumerate(tune()):
        b = []
        for n, d in v:
            b += [0 if n == 0 else n - 23, d * TEMPO]
        b += [0, 0]
        s.append(f"tune{i}\n")
        for j in range(0, len(b), 16):
            s.append("        .byte " + ",".join(f"${x:02x}" for x in b[j:j + 16]) + "\n")
    notes = [freq(note_hz(n + 23)) if n else 0 for n in range(96)]
    s.append("notelo  .byte " + ",".join(f"${f & 255:02x}" for f in notes) + "\n")
    s.append("notehi  .byte " + ",".join(f"${f >> 8:02x}" for f in notes) + "\n")
    return "".join(s)
