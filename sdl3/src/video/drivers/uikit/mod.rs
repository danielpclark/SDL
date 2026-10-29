// Rust translation of src/video/uikit/SDL_uikitvideo.m and
// SDL_uikitvideo.h from Simple DirectMedia Layer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! The UIKit video driver (iOS, tvOS, visionOS): the app delegate that
//! `UIApplicationMain()` runs and that calls the app's main function
//! ([`appdelegate`]), the lifecycle notifications and the event pump
//! ([`events`]), the displays and their modes ([`modes`]), and the
//! windows ([`window`]), with their views ([`view`], which turn UIKit's
//! touches, pointer, pinches and hardware keys' presses into SDL's
//! events) and view controllers ([`viewcontroller`], with text input and
//! the on-screen keyboard), the Apple Pencil ([`pen`]), the game
//! controllers' keyboards and mice ([`events`]), the clipboard
//! ([`clipboard`], the general pasteboard's text), message boxes
//! ([`messagebox`], alert controllers), Metal views ([`metalview`],
//! which the Metal renderer and the GPU API's swapchains draw to), Vulkan
//! surfaces on Metal views ([`vulkan`], through MoltenVK) and, on iOS and
//! tvOS, OpenGL ES contexts ([`opengles`], with their views,
//! [`openglview`]).
//!
//! Upstream writes the driver in Objective-C; its classes
//! (`SDLUIKitDelegate`, `SDLUIKitSceneDelegate`, `SDLLaunchScreenController`,
//! `SDL_LifecycleObserver`, `SDL_DisplayWatch`, ...) are declared at run
//! time here, their methods implemented in Rust (see
//! [`crate::core::apple::objc`]), as the Cocoa driver's are.
//!
//! The driver only starts in an app: on the main thread of a process whose
//! `UIApplicationMain()` runs ([`crate::app::run_app`]). Note: upstream
//! never runs outside `UIApplicationMain()`; here a program that isn't an
//! app (a test program run as a plain process of a simulated device)
//! falls through to the next driver, as one on another thread does with
//! the Cocoa driver.
//!
//! `SDL_NSLog()`, which upstream copies here from the Cocoa driver, is
//! [`crate::log`]'s on every Apple platform.

#![allow(non_upper_case_globals, non_snake_case)] // (the SDK's names are kept)

pub(crate) mod appdelegate;
// (visionOS's Swift bridge: stand-ins, see the module)
#[cfg(target_os = "visionos")]
pub(crate) mod bridge;
pub(crate) mod clipboard;
pub(crate) mod events;
pub(crate) mod messagebox;
pub(crate) mod metalview;
pub(crate) mod modes;
// OpenGL ES: SDL_VIDEO_OPENGL_ES and SDL_VIDEO_OPENGL_ES2, which upstream's
// SDL_build_config_ios.h builds on iOS and tvOS, not on visionOS or Mac
// Catalyst. Note: upstream's CMake build leaves it out on tvOS too
// (SDL_OPENGLES is off there); this follows the Xcode project's
// configuration, with OpenGLES.framework linked on both.
#[cfg(all(any(target_os = "ios", target_os = "tvos"), not(target_abi = "macabi")))]
pub(crate) mod opengles;
#[cfg(all(any(target_os = "ios", target_os = "tvos"), not(target_abi = "macabi")))]
pub(crate) mod openglview;
pub(crate) mod pen;
pub(crate) mod sys;
pub(crate) mod view;
pub(crate) mod viewcontroller;
// SDL_VIDEO_VULKAN, which upstream builds wherever it builds Metal (64-bit
// devices, and the Simulators from iOS 13 and tvOS 13)
pub(crate) mod vulkan;
pub(crate) mod window;

use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex, MutexGuard};

use crate::core::apple::foundation::CGRect;
use crate::core::apple::objc::{autoreleasepool, class, msg_send, Bool, Id, StrongPtr};
use crate::core::apple::uikit::shared_application;
use crate::error::Result;
use crate::events::mouse::MouseFeature;
use crate::events::{DisplayID, WindowID};
use crate::properties::Properties;
use crate::video::sysvideo::{
    DeviceCaps, FullscreenOp, FullscreenResult, SystemTheme, VideoBootStrap, VideoDriver,
};
use crate::video::window::WindowOp;
use crate::video::{DisplayMode, Rect};

#[cfg(not(any(target_os = "tvos", target_os = "visionos")))]
use sys::UIInterfaceOrientation;
use sys::{UISceneActivationStateForegroundActive, UISceneActivationStateForegroundInactive};
#[cfg(not(target_os = "visionos"))]
use sys::{UIUserInterfaceStyleDark, UIUserInterfaceStyleLight};

/// The UIKit video driver (the function pointers of the `SDL_VideoDevice`
/// that `UIKit_CreateDevice()` fills in), with its data.
pub(crate) struct UIKitVideo {
    data: Arc<VideoData>,
}

/// The driver's data (`_this->internal`). Translation of
/// `SDL_UIKitVideoData`: the clipboard's state.
pub(crate) struct VideoData {
    pasteboard_observer: Mutex<StrongPtr>,
    /// Whether the pasteboard is being set (its change notification is
    /// SDL's own then; tvOS has no pasteboard).
    #[cfg_attr(target_os = "tvos", allow(dead_code))]
    pub(crate) setting_clipboard: AtomicBool,
}

impl VideoData {
    /// `data.pasteboardObserver`.
    pub(crate) fn pasteboard_observer(&self) -> MutexGuard<'_, StrongPtr> {
        self.pasteboard_observer
            .lock()
            .unwrap_or_else(|e| e.into_inner())
    }
}

/// Translation of `UIKit_CreateDevice()`.
fn create_device() -> Option<Arc<dyn VideoDriver>> {
    autoreleasepool(|| {
        // Note: upstream always runs in UIApplicationMain(), on the main
        // thread; outside an app (or on another thread) there's no UIKit
        // to drive, so the next driver gets its chance.
        if !crate::core::apple::foundation::is_main_thread() || shared_application().is_null() {
            return None; // this doesn't SDL_SetError() because SDL_InitVideo is just going to overwrite it.
        }

        let data = Arc::new(VideoData {
            pasteboard_observer: Mutex::new(StrongPtr::nil()),
            setting_clipboard: AtomicBool::new(false),
        });
        let video: Arc<dyn VideoDriver> = Arc::new(UIKitVideo { data });
        Some(video)
    })
}

/// Translation of `UIKIT_bootstrap`.
pub(crate) static UIKIT_BOOTSTRAP: VideoBootStrap = VideoBootStrap {
    name: "uikit",
    desc: "SDL UIKit video driver",
    create: create_device,
    show_message_box: Some(messagebox::show_message_box),
    is_preferred: false,
};

/// Translation of `UIKit_VideoInit()`.
fn video_init(data: &Arc<VideoData>) -> Result<()> {
    let _ = crate::video::core::with_device(|v| v.gl_driver_loaded = 1);

    if !modes::init_modes() {
        return Err(crate::error::Error::new(
            "The UIKit video driver couldn't add its displays",
        ));
    }

    events::init_gc_keyboard();
    events::init_gc_mouse();

    clipboard::init_clipboard(data);

    Ok(())
}

/// Translation of `UIKit_VideoQuit()`.
fn video_quit(data: &VideoData) {
    clipboard::quit_clipboard(data);

    events::quit_gc_keyboard();
    events::quit_gc_mouse();
    pen::quit_pen();

    modes::quit_modes();
}

/// Translation of `UIKit_SuspendScreenSaver()`.
fn suspend_screen_saver(suspend: bool) -> Result<()> {
    autoreleasepool(|| {
        let app = shared_application();

        // Prevent the display from dimming and going to sleep.
        // SAFETY: a UIApplication property.
        unsafe {
            let () = msg_send![app, setIdleTimerDisabled: Bool::from(suspend)];
        }
    });
    Ok(())
}

/// Translation of `UIKit_IsSystemVersionAtLeast()`.
pub(crate) fn is_system_version_at_least(version: f64) -> bool {
    // SAFETY: UIDevice's systemVersion, an NSString.
    unsafe {
        let system_version: Id =
            msg_send![crate::core::apple::uikit::current_device(), systemVersion];
        let value: f64 = msg_send![system_version, doubleValue];
        value >= version
    }
}

/// The system theme, from the main screen's traits. Translation of
/// `UIKit_GetSystemTheme()`.
pub(crate) fn get_system_theme() -> SystemTheme {
    #[cfg(not(target_os = "visionos"))]
    if crate::core::apple::foundation::available!(ios(12, 0), tvos(10, 0)) {
        // SAFETY: UIScreen's main screen and its trait collection.
        let style: sys::UIUserInterfaceStyle = unsafe {
            let screen: Id = msg_send![class!(UIScreen), mainScreen];
            let traits: Id = msg_send![screen, traitCollection];
            msg_send![traits, userInterfaceStyle]
        };
        match style {
            UIUserInterfaceStyleDark => return SystemTheme::Dark,
            UIUserInterfaceStyleLight => return SystemTheme::Light,
            _ => {}
        }
    }
    SystemTheme::Unknown
}

/// The interface orientation of the app (its active window scene's, from
/// iOS 27). Translation of `UIKit_GetInterfaceOrientation()`.
#[cfg(not(any(target_os = "tvos", target_os = "visionos")))]
pub(crate) fn get_interface_orientation() -> UIInterfaceOrientation {
    if crate::core::apple::foundation::available!(ios(27, 0)) {
        let window_scene = get_active_window_scene();
        if !window_scene.is_null() {
            // SAFETY: UIWindowScene's effective geometry (iOS 26).
            return unsafe {
                let geometry: Id = msg_send![window_scene, effectiveGeometry];
                msg_send![geometry, interfaceOrientation]
            };
        }
        return sys::UIInterfaceOrientationUnknown;
    }

    // (deprecated since iOS 13)
    // SAFETY: a UIApplication property.
    unsafe { msg_send![shared_application(), statusBarOrientation] }
}

/// The app's active window scene, else a foreground one, else any (nil if
/// it has none, and before iOS 13). Translation of
/// `UIKit_GetActiveWindowScene()`.
pub(crate) fn get_active_window_scene() -> Id {
    if crate::core::apple::foundation::available!(ios(13, 0), tvos(13, 0)) {
        // SAFETY: UIApplication's connected scenes (an NSSet), and the
        // scenes' properties.
        unsafe {
            let connected: Id = msg_send![shared_application(), connectedScenes];
            let all: Id = msg_send![connected, allObjects];
            let window_scenes: Vec<Id> = crate::core::apple::foundation::ns_array_items(all)
                .into_iter()
                .filter(|&scene| crate::core::apple::objc::is_kind_of(scene, class!(UIWindowScene)))
                .collect();
            let state = |scene: Id| -> crate::core::apple::objc::NSInteger {
                msg_send![scene, activationState]
            };

            // First, try to find an active foreground scene
            if let Some(&scene) = window_scenes
                .iter()
                .find(|&&s| state(s) == UISceneActivationStateForegroundActive)
            {
                return scene;
            }

            // If no active scene, return any foreground scene
            if let Some(&scene) = window_scenes
                .iter()
                .find(|&&s| state(s) == UISceneActivationStateForegroundInactive)
            {
                return scene;
            }

            // Last resort: return first window scene
            if let Some(&scene) = window_scenes.first() {
                return scene;
            }
        }
    }

    crate::core::apple::objc::NIL
}

/// The frame of a window's view: its `UIWindow`'s bounds (the screen's
/// before it has one), turned to the interface's orientation (or to
/// landscape for a window that supports no portrait one) when it covers
/// the screen. Translation of `UIKit_ComputeViewFrame()`.
#[cfg(not(target_os = "visionos"))]
pub(crate) fn compute_view_frame(window: WindowID, screen: Id) -> CGRect {
    let data = window::window_data(window);
    // SAFETY: UIScreen's and UIWindow's bounds.
    let screen_bounds: CGRect = unsafe { msg_send![screen, bounds] };
    #[allow(unused_mut)]
    let mut frame = screen_bounds;

    /* Use the UIWindow bounds instead of the UIScreen bounds, when possible.
     * The uiwindow bounds may be smaller than the screen bounds when Split View
     * is used on an iPad. */
    if let Some(data) = data.filter(|d| !d.uiwindow().is_null()) {
        // SAFETY: as above.
        frame = unsafe { msg_send![data.uiwindow(), bounds] };
    }

    #[cfg(not(target_os = "tvos"))]
    {
        /* iOS 10 seems to have a bug where, in certain conditions, putting the
         * device to sleep with the a landscape-only app open, re-orienting the
         * device to portrait, and turning it back on will result in the screen
         * bounds returning portrait orientation despite the app being in landscape.
         * This is a workaround until a better solution can be found.
         * https://bugzilla.libsdl.org/show_bug.cgi?id=3505
         * https://bugzilla.libsdl.org/show_bug.cgi?id=3465
         * https://forums.developer.apple.com/thread/65337 */

        let orient = get_interface_orientation();

        let landscape = sys::UIInterfaceOrientationIsLandscape(orient)
            || (window::get_supported_orientations(window)
                & (sys::UIInterfaceOrientationMaskPortrait
                    | sys::UIInterfaceOrientationMaskPortraitUpsideDown))
                == 0;
        let fullscreen = screen_bounds == frame;

        /* The orientation flip doesn't make sense when the window is smaller
         * than the screen (iPad Split View, for example). */
        if fullscreen && (landscape != (frame.size.width > frame.size.height)) {
            // (as upstream, through a float)
            let height = frame.size.width as f32;
            frame.size.width = frame.size.height;
            frame.size.height = height as f64;
        }
    }

    frame
}

/// The frame of a window's view on visionOS: the window's size. Translation
/// of `UIKit_ComputeViewFrame()` (visionOS).
#[cfg(target_os = "visionos")]
pub(crate) fn compute_view_frame(window: WindowID) -> CGRect {
    // View origin is always (0,0) relative to the UIWindow.
    // window->x/y are screen-level positions (often SDL_WINDOWPOS_UNDEFINED).
    let (w, h) =
        crate::video::core::with_window(window, |w| (w.core.w, w.core.h)).unwrap_or_default();
    CGRect::new(0.0, 0.0, w as f64, h as f64)
}

/// Let the game controllers' events (or not) reach the UIKit windows of
/// every window scene. Translation of `UIKit_SetGameControllerInteraction()`,
/// which the MFi joystick driver calls when it starts and quits.
pub(crate) fn set_game_controller_interaction(enabled: bool) {
    if crate::core::apple::foundation::available!(ios(13, 0), tvos(13, 0)) {
        // SAFETY: UIApplication's connected scenes (an NSSet) and the
        // window scenes' windows (an NSArray).
        unsafe {
            let connected: Id = msg_send![shared_application(), connectedScenes];
            let all: Id = msg_send![connected, allObjects];
            for scene in crate::core::apple::foundation::ns_array_items(all) {
                if !crate::core::apple::objc::is_kind_of(scene, class!(UIWindowScene)) {
                    continue;
                }

                let window_scene = scene;
                let windows: Id = msg_send![window_scene, windows];
                for window in crate::core::apple::foundation::ns_array_items(windows) {
                    set_view_game_controller_interaction(window, enabled);
                }
            }
        }
    }
}

/// Add (or remove) a view's interaction for the game controllers' events
/// (`GCEventInteraction`, iOS 18 and visionOS 2). Translation of
/// `UIKit_SetViewGameControllerInteraction()`. Note: GameController is
/// loaded at run time here (the MFi joystick driver loads it), so without
/// it there's no interaction to add.
pub(crate) fn set_view_game_controller_interaction(view: Id, enabled: bool) {
    #[cfg(any(target_os = "ios", target_os = "visionos"))]
    if crate::core::apple::foundation::available!(ios(18, 0), visionos(2, 0)) {
        let interaction_class = class!(GCEventInteraction);
        if interaction_class.is_null() {
            return;
        }
        // SAFETY: GCEventInteraction's alloc and init (owned) and
        // properties, UIView's interactions (an NSArray) and methods.
        unsafe {
            if enabled {
                let interaction: Id = msg_send![interaction_class, alloc];
                let interaction =
                    crate::core::apple::objc::StrongPtr::from_owned(msg_send![interaction, init]);
                let () =
                    msg_send![interaction.get(), setHandledEventTypes: sys::GCUIEventTypeGamepad];
                let () = msg_send![view, addInteraction: interaction.get()];
            } else {
                let interactions: Id = msg_send![view, interactions];
                for entry in crate::core::apple::foundation::ns_array_items(interactions) {
                    if crate::core::apple::objc::is_kind_of(entry, interaction_class) {
                        let interaction = entry;
                        let types: sys::GCUIEventTypes = msg_send![interaction, handledEventTypes];
                        if types == sys::GCUIEventTypeGamepad {
                            let () = msg_send![view, removeInteraction: interaction];
                            break;
                        }
                    }
                }
            }
        }
    }
    #[cfg(not(any(target_os = "ios", target_os = "visionos")))]
    let _ = (view, enabled);
}

/// Make the focused window's view controller update its home indicator
/// and the screen edges deferring system gestures. Translation of
/// `UIKit_ForceUpdateHomeIndicator()`.
pub(crate) fn force_update_home_indicator() {
    #[cfg(not(target_os = "tvos"))]
    {
        // Force the main SDL window to re-evaluate home indicator state
        let Some(focus) = crate::events::keyboard::keyboard_focus() else {
            return;
        };
        if let Some(data) = window::window_data(focus) {
            // SAFETY: NSObject's method, with UIViewController's selectors
            // (which take no argument).
            unsafe {
                let () = msg_send![data.viewcontroller(),
                    performSelectorOnMainThread: crate::core::apple::objc::sel!(setNeedsUpdateOfHomeIndicatorAutoHidden)
                    withObject: crate::core::apple::objc::NIL
                    waitUntilDone: crate::core::apple::objc::NO];
                let () = msg_send![data.viewcontroller(),
                    performSelectorOnMainThread: crate::core::apple::objc::sel!(setNeedsUpdateOfScreenEdgesDeferringSystemGestures)
                    withObject: crate::core::apple::objc::NIL
                    waitUntilDone: crate::core::apple::objc::NO];
            }
        }
    }
}

impl VideoDriver for UIKitVideo {
    fn caps(&self) -> DeviceCaps {
        DeviceCaps::SENDS_FULLSCREEN_DIMENSIONS
    }

    fn video_init(&self) -> Result<()> {
        // (UIKit_CreateDevice() sets device->system_theme, which exists
        // once the driver is chosen; no event is sent. It also sets
        // gl_config.accelerated to 1, which SDL_VideoInit() resets.)
        let theme = autoreleasepool(get_system_theme);
        let _ = crate::video::core::with_device(|v| v.system_theme = theme);
        video_init(&self.data)
    }

    fn video_quit(&self) {
        video_quit(&self.data)
    }

    fn display_usable_bounds(&self, display: DisplayID) -> Option<Result<Rect>> {
        Some(modes::get_display_usable_bounds(display))
    }

    fn display_modes(&self, display: DisplayID) -> Option<()> {
        modes::get_display_modes(display);
        Some(())
    }

    fn set_display_mode(&self, display: DisplayID, mode: &DisplayMode) -> Option<Result<()>> {
        Some(modes::set_display_mode(display, mode))
    }

    fn create_window(&self, window: WindowID, create_props: &Properties) -> Option<Result<()>> {
        Some(window::create_window(window, create_props))
    }

    fn set_window_title(&self, window: WindowID) -> Option<()> {
        window::set_window_title(window);
        Some(())
    }

    fn set_window_size(&self, window: WindowID) -> Option<()> {
        window::set_window_size(window);
        Some(())
    }

    fn show_window(&self, window: WindowID) -> Option<()> {
        window::show_window(window);
        Some(())
    }

    fn hide_window(&self, window: WindowID) -> Option<()> {
        window::hide_window(window);
        Some(())
    }

    fn raise_window(&self, window: WindowID) -> Option<()> {
        window::raise_window(window);
        Some(())
    }

    fn set_window_bordered(&self, window: WindowID, bordered: bool) -> Option<()> {
        window::set_window_bordered(window, bordered);
        Some(())
    }

    fn set_window_fullscreen(
        &self,
        window: WindowID,
        _display: DisplayID,
        fullscreen: FullscreenOp,
    ) -> Option<FullscreenResult> {
        Some(window::set_window_fullscreen(window, fullscreen))
    }

    fn destroy_window(&self, window: WindowID) -> Option<()> {
        window::destroy_window(window);
        Some(())
    }

    fn window_size_in_pixels(&self, window: WindowID) -> Option<(i32, i32)> {
        Some(window::get_window_size_in_pixels(window))
    }

    fn implements_window_op(&self, op: WindowOp) -> bool {
        matches!(op, WindowOp::SetBordered | WindowOp::SetFullscreen)
    }

    // SDL_IPHONE_KEYBOARD
    fn has_screen_keyboard_support(&self) -> Option<bool> {
        Some(viewcontroller::has_screen_keyboard_support())
    }

    fn start_text_input(&self, window: WindowID, props: Option<&Properties>) -> Option<Result<()>> {
        Some(viewcontroller::uikit_start_text_input(window, props))
    }

    fn stop_text_input(&self, window: WindowID) -> Option<Result<()>> {
        Some(viewcontroller::uikit_stop_text_input(window))
    }

    fn set_text_input_properties(
        &self,
        window: WindowID,
        props: Option<&Properties>,
    ) -> Option<()> {
        viewcontroller::uikit_set_text_input_properties(window, props);
        Some(())
    }

    fn update_text_input_area(&self, window: WindowID) -> Option<Result<()>> {
        Some(viewcontroller::uikit_update_text_input_area(window))
    }

    fn implements_clipboard_text(&self) -> bool {
        true
    }

    fn set_clipboard_text(&self, text: &str) -> Option<Result<()>> {
        Some(clipboard::set_clipboard_text(&self.data, text))
    }

    fn clipboard_text(&self) -> Option<Option<String>> {
        Some(Some(clipboard::get_clipboard_text()))
    }

    fn has_clipboard_text(&self) -> Option<bool> {
        Some(clipboard::has_clipboard_text())
    }

    // SDL_VIDEO_OPENGL_ES || SDL_VIDEO_OPENGL_ES2 (no GL_UnloadLibrary,
    // GL_SetSwapInterval or GL_GetSwapInterval)

    #[cfg(all(any(target_os = "ios", target_os = "tvos"), not(target_abi = "macabi")))]
    fn implements_gl_contexts(&self) -> bool {
        true
    }
    #[cfg(all(any(target_os = "ios", target_os = "tvos"), not(target_abi = "macabi")))]
    fn gl_load_library(&self, path: Option<&str>) -> Option<Result<()>> {
        Some(opengles::gl_load_library(path))
    }
    #[cfg(all(any(target_os = "ios", target_os = "tvos"), not(target_abi = "macabi")))]
    fn gl_get_proc_address(
        &self,
        proc_name: &str,
    ) -> Option<Option<std::ptr::NonNull<std::ffi::c_void>>> {
        Some(opengles::gl_get_proc_address(proc_name))
    }
    #[cfg(all(any(target_os = "ios", target_os = "tvos"), not(target_abi = "macabi")))]
    fn gl_create_context(
        &self,
        window: WindowID,
    ) -> Option<Result<crate::video::gl::RawGlContext>> {
        Some(opengles::gl_create_context(window))
    }
    #[cfg(all(any(target_os = "ios", target_os = "tvos"), not(target_abi = "macabi")))]
    fn gl_make_current(
        &self,
        window: Option<WindowID>,
        context: Option<crate::video::gl::RawGlContext>,
    ) -> Option<Result<()>> {
        Some(opengles::gl_make_current(window, context))
    }
    #[cfg(all(any(target_os = "ios", target_os = "tvos"), not(target_abi = "macabi")))]
    fn gl_swap_window(&self, window: WindowID) -> Option<Result<()>> {
        Some(opengles::gl_swap_window(window))
    }
    #[cfg(all(any(target_os = "ios", target_os = "tvos"), not(target_abi = "macabi")))]
    fn gl_destroy_context(&self, context: crate::video::gl::RawGlContext) -> Option<Result<()>> {
        Some(opengles::gl_destroy_context(context))
    }

    // SDL_VIDEO_VULKAN (no Vulkan_GetPresentationSupport: presenting is
    // always supported)

    fn implements_vulkan_surfaces(&self) -> bool {
        true
    }
    fn vulkan_load_library(&self, path: Option<&str>) -> Option<Result<()>> {
        Some(vulkan::vulkan_load_library(path))
    }
    fn vulkan_unload_library(&self) -> Option<()> {
        vulkan::vulkan_unload_library();
        Some(())
    }
    fn vulkan_loader_path(&self) -> Option<Option<String>> {
        Some(vulkan::vulkan_loader_path())
    }
    fn vulkan_get_instance_proc_addr(&self) -> Option<usize> {
        vulkan::vk_get_instance_proc_addr()
    }
    fn vulkan_instance_extensions(&self) -> Option<Vec<&'static str>> {
        Some(vulkan::vulkan_get_instance_extensions())
    }
    fn vulkan_create_surface(
        &self,
        window: WindowID,
        instance: usize,
        allocator: usize,
    ) -> Option<Result<u64>> {
        Some(vulkan::vulkan_create_surface(window, instance, allocator))
    }
    fn vulkan_destroy_surface(
        &self,
        instance: usize,
        surface: u64,
        allocator: usize,
    ) -> Option<()> {
        vulkan::vulkan_destroy_surface(instance, surface, allocator);
        Some(())
    }

    // SDL_VIDEO_METAL

    fn implements_metal_views(&self) -> bool {
        true
    }
    fn metal_create_view(&self, window: WindowID) -> Option<Option<usize>> {
        Some(metalview::metal_create_view(window))
    }
    fn metal_destroy_view(&self, view: usize) -> Option<()> {
        metalview::metal_destroy_view(view);
        Some(())
    }
    fn metal_get_layer(&self, view: usize) -> Option<Option<usize>> {
        Some(metalview::metal_get_layer(view))
    }

    fn pump_events(&self) -> Option<()> {
        events::pump_events();
        Some(())
    }

    fn suspend_screen_saver(&self, suspend: bool) -> Option<Result<()>> {
        Some(suspend_screen_saver(suspend))
    }

    // (SDL_InitGCMouse() sets SDL_GetMouse()->SetRelativeMouseMode, and
    // SDL_QuitGCMouse() clears it)
    fn set_relative_mouse_mode(&self, enabled: bool) -> Option<Result<()>> {
        events::has_gc_mouse_relative_mode().then(|| events::set_gc_mouse_relative_mode(enabled))
    }

    fn implements_mouse_feature(&self, feature: MouseFeature) -> bool {
        feature == MouseFeature::SetRelativeMouseMode && events::has_gc_mouse_relative_mode()
    }
}
