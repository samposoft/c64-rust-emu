; SPDX-License-Identifier: GPL-2.0-or-later
; Copyright (C) 2026 SampoSoft - Francesco Sampoli
;
; Multiplication tables, built at start-up (nothing to load from disk).
;
;   SQ1LO/HI[x] = floor(x*x/4)          x = 0..511
;   SQ2LO/HI[x] = floor((x-255)^2/4)    x = 0..511
;   SQRLO/HI[a] = a*a                   a = 0..255
;   T38LO/HI[a] = 38*a                  a = 0..255
;
; With y = a and the table bases moved by b (SQ1 + b, SQ2 + 255 - b):
;   a*b = SQ1[a+b] - SQ2[a-b+255]
; which is the product of two bytes in four indexed loads.

        .section kbss
        .align 256
SQ1LO   .fill 512
SQ1HI   .fill 512
SQ2LO   .fill 512
SQ2HI   .fill 512
SQRLO   .fill 256
SQRHI   .fill 256
T38LO   .fill 256
T38HI   .fill 256
        .send kbss

build_tables .proc
        ; SQ1: f(x+1) = f(x) + floor((x+1)/2)
        lda #0
        sta fe_t                ; f lo
        sta fe_c0               ; f hi
        sta fe_c1               ; x+1 lo
        sta fe_n                ; x+1 hi
        tax
_sq1    lda fe_t
        sta SQ1LO,x
        lda fe_c0
        sta SQ1HI,x
        jsr _step
        inx
        bne _sq1
_sq1b   lda fe_t
        sta SQ1LO+256,x
        lda fe_c0
        sta SQ1HI+256,x
        jsr _step
        inx
        bne _sq1b
        ; SQ2[x] = SQ1[255-x] for x <= 255, SQ1[x-255] above
        ldx #0
        ldy #255
-       lda SQ1LO,y
        sta SQ2LO,x
        lda SQ1HI,y
        sta SQ2HI,x
        inx
        dey
        cpy #255
        bne -
        ldx #0                  ; SQ2[255+k] = SQ1[k], k = 0..256
-       lda SQ1LO,x
        sta SQ2LO+255,x
        lda SQ1HI,x
        sta SQ2HI+255,x
        inx
        bne -
        lda SQ1LO+256           ; k = 256 (index 511, never read)
        sta SQ2LO+511
        lda SQ1HI+256
        sta SQ2HI+511
        ; SQR[a] = SQ1[2a]
        ldx #0
-       txa
        asl a
        tay
        bcs _hi
        lda SQ1LO,y
        sta SQRLO,x
        lda SQ1HI,y
        sta SQRHI,x
        jmp _nx
_hi     lda SQ1LO+256,y
        sta SQRLO,x
        lda SQ1HI+256,y
        sta SQRHI,x
_nx     inx
        bne -
        ; T38[a] = 38a
        lda #0
        sta fe_t
        sta fe_c0
        tax
-       lda fe_t
        sta T38LO,x
        lda fe_c0
        sta T38HI,x
        lda fe_t
        clc
        adc #38
        sta fe_t
        bcc +
        inc fe_c0
+       inx
        bne -
        rts

_step   inc fe_c1               ; x+1
        bne +
        inc fe_n
+       lda fe_n                ; f += (x+1) >> 1
        lsr a
        sta fe_tmp
        lda fe_c1
        ror a
        clc
        adc fe_t
        sta fe_t
        lda fe_tmp
        adc fe_c0
        sta fe_c0
        rts
        .pend
