// SPDX-License-Identifier: GPL-2.0-or-later
// Copyright (C) 2026 SampoSoft - Francesco Sampoli

//! Graphical frontend: window and input with winit, drawing with wgpu
//! (feature `gpu`, needed for the CRT emulation `--crt`) or softbuffer,
//! audio with cpal, gamepad with gilrs (feature `gamepad`). No non-Rust
//! libraries. Under the C64 screen is the status bar
//! (`frontend::status`), hidden in fullscreen; the Datasette buttons and
//! the counter can be clicked with the mouse. Paddles in a control port
//! follow the pointer over the C64 screen; with a 1351 mouse a click on the
//! screen captures the host mouse, the middle button or the Cmd
//! (Windows/Super) key releases it (`frontend::window::MouseCapture`).

use std::sync::Arc;

use winit::application::ApplicationHandler;
use winit::event::{DeviceEvent, DeviceId, ElementState, KeyEvent, MouseButton, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::keyboard::PhysicalKey;
use winit::window::{Fullscreen, Window, WindowId};

use c64::frontend::session::{save_on_exit, Hotkey, MachineOptions, Session};
use c64::frontend;
use c64::frontend::status::{self, Bar};
use c64::ctrlport::Button;
use c64::frontend::window::{Display, MouseCapture, Placement, Pointer};

// ── Application ──────────────────────────────────────────────────────────────

struct App {
    session: Session,
    window: Option<Arc<Window>>,
    display: Option<Display>,
    /// Current window title (file in use, load turbo).
    title: String,
    bar: Bar,
    /// Position of the mouse and of the image in the window, for clicks.
    cursor: (f64, f64),
    placement: Placement,
    /// Host mouse for the paddles and the 1351 mouse.
    capture: MouseCapture,
}


/// Key reminders in the status bar, in pages.
const HELP: &[&str] = &[
    "F5 save state  F9 load state  TAB joystick",
    "F8 next medium  F10 fullscreen  F11 reset",
    "F12 quit  click the Datasette buttons",
    "paddles: point at the screen, click = fire",
    "1351 mouse: click the screen, Cmd or middle button releases",
];

impl App {
    fn handle_key(&mut self, event: &KeyEvent, event_loop: &ActiveEventLoop) {
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
            match self.session.host_key(hk, pressed, event.repeat) {
                Some(Hotkey::Quit) => event_loop.exit(),
                Some(Hotkey::Fullscreen) => {
                    if let Some(w) = &self.window {
                        let new_mode = if w.fullscreen().is_none() {
                            Some(Fullscreen::Borderless(None))
                        } else {
                            None
                        };
                        w.set_fullscreen(new_mode);
                    }
                }
                None => {}
            }
            return;
        }

        // Printable key: the character of the operating system layout, held
        // down on the C64 for as long as the key (programs such as GEOS
        // debounce the keyboard); any further characters (input methods)
        // are typed from the queue
        let id = frontend::window::key_id(&event.physical_key);
        if pressed {
            if let Some(text) = &event.text {
                let mut chars = frontend::printable_chars(text);
                if let Some(c) = chars.next() {
                    self.session.char_down(id, c);
                }
                for c in chars {
                    self.session.type_char(c);
                }
            }
        } else {
            self.session.char_up(id);
        }
    }

    fn run_due_frames(&mut self) {
        let (dx, dy) = self.capture.take();
        if (dx, dy) != (0, 0) {
            self.session.mouse_move(dx, dy);
        }
        let redraw = self.session.run_due_frames();
        let Some(w) = &self.window else { return };
        if redraw {
            w.request_redraw();
        }
        let title = self.session.title();
        if title != self.title {
            w.set_title(&title);
            self.title = title;
        }
    }

    fn draw(&mut self) {
        let (Some(window), Some(display)) = (&self.window, &mut self.display) else { return };
        let size = window.inner_size();
        let (screen, crt) = self.session.display();
        let bar = if window.fullscreen().is_some() {
            None
        } else {
            let mut status = self.session.status(HELP);
            status.mouse_captured = self.capture.captured();
            self.bar.render(&status);
            Some(self.bar.pixels())
        };
        if let Some(p) = display.draw((size.width, size.height), screen, bar, crt) {
            self.placement = p;
        }
    }

    /// Mouse button: to the paddles or the mouse while captured, otherwise
    /// a click in the window.
    fn mouse_button(&mut self, button: MouseButton, pressed: bool) {
        let at = self.placement.to_image_signed(self.cursor.0, self.cursor.1);
        match self.capture.button(button, pressed, at, self.session.analog(), self.placement.rows) {
            Pointer::Capture => {
                let Some(w) = &self.window else { return };
                if self.capture.capture(w) {
                    c64::notice!("Mouse captured ({}): Cmd or the middle button releases it.", self.session.mouse_devices());
                } else {
                    c64::notice!("WARN: cannot capture the mouse pointer.");
                }
            }
            Pointer::Release => self.release_mouse(),
            Pointer::Button(b, p) => self.session.mouse_button(b, p),
            Pointer::Bar(x, y) => self.bar_click(x, y),
            Pointer::Nothing => {}
        }
    }

    /// Pointer moved: the paddles follow it over the C64 screen.
    fn pointer_moved(&mut self, x: f64, y: f64) {
        self.cursor = (x, y);
        let at = self.placement.to_image_signed(x, y);
        if let Some((x, y)) = self.capture.paddles_at(at, self.session.analog(), self.placement.rows) {
            self.session.point_paddles(x, y);
        }
    }

    fn release_mouse(&mut self) {
        let Some(w) = &self.window else { return };
        if self.capture.captured() {
            self.capture.release(w);
            self.session.mouse_button(Button::Left, false);
            self.session.mouse_button(Button::Right, false);
            c64::notice!("Mouse released.");
        }
    }

    /// Click on the status bar: Datasette buttons and counter.
    fn bar_click(&mut self, x: usize, y: usize) {
        if self.window.as_ref().is_some_and(|w| w.fullscreen().is_some()) {
            return;
        }
        if let Some(c) = status::click_at(x, y) {
            self.session.click(c);
            if let Some(w) = &self.window {
                w.request_redraw();
            }
        }
    }
}

impl ApplicationHandler for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_some() { return; }
        let attrs = frontend::window::window_attributes(&self.title);
        let window = Arc::new(event_loop.create_window(attrs).expect("window"));
        frontend::window::set_app_icon(&window);
        self.display = Some(Display::new(window.clone()));
        self.window = Some(window);
        self.session.restart_clock();
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        match event {
            WindowEvent::CloseRequested => event_loop.exit(),
            WindowEvent::KeyboardInput { event, .. } => self.handle_key(&event, event_loop),
            WindowEvent::RedrawRequested => self.draw(),
            WindowEvent::CursorMoved { position, .. } => self.pointer_moved(position.x, position.y),
            WindowEvent::MouseInput { state, button, .. } => self.mouse_button(button, state == ElementState::Pressed),
            WindowEvent::Focused(false) => {
                self.release_mouse();
                self.session.release_chars();
            }
            _ => {}
        }
    }

    fn device_event(&mut self, _event_loop: &ActiveEventLoop, _id: DeviceId, event: DeviceEvent) {
        if let (DeviceEvent::MouseMotion { delta }, Some(w)) = (event, &self.window) {
            self.capture.motion(delta.0, delta.1, w, &self.placement);
        }
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        self.run_due_frames();
        event_loop.set_control_flow(if self.session.loading() {
            ControlFlow::Poll
        } else {
            ControlFlow::WaitUntil(self.session.next_frame())
        });
    }
}

fn main() {
    // ── CLI arguments ─────────────────────────────────────────────────────────
    let mut opts = MachineOptions::default();
    let mut turbo = true;
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        match a.as_str() {
            "--no-turbo" => turbo = false,
            "-h" | "--help" => {
                println!("Usage: c64 [--no-turbo] {}", MachineOptions::USAGE);
                println!("  --no-turbo   load from disk and tape at real speed (by default they run at maximum speed)");
                println!("{}", MachineOptions::HELP);
                return;
            }
            other => opts.parse_arg(other, &mut args),
        }
    }

    // ── C64, audio, gamepad ───────────────────────────────────────────────────
    let session = match Session::new(&opts, turbo) {
        Ok(s) => s,
        Err(e) => { eprintln!("{e}"); std::process::exit(2); }
    };

    // ── Window and event loop ─────────────────────────────────────────────────
    let event_loop = EventLoop::new().expect("event loop");
    let bar = Bar::new(&session.c64.bus.char_rom);
    let mut app = App {
        title: session.title(), session, window: None, display: None, bar,
        cursor: (0.0, 0.0), placement: Placement::default(), capture: MouseCapture::default(),
    };
    event_loop.run_app(&mut app).expect("event loop");

    save_on_exit(&mut app.session.c64);
}
