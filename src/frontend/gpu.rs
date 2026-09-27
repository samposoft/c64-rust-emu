// SPDX-License-Identifier: GPL-2.0-or-later
// Copyright (C) 2026 SampoSoft - Francesco Sampoli

//! Drawing on the GPU with wgpu (feature `gpu`): the window of `c64` and
//! `c64dbg --window`, and the screenshots with the CRT emulation. Without
//! the CRT the screen and the status bar are copied with integer scaling,
//! as `window::blit_scaled` does; with it the frame goes through the model
//! of `crate::crt` (shaders in crt.wgsl).

use std::sync::Arc;

use winit::window::Window;

use super::window::{layout, CrtView, Placement, Rect};
use crate::crt::{self, Crt, Signal};
use crate::timing::Standard;
use crate::vic::{HEIGHT, WIDTH};

const SHADER: &str = include_str!("crt.wgsl");

/// Layout of the parameters (floats): the values that change with the
/// window, then the tables of the model.
const U_PAL_Y: usize = 24;
const U_PAL_UV: usize = U_PAL_Y + 16;
const U_LUMA: usize = U_PAL_UV + 64;
const U_CHROMA: usize = U_LUMA + crt::LUMA_PHASES * crt::LUMA_TAPS;
const U_LUMA_LPF: usize = U_CHROMA + crt::FIR_LEN.next_multiple_of(4);
const U_MOD: usize = U_LUMA_LPF + crt::FIR_LEN.next_multiple_of(4);
const U_LUMA_IN: usize = U_MOD + crt::SEP_LEN.next_multiple_of(4);
const U_CHROMA_IN: usize = U_LUMA_IN + crt::SEP_LEN.next_multiple_of(4);
const PARAMS: usize = U_CHROMA_IN + crt::SEP_LEN.next_multiple_of(4);

/// Format of the intermediate images (signal, light, halation).
const SIGNAL_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;

/// The shader with the constants of the model for the signal of a standard
/// in front.
fn shader_source(sig: &Signal) -> String {
    let std = sig.standard;
    format!(
        "const W: i32 = {};\nconst H: i32 = {};\nconst NS: i32 = {};\nconst PH: i32 = {};\n\
         const LUMA_TAPS: i32 = {};\nconst LUMA_FIRST: i32 = {};\nconst FIR_HALF: i32 = {};\nconst SEP_HALF: i32 = {};\n\
         const FIRST_LINE: i32 = {};\nconst LINES: i32 = {};\nconst PAL: bool = {};\nconst NV: u32 = {};\n\
         const U_PAL_Y: u32 = {U_PAL_Y};\nconst U_PAL_UV: u32 = {U_PAL_UV};\nconst U_LUMA: u32 = {U_LUMA};\n\
         const U_CHROMA: u32 = {U_CHROMA};\nconst U_LUMA_LPF: u32 = {U_LUMA_LPF};\nconst U_MOD: u32 = {U_MOD};\n\
         const U_LUMA_IN: u32 = {U_LUMA_IN};\nconst U_CHROMA_IN: u32 = {U_CHROMA_IN};\n\
         const CONTRAST: f32 = {:?};\nconst TUBE_GAMMA: f32 = {:?};\nconst DISPLAY_GAMMA: f32 = {:?};\n{SHADER}",
        WIDTH, std.fb_height(), sig.samples, sig.phases, crt::LUMA_TAPS, crt::LUMA_FIRST, crt::FIR_HALF, crt::SEP_HALF,
        std.first_fb_line(), std.raster_lines(), sig.pal(), PARAMS / 4,
        crt::CONTRAST as f32, crt::TUBE_GAMMA as f32, crt::DISPLAY_GAMMA as f32,
    )
}

/// Tables of the model: palette in YUV, luma of the VIC, filters of the
/// monitor.
fn tables(crt: Crt) -> Vec<f32> {
    let m = crt.model.monitor();
    let sig = m.signal();
    let mut t = vec![0f32; PARAMS];
    let (even, odd) = (crt::palette_yuv(false, &sig), crt::palette_yuv(true, &sig));
    for c in 0..16 {
        t[U_PAL_Y + c] = even[c][0] as f32;
        t[U_PAL_UV + 4 * c..U_PAL_UV + 4 * c + 4]
            .copy_from_slice(&[even[c][1], even[c][2], odd[c][1], odd[c][2]].map(|x| x as f32));
    }
    for (phase, k) in crt::luma_kernels(&sig).iter().enumerate() {
        t[U_LUMA + phase * crt::LUMA_TAPS..][..crt::LUMA_TAPS].copy_from_slice(k);
    }
    t[U_CHROMA..][..crt::FIR_LEN].copy_from_slice(&crt::chroma_fir(m, crt.input));
    t[U_LUMA_LPF..][..crt::FIR_LEN].copy_from_slice(&crt::luma_fir(m, crt.input));
    t[U_MOD..][..crt::SEP_LEN].copy_from_slice(&crt::modulator_fir(&sig));
    t[U_LUMA_IN..][..crt::SEP_LEN].copy_from_slice(&crt::luma_input_fir(m, crt.input));
    t[U_CHROMA_IN..][..crt::SEP_LEN].copy_from_slice(&crt::chroma_input_fir(m, crt.input));
    t
}

/// Render pipeline of the entry point `entry` of `module`.
fn pipeline(device: &wgpu::Device, module: &wgpu::ShaderModule, entry: &str, format: wgpu::TextureFormat) -> wgpu::RenderPipeline {
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some(entry),
        layout: None,
        vertex: wgpu::VertexState {
            module,
            entry_point: Some("vs"),
            compilation_options: Default::default(),
            buffers: &[],
        },
        primitive: wgpu::PrimitiveState::default(),
        depth_stencil: None,
        multisample: wgpu::MultisampleState::default(),
        fragment: Some(wgpu::FragmentState {
            module,
            entry_point: Some(entry),
            compilation_options: Default::default(),
            targets: &[Some(format.into())],
        }),
        multiview_mask: None,
        cache: None,
    })
}

fn shader(device: &wgpu::Device, sig: &Signal) -> wgpu::ShaderModule {
    device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("crt.wgsl"),
        source: wgpu::ShaderSource::Wgsl(shader_source(sig).into()),
    })
}

/// Pipelines and images of the CRT emulation for the signal of one
/// standard (lines and samples per line differ).
struct CrtSet {
    sig: Signal,
    vic: wgpu::RenderPipeline,
    cable: wgpu::RenderPipeline,
    separate: wgpu::RenderPipeline,
    decode: wgpu::RenderPipeline,
    glow_h: wgpu::RenderPipeline,
    glow_v: wgpu::RenderPipeline,
    crt: wgpu::RenderPipeline,
    /// Palette indices of the frame (R8Uint).
    index_tex: wgpu::Texture,
    /// Signal of the VIC, after the cable and after the set's input stage,
    /// light of the last two frames, halation.
    sig_tex: wgpu::TextureView,
    cab: wgpu::TextureView,
    sep: wgpu::TextureView,
    lin: [wgpu::TextureView; 2],
    glow: [wgpu::TextureView; 2],
}

impl CrtSet {
    fn new(device: &wgpu::Device, format: wgpu::TextureFormat, standard: Standard) -> CrtSet {
        let sig = Signal::of(standard);
        let module = shader(device, &sig);
        let pipe = |entry: &str, format| pipeline(device, &module, entry, format);
        let rows = standard.fb_height();
        let image = || view(&texture(device, sig.samples, rows, SIGNAL_FORMAT, true));
        CrtSet {
            vic: pipe("vic", SIGNAL_FORMAT),
            cable: pipe("cable", SIGNAL_FORMAT),
            separate: pipe("separate", SIGNAL_FORMAT),
            decode: pipe("decode_fs", SIGNAL_FORMAT),
            glow_h: pipe("glow_h", SIGNAL_FORMAT),
            glow_v: pipe("glow_v", SIGNAL_FORMAT),
            crt: pipe("crt", format),
            index_tex: texture(device, WIDTH, rows, wgpu::TextureFormat::R8Uint, false),
            sig_tex: image(),
            cab: image(),
            sep: image(),
            lin: [image(), image()],
            glow: [image(), image()],
            sig,
        }
    }
}

/// Renderer on a device, for targets of one format.
pub struct Renderer {
    device: wgpu::Device,
    queue: wgpu::Queue,
    format: wgpu::TextureFormat,
    blit: wgpu::RenderPipeline,
    /// CRT emulation for the standard of the last frame drawn with it.
    crt: Option<CrtSet>,
    params: wgpu::Buffer,
    sampler: wgpu::Sampler,
    /// The frame and the bar as they are (Bgra8Unorm: the bytes of the ARGB
    /// pixels).
    screen_tex: wgpu::Texture,
    bar_tex: wgpu::Texture,
    /// Values of the parameters, with the tables for `tables_for`.
    values: Vec<f32>,
    tables_for: Option<Crt>,
    /// Indices of the frame being uploaded and of the one in `lin[cur]`,
    /// with the settings it was computed with.
    indices: Vec<u8>,
    done: Vec<u8>,
    done_crt: Option<Crt>,
    bytes: Vec<u8>,
    /// Lines of `screen_tex`: those of the last frame (284 PAL, 253 NTSC).
    screen_rows: usize,
    /// CRT emulation asked for on a frame of another standard than its
    /// monitor's: said once.
    crt_warned: bool,
    cur: usize,
    /// `lin[cur ^ 1]` holds the previous frame.
    have_prev: bool,
}

fn texture(device: &wgpu::Device, w: usize, h: usize, format: wgpu::TextureFormat, target: bool) -> wgpu::Texture {
    let mut usage = wgpu::TextureUsages::TEXTURE_BINDING;
    usage |= if target { wgpu::TextureUsages::RENDER_ATTACHMENT } else { wgpu::TextureUsages::COPY_DST };
    device.create_texture(&wgpu::TextureDescriptor {
        label: None,
        size: wgpu::Extent3d { width: w as u32, height: h as u32, depth_or_array_layers: 1 },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage,
        view_formats: &[],
    })
}

fn view(t: &wgpu::Texture) -> wgpu::TextureView {
    t.create_view(&wgpu::TextureViewDescriptor::default())
}

impl Renderer {
    pub fn new(device: wgpu::Device, queue: wgpu::Queue, format: wgpu::TextureFormat) -> Renderer {
        let blit = pipeline(&device, &shader(&device, &Signal::of(Standard::Pal)), "blit", format);
        let params = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("params"),
            size: (PARAMS * 4) as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        let bar_h = super::status::BAR_HEIGHT;
        Renderer {
            screen_tex: texture(&device, WIDTH, HEIGHT, wgpu::TextureFormat::Bgra8Unorm, false),
            bar_tex: texture(&device, WIDTH, bar_h, wgpu::TextureFormat::Bgra8Unorm, false),
            blit, crt: None, format, params, sampler, device, queue,
            values: vec![0.0; PARAMS],
            tables_for: None,
            indices: Vec::new(),
            done: Vec::new(),
            done_crt: None,
            bytes: Vec::new(),
            screen_rows: HEIGHT,
            crt_warned: false,
            cur: 0,
            have_prev: false,
        }
    }

    fn write_image(&mut self, tex: &wgpu::Texture, pixels: &[u32]) {
        self.bytes.clear();
        self.bytes.extend(pixels.iter().flat_map(|p| p.to_le_bytes()));
        let rows = (pixels.len() / WIDTH) as u32;
        self.queue.write_texture(
            tex.as_image_copy(),
            &self.bytes,
            wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(4 * WIDTH as u32), rows_per_image: None },
            wgpu::Extent3d { width: WIDTH as u32, height: rows, depth_or_array_layers: 1 },
        );
    }

    fn bind(&self, pipe: &wgpu::RenderPipeline, entries: &[(u32, wgpu::BindingResource)]) -> wgpu::BindGroup {
        let entries: Vec<wgpu::BindGroupEntry> = entries.iter()
            .map(|(binding, resource)| wgpu::BindGroupEntry { binding: *binding, resource: resource.clone() })
            .collect();
        self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: None,
            layout: &pipe.get_bind_group_layout(0),
            entries: &entries,
        })
    }

    /// One pass of the signal chain into `target` (signal resolution).
    fn signal_pass(&self, encoder: &mut wgpu::CommandEncoder, pipe: &wgpu::RenderPipeline, group: &wgpu::BindGroup, target: &wgpu::TextureView) {
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: None,
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: target,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color::BLACK), store: wgpu::StoreOp::Store },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        pass.set_pipeline(pipe);
        pass.set_bind_group(0, group, &[]);
        pass.draw(0..3, 0..1);
    }

    /// Parameters for the CRT drawn in `screen` (window pixels).
    fn set_params(&mut self, view: &CrtView, screen: &Rect, rows: usize) {
        if self.tables_for != Some(view.crt) {
            self.values = tables(view.crt);
            self.tables_for = Some(view.crt);
        }
        let m = view.crt.model.monitor();
        let sig = m.signal();
        // Window pixels per mm on the tube, from the line pitch
        let px_mm = screen.sy / m.line_pitch();
        let samples_per_mm = sig.samples as f64 / WIDTH as f64 / m.pixel_width();
        let v = &mut self.values;
        let dynamic = [
            (view.crt.input != crt::Input::LumaChroma) as u8 as f64, view.blend as u8 as f64,
            crt::MASK_STRENGTH, m.glow.0,
            screen.x, screen.y, screen.sx * WIDTH as f64, screen.sy * rows as f64,
            m.triad_pitch * px_mm, m.slot_pitch * px_mm, m.stripe, m.bridge,
            m.beam_sigma.0, m.beam_sigma.1, m.glow.1 * samples_per_mm, m.glow.1 / m.line_pitch(),
            crt::MASK_WHITE_LOSS,
        ];
        let white = crt::white_balance(m);
        for (i, x) in dynamic.iter().chain(&white).enumerate() {
            v[i] = *x as f32;
        }
        v[20] = (view.frame & 1) as f32;
        let bytes: Vec<u8> = v.iter().flat_map(|x| x.to_le_bytes()).collect();
        self.queue.write_buffer(&self.params, 0, &bytes);
    }

    /// Signal chain of a new frame (or of new settings) into `lin[cur]` of
    /// the CRT set.
    fn run_signal(&mut self, encoder: &mut wgpu::CommandEncoder, screen: &[u32], crt: Crt, frame: u64) {
        self.indices.clear();
        let mut last = (u32::MAX, 0u8);
        for &p in screen {
            let c = p & 0xFF_FFFF;
            if c != last.0 {
                last = (c, crt::palette_index(c) as u8);
            }
            self.indices.push(last.1);
        }
        // The frame parity counts too: the NTSC subcarrier alternates
        self.indices.push((frame & 1) as u8);
        let same_crt = self.done_crt == Some(crt);
        if self.indices == self.done && same_crt {
            return;
        }
        if same_crt {
            // A new frame: the one shown so far becomes the previous one
            self.have_prev = !self.done.is_empty();
            self.cur ^= 1;
        }
        std::mem::swap(&mut self.indices, &mut self.done);
        self.done_crt = Some(crt);
        let set = self.crt.as_ref().expect("CRT set");
        let rows = screen.len() / WIDTH;
        self.queue.write_texture(
            set.index_tex.as_image_copy(),
            &self.done[..screen.len()],
            wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(WIDTH as u32), rows_per_image: None },
            wgpu::Extent3d { width: WIDTH as u32, height: rows as u32, depth_or_array_layers: 1 },
        );
        let params = self.params.as_entire_binding();
        let index = view(&set.index_tex);
        let tv = wgpu::BindingResource::TextureView;
        let vic = self.bind(&set.vic, &[(0, params.clone()), (1, tv(&index))]);
        self.signal_pass(encoder, &set.vic, &vic, &set.sig_tex);
        let mut input = &set.sig_tex;
        if crt.input != crt::Input::LumaChroma {
            let cable = self.bind(&set.cable, &[(0, params.clone()), (2, tv(&set.sig_tex))]);
            self.signal_pass(encoder, &set.cable, &cable, &set.cab);
            input = &set.cab;
        }
        let separate = self.bind(&set.separate, &[(0, params.clone()), (2, tv(input))]);
        self.signal_pass(encoder, &set.separate, &separate, &set.sep);
        let decode = self.bind(&set.decode, &[(0, params.clone()), (2, tv(&set.sep))]);
        self.signal_pass(encoder, &set.decode, &decode, &set.lin[self.cur]);
        let glow_h = self.bind(&set.glow_h, &[(0, params.clone()), (3, tv(&set.lin[self.cur]))]);
        self.signal_pass(encoder, &set.glow_h, &glow_h, &set.glow[0]);
        let glow_v = self.bind(&set.glow_v, &[(0, params), (5, tv(&set.glow[0]))]);
        self.signal_pass(encoder, &set.glow_v, &glow_v, &set.glow[1]);
    }

    /// Draws the screen (and the bar, if any) into `target`, `dw`×`dh`.
    pub fn render(&mut self, encoder: &mut wgpu::CommandEncoder, target: &wgpu::TextureView, dw: u32, dh: u32,
                  screen: &[u32], bar: Option<&[u32]>, crt: Option<CrtView>) -> Placement {
        let rows = screen.len() / WIDTH;
        // The monitor decodes one standard: the frame must be of it (after
        // loading a state of the other standard it is drawn as it is)
        let crt = match crt {
            Some(v) if v.crt.model.monitor().standard.fb_height() != rows => {
                if !self.crt_warned {
                    self.crt_warned = true;
                    crate::notice!("WARN: the {} does not show this video standard: CRT emulation off.", v.crt.model.full_name());
                }
                None
            }
            c => c,
        };
        if rows != self.screen_rows {
            self.screen_tex = texture(&self.device, WIDTH, rows, wgpu::TextureFormat::Bgra8Unorm, false);
            self.screen_rows = rows;
        }
        let aspect = crt.map(|v| v.crt.model.monitor().signal().pixel_aspect());
        let placement = layout(dw as usize, dh as usize, rows, bar.is_some(), aspect);
        let screen_group = match &crt {
            Some(v) => {
                let standard = v.crt.model.monitor().standard;
                if self.crt.as_ref().is_none_or(|set| set.sig.standard != standard) {
                    self.crt = Some(CrtSet::new(&self.device, self.format, standard));
                    self.done.clear();
                    self.done_crt = None;
                    self.have_prev = false;
                }
                self.set_params(v, &placement.screen, rows);
                self.run_signal(encoder, screen, v.crt, v.frame);
                let set = self.crt.as_ref().expect("CRT set");
                let prev = if self.have_prev { self.cur ^ 1 } else { self.cur };
                let tv = wgpu::BindingResource::TextureView;
                let entries = [
                    (0, self.params.as_entire_binding()),
                    (3, tv(&set.lin[self.cur])),
                    (4, tv(&set.lin[prev])),
                    (5, tv(&set.glow[1])),
                    (6, wgpu::BindingResource::Sampler(&self.sampler)),
                ];
                self.bind(&set.crt, &entries)
            }
            None => {
                let tex = self.screen_tex.clone();
                self.write_image(&tex, screen);
                self.bind(&self.blit, &[(7, wgpu::BindingResource::TextureView(&view(&tex)))])
            }
        };
        let bar_group = bar.map(|pixels| {
            let tex = self.bar_tex.clone();
            self.write_image(&tex, pixels);
            self.bind(&self.blit, &[(7, wgpu::BindingResource::TextureView(&view(&tex)))])
        });

        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: None,
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: target,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color::BLACK), store: wgpu::StoreOp::Store },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        // The viewport stays inside the target (in windows smaller than
        // the image, which is then squeezed)
        let viewport = |pass: &mut wgpu::RenderPass, r: &Rect, rows: usize| {
            let (x0, y0) = (r.x.max(0.0), r.y.max(0.0));
            let x1 = (r.x + r.sx * WIDTH as f64).min(dw as f64);
            let y1 = (r.y + r.sy * rows as f64).min(dh as f64);
            pass.set_viewport(x0 as f32, y0 as f32, (x1 - x0).max(1.0) as f32, (y1 - y0).max(1.0) as f32, 0.0, 1.0);
        };
        viewport(&mut pass, &placement.screen, rows);
        match (&crt, &self.crt) {
            (Some(_), Some(set)) => pass.set_pipeline(&set.crt),
            _ => pass.set_pipeline(&self.blit),
        }
        pass.set_bind_group(0, &screen_group, &[]);
        pass.draw(0..3, 0..1);
        if let (Some(group), Some(r)) = (&bar_group, &placement.bar) {
            viewport(&mut pass, r, super::status::BAR_HEIGHT);
            pass.set_pipeline(&self.blit);
            pass.set_bind_group(0, group, &[]);
            pass.draw(0..3, 0..1);
        }
        drop(pass);
        placement
    }
}

// ── Window ───────────────────────────────────────────────────────────────────

/// Drawing into a window.
pub struct WindowRenderer {
    surface: wgpu::Surface<'static>,
    config: wgpu::SurfaceConfiguration,
    /// Format of the views drawn into (without sRGB: the shaders apply the
    /// display gamma themselves, and the plain image is copied as it is).
    format: wgpu::TextureFormat,
    renderer: Renderer,
}

impl WindowRenderer {
    pub fn new(window: Arc<Window>) -> Result<WindowRenderer, String> {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
        let surface = instance.create_surface(window.clone()).map_err(|e| e.to_string())?;
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::default(),
            force_fallback_adapter: false,
            compatible_surface: Some(&surface),
            apply_limit_buckets: false,
        })).map_err(|e| e.to_string())?;
        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor::default()))
            .map_err(|e| e.to_string())?;
        let caps = surface.get_capabilities(&adapter);
        let surface_format = caps.formats.iter().copied().find(|f| !f.is_srgb())
            .or(caps.formats.first().copied())
            .ok_or("no surface format")?;
        let format = surface_format.remove_srgb_suffix();
        // Not tied to the display refresh: the emulator keeps its own time
        let present_mode = [wgpu::PresentMode::Mailbox, wgpu::PresentMode::Immediate]
            .into_iter()
            .find(|m| caps.present_modes.contains(m))
            .unwrap_or(wgpu::PresentMode::Fifo);
        let size = window.inner_size();
        let config = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            format: surface_format,
            width: size.width.max(1),
            height: size.height.max(1),
            present_mode,
            desired_maximum_frame_latency: 2,
            alpha_mode: caps.alpha_modes.first().copied().unwrap_or(wgpu::CompositeAlphaMode::Auto),
            view_formats: if format != surface_format { vec![format] } else { vec![] },
            color_space: wgpu::SurfaceColorSpace::Auto,
        };
        surface.configure(&device, &config);
        Ok(WindowRenderer { surface, config, format, renderer: Renderer::new(device, queue, format) })
    }

    /// Draws in the window of size `size` (physical pixels); None if the
    /// window cannot be drawn now (minimized, being resized).
    pub fn draw(&mut self, size: (u32, u32), screen: &[u32], bar: Option<&[u32]>, crt: Option<CrtView>) -> Option<Placement> {
        let device = &self.renderer.device;
        if (self.config.width, self.config.height) != size {
            self.config.width = size.0;
            self.config.height = size.1;
            self.surface.configure(device, &self.config);
        }
        let frame = match self.surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(t) | wgpu::CurrentSurfaceTexture::Suboptimal(t) => t,
            wgpu::CurrentSurfaceTexture::Outdated | wgpu::CurrentSurfaceTexture::Lost => {
                self.surface.configure(device, &self.config);
                return None;
            }
            _ => return None,
        };
        let target = frame.texture.create_view(&wgpu::TextureViewDescriptor {
            format: Some(self.format),
            ..Default::default()
        });
        let mut encoder = device.create_command_encoder(&Default::default());
        let placement = self.renderer.render(&mut encoder, &target, size.0, size.1, screen, bar, crt);
        self.renderer.queue.submit([encoder.finish()]);
        self.renderer.queue.present(frame);
        Some(placement)
    }
}

// ── Images ───────────────────────────────────────────────────────────────────

/// The C64 screen through the CRT emulation, as an ARGB image `height`
/// pixels high (width from the pixel aspect of the standard), for frame
/// number `frame` (its parity: the NTSC subcarrier alternates). Creates a
/// device of its own: for screenshots, not for every frame.
pub fn crt_image(screen: &[u32], crt: Crt, height: usize, frame: u64) -> Result<(Vec<u32>, usize, usize), String> {
    let standard = crt.model.monitor().standard;
    if screen.len() != standard.fb_len() {
        return Err(format!("the {} does not show this video standard", crt.model.full_name()));
    }
    let rows = standard.fb_height();
    let aspect = crt.model.monitor().signal().pixel_aspect();
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
    let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions::default()))
        .map_err(|e| format!("no GPU: {e}"))?;
    let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor::default()))
        .map_err(|e| e.to_string())?;
    let format = wgpu::TextureFormat::Bgra8Unorm;
    let (w, h) = ((height as f64 * WIDTH as f64 * aspect / rows as f64).round() as usize, height);
    let target = device.create_texture(&wgpu::TextureDescriptor {
        label: None,
        size: wgpu::Extent3d { width: w as u32, height: h as u32, depth_or_array_layers: 1 },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    let row = (4 * w).next_multiple_of(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT as usize);
    let buffer = device.create_buffer(&wgpu::BufferDescriptor {
        label: None,
        size: (row * h) as u64,
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let mut renderer = Renderer::new(device.clone(), queue.clone(), format);
    let mut encoder = device.create_command_encoder(&Default::default());
    renderer.render(&mut encoder, &view(&target), w as u32, h as u32, screen, None,
                    Some(CrtView { crt, blend: false, frame }));
    encoder.copy_texture_to_buffer(
        target.as_image_copy(),
        wgpu::TexelCopyBufferInfo {
            buffer: &buffer,
            layout: wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(row as u32), rows_per_image: None },
        },
        wgpu::Extent3d { width: w as u32, height: h as u32, depth_or_array_layers: 1 },
    );
    queue.submit([encoder.finish()]);
    buffer.map_async(wgpu::MapMode::Read, .., |_| {});
    device.poll(wgpu::PollType::wait_indefinitely()).map_err(|e| e.to_string())?;
    let data = buffer.get_mapped_range(..).map_err(|e| format!("{e:?}"))?;
    let image = (0..h)
        .flat_map(|y| data[y * row..y * row + 4 * w].chunks_exact(4).map(|p| u32::from_le_bytes([p[0], p[1], p[2], 0xFF])).collect::<Vec<_>>())
        .collect();
    Ok((image, w, h))
}
