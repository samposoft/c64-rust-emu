#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-2.0-or-later
# Copyright (C) 2026 SampoSoft - Francesco Sampoli
"""Ed25519 verification against RFC 8032 and the Python reference."""

import random
import sys
from harness import Machine, assemble
from ed25519_ref import L, sign, public_key, verify

MSG = 0x0400
PAL = 985248

RFC = [  # secret, public, message, signature (RFC 8032, 7.1, tests 1-3)
    ("9d61b19deffd5a60ba844af492ec2cc44449c5697b326919703bac031cae7f60",
     "d75a980182b10ab7d54bfed3c964073a0ee172f3daa62325af021a68f707511a", "",
     "e5564300c360ac729086e2cc806e828a84877f1eb8e5d974d873e06522490155"
     "5fb8821590a33bacc61e39701cf9b46bd25bf5f0595bbe24655141438e7a100b"),
    ("4ccd089b28ff96da9db6c346ec114e0f5b8a319f35aba624da8cf6ed4fb8a6fb",
     "3d4017c3e843895a92b70aa74d1b7ebc9c982ccf2ec4968cc0cd55f12af4660c", "72",
     "92a009a9f0d4cab8720e820b5f642540a2b27b5416503f8fb3762223ebdb69da"
     "085ac1e43e15996e458f3613d0f11d8c387b2eaeb4302aeeb00d291612bb0c00"),
    ("c5aa8df43f9f837bedb7442f31dcb7b166d38535076f094b85ce3a2e0b4458f7",
     "fc51cd8e6218a1a38da47ed00230f0580816ed13ba3303ac5deb911548908025", "af82",
     "6291d657deec24024827e69c3abe01a30ce548a284743a445e3680d7db5ac3ac"
     "18ff9b538d16f290ae67f760984dc6594a7c15e9716ed28dc027beceea1ec40a"),
]


def main():
    prg, labels = assemble("crypto_test.asm", "crypto_test")
    m = Machine(prg, labels)
    m.call("e_build_tables")

    def verify_on_c64(pub, msg, sig):
        m.write("ed_pub", pub)
        m.write("ed_sig", sig)
        m.write(MSG, msg or b"\0")
        m.write("ed_msg", MSG.to_bytes(2, "little") + len(msg).to_bytes(2, "little"))
        c, a, *_ = m.call("t_verify")
        return c, a == 0

    # h mod L on values at the edges of its comparison and subtraction
    rnd0 = random.Random(3)
    cases = [0, 1, L - 1, L, L + 1, 2 * L - 1, 2 * L, 2**512 - 1, 2**252, 2**252 - 1,
             2**253 - 1, L * (2**259) + L - 1, 2**256 * L, (2**252) * 2**8 + 2**248 - 1]
    cases += [rnd0.getrandbits(512) for _ in range(40)]
    cases += [(rnd0.getrandbits(256) * L + rnd0.choice([0, 1, L - 1])) % 2**512 for _ in range(20)]
    for v in cases:
        m.write("ed_h", v.to_bytes(64, "little"))
        m.call("ed_reduce")
        got = int.from_bytes(m.read("ed_h", 32), "little")
        if got != v % L:
            sys.exit(f"{v:#x} mod L = {got:#x}")

    for secret, pub, msg, sig in RFC:
        pub, msg, sig = bytes.fromhex(pub), bytes.fromhex(msg), bytes.fromhex(sig)
        assert public_key(bytes.fromhex(secret))[2] == pub and verify(pub, msg, sig)
        c, ok = verify_on_c64(pub, msg, sig)
        if not ok:
            sys.exit(f"RFC vector rejected: {sig.hex()}")
    print(f"verify {c} cycles = {c / PAL:.1f} s on a PAL C64")

    rnd = random.Random(11)
    for n in range(6):
        secret = rnd.randbytes(32)
        pub = public_key(secret)[2]
        msg = rnd.randbytes(32)          # an SSH exchange hash
        sig = bytearray(sign(secret, msg))
        _, ok = verify_on_c64(pub, msg, bytes(sig))
        if not ok:
            sys.exit("good signature rejected")
        bad = bytearray(sig)
        bad[rnd.randrange(64)] ^= 1 << rnd.randrange(8)
        _, ok = verify_on_c64(pub, msg, bytes(bad))
        if ok != verify(pub, msg, bytes(bad)):
            sys.exit(f"tampered signature: c64 says {ok}")
        _, ok = verify_on_c64(pub, msg[:-1] + bytes([msg[-1] ^ 1]), bytes(sig))
        if ok:
            sys.exit("signature of another message accepted")
    s = int.from_bytes(sig[32:], "little") + L          # S >= L
    _, ok = verify_on_c64(pub, msg, bytes(sig[:32]) + s.to_bytes(32, "little"))
    if ok:
        sys.exit("S >= L accepted")
    # the comb of -A, then the verification through it
    from ed25519_ref import B as BASE, D, P as PRIME, add, mul, decode
    for n in range(3):
        secret = rnd.randbytes(32)
        pub = public_key(secret)[2]
        m.write("ed_pub", pub)
        c, *_ = m.call("e_ed_comb_build")
        a = decode(pub)
        neg = ((PRIME - a[0]) % PRIME, a[1])
        table = m.read("ed_ta", 32 * 96)
        pr = [mul(1 << (43 * r), neg) for r in range(6)]
        for e in range(32):
            p = pr[0]
            for r in range(1, 6):
                x, y = pr[r]
                p = add(p, (x, y) if e >> (r - 1) & 1 else ((PRIME - x) % PRIME, y))
            x, y = p
            want = [(y + x) % PRIME, (y - x) % PRIME, 2 * D * x * y % PRIME]
            got = [int.from_bytes(table[96 * e + 32 * k:96 * e + 32 * k + 32], "little") % PRIME
                   for k in range(3)]
            if got != want:
                sys.exit(f"comb entry {e} wrong")
        if n == 0:
            print(f"comb of -A {c} cycles = {c / PAL:.1f} s")
        for k in range(3):
            msg = rnd.randbytes(32)
            sig = sign(secret, msg)
            if k == 2:
                sig = bytearray(sig)
                sig[rnd.randrange(64)] ^= 4
                sig = bytes(sig)
            m.write("ed_sig", sig)
            m.write(MSG, msg)
            m.write("ed_msg", MSG.to_bytes(2, "little") + (32).to_bytes(2, "little"))
            c, a_, *_ = m.call("t_verify_comb")
            if (a_ == 0) != verify(pub, msg, sig):
                sys.exit("comb verification disagrees")
        print(f"verify with the comb {c} cycles = {c / PAL:.1f} s")
    print("all good")
    m.close()


if __name__ == "__main__":
    main()
