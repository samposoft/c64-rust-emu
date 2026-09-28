# retrocbbs

A PETSCII terminal for the Commodore 64 with an RR-Net compatible Ethernet
cartridge that calls one BBS only: [RetroCampus BBS](https://www.retrocampus.com),
`bbs.retrocampus.com`, port 6510. No questions: it gets an address, looks
the name up, connects, and the BBS's colours and graphics come up as on a
C64 with a modem.

- Network: ssh64's own stack in 6502 assembly (`../ssh64/src/net.asm` and
  `tcp.asm`: DHCP, DNS, ARP, TCP), included as it is
- Screen and keyboard: the KERNAL's, as a real C64 terminal program does.
  Every byte from the BBS goes to CHROUT, the screen editor PETSCII BBSs are
  drawn for (colours, reverse, cursor moves, clear, upper/lower case), and
  every key from GETIN goes to the BBS as it is (F1-F8, cursor keys,
  colours with CTRL and C=). The screen editor is kept out of quote and
  insert mode, so that a `"` does not turn the next control codes into
  characters; BEL rings
- Downloads: XMODEM to drive 8 (F3 or CTRL + D), with CRC-16 or checksum, blocks
  of 128 or 1024 bytes
- Raw TCP, no telnet: the BBS sends PETSCII without any telnet negotiation
- The KERNAL and BASIC stay on: the program returns to BASIC

## Building

64tass, then from this directory:

```bash
64tass -C -a -B -o build/retrocbbs.prg src/retrocbbs.asm
```

It includes `../ssh64/src/net.asm` and `../ssh64/src/tcp.asm`: the two
demos must stay side by side.

A disk to run it from:

```bash
c1541 -format "retrocbbs,64" d64 build/retrocbbs.d64 -write build/retrocbbs.prg retrocbbs
```

Where `retrocbbs.d64` and `retrocbbs.prg` come with the sources, they are
that disk and that program, ready to run: the disk holds the program alone,
the rest is free for the downloads.

## Running on the emulator

```bash
./target/release/c64 --eth rrnet demo/retrocbbs/build/retrocbbs.d64
```

The disk loads and starts the program, and downloads are saved on it
(written back to the `.d64` when the emulator quits). On a real C64,
`LOAD"RETROCBBS",8` and `RUN`, with an RR-Net (or compatible: CS8900A at
`$DE00`, as on a Retro Replay's clock port) on a network with a DHCP
server.

Keys:

| Key | on the emulator | |
| --- | --- | --- |
| C= + RUN/STOP | right Alt + Esc | hang up |
| F3 or CTRL + D | F3 (fn + F3 on a Mac), right Ctrl + D | download a file (XMODEM) |
| RETURN | Return | after the session: call again |
| RUN/STOP | Esc | after the session: back to BASIC; while calling: stop |

RUN/STOP alone goes to the BBS, which uses it to stop a listing. F3 is
CCGMS's download key, which BBSs leave to the terminal; CTRL + D does the
same (on the emulator the C64's CTRL is the right Ctrl, which Mac laptops
lack). On the
emulator, with a joystick port active the arrow keys and the left Ctrl and
Alt are the joystick: TAB twice (no port) gives the cursor keys back.

## Downloading

When the BBS says to start the XMODEM transfer (on RetroCampus: Files, a
release, then any key), press F3 or CTRL + D and type a file name, up to 16
characters. The file is saved as a PRG; a name with its own type, as
`notes,s`, is saved with that type. `@0:name` replaces a file that exists.
The count of bytes saved grows as the blocks come; RUN/STOP stops the
transfer.

The C64 starts the transfer: `C` asks the BBS for blocks with a CRC-16,
and after six unanswered ones, 3 s apart (a BBS may still be getting the
file ready: RetroCampus fetches it from CSDb), a NAK asks for the old
checksum; after 36 s without an answer the C64 gives up. Blocks of
128 (SOH) and 1024 bytes (STX) are both taken. Every good block is
acknowledged at once and written while the BBS sends the next one; a
damaged block is asked for again (NAK), up to 10 errors in a row.

XMODEM sends whole blocks, and pads the last one with `$1A` bytes: the
padding is cut off, so that a PRG keeps its length (a file of its own that
ends with `$1A` bytes loses them). If the transfer fails (stopped,
cancelled by the BBS, too many errors, the connection lost) the C64
cancels it with CAN and scratches the partial file (with `@0:`, the file
it was replacing is gone too). What the drive has to
say (disk full, file exists, no disk) is shown.

Tried against RetroCampus: PIGX by Chillax from CSDb, 2098 bytes, as the
BBS gives its size, and it runs. Also against a test server: 1K and 128-byte
blocks mixed, checksum only, a damaged block, a cancel from the BBS.

## Memory

Code from `$0801`, then the variables and the network's buffers (frames,
the TCP rings, two XMODEM blocks): 7.3 KB of program, everything below
`$A000`. The network uses some of BASIC's zero-page temporaries (`$57-$69`),
saved at the start and put back at the end, and XMODEM the bytes left to
programs (`$FB-$FE`); a tick counter runs in the KERNAL's IRQ, which is put
back too. SID voice 3 runs noise, muted, as a random source for the
network (ports, identifiers, the locally administered MAC address).

Tried on the emulator of this repository, with the BBS on the Internet:
not yet on a real C64 with a real RR-Net.
