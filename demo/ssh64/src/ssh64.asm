; SPDX-License-Identifier: GPL-2.0-or-later
; Copyright (C) 2026 SampoSoft - Francesco Sampoli
;
; ssh64: an SSH client for the Commodore 64 with an RR-Net compatible
; Ethernet cartridge.
;
; Build: 64tass -C -a -B -o build/ssh64.prg src/ssh64.asm
;
; Memory: code from $0801, then variables (bssl) up to $9fff; $a000-$bfff
; holds the multiplication tables and the key exchange's numbers, then the
; GHASH tables of the session; font $c000, screen $c800; more variables
; (bss) from $e000. BASIC and the KERNAL are off.

        .cpu "6502"
        .include "zp.asm"

gcm_m0  = $a000                 ; GHASH tables, 4 KB aligned
gcm_m1  = $b000
kh_buf  = $cc00                 ; known hosts (1 KB)

        * = $0801
        .word +, 2026
        .null $9e, format("%d", start)
+       .word 0

start   sei
        ldx #$ff
        txs
        lda #$34                ; the tables under the I/O
        sta $01
        ldx #0
-       .for k = 0, k < (reloc_len + 255) / 256, k += 1
        lda reloc_src+256*k,x
        sta $d000+256*k,x
        .next
        inx
        bne -
        lda #$37
        sta $01
        lda #<$e000             ; variables start at zero
        ldx #>$e000
        ldy #>(bss_end+255)
        jsr _zero
        jsr boot                ; the set-up, then the variables over it
        lda #<reloc_src         ; (and over the tables' copy in the program)
        ldx #>reloc_src
        ldy #>(bssl_end+255)
        jsr _zero
        jmp main
_zero   sta $fb                 ; from A/X up to page Y
        stx $fc
        sty $fd
        ldy #0
        tya
-       sta ($fb),y
        iny
        bne -
        inc $fc
        ldx $fc
        cpx $fd
        bne -
        rts

start_end
        ; the page-aligned tables first, so that nothing is lost to the
        ; alignment: small routines fill the space up to them
        .align 256
        .include "aes_tab.asm"

        .dsection code
code_end
        .virtual reloc_src
        .dsection bssl
bssl_end
        .endv
        .cerror bssl_end > $a000, "code and low variables past $a000: ", bssl_end
        .virtual $a000
        .dsection kbss
        .dsection ktail
kbss_end
        .endv
        .cerror kbss_end > $c000, "key exchange area too long"
        .virtual $e000
        .dsection bss
bss_end
        .endv
        .cerror bss_end > $fffa, "variables past $fffa"

        .section bss
        .align 256
gcm_rk0 .fill 256               ; AES round keys of the two directions
gcm_rk1 .fill 256
ssh_host .fill 64               ; length, name
ssh_port .fill 2                ; big-endian
host_ip .fill 4
        .send bss

        .section code
        .include "sys.asm"
        .include "kbd.asm"
        .include "rng.asm"
        .include "term.asm"
        .include "fmt.asm"
        .include "ui.asm"
        .include "net.asm"
        .include "tcp.asm"
        .include "tables.asm"
        .include "fe25519.asm"
        .include "feprog.asm"
        .include "x25519.asm"
        .include "sha256.asm"
        .include "sha512.asm"
        .include "ed25519.asm"
        .include "ed25519_tab.asm"
        .include "aes.asm"
        .include "gcm.asm"
        .include "ssh.asm"
        .include "disk.asm"
        .include "kh.asm"

; memory free while X25519 runs, for its table (x25519.asm): address, 33-byte
; entries. The slots it does not use and the rest of the key exchange area
; (SHA-512 and Ed25519 run after it), the font (the screen is off; it is
; made again), the packet read (already taken apart) with the version
; after it; the addresses of the entries in the packet buffer being built
MX1_A   = fe_slots + 32 * 29
mx1_areas
        .word MX1_A
        .byte ($c000 - MX1_A) / 33
        .word FONT
        .byte $800 / 33
        .word ssh_ibuf
        .byte (SSH_MAXIN + 256) / 33
        .cerror ($c000 - MX1_A) / 33 + $800 / 33 + (SSH_MAXIN + 256) / 33 < 255, "no room for the table of X25519"
        .cerror ssh_vs != ssh_ibuf + SSH_MAXIN, "the version must follow the packet buffer"
MX1_PTR = ssh_obuf
        .cerror SSH_MAXOUT < 512, "no room for the table's addresses"

main    jsr term_init
        jsr puts
        .null "ssh64 0.1 - SSH for the Commodore 64", 13
        jsr kh_load
        ; a random locally administered MAC address
        jsr rng_secret
        lda #$02
        sta my_mac
        lda #$c6
        sta my_mac+1
        lda #$4c
        sta my_mac+2
        lda sha256_out
        sta my_mac+3
        lda sha256_out+1
        sta my_mac+4
        lda sha256_out+2
        sta my_mac+5
        jsr eth_init
        bcc +
        jsr puts
        .null "no RR-Net cartridge at $de00", 13
-       jmp -
+       lda #0
        ldx #19
-       sta my_ip,x             ; my_ip, mask, router, DNS
        dex
        bpl -
        jsr dhcp_start
        lda ticks+1
        sta dhcp_at
        lda #<sys_netpoll       ; the network between field operations
        sta fe_idle+1
        lda #>sys_netpoll
        sta fe_idle+2
_again  jsr puts
        .null 13, "host: "
        ldx #<ssh_host
        ldy #>ssh_host
        lda #60
        clc
        jsr ui_input
        lda ssh_host
        beq _again
        jsr puts
        .null 13, "user: "
        ldx #<ssh_user
        ldy #>ssh_user
        lda #60
        clc
        jsr ui_input
        jsr puts
        .null 13, "password: "
        ldx #<ssh_pass
        ldy #>ssh_pass
        lda #60
        sec
        jsr ui_input
        jsr ui_nl
        jsr kh_setname
        jsr host_port
        lda #0                  ; a new key pair for this connection
        sta ssh_havekey
_known  lda #0
        sta kh_new
        jsr kh_prepare          ; a known host: the comb of its key
        jsr connect
        jsr tcp_close
        jsr ui_nl
        lda ssh_err
        ora ssh_err+1
        beq +
        lda ssh_err
        sta str_p
        lda ssh_err+1
        sta str_p+1
        jsr _pstr
+       lda kh_new              ; a new host accepted: again, as a known one
        bne _known
        jsr puts
        .null 13, "connection closed", 13
        jmp _again

_pstr   ldy #0                  ; zero-terminated string at str_p
-       lda (str_p),y
        beq +
        jsr putc
        iny
        bne -
+       rts

; network in the background while questions are asked: DHCP again
; every 4 s until it answers
ui_idle .proc
        jsr net_poll
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

; host[:port] -> address, key pair, TCP, SSH; ssh_err on failure
connect .proc
        lda #0
        sta ssh_err
        sta ssh_err+1
        jsr ui_status
        .null "waiting for DHCP"
-       jsr ui_idle
        lda dhcp_state
        cmp #3
        bne -
        jsr puts
        .null ": "
        ldx #<my_ip
        ldy #>my_ip
        jsr putip
        jsr parse_ip
        bcc _ip
        jsr ui_status
        .null "looking up the name"
        lda #<(ssh_host+1)
        sta net_q
        lda #>(ssh_host+1)
        sta net_q+1
        jsr dns_query
-       jsr net_poll
        jsr dns_poll
        lda dns_state
        cmp #1
        beq -
        cmp #2
        beq +
        lda #<_m_dns
        ldx #>_m_dns
        jmp ssh_fail
+       #cp4 host_ip, dns_ip
_ip     jsr puts
        .null ": "
        ldx #<host_ip
        ldy #>host_ip
        jsr putip
        ; our key for this connection (kept for the one after a new host)
        jsr build_tables
        jsr rng_secret          ; the mask of the inversions
        ldx #31
-       lda sha256_out,x
        sta fe_mask,x
        dex
        bpl -
        lda ssh_havekey
        bne _tcp
        inc ssh_havekey
        jsr ui_status
        .null "making a key"
        jsr rng_secret
        ldx #31
-       lda sha256_out,x
        sta ssh_priv,x
        sta x25519_k,x
        dex
        bpl -
        lda #14                 ; light blue
        jsr sys_crunch
        jsr x25519_base
        jsr sys_awake
        ldx #31
-       lda x25519_out,x
        sta ssh_qc,x
        dex
        bpl -
_tcp
        ; TCP
        jsr ui_status
        .null "connecting"
        #cp4 tx_dst, host_ip
        lda ssh_port
        sta tcp_rport
        lda ssh_port+1
        sta tcp_rport+1
        jsr tcp_connect
-       jsr net_poll
        jsr tcp_poll
        lda tcp_state
        cmp #TCP_SYNSENT
        beq -
        cmp #TCP_OPEN
        beq +
        lda #<_m_tcp
        ldx #>_m_tcp
        jmp ssh_fail
+       jsr ssh_handshake
        bcs _r
        jsr ui_status
        .null "logging in"
        jsr ssh_auth
        bcs _r
        jsr ssh_channel
        bcs _r
        jsr ui_nl
        jsr ssh_session
_r      rts
_m_dns  .null "name not found"
_m_tcp  .null "no connection"
        .pend

; splits ":port" off ssh_host (default 22), and ends the name with 0
host_port .proc
        lda #0
        sta ssh_port
        lda #22
        sta ssh_port+1
        ldx #0
-       cpx ssh_host
        beq _end
        lda ssh_host+1,x
        cmp #':'
        beq _port
        inx
        bne -
_port   txa                     ; the name stops here
        pha
        lda #0
        sta ssh_port+1
        inx
-       cpx ssh_host
        beq _pend
        lda ssh_host+1,x
        sec
        sbc #'0'
        cmp #10
        bcs _pend
        sta sys_t               ; port = 10 port + digit
        lda ssh_port+1
        sta sys_t+1
        lda ssh_port
        pha
        asl ssh_port+1
        rol ssh_port
        asl ssh_port+1
        rol ssh_port
        lda ssh_port+1
        clc
        adc sys_t+1
        sta ssh_port+1
        pla
        adc ssh_port
        sta ssh_port
        asl ssh_port+1
        rol ssh_port
        lda ssh_port+1
        clc
        adc sys_t
        sta ssh_port+1
        bcc +
        inc ssh_port
+       inx
        bne -
_pend   pla
        sta ssh_host
        tax
_end    lda #0
        sta ssh_host+1,x
        rts
        .pend

; ssh_host as a.b.c.d into host_ip; carry set if it is not one
parse_ip .proc
        ldx #0                  ; x: character, y: byte
        ldy #0
        lda #0
        sta host_ip
_ch     lda ssh_host+1,x
        beq _end
        inx
        cmp #'.'
        beq _dot
        sec
        sbc #'0'
        cmp #10
        bcs _no
        sta sys_t
        lda host_ip,y           ; 10 b + digit
        asl a
        asl a
        adc host_ip,y
        asl a
        adc sys_t
        sta host_ip,y
        jmp _ch
_dot    iny
        cpy #4
        bcs _no
        lda #0
        sta host_ip,y
        jmp _ch
_end    cpy #3
        bne _no
        clc
        rts
_no     sec
        rts
        .pend

        .section bss
dhcp_at .fill 1
        .send bss

; constant tables, copied to the RAM under the I/O at the start
reloc_src
        .logical $d000
        .include "ed25519_pts.asm"
        .dsection reloc         ; (SHA-256 and SHA-512 constants)
        .here
reloc_len = * - reloc_src
        .cerror reloc_len > $1000, "tables under the I/O too long"

        ; the set-up that runs once, before the variables over it are
        ; zeroed: after the tables' copy, also under the variables
boot    jsr sys_init
        jsr kbd_init
        jsr rng_init
        jmp aes_init
        .dsection boot
boot_end
        .cerror boot_end > $a000, "set-up code past $a000"
        .send code
