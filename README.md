# c64

Commodore 64 emulator in Rust: 6510 CPU with illegal opcodes, cycle-exact
VIC-II, SID, CIA, 1541 drive, Datasette, REU, keyboard, joystick, paddles,
1351 mouse, PRG/D64/G64/TAP/T64/CRT.
The frontend uses only pure Rust crates (`winit`, `softbuffer`, `cpal`, `gilrs`):
no external libraries to install, a single executable on macOS, Linux
and Windows.

## Build

```bash
cargo build --release
./target/release/c64 prg/game.d64                          # GUI
./target/release/c64 prg/game.tap                          # from tape
./target/release/c64 side_a.tap side_b.tap                 # several media: F8 switches to the next one
./target/release/c64 new.tap                               # blank tape (file does not exist) for SAVE
./target/release/c64 --no-turbo --tape-sound prg/game.tap  # with the tape sound
./target/release/c64term prg/game.d64                      # in the terminal, below the prompt (macOS, Linux)
./target/release/c64term --fullscreen prg/game.d64         # in the terminal, whole window
./target/release/c64dbg prg/game.d64                       # headless debugger, see DEBUGGER.md
./target/release/c64dbg --window prg/game.d64              # debugger with a window
./target/release/c64 --reu 512 prg/game.d64                # with a 512 KB REU 1750
./target/release/c64 --sid 8580 prg/game.d64               # SID 8580 instead of 6581
./target/release/c64 --sid2 d420 prg/music.prg             # second SID at $D420 (stereo)
./target/release/c64 --port1 mouse prg/program.prg         # 1351 mouse in control port 1
./target/release/c64 --port1 joymouse prg/geos.d64         # 1351 in joystick mode (GEOS 1.x)
./target/release/c64 --port1 paddles prg/Arkanoid.d64      # paddles in control port 1
./target/release/c64 --remote prg/game.d64                 # remote monitor for other programs, see DEBUGGER.md
./target/release/c64 --blend demo/earthrise/build/ifli.prg # frames mixed as the eye sees a CRT (interlace pictures)
./target/release/c64mcp                                    # MCP server for Claude, see below
```

The ROMs `kernal.rom`, `basic.rom`, `chargen.rom` are needed in the `roms/`
directory (next to the executable or in a directory above it, as the
project's for `target/release`, otherwise in the current directory), plus
`dos1541.rom` (16 KB; VICE's names `1541.rom` and
`dos1541-325302-01+901229-05.bin` also work) for the 1541 drive. Without the
drive ROM, or with `--no-drive`, D64s are loaded through the KERNAL trap.

While the drive is loading, the emulator runs at maximum speed (turbo, about
10 times real time, without audio; the window title and the status bar show
it) and goes back
to 50 frames/s as soon as the drive stops. The emulation stays identical:
only the waiting changes. Loading is detected from disk reads and writes,
changes of the serial bus lines and DOS jobs queued with the motor on
(spinning up the motor takes almost a second without disk accesses); not
from the motor alone, which the DOS leaves running for about 3 seconds after
the end. A PRG given on the command line is injected once the KERNAL has
booted, and the boot runs at maximum speed as well. With `--no-turbo` loads
run at real speed.

The application icon (Dock on macOS, window on Windows and X11) is in
`assets/`: `icon.png` and `icon64.rgba` are drawn from shapes and pixel digits, with
no third-party logos or fonts.

Platform requirements:
- **macOS**: nothing besides Rust.
- **Windows**: nothing besides Rust.
- **Linux**: the `gamepad` feature (on by default) builds `gilrs`, which
  requires `libudev-dev`; with `cargo build --release --no-default-features`
  the gamepad is left out and nothing else is needed. For audio, ALSA is
  already available on any desktop.

## In the terminal: `c64term`

`c64term` runs the emulator inside the terminal, with the same arguments,
keys, audio and gamepad as the window.

- **Inline** (default): reserves below the prompt only the rows it needs,
  that is the screen at the largest integer scale that fits in the window (at
  most 3×) plus a status line, scrolling the terminal if there are not
  enough. The screen sits on the left, aligned with the text like the output
  of a command, and the terminal colors stay around it. On exit the last
  frame stays in the scrollback and the prompt comes back right below it. If
  the window is resized the area is recomputed.
- **`--fullscreen`**: uses the whole window in the alternate screen, like a
  fullscreen application (`less`, `vim`), with the screen centered on a black
  background; on exit the terminal goes back to how it was.

At startup it asks the terminal what it supports and chooses by itself:

- **Graphics**: in terminals with the Kitty graphics protocol (Kitty, Ghostty,
  WezTerm) the screen is at full resolution, scaled by an integer factor and
  centered. Locally the frame goes through shared memory and only its name is
  sent to the terminal; over ssh it travels as base64 (the audio, however,
  comes out of the remote computer). In other terminals (Terminal.app,
  iTerm2, xterm) it is drawn with `▀` half blocks, two pixels per character:
  with 403×143 characters every pixel is visible, with fewer the screen is
  scaled down (in Terminal.app it is worth making the font much smaller). It
  is also slower: when almost the whole screen changes, 50 frames per second
  cannot be kept. `--halfblock` forces half blocks.
- **Keyboard**: with the Kitty keyboard protocol the press and release of
  every key arrive, modifiers included, as in the window. Without it, the
  terminal sends only characters: each key stays pressed for 150 ms, and keys
  held down in games and the joystick do not work well.

The status line, below the screen, shows on the left the main keys and the
emulator's messages (saves, joystick port, turbo), on the right the text
version of the window's status bar: Datasette button and counter, drive LED
and track, joystick port, paddles (`P1`) and mouse (`M1`), speed
(`▶ 123 ⟳ · 8:● 18.0 · J2 · 100% 50fps`); on exit the messages are
printed again in the terminal. F10
does nothing: fullscreen belongs to the terminal. Pasted text is
typed into the C64.

## Claude: the MCP server `c64mcp`

`c64mcp` connects the emulator to Claude (Claude Desktop, Claude Code or any
other MCP client): you ask for a program in BASIC or assembly, and Claude
writes it, runs it on the emulator, looks at the screen and fixes it, while
you watch it in the window. It is an MCP server over stdio, written without
MCP or JSON libraries: the client starts it and talks JSON-RPC 2.0 with it,
one message per line on stdin and stdout.

```
Claude Desktop ──stdio, JSON-RPC──▶ c64mcp ──Unix socket──▶ c64 --remote
                                   (petcat, 64tass)       (remote monitor)
```

It drives the emulator through its [remote monitor](DEBUGGER.md#remote-monitor):
if no emulator is listening, the first tool call starts `c64 --remote` from
the same directory as `c64mcp`, and the emulator keeps running when Claude
stops the server. To let Claude drive an emulator you started yourself, start
it with `--remote`. BASIC programs are tokenized by `petcat`, which comes with
VICE, assembly is assembled by `64tass`: both are external programs, looked
up in the `PATH` and in Homebrew's directory.

```bash
brew install vice tass64        # petcat and 64tass
cargo build --release
```

Claude Desktop: in `~/Library/Application Support/Claude/claude_desktop_config.json`
(Settings → Developer → Edit Config), then restart Claude Desktop:

```json
{
  "mcpServers": {
    "c64": { "command": "/path/to/c64/target/release/c64mcp" }
  }
}
```

Claude Code: `claude mcp add c64 -- /path/to/c64/target/release/c64mcp`.

Options of `c64mcp`: `--socket PATH` (another socket), `--emulator PATH` (the
program to start, for example `c64dbg`) and, after `--`, the options for the
emulator it starts (`-- --no-drive --sid 8580`). Its messages go to stderr,
which Claude Desktop keeps in its logs (`mcp-server-c64.log`).

The tools Claude sees:

| Tool | What it does |
|---|---|
| `run_basic` | resets the C64, tokenizes the BASIC program with petcat, loads it at `$0801`, types `RUN`; returns the screen as text and a screenshot. The program is written in uppercase as on the C64 (the case is swapped for petcat), with petcat's codes such as `{clr}` in strings |
| `run_asm` | resets the C64, assembles with 64tass, loads the program and starts it (`RUN` for a program at `$0801` with a BASIC SYS line, otherwise `SYS` to its first byte); returns the assembler's errors, or screen, screenshot and labels |
| `screenshot`, `screen_text` | the screen as an image (PNG) or as 40×25 text |
| `type_text` | types on the keyboard, with `{return}`, `{f1}`, `{runstop}`... |
| `joystick` | holds directions and fire for a number of frames |
| `wait` | lets a number of frames pass, then returns the screen |
| `reset` | reset, up to READY |
| `monitor` | any debugger command (`regs`, `mem`, `break`, `until`, `step`, `sprites`...) |

The tools that run the emulation last at most 3000 frames (one minute). A
breakpoint or a `step` from `monitor` leaves the machine paused, with
`PAUSED` in the status bar, until `resume`, `run_basic`, `run_asm`, `reset`,
F11, or until Claude's connection closes. `c64mcp` works on macOS and Linux
(the remote monitor uses Unix sockets).

## Keys

| Key | Function |
|---|---|
| Arrows, left Ctrl / left Alt / Space | joystick (directions, fire) |
| TAB | joystick port: 2 (default) → 1 → none |
| Esc | RUN/STOP |
| Right Ctrl, right Alt | Control, Commodore |
| F1–F8 | C64 function keys |
| F5 / F9 | save / load the full state (`c64_state.sav`) |
| F8 | next medium (side B, disk 2) among the files on the command line; with a single file, disk directory |
| F10 | fullscreen |
| F11 | reset (it also ends a pause of the remote monitor) |
| F12 | quit |
| Click on the status bar | Datasette buttons (RECORD, PLAY, REW, FF, STOP); on the counter: reset it |
| Pointer over the C64 screen, left/right click | with paddles in a control port: position of the knobs, fire of paddle X / Y |
| Click on the C64 screen | with a 1351 mouse in a control port: captures the host mouse for it |
| Cmd (Windows/Super key on Windows and Linux), middle mouse button | releases the captured mouse (so does switching to another window) |

Letters, digits and symbols follow the layout of the operating system and
stay pressed on the C64 for as long as the key is held, at least 3 frames,
as VICE does: games that use letter keys and programs that debounce the
keyboard (GEOS) see them. The auto-repeat of the host is ignored: the C64
repeats by itself. Letters keep their case: a capital (Shift or Caps Lock
on the computer) is SHIFT + letter on the C64, a capital in GEOS and in
lowercase mode, a graphic character in BASIC's uppercase mode, as on the
real keyboard. Pasted text and the debugger's `keys` type letters without
SHIFT instead, so that BASIC commands come out whatever their case. A gamepad, if connected, acts on port 2.

### Paddles and 1351 mouse

`--port1 DEV` and `--port2 DEV` plug a device into a control port:
`joystick` (default), `paddles`, `mouse` (a 1351 in proportional mode) or
`joymouse` (a 1351 in joystick mode, as the older 1350).
They are driven by the host mouse, in the window of `c64` and of
`c64dbg --window` (not in `c64term`).

- **Paddles**: they follow the pointer, which stays visible: its horizontal
  position over the C64 screen (the 320×200 area inside the border) is the
  position of paddle X, from fully left at the left edge to fully right at
  the right edge, the vertical one that of paddle Y. Left click = fire of
  paddle X (joystick left line), right click = fire of paddle Y (right line).
  In Arkanoid, for example, the Vaus follows the pointer from side to side.
- **1351 mouse**: a click on the C64 screen captures the host mouse (the
  pointer disappears and stays in the window); the Cmd key (the Windows/Super
  key on Windows and Linux), the middle button or switching to another
  window releases it. The movement goes to the SID's POT registers as the
  real mouse does (position modulo 64 in bits 1-6, one C64 pixel of
  movement = 2 units, at most 63 units per frame as in VICE); left button =
  fire, right button = up. While it is captured, paddles in the other port
  turn with the relative movement, one step per pixel.
- **1351 in joystick mode** (`joymouse`; on the real mouse, the right
  button held at power-on): captured like the 1351, but the movement goes
  to the joystick lines, as with the 1350 mouse and VICE's digital mouse:
  every C64 pixel of movement pulls the line of that direction low for a
  frame and a half, so a moving mouse holds the direction (and the
  program's joystick acceleration builds up). Left button =
  fire, right button = POTX to 0. It is for programs that only read a
  joystick, such as GEOS 1.x without a 1351 driver (in port 1). The lines
  of port 1 are wired to the keyboard rows: a direction pulse while typing
  looks like extra keys (lost or doubled letters in GEOS), as on a real C64.
  So while a key is held on the computer, and for half a second after the
  last one, the host mouse movement is ignored in this mode (releasing the
  mouse with Cmd before typing works too).

As on the C64, CIA1 PA6/PA7 (`$DC00` bits 6-7) choose which port reaches the
SID, and the SID measures every 512 cycles: after switching port the
registers hold the old measurement until the next one, and a switch during a
measurement gives a mixed value. The KERNAL keyboard scan leaves port 1
selected. In the debugger: `port`, `mouse`, `paddle` (see DEBUGGER.md).


### Interlace pictures: `--blend`

Some pictures and demos alternate two frames (interlace, IFLI, color
mixing): on a 50 Hz CRT the eye mixes them into colors the VIC-II does not
have. A 60 or 120 Hz monitor shows the 50 frames per second unevenly, some
twice and some once, so the mix flickers. `--blend` (all frontends; in the
debugger `blend on|off`) shows every frame averaged in linear light with
the previous one, which is what the eye sees on the CRT; screenshots are
blended too. Moving objects leave a half-bright trail, so it is off by
default. `demo/earthrise` is an IFLI picture made for it.

### Window title and status bar

The window title is `SampoSoft C64 Emulator — game.d64`: the application
name and the file in use (the one given on the command line, or the medium F8
switched to), followed by `— loading (turbo)` during the load turbo, or by
`— paused (remote monitor)`. `c64term` sets the same title on the terminal
window.

Below the C64 screen, in the window, there is a status bar like VICE's, at
the same scale as the screen and written with the C64 font (it disappears in
fullscreen):

- first row: the five Datasette buttons in the Datasette's order (the
  pressed one is highlighted, and they can be clicked), the 000-999 counter
  (a click resets it) and the reel, which turns while the tape moves and is
  green with the motor on; then drive 8 with the 1541's red LED (on during
  accesses, blinking on errors), the track under the head (`18.5` for half
  tracks) and the disk spinning with the motor; the two joystick ports with
  the directions and fire pressed (the number in white is the port driven by
  the keyboard), or the paddle and mouse icons with their buttons (the
  mouse is lit while the host mouse is captured), and the gamepad icon if
  there is one; on the right the speed
  (`100% 50fps`, `TURBO 850%` during loads, `PAUSED` when the remote monitor
  stops the machine);
- second row: inserted tape (T), disk (D) and cartridge (C), with `*`
  when there are changes not yet written to the file; on the right the SID
  (`6581x2` with the second one), REU and, with several media on the command
  line, which one is inserted (`F8 2/3`);
- third row: the emulator's latest message for a few seconds (long ones
  scroll), otherwise the key reminders.

Messages used to go only to stderr, invisible from the window. In the
debugger `screenshot file.png bar` saves the screen with the bar below it.

Audio comes out at the device's rate (44.1 or 48 kHz), resampled from the
SID as VICE does; the emulation runs exactly at the PAL clock (985,248 Hz,
one frame every 19.999 ms). `--sid` chooses the chip: `6581` (default, the
original C64), `8580` (C64C) or `8580d` (8580 with the "digiboost" mod, to
hear digitized samples). `--sid2 ADDR` attaches a second SID for stereo
music, like VICE's `-sidextra 1 -sid2address`: at an address between `$D420`
and `$D7E0` or between `$DE00` and `$DFE0`, in steps of `$20` (the most
common are `d420`, `d500` and `de00`), with the same model as the first. The
first SID goes to the left and the second to the right; on a mono device
they are mixed as in VICE.

## Emulation status

Microcycle 6510 CPU with BA/AEC from the VIC, cycle-exact VIC-II 6569
ported from VICE's x64sc core (c/g/sprite accesses on the chip's cycles, the
three BA cycles in which the c-access reads $FF as in FLI, borders, sprites
with DMA, expansion and crunch, and the drawing pipeline: every register
write takes effect on the same pixel as in VICE), open bus (reads of
unconnected addresses, such as empty I/O1/I/O2, the holes of Ultimax mode and
the top nibble of the color RAM, give the byte the VIC read in the first half
of the same cycle, and writes to `$00`/`$01` store it in the RAM under the
processor port, as in VICE), reset as in VICE (F11, or inserting a
cartridge: CIAs, SIDs and processor port as at power-on, so the KERNAL comes
back whatever was banked in; the VIC keeps its registers but restarts from
line 0 with the interrupt mask cleared), processor port as in VICE (input
bits keep the last value driven, bits 6-7 their charge for about 350000
cycles), keyboard matrix solved as in VICE (ghost keys, backwards scanning
with port B selecting and port A reading, joysticks through the keys),
complete CIA 6526 (timers
with start and load latencies, cascading, ICR with its delays, TOD with
alarm, serial port output), SID 6581 and 8580 ported from reSID as VICE
uses it (see below), 1541 drive (see below), cartridges, REU, PRG, full save
state (restored mid-frame it continues identically).

The comparison with VICE 3.10 (x64sc) is the reference check: the boot
matches instruction by instruction, with registers and cycle count, for
2 million instructions and 299 IRQs; regression tests read
SID, CIA and VIC at the cycle (raster, IRQ flags, cycles stolen by bad lines
and sprites, collisions) and give the same bytes as VICE, and two programs
that change the VIC registers on every line one cycle later give the same
screen as VICE, pixel for pixel. The comparison and the tests are described
in [DEBUGGER.md](DEBUGGER.md).

### SID

The SID is a port of reSID 1.0 in the VICE 3.10 version, with the
`filter8580new` filter that VICE uses for both models, clocked with the CPU
clock (writes take effect on the right cycle):

- digital part: oscillators, sync and ring modulation, noise register
  with its latencies and the test bit, combined waveforms read from the
  tables sampled from real chips (`src/sid/samples/`, from reSID), with the
  write-back into the noise register and, on the 6581, the accumulator MSB
  pulled down by the combined sawtooth; ADSR envelopes with the pipeline
  delays and the ADSR bug; output held without a waveform fading out bit by
  bit; reading write-only registers gives the last value on the data bus,
  which decays after a while (about 7400 cycles on the 6581);
- analog part: waveform and envelope DACs with the chips' imperfections
  (the 6581 lacks the R-2R ladder termination and is not monotonic),
  transistor-level filter, mixer and volume on the measured op-amp curve,
  C64 output stage (16 kHz low-pass, 16 Hz high-pass) and resampling with a
  Kaiser FIR filter. On the 6581 the cutoff DAC is not linear and each voice
  reaches the mixer with its DC component: the volume register shifts the
  output level, and digitized samples via `$D418` can be heard.

The analog part is computed only with audio on: without it (debugger,
tests, load turbo) it costs nothing, with audio about 3% of a core. The
filter tables are built at startup (0.2-0.4 s).

Check against VICE: the digital part gives the same bytes read from
OSC3, ENV3 and the write-only registers on 6581 and 8580
; the analog output, compared cycle by cycle with
reSID's on tones, combined waveforms, filters
with cutoff and resonance sweeps, noise, three voices and samples via
`$D418`, differs by 1-4 units out of 65536: it is the noise that reSID adds
to the filter inputs with `rand()` (here a fixed generator). The second SID
gives the same bytes as VICE at `$D420` and at `$DE00` (separate data buses,
mirrors of the first outside its 32 bytes) and the same output. See
[DEBUGGER.md](DEBUGGER.md).

### Cartridges

`.crt` files of the types used by games, verified against VICE with test cartridges: Normal (8K, 16K, Ultimax), Ocean, Fun Play / Power
Play, Super Games, C64 Game System / System 3, Dinamic, Zaxxon, Magic Desk /
Domark / HES Australia, Ross, EasyFlash, RGCD / Hucky, GMod2, Drean, Magic
Desk 16K, Megabyter, Magic Desk Plus. Freezer cartridges (Action Replay,
Retro Replay, Final Cartridge, Super Snapshot…) and utility and language
cartridges are missing; an unsupported type is rejected with its name.

The flash chips of EasyFlash (two Am29F040B), GMod2 (Am29F040) and Megabyter
(MX29F800CB) can be written: programming, sector or chip erase with their
timings and status bits, autoselect, as in VICE. Flash and the EEPROM of
GMod2 and Magic Desk Plus (game saves) are kept in a `.nvram` file next to
the `.crt`, written on exit if something changed; the `.crt` is never
modified, and deleting the `.nvram` restores the original cartridge.
EasyFlash games save through EAPI, the driver contained in the image: it
works if it is the one for the Am29F040 (the normal case); VICE instead
always replaces it with its own.

### REU

`--reu KB` (in `c64` and `c64dbg`, or the debugger's `reu` command) attaches a
RAM Expansion Unit: 128 (1700), 256 (1764), 512 (1750) or 1024-16384 KB
(modified REUs and 1750 XL). The REC controller is complete: C64→REU,
REU→C64, swap and verify transfers, fixed addresses, autoload, immediate
start or start on a write to `$FF00`, end-of-block and verify IRQs, address
wrap-around at 512K (128K on the 1700), the 1764's addresses without DRAM
and the RAM contents at power-on. The DMA stops the CPU and uses the bus
cycle by cycle as in VICE: one byte per cycle (two in a swap), waiting while
the VIC takes the bus for bad lines and sprites, with accesses that go
through I/O (transfers into the VIC or SID registers). The REU contents are
not saved to a file between sessions (they are kept in the save state).

### 1541 drive

The 1541 is emulated as hardware: its 6502 CPU (with decimal mode and the
SO pin connected to BYTE READY), the two 6522 VIAs cycle by cycle, the
mechanics (stepper motor on half tracks, four density zones, rotation)
and the read/write circuit ported from VICE (UE7/UF4 counters at
16 MHz, SYNC detection, BYTE READY), which works on the disk's GCR stream.
With the original DOS, fastloaders, IFFL and protections that talk to the
drive therefore work (Out Run, Last Ninja 2, Shinobi, Altered Beast…). The
serial bus is open-collector: C64 and drive pull ATN, CLK and DATA low, the
drive's ATN ACK circuit is combinational, and a C64 write to `$DD00` reaches
the drive from the next cycle, the delay on which fast 2-bit transfers are
timed. The drive runs at 1 MHz against the PAL C64's 985248 Hz.

D64s (converted to GCR as VICE does, including error bytes and tracks
36-40) and G64s (also with half tracks and GCR-level protections) are
loaded. Opening a D64 or G64 starts `LOAD"*",8,1` and then `RUN`; in the
debugger `drive insert` changes the disk without autoloading. What the drive
writes (SAVE, formatting, game saves) goes back into the file on exit: a D64
with the sectors decoded from GCR, a G64 track by track.

The check against VICE uses programs that run code in the drive and a shared G64 disk: timed GCR reads, SYNC and BYTE READY
read at the cycle, and a fast 2-bit transfer taken from IFFL give the same
bytes as VICE.

### Tape

The 1530 Datasette is emulated as in VICE: the tape pulses arrive on the
CIA1 FLAG line, the pressed buttons are read from bit 4 of `$01` and the
motor, controlled by bit 5, takes 32000 cycles to start and as many to
stop. TAP files (versions 0, 1 and 2) contain the pulses as they were
recorded, so turbo loaders work too. The tape speed wobbles as in VICE
(±0.5%).

All the buttons are there: PLAY, RECORD, STOP, fast forward and rewind (FF
and REW, at the Datasette's speed: the tape speeds up as the reel fills; the
000-999 counter follows the position). There is no need to press them by
hand: when the KERNAL prints PRESS PLAY ON TAPE or PRESS RECORD & PLAY ON
TAPE the emulator presses PLAY or RECORD, as the person in front of the C64
would. While recording, every rising edge of bit 3 of `$01` closes a pulse,
written into the TAP from the current position as VICE does: `SAVE` from
BASIC, and also programs that save with their own routines. A `.tap` file
that does not exist is a blank tape (`c64 new.tap`), without autoloading;
recordings are written to the file on exit, or when the tape is changed
with F8.

Off by default, as in VICE: `--tape-sound` makes the tape audible while it
runs in PLAY (one square wave period per pulse, ±1024 on the SIDs' 16-bit
scale like VICE's default volume, mixed into all channels; each sample is
the average of the wave over its interval). During the load turbo the audio
is off: to hear it, `--no-turbo` is needed. `--tape-azimuth CYCLES`
simulates a misaligned head: each distance between pulses is shifted at
random by up to that many cycles (from 0.001 to 10), with the remainder
carried over to the next pulse, to put loaders to the test. In VICE 3.10
the azimuth error (`-dstapeerror`) does not work: the negative random value
becomes an unsigned integer and every pulse that gets it lasts 4 seconds
longer (the tape does not load); here the shift is signed. The random
generator is VICE's with a fixed initial state, so two runs give the same
errors.

Opening a TAP starts `LOAD` with PLAY pressed, a C= skips the KERNAL's
pause after FOUND, and at the end of the load `RUN` starts; PLAY stays
pressed for games that load further parts. When the tape has been running
without stopping for at least one second the emulator goes to maximum speed
(see the turbo above): a standard load of a few minutes takes a few
seconds, and the same goes for recording and fast winding. The one-second wait excludes motor blips during
gameplay: many games set `$01` to `$00` to read the RAM under the I/O, and
so switch the motor on for a couple of frames (the tape really moves, as on
a real C64). Reset stops the
Datasette and rewinds the tape. A T64 is a file container: its first
program is loaded, like a PRG. In the debugger the `tape` command inserts a
tape, presses the buttons, shows counter and position and adjusts the
wobble.

For games on several tapes or sides, give all the files:
`c64 side_a.tap side_b.tap`. The first one is loaded; when the game asks to
turn the tape over, F8 does what a person would do: it inserts the next
tape rewound and presses PLAY. The same goes for disks
(`c64 disk1.d64 disk2.d64`): F8 changes the disk in the drive, after
saving its changes. After the last medium it starts again from the first.

The check against VICE: a program loaded by the KERNAL
switches the motor back on and records the CIA timer at each pulse of a
sequence of test pulses (long pauses, pulses beyond 16 bits): the same values as
VICE, to the cycle, with and without speed wobble. A program that writes
pulses of known length with RECORD pressed gives the same TAP as VICE byte
for byte; FF to the end and REW to the start take as long as in VICE
(measured with the CIA timers).  As on a real C64, the KERNAL cannot load from tape a
file that ends up under the BASIC ROM (`$A000`-`$BFFF`): in the verify pass
it reads back the ROM instead of the RAM and gives `?LOAD ERROR`.

### What really remains

- SID: at most two chips (VICE 3.10 handles up to eight). Control ports:
  joystick, paddles and 1351 mouse in both modes only (no light pen, Neos
  mouse, Koalapad), and the mouse is not available in `c64term`.
- Freezer and utility cartridges (see above).
- Drive: only one (number 8), no 1571/1581, parallel cables or drive RAM
  expansions; NIB/P64 images not supported.
- Tape: no fine speed adjustment (in VICE it is 0 by default); buttons are
  pressed automatically only on KERNAL messages (otherwise there is the
  debugger's `tape` command); only the first file is loaded from T64s.

## License

Copyright (C) 2026 SampoSoft - Francesco Sampoli.

This program is free software: you can redistribute it and/or modify it under
the terms of the GNU General Public License as published by the Free Software
Foundation, either version 2 of the License, or (at your option) any later
version (see `LICENSE`). It is distributed in the hope that it will be useful,
but WITHOUT ANY WARRANTY. `--version` prints version, copyright and license;
a short notice is printed at startup on the terminal the program is started
from, as VICE does, and before the `c64dbg` prompt.

The VIC-II, the SID, the Datasette and the 1541 disk rotation are ports of
[VICE](https://vice-emu.sourceforge.io/) 3.10 and of reSID by Dag Lem, both
under the same license: see `CREDITS.md` for authors and files. The C64 and
1541 ROMs are copyrighted by Commodore and are not included.
