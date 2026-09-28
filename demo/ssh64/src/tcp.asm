; SPDX-License-Identifier: GPL-2.0-or-later
; Copyright (C) 2026 SampoSoft - Francesco Sampoli
;
; TCP: one connection, as a client (RFC 793, 1122, 6298 for the timer).
;
; Received data goes, in order only, to a 2 KB ring that the application
; reads; the window we announce is the free space of that ring, so the
; server never sends more than fits. Data to send waits in another 2 KB
; ring until acknowledged; if nothing is acknowledged for tcp_rto ticks,
; everything unacknowledged is sent again (go-back-N) and the time
; doubles, up to 32 s, 10 times. Sequence numbers are kept little-endian.
;
;   tcp_connect   tx_dst = address, tcp_rport = port (big-endian)
;   tcp_poll      sends what there is to send, timers
;   tcp_write     A bytes at (net_q) into the send ring (carry set: no room)
;   tcp_read      a byte from the receive ring in A (carry set: none)

TCP_CLOSED = 0
TCP_SYNSENT = 1
TCP_OPEN = 2
TCP_FIN = 3                     ; our FIN sent

RING    = 1024                  ; both rings

        .section bss
        .align 256
tcp_rxbuf .fill RING
tcp_txbuf .fill RING
tcp_state .fill 1
tcp_err .fill 1                 ; 1 reset, 2 timed out
tcp_ip  .fill 4
tcp_rport .fill 2               ; big-endian
tcp_lport .fill 2               ; big-endian
snd_una .fill 4
snd_nxt .fill 4
rcv_nxt .fill 4
snd_wnd .fill 2
tcp_mss .fill 2
rx_head .fill 2                 ; bytes ever written to / read from the ring
rx_tail .fill 2
tx_una  .fill 2                 ; ring position of snd_una
tx_len  .fill 2                 ; bytes in the send ring
tx_sent .fill 2                 ; of which sent (= snd_nxt - snd_una)
tcp_wndsent .fill 2             ; window in our last segment
tcp_fin .fill 1                 ; the server closed
tcp_rto .fill 2                 ; ticks
tcp_timer .fill 2               ; deadline, 0 = off
tcp_tries .fill 1
seg_flags .fill 1
seg_len .fill 2                 ; data in the segment
seg_seq .fill 4
seg_ack .fill 4
seg_data .fill 2                ; where its data starts
        .send bss

TCP_FIN_F = $01
TCP_SYN = $02
TCP_RST = $04
TCP_PSH = $08
TCP_ACK = $10

; 32-bit little-endian helpers
add32   .macro d, a
        clc
        .for k = 0, k < 4, k += 1
        lda \d+k
        adc \a+k
        sta \d+k
        .next
        .endm

; seg_seq = network order 4 bytes at 'src'
get32be .macro d, src
        .for k = 0, k < 4, k += 1
        lda \src+3-k
        sta \d+k
        .next
        .endm

tcp_connect .proc
        #cp4 tcp_ip, tx_dst
        lda $d41b               ; our port 49152 + random, ISS random
        ora #$c0
        sta tcp_lport
        lda $dc04
        eor ticks
        sta tcp_lport+1
        ldx #3
-       lda $d41b
        eor $dc04,x
        eor ticks
        sta snd_una,x
        sta snd_nxt,x
        dex
        bpl -
        lda #0
        ldx #rx_tail-rx_head+1
-       sta rx_head,x           ; rings empty
        dex
        bpl -
        sta tx_una
        sta tx_una+1
        sta tx_len
        sta tx_len+1
        sta tx_sent
        sta tx_sent+1
        sta tcp_fin
        sta tcp_err
        sta tcp_tries
        lda #<536
        sta tcp_mss
        lda #>536
        sta tcp_mss+1
        lda #TCP_SYNSENT
        sta tcp_state
        lda #60                 ; 1 s
        sta tcp_rto
        lda #0
        sta tcp_rto+1
        jsr tcp_syn
        jmp tcp_arm
        .pend

; sends the SYN (sequence snd_una)
tcp_syn .proc
        lda #TCP_SYN
        sta seg_flags
        #cp4 seg_seq, snd_una
        lda #0
        sta seg_len
        sta seg_len+1
        jmp tcp_segment
        .pend

; timer to now + tcp_rto
tcp_arm .proc
        lda ticks
        clc
        adc tcp_rto
        sta tcp_timer
        lda ticks+1
        adc tcp_rto+1
        sta tcp_timer+1
        ora tcp_timer           ; 0 means off: move by one
        bne +
        inc tcp_timer
+       rts
        .pend

; carry set if the timer is on and has expired
tcp_expired .proc
        lda tcp_timer
        ora tcp_timer+1
        beq _no
        lda ticks
        sec
        sbc tcp_timer
        lda ticks+1
        sbc tcp_timer+1
        bmi _no                 ; ticks < timer
        sec
        rts
_no     clc
        rts
        .pend

; free space of the receive ring: net_t (2)
rx_free .proc
        lda rx_head
        sec
        sbc rx_tail
        sta net_t
        lda rx_head+1
        sbc rx_tail+1
        sta net_t+1
        lda #<RING
        sec
        sbc net_t
        sta net_t
        lda #>RING
        sbc net_t+1
        sta net_t+1
        rts
        .pend

; builds and sends a segment: seg_flags, seg_seq, seg_len bytes of the
; send ring from position net_t+2 (2); ACK and window always
tcp_segment .proc
        lda tcp_lport
        sta TIP+20
        lda tcp_lport+1
        sta TIP+21
        lda tcp_rport
        sta TIP+22
        lda tcp_rport+1
        sta TIP+23
        .for k = 0, k < 4, k += 1
        lda seg_seq+3-k
        sta TIP+24+k
        lda rcv_nxt+3-k
        sta TIP+28+k
        .next
        lda seg_flags
        ldx #$50                ; header 20 bytes
        and #TCP_SYN
        beq +
        ldx #$60                ; 24 with the MSS option
+       stx TIP+32
        lda seg_flags
        and #TCP_SYN
        bne +
        lda seg_flags
        ora #TCP_ACK
        sta seg_flags
+       lda seg_flags
        sta TIP+33
        jsr rx_free
        lda net_t
        sta tcp_wndsent
        sta TIP+35
        lda net_t+1
        sta tcp_wndsent+1
        sta TIP+34
        lda #0
        sta TIP+36              ; checksum
        sta TIP+37
        sta TIP+38              ; urgent
        sta TIP+39
        lda #20                 ; header length
        sta net_len
        lda #0
        sta net_len+1
        lda TIP+32
        cmp #$60
        bne +
        lda #2                  ; MSS 1460
        sta TIP+40
        lda #4
        sta TIP+41
        lda #>1460
        sta TIP+42
        lda #<1460
        sta TIP+43
        lda #24
        sta net_len
+       lda seg_len             ; data from the ring
        ora seg_len+1
        beq _sum
        lda #<(TIP+40)
        sta net_q
        lda #>(TIP+40)
        sta net_q+1
        lda net_t+2
        sta net_p
        lda net_t+3
        sta net_p+1
        lda seg_len
        sta net_n
        lda seg_len+1
        sta net_n+1
        jsr ring_out
        lda net_len
        clc
        adc seg_len
        sta net_len
        lda net_len+1
        adc seg_len+1
        sta net_len+1
_sum    #cp4 tx_dst, tcp_ip
        #cp4 TIP+12, my_ip      ; (ip_send fills the rest of the header)
        #cp4 TIP+16, tcp_ip
        jsr ck_clear            ; pseudo header: addresses, length, 6
        lda #8
        ldx #<(TIP+12)
        ldy #>(TIP+12)
        jsr ck_addxy
        lda net_ck
        clc
        adc net_len
        sta net_ck
        lda net_ck+1
        adc net_len+1
        sta net_ck+1
        bcc +
        inc net_ck+2
+       lda net_ck
        clc
        adc #6
        sta net_ck
        bcc +
        inc net_ck+1
        bne +
        inc net_ck+2
+       lda #<(TIP+20)
        sta net_p
        lda #>(TIP+20)
        sta net_p+1
        lda net_len
        sta net_n
        lda net_len+1
        sta net_n+1
        jsr ck_add
        jsr ck_end
        sta TIP+36
        stx TIP+37
        lda #6
        sta tx_proto
        jmp ip_send
        .pend

; copies net_n bytes of the send ring from position (net_p) (an offset
; 0..RING-1) to (net_q)
ring_out .proc
        lda net_p+1
        and #>(RING-1)
        clc
        adc #>tcp_txbuf
        sta net_p+1
-       lda net_n
        ora net_n+1
        beq _done
        ldy #0
        lda (net_p),y
        sta (net_q),y
        inc net_q
        bne +
        inc net_q+1
+       inc net_p
        bne +
        inc net_p+1
        lda net_p+1
        cmp #>(tcp_txbuf+RING)
        bne +
        lda #>tcp_txbuf
        sta net_p+1
+       lda net_n
        bne +
        dec net_n+1
+       dec net_n
        jmp -
_done   rts
        .pend

; an ACK now (sequence snd_nxt)
tcp_ack .proc
        lda #TCP_ACK
        sta seg_flags
        #cp4 seg_seq, snd_nxt
        lda #0
        sta seg_len
        sta seg_len+1
        jmp tcp_segment
        .pend

; a segment for our connection? (IP packet in net_rx)
tcp_in  .proc
        lda tcp_state
        bne +
        rts
+       ldx #3
-       lda IP+12,x
        cmp tcp_ip,x
        bne _no
        dex
        bpl -
        lda IP+20
        cmp tcp_rport
        bne _no
        lda IP+21
        cmp tcp_rport+1
        bne _no
        lda IP+22
        cmp tcp_lport
        bne _no
        lda IP+23
        cmp tcp_lport+1
        beq +
_no     rts
+       ; checksum: pseudo header + segment must sum to $ffff
        lda IP+2                ; TCP length = total - 20
        sta net_t+1
        lda IP+3
        sec
        sbc #20
        sta net_t
        bcs +
        dec net_t+1
+       jsr ck_clear
        lda #8
        ldx #<(IP+12)
        ldy #>(IP+12)
        jsr ck_addxy
        lda net_ck
        clc
        adc net_t
        sta net_ck
        lda net_ck+1
        adc net_t+1
        sta net_ck+1
        bcc +
        inc net_ck+2
+       lda net_ck
        clc
        adc #6
        sta net_ck
        bcc +
        inc net_ck+1
        bne +
        inc net_ck+2
+       lda #<(IP+20)
        sta net_p
        lda #>(IP+20)
        sta net_p+1
        lda net_t
        sta net_n
        lda net_t+1
        sta net_n+1
        jsr ck_add
        jsr ck_end
        cmp #0
        bne _no2
        cpx #0
        beq _good
_no2    rts
_good   ; header fields
        lda IP+33
        sta seg_flags
        #get32be seg_seq, IP+24
        #get32be seg_ack, IP+28
        lda IP+32               ; data offset
        lsr a
        lsr a
        sta net_t+2             ; header bytes
        lda net_t
        sec
        sbc net_t+2
        sta seg_len
        lda net_t+1
        sbc #0
        sta seg_len+1
        lda net_t+2
        clc
        adc #<(IP+20)
        sta seg_data
        lda #>(IP+20)
        adc #0
        sta seg_data+1
        lda seg_flags
        and #TCP_RST
        beq +
        lda #TCP_CLOSED
        sta tcp_state
        lda #1
        sta tcp_err
        rts
+       lda tcp_state
        cmp #TCP_SYNSENT
        bne _open
        ; SYN+ACK acknowledging our SYN
        lda seg_flags
        and #TCP_SYN|TCP_ACK
        cmp #TCP_SYN|TCP_ACK
        bne _drop
        #cp4 net_t, snd_una     ; ack = ISS + 1 ?
        inc net_t
        bne +
        inc net_t+1
        bne +
        inc net_t+2
        bne +
        inc net_t+3
+       ldx #3
-       lda seg_ack,x
        cmp net_t,x
        bne _drop
        dex
        bpl -
        #cp4 snd_una, seg_ack
        #cp4 snd_nxt, seg_ack
        #cp4 rcv_nxt, seg_seq
        inc rcv_nxt             ; the SYN takes one
        bne +
        inc rcv_nxt+1
        bne +
        inc rcv_nxt+2
        bne +
        inc rcv_nxt+3
+       jsr _mss
        jsr _window
        lda #TCP_OPEN
        sta tcp_state
        lda #0
        sta tcp_timer
        sta tcp_timer+1
        sta tcp_tries
        jmp tcp_ack
_drop   rts

_open   ; acknowledgement: snd_una < ack <= snd_nxt
        lda seg_flags
        and #TCP_ACK
        beq _data
        sec                     ; net_t = ack - snd_una, below 64 K
        .for k = 0, k < 4, k += 1
        lda seg_ack+k
        sbc snd_una+k
        sta net_t+k
        .next
        lda net_t+2
        ora net_t+3
        bne _wnd
        lda net_t
        ora net_t+1
        beq _wnd                ; nothing new
        lda tx_sent             ; acked <= sent (FIN counts as one more)
        cmp net_t
        lda tx_sent+1
        sbc net_t+1
        bcs +
        lda tcp_state           ; only the FIN past the data
        cmp #TCP_FIN
        bne _wnd
        lda #TCP_CLOSED
        sta tcp_state
        rts
+       #cp4 snd_una, seg_ack
        lda tx_sent
        sec
        sbc net_t
        sta tx_sent
        lda tx_sent+1
        sbc net_t+1
        sta tx_sent+1
        lda tx_len
        sec
        sbc net_t
        sta tx_len
        lda tx_len+1
        sbc net_t+1
        sta tx_len+1
        lda tx_una
        clc
        adc net_t
        sta tx_una
        lda tx_una+1
        adc net_t+1
        and #>(RING-1)
        sta tx_una+1
        lda #0                  ; progress: timer again, from 1 s
        sta tcp_tries
        sta tcp_rto+1
        lda #60
        sta tcp_rto
        lda #0
        sta tcp_timer
        sta tcp_timer+1
        lda tx_sent
        ora tx_sent+1
        beq _wnd
        jsr tcp_arm
_wnd    jsr _window
_data   lda seg_len
        ora seg_len+1
        bne +
        lda seg_flags
        and #TCP_FIN_F
        bne +
        rts                     ; a bare ACK: nothing to answer
+       ldx #3                  ; in order?
-       lda seg_seq,x
        cmp rcv_nxt,x
        bne _ooo
        dex
        bpl -
        jsr rx_free             ; as much as fits
        lda seg_len
        cmp net_t
        lda seg_len+1
        sbc net_t+1
        bcc +
        lda net_t
        sta seg_len
        lda net_t+1
        sta seg_len+1
        lda #0                  ; (a FIN behind data we drop: not yet)
        sta seg_flags
+       jsr _store
        lda rcv_nxt
        clc
        adc seg_len
        sta rcv_nxt
        lda rcv_nxt+1
        adc seg_len+1
        sta rcv_nxt+1
        bcc +
        inc rcv_nxt+2
        bne +
        inc rcv_nxt+3
+       lda seg_flags
        and #TCP_FIN_F
        beq _ack
        lda tcp_fin
        bne _ack
        lda #1
        sta tcp_fin
        inc rcv_nxt
        bne _ack
        inc rcv_nxt+1
        bne _ack
        inc rcv_nxt+2
        bne _ack
        inc rcv_nxt+3
_ack    jmp tcp_ack
_ooo    jmp tcp_ack             ; out of order: where we are

_window lda IP+34
        sta snd_wnd+1
        lda IP+35
        sta snd_wnd
        rts

_mss    lda net_t+2             ; options: MSS (kind 2)
        cmp #24
        bcc +
        lda IP+40
        cmp #2
        bne +
        lda IP+42
        sta tcp_mss+1
        lda IP+43
        sta tcp_mss
        lda tcp_mss+1           ; at most 536 (the size of net_tx)
        cmp #>536
        bcc +
        bne _m536
        lda tcp_mss
        cmp #<536
        bcc +
_m536   lda #<536
        sta tcp_mss
        lda #>536
        sta tcp_mss+1
+       rts

_store  lda seg_data            ; seg_len bytes into the receive ring
        sta net_p
        lda seg_data+1
        sta net_p+1
        lda seg_len
        sta net_n
        lda seg_len+1
        sta net_n+1
-       lda net_n
        ora net_n+1
        beq _stored
        lda rx_head+1
        and #>(RING-1)
        clc
        adc #>tcp_rxbuf
        sta net_q+1
        lda rx_head
        sta net_q
        ldy #0
        lda (net_p),y
        sta (net_q),y
        inc rx_head
        bne +
        inc rx_head+1
+       inc net_p
        bne +
        inc net_p+1
+       lda net_n
        bne +
        dec net_n+1
+       dec net_n
        jmp -
_stored rts
        .pend

; sends new data, handles the timer
tcp_poll .proc
        lda tcp_state
        cmp #TCP_SYNSENT
        bne _open
        jsr tcp_expired
        bcc _ret
        jsr _backoff
        bcs _ret
        jsr tcp_syn
        jmp tcp_arm
_ret    rts
_open   cmp #TCP_OPEN
        bcc _ret
        jsr tcp_expired         ; nothing acknowledged in time: again
        bcc _new
        jsr _backoff
        bcs _ret
        lda #0
        sta tx_sent
        sta tx_sent+1
        #cp4 snd_nxt, snd_una
        jsr tcp_arm
_new    ; unsent = tx_len - tx_sent, room = snd_wnd - tx_sent
        lda tx_len
        sec
        sbc tx_sent
        sta net_t
        lda tx_len+1
        sbc tx_sent+1
        sta net_t+1
        ora net_t
        beq _upd
        lda snd_wnd
        sec
        sbc tx_sent
        sta seg_len
        lda snd_wnd+1
        sbc tx_sent+1
        sta seg_len+1
        bcc _upd                ; window full
        ora seg_len
        beq _upd
        lda net_t               ; len = min(unsent, room, mss)
        cmp seg_len
        lda net_t+1
        sbc seg_len+1
        bcs +
        lda net_t
        sta seg_len
        lda net_t+1
        sta seg_len+1
+       lda tcp_mss
        cmp seg_len
        lda tcp_mss+1
        sbc seg_len+1
        bcs +
        lda tcp_mss
        sta seg_len
        lda tcp_mss+1
        sta seg_len+1
+       #cp4 seg_seq, snd_nxt
        lda tx_una              ; ring position: una + sent
        clc
        adc tx_sent
        sta net_t+2
        lda tx_una+1
        adc tx_sent+1
        sta net_t+3
        lda #TCP_ACK|TCP_PSH
        sta seg_flags
        jsr tcp_segment
        bcs _ret                ; no next hop yet: later
        lda tx_sent             ; timer if this is the first in flight
        ora tx_sent+1
        bne +
        jsr tcp_arm
+       lda tx_sent
        clc
        adc seg_len
        sta tx_sent
        lda tx_sent+1
        adc seg_len+1
        sta tx_sent+1
        lda snd_nxt
        clc
        adc seg_len
        sta snd_nxt
        lda snd_nxt+1
        adc seg_len+1
        sta snd_nxt+1
        bcc +
        inc snd_nxt+2
        bne +
        inc snd_nxt+3
+       rts
_upd    ; the window we announced grew by half the ring: say so
        jsr rx_free
        lda net_t
        sec
        sbc tcp_wndsent
        lda net_t+1
        sbc tcp_wndsent+1
        cmp #>(RING/2)
        bcc +
        jmp tcp_ack
+       rts

_backoff                        ; next try: carry set when given up
        inc tcp_tries
        lda tcp_tries
        cmp #11
        bcc +
        lda #TCP_CLOSED
        sta tcp_state
        lda #2
        sta tcp_err
        sec
        rts
+       asl tcp_rto             ; double, at most 32 s
        rol tcp_rto+1
        lda tcp_rto+1
        cmp #>(32*60)
        bcc +
        lda #<(32*60)
        sta tcp_rto
        lda #>(32*60)
        sta tcp_rto+1
+       clc
        rts
        .pend

; ends the connection: a reset to the server, if there was one
tcp_close .proc
        lda tcp_lport
        beq +
        lda #TCP_RST
        sta seg_flags
        #cp4 seg_seq, snd_nxt
        lda #0
        sta seg_len
        sta seg_len+1
        jsr tcp_segment
        lda #0
        sta tcp_lport
+       lda #TCP_CLOSED
        sta tcp_state
        rts
        .pend

; A bytes from (net_q) into the send ring; carry set if they do not fit
tcp_write .proc
        sta net_n
        lda #<RING              ; free = RING - tx_len
        sec
        sbc tx_len
        tax
        lda #>RING
        sbc tx_len+1
        bne +                   ; 256 or more
        cpx net_n
        bcs +
        sec                     ; no room
        rts
+       lda net_n
        beq _done
        ldy #0
-       lda tx_una              ; position una + len
        clc
        adc tx_len
        sta net_p
        lda tx_una+1
        adc tx_len+1
        and #>(RING-1)
        clc
        adc #>tcp_txbuf
        sta net_p+1
        lda (net_q),y
        sty net_t
        ldy #0
        sta (net_p),y
        ldy net_t
        inc tx_len
        bne +
        inc tx_len+1
+       iny
        cpy net_n
        bne -
_done   clc
        rts
        .pend

; bytes waiting in the receive ring: net_t (2)
tcp_avail .proc
        lda rx_head
        sec
        sbc rx_tail
        sta net_t
        lda rx_head+1
        sbc rx_tail+1
        sta net_t+1
        rts
        .pend

; next received byte in A; carry set if there is none
tcp_read .proc
        lda rx_tail
        cmp rx_head
        bne +
        lda rx_tail+1
        cmp rx_head+1
        bne +
        sec
        rts
+       lda rx_tail+1
        and #>(RING-1)
        clc
        adc #>tcp_rxbuf
        sta _ld+2
        ldx rx_tail
_ld     lda $ff00,x
        inc rx_tail
        bne +
        inc rx_tail+1
+       clc
        rts
        .pend
