; SPDX-License-Identifier: GPL-2.0-or-later
; Copyright (C) 2026 SampoSoft - Francesco Sampoli
;
; Known hosts, in the file SSH64 HOSTS: records of the host as typed
; (length byte, text, lower case), key type (1 = ssh-ed25519), key (32
; bytes); a 0 length ends them. And the comb of each key, which its
; signatures are verified with, in SSH64 Kxxxxxxxx (the first bytes of the
; key in hex): read before connecting, or made then if it is missing.

KH_SIZE = 1024

        .section bssl
kh_name .fill 64                ; this host: length, text
kh_state .fill 1                ; 0 unknown, 1 known, 2 known with another key
kh_new  .fill 1                 ; a new host key was accepted
kh_combname .fill 16
        .send bssl

kh_file .byte 11
        .text "SSH64 HOSTS"

; the host list from the disk (empty if there is none)
kh_load .proc
        lda #<kh_buf
        sta disk_addr
        lda #>kh_buf
        sta disk_addr+1
        ldx #<kh_file
        ldy #>kh_file
        jsr disk_load
        bcc +
        lda #0
        sta kh_buf
+       lda #0                  ; a 0 at the very end, in any case
        sta kh_buf+KH_SIZE-1
        rts
        .pend

; kh_name from ssh_host, in lower case
kh_setname .proc
        ldx ssh_host
        stx kh_name
-       lda ssh_host,x
        cmp #'A'
        bcc +
        cmp #'Z'+1
        bcs +
        ora #$20
+       sta kh_name,x
        dex
        bne -
        rts
        .pend

; str_p to the record of kh_name (carry clear), or to the end of the list
; (carry set)
kh_find .proc
        lda #<kh_buf
        sta str_p
        lda #>kh_buf
        sta str_p+1
_rec    ldy #0
        lda (str_p),y
        bne +
        sec
        rts
+       cmp kh_name
        bne _next
        tay
-       lda (str_p),y
        cmp kh_name,y
        bne _next
        dey
        bne -
        clc
        rts
_next   ldy #0                  ; past name, type and key
        lda (str_p),y
        clc
        adc #34
        adc str_p
        sta str_p
        bcc _rec
        inc str_p+1
        jmp _rec
        .pend

; kh_state for ssh_hkey
kh_check .proc
        jsr kh_find
        lda #0
        bcs _set
        ldy #0                  ; the key after the name and type
        lda (str_p),y
        tay
        iny
        lda (str_p),y
        cmp #1
        bne _other
        ldx #0
-       iny
        lda (str_p),y
        cmp ssh_hkey,x
        bne _other
        inx
        cpx #32
        bne -
        lda #1
        bne _set
_other  lda #2
_set    sta kh_state
        rts
        .pend

; adds this host with ssh_hkey and saves the list
kh_add .proc
        jsr kh_find             ; the end
        lda str_p               ; room for it and the final 0?
        clc
        adc kh_name
        sta sys_t
        lda str_p+1
        adc #0
        sta sys_t+1
        lda sys_t
        clc
        adc #1+1+32+1
        sta sys_t
        bcc +
        inc sys_t+1
+       lda sys_t
        cmp #<(kh_buf+KH_SIZE)
        lda sys_t+1
        sbc #>(kh_buf+KH_SIZE)
        bcc +
        rts                     ; full (carry set)
+       ldy kh_name
-       lda kh_name,y
        sta (str_p),y
        dey
        bpl -
        ldy kh_name
        iny
        lda #1
        sta (str_p),y
        ldx #0
-       iny
        lda ssh_hkey,x
        sta (str_p),y
        inx
        cpx #32
        bne -
        iny
        lda #0
        sta (str_p),y
        iny
        tya                     ; the end, excluded
        clc
        adc str_p
        sta disk_end
        lda str_p+1
        adc #0
        sta disk_end+1
        lda #<kh_buf
        sta disk_addr
        lda #>kh_buf
        sta disk_addr+1
        ldx #<kh_file
        ldy #>kh_file
        jmp disk_save
        .pend

; the name of the comb file of the key ssh_hkey
kh_setcomb .proc
        ldx #0
-       lda _prefix,x
        sta kh_combname,x
        inx
        cpx #8
        bne -
        ldy #0
-       lda ssh_hkey,y
        lsr a
        lsr a
        lsr a
        lsr a
        jsr _hex
        lda ssh_hkey,y
        and #15
        jsr _hex
        iny
        cpy #4
        bne -
        rts
_hex    cmp #10
        bcc +
        adc #6
+       adc #'0'
        sta kh_combname,x
        inx
        rts
_prefix .byte 15
        .text "SSH64 K"
        .pend

kh_combfile .proc
        lda #<ed_ta
        sta disk_addr
        lda #>ed_ta
        sta disk_addr+1
        ldx #<kh_combname
        ldy #>kh_combname
        rts
        .pend

; the comb of ssh_hkey from the disk; carry set if it is not there
kh_loadcomb .proc
        jsr kh_combfile
        jsr disk_load
        bcs _r
        cpx #<(ed_ta+ED_TALEN)  ; all of it?
        bne _no
        cpy #>(ed_ta+ED_TALEN)
        bne _no
        clc
_r      rts
_no     sec
        rts
        .pend

kh_savecomb .proc
        jsr kh_combfile
        lda #<(ed_ta+ED_TALEN)
        sta disk_end
        lda #>(ed_ta+ED_TALEN)
        sta disk_end+1
        jmp disk_save
        .pend

; before connecting to a known host: its key into ssh_hkey, its comb into
; ed_ta, from the disk or made now (then saved); carry set if the host is
; unknown
kh_prepare .proc
        jsr kh_find
        bcc +
        rts
+       ldy #0                  ; the key after the name and type
        lda (str_p),y
        tay
        iny
        ldx #0
-       iny
        lda (str_p),y
        sta ssh_hkey,x
        sta ed_pub,x
        inx
        cpx #32
        bne -
        jsr kh_setcomb
        jsr kh_loadcomb
        bcc _r
        jsr ui_status
        .null "preparing this host's key (80 s)"
        jsr build_tables
        ldx #31
        lda #0
-       sta fe_mask,x           ; (public numbers: no mask)
        dex
        bpl -
        lda #6                  ; blue
        jsr sys_crunch
        jsr ed_comb_build
        php
        jsr sys_awake
        plp
        bcs +                   ; not a point: the connection will say
        jsr kh_savecomb
+       clc
_r      rts
        .pend
