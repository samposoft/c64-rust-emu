#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-2.0-or-later
# Copyright (C) 2026 SampoSoft - Francesco Sampoli
"""SHA-256 against hashlib, with data fed in pieces."""

import hashlib
import random
import sys
from harness import Machine, assemble

DATA = 0x0400


def main():
    prg, labels = assemble("crypto_test.asm", "crypto_test")
    m = Machine(prg, labels)
    rnd = random.Random(7)
    lengths = [0, 1, 3, 55, 56, 57, 63, 64, 65, 119, 120, 128, 200, 1000]
    blocks = cycles = 0
    for n in lengths:
        data = rnd.randbytes(n)
        m.call("e_sha256_init")
        at = 0
        while at < n:
            k = min(n - at, rnd.randrange(1, 300))
            m.write(DATA, data[at:at + k])
            m.write("sha_p", DATA.to_bytes(2, "little") + k.to_bytes(2, "little"))
            c, *_ = m.call("e_sha256_update")
            cycles += c
            at += k
        c, *_ = m.call("e_sha256_final")
        cycles += c
        blocks += (n + 9 + 63) // 64
        got = m.read("sha256_out", 32)
        if got != hashlib.sha256(data).digest():
            sys.exit(f"sha256 of {n} bytes: {got.hex()}")
    print(f"sha256: {cycles // blocks} cycles per block (with the copying)")
    blocks = cycles = 0
    for n in lengths + [300]:
        data = rnd.randbytes(n)
        m.call("e_sha512_init")
        at = 0
        while at < n:
            k = min(n - at, rnd.randrange(1, 300))
            m.write(DATA, data[at:at + k])
            m.write("sha_p", DATA.to_bytes(2, "little") + k.to_bytes(2, "little"))
            c, *_ = m.call("e_sha512_update")
            cycles += c
            at += k
        c, *_ = m.call("e_sha512_final")
        cycles += c
        blocks += (n + 17 + 127) // 128
        got = m.read("sha512_out", 64)
        if got != hashlib.sha512(data).digest():
            sys.exit(f"sha512 of {n} bytes: {got.hex()}")
    print(f"sha512: {cycles // blocks} cycles per block")
    print("all good")
    m.close()


if __name__ == "__main__":
    main()
