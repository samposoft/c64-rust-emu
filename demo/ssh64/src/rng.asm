; SPDX-License-Identifier: GPL-2.0-or-later
; Copyright (C) 2026 SampoSoft - Francesco Sampoli
;
; Random numbers. Entropy is stirred into a 32-byte pool from the moments
; things happen (keys pressed, frames received: CIA timer, raster, the
; SID's noise oscillator); secrets are SHA-256 of pool and a counter.
; The noise of voice 3 alone would not do: it is a register that shifts.

        .section bss
rng_pool .fill 32
rng_i   .fill 1
rng_ctr .fill 4
rng_fast .fill 1
        .send bss

        .section boot           ; (run once: see ssh64.asm)
rng_init .proc
        lda #$ff                ; voice 3: noise at the top frequency, silent
        sta $d40e
        sta $d40f
        lda #$80
        sta $d412
        lda #0
        sta $d418
        rts
        .pend
        .send boot

; stirs A and the machine's state into the pool
rng_stir .proc
        ldx rng_i
        eor rng_pool,x
        eor $dc04
        eor $d012
        sta rng_pool,x
        inx
        txa
        and #31
        tax
        lda rng_pool,x
        eor $d41b
        eor $dd04
        adc ticks
        sta rng_pool,x
        inx
        txa
        and #31
        sta rng_i
        rts
        .pend

; 32 random bytes in sha256_out (uses SHA-256: not during the exchange hash)
rng_secret .proc
        lda $dc04
        jsr rng_stir
        jsr sha256_init
        lda #<rng_pool
        sta sha_p
        lda #>rng_pool
        sta sha_p+1
        lda #37                 ; pool, index, counter
        sta sha_len
        lda #0
        sta sha_len+1
        jsr sha256_update
        inc rng_ctr
        bne +
        inc rng_ctr+1
+       jmp sha256_final
        .pend

; a byte that only needs to look random (padding): in A
rng_byte .proc
        lda rng_fast
        asl a
        bcc +
        eor #$1d
+       eor $d41b
        eor $dc04
        sta rng_fast
        rts
        .pend
