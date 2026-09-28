#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-2.0-or-later
# Copyright (C) 2026 SampoSoft - Francesco Sampoli
"""Field arithmetic mod 2^255-19 and X25519 against Python and RFC 7748."""

import random
import sys
from harness import Machine, assemble

P = 2**255 - 19
PAL = 985248


def le(n):
    return n.to_bytes(32, "little")


def num(b):
    return int.from_bytes(b, "little")


def x25519_ref(k, u):
    k = bytearray(k)
    k[0] &= 248
    k[31] &= 127
    k[31] |= 64
    k = num(k)
    x1 = num(u) & (2**255 - 1)
    x2, z2, x3, z3, swap = 1, 0, x1, 1, 0
    for t in reversed(range(255)):
        kt = (k >> t) & 1
        swap ^= kt
        if swap:
            x2, x3, z2, z3 = x3, x2, z3, z2
        swap = kt
        a = x2 + z2; aa = a * a; b = x2 - z2; bb = b * b; e = aa - bb
        c = x3 + z3; d = x3 - z3; da = d * a; cb = c * b
        x3 = (da + cb) ** 2 % P
        z3 = x1 * (da - cb) ** 2 % P
        x2 = aa * bb % P
        z2 = e * (aa + 121665 * e) % P
    if swap:
        x2, x3, z2, z3 = x3, x2, z3, z2
    return le(x2 * pow(z2, P - 2, P) % P)


def main():
    prg, labels = assemble("crypto_test.asm", "crypto_test")
    m = Machine(prg, labels)
    m.call("e_build_tables")
    rnd = random.Random(1)
    edge = [0, 1, 2, 19, 38, P - 1, P, P + 1, 2**255, 2**256 - 1, 2**256 - 38,
            2**256 - 19, 2**255 - 1, 2**128, 2**256 - 2**128]
    values = edge + [rnd.getrandbits(256) for _ in range(40)] + \
        [int.from_bytes(bytes(rnd.choice([0, 255]) for _ in range(32)), "little")
         for _ in range(20)]
    cycles = {}

    def check(name, entry, fn, a, b=0):
        m.write("t_a", le(a))
        m.write("t_b", le(b))
        c, *_ = m.call(entry)
        cycles.setdefault(name, []).append(c)
        got = num(m.read("t_d", 32))
        want = fn(a, b) % P
        if got % P != want:
            sys.exit(f"{name}({a:#x}, {b:#x}) = {got:#x}, want {want:#x}")

    for a in values:
        for b in rnd.sample(values, 8):
            check("mul", "t_mul", lambda x, y: x * y, a, b)
            check("add", "t_add", lambda x, y: x + y, a, b)
            check("sub", "t_sub", lambda x, y: x - y, a, b)
        check("sqr", "t_sqr", lambda x, y: x * x, a)
        check("mul121665", "t_m24", lambda x, y: x * 121665, a)
        m.write("t_d", le(a))
        m.call("t_freeze")
        got = num(m.read("t_d", 32))
        if got != a % P:
            sys.exit(f"freeze({a:#x}) = {got:#x}")
    for name, cs in cycles.items():
        print(f"{name:10} {min(cs):6} .. {max(cs):6} cycles")

    inv_cycles = {}
    for mask in (0, rnd.getrandbits(256)):
        m.write("fe_mask", le(mask))
        for a in values + [rnd.getrandbits(255) for _ in range(100)]:
            m.write("t_a", le(a))
            c, *_ = m.call("t_inv")
            inv_cycles.setdefault(mask != 0, []).append(c)
            got = num(m.read("t_d", 32))
            if got != pow(a, P - 2, P):
                sys.exit(f"inv({a:#x}) = {got:#x} (mask {mask:#x})")
    for masked, cs in inv_cycles.items():
        print(f"invert     {min(cs):6} .. {max(cs):6} cycles" + (" (masked)" if masked else ""))
    m.write("fe_mask", le(rnd.getrandbits(256)))

    # RFC 7748 section 5.2 and 6.1
    vectors = [
        ("a546e36bf0527c9d3b16154b82465edd62144c0ac1fc5a18506a2244ba449ac4",
         "e6db6867583030db3594c1a424b15f7c726624ec26b3353b10a903a6d0ab1c4c",
         "c3da55379de9c6908e94ea4df28d084f32eccf03491c71f754b4075577a28552"),
        ("4b66e9d4d1b4673c5ad22691957d6af5c11b6421e0ea01d42ca4169e7918ba0d",
         "e5210f12786811d3f4b7959d0538ae2c31dbe7106fc03c3efc4cd549c715a493",
         "95cbde9476e8907d7aade45cb4b873f88b595a68799fa152e6f8f7647aac7957"),
        ("77076d0a7318a57d3c16c17251b26645df4c2f87ebc0992ab177fba51db92c2a",
         "0900000000000000000000000000000000000000000000000000000000000000",
         "8520f0098930a754748b7ddcb43ef75a0dbf3a0d26381af4eba4a98eaa9b4e6a"),
        ("5dab087e624a8a4b79e17f8b83800ee66f3bb1292618b6fd1c2f8b27ff88e0eb",
         "0900000000000000000000000000000000000000000000000000000000000000",
         "de9edb7d7b7dc1b4d35b61c2ece435373f8343c85b78674dadfc7e146f882b4f"),
        ("77076d0a7318a57d3c16c17251b26645df4c2f87ebc0992ab177fba51db92c2a",
         "de9edb7d7b7dc1b4d35b61c2ece435373f8343c85b78674dadfc7e146f882b4f",
         "4a5d9d5ba4ce2de1728e3bf480350f25e07e21c947d19e3376f09b3c1e161742"),
    ]
    for k, u, want in vectors:
        m.write("x25519_k", bytes.fromhex(k))
        m.write("x25519_u", bytes.fromhex(u))
        c, *_ = m.call("e_x25519")
        got = m.read("x25519_out", 32).hex()
        if got != want:
            sys.exit(f"x25519: {got}, want {want}")
    print(f"x25519     {c:6} cycles = {c / PAL:.1f} s on a PAL C64")
    for _ in range(2):
        k, u = rnd.randbytes(32), rnd.randbytes(32)
        m.write("x25519_k", k)
        m.write("x25519_u", u)
        m.call("e_x25519")
        if m.read("x25519_out", 32) != x25519_ref(k, u):
            sys.exit("x25519 random mismatch")
    cs = []
    for _ in range(4):
        k = rnd.randbytes(32)
        m.write("x25519_k", k)
        c, *_ = m.call("e_x25519_base")
        cs.append(c)
        if m.read("x25519_out", 32) != x25519_ref(k, bytes([9]) + bytes(31)):
            sys.exit("x25519_base (comb) mismatch")
    print(f"x25519_base {max(cs)} cycles = {max(cs) / PAL:.1f} s (public key, comb)")
    print("all good")
    m.close()


if __name__ == "__main__":
    main()
