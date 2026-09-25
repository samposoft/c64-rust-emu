// SPDX-License-Identifier: GPL-2.0-or-later
// Copyright (C) 2026 SampoSoft - Francesco Sampoli

//! Cycle-exact 6510.
//!
//! Each call to [`Cpu::cycle`] performs exactly one bus access (a read or a
//! write), following the 6502 sequences documented in 64doc.txt: dummy reads
//! in indexed addressing, double write in read-modify-write instructions,
//! stack push/pull in the right cycles. The system decides, cycle by cycle,
//! whether the CPU may access the bus (VIC BA/AEC): with BA low the CPU only
//! completes write cycles, with AEC low it is stalled.

mod tests;

use crate::disasm::{Mode, OPCODES};
use crate::mem::Bus;

/// Bus as seen by the CPU: the C64's memory or the drive's.
pub trait CpuBus {
    fn read(&mut self, addr: u16) -> u8;
    fn write(&mut self, addr: u16, val: u8);
    /// First write (of the value read) of a read-modify-write.
    fn write_dummy(&mut self, addr: u16, val: u8) {
        self.write(addr, val);
    }
    /// PC of the instruction about to start (debugger watchpoints).
    fn set_pc(&mut self, _pc: u16) {}
    /// SO pin (on the 1541 the disk's BYTE READY signal): true if an edge not
    /// yet seen by the CPU has arrived, looking at the disk up to `ahead`
    /// cycles past the current one. The edge is consumed.
    fn so_edge(&mut self, _ahead: u8) -> bool {
        false
    }
    /// An instruction cleared V: edges that arrived up to then are lost.
    fn so_discard(&mut self, _ahead: u8) {}
}

impl CpuBus for Bus {
    #[inline(always)]
    fn read(&mut self, addr: u16) -> u8 {
        Bus::read(self, addr)
    }
    #[inline(always)]
    fn write(&mut self, addr: u16, val: u8) {
        Bus::write(self, addr, val)
    }
    #[inline(always)]
    fn write_dummy(&mut self, addr: u16, val: u8) {
        Bus::write_dummy(self, addr, val)
    }
    #[inline(always)]
    fn set_pc(&mut self, pc: u16) {
        self.cpu_pc = pc;
    }
}
use std::sync::OnceLock;

/// Status register (P) flags of the 6502/6510.
#[derive(Clone, Copy)]
pub struct Flags(pub u8);

impl Flags {
    pub const N: u8 = 0b1000_0000;
    pub const V: u8 = 0b0100_0000;
    pub const U: u8 = 0b0010_0000;
    pub const B: u8 = 0b0001_0000;
    pub const D: u8 = 0b0000_1000;
    pub const I: u8 = 0b0000_0100;
    pub const Z: u8 = 0b0000_0010;
    pub const C: u8 = 0b0000_0001;

    pub fn get(self, flag: u8) -> bool { self.0 & flag != 0 }
    pub fn set(&mut self, flag: u8, v: bool) {
        if v { self.0 |= flag } else { self.0 &= !flag }
    }
    pub fn set_nz(&mut self, v: u8) {
        self.set(Self::N, v & 0x80 != 0);
        self.set(Self::Z, v == 0);
    }
}

// ── Decoding ──────────────────────────────────────────────────────────────────

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Op {
    Lda, Ldx, Ldy, Sta, Stx, Sty, Adc, Sbc, And, Ora, Eor, Cmp, Cpx, Cpy, Bit,
    Inc, Dec, Asl, Lsr, Rol, Ror,
    Tax, Tay, Txa, Tya, Tsx, Txs, Inx, Iny, Dex, Dey,
    Clc, Sec, Cli, Sei, Clv, Cld, Sed, Nop,
    Pha, Php, Pla, Plp, Jmp, Jsr, Rts, Rti, Brk,
    Bpl, Bmi, Bvc, Bvs, Bcc, Bcs, Bne, Beq,
    Lax, Sax, Dcp, Isc, Slo, Rla, Sre, Rra, Anc, Alr, Arr, Sbx, Xaa, Ahx, Shx, Shy, Tas, Las, Kil,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Kind { Read, Rmw, Write }

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Idx { X, Y }

/// Cycle sequence of an instruction (after the opcode fetch).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Seq {
    Imp, Imm,
    Zp(Kind), Zpi(Kind, Idx),
    Abs(Kind), Abi(Kind, Idx),
    Izx(Kind), Izy(Kind),
    Rel, Push, Pull, Rts, Rti, Jsr, Brk, JmpAbs, JmpInd,
    /// Interrupt sequence (IRQ or NMI): 7 cycles, no opcode.
    Int,
}

fn op_from_mnemonic(m: &str) -> Op {
    match m {
        "LDA" => Op::Lda, "LDX" => Op::Ldx, "LDY" => Op::Ldy, "STA" => Op::Sta, "STX" => Op::Stx,
        "STY" => Op::Sty, "ADC" => Op::Adc, "SBC" => Op::Sbc, "AND" => Op::And, "ORA" => Op::Ora,
        "EOR" => Op::Eor, "CMP" => Op::Cmp, "CPX" => Op::Cpx, "CPY" => Op::Cpy, "BIT" => Op::Bit,
        "INC" => Op::Inc, "DEC" => Op::Dec, "ASL" => Op::Asl, "LSR" => Op::Lsr, "ROL" => Op::Rol,
        "ROR" => Op::Ror, "TAX" => Op::Tax, "TAY" => Op::Tay, "TXA" => Op::Txa, "TYA" => Op::Tya,
        "TSX" => Op::Tsx, "TXS" => Op::Txs, "INX" => Op::Inx, "INY" => Op::Iny, "DEX" => Op::Dex,
        "DEY" => Op::Dey, "CLC" => Op::Clc, "SEC" => Op::Sec, "CLI" => Op::Cli, "SEI" => Op::Sei,
        "CLV" => Op::Clv, "CLD" => Op::Cld, "SED" => Op::Sed, "NOP" => Op::Nop, "PHA" => Op::Pha,
        "PHP" => Op::Php, "PLA" => Op::Pla, "PLP" => Op::Plp, "JMP" => Op::Jmp, "JSR" => Op::Jsr,
        "RTS" => Op::Rts, "RTI" => Op::Rti, "BRK" => Op::Brk, "BPL" => Op::Bpl, "BMI" => Op::Bmi,
        "BVC" => Op::Bvc, "BVS" => Op::Bvs, "BCC" => Op::Bcc, "BCS" => Op::Bcs, "BNE" => Op::Bne,
        "BEQ" => Op::Beq, "LAX" => Op::Lax, "SAX" => Op::Sax, "DCP" => Op::Dcp, "ISC" => Op::Isc,
        "SLO" => Op::Slo, "RLA" => Op::Rla, "SRE" => Op::Sre, "RRA" => Op::Rra, "ANC" => Op::Anc,
        "ALR" => Op::Alr, "ARR" => Op::Arr, "SBX" => Op::Sbx, "XAA" => Op::Xaa, "AHX" => Op::Ahx,
        "SHX" => Op::Shx, "SHY" => Op::Shy, "TAS" => Op::Tas, "LAS" => Op::Las, "KIL" => Op::Kil,
        _ => Op::Nop,
    }
}

fn kind_of(op: Op) -> Kind {
    match op {
        Op::Sta | Op::Stx | Op::Sty | Op::Sax | Op::Ahx | Op::Shx | Op::Shy | Op::Tas => Kind::Write,
        Op::Inc | Op::Dec | Op::Asl | Op::Lsr | Op::Rol | Op::Ror
        | Op::Dcp | Op::Isc | Op::Slo | Op::Rla | Op::Sre | Op::Rra => Kind::Rmw,
        _ => Kind::Read,
    }
}

fn seq_of(op: Op, mode: Mode) -> Seq {
    let k = kind_of(op);
    match mode {
        Mode::Imp => match op {
            Op::Pha | Op::Php => Seq::Push,
            Op::Pla | Op::Plp => Seq::Pull,
            Op::Rts => Seq::Rts,
            Op::Rti => Seq::Rti,
            Op::Brk => Seq::Brk,
            _ => Seq::Imp,
        },
        Mode::Acc => Seq::Imp,
        Mode::Imm => Seq::Imm,
        Mode::Zp  => Seq::Zp(k),
        Mode::Zpx => Seq::Zpi(k, Idx::X),
        Mode::Zpy => Seq::Zpi(k, Idx::Y),
        Mode::Abs => match op { Op::Jmp => Seq::JmpAbs, Op::Jsr => Seq::Jsr, _ => Seq::Abs(k) },
        Mode::Abx => Seq::Abi(k, Idx::X),
        Mode::Aby => Seq::Abi(k, Idx::Y),
        Mode::Ind => Seq::JmpInd,
        Mode::Izx => Seq::Izx(k),
        Mode::Izy => Seq::Izy(k),
        Mode::Rel => Seq::Rel,
    }
}

fn decode_table() -> &'static [(Op, Seq); 256] {
    static TABLE: OnceLock<[(Op, Seq); 256]> = OnceLock::new();
    TABLE.get_or_init(|| {
        let mut t = [(Op::Nop, Seq::Imp); 256];
        for (i, (mn, mode, _)) in OPCODES.iter().enumerate() {
            let op = op_from_mnemonic(mn);
            t[i] = (op, seq_of(op, *mode));
        }
        t
    })
}

// ── CPU ───────────────────────────────────────────────────────────────────────

/// Outcome of one CPU machine cycle.
#[derive(Clone, Copy, Default)]
pub struct CycleResult {
    /// The instruction (or interrupt sequence) finished in this cycle.
    pub done: bool,
    /// The CPU was stalled (BA/AEC) and did nothing.
    pub stalled: bool,
}

pub struct Cpu {
    pub a: u8,
    pub x: u8,
    pub y: u8,
    pub sp: u8,
    pub pc: u16,
    pub p: Flags,
    /// Elapsed machine cycles (counted by the system, even while stalled).
    pub total_cycles: u64,
    /// NMI latched (falling edge seen, not yet serviced).
    pub nmi_pending: bool,
    /// Last sample of the IRQ line (informational).
    pub irq_pending: bool,
    /// Interrupt serviced by the last completed sequence: 0 none, 1 IRQ, 2 NMI.
    pub last_interrupt: u8,
    /// Opcode of the last instruction started.
    pub last_opcode: u8,

    // Microcycle state
    op: Op,
    seq: Seq,
    step: u8,
    lo: u8,
    hi: u8,
    ptr: u8,
    data: u8,
    addr: u16,
    page_cross: bool,
    int_vector: u16,
    int_kind: u8,
    /// I flag at the start of the current cycle (interrupt polling reads it
    /// before the last cycle of CLI/SEI/PLP/RTI changes it).
    i_at_cycle_start: bool,
    poll_i: bool,
    /// Cycles executed by the CPU since the IRQ line was raised (and since the
    /// NMI edge): the interrupt is serviced once there are at least 2.
    irq_delay: u8,
    nmi_delay: u8,
    nmi_line_prev: bool,
    /// The VIC stalled the CPU after the last executed cycle.
    stall_seen: bool,
    /// Decision taken at the instruction boundary, before a possible stall on
    /// the fetch: 0 no interrupt, 1 IRQ, 2 NMI.
    int_latched: Option<u8>,
    /// The instruction just finished was a taken branch: the extra cycle of
    /// the jump does not resample interrupts, so the line must be active one
    /// cycle earlier than usual (VICE: OPCODE_DELAYS_INTERRUPT).
    branch_taken: bool,
    /// The current instruction cleared V (for the 1541 SO pin).
    v_cleared: bool,
}

impl Default for Cpu {
    fn default() -> Self { Self::new() }
}

impl Cpu {
    pub fn new() -> Self {
        Self {
            a: 0, x: 0, y: 0,
            sp: 0xFD,
            pc: 0,
            p: Flags(0x24),
            total_cycles: 0,
            nmi_pending: false,
            irq_pending: false,
            last_interrupt: 0,
            last_opcode: 0xEA,
            op: Op::Nop,
            seq: Seq::Imp,
            step: 0,
            lo: 0, hi: 0, ptr: 0, data: 0, addr: 0,
            page_cross: false,
            int_vector: 0xFFFE,
            int_kind: 0,
            i_at_cycle_start: true,
            poll_i: true,
            irq_delay: 0,
            nmi_delay: 0,
            nmi_line_prev: false,
            stall_seen: false,
            int_latched: None,
            branch_taken: false,
            v_cleared: false,
        }
    }

    pub fn reset<B: CpuBus>(&mut self, bus: &mut B) {
        let lo = bus.read(0xFFFC) as u16;
        let hi = bus.read(0xFFFD) as u16;
        self.pc = (hi << 8) | lo;
        self.sp = 0xFD;
        self.p.set(Flags::I, true);
        self.step = 0;
        self.poll_i = true;
        self.irq_delay = 0;
        self.nmi_delay = 0;
        self.int_latched = None;
        self.nmi_pending = false;
    }

    /// True if the CPU is between one instruction and the next.
    pub fn at_boundary(&self) -> bool { self.step == 0 }

    /// Samples the interrupt lines at the end of a machine cycle (called by
    /// the system after every cycle, even if the CPU was stalled).
    ///
    /// As in VICE, counts the cycles executed with the interrupt already
    /// active (including the cycle in which the line rises). Cycles stolen by
    /// the VIC do not count, but if the interrupt arrived during a stall, the
    /// stall counts as one cycle (except during SEI).
    pub fn sample_lines(&mut self, irq_line: bool, nmi_line: bool, stalled: bool) {
        let nmi_edge = nmi_line && !self.nmi_line_prev;
        if nmi_edge {
            self.nmi_pending = true;
            self.nmi_delay = 0;
        }
        self.nmi_line_prev = nmi_line;
        let irq_edge = irq_line && !self.irq_pending;
        if irq_edge {
            self.irq_delay = 0;
        }
        self.irq_pending = irq_line;

        if stalled {
            self.stall_seen = true;
            return;
        }
        if self.stall_seen {
            self.stall_seen = false;
            if irq_line && !irq_edge && self.irq_delay == 0 && self.last_opcode != 0x78 {
                self.irq_delay = 1;
            }
            if self.nmi_pending && !nmi_edge && self.nmi_delay == 0 {
                self.nmi_delay = 1;
            }
        }
        if irq_line {
            self.irq_delay = self.irq_delay.saturating_add(1);
        }
        if self.nmi_pending {
            self.nmi_delay = self.nmi_delay.saturating_add(1);
        }
    }

    /// Interrupt to service at the instruction boundary: 0 none, 1 IRQ, 2 NMI.
    /// Requires at least 2 cycles executed with the interrupt active (3 after
    /// a taken branch without page crossing) and, for IRQ, the I flag read as
    /// 0 at the start of the instruction's last cycle.
    fn pending_interrupt(&self) -> u8 {
        let need = if self.branch_taken { 3 } else { 2 };
        if self.nmi_pending && self.nmi_delay >= need { 2 }
        else if self.irq_pending && self.irq_delay >= need && !self.poll_i { 1 }
        else { 0 }
    }

    /// True if the next fetch cycle will start an interrupt sequence (used by
    /// the debugger for tracing).
    pub fn interrupt_imminent(&self) -> bool {
        self.step == 0 && self.int_latched.unwrap_or_else(|| self.pending_interrupt()) != 0
    }

    /// Is the next cycle a write? (With BA low, writes go through.)
    #[inline(always)]
    pub fn next_is_write(&self) -> bool {
        let s = self.step;
        match self.seq {
            _ if s == 0 => false,
            Seq::Zp(Kind::Write) => s == 2,
            Seq::Zp(Kind::Rmw) => s == 3 || s == 4,
            Seq::Zpi(Kind::Write, _) => s == 3,
            Seq::Zpi(Kind::Rmw, _) => s == 4 || s == 5,
            Seq::Abs(Kind::Write) => s == 3,
            Seq::Abs(Kind::Rmw) => s == 4 || s == 5,
            Seq::Abi(Kind::Write, _) => s == 4,
            Seq::Abi(Kind::Rmw, _) => s == 5 || s == 6,
            Seq::Izx(Kind::Write) => s == 5,
            Seq::Izx(Kind::Rmw) => s == 6 || s == 7,
            Seq::Izy(Kind::Write) => s == 5,
            Seq::Izy(Kind::Rmw) => s == 6 || s == 7,
            Seq::Push => s == 2,
            Seq::Jsr => s == 3 || s == 4,
            Seq::Brk | Seq::Int => (2..=4).contains(&s),
            _ => false,
        }
    }

    /// Cycle in which another device (REU DMA) owns the bus: the CPU is
    /// stalled. At the instruction boundary the interrupt decision is taken
    /// anyway, as for a VIC stall.
    /// SO pin: a falling edge sets the V flag (on the 1541 it is the disk's
    /// BYTE READY signal).
    pub fn set_overflow(&mut self) {
        self.p.set(Flags::V, true);
    }

    pub fn dma_stall(&mut self) {
        if self.step == 0 && self.int_latched.is_none() {
            self.int_latched = Some(self.pending_interrupt());
        }
    }

    /// One machine cycle. `ba_low`: the VIC requests the bus (the CPU only
    /// completes writes); `aec_low`: the VIC owns the bus (the CPU is stalled).
    pub fn cycle<B: CpuBus>(&mut self, bus: &mut B, ba_low: bool, aec_low: bool) -> CycleResult {
        // At the instruction boundary the interrupt decision is taken right
        // away, even if the fetch will be stalled by the VIC
        if self.step == 0 && self.int_latched.is_none() {
            self.int_latched = Some(self.pending_interrupt());
        }
        let is_write = self.next_is_write();
        if aec_low || (ba_low && !is_write) {
            return CycleResult { done: false, stalled: true };
        }
        self.i_at_cycle_start = self.p.get(Flags::I);
        let done = if self.step == 0 { self.fetch(bus); false } else { self.micro(bus) };
        if done && self.v_cleared {
            // Like VICE: V cleared at the end of an instruction discards SO edges
            self.v_cleared = false;
            bus.so_discard(1);
        }
        if done {
            self.poll_i = self.i_at_cycle_start;
            self.branch_taken = self.seq == Seq::Rel && self.step >= 2;
            self.step = 0;
        } else {
            self.step += 1;
        }
        CycleResult { done, stalled: false }
    }

    /// Runs a whole instruction without the VIC (tests and debugger): returns the cycles.
    pub fn step<B: CpuBus>(&mut self, bus: &mut B) -> u8 {
        let mut n = 0u8;
        loop {
            let r = self.cycle(bus, false, false);
            n = n.wrapping_add(1);
            self.total_cycles += 1;
            let irq = self.irq_pending;
            self.sample_lines(irq, false, false);
            if r.done { return n; }
        }
    }

    // ── Cycle 0: opcode fetch or interrupt start ──────────────────────────────

    fn fetch<B: CpuBus>(&mut self, bus: &mut B) {
        self.last_interrupt = 0;
        bus.set_pc(self.pc);
        let pending = self.int_latched.take().unwrap_or_else(|| self.pending_interrupt());
        match pending {
            2 => {
                self.nmi_pending = false;
                self.int_kind = 2;
                self.int_vector = 0xFFFA;
                self.seq = Seq::Int;
                self.op = Op::Brk;
                let _ = bus.read(self.pc); // dummy fetch, PC does not advance
            }
            1 => {
                self.int_kind = 1;
                self.int_vector = 0xFFFE;
                self.seq = Seq::Int;
                self.op = Op::Brk;
                let _ = bus.read(self.pc);
            }
            _ => {
                let opcode = bus.read(self.pc);
                self.pc = self.pc.wrapping_add(1);
                self.last_opcode = opcode;
                let (op, seq) = decode_table()[opcode as usize];
                self.op = op;
                self.seq = seq;
            }
        }
        self.page_cross = false;
    }

    // ── Microcycles ───────────────────────────────────────────────────────────

    fn micro<B: CpuBus>(&mut self, bus: &mut B) -> bool {
        let s = self.step;
        match self.seq {
            Seq::Imp => {
                let _ = bus.read(self.pc); // dummy read of the next opcode
                self.exec_implied();
                true
            }
            Seq::Imm => {
                let v = bus.read(self.pc);
                self.pc = self.pc.wrapping_add(1);
                self.exec_read(v);
                true
            }

            Seq::Zp(k) => match s {
                1 => { self.lo = self.fetch_byte(bus); self.addr = self.lo as u16; false }
                _ => self.access(bus, k, s - 2),
            },
            Seq::Zpi(k, idx) => match s {
                1 => { self.lo = self.fetch_byte(bus); false }
                2 => {
                    let _ = bus.read(self.lo as u16); // dummy read before indexing
                    self.addr = self.lo.wrapping_add(self.index(idx)) as u16;
                    false
                }
                _ => self.access(bus, k, s - 3),
            },
            Seq::Abs(k) => match s {
                1 => { self.lo = self.fetch_byte(bus); false }
                2 => { self.hi = self.fetch_byte(bus); self.addr = u16::from_le_bytes([self.lo, self.hi]); false }
                _ => self.access(bus, k, s - 3),
            },
            Seq::Abi(k, idx) => match s {
                1 => { self.lo = self.fetch_byte(bus); false }
                2 => {
                    self.hi = self.fetch_byte(bus);
                    self.setup_indexed(self.index(idx));
                    false
                }
                _ => self.access_indexed(bus, k, s - 3),
            },
            Seq::Izx(k) => match s {
                1 => { self.ptr = self.fetch_byte(bus); false }
                2 => { let _ = bus.read(self.ptr as u16); self.ptr = self.ptr.wrapping_add(self.x); false }
                3 => { self.lo = bus.read(self.ptr as u16); false }
                4 => { self.hi = bus.read(self.ptr.wrapping_add(1) as u16); self.addr = u16::from_le_bytes([self.lo, self.hi]); false }
                _ => self.access(bus, k, s - 5),
            },
            Seq::Izy(k) => match s {
                1 => { self.ptr = self.fetch_byte(bus); false }
                2 => { self.lo = bus.read(self.ptr as u16); false }
                3 => {
                    self.hi = bus.read(self.ptr.wrapping_add(1) as u16);
                    self.setup_indexed(self.y);
                    false
                }
                _ => self.access_indexed(bus, k, s - 4),
            },

            Seq::Rel => match s {
                1 => {
                    self.lo = self.fetch_byte(bus);
                    // An SO edge arriving within the first cycle sets V
                    if matches!(self.op, Op::Bvc | Op::Bvs) && bus.so_edge(0) {
                        self.p.set(Flags::V, true);
                    }
                    !self.branch_taken() // not taken: done in 2 cycles
                }
                2 => {
                    let _ = bus.read(self.pc); // dummy
                    let target = self.pc.wrapping_add(self.lo as i8 as u16);
                    self.page_cross = (target & 0xFF00) != (self.pc & 0xFF00);
                    // PCL updated immediately; PCH fixed in the next cycle if needed
                    self.addr = target;
                    self.pc = (self.pc & 0xFF00) | (target & 0x00FF);
                    if !self.page_cross { true } else { false }
                }
                _ => {
                    let _ = bus.read(self.pc); // dummy read at the address with the wrong PCH
                    self.pc = self.addr;
                    true
                }
            },

            Seq::Push => match s {
                1 => { let _ = bus.read(self.pc); false }
                _ => {
                    if self.op == Op::Php && bus.so_edge(0) {
                        self.p.set(Flags::V, true);
                    }
                    let v = if self.op == Op::Php { self.p.0 | Flags::B | Flags::U } else { self.a };
                    self.push(bus, v);
                    true
                }
            },
            Seq::Pull => match s {
                1 => { let _ = bus.read(self.pc); false }
                2 => { let _ = bus.read(0x0100 | self.sp as u16); self.sp = self.sp.wrapping_add(1); false }
                _ => {
                    let v = bus.read(0x0100 | self.sp as u16);
                    if self.op == Op::Plp {
                        self.p = Flags((v & !Flags::B) | Flags::U);
                    } else {
                        self.a = v;
                        self.p.set_nz(v);
                    }
                    true
                }
            },
            Seq::Rts => match s {
                1 => { let _ = bus.read(self.pc); false }
                2 => { let _ = bus.read(0x0100 | self.sp as u16); self.sp = self.sp.wrapping_add(1); false }
                3 => { self.lo = bus.read(0x0100 | self.sp as u16); self.sp = self.sp.wrapping_add(1); false }
                4 => { self.hi = bus.read(0x0100 | self.sp as u16); self.pc = u16::from_le_bytes([self.lo, self.hi]); false }
                _ => { let _ = bus.read(self.pc); self.pc = self.pc.wrapping_add(1); true }
            },
            Seq::Rti => match s {
                1 => { let _ = bus.read(self.pc); false }
                2 => { let _ = bus.read(0x0100 | self.sp as u16); self.sp = self.sp.wrapping_add(1); false }
                3 => {
                    let v = bus.read(0x0100 | self.sp as u16);
                    self.sp = self.sp.wrapping_add(1);
                    self.p = Flags((v & !Flags::B) | Flags::U);
                    false
                }
                4 => { self.lo = bus.read(0x0100 | self.sp as u16); self.sp = self.sp.wrapping_add(1); false }
                _ => { self.hi = bus.read(0x0100 | self.sp as u16); self.pc = u16::from_le_bytes([self.lo, self.hi]); true }
            },
            Seq::Jsr => match s {
                1 => { self.lo = self.fetch_byte(bus); false }
                2 => { let _ = bus.read(0x0100 | self.sp as u16); false }
                3 => { let v = (self.pc >> 8) as u8; self.push(bus, v); false }
                4 => { let v = self.pc as u8; self.push(bus, v); false }
                _ => { self.hi = bus.read(self.pc); self.pc = u16::from_le_bytes([self.lo, self.hi]); true }
            },
            Seq::Brk => match s {
                1 => { let _ = self.fetch_byte(bus); false } // padding byte
                2 => { let v = (self.pc >> 8) as u8; self.push(bus, v); false }
                3 => { let v = self.pc as u8; self.push(bus, v); false }
                4 => { let v = self.p.0 | Flags::B | Flags::U; self.push(bus, v); false }
                5 => { self.lo = bus.read(0xFFFE); self.p.set(Flags::I, true); false }
                _ => { self.hi = bus.read(0xFFFF); self.pc = u16::from_le_bytes([self.lo, self.hi]); true }
            },
            Seq::Int => match s {
                1 => { let _ = bus.read(self.pc); false }
                2 => { let v = (self.pc >> 8) as u8; self.push(bus, v); false }
                3 => { let v = self.pc as u8; self.push(bus, v); false }
                4 => { let v = (self.p.0 & !Flags::B) | Flags::U; self.push(bus, v); false }
                5 => { self.lo = bus.read(self.int_vector); self.p.set(Flags::I, true); false }
                _ => {
                    self.hi = bus.read(self.int_vector.wrapping_add(1));
                    self.pc = u16::from_le_bytes([self.lo, self.hi]);
                    self.last_interrupt = self.int_kind;
                    true
                }
            },
            Seq::JmpAbs => match s {
                1 => { self.lo = self.fetch_byte(bus); false }
                _ => { self.hi = bus.read(self.pc); self.pc = u16::from_le_bytes([self.lo, self.hi]); true }
            },
            Seq::JmpInd => match s {
                1 => { self.lo = self.fetch_byte(bus); false }
                2 => { self.hi = self.fetch_byte(bus); self.addr = u16::from_le_bytes([self.lo, self.hi]); false }
                3 => { self.lo = bus.read(self.addr); false }
                _ => {
                    // 6502 bug: the high byte is read without carry into the page
                    let hi_addr = (self.addr & 0xFF00) | ((self.addr + 1) & 0x00FF);
                    self.hi = bus.read(hi_addr);
                    self.pc = u16::from_le_bytes([self.lo, self.hi]);
                    true
                }
            },
        }
    }

    fn fetch_byte<B: CpuBus>(&mut self, bus: &mut B) -> u8 {
        let v = bus.read(self.pc);
        self.pc = self.pc.wrapping_add(1);
        v
    }

    fn index(&self, idx: Idx) -> u8 {
        match idx { Idx::X => self.x, Idx::Y => self.y }
    }

    /// Base address in lo/hi: computes the "wrong" address (without carry) and
    /// the correct one, recording the page cross.
    fn setup_indexed(&mut self, idx: u8) {
        let base = u16::from_le_bytes([self.lo, self.hi]);
        let eff = base.wrapping_add(idx as u16);
        self.page_cross = (base & 0xFF00) != (eff & 0xFF00);
        self.addr = eff;
        self.lo = self.lo.wrapping_add(idx); // for the address without carry
    }

    fn wrong_addr(&self) -> u16 {
        (self.addr & 0xFF00).wrapping_sub(if self.page_cross { 0x100 } else { 0 }) | self.lo as u16
    }

    /// Final access for non-indexed addressing: `n` is the relative cycle
    /// (0 = first access to the effective address).
    fn access<B: CpuBus>(&mut self, bus: &mut B, k: Kind, n: u8) -> bool {
        match (k, n) {
            (Kind::Read, _) => { let v = bus.read(self.addr); self.exec_read(v); true }
            (Kind::Write, _) => { let v = self.store_value(); bus.write(self.addr, v); true }
            (Kind::Rmw, 0) => { self.data = bus.read(self.addr); false }
            (Kind::Rmw, 1) => { bus.write_dummy(self.addr, self.data); self.data = self.exec_rmw(self.data); false }
            (Kind::Rmw, _) => { bus.write(self.addr, self.data); true }
        }
    }

    /// Final access for indexed addressing (abs,X/Y and (zp),Y): the first
    /// cycle reads at the address without carry; pure reads end there if
    /// there is no page cross.
    fn access_indexed<B: CpuBus>(&mut self, bus: &mut B, k: Kind, n: u8) -> bool {
        match (k, n) {
            (Kind::Read, 0) => {
                let v = bus.read(self.wrong_addr());
                if self.page_cross { false } else { self.exec_read(v); true }
            }
            (Kind::Read, _) => { let v = bus.read(self.addr); self.exec_read(v); true }
            (Kind::Write, 0) => { let _ = bus.read(self.wrong_addr()); false }
            (Kind::Write, _) => { let v = self.store_value(); bus.write(self.addr, v); true }
            (Kind::Rmw, 0) => { let _ = bus.read(self.wrong_addr()); false }
            (Kind::Rmw, 1) => { self.data = bus.read(self.addr); false }
            (Kind::Rmw, 2) => { bus.write_dummy(self.addr, self.data); self.data = self.exec_rmw(self.data); false }
            (Kind::Rmw, _) => { bus.write(self.addr, self.data); true }
        }
    }

    // ── Stack ─────────────────────────────────────────────────────────────────

    fn push<B: CpuBus>(&mut self, bus: &mut B, val: u8) {
        bus.write(0x0100 | self.sp as u16, val);
        self.sp = self.sp.wrapping_sub(1);
    }

    // ── Instruction semantics ─────────────────────────────────────────────────

    fn branch_taken(&self) -> bool {
        match self.op {
            Op::Bpl => !self.p.get(Flags::N), Op::Bmi => self.p.get(Flags::N),
            Op::Bvc => !self.p.get(Flags::V), Op::Bvs => self.p.get(Flags::V),
            Op::Bcc => !self.p.get(Flags::C), Op::Bcs => self.p.get(Flags::C),
            Op::Bne => !self.p.get(Flags::Z), Op::Beq => self.p.get(Flags::Z),
            _ => false,
        }
    }

    fn exec_implied(&mut self) {
        match self.op {
            Op::Tax => { self.x = self.a; self.p.set_nz(self.x); }
            Op::Tay => { self.y = self.a; self.p.set_nz(self.y); }
            Op::Txa => { self.a = self.x; self.p.set_nz(self.a); }
            Op::Tya => { self.a = self.y; self.p.set_nz(self.a); }
            Op::Tsx => { self.x = self.sp; self.p.set_nz(self.x); }
            Op::Txs => { self.sp = self.x; }
            Op::Inx => { self.x = self.x.wrapping_add(1); self.p.set_nz(self.x); }
            Op::Iny => { self.y = self.y.wrapping_add(1); self.p.set_nz(self.y); }
            Op::Dex => { self.x = self.x.wrapping_sub(1); self.p.set_nz(self.x); }
            Op::Dey => { self.y = self.y.wrapping_sub(1); self.p.set_nz(self.y); }
            Op::Clc => self.p.set(Flags::C, false),
            Op::Sec => self.p.set(Flags::C, true),
            Op::Cli => self.p.set(Flags::I, false),
            Op::Sei => self.p.set(Flags::I, true),
            Op::Clv => self.set_v(false),
            Op::Cld => self.p.set(Flags::D, false),
            Op::Sed => self.p.set(Flags::D, true),
            Op::Asl => { self.a = self.asl(self.a); }
            Op::Lsr => { self.a = self.lsr(self.a); }
            Op::Rol => { self.a = self.rol(self.a); }
            Op::Ror => { self.a = self.ror(self.a); }
            // KIL: the 6510 jams; here it stays on the instruction
            Op::Kil => { self.pc = self.pc.wrapping_sub(1); }
            _ => {} // NOP and variants
        }
    }

    fn exec_read(&mut self, v: u8) {
        match self.op {
            Op::Lda => { self.a = v; self.p.set_nz(v); }
            Op::Ldx => { self.x = v; self.p.set_nz(v); }
            Op::Ldy => { self.y = v; self.p.set_nz(v); }
            Op::Lax => { self.a = v; self.x = v; self.p.set_nz(v); }
            Op::Adc => self.adc(v),
            Op::Sbc => self.sbc(v),
            Op::And => self.and(v),
            Op::Ora => self.ora(v),
            Op::Eor => self.eor(v),
            Op::Cmp => { let a = self.a; self.cmp_reg(a, v); }
            Op::Cpx => { let x = self.x; self.cmp_reg(x, v); }
            Op::Cpy => { let y = self.y; self.cmp_reg(y, v); }
            Op::Bit => self.bit(v),
            Op::Anc => { self.and(v); let n = self.p.get(Flags::N); self.p.set(Flags::C, n); }
            Op::Alr => { self.and(v); self.a = self.lsr(self.a); }
            Op::Arr => self.arr(v),
            Op::Sbx => {
                let ax = self.a & self.x;
                let (r, borrow) = ax.overflowing_sub(v);
                self.x = r;
                self.p.set_nz(r);
                self.p.set(Flags::C, !borrow);
            }
            Op::Xaa => { self.a = self.x & v; self.p.set_nz(self.a); }
            Op::Las => { let r = v & self.sp; self.a = r; self.x = r; self.sp = r; self.p.set_nz(r); }
            _ => {} // NOP with operand
        }
    }

    fn exec_rmw(&mut self, v: u8) -> u8 {
        match self.op {
            Op::Inc => { let r = v.wrapping_add(1); self.p.set_nz(r); r }
            Op::Dec => { let r = v.wrapping_sub(1); self.p.set_nz(r); r }
            Op::Asl => self.asl(v),
            Op::Lsr => self.lsr(v),
            Op::Rol => self.rol(v),
            Op::Ror => self.ror(v),
            Op::Dcp => { let r = v.wrapping_sub(1); let a = self.a; self.cmp_reg(a, r); r }
            Op::Isc => { let r = v.wrapping_add(1); self.sbc(r); r }
            Op::Slo => { let r = self.asl(v); self.ora(r); r }
            Op::Rla => { let r = self.rol(v); self.and(r); r }
            Op::Sre => { let r = self.lsr(v); self.eor(r); r }
            Op::Rra => { let r = self.ror(v); self.adc(r); r }
            _ => v,
        }
    }

    fn store_value(&self) -> u8 {
        let hi1 = ((self.addr >> 8) as u8).wrapping_add(1);
        match self.op {
            Op::Sta => self.a,
            Op::Stx => self.x,
            Op::Sty => self.y,
            Op::Sax => self.a & self.x,
            Op::Ahx => self.a & self.x & hi1,
            Op::Shx => self.x & hi1,
            Op::Shy => self.y & hi1,
            Op::Tas => self.a & self.x & hi1,
            _ => 0,
        }
    }

    // ── ALU ───────────────────────────────────────────────────────────────────

    fn set_v(&mut self, v: bool) {
        self.p.set(Flags::V, v);
        self.v_cleared |= !v;
    }

    /// ADC; in decimal mode like the NMOS 6502: N and V from the result before
    /// the tens correction, Z from the binary sum.
    fn adc(&mut self, val: u8) {
        let carry = self.p.get(Flags::C) as u16;
        let a = self.a as u16;
        let v = val as u16;
        if self.p.get(Flags::D) {
            let mut lo = (a & 0x0F) + (v & 0x0F) + carry;
            if lo > 9 {
                lo += 6;
            }
            let mut t = (lo & 0x0F) + (a & 0xF0) + (v & 0xF0) + if lo > 0x0F { 0x10 } else { 0 };
            self.p.set(Flags::Z, (a + v + carry) & 0xFF == 0);
            self.p.set(Flags::N, t & 0x80 != 0);
            self.set_v((a ^ t) & 0x80 != 0 && (a ^ v) & 0x80 == 0);
            if t & 0x1F0 > 0x90 {
                t += 0x60;
            }
            self.p.set(Flags::C, t & 0xFF0 > 0xF0);
            self.a = t as u8;
        } else {
            let result = a + v + carry;
            self.p.set(Flags::C, result > 0xFF);
            self.set_v((!(a ^ v) & (a ^ result) & 0x80) != 0);
            self.a = result as u8;
            self.p.set_nz(self.a);
        }
    }

    /// SBC; in decimal mode like the NMOS 6502: flags from the binary
    /// subtraction, result corrected digit by digit.
    fn sbc(&mut self, val: u8) {
        if !self.p.get(Flags::D) {
            self.adc(!val);
            return;
        }
        let a = self.a as u32;
        let s = val as u32;
        let borrow = !self.p.get(Flags::C) as u32;
        let t = a.wrapping_sub(s).wrapping_sub(borrow) & 0xFFFF;
        let mut r = (a & 0x0F).wrapping_sub(s & 0x0F).wrapping_sub(borrow);
        r = if r & 0x10 != 0 {
            (r.wrapping_sub(6) & 0x0F) | (a & 0xF0).wrapping_sub(s & 0xF0).wrapping_sub(0x10)
        } else {
            (r & 0x0F) | (a & 0xF0).wrapping_sub(s & 0xF0)
        };
        if r & 0x100 != 0 {
            r = r.wrapping_sub(0x60);
        }
        self.p.set(Flags::C, t < 0x100);
        self.p.set_nz(t as u8);
        self.set_v((a ^ t) & 0x80 != 0 && (a ^ s) & 0x80 != 0);
        self.a = r as u8;
    }

    /// ARR (illegal AND + ROR); in decimal mode with the NMOS BCD corrections.
    fn arr(&mut self, val: u8) {
        let t = (self.a & val) as u32;
        let c = self.p.get(Flags::C) as u32;
        let mut r = (t | c << 8) >> 1;
        if self.p.get(Flags::D) {
            self.p.set(Flags::N, c != 0);
            self.p.set(Flags::Z, r == 0);
            self.set_v((r ^ t) & 0x40 != 0);
            if (t & 0x0F) + (t & 0x01) > 5 {
                r = (r & 0xF0) | ((r + 6) & 0x0F);
            }
            let carry = (t & 0xF0) + (t & 0x10) > 0x50;
            if carry {
                r = (r & 0x0F) | ((r + 0x60) & 0xF0);
            }
            self.p.set(Flags::C, carry);
        } else {
            self.p.set_nz(r as u8);
            self.p.set(Flags::C, r & 0x40 != 0);
            self.set_v(((r >> 6) ^ (r >> 5)) & 1 != 0);
        }
        self.a = r as u8;
    }

    fn cmp_reg(&mut self, reg: u8, val: u8) {
        let result = reg.wrapping_sub(val);
        self.p.set(Flags::C, reg >= val);
        self.p.set_nz(result);
    }

    fn and(&mut self, val: u8) { self.a &= val; self.p.set_nz(self.a); }
    fn ora(&mut self, val: u8) { self.a |= val; self.p.set_nz(self.a); }
    fn eor(&mut self, val: u8) { self.a ^= val; self.p.set_nz(self.a); }

    fn asl(&mut self, val: u8) -> u8 {
        self.p.set(Flags::C, val & 0x80 != 0);
        let r = val << 1; self.p.set_nz(r); r
    }
    fn lsr(&mut self, val: u8) -> u8 {
        self.p.set(Flags::C, val & 0x01 != 0);
        let r = val >> 1; self.p.set_nz(r); r
    }
    fn rol(&mut self, val: u8) -> u8 {
        let c = self.p.get(Flags::C) as u8;
        self.p.set(Flags::C, val & 0x80 != 0);
        let r = (val << 1) | c; self.p.set_nz(r); r
    }
    fn ror(&mut self, val: u8) -> u8 {
        let c = self.p.get(Flags::C) as u8;
        self.p.set(Flags::C, val & 0x01 != 0);
        let r = (val >> 1) | (c << 7); self.p.set_nz(r); r
    }

    fn bit(&mut self, val: u8) {
        self.p.set(Flags::N, val & 0x80 != 0);
        self.set_v(val & 0x40 != 0);
        self.p.set(Flags::Z, self.a & val == 0);
    }
}

// ── Saveable state ───────────────────────────────────────────────────────────

use crate::snapshot::{impl_state, impl_state_enum, Reader, Result as StateResult, State, Writer};

impl State for Flags {
    fn save(&self, w: &mut Writer) { self.0.save(w); }
    fn load(&mut self, r: &mut Reader) -> StateResult<()> { self.0.load(r) }
}

impl_state_enum!(Op {
    Lda, Ldx, Ldy, Sta, Stx, Sty, Adc, Sbc, And, Ora, Eor, Cmp, Cpx, Cpy, Bit,
    Inc, Dec, Asl, Lsr, Rol, Ror,
    Tax, Tay, Txa, Tya, Tsx, Txs, Inx, Iny, Dex, Dey,
    Clc, Sec, Cli, Sei, Clv, Cld, Sed, Nop,
    Pha, Php, Pla, Plp, Jmp, Jsr, Rts, Rti, Brk,
    Bpl, Bmi, Bvc, Bvs, Bcc, Bcs, Bne, Beq,
    Lax, Sax, Dcp, Isc, Slo, Rla, Sre, Rra, Anc, Alr, Arr, Sbx, Xaa, Ahx, Shx, Shy, Tas, Las, Kil,
});
impl_state_enum!(Kind { Read, Rmw, Write });
impl_state_enum!(Idx { X, Y });

impl State for Seq {
    fn save(&self, w: &mut Writer) {
        let (tag, kind, idx): (u8, Option<Kind>, Option<Idx>) = match *self {
            Seq::Imp => (0, None, None),
            Seq::Imm => (1, None, None),
            Seq::Zp(k) => (2, Some(k), None),
            Seq::Zpi(k, i) => (3, Some(k), Some(i)),
            Seq::Abs(k) => (4, Some(k), None),
            Seq::Abi(k, i) => (5, Some(k), Some(i)),
            Seq::Izx(k) => (6, Some(k), None),
            Seq::Izy(k) => (7, Some(k), None),
            Seq::Rel => (8, None, None),
            Seq::Push => (9, None, None),
            Seq::Pull => (10, None, None),
            Seq::Rts => (11, None, None),
            Seq::Rti => (12, None, None),
            Seq::Jsr => (13, None, None),
            Seq::Brk => (14, None, None),
            Seq::JmpAbs => (15, None, None),
            Seq::JmpInd => (16, None, None),
            Seq::Int => (17, None, None),
        };
        tag.save(w);
        if let Some(k) = kind { k.save(w); }
        if let Some(i) = idx { i.save(w); }
    }

    fn load(&mut self, r: &mut Reader) -> StateResult<()> {
        let mut tag = 0u8;
        tag.load(r)?;
        let mut k = Kind::Read;
        let mut i = Idx::X;
        if matches!(tag, 2..=7) { k.load(r)?; }
        if matches!(tag, 3 | 5) { i.load(r)?; }
        *self = match tag {
            0 => Seq::Imp, 1 => Seq::Imm, 2 => Seq::Zp(k), 3 => Seq::Zpi(k, i),
            4 => Seq::Abs(k), 5 => Seq::Abi(k, i), 6 => Seq::Izx(k), 7 => Seq::Izy(k),
            8 => Seq::Rel, 9 => Seq::Push, 10 => Seq::Pull, 11 => Seq::Rts, 12 => Seq::Rti,
            13 => Seq::Jsr, 14 => Seq::Brk, 15 => Seq::JmpAbs, 16 => Seq::JmpInd, 17 => Seq::Int,
            t => return Err(format!("Seq: invalid variant {t}")),
        };
        Ok(())
    }
}

impl_state!(Cpu {
    a, x, y, sp, pc, p, total_cycles, nmi_pending, irq_pending, last_interrupt, last_opcode,
    op, seq, step, lo, hi, ptr, data, addr, page_cross, int_vector, int_kind,
    i_at_cycle_start, poll_i, irq_delay, nmi_delay, nmi_line_prev, stall_seen, int_latched,
    branch_taken, v_cleared,
});
