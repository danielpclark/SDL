// Rust translation of src/video/uikit/SDL_uikitmetalview.m and
// SDL_uikitmetalview.h from Simple DirectMedia Layer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.
//
// @author Mark Callow, www.edgewise-consulting.com.
//
// Thanks to @slime73 on GitHub for their gist showing how to add a CAMetalLayer
// backed view.

//! Metal views: `SDL_uikitmetalview`, an [`SDL_uikitview`](super::view)
//! backed by a `CAMetalLayer` (`SDL_Metal_CreateView()`), which becomes its
//! window's view (its view controller's, as any `SDL_uikitview` does when
//! it joins a window) and which the Metal renderer and the GPU API's
//! swapchains draw to. Its drawable size follows its bounds at its scale
//! (the screen's native scale for a window of high pixel density), each
//! change a `SDL_EVENT_WINDOW_METAL_VIEW_RESIZED` event; its tag is
//! `SDL_METALVIEW_TAG`, which the window's
//! `SDL_PROP_WINDOW_UIKIT_METAL_VIEW_TAG_NUMBER` names, so the renderer
//! finds a window's Metal view.

use super::view::{get_sdl_window, set_sdl_window, uikitview_class};
use super::window::{window_data, SDL_METALVIEW_TAG};
use crate::core::apple::foundation::{CGFloat, CGRect, CGSize};
use crate::core::apple::objc::{
    autoreleasepool, class, imp, is_kind_of, msg_send, msg_send_super, release, sel, Class,
    ClassBuilder, DeclaredClass, Id, Sel,
};
use crate::events::window::{send_window_event, WindowFlags};
use crate::events::{EventType, WindowID};
use crate::video::core::with_window;

// CAMetalLayer is QuartzCore's.
#[link(name = "QuartzCore", kind = "framework")]
unsafe extern "C" {}

static SDL_UIKITMETALVIEW: DeclaredClass = DeclaredClass::new();

/// The `SDL_uikitmetalview` class (`@interface SDL_uikitmetalview :
/// SDL_uikitview`).
fn uikitmetalview_class() -> Class {
    SDL_UIKITMETALVIEW.get(|| {
        let mut b = ClassBuilder::new(c"SDL_uikitmetalview", uikitview_class())
            .expect("SDL_uikitmetalview");
        // SAFETY: the implementations have the signatures of the encodings.
        unsafe {
            b.add_class_method(
                sel!(layerClass),
                imp!(metalview_layer_class as extern "C" fn(Id, Sel) -> Class),
                c"#@:",
            );
            b.add_method(
                sel!(layoutSubviews),
                imp!(metalview_layout_subviews as extern "C" fn(Id, Sel)),
                c"v@:",
            );
            b.add_method(
                sel!(updateDrawableSize),
                imp!(metalview_update_drawable_size as extern "C" fn(Id, Sel)),
                c"v@:",
            );
        }
        b.register()
    })
}

/// Translation of `+[SDL_uikitmetalview layerClass]`: a Metal-compatible
/// layer.
extern "C" fn metalview_layer_class(_this: Id, _sel: Sel) -> Class {
    class!(CAMetalLayer)
}

/// Translation of `-[SDL_uikitmetalview initWithFrame:scale:]` on an
/// allocated view.
///
/// # Safety
///
/// `obj` must be an allocated `SDL_uikitmetalview`.
unsafe fn init_metal_view(obj: Id, frame: CGRect, scale: CGFloat) -> Id {
    // SAFETY: SDL_uikitview's initializer, then UIView's properties.
    unsafe {
        let this: Id = msg_send_super![obj, uikitmetalview_class(), initWithFrame: frame];
        if !this.is_null() {
            let () = msg_send![this, setTag: SDL_METALVIEW_TAG];
            let layer: Id = msg_send![this, layer];
            let () = msg_send![layer, setContentsScale: scale];
            let () = msg_send![this, updateDrawableSize];
        }
        this
    }
}

/// Translation of `-[SDL_uikitmetalview layoutSubviews]`: set the size of
/// the metal drawables when the view is resized.
extern "C" fn metalview_layout_subviews(this: Id, _sel: Sel) {
    // SAFETY: UIView's method, then the view's own.
    unsafe {
        let () = msg_send_super![this, uikitmetalview_class(), layoutSubviews];
        let () = msg_send![this, updateDrawableSize];
    }
}

/// Translation of `-[SDL_uikitmetalview updateDrawableSize]`.
extern "C" fn metalview_update_drawable_size(this: Id, _sel: Sel) {
    // SAFETY: the view's bounds and its layer, a CAMetalLayer.
    unsafe {
        let layer: Id = msg_send![this, layer];
        let bounds: CGRect = msg_send![this, bounds];
        let scale: CGFloat = msg_send![layer, contentsScale];
        let mut size = bounds.size;
        size.width *= scale;
        size.height *= scale;

        // Skip invalid sizes (can happen on visionOS before scene geometry is applied)
        if size.width <= 0.0 || size.height <= 0.0 {
            return;
        }

        let drawable_size: CGSize = msg_send![layer, drawableSize];
        if drawable_size.width != size.width || drawable_size.height != size.height {
            let () = msg_send![layer, setDrawableSize: size];
            // (a view out of its window has none: SDL_SendWindowEvent()
            // ignores a NULL window)
            if let Some(window) = get_sdl_window(this) {
                send_window_event(window, EventType::WINDOW_METAL_VIEW_RESIZED, 0, 0);
            }
        }
    }
}

/// Translation of `UIKit_Metal_CreateView()`: the view, as an
/// `SDL_MetalView` handle owning a reference.
pub(crate) fn metal_create_view(window: WindowID) -> Option<usize> {
    autoreleasepool(|| {
        let data = window_data(window)?;
        let flags = with_window(window, |w| w.flags()).ok()?;
        let mut scale: CGFloat = 1.0;

        if flags.contains(WindowFlags::HIGH_PIXEL_DENSITY) {
            /* Set the scale to the natural scale factor of the screen - then
             * the backing dimensions of the Metal view will match the pixel
             * dimensions of the screen rather than the dimensions in points
             * yielding high resolution on retine displays.
             */
            #[cfg(not(target_os = "visionos"))]
            {
                // SAFETY: the window's UIWindow and its screen.
                scale = unsafe {
                    let screen: Id = msg_send![data.uiwindow(), screen];
                    msg_send![screen, nativeScale]
                };
            }
            // VisionOS doesn't use the concept of "nativeScale" like other iOS devices.
            // We use a fixed scale factor of 2.0 to achieve better pixel density.
            // This is because VisionOS presents a virtual 1280x720 "screen", but we need
            // to render at a higher resolution for optimal visual quality.
            // TODO: Consider making this configurable or determining it dynamically
            // based on the specific visionOS device capabilities.
            #[cfg(target_os = "visionos")]
            {
                scale = 2.0;
            }
        }

        // SAFETY: the window's bounds, and the view class declared above.
        let metalview = unsafe {
            let bounds: CGRect = msg_send![data.uiwindow(), bounds];
            let obj: Id = msg_send![uikitmetalview_class(), alloc];
            init_metal_view(obj, bounds, scale)
        };
        if metalview.is_null() {
            // (SDL_OutOfMemory())
            return None;
        }

        set_sdl_window(metalview, Some(window));

        // (CFBridgingRetain: the handle keeps the +1 from alloc/init)
        Some(metalview as usize)
    })
}

/// Translation of `UIKit_Metal_DestroyView()`.
pub(crate) fn metal_destroy_view(view: usize) {
    autoreleasepool(|| {
        let metalview = view as Id;
        // SAFETY: a view from metal_create_view(), whose reference the
        // handle owns (CFBridgingRelease).
        unsafe {
            if is_kind_of(metalview, uikitmetalview_class()) {
                set_sdl_window(metalview, None);
            }
            release(metalview);
        }
    })
}

/// Translation of `UIKit_Metal_GetLayer()`.
pub(crate) fn metal_get_layer(view: usize) -> Option<usize> {
    autoreleasepool(|| {
        let uiview = view as Id;
        // SAFETY: a view from metal_create_view().
        let layer: Id = unsafe { msg_send![uiview, layer] };
        (!layer.is_null()).then_some(layer as usize)
    })
}
