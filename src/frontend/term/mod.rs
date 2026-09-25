// SPDX-License-Identifier: GPL-2.0-or-later
// Copyright (C) 2026 SampoSoft - Francesco Sampoli

//! Terminal frontend (`c64term`), libc only: raw mode, detection of the Kitty
//! protocols (keyboard and graphics), signals, message capture and terminal
//! restore on exit, even after a panic.
//!
//! Two modes ([`Mode`]): inline, in the rows below the prompt, with the last
//! frame left in the scrollback; fullscreen, in the alternate screen, which
//! disappears on exit.
//!
//! The emulator's messages (`eprintln!`, `println!`) would land in the middle
//! of the screen: while the terminal is in use, stdout and stderr go to a
//! pipe; the latest line is shown in the status line and everything is
//! reprinted to stderr on exit.

pub mod input;
pub mod screen;

use std::fs::{File, OpenOptions};
use std::io::{BufRead, BufReader, Read, Write};
use std::os::fd::{AsRawFd, FromRawFd, RawFd};
use std::os::unix::fs::OpenOptionsExt;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, Once};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use input::{Event, Parser};
use screen::{base64, shm_name, shm_unlink, shm_write, Transfer};

/// Terminal size in cells and, if the terminal reports it, in pixels.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct TermSize {
    pub cols: usize,
    pub rows: usize,
    pub xpix: usize,
    pub ypix: usize,
}

/// Where the emulator runs in the terminal.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Mode {
    /// Rows reserved below the prompt; the last frame stays on exit.
    Inline,
    /// The whole window, in the alternate screen (`--fullscreen`).
    Fullscreen,
}

/// Terminal features detected at startup.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Caps {
    /// Kitty keyboard protocol: press and release of every key.
    pub kitty_keyboard: bool,
    /// Kitty graphics protocol, with the best transfer mode.
    pub graphics: Option<Transfer>,
    /// 24-bit color; otherwise the 256-color palette.
    pub truecolor: bool,
}

/// Keyboard protocol flags: disambiguated keys (1), event type with repeat
/// and release (2), all keys as escape codes, modifiers included (8), text
/// produced by the key (16).
const KEYBOARD_FLAGS: u32 = 1 | 2 | 8 | 16;

// ── Signals ──────────────────────────────────────────────────────────────────

static QUIT: AtomicBool = AtomicBool::new(false);
static RESIZED: AtomicBool = AtomicBool::new(false);

extern "C" fn on_signal(sig: libc::c_int) {
    if sig == libc::SIGWINCH {
        RESIZED.store(true, Ordering::Relaxed);
    } else {
        QUIT.store(true, Ordering::Relaxed);
    }
}

fn install_signal_handlers() {
    for sig in [libc::SIGWINCH, libc::SIGTERM, libc::SIGHUP, libc::SIGINT, libc::SIGQUIT] {
        // SAFETY: the handler only touches atomics; sigaction zeroed and filled in
        unsafe {
            let mut sa: libc::sigaction = std::mem::zeroed();
            sa.sa_sigaction = on_signal as extern "C" fn(libc::c_int) as libc::sighandler_t;
            libc::sigemptyset(&mut sa.sa_mask);
            libc::sigaction(sig, &sa, std::ptr::null_mut());
        }
    }
}

/// Terminal closed or process terminated (SIGTERM, SIGHUP...): exit,
/// saving as with F12.
pub fn quit_requested() -> bool {
    QUIT.load(Ordering::Relaxed)
}

/// The terminal has been resized since the last call.
pub fn take_resized() -> bool {
    RESIZED.swap(false, Ordering::Relaxed)
}

// ── Restore ──────────────────────────────────────────────────────────────────

/// What is needed to put the terminal back in order, reachable from the
/// panic hook too.
struct Restore {
    tty: RawFd,
    termios: libc::termios,
    /// Sequences to write on exit (the inverse of those of `enter`).
    exit_seq: Vec<u8>,
    /// Original stdout and stderr, during the capture.
    saved: Option<(RawFd, RawFd)>,
}

static RESTORE: Mutex<Option<Restore>> = Mutex::new(None);

/// Puts the terminal back as it was; does nothing from the second time on.
fn restore() {
    let mut guard = RESTORE.lock().unwrap_or_else(|e| e.into_inner());
    let Some(r) = guard.take() else { return };
    // SAFETY: descriptors opened by us and still valid
    unsafe {
        libc::write(r.tty, r.exit_seq.as_ptr().cast(), r.exit_seq.len());
        libc::tcsetattr(r.tty, libc::TCSAFLUSH, &r.termios);
        if let Some((out, err)) = r.saved {
            libc::dup2(out, 1);
            libc::dup2(err, 2);
            libc::close(out);
            libc::close(err);
        }
    }
}

fn install_panic_hook() {
    static ONCE: Once = Once::new();
    ONCE.call_once(|| {
        let prev = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            restore();
            prev(info);
        }));
    });
}

// ── Capture of stdout and stderr ─────────────────────────────────────────────

#[derive(Default)]
struct Log {
    lines: Vec<String>,
    latest: Option<(String, Instant)>,
}

struct Capture {
    log: Arc<Mutex<Log>>,
    reader: JoinHandle<()>,
}

impl Capture {
    /// Redirects stdout and stderr to a pipe read by a thread, so writers
    /// never block. Also returns the original descriptors.
    fn start() -> std::io::Result<(Self, (RawFd, RawFd))> {
        let _ = std::io::stdout().flush();
        let mut fds = [0; 2];
        // SAFETY: POSIX calls on freshly created or standard descriptors
        let saved = unsafe {
            if libc::pipe(fds.as_mut_ptr()) != 0 { return Err(std::io::Error::last_os_error()); }
            let saved = (libc::dup(1), libc::dup(2));
            libc::dup2(fds[1], 1);
            libc::dup2(fds[1], 2);
            libc::close(fds[1]);
            saved
        };
        let log = Arc::new(Mutex::new(Log::default()));
        let thread_log = log.clone();
        // SAFETY: read end of the pipe, owned exclusively by the thread
        let pipe = unsafe { File::from_raw_fd(fds[0]) };
        let reader = std::thread::spawn(move || {
            for line in BufReader::new(pipe).lines() {
                let Ok(line) = line else { break };
                let mut log = thread_log.lock().unwrap();
                if !line.trim().is_empty() {
                    log.latest = Some((line.clone(), Instant::now()));
                }
                log.lines.push(line);
            }
        });
        Ok((Self { log, reader }, saved))
    }
}

// ── Terminal ─────────────────────────────────────────────────────────────────

pub struct Terminal {
    tty: File,
    capture: Option<Capture>,
}

impl Terminal {
    /// Opens the controlling terminal and puts it in raw mode: keys at once,
    /// no echo, Ctrl+C and Ctrl+Z as normal keys.
    pub fn open() -> std::io::Result<Self> {
        let tty = OpenOptions::new().read(true).write(true)
            .custom_flags(libc::O_CLOEXEC)
            .open("/dev/tty")?;
        let fd = tty.as_raw_fd();
        // SAFETY: valid fd; termios filled in by tcgetattr
        let orig = unsafe {
            let mut t: libc::termios = std::mem::zeroed();
            if libc::tcgetattr(fd, &mut t) != 0 { return Err(std::io::Error::last_os_error()); }
            t
        };
        *RESTORE.lock().unwrap() = Some(Restore { tty: fd, termios: orig, exit_seq: Vec::new(), saved: None });
        install_panic_hook();
        install_signal_handlers();
        let mut raw = orig;
        // SAFETY: valid termios, valid fd
        unsafe {
            libc::cfmakeraw(&mut raw);
            if libc::tcsetattr(fd, libc::TCSAFLUSH, &raw) != 0 {
                return Err(std::io::Error::last_os_error());
            }
        }
        Ok(Self { tty, capture: None })
    }

    /// Asks the terminal which protocols it supports. Terminals that do not
    /// know them ignore the queries; the reply to `CSI c`, which all of them
    /// give, ends the wait.
    pub fn detect(&mut self, allow_graphics: bool) -> Caps {
        let mut caps = Caps { kitty_keyboard: false, graphics: None, truecolor: false };
        let mut q = b"\x1b[?u".to_vec();
        let shm_query = shm_name(u32::MAX);
        if allow_graphics {
            // One black pixel, in base64 and in shared memory
            q.extend_from_slice(b"\x1b_Gi=31,s=1,v=1,a=q,t=d,f=24;AAAA\x1b\\");
            if shm_write(&shm_query, &[0, 0, 0]).is_ok() {
                q.extend_from_slice(b"\x1b_Gi=32,s=1,v=1,a=q,t=s,f=24,S=3;");
                base64(shm_query.as_bytes(), &mut q);
                q.extend_from_slice(b"\x1b\\");
            }
        }
        q.extend_from_slice(b"\x1b[c");
        self.write(&q);

        let mut parser = Parser::new();
        let deadline = Instant::now() + Duration::from_secs(2);
        'wait: while let Some(left) = deadline.checked_duration_since(Instant::now()) {
            for ev in parser.feed(&self.read(Some(left))) {
                match ev {
                    Event::KeyboardFlags(_) => caps.kitty_keyboard = true,
                    Event::Graphics { id: 31, ok: true } => {
                        caps.graphics.get_or_insert(Transfer::Direct);
                    }
                    Event::Graphics { id: 32, ok: true } => caps.graphics = Some(Transfer::Shm),
                    Event::DeviceAttributes => break 'wait,
                    _ => {}
                }
            }
        }
        shm_unlink(&shm_query);
        let colorterm = std::env::var("COLORTERM").unwrap_or_default();
        caps.truecolor = matches!(colorterm.as_str(), "truecolor" | "24bit")
            || caps.graphics.is_some()
            || caps.kitty_keyboard;
        caps
    }

    /// Hides the cursor, enables the keyboard protocol and focus events,
    /// captures messages; in fullscreen switches to the alternate screen.
    /// `image_id` is the image to delete on exit in fullscreen (inline, the
    /// last frame stays).
    pub fn enter(&mut self, caps: &Caps, mode: Mode, image_id: u32) {
        let fullscreen = mode == Mode::Fullscreen;
        // Title saved, cursor hidden, no auto-wrap, focus
        // events
        let mut enter = b"\x1b[22;0t".to_vec();
        if fullscreen { enter.extend_from_slice(b"\x1b[?1049h"); }
        enter.extend_from_slice(b"\x1b[?25l\x1b[?7l\x1b[?1004h");
        let mut exit = Vec::new();
        if caps.kitty_keyboard {
            // Each screen has its own flag stack: pop them before
            // leaving the alternate one
            enter.extend_from_slice(format!("\x1b[>{KEYBOARD_FLAGS}u").as_bytes());
            exit.extend_from_slice(b"\x1b[<u");
        }
        if fullscreen && caps.graphics.is_some() {
            exit.extend_from_slice(format!("\x1b_Ga=d,d=I,i={image_id},q=2\x1b\\").as_bytes());
        }
        exit.extend_from_slice(b"\x1b[?1004l\x1b[?7h\x1b[0m\x1b[?25h");
        if fullscreen { exit.extend_from_slice(b"\x1b[?1049l"); }
        exit.extend_from_slice(b"\x1b[23;0t");
        self.write(&enter);

        let saved = match Capture::start() {
            Ok((capture, saved)) => { self.capture = Some(capture); Some(saved) }
            Err(_) => None,
        };
        if let Some(r) = RESTORE.lock().unwrap().as_mut() {
            r.exit_seq = exit;
            r.saved = saved;
        }
    }

    pub fn size(&self) -> TermSize {
        // SAFETY: winsize filled in by ioctl on a valid fd
        let ws = unsafe {
            let mut ws: libc::winsize = std::mem::zeroed();
            libc::ioctl(self.tty.as_raw_fd(), libc::TIOCGWINSZ, &mut ws);
            ws
        };
        TermSize {
            cols: if ws.ws_col > 0 { ws.ws_col as usize } else { 80 },
            rows: if ws.ws_row > 0 { ws.ws_row as usize } else { 24 },
            xpix: ws.ws_xpixel as usize,
            ypix: ws.ws_ypixel as usize,
        }
    }

    /// Cursor row (from 0), queried with `CSI 6n`. Other events received in
    /// the meantime (keys) are returned to be applied.
    pub fn cursor_row(&mut self, parser: &mut Parser) -> (Option<usize>, Vec<Event>) {
        parser.expect_cursor_report();
        self.write(b"\x1b[6n");
        let mut others = Vec::new();
        let deadline = Instant::now() + Duration::from_secs(1);
        let mut found = None;
        while found.is_none() {
            let Some(left) = deadline.checked_duration_since(Instant::now()) else { break };
            for ev in parser.feed(&self.read(Some(left))) {
                match ev {
                    Event::CursorPosition { row, .. } if found.is_none() => found = Some(row.saturating_sub(1)),
                    ev => others.push(ev),
                }
            }
        }
        (found, others)
    }

    /// Waits for input at most `timeout` (`None` = no limit) and returns the
    /// bytes read, possibly none.
    pub fn read(&mut self, timeout: Option<Duration>) -> Vec<u8> {
        // select, not poll: on macOS poll does not support devices such as
        // /dev/tty (it returns POLLNVAL and the read would block)
        let fd = self.tty.as_raw_fd();
        let mut tv = timeout.map(|t| libc::timeval {
            tv_sec: t.as_secs().min(i32::MAX as u64) as libc::time_t,
            tv_usec: t.subsec_micros() as libc::suseconds_t,
        });
        let tv_ptr = tv.as_mut().map_or(std::ptr::null_mut(), |t| t as *mut libc::timeval);
        // SAFETY: fd_set zeroed with a single valid descriptor, below FD_SETSIZE
        let n = unsafe {
            let mut set: libc::fd_set = std::mem::zeroed();
            libc::FD_ZERO(&mut set);
            libc::FD_SET(fd, &mut set);
            libc::select(fd + 1, &mut set, std::ptr::null_mut(), std::ptr::null_mut(), tv_ptr)
        };
        if n <= 0 { return Vec::new(); } // timeout or signal
        let mut buf = vec![0; 65536];
        match self.tty.read(&mut buf) {
            Ok(0) => { QUIT.store(true, Ordering::Relaxed); Vec::new() }
            Ok(n) => { buf.truncate(n); buf }
            Err(_) => Vec::new(),
        }
    }

    /// Writes everything; if the terminal is gone, quit.
    pub fn write(&mut self, bytes: &[u8]) {
        if !bytes.is_empty() && self.tty.write_all(bytes).is_err() {
            QUIT.store(true, Ordering::Relaxed);
        }
    }

    /// Latest emulator message and when it arrived.
    pub fn latest_message(&self) -> Option<(String, Instant)> {
        self.capture.as_ref().and_then(|c| c.log.lock().unwrap().latest.clone())
    }

    /// Restores the terminal and returns the captured messages.
    pub fn close(mut self) -> Vec<String> {
        let _ = std::io::stdout().flush();
        restore();
        match self.capture.take() {
            Some(c) => {
                // stdout and stderr restored: the pipe has no writers left
                let _ = c.reader.join();
                std::mem::take(&mut c.log.lock().unwrap().lines)
            }
            None => Vec::new(),
        }
    }
}

impl Drop for Terminal {
    fn drop(&mut self) {
        restore();
    }
}

/// Black background in SGR, 24-bit or from the 256-color palette.
pub fn black_background(truecolor: bool) -> &'static str {
    if truecolor { "\x1b[0;48;2;0;0;0m" } else { "\x1b[0;48;5;16m" }
}

/// Clears the screen with a black background.
pub fn clear(out: &mut Vec<u8>, truecolor: bool) {
    out.extend_from_slice(black_background(truecolor).as_bytes());
    out.extend_from_slice(b"\x1b[2J");
}

/// Clears `n` whole rows starting at `row` (from 0) with the `sgr` colors.
pub fn clear_rows(out: &mut Vec<u8>, sgr: &str, row: usize, n: usize) {
    out.extend_from_slice(sgr.as_bytes());
    for r in row..row + n {
        let _ = write!(out, "\x1b[{};1H\x1b[2K", r + 1);
    }
}

/// Writes `text` on row `row` (from 0) with the `sgr` colors, truncated to
/// width `cols`.
pub fn status_line(out: &mut Vec<u8>, row: usize, cols: usize, sgr: &str, text: &str) {
    let _ = write!(out, "\x1b[{};1H{sgr}\x1b[2K", row + 1);
    let text: String = text.chars().filter(|c| !c.is_control()).take(cols.saturating_sub(1)).collect();
    out.extend_from_slice(text.as_bytes());
}

/// Terminal window title.
pub fn set_title(out: &mut Vec<u8>, title: &str) {
    let _ = write!(out, "\x1b]2;{title}\x07");
}
