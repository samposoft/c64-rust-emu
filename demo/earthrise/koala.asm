; EARTHRISE - the same picture as a Koala (multicolor bitmap), for
; comparison with the IFLI version. For 64tass (without -a); data from
; convert.py (build/koala.bin: bitmap, screen, color RAM).
;
; Bank 1: screen at $5C00, bitmap at $6000. SPACE goes back to BASIC.

        * = $0801
        .word +, 2026
        .null $9e, format("%d", start)
+       .word 0

start   lda #0
        sta $d020
        sta $d021
        ldx #0                  ; color RAM
-       lda cram,x
        sta $d800,x
        lda cram+250,x
        sta $d800+250,x
        lda cram+500,x
        sta $d800+500,x
        lda cram+750,x
        sta $d800+750,x
        inx
        cpx #250
        bne -
        lda $dd00               ; bank 1 ($4000)
        and #$fc
        ora #2
        sta $dd00
        lda #$78                ; screen $5C00, bitmap $6000
        sta $d018
        lda #$18                ; multicolor, 40 columns
        sta $d016
        lda #$3b                ; bitmap, screen on
        sta $d011

-       jsr $ffe4               ; GETIN: wait for SPACE
        cmp #' '
        bne -

        lda #$1b
        sta $d011
        lda #$c8
        sta $d016
        lda #$15
        sta $d018
        lda $dd00
        ora #3
        sta $dd00
        lda #14
        sta $d020
        lda #6
        sta $d021
        jmp $e544               ; clear screen, back to BASIC

cram    .binary "build/koala.bin", 9000, 1000

        .cerror * > $5c00, "code over $5C00"

        * = $5c00
        .binary "build/koala.bin", 8000, 1000
        * = $6000
        .binary "build/koala.bin", 0, 8000
