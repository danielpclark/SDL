// Rust translation of src/video/cocoa/SDL_cocoametalview.m and
// SDL_cocoametalview.h from Simple DirectMedia Layer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.
//
// @author Mark Callow, www.edgewise-consulting.com.
//
// Thanks to @slime73 on GitHub for their gist showing how to add a CAMetalLayer
// backed view.

//! Metal views: `SDL3_cocoametalview`, an `NSView` backed by a
//! `CAMetalLayer`, added as a subview of the window's content view
//! (`SDL_Metal_CreateView()`), which the Vulkan surfaces of MoltenVK and
//! the Metal renderers draw to. The view's properties (`highDPI`,
//! `sdlWindowID`) and its window event watch are a Rust structure the view
//! points to.

use std::ffi::c_void;

use super::window::{window_data, SDL_METALVIEW_TAG};
use crate::core::apple::foundation::{NSPoint, NSRect};
use crate::core::apple::objc::{
    autoreleasepool, class, encoding, imp, ivar_ptr, msg_send, msg_send_super, release, sel, Bool,
    Class, ClassBuilder, DeclaredClass, Id, NSInteger, NSUInteger, Sel, WeakId, NIL, YES,
};
use crate::events::queue::EventWatch;
use crate::events::window::{add_window_event_watch, WindowEventWatchPriority, WindowFlags};
use crate::events::{Event, EventType, WindowID};
use crate::hints;
use crate::video::core::with_window;

// CAMetalLayer is QuartzCore's.
#[link(name = "QuartzCore", kind = "framework")]
unsafe extern "C" {}

/// `NSViewWidthSizable`.
const NSViewWidthSizable: NSUInteger = 2;
/// `NSViewHeightSizable`.
const NSViewHeightSizable: NSUInteger = 16;

/// The properties of `SDL3_cocoametalview`.
struct MetalViewState {
    high_dpi: bool,
    sdl_window_id: WindowID,
    /// The `SDL_MetalViewEventWatch` registration (removed in dealloc).
    watch: Option<EventWatch>,
}

/// Translation of `SDL_MetalViewEventWatch()`.
///
/// Update the drawable size when SDL receives a size changed event for
/// the window that contains the metal view. It would be nice to use
/// - (void)resizeWithOldSuperviewSize:(NSSize)oldSize and
/// - (void)viewDidChangeBackingProperties instead, but SDL's size change
///   events don't always happen in the same frame (for example when a
///   resizable window exits a fullscreen Space via the user pressing the OS
///   exit-space button).
fn metal_view_event_watch(view: WeakId, event: &Event) {
    if let Event::Window(w) = event {
        if w.event_type == EventType::WINDOW_PIXEL_SIZE_CHANGED {
            autoreleasepool(|| {
                // SAFETY: the view, alive while its watch is registered.
                if unsafe { view_state(view.0) }.sdl_window_id == w.window_id {
                    // SAFETY: the method declared below.
                    let _: () = unsafe { msg_send![view.0, updateDrawableSize] };
                }
            })
        }
    }
}

static SDL3_COCOAMETALVIEW: DeclaredClass = DeclaredClass::new();

/// The `SDL3_cocoametalview` class (`@implementation SDL3_cocoametalview`).
fn sdl3_cocoametalview_class() -> Class {
    SDL3_COCOAMETALVIEW.get(|| {
        let mut b =
            ClassBuilder::new(c"SDL3_cocoametalview", class!(NSView)).expect("SDL3_cocoametalview");
        b.add_ivar::<*mut c_void>(c"_state", c"^v");
        // SAFETY: the implementations have the signatures of the encodings.
        unsafe {
            b.add_class_method(
                sel!(layerClass),
                imp!(metalview_layer_class as extern "C" fn(Id, Sel) -> Class),
                c"#@:",
            );
            b.add_method(
                sel!(wantsUpdateLayer),
                imp!(metalview_wants_update_layer as extern "C" fn(Id, Sel) -> Bool),
                &encoding("B@:"),
            );
            b.add_method(
                sel!(makeBackingLayer),
                imp!(metalview_make_backing_layer as extern "C" fn(Id, Sel) -> Id),
                c"@@:",
            );
            b.add_method(
                sel!(tag),
                imp!(metalview_tag as extern "C" fn(Id, Sel) -> NSInteger),
                c"q@:",
            );
            b.add_method(
                sel!(updateDrawableSize),
                imp!(metalview_update_drawable_size as extern "C" fn(Id, Sel)),
                c"v@:",
            );
            b.add_method(
                sel!(hitTest:),
                imp!(metalview_hit_test as extern "C" fn(Id, Sel, NSPoint) -> Id),
                c"@@:{CGPoint=dd}",
            );
            b.add_method(
                sel!(dealloc),
                imp!(metalview_dealloc as extern "C" fn(Id, Sel)),
                c"v@:",
            );
        }
        b.register()
    })
}

/// The properties of an `SDL3_cocoametalview`.
///
/// # Safety
///
/// `this` must be an initialized `SDL3_cocoametalview`.
unsafe fn view_state<'a>(this: Id) -> &'a MetalViewState {
    // SAFETY: the caller's contract; `_state` is set by init_metal_view()
    // and freed by dealloc.
    unsafe {
        &*(*ivar_ptr::<*mut c_void>(this, sdl3_cocoametalview_class(), c"_state")
            as *const MetalViewState)
    }
}

/// Translation of `+[SDL3_cocoametalview layerClass]`: a Metal-compatible
/// layer.
extern "C" fn metalview_layer_class(_this: Id, _sel: Sel) -> Class {
    class!(CAMetalLayer)
}

/// Translation of `-[SDL3_cocoametalview wantsUpdateLayer]`: the view
/// draws using a backing layer instead of drawRect.
extern "C" fn metalview_wants_update_layer(_this: Id, _sel: Sel) -> Bool {
    YES
}

/// Translation of `-[SDL3_cocoametalview makeBackingLayer]`: when the
/// wantsLayer property is set to YES, this method will be invoked to return
/// a layer instance.
extern "C" fn metalview_make_backing_layer(this: Id, _sel: Sel) -> Id {
    // SAFETY: the view's class and its layer class's factory method.
    unsafe {
        let cls: Class = msg_send![this, class];
        let layer_class: Class = msg_send![cls, layerClass];
        msg_send![layer_class, layer]
    }
}

/// Translation of `-[SDL3_cocoametalview tag]`.
extern "C" fn metalview_tag(_this: Id, _sel: Sel) -> NSInteger {
    SDL_METALVIEW_TAG
}

/// Translation of `-[SDL3_cocoametalview updateDrawableSize]`.
extern "C" fn metalview_update_drawable_size(this: Id, _sel: Sel) {
    // SAFETY: an SDL3_cocoametalview, whose layer is a CAMetalLayer.
    unsafe {
        let metal_layer: Id = msg_send![this, layer];
        let bounds: NSRect = msg_send![this, bounds];
        let size = bounds.size;
        let mut backing_size = size;

        if view_state(this).high_dpi {
            /* Note: NSHighResolutionCapable must be set to true in the app's
             * Info.plist in order for the backing size to be high res.
             */
            backing_size = msg_send![this, convertSizeToBacking: size];
        }

        let _: () = msg_send![metal_layer, setContentsScale: backing_size.height / size.height];
        let _: () = msg_send![metal_layer, setDrawableSize: backing_size];
    }
}

/// Translation of `-[SDL3_cocoametalview hitTest:]`.
extern "C" fn metalview_hit_test(_this: Id, _sel: Sel, _point: NSPoint) -> Id {
    NIL
}

/// `-[SDL3_cocoametalview dealloc]`.
extern "C" fn metalview_dealloc(this: Id, _sel: Sel) {
    // SAFETY: the state init_metal_view() leaked, freed once (removing
    // the event watch), then the superclass's dealloc.
    unsafe {
        let p = ivar_ptr::<*mut c_void>(this, sdl3_cocoametalview_class(), c"_state");
        if !(*p).is_null() {
            drop(Box::from_raw(*p as *mut MetalViewState));
            *p = std::ptr::null_mut();
        }
        let _: () = msg_send_super![this, sdl3_cocoametalview_class(), dealloc];
    }
}

/// Translation of `-[SDL3_cocoametalview initWithFrame:highDPI:windowID:opaque:]`
/// on an allocated view.
///
/// # Safety
///
/// `obj` must be an allocated `SDL3_cocoametalview`.
unsafe fn init_metal_view(
    obj: Id,
    frame: NSRect,
    high_dpi: bool,
    window_id: WindowID,
    opaque: bool,
) -> Id {
    // SAFETY: NSView's initializer, then the view's own properties.
    unsafe {
        let this: Id = msg_send_super![obj, sdl3_cocoametalview_class(), initWithFrame: frame];
        if !this.is_null() {
            let state = Box::new(MetalViewState {
                high_dpi,
                sdl_window_id: window_id,
                watch: None,
            });
            *ivar_ptr::<*mut c_void>(this, sdl3_cocoametalview_class(), c"_state") =
                Box::into_raw(state) as *mut c_void;
            let _: () = msg_send![this, setWantsLayer: YES];

            // Allow resize.
            let _: () =
                msg_send![this, setAutoresizingMask: NSViewWidthSizable | NSViewHeightSizable];

            let layer: Id = msg_send![this, layer];
            let _: () = msg_send![layer, setOpaque: Bool::from(opaque)];

            if hints::get_bool(hints::VIDEO_METAL_AUTO_RESIZE_DRAWABLE, true) {
                let view = WeakId(this);
                let watch = add_window_event_watch(WindowEventWatchPriority::Early, move |e| {
                    metal_view_event_watch(view, e)
                });
                let p = *ivar_ptr::<*mut c_void>(this, sdl3_cocoametalview_class(), c"_state");
                (*(p as *mut MetalViewState)).watch = Some(watch);
            }

            let _: () = msg_send![this, updateDrawableSize];
        }
        this
    }
}

/// Translation of `Cocoa_Metal_CreateView()`: the view, as an
/// `SDL_MetalView` handle owning a reference.
pub(crate) fn metal_create_view(window: WindowID) -> Option<usize> {
    autoreleasepool(|| {
        let data = window_data(window)?;
        let flags = with_window(window, |w| w.flags()).ok()?;
        // SAFETY: the window's content view, and the view class declared above.
        unsafe {
            let view: Id = msg_send![data.nswindow(), contentView];
            let high_dpi = flags.contains(WindowFlags::HIGH_PIXEL_DENSITY);
            let opaque = !flags.contains(WindowFlags::TRANSPARENT);
            let window_id = window;

            let frame: NSRect = msg_send![view, frame];
            let newview = init_metal_view(
                super::alloc(sdl3_cocoametalview_class()),
                frame,
                high_dpi,
                window_id,
                opaque,
            );
            if newview.is_null() {
                // (SDL_OutOfMemory())
                return None;
            }

            let _: () = msg_send![view, addSubview: newview];

            // Make sure the drawable size is up to date after attaching the view.
            let _: () = msg_send![newview, updateDrawableSize];

            // (CFBridgingRetain: the handle keeps the +1 from alloc/init)
            Some(newview as usize)
        }
    })
}

/// Translation of `Cocoa_Metal_DestroyView()`.
pub(crate) fn metal_destroy_view(view: usize) {
    autoreleasepool(|| {
        let metalview = view as Id;
        // SAFETY: a view from metal_create_view(), whose reference the
        // handle owns (CFBridgingRelease).
        unsafe {
            let _: () = msg_send![metalview, removeFromSuperview];
            release(metalview);
        }
    })
}

/// Translation of `Cocoa_Metal_GetLayer()`.
pub(crate) fn metal_get_layer(view: usize) -> Option<usize> {
    autoreleasepool(|| {
        let cocoaview = view as Id;
        // SAFETY: a view from metal_create_view().
        let layer: Id = unsafe { msg_send![cocoaview, layer] };
        (!layer.is_null()).then_some(layer as usize)
    })
}
