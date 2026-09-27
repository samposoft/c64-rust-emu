// SPDX-License-Identifier: GPL-2.0-or-later
// Copyright (C) 2026 SampoSoft - Francesco Sampoli

//! Virtual router between the emulated Ethernet cartridge and the host, in
//! the manner of QEMU's user-mode networking ("slirp"): the C64 sees an
//! ordinary Ethernet LAN, and its connections become ordinary sockets of
//! the host program. No privileges, no bridging, and it works the same over
//! Wi-Fi, cable or VPN; the C64 is not visible on the real LAN.
//!
//! | address   | what it is                                                  |
//! |-----------|-------------------------------------------------------------|
//! | 10.0.2.2  | the router; connections to it reach the host's 127.0.0.1    |
//! | 10.0.2.3  | DNS: names resolved by the host                             |
//! | 10.0.2.15 | handed to the C64 by DHCP                                   |
//!
//! - ARP: the router answers for every address but the C64's own, so a
//!   C64 configured by hand on another subnet works too; probes (sender
//!   0.0.0.0) only for the router's addresses.
//! - DHCP: lease of 10.0.2.15, netmask 255.255.255.0, the router as
//!   gateway and 10.0.2.3 as DNS.
//! - DNS: A queries to 10.0.2.3, resolved in a thread by the host's
//!   resolver (hosts file and all); "localhost" becomes 10.0.2.2.
//! - TCP (`super::tcp`): every connection ended and continued by the host.
//! - UDP: one host socket per C64 port; the replies come back to it.
//! - ICMP echo: answered by the router for its own addresses; elsewhere
//!   sent with the host's unprivileged ICMP sockets (macOS, and Linux
//!   within `net.ipv4.ping_group_range`).
//! - Port forwards (`forward`): a host port on 127.0.0.1 connected to a
//!   TCP port of the C64, for servers running on it. As a real host, the
//!   router asks for the C64's address (ARP) before its first SYN: stacks
//!   such as ip65 learn the router's address from that request, and would
//!   otherwise drop the SYN to go and ask for it.
//!
//! Everything on the C64 side happens in emulated time: `exchange` runs
//! once per frame with the machine's clock in microseconds.

use std::collections::HashMap;
use std::net::{IpAddr, Ipv4Addr, SocketAddr, TcpListener, ToSocketAddrs, UdpSocket};
use std::sync::mpsc;

use super::cs8900::{Cs8900, RX_QUEUE_MAX};
use super::tcp::{Packet, Tcp};
use super::wire::{self, Mac, BROADCAST, ETH_ARP, ETH_IPV4, PROTO_ICMP, PROTO_TCP, PROTO_UDP};

pub const ROUTER: Ipv4Addr = Ipv4Addr::new(10, 0, 2, 2);
pub const DNS: Ipv4Addr = Ipv4Addr::new(10, 0, 2, 3);
pub const LEASE: Ipv4Addr = Ipv4Addr::new(10, 0, 2, 15);
const NETMASK: Ipv4Addr = Ipv4Addr::new(255, 255, 255, 0);
/// Locally administered address of the router.
const ROUTER_MAC: Mac = [0x02, 0x4E, 0x41, 0x54, 0x00, 0x02];
const LEASE_SECONDS: u32 = 86_400;
/// Emulated time after which an unused UDP port or ping is closed.
const UDP_IDLE: u64 = 120_000_000;
const PING_IDLE: u64 = 10_000_000;

struct UdpFlow {
    sock: UdpSocket,
    last: u64,
}

struct DnsJob {
    c64: Ipv4Addr,
    port: u16,
    query: Vec<u8>,
    /// End of the question section in `query`.
    question_end: usize,
    qtype: u16,
    rx: mpsc::Receiver<Result<Vec<Ipv4Addr>, ()>>,
}

struct Forward {
    host_port: u16,
    c64_port: u16,
    listener: TcpListener,
}

/// Counters, for the debugger.
#[derive(Default, Clone, Copy)]
pub struct NatStats {
    pub frames_in: u64,
    pub frames_out: u64,
    pub dns_queries: u64,
    pub dhcp_leases: u64,
}

pub struct Nat {
    c64_mac: Option<Mac>,
    /// The C64's address, from DHCP or from what it sends.
    c64_ip: Option<Ipv4Addr>,
    /// The C64 knows the router's MAC address: it asked for it, or it
    /// answered the router's ARP request.
    c64_arp: bool,
    /// When the router last asked for the C64's address.
    arp_asked: Option<u64>,
    ip_id: u16,
    tcp: Tcp,
    udp: HashMap<(Ipv4Addr, u16), UdpFlow>,
    #[cfg(unix)]
    pings: HashMap<(Ipv4Addr, Ipv4Addr, u16), ping::Ping>,
    dns: Vec<DnsJob>,
    forwards: Vec<Forward>,
    next_forward_port: u16,
    /// Packets for the C64, before the Ethernet header.
    out: Vec<Packet>,
    /// Frames ready for the C64 (with their own Ethernet destination).
    frames: Vec<Vec<u8>>,
    pub stats: NatStats,
}

impl Default for Nat {
    fn default() -> Self {
        Self::new()
    }
}

impl Nat {
    pub fn new() -> Self {
        Nat {
            c64_mac: None,
            c64_ip: None,
            c64_arp: false,
            arp_asked: None,
            ip_id: 0,
            tcp: Tcp::new(),
            udp: HashMap::new(),
            #[cfg(unix)]
            pings: HashMap::new(),
            dns: Vec::new(),
            forwards: Vec::new(),
            next_forward_port: 49152,
            out: Vec::new(),
            frames: Vec::new(),
            stats: NatStats::default(),
        }
    }

    /// Drops every connection, flow and pending query (machine reset,
    /// savestate loaded): the C64 on the other side no longer knows them.
    /// The port forwards stay.
    pub fn reset(&mut self) {
        self.tcp.reset();
        self.udp.clear();
        #[cfg(unix)]
        self.pings.clear();
        self.dns.clear();
        self.out.clear();
        self.frames.clear();
        // A C64 that starts again has forgotten the router's address
        self.c64_arp = false;
        self.arp_asked = None;
    }

    /// Forwards the host's TCP port `host_port` (on 127.0.0.1) to port
    /// `c64_port` of the C64.
    pub fn forward(&mut self, host_port: u16, c64_port: u16) -> Result<(), String> {
        if self.forwards.iter().any(|f| f.host_port == host_port) {
            return Err(format!("host port {host_port} is already forwarded"));
        }
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, host_port))
            .map_err(|e| format!("cannot listen on 127.0.0.1:{host_port}: {e}"))?;
        listener.set_nonblocking(true).map_err(|e| e.to_string())?;
        self.forwards.push(Forward { host_port, c64_port, listener });
        Ok(())
    }

    /// The port forwards: (host port, C64 port).
    pub fn forwards(&self) -> Vec<(u16, u16)> {
        self.forwards.iter().map(|f| (f.host_port, f.c64_port)).collect()
    }

    /// State of the network, for the debugger.
    pub fn describe(&self) -> String {
        let mut s = format!(
            "  Network: router {ROUTER} (the host's 127.0.0.1), DNS {DNS}, DHCP lease {LEASE}/24\n  C64: MAC {}, IP {}\n",
            self.c64_mac.map_or("not seen yet".into(), |m| mac_str(&m)),
            self.c64_ip.map_or("not seen yet".into(), |ip| ip.to_string()),
        );
        for f in &self.forwards {
            s += &format!("  Forward: 127.0.0.1:{} -> C64 port {}\n", f.host_port, f.c64_port);
        }
        for line in self.tcp.describe() {
            s += &format!("  {line}\n");
        }
        #[cfg(unix)]
        let pings = self.pings.len();
        #[cfg(not(unix))]
        let pings = 0;
        s += &format!(
            "  {} TCP, {} UDP, {} ping; frames {} from the C64, {} to it; {} DNS queries, {} DHCP leases\n",
            self.tcp.count(), self.udp.len(), pings, self.stats.frames_in, self.stats.frames_out,
            self.stats.dns_queries, self.stats.dhcp_leases,
        );
        s
    }

    /// Takes the frames the chip sent, runs the router and the host sockets,
    /// and queues the answers for the chip. `now`: emulated time in
    /// microseconds.
    pub fn exchange(&mut self, chip: &mut Cs8900, now: u64) {
        for f in std::mem::take(&mut chip.tx_queue) {
            self.stats.frames_in += 1;
            self.frame(&f, now);
        }
        self.poll(now);
        for f in self.frames.drain(..) {
            if chip.rx_queue.len() >= RX_QUEUE_MAX {
                chip.stats.dropped += 1;
                continue;
            }
            self.stats.frames_out += 1;
            chip.rx_queue.push_back(f);
        }
    }

    // ── From the C64 ──────────────────────────────────────────────────────────

    fn frame(&mut self, f: &[u8], now: u64) {
        let Some(e) = wire::parse_ether(f) else { return };
        if e.dst != BROADCAST && e.dst != ROUTER_MAC {
            return;
        }
        self.c64_mac = Some(e.src);
        match e.ethertype {
            ETH_ARP => self.arp(e.payload),
            ETH_IPV4 => self.ipv4(e.payload, now),
            _ => {}
        }
    }

    fn arp(&mut self, p: &[u8]) {
        let Some(a) = wire::parse_arp(p) else { return };
        if a.op == wire::ARP_REPLY && a.tpa == ROUTER && !a.spa.is_unspecified() {
            // The answer to the router's request
            self.c64_ip = Some(a.spa);
            self.c64_arp = true;
            return;
        }
        // Not the announcements of the C64's own address, nor the probes
        // (sender 0.0.0.0) of an address the C64 wants: those must go
        // unanswered, or it would see a conflict. A probe of the router's
        // own addresses is answered, as a real router does
        let own = a.tpa == ROUTER || a.tpa == DNS;
        if a.op != wire::ARP_REQUEST || a.tpa == a.spa || (a.spa.is_unspecified() && !own) {
            return;
        }
        if !a.spa.is_unspecified() {
            self.c64_ip = Some(a.spa);
            if a.tpa == ROUTER {
                self.c64_arp = true;
            }
        }
        let reply = wire::arp(&wire::Arp { op: wire::ARP_REPLY, sha: ROUTER_MAC, spa: a.tpa, tha: a.sha, tpa: a.spa });
        self.frames.push(wire::ether(a.sha, ROUTER_MAC, ETH_ARP, &reply));
    }

    fn ipv4(&mut self, p: &[u8], now: u64) {
        let Some(ip) = wire::parse_ipv4(p) else { return };
        if !ip.src.is_unspecified() && !ip.src.is_broadcast() {
            self.c64_ip = Some(ip.src);
        }
        match ip.proto {
            PROTO_UDP => {
                let Some(u) = wire::parse_udp(&ip) else { return };
                if u.dst_port == 67 && (ip.dst.is_broadcast() || ip.dst == ROUTER) {
                    self.dhcp(u.payload);
                } else if ip.dst == DNS && u.dst_port == 53 {
                    self.dns_query(ip.src, u.src_port, u.payload);
                } else if let Some(target) = host_addr(ip.dst, u.dst_port) {
                    self.udp_send(ip.src, u.src_port, target, u.payload, now);
                }
            }
            PROTO_TCP => {
                let Some(seg) = wire::parse_tcp(&ip) else { return };
                let target = host_addr(ip.dst, seg.dst_port);
                self.tcp.segment(ip.src, ip.dst, &seg, || target, now, &mut self.out);
            }
            PROTO_ICMP => {
                let Some((kind, _, rest)) = wire::parse_icmp(ip.payload) else { return };
                if kind != wire::ICMP_ECHO_REQUEST {
                    return;
                }
                if ip.dst == ROUTER || ip.dst == DNS {
                    self.out.push(Packet {
                        src: ip.dst, dst: ip.src, proto: PROTO_ICMP,
                        data: wire::icmp(wire::ICMP_ECHO_REPLY, 0, rest),
                    });
                } else if host_addr(ip.dst, 0).is_some() {
                    #[cfg(unix)]
                    self.ping(ip.src, ip.dst, rest, now);
                }
            }
            _ => {}
        }
    }

    fn udp_send(&mut self, c64: Ipv4Addr, port: u16, target: SocketAddr, data: &[u8], now: u64) {
        let flow = match self.udp.entry((c64, port)) {
            std::collections::hash_map::Entry::Occupied(e) => e.into_mut(),
            std::collections::hash_map::Entry::Vacant(e) => {
                let Ok(sock) = UdpSocket::bind((Ipv4Addr::UNSPECIFIED, 0)) else { return };
                if sock.set_nonblocking(true).is_err() {
                    return;
                }
                e.insert(UdpFlow { sock, last: now })
            }
        };
        flow.last = now;
        let _ = flow.sock.send_to(data, target);
    }

    // ── DHCP ──────────────────────────────────────────────────────────────────

    fn dhcp(&mut self, p: &[u8]) {
        if p.len() < 240 || p[0] != 1 || p[236..240] != [99, 130, 83, 99] {
            return;
        }
        let (mut msg, mut requested) = (0u8, None);
        let mut opts = &p[240..];
        while let [code, rest @ ..] = opts {
            match code {
                0 => opts = rest,
                255 => break,
                _ => {
                    let Some((&len, rest)) = rest.split_first() else { break };
                    let Some(value) = rest.get(..len as usize) else { break };
                    match (code, value) {
                        (53, [t]) => msg = *t,
                        (50, [a, b, c, d]) => requested = Some(Ipv4Addr::new(*a, *b, *c, *d)),
                        _ => {}
                    }
                    opts = &rest[len as usize..];
                }
            }
        }
        let ciaddr = Ipv4Addr::new(p[12], p[13], p[14], p[15]);
        let (reply, yiaddr) = match msg {
            1 => (2, LEASE), // DISCOVER -> OFFER
            3 => {
                // REQUEST: ACK for our address, NAK for any other
                let wanted = requested.unwrap_or(ciaddr);
                if wanted == LEASE || wanted.is_unspecified() { (5, LEASE) } else { (6, Ipv4Addr::UNSPECIFIED) }
            }
            8 => (5, Ipv4Addr::UNSPECIFIED), // INFORM -> ACK, no address
            _ => return,
        };
        let mut r = vec![0u8; 240];
        r[0] = 2;
        r[1] = 1;
        r[2] = 6;
        r[4..8].copy_from_slice(&p[4..8]); // xid
        r[10..12].copy_from_slice(&p[10..12]); // flags
        r[12..16].copy_from_slice(&p[12..16]); // ciaddr
        r[16..20].copy_from_slice(&yiaddr.octets());
        r[20..24].copy_from_slice(&ROUTER.octets());
        r[28..44].copy_from_slice(&p[28..44]); // chaddr
        r[236..240].copy_from_slice(&[99, 130, 83, 99]);
        r.extend_from_slice(&[53, 1, reply, 54, 4]);
        r.extend_from_slice(&ROUTER.octets());
        if reply != 6 {
            if reply == 2 || yiaddr == LEASE {
                r.extend_from_slice(&[51, 4]);
                r.extend_from_slice(&LEASE_SECONDS.to_be_bytes());
            }
            r.extend_from_slice(&[1, 4]);
            r.extend_from_slice(&NETMASK.octets());
            r.extend_from_slice(&[3, 4]);
            r.extend_from_slice(&ROUTER.octets());
            r.extend_from_slice(&[6, 4]);
            r.extend_from_slice(&DNS.octets());
        }
        r.push(255);
        r.resize(r.len().max(300), 0);
        if reply == 5 && yiaddr == LEASE {
            self.c64_ip = Some(LEASE);
            self.stats.dhcp_leases += 1;
        }
        // To the broadcast address: the C64 has no address yet
        let udp = wire::udp(ROUTER, Ipv4Addr::BROADCAST, 67, 68, &r);
        let id = self.next_id();
        let ip = wire::ipv4(ROUTER, Ipv4Addr::BROADCAST, PROTO_UDP, id, &udp);
        self.frames.push(wire::ether(BROADCAST, ROUTER_MAC, ETH_IPV4, &ip));
    }

    // ── DNS ───────────────────────────────────────────────────────────────────

    fn dns_query(&mut self, c64: Ipv4Addr, port: u16, q: &[u8]) {
        if q.len() < 12 || q[2] & 0x80 != 0 || u16::from_be_bytes([q[4], q[5]]) != 1 {
            return;
        }
        self.stats.dns_queries += 1;
        // The name, label by label
        let mut name = String::new();
        let mut i = 12;
        loop {
            let Some(&len) = q.get(i) else { return };
            i += 1;
            if len == 0 {
                break;
            }
            if len & 0xC0 != 0 || i + len as usize > q.len() {
                return;
            }
            if !name.is_empty() {
                name.push('.');
            }
            name.push_str(&String::from_utf8_lossy(&q[i..i + len as usize]));
            i += len as usize;
        }
        let Some(tail) = q.get(i..i + 4) else { return };
        let qtype = u16::from_be_bytes([tail[0], tail[1]]);
        let qclass = u16::from_be_bytes([tail[2], tail[3]]);
        let question_end = i + 4;
        let opcode = (q[2] >> 3) & 0x0F;
        let (tx, rx) = mpsc::channel();
        if opcode == 0 && qclass == 1 && matches!(qtype, 1 | 28 | 255) && !name.is_empty() {
            std::thread::spawn(move || {
                let r = (name.as_str(), 0).to_socket_addrs().map_err(|_| ()).map(|addrs| {
                    let mut v: Vec<Ipv4Addr> = Vec::new();
                    for a in addrs {
                        if let IpAddr::V4(ip) = a.ip() {
                            if !v.contains(&ip) {
                                v.push(ip);
                            }
                        }
                    }
                    v
                });
                let _ = tx.send(r);
            });
        }
        // Otherwise the channel is closed at once: "not implemented"
        self.dns.push(DnsJob { c64, port, query: q.to_vec(), question_end, qtype, rx });
    }

    fn dns_answer(&mut self, job: &DnsJob, result: Option<Result<Vec<Ipv4Addr>, ()>>) {
        let q = &job.query;
        let (rcode, addrs) = match result {
            None => (4, Vec::new()),                     // NOTIMP
            Some(Err(())) => (3, Vec::new()),            // NXDOMAIN
            Some(Ok(_)) if job.qtype == 28 => (0, Vec::new()), // no IPv6 addresses
            Some(Ok(v)) if v.is_empty() => (3, v),
            Some(Ok(v)) => (0, v),
        };
        let addrs: Vec<Ipv4Addr> = addrs.into_iter().take(8).map(|a| if a.is_loopback() { ROUTER } else { a }).collect();
        let mut r = Vec::with_capacity(job.question_end + 16 * addrs.len());
        r.extend_from_slice(&q[0..2]);
        r.push(0x80 | (q[2] & 0x79)); // QR, opcode and RD from the query
        r.push(0x80 | rcode); // RA
        r.extend_from_slice(&[0, 1]);
        r.extend_from_slice(&(addrs.len() as u16).to_be_bytes());
        r.extend_from_slice(&[0, 0, 0, 0]);
        r.extend_from_slice(&q[12..job.question_end]);
        for a in &addrs {
            // Name pointer to the question, A, IN, TTL 60 s
            r.extend_from_slice(&[0xC0, 12, 0, 1, 0, 1, 0, 0, 0, 60, 0, 4]);
            r.extend_from_slice(&a.octets());
        }
        self.out.push(Packet { src: DNS, dst: job.c64, proto: PROTO_UDP, data: wire::udp(DNS, job.c64, 53, job.port, &r) });
    }

    // ── ICMP echo through the host ────────────────────────────────────────────

    #[cfg(unix)]
    fn ping(&mut self, c64: Ipv4Addr, remote: Ipv4Addr, rest: &[u8], now: u64) {
        if rest.len() < 4 {
            return;
        }
        let id = u16::from_be_bytes([rest[0], rest[1]]);
        let p = match self.pings.entry((c64, remote, id)) {
            std::collections::hash_map::Entry::Occupied(e) => e.into_mut(),
            std::collections::hash_map::Entry::Vacant(e) => match ping::Ping::open() {
                Some(p) => e.insert(p),
                None => return,
            },
        };
        p.last = now;
        p.send(remote, &wire::icmp(wire::ICMP_ECHO_REQUEST, 0, rest));
    }

    // ── Host side ─────────────────────────────────────────────────────────────

    fn poll(&mut self, now: u64) {
        // New connections on the port forwards
        for i in 0..self.forwards.len() {
            while let Ok((stream, _)) = self.forwards[i].listener.accept() {
                let Some(c64) = self.c64_ip else { continue };
                let c64_port = self.forwards[i].c64_port;
                let port = self.tcp.free_port(c64, c64_port, ROUTER, self.next_forward_port);
                self.next_forward_port = if port == u16::MAX { 49152 } else { port + 1 };
                self.tcp.open(stream, c64, c64_port, ROUTER, port, now);
            }
        }
        // A connection to open towards the C64: first ask for its address,
        // as a real host does, once a second
        if self.tcp.awaiting_neighbour() && !self.c64_arp {
            if let Some(ip) = self.c64_ip {
                if self.arp_asked.is_none_or(|t| now >= t + 1_000_000) {
                    self.arp_asked = Some(now);
                    let req = wire::arp(&wire::Arp { op: wire::ARP_REQUEST, sha: ROUTER_MAC, spa: ROUTER, tha: [0; 6], tpa: ip });
                    self.frames.push(wire::ether(BROADCAST, ROUTER_MAC, ETH_ARP, &req));
                }
            }
        }
        self.tcp.poll(now, self.c64_arp, &mut self.out);
        // UDP replies
        let mut buf = [0u8; 2048];
        for (&(c64, port), flow) in self.udp.iter_mut() {
            while let Ok((n, from)) = flow.sock.recv_from(&mut buf) {
                let SocketAddr::V4(from) = from else { continue };
                let src = if from.ip().is_loopback() { ROUTER } else { *from.ip() };
                flow.last = now;
                self.out.push(Packet { src, dst: c64, proto: PROTO_UDP, data: wire::udp(src, c64, from.port(), port, &buf[..n]) });
            }
        }
        self.udp.retain(|_, f| now < f.last + UDP_IDLE);
        // Ping replies
        #[cfg(unix)]
        {
            for (&(c64, remote, id), p) in self.pings.iter_mut() {
                while let Some(mut rest) = p.recv() {
                    // The id as the C64 sent it (Linux puts its own)
                    rest[0..2].copy_from_slice(&id.to_be_bytes());
                    self.out.push(Packet { src: remote, dst: c64, proto: PROTO_ICMP, data: wire::icmp(wire::ICMP_ECHO_REPLY, 0, &rest) });
                }
            }
            self.pings.retain(|_, p| now < p.last + PING_IDLE);
        }
        // Resolved names
        let mut jobs = std::mem::take(&mut self.dns);
        jobs.retain(|job| match job.rx.try_recv() {
            Ok(r) => { self.dns_answer(job, Some(r)); false }
            Err(mpsc::TryRecvError::Disconnected) => { self.dns_answer(job, None); false }
            Err(mpsc::TryRecvError::Empty) => true,
        });
        self.dns.extend(jobs);
        // Ethernet frames of everything produced
        let Some(mac) = self.c64_mac else {
            self.out.clear();
            return;
        };
        for p in std::mem::take(&mut self.out) {
            let id = self.next_id();
            let ip = wire::ipv4(p.src, p.dst, p.proto, id, &p.data);
            self.frames.push(wire::ether(mac, ROUTER_MAC, ETH_IPV4, &ip));
        }
    }

    fn next_id(&mut self) -> u16 {
        self.ip_id = self.ip_id.wrapping_add(1);
        self.ip_id
    }
}

/// Host address for a destination of the C64: the router is the host
/// itself, the virtual LAN and the special addresses go nowhere.
fn host_addr(dst: Ipv4Addr, port: u16) -> Option<SocketAddr> {
    if dst == ROUTER {
        return Some(SocketAddr::from((Ipv4Addr::LOCALHOST, port)));
    }
    let o = dst.octets();
    let lan = o[0] == 10 && o[1] == 0 && o[2] == 2;
    if lan || dst.is_unspecified() || dst.is_broadcast() || dst.is_multicast() || dst.is_loopback() {
        return None;
    }
    Some(SocketAddr::from((dst, port)))
}

fn mac_str(m: &Mac) -> String {
    m.iter().map(|b| format!("{b:02X}")).collect::<Vec<_>>().join(":")
}

/// Unprivileged ICMP echo sockets (`SOCK_DGRAM`, `IPPROTO_ICMP`).
#[cfg(unix)]
mod ping {
    use std::net::Ipv4Addr;
    use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};

    pub struct Ping {
        fd: OwnedFd,
        pub last: u64,
    }

    impl Ping {
        pub fn open() -> Option<Ping> {
            // SAFETY: plain socket creation; the descriptor is owned at once
            let fd = unsafe { libc::socket(libc::AF_INET, libc::SOCK_DGRAM, libc::IPPROTO_ICMP) };
            if fd < 0 {
                return None;
            }
            // SAFETY: fd is a valid descriptor that nothing else owns
            let fd = unsafe { OwnedFd::from_raw_fd(fd) };
            // SAFETY: fcntl on a valid descriptor
            unsafe {
                let flags = libc::fcntl(fd.as_raw_fd(), libc::F_GETFL);
                libc::fcntl(fd.as_raw_fd(), libc::F_SETFL, flags | libc::O_NONBLOCK);
            }
            Some(Ping { fd, last: 0 })
        }

        pub fn send(&self, to: Ipv4Addr, msg: &[u8]) {
            // SAFETY: a zeroed sockaddr_in is valid; the fields set below
            let mut sa: libc::sockaddr_in = unsafe { std::mem::zeroed() };
            sa.sin_family = libc::AF_INET as _;
            sa.sin_addr.s_addr = u32::from_ne_bytes(to.octets());
            #[cfg(any(target_os = "macos", target_os = "ios", target_os = "freebsd"))]
            {
                sa.sin_len = std::mem::size_of::<libc::sockaddr_in>() as u8;
            }
            // SAFETY: msg and sa outlive the call, lengths are theirs
            unsafe {
                libc::sendto(
                    self.fd.as_raw_fd(), msg.as_ptr().cast(), msg.len(), 0,
                    (&sa as *const libc::sockaddr_in).cast(), std::mem::size_of::<libc::sockaddr_in>() as _,
                );
            }
        }

        /// The next echo reply: the ICMP message after the checksum.
        pub fn recv(&self) -> Option<Vec<u8>> {
            let mut buf = [0u8; 2048];
            loop {
                // SAFETY: buf is writable for its length
                let n = unsafe { libc::recv(self.fd.as_raw_fd(), buf.as_mut_ptr().cast(), buf.len(), 0) };
                if n <= 0 {
                    return None;
                }
                let mut m = &buf[..n as usize];
                // macOS returns the IP header too, Linux only the message
                if m[0] >> 4 == 4 && m.len() >= 20 {
                    let ihl = (m[0] & 0x0F) as usize * 4;
                    m = &m[ihl.min(m.len())..];
                }
                if m.len() >= 8 && m[0] == super::wire::ICMP_ECHO_REPLY {
                    return Some(m[4..].to_vec());
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::net::wire::{parse_ether, parse_ipv4, parse_tcp, parse_udp, TcpOut, ACK, FIN, PSH, SYN};
    use std::io::{Read, Write};

    const C64_MAC: Mac = [0x02, 0xC6, 0x4C, 0x64, 0x00, 0x01];

    /// The router with a chip in between, driven by hand-made frames.
    struct Rig {
        nat: Nat,
        chip: Cs8900,
        now: u64,
    }

    impl Rig {
        fn new() -> Self {
            Rig { nat: Nat::new(), chip: Cs8900::new(), now: 0 }
        }

        /// Sends frames from the C64 and runs 20 ms (one frame).
        fn send(&mut self, frames: Vec<Vec<u8>>) -> Vec<Vec<u8>> {
            self.chip.tx_queue.extend(frames);
            self.step()
        }

        fn step(&mut self) -> Vec<Vec<u8>> {
            self.now += 20_000;
            self.nat.exchange(&mut self.chip, self.now);
            self.chip.rx_queue.drain(..).collect()
        }

        /// Steps until a frame comes back, for up to `ms` of real time.
        fn wait(&mut self, ms: u64) -> Vec<Vec<u8>> {
            for _ in 0..ms / 5 {
                let got = self.step();
                if !got.is_empty() {
                    return got;
                }
                std::thread::sleep(std::time::Duration::from_millis(5));
            }
            Vec::new()
        }
    }

    fn ip_frame(src: Ipv4Addr, dst: Ipv4Addr, proto: u8, payload: &[u8]) -> Vec<u8> {
        wire::ether(ROUTER_MAC, C64_MAC, ETH_IPV4, &wire::ipv4(src, dst, proto, 1, payload))
    }

    fn tcp_frame(dst: Ipv4Addr, port: u16, seq: u32, ack: u32, flags: u8, data: &[u8]) -> Vec<u8> {
        let seg = wire::tcp(LEASE, dst, &TcpOut {
            src_port: 1025, dst_port: port, seq, ack, flags, window: 4096,
            mss: (flags & SYN != 0).then_some(1460), payload: data,
        });
        ip_frame(LEASE, dst, PROTO_TCP, &seg)
    }

    #[test]
    fn arp_for_any_address_but_its_own() {
        let mut r = Rig::new();
        let req = |tpa: Ipv4Addr, spa: Ipv4Addr| {
            let a = wire::arp(&wire::Arp { op: 1, sha: C64_MAC, spa, tha: [0; 6], tpa });
            wire::ether(BROADCAST, C64_MAC, ETH_ARP, &a)
        };
        let got = r.send(vec![req(ROUTER, LEASE)]);
        assert_eq!(got.len(), 1);
        let a = wire::parse_arp(parse_ether(&got[0]).unwrap().payload).unwrap();
        assert_eq!((a.op, a.sha, a.spa, a.tpa), (2, ROUTER_MAC, ROUTER, LEASE));
        // Another subnet: answered too; probes and announcements of the
        // C64's address are not, probes of the router's are
        assert_eq!(r.send(vec![req(Ipv4Addr::new(192, 168, 1, 1), Ipv4Addr::new(192, 168, 1, 64))]).len(), 1);
        assert!(r.send(vec![req(LEASE, Ipv4Addr::UNSPECIFIED), req(LEASE, LEASE)]).is_empty());
        assert_eq!(r.send(vec![req(ROUTER, Ipv4Addr::UNSPECIFIED)]).len(), 1);
    }

    #[test]
    fn dhcp_discover_and_request() {
        let mut r = Rig::new();
        let dhcp = |msg: u8| {
            let mut p = vec![0u8; 240];
            p[0] = 1;
            p[1] = 1;
            p[2] = 6;
            p[4..8].copy_from_slice(&[1, 2, 3, 4]);
            p[28..34].copy_from_slice(&C64_MAC);
            p[236..240].copy_from_slice(&[99, 130, 83, 99]);
            p.extend_from_slice(&[53, 1, msg, 255]);
            let u = wire::udp(Ipv4Addr::UNSPECIFIED, Ipv4Addr::BROADCAST, 68, 67, &p);
            wire::ether(BROADCAST, C64_MAC, ETH_IPV4, &wire::ipv4(Ipv4Addr::UNSPECIFIED, Ipv4Addr::BROADCAST, PROTO_UDP, 1, &u))
        };
        for (msg, want) in [(1, 2), (3, 5)] {
            let got = r.send(vec![dhcp(msg)]);
            assert_eq!(got.len(), 1);
            let e = parse_ether(&got[0]).unwrap();
            let ip = parse_ipv4(e.payload).unwrap();
            let u = parse_udp(&ip).unwrap();
            assert_eq!((u.src_port, u.dst_port), (67, 68));
            assert_eq!(&u.payload[16..20], &LEASE.octets(), "yiaddr");
            assert_eq!(&u.payload[240..243], &[53, 1, want]);
        }
        assert_eq!(r.nat.c64_ip, Some(LEASE));
    }

    #[test]
    fn dns_resolves_localhost_to_the_router() {
        let mut r = Rig::new();
        let mut q = vec![0xAB, 0xCD, 0x01, 0x00, 0, 1, 0, 0, 0, 0, 0, 0];
        q.extend_from_slice(b"\x09localhost\x00\x00\x01\x00\x01");
        let u = wire::udp(LEASE, DNS, 1234, 53, &q);
        let mut got = r.send(vec![ip_frame(LEASE, DNS, PROTO_UDP, &u)]);
        if got.is_empty() {
            got = r.wait(2000);
        }
        let e = parse_ether(&got[0]).unwrap();
        let ip = parse_ipv4(e.payload).unwrap();
        let a = parse_udp(&ip).unwrap().payload;
        assert_eq!(&a[0..2], &[0xAB, 0xCD]);
        assert_eq!(a[3] & 0x0F, 0, "no error");
        assert!(u16::from_be_bytes([a[6], a[7]]) >= 1, "an answer");
        assert_eq!(&a[a.len() - 4..], &ROUTER.octets());
    }

    #[test]
    fn ping_the_router() {
        let mut r = Rig::new();
        let echo = wire::icmp(wire::ICMP_ECHO_REQUEST, 0, &[0x12, 0x34, 0, 1, b'h', b'i']);
        let got = r.send(vec![ip_frame(LEASE, ROUTER, PROTO_ICMP, &echo)]);
        let ip = parse_ipv4(parse_ether(&got[0]).unwrap().payload).unwrap();
        assert_eq!(wire::parse_icmp(ip.payload), Some((0, 0, &[0x12, 0x34, 0, 1, b'h', b'i'][..])));
    }

    #[test]
    fn udp_to_the_host() {
        let server = UdpSocket::bind("127.0.0.1:0").unwrap();
        let port = server.local_addr().unwrap().port();
        let mut r = Rig::new();
        let u = wire::udp(LEASE, ROUTER, 3000, port, b"ping");
        r.send(vec![ip_frame(LEASE, ROUTER, PROTO_UDP, &u)]);
        let mut buf = [0u8; 16];
        let (n, from) = server.recv_from(&mut buf).unwrap();
        assert_eq!(&buf[..n], b"ping");
        server.send_to(b"pong", from).unwrap();
        let got = r.wait(2000);
        let ip = parse_ipv4(parse_ether(&got[0]).unwrap().payload).unwrap();
        assert_eq!(ip.src, ROUTER);
        let u = parse_udp(&ip).unwrap();
        assert_eq!((u.src_port, u.dst_port, u.payload), (port, 3000, &b"pong"[..]));
    }

    /// The TCP segments in the frames.
    fn segments(frames: &[Vec<u8>]) -> Vec<(u32, u32, u8, Vec<u8>)> {
        frames.iter().filter_map(|f| {
            let ip = parse_ipv4(parse_ether(f)?.payload)?;
            let t = parse_tcp(&ip)?;
            Some((t.seq, t.ack, t.flags, t.payload.to_vec()))
        }).collect()
    }

    #[test]
    fn tcp_connection_to_the_host() {
        let server = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = server.local_addr().unwrap().port();
        let mut r = Rig::new();
        // SYN: answered once the host socket is connected
        let syn_ack = segments(&{
            r.send(vec![tcp_frame(ROUTER, port, 1000, 0, SYN, &[])]);
            r.wait(2000)
        });
        let (iss, ack, flags, _) = syn_ack[0];
        assert_eq!((ack, flags), (1001, SYN | ACK));
        let (mut host, _) = server.accept().unwrap();
        // Data from the C64 reaches the host
        let got = segments(&r.send(vec![tcp_frame(ROUTER, port, 1001, iss + 1, ACK | PSH, b"GET /")]));
        assert_eq!(got.last().unwrap().1, 1006, "acknowledged");
        let mut buf = [0u8; 5];
        host.read_exact(&mut buf).unwrap();
        assert_eq!(&buf, b"GET /");
        // Data and FIN from the host reach the C64
        host.write_all(b"hello").unwrap();
        drop(host);
        let mut data = Vec::new();
        let mut fin = None;
        for _ in 0..100 {
            for (seq, _, flags, payload) in segments(&r.wait(50)) {
                data.extend(payload);
                if flags & FIN != 0 {
                    fin = Some(seq);
                }
            }
            if fin.is_some() {
                break;
            }
        }
        assert_eq!(data, b"hello");
        assert_eq!(fin, Some(iss + 6));
        // The C64 acknowledges and closes: the connection goes away
        r.send(vec![tcp_frame(ROUTER, port, 1006, iss + 7, ACK | FIN, &[])]);
        r.now += 3_000_000;
        r.step();
        assert_eq!(r.nat.tcp.count(), 0);
    }

    #[test]
    fn tcp_refused_and_unknown() {
        // A port nobody listens on: RST
        let port = std::net::TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port();
        let mut r = Rig::new();
        r.send(vec![tcp_frame(ROUTER, port, 50, 0, SYN, &[])]);
        let got = segments(&r.wait(2000));
        assert_eq!((got[0].1, got[0].2), (51, wire::RST | ACK));
        // A segment of no connection: RST with its acknowledgement number
        let got = segments(&r.send(vec![tcp_frame(ROUTER, 80, 7, 1234, ACK, b"x")]));
        assert_eq!((got[0].0, got[0].2), (1234, wire::RST));
    }

    #[test]
    fn tcp_retransmits_until_acknowledged() {
        let server = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = server.local_addr().unwrap().port();
        let mut r = Rig::new();
        r.send(vec![tcp_frame(ROUTER, port, 1000, 0, SYN, &[])]);
        let (iss, ..) = segments(&r.wait(2000))[0];
        let (mut host, _) = server.accept().unwrap();
        r.send(vec![tcp_frame(ROUTER, port, 1001, iss + 1, ACK, &[])]);
        host.write_all(b"data").unwrap();
        let first = segments(&r.wait(2000));
        assert_eq!(first[0].3, b"data");
        // Not acknowledged: the same data again after the timeout
        r.now += 1_100_000;
        let again = segments(&r.step());
        assert_eq!((again[0].0, &again[0].3[..]), (iss + 1, &b"data"[..]));
    }

    #[test]
    fn port_forward_to_the_c64() {
        let mut r = Rig::new();
        let probe = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let host_port = probe.local_addr().unwrap().port();
        drop(probe);
        r.nat.forward(host_port, 80).unwrap();
        // The C64's address comes from what it sends
        let echo = wire::icmp(wire::ICMP_ECHO_REQUEST, 0, &[0, 1, 0, 1]);
        r.send(vec![ip_frame(LEASE, ROUTER, PROTO_ICMP, &echo)]);
        let mut client = std::net::TcpStream::connect(("127.0.0.1", host_port)).unwrap();
        // The router asks for the C64's address before its SYN
        let asked = r.wait(2000);
        let a = wire::parse_arp(parse_ether(&asked[0]).unwrap().payload).unwrap();
        assert_eq!((a.op, a.spa, a.tpa), (1, ROUTER, LEASE));
        assert!(segments(&asked).is_empty(), "no SYN before the ARP answer");
        let answer = wire::arp(&wire::Arp { op: 2, sha: C64_MAC, spa: LEASE, tha: ROUTER_MAC, tpa: ROUTER });
        let syn = segments(&r.send(vec![wire::ether(ROUTER_MAC, C64_MAC, ETH_ARP, &answer)]));
        let (iss, _, flags, _) = syn[0];
        assert_eq!(flags, SYN);
        // The first forwarded connection comes from router port 49152
        let frame = |seq: u32, ack: u32, flags: u8, data: &[u8]| {
            let seg = wire::tcp(LEASE, ROUTER, &TcpOut {
                src_port: 80, dst_port: 49152, seq, ack, flags, window: 4096, mss: Some(1460), payload: data,
            });
            ip_frame(LEASE, ROUTER, PROTO_TCP, &seg)
        };
        let got = segments(&r.send(vec![frame(500, iss + 1, SYN | ACK, &[])]));
        assert_eq!((got[0].1, got[0].2), (501, ACK));
        r.send(vec![frame(501, iss + 1, ACK | PSH, b"hi!")]);
        let mut buf = [0u8; 3];
        client.read_exact(&mut buf).unwrap();
        assert_eq!(&buf, b"hi!");
    }
}
