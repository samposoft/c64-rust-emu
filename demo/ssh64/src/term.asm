; SPDX-License-Identifier: GPL-2.0-or-later
; Copyright (C) 2026 SampoSoft - Francesco Sampoli
;
; Terminal: a VT100 (with the ANSI colours of an xterm) on 40 x 25 cells.
;
; A cell is the ASCII code of the character, +128 for reverse video (the
; font has the reversed glyphs there), and its colour. The C64 has one
; background colour for the whole text screen, so a coloured background
; is shown as reverse video in that colour. The cursor is the cell
; reversed. Characters outside ASCII (UTF-8) come out as '?', the DEC line
; drawing set as - | +.
;
;   term_init, term_out (A = byte from the server), term_cursor_on / _off
;   term_reply / term_rlen: answers to the server (DSR, DA)

T_COLS  = 40
T_ROWS  = 25

; parser states
TS_NORM = 0
TS_ESC  = 1
TS_CSI  = 2
TS_CHARSET = 3
TS_OSC  = 4
TS_UTF8 = 5
TS_OSCESC = 6
TS_HASH = 7

        .section bss
t_x     .fill 1
t_y     .fill 1
t_wrap  .fill 1                 ; pending wrap (cursor past the last column)
t_color .fill 1                 ; foreground (C64 colour)
t_rev   .fill 1                 ; $80 when reverse
t_bold  .fill 1
t_fg    .fill 1                 ; ANSI foreground 0-7, 9 = default
t_bg    .fill 1                 ; ANSI background, 9 = default
t_inv   .fill 1                 ; SGR 7
t_top   .fill 1                 ; scrolling region
t_bot   .fill 1
t_state .fill 1
t_par   .fill 8                 ; CSI parameters
t_np    .fill 1                 ; parameters so far (index of the current)
t_priv  .fill 1                 ; '?' or other private marker
t_utf   .fill 1                 ; continuation bytes to skip
t_g0    .fill 1                 ; 1: DEC line drawing
t_sx    .fill 1                 ; saved cursor
t_sy    .fill 1
t_scol  .fill 1
t_srev  .fill 1
t_curon .fill 1                 ; cursor drawn
t_curvis .fill 1                ; cursor wanted (DECTCEM)
t_appkeys .fill 1               ; DECCKM: cursor keys as ESC O x
t_autowrap .fill 1
term_reply .fill 16
term_rlen .fill 1
t_tmp   .fill 8
        .send bss

T_DEFCOL = 15                   ; light grey

term_init .proc
        lda #0
        sta t_state
        sta t_g0
        sta t_appkeys
        sta term_rlen
        sta t_curon
        lda #1
        sta t_curvis
        sta t_autowrap
        jsr t_sgr0
        lda #0
        sta t_top
        lda #T_ROWS-1
        sta t_bot
        jsr t_home
        jmp t_clear_all
        .pend

t_sgr0  .proc
        lda #9
        sta t_fg
        sta t_bg
        lda #0
        sta t_bold
        sta t_inv
        .pend
; colour and reverse from the SGR state
t_attr  .proc
        lda t_fg
        cmp #9
        bne +
        lda #T_DEFCOL           ; default: grey, white when bold
        ldx t_bold
        beq ++
        lda #1
        bne ++
+       ora t_bold              ; (t_bold is 0 or 8)
        tax
        lda _ansi,x
+       sta t_color
        lda t_inv
        sta t_rev
        lda t_bg                ; a coloured background: reverse, in it
        cmp #9
        beq +
        cmp #0
        beq +
        tax
        lda _ansi,x
        sta t_color
        lda t_inv
        eor #$80
        sta t_rev
+       rts
_ansi   .byte 11, 2, 5, 7, 14, 4, 3, 15        ; 0-7
        .byte 12, 10, 13, 7, 14, 4, 3, 1        ; bright
        .pend

t_home  .proc
        lda #0
        sta t_x
        sta t_y
        sta t_wrap
        rts
        .pend

; row pointers of row A in t_p (screen) and t_c (colours)
t_row   .proc
        tay
        lda _lo,y
        sta t_p
        sta t_c
        lda _hi,y
        sta t_p+1
        clc
        adc #>(COLORS-SCREEN)
        sta t_c+1
        rts
_lo     .byte <(SCREEN + 40 * range(25))
_hi     .byte >(SCREEN + 40 * range(25))
        .pend

; clears cells A .. X-1 of row Y (in the current colour)
t_clear .proc
        sta t_tmp
        stx t_tmp+1
        tya
        jsr t_row
        ldy t_tmp
-       cpy t_tmp+1
        bcs +
        lda #' '
        sta (t_p),y
        lda t_color
        sta (t_c),y
        iny
        bne -
+       rts
        .pend

t_clear_all .proc
        lda #0
        sta t_tmp+2
-       ldy t_tmp+2
        lda #0
        ldx #T_COLS
        jsr t_clear
        inc t_tmp+2
        lda t_tmp+2
        cmp #T_ROWS
        bne -
        rts
        .pend

; copies row A to row X (cells and colours)
t_copyrow .proc
        sta t_tmp+3
        txa
        jsr t_row
        lda t_p
        sta t_q
        lda t_p+1
        sta t_q+1
        lda t_c
        sta t_r
        lda t_c+1
        sta t_r+1
        lda t_tmp+3
        jsr t_row
        ldy #T_COLS-1
-       lda (t_p),y
        sta (t_q),y
        lda (t_c),y
        sta (t_r),y
        dey
        bpl -
        rts
        .pend

; rows A..X up by one (row X cleared)
t_scrup .proc
        sta t_tmp+4
        stx t_tmp+5
-       lda t_tmp+4
        cmp t_tmp+5
        bcs +
        tax                     ; row + 1 -> row
        inc t_tmp+4
        lda t_tmp+4
        jsr t_copyrow
        jmp -
+       ldy t_tmp+5
        lda #0
        ldx #T_COLS
        jmp t_clear
        .pend

; rows A..X down by one (row A cleared)
t_scrdn .proc
        sta t_tmp+4
        stx t_tmp+5
-       lda t_tmp+5
        cmp t_tmp+4
        beq +
        bcc +
        tax                     ; row - 1 -> row
        dec t_tmp+5
        lda t_tmp+5
        jsr t_copyrow
        jmp -
+       ldy t_tmp+4
        lda #0
        ldx #T_COLS
        jmp t_clear
        .pend

; line feed: down, scrolling the region at its bottom
t_lf    .proc
        lda t_y
        cmp t_bot
        bne +
        lda t_top
        ldx t_bot
        jmp t_scrup
+       cmp #T_ROWS-1
        bcs +
        inc t_y
+       rts
        .pend

; reverse line feed
t_ri    .proc
        lda t_y
        cmp t_top
        bne +
        lda t_top
        ldx t_bot
        jmp t_scrdn
+       cmp #0
        beq +
        dec t_y
+       rts
        .pend

; a printable character (ASCII or glyph code) at the cursor
t_put   .proc
        pha
        lda t_wrap
        beq +
        lda #0
        sta t_x
        sta t_wrap
        jsr t_lf
+       lda t_y
        jsr t_row
        ldy t_x
        pla
        ora t_rev
        sta (t_p),y
        lda t_color
        sta (t_c),y
        cpy #T_COLS-1
        bcc +
        lda t_autowrap
        sta t_wrap
        rts
+       inc t_x
        rts
        .pend

; cursor: the cell at the cursor reversed
term_cursor_on .proc
        lda t_curvis
        beq _no
        lda t_curon
        bne _no
        lda #1
        sta t_curon
        jmp t_flip
_no     rts
        .pend

term_cursor_off .proc
        lda t_curon
        beq +
        lda #0
        sta t_curon
        jmp t_flip
+       rts
        .pend

t_flip  .proc
        lda t_y
        jsr t_row
        ldy t_x
        lda (t_p),y
        eor #$80
        sta (t_p),y
        rts
        .pend

; a byte from the server
term_out .proc
        ldx t_state
        bne _state
        jmp _norm
_state  cpx #TS_ESC
        bne +
        jmp t_esc
+       cpx #TS_CSI
        bne +
        jmp t_csi
+       cpx #TS_CHARSET
        bne _osc
        ldx #0                  ; ESC ( x: '0' line drawing, else ASCII
        cmp #'0'
        bne _g0
        inx
_g0     stx t_g0
        jmp _toNorm
_osc    cpx #TS_OSC
        bne _oscesc
        cmp #7                  ; OSC ends with BEL or ESC \
        beq _toNorm
        cmp #27
        bne _r
        lda #TS_OSCESC
        sta t_state
_r      rts
_oscesc cpx #TS_OSCESC
        beq _toNorm
        cpx #TS_HASH
        beq _toNorm
        ; TS_UTF8: continuation bytes
        cmp #$80
        bcc _norm1              ; not one: start again
        cmp #$c0
        bcs _norm1
        dec t_utf
        bne _r
        lda #TS_NORM
        sta t_state
        lda #'?'
        jmp t_put
_toNorm lda #TS_NORM
        sta t_state
        rts
_norm1  ldx #TS_NORM
        stx t_state
_norm   cmp #32
        bcc _ctl
        cmp #127
        bcc _print
        beq _r                  ; DEL: nothing
        cmp #$c0                ; UTF-8 lead byte
        bcc _q                  ; (a stray continuation)
        ldx #1
        cmp #$e0
        bcc +
        inx
        cmp #$f0
        bcc +
        inx
+       stx t_utf
        lda #TS_UTF8
        sta t_state
        rts
_q      lda #'?'
_print  ldx t_g0                ; DEC line drawing
        beq +
        cmp #$6a
        bcc +
        cmp #$79
        bcs +
        tax
        lda _dec-$6a,x
+       jmp t_put
_ctl    cmp #13
        beq _cr
        cmp #10
        beq _lf
        cmp #11
        beq _lf
        cmp #12
        beq _lf
        cmp #8
        beq _bs
        cmp #9
        beq _tab
        cmp #27
        beq _esc
        cmp #7
        beq _bell
        rts
_cr     lda #0
        sta t_x
        sta t_wrap
        rts
_lf     lda #0
        sta t_wrap
        jmp t_lf
_bs     lda #0
        sta t_wrap
        lda t_x
        beq +
        dec t_x
+       rts
_tab    lda t_x                 ; next multiple of 8
        and #$f8
        clc
        adc #8
        cmp #T_COLS
        bcc +
        lda #T_COLS-1
+       sta t_x
        rts
_esc    lda #TS_ESC
        sta t_state
        rts
_bell   inc $d020               ; a short flash of the border
        ldx #0
-       dex
        bne -
        dec $d020
        rts
; j k l m n o p q r s t u v w x: corners, crossings and lines
_dec    .text "+++++-----++++|"
        .pend

; after ESC
t_esc   .proc
        ldx #TS_NORM
        stx t_state
        cmp #'['
        bne +
        ldx #0
        stx t_np
        stx t_priv
        stx t_par
        stx t_par+1
        lda #TS_CSI
        sta t_state
        rts
+       cmp #']'
        bne +
        lda #TS_OSC
        sta t_state
        rts
+       cmp #'('
        bne +
        lda #TS_CHARSET
        sta t_state
        rts
+       cmp #')'                ; G1: taken, ignored
        beq _g1
        cmp #'#'
        bne +
        lda #TS_HASH
        sta t_state
        rts
+       cmp #'D'
        bne +
        jmp t_lf
+       cmp #'M'
        bne +
        jmp t_ri
+       cmp #'E'
        bne +
        lda #0
        sta t_x
        jmp t_lf
+       cmp #'7'
        bne +
        jmp t_save
+       cmp #'8'
        bne +
        jmp t_restore
+       cmp #'c'
        bne +
        jmp term_init
+       rts
_g1     lda #TS_HASH            ; (swallows the next byte)
        sta t_state
        rts
        .pend

t_save  .proc
        lda t_x
        sta t_sx
        lda t_y
        sta t_sy
        lda t_color
        sta t_scol
        lda t_rev
        sta t_srev
        rts
        .pend

t_restore .proc
        lda t_sx
        sta t_x
        lda t_sy
        sta t_y
        lda t_scol
        sta t_color
        lda t_srev
        sta t_rev
        lda #0
        sta t_wrap
        rts
        .pend

; inside ESC [
t_csi   .proc
        cmp #'0'
        bcc _notdig
        cmp #'9'+1
        bcs _notdig
        and #15                 ; parameter = 10 p + digit, at most 255
        sta t_tmp
        ldx t_np
        cpx #8
        bcs _r
        lda t_par,x
        cmp #26
        bcs _max
        asl a
        asl a
        adc t_par,x
        asl a
        adc t_tmp
        bcs _max
        sta t_par,x
_r      rts
_max    lda #255
        sta t_par,x
        rts
_notdig cmp #';'
        bne +
        inc t_np
        ldx t_np
        cpx #8
        bcs _r
        lda #0
        sta t_par,x
        rts
+       cmp #'@'                ; final byte?
        bcs _final
        cmp #'<'                ; private markers ? > = <
        bcc _r
        sta t_priv
        rts
_final  ldx #TS_NORM
        stx t_state
        inc t_np                ; parameters given
        ldx #_nf-1
-       cmp _finals,x
        beq +
        dex
        bpl -
        rts
+       txa
        asl a
        tax
        lda _jumps+1,x
        pha
        lda _jumps,x
        pha
        rts
_finals .text "ABCDEFGHJKLMPX@mrhlndsuSTfc`"
_nf     = * - _finals
_jumps  .word t_cuu-1, t_cud-1, t_cuf-1, t_cub-1, t_cnl-1, t_cpl-1
        .word t_cha-1, t_cup-1, t_ed-1, t_el-1, t_il-1, t_dl-1
        .word t_dch-1, t_ech-1, t_ich-1, t_sgr-1, t_stbm-1
        .word t_sm-1, t_rm-1, t_dsr-1, t_vpa-1, t_save-1, t_restore-1
        .word t_su-1, t_sd-1, t_cup-1, t_da-1, t_cha-1
        .pend

; first parameter, 0 counted as 1: in A
t_p1    .proc
        lda t_par
        bne +
        lda #1
+       rts
        .pend

t_cuu   .proc
        jsr t_p1
        sta t_tmp
        lda t_y
        sec
        sbc t_tmp
        bcc _top
        cmp t_top               ; not above the region if inside it
        bcs +
        ldx t_y
        cpx t_top
        bcc +
_top    lda t_top
        ldx t_y
        cpx t_top
        bcs +
        lda #0
+       sta t_y
        lda #0
        sta t_wrap
        rts
        .pend

t_cud   .proc
        jsr t_p1
        clc
        adc t_y
        bcs _bot
        ldx t_y
        cpx t_bot
        bcc +                   ; inside the region: stop at its bottom
        beq +
        cmp #T_ROWS
        bcc _set
        lda #T_ROWS-1
        bne _set
+       cmp t_bot
        bcc _set
        beq _set
_bot    lda t_bot
_set    sta t_y
        lda #0
        sta t_wrap
        rts
        .pend

t_cuf   .proc
        jsr t_p1
        clc
        adc t_x
        bcs +
        cmp #T_COLS
        bcc ++
+       lda #T_COLS-1
+       sta t_x
        lda #0
        sta t_wrap
        rts
        .pend

t_cub   .proc
        jsr t_p1
        sta t_tmp
        lda t_x
        sec
        sbc t_tmp
        bcs +
        lda #0
+       sta t_x
        lda #0
        sta t_wrap
        rts
        .pend

t_cnl   .proc
        jsr t_cud
        lda #0
        sta t_x
        rts
        .pend

t_cpl   .proc
        jsr t_cuu
        lda #0
        sta t_x
        rts
        .pend

t_cha   .proc                   ; column (1-based)
        jsr t_p1
        sec
        sbc #1
        cmp #T_COLS
        bcc +
        lda #T_COLS-1
+       sta t_x
        lda #0
        sta t_wrap
        rts
        .pend

t_vpa   .proc                   ; row (1-based)
        jsr t_p1
        sec
        sbc #1
        cmp #T_ROWS
        bcc +
        lda #T_ROWS-1
+       sta t_y
        lda #0
        sta t_wrap
        rts
        .pend

t_cup   .proc
        jsr t_vpa
        lda t_par+1
        sta t_par
        jmp t_cha
        .pend

t_ed    .proc                   ; 0: to the end, 1: from the start, 2: all
        lda t_par
        beq _end
        cmp #1
        beq _start
        jmp t_clear_all
_end    jsr _eol
        ldx t_y
-       inx
        cpx #T_ROWS
        bcs +
        stx t_tmp+2
        txa
        tay
        lda #0
        ldx #T_COLS
        jsr t_clear
        ldx t_tmp+2
        jmp -
+       rts
_start  lda #0
        sta t_tmp+2
-       lda t_tmp+2
        cmp t_y
        bcs +
        tay
        lda #0
        ldx #T_COLS
        jsr t_clear
        inc t_tmp+2
        jmp -
+       jmp _bol
_eol    lda t_x                 ; cursor .. end of line
        ldx #T_COLS
        ldy t_y
        jmp t_clear
_bol    lda #0                  ; start of line .. cursor
        ldx t_x
        inx
        ldy t_y
        jmp t_clear
        .pend

t_el    .proc
        lda t_par
        bne +
        jmp t_ed._eol
+       cmp #1
        bne +
        jmp t_ed._bol
+       lda #0
        ldx #T_COLS
        ldy t_y
        jmp t_clear
        .pend

t_il    .proc                   ; insert lines at the cursor row
        jsr t_p1
        sta t_tmp+6
        lda t_y
        cmp t_top
        bcc _r
        cmp t_bot
        beq +
        bcs _r
+
-       lda t_y
        ldx t_bot
        jsr t_scrdn
        dec t_tmp+6
        bne -
        lda #0
        sta t_x
_r      rts
        .pend

t_dl    .proc                   ; delete lines at the cursor row
        jsr t_p1
        sta t_tmp+6
        lda t_y
        cmp t_top
        bcc _r
        cmp t_bot
        beq +
        bcs _r
+
-       lda t_y
        ldx t_bot
        jsr t_scrup
        dec t_tmp+6
        bne -
        lda #0
        sta t_x
_r      rts
        .pend

t_dch   .proc                   ; delete characters: the rest moves left
        jsr t_p1
        sta t_tmp+6
        lda t_y
        jsr t_row
-       ldy t_x
_mv     iny
        cpy #T_COLS
        bcs _last
        lda (t_p),y
        dey
        sta (t_p),y
        iny
        lda (t_c),y
        dey
        sta (t_c),y
        iny
        jmp _mv
_last   ldy #T_COLS-1
        lda #' '
        sta (t_p),y
        lda t_color
        sta (t_c),y
        dec t_tmp+6
        bne -
        rts
        .pend

t_ich   .proc                   ; insert blanks: the rest moves right
        jsr t_p1
        sta t_tmp+6
        lda t_y
        jsr t_row
-       ldy #T_COLS-1
_mv     cpy t_x
        beq _blank
        bcc _blank
        dey
        lda (t_p),y
        iny
        sta (t_p),y
        dey
        lda (t_c),y
        iny
        sta (t_c),y
        dey
        jmp _mv
_blank  ldy t_x
        lda #' '
        sta (t_p),y
        lda t_color
        sta (t_c),y
        dec t_tmp+6
        bne -
        rts
        .pend

t_ech   .proc                   ; erase characters from the cursor
        jsr t_p1
        clc
        adc t_x
        bcs +
        cmp #T_COLS
        bcc ++
+       lda #T_COLS
+       tax
        lda t_x
        ldy t_y
        jmp t_clear
        .pend

t_sgr   .proc
        ldx #0
_next   cpx t_np
        bcs _done
        lda t_par,x
        bne +
        stx t_tmp+6             ; 0: all off
        jsr t_sgr0
        ldx t_tmp+6
        jmp _step
+       cmp #1
        bne +
        lda #8
        sta t_bold
        bne _step
+       cmp #7
        bne +
        lda #$80
        sta t_inv
        bne _step
+       cmp #22
        bne +
        lda #0
        sta t_bold
        beq _step
+       cmp #27
        bne +
        lda #0
        sta t_inv
        beq _step
+       cmp #30                 ; 30-37, 39 foreground
        bcc _step
        cmp #40
        bcs +
        sec
        sbc #30
        sta t_fg
        bpl _step
+       cmp #48                 ; 40-47, 49 background
        bcs +
        sec
        sbc #40
        sta t_bg
        bpl _step
+       cmp #38+10              ; 38/48;5;n and 38;2;r;g;b: skipped
        beq _skip
        cmp #90                 ; 90-97: bright foreground
        bcc _step
        cmp #98
        bcs _step
        sec
        sbc #90
        sta t_fg
        lda #8
        sta t_bold
        bne _step
_skip   inx
        inx
_step   inx
        jmp _next
_done   jmp t_attr
        .pend

t_stbm  .proc                   ; scrolling region, cursor home
        jsr t_p1
        sec
        sbc #1
        sta t_tmp
        lda t_np
        cmp #2
        bcc +
        lda t_par+1
        bne ++
+       lda #T_ROWS
+       sec
        sbc #1
        cmp #T_ROWS
        bcc +
        lda #T_ROWS-1
+       cmp t_tmp
        beq _r
        bcc _r
        sta t_bot
        lda t_tmp
        sta t_top
        jmp t_home
_r      rts
        .pend

t_sm    .proc                   ; set modes
        lda #1
        .byte $2c
        .pend
t_rm    .proc                   ; reset modes
        lda #0
        sta t_tmp
        lda t_priv
        cmp #'?'
        bne _r
        lda t_par
        cmp #1
        bne +
        lda t_tmp               ; DECCKM
        sta t_appkeys
_r      rts
+       cmp #7
        bne +
        lda t_tmp               ; autowrap
        sta t_autowrap
        rts
+       cmp #25
        bne +
        lda t_tmp               ; cursor visible
        sta t_curvis
        rts
+       cmp #47                 ; alternate screen: a clear screen at least
        beq +
        cmp #255                ; (1047, 1049 capped at 255)
        bne _r
+       jsr t_sgr0
        jsr t_home
        jmp t_clear_all
        .pend

t_dsr   .proc                   ; 6: cursor position report
        lda t_par
        cmp #6
        bne _five
        ldx #0
        lda #27
        jsr _add
        lda #'['
        jsr _add
        ldy t_y
        iny
        tya
        jsr _num
        lda #';'
        jsr _add
        ldy t_x
        iny
        tya
        jsr _num
        lda #'R'
        jsr _add
        stx term_rlen
        rts
_five   cmp #5
        bne _r
        ldx #0
-       lda _ok,x
        sta term_reply,x
        inx
        cpx #4
        bne -
        stx term_rlen
_r      rts
_ok     .text 27, "[0n"
_num    ldy #'0'-1              ; two digits at most (<= 40)
-       iny
        sec
        sbc #10
        bcs -
        adc #10
        pha
        tya
        cmp #'0'
        beq +
        jsr _add
+       pla
        ora #'0'
_add    sta term_reply,x
        inx
        rts
        .pend

t_da    .proc                   ; device attributes: a VT100
        lda t_priv
        bne _r
        ldx #0
-       lda _vt,x
        sta term_reply,x
        inx
        cpx #_n
        bne -
        stx term_rlen
_r      rts
_vt     .text 27, "[?1;0c"
_n      = * - _vt
        .pend

t_su    .proc
        jsr t_p1
        sta t_tmp+6
-       lda t_top
        ldx t_bot
        jsr t_scrup
        dec t_tmp+6
        bne -
        rts
        .pend

t_sd    .proc
        jsr t_p1
        sta t_tmp+6
-       lda t_top
        ldx t_bot
        jsr t_scrdn
        dec t_tmp+6
        bne -
        rts
        .pend
