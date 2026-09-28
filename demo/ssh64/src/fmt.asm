; SPDX-License-Identifier: GPL-2.0-or-later
; Copyright (C) 2026 SampoSoft - Francesco Sampoli
;
; Printing through putc: puts (inline string), puthex, putdec, putip.

; prints the string that follows the jsr (ends with 0)
puts    .proc
        pla
        sta str_p
        pla
        sta str_p+1
-       inc str_p
        bne +
        inc str_p+1
+       ldy #0
        lda (str_p),y
        beq +
        jsr putc
        jmp -
+       lda str_p+1
        pha
        lda str_p
        pha
        rts
        .pend

; A as two hex digits
puthex  .proc
        pha
        lsr a
        lsr a
        lsr a
        lsr a
        jsr _dig
        pla
        and #15
_dig    cmp #10
        bcc +
        adc #6
+       adc #'0'
        jmp putc
        .pend

; A as a decimal number
putdec  .proc
        ldx #0                  ; hundreds
-       cmp #100
        bcc +
        sbc #100
        inx
        bne -
+       stx sys_t
        ldx #0                  ; tens
-       cmp #10
        bcc +
        sbc #10
        inx
        bne -
+       stx sys_t+1
        pha                     ; units
        lda sys_t
        beq +
        ora #'0'
        jsr putc
        jmp _t
+       lda sys_t+1
        beq _u
_t      lda sys_t+1
        ora #'0'
        jsr putc
_u      pla
        ora #'0'
        jmp putc
        .pend

; the 4 bytes at X (low) / Y (high) as a.b.c.d
putip   .proc
        stx str_p
        sty str_p+1
        lda #0
        sta _i
-       ldy _i
        lda (str_p),y
        jsr putdec
        inc _i
        lda _i
        cmp #4
        beq +
        lda #'.'
        jsr putc
        jmp -
+       rts
        .section bss
_i      .fill 1
        .send bss
        .pend
