; SID player (init, play), adapted from demo/sid/game_tune/player.asm:
; the drums set "beat" (1 = kick, 2 = snare) instead of flashing the border.

init    ldx #2
-       lda #0
        sta ordpos,x
        sta hr,x
        sta cwave,x
        sta ins,x
        lda #$fe
        sta gmask,x
        dex
        bpl -
        lda #0
        sta tick
        sta row
        sta froute
        sta fdec
        sta beat
        sta $d415
        lda #$10
        sta fceil
        lda #$28
        sta fcut
        lda #$1f
        sta $d418
        rts

; --- one frame of the player ---------------------------------------------
play    lda tick
        bne fx
        ldx #0
-       lda row
        bne +
        jsr nextpat
+       jsr readrow
        inx
        cpx #3
        bne -
        lda row
        clc
        adc #1
        and #15
        sta row
fx      inc tick
        lda tick
        cmp #SPEED
        bcc +
        lda #0
        sta tick
+       ldx #0
-       jsr voice
        inx
        cpx #3
        bne -
        jmp filter

; voice X: next pattern from the order list ($FF,n = jump to n)
nextpat lda ordlo,x
        sta zp
        lda ordhi,x
        sta zp+1
        ldy ordpos,x
        lda (zp),y
        cmp #$ff
        bne np1
        iny
        lda (zp),y
        tay
        cpx #0
        bne +
        lda #$10
        sta fceil
+       lda (zp),y
np1     iny
        sta tmp
        tya
        sta ordpos,x
        ldy tmp
        lda patlo,y
        sta pplo,x
        lda pathi,y
        sta pphi,x
        rts

; voice X: reads one row of the pattern
readrow lda pplo,x
        sta zp
        lda pphi,x
        sta zp+1
        ldy #0
        lda (zp),y
        cmp #$e0
        bcc +
        and #$1f
        sta ins,x
        iny
        lda (zp),y
+       iny
        sta tmp
        tya
        clc
        adc zp
        sta pplo,x
        lda zp+1
        adc #0
        sta pphi,x
        lda tmp
        beq rrdone
        cmp #$60
        bne rrnote
        lda #$fe
        sta gmask,x
rrdone  rts
rrnote  sta note,x
        lda #2                  ; hard restart: 2 frames with the gate off, ADSR 0
        sta hr,x
        lda #$fe
        sta gmask,x
        ldy sidoff,x
        lda #0
        sta $d405,y
        sta $d406,y
        lda cwave,x
        and #$fe
        sta $d404,y
        rts

; voice X: effects of the frame and SID registers
voice   lda hr,x
        bne +
        jmp vrun
+       dec hr,x
        beq vstart
        rts
vstart  ldy ins,x
        lda i_wt,y
        sta wtp,x
        lda i_pw,y
        sta pwhi,x
        lda #0
        sta pwlo,x
        sta pwdir,x
        sta vibp,x
        sta viblo,x
        sta vibhi,x
        lda i_vdel,y
        sta vibd,x
        lda #$ff
        sta gmask,x
        lda i_vsh,y             ; vibrato step = (f(n+1)-f(n)) >> vsh
        beq vs2
        sta tmp
        ldy note,x
        lda flo+1,y
        sec
        sbc flo,y
        sta vdlo,x
        lda fhi+1,y
        sbc fhi,y
        sta vdhi,x
-       lsr vdhi,x
        ror vdlo,x
        dec tmp
        bne -
vs2     ldy ins,x
        lda i_flt,y
        beq vs3
        lda i_fst,y
        sta fcut
        lda i_fdec,y
        sta fdec
        lda froute
        ora vbit,x
        sta froute
        jmp vs4
vs3     lda vbit,x
        eor #$ff
        and froute
        sta froute
vs4     lda i_beat,y
        beq +
        sta beat
+       lda i_ad,y
        sta tmp
        lda i_sr,y
        ldy sidoff,x
        sta $d406,y
        lda tmp
        sta $d405,y

vrun    ldy wtp,x               ; wave table
        lda wtw,y
        cmp #$ff
        bne +
        lda wtn,y
        sta wtp,x
        tay
        lda wtw,y
+       cmp #$fe
        beq vfreq
        sta cwave,x
        lda wtn,y
        bmi +
        clc
        adc note,x
        jmp ++
+       and #$7f
+       sta cnote,x
        inc wtp,x

vfreq   ldy cnote,x
        lda flo,y
        sta tmp
        lda fhi,y
        sta tmp2
        ldy ins,x
        lda i_vsh,y
        beq vfw
        lda vibd,x
        beq +
        dec vibd,x
        jmp vfw
+       lda vibp,x              ; triangle: up 2, down 4, up 2
        inc vibp,x
        and #7
        cmp #2
        bcc vup
        cmp #6
        bcc vdn
vup     lda viblo,x
        clc
        adc vdlo,x
        sta viblo,x
        lda vibhi,x
        adc vdhi,x
        sta vibhi,x
        jmp vadd
vdn     lda viblo,x
        sec
        sbc vdlo,x
        sta viblo,x
        lda vibhi,x
        sbc vdhi,x
        sta vibhi,x
vadd    lda tmp
        clc
        adc viblo,x
        sta tmp
        lda tmp2
        adc vibhi,x
        sta tmp2

vfw     ldy ins,x               ; PWM between $300 and $E00
        lda i_pws,y
        beq vpw
        sta tmp3
        lda pwdir,x
        bne vpd
        lda pwlo,x
        clc
        adc tmp3
        sta pwlo,x
        lda pwhi,x
        adc #0
        sta pwhi,x
        cmp #$0e
        bcc vpw
        lda #1
        sta pwdir,x
        bne vpw
vpd     lda pwlo,x
        sec
        sbc tmp3
        sta pwlo,x
        lda pwhi,x
        sbc #0
        sta pwhi,x
        cmp #$03
        bcs vpw
        lda #0
        sta pwdir,x
vpw     ldy sidoff,x
        lda tmp
        sta $d400,y
        lda tmp2
        sta $d401,y
        lda pwlo,x
        sta $d402,y
        lda pwhi,x
        sta $d403,y
        lda cwave,x
        and gmask,x
        sta $d404,y
        rts

; filter: cutoff envelope per note + slow opening in the intro
filter  lda fcut
        sec
        sbc fdec
        bcc +
        cmp #$28
        bcs ++
+       lda #$28
+       sta fcut
        lda fceil
        cmp #$ff
        beq +
        inc fceil
+       lda fcut
        cmp fceil
        bcc +
        lda fceil
+       sta $d416
        lda froute
        ora #$c0
        sta $d417
        rts

sidoff  .byte 0, 7, 14
vbit    .byte 1, 2, 4

; PAL frequencies for MIDI notes 0..96
flo     .for n = 0, n < 97, n += 1
        .byte <(int(440.0 * 2.0**((n-69)/12.0) * 17.0284 + 0.5))
        .next
fhi     .for n = 0, n < 97, n += 1
        .byte >(int(440.0 * 2.0**((n-69)/12.0) * 17.0284 + 0.5))
        .next


; player state
tick    .byte 0
row     .byte 0
fcut    .byte 0
fceil   .byte 0
fdec    .byte 0
froute  .byte 0
beat    .byte 0              ; set by the drums, cleared by the show
ordpos  .fill 3, 0
pplo    .fill 3, 0
pphi    .fill 3, 0
ins     .fill 3, 0
note    .fill 3, 0
hr      .fill 3, 0
gmask   .fill 3, 0
wtp     .fill 3, 0
cwave   .fill 3, 0
cnote   .fill 3, 0
pwlo    .fill 3, 0
pwhi    .fill 3, 0
pwdir   .fill 3, 0
vibd    .fill 3, 0
vibp    .fill 3, 0
viblo   .fill 3, 0
vibhi   .fill 3, 0
vdlo    .fill 3, 0
vdhi    .fill 3, 0
