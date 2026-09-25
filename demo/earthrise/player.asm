; SID player (init, play) and VU meter levels, adapted from
; demo/robo_nine/player.asm: no drums, the filter cutoff follows a slow
; triangle LFO instead of an envelope per note, and `vu` follows the
; envelope of each voice for the meters.
;
; Tables from music.asm (music.py): instruments i_*, wave table wtw/wtn,
; patterns and order lists. Zero page: zp (2 bytes), tmp, tmp2, tmp3.

init    ldx #2
-       lda #0
        sta ordpos,x
        sta hr,x
        sta cwave,x
        sta ins,x
        sta env,x
        sta vgate,x
        sta vphase,x
        sta level,x
        sta peak,x
        sta phold,x
        lda #$fe
        sta gmask,x
        dex
        bpl -
        lda #0
        sta tick
        sta row
        sta froute
        sta lfo
        sta $d415
        lda #$1f                ; low pass, volume 15
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
        jsr vu
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
        lda (zp),y
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
        lda froute
        ora vbit,x
        sta froute
        jmp vs4
vs3     lda vbit,x
        eor #$ff
        and froute
        sta froute
vs4     lda i_ad,y
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

; voice X: VU meter. The SID envelope cannot be read (only voice 3's), so
; `env` (0-255) imitates it with the instrument's rates, computed by
; music.py from its ADSR and loudness: attack up to the instrument's peak
; when the gate opens, decay to the sustain level, release to 0 when the
; gate closes. `level` (0-VUSEGS)
; is the number of lit segments, `peak` the segment that stays lit for
; PEAKHOLD frames and then falls by one every 2 frames.
vu      ldy ins,x
        lda cwave,x
        and gmask,x
        and #1
        cmp vgate,x
        sta vgate,x
        beq vu1
        cmp #1                  ; the gate has just opened: attack
        bne vu1
        lda #0
        sta vphase,x
vu1     lda vgate,x
        beq vurel
        lda vphase,x
        bne vudec
        lda env,x               ; attack, up to the instrument's peak
        clc
        adc i_va,y
        bcs vuatop
        cmp i_vmax,y
        bcc vuatk
vuatop  lda i_vmax,y
        inc vphase,x
vuatk   sta env,x
        jmp vulev
vudec   lda env,x               ; decay to the sustain level
        sec
        sbc i_vd,y
        bcc +
        cmp i_vs,y
        bcs ++
+       lda i_vs,y
+       sta env,x
        jmp vulev
vurel   lda env,x               ; release
        sec
        sbc i_vr,y
        bcs +
        lda #0
+       sta env,x
vulev   lda env,x
        lsr a
        lsr a
        tay
        lda vulevel,y
        sta level,x
        cmp peak,x
        bcc vupk
        sta peak,x
        lda #PEAKHOLD
        sta phold,x
        rts
vupk    lda phold,x
        beq +
        dec phold,x
        rts
+       lda tick                ; falls every other frame
        and #1
        bne +
        dec peak,x
+       rts

; filter: the cutoff follows a slow triangle (about 20 s a cycle)
filter  inc lfo
        bne +
        inc lfo+1
+       lda lfo
        sta tmp
        lda lfo+1               ; phase: bits 9-2 of the counter
        lsr a
        ror tmp
        lsr a
        ror tmp
        lda tmp
        bpl +
        eor #$ff
+       clc                     ; $0C-$8B
        adc #$0c
        sta $d416
        lda froute
        ora #$e0                ; resonance 14
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

; lit segments for env >> 2
vulevel .for e = 0, e < 64, e += 1
        .byte (e * (VUSEGS + 1)) / 64
        .next

; player state
tick    .byte 0
row     .byte 0
froute  .byte 0
lfo     .word 0
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
env     .fill 3, 0
vgate   .fill 3, 0
vphase  .fill 3, 0
level   .fill 3, 0              ; read by the meters
peak    .fill 3, 0
phold   .fill 3, 0
