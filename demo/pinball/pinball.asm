; STARFALL - a pinball table for the Commodore 64, for 64tass (without -a)
;
; One screen, multicolour bitmap. The ball and the flippers are sprites.
; The physics runs 4 substeps per frame in fixed point: positions in 1/256
; pixel, speeds in 1/256 pixel per substep. The walls come from two maps
; made by gen_data.py from the same shapes as the picture: for every 2x2
; pixel cell near a wall, the direction of the wall normal (64 steps) and
; its material, and the penetration of a ball centred in the cell (1/16
; pixel). Inside a cell the wall is a plane, so the ball is pushed out
; exactly and rolls smoothly along curves. The flippers are computed
; analytically: the ball is rotated into the flipper's frame, where the
; flipper is a tapered bar with a round tip, and a moving flipper adds the
; speed of its surface at the point of contact.
;
; Data (maps, picture, lamps, sprites, font, sound) come from gen_data.py.

        .include "consts.asm"

; --- memory -----------------------------------------------------------------
MAPA    = $7000                 ; collision maps (after the init data is used)
MAPD    = MAPA + MAP_SIZE
VARS    = $b800                 ; variables
SCREEN  = $c000                 ; VIC bank 3
SPRMEM  = $c400
BITMAP  = $e000
COLRAM  = $d800
SPRBLK  = (SPRMEM & $3fff) / 64

BLK_SHINE   = SPRBLK
BLK_BODY    = SPRBLK + 1
BLK_PLUNGER = SPRBLK + 2
BLK_LFLIP   = SPRBLK + 4        ; 11 frames x 2 sprites
BLK_RFLIP   = SPRBLK + 26

; --- physics ----------------------------------------------------------------
SUBSTEPS = 4
GRAV    = 2                     ; 1/256 px per substep^2
VMAX    = 600                   ; speed limit per axis
E_WALL  = 45                    ; restitution x 128
E_RUBBER = 82
E_BUMP  = 40
E_SLING = 40
E_STAR  = 30
E_DROP  = 25
E_GATE  = 16
E_FLIP  = 36
SLIDE   = 44                    ; slower impacts do not bounce
KICK_BUMP = 290
KICK_SLING = 250
LAUNCH0 = 240                   ; plunger speed: LAUNCH0 + pull * LAUNCHK
LAUNCHK = 22
PULLMAX = 16

; --- game -------------------------------------------------------------------
ST_ATTRACT = 0
ST_PLUNGER = 1
ST_PLAY    = 2
ST_SAUCER  = 3
ST_DRAIN   = 4
ST_OVER    = 5

BALLS   = 3
SAVETIME = 400                  ; ball save: 8 s after the ball enters play

; input bits
K_LEFT  = 1
K_RIGHT = 2
K_LAUNCH = 4
K_NUDGE = 8
K_START = 16
K_PAUSE = 32
K_FIRE  = 64

; points (index into the points tables)
P10     = 0
P50     = 1
P100    = 2
P250    = 3
P500    = 4
P1000   = 5
P2500   = 6
P5000   = 7
P10000  = 8

; panel
PX      = PANEL_C0 + 1          ; first text column of the panel
ROW_SCORE = 5
ROW_BALL = 9
ROW_HIGH = 11
ROW_MSG = 15

; --- zero page --------------------------------------------------------------
        * = $02
frameflag .byte ?
frame   .byte ?
bxl     .byte ?                 ; ball x, 8.8 pixels
bxh     .byte ?
byl     .byte ?                 ; ball y
byh     .byte ?
vxl     .byte ?                 ; ball speed, 1/256 px per substep
vxh     .byte ?
vyl     .byte ?
vyh     .byte ?
ptr     .word ?
ptr2    .word ?
sptr    .word ?                 ; string pointer
m16l    .byte ?                 ; multiply
m16h    .byte ?
m8      .byte ?
msign   .byte ?
ma      .byte ?
mb      .byte ?
ql      .byte ?
qh      .byte ?
r0      .byte ?
r1      .byte ?
r2      .byte ?
tml     .byte ?
tmh     .byte ?
plo     .byte ?
phi     .byte ?
cang    .byte ?                 ; contact: normal angle (256 steps)
cnx     .byte ?                 ; normal, x 127
cny     .byte ?
pen     .byte ?                 ; penetration, 1/16 px
emat    .byte ?                 ; restitution x 128
kickl   .byte ?                 ; extra kick along the normal
kickh   .byte ?
surfl   .byte ?                 ; surface speed along the normal
surfh   .byte ?
vnl     .byte ?
vnh     .byte ?
jl      .byte ?
jh      .byte ?
t0      .byte ?
t1      .byte ?
t2      .byte ?
t3      .byte ?
dxl     .byte ?                 ; flipper: ball relative to the pivot, 1/16 px
dxh     .byte ?
dyl     .byte ?
dyh     .byte ?
all     .byte ?                 ; along the flipper
alh     .byte ?
prl     .byte ?                 ; across (positive above)
prh     .byte ?
fang    .byte ?                 ; flipper angle
fw      .byte ?                 ; flipper angle step this substep
fc      .byte ?
fs      .byte ?
frr     .byte ?
mirror  .byte ?
cellA   .byte ?
cellD   .byte ?
pcol    .byte ?                 ; text output
prow    .byte ?
pcolor  .byte ?
iptr    .word ?                 ; interrupt: pointer and temporary
it0     .byte ?

; --- variables --------------------------------------------------------------
        .virtual VARS
state   .byte ?
input   .byte ?
pressed .byte ?
previn  .byte ?
paused  .byte ?
drained .byte ?
substep .byte ?
fpos    .fill 2                 ; flipper position: 0 = up, 40 = rest
ftgt    .fill 2                 ; 0 = up, 1 = rest
fwv     .fill 2                 ; last step
pull    .byte ?                 ; plunger pulled back (pixels)
pullvis .byte ?                 ; plunger drawn
ballvis .byte ?
score   .fill 4                 ; BCD, lowest digits first
hiscore .fill 4
shown   .fill 8                 ; digits on the panel ($ff = redraw)
hishown .byte ?
ball    .byte ?
bonus   .byte ?                 ; x 1000
mult    .byte ?
again   .byte ?                 ; shoot again
extralit .byte ?
lanes   .byte ?                 ; lit top lanes, bits 0-2
stars   .byte ?                 ; lit S-T-A-R, bits 0-3
tgt_up  .fill 3
tgtdown .byte ?                 ; number of targets down
tgtreset .byte ?                ; frames before the bank resets
starreset .byte ?
lanereset .byte ?
skill   .byte ?                 ; skill shot lane, $ff = none
entered .byte ?                 ; the ball left the plunger lane
saved   .byte ?                 ; ball save used
ballsave .word ?
saucert .byte ?
saucerign .byte ?
tilted  .byte ?
tiltmeter .byte ?
zone    .byte ?
lastzone .byte ?
ev_bump .byte ?
ev_sling .byte ?
ev_star .byte ?
ev_drop .byte ?
bumpcool .fill 3
slingcool .fill 2
starcool .fill 4
statet  .byte ?                 ; state timer
drainph .byte ?
msgt    .byte ?                 ; frames before the message goes away
lampst  .fill NLAMPS            ; 0 off, 1 on, 2 slow blink, 3 fast blink
lampfl  .fill NLAMPS            ; flash frames
lampshown .fill NLAMPS
impact  .byte ?
musicon .byte ?
mptr    .fill 6
mdur    .fill 3
sfxlo_  .fill 3
sfxhi_  .fill 3
sfxon   .fill 3
sfxpri  .fill 3
sfxctl  .fill 3
rnd     .byte ?
overruns .byte ?                ; frames that took longer than a frame
stuck   .byte ?                 ; frames with the ball (almost) still
msgptr  .word ?                 ; message being drawn
msgidx  .byte ?                 ; next character (24 = done)
t1digits .byte ?
scoredirty .byte ?
attract_t .byte ?
        .endv

; --- BASIC stub -------------------------------------------------------------
        * = $0801
        .word (+), 2026
        .null $9e, format("%d", start)
+       .word 0

; ============================================================================
; start
; ============================================================================
start   sei
        cld
        ldx #$ff
        txs
        lda #$35                ; RAM, I/O, no BASIC or KERNAL
        sta $01
        lda #$7f
        sta $dc0d
        sta $dd0d
        lda $dc0d
        lda $dd0d
        lda #<nmi
        sta $fffa
        lda #>nmi
        sta $fffb
        lda #0
        sta $d011               ; screen off while copying
        sta $d015
        sta $d020

        ; picture, colours and sprites into VIC bank 3
        lda #<src_bitmap
        ldx #>src_bitmap
        ldy #>BITMAP
        jsr setcopy
        ldx #(>8000)+1
        jsr copypages
        lda #<src_screen
        ldx #>src_screen
        ldy #>SCREEN
        jsr setcopy
        ldx #4
        jsr copypages
        lda #<src_colram
        ldx #>src_colram
        ldy #>COLRAM
        jsr setcopy
        ldx #4
        jsr copypages
        lda #<src_sprites
        ldx #>src_sprites
        ldy #>SPRMEM
        jsr setcopy
        ldx #>SPR_BYTES
        jsr copypages

        ; the collision maps over the init data
        lda #<mapa_rle
        ldx #>mapa_rle
        ldy #>MAPA
        jsr setcopy
        lda #<MAPA+MAP_SIZE
        ldx #>MAPA+MAP_SIZE
        jsr unrle
        lda #<mapd_rle
        ldx #>mapd_rle
        jsr setsrc
        lda #<MAPD
        sta ptr2
        lda #>MAPD
        sta ptr2+1
        lda #<MAPD+MAP_SIZE
        ldx #>MAPD+MAP_SIZE
        jsr unrle

        ; variables
        lda #0
        tax
-       sta VARS,x
        sta VARS+$100,x
        inx
        bne -
        lda #$ff
        sta lastzone
        lda #$00
        sta hiscore
        lda #$00
        sta hiscore+1           ; high score 50,000 to beat
        lda #$05
        sta hiscore+2
        lda #0
        sta hiscore+3
        lda #$5a
        sta rnd
        lda #1
        sta scoredirty          ; show the score (0) from the start

        jsr vic_init
        jsr sid_init
        jsr panel_init
        jsr targets_all_up
        lda #40
        sta fpos
        sta fpos+1
        lda #1
        sta ftgt
        sta ftgt+1
        jsr to_attract

        ; raster interrupt at line 250
        lda #<irq
        sta $fffe
        lda #>irq
        sta $ffff
        lda #$3b
        sta $d011
        lda #250
        sta $d012
        lda #1
        sta $d01a
        sta $d019
        cli

; ============================================================================
; main loop: one pass per frame, started by the interrupt at line 250
; ============================================================================
main    lda frameflag
        beq main
        lda #0
        sta frameflag
        jsr update_sprites
        jsr update_lamps
        jsr read_input
        lda pressed
        and #K_PAUSE
        beq _run
        lda paused
        eor #1
        sta paused
        jsr volume
        ldx #<txt_paused
        ldy #>txt_paused
        lda paused
        bne _msg
        ldx #<txt_blank
        ldy #>txt_blank
_msg    jsr message
        jsr msg_job             ; (the game is stopped: draw it all now)
        jsr msg_job
        jsr msg_job
        jsr msg_job
_run    lda paused
        bne main
        jsr game_frame
        jsr draw_score
        jsr msg_job
        jsr random
        lda frameflag           ; the next frame began already?
        beq main
        inc overruns
        jmp main

nmi     rti

irq     pha
        txa
        pha
        tya
        pha
        lda #$ff
        sta $d019
        lda #1
        sta frameflag
        inc frame
        lda paused
        bne +
        jsr sound_frame
+       pla
        tay
        pla
        tax
        pla
        rti

; ============================================================================
; setup
; ============================================================================

; copy source A/X to page Y (ptr -> ptr2)
setcopy sty ptr2+1
        ldy #0
        sty ptr2
setsrc  sta ptr
        stx ptr+1
        rts

; copy X pages from ptr to ptr2
copypages
        ldy #0
-       lda (ptr),y
        sta (ptr2),y
        iny
        bne -
        inc ptr+1
        inc ptr2+1
        dex
        bne -
        rts

; unpack the RLE stream at ptr into ptr2 until ptr2 reaches A/X
unrle   sta t0
        stx t1
_loop   ldy #0
        lda (ptr),y
        jsr _inc
        cmp #$80
        bcs _run
        tax
        inx
_lit    lda (ptr),y
        sta (ptr2),y
        jsr _inc
        jsr _incd
        dex
        bne _lit
        beq _chk
_run    sbc #$7d
        tax
        lda (ptr),y
        jsr _inc
_rep    sta (ptr2),y
        jsr _incd
        dex
        bne _rep
_chk    lda ptr2
        cmp t0
        lda ptr2+1
        sbc t1
        bcc _loop
        rts
_inc    inc ptr
        bne +
        inc ptr+1
+       rts
_incd   inc ptr2
        bne +
        inc ptr2+1
+       rts

vic_init
        lda $dd02
        ora #3
        sta $dd02
        lda $dd00
        and #$fc                ; bank 3: $c000-$ffff
        sta $dd00
        lda #$08                ; screen $c000, bitmap $e000
        sta $d018
        lda #$18                ; multicolour, 40 columns
        sta $d016
        lda #0
        sta $d020
        lda #BG
        sta $d021
        lda #%01111111
        sta $d015
        lda #%01111100          ; flippers and plunger are multicolour
        sta $d01c
        lda #0
        sta $d017
        sta $d01d
        sta $d01b
        sta $d010
        lda #2                  ; MC1: red rubber
        sta $d025
        lda #1                  ; MC2: white
        sta $d026
        lda #1
        sta $d027               ; ball shine
        lda #15
        sta $d028               ; ball
        lda #12
        sta $d029
        sta $d02a
        sta $d02b
        sta $d02c
        lda #11
        sta $d02d               ; plunger spring
        lda #BLK_SHINE
        sta SCREEN+$3f8
        lda #BLK_BODY
        sta SCREEN+$3f9
        lda #BLK_PLUNGER
        sta SCREEN+$3fe
        lda #LANE_CX-6+24
        sta $d00c
        rts

; ============================================================================
; input
; ============================================================================
read_input
        lda #$ff
        sta $dc02
        lda #0
        sta $dc03
        lda #$ff
        sta $dc00
        lda $dc00               ; joystick 2
        eor #$ff
        and #$1f
        sta t0
        lda #0
        sta t1
        lda t0
        and #4                  ; left
        beq +
        lda #K_LEFT
        sta t1
+       lda t0
        and #8                  ; right
        beq +
        lda t1
        ora #K_RIGHT
        sta t1
+       lda t0
        and #2                  ; down
        beq +
        lda t1
        ora #K_LAUNCH
        sta t1
+       lda t0
        and #1                  ; up
        beq +
        lda t1
        ora #K_NUDGE
        sta t1
+       lda t0
        and #$10                ; fire
        beq +
        lda t1
        ora #K_FIRE|K_START
        sta t1
+
        ; keyboard: left SHIFT, Z
        lda #%11111101
        sta $dc00
        lda $dc01
        and #$90
        cmp #$90
        beq +
        lda t1
        ora #K_LEFT
        sta t1
+       ; right SHIFT, /
        lda #%10111111
        sta $dc00
        lda $dc01
        and #$90
        cmp #$90
        beq +
        lda t1
        ora #K_RIGHT
        sta t1
+       ; SPACE
        lda #%01111111
        sta $dc00
        lda $dc01
        and #$10
        bne +
        lda t1
        ora #K_FIRE|K_START
        sta t1
+       ; F1 starts, RETURN launches
        lda #%11111110
        sta $dc00
        lda $dc01
        tax
        and #$10
        bne +
        lda t1
        ora #K_START
        sta t1
+       txa
        and #$02
        bne +
        lda t1
        ora #K_LAUNCH
        sta t1
+       ; P pauses
        lda #%11011111
        sta $dc00
        lda $dc01
        and #$02
        bne +
        lda t1
        ora #K_PAUSE
        sta t1
+       ; N nudges
        lda #%11101111
        sta $dc00
        lda $dc01
        and #$80
        bne +
        lda t1
        ora #K_NUDGE
        sta t1
+       lda #$ff
        sta $dc00
        lda previn
        eor #$ff
        and t1
        sta pressed
        lda t1
        sta input
        sta previn
        rts

; ============================================================================
; game states
; ============================================================================
game_frame
        lda state
        asl
        tax
        lda states,x
        sta ptr
        lda states+1,x
        sta ptr+1
        jmp (ptr)

states  .word st_attract, st_plunger, st_play, st_saucer, st_drain, st_over

; --- attract mode ------------------------------------------------------------
to_attract
        lda #ST_ATTRACT
        sta state
        lda #0
        sta ballvis
        lda #1
        sta ftgt
        sta ftgt+1
        jsr music_start
        ldx #<txt_press
        ldy #>txt_press
        jsr message
        lda #0
        sta msgt
        rts

st_attract
        jsr flippers_frame
        ; lamps chase round the table
        lda frame
        lsr
        lsr
        sta t0
        ldx #NLAMPS-1
-       txa
        asl
        adc t0
        and #7
        cmp #2
        lda #0
        rol
        eor #1
        sta lampst,x
        dex
        bpl -
        ; PRESS FIRE blinks, now and then the credits
        lda frame
        and #31
        bne +
        inc attract_t
        lda attract_t
        and #$0c
        cmp #$08
        bne _blink
        lda attract_t
        and #3
        bne +
        ldx #<txt_credit
        ldy #>txt_credit
        jsr message
        jmp +
_blink  lda attract_t
        and #1
        beq _on
        ldx #<txt_blank
        ldy #>txt_blank
        jsr message
        jmp +
_on     ldx #<txt_press
        ldy #>txt_press
        jsr message
+       lda #0
        sta msgt
        lda pressed
        and #K_START
        beq +
        jmp new_game
+       rts

new_game
        jsr music_stop
        lda #0
        sta score
        sta score+1
        sta score+2
        sta score+3
        sta again
        lda #1
        sta scoredirty
        lda #0
        sta extralit
        sta lanes
        sta stars
        ldx #NLAMPS-1
-       sta lampst,x
        sta lampfl,x
        dex
        bpl -
        lda #1
        sta ball
        jsr targets_all_up
        lda #SFX_START
        ldx #2
        jsr sfx
        ldx #<txt_blank
        ldy #>txt_blank
        jsr message
        ; fall into new_ball

new_ball
        lda #0
        sta bonus
        sta tilted
        sta tiltmeter
        sta entered
        sta saved
        sta ballsave
        sta ballsave+1
        sta pull
        sta ev_bump
        sta ev_sling
        sta ev_star
        sta ev_drop
        lda #1
        sta mult
        jsr show_mult
        jsr show_ball
        lda #ST_PLUNGER
        sta state
        lda #1
        sta ballvis
        ; skill shot: one top lane blinks
        jsr random
        and #3
        cmp #3
        bcc +
        lda #1
+       sta skill
        jsr lane_lamps
        lda #0
        sta lampst+L_SAVE
        rts

; --- ball on the plunger -----------------------------------------------------
st_plunger
        jsr flip_input
        jsr flippers_frame
        lda input
        and #K_LAUNCH|K_FIRE
        beq _release
        ldx pull                ; start pulling only on a new press
        bne +
        lda pressed
        and #K_LAUNCH|K_FIRE
        beq _place
        inc pull
        bne _place
+       lda frame
        and #1
        bne _place
        lda pull
        cmp #PULLMAX
        bcs _place
        inc pull
        and #3
        bne _place
        lda #SFX_PULL
        ldx #0
        jsr sfx
        jmp _place
_release
        lda pull
        beq _place
        ; launch
        ldy #LAUNCHK
        jsr umul8               ; pull * LAUNCHK
        clc
        lda ql
        adc #<LAUNCH0
        sta ql
        lda qh
        adc #>LAUNCH0
        sta qh
        lda #0
        sec
        sbc ql
        sta vyl
        lda #0
        sbc qh
        sta vyh
        lda #0
        sta vxl
        sta vxh
        sta pull
        sta drained
        lda #ST_PLAY
        sta state
        lda #SFX_LAUNCH
        ldx #0
        jsr sfx
        rts
_place  lda #LANE_CX
        sta bxh
        lda #$80
        sta bxl
        lda pull                ; the plunger moves half a pixel per step
        lsr
        sta pullvis
        lda #0
        ror
        sta byl
        lda pullvis
        clc
        adc #PLUNGER_Y
        sta byh
        rts

; --- ball in play --------------------------------------------------------------
st_play jsr flip_input
        jsr nudge
        jsr physics
        lda drained
        beq +
        jmp ball_drained
+       lda impact
        cmp #2
        bcc +
        lda #SFX_THUD
        ldx #1
        jsr sfx
+       lda #0
        sta impact
        jsr unstick
        jsr events
        jsr zones
        jsr timers
        ; plunger returns
        lda pullvis
        beq +
        sec
        sbc #4
        bcs ++
        lda #0
+
+       sta pullvis
        ; back on the plunger?
        lda entered
        bne _in
        lda bxh
        cmp #LANE_X0+3
        bcc _enter
        lda byh
        cmp #PLUNGER_Y
        bcc _r
        lda vyh
        bmi _r
        lda #0
        sta pull
        sta vxl
        sta vxh
        sta vyl
        sta vyh
        lda #ST_PLUNGER
        sta state
_r      rts
_enter  lda byh
        cmp #120
        bcs _r
        lda #1
        sta entered
        lda saved
        bne _r
        lda #<SAVETIME
        sta ballsave
        lda #>SAVETIME
        sta ballsave+1
        lda #3
        sta lampst+L_SAVE
        rts
_in     rts

; --- ball in the saucer ------------------------------------------------------
st_saucer
        jsr flip_input
        jsr flippers_frame
        jsr timers
        dec statet
        bne +
        ; eject towards the lower left
        lda #<-280
        sta vxl
        lda #>-280
        sta vxh
        lda #<-40
        sta vyl
        lda #>-40
        sta vyh
        lda #30
        sta saucerign
        lda #ST_PLAY
        sta state
        lda #SFX_EJECT
        ldx #2
        jsr sfx
+       rts

; --- ball lost: bonus count ----------------------------------------------------
ball_drained
        lda #0
        sta ballvis
        sta lampst+L_SAVE
        lda ballsave
        ora ballsave+1
        beq _lost
        lda tilted
        bne _lost
        ; ball save
        lda #1
        sta saved
        lda #0
        sta entered
        sta pull
        sta ballsave
        sta ballsave+1
        lda #1
        sta ballvis
        lda #ST_PLUNGER
        sta state
        ldx #<txt_saved
        ldy #>txt_saved
        jsr message
        lda #SFX_EXTRA
        ldx #2
        jsr sfx
        rts
_lost   lda #ST_DRAIN
        sta state
        lda #1
        sta ftgt
        sta ftgt+1
        lda #0
        sta drainph
        lda #50
        sta statet
        lda tilted
        bne +
        lda #SFX_DRAIN
        ldx #2
        jsr sfx
+       rts

st_drain
        jsr flippers_frame
        jsr timers
        lda drainph
        bne _count
        dec statet
        bne _r
        inc drainph
        lda tilted
        beq +
        lda #0
        sta bonus
+       jsr show_bonus
        lda #6
        sta statet
_r      rts
_count  cmp #2
        beq _wait
        dec statet
        bne _r
        lda #5
        sta statet
        lda bonus
        beq _done
        dec bonus
        ldx mult
-       txa
        pha
        ldx #P1000
        jsr points
        pla
        tax
        dex
        bne -
        jsr show_bonus
        lda #SFX_TICK
        ldx #1
        jsr sfx
        rts
_done   lda #2
        sta drainph
        lda #60
        sta statet
        rts
_wait   dec statet
        bne _r
        lda again
        beq _next
        lda #0
        sta again
        lda #0
        sta lampst+L_AGAIN
        ldx #<txt_again
        ldy #>txt_again
        jsr message
        jmp new_ball
_next   inc ball
        lda ball
        cmp #BALLS+1
        bcs game_over
        ldx #<txt_blank
        ldy #>txt_blank
        jsr message
        jmp new_ball

game_over
        lda #ST_OVER
        sta state
        lda #250
        sta statet
        lda #SFX_OVER
        ldx #2
        jsr sfx
        ; new high score?
        ldx #3
-       lda score,x
        cmp hiscore,x
        bcc _no
        bne _yes
        dex
        bpl -
        bmi _no
_yes    ldx #3
-       lda score,x
        sta hiscore,x
        dex
        bpl -
        jsr draw_hiscore
        ldx #<txt_newhigh
        ldy #>txt_newhigh
        jsr message
        lda #250
        sta msgt
        rts
_no     ldx #<txt_over
        ldy #>txt_over
        jsr message
        lda #250
        sta msgt
        rts

st_over jsr flippers_frame
        jsr timers
        dec statet
        bne +
        jmp to_attract
+       rts

; ============================================================================
; flippers
; ============================================================================

; targets from the input (dead when tilted)
flip_input
        lda tilted
        bne _rest
        ; lane change on a new press
        lda state
        cmp #ST_PLAY
        bne _nolc
        lda pressed
        and #K_LEFT
        beq +
        jsr lanes_left
+       lda pressed
        and #K_RIGHT
        beq _nolc
        jsr lanes_right
_nolc   ldx #0
        lda #K_LEFT
        jsr _one
        ldx #1
        lda #K_RIGHT
_one    sta t0
        lda state
        cmp #ST_PLUNGER
        beq +
        lda t0
        ora #K_FIRE             ; fire raises both flippers
        sta t0
+       lda input
        and t0
        beq _down
        lda ftgt,x
        beq +
        stx t3
        lda #SFX_FLIP
        ldx #0
        jsr sfx
        ldx t3
+       lda #0
        sta ftgt,x
        rts
_down   lda #1
        sta ftgt,x
        rts
_rest   lda #1
        sta ftgt
        sta ftgt+1
        rts

; move both flippers one substep: up 4 steps, down 2 steps (256 per turn)
flippers_step
        ldx #1
_l      lda fpos,x
        ldy ftgt,x
        bne _fall
        sec
        sbc #4
        bcs _set
        lda #0
        beq _set
_fall   clc
        adc #2
        cmp #41
        bcc _set
        lda #40
_set    tay
        sec
        sbc fpos,x
        sta fwv,x
        tya
        sta fpos,x
        dex
        bpl _l
        rts

; the flippers alone for a whole frame (no ball in play)
flippers_frame
        ldx #SUBSTEPS
-       txa
        pha
        jsr flippers_step
        pla
        tax
        dex
        bne -
        rts

; ============================================================================
; physics
; ============================================================================
physics lda #SUBSTEPS
        sta substep
_loop   jsr flippers_step
        jsr ball_step
        lda drained
        bne _out
        dec substep
        bne _loop
        ; speed limit
_cap    ldx #vxl
        jsr overmax
        bcs _slow
        ldx #vyl
        jsr overmax
        bcc _fric
_slow   lda t0                  ; far over the limit: bigger steps
        ldy #4
        cmp #>VMAX+1
        bcc +
        ldy #2
+       sty t2
        ldx #vxl
        jsr fraction_off
        ldx #vyl
        jsr fraction_off
        jmp _cap
_fric   ; rolling friction: v -= v/512
        ldx #vxl
        jsr friction
        ldx #vyl
        jsr friction
_out    rts

; C = 1 if |word at zero page X| >= VMAX; t0 = high byte of |word|
overmax lda 1,x
        bpl +
        lda #0
        sec
        sbc 0,x
        sta t1
        lda #0
        sbc 1,x
        jmp ++
+       lda 0,x
        sta t1
        lda 1,x
+       sta t0
        cmp #>VMAX
        bne +
        lda t1
        cmp #<VMAX
+       rts

; v -= v >> t2 for the word at zero page X
fraction_off
        lda 1,x
        sta t1
        lda 0,x
        ldy t2
-       pha
        lda t1
        cmp #$80
        ror t1
        pla
        ror
        dey
        bne -
        sta t0
        sec
        lda 0,x
        sbc t0
        sta 0,x
        lda 1,x
        sbc t1
        sta 1,x
        rts

friction
        lda 1,x
        cmp #$80
        ror
        sta t0                  ; v / 512 (arithmetic)
        lda #0
        bit t0
        bpl +
        lda #$ff
+       sta t1
        sec
        lda 0,x
        sbc t0
        sta 0,x
        lda 1,x
        sbc t1
        sta 1,x
        rts

ball_step
        clc
        lda vyl
        adc #GRAV
        sta vyl
        bcc +
        inc vyh
+       clc
        lda bxl
        adc vxl
        sta bxl
        lda bxh
        adc vxh
        sta bxh
        clc
        lda byl
        adc vyl
        sta byl
        lda byh
        adc vyh
        sta byh
        cmp #204
        bcc +
        cmp #240
        bcs +
        inc drained
        rts
+       jsr collide_static
        jsr collide_flippers
        jmp collide_dynamic

; --- static walls ---------------------------------------------------------------
collide_static
        lda byh
        cmp #200
        bcs _out
        lsr
        tay
        lda bxh
        sec
        sbc #MAP_X0
        bcc _out
        cmp #MAP_W*2
        bcs _out
        lsr
        clc
        adc maprowlo,y
        sta ptr
        lda maprowhi,y
        adc #>MAPA
        sta ptr+1
        ldy #0
        lda (ptr),y
        bne +
_out    rts
+       sta cellA
        clc
        lda ptr
        adc #<MAP_SIZE
        sta ptr2
        lda ptr+1
        adc #>MAP_SIZE
        sta ptr2+1
        lda (ptr2),y
        sta cellD
        lda cellA
        and #63
        asl
        asl
        sta cang
        ; ball offset from the cell centre (1/16 px) along the normal
        lda bxh
        lsr
        lda bxl
        ror
        lsr
        lsr
        lsr
        sec
        sbc #16
        ldx cang
        ldy costab,x
        jsr smul8
        lda ql
        sta t2
        lda qh
        sta t3
        lda byh
        lsr
        lda byl
        ror
        lsr
        lsr
        lsr
        sec
        sbc #16
        ldx cang
        ldy sintab,x
        jsr smul8
        clc
        lda ql
        adc t2
        sta t2
        lda qh
        adc t3
        asl t2                  ; dot / 128 (fits a byte: |dot| < 24)
        rol
        sta t2
        ; pen = D - dot
        lda cellD
        sec
        sbc t2
        bvs _ovf
        bmi _out
        beq _out
        bne _pen
_ovf    bpl _out                ; overflow: the true result has the other sign
        lda #127
_pen    sta pen
        ; material
        lda #0
        sta kickl
        sta kickh
        sta surfl
        sta surfh
        lda cellA
        and #$c0
        cmp #$80
        beq _rub
        bcs _spec
        lda #E_WALL
        sta emat
        jmp contact
_rub    lda #E_RUBBER
        sta emat
        jmp contact
_spec   jsr special
        jmp contact

; A (signed byte) -> m16
sext    sta m16l
        ora #$7f
        bmi +
        lda #0
+       sta m16h
        rts

; material 3: which object?
special ldx #2
_b      lda bxh
        sec
        sbc bumpx,x
        clc
        adc #14
        cmp #29
        bcs _nb
        lda byh
        sec
        sbc bumpy,x
        clc
        adc #14
        cmp #29
        bcs _nb
        lda #E_BUMP
        sta emat
        lda bumpcool,x
        bne _r
        lda #8
        sta bumpcool,x
        lda #<KICK_BUMP
        sta kickl
        lda #>KICK_BUMP
        sta kickh
        lda bitmask,x
        ora ev_bump
        sta ev_bump
_r      rts
_nb     dex
        bpl _b
        lda byh
        cmp #SLING_Y0
        bcc _star
        cmp #SLING_Y1
        bcs _star
        ldx #0
        lda bxh
        sec
        sbc #LSLING_X0
        cmp #SLING_XW
        bcc _sl
        ldx #1
        lda bxh
        sec
        sbc #RSLING_X0
        cmp #SLING_XW
        bcs _star
_sl     lda #E_SLING
        sta emat
        lda slingcool,x
        bne _r
        lda #8
        sta slingcool,x
        lda #<KICK_SLING
        sta kickl
        lda #>KICK_SLING
        sta kickh
        lda bitmask,x
        ora ev_sling
        sta ev_sling
        rts
_star   lda byh
        sec
        sbc bxh
        clc
        adc #40
        ldx #3
_s      cmp starlo,x
        bcc _ns
        cmp starhi,x
        bcs _ns
        lda #E_STAR
        sta emat
        lda starcool,x
        bne _r
        lda #8
        sta starcool,x
        lda bitmask,x
        ora ev_star
        sta ev_star
        rts
_ns     dex
        bpl _s
        lda #E_WALL
        sta emat
        rts

; --- contact ----------------------------------------------------------------------
; normal angle cang, penetration pen, restitution emat, kick, surface speed:
; push the ball out along the normal, then bounce if it moves into the wall
contact ldx cang
        lda costab,x
        sta cnx
        lda sintab,x
        sta cny
        lda pen                 ; push out: pen * n / 8 (1/256 px)
        ldy cnx
        jsr smul8
        jsr q_sar3
        clc
        lda bxl
        adc ql
        sta bxl
        lda bxh
        adc qh
        sta bxh
        lda pen
        ldy cny
        jsr smul8
        jsr q_sar3
        clc
        lda byl
        adc ql
        sta byl
        lda byh
        adc qh
        sta byh
        ; vn = v.n - surface speed
        lda vxl
        sta m16l
        lda vxh
        sta m16h
        lda cnx
        jsr mul16s8
        lda plo
        sta vnl
        lda phi
        sta vnh
        lda vyl
        sta m16l
        lda vyh
        sta m16h
        lda cny
        jsr mul16s8
        clc
        lda vnl
        adc plo
        sta vnl
        lda vnh
        adc phi
        sta vnh
        sec
        lda vnl
        sbc surfl
        sta vnl
        lda vnh
        sbc surfh
        sta vnh
        bmi +
        rts
+       lda #0
        sec
        sbc vnl
        sta jl
        lda #0
        sbc vnh
        sta jh
        bne _bounce
        lda jl
        cmp #SLIDE
        bcc _kick
_bounce lda jl
        sta m16l
        lda jh
        sta m16h
        lda emat
        jsr mul16s8
        clc
        lda jl
        adc plo
        sta jl
        lda jh
        adc phi
        sta jh
_kick   clc
        lda jl
        adc kickl
        sta jl
        lda jh
        adc kickh
        sta jh
        cmp impact
        bcc +
        sta impact
+       lda jl
        sta m16l
        lda jh
        sta m16h
        lda cnx
        jsr mul16s8
        clc
        lda vxl
        adc plo
        sta vxl
        lda vxh
        adc phi
        sta vxh
        lda jl
        sta m16l
        lda jh
        sta m16h
        lda cny
        jsr mul16s8
        clc
        lda vyl
        adc plo
        sta vyl
        lda vyh
        adc phi
        sta vyh
        rts

; ql/qh >>= 3 (arithmetic)
q_sar3  lda qh
        cmp #$80
        ror
        ror ql
        cmp #$80
        ror
        ror ql
        cmp #$80
        ror
        ror ql
        sta qh
        rts

; --- flippers ---------------------------------------------------------------------
collide_flippers
        lda bxh
        sec
        sbc #LPIVX-8
        cmp #FLIP_L+20
        bcs _right
        lda byh
        sec
        sbc #PIVY-20
        cmp #44
        bcs _right
        lda #0
        sta mirror
        lda fpos
        clc
        adc #FLIP_UP & 255
        sta fang
        lda fwv
        sta fw
        jsr flipper_hit
_right  lda bxh
        sec
        sbc #RPIVX-FLIP_L-12
        cmp #FLIP_L+20
        bcs _out
        lda byh
        sec
        sbc #PIVY-20
        cmp #44
        bcs _out
        lda #1
        sta mirror
        lda fpos+1
        clc
        adc #FLIP_UP & 255
        sta fang
        lda fwv+1
        sta fw
        jmp flipper_hit
_out    rts

; (A + Y/256) * 16 -> t0/t1 (A signed)
to16    sta t0
        ora #$7f
        bmi +
        lda #0
+       sta t1
        ldx #4
-       asl t0
        rol t1
        dex
        bne -
        tya
        lsr
        lsr
        lsr
        lsr
        ora t0
        sta t0
        rts

flipper_hit
        ; ball relative to the pivot, mirrored for the right flipper
        lda bxh
        sec
        ldx mirror
        sbc pivx,x
        ldy bxl
        jsr to16
        lda mirror
        beq +
        lda #0
        sec
        sbc t0
        sta t0
        lda #0
        sbc t1
        sta t1
+       lda t0
        sta dxl
        lda t1
        sta dxh
        lda byh
        sec
        sbc #PIVY
        ldy byl
        jsr to16
        lda t0
        sta dyl
        lda t1
        sta dyh
        ldx fang
        lda costab,x
        sta fc
        lda sintab,x
        sta fs
        ; across = (dx s - dy c) / 128, positive above the flipper
        lda dxl
        sta m16l
        lda dxh
        sta m16h
        lda fs
        jsr mul16s8
        lda plo
        sta prl
        lda phi
        sta prh
        lda dyl
        sta m16l
        lda dyh
        sta m16h
        lda fc
        jsr mul16s8
        sec
        lda prl
        sbc plo
        sta prl
        lda prh
        sbc phi
        sta prh
        ; far from both faces: nothing to do
        lda prh
        bmi +
        bne _far
        lda prl
        cmp #8*16
        bcs _far
        bcc _near
+       cmp #$ff
        bne _far
        lda prl
        cmp #256-8*16
        bcs _near
_far    rts
_near
        ; along = (dx c + dy s) / 128
        lda dxl
        sta m16l
        lda dxh
        sta m16h
        lda fc
        jsr mul16s8
        lda plo
        sta all
        lda phi
        sta alh
        lda dyl
        sta m16l
        lda dyh
        sta m16h
        lda fs
        jsr mul16s8
        clc
        lda all
        adc plo
        sta all
        lda alh
        adc phi
        sta alh
        ; behind the pivot only the first pixel counts
        lda alh
        bpl _front
        cmp #$ff
        bne _none
        lda all
        cmp #$f0
        bcc _none
        lda #0
        sta all
        sta alh
_front  lda all
        cmp #<FLIP_L*16
        lda alh
        sbc #>FLIP_L*16
        bcc +
        jmp _tip
+       ; along the bar: contact radius for this point
        lda alh
        sta t0
        lda all
        lsr t0
        ror
        lsr t0
        ror
        lsr t0
        ror
        lsr t0
        ror
        sta frr
        tax
        lda fliprad,x
        sta t1
        lda prh
        bmi _below
        bne _none
        lda prl
        cmp t1
        bcs _none
        eor #$ff                ; pen = radius - across
        sec
        adc t1
        sta pen
        lda fang
        sec
        sbc #64
        sta cang
        lda #0
        sta t2                  ; top face
        jmp _surf
_none   rts
_below  cmp #$ff
        bne _none
        lda prl
        beq _none
        eor #$ff
        clc
        adc #1                  ; |across|
        cmp t1
        bcs _none
        eor #$ff
        sec
        adc t1
        sta pen
        lda fang
        clc
        adc #64
        sta cang
        lda #$80
        sta t2                  ; bottom face
_surf   ; surface speed along the normal: -fw * 2 pi r (top face)
        lda #0
        sta surfl
        sta surfh
        lda fw
        beq _go
        bpl +
        eor #$ff
        clc
        adc #1
+       ldx frr
        ldy flipsurf,x
        jsr umul8
        lda fw
        eor t2
        bpl _neg                ; falling top face or rising bottom face
        lda ql
        sta surfl
        lda qh
        sta surfh
        jmp _go
_neg    lda #0
        sec
        sbc ql
        sta surfl
        lda #0
        sbc qh
        sta surfh
_go     lda mirror
        beq +
        lda #128
        sec
        sbc cang
        sta cang
+       lda #E_FLIP
        sta emat
        lda #0
        sta kickl
        sta kickh
        jmp contact

_tip    ; round tip: offsets in half pixels from the tip centre
        lda all
        sec
        sbc #<FLIP_L*16
        sta t0
        lda alh
        sbc #>FLIP_L*16
        bne _tnone
        lda t0
        cmp #13*8
        bcs _tnone
        lsr
        lsr
        lsr
        tax                     ; 0-12
        lda prh
        bmi _tneg
        bne _tnone
        lda prl
        cmp #13*8
        bcs _tnone
        bcc _tok
_tnone  rts
_tneg   cmp #$ff
        bne _tnone
        lda prl
        cmp #256-12*8
        bcc _tnone
_tok    cmp #$80
        ror
        cmp #$80
        ror
        cmp #$80
        ror
        clc
        adc #12                 ; 0-24
        clc
        adc tiprowlo,x
        sta t0
        lda tiprowhi,x
        adc #0
        sta t1
        clc
        lda t0
        adc #<tipdist
        sta ptr
        lda t1
        adc #>tipdist
        sta ptr+1
        ldy #0
        lda (ptr),y
        cmp #TIPR16
        bcs _tnone
        eor #$ff
        sec
        adc #TIPR16
        sta pen
        clc
        lda t0
        adc #<tipang
        sta ptr
        lda t1
        adc #>tipang
        sta ptr+1
        lda (ptr),y
        sta t3                  ; local angle of the normal
        clc
        adc fang
        sta cang
        ; surface speed: -fw * 2 pi L, times the normal's share of it
        lda #0
        sta surfl
        sta surfh
        lda fw
        beq _go2
        bpl +
        eor #$ff
        clc
        adc #1
+       ldy flipsurf+FLIP_L
        jsr umul8
        lda ql
        sta m16l
        lda qh
        sta m16h
        lda fw
        bmi +
        lda #0
        sec
        sbc m16l
        sta m16l
        lda #0
        sbc m16h
        sta m16h
+       lda t3
        clc
        adc #64
        tax
        lda costab,x
        jsr mul16s8
        lda plo
        sta surfl
        lda phi
        sta surfh
_go2    jmp _go

; --- drop targets and the lane gate ---------------------------------------------
collide_dynamic
        lda byh
        cmp #TARGET_Y0
        bcc _gate
        cmp #TARGET_Y1
        bcs _gate
        lda bxh
        cmp #TARGET_X+4
        bcs _gate
        lda byh
        sec
        sbc #TARGET_Y0
        lsr
        lsr
        lsr
        sta t3
        tax
        lda tgt_up,x
        beq _gate
        ; pen = (TARGET_X + 3.5 - x) * 16
        lda bxh
        ldy bxl
        jsr to16
        ldx t3
        lda #<(TARGET_X*16+56)
        sec
        sbc t0
        sta pen
        lda #>(TARGET_X*16+56)
        sbc t1
        bne _gate               ; (never: the ball cannot be that far in)
        lda pen
        beq _gate
        bmi _gate
        lda #0
        sta cang
        sta kickl
        sta kickh
        sta surfl
        sta surfh
        lda #E_DROP
        sta emat
        lda bitmask,x
        ora ev_drop
        sta ev_drop
        jmp contact
_gate   lda byh
        cmp #GATE_Y0
        bcc _out
        cmp #GATE_Y1
        bcs _out
        lda vxh
        bmi _out
        ora vxl
        beq _out
        lda bxh
        cmp #GATE_X-4
        bcc _out
        cmp #GATE_X
        bcs _out
        ; pen = (x + 3.5 - GATE_X) * 16
        sec
        sbc #GATE_X-4
        ldy bxl
        jsr to16                ; (x - (GATE_X - 4)) * 16
        lda t0
        sec
        sbc #8
        bcc _out
        beq _out
        sta pen
        lda #128
        sta cang
        lda #0
        sta kickl
        sta kickh
        sta surfl
        sta surfh
        lda #E_GATE
        sta emat
        jmp contact
_out    rts

; ============================================================================
; maths
; ============================================================================

; A * Y (unsigned) -> ql, qh (and A), with quarter squares:
; ab = sq1[a + b] - sq2[b - a + 255]; the table bases get a and 255 - a
umul8   sta _s1+1
        sta _s3+1
        eor #$ff
        sta _s2+1
        sta _s4+1
        sec
_s1     lda sq1lo,y
_s2     sbc sq2lo,y
        sta ql
_s3     lda sq1hi,y
_s4     sbc sq2hi,y
        sta qh
        rts

; A * Y (signed) -> ql, qh (signed word)
smul8   sta ma
        sty mb
        jsr umul8
        lda ma
        bpl +
        lda qh
        sec
        sbc mb
        sta qh
+       lda mb
        bpl +
        lda qh
        sec
        sbc ma
        sta qh
+       rts

; (m16 * A) / 128: m16 signed word, A signed byte -> plo/phi
mul16s8 sta m8
        eor m16h
        sta msign
        lda m8
        bpl +
        eor #$ff
        clc
        adc #1
        sta m8
+       lda m16h
        bpl +
        lda #0
        sec
        sbc m16l
        sta tml
        lda #0
        sbc m16h
        sta tmh
        jmp _mul
+       lda m16l
        sta tml
        lda m16h
        sta tmh
_mul    lda tml
        ldy m8
        jsr umul8
        lda ql
        sta r0
        lda qh
        sta r1
        lda tmh
        bne +
        sta r2                  ; |m16| < 256: one product is enough
        beq _shift
+       ldy m8
        jsr umul8
        clc
        lda ql
        adc r1
        sta r1
        lda qh
        adc #0
        sta r2
_shift
        asl r0
        rol r1
        rol r2
        lda msign
        bpl +
        lda #0
        sec
        sbc r1
        sta plo
        lda #0
        sbc r2
        sta phi
        rts
+       lda r1
        sta plo
        lda r2
        sta phi
        rts

random  lda rnd
        asl
        bcc +
        eor #$1d
+       sta rnd
        rts

; ============================================================================
; rules
; ============================================================================

; nudge: joystick up or N; three quick ones tilt the table
nudge   lda tilted
        bne _r
        lda pressed
        and #K_NUDGE
        beq _r
        sec
        lda vyl
        sbc #120
        sta vyl
        bcs +
        dec vyh
+       lda frame
        and #1
        tax
        clc
        lda vxl
        adc nudgexl,x
        sta vxl
        lda vxh
        adc nudgexh,x
        sta vxh
        lda #SFX_THUD
        ldx #1
        jsr sfx
        lda tiltmeter
        clc
        adc #50
        sta tiltmeter
        cmp #120
        bcs _tilt
        cmp #70
        bcc _r
        ldx #<txt_danger
        ldy #>txt_danger
        jmp message
_tilt   lda #1
        sta tilted
        lda #SFX_TILT
        ldx #2
        jsr sfx
        ldx #NLAMPS-1
        lda #0
-       sta lampst,x
        dex
        bpl -
        ldx #<txt_tilt
        ldy #>txt_tilt
        jmp message
_r      rts

; a ball resting anywhere but on the flippers for 4 s gets a push
unstick lda byh
        cmp #150
        bcs _moving
        ldx #vxl
        jsr slow
        bcs _moving
        ldx #vyl
        jsr slow
        bcs _moving
        inc stuck
        lda stuck
        cmp #200
        bcc _r
        lda #<-200
        sta vyl
        lda #>-200
        sta vyh
        lda rnd
        and #$7f
        sec
        sbc #$40
        sta vxl
        lda #0
        sbc #0
        sta vxh
        lda #SFX_THUD
        ldx #1
        jsr sfx
_moving lda #0
        sta stuck
_r      rts

; C = 0 if |word at zero page X| < 24
slow    lda 1,x
        beq _pos
        cmp #$ff
        bne _fast
        lda 0,x
        cmp #256-24             ; C = 1: slow
        lda #0
        rol
        eor #1
        lsr
        rts
_pos    lda 0,x
        cmp #24
        rts
_fast   sec
        rts

nudgexl .byte <80, <-80
nudgexh .byte >80, >-80

; scoring from what the ball hit this frame
events  lda ev_bump
        beq _sl
        ldx #2
_b      lda ev_bump
        and bitmask,x
        beq _nb
        txa
        pha
        lda #6
        sta lampfl+L_BUMP,x
        ldx #P100
        jsr points
        lda #SFX_BUMPER
        ldx #1
        jsr sfx
        lda #$ff
        sta skill
        pla
        tax
_nb     dex
        bpl _b
        lda #0
        sta ev_bump
_sl     lda ev_sling
        beq _st
        ldx #1
_s      lda ev_sling
        and bitmask,x
        beq _ns
        txa
        pha
        lda #6
        sta lampfl+L_SLING,x
        ldx #P10
        jsr points
        lda #SFX_SLING
        ldx #1
        jsr sfx
        pla
        tax
_ns     dex
        bpl _s
        lda #0
        sta ev_sling
_st     lda ev_star
        beq _dr
        ldx #3
_t      lda ev_star
        and bitmask,x
        beq _nt
        txa
        pha
        jsr star_hit
        pla
        tax
_nt     dex
        bpl _t
        lda #0
        sta ev_star
_dr     lda ev_drop
        beq _r
        ldx #2
_d      lda ev_drop
        and bitmask,x
        beq _nd
        txa
        pha
        jsr drop_hit
        pla
        tax
_nd     dex
        bpl _d
        lda #0
        sta ev_drop
_r      rts

star_hit
        lda starreset
        bne _r
        lda stars
        and bitmask,x
        beq +
        ldx #P50
        jsr points
        lda #SFX_THUD
        ldx #1
        jmp sfx
+       lda stars
        ora bitmask,x
        sta stars
        lda #1
        sta lampst+L_STAR,x
        inc bonus
        ldx #P250
        jsr points
        lda #SFX_STAR
        ldx #1
        jsr sfx
        lda stars
        cmp #15
        bne _r
        ; S-T-A-R complete
        ldx #P10000
        jsr points
        lda bonus
        clc
        adc #5
        sta bonus
        ldx #3
-       lda #3
        sta lampst+L_STAR,x
        dex
        bpl -
        lda #100
        sta starreset
        lda #SFX_JINGLE
        ldx #2
        jsr sfx
        ldx #<txt_star
        ldy #>txt_star
        jmp message
_r      rts

drop_hit
        lda tgt_up,x
        beq _r
        jsr target_down
        inc bonus
        ldx #P500
        jsr points
        lda #SFX_TARGET
        ldx #1
        jsr sfx
        inc tgtdown
        lda tgtdown
        cmp #3
        bne _r
        ldx #P5000
        jsr points
        lda #75
        sta tgtreset
        lda again
        ora extralit
        bne +
        lda #1
        sta extralit
        lda #2
        sta lampst+L_EXTRA
        lda #SFX_JINGLE
        ldx #2
        jsr sfx
        ldx #<txt_extralit
        ldy #>txt_extralit
        jmp message
+       lda #SFX_LANES
        ldx #2
        jmp sfx
_r      rts

; rollovers and lanes: zone of the ball centre, acting when it changes
zones   ldx #NZONES-1
-       lda bxh
        cmp zx0,x
        bcc +
        cmp zx1,x
        bcs +
        lda byh
        cmp zy0,x
        bcc +
        cmp zy1,x
        bcc _found
+       dex
        bpl -
_found  stx zone
        cpx lastzone
        beq _saucer
        stx lastzone
        txa
        bmi _saucer
        asl
        tax
        lda zonejmp,x
        sta ptr
        lda zonejmp+1,x
        sta ptr+1
        jsr _jmp
_saucer lda saucerign
        beq +
        dec saucerign
        rts
+       lda bxh
        sec
        sbc #SAUCER_X-3
        cmp #7
        bcs _r
        lda byh
        sec
        sbc #SAUCER_Y-3
        cmp #7
        bcs _r
        ; captured
        lda #SAUCER_X
        sta bxh
        lda #SAUCER_Y
        sta byh
        lda #$80
        sta bxl
        sta byl
        lda #0
        sta vxl
        sta vxh
        sta vyl
        sta vyh
        lda #ST_SAUCER
        sta state
        lda #60
        sta statet
        lda #SFX_SAUCER
        ldx #2
        jsr sfx
        inc bonus
        ldx #P1000
        jsr points
        lda extralit
        beq _r
        lda #0
        sta extralit
        sta lampst+L_EXTRA
        lda #1
        sta again
        sta lampst+L_AGAIN
        lda #120
        sta statet
        lda #SFX_EXTRA
        ldx #2
        jsr sfx
        ldx #<txt_extra
        ldy #>txt_extra
        jmp message
_r      rts
_jmp    jmp (ptr)

NZONES  = 7
;           lanes           left out, in  right in, out
zx0     .byte 78, 98, 118,  16, 31,        156, 170
zx1     .byte 90, 110, 130, 28, 42,        167, 181
zy0     .byte 26, 26, 26,   140, 136,      136, 140
zy1     .byte 40, 40, 40,   156, 152,      152, 156
zonejmp .word lane0, lane1, lane2, outlane, inlane, inlane, outlane

lane0   ldx #0
        .byte $2c
lane1   ldx #1
        .byte $2c
lane2   ldx #2
        lda lanereset
        beq _go
        rts
_go     cpx skill
        bne +
        stx t3
        ldx #P5000
        jsr points
        ldx #<txt_skill
        ldy #>txt_skill
        jsr message
        lda #SFX_EXTRA
        ldx #2
        jsr sfx
        ldx t3
+       lda #$ff
        sta skill
        lda lanes
        and bitmask,x
        beq +
        ldx #P100
        jsr points
        lda #SFX_DING
        ldx #2
        jsr sfx
        jmp lane_lamps
+       lda lanes
        ora bitmask,x
        sta lanes
        inc bonus
        ldx #P500
        jsr points
        lda #SFX_DING
        ldx #2
        jsr sfx
        lda lanes
        cmp #7
        bne lane_lamps
        ; all three: multiplier up
        ldx #P2500
        jsr points
        lda mult
        cmp #5
        bcs +
        inc mult
        jsr show_mult
+       lda #60
        sta lanereset
        lda #SFX_LANES
        ldx #2
        jsr sfx
        lda mult
        ora #"0"
        sta txt_multd
        ldx #<txt_mult
        ldy #>txt_mult
        jsr message
        jmp lane_lamps
_r      rts

inlane  inc bonus
        ldx #P500
        jsr points
        lda #SFX_DING
        ldx #2
        jmp sfx

outlane ldx #P2500
        jsr points
        lda #SFX_THUD
        ldx #1
        jmp sfx

lanes_left
        lda lanereset
        bne +
        lda lanes
        asl
        cmp #8
        and #7
        adc #0
        sta lanes
        jmp lane_lamps
+       rts

lanes_right
        lda lanereset
        bne _r
        lda lanes
        lsr
        bcc +
        ora #4
+       sta lanes
        jmp lane_lamps
_r      rts

lane_lamps
        ldx #2
-       lda lanes
        and bitmask,x
        beq +
        lda #1
+       cpx skill
        bne +
        lda #3
+       sta lampst+L_LANE,x
        dex
        bpl -
        rts

show_mult
        ldx #4
-       lda #0
        cpx mult
        bcs +
        lda #1
+       sta lampst+L_MULT-1,x
        dex
        bne -
        ; panel: "2X"
        lda mult
        ora #"0"
        sta txt_multv
        ldx #<txt_multp
        ldy #>txt_multp
        lda #14
        sta pcolor
        lda #ROW_BALL
        sta prow
        lda #PX+9
        sta pcol
        jmp print

; per-frame timers
timers  ldx #2
-       lda bumpcool,x
        beq +
        dec bumpcool,x
+       dex
        bpl -
        ldx #1
-       lda slingcool,x
        beq +
        dec slingcool,x
+       dex
        bpl -
        ldx #3
-       lda starcool,x
        beq +
        dec starcool,x
+       dex
        bpl -
        lda tiltmeter
        beq +
        dec tiltmeter
+       lda ballsave
        ora ballsave+1
        beq _nosave
        lda state
        cmp #ST_PLAY
        bne _nosave
        lda ballsave
        bne +
        dec ballsave+1
+       dec ballsave
        lda ballsave
        ora ballsave+1
        bne _nosave
        lda #0
        sta lampst+L_SAVE
_nosave lda msgt
        beq +
        dec msgt
        bne +
        ldx #<txt_blank
        ldy #>txt_blank
        jsr message
        lda #0
        sta msgt
+       lda starreset
        beq +
        dec starreset
        bne +
        lda #0
        sta stars
        ldx #3
-       sta lampst+L_STAR,x
        dex
        bpl -
+       lda lanereset
        beq +
        dec lanereset
        bne +
        lda #0
        sta lanes
        jsr lane_lamps
+       lda tgtreset
        beq _r
        lda bxh
        cmp #TARGET_X+16
        bcc _r                  ; wait for the ball to leave the bank
        dec tgtreset
        bne _r
        jmp targets_all_up
_r      rts

; add points (X = index), BCD
points  lda tilted
        bne +
        sed
        clc
        lda score
        adc pts0,x
        sta score
        lda score+1
        adc pts1,x
        sta score+1
        lda score+2
        adc pts2,x
        sta score+2
        lda score+3
        adc #0
        sta score+3
        cld
        lda #1
        sta scoredirty
+       rts

;          10  50  100 250 500 1k  2.5k 5k  10k
pts0    .byte $10,$50,$00,$50,$00,$00,$00,$00,$00
pts1    .byte $00,$00,$01,$02,$05,$10,$25,$50,$00
pts2    .byte $00,$00,$00,$00,$00,$00,$00,$00,$01

bitmask .byte 1, 2, 4, 8, 16, 32, 64, 128
bumpx   .byte BUMP0X, BUMP1X, BUMP2X
bumpy   .byte BUMP0Y, BUMP1Y, BUMP2Y
pivx    .byte LPIVX, RPIVX

; ============================================================================
; drop targets: bitmap patches
; ============================================================================
targets_all_up
        ldx #2
-       lda #1
        sta tgt_up,x
        txa
        pha
        lda #1
        jsr target_patch
        pla
        tax
        dex
        bpl -
        lda #0
        sta tgtdown
        sta tgtreset
        rts

target_down
        lda #0
        sta tgt_up,x
        txa
        pha
        lda #0
        jsr target_patch
        pla
        tax
        rts

; X = target, A = 1 up / 0 down: copy its cells
target_patch
        sta t0
        txa
        asl
        ora t0
        tay
        lda tgtdata,y
        sta sptr
        lda tgtdatah,y
        sta sptr+1
        lda tgtcl,x
        sta ptr2
        lda tgtch,x
        sta ptr2+1
        ldy #0
        lda (ptr2),y
        sta t1                  ; cells
        inc ptr2
        bne _cell
        inc ptr2+1
_cell   ldy #0
        lda (ptr2),y
        sta t2
        iny
        lda (ptr2),y
        sta t3                  ; cell offset t2/t3
        ; bitmap: BITMAP + (offset / 40) * 320 + (offset % 40) * 8 = 8 * offset + (offset/40)*0
        ; (320 = 8 * 40, so the bitmap address is simply 8 * offset)
        lda t2
        sta ptr
        lda t3
        asl ptr
        rol
        asl ptr
        rol
        asl ptr
        rol
        ora #>BITMAP
        sta ptr+1
        ldy #7
-       lda (sptr),y
        sta (ptr),y
        dey
        bpl -
        lda t2
        sta ptr
        lda t3
        ora #>SCREEN
        sta ptr+1
        ldy #8
        lda (sptr),y
        ldy #0
        sta (ptr),y
        lda t3
        ora #>COLRAM
        sta ptr+1
        ldy #9
        lda (sptr),y
        ldy #0
        sta (ptr),y
        clc
        lda sptr
        adc #10
        sta sptr
        bcc +
        inc sptr+1
+       clc
        lda ptr2
        adc #2
        sta ptr2
        bcc +
        inc ptr2+1
+       dec t1
        bne _cell
        rts

tgtdata  .byte <tgtdown0, <tgtup0, <tgtdown1, <tgtup1, <tgtdown2, <tgtup2
tgtdatah .byte >tgtdown0, >tgtup0, >tgtdown1, >tgtup1, >tgtdown2, >tgtup2
tgtcl   .byte <tgtcells0, <tgtcells1, <tgtcells2
tgtch   .byte >tgtcells0, >tgtcells1, >tgtcells2

; ============================================================================
; display
; ============================================================================
update_sprites
        lda ballvis
        beq _hide
        lda bxl
        sec
        sbc #$80
        lda bxh
        sbc #3
        clc
        adc #24
        sta $d000
        sta $d002
        lda byl
        sec
        sbc #$80
        lda byh
        sbc #3
        clc
        adc #50
        sta $d001
        sta $d003
        jmp _flip
_hide   lda #0
        sta $d001
        sta $d003
_flip   lda fpos
        clc
        adc #2
        lsr
        lsr
        tax
        lda fliplx,x
        sta $d004
        clc
        adc #24
        sta $d006
        lda fliply,x
        sta $d005
        sta $d007
        txa
        asl
        adc #BLK_LFLIP
        sta SCREEN+$3fa
        adc #1
        sta SCREEN+$3fb
        lda fpos+1
        clc
        adc #2
        lsr
        lsr
        tax
        lda fliprx,x
        sta $d008
        clc
        adc #24
        sta $d00a
        lda flipry,x
        sta $d009
        sta $d00b
        txa
        asl
        adc #BLK_RFLIP
        sta SCREEN+$3fc
        adc #1
        sta SCREEN+$3fd
        lda pullvis
        clc
        adc #PLUNGER_Y+4+50
        sta $d00d
        rts

; lamps: colour RAM of their cells when they change
update_lamps
        ldx #NLAMPS-1
_l      lda lampfl,x
        beq +
        dec lampfl,x
        lda #1
        bne _set
+       lda lampst,x
        cmp #2
        bcc _set
        beq _slow
        lda frame
        lsr
        lsr
        and #1
        jmp _set
_slow   lda frame
        lsr
        lsr
        lsr
        lsr
        and #1
_set    tay
        beq _off
        lda lampon,x
        bne +
_off    lda lampoff,x
+       cmp lampshown,x
        beq _next
        sta lampshown,x
        sta t0
        stx t1
        lda lampfirst+1,x
        sta t2
        lda lampfirst,x
        tax
_c      cpx t2
        bcs _done
        lda lampcelllo,x
        sta ptr
        lda lampcellhi,x
        ora #>COLRAM
        sta ptr+1
        ldy #0
        lda t0
        sta (ptr),y
        inx
        bne _c
_done   ldx t1
_next   dex
        bpl _l
        rts

; --- panel -----------------------------------------------------------------------
panel_init
        ; big score digits: yellow above, orange below
        ldx #7
-       lda #7
        sta COLRAM+ROW_SCORE*40+40+PX+2,x
        lda #8
        sta COLRAM+ROW_SCORE*40+80+PX+2,x
        lda #0
        sta SCREEN+ROW_SCORE*40+40+PX+2,x
        sta SCREEN+ROW_SCORE*40+80+PX+2,x
        lda #$ff
        sta shown,x
        dex
        bpl -
        lda #14
        ldx #<txt_score
        ldy #>txt_score
        jsr print_at_score
        lda #14
        ldx #<txt_high
        ldy #>txt_high
        jsr print_at_high
        jsr draw_hiscore
        ; help
        ldx #0
-       txa
        pha
        lda helprows,x
        sta prow
        lda #PX
        sta pcol
        lda #12
        sta pcolor
        lda helplo,x
        sta sptr
        lda helphi,x
        sta sptr+1
        jsr print_s
        pla
        tax
        inx
        cpx #5
        bne -
        rts

print_at_score
        sta pcolor
        lda #ROW_SCORE
        sta prow
        lda #PX
        sta pcol
        jmp print
print_at_high
        sta pcolor
        lda #ROW_HIGH
        sta prow
        lda #PX
        sta pcol
        jmp print

show_ball
        lda ball
        ora #"0"
        sta txt_ballv
        lda #14
        sta pcolor
        lda #ROW_BALL
        sta prow
        lda #PX
        sta pcol
        ldx #<txt_ball
        ldy #>txt_ball
        jmp print

show_bonus
        lda bonus
        cmp #100
        bcc +
        lda #99
        sta bonus
+
        ldx #"0"-1
        sec
-       inx
        sbc #10
        bcs -
        adc #10+"0"
        stx txt_bonusv
        sta txt_bonusv+1
        lda mult
        ora #"0"
        sta txt_bonusv+4
        ldx #<txt_bonus
        ldy #>txt_bonus
        jsr message
        lda #0
        sta msgt
        rts

; message: X/Y = two lines of 12 characters, drawn a few per frame
message stx msgptr
        sty msgptr+1
        lda #0
        sta msgidx
        lda #100
        sta msgt
        rts

MSGCHARS = 6                    ; characters drawn per frame

msg_job lda #MSGCHARS
        sta t3
_c      ldy msgidx
        cpy #24
        bcs _r
        inc msgidx
        lda #7                  ; first line yellow, second white
        ldx #ROW_MSG
        cpy #12
        bcc +
        iny                     ; skip the 0 after the first line
        lda #1
        inx
+       sta pcolor
        stx prow
        lda msgptr
        sta sptr
        lda msgptr+1
        sta sptr+1
        lda (sptr),y
        tax
        lda msgidx
        sec
        sbc #1
        cmp #12
        bcc +
        sbc #12
+       clc
        adc #PX
        sta pcol
        lda fontidx,x
        jsr putglyph
        dec t3
        bne _c
_r      rts

; X/Y = string, pcol/prow/pcolor set
print   stx sptr
        sty sptr+1
print_s ldy #0
        lda (sptr),y
        beq _done
        tax
        lda fontidx,x
        jsr putglyph
        inc pcol
        inc sptr
        bne print_s
        inc sptr+1
        bne print_s
_done   rts

; glyph A at pcol/prow, colour pcolor
putglyph
        sta t0
        lda #0
        sta t1
        asl t0
        rol t1
        asl t0
        rol t1
        asl t0
        rol t1
        clc
        lda t0
        adc #<font
        sta ptr
        lda t1
        adc #>font
        sta ptr+1
        jsr putcellbm
        ldy prow
        clc
        lda scrrowlo,y
        adc pcol
        sta ptr2
        lda scrrowhi,y
        adc #>COLRAM
        sta ptr2+1
        ldy #0
        lda pcolor
        sta (ptr2),y
        lda ptr2+1
        eor #(>COLRAM)^(>SCREEN)
        sta ptr2+1
        tya
        sta (ptr2),y
        rts

; score: 8 big digits, only those that changed
draw_score
        lda scoredirty
        bne +
        rts
+       lda #0
        sta scoredirty
        sta t3                  ; 1 after the first non-zero digit
        lda #3
        sta t1digits            ; digits drawn at most per frame
        ldx #0
_d      txa
        lsr
        eor #3
        tay
        lda score,y
        pha
        txa
        and #1
        bne +
        pla
        lsr
        lsr
        lsr
        lsr
        jmp ++
+       pla
        and #15
+       bne _nz
        ldy t3
        bne _nz
        cpx #7
        beq _nz
        lda #10                 ; leading blank
        bne _show
_nz     ldy #1
        sty t3
_show   cmp shown,x
        beq _next
        sta shown,x
        stx t2
        ; 16 bytes of the big font
        asl
        asl
        asl
        asl
        clc
        adc #<bigfont
        sta ptr
        lda #>bigfont
        adc #0
        sta ptr+1
        txa
        clc
        adc #PX+2
        sta pcol
        lda #ROW_SCORE+1
        sta prow
        ldx #8
        jsr putcellbm
        clc
        lda ptr
        adc #8
        sta ptr
        bcc +
        inc ptr+1
+       inc prow
        ldx #8
        jsr putcellbm
        ldx t2
        dec t1digits
        bne _next
        inc scoredirty          ; more digits next frame
        rts
_next   inx
        cpx #8
        bne _d
_r      rts

; copy 8 bytes (ptr) into the bitmap cell pcol/prow
putcellbm
        lda pcol
        sta t0
        lda #0
        asl t0
        rol
        asl t0
        rol
        asl t0
        rol
        sta t1
        ldy prow
        clc
        lda bmrowlo,y
        adc t0
        sta ptr2
        lda bmrowhi,y
        adc t1
        adc #>BITMAP
        sta ptr2+1
        ldy #7
-       lda (ptr),y
        sta (ptr2),y
        dey
        bpl -
        rts

draw_hiscore
        ldx #0
        ldy #3
-       lda hiscore,y
        lsr
        lsr
        lsr
        lsr
        ora #"0"
        sta txt_hiv,x
        lda hiscore,y
        and #15
        ora #"0"
        sta txt_hiv+1,x
        inx
        inx
        dey
        bpl -
        ; leading zeros off
        ldx #0
-       lda txt_hiv,x
        cmp #"0"
        bne +
        lda #" "
        sta txt_hiv,x
        inx
        cpx #7
        bne -
+       lda #ROW_HIGH+1
        sta prow
        lda #PX+2
        sta pcol
        lda #15
        sta pcolor
        ldx #<txt_hiv
        ldy #>txt_hiv
        jmp print

; --- texts ----------------------------------------------------------------------
txt_score .null "SCORE"
txt_high .null "HIGH SCORE"
txt_hiv .null "00000000"
txt_ball .text "BALL "
txt_ballv .null "1"
txt_multp .text " "
txt_multv .null "1X"
txt_blank .null "            "
        .null "            "
txt_press .null " PRESS FIRE "
        .null "  TO START  "
txt_credit .null " SAMPOSOFT  "
        .null "    2026    "
txt_saved .null " BALL SAVED "
        .null "SHOOT AGAIN!"
txt_again .null "SHOOT AGAIN!"
        .null "            "
txt_over .null " GAME  OVER "
        .null "            "
txt_newhigh .null "  NEW HIGH  "
        .null "   SCORE!   "
txt_paused .null "   PAUSED   "
        .null "            "
txt_danger .null "   DANGER   "
        .null "            "
txt_tilt .null "    TILT    "
        .null "            "
txt_star .null " STAR BONUS "
        .null "   10000    "
txt_extralit .null " EXTRA BALL "
        .null "   IS LIT   "
txt_extra .null " EXTRA BALL "
        .null "            "
txt_skill .null " SKILL SHOT "
        .null "    5000    "
txt_mult .text "  BONUS "
txt_multd .null "2X  "
        .null "            "
txt_bonus .text " BONUS "
txt_bonusv .null "00 X1"
        .null "            "

helprows .byte 19, 20, 21, 22, 23
helplo  .byte <h1, <h2, <h3, <h4, <h5
helphi  .byte >h1, >h2, >h3, >h4, >h5
h1      .null "FIRE  START"
h2      .null "SHIFT FLIP"
h3      .null "SPACE LAUNCH"
h4      .null "UP    NUDGE"
h5      .null "P     PAUSE"

; ============================================================================
; sound: effects on the three voices, the tune in attract mode
; ============================================================================
sid_init
        ldx #$18
        lda #0
-       sta $d400,x
        dex
        bpl -
volume  lda paused
        beq +
        lda #0
        sta $d418
        rts
+       lda #15
        sta $d418
        rts

; start effect A on voice X (unless a more important one is playing)
sfx     ldy musicon
        beq +
        rts
+        tay
        lda sfxlo,y
        sta ptr
        lda sfxhi,y
        sta ptr+1
        lda sfxon,x
        beq _go
        ldy #0
        lda (ptr),y
        cmp sfxpri,x
        bcc _r
_go     sei
        ldy voiceoff,x
        sty t1
        lda #0
        sta $d404,y             ; gate off: the effect starts next frame
        tay
        lda (ptr),y
        sta sfxpri,x
        iny
        lda (ptr),y
        ldy t1
        sta $d405,y
        ldy #2
        lda (ptr),y
        ldy t1
        sta $d406,y
        ldy #3
        lda (ptr),y
        ldy t1
        sta $d403,y
        clc
        lda ptr
        adc #4
        sta sfxlo_,x
        lda ptr+1
        adc #0
        sta sfxhi_,x
        lda #1
        sta sfxon,x
        cli
_r      rts

voiceoff .byte 0, 7, 14

sound_frame
        lda musicon
        beq +
        jmp music
+       ldx #2
_v      lda sfxon,x
        beq _next
        lda sfxlo_,x
        sta iptr
        lda sfxhi_,x
        sta iptr+1
        ldy voiceoff,x
        sty it0
        ldy #0
        lda (iptr),y
        beq _end
        sta sfxctl,x
        iny
        lda (iptr),y
        pha
        iny
        lda (iptr),y
        ldy it0
        sta $d401,y
        pla
        sta $d400,y
        lda sfxctl,x
        sta $d404,y
        clc
        lda sfxlo_,x
        adc #3
        sta sfxlo_,x
        bcc _next
        inc sfxhi_,x
        jmp _next
_end    ldy it0
        lda sfxctl,x
        and #$fe
        sta $d404,y
        lda #0
        sta sfxon,x
_next   dex
        bpl _v
        rts

; --- tune ----------------------------------------------------------------------
music_start
        sei
        ldx #2
-       lda #0
        sta sfxon,x
        lda tunelo,x
        sta mptr,x
        lda tunehi,x
        sta mptr+3,x
        lda #1
        sta mdur,x
        ldy voiceoff,x
        lda instad,x
        sta $d405,y
        lda instsr,x
        sta $d406,y
        lda #$08
        sta $d403,y
        dex
        bpl -
        lda #1
        sta musicon
        cli
        rts

music_stop
        sei
        lda #0
        sta musicon
        ldx #2
-       ldy voiceoff,x
        sta $d404,y
        sta sfxon,x
        dex
        bpl -
        cli
        rts

music   ldx #2
_v      dec mdur,x
        bne _next
        lda mptr,x
        sta iptr
        lda mptr+3,x
        sta iptr+1
        ldy #1
        lda (iptr),y
        bne +
        ; end: loop
        lda tunelo,x
        sta mptr,x
        sta iptr
        lda tunehi,x
        sta mptr+3,x
        sta iptr+1
        lda (iptr),y
+       sta mdur,x
        dey
        lda (iptr),y
        ldy voiceoff,x
        sty it0
        tay
        beq _rest
        lda notelo,y
        pha
        lda notehi,y
        ldy it0
        sta $d401,y
        pla
        sta $d400,y
        lda instwave,x
        sta $d404,y
        ora #1
        sta $d404,y
        jmp _adv
_rest   ldy it0
        lda instwave,x
        sta $d404,y
_adv    clc
        lda mptr,x
        adc #2
        sta mptr,x
        bcc _next
        inc mptr+3,x
_next   dex
        bpl _v
        rts

tunelo  .byte <tune0, <tune1, <tune2
tunehi  .byte >tune0, >tune1, >tune2
instad  .byte $0a, $09, $02
instsr  .byte $a0, $6a, $30
instwave .byte $40, $40, $10

; ============================================================================
; data
; ============================================================================
        .include "data.asm"

        .cerror * > $7000, "code and tables overlap the init data"

; init-only data: copied into VIC bank 3 at start, then the collision maps
; are unpacked over it
        * = MAPA
        .include "init.asm"
        .cerror * > $c000, "init data too long"
        .cerror MAPD + MAP_SIZE > VARS, "maps overlap the variables"
