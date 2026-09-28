// SPDX-License-Identifier: GPL-2.0-or-later
// Copyright (C) 2026 SampoSoft - Francesco Sampoli
// The cartridge mapping follows VICE 3.10 c64/cart/ethernetcart.c and
// c64/cart/clockport-rrnet.c, Copyright (C) Marco van den Heuvel.
// Behaviour modelled on VICE; see CREDITS.md.

//! Ethernet cartridge with the CS8900A (`cs8900`), and the virtual network
//! it is plugged into (`nat`).
//!
//! Two cartridges share the chip, differing in how its 16 bytes are mapped
//! in the I/O area:
//!
//! - **TFE** (The Final Ethernet): the ports in order, RxTxData at the base.
//! - **RR-Net** (the Retro Replay module, and the RR-Net MK3): address line
//!   A3 inverted, so RxTxData is at base+8 and the PacketPage pointer at
//!   base+2. The first two bytes are the Retro Replay's registers: they read
//!   0 and ignore writes, as in VICE.
//!
//! The base is $DE00 by default, or any 16-byte slot of I/O1 and I/O2.
//! The network is not the host's LAN but a router inside the emulator
//! (`nat::Nat`), which turns the C64's connections into ordinary sockets
//! of the host.

pub mod cs8900;
pub mod nat;
mod tcp;
mod wire;

pub use cs8900::Cs8900;
pub use nat::Nat;

/// How the chip's ports are mapped in the I/O area.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum EthMode {
    /// The Final Ethernet: ports in order.
    Tfe,
    /// RR-Net: A3 inverted, the first two bytes unused.
    #[default]
    RrNet,
}

impl EthMode {
    pub fn name(self) -> &'static str {
        match self {
            EthMode::Tfe => "TFE",
            EthMode::RrNet => "RR-Net",
        }
    }
}

/// Parses `rrnet` or `tfe`, optionally followed by `@ADDR` (hex, a
/// 16-byte slot in $DE00-$DFF0), as on the command line and in the
/// debugger.
pub fn parse_ethernet(s: &str) -> Result<(EthMode, u16), String> {
    let (kind, addr) = match s.split_once('@') {
        Some((k, a)) => (k, Some(a)),
        None => (s, None),
    };
    let mode = match kind.to_ascii_lowercase().as_str() {
        "rrnet" | "rr-net" => EthMode::RrNet,
        "tfe" => EthMode::Tfe,
        _ => return Err(format!("unknown Ethernet cartridge '{kind}' (rrnet or tfe)")),
    };
    let base = match addr {
        None => 0xDE00,
        Some(a) => crate::frontend::session::parse_address(a)
            .ok_or_else(|| format!("invalid address '{a}'"))?,
    };
    if !valid_base(base) {
        return Err(format!("invalid Ethernet cartridge address ${base:04X} ($DE00-$DFF0, in steps of $10)"));
    }
    Ok((mode, base))
}

/// The cartridge's base address must be a 16-byte slot of I/O1 or I/O2.
pub fn valid_base(base: u16) -> bool {
    (0xDE00..=0xDFF0).contains(&base) && base & 0x0F == 0
}

/// Ethernet cartridge: the chip and its place in the I/O area.
#[derive(Default)]
pub struct EthernetCart {
    pub mode: EthMode,
    pub base: u16,
    pub chip: Cs8900,
}

impl EthernetCart {
    pub fn new(mode: EthMode, base: u16) -> Self {
        Self { mode, base, chip: Cs8900::new() }
    }

    /// The address is one of the cartridge's 16 bytes.
    #[inline]
    pub fn contains(&self, addr: u16) -> bool {
        addr & 0xFFF0 == self.base
    }

    /// Chip port behind the cartridge byte `off` (0-15), `None` for the
    /// two unused bytes of the RR-Net.
    fn port(&self, off: u8) -> Option<u8> {
        match self.mode {
            EthMode::Tfe => Some(off),
            EthMode::RrNet if off < 2 => None,
            EthMode::RrNet => Some(off ^ 8),
        }
    }

    pub fn read(&mut self, addr: u16) -> u8 {
        self.port(addr as u8 & 0x0F).map_or(0, |p| self.chip.read(p))
    }

    pub fn peek(&self, addr: u16) -> u8 {
        self.port(addr as u8 & 0x0F).map_or(0, |p| self.chip.peek(p))
    }

    pub fn write(&mut self, addr: u16, val: u8) {
        if let Some(p) = self.port(addr as u8 & 0x0F) {
            self.chip.write(p, val);
        }
    }

    pub fn reset(&mut self) {
        self.chip.reset();
    }
}

crate::snapshot::impl_state_enum!(EthMode { Tfe, RrNet });
crate::snapshot::impl_state!(EthernetCart { mode, base, chip });

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rrnet_inverts_a3() {
        let mut cart = EthernetCart::new(EthMode::RrNet, 0xDE00);
        // PacketPage pointer at $DE02, data at $DE04: the product ID
        cart.write(0xDE02, 0x00);
        cart.write(0xDE03, 0x00);
        assert_eq!((cart.read(0xDE04), cart.read(0xDE05)), (0x0E, 0x63));
        assert_eq!(cart.read(0xDE00), 0);
        let mut tfe = EthernetCart::new(EthMode::Tfe, 0xDE10);
        assert!(tfe.contains(0xDE1A) && !tfe.contains(0xDE00));
        tfe.write(0xDE1A, 0x00);
        tfe.write(0xDE1B, 0x00);
        assert_eq!((tfe.read(0xDE1C), tfe.read(0xDE1D)), (0x0E, 0x63));
    }

    #[test]
    fn rrnet_on_the_retro_replay_clock_port() {
        // Retro Replay, 4 empty 8K banks
        let mut crt = b"C64 CARTRIDGE   ".to_vec();
        crt.extend(0x40u32.to_be_bytes());
        crt.extend([1, 0, 0, 36, 0, 1, 0]);
        crt.resize(0x40, 0);
        for bank in 0..4u16 {
            crt.extend(b"CHIP");
            crt.extend((16u32 + 0x2000).to_be_bytes());
            crt.extend([0, 0]);
            crt.extend(bank.to_be_bytes());
            crt.extend([0x80, 0x00, 0x20, 0x00]);
            crt.extend([0u8; 0x2000]);
        }
        let mut c64 = crate::c64::C64::new();
        c64.set_ethernet(Some((EthMode::RrNet, 0xDE00))).unwrap();
        c64.load_cartridge(&crt).unwrap();
        let id = |c64: &mut crate::c64::C64| {
            c64.bus.write(0xDE02, 0x00);
            c64.bus.write(0xDE03, 0x00);
            (c64.bus.read(0xDE04), c64.bus.read(0xDE05))
        };
        // $DE00 is the Retro Replay's status; with the clock port off the
        // RR-Net does not answer
        assert_ne!(id(&mut c64), (0x0E, 0x63));
        c64.bus.write(0xDE00, 0x98);
        assert_eq!(c64.bus.read(0xDE00), 0x98);
        c64.bus.write(0xDE01, 0x01);
        assert_eq!(id(&mut c64), (0x0E, 0x63));
        assert_eq!(c64.bus.read(0xDE00), 0x00);
        // Registers disabled: the clock port stays
        c64.bus.write(0xDE00, 0x06);
        assert_eq!(id(&mut c64), (0x0E, 0x63));
    }

    #[test]
    fn parse() {
        assert_eq!(parse_ethernet("rrnet"), Ok((EthMode::RrNet, 0xDE00)));
        assert_eq!(parse_ethernet("TFE@df10"), Ok((EthMode::Tfe, 0xDF10)));
        assert!(parse_ethernet("tfe@de08").is_err());
        assert!(parse_ethernet("ne2000").is_err());
    }
}

/// The whole path: a 6502 driver (test_driver/driver.asm) on the emulated
/// CPU drives the RR-Net registers, and the router answers from the host.
#[cfg(test)]
mod through_the_6502 {
    use super::nat::{DNS, LEASE, ROUTER};
    use super::wire::{self, BROADCAST, ETH_IPV4, PROTO_UDP};
    use super::EthMode;
    use crate::c64::C64;
    use std::net::{Ipv4Addr, UdpSocket};

    const DRIVER: &[u8] = include_bytes!("test_driver/driver.bin");
    const MAC: [u8; 6] = [0x02, 0xC6, 0x4C, 0x64, 0x00, 0x01];

    fn machine() -> C64 {
        let mut c64 = C64::new();
        c64.set_ethernet(Some((EthMode::RrNet, 0xDE00))).unwrap();
        c64.reset();
        c64.bus.ram[0xC000..0xC000 + DRIVER.len()].copy_from_slice(DRIVER);
        c64.bus.ram[0xC810..0xC816].copy_from_slice(&MAC);
        c64.cpu.pc = 0xC000;
        c64.run_frame();
        c64
    }

    /// Sends a frame through the driver.
    fn send(c64: &mut C64, frame: &[u8]) {
        c64.bus.ram[0xC900..0xC900 + frame.len()].copy_from_slice(frame);
        c64.bus.ram[0xC802..0xC804].copy_from_slice(&(frame.len() as u16).to_le_bytes());
        c64.bus.ram[0xC800] = 1;
        for _ in 0..10 {
            c64.run_frame();
            if c64.bus.ram[0xC800] == 0 {
                return;
            }
        }
        panic!("the driver did not send the frame");
    }

    /// The frames the driver has received so far.
    fn received(c64: &C64) -> Vec<Vec<u8>> {
        let count = u16::from_le_bytes([c64.bus.ram[0xC804], c64.bus.ram[0xC805]]);
        let mut at = 0x4000;
        (0..count).map(|_| {
            let len = u16::from_le_bytes([c64.bus.ram[at], c64.bus.ram[at + 1]]) as usize;
            at += 2 + len;
            c64.bus.ram[at - len..at].to_vec()
        }).collect()
    }

    /// Runs frames, in real time too, until `n` frames have arrived.
    fn wait_for(c64: &mut C64, n: usize) -> Vec<Vec<u8>> {
        for _ in 0..200 {
            c64.run_frame();
            let got = received(c64);
            if got.len() >= n {
                return got;
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        panic!("only {} frames received", received(c64).len());
    }

    fn udp_frame(dst_mac: [u8; 6], src: Ipv4Addr, dst: Ipv4Addr, sport: u16, dport: u16, data: &[u8]) -> Vec<u8> {
        let u = wire::udp(src, dst, sport, dport, data);
        wire::ether(dst_mac, MAC, ETH_IPV4, &wire::ipv4(src, dst, PROTO_UDP, 1, &u))
    }

    #[test]
    fn dhcp_arp_and_udp() {
        let mut c64 = machine();
        // DHCP DISCOVER: the OFFER of 10.0.2.15 comes back
        let mut discover = vec![0u8; 240];
        discover[..3].copy_from_slice(&[1, 1, 6]);
        discover[28..34].copy_from_slice(&MAC);
        discover[236..240].copy_from_slice(&[99, 130, 83, 99]);
        discover.extend_from_slice(&[53, 1, 1, 255]);
        send(&mut c64, &udp_frame(BROADCAST, Ipv4Addr::UNSPECIFIED, Ipv4Addr::BROADCAST, 68, 67, &discover));
        let got = wait_for(&mut c64, 1);
        let ip = wire::parse_ipv4(wire::parse_ether(&got[0]).unwrap().payload).unwrap();
        let offer = wire::parse_udp(&ip).unwrap().payload;
        assert_eq!(&offer[16..20], &LEASE.octets());
        assert_eq!(&offer[240..243], &[53, 1, 2]);
        // ARP for the router
        let arp = wire::arp(&wire::Arp { op: 1, sha: MAC, spa: LEASE, tha: [0; 6], tpa: ROUTER });
        send(&mut c64, &wire::ether(BROADCAST, MAC, wire::ETH_ARP, &arp));
        let got = wait_for(&mut c64, 2);
        let reply = wire::parse_arp(wire::parse_ether(&got[1]).unwrap().payload).unwrap();
        assert_eq!((reply.op, reply.spa), (2, ROUTER));
        let router_mac = reply.sha;
        // UDP to the router reaches the host's 127.0.0.1, and the answer
        // comes back to the C64
        let server = UdpSocket::bind("127.0.0.1:0").unwrap();
        let port = server.local_addr().unwrap().port();
        send(&mut c64, &udp_frame(router_mac, LEASE, ROUTER, 4000, port, b"HELLO FROM THE C64"));
        let mut buf = [0u8; 64];
        let (n, from) = server.recv_from(&mut buf).unwrap();
        assert_eq!(&buf[..n], b"HELLO FROM THE C64");
        server.send_to(b"HELLO C64", from).unwrap();
        let got = wait_for(&mut c64, 3);
        let ip = wire::parse_ipv4(wire::parse_ether(&got[2]).unwrap().payload).unwrap();
        let u = wire::parse_udp(&ip).unwrap();
        assert_eq!((ip.src, u.dst_port, u.payload), (ROUTER, 4000, &b"HELLO C64"[..]));
        // A name resolved by the host
        let mut q = vec![0x12, 0x34, 0x01, 0x00, 0, 1, 0, 0, 0, 0, 0, 0];
        q.extend_from_slice(b"\x09localhost\x00\x00\x01\x00\x01");
        send(&mut c64, &udp_frame(router_mac, LEASE, DNS, 4001, 53, &q));
        let got = wait_for(&mut c64, 4);
        let ip = wire::parse_ipv4(wire::parse_ether(&got[3]).unwrap().payload).unwrap();
        let a = wire::parse_udp(&ip).unwrap().payload;
        assert_eq!(&a[a.len() - 4..], &ROUTER.octets());
    }

    #[test]
    fn savestate_keeps_the_chip() {
        let c64 = machine();
        let state = c64.snapshot();
        let mut other = C64::new();
        other.restore(&state).unwrap();
        let eth = other.bus.eth.as_ref().expect("cartridge in the state");
        assert_eq!(eth.chip.mac(), MAC);
        assert_eq!(eth.chip.enabled(), (true, true));
        assert!(other.network().is_some(), "a network for the restored cartridge");
    }
}
