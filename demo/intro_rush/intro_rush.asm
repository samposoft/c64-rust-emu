; intro_rush_full.asm - versione autonoma di intro_rush.asm (tutti gli include espansi)
; Assemblare con: 64tass intro_rush_full.asm -o intro_rush_full.prg

; RUSH - intro SAMPOSOFT con fulmini e musica SID (64tass, senza -a)
; Grafica e tabella effetti generate da gen_gfx.py, musica da ../sid/game_tune.
;
; Schermo: bitmap multicolor ($2000, colori in $0400/$D800) fino alla riga 22,
; poi split raster a riga testo per lo scroller (righe 23-24).
; Pioggia: 8 sprite espansi riusati su 4 fasce (multiplexing = 32 sprite),
; forme che scorrono verso il basso. Tremolio dello schermo sui lampi.
; IRQ B (linea BOTTOM, bordo basso): torna in bitmap (con tremolio), fascia 0
;   della pioggia, musica, fulmini sincronizzati, lampo, riflesso, scroller, pioggia.
; IRQ R (3 volte, a meta' delle fasce 0-2): sposta gli sprite sulla fascia successiva.
; IRQ A (linea SPLIT): passa a modo testo + scroll fine.

* = $0801
        .word (+), 2026
        .null $9e, format("%d", start)
+       .word 0

zp      = $fb           ; usati anche dal player
tmp     = $fd
tmp2    = $fe
tmp3    = $02
tp      = $03           ; puntatore al testo dello scroller

SPLIT   = 232           ; dentro la riga 22 (vuota): il cambio di modo non si vede
BOTTOM  = 252
RAINPTR = (rainspr & $3fff) / 64    ; primo blocco sprite della pioggia (253)

start   sei
        lda #$7f
        sta $dc0d
        sta $dd0d
        lda $dc0d
        lda $dd0d
        lda #$0b                ; schermo spento durante la preparazione
        sta $d011
        lda #0
        sta $d020
        sta $d021
        ldx #$18
-       sta $d400,x
        dex
        bpl -
        ldx #0
-       lda scrcol,x
        sta $0400,x
        lda scrcol+250,x
        sta $0400+250,x
        lda scrcol+500,x
        sta $0400+500,x
        lda scrcol+750,x
        sta $0400+750,x
        lda colram,x
        sta $d800,x
        lda colram+250,x
        sta $d800+250,x
        lda colram+500,x
        sta $d800+500,x
        lda colram+750,x
        sta $d800+750,x
        inx
        cpx #250
        bne -
        jsr init
        lda #<stext
        sta tp
        lda #>stext
        sta tp+1
        lda #7
        sta xs
        lda #0
        sta skyst
        sta shp
        sta shc
        sta shk
        ldx #3
-       sta bst,x
        dex
        bpl -
        ldx #7                  ; sprite della pioggia: X fisse, colori
-       lda sprx,x
        sta tmp
        txa
        asl
        tay
        lda tmp
        sta $d000,y
        lda sprc,x
        sta $d027,x
        dex
        bpl -
        lda #$c0                ; sprite 6 e 7 oltre X=255
        sta $d010
        lda #$ff                ; tutti accesi, espansi in X e Y, davanti alla grafica
        sta $d015
        sta $d017
        sta $d01d
        lda #0
        sta $d01c
        sta $d01b
        lda #$18                ; multicolor, 40 colonne / schermo $0400, bitmap $2000
        sta $d016
        sta $d018
        lda #<irqa
        sta $0314
        lda #>irqa
        sta $0315
        lda #SPLIT
        sta $d012
        lda #$3b                ; bitmap, schermo acceso
        sta $d011
        lda #$01
        sta $d019
        sta $d01a
        cli
main    jmp main

irqa    lda #$01
        sta $d019
        lda #$1b                ; modo testo
        sta $d011
        lda xs                  ; 40 colonne (come la bitmap) + scroll fine;
        ora #$08                ; la colonna 0 ha colore nero e nasconde l'uscita
        sta $d016
        lda #$14
        sta $d018
        lda #<irqb
        sta $0314
        lda #>irqb
        sta $0315
        lda #BOTTOM
        sta $d012
        jmp $ea81

; ack e prossimo IRQ impostati subito: se il lavoro sfora oltre la linea della
; fascia 1, l'IRQ resta in attesa in $D019 e parte appena finiamo
irqb    lda #$01
        sta $d019
        jsr shake               ; bitmap multicolor, con eventuale tremolio
        lda #$18
        sta $d018
        ldx #0
        jsr setband
        lda #1
        sta band
        lda #<irqr
        sta $0314
        lda #>irqr
        sta $0315
        lda rln+1
        sta $d012
        jsr play
        jsr fxrow
        jsr bolts
        jsr sky
        jsr shine
        jsr scroll
        jsr rain
        jmp $ea81

irqr    lda #$01
        sta $d019
        ldx band
        jsr setband
        inx
        stx band
        cpx #4
        bne +
        lda #<irqa
        sta $0314
        lda #>irqa
        sta $0315
        lda #SPLIT
        sta $d012
        jmp $ea81
+       lda rln,x
        sta $d012
        jmp $ea81

; fascia X della pioggia: Y di tutti gli sprite e forme (puntatori) della fascia
setband lda ryt,x
        .for i = 0, i < 8, i += 1
        sta $d001+i*2
        .next
        txa
        asl
        asl
        asl
        tay
        .for i = 0, i < 8, i += 1
        lda ptab+i,y
        sta $07f8+i
        .next
        rts

; tremolio: valori di scroll fine X/Y per qualche frame dopo un lampo
shake   ldy shk
        beq shn
        lda shky,y
        cmp #$ff
        bne +
        lda #0
        sta shk
        beq shn
+       ora #$38
        sta $d011
        lda shkx,y
        ora #$18
        sta $d016
        inc shk
        rts
shn     lda #$3b
        sta $d011
        lda #$18
        sta $d016
        rts

; pioggia: ruota le 3 forme di 2 righe verso il basso (4 linee con l'espansione)
rain    .for s = 0, s < 3, s += 1
        ldx #5
-       lda rainspr+s*64+57,x
        sta rsave,x
        dex
        bpl -
        .for i = 56, i >= 0, i -= 1
        lda rainspr+s*64+i
        sta rainspr+s*64+i+6
        .next
        ldx #5
-       lda rsave,x
        sta rainspr+s*64,x
        dex
        bpl -
        .next
        rts

; effetti della riga appena letta, 2 frame dopo (quando la nota suona davvero,
; dopo l'hard restart). fxtab: bit 0-3 = fulmini, bit 4 = lampo nel cielo
fxrow   lda tick
        cmp #3
        bne fxdone
        ldx ordpos              ; voce 0: battuta corrente = ordpos - 1
        dex
        lda fxlo,x
        sta zp
        lda fxhi,x
        sta zp+1
        lda row
        sec
        sbc #1
        and #15
        tay
        lda (zp),y
        beq fxdone
        sta tmp
        and #$10
        beq +
        lda #1
        sta skyst
        sta shk
+       ldx #0
-       lsr tmp
        bcc +
        lda #1
        sta bst,x
+       inx
        cpx #4
        bne -
fxdone  rts

; anima i fulmini: colore della color RAM delle loro celle da bseq
bolts   ldx #3
bl1     lda bst,x
        beq bl3
        tay
        lda bseq,y
        cmp #$ff
        bne bl2
        lda #0
        sta bst,x
        jsr drawbolt
        jmp bl3
bl2     inc bst,x
        jsr drawbolt
bl3     dex
        bpl bl1
        rts

drawbolt                        ; A = colore, X = fulmine
        pha
        lda boltlo,x
        sta bjmp+1
        lda bolthi,x
        sta bjmp+2
        pla
bjmp    jmp $0000

; lampo: colore di sfondo da skyseq
sky     ldy skyst
        beq skx
        lda skyseq,y
        cmp #$ff
        bne +
        lda #0
        sta skyst
+       sta $d021
        lda skyst
        beq skx
        inc skyst
skx     rts

; riflesso che attraversa il logo: nibble alto (colore 01) bianco/grigio chiaro
shine   inc shc
        lda shc
        and #1
        bne shx
        lda shp
        sec
        sbc #2
        tax
        cpx #40
        bcs +
        jsr restcol
+       ldx shp
        dex
        cpx #40
        bcs +
        lda #$f0
        jsr colset
+       ldx shp
        cpx #40
        bcs +
        lda #$10
        jsr colset
+       inc shp
        lda shp
        cmp #80                 ; 40 colonne di passaggio + pausa
        bcc shx
        lda #0
        sta shp
shx     rts

colset  sta tmp3
        .for r = LOGO_R0, r <= LOGO_R1, r += 1
        lda scrcol+r*40,x
        and #$0f
        ora tmp3
        sta $0400+r*40,x
        .next
        rts

restcol .for r = LOGO_R0, r <= LOGO_R1, r += 1
        lda scrcol+r*40,x
        sta $0400+r*40,x
        .next
        rts

; scroller sulla riga 23, 2 pixel per frame
scroll  lda xs
        sec
        sbc #2
        bpl scs
        clc
        adc #8
        sta xs
        ldx #0
-       lda $0400+23*40+1,x
        sta $0400+23*40,x
        inx
        cpx #39
        bne -
        ldy #0
        lda (tp),y
        bne +
        lda #<stext
        sta tp
        lda #>stext
        sta tp+1
        lda (tp),y
+       sta $0400+23*40+39
        inc tp
        bne +
        inc tp+1
+       rts
scs     sta xs
        rts

bseq    .byte 0, 1, 1, 1, 0, 0, 1, 1, 14, 14, 6, 6, 6, $ff
skyseq  .byte 0, 12, 12, 11, 0, 0, 12, 11, 11, 0, $ff

shkx    .byte 0, 4, 1, 6, 2, 5, 0, 3, 1, 2, 0, 1, 0, $ff
shky    .byte 3, 5, 1, 4, 2, 4, 3, 2, 4, 3, 3, 4, 3, $ff

ryt     .byte 45, 87, 129, 171          ; Y delle 4 fasce (42 linee ciascuna)
rln     .byte 0, 59, 101, 143           ; linea raster in cui spostare gli sprite sulla fascia
sprx    .byte 24, 64, 104, 144, 184, 224, 264-256, 304-256
sprc    .byte 15, 12, 14, 12, 15, 12, 14, 12
ptab    .for b = 0, b < 4, b += 1       ; forma per sprite e fascia, per non ripetere il disegno
        .for i = 0, i < 8, i += 1
        .byte RAINPTR + (i * 2 + b) % 3
        .next
        .next

xs      .byte 0
skyst   .byte 0
shp     .byte 0
shc     .byte 0
shk     .byte 0
band    .byte 0
bst     .fill 4, 0
rsave   .fill 6, 0

; ---- inizio player.asm ----
; player SID condiviso (init, play) - incluso da game_tune.asm e dalle intro

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
        sta flash
        sta $d415
        lda #$10
        sta fceil
        lda #$28
        sta fcut
        lda #$1f
        sta $d418
        rts

; --- un frame del player -----------------------------------------------------
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

; voce X: prende il prossimo pattern dall'orderlist ($FF,n = salta a n)
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

; voce X: legge una riga del pattern
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
        lda #2                  ; hard restart: 2 frame a gate chiuso, ADSR a 0
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

; voce X: effetti del frame e scrittura registri SID
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
        lda i_vsh,y             ; delta vibrato = (f(n+1)-f(n)) >> vsh
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
vs4     lda i_flash,y
        beq +
        sta $d020
        lda #4
        sta flash
+       lda i_ad,y
        sta tmp
        lda i_sr,y
        ldy sidoff,x
        sta $d406,y
        lda tmp
        sta $d405,y

vrun    ldy wtp,x               ; wavetable
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
+       lda vibp,x              ; triangolo: su 2, giu 4, su 2
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

vfw     ldy ins,x               ; PWM tra $300 e $E00
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

; filtro: inviluppo di cutoff per nota + apertura lenta nell'intro
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
        lda flash
        beq +
        dec flash
        bne +
        lda #0
        sta $d020
+       rts

sidoff  .byte 0, 7, 14
vbit    .byte 1, 2, 4

; frequenze PAL per nota MIDI 0..96
flo     .for n = 0, n < 97, n += 1
        .byte <(int(440.0 * 2.0**((n-69)/12.0) * 17.0284 + 0.5))
        .next
fhi     .for n = 0, n < 97, n += 1
        .byte >(int(440.0 * 2.0**((n-69)/12.0) * 17.0284 + 0.5))
        .next

; ---- fine player.asm ----
; ---- inizio tune_data.asm ----
; generato da gen_tune.py - non modificare a mano

SPEED   = 5

i_ad    .byte $00, $09, $05, $00, $00, $07, $07
i_sr    .byte $00, $a8, $a9, $f6, $f7, $5a, $5a
i_pw    .byte $00, $08, $04, $08, $08, $04, $04
i_pws   .byte $00, $18, $00, $00, $00, $10, $10
i_vdel  .byte $00, $0c, $00, $00, $00, $00, $00
i_vsh   .byte $00, $03, $00, $00, $00, $00, $00
i_flt   .byte $00, $00, $01, $00, $00, $00, $00
i_fst   .byte $00, $00, $b0, $00, $00, $00, $00
i_fdec  .byte $00, $00, $0a, $00, $00, $00, $00
i_flash .byte $00, $00, $00, $06, $0e, $00, $00
i_wt    .byte $00, $00, $03, $06, $0d, $14, $18

wtw     .byte $41, $41, $fe, $41, $21, $fe, $81, $41, $41, $41, $41, $40, $fe, $81, $41, $81, $81, $81, $80, $fe, $41, $41, $41, $ff, $41, $41, $41, $ff
wtn     .byte $0c, $00, $00, $0c, $00, $00, $bc, $ad, $a6, $a1, $9e, $9c, $00, $d4, $b8, $d0, $ce, $cc, $ca, $00, $00, $03, $07, $14, $00, $04, $07, $18

patlo   .byte <pat0, <pat1, <pat2, <pat3, <pat4, <pat5, <pat6, <pat7, <pat8, <pat9, <pat10, <pat11, <pat12, <pat13, <pat14, <pat15, <pat16, <pat17, <pat18, <pat19, <pat20, <pat21, <pat22, <pat23, <pat24, <pat25, <pat26, <pat27, <pat28, <pat29, <pat30, <pat31, <pat32, <pat33, <pat34, <pat35, <pat36, <pat37
pathi   .byte >pat0, >pat1, >pat2, >pat3, >pat4, >pat5, >pat6, >pat7, >pat8, >pat9, >pat10, >pat11, >pat12, >pat13, >pat14, >pat15, >pat16, >pat17, >pat18, >pat19, >pat20, >pat21, >pat22, >pat23, >pat24, >pat25, >pat26, >pat27, >pat28, >pat29, >pat30, >pat31, >pat32, >pat33, >pat34, >pat35, >pat36, >pat37
pat0    .byte $60, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00
pat1    .byte $e1, $4c, $00, $00, $00, $00, $00, $00, $00, $e1, $4f, $00, $00, $00, $00, $00, $00, $00
pat2    .byte $e1, $51, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00
pat3    .byte $e1, $45, $00, $e1, $4a, $00, $e1, $4d, $00, $e1, $51, $00, $00, $00, $00, $00, $e1, $4f, $00, $e1, $4d, $00
pat4    .byte $e1, $4f, $00, $00, $00, $e1, $4d, $00, $e1, $4a, $00, $e1, $4d, $00, $00, $00, $00, $00, $00, $00
pat5    .byte $e1, $4c, $00, $e1, $48, $00, $e1, $4c, $00, $e1, $4f, $00, $00, $00, $00, $00, $e1, $4d, $00, $e1, $4c, $00
pat6    .byte $e1, $4d, $00, $00, $00, $e1, $4c, $00, $e1, $48, $00, $e1, $4a, $00, $00, $00, $00, $00, $00, $00
pat7    .byte $e1, $45, $00, $e1, $4a, $00, $e1, $4d, $00, $e1, $51, $00, $00, $00, $00, $00, $e1, $4f, $00, $e1, $51, $00
pat8    .byte $e1, $52, $00, $00, $00, $e1, $51, $00, $e1, $4f, $00, $e1, $4d, $00, $00, $00, $e1, $4a, $00, $e1, $4d, $00
pat9    .byte $e1, $4f, $00, $e1, $4c, $00, $e1, $48, $00, $e1, $4f, $00, $e1, $51, $00, $00, $00, $e1, $4f, $00, $e1, $4c, $00
pat10   .byte $e1, $4c, $00, $00, $00, $e1, $49, $00, $e1, $4c, $00, $e1, $51, $00, $00, $00, $00, $00, $00, $00
pat11   .byte $e1, $56, $00, $00, $e1, $54, $00, $00, $e1, $52, $00, $e1, $51, $00, $e1, $4f, $00, $e1, $52, $00, $00, $00
pat12   .byte $e1, $51, $00, $00, $e1, $4f, $00, $00, $e1, $4d, $00, $e1, $4c, $00, $e1, $4d, $00, $e1, $51, $00, $00, $00
pat13   .byte $e1, $4d, $00, $e1, $4f, $00, $e1, $51, $00, $e1, $52, $00, $e1, $56, $00, $00, $00, $e1, $54, $00, $e1, $52, $00
pat14   .byte $e1, $51, $00, $00, $00, $00, $00, $e1, $4f, $00, $e1, $4c, $00, $e1, $49, $00, $e1, $4c, $00, $00, $00
pat15   .byte $e1, $51, $00, $00, $e1, $4f, $00, $00, $e1, $4d, $00, $e1, $4c, $00, $e1, $4a, $00, $e1, $4d, $00, $00, $00
pat16   .byte $e1, $4a, $00, $e1, $4d, $00, $e1, $52, $00, $e1, $56, $00, $e1, $59, $00, $00, $00, $e1, $58, $00, $e1, $56, $00
pat17   .byte $e1, $58, $00, $00, $00, $e1, $55, $00, $00, $00, $e1, $51, $00, $00, $00, $00, $00, $00, $00
pat18   .byte $e3, $3c, $00, $e2, $26, $e2, $32, $e2, $26, $00, $e2, $26, $e2, $32, $e3, $3c, $00, $e2, $26, $e2, $32, $e2, $26, $00, $e2, $26, $e2, $32
pat19   .byte $e3, $3c, $00, $e2, $22, $e2, $2e, $e2, $22, $00, $e2, $22, $e2, $2e, $e3, $3c, $00, $e2, $22, $e2, $2e, $e2, $22, $00, $e2, $22, $e2, $2e
pat20   .byte $e3, $3c, $00, $e2, $24, $e2, $30, $e4, $3c, $00, $e2, $24, $e2, $30, $e3, $3c, $00, $e2, $24, $e2, $30, $e4, $3c, $00, $e2, $24, $e2, $30
pat21   .byte $e3, $3c, $00, $e2, $21, $e2, $2d, $e4, $3c, $00, $e2, $21, $e2, $2d, $e3, $3c, $00, $e2, $21, $e2, $2d, $e4, $3c, $e4, $3c, $e4, $3c, $e4, $3c
pat22   .byte $e3, $3c, $00, $e2, $26, $e2, $32, $e4, $3c, $00, $e2, $26, $e2, $32, $e3, $3c, $00, $e2, $26, $e2, $32, $e4, $3c, $00, $e2, $26, $e2, $32
pat23   .byte $e3, $3c, $00, $e2, $22, $e2, $2e, $e4, $3c, $00, $e2, $22, $e2, $2e, $e3, $3c, $00, $e2, $22, $e2, $2e, $e4, $3c, $00, $e2, $22, $e2, $2e
pat24   .byte $e3, $3c, $00, $e2, $1f, $e2, $2b, $e4, $3c, $00, $e2, $1f, $e2, $2b, $e3, $3c, $00, $e2, $1f, $e2, $2b, $e4, $3c, $00, $e2, $1f, $e2, $2b
pat25   .byte $e3, $3c, $00, $e2, $21, $e2, $2d, $e4, $3c, $00, $e2, $21, $e2, $2d, $e3, $3c, $00, $e2, $21, $e2, $2d, $e4, $3c, $00, $e2, $21, $e2, $2d
pat26   .byte $e5, $3e, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00
pat27   .byte $e6, $3a, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00
pat28   .byte $e6, $3c, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00
pat29   .byte $e6, $39, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00
pat30   .byte $e5, $3e, $00, $00, $e5, $3e, $00, $00, $e5, $3e, $00, $e5, $3e, $00, $00, $e5, $3e, $00, $00, $e5, $3e, $00
pat31   .byte $e6, $3a, $00, $00, $e6, $3a, $00, $00, $e6, $3a, $00, $e6, $3a, $00, $00, $e6, $3a, $00, $00, $e6, $3a, $00
pat32   .byte $e6, $3c, $00, $00, $e6, $3c, $00, $00, $e6, $3c, $00, $e6, $3c, $00, $00, $e6, $3c, $00, $00, $e6, $3c, $00
pat33   .byte $e6, $39, $00, $00, $e6, $39, $00, $00, $e6, $39, $00, $e6, $39, $00, $00, $e6, $39, $00, $00, $e6, $39, $00
pat34   .byte $e5, $37, $00, $e5, $37, $00, $e5, $37, $00, $e5, $37, $00, $e5, $37, $00, $e5, $37, $00, $e5, $37, $00, $e5, $37, $00
pat35   .byte $e5, $3e, $00, $e5, $3e, $00, $e5, $3e, $00, $e5, $3e, $00, $e5, $3e, $00, $e5, $3e, $00, $e5, $3e, $00, $e5, $3e, $00
pat36   .byte $e6, $3a, $00, $e6, $3a, $00, $e6, $3a, $00, $e6, $3a, $00, $e6, $3a, $00, $e6, $3a, $00, $e6, $3a, $00, $e6, $3a, $00
pat37   .byte $e6, $39, $00, $e6, $39, $00, $e6, $39, $00, $e6, $39, $00, $e6, $39, $00, $e6, $39, $00, $e6, $39, $00, $e6, $39, $00

ord0    .byte $00, $00, $01, $02, $03, $04, $05, $06, $07, $08, $09, $0a, $0b, $0c, $0d, $0e, $0b, $0f, $10, $11, $ff, $00
ord1    .byte $12, $13, $14, $15, $16, $17, $14, $16, $16, $17, $14, $15, $18, $16, $17, $19, $18, $16, $17, $15, $ff, $00
ord2    .byte $1a, $1b, $1c, $1d, $1e, $1f, $20, $1e, $1e, $1f, $20, $21, $22, $23, $24, $25, $22, $23, $24, $25, $ff, $00
ordlo   .byte <ord0, <ord1, <ord2
ordhi   .byte >ord0, >ord1, >ord2
; ---- fine tune_data.asm ----
; ---- inizio player_vars.asm ----
; stato del player
tick    .byte 0
row     .byte 0
fcut    .byte 0
fceil   .byte 0
fdec    .byte 0
froute  .byte 0
flash   .byte 0
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
; ---- fine player_vars.asm ----

        .enc "screen"
stext   .text "          *** SAMPOSOFT PRESENTA: RUSH IN D MINOR ***"
        .text "     MUSICA SID CON HARD RESTART, BATTERIA A WAVETABLE,"
        .text " LEAD PWM CON VIBRATO, BASSO FILTRATO E ARPEGGI A 50 HZ..."
        .text "     FULMINI SINCRONIZZATI CON LA MUSICA..."
        .text "     GRAFICA BITMAP MULTICOLOR CON SPLIT RASTER PER LO SCROLLER..."
        .text "     PIOGGIA CON 8 SPRITE RIUSATI SU 4 FASCE: 32 SPRITE A SCHERMO..."
        .text "     E LO SCHERMO TREMA A OGNI TUONO!"
        .text "     SALUTI A TUTTI GLI AMICI DEL COMMODORE 64!"
        .text "                                        "
        .byte 0
        .enc "none"

        .cerror * > $2000, "il codice sfora nella bitmap a $2000"

* = $2000
; ---- inizio gfx_data.asm ----
; generato da gen_gfx.py - non modificare a mano

LOGO_R0 = 7
LOGO_R1 = 12

bitmap  .byte $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00
        .byte $00, $03, $03, $03, $03, $03, $03, $03, $c0, $00, $00, $00, $00, $00, $00, $00
        .byte $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00
        .byte $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00
        .byte $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00
        .byte $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00
        .byte $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00
        .byte $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00
        .byte $30, $30, $30, $30, $30, $30, $0c, $0c, $00, $00, $00, $00, $00, $00, $00, $00
        .byte $00, $00, $00, $00, $00, $00, $10, $00, $00, $00, $00, $00, $00, $00, $00, $00
        .byte $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00
        .byte $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00
        .byte $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00
        .byte $0c, $0c, $0c, $0c, $0c, $30, $30, $30, $00, $00, $00, $00, $00, $00, $00, $00
        .byte $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00
        .byte $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00
        .byte $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00
        .byte $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00
        .byte $c0, $c0, $c0, $c0, $c0, $c0, $c0, $c0, $00, $00, $00, $00, $00, $00, $00, $00
        .byte $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00
        .byte $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00
        .byte $03, $03, $03, $03, $03, $00, $00, $00, $00, $00, $00, $00, $00, $c0, $c0, $c0
        .byte $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00
        .byte $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00
        .byte $00, $00, $00, $00, $00, $00, $10, $00, $00, $00, $00, $00, $00, $00, $00, $00
        .byte $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00
        .byte $00, $00, $00, $00, $10, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00
        .byte $00, $00, $00, $00, $00, $00, $00, $40, $00, $00, $00, $00, $00, $00, $00, $00
        .byte $0c, $0c, $0c, $03, $03, $03, $00, $00, $00, $00, $00, $00, $00, $00, $c4, $c0
        .byte $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00
        .byte $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00
        .byte $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00
        .byte $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00
        .byte $0c, $0c, $0c, $0c, $0c, $03, $03, $03, $00, $00, $00, $00, $00, $00, $00, $00
        .byte $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00
        .byte $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00
        .byte $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00
        .byte $00, $00, $00, $00, $00, $00, $00, $00, $00, $03, $03, $03, $03, $03, $03, $03
        .byte $c0, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00
        .byte $00, $00, $00, $00, $00, $00, $00, $00, $10, $00, $00, $00, $00, $00, $00, $00
        .byte $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00
        .byte $03, $03, $03, $03, $0c, $0c, $0c, $30, $00, $00, $00, $00, $00, $00, $00, $00
        .byte $00, $00, $00, $00, $00, $00, $10, $00, $00, $00, $00, $00, $00, $00, $00, $00
        .byte $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00
        .byte $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00
        .byte $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00
        .byte $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00
        .byte $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00
        .byte $00, $00, $00, $00, $00, $00, $00, $00, $30, $30, $30, $30, $30, $0c, $0c, $0c
        .byte $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00
        .byte $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $01
        .byte $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00
        .byte $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00
        .byte $0c, $0c, $0c, $30, $30, $30, $30, $30, $00, $00, $00, $00, $00, $00, $00, $00
        .byte $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00
        .byte $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00
        .byte $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00
        .byte $00, $00, $00, $00, $00, $00, $00, $00, $03, $03, $0c, $0c, $30, $0c, $0c, $0c
        .byte $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00
        .byte $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00
        .byte $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00
        .byte $30, $30, $30, $30, $30, $30, $30, $30, $00, $00, $00, $00, $00, $00, $00, $00
        .byte $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00
        .byte $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00
        .byte $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00
        .byte $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $40, $00
        .byte $00, $00, $00, $04, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00
        .byte $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00
        .byte $00, $00, $00, $00, $00, $00, $00, $00, $03, $03, $03, $03, $00, $00, $00, $00
        .byte $00, $00, $00, $00, $c0, $c0, $c0, $c0, $00, $00, $00, $00, $00, $00, $00, $00
        .byte $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00
        .byte $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00
        .byte $00, $00, $00, $00, $00, $00, $00, $00, $00, $03, $03, $03, $03, $03, $03, $03
        .byte $c0, $00, $40, $00, $00, $c0, $3c, $03, $00, $00, $00, $00, $00, $00, $00, $c0
        .byte $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00
        .byte $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00
        .byte $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00
        .byte $00, $00, $00, $00, $00, $00, $00, $00, $0c, $0c, $0c, $03, $03, $03, $03, $03
        .byte $00, $00, $00, $00, $00, $c0, $c0, $c0, $00, $00, $00, $00, $00, $00, $00, $00
        .byte $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00
        .byte $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00
        .byte $30, $30, $30, $c0, $c0, $c0, $c0, $c0, $00, $00, $00, $00, $00, $00, $00, $00
        .byte $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00
        .byte $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00
        .byte $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00
        .byte $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00
        .byte $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00
        .byte $00, $01, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00
        .byte $00, $00, $00, $00, $00, $00, $00, $00, $00, $03, $03, $03, $03, $03, $03, $03
        .byte $c0, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00
        .byte $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00
        .byte $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00
        .byte $00, $00, $00, $00, $00, $00, $00, $00, $03, $03, $03, $00, $00, $00, $00, $00
        .byte $00, $00, $00, $c0, $c0, $c0, $c0, $c0, $30, $30, $0c, $0c, $0c, $0c, $0c, $03
        .byte $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00
        .byte $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00
        .byte $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00
        .byte $00, $00, $00, $00, $00, $00, $00, $00, $03, $03, $03, $03, $03, $0c, $0c, $0c
        .byte $30, $0c, $03, $03, $00, $03, $03, $3c, $00, $00, $00, $00, $c0, $00, $00, $00
        .byte $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00
        .byte $00, $00, $00, $00, $00, $00, $00, $00, $00, $03, $03, $00, $00, $00, $00, $00
        .byte $c0, $00, $00, $c0, $c0, $c0, $30, $30, $00, $00, $00, $00, $00, $00, $00, $00
        .byte $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00
        .byte $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00
        .byte $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00
        .byte $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00
        .byte $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00
        .byte $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00
        .byte $00, $00, $00, $00, $00, $00, $00, $00, $03, $03, $0c, $0c, $0c, $0c, $0c, $0c
        .byte $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00
        .byte $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00
        .byte $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00
        .byte $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $40, $00, $00, $00
        .byte $c0, $c0, $c0, $c0, $c0, $c0, $c0, $c0, $03, $00, $00, $00, $03, $03, $03, $03
        .byte $00, $c0, $c0, $c0, $00, $00, $c0, $30, $00, $00, $00, $00, $00, $00, $00, $00
        .byte $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00
        .byte $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00
        .byte $00, $00, $00, $00, $00, $00, $00, $00, $0c, $03, $03, $03, $03, $03, $03, $03
        .byte $3f, $33, $c3, $cc, $cc, $cc, $33, $30, $00, $00, $00, $00, $00, $00, $00, $c0
        .byte $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00
        .byte $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $03, $03, $0c, $0c, $0c, $03
        .byte $f0, $f0, $30, $30, $0c, $0c, $0c, $0c, $00, $00, $00, $00, $00, $00, $00, $00
        .byte $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00
        .byte $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00
        .byte $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $04, $00, $00, $00, $00, $00
        .byte $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00
        .byte $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00
        .byte $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00
        .byte $00, $00, $00, $00, $00, $00, $00, $00, $03, $03, $13, $03, $03, $00, $00, $03
        .byte $00, $00, $00, $00, $00, $c0, $c0, $00, $00, $00, $00, $00, $00, $00, $00, $00
        .byte $00, $00, $00, $10, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00
        .byte $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00
        .byte $00, $00, $00, $00, $00, $00, $00, $00, $00, $03, $03, $03, $00, $00, $00, $00
        .byte $c0, $00, $00, $00, $c0, $c0, $c0, $c0, $03, $0c, $0c, $30, $c0, $c0, $30, $30
        .byte $0f, $00, $00, $00, $00, $00, $00, $00, $00, $c0, $c0, $c0, $30, $30, $30, $30
        .byte $00, $00, $10, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00
        .byte $01, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00
        .byte $00, $00, $00, $00, $00, $00, $03, $00, $03, $03, $03, $03, $3c, $c0, $00, $00
        .byte $30, $30, $30, $0c, $0c, $0c, $0c, $03, $3c, $03, $00, $00, $00, $00, $00, $00
        .byte $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00
        .byte $00, $00, $00, $00, $00, $00, $00, $00, $03, $03, $0c, $0c, $0c, $0c, $03, $03
        .byte $0c, $02, $0a, $29, $25, $25, $a5, $95, $2a, $a5, $55, $55, $55, $55, $55, $55
        .byte $aa, $55, $55, $55, $55, $55, $55, $55, $80, $a0, $68, $58, $5a, $56, $56, $56
        .byte $00, $02, $02, $0a, $09, $09, $09, $89, $00, $aa, $55, $55, $55, $55, $55, $55
        .byte $00, $aa, $55, $55, $55, $55, $55, $55, $00, $a0, $60, $60, $68, $68, $68, $68
        .byte $00, $2a, $25, $25, $a5, $95, $95, $95, $00, $aa, $55, $55, $55, $55, $55, $55
        .byte $00, $a8, $58, $58, $5a, $5a, $5a, $5a, $00, $aa, $95, $95, $95, $95, $55, $55
        .byte $00, $aa, $55, $55, $55, $55, $55, $55, $00, $aa, $5a, $5a, $6a, $6a, $6a, $69
        .byte $00, $aa, $55, $55, $55, $55, $55, $55, $03, $aa, $55, $55, $55, $55, $55, $55
        .byte $00, $aa, $56, $55, $55, $55, $55, $55, $00, $00, $80, $80, $a2, $62, $62, $6a
        .byte $02, $0a, $29, $a5, $95, $55, $55, $55, $aa, $55, $55, $55, $55, $55, $55, $55
        .byte $a8, $5a, $56, $55, $55, $55, $55, $55, $00, $00, $80, $80, $a0, $62, $62, $6a
        .byte $02, $0a, $29, $a5, $95, $95, $55, $55, $aa, $55, $55, $55, $55, $55, $55, $55
        .byte $a8, $5a, $55, $55, $55, $55, $55, $55, $0f, $80, $80, $a3, $63, $63, $6a, $6a
        .byte $00, $c2, $ca, $29, $a5, $95, $95, $55, $aa, $95, $55, $55, $55, $55, $55, $55
        .byte $aa, $56, $55, $55, $55, $55, $55, $55, $00, $80, $a0, $60, $68, $58, $58, $5a
        .byte $00, $2a, $a5, $95, $95, $95, $95, $95, $00, $aa, $55, $55, $55, $55, $55, $55
        .byte $00, $aa, $55, $55, $55, $55, $55, $55, $00, $aa, $55, $55, $55, $55, $55, $55
        .byte $00, $aa, $55, $55, $55, $55, $55, $55, $c0, $aa, $55, $55, $55, $55, $55, $55
        .byte $00, $aa, $56, $56, $56, $56, $5a, $5a, $00, $00, $00, $00, $80, $80, $80, $80
        .byte $00, $00, $00, $00, $00, $00, $00, $00, $03, $00, $00, $02, $02, $02, $02, $02
        .byte $95, $95, $95, $95, $55, $55, $55, $55, $55, $56, $56, $56, $56, $56, $56, $56
        .byte $55, $95, $95, $95, $95, $55, $55, $aa, $56, $56, $56, $56, $56, $56, $5a, $aa
        .byte $a9, $a5, $a5, $a5, $a5, $a5, $a5, $95, $55, $55, $55, $55, $55, $55, $55, $55
        .byte $55, $55, $55, $55, $55, $55, $55, $55, $68, $68, $68, $68, $68, $68, $6a, $6a
        .byte $95, $95, $95, $95, $95, $95, $95, $55, $55, $55, $55, $55, $55, $55, $55, $55
        .byte $5a, $5a, $5a, $5a, $59, $59, $59, $59, $55, $55, $55, $55, $55, $55, $55, $55
        .byte $55, $55, $55, $55, $55, $55, $55, $55, $69, $69, $69, $69, $69, $69, $a9, $a9
        .byte $55, $55, $55, $55, $55, $55, $55, $55, $55, $59, $59, $59, $59, $69, $69, $69
        .byte $55, $55, $55, $55, $55, $55, $55, $55, $69, $69, $69, $69, $65, $65, $65, $65
        .byte $55, $55, $55, $55, $55, $55, $55, $55, $59, $69, $69, $69, $69, $69, $69, $65
        .byte $55, $55, $55, $55, $55, $55, $55, $55, $6a, $69, $69, $69, $69, $69, $69, $69
        .byte $55, $55, $55, $55, $55, $55, $55, $55, $59, $59, $69, $69, $69, $69, $69, $5a
        .byte $55, $55, $55, $55, $55, $55, $55, $aa, $6a, $6a, $6a, $69, $69, $69, $69, $a9
        .byte $55, $55, $55, $55, $55, $55, $55, $55, $55, $5a, $5a, $5a, $5a, $5a, $59, $59
        .byte $55, $55, $55, $55, $55, $55, $55, $55, $5a, $5a, $5a, $5a, $5a, $5a, $5a, $5a
        .byte $95, $95, $95, $95, $95, $55, $55, $55, $55, $55, $55, $56, $56, $56, $56, $56
        .byte $55, $55, $aa, $aa, $aa, $a0, $80, $80, $55, $55, $99, $aa, $a9, $09, $09, $09
        .byte $55, $55, $55, $55, $55, $55, $55, $55, $55, $55, $56, $5a, $5a, $5a, $5a, $5a
        .byte $5a, $5a, $6a, $aa, $aa, $a8, $30, $0c, $80, $00, $00, $00, $00, $00, $00, $00
        .byte $00, $00, $00, $00, $00, $00, $00, $00, $02, $02, $02, $02, $02, $00, $00, $00
        .byte $55, $55, $55, $55, $95, $95, $a5, $25, $55, $55, $55, $55, $55, $55, $55, $55
        .byte $aa, $6a, $68, $5a, $56, $56, $55, $55, $aa, $aa, $00, $02, $02, $82, $a2, $62
        .byte $95, $95, $95, $95, $55, $55, $55, $55, $55, $55, $55, $59, $59, $59, $59, $59
        .byte $55, $55, $55, $55, $55, $55, $55, $55, $6a, $6a, $6a, $6a, $6a, $6a, $6a, $6a
        .byte $55, $55, $55, $55, $55, $55, $55, $55, $55, $55, $55, $55, $55, $55, $55, $55
        .byte $59, $55, $55, $55, $55, $55, $55, $55, $55, $55, $55, $55, $55, $55, $55, $55
        .byte $55, $55, $55, $55, $55, $55, $55, $56, $a5, $a5, $a5, $a5, $a5, $a5, $a5, $a5
        .byte $55, $55, $55, $55, $55, $55, $55, $55, $69, $69, $69, $65, $55, $55, $55, $55
        .byte $55, $55, $55, $55, $55, $55, $55, $56, $a5, $a5, $a5, $a5, $a5, $95, $95, $95
        .byte $55, $55, $55, $55, $55, $55, $55, $55, $65, $a5, $a5, $a5, $a5, $a5, $a5, $a5
        .byte $55, $55, $55, $55, $55, $55, $55, $55, $69, $69, $69, $69, $69, $aa, $aa, $aa
        .byte $55, $55, $55, $55, $55, $55, $55, $95, $5a, $56, $55, $55, $55, $55, $55, $55
        .byte $aa, $aa, $a0, $60, $68, $5a, $56, $56, $a9, $a9, $09, $29, $25, $25, $25, $25
        .byte $55, $55, $55, $55, $55, $55, $55, $55, $69, $69, $69, $69, $69, $69, $69, $65
        .byte $55, $55, $55, $55, $55, $55, $55, $55, $5a, $5a, $5a, $5a, $6a, $6a, $69, $69
        .byte $55, $55, $55, $55, $55, $55, $55, $55, $56, $56, $55, $55, $55, $55, $55, $55
        .byte $80, $a8, $58, $58, $5a, $5a, $5a, $5a, $09, $09, $09, $09, $09, $29, $25, $25
        .byte $55, $55, $55, $55, $55, $55, $55, $55, $5a, $5a, $6a, $6a, $6a, $68, $68, $68
        .byte $0c, $03, $03, $0c, $0c, $0c, $0c, $0c, $00, $00, $00, $00, $00, $00, $00, $00
        .byte $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $0a, $09, $09, $09
        .byte $29, $0a, $02, $03, $aa, $55, $55, $55, $55, $55, $95, $95, $a5, $65, $69, $69
        .byte $55, $55, $55, $55, $55, $55, $55, $55, $62, $6a, $69, $69, $69, $69, $69, $65
        .byte $55, $55, $55, $55, $55, $55, $55, $55, $59, $69, $69, $69, $69, $55, $55, $55
        .byte $55, $55, $55, $55, $55, $55, $55, $55, $69, $69, $69, $69, $69, $69, $69, $69
        .byte $55, $55, $55, $55, $55, $55, $55, $55, $65, $65, $65, $65, $65, $65, $65, $a5
        .byte $55, $55, $55, $55, $55, $55, $55, $55, $65, $55, $55, $95, $95, $95, $95, $95
        .byte $56, $56, $56, $56, $56, $56, $56, $56, $a5, $a5, $95, $95, $95, $95, $95, $95
        .byte $55, $55, $55, $55, $55, $55, $55, $55, $55, $55, $55, $aa, $aa, $aa, $a0, $a0
        .byte $56, $5a, $aa, $aa, $aa, $a0, $02, $02, $95, $95, $95, $95, $95, $95, $95, $55
        .byte $55, $55, $55, $56, $56, $56, $56, $56, $a5, $95, $95, $95, $95, $95, $95, $95
        .byte $55, $55, $55, $55, $55, $55, $56, $56, $a0, $a0, $a0, $a0, $aa, $95, $95, $95
        .byte $a5, $25, $29, $0a, $aa, $55, $56, $56, $55, $55, $55, $55, $55, $95, $95, $95
        .byte $56, $55, $55, $55, $55, $55, $55, $55, $a5, $a5, $a5, $a5, $a5, $a5, $95, $95
        .byte $55, $55, $55, $55, $55, $55, $55, $55, $65, $65, $a5, $a5, $a5, $a5, $a5, $a5
        .byte $55, $55, $55, $55, $55, $55, $55, $55, $69, $69, $69, $69, $69, $a9, $a9, $a9
        .byte $55, $55, $55, $55, $55, $55, $55, $55, $55, $55, $55, $5a, $6a, $6a, $6a, $68
        .byte $5a, $5a, $6a, $aa, $aa, $a8, $00, $00, $25, $25, $25, $25, $25, $25, $a5, $95
        .byte $55, $55, $55, $55, $55, $55, $55, $55, $68, $68, $68, $a8, $a8, $a8, $a0, $a0
        .byte $0c, $0c, $0c, $03, $03, $03, $00, $00, $00, $00, $00, $00, $00, $00, $c0, $c0
        .byte $00, $00, $00, $00, $00, $00, $00, $00, $29, $25, $25, $25, $25, $25, $25, $25
        .byte $55, $55, $55, $55, $55, $55, $55, $55, $69, $69, $a9, $a5, $a5, $65, $55, $55
        .byte $55, $55, $55, $55, $55, $55, $55, $55, $65, $65, $65, $65, $65, $95, $95, $95
        .byte $55, $55, $55, $55, $55, $55, $55, $55, $55, $55, $55, $55, $55, $a5, $a5, $a5
        .byte $55, $55, $55, $55, $55, $55, $55, $55, $69, $69, $65, $65, $65, $65, $65, $65
        .byte $55, $55, $55, $55, $55, $55, $55, $55, $a5, $a5, $a5, $a5, $a5, $95, $95, $95
        .byte $55, $56, $56, $56, $56, $56, $56, $5a, $95, $95, $95, $55, $55, $55, $55, $55
        .byte $56, $5a, $5a, $5a, $5a, $5a, $5a, $5a, $95, $95, $95, $55, $55, $55, $55, $55
        .byte $56, $56, $56, $56, $56, $56, $56, $56, $ac, $ac, $af, $8c, $83, $83, $80, $80
        .byte $02, $02, $02, $c2, $c2, $32, $ce, $3e, $55, $55, $55, $55, $55, $55, $55, $55
        .byte $56, $56, $56, $56, $56, $56, $55, $55, $95, $95, $55, $55, $55, $55, $55, $55
        .byte $56, $56, $56, $56, $56, $5a, $5a, $5a, $95, $95, $95, $95, $95, $95, $95, $95
        .byte $56, $56, $56, $56, $56, $56, $55, $55, $95, $95, $95, $95, $95, $55, $55, $55
        .byte $55, $55, $56, $56, $56, $56, $56, $5a, $95, $95, $95, $95, $95, $95, $95, $95
        .byte $55, $55, $55, $55, $56, $55, $55, $55, $a5, $95, $95, $95, $95, $55, $55, $55
        .byte $55, $55, $55, $55, $56, $56, $56, $56, $a5, $a5, $a5, $a5, $a5, $a5, $a5, $a5
        .byte $55, $55, $55, $55, $55, $55, $55, $55, $68, $68, $68, $68, $68, $68, $a8, $a8
        .byte $00, $00, $00, $00, $00, $00, $00, $00, $95, $95, $95, $95, $95, $95, $95, $95
        .byte $55, $55, $55, $55, $55, $56, $56, $56, $a0, $a0, $a0, $a0, $a0, $a0, $a0, $a0
        .byte $03, $03, $03, $03, $03, $03, $03, $03, $00, $00, $00, $00, $00, $00, $00, $00
        .byte $00, $00, $00, $00, $00, $00, $00, $00, $29, $09, $0a, $02, $00, $00, $00, $00
        .byte $55, $55, $55, $95, $a5, $2a, $0a, $02, $55, $55, $55, $55, $55, $aa, $aa, $aa
        .byte $56, $56, $5a, $6a, $aa, $aa, $ab, $a3, $95, $95, $55, $55, $aa, $2a, $2a, $c0
        .byte $55, $56, $56, $56, $aa, $aa, $aa, $00, $a5, $a5, $a5, $a5, $aa, $82, $82, $00
        .byte $55, $55, $55, $55, $aa, $aa, $aa, $00, $65, $a5, $a5, $a5, $aa, $a2, $a2, $00
        .byte $55, $55, $56, $56, $aa, $aa, $aa, $00, $95, $95, $95, $95, $aa, $8a, $8a, $00
        .byte $5a, $5a, $5a, $5a, $aa, $aa, $aa, $00, $55, $55, $55, $55, $aa, $2a, $2a, $00
        .byte $5a, $5a, $5a, $6a, $aa, $aa, $a8, $00, $55, $55, $55, $55, $aa, $2a, $2a, $00
        .byte $56, $5a, $5a, $5a, $aa, $aa, $aa, $30, $80, $80, $80, $80, $00, $00, $00, $00
        .byte $32, $33, $0c, $0c, $03, $00, $00, $00, $95, $95, $a5, $29, $3a, $c2, $30, $00
        .byte $55, $55, $55, $55, $55, $aa, $aa, $2a, $55, $55, $55, $56, $6a, $aa, $aa, $a8
        .byte $6a, $6a, $aa, $a8, $a8, $a0, $80, $03, $95, $a5, $e5, $e9, $ca, $c2, $c0, $00
        .byte $55, $55, $55, $55, $55, $aa, $aa, $2a, $55, $55, $55, $55, $5a, $aa, $aa, $aa
        .byte $5a, $6a, $6a, $aa, $a8, $a8, $a0, $00, $a5, $a5, $29, $09, $0a, $00, $00, $00
        .byte $55, $55, $55, $55, $95, $aa, $aa, $0a, $55, $55, $55, $55, $5a, $aa, $aa, $aa
        .byte $5a, $6a, $6a, $aa, $a8, $a8, $a0, $00, $a5, $95, $95, $95, $aa, $0a, $0a, $00
        .byte $55, $55, $55, $55, $aa, $aa, $aa, $00, $a8, $a0, $a0, $a0, $a0, $a0, $a0, $00
        .byte $02, $02, $02, $02, $02, $00, $00, $00, $95, $55, $55, $55, $aa, $2a, $2a, $00
        .byte $56, $56, $56, $56, $aa, $aa, $aa, $00, $80, $80, $80, $80, $80, $80, $80, $00
        .byte $0c, $0c, $0c, $0c, $0c, $0c, $0c, $0c, $00, $00, $00, $00, $00, $00, $00, $00
        .byte $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00
        .byte $00, $00, $00, $04, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00
        .byte $00, $00, $00, $00, $00, $00, $00, $00, $f0, $cc, $f0, $f0, $30, $30, $3c, $0c
        .byte $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00
        .byte $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00
        .byte $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00
        .byte $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00
        .byte $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00
        .byte $0c, $0c, $0c, $03, $03, $03, $03, $03, $00, $00, $00, $00, $00, $00, $00, $00
        .byte $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $01, $00, $00, $00
        .byte $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00
        .byte $03, $03, $0c, $0c, $0f, $0f, $0c, $0c, $00, $00, $00, $00, $00, $00, $c0, $c0
        .byte $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00
        .byte $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00
        .byte $00, $00, $00, $00, $00, $00, $04, $00, $00, $00, $00, $00, $00, $00, $00, $00
        .byte $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00
        .byte $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00
        .byte $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00
        .byte $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00
        .byte $0c, $0c, $0c, $0c, $03, $03, $03, $03, $00, $00, $00, $00, $00, $00, $00, $00
        .byte $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00
        .byte $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00
        .byte $00, $00, $00, $00, $00, $00, $00, $00, $0c, $03, $03, $03, $03, $03, $0c, $0c
        .byte $00, $00, $c0, $c0, $c0, $30, $30, $30, $00, $00, $00, $00, $00, $00, $00, $00
        .byte $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00
        .byte $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00
        .byte $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00
        .byte $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00
        .byte $03, $03, $03, $03, $03, $03, $03, $03, $00, $00, $00, $00, $00, $00, $00, $00
        .byte $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00
        .byte $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00
        .byte $0c, $0c, $03, $03, $03, $03, $03, $03, $70, $0c, $03, $03, $0c, $0c, $30, $30
        .byte $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00
        .byte $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00
        .byte $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00
        .byte $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00
        .byte $00, $00, $00, $00, $00, $00, $00, $00, $00, $40, $00, $00, $00, $00, $00, $00
        .byte $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00
        .byte $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00
        .byte $00, $00, $00, $00, $00, $00, $00, $00, $c0, $c0, $30, $30, $30, $30, $30, $30
        .byte $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00
        .byte $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00
        .byte $00, $00, $00, $00, $00, $00, $00, $00, $15, $15, $17, $17, $17, $14, $15, $15
        .byte $50, $50, $14, $14, $14, $d4, $5c, $53, $14, $14, $14, $14, $14, $14, $14, $14
        .byte $14, $14, $14, $14, $14, $14, $14, $14, $05, $05, $14, $14, $14, $14, $05, $05
        .byte $50, $50, $14, $14, $00, $00, $50, $50, $14, $14, $14, $14, $14, $14, $15, $15
        .byte $14, $14, $14, $14, $14, $14, $54, $54, $00, $00, $00, $00, $00, $00, $00, $00
        .byte $00, $00, $00, $00, $00, $00, $00, $00, $05, $05, $01, $01, $01, $01, $01, $01
        .byte $53, $53, $43, $4c, $4c, $43, $43, $43, $14, $14, $15, $15, $15, $15, $15, $15
        .byte $14, $14, $14, $14, $54, $54, $54, $54, $00, $00, $00, $00, $00, $00, $00, $00
        .byte $00, $00, $00, $00, $00, $00, $00, $00, $15, $15, $14, $14, $14, $14, $14, $14
        .byte $4c, $4c, $5c, $53, $17, $14, $14, $14, $30, $0c, $0c, $0c, $03, $c3, $c3, $3c
        .byte $00, $00, $00, $00, $00, $00, $00, $00, $14, $14, $15, $15, $15, $15, $14, $14
        .byte $05, $05, $15, $15, $55, $55, $45, $45, $05, $05, $01, $01, $01, $01, $01, $01
        .byte $50, $50, $40, $40, $40, $40, $40, $40, $14, $14, $15, $15, $15, $15, $15, $15
        .byte $14, $14, $14, $14, $54, $54, $54, $54, $05, $05, $14, $14, $14, $14, $14, $14
        .byte $50, $50, $14, $14, $14, $14, $14, $14, $15, $15, $14, $14, $14, $14, $15, $15
        .byte $50, $50, $14, $14, $14, $14, $50, $50, $00, $00, $00, $00, $00, $00, $00, $00
        .byte $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00
        .byte $00, $00, $00, $00, $03, $03, $03, $03, $c0, $c0, $c0, $c0, $00, $00, $00, $00
        .byte $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00
        .byte $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00
        .byte $00, $00, $00, $00, $00, $00, $00, $00, $15, $15, $14, $14, $14, $14, $00, $00
        .byte $43, $73, $50, $50, $14, $14, $00, $00, $14, $14, $d4, $d4, $c5, $c5, $c0, $c0
        .byte $14, $14, $14, $14, $50, $50, $00, $00, $00, $00, $14, $14, $05, $05, $00, $00
        .byte $14, $14, $14, $14, $50, $50, $00, $00, $14, $14, $14, $14, $14, $14, $00, $00
        .byte $14, $14, $14, $14, $14, $14, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00
        .byte $00, $00, $00, $00, $00, $00, $00, $00, $01, $01, $01, $01, $05, $05, $00, $00
        .byte $4c, $4c, $4c, $4c, $5c, $5c, $0c, $0c, $14, $14, $14, $14, $14, $14, $00, $00
        .byte $54, $54, $14, $14, $14, $14, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00
        .byte $00, $00, $00, $00, $00, $00, $00, $00, $14, $14, $14, $14, $15, $15, $00, $00
        .byte $14, $14, $50, $50, $40, $40, $00, $00, $30, $3c, $0c, $0c, $0f, $0f, $03, $03
        .byte $00, $00, $00, $00, $00, $00, $f0, $00, $14, $14, $14, $14, $14, $14, $00, $00
        .byte $05, $05, $05, $05, $05, $05, $00, $00, $01, $01, $01, $01, $05, $05, $00, $00
        .byte $40, $40, $40, $40, $50, $50, $00, $00, $14, $14, $14, $14, $14, $14, $00, $00
        .byte $54, $54, $14, $14, $14, $14, $00, $00, $14, $14, $14, $14, $05, $05, $00, $00
        .byte $14, $14, $14, $14, $50, $50, $00, $00, $15, $15, $14, $14, $14, $14, $00, $00
        .byte $40, $40, $50, $50, $14, $14, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00
        .byte $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00
        .byte $03, $0c, $0c, $0c, $0c, $0c, $0c, $0c, $00, $00, $00, $00, $00, $00, $00, $00
        .byte $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00
        .byte $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00
        .byte $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00
        .byte $00, $03, $03, $00, $00, $00, $00, $00, $c0, $00, $00, $c0, $c0, $c0, $c0, $c0
        .byte $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00
        .byte $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00
        .byte $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00
        .byte $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00
        .byte $0c, $0c, $0c, $0c, $03, $03, $03, $03, $00, $00, $00, $00, $00, $00, $00, $00
        .byte $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00
        .byte $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00
        .byte $00, $00, $00, $00, $00, $00, $00, $00, $03, $0c, $0c, $0c, $0c, $0c, $0c, $0c
        .byte $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00
        .byte $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00
        .byte $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00
        .byte $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00
        .byte $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00
        .byte $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00
        .byte $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00
        .byte $30, $30, $30, $30, $30, $30, $30, $30, $00, $00, $00, $00, $00, $00, $00, $00
        .byte $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00
        .byte $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00
        .byte $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00
        .byte $00, $00, $00, $03, $03, $03, $03, $03, $c0, $c0, $c0, $00, $00, $00, $00, $00
        .byte $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00
        .byte $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00
        .byte $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00
        .byte $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00
        .byte $03, $03, $0c, $0c, $0c, $03, $03, $03, $00, $00, $00, $00, $00, $00, $00, $00
        .byte $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00
        .byte $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00
        .byte $00, $00, $00, $00, $00, $00, $00, $00, $0c, $0c, $0c, $0c, $0c, $0c, $0c, $0c
        .byte $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00
        .byte $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00
        .byte $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $2a
        .byte $00, $00, $00, $00, $00, $00, $00, $aa, $00, $80, $80, $80, $80, $80, $80, $aa
        .byte $00, $00, $00, $00, $00, $00, $00, $a8, $00, $00, $00, $00, $00, $00, $00, $00
        .byte $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00
        .byte $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $03
        .byte $30, $30, $30, $c0, $c0, $c0, $c0, $00, $00, $00, $00, $00, $00, $00, $00, $00
        .byte $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00
        .byte $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $aa, $aa, $aa
        .byte $00, $00, $00, $00, $00, $aa, $aa, $aa, $00, $00, $00, $00, $00, $a8, $a8, $a8
        .byte $03, $03, $03, $03, $0c, $0c, $aa, $aa, $00, $00, $00, $00, $00, $00, $aa, $aa
        .byte $00, $00, $00, $00, $00, $00, $aa, $aa, $00, $00, $00, $00, $00, $00, $aa, $aa
        .byte $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00
        .byte $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $2a, $2a, $2a
        .byte $00, $00, $00, $00, $00, $aa, $aa, $aa, $00, $00, $00, $00, $00, $aa, $aa, $aa
        .byte $03, $03, $03, $03, $03, $a3, $a3, $a3, $00, $00, $00, $00, $00, $00, $00, $00
        .byte $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00
        .byte $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00
        .byte $00, $00, $00, $00, $00, $00, $00, $00, $0c, $0c, $0c, $0c, $0c, $0c, $0c, $0c
        .byte $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00
        .byte $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00
        .byte $00, $00, $00, $00, $00, $00, $00, $00, $2a, $2a, $2a, $2a, $2a, $2a, $26, $26
        .byte $aa, $aa, $6a, $6a, $aa, $aa, $66, $66, $aa, $aa, $aa, $aa, $aa, $aa, $aa, $aa
        .byte $a8, $a8, $68, $68, $a8, $a8, $a8, $a8, $00, $00, $00, $00, $aa, $aa, $aa, $aa
        .byte $00, $00, $00, $00, $aa, $aa, $aa, $a9, $00, $00, $00, $00, $aa, $aa, $aa, $aa
        .byte $00, $00, $00, $00, $aa, $aa, $aa, $aa, $03, $03, $03, $03, $aa, $aa, $aa, $aa
        .byte $00, $00, $00, $00, $aa, $aa, $aa, $6a, $00, $00, $00, $00, $80, $80, $80, $80
        .byte $00, $00, $00, $00, $00, $aa, $aa, $aa, $00, $00, $00, $00, $00, $aa, $aa, $aa
        .byte $00, $00, $00, $00, $00, $aa, $aa, $aa, $aa, $aa, $aa, $aa, $aa, $aa, $aa, $aa
        .byte $9a, $9a, $aa, $aa, $a9, $a9, $aa, $aa, $a8, $a8, $a8, $a8, $a8, $a8, $a8, $a8
        .byte $aa, $a9, $a9, $aa, $aa, $aa, $aa, $aa, $aa, $a9, $a9, $aa, $aa, $aa, $aa, $aa
        .byte $aa, $99, $99, $aa, $aa, $9a, $9a, $aa, $aa, $9a, $9a, $aa, $aa, $9a, $9a, $aa
        .byte $00, $00, $00, $2a, $2a, $2a, $26, $26, $00, $00, $00, $aa, $aa, $aa, $66, $66
        .byte $00, $00, $00, $aa, $aa, $aa, $aa, $aa, $2a, $2a, $2a, $aa, $a6, $a6, $aa, $aa
        .byte $aa, $aa, $aa, $aa, $a6, $a6, $aa, $aa, $a6, $a6, $aa, $aa, $aa, $aa, $aa, $aa
        .byte $60, $60, $a0, $a0, $a0, $aa, $aa, $aa, $c0, $c0, $c0, $30, $30, $aa, $aa, $aa
        .byte $00, $00, $00, $00, $00, $aa, $aa, $aa, $00, $00, $02, $02, $02, $aa, $aa, $aa
        .byte $00, $00, $aa, $aa, $aa, $aa, $aa, $aa, $00, $00, $aa, $aa, $aa, $aa, $aa, $aa
        .byte $00, $00, $aa, $aa, $aa, $aa, $aa, $aa, $0c, $0c, $a3, $a3, $a3, $a3, $ac, $ac
        .byte $00, $00, $aa, $aa, $aa, $9a, $9a, $aa, $00, $00, $aa, $aa, $aa, $aa, $aa, $aa
        .byte $00, $00, $00, $00, $00, $00, $aa, $aa, $00, $00, $00, $00, $00, $00, $aa, $aa
        .byte $00, $00, $00, $00, $00, $00, $aa, $aa, $2a, $2a, $26, $26, $2a, $2a, $2a, $2a
        .byte $aa, $aa, $aa, $aa, $aa, $aa, $a6, $a6, $aa, $aa, $a6, $a6, $aa, $aa, $6a, $6a
        .byte $a8, $a8, $a8, $a8, $a8, $a8, $a8, $a8, $aa, $aa, $aa, $aa, $aa, $aa, $aa, $aa
        .byte $a9, $aa, $aa, $aa, $aa, $aa, $aa, $9a, $aa, $aa, $aa, $99, $99, $aa, $aa, $99
        .byte $aa, $aa, $aa, $aa, $aa, $aa, $aa, $a6, $aa, $aa, $aa, $aa, $aa, $aa, $aa, $a6
        .byte $6a, $aa, $aa, $a6, $a6, $aa, $aa, $a6, $aa, $aa, $aa, $a6, $a6, $aa, $aa, $a6
        .byte $aa, $aa, $aa, $aa, $a9, $a9, $aa, $aa, $aa, $aa, $aa, $aa, $aa, $aa, $aa, $aa
        .byte $9a, $9a, $aa, $aa, $aa, $aa, $aa, $aa, $aa, $aa, $aa, $aa, $aa, $aa, $aa, $aa
        .byte $aa, $aa, $aa, $aa, $aa, $aa, $aa, $aa, $a8, $a8, $a8, $a8, $98, $98, $a8, $a8
        .byte $aa, $99, $99, $aa, $aa, $aa, $aa, $aa, $aa, $9a, $9a, $aa, $aa, $aa, $aa, $aa
        .byte $aa, $a9, $a9, $aa, $aa, $9a, $9a, $aa, $aa, $9a, $9a, $aa, $aa, $9a, $9a, $aa
        .byte $2a, $2a, $26, $26, $2a, $2a, $2a, $2a, $aa, $aa, $6a, $6a, $aa, $aa, $aa, $aa
        .byte $aa, $aa, $66, $66, $aa, $aa, $aa, $aa, $a6, $a6, $aa, $aa, $a6, $a6, $aa, $aa
        .byte $a6, $a6, $aa, $aa, $a6, $a6, $aa, $aa, $aa, $aa, $aa, $aa, $aa, $aa, $aa, $aa
        .byte $aa, $aa, $aa, $aa, $69, $69, $aa, $aa, $99, $99, $aa, $aa, $9a, $9a, $aa, $aa
        .byte $9a, $9a, $aa, $aa, $9a, $9a, $aa, $aa, $aa, $aa, $aa, $aa, $aa, $aa, $aa, $aa
        .byte $aa, $6a, $6a, $aa, $aa, $6a, $6a, $aa, $aa, $a6, $a6, $aa, $aa, $aa, $aa, $aa
        .byte $aa, $a6, $a6, $aa, $aa, $6a, $6a, $aa, $ac, $6c, $6c, $a3, $a3, $a3, $a3, $a3
        .byte $aa, $a9, $a9, $aa, $aa, $9a, $9a, $aa, $aa, $9a, $9a, $aa, $aa, $aa, $aa, $aa
        .byte $aa, $a9, $a9, $aa, $aa, $99, $99, $aa, $aa, $99, $99, $aa, $aa, $aa, $aa, $aa
        .byte $aa, $aa, $aa, $aa, $aa, $9a, $9a, $aa, $2a, $2a, $2a, $2a, $2a, $2a, $2a, $2a
        .byte $aa, $aa, $a6, $a6, $aa, $aa, $aa, $aa, $aa, $aa, $66, $66, $aa, $aa, $aa, $aa
        .byte $a8, $a8, $a8, $a8, $a8, $a8, $a8, $a8, $aa, $aa, $aa, $aa, $aa, $aa, $aa, $aa
        .byte $9a, $aa, $aa, $aa, $aa, $aa, $aa, $aa, $99, $aa, $aa, $99, $99, $aa, $aa, $aa
        .byte $a6, $aa, $aa, $aa, $aa, $aa, $aa, $aa, $a6, $aa, $aa, $6a, $6a, $aa, $aa, $aa
        .byte $a6, $aa, $aa, $aa, $aa, $aa, $aa, $aa, $a6, $aa, $aa, $a6, $a6, $aa, $aa, $aa
        .byte $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00
        .byte $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00
        .byte $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00
        .byte $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00
        .byte $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00
        .byte $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00
        .byte $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00
        .byte $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00
        .byte $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00
        .byte $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00
        .byte $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00
        .byte $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00
        .byte $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00
        .byte $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00
        .byte $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00
        .byte $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00
        .byte $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00
        .byte $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00
        .byte $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00
        .byte $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00
        .byte $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00
        .byte $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00
        .byte $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00
        .byte $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00
        .byte $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00
        .byte $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00
        .byte $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00
        .byte $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00
        .byte $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00
        .byte $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00
        .byte $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00
        .byte $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00
        .byte $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00
        .byte $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00
        .byte $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00
        .byte $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00
        .byte $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00
        .byte $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00
        .byte $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00
        .byte $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00
        .byte $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00
        .byte $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00
        .byte $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00
        .byte $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00
        .byte $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00
        .byte $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00
        .byte $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00
        .byte $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00
        .byte $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00
        .byte $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00
        .byte $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00
        .byte $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00
        .byte $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00
        .byte $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00
        .byte $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00
        .byte $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00
        .byte $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00
        .byte $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00
        .byte $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00
        .byte $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00
rainspr .byte $00, $00, $02, $00, $00, $02, $00, $00, $00, $00, $00, $00, $10, $00, $00, $10
        .byte $00, $00, $10, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00
        .byte $00, $00, $80, $00, $00, $80, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00
        .byte $00, $00, $20, $00, $00, $20, $00, $02, $20, $00, $02, $00, $00, $00, $00, $00
        .byte $50, $00, $00, $10, $00, $00, $00, $00, $00, $00, $01, $00, $00, $01, $00, $00
        .byte $01, $00, $00, $02, $00, $00, $02, $00, $00, $00, $00, $00, $00, $00, $00, $00
        .byte $00, $00, $00, $00, $00, $80, $00, $00, $80, $00, $00, $80, $00, $00, $00, $00
        .byte $00, $00, $00, $00, $00, $00, $00, $00, $00, $40, $00, $00, $50, $00, $00, $00
        .byte $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $40
        .byte $00, $00, $40, $00, $00, $40, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00
        .byte $00, $00, $00, $08, $00, $00, $08, $00, $00, $08, $00, $00, $02, $00, $00, $02
        .byte $00, $48, $00, $00, $48, $00, $00, $08, $00, $00, $00, $00, $00, $00, $00, $00
scrcol  .byte $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $c0, $00
        .byte $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00
        .byte $00, $00, $00, $00, $00, $00, $00, $00, $c0, $00, $00, $00, $c0, $00, $c0, $00, $00, $c0, $00, $00
        .byte $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $c0
        .byte $00, $00, $00, $00, $c0, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00
        .byte $00, $c0, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00
        .byte $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $c0, $c0, $00, $00, $00, $00, $00, $00, $00
        .byte $00, $00, $00, $00, $00, $00, $c0, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00
        .byte $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $c0, $00, $00, $00, $00, $00
        .byte $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00
        .byte $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00
        .byte $00, $00, $00, $00, $00, $c0, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00
        .byte $00, $00, $00, $00, $00, $00, $00, $00, $00, $c0, $00, $00, $00, $00, $00, $00, $00, $c0, $00, $00
        .byte $c0, $00, $00, $00, $00, $00, $00, $00, $00, $00, $c0, $00, $c0, $00, $00, $00, $00, $00, $00, $00
        .byte $00, $00, $16, $16, $16, $16, $16, $16, $16, $16, $16, $16, $16, $16, $16, $16, $16, $16, $16, $16
        .byte $16, $16, $16, $16, $16, $16, $16, $16, $16, $16, $16, $16, $16, $16, $16, $16, $16, $16, $16, $06
        .byte $00, $06, $f6, $f6, $f6, $f6, $f6, $f0, $f0, $f6, $f6, $f0, $f6, $f0, $f0, $f6, $f0, $f6, $f0, $f6
        .byte $f0, $f6, $f0, $f6, $f0, $f6, $f6, $f6, $f0, $f6, $f0, $f6, $f6, $f6, $f6, $f6, $f0, $f6, $f6, $06
        .byte $00, $06, $f6, $f0, $f6, $f6, $f6, $f6, $f0, $f6, $f0, $f0, $f6, $f0, $f6, $f6, $f0, $f6, $f6, $f6
        .byte $f0, $f6, $f0, $f6, $f6, $f6, $f6, $f6, $f0, $f6, $f0, $f6, $f0, $f6, $f6, $f6, $f0, $f6, $00, $00
        .byte $00, $36, $36, $36, $30, $36, $30, $36, $30, $36, $30, $36, $30, $36, $36, $36, $30, $36, $36, $36
        .byte $36, $36, $36, $36, $36, $36, $36, $36, $30, $36, $30, $36, $30, $36, $36, $36, $30, $36, $00, $00
        .byte $00, $e6, $e0, $e6, $e0, $e6, $e0, $e6, $e0, $e6, $e0, $e6, $e6, $e6, $e6, $e6, $e6, $06, $06, $e0
        .byte $e6, $e6, $e6, $e6, $e6, $e6, $e6, $e6, $e6, $e6, $e6, $e6, $e0, $e6, $00, $e6, $e6, $06, $00, $00
        .byte $00, $e6, $e6, $e6, $e6, $e6, $e6, $e6, $e6, $e6, $e6, $e6, $e6, $e6, $e6, $e6, $e6, $06, $06, $e6
        .byte $e6, $e6, $e6, $e6, $e6, $e6, $e6, $e6, $e6, $e6, $e6, $e6, $e6, $06, $06, $e6, $e6, $06, $00, $00
        .byte $00, $00, $c0, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $c0
        .byte $00, $00, $00, $00, $00, $00, $00, $00, $c0, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00
        .byte $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00
        .byte $00, $00, $00, $c0, $00, $00, $00, $00, $00, $00, $00, $00, $00, $c0, $00, $00, $00, $00, $00, $00
        .byte $00, $00, $00, $00, $00, $70, $70, $70, $70, $70, $70, $70, $70, $00, $00, $70, $70, $70, $70, $00
        .byte $00, $70, $70, $00, $00, $70, $70, $70, $70, $70, $70, $70, $70, $70, $70, $00, $00, $00, $00, $00
        .byte $00, $00, $00, $00, $00, $70, $70, $70, $70, $70, $70, $70, $70, $00, $00, $70, $70, $70, $70, $00
        .byte $00, $70, $70, $00, $00, $70, $70, $70, $70, $70, $70, $70, $70, $70, $70, $00, $00, $00, $00, $00
        .byte $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00
        .byte $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00
        .byte $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00
        .byte $00, $00, $00, $00, $00, $00, $00, $00, $00, $0b, $0b, $0b, $0b, $00, $00, $00, $00, $00, $00, $00
        .byte $00, $00, $00, $0b, $0b, $0b, $0b, $0b, $0b, $0b, $00, $00, $00, $0b, $0b, $0b, $0b, $00, $00, $00
        .byte $00, $00, $00, $00, $00, $00, $00, $00, $00, $7b, $7b, $0b, $7b, $0b, $7b, $0b, $0b, $0b, $7b, $0b
        .byte $0b, $0b, $0b, $0b, $7b, $0b, $7b, $7b, $7b, $7b, $7b, $7b, $0b, $7b, $7b, $7b, $7b, $0b, $0b, $0b
        .byte $0b, $0b, $0b, $0b, $7b, $0b, $0b, $0b, $0b, $7b, $7b, $7b, $0b, $0b, $7b, $7b, $7b, $7b, $7b, $7b
        .byte $7b, $0b, $7b, $0b, $0b, $7b, $7b, $7b, $7b, $7b, $7b, $7b, $7b, $7b, $7b, $0b, $7b, $7b, $7b, $0b
        .byte $7b, $7b, $7b, $7b, $7b, $7b, $7b, $7b, $7b, $0b, $7b, $7b, $0b, $0b, $7b, $7b, $7b, $7b, $7b, $7b
        .byte $20, $20, $20, $20, $20, $20, $20, $20, $20, $20, $20, $20, $20, $20, $20, $20, $20, $20, $20, $20
        .byte $20, $20, $20, $20, $20, $20, $20, $20, $20, $20, $20, $20, $20, $20, $20, $20, $20, $20, $20, $20
        .byte $20, $20, $20, $20, $20, $20, $20, $20, $20, $20, $20, $20, $20, $20, $20, $20, $20, $20, $20, $20
        .byte $20, $20, $20, $20, $20, $20, $20, $20, $20, $20, $20, $20, $20, $20, $20, $20, $20, $20, $20, $20
        .byte $20, $20, $20, $20, $20, $20, $20, $20, $20, $20, $20, $20, $20, $20, $20, $20, $20, $20, $20, $20
        .byte $20, $20, $20, $20, $20, $20, $20, $20, $20, $20, $20, $20, $20, $20, $20, $20, $20, $20, $20, $20
colram  .byte $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00
        .byte $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00
        .byte $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00
        .byte $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00
        .byte $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00
        .byte $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00
        .byte $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00
        .byte $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00
        .byte $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00
        .byte $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00
        .byte $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00
        .byte $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00
        .byte $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00
        .byte $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00
        .byte $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00
        .byte $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00
        .byte $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00
        .byte $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00
        .byte $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00
        .byte $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00
        .byte $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00
        .byte $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00
        .byte $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00
        .byte $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00
        .byte $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00
        .byte $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00
        .byte $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00
        .byte $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00
        .byte $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00
        .byte $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00
        .byte $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00
        .byte $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00
        .byte $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00
        .byte $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00
        .byte $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00
        .byte $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00
        .byte $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00
        .byte $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00
        .byte $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00
        .byte $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00
        .byte $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00
        .byte $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00
        .byte $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00
        .byte $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00
        .byte $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00
        .byte $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00
        .byte $00, $0b, $0b, $0b, $0b, $0c, $0c, $0c, $0c, $0c, $0f, $0f, $0f, $0f, $0f, $01, $01, $01, $01, $01
        .byte $01, $01, $01, $01, $01, $0f, $0f, $0f, $0f, $0f, $0c, $0c, $0c, $0c, $0c, $0b, $0b, $0b, $0b, $0b
        .byte $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00
        .byte $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00
bolt0
        sta $d802
        sta $d803
        sta $d82a
        sta $d82b
        sta $d852
        sta $d87a
        sta $d8a2
        sta $d8c9
        sta $d8ca
        sta $d8f1
        sta $d8f2
        sta $d919
        sta $d91a
        sta $d941
        sta $d992
        sta $d9e4
        sta $d9e5
        sta $da0d
        sta $da35
        sta $da36
        sta $da5d
        sta $da5e
        sta $da86
        sta $da87
        sta $daae
        sta $daaf
        sta $dad6
        sta $dad7
        sta $dafe
        rts
bolt1
        sta $d810
        sta $d838
        sta $d839
        sta $d861
        sta $d889
        sta $d88a
        sta $d8b1
        sta $d8b2
        sta $d8d9
        sta $d901
        sta $d902
        sta $d929
        sta $d9c9
        sta $d9ca
        sta $d9f0
        sta $d9f2
        sta $d9f3
        sta $da18
        sta $da40
        sta $da68
        sta $da90
        sta $dab8
        sta $dae0
        sta $db08
        sta $db31
        rts
bolt2
        sta $d81a
        sta $d842
        sta $d86a
        sta $d891
        sta $d892
        sta $d893
        sta $d8b9
        sta $d8ba
        sta $d8bb
        sta $d8e2
        sta $d8e3
        sta $d8e4
        sta $d909
        sta $d90a
        sta $d90b
        sta $d90c
        sta $d90d
        sta $d933
        sta $d934
        sta $d9f6
        sta $d9f7
        sta $da1e
        sta $da1f
        sta $da46
        sta $da47
        sta $da6e
        sta $da6f
        sta $da97
        sta $da98
        sta $dabf
        sta $dae7
        sta $db0f
        sta $db37
        sta $db5f
        rts
bolt3
        sta $d824
        sta $d84b
        sta $d84c
        sta $d873
        sta $d89b
        sta $d89c
        sta $d8c3
        sta $d8c4
        sta $d8c5
        sta $d8eb
        sta $d8ec
        sta $d8ed
        sta $d912
        sta $d913
        sta $d914
        sta $d915
        sta $d93d
        sta $d966
        sta $d98e
        sta $d9b6
        sta $d9b7
        sta $d9de
        sta $da06
        sta $da2e
        sta $da57
        sta $da7e
        sta $da7f
        sta $daa6
        sta $dace
        sta $daf5
        sta $daf6
        sta $db1d
        rts
boltlo  .byte <bolt0, <bolt1, <bolt2, <bolt3
bolthi  .byte >bolt0, >bolt1, >bolt2, >bolt3
fxtab   .byte $04, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00
        .byte $02, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00
        .byte $08, $00, $00, $00, $00, $00, $00, $00, $01, $00, $00, $00, $00, $00, $00, $00
        .byte $04, $00, $00, $00, $00, $00, $00, $00, $02, $00, $00, $00, $01, $02, $04, $08
        .byte $1f, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00
        .byte $01, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00
        .byte $04, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00
        .byte $02, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00
        .byte $08, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00
        .byte $01, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00
        .byte $04, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00
        .byte $02, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $00, $01, $02, $04, $08
        .byte $1f, $00, $00, $00, $00, $00, $00, $00, $01, $00, $00, $00, $00, $00, $00, $00
        .byte $04, $00, $00, $00, $00, $00, $00, $00, $02, $00, $00, $00, $00, $00, $00, $00
        .byte $08, $00, $00, $00, $00, $00, $00, $00, $01, $00, $00, $00, $00, $00, $00, $00
        .byte $04, $00, $00, $00, $00, $00, $00, $00, $02, $00, $00, $00, $00, $00, $00, $00
        .byte $08, $00, $00, $00, $00, $00, $00, $00, $01, $00, $00, $00, $00, $00, $00, $00
        .byte $04, $00, $00, $00, $00, $00, $00, $00, $02, $00, $00, $00, $00, $00, $00, $00
        .byte $08, $00, $00, $00, $00, $00, $00, $00, $01, $00, $00, $00, $00, $00, $00, $00
        .byte $04, $00, $00, $00, $00, $00, $00, $00, $02, $00, $00, $00, $01, $02, $04, $1f
fxlo    .byte <(fxtab+0), <(fxtab+16), <(fxtab+32), <(fxtab+48), <(fxtab+64), <(fxtab+80), <(fxtab+96), <(fxtab+112), <(fxtab+128), <(fxtab+144), <(fxtab+160), <(fxtab+176), <(fxtab+192), <(fxtab+208), <(fxtab+224), <(fxtab+240), <(fxtab+256), <(fxtab+272), <(fxtab+288), <(fxtab+304)
fxhi    .byte >(fxtab+0), >(fxtab+16), >(fxtab+32), >(fxtab+48), >(fxtab+64), >(fxtab+80), >(fxtab+96), >(fxtab+112), >(fxtab+128), >(fxtab+144), >(fxtab+160), >(fxtab+176), >(fxtab+192), >(fxtab+208), >(fxtab+224), >(fxtab+240), >(fxtab+256), >(fxtab+272), >(fxtab+288), >(fxtab+304)
; ---- fine gfx_data.asm ----
