; SPDX-License-Identifier: GPL-2.0-or-later
; Copyright (C) 2026 SampoSoft - Francesco Sampoli
;
; Ethernet (CS8900A on an RR-Net at $DE00), ARP, IPv4, ICMP echo, UDP, DHCP
; and DNS: what a client needs to reach one server, nothing more.
;
; Frames are read whole into net_rx and built in net_tx. One ARP entry is
; kept, for the next hop of the last destination (the router, normally).
; net_poll handles one received frame; it is also called from inside the
; long computations of the key exchange, so it only uses its own zero page.
;
; Shared with ../../retrocbbs, which defines the same zero-page names
; (net_p ... ticks) and sections (bss, bssl).

; RR-Net: the CS8900A ports with address line 3 inverted
CS_ISQ  = $de00
CS_PPTR = $de02
CS_PDATA = $de04
CS_RXTX = $de08
CS_TXCMD = $de0c
CS_TXLEN = $de0e

ETH_HDR = 14
IP_HDR  = 20
IP      = net_rx + ETH_HDR      ; received IP packet
TIP     = net_tx + ETH_HDR      ; IP packet being sent

        .section bssl
net_rx  .fill 1536
net_tx  .fill 640               ; our frames: TCP segments of at most
                                ; 536 bytes, echo replies of 511
net_rxlen .fill 2
my_mac  .fill 6
my_ip   .fill 4
net_mask .fill 4
net_gw  .fill 4
net_dns .fill 4
net_hop .fill 4                 ; next hop of the destination (tx_dst)
arp_ip  .fill 4                 ; the ARP entry
arp_mac .fill 6
arp_ok  .fill 1                 ; 0: unknown, 1: known
arp_at  .fill 2                 ; ticks of the last request
ip_id   .fill 2
tx_dst  .fill 4                 ; destination of the IP packet being sent
tx_proto .fill 1
dhcp_state .fill 1              ; 0 idle, 1 discover sent, 2 request sent, 3 bound
dhcp_xid .fill 4
dhcp_srv .fill 4
dhcp_offer .fill 4
dns_state .fill 1               ; 0 idle, 1 asked, 2 answered, 3 failed
dns_id  .fill 2
dns_ip  .fill 4
dns_name .fill 2               ; name being asked
dns_at  .fill 2                 ; ticks of the last query
dns_tries .fill 1
udp_sport .fill 2               ; big-endian
ck_carry .fill 1                ; high byte of the checksum carries
        .send bssl

; --- CS8900A ---------------------------------------------------------------

ppsel   .macro reg
        lda #<\reg
        sta CS_PPTR
        lda #>\reg
        sta CS_PPTR+1
        .endm

; carry set if there is no chip
eth_init .proc
        lda $de01               ; Retro Replay: clock port on
        ora #1
        sta $de01
        #ppsel $0000            ; product id $630e
        lda CS_PDATA
        cmp #$0e
        bne _none
        lda CS_PDATA+1
        cmp #$63
        bne _none
        ldx #0                  ; MAC address
-       txa
        clc
        adc #$58
        sta CS_PPTR
        lda #$01
        sta CS_PPTR+1
        lda my_mac,x
        sta CS_PDATA
        lda my_mac+1,x
        sta CS_PDATA+1
        inx
        inx
        cpx #6
        bne -
        #ppsel $0104            ; RxCTL: RxOK, individual, broadcast
        lda #$05
        sta CS_PDATA
        lda #$0d
        sta CS_PDATA+1
        #ppsel $0112            ; LineCTL: SerRxON, SerTxON
        lda #$d3
        sta CS_PDATA
        lda #$00
        sta CS_PDATA+1
        clc
        rts
_none   sec
        rts
        .pend

; a received frame into net_rx; carry set if none (the chip takes no
; frame longer than 1518 bytes, the buffer holds them)
eth_rx  .proc
        #ppsel $0124            ; RxEvent
        lda CS_PDATA
        lda CS_PDATA+1
        and #1                  ; RxOK
        bne +
        sec
        rts
+       lda CS_RXTX+1           ; RxStatus, high byte first
        lda CS_RXTX
        lda CS_RXTX+1           ; RxLength, high byte first
        sta net_rxlen+1
        sta net_n+1
        lda CS_RXTX
        sta net_rxlen
        clc                     ; words = (length + 1) / 2
        adc #1
        sta net_n
        bcc +
        inc net_n+1
+       lsr net_n+1
        ror net_n
        lda #<net_rx
        sta net_p
        lda #>net_rx
        sta net_p+1
        ldy #0
_word   lda net_n
        ora net_n+1
        beq _done
        lda CS_RXTX             ; low byte first
        sta (net_p),y
        iny
        lda CS_RXTX+1
        sta (net_p),y
        iny
        bne +
        inc net_p+1
+       lda net_n
        bne +
        dec net_n+1
+       dec net_n
        jmp _word
_done   clc
        rts
        .pend

; sends net_tx, net_len bytes (short frames are padded by the chip)
eth_tx  .proc
        lda #$c9                ; TxCMD: start after the whole frame
        sta CS_TXCMD
        lda #0
        sta CS_TXCMD+1
        lda net_len
        sta CS_TXLEN
        lda net_len+1
        sta CS_TXLEN+1
        #ppsel $0138            ; BusST: Rdy4TxNOW
        ldx #0
-       lda CS_PDATA
        lda CS_PDATA+1
        and #1
        bne +
        dex
        bne -
        rts                     ; the chip does not take it: dropped
+       lda #<net_tx
        sta net_p
        lda #>net_tx
        sta net_p+1
        lda net_len             ; words = (length + 1) / 2
        clc
        adc #1
        sta net_n
        lda net_len+1
        adc #0
        lsr a
        sta net_n+1
        ror net_n
        ldy #0
-       lda (net_p),y
        sta CS_RXTX
        iny
        lda (net_p),y
        sta CS_RXTX+1
        iny
        bne +
        inc net_p+1
+       lda net_n
        bne +
        dec net_n+1
+       dec net_n
        lda net_n
        ora net_n+1
        bne -
        rts
        .pend

; --- checksums (RFC 1071) ---------------------------------------------------

ck_clear .proc
        lda #0
        sta net_ck
        sta net_ck+1
        sta net_ck+2
        sta ck_carry
        rts
        .pend

; adds net_n bytes at (net_p), as big-endian 16-bit words
ck_add  .proc
        ldy #0
_loop   lda net_n+1
        bne _pair
        lda net_n
        beq _done
        cmp #1
        beq _odd
_pair   clc
        iny
        lda (net_p),y
        adc net_ck
        sta net_ck
        dey
        lda (net_p),y
        adc net_ck+1
        sta net_ck+1
        bcc +
        inc net_ck+2            ; carries: 16 bits (730 words in a segment)
        bne +
        inc ck_carry
+       iny
        iny
        bne +
        inc net_p+1
+       lda net_n
        sec
        sbc #2
        sta net_n
        bcs _loop
        dec net_n+1
        jmp _loop
_odd    lda (net_p),y           ; last byte: the high half of a word
        clc
        adc net_ck+1
        sta net_ck+1
        bcc _done
        inc net_ck+2
        bne _done
        inc ck_carry
_done   rts
        .pend

; ck_add for A bytes at X (low) / Y (high)
ck_addxy .proc
        stx net_p
        sty net_p+1
        sta net_n
        lda #0
        sta net_n+1
        jmp ck_add
        .pend

; the complemented sum: A = high byte, X = low byte
ck_end  .proc
-       lda net_ck+2            ; fold the carries back in
        ora ck_carry
        beq +
        lda net_ck
        clc
        adc net_ck+2
        sta net_ck
        lda net_ck+1
        adc ck_carry
        sta net_ck+1
        lda #0
        sta ck_carry
        adc #0
        sta net_ck+2
        jmp -
+       lda net_ck
        eor #$ff
        tax
        lda net_ck+1
        eor #$ff
        rts
        .pend

; --- IPv4 ----------------------------------------------------------------------

; zero-page copy of 4 bytes
cp4     .macro d, s
        .for k = 0, k < 4, k += 1
        lda \s+k
        sta \d+k
        .next
        .endm

; sends the IP packet in net_tx: payload of net_len bytes at TIP+20, to
; tx_dst, protocol tx_proto. Carry set if the next hop is still unknown
; (an ARP request went out instead).
ip_send .proc
        jsr arp_hop
        bcc +
        rts
+       ldx #5                  ; Ethernet header
-       lda arp_mac,x
        sta net_tx,x
        lda my_mac,x
        sta net_tx+6,x
        dex
        bpl -
        lda #$08
        sta net_tx+12
        lda #$00
        sta net_tx+13
        lda #$45                ; IP header
        sta TIP
        lda #0
        sta TIP+1
        lda net_len             ; total length
        clc
        adc #IP_HDR
        sta TIP+3
        lda net_len+1
        adc #0
        sta TIP+2
        inc ip_id
        bne +
        inc ip_id+1
+       lda ip_id+1
        sta TIP+4
        lda ip_id
        sta TIP+5
        lda #$40                ; don't fragment
        sta TIP+6
        lda #0
        sta TIP+7
        sta TIP+10
        sta TIP+11
        lda #64
        sta TIP+8
        lda tx_proto
        sta TIP+9
        #cp4 TIP+12, my_ip
        #cp4 TIP+16, tx_dst
        jsr ck_clear
        lda #IP_HDR
        ldx #<TIP
        ldy #>TIP
        jsr ck_addxy
        jsr ck_end
        sta TIP+10
        stx TIP+11
        lda net_len             ; frame length
        clc
        adc #ETH_HDR+IP_HDR
        sta net_len
        bcc +
        inc net_len+1
+       jsr eth_tx
        clc
        rts
        .pend

; next hop of tx_dst into net_hop; carry clear with its MAC in arp_mac,
; else an ARP request is sent (at most one a second) and carry set
arp_hop .proc
        lda tx_dst              ; broadcast
        and tx_dst+1
        and tx_dst+2
        and tx_dst+3
        cmp #$ff
        bne +
        ldx #5
        lda #$ff
-       sta arp_mac,x
        dex
        bpl -
        clc
        rts
+       ldx #3                  ; on our subnet?
-       lda tx_dst,x
        eor my_ip,x
        and net_mask,x
        bne _gw
        dex
        bpl -
        #cp4 net_hop, tx_dst
        jmp _look
_gw     #cp4 net_hop, net_gw
_look   ldx #3
-       lda net_hop,x
        cmp arp_ip,x
        bne _new
        dex
        bpl -
        lda arp_ok
        beq _ask
        clc
        rts
_new    #cp4 arp_ip, net_hop
        lda #0
        sta arp_ok
        jmp _req
_ask    lda ticks               ; asked less than a second ago?
        sec
        sbc arp_at
        tax
        lda ticks+1
        sbc arp_at+1
        bne _req
        cpx #60
        bcs _req
        sec
        rts
_req    lda ticks
        sta arp_at
        lda ticks+1
        sta arp_at+1
        ldx #5
-       lda #$ff
        sta net_tx,x            ; broadcast
        lda my_mac,x
        sta net_tx+6,x
        sta net_tx+22,x         ; sha
        lda #0
        sta net_tx+32,x         ; tha
        dex
        bpl -
        ldx #9
-       lda _arphdr,x
        sta net_tx+12,x
        dex
        bpl -
        #cp4 net_tx+28, my_ip
        #cp4 net_tx+38, arp_ip
        lda #42
        sta net_len
        lda #0
        sta net_len+1
        jsr eth_tx
        sec
        rts
_arphdr .byte $08, $06, $00, $01, $08, $00, $06, $04, $00, $01
        .pend

; handles the frame in net_rx, if any; carry set if there was none
net_poll .proc
        jsr eth_rx
        bcc +
        rts
+       lda net_rx+12
        cmp #$08
        bne _done
        lda net_rx+13
        beq _ip
        cmp #$06
        bne _done
        jsr arp_in
_done   clc
        rts
_ip     jsr ip_in
        clc
        rts
        .pend

arp_in  .proc
        ldx #3                  ; learn the sender if it is the one we want
-       lda net_rx+28,x         ; spa
        cmp arp_ip,x
        bne _req
        dex
        bpl -
        ldx #5
-       lda net_rx+22,x
        sta arp_mac,x
        dex
        bpl -
        lda #1
        sta arp_ok
_req    lda net_rx+21           ; a request for us: answer
        cmp #1
        bne _no
        ldx #3
-       lda net_rx+38,x
        cmp my_ip,x
        bne _no
        dex
        bpl -
        ldx #5
-       lda net_rx+22,x         ; to the asker
        sta net_tx,x
        sta net_tx+32,x
        lda my_mac,x
        sta net_tx+6,x
        sta net_tx+22,x
        dex
        bpl -
        ldx #9
-       lda arp_hop._arphdr,x
        sta net_tx+12,x
        dex
        bpl -
        lda #2                  ; reply
        sta net_tx+21
        #cp4 net_tx+28, my_ip
        #cp4 net_tx+38, net_rx+28
        lda #42
        sta net_len
        lda #0
        sta net_len+1
        jmp eth_tx
_no     rts
        .pend

; an IP packet for us: to ICMP, UDP or TCP
ip_in   .proc
        lda IP                  ; IPv4, no options
        cmp #$45
        bne _no
        lda IP+6                ; no fragments
        and #$3f
        ora IP+7
        bne _no
        lda dhcp_state          ; for us (anything while we have no address)
        cmp #3
        bne +
        ldx #3
-       lda IP+16,x
        cmp my_ip,x
        bne _no
        dex
        bpl -
+       lda IP+9
        cmp #6
        bne +
        jmp tcp_in
+       cmp #17
        beq udp_in
        cmp #1
        beq icmp_in
_no     rts
        .pend

; echo request: sent back as reply
icmp_in .proc
        lda IP+20
        cmp #8
        bne _no
        lda IP+2                ; length = IP total - 20
        sta net_len+1
        lda IP+3
        sec
        sbc #IP_HDR
        sta net_len
        bcs +
        dec net_len+1
+       lda net_len+1           ; (up to 511 bytes)
        cmp #2
        bcs _no
        lda #<(IP+20)
        sta net_p
        lda #>(IP+20)
        sta net_p+1
        lda #<(TIP+20)
        sta net_q
        lda #>(TIP+20)
        sta net_q+1
        lda net_len
        sta net_n
        lda net_len+1
        sta net_n+1
        jsr net_copy
        lda #0                  ; type 0, new checksum
        sta TIP+20
        sta TIP+22
        sta TIP+23
        jsr ck_clear
        lda #<(TIP+20)
        sta net_p
        lda #>(TIP+20)
        sta net_p+1
        lda net_len
        sta net_n
        lda net_len+1
        sta net_n+1
        jsr ck_add
        jsr ck_end
        sta TIP+22
        stx TIP+23
        #cp4 tx_dst, IP+12
        lda #1
        sta tx_proto
        jmp ip_send
_no     rts
        .pend

; copies net_n bytes from (net_p) to (net_q)
net_copy .proc
        ldy #0
        ldx net_n+1
        beq _last
-       lda (net_p),y
        sta (net_q),y
        iny
        bne -
        inc net_p+1
        inc net_q+1
        dex
        bne -
_last   ldx net_n
        beq _done
-       lda (net_p),y
        sta (net_q),y
        iny
        dex
        bne -
_done   rts
        .pend

udp_in  .proc
        lda IP+22               ; destination port
        bne _dns
        lda IP+23
        cmp #68
        bne _dns
        jmp dhcp_in
_dns    lda dns_state
        cmp #1
        bne _no
        lda IP+22
        cmp dns_id
        bne _no
        lda IP+23
        cmp dns_id+1
        bne _no
        jmp dns_in
_no     rts
        .pend

; sends the UDP datagram at TIP+28, net_len bytes of data, from port
; udp_sport to port Y
udp_send .proc
        lda #0
        sta TIP+22              ; destination port
        sty TIP+23
        lda udp_sport           ; source port
        sta TIP+20
        lda udp_sport+1
        sta TIP+21
        lda net_len             ; UDP length
        clc
        adc #8
        sta net_len
        sta TIP+25
        lda net_len+1
        adc #0
        sta net_len+1
        sta TIP+24
        lda #0                  ; no checksum
        sta TIP+26
        sta TIP+27
        lda #17
        sta tx_proto
        jmp ip_send
        .pend

; --- DHCP ------------------------------------------------------------------------

; sends DISCOVER (dhcp_state 0 -> 1) or REQUEST (2)
dhcp_send .proc
        ldx #0                  ; BOOTP header, zeros
        txa
-       sta TIP+28,x
        inx
        cpx #240
        bne -
        lda #1                  ; request, Ethernet
        sta TIP+28
        sta TIP+29
        lda #6
        sta TIP+30
        #cp4 TIP+32, dhcp_xid
        lda #$80                ; answer by broadcast
        sta TIP+38
        ldx #5
-       lda my_mac,x
        sta TIP+56,x
        dex
        bpl -
        lda #99                 ; magic cookie
        sta TIP+264
        lda #130
        sta TIP+265
        lda #83
        sta TIP+266
        lda #99
        sta TIP+267
        ldx #0
        lda dhcp_state
        cmp #2
        beq _req
        ldy #0                  ; DISCOVER
-       lda _disc,y
        sta TIP+268,y
        iny
        cpy #_disc_n
        bne -
        lda #<(240+_disc_n)
        ldx #>(240+_disc_n)
        jmp _send
_req    ldy #0
-       lda _reqo,y
        sta TIP+268,y
        iny
        cpy #_reqo_n
        bne -
        #cp4 TIP+268+5, dhcp_offer      ; requested address
        #cp4 TIP+268+11, dhcp_srv       ; server identifier
        lda #<(240+_reqo_n)
        ldx #>(240+_reqo_n)
_send   sta net_len
        stx net_len+1
        ldx #0
        stx udp_sport
        ldx #68
        stx udp_sport+1
        ldx #$ff                ; to 255.255.255.255, from 0.0.0.0
        stx tx_dst
        stx tx_dst+1
        stx tx_dst+2
        stx tx_dst+3
        ldx #68
        ldy #67
        jmp udp_send
_disc   .byte 53, 1, 1, 55, 3, 1, 3, 6, 255
_disc_n = * - _disc
_reqo   .byte 53, 1, 3, 50, 4, 0, 0, 0, 0, 54, 4, 0, 0, 0, 0, 55, 3, 1, 3, 6, 255
_reqo_n = * - _reqo
        .pend

; starts DHCP (my_ip must be 0.0.0.0)
dhcp_start .proc
        lda ticks
        eor $d41b
        sta dhcp_xid
        lda $dc04
        sta dhcp_xid+1
        lda #$c6
        sta dhcp_xid+2
        lda #$40
        sta dhcp_xid+3
        lda #1
        sta dhcp_state
        jmp dhcp_send
        .pend

; an answer from a DHCP server
dhcp_in .proc
        ldx #3                  ; our transaction
-       lda IP+32,x
        cmp dhcp_xid,x
        bne _no
        dex
        bpl -
        lda #<(IP+268)          ; options
        sta net_p
        lda #>(IP+268)
        sta net_p+1
        lda #0
        sta net_t               ; message type
_opt    ldy #0
        lda (net_p),y
        cmp #255
        beq _end
        cmp #0
        bne +
        inc net_p               ; pad
        bne _opt
        inc net_p+1
        jmp _opt
+       tax
        iny
        lda (net_p),y           ; length
        sta net_t+1
        iny
        cpx #53
        bne +
        lda (net_p),y
        sta net_t
+       cpx #54
        bne +
        ldx #0
-       lda (net_p),y
        sta dhcp_srv,x
        iny
        inx
        cpx #4
        bne -
        jmp _next
+       cpx #1
        bne +
        ldx #0
-       lda (net_p),y
        sta net_mask,x
        iny
        inx
        cpx #4
        bne -
        jmp _next
+       cpx #3
        bne +
        ldx #0
-       lda (net_p),y
        sta net_gw,x
        iny
        inx
        cpx #4
        bne -
        jmp _next
+       cpx #6
        bne _next
        ldx #0
-       lda (net_p),y
        sta net_dns,x
        iny
        inx
        cpx #4
        bne -
_next   lda net_t+1             ; past the option
        clc
        adc #2
        adc net_p
        sta net_p
        bcc _opt
        inc net_p+1
        jmp _opt
_end    lda net_t
        cmp #2                  ; OFFER
        bne +
        lda dhcp_state
        cmp #1
        bne _no
        #cp4 dhcp_offer, IP+44  ; yiaddr
        lda #2
        sta dhcp_state
        jmp dhcp_send
+       cmp #5                  ; ACK
        bne _no
        lda dhcp_state
        cmp #2
        bne _no
        #cp4 my_ip, IP+44
        lda #3
        sta dhcp_state
_no     rts
        .pend

; --- DNS ---------------------------------------------------------------------------

; asks net_dns for the A record of the name at (net_q), ending with 0;
; the answer comes in dns_state / dns_ip (dns_poll asks again)
dns_query .proc
        lda net_q
        sta dns_name
        lda net_q+1
        sta dns_name+1
        lda #0
        sta dns_tries
        lda $d41b               ; id, also our port $c6xx
        eor ticks
        sta dns_id+1
        lda #$c6
        sta dns_id
        lda #1
        sta dns_state
_send   lda dns_name
        sta net_q
        lda dns_name+1
        sta net_q+1
        lda ticks
        sta dns_at
        lda ticks+1
        sta dns_at+1
        ldx #11
-       lda _hdr,x
        sta TIP+28,x
        dex
        bpl -
        lda dns_id
        sta TIP+28
        lda dns_id+1
        sta TIP+29
        ; the name as labels: TIP+40 onwards
        ldy #0                  ; y: source, x: where the length goes
        ldx #0
_label  stx net_t               ; length byte position
        inx
        lda #0
        sta net_t+1             ; label length
-       lda (net_q),y
        beq _last
        iny
        cmp #'.'
        beq _dot
        sta TIP+40,x
        inx
        inc net_t+1
        bne -
_dot    jsr _setlen
        jmp _label
_last   jsr _setlen
        lda #0                  ; root, type A, class IN
        sta TIP+40,x
        sta TIP+41,x
        sta TIP+43,x
        lda #1
        sta TIP+42,x
        sta TIP+44,x
        txa
        clc
        adc #12+5
        sta net_len
        lda #0
        sta net_len+1
        #cp4 tx_dst, net_dns
        lda dns_id
        sta udp_sport
        lda dns_id+1
        sta udp_sport+1
        ldy #53
        jmp udp_send
_setlen stx net_t+2
        ldx net_t
        lda net_t+1
        sta TIP+40,x
        ldx net_t+2
        rts
_hdr    .byte 0, 0, $01, $00, 0, 1, 0, 0, 0, 0, 0, 0
        .pend

; while waiting: the query again every second, 8 times
dns_poll .proc
        lda dns_state
        cmp #1
        bne _no
        lda ticks
        sec
        sbc dns_at
        tax
        lda ticks+1
        sbc dns_at+1
        bne +
        cpx #60
        bcc _no
+       inc dns_tries
        lda dns_tries
        cmp #8
        bcc +
        lda #3
        sta dns_state
_no     rts
+       jmp dns_query._send
        .pend

dns_in  .proc
        lda IP+28+3             ; rcode
        and #$0f
        bne _fail
        lda IP+28+7             ; answers
        beq _fail
        sta net_t+3
        lda #<(IP+40)           ; skip the question
        sta net_p
        lda #>(IP+40)
        sta net_p+1
        jsr _name
        lda #4
        jsr _skip
_ans    jsr _name
        ldy #1                  ; type A, class IN, 4 bytes?
        lda (net_p),y
        cmp #1
        bne _other
        ldy #9
        lda (net_p),y
        cmp #4
        bne _other
        ldy #10
        ldx #0
-       lda (net_p),y
        sta dns_ip,x
        iny
        inx
        cpx #4
        bne -
        lda #2
        sta dns_state
        rts
_other  ldy #9                  ; skip type, class, ttl, length, data
        lda (net_p),y
        clc
        adc #10
        jsr _skip
        dec net_t+3
        bne _ans
_fail   lda #3
        sta dns_state
        rts

_name   ldy #0                  ; past a name (labels, or a pointer)
-       lda (net_p),y
        beq _end0
        cmp #$c0
        bcs _ptr
        sec
        adc #0                  ; length + 1
        jsr _skip
        jmp _name
_ptr    lda #2
        jmp _skip
_end0   lda #1
_skip   clc
        adc net_p
        sta net_p
        bcc +
        inc net_p+1
+       rts
        .pend
