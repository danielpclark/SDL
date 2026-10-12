// Tests of the Cocoa video driver.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! The Cocoa video driver only starts on the main thread (as upstream's
//! `Cocoa_CreateDevice()`), and the test harness runs tests on other
//! threads, so this is a program of its own (`harness = false`) whose
//! `main` drives the driver through the public API: displays and modes,
//! windows (geometry, visibility, flags, fullscreen, popups), text input,
//! the clipboard, cursors, the keymap, mouse warping and message boxes;
//! OpenGL contexts (NSOpenGL: legacy and core profiles, sharing, swap
//! intervals) drawn and read back, the opengl 2D renderer against the
//! software renderer, window surfaces through a renderer texture, Metal views and their layers' drawable
//! sizes, Vulkan surfaces through MoltenVK (capability `vulkan`: the
//! loader by its default names, or Homebrew's), and OpenGL ES through EGL
//! with ANGLE
//! (capability `egl`: `SDL3_TEST_ANGLE_DIR`, or the copy the Android
//! emulator ships).
//!
//! Without a window server session (capability `cocoa`) it skips, unless
//! `SDL3_TEST_REQUIRE` asks for `cocoa` (or `all`).

#[cfg(not(target_os = "macos"))]
fn main() {}

#[cfg(target_os = "macos")]
fn main() {
    macos::main();
}

#[cfg(target_os = "macos")]
mod macos {
    use std::ffi::{c_char, c_void};
    use std::time::{Duration, Instant};

    use sdl3::events::keyboard::{key_from_scancode, Keycode, Keymod, Scancode};
    use sdl3::events::window::WindowFlags;
    use sdl3::events::{Event, EventType};
    use sdl3::hints;
    use sdl3::init::{self, InitFlags};
    use sdl3::video::messagebox::{
        show_message_box, MessageBoxButtonData, MessageBoxButtonFlags, MessageBoxData,
        MessageBoxFlags,
    };
    use sdl3::video::{self, PixelFormat, Surface, Window};

    /// Whether `SDL3_TEST_REQUIRE` makes the `cocoa` capability mandatory.
    fn required() -> bool {
        std::env::var("SDL3_TEST_REQUIRE")
            .unwrap_or_default()
            .split(',')
            .map(str::trim)
            .any(|c| c == "all" || c == "cocoa")
    }

    fn skip(reason: &str) {
        if required() {
            panic!("required capability cocoa unavailable: {reason}");
        }
        println!("note: skipping, cocoa unavailable: {reason}");
    }

    /// Pump events for a while, returning what came in.
    fn pump_for(ms: u64) -> Vec<Event> {
        let end = Instant::now() + Duration::from_millis(ms);
        let mut events = Vec::new();
        while Instant::now() < end {
            while let Some(e) = sdl3::events::poll() {
                events.push(e);
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        events
    }

    fn has_window_event(events: &[Event], window: &Window, ty: EventType) -> bool {
        events.iter().any(|e| match e {
            Event::Window(w) => w.window_id == window.id() && w.event_type == ty,
            _ => false,
        })
    }

    // The bits of the Objective-C runtime and GCD the message box check
    // needs to answer the alert from outside.
    unsafe extern "C" {
        fn objc_getClass(name: *const c_char) -> *mut c_void;
        fn sel_registerName(name: *const c_char) -> *mut c_void;
        fn objc_msgSend();
        fn object_getClassName(obj: *mut c_void) -> *const c_char;
        static _dispatch_main_q: c_void;
        fn dispatch_async_f(
            queue: *const c_void,
            context: *mut c_void,
            work: unsafe extern "C" fn(*mut c_void),
        );
    }

    /// `[NSApp stopModalWithCode:1001]` (the second button), run by the main
    /// queue inside the alert's modal loop.
    unsafe extern "C" fn stop_modal_with_second_button(_ctx: *mut c_void) {
        // SAFETY: NSApplication's class method and instance method, with
        // their signatures.
        unsafe {
            let send0: unsafe extern "C" fn(*mut c_void, *mut c_void) -> *mut c_void =
                std::mem::transmute(objc_msgSend as unsafe extern "C" fn());
            let send1: unsafe extern "C" fn(*mut c_void, *mut c_void, isize) =
                std::mem::transmute(objc_msgSend as unsafe extern "C" fn());
            let app = send0(
                objc_getClass(c"NSApplication".as_ptr()),
                sel_registerName(c"sharedApplication".as_ptr()),
            );
            send1(app, sel_registerName(c"stopModalWithCode:".as_ptr()), 1001);
        }
    }

    pub(super) fn main() {
        // A hung transition shouldn't hang the test run.
        std::thread::spawn(|| {
            std::thread::sleep(Duration::from_secs(240));
            eprintln!("cocoa test timed out");
            std::process::exit(1);
        });

        hints::set(hints::VIDEO_DRIVER, "cocoa").unwrap();
        if let Err(e) = init::init(InitFlags::VIDEO) {
            skip(e.message());
            return;
        }
        assert_eq!(video::current_video_driver().unwrap(), "cocoa");
        println!("cocoa: video initialized");

        displays_and_modes();
        windows();
        popups();
        fullscreen();
        text_input();
        clipboard();
        cursors();
        keymap();
        mouse_warp();
        message_box();
        opengl();
        renderer();
        window_surface();
        metal();
        vulkan();
        opengl_es();

        init::quit();
        println!("cocoa: all checks passed");
    }

    // The GL functions the checks call (through SDL_GL_GetProcAddress()).
    type GlGetString = unsafe extern "C" fn(u32) -> *const c_char;
    type GlClearColor = unsafe extern "C" fn(f32, f32, f32, f32);
    type GlClear = unsafe extern "C" fn(u32);
    type GlReadPixels = unsafe extern "C" fn(i32, i32, i32, i32, u32, u32, *mut c_void);
    type GlFinish = unsafe extern "C" fn();
    type GlGenTextures = unsafe extern "C" fn(i32, *mut u32);
    type GlIsTexture = unsafe extern "C" fn(u32) -> u8;
    type GlBindTexture = unsafe extern "C" fn(u32, u32);
    type GlGetError = unsafe extern "C" fn() -> u32;
    const GL_VERSION: u32 = 0x1F02;
    const GL_RENDERER: u32 = 0x1F01;
    const GL_COLOR_BUFFER_BIT: u32 = 0x4000;
    const GL_RGBA: u32 = 0x1908;
    const GL_UNSIGNED_BYTE: u32 = 0x1401;
    const GL_TEXTURE_2D: u32 = 0x0DE1;

    /// A GL function of the current library.
    fn glfn<F: Copy>(name: &str) -> F {
        let p = video::gl::gl_get_proc_address(name).unwrap_or_else(|| panic!("no {name}"));
        assert_eq!(size_of::<F>(), size_of::<*const c_void>());
        // SAFETY: the caller names F as the function's type.
        unsafe { std::mem::transmute_copy(&p) }
    }

    fn gl_string(name: u32) -> String {
        let get_string: GlGetString = glfn("glGetString");
        // SAFETY: a current context; glGetString returns a static string.
        let s = unsafe { get_string(name) };
        assert!(!s.is_null());
        // SAFETY: a NUL-terminated string.
        unsafe { std::ffi::CStr::from_ptr(s) }
            .to_string_lossy()
            .into_owned()
    }

    /// Clear the current context's back buffer to a color and read the
    /// middle pixel back.
    fn gl_clear_and_read(window: &Window, rgba: [f32; 4]) -> [u8; 4] {
        let clear_color: GlClearColor = glfn("glClearColor");
        let clear: GlClear = glfn("glClear");
        let read_pixels: GlReadPixels = glfn("glReadPixels");
        let finish: GlFinish = glfn("glFinish");
        let (w, h) = window.size_in_pixels().unwrap();
        let mut px = [0u8; 4];
        // SAFETY: a current context; the buffer has room for one pixel.
        unsafe {
            clear_color(rgba[0], rgba[1], rgba[2], rgba[3]);
            clear(GL_COLOR_BUFFER_BIT);
            finish();
            read_pixels(
                w / 2,
                h / 2,
                1,
                1,
                GL_RGBA,
                GL_UNSIGNED_BYTE,
                px.as_mut_ptr().cast(),
            );
        }
        px
    }

    fn opengl() {
        use video::gl::{self, GlAttr};

        // The default (legacy) profile
        gl::gl_reset_attributes();
        let window = Window::create("SDL Cocoa GL", 96, 64, WindowFlags::OPENGL).unwrap();
        assert!(window.flags().unwrap().contains(WindowFlags::OPENGL));
        pump_for(200);
        let ctx = gl::gl_create_context(&window).unwrap();
        assert!(ctx.is_current());
        assert_eq!(
            gl::gl_current_window().unwrap().map(|w| w.id()),
            Some(window.id())
        );
        let version = gl_string(GL_VERSION);
        println!("cocoa: GL {version} on {}", gl_string(GL_RENDERER));
        assert!(version.starts_with("2.1"), "{version}");
        assert_eq!(
            gl_clear_and_read(&window, [1.0, 0.0, 0.0, 1.0]),
            [255, 0, 0, 255]
        );
        assert_eq!(
            gl_clear_and_read(&window, [0.0, 0.5, 1.0, 1.0])[..3],
            [0, 128, 255]
        );

        // Swap intervals, vsync through the display link
        gl::gl_set_swap_interval(0).unwrap();
        assert_eq!(gl::gl_get_swap_interval().unwrap(), 0);
        let start = Instant::now();
        for _ in 0..5 {
            gl::gl_swap_window(&window).unwrap();
        }
        let unsynced = start.elapsed();
        gl::gl_set_swap_interval(1).unwrap();
        assert_eq!(gl::gl_get_swap_interval().unwrap(), 1);
        let start = Instant::now();
        for _ in 0..5 {
            gl::gl_swap_window(&window).unwrap();
        }
        let synced = start.elapsed();
        println!("cocoa: 5 swaps in {unsynced:?} unsynced, {synced:?} with vsync");
        // Five waits for a display refresh (at most 240 Hz).
        assert!(synced >= Duration::from_millis(15), "{synced:?}");
        assert!(synced < Duration::from_secs(2), "{synced:?}");
        gl::gl_set_swap_interval(-1).unwrap();
        assert_eq!(gl::gl_get_swap_interval().unwrap(), -1);
        gl::gl_swap_window(&window).unwrap();
        gl::gl_set_swap_interval(0).unwrap();

        // A context sharing objects with the current one
        let gen_textures: GlGenTextures = glfn("glGenTextures");
        let bind_texture: GlBindTexture = glfn("glBindTexture");
        let is_texture: GlIsTexture = glfn("glIsTexture");
        let get_error: GlGetError = glfn("glGetError");
        let mut tex = 0u32;
        // SAFETY: a current context.
        unsafe {
            gen_textures(1, &mut tex);
            bind_texture(GL_TEXTURE_2D, tex);
            assert_eq!(get_error(), 0);
        }
        gl::gl_set_attribute(GlAttr::ShareWithCurrentContext, 1).unwrap();
        let shared = gl::gl_create_context(&window).unwrap();
        assert!(shared.is_current() && !ctx.is_current());
        // SAFETY: the shared context is current.
        assert_eq!(unsafe { is_texture(tex) }, 1);
        gl::gl_set_attribute(GlAttr::ShareWithCurrentContext, 0).unwrap();
        let unshared = gl::gl_create_context(&window).unwrap();
        // SAFETY: the unshared context is current.
        assert_eq!(unsafe { is_texture(tex) }, 0);
        drop(unshared);
        drop(shared);
        ctx.make_current(Some(&window)).unwrap();
        assert_eq!(
            gl_clear_and_read(&window, [0.0, 1.0, 0.0, 1.0]),
            [0, 255, 0, 255]
        );
        gl::gl_release_current().unwrap();
        assert!(gl::gl_current_context().unwrap().is_none());
        drop(ctx);
        window.destroy();

        // A core profile, at 3.2 or better
        gl::gl_set_attribute(GlAttr::ContextProfileMask, gl::GL_CONTEXT_PROFILE_CORE).unwrap();
        gl::gl_set_attribute(GlAttr::ContextMajorVersion, 3).unwrap();
        gl::gl_set_attribute(GlAttr::ContextMinorVersion, 2).unwrap();
        let window = Window::create("SDL Cocoa GL core", 96, 64, WindowFlags::OPENGL).unwrap();
        pump_for(100);
        let ctx = gl::gl_create_context(&window).unwrap();
        let version = gl_string(GL_VERSION);
        println!("cocoa: core profile GL {version}");
        let major: i32 = version.split('.').next().unwrap().parse().unwrap();
        assert!(major >= 3 && !version.starts_with("3.0") && !version.starts_with("3.1"));
        assert_eq!(
            gl_clear_and_read(&window, [0.0, 0.0, 1.0, 1.0]),
            [0, 0, 255, 255]
        );
        gl::gl_swap_window(&window).unwrap();
        drop(ctx);

        // A version the system doesn't have
        gl::gl_set_attribute(GlAttr::ContextMajorVersion, 4).unwrap();
        gl::gl_set_attribute(GlAttr::ContextMinorVersion, 6).unwrap();
        let err = gl::gl_create_context(&window).unwrap_err();
        assert_eq!(
            err.message(),
            "Failed creating OpenGL context at version requested"
        );
        assert!(gl::gl_current_context().unwrap().is_none());
        window.destroy();
        gl::gl_reset_attributes();
        println!("cocoa: opengl ok");
    }

    /// The directory of ANGLE's libEGL.dylib and libGLESv2.dylib.
    fn angle_dir() -> Option<std::path::PathBuf> {
        let dir = match std::env::var_os("SDL3_TEST_ANGLE_DIR") {
            Some(dir) => std::path::PathBuf::from(dir),
            None => std::path::PathBuf::from(std::env::var_os("HOME")?)
                .join("Library/Android/sdk/emulator/lib64/gles_angle"),
        };
        (dir.join("libEGL.dylib").exists() && dir.join("libGLESv2.dylib").exists()).then_some(dir)
    }

    fn opengl_es() {
        use video::gl::{self, GlAttr};

        let Some(dir) = angle_dir() else {
            skip_capability("egl", "no ANGLE (set SDL3_TEST_ANGLE_DIR)");
            return;
        };
        // (a copy for another architecture doesn't load: under Rosetta, say)
        let egl = dir.join("libEGL.dylib");
        if let Err(e) = sdl3::loadso::SharedObject::load(egl.to_str().unwrap()) {
            skip_capability("egl", &format!("ANGLE doesn't load ({})", e.message()));
            return;
        }
        hints::set(
            hints::EGL_LIBRARY,
            dir.join("libEGL.dylib").to_str().unwrap(),
        )
        .unwrap();
        hints::set(
            hints::OPENGL_LIBRARY,
            dir.join("libGLESv2.dylib").to_str().unwrap(),
        )
        .unwrap();
        gl::gl_reset_attributes();
        gl::gl_set_attribute(GlAttr::ContextProfileMask, gl::GL_CONTEXT_PROFILE_ES).unwrap();
        gl::gl_set_attribute(GlAttr::ContextMajorVersion, 3).unwrap();
        gl::gl_set_attribute(GlAttr::ContextMinorVersion, 0).unwrap();
        let window = Window::create("SDL Cocoa GLES", 96, 64, WindowFlags::OPENGL).unwrap();
        pump_for(100);
        let ctx = gl::gl_create_context(&window).unwrap();
        let version = gl_string(GL_VERSION);
        println!("cocoa: {version} on {}", gl_string(GL_RENDERER));
        assert!(version.starts_with("OpenGL ES 3"), "{version}");
        assert!(gl::egl_window_surface(&window).unwrap().is_some());
        assert_eq!(
            gl_clear_and_read(&window, [1.0, 1.0, 0.0, 1.0]),
            [255, 255, 0, 255]
        );
        gl::gl_set_swap_interval(1).unwrap();
        gl::gl_swap_window(&window).unwrap();
        gl::gl_set_swap_interval(0).unwrap();
        drop(ctx);
        window.destroy();

        // Back to NSOpenGL for a desktop profile
        gl::gl_reset_attributes();
        hints::reset(hints::EGL_LIBRARY);
        hints::reset(hints::OPENGL_LIBRARY);
        let window = Window::create("SDL Cocoa GL again", 96, 64, WindowFlags::OPENGL).unwrap();
        let ctx = gl::gl_create_context(&window).unwrap();
        assert!(gl_string(GL_VERSION).starts_with("2.1"));
        assert!(gl::egl_window_surface(&window).is_err());
        drop(ctx);
        window.destroy();
        println!("cocoa: opengl es ok");
    }

    /// `[obj selector]` returning an object.
    fn send_id(obj: *mut c_void, selector: &std::ffi::CStr) -> *mut c_void {
        // SAFETY: a method without arguments returning an object.
        unsafe {
            let send: unsafe extern "C" fn(*mut c_void, *mut c_void) -> *mut c_void =
                std::mem::transmute(objc_msgSend as unsafe extern "C" fn());
            send(obj, sel_registerName(selector.as_ptr()))
        }
    }

    /// The class name of an object.
    fn class_name(obj: *mut c_void) -> String {
        // SAFETY: a valid object; the name is a static C string.
        unsafe { std::ffi::CStr::from_ptr(object_getClassName(obj)) }
            .to_string_lossy()
            .into_owned()
    }

    /// `[layer drawableSize]` of a CAMetalLayer, rounded.
    fn drawable_size(layer: *mut c_void) -> (i32, i32) {
        #[repr(C)]
        struct CGSize {
            w: f64,
            h: f64,
        }
        // SAFETY: a CAMetalLayer's drawableSize (a CGSize, returned in
        // registers on both architectures).
        let size = unsafe {
            let send: unsafe extern "C" fn(*mut c_void, *mut c_void) -> CGSize =
                std::mem::transmute(objc_msgSend as unsafe extern "C" fn());
            send(layer, sel_registerName(c"drawableSize".as_ptr()))
        };
        (size.w.round() as i32, size.h.round() as i32)
    }

    fn metal() {
        let window = Window::create(
            "SDL Cocoa Metal",
            120,
            80,
            WindowFlags::METAL | WindowFlags::RESIZABLE | WindowFlags::HIGH_PIXEL_DENSITY,
        )
        .unwrap();
        pump_for(200);
        let flags = window.flags().unwrap();
        assert!(flags.contains(WindowFlags::METAL) && !flags.contains(WindowFlags::OPENGL));
        let view = video::vulkan::metal_create_view(&window).unwrap();
        assert_eq!(class_name(view as *mut c_void), "SDL3_cocoametalview");
        let layer = video::vulkan::metal_get_layer(view).unwrap();
        assert_eq!(class_name(layer as *mut c_void), "CAMetalLayer");
        assert_eq!(
            drawable_size(layer as *mut c_void),
            window.size_in_pixels().unwrap()
        );

        // The view tags itself (SDL_METALVIEW_TAG) and is the content view's subview.
        let props = window.properties().unwrap();
        assert_eq!(
            props.get_number(video::window::PROP_WINDOW_COCOA_METAL_VIEW_TAG_NUMBER),
            Some(255)
        );
        let nswindow = props
            .get_number(video::window::PROP_WINDOW_COCOA_WINDOW_POINTER)
            .unwrap();
        let content = send_id(nswindow as *mut c_void, c"contentView");
        // SAFETY: -[NSView viewWithTag:].
        let tagged = unsafe {
            let send: unsafe extern "C" fn(*mut c_void, *mut c_void, isize) -> *mut c_void =
                std::mem::transmute(objc_msgSend as unsafe extern "C" fn());
            send(content, sel_registerName(c"viewWithTag:".as_ptr()), 255)
        };
        assert_eq!(tagged as usize, view);

        // The drawable follows the window's pixel size.
        window.set_size(200, 150).unwrap();
        let events = pump_for(300);
        assert!(has_window_event(
            &events,
            &window,
            EventType::WINDOW_PIXEL_SIZE_CHANGED
        ));
        let pixels = window.size_in_pixels().unwrap();
        assert_eq!(drawable_size(layer as *mut c_void), pixels);
        println!(
            "cocoa: metal layer drawable {:?} for {:?}",
            drawable_size(layer as *mut c_void),
            window.size().unwrap()
        );
        video::vulkan::metal_destroy_view(view);
        // SAFETY: as above.
        let tagged = unsafe {
            let send: unsafe extern "C" fn(*mut c_void, *mut c_void, isize) -> *mut c_void =
                std::mem::transmute(objc_msgSend as unsafe extern "C" fn());
            send(content, sel_registerName(c"viewWithTag:".as_ptr()), 255)
        };
        assert!(tagged.is_null());
        window.destroy();

        // A view on an OpenGL window turns it into a Metal window.
        let window =
            Window::create("SDL Cocoa Metal from GL", 64, 64, WindowFlags::OPENGL).unwrap();
        let view = video::vulkan::metal_create_view(&window).unwrap();
        let flags = window.flags().unwrap();
        assert!(flags.contains(WindowFlags::METAL) && !flags.contains(WindowFlags::OPENGL));
        // Without high pixel density the drawable is in points.
        let layer = video::vulkan::metal_get_layer(view).unwrap();
        assert_eq!(drawable_size(layer as *mut c_void), (64, 64));
        video::vulkan::metal_destroy_view(view);
        window.destroy();
        println!("cocoa: metal views ok");
    }

    /// Report a missing capability (panicking when it's required).
    fn skip_capability(capability: &str, reason: &str) {
        let required = std::env::var("SDL3_TEST_REQUIRE")
            .unwrap_or_default()
            .split(',')
            .any(|c| c.trim() == "all" || c.trim() == capability);
        assert!(
            !required,
            "required capability {capability} unavailable: {reason}"
        );
        println!("note: skipping, {capability} unavailable: {reason}");
    }

    // The Vulkan bits the surface check needs.
    #[repr(C)]
    struct VkApplicationInfo {
        s_type: i32,
        p_next: *const c_void,
        p_application_name: *const c_char,
        application_version: u32,
        p_engine_name: *const c_char,
        engine_version: u32,
        api_version: u32,
    }
    #[repr(C)]
    struct VkInstanceCreateInfo {
        s_type: i32,
        p_next: *const c_void,
        flags: u32,
        p_application_info: *const VkApplicationInfo,
        enabled_layer_count: u32,
        pp_enabled_layer_names: *const *const c_char,
        enabled_extension_count: u32,
        pp_enabled_extension_names: *const *const c_char,
    }
    #[repr(C)]
    #[derive(Default, Debug)]
    struct VkExtent2D {
        width: u32,
        height: u32,
    }
    #[repr(C)]
    #[derive(Default, Debug)]
    struct VkSurfaceCapabilitiesKHR {
        min_image_count: u32,
        max_image_count: u32,
        current_extent: VkExtent2D,
        min_image_extent: VkExtent2D,
        max_image_extent: VkExtent2D,
        max_image_array_layers: u32,
        supported_transforms: u32,
        current_transform: u32,
        supported_composite_alpha: u32,
        supported_usage_flags: u32,
    }
    type VkGetInstanceProcAddr = unsafe extern "C" fn(*mut c_void, *const c_char) -> *const c_void;
    type VkCreateInstance =
        unsafe extern "C" fn(*const VkInstanceCreateInfo, *const c_void, *mut *mut c_void) -> i32;
    type VkDestroyInstance = unsafe extern "C" fn(*mut c_void, *const c_void);
    type VkEnumeratePhysicalDevices =
        unsafe extern "C" fn(*mut c_void, *mut u32, *mut *mut c_void) -> i32;
    type VkGetPhysicalDeviceSurfaceSupportKHR =
        unsafe extern "C" fn(*mut c_void, u32, u64, *mut u32) -> i32;
    type VkGetPhysicalDeviceSurfaceCapabilitiesKHR =
        unsafe extern "C" fn(*mut c_void, u64, *mut VkSurfaceCapabilitiesKHR) -> i32;

    fn vulkan() {
        use std::ffi::CString;
        use video::vulkan as vk;

        let window = Window::create(
            "SDL Cocoa Vulkan",
            160,
            100,
            WindowFlags::VULKAN | WindowFlags::HIGH_PIXEL_DENSITY,
        );
        let window = match window {
            Ok(w) => w,
            Err(e) if vk::vulkan_load_library(None).is_err() => {
                // Not on the default paths: try Homebrew's loader.
                let brew = [
                    "/opt/homebrew/lib/libvulkan.1.dylib",
                    "/usr/local/lib/libvulkan.1.dylib",
                ]
                .into_iter()
                .find(|p| std::path::Path::new(p).exists());
                let Some(brew) = brew else {
                    skip_capability("vulkan", &format!("no Vulkan loader ({})", e.message()));
                    return;
                };
                hints::set(hints::VULKAN_LIBRARY, brew).unwrap();
                match Window::create(
                    "SDL Cocoa Vulkan",
                    160,
                    100,
                    WindowFlags::VULKAN | WindowFlags::HIGH_PIXEL_DENSITY,
                ) {
                    Ok(w) => w,
                    Err(e) => {
                        skip_capability("vulkan", &format!("no Vulkan loader ({})", e.message()));
                        return;
                    }
                }
            }
            Err(e) => panic!("{}", e.message()),
        };
        pump_for(200);
        let extensions = vk::vulkan_instance_extensions().unwrap();
        assert_eq!(extensions, ["VK_KHR_surface", "VK_EXT_metal_surface"]);

        let gipa = vk::vulkan_get_vk_get_instance_proc_addr().unwrap();
        // SAFETY: the loader's vkGetInstanceProcAddr.
        let gipa: VkGetInstanceProcAddr = unsafe { std::mem::transmute(gipa) };
        let proc_addr = |instance: *mut c_void, name: &std::ffi::CStr| -> *const c_void {
            // SAFETY: a NUL-terminated name, a null or live instance.
            let p = unsafe { gipa(instance, name.as_ptr()) };
            assert!(!p.is_null(), "no {name:?}");
            p
        };

        // An instance with SDL's extensions (and the portability
        // enumeration MoltenVK's devices need through the loader).
        let mut names: Vec<CString> = extensions
            .iter()
            .map(|e| CString::new(*e).unwrap())
            .collect();
        names.push(CString::new("VK_KHR_portability_enumeration").unwrap());
        let ptrs: Vec<*const c_char> = names.iter().map(|n| n.as_ptr()).collect();
        let app = VkApplicationInfo {
            s_type: 0,
            p_next: std::ptr::null(),
            p_application_name: c"SDL Cocoa test".as_ptr(),
            application_version: 0,
            p_engine_name: std::ptr::null(),
            engine_version: 0,
            api_version: 1 << 22,
        };
        let create_info = VkInstanceCreateInfo {
            s_type: 1,
            p_next: std::ptr::null(),
            flags: 1, // VK_INSTANCE_CREATE_ENUMERATE_PORTABILITY_BIT_KHR
            p_application_info: &app,
            enabled_layer_count: 0,
            pp_enabled_layer_names: std::ptr::null(),
            enabled_extension_count: ptrs.len() as u32,
            pp_enabled_extension_names: ptrs.as_ptr(),
        };
        let mut instance: *mut c_void = std::ptr::null_mut();
        // SAFETY: the global function, of this type, with a valid create info.
        let rc = unsafe {
            let create: VkCreateInstance =
                std::mem::transmute(proc_addr(std::ptr::null_mut(), c"vkCreateInstance"));
            create(&create_info, std::ptr::null(), &mut instance)
        };
        assert_eq!(rc, 0, "vkCreateInstance");

        let surface = vk::vulkan_create_surface(&window, instance as usize, 0).unwrap();
        assert_ne!(surface, 0);

        // SAFETY: the instance's functions, of these types.
        unsafe {
            let enumerate: VkEnumeratePhysicalDevices =
                std::mem::transmute(proc_addr(instance, c"vkEnumeratePhysicalDevices"));
            let support: VkGetPhysicalDeviceSurfaceSupportKHR =
                std::mem::transmute(proc_addr(instance, c"vkGetPhysicalDeviceSurfaceSupportKHR"));
            let capabilities: VkGetPhysicalDeviceSurfaceCapabilitiesKHR = std::mem::transmute(
                proc_addr(instance, c"vkGetPhysicalDeviceSurfaceCapabilitiesKHR"),
            );
            let mut count = 1u32;
            let mut device: *mut c_void = std::ptr::null_mut();
            let rc = enumerate(instance, &mut count, &mut device);
            assert!(rc == 0 || rc == 5, "vkEnumeratePhysicalDevices {rc}"); // VK_INCOMPLETE
            assert_eq!(count, 1, "no Vulkan devices");
            assert!(
                vk::vulkan_presentation_support(instance as usize, device as usize, 0).unwrap()
            );
            let mut supported = 0u32;
            assert_eq!(support(device, 0, surface, &mut supported), 0);
            assert_eq!(supported, 1);
            let mut caps = VkSurfaceCapabilitiesKHR::default();
            assert_eq!(capabilities(device, surface, &mut caps), 0);
            let (pw, ph) = window.size_in_pixels().unwrap();
            println!(
                "cocoa: vulkan surface extent {}x{} for {pw}x{ph} pixels, {}..{} images",
                caps.current_extent.width,
                caps.current_extent.height,
                caps.min_image_count,
                caps.max_image_count
            );
            assert_eq!(
                (caps.current_extent.width, caps.current_extent.height),
                (pw as u32, ph as u32)
            );
        }

        vk::vulkan_destroy_surface(instance as usize, surface, 0);
        // SAFETY: the instance made above, its surfaces destroyed.
        unsafe {
            let destroy: VkDestroyInstance =
                std::mem::transmute(proc_addr(instance, c"vkDestroyInstance"));
            destroy(instance, std::ptr::null());
        }
        window.destroy();
        hints::reset(hints::VULKAN_LIBRARY);
        println!("cocoa: vulkan ok");
    }

    // The opengl renderer against the software renderer (as
    // render::opengl::tests on the other platforms).

    const RW: i32 = 64;
    const RH: i32 = 48;

    /// The pixels of a surface as tightly packed RGBA32.
    fn rgba(surface: &Surface<'_>) -> (i32, i32, Vec<u8>) {
        let s = surface.convert(PixelFormat::RGBA32).unwrap();
        let (w, h, pitch) = (s.width(), s.height(), s.pitch() as usize);
        let pixels = s.pixels().unwrap();
        let mut out = Vec::new();
        for y in 0..h as usize {
            out.extend_from_slice(&pixels[y * pitch..y * pitch + w as usize * 4]);
        }
        (w, h, out)
    }

    /// Compare two images: at most `max_bad` pixels may have a color
    /// channel off by more than `tolerance`.
    fn compare(
        what: &str,
        a: &(i32, i32, Vec<u8>),
        b: &(i32, i32, Vec<u8>),
        tolerance: u8,
        max_bad: usize,
    ) {
        assert_eq!((a.0, a.1), (b.0, b.1), "{what}: sizes");
        let mut bad = 0;
        let mut first = None;
        for (i, (pa, pb)) in a.2.chunks(4).zip(b.2.chunks(4)).enumerate() {
            if (0..3).any(|c| pa[c].abs_diff(pb[c]) > tolerance) {
                bad += 1;
                if first.is_none() {
                    first = Some((i as i32 % a.0, i as i32 / a.0, pa.to_vec(), pb.to_vec()));
                }
            }
        }
        assert!(
            bad <= max_bad,
            "{what}: {bad} pixels differ (first at {first:?}: opengl vs software)"
        );
    }

    /// Draw a scene with the window's renderer and a software renderer and
    /// compare the results.
    fn compare_scene(
        gl: &mut sdl3::render::Renderer,
        what: &str,
        tolerance: u8,
        max_bad: usize,
        scene: impl Fn(&mut sdl3::render::Renderer) -> sdl3::error::Result<()>,
    ) {
        scene(gl).unwrap_or_else(|e| panic!("{what}: opengl: {}", e.message()));
        let a = rgba(&gl.read_pixels(None).unwrap());
        gl.present().unwrap();
        let mut sw =
            sdl3::render::Renderer::software(Surface::new(RW, RH, PixelFormat::ARGB8888).unwrap())
                .unwrap();
        scene(&mut sw).unwrap_or_else(|e| panic!("{what}: software: {}", e.message()));
        let b = rgba(&sw.read_pixels(None).unwrap());
        compare(what, &a, &b, tolerance, max_bad);
    }

    /// A 16x16 texture of a pattern with varying alpha.
    fn pattern_surface(format: PixelFormat) -> Surface<'static> {
        let mut s = Surface::new(16, 16, format).unwrap();
        for y in 0..16 {
            for x in 0..16 {
                let c = video::Color::new(
                    (x * 16) as u8,
                    (y * 16) as u8,
                    ((x + y) * 8) as u8,
                    if (x + y) % 3 == 0 {
                        255
                    } else {
                        (x * 8 + y * 7) as u8
                    },
                );
                s.write_pixel(x, y, c).unwrap();
            }
        }
        s
    }

    fn renderer() {
        use sdl3::render::{Renderer, TextureAccess};
        use video::BlendMode;
        use video::{FPoint, FRect};

        let window = Window::create("SDL Cocoa renderer", RW, RH, WindowFlags::default()).unwrap();
        pump_for(100);
        let mut r = Renderer::for_window(&window, Some("opengl")).unwrap();
        assert_eq!(r.name(), "opengl");
        assert_eq!(r.output_size().unwrap(), (RW, RH));
        // Apple's 4:2:2 YCbCr textures (GL_APPLE_ycbcr_422)
        assert!(
            r.texture_formats().contains(&PixelFormat::UYVY),
            "{:?}",
            r.texture_formats()
        );

        compare_scene(&mut r, "clear and fill", 0, 0, |r| {
            r.set_draw_color(10, 20, 30, 255);
            r.clear()?;
            r.set_draw_color(200, 100, 50, 255);
            r.render_fill_rect(Some(&FRect::new(8.0, 8.0, 32.0, 24.0)))?;
            r.set_draw_color(0, 255, 0, 255);
            r.render_fill_rects(&[
                FRect::new(-5.0, -5.0, 10.0, 10.0),
                FRect::new(50.0, 40.0, 30.0, 30.0),
            ])?;
            r.set_draw_color(255, 255, 255, 255);
            r.render_rect(Some(&FRect::new(2.0, 30.0, 12.0, 10.0)))
        });
        compare_scene(&mut r, "points and lines", 0, 4, |r| {
            r.set_draw_color(0, 0, 0, 255);
            r.clear()?;
            r.set_draw_color(255, 128, 0, 255);
            let points: Vec<FPoint> = (0..40)
                .map(|i| FPoint {
                    x: (i * 7 % 64) as f32,
                    y: (i * 5 % 48) as f32,
                })
                .collect();
            r.render_points(&points)?;
            r.set_draw_color(255, 255, 0, 255);
            r.render_line(0.0, 0.0, 63.0, 0.0)?;
            r.render_line(5.0, 3.0, 5.0, 40.0)?;
            r.render_line(10.0, 10.0, 40.0, 30.0)
        });
        compare_scene(&mut r, "viewport and clip", 0, 0, |r| {
            r.set_draw_color(0, 0, 0, 255);
            r.clear()?;
            r.set_viewport(Some(&video::Rect::new(10, 6, 40, 30)))?;
            r.set_draw_color(255, 0, 0, 255);
            r.render_fill_rect(None)?;
            r.set_clip_rect(Some(&video::Rect::new(5, 5, 12, 8)))?;
            r.set_draw_color(0, 0, 255, 255);
            r.render_fill_rect(Some(&FRect::new(0.0, 0.0, 30.0, 30.0)))?;
            r.set_clip_rect(None)?;
            r.set_viewport(None)
        });
        compare_scene(&mut r, "blend modes", 2, 0, |r| {
            r.set_draw_blend_mode(BlendMode::NONE)?;
            r.set_draw_color(10, 20, 30, 255);
            r.clear()?;
            for (i, mode) in [
                BlendMode::BLEND,
                BlendMode::ADD,
                BlendMode::MOD,
                BlendMode::MUL,
            ]
            .into_iter()
            .enumerate()
            {
                r.set_draw_blend_mode(mode)?;
                r.set_draw_color(220, 90, 40, 128);
                r.render_fill_rect(Some(&FRect::new(i as f32 * 12.0, 12.0, 10.0, 30.0)))?;
            }
            Ok(())
        });
        for format in [
            PixelFormat::ARGB8888,
            PixelFormat::ABGR8888,
            PixelFormat::RGB24,
            PixelFormat::XRGB8888,
        ] {
            compare_scene(&mut r, &format!("texture {format:?}"), 2, 0, |r| {
                r.set_draw_blend_mode(BlendMode::NONE)?;
                r.set_draw_color(40, 50, 60, 255);
                r.clear()?;
                let tex = r.create_texture_from_surface(&mut pattern_surface(format))?;
                r.set_texture_scale_mode(tex, video::ScaleMode::Nearest)?;
                r.render_texture(tex, None, Some(&FRect::new(4.0, 4.0, 16.0, 16.0)))?;
                r.set_texture_color_mod(tex, 255, 128, 64)?;
                r.set_texture_alpha_mod(tex, 160)?;
                r.render_texture(tex, None, Some(&FRect::new(24.0, 8.0, 32.0, 32.0)))?;
                r.destroy_texture(tex);
                Ok(())
            });
        }
        // A streaming texture (Apple client storage when 8-aligned), updated twice
        compare_scene(&mut r, "streaming texture", 0, 0, |r| {
            r.set_draw_color(0, 0, 0, 255);
            r.clear()?;
            let tex = r.create_texture(PixelFormat::ARGB8888, TextureAccess::Streaming, 16, 8)?;
            r.set_texture_scale_mode(tex, video::ScaleMode::Nearest)?;
            let mut pixels = vec![0u8; 16 * 8 * 4];
            for (i, p) in pixels.chunks_mut(4).enumerate() {
                p.copy_from_slice(&[(i * 3) as u8, (i * 5) as u8, 200, 255]);
            }
            r.update_texture(tex, None, &pixels, 64)?;
            r.render_texture(tex, None, Some(&FRect::new(0.0, 0.0, 16.0, 8.0)))?;
            for p in pixels.chunks_mut(4) {
                p[..3].iter_mut().for_each(|b| *b = b.wrapping_add(40));
            }
            r.update_texture(tex, None, &pixels, 64)?;
            r.render_texture(tex, None, Some(&FRect::new(20.0, 20.0, 16.0, 8.0)))?;
            r.destroy_texture(tex);
            Ok(())
        });
        // Render targets
        compare_scene(&mut r, "render target", 0, 0, |r| {
            let target = r.create_texture(PixelFormat::ARGB8888, TextureAccess::Target, 32, 24)?;
            r.set_render_target(Some(target))?;
            r.set_draw_color(255, 0, 255, 255);
            r.clear()?;
            r.set_draw_color(0, 255, 0, 255);
            r.render_fill_rect(Some(&FRect::new(4.0, 4.0, 8.0, 8.0)))?;
            r.set_render_target(None)?;
            r.set_draw_color(0, 0, 0, 255);
            r.clear()?;
            r.set_texture_scale_mode(target, video::ScaleMode::Nearest)?;
            r.render_texture(target, None, Some(&FRect::new(8.0, 8.0, 32.0, 24.0)))?;
            r.destroy_texture(target);
            Ok(())
        });
        drop(r);
        window.destroy();
        println!("cocoa: opengl renderer ok");
    }

    fn window_surface() {
        // Cocoa has no window framebuffer: surfaces go through a renderer's
        // streaming texture (SDL_CreateWindowTexture()).
        let window = Window::create("SDL Cocoa surface", 80, 60, WindowFlags::RESIZABLE).unwrap();
        pump_for(100);
        let surface = window.surface().unwrap();
        assert!(window.has_surface().unwrap());
        {
            let mut s = surface.lock();
            assert_eq!((s.width(), s.height()), window.size_in_pixels().unwrap());
            println!(
                "cocoa: window surface {:?} {}x{}",
                s.format(),
                s.width(),
                s.height()
            );
            let color = s
                .format()
                .details()
                .unwrap()
                .map_rgba(None, video::Color::new(255, 0, 0, 255))
                .unwrap();
            s.fill_rect(None, color).unwrap();
        }
        surface.update().unwrap();
        surface
            .update_rects(&[
                video::Rect::new(0, 0, 10, 10),
                video::Rect::new(20, 30, 5, 5),
            ])
            .unwrap();
        window.set_surface_vsync(1).unwrap();
        assert_eq!(window.surface_vsync().unwrap(), 1);
        window.set_surface_vsync(0).unwrap();
        // The window's renderer is the surface's.
        assert!(sdl3::render::Renderer::for_window(&window, None).is_err());

        // A new size invalidates the surface; the next one has the new size.
        window.set_size(100, 70).unwrap();
        pump_for(200);
        assert!(surface.update().is_err());
        let surface = window.surface().unwrap();
        assert_eq!(
            {
                let s = surface.lock();
                (s.width(), s.height())
            },
            window.size_in_pixels().unwrap()
        );
        surface.update().unwrap();
        window.destroy_surface().unwrap();
        assert!(!window.has_surface().unwrap());
        // The renderer went with the surface.
        let r = sdl3::render::Renderer::for_window(&window, Some("opengl")).unwrap();
        drop(r);
        window.destroy();
        println!("cocoa: window surfaces ok");
    }

    fn displays_and_modes() {
        let displays = video::displays().unwrap();
        assert!(!displays.is_empty(), "no displays");
        let primary = video::primary_display().unwrap();
        assert_eq!(primary, displays[0]);
        for &d in &displays {
            let name = video::display_name(d).unwrap();
            let bounds = video::display_bounds(d).unwrap();
            let usable = video::display_usable_bounds(d).unwrap();
            let desktop = video::desktop_display_mode(d).unwrap();
            let current = video::current_display_mode(d).unwrap();
            let scale = video::display_content_scale(d).unwrap();
            let modes = video::fullscreen_display_modes(d).unwrap();
            println!(
                "cocoa: display {d} {name:?} {bounds:?} usable {usable:?} desktop {}x{}@{} x{} {:?}, {} modes, content scale {scale}",
                desktop.w,
                desktop.h,
                desktop.refresh_rate,
                desktop.pixel_density,
                desktop.format,
                modes.len()
            );
            assert!(!name.is_empty());
            assert!(bounds.w > 0 && bounds.h > 0);
            assert!(usable.w > 0 && usable.h > 0);
            assert!(usable.x >= bounds.x && usable.y >= bounds.y);
            assert!(usable.x + usable.w <= bounds.x + bounds.w);
            assert!(usable.y + usable.h <= bounds.y + bounds.h);
            assert_eq!((desktop.w, desktop.h), (bounds.w, bounds.h));
            assert!(desktop.pixel_density >= 1.0);
            assert!(matches!(
                desktop.format,
                PixelFormat::ARGB8888 | PixelFormat::ARGB2101010 | PixelFormat::ARGB1555
            ));
            assert_eq!(current, desktop);
            assert!(!modes.is_empty());
            for m in &modes {
                assert!(m.w > 0 && m.h > 0 && m.pixel_density >= 1.0);
                assert_eq!(m.display_id, d);
            }
            assert!(scale >= 1.0);
        }
    }

    fn windows() {
        let window = Window::create(
            "SDL Cocoa test",
            320,
            240,
            WindowFlags::RESIZABLE | WindowFlags::HIGH_PIXEL_DENSITY,
        )
        .unwrap();
        let events = pump_for(300);
        let flags = window.flags().unwrap();
        assert!(!flags.contains(WindowFlags::HIDDEN), "{flags:?}");
        assert!(flags.contains(WindowFlags::RESIZABLE));
        assert!(has_window_event(&events, &window, EventType::WINDOW_SHOWN));
        assert_eq!(window.size().unwrap(), (320, 240));
        let density = window.pixel_density().unwrap();
        let (pw, ph) = window.size_in_pixels().unwrap();
        assert_eq!(
            (pw, ph),
            ((320.0 * density) as i32, (240.0 * density) as i32)
        );
        assert!(window.display().unwrap() != 0);
        println!(
            "cocoa: window {:?} density {density}",
            window.position().unwrap()
        );

        // Title
        window.set_title("SDL Cocoa test ✓").unwrap();
        assert_eq!(window.title().unwrap(), "SDL Cocoa test ✓");

        // Position and size round trips
        window.set_position(120, 160).unwrap();
        window.sync().unwrap();
        let events = pump_for(100);
        assert_eq!(window.position().unwrap(), (120, 160));
        let _ = events;
        window.set_size(400, 300).unwrap();
        window.sync().unwrap();
        let events = pump_for(100);
        assert_eq!(window.size().unwrap(), (400, 300));
        assert_eq!(window.position().unwrap(), (120, 160));
        let _ = events;

        // Minimum and maximum sizes
        window.set_minimum_size(200, 150).unwrap();
        window.set_maximum_size(800, 600).unwrap();
        window.set_size(100, 100).unwrap();
        window.sync().unwrap();
        pump_for(100);
        let (w, h) = window.size().unwrap();
        assert!(w >= 200 && h >= 150, "{w}x{h}");
        window.set_minimum_size(0, 0).unwrap();
        window.set_maximum_size(0, 0).unwrap();

        // Hide and show
        window.hide().unwrap();
        let events = pump_for(200);
        assert!(window.flags().unwrap().contains(WindowFlags::HIDDEN));
        assert!(has_window_event(&events, &window, EventType::WINDOW_HIDDEN));
        window.show().unwrap();
        let events = pump_for(200);
        assert!(!window.flags().unwrap().contains(WindowFlags::HIDDEN));
        assert!(has_window_event(&events, &window, EventType::WINDOW_SHOWN));

        // Borders, always on top, opacity
        window.set_bordered(false).unwrap();
        pump_for(50);
        assert!(window.flags().unwrap().contains(WindowFlags::BORDERLESS));
        window.set_bordered(true).unwrap();
        pump_for(50);
        assert!(!window.flags().unwrap().contains(WindowFlags::BORDERLESS));
        window.set_always_on_top(true).unwrap();
        assert!(window.flags().unwrap().contains(WindowFlags::ALWAYS_ON_TOP));
        window.set_always_on_top(false).unwrap();
        window.set_opacity(0.5).unwrap();
        assert_eq!(window.opacity().unwrap(), 0.5);
        window.set_opacity(1.0).unwrap();
        window.set_resizable(false).unwrap();
        assert!(!window.flags().unwrap().contains(WindowFlags::RESIZABLE));
        window.set_resizable(true).unwrap();

        // Minimize and restore (animated: sync waits for them)
        window.minimize().unwrap();
        window.sync().unwrap();
        let events = pump_for(200);
        assert!(window.flags().unwrap().contains(WindowFlags::MINIMIZED));
        assert!(has_window_event(
            &events,
            &window,
            EventType::WINDOW_MINIMIZED
        ));
        window.restore().unwrap();
        window.sync().unwrap();
        let events = pump_for(500);
        assert!(!window.flags().unwrap().contains(WindowFlags::MINIMIZED));
        assert!(has_window_event(
            &events,
            &window,
            EventType::WINDOW_RESTORED
        ));

        // Maximize and restore
        window.maximize().unwrap();
        window.sync().unwrap();
        pump_for(300);
        assert!(window.flags().unwrap().contains(WindowFlags::MAXIMIZED));
        window.restore().unwrap();
        window.sync().unwrap();
        pump_for(300);
        assert!(!window.flags().unwrap().contains(WindowFlags::MAXIMIZED));

        // A second window, raised over the first
        let other = Window::create("SDL Cocoa test 2", 200, 100, WindowFlags::NONE).unwrap();
        pump_for(200);
        other.raise().unwrap();
        pump_for(100);
        other.destroy();
        pump_for(100);

        // The window's properties
        let props = window.properties().unwrap();
        assert!(props.get_number("SDL.window.cocoa.window").unwrap_or(0) != 0);
        assert_eq!(
            props.get_number("SDL.window.cocoa.metal_view_tag"),
            Some(255)
        );

        window.destroy();
        pump_for(100);
        println!("cocoa: windows ok");
    }

    fn popups() {
        let parent = Window::create("SDL Cocoa popup parent", 300, 200, WindowFlags::NONE).unwrap();
        parent.set_position(200, 200).unwrap();
        parent.sync().unwrap();
        pump_for(200);
        let tooltip = Window::create_popup(&parent, 10, 20, 100, 30, WindowFlags::TOOLTIP).unwrap();
        pump_for(200);
        assert!(tooltip.flags().unwrap().contains(WindowFlags::TOOLTIP));
        assert_eq!(tooltip.position().unwrap(), (10, 20));
        let menu = Window::create_popup(&parent, 30, 40, 120, 80, WindowFlags::POPUP_MENU).unwrap();
        pump_for(200);
        assert!(menu.flags().unwrap().contains(WindowFlags::POPUP_MENU));
        menu.destroy();
        tooltip.destroy();
        parent.destroy();
        pump_for(100);
        println!("cocoa: popups ok");
    }

    fn fullscreen() {
        let window =
            Window::create("SDL Cocoa fullscreen", 320, 240, WindowFlags::RESIZABLE).unwrap();
        pump_for(300);
        let display = window.display().unwrap();
        let bounds = video::display_bounds(display).unwrap();

        // Desktop fullscreen (a Space, animated)
        window.set_fullscreen(true).unwrap();
        window.sync().unwrap();
        let events = pump_for(500);
        assert!(window.flags().unwrap().contains(WindowFlags::FULLSCREEN));
        assert!(has_window_event(
            &events,
            &window,
            EventType::WINDOW_ENTER_FULLSCREEN
        ));
        let (w, h) = window.size().unwrap();
        println!("cocoa: fullscreen {w}x{h} on {}x{}", bounds.w, bounds.h);
        assert_eq!(w, bounds.w);
        assert!(h > 0 && h <= bounds.h);

        window.set_fullscreen(false).unwrap();
        window.sync().unwrap();
        let events = pump_for(500);
        assert!(!window.flags().unwrap().contains(WindowFlags::FULLSCREEN));
        assert!(has_window_event(
            &events,
            &window,
            EventType::WINDOW_LEAVE_FULLSCREEN
        ));
        assert_eq!(window.size().unwrap(), (320, 240));

        window.destroy();
        pump_for(200);
        println!("cocoa: fullscreen ok");
    }

    fn text_input() {
        let window = Window::create("SDL Cocoa text input", 200, 100, WindowFlags::NONE).unwrap();
        pump_for(200);
        window.start_text_input().unwrap();
        assert!(window.text_input_active().unwrap());
        window
            .set_text_input_area(Some(&video::Rect::new(10, 20, 30, 40)), 5)
            .unwrap();
        window.stop_text_input().unwrap();
        assert!(!window.text_input_active().unwrap());
        window.destroy();
        pump_for(100);
        println!("cocoa: text input ok");
    }

    fn clipboard() {
        // (the user's clipboard text is put back afterwards)
        let saved = video::clipboard::clipboard_text().ok();
        video::clipboard::set_clipboard_text("héllo from SDL ✓").unwrap();
        assert!(video::clipboard::has_clipboard_text().unwrap());
        assert_eq!(
            video::clipboard::clipboard_text().unwrap(),
            "héllo from SDL ✓"
        );
        assert!(video::clipboard::has_clipboard_data("text/plain;charset=utf-8").unwrap());
        // Data offered by mime type, produced when asked for
        let png = vec![0x89u8, b'P', b'N', b'G', 1, 2, 3];
        let data = png.clone();
        video::clipboard::set_clipboard_data(
            Some(std::sync::Arc::new(move |mime: &str| {
                (mime == "image/png").then(|| data.clone())
            })),
            &["image/png"],
        )
        .unwrap();
        assert!(video::clipboard::has_clipboard_data("image/png").unwrap());
        assert_eq!(
            video::clipboard::clipboard_data("image/png")
                .unwrap()
                .as_deref(),
            Some(&png[..])
        );
        if let Some(saved) = saved.filter(|s| !s.is_empty()) {
            let _ = video::clipboard::set_clipboard_text(&saved);
        }
        println!("cocoa: clipboard ok");
    }

    fn cursors() {
        use sdl3::events::mouse::*;
        let window = Window::create("SDL Cocoa cursors", 200, 100, WindowFlags::NONE).unwrap();
        pump_for(100);
        for id in [
            SystemCursor::Default,
            SystemCursor::Text,
            SystemCursor::Wait,
            SystemCursor::Crosshair,
            SystemCursor::Progress,
            SystemCursor::NwseResize,
            SystemCursor::NeswResize,
            SystemCursor::EwResize,
            SystemCursor::NsResize,
            SystemCursor::Move,
            SystemCursor::NotAllowed,
            SystemCursor::Pointer,
            SystemCursor::NwResize,
            SystemCursor::NResize,
            SystemCursor::NeResize,
            SystemCursor::EResize,
            SystemCursor::SeResize,
            SystemCursor::SResize,
            SystemCursor::SwResize,
            SystemCursor::WResize,
        ] {
            let cursor = create_system_cursor(id).unwrap();
            set_cursor(Some(&cursor)).unwrap();
            pump_for(10);
            destroy_cursor(cursor);
        }
        let mut surface = Surface::new(16, 16, PixelFormat::ARGB8888).unwrap();
        surface.fill_rect(None, 0xff00ff00).unwrap();
        let cursor = create_color_cursor(&surface, 8, 8).unwrap();
        set_cursor(Some(&cursor)).unwrap();
        hide_cursor();
        assert!(!cursor_visible());
        show_cursor();
        assert!(cursor_visible());
        pump_for(50);
        set_cursor(default_cursor().as_ref()).unwrap();
        destroy_cursor(cursor);
        window.destroy();
        pump_for(50);
        println!("cocoa: cursors ok");
    }

    fn keymap() {
        // Layout-independent keys
        assert_eq!(
            key_from_scancode(Scancode::RETURN, Keymod::NONE, false),
            Keycode(b'\r' as u32)
        );
        assert_eq!(
            key_from_scancode(Scancode::ESCAPE, Keymod::NONE, false),
            Keycode(0x1b)
        );
        // The letter keys type something on any layout
        let a = key_from_scancode(Scancode::A, Keymod::NONE, false);
        assert!(
            a != Keycode::UNKNOWN && a.0 & Keycode::SCANCODE_MASK == 0,
            "{a:?}"
        );
        assert_eq!(Scancode::LALT.name(), "Left Option");
        assert_eq!(Scancode::RGUI.name(), "Right Command");
        println!("cocoa: keymap ok (A types {:?})", char::from_u32(a.0));
    }

    fn mouse_warp() {
        use sdl3::events::mouse::*;
        let window = Window::create("SDL Cocoa warp", 300, 200, WindowFlags::NONE).unwrap();
        window.set_position(150, 150).unwrap();
        window.sync().unwrap();
        pump_for(300);
        let (ox, oy, _) = global_mouse_state();
        warp_mouse_in_window(Some(window.id()), 40.0, 30.0);
        pump_for(100);
        let (x, y, _) = global_mouse_state();
        let (wx, wy) = window.position().unwrap();
        println!("cocoa: warped to ({x}, {y}), window at ({wx}, {wy})");
        assert!((x - (wx + 40) as f32).abs() <= 1.0 && (y - (wy + 30) as f32).abs() <= 1.0);
        let _ = warp_mouse_global(ox, oy);
        window.destroy();
        pump_for(50);
        println!("cocoa: mouse warp ok");
    }

    fn message_box() {
        let data = MessageBoxData {
            flags: MessageBoxFlags::WARNING,
            window: None,
            title: "SDL Cocoa test".into(),
            message: "This alert closes itself.".into(),
            buttons: vec![
                MessageBoxButtonData {
                    flags: MessageBoxButtonFlags::RETURNKEY_DEFAULT,
                    button_id: 7,
                    text: "First".into(),
                },
                MessageBoxButtonData {
                    flags: MessageBoxButtonFlags::ESCAPEKEY_DEFAULT,
                    button_id: 42,
                    text: "Second".into(),
                },
            ],
            color_scheme: None,
        };
        // Answer with the second button from the main queue, which the
        // alert's modal loop runs.
        // SAFETY: the main queue and a function taking no context.
        unsafe {
            dispatch_async_f(
                std::ptr::addr_of!(_dispatch_main_q),
                std::ptr::null_mut(),
                stop_modal_with_second_button,
            );
        }
        assert_eq!(show_message_box(&data).unwrap(), 42);
        println!("cocoa: message box ok");
    }
}
