; SPDX-License-Identifier: GPL-2.0-or-later
; Copyright (C) 2026 SampoSoft - Francesco Sampoli
;
; Arithmetic modulo p = 2^255 - 19 on 32-byte little-endian numbers.
;
; Values are kept below 2^256 and only reduced below p by fe_freeze. The
; routines take pointers in fe_pa, fe_pb (operands) and fe_pd (result); the
; result may be one of the operands.
;
; Products: Karatsuba on two levels (32 = 2 x 16 = 4 x 8 bytes), then
; operand scanning in blocks of 8 x 8, one row per byte a_i of the first
; operand, the 8 products of a row unrolled. The table bases of every
; column are set once from b_j, so a product is four loads indexed by a_i:
;       lo, hi = SQ1[a+b] - SQ2[a-b+255]
; r[i+j] + lo + carry goes back to r[i+j], hi + carries to the next column.

; --- one product: column j, carry in cp, carry out cn ---------------------
; the table bases are the low bytes at +2, +5, +10, +13 (see mulpatch)
prod    .macro j, cp, cn, first
        sec
        lda SQ1LO,y
        sbc SQ2LO,y
        sta fe_t
        lda SQ1HI,y
        sbc SQ2HI,y
        sta \cn
        lda fe_t
        .if \first
        clc
        .else
        clc
        adc \cp
        bcc +
        inc \cn
        clc
+
        .endif
        adc FE_R+\j,x
        sta FE_R+\j,x
        bcc +
        inc \cn
+
        .endm

; --- set the table bases of the product at 'at' from A = b_j ------------
mulpatch .macro at
        sta \at+2
        sta \at+10
        eor #$ff
        sta \at+5
        sta \at+13
        .endm

; --- 8 x 8 byte blocks -------------------------------------------------
; A block multiplies 8 row bytes by 8 patched columns into FE_R+x: row i
; adds its products to x+i .. x+i+7 (zero before row 0) and stores its
; carry at x+i+8. Any zero page address is FE_R + some x, since zero page
; indexing wraps. A row in three passes, none with a branch: the 8
; products into M8_L (low bytes) and M8_H (high bytes; each subtraction
; leaves the carry set, a product being >= 0), S = L + 256 H into M8_L,
; then S into the result.

; a product of the first pass: table bases at +1, +9 (b) and +4, +12 (~b)
prod2   .macro j
        lda SQ1LO,y
        sbc SQ2LO,y
        sta M8_L+\j
        lda SQ1HI,y
        sbc SQ2HI,y
        sta M8_H+\j
        .endm

mulpatch2 .macro at
        sta \at+1
        sta \at+9
        eor #$ff
        sta \at+4
        sta \at+12
        .endm

; patch the 8 columns from the 8 bytes at (fe_pc)
m8_patch .proc
        ldy #0
        .for j = 0, j < 8, j += 1
        lda (fe_pc),y
        #mulpatch2 m8_cols[j]
        .if j < 7
        iny
        .endif
        .next
        rts
        .pend

m8_rows .proc
_row
_src    ldy $ffff,x             ; row byte (patched: source - base)
        sec
_cols   := []
        .for j = 0, j < 8, j += 1
_cols   ..= [*]
        #prod2 j
        .next
        clc                     ; S = L + 256 H
        .for j = 1, j < 8, j += 1
        lda M8_L+j
        adc M8_H+j-1
        sta M8_L+j
        .next
        lda M8_H+7
        adc #0
        sta fe_t
        clc                     ; into the result
        .for j = 0, j < 8, j += 1
        lda M8_L+j
        adc FE_R+j,x
        sta FE_R+j,x
        .next
        lda fe_t
        adc #0
        sta FE_R+8,x
        inx
        cpx fe_end
        beq +
        jmp _row
+       rts
        .pend
m8_cols = m8_rows._cols

; a block: rows at (k_r), columns at (fe_pc), result at FE_R+A (16 bytes,
; the first 8 zero)
m8      .proc
        sta fe_x
        clc
        adc #8
        sta fe_end
        jsr m8_patch
        lda k_r                 ; row source - base
        sec
        sbc fe_x
        sta m8_rows._src+1
        lda k_r+1
        sbc #0
        sta m8_rows._src+2
        ldx fe_x
        jmp m8_rows
        .pend

; squares of the 8 bytes at (fe_pc) into FE_R+A (16 bytes): cross products
; by rows entered at column i+1, doubled, plus the squares on the diagonal
s8      .proc
        sta fe_x
        .for j = 1, j < 8, j += 1
        ldy #j
        lda (fe_pc),y
        #mulpatch s8_cols[j-1]
        .next
        lda fe_pc               ; row source: (fe_pc) - base
        sec
        sbc fe_x
        sta _src+1
        lda fe_pc+1
        sbc #0
        sta _src+2
        lda fe_x
        clc
        adc #7
        sta fe_end
        ldx fe_x
        lda #0                  ; (row i stores x+i+8 before it is added to)
        .for k = 0, k < 8, k += 1
        sta FE_R+k,x
        .next
        sta FE_R+15,x
        lda #<_first            ; row 0 enters at column 1
        sta _jmp+1
        lda #>_first
        sta _jmp+2
_row    lda #0                  ; no carry into the first product
        sta fe_c0
        sta fe_c1
_src    ldy $ffff,x             ; row byte (patched: (fe_pc) - base)
_jmp    jmp $ffff
_first
_cols   := []
        .for j = 1, j < 8, j += 1
_cols   ..= [*]
        .if j % 2 == 1
        #prod j, fe_c0, fe_c1, 0
        .else
        #prod j, fe_c1, fe_c0, 0
        .endif
        .next
        lda fe_c1
        sta FE_R+8,x
        lda _jmp+1              ; next row: one column further
        clc
        adc #_size
        sta _jmp+1
        bcc +
        inc _jmp+2
+       inx
        cpx fe_end
        bne _row
        ldx fe_x                ; double
        asl FE_R,x
        .for k = 1, k < 16, k += 1
        rol FE_R+k,x
        .next
        clc                     ; add the squares
        .for i = 0, i < 8, i += 1
        ldy #i
        lda (fe_pc),y
        tay
        lda SQRLO,y
        adc FE_R+2*i,x
        sta FE_R+2*i,x
        lda SQRHI,y
        adc FE_R+2*i+1,x
        sta FE_R+2*i+1,x
        .next
        rts
_size   = _cols[1] - _cols[0]
        .cerror _cols[6] - _cols[5] != _size, "products of different sizes"
        .pend
s8_cols = s8._cols

; --- Karatsuba, second level: 16 x 16 bytes -------------------------------
; u = u0 + u1 2^64, v = v0 + v1 2^64:
;   u v = w0 + (w0 + w2 +- |u0-u1| |v1-v0|) 2^64 + w2 2^128
; w0 at FE_R+k_o, w2 at FE_R+k_o+16, the middle product in k2_w.

K2_W    = (k2_w - FE_R) & $ff   ; offsets from FE_R

; zero page X (n bytes) = |(fe_pc)[0..n) - (fe_pc)[n..2n)|, compared
; from the top first; A = 1 when the second part is the larger. k_r
; points to the second part meanwhile.
absdiff .macro n
        lda fe_pc
        clc
        adc #\n
        sta k_r
        lda fe_pc+1
        adc #0
        sta k_r+1
        ldy #\n-1
-       lda (fe_pc),y
        cmp (k_r),y
        bne +
        dey
        bpl -
+       bcc _swap
        ldy #0
        .for k = 0, k < \n, k += 1
        lda (fe_pc),y
        sbc (k_r),y
        sta k,x
        .if k < \n-1
        iny
        .endif
        .next
        lda #0
        rts
_swap   ldy #0
        sec
        .for k = 0, k < \n, k += 1
        lda (k_r),y
        sbc (fe_pc),y
        sta k,x
        .if k < \n-1
        iny
        .endif
        .next
        lda #1
        rts
        .endm

absdiff8 .proc
        #absdiff 8
        .pend

absdiff16 .proc
        #absdiff 16
        .pend

; k2_tt (17 bytes) = w0 + w2 (FE_R+X, FE_R+X+16)
k2_sum  .macro
        clc
        .for k = 0, k < 16, k += 1
        lda FE_R+k,x
        adc FE_R+16+k,x
        sta k2_tt+k
        .next
        lda #0
        adc #0
        sta k2_tt+16
        .endm

; k2_tt +- k2_w: add if A = 0
k2_mid  .proc
        bne _sub
        clc
        .for k = 0, k < 16, k += 1
        lda k2_tt+k
        adc k2_w+k
        sta k2_tt+k
        .next
        lda k2_tt+16
        adc #0
        sta k2_tt+16
        rts
_sub    sec
        .for k = 0, k < 16, k += 1
        lda k2_tt+k
        sbc k2_w+k
        sta k2_tt+k
        .next
        lda k2_tt+16
        sbc #0
        sta k2_tt+16
        rts
        .pend

; FE_R+X+8 .. (17 bytes) += k2_tt, the carry on up to X+31
k2_join .proc
        clc
        .for k = 0, k < 17, k += 1
        lda FE_R+8+k,x
        adc k2_tt+k
        sta FE_R+8+k,x
        .next
        bcc _done
        .for k = 25, k < 32, k += 1
        inc FE_R+k,x
        bne _done
        .next
_done   rts
        .pend

; FE_R+k_o (32 bytes) = (k_u) * (k_v), 16 bytes each
k16     .proc
        lda k_u                 ; |u0 - u1|
        sta fe_pc
        lda k_u+1
        sta fe_pc+1
        ldx #k2_du
        jsr absdiff8
        sta k_s
        lda k_v                 ; |v1 - v0|
        sta fe_pc
        lda k_v+1
        sta fe_pc+1
        ldx #k2_dv
        jsr absdiff8
        eor k_s
        eor #1
        sta k_s                 ; 1: subtract the middle product
        ldx k_o                 ; w0 = u0 v0
        lda #0
        .for k = 0, k < 8, k += 1
        sta FE_R+k,x
        sta FE_R+16+k,x
        sta k2_w+k
        .next
        lda k_u
        sta k_r
        lda k_u+1
        sta k_r+1
        lda k_v
        sta fe_pc
        lda k_v+1
        sta fe_pc+1
        lda k_o
        jsr m8
        lda k_u                 ; w2 = u1 v1
        clc
        adc #8
        sta k_r
        lda k_u+1
        adc #0
        sta k_r+1
        lda k_v
        clc
        adc #8
        sta fe_pc
        lda k_v+1
        adc #0
        sta fe_pc+1
        lda k_o
        clc
        adc #16
        jsr m8
        lda #<k2_du             ; |u0-u1| |v1-v0|
        sta k_r
        lda #0
        sta k_r+1
        lda #<k2_dv
        sta fe_pc
        lda #0
        sta fe_pc+1
        lda #K2_W
        jsr m8
        ldx k_o
        #k2_sum
        lda k_s
        jsr k2_mid
        ldx k_o
        jmp k2_join
        .pend

; FE_R+k_o (32 bytes) = (k_u)^2, 16 bytes:
;   w0 + (w0 + w2 - (u0-u1)^2) 2^64 + w2 2^128
sq16    .proc
        lda k_u
        sta fe_pc
        lda k_u+1
        sta fe_pc+1
        ldx #k2_du
        jsr absdiff8
        lda k_o                 ; w0 = u0^2
        jsr s8
        lda k_u                 ; w2 = u1^2
        clc
        adc #8
        sta fe_pc
        lda k_u+1
        adc #0
        sta fe_pc+1
        lda k_o
        clc
        adc #16
        jsr s8
        lda #<k2_du             ; (u0 - u1)^2
        sta fe_pc
        lda #0
        sta fe_pc+1
        lda #K2_W
        jsr s8
        ldx k_o
        #k2_sum
        lda #1
        jsr k2_mid
        ldx k_o
        jmp k2_join
        .pend

; --- Karatsuba, first level: 32 x 32 bytes --------------------------------
; a = a0 + a1 B, b = b0 + b1 B (B = 2^128):
;   a b = z0 + (z0 + z2 + (a0 - a1)(b1 - b0)) B + z2 B^2
; with z0 = a0 b0 in FE_R, z2 = a1 b1 in FE_R+32, |a0-a1| |b1-b0| in FE_M.

; fe_tt (33 bytes) = z0 + z2
kara_sum .macro
        clc
        .for k = 0, k < 32, k += 1
        lda FE_R+k
        adc FE_R+32+k
        sta fe_tt+k
        .next
        lda #0
        adc #0
        sta fe_tt+32
        .endm

; FE_R+16 .. += fe_tt, then reduce
kara_join .proc
        clc
        .for k = 0, k < 33, k += 1
        lda FE_R+16+k
        adc fe_tt+k
        sta FE_R+16+k
        .next
        bcc _done
        ldx #49
-       inc FE_R,x
        bne _done
        inx
        cpx #64
        bne -
_done   jmp fe_reduce
        .pend

; k16 / sq16 of the 16 bytes at A/X (and Y/fe_tmp) into FE_R+offset
k16at   .macro u, v, o
        lda #<\u
        sta k_u
        lda #>\u
        sta k_u+1
        lda #<\v
        sta k_v
        lda #>\v
        sta k_v+1
        lda #\o
        sta k_o
        jsr k16
        .endm

sq16at  .macro u, o
        lda #<\u
        sta k_u
        lda #>\u
        sta k_u+1
        lda #\o
        sta k_o
        jsr sq16
        .endm

; k_u = zero page pointer p + offset
k16ptr  .macro p, off
        lda \p
        clc
        adc #\off
        sta k_u
        lda \p+1
        adc #0
        sta k_u+1
        .endm

; k_u, k_v = zero page pointers p, q + offset
k16ptr2 .macro p, q, off
        #k16ptr \p, \off
        lda \q
        clc
        adc #\off
        sta k_v
        lda \q+1
        adc #0
        sta k_v+1
        .endm

; (fe_pd) = (fe_pa) * (fe_pb): the operands are read where they are (the
; result is written only at the end: it may be one of them)
fe_mul  .proc
        ; |a0 - a1| and |b1 - b0|, k_s1 = sign of their product
        lda fe_pa
        sta fe_pc
        lda fe_pa+1
        sta fe_pc+1
        ldx #fe_ka
        jsr absdiff16
        sta k_s1
        lda fe_pb
        sta fe_pc
        lda fe_pb+1
        sta fe_pc+1
        ldx #fe_kb
        jsr absdiff16
        eor k_s1
        eor #1
        sta k_s1
        #k16ptr2 fe_pa, fe_pb, 0
        lda #0
        sta k_o
        jsr k16
        #k16ptr2 fe_pa, fe_pb, 16
        lda #32
        sta k_o
        jsr k16
        #k16at fe_ka, fe_kb, 64
        #kara_sum
        lda k_s1
        beq +
        jmp kara_sub
+       clc
        .for k = 0, k < 32, k += 1
        lda fe_tt+k
        adc FE_M+k
        sta fe_tt+k
        .next
        lda fe_tt+32
        adc #0
        sta fe_tt+32
        jmp kara_join
        .pend

; fe_tt -= FE_M, then join
kara_sub .proc
        sec
        .for k = 0, k < 32, k += 1
        lda fe_tt+k
        sbc FE_M+k
        sta fe_tt+k
        .next
        lda fe_tt+32
        sbc #0
        sta fe_tt+32
        jmp kara_join
        .pend

; (fe_pd) = (fe_pa)^2 = z0 + (z0 + z2 - (a0 - a1)^2) B + z2 B^2
fe_sqr  .proc
        lda fe_pa
        sta fe_pc
        lda fe_pa+1
        sta fe_pc+1
        ldx #fe_ka
        jsr absdiff16
        #k16ptr fe_pa, 0
        lda #0
        sta k_o
        jsr sq16
        #k16ptr fe_pa, 16
        lda #32
        sta k_o
        jsr sq16
        #sq16at fe_ka, 64
        #kara_sum
        jmp kara_sub
        .pend

; (fe_pd) = (fe_pa) * 121665 ($01db41): rows of products by $41 and $db
; over the 32 bytes, row r adding at FE_R+r, stored carry at FE_R+r+32
; (two columns a turn); then + a 2^16, a plain sum
fe_mul121665 .proc
        ldy #31
-       lda (fe_pa),y
        sta fe_mb,y
        dey
        bpl -
        lda #0
        ldx #63
-       sta FE_R,x
        dex
        bpl -
        lda #$41
        ldx #0
        jsr _row
        lda #$db
        ldx #1
        jsr _row
        clc                     ; + a 2^16 (x = $e0..$ff: the carry goes on;
        ldx #$e0                ; zero page indexing wraps)
-       lda fe_mb-$e0,x
        adc _r2,x
        sta _r2,x
        inx
        bne -
        lda FE_R+34
        adc #0
        sta FE_R+34
        jmp fe_reduce

_r2     = (FE_R + 2 - $e0) & $ff
_row    sta _p1+1               ; multiplier A in the table bases
        sta _p3+1
        sta _q1+1
        sta _q3+1
        eor #$ff
        sta _p2+1
        sta _p4+1
        sta _q2+1
        sta _q4+1
        txa                     ; row offset X in the result operands
        clc
        adc #FE_R
        sta _ra+1
        sta _rs+1
        sta _rb+1
        sta _rt+1
        adc #32
        sta _rf+1
        lda #0
        sta fe_c1
        ldx #0
_col    ldy fe_mb,x             ; even column: carry in c1, out c0
        sec
_p1     lda SQ1LO,y
_p2     sbc SQ2LO,y
        sta fe_t
_p3     lda SQ1HI,y
_p4     sbc SQ2HI,y
        sta fe_c0
        lda fe_t
        clc
        adc fe_c1
        bcc +
        inc fe_c0
        clc
+
_ra     adc FE_R,x
_rs     sta FE_R,x
        bcc +
        inc fe_c0
+       inx
        ldy fe_mb,x             ; odd column: carry in c0, out c1
        sec
_q1     lda SQ1LO,y
_q2     sbc SQ2LO,y
        sta fe_t
_q3     lda SQ1HI,y
_q4     sbc SQ2HI,y
        sta fe_c1
        lda fe_t
        clc
        adc fe_c0
        bcc +
        inc fe_c1
        clc
+
_rb     adc FE_R,x
_rt     sta FE_R,x
        bcc +
        inc fe_c1
+       inx
        cpx #32
        bne _col
        lda fe_c1
_rf     sta FE_R+32
        rts
        .pend

; FE_R (64 bytes) -> (fe_pd), below 2^256: r_lo + 38 r_hi
fe_reduce .proc
        ; r_hi = 38 r_hi (33 bytes, the top one in fe_x)
        clc
        ldy FE_R+32
        lda T38LO,y
        sta FE_R+32
        lda T38HI,y
        sta fe_t
        .for k = 1, k < 32, k += 1
        ldy FE_R+32+k
        lda T38LO,y
        adc fe_t
        sta FE_R+32+k
        lda T38HI,y
        sta fe_t
        .next
        lda fe_t
        adc #0
        sta fe_x
        ; d = r_lo + 38 r_hi
        clc
        ldy #0
        .for k = 0, k < 32, k += 1
        lda FE_R+k
        adc FE_R+32+k
        sta (fe_pd),y
        .if k < 31
        iny
        .endif
        .next
        lda fe_x                ; top (at most 39): add top*38 again
        adc #0
        tax
        ldy #0
        clc
        lda (fe_pd),y
        adc T38LO,x
        sta (fe_pd),y
        iny
        lda (fe_pd),y
        adc T38HI,x
        sta (fe_pd),y
        bcs _prop
        rts
_prop   iny
        cpy #32                 ; y < 32: carry clear
        beq fold_add38
        lda (fe_pd),y
        adc #1
        sta (fe_pd),y
        bcs _prop
        rts
        .pend

; (fe_pd) += 38, again for every wrap past 2^256 (2^256 = 38 mod p)
fold_add38 .proc
        ldy #0
        lda (fe_pd),y
        clc
        adc #38
        sta (fe_pd),y
        bcs _prop
        rts
_prop   iny
        cpy #32                 ; y < 32: carry clear
        beq fold_add38
        lda (fe_pd),y
        adc #1
        sta (fe_pd),y
        bcs _prop
        rts
        .pend

; (fe_pd) -= 38, again for every borrow of 2^256
fold_sub38 .proc
        ldy #0
        lda (fe_pd),y
        sec
        sbc #38
        sta (fe_pd),y
        bcc _prop
        rts
_prop   iny
        cpy #32                 ; y < 32: carry clear, sbc #0 takes 1
        beq fold_sub38
        lda (fe_pd),y
        sbc #0
        sta (fe_pd),y
        bcc _prop
        rts
        .pend

; (fe_pd) = (fe_pa) + (fe_pb)
fe_add  .proc
        clc
        ldy #0
        .for k = 0, k < 32, k += 1
        lda (fe_pa),y
        adc (fe_pb),y
        sta (fe_pd),y
        .if k < 31
        iny
        .endif
        .next
        bcc +
        jmp fold_add38
+       rts
        .pend

; (fe_pd) = (fe_pa) - (fe_pb)
fe_sub  .proc
        sec
        ldy #0
        .for k = 0, k < 32, k += 1
        lda (fe_pa),y
        sbc (fe_pb),y
        sta (fe_pd),y
        .if k < 31
        iny
        .endif
        .next
        bcs +
        jmp fold_sub38
+       rts
        .pend

; (fe_pd) = (fe_pa)
fe_copy .proc
        ldy #31
-       lda (fe_pa),y
        sta (fe_pd),y
        dey
        bpl -
        rts
        .pend

; (fe_pd) reduced below p (in place)
fe_freeze .proc
        jsr _once
_once   ; v >= p  <=>  t = v + 19 has bit 255 set or carries out;
        ; then v - p is t with bit 255 flipped
        ldy #0
        lda (fe_pd),y
        clc
        adc #19
        sta FE_R
        .for k = 1, k < 32, k += 1
        iny
        lda (fe_pd),y
        adc #0
        sta FE_R+k
        .next
        lda FE_R+31
        bcs _take
        bmi _take
        rts
_take   eor #$80
        sta FE_R+31
        ldy #31
-       lda FE_R,y
        sta (fe_pd),y
        dey
        bpl -
        rts
        .pend
