// SPDX-License-Identifier: GPL-2.0-or-later
// Copyright (C) 2026 SampoSoft - Francesco Sampoli

//! Minimal JSON: parser and compact serializer, enough for JSON-RPC.
//! Objects keep the order of their keys.

use std::fmt::{self, Write};

#[derive(Clone, Debug, PartialEq)]
pub enum Json {
    Null,
    Bool(bool),
    Num(f64),
    Str(String),
    Arr(Vec<Json>),
    Obj(Vec<(String, Json)>),
}

/// Nesting beyond this depth is refused (no stack overflow on hostile input).
const MAX_DEPTH: usize = 128;

impl Json {
    pub fn parse(text: &str) -> Result<Json, String> {
        let mut p = Parser { s: text.as_bytes(), i: 0 };
        let v = p.value(0)?;
        p.ws();
        if p.i != p.s.len() {
            return Err(format!("unexpected text at byte {}", p.i));
        }
        Ok(v)
    }

    /// Object from (key, value) pairs.
    pub fn obj<K: Into<String>>(pairs: impl IntoIterator<Item = (K, Json)>) -> Json {
        Json::Obj(pairs.into_iter().map(|(k, v)| (k.into(), v)).collect())
    }

    pub fn get(&self, key: &str) -> Option<&Json> {
        match self {
            Json::Obj(pairs) => pairs.iter().find(|(k, _)| k == key).map(|(_, v)| v),
            _ => None,
        }
    }

    pub fn as_str(&self) -> Option<&str> {
        match self { Json::Str(s) => Some(s), _ => None }
    }

    pub fn as_f64(&self) -> Option<f64> {
        match self { Json::Num(n) => Some(*n), _ => None }
    }

    /// Non-negative integer.
    pub fn as_u64(&self) -> Option<u64> {
        self.as_f64().filter(|n| n.fract() == 0.0 && *n >= 0.0 && *n <= 9.007e15).map(|n| n as u64)
    }
}

impl From<&str> for Json {
    fn from(s: &str) -> Json { Json::Str(s.to_string()) }
}

impl From<String> for Json {
    fn from(s: String) -> Json { Json::Str(s) }
}

impl From<bool> for Json {
    fn from(b: bool) -> Json { Json::Bool(b) }
}

impl From<u64> for Json {
    fn from(n: u64) -> Json { Json::Num(n as f64) }
}

impl From<Vec<Json>> for Json {
    fn from(v: Vec<Json>) -> Json { Json::Arr(v) }
}

impl fmt::Display for Json {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            Json::Null => f.write_str("null"),
            Json::Bool(b) => write!(f, "{b}"),
            Json::Num(n) if !n.is_finite() => f.write_str("null"),
            Json::Num(n) if n.fract() == 0.0 && n.abs() < 1e15 => write!(f, "{}", *n as i64),
            Json::Num(n) => write!(f, "{n}"),
            Json::Str(s) => write_str(f, s),
            Json::Arr(items) => {
                f.write_char('[')?;
                for (i, v) in items.iter().enumerate() {
                    if i > 0 { f.write_char(',')?; }
                    write!(f, "{v}")?;
                }
                f.write_char(']')
            }
            Json::Obj(pairs) => {
                f.write_char('{')?;
                for (i, (k, v)) in pairs.iter().enumerate() {
                    if i > 0 { f.write_char(',')?; }
                    write_str(f, k)?;
                    write!(f, ":{v}")?;
                }
                f.write_char('}')
            }
        }
    }
}

fn write_str(f: &mut fmt::Formatter, s: &str) -> fmt::Result {
    f.write_char('"')?;
    for c in s.chars() {
        match c {
            '"' => f.write_str("\\\"")?,
            '\\' => f.write_str("\\\\")?,
            '\n' => f.write_str("\\n")?,
            '\r' => f.write_str("\\r")?,
            '\t' => f.write_str("\\t")?,
            c if (c as u32) < 0x20 => write!(f, "\\u{:04x}", c as u32)?,
            c => f.write_char(c)?,
        }
    }
    f.write_char('"')
}

struct Parser<'a> {
    s: &'a [u8],
    i: usize,
}

impl Parser<'_> {
    fn ws(&mut self) {
        while self.i < self.s.len() && matches!(self.s[self.i], b' ' | b'\t' | b'\n' | b'\r') {
            self.i += 1;
        }
    }

    fn err<T>(&self, what: &str) -> Result<T, String> {
        Err(format!("{what} at byte {}", self.i))
    }

    fn literal(&mut self, word: &str, v: Json) -> Result<Json, String> {
        if self.s[self.i..].starts_with(word.as_bytes()) {
            self.i += word.len();
            Ok(v)
        } else {
            self.err("invalid literal")
        }
    }

    fn value(&mut self, depth: usize) -> Result<Json, String> {
        if depth > MAX_DEPTH {
            return self.err("too deeply nested");
        }
        self.ws();
        match self.s.get(self.i) {
            None => self.err("unexpected end"),
            Some(b'n') => self.literal("null", Json::Null),
            Some(b't') => self.literal("true", Json::Bool(true)),
            Some(b'f') => self.literal("false", Json::Bool(false)),
            Some(b'"') => Ok(Json::Str(self.string()?)),
            Some(b'[') => {
                self.i += 1;
                let mut items = Vec::new();
                self.ws();
                if self.s.get(self.i) == Some(&b']') {
                    self.i += 1;
                    return Ok(Json::Arr(items));
                }
                loop {
                    items.push(self.value(depth + 1)?);
                    self.ws();
                    match self.s.get(self.i) {
                        Some(b',') => self.i += 1,
                        Some(b']') => { self.i += 1; return Ok(Json::Arr(items)); }
                        _ => return self.err("expected , or ]"),
                    }
                }
            }
            Some(b'{') => {
                self.i += 1;
                let mut pairs = Vec::new();
                self.ws();
                if self.s.get(self.i) == Some(&b'}') {
                    self.i += 1;
                    return Ok(Json::Obj(pairs));
                }
                loop {
                    self.ws();
                    if self.s.get(self.i) != Some(&b'"') {
                        return self.err("expected a key");
                    }
                    let key = self.string()?;
                    self.ws();
                    if self.s.get(self.i) != Some(&b':') {
                        return self.err("expected :");
                    }
                    self.i += 1;
                    pairs.push((key, self.value(depth + 1)?));
                    self.ws();
                    match self.s.get(self.i) {
                        Some(b',') => self.i += 1,
                        Some(b'}') => { self.i += 1; return Ok(Json::Obj(pairs)); }
                        _ => return self.err("expected , or }"),
                    }
                }
            }
            Some(b'-' | b'0'..=b'9') => self.number(),
            Some(_) => self.err("unexpected character"),
        }
    }

    fn number(&mut self) -> Result<Json, String> {
        let start = self.i;
        while self.i < self.s.len() && matches!(self.s[self.i], b'-' | b'+' | b'.' | b'e' | b'E' | b'0'..=b'9') {
            self.i += 1;
        }
        std::str::from_utf8(&self.s[start..self.i]).ok()
            .and_then(|t| t.parse::<f64>().ok())
            .map(Json::Num)
            .map_or_else(|| self.err("invalid number"), Ok)
    }

    fn hex4(&mut self) -> Result<u32, String> {
        let digits = self.s.get(self.i..self.i + 4).and_then(|d| std::str::from_utf8(d).ok());
        match digits.and_then(|d| u32::from_str_radix(d, 16).ok()) {
            Some(v) => { self.i += 4; Ok(v) }
            None => self.err("invalid \\u escape"),
        }
    }

    fn string(&mut self) -> Result<String, String> {
        self.i += 1; // opening quote
        let mut out = String::new();
        loop {
            // Run of plain bytes: copied as they are (the input is UTF-8)
            let start = self.i;
            while self.i < self.s.len() && !matches!(self.s[self.i], b'"' | b'\\') {
                self.i += 1;
            }
            out.push_str(std::str::from_utf8(&self.s[start..self.i]).map_err(|_| "invalid UTF-8".to_string())?);
            match self.s.get(self.i) {
                None => return self.err("unterminated string"),
                Some(b'"') => { self.i += 1; return Ok(out); }
                _ => {}
            }
            self.i += 1; // backslash
            let Some(&e) = self.s.get(self.i) else { return self.err("unterminated string") };
            self.i += 1;
            match e {
                b'"' => out.push('"'),
                b'\\' => out.push('\\'),
                b'/' => out.push('/'),
                b'b' => out.push('\u{8}'),
                b'f' => out.push('\u{c}'),
                b'n' => out.push('\n'),
                b'r' => out.push('\r'),
                b't' => out.push('\t'),
                b'u' => {
                    let mut c = self.hex4()?;
                    // Surrogate pair
                    if (0xD800..0xDC00).contains(&c) && self.s[self.i..].starts_with(b"\\u") {
                        self.i += 2;
                        let low = self.hex4()?;
                        if !(0xDC00..0xE000).contains(&low) {
                            return self.err("invalid surrogate pair");
                        }
                        c = 0x10000 + ((c - 0xD800) << 10) + (low - 0xDC00);
                    }
                    out.push(char::from_u32(c).unwrap_or('\u{FFFD}'));
                }
                _ => return self.err("invalid escape"),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_and_print() {
        let text = r#"{"jsonrpc":"2.0","id":7,"params":{"a":[1,-2.5,true,false,null],"s":"x\"y\\z\nè😀"}}"#;
        let v = Json::parse(text).unwrap();
        assert_eq!(v.get("id").and_then(Json::as_u64), Some(7));
        let s = v.get("params").and_then(|p| p.get("s")).and_then(Json::as_str).unwrap();
        assert_eq!(s, "x\"y\\z\nè😀");
        assert_eq!(Json::parse(&v.to_string()).unwrap(), v);
        assert_eq!(Json::parse(" [ ] ").unwrap(), Json::Arr(vec![]));
        assert_eq!(Json::obj([("n", Json::from(3u64)), ("c", Json::from("\u{1}"))]).to_string(), r#"{"n":3,"c":"\u0001"}"#);
    }

    #[test]
    fn parse_errors() {
        for bad in ["", "{", "[1,]", r#"{"a" 1}"#, r#""abc"#, "tru", "1 2", r#""\x""#, &"[".repeat(500)] {
            assert!(Json::parse(bad).is_err(), "{bad:?}");
        }
    }
}
