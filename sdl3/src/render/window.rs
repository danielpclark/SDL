// Rust translation of the window parts of src/render/SDL_render.c
// from Simple DirectMedia Layer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! Renderers for windows.
//!
//! Upstream, the video subsystem holds a pointer to a window's renderer: a
//! window event watcher updates the renderer as the window changes, and
//! destroying the window destroys the renderer's resources. Here the app
//! owns the [`Renderer`], so the watcher records the window's events in a
//! [`WindowLink`] shared with the renderer, which applies them at the start
//! of its next window-dependent call (drawing, presenting, setting state,
//! querying the output size). Once the window is destroyed, the renderer's
//! calls fail with "Renderer's window has been destroyed, can't use further",
//! as upstream's do; dropping it afterwards is fine, so the window and the
//! renderer can go away in either order.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use super::{
    Renderer, RendererCreateInfo, PROP_RENDERER_HDR_ENABLED_BOOLEAN,
    PROP_RENDERER_HDR_HEADROOM_FLOAT, PROP_RENDERER_SDR_WHITE_POINT_FLOAT, RENDER_DRIVERS,
};
use crate::error::{Error, Result};
use crate::events::window::{WindowEventWatchPriority, WindowFlags};
use crate::events::WindowID;
use crate::events::{Event, EventType, EventWatch};
use crate::hints;
use crate::video::blendmode::{BlendFactor as BF, BlendOperation as BO};
use crate::video::pixels::Colorspace;
use crate::video::window::{
    PROP_WINDOW_HDR_HEADROOM_FLOAT, PROP_WINDOW_SDR_WHITE_LEVEL_FLOAT, PROP_WINDOW_SHAPE_POINTER,
};
use crate::video::{BlendMode, Surface, Window};

/// What the window event watcher shares with the renderer.
#[derive(Default)]
struct LinkShared {
    /// The window events not yet applied to the renderer.
    events: Mutex<Vec<EventType>>,
    /// The window was destroyed.
    destroyed: AtomicBool,
    /// The video subsystem quit, which frees its renderers.
    freed: AtomicBool,
}

/// The renderer of each window (`SDL_PROP_WINDOW_RENDERER_POINTER`).
static WINDOW_RENDERERS: Mutex<Option<HashMap<WindowID, Arc<LinkShared>>>> = Mutex::new(None);

fn window_renderers<R>(f: impl FnOnce(&mut HashMap<WindowID, Arc<LinkShared>>) -> R) -> R {
    let mut guard = WINDOW_RENDERERS.lock().unwrap_or_else(|e| e.into_inner());
    f(guard.get_or_insert_with(HashMap::new))
}

/// Whether `window` has a renderer. Translation of `SDL_GetRenderer()`
/// (the renderer itself belongs to the app).
fn window_has_renderer(window: WindowID) -> bool {
    window_renderers(|r| r.contains_key(&window))
}

/// The window being destroyed takes its renderer down with it: the
/// renderer frees its resources the next time it's used. The window part
/// of `SDL_DestroyRendererWithoutFreeing()`, called by
/// `SDL_DestroyWindow()`.
pub(crate) fn destroy_window_renderer(window: WindowID) {
    if let Some(shared) = window_renderers(|r| r.remove(&window)) {
        shared.destroyed.store(true, Ordering::SeqCst);
    }
}

/// Free the window renderers as the video subsystem quits: their calls fail
/// as if the renderer were freed. Translation of `SDL_QuitRender()` (the
/// app owns the software renderers of surfaces, which stay usable).
pub(crate) fn quit_render() {
    let all: Vec<_> = window_renderers(|r| r.drain().map(|(_, s)| s).collect());
    for shared in all {
        shared.freed.store(true, Ordering::SeqCst);
        shared.destroyed.store(true, Ordering::SeqCst);
    }
}

/// The connection between a renderer and its window
/// (`renderer->window` and the renderer's window event watch).
pub(crate) struct WindowLink {
    window: Window,
    shared: Arc<LinkShared>,
    watch: Option<EventWatch>,
}

impl WindowLink {
    pub(crate) fn new(window: Window) -> WindowLink {
        WindowLink {
            window,
            shared: Arc::new(LinkShared::default()),
            watch: None,
        }
    }

    /// Become the window's renderer (`SDL_PROP_WINDOW_RENDERER_POINTER`,
    /// `SDL_AddWindowRenderer()`).
    pub(crate) fn register(&self) {
        window_renderers(|r| r.insert(self.window.id(), self.shared.clone()));
    }

    /// Translation of `SDL_AddWindowEventWatch(SDL_WINDOW_EVENT_WATCH_NORMAL,
    /// SDL_RendererEventWatch, renderer)`.
    pub(crate) fn watch(&mut self) {
        let window_id = self.window.id();
        let shared = Arc::downgrade(&self.shared);
        self.watch = Some(crate::events::window::add_window_event_watch(
            WindowEventWatchPriority::Normal,
            move |event| {
                let Event::Window(w) = event else { return };
                if w.window_id != window_id {
                    return;
                }
                if let Some(shared) = shared.upgrade() {
                    shared
                        .events
                        .lock()
                        .unwrap_or_else(|e| e.into_inner())
                        .push(w.event_type);
                }
            },
        ));
    }

    /// Stop watching the window and stop being its renderer.
    pub(crate) fn unregister(&mut self) {
        // SDL_RemoveWindowEventWatch()
        self.watch = None;

        let window = self.window.id();
        window_renderers(|r| {
            if r.get(&window).is_some_and(|s| Arc::ptr_eq(s, &self.shared)) {
                r.remove(&window);
            }
        });
    }

    /// Whether the window was destroyed.
    pub(crate) fn window_destroyed(&self) -> bool {
        self.shared.destroyed.load(Ordering::SeqCst)
    }

    /// Whether the renderer was freed by `SDL_QuitRender()`.
    fn freed(&self) -> bool {
        self.shared.freed.load(Ordering::SeqCst)
    }

    fn take_events(&self) -> Vec<EventType> {
        std::mem::take(&mut *self.shared.events.lock().unwrap_or_else(|e| e.into_inner()))
    }
}

/// Create a window and a renderer for it. The window is created hidden and
/// shown once the renderer exists (unless `flags` has
/// [`WindowFlags::HIDDEN`]), so the renderer recreating it doesn't flash on
/// screen. Translation of `SDL_CreateWindowAndRenderer()`.
pub fn create_window_and_renderer(
    title: &str,
    width: i32,
    height: i32,
    window_flags: WindowFlags,
) -> Result<(Window, Renderer)> {
    // Hide the window so if the renderer recreates it, we don't get a visual flash on screen
    let hidden = window_flags.contains(WindowFlags::HIDDEN);
    let window = Window::create(title, width, height, window_flags | WindowFlags::HIDDEN)?;

    let renderer = match Renderer::for_window(&window, None) {
        Ok(renderer) => renderer,
        Err(e) => {
            window.destroy();
            return Err(e);
        }
    };

    if !hidden {
        let _ = window.show();
    }

    Ok((window, renderer))
}

impl Renderer {
    /// A renderer for `window`, using the named rendering driver (a comma
    /// separated list to try in order) or the first one that works. Without
    /// a name, the [`hints::RENDER_DRIVER`] hint is used if set.
    /// Translation of `SDL_CreateRenderer()`.
    pub fn for_window(window: &Window, name: Option<&str>) -> Result<Renderer> {
        Renderer::for_window_with(
            window,
            &RendererCreateInfo {
                name: name.map(str::to_owned),
                ..RendererCreateInfo::default()
            },
        )
    }

    /// A renderer for `window`, with creation options. Translation of
    /// `SDL_CreateRendererWithProperties()` with
    /// `SDL_PROP_RENDERER_CREATE_WINDOW_POINTER`.
    pub fn for_window_with(window: &Window, info: &RendererCreateInfo) -> Result<Renderer> {
        if window.has_surface()? {
            return Err(Error::new("Surface already associated with window"));
        }

        if window_has_renderer(window.id()) {
            return Err(Error::new("Renderer already associated with window"));
        }

        let present_vsync = super::present_vsync_with_hint(info);

        let mut driver_name = info.name.clone();
        if driver_name.is_none() {
            driver_name = hints::get(hints::RENDER_DRIVER);
        }

        let mut created = None;
        let mut driver_error: Option<Error> = None;
        match driver_name.as_deref().filter(|n| !n.is_empty()) {
            Some(names) => {
                for attempt in names.split(',') {
                    if created.is_some() || attempt.is_empty() {
                        break;
                    }
                    for &driver in RENDER_DRIVERS {
                        if driver.eq_ignore_ascii_case(attempt) {
                            // Free any previous driver error
                            driver_error = None;

                            match create_for_window(driver, *window, present_vsync != 0, info) {
                                Ok(r) => {
                                    created = Some(r);
                                    break;
                                }
                                Err(e) => {
                                    crate::log::warn!(
                                        crate::log::Category::Render,
                                        "Couldn't create renderer {}: {}\n",
                                        driver,
                                        e
                                    );
                                    driver_error = Some(e);
                                }
                            }
                        }
                    }
                }
            }
            None => {
                for &driver in RENDER_DRIVERS {
                    if let Ok(r) = create_for_window(driver, *window, present_vsync != 0, info) {
                        created = Some(r);
                        break;
                    }
                }
            }
        }

        let Some((backend, format)) = created else {
            return Err(match (driver_name, driver_error) {
                (Some(_), Some(e)) => e,
                (Some(name), None) => Error::new(format!("{name} not available")),
                (None, _) => Error::new("Couldn't find matching render driver"),
            });
        };

        Renderer::finish_create(backend, format, (0, 0), info, present_vsync, Some(*window))
    }

    /// The window the renderer draws into, if any. Translation of
    /// `SDL_GetRenderWindow()`.
    pub fn window(&self) -> Option<Window> {
        if self.destroyed || self.window.as_ref().is_some_and(|l| l.window_destroyed()) {
            return None;
        }
        self.window_handle()
    }

    pub(crate) fn window_handle(&self) -> Option<Window> {
        self.window.as_ref().map(|link| link.window)
    }

    /// Apply the window events since the last call, or fail if the window
    /// is gone (`CHECK_RENDERER_MAGIC()` on a renderer destroyed with its
    /// window).
    pub(crate) fn sync_window(&mut self) -> Result<()> {
        let Some(link) = &self.window else {
            return Ok(());
        };
        if link.window_destroyed() && !self.destroyed {
            let freed = link.freed();
            self.destroy_without_freeing();
            if freed {
                self.freed = true;
            }
        }
        if self.freed {
            return Err(Error::invalid_param("renderer"));
        }
        if self.destroyed {
            return Err(Error::new(
                "Renderer's window has been destroyed, can't use further",
            ));
        }
        let events = self
            .window
            .as_ref()
            .map(WindowLink::take_events)
            .unwrap_or_default();
        for event_type in events {
            self.renderer_event(event_type);
        }
        Ok(())
    }

    /// Translation of `SDL_RendererEventWatch()`.
    fn renderer_event(&mut self, event_type: EventType) {
        let Some(window) = self.window_handle() else {
            return;
        };

        self.backend.window_event(event_type);

        if event_type == EventType::WINDOW_RESIZED
            || event_type == EventType::WINDOW_PIXEL_SIZE_CHANGED
            || event_type == EventType::WINDOW_METAL_VIEW_RESIZED
        {
            self.force_main_view = true; // only update the main_view (the window framebuffer) for window changes.
            self.update_logical_presentation();
            self.force_main_view = false; // put us back on whatever the current render target's actual view is.
        } else if event_type == EventType::WINDOW_HIDDEN {
            self.hidden = true;
        } else if event_type == EventType::WINDOW_SHOWN {
            if !window
                .flags()
                .unwrap_or_default()
                .contains(WindowFlags::MINIMIZED)
            {
                self.hidden = false;
            }
        } else if event_type == EventType::WINDOW_MINIMIZED {
            self.hidden = true;
        } else if event_type == EventType::WINDOW_RESTORED
            || event_type == EventType::WINDOW_MAXIMIZED
        {
            if !window
                .flags()
                .unwrap_or_default()
                .contains(WindowFlags::HIDDEN)
            {
                self.hidden = false;
            }
        } else if event_type == EventType::WINDOW_DISPLAY_CHANGED
            || event_type == EventType::WINDOW_HDR_STATE_CHANGED
        {
            self.update_hdr_properties();
        }
    }

    /// Translation of `UpdateHDRProperties()`.
    pub(crate) fn update_hdr_properties(&mut self) {
        let Some(window_props) = self.window_handle().and_then(|w| w.properties().ok()) else {
            return;
        };
        let renderer_props = self.props.clone();

        if self.output_colorspace == Colorspace::SRGB_LINEAR
            || self.output_colorspace == Colorspace::HDR10
        {
            self.sdr_white_point = window_props
                .get_float(PROP_WINDOW_SDR_WHITE_LEVEL_FLOAT)
                .unwrap_or(1.0);
            self.hdr_headroom = window_props
                .get_float(PROP_WINDOW_HDR_HEADROOM_FLOAT)
                .unwrap_or(1.0);
        } else {
            self.sdr_white_point = 1.0;
            self.hdr_headroom = 1.0;
        }

        let _ = renderer_props.set(PROP_RENDERER_HDR_ENABLED_BOOLEAN, self.hdr_headroom > 1.0);
        let _ = renderer_props.set(PROP_RENDERER_SDR_WHITE_POINT_FLOAT, self.sdr_white_point);
        let _ = renderer_props.set(PROP_RENDERER_HDR_HEADROOM_FLOAT, self.hdr_headroom);

        self.update_color_scale();
    }

    /// Translation of `SDL_RenderApplyWindowShape()`.
    pub(crate) fn apply_window_shape(&mut self) {
        let shape = self
            .window_handle()
            .and_then(|w| w.properties().ok())
            .and_then(|p| p.get_any::<Surface<'static>>(PROP_WINDOW_SHAPE_POINTER));
        let same = match (&shape, &self.shape_surface) {
            (Some(a), Some(b)) => Arc::ptr_eq(a, b),
            (None, None) => true,
            _ => false,
        };
        if !same {
            if let Some(texture) = self.shape_texture.take() {
                self.destroy_texture(texture);
            }

            if let Some(shape) = &shape {
                // There's nothing we can do if this fails, so just keep on going
                self.shape_texture = shape
                    .duplicate()
                    .and_then(|mut s| self.create_texture_from_surface(&mut s))
                    .ok();

                if let Some(texture) = self.shape_texture {
                    let _ = self.set_texture_blend_mode(
                        texture,
                        BlendMode::compose_custom(
                            BF::Zero,
                            BF::SrcAlpha,
                            BO::Add,
                            BF::Zero,
                            BF::SrcAlpha,
                            BO::Add,
                        ),
                    );
                }
            }
            self.shape_surface = shape;
        }

        if let Some(texture) = self.shape_texture {
            let _ = self.render_texture(texture, None, None);
        }
    }

    /// Convert the coordinates in an event from window coordinates to
    /// render coordinates (events for other windows are left alone).
    /// Translation of `SDL_ConvertEventToRenderCoordinates()`.
    pub fn convert_event_to_render_coordinates(&mut self, event: &mut Event) -> Result<()> {
        self.sync_window()?;
        let window = self.window_handle().map(|w| w.id());
        let ours = |id: WindowID| window.is_some() && window_from_id(id) == window;

        match event {
            Event::MouseMotion(m) => {
                if ours(m.window_id) {
                    (m.x, m.y) = self.coordinates_from_window(m.x, m.y);
                    (m.xrel, m.yrel) = self.render_vector_from_window(m.xrel, m.yrel);
                }
            }
            Event::MouseButton(b) => {
                if ours(b.window_id) {
                    (b.x, b.y) = self.coordinates_from_window(b.x, b.y);
                }
            }
            Event::MouseWheel(w) => {
                if ours(w.window_id) {
                    (w.mouse_x, w.mouse_y) = self.coordinates_from_window(w.mouse_x, w.mouse_y);
                }
            }
            Event::TouchFinger(f)
                if f.event_type == EventType::FINGER_DOWN
                    || f.event_type == EventType::FINGER_UP
                    || f.event_type == EventType::FINGER_CANCELED
                    || f.event_type == EventType::FINGER_MOTION =>
            {
                // FIXME: Are these events guaranteed to be window relative?
                if let Some(window) = self.window_handle() {
                    let (w, h) = window.size()?;
                    let (w, h) = (w as f32, h as f32);
                    (f.x, f.y) = self.coordinates_from_window(f.x * w, f.y * h);
                    (f.dx, f.dy) = self.render_vector_from_window(f.dx * w, f.dy * h);
                }
            }
            Event::PenMotion(p) => {
                if ours(p.window_id) {
                    (p.x, p.y) = self.coordinates_from_window(p.x, p.y);
                }
            }
            Event::PenTouch(p) => {
                if ours(p.window_id) {
                    (p.x, p.y) = self.coordinates_from_window(p.x, p.y);
                }
            }
            Event::PenButton(p) => {
                if ours(p.window_id) {
                    (p.x, p.y) = self.coordinates_from_window(p.x, p.y);
                }
            }
            Event::PenAxis(p) => {
                if ours(p.window_id) {
                    (p.x, p.y) = self.coordinates_from_window(p.x, p.y);
                }
            }
            Event::Drop(d)
                if (d.event_type == EventType::DROP_POSITION
                    || d.event_type == EventType::DROP_FILE
                    || d.event_type == EventType::DROP_TEXT
                    || d.event_type == EventType::DROP_COMPLETE)
                    && ours(d.window_id) =>
            {
                (d.x, d.y) = self.coordinates_from_window(d.x, d.y);
            }
            _ => {}
        }
        Ok(())
    }
}

/// `SDL_GetWindowFromID()`: the window's ID if it exists.
fn window_from_id(id: WindowID) -> Option<WindowID> {
    Window::from_id(id).ok().map(|w| w.id())
}

/// `driver->CreateRenderer(renderer, window, props)` for the drivers
/// compiled in.
fn create_for_window(
    driver: &str,
    window: Window,
    present_vsync: bool,
    info: &RendererCreateInfo,
) -> Result<(Box<dyn super::RenderBackend>, crate::video::PixelFormat)> {
    match driver {
        super::opengl::OPENGL_RENDERER => {
            let output_colorspace = info.output_colorspace.unwrap_or(Colorspace::SRGB);
            let backend = super::opengl::GlRenderer::for_window(window, output_colorspace)?;
            Ok((Box::new(backend), crate::video::PixelFormat::UNKNOWN))
        }
        super::opengles2::GLES2_RENDERER => {
            let output_colorspace = info.output_colorspace.unwrap_or(Colorspace::SRGB);
            let backend = super::opengles2::Gles2Renderer::for_window(window, output_colorspace)?;
            Ok((Box::new(backend), crate::video::PixelFormat::UNKNOWN))
        }
        super::vulkan::VULKAN_RENDERER => {
            let output_colorspace = info.output_colorspace.unwrap_or(Colorspace::SRGB);
            let backend = super::vulkan::VulkanRenderer::for_window(window, output_colorspace)?;
            Ok((Box::new(backend), crate::video::PixelFormat::UNKNOWN))
        }
        super::SOFTWARE_RENDERER => {
            let (backend, format) = super::SwRenderer::for_window(window, present_vsync)?;
            Ok((Box::new(backend), format))
        }
        _ => Err(Error::new(format!("{driver} not available"))),
    }
}
