; SPDX-License-Identifier: GPL-2.0-or-later
; Copyright (C) 2026 SampoSoft - Francesco Sampoli
;
; Field programs: formulas written as lists of operations on numbered
; slots of 32 bytes. Slots are reached through fe_map, so the Montgomery
; ladder swaps two points by swapping their map entries.
;
;   .byte op, d, a, b        d = a op b     (op 0 ends the program)

FE_MUL  = 1                     ; d = a * b
FE_SQR  = 2                     ; d = a^2
FE_ADD  = 3                     ; d = a + b
FE_SUB  = 4                     ; d = a - b
FE_M24  = 5                     ; d = a * 121665
FE_CPY  = 6                     ; d = a
FE_SQN  = 7                     ; d = a^(2^b), b squarings (b >= 1)
FE_MX1  = 8                     ; d = a * x1 of X25519 (its table)

FE_NSLOTS = 105                 ; 41 slots, then a 2 KB area (see ed25519.asm)

        .section kbss
fe_mb   .fill 32
fe_tt   .fill 33                ; Karatsuba middle sums
k2_tt   .fill 17
fe_map  .fill FE_NSLOTS         ; logical slot -> physical slot
        .send kbss

        ; the slots at the start of the variables that X25519 does not
        ; use: from slot 29 on, they hold its table (x25519.asm)
        .section ktail
fe_slots .fill 32 * FE_NSLOTS
        .send ktail

fe_sloths .byte >(fe_slots + 32 * range(FE_NSLOTS))
fe_slotlo .byte <(fe_slots + 32 * range(8))    ; (the same every 8 slots)

; identity slot map
fe_map_reset .proc
        ldx #FE_NSLOTS-1
-       txa
        sta fe_map,x
        dex
        bpl -
        rts
        .pend

; address of logical slot A in (X = zp pointer)
fe_slot_ptr .proc
        tay
        lda fe_map,y
        tay
        lda fe_sloths,y
        sta 1,x
        tya
        and #7
        tay
        lda fe_slotlo,y
        sta 0,x
        rts
        .pend

; called between two field operations (every 30-50 thousand cycles)
fe_idle jmp fe_idle_rts
fe_idle_rts rts

; runs the program at (fe_ip)
fe_run  .proc
_next   ldy #0
        lda (fe_ip),y
        beq _end
        sta fe_op
        iny
        lda (fe_ip),y
        ldx #fe_pd
        jsr fe_slot_ptr
        ldy #2
        lda (fe_ip),y
        ldx #fe_pa
        jsr fe_slot_ptr
        ldy #3
        lda (fe_ip),y
        sta fe_cnt
        ldx #fe_pb
        jsr fe_slot_ptr
        lda fe_ip
        clc
        adc #4
        sta fe_ip
        bcc +
        inc fe_ip+1
+       ldx fe_op
        lda _oplo-1,x
        sta _jmp+1
        lda _ophi-1,x
        sta _jmp+2
        jsr _jmp
        jsr fe_idle             ; the network, between two operations
        jmp _next
_end    rts
_jmp    jmp $ffff
_ops    = [fe_mul, fe_sqr, fe_add, fe_sub, fe_mul121665, fe_copy, _sqn, fe_mulx1]
_oplo   .byte <(_ops)
_ophi   .byte >(_ops)
_sqn    jsr fe_sqr
        lda fe_pd
        sta fe_pa
        lda fe_pd+1
        sta fe_pa+1
-       dec fe_cnt
        beq +
        jsr fe_sqr
        jmp -
+       rts
        .pend

; runs the program whose address follows the jsr
fe_prog .macro prog
        lda #<\prog
        sta fe_ip
        lda #>\prog
        sta fe_ip+1
        jsr fe_run
        .endm

; zeroes logical slot A and puts X in its first byte
fe_set_small .proc
        stx fe_tmp
        ldx #fe_pd
        jsr fe_slot_ptr
        ldy #31
        lda #0
-       sta (fe_pd),y
        dey
        bne -
        lda fe_tmp
        sta (fe_pd),y
        rts
        .pend

; copies 32 bytes from (fe_pa) to logical slot A
fe_load .proc
        ldx #fe_pd
        jsr fe_slot_ptr
        jmp fe_copy
        .pend

; copies logical slot A, reduced below p, to (fe_pb)
fe_store .proc
        ldx #fe_pd
        jsr fe_slot_ptr
        jsr fe_freeze
        ldy #31
-       lda (fe_pd),y
        sta (fe_pb),y
        dey
        bpl -
        rts
        .pend

; --- the pow chain of z^((p-5)/8) for the square roots of ed25519 --------
; slot FS_Z in, temporaries in slots 21-27
FS_Z    = 20
FS_Z2   = 21
FS_Z9   = 22
FS_Z11  = 23
FS_Z50  = 24
FS_Z10  = 25
FS_T    = 26
FS_Z100 = 27
FS_INV  = 28

fe_prog_pow_head        ; ends with FS_T = z^(2^250 - 1)
        .byte FE_SQR, FS_Z2, FS_Z, 0            ; 2
        .byte FE_SQN, FS_T, FS_Z2, 2            ; 8
        .byte FE_MUL, FS_Z9, FS_T, FS_Z         ; 9
        .byte FE_MUL, FS_Z11, FS_Z9, FS_Z2      ; 11
        .byte FE_SQR, FS_T, FS_Z11, 0           ; 22
        .byte FE_MUL, FS_T, FS_T, FS_Z9         ; 2^5 - 1
        .byte FE_SQN, FS_Z10, FS_T, 5
        .byte FE_MUL, FS_Z10, FS_Z10, FS_T      ; 2^10 - 1
        .byte FE_SQN, FS_T, FS_Z10, 10
        .byte FE_MUL, FS_T, FS_T, FS_Z10        ; 2^20 - 1
        .byte FE_SQN, FS_Z50, FS_T, 20
        .byte FE_MUL, FS_T, FS_Z50, FS_T        ; 2^40 - 1
        .byte FE_SQN, FS_T, FS_T, 10
        .byte FE_MUL, FS_Z50, FS_T, FS_Z10      ; 2^50 - 1
        .byte FE_SQN, FS_T, FS_Z50, 50
        .byte FE_MUL, FS_Z100, FS_T, FS_Z50     ; 2^100 - 1
        .byte FE_SQN, FS_T, FS_Z100, 100
        .byte FE_MUL, FS_T, FS_T, FS_Z100       ; 2^200 - 1
        .byte FE_SQN, FS_T, FS_T, 50
        .byte FE_MUL, FS_T, FS_T, FS_Z50        ; 2^250 - 1
        .byte 0

; --- inversion: slot FS_INV = slot FS_Z ^ -1 (0 if it is 0) ---------------
; Binary extended Euclid, u = z and v = p, with x1 z = u and x2 z = v
; (mod p) all along: the even one of u, v is halved (and its x), the
; smaller subtracted from the larger (and the x's), until u = v = 1. A
; few hundred thousand cycles, where z^(p-2) takes 7 million; but the time
; depends on z, so z is multiplied first by the random number in fe_mask
; (when it is not zero), the inverse by it again.

        .section kbss
fe_mask .fill 32                ; random (or zero: no masking)
        .send kbss

; x = x / 2 mod p (x < p): odd x is x + p = ((x - 19) mod 2^256) ^ 2^255
; halved; n = n / 2
gcd_half .macro n, x
        lsr \n+31
        .for k = 30, k >= 0, k -= 1
        ror \n+k
        .next
        lda \x
        lsr a
        bcc _even
        lda \x
        sbc #19                 ; (carry set)
        sta \x
        bcs _top
        ldx #1
-       lda \x,x
        sbc #0
        sta \x,x
        bcs _top
        inx
        cpx #32
        bne -
_top    lda \x+31
        eor #$80
        sta \x+31
_even   lsr \x+31
        .for k = 30, k >= 0, k -= 1
        ror \x+k
        .next
        .endm

; a = a - b (a > b), xa = xa - xb mod p: a borrow adds p, as -19 and the
; top bit flipped (no borrow then)
gcd_sub .macro a, b, xa, xb
        sec
        .for k = 0, k < 32, k += 1
        lda \a+k
        sbc \b+k
        sta \a+k
        .next
        sec
        .for k = 0, k < 32, k += 1
        lda \xa+k
        sbc \xb+k
        sta \xa+k
        .next
        bcs _done
        lda \xa
        sbc #18                 ; (carry clear: 19)
        sta \xa
        bcs _top
        ldx #1
-       lda \xa,x
        sbc #0
        sta \xa,x
        bcs _top
        inx
        bne -
_top    lda \xa+31
        eor #$80
        sta \xa+31
_done
        .endm

; (fe_pd) = (fe_pd) * fe_mask, when fe_mask is not zero
fe_masked .proc
        ldx #31
        lda #0
-       ora fe_mask,x
        dex
        bpl -
        tax
        beq +
        lda fe_pd
        sta fe_pa
        lda fe_pd+1
        sta fe_pa+1
        lda #<fe_mask
        sta fe_pb
        lda #>fe_mask
        sta fe_pb+1
        jmp fe_mul
+       rts
        .pend

fe_invert .proc
        lda #FS_Z
        ldx #fe_pd
        jsr fe_slot_ptr
        jsr fe_masked
        jsr fe_freeze
        ldy #31                 ; u = z, v = p, x1 = 1, x2 = 0
        lda #0
        sta fe_tmp
-       lda (fe_pd),y
        sta GCD_U,y
        ora fe_tmp
        sta fe_tmp
        lda #$ff
        sta GCD_V,y
        lda #0
        sta GCD_X1,y
        sta GCD_X2,y
        dey
        bpl -
        lda #$ed
        sta GCD_V
        lda #$7f
        sta GCD_V+31
        inc GCD_X1
        lda fe_tmp
        beq _done               ; z = 0: 0 (x2)
        jmp _hu
_vu     #gcd_sub GCD_V, GCD_U, GCD_X2, GCD_X1
_hv     #gcd_half GCD_V, GCD_X2
        lda GCD_V
        lsr a
        bcc _hv
        bcs _cmp
_uv     #gcd_sub GCD_U, GCD_V, GCD_X1, GCD_X2
_hu     lda GCD_U
        lsr a
        bcs _cmp
        #gcd_half GCD_U, GCD_X1
        jmp _hu
_cmp    ldx #31                 ; both odd: which is larger?
-       lda GCD_U,x
        cmp GCD_V,x
        bne +
        dex
        bpl -
        bmi _done               ; equal: 1
+       bcs _uv
        jmp _vu
_done   lda #FS_INV
        ldx #fe_pd
        jsr fe_slot_ptr
        ldy #31
-       lda GCD_X2,y
        sta (fe_pd),y
        dey
        bpl -
        jmp fe_masked
        .pend
