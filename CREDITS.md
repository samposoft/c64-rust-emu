# Credits

This emulator is Copyright (C) 2026 SampoSoft - Francesco Sampoli and is
distributed under the GNU General Public License, version 2 or (at your
option) any later version (see `LICENSE`). It comes with ABSOLUTELY NO
WARRANTY.

Several parts are ports of, or are based on, the code of two GPL projects.
Their authors keep their copyright on that code, and the whole program is
distributed under the same license terms. The derived files were ported to
Rust and modified by SampoSoft in 2026; each of them says so in its header.

## VICE

VICE, the Versatile Commodore Emulator (<https://vice-emu.sourceforge.io/>),
Copyright (C) the VICE Team: see the "Copyright notice" section of VICE's
README for the complete list of core team members and former contributors.
Licensed under the GNU General Public License, version 2 or any later
version. Version 3.10 (x64sc) is the reference of this emulator: most modules
are compared with it cycle by cycle by regression tests.

### Ported from the VICE source code

| Module here | VICE sources | Authors (from the file headers) |
|---|---|---|
| `src/vic/mod.rs` (VIC-II, cycle-exact) | `src/viciisc/vicii-cycle.c`, `vicii-fetch.c`, `vicii-draw-cycle.c`, `vicii-mem.c`, `vicii-chip-model.c` | Hannu Nuotio, Daniel Kahlin, Ettore Perazzoli, Andreas Boose |
| `src/vic/tables.rs` (cycle tables of the 6569 and 6567R8) | `src/viciisc/vicii-chip-model.c` | Hannu Nuotio, Daniel Kahlin |
| `src/tape.rs` (Datasette, TAP, T64 and its KERNAL tape traps) | `src/datasette/datasette.c`, `src/tape/tap.c`, `src/tape/t64.c`, `src/tape/tape.c` | Andreas Boose, Andreas Matthies, Marco van den Heuvel, David Hansel, Ettore Perazzoli, Compyx, Jouko Valta |
| `src/random.rs` (PCG random generator: power-on RAM, tape azimuth error) | `src/lib.c` | Andreas Boose, Marco van den Heuvel |
| `src/mem/power_on.rs` (RAM pattern and color RAM at power-on) | `src/ram.c`, `src/vicii-colorram.h` (color RAM measured by William McCabe) | Andreas Matthies, Marco van den Heuvel |
| `src/drive/mod.rs` (1541 disk rotation and read/write circuit) | `src/drive/rotation.c` | Andreas Boose, Istvan Fabian, Benjamin 'BeRo' Rosseaux, Peter Rittwage |
| `src/sid/mod.rs` (`audio_mix`, mono mix of two SIDs) | `src/sound.c` (`sound_audio_mix`) | Teemu Rantanen, Marco van den Heuvel, Ettore Perazzoli |
| `src/net/cs8900.rs` (CS8900A Ethernet controller: PacketPage, transmit and receive, address filter) | `src/core/cs8900.c` | Spiro Trikaliotis, Christian Vogelgsang |

### Behaviour modelled on VICE

These modules follow the behaviour and timings of VICE's code, described in
their comments and checked against VICE by the tests:

| Module here | VICE sources | Authors (from the file headers) |
|---|---|---|
| `src/timing.rs` (PAL and NTSC clocks, raster and mains frequency) | `src/c64/c64.h`, `src/c64/c64.c` (`machine_change_timing`) | the VICE Team |
| `src/cia/mod.rs` (CIA 6526 timers and interrupts; written from the description, not copied) | `src/core/ciacore.c`, `src/core/ciatimer.c` | Andre Fachat, Ettore Perazzoli, Andreas Boose, Alexander Bluhm, Olaf Seibert |
| `src/drive/via.rs` (VIA 6522 of the 1541) | `src/core/viacore.c` | Andre Fachat, Andreas Boose, Olaf Seibert |
| `src/drive/gcr.rs` (D64 to GCR tracks) | `src/gcr.c`, `src/diskimage/fsimage-gcr.c` | Andreas Boose, Daniel Sladic, Kajtar Zsolt |
| `src/mem/mod.rs`, `src/keyboard.rs` (CIA1 ports: keyboard matrix solver, ghost keys, backwards scanning) | `src/c64/c64cia1.c` | Andre Fachat, Ettore Perazzoli, Andreas Boose, Marco van den Heuvel |
| `src/mem/mod.rs` (processor port $00/$01: pin values, bits 6-7 fall-off) | `src/c64/c64pla.c`, `src/c64/c64memsc.c` | Andreas Boose, Ettore Perazzoli, Marco van den Heuvel |
| `src/ctrlport.rs` (1351 mouse reading and movement limit, POT port selection) | `src/joyport/mouse_1351.c`, `src/joyport/mouse.c`, `src/joyport/joyport.c` | Marco van den Heuvel, Andreas Boose, Hannu Nuotio |
| `src/ctrlport.rs` (1351 joystick mode: direction pulses, right button on POTX) | `src/joyport/mouse_digital.c` | the VICE Team |
| `src/reu.rs` (REU 1700/1764/1750) | `src/c64/cart/reu.c` | Wolfgang Moser, Jouko Valta, Richard Hable, Ettore Perazzoli |
| `src/net/mod.rs` (Ethernet cartridge mapping: TFE, RR-Net with A3 inverted) | `src/c64/cart/ethernetcart.c`, `src/c64/cart/clockport-rrnet.c` | Marco van den Heuvel |
| `src/cart/flash.rs` (Am29F040 and compatible flash) | `src/core/flash040core.c` | Hannu Nuotio, Marko Mäkelä |
| `src/cart/eeprom.rs` (M93C86 EEPROM of GMod2) | `src/core/m93c86.c`, `src/c64/cart/gmod2.c` | Groepaz |
| `src/cart/mod.rs` (cartridge types) | `src/c64/cart/*.c` (e.g. `easyflash.c`: ALeX Kazik, Marco van den Heuvel) | the VICE Team |

## reSID

reSID, the MOS 6581/8580 SID emulation engine, Copyright (C) 2010 Dag Lem
<resid@nimrod.no>, licensed under the GNU General Public License, version 2 or
any later version. The SID in `src/sid/` is a port of reSID 1.0 as shipped with
VICE 3.10 (ReSID engine with the `filter8580new` filter):

- `src/sid/mod.rs`, `wave.rs`, `envelope.rs`, `dac.rs`, `extfilt.rs`,
  `filter.rs`: ported from `sid.cc`, `wave.h`/`wave.cc`, `envelope.h`/`envelope.cc`,
  `dac.cc`, `extfilt.h`, `filter8580new.h`/`filter8580new.cc`;
- `src/sid/samples/*.dat`: the combined-waveform tables of reSID, unchanged;
- the op-amp transfer tables and filter constants in `src/sid/filter.rs`.

## Sources of the CRT emulation

The monitor emulation (`src/crt.rs`, `src/frontend/crt.wgsl`) was written
for this emulator; no code was copied. Its figures come from:

- Philip "Pepto" Timmermann, "colodore" (2017),
  <https://www.pepto.de/projects/colorvic/>: luma levels, chroma angles and
  the YUV to RGB conversion of the VIC-II colors (the palette of the
  emulator comes from it too);
- the chroma phase of odd and even lines measured by Tobias on the 6569R5,
  as listed in VICE's `src/vicii/vicii-color.c`;
- the luma step response measured by Jam on Toads,
  <https://jamontoads.net/p/lumachroma.html>;
- the Commodore 1084S-P1, 1084S-D1 and 1901 service manuals (picture tube,
  slot pitch, picture size, video bandwidth, PAL decoder and delay line; for
  the 1901 also the luma peaking network, the chroma band-pass and the white
  point, from its schematics);
- the Philips CP90 chassis service manual (TV 15CE1510) and the Philips data
  handbook T08 (1986) for its picture tube; the EPCOS K2966M datasheet for
  the response of a PAL B/G IF filter; the TDA3562A datasheet's application
  circuit for the chroma band-pass of a TV decoder;
- the C64 Service Manual, for the luma network of the RF modulator
  (schematics 251025 and 251696);
- the Commodore 1701/1702 service manual (314004-01) and the PAL 1701/1702
  user's guides, for the NTSC monitor.

The ways of drawing the beam (energy-conserving gaussian integrated over
the pixel, wider when brighter) and the slot mask follow ideas described
for the libretro shaders crt-royale and crt-guest-advanced, and by John
Novak's articles on period-correct emulation.

## Not included

The ROMs of the C64 and of the 1541 (KERNAL, BASIC, character generator, DOS)
are copyrighted by Commodore and are not part of this repository: they must be
placed in `roms/` by the user.
