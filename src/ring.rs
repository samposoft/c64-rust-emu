// SPDX-License-Identifier: GPL-2.0-or-later
// Copyright (C) 2026 SampoSoft - Francesco Sampoli

//! Lock-free ring buffer, single producer and single consumer (SPSC), for
//! audio samples between the emulation thread and the real-time callback.
//!
//! The indices `head` (written only by the producer) and `tail` (written only
//! by the consumer) grow without bound and are compared modulo 2^usize; the
//! position in the buffer is `index & mask`. The producer writes the samples
//! and then publishes `head` with Release; the consumer reads `head` with
//! Acquire and so sees the written samples. Symmetrically for `tail`, so the
//! producer never overwrites cells not yet read. Neither side ever waits
//! for the other.

use std::sync::atomic::{AtomicI16, AtomicUsize, Ordering};
use std::sync::Arc;

struct Shared {
    buf: Box<[AtomicI16]>,
    mask: usize,
    head: AtomicUsize,
    tail: AtomicUsize,
}

/// Writing side (emulation thread).
pub struct Producer(Arc<Shared>);

/// Reading side (audio callback).
pub struct Consumer(Arc<Shared>);

/// Creates a queue with capacity `capacity` rounded up to the next power
/// of two.
pub fn channel(capacity: usize) -> (Producer, Consumer) {
    let cap = capacity.max(2).next_power_of_two();
    let shared = Arc::new(Shared {
        buf: (0..cap).map(|_| AtomicI16::new(0)).collect(),
        mask: cap - 1,
        head: AtomicUsize::new(0),
        tail: AtomicUsize::new(0),
    });
    (Producer(shared.clone()), Consumer(shared))
}

impl Producer {
    /// Samples in the queue (as seen by the producer: the consumer may have
    /// read more in the meantime).
    pub fn len(&self) -> usize {
        let head = self.0.head.load(Ordering::Relaxed);
        let tail = self.0.tail.load(Ordering::Acquire);
        head.wrapping_sub(tail)
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub fn capacity(&self) -> usize {
        self.0.buf.len()
    }

    /// Queues as many samples as possible; returns how many fit.
    pub fn push_slice(&mut self, samples: &[i16]) -> usize {
        let s = &*self.0;
        let head = s.head.load(Ordering::Relaxed);
        let tail = s.tail.load(Ordering::Acquire);
        let free = s.buf.len() - head.wrapping_sub(tail);
        let n = samples.len().min(free);
        for (i, &v) in samples[..n].iter().enumerate() {
            s.buf[head.wrapping_add(i) & s.mask].store(v, Ordering::Relaxed);
        }
        s.head.store(head.wrapping_add(n), Ordering::Release);
        n
    }
}

impl Consumer {
    /// Pops the oldest sample, if any.
    pub fn pop(&mut self) -> Option<i16> {
        let s = &*self.0;
        let tail = s.tail.load(Ordering::Relaxed);
        let head = s.head.load(Ordering::Acquire);
        if tail == head {
            return None;
        }
        let v = s.buf[tail & s.mask].load(Ordering::Relaxed);
        s.tail.store(tail.wrapping_add(1), Ordering::Release);
        Some(v)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fifo_and_capacity() {
        let (mut p, mut c) = channel(5);
        assert_eq!(p.capacity(), 8);
        assert_eq!(c.pop(), None);
        assert_eq!(p.push_slice(&[1, 2, 3]), 3);
        assert_eq!(p.len(), 3);
        assert_eq!(c.pop(), Some(1));
        // Full: only the 6 free slots are filled
        assert_eq!(p.push_slice(&[4, 5, 6, 7, 8, 9, 10, 11]), 6);
        assert_eq!(p.push_slice(&[99]), 0);
        let got: Vec<i16> = std::iter::from_fn(|| c.pop()).collect();
        assert_eq!(got, [2, 3, 4, 5, 6, 7, 8, 9]);
        assert!(p.is_empty());
    }

    #[test]
    fn indices_wrap_around() {
        let (mut p, mut c) = channel(4);
        // Start near usize::MAX to cross the overflow
        p.0.head.store(usize::MAX - 1, Ordering::Relaxed);
        p.0.tail.store(usize::MAX - 1, Ordering::Relaxed);
        assert_eq!(p.push_slice(&[1, 2, 3, 4]), 4);
        assert_eq!(p.len(), 4);
        let got: Vec<i16> = std::iter::from_fn(|| c.pop()).collect();
        assert_eq!(got, [1, 2, 3, 4]);
    }

    #[test]
    fn two_threads_keep_order() {
        const N: i32 = 200_000;
        let (mut p, mut c) = channel(64);
        let producer = std::thread::spawn(move || {
            let mut next = 0i32;
            while next < N {
                let chunk: Vec<i16> = (next..(next + 7).min(N)).map(|v| v as i16).collect();
                let n = p.push_slice(&chunk);
                next += n as i32;
                if n == 0 { std::thread::yield_now(); }
            }
        });
        let mut expected = 0i32;
        while expected < N {
            match c.pop() {
                Some(v) => { assert_eq!(v, expected as i16); expected += 1; }
                None => std::thread::yield_now(),
            }
        }
        producer.join().unwrap();
        assert_eq!(c.pop(), None);
    }
}
