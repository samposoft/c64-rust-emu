; SPDX-License-Identifier: GPL-2.0-or-later
; Copyright (C) 2026 SampoSoft - Francesco Sampoli
;
; Files on drive 8 through the KERNAL, switched in only for that. The
; KERNAL finds its own state in the zero page and its vectors as we left
; them: the few it needs are set again first (no files open, no messages),
; interrupts go to a handler of ours that just acknowledges the timer and
; counts the ticks. File names and the memory saved must be below $a000 or
; in $c000-$cfff, where the KERNAL sees RAM.
;
;   disk_load   name at X/Y (length byte, text), to address in disk_addr;
;               carry set on error
;   disk_save   name at X/Y, from disk_addr to disk_end (excluded)

DISK_DEV = 8

        .section bssl
disk_addr .fill 2
disk_end .fill 2
disk_name .fill 2
disk_vec .fill 4                ; IRQ and NMI vectors of the KERNAL
disk_cmd .fill 20               ; "S0:" + name
        .send bssl

; KERNAL in, its state made sane; interrupts to disk_irq
disk_on .proc
        sei
        lda #$36
        sta $01
        lda $0314
        sta disk_vec
        lda $0315
        sta disk_vec+1
        lda $0318
        sta disk_vec+2
        lda $0319
        sta disk_vec+3
        lda #<disk_irq
        sta $0314
        lda #>disk_irq
        sta $0315
        lda #<disk_nmi
        sta $0318
        lda #>disk_nmi
        sta $0319
        lda #0
        sta $9d                 ; no messages
        sta $90                 ; status
        jsr $ffe7               ; CLALL: no files open, default devices
        rts
        .pend

disk_off .proc
        php                     ; (the carry of the KERNAL call)
        lda disk_vec
        sta $0314
        lda disk_vec+1
        sta $0315
        lda disk_vec+2
        sta $0318
        lda disk_vec+3
        sta $0319
        lda #$35
        sta $01
        plp
        cli
        rts
        .pend

; interrupts while the KERNAL is in: it has pushed A, X, Y
disk_irq .proc
        lda $dc0d
        inc ticks
        bne +
        inc ticks+1
+       pla
        tay
        pla
        tax
        pla
disk_nmi rti
        .pend
disk_nmi = disk_irq.disk_nmi

; SETNAM with the name at disk_name (length byte, text)
disk_setnam .proc
        ldy #0
        lda disk_name
        sta sys_t
        lda disk_name+1
        sta sys_t+1
        lda (sys_t),y
        pha
        inc sys_t
        bne +
        inc sys_t+1
+       pla
        ldx sys_t
        ldy sys_t+1
        jmp $ffbd
        .pend

disk_load .proc
        stx disk_name
        sty disk_name+1
        jsr disk_on
        lda #1
        ldx #DISK_DEV
        ldy #0                  ; to our address
        jsr $ffba
        jsr disk_setnam
        lda #0
        ldx disk_addr
        ldy disk_addr+1
        jsr $ffd5
        jmp disk_off
        .pend

; scratches the file, then saves the new one
disk_save .proc
        stx sys_t               ; "S0:" + name
        sty sys_t+1
        stx disk_name
        sty disk_name+1
        ldy #0
        lda (sys_t),y
        tay
        clc
        adc #3
        sta disk_cmd
        cpy #0
        beq +
-       lda (sys_t),y
        sta disk_cmd+3,y
        dey
        bne -
+       lda #'S'
        sta disk_cmd+1
        lda #'0'
        sta disk_cmd+2
        lda #':'
        sta disk_cmd+3
        jsr disk_on
        lda #15
        ldx #DISK_DEV
        ldy #15
        jsr $ffba
        lda disk_name
        pha
        lda disk_name+1
        pha
        lda #<disk_cmd
        sta disk_name
        lda #>disk_cmd
        sta disk_name+1
        jsr disk_setnam
        jsr $ffc0               ; OPEN 15,8,15,"S0:name"
        lda #15
        jsr $ffc3               ; CLOSE
        pla
        sta disk_name+1
        pla
        sta disk_name
        lda #1
        ldx #DISK_DEV
        ldy #0
        jsr $ffba
        jsr disk_setnam
        lda disk_addr
        sta $fb
        lda disk_addr+1
        sta $fc
        lda #$fb
        ldx disk_end
        ldy disk_end+1
        jsr $ffd8
        jmp disk_off
        .pend
