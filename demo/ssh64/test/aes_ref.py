# SPDX-License-Identifier: GPL-2.0-or-later
# Copyright (C) 2026 SampoSoft - Francesco Sampoli
"""AES encryption and GCM in plain Python, reference for the tests."""


def _xtime(a):
    return ((a << 1) ^ (0x1b if a & 0x80 else 0)) & 0xff


def _gmul(a, b):
    r = 0
    while b:
        if b & 1:
            r ^= a
        a = _xtime(a)
        b >>= 1
    return r


_INV = [0] + [next(y for y in range(1, 256) if _gmul(x, y) == 1) for x in range(1, 256)]
SBOX = []
for _x in range(256):
    _b = _INV[_x]
    _s = _b
    for _k in range(1, 5):
        _s ^= ((_b << _k) | (_b >> (8 - _k))) & 0xff
    SBOX.append(_s ^ 0x63)


def expand(key):
    nk = len(key) // 4
    nr = nk + 6
    w = [list(key[4 * i:4 * i + 4]) for i in range(nk)]
    rcon = 1
    for i in range(nk, 4 * (nr + 1)):
        t = list(w[i - 1])
        if i % nk == 0:
            t = [SBOX[b] for b in t[1:] + t[:1]]
            t[0] ^= rcon
            rcon = _xtime(rcon)
        elif nk > 6 and i % nk == 4:
            t = [SBOX[b] for b in t]
        w.append([a ^ b for a, b in zip(w[i - nk], t)])
    return bytes(b for word in w for b in word), nr


def encrypt_block(rk, nr, block):
    s = [a ^ b for a, b in zip(block, rk[:16])]
    for r in range(1, nr + 1):
        s = [SBOX[b] for b in s]
        s = [s[(i + 4 * (i % 4)) % 16] for i in range(16)]       # ShiftRows
        if r < nr:
            t = []
            for c in range(4):
                a = s[4 * c:4 * c + 4]
                t += [_gmul(a[0], 2) ^ _gmul(a[1], 3) ^ a[2] ^ a[3],
                      a[0] ^ _gmul(a[1], 2) ^ _gmul(a[2], 3) ^ a[3],
                      a[0] ^ a[1] ^ _gmul(a[2], 2) ^ _gmul(a[3], 3),
                      _gmul(a[0], 3) ^ a[1] ^ a[2] ^ _gmul(a[3], 2)]
            s = t
        s = [a ^ b for a, b in zip(s, rk[16 * r:16 * r + 16])]
    return bytes(s)


def _gf_mul(x, y):
    """GF(2^128) product in GCM bit order (x, y as 128-bit integers, byte 0 high)."""
    z = 0
    v = y
    for i in range(128):
        if x >> (127 - i) & 1:
            z ^= v
        v = (v >> 1) ^ (0xE1 << 120 if v & 1 else 0)
    return z


def ghash(h, aad, ct):
    h = int.from_bytes(h, "big")
    y = 0

    def blocks(data):
        for i in range(0, len(data), 16):
            yield data[i:i + 16].ljust(16, b"\0")
    for b in list(blocks(aad)) + list(blocks(ct)) + \
            [(8 * len(aad)).to_bytes(8, "big") + (8 * len(ct)).to_bytes(8, "big")]:
        y = _gf_mul(y ^ int.from_bytes(b, "big"), h)
    return y.to_bytes(16, "big")


def gcm_encrypt(key, iv, aad, pt):
    rk, nr = expand(key)
    h = encrypt_block(rk, nr, bytes(16))
    ct = bytearray()
    for i in range(0, len(pt), 16):
        ks = encrypt_block(rk, nr, iv + (2 + i // 16).to_bytes(4, "big"))
        ct += bytes(a ^ b for a, b in zip(pt[i:i + 16], ks))
    s = ghash(h, aad, bytes(ct))
    tag = bytes(a ^ b for a, b in zip(s, encrypt_block(rk, nr, iv + b"\0\0\0\1")))
    return bytes(ct), tag
