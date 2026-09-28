; SPDX-License-Identifier: GPL-2.0-or-later
; Copyright (C) 2026 SampoSoft - Francesco Sampoli
;
; XMODEM download to drive 8 (CTRL + D or F3 in the session).
;
; The receiver starts the transfer: 'C' asks for CRC-16 blocks, NAK (after
; XM_CTRIES unanswered 'C's: a BBS may still be getting the file ready) for
; the old checksum. Blocks of 128 (SOH) and
; 1024 bytes (STX) are taken. A good block is acknowledged at once and
; written to the disk while the BBS sends the next one; it is written one
; block late, so that at EOT the padding of the last one (SUB, $1a) can be
; cut off: a PRG keeps its length. A transfer that fails sends CAN to the
; BBS and scratches the partial file.

XM_SOH  = $01
XM_STX  = $02
XM_EOT  = $04
XM_ACK  = $06
XM_NAK  = $15
XM_CAN  = $18
XM_SUB  = $1a
XM_C    = $43               ; 'C' in ASCII: CRC blocks, please
XM_CTRIES = 6                   ; 'C's, 3 s apart, before the checksum
XM_TRIES = 12                   ; and in all, before giving up

xm_p    = $fb           ; buffer pointers (the zero page left to programs)
xm_q    = $fd

SETLFS  = $ffba
SETNAM  = $ffbd
OPEN    = $ffc0
CLOSE   = $ffc3
CHKIN   = $ffc6
CHKOUT  = $ffc9
CLRCHN  = $ffcc
CHRIN   = $ffcf

        .section bss
xm_fn   .fill 24                ; file name as typed, then ",p,w"
xm_fnl  .fill 1                 ; its length as typed
xm_from .fill 1                 ; where the name starts, past "@0:"
xm_cmd  .fill 32                ; "s0:" name, for the scratch
xm_st   .fill 40                ; drive status line
xm_buf  .fill 1024              ; block being received
xm_pend .fill 1024              ; last good block, not yet written
xm_plen .fill 2                 ; its length (0: none)
xm_len  .fill 2                 ; length of the block being received
xm_blk  .fill 1                 ; block number expected
xm_hdr  .fill 2                 ; number and complement received
xm_crc  .fill 2                 ; CRC-16 (low, high) or checksum (low)
xm_mode .fill 1                 ; 1: CRC, 0: checksum
xm_errs .fill 1                 ; errors in a row
xm_sent .fill 1                 ; something was sent to the BBS
xm_to   .fill 2                 ; timeout of xm_get, ticks
xm_dl   .fill 2                 ; its deadline
xm_sp   .fill 1                 ; stack of xm_receive, for the failures
xm_res  .fill 1                 ; result: an XR_ code
xm_bytes .fill 3                ; bytes saved
xm_row  .fill 1                 ; screen row of the counter
xm_dig  .fill 8
        .send bss

; results
XR_OK   = 0
XR_STOP = 1                     ; RUN/STOP
XR_CAN  = 2                     ; the BBS cancelled
XR_SILENT = 3                   ; no answer
XR_ERRS = 4                     ; too many errors
XR_SEQ  = 5                     ; a block out of sequence
XR_LOST = 6                     ; connection lost

xm_download .proc
        jsr bottom
        jsr puts
        .enc "petscii"
        .null "{off}{white}XMODEM download as: "
        .enc "none"
        jsr xm_name
        bcc +
        jsr puts
        .enc "petscii"
        .null 13, "Cancelled.", 13
        .enc "none"
        rts
+       jsr xm_open
        bcc +
        rts
+       jsr puts
        .enc "petscii"
        .null 13, "RUN/STOP stops it. Received: "
        .enc "none"
        sec                     ; the counter's place
        jsr PLOT
        stx xm_row
        jsr xm_receive
        jsr xm_show
        lda xm_res
        cmp #XR_LOST            ; a transfer that did not start: nothing
        beq +                   ; to cancel on a connection that is gone
        cmp #XR_OK
        beq +
        lda xm_sent
        beq +
        jsr xm_cancel
+       lda #2                  ; the file
        jsr CLOSE
        lda xm_res
        beq _ok
        jsr xm_scratch
        lda #15
        jsr CLOSE
        jsr puts
        .enc "petscii"
        .null " bytes", 13, "{red}"
        .enc "none"
        ldx xm_res
        lda _msg_lo-1,x
        sta str_p
        lda _msg_hi-1,x
        sta str_p+1
        jsr _pstr
        jsr puts
        .enc "petscii"
        .null " File scratched.", 13, "{white}"
        .enc "none"
        rts
_ok     jsr xm_status           ; the drive's last word: disk full...
        lda #15
        jsr CLOSE
        jsr puts
        .enc "petscii"
        .null " bytes", 13
        .enc "none"
        lda xm_st
        cmp #'0'
        bne _derr
        lda xm_st+1
        cmp #'0'
        bne _derr
        jsr puts
        .enc "petscii"
        .null "{yellow}Saved.{white}", 13
        .enc "none"
        rts
_derr   jsr puts
        .enc "petscii"
        .null "{red}Drive: "
        .enc "none"
        jsr xm_pst
        lda #13
        jmp CHROUT

_pstr   ldy #0                  ; zero-terminated string at str_p
-       lda (str_p),y
        beq +
        jsr CHROUT
        iny
        bne -
+       rts

        .enc "petscii"
_m_stop .null "Stopped."
_m_can  .null "The BBS cancelled."
_m_sil  .null "No answer from the BBS."
_m_errs .null "Too many errors."
_m_seq  .null "Blocks out of sequence."
_m_lost .null "Connection lost."
        .enc "none"
_msg_lo .byte <_m_stop, <_m_can, <_m_sil, <_m_errs, <_m_seq, <_m_lost
_msg_hi .byte >_m_stop, >_m_can, >_m_sil, >_m_errs, >_m_seq, >_m_lost
        .pend

; the file name from the keyboard, up to 16 characters: RETURN ends it,
; RUN/STOP or an empty name cancels (carry set)
xm_name .proc
        lda #0
        sta xm_fnl
_key    jsr cursor_on
        jsr net_poll            ; the connection stays alive meanwhile
        jsr tcp_poll
        jsr GETIN
        cmp #0
        beq _key
        pha
        jsr cursor_off
        pla
        cmp #13
        beq _end
        cmp #3
        beq _stop
        cmp #$14                ; DEL
        bne +
        ldx xm_fnl
        beq _key
        dec xm_fnl
        jsr CHROUT
        jmp _key
+       ldx xm_fnl
        cpx #16
        bcs _key
        cmp #$20                ; space .. ], and the shifted letters
        bcc _key
        cmp #$60
        bcc _take
        cmp #$c1
        bcc _key
        cmp #$db
        bcs _key
_take   sta xm_fn,x
        inc xm_fnl
        ldx #0
        stx QTSW
        jsr CHROUT
        jmp _key
_end    lda xm_fnl
        beq _stop
        clc
        rts
_stop   sec
        rts
        .pend

; opens the command channel and the file ("name,p,w", or "name,w" if the
; name has its own type); carry set, with the reason printed, if it fails
xm_open .proc
        ldx xm_fnl              ; the suffix
        ldy #0
-       lda xm_fn,y
        cmp #','
        beq _typed
        iny
        cpy xm_fnl
        bne -
        lda #','
        sta xm_fn,x
        inx
        lda #'p'
        sta xm_fn,x
        inx
_typed  lda #','
        sta xm_fn,x
        inx
        lda #'w'
        sta xm_fn,x
        inx
        txa
        ldx #<xm_fn
        ldy #>xm_fn
        jsr SETNAM
        lda #2
        ldx #8
        ldy #2
        jsr SETLFS
        jsr OPEN
        bcs _kerr
        lda #0                  ; the command channel, to read the status
        jsr SETNAM
        lda #15
        ldx #8
        ldy #15
        jsr SETLFS
        jsr OPEN
        bcs _kerr
        jsr xm_status
        bcs _none
        lda xm_st
        cmp #'0'
        bne _derr
        lda xm_st+1
        cmp #'0'
        bne _derr
        clc
        rts
_derr   jsr puts
        .enc "petscii"
        .null 13, "{red}Drive: "
        .enc "none"
        jsr xm_pst
        jsr puts
        .enc "petscii"
        .null "{white}", 13
        .enc "none"
        jmp _close
_none   jsr puts
        .enc "petscii"
        .null 13, "{red}No drive 8.{white}", 13
        .enc "none"
_close  lda #2
        jsr CLOSE
        lda #15
        jsr CLOSE
        sec
        rts
_kerr   cmp #5                  ; KERNAL error: device not present?
        beq _none
        jsr puts
        .enc "petscii"
        .null 13, "{red}The file cannot be opened.{white}", 13
        .enc "none"
        jmp _close
        .pend

; the drive's status line into xm_st (ends with 13); carry set if the
; drive does not answer
xm_status .proc
        ldx #15
        jsr CHKIN
        bcs _no
        ldy #0
-       jsr CHRIN
        sta xm_st,y
        cmp #13
        beq +
        jsr READST
        bne +
        iny
        cpy #39
        bne -
+       lda #13
        sta xm_st,y
        jsr READST
        pha
        jsr CLRCHN
        pla
        bmi _no                 ; device not present
        clc
        rts
_no     jsr CLRCHN
        sec
        rts
        .pend

READST  = $ffb7

; prints xm_st up to its 13
xm_pst  .proc
        ldy #0
-       lda xm_st,y
        cmp #13
        beq +
        jsr CHROUT
        iny
        cpy #40
        bne -
+       rts
        .pend

; "s0:name" on the command channel: the partial file goes (the name
; without a drive prefix such as "@0:" and without its type)
xm_scratch .proc
        ldy #0                  ; past the last ':' before a ','
        ldx #0
_find   cpx xm_fnl
        beq _found
        lda xm_fn,x
        inx
        cmp #','
        beq _found
        cmp #':'
        bne _find
        txa
        tay
        jmp _find
_found  sty xm_from             ; where the name starts
        ldx #15
        jsr CHKOUT
        bcs _r
        lda #'s'
        jsr CHROUT
        lda #'0'
        jsr CHROUT
        lda #':'
        jsr CHROUT
        ldy xm_from
-       lda xm_fn,y
        cmp #','
        beq +
        jsr CHROUT
        iny
        cpy xm_fnl
        bne -
+       lda #13
        jsr CHROUT
_r      jmp CLRCHN
        .pend

; three CANs to the BBS
xm_cancel .proc
        lda #XM_CAN
        jsr xm_send
        lda #XM_CAN
        jsr xm_send
        lda #XM_CAN
        jmp xm_send
        .pend

; one byte to the BBS, out at once
xm_send .proc
        sta t_key
        lda #1
        sta xm_sent
        lda #<t_key
        sta net_q
        lda #>t_key
        sta net_q+1
        lda #1
        jsr tcp_write
        jmp tcp_poll
        .pend

; the transfer itself; its result in xm_res
xm_receive .proc
        tsx
        stx xm_sp
        lda #0
        sta xm_plen
        sta xm_plen+1
        sta xm_bytes
        sta xm_bytes+1
        sta xm_bytes+2
        sta xm_errs
        sta xm_sent
        lda #1
        sta xm_blk
        sta xm_mode
        jsr xm_show
        ; the start: 'C', then NAK, 3 s apart
_start  lda #XM_C
        ldx xm_mode
        bne +
        lda #XM_NAK
+       jsr xm_send
        lda #<180
        ldx #>180
        jsr xm_get
        bcc _head
        inc xm_errs
        lda xm_errs
        cmp #XM_CTRIES
        bne +
        lda #0                  ; no CRC: the checksum
        sta xm_mode
+       lda xm_errs
        cmp #XM_TRIES
        bcc _start
        lda #XR_SILENT
        jmp xm_fail

_next   lda #<600               ; the next block, within 10 s
        ldx #>600
        jsr xm_get
        bcc _head
        lda #XM_NAK             ; nothing: ask again
        jmp _retry
_head   cmp #XM_SOH
        beq _128
        cmp #XM_STX
        beq _1k
        cmp #XM_EOT
        beq _eot
        cmp #XM_CAN
        bne _bad
        lda #<60                ; a second CAN within 1 s: cancelled
        ldx #>60
        jsr xm_get
        bcs _bad
        cmp #XM_CAN
        bne _bad
        lda #XR_CAN
        jmp xm_fail
_128    lda #<128
        ldx #>128
        jmp _block
_1k     lda #<1024
        ldx #>1024
_block  sta xm_len
        stx xm_len+1
        jsr xm_block
        bcs _bad
        lda xm_hdr              ; the number and its complement
        eor xm_hdr+1
        cmp #$ff
        bne _bad
        lda xm_hdr
        cmp xm_blk
        beq _new
        clc                     ; the previous one again (our ACK lost)
        adc #1
        cmp xm_blk
        bne _seq
        lda #XM_ACK
        jsr xm_send
        jmp _next
_seq    lda #XR_SEQ
        jmp xm_fail
_new    lda #0
        sta xm_errs
        lda #XM_ACK             ; acknowledged, then written while the
        jsr xm_send             ; next one comes
        inc xm_blk
        jsr xm_flush
        jsr xm_keep
        jmp _next
_bad    lda #XM_NAK
_retry  pha
        inc xm_errs
        lda xm_errs
        cmp #10
        bcc +
        pla
        lda #XR_ERRS
        jmp xm_fail
+       jsr xm_purge
        pla
        jsr xm_send
        jmp _next
_eot    lda #XM_ACK
        jsr xm_send
        jsr xm_trim
        jsr xm_flush
        lda #XR_OK
        sta xm_res
        rts
        .pend

; ends xm_receive with the result A, from any depth
xm_fail .proc
        sta xm_res
        ldx xm_sp
        txs
        rts
        .pend

; the rest of a block: number, complement, xm_len bytes into xm_buf,
; CRC or checksum; carry set if it does not arrive or is wrong
xm_block .proc
        lda #<180               ; 3 s for each byte
        sta xm_to
        lda #>180
        sta xm_to+1
        jsr _get
        sta xm_hdr
        jsr _get
        sta xm_hdr+1
        lda #0
        sta xm_crc
        sta xm_crc+1
        lda #<xm_buf
        sta xm_p
        lda #>xm_buf
        sta xm_p+1
        lda xm_len
        sta xm_q
        lda xm_len+1
        sta xm_q+1
_data   jsr _get
        ldy #0
        sta (xm_p),y
        jsr xm_sum
        inc xm_p
        bne +
        inc xm_p+1
+       lda xm_q
        bne +
        dec xm_q+1
+       dec xm_q
        lda xm_q
        ora xm_q+1
        bne _data
        lda xm_mode
        beq _chk
        jsr _get                ; CRC, high byte first
        cmp xm_crc+1
        bne _wrong
        jsr _get
        cmp xm_crc
        bne _wrong
        clc
        rts
_chk    jsr _get
        cmp xm_crc
        bne _wrong
        clc
        rts
_wrong  sec
        rts
_get    lda xm_to               ; a byte, or out of xm_block (carry set)
        ldx xm_to+1
        jsr xm_get
        bcs +
        rts
+       pla
        pla
        sec
        rts
        .pend

; adds A to the CRC-16 (polynomial $1021) or to the checksum
xm_sum  .proc
        ldx xm_mode
        bne _crc
        clc
        adc xm_crc
        sta xm_crc
        rts
_crc    eor xm_crc+1
        sta xm_crc+1
        ldx #8
-       asl xm_crc
        rol xm_crc+1
        bcc +
        lda xm_crc+1
        eor #$10
        sta xm_crc+1
        lda xm_crc
        eor #$21
        sta xm_crc
+       dex
        bne -
        rts
        .pend

; a byte from the BBS within A/X ticks: carry clear with it in A, carry
; set if none came; RUN/STOP or a lost connection end the transfer
xm_get  .proc
        clc
        adc ticks
        sta xm_dl
        txa
        adc ticks+1
        sta xm_dl+1
_wait   jsr tcp_read
        bcs +
        rts
+       jsr net_poll
        jsr tcp_poll
        lda tcp_state
        cmp #TCP_OPEN
        beq +
        lda #XR_LOST
        jmp xm_fail
+       jsr GETIN
        cmp #3
        bne +
        lda #XR_STOP
        jmp xm_fail
+       lda ticks               ; past the deadline?
        sec
        sbc xm_dl
        lda ticks+1
        sbc xm_dl+1
        bmi _wait
        sec
        rts
        .pend

; what is left of a bad block, until 1 s of silence
xm_purge .proc
-       lda #<60
        ldx #>60
        jsr xm_get
        bcc -
        rts
        .pend

; the pending block to the disk
xm_flush .proc
        lda xm_plen
        ora xm_plen+1
        beq _r
        ldx #2
        jsr CHKOUT
        lda #<xm_pend
        sta xm_p
        lda #>xm_pend
        sta xm_p+1
        lda xm_plen
        sta xm_q
        lda xm_plen+1
        sta xm_q+1
-       ldy #0
        lda (xm_p),y
        jsr CHROUT
        inc xm_p
        bne +
        inc xm_p+1
+       lda xm_q
        bne +
        dec xm_q+1
+       dec xm_q
        lda xm_q
        ora xm_q+1
        bne -
        jsr CLRCHN
        lda xm_bytes            ; counted, and shown
        clc
        adc xm_plen
        sta xm_bytes
        lda xm_bytes+1
        adc xm_plen+1
        sta xm_bytes+1
        bcc +
        inc xm_bytes+2
+       lda #0
        sta xm_plen
        sta xm_plen+1
        jmp xm_show
_r      rts
        .pend

; the block just received becomes the pending one
xm_keep .proc
        lda #<xm_buf
        sta xm_p
        lda #>xm_buf
        sta xm_p+1
        lda #<xm_pend
        sta xm_q
        lda #>xm_pend
        sta xm_q+1
        ldx xm_len+1            ; whole pages (4 for 1K, none for 128)
        ldy #0
        txa
        beq _part
-       lda (xm_p),y
        sta (xm_q),y
        iny
        bne -
        inc xm_p+1
        inc xm_q+1
        dex
        bne -
_part   ldx xm_len
        beq +
-       lda (xm_p),y
        sta (xm_q),y
        iny
        dex
        bne -
+       lda xm_len
        sta xm_plen
        lda xm_len+1
        sta xm_plen+1
        rts
        .pend

; the padding (SUB) off the end of the pending block, the last one
xm_trim .proc
-       lda xm_plen
        ora xm_plen+1
        beq _r
        lda xm_plen             ; the byte at plen - 1
        sec
        sbc #1
        sta xm_p
        lda xm_plen+1
        sbc #0
        clc
        adc #>xm_pend
        sta xm_p+1
        lda xm_p
        clc
        adc #<xm_pend
        sta xm_p
        bcc +
        inc xm_p+1
+       ldy #0
        lda (xm_p),y
        cmp #XM_SUB
        bne _r
        lda xm_plen
        bne +
        dec xm_plen+1
+       dec xm_plen
        jmp -
_r      rts
        .pend

; the bytes saved, at the counter's place
xm_show .proc
        lda xm_row
        tax
        ldy #29
        clc
        jsr PLOT
        ldx #2                  ; xm_bytes into decimal digits, lowest first
-       lda xm_bytes,x
        sta xm_t,x
        dex
        bpl -
        ldy #0
_div    lda #0                  ; xm_t / 10, remainder in A
        ldx #24
-       asl xm_t
        rol xm_t+1
        rol xm_t+2
        rol a
        cmp #10
        bcc +
        sbc #10
        inc xm_t
+       dex
        bne -
        ora #'0'
        sta xm_dig,y
        iny
        lda xm_t
        ora xm_t+1
        ora xm_t+2
        bne _div
-       dey
        lda xm_dig,y
        jsr CHROUT
        tya
        bne -
        rts
        .section bss
xm_t    .fill 3
        .send bss
        .pend
