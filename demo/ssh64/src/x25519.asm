; SPDX-License-Identifier: GPL-2.0-or-later
; Copyright (C) 2026 SampoSoft - Francesco Sampoli
;
; X25519 (RFC 7748): x25519_out = x25519_k * x25519_u on Curve25519.
;
; Montgomery ladder over the 255 bits of the clamped scalar; its lowest 3
; bits are zero, doublings of (x2, z2) alone. The keys are ephemeral (one
; per connection), so the ladder is not written to take the same time for
; every scalar: the conditional swap is a swap of slot map entries, taken
; only when needed.
;
; Every step multiplies by x1, the point's u: a table of n x1, n = 1..255
; (33 bytes each, 8 KB) makes that product 32 rows of the table added up,
; half the time of a product. The program lists memory free while X25519
; runs in mx1_areas (address, number of entries; entries 1.. in order) and
; 512 bytes for the entries' addresses at MX1_PTR.

XS_X1   = 0
XS_X2   = 1
XS_Z2   = 2
XS_X3   = 3
XS_Z3   = 4
XS_A    = 5
XS_AA   = 6
XS_B    = 7
XS_BB   = 8
XS_E    = 9
XS_C    = 10
XS_D    = 11
XS_DA   = 12
XS_CB   = 13
XS_T1   = 14
XS_T2   = 15

        .section kbss
x25519_k   .fill 32             ; scalar (clamped in place)
x25519_u   .fill 32             ; u coordinate of the point
x25519_out .fill 32
x25519_swap .fill 1
x25519_byte .fill 1
x25519_mask .fill 1
        .send kbss

x25519_step
        .byte FE_ADD, XS_A, XS_X2, XS_Z2
        .byte FE_SQR, XS_AA, XS_A, 0
        .byte FE_SUB, XS_B, XS_X2, XS_Z2
        .byte FE_SQR, XS_BB, XS_B, 0
        .byte FE_SUB, XS_E, XS_AA, XS_BB
        .byte FE_ADD, XS_C, XS_X3, XS_Z3
        .byte FE_SUB, XS_D, XS_X3, XS_Z3
        .byte FE_MUL, XS_DA, XS_D, XS_A
        .byte FE_MUL, XS_CB, XS_C, XS_B
        .byte FE_ADD, XS_T1, XS_DA, XS_CB
        .byte FE_SQR, XS_X3, XS_T1, 0
        .byte FE_SUB, XS_T2, XS_DA, XS_CB
        .byte FE_SQR, XS_T2, XS_T2, 0
        .byte FE_MX1, XS_Z3, XS_T2, 0
        .byte FE_MUL, XS_X2, XS_AA, XS_BB
        .byte FE_M24, XS_T1, XS_E, 0
        .byte FE_ADD, XS_T1, XS_AA, XS_T1
        .byte FE_MUL, XS_Z2, XS_E, XS_T1
        .byte 0

x25519_dbl
        .byte FE_ADD, XS_A, XS_X2, XS_Z2
        .byte FE_SQR, XS_AA, XS_A, 0
        .byte FE_SUB, XS_B, XS_X2, XS_Z2
        .byte FE_SQR, XS_BB, XS_B, 0
        .byte FE_SUB, XS_E, XS_AA, XS_BB
        .byte FE_MUL, XS_X2, XS_AA, XS_BB
        .byte FE_M24, XS_T1, XS_E, 0
        .byte FE_ADD, XS_T1, XS_AA, XS_T1
        .byte FE_MUL, XS_Z2, XS_E, XS_T1
        .byte 0

x25519_final
        .byte FE_CPY, FS_Z, XS_Z2, 0
        .byte 0
x25519_final2
        .byte FE_MUL, XS_X2, XS_X2, FS_INV
        .byte 0

; swaps the slots of (x2, z2) and (x3, z3)
x25519_cswap .proc
        ldx fe_map+XS_X2
        ldy fe_map+XS_X3
        stx fe_map+XS_X3
        sty fe_map+XS_X2
        ldx fe_map+XS_Z2
        ldy fe_map+XS_Z3
        stx fe_map+XS_Z3
        sty fe_map+XS_Z2
        rts
        .pend

x25519  .proc
        lda x25519_k            ; clamp
        and #$f8
        sta x25519_k
        lda x25519_k+31
        and #$7f
        ora #$40
        sta x25519_k+31
        jsr fe_map_reset
        lda #<x25519_u
        sta fe_pa
        lda #>x25519_u
        sta fe_pa+1
        lda #XS_X1
        jsr fe_load
        lda fe_slots+32*XS_X1+31        ; top bit of u ignored
        and #$7f
        sta fe_slots+32*XS_X1+31
        jsr x25519_mx1
        lda #<(fe_slots+32*XS_X1)
        sta fe_pa
        lda #>(fe_slots+32*XS_X1)
        sta fe_pa+1
        lda #XS_X3
        jsr fe_load
        lda #XS_X2
        ldx #1
        jsr fe_set_small
        lda #XS_Z3
        ldx #1
        jsr fe_set_small
        lda #XS_Z2
        ldx #0
        jsr fe_set_small
        lda #0
        sta x25519_swap
        lda #31
        sta x25519_byte
        lda #$40                ; bit 254
        sta x25519_mask
_bit    ldx x25519_byte
        lda x25519_k,x
        and x25519_mask
        beq +
        lda #1
+       tax                     ; k_t
        eor x25519_swap
        stx x25519_swap
        beq +
        jsr x25519_cswap
+
        #fe_prog x25519_step
        lsr x25519_mask
        bne +
        lda #$80
        sta x25519_mask
        dec x25519_byte
        jmp _bit
+       lda x25519_byte         ; down to bit 3
        bne _bit
        lda x25519_mask
        cmp #$04
        bne _bit
        lda x25519_swap
        beq +
        jsr x25519_cswap
+       lda #3
        sta x25519_byte
-       #fe_prog x25519_dbl
        dec x25519_byte
        bne -
x25519_ladder_done
        #fe_prog x25519_final
        jsr fe_invert
        #fe_prog x25519_final2
        lda #<x25519_out
        sta fe_pb
        lda #>x25519_out
        sta fe_pb+1
        lda #XS_X2
        jmp fe_store
        .pend

; --- the table of n x1 ------------------------------------------------------------

MX1_LO  = MX1_PTR               ; entry n at MX1_LO+n / MX1_HI+n
MX1_HI  = MX1_PTR + 256

; the addresses of the entries through the areas, then entry n = entry
; n-1 + x1 (x1 in FE_M meanwhile)
x25519_mx1 .proc
        ldx #0                  ; area
        ldy #1                  ; entry
_area   lda mx1_areas,x
        sta fe_pc
        lda mx1_areas+1,x
        sta fe_pc+1
        lda mx1_areas+2,x
        sta fe_n
-       lda fe_pc
        sta MX1_LO,y
        lda fe_pc+1
        sta MX1_HI,y
        iny
        beq _fill
        lda fe_pc
        clc
        adc #33
        sta fe_pc
        bcc +
        inc fe_pc+1
+       dec fe_n
        bne -
        inx
        inx
        inx
        bne _area
_fill   ldy #31                 ; x1 (slot XS_X1, not yet remapped)
-       lda fe_slots+32*XS_X1,y
        sta FE_M,y
        dey
        bpl -
        lda MX1_LO+1            ; entry 1 = x1
        sta fe_pd
        lda MX1_HI+1
        sta fe_pd+1
        ldy #31
-       lda FE_M,y
        sta (fe_pd),y
        dey
        bpl -
        ldy #32
        lda #0
        sta (fe_pd),y
        ldx #2
_entry  lda fe_pd               ; the one before
        sta fe_pa
        lda fe_pd+1
        sta fe_pa+1
        lda MX1_LO,x
        sta fe_pd
        lda MX1_HI,x
        sta fe_pd+1
        ldy #0
        clc
-       lda (fe_pa),y
        adc FE_M,y
        sta (fe_pd),y
        iny
        tya                     ; (the carry stays)
        eor #32
        bne -
        lda (fe_pa),y
        adc #0
        sta (fe_pd),y
        inx
        bne _entry
        rts
        .pend

; (fe_pd) = (fe_pa) * x1: row k adds entry a_k at FE_R+k (33 bytes and a
; carry: the partial sums stay below the product, below 2^511)
fe_mulx1 .proc
        lda #0
        ldx #64
-       sta FE_R,x
        dex
        bpl -
        stx fe_x                ; k = -1, then 0..31
_k      inc fe_x
        ldy fe_x
        cpy #32
        beq _done
        lda (fe_pa),y
        beq _k
        tax
        lda MX1_LO,x
        sta fe_pc
        lda MX1_HI,x
        sta fe_pc+1
        ldx fe_x
        ldy #0
        clc
        .for i = 0, i < 33, i += 1
        lda (fe_pc),y
        adc FE_R+i,x
        sta FE_R+i,x
        .if i < 32
        iny
        .endif
        .next
        bcc _k
        inc FE_R+33,x
        jmp _k
_done   jmp fe_reduce
        .pend
