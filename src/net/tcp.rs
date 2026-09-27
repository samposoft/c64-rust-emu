// SPDX-License-Identifier: GPL-2.0-or-later
// Copyright (C) 2026 SampoSoft - Francesco Sampoli

//! TCP connections of the virtual router. Every connection of the C64 is
//! ended here and continued as an ordinary socket of the host: the C64
//! talks TCP with the router, the host with the remote server, and the
//! bytes are copied between the two.
//!
//! The C64 side runs in emulated time (microseconds of the machine's
//! clock): retransmissions and timeouts follow the C64, so pausing it in
//! the debugger or running it in warp does not break its connections. The
//! host side is the operating system's, in real time; the buffers in
//! between absorb the difference, and when they are full the router stops
//! reading, so TCP flow control holds the remote server back.
//!
//! A connection the C64 opens (SYN) is answered only once the host socket
//! is connected: SYN-ACK if it succeeds, RST if it is refused, as a real
//! server would. The port forwards (`Nat::forward`) open connections the
//! other way, from the router to the C64.

use std::collections::{HashMap, VecDeque};
use std::io::{ErrorKind, Read, Write};
use std::net::{Ipv4Addr, Shutdown, SocketAddr, TcpStream};
use std::sync::mpsc;
use std::time::Duration;

use super::wire::{self, TcpOut, ACK, FIN, PROTO_TCP, PSH, RST, SYN};

/// Our maximum segment size: what the C64 may send in one segment.
const OUR_MSS: u16 = 1460;
/// MSS assumed when the C64's SYN does not say (RFC 1122).
const DEFAULT_MSS: usize = 536;
/// Bytes read from the host and not yet acknowledged by the C64, at most.
const SEND_LIMIT: usize = 64 * 1024;
/// Bytes from the C64 not yet written to the host, at most: our window.
const RECV_LIMIT: usize = 16 * 1024;
const RTO_INITIAL: u64 = 1_000_000;
const RTO_MAX: u64 = 30_000_000;
/// Retransmissions of the same data before the connection is reset.
const MAX_RETRIES: u32 = 10;
/// A closed connection stays this long, to acknowledge a repeated FIN.
const TIME_WAIT: u64 = 2_000_000;
/// Real time the host has to connect.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
/// Emulated time a forwarded connection waits for the C64 to answer ARP.
const NEIGHBOUR_TIMEOUT: u64 = 10_000_000;
const MAX_CONNS: usize = 64;

/// Packet for the C64, to be put in an IPv4 frame.
pub(super) struct Packet {
    pub src: Ipv4Addr,
    pub dst: Ipv4Addr,
    pub proto: u8,
    pub data: Vec<u8>,
}

fn lt(a: u32, b: u32) -> bool {
    (a.wrapping_sub(b) as i32) < 0
}

fn le(a: u32, b: u32) -> bool {
    !lt(b, a)
}

/// C64 address and port, remote address and port (as the C64 sees it).
type Key = (Ipv4Addr, u16, Ipv4Addr, u16);

enum Host {
    /// Connection in progress, in a thread.
    Connecting(mpsc::Receiver<std::io::Result<TcpStream>>),
    Open(TcpStream),
}

struct Conn {
    host: Host,
    /// Where the host socket goes, for the debugger.
    target: SocketAddr,
    /// The router opened the connection (port forward): our SYN first.
    active: bool,
    /// The C64's SYN has been received.
    peer_syn: bool,
    /// Our SYN has been acknowledged.
    syn_acked: bool,
    /// The host closed its side: FIN after the data.
    host_eof: bool,
    /// The host socket's write side is shut down (after the C64's FIN).
    host_shut: bool,
    iss: u32,
    snd_una: u32,
    snd_nxt: u32,
    snd_max: u32,
    /// The C64's receive window.
    snd_wnd: u32,
    /// The C64's maximum segment size.
    mss: usize,
    /// Data from the host, from sequence number `buf_seq` on.
    send_buf: VecDeque<u8>,
    buf_seq: u32,
    rcv_nxt: u32,
    /// Data from the C64, for the host.
    recv_buf: VecDeque<u8>,
    /// The C64 sent its FIN.
    peer_fin: bool,
    ack_needed: bool,
    /// Window in the last segment sent.
    last_wnd: u16,
    rto: u64,
    /// Retransmission or window probe timer.
    deadline: Option<u64>,
    retries: u32,
    /// Closed on both sides, since (emulated time).
    closed_at: Option<u64>,
    /// Reset: remove at the next poll.
    dead: bool,
    /// Created at (emulated time).
    born: u64,
    /// Bytes carried each way, for the debugger.
    to_host: u64,
    from_host: u64,
}

impl Conn {
    fn new(host: Host, target: SocketAddr, active: bool, iss: u32, born: u64) -> Self {
        Conn {
            host, target, active,
            peer_syn: false, syn_acked: false, host_eof: false, host_shut: false,
            iss, snd_una: iss, snd_nxt: iss, snd_max: iss, snd_wnd: 0, mss: DEFAULT_MSS,
            send_buf: VecDeque::new(), buf_seq: iss.wrapping_add(1),
            rcv_nxt: 0, recv_buf: VecDeque::new(), peer_fin: false,
            ack_needed: false, last_wnd: 0,
            rto: RTO_INITIAL, deadline: None, retries: 0, closed_at: None, dead: false, born,
            to_host: 0, from_host: 0,
        }
    }

    fn window(&self) -> u16 {
        (RECV_LIMIT - self.recv_buf.len()).min(u16::MAX as usize) as u16
    }

    fn buf_end(&self) -> u32 {
        self.buf_seq.wrapping_add(self.send_buf.len() as u32)
    }

    fn fin_acked(&self) -> bool {
        self.host_eof && self.snd_una == self.buf_end().wrapping_add(1)
    }

    fn state(&self) -> &'static str {
        match () {
            _ if matches!(self.host, Host::Connecting(_)) => "connecting",
            _ if !self.syn_acked || !self.peer_syn => "handshake",
            _ if self.closed_at.is_some() => "closed",
            _ if self.peer_fin && self.host_eof => "closing",
            _ if self.peer_fin => "C64 closed",
            _ if self.host_eof => "host closed",
            _ => "established",
        }
    }
}

pub(super) struct Tcp {
    conns: HashMap<Key, Conn>,
    iss_seed: u32,
}

impl Tcp {
    pub fn new() -> Self {
        Tcp { conns: HashMap::new(), iss_seed: 0x5EED_C064 }
    }

    /// Closes every connection (machine reset, savestate loaded).
    pub fn reset(&mut self) {
        for c in self.conns.values() {
            if let Host::Open(s) = &c.host {
                let _ = s.shutdown(Shutdown::Both);
            }
        }
        self.conns.clear();
    }

    fn next_iss(&mut self, now: u64) -> u32 {
        self.iss_seed = self.iss_seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223) ^ now as u32;
        self.iss_seed
    }

    pub fn count(&self) -> usize {
        self.conns.len()
    }

    /// One line per connection, for the debugger.
    pub fn describe(&self) -> Vec<String> {
        let mut v: Vec<_> = self.conns.iter().collect();
        v.sort_by_key(|(k, _)| **k);
        v.into_iter()
            .map(|((ci, cp, ri, rp), c)| {
                let dir = if c.active { "<-" } else { "->" };
                format!("TCP {ci}:{cp} {dir} {ri}:{rp} (host {}): {}, {} bytes out, {} in",
                    c.target, c.state(), c.to_host, c.from_host)
            })
            .collect()
    }

    /// Segment from the C64 (`c64` its address, `remote` the destination).
    /// `target` gives the host address to connect to for a new connection
    /// (`None`: refused).
    pub fn segment(
        &mut self,
        c64: Ipv4Addr,
        remote: Ipv4Addr,
        seg: &wire::Tcp,
        target: impl FnOnce() -> Option<SocketAddr>,
        now: u64,
        out: &mut Vec<Packet>,
    ) {
        let key = (c64, seg.src_port, remote, seg.dst_port);
        if let Some(conn) = self.conns.get_mut(&key) {
            input(conn, seg, now);
            return;
        }
        if seg.flags & RST != 0 {
            return;
        }
        if seg.flags & (SYN | ACK) == SYN {
            match target() {
                Some(addr) if self.conns.len() < MAX_CONNS => {
                    let (tx, rx) = mpsc::channel();
                    std::thread::spawn(move || {
                        let _ = tx.send(TcpStream::connect_timeout(&addr, CONNECT_TIMEOUT));
                    });
                    let iss = self.next_iss(now);
                    let mut c = Conn::new(Host::Connecting(rx), addr, false, iss, now);
                    c.peer_syn = true;
                    c.rcv_nxt = seg.seq.wrapping_add(1);
                    c.snd_wnd = seg.window as u32;
                    c.mss = seg.mss.map_or(DEFAULT_MSS, |m| m as usize).clamp(64, OUR_MSS as usize);
                    self.conns.insert(key, c);
                    return;
                }
                _ => {}
            }
        }
        // No connection: reset, as RFC 793 wants
        let len = seg.payload.len() as u32 + (seg.flags & SYN != 0) as u32 + (seg.flags & FIN != 0) as u32;
        let (seq, ack, flags) = if seg.flags & ACK != 0 {
            (seg.ack, 0, RST)
        } else {
            (0, seg.seq.wrapping_add(len), RST | ACK)
        };
        out.push(packet(remote, c64, &TcpOut {
            src_port: seg.dst_port, dst_port: seg.src_port, seq, ack, flags, window: 0, mss: None, payload: &[],
        }));
    }

    /// Opens a connection to the C64 for a host connection accepted on a
    /// port forward: from `router`:`port` to `c64`:`c64_port`.
    pub fn open(&mut self, stream: TcpStream, c64: Ipv4Addr, c64_port: u16, router: Ipv4Addr, port: u16, now: u64) {
        if self.conns.len() >= MAX_CONNS || stream.set_nonblocking(true).is_err() {
            return;
        }
        let _ = stream.set_nodelay(true);
        let target = stream.peer_addr().unwrap_or_else(|_| SocketAddr::from(([0, 0, 0, 0], 0)));
        let iss = self.next_iss(now);
        self.conns.insert((c64, c64_port, router, port), Conn::new(Host::Open(stream), target, true, iss, now));
    }

    /// A router port for a new forwarded connection, not in use.
    pub fn free_port(&self, c64: Ipv4Addr, c64_port: u16, router: Ipv4Addr, hint: u16) -> u16 {
        let mut p = hint;
        while self.conns.contains_key(&(c64, c64_port, router, p)) {
            p = if p == u16::MAX { 49152 } else { p + 1 };
        }
        p
    }

    /// A connection to the C64 is waiting to send its first SYN.
    pub fn awaiting_neighbour(&self) -> bool {
        self.conns.values().any(|c| c.active && !c.dead && c.snd_max == c.iss)
    }

    /// Host sockets, timers and segments to send. `neighbour`: the C64 has
    /// resolved the router's address (ARP), so the connections the router
    /// opens can send their SYN: a real host asks for the C64's address
    /// before it connects, and stacks such as ip65 learn the router's from
    /// that request; without it they drop the SYN to go and ask.
    pub fn poll(&mut self, now: u64, neighbour: bool, out: &mut Vec<Packet>) {
        for (&(c64, c64_port, remote, port), c) in self.conns.iter_mut() {
            let seg = |c: &mut Conn, flags: u8, seq: u32, payload: &[u8], mss: Option<u16>| {
                let window = c.window();
                c.last_wnd = window;
                let ack = if flags & ACK != 0 { c.rcv_nxt } else { 0 };
                packet(remote, c64, &TcpOut { src_port: port, dst_port: c64_port, seq, ack, flags, window, mss, payload })
            };
            if c.dead {
                continue;
            }
            host_io(c);
            // Waiting for the C64 to answer ARP, for too long: give up
            // without a word to the C64, which never saw the connection
            if c.active && c.snd_max == c.iss && !neighbour && now >= c.born + NEIGHBOUR_TIMEOUT {
                c.dead = true;
                continue;
            }
            if c.dead {
                // Refused, or the host socket failed: reset the C64's side
                let flags = if c.peer_syn { RST | ACK } else { RST };
                let seq = if c.syn_acked || c.active { c.snd_nxt } else { 0 };
                out.push(seg(c, flags, seq, &[], None));
                continue;
            }
            if matches!(c.host, Host::Connecting(_)) {
                continue;
            }
            timers(c, now);
            if c.dead {
                out.push(seg(c, RST | ACK, c.snd_nxt, &[], None));
                continue;
            }
            // SYN (SYN-ACK when the C64 opened the connection)
            if !c.syn_acked {
                if c.snd_nxt == c.iss && (neighbour || !c.active) {
                    let flags = if c.peer_syn { SYN | ACK } else { SYN };
                    out.push(seg(c, flags, c.iss, &[], Some(OUR_MSS)));
                    c.snd_nxt = c.iss.wrapping_add(1);
                    c.snd_max = c.snd_nxt;
                    c.deadline.get_or_insert(now + c.rto);
                }
                continue;
            }
            // Data, within the C64's window
            let end = c.buf_end();
            while lt(c.snd_nxt, end) {
                let wnd_end = c.snd_una.wrapping_add(c.snd_wnd);
                let room = if lt(c.snd_nxt, wnd_end) { wnd_end.wrapping_sub(c.snd_nxt) as usize } else { 0 };
                if room == 0 {
                    break;
                }
                let avail = end.wrapping_sub(c.snd_nxt) as usize;
                let n = avail.min(room).min(c.mss);
                let off = c.snd_nxt.wrapping_sub(c.buf_seq) as usize;
                let data: Vec<u8> = c.send_buf.range(off..off + n).copied().collect();
                let flags = if n == avail { ACK | PSH } else { ACK };
                out.push(seg(c, flags, c.snd_nxt, &data, None));
                c.snd_nxt = c.snd_nxt.wrapping_add(n as u32);
                c.ack_needed = false;
                c.deadline.get_or_insert(now + c.rto);
            }
            // FIN once the data is all sent
            if c.host_eof && c.snd_nxt == end {
                out.push(seg(c, FIN | ACK, end, &[], None));
                c.snd_nxt = end.wrapping_add(1);
                c.ack_needed = false;
                c.deadline.get_or_insert(now + c.rto);
            }
            if lt(c.snd_max, c.snd_nxt) {
                c.snd_max = c.snd_nxt;
            }
            // Zero window with data waiting: probe it
            if c.snd_wnd == 0 && lt(c.snd_nxt, end) {
                c.deadline.get_or_insert(now + c.rto);
            }
            // Window update when our window has opened up again
            let wnd = c.window() as usize;
            if c.peer_syn && wnd >= c.last_wnd as usize + c.mss.min(RECV_LIMIT / 2) {
                c.ack_needed = true;
            }
            if c.ack_needed {
                out.push(seg(c, ACK, c.snd_nxt, &[], None));
                c.ack_needed = false;
            }
            if c.closed_at.is_none() && c.peer_fin && c.fin_acked() && c.recv_buf.is_empty() {
                c.closed_at = Some(now);
            }
        }
        self.conns.retain(|_, c| !c.dead && c.closed_at.is_none_or(|t| now < t + TIME_WAIT));
    }
}

fn packet(src: Ipv4Addr, dst: Ipv4Addr, s: &TcpOut) -> Packet {
    Packet { src, dst, proto: PROTO_TCP, data: wire::tcp(src, dst, s) }
}

/// Segment from the C64 on an existing connection.
fn input(c: &mut Conn, seg: &wire::Tcp, now: u64) {
    if seg.flags & RST != 0 {
        // Removed at the next poll, without an answer
        c.dead = true;
        if let Host::Open(s) = &c.host {
            let _ = s.shutdown(Shutdown::Both);
        }
        return;
    }
    if seg.flags & SYN != 0 {
        if !c.peer_syn {
            // SYN-ACK to our SYN (port forward)
            if seg.flags & ACK != 0 && seg.ack == c.iss.wrapping_add(1) {
                c.peer_syn = true;
                c.rcv_nxt = seg.seq.wrapping_add(1);
                c.mss = seg.mss.map_or(DEFAULT_MSS, |m| m as usize).clamp(64, OUR_MSS as usize);
                acknowledged(c, seg.ack, now);
                c.snd_wnd = seg.window as u32;
                c.ack_needed = true;
            }
        } else if !c.syn_acked {
            // The SYN again: our SYN-ACK was lost, send it again
            c.snd_nxt = c.iss;
        } else {
            c.ack_needed = true;
        }
        return;
    }
    if !c.peer_syn {
        return;
    }
    if seg.flags & ACK != 0 && le(seg.ack, c.snd_max) {
        if lt(c.snd_una, seg.ack) || (!c.syn_acked && seg.ack == c.iss.wrapping_add(1)) {
            acknowledged(c, seg.ack, now);
        }
        if le(c.snd_una, seg.ack) {
            c.snd_wnd = seg.window as u32;
        }
    }
    let len = seg.payload.len() as u32 + (seg.flags & FIN != 0) as u32;
    if len == 0 {
        return;
    }
    let end = seg.seq.wrapping_add(len);
    if le(seg.seq, c.rcv_nxt) && lt(c.rcv_nxt, end) {
        let skip = (c.rcv_nxt.wrapping_sub(seg.seq) as usize).min(seg.payload.len());
        let data = &seg.payload[skip..];
        let take = data.len().min(RECV_LIMIT - c.recv_buf.len());
        c.recv_buf.extend(&data[..take]);
        c.rcv_nxt = c.rcv_nxt.wrapping_add(take as u32);
        if seg.flags & FIN != 0 && take == data.len() && !c.peer_fin {
            c.peer_fin = true;
            c.rcv_nxt = c.rcv_nxt.wrapping_add(1);
        }
    }
    // In order, repeated or out of order: tell the C64 where we are
    c.ack_needed = true;
}

/// The C64 acknowledged everything before `ack`.
fn acknowledged(c: &mut Conn, ack: u32, now: u64) {
    if !c.syn_acked {
        c.syn_acked = true;
        c.snd_una = c.iss.wrapping_add(1);
    }
    let data = (ack.wrapping_sub(c.buf_seq) as usize).min(c.send_buf.len());
    c.send_buf.drain(..data);
    c.buf_seq = c.buf_seq.wrapping_add(data as u32);
    c.snd_una = ack;
    if lt(c.snd_nxt, ack) {
        c.snd_nxt = ack;
    }
    c.retries = 0;
    c.rto = RTO_INITIAL;
    c.deadline = (c.snd_una != c.snd_max).then_some(now + c.rto);
}

/// Retransmission and window probe.
fn timers(c: &mut Conn, now: u64) {
    let Some(deadline) = c.deadline else { return };
    if now < deadline {
        return;
    }
    c.rto = (c.rto * 2).min(RTO_MAX);
    c.deadline = Some(now + c.rto);
    if c.syn_acked && c.snd_wnd == 0 && lt(c.snd_una, c.buf_end()) {
        // The C64's window is closed with data waiting: probe it with one
        // byte, for as long as it takes (the C64 is alive, it answers)
        c.snd_nxt = c.snd_una;
        c.snd_wnd = 1;
    } else if lt(c.snd_una, c.snd_max) {
        // Unacknowledged data: send it again from the first byte
        c.retries += 1;
        if c.retries > MAX_RETRIES {
            c.dead = true;
            return;
        }
        c.snd_nxt = c.snd_una;
    } else {
        c.deadline = None;
    }
}

/// Moves the data between the connection and its host socket.
fn host_io(c: &mut Conn) {
    if let Host::Connecting(rx) = &c.host {
        match rx.try_recv() {
            Ok(Ok(s)) => {
                if s.set_nonblocking(true).is_err() {
                    c.dead = true;
                    return;
                }
                let _ = s.set_nodelay(true);
                c.host = Host::Open(s);
            }
            Ok(Err(_)) | Err(mpsc::TryRecvError::Disconnected) => {
                c.dead = true;
                return;
            }
            Err(mpsc::TryRecvError::Empty) => return,
        }
    }
    let Host::Open(s) = &mut c.host else { return };
    while !c.recv_buf.is_empty() {
        let (a, _) = c.recv_buf.as_slices();
        match s.write(a) {
            Ok(0) => break,
            Ok(n) => {
                c.recv_buf.drain(..n);
                c.to_host += n as u64;
            }
            Err(e) if e.kind() == ErrorKind::Interrupted => {}
            Err(e) if e.kind() == ErrorKind::WouldBlock => break,
            Err(_) => {
                c.dead = true;
                return;
            }
        }
    }
    if c.peer_fin && c.recv_buf.is_empty() && !c.host_shut {
        let _ = s.shutdown(Shutdown::Write);
        c.host_shut = true;
    }
    let mut tmp = [0u8; 4096];
    while !c.host_eof && c.send_buf.len() < SEND_LIMIT {
        let want = tmp.len().min(SEND_LIMIT - c.send_buf.len());
        match s.read(&mut tmp[..want]) {
            Ok(0) => c.host_eof = true,
            Ok(n) => {
                c.send_buf.extend(&tmp[..n]);
                c.from_host += n as u64;
            }
            Err(e) if e.kind() == ErrorKind::Interrupted => {}
            Err(e) if e.kind() == ErrorKind::WouldBlock => break,
            Err(_) => {
                c.dead = true;
                return;
            }
        }
    }
}
