// SPDX-License-Identifier: GPL-2.0-or-later
// Copyright (C) 2026 SampoSoft - Francesco Sampoli

//! Remote monitor: the debugger's commands from other programs, on the
//! emulator that is already running, over a Unix socket (`--remote`), in
//! the spirit of VICE's remote monitor. `c64mcp`, the MCP server for Claude,
//! drives the emulator through it.
//!
//! Protocol: the client sends one command per line, as typed at the `c64dbg`
//! prompt; every command gets exactly one response, `OK <n>\n` or
//! `ERR <n>\n` followed by n bytes: the command's output (text, or the PNG
//! of `screenshot -`) or the error message. The commands of a connection
//! are executed in order; several connections are served together. `hello`
//! answers with the protocol version and the kind of frontend.
//!
//! The socket is created with permissions 0600, in the user's private
//! temporary directory on macOS (`$XDG_RUNTIME_DIR` on Linux, when set):
//! only the user's own programs can connect, since the commands can write
//! files (`screenshot`, `savestate`, `audio`).
//!
//! In `c64` and `c64term` (`RemoteMonitor`) the machine keeps running at
//! its own pace between commands, which are executed between two frames.
//! Those that run the emulation (`run`, `frames`, `cycles`, `until`,
//! `next`) answer when they finish, while the frames keep coming at real
//! speed. A stop (breakpoint, watchpoint...) pauses the machine; `pause`
//! and `resume` pause it and resume it by hand, `step` pauses it. The pause
//! also ends with a reset (`reset`, F11) or when the connection that asked
//! for it closes. In `c64dbg` the commands go to its debugger, like those
//! typed at the prompt.

use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::sync::mpsc::Sender;

use crate::c64::{StepResult, C64};
use crate::debugger::{parse_addr, parse_dec, reg_line, Debugger, Limit, Stop};

/// Version of the protocol, in the reply to `hello`.
pub const PROTOCOL: u32 = 1;

/// Reply to `hello`: protocol, emulator and how the frontend runs the
/// machine (`running`: at real speed between commands; `debugger`: only
/// during the commands that run it).
pub fn hello(mode: &str) -> String {
    format!("c64 remote monitor {PROTOCOL}; {} {}; {mode}\n",
        super::session::APP_NAME, env!("CARGO_PKG_VERSION"))
}

/// Socket used by `--remote` and by `c64mcp` when no path is given.
pub fn default_path() -> PathBuf {
    sys::default_path()
}

/// A command received from a connection. If it is dropped without an
/// answer, the client gets an error.
pub struct Request {
    /// Connection it came from.
    pub conn: u64,
    pub line: String,
    reply: Option<Sender<Reply>>,
}

/// Response to a command.
pub struct Reply {
    pub ok: bool,
    pub data: Vec<u8>,
}

impl Request {
    pub fn answer(mut self, ok: bool, data: Vec<u8>) {
        if let Some(tx) = self.reply.take() {
            let _ = tx.send(Reply { ok, data });
        }
    }
}

impl Drop for Request {
    fn drop(&mut self) {
        if let Some(tx) = self.reply.take() {
            let _ = tx.send(Reply { ok: false, data: b"command cancelled".to_vec() });
        }
    }
}

/// What the listener receives.
pub enum Event {
    Command(Request),
    /// The connection was closed.
    Closed(u64),
}

pub use sys::{Client, Listener};

#[cfg(unix)]
mod sys {
    use super::{Event, Reply, Request};
    use std::io::{self, BufRead, BufReader, Read, Write};
    use std::os::unix::fs::PermissionsExt;
    use std::os::unix::net::{UnixListener, UnixStream};
    use std::path::{Path, PathBuf};
    use std::sync::mpsc::{self, Receiver, Sender};

    #[cfg(target_os = "macos")]
    pub fn default_path() -> PathBuf {
        // The per-user temporary directory (0700), whatever the environment
        // of the process: Claude Desktop starts c64mcp with a minimal one
        use std::os::unix::ffi::OsStringExt;
        let mut buf = vec![0u8; 1024];
        let n = unsafe { libc::confstr(libc::_CS_DARWIN_USER_TEMP_DIR, buf.as_mut_ptr().cast(), buf.len()) };
        let dir = if n > 1 && n <= buf.len() {
            buf.truncate(n - 1);
            PathBuf::from(std::ffi::OsString::from_vec(buf))
        } else {
            std::env::temp_dir()
        };
        dir.join("c64-remote.sock")
    }

    #[cfg(not(target_os = "macos"))]
    pub fn default_path() -> PathBuf {
        match std::env::var_os("XDG_RUNTIME_DIR") {
            Some(dir) => PathBuf::from(dir).join("c64-remote.sock"),
            None => PathBuf::from(format!("/tmp/c64-remote-{}.sock", unsafe { libc::getuid() })),
        }
    }

    /// Socket of the remote monitor: a thread accepts the connections and
    /// one per connection reads the commands; they arrive here as events.
    pub struct Listener {
        events: Receiver<Event>,
        path: PathBuf,
    }

    impl Listener {
        /// Creates the socket at `path`. A socket left there by an emulator
        /// that no longer runs is replaced; one still in use is an error.
        pub fn start(path: &Path) -> Result<Self, String> {
            if path.symlink_metadata().is_ok() {
                if UnixStream::connect(path).is_ok() {
                    return Err(format!("remote monitor: {} is in use by another emulator", path.display()));
                }
                let _ = std::fs::remove_file(path);
            }
            // Private from the start: the umask covers the moment between
            // bind and chmod
            let old = unsafe { libc::umask(0o177) };
            let bound = UnixListener::bind(path);
            unsafe { libc::umask(old) };
            let listener = bound.map_err(|e| format!("remote monitor: {}: {e}", path.display()))?;
            let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600));

            let (tx, events) = mpsc::channel();
            std::thread::spawn(move || {
                let mut next = 0u64;
                for stream in listener.incoming() {
                    let Ok(stream) = stream else { continue };
                    next += 1;
                    let (conn, tx) = (next, tx.clone());
                    std::thread::spawn(move || serve(conn, stream, tx));
                }
            });
            Ok(Self { events, path: path.to_path_buf() })
        }

        pub fn path(&self) -> &Path { &self.path }

        /// Next event, if one has arrived.
        pub fn try_recv(&self) -> Option<Event> {
            self.events.try_recv().ok()
        }

        /// Waits for the next event.
        pub fn recv(&self) -> Option<Event> {
            self.events.recv().ok()
        }
    }

    impl Drop for Listener {
        fn drop(&mut self) {
            let _ = std::fs::remove_file(&self.path);
        }
    }

    /// One connection: a command per line, each answered before reading
    /// the next.
    fn serve(conn: u64, stream: UnixStream, events: Sender<Event>) {
        let Ok(read_half) = stream.try_clone() else { return };
        let mut reader = BufReader::new(read_half);
        let mut writer = stream;
        let mut line = Vec::new();
        loop {
            line.clear();
            match reader.read_until(b'\n', &mut line) {
                Ok(0) | Err(_) => break,
                Ok(_) => {}
            }
            let text = String::from_utf8_lossy(&line).trim_end_matches(['\r', '\n']).to_string();
            let (tx, rx) = mpsc::channel();
            let req = Request { conn, line: text, reply: Some(tx) };
            if events.send(Event::Command(req)).is_err() { break; }
            let Ok(reply) = rx.recv() else { break };
            let head = format!("{} {}\n", if reply.ok { "OK" } else { "ERR" }, reply.data.len());
            let sent = writer.write_all(head.as_bytes())
                .and_then(|_| writer.write_all(&reply.data))
                .and_then(|_| writer.flush());
            if sent.is_err() { break; }
        }
        let _ = events.send(Event::Closed(conn));
    }

    /// Client side of the protocol (`c64mcp`, scripts).
    pub struct Client {
        reader: BufReader<UnixStream>,
        writer: UnixStream,
    }

    impl Client {
        pub fn connect(path: &Path) -> io::Result<Self> {
            let writer = UnixStream::connect(path)?;
            let reader = BufReader::new(writer.try_clone()?);
            Ok(Self { reader, writer })
        }

        /// Sends one command and waits for its response.
        pub fn command(&mut self, line: &str) -> io::Result<Reply> {
            if line.contains('\n') {
                return Err(io::Error::new(io::ErrorKind::InvalidInput, "a command is a single line"));
            }
            self.writer.write_all(format!("{line}\n").as_bytes())?;
            self.writer.flush()?;
            let mut head = String::new();
            if self.reader.read_line(&mut head)? == 0 {
                return Err(io::ErrorKind::UnexpectedEof.into());
            }
            let bad = || io::Error::new(io::ErrorKind::InvalidData, format!("invalid response: {head:?}"));
            let (status, len) = head.trim_end().split_once(' ').ok_or_else(bad)?;
            let ok = match status { "OK" => true, "ERR" => false, _ => return Err(bad()) };
            let len: usize = len.parse().map_err(|_| bad())?;
            let mut data = vec![0; len];
            self.reader.read_exact(&mut data)?;
            Ok(Reply { ok, data })
        }
    }
}

#[cfg(not(unix))]
mod sys {
    use super::{Event, Reply};
    use std::io;
    use std::path::{Path, PathBuf};

    pub fn default_path() -> PathBuf {
        std::env::temp_dir().join("c64-remote.sock")
    }

    pub struct Listener;

    impl Listener {
        pub fn start(_path: &Path) -> Result<Self, String> {
            Err("the remote monitor needs macOS or Linux (Unix sockets)".into())
        }
        pub fn path(&self) -> &Path { Path::new("") }
        pub fn try_recv(&self) -> Option<Event> { None }
        pub fn recv(&self) -> Option<Event> { None }
    }

    pub struct Client;

    impl Client {
        pub fn connect(_path: &Path) -> io::Result<Self> {
            Err(io::Error::new(io::ErrorKind::Unsupported, "the remote monitor needs macOS or Linux"))
        }
        pub fn command(&mut self, _line: &str) -> io::Result<Reply> {
            Err(io::ErrorKind::Unsupported.into())
        }
    }
}

// ── Monitor for the real-time frontends ──────────────────────────────────────

/// Command that runs the emulation, answered when it stops.
struct Run {
    req: Request,
    limit: Limit,
    instructions: u64,
    frames: u64,
    cycles: u64,
}

impl Run {
    fn new(req: Request, limit: Limit) -> Self {
        Self { req, limit, instructions: 0, frames: 0, cycles: 0 }
    }

    /// Counts an instruction; the limit, once reached.
    fn advance(&mut self, res: &StepResult) -> Option<Stop> {
        self.instructions += 1;
        self.cycles += res.elapsed as u64;
        if res.frame_done { self.frames += 1; }
        self.reached()
    }

    fn reached(&self) -> Option<Stop> {
        match self.limit {
            Limit::Instructions(n) if self.instructions >= n => Some(Stop::Limit(format!("{n} instructions"))),
            Limit::Frames(n) if self.frames >= n => Some(Stop::Limit(format!("{n} frames"))),
            Limit::Cycles(n) if self.cycles >= n => Some(Stop::Limit(format!("{n} cycles"))),
            _ => None,
        }
    }
}

/// How a command that runs the emulation starts.
enum Start {
    Run(Limit),
    /// `next` on an instruction that is not a JSR: a step.
    Step,
}

/// The remote monitor of `c64` and `c64term`: executes the commands between
/// frames with its own debugger (breakpoints, watchpoints, trace) and runs
/// the frames, checking the stops only when there is something to check.
pub struct RemoteMonitor {
    listener: Listener,
    pub dbg: Debugger,
    paused: bool,
    /// Connection that asked for the pause: it ends when it closes.
    paused_by: Option<u64>,
    /// Commands that run the emulation, waiting for the one in progress.
    queue: VecDeque<Request>,
    active: Option<Run>,
}

impl RemoteMonitor {
    pub fn start(path: &Path) -> Result<Self, String> {
        Ok(Self {
            listener: Listener::start(path)?,
            dbg: Debugger::new(),
            paused: false,
            paused_by: None,
            queue: VecDeque::new(),
            active: None,
        })
    }

    pub fn path(&self) -> &Path { self.listener.path() }

    /// The machine is stopped (no command is running it).
    pub fn paused(&self) -> bool {
        self.paused && self.active.is_none()
    }

    /// Ends the pause (hardware reset from the frontend).
    pub fn resume(&mut self) {
        self.paused = false;
        self.paused_by = None;
    }

    /// Executes the commands that have arrived; true if there were any (the
    /// screen may have changed).
    pub fn poll(&mut self, c64: &mut C64) -> bool {
        let mut any = false;
        while let Some(ev) = self.listener.try_recv() {
            any = true;
            match ev {
                Event::Command(req) => self.command(c64, req),
                Event::Closed(conn) => self.closed(conn),
            }
        }
        self.start_next(c64);
        any
    }

    /// Runs the emulation up to the end of a frame, unless paused. Returns
    /// true if a frame was completed (false if paused, possibly in the middle
    /// of a frame).
    pub fn run_frame(&mut self, c64: &mut C64) -> bool {
        if self.paused() {
            return false;
        }
        let frames_only = self.active.as_ref().is_none_or(|r| matches!(r.limit, Limit::Frames(_)));
        if frames_only && !self.dbg.needs_checks(c64) {
            // Nothing to check: the whole frame at full speed
            c64.run_frame();
            self.dbg.record_frame(c64);
            if let Some(run) = self.active.as_mut() {
                run.frames += 1;
                if let Some(stop) = run.reached() {
                    self.finish(c64, &stop);
                }
            }
            return true;
        }
        loop {
            let res = self.dbg.step_traced(c64);
            if res.frame_done {
                self.dbg.record_frame(c64);
            }
            let stop = self.dbg.check_stop(c64)
                .or_else(|| self.active.as_mut().and_then(|r| r.advance(&res)));
            if let Some(stop) = stop {
                if !stop.is_limit() {
                    self.paused = true;
                    self.paused_by = None;
                }
                self.finish(c64, &stop);
                if self.paused() {
                    return res.frame_done;
                }
            }
            if res.frame_done {
                return true;
            }
        }
    }

    /// A command that just arrived.
    fn command(&mut self, c64: &mut C64, req: Request) {
        let line = req.line.trim();
        let cmd = line.split_whitespace().next().unwrap_or("").to_ascii_lowercase();
        match cmd.as_str() {
            "hello" => req.answer(true, hello("running").into_bytes()),
            "quit" | "q" | "exit" => req.answer(false,
                b"the remote monitor does not quit the emulator: close the connection".to_vec()),
            "speed" => req.answer(false, b"speed: in the remote monitor the frontend sets the speed".to_vec()),
            "pause" => {
                self.paused = true;
                self.paused_by = Some(req.conn);
                let text = format!("paused\n{}\n", reg_line(c64));
                req.answer(true, text.into_bytes());
            }
            "resume" | "continue" | "cont" => {
                self.resume();
                req.answer(true, b"running\n".to_vec());
            }
            "run" | "go" | "frames" | "cycles" | "until" | "next" | "n" => self.queue.push_back(req),
            "step" | "s" => {
                self.paused = true;
                self.paused_by = Some(req.conn);
                let line = req.line.clone();
                self.exec(c64, req, &line);
            }
            "reset" => {
                let line = req.line.clone();
                self.exec(c64, req, &line);
                self.resume();
            }
            _ => {
                let line = req.line.clone();
                self.exec(c64, req, &line);
            }
        }
    }

    /// Executes `line` with the debugger and answers.
    fn exec(&mut self, c64: &mut C64, req: Request, line: &str) {
        let mut out = Vec::new();
        match self.dbg.exec_result(c64, line, &mut out) {
            Ok(()) => req.answer(true, out),
            Err(e) => {
                out.extend_from_slice(e.as_bytes());
                req.answer(false, out);
            }
        }
    }

    /// Starts the next command that runs the emulation, if none is running.
    fn start_next(&mut self, c64: &mut C64) {
        while self.active.is_none() {
            let Some(req) = self.queue.pop_front() else { return };
            match self.prepare(c64, &req.line) {
                Ok(Start::Run(limit)) => self.active = Some(Run::new(req, limit)),
                Ok(Start::Step) => {
                    self.paused = true;
                    self.paused_by = Some(req.conn);
                    self.exec(c64, req, "step");
                }
                Err(e) => req.answer(false, e.into_bytes()),
            }
        }
    }

    /// Limit and temporary breakpoint of a command that runs the emulation.
    fn prepare(&mut self, c64: &C64, line: &str) -> Result<Start, String> {
        let mut words = line.split_whitespace();
        let cmd = words.next().unwrap_or("").to_ascii_lowercase();
        let arg = words.next();
        let count = |usage: &str| arg.and_then(parse_dec).filter(|&n| n > 0).ok_or_else(|| usage.to_string());
        Ok(match cmd.as_str() {
            "run" | "go" => Start::Run(Limit::Frames(match arg {
                Some(_) => count("usage: run [frames]")?,
                None => 50_000,
            })),
            "frames" => Start::Run(Limit::Frames(count("usage: frames n")?)),
            "cycles" => Start::Run(Limit::Cycles(count("usage: cycles n")?)),
            "until" => {
                self.dbg.temp_break = Some(parse_addr(arg.ok_or("usage: until addr")?)?);
                Start::Run(Limit::Instructions(50_000_000))
            }
            _ => {
                // next: a JSR runs until it returns, anything else is a step
                let pc = c64.cpu.pc;
                if c64.bus.peek(pc) != 0x20 {
                    return Ok(Start::Step);
                }
                self.dbg.temp_break = Some(pc.wrapping_add(3));
                Start::Run(Limit::Instructions(50_000_000))
            }
        })
    }

    /// The command in progress stopped, or a stop came with none running.
    fn finish(&mut self, c64: &C64, stop: &Stop) {
        self.dbg.temp_break = None;
        let text = format!("STOP: {}\n{}\n", stop.describe(), reg_line(c64));
        match self.active.take() {
            Some(run) => run.req.answer(true, text.into_bytes()),
            None => crate::notice!("Remote monitor: {}, paused", stop.describe()),
        }
    }

    fn closed(&mut self, conn: u64) {
        if self.paused_by == Some(conn) {
            self.resume();
            crate::notice!("Remote monitor: connection closed, running again");
        }
        self.queue.retain(|r| r.conn != conn);
        if self.active.as_ref().is_some_and(|r| r.req.conn == conn) {
            self.active = None;
            self.dbg.temp_break = None;
        }
    }
}
