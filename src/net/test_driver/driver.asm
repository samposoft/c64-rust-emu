; SPDX-License-Identifier: GPL-2.0-or-later
; Copyright (C) 2026 SampoSoft - Francesco Sampoli
;
; Test driver of the RR-Net at $DE00, for the tests in src/net/mod.rs: it
; sends the frame the test puts in memory and stores every frame it
; receives, through the CS8900A registers as ip65 does. Needs no ROM: the
; test starts it at $C000.
;
; Build: 64tass -a -b -o src/net/test_driver/driver.bin src/net/test_driver/driver.asm
;
; $C800      command: 1 = send the frame at $C900 (length at $C802), back
;            to 0 once sent
; $C804      frames received (16 bits)
; $C806      where the next received frame goes: its length (16 bits),
;            then its bytes; from $4000
; $C810-15   MAC address

PPPTR   = $de02
PPDATA  = $de04
RXTX    = $de08
TXCMD   = $de0c
TXLEN   = $de0e

CMD     = $c800
LEN     = $c802
COUNT   = $c804
WPTR    = $c806
MAC     = $c810
FRAME   = $c900

ptr     = $fb
cnt     = $fd

ppsel   .macro
        lda #<\1
        sta PPPTR
        lda #>\1
        sta PPPTR+1
        .endm

ppwr    .macro
        #ppsel \1
        lda #<\2
        sta PPDATA
        lda #>\2
        sta PPDATA+1
        .endm

        * = $c000
start   sei
        ldx #$ff
        txs
        lda #<$4000
        sta WPTR
        lda #>$4000
        sta WPTR+1
        lda #0
        sta COUNT
        sta COUNT+1
        sta CMD
        ldx #0                  ; MAC address, a word at a time
-       txa
        clc
        adc #$58
        sta PPPTR
        lda #$01
        sta PPPTR+1
        lda MAC,x
        sta PPDATA
        lda MAC+1,x
        sta PPDATA+1
        inx
        inx
        cpx #6
        bne -
        #ppwr $0104, $0d05      ; RxCTL: RxOK, individual, broadcast
        #ppwr $0112, $00d3      ; LineCTL: SerRxON, SerTxON
main    lda CMD
        cmp #1
        bne +
        jsr send
        lda #0
        sta CMD
+       jsr poll
        jmp main

; Sends the frame at FRAME, LEN bytes
send    lda #$c9                ; TxCMD: start after the whole frame
        sta TXCMD
        lda #0
        sta TXCMD+1
        lda LEN
        sta TXLEN
        lda LEN+1
        sta TXLEN+1
        #ppsel $0138            ; BusST: wait for Rdy4TxNOW
-       lda PPDATA
        lda PPDATA+1
        and #1
        beq -
        lda #<FRAME
        sta ptr
        lda #>FRAME
        sta ptr+1
        lda LEN
        sta cnt
        lda LEN+1
        sta cnt+1
        ldy #0
sloop   lda cnt
        ora cnt+1
        beq sdone
        lda (ptr),y
        sta RXTX                ; low byte
        jsr next
        lda cnt
        ora cnt+1
        beq sdone
        lda (ptr),y
        sta RXTX+1              ; high byte
        jsr next
        jmp sloop
sdone   rts

; A received frame, if any, to (WPTR)
poll    #ppsel $0124            ; RxEvent
        lda PPDATA
        lda PPDATA+1
        and #1                  ; RxOK
        bne +
        rts
+       lda RXTX+1              ; RxStatus, high byte first
        lda RXTX
        lda RXTX+1              ; RxLength, high byte first
        sta cnt+1
        lda RXTX
        sta cnt
        lda WPTR
        sta ptr
        lda WPTR+1
        sta ptr+1
        ldy #0
        lda cnt
        sta (ptr),y
        iny
        lda cnt+1
        sta (ptr),y
        clc
        lda ptr
        adc #2
        sta ptr
        lda ptr+1
        adc #0
        sta ptr+1
        ldy #0
rloop   lda cnt
        ora cnt+1
        beq rdone
        lda RXTX                ; frame: low byte first
        sta (ptr),y
        jsr next
        lda cnt
        ora cnt+1
        beq rdone
        lda RXTX+1
        sta (ptr),y
        jsr next
        jmp rloop
rdone   lda ptr
        sta WPTR
        lda ptr+1
        sta WPTR+1
        inc COUNT
        bne +
        inc COUNT+1
+       rts

; ptr + 1, cnt - 1
next    inc ptr
        bne +
        inc ptr+1
+       lda cnt
        bne +
        dec cnt+1
+       dec cnt
        rts
