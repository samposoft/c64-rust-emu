; SPDX-License-Identifier: GPL-2.0-or-later
; Copyright (C) 2026 SampoSoft - Francesco Sampoli
;
; Machine set-up: all RAM but I/O ($01 = $35), our own IRQ (60 Hz: ticks,
; keyboard) and NMI (RESTORE ignored), VIC in bank 3 with an ASCII font
; built from the machine's character ROM, screen at $C800.

SCREEN  = $c800
FONT    = $c000
COLORS  = $d800

        .section boot           ; (run once: see ssh64.asm)
sys_init .proc
        sei
        cld
        lda #$7f                ; CIA interrupts off
        sta $dc0d
        sta $dd0d
        lda $dc0d
        lda $dd0d
        jsr font_build          ; (reads the ROM: before the banking)
        lda #$35
        sta $01
        lda #<irq
        sta $fffe
        lda #>irq
        sta $ffff
        lda #<nmi
        sta $fffa
        sta $fffc
        lda #>nmi
        sta $fffb
        sta $fffd
        lda $dd00               ; VIC bank 3
        and #$fc
        sta $dd00
        lda #$20                ; screen $0800, font $0000 in the bank
        sta $d018
        lda #0
        sta $d020
        sta $d021
        sta ticks
        sta ticks+1
        lda #<(985248 / 60)     ; CIA1 timer A: 60 Hz
        sta $dc04
        lda #>(985248 / 60)
        sta $dc05
        lda #$11                ; start, continuous
        sta $dc0e
        lda #$81                ; its interrupt on
        sta $dc0d
        cli
        rts
        .pend
        .send boot

        .section bss
sys_busy .fill 1                ; a long computation: its border colour
        .send bss

; a long computation begins: the screen off (the VIC takes no cycles), the
; keyboard not scanned, the border blinking in colour A (not 0). A C128
; runs at 2 MHz meanwhile ($d030 bit 0, its fast mode, possible with the
; screen off; nothing on a C64)
sys_crunch .proc
        sta sys_busy
        lda $d011
        and #$ef
        sta $d011
        lda #1
        sta $d030
        rts
        .pend

; and ends; the font is made again first (X25519 keeps a table there)
sys_awake .proc
        lda #0
        sta $d030
        sta sys_busy
        sta $d020
        sei                     ; (the character ROM in: the KERNAL's vectors)
        jsr font_build
        cli
        lda $d011
        ora #$10
        sta $d011
        rts
        .pend

; the network between two field operations: at 1 MHz, the cartridge might
; not keep up with a C128 in its fast mode
sys_netpoll .proc
        lda #0
        sta $d030
        jsr net_poll
        lda sys_busy
        beq +
        lda #1
        sta $d030
+       rts
        .pend

irq     .proc
        sta irq_a
        stx irq_x
        sty irq_y
        lda $01                 ; the I/O may be banked out
        pha
        lda #$35
        sta $01
        lda $dc0d               ; acknowledge
        inc ticks
        bne +
        inc ticks+1
+       lda sys_busy
        beq _scan
        lda ticks               ; blinking, twice a second
        and #$20
        beq +
        lda sys_busy
+       sta $d020
        jmp _done
_scan   jsr kbd_scan
_done   pla
        sta $01
        lda irq_a
        ldx irq_x
        ldy irq_y
nmi     rti
        .pend
nmi     = irq.nmi

; ASCII font at FONT: glyph c for code c (32-126), its reverse at c + 128
font_build .proc
        lda $01
        pha
        lda #$33                ; character ROM at $d000
        sta $01
        ldx #0
_ch     txa                     ; ROM glyph of code x (lower case set)
        cmp #32
        bcc _blank
        cmp #64
        bcc _rom                ; space .. ?: the same
        cmp #127
        bcs _blank
        tay
        lda _map-64,y
        cmp #$ff
        beq _own
_rom    ldy #0                  ; ROM set 2 at $d800 + 8a
        sty sys_t+1
        asl a
        rol sys_t+1
        asl a
        rol sys_t+1
        asl a
        rol sys_t+1
        sta sys_t
        lda sys_t+1
        clc
        adc #$d8
        sta sys_t+1
        jmp _copy
_own    txa                     ; our glyph: _own_font + 8 (index)
        ldy #_nown-1
-       cmp _owncodes,y
        beq +
        dey
        bpl -
+       tya
        asl a
        asl a
        asl a
        clc
        adc #<_own_font
        sta sys_t
        lda #>_own_font
        adc #0
        sta sys_t+1
        jmp _copy
_blank  lda #<_zero
        sta sys_t
        lda #>_zero
        sta sys_t+1
_copy   txa                     ; to FONT + 8x and FONT + 8(x+128)
        pha
        ldy #0
        sty scr_p+1
        asl a
        rol scr_p+1
        asl a
        rol scr_p+1
        asl a
        rol scr_p+1
        sta scr_p
        lda scr_p+1
        adc #>FONT
        sta scr_p+1
        ldy #7
-       lda (sys_t),y
        sta (scr_p),y
        dey
        bpl -
        lda scr_p+1             ; + 1024: the reverse
        clc
        adc #4
        sta scr_p+1
        ldy #7
-       lda (sys_t),y
        eor #$ff
        sta (scr_p),y
        dey
        bpl -
        pla
        tax
        inx
        cpx #128
        bne _ch
        pla
        sta $01
        rts

; codes 64..126 -> ROM screen code, $ff = our own glyph
_map    .byte 0                                         ; @
        .byte range(65, 91)                             ; A-Z
        .byte 27, $ff, 29, $ff, $ff, $ff                ; [ \ ] ^ _ `
        .byte range(1, 27)                              ; a-z
        .byte $ff, $ff, $ff, $ff                        ; { | } ~
_owncodes .byte $5c, $5e, $5f, $60, $7b, $7c, $7d, $7e
_nown   = * - _owncodes
_own_font
        .byte $00, $60, $30, $18, $0c, $06, $03, $00    ; backslash
        .byte $18, $3c, $66, $00, $00, $00, $00, $00    ; ^
        .byte $00, $00, $00, $00, $00, $00, $00, $ff    ; _
        .byte $30, $18, $0c, $00, $00, $00, $00, $00    ; `
        .byte $0e, $18, $18, $70, $18, $18, $0e, $00    ; {
        .byte $18, $18, $18, $18, $18, $18, $18, $18    ; |
        .byte $70, $18, $18, $0e, $18, $18, $70, $00    ; }
        .byte $00, $00, $32, $7e, $4c, $00, $00, $00    ; ~
_zero   .fill 8
        .pend

