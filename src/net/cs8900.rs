// SPDX-License-Identifier: GPL-2.0-or-later
// Copyright (C) 2026 SampoSoft - Francesco Sampoli
// Derived from VICE 3.10 core/cs8900.c, Copyright (C) Spiro Trikaliotis,
// Christian Vogelgsang. Ported to Rust and modified by SampoSoft in 2026;
// see CREDITS.md.

//! Cirrus Logic CS8900A Ethernet controller, as the C64 cartridges (TFE,
//! RR-Net) use it: in I/O mode, eight 16-bit ports read and written a byte
//! at a time, and behind them the 4 KB PacketPage with the configuration,
//! the status registers and the frame buffers.
//!
//! | port | register                                                   |
//! |------|------------------------------------------------------------|
//! | 0, 2 | RxTxData: the received frame (read), the one to send (write) |
//! | 4    | TxCMD, transmit command (PacketPage $0144)                 |
//! | 6    | TxLength (PacketPage $0146)                                |
//! | 8    | ISQ, interrupt status queue (PacketPage $0120)             |
//! | A    | PacketPage pointer (bit 15: auto-increment)                |
//! | C, E | PacketPage data at the pointer                             |
//!
//! Transmitting: TxCMD, then TxLength, then BusST ($0138) until Rdy4TxNOW
//! (bit 8) is set, then the frame into RxTxData, low byte first; the last
//! byte sends it. Receiving: reading RxEvent ($0124) fetches the next frame
//! from the network; with RxOK (bit 8) RxTxData gives RxStatus and
//! RxLength, high byte first, then the frame, low byte first.
//!
//! The network side is two queues: `rx_queue` holds the frames that arrive
//! (`crate::net::Nat` fills it), `tx_queue` collects the ones the C64 sends.
//! As in VICE, a frame is taken from the queue only when the program reads
//! RxEvent, and one that has not been read in full is overwritten by the
//! next (the chip's "implied skip").

use std::collections::VecDeque;

/// Size of the PacketPage.
const PP_SIZE: usize = 0x1000;

// I/O ports (offsets in the 16 bytes of the chip)
const ADDR_RXTXDATA: u8 = 0x00;
const ADDR_RXTXDATA2: u8 = 0x02;
const ADDR_TXCMD: u8 = 0x04;
const ADDR_TXLENGTH: u8 = 0x06;
const ADDR_INTSTQUEUE: u8 = 0x08;
const ADDR_PP_PTR: u8 = 0x0A;
const ADDR_PP_DATA: u8 = 0x0C;
const ADDR_PP_DATA2: u8 = 0x0E;

// PacketPage registers
const PP_PRODUCTID: u16 = 0x0000;
const PP_IOBASE: u16 = 0x0020;
const PP_INTNO: u16 = 0x0022;
const PP_DMA_CHAN: u16 = 0x0024;
const PP_CC_RXCFG: u16 = 0x0102;
const PP_CC_RXCTL: u16 = 0x0104;
const PP_CC_TXCFG: u16 = 0x0106;
const PP_CC_TXCMD: u16 = 0x0108;
const PP_CC_BUFCFG: u16 = 0x010A;
const PP_CC_LINECTL: u16 = 0x0112;
const PP_CC_SELFCTL: u16 = 0x0114;
const PP_CC_BUSCTL: u16 = 0x0116;
const PP_CC_TESTCTL: u16 = 0x0118;
const PP_SE_ISQ: u16 = 0x0120;
const PP_SE_RXEVENT: u16 = 0x0124;
const PP_SE_TXEVENT: u16 = 0x0128;
const PP_SE_BUFEVENT: u16 = 0x012C;
const PP_SE_RXMISS: u16 = 0x0130;
const PP_SE_TXCOL: u16 = 0x0132;
const PP_SE_LINEST: u16 = 0x0134;
const PP_SE_SELFST: u16 = 0x0136;
const PP_SE_BUSST: u16 = 0x0138;
const PP_SE_TDR: u16 = 0x013C;
const PP_TXCMD: u16 = 0x0144;
const PP_TXLENGTH: u16 = 0x0146;
const PP_LOG_ADDR_FILTER: u16 = 0x0150;
const PP_MAC_ADDR: u16 = 0x0158;
const PP_RXSTATUS: u16 = 0x0400;
const PP_RXLENGTH: u16 = 0x0402;
const PP_RX_FRAMELOC: u16 = 0x0404;
const PP_TX_FRAMELOC: u16 = 0x0A00;

/// PacketPage pointer: auto-increment flag, flag bits, address.
const PP_PTR_AUTO_INCR: u16 = 0x8000;
const PP_PTR_FLAG_MASK: u16 = 0xF000;
const PP_PTR_ADDR_MASK: u16 = 0x0FFF;

const MAX_TXLENGTH: u16 = 1518;
const MAX_RXLENGTH: usize = 1518;
const MIN_RXLENGTH: usize = 64;

/// Frames waiting in `rx_queue` beyond which new ones are dropped, as a
/// switch drops them when the port is not read.
pub const RX_QUEUE_MAX: usize = 64;

#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
enum TxState {
    #[default]
    Idle,
    GotCmd,
    GotLen,
    ReadBusSt,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
enum RxState {
    #[default]
    Idle,
    GotFrame,
}

/// Frame counters, for the debugger.
#[derive(Clone, Copy, Default, Debug)]
pub struct Stats {
    pub sent: u64,
    pub received: u64,
    /// Frames the address filter rejected.
    pub filtered: u64,
    /// Frames lost because `rx_queue` was full.
    pub dropped: u64,
}

pub struct Cs8900 {
    /// Last word of each I/O port, low and high byte: a byte write takes
    /// the other half from here.
    io: [u8; 16],
    pp: Vec<u8>,
    pp_ptr: u16,
    /// Individual address (MAC). The spec leaves it undefined after a
    /// reset; the real chip keeps it, and so does VICE.
    ia_mac: [u8; 6],
    /// Logical address filter (hash of the destination address).
    hash_mask: [u32; 2],
    /// Copy of RxCTL: which frames the receiver accepts.
    recv_control: u16,
    tx_buffer: u16,
    rx_buffer: u16,
    tx_count: u16,
    rx_count: u16,
    tx_length: u16,
    rx_length: u16,
    tx_state: TxState,
    rx_state: RxState,
    tx_enabled: bool,
    rx_enabled: bool,
    /// Which halves of RxEvent were read since the last reception (bit 0
    /// high, bit 1 low): a new frame is fetched only once per word, however
    /// the program orders the two reads.
    rxevent_read_mask: u8,
    /// Frames from the network, oldest first.
    pub rx_queue: VecDeque<Vec<u8>>,
    /// Frames sent by the C64, for the network.
    pub tx_queue: Vec<Vec<u8>>,
    pub stats: Stats,
}

impl Default for Cs8900 {
    fn default() -> Self {
        Self::new()
    }
}

impl Cs8900 {
    pub fn new() -> Self {
        let mut c = Self {
            io: [0; 16],
            pp: vec![0; PP_SIZE],
            pp_ptr: 0,
            ia_mac: [0; 6],
            hash_mask: [0; 2],
            recv_control: 0,
            tx_buffer: PP_TX_FRAMELOC,
            rx_buffer: PP_RXSTATUS,
            tx_count: 0,
            rx_count: 0,
            tx_length: 0,
            rx_length: 0,
            tx_state: TxState::Idle,
            rx_state: RxState::Idle,
            tx_enabled: false,
            rx_enabled: false,
            rxevent_read_mask: 3,
            rx_queue: VecDeque::new(),
            tx_queue: Vec::new(),
            stats: Stats::default(),
        };
        c.reset();
        c
    }

    /// Chip reset (power-on, machine reset, or bit 6 of SelfCTL): registers
    /// at their reset values, the MAC address kept.
    pub fn reset(&mut self) {
        self.io = [0; 16];
        self.pp.fill(0);
        self.set_pp32(PP_PRODUCTID, 0x0900_630E); // Rev. D, bytes 0E 63 00 09
        self.set_pp16(PP_IOBASE, 0x0300);
        self.set_pp16(PP_INTNO, 0x0004);
        self.set_pp16(PP_DMA_CHAN, 0x0003);
        // Control and status registers hold their number in bits 0-5
        self.set_pp16(PP_CC_RXCFG, 0x0003);
        self.set_pp16(PP_CC_RXCTL, 0x0005);
        self.set_pp16(PP_CC_TXCFG, 0x0007);
        self.set_pp16(PP_CC_TXCMD, 0x0009);
        self.set_pp16(PP_CC_BUFCFG, 0x000B);
        self.set_pp16(PP_CC_LINECTL, 0x0013);
        self.set_pp16(PP_CC_SELFCTL, 0x0015);
        self.set_pp16(PP_CC_BUSCTL, 0x0017);
        self.set_pp16(PP_CC_TESTCTL, 0x0019);
        self.set_pp16(PP_SE_ISQ, 0x0000);
        self.set_pp16(PP_SE_RXEVENT, 0x0004);
        self.set_pp16(PP_SE_TXEVENT, 0x0008);
        self.set_pp16(PP_SE_BUFEVENT, 0x000C);
        self.set_pp16(PP_SE_RXMISS, 0x0010);
        self.set_pp16(PP_SE_TXCOL, 0x0012);
        // The reset value is $0014; as VICE, Link OK, 10BASE-T and
        // Polarity OK are set too, since programs check them
        self.set_pp16(PP_SE_LINEST, 0x1294);
        // INITD: the chip is ready
        self.set_pp16(PP_SE_SELFST, 0x0896);
        self.set_pp16(PP_SE_BUSST, 0x0018);
        self.set_pp16(PP_SE_TDR, 0x001C);
        self.set_pp16(PP_TXCMD, 0x0009);
        self.recv_control = self.pp16(PP_CC_RXCTL);
        for i in 0..6 {
            self.pp[PP_MAC_ADDR as usize + i] = self.ia_mac[i];
        }
        self.set_transmitter(false);
        self.set_receiver(false);
    }

    /// MAC address set by the program.
    pub fn mac(&self) -> [u8; 6] {
        self.ia_mac
    }

    /// Transmitter and receiver enabled (LineCTL SerTxON, SerRxON).
    pub fn enabled(&self) -> (bool, bool) {
        (self.tx_enabled, self.rx_enabled)
    }

    /// RxCTL: the frames the receiver accepts.
    pub fn rx_control(&self) -> u16 {
        self.recv_control
    }

    // ── PacketPage access ─────────────────────────────────────────────────────

    fn pp16(&self, a: u16) -> u16 {
        let a = a as usize;
        u16::from_le_bytes([self.pp[a], self.pp[a + 1]])
    }

    fn set_pp16(&mut self, a: u16, v: u16) {
        let a = a as usize;
        self.pp[a..a + 2].copy_from_slice(&v.to_le_bytes());
    }

    fn set_pp32(&mut self, a: u16, v: u32) {
        let a = a as usize;
        self.pp[a..a + 4].copy_from_slice(&v.to_le_bytes());
    }

    fn set_tx_status(&mut self, ready: bool, error: bool) {
        let mut s = self.pp16(PP_SE_BUSST) & !0x180;
        if ready {
            s |= 0x100; // Rdy4TxNOW
        }
        if error {
            s |= 0x080; // TxBidErr
        }
        self.set_pp16(PP_SE_BUSST, s);
    }

    fn set_receiver(&mut self, enabled: bool) {
        self.rx_enabled = enabled;
        self.rx_state = RxState::Idle;
        self.rxevent_read_mask = 3;
    }

    fn set_transmitter(&mut self, enabled: bool) {
        self.tx_enabled = enabled;
        self.tx_state = TxState::Idle;
        self.set_tx_status(false, false);
    }

    // ── Reception ─────────────────────────────────────────────────────────────

    /// Whether RxCTL lets the frame in, with the flags of the RxEvent word:
    /// (accept, individual address, broadcast, hashed, multicast, hash
    /// index).
    fn accepts(&self, frame: &[u8]) -> (bool, bool, bool, bool, bool, u16) {
        let rxctl = self.recv_control;
        let promiscuous = rxctl & 0x0080 != 0;
        let da = &frame[..6];
        let correct_mac = da == self.ia_mac;
        if correct_mac && (rxctl & 0x0400 != 0 || promiscuous) {
            return (true, true, false, false, false, 0);
        }
        if da == [0xFF; 6] {
            return (rxctl & 0x0800 != 0 || promiscuous, correct_mac, true, false, false, 0);
        }
        let hash = (!crc32(da) >> 26) & 0x3F;
        if self.hash_mask[(hash >= 32) as usize] & (1 << (hash & 0x1F)) != 0 {
            if da[0] & 0x80 != 0 {
                // A multicast address that passes the filter: the hashed
                // bit stays clear
                return (rxctl & 0x0200 != 0 || promiscuous, correct_mac, false, false, true, hash as u16);
            }
            return (rxctl & 0x0040 != 0 || promiscuous, correct_mac, false, true, false, hash as u16);
        }
        (promiscuous, correct_mac, false, false, false, 0)
    }

    /// Takes the next accepted frame from the network into the receive
    /// buffer. Returns the RxEvent word.
    fn receive(&mut self) -> u16 {
        let mut ret = 0x0004;
        while let Some(frame) = self.rx_queue.pop_front() {
            if frame.len() < 6 {
                continue;
            }
            let (accept, correct_mac, broadcast, hashed, multicast, hash_index) = self.accepts(&frame);
            if !accept {
                self.stats.filtered += 1;
                continue;
            }
            // Every frame arrives with a good CRC
            ret |= 0x0100;
            if multicast {
                ret |= 0x0200;
            } else if hashed {
                ret |= 0x0040;
            }
            // The length is always even, as in VICE's network layer
            let mut len = frame.len() + (frame.len() & 1);
            if hashed {
                ret |= hash_index << 9;
            } else {
                if correct_mac { ret |= 0x0400; }
                if broadcast { ret |= 0x0800; }
                if len < MIN_RXLENGTH { ret |= 0x2000; }
                if len > MAX_RXLENGTH { ret |= 0x4000; }
            }
            len = len.min(MAX_RXLENGTH);
            self.set_pp16(PP_RXLENGTH, len as u16);
            let at = PP_RX_FRAMELOC as usize;
            let n = frame.len().min(len);
            self.pp[at..at + n].copy_from_slice(&frame[..n]);
            if n < len {
                self.pp[at + n] = 0;
            }
            self.rx_buffer = PP_RXSTATUS;
            self.rx_length = len as u16;
            self.rx_count = 0;
            self.rx_state = RxState::GotFrame;
            self.stats.received += 1;
            break;
        }
        ret
    }

    /// Byte of RxTxData: RxStatus and RxLength high byte first, then the
    /// frame low byte first. `advance` false leaves the state as it is
    /// (debugger).
    fn read_rx_buffer(&mut self, odd: bool, advance: bool) -> u8 {
        if self.rx_state != RxState::GotFrame {
            // Always zero on the real chip
            return 0;
        }
        let (mut buffer, mut count, mut state) = (self.rx_buffer, self.rx_count, self.rx_state);
        let value;
        if count < 4 {
            value = self.pp[(buffer + odd as u16) as usize];
            count += 1;
            // Advance after the even (low) byte of RxStatus and RxLength
            if !odd {
                buffer += 2;
            }
        } else {
            // In the frame, advance before each even byte but the first
            if count >= 6 && !odd {
                buffer += 2;
            }
            value = self.pp[(buffer + odd as u16) as usize & (PP_SIZE - 1)];
            count += 1;
        }
        if count >= self.rx_length + 4 {
            state = RxState::Idle;
        }
        if advance {
            (self.rx_buffer, self.rx_count, self.rx_state) = (buffer, count, state);
        }
        value
    }

    // ── Transmission ──────────────────────────────────────────────────────────

    fn write_tx_buffer(&mut self, value: u8, odd: bool) {
        if self.tx_state != TxState::ReadBusSt {
            // No transmit condition (also after a transmission shorter than
            // 4 bytes was started)
            self.set_tx_status(false, false);
            return;
        }
        // Always low, high, low, high...
        let mut addr = self.tx_buffer;
        if odd {
            addr += 1;
            self.tx_buffer += 2;
        }
        self.tx_count += 1;
        self.pp[addr as usize & (PP_SIZE - 1)] = value;
        if self.tx_count == self.tx_length {
            if self.tx_enabled {
                let at = PP_TX_FRAMELOC as usize;
                let mut frame = self.pp[at..at + self.tx_length as usize].to_vec();
                // TxPadDis clear: the chip pads short frames to 60 bytes
                if self.pp16(PP_CC_TXCMD) & 0x2000 == 0 && frame.len() < 60 {
                    frame.resize(60, 0);
                }
                self.tx_queue.push(frame);
                self.stats.sent += 1;
            }
            self.tx_state = TxState::Idle;
            self.set_tx_status(false, false);
        }
    }

    // ── Register side effects ─────────────────────────────────────────────────

    /// After a write to the PacketPage register `pp` (the byte at `odd`).
    fn after_write(&mut self, pp: u16, odd: bool) {
        let content = self.pp16(pp);
        match pp {
            PP_CC_RXCFG => {
                // Skip_1: drop the frame being transmitted (acts once)
                if content & 0x40 != 0 {
                    self.set_transmitter(self.tx_enabled);
                    self.set_pp16(pp, content & !0x40);
                }
            }
            PP_CC_RXCTL => self.recv_control = content,
            PP_CC_LINECTL => {
                let tx = content & 0x0080 != 0;
                let rx = content & 0x0040 != 0;
                if tx != self.tx_enabled || rx != self.rx_enabled {
                    self.set_transmitter(tx);
                    self.set_receiver(rx);
                }
            }
            PP_CC_SELFCTL => {
                if content & 0x40 != 0 {
                    self.reset();
                }
            }
            PP_TXCMD => {
                if odd {
                    // The transmit status register gets the last command
                    self.set_pp16(PP_CC_TXCMD, self.pp16(PP_TXCMD));
                    self.tx_state = TxState::GotCmd;
                    self.set_tx_status(false, false);
                }
            }
            PP_TXLENGTH => {
                if odd && self.tx_state == TxState::GotCmd {
                    let len = self.pp16(PP_TXLENGTH);
                    let cmd = self.pp16(PP_CC_TXCMD);
                    if len < 4 {
                        // Too short: space available, nothing committed
                        self.tx_state = TxState::Idle;
                        self.set_tx_status(true, false);
                    } else if len > MAX_TXLENGTH || (len > MAX_TXLENGTH - 4 && cmd & 0x1000 == 0) {
                        self.tx_state = TxState::Idle;
                        self.set_tx_status(false, true);
                    } else {
                        self.tx_buffer = PP_TX_FRAMELOC;
                        self.tx_count = 0;
                        self.tx_length = len;
                        self.tx_state = TxState::GotLen;
                        self.set_tx_status(true, false);
                    }
                }
            }
            a if (PP_LOG_ADDR_FILTER..PP_LOG_ADDR_FILTER + 8).contains(&a) => {
                let pos = 8 * (a - PP_LOG_ADDR_FILTER + odd as u16) as u32;
                let byte = self.pp[(a + odd as u16) as usize] as u32;
                let word = &mut self.hash_mask[(pos >= 32) as usize];
                let shift = pos & 31;
                *word = *word & !(0xFF << shift) | byte << shift;
            }
            a if (PP_MAC_ADDR..PP_MAC_ADDR + 6).contains(&a) => {
                let i = (a - PP_MAC_ADDR + odd as u16) as usize;
                self.ia_mac[i] = self.pp[a as usize + odd as usize];
            }
            _ => {}
        }
    }

    /// Before a read of the PacketPage register `pp`.
    fn before_read(&mut self, pp: u16, odd: bool) {
        match pp {
            PP_SE_RXEVENT => {
                // A new frame is fetched once per word read, in either byte
                // order (LH, HL, or only one of the halves every time)
                let access = if odd { 1 } else { 2 };
                if access & self.rxevent_read_mask != 0 {
                    if self.rx_enabled {
                        let ev = self.receive();
                        self.set_pp16(PP_RXSTATUS, ev);
                        self.set_pp16(PP_SE_RXEVENT, ev);
                    }
                    self.rxevent_read_mask = access;
                } else {
                    self.rxevent_read_mask |= access;
                }
            }
            PP_SE_BUSST if odd && self.tx_state == TxState::GotLen && self.pp16(PP_SE_BUSST) & 0x100 != 0 => {
                self.tx_state = TxState::ReadBusSt;
            }
            _ => {}
        }
    }

    /// Value of the PacketPage register `pp` as the chip returns it:
    /// reserved areas read $0300, the frame buffers 0.
    fn read_register(&self, pp: u16) -> u16 {
        let value = self.pp16(pp);
        match pp {
            0x0004..=0x001F => 0x0300,
            0x0100..=0x011F => {
                let reg = ((pp - 0x100) & !1) + 1;
                if reg == 0x01 || reg == 0x11 || reg > 0x19 { 0x0300 } else { value }
            }
            0x0120..=0x013F => {
                let reg = (pp - 0x120) & !1;
                if matches!(reg, 0x02 | 0x06 | 0x0A | 0x0E | 0x1A | 0x1E) { 0x0300 } else { value }
            }
            0x0140..=0x014F => if pp == PP_TXCMD || pp == PP_TXLENGTH { value } else { 0x0300 },
            0x0150..=0x015F => if pp >= 0x015E { 0x0300 } else { value },
            0x0160..=0x03FF => 0x0300,
            0x0400..=0x0FFF => 0,
            _ => value,
        }
    }

    fn write_register(&mut self, pp: u16, mut value: u16) {
        match pp {
            0x0000..=0x00FF => {
                if pp < 0x20 || (0x26..0x2C).contains(&pp) || pp == 0x38 || pp >= 0x44 {
                    return;
                }
            }
            0x0100..=0x011F => {
                let reg = ((pp - 0x100) & !1) + 1;
                // Bits 0-5 always hold the register number
                value = value & !0x3F | reg;
                if reg == 0x01 || reg == 0x11 || reg > 0x19 {
                    return;
                }
            }
            0x0120..=0x013F => return,
            0x0140..=0x014F => match pp {
                PP_TXCMD => value = (value & !0x3F | 0x09) & 0x33FF,
                PP_TXLENGTH => value &= 0x0FFF,
                0x0145 | 0x0147 => {}
                _ => return,
            },
            0x0150..=0x015F => {
                if pp >= 0x015E {
                    return;
                }
            }
            _ => return,
        }
        self.set_pp16(pp, value);
    }

    fn auto_increment(&mut self) {
        if self.pp_ptr & PP_PTR_AUTO_INCR != 0 {
            let ptr = (self.pp_ptr & PP_PTR_ADDR_MASK) + 1;
            self.pp_ptr = ptr & PP_PTR_ADDR_MASK | self.pp_ptr & PP_PTR_FLAG_MASK;
        }
    }

    /// PacketPage address behind the I/O port `base` (even).
    fn port_pp(&self, base: u8) -> u16 {
        match base {
            ADDR_PP_DATA | ADDR_PP_DATA2 => self.pp_ptr & PP_PTR_ADDR_MASK & !1,
            ADDR_INTSTQUEUE => PP_SE_ISQ,
            ADDR_TXCMD => PP_TXCMD,
            ADDR_TXLENGTH => PP_TXLENGTH,
            _ => PP_PRODUCTID,
        }
    }

    // ── Bus interface ─────────────────────────────────────────────────────────

    /// CPU read of port byte `reg` (0-15).
    pub fn read(&mut self, reg: u8) -> u8 {
        let base = reg & !1;
        let odd = reg & 1 != 0;
        if base == ADDR_RXTXDATA || base == ADDR_RXTXDATA2 {
            return self.read_rx_buffer(odd, true);
        }
        let word = if base == ADDR_PP_PTR {
            self.pp_ptr
        } else {
            let pp = self.port_pp(base);
            if base == ADDR_PP_DATA || base == ADDR_PP_DATA2 {
                self.auto_increment();
            }
            self.before_read(pp, odd);
            self.read_register(pp)
        };
        self.io[base as usize..base as usize + 2].copy_from_slice(&word.to_le_bytes());
        word.to_le_bytes()[odd as usize]
    }

    /// Read without side effects, for the debugger.
    pub fn peek(&self, reg: u8) -> u8 {
        let base = reg & !1;
        let odd = reg & 1 != 0;
        if base == ADDR_RXTXDATA || base == ADDR_RXTXDATA2 {
            // Runs the reading step on a copy of the few fields it touches
            let mut state = (self.rx_buffer, self.rx_count, self.rx_state);
            let c = Cs8900Peek { chip: self, state: &mut state };
            return c.byte(odd);
        }
        let word = if base == ADDR_PP_PTR { self.pp_ptr } else { self.read_register(self.port_pp(base)) };
        word.to_le_bytes()[odd as usize]
    }

    /// CPU write of port byte `reg` (0-15).
    pub fn write(&mut self, reg: u8, val: u8) {
        let base = reg & !1;
        let odd = reg & 1 != 0;
        if base == ADDR_RXTXDATA || base == ADDR_RXTXDATA2 {
            self.write_tx_buffer(val, odd);
            return;
        }
        let b = base as usize;
        let word = if odd {
            u16::from_le_bytes([self.io[b], val])
        } else {
            u16::from_le_bytes([val, self.io[b + 1]])
        };
        let word = if base == ADDR_PP_PTR {
            // Bits 12-13 always read as 1
            self.pp_ptr = word | 0x3000;
            self.pp_ptr
        } else {
            let pp = self.port_pp(base);
            if base == ADDR_PP_DATA || base == ADDR_PP_DATA2 {
                self.auto_increment();
            }
            self.write_register(pp, word);
            self.after_write(pp, odd);
            self.pp16(pp)
        };
        self.io[b..b + 2].copy_from_slice(&word.to_le_bytes());
    }
}

/// Side-effect-free step of the RxTxData read, on a copy of the receive
/// position.
struct Cs8900Peek<'a> {
    chip: &'a Cs8900,
    state: &'a mut (u16, u16, RxState),
}

impl Cs8900Peek<'_> {
    fn byte(self, odd: bool) -> u8 {
        let (buffer, count, state) = *self.state;
        if state != RxState::GotFrame {
            return 0;
        }
        let at = if count < 4 {
            buffer + odd as u16
        } else if count >= 6 && !odd {
            buffer + 2
        } else {
            buffer + odd as u16
        };
        self.chip.pp[at as usize & (PP_SIZE - 1)]
    }
}

/// CRC-32 (Ethernet, reflected polynomial $EDB88320), for the hash filter.
pub fn crc32(data: &[u8]) -> u32 {
    let mut crc = 0xFFFF_FFFFu32;
    for &b in data {
        crc ^= b as u32;
        for _ in 0..8 {
            crc = if crc & 1 != 0 { 0xEDB8_8320 ^ (crc >> 1) } else { crc >> 1 };
        }
    }
    !crc
}

crate::snapshot::impl_state_enum!(TxState { Idle, GotCmd, GotLen, ReadBusSt });
crate::snapshot::impl_state_enum!(RxState { Idle, GotFrame });

// The queues are the network cable, not the chip: frames in flight are
// lost across a savestate, as on a real reset of the connection.
crate::snapshot::impl_state!(Cs8900 {
    io, pp, pp_ptr, ia_mac, hash_mask, recv_control, tx_buffer, rx_buffer, tx_count, rx_count,
    tx_length, rx_length, tx_state, rx_state, tx_enabled, rx_enabled, rxevent_read_mask,
} skip { rx_queue, tx_queue, stats });

#[cfg(test)]
mod tests {
    use super::*;

    fn pp_write(c: &mut Cs8900, addr: u16, v: u16) {
        c.write(ADDR_PP_PTR, addr as u8);
        c.write(ADDR_PP_PTR + 1, (addr >> 8) as u8);
        c.write(ADDR_PP_DATA, v as u8);
        c.write(ADDR_PP_DATA + 1, (v >> 8) as u8);
    }

    fn pp_read(c: &mut Cs8900, addr: u16) -> u16 {
        c.write(ADDR_PP_PTR, addr as u8);
        c.write(ADDR_PP_PTR + 1, (addr >> 8) as u8);
        let lo = c.read(ADDR_PP_DATA);
        let hi = c.read(ADDR_PP_DATA + 1);
        u16::from_le_bytes([lo, hi])
    }

    fn chip() -> Cs8900 {
        let mut c = Cs8900::new();
        for (i, w) in [0x0201u16, 0x0403, 0x0605].into_iter().enumerate() {
            pp_write(&mut c, PP_MAC_ADDR + 2 * i as u16, w);
        }
        pp_write(&mut c, PP_CC_RXCTL, 0x0D05);
        pp_write(&mut c, PP_CC_LINECTL, 0x00D3);
        c
    }

    #[test]
    fn product_id_and_reserved_areas() {
        let mut c = Cs8900::new();
        assert_eq!(pp_read(&mut c, PP_PRODUCTID), 0x630E);
        assert_eq!(pp_read(&mut c, 0x0010), 0x0300);
        assert_eq!(pp_read(&mut c, PP_SE_SELFST) & 0x80, 0x80);
        // Control registers keep their number in bits 0-5
        pp_write(&mut c, PP_CC_RXCTL, 0x0D00);
        assert_eq!(pp_read(&mut c, PP_CC_RXCTL), 0x0D05);
    }

    #[test]
    fn transmit_a_frame() {
        let mut c = chip();
        assert_eq!(c.mac(), [1, 2, 3, 4, 5, 6]);
        c.write(ADDR_TXCMD, 0xC9);
        c.write(ADDR_TXCMD + 1, 0x00);
        c.write(ADDR_TXLENGTH, 61);
        c.write(ADDR_TXLENGTH + 1, 0);
        assert_eq!(pp_read(&mut c, PP_SE_BUSST) & 0x100, 0x100, "Rdy4TxNOW");
        for i in 0..61u8 {
            c.write(ADDR_RXTXDATA + (i & 1), i);
        }
        assert_eq!(c.tx_queue.len(), 1);
        assert_eq!(c.tx_queue[0], (0..61).collect::<Vec<u8>>());
        assert_eq!(pp_read(&mut c, PP_SE_BUSST) & 0x100, 0, "Rdy4TxNOW cleared after sending");
    }

    #[test]
    fn short_frames_are_padded_unless_disabled() {
        let mut c = chip();
        for (cmd, want) in [(0x00C9u16, 60), (0x20C9, 42)] {
            c.write(ADDR_TXCMD, cmd as u8);
            c.write(ADDR_TXCMD + 1, (cmd >> 8) as u8);
            c.write(ADDR_TXLENGTH, 42);
            c.write(ADDR_TXLENGTH + 1, 0);
            pp_read(&mut c, PP_SE_BUSST);
            for i in 0..42u8 {
                c.write(ADDR_RXTXDATA + (i & 1), 0xAA);
            }
            assert_eq!(c.tx_queue.pop().unwrap().len(), want);
        }
    }

    #[test]
    fn receive_status_and_length_high_byte_first() {
        let mut c = chip();
        let mut frame = vec![1, 2, 3, 4, 5, 6, 9, 9, 9, 9, 9, 9, 0x08, 0x00];
        frame.extend(0..51u8);
        c.rx_queue.push_back(frame.clone());
        let ev = pp_read(&mut c, PP_SE_RXEVENT);
        assert_eq!(ev & 0x0100, 0x0100, "RxOK");
        assert_eq!(ev & 0x0400, 0x0400, "individual address");
        let hi = c.read(ADDR_RXTXDATA + 1);
        let lo = c.read(ADDR_RXTXDATA);
        assert_eq!(u16::from_le_bytes([lo, hi]), ev, "RxStatus");
        let hi = c.read(ADDR_RXTXDATA + 1);
        assert_eq!(c.peek(ADDR_RXTXDATA), 66, "peek does not advance");
        let lo = c.read(ADDR_RXTXDATA);
        assert_eq!(u16::from_le_bytes([lo, hi]), 66, "odd length rounded up");
        let mut got = Vec::new();
        for _ in 0..33 {
            got.push(c.read(ADDR_RXTXDATA));
            got.push(c.read(ADDR_RXTXDATA + 1));
        }
        frame.push(0);
        assert_eq!(got, frame);
        assert_eq!(c.read(ADDR_RXTXDATA), 0, "frame fully read");
    }

    #[test]
    fn address_filter() {
        let mut c = chip();
        let other = vec![7; 60];
        let mut bcast = vec![0xFF; 6];
        bcast.resize(60, 0);
        c.rx_queue.push_back(other);
        c.rx_queue.push_back(bcast);
        let ev = pp_read(&mut c, PP_SE_RXEVENT);
        assert_eq!(ev & 0x0900, 0x0900, "the broadcast, the other frame filtered");
        assert_eq!(c.stats.filtered, 1);
        assert_eq!(pp_read(&mut c, PP_SE_RXEVENT), 0x0004, "nothing more");
    }

    #[test]
    fn crc32_check_value() {
        assert_eq!(crc32(b"123456789"), 0xCBF4_3926);
    }
}
