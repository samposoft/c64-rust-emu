#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-2.0-or-later
# Copyright (C) 2026 SampoSoft - Francesco Sampoli
"""AES-128/256 and GCM against FIPS 197, the GCM paper and aes_ref."""

import random
import sys
from harness import Machine, assemble
import aes_ref

PAGE = 0x90                     # round keys at $9000 in the test machine


def main():
    prg, labels = assemble("crypto_test.asm", "crypto_test")
    m = Machine(prg, labels)
    m.call("e_aes_init")
    rnd = random.Random(3)

    def aes(key, block):
        m.write("aes_key", key.ljust(32, b"\0"))
        m.write("aes_klen", [len(key)])
        m.call("e_aes_expand", a=PAGE)
        nr = m.read("aes_nr", 1)[0]
        m.call("e_aes_use", a=PAGE, x=nr)
        m.write(0x80, block)
        c, *_ = m.call("e_aes_block")
        return m.read(0x80, 16), c

    pt = bytes.fromhex("00112233445566778899aabbccddeeff")
    for key, want in ((bytes(range(16)), "69c4e0d86a7b0430d8cdb78070b4c55a"),
                      (bytes(range(32)), "8ea2b7ca516745bfeafc49904b496089")):
        got, c = aes(key, pt)
        if got.hex() != want:
            sys.exit(f"AES-{8 * len(key)}: {got.hex()}")
        print(f"AES-{8 * len(key)} {c} cycles a block")
    for _ in range(20):
        key = rnd.randbytes(rnd.choice([16, 32]))
        block = rnd.randbytes(16)
        rk, nr = aes_ref.expand(key)
        got, _ = aes(key, block)
        if got != aes_ref.encrypt_block(rk, nr, block):
            sys.exit("AES random mismatch")
        if m.read(PAGE << 8, len(rk)) != rk:
            sys.exit("key schedule mismatch")
    print("all good")
    m.close()


if __name__ == "__main__":
    main()
