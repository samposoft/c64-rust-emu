// SPDX-License-Identifier: GPL-2.0-or-later
// Copyright (C) 2026 SampoSoft - Francesco Sampoli

//! Connection to the emulator's remote monitor. If nobody listens on the
//! socket, the emulator is started (`c64 --remote`, detached, so that it
//! outlives this process) and waited for; a connection lost because the
//! emulator was closed is reopened, starting it again.

use std::io;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use c64::frontend::remote::{self, Client};

/// How long a starting emulator gets to open its socket.
const START_TIMEOUT: Duration = Duration::from_secs(20);

pub struct Machine {
    socket: PathBuf,
    emulator: PathBuf,
    /// Options for the emulator, when it is started from here.
    args: Vec<String>,
    /// Emulator's stderr, when started from here.
    log: PathBuf,
    client: Option<Client>,
}

impl Machine {
    pub fn new(socket: PathBuf, emulator: PathBuf, args: Vec<String>, log: PathBuf) -> Self {
        Self { socket, emulator, args, log, client: None }
    }

    /// Sends a command: its text output, or the error as text.
    pub fn cmd(&mut self, line: &str) -> Result<String, String> {
        self.cmd_bytes(line).map(|b| String::from_utf8_lossy(&b).into_owned())
    }

    /// Sends a command: its output as bytes (the PNG of `screenshot -`).
    pub fn cmd_bytes(&mut self, line: &str) -> Result<Vec<u8>, String> {
        let reply = self.send(line)?;
        if reply.ok {
            Ok(reply.data)
        } else {
            Err(String::from_utf8_lossy(&reply.data).trim_end().to_string())
        }
    }

    fn send(&mut self, line: &str) -> Result<remote::Reply, String> {
        for attempt in 0..2 {
            if self.client.is_none() {
                self.client = Some(self.connect()?);
            }
            match self.client.as_mut().unwrap().command(line) {
                Ok(reply) => return Ok(reply),
                Err(e) => {
                    self.client = None;
                    // Emulator closed since the last command: once more,
                    // with a new connection (and a new emulator)
                    let gone = matches!(e.kind(), io::ErrorKind::BrokenPipe
                        | io::ErrorKind::UnexpectedEof | io::ErrorKind::ConnectionReset);
                    if !gone || attempt == 1 {
                        return Err(format!("connection to the emulator lost: {e}"));
                    }
                }
            }
        }
        unreachable!()
    }

    fn connect(&self) -> Result<Client, String> {
        if let Ok(c) = Client::connect(&self.socket) {
            return Ok(c);
        }
        let mut child = self.start()?;
        let deadline = Instant::now() + START_TIMEOUT;
        loop {
            std::thread::sleep(Duration::from_millis(100));
            if let Ok(c) = Client::connect(&self.socket) {
                return Ok(c);
            }
            if let Ok(Some(status)) = child.try_wait() {
                return Err(format!("the emulator {} exited ({status}) without opening the remote monitor{}",
                    self.emulator.display(), self.log_tail()));
            }
            if Instant::now() > deadline {
                return Err(format!("the emulator {} did not open {} within {} s{}",
                    self.emulator.display(), self.socket.display(), START_TIMEOUT.as_secs(), self.log_tail()));
            }
        }
    }

    /// Starts the emulator with the remote monitor on our socket. Its
    /// standard output must not reach ours (the MCP channel); its own
    /// process group keeps it alive when the client stops this server.
    fn start(&self) -> Result<std::process::Child, String> {
        let mut cmd = Command::new(&self.emulator);
        if self.socket == remote::default_path() {
            cmd.arg("--remote");
        } else {
            cmd.arg("--remote-socket").arg(&self.socket);
        }
        cmd.args(&self.args);
        let log = std::fs::File::create(&self.log).map(Stdio::from).unwrap_or_else(|_| Stdio::null());
        cmd.stdin(Stdio::null()).stdout(Stdio::null()).stderr(log);
        #[cfg(unix)]
        {
            use std::os::unix::process::CommandExt;
            cmd.process_group(0);
        }
        eprintln!("c64mcp: starting {}", self.emulator.display());
        cmd.spawn().map_err(|e| format!("cannot start the emulator {}: {e}", self.emulator.display()))
    }

    /// The last lines the started emulator wrote, for error messages.
    fn log_tail(&self) -> String {
        let text = std::fs::read_to_string(&self.log).unwrap_or_default();
        let lines: Vec<&str> = text.lines().collect();
        if lines.is_empty() {
            return String::new();
        }
        format!("; its messages:\n{}", lines[lines.len().saturating_sub(10)..].join("\n"))
    }
}

/// Default emulator: `c64` next to this executable.
pub fn default_emulator() -> PathBuf {
    std::env::current_exe().ok()
        .and_then(|p| p.parent().map(|d| d.join("c64")))
        .unwrap_or_else(|| PathBuf::from("c64"))
}

/// Finds an external program in the PATH or where Homebrew installs it:
/// the MCP client may start us with a minimal PATH.
pub fn find_program(name: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH").unwrap_or_default();
    std::env::split_paths(&path)
        .chain(["/opt/homebrew/bin", "/usr/local/bin", "/usr/bin"].iter().map(PathBuf::from))
        .map(|d| d.join(name))
        .find(|p| is_executable(p))
}

fn is_executable(p: &Path) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        p.metadata().is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
    }
    #[cfg(not(unix))]
    {
        p.is_file()
    }
}
