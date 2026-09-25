// SPDX-License-Identifier: GPL-2.0-or-later
// Copyright (C) 2026 SampoSoft - Francesco Sampoli

//! Messages for the emulator user (tape inserted, snapshot saved,
//! Datasette key pressed...): they go to stderr as before, and the
//! latest stays available for the window's status bar, which would
//! otherwise not show them. `notice!` is used like `eprintln!`.

use std::sync::Mutex;
use std::time::Instant;

static LAST: Mutex<Option<(String, Instant)>> = Mutex::new(None);

/// Writes the message to stderr and remembers it as the latest.
pub fn post(msg: String) {
    eprintln!("{msg}");
    if let Ok(mut last) = LAST.lock() {
        *last = Some((msg, Instant::now()));
    }
}

/// Latest message and when it arrived.
pub fn latest() -> Option<(String, Instant)> {
    LAST.lock().ok()?.clone()
}

/// Like `eprintln!`, and the message also appears in the status bar.
#[macro_export]
macro_rules! notice {
    ($($arg:tt)*) => { $crate::notice::post(format!($($arg)*)) };
}
