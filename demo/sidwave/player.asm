; SPDX-License-Identifier: GPL-2.0-or-later
; Copyright (C) 2026 SampoSoft - Francesco Sampoli
;
; SIDWAVE player: a table-driven SID player in the style of the modern
; trackers (wave, pulse and filter programs per instrument).
;
;   mus_init   resets the song
;   mus_play   one frame (call once per frame)
;
; Per voice, every frame: gate timer, wave table step (waveform and note,
; relative or absolute), pitch = note + slide + vibrato in 1/16 semitones
; interpolated on the frequency table, pulse program; then the global
; filter program and the volume fade.
;
; Hard restart: the player reads the next row on frame 1, 2 or 3 of the
; last row of an event (voice 1, 2, 3: never all three on the same frame);
; HRT frames before a new note (not legato) it turns the gate off and sets
; ADSR to $0000,
; so the envelope's rate counter starts again from zero and the attack of
; the new note is always on time (the "ADSR bug").
;
; The tables come from music.asm (music.py), which also documents the
; formats. The includer defines MZP and MZP2 (two zero page words).

HRT     = 2

; voice arrays: indexed by X = 0, 7, 14 (the voice's SID register offset)
VSIZE   = 15
mvars
pptrlo  .fill VSIZE     ; pattern pointer
pptrhi  .fill VSIZE
opos    .fill VSIZE     ; order list position
transp  .fill VSIZE     ; transposition from the order list
rows    .fill VSIZE     ; rows left of the current event
dur     .fill VSIZE     ; event duration in rows (sticky)
pend    .fill VSIZE     ; pending event: 0 none, 1 note, 2 rest, 3 tie
pflag   .fill VSIZE     ; pending: bit 0 legato, bit 1 portamento, bit 7 new instrument
pnote   .fill VSIZE
pinst   .fill VSIZE
pspd    .fill VSIZE
inst    .fill VSIZE     ; current instrument
note    .fill VSIZE     ; current note (transposed)
wave    .fill VSIZE     ; waveform register value (with gate)
gmask   .fill VSIZE     ; $ff gate on, $fe gate off
gtimer  .fill VSIZE     ; frames to the automatic gate off
wtptr   .fill VSIZE     ; wave table position (0 = stopped)
wtnote  .fill VSIZE     ; last note byte of the wave table
ptptr   .fill VSIZE     ; pulse table position (0 = stopped)
pcnt    .fill VSIZE     ; pulse sweep frames left
pwlo    .fill VSIZE
pwhi    .fill VSIZE
slidelo .fill VSIZE     ; portamento offset (1/16 semitones, signed)
slidehi .fill VSIZE
slspd   .fill VSIZE
vibdel  .fill VSIZE     ; frames before the vibrato starts
vibval  .fill VSIZE     ; vibrato offset (1/16 semitones, signed)
vibcnt  .fill VSIZE
vibdir  .fill VSIZE
trig    .fill VSIZE     ; 1 when a note starts (cleared by the demo)
vfhi    .fill VSIZE
phr     .fill VSIZE     ; 1: hard restart before the pending note     ; frequency high byte written (for the demo)
tick    .byte 0
curspd  .byte 0
spdidx  .byte 0
rowlo   .byte 0         ; rows played since the start
rowhi   .byte 0
fptr    .byte 0         ; filter program
fcnt    .byte 0
fcutlo  .byte 0
fcuthi  .byte 0
fmode   .byte 0         ; $d418 bits 4-6
vol     .byte 0
fadespd .byte 0
fadecnt .byte 0
gpfilt  .byte 0         ; pending global commands (applied at the row start)
gpfade  .byte 0
gpsync  .byte 0
sync    .byte 0         ; last sync value from the patterns (for the demo)
ftmp    .byte 0
dlo     .byte 0
dhi     .byte 0
flo     .byte 0
fhi     .byte 0
mvars_end

mus_init
        lda #0
        tax
-       sta mvars,x
        inx
        bne -
        ldx #mvars_end-mvars-256
-       sta mvars+255,x
        dex
        bne -
        ldx #$18
-       sta $d400,x
        dex
        bpl -
        lda #15
        sta vol
        lda #$ff
        sta gpfade
        ldx #0
        jsr vinit
        ldx #7
        jsr vinit
        ldx #14
        jsr vinit
        lda speedtab
        sta curspd
        sec
        sbc #1
        sta tick                ; the first mus_play starts a row
        rts

vinit   lda #$fe
        sta gmask,x
        lda #<startpat          ; an empty pattern: the first fetch reads the order list
        sta pptrlo,x
        lda #>startpat
        sta pptrhi,x
        lda #1
        sta rows,x
        jmp fetch

; --- one frame -------------------------------------------------------------
mus_play
        ldy tick
        iny
        cpy curspd
        bcc +
        ldy #0
+       sty tick
        bne pl_voices
        ; a new row
        inc rowlo
        bne +
        inc rowhi
+       lda spdidx
        eor #1
        sta spdidx
        tay
        lda speedtab,y
        sta curspd
        lda gpfilt
        beq +
        sta fptr
        lda #0
        sta fcnt
        sta gpfilt
+       lda gpfade
        bmi +
        sta fadespd
        sta fadecnt
        lda #$ff
        sta gpfade
        lda fadespd
        bne +
        lda #15
        sta vol
+       lda gpsync
        beq pl_voices
        sta sync
        lda #0
        sta gpsync
pl_voices
        ldx #0
        jsr voice
        ldx #7
        jsr voice
        ldx #14
        jsr voice
        jmp filter

; --- voice X ---------------------------------------------------------------
voice   lda tick
        bne v_fetch
        lda pend,x
        beq +
        jsr apply
        jmp v_fx
+       dec rows,x
        jmp v_fx
v_fetch lda rows,x              ; the last row of the event:
        cmp #1
        bne v_fx
        lda tick
        cmp ftick,x             ; read the next event (each voice on its own frame)
        bne +
        jsr fetch
        jmp v_fx
+       lda curspd              ; HRT frames before it: hard restart
        sec
        sbc #HRT
        cmp tick
        bne v_fx
        lda phr,x
        beq v_fx
        lda #$fe
        sta gmask,x
        lda #0
        sta phr,x
        sta $d405,x
        sta $d406,x
        lda wave,x
        and #$fe
        sta $d404,x

v_fx    lda gtimer,x            ; automatic gate off
        beq +
        dec gtimer,x
        bne +
        lda #$fe
        sta gmask,x

+       ldy wtptr,x             ; wave table
        beq wt_done
wt_rd   lda wt_l,y
        cmp #$ff
        bne +
        lda wt_r,y              ; jump ($00 = stop)
        sta wtptr,x
        beq wt_done
        tay
        jmp wt_rd
+       cmp #0
        beq +
        sta wave,x
+       lda wt_r,y
        sta wtnote,x
        iny
        tya
        sta wtptr,x
wt_done

        lda wtnote,x            ; pitch
        bpl +
        and #$7f                ; absolute note: no slide or vibrato
        tay
        jmp freq_plain
+       cmp #$60                ; $60-$7f: -32..-1
        bcc +
        eor #$80
+       clc
        adc note,x
        sta ftmp

        lda slidelo,x           ; portamento towards 0
        ora slidehi,x
        beq sl_done
        lda slidehi,x
        bmi sl_neg
        lda slidelo,x
        sec
        sbc slspd,x
        sta slidelo,x
        lda slidehi,x
        sbc #0
        sta slidehi,x
        bcs sl_done
        bcc sl_zero
sl_neg  lda slidelo,x
        clc
        adc slspd,x
        sta slidelo,x
        lda slidehi,x
        adc #0
        sta slidehi,x
        bcc sl_done
sl_zero lda #0
        sta slidelo,x
        sta slidehi,x
sl_done

        ldy inst,x              ; vibrato
        lda vibdel,x
        beq +
        dec vibdel,x
        jmp vib_done
+       lda i_vibs,y            ; frames per half period (0 = no vibrato)
        beq vib_done
        lda vibdir,x
        bne +
        lda vibval,x
        clc
        adc i_vibd,y
        jmp ++
+       lda vibval,x
        sec
        sbc i_vibd,y
+       sta vibval,x
        dec vibcnt,x
        bne vib_done
        lda i_vibs,y
        sta vibcnt,x
        lda vibdir,x
        eor #1
        sta vibdir,x
vib_done

        ; fine offset = slide + vibrato (16-bit, 1/16 semitones)
        lda vibval,x
        sta dlo
        ora slidelo,x
        ora slidehi,x
        bne +
        ldy ftmp
        jmp freq_plain
+       lda #0
        bit dlo
        bpl +
        lda #$ff
+       sta dhi
        lda dlo
        clc
        adc slidelo,x
        sta dlo
        lda dhi
        adc slidehi,x
        sta dhi
        ; pitch = note * 16 + offset
        lda ftmp
        lsr
        lsr
        lsr
        lsr
        sta fhi
        lda ftmp
        asl
        asl
        asl
        asl
        clc
        adc dlo
        sta dlo
        lda fhi
        adc dhi
        sta dhi                 ; dhi:dlo = pitch
        lda dlo
        and #$0f
        sta ftmp                ; ftmp = fraction
        lda dlo                 ; y = whole note
        lsr dhi
        ror
        lsr dhi
        ror
        lsr dhi
        ror
        lsr dhi
        ror
        tay
        ; delta = f[n+1] - f[n]
        lda freqlo+1,y
        sec
        sbc freqlo,y
        sta dlo
        lda freqhi+1,y
        sbc freqhi,y
        sta dhi
        lda freqlo,y
        sta flo
        lda freqhi,y
        sta fhi
        ldy #4                  ; add delta * fraction / 16
-       lsr dhi
        ror dlo
        lda ftmp
        asl
        sta ftmp
        and #$10
        beq +
        lda flo
        clc
        adc dlo
        sta flo
        lda fhi
        adc dhi
        sta fhi
+       dey
        bne -
        lda flo
        sta $d400,x
        lda fhi
        sta $d401,x
        sta vfhi,x
        jmp pulse
freq_plain
        lda freqlo,y
        sta $d400,x
        lda freqhi,y
        sta $d401,x
        sta vfhi,x

pulse   ldy ptptr,x             ; pulse program
        beq pt_out
pt_rd   lda pt_l,y
        cmp #$ff
        bne +
        lda pt_r,y              ; jump ($00 = stop)
        sta ptptr,x
        beq pt_out
        tay
        jmp pt_rd
+       cmp #$80
        bcc pt_sweep
        and #$0f                ; $8h ll: pulse width = $hll
        sta pwhi,x
        lda pt_r,y
        sta pwlo,x
        iny
        tya
        sta ptptr,x
        jmp pt_out
pt_sweep
        lda pcnt,x              ; nn ss: nn frames of +ss (signed) per frame
        bne +
        lda pt_l,y
        sta pcnt,x
+       lda pt_r,y
        bmi +
        clc
        adc pwlo,x
        sta pwlo,x
        bcc pt_next
        inc pwhi,x
        jmp pt_next
+       clc
        adc pwlo,x
        sta pwlo,x
        bcs pt_next
        dec pwhi,x
pt_next lda pwhi,x
        and #$0f
        sta pwhi,x
        dec pcnt,x
        bne pt_out
        iny
        tya
        sta ptptr,x
pt_out  lda pwlo,x
        sta $d402,x
        lda pwhi,x
        sta $d403,x
        lda wave,x
        and gmask,x
        sta $d404,x
        rts

; --- reads the next event of voice X (HRT frames before its row) -----------
fetch   lda #0
        sta phr,x
        lda pptrlo,x
        sta MZP
        lda pptrhi,x
        sta MZP+1
        lda #0
        sta pflag,x
        ldy #0
f_next  lda (MZP),y
        iny
        cmp #$60
        bcs +
        jmp f_note
+       beq f_rest
        cmp #$61
        beq f_tie
        cmp #$ff
        beq f_endpat
        cmp #$80
        bcc f_next              ; ($62-$7f: unused)
        cmp #$c0
        bcs +
        and #$3f                ; $80-$bf: instrument
        sta pinst,x
        lda pflag,x
        ora #$80
        sta pflag,x
        bne f_next
+       cmp #$e0
        bcs f_cmd
        and #$1f                ; $c0-$df: duration 1-32 rows
        clc
        adc #1
        sta dur,x
        bne f_next
f_endpat
        jsr nextpat
        ldy #0
        beq f_next
f_cmd   cmp #$e0
        bne +
        lda pflag,x             ; $e0: legato (no new attack)
        ora #1
        sta pflag,x
        bne f_next
+       sta ftmp
        lda (MZP),y             ; commands with a parameter
        iny
        pha
        lda ftmp
        cmp #$e1
        bne +
        pla                     ; $e1 ss: portamento, ss/16 semitones per frame
        sta pspd,x
        lda pflag,x
        ora #3
        sta pflag,x
        bne f_next
+       cmp #$e2
        bne +
        pla                     ; $e2 pp: start the filter program pp
        sta gpfilt
        jmp f_next
+       cmp #$e3
        bne +
        pla                     ; $e3 ss: fade out, a step every ss frames (0: full volume)
        sta gpfade
        jmp f_next
+       pla                     ; $e4 vv: sync value for the demo
        sta gpsync
        jmp f_next
f_rest  lda #2
        bne f_set
f_tie   lda #3
        bne f_set
f_note  sta pnote,x
        lda #1
f_set   sta pend,x
        tya                     ; save the pattern pointer
        clc
        adc MZP
        sta pptrlo,x
        lda MZP+1
        adc #0
        sta pptrhi,x
        lda pend,x
        cmp #1
        bne f_done
        lda pflag,x
        lsr
        bcs f_done              ; legato: no hard restart
        ldy inst,x
        lda pflag,x
        bpl +
        ldy pinst,x
+       lda i_flags,y
        bmi f_done              ; bit 7: no hard restart
        lda #1
        sta phr,x
f_done  rts

; order list of voice X: the next pattern into MZP
nextpat lda ordlo,x
        sta MZP2
        lda ordhi,x
        sta MZP2+1
        ldy opos,x
np_rd   lda (MZP2),y
        cmp #$fe
        beq np_end
        bcs np_jump
        cmp #$80
        bcc np_pat
        sbc #$a0                ; $80-$bf: transposition -32..+31 (carry set)
        sta transp,x
        iny
        bne np_rd
np_jump iny                     ; $ff nn: go to position nn
        lda (MZP2),y
        tay
        jmp np_rd
np_end  lda #<endpat            ; $fe: end of the song
        sta MZP
        lda #>endpat
        sta MZP+1
        rts
np_pat  iny
        pha
        tya
        sta opos,x
        pla
        tay
        lda pat_lo,y
        sta MZP
        lda pat_hi,y
        sta MZP+1
        rts

; the frame of its last row on which each voice reads the next event
ftick   .byte 1, 0, 0, 0, 0, 0, 0, 2, 0, 0, 0, 0, 0, 0, 3

endpat  .byte $df, $60, $ff     ; 32 rows of rest
startpat .byte $ff

; --- starts the pending event of voice X (at its row's first frame) --------
apply   lda dur,x
        sta rows,x
        lda pend,x
        pha
        lda #0
        sta pend,x
        pla
        cmp #2
        bcc ap_note
        bne +
        lda #$fe                ; rest: gate off
        sta gmask,x
+       rts
ap_note lda pflag,x
        bpl +
        lda pinst,x
        sta inst,x
+       lda pnote,x
        clc
        adc transp,x
        sta ftmp
        lda pflag,x
        lsr
        bcc ap_new
        lsr                     ; legato
        bcc ap_leg
        lda note,x              ; portamento: slide += (old - new) * 16
        sec
        sbc ftmp
        sta dlo
        lda #0
        sbc #0
        sta dhi
        ldy #4
-       asl dlo
        rol dhi
        dey
        bne -
        lda slidelo,x
        clc
        adc dlo
        sta slidelo,x
        lda slidehi,x
        adc dhi
        sta slidehi,x
        lda pspd,x
        sta slspd,x
ap_leg  lda ftmp
        sta note,x
        lda #1
        sta trig,x
        rts
ap_new  lda ftmp
        sta note,x
        lda #0
        sta slidelo,x
        sta slidehi,x
        sta vibval,x
        sta vibdir,x
        ldy inst,x
        lda i_ad,y
        sta $d405,x
        lda i_sr,y
        sta $d406,x
        lda i_wave,y
        sta wtptr,x
        lda i_pulse,y
        beq +
        sta ptptr,x
        lda #0
        sta pcnt,x
+       lda i_filt,y
        beq +
        sta fptr
        lda #0
        sta fcnt
+       lda i_vdel,y
        sta vibdel,x
        lda i_vibs,y
        lsr
        adc #0
        sta vibcnt,x            ; the first quarter period: half a sweep
        lda i_gate,y
        sta gtimer,x
        lda #$ff
        sta gmask,x
        lda #1
        sta trig,x
        rts

; --- filter program and volume ---------------------------------------------
filter  ldy fptr
        beq fl_out
fl_rd   lda ft_l,y
        cmp #$ff
        bne +
        lda ft_r,y              ; jump ($00 = stop)
        sta fptr
        beq fl_out
        tay
        jmp fl_rd
+       cmp #$80
        bcc +
        and #$70                ; $80+mode rr: mode bits ($10 LP, $20 BP, $40 HP), $d417 = rr
        sta fmode
        lda ft_r,y
        sta $d417
        jmp fl_step
+       cmp #0
        bne fl_sweep
        lda ft_r,y              ; $00 cc: cutoff = cc
        sta fcuthi
        lda #0
        sta fcutlo
fl_step iny
        sty fptr
        jmp fl_out
fl_sweep
        lda fcnt                ; nn ss: nn frames of +ss/16 (signed) per frame
        bne +
        lda ft_l,y
        sta fcnt
+       lda ft_r,y
        asl
        asl
        asl
        asl
        clc
        adc fcutlo
        sta fcutlo
        lda ft_r,y
        php
        lsr
        lsr
        lsr
        lsr
        plp
        bpl +
        ora #$f0
+       adc fcuthi
        sta fcuthi
        dec fcnt
        bne fl_out
        iny
        sty fptr
fl_out  lda fcutlo
        lsr
        lsr
        lsr
        lsr
        lsr
        sta $d415
        lda fcuthi
        sta $d416
        lda fadespd             ; fade out
        beq fl_vol
        dec fadecnt
        bne fl_vol
        lda fadespd
        sta fadecnt
        lda vol
        beq fl_vol
        dec vol
fl_vol  lda vol
        ora fmode
        sta $d418
        rts
