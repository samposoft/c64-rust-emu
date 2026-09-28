; SPDX-License-Identifier: GPL-2.0-or-later
; Copyright (C) 2026 SampoSoft - Francesco Sampoli
;
; SSH-2 client (RFC 4250-4254): curve25519-sha256 (RFC 8731), ssh-ed25519
; host keys, aes128-gcm / aes256-gcm (RFC 5647), strict key exchange
; (OpenSSH's answer to Terrapin), password and keyboard-interactive
; authentication, one session channel with a pty.
;
; Packets are read whole into ssh_ibuf (at most SSH_MAXIN) and built in
; ssh_obuf; the exchange hash is computed as the parts go by.

SSH_MAXIN = 2560
SSH_MAXOUT = 640                ; our packets are small
SSH_WINDOW = 16384              ; our channel window, topped up at half

        .section bss
ssh_obuf .fill SSH_MAXOUT
        .send bss

; message numbers
MSG_DISCONNECT = 1
MSG_IGNORE = 2
MSG_UNIMPL = 3
MSG_DEBUG = 4
MSG_SERVICE_REQUEST = 5
MSG_SERVICE_ACCEPT = 6
MSG_EXT_INFO = 7
MSG_KEXINIT = 20
MSG_NEWKEYS = 21
MSG_ECDH_INIT = 30
MSG_ECDH_REPLY = 31
MSG_USERAUTH_REQUEST = 50
MSG_USERAUTH_FAILURE = 51
MSG_USERAUTH_SUCCESS = 52
MSG_USERAUTH_BANNER = 53
MSG_USERAUTH_INFO_REQUEST = 60
MSG_USERAUTH_INFO_RESPONSE = 61
MSG_GLOBAL_REQUEST = 80
MSG_REQUEST_FAILURE = 82
MSG_CHANNEL_OPEN = 90
MSG_CHANNEL_OPEN_CONFIRMATION = 91
MSG_CHANNEL_OPEN_FAILURE = 92
MSG_CHANNEL_WINDOW_ADJUST = 93
MSG_CHANNEL_DATA = 94
MSG_CHANNEL_EXTENDED_DATA = 95
MSG_CHANNEL_EOF = 96
MSG_CHANNEL_CLOSE = 97
MSG_CHANNEL_REQUEST = 98
MSG_CHANNEL_SUCCESS = 99
MSG_CHANNEL_FAILURE = 100

        .section bss
ssh_ibuf .fill SSH_MAXIN
ssh_vs  .fill 256               ; server version: length, text
        .send bss

        .section bssl
ssh_rcnt .fill 2                ; bytes of the packet read so far
ssh_rneed .fill 2               ; bytes it has in all (0: length not read)
ssh_plen .fill 2                ; payload length of the packet read
ssh_type .fill 1                ; its message number
ssh_renc .fill 1                ; receiving encrypted
ssh_senc .fill 1                ; sending encrypted
ssh_strict .fill 1
ssh_klen .fill 1                ; AES key: 16 or 32
ssh_priv .fill 32               ; our X25519 key
ssh_havekey .fill 1             ; made (ssh_priv, ssh_qc)
ssh_qc  .fill 32                ; its public half
ssh_qs  .fill 32                ; the server's
ssh_kmp .fill 34                ; K as mpint: length, bytes
ssh_h   .fill 32                ; exchange hash
ssh_sid .fill 32                ; session id
ssh_hkey .fill 32               ; host key
ssh_hsig .fill 64               ; its signature
ssh_fp  .fill 32                ; SHA-256 of the host key blob
ssh_err .fill 2                 ; message of the last error
ch_remote .fill 4               ; server's channel number
ch_lwin .fill 2                 ; what the server may still send us
ch_rwin .fill 4                 ; what we may still send
ssh_kbd_tried .fill 1
ssh_user .fill 64               ; length, name
ssh_pass .fill 64               ; length, password
ssh_keybuf .fill 32             ; keys to send
ssh_keylen .fill 1
ssh_open .fill 1                ; channel open
ssh_blk .fill 1                 ; padding block: 8 or 16
ssh_tmp .fill 4
ssh_t2  .fill 2
ssh_y   .fill 1
ssh_want .fill 1
ssh_ks  .fill 4                 ; host key blob: pointer, length
        .send bssl

; --- building packets ---------------------------------------------------------

; starts a packet of type A
ssh_begin .proc
        ldx #<(ssh_obuf+5)
        stx ssh_wp
        ldx #>(ssh_obuf+5)
        stx ssh_wp+1
        .pend
put_u8  .proc
        sty ssh_y
        ldy #0
        sta (ssh_wp),y
        ldy ssh_y
        inc ssh_wp
        bne +
        inc ssh_wp+1
+       rts
        .pend

; uint32 from 16 bits: A = high, X = low
put_u16 .proc
        pha
        lda #0
        jsr put_u8
        jsr put_u8
        pla
        jsr put_u8
        txa
        jmp put_u8
        .pend

; ssh_sl bytes at (ssh_sp), no length
put_mem .proc
-       lda ssh_sl
        ora ssh_sl+1
        beq _done
        ldy #0
        lda (ssh_sp),y
        jsr put_u8
        inc ssh_sp
        bne +
        inc ssh_sp+1
+       lda ssh_sl
        bne +
        dec ssh_sl+1
+       dec ssh_sl
        jmp -
_done   rts
        .pend

; string: length ssh_sl, bytes at (ssh_sp)
put_str .proc
        lda ssh_sl+1
        ldx ssh_sl
        jsr put_u16
        jmp put_mem
        .pend

; string whose length byte and text follow the jsr
put_istr .proc
        pla
        sta str_p
        pla
        sta str_p+1
        ldy #1
        lda (str_p),y
        sta ssh_sl
        lda #0
        sta ssh_sl+1
        lda str_p
        clc
        adc #2
        sta ssh_sp
        lda str_p+1
        adc #0
        sta ssh_sp+1
        lda str_p               ; return to the last byte of the text
        sec
        adc ssh_sl
        tax
        lda str_p+1
        adc #0
        pha
        txa
        pha
        jmp put_str
        .pend

; a string of ours (length byte, text) at X/Y, as an SSH string
put_pstr .proc
        stx ssh_sp
        sty ssh_sp+1
        ldy #0
        lda (ssh_sp),y
        sta ssh_sl
        sty ssh_sl+1
        inc ssh_sp
        bne +
        inc ssh_sp+1
+       jmp put_str
        .pend

; pads, encrypts and sends the packet
ssh_send .proc
        lda ssh_wp              ; payload length (low byte is enough)
        sec
        sbc #<(ssh_obuf+5)
        ldx ssh_senc            ; encrypted: 1 + n + pad = 0 mod 16
        beq +                   ; in clear: 4 + 1 + n + pad = 0 mod 8
        ldx #16
        clc
        adc #1
        jmp ++
+       ldx #8
        clc
        adc #5
+       stx ssh_blk
        dex
        stx ssh_tmp
        and ssh_tmp             ; used part of the last block
        sta ssh_tmp
        lda ssh_blk
        sec
        sbc ssh_tmp
        cmp #4
        bcs +
        adc ssh_blk
+       sta ssh_obuf+4          ; padding length
        tax
-       jsr rng_byte
        jsr put_u8
        dex
        bne -
        lda ssh_wp              ; packet length = wp - (obuf + 4)
        sec
        sbc #<(ssh_obuf+4)
        sta ssh_obuf+3
        sta gcm_n
        lda ssh_wp+1
        sbc #>(ssh_obuf+4)
        sta ssh_obuf+2
        sta gcm_n+1
        lda #0
        sta ssh_obuf
        sta ssh_obuf+1
        lda ssh_senc
        beq _out
        lda #<ssh_obuf
        sta gcm_p
        lda #>ssh_obuf
        sta gcm_p+1
        ldx #0
        jsr gcm_seal
        lda ssh_wp              ; + the tag
        clc
        adc #16
        sta ssh_wp
        bcc _out
        inc ssh_wp+1
_out    lda #<ssh_obuf          ; to TCP, in pieces as they fit
        sta ssh_sp
        lda #>ssh_obuf
        sta ssh_sp+1
_more   lda ssh_wp
        sec
        sbc ssh_sp
        sta ssh_sl
        lda ssh_wp+1
        sbc ssh_sp+1
        sta ssh_sl+1
        ora ssh_sl
        beq _sent
        lda ssh_sl+1
        bne +
        lda ssh_sl
        cmp #128
        bcc ++
+       lda #128
        sta ssh_sl
+       lda ssh_sp
        sta net_q
        lda ssh_sp+1
        sta net_q+1
        lda ssh_sl
        jsr tcp_write
        bcc +
        jsr ssh_net             ; no room: let the network move
        lda tcp_state
        bne _more
        rts
+       lda ssh_sp
        clc
        adc ssh_sl
        sta ssh_sp
        bcc _more
        inc ssh_sp+1
        jmp _more
_sent   jmp tcp_poll
        .pend

; the network once
ssh_net .proc
        jsr net_poll
        jsr tcp_poll
        jmp dns_poll
        .pend

; --- reading packets ------------------------------------------------------------

; reads what TCP has; carry clear when a whole packet is in ssh_ibuf
; (type in ssh_type, ssh_rp after it); ssh_err set on a fatal error
ssh_recv .proc
_loop   lda ssh_rneed           ; bytes wanted now
        ora ssh_rneed+1
        bne +
        lda #4
        ldx #0
        jmp ++
+       lda ssh_rneed
        ldx ssh_rneed+1
+       sta ssh_tmp
        stx ssh_tmp+1
-       lda ssh_rcnt            ; enough?
        cmp ssh_tmp
        lda ssh_rcnt+1
        sbc ssh_tmp+1
        bcs _have
        jsr tcp_read
        bcc +
        rts                     ; carry set: not yet
+       pha                     ; into ssh_ibuf + ssh_rcnt
        lda #<ssh_ibuf
        clc
        adc ssh_rcnt
        sta sys_t
        lda #>ssh_ibuf
        adc ssh_rcnt+1
        sta sys_t+1
        pla
        ldy #0
        sta (sys_t),y
        inc ssh_rcnt
        bne -
        inc ssh_rcnt+1
        jmp -
_have   lda ssh_rneed
        ora ssh_rneed+1
        bne _whole
        ; the length: at most SSH_MAXIN - 20, a multiple of 16 when encrypted
        lda ssh_ibuf
        ora ssh_ibuf+1
        bne _big
        lda ssh_ibuf+2
        cmp #>(SSH_MAXIN-20)
        bcc +
        bne _big
        lda ssh_ibuf+3
        cmp #<(SSH_MAXIN-20)
        bcs _big
+       lda ssh_ibuf+3
        clc
        adc #4
        sta ssh_rneed
        lda ssh_ibuf+2
        adc #0
        sta ssh_rneed+1
        lda ssh_renc
        beq _loop
        lda ssh_ibuf+3
        and #15
        bne _big
        lda ssh_rneed
        clc
        adc #16
        sta ssh_rneed
        bcc _loop
        inc ssh_rneed+1
        jmp _loop
_big    lda #<_msg_len
        ldx #>_msg_len
        jmp ssh_fail
_whole  lda #0
        sta ssh_rneed
        sta ssh_rneed+1
        sta ssh_rcnt
        sta ssh_rcnt+1
        lda ssh_renc
        beq +
        lda #<ssh_ibuf
        sta gcm_p
        lda #>ssh_ibuf
        sta gcm_p+1
        lda ssh_ibuf+3
        sta gcm_n
        lda ssh_ibuf+2
        sta gcm_n+1
        ldx #1
        jsr gcm_open
        bcc +
        lda #<_msg_mac
        ldx #>_msg_mac
        jmp ssh_fail
+       lda ssh_ibuf+3          ; payload = length - padding - 1
        sec
        sbc ssh_ibuf+4
        sta ssh_plen
        lda ssh_ibuf+2
        sbc #0
        sta ssh_plen+1
        lda ssh_plen
        bne +
        dec ssh_plen+1
+       dec ssh_plen
        lda ssh_ibuf+5
        sta ssh_type
        lda #<(ssh_ibuf+6)
        sta ssh_rp
        lda #>(ssh_ibuf+6)
        sta ssh_rp+1
        clc
        rts
_msg_len .null "packet too long"
_msg_mac .null "bad packet (MAC)"
        .pend

; carry set if the server has closed the connection and everything it
; sent has been read
ssh_eof .proc
        lda tcp_fin
        beq _no
        jsr tcp_avail
        lda net_t
        ora net_t+1
        bne _no
        sec
        rts
_no     clc
        rts
        .pend

; records an error (message at A/X), carry set
ssh_fail .proc
        sta ssh_err
        stx ssh_err+1
        jsr sys_awake           ; (in case a computation was running)
        lda #TCP_CLOSED
        sta tcp_state
        sec
        rts
        .pend

; --- parsing ----------------------------------------------------------------------

get_u8  .proc
        ldy #0
        lda (ssh_rp),y
        inc ssh_rp
        bne +
        inc ssh_rp+1
+       rts
        .pend

; uint32 into ssh_tmp (big-endian, as it is)
get_u32 .proc
        ldx #0
-       jsr get_u8
        sta ssh_tmp,x
        inx
        cpx #4
        bne -
        rts
        .pend

; string: ssh_sp, ssh_sl (lengths of 64 K or more are cut)
get_str .proc
        jsr get_u32
        lda ssh_tmp+3
        sta ssh_sl
        lda ssh_tmp+2
        sta ssh_sl+1
        lda ssh_rp
        sta ssh_sp
        clc
        adc ssh_sl
        sta ssh_rp
        lda ssh_rp+1
        sta ssh_sp+1
        adc ssh_sl+1
        sta ssh_rp+1
        rts
        .pend

; carry clear if the name list ssh_sp/ssh_sl contains the name whose
; length byte and text are at X/Y
list_has .proc
        stx str_p
        sty str_p+1
        lda ssh_sp
        sta sys_t
        lda ssh_sp+1
        sta sys_t+1
        lda ssh_sl
        sta ssh_tmp
        lda ssh_sl+1
        sta ssh_tmp+1
_name   ; compare the name here, then expect ',' or the end
        ldy #0
        lda (str_p),y
        sta ssh_tmp+2           ; name length
        ldx #0                  ; x: bytes matched
_cmp    cpx ssh_tmp+2
        beq _end
        txa                     ; still inside the list?
        cmp ssh_tmp
        lda ssh_tmp+1
        bne +
        bcs _skip
+       txa
        tay
        lda (sys_t),y
        iny
        cmp (str_p),y
        bne _skip
        inx
        bne _cmp
_end    txa                     ; whole name matched: then ',' or the end
        cmp ssh_tmp
        lda ssh_tmp+1
        bne +
        bcs _yes
+       txa
        tay
        lda (sys_t),y
        cmp #','
        beq _yes
_skip   ; to the byte after the next ','
        ldy #0
-       lda ssh_tmp
        ora ssh_tmp+1
        beq _no
        lda (sys_t),y
        pha
        inc sys_t
        bne +
        inc sys_t+1
+       lda ssh_tmp
        bne +
        dec ssh_tmp+1
+       dec ssh_tmp
        pla
        cmp #','
        bne -
        jmp _name
_yes    clc
        rts
_no     sec
        rts
        .pend

; --- exchange hash -------------------------------------------------------------------

; SHA-256 of an SSH string: length ssh_sl, bytes at (ssh_sp)
hash_str .proc
        lda #0
        jsr sha256_byte
        lda #0
        jsr sha256_byte
        lda ssh_sl+1
        jsr sha256_byte
        lda ssh_sl
        jsr sha256_byte
        lda ssh_sp
        sta sha_p
        lda ssh_sp+1
        sta sha_p+1
        lda ssh_sl
        sta sha_len
        lda ssh_sl+1
        sta sha_len+1
        jmp sha256_update
        .pend

; hash_str of a string of ours (length byte, text) at X/Y
hash_pstr .proc
        stx ssh_sp
        sty ssh_sp+1
        ldy #0
        lda (ssh_sp),y
        sta ssh_sl
        sty ssh_sl+1
        inc ssh_sp
        bne +
        inc ssh_sp+1
+       jmp hash_str
        .pend

; hash_str of 32 bytes at X/Y
hash_32 .proc
        stx ssh_sp
        sty ssh_sp+1
        lda #32
        sta ssh_sl
        lda #0
        sta ssh_sl+1
        jmp hash_str
        .pend

; --- waiting ----------------------------------------------------------------------------

; the next packet, apart from those any moment may bring (ignore, debug,
; global requests: refused); carry set if the connection is over
ssh_wait .proc
_again  jsr ssh_net
        lda tcp_state
        cmp #TCP_OPEN
        bne _gone
        jsr ssh_recv
        bcc +
        jsr ssh_eof
        bcc _again
        jmp _gone
+       lda ssh_type
        cmp #MSG_IGNORE
        beq _again
        cmp #MSG_DEBUG
        beq _again
        cmp #MSG_UNIMPL
        beq _again
        cmp #MSG_DISCONNECT
        beq _disc
        cmp #MSG_GLOBAL_REQUEST
        bne _ok
        jsr get_str             ; name
        jsr get_u8              ; want reply?
        beq _again
        lda #MSG_REQUEST_FAILURE
        jsr ssh_begin
        jsr ssh_send
        jmp _again
_ok     clc
        rts
_disc   jsr get_u32             ; reason code, then the text
        jsr get_str
        jsr ui_nl
        jsr ui_msg
        lda #<_msg_disc
        ldx #>_msg_disc
        jmp ssh_fail
_gone   lda ssh_err+1
        bne +
        lda #<_msg_lost
        ldx #>_msg_lost
        jmp ssh_fail
+       sec
        rts
_msg_disc .null "disconnected by the server"
_msg_lost .null "connection lost"
        .pend

; waits for a packet of type A; carry set on anything else
ssh_expect .proc
        sta ssh_want
        jsr ssh_wait
        bcs _r
        lda ssh_type
        cmp ssh_want
        bne _bad
        clc
_r      rts
_bad    lda #<_msg
        ldx #>_msg
        jmp ssh_fail
_msg    .null "unexpected message"
        .pend

; --- the key exchange ----------------------------------------------------------------

V_C     .byte vc_len
vc_text .text "SSH-2.0-ssh64_0.1"
vc_len  = * - vc_text

ssh_kexinit .proc
        lda #MSG_KEXINIT
        jsr ssh_begin
        jsr rng_secret          ; cookie
        ldx #0
-       lda sha256_out,x
        jsr put_u8
        inx
        cpx #16
        bne -
        jsr put_istr
        .byte _kex_n
_kex    .text "curve25519-sha256,curve25519-sha256@libssh.org,"
        .text "kex-strict-c-v00@openssh.com"
_kex_n  = * - _kex
        jsr put_istr
        .byte 11
        .text "ssh-ed25519"
        jsr _ciphers
        jsr _ciphers
        jsr _macs
        jsr _macs
        jsr put_istr
        .byte 4
        .text "none"
        jsr put_istr
        .byte 4
        .text "none"
        lda #0
        tax
        jsr put_u16             ; languages: two empty lists
        lda #0
        tax
        jsr put_u16
        lda #0
        jsr put_u8              ; no guessed packet
        lda #0
        tax
        jmp put_u16             ; reserved
_ciphers jsr put_istr
        .byte _c_n
_c      .text "aes128-gcm@openssh.com,aes256-gcm@openssh.com"
_c_n    = * - _c
        rts
_macs   jsr put_istr
        .byte _m_n
_m      .text "hmac-sha2-256"   ; (not used with AES-GCM; short: it is hashed)
_m_n    = * - _m
        rts
        .pend

; the server's KEXINIT (in ssh_ibuf): algorithms agreed? carry set if not
ssh_negotiate .proc
        lda ssh_rp              ; past the cookie
        clc
        adc #16
        sta ssh_rp
        bcc +
        inc ssh_rp+1
+       jsr get_str             ; kex
        ldx #<_c25519
        ldy #>_c25519
        jsr list_has
        bcc +
        ldx #<_c25519b
        ldy #>_c25519b
        jsr list_has
        bcs _nokex
+       lda #0
        sta ssh_strict
        ldx #<_strict
        ldy #>_strict
        jsr list_has
        bcs +
        inc ssh_strict
+       jsr get_str             ; host key
        ldx #<_ed
        ldy #>_ed
        jsr list_has
        bcs _nohk
        jsr get_str             ; ciphers, both ways: aes128-gcm else aes256
        jsr _cipher
        bcs _noc
        sta ssh_klen
        jsr get_str
        jsr _cipher
        bcs _noc
        cmp ssh_klen
        bne _noc
        jsr get_str             ; MACs: not used with GCM
        jsr get_str
        jsr get_str             ; compression
        ldx #<_none
        ldy #>_none
        jsr list_has
        bcs _nocomp
        jsr get_str
        ldx #<_none
        ldy #>_none
        jsr list_has
        bcs _nocomp
        clc
        rts
_cipher ldx #<_g128
        ldy #>_g128
        jsr list_has
        lda #16
        bcc +
        ldx #<_g256
        ldy #>_g256
        jsr list_has
        lda #32
+       rts
_nokex  lda #<_m_kex
        ldx #>_m_kex
        jmp ssh_fail
_nohk   lda #<_m_hk
        ldx #>_m_hk
        jmp ssh_fail
_noc    lda #<_m_c
        ldx #>_m_c
        jmp ssh_fail
_nocomp lda #<_m_comp
        ldx #>_m_comp
        jmp ssh_fail
_c25519 .byte 17
        .text "curve25519-sha256"
_c25519b .byte 28
        .text "curve25519-sha256@libssh.org"
_strict .byte 28
        .text "kex-strict-s-v00@openssh.com"
_ed     .byte 11
        .text "ssh-ed25519"
_g128   .byte 22
        .text "aes128-gcm@openssh.com"
_g256   .byte 22
        .text "aes256-gcm@openssh.com"
_none   .byte 4
        .text "none"
_m_kex  .null "server has no curve25519-sha256"
_m_hk   .null "server has no ssh-ed25519 host key"
_m_c    .null "server has no aes-gcm cipher"
_m_comp .null "server wants compression"
        .pend

        .section bss
ssh_kh  .fill 100               ; SHA-256 state after K || H (see below)
        .send bss

; the part the keys have in common, K (mpint in ssh_kmp) || H: its hash
; state kept in ssh_kh (K || H is longer than a block)
ssh_derive_kh .proc
        jsr sha256_init
        lda ssh_kmp
        sta ssh_sl
        lda #0
        sta ssh_sl+1
        lda #<(ssh_kmp+1)
        sta ssh_sp
        lda #>(ssh_kmp+1)
        sta ssh_sp+1
        jsr hash_str
        lda #<ssh_h
        sta sha_p
        lda #>ssh_h
        sta sha_p+1
        lda #32
        sta sha_len
        lda #0
        sta sha_len+1
        jsr sha256_update
        ldx #99
-       lda sha_h,x
        sta ssh_kh,x
        dex
        bpl -
        rts
        .pend

; key K || H || letter A || session id: SHA-256
ssh_derive .proc
        sta ssh_tmp+3
        ldx #99
-       lda ssh_kh,x
        sta sha_h,x
        dex
        bpl -
        lda ssh_tmp+3
        jsr sha256_byte
        lda #<ssh_sid
        sta sha_p
        lda #>ssh_sid
        sta sha_p+1
        lda #32
        sta sha_len
        lda #0
        sta sha_len+1
        jsr sha256_update
        jmp sha256_final
        .pend

; GCM keys of direction X from letters: IV A, key A + 2
ssh_keys .proc
        stx ssh_tmp+2
        sta ssh_tmp+1
        jsr ssh_derive          ; IV
        ldx #11
-       lda sha256_out,x
        sta gcm_iv_in,x
        dex
        bpl -
        lda ssh_tmp+1
        clc
        adc #2
        jsr ssh_derive          ; key
        ldx #31
-       lda sha256_out,x
        sta aes_key,x
        dex
        bpl -
        lda ssh_klen
        sta aes_klen
        ldx ssh_tmp+2
        jmp gcm_setup
        .pend

; the whole transport set-up up to the encrypted channel; carry set on
; failure (ssh_err)
ssh_handshake .proc
        lda #0
        sta ssh_renc
        sta ssh_senc
        sta ssh_rcnt
        sta ssh_rcnt+1
        sta ssh_rneed
        sta ssh_rneed+1
        sta ssh_err
        sta ssh_err+1
        lda #$ff
        sta gcm_dir
        ; our version
        lda #<vc_text
        sta net_q
        lda #>vc_text
        sta net_q+1
        lda #vc_len
        jsr tcp_write
        lda #<_crlf
        sta net_q
        lda #>_crlf
        sta net_q+1
        lda #2
        jsr tcp_write
        ; the server's: lines until one starts with SSH-
_line   lda #0
        sta ssh_vs
-       jsr ssh_net
        lda tcp_state
        cmp #TCP_OPEN
        beq +
        lda #<_m_nover
        ldx #>_m_nover
        jmp ssh_fail
+       jsr tcp_read
        bcs -
        cmp #10
        beq _eol
        cmp #13
        beq -
        ldx ssh_vs
        cpx #250
        bcs -
        sta ssh_vs+1,x
        inc ssh_vs
        jmp -
_eol    lda ssh_vs+1
        cmp #'S'
        bne _line
        lda ssh_vs+2
        cmp #'S'
        bne _line
        lda ssh_vs+3
        cmp #'H'
        bne _line
        lda ssh_vs+4
        cmp #'-'
        bne _line
        lda ssh_vs+5            ; SSH-2.0 or SSH-1.99
        cmp #'2'
        beq +
        cmp #'1'
        bne _badver
        lda ssh_vs+7
        cmp #'9'
        beq +
_badver lda #<_m_ver
        ldx #>_m_ver
        jmp ssh_fail
+       jsr ui_status
        .null "server: "
        lda #<(ssh_vs+1)
        sta ssh_sp
        lda #>(ssh_vs+1)
        sta ssh_sp+1
        lda ssh_vs
        sta ssh_sl
        lda #0
        sta ssh_sl+1
        jsr ui_msg
        ; KEXINIT: ours, then the hash from V_C, V_S, I_C
        jsr ssh_kexinit
        jsr sha256_init
        ldx #<V_C
        ldy #>V_C
        jsr hash_pstr
        ldx #<ssh_vs
        ldy #>ssh_vs
        jsr hash_pstr
        lda ssh_wp              ; I_C: the payload built so far
        sec
        sbc #<(ssh_obuf+5)
        sta ssh_sl
        lda ssh_wp+1
        sbc #>(ssh_obuf+5)
        sta ssh_sl+1
        lda #<(ssh_obuf+5)
        sta ssh_sp
        lda #>(ssh_obuf+5)
        sta ssh_sp+1
        jsr hash_str
        jsr ssh_send
        ; theirs (strict: it must be the first packet)
        lda #MSG_KEXINIT
        jsr ssh_expect
        bcc +
        rts
+       lda ssh_plen            ; I_S
        sta ssh_sl
        lda ssh_plen+1
        sta ssh_sl+1
        lda #<(ssh_ibuf+5)
        sta ssh_sp
        lda #>(ssh_ibuf+5)
        sta ssh_sp+1
        jsr hash_str
        jsr ssh_negotiate
        bcc +
        rts
+       ; ECDH
        lda #MSG_ECDH_INIT
        jsr ssh_begin
        lda #<ssh_qc
        sta ssh_sp
        lda #>ssh_qc
        sta ssh_sp+1
        lda #32
        sta ssh_sl
        lda #0
        sta ssh_sl+1
        jsr put_str
        jsr ssh_send
        lda #MSG_ECDH_REPLY
        jsr ssh_expect
        bcc +
        rts
+       jsr get_str             ; K_S: hashed, and its key taken
        lda ssh_sp
        sta ssh_ks
        lda ssh_sp+1
        sta ssh_ks+1
        lda ssh_sl
        sta ssh_ks+2
        lda ssh_sl+1
        sta ssh_ks+3
        jsr hash_str
        lda ssh_rp              ; (inside it)
        pha
        lda ssh_rp+1
        pha
        lda ssh_sp
        sta ssh_rp
        lda ssh_sp+1
        sta ssh_rp+1
        jsr get_str             ; "ssh-ed25519"
        jsr get_str             ; the key
        lda ssh_sl
        cmp #32
        bne _badkey
        ldy #31
-       lda (ssh_sp),y
        sta ssh_hkey,y
        dey
        bpl -
        pla
        sta ssh_rp+1
        pla
        sta ssh_rp
        ldx #<ssh_qc            ; Q_C
        ldy #>ssh_qc
        jsr hash_32
        jsr get_str             ; Q_S
        lda ssh_sl
        cmp #32
        bne _badkey2
        ldy #31
-       lda (ssh_sp),y
        sta ssh_qs,y
        dey
        bpl -
        ldx #<ssh_qs
        ldy #>ssh_qs
        jsr hash_32
        jsr get_str             ; signature blob: "ssh-ed25519", 64 bytes
        lda ssh_sp
        sta ssh_rp
        lda ssh_sp+1
        sta ssh_rp+1
        jsr get_str
        jsr get_str
        lda ssh_sl
        cmp #64
        bne _badkey2
        ldy #63
-       lda (ssh_sp),y
        sta ssh_hsig,y
        dey
        bpl -
        ; the host key: known? A new one is only shown and saved: its comb
        ; is made after this connection, and the next one is checked (the
        ; exchange hash is still being computed: no other SHA-256 now)
        jsr kh_check
        lda kh_state
        bne +
        jmp ssh_hostkey
+       cmp #2
        bne +
        lda #<_m_changed
        ldx #>_m_changed
        jmp ssh_fail
+       ; K = X25519(ours, theirs)
        jsr ui_status
        .null "computing the shared key"
        lda #2                  ; red
        jsr sys_crunch
        ldx #31
-       lda ssh_priv,x
        sta x25519_k,x
        lda ssh_qs,x
        sta x25519_u,x
        dex
        bpl -
        jsr x25519
        ; as mpint: leading zeros off, a zero in front of a high bit
        ldx #0
-       lda x25519_out,x
        bne +
        inx
        cpx #32
        bne -
        lda #<_m_zero
        ldx #>_m_zero
        jmp ssh_fail
+       ldy #1
        lda x25519_out,x
        bpl +
        lda #0
        sta ssh_kmp,y
        iny
+
-       lda x25519_out,x
        sta ssh_kmp,y
        iny
        inx
        cpx #32
        bne -
        dey
        sty ssh_kmp
        lda #<(ssh_kmp+1)
        sta ssh_sp
        lda #>(ssh_kmp+1)
        sta ssh_sp+1
        lda ssh_kmp
        sta ssh_sl
        lda #0
        sta ssh_sl+1
        jsr hash_str
        jsr sha256_final
        ldx #31
-       lda sha256_out,x
        sta ssh_h,x
        sta ssh_sid,x           ; (first exchange: also the session id)
        dex
        bpl -
        ; the signature of H, through the comb of the key (kh_prepare)
        jsr ui_status
        .null "checking the server's signature"
        ldx #31
-       lda ssh_hkey,x
        sta ed_pub,x
        dex
        bpl -
        ldx #63
-       lda ssh_hsig,x
        sta ed_sig,x
        dex
        bpl -
        lda #<ssh_h
        sta ed_msg
        lda #>ssh_h
        sta ed_msg+1
        lda #32
        sta ed_msglen
        lda #0
        sta ed_msglen+1
        jsr ed25519_verify_comb
        php
        jsr sys_awake
        plp
        bcc +
        lda #<_m_sig
        ldx #>_m_sig
        jmp ssh_fail
+       ; new keys
        lda #MSG_NEWKEYS
        jsr ssh_begin
        jsr ssh_send
        jsr ssh_derive_kh
        lda #'A'
        ldx #0
        jsr ssh_keys
        lda #1
        sta ssh_senc
        ; the authentication service; the login follows at once (the
        ; server's time limit ends with it), then the keys of the other
        ; direction (ssh_kexend); its answer after the service's
        lda #MSG_SERVICE_REQUEST
        jsr ssh_begin
        jsr put_istr
        .byte 12
        .text "ssh-userauth"
        jsr ssh_send
        clc
        rts
_badkey pla
        pla
_badkey2 lda #<_m_key
        ldx #>_m_key
        jmp ssh_fail
_crlf   .text 13, 10
_m_nover .null "no answer from the server"
_m_ver  .null "not an SSH-2 server"
_m_key  .null "bad key from the server"
_m_zero .null "bad shared key"
_m_sig  .null "the server's signature is WRONG"
_m_changed .null "THE HOST KEY HAS CHANGED: someone may be in between (remove the host from SSH64 HOSTS if the change is expected)"
        .pend

; --- authentication ------------------------------------------------------------------

; USERAUTH_REQUEST header for method X/Y (our string)
auth_head .proc
        stx ssh_t2
        sty ssh_t2+1
        lda #MSG_USERAUTH_REQUEST
        jsr ssh_begin
        ldx #<ssh_user
        ldy #>ssh_user
        jsr put_pstr
        jsr put_istr
        .byte 14
        .text "ssh-connection"
        ldx ssh_t2
        ldy ssh_t2+1
        jmp put_pstr
        .pend

; the end of the key exchange: the server's NEWKEYS, the keys it sends with
ssh_kexend .proc
        lda #MSG_NEWKEYS
        jsr ssh_expect
        bcc +
        rts
+       lda #'B'
        ldx #1
        jsr ssh_keys
        lda #1
        sta ssh_renc
        clc
        rts
        .pend

; password, then keyboard-interactive if offered; carry set if refused
ssh_auth .proc
        lda #0
        sta ssh_kbd_tried
        ldx #<_pw
        ldy #>_pw
        jsr auth_head
        lda #0
        jsr put_u8
        ldx #<ssh_pass
        ldy #>ssh_pass
        jsr put_pstr
        jsr ssh_send
        jsr ssh_kexend
        bcc _wait
        rts
_wait   jsr ssh_wait
        bcc +
        rts
+       lda ssh_type
        cmp #MSG_USERAUTH_SUCCESS
        bne +
        clc
        rts
+       cmp #MSG_USERAUTH_BANNER
        bne +
        jsr get_str
        jsr ui_msg
        jmp _wait
+       cmp #MSG_USERAUTH_INFO_REQUEST
        beq _info
        cmp #MSG_USERAUTH_FAILURE
        beq +
        jmp _wait
+       jsr get_str             ; methods that can go on
        lda ssh_kbd_tried
        bne _denied
        inc ssh_kbd_tried
        ldx #<_kbd
        ldy #>_kbd
        jsr list_has
        bcs _denied
        ldx #<_kbd
        ldy #>_kbd
        jsr auth_head
        lda #0                  ; language, submethods
        tax
        jsr put_u16
        lda #0
        tax
        jsr put_u16
        jsr ssh_send
        jmp _wait
_info   jsr get_str             ; name
        jsr get_str             ; instruction
        jsr get_str             ; language
        jsr get_u32             ; prompts
        lda ssh_tmp+3
        sta ssh_t2
        lda #MSG_USERAUTH_INFO_RESPONSE
        jsr ssh_begin
        lda #0
        ldx ssh_t2
        jsr put_u16
-       lda ssh_t2              ; the password for every prompt
        beq +
        ldx #<ssh_pass
        ldy #>ssh_pass
        jsr put_pstr
        dec ssh_t2
        jmp -
+       jsr ssh_send
        jmp _wait
_denied lda #<_m_denied
        ldx #>_m_denied
        jmp ssh_fail
_pw     .byte 8
        .text "password"
_kbd    .byte 20
        .text "keyboard-interactive"
_m_denied .null "access denied"
        .pend

; --- the session channel --------------------------------------------------------------

ssh_channel .proc
        lda #MSG_CHANNEL_OPEN
        jsr ssh_begin
        jsr put_istr
        .byte 7
        .text "session"
        lda #0                  ; our number 0
        tax
        jsr put_u16
        lda #>SSH_WINDOW        ; window
        ldx #<SSH_WINDOW
        jsr put_u16
        lda #>1024              ; largest packet
        ldx #<1024
        jsr put_u16
        jsr ssh_send
        lda #<SSH_WINDOW
        sta ch_lwin
        lda #>SSH_WINDOW
        sta ch_lwin+1
-       jsr ssh_wait
        bcc +
        rts
+       lda ssh_type
        cmp #MSG_CHANNEL_OPEN_CONFIRMATION
        beq +
        cmp #MSG_CHANNEL_OPEN_FAILURE
        bne -
        lda #<_m_open
        ldx #>_m_open
        jmp ssh_fail
+       jsr get_u32             ; our number
        jsr get_u32             ; theirs
        ldx #3
-       lda ssh_tmp,x
        sta ch_remote,x
        dex
        bpl -
        jsr get_u32             ; their window
        ldx #3
-       lda ssh_tmp,x
        sta ch_rwin,x
        dex
        bpl -
        ; a pty of 40 x 25, then the shell
        jsr _req
        .byte 7
        .text "pty-req"
        lda #0
        jsr put_u8
        jsr put_istr
        .byte 5
        .text "vt100"
        lda #0
        ldx #T_COLS
        jsr put_u16
        lda #0
        ldx #T_ROWS
        jsr put_u16
        lda #0
        tax
        jsr put_u16
        lda #0
        tax
        jsr put_u16
        jsr put_istr            ; modes: TTY_OP_END only
        .byte 1
        .byte 0
        jsr ssh_send
        jsr _req
        .byte 5
        .text "shell"
        lda #1                  ; want reply
        jsr put_u8
        jsr ssh_send
        lda #1
        sta ssh_open
        clc
        rts
_req    ; CHANNEL_REQUEST to their channel, the name after the jsr
        lda #MSG_CHANNEL_REQUEST
        jsr ssh_begin
        ldx #<ch_remote
        ldy #>ch_remote
        jsr _raw4
        jmp put_istr            ; (the jsr's return address is still ours)
_raw4   stx sys_t
        sty sys_t+1
        ldy #0
-       lda (sys_t),y
        jsr put_u8
        iny
        cpy #4
        bne -
        rts
_m_open .null "the server refused the session"
        .pend

; --- the session --------------------------------------------------------------------

; CHANNEL_DATA with ssh_sl bytes at (ssh_sp)
ssh_data .proc
        lda #MSG_CHANNEL_DATA
        jsr ssh_begin
        ldx #<ch_remote
        ldy #>ch_remote
        jsr ssh_channel._raw4
        jsr put_str
        jmp ssh_send
        .pend

; runs the terminal until the channel or the connection closes
ssh_session .proc
        lda #0
        sta ssh_keylen
        jsr term_cursor_on
_loop   jsr ssh_net
        lda tcp_state
        cmp #TCP_OPEN
        beq +
        jmp _end
+       jsr ssh_recv
        bcc +
        jsr ssh_eof             ; the server closed, all read: over
        bcc _keys
        jmp _end
+
        jsr term_cursor_off
        jsr _packet
        jsr term_cursor_on
        lda ssh_open
        bne _keys
        jmp _end
_keys   jsr kbd_get             ; keys typed: collected, then sent
        bcs _send
        pha
        jsr rng_stir
        pla
        jsr _key
        lda ssh_keylen
        cmp #24
        bcc _keys
_send   lda ssh_keylen
        beq _reply
        lda #<ssh_keybuf
        sta ssh_sp
        lda #>ssh_keybuf
        sta ssh_sp+1
        lda ssh_keylen
        sta ssh_sl
        lda #0
        sta ssh_sl+1
        sta ssh_keylen
        jsr ssh_data
_reply  lda term_rlen           ; answers of the terminal
        beq _loop
        lda #<term_reply
        sta ssh_sp
        lda #>term_reply
        sta ssh_sp+1
        lda term_rlen
        sta ssh_sl
        lda #0
        sta ssh_sl+1
        sta term_rlen
        jsr ssh_data
        jmp _loop
_end    jsr term_cursor_off
        rts

_packet lda ssh_type
        cmp #MSG_CHANNEL_DATA
        beq _data
        cmp #MSG_CHANNEL_EXTENDED_DATA
        bne +
        jsr get_u32
        jmp _data
+       cmp #MSG_CHANNEL_WINDOW_ADJUST
        beq _adjust
        cmp #MSG_CHANNEL_CLOSE
        beq _close
        cmp #MSG_CHANNEL_REQUEST
        beq _chreq
        cmp #MSG_GLOBAL_REQUEST
        beq _global
        cmp #MSG_DISCONNECT
        beq _disc
        rts
_data   jsr get_u32             ; (our channel)
        jsr get_str
        lda ch_lwin             ; our window shrinks
        sec
        sbc ssh_sl
        sta ch_lwin
        lda ch_lwin+1
        sbc ssh_sl+1
        sta ch_lwin+1
_dloop  lda ssh_sl
        ora ssh_sl+1
        beq _topup
        ldy #0
        lda (ssh_sp),y
        jsr term_out
        inc ssh_sp
        bne +
        inc ssh_sp+1
+       lda ssh_sl
        bne +
        dec ssh_sl+1
+       dec ssh_sl
        jmp _dloop
_topup  lda ch_lwin+1           ; below half: give it back
        cmp #>(SSH_WINDOW/2)
        bcs _r
        lda #MSG_CHANNEL_WINDOW_ADJUST
        jsr ssh_begin
        ldx #<ch_remote
        ldy #>ch_remote
        jsr ssh_channel._raw4
        lda #<SSH_WINDOW        ; what was used
        sec
        sbc ch_lwin
        tax
        lda #>SSH_WINDOW
        sbc ch_lwin+1
        jsr put_u16
        lda #<SSH_WINDOW
        sta ch_lwin
        lda #>SSH_WINDOW
        sta ch_lwin+1
        jmp ssh_send
_r      rts
_adjust jsr get_u32             ; (our channel)
        jsr get_u32
        clc
        ldx #3
-       lda ch_rwin,x
        adc ssh_tmp,x
        sta ch_rwin,x
        dex
        bpl -
        rts
_close  lda #MSG_CHANNEL_CLOSE
        jsr ssh_begin
        ldx #<ch_remote
        ldy #>ch_remote
        jsr ssh_channel._raw4
        jsr ssh_send
        lda #0
        sta ssh_open
        rts
_chreq  jsr get_u32             ; exit-status and such: refused if asked
        jsr get_str
        jsr get_u8
        beq _r
        lda #MSG_CHANNEL_FAILURE
        jsr ssh_begin
        ldx #<ch_remote
        ldy #>ch_remote
        jsr ssh_channel._raw4
        jmp ssh_send
_global jsr get_str
        jsr get_u8
        beq _r
        lda #MSG_REQUEST_FAILURE
        jsr ssh_begin
        jmp ssh_send
_disc   lda #0
        sta ssh_open
        rts

; a key into ssh_keybuf as the bytes a VT100 sends
_key    cmp #$80
        bcs _special
_byte   ldx ssh_keylen
        sta ssh_keybuf,x
        inc ssh_keylen
        rts
_special cmp #K_MENU
        bne +
        lda #TCP_CLOSED         ; C= + RUN/STOP: hang up
        sta tcp_state
        lda #<_m_bye
        sta ssh_err
        lda #>_m_bye
        sta ssh_err+1
        rts
+       cmp #K_F1
        bcs _fkey
        cmp #K_INS
        beq _ins
        cmp #K_CLR
        bne +
        lda #12                 ; CLR: ^L
        bne _byte
+       and #7                  ; arrows, home: ESC [ x or ESC O x
        tax
        lda _arrows,x
        pha
        lda #27
        jsr _byte
        lda #'['
        ldx t_appkeys
        beq +
        lda #'O'
+       jsr _byte
        pla
        jmp _byte
_ins    lda #27
        jsr _byte
        lda #'['
        jsr _byte
        lda #'2'
        jsr _byte
        lda #'~'
        jmp _byte
_fkey   sec                     ; F1-F4: ESC O P..S (F5-F8 the same)
        sbc #K_F1
        and #3
        clc
        adc #'P'
        pha
        lda #27
        jsr _byte
        lda #'O'
        jsr _byte
        pla
        jmp _byte
_arrows .text "ABDCH"
_m_bye  .null "closed"
        .pend

; the SHA-256 fingerprint of the host key
ssh_fingerprint .proc
        jsr sha256_init
        lda ssh_ks
        sta sha_p
        lda ssh_ks+1
        sta sha_p+1
        lda ssh_ks+2
        sta sha_len
        lda ssh_ks+3
        sta sha_len+1
        jsr sha256_update
        jsr sha256_final
        ldx #31
-       lda sha256_out,x
        sta ssh_fp,x
        dex
        bpl -
        rts
        .pend

; a new host key: shown, and if the user accepts it saved (kh_new set);
; carry set in any case, the connection ends here
ssh_hostkey .proc
        jsr ssh_fingerprint
        jsr ui_status
        .null "host key ED25519 SHA256:"
        jsr ui_nl
        jsr b64_fp
        jsr ui_status
        .null "accept it (y/n)? "
        jsr term_cursor_on
-       jsr ssh_net
        jsr kbd_get
        bcs -
        pha
        jsr term_cursor_off
        pla
        cmp #'y'
        beq +
        cmp #'Y'
        beq +
        lda #<_m_no
        ldx #>_m_no
        jmp ssh_fail
+       jsr putc
        jsr kh_add
        bcc +
        lda #<_m_full
        ldx #>_m_full
        jmp ssh_fail
+       lda #1
        sta kh_new
        lda #<_m_saved
        ldx #>_m_saved
        jmp ssh_fail
_m_no   .null "host key not accepted"
_m_full .null "no room for another known host"
_m_saved .null "host key saved"
        .pend

; ssh_fp in base64, without padding (43 characters)
b64_fp  .proc
        ldx #0
_group  lda ssh_fp,x            ; 3 bytes -> 4 characters
        sta ssh_tmp
        lda ssh_fp+1,x
        sta ssh_tmp+1
        lda ssh_fp+2,x
        sta ssh_tmp+2
        stx ssh_t2
        lda ssh_tmp
        lsr a
        lsr a
        jsr _out
        lda ssh_tmp
        and #3
        asl a
        asl a
        asl a
        asl a
        sta ssh_t2+1
        lda ssh_tmp+1
        lsr a
        lsr a
        lsr a
        lsr a
        ora ssh_t2+1
        jsr _out
        ldx ssh_t2
        cpx #30                 ; the last group has 2 bytes
        beq _last
        lda ssh_tmp+1
        and #15
        asl a
        asl a
        sta ssh_t2+1
        lda ssh_tmp+2
        lsr a
        lsr a
        lsr a
        lsr a
        lsr a
        lsr a
        ora ssh_t2+1
        jsr _out
        lda ssh_tmp+2
        and #63
        jsr _out
        ldx ssh_t2
        inx
        inx
        inx
        jmp _group
_last   lda ssh_tmp+1
        and #15
        asl a
        asl a
_out    tax
        lda _chars,x
        jmp putc
_chars  .text "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/"
        .pend
