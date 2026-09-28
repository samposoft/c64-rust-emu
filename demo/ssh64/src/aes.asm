; SPDX-License-Identifier: GPL-2.0-or-later
; Copyright (C) 2026 SampoSoft - Francesco Sampoli
;
; AES-128 and AES-256 encryption (FIPS 197); CTR and GCM only encrypt.
;
; A round computes every output byte as four table loads: with S the
; S-box and S2 = 2S, S3 = 3S (in GF(2^8)), SubBytes, ShiftRows and
; MixColumns of a column are
;       b0 = S2[a0] ^ S3[a1] ^ S [a2] ^ S [a3] ^ k0     (and rotations)
; about 560 cycles a round. The round keys of the two directions are in
; their own pages; aes_use points the code at one of them.
;
;   aes_expand   aes_key (16 or 32 bytes, aes_klen) -> page at aes_page
;   aes_use      A = page of the round keys, X = rounds (10 or 14)
;   aes_block    aes_s -> aes_s

        .section bss
        .align 256
AES_S2  .fill 256
AES_S3  .fill 256
aes_key .fill 32
aes_klen .fill 1                ; 16 or 32
aes_nr  .fill 1                 ; rounds of the keys in use
aes_rcon .fill 1
aes_w   .fill 240               ; key schedule being built
        .send bss

        .section boot           ; (run once: see ssh64.asm)
aes_init .proc                  ; S2, S3 from the S-box
        ldx #0
-       lda aes_sbox,x
        asl a
        bcc +
        eor #$1b
+       sta AES_S2,x
        eor aes_sbox,x
        sta AES_S3,x
        inx
        bne -
        rts
        .pend
        .send boot

; one full round from state 'fr' to state 'to', keys at AES_RK+y
aes_round .segment fr, to
        .for c = 0, c < 4, c += 1
        .for r = 0, r < 4, r += 1
        ; column c, row r: the four inputs a0..a3 and their tables
        ldx \fr+4*((c+0)%4)+0
        .if r == 0
        lda AES_S2,x
        .elsif r == 3
        lda AES_S3,x
        .else
        lda aes_sbox,x
        .endif
        ldx \fr+4*((c+1)%4)+1
        .if r == 0
        eor AES_S3,x
        .elsif r == 1
        eor AES_S2,x
        .else
        eor aes_sbox,x
        .endif
        ldx \fr+4*((c+2)%4)+2
        .if r == 1
        eor AES_S3,x
        .elsif r == 2
        eor AES_S2,x
        .else
        eor aes_sbox,x
        .endif
        ldx \fr+4*((c+3)%4)+3
        .if r == 2
        eor AES_S3,x
        .elsif r == 3
        eor AES_S2,x
        .else
        eor aes_sbox,x
        .endif
aes_rkops ..= [* + 2]
        eor $ff00+4*c+r,y
        sta \to+4*c+r
        .next
        .next
        tya
        clc
        adc #16
        tay
        .endsegment

; last round (no MixColumns), 'fr' -> aes_s
aes_last .segment fr
        .for c = 0, c < 4, c += 1
        .for r = 0, r < 4, r += 1
        ldx \fr+4*((c+r)%4)+r
        lda aes_sbox,x
aes_rkops ..= [* + 2]
        eor $ff00+4*c+r,y
        sta aes_s+4*c+r
        .next
        .next
        .endsegment

aes_rkops := []

aes_block .proc
        ldy #0                  ; round 0: add the key
        .for k = 0, k < 16, k += 1
        lda aes_s+k
aes_rkops ..= [* + 2]
        eor $ff00+k,y
        sta aes_s+k
        .next
        ldy #16
        ldx aes_nr
        dex
        stx aes_n
_loop   #aes_round aes_s, aes_t
        dec aes_n
        bne +
        #aes_last aes_t
        rts
+       #aes_round aes_t, aes_s
        dec aes_n
        beq +
        jmp _loop
+       #aes_last aes_s
        rts
        .pend

AES_RKOPS = aes_rkops
        .cerror len(AES_RKOPS) != 80, "round key operands: ", len(AES_RKOPS)

; the code uses the round keys in page A, X rounds
aes_use .proc
        stx aes_nr
        .for op in AES_RKOPS
        sta op
        .next
        rts
        .pend

; round keys of aes_key (aes_klen bytes) into the page A; sets aes_nr
aes_expand .proc
        sta gcm_ap+1
        lda #0
        sta gcm_ap
        ldx aes_klen            ; the key is w[0..nk-1]
        dex
-       lda aes_key,x
        sta aes_w,x
        dex
        bpl -
        lda #1
        sta aes_rcon
        ldx #10
        lda aes_klen
        cmp #32
        bne +
        ldx #14
+       stx aes_nr
        inx                     ; 16 (nr + 1) bytes in all
        txa
        asl a
        asl a
        asl a
        asl a
        sta gcm_tmp
        ldx aes_klen
_word   lda aes_w-4,x           ; t = w[i-1]
        sta aes_t
        lda aes_w-3,x
        sta aes_t+1
        lda aes_w-2,x
        sta aes_t+2
        lda aes_w-1,x
        sta aes_t+3
        txa                     ; i mod nk (in bytes)
        ldy aes_klen
        cpy #32
        beq +
        and #15
        jmp ++
+       and #31
+       bne _k256
        ldy aes_t               ; RotWord, SubWord, rcon
        lda aes_t+1
        sta aes_t
        lda aes_t+2
        sta aes_t+1
        lda aes_t+3
        sta aes_t+2
        sty aes_t+3
        jsr _sub
        lda aes_t
        eor aes_rcon
        sta aes_t
        lda aes_rcon
        asl a
        bcc +
        eor #$1b
+       sta aes_rcon
        jmp _xor
_k256   cmp #16                 ; AES-256, i mod 8 = 4: SubWord
        bne _xor
        jsr _sub
_xor    txa                     ; w[i] = w[i-nk] ^ t
        sec
        sbc aes_klen
        tay
        .for k = 0, k < 4, k += 1
        lda aes_w+k,y
        eor aes_t+k
        sta aes_w+k,x
        .next
        inx
        inx
        inx
        inx
        cpx gcm_tmp
        bne _word
        ldy gcm_tmp             ; into the page
        dey
-       lda aes_w,y
        sta (gcm_ap),y
        dey
        cpy #255
        bne -
        rts

_sub    .for k = 0, k < 4, k += 1
        ldy aes_t+k
        lda aes_sbox,y
        sta aes_t+k
        .next
        rts
        .pend
