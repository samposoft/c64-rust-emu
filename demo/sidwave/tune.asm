; SPDX-License-Identifier: GPL-2.0-or-later
; Copyright (C) 2026 SampoSoft - Francesco Sampoli
;
; The tune alone: the player called by a raster interrupt at line $40. The
; border changes colour while the player runs (its time, in raster lines).
MZP = $fb
MZP2 = $fd
        * = $0801
        .word +, 2026
        .null $9e, format("%d", start)
+       .word 0
start   sei
        lda #$35
        sta $01
        lda #$7f
        sta $dc0d
        lda $dc0d
        jsr mus_init
        lda #<irq
        sta $fffe
        lda #>irq
        sta $ffff
        lda #$1b
        sta $d011
        lda #$40
        sta $d012
        lda #1
        sta $d01a
        sta $d019
        cli
-       jmp -
irq     pha
        txa
        pha
        tya
        pha
        inc $d020
        jsr mus_play
        dec $d020
        asl $d019
        pla
        tay
        pla
        tax
        pla
        rti
        .include "player.asm"
        .include "music.asm"
