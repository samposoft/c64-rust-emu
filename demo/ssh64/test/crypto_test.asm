; SPDX-License-Identifier: GPL-2.0-or-later
; Copyright (C) 2026 SampoSoft - Francesco Sampoli
;
; Crypto routines of ssh64 for the test harness (flat RAM, no ROMs).

        .cpu "6502"
        .include "../src/zp.asm"

        * = $0800
        .dsection code
        .virtual *
        .dsection bssl
bssl_end
        .endv
        .cerror bssl_end > $a000, "code too long"

        ; GCM pages (4 KB aligned) and round keys
gcm_m0  = $d000
gcm_m1  = $e000
gcm_rk0 = $f000
gcm_rk1 = $f100

        .virtual $a000
        .dsection kbss
        .dsection ktail
ktail_end
        .dsection bss
bss_end
        .endv
        .cerror bss_end > $d000, "bss too long"

        .section code
        .include "../src/tables.asm"
        .include "../src/fe25519.asm"
        .include "../src/feprog.asm"
        .include "../src/x25519.asm"
        .include "../src/sha256.asm"
        .include "../src/sha512.asm"
        .include "../src/ed25519.asm"
        .include "../src/ed25519_tab.asm"
        .include "../src/ed25519_pts.asm"
        .include "../src/aes.asm"
        .include "../src/gcm.asm"
        .align 256
        .include "../src/aes_tab.asm"
        .dsection reloc
        .dsection boot

; memory free while X25519 runs, for its table (see ssh64.asm)
MX1_A   = fe_slots + 32 * 29
mx1_areas
        .word MX1_A
        .byte (ktail_end - MX1_A) / 33
        .word $e000
        .byte $1000 / 33
        .word $f200
        .byte ($fff0 - $f200) / 33
MX1_PTR = $0600

; entry points for the tests (a .proc nobody references is not assembled)
e_build_tables jmp build_tables
e_x25519 jmp x25519
e_x25519_base jmp x25519_base
e_sha256_init jmp sha256_init
e_sha256_update jmp sha256_update
e_sha256_final jmp sha256_final
e_sha512_init jmp sha512_init
e_sha512_update jmp sha512_update
e_sha512_final jmp sha512_final
e_ed_comb_build jmp ed_comb_build
e_aes_init jmp aes_init
e_aes_expand jmp aes_expand
e_aes_use jmp aes_use
e_aes_block jmp aes_block
e_gcm_setup jmp gcm_setup
e_gcm_seal jmp gcm_seal
e_gcm_open jmp gcm_open

; slot operations on fixed buffers
        .section bss
t_a     .fill 32
t_b     .fill 32
t_d     .fill 32
        .send bss

t_ptrs  .macro
        lda #<t_a
        sta fe_pa
        lda #>t_a
        sta fe_pa+1
        lda #<t_b
        sta fe_pb
        lda #>t_b
        sta fe_pb+1
        lda #<t_d
        sta fe_pd
        lda #>t_d
        sta fe_pd+1
        .endm

t_mul   #t_ptrs
        jmp fe_mul
t_sqr   #t_ptrs
        jmp fe_sqr
t_add   #t_ptrs
        jmp fe_add
t_sub   #t_ptrs
        jmp fe_sub
t_m24   #t_ptrs
        jmp fe_mul121665
t_freeze #t_ptrs
        jmp fe_freeze
t_inv   jsr fe_map_reset
        #t_ptrs
        lda #FS_Z
        jsr fe_load
        jsr fe_invert
        lda #FS_INV
        ldx #<t_d
        stx fe_pb
        ldx #>t_d
        stx fe_pb+1
        jmp fe_store
        .send code

; one ladder step on slots 0-4 (written by the test through the map)
        .section code
t_step  #fe_prog x25519_step
        rts
e_map_reset jmp fe_map_reset
        .send code

        .section code
t_verify jsr ed_comb_build      ; A = 0 if good (a new host)
        bcs +
        jsr ed25519_verify_comb
+       lda #0
        rol a
        rts
        .send code

        .section code
t_open  ldx #1                  ; A = 1 if the tag is wrong
        jsr gcm_open
        lda #0
        rol a
        rts
        .send code

        .section code
t_verify_comb
        jsr ed25519_verify_comb ; A = 0 if good
        lda #0
        rol a
        rts
        .send code
