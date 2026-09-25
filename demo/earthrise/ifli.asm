; EARTHRISE - IFLI picture viewer for 64tass (without -a)
;
; Two multicolor FLI pictures on alternate frames: the eye mixes them into
; up to 16x16 colors, and frame B is one hires pixel to the right ($D016 = 1)
; so the mix has 320 pixels of horizontal detail. Data from convert.py.
;
; Memory:
;   $0801-$3FFF  code, color RAM tables
;   $4000-$7FFF  frame A: screens 0-7 at $4000-$5FFF, bitmap at $6000 (bank 1)
;   $8000-$BFFF  frame B as loaded, copied at start to $C000-$FFFF (bank 3:
;                banks 0 and 2 show the character ROM at $1000-$1FFF, which
;                would cover four screens). $D000-$DFFF is RAM under the I/O,
;                and the VIC always sees RAM.
;
; Nothing runs on interrupts: after one sync to the exact cycle, every
; frame takes exactly 312 x 63 cycles of straight code and counted delays.
;
; FLI: a bad line on every raster line, forced by writing $D011 with the
; Y scroll of the line at cycle 14, after the row counter check of cycle 14
; (earlier would restart the character row) and before the c-accesses: the
; VIC fetches the 40 screen bytes again, from the screen that $D018 chose,
; so every line has its own two screen colors per 4x1 cell. The bad line
; stops the CPU until cycle 54, which keeps the loop in step: 23 cycles per
; line. The VIC starts the forced fetch three cycles late, so the first
; three character columns show the FLI bug; the picture keeps them black.
; Raster $33 is a normal bad line (Y scroll 3); bad lines cannot be forced
; after $F7, so lines 197-199 keep the colors fetched at line 196.
;
; Between frames the code switches the VIC bank ($DD00) and $D016 and
; rewrites the color RAM cells that differ between the frames.
;
; Music and VU meters: the player (player.asm, tune from music.py) runs
; below the picture and takes a different time every frame, so the code
; syncs to the exact cycle again after it, as at the start. Then, in
; constant time, it draws the meters and rewrites the color RAM. The meters
; are six X-expanded sprites in the lower border, opened by switching to 24
; rows at line $F8 (after the 24-row compare at $F7, before the 25-row one
; at $FB) and back to 25 rows before $33. Each bank has its own copy of the
; sprites, in the blocks the pictures do not use: row 24 of screens 5-7
; (lines 197-199 use screen 4) and the end of the bitmap. The meters are
; drawn into the bank of the next frame.
;
; SPACE goes back to BASIC; RUN starts the picture again.

; --- timing constants (cycles of a line, 0-62, as in the emulator's VIC) ----
WRITE   = 11            ; cycle of the $D011 write that starts each frame (line $33)
RESUME  = 54            ; first CPU cycle after a bad line
FILLER  = 11            ; per line: 23 - 12 cycles of LDA/STA
SYNCS   = 280           ; `sync` aligns on this line, after the meters' sprites
SYNCLN  = SYNCS + 16    ; line and cycle at which it returns
SYNCED  = 15

; cycles from the write at line $33 of one frame to the same write in the next
FRAME   = 312 * 63
; from the return of `sync` to WRITE of the next frame
AFTERSYNC = (312 - SYNCLN + $33) * 63 + WRITE - SYNCED

; --- meters ---------------------------------------------------------------
VUSEGS  = 36            ; segments per bar: 6 sprites x 3 bytes x 2
PEAKHOLD = 30           ; frames the peak stays before falling
VUY     = $fc           ; sprite Y: raster lines 252-272
VUX     = 40            ; X of the first sprite (48 pixels each)
; sprite blocks in both banks: row 24 of screens 5-7, end of the bitmap
VUBLK   = [$17c0, $1bc0, $1fc0, $3f40, $3f80, $3fc0]
VUCOL   = [5, 5, 5, 5, 7, 2]  ; green, yellow, red
VUCYC   = 3 * (8 + 18 * 28)   ; cycles of draw_a/draw_b without JSR/RTS

src     = $fb           ; copy pointers (start only)
dst     = $fd
zp      = $fb           ; player
tmp     = $fd
tmp2    = $fe
tmp3    = $04
bank_a  = $02           ; $DD00 values of the two frames
bank_b  = $03

; --- delays -----------------------------------------------------------------

; Exactly \1 cycles (0 or >= 2) with NOP and BIT $EA.
nops    .macro
        .if \1 % 2
        bit $ea
        .rept (\1 - 3) / 2
        nop
        .endrept
        .else
        .rept \1 / 2
        nop
        .endrept
        .endif
        .endm

; Exactly \1 cycles (0 or >= 2): calls to `wait` (5n + 13 cycles with
; LDX #n) and NOPs.
delay   .macro
rest    := \1
        .while rest >= 5 * 256 + 13 + 2
        ldx #0
        jsr wait
rest    := rest - (5 * 256 + 13)
        .endwhile
        .if rest >= 20
n       := (rest - 13) / 5
        .if rest - 13 - 5 * n == 1
n       := n - 1
        .endif
        ldx #n & 255
        jsr wait
rest    := rest - (5 * n + 13)
        .endif
        #nops rest
        .endm

; --- BASIC line: 2026 SYS2061 -----------------------------------------------
        * = $0801
        .word +, 2026
        .null $9e, format("%d", start)
+       .word 0

start   sei
        lda #$7f
        sta $dc0d
        sta $dd0d
        lda $dc0d
        lda $dd0d
        lda #$0b                ; screen off: no bad lines until the sync
        sta $d011
        lda #0
        sta $d015
        sta $d01a
        sta $d020
        sta $d021
        lda #$ff
        sta $d019
        lda $dd00
        and #$fc
        sta bank_b              ; bank 3 ($C000): bits 00
        ora #2
        sta bank_a              ; bank 1 ($4000): bits 10

        lda #$34                ; all RAM: copy frame B to $C000-$FFFF
        sta $01
        lda #0
        sta src
        sta dst
        lda #$80
        sta src+1
        lda #$c0
        sta dst+1
        ldx #$40
        ldy #0
-       lda (src),y
        sta (dst),y
        iny
        bne -
        inc src+1
        inc dst+1
        dex
        bne -
        lda #$35                ; I/O back, no ROMs: our vectors at $FFFA
        sta $01
        lda #<nmi               ; the NMI below comes once: then these
        sta $fffa               ; two bytes are a meter sprite's
        lda #>nmi
        sta $fffb
        lda #0                  ; no IRQs; $FFFF is also the idle byte of
        sta $fffe               ; bank 3, seen in the open borders
        sta $ffff

        ; RESTORE would break the timing: a CIA2 NMI that is never
        ; acknowledged keeps the NMI line low, so no other edge can come.
        lda #1
        sta $dd04
        lda #0
        sta $dd05
        lda #$81
        sta $dd0d
        lda #$19                ; start, one shot, load
        sta $dd0e

        ldx #0                  ; color RAM of frame A
-       lda cram_a,x
        sta $d800,x
        lda cram_a+250,x
        sta $d800+250,x
        lda cram_a+500,x
        sta $d800+500,x
        lda cram_a+750,x
        sta $d800+750,x
        inx
        cpx #250
        bne -

        lda #$7f                ; keyboard row 7, for SPACE
        sta $dc00
        lda bank_a
        sta $dd00
        lda #$18                ; multicolor, 40 columns
        sta $d016
        lda #$08                ; screen 0, bitmap at +$2000
        sta $d018

        ldx #5                  ; meter sprites
-       lda vucol,x
        sta $d027,x
        lda vuptr,x
        sta $43f8,x
        sta $c3f8,x
        dex
        bpl -
        ldx #0
-       lda vux,x
        sta $d000,x
        lda #VUY
        sta $d001,x
        inx
        inx
        cpx #12
        bne -
        lda #$20                ; sprite 5: X over 255
        sta $d010
        lda #$3f
        sta $d01d               ; X expanded
        sta $d015
        lda #0
        sta $d017
        sta $d01b
        sta $d01c               ; hires

        jsr init                ; music

        lda #$f8                ; the coarse wait of `sync` must start after $F7
-       cmp $d012
        bne -
        jsr sync                ; line SYNCLN, cycle SYNCED
synced  lda #$3b                ; bitmap, screen on (bad lines from the next frame)
        sta $d011
        ; to the first write, as frame_b's jump to frame_a
        #delay (312 - SYNCLN + $33) * 63 + WRITE - SYNCED - (6 + 3 + 2 + 6 + 4 - 1)
        jmp frame_a

; --- the two frames ---------------------------------------------------------
; From the return of `sync`: $01 10 (zero page), meters JSR+RTS 12 + VUCYC,
; color RAM JSR+RTS 12 + 6 per cell, SPACE 8, the delay, then bank, $D016
; and $D011 19, LDA 2 + JSR 6 + STA 4 to the write (its last cycle).
FIXED   = 10 + 12 + VUCYC + 12 + 6 * NDIFF + 8 + 19 + 2 + 6 + 4 - 1

frame_a lda #$3b
        jsr fli
back_a  lda #$33                ; 24 rows at $F8: the lower border stays open
        sta $d011
        lda #$08
        sta $d018
        jsr play                ; music and meter levels, in variable time
        jsr sync
        lda #$34                ; bank 3 sprites are under the I/O
        sta $01
        jsr draw_b
        lda #$35
        sta $01
        jsr copy_b
        lda $dc01               ; SPACE?
        and #$10
        beq exit
        #delay AFTERSYNC - FIXED
show_b  lda bank_b              ; now in the top border
        sta $dd00
        lda #$19                ; frame B one hires pixel to the right
        sta $d016
        lda #$3b                ; 25 rows again before the compare at $33
        sta $d011

frame_b lda #$3b
        jsr fli
        lda #$33
        sta $d011
        lda #$08
        sta $d018
        jsr play
        jsr sync
        lda #$34
        sta $01
        jsr draw_a
        lda #$35
        sta $01
        jsr copy_a
        lda $dc01
        and #$10
        beq exit
        #delay AFTERSYNC - FIXED - 3
show_a  lda bank_a
        sta $dd00
        lda #$18
        sta $d016
        lda #$3b
        sta $d011
        jmp frame_a

exit    lda #$7f                ; release the NMI line
        sta $dd0d
        lda $dd0d
        lda #$37
        sta $01
        lda #$1b
        sta $d011
        lda #$c8
        sta $d016
        lda #$15
        sta $d018
        lda bank_b
        ora #3
        sta $dd00
        lda #14
        sta $d020
        lda #6
        sta $d021
        lda #$ff
        sta $dc00
        lda #0
        sta $d015
        sta $d01d
        sta $d418               ; silence
        lda #$81                ; KERNAL timer interrupt back on
        sta $dc0d
        jsr $e544               ; clear screen (and color RAM)
-       lda $dc01               ; wait for SPACE to be released
        and #$10
        beq -
        cli
        jmp ($a002)             ; BASIC warm start

nmi     rti

        .align 256              ; the branch must not cross a page
wait    dex
        bne wait
        rts

; --- sync -------------------------------------------------------------------
; Returns at cycle SYNCED of raster line SYNCLN, called after line $F7 and
; before SYNCS - 1 (lines over 255 read as $D012 = line - 256), without bad
; lines or sprites in the lines it waits. The coarse wait leaves 0-6 cycles of jitter; then
; each 63-cycle iteration reads $D012 at the same point of a line and, while
; the read comes before the line change, the taken BEQ adds one cycle, until
; the read lands on the change (see tools/vice_tests.py, Timing.stabilize).
sync    ldx #<(SYNCS - 1)
-       cpx $d012
        bne -
        ldy #16
        ldx #<SYNCS
        .page
-       cpx $d012
        bne -
loop    #nops 50
        cpx $d012
        beq +
+       inx
        dey
        bne loop
        .endp
        rts

; --- FLI: 196 forced bad lines, raster $34-$F7 ------------------------------
; Called so that the first STA writes $3B to $D011 at cycle WRITE of line
; $33; the CPU stops right after it, at the next read, until cycle RESUME.
; Each line then writes $D018 (screen y & 7) in the previous line's right
; border and $D011 (Y scroll = raster & 7) at cycle 14. It returns after
; the bad line of $F7, at cycle RESUME.
fli     sta $d011
        .for y = 1, y < 197, y += 1
        lda #((y & 7) << 4) | 8
        sta $d018
        lda #$38 | ((y + 3) & 7)
        #nops FILLER
        sta $d011
        .next
        rts

; --- meters -----------------------------------------------------------------
; draw_a/draw_b: the three bars (rows 1-5, 8-12, 15-19 of the sprites) in
; bank 1 or 3, in constant time. Byte k of a bar (sprite k / 3, byte k % 3)
; holds segments 2k and 2k+1: vultab + k gives it for the level, vuptab + k
; the peak segment.
draw    .macro bank
        .for v = 0, v < 3, v += 1
        ldx level+v
        ldy peak+v
        .for k = 0, k < 18, k += 1
        lda vultab + VUTAB(k),x
        ora vuptab + VUTAB(k),y
        .for r = 0, r < 5, r += 1
        sta \bank + VUBLK[k / 3] + (1 + 7 * v + r) * 3 + k % 3
        .next
        .next
        .next
        rts
        .endm

draw_a  #draw $4000
draw_b  #draw $c000

vux     .for i = 0, i < 6, i += 1
        .byte <(VUX + 48 * i), 0
        .next
vucol   .byte VUCOL
vuptr   .for i = 0, i < 6, i += 1
        .byte VUBLK[i] / 64
        .next

; level and peak tables of byte k, 42 bytes apart and 6 to a page, so that
; the indexed reads never cross a page
VUTAB   .function k
        .endf (k / 6) * 256 + (k % 6) * 42
        .align 256
vultab  .for k = 0, k < 18, k += 1
        .fill VUTAB(k) - (* - vultab)
        .for n = 0, n <= VUSEGS, n += 1
        .byte n <= 2 * k ? 0 : n == 2 * k + 1 ? $e0 : $ee
        .next
        .next
        .align 256
vuptab  .for k = 0, k < 18, k += 1
        .fill VUTAB(k) - (* - vuptab)
        .for n = 0, n <= VUSEGS, n += 1
        .byte n - 1 == 2 * k ? $e0 : n - 1 == 2 * k + 1 ? $0e : 0
        .next
        .next

        .include "player.asm"
        .include "music.asm"
        .include "build/ifli_colors.asm"   ; NDIFF, copy_a, copy_b, cram_a

        .cerror * > $4000, "code over $4000"

; --- pictures ---------------------------------------------------------------
        * = $4000
        .binary "build/ifli_a.bin"
        * = $8000
        .binary "build/ifli_b.bin"
