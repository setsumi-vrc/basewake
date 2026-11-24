//! The "Basewake" panel in the SteamVR dashboard: the same egui panel as the desktop window,
//! rendered off-screen with OpenGL into a texture and handed to the dashboard overlay.

use std::ffi::{CString, c_void};
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};

use anyhow::{Context as _, Result, anyhow, bail};
use eframe::egui;
use glow::HasContext;
use windows::Win32::Foundation::{HMODULE, HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::Graphics::Gdi::{GetDC, HDC, ReleaseDC};
use windows::Win32::Graphics::OpenGL::{
    ChoosePixelFormat, HGLRC, PFD_DOUBLEBUFFER, PFD_DRAW_TO_WINDOW, PFD_MAIN_PLANE, PFD_SUPPORT_OPENGL,
    PFD_TYPE_RGBA, PIXELFORMATDESCRIPTOR, SetPixelFormat, wglCreateContext, wglDeleteContext, wglGetProcAddress,
    wglMakeCurrent,
};
use windows::Win32::System::LibraryLoader::{GetModuleHandleW, GetProcAddress, LoadLibraryW};
use windows::Win32::UI::WindowsAndMessaging::{
    CS_OWNDC, CreateWindowExW, DefWindowProcW, DestroyWindow, RegisterClassW, WINDOW_EX_STYLE, WNDCLASSW,
    WS_OVERLAPPEDWINDOW,
};
use windows::core::{PCSTR, w};

use super::openvr::{self, OpenVr, OverlayHandle};
use crate::app::AppCtl;
use crate::ui::{self, PanelState};

/// Texture size in pixels and UI scale. 1024×1152 at 1.6 → 640×720 points, like the window.
const WIDTH: u32 = 1024;
const HEIGHT: u32 = 1152;
const PIXELS_PER_POINT: f32 = 1.6;
/// Fastest repaint while the pointer is moving (hover effects, spinners).
const MIN_FRAME: Duration = Duration::from_millis(33);

pub struct Dashboard {
    main: OverlayHandle,
    thumb: OverlayHandle,
    gl: GlContext,
    painter: egui_glow::Painter,
    fbo: glow::Framebuffer,
    texture: glow::Texture,
    egui: egui::Context,
    panel: PanelState,
    events: Vec<egui::Event>,
    pointer: egui::Pos2,
    start: Instant,
    last_paint: Option<Instant>,
    next_paint: Option<Instant>,
    visible: bool,
    thumb_tone: Option<ui::icons::Tone>,
}

impl Dashboard {
    pub fn new(vr: &OpenVr) -> Result<Dashboard> {
        let (main, thumb) = vr.create_dashboard_overlay("local.basewake.dashboard", "Basewake")?;
        vr.set_width(main, 1.6)?;
        vr.set_mouse_input(main, WIDTH as f32, HEIGHT as f32)?;
        let _ = vr.set_flag(main, openvr::FLAG_SEND_DISCRETE_SCROLL, true);

        let gl = GlContext::new().context("OpenGL context")?;
        let (fbo, texture) = unsafe { create_target(&gl.gl)? };
        let painter = egui_glow::Painter::new(gl.gl.clone(), "", None, false)
            .map_err(|e| anyhow!("egui painter: {e}"))?;
        let egui = egui::Context::default();
        ui::style(&egui);

        let mut d = Dashboard {
            main,
            thumb,
            gl,
            painter,
            fbo,
            texture,
            egui,
            panel: PanelState::default(),
            events: Vec::new(),
            pointer: egui::Pos2::ZERO,
            start: Instant::now(),
            last_paint: None,
            next_paint: Some(Instant::now()),
            visible: false,
            thumb_tone: None,
        };
        d.update_thumbnail(vr, ui::icons::Tone::Off);
        Ok(d)
    }

    pub fn visible(&self) -> bool {
        self.visible
    }

    /// Handles overlay input and repaints if needed. Call often (every ~16-100 ms).
    pub fn tick(&mut self, vr: &OpenVr, ctl: &Arc<AppCtl>) {
        while let Some(ev) = vr.poll_overlay_event(self.main) {
            self.on_event(ev);
        }
        let visible = vr.is_visible(self.main);
        if visible && !self.visible {
            self.next_paint = Some(Instant::now());
        }
        self.visible = visible;

        let tone = ui::overall_tone(&ctl.handle.snapshot());
        if self.thumb_tone != Some(tone) {
            self.update_thumbnail(vr, tone);
        }
        if !visible {
            return;
        }
        if ctl.vr_dirty.swap(false, Ordering::AcqRel) || !self.events.is_empty() {
            self.next_paint = Some(Instant::now());
        }
        let now = Instant::now();
        let due = self.next_paint.is_some_and(|t| t <= now);
        let throttled = self.last_paint.is_some_and(|t| now - t < MIN_FRAME);
        if due && !throttled
            && let Err(e) = self.paint(vr, ctl) {
                tracing::warn!("dashboard paint failed: {e:#}");
                self.next_paint = Some(now + Duration::from_secs(1));
            }
    }

    fn on_event(&mut self, ev: openvr::VrEvent) {
        let to_ui = |x: f32, y: f32| {
            // Overlay mouse coordinates are GL-style: (0,0) is the bottom-left corner.
            egui::pos2(x / PIXELS_PER_POINT, (HEIGHT as f32 - y) / PIXELS_PER_POINT)
        };
        match ev.event_type {
            openvr::EVENT_MOUSE_MOVE => {
                let (x, y, _) = ev.mouse();
                self.pointer = to_ui(x, y);
                self.events.push(egui::Event::PointerMoved(self.pointer));
            }
            openvr::EVENT_MOUSE_DOWN | openvr::EVENT_MOUSE_UP => {
                let (x, y, button) = ev.mouse();
                if button == openvr::MOUSE_LEFT {
                    self.pointer = to_ui(x, y);
                    self.events.push(egui::Event::PointerButton {
                        pos: self.pointer,
                        button: egui::PointerButton::Primary,
                        pressed: ev.event_type == openvr::EVENT_MOUSE_DOWN,
                        modifiers: egui::Modifiers::default(),
                    });
                }
            }
            openvr::EVENT_SCROLL_DISCRETE | openvr::EVENT_SCROLL_SMOOTH => {
                let (dx, dy) = ev.scroll();
                self.events.push(egui::Event::MouseWheel {
                    unit: egui::MouseWheelUnit::Line,
                    delta: egui::vec2(dx, dy),
                    phase: egui::TouchPhase::Move,
                    source: egui::MouseWheelSource::Unknown,
                    modifiers: egui::Modifiers::default(),
                });
            }
            openvr::EVENT_OVERLAY_HIDDEN => self.events.push(egui::Event::PointerGone),
            _ => {}
        }
    }

    fn paint(&mut self, vr: &OpenVr, ctl: &Arc<AppCtl>) -> Result<()> {
        self.gl.make_current()?;
        let size = egui::vec2(WIDTH as f32, HEIGHT as f32) / PIXELS_PER_POINT;
        let mut raw = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, size)),
            time: Some(self.start.elapsed().as_secs_f64()),
            events: std::mem::take(&mut self.events),
            focused: true,
            max_texture_side: Some(4096),
            ..Default::default()
        };
        raw.viewports.entry(egui::ViewportId::ROOT).or_default().native_pixels_per_point = Some(PIXELS_PER_POINT);

        let panel = &mut self.panel;
        let mut out = self.egui.run_ui(raw, |ui| {
            egui::CentralPanel::default().frame(egui::Frame::new().fill(ui::BG)).show(ui, |ui| {
                ui::panel::draw(ui, ctl, panel);
            });
        });
        let prims = self.egui.tessellate(std::mem::take(&mut out.shapes), out.pixels_per_point);

        unsafe {
            let gl = &self.gl.gl;
            gl.bind_framebuffer(glow::FRAMEBUFFER, Some(self.fbo));
            gl.viewport(0, 0, WIDTH as i32, HEIGHT as i32);
            gl.clear_color(0.08, 0.09, 0.1, 1.0);
            gl.clear(glow::COLOR_BUFFER_BIT);
        }
        self.painter.paint_and_update_textures([WIDTH, HEIGHT], out.pixels_per_point, &prims, &mut out.textures_delta);
        unsafe {
            self.gl.gl.bind_framebuffer(glow::FRAMEBUFFER, Some(self.fbo));
            self.gl.gl.finish();
        }
        vr.set_gl_texture(self.main, self.texture.0.get())?;

        let now = Instant::now();
        self.last_paint = Some(now);
        let delay = out.viewport_output.get(&egui::ViewportId::ROOT).map(|v| v.repaint_delay).unwrap_or(Duration::MAX);
        self.next_paint = (delay < Duration::from_secs(3600)).then(|| now + delay.max(MIN_FRAME));
        Ok(())
    }

    fn update_thumbnail(&mut self, vr: &OpenVr, tone: ui::icons::Tone) {
        let px = ui::icons::rgba(tone, 128);
        match vr.set_raw(self.thumb, &px, 128, 128) {
            Ok(()) => self.thumb_tone = Some(tone),
            Err(e) => tracing::warn!("thumbnail: {e:#}"),
        }
    }

    pub fn destroy(mut self, vr: &OpenVr) {
        let _ = self.gl.make_current();
        self.painter.destroy();
        unsafe {
            self.gl.gl.delete_framebuffer(self.fbo);
            self.gl.gl.delete_texture(self.texture);
        }
        vr.destroy_overlay(self.main);
    }
}

/// RGBA8 texture + framebuffer to render the panel into.
unsafe fn create_target(gl: &glow::Context) -> Result<(glow::Framebuffer, glow::Texture)> {
    unsafe {
        let tex = gl.create_texture().map_err(|e| anyhow!(e))?;
        gl.bind_texture(glow::TEXTURE_2D, Some(tex));
        gl.tex_image_2d(
            glow::TEXTURE_2D,
            0,
            glow::RGBA8 as i32,
            WIDTH as i32,
            HEIGHT as i32,
            0,
            glow::RGBA,
            glow::UNSIGNED_BYTE,
            glow::PixelUnpackData::Slice(None),
        );
        gl.tex_parameter_i32(glow::TEXTURE_2D, glow::TEXTURE_MIN_FILTER, glow::LINEAR as i32);
        gl.tex_parameter_i32(glow::TEXTURE_2D, glow::TEXTURE_MAG_FILTER, glow::LINEAR as i32);
        let fbo = gl.create_framebuffer().map_err(|e| anyhow!(e))?;
        gl.bind_framebuffer(glow::FRAMEBUFFER, Some(fbo));
        gl.framebuffer_texture_2d(glow::FRAMEBUFFER, glow::COLOR_ATTACHMENT0, glow::TEXTURE_2D, Some(tex), 0);
        let status = gl.check_framebuffer_status(glow::FRAMEBUFFER);
        if status != glow::FRAMEBUFFER_COMPLETE {
            bail!("framebuffer incomplete (0x{status:X})");
        }
        Ok((fbo, tex))
    }
}

/// An OpenGL context on a hidden window, current on the SteamVR thread.
struct GlContext {
    hwnd: HWND,
    hdc: HDC,
    hglrc: HGLRC,
    gl: Arc<glow::Context>,
}

impl GlContext {
    fn new() -> Result<GlContext> {
        unsafe extern "system" fn wndproc(h: HWND, m: u32, w: WPARAM, l: LPARAM) -> LRESULT {
            unsafe { DefWindowProcW(h, m, w, l) }
        }
        unsafe {
            let hinstance = GetModuleHandleW(None)?;
            let class = w!("BasewakeGL");
            let wc = WNDCLASSW {
                style: CS_OWNDC,
                lpfnWndProc: Some(wndproc),
                hInstance: hinstance.into(),
                lpszClassName: class,
                ..Default::default()
            };
            RegisterClassW(&wc); // fails harmlessly if already registered
            let hwnd = CreateWindowExW(
                WINDOW_EX_STYLE::default(),
                class,
                w!("Basewake GL"),
                WS_OVERLAPPEDWINDOW,
                0,
                0,
                16,
                16,
                None,
                None,
                Some(hinstance.into()),
                None,
            )?;
            let hdc = GetDC(Some(hwnd));
            let pfd = PIXELFORMATDESCRIPTOR {
                nSize: std::mem::size_of::<PIXELFORMATDESCRIPTOR>() as u16,
                nVersion: 1,
                dwFlags: PFD_DRAW_TO_WINDOW | PFD_SUPPORT_OPENGL | PFD_DOUBLEBUFFER,
                iPixelType: PFD_TYPE_RGBA,
                cColorBits: 32,
                cAlphaBits: 8,
                cDepthBits: 24,
                cStencilBits: 8,
                iLayerType: PFD_MAIN_PLANE.0 as u8,
                ..Default::default()
            };
            let format = ChoosePixelFormat(hdc, &pfd);
            if format == 0 {
                bail!("no OpenGL pixel format");
            }
            SetPixelFormat(hdc, format, &pfd)?;
            let hglrc = wglCreateContext(hdc)?;
            wglMakeCurrent(hdc, hglrc)?;

            let opengl32: HMODULE = LoadLibraryW(w!("opengl32.dll"))?;
            let gl = glow::Context::from_loader_function(|name| {
                let c = CString::new(name).unwrap();
                let p = wglGetProcAddress(PCSTR(c.as_ptr().cast()));
                let addr = p.map(|f| f as usize).unwrap_or(0);
                // wglGetProcAddress returns small sentinel values for core 1.1 functions.
                if addr > 3 && addr != usize::MAX {
                    return addr as *const c_void;
                }
                GetProcAddress(opengl32, PCSTR(c.as_ptr().cast())).map(|f| f as usize).unwrap_or(0) as *const c_void
            });
            Ok(GlContext { hwnd, hdc, hglrc, gl: Arc::new(gl) })
        }
    }

    fn make_current(&self) -> Result<()> {
        unsafe { wglMakeCurrent(self.hdc, self.hglrc).map_err(Into::into) }
    }
}

impl Drop for GlContext {
    fn drop(&mut self) {
        unsafe {
            let _ = wglMakeCurrent(HDC::default(), HGLRC::default());
            let _ = wglDeleteContext(self.hglrc);
            ReleaseDC(Some(self.hwnd), self.hdc);
            let _ = DestroyWindow(self.hwnd);
        }
    }
}
