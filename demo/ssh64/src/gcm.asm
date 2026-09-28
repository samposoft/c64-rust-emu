; SPDX-License-Identifier: GPL-2.0-or-later
; Copyright (C) 2026 SampoSoft - Francesco Sampoli
;
; AES-GCM for SSH (aes128-gcm@openssh.com, aes256-gcm@openssh.com, RFC 5647).
;
; A packet is its 4-byte length (authenticated, not encrypted) followed by
; the encrypted body and the 16-byte tag. The nonce is a fixed IV of 12
; bytes whose last 8 count the packets.
;
; GHASH multiplies by H with Shoup's 8-bit tables: M_k[b] is byte k of
; b * H, one page per k, 16 pages (4 KB, 4 KB aligned) per direction. A
; product is Horner over the 16 bytes, Z = Z * x^8 + M[byte]: the shift is
; just an offset of one byte between two buffers, the byte that falls out
; comes back through gcm_r8a/b. About 2800 cycles a block.
;
;   gcm_setup   X = direction (0 send, 1 receive): aes_key, aes_klen,
;               gcm_iv_in (12 bytes)
;   gcm_seal    X = 0, gcm_p = packet, gcm_n = body length (multiple of 16)
;   gcm_open    X = 1, same; carry set if the tag is wrong

        .section bss
gcm_iv  .fill 2*12              ; nonces of the two directions
gcm_nr  .fill 2
gcm_iv_in .fill 12
gcm_dir .fill 1                 ; direction the code is set for ($ff: none)
gcm_mode .fill 1                ; 0 seal, 1 open
gcm_cnt .fill 1                 ; blocks left
gh_j    .fill 1
gcm_d   .fill 1                 ; direction being set up
        .send bss

GCM_RK  = [gcm_rk0, gcm_rk1]
GCM_M   = [gcm_m0, gcm_m1]

gh_mops := []                   ; operands to point at the M pages

; one Horner step: 'to' = 'fr' * x^8 + M[acc[gh_j]]
gh_step .segment fr, to
        ldx gh_j
        ldy gh_acc,x
        dec gh_j
        ldx \fr+15
gh_mops ..= [(* + 2, 0)]
        lda $ff00,y
        eor gcm_r8a,x
        sta \to
        lda \fr
gh_mops ..= [(* + 2, 1)]
        eor $ff00,y
        eor gcm_r8b,x
        sta \to+1
        .for k = 2, k < 16, k += 1
        lda \fr+k-1
gh_mops ..= [(* + 2, k)]
        eor $ff00,y
        sta \to+k
        .next
        .endsegment

; gh_acc = gh_acc * H
gh_mul  .proc
        ldy gh_acc+15
        .for k = 0, k < 16, k += 1
gh_mops ..= [(* + 2, k)]
        lda $ff00,y
        sta gh_za+k
        .next
        lda #14
        sta gh_j
_pair   #gh_step gh_za, gh_zb
        #gh_step gh_zb, gh_za
        lda gh_j
        bne _pair
        #gh_step gh_za, gh_acc
        rts
        .pend
GH_MOPS = gh_mops
        .cerror len(GH_MOPS) != 64, "GHASH operands: ", len(GH_MOPS)

; points the code at direction X (AES keys and M tables)
gcm_select .proc
        cpx gcm_dir
        bne +
        rts
+       stx gcm_dir
        lda _mpage,x
        .for op in GH_MOPS
        ora #op[1]
        sta op[0]
        and #$f0
        .next
        ldy gcm_dir
        ldx gcm_nr,y
        lda _rkpage,y
        jmp aes_use
_mpage  .byte >(GCM_M)
_rkpage .byte >(GCM_RK)
        .pend

; gh_acc ^= 16 bytes at (gcm_ap)
gh_xor  .proc
        ldy #15
-       lda (gcm_ap),y
        eor gh_acc,y
        sta gh_acc,y
        dey
        bpl -
        rts
        .pend

gcm_setup .proc
        stx gcm_d
        lda gcm_select._rkpage,x
        jsr aes_expand
        ldx gcm_d
        lda aes_nr
        sta gcm_nr,x
        lda #$ff                ; force gcm_select
        sta gcm_dir
        jsr gcm_select
        ldx gcm_d             ; nonce
        lda _ivofs,x
        tax
        ldy #0
-       lda gcm_iv_in,y
        sta gcm_iv,x
        inx
        iny
        cpy #12
        bne -
        ; H = E(0)
        ldx #15
        lda #0
-       sta aes_s,x
        dex
        bpl -
        jsr aes_block
        ; M[128 >> i] = H x^i, then M[a ^ b] = M[a] ^ M[b]
        ldx gcm_d
        lda _mpage,x
        sta gcm_ap+1
        lda #0
        sta gcm_ap
        tay                     ; M[0] = 0
        ldx #16
-       sta (gcm_ap),y
        inc gcm_ap+1
        dex
        bne -
        jsr _back
        lda #$80
        sta gcm_n
_pow    ldy gcm_n
        jsr _put
        lsr gcm_n               ; next power: shift right one bit
        beq _sums
        lda aes_s+15            ; the bit x^127 falls out: reduce
        lsr a
        php
        .for k = 0, k < 16, k += 1
        .if k == 0
        lsr aes_s
        .else
        ror aes_s+k
        .endif
        .next
        plp
        bcc _pow
        lda aes_s
        eor #$e1
        sta aes_s
        jmp _pow
_sums   lda #2                  ; i = 2, 4, .., 128; j = 1 .. i-1
        sta gcm_n
_i      lda #1
        sta gcm_cnt
_j      lda gcm_n               ; M[i + j] = M[i] ^ M[j], page by page
        ora gcm_cnt
        sta gh_j
        ldx #16
-       ldy gcm_n
        lda (gcm_ap),y
        ldy gcm_cnt
        eor (gcm_ap),y
        ldy gh_j
        sta (gcm_ap),y
        inc gcm_ap+1
        dex
        bne -
        jsr _back
        inc gcm_cnt
        lda gcm_cnt
        cmp gcm_n
        bne _j
        asl gcm_n
        bcc _i
        rts

_put    ; M[y] = aes_s
        ldx #0
-       lda aes_s,x
        sta (gcm_ap),y
        inc gcm_ap+1
        inx
        cpx #16
        bne -
_back   lda gcm_ap+1
        sec
        sbc #16
        sta gcm_ap+1
        rts
_mpage  = gcm_select._mpage
_ivofs  .byte 0, 12
        .pend

gcm_seal .proc
        lda #0
        jmp gcm_crypt
        .pend

gcm_open .proc
        lda #1
        jmp gcm_crypt
        .pend

; A = mode, X = direction
gcm_crypt .proc
        sta gcm_mode
        stx gcm_tmp
        jsr gcm_select
        ldx gcm_tmp             ; counter block = nonce || 1
        lda gcm_setup._ivofs,x
        tax
        ldy #0
-       lda gcm_iv,x
        sta gcm_ctr,y
        inx
        iny
        cpy #12
        bne -
        lda #0
        sta gcm_ctr+12
        sta gcm_ctr+13
        sta gcm_ctr+14
        lda #1
        sta gcm_ctr+15
        jsr _keystream          ; E(J0) masks the tag
        ldx #15
-       lda aes_s,x
        sta gcm_ej0,x
        dex
        bpl -
        ; GHASH of the length (AAD, 4 bytes)
        ldx #15
        lda #0
-       sta gh_acc,x
        dex
        bpl -
        ldy #3
-       lda (gcm_p),y
        sta gh_acc,y
        dey
        bpl -
        jsr gh_mul
        lda gcm_p               ; body
        clc
        adc #4
        sta gcm_ap
        lda gcm_p+1
        adc #0
        sta gcm_ap+1
        lda gcm_n+1             ; blocks = n / 16
        sta gcm_cnt
        lda gcm_n
        .for k = 0, k < 4, k += 1
        lsr gcm_cnt
        ror a
        .next
        sta gcm_cnt
        beq _len
_block  lda gcm_mode            ; open: GHASH the ciphertext first
        beq +
        jsr gh_xor
        jsr gh_mul
+       jsr _keystream
        ldy #15
-       lda (gcm_ap),y
        eor aes_s,y
        sta (gcm_ap),y
        dey
        bpl -
        lda gcm_mode
        bne +
        jsr gh_xor
        jsr gh_mul
+       lda gcm_ap
        clc
        adc #16
        sta gcm_ap
        bcc +
        inc gcm_ap+1
+       dec gcm_cnt
        bne _block
_len    ; lengths in bits: AAD 32, body 8n
        lda #0
        ldx #15
-       sta aes_s,x
        dex
        bpl -
        lda #32
        sta aes_s+7
        lda gcm_n
        asl a
        sta aes_s+15
        lda gcm_n+1
        rol a
        sta aes_s+14
        lda #0
        rol a
        sta aes_s+13
        lda aes_s+15            ; (8n: three shifts)
        asl a
        sta aes_s+15
        rol aes_s+14
        rol aes_s+13
        asl aes_s+15
        rol aes_s+14
        rol aes_s+13
        ldx #15
-       lda aes_s,x
        eor gh_acc,x
        sta gh_acc,x
        dex
        bpl -
        jsr gh_mul
        ; tag
        ldx gcm_tmp             ; next nonce
        lda gcm_setup._ivofs,x
        clc
        adc #11
        tax
        ldy #8
-       inc gcm_iv,x
        bne +
        dex
        dey
        bne -
+       ldy #15
        lda gcm_mode
        bne _check
-       lda gh_acc,y
        eor gcm_ej0,y
        sta (gcm_ap),y
        dey
        bpl -
        clc
        rts
_check  lda gh_acc,y
        eor gcm_ej0,y
        cmp (gcm_ap),y
        bne _bad
        dey
        bpl _check
        clc
        rts
_bad    sec
        rts

_keystream                      ; aes_s = E(counter), counter + 1
        ldx #15
-       lda gcm_ctr,x
        sta aes_s,x
        dex
        bpl -
        inc gcm_ctr+15
        bne +
        inc gcm_ctr+14
        bne +
        inc gcm_ctr+13
        bne +
        inc gcm_ctr+12
+       jmp aes_block
        .pend
