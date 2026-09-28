#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-2.0-or-later
# Copyright (C) 2026 SampoSoft - Francesco Sampoli
"""SSH packets through AES-GCM (RFC 5647) against aes_ref."""

import random
import sys
from harness import Machine, assemble
import aes_ref

PKT = 0x3000


def main():
    prg, labels = assemble("crypto_test.asm", "crypto_test")
    m = Machine(prg, labels)
    m.call("e_aes_init")
    rnd = random.Random(5)

    for klen in (16, 32):
        keys = [rnd.randbytes(klen) for _ in range(2)]
        ivs = [rnd.randbytes(12) for _ in range(2)]
        ivs[1] = ivs[1][:4] + b"\xff" * 7 + b"\xfe"          # counter carries
        for d in (0, 1):
            m.write("aes_key", keys[d].ljust(32, b"\0"))
            m.write("aes_klen", [klen])
            m.write("gcm_iv_in", ivs[d])
            c, *_ = m.call("e_gcm_setup", x=d)
        print(f"AES-{8 * klen}-GCM setup {c} cycles")
        for n in range(8):
            for d in (0, 1):
                body = rnd.randbytes(16 * rnd.choice([1, 2, 3, 7, 30]))
                length = len(body).to_bytes(4, "big")
                iv = ivs[d]
                ct, tag = aes_ref.gcm_encrypt(keys[d], iv, length, body)
                ivs[d] = iv[:4] + ((int.from_bytes(iv[4:], "big") + 1) % 2**64).to_bytes(8, "big")
                m.write("gcm_p", PKT.to_bytes(2, "little") + len(body).to_bytes(2, "little"))
                if d == 0:
                    m.write(PKT, length + body)
                    c, *_ = m.call("e_gcm_seal", x=0)
                    got = m.read(PKT + 4, len(body) + 16)
                    if got != ct + tag:
                        sys.exit(f"seal {len(body)}: wrong")
                else:
                    bad = n == 5
                    m.write(PKT, length + ct + (bytes([tag[0] ^ 1]) + tag[1:] if bad else tag))
                    c, a, *_ = m.call("t_open")
                    if m.read(PKT + 4, len(body)) != body:
                        sys.exit(f"open {len(body)}: wrong plaintext")
                    if a != bad:
                        sys.exit(f"open {len(body)}: tag {'accepted' if bad else 'rejected'}")
                if n == 7:
                    print(f"  {'seal' if d == 0 else 'open'} {len(body)} bytes: {c} cycles, "
                          f"{c // (len(body) // 16)} a block")
    print("all good")
    m.close()


if __name__ == "__main__":
    main()
