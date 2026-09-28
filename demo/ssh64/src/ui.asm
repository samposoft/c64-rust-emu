; SPDX-License-Identifier: GPL-2.0-or-later
; Copyright (C) 2026 SampoSoft - Francesco Sampoli
;
; Messages and questions, through the terminal: putc (13 = new line),
; ui_status (a line of its own), ui_msg (text from the server, controls
; shown as '?'), ui_input (a line typed, with DEL; ui_idle runs meanwhile).

putc    .proc
        cmp #13
        bne +
        jsr term_out
        lda #10
+       jmp term_out
        .pend

ui_nl   .proc
        lda #13
        jmp putc
        .pend

; a new line, then the string after the jsr
ui_status .proc
        jsr ui_nl
        jmp puts
        .pend

; the SSH string at ssh_sp / ssh_sl
ui_msg  .proc
-       lda ssh_sl
        ora ssh_sl+1
        beq _done
        ldy #0
        lda (ssh_sp),y
        cmp #10
        beq _nl
        cmp #13
        beq _next
        cmp #32
        bcc _q
        cmp #127
        bcc _put
_q      lda #'?'
_put    jsr term_out
        jmp _next
_nl     jsr ui_nl
_next   inc ssh_sp
        bne +
        inc ssh_sp+1
+       lda ssh_sl
        bne +
        dec ssh_sl+1
+       dec ssh_sl
        jmp -
_done   rts
        .pend

; the exact moments keys go down and up, as randomness: the whole matrix
; read here (not in the 60 Hz interrupt, whose moments are known) and, when
; it changes, the CIA timer, the raster and the SID noise stirred in
ui_entropy .proc
        lda #0
        sta $dc00
        lda $dc01
        ldx #$ff
        stx $dc00
        cmp ui_keys
        beq +
        sta ui_keys
        eor $dc04
        jsr rng_stir
+       rts
        .section bss
ui_keys .fill 1
        .send bss
        .pend

; a line into the buffer at X/Y (length byte, text), at most A characters;
; C set: shown as '*'
ui_input .proc
        stx str_p
        sty str_p+1
        sta ui_max
        lda #0
        rol a
        sta ui_mask
        lda #0
        tay
        sta (str_p),y
        jsr term_cursor_on
_wait   jsr ui_idle
        jsr ui_entropy
        jsr kbd_get
        bcs _wait
        pha
        jsr rng_stir
        pla
        cmp #13
        beq _done
        cmp #$7f
        beq _del
        cmp #32
        bcc _wait
        cmp #127
        bcs _wait
        ldy #0
        pha
        lda (str_p),y
        cmp ui_max
        pla
        bcs _wait
        pha
        lda (str_p),y
        clc
        adc #1
        sta (str_p),y
        tay
        pla
        sta (str_p),y
        ldx ui_mask
        beq +
        lda #'*'
+       pha
        jsr term_cursor_off
        pla
        jsr term_out
        jsr term_cursor_on
        jmp _wait
_del    ldy #0
        lda (str_p),y
        beq _wait
        sec
        sbc #1
        sta (str_p),y
        jsr term_cursor_off
        lda #8
        jsr term_out
        lda #' '
        jsr term_out
        lda #8
        jsr term_out
        jsr term_cursor_on
        jmp _wait
_done   jsr term_cursor_off
        rts
        .section bss
ui_max  .fill 1
ui_mask .fill 1
        .send bss
        .pend
