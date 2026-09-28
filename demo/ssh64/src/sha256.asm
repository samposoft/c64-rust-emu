; SPDX-License-Identifier: GPL-2.0-or-later
; Copyright (C) 2026 SampoSoft - Francesco Sampoli
;
; SHA-256 (FIPS 180-4).
;
; Words are kept little-endian inside (byte 0 is the least significant);
; message and digest are big-endian as in the standard. The working
; variables a..h slide down a window: at round t they are at sha_v+X, X
; going down by 4 a round, so the new a is simply written below a and the
; renaming of the other seven costs nothing.
;
;   sha256_init
;   sha256_update   sha_p, sha_len: data
;   sha256_final    digest in sha256_out

        .section bss
sha_h   .fill 32                ; state H0..H7
sha_buf .fill 64                ; block being filled
sha_cnt .fill 4                 ; bytes hashed so far
        .cerror sha_cnt != sha_h + 96, "the state is copied as one block"
sha_w   .fill 256               ; message schedule
sha_v   .fill 160               ; window: 32 rounds + 8 words
sha256_out .fill 32
        .send bss

; --- 32-bit helpers on zero-page temporaries ------------------------------
rotr1   .macro t
        lda \t
        lsr a
        ror \t+3
        ror \t+2
        ror \t+1
        ror \t
        .endm
rotl1   .macro t
        lda \t+3
        asl a
        rol \t
        rol \t+1
        rol \t+2
        rol \t+3
        .endm
; d = t rotated right by 8r bits
setrot  .macro d, t, r
        .for m = 0, m < 4, m += 1
        lda \t+(m+\r)%4
        sta \d+m
        .next
        .endm
; d ^= t rotated right by 8r bits
xorrot  .macro d, t, r
        .for m = 0, m < 4, m += 1
        lda \d+m
        eor \t+(m+\r)%4
        sta \d+m
        .next
        .endm
; t = 4 bytes at 'src' indexed by x/y
ldx32   .macro t, src
        .for m = 0, m < 4, m += 1
        lda \src+m,x
        sta \t+m
        .next
        .endm
ldy32   .macro t, src
        .for m = 0, m < 4, m += 1
        lda \src+m,y
        sta \t+m
        .next
        .endm
cp32    .macro d, t
        .for m = 0, m < 4, m += 1
        lda \t+m
        sta \d+m
        .next
        .endm

; sha_s = Sigma1(e) = rotr6 ^ rotr11 ^ rotr25, e at sha_v+16,x
bsig1   .macro
        #ldx32 sha_t, sha_v+16
        #cp32 sha_u, sha_t
        #rotr1 sha_t
        #setrot sha_s, sha_t, 3         ; rotr25
        #rotr1 sha_t
        #rotr1 sha_t
        #xorrot sha_s, sha_t, 1         ; rotr11
        #rotl1 sha_u
        #rotl1 sha_u
        #xorrot sha_s, sha_u, 1         ; rotr6
        .endm

; sha_s2 = Sigma0(a) = rotr2 ^ rotr13 ^ rotr22, a at sha_v,x
bsig0   .macro
        #ldx32 sha_t, sha_v
        #cp32 sha_u, sha_t
        #rotr1 sha_t
        #rotr1 sha_t
        #setrot sha_s2, sha_t, 0        ; rotr2
        #rotl1 sha_u
        #rotl1 sha_u
        #xorrot sha_s2, sha_u, 3        ; rotr22
        #rotl1 sha_u
        #xorrot sha_s2, sha_u, 2        ; rotr13
        .endm

sha256_iv
        .dword $6a09e667, $bb67ae85, $3c6ef372, $a54ff53a
        .dword $510e527f, $9b05688c, $1f83d9ab, $5be0cd19

        .section reloc          ; (in the RAM under the I/O: see sha256_block)
sha256_k
        .dword $428a2f98, $71374491, $b5c0fbcf, $e9b5dba5
        .dword $3956c25b, $59f111f1, $923f82a4, $ab1c5ed5
        .dword $d807aa98, $12835b01, $243185be, $550c7dc3
        .dword $72be5d74, $80deb1fe, $9bdc06a7, $c19bf174
        .dword $e49b69c1, $efbe4786, $0fc19dc6, $240ca1cc
        .dword $2de92c6f, $4a7484aa, $5cb0a9dc, $76f988da
        .dword $983e5152, $a831c66d, $b00327c8, $bf597fc7
        .dword $c6e00bf3, $d5a79147, $06ca6351, $14292967
        .dword $27b70a85, $2e1b2138, $4d2c6dfc, $53380d13
        .dword $650a7354, $766a0abb, $81c2c92e, $92722c85
        .dword $a2bfe8a1, $a81a664b, $c24b8b70, $c76c51a3
        .dword $d192e819, $d6990624, $f40e3585, $106aa070
        .dword $19a4c116, $1e376c08, $2748774c, $34b0bcb5
        .dword $391c0cb3, $4ed8aa4a, $5b9cca4f, $682e6ff3
        .dword $748f82ee, $78a5636f, $84c87814, $8cc70208
        .dword $90befffa, $a4506ceb, $bef9a3f7, $c67178f2
        .send reloc

sha256_init .proc
        ldx #31
-       lda sha256_iv,x
        sta sha_h,x
        dex
        bpl -
        lda #0
        sta sha_cnt
        sta sha_cnt+1
        sta sha_cnt+2
        sta sha_cnt+3
        rts
        .pend

; adds sha_len bytes from (sha_p)
sha256_update .proc
_next   lda sha_len
        ora sha_len+1
        beq _done
        lda sha_cnt
        and #63
        tax
        ldy #0
        lda (sha_p),y
        sta sha_buf,x
        inc sha_p
        bne +
        inc sha_p+1
+       lda sha_len
        bne +
        dec sha_len+1
+       dec sha_len
        inc sha_cnt
        bne +
        inc sha_cnt+1
        bne +
        inc sha_cnt+2
        bne +
        inc sha_cnt+3
+       lda sha_cnt
        and #63
        bne _next
        jsr sha256_block
        jmp _next
_done   rts
        .pend

; adds the byte in A
sha256_byte .proc
        sta sha_tmp
        lda #<sha_tmp
        sta sha_p
        lda #>sha_tmp
        sta sha_p+1
        lda #1
        sta sha_len
        lda #0
        sta sha_len+1
        jmp sha256_update
        .pend

; padding and length, digest (big-endian) in sha256_out
sha256_final .proc
        ; bit length = 8 * bytes, big-endian, taken before the padding
        ldx #3
-       lda sha_cnt,x
        sta sha_t,x
        dex
        bpl -
        ldx #3
-       asl sha_t
        rol sha_t+1
        rol sha_t+2
        rol sha_t+3
        dex
        bne -
        lda sha_t+3
        sta _len
        lda sha_t+2
        sta _len+1
        lda sha_t+1
        sta _len+2
        lda sha_t
        sta _len+3
        lda #$80
-       jsr sha256_byte
        lda sha_cnt
        and #63
        cmp #56
        beq +
        lda #0
        beq -
+       lda #0                  ; high 32 bits of the length
        jsr sha256_byte
        jsr sha256_byte
        jsr sha256_byte
        jsr sha256_byte
        lda #<_len
        sta sha_p
        lda #>_len
        sta sha_p+1
        lda #4
        sta sha_len
        lda #0
        sta sha_len+1
        jsr sha256_update
        ldx #0                  ; digest: words big-endian
-       lda sha_h+3,x
        sta sha256_out,x
        lda sha_h+2,x
        sta sha256_out+1,x
        lda sha_h+1,x
        sta sha256_out+2,x
        lda sha_h,x
        sta sha256_out+3,x
        inx
        inx
        inx
        inx
        cpx #32
        bne -
        rts
        .section bss
_len    .fill 4
        .send bss
        .pend

; compresses sha_buf into sha_h (the I/O out meanwhile: K is in the RAM
; under it)
sha256_block .proc
        lda $01
        pha
        lda #$34
        sta $01
        ; W[0..15]: big-endian words of the block
        ldx #0
-       lda sha_buf+3,x
        sta sha_w,x
        lda sha_buf+2,x
        sta sha_w+1,x
        lda sha_buf+1,x
        sta sha_w+2,x
        lda sha_buf,x
        sta sha_w+3,x
        inx
        inx
        inx
        inx
        cpx #64
        bne -
        ; W[t] = s1(W[t-2]) + W[t-7] + s0(W[t-15]) + W[t-16]
        ldy #64
_sched  ; s1(W[t-2]) = rotr17 ^ rotr19 ^ shr10 -> sha_s
        #ldy32 sha_t, sha_w-8
        #rotr1 sha_t
        #setrot sha_s, sha_t, 2         ; rotr17
        #rotr1 sha_t
        #rotr1 sha_t
        #xorrot sha_s, sha_t, 2         ; rotr19
        lda sha_w-8+1,y                 ; shr10: >> 8, then >> 2
        sta sha_u
        lda sha_w-8+2,y
        sta sha_u+1
        lda sha_w-8+3,y
        lsr a
        ror sha_u+1
        ror sha_u
        lsr a
        ror sha_u+1
        ror sha_u
        eor sha_s+2
        sta sha_s+2
        lda sha_s
        eor sha_u
        sta sha_s
        lda sha_s+1
        eor sha_u+1
        sta sha_s+1
        ; s0(W[t-15]) = rotr7 ^ rotr18 ^ shr3 -> sha_s2
        #ldy32 sha_t, sha_w-60
        #cp32 sha_u, sha_t
        #rotl1 sha_u
        #setrot sha_s2, sha_u, 1        ; rotr7
        #rotr1 sha_t
        #rotr1 sha_t
        #xorrot sha_s2, sha_t, 2        ; rotr18
        #rotr1 sha_t                    ; rotr3, top 3 bits cleared = shr3
        lda sha_t+3
        and #$1f
        sta sha_t+3
        #xorrot sha_s2, sha_t, 0
        ; sum
        clc
        .for m = 0, m < 4, m += 1
        lda sha_s+m
        adc sha_s2+m
        sta sha_s+m
        .next
        clc
        .for m = 0, m < 4, m += 1
        lda sha_s+m
        adc sha_w-28+m,y
        sta sha_s+m
        .next
        clc
        .for m = 0, m < 4, m += 1
        lda sha_s+m
        adc sha_w-64+m,y
        sta sha_w+m,y
        .next
        iny
        iny
        iny
        iny
        beq _rounds
        jmp _sched
_rounds ; a..h = H at the top of the window
        ldx #31
-       lda sha_h,x
        sta sha_v+128,x
        dex
        bpl -
        ldx #128
        ldy #0
_round  ; T1 = h + Sigma1(e) + Ch(e,f,g) + K[t] + W[t]  -> sha_s
        #bsig1
        clc
        .for m = 0, m < 4, m += 1
        lda sha_v+20+m,x                ; Ch = g ^ (e & (f ^ g))
        eor sha_v+24+m,x
        and sha_v+16+m,x
        eor sha_v+24+m,x
        adc sha_s+m
        sta sha_s+m
        .next
        clc
        .for m = 0, m < 4, m += 1
        lda sha_s+m
        adc sha_v+28+m,x
        sta sha_s+m
        .next
        clc
        .for m = 0, m < 4, m += 1
        lda sha_s+m
        adc sha256_k+m,y
        sta sha_s+m
        .next
        clc
        .for m = 0, m < 4, m += 1
        lda sha_s+m
        adc sha_w+m,y
        sta sha_s+m
        .next
        ; d += T1
        clc
        .for m = 0, m < 4, m += 1
        lda sha_v+12+m,x
        adc sha_s+m
        sta sha_v+12+m,x
        .next
        ; new a = T1 + Sigma0(a) + Maj(a,b,c)
        #bsig0
        clc
        .for m = 0, m < 4, m += 1
        lda sha_v+m,x                   ; Maj = (a & b) | (c & (a | b))
        ora sha_v+4+m,x
        and sha_v+8+m,x
        sta sha_tmp
        lda sha_v+m,x
        and sha_v+4+m,x
        ora sha_tmp
        adc sha_s2+m
        sta sha_s2+m
        .next
        clc
        .for m = 0, m < 4, m += 1
        lda sha_s+m
        adc sha_s2+m
        sta sha_v-4+m,x
        .next
        dex
        dex
        dex
        dex
        iny
        iny
        iny
        iny
        beq _end
        cpx #0
        beq +
        jmp _round
+       ldx #31                         ; window bottom: move up again
-       lda sha_v,x
        sta sha_v+128,x
        dex
        bpl -
        ldx #128
        jmp _round
_end    ; H += a..h (at sha_v+x)
        stx sha_tmp
        ldy #0
-       clc
        lda sha_h,y
        adc sha_v,x
        sta sha_h,y
        lda sha_h+1,y
        adc sha_v+1,x
        sta sha_h+1,y
        lda sha_h+2,y
        adc sha_v+2,x
        sta sha_h+2,y
        lda sha_h+3,y
        adc sha_v+3,x
        sta sha_h+3,y
        inx
        inx
        inx
        inx
        iny
        iny
        iny
        iny
        cpy #32
        bne -
        pla
        sta $01
        rts
        .pend
