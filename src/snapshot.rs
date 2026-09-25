// SPDX-License-Identifier: GPL-2.0-or-later
// Copyright (C) 2026 SampoSoft - Francesco Sampoli

//! Snapshot of the complete machine state (savestate).
//!
//! Every component implements `State`: `save` writes the fields into a buffer,
//! `load` reads them back in the same order onto an already built value. The
//! format is little-endian binary, without field names: the version in the
//! file (see `C64::save_state`) must be bumped on every layout change.
//!
//! Structs use `impl_state!`, which destructures the type listing all its
//! fields, saved or excluded with `skip`: adding a field without deciding
//! whether to save it does not compile.

use std::collections::VecDeque;

pub type Result<T> = std::result::Result<T, String>;

pub trait State {
    fn save(&self, w: &mut Writer);
    fn load(&mut self, r: &mut Reader) -> Result<()>;
}

#[derive(Default)]
pub struct Writer {
    pub buf: Vec<u8>,
}

impl Writer {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn put(&mut self, bytes: &[u8]) {
        self.buf.extend_from_slice(bytes);
    }
}

pub struct Reader<'a> {
    data: &'a [u8],
    pos: usize,
}

impl<'a> Reader<'a> {
    pub fn new(data: &'a [u8]) -> Self {
        Self { data, pos: 0 }
    }

    pub fn take(&mut self, n: usize) -> Result<&'a [u8]> {
        let end = self.pos.checked_add(n).filter(|&e| e <= self.data.len())
            .ok_or_else(|| format!("truncated state (offset {})", self.pos))?;
        let s = &self.data[self.pos..end];
        self.pos = end;
        Ok(s)
    }

    pub fn is_empty(&self) -> bool {
        self.pos == self.data.len()
    }
}

// ── Basic types ──────────────────────────────────────────────────────────────

macro_rules! state_le {
    ($($t:ty),*) => {$(
        impl State for $t {
            fn save(&self, w: &mut Writer) {
                w.put(&self.to_le_bytes());
            }
            fn load(&mut self, r: &mut Reader) -> Result<()> {
                let b = r.take(std::mem::size_of::<$t>())?;
                *self = <$t>::from_le_bytes(b.try_into().unwrap());
                Ok(())
            }
        }
    )*};
}
state_le!(u8, u16, u32, u64, i8, i16, i32, i64, f32, f64);

impl State for bool {
    fn save(&self, w: &mut Writer) {
        w.put(&[*self as u8]);
    }
    fn load(&mut self, r: &mut Reader) -> Result<()> {
        *self = match r.take(1)?[0] {
            0 => false,
            1 => true,
            v => return Err(format!("invalid bool: {v}")),
        };
        Ok(())
    }
}

impl State for usize {
    fn save(&self, w: &mut Writer) {
        (*self as u64).save(w);
    }
    fn load(&mut self, r: &mut Reader) -> Result<()> {
        let mut v = 0u64;
        v.load(r)?;
        *self = usize::try_from(v).map_err(|_| format!("usize out of range: {v}"))?;
        Ok(())
    }
}

impl State for char {
    fn save(&self, w: &mut Writer) {
        (*self as u32).save(w);
    }
    fn load(&mut self, r: &mut Reader) -> Result<()> {
        let mut v = 0u32;
        v.load(r)?;
        *self = char::from_u32(v).ok_or_else(|| format!("invalid char: {v}"))?;
        Ok(())
    }
}

impl State for String {
    fn save(&self, w: &mut Writer) {
        save_len(self.len(), w);
        w.put(self.as_bytes());
    }
    fn load(&mut self, r: &mut Reader) -> Result<()> {
        let n = load_len(r)?;
        *self = String::from_utf8(r.take(n)?.to_vec()).map_err(|_| "invalid string".to_string())?;
        Ok(())
    }
}

// ── Compound types ───────────────────────────────────────────────────────────

impl<T: State, const N: usize> State for [T; N] {
    fn save(&self, w: &mut Writer) {
        for x in self {
            x.save(w);
        }
    }
    fn load(&mut self, r: &mut Reader) -> Result<()> {
        for x in self {
            x.load(r)?;
        }
        Ok(())
    }
}

impl<A: State, B: State> State for (A, B) {
    fn save(&self, w: &mut Writer) {
        self.0.save(w);
        self.1.save(w);
    }
    fn load(&mut self, r: &mut Reader) -> Result<()> {
        self.0.load(r)?;
        self.1.load(r)
    }
}

impl<T: State + ?Sized> State for Box<T> {
    fn save(&self, w: &mut Writer) {
        (**self).save(w);
    }
    fn load(&mut self, r: &mut Reader) -> Result<()> {
        (**self).load(r)
    }
}

fn save_len(len: usize, w: &mut Writer) {
    (len as u32).save(w);
}

fn load_len(r: &mut Reader) -> Result<usize> {
    let mut n = 0u32;
    n.load(r)?;
    Ok(n as usize)
}

impl<T: State + Default> State for Vec<T> {
    fn save(&self, w: &mut Writer) {
        save_len(self.len(), w);
        for x in self {
            x.save(w);
        }
    }
    fn load(&mut self, r: &mut Reader) -> Result<()> {
        let n = load_len(r)?;
        self.clear();
        for _ in 0..n {
            let mut x = T::default();
            x.load(r)?;
            self.push(x);
        }
        Ok(())
    }
}

impl<T: State + Default> State for VecDeque<T> {
    fn save(&self, w: &mut Writer) {
        save_len(self.len(), w);
        for x in self {
            x.save(w);
        }
    }
    fn load(&mut self, r: &mut Reader) -> Result<()> {
        let n = load_len(r)?;
        self.clear();
        for _ in 0..n {
            let mut x = T::default();
            x.load(r)?;
            self.push_back(x);
        }
        Ok(())
    }
}

impl<T: State + Default> State for Option<T> {
    fn save(&self, w: &mut Writer) {
        match self {
            None => false.save(w),
            Some(x) => {
                true.save(w);
                x.save(w);
            }
        }
    }
    fn load(&mut self, r: &mut Reader) -> Result<()> {
        let mut some = false;
        some.load(r)?;
        *self = if some {
            let mut x = T::default();
            x.load(r)?;
            Some(x)
        } else {
            None
        };
        Ok(())
    }
}

// ── Macros for structs and enums ─────────────────────────────────────────────

/// `impl_state!(Type { a, b, c } skip { d, e })`: saves and reloads the listed
/// fields, in order; those in `skip` are not part of the state.
/// The destructuring forces every field of the type to be named.
macro_rules! impl_state {
    ($t:ty { $($f:ident),* $(,)? } $(skip { $($s:ident),* $(,)? })?) => {
        impl $crate::snapshot::State for $t {
            fn save(&self, w: &mut $crate::snapshot::Writer) {
                let Self { $($f,)* $($($s: _,)*)? } = self;
                $( $crate::snapshot::State::save($f, w); )*
            }
            fn load(&mut self, r: &mut $crate::snapshot::Reader) -> $crate::snapshot::Result<()> {
                let Self { $($f,)* $($($s: _,)*)? } = self;
                $( $crate::snapshot::State::load($f, r)?; )*
                Ok(())
            }
        }
    };
}
pub(crate) use impl_state;

/// `impl_state_enum!(Type { A, B, C })`: data-less enum, saved as the variant
/// index. The exhaustive `match` forces all variants to be listed.
macro_rules! impl_state_enum {
    ($t:ty { $($v:ident),* $(,)? }) => {
        impl $crate::snapshot::State for $t {
            fn save(&self, w: &mut $crate::snapshot::Writer) {
                type E = $t;
                const ALL: &[E] = &[$(E::$v),*];
                match self { $(E::$v)|* => {} }
                let i = ALL.iter().position(|v| v == self).unwrap() as u8;
                $crate::snapshot::State::save(&i, w);
            }
            fn load(&mut self, r: &mut $crate::snapshot::Reader) -> $crate::snapshot::Result<()> {
                type E = $t;
                const ALL: &[E] = &[$(E::$v),*];
                let mut i = 0u8;
                $crate::snapshot::State::load(&mut i, r)?;
                *self = *ALL.get(i as usize)
                    .ok_or_else(|| format!("{}: invalid variant {i}", stringify!($t)))?;
                Ok(())
            }
        }
    };
}
pub(crate) use impl_state_enum;

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Default, Debug, PartialEq)]
    struct Sample {
        a: u8,
        b: u16,
        c: bool,
        d: [u32; 3],
        e: Vec<i16>,
        f: Option<u64>,
        g: VecDeque<char>,
        h: f32,
        cache: u32,
    }
    impl_state!(Sample { a, b, c, d, e, f, g, h } skip { cache });

    #[test]
    fn round_trip() {
        let s = Sample {
            a: 1, b: 0xBEEF, c: true, d: [7, 8, 9], e: vec![-1, 2],
            f: Some(u64::MAX), g: "ciao".chars().collect(), h: 1.5, cache: 42,
        };
        let mut w = Writer::new();
        s.save(&mut w);
        let mut t = Sample::default();
        let mut r = Reader::new(&w.buf);
        t.load(&mut r).unwrap();
        assert!(r.is_empty());
        assert_eq!(t, Sample { cache: 0, ..s });
    }

    #[test]
    fn truncated_and_invalid_data_are_errors() {
        let mut t = Sample::default();
        assert!(t.load(&mut Reader::new(&[1, 2])).is_err());
        let mut b = false;
        assert!(b.load(&mut Reader::new(&[7])).is_err());
    }
}
