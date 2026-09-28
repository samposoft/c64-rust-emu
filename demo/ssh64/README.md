# ssh64

An SSH-2 client for the Commodore 64 with an RR-Net compatible Ethernet
cartridge, written from scratch in 6502 assembly: its own TCP/IP stack and
its own cryptography, nothing from the KERNAL or BASIC once it runs.

- Key exchange: `curve25519-sha256` (RFC 8731), strict key exchange
  (`kex-strict-c-v00@openssh.com`)
- Server host key: `ssh-ed25519`, the signature checked for every connection.
  A new host shows its SHA256 fingerprint and must be accepted; known hosts
  are kept on the disk (`SSH64 HOSTS`), and a host whose key has changed is
  refused
- Cipher: `aes128-gcm@openssh.com`, else `aes256-gcm@openssh.com`
- Login: password, then keyboard-interactive if the server wants that
- One session with a pty: a VT100 terminal of 40 x 25 characters, ANSI
  colours (a coloured background becomes reverse video)
- Network: DHCP, DNS, ARP, TCP (one connection), ICMP echo

Not there: RSA or ECDSA host keys, other ciphers than AES-GCM (a server
without `ssh-ed25519` or AES-GCM is refused), public key login, a new key
exchange during a session, more than one channel (no port forwarding, scp
or sftp), UTF-8 and line drawing characters in the terminal.

## What it needs

A C64 (PAL or NTSC) with an RR-Net compatible cartridge (CS8900A at
`$DE00`, as on a Retro Replay's clock port) and a drive 8 with a disk that
can be written: the known hosts and their combs are kept there.

Tried on the emulator of this repository only: not yet on a real C64 with a
real RR-Net, nor on a C128, nor against a real OpenSSH server (the
development server is AsyncSSH).

## Building

64tass, then from this directory:

```bash
64tass -C -a -B -o build/ssh64.prg src/ssh64.asm
```

`gen_tables.py` writes `src/ed25519_tab.asm` (curve constants),
`src/ed25519_pts.asm` (the comb of the ed25519 base point) and
`src/aes_tab.asm` (AES S-box, GHASH reduction table): run it only
if they change.

A disk to run it from, with room for the known hosts:

```bash
c1541 -format "ssh64,64" d64 build/ssh64.d64 -write build/ssh64.prg ssh64
```

Where `ssh64.d64` and `ssh64.prg` come with the sources, they are that
disk and that program, ready to run: the disk holds the program alone, the
rest is free for the known hosts (it must not be write-protected).

## Running on the emulator

```bash
./target/release/c64 --eth rrnet demo/ssh64/build/ssh64.d64
```

It asks for `host[:port]`, user and password (DHCP runs in the meantime),
then computes its key, connects and checks the server. The router of the
emulator's network, 10.0.2.2, is the host machine itself.

Keys the C64 lacks: SHIFT `:` `[`, SHIFT `;` `]`, C= `:` `{`, C= `;` `}`,
`£` `\`, SHIFT `£` `|`, up arrow `^`, SHIFT up arrow `~`, left arrow `_`,
SHIFT left arrow `` ` ``. CTRL + letter sends the control code, RUN/STOP is
ESC, INST/DEL is DEL, the cursor keys and F1-F8 send VT100 sequences.
C= + RUN/STOP hangs up.

The first connection to a host takes longer: the key pair (21 s), a short
connection that fetches the host's key and shows its fingerprint, then
once it is accepted the comb of the key (77 s) and its saving (13 blocks),
and the connection again, as to a known host (about 115 s to the login).

On the disk: the program (140 blocks), `SSH64 HOSTS` (1 KB at most: about
20 hosts with names of 15 characters) and a `SSH64 Kxxxxxxxx` of 13 blocks
for every host key, about 40 of them on an empty disk.

A host whose key has changed is refused, with a warning: someone may be in
between. If the change is expected (the server was reinstalled), there is
no command to forget one host yet: scratch the whole list from BASIC,
before running the client,

```
OPEN 15,8,15,"S0:SSH64 HOSTS":CLOSE 15
```

and accept the hosts again (their comb files can stay: they are named after
the key, not the host).

On a server of your own, a longer login time leaves room for a slow
network (in `/etc/ssh/sshd_config`, then restart sshd):

```
LoginGraceTime 300
```

## Speed

OpenSSH closes a connection whose login has not ended within 120 s
(`LoginGraceTime`), counted from the TCP connection: everything the C64
computes after that must fit. Times of a stock PAL C64 (1 MHz):

| step | time | in the 120 s |
| --- | --- | --- |
| our key pair (X25519 of the base point: its comb) | 21 s | no, before connecting |
| shared secret (X25519, Montgomery ladder) | 74 s | yes |
| server signature (its host key's comb) | 34 s | yes |
| exchange hash, key derivation, AES-GCM set-up | about 4 s | yes |
| a new host key's comb, once | 77 s | no |

The login is sent as soon as the keys of our direction are ready (the keys
of the server's are made after it: the time limit ends with the login).
Against a development server that closes at 120 s like OpenSSH, the
emulator at the real speed of a C64 logged in after 114.6 s: some margin,
not much on a slow network (a server with a longer `LoginGraceTime` leaves
more). A C128 in C64 mode should run the computations at 2 MHz (its fast
mode, with the screen off), in about half the time: not tried on one. The emulator in turbo
does it all in seconds.

While a long computation runs the screen is off (the VIC then takes no
cycles from the CPU) and the border blinks; the keyboard is not scanned.

The field arithmetic modulo 2^255 - 19, the bulk of it all: products of
bytes from quarter-square tables (four indexed loads, the table bases set
once per column), rows of 8 in three passes without branches (the 8 products,
then low and high bytes summed, then added to the result), Karatsuba on two
levels (32 = 2 x 16 = 4 x 8 bytes): a product about 36 thousand cycles, a
square about 26 thousand. Inversions by the binary extended Euclid (0.3
million cycles, where Fermat takes 7.4), the number first multiplied by a
random one, as its time depends on it. In every step of X25519 one product
is by the server's point u: a table of its 255 multiples (8 KB, in memory
free while X25519 runs, the font among it) makes it 32 rows added up, half
the time.

Both points of a signature check go through signed combs of 6 teeth 43 bits
apart (32 entries of 96 bytes: 42 doublings, 43 additions of each): that of
the base point is a table, that of the host key is made once, when the host
is new, and kept on the disk as `SSH64 Kxxxxxxxx` (the key's first bytes).

A new host takes a first connection that only fetches its key: its
fingerprint is shown, and when it is accepted the host is saved, the comb of
its key made with the connection closed, and the connection made again as
to a known host; the signature is checked then, before the password is
sent. A known host's comb is read from the disk before connecting.

The times are those of a PAL C64 (985 kHz); an NTSC C64 (1023 kHz) should
be about 4% faster.

## Security

- The server is always checked: its signature of the exchange hash,
  through the comb of its known key, before the password is sent; a
  changed key is refused.
- The key pair is new for every connection (and kept for the second one
  after a new host's first); it comes from SHA-256 of a pool stirred with
  the exact moments keys go down and up while the questions are answered
  (the keyboard matrix read continuously, the CIA timers, the raster, the
  SID's noise), not from the 60 Hz scan, whose moments are known.
- Not written to take the same time for every key: the Montgomery ladder
  (its swaps are taken only when needed) and the comb of the base point;
  the keys are used once, and their time is seen from far only, among the
  network's. The inversions, whose time depends most on the number, work
  on it multiplied by a random number.
- The password stays in memory for the connection only, never on the disk.
  The known hosts file holds public keys: not secret, but whoever can write
  the disk can change it.

## How it got here

The first version that worked took about 170 s from the TCP connection to
the login on a C64, too long for OpenSSH's 120 s. The steps, with the times
of a PAL C64 (cycle counts of the test harness):

| change | effect |
| --- | --- |
| inversion by the binary extended Euclid instead of z^(p-2) | 7.4 to 0.3 million cycles: 7 s less in X25519, in the check, in the key pair |
| the last 3 ladder steps of X25519 (zero bits) as doublings | 0.5 s |
| signed combs of 6 teeth (32 entries) for the base point and the host key, in place of 4 teeth (15 entries) and of the sliding windows | check of a known host 51 to 37 s, key pair 33 to 23 s (after Euclid) |
| rows of the 8 x 8 byte blocks in three passes without branches | a product 39.5 to 36.5 thousand cycles |
| squares of 8 bytes read in place, fewer bytes zeroed | a square 2.5% faster |
| mul121665: the row of the byte $01 as a plain sum | 8 to 6.5 thousand cycles |
| Karatsuba differences compared from the top first, no negation | 0.4 s, and 476 bytes of code |
| the table of the multiples of the server's point in X25519 | X25519 78.5 to 74 s |
| h mod L with the remainder in the zero page and L's zero bytes skipped | 0.25 s |
| the key derivation's common block hashed once, a shorter KEXINIT | 0.4 s |
| the login sent before the server's direction's keys are made | 0.5 s |
| screen off, keyboard not scanned during the computations | on a real C64: the cycles the VIC takes (about 5%) and the keyboard scan's |

With the network: a new host's key is only fetched by a first connection
and its comb made with the connection closed, so that only the known
host's path counts; the known host's comb is read before connecting. A
defect found on the way: the TCP window was announced again only after
1 KB was free, a whole ring once the rings were made 1 KB, which could
leave the server waiting for its timer; now at half the ring. Measured on
the emulator at real speed against a development server closing at 120 s:
too late (the key exchange done at 118.6 s, the time up during the
login), then the login at 119.3 s, then at 114.6 s.

Tried and left: a third level of Karatsuba (4 x 4 byte blocks: the
products saved are paid back in sums), the square of 16 bytes without
Karatsuba (1.5% faster, 256 bytes more), Pornin's half-size scalars for
new hosts (no longer needed: new hosts go through the comb too), SHA-256
with rotation tables (3 to 4 KB of tables), the REU or the drive's 6502 as
helpers (not on every C64).

Room was found for all this: the output packet buffer made 640 bytes, the
TCP rings 1 KB, the AES tables moved to `$0900` (no more space lost to their
alignment), the SHA constants moved under the I/O, the one-time set-up code
put under the variables.

## Sources

| file | |
| --- | --- |
| `src/ssh64.asm` | the program: start, questions, connection; the memory map |
| `src/ssh.asm` | SSH-2: packets, key exchange, host key, login, channel, session |
| `src/kh.asm` | known hosts and the combs of their keys on the disk |
| `src/disk.asm` | files on drive 8 through the KERNAL |
| `src/net.asm` | CS8900A driver, ARP, IPv4, ICMP, UDP, DHCP, DNS |
| `src/tcp.asm` | TCP, one connection |
| `src/fe25519.asm` | arithmetic modulo 2^255 - 19 |
| `src/feprog.asm` | field programs over slots, inversion, the power chain of the square roots |
| `src/x25519.asm` | X25519 and the table of the server's point |
| `src/ed25519.asm` | Ed25519 verification, combs, the key pair through the comb of the base point |
| `src/tables.asm` | multiplication tables, built at the start |
| `src/sha256.asm`, `src/sha512.asm` | the hashes |
| `src/aes.asm`, `src/gcm.asm` | AES-128/256 and GCM |
| `src/rng.asm` | randomness |
| `src/term.asm` | VT100 terminal on 40 x 25 cells |
| `src/ui.asm`, `src/fmt.asm`, `src/kbd.asm`, `src/sys.asm` | messages and questions, printing, keyboard, machine set-up and interrupts |
| `src/zp.asm` | the zero page |
| `src/*_tab.asm`, `src/ed25519_pts.asm` | tables written by `gen_tables.py` |
| `src/scr.asm` | a plain screen for the network test |

## Tests

The routines run on the emulator's 6510 over flat RAM (`harness/`, a small
Rust program driven by the Python tests), against RFC 7748, RFC 8032,
FIPS 197, the GCM specification and Python references, with cycle counts:

```bash
cd demo/ssh64/test
python3 test_x25519.py
python3 test_ed25519.py
python3 test_sha.py
python3 test_aes.py
python3 test_gcm.py
```

The harness also profiles a routine: `prof ADDR FILE` writes the cycles
spent at each address.

`test/nettest.asm` is a network test (DHCP, DNS, TCP to an echo server on
port 7777 of the host).

The login against the 120 s limit, on the emulator: an SSH server on the
host with a login time of 120 s (the development one was AsyncSSH, with
`login_timeout=120` and a password of its own), the client run at real
speed with its window in front (the operating system may slow down an
emulator in the background, and the server counts real seconds):

```bash
./target/release/c64dbg --window --eth rrnet demo/ssh64/build/ssh64.d64
```

The server's log then gives the time from the connection to the login.

## Memory

The AES tables at `$0900`, then the code up to about `$9300`; variables up
to about `$9a00` (over the tail of the program: the comb of the base point
and the constants of SHA-256 and SHA-512, copied at the start to the RAM
under the I/O, `$d000`, and the set-up code that runs once before), among
them the comb of the host key; free up to `$a000` (about 1.5 KB);
`$a000-$bfff` holds the multiplication tables and the numbers of the key
exchange, then the GHASH tables of the session (so a new
key exchange during a session is not supported); font at `$c000`, screen at
`$c800`, known hosts at `$cc00`, more variables from `$e000`. While X25519
runs, the table of the multiples of the server's point takes the slots and
variables it does not use, the font and the input packet buffer. The KERNAL
is switched in only to read and write the disk. Network buffers: TCP rings
of 1 KB each (the window announced again at half the ring), frames sent of
at most 640 bytes (segments of at most 536 bytes), packets received of at
most 2560 bytes.
