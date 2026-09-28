// SPDX-License-Identifier: GPL-2.0-or-later
// Copyright (C) 2026 SampoSoft - Francesco Sampoli

//! Runs single routines of ssh64 on the emulator's 6510 over 64 KB of flat
//! RAM and counts their cycles. Driven by the test scripts through stdin, one
//! command per line, one answer line each:
//!
//! ```text
//! load FILE.prg            -> ok
//! w ADDR HEXBYTES          -> ok
//! r ADDR LEN               -> HEXBYTES
//! call ADDR [A X Y]        -> CYCLES A X Y   (cycles of the routine and its RTS)
//! prof ADDR FILE           -> CYCLES         (as call; cycles per PC into FILE)
//! ```
//! Addresses and bytes in hex, LEN in decimal.

use c64::cpu::{Cpu, CpuBus};
use std::io::{self, BufRead, Write};

struct Flat(Box<[u8; 65536]>);

impl CpuBus for Flat {
    fn read(&mut self, addr: u16) -> u8 {
        self.0[addr as usize]
    }
    fn write(&mut self, addr: u16, val: u8) {
        self.0[addr as usize] = val;
    }
}

/// JSR to the routine lives here; the CPU stops when it gets back.
const TRAP: u16 = 0x02F0;
const MAX_CYCLES: u64 = 20_000_000_000;

fn hex(s: &str) -> Result<u32, String> {
    u32::from_str_radix(s, 16).map_err(|_| format!("bad hex {s:?}"))
}

fn bytes(s: &str) -> Result<Vec<u8>, String> {
    if s.len() % 2 != 0 {
        return Err("odd hex length".into());
    }
    (0..s.len()).step_by(2).map(|i| hex(&s[i..i + 2]).map(|b| b as u8)).collect()
}

fn command(mem: &mut Flat, cpu: &mut Cpu, line: &str) -> Result<String, String> {
    let args: Vec<&str> = line.split_whitespace().collect();
    match &args[..] {
        ["load", path] => {
            let data = std::fs::read(path).map_err(|e| format!("{path}: {e}"))?;
            if data.len() < 2 {
                return Err("short file".into());
            }
            let at = u16::from_le_bytes([data[0], data[1]]) as usize;
            let body = &data[2..];
            if at + body.len() > 65536 {
                return Err("file past $FFFF".into());
            }
            mem.0[at..at + body.len()].copy_from_slice(body);
            Ok("ok".into())
        }
        ["w", addr, data] => {
            let at = hex(addr)? as usize;
            let b = bytes(data)?;
            if at + b.len() > 65536 {
                return Err("write past $FFFF".into());
            }
            mem.0[at..at + b.len()].copy_from_slice(&b);
            Ok("ok".into())
        }
        ["r", addr, len] => {
            let at = hex(addr)? as usize;
            let n: usize = len.parse().map_err(|_| "bad length")?;
            if at + n > 65536 {
                return Err("read past $FFFF".into());
            }
            Ok(mem.0[at..at + n].iter().map(|b| format!("{b:02x}")).collect())
        }
        ["call", addr, regs @ ..] => {
            let target = hex(addr)? as u16;
            let [lo, hi] = target.to_le_bytes();
            mem.0[TRAP as usize..TRAP as usize + 3].copy_from_slice(&[0x20, lo, hi]);
            if let [a, x, y] = regs {
                cpu.a = hex(a)? as u8;
                cpu.x = hex(x)? as u8;
                cpu.y = hex(y)? as u8;
            }
            cpu.pc = TRAP;
            cpu.sp = 0xFF;
            let jsr = cpu.step(mem) as u64;
            let start = cpu.total_cycles;
            while cpu.pc != TRAP + 3 {
                cpu.step(mem);
                if cpu.total_cycles - start > MAX_CYCLES {
                    return Err(format!("no return after {MAX_CYCLES} cycles, PC ${:04X}", cpu.pc));
                }
            }
            let _ = jsr;
            Ok(format!("{} {:02x} {:02x} {:02x}", cpu.total_cycles - start, cpu.a, cpu.x, cpu.y))
        }
        ["prof", addr, file] => {
            let target = hex(addr)? as u16;
            let [lo, hi] = target.to_le_bytes();
            mem.0[TRAP as usize..TRAP as usize + 3].copy_from_slice(&[0x20, lo, hi]);
            cpu.pc = TRAP;
            cpu.sp = 0xFF;
            cpu.step(mem);
            let start = cpu.total_cycles;
            let mut per_pc = vec![0u64; 65536];
            while cpu.pc != TRAP + 3 {
                let pc = cpu.pc as usize;
                let before = cpu.total_cycles;
                cpu.step(mem);
                per_pc[pc] += cpu.total_cycles - before;
                if cpu.total_cycles - start > MAX_CYCLES {
                    return Err(format!("no return after {MAX_CYCLES} cycles"));
                }
            }
            let text: String = per_pc
                .iter()
                .enumerate()
                .filter(|(_, c)| **c > 0)
                .map(|(pc, c)| format!("{pc:04x} {c}\n"))
                .collect();
            std::fs::write(file, text).map_err(|e| format!("{file}: {e}"))?;
            Ok(format!("{}", cpu.total_cycles - start))
        }
        _ => Err(format!("unknown command {line:?}")),
    }
}

fn main() {
    let mut mem = Flat(Box::new([0; 65536]));
    let mut cpu = Cpu::new();
    cpu.p.0 = 0x24;
    let stdin = io::stdin();
    let mut out = io::stdout().lock();
    for line in stdin.lock().lines() {
        let Ok(line) = line else { break };
        if line.trim().is_empty() {
            continue;
        }
        let answer = command(&mut mem, &mut cpu, &line).unwrap_or_else(|e| format!("error {e}"));
        let _ = writeln!(out, "{answer}");
        let _ = out.flush();
    }
}
