; SPDX-License-Identifier: GPL-2.0-or-later
; Copyright (C) 2026 SampoSoft - Francesco Sampoli
;
; Ed25519 signature verification (RFC 8032), as the ref10 code that
; OpenSSH uses: R' = [h](-A) + [S]B with h = SHA-512(R || A || M) mod L,
; accepted when R' encodes to the R of the signature.
;
; Both scalars through combs: that of B is a table, that of -A is built
; once per host key (ed_comb_build) and kept on disk. Points in extended
; coordinates (X:Y:Z:T), additions and doublings from ref10.
;
;   ed_pub (32), ed_sig (64), ed_msg / ed_msglen: message
;   ed_comb_build, then ed25519_verify_comb -> carry clear if good

; logical slots
E_XX    = 0
E_YY    = 1
E_B2    = 2
E_T0    = 3
E_A     = 4
E_T1    = 5
E_B     = 6
E_C     = 7
E_D     = 8
E_U     = 9
E_V     = 10
E_V3    = 11
E_CHK   = 12
E_DC    = 16                    ; d
E_D2    = 17                    ; 2d
E_SQ    = 18                    ; sqrt(-1)
E_ZERO  = 19
E_X     = 30                    ; current point
E_Y     = 31
E_Z     = 32
E_T     = 33
E_E     = 34                    ; completed point (ref10 p1p1)
E_F     = 35
E_G     = 36
E_H     = 37
E_BP    = 38                    ; affine point being added: y+x
E_BM    = 39                    ; y-x (mapped on a comb entry, or loaded)
E_BT    = 40                    ; 2dxy
E_SAVE  = 13                    ; 13, 14, 15, 29: a point kept aside
; slots 41-104 while the comb of -A is built:
E_P0    = 41                    ; 2^(43 r) (-A), r = 0..5, cached (4 slots each),
E_Q0    = 41                    ; then the products of the Z's of the 32 entries
E_2P0   = 73                    ; 2^(43 r + 1) (-A), r = 1..5, cached
E_TX    = 93                    ; an entry: X, Y, Z (then y+x, y-x, 2dxy)
E_TY    = 94
E_TZ    = 95
; mapped names
E_QK    = 96                    ; product of the first k Z's
E_QK1   = 97                    ; of the first k-1
E_AIP   = 101                   ; cached point in use: Y+X
E_AIM   = 102                   ; Y-X
E_AIZ   = 103                   ; Z
E_AIT   = 104                   ; 2dT

ED_COLS = 43                    ; the combs: 6 teeth 43 bits apart
ED_TALEN = 32 * 96              ; 32 entries

        .section bssl
ed_ta   .fill ED_TALEN          ; the comb of -A
        .send bssl

        .section ktail
ed_pub  .fill 32
ed_sig  .fill 64
ed_msg  .fill 2                 ; message pointer
ed_msglen .fill 2
ed_h    .fill 64                ; SHA-512, then h mod L (32)
        .send ktail

; --- field programs --------------------------------------------------------
ed_prog_dbl                     ; (X:Y:Z) doubled -> E F G H
        .byte FE_SQR, E_XX, E_X, 0
        .byte FE_SQR, E_YY, E_Y, 0
        .byte FE_SQR, E_B2, E_Z, 0
        .byte FE_ADD, E_B2, E_B2, E_B2
        .byte FE_ADD, E_T0, E_X, E_Y
        .byte FE_SQR, E_T0, E_T0, 0
        .byte FE_ADD, E_H, E_YY, E_XX
        .byte FE_SUB, E_E, E_T0, E_H
        .byte FE_SUB, E_G, E_YY, E_XX
        .byte FE_SUB, E_F, E_B2, E_G
        .byte 0
ed_prog_p3                      ; E F G H -> X Y Z T
        .byte FE_MUL, E_T, E_E, E_H
ed_prog_p2                      ; E F G H -> X Y Z
        .byte FE_MUL, E_X, E_E, E_F
        .byte FE_MUL, E_Y, E_H, E_G
        .byte FE_MUL, E_Z, E_G, E_F
        .byte 0
ed_prog_add                     ; X Y Z T + cached -> E F G H
        .byte FE_ADD, E_T0, E_Y, E_X
        .byte FE_MUL, E_A, E_T0, E_AIP
        .byte FE_SUB, E_T1, E_Y, E_X
        .byte FE_MUL, E_B, E_T1, E_AIM
        .byte FE_MUL, E_C, E_AIT, E_T
        .byte FE_MUL, E_D, E_Z, E_AIZ
        .byte FE_ADD, E_D, E_D, E_D
        .byte FE_SUB, E_E, E_A, E_B
        .byte FE_ADD, E_H, E_A, E_B
        .byte FE_ADD, E_G, E_D, E_C
        .byte FE_SUB, E_F, E_D, E_C
        .byte 0
ed_prog_sub                     ; X Y Z T - cached -> E F G H
        .byte FE_ADD, E_T0, E_Y, E_X
        .byte FE_MUL, E_A, E_T0, E_AIM
        .byte FE_SUB, E_T1, E_Y, E_X
        .byte FE_MUL, E_B, E_T1, E_AIP
        .byte FE_MUL, E_C, E_AIT, E_T
        .byte FE_MUL, E_D, E_Z, E_AIZ
        .byte FE_ADD, E_D, E_D, E_D
        .byte FE_SUB, E_E, E_A, E_B
        .byte FE_ADD, E_H, E_A, E_B
        .byte FE_SUB, E_G, E_D, E_C
        .byte FE_ADD, E_F, E_D, E_C
        .byte 0
ed_prog_madd                    ; X Y Z T + affine multiple of B
        .byte FE_ADD, E_T0, E_Y, E_X
        .byte FE_MUL, E_A, E_T0, E_BP
        .byte FE_SUB, E_T1, E_Y, E_X
        .byte FE_MUL, E_B, E_T1, E_BM
        .byte FE_MUL, E_C, E_BT, E_T
        .byte FE_ADD, E_D, E_Z, E_Z
        .byte FE_SUB, E_E, E_A, E_B
        .byte FE_ADD, E_H, E_A, E_B
        .byte FE_ADD, E_G, E_D, E_C
        .byte FE_SUB, E_F, E_D, E_C
        .byte 0
ed_prog_msub                    ; X Y Z T - affine multiple of B
        .byte FE_ADD, E_T0, E_Y, E_X
        .byte FE_MUL, E_A, E_T0, E_BM
        .byte FE_SUB, E_T1, E_Y, E_X
        .byte FE_MUL, E_B, E_T1, E_BP
        .byte FE_MUL, E_C, E_BT, E_T
        .byte FE_ADD, E_D, E_Z, E_Z
        .byte FE_SUB, E_E, E_A, E_B
        .byte FE_ADD, E_H, E_A, E_B
        .byte FE_SUB, E_G, E_D, E_C
        .byte FE_ADD, E_F, E_D, E_C
        .byte 0
ed_prog_cache                   ; X Y Z T -> cached (mapped slots)
        .byte FE_ADD, E_AIP, E_Y, E_X
        .byte FE_SUB, E_AIM, E_Y, E_X
        .byte FE_CPY, E_AIZ, E_Z, 0
        .byte FE_MUL, E_AIT, E_T, E_D2
        .byte 0
ed_prog_dec1                    ; y -> u = y^2 - 1, v = dy^2 + 1, FS_Z = u v^7
        .byte FE_SQR, E_U, E_Y, 0
        .byte FE_MUL, E_V, E_U, E_DC
        .byte FE_SUB, E_U, E_U, E_Z
        .byte FE_ADD, E_V, E_V, E_Z
        .byte FE_SQR, E_V3, E_V, 0
        .byte FE_MUL, E_V3, E_V3, E_V
        .byte FE_SQR, E_X, E_V3, 0
        .byte FE_MUL, E_X, E_X, E_V
        .byte FE_MUL, FS_Z, E_X, E_U
        .byte 0
ed_prog_dec2                    ; x = u v^3 (u v^7)^((p-5)/8), CHK = v x^2
        .byte FE_SQN, FS_T, FS_T, 2
        .byte FE_MUL, E_X, FS_T, FS_Z
        .byte FE_MUL, E_X, E_X, E_V3
        .byte FE_MUL, E_X, E_X, E_U
        .byte FE_SQR, E_CHK, E_X, 0
        .byte FE_MUL, E_CHK, E_CHK, E_V
        .byte FE_SUB, E_T0, E_CHK, E_U
        .byte FE_ADD, E_T1, E_CHK, E_U
        .byte 0
ed_prog_sqm1
        .byte FE_MUL, E_X, E_X, E_SQ
        .byte 0
ed_prog_neg
        .byte FE_SUB, E_X, E_ZERO, E_X
        .byte 0
ed_prog_t
        .byte FE_MUL, E_T, E_X, E_Y
        .byte 0
ed_prog_enc1
        .byte FE_CPY, FS_Z, E_Z, 0
        .byte 0
ed_prog_enc2
        .byte FE_MUL, E_X, E_X, FS_INV
        .byte FE_MUL, E_Y, E_Y, FS_INV
        .byte 0

; --- helpers ---------------------------------------------------------------

; logical slot A, reduced in place: Z set if zero, carry = lowest bit
ed_freeze_test .proc
        ldx #fe_pd
        jsr fe_slot_ptr
        jsr fe_freeze
        ldy #31
        lda #0
-       ora (fe_pd),y
        dey
        bpl -
        sta fe_tmp
        iny
        lda (fe_pd),y
        lsr a                   ; carry = lowest bit
        lda fe_tmp              ; Z = all zero
        rts
        .pend

; loads 32 bytes at (ed_p) into logical slot A (the tables are in the RAM
; under the I/O: banked out meanwhile)
ed_load .proc
        ldx ed_p
        stx fe_pa
        ldx ed_p+1
        stx fe_pa+1
        ldx #$34
        stx $01
        jsr fe_load
        lda #$35
        sta $01
        rts
        .pend

ed_const .macro slot, addr
        lda #<\addr
        sta ed_p
        lda #>\addr
        sta ed_p+1
        lda #\slot
        jsr ed_load
        .endm

; the 3 numbers at (ed_p) into logical slots A, A+1, A+2
ed_load3 .proc
        sta ed_v
        jsr ed_load
        jsr ed_next
        inc ed_v
        lda ed_v
        jsr ed_load
        jsr ed_next
        inc ed_v
        lda ed_v
        jmp ed_load
        .pend

; logical slots A, A+1, A+2 to (ed_p) (in the RAM, not under the I/O)
ed_store3 .proc
        sta ed_v
        lda #3
        sta ed_b
-       lda ed_p
        sta fe_pd
        lda ed_p+1
        sta fe_pd+1
        lda ed_v
        ldx #fe_pa
        jsr fe_slot_ptr
        jsr fe_copy
        jsr ed_next
        inc ed_v
        dec ed_b
        bne -
        rts
        .pend

; ed_p += 32
ed_next .proc
        lda ed_p
        clc
        adc #32
        sta ed_p
        bcc +
        inc ed_p+1
+       rts
        .pend

; --- scalars ---------------------------------------------------------------

; ed_h (64 bytes) = ed_h mod L, the result in its first 32 bytes: its bits
; from the top into r (FE_R: r < 2L < 2^253), r -= L whenever r >= L. L's
; top byte is $10, bytes 16-30 are zero: the comparison is decided by the
; top byte but once in 16 times, the subtraction takes the low 16 bytes and
; a borrow.
ED_R    = FE_R

ed_reduce .proc
        lda #0
        ldx #31
-       sta ED_R,x
        dex
        bpl -
        ldx #63
        stx ed_i
_byte   lda #8
        sta ed_q
_bit    ldx ed_i
        asl ed_h,x              ; r = 2r + the next bit
        .for k = 0, k < 32, k += 1
        rol ED_R+k
        .next
        lda ED_R+31             ; r >= L ?
        cmp #$10
        bcc _next
        bne _sub
        ldx #30
-       lda ED_R,x
        cmp ed_l,x
        bne +
        dex
        bpl -
        bmi _sub                ; equal
+       bcc _next
_sub    sec
        .for k = 0, k < 16, k += 1
        lda ED_R+k
        sbc ed_l+k
        sta ED_R+k
        .next
        ldx #16                 ; the borrow through the zero bytes
-       bcs _top
        lda ED_R,x
        sbc #0
        sta ED_R,x
        inx
        txa                     ; (the carry stays)
        eor #31
        bne -
_top    lda ED_R+31
        sbc #$10
        sta ED_R+31
_next   dec ed_q
        bne _bit
        dec ed_i
        bpl _byte
        ldx #31
-       lda ED_R,x
        sta ed_h,x
        dex
        bpl -
        rts
        .pend

; carry clear if the 32 bytes at (ed_p) are below L
ed_below_l .proc
        ldy #31
-       lda (ed_p),y
        cmp ed_l,y
        bne +
        dey
        bpl -
        sec                     ; equal
        rts
+       rts                     ; carry: >= L
        .pend

; --- verification ------------------------------------------------------------

; decodes ed_pub into -A in X Y Z T; carry set if it is not a point
ed_decode .proc
        #ed_const E_Y, ed_pub
        lda fe_slots+32*E_Y+31
        and #$7f
        sta fe_slots+32*E_Y+31
        lda #E_Z
        ldx #1
        jsr fe_set_small
        #fe_prog ed_prog_dec1
        #fe_prog fe_prog_pow_head
        #fe_prog ed_prog_dec2
        lda #E_T0               ; v x^2 = u ?
        jsr ed_freeze_test
        beq _root
        lda #E_T1               ; v x^2 = -u: x = x sqrt(-1)
        jsr ed_freeze_test
        bne _bad
        #fe_prog ed_prog_sqm1
_root   lda #E_X                ; -A: x of the sign opposite to the bit
        jsr ed_freeze_test
        php
        lda #0
        rol a
        sta ed_q
        plp
        bne +
        lda ed_pub+31           ; x = 0 with the sign bit: not a point
        bmi _bad
+       lda ed_pub+31
        rol a
        lda #0
        rol a
        eor ed_q
        bne +                   ; x of the parity of the bit is A's: negate
        #fe_prog ed_prog_neg
+       #fe_prog ed_prog_t
        clc
        rts
_bad    sec
        rts
        .pend

; the checks and the hash of both verifications: S < L, h = SHA-512(R ||
; A || M) mod L in ed_h; carry set if S is too big
ed_prologue .proc
        jsr fe_map_reset
        lda #<(ed_sig+32)       ; S < L
        sta ed_p
        lda #>(ed_sig+32)
        sta ed_p+1
        jsr ed_below_l
        bcc +
        rts
+       ; h = SHA-512(R || A || M) mod L
        jsr sha512_init
        lda #<ed_sig
        ldx #>ed_sig
        ldy #32
        jsr _hash
        lda #<ed_pub
        ldx #>ed_pub
        ldy #32
        jsr _hash
        lda ed_msg
        sta sha_p
        lda ed_msg+1
        sta sha_p+1
        lda ed_msglen
        sta sha_len
        lda ed_msglen+1
        sta sha_len+1
        jsr sha512_update
        jsr sha512_final
        ldx #63
-       lda sha512_out,x
        sta ed_h,x
        dex
        bpl -
        jsr ed_reduce
        clc
        rts
_hash   sta sha_p
        stx sha_p+1
        sty sha_len
        lda #0
        sta sha_len+1
        jmp sha512_update
        .pend

; R (X Y Z) encoded and compared with the R of the signature: carry clear
; if they are equal
ed_finish .proc
        #fe_prog ed_prog_enc1
        jsr fe_invert
        #fe_prog ed_prog_enc2
        lda #E_X
        jsr ed_freeze_test
        lda #0
        ror a                   ; parity of x in bit 7
        sta ed_q
        lda #E_Y
        jsr ed_freeze_test
        ldy #31
        lda (fe_pd),y
        ora ed_q
        cmp ed_sig+31
        bne _no
        dey
-       lda (fe_pd),y
        cmp ed_sig,y
        bne _no
        dey
        bpl -
        clc
        rts
_no     sec
        rts
        .pend


; --- the combs ------------------------------------------------------------------
; Signed combs of 6 teeth 43 bits apart: entry j (0..31) of the comb of a
; point P is P0 + e1 P1 + ... + e5 P5, Pr = 2^(43 r) P, er = +1 if bit r-1
; of j is set, else -1. A scalar k is made odd, k + L if it is even (L P
; is the neutral point for the points of the base's group), and then
; k = sum of (2 bi - 1) 2^i over 258 bits with b = (k - 1) / 2 + 2^257.
; Column c (42 down to 0) adds (2 b_c - 1) times the entry whose bit r-1
; says whether b_(43 r + c) equals b_c:
;   Q = 2 Q +- comb[j]
; 42 doublings and 43 additions, none of them skipped. The comb of B is a
; table; that of -A (ed_ta) is built once per host key and kept on disk.

        .section ktail
ed_kb   .fill 66                ; the b of two scalars, 33 bytes each
ed_col  .fill 1
ed_off  .fill 1
ed_m    .fill 1                 ; comb building: Gray code of the step
ed_gray .fill 1                 ; of the step before
ed_pt   .fill 1                 ; point counter
        .send ktail

; ed_kb + X (0 or 33) = b of the scalar at (ed_p) (32 bytes, below 2^255)
ed_recode .proc
        stx ed_off
        ldy #0
        lda (ed_p),y
        lsr a
        lda #0                  ; odd: + 0
        bcs +
        lda #$ff                ; even: + L
+       sta ed_q
        lda #32
        sta ed_b
        clc
-       lda ed_l,y
        and ed_q
        adc (ed_p),y
        sta ed_kb,x
        inx
        iny
        dec ed_b                ; (the carry goes on)
        bne -
        lda #2                  ; 2^257
        sta ed_kb,x
        ldy #31                 ; (k - 1) / 2: k shifted, its bit 0 is 1
        dex
        lsr ed_kb,x
-       dex
        ror ed_kb,x
        dey
        bne -
        rts
        .pend

; bit 43 r + ed_col of the b at ed_kb + ed_off, in the carry
ed_bit  .macro r
        lda ed_col              ; = byte 5 r + (3 r + c) / 8, bit (3 r + c) % 8
        clc
        adc #3*\r
        tax
        and #7
        tay
        txa
        lsr a
        lsr a
        lsr a
        clc
        adc #5*\r
        adc ed_off
        tax
        lda ed_kb,x
        and ed_bitm,y
        cmp #1
        .endm

ed_bitm .byte 1, 2, 4, 8, 16, 32, 64, 128

; column ed_col of the b at ed_kb + Y: A = the entry, carry set to add it,
; clear to subtract it
ed_digit .proc
        sty ed_off
        lda #0
        sta ed_q
        .for r = 5, r >= 1, r -= 1
        #ed_bit r
        rol ed_q
        .next
        #ed_bit 0
        lda ed_q                ; bits equal to b_c
        bcs +
        eor #31
+       rts
        .pend

; ed_p = entry A of the comb of B (_b) or of -A (_a)
ed_entry .proc
_b      ldx #<ed_comb
        ldy #>ed_comb
        bne +
_a      ldx #<ed_ta
        ldy #>ed_ta
+       stx ed_p
        sty ed_p+1
        tax
        lda _lo,x
        clc
        adc ed_p
        sta ed_p
        lda _hi,x
        adc ed_p+1
        sta ed_p+1
        rts
_lo     .byte <(96 * range(32))
_hi     .byte >(96 * range(32))
        .pend

; Q (X Y Z T) +- entry A of the comb of B (_b) or of -A (_a), carry set
; to add
ed_addcomb .proc
_b      php
        jsr ed_entry._b
        jmp +
_a      php
        jsr ed_entry._a
+       lda #E_BP
        jsr ed_load3
        plp
        bcc +
        #fe_prog ed_prog_madd
        rts
+       #fe_prog ed_prog_msub
        rts
        .pend

; X Y Z T = the neutral point, first column next
ed_start .proc
        lda #E_X
        ldx #0
        jsr fe_set_small
        lda #E_T
        ldx #0
        jsr fe_set_small
        lda #E_Y
        ldx #1
        jsr fe_set_small
        lda #E_Z
        ldx #1
        jsr fe_set_small
        lda #ED_COLS-1
        sta ed_col
        rts
        .pend

; doubles Q (X Y Z) into X Y Z T, but not before the first column
ed_colstep .proc
        lda ed_col
        cmp #ED_COLS-1
        beq +
        #fe_prog ed_prog_dbl
        #fe_prog ed_prog_p3
+       rts
        .pend

; X Y Z (p2) = [scalar at (ed_p)] B
ed_basemul .proc
        ldx #0
        jsr ed_recode
        jsr fe_map_reset
        jsr ed_start
-       jsr ed_colstep
        ldy #0
        jsr ed_digit
        jsr ed_addcomb._b
        #fe_prog ed_prog_p2
        dec ed_col
        bpl -
        rts
        .pend

; X25519 public key: x25519_out = u of [x25519_k] B, u = (Z + Y) / (Z - Y)
; (the Edwards curve and Curve25519 share the base point: u = 9)
ed_prog_mont1
        .byte FE_SUB, FS_Z, E_Z, E_Y
        .byte 0
ed_prog_mont2
        .byte FE_ADD, E_T0, E_Z, E_Y
        .byte FE_MUL, E_X, E_T0, FS_INV
        .byte 0

x25519_base .proc
        lda x25519_k            ; clamp (in place)
        and #$f8
        sta x25519_k
        lda x25519_k+31
        and #$7f
        ora #$40
        sta x25519_k+31
        lda #<x25519_k
        sta ed_p
        lda #>x25519_k
        sta ed_p+1
        jsr ed_basemul
        #fe_prog ed_prog_mont1
        jsr fe_invert
        #fe_prog ed_prog_mont2
        lda #<x25519_out
        sta fe_pb
        lda #>x25519_out
        sta fe_pb+1
        lda #E_X
        jmp fe_store
        .pend

; the verification with the comb of -A in ed_ta: [S]B + [h](-A), 42
; doublings; carry clear if the signature is good
ed25519_verify_comb .proc
        jsr ed_prologue
        bcc +
        rts
+       lda #<(ed_sig+32)       ; S
        sta ed_p
        lda #>(ed_sig+32)
        sta ed_p+1
        ldx #0
        jsr ed_recode
        lda #<ed_h              ; h
        sta ed_p
        lda #>ed_h
        sta ed_p+1
        ldx #33
        jsr ed_recode
        jsr ed_start
-       jsr ed_colstep
        ldy #0
        jsr ed_digit
        jsr ed_addcomb._b
        #fe_prog ed_prog_p3
        ldy #33
        jsr ed_digit
        jsr ed_addcomb._a
        #fe_prog ed_prog_p2
        dec ed_col
        bpl -
        jmp ed_finish
        .pend

; --- building the comb of -A ------------------------------------------------------
; The points Pr = 2^(43 r) (-A) (215 doublings), their sum entry 31, then
; the other entries in Gray code order, each one step of +-2 Pr from the
; one before; all made affine with one inversion (Montgomery's trick: the
; products of the Z's).

; maps X logical slots from A onto the physical slots from Y
ed_map  .proc
        sta ed_q
        sty _y+1
        txa
        clc
        adc ed_q
        sta _end+1
        ldx ed_q
_y      lda #0
-       sta fe_map,x
        clc
        adc #1
        inx
_end    cpx #0
        bne -
        rts
        .pend

; E_AIP..E_AIT on the cached point at physical slot A
ed_map_ai .proc
        tay
        lda #E_AIP
        ldx #4
        jmp ed_map
        .pend

; E_QK, E_QK1 on the products of the Z's of entries 0..A and 0..A-1
ed_map_q .proc
        clc
        adc #E_Q0
        tay
        lda #E_QK
        ldx #1
        jsr ed_map
        dey
        tya
        tay
        lda #E_QK1
        ldx #1
        jmp ed_map
        .pend

ed_prog_save
        .byte FE_CPY, E_SAVE, E_X, 0
        .byte FE_CPY, E_SAVE+1, E_Y, 0
        .byte FE_CPY, E_SAVE+2, E_Z, 0
        .byte FE_CPY, 29, E_T, 0
        .byte 0
ed_prog_rest
        .byte FE_CPY, E_X, E_SAVE, 0
        .byte FE_CPY, E_Y, E_SAVE+1, 0
        .byte FE_CPY, E_Z, E_SAVE+2, 0
        .byte FE_CPY, E_T, 29, 0
        .byte 0
ed_prog_q1
        .byte FE_CPY, E_QK, E_TZ, 0
        .byte 0
ed_prog_qk
        .byte FE_MUL, E_QK, E_QK1, E_TZ
        .byte 0
ed_prog_qinv
        .byte FE_CPY, FS_Z, E_QK, 0
        .byte 0
ed_prog_zinv                    ; E_A = 1 / Z_k, FS_INV = 1 / (Z_0 .. Z_k-1)
        .byte FE_MUL, E_A, FS_INV, E_QK1
        .byte FE_MUL, FS_INV, FS_INV, E_TZ
        .byte 0
ed_prog_zinv1
        .byte FE_CPY, E_A, FS_INV, 0
        .byte 0
ed_prog_niels                   ; X, Y, (Z) -> y+x, y-x, 2dxy with 1/Z in E_A
        .byte FE_MUL, E_B, E_TX, E_A
        .byte FE_MUL, E_C, E_TY, E_A
        .byte FE_ADD, E_TX, E_C, E_B
        .byte FE_SUB, E_TY, E_C, E_B
        .byte FE_MUL, E_D, E_B, E_C
        .byte FE_MUL, E_TZ, E_D, E_D2
        .byte 0

; E_AIP..E_AIT on the cached point Pr (A = r) or 2 Pr (_2p)
ed_map_p .proc
        asl a
        asl a
        adc #E_P0
        jmp ed_map_ai
_2p     asl a
        asl a
        adc #E_2P0-4
        jmp ed_map_ai
        .pend

; the comb of -A (ed_pub) into ed_ta; carry set if ed_pub is not a point
ed_comb_build .proc
        jsr fe_map_reset
        #ed_const E_DC, ed_d
        #ed_const E_D2, ed_d2
        #ed_const E_SQ, ed_sqrtm1
        lda #E_ZERO
        ldx #0
        jsr fe_set_small
        jsr ed_decode
        bcc +
        rts
+       lda #0                  ; r
        sta ed_pt
_chain  lda ed_pt               ; Pr cached
        jsr ed_map_p
        #fe_prog ed_prog_cache
        lda ed_pt
        cmp #5
        beq _last
        lda #ED_COLS            ; 43 doublings to P(r+1)
        sta ed_col
        #fe_prog ed_prog_dbl
        lda ed_pt
        beq _p2                 ; 2 P0 is not needed
        #fe_prog ed_prog_p3
        lda ed_pt
        jsr ed_map_p._2p
        #fe_prog ed_prog_cache
        jmp +
_p2     #fe_prog ed_prog_p2
+       dec ed_col
-       #fe_prog ed_prog_dbl
        dec ed_col
        beq +
        #fe_prog ed_prog_p2
        jmp -
+       #fe_prog ed_prog_p3
        inc ed_pt
        jmp _chain
_last   #fe_prog ed_prog_save   ; 2 P5 cached, P5 kept
        #fe_prog ed_prog_dbl
        #fe_prog ed_prog_p3
        lda #5
        jsr ed_map_p._2p
        #fe_prog ed_prog_cache
        #fe_prog ed_prog_rest
        lda #4                  ; entry 31: P5 + P4 + ... + P0
        sta ed_pt
-       lda ed_pt
        jsr ed_map_p
        #fe_prog ed_prog_add
        #fe_prog ed_prog_p3
        dec ed_pt
        bpl -
        lda #0
        sta ed_gray
        lda #31
        jsr _put
        lda #1                  ; step i: entry 31 ^ gray(i)
        sta ed_col
_step   lda ed_col
        lsr a
        eor ed_col
        sta ed_m                ; gray(i)
        eor ed_gray             ; the bit that changes: r - 1
        ldx #0
-       lsr a
        bcs +
        inx
        bne -
+       inx
        txa
        jsr ed_map_p._2p
        lda ed_m
        eor ed_gray
        and ed_m                ; set in gray(i): cleared in the entry
        bne _sub
        #fe_prog ed_prog_add
        jmp +
_sub    #fe_prog ed_prog_sub
+       #fe_prog ed_prog_p3
        lda ed_m
        sta ed_gray
        eor #31
        jsr _put
        inc ed_col
        lda ed_col
        cmp #32
        bne _step
        ; products of the Z's
        lda #0
        sta ed_col
-       lda ed_col
        jsr _get
        lda ed_col
        jsr ed_map_q
        lda ed_col
        bne +
        #fe_prog ed_prog_q1
        jmp ++
+       #fe_prog ed_prog_qk
+       inc ed_col
        lda ed_col
        cmp #32
        bne -
        #fe_prog ed_prog_qinv   ; (E_QK is the last)
        jsr fe_invert
        lda #31                 ; from the last entry down
        sta ed_col
-       lda ed_col
        jsr _get
        lda ed_col
        beq +
        jsr ed_map_q
        #fe_prog ed_prog_zinv
        jmp ++
+       #fe_prog ed_prog_zinv1
+       #fe_prog ed_prog_niels
        lda ed_col
        jsr ed_entry._a
        lda #E_TX
        jsr ed_store3
        dec ed_col
        bpl -
        clc
        rts

_put    jsr ed_entry._a         ; entry A = X Y Z
        lda #E_X
        jmp ed_store3
_get    jsr ed_entry._a         ; E_TX E_TY E_TZ = entry A
        lda #E_TX
        jmp ed_load3
        .pend
