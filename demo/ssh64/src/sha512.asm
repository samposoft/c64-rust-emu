; SPDX-License-Identifier: GPL-2.0-or-later
; Copyright (C) 2026 SampoSoft - Francesco Sampoli
;
; SHA-512 (FIPS 180-4), written small rather than fast: it hashes one
; block per connection (the Ed25519 check).
;
; 64-bit words little-endian inside, in one page (s5_page) reached by
; offsets in X and Y:
;   0 a..h   64 W[16] ring   192 T1   200 T2   208 scratch
;
;   sha512_init, sha512_update (sha_p, sha_len), sha512_final -> sha512_out

S5_A    = 0
S5_B    = 8
S5_C    = 16
S5_D    = 24
S5_E    = 32
S5_F    = 40
S5_G    = 48
S5_H    = 56
S5_W    = 64
S5_T1   = 192
S5_T2   = 200
S5_X    = 208

        .section ktail
        .align 256
s5_page .fill 256
s5_h    .fill 64                ; state
s5_buf  .fill 128
s5_cnt  .fill 4
sha512_out .fill 64
        .send ktail

sha512_iv
        .byte $08, $c9, $bc, $f3, $67, $e6, $09, $6a, $3b, $a7, $ca, $84, $85, $ae, $67, $bb
        .byte $2b, $f8, $94, $fe, $72, $f3, $6e, $3c, $f1, $36, $1d, $5f, $3a, $f5, $4f, $a5
        .byte $d1, $82, $e6, $ad, $7f, $52, $0e, $51, $1f, $6c, $3e, $2b, $8c, $68, $05, $9b
        .byte $6b, $bd, $41, $fb, $ab, $d9, $83, $1f, $79, $21, $7e, $13, $19, $cd, $e0, $5b
        .section reloc          ; (in the RAM under the I/O: see sha512_block)
sha512_k
        .byte $22, $ae, $28, $d7, $98, $2f, $8a, $42, $cd, $65, $ef, $23, $91, $44, $37, $71
        .byte $2f, $3b, $4d, $ec, $cf, $fb, $c0, $b5, $bc, $db, $89, $81, $a5, $db, $b5, $e9
        .byte $38, $b5, $48, $f3, $5b, $c2, $56, $39, $19, $d0, $05, $b6, $f1, $11, $f1, $59
        .byte $9b, $4f, $19, $af, $a4, $82, $3f, $92, $18, $81, $6d, $da, $d5, $5e, $1c, $ab
        .byte $42, $02, $03, $a3, $98, $aa, $07, $d8, $be, $6f, $70, $45, $01, $5b, $83, $12
        .byte $8c, $b2, $e4, $4e, $be, $85, $31, $24, $e2, $b4, $ff, $d5, $c3, $7d, $0c, $55
        .byte $6f, $89, $7b, $f2, $74, $5d, $be, $72, $b1, $96, $16, $3b, $fe, $b1, $de, $80
        .byte $35, $12, $c7, $25, $a7, $06, $dc, $9b, $94, $26, $69, $cf, $74, $f1, $9b, $c1
        .byte $d2, $4a, $f1, $9e, $c1, $69, $9b, $e4, $e3, $25, $4f, $38, $86, $47, $be, $ef
        .byte $b5, $d5, $8c, $8b, $c6, $9d, $c1, $0f, $65, $9c, $ac, $77, $cc, $a1, $0c, $24
        .byte $75, $02, $2b, $59, $6f, $2c, $e9, $2d, $83, $e4, $a6, $6e, $aa, $84, $74, $4a
        .byte $d4, $fb, $41, $bd, $dc, $a9, $b0, $5c, $b5, $53, $11, $83, $da, $88, $f9, $76
        .byte $ab, $df, $66, $ee, $52, $51, $3e, $98, $10, $32, $b4, $2d, $6d, $c6, $31, $a8
        .byte $3f, $21, $fb, $98, $c8, $27, $03, $b0, $e4, $0e, $ef, $be, $c7, $7f, $59, $bf
        .byte $c2, $8f, $a8, $3d, $f3, $0b, $e0, $c6, $25, $a7, $0a, $93, $47, $91, $a7, $d5
        .byte $6f, $82, $03, $e0, $51, $63, $ca, $06, $70, $6e, $0e, $0a, $67, $29, $29, $14
        .byte $fc, $2f, $d2, $46, $85, $0a, $b7, $27, $26, $c9, $26, $5c, $38, $21, $1b, $2e
        .byte $ed, $2a, $c4, $5a, $fc, $6d, $2c, $4d, $df, $b3, $95, $9d, $13, $0d, $38, $53
        .byte $de, $63, $af, $8b, $54, $73, $0a, $65, $a8, $b2, $77, $3c, $bb, $0a, $6a, $76
        .byte $e6, $ae, $ed, $47, $2e, $c9, $c2, $81, $3b, $35, $82, $14, $85, $2c, $72, $92
        .byte $64, $03, $f1, $4c, $a1, $e8, $bf, $a2, $01, $30, $42, $bc, $4b, $66, $1a, $a8
        .byte $91, $97, $f8, $d0, $70, $8b, $4b, $c2, $30, $be, $54, $06, $a3, $51, $6c, $c7
        .byte $18, $52, $ef, $d6, $19, $e8, $92, $d1, $10, $a9, $65, $55, $24, $06, $99, $d6
        .byte $2a, $20, $71, $57, $85, $35, $0e, $f4, $b8, $d1, $bb, $32, $70, $a0, $6a, $10
        .byte $c8, $d0, $d2, $b8, $16, $c1, $a4, $19, $53, $ab, $41, $51, $08, $6c, $37, $1e
        .byte $99, $eb, $8e, $df, $4c, $77, $48, $27, $a8, $48, $9b, $e1, $b5, $bc, $b0, $34
        .byte $63, $5a, $c9, $c5, $b3, $0c, $1c, $39, $cb, $8a, $41, $e3, $4a, $aa, $d8, $4e
        .byte $73, $e3, $63, $77, $4f, $ca, $9c, $5b, $a3, $b8, $b2, $d6, $f3, $6f, $2e, $68
        .byte $fc, $b2, $ef, $5d, $ee, $82, $8f, $74, $60, $2f, $17, $43, $6f, $63, $a5, $78
        .byte $72, $ab, $f0, $a1, $14, $78, $c8, $84, $ec, $39, $64, $1a, $08, $02, $c7, $8c
        .byte $28, $1e, $63, $23, $fa, $ff, $be, $90, $e9, $bd, $82, $de, $eb, $6c, $50, $a4
        .byte $15, $79, $c6, $b2, $f7, $a3, $f9, $be, $2b, $53, $72, $e3, $f2, $78, $71, $c6
        .byte $9c, $61, $26, $ea, $ce, $3e, $27, $ca, $07, $c2, $c0, $21, $c7, $b8, $86, $d1
        .byte $1e, $eb, $e0, $cd, $d6, $7d, $da, $ea, $78, $d1, $6e, $ee, $7f, $4f, $7d, $f5
        .byte $ba, $6f, $17, $72, $aa, $67, $f0, $06, $a6, $98, $c8, $a2, $c5, $7d, $63, $0a
        .byte $ae, $0d, $f9, $be, $04, $98, $3f, $11, $1b, $47, $1c, $13, $35, $0b, $71, $1b
        .byte $84, $7d, $04, $23, $f5, $77, $db, $28, $93, $24, $c7, $40, $7b, $ab, $ca, $32
        .byte $bc, $be, $c9, $15, $0a, $be, $9e, $3c, $4c, $0d, $10, $9c, $c4, $67, $1d, $43
        .byte $b6, $42, $3e, $cb, $be, $d4, $c5, $4c, $2a, $7e, $65, $fc, $9c, $29, $7f, $59
        .byte $ec, $fa, $d6, $3a, $ab, $6f, $cb, $5f, $17, $58, $47, $4a, $8c, $19, $44, $6c
        .send reloc

sha512_init .proc
        ldx #63
-       lda sha512_iv,x
        sta s5_h,x
        dex
        bpl -
        lda #0
        sta s5_cnt
        sta s5_cnt+1
        sta s5_cnt+2
        sta s5_cnt+3
        rts
        .pend

sha512_update .proc
_next   lda sha_len
        ora sha_len+1
        beq _done
        lda s5_cnt
        and #127
        tax
        ldy #0
        lda (sha_p),y
        sta s5_buf,x
        inc sha_p
        bne +
        inc sha_p+1
+       lda sha_len
        bne +
        dec sha_len+1
+       dec sha_len
        inc s5_cnt
        bne +
        inc s5_cnt+1
        bne +
        inc s5_cnt+2
        bne +
        inc s5_cnt+3
+       lda s5_cnt
        and #127
        bne _next
        jsr sha512_block
        jmp _next
_done   rts
        .pend

sha512_byte .proc
        sta sha_tmp
        lda #<sha_tmp
        sta sha_p
        lda #>sha_tmp
        sta sha_p+1
        lda #1
        sta sha_len
        lda #0
        sta sha_len+1
        jmp sha512_update
        .pend

sha512_final .proc
        ldx #3                  ; bit length (of up to 512 MB), big-endian
-       lda s5_cnt,x
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
        lda #$80
-       jsr sha512_byte
        lda s5_cnt
        and #127
        cmp #112                ; room for the 16-byte length
        beq +
        lda #0
        beq -
+       lda #12                 ; its upper 12 bytes are zero
        sta s5_n
-       lda #0
        jsr sha512_byte
        dec s5_n
        bne -
        lda sha_t+3
        jsr sha512_byte
        lda sha_t+2
        jsr sha512_byte
        lda sha_t+1
        jsr sha512_byte
        lda sha_t
        jsr sha512_byte
        ldy #0                  ; digest: words big-endian
_w      tya
        ora #7
        tax
-       lda s5_h,x
        sta sha512_out,y
        iny
        dex
        tya
        and #7
        bne -
        cpy #64
        bne _w
        rts
        .pend

; --- 64-bit operations on s5_page, source X, destination Y ----------------

s5_mov  .proc                   ; [Y] = [X]
        lda #8
        sta s5_n
-       lda s5_page,x
        sta s5_page,y
        inx
        iny
        dec s5_n
        bne -
        rts
        .pend

s5_add  .proc                   ; [Y] += [X]
        lda #8
        sta s5_n
        clc
-       lda s5_page,x
        adc s5_page,y
        sta s5_page,y
        inx
        iny
        dec s5_n
        bne -
        rts
        .pend

s5_adds .proc                   ; [Y] += s5_s
        ldx #0
        clc
-       lda s5_s,x
        adc s5_page,y
        sta s5_page,y
        iny
        inx
        txa
        eor #8
        bne -
        rts
        .pend

; s5_s = rot(X, n1) ^ rot(X, n2) ^ rot(X, n3): rotations right, or a shift
; when bit 7 of the amount is set; the three amounts follow the jsr
s5_sigma .proc
        stx s5_x
        pla
        sta s5_k
        pla
        sta s5_k+1
        lda #0
        ldx #7
-       sta s5_s,x
        dex
        bpl -
        ldy #1
        jsr _rot
        ldy #2
        jsr _rot
        ldy #3
        jsr _rot
        lda s5_k                ; return past the three bytes
        clc
        adc #3
        tax
        lda s5_k+1
        adc #0
        pha
        txa
        pha
        rts

_rot    lda (s5_k),y
        sta s5_sh
        and #$3f
        lsr a
        lsr a
        lsr a
        sta s5_kk
        ldy #0                  ; r[m] = x[m + n/8], wrapping or zero
_cp     tya
        clc
        adc s5_kk
        cmp #8
        bcc _in
        bit s5_sh
        bmi _zero
        sbc #8
_in     clc
        adc s5_x
        tax
        lda s5_page,x
        jmp _st
_zero   lda #0
_st     sta s5_r,y
        iny
        cpy #8
        bne _cp
        lda s5_sh               ; then n mod 8 single bits
        and #7
        beq _xor
        tax
_bit    bit s5_sh
        bmi _shr
        lda s5_r
        lsr a                   ; bit 0 goes round to the top
        jmp +
_shr    clc
+       ror s5_r+7
        ror s5_r+6
        ror s5_r+5
        ror s5_r+4
        ror s5_r+3
        ror s5_r+2
        ror s5_r+1
        ror s5_r
        dex
        bne _bit
_xor    ldx #7
-       lda s5_s,x
        eor s5_r,x
        sta s5_s,x
        dex
        bpl -
        rts
        .pend

; offset of W[A mod 16]
s5_wofs .proc
        and #15
        asl a
        asl a
        asl a
        clc
        adc #S5_W
        rts
        .pend

sha512_block .proc
        lda $01                 ; the I/O out: K is in the RAM under it
        pha
        lda #$34
        sta $01
        ldx #0                  ; W[0..15]: page[W + 8t + m] = buf[8t + 7 - m]
-       txa
        eor #7
        tay
        lda s5_buf,y
        sta s5_page+S5_W,x
        inx
        bpl -
        ldx #63                 ; a..h = H
-       lda s5_h,x
        sta s5_page,x
        dex
        bpl -
        lda #<sha512_k
        sta s5_kp
        lda #>sha512_k
        sta s5_kp+1
        lda #0
        sta s5_t
_round  lda s5_t
        cmp #16
        bcc _t1
        ; W[t] += s0(W[t-15]) + s1(W[t-2]) + W[t-7]
        adc #0                  ; (carry set) t+1 = t-15 mod 16
        jsr s5_wofs
        tax
        jsr s5_sigma
        .byte 1, 8, $80|7
        jsr _wt
        jsr s5_adds
        lda s5_t
        clc
        adc #14
        jsr s5_wofs
        tax
        jsr s5_sigma
        .byte 19, 61, $80|6
        jsr _wt
        jsr s5_adds
        lda s5_t
        clc
        adc #9
        jsr s5_wofs
        tax
        jsr _wt
        jsr s5_add
_t1     ; T1 = h + S1(e) + Ch(e,f,g) + K[t] + W[t]
        ldx #S5_H
        ldy #S5_T1
        jsr s5_mov
        ldx #S5_E
        jsr s5_sigma
        .byte 14, 18, 41
        ldy #S5_T1
        jsr s5_adds
        ldx #7
-       lda s5_page+S5_F,x      ; Ch = g ^ (e & (f ^ g))
        eor s5_page+S5_G,x
        and s5_page+S5_E,x
        eor s5_page+S5_G,x
        sta s5_s,x
        dex
        bpl -
        ldy #S5_T1
        jsr s5_adds
        ldy #0
        ldx #S5_T1
        clc
-       lda s5_page,x
        adc (s5_kp),y
        sta s5_page,x
        inx
        iny
        tya
        eor #8
        bne -
        lda s5_kp
        clc
        adc #8
        sta s5_kp
        bcc +
        inc s5_kp+1
+       lda s5_t
        jsr s5_wofs
        tax
        ldy #S5_T1
        jsr s5_add
        ; T2 = S0(a) + Maj(a,b,c)
        ldx #S5_A
        jsr s5_sigma
        .byte 28, 34, 39
        ldx #7
-       lda s5_page+S5_A,x      ; Maj = (c & (a | b)) | (a & b)
        ora s5_page+S5_B,x
        and s5_page+S5_C,x
        sta s5_n
        lda s5_page+S5_A,x
        and s5_page+S5_B,x
        ora s5_n
        sta s5_page+S5_T2,x
        dex
        bpl -
        ldy #S5_T2
        jsr s5_adds
        ; h..b = g..a, then a = T1 + T2, e = d + T1
        ldx #55
-       lda s5_page,x
        sta s5_page+8,x
        dex
        bpl -
        ldx #S5_T1
        ldy #S5_A
        jsr s5_mov
        ldx #S5_T2
        ldy #S5_A
        jsr s5_add
        ldx #S5_T1
        ldy #S5_E
        jsr s5_add
        inc s5_t
        lda s5_t
        cmp #80
        beq +
        jmp _round
+       ldx #0                  ; H += a..h
-       ldy #8
        clc
-       lda s5_h,x
        adc s5_page,x
        sta s5_h,x
        inx
        dey
        bne -
        cpx #64
        bne --
        pla
        sta $01
        rts

_wt     lda s5_t                ; Y = offset of W[t]
        jsr s5_wofs
        tay
        rts
        .pend
