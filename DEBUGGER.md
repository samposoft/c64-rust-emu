# c64dbg — debugger

`c64dbg` runs the emulator without a window or audio (the SID output can be
recorded to WAV with the `audio` command) and drives it with text commands
read from `-e`, from a `-x` file or from stdin. Every command produces
deterministic output on stdout, meant for pipes, `grep` and scripts. With
`--window` it also shows the screen (see [Window](#window)).

```bash
cargo build --release
./target/release/c64dbg prg/Commando.prg -e "run 200; keys RUN\n; run 100; screen; quit"
printf 'break 0810\nrun\ndis\nquit\n' | ./target/release/c64dbg prg/game.prg
./target/release/c64dbg                       # interactive: license notice, then the dbg> prompt
```

Options: `[file.prg|.d64|.g64|.tap|.t64|.crt] [more .tap/.t64/.d64/.g64...]` (with
several files the first is loaded, `swap` moves to the next ones), `--version`, `--ntsc` (an NTSC C64 instead of PAL), `--vic CHIP` (the VIC-II and its standard: 6569, 6569r1, 8565, 6567, 6567r56a, 8562, 6572; see README.md), `--c64c` (8565 or 8562, 8580, 6526A), `--cia 6526|6526a`, `--roms DIR`, `-x script`, `-e "cmd; cmd"`
(repeatable), `--no-stdin`, `--window`, `--audio`, `--reu KB` (REU from 128 to
16384 KB), `--eth rrnet|tfe[@ADDR]` (Ethernet cartridge on the virtual network, see
README.md), `--eth-forward HOST:C64` (a host TCP port forwarded to the C64), `--no-drive` (no 1541 drive: D64s through the KERNAL trap),
`--sid 6581|8580|8580d` (SID model, default 6581; `8580d` is the 8580 with
digiboost), `--sid2 ADDR` (second SID, e.g. `d420` or `de00`), `--tape-sound` (tape sound),
`--tape-azimuth CYCLES` (Datasette azimuth error), `--port1 DEV`/`--port2 DEV`
(control port devices: `joystick`, `paddles`, `mouse`, `joymouse`), `--blend`
(frame blending, see `blend`), `--crt MONITOR`, `--composite` and `--rf`
(monitor and TV emulation in the window, see `crt`), `--remote`
and `--remote-socket PATH` (commands also from the [remote monitor](#remote-monitor)).
ROMs are looked up in `roms/` next to the executable or in a directory above
it (from `target/release`, the project's), then in `./roms`.

**Number convention**: addresses and values in hex (`$` optional, `#123`
for decimal); counters (frames, instructions, cycles, raster line) in
decimal.

## Commands

| Execution | |
|---|---|
| `run [n]` / `frames n` | runs until a stop, at most n frames (default 50000) |
| `step [n]`, `s` | n instructions; prints the register line of each (only the last if n > 64) |
| `cycles n` | runs at least n machine cycles (including those stolen by the VIC) |
| `next`, `n` | step over: a JSR is run until it returns |
| `until addr` | runs until PC = addr |

| Stop | |
|---|---|
| `break addr`, `delete addr\|all`, `breaks` | breakpoint on PC |
| `watch a [b]`, `watchr`, `watchw`, `unwatch a\|all`, `watches` | watchpoint on CPU read/write in [a,b]; stops after the accessing instruction, reporting address, value and PC |
| `rbreak line\|off` | stop at cycle 0 of the raster line (every frame; 0-311 PAL and PAL-N, 0-262 NTSC, 0-261 old NTSC) |
| `irqbreak on\|off`, `nmibreak on\|off` | stop on the first instruction of every ISR |
| `kilbreak on\|off` | stop on a KIL/JAM opcode (default on) |

| Inspection | |
|---|---|
| `regs`, `r` | VICE-style register line + raster, frame, pending IRQ, `$01` |
| `set a\|x\|y\|sp\|pc\|p val` | changes a register |
| `mem a [b]`, `m` | hex dump (default 128 bytes), reads without side effects |
| `read a [a..]` | reads through the bus, with side effects (e.g. CIA ICR, `$D01E`/`$D01F` are cleared) |
| `poke a v [v..]`, `fill a b v`, `find a b v [v..]` | write (via the bus, with I/O effects) and search |
| `dis [addr] [n]`, `d` | disassembles n instructions (default PC, 16); `*` marks the PC; illegal opcodes in lower case |
| `stack` | stack bytes and plausible return addresses (preceded by a JSR) |
| `screen` | screen RAM as 40×25 text (uses the current VIC bank and `$D018`) |
| `bank` | memory configuration `$01`, cartridge (type, mode, banks, registers), VIC bank, screen/charset/bitmap |
| `vic`, `sprites`, `cia1`, `cia2`, `sid` | decoded registers; `vic` also the chip and its raster, `cia1` and `cia2` also the model, `sid` also model, OSC3, ENV3, value on the data bus and audio |
| `cia [6526\|6526a]` | model of both CIAs: the old 6526 (default) or the 6526A (8521) of the C64C, whose timer interrupts come one cycle earlier; without arguments it shows it |
| `sid 6581\|8580\|8580d` | changes the SID model (`8580d`: with digiboost) |
| `sid2 [addr\|off]` | second SID: state; with an address (`$D420`-`$D7E0`, `$DE00`-`$DFE0`, in steps of `$20`) it attaches it, as just powered on; `off` removes it |
| `blend [on\|off]` | frame blending: the framebuffer (window and screenshots) becomes the average, in linear light, of the last two frames, as the eye sees a 50 Hz CRT; for pictures that alternate two frames (interlace, IFLI). Off by default, `--blend` turns it on |
| `hdr [on\|off]` | HDR output of the CRT emulation in the window: on a display with headroom above white (Apple EDR; the headroom is asked of the system about once a second) the slot mask is shown at its full depth, its stripes brighter than white, instead of half depth; the picture up to white is the same. Off by default, `--hdr` turns it on; where the display has no HDR it stays SDR |
| `crt [off\|SET\|lc\|composite\|rf\|KNOB=N\|comb=on\|off\|bars=on\|off]...` | CRT emulation of the window (`--window`, or the `c64` window through the remote monitor), on the GPU. Sets: for PAL `1084s` (or `1084s-p1`: the Commodore 1084S-P1), `1084s-d1`, `1901`, `cp90` (the Philips CP90 TV); for NTSC `1702`, `1084s` (on an NTSC machine: the NTSC 1084S-P, or `1084s-p`), `kv1311` (the Sony KV-1311CR TV); for PAL-N `cnt4442` (the Sontec CNT-4442 B TV); `tv` the TV of the machine's standard; `1900` (the green monochrome 1900 M) for all. Inputs: `lc` the luma/chroma input (the monitors' default), `composite`, `rf` the TVs' antenna input (their default). Knobs, -100 to 100 with 0 at the centre, only those the set has: `brightness`, `contrast`, `color`, `tint` (NTSC sets), `sharpness` (NTSC 1084S). Switches: `comb=off` (the NTSC 1084S's comb defeat), `bars=off` (no VIC-II jail bars). Several arguments, also comma-separated, apply in order: `crt 1702 composite tint=-20`. An input turns the emulation on if it is off, a set keeps the input and the knobs it has. Without arguments it shows the setting. See "Monitor and TV" in README.md |
| `screenshot file.png [bar \| crt [height]]` | saves the framebuffer (403×284); with `bar` also the window's status bar below it (403×316); with `crt` the screen through the monitor emulation (the one set with `crt`, otherwise the set of the machine's standard: the 1084S-P1 with luma/chroma, the 1702 on NTSC, the Sontec TV on PAL-N), `height` pixels high (284-2272, default 1136) with the pixel aspect of the standard, only the part on the set's screen (a TV overscans); it needs a GPU. With `-` as the file the PNG goes to the output, for the [remote monitor](#remote-monitor) |
| `info` | frames, instructions, cycles, PRG waiting to be injected (`prg_pending`), keys still to type (`typing`), state of file/break/watch/trace, video standard (`video=PAL`, `NTSC`, `old-NTSC` or `PAL-N`) and VIC-II (`vic=6569`...) |

| Input | |
|---|---|
| `keys "text"` | queues characters on the keyboard, one per frame; `\n` = RETURN, `{name}` = a key named as for `key` (`{return}`, `{f1}`, `{runstop}`, `{clr}`, `{left}`...), queued in order with the characters. One pair of quotes around the text is removed |
| `key name down\|up\|press` | single key: `return space runstop f1..f8 home clr del inst left right up down lshift rshift ctrl commodore restore` or a character. `press` holds it for 4 frames |
| `joy1\|joy2 [+\|-]up\|down\|left\|right\|fire ...`, `joy2 none` | joystick state (stays until you change it) |
| `port [1\|2 joystick\|paddles\|mouse\|joymouse]` | control ports: devices, paddle readings, mouse position or last direction pulses, which port the SID POT pins are on (CIA1 PA6/PA7) and `$D419`/`$D41A`; with arguments it plugs a device |
| `mouse dx dy` | moves the host mouse by dx, dy C64 pixels (y downwards): 1351 mouse (2 POT units per pixel, at most 63 per call), 1351 in joystick mode (a direction pulse every pixel) and paddles (one step per pixel, X horizontally, Y vertically) |
| `mouse left\|right down\|up\|press` | mouse button: 1351 left = fire, right = up (in joystick mode POTX to 0); paddles: fire of paddle X (left line) and Y (right line). `press` holds it for 4 frames |
| `paddle 1\|2 x y` | paddle readings of a port, 0-255 (what `$D419`/`$D41A` give after the next measurement) |

| Files and state | |
|---|---|
| `load file` | .prg (queued, injected after boot), .d64/.g64 (autoload), .tap (autoload: LOAD, PLAY, C= after FOUND, RUN), .t64 (autoload as a tape: LOAD, then RUN, or SYS for a first program that is not at `$0801`; the KERNAL tape routines read its programs in order), .crt |
| `swap` | next medium among the command-line files, like F8 in the window: a tape is inserted rewound with PLAY pressed, a disk replaces the one in the drive after saving its changes |
| `tape [play\|record\|stop\|ff\|rew]` | Datasette: button, motor, counter, position on the tape, pulses read (with a T64: its programs, `>` on the one found last); without arguments it shows the state, otherwise it presses the button (RECORD only with a tape; FF and REW move the tape with the motor on, which the KERNAL turns on when it sees a button pressed) |
| `tape insert file\|eject\|save` | `insert` inserts a TAP or a T64 without autoloading it (a missing .tap is a blank tape); `eject` removes it and `save` writes the recordings to the file right away (otherwise on exit or when the tape is changed) |
| `tape rewind\|counter` | rewinds instantly, without REW (a T64 goes back to its first program) / resets the counter |
| `tape auto on\|off` | buttons pressed automatically when the KERNAL asks PRESS PLAY ON TAPE or PRESS RECORD & PLAY ON TAPE (default on) |
| `tape azimuth [cycles \| off]` | azimuth error: each distance between pulses shifts randomly by up to that many cycles (0.001-10), with the remainder carried over to the next one; off by default. In VICE 3.10 `-dstapeerror` is broken (a negative error becomes +4.3 million cycles): here it keeps its sign |
| `tape sound [on [volume] \| off]` | tape sound in PLAY: one period of square wave per pulse, mixed with the SIDs (volume 1-4096, default 1024 as in VICE); off by default. It is heard when audio is on: `--audio` at real speed (not during the `speed auto` turbo) or `audio file.wav` |
| `tape wobble [off \| % Hz]` | tape speed wobble: default ±0.5% at 3 Hz as in VICE (`-dstapewobbleamp`, `-dstapewobblefreq`); the phase advances at every pulse by a step proportional to the tape length, as in VICE |
| `drive [mem a [b]]` | 1541 drive: CPU, track and head position, motor, VIA, serial bus, activity counter and queued DOS job (the load turbo's signals); `mem` reads the drive's memory |
| `drive insert file` | inserts a D64/G64 in the drive without autoloading it |
| `drive g64 file` | saves the disk in the drive as a G64 image |
| `drive trace on file`, `drive trace off` | logs every drive instruction (`.8:pc`, registers, drive cycle) |
| `reu [KB\|off]` | REU registers; with a size it attaches it (RAM as at power-on), `off` removes it |
| `eth [rrnet\|tfe[@addr]\|off]` | Ethernet cartridge (CS8900A): mapping, MAC address set by the program, transmitter and receiver, frame counters, and the virtual network (the C64's address, TCP connections with their host socket and bytes carried, UDP ports, pings, DNS and DHCP counters); `rrnet` or `tfe` attaches it (at `$DE00` or `addr`), `off` removes it; with a Retro Replay inserted, the RR-Net at `$DE00` answers only while its clock port is enabled |
| `eth forward HOSTPORT C64PORT` | forwards TCP port HOSTPORT of the host (127.0.0.1 only) to port C64PORT of the C64, for a server running on it |
| `freeze` | presses the freeze button of the cartridge (Action Replay, Final Cartridge III, Retro Replay), like Shift+F11 in the window: within a frame the cartridge pulls NMI low and takes over; `bank` shows its RAM, whether it is disabled and whether it holds the NMI |
| `savestate file`, `loadstate file`, `reset` | complete state (CPU at the microcycle, RAM, VIC, SID, CIA, cartridge, mounted disk, screen), about 1.2 MB; the ROMs stay the ones loaded, and the network connections of the C64 are closed (the Ethernet chip is in the state, the host's sockets are not) |
| `trace on [file] [from to]`, `trace off` | logs every instruction (register line) with PC in the range; without a file it writes to stdout |
| `audio file.wav [Hz]`, `audio off` | records the output of the SIDs (and the tape sound, if on) from the next frames (default 44100 Hz, or the rate of the audio already on; stereo with the second SID); `off` saves the file |
| `audio raw file` | captures the first SID's filter output every cycle, before the C64 output stage and resampling: 16-bit little endian, like VICE's `-residrawoutput`; saved by `audio off` |
| `speed [auto\|real\|max]` | `auto`: 50 frames/s but maximum while loading from disk and tape (default with `--window`); `real`: always 50 frames/s; `max`: maximum (default headless) |

A PRG with load address `$0801` is BASIC: after boot (about 150 frames, at
maximum speed with `speed auto`, like a load) it must be started with
`keys "RUN\n"`. A machine-language PRG is started with
`SYS` automatically. A D64 or G64 is loaded and started by itself; on exit
the changes the drive made to the disk are written to the file.

## Window

```bash
./target/release/c64dbg --window prg/game.d64          # screen + dbg> prompt
./target/release/c64dbg --window --audio prg/game.d64  # audio too
```

Commands are always given from the terminal; the window shows the screen
and accepts keyboard and joystick like the GUI (arrows + Ctrl/Alt/Space, TAB
switches port, gamepad on port 2).

- `run` goes at real speed, so you can play, and at maximum speed while
  loading from the drive (`speed auto`, the title shows it); `speed real`
  removes the turbo, `speed max` goes back to maximum speed (the window is
  still refreshed every 20 ms, the audio is silent).
- **F12** pauses: `run` stops with `STOP: paused from the window` and the
  prompt returns. **F10** full screen. Closing the window quits.
- When paused the window shows the frame being built: the current frame up
  to the VIC's beam, the previous one below it, as in VICE. The title shows
  PC, raster line, cycle and frame.
- Below the screen is the GUI's status bar (Datasette, drive, joystick,
  speed, media, last message; see the README), with `PAUSED` when emulation
  is stopped. Clicks on the Datasette buttons and on the counter reach the
  C64 with the next frame.
- With paddles in a control port (`--port1`/`--port2`, `port`) they follow
  the pointer over the C64 screen, and the clicks are their fire buttons;
  with a 1351 mouse a click on the screen captures the host mouse, and the
  Cmd key (Windows/Super), the middle button or switching window releases
  it.
- Input from the window makes execution non-deterministic; without
  `--window` nothing changes.

The debugger, which owns the C64, runs on a secondary thread; the window on
the main thread (required by winit on macOS). Towards the window: the latest
frame in a shared buffer (`Arc<Mutex<Vec<u32>>>`, the latest always wins, no
queue) and a wake-up via `EventLoopProxy`. Towards the debugger: input
events on an `mpsc` channel, drained at every frame and after every command,
and an `AtomicBool` for the pause. The code shared with the GUI (keys,
joystick, gamepad, audio, scaled drawing) is in `src/frontend/`.

## Remote monitor

With `--remote` the emulator also takes the debugger's commands from other
programs, over a Unix socket (macOS and Linux), in the spirit of VICE's
remote monitor. It works in `c64`, `c64term` and `c64dbg`; `c64mcp`, the MCP
server for Claude (see the README), drives the emulator this way.

```bash
./target/release/c64 --remote prg/game.d64          # the socket is printed on stderr
./target/release/c64 --remote-socket /tmp/c64.sock  # another socket
nc -U /tmp/c64.sock                                  # by hand: type commands, read the responses
```

The default socket is `c64-remote.sock` in the user's private temporary
directory on macOS (`getconf DARWIN_USER_TEMP_DIR`), in `$XDG_RUNTIME_DIR` on
Linux (or `/tmp/c64-remote-UID.sock`). It is created with permissions 0600,
since the commands write files; one left behind by an emulator that no
longer runs is replaced, one in use is an error. The path must be shorter
than 104 bytes (a limit of Unix sockets).

**Protocol**: one command per line, as at the `dbg>` prompt; every command
gets exactly one response, `OK <n>\n` or `ERR <n>\n` followed by n bytes:
the output of the command (text, or the PNG of `screenshot -`) or the error
message. The commands of a connection run in order, several connections are
served together. `hello` answers with the protocol version (1), the
emulator and the mode: `running` or `debugger`. `quit` is refused: the
client closes the connection instead.

**In `c64` and `c64term`** the machine keeps running at its pace (real speed,
turbo while loading) and the commands are executed between two frames:

- `run [n]`, `frames n`, `cycles n`, `until addr` and `next` answer when they
  stop, while the frames keep coming at the usual speed;
- a stop (breakpoint, watchpoint, `rbreak`, `irqbreak`, `nmibreak`, and a
  KIL opcode while any of these is set) pauses the machine, also when it comes with no command in progress (a message in the
  status bar says why); `step` pauses it; `pause` and `resume` pause and
  restart it by hand. While paused the window shows the frame being built,
  the status bar `PAUSED` and the title `paused (remote monitor)`; a command
  that runs the emulation (`frames 10`) runs it and pauses it again at the
  end;
- the pause ends with `resume`, with a reset (`reset`, F11) and when the
  connection that asked for it (with `pause` or `step`) closes;
- the stops are checked after every instruction only while there is one to
  check, a trace or a command in progress other than `frames`: otherwise the
  frames run at full speed as usual;
- `speed` is refused (the frontend sets the speed); `key ... press` and
  `mouse ... press` run their 6 frames at once, between two frames.

**In `c64dbg`** the commands from the socket are executed like those from
stdin, in the order they arrive, and the machine runs only during the
commands that run it, as always; `pause` and `resume` do nothing. With
`--remote`, the end of stdin (or `--no-stdin`) no longer ends the session.

## Register line format

```
.C:e5cd  A5 C6       LDA $C6        - A:00 X:00 Y:0A SP:f6 ..-..IZ.  3364019
```

It is the same as the VICE monitor's (PC, bytes, disassembly, registers,
flags `NV-BDIZC`, total cycles), so a trace of ours and one from VICE can be
compared line by line.

## Implementation notes

- The VIC (`src/vic/mod.rs`) is VICE's x64sc core ported 1:1
  (vicii-cycle.c, vicii-fetch.c, vicii-draw-cycle.c, vicii-mem.c).
  `vic::cycle` does what `vicii_cycle()` does: it completes the phi2
  accesses of the cycle just ended (sprite DMA 0 and 2, after the CPU
  access), moves to the next cycle, does the phi1 access (g-access, sprite
  pointer and DMA 1), checks the border, draws 8 pixels, then raster and
  raster IRQ (every cycle, also when writing $D012), vertical border, sprite
  logic (MCBASE at cycle 16, DMA at cycles 55 and 56, expansion at 56,
  display and MC at 58), bad line, VC/RC and BA; finally the c-access in
  phi2. A cycle table says what happens in each one. Line 0 starts at
  cycle 2 (line 311 lasts one cycle more, as in VICE). In the first 3 BA
  cycles the bus still belongs to the CPU: the c-access reads $FF and the
  color from RAM[PC] (the FLI first-columns artifact), the sprite DMA the
  byte on the internal bus. Writes to $D017 at cycle 15 do the MC crunch.
- Drawing is VICE's pipeline: graphics come out two cycles after the
  g-access (XSCROLL and mode latched with their delays, the mode bits
  change at pixels 4 and 6), sprites start when the previous cycle's X
  matches the one latched the cycle before, stop during their own DMA, the
  main border uses the state of the previous cycle (the checks are at
  cycles 17/18 on the left and 56/57 on the right, before the CPU access)
  and colors are codes resolved one cycle after drawing. So every write
  takes effect at the same pixel as in VICE: $D016 at cycles 16 and 56 opens
  the side borders, $D021 at cycle W changes color from X = 8W - 111. Pixel
  i drawn at cycle k has X = 8k - 120 + i and goes into the framebuffer at
  X + 17. Fast paths with the same result: all-border cycles without
  sprites and with the graphics pipeline empty, sprites when none is active
  or starting, mode unchanged within the cycle.
- Complete 6526 CIAs: timer B cascaded on A's underflow, timer outputs on
  PB6/PB7 (toggle or pulse), TOD in BCD at 50 Hz with 50/60 Hz prescaler,
  latch on reading the hours, stop on writing the hours and alarm (ICR
  bit 2); serial port output (ICR bit 3 after 8 bits, one bit every two
  underflows of A). CNT is considered fixed high (nothing connected).
- Microcycle CPU (`src/cpu/mod.rs`): every machine cycle performs a single
  bus access following the 6502 sequences (dummy reads, double write in
  RMW instructions, push/pull in the right cycles). The VIC cycle gives BA
  and AEC (`vic::Tick`): with BA low the CPU completes only writes, with AEC
  low (VIC phi2 access after the 3 warning cycles) it is halted. Bad line:
  BA from cycle 12, accesses from 15 to 54 (40-43 cycles stolen depending
  on the instruction). Sprite DMA: 3 warning cycles plus 2 access cycles
  per active sprite (cycles 55-62 and 0-10). The RASTER counter advances at
  cycle 1 as in VICE.
  Interrupts, as in VICE: CIA timers start 3 cycles after the CRA write (2
  for a forced load) and underflow in the cycle in which they reach 0; the
  CIA raises ICR bit 7 and the IRQ/NMI line the cycle after the flag. The
  CPU counts the cycles executed with the line active and serves the
  interrupt at the instruction boundary if there are at least 2 (3 after a
  taken branch) and if the I flag was 0 at the start of the last cycle:
  CLI, SEI, PLP and RTI behave as on the chip. Cycles stolen by the VIC do
  not count, but an interrupt arriving during a stall makes it count as one
  cycle; the decision is taken before a stall on the fetch.
- Zero-cost hooks when inactive: watchpoints in the `Bus` (checked only if
  the list is not empty), `DebugHooks::raster_break` in the VIC tick,
  `Cpu::last_interrupt` for the stops on IRQ/NMI.
- `Bus::peek()` reads like the CPU but without side effects (the CIAs' ICR
  is not cleared): it is what dump and disassembler use.
- The disassembler (`src/disasm.rs`) is generated by a script that is not part of this repository.
- The PNG is written without dependencies (`src/png.rs`, "stored" deflate).
