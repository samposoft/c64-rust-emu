// SPDX-License-Identifier: GPL-2.0-or-later
// Copyright (C) 2026 SampoSoft - Francesco Sampoli

#[cfg(test)]
mod tests {
    use crate::cpu::Cpu;
    use crate::mem::Bus;

    fn make() -> (Cpu, Bus) {
        (Cpu::new(), Bus::new())
    }

    fn load_and_run(program: &[u8], steps: u32) -> (Cpu, Bus) {
        let (mut cpu, mut bus) = make();
        // Disable ROM banking: all RAM (LORAM=HIRAM=CHAREN=0)
        bus.cpu_port_dir = 0x07;
        bus.cpu_port_data = 0x00;
        bus.load_ram(0x0200, program);
        // Reset vector at $0200
        bus.load_ram(0xFFFC, &[0x00, 0x02]);
        cpu.reset(&mut bus);
        for _ in 0..steps { cpu.step(&mut bus); }
        (cpu, bus)
    }

    /// ADC/SBC in decimal mode: NMOS 6502 result and flags (N, V and Z from the
    /// binary sum, C from the corrected result).
    #[test]
    fn decimal_mode() {
        use crate::cpu::Flags;
        let cases: [(&[u8], u8, bool, bool); 5] = [
            // SED; CLC/SEC; LDA #a; ADC/SBC #b
            (&[0xF8, 0x18, 0xA9, 0x09, 0x69, 0x01], 0x10, false, false),
            (&[0xF8, 0x18, 0xA9, 0x99, 0x69, 0x01], 0x00, true, false), // Z from binary ($9A)
            (&[0xF8, 0x38, 0xA9, 0x10, 0xE9, 0x01], 0x09, true, false),
            (&[0xF8, 0x38, 0xA9, 0x00, 0xE9, 0x01], 0x99, false, false),
            (&[0xF8, 0x18, 0xA9, 0x50, 0x69, 0x50], 0x00, true, true),  // V from binary
        ];
        for (prog, a, c, v) in cases {
            let (cpu, _) = load_and_run(prog, 4);
            assert_eq!((cpu.a, cpu.p.get(Flags::C), cpu.p.get(Flags::V)), (a, c, v), "{prog:02x?}");
        }
        let (cpu, _) = load_and_run(&[0xF8, 0x18, 0xA9, 0x99, 0x69, 0x01], 4);
        assert!(!cpu.p.get(Flags::Z), "on NMOS, Z comes from the binary sum");
    }

    #[test]
    fn lda_immediate() {
        let (cpu, _) = load_and_run(&[0xA9, 0x42, 0x00], 1);
        assert_eq!(cpu.a, 0x42);
        assert!(!cpu.p.get(crate::cpu::Flags::Z));
        assert!(!cpu.p.get(crate::cpu::Flags::N));
    }

    #[test]
    fn lda_sets_zero_flag() {
        let (cpu, _) = load_and_run(&[0xA9, 0x00, 0x00], 1);
        assert_eq!(cpu.a, 0x00);
        assert!(cpu.p.get(crate::cpu::Flags::Z));
    }

    #[test]
    fn lda_sets_negative_flag() {
        let (cpu, _) = load_and_run(&[0xA9, 0x80, 0x00], 1);
        assert!(cpu.p.get(crate::cpu::Flags::N));
    }

    #[test]
    fn adc_no_carry() {
        // LDA #$10, ADC #$20
        let (cpu, _) = load_and_run(&[0xA9, 0x10, 0x69, 0x20], 2);
        assert_eq!(cpu.a, 0x30);
        assert!(!cpu.p.get(crate::cpu::Flags::C));
    }

    #[test]
    fn adc_carry_out() {
        // LDA #$FF, ADC #$01
        let (cpu, _) = load_and_run(&[0xA9, 0xFF, 0x69, 0x01], 2);
        assert_eq!(cpu.a, 0x00);
        assert!(cpu.p.get(crate::cpu::Flags::C));
        assert!(cpu.p.get(crate::cpu::Flags::Z));
    }

    #[test]
    fn sbc_basic() {
        // LDA #$50, SEC, SBC #$10
        let (cpu, _) = load_and_run(&[0xA9, 0x50, 0x38, 0xE9, 0x10], 3);
        assert_eq!(cpu.a, 0x40);
        assert!(cpu.p.get(crate::cpu::Flags::C));
    }

    #[test]
    fn inx_wraps() {
        // LDX #$FF, INX
        let (cpu, _) = load_and_run(&[0xA2, 0xFF, 0xE8], 2);
        assert_eq!(cpu.x, 0x00);
        assert!(cpu.p.get(crate::cpu::Flags::Z));
    }

    #[test]
    fn jmp_absolute() {
        // JMP $0205, then NOP at $0205, LDA #$AA at $0206
        let mut prog = [0u8; 10];
        prog[0] = 0x4C; prog[1] = 0x05; prog[2] = 0x02; // JMP $0205
        prog[3] = 0xA9; prog[4] = 0x00;                  // LDA #0 (skipped)
        prog[5] = 0xEA;                                   // NOP
        prog[6] = 0xA9; prog[7] = 0xAA;                  // LDA #$AA
        let (cpu, _) = load_and_run(&prog, 3);
        assert_eq!(cpu.a, 0xAA);
    }

    #[test]
    fn jsr_rts() {
        // JSR $0206, BRK, BRK, LDA #$42, RTS
        let mut prog = [0u8; 10];
        prog[0] = 0x20; prog[1] = 0x05; prog[2] = 0x02; // JSR $0205
        prog[3] = 0x00;                                   // BRK (not reached)
        prog[4] = 0x00;
        prog[5] = 0xA9; prog[6] = 0x42;                  // LDA #$42
        prog[7] = 0x60;                                   // RTS
        let (cpu, _) = load_and_run(&prog, 3);
        assert_eq!(cpu.a, 0x42);
    }

    #[test]
    fn branch_bne_taken() {
        // LDA #$01, CMP #$02, BNE +2, LDA #$FF, LDA #$42
        // BNE skips the 2 bytes of LDA #$FF, landing on LDA #$42
        let prog = [
            0xA9, 0x01,       // $0200: LDA #$01
            0xC9, 0x02,       // $0202: CMP #$02  -> Z=0
            0xD0, 0x02,       // $0204: BNE +2    -> jumps to $0208
            0xA9, 0xFF,       // $0206: LDA #$FF  <- skipped
            0xA9, 0x42,       // $0208: LDA #$42
        ];
        // step 1: LDA, step 2: CMP, step 3: BNE, step 4: LDA #$42
        let (cpu, _) = load_and_run(&prog, 4);
        assert_eq!(cpu.a, 0x42);
    }

    #[test]
    fn stack_push_pop() {
        // LDA #$AB, PHA, LDA #$00, PLA
        let (cpu, _) = load_and_run(&[0xA9, 0xAB, 0x48, 0xA9, 0x00, 0x68], 4);
        assert_eq!(cpu.a, 0xAB);
    }

    #[test]
    fn asl_accumulator() {
        // LDA #$40, ASL A
        let (cpu, _) = load_and_run(&[0xA9, 0x40, 0x0A], 2);
        assert_eq!(cpu.a, 0x80);
        assert!(cpu.p.get(crate::cpu::Flags::N));
        assert!(!cpu.p.get(crate::cpu::Flags::C));
    }

    #[test]
    fn rol_with_carry() {
        // SEC, LDA #$00, ROL A
        let (cpu, _) = load_and_run(&[0x38, 0xA9, 0x00, 0x2A], 3);
        assert_eq!(cpu.a, 0x01);
    }

    #[test]
    fn zero_page_read_write() {
        // LDA #$55, STA $10, LDA #$00, LDA $10
        let (cpu, _) = load_and_run(&[0xA9, 0x55, 0x85, 0x10, 0xA9, 0x00, 0xA5, 0x10], 4);
        assert_eq!(cpu.a, 0x55);
    }
}
