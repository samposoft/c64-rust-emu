; SPDX-License-Identifier: GPL-2.0-or-later
; Copyright (C) 2026 SampoSoft - Francesco Sampoli
;
; A plain screen for the tests (the client has the terminal): cls, putc.

; clears the screen, cursor home
cls     .proc
        ldx #0
-       lda #' '
        sta SCREEN,x
        sta SCREEN+$100,x
        sta SCREEN+$200,x
        sta SCREEN+$2e8,x
        lda #13                 ; light green
        sta COLORS,x
        sta COLORS+$100,x
        sta COLORS+$200,x
        sta COLORS+$2e8,x
        inx
        bne -
        lda #0
        sta cur_x
        sta cur_y
        rts
        .pend

; a character on the screen (ASCII, 13 = new line), scrolling at the end
putc    .proc
        cmp #13
        beq _nl
        cmp #10
        beq _rts
        pha
        ldy cur_y
        lda _rowlo,y
        sta scr_p
        lda _rowhi,y
        sta scr_p+1
        ldy cur_x
        pla
        sta (scr_p),y
        inc cur_x
        lda cur_x
        cmp #40
        bcc _rts
_nl     lda #0
        sta cur_x
        inc cur_y
        lda cur_y
        cmp #25
        bcc _rts
        dec cur_y
        ldx #0                  ; scroll up one line
-       lda SCREEN+40,x
        sta SCREEN,x
        lda SCREEN+40+240,x
        sta SCREEN+240,x
        lda SCREEN+40+480,x
        sta SCREEN+480,x
        lda SCREEN+40+720,x
        sta SCREEN+720,x
        inx
        cpx #240
        bne -
        lda #' '
        ldx #39
-       sta SCREEN+960,x
        dex
        bpl -
_rts    rts
_rowlo  .byte <(SCREEN + 40 * range(25))
_rowhi  .byte >(SCREEN + 40 * range(25))
        .pend

