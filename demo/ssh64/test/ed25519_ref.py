# SPDX-License-Identifier: GPL-2.0-or-later
# Copyright (C) 2026 SampoSoft - Francesco Sampoli
"""Ed25519 (RFC 8032) in plain Python: reference for the tests and the
generator of the constant tables."""

import hashlib

P = 2**255 - 19
L = 2**252 + 27742317777372353535851937790883648493
D = -121665 * pow(121666, P - 2, P) % P
SQRTM1 = pow(2, (P - 1) // 4, P)


def inv(x):
    return pow(x, P - 2, P)


def recover_x(y, sign):
    xx = (y * y - 1) * inv(D * y * y + 1) % P
    x = pow(xx, (P + 3) // 8, P)
    if (x * x - xx) % P:
        x = x * SQRTM1 % P
    if (x * x - xx) % P:
        return None
    if x == 0 and sign:
        return None
    if x & 1 != sign:
        x = P - x
    return x


B = (recover_x(4 * inv(5) % P, 0), 4 * inv(5) % P)
IDENTITY = (0, 1)


def add(p, q):
    x1, y1 = p
    x2, y2 = q
    t = D * x1 * x2 * y1 * y2 % P
    return ((x1 * y2 + x2 * y1) * inv(1 + t) % P,
            (y1 * y2 + x1 * x2) * inv(1 - t) % P)


def mul(k, p):
    r = IDENTITY
    while k:
        if k & 1:
            r = add(r, p)
        p = add(p, p)
        k >>= 1
    return r


def encode(p):
    x, y = p
    return (y | (x & 1) << 255).to_bytes(32, "little")


def decode(s):
    n = int.from_bytes(s, "little")
    y = n & (2**255 - 1)
    if y >= P:
        return None
    x = recover_x(y, n >> 255)
    return None if x is None else (x, y)


def hint(data):
    return int.from_bytes(hashlib.sha512(data).digest(), "little")


def public_key(secret):
    h = hashlib.sha512(secret).digest()
    a = int.from_bytes(h[:32], "little")
    a &= (1 << 254) - 8
    a |= 1 << 254
    return a, h[32:], encode(mul(a, B))


def sign(secret, msg):
    a, prefix, pub = public_key(secret)
    r = hint(prefix + msg) % L
    rb = encode(mul(r, B))
    s = (r + hint(rb + pub + msg) * a) % L
    return rb + s.to_bytes(32, "little")


def verify(pub, msg, sig):
    a = decode(pub)
    if a is None:
        return False
    s = int.from_bytes(sig[32:], "little")
    if s >= L:
        return False
    h = hint(sig[:32] + pub + msg) % L
    minus_a = ((P - a[0]) % P, a[1])
    return encode(add(mul(s, B), mul(h, minus_a))) == sig[:32]
