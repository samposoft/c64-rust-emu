; SPDX-License-Identifier: GPL-2.0-or-later
; Copyright (C) 2026 SampoSoft - Francesco Sampoli
;
; Network test of ssh64: DHCP, DNS of "localhost", TCP to port 7777 of
; that address, sends a line and shows whatever comes back.
; On the emulator: c64 --eth rrnet build/nettest.prg, with an echo server
; on the host's 127.0.0.1:7777.

        .cpu "6502"
        .include "../src/zp.asm"

        * = $0801
        .word +, 10
        .null $9e, format("%d", start)
+       .word 0
start   jmp main

        .dsection code
        .cerror * > $a000, "code too long"
        .virtual $e000
        .dsection bss
bss_end
        .endv
        .virtual $8000
        .dsection bssl
        .endv
        .cerror bss_end > $fff0, "bss too long"

        .section code
        .include "../src/sys.asm"
        .include "../src/scr.asm"
        .include "../src/fmt.asm"
        .include "../src/kbd.asm"
        .include "../src/net.asm"
        .include "../src/tcp.asm"

main    jsr sys_init
        jsr cls
        jsr puts
        .null "ssh64 network test", 13
        ldx #5
-       lda _mac,x
        sta my_mac,x
        dex
        bpl -
        jsr eth_init
        bcc +
        jsr puts
        .null "no RR-Net at $de00", 13
-       jmp -
+       jsr puts
        .null "dhcp "
        jsr dhcp_start
        lda ticks+1
        sta _at
_dhcp   jsr net_poll
        lda dhcp_state
        cmp #3
        beq _bound
        lda ticks+1             ; ~4 s: again
        sec
        sbc _at
        cmp #1
        bcc _dhcp
        lda ticks+1
        sta _at
        lda #'.'
        jsr putc
        jsr dhcp_start
        jmp _dhcp
_bound  ldx #<my_ip
        ldy #>my_ip
        jsr putip
        jsr puts
        .null 13, "router "
        ldx #<net_gw
        ldy #>net_gw
        jsr putip
        jsr puts
        .null " dns "
        ldx #<net_dns
        ldy #>net_dns
        jsr putip
        jsr puts
        .null 13, "localhost = "
        lda #<_name
        sta net_q
        lda #>_name
        sta net_q+1
        jsr dns_query
-       jsr net_poll
        jsr dns_poll
        lda dns_state
        cmp #1
        beq -
        cmp #2
        beq +
        jsr puts
        .null "not found", 13
-       jmp -
+       ldx #<dns_ip
        ldy #>dns_ip
        jsr putip
        jsr puts
        .null 13, "tcp "
        #cp4 tx_dst, dns_ip
        lda #>7777
        sta tcp_rport
        lda #<7777
        sta tcp_rport+1
        jsr tcp_connect
-       jsr net_poll
        jsr tcp_poll
        lda tcp_state
        cmp #TCP_SYNSENT
        beq -
        cmp #TCP_OPEN
        beq +
        jsr puts
        .null "failed", 13
-       jmp -
+       jsr puts
        .null "open", 13
        lda #<_hello
        sta net_q
        lda #>_hello
        sta net_q+1
        lda #_hello_n
        jsr tcp_write
_echo   jsr net_poll
        jsr tcp_poll
-       jsr tcp_read
        bcs +
        jsr putc
        jmp -
+       lda tcp_state
        bne _echo
        jsr puts
        .null 13, "closed", 13
-       jmp -

_mac    .byte $02, $c6, $4c, $64, $00, $02
_name   .null "localhost"
_hello  .text "hello from the c64", 13, 10
_hello_n = * - _hello
        .section bss
_at     .fill 1
        .send bss
        .send code
