// SPDX-License-Identifier: GPL-2.0-or-later
// Copyright (C) 2026 SampoSoft - Francesco Sampoli

//! Ethernet, ARP, IPv4, UDP and TCP headers: parsing the frames the C64
//! sends and building the ones the virtual router answers with.

use std::net::Ipv4Addr;

pub type Mac = [u8; 6];

pub const BROADCAST: Mac = [0xFF; 6];
pub const ETH_IPV4: u16 = 0x0800;
pub const ETH_ARP: u16 = 0x0806;
pub const PROTO_ICMP: u8 = 1;
pub const PROTO_TCP: u8 = 6;
pub const PROTO_UDP: u8 = 17;

pub const FIN: u8 = 0x01;
pub const SYN: u8 = 0x02;
pub const RST: u8 = 0x04;
pub const PSH: u8 = 0x08;
pub const ACK: u8 = 0x10;

fn be16(b: &[u8]) -> u16 {
    u16::from_be_bytes([b[0], b[1]])
}

fn be32(b: &[u8]) -> u32 {
    u32::from_be_bytes([b[0], b[1], b[2], b[3]])
}

fn ip(b: &[u8]) -> Ipv4Addr {
    Ipv4Addr::new(b[0], b[1], b[2], b[3])
}

/// Internet checksum (one's complement of the one's complement sum of
/// 16-bit words) of the concatenated parts; all but the last have an even
/// length.
pub fn checksum(parts: &[&[u8]]) -> u16 {
    let mut sum = 0u32;
    for part in parts {
        let (words, rest) = part.as_chunks::<2>();
        for w in words {
            sum += u16::from_be_bytes(*w) as u32;
        }
        if let [last] = rest {
            sum += (*last as u32) << 8;
        }
    }
    while sum > 0xFFFF {
        sum = (sum & 0xFFFF) + (sum >> 16);
    }
    !(sum as u16)
}

/// Pseudo-header of the TCP and UDP checksums.
fn pseudo(src: Ipv4Addr, dst: Ipv4Addr, proto: u8, len: usize) -> [u8; 12] {
    let mut p = [0u8; 12];
    p[0..4].copy_from_slice(&src.octets());
    p[4..8].copy_from_slice(&dst.octets());
    p[9] = proto;
    p[10..12].copy_from_slice(&(len as u16).to_be_bytes());
    p
}

// ── Ethernet ──────────────────────────────────────────────────────────────────

pub struct Ether<'a> {
    pub dst: Mac,
    pub src: Mac,
    pub ethertype: u16,
    pub payload: &'a [u8],
}

pub fn parse_ether(f: &[u8]) -> Option<Ether<'_>> {
    if f.len() < 14 {
        return None;
    }
    Some(Ether {
        dst: f[0..6].try_into().unwrap(),
        src: f[6..12].try_into().unwrap(),
        ethertype: be16(&f[12..14]),
        payload: &f[14..],
    })
}

/// Ethernet frame, padded to the 60-byte minimum as on the wire.
pub fn ether(dst: Mac, src: Mac, ethertype: u16, payload: &[u8]) -> Vec<u8> {
    let mut f = Vec::with_capacity((14 + payload.len()).max(60));
    f.extend_from_slice(&dst);
    f.extend_from_slice(&src);
    f.extend_from_slice(&ethertype.to_be_bytes());
    f.extend_from_slice(payload);
    if f.len() < 60 {
        f.resize(60, 0);
    }
    f
}

// ── ARP ───────────────────────────────────────────────────────────────────────

pub const ARP_REQUEST: u16 = 1;
pub const ARP_REPLY: u16 = 2;

pub struct Arp {
    pub op: u16,
    pub sha: Mac,
    pub spa: Ipv4Addr,
    pub tha: Mac,
    pub tpa: Ipv4Addr,
}

pub fn parse_arp(p: &[u8]) -> Option<Arp> {
    // Ethernet and IPv4 only
    if p.len() < 28 || be16(&p[0..2]) != 1 || be16(&p[2..4]) != ETH_IPV4 || p[4] != 6 || p[5] != 4 {
        return None;
    }
    Some(Arp {
        op: be16(&p[6..8]),
        sha: p[8..14].try_into().unwrap(),
        spa: ip(&p[14..18]),
        tha: p[18..24].try_into().unwrap(),
        tpa: ip(&p[24..28]),
    })
}

pub fn arp(a: &Arp) -> Vec<u8> {
    let mut p = Vec::with_capacity(28);
    p.extend_from_slice(&[0, 1, 0x08, 0x00, 6, 4]);
    p.extend_from_slice(&a.op.to_be_bytes());
    p.extend_from_slice(&a.sha);
    p.extend_from_slice(&a.spa.octets());
    p.extend_from_slice(&a.tha);
    p.extend_from_slice(&a.tpa.octets());
    p
}

// ── IPv4 ──────────────────────────────────────────────────────────────────────

pub struct Ipv4<'a> {
    pub src: Ipv4Addr,
    pub dst: Ipv4Addr,
    pub proto: u8,
    pub payload: &'a [u8],
}

/// IPv4 packet with a valid header. Fragments are refused: the C64 stacks
/// never send them.
pub fn parse_ipv4(p: &[u8]) -> Option<Ipv4<'_>> {
    if p.len() < 20 || p[0] >> 4 != 4 {
        return None;
    }
    let ihl = (p[0] & 0x0F) as usize * 4;
    let total = be16(&p[2..4]) as usize;
    if ihl < 20 || total < ihl || total > p.len() || checksum(&[&p[..ihl]]) != 0 {
        return None;
    }
    let more_fragments = p[6] & 0x20 != 0;
    let offset = be16(&p[6..8]) & 0x1FFF;
    if more_fragments || offset != 0 {
        return None;
    }
    Some(Ipv4 { src: ip(&p[12..16]), dst: ip(&p[16..20]), proto: p[9], payload: &p[ihl..total] })
}

pub fn ipv4(src: Ipv4Addr, dst: Ipv4Addr, proto: u8, id: u16, payload: &[u8]) -> Vec<u8> {
    let mut p = Vec::with_capacity(20 + payload.len());
    p.extend_from_slice(&[0x45, 0]);
    p.extend_from_slice(&((20 + payload.len()) as u16).to_be_bytes());
    p.extend_from_slice(&id.to_be_bytes());
    p.extend_from_slice(&[0x40, 0, 64, proto, 0, 0]); // DF, TTL 64
    p.extend_from_slice(&src.octets());
    p.extend_from_slice(&dst.octets());
    let sum = checksum(&[&p]);
    p[10..12].copy_from_slice(&sum.to_be_bytes());
    p.extend_from_slice(payload);
    p
}

// ── ICMP ──────────────────────────────────────────────────────────────────────

pub const ICMP_ECHO_REPLY: u8 = 0;
pub const ICMP_ECHO_REQUEST: u8 = 8;

/// ICMP message with a valid checksum: (type, code, the rest after the
/// checksum).
pub fn parse_icmp(p: &[u8]) -> Option<(u8, u8, &[u8])> {
    (p.len() >= 8 && checksum(&[p]) == 0).then(|| (p[0], p[1], &p[4..]))
}

pub fn icmp(kind: u8, code: u8, rest: &[u8]) -> Vec<u8> {
    let mut p = vec![kind, code, 0, 0];
    p.extend_from_slice(rest);
    let sum = checksum(&[&p]);
    p[2..4].copy_from_slice(&sum.to_be_bytes());
    p
}

// ── UDP ───────────────────────────────────────────────────────────────────────

pub struct Udp<'a> {
    pub src_port: u16,
    pub dst_port: u16,
    pub payload: &'a [u8],
}

pub fn parse_udp<'a>(ip: &Ipv4<'a>) -> Option<Udp<'a>> {
    let p = ip.payload;
    if p.len() < 8 {
        return None;
    }
    let len = be16(&p[4..6]) as usize;
    if len < 8 || len > p.len() {
        return None;
    }
    // A zero checksum means none was computed
    if be16(&p[6..8]) != 0 && checksum(&[&pseudo(ip.src, ip.dst, PROTO_UDP, len), &p[..len]]) != 0 {
        return None;
    }
    Some(Udp { src_port: be16(&p[0..2]), dst_port: be16(&p[2..4]), payload: &p[8..len] })
}

pub fn udp(src: Ipv4Addr, dst: Ipv4Addr, src_port: u16, dst_port: u16, payload: &[u8]) -> Vec<u8> {
    let len = 8 + payload.len();
    let mut p = Vec::with_capacity(len);
    p.extend_from_slice(&src_port.to_be_bytes());
    p.extend_from_slice(&dst_port.to_be_bytes());
    p.extend_from_slice(&(len as u16).to_be_bytes());
    p.extend_from_slice(&[0, 0]);
    p.extend_from_slice(payload);
    let sum = match checksum(&[&pseudo(src, dst, PROTO_UDP, len), &p]) {
        0 => 0xFFFF,
        s => s,
    };
    p[6..8].copy_from_slice(&sum.to_be_bytes());
    p
}

// ── TCP ───────────────────────────────────────────────────────────────────────

pub struct Tcp<'a> {
    pub src_port: u16,
    pub dst_port: u16,
    pub seq: u32,
    pub ack: u32,
    pub flags: u8,
    pub window: u16,
    /// Maximum segment size option (SYN segments).
    pub mss: Option<u16>,
    pub payload: &'a [u8],
}

pub fn parse_tcp<'a>(ip: &Ipv4<'a>) -> Option<Tcp<'a>> {
    let p = ip.payload;
    if p.len() < 20 {
        return None;
    }
    let off = (p[12] >> 4) as usize * 4;
    if off < 20 || off > p.len() || checksum(&[&pseudo(ip.src, ip.dst, PROTO_TCP, p.len()), p]) != 0 {
        return None;
    }
    let mut mss = None;
    let mut opts = &p[20..off];
    while let [kind, rest @ ..] = opts {
        match kind {
            0 => break,
            1 => opts = rest,
            _ => {
                let Some(&len) = rest.first() else { break };
                let len = len as usize;
                if len < 2 || len > opts.len() {
                    break;
                }
                if *kind == 2 && len == 4 {
                    mss = Some(be16(&opts[2..4]));
                }
                opts = &opts[len..];
            }
        }
    }
    Some(Tcp {
        src_port: be16(&p[0..2]),
        dst_port: be16(&p[2..4]),
        seq: be32(&p[4..8]),
        ack: be32(&p[8..12]),
        flags: p[13],
        window: be16(&p[14..16]),
        mss,
        payload: &p[off..],
    })
}

/// TCP segment to send.
pub struct TcpOut<'a> {
    pub src_port: u16,
    pub dst_port: u16,
    pub seq: u32,
    pub ack: u32,
    pub flags: u8,
    pub window: u16,
    pub mss: Option<u16>,
    pub payload: &'a [u8],
}

pub fn tcp(src: Ipv4Addr, dst: Ipv4Addr, s: &TcpOut) -> Vec<u8> {
    let hlen = if s.mss.is_some() { 24 } else { 20 };
    let mut p = Vec::with_capacity(hlen + s.payload.len());
    p.extend_from_slice(&s.src_port.to_be_bytes());
    p.extend_from_slice(&s.dst_port.to_be_bytes());
    p.extend_from_slice(&s.seq.to_be_bytes());
    p.extend_from_slice(&s.ack.to_be_bytes());
    p.push((hlen as u8 / 4) << 4);
    p.push(s.flags);
    p.extend_from_slice(&s.window.to_be_bytes());
    p.extend_from_slice(&[0, 0, 0, 0]);
    if let Some(mss) = s.mss {
        p.extend_from_slice(&[2, 4]);
        p.extend_from_slice(&mss.to_be_bytes());
    }
    p.extend_from_slice(s.payload);
    let sum = checksum(&[&pseudo(src, dst, PROTO_TCP, p.len()), &p]);
    p[16..18].copy_from_slice(&sum.to_be_bytes());
    p
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ipv4_header_checksum() {
        let p = ipv4(Ipv4Addr::new(10, 0, 2, 2), Ipv4Addr::new(10, 0, 2, 15), PROTO_UDP, 7, &[1, 2, 3]);
        assert_eq!(checksum(&[&p[..20]]), 0);
        let ip = parse_ipv4(&p).unwrap();
        assert_eq!((ip.proto, ip.payload), (PROTO_UDP, &[1, 2, 3][..]));
    }

    #[test]
    fn tcp_round_trip_with_mss() {
        let (a, b) = (Ipv4Addr::new(1, 2, 3, 4), Ipv4Addr::new(5, 6, 7, 8));
        let seg = tcp(a, b, &TcpOut {
            src_port: 1000, dst_port: 80, seq: 0xDEADBEEF, ack: 5, flags: SYN | ACK,
            window: 4096, mss: Some(1460), payload: b"odd",
        });
        let packet = ipv4(a, b, PROTO_TCP, 1, &seg);
        let ip = parse_ipv4(&packet).unwrap();
        let t = parse_tcp(&ip).unwrap();
        assert_eq!((t.seq, t.ack, t.flags, t.window, t.mss, t.payload), (0xDEADBEEF, 5, SYN | ACK, 4096, Some(1460), &b"odd"[..]));
    }
}
