# SPDX-License-Identifier: GPL-2.0-or-later
# Copyright (C) 2026 SampoSoft - Francesco Sampoli
"""Drives harness/ (the emulator's 6510 on flat RAM) from Python tests."""

import os
import re
import subprocess

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.dirname(HERE)
BUILD = os.path.join(ROOT, "build")


def assemble(source, name):
    """Assembles test/SOURCE into build/NAME.prg; returns its labels."""
    os.makedirs(BUILD, exist_ok=True)
    prg = os.path.join(BUILD, name + ".prg")
    lbl = os.path.join(BUILD, name + ".lbl")
    subprocess.run(["64tass", "-C", "-a", "-q", "--long-branch", "-o", prg, "-l", lbl,
                    "-L", os.path.join(BUILD, name + ".lst"),
                    os.path.join(HERE, source)], check=True)
    labels = {}
    for line in open(lbl):
        m = re.match(r"(\w+)\s*=\s*\$([0-9a-fA-F]+)\s*$", line)
        if m:
            labels[m.group(1)] = int(m.group(2), 16)
    return prg, labels


def harness_binary():
    subprocess.run(["cargo", "build", "--release", "-q"],
                   cwd=os.path.join(ROOT, "harness"), check=True)
    return os.path.join(ROOT, "harness", "target", "release", "ssh64-harness")


class Machine:
    def __init__(self, prg, labels):
        self.labels = labels
        self.p = subprocess.Popen([harness_binary()], stdin=subprocess.PIPE,
                                  stdout=subprocess.PIPE, text=True)
        self.cmd(f"load {prg}")

    def cmd(self, line):
        self.p.stdin.write(line + "\n")
        self.p.stdin.flush()
        answer = self.p.stdout.readline().strip()
        if answer.startswith("error"):
            raise RuntimeError(f"{line!r}: {answer}")
        return answer

    def addr(self, where):
        return self.labels[where] if isinstance(where, str) else where

    def write(self, where, data):
        self.cmd(f"w {self.addr(where):x} {bytes(data).hex()}")

    def read(self, where, n):
        return bytes.fromhex(self.cmd(f"r {self.addr(where):x} {n}"))

    def call(self, where, a=0, x=0, y=0):
        """Runs a routine; returns (cycles, a, x, y)."""
        c, ra, rx, ry = self.cmd(f"call {self.addr(where):x} {a:x} {x:x} {y:x}").split()
        return int(c), int(ra, 16), int(rx, 16), int(ry, 16)

    def close(self):
        self.p.stdin.close()
        self.p.wait()
