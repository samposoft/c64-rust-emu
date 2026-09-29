// SPDX-License-Identifier: GPL-2.0-or-later
// Copyright (C) 2026 SampoSoft - Francesco Sampoli

//! The MCP tools: their descriptions for the model and their implementation
//! with the remote monitor's commands, petcat (BASIC) and 64tass (assembly).

use std::path::{Path, PathBuf};
use std::process::Command;

use crate::json::Json;
use crate::machine::{find_program, Machine};

/// Heights of a screenshot through the CRT emulation (the debugger's
/// limits), and the default: twice the framebuffer, enough for the
/// scanlines and the mask while the PNG stays small (the noise of a
/// composite signal makes larger ones megabytes).
const CRT_HEIGHTS: (usize, usize) = (284, 2272);
const CRT_HEIGHT: usize = 568;

/// Frames a tool can run the emulation for: one minute, within the
/// clients' time limits for a tool call.
const MAX_FRAMES: u64 = 3000;

/// Text for the model at initialization: what the server is and how to use
/// the tools together.
pub const INSTRUCTIONS: &str = "\
Drives a Commodore 64 emulator (PAL: 6510 at 985 kHz, VIC-II, SID, 1541 drive) that the user watches in its window.
Write programs with run_basic (BASIC V2) or run_asm (6502 assembly for 64tass); both reset the machine, load the program, \
start it and return the screen as text plus a screenshot. Check the result, fix and run again.
Interact with type_text (keyboard) and joystick; let time pass with wait; look with screenshot and screen_text \
(screenshot with crt shows the picture as on a real monitor: color bleeding, scanlines, the shadow mask).
Debug with monitor, the emulator's debugger: regs, mem a b, poke a v, dis a n, break a, until a, step, next, \
watch a b, vic, sprites, sid, cia1, bank, info; addresses in hex. After a breakpoint or step the machine stays paused: \
'resume' restarts it (run_basic, run_asm and reset do too).
C64 facts: screen RAM $0400 (40x25), color RAM $D800, border $D020, background $D021, sprite pointers $07F8, \
VIC $D000, SID $D400, CIA1 $DC00 (keyboard, joystick 2 at $DC00 bits 0-4, active low), CIA2 $DD00. \
Free RAM for machine code: $C000-$CFFF, or from $0801 behind a BASIC SYS line. KERNAL: CHROUT $FFD2, GETIN $FFE4, \
PLOT $FFF0; the IRQ vector at $0314 is the usual place for a raster interrupt.";

pub struct Tools {
    machine: Machine,
    /// Directory for the sources and PRGs of the programs being built.
    work: PathBuf,
}

/// Result of a tool: MCP content blocks.
pub struct Output {
    pub content: Vec<Json>,
    pub is_error: bool,
}

impl Output {
    fn error(text: impl Into<String>) -> Output {
        Output { content: vec![text_block(text.into())], is_error: true }
    }
}

fn text_block(text: String) -> Json {
    Json::obj([("type", Json::from("text")), ("text", Json::from(text))])
}

fn image_block(png: &[u8]) -> Json {
    Json::obj([
        ("type", Json::from("image")),
        ("data", Json::from(base64(png))),
        ("mimeType", Json::from("image/png")),
    ])
}

/// Tool definition for `tools/list`.
fn tool(name: &str, title: &str, description: &str, properties: Vec<(&str, Json)>, required: &[&str]) -> Json {
    Json::obj([
        ("name", Json::from(name)),
        ("title", Json::from(title)),
        ("description", Json::from(description)),
        ("inputSchema", Json::obj([
            ("type", Json::from("object")),
            ("properties", Json::obj(properties)),
            ("required", Json::Arr(required.iter().map(|&r| Json::from(r)).collect())),
            ("additionalProperties", Json::Bool(false)),
        ])),
    ])
}

fn string_prop(description: &str) -> Json {
    Json::obj([("type", Json::from("string")), ("description", Json::from(description))])
}

fn frames_prop(description: &str, default: u64) -> Json {
    Json::obj([
        ("type", Json::from("integer")),
        ("description", Json::from(description)),
        ("minimum", Json::from(1u64)),
        ("maximum", Json::from(MAX_FRAMES)),
        ("default", Json::from(default)),
    ])
}

/// The list for `tools/list`.
pub fn list() -> Json {
    let frames_after = || frames_prop("Frames to run after the start before looking at the screen (50 per second).", 100);
    Json::Arr(vec![
        tool("run_basic", "Run a BASIC program",
            "Reset the C64, type in a BASIC V2 program and RUN it. The program is tokenized with petcat (from VICE) \
and loaded at $0801. Write it as it appears on a C64 screen: line numbers, keywords, variables and text in \
UPPERCASE; lowercase letters become shifted characters (graphics symbols in the default character set). \
Control characters in strings use petcat codes in braces: {clr} {home} {up} {down} {left} {rght} {rvon} {rvof} \
{del} {inst}, colors {blk} {wht} {red} {cyn} {pur} {grn} {blu} {yel} {orng} {brn} {lred} {gry1} {gry2} {lgrn} \
{lblu} {gry3}, a repeat as {3 down}, any code as {147}; or use CHR$(). Returns the screen as text and a screenshot.",
            vec![("source", string_prop("The program, one numbered line per line.")), ("frames", frames_after())],
            &["source"]),
        tool("run_asm", "Run an assembly program",
            "Reset the C64, assemble a 6502/6510 program with 64tass and start it. Start at $0801 with a BASIC line \
that SYSes into the code (then the tool types RUN), or at any other address such as `* = $c000` (then the tool \
types SYS to the first byte). A BASIC stub in 64tass syntax:\n\
        * = $0801\n        .word (+), 2026\n        .null $9e, format(\"%d\", start)\n+       .word 0\n\
start   inc $d020\n        jmp start\n\
64tass syntax: labels in the first column, `lda #$01`, `.byte`, `.word`, `.fill 40, $20`, `.macro`/`.endm`, \
`.if`, `+`/`-` anonymous labels, `*=` to move the program counter; illegal opcodes work too. `.text \"HELLO\"` \
and `.null` copy the characters as they are, which is PETSCII for CHROUT ($FFD2) as long as the text is in \
UPPERCASE (lowercase letters are the shifted characters); after `.enc \"screen\"` they become screen codes, to \
store straight into screen RAM. Returns the assembler's errors with line numbers, or the screen as text, a screenshot \
and the labels with their addresses (useful with the monitor tool: break, until, mem).",
            vec![("source", string_prop("The assembly source for 64tass.")), ("frames", frames_after())],
            &["source"]),
        tool("screenshot", "Screenshot",
            "The C64 screen as an image, 403x284 pixels with the border. With crt, the screen as a real set shows it, \
through the emulation of its analog video path on the GPU (color bleeding and cross-color, scanlines, shadow mask, \
halation): the set chosen with the debugger's crt command (monitor tool: `crt 1901`, `crt 1084s composite`, `crt tv`; \
it changes the user's window too), otherwise the usual one for the machine (on PAL a Commodore 1084S through \
luma/chroma). Useful to judge what artists made for a CRT: dithering, colors mixed by the blur, interlace.",
            vec![
                ("crt", Json::obj([
                    ("type", Json::from("boolean")), ("default", Json::Bool(false)),
                    ("description", Json::from("Through the monitor emulation instead of the raw pixels.")),
                ])),
                ("height", Json::obj([
                    ("type", Json::from("integer")),
                    ("description", Json::from("With crt: height of the image in pixels (the width follows the \
pixel aspect of the set).")),
                    ("minimum", Json::from(CRT_HEIGHTS.0 as u64)),
                    ("maximum", Json::from(CRT_HEIGHTS.1 as u64)),
                    ("default", Json::from(CRT_HEIGHT as u64)),
                ])),
            ],
            &[]),
        tool("screen_text", "Screen text",
            "The 40x25 text screen as characters (from screen RAM, following the VIC bank and $D018): exact and \
cheap for text output, program listings and error messages. Graphics and colors are only in the screenshot.",
            vec![], &[]),
        tool("type_text", "Type on the keyboard",
            "Type on the C64 keyboard, one key per frame, and return the screen once everything has been typed. \
A newline is RETURN. Special keys in braces: {return} {space} {runstop} {f1} ... {f8} {home} {clr} {del} {inst} \
{up} {down} {left} {right} {lshift} {rshift} {ctrl} {commodore}. Letters are typed unshifted, so they appear in \
uppercase on the default screen.",
            vec![("text", string_prop("The keys to type."))], &["text"]),
        tool("joystick", "Move the joystick",
            "Hold joystick directions and fire for some frames, then release them; returns a screenshot. Most games \
read the joystick in port 2.",
            vec![
                ("port", Json::obj([
                    ("type", Json::from("integer")), ("enum", Json::Arr(vec![Json::from(1u64), Json::from(2u64)])),
                    ("default", Json::from(2u64)), ("description", Json::from("Control port.")),
                ])),
                ("directions", Json::obj([
                    ("type", Json::from("array")),
                    ("items", Json::obj([("type", Json::from("string")), ("enum", Json::Arr(
                        ["up", "down", "left", "right", "fire"].iter().map(|&d| Json::from(d)).collect()))])),
                    ("description", Json::from("Directions and fire held together; empty to just wait with the stick released.")),
                ])),
                ("frames", frames_prop("How long to hold them, in frames (50 per second).", 25)),
            ],
            &["directions"]),
        tool("wait", "Wait",
            "Let the emulation run for some frames (50 per second), then return the screen as text and a screenshot.",
            vec![("frames", frames_prop("Frames to run.", 50))], &["frames"]),
        tool("reset", "Reset",
            "Hardware reset of the C64 (as the reset button: memory is kept), waiting for the READY prompt.",
            vec![], &[]),
        tool("monitor", "Debugger command",
            "Run commands of the emulator's debugger, one per line, and return their output. Addresses and values \
in hex, counts in decimal. Among them: regs; mem a [b]; poke a v...; fill a b v; dis [a] [n]; break a; delete a|all; \
watch a [b]; until a; step [n]; next; run [frames]; frames n; resume; pause; screen; vic; sprites; sid; cia1; cia2; \
bank; stack; info; load file; savestate file; loadstate file; keys text; joy2 up fire / joy2 none; help for all of \
them. Commands that run the emulation (run, frames, until, next) answer when they stop; a breakpoint or a step \
leaves the machine paused until resume.",
            vec![("command", string_prop("One or more debugger commands, one per line."))], &["command"]),
    ])
}

impl Tools {
    pub fn new(machine: Machine, work: PathBuf) -> Self {
        Self { machine, work }
    }

    /// Executes a tool; `None` if there is no tool by that name.
    pub fn call(&mut self, name: &str, args: &Json) -> Option<Output> {
        let r = match name {
            "run_basic" => self.run_basic(args),
            "run_asm" => self.run_asm(args),
            "screenshot" => screenshot_command(args)
                .and_then(|c| self.machine.cmd_bytes(&c))
                .map(|png| vec![image_block(&png)]),
            "screen_text" => self.machine.cmd("screen").map(|t| vec![text_block(t)]),
            "type_text" => self.type_text(args),
            "joystick" => self.joystick(args),
            "wait" => frames_arg(args, 50).and_then(|n| {
                self.machine.cmd(&format!("frames {n}"))?;
                self.look()
            }),
            "reset" => self.reset().and_then(|_| self.machine.cmd("screen")).map(|t| vec![text_block(t)]),
            "monitor" => return Some(self.monitor(args)),
            _ => return None,
        };
        Some(match r {
            Ok(content) => Output { content, is_error: false },
            Err(e) => Output::error(e),
        })
    }

    fn screenshot(&mut self) -> Result<Vec<u8>, String> {
        self.machine.cmd_bytes("screenshot -")
    }

    /// Screen as text and as an image.
    fn look(&mut self) -> Result<Vec<Json>, String> {
        let text = self.machine.cmd("screen")?;
        let png = self.screenshot()?;
        Ok(vec![text_block(text), image_block(&png)])
    }

    /// Reset, then the KERNAL's boot up to READY.
    fn reset(&mut self) -> Result<(), String> {
        self.machine.cmd("reset")?;
        for _ in 0..40 {
            self.machine.cmd("frames 10")?;
            if self.machine.cmd("screen")?.contains("READY.") {
                return Ok(());
            }
        }
        Err("after the reset the READY prompt did not appear".into())
    }

    fn run_basic(&mut self, args: &Json) -> Result<Vec<Json>, String> {
        let source = source_arg(args)?;
        let frames = frames_arg(args, 100)?;
        let petcat = find_program("petcat").ok_or("petcat not found: it comes with VICE (brew install vice)")?;
        let bas = self.work.join("program.bas");
        let prg = self.work.join("program.prg");
        let _ = std::fs::remove_file(&prg);
        std::fs::write(&bas, petcat_text(&source)).map_err(|e| format!("{}: {e}", bas.display()))?;
        let out = Command::new(&petcat).arg("-w2").arg("-o").arg(&prg).arg("--").arg(&bas).output()
            .map_err(|e| format!("cannot run {}: {e}", petcat.display()))?;
        let messages = String::from_utf8_lossy(&out.stderr).trim().to_string();
        if !out.status.success() || !prg.exists() {
            return Err(format!("petcat did not accept the program:\n{messages}"));
        }
        let mut content = self.load_and_run(&prg, frames)?;
        if !messages.is_empty() {
            content.insert(0, text_block(format!("petcat: {messages}")));
        }
        Ok(content)
    }

    fn run_asm(&mut self, args: &Json) -> Result<Vec<Json>, String> {
        let source = source_arg(args)?;
        let frames = frames_arg(args, 100)?;
        let tass = find_program("64tass").ok_or("64tass not found: install it (brew install tass64)")?;
        let asm = self.work.join("program.asm");
        let prg = self.work.join("program.prg");
        let labels = self.work.join("program.labels");
        let _ = std::fs::remove_file(&prg);
        std::fs::write(&asm, &source).map_err(|e| format!("{}: {e}", asm.display()))?;
        let out = Command::new(&tass)
            .arg("--quiet")
            .arg("-o").arg(&prg).arg("-l").arg(&labels).arg(&asm)
            .current_dir(&self.work)
            .output()
            .map_err(|e| format!("cannot run {}: {e}", tass.display()))?;
        let mut messages = String::from_utf8_lossy(&out.stderr).into_owned();
        messages.push_str(&String::from_utf8_lossy(&out.stdout));
        let messages = messages.trim().to_string();
        if !out.status.success() || !prg.exists() {
            return Err(format!("64tass errors:\n{messages}"));
        }
        let mut content = self.load_and_run(&prg, frames)?;
        let symbols = std::fs::read_to_string(&labels).unwrap_or_default();
        let symbols: Vec<&str> = symbols.lines().filter(|l| !l.trim().is_empty()).collect();
        let mut text = String::new();
        if !messages.is_empty() {
            text += &format!("64tass: {messages}\n");
        }
        if !symbols.is_empty() {
            const MAX: usize = 80;
            text += &format!("labels:\n{}", symbols[..symbols.len().min(MAX)].join("\n"));
            if symbols.len() > MAX {
                text += &format!("\n... and {} more", symbols.len() - MAX);
            }
        }
        if !text.is_empty() {
            content.insert(0, text_block(text));
        }
        Ok(content)
    }

    /// Reset, load the PRG (injected once the KERNAL is ready, at full
    /// speed, and started by the loader: RUN for BASIC, SYS for machine
    /// code), then `frames` frames of the program.
    fn load_and_run(&mut self, prg: &Path, frames: u64) -> Result<Vec<Json>, String> {
        let data = std::fs::read(prg).map_err(|e| format!("{}: {e}", prg.display()))?;
        if data.len() < 3 {
            return Err("the program is empty".into());
        }
        let start = u16::from_le_bytes([data[0], data[1]]);
        let end = start as usize + data.len() - 2;
        self.machine.cmd("reset")?;
        self.machine.cmd(&format!("load {}", prg.display()))?;
        let mut loaded = false;
        for _ in 0..60 {
            self.machine.cmd("frames 10")?;
            if self.machine.cmd("info")?.contains("prg_pending=0") {
                loaded = true;
                break;
            }
        }
        if !loaded {
            return Err("the program was not loaded: the KERNAL did not reach READY".into());
        }
        self.machine.cmd(&format!("frames {frames}"))?;
        let mut content = vec![text_block(format!(
            "loaded ${start:04X}-${:04X} ({} bytes), started with {}; screen after {frames} frames:",
            end - 1, data.len() - 2, if start == 0x0801 { "RUN".to_string() } else { format!("SYS {start}") }))];
        content.extend(self.look()?);
        Ok(content)
    }

    fn type_text(&mut self, args: &Json) -> Result<Vec<Json>, String> {
        let text = args.get("text").and_then(Json::as_str).ok_or("missing text")?;
        // One line for the monitor: newline = RETURN, backslash escaped
        let line: String = text.chars().map(|c| match c {
            '\n' => "{return}".to_string(),
            '\r' => String::new(),
            '\\' => "\\\\".to_string(),
            c => c.to_string(),
        }).collect();
        self.machine.cmd(&format!("keys \"{line}\""))?;
        // One key per frame (two for a repeated key): until the queue is empty
        let mut left = text.chars().count() as u64 * 2 + 10;
        loop {
            self.machine.cmd("frames 5")?;
            if self.machine.cmd("info")?.contains("typing=0") { break; }
            if left < 5 { return Err("the keys were not all typed".into()); }
            left -= 5;
        }
        self.look()
    }

    fn joystick(&mut self, args: &Json) -> Result<Vec<Json>, String> {
        let port = args.get("port").and_then(Json::as_u64).unwrap_or(2);
        if !(1..=2).contains(&port) {
            return Err("port: 1 or 2".into());
        }
        let frames = frames_arg(args, 25)?;
        let dirs = match args.get("directions") {
            Some(Json::Arr(items)) => items.iter()
                .map(|d| d.as_str().filter(|d| ["up", "down", "left", "right", "fire"].contains(d))
                    .ok_or("directions: up, down, left, right, fire".to_string()))
                .collect::<Result<Vec<_>, _>>()?,
            None => Vec::new(),
            _ => return Err("directions: a list".into()),
        };
        self.machine.cmd(&format!("joy{port} none"))?;
        if !dirs.is_empty() {
            self.machine.cmd(&format!("joy{port} {}", dirs.join(" ")))?;
        }
        let run = self.machine.cmd(&format!("frames {frames}"));
        self.machine.cmd(&format!("joy{port} none"))?;
        run?;
        self.look()
    }

    fn monitor(&mut self, args: &Json) -> Output {
        let Some(command) = args.get("command").and_then(Json::as_str) else {
            return Output::error("missing command");
        };
        let mut content = Vec::new();
        let mut text = String::new();
        for line in command.lines().map(str::trim).filter(|l| !l.is_empty()) {
            let is_png = line.split_whitespace().collect::<Vec<_>>() == ["screenshot", "-"];
            match self.machine.cmd_bytes(line) {
                Ok(data) if is_png => content.push(image_block(&data)),
                Ok(data) => text += &String::from_utf8_lossy(&data),
                Err(e) => {
                    text += &format!("ERROR: {e}\n");
                    content.insert(0, text_block(text));
                    return Output { content, is_error: true };
                }
            }
        }
        if !text.is_empty() || content.is_empty() {
            content.insert(0, text_block(if text.is_empty() { "ok".into() } else { text }));
        }
        Output { content, is_error: false }
    }
}

/// The debugger command for the screenshot tool: the PNG on the output, of
/// the framebuffer or through the CRT emulation.
fn screenshot_command(args: &Json) -> Result<String, String> {
    let crt = match args.get("crt") {
        None | Some(Json::Null) => false,
        Some(Json::Bool(b)) => *b,
        Some(_) => return Err("crt: true or false".into()),
    };
    let height = match args.get("height") {
        None | Some(Json::Null) => None,
        Some(v) => Some(v.as_u64().map(|h| h as usize).filter(|h| (CRT_HEIGHTS.0..=CRT_HEIGHTS.1).contains(h))
            .ok_or_else(|| format!("height: an integer from {} to {}", CRT_HEIGHTS.0, CRT_HEIGHTS.1))?),
    };
    match (crt, height) {
        (false, None) => Ok("screenshot -".into()),
        (false, Some(_)) => Err("height is for the screenshot through the CRT emulation (crt: true)".into()),
        (true, h) => Ok(format!("screenshot - crt {}", h.unwrap_or(CRT_HEIGHT))),
    }
}

fn source_arg(args: &Json) -> Result<String, String> {
    let s = args.get("source").and_then(Json::as_str).ok_or("missing source")?;
    if s.trim().is_empty() {
        return Err("the source is empty".into());
    }
    Ok(s.to_string())
}

fn frames_arg(args: &Json, default: u64) -> Result<u64, String> {
    match args.get("frames") {
        None | Some(Json::Null) => Ok(default),
        Some(v) => v.as_u64().filter(|n| (1..=MAX_FRAMES).contains(n))
            .ok_or_else(|| format!("frames: an integer from 1 to {MAX_FRAMES}")),
    }
}

/// petcat reads lowercase ASCII as the unshifted PETSCII letters (uppercase
/// on screen, and the keywords): the program written in uppercase, as on
/// the C64, gets its case swapped, except inside the {codes}.
fn petcat_text(source: &str) -> String {
    let mut out = String::with_capacity(source.len() + 1);
    let mut in_code = false;
    for c in source.chars() {
        match c {
            '{' => in_code = true,
            '}' => in_code = false,
            _ => {}
        }
        out.push(if in_code {
            c.to_ascii_lowercase()
        } else if c.is_ascii_uppercase() {
            c.to_ascii_lowercase()
        } else if c.is_ascii_lowercase() {
            c.to_ascii_uppercase()
        } else {
            c
        });
    }
    if !out.ends_with('\n') {
        out.push('\n');
    }
    out
}

/// Standard base64, with padding.
pub fn base64(data: &[u8]) -> String {
    const ABC: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(data.len().div_ceil(3) * 4);
    for chunk in data.chunks(3) {
        let b = [chunk[0], *chunk.get(1).unwrap_or(&0), *chunk.get(2).unwrap_or(&0)];
        let n = (b[0] as u32) << 16 | (b[1] as u32) << 8 | b[2] as u32;
        for i in 0..4 {
            if i <= chunk.len() {
                out.push(ABC[(n >> (18 - 6 * i) & 63) as usize] as char);
            } else {
                out.push('=');
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base64_vectors() {
        for (input, expected) in [("", ""), ("f", "Zg=="), ("fo", "Zm8="), ("foo", "Zm9v"), ("foobar", "Zm9vYmFy")] {
            assert_eq!(base64(input.as_bytes()), expected);
        }
    }

    #[test]
    fn petcat_case() {
        assert_eq!(petcat_text("10 PRINT \"{CLR}HI there\""), "10 print \"{clr}hi THERE\"\n");
    }

    #[test]
    fn screenshot_arguments() {
        let args = |text: &str| Json::parse(text).unwrap();
        assert_eq!(screenshot_command(&args("{}")).unwrap(), "screenshot -");
        assert_eq!(screenshot_command(&args(r#"{"crt": false}"#)).unwrap(), "screenshot -");
        assert_eq!(screenshot_command(&args(r#"{"crt": true}"#)).unwrap(), "screenshot - crt 568");
        assert_eq!(screenshot_command(&args(r#"{"crt": true, "height": 1136}"#)).unwrap(), "screenshot - crt 1136");
        assert!(screenshot_command(&args(r#"{"crt": true, "height": 100}"#)).is_err());
        assert!(screenshot_command(&args(r#"{"height": 568}"#)).is_err());
        assert!(screenshot_command(&args(r#"{"crt": "yes"}"#)).is_err());
    }

    #[test]
    fn tool_list_is_valid_json() {
        let text = list().to_string();
        let parsed = Json::parse(&text).unwrap();
        let Json::Arr(tools) = parsed else { panic!() };
        assert_eq!(tools.len(), 9);
        assert!(tools.iter().all(|t| t.get("inputSchema").is_some()));
    }
}
