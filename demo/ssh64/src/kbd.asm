; SPDX-License-Identifier: GPL-2.0-or-later
; Copyright (C) 2026 SampoSoft - Francesco Sampoli
;
; Keyboard: the matrix scanned in the 60 Hz IRQ, keys translated to ASCII
; (or K_* codes) into a 16-byte queue; a held key repeats after 0.5 s,
; 15 times a second.
;
; ASCII the C64 keyboard lacks:
;   SHIFT : [   SHIFT ; ]   C= : {   C= ; }   £ \   SHIFT £ |
;   up-arrow ^  SHIFT up-arrow ~  left-arrow _  SHIFT left-arrow `
;   CTRL + letter: control code; RUN/STOP: ESC; INST/DEL: DEL ($7f)

K_UP    = $80
K_DOWN  = $81
K_LEFT  = $82
K_RIGHT = $83
K_HOME  = $84
K_CLR   = $85
K_INS   = $86
K_F1    = $87                   ; F1..F8: $87..$8e
K_MENU  = $8f                   ; C= + RUN/STOP: local menu

KB_SHIFT = 1
KB_CBM  = 2
KB_CTRL = 4

        .section bss
kb_queue .fill 16
kb_head .fill 1
kb_tail .fill 1
kb_last .fill 1                 ; key held down ($ff none)
kb_rep  .fill 1                 ; ticks to the next repeat
kb_time .fill 1                 ; ticks of the last key (entropy)
        .send bss

        .section boot           ; (run once: see ssh64.asm)
kbd_init .proc
        lda #0
        sta kb_head
        sta kb_tail
        lda #$ff
        sta kb_last
        rts
        .pend
        .send boot

; scans the matrix (IRQ): the first non-modifier key and the modifiers
kbd_scan .proc
        lda #$ff
        sta kb_code
        lda #0
        sta kb_mods
        sta $dc02+1             ; (port B input)
        lda #$ff
        sta $dc02               ; port A output
        lda #$fe
        sta kb_col
        ldx #0                  ; x = key code of column*8
_col    lda kb_col
        sta $dc00
        lda $dc01
        eor #$ff
        beq _next
        sta kb_bits
        ldy #0
-       lsr kb_bits
        bcc +
        txa                     ; code = x + row
        sty kb_tmp
        ora kb_tmp
        jsr _key
+       iny
        cpy #8
        bne -
_next   txa
        clc
        adc #8
        tax
        sec
        rol kb_col
        bcs _col
        lda #$ff                ; rows idle
        sta $dc00
        ; a key: new, or repeating
        ldx kb_code
        cpx #$ff
        bne +
        stx kb_last             ; none held
        rts
+       cpx kb_last
        beq _held
        stx kb_last
        lda #30
        sta kb_rep
        jmp _put
_held   dec kb_rep
        bne _ret
        lda #4
        sta kb_rep
_put    lda ticks
        sta kb_time
        lda kb_mods             ; translation table by modifiers
        and #KB_CTRL
        bne _ctrl
        lda kb_mods
        and #KB_CBM
        bne _cbm
        lda kb_mods
        and #KB_SHIFT
        bne _shift
        lda _plain,x
        jmp _q
_shift  lda _shifted,x
        jmp _q
_cbm    lda _cbmkeys,x
        jmp _q
_ctrl   lda _plain,x            ; letters -> 1..26, some punctuation
        cmp #'a'
        bcc +
        cmp #'z'+1
        bcs +
        and #$1f
        jmp _q
+       ldy #_nctl-1
-       cmp _ctlkey,y
        beq +
        dey
        bpl -
        rts
+       lda _ctlval,y
_q      beq _ret                ; nothing for this key
        ldy kb_head
        sta kb_queue,y
        iny
        tya
        and #15
        cmp kb_tail
        beq _ret                ; full
        sta kb_head
_ret    rts

_key    ; A = code: modifiers apart
        cmp #15
        beq _sh
        cmp #52
        beq _sh
        cmp #58
        beq _ct
        cmp #61
        beq _cb
        sta kb_code
        rts
_sh     lda #KB_SHIFT
        .byte $2c
_ct     lda #KB_CTRL
        .byte $2c
_cb     lda #KB_CBM
        ora kb_mods
        sta kb_mods
        rts

; by key code: column 0..7, row 0..7
_plain  .byte $7f, 13, K_RIGHT, K_F1+6, K_F1, K_F1+2, K_F1+4, K_DOWN
        .text "3wa4zse", 0
        .text "5rd6cftx"
        .text "7yg8bhuv"
        .text "9ij0mkon"
        .text "+pl-.:@,"
        .byte $5c
        .text "*;"
        .byte K_HOME, 0
        .text "="
        .byte $5e
        .text "/"
        .text "1"
        .byte $5f, 0
        .text "2 "
        .byte 0
        .text "q"
        .byte 27
_shifted .byte K_INS, 10, K_LEFT, K_F1+7, K_F1+1, K_F1+3, K_F1+5, K_UP
        .text "#WA$ZSE", 0
        .text "%RD&CFTX"
        .text "'YG(BHUV"
        .text ")IJ0MKON"
        .text "+PL->[@<"
        .byte $7c
        .text "*]"
        .byte K_CLR, 0
        .text "="
        .byte $7e
        .text "?"
        .text "!"
        .byte $60, 0
        .byte '"', ' ' 
        .byte 0
        .text "Q"
        .byte 27
_cbmkeys .byte $7f, 13, K_RIGHT, K_F1+6, K_F1, K_F1+2, K_F1+4, K_DOWN
        .text "3wa4zse", 0
        .text "5rd6cftx"
        .text "7yg8bhuv"
        .text "9ij0mkon"
        .byte '+', 'p', 'l', '-', '.', $7b, '@', ','
        .byte $7c, '*', $7d, K_HOME, 0, '=', $7e, '/'
        .byte '1', $60, 0, '2', ' ', 0, 'q', K_MENU
_ctlkey .byte ':', ';', '@', $5c, $5e, $5f, '2', '3', '4', '5', '6', '7', '8'
_nctl   = * - _ctlkey
_ctlval .byte 27, 29, 0, 28, 30, 31, 0, 27, 28, 29, 30, 31, $7f
        .pend

; next key in A; carry set if none
kbd_get .proc
        ldx kb_tail
        cpx kb_head
        bne +
        sec
        rts
+       lda kb_queue,x
        inx
        txa
        and #15
        sta kb_tail
        lda kb_queue-1,x        ; (x was tail + 1)
        clc
        rts
        .pend
