; SPDX-License-Identifier: GPL-2.0-or-later
; Copyright (C) 2026 SampoSoft - Francesco Sampoli
;
; ssh64 - zero page map. The whole zero page is ours: BASIC and the KERNAL
; are banked out while the client runs.

        .enc "ascii"            ; strings are ASCII, not PETSCII
        .cdef " ~", 32

; field arithmetic (fe25519.asm)
fe_pa   = $02           ; operand pointers
fe_pb   = $04
fe_pd   = $06           ; destination pointer
fe_t    = $08           ; product temporaries
fe_c0   = $09
fe_c1   = $0a
fe_n    = $0b           ; repeat counter
fe_ip   = $0c           ; fe program pointer (2)
fe_x    = $0e           ; saved row / scratch
fe_tmp  = $0f
fe_op   = $10
fe_cnt  = $11
M8_L    = $12           ; 8 x 8 blocks: low bytes of a row's products, $12-$19
M8_H    = $46           ; high bytes, $46-$4d (in the SHA-512 area)
k_u     = $1a           ; Karatsuba (16 bytes): u, v pointers (2 each)
k_v     = $1c
k_r     = $1e           ; row source of an 8 x 8 block (2)
k_o     = $20           ; result offset from FE_R
k_s     = $21           ; sign of the middle product
fe_pc   = $22           ; column source pointer (2)
fe_end  = $24           ; end of the rows of a block
k_s1    = $25           ; sign of the first level's middle product

FE_M    = $c0           ; Karatsuba middle product, $c0-$df
fe_ka   = $e0           ; |a0 - a1|, $e0-$ef
fe_kb   = $f0           ; |b1 - b0|, $f0-$ff

FE_R    = $80           ; 64-byte product, $80-$bf

; inversion (feprog.asm), in the same area
GCD_U   = $80
GCD_V   = $a0
GCD_X1  = $c0
GCD_X2  = $e0

; the second Karatsuba level borrows the SHA area (never at once)
k2_w    = $26           ; middle product, 16 bytes
k2_du   = $36           ; |u0 - u1|, 8 bytes
k2_dv   = $3e           ; |v1 - v0|, 8 bytes

; SHA-256 (sha256.asm); free for the field code in between
sha_p   = $26           ; input pointer (2)
sha_len = $28           ; input length (2)
sha_t   = $2a           ; 32-bit temporaries, 4 bytes each
sha_u   = $2e
sha_s   = $32
sha_s2  = $36
sha_tmp = $3a

; SHA-512 (sha512.asm)
s5_r    = $3c           ; rotation result (8)
s5_s    = $44           ; sigma accumulator (8)
s5_n    = $4c           ; loop counter
s5_x    = $4d           ; word being rotated
s5_sh   = $4e           ; rotation amount, bit 7 = shift
s5_kk   = $4f           ; byte part of the amount
s5_k    = $50           ; parameters of s5_sigma (2)
s5_kp   = $52           ; K pointer (2)
s5_t    = $54           ; round

; ed25519 (ed25519.asm)
ed_i    = $55           ; bit position
ed_p    = $56           ; pointer (2)
ed_v    = $58           ; slide: 1 << b
ed_b    = $59           ; slide: b
ed_q    = $5a           ; scratch
kb_tmp  = $5b           ; keyboard scan (IRQ)

; AES-GCM (aes.asm, gcm.asm): in the field area, never used at once
aes_s   = $80           ; AES state, 16 bytes
aes_t   = $90           ; second state
gh_acc  = $a0           ; GHASH accumulator
gh_za   = $b0           ; GHASH products, ping-pong
gh_zb   = $c0
gcm_ctr = $d0           ; counter block
gcm_ej0 = $e0           ; E(J0), masks the tag
gcm_p   = $f0           ; data pointer (2)
gcm_n   = $f2           ; bytes left (2)
aes_n   = $f4           ; rounds left
gcm_tmp = $f5
gcm_ap  = $f6           ; pointer (2)

; network (net.asm, tcp.asm): kept apart from the crypto areas, so that
; net_poll may run in the middle of a long computation
net_p   = $60           ; pointers (2 each)
net_q   = $62
net_n   = $64           ; byte count (2)
net_ck  = $66           ; checksum: low, high, carries (3)
net_t   = $69           ; scratch (4)
net_len = $6d           ; length of the packet being built (2)
ticks   = $6f           ; 1/60 s counter, from the IRQ (2)

; system, screen (sys.asm)
irq_a   = $71           ; IRQ register save
irq_x   = $72
irq_y   = $73
scr_p   = $74           ; pointer (2)
cur_x   = $76
cur_y   = $77
str_p   = $78           ; inline string pointer (2)
sys_t   = $7a           ; scratch (2)

; keyboard (kbd.asm), in the IRQ
kb_col  = $7c           ; column being scanned
kb_bits = $7d
kb_code = $7e           ; key pressed in this scan ($ff none)
kb_mods = $7f           ; bit 0 shift, 1 C=, 2 ctrl

; terminal (term.asm)
t_p     = $5c           ; row pointers: screen (2), colours (2)
t_c     = $5e
t_q     = $20           ; (shared with the field code: never at once) second row (2)
t_r     = $22           ; colours of the second row (2)

; SSH (ssh.asm): shares $3c-$43 with SHA-512, which runs only inside
; ed25519_verify, when no packet is being read or built
ssh_rp  = $3c           ; read pointer in the received payload (2)
ssh_wp  = $3e           ; write pointer in the packet being built (2)
ssh_sp  = $40           ; last string read: pointer (2)
ssh_sl  = $42           ; and length (2)
