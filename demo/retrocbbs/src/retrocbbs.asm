; SPDX-License-Identifier: GPL-2.0-or-later
; Copyright (C) 2026 SampoSoft - Francesco Sampoli
;
; retrocbbs: a PETSCII terminal for the Commodore 64 with an RR-Net
; compatible Ethernet cartridge, that calls one BBS only: RetroCampus BBS,
; bbs.retrocampus.com, port 6510.
;
; Build: 64tass -C -a -B -o build/retrocbbs.prg src/retrocbbs.asm
;
; The network (Ethernet, ARP, IP, DHCP, DNS, TCP) is ssh64's
; (../../ssh64/src). The BBS speaks raw PETSCII over TCP, no telnet: every
; byte it sends goes to the KERNAL's CHROUT, the screen editor of a real
; C64 that PETSCII BBSs are drawn for, and every key from GETIN goes to it
; as it is. The KERNAL and BASIC stay on: the program returns to BASIC.
; Files come by XMODEM to drive 8 (xmodem.asm).
;
; Memory: code from $0801, then the variables and the network's buffers,
; below $a000.

        .cpu "6502"

BBS_PORT = 6510

; zero page: some of BASIC's temporaries, which the KERNAL does not use;
; saved at the start and put back for BASIC at the end
ZP_FIRST = $57
net_p   = $57           ; pointers (2 each)
net_q   = $59
net_n   = $5b           ; byte count (2)
net_ck  = $5d           ; checksum: low, high, carries (3)
net_t   = $60           ; scratch (4)
net_len = $64           ; length of the packet being built (2)
ticks   = $66           ; 1/60 s counter, from the IRQ (2)
str_p   = $68           ; inline string pointer (2)
ZP_END  = $6a

; KERNAL
CHROUT  = $ffd2
GETIN   = $ffe4
STOP    = $ffe1
PLOT    = $fff0
SHFLAG  = $028d         ; bit 1: C= held
BLNSW   = $cc           ; 0: the cursor blinks
BLNON   = $cf           ; the cursor is shown
GDBLN   = $ce           ; character under the cursor
GDCOL   = $0287         ; and its colour
QTSW    = $d4           ; quote mode
INSRT   = $d8           ; inserts pending
DSPP    = $ea13         ; A at the cursor, colour X

        ; strings: ASCII for the network; PETSCII for the screen and the
        ; drive (-a converts "none" to PETSCII: lower case $41-$5a)
        .enc "ascii"
        .cdef " ~", 32
        .enc "petscii"
        .cdef " @", $20
        .cdef "AZ", $c1
        .cdef "az", $41
        .cdef "[[", $5b
        .cdef "]]", $5d
        .edef "{clr}", $93
        .edef "{lower}", $0e
        .edef "{lock}", $08
        .edef "{white}", $05
        .edef "{grey}", $98
        .edef "{cyan}", $9f
        .edef "{yellow}", $9e
        .edef "{red}", $1c
        .edef "{rvs}", $12
        .edef "{off}", $92
        .enc "none"

        * = $0801
        .word +, 2026
        .null $9e, format("%d", start)
+       .word 0

start   ldx #ZP_END-ZP_FIRST-1
-       lda ZP_FIRST,x
        sta zp_save,x
        dex
        bpl -
        lda #<bss_start         ; variables at zero
        ldx #>bss_start
        ldy #>(bss_end+255)
        jsr zero
        tsx                     ; for the way back to BASIC
        stx sp_basic
        lda #0
        sta ticks
        sta ticks+1
        sta $d020
        sta $d021
        sei                     ; our tick in the KERNAL's IRQ (60 Hz)
        lda $0314
        sta irq_old
        lda $0315
        sta irq_old+1
        lda #<irq
        sta $0314
        lda #>irq
        sta $0315
        cli
        lda #$ff                ; SID voice 3: noise at the top frequency,
        sta $d40e               ; read at $d41b by the network (random
        sta $d40f               ; ports, identifiers); voice 3 muted
        lda #$80
        sta $d412
        lda #$8f
        sta $d418
        jsr puts
        .enc "petscii"
        .text "{clr}{lower}{lock}{white}RetroCampus BBS terminal", 13
        .null "{grey}bbs.retrocampus.com, port 6510", 13, 13
        .enc "none"
        ; a random locally administered MAC address
        lda #$02
        sta my_mac
        lda #$c6
        sta my_mac+1
        ldx #5
-       lda $d41b
        eor $dc04
        eor $d012
        sta my_mac,x
        dex
        cpx #2
        bne -
        jsr eth_init
        bcc +
        jsr puts
        .enc "petscii"
        .null "{red}No RR-Net cartridge at $DE00.", 13
        .enc "none"
        jmp quit
+       lda #0
        ldx #19
-       sta my_ip,x             ; my_ip, mask, router, DNS
        dex
        bpl -
        jsr dhcp_start
        lda ticks+1
        sta dhcp_at

call    jsr connect
        bcs _ask
        jsr term
_ask    jsr cursor_off
        jsr puts
        .enc "petscii"
        .null 13, "{white}RETURN: call again, RUN/STOP: quit", 13
        .enc "none"
-       jsr idle
        jsr GETIN
        cmp #13
        beq call
        cmp #3
        bne -

quit    jsr tcp_close
        sei                     ; the KERNAL's IRQ alone again
        lda irq_old
        sta $0314
        lda irq_old+1
        sta $0315
        cli
-       jsr STOP                ; RUN/STOP released, or BASIC stops too
        beq -
        ldx #ZP_END-ZP_FIRST-1
-       lda zp_save,x
        sta ZP_FIRST,x
        dex
        bpl -
        ldx sp_basic
        txs
        rts

zp_save .fill ZP_END-ZP_FIRST

; zeroes from A/X up to page Y
zero    .proc
        sta net_p
        stx net_p+1
        sty net_n
        ldy #0
        tya
-       sta (net_p),y
        iny
        bne -
        inc net_p+1
        ldx net_p+1
        cpx net_n
        bne -
        rts
        .pend

irq     .proc
        inc ticks
        bne +
        inc ticks+1
+       jmp (irq_old)
        .pend

; the network while waiting: DHCP again every 4 s until it answers
idle    .proc
        jsr net_poll
        jsr tcp_poll
        lda dhcp_state
        cmp #3
        beq +
        lda ticks+1
        sec
        sbc dhcp_at
        beq +
        lda ticks+1
        sta dhcp_at
        jsr dhcp_start
+       rts
        .pend

; carry set if RUN/STOP is in the keyboard buffer (other keys are dropped)
stopped .proc
        jsr GETIN
        cmp #3
        beq +
        clc
+       rts
        .pend

; address, name, TCP; carry set on failure (message printed) or RUN/STOP
connect .proc
        jsr cursor_off
        jsr puts
        .enc "petscii"
        .null "{cyan}Address... "
        .enc "none"
-       jsr idle
        jsr stopped
        bcs _stop
        lda dhcp_state
        cmp #3
        bne -
        ldx #<my_ip
        ldy #>my_ip
        jsr putip
        jsr puts
        .enc "petscii"
        .null 13, "Looking up bbs.retrocampus.com... "
        .enc "none"
        lda #<host
        sta net_q
        lda #>host
        sta net_q+1
        jsr dns_query
-       jsr idle
        jsr dns_poll
        jsr stopped
        bcs _stop
        lda dns_state
        cmp #1
        beq -
        cmp #2
        beq +
        jsr puts
        .enc "petscii"
        .null "{red}not found", 13
        .enc "none"
        sec
        rts
+       ldx #<dns_ip
        ldy #>dns_ip
        jsr putip
        jsr puts
        .enc "petscii"
        .null 13, "Calling... "
        .enc "none"
        #cp4 tx_dst, dns_ip
        lda #>BBS_PORT
        sta tcp_rport
        lda #<BBS_PORT
        sta tcp_rport+1
        jsr tcp_connect
-       jsr idle
        jsr stopped
        bcs _stop
        lda tcp_state
        cmp #TCP_SYNSENT
        beq -
        cmp #TCP_OPEN
        beq +
        jsr puts
        .enc "petscii"
        .null "{red}no answer", 13
        .enc "none"
        sec
        rts
+       jsr puts
        .enc "petscii"
        .null "{yellow}connected", 13
        .enc "none"
        clc
        rts
_stop   jsr tcp_close
        jsr puts
        .enc "petscii"
        .null 13, "{red}stopped", 13
        .enc "none"
        sec
        rts
        .pend

; the session: what comes goes to the screen, keys go out; until the BBS
; hangs up, the connection is lost or C= + RUN/STOP. CTRL + D (code 4,
; nothing in PETSCII) and F3 (CCGMS's download key, which BBSs leave to
; the terminal) download a file instead of going out
term    .proc
_loop   jsr net_poll
        jsr tcp_poll
        ; received bytes, up to 64 before the network is looked at again
        lda #64
        sta t_n
_rx     jsr tcp_read
        bcs _key
        pha
        jsr cursor_off
        pla
        jsr bbs_out
        dec t_n
        bne _rx
        jmp _loop
_key    jsr cursor_on
        jsr GETIN
        cmp #0
        beq _state
        cmp #$83                ; C= + RUN/STOP: hang up
        bne +
        lda SHFLAG
        and #2
        beq +
        jsr tcp_close
        jsr bottom
        jsr puts
        .enc "petscii"
        .null "{off}{white}Hung up.", 13
        .enc "none"
        rts
+       cmp #4                  ; CTRL + D, or F3 as in CCGMS: download
        beq _dl                 ; (XMODEM)
        cmp #$86
        bne +
_dl     jsr cursor_off
        jsr xm_download
        jmp _state
+       sta t_key
        lda #<t_key
        sta net_q
        lda #>t_key
        sta net_q+1
        lda #1
        jsr tcp_write           ; (full ring: the key is lost)
_state  lda tcp_state
        cmp #TCP_OPEN
        bne _lost
        lda tcp_fin             ; the BBS closed and all it sent is shown
        beq _loop
        jsr tcp_avail
        lda net_t
        ora net_t+1
        bne _loop
        jsr tcp_close
        jsr bottom
        jsr puts
        .enc "petscii"
        .null "{off}{white}The BBS hung up.", 13
        .enc "none"
        rts
_lost   jsr tcp_close
        jsr bottom
        jsr puts
        .enc "petscii"
        .null "{off}{red}Connection lost.", 13
        .enc "none"
        rts
        .pend

; the cursor off, on a new line at the bottom of the screen: messages
; after the session scroll the BBS's last page up, not over it
bottom  .proc
        jsr cursor_off
        ldx #24
        ldy #0
        clc
        jsr PLOT
        lda #13
        jmp CHROUT
        .pend

; a byte from the BBS: to the screen editor, never in quote or insert mode
; (a quote must not turn the next control codes into characters); BEL
; rings
bbs_out .proc
        cmp #7
        beq bell
        ldx #0
        stx QTSW
        stx INSRT
        jmp CHROUT
        .pend

; a short beep on voice 1
bell    .proc
        lda #$00
        sta $d400
        lda #$30                ; about 2.9 kHz
        sta $d401
        lda #$09                ; attack 2 ms, decay 750 ms, no sustain
        sta $d405
        lda #$00
        sta $d406
        lda #$10                ; triangle, gate retriggered
        sta $d404
        lda #$11
        sta $d404
        rts
        .pend

; the cursor off before printing, as the KERNAL's GETIN loop does it
cursor_off .proc
        lda BLNSW
        bne _ret
        sei
        lda #1
        sta BLNSW
        lda BLNON
        beq +
        lda GDBLN
        ldx GDCOL
        ldy #0
        sty BLNON
        jsr DSPP
+       cli
_ret    rts
        .pend

; and blinking again while nothing comes
cursor_on .proc
        lda #0
        sta BLNSW
        rts
        .pend

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
        jsr CHROUT
        jmp -
+       lda str_p+1
        pha
        lda str_p
        pha
        rts
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
        jsr CHROUT
        jmp -
+       rts
_i      .byte 0
        .pend

; A as a decimal number
putdec  .proc
        ldx #0                  ; hundreds
-       cmp #100
        bcc +
        sbc #100
        inx
        bne -
+       stx _h
        ldx #0                  ; tens
-       cmp #10
        bcc +
        sbc #10
        inx
        bne -
+       stx _t
        pha                     ; units
        lda _h
        beq +
        ora #'0'
        jsr CHROUT
        jmp _tens
+       lda _t
        beq _u
_tens   lda _t
        ora #'0'
        jsr CHROUT
_u      pla
        ora #'0'
        jmp CHROUT
_h      .byte 0
_t      .byte 0
        .pend

        .enc "ascii"
host    .null "bbs.retrocampus.com"
        .enc "none"

        .dsection code
        .section code
        .include "../../ssh64/src/net.asm"
        .include "../../ssh64/src/tcp.asm"
        .include "xmodem.asm"
        .send code

code_end
        .virtual code_end
bss_start
        .dsection bssl
        .dsection bss
bss_end
        .endv
        .cerror bss_end > $a000, "variables past $a000: ", bss_end

        .section bss
sp_basic .fill 1
irq_old .fill 2
dhcp_at .fill 1
t_n     .fill 1
t_key   .fill 1
        .send bss
