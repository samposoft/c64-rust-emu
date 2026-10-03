; SPDX-License-Identifier: GPL-2.0-or-later
; Copyright (C) 2026 SampoSoft - Francesco Sampoli
;
; SIDWAVE: a synthwave sunset for the C64, with an original SID tune.
; See README.md. Generated files: music.asm (music.py), gfx_*.asm (gfx.py).
;
; Frame (PAL raster lines):
;   46       interrupt: stabilizes the raster (double interrupt), then the
;            kernel colours every line from 51 to 192 (gfx_code.asm)
;   193-253  interrupts for the events: the near grid lines (at most two),
;            the 24 rows that open the borders and back to 25
;   253      still in the last event's interrupt: scroller sprites, music
;   295      interrupt: sprites of the logo, registers for the sky
;   the main loop prepares the next frame from line 192 on (the
;   interrupts come in between): sprites, grid, sun, buildings

MZP     = $fb                   ; player pointers
MZP2    = $fd

SCREEN  = $4000                 ; VIC bank 1
SPRITES = $4400                 ; logo letters at pointer $10, scroller at $17
SKYCH   = $4800
GNDCH   = $5000
FONT    = $5800                 ; the ROM's upper case set, copied at start
SPR_LOGO = $10
SPR_SCROLL = $17
SCROLLSPR = SPRITES + 7 * 64

STAB_LINE = 46                  ; the first interrupt (the second comes on the next line)
LOGO_Y  = 18                    ; logo top line (it bounces down to +6: done by line 45)
SCROLL_Y = 262 - 256            ; scroller top line (262) in the 8-bit sprite Y
LOGO_IRQ = 295                  ; after the scroller (21 lines from 262-270)
SUSTAIN = 16                    ; building height of a held note

; zero page
frameflag = $02
fcount  = $03                   ; frames (2 bytes)
section = $05
secfr   = $06                   ; frames since the section started (2 bytes)
gsub    = $08                   ; grid phase: fraction and 0-63
gphase  = $09
gspeed  = $0a                   ; grid speed in 1/16 phase per frame
ssub    = $0b                   ; sun stripes: fraction and 0-15
sphase  = $0c
fade    = $0d                   ; sun fade level (0 full)
flash   = $0e                   ; frames of grid flash left
ptr     = $0f                   ; 2 bytes
ptr2    = $11
sbase   = $13                   ; scroller: x of the first sprite (-47..0)
sfirst  = $14                   ; slot of the leftmost sprite
tptr    = $15                   ; text pointer (2 bytes)
rnd     = $17
tmp     = $18
tmp2    = $19
nold    = $1a                   ; grid lines drawn last frame
lvl     = $1b                   ; envelope of the voices for the buildings (3)
bump    = $1e                   ; logo letter bumps (7)
lastsec = $25
sunfade = $2a                   ; fade level and stripe phase in kd023
sunphase = $2b
newnote = $26                   ; a note started on voice 1, 2, 3 (3)
sbasehi = $29

STAB_WAIT = 9                   ; raster stabilization (measured)
STAB_NOPS = 0
KWAIT   = 25                    ; from the stable point to the kernel
KNOPS   = 3
TPHASE  = 40                    ; (measured)
EV_CYCLE = 50                   ; near grid lines: the timer read; the write comes 12-20 cycles later

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
        ldx #2                  ; save the zero page for BASIC
-       lda $00,x
        sta zpsave,x
        inx
        bne -
        lda #$33                ; character ROM in
        sta $01
        ldx #0
-       lda $d000,x
        sta FONT,x
        lda $d100,x
        sta FONT+$100,x
        inx
        bne -
        lda #$35                ; RAM, I/O, no ROMs
        sta $01
        lda #0
        sta $d020
        sta $d021
        sta $d015
        sta $7fff               ; idle byte: the open borders show $d021
        ; colour RAM
        lda #<colourdata
        sta ptr
        lda #>colourdata
        sta ptr+1
        lda #<$d800
        sta ptr2
        lda #>$d800
        sta ptr2+1
        ldx #4
        ldy #0
-       lda (ptr),y
        sta (ptr2),y
        iny
        bne -
        inc ptr+1
        inc ptr2+1
        dex
        bne -
        ; scroller sprites: empty
        lda #0
        tax
-       sta SCROLLSPR,x
        sta SCROLLSPR+$100,x
        inx
        bne -
        ; kernel tables
        ldx #0
-       lda kd021_init,x
        sta kd021,x
        inx
        cpx #KLINES
        bne -
        ldx #0
-       lda skycol,x
        sta kd023,x
        inx
        cpx #104
        bne -
        ; variables
        lda #0
        ldx #lastsec-frameflag
-       sta frameflag,x
        dex
        bpl -
        sta sbase
        sta sfirst
        lda #$ff
        sta lastsec
        lda #FADE_LEVELS-1
        sta fade
        lda #$ff
        sta sunfade
        lda #$5a
        sta rnd
        ldx #BARS-1
-       lda barmin,x
        sta barh,x
        lda #$ff
        sta bard,x
        dex
        bpl -
        lda #<scrolltext
        sta tptr
        lda #>scrolltext
        sta tptr+1
        ldx #0                  ; the first 8 sprites of text
-       stx sfirst
        jsr render
        ldx sfirst
        inx
        cpx #8
        bne -
        lda #0
        sta sfirst
        jsr scroller
        jsr logo
        jsr ground              ; the first list of events
        jsr drawbars_a
        jsr drawbars_b
        ; VIC
        lda $dd00
        and #$fc
        ora #2                  ; bank 1 ($4000)
        sta $dd00
        lda #$02                ; screen $4000, characters $4800
        sta $d018
        lda #$18                ; multicolour, 40 columns
        sta $d016
        lda #1
        sta $d022
        lda #1                  ; logo: chrome white on top, pink below
        sta $d025
        lda #10
        sta $d026
        jsr mus_init
        lda #<nmi
        sta $fffa
        lda #>nmi
        sta $fffb
        lda #<irq_l
        sta $fffe
        lda #>irq_l
        sta $ffff
        lda #$9b                ; 25 rows; the interrupt at LOGO_IRQ (bit 8)
        sta $d011
        lda #LOGO_IRQ-256
        sta $d012
        lda #1
        sta $d01a
        sta $d019
        cli

; --- main loop: one frame of work after the music ---------------------------
main    lda frameflag
        beq main
        lda #0
        sta frameflag
        ldx #0                  ; the notes started by the player
        lda trig
        stx trig
        sta newnote
        lda trig+7
        stx trig+7
        sta newnote+1
        lda trig+14
        stx trig+14
        sta newnote+2
        jsr scroller            ; the sprites first: their interrupts come at 252 and 295
        jsr logo
        jsr script
        jsr ground
        jsr sun
        jsr meters
        lda fcount              ; half of the buildings per frame
        lsr
        bcc +
        jsr drawbars_a
        jmp ++
+       jsr drawbars_b
+
        jsr stars
workend lda #$7f                ; SPACE: back to BASIC
        sta $dc00
        lda $dc01
        and #$10
        bne main
        jmp exit

nmi     rti

; --- interrupts --------------------------------------------------------------
irq_k   pha
        txa
        pha
        tya
        pha
        lda #<irq_k2
        sta $fffe
        lda #>irq_k2
        sta $ffff
        inc $d012
        asl $d019
        tsx
        cli
        .fill 20, $ea           ; the second interrupt comes during these NOPs
irq_k2  txs                     ; (drops its return address: we go on from here)
        ldx #STAB_WAIT
-       dex
        bne -
        .fill STAB_NOPS, $ea
        lda #STAB_LINE+1
        cmp $d012               ; still on the same line: one cycle more
        beq +
+
stable  lda #62+TPHASE          ; CIA1 timer A: period 63 cycles, a line;
        sta $dc04               ; restarted here, on a fixed cycle, every frame,
        lda #0                  ; from 62 + TPHASE so that it reads 62 - cycle
        sta $dc05
        lda #%00010001
        sta $dc0e
        lda #62
        sta $dc04
        jsr kernel_pre          ; the values of line 51
        ldx #KWAIT
-       dex
        bne -
        .fill KNOPS, $ea
        jsr kernel              ; lines 51-192
        lda evbuild             ; the events built last frame; the next ones go in the other half
        sta evpos
        clc
        adc evcount
        sta evend
        lda evbuild
        eor #8
        sta evbuild
        inc frameflag           ; the main loop prepares the next frame from here on
        jmp ev_run

; the events below the kernel: near grid lines, the open borders
irq_e   pha
        txa
        pha
        tya
        pha
        asl $d019
ev_run  ldx evpos
        lda evline,x            ; more than 4 lines away: an interrupt 3 lines before
        sec
        sbc #4
        cmp $d012
        bcc ev_now
        beq ev_now
        adc #0                  ; (carry set: +1)
        sta $d012
        lda #<irq_e
        sta $fffe
        lda #>irq_e
        sta $ffff
        pla
        tay
        pla
        tax
        pla
        rti
ev_now  ldy evline,x            ; wait for the line before
        dey
-       cpy $d012
        beq +
        bcs -
+       ldy evreg,x
        bne ev_d011
-       lda $dc04               ; then for its last cycles (after a bad line's DMA too)
        cmp #63-EV_CYCLE
        bcs -
        lda evval,x
        sta $d021
        jmp ev_next
ev_d011 lda evval,x
        sta $d011
ev_next inx
        stx evpos
        cpx evend
        bne ev_run

        ; lower border: scroller sprites, then the music
        lda #$ff
        sta $d015
        sta $d01d
        lda #0
        sta $d01c
.for i = 0, i < 8, i += 1
        lda scr_x+i
        sta $d000+2*i
        lda scr_y+i
        sta $d001+2*i
        lda scr_col+i
        sta $d027+i
        lda #SPR_SCROLL+i
        sta SCREEN+$3f8+i
.next
        lda scr_msb
        sta $d010
        asl $d019               ; the next interrupt before the music: if the music
        lda #<irq_l             ; runs past that line, it comes right after
        sta $fffe
        lda #>irq_l
        sta $ffff
        lda #LOGO_IRQ-256
        sta $d012
        jsr mus_play
        pla
        tay
        pla
        tax
        pla
        rti

irq_l   pha
        txa
        pha
        lda #$7f
        sta $d015
        sta $d01c
.for i = 0, i < 7, i += 1
        lda logo_x+i
        sta $d000+2*i
        lda logo_y+i
        sta $d001+2*i
        lda logo_col+i
        sta $d027+i
        lda #SPR_LOGO+i
        sta SCREEN+$3f8+i
.next
        lda logo_msb
        sta $d010
        lda #$18                ; the sky: multicolour, sky characters, white stars
        sta $d016
        lda #$02
        sta $d018
        lda #1
        sta $d022
        lda #<irq_k
        sta $fffe
        lda #>irq_k
        sta $ffff
        lda #$1b
        sta $d011
        lda #STAB_LINE
        sta $d012
        asl $d019
        pla
        tax
        pla
        rti

; --- back to BASIC -----------------------------------------------------------
exit    sei
        lda #0
        sta $d01a
        sta $d015
        sta $d418
        lda #$ff
        sta $d019
        ldx #2
-       lda zpsave,x
        sta $00,x
        inx
        bne -
        lda #$37
        sta $01
        jsr $fda3               ; IOINIT: CIAs, the KERNAL's timer interrupt
        jsr $ff81               ; CINT: VIC, clear screen
        cli
        jmp ($a002)             ; BASIC warm start

; --- the show, from the music's sync values -------------------------------
; sections: 1 intro, 2 verse, 3 chorus, 4 break, 5 build, 6 chorus 2, 7 outro
script  inc fcount
        bne +
        inc fcount+1
+       lda sync
        cmp lastsec
        beq +
        sta lastsec
        sta section
        lda #0
        sta secfr
        sta secfr+1
+       inc secfr
        bne +
        inc secfr+1
+       ldx section
        lda secspeed,x          ; grid speed
        bpl +
        jsr speedramp
+       sta gspeed
        lda secfr               ; A = frames / 128
        asl
        lda secfr+1
        rol
        cpx #1                  ; intro: the sun rises (a level every 128 frames)
        bne +
        sta tmp
        lda #FADE_LEVELS-1
        sec
        sbc tmp
        bcs sc_fade
        lda #0
        beq sc_fade
+       cpx #7                  ; outro: it sets from frame 256
        bne +
        sec
        sbc #2
        bcc +
        cmp #FADE_LEVELS
        bcc sc_fade
        lda #FADE_LEVELS-1
        bne sc_fade
+       lda #0
sc_fade sta fade
        lda newnote             ; a kick: the grid flashes (in the choruses)
        beq +
        ldy inst
        lda i_vis,y
        cmp #1
        bne +
        lda secflash,x
        sta flash
+       rts

; build: 8 -> 56 over 4 bars; outro: 48 -> 0 over 8 bars
speedramp
        lda secfr+1
        sta tmp
        lda secfr
        lsr tmp
        ror
        lsr tmp
        ror
        lsr tmp
        ror                     ; frames / 8
        cpx #5
        bne +
        clc
        adc #8
        rts
+       lsr                     ; frames / 16
        sta tmp
        lda #48
        sec
        sbc tmp
        bcs +
        lda #0
+       rts

; per section (index 0 unused): grid speed in 1/16 phase per frame ($ff:
; ramp), flash frames on the kick
secspeed .byte 16, 12, 32, 56, 10, $ff, 64, $ff
secflash .byte 0, 0, 0, 4, 0, 0, 4, 0

; --- the grid: horizontal lines are kd021 values -------------------------
ground  lda gspeed              ; phase += speed / 16
        asl
        asl
        asl
        asl
        clc
        adc gsub
        sta gsub
        lda gspeed
        lsr
        lsr
        lsr
        lsr
        adc gphase
        and #63
        sta gphase
        ldy nold                ; erase the old lines
        beq +
-       ldx oldidx-1,y
        lda gndbg-104,x
        sta kd021,x
        dey
        bne -
+       ldx gphase
        lda gline_lo,x
        sta ptr
        lda gline_hi,x
        sta ptr+1
        ldy #0
        lda flash
        beq gr_plain
        dec flash
-       lda (ptr),y
        cmp #$ff
        beq gr_done
        tax
        lda flashcol-104,x
        sta kd021,x
        txa
        sta oldidx,y
        iny
        bne -
gr_plain
-       lda (ptr),y
        cmp #$ff
        beq gr_done
        tax
        lda linecol-104,x
        sta kd021,x
        txa
        sta oldidx,y
        iny
        bne -
gr_done sty nold
        ; the near lines: events for the interrupts
        ldx gphase
        lda gnear_lo,x
        sta ptr
        lda gnear_hi,x
        sta ptr+1
        ldx evbuild             ; event index
        lda #0
        sta tmp2                ; 1: the $d011 event is in
        ldy #0
-       lda (ptr),y
        cmp #$ff
        beq gr_end
        sta tmp
        cmp #250
        bcc +
        jsr ev13
+       lda tmp                 ; on: the line's colour
        sta evline,x
        ldy tmp
        lda flash
        beq +
        lda flashcol-155,y
        bne ++
+       lda linecol-155,y
+       sta evval,x
        lda #0
        sta evreg,x
        inx
        lda ptr                 ; next pair (y advances by 2)
        clc
        adc #1
        sta ptr
        bcc +
        inc ptr+1
+       ldy #0
        lda tmp                 ; off: back to black
        clc
        adc (ptr),y
        sta tmp
        cmp #250
        bcc +
        jsr ev13
+       lda tmp
        sta evline,x
        lda #0
        sta evval,x
        sta evreg,x
        inx
        inc ptr
        bne -
        inc ptr+1
        bne -
gr_end  jsr ev13
        lda #253                ; 25 rows again after line 251 (the border stays open)
        sta evline,x
        lda #$9b                ; (bit 8: the interrupt at LOGO_IRQ)
        sta evval,x
        lda #1
        sta evreg,x
        inx
        txa
        sec
        sbc evbuild
        sta evcount
        rts

; the 24 rows event (on line 248: after the border check of line 247)
ev13    lda tmp2
        bne +
        inc tmp2
        lda #249
        sta evline,x
        lda #$13
        sta evval,x
        lda #1
        sta evreg,x
        inx
+       rts

; --- the sun: gradient of the fade level, then the stripes ---------------
sun     lda gspeed              ; the stripes scroll down slowly, with the grid
        clc
        adc ssub
        sta ssub
        lda sphase
        adc #0
        and #15
        sta sphase
        ldx fade                ; ptr = gradient - (SUN_TOP - 51): indexed like kd023
        lda sungrad_lo,x
        sec
        sbc #SUN_TOP-51
        sta ptr
        lda sungrad_hi,x
        sbc #0
        sta ptr+1
        cpx sunfade
        beq su_gaps
        stx sunfade             ; a new fade level: the whole gradient
        ldy #SUN_TOP-51+SUN_LINES-1
-       lda (ptr),y
        sta kd023,y
        dey
        cpy #SUN_TOP-51
        bcs -
        bcc su_new
su_gaps lda sphase              ; same level: only when the stripes moved
        cmp sunphase
        beq su_done
        ldx sunphase            ; put the old gaps back
        lda stripe_lo,x
        sta ptr2
        lda stripe_hi,x
        sta ptr2+1
        ldx #0
-       txa
        tay
        lda (ptr2),y
        cmp #$ff
        beq su_new
        tay
        lda (ptr),y
        sta kd023,y
        inx
        bne -
su_new  ldx sphase              ; the new gaps: the sky shows through
        stx sunphase
        lda stripe_lo,x
        sta ptr2
        lda stripe_hi,x
        sta ptr2+1
        ldy #0
-       lda (ptr2),y
        cmp #$ff
        beq su_done
        tax
        lda kd021,x
        sta kd023,x
        iny
        bne -
su_done rts

; --- the buildings: a spectrum analyser of the three voices --------------
meters  ldx #BARS-1             ; fall, down to the building's own height
-       lda barh,x
        sec
        sbc #2
        bcc +
        cmp barmin,x
        bcs ++
+       lda barmin,x
+       sta barh,x
        dex
        bpl -
        ldx #0
        jsr meter
        ldx #7
        jsr meter
        ldx #14
meter   ldy vidx,x
        sty tmp2
        lda wave,x
        and gmask,x
        lsr
        bcc m_off
        lda newnote,y           ; a new note: near the top
        beq +
        lda #BAR_MAX-6
        sta lvl,y
        bne m_show
+       lda lvl,y               ; held: down to a sustain level
        cmp #SUSTAIN+2
        bcs +
        lda #SUSTAIN+2
+       sbc #2                  ; (carry set)
        sta lvl,y
        jmp m_show
m_off   lda lvl,y
        sec
        sbc #4
        bcs +
        lda #0
+       sta lvl,y
        beq m_done
m_show  lda wave,x              ; noise: the drum's bar
        bpl +
        ldy inst,x
        lda i_vis,y
        tay
        lda noisebar,y
        jmp ++
+       ldy vfhi,x
        lda fbar,y
+       tay
        ldx tmp2
        lda lvl,x
        cmp barh,y
        bcc +
        sta barh,y
+       sec                     ; the neighbours, lower
        sbc #18
        bcc m_done
        sta tmp
        cpy #0
        beq +
        cmp barh-1,y
        bcc +
        sta barh-1,y
+       cpy #BARS-1
        beq m_done
        lda tmp
        cmp barh+1,y
        bcc m_done
        sta barh+1,y
m_done  rts

; --- the scroller: 8 sprites, X-expanded, moving left --------------------
scroller
        dec sbase
        dec sbase
        lda sbase
        cmp #-48
        bne +
        lda #0
        sta sbase
        ldx sfirst
        jsr render
        lda sfirst
        clc
        adc #1
        and #7
        sta sfirst
+       lda sbase               ; x0 = base + 24 (-22..24)
        clc
        adc #24
        sta tmp
        ldx sfirst              ; the leftmost sprite
        ldy #0                  ; high byte of x0
        lda tmp
        bpl +
        dey
+       sty sbasehi
        lda tmp
        bpl +
        clc                     ; negative: 504 + x0 = 256 + (248 + x0)
        adc #248
        sta scr_x,x
        lda bitmask,x
        bne ++
+       sta scr_x,x
        lda #0
+       sta scr_msb
        ldy #1                  ; the others: x0 + 48 * k, 26..360
-       txa
        clc
        adc #1
        and #7
        tax
        lda tmp
        clc
        adc t48lo,y
        sta scr_x,x
        lda t48hi,y
        adc sbasehi
        beq +
        lda bitmask,x
        ora scr_msb
        sta scr_msb
+       iny
        cpy #8
        bne -
        ldx #7                  ; the wave and the colours
-       lda scr_x,x
        lsr
        lsr
        clc
        adc fcount
        adc fcount
        tay
        lda wave8,y
        clc
        adc #SCROLL_Y
        sta scr_y,x
        dex
        bpl -
        lda fcount              ; the colours: a rainbow that moves every 8 frames
        and #7
        bne +
        ldx #7
-       txa
        sec
        sbc sfirst
        sta tmp2
        lda fcount
        lsr
        lsr
        lsr
        clc
        adc tmp2
        and #7
        tay
        lda rainbow,y
        sta scr_col,x
        dex
        bpl -
+       rts

; renders the next 3 characters of the text into the sprite of slot X
render  txa
        asl
        asl
        asl
        asl
        asl
        asl                     ; X * 64
        clc
        adc #<SCROLLSPR
        sta ptr2
        lda #>SCROLLSPR
        adc #0
        sta ptr2+1
        cpx #4                  ; slots 4-7 are 256 bytes further
        bcc +
        inc ptr2+1
+       lda #0
        sta tmp2                ; column 0-2
r_char  ldy #0
        lda (tptr),y
        cmp #$ff
        bne +
        lda #<scrolltext
        sta tptr
        lda #>scrolltext
        sta tptr+1
        lda (tptr),y
+       inc tptr
        bne +
        inc tptr+1
+       sta ptr                 ; font address = FONT + code * 8
        lda #0
        asl ptr
        rol
        asl ptr
        rol
        asl ptr
        rol
        adc #>FONT
        sta ptr+1
        ldy #0
        ldx tmp2
-       lda (ptr),y
        sty tmp
        pha
        tya                     ; sprite byte = row * 6 + column: two rows per font row
        asl
        adc tmp
        asl
        adc tmp2
        tay
        pla
        sta (ptr2),y
        iny
        iny
        iny
        sta (ptr2),y
        ldy tmp
        iny
        cpy #8
        bne -
        inc tmp2
        lda tmp2
        cmp #3
        bne r_char
        rts

; --- the logo: a wave, letters jump on the lead's notes ------------------
logo    lda newnote+1           ; the lead (voice 2): the letter of its note jumps
        beq ++
        lda note+7
-       cmp #7
        bcc +
        sbc #7
        bcs -
+       tax
        lda #6
        sta bump,x
+       lda fcount              ; half of the letters per frame
        and #1
        tax
-       txa                     ; y = top + wave (0-6) - jump (0-6), at least 15
        asl
        asl
        asl
        asl
        asl
        sta tmp
        lda fcount
        asl
        asl
        clc
        adc tmp
        tay
        lda wave6,y
        clc
        adc #LOGO_Y
        sec
        sbc bump,x
        cmp #15
        bcs +
        lda #15
+       sta logo_y,x
        lda bump,x
        beq +
        dec bump,x
        lda #1                  ; white while it jumps
        bne ++
+       lda logocol,x
+       sta logo_col,x
        inx
        inx
        cpx #7
        bcc -
        rts

; --- the stars twinkle -----------------------------------------------------
stars   lda fcount
        and #3
        bne st_done
        lda rnd                 ; 8-bit LFSR
        asl
        bcc +
        eor #$1d
+       sta rnd
        and #31
        cmp #STARS
        bcs st_done
        tax
        lda starlo,x
        sta ptr
        lda starhi,x
        sta ptr+1
        ldy #0
        lda #STAR_DIM
        cmp (ptr),y
        bne +
        lda #STAR_BRIGHT
+       sta (ptr),y
st_done rts

; --- tables ------------------------------------------------------------------
t48lo   .byte <0, <48, <96, <144, <192, <240, <288, <336
t48hi   .byte >0, >48, >96, >144, >192, >240, >288, >336
bitmask .byte 1, 2, 4, 8, 16, 32, 64, 128
vidx    .byte 0, 0, 0, 0, 0, 0, 0, 1, 0, 0, 0, 0, 0, 0, 2
rainbow .byte 3, 14, 4, 10, 7, 10, 4, 14
logocol .byte 3, 3, 14, 3, 3, 14, 3
logo_x  .byte <(30+44*0), <(30+44*1), <(30+44*2), <(30+44*3), <(30+44*4), <(30+44*5), <(30+44*6)
logo_msb .byte %01000000        ; the last letter is at x = 294
; noise bar per instrument kind (vis): kick low, snare middle, hi-hat high
noisebar .byte 0, 0, 6, 13, 0, 0, 0
; building heights at rest
barmin  .byte 14, 22, 10, 30, 18, 8, 12, 12, 8, 20, 34, 16, 26, 12

        .include "player.asm"
        .include "gfx_code.asm"

; --- variables -----------------------------------------------------------------
scr_x   .fill 8
scr_y   .fill 8
scr_col .fill 8
scr_msb .byte 0
logo_y  .fill 7
logo_col .fill 7
oldidx  .fill 32
evline  .fill 16                ; events below the kernel, two lists of 8 (one is built
evval   .fill 16                ; while the other runs): line, value, register
evreg   .fill 16                ; 0: $d021, 1: $d011
evcount .byte 0                 ; events in the list built
evbuild .byte 0                 ; 0 or 8: the list being built
evpos   .byte 0
evend   .byte 0
barh    .fill BARS
bard    .fill BARS
        .align 256
kd021   .fill KLINES            ; $d021 for the kernel's lines (51-192)
        .align 256
kd023   .fill 104               ; $d023 for lines 51-154
zpsave  .fill 256
        .cerror * > SCREEN, "code over the screen"

; --- the VIC bank -------------------------------------------------------------
        * = SCREEN
        .include "gfx_screen.asm"
        * = SPRITES
        .include "gfx_sprites.asm"
        * = SKYCH
        .include "gfx_sky.asm"
        * = GNDCH
        .include "gfx_ground.asm"

; --- above the VIC data ----------------------------------------------------------
        * = FONT + $200
        .include "music.asm"
        .include "gfx_data.asm"
        .cerror * > $7fff, "data over $7fff"
