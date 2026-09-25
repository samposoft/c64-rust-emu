# EARTHRISE

A photo of the Earth over the Moon pushed as far as a stock C64 goes: an
IFLI picture (interlaced multicolor FLI) with a converter written for it,
and the same picture as a Koala for comparison. Under it, in the lower
border, three VU meters follow an original ambient tune.

The VIC-II has 16 colors and, in multicolor bitmap mode, 4 per 4x8 cell.
IFLI works around both limits:

- **FLI**: a bad line forced on every raster line makes the VIC fetch the
  screen colors again from a different screen, so the two screen colors
  change every line (4x1 cells instead of 4x8).
- **Interlace**: two FLI pictures alternate at 50 Hz and the eye mixes them.
  Every pixel can be one of up to 16x16 averages of two colors (136 different
  mixes). Pairs with close lightness do not flicker.
- **Half-pixel shift**: frame B is one hires pixel to the right
  (`$D016` = 1), so the mix has 320 pixels of horizontal detail instead of
  160.
- **Color RAM per frame**: between frames the viewer rewrites the color RAM
  cells that differ, so the fourth color of each 4x8 cell is not shared by
  the two frames either.

## Building

The source picture is not ours: it is not in the repository. Put it in this
directory as `source.webp` (or pass another picture to `convert.py`), then:

```bash
cd demo/earthrise
python3 convert.py                      # build/: data and previews (numpy, Pillow)
python3 music.py                        # music.asm (already in the repository)
64tass -C -o build/ifli.prg --labels=build/ifli.lbl ifli.asm
64tass -C -o build/koala.prg koala.asm
../../target/release/c64 --blend build/ifli.prg
```

At the `READY.` prompt type `RUN`. SPACE goes back to BASIC, and `RUN`
shows the picture again. The PRGs also run in VICE
(`x64sc -autostart build/ifli.prg`).

`--blend` makes the emulator show every frame mixed with the previous one,
as the eye sees a CRT. Without it, a 60 or 120 Hz monitor shows the 50
frames per second unevenly and the mix flickers.

`convert.py --help` lists the knobs: crop (`--x0`, `--width`), saturation,
gamma, star recovery, the chroma weight of the error, the flicker penalty
(`--flicker`, higher for a real CRT, 0 for the emulator with `--blend`),
the ordered dithering amplitude and the fitting passes.

The picture area of `build/ifli.prg` (the meters aside) matches the
converter's `expect_a.png`/`expect_b.png` pixel by pixel.

Previews in `build/`: `target.png` (the source as scaled), `koala.png`,
`fli.png` (a single FLI frame), `ifli.png` (the two frames mixed),
`ifli_a.png`, `ifli_b.png`.

## The converter

For each frame, with the other one fixed, `convert.py` picks the colors of
every cell by exhaustive search: 136 screen color pairs × 16 color RAM
values, scored by the CIELAB error of the mixed result, with chroma weighted
more than CIELAB does (the C64 blue is far more violet than the Earth's
oceans) and a penalty on the lightness difference between the frames
(flicker). Each pixel then takes the best of the four colors of its cell.
The two frames alternate until the error stops improving (about six passes).
The mixes are computed in linear light, as the eye averages light.

Dithering is ordered, folded into the search: the target lightness is offset
by a 4x4 Bayer pattern, so the search picks colors that alternate around it.
Error diffusion was tried and dropped: with 4 colors per cell it drew
horizontal worms.

Scaling averages the stars away (they are 1-2 pixels in the source): on the
dark sky the positive detail (luminance minus its blur) is max-pooled into
the target pixel instead, and added back.

The target is scaled for the PAL pixel aspect (0.9365): on a TV the Earth is
round. The emulator draws square pixels, so there it is 7% wider.

## The music

"Earthrise" (`music.py`, written for this demo): D minor, a sixteenth every
8 frames (94 BPM), four bars of intro, then a 16-bar loop, 51 seconds in all.

| Voice | |
|---|---|
| 1 | pulse pad with slow PWM through the low-pass filter; the cutoff follows a triangle LFO of about 20 seconds, resonance 14 |
| 2 | arpeggio in eighths over the chord (with 7th and 9th); on the off sixteenths each note comes back three sixteenths later on a softer triangle, like an echo |
| 3 | filtered noise wind in the intro, a triangle bass in section A, a pulse lead with delayed vibrato in section B |

The player (`player.asm`) comes from ROBO NINE's: wave tables, hard
restart, PWM, vibrato.

The meters show one voice each, 36 segments (green, yellow, red) and a peak
that stays for 30 frames and then falls. The SID lets the CPU read only
voice 3's envelope, so the player imitates all three: for each instrument,
`music.py` turns the ADSR (with the SID's times) and a loudness into steps
per frame of attack, decay and release, and a sustain level.

## The viewer

`ifli.asm` runs without interrupts: it syncs once to the exact cycle, then
every frame takes exactly 312 × 63 cycles of straight code and counted
delays.

| Memory | |
|---|---|
| `$0801` | code, meter tables, player and tune, color RAM code (`build/ifli_colors.asm`) |
| `$4000-$7FFF` | frame A: screens 0-7 at `$4000-$5FFF`, bitmap at `$6000` (VIC bank 1) |
| `$8000-$BFFF` | frame B as loaded, copied at start to `$C000-$FFFF` (bank 3) |

Frame B needs bank 3: banks 0 and 2 show the character ROM at
`$1000-$1FFF`, which would cover four of the eight screens. In bank 3,
`$D000-$DFFF` is RAM under the I/O, and the VIC always sees RAM.

Each frame:

1. Raster `$33` is a normal bad line (Y scroll 3). The code gets there so
   that its `STA $D011` writes at cycle 11: the CPU stops at the next read
   until cycle 54.
2. For raster `$34-$F7`, 23 cycles per line: `$D018` (screen `y & 7`)
   in the right border of the previous line, then `$D011` with Y scroll =
   raster & 7 at cycle 14. That is after the row counter check (earlier
   would restart the character row) and before the fetches: the VIC reads
   the 40 screen bytes again. The bad line stops the CPU until cycle 54,
   which keeps the loop in step. The VIC starts the forced fetch three
   cycles late, so the first three character columns show the FLI bug;
   the picture keeps them black.
3. Bad lines cannot be forced after `$F7`: lines 197-199 keep the colors of
   line 196, and the converter knows it.
4. At line `$F8` `$D011` goes to 24 rows: its bottom compare (`$F7`) has
   passed and the 25-row one (`$FB`) will not match, so the lower border
   stays open for the meters, which are six X-expanded sprites on lines
   252-272.
5. The player runs: it takes a different time every frame (at most until
   line 273), so the code syncs to the exact cycle again at line 280, as at
   the start.
6. In constant time: draw the meters into the bank of the next frame,
   rewrite the color RAM cells that differ between the frames (6 cycles
   each), check SPACE, wait, then in the top border switch the VIC bank
   (`$DD00`) and `$D016`, and go back to 25 rows before the compare at
   `$33`.

Each bank holds its own copy of the meter sprites, in blocks the pictures
do not use: row 24 of screens 5-7 (lines 197-199 use screen 4) and the end
of the bitmap. With the borders open the VIC shows the idle byte (`$3FFF` of
the bank) outside the picture, so `$7FFF` and `$FFFF` stay 0: bank 3's IRQ
vector is cleared (interrupts are off), and its NMI vector becomes sprite
data once the NMI below has come.

RESTORE would break the timing: a CIA2 NMI that is never acknowledged
keeps the NMI line low, so no other edge can come.

Both frames were checked pixel by pixel against the converter's
`expect_a.png`/`expect_b.png`, in this emulator and in VICE 3.10 (x64sc,
stopped by its monitor at the frame switch).
