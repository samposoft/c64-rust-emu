// SPDX-License-Identifier: GPL-2.0-or-later
// Copyright (C) 2026 SampoSoft - Francesco Sampoli

//! c64dbg — debugger for the emulator, with no window or audio (headless) or
//! with `--window`: the same command interface, plus a window that shows the
//! screen and accepts keyboard and joystick input.
//!
//! Usage: c64dbg [-x script] [-e "cmd"]... [--no-stdin] [--window] [--audio]
//!               [machine options, see `MachineOptions::USAGE`] [file.prg|.d64|.g64|.tap|.t64|.crt]
//!
//! Commands are read, in order, from each `-e`, from the `-x` file and finally
//! from stdin (with a prompt if stdin is a terminal). `help` lists the commands.
//! With `--remote` they also arrive from the remote monitor's socket, and are
//! executed like those from stdin, in the order they arrive; stdin reaching
//! its end (or `--no-stdin`) then no longer ends the session.
//!
//! With `--window` the debugger (which owns the C64) runs on a secondary
//! thread, the window on the main thread (required by winit on macOS).
//! Debugger → window: the latest frame in a shared buffer and a wake-up via
//! `EventLoopProxy`. Window → debugger: input events on an mpsc channel,
//! drained every frame, and an atomic flag for pausing (F12). With every
//! frame the debugger also publishes the state for the bar below the screen,
//! which the window draws; clicks on the bar come back as input, and so do
//! the pointer position over the screen for the paddles and, once a click
//! on the screen has captured the host mouse for the 1351 mouse, its
//! movements and buttons (Cmd or the middle button releases it).

use c64::c64::C64;
use c64::debugger::{Debugger, FrameHook};
use c64::frontend::session::{save_on_exit, MachineOptions};
use c64::frontend::status::{self, Bar, Click, SpeedMeter, Status};
use c64::ctrlport::{Button, Device};
use c64::frontend::window::{Analog, CrtView, Display, MouseCapture, Placement, Pointer};
use c64::frontend::remote::{self, Event, Listener};
use c64::frontend::{self, AudioOut, Gamepad, JoyPort};
use c64::keyboard::HostKey;
use c64::vic::{HEIGHT, WIDTH};
use std::io::{BufRead, IsTerminal, Write};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use winit::application::ApplicationHandler;
use winit::event::{DeviceEvent, DeviceId, ElementState, MouseButton, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop, EventLoopProxy};
use winit::keyboard::PhysicalKey;
use winit::window::{Fullscreen, Window, WindowId};

struct Options {
    machine: MachineOptions,
    script: Option<String>,
    inline: Vec<String>,
    no_stdin: bool,
    window: bool,
    audio: bool,
}

fn main() {
    let mut args = std::env::args().skip(1);
    let mut o = Options {
        machine: MachineOptions::default(), script: None, inline: Vec::new(),
        no_stdin: false, window: false, audio: false,
    };

    while let Some(a) = args.next() {
        match a.as_str() {
            "-x" | "--script" => o.script = args.next(),
            "-e" => if let Some(c) = args.next() { o.inline.push(c) },
            "--no-stdin" => o.no_stdin = true,
            "--window" => o.window = true,
            "--audio" => o.audio = true,
            "-h" | "--help" => {
                println!("Usage: c64dbg [-x script] [-e \"cmd\"]... [--no-stdin] [--window] [--audio] {}", MachineOptions::USAGE);
                println!("{}", MachineOptions::HELP);
                println!("{}", Debugger::help());
                return;
            }
            other => o.machine.parse_arg(other, &mut args),
        }
    }

    if !o.window {
        run_debugger(o, Debugger::new());
        return;
    }

    let event_loop = EventLoop::<UiEvent>::with_user_event().build().expect("event loop");
    let shared = Arc::new(Shared {
        fb: Mutex::new(vec![0; WIDTH * HEIGHT]),
        status: Mutex::new(None),
        crt: Mutex::new(None),
        font: Mutex::new(Vec::new()),
        pause: AtomicBool::new(false),
        quit: AtomicBool::new(false),
        running: AtomicBool::new(false),
    });
    let (input_tx, input_rx) = mpsc::channel();
    let proxy = event_loop.create_proxy();

    let worker_shared = shared.clone();
    let worker = std::thread::spawn(move || {
        let mut dbg = Debugger::new();
        dbg.realtime = true;
        dbg.load_turbo = true;
        let audio = o.audio;
        dbg.hook = Some(Box::new(WindowLink::new(worker_shared, proxy.clone(), input_rx, audio)));
        run_debugger(o, dbg);
        let _ = proxy.send_event(UiEvent::Quit);
    });

    let mut app = DbgWindow {
        window: None, display: None, shared: shared.clone(), input: input_tx,
        bar: None, cursor: (0.0, 0.0), placement: Placement::default(), capture: MouseCapture::default(),
    };
    event_loop.run_app(&mut app).expect("event loop");

    // Window closed (or `quit`). If emulation is running, the debugger stops
    // at the next frame, finishes the command and exits by itself: wait for it.
    // If instead it is idle at the prompt, blocked on stdin, it has nothing to
    // save (the trace is flushed after every command) and the process exits.
    // No stdout flush from here: the lock belongs to the debugger thread.
    let deadline = Instant::now() + Duration::from_secs(2);
    while !worker.is_finished()
        && shared.running.load(Ordering::Relaxed)
        && Instant::now() < deadline
    {
        std::thread::sleep(Duration::from_millis(5));
    }
    if worker.is_finished() {
        let _ = worker.join();
    } else {
        std::process::exit(0);
    }
}

/// Loads ROMs and files, then runs the commands from -e, -x and stdin. On
/// exit it writes the cartridge's saves (EEPROM), if changed.
fn run_debugger(o: Options, dbg: Debugger) {
    let mut c64 = match o.machine.build() {
        Ok(c64) => c64,
        Err(e) => { eprintln!("ERROR: {e}"); std::process::exit(2); }
    };
    run_commands(o, dbg, &mut c64);
    save_on_exit(&mut c64);
}

fn run_commands(o: Options, mut dbg: Debugger, c64: &mut C64) {
    let stdout = std::io::stdout();
    let mut out = stdout.lock();

    if let Err(e) = o.machine.check_media() {
        let _ = writeln!(out, "ERROR: {e}");
    }
    if let Some(path) = o.machine.input_files.first() {
        match c64.load_file(path) {
            Ok(msg) => { let _ = writeln!(out, "{msg}"); }
            Err(e) => { let _ = writeln!(out, "ERROR: {e}"); }
        }
    }
    dbg.media = o.machine.input_files.clone();
    if let Some(h) = dbg.hook.as_mut() {
        h.idle(c64);
    }

    // -e commands: ';' separates several commands in the same string
    for cmd in o.inline {
        for c in cmd.split(';') {
            if dbg.quit { break; }
            let _ = dbg.exec(c64, c, &mut out);
        }
    }

    if let Some(path) = o.script {
        if !dbg.quit {
            match std::fs::read_to_string(&path) {
                Ok(text) => for line in text.lines() {
                    if dbg.quit { break; }
                    let _ = dbg.exec(c64, line, &mut out);
                },
                Err(e) => { let _ = writeln!(out, "ERROR: script {path}: {e}"); }
            }
        }
    }

    if dbg.quit { let _ = out.flush(); return; }
    let remote = match &o.machine.remote {
        Some(path) => match Listener::start(path) {
            Ok(l) => {
                eprintln!("Remote monitor on {}", l.path().display());
                Some(l)
            }
            Err(e) => { let _ = writeln!(out, "ERROR: {e}"); None }
        },
        None => None,
    };
    if let Some(listener) = remote {
        let _ = out.flush();
        drop(out);
        serve_remote(dbg, c64, listener, !o.no_stdin);
        return;
    }
    if o.no_stdin { let _ = out.flush(); return; }

    let stdin = std::io::stdin();
    let interactive = stdin.is_terminal();
    if interactive {
        let _ = writeln!(out, "{}", c64::frontend::session::NOTICE);
        let _ = writeln!(out, "c64dbg — 'help' for the commands, 'quit' to exit");
    }
    let mut lines = stdin.lock().lines();
    loop {
        if interactive { let _ = write!(out, "dbg> "); }
        let _ = out.flush();
        let line = match lines.next() { Some(Ok(l)) => l, _ => break };
        let _ = dbg.exec(c64, &line, &mut out);
        if dbg.quit { break; }
    }
    let _ = out.flush();
}

/// Where a command comes from, with `--remote`.
enum Source {
    Stdin(String),
    StdinClosed,
    Remote(Event),
}

/// Commands from stdin (if `stdin`) and from the remote monitor, in the
/// order they arrive. The output of the remote ones goes back to their
/// client; the session ends with `quit` from stdin or when the window closes.
fn serve_remote(mut dbg: Debugger, c64: &mut C64, listener: Listener, stdin: bool) {
    let (tx, rx) = mpsc::channel();
    let interactive = stdin && std::io::stdin().is_terminal();
    if stdin {
        let tx = tx.clone();
        std::thread::spawn(move || {
            for line in std::io::stdin().lock().lines() {
                let Ok(line) = line else { break };
                if tx.send(Source::Stdin(line)).is_err() { return; }
            }
            let _ = tx.send(Source::StdinClosed);
        });
    }
    std::thread::spawn(move || {
        while let Some(ev) = listener.recv() {
            if tx.send(Source::Remote(ev)).is_err() { return; }
        }
    });

    let stdout = std::io::stdout();
    if interactive {
        let mut out = stdout.lock();
        let _ = writeln!(out, "{}", c64::frontend::session::NOTICE);
        let _ = writeln!(out, "c64dbg — 'help' for the commands, 'quit' to exit");
    }
    let mut prompt = interactive;
    while !dbg.quit {
        if prompt {
            let mut out = stdout.lock();
            let _ = write!(out, "dbg> ");
            let _ = out.flush();
            prompt = false;
        }
        match rx.recv() {
            Ok(Source::Stdin(line)) => {
                let mut out = stdout.lock();
                let _ = dbg.exec(c64, &line, &mut out);
                let _ = out.flush();
                prompt = interactive;
            }
            Ok(Source::StdinClosed) => {}
            Ok(Source::Remote(Event::Command(req))) => {
                let cmd = req.line.split_whitespace().next().unwrap_or("").to_ascii_lowercase();
                match cmd.as_str() {
                    "hello" => req.answer(true, remote::hello("debugger").into_bytes()),
                    // The debugger runs the machine only during the commands
                    // that run it: it is always paused in between
                    "pause" | "resume" | "continue" | "cont" => req.answer(true,
                        b"c64dbg runs the machine only during run, frames, until, next, step\n".to_vec()),
                    "quit" | "q" | "exit" => req.answer(false,
                        b"the remote monitor does not quit the emulator: close the connection".to_vec()),
                    _ => {
                        let mut buf = Vec::new();
                        let line = req.line.clone();
                        match dbg.exec_result(c64, &line, &mut buf) {
                            Ok(()) => req.answer(true, buf),
                            Err(e) => {
                                buf.extend_from_slice(e.as_bytes());
                                req.answer(false, buf);
                            }
                        }
                    }
                }
            }
            Ok(Source::Remote(Event::Closed(_))) => {}
            Err(_) => break,
        }
    }
    let _ = stdout.lock().flush();
}

// ── Window ───────────────────────────────────────────────────────────────────

/// State shared between the debugger and the window.
struct Shared {
    /// Latest frame published by the debugger.
    fb: Mutex<Vec<u32>>,
    /// State for the bar, published with the frame.
    status: Mutex<Option<Status>>,
    /// CRT emulation of the frame, if on.
    crt: Mutex<Option<CrtView>>,
    /// Character ROM, for the bar's text.
    font: Mutex<Vec<u8>>,
    /// Pause requested by the window (F12), consumed by the debugger.
    pause: AtomicBool,
    /// Window closed: the debugger must quit.
    quit: AtomicBool,
    /// Emulation is running (not idle at the prompt).
    running: AtomicBool,
}

/// Debugger → window.
enum UiEvent {
    /// New frame in `Shared::fb`.
    Frame,
    Title(String),
    Quit,
}

/// Window → debugger.
enum Input {
    Key(HostKey, bool),
    /// Printable character held down by a host key (id), released by
    /// `CharUp` with the same id; `Char` types one from the queue.
    CharDown(u64, char),
    CharUp(u64),
    Char(char),
    /// The window lost the focus: release the held characters.
    ReleaseChars,
    NextJoyPort,
    /// Click on the status bar.
    Click(Click),
    /// Host mouse movement (C64 pixels) and buttons, for paddles and mouse.
    MouseMove(i32, i32),
    MouseButton(Button, bool),
    /// Pointer over the screen (framebuffer coordinates), for the paddles.
    PaddlePointer(i32, i32),
}

/// Key reminders in the status bar, in pages.
const HELP: &[&str] = &[
    "F12 pause (commands in terminal)  TAB joystick",
    "F10 full screen  click Datasette: buttons",
    "paddles: point at the screen, click = fire",
    "1351 mouse: click the screen, Cmd or middle button releases",
];

/// Debugger side: applies input, publishes frames, plays audio.
struct WindowLink {
    shared: Arc<Shared>,
    proxy: EventLoopProxy<UiEvent>,
    input: Receiver<Input>,
    joy_port: JoyPort,
    gamepad: Gamepad,
    audio: Option<AudioOut>,
    last_publish: Instant,
    running: bool,
    /// Real speed in the last frame (for the window title).
    realtime: bool,
    speed: SpeedMeter,
}

/// At maximum speed at most one frame is published every 20 ms.
const PUBLISH_INTERVAL: Duration = Duration::from_millis(20);

impl WindowLink {
    fn new(shared: Arc<Shared>, proxy: EventLoopProxy<UiEvent>, input: Receiver<Input>, audio: bool) -> Self {
        let audio = if audio {
            let a = AudioOut::open();
            if a.is_none() { eprintln!("WARN: audio not available, no sound."); }
            a
        } else {
            None
        };
        Self {
            shared, proxy, input, audio,
            joy_port: JoyPort::Two,
            gamepad: Gamepad::new(),
            last_publish: Instant::now(),
            running: false,
            realtime: true,
            speed: SpeedMeter::new(),
        }
    }

    fn apply_input(&mut self, c64: &mut C64) {
        while let Ok(ev) = self.input.try_recv() {
            match ev {
                Input::Key(hk, pressed) => frontend::apply_host_key(c64, self.joy_port, hk, pressed),
                Input::CharDown(id, c) => c64.bus.keyboard.char_down(id, c),
                Input::CharUp(id) => c64.bus.keyboard.char_up(id),
                Input::Char(c) => c64.bus.keyboard.push_char(c),
                Input::ReleaseChars => c64.bus.keyboard.release_chars(),
                Input::NextJoyPort => {
                    self.joy_port = self.joy_port.next();
                    frontend::set_joy_port(c64, self.joy_port);
                }
                Input::Click(click) => status::apply_click(c64, click),
                Input::MouseMove(dx, dy) => c64.mouse_move(dx, dy),
                Input::MouseButton(b, pressed) => c64.mouse_button(b, pressed),
                Input::PaddlePointer(x, y) => c64.point_paddles(x, y),
            }
        }
        self.gamepad.poll(&mut c64.bus.joy2);
    }

    /// Publishes the frame (the one being built if `paused`) and the bar's
    /// state.
    fn publish(&mut self, c64: &C64, paused: bool) {
        // With the CRT emulation the frame as the VIC drew it: the GPU
        // does the blending
        let crt = c64.crt().map(|crt| CrtView { crt, blend: c64.blend(), frame: c64.frame_count });
        let fb = match (paused, crt) {
            (true, _) => c64.live_framebuffer(),
            (false, Some(_)) => c64.last_frame(),
            (false, None) => &c64.framebuffer[..],
        };
        let mut shared = self.shared.fb.lock().unwrap();
        shared.clear();
        shared.extend_from_slice(fb);
        drop(shared);
        *self.shared.crt.lock().unwrap() = crt;
        let mut s = Status::of(c64);
        s.keyboard_port = self.joy_port;
        s.gamepad = self.gamepad.connected();
        if let Some(fps) = self.speed.fps() { s.fps = fps; }
        s.turbo = !self.realtime;
        s.paused = paused;
        s.message = c64::notice::latest();
        s.help = HELP;
        *self.shared.status.lock().unwrap() = Some(s);
        self.last_publish = Instant::now();
        let _ = self.proxy.send_event(UiEvent::Frame);
    }
}

impl FrameHook for WindowLink {
    fn frame(&mut self, c64: &mut C64, realtime: bool) -> bool {
        if !self.running || realtime != self.realtime {
            self.running = true;
            self.realtime = realtime;
            self.shared.running.store(true, Ordering::Relaxed);
            let title = if realtime {
                "c64dbg — running (F12 = pause)"
            } else {
                "c64dbg — running at maximum speed (F12 = pause)"
            };
            let _ = self.proxy.send_event(UiEvent::Title(title.into()));
        }
        self.apply_input(c64);
        self.speed.frame();
        if let Some(audio) = &mut self.audio {
            if realtime {
                audio.push(&c64.audio_buf, c64.audio_channels());
            }
            // At maximum speed the SID computes only its digital part
            let _ = c64.set_audio(realtime.then(|| audio.rate()));
        }
        if realtime || self.last_publish.elapsed() >= PUBLISH_INTERVAL {
            self.publish(c64, false);
        }
        self.shared.pause.swap(false, Ordering::Relaxed) || self.quit_requested()
    }

    fn quit_requested(&self) -> bool {
        self.shared.quit.load(Ordering::Relaxed)
    }

    fn idle(&mut self, c64: &mut C64) {
        self.running = false;
        self.shared.running.store(false, Ordering::Relaxed);
        self.apply_input(c64);
        // A pause requested while emulation is already stopped does not apply to the next run
        self.shared.pause.store(false, Ordering::Relaxed);
        // Frame being built: shows how far the VIC's beam has got
        let mut font = self.shared.font.lock().unwrap();
        if font.is_empty() {
            *font = c64.bus.char_rom.to_vec();
        }
        drop(font);
        self.publish(c64, true);
        let title = format!(
            "c64dbg — paused: PC ${:04X}, line {}, cycle {}, frame {}",
            c64.cpu.pc, c64.bus.vic.raster_line, c64.bus.vic.cycle, c64.frame_count
        );
        let _ = self.proxy.send_event(UiEvent::Title(title));
    }
}

/// Window side: draws the latest frame and forwards input.
struct DbgWindow {
    window: Option<Arc<Window>>,
    display: Option<Display>,
    shared: Arc<Shared>,
    input: Sender<Input>,
    /// Status bar, created when the character ROM arrives.
    bar: Option<Bar>,
    /// Mouse and image position in the window, for clicks.
    cursor: (f64, f64),
    placement: Placement,
    capture: MouseCapture,
}

impl DbgWindow {
    fn draw(&mut self) {
        let (Some(window), Some(display)) = (&self.window, &mut self.display) else { return };
        let size = window.inner_size();
        if self.bar.is_none() {
            let font = self.shared.font.lock().unwrap();
            if !font.is_empty() {
                self.bar = Some(Bar::new(&font));
            }
        }
        let fb = self.shared.fb.lock().unwrap();
        let status = self.shared.status.lock().unwrap().clone();
        let crt = *self.shared.crt.lock().unwrap();
        let bar = match (&mut self.bar, status) {
            (Some(bar), Some(mut s)) if window.fullscreen().is_none() => {
                s.mouse_captured = self.capture.captured();
                bar.render(&s);
                Some(bar.pixels())
            }
            _ => None,
        };
        if let Some(p) = display.draw((size.width, size.height), &fb, bar, crt) {
            self.placement = p;
        }
    }

    /// Paddles and mouse in the control ports, from the last published state.
    fn analog(&self) -> Analog {
        let status = self.shared.status.lock().unwrap();
        let ports = status.as_ref().map(|s| s.ports).unwrap_or_default();
        Analog { paddles: ports.contains(&Device::Paddles), mouse: ports.iter().any(|d| d.is_mouse()) }
    }

    /// Mouse button: paddle fire, 1351 buttons, capture and release, clicks
    /// on the bar (see `MouseCapture::button`).
    fn mouse_button(&mut self, button: MouseButton, pressed: bool) {
        let at = self.placement.to_image_signed(self.cursor.0, self.cursor.1);
        match self.capture.button(button, pressed, at, self.analog(), self.placement.rows) {
            Pointer::Capture => {
                let Some(w) = &self.window else { return };
                if self.capture.capture(w) {
                    c64::notice!("Mouse captured: Cmd or the middle button releases it.");
                } else {
                    c64::notice!("WARN: cannot capture the mouse pointer.");
                }
            }
            Pointer::Release => self.release_mouse(),
            Pointer::Button(b, p) => { let _ = self.input.send(Input::MouseButton(b, p)); }
            Pointer::Bar(x, y) => self.bar_click(x, y),
            Pointer::Nothing => {}
        }
    }

    /// Pointer moved: the paddles follow it over the C64 screen.
    fn pointer_moved(&mut self, x: f64, y: f64) {
        self.cursor = (x, y);
        let at = self.placement.to_image_signed(x, y);
        if let Some((x, y)) = self.capture.paddles_at(at, self.analog(), self.placement.rows) {
            let _ = self.input.send(Input::PaddlePointer(x, y));
        }
    }

    fn release_mouse(&mut self) {
        let Some(w) = &self.window else { return };
        if self.capture.captured() {
            self.capture.release(w);
            let _ = self.input.send(Input::MouseButton(Button::Left, false));
            let _ = self.input.send(Input::MouseButton(Button::Right, false));
            c64::notice!("Mouse released.");
        }
    }

    /// Click on the status bar: Datasette buttons and counter.
    fn bar_click(&mut self, x: usize, y: usize) {
        if self.window.as_ref().is_some_and(|w| w.fullscreen().is_some()) {
            return;
        }
        if let Some(c) = status::click_at(x, y) {
            let _ = self.input.send(Input::Click(c));
        }
    }

    fn key(&mut self, event: &winit::event::KeyEvent) {
        let pressed = event.state == ElementState::Pressed;
        let code = match event.physical_key {
            PhysicalKey::Code(c) => Some(c),
            _ => None,
        };
        if pressed && code.is_some_and(MouseCapture::is_release_key) {
            self.release_mouse();
            return;
        }
        if let Some(hk) = code.and_then(frontend::window::host_key) {
            if pressed && event.repeat { return; }
            if pressed {
                match hk {
                    HostKey::F12 => { self.shared.pause.store(true, Ordering::Relaxed); return; }
                    HostKey::F10 => {
                        if let Some(w) = &self.window {
                            let mode = if w.fullscreen().is_none() { Some(Fullscreen::Borderless(None)) } else { None };
                            w.set_fullscreen(mode);
                        }
                        return;
                    }
                    HostKey::Tab => { let _ = self.input.send(Input::NextJoyPort); return; }
                    _ => {}
                }
            }
            let _ = self.input.send(Input::Key(hk, pressed));
            return;
        }
        // Printable key: held down on the C64 for as long as on the host
        let id = frontend::window::key_id(&event.physical_key);
        if pressed {
            if let Some(text) = &event.text {
                let mut chars = frontend::printable_chars(text);
                if let Some(c) = chars.next() {
                    let _ = self.input.send(Input::CharDown(id, c));
                }
                for c in chars {
                    let _ = self.input.send(Input::Char(c));
                }
            }
        } else {
            let _ = self.input.send(Input::CharUp(id));
        }
    }
}

impl ApplicationHandler<UiEvent> for DbgWindow {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_some() { return; }
        let window = Arc::new(event_loop.create_window(frontend::window::window_attributes("c64dbg")).expect("window"));
        frontend::window::set_app_icon(&window);
        self.display = Some(Display::new(window.clone()));
        self.window = Some(window);
        event_loop.set_control_flow(ControlFlow::Wait);
    }

    fn user_event(&mut self, event_loop: &ActiveEventLoop, event: UiEvent) {
        match event {
            UiEvent::Frame => if let Some(w) = &self.window { w.request_redraw(); },
            UiEvent::Title(t) => if let Some(w) = &self.window { w.set_title(&t); },
            UiEvent::Quit => event_loop.exit(),
        }
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        match event {
            WindowEvent::CloseRequested => {
                self.shared.quit.store(true, Ordering::Relaxed);
                event_loop.exit();
            }
            WindowEvent::KeyboardInput { event, .. } => self.key(&event),
            WindowEvent::RedrawRequested => self.draw(),
            WindowEvent::CursorMoved { position, .. } => self.pointer_moved(position.x, position.y),
            WindowEvent::MouseInput { state, button, .. } => self.mouse_button(button, state == ElementState::Pressed),
            WindowEvent::Focused(false) => {
                self.release_mouse();
                let _ = self.input.send(Input::ReleaseChars);
            }
            _ => {}
        }
    }

    fn device_event(&mut self, _event_loop: &ActiveEventLoop, _id: DeviceId, event: DeviceEvent) {
        let (DeviceEvent::MouseMotion { delta }, Some(w)) = (event, &self.window) else { return };
        self.capture.motion(delta.0, delta.1, w, &self.placement);
        let (dx, dy) = self.capture.take();
        if (dx, dy) != (0, 0) {
            let _ = self.input.send(Input::MouseMove(dx, dy));
        }
    }
}
