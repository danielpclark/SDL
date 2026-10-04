// Rust translation of src/video/x11/SDL_x11dyn.c, SDL_x11dyn.h and
// SDL_x11sym.h from Simple DirectMedia Layer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! Loading Xlib and the X extension libraries at run time.
//!
//! Upstream's `SDL_x11sym.h` lists every function SDL calls, grouped in
//! modules (`SDL_X11_MODULE(BASEXLIB)`, `XRANDR`, ...); `SDL_x11dyn.c`
//! looks each one up in all of the libraries it opened and clears a
//! module's `SDL_X11_HAVE_*` flag when one of its symbols is missing. Here a
//! module is a struct of function pointers that exists (`Some`) exactly
//! when all of its symbols were found; the base Xlib module is required.
//!
//! The libraries are those of a build of upstream with dynamic X11
//! loading: `libX11.so.6`, `libXext.so.6`, `libXcursor.so.1`, `libXi.so.6`,
//! `libXfixes.so.3`, `libXrandr.so.2`, `libXss.so.1` and `libXtst.so.6`.
//! They are reference counted like `x11_load_refcount`: the video driver
//! and the message box each hold a [`X11Syms`] handle.

#![allow(non_snake_case, non_camel_case_types)] // (the names of SDL_x11sym.h)
#![allow(dead_code)] // (the symbol table lists every function of SDL_x11sym.h)

use std::ffi::{c_char, c_double, c_int, c_long, c_short, c_uchar, c_uint, c_ulong, c_void};
use std::sync::{Arc, Mutex};

use super::sys::*;
use crate::loadso::SharedObject;
// (Option::None, not the X11 None of sys)
use std::option::Option::None;

const DEBUG_DYNAMIC_X11: bool = false;

/// The libraries SDL looks symbols up in (`x11libs[]`), in order.
const X11LIBS: [&str; 8] = [
    "libX11.so.6",     // SDL_VIDEO_DRIVER_X11_DYNAMIC
    "libXext.so.6",    // SDL_VIDEO_DRIVER_X11_DYNAMIC_XEXT
    "libXcursor.so.1", // SDL_VIDEO_DRIVER_X11_DYNAMIC_XCURSOR
    "libXi.so.6",      // SDL_VIDEO_DRIVER_X11_DYNAMIC_XINPUT2
    "libXfixes.so.3",  // SDL_VIDEO_DRIVER_X11_DYNAMIC_XFIXES
    "libXrandr.so.2",  // SDL_VIDEO_DRIVER_X11_DYNAMIC_XRANDR
    "libXss.so.1",     // SDL_VIDEO_DRIVER_X11_DYNAMIC_XSS
    "libXtst.so.6",    // SDL_VIDEO_DRIVER_X11_DYNAMIC_XTEST
];

/// Look a symbol up in the loaded libraries. Translation of `X11_GetSym()`:
/// a missing symbol clears `have_module` ("kill this module").
fn x11_get_sym<F: Copy>(libs: &[SharedObject], fnname: &str, have_module: &mut bool) -> Option<F> {
    let mut found = None;
    for lib in libs {
        // SAFETY: F is the function pointer type declared for `fnname` in
        // the module tables below, matching the C prototype in
        // SDL_x11sym.h; the libraries outlive the pointers (they are kept
        // in the same X11Syms).
        if let Ok(f) = unsafe { lib.function::<F>(fnname) } {
            found = Some((f, lib.name()));
            break;
        }
    }

    if DEBUG_DYNAMIC_X11 {
        match &found {
            Some((_, libname)) => crate::log!("X11: Found '{fnname}' in {libname}"),
            None => crate::log!("X11: Symbol '{fnname}' NOT FOUND!"),
        }
    }

    if found.is_none() {
        *have_module = false; // kill this module.
    }

    found.map(|(f, _)| f)
}

/// Declare a module of `SDL_x11sym.h`: a struct of the module's function
/// pointers that loads when all of them are found.
macro_rules! x11_module {
    ($(#[$doc:meta])* $Name:ident { $($sym:ident: $ty:ty,)* }) => {
        $(#[$doc])*
        pub(crate) struct $Name {
            $(pub(crate) $sym: $ty,)*
        }

        impl $Name {
            /// Look up all of the module's symbols (`SDL_X11_HAVE_*` stays
            /// set only if every one is found).
            fn load(libs: &[SharedObject]) -> Option<$Name> {
                let mut have = true; // default yes
                $( let $sym = x11_get_sym::<$ty>(libs, stringify!($sym), &mut have); )*
                if !have {
                    return None;
                }
                Some($Name { $($sym: $sym?,)* })
            }
        }
    };
}

// "evil function signatures..."
pub(crate) type SDL_X11_XESetWireToEventRetType =
    Option<unsafe extern "C" fn(*mut Display, *mut XEvent, *mut c_void) -> Bool>;
pub(crate) type SDL_X11_XSynchronizeRetType = Option<unsafe extern "C" fn(*mut Display) -> c_int>;
pub(crate) type SDL_X11_XESetEventToWireRetType =
    Option<unsafe extern "C" fn(*mut Display, *mut XEvent, *mut c_void) -> Status>;

x11_module! {
    /// `SDL_X11_MODULE(BASEXLIB)`: the core Xlib functions (required).
    BaseXlib {
        XAllocSizeHints: unsafe extern "C" fn() -> *mut XSizeHints,
        XAllocWMHints: unsafe extern "C" fn() -> *mut XWMHints,
        XAllocClassHint: unsafe extern "C" fn() -> *mut XClassHint,
        XChangePointerControl: unsafe extern "C" fn(*mut Display, Bool, Bool, c_int, c_int, c_int) -> c_int,
        XChangeProperty: unsafe extern "C" fn(*mut Display, Window, Atom, Atom, c_int, c_int, *const c_uchar, c_int) -> c_int,
        XCheckIfEvent: unsafe extern "C" fn(*mut Display, *mut XEvent, XIfEventPredicate, XPointer) -> Bool,
        XClearWindow: unsafe extern "C" fn(*mut Display, Window) -> c_int,
        XCloseDisplay: unsafe extern "C" fn(*mut Display) -> c_int,
        XConvertSelection: unsafe extern "C" fn(*mut Display, Atom, Atom, Atom, Window, Time) -> c_int,
        XCreateBitmapFromData: unsafe extern "C" fn(*mut Display, Drawable, *const c_char, c_uint, c_uint) -> Pixmap,
        XCreateColormap: unsafe extern "C" fn(*mut Display, Window, *mut Visual, c_int) -> Colormap,
        XCreatePixmapCursor: unsafe extern "C" fn(*mut Display, Pixmap, Pixmap, *mut XColor, *mut XColor, c_uint, c_uint) -> Cursor,
        XCreatePixmap: unsafe extern "C" fn(*mut Display, Drawable, c_uint, c_uint, c_uint) -> Pixmap,
        XCreateFontCursor: unsafe extern "C" fn(*mut Display, c_uint) -> Cursor,
        XCreateFontSet: unsafe extern "C" fn(*mut Display, *const c_char, *mut *mut *mut c_char, *mut c_int, *mut *mut c_char) -> XFontSet,
        XCreateGC: unsafe extern "C" fn(*mut Display, Drawable, c_ulong, *mut XGCValues) -> GC,
        XSetFont: unsafe extern "C" fn(*mut Display, GC, Font),
        XAllocColor: unsafe extern "C" fn(*mut Display, Colormap, *mut XColor) -> Status,
        XCreateImage: unsafe extern "C" fn(*mut Display, *mut Visual, c_uint, c_int, c_int, *mut c_char, c_uint, c_uint, c_int, c_int) -> *mut XImage,
        XCreateWindow: unsafe extern "C" fn(*mut Display, Window, c_int, c_int, c_uint, c_uint, c_uint, c_int, c_uint, *mut Visual, c_ulong, *mut XSetWindowAttributes) -> Window,
        XCopyArea: unsafe extern "C" fn(*mut Display, Drawable, Drawable, GC, c_int, c_int, c_uint, c_uint, c_int, c_int),
        XDefineCursor: unsafe extern "C" fn(*mut Display, Window, Cursor) -> c_int,
        XDeleteProperty: unsafe extern "C" fn(*mut Display, Window, Atom) -> c_int,
        XDestroyWindow: unsafe extern "C" fn(*mut Display, Window) -> c_int,
        XDisplayKeycodes: unsafe extern "C" fn(*mut Display, *mut c_int, *mut c_int) -> c_int,
        XDrawRectangle: unsafe extern "C" fn(*mut Display, Drawable, GC, c_int, c_int, c_uint, c_uint) -> c_int,
        XFontsOfFontSet: unsafe extern "C" fn(XFontSet, *mut *mut *mut XFontStruct, *mut *mut *mut c_char) -> c_int,
        XFillArc: unsafe extern "C" fn(*mut Display, Drawable, GC, c_int, c_int, c_uint, c_uint, c_int, c_int) -> c_int,
        XDisplayName: unsafe extern "C" fn(*const c_char) -> *mut c_char,
        XDrawString: unsafe extern "C" fn(*mut Display, Drawable, GC, c_int, c_int, *const c_char, c_int) -> c_int,
        XEventsQueued: unsafe extern "C" fn(*mut Display, c_int) -> c_int,
        XFillRectangle: unsafe extern "C" fn(*mut Display, Drawable, GC, c_int, c_int, c_uint, c_uint) -> c_int,
        XFilterEvent: unsafe extern "C" fn(*mut XEvent, Window) -> Bool,
        XFlush: unsafe extern "C" fn(*mut Display) -> c_int,
        XFree: unsafe extern "C" fn(*mut c_void) -> c_int,
        XFreeCursor: unsafe extern "C" fn(*mut Display, Cursor) -> c_int,
        XFreeFontSet: unsafe extern "C" fn(*mut Display, XFontSet),
        XFreeGC: unsafe extern "C" fn(*mut Display, GC) -> c_int,
        XFreeFont: unsafe extern "C" fn(*mut Display, *mut XFontStruct) -> c_int,
        XFreeModifiermap: unsafe extern "C" fn(*mut XModifierKeymap) -> c_int,
        XFreePixmap: unsafe extern "C" fn(*mut Display, Pixmap) -> c_int,
        XFreeColormap: unsafe extern "C" fn(*mut Display, Colormap) -> c_int,
        XFreeStringList: unsafe extern "C" fn(*mut *mut c_char),
        XGetAtomName: unsafe extern "C" fn(*mut Display, Atom) -> *mut c_char,
        XGetInputFocus: unsafe extern "C" fn(*mut Display, *mut Window, *mut c_int) -> c_int,
        XGetKeyboardMapping: unsafe extern "C" fn(*mut Display, KeyCode, c_int, *mut c_int) -> *mut KeySym,
        XGetErrorDatabaseText: unsafe extern "C" fn(*mut Display, *const c_char, *const c_char, *const c_char, *mut c_char, c_int) -> c_int,
        XGetModifierMapping: unsafe extern "C" fn(*mut Display) -> *mut XModifierKeymap,
        XGetPointerControl: unsafe extern "C" fn(*mut Display, *mut c_int, *mut c_int, *mut c_int) -> c_int,
        XGetSelectionOwner: unsafe extern "C" fn(*mut Display, Atom) -> Window,
        XGetVisualInfo: unsafe extern "C" fn(*mut Display, c_long, *mut XVisualInfo, *mut c_int) -> *mut XVisualInfo,
        XGetWindowAttributes: unsafe extern "C" fn(*mut Display, Window, *mut XWindowAttributes) -> Status,
        XGetWindowProperty: unsafe extern "C" fn(*mut Display, Window, Atom, c_long, c_long, Bool, Atom, *mut Atom, *mut c_int, *mut c_ulong, *mut c_ulong, *mut *mut c_uchar) -> c_int,
        XGetWMHints: unsafe extern "C" fn(*mut Display, Window) -> *mut XWMHints,
        XGetWMNormalHints: unsafe extern "C" fn(*mut Display, Window, *mut XSizeHints, *mut c_long) -> Status,
        XIfEvent: unsafe extern "C" fn(*mut Display, *mut XEvent, XIfEventPredicate, XPointer) -> c_int,
        XGrabKeyboard: unsafe extern "C" fn(*mut Display, Window, Bool, c_int, c_int, Time) -> c_int,
        XGrabPointer: unsafe extern "C" fn(*mut Display, Window, Bool, c_uint, c_int, c_int, Window, Cursor, Time) -> c_int,
        XGrabServer: unsafe extern "C" fn(*mut Display) -> c_int,
        XIconifyWindow: unsafe extern "C" fn(*mut Display, Window, c_int) -> Status,
        XKeysymToKeycode: unsafe extern "C" fn(*mut Display, KeySym) -> KeyCode,
        XKeysymToString: unsafe extern "C" fn(KeySym) -> *mut c_char,
        XInstallColormap: unsafe extern "C" fn(*mut Display, Colormap) -> c_int,
        XInternAtom: unsafe extern "C" fn(*mut Display, *const c_char, Bool) -> Atom,
        XListPixmapFormats: unsafe extern "C" fn(*mut Display, *mut c_int) -> *mut XPixmapFormatValues,
        XLoadQueryFont: unsafe extern "C" fn(*mut Display, *const c_char) -> *mut XFontStruct,
        XLookupKeysym: unsafe extern "C" fn(*mut XKeyEvent, c_int) -> KeySym,
        XLookupString: unsafe extern "C" fn(*mut XKeyEvent, *mut c_char, c_int, *mut KeySym, *mut XComposeStatus) -> c_int,
        XMapRaised: unsafe extern "C" fn(*mut Display, Window) -> c_int,
        XMatchVisualInfo: unsafe extern "C" fn(*mut Display, c_int, c_int, c_int, *mut XVisualInfo) -> Status,
        XMissingExtension: unsafe extern "C" fn(*mut Display, *const c_char) -> c_int,
        XMoveWindow: unsafe extern "C" fn(*mut Display, Window, c_int, c_int) -> c_int,
        XOpenDisplay: unsafe extern "C" fn(*const c_char) -> *mut Display,
        XInitThreads: unsafe extern "C" fn() -> Status,
        XPeekEvent: unsafe extern "C" fn(*mut Display, *mut XEvent) -> c_int,
        XPending: unsafe extern "C" fn(*mut Display) -> c_int,
        XGetImage: unsafe extern "C" fn(*mut Display, Drawable, c_int, c_int, c_uint, c_uint, c_ulong, c_int) -> *mut XImage,
        XPutImage: unsafe extern "C" fn(*mut Display, Drawable, GC, *mut XImage, c_int, c_int, c_int, c_int, c_uint, c_uint) -> c_int,
        XQueryKeymap: unsafe extern "C" fn(*mut Display, *mut c_char) -> c_int,
        XQueryPointer: unsafe extern "C" fn(*mut Display, Window, *mut Window, *mut Window, *mut c_int, *mut c_int, *mut c_int, *mut c_int, *mut c_uint) -> Bool,
        XRaiseWindow: unsafe extern "C" fn(*mut Display, Window) -> c_int,
        XReparentWindow: unsafe extern "C" fn(*mut Display, Window, Window, c_int, c_int) -> c_int,
        XResetScreenSaver: unsafe extern "C" fn(*mut Display) -> c_int,
        XResizeWindow: unsafe extern "C" fn(*mut Display, Window, c_uint, c_uint) -> c_int,
        XScreenNumberOfScreen: unsafe extern "C" fn(*mut Screen) -> c_int,
        XSelectInput: unsafe extern "C" fn(*mut Display, Window, c_long) -> c_int,
        XSendEvent: unsafe extern "C" fn(*mut Display, Window, Bool, c_long, *mut XEvent) -> Status,
        XSetErrorHandler: unsafe extern "C" fn(XErrorHandler) -> XErrorHandler,
        XSetForeground: unsafe extern "C" fn(*mut Display, GC, c_ulong) -> c_int,
        XSetIOErrorHandler: unsafe extern "C" fn(XIOErrorHandler) -> XIOErrorHandler,
        XSetInputFocus: unsafe extern "C" fn(*mut Display, Window, c_int, Time) -> c_int,
        XSetSelectionOwner: unsafe extern "C" fn(*mut Display, Atom, Window, Time) -> c_int,
        XSetTransientForHint: unsafe extern "C" fn(*mut Display, Window, Window) -> c_int,
        XSetTextProperty: unsafe extern "C" fn(*mut Display, Window, *mut XTextProperty, Atom),
        XSetWindowBackground: unsafe extern "C" fn(*mut Display, Window, c_ulong) -> c_int,
        XSetWMHints: unsafe extern "C" fn(*mut Display, Window, *mut XWMHints),
        XSetWMNormalHints: unsafe extern "C" fn(*mut Display, Window, *mut XSizeHints),
        XSetWMProperties: unsafe extern "C" fn(*mut Display, Window, *mut XTextProperty, *mut XTextProperty, *mut *mut c_char, c_int, *mut XSizeHints, *mut XWMHints, *mut XClassHint),
        XSetWMProtocols: unsafe extern "C" fn(*mut Display, Window, *mut Atom, c_int) -> Status,
        XStoreColors: unsafe extern "C" fn(*mut Display, Colormap, *mut XColor, c_int) -> c_int,
        XStoreName: unsafe extern "C" fn(*mut Display, Window, *const c_char) -> c_int,
        XStringListToTextProperty: unsafe extern "C" fn(*mut *mut c_char, c_int, *mut XTextProperty) -> Status,
        XSync: unsafe extern "C" fn(*mut Display, Bool) -> c_int,
        XTextExtents: unsafe extern "C" fn(*mut XFontStruct, *const c_char, c_int, *mut c_int, *mut c_int, *mut c_int, *mut XCharStruct) -> c_int,
        XTranslateCoordinates: unsafe extern "C" fn(*mut Display, Window, Window, c_int, c_int, *mut c_int, *mut c_int, *mut Window) -> Bool,
        XUndefineCursor: unsafe extern "C" fn(*mut Display, Window) -> c_int,
        XUngrabKeyboard: unsafe extern "C" fn(*mut Display, Time) -> c_int,
        XUngrabPointer: unsafe extern "C" fn(*mut Display, Time) -> c_int,
        XUngrabServer: unsafe extern "C" fn(*mut Display) -> c_int,
        XUninstallColormap: unsafe extern "C" fn(*mut Display, Colormap) -> c_int,
        XUnloadFont: unsafe extern "C" fn(*mut Display, Font) -> c_int,
        XWarpPointer: unsafe extern "C" fn(*mut Display, Window, Window, c_int, c_int, c_uint, c_uint, c_int, c_int) -> c_int,
        XWindowEvent: unsafe extern "C" fn(*mut Display, Window, c_long, *mut XEvent) -> c_int,
        XWithdrawWindow: unsafe extern "C" fn(*mut Display, Window, c_int) -> Status,
        XVisualIDFromVisual: unsafe extern "C" fn(*mut Visual) -> VisualID,
        XGetDefault: unsafe extern "C" fn(*mut Display, *const c_char, *const c_char) -> *mut c_char,
        XQueryExtension: unsafe extern "C" fn(*mut Display, *const c_char, *mut c_int, *mut c_int, *mut c_int) -> Bool,
        XDisplayString: unsafe extern "C" fn(*mut Display) -> *mut c_char,
        XGetErrorText: unsafe extern "C" fn(*mut Display, c_int, *mut c_char, c_int) -> c_int,
        _XEatData: unsafe extern "C" fn(*mut Display, c_ulong),
        _XFlush: unsafe extern "C" fn(*mut Display),
        _XFlushGCCache: unsafe extern "C" fn(*mut Display, GC),
        _XRead: unsafe extern "C" fn(*mut Display, *mut c_char, c_long) -> c_int,
        _XReadPad: unsafe extern "C" fn(*mut Display, *mut c_char, c_long),
        _XSend: unsafe extern "C" fn(*mut Display, *const c_char, c_long),
        _XReply: unsafe extern "C" fn(*mut Display, *mut c_void, c_int, Bool) -> Status,
        _XSetLastRequestRead: unsafe extern "C" fn(*mut Display, *mut c_void) -> c_ulong,
        XSynchronize: unsafe extern "C" fn(*mut Display, Bool) -> SDL_X11_XSynchronizeRetType,
        XESetWireToEvent: unsafe extern "C" fn(*mut Display, c_int, SDL_X11_XESetWireToEventRetType) -> SDL_X11_XESetWireToEventRetType,
        XESetEventToWire: unsafe extern "C" fn(*mut Display, c_int, SDL_X11_XESetEventToWireRetType) -> SDL_X11_XESetEventToWireRetType,
        XRefreshKeyboardMapping: unsafe extern "C" fn(*mut XMappingEvent),
        XQueryTree: unsafe extern "C" fn(*mut Display, Window, *mut Window, *mut Window, *mut *mut Window, *mut c_uint) -> c_int,
        XSupportsLocale: unsafe extern "C" fn() -> Bool,
        XmbTextListToTextProperty: unsafe extern "C" fn(*mut Display, *mut *mut c_char, c_int, XICCEncodingStyle, *mut XTextProperty) -> Status,
        XCreateRegion: unsafe extern "C" fn() -> Region,
        XUnionRectWithRegion: unsafe extern "C" fn(*mut XRectangle, Region, Region) -> c_int,
        XDestroyRegion: unsafe extern "C" fn(Region),
        XrmInitialize: unsafe extern "C" fn(),
        XResourceManagerString: unsafe extern "C" fn(*mut Display) -> *mut c_char,
        XrmGetStringDatabase: unsafe extern "C" fn(*mut c_char) -> XrmDatabase,
        XrmDestroyDatabase: unsafe extern "C" fn(XrmDatabase),
        XrmGetResource: unsafe extern "C" fn(XrmDatabase, *mut c_char, *mut c_char, *mut *mut c_char, *mut XrmValue) -> Bool,
        XGetPointerMapping: unsafe extern "C" fn(*mut Display, *mut c_uchar, c_uint) -> c_int,
    }
}

x11_module! {
    /// `SDL_X11_MODULE(XFIXES)`.
    XFixes {
        XFixesCreatePointerBarrier: unsafe extern "C" fn(*mut Display, Window, c_int, c_int, c_int, c_int, c_int, c_int, *mut c_int) -> PointerBarrier,
        XFixesDestroyPointerBarrier: unsafe extern "C" fn(*mut Display, PointerBarrier),
        // this is actually Xinput2
        XIBarrierReleasePointer: unsafe extern "C" fn(*mut Display, c_int, PointerBarrier, BarrierEventID) -> c_int,
        XFixesQueryVersion: unsafe extern "C" fn(*mut Display, *mut c_int, *mut c_int) -> Status,
        XFixesSelectSelectionInput: unsafe extern "C" fn(*mut Display, Window, Atom, c_ulong) -> Status,
    }
}

x11_module! {
    /// `SDL_X11_MODULE(XSYNC)`.
    XSync {
        XSyncQueryExtension: unsafe extern "C" fn(*mut Display, *mut c_int, *mut c_int) -> Status,
        XSyncInitialize: unsafe extern "C" fn(*mut Display, *mut c_int, *mut c_int) -> Status,
        XSyncCreateCounter: unsafe extern "C" fn(*mut Display, XSyncValue) -> XSyncCounter,
        XSyncDestroyCounter: unsafe extern "C" fn(*mut Display, XSyncCounter) -> Status,
        XSyncSetCounter: unsafe extern "C" fn(*mut Display, XSyncCounter, XSyncValue) -> Status,
    }
}

x11_module! {
    /// `SDL_X11_MODULE(XTEST)`.
    XTest {
        XTestQueryExtension: unsafe extern "C" fn(*mut Display, *mut c_int, *mut c_int, *mut c_int, *mut c_int) -> Status,
        XTestFakeMotionEvent: unsafe extern "C" fn(*mut Display, c_int, c_int, c_int, c_ulong) -> c_int,
    }
}

x11_module! {
    /// The `SDL_VIDEO_DRIVER_X11_SUPPORTS_GENERIC_EVENTS` and
    /// `SDL_VIDEO_DRIVER_X11_HAS_XKBLIB` functions. Upstream lists them
    /// after `SDL_X11_MODULE(XTEST)` without a module of their own, so they
    /// count as part of XTEST (see [`X11Syms::xtest`]); SDL calls them
    /// without checking any flag.
    XkbGeneric {
        XGetEventData: unsafe extern "C" fn(*mut Display, *mut XGenericEventCookie) -> Bool,
        XFreeEventData: unsafe extern "C" fn(*mut Display, *mut XGenericEventCookie),
        XkbQueryExtension: unsafe extern "C" fn(*mut Display, *mut c_int, *mut c_int, *mut c_int, *mut c_int, *mut c_int) -> Bool,
        XkbKeycodeToKeysym: unsafe extern "C" fn(*mut Display, KeyCode, c_uint, c_uint) -> KeySym,
        XkbSelectEvents: unsafe extern "C" fn(*mut Display, c_uint, c_uint, c_uint) -> Bool,
        XkbSelectEventDetails: unsafe extern "C" fn(*mut Display, c_uint, c_uint, c_ulong, c_ulong) -> Bool,
        XkbGetNames: unsafe extern "C" fn(*mut Display, c_uint, XkbDescPtr) -> Status,
        XkbGetState: unsafe extern "C" fn(*mut Display, c_uint, *mut XkbStateRec) -> Status,
        XkbGetUpdatedMap: unsafe extern "C" fn(*mut Display, c_uint, XkbDescPtr) -> Status,
        XkbGetMap: unsafe extern "C" fn(*mut Display, c_uint, c_uint) -> XkbDescPtr,
        XkbFreeClientMap: unsafe extern "C" fn(XkbDescPtr, c_uint, Bool),
        XkbFreeKeyboard: unsafe extern "C" fn(XkbDescPtr, c_uint, Bool),
        XkbRefreshKeyboardMapping: unsafe extern "C" fn(*mut XkbMapNotifyEvent) -> Status,
        XkbSetDetectableAutoRepeat: unsafe extern "C" fn(*mut Display, Bool, *mut Bool) -> Bool,
    }
}

x11_module! {
    /// `SDL_X11_MODULE(UTF8)` (`X_HAVE_UTF8_STRING`), including the
    /// variadic input method functions.
    Utf8 {
        Xutf8TextListToTextProperty: unsafe extern "C" fn(*mut Display, *mut *mut c_char, c_int, XICCEncodingStyle, *mut XTextProperty) -> c_int,
        Xutf8LookupString: unsafe extern "C" fn(XIC, *mut XKeyEvent, *mut c_char, c_int, *mut KeySym, *mut Status) -> c_int,
        XCreateIC: unsafe extern "C" fn(XIM, ...) -> XIC,
        XDestroyIC: unsafe extern "C" fn(XIC),
        XGetICValues: unsafe extern "C" fn(XIC, ...) -> *mut c_char,
        XSetICValues: unsafe extern "C" fn(XIC, ...) -> *mut c_char,
        XVaCreateNestedList: unsafe extern "C" fn(c_int, ...) -> XVaNestedList,
        XSetICFocus: unsafe extern "C" fn(XIC),
        XUnsetICFocus: unsafe extern "C" fn(XIC),
        XOpenIM: unsafe extern "C" fn(*mut Display, *mut _XrmHashBucketRec, *mut c_char, *mut c_char) -> XIM,
        XCloseIM: unsafe extern "C" fn(XIM) -> Status,
        XSetIMValues: unsafe extern "C" fn(XIM, ...) -> *mut c_char,
        XRegisterIMInstantiateCallback: unsafe extern "C" fn(*mut Display, *mut _XrmHashBucketRec, *mut c_char, *mut c_char, XIDProc, XPointer) -> Bool,
        XUnregisterIMInstantiateCallback: unsafe extern "C" fn(*mut Display, *mut _XrmHashBucketRec, *mut c_char, *mut c_char, XIDProc, XPointer) -> Bool,
        Xutf8DrawString: unsafe extern "C" fn(*mut Display, Drawable, XFontSet, GC, c_int, c_int, *const c_char, c_int),
        Xutf8TextExtents: unsafe extern "C" fn(XFontSet, *const c_char, c_int, *mut XRectangle, *mut XRectangle) -> c_int,
        XSetLocaleModifiers: unsafe extern "C" fn(*const c_char) -> *mut c_char,
        Xutf8ResetIC: unsafe extern "C" fn(XIC) -> *mut c_char,
        XExtentsOfFontSet: unsafe extern "C" fn(XFontSet) -> *mut XFontSetExtents,
        XContextDependentDrawing: unsafe extern "C" fn(XFontSet) -> Bool,
    }
}

x11_module! {
    /// `SDL_X11_MODULE(SHM)` (the MIT shared memory extension).
    Shm {
        XShmAttach: unsafe extern "C" fn(*mut Display, *mut XShmSegmentInfo) -> Status,
        XShmDetach: unsafe extern "C" fn(*mut Display, *mut XShmSegmentInfo) -> Status,
        XShmPutImage: unsafe extern "C" fn(*mut Display, Drawable, GC, *mut XImage, c_int, c_int, c_int, c_int, c_uint, c_uint, Bool) -> Status,
        XShmGetImage: unsafe extern "C" fn(*mut Display, Drawable, *mut XImage, c_int, c_int, c_ulong) -> Status,
        XShmCreateImage: unsafe extern "C" fn(*mut Display, *mut Visual, c_uint, c_int, *mut c_char, *mut XShmSegmentInfo, c_uint, c_uint) -> *mut XImage,
        XShmCreatePixmap: unsafe extern "C" fn(*mut Display, Drawable, *mut c_char, *mut XShmSegmentInfo, c_uint, c_uint, c_uint) -> Pixmap,
        XShmQueryExtension: unsafe extern "C" fn(*mut Display) -> Bool,
        XShmQueryVersion: unsafe extern "C" fn(*mut Display, *mut c_int, *mut c_int, *mut Bool) -> Status,
        XShmPixmapFormat: unsafe extern "C" fn(*mut Display) -> c_int,
    }
}

x11_module! {
    /// `SDL_X11_MODULE(IO_32BIT)`: "Not required...these only exist in code
    /// in headers on some 64-bit platforms, and are removed via macros
    /// elsewhere, so it's safe for them to be missing."
    Io32Bit {
        _XData32: unsafe extern "C" fn(*mut Display, *const c_long, c_uint) -> c_int,
        _XRead32: unsafe extern "C" fn(*mut Display, *mut c_long, c_long),
    }
}

x11_module! {
    /// `SDL_X11_MODULE(XCURSOR)`.
    XCursor {
        XcursorImageCreate: unsafe extern "C" fn(c_int, c_int) -> *mut XcursorImage,
        XcursorImagesCreate: unsafe extern "C" fn(c_int) -> *mut XcursorImages,
        XcursorImageDestroy: unsafe extern "C" fn(*mut XcursorImage),
        XcursorImagesDestroy: unsafe extern "C" fn(*mut XcursorImages),
        XcursorImageLoadCursor: unsafe extern "C" fn(*mut Display, *const XcursorImage) -> Cursor,
        XcursorImagesLoadCursor: unsafe extern "C" fn(*mut Display, *const XcursorImages) -> Cursor,
        XcursorLibraryLoadCursor: unsafe extern "C" fn(*mut Display, *const c_char) -> Cursor,
    }
}

/// `XdbeBackBuffer`.
pub(crate) type XdbeBackBuffer = Drawable;
/// `XdbeSwapAction`.
pub(crate) type XdbeSwapAction = c_uchar;

/// Translation of `XdbeSwapInfo`.
#[repr(C)]
pub(crate) struct XdbeSwapInfo {
    pub(crate) swap_window: Window,
    pub(crate) swap_action: XdbeSwapAction,
}

x11_module! {
    /// `SDL_X11_MODULE(XDBE)` (the double buffer extension).
    Xdbe {
        XdbeQueryExtension: unsafe extern "C" fn(*mut Display, *mut c_int, *mut c_int) -> Status,
        XdbeAllocateBackBufferName: unsafe extern "C" fn(*mut Display, Window, XdbeSwapAction) -> XdbeBackBuffer,
        XdbeDeallocateBackBufferName: unsafe extern "C" fn(*mut Display, XdbeBackBuffer) -> Status,
        XdbeSwapBuffers: unsafe extern "C" fn(*mut Display, *mut XdbeSwapInfo, c_int) -> Status,
        XdbeBeginIdiom: unsafe extern "C" fn(*mut Display) -> Status,
        XdbeEndIdiom: unsafe extern "C" fn(*mut Display) -> Status,
        XdbeGetVisualInfo: unsafe extern "C" fn(*mut Display, *mut Drawable, *mut c_int) -> *mut c_void,
        XdbeFreeVisualInfo: unsafe extern "C" fn(*mut c_void),
        XdbeGetBackBufferAttributes: unsafe extern "C" fn(*mut Display, XdbeBackBuffer) -> *mut c_void,
    }
}

x11_module! {
    /// `SDL_X11_MODULE(XINPUT2)`: XInput2 support for multiple mice,
    /// tablets, etc.
    XInput2 {
        XIQueryDevice: unsafe extern "C" fn(*mut Display, c_int, *mut c_int) -> *mut XIDeviceInfo,
        XIFreeDeviceInfo: unsafe extern "C" fn(*mut XIDeviceInfo),
        XISelectEvents: unsafe extern "C" fn(*mut Display, Window, *mut XIEventMask, c_int) -> c_int,
        XIGrabTouchBegin: unsafe extern "C" fn(*mut Display, c_int, Window, c_int, *mut XIEventMask, c_int, *mut XIGrabModifiers) -> c_int,
        XIUngrabTouchBegin: unsafe extern "C" fn(*mut Display, c_int, Window, c_int, *mut XIGrabModifiers) -> c_int,
        XIQueryVersion: unsafe extern "C" fn(*mut Display, *mut c_int, *mut c_int) -> Status,
        XIGetSelectedEvents: unsafe extern "C" fn(*mut Display, Window, *mut c_int) -> *mut XIEventMask,
        XIGetClientPointer: unsafe extern "C" fn(*mut Display, Window, *mut c_int) -> Bool,
        XIWarpPointer: unsafe extern "C" fn(*mut Display, c_int, Window, Window, c_double, c_double, c_int, c_int, c_double, c_double) -> Bool,
        XIGetProperty: unsafe extern "C" fn(*mut Display, c_int, Atom, c_long, c_long, Bool, Atom, *mut Atom, *mut c_int, *mut c_ulong, *mut c_ulong, *mut *mut c_uchar) -> Status,
    }
}

x11_module! {
    /// `SDL_X11_MODULE(XRANDR)`.
    XRandR {
        XRRQueryVersion: unsafe extern "C" fn(*mut Display, *mut c_int, *mut c_int) -> Status,
        XRRQueryExtension: unsafe extern "C" fn(*mut Display, *mut c_int, *mut c_int) -> Bool,
        XRRGetScreenInfo: unsafe extern "C" fn(*mut Display, Drawable) -> *mut XRRScreenConfiguration,
        XRRConfigCurrentConfiguration: unsafe extern "C" fn(*mut XRRScreenConfiguration, *mut Rotation) -> SizeID,
        XRRConfigCurrentRate: unsafe extern "C" fn(*mut XRRScreenConfiguration) -> c_short,
        XRRConfigRates: unsafe extern "C" fn(*mut XRRScreenConfiguration, c_int, *mut c_int) -> *mut c_short,
        XRRConfigSizes: unsafe extern "C" fn(*mut XRRScreenConfiguration, *mut c_int) -> *mut XRRScreenSize,
        XRRSetScreenConfigAndRate: unsafe extern "C" fn(*mut Display, *mut XRRScreenConfiguration, Drawable, c_int, Rotation, c_short, Time) -> Status,
        XRRFreeScreenConfigInfo: unsafe extern "C" fn(*mut XRRScreenConfiguration),
        XRRSetScreenSize: unsafe extern "C" fn(*mut Display, Window, c_int, c_int, c_int, c_int),
        XRRGetScreenSizeRange: unsafe extern "C" fn(*mut Display, Window, *mut c_int, *mut c_int, *mut c_int, *mut c_int) -> Status,
        XRRGetScreenResources: unsafe extern "C" fn(*mut Display, Window) -> *mut XRRScreenResources,
        XRRGetScreenResourcesCurrent: unsafe extern "C" fn(*mut Display, Window) -> *mut XRRScreenResources,
        XRRFreeScreenResources: unsafe extern "C" fn(*mut XRRScreenResources),
        XRRGetOutputInfo: unsafe extern "C" fn(*mut Display, *mut XRRScreenResources, RROutput) -> *mut XRROutputInfo,
        XRRFreeOutputInfo: unsafe extern "C" fn(*mut XRROutputInfo),
        XRRGetCrtcInfo: unsafe extern "C" fn(*mut Display, *mut XRRScreenResources, RRCrtc) -> *mut XRRCrtcInfo,
        XRRFreeCrtcInfo: unsafe extern "C" fn(*mut XRRCrtcInfo),
        XRRSetCrtcConfig: unsafe extern "C" fn(*mut Display, *mut XRRScreenResources, RRCrtc, Time, c_int, c_int, RRMode, Rotation, *mut RROutput, c_int) -> Status,
        XRRListOutputProperties: unsafe extern "C" fn(*mut Display, RROutput, *mut c_int) -> *mut Atom,
        XRRQueryOutputProperty: unsafe extern "C" fn(*mut Display, RROutput, Atom) -> *mut XRRPropertyInfo,
        XRRGetOutputProperty: unsafe extern "C" fn(*mut Display, RROutput, Atom, c_long, c_long, Bool, Bool, Atom, *mut Atom, *mut c_int, *mut c_ulong, *mut c_ulong, *mut *mut c_uchar) -> c_int,
        XRRGetOutputPrimary: unsafe extern "C" fn(*mut Display, Window) -> RROutput,
        XRRSelectInput: unsafe extern "C" fn(*mut Display, Window, c_int),
        XRRGetCrtcTransform: unsafe extern "C" fn(*mut Display, RRCrtc, *mut *mut XRRCrtcTransformAttributes) -> Status,
    }
}

x11_module! {
    /// `SDL_X11_MODULE(XSS)`: MIT-SCREEN-SAVER support.
    Xss {
        XScreenSaverQueryExtension: unsafe extern "C" fn(*mut Display, *mut c_int, *mut c_int) -> Bool,
        XScreenSaverQueryVersion: unsafe extern "C" fn(*mut Display, *mut c_int, *mut c_int) -> Status,
        XScreenSaverSuspend: unsafe extern "C" fn(*mut Display, Bool),
    }
}

x11_module! {
    /// `SDL_X11_MODULE(XSHAPE)`.
    XShape {
        XShapeCombineMask: unsafe extern "C" fn(*mut Display, Window, c_int, c_int, c_int, Pixmap, c_int),
        XShapeCombineRegion: unsafe extern "C" fn(*mut Display, Window, c_int, c_int, c_int, Region, c_int),
    }
}

/// The loaded X11 functions: the base Xlib functions (reached through
/// `Deref`, as upstream's `X11_XFlush` and friends) and the optional
/// modules, `None` when their `SDL_X11_HAVE_*` flag is 0.
pub(crate) struct X11Syms {
    base: BaseXlib,
    /// `SDL_X11_HAVE_XFIXES`
    pub(crate) xfixes: Option<XFixes>,
    /// `SDL_X11_HAVE_XSYNC`
    pub(crate) xsync: Option<XSync>,
    /// `SDL_X11_HAVE_XTEST`
    // FIXME (upstream): SDL_x11sym.h lists XGetEventData, XFreeEventData and
    // the Xkb* functions after SDL_X11_MODULE(XTEST), so a missing one of
    // those also clears SDL_X11_HAVE_XTEST (kept: `xtest` needs `xkb`).
    pub(crate) xtest: Option<XTest>,
    /// The generic event and XKB functions (see [`XkbGeneric`]).
    pub(crate) xkb: Option<XkbGeneric>,
    /// `SDL_X11_HAVE_UTF8`
    pub(crate) utf8: Option<Utf8>,
    /// `SDL_X11_HAVE_SHM`
    pub(crate) shm: Option<Shm>,
    /// `SDL_X11_HAVE_IO_32BIT`
    pub(crate) io_32bit: Option<Io32Bit>,
    /// `SDL_X11_HAVE_XCURSOR`
    pub(crate) xcursor: Option<XCursor>,
    /// `SDL_X11_HAVE_XDBE`
    pub(crate) xdbe: Option<Xdbe>,
    /// `SDL_X11_HAVE_XINPUT2`
    pub(crate) xinput2: Option<XInput2>,
    /// `SDL_X11_HAVE_XRANDR`
    pub(crate) xrandr: Option<XRandR>,
    /// `SDL_X11_HAVE_XSS`
    pub(crate) xss: Option<Xss>,
    /// `SDL_X11_HAVE_XSHAPE`
    pub(crate) xshape: Option<XShape>,
    /// The libraries the functions live in, unloaded on drop (after the
    /// function pointers above, which are never used past that point).
    _libs: Vec<SharedObject>,
}

impl std::ops::Deref for X11Syms {
    type Target = BaseXlib;
    fn deref(&self) -> &BaseXlib {
        &self.base
    }
}

impl X11Syms {
    /// Open the libraries and look up all the symbols; `None` if the base
    /// Xlib functions aren't all there. The body of `SDL_X11_LoadSymbols()`.
    fn load() -> Option<X11Syms> {
        let libs: Vec<SharedObject> = X11LIBS
            .iter()
            .filter_map(|name| SharedObject::load(name).ok())
            .collect();

        let base = BaseXlib::load(&libs);
        let xfixes = XFixes::load(&libs);
        let xsync = XSync::load(&libs);
        let xtest = XTest::load(&libs);
        let xkb = XkbGeneric::load(&libs);
        let xtest = if xkb.is_some() { xtest } else { None };
        let utf8 = Utf8::load(&libs);
        let shm = Shm::load(&libs);
        let io_32bit = if cfg!(target_pointer_width = "64") {
            Io32Bit::load(&libs)
        } else {
            None
        };
        let xcursor = XCursor::load(&libs);
        let xdbe = Xdbe::load(&libs);
        let xinput2 = XInput2::load(&libs);
        let xrandr = XRandR::load(&libs);
        let xss = Xss::load(&libs);
        let xshape = XShape::load(&libs);

        // in case something got loaded... (dropping `libs` unloads it)
        Some(X11Syms {
            base: base?,
            xfixes,
            xsync,
            xtest,
            xkb,
            utf8,
            shm,
            io_32bit,
            xcursor,
            xdbe,
            xinput2,
            xrandr,
            xss,
            xshape,
            _libs: libs,
        })
    }
}

/// `x11_load_refcount` and the loaded symbols.
static LOADED: Mutex<(usize, Option<Arc<X11Syms>>)> = Mutex::new((0, None));

/// The loaded symbols, if any (for the error handlers, which Xlib calls
/// without a device).
pub(crate) fn loaded_symbols() -> Option<Arc<X11Syms>> {
    LOADED.lock().unwrap_or_else(|e| e.into_inner()).1.clone()
}

/// Load the X11 libraries (or take another reference to them); `None` if
/// the needed symbols couldn't be loaded. Translation of
/// `SDL_X11_LoadSymbols()`.
pub(crate) fn load_symbols() -> Option<Arc<X11Syms>> {
    let mut loaded = LOADED.lock().unwrap_or_else(|e| e.into_inner());
    // deal with multiple modules (dga, x11, etc) needing these symbols...
    if loaded.0 == 0 {
        // (on failure the reference is dropped again, as upstream's
        // SDL_X11_UnloadSymbols() call does)
        loaded.1 = Some(Arc::new(X11Syms::load()?));
    }
    loaded.0 += 1;
    loaded.1.clone()
}

/// Release a reference from [`load_symbols`]; the libraries are unloaded
/// when the last one goes (and the last handle is dropped). Translation of
/// `SDL_X11_UnloadSymbols()`.
pub(crate) fn unload_symbols() {
    let mut loaded = LOADED.lock().unwrap_or_else(|e| e.into_inner());
    // Don't actually unload if more than one module is using the libs...
    if loaded.0 > 0 {
        loaded.0 -= 1;
        if loaded.0 == 0 {
            loaded.1 = None;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn load_and_unload() {
        let Some(x) = load_symbols() else {
            println!("note: libX11 isn't installed; skipping the X11 symbol test");
            return;
        };
        // A second reference shares the same symbols.
        let y = load_symbols().unwrap();
        assert!(Arc::ptr_eq(&x, &y));
        // SAFETY: XSupportsLocale takes no arguments.
        let _ = unsafe { (x.XSupportsLocale)() };
        unload_symbols();
        unload_symbols();
    }
}
