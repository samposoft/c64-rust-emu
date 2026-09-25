// SPDX-License-Identifier: GPL-2.0-or-later
// Copyright (C) 2026 SampoSoft - Francesco Sampoli

//! c64mcp — MCP server (Model Context Protocol) that lets Claude, in Claude
//! Desktop or Claude Code, write programs for the C64 and run them on the
//! emulator.
//!
//! Usage: c64mcp [--socket PATH] [--emulator PATH] [-- emulator options]
//!
//! The client starts it and talks JSON-RPC 2.0 on stdin and stdout, one
//! message per line (the MCP stdio transport); stdout carries nothing else,
//! messages go to stderr. The tools (`tools.rs`) drive the emulator through
//! its remote monitor (`c64::frontend::remote`): if no emulator listens on
//! the socket, the first tool call starts `c64 --remote` (from the same
//! directory as this program), which keeps running when this server stops.
//! BASIC is tokenized with VICE's petcat and assembly assembled with 64tass,
//! both external programs.

mod json;
mod machine;
mod tools;

use std::io::{BufRead, Write};
use std::path::PathBuf;

use json::Json;
use machine::Machine;
use tools::Tools;

/// Protocol version answered when the client asks for none.
const DEFAULT_PROTOCOL: &str = "2025-06-18";

const USAGE: &str = "Usage: c64mcp [--socket PATH] [--emulator PATH] [-- emulator options]
MCP server (stdio) for Claude: writes BASIC and assembly programs and runs them on the emulator.
  --socket PATH    remote monitor socket (default: the one of `c64 --remote`)
  --emulator PATH  emulator started when none is listening (default: c64 next to c64mcp);
                   it gets --remote (or --remote-socket PATH) and the options after --
Needs petcat (from VICE) for BASIC and 64tass for assembly, in the PATH or in Homebrew's directory.
Claude Desktop, in claude_desktop_config.json:
  {\"mcpServers\": {\"c64\": {\"command\": \"/path/to/c64mcp\"}}}
Claude Code: claude mcp add c64 -- /path/to/c64mcp";

fn main() {
    let mut socket = None;
    let mut emulator = None;
    let mut emulator_args = Vec::new();
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        match a.as_str() {
            "--socket" => socket = args.next().map(PathBuf::from),
            "--emulator" => emulator = args.next().map(PathBuf::from),
            "--" => emulator_args.extend(args.by_ref()),
            "-h" | "--help" => { println!("{USAGE}"); return; }
            "--version" => { println!("{}", c64::frontend::session::version_text()); return; }
            other => { eprintln!("c64mcp: unknown option {other}\n{USAGE}"); std::process::exit(2); }
        }
    }
    if cfg!(not(unix)) {
        eprintln!("c64mcp needs macOS or Linux: the emulator's remote monitor uses Unix sockets");
        std::process::exit(1);
    }

    let work = std::env::temp_dir().join(format!("c64mcp-{}", std::process::id()));
    if let Err(e) = std::fs::create_dir_all(&work) {
        eprintln!("c64mcp: {}: {e}", work.display());
        std::process::exit(1);
    }
    let machine = Machine::new(
        socket.unwrap_or_else(c64::frontend::remote::default_path),
        emulator.unwrap_or_else(machine::default_emulator),
        emulator_args,
        work.join("emulator.log"),
    );
    let mut tools = Tools::new(machine, work.clone());

    let stdin = std::io::stdin();
    let stdout = std::io::stdout();
    for line in stdin.lock().lines() {
        let Ok(line) = line else { break };
        if line.trim().is_empty() { continue; }
        let reply = match Json::parse(&line) {
            Ok(msg) => handle(&mut tools, &msg),
            Err(e) => Some(error(Json::Null, -32700, &format!("parse error: {e}"))),
        };
        if let Some(reply) = reply {
            let mut out = stdout.lock();
            if writeln!(out, "{reply}").and_then(|_| out.flush()).is_err() { break; }
        }
    }
    let _ = std::fs::remove_dir_all(&work);
}

/// Response to a message; none for notifications.
fn handle(tools: &mut Tools, msg: &Json) -> Option<Json> {
    let method = msg.get("method").and_then(Json::as_str);
    let Some(id) = msg.get("id").cloned() else {
        // Notification (initialized, cancelled...) or a response: nothing to answer
        return None;
    };
    let Some(method) = method else {
        return Some(error(id, -32600, "invalid request: no method"));
    };
    let params = msg.get("params").cloned().unwrap_or(Json::Obj(Vec::new()));
    Some(match method {
        "initialize" => {
            // Our messages are the same in every version so far: the one the client asks for
            let version = params.get("protocolVersion").and_then(Json::as_str).unwrap_or(DEFAULT_PROTOCOL);
            result(id, Json::obj([
                ("protocolVersion", Json::from(version)),
                ("capabilities", Json::obj([("tools", Json::obj([("listChanged", Json::Bool(false))]))])),
                ("serverInfo", Json::obj([
                    ("name", Json::from("c64")),
                    ("title", Json::from(c64::frontend::session::APP_NAME)),
                    ("version", Json::from(env!("CARGO_PKG_VERSION"))),
                ])),
                ("instructions", Json::from(tools::INSTRUCTIONS)),
            ]))
        }
        "ping" => result(id, Json::obj::<&str>([])),
        "tools/list" => result(id, Json::obj([("tools", tools::list())])),
        "tools/call" => {
            let name = params.get("name").and_then(Json::as_str).unwrap_or("");
            let empty = Json::Obj(Vec::new());
            let args = params.get("arguments").unwrap_or(&empty);
            match tools.call(name, args) {
                Some(out) => result(id, Json::obj([
                    ("content", Json::Arr(out.content)),
                    ("isError", Json::Bool(out.is_error)),
                ])),
                None => error(id, -32602, &format!("unknown tool: {name}")),
            }
        }
        _ => error(id, -32601, &format!("method not found: {method}")),
    })
}

fn result(id: Json, result: Json) -> Json {
    Json::obj([("jsonrpc", Json::from("2.0")), ("id", id), ("result", result)])
}

fn error(id: Json, code: i64, message: &str) -> Json {
    Json::obj([
        ("jsonrpc", Json::from("2.0")),
        ("id", id),
        ("error", Json::obj([("code", Json::Num(code as f64)), ("message", Json::from(message))])),
    ])
}
