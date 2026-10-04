// Hand-written declarations of the parts of <X11/Xlib.h>, <X11/Xutil.h>,
// <X11/Xatom.h>, <X11/XKBlib.h> and the X extension headers that the X11
// video driver of Simple DirectMedia Layer uses.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! The X11 types, structures, constants and header macros the driver uses
//! (what `SDL_x11dyn.h` gets from `#include <X11/Xlib.h>` and friends).
//!
//! Nothing here links to a library: the functions are loaded at run time
//! by [`dyn`](super::dyn) as upstream's `SDL_x11dyn.c` does. The layouts
//! follow the C headers for the platform's `int`/`long`/pointer sizes and
//! are checked against the C compiler's in the tests below.

#![allow(non_camel_case_types, non_snake_case, non_upper_case_globals)]
#![allow(clippy::upper_case_acronyms)] // (the names of the X headers)
#![allow(dead_code)] // (a header: not every declaration is used)

use std::ffi::{
    c_char, c_double, c_int, c_long, c_short, c_uchar, c_uint, c_ulong, c_ushort, c_void,
};

// ---------------------------------------------------------------------------
// <X11/X.h>, <X11/Xlib.h>: basic types
// ---------------------------------------------------------------------------

pub type XID = c_ulong;
pub type Mask = c_ulong;
pub type Atom = c_ulong;
pub type VisualID = c_ulong;
pub type Time = c_ulong;
pub type Window = XID;
pub type Drawable = XID;
pub type Font = XID;
pub type Pixmap = XID;
pub type Cursor = XID;
pub type Colormap = XID;
pub type GContext = XID;
pub type KeySym = XID;
pub type KeyCode = c_uchar;
pub type Bool = c_int;
pub type Status = c_int;
pub type XPointer = *mut c_char;

/// `struct _XDisplay`, only ever used behind a pointer.
#[repr(C)]
pub struct Display {
    _private: [u8; 0],
}

/// `struct _XGC`, only ever used behind a pointer.
#[repr(C)]
pub struct _XGC {
    _private: [u8; 0],
}
pub type GC = *mut _XGC;

/// `struct _XIM`, only ever used behind a pointer.
#[repr(C)]
pub struct _XIM {
    _private: [u8; 0],
}
pub type XIM = *mut _XIM;

/// `struct _XIC`, only ever used behind a pointer.
#[repr(C)]
pub struct _XIC {
    _private: [u8; 0],
}
pub type XIC = *mut _XIC;

/// `struct _XOC`, only ever used behind a pointer.
#[repr(C)]
pub struct _XOC {
    _private: [u8; 0],
}
pub type XFontSet = *mut _XOC;

/// `struct _XRegion`, only ever used behind a pointer.
#[repr(C)]
pub struct _XRegion {
    _private: [u8; 0],
}
pub type Region = *mut _XRegion;

/// `struct _XrmHashBucketRec`, only ever used behind a pointer.
#[repr(C)]
pub struct _XrmHashBucketRec {
    _private: [u8; 0],
}
pub type XrmDatabase = *mut _XrmHashBucketRec;

pub type XVaNestedList = *mut c_void;

pub const None: c_ulong = 0;
pub const ParentRelative: c_ulong = 1;
pub const CopyFromParent: c_ulong = 0;
pub const PointerWindow: Window = 0;
pub const InputFocus: Window = 1;
pub const PointerRoot: Window = 1;
pub const AnyPropertyType: Atom = 0;
pub const AnyKey: c_int = 0;
pub const AnyButton: c_int = 0;
pub const AllTemporary: c_ulong = 0;
pub const CurrentTime: Time = 0;
pub const NoSymbol: KeySym = 0;

pub const True: Bool = 1;
pub const False: Bool = 0;

// Input event masks
pub const NoEventMask: c_long = 0;
pub const KeyPressMask: c_long = 1 << 0;
pub const KeyReleaseMask: c_long = 1 << 1;
pub const ButtonPressMask: c_long = 1 << 2;
pub const ButtonReleaseMask: c_long = 1 << 3;
pub const EnterWindowMask: c_long = 1 << 4;
pub const LeaveWindowMask: c_long = 1 << 5;
pub const PointerMotionMask: c_long = 1 << 6;
pub const PointerMotionHintMask: c_long = 1 << 7;
pub const Button1MotionMask: c_long = 1 << 8;
pub const Button2MotionMask: c_long = 1 << 9;
pub const Button3MotionMask: c_long = 1 << 10;
pub const Button4MotionMask: c_long = 1 << 11;
pub const Button5MotionMask: c_long = 1 << 12;
pub const ButtonMotionMask: c_long = 1 << 13;
pub const KeymapStateMask: c_long = 1 << 14;
pub const ExposureMask: c_long = 1 << 15;
pub const VisibilityChangeMask: c_long = 1 << 16;
pub const StructureNotifyMask: c_long = 1 << 17;
pub const ResizeRedirectMask: c_long = 1 << 18;
pub const SubstructureNotifyMask: c_long = 1 << 19;
pub const SubstructureRedirectMask: c_long = 1 << 20;
pub const FocusChangeMask: c_long = 1 << 21;
pub const PropertyChangeMask: c_long = 1 << 22;
pub const ColormapChangeMask: c_long = 1 << 23;
pub const OwnerGrabButtonMask: c_long = 1 << 24;

// Event names
pub const KeyPress: c_int = 2;
pub const KeyRelease: c_int = 3;
pub const ButtonPress: c_int = 4;
pub const ButtonRelease: c_int = 5;
pub const MotionNotify: c_int = 6;
pub const EnterNotify: c_int = 7;
pub const LeaveNotify: c_int = 8;
pub const FocusIn: c_int = 9;
pub const FocusOut: c_int = 10;
pub const KeymapNotify: c_int = 11;
pub const Expose: c_int = 12;
pub const GraphicsExpose: c_int = 13;
pub const NoExpose: c_int = 14;
pub const VisibilityNotify: c_int = 15;
pub const CreateNotify: c_int = 16;
pub const DestroyNotify: c_int = 17;
pub const UnmapNotify: c_int = 18;
pub const MapNotify: c_int = 19;
pub const MapRequest: c_int = 20;
pub const ReparentNotify: c_int = 21;
pub const ConfigureNotify: c_int = 22;
pub const ConfigureRequest: c_int = 23;
pub const GravityNotify: c_int = 24;
pub const ResizeRequest: c_int = 25;
pub const CirculateNotify: c_int = 26;
pub const CirculateRequest: c_int = 27;
pub const PropertyNotify: c_int = 28;
pub const SelectionClear: c_int = 29;
pub const SelectionRequest: c_int = 30;
pub const SelectionNotify: c_int = 31;
pub const ColormapNotify: c_int = 32;
pub const ClientMessage: c_int = 33;
pub const MappingNotify: c_int = 34;
pub const GenericEvent: c_int = 35;
pub const LASTEvent: c_int = 36;

// Key masks
pub const ShiftMask: c_uint = 1 << 0;
pub const LockMask: c_uint = 1 << 1;
pub const ControlMask: c_uint = 1 << 2;
pub const Mod1Mask: c_uint = 1 << 3;
pub const Mod2Mask: c_uint = 1 << 4;
pub const Mod3Mask: c_uint = 1 << 5;
pub const Mod4Mask: c_uint = 1 << 6;
pub const Mod5Mask: c_uint = 1 << 7;

// modifier names
pub const ShiftMapIndex: c_int = 0;
pub const LockMapIndex: c_int = 1;
pub const ControlMapIndex: c_int = 2;
pub const Mod1MapIndex: c_int = 3;
pub const Mod2MapIndex: c_int = 4;
pub const Mod3MapIndex: c_int = 5;
pub const Mod4MapIndex: c_int = 6;
pub const Mod5MapIndex: c_int = 7;

// button masks
pub const Button1Mask: c_uint = 1 << 8;
pub const Button2Mask: c_uint = 1 << 9;
pub const Button3Mask: c_uint = 1 << 10;
pub const Button4Mask: c_uint = 1 << 11;
pub const Button5Mask: c_uint = 1 << 12;
pub const AnyModifier: c_uint = 1 << 15;

// button names
pub const Button1: c_uint = 1;
pub const Button2: c_uint = 2;
pub const Button3: c_uint = 3;
pub const Button4: c_uint = 4;
pub const Button5: c_uint = 5;

// Notify modes
pub const NotifyNormal: c_int = 0;
pub const NotifyGrab: c_int = 1;
pub const NotifyUngrab: c_int = 2;
pub const NotifyWhileGrabbed: c_int = 3;
pub const NotifyHint: c_int = 1;

// Notify detail
pub const NotifyAncestor: c_int = 0;
pub const NotifyVirtual: c_int = 1;
pub const NotifyInferior: c_int = 2;
pub const NotifyNonlinear: c_int = 3;
pub const NotifyNonlinearVirtual: c_int = 4;
pub const NotifyPointer: c_int = 5;
pub const NotifyPointerRoot: c_int = 6;
pub const NotifyDetailNone: c_int = 7;

// Visibility notify
pub const VisibilityUnobscured: c_int = 0;
pub const VisibilityPartiallyObscured: c_int = 1;
pub const VisibilityFullyObscured: c_int = 2;

// Property notification
pub const PropertyNewValue: c_int = 0;
pub const PropertyDelete: c_int = 1;

// Mapping requests
pub const MappingModifier: c_int = 0;
pub const MappingKeyboard: c_int = 1;
pub const MappingPointer: c_int = 2;

// GrabPointer, GrabButton, GrabKeyboard, GrabKey Modes
pub const GrabModeSync: c_int = 0;
pub const GrabModeAsync: c_int = 1;

// GrabPointer, GrabKeyboard reply status
pub const GrabSuccess: c_int = 0;
pub const AlreadyGrabbed: c_int = 1;
pub const GrabInvalidTime: c_int = 2;
pub const GrabNotViewable: c_int = 3;
pub const GrabFrozen: c_int = 4;

// Used in SetInputFocus, GetInputFocus
pub const RevertToNone: c_int = 0;
pub const RevertToPointerRoot: c_int = 1;
pub const RevertToParent: c_int = 2;

// Error codes
pub const Success: c_int = 0;
pub const BadRequest: c_uchar = 1;
pub const BadValue: c_uchar = 2;
pub const BadWindow: c_uchar = 3;
pub const BadPixmap: c_uchar = 4;
pub const BadAtom: c_uchar = 5;
pub const BadCursor: c_uchar = 6;
pub const BadFont: c_uchar = 7;
pub const BadMatch: c_uchar = 8;
pub const BadDrawable: c_uchar = 9;
pub const BadAccess: c_uchar = 10;
pub const BadAlloc: c_uchar = 11;
pub const BadColor: c_uchar = 12;
pub const BadGC: c_uchar = 13;
pub const BadIDChoice: c_uchar = 14;
pub const BadName: c_uchar = 15;
pub const BadLength: c_uchar = 16;
pub const BadImplementation: c_uchar = 17;

// Window classes used by CreateWindow
pub const InputOutput: c_uint = 1;
pub const InputOnly: c_uint = 2;

// Window attributes for CreateWindow and ChangeWindowAttributes
pub const CWBackPixmap: c_ulong = 1 << 0;
pub const CWBackPixel: c_ulong = 1 << 1;
pub const CWBorderPixmap: c_ulong = 1 << 2;
pub const CWBorderPixel: c_ulong = 1 << 3;
pub const CWBitGravity: c_ulong = 1 << 4;
pub const CWWinGravity: c_ulong = 1 << 5;
pub const CWBackingStore: c_ulong = 1 << 6;
pub const CWBackingPlanes: c_ulong = 1 << 7;
pub const CWBackingPixel: c_ulong = 1 << 8;
pub const CWOverrideRedirect: c_ulong = 1 << 9;
pub const CWSaveUnder: c_ulong = 1 << 10;
pub const CWEventMask: c_ulong = 1 << 11;
pub const CWDontPropagate: c_ulong = 1 << 12;
pub const CWColormap: c_ulong = 1 << 13;
pub const CWCursor: c_ulong = 1 << 14;

// Used in GetWindowAttributes reply
pub const IsUnmapped: c_int = 0;
pub const IsUnviewable: c_int = 1;
pub const IsViewable: c_int = 2;

// Used in ChangeSaveSet / backing store
pub const NotUseful: c_int = 0;
pub const WhenMapped: c_int = 1;
pub const Always: c_int = 2;

// Property modes
pub const PropModeReplace: c_int = 0;
pub const PropModePrepend: c_int = 1;
pub const PropModeAppend: c_int = 2;

// GC components
pub const GCFunction: c_ulong = 1 << 0;
pub const GCPlaneMask: c_ulong = 1 << 1;
pub const GCForeground: c_ulong = 1 << 2;
pub const GCBackground: c_ulong = 1 << 3;
pub const GCLineWidth: c_ulong = 1 << 4;
pub const GCFont: c_ulong = 1 << 14;
pub const GCGraphicsExposures: c_ulong = 1 << 16;

// ImageFormat -- PutImage, GetImage
pub const XYBitmap: c_int = 0;
pub const XYPixmap: c_int = 1;
pub const ZPixmap: c_int = 2;

// Byte order used in imageByteOrder and bitmapBitOrder
pub const LSBFirst: c_int = 0;
pub const MSBFirst: c_int = 1;

// Used in CreateColormap
pub const AllocNone: c_int = 0;
pub const AllocAll: c_int = 1;

// Flags used in StoreNamedColor, StoreColors
pub const DoRed: c_char = 1 << 0;
pub const DoGreen: c_char = 1 << 1;
pub const DoBlue: c_char = 1 << 2;

// Display classes used in opening the connection
pub const StaticGray: c_int = 0;
pub const GrayScale: c_int = 1;
pub const StaticColor: c_int = 2;
pub const PseudoColor: c_int = 3;
pub const TrueColor: c_int = 4;
pub const DirectColor: c_int = 5;

/// `AllPlanes`
pub const AllPlanes: c_ulong = !0;

// Xlib.h: XEventsQueued modes
pub const QueuedAlready: c_int = 0;
pub const QueuedAfterReading: c_int = 1;
pub const QueuedAfterFlush: c_int = 2;

// Xlibint.h: display flags
pub const XlibDisplayIOError: c_ulong = 1 << 0;

// ---------------------------------------------------------------------------
// <X11/Xlib.h>: structures
// ---------------------------------------------------------------------------

/// Extensions need a way to hang private data on some structures.
#[repr(C)]
pub struct XExtData {
    pub number: c_int,
    pub next: *mut XExtData,
    pub free_private: Option<unsafe extern "C" fn(*mut XExtData) -> c_int>,
    pub private_data: XPointer,
}

/// Translation of `Visual`.
#[repr(C)]
pub struct Visual {
    pub ext_data: *mut XExtData,
    pub visualid: VisualID,
    pub class: c_int,
    pub red_mask: c_ulong,
    pub green_mask: c_ulong,
    pub blue_mask: c_ulong,
    pub bits_per_rgb: c_int,
    pub map_entries: c_int,
}

/// Translation of `Depth`.
#[repr(C)]
pub struct Depth {
    pub depth: c_int,
    pub nvisuals: c_int,
    pub visuals: *mut Visual,
}

/// Translation of `Screen`.
#[repr(C)]
pub struct Screen {
    pub ext_data: *mut XExtData,
    pub display: *mut Display,
    pub root: Window,
    pub width: c_int,
    pub height: c_int,
    pub mwidth: c_int,
    pub mheight: c_int,
    pub ndepths: c_int,
    pub depths: *mut Depth,
    pub root_depth: c_int,
    pub root_visual: *mut Visual,
    pub default_gc: GC,
    pub cmap: Colormap,
    pub white_pixel: c_ulong,
    pub black_pixel: c_ulong,
    pub max_maps: c_int,
    pub min_maps: c_int,
    pub backing_store: c_int,
    pub save_unders: Bool,
    pub root_input_mask: c_long,
}

/// Translation of `ScreenFormat`.
#[repr(C)]
pub struct ScreenFormat {
    pub ext_data: *mut XExtData,
    pub depth: c_int,
    pub bits_per_pixel: c_int,
    pub scanline_pad: c_int,
}

/// The public view of `struct _XDisplay` (`_XPrivDisplay` in Xlib.h, used
/// by the header macros), with `private16` named `flags` as in Xlibint.h.
#[repr(C)]
pub struct XPrivDisplay {
    pub ext_data: *mut XExtData,
    pub private1: *mut c_void,
    pub fd: c_int,
    pub private2: c_int,
    pub proto_major_version: c_int,
    pub proto_minor_version: c_int,
    pub vendor: *mut c_char,
    pub private3: XID,
    pub private4: XID,
    pub private5: XID,
    pub private6: c_int,
    pub resource_alloc: Option<unsafe extern "C" fn(*mut Display) -> XID>,
    pub byte_order: c_int,
    pub bitmap_unit: c_int,
    pub bitmap_pad: c_int,
    pub bitmap_bit_order: c_int,
    pub nformats: c_int,
    pub pixmap_format: *mut ScreenFormat,
    pub private8: c_int,
    pub release: c_int,
    pub private9: *mut c_void,
    pub private10: *mut c_void,
    pub qlen: c_int,
    pub last_request_read: c_ulong,
    pub request: c_ulong,
    pub private11: XPointer,
    pub private12: XPointer,
    pub private13: XPointer,
    pub private14: XPointer,
    pub max_request_size: c_uint,
    pub db: *mut _XrmHashBucketRec,
    pub private15: Option<unsafe extern "C" fn(*mut Display) -> c_int>,
    pub display_name: *mut c_char,
    pub default_screen: c_int,
    pub nscreens: c_int,
    pub screens: *mut Screen,
    pub motion_buffer: c_ulong,
    /// `private16` in Xlib.h, `flags` in Xlibint.h.
    pub flags: c_ulong,
    pub min_keycode: c_int,
    pub max_keycode: c_int,
    pub private17: XPointer,
    pub private18: XPointer,
    pub private19: c_int,
    pub xdefaults: *mut c_char,
}

/// Translation of `XSetWindowAttributes`.
#[repr(C)]
pub struct XSetWindowAttributes {
    pub background_pixmap: Pixmap,
    pub background_pixel: c_ulong,
    pub border_pixmap: Pixmap,
    pub border_pixel: c_ulong,
    pub bit_gravity: c_int,
    pub win_gravity: c_int,
    pub backing_store: c_int,
    pub backing_planes: c_ulong,
    pub backing_pixel: c_ulong,
    pub save_under: Bool,
    pub event_mask: c_long,
    pub do_not_propagate_mask: c_long,
    pub override_redirect: Bool,
    pub colormap: Colormap,
    pub cursor: Cursor,
}

/// Translation of `XWindowAttributes`.
#[repr(C)]
pub struct XWindowAttributes {
    pub x: c_int,
    pub y: c_int,
    pub width: c_int,
    pub height: c_int,
    pub border_width: c_int,
    pub depth: c_int,
    pub visual: *mut Visual,
    pub root: Window,
    pub class: c_int,
    pub bit_gravity: c_int,
    pub win_gravity: c_int,
    pub backing_store: c_int,
    pub backing_planes: c_ulong,
    pub backing_pixel: c_ulong,
    pub save_under: Bool,
    pub colormap: Colormap,
    pub map_installed: Bool,
    pub map_state: c_int,
    pub all_event_masks: c_long,
    pub your_event_mask: c_long,
    pub do_not_propagate_mask: c_long,
    pub override_redirect: Bool,
    pub screen: *mut Screen,
}

/// Translation of `XPixmapFormatValues`.
#[repr(C)]
pub struct XPixmapFormatValues {
    pub depth: c_int,
    pub bits_per_pixel: c_int,
    pub scanline_pad: c_int,
}

/// Translation of `XGCValues`.
#[repr(C)]
pub struct XGCValues {
    pub function: c_int,
    pub plane_mask: c_ulong,
    pub foreground: c_ulong,
    pub background: c_ulong,
    pub line_width: c_int,
    pub line_style: c_int,
    pub cap_style: c_int,
    pub join_style: c_int,
    pub fill_style: c_int,
    pub fill_rule: c_int,
    pub arc_mode: c_int,
    pub tile: Pixmap,
    pub stipple: Pixmap,
    pub ts_x_origin: c_int,
    pub ts_y_origin: c_int,
    pub font: Font,
    pub subwindow_mode: c_int,
    pub graphics_exposures: Bool,
    pub clip_x_origin: c_int,
    pub clip_y_origin: c_int,
    pub clip_mask: Pixmap,
    pub dash_offset: c_int,
    pub dashes: c_char,
}

/// The image manipulation routines of an `XImage` (`struct funcs`).
#[repr(C)]
pub struct XImageFuncs {
    pub create_image: Option<
        unsafe extern "C" fn(
            *mut Display,
            *mut Visual,
            c_uint,
            c_int,
            c_int,
            *mut c_char,
            c_uint,
            c_uint,
            c_int,
            c_int,
        ) -> *mut XImage,
    >,
    pub destroy_image: Option<unsafe extern "C" fn(*mut XImage) -> c_int>,
    pub get_pixel: Option<unsafe extern "C" fn(*mut XImage, c_int, c_int) -> c_ulong>,
    pub put_pixel: Option<unsafe extern "C" fn(*mut XImage, c_int, c_int, c_ulong) -> c_int>,
    pub sub_image:
        Option<unsafe extern "C" fn(*mut XImage, c_int, c_int, c_uint, c_uint) -> *mut XImage>,
    pub add_pixel: Option<unsafe extern "C" fn(*mut XImage, c_long) -> c_int>,
}

/// Translation of `XImage`.
#[repr(C)]
pub struct XImage {
    pub width: c_int,
    pub height: c_int,
    pub xoffset: c_int,
    pub format: c_int,
    pub data: *mut c_char,
    pub byte_order: c_int,
    pub bitmap_unit: c_int,
    pub bitmap_bit_order: c_int,
    pub bitmap_pad: c_int,
    pub depth: c_int,
    pub bytes_per_line: c_int,
    pub bits_per_pixel: c_int,
    pub red_mask: c_ulong,
    pub green_mask: c_ulong,
    pub blue_mask: c_ulong,
    pub obdata: XPointer,
    pub f: XImageFuncs,
}

/// Translation of `XColor`.
#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct XColor {
    pub pixel: c_ulong,
    pub red: c_ushort,
    pub green: c_ushort,
    pub blue: c_ushort,
    pub flags: c_char,
    pub pad: c_char,
}

/// Translation of `XRectangle`.
#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct XRectangle {
    pub x: c_short,
    pub y: c_short,
    pub width: c_ushort,
    pub height: c_ushort,
}

/// Translation of `XPoint`.
#[repr(C)]
#[derive(Clone, Copy, Default, PartialEq, Eq, Debug)]
pub struct XPoint {
    pub x: c_short,
    pub y: c_short,
}

/// Translation of `XModifierKeymap`.
#[repr(C)]
pub struct XModifierKeymap {
    pub max_keypermod: c_int,
    pub modifiermap: *mut KeyCode,
}

/// Translation of `XCharStruct`.
#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct XCharStruct {
    pub lbearing: c_short,
    pub rbearing: c_short,
    pub width: c_short,
    pub ascent: c_short,
    pub descent: c_short,
    pub attributes: c_ushort,
}

/// Translation of `XFontProp`.
#[repr(C)]
pub struct XFontProp {
    pub name: Atom,
    pub card32: c_ulong,
}

/// Translation of `XFontStruct`.
#[repr(C)]
pub struct XFontStruct {
    pub ext_data: *mut XExtData,
    pub fid: Font,
    pub direction: c_uint,
    pub min_char_or_byte2: c_uint,
    pub max_char_or_byte2: c_uint,
    pub min_byte1: c_uint,
    pub max_byte1: c_uint,
    pub all_chars_exist: Bool,
    pub default_char: c_uint,
    pub n_properties: c_int,
    pub properties: *mut XFontProp,
    pub min_bounds: XCharStruct,
    pub max_bounds: XCharStruct,
    pub per_char: *mut XCharStruct,
    pub ascent: c_int,
    pub descent: c_int,
}

/// Translation of `XFontSetExtents`.
#[repr(C)]
pub struct XFontSetExtents {
    pub max_ink_extent: XRectangle,
    pub max_logical_extent: XRectangle,
}

/// Translation of `XrmValue`.
#[repr(C)]
pub struct XrmValue {
    pub size: c_uint,
    pub addr: XPointer,
}

// ---------------------------------------------------------------------------
// <X11/Xlib.h>: events
// ---------------------------------------------------------------------------

/// Translation of `XAnyEvent`.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct XAnyEvent {
    pub type_: c_int,
    pub serial: c_ulong,
    pub send_event: Bool,
    pub display: *mut Display,
    pub window: Window,
}

/// Translation of `XKeyEvent`.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct XKeyEvent {
    pub type_: c_int,
    pub serial: c_ulong,
    pub send_event: Bool,
    pub display: *mut Display,
    pub window: Window,
    pub root: Window,
    pub subwindow: Window,
    pub time: Time,
    pub x: c_int,
    pub y: c_int,
    pub x_root: c_int,
    pub y_root: c_int,
    pub state: c_uint,
    pub keycode: c_uint,
    pub same_screen: Bool,
}

/// Translation of `XButtonEvent`.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct XButtonEvent {
    pub type_: c_int,
    pub serial: c_ulong,
    pub send_event: Bool,
    pub display: *mut Display,
    pub window: Window,
    pub root: Window,
    pub subwindow: Window,
    pub time: Time,
    pub x: c_int,
    pub y: c_int,
    pub x_root: c_int,
    pub y_root: c_int,
    pub state: c_uint,
    pub button: c_uint,
    pub same_screen: Bool,
}

/// Translation of `XMotionEvent`.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct XMotionEvent {
    pub type_: c_int,
    pub serial: c_ulong,
    pub send_event: Bool,
    pub display: *mut Display,
    pub window: Window,
    pub root: Window,
    pub subwindow: Window,
    pub time: Time,
    pub x: c_int,
    pub y: c_int,
    pub x_root: c_int,
    pub y_root: c_int,
    pub state: c_uint,
    pub is_hint: c_char,
    pub same_screen: Bool,
}

/// Translation of `XCrossingEvent`.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct XCrossingEvent {
    pub type_: c_int,
    pub serial: c_ulong,
    pub send_event: Bool,
    pub display: *mut Display,
    pub window: Window,
    pub root: Window,
    pub subwindow: Window,
    pub time: Time,
    pub x: c_int,
    pub y: c_int,
    pub x_root: c_int,
    pub y_root: c_int,
    pub mode: c_int,
    pub detail: c_int,
    pub same_screen: Bool,
    pub focus: Bool,
    pub state: c_uint,
}

/// Translation of `XFocusChangeEvent`.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct XFocusChangeEvent {
    pub type_: c_int,
    pub serial: c_ulong,
    pub send_event: Bool,
    pub display: *mut Display,
    pub window: Window,
    pub mode: c_int,
    pub detail: c_int,
}

/// Translation of `XKeymapEvent`.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct XKeymapEvent {
    pub type_: c_int,
    pub serial: c_ulong,
    pub send_event: Bool,
    pub display: *mut Display,
    pub window: Window,
    pub key_vector: [c_char; 32],
}

/// Translation of `XExposeEvent`.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct XExposeEvent {
    pub type_: c_int,
    pub serial: c_ulong,
    pub send_event: Bool,
    pub display: *mut Display,
    pub window: Window,
    pub x: c_int,
    pub y: c_int,
    pub width: c_int,
    pub height: c_int,
    pub count: c_int,
}

/// Translation of `XVisibilityEvent`.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct XVisibilityEvent {
    pub type_: c_int,
    pub serial: c_ulong,
    pub send_event: Bool,
    pub display: *mut Display,
    pub window: Window,
    pub state: c_int,
}

/// Translation of `XDestroyWindowEvent`.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct XDestroyWindowEvent {
    pub type_: c_int,
    pub serial: c_ulong,
    pub send_event: Bool,
    pub display: *mut Display,
    pub event: Window,
    pub window: Window,
}

/// Translation of `XUnmapEvent`.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct XUnmapEvent {
    pub type_: c_int,
    pub serial: c_ulong,
    pub send_event: Bool,
    pub display: *mut Display,
    pub event: Window,
    pub window: Window,
    pub from_configure: Bool,
}

/// Translation of `XMapEvent`.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct XMapEvent {
    pub type_: c_int,
    pub serial: c_ulong,
    pub send_event: Bool,
    pub display: *mut Display,
    pub event: Window,
    pub window: Window,
    pub override_redirect: Bool,
}

/// Translation of `XReparentEvent`.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct XReparentEvent {
    pub type_: c_int,
    pub serial: c_ulong,
    pub send_event: Bool,
    pub display: *mut Display,
    pub event: Window,
    pub window: Window,
    pub parent: Window,
    pub x: c_int,
    pub y: c_int,
    pub override_redirect: Bool,
}

/// Translation of `XConfigureEvent`.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct XConfigureEvent {
    pub type_: c_int,
    pub serial: c_ulong,
    pub send_event: Bool,
    pub display: *mut Display,
    pub event: Window,
    pub window: Window,
    pub x: c_int,
    pub y: c_int,
    pub width: c_int,
    pub height: c_int,
    pub border_width: c_int,
    pub above: Window,
    pub override_redirect: Bool,
}

impl Default for XConfigureEvent {
    fn default() -> Self {
        XConfigureEvent {
            type_: 0,
            serial: 0,
            send_event: 0,
            display: std::ptr::null_mut(),
            event: 0,
            window: 0,
            x: 0,
            y: 0,
            width: 0,
            height: 0,
            border_width: 0,
            above: 0,
            override_redirect: 0,
        }
    }
}

/// Translation of `XPropertyEvent`.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct XPropertyEvent {
    pub type_: c_int,
    pub serial: c_ulong,
    pub send_event: Bool,
    pub display: *mut Display,
    pub window: Window,
    pub atom: Atom,
    pub time: Time,
    pub state: c_int,
}

/// Translation of `XSelectionClearEvent`.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct XSelectionClearEvent {
    pub type_: c_int,
    pub serial: c_ulong,
    pub send_event: Bool,
    pub display: *mut Display,
    pub window: Window,
    pub selection: Atom,
    pub time: Time,
}

/// Translation of `XSelectionRequestEvent`.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct XSelectionRequestEvent {
    pub type_: c_int,
    pub serial: c_ulong,
    pub send_event: Bool,
    pub display: *mut Display,
    pub owner: Window,
    pub requestor: Window,
    pub selection: Atom,
    pub target: Atom,
    pub property: Atom,
    pub time: Time,
}

/// Translation of `XSelectionEvent`.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct XSelectionEvent {
    pub type_: c_int,
    pub serial: c_ulong,
    pub send_event: Bool,
    pub display: *mut Display,
    pub requestor: Window,
    pub selection: Atom,
    pub target: Atom,
    pub property: Atom,
    pub time: Time,
}

/// The `data` union of `XClientMessageEvent` (as longs; the bytes and
/// shorts views are methods).
#[repr(C)]
#[derive(Clone, Copy)]
pub struct ClientMessageData {
    pub l: [c_long; 5],
}

impl ClientMessageData {
    /// The `data.b` view.
    pub fn b(&self) -> &[c_char; 20] {
        // SAFETY: `[c_long; 5]` is at least 20 bytes and suitably aligned
        // for bytes; every bit pattern is a valid c_char.
        unsafe { &*(self.l.as_ptr() as *const [c_char; 20]) }
    }
    /// The mutable `data.b` view.
    pub fn b_mut(&mut self) -> &mut [c_char; 20] {
        // SAFETY: as in `b()`.
        unsafe { &mut *(self.l.as_mut_ptr() as *mut [c_char; 20]) }
    }
}

/// Translation of `XClientMessageEvent`.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct XClientMessageEvent {
    pub type_: c_int,
    pub serial: c_ulong,
    pub send_event: Bool,
    pub display: *mut Display,
    pub window: Window,
    pub message_type: Atom,
    pub format: c_int,
    pub data: ClientMessageData,
}

/// Translation of `XMappingEvent`.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct XMappingEvent {
    pub type_: c_int,
    pub serial: c_ulong,
    pub send_event: Bool,
    pub display: *mut Display,
    pub window: Window,
    pub request: c_int,
    pub first_keycode: c_int,
    pub count: c_int,
}

/// Translation of `XErrorEvent`.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct XErrorEvent {
    pub type_: c_int,
    pub display: *mut Display,
    pub resourceid: XID,
    pub serial: c_ulong,
    pub error_code: c_uchar,
    pub request_code: c_uchar,
    pub minor_code: c_uchar,
}

/// Translation of `XGenericEvent`.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct XGenericEvent {
    pub type_: c_int,
    pub serial: c_ulong,
    pub send_event: Bool,
    pub display: *mut Display,
    pub extension: c_int,
    pub evtype: c_int,
}

/// Translation of `XGenericEventCookie`.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct XGenericEventCookie {
    pub type_: c_int,
    pub serial: c_ulong,
    pub send_event: Bool,
    pub display: *mut Display,
    pub extension: c_int,
    pub evtype: c_int,
    pub cookie: c_uint,
    pub data: *mut c_void,
}

/// Translation of `XEvent`: a union of the event structures, always 24
/// longs in size. Every member is plain data (integers and raw pointers),
/// so any bit pattern is valid for any member; the accessors below read
/// them safely.
#[repr(C)]
#[derive(Clone, Copy)]
pub union XEvent {
    pub type_: c_int,
    pub xany: XAnyEvent,
    pub xkey: XKeyEvent,
    pub xbutton: XButtonEvent,
    pub xmotion: XMotionEvent,
    pub xcrossing: XCrossingEvent,
    pub xfocus: XFocusChangeEvent,
    pub xexpose: XExposeEvent,
    pub xvisibility: XVisibilityEvent,
    pub xdestroywindow: XDestroyWindowEvent,
    pub xunmap: XUnmapEvent,
    pub xmap: XMapEvent,
    pub xreparent: XReparentEvent,
    pub xconfigure: XConfigureEvent,
    pub xproperty: XPropertyEvent,
    pub xselectionclear: XSelectionClearEvent,
    pub xselectionrequest: XSelectionRequestEvent,
    pub xselection: XSelectionEvent,
    pub xclient: XClientMessageEvent,
    pub xmapping: XMappingEvent,
    pub xerror: XErrorEvent,
    pub xkeymap: XKeymapEvent,
    pub xgeneric: XGenericEvent,
    pub xcookie: XGenericEventCookie,
    pub pad: [c_long; 24],
}

macro_rules! xevent_views {
    ($($get:ident, $get_mut:ident: $field:ident: $ty:ty;)*) => {
        impl XEvent {
            $(
                #[doc = concat!("The `", stringify!($field), "` member.")]
                #[inline]
                pub fn $get(&self) -> &$ty {
                    // SAFETY: all members are plain data inside the
                    // 24-long padding; any bit pattern is valid.
                    unsafe { &self.$field }
                }
                #[doc = concat!("The `", stringify!($field), "` member, mutably.")]
                #[inline]
                pub fn $get_mut(&mut self) -> &mut $ty {
                    // SAFETY: as above.
                    unsafe { &mut self.$field }
                }
            )*
        }
    };
}

xevent_views! {
    any, any_mut: xany: XAnyEvent;
    key, key_mut: xkey: XKeyEvent;
    button, button_mut: xbutton: XButtonEvent;
    motion, motion_mut: xmotion: XMotionEvent;
    crossing, crossing_mut: xcrossing: XCrossingEvent;
    focus, focus_mut: xfocus: XFocusChangeEvent;
    expose, expose_mut: xexpose: XExposeEvent;
    visibility, visibility_mut: xvisibility: XVisibilityEvent;
    destroywindow, destroywindow_mut: xdestroywindow: XDestroyWindowEvent;
    unmap, unmap_mut: xunmap: XUnmapEvent;
    map, map_mut: xmap: XMapEvent;
    reparent, reparent_mut: xreparent: XReparentEvent;
    configure, configure_mut: xconfigure: XConfigureEvent;
    property, property_mut: xproperty: XPropertyEvent;
    selectionclear, selectionclear_mut: xselectionclear: XSelectionClearEvent;
    selectionrequest, selectionrequest_mut: xselectionrequest: XSelectionRequestEvent;
    selection, selection_mut: xselection: XSelectionEvent;
    client, client_mut: xclient: XClientMessageEvent;
    mapping, mapping_mut: xmapping: XMappingEvent;
    error, error_mut: xerror: XErrorEvent;
    keymap, keymap_mut: xkeymap: XKeymapEvent;
    generic, generic_mut: xgeneric: XGenericEvent;
    cookie, cookie_mut: xcookie: XGenericEventCookie;
}

impl XEvent {
    /// An all-zero event (`SDL_zero(xevent)`).
    pub fn zeroed() -> XEvent {
        XEvent { pad: [0; 24] }
    }

    /// The `type` member.
    #[inline]
    pub fn get_type(&self) -> c_int {
        // SAFETY: every member starts with the int `type`.
        unsafe { self.type_ }
    }

    /// Reinterpret the event as an extension event structure `T` (the C
    /// casts `(XkbEvent *)xevent`, `(XRRNotifyEvent *)xevent`...).
    ///
    /// # Safety
    ///
    /// `T` must be a plain-data `#[repr(C)]` event structure no larger
    /// than `XEvent`.
    pub unsafe fn cast<T>(&self) -> &T {
        const { assert!(std::mem::size_of::<T>() <= std::mem::size_of::<XEvent>()) };
        // SAFETY: the caller guarantees T is plain data that fits; XEvent
        // is aligned for longs and pointers, which covers T's members.
        unsafe { &*(self as *const XEvent as *const T) }
    }
}

pub type XErrorHandler = Option<unsafe extern "C" fn(*mut Display, *mut XErrorEvent) -> c_int>;
pub type XIOErrorHandler = Option<unsafe extern "C" fn(*mut Display) -> c_int>;
pub type XIfEventPredicate =
    Option<unsafe extern "C" fn(*mut Display, *mut XEvent, XPointer) -> Bool>;

// ---------------------------------------------------------------------------
// <X11/Xutil.h>
// ---------------------------------------------------------------------------

// Bitmask returned by XParseGeometry / size hints flags
pub const USPosition: c_long = 1 << 0;
pub const USSize: c_long = 1 << 1;
pub const PPosition: c_long = 1 << 2;
pub const PSize: c_long = 1 << 3;
pub const PMinSize: c_long = 1 << 4;
pub const PMaxSize: c_long = 1 << 5;
pub const PResizeInc: c_long = 1 << 6;
pub const PAspect: c_long = 1 << 7;
pub const PBaseSize: c_long = 1 << 8;
pub const PWinGravity: c_long = 1 << 9;

/// The `min_aspect`/`max_aspect` of `XSizeHints`.
#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct XAspect {
    pub x: c_int,
    pub y: c_int,
}

/// Translation of `XSizeHints`.
#[repr(C)]
pub struct XSizeHints {
    pub flags: c_long,
    pub x: c_int,
    pub y: c_int,
    pub width: c_int,
    pub height: c_int,
    pub min_width: c_int,
    pub min_height: c_int,
    pub max_width: c_int,
    pub max_height: c_int,
    pub width_inc: c_int,
    pub height_inc: c_int,
    pub min_aspect: XAspect,
    pub max_aspect: XAspect,
    pub base_width: c_int,
    pub base_height: c_int,
    pub win_gravity: c_int,
}

// definition for flags of XWMHints
pub const InputHint: c_long = 1 << 0;
pub const StateHint: c_long = 1 << 1;
pub const IconPixmapHint: c_long = 1 << 2;
pub const IconWindowHint: c_long = 1 << 3;
pub const IconPositionHint: c_long = 1 << 4;
pub const IconMaskHint: c_long = 1 << 5;
pub const WindowGroupHint: c_long = 1 << 6;
pub const XUrgencyHint: c_long = 1 << 8;

// definitions for initial window state
pub const WithdrawnState: c_int = 0;
pub const NormalState: c_int = 1;
pub const IconicState: c_int = 3;

/// Translation of `XWMHints`.
#[repr(C)]
pub struct XWMHints {
    pub flags: c_long,
    pub input: Bool,
    pub initial_state: c_int,
    pub icon_pixmap: Pixmap,
    pub icon_window: Window,
    pub icon_x: c_int,
    pub icon_y: c_int,
    pub icon_mask: Pixmap,
    pub window_group: XID,
}

/// Translation of `XTextProperty`.
#[repr(C)]
pub struct XTextProperty {
    pub value: *mut c_uchar,
    pub encoding: Atom,
    pub format: c_int,
    pub nitems: c_ulong,
}

impl Default for XTextProperty {
    fn default() -> Self {
        XTextProperty {
            value: std::ptr::null_mut(),
            encoding: 0,
            format: 0,
            nitems: 0,
        }
    }
}

/// Translation of `XICCEncodingStyle`.
pub type XICCEncodingStyle = c_int;
pub const XStringStyle: XICCEncodingStyle = 0;
pub const XCompoundTextStyle: XICCEncodingStyle = 1;
pub const XTextStyle: XICCEncodingStyle = 2;
pub const XStdICCTextStyle: XICCEncodingStyle = 3;
pub const XUTF8StringStyle: XICCEncodingStyle = 4;

/// Translation of `XClassHint`.
#[repr(C)]
pub struct XClassHint {
    pub res_name: *mut c_char,
    pub res_class: *mut c_char,
}

/// Translation of `XComposeStatus`.
#[repr(C)]
pub struct XComposeStatus {
    pub compose_ptr: XPointer,
    pub chars_matched: c_int,
}

/// Translation of `XVisualInfo`.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct XVisualInfo {
    pub visual: *mut Visual,
    pub visualid: VisualID,
    pub screen: c_int,
    pub depth: c_int,
    pub class: c_int,
    pub red_mask: c_ulong,
    pub green_mask: c_ulong,
    pub blue_mask: c_ulong,
    pub colormap_size: c_int,
    pub bits_per_rgb: c_int,
}

impl Default for XVisualInfo {
    fn default() -> Self {
        XVisualInfo {
            visual: std::ptr::null_mut(),
            visualid: 0,
            screen: 0,
            depth: 0,
            class: 0,
            red_mask: 0,
            green_mask: 0,
            blue_mask: 0,
            colormap_size: 0,
            bits_per_rgb: 0,
        }
    }
}

pub const VisualNoMask: c_long = 0x0;
pub const VisualIDMask: c_long = 0x1;
pub const VisualScreenMask: c_long = 0x2;
pub const VisualDepthMask: c_long = 0x4;
pub const VisualClassMask: c_long = 0x8;

// Return values from XLookupString / Xutf8LookupString status
pub const XBufferOverflow: Status = -1;
pub const XLookupNone: Status = 1;
pub const XLookupChars: Status = 2;
pub const XLookupKeySym: Status = 3;
pub const XLookupBoth: Status = 4;

// ---------------------------------------------------------------------------
// XIM (<X11/Xlib.h>)
// ---------------------------------------------------------------------------

pub type XIMStyle = c_ulong;
pub const XIMPreeditArea: XIMStyle = 0x0001;
pub const XIMPreeditCallbacks: XIMStyle = 0x0002;
pub const XIMPreeditPosition: XIMStyle = 0x0004;
pub const XIMPreeditNothing: XIMStyle = 0x0008;
pub const XIMPreeditNone: XIMStyle = 0x0010;
pub const XIMStatusArea: XIMStyle = 0x0100;
pub const XIMStatusCallbacks: XIMStyle = 0x0200;
pub const XIMStatusNothing: XIMStyle = 0x0400;
pub const XIMStatusNone: XIMStyle = 0x0800;

pub type XIMFeedback = c_ulong;
pub const XIMReverse: XIMFeedback = 1;
pub const XIMUnderline: XIMFeedback = 1 << 1;
pub const XIMHighlight: XIMFeedback = 1 << 2;
pub const XIMPrimary: XIMFeedback = 1 << 5;
pub const XIMSecondary: XIMFeedback = 1 << 6;
pub const XIMTertiary: XIMFeedback = 1 << 7;
pub const XIMVisibleToForward: XIMFeedback = 1 << 8;
pub const XIMVisibleToBackword: XIMFeedback = 1 << 9;
pub const XIMVisibleToCenter: XIMFeedback = 1 << 10;

pub type XIMProc = Option<unsafe extern "C" fn(XIM, XPointer, XPointer)>;
pub type XICProc = Option<unsafe extern "C" fn(XIC, XPointer, XPointer) -> Bool>;
pub type XIDProc = Option<unsafe extern "C" fn(*mut Display, XPointer, XPointer)>;

/// Translation of `XIMCallback`.
#[repr(C)]
pub struct XIMCallback {
    pub client_data: XPointer,
    pub callback: XIMProc,
}

/// Translation of `XICCallback`.
#[repr(C)]
pub struct XICCallback {
    pub client_data: XPointer,
    pub callback: XICProc,
}

/// Translation of `XIMText` (its `string` union as a byte pointer).
#[repr(C)]
pub struct XIMText {
    pub length: c_ushort,
    pub feedback: *mut XIMFeedback,
    pub encoding_is_wchar: Bool,
    pub string: *mut c_char,
}

/// Translation of `XIMPreeditDrawCallbackStruct`.
#[repr(C)]
pub struct XIMPreeditDrawCallbackStruct {
    pub caret: c_int,
    pub chg_first: c_int,
    pub chg_length: c_int,
    pub text: *mut XIMText,
}

pub type XIMCaretDirection = c_int;
pub const XIMForwardChar: XIMCaretDirection = 0;
pub const XIMBackwardChar: XIMCaretDirection = 1;
pub const XIMForwardWord: XIMCaretDirection = 2;
pub const XIMBackwardWord: XIMCaretDirection = 3;
pub const XIMCaretUp: XIMCaretDirection = 4;
pub const XIMCaretDown: XIMCaretDirection = 5;
pub const XIMNextLine: XIMCaretDirection = 6;
pub const XIMPreviousLine: XIMCaretDirection = 7;
pub const XIMLineStart: XIMCaretDirection = 8;
pub const XIMLineEnd: XIMCaretDirection = 9;
pub const XIMAbsolutePosition: XIMCaretDirection = 10;
pub const XIMDontChange: XIMCaretDirection = 11;

pub type XIMCaretStyle = c_int;

/// Translation of `XIMPreeditCaretCallbackStruct`.
#[repr(C)]
pub struct XIMPreeditCaretCallbackStruct {
    pub position: c_int,
    pub direction: XIMCaretDirection,
    pub style: XIMCaretStyle,
}

/// Translation of `XIMStyles`.
#[repr(C)]
pub struct XIMStyles {
    pub count_styles: c_ushort,
    pub supported_styles: *mut XIMStyle,
}

pub const XNVaNestedList: &std::ffi::CStr = c"XNVaNestedList";
pub const XNQueryInputStyle: &std::ffi::CStr = c"queryInputStyle";
pub const XNClientWindow: &std::ffi::CStr = c"clientWindow";
pub const XNInputStyle: &std::ffi::CStr = c"inputStyle";
pub const XNFocusWindow: &std::ffi::CStr = c"focusWindow";
pub const XNFilterEvents: &std::ffi::CStr = c"filterEvents";
pub const XNPreeditAttributes: &std::ffi::CStr = c"preeditAttributes";
pub const XNStatusAttributes: &std::ffi::CStr = c"statusAttributes";
pub const XNSpotLocation: &std::ffi::CStr = c"spotLocation";
pub const XNPreeditStartCallback: &std::ffi::CStr = c"preeditStartCallback";
pub const XNPreeditDoneCallback: &std::ffi::CStr = c"preeditDoneCallback";
pub const XNPreeditDrawCallback: &std::ffi::CStr = c"preeditDrawCallback";
pub const XNPreeditCaretCallback: &std::ffi::CStr = c"preeditCaretCallback";
pub const XNDestroyCallback: &std::ffi::CStr = c"destroyCallback";
pub const XNResetState: &std::ffi::CStr = c"resetState";

// ---------------------------------------------------------------------------
// <X11/Xatom.h>
// ---------------------------------------------------------------------------

pub const XA_PRIMARY: Atom = 1;
pub const XA_SECONDARY: Atom = 2;
pub const XA_ARC: Atom = 3;
pub const XA_ATOM: Atom = 4;
pub const XA_BITMAP: Atom = 5;
pub const XA_CARDINAL: Atom = 6;
pub const XA_COLORMAP: Atom = 7;
pub const XA_CURSOR: Atom = 8;
pub const XA_CUT_BUFFER0: Atom = 9;
pub const XA_DRAWABLE: Atom = 17;
pub const XA_FONT: Atom = 18;
pub const XA_INTEGER: Atom = 19;
pub const XA_PIXMAP: Atom = 20;
pub const XA_POINT: Atom = 21;
pub const XA_RECTANGLE: Atom = 22;
pub const XA_RESOURCE_MANAGER: Atom = 23;
pub const XA_STRING: Atom = 31;
pub const XA_VISUALID: Atom = 32;
pub const XA_WINDOW: Atom = 33;
pub const XA_WM_COMMAND: Atom = 34;
pub const XA_WM_HINTS: Atom = 35;
pub const XA_WM_CLIENT_MACHINE: Atom = 36;
pub const XA_WM_ICON_NAME: Atom = 37;
pub const XA_WM_ICON_SIZE: Atom = 38;
pub const XA_WM_NAME: Atom = 39;
pub const XA_WM_NORMAL_HINTS: Atom = 40;
pub const XA_WM_SIZE_HINTS: Atom = 41;
pub const XA_WM_ZOOM_HINTS: Atom = 42;
pub const XA_WM_CLASS: Atom = 67;
pub const XA_WM_TRANSIENT_FOR: Atom = 68;

// ---------------------------------------------------------------------------
// <X11/cursorfont.h>
// ---------------------------------------------------------------------------

pub const XC_X_cursor: c_uint = 0;
pub const XC_bottom_left_corner: c_uint = 12;
pub const XC_bottom_right_corner: c_uint = 14;
pub const XC_bottom_side: c_uint = 16;
pub const XC_cross: c_uint = 30;
pub const XC_fleur: c_uint = 52;
pub const XC_hand2: c_uint = 60;
pub const XC_left_ptr: c_uint = 68;
pub const XC_left_side: c_uint = 70;
pub const XC_pirate: c_uint = 88;
pub const XC_question_arrow: c_uint = 92;
pub const XC_right_side: c_uint = 96;
pub const XC_sb_h_double_arrow: c_uint = 108;
pub const XC_sb_v_double_arrow: c_uint = 116;
pub const XC_tcross: c_uint = 130;
pub const XC_top_left_corner: c_uint = 134;
pub const XC_top_right_corner: c_uint = 136;
pub const XC_top_side: c_uint = 138;
pub const XC_watch: c_uint = 150;
pub const XC_xterm: c_uint = 152;

// ---------------------------------------------------------------------------
// XKB (<X11/XKBlib.h>, <X11/extensions/XKB.h>, <X11/extensions/XKBstr.h>)
// ---------------------------------------------------------------------------

pub const XkbMajorVersion: c_int = 1;
pub const XkbMinorVersion: c_int = 0;
pub const XkbUseCoreKbd: c_uint = 0x0100;
pub const XkbNumKbdGroups: usize = 4;
pub const XkbNumVirtualMods: usize = 16;
pub const XkbNumIndicators: usize = 32;

// XKB event codes
pub const XkbNewKeyboardNotify: c_int = 0;
pub const XkbMapNotify: c_int = 1;
pub const XkbStateNotify: c_int = 2;

pub const XkbNewKeyboardNotifyMask: c_ulong = 1 << 0;
pub const XkbMapNotifyMask: c_ulong = 1 << 1;
pub const XkbStateNotifyMask: c_ulong = 1 << 2;

pub const XkbModifierStateMask: c_ulong = 1 << 0;
pub const XkbGroupStateMask: c_ulong = 1 << 4;

// XkbGetMap / XkbGetUpdatedMap components
pub const XkbKeyTypesMask: c_uint = 1 << 0;
pub const XkbKeySymsMask: c_uint = 1 << 1;
pub const XkbModifierMapMask: c_uint = 1 << 2;
pub const XkbVirtualModsMask: c_uint = 1 << 6;
pub const XkbAllClientInfoMask: c_uint = XkbKeyTypesMask | XkbKeySymsMask | XkbModifierMapMask;

// XkbGetNames components
pub const XkbVirtualModNamesMask: c_uint = 1 << 11;

// group_info out-of-range actions
pub const XkbWrapIntoRange: c_uchar = 0x00;
pub const XkbClampIntoRange: c_uchar = 0x40;
pub const XkbRedirectIntoRange: c_uchar = 0x80;

/// Translation of `XkbStateRec`.
#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct XkbStateRec {
    pub group: c_uchar,
    pub locked_group: c_uchar,
    pub base_group: c_ushort,
    pub latched_group: c_ushort,
    pub mods: c_uchar,
    pub base_mods: c_uchar,
    pub latched_mods: c_uchar,
    pub locked_mods: c_uchar,
    pub compat_state: c_uchar,
    pub grab_mods: c_uchar,
    pub compat_grab_mods: c_uchar,
    pub lookup_mods: c_uchar,
    pub compat_lookup_mods: c_uchar,
    pub ptr_buttons: c_ushort,
}

/// Translation of `XkbModsRec`.
#[repr(C)]
pub struct XkbModsRec {
    pub mask: c_uchar,
    pub real_mods: c_uchar,
    pub vmods: c_ushort,
}

/// Translation of `XkbKTMapEntryRec`.
#[repr(C)]
pub struct XkbKTMapEntryRec {
    pub active: Bool,
    pub level: c_uchar,
    pub mods: XkbModsRec,
}

/// Translation of `XkbKeyTypeRec`.
#[repr(C)]
pub struct XkbKeyTypeRec {
    pub mods: XkbModsRec,
    pub num_levels: c_uchar,
    pub map_count: c_uchar,
    pub map: *mut XkbKTMapEntryRec,
    pub preserve: *mut XkbModsRec,
    pub name: Atom,
    pub level_names: *mut Atom,
}

/// Translation of `XkbSymMapRec`.
#[repr(C)]
pub struct XkbSymMapRec {
    pub kt_index: [c_uchar; XkbNumKbdGroups],
    pub group_info: c_uchar,
    pub width: c_uchar,
    pub offset: c_ushort,
}

/// Translation of `XkbClientMapRec`.
#[repr(C)]
pub struct XkbClientMapRec {
    pub size_types: c_uchar,
    pub num_types: c_uchar,
    pub types: *mut XkbKeyTypeRec,
    pub size_syms: c_ushort,
    pub num_syms: c_ushort,
    pub syms: *mut KeySym,
    pub key_sym_map: *mut XkbSymMapRec,
    pub modmap: *mut c_uchar,
}

/// Translation of `XkbServerMapRec`.
#[repr(C)]
pub struct XkbServerMapRec {
    pub num_acts: c_ushort,
    pub size_acts: c_ushort,
    pub acts: *mut c_void,
    pub behaviors: *mut c_void,
    pub key_acts: *mut c_ushort,
    pub explicit: *mut c_uchar,
    pub vmods: [c_uchar; XkbNumVirtualMods],
    pub vmodmap: *mut c_ushort,
}

/// Translation of `XkbNamesRec`.
#[repr(C)]
pub struct XkbNamesRec {
    pub keycodes: Atom,
    pub geometry: Atom,
    pub symbols: Atom,
    pub types: Atom,
    pub compat: Atom,
    pub vmods: [Atom; XkbNumVirtualMods],
    pub indicators: [Atom; XkbNumIndicators],
    pub groups: [Atom; XkbNumKbdGroups],
    pub keys: *mut c_void,
    pub key_aliases: *mut c_void,
    pub radio_groups: *mut Atom,
    pub phys_symbols: Atom,
    pub num_keys: c_uchar,
    pub num_key_aliases: c_uchar,
    pub num_rg: c_ushort,
}

/// Translation of `XkbDescRec`.
#[repr(C)]
pub struct XkbDescRec {
    pub dpy: *mut Display,
    pub flags: c_ushort,
    pub device_spec: c_ushort,
    pub min_key_code: KeyCode,
    pub max_key_code: KeyCode,
    pub ctrls: *mut c_void,
    pub server: *mut XkbServerMapRec,
    pub map: *mut XkbClientMapRec,
    pub indicators: *mut c_void,
    pub names: *mut XkbNamesRec,
    pub compat: *mut c_void,
    pub geom: *mut c_void,
}
pub type XkbDescPtr = *mut XkbDescRec;

/// Translation of `XkbNumGroups()`.
pub const fn XkbNumGroups(g: c_uchar) -> c_uchar {
    g & 0x0f
}
/// Translation of `XkbOutOfRangeGroupAction()`.
pub const fn XkbOutOfRangeGroupAction(g: c_uchar) -> c_uchar {
    g & 0xc0
}
/// Translation of `XkbOutOfRangeGroupNumber()`.
pub const fn XkbOutOfRangeGroupNumber(g: c_uchar) -> c_uchar {
    (g & 0x30) >> 4
}

/// Translation of `XkbKeyGroupInfo()`.
///
/// # Safety
///
/// `d` must be a valid keyboard description with a client map covering
/// keycode `k`.
pub unsafe fn XkbKeyGroupInfo(d: XkbDescPtr, k: c_uint) -> c_uchar {
    // SAFETY: the caller's contract.
    unsafe { (*(*(*d).map).key_sym_map.add(k as usize)).group_info }
}

/// Translation of `XkbKeyNumGroups()`.
///
/// # Safety
///
/// As for [`XkbKeyGroupInfo`].
pub unsafe fn XkbKeyNumGroups(d: XkbDescPtr, k: c_uint) -> c_uchar {
    // SAFETY: the caller's contract.
    XkbNumGroups(unsafe { XkbKeyGroupInfo(d, k) })
}

/// Translation of `XkbKeyKeyType()`.
///
/// # Safety
///
/// As for [`XkbKeyGroupInfo`], and the key's type index for group `g`
/// must be valid.
pub unsafe fn XkbKeyKeyType(d: XkbDescPtr, k: c_uint, g: c_uint) -> *mut XkbKeyTypeRec {
    // SAFETY: the caller's contract.
    unsafe {
        let map = (*d).map;
        let index = (*(*map).key_sym_map.add(k as usize)).kt_index[(g & 0x3) as usize];
        (*map).types.add(index as usize)
    }
}

/// Translation of `XkbAnyEvent`.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct XkbAnyEvent {
    pub type_: c_int,
    pub serial: c_ulong,
    pub send_event: Bool,
    pub display: *mut Display,
    pub time: Time,
    pub xkb_type: c_int,
    pub device: c_uint,
}

/// Translation of `XkbStateNotifyEvent`.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct XkbStateNotifyEvent {
    pub type_: c_int,
    pub serial: c_ulong,
    pub send_event: Bool,
    pub display: *mut Display,
    pub time: Time,
    pub xkb_type: c_int,
    pub device: c_int,
    pub changed: c_uint,
    pub group: c_int,
    pub base_group: c_int,
    pub latched_group: c_int,
    pub locked_group: c_int,
    pub mods: c_uint,
    pub base_mods: c_uint,
    pub latched_mods: c_uint,
    pub locked_mods: c_uint,
    pub compat_state: c_int,
    pub grab_mods: c_uchar,
    pub compat_grab_mods: c_uchar,
    pub lookup_mods: c_uchar,
    pub compat_lookup_mods: c_uchar,
    pub ptr_buttons: c_int,
    pub keycode: KeyCode,
    pub event_type: c_char,
    pub req_major: c_char,
    pub req_minor: c_char,
}

/// Translation of `XkbMapNotifyEvent`.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct XkbMapNotifyEvent {
    pub type_: c_int,
    pub serial: c_ulong,
    pub send_event: Bool,
    pub display: *mut Display,
    pub time: Time,
    pub xkb_type: c_int,
    pub device: c_int,
    pub changed: c_uint,
    pub flags: c_uint,
    pub first_type: c_int,
    pub num_types: c_int,
    pub min_key_code: KeyCode,
    pub max_key_code: KeyCode,
    pub first_key_sym: KeyCode,
    pub first_key_act: KeyCode,
    pub first_key_behavior: KeyCode,
    pub first_key_explicit: KeyCode,
    pub first_modmap_key: KeyCode,
    pub first_vmodmap_key: KeyCode,
    pub num_key_syms: c_int,
    pub num_key_acts: c_int,
    pub num_key_behaviors: c_int,
    pub num_key_explicit: c_int,
    pub num_modmap_keys: c_int,
    pub num_vmodmap_keys: c_int,
    pub vmods: c_uint,
}

// ---------------------------------------------------------------------------
// MIT-SHM (<X11/extensions/XShm.h>)
// ---------------------------------------------------------------------------

pub type ShmSeg = c_ulong;

/// Translation of `XShmSegmentInfo`.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct XShmSegmentInfo {
    pub shmseg: ShmSeg,
    pub shmid: c_int,
    pub shmaddr: *mut c_char,
    pub readOnly: Bool,
}

impl Default for XShmSegmentInfo {
    fn default() -> Self {
        XShmSegmentInfo {
            shmseg: 0,
            shmid: 0,
            shmaddr: std::ptr::null_mut(),
            readOnly: 0,
        }
    }
}

// ---------------------------------------------------------------------------
// SHAPE (<X11/extensions/shape.h>)
// ---------------------------------------------------------------------------

pub const ShapeSet: c_int = 0;
pub const ShapeUnion: c_int = 1;
pub const ShapeIntersect: c_int = 2;
pub const ShapeSubtract: c_int = 3;
pub const ShapeInvert: c_int = 4;
pub const ShapeBounding: c_int = 0;
pub const ShapeClip: c_int = 1;
pub const ShapeInput: c_int = 2;

// ---------------------------------------------------------------------------
// SYNC (<X11/extensions/sync.h>)
// ---------------------------------------------------------------------------

pub type XSyncCounter = XID;

/// Translation of `XSyncValue`.
#[repr(C)]
#[derive(Clone, Copy, Default, PartialEq, Eq, Debug)]
pub struct XSyncValue {
    pub hi: c_int,
    pub lo: c_uint,
}

// ---------------------------------------------------------------------------
// RandR (<X11/extensions/Xrandr.h>, <X11/extensions/randr.h>,
// <X11/extensions/Xrender.h>)
// ---------------------------------------------------------------------------

pub type RROutput = XID;
pub type RRCrtc = XID;
pub type RRMode = XID;
pub type XRRModeFlags = c_ulong;
pub type Rotation = c_ushort;
pub type SizeID = c_ushort;
pub type SubpixelOrder = c_ushort;
pub type Connection = c_ushort;
pub type XFixed = c_int;

pub const RR_Rotate_0: Rotation = 1;
pub const RR_Rotate_90: Rotation = 2;
pub const RR_Rotate_180: Rotation = 4;
pub const RR_Rotate_270: Rotation = 8;

pub const RR_Connected: Connection = 0;
pub const RR_Disconnected: Connection = 1;
pub const RR_UnknownConnection: Connection = 2;

pub const RR_HSyncPositive: c_ulong = 0x00000001;
pub const RR_Interlace: c_ulong = 0x00000010;
pub const RR_DoubleScan: c_ulong = 0x00000020;

pub const RRNotify: c_int = 1;
pub const RRNotify_CrtcChange: c_int = 0;
pub const RRNotify_OutputChange: c_int = 1;
pub const RRNotify_OutputProperty: c_int = 2;

pub const RRScreenChangeNotifyMask: c_int = 1 << 0;
pub const RRCrtcChangeNotifyMask: c_int = 1 << 1;
pub const RROutputChangeNotifyMask: c_int = 1 << 2;
pub const RROutputPropertyNotifyMask: c_int = 1 << 3;

/// Translation of `XRRScreenSize`.
#[repr(C)]
pub struct XRRScreenSize {
    pub width: c_int,
    pub height: c_int,
    pub mwidth: c_int,
    pub mheight: c_int,
}

/// `XRRScreenConfiguration`, only ever used behind a pointer.
#[repr(C)]
pub struct XRRScreenConfiguration {
    _private: [u8; 0],
}

/// Translation of `XRRModeInfo`.
#[repr(C)]
pub struct XRRModeInfo {
    pub id: RRMode,
    pub width: c_uint,
    pub height: c_uint,
    pub dotClock: c_ulong,
    pub hSyncStart: c_uint,
    pub hSyncEnd: c_uint,
    pub hTotal: c_uint,
    pub hSkew: c_uint,
    pub vSyncStart: c_uint,
    pub vSyncEnd: c_uint,
    pub vTotal: c_uint,
    pub name: *mut c_char,
    pub nameLength: c_uint,
    pub modeFlags: XRRModeFlags,
}

/// Translation of `XRRScreenResources`.
#[repr(C)]
pub struct XRRScreenResources {
    pub timestamp: Time,
    pub configTimestamp: Time,
    pub ncrtc: c_int,
    pub crtcs: *mut RRCrtc,
    pub noutput: c_int,
    pub outputs: *mut RROutput,
    pub nmode: c_int,
    pub modes: *mut XRRModeInfo,
}

/// Translation of `XRROutputInfo`.
#[repr(C)]
pub struct XRROutputInfo {
    pub timestamp: Time,
    pub crtc: RRCrtc,
    pub name: *mut c_char,
    pub nameLen: c_int,
    pub mm_width: c_ulong,
    pub mm_height: c_ulong,
    pub connection: Connection,
    pub subpixel_order: SubpixelOrder,
    pub ncrtc: c_int,
    pub crtcs: *mut RRCrtc,
    pub nclone: c_int,
    pub clones: *mut RROutput,
    pub nmode: c_int,
    pub npreferred: c_int,
    pub modes: *mut RRMode,
}

/// Translation of `XRRCrtcInfo`.
#[repr(C)]
pub struct XRRCrtcInfo {
    pub timestamp: Time,
    pub x: c_int,
    pub y: c_int,
    pub width: c_uint,
    pub height: c_uint,
    pub mode: RRMode,
    pub rotation: Rotation,
    pub noutput: c_int,
    pub outputs: *mut RROutput,
    pub rotations: Rotation,
    pub npossible: c_int,
    pub possible: *mut RROutput,
}

/// Translation of `XRRPropertyInfo`.
#[repr(C)]
pub struct XRRPropertyInfo {
    pub pending: Bool,
    pub range: Bool,
    pub immutable: Bool,
    pub num_values: c_int,
    pub values: *mut c_long,
}

/// Translation of `XTransform`.
#[repr(C)]
pub struct XTransform {
    pub matrix: [[XFixed; 3]; 3],
}

/// Translation of `XRRCrtcTransformAttributes`.
#[repr(C)]
pub struct XRRCrtcTransformAttributes {
    pub pendingTransform: XTransform,
    pub pendingFilter: *mut c_char,
    pub pendingNparams: c_int,
    pub pendingParams: *mut XFixed,
    pub currentTransform: XTransform,
    pub currentFilter: *mut c_char,
    pub currentNparams: c_int,
    pub currentParams: *mut XFixed,
}

/// Translation of `XRRNotifyEvent`.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct XRRNotifyEvent {
    pub type_: c_int,
    pub serial: c_ulong,
    pub send_event: Bool,
    pub display: *mut Display,
    pub window: Window,
    pub subtype: c_int,
}

/// Translation of `XRROutputChangeNotifyEvent`.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct XRROutputChangeNotifyEvent {
    pub type_: c_int,
    pub serial: c_ulong,
    pub send_event: Bool,
    pub display: *mut Display,
    pub window: Window,
    pub subtype: c_int,
    pub output: RROutput,
    pub crtc: RRCrtc,
    pub mode: RRMode,
    pub rotation: Rotation,
    pub connection: Connection,
    pub subpixel_order: SubpixelOrder,
}

// ---------------------------------------------------------------------------
// XFIXES (<X11/extensions/Xfixes.h>, <X11/extensions/xfixeswire.h>)
// ---------------------------------------------------------------------------

pub type PointerBarrier = XID;
pub type BarrierEventID = c_uint;

pub const XFixesSelectionNotify: c_int = 0;
pub const XFixesSetSelectionOwnerNotifyMask: c_ulong = 1 << 0;
pub const XFixesSelectionWindowDestroyNotifyMask: c_ulong = 1 << 1;
pub const XFixesSelectionClientCloseNotifyMask: c_ulong = 1 << 2;
pub const XFixesSetSelectionOwnerNotify: c_int = 0;

pub const BarrierPositiveX: c_int = 1 << 0;
pub const BarrierPositiveY: c_int = 1 << 1;
pub const BarrierNegativeX: c_int = 1 << 2;
pub const BarrierNegativeY: c_int = 1 << 3;

/// Translation of `XFixesSelectionNotifyEvent`.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct XFixesSelectionNotifyEvent {
    pub type_: c_int,
    pub serial: c_ulong,
    pub send_event: Bool,
    pub display: *mut Display,
    pub window: Window,
    pub subtype: c_int,
    pub owner: Window,
    pub selection: Atom,
    pub timestamp: Time,
    pub selection_timestamp: Time,
}

// ---------------------------------------------------------------------------
// XInput2 (<X11/extensions/XInput2.h>, <X11/extensions/XI2.h>)
// ---------------------------------------------------------------------------

pub const XI_2_Major: c_int = 2;
pub const XI_2_Minor: c_int = 4;

pub const XIAllDevices: c_int = 0;
pub const XIAllMasterDevices: c_int = 1;

// Device types
pub const XIMasterPointer: c_int = 1;
pub const XIMasterKeyboard: c_int = 2;
pub const XISlavePointer: c_int = 3;
pub const XISlaveKeyboard: c_int = 4;
pub const XIFloatingSlave: c_int = 5;

// Hierarchy flags
pub const XIMasterAdded: c_int = 1 << 0;
pub const XIMasterRemoved: c_int = 1 << 1;
pub const XISlaveAdded: c_int = 1 << 2;
pub const XISlaveRemoved: c_int = 1 << 3;
pub const XISlaveAttached: c_int = 1 << 4;
pub const XISlaveDetached: c_int = 1 << 5;
pub const XIDeviceEnabled: c_int = 1 << 6;
pub const XIDeviceDisabled: c_int = 1 << 7;

// Device classes
pub const XIKeyClass: c_int = 0;
pub const XIButtonClass: c_int = 1;
pub const XIValuatorClass: c_int = 2;
pub const XIScrollClass: c_int = 3;
pub const XITouchClass: c_int = 8;
pub const XIGestureClass: c_int = 9;

// Scroll types
pub const XIScrollTypeVertical: c_int = 1;
pub const XIScrollTypeHorizontal: c_int = 2;

// Valuator modes
pub const XIModeRelative: c_int = 0;
pub const XIModeAbsolute: c_int = 1;

// Touch modes
pub const XIDirectTouch: c_int = 1;
pub const XIDependentTouch: c_int = 2;

// Property event flags
pub const XIPropertyDeleted: c_int = 0;
pub const XIPropertyCreated: c_int = 1;
pub const XIPropertyModified: c_int = 2;

// Device event flags (common)
pub const XIPointerEmulated: c_int = 1 << 16;
// Device event flags (key events only)
pub const XIKeyRepeat: c_int = 1 << 16;

pub const XIAnyModifier: c_int = 1 << 31;

// Event types
pub const XI_DeviceChanged: c_int = 1;
pub const XI_KeyPress: c_int = 2;
pub const XI_KeyRelease: c_int = 3;
pub const XI_ButtonPress: c_int = 4;
pub const XI_ButtonRelease: c_int = 5;
pub const XI_Motion: c_int = 6;
pub const XI_Enter: c_int = 7;
pub const XI_Leave: c_int = 8;
pub const XI_FocusIn: c_int = 9;
pub const XI_FocusOut: c_int = 10;
pub const XI_HierarchyChanged: c_int = 11;
pub const XI_PropertyEvent: c_int = 12;
pub const XI_RawKeyPress: c_int = 13;
pub const XI_RawKeyRelease: c_int = 14;
pub const XI_RawButtonPress: c_int = 15;
pub const XI_RawButtonRelease: c_int = 16;
pub const XI_RawMotion: c_int = 17;
pub const XI_TouchBegin: c_int = 18;
pub const XI_TouchUpdate: c_int = 19;
pub const XI_TouchEnd: c_int = 20;
pub const XI_TouchOwnership: c_int = 21;
pub const XI_RawTouchBegin: c_int = 22;
pub const XI_RawTouchUpdate: c_int = 23;
pub const XI_RawTouchEnd: c_int = 24;
pub const XI_BarrierHit: c_int = 25;
pub const XI_BarrierLeave: c_int = 26;
pub const XI_GesturePinchBegin: c_int = 27;
pub const XI_GesturePinchUpdate: c_int = 28;
pub const XI_GesturePinchEnd: c_int = 29;
pub const XI_GestureSwipeBegin: c_int = 30;
pub const XI_GestureSwipeUpdate: c_int = 31;
pub const XI_GestureSwipeEnd: c_int = 32;
pub const XI_LASTEVENT: c_int = XI_GestureSwipeEnd;

/// Translation of `XIMaskLen()`.
pub const fn XIMaskLen(event: c_int) -> usize {
    ((event >> 3) + 1) as usize
}

/// Translation of `XISetMask()`.
pub fn XISetMask(mask: &mut [c_uchar], event: c_int) {
    mask[(event >> 3) as usize] |= 1 << (event & 7);
}

/// Translation of `XIMaskIsSet()`.
pub fn XIMaskIsSet(mask: &[c_uchar], event: c_int) -> bool {
    mask.get((event >> 3) as usize)
        .is_some_and(|m| m & (1 << (event & 7)) != 0)
}

/// Translation of `XIEventMask`.
#[repr(C)]
pub struct XIEventMask {
    pub deviceid: c_int,
    pub mask_len: c_int,
    pub mask: *mut c_uchar,
}

/// Translation of `XIAnyClassInfo`.
#[repr(C)]
pub struct XIAnyClassInfo {
    pub type_: c_int,
    pub sourceid: c_int,
}

/// Translation of `XIButtonState`.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct XIButtonState {
    pub mask_len: c_int,
    pub mask: *mut c_uchar,
}

/// Translation of `XIButtonClassInfo`.
#[repr(C)]
pub struct XIButtonClassInfo {
    pub type_: c_int,
    pub sourceid: c_int,
    pub num_buttons: c_int,
    pub labels: *mut Atom,
    pub state: XIButtonState,
}

/// Translation of `XIValuatorClassInfo`.
#[repr(C)]
pub struct XIValuatorClassInfo {
    pub type_: c_int,
    pub sourceid: c_int,
    pub number: c_int,
    pub label: Atom,
    pub min: c_double,
    pub max: c_double,
    pub value: c_double,
    pub resolution: c_int,
    pub mode: c_int,
}

/// Translation of `XIScrollClassInfo`.
#[repr(C)]
pub struct XIScrollClassInfo {
    pub type_: c_int,
    pub sourceid: c_int,
    pub number: c_int,
    pub scroll_type: c_int,
    pub increment: c_double,
    pub flags: c_int,
}

/// Translation of `XITouchClassInfo`.
#[repr(C)]
pub struct XITouchClassInfo {
    pub type_: c_int,
    pub sourceid: c_int,
    pub mode: c_int,
    pub num_touches: c_int,
}

/// Translation of `XIDeviceInfo`.
#[repr(C)]
pub struct XIDeviceInfo {
    pub deviceid: c_int,
    pub name: *mut c_char,
    pub use_: c_int,
    pub attachment: c_int,
    pub enabled: Bool,
    pub num_classes: c_int,
    pub classes: *mut *mut XIAnyClassInfo,
}

/// Translation of `XIGrabModifiers`.
#[repr(C)]
pub struct XIGrabModifiers {
    pub modifiers: c_int,
    pub status: c_int,
}

/// Translation of `XIModifierState` (and `XIGroupState`).
#[repr(C)]
#[derive(Clone, Copy)]
pub struct XIModifierState {
    pub base: c_int,
    pub latched: c_int,
    pub locked: c_int,
    pub effective: c_int,
}
pub type XIGroupState = XIModifierState;

/// Translation of `XIValuatorState`.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct XIValuatorState {
    pub mask_len: c_int,
    pub mask: *mut c_uchar,
    pub values: *mut c_double,
}

/// Translation of `XIDeviceEvent`.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct XIDeviceEvent {
    pub type_: c_int,
    pub serial: c_ulong,
    pub send_event: Bool,
    pub display: *mut Display,
    pub extension: c_int,
    pub evtype: c_int,
    pub time: Time,
    pub deviceid: c_int,
    pub sourceid: c_int,
    pub detail: c_int,
    pub root: Window,
    pub event: Window,
    pub child: Window,
    pub root_x: c_double,
    pub root_y: c_double,
    pub event_x: c_double,
    pub event_y: c_double,
    pub flags: c_int,
    pub buttons: XIButtonState,
    pub valuators: XIValuatorState,
    pub mods: XIModifierState,
    pub group: XIGroupState,
}

/// Translation of `XIRawEvent`.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct XIRawEvent {
    pub type_: c_int,
    pub serial: c_ulong,
    pub send_event: Bool,
    pub display: *mut Display,
    pub extension: c_int,
    pub evtype: c_int,
    pub time: Time,
    pub deviceid: c_int,
    pub sourceid: c_int,
    pub detail: c_int,
    pub flags: c_int,
    pub valuators: XIValuatorState,
    pub raw_values: *mut c_double,
}

/// Translation of `XIEnterEvent` (and `XILeaveEvent`, `XIFocusInEvent`...).
#[repr(C)]
#[derive(Clone, Copy)]
pub struct XIEnterEvent {
    pub type_: c_int,
    pub serial: c_ulong,
    pub send_event: Bool,
    pub display: *mut Display,
    pub extension: c_int,
    pub evtype: c_int,
    pub time: Time,
    pub deviceid: c_int,
    pub sourceid: c_int,
    pub detail: c_int,
    pub root: Window,
    pub event: Window,
    pub child: Window,
    pub root_x: c_double,
    pub root_y: c_double,
    pub event_x: c_double,
    pub event_y: c_double,
    pub mode: c_int,
    pub focus: Bool,
    pub same_screen: Bool,
    pub buttons: XIButtonState,
    pub mods: XIModifierState,
    pub group: XIGroupState,
}

/// Translation of `XIHierarchyInfo`.
#[repr(C)]
pub struct XIHierarchyInfo {
    pub deviceid: c_int,
    pub attachment: c_int,
    pub use_: c_int,
    pub enabled: Bool,
    pub flags: c_int,
}

/// Translation of `XIHierarchyEvent`.
#[repr(C)]
pub struct XIHierarchyEvent {
    pub type_: c_int,
    pub serial: c_ulong,
    pub send_event: Bool,
    pub display: *mut Display,
    pub extension: c_int,
    pub evtype: c_int,
    pub time: Time,
    pub flags: c_int,
    pub num_info: c_int,
    pub info: *mut XIHierarchyInfo,
}

/// Translation of `XIDeviceChangedEvent`.
#[repr(C)]
pub struct XIDeviceChangedEvent {
    pub type_: c_int,
    pub serial: c_ulong,
    pub send_event: Bool,
    pub display: *mut Display,
    pub extension: c_int,
    pub evtype: c_int,
    pub time: Time,
    pub deviceid: c_int,
    pub sourceid: c_int,
    pub reason: c_int,
    pub num_classes: c_int,
    pub classes: *mut *mut XIAnyClassInfo,
}

/// Translation of `XIPropertyEvent`.
#[repr(C)]
pub struct XIPropertyEvent {
    pub type_: c_int,
    pub serial: c_ulong,
    pub send_event: Bool,
    pub display: *mut Display,
    pub extension: c_int,
    pub evtype: c_int,
    pub time: Time,
    pub deviceid: c_int,
    pub property: Atom,
    pub what: c_int,
}

/// Translation of `XIGesturePinchEvent`.
#[repr(C)]
pub struct XIGesturePinchEvent {
    pub type_: c_int,
    pub serial: c_ulong,
    pub send_event: Bool,
    pub display: *mut Display,
    pub extension: c_int,
    pub evtype: c_int,
    pub time: Time,
    pub deviceid: c_int,
    pub sourceid: c_int,
    pub detail: c_int,
    pub root: Window,
    pub event: Window,
    pub child: Window,
    pub root_x: c_double,
    pub root_y: c_double,
    pub event_x: c_double,
    pub event_y: c_double,
    pub delta_x: c_double,
    pub delta_y: c_double,
    pub delta_unaccel_x: c_double,
    pub delta_unaccel_y: c_double,
    pub scale: c_double,
    pub delta_angle: c_double,
    pub flags: c_int,
    pub mods: XIModifierState,
    pub group: XIGroupState,
}

// ---------------------------------------------------------------------------
// Xcursor (<X11/Xcursor/Xcursor.h>)
// ---------------------------------------------------------------------------

pub type XcursorBool = c_int;
pub type XcursorUInt = c_uint;
pub type XcursorDim = XcursorUInt;
pub type XcursorPixel = XcursorUInt;

/// Translation of `XcursorImage`.
#[repr(C)]
pub struct XcursorImage {
    pub version: XcursorUInt,
    pub size: XcursorDim,
    pub width: XcursorDim,
    pub height: XcursorDim,
    pub xhot: XcursorDim,
    pub yhot: XcursorDim,
    pub delay: XcursorUInt,
    pub pixels: *mut XcursorPixel,
}

/// Translation of `XcursorImages`.
#[repr(C)]
pub struct XcursorImages {
    pub nimage: c_int,
    pub images: *mut *mut XcursorImage,
    pub name: *mut c_char,
}

// ---------------------------------------------------------------------------
// The Xlib header macros
// ---------------------------------------------------------------------------

/// The public view of a display (for the header macros).
///
/// # Safety
///
/// `dpy` must be an open display.
unsafe fn priv_display<'a>(dpy: *mut Display) -> &'a XPrivDisplay {
    // SAFETY: an open Display starts with the public `_XPrivDisplay` layout.
    unsafe { &*(dpy as *const XPrivDisplay) }
}

/// Translation of `ScreenOfDisplay()`.
///
/// # Safety
///
/// `dpy` must be an open display and `scr` one of its screens.
pub unsafe fn ScreenOfDisplay(dpy: *mut Display, scr: c_int) -> *mut Screen {
    // SAFETY: the caller's contract; `screens` has `nscreens` entries.
    unsafe { priv_display(dpy).screens.add(scr as usize) }
}

/// Translation of `DefaultScreen()`.
///
/// # Safety
///
/// `dpy` must be an open display.
pub unsafe fn DefaultScreen(dpy: *mut Display) -> c_int {
    // SAFETY: the caller's contract.
    unsafe { priv_display(dpy).default_screen }
}

/// Translation of `ScreenCount()`.
///
/// # Safety
///
/// `dpy` must be an open display.
pub unsafe fn ScreenCount(dpy: *mut Display) -> c_int {
    // SAFETY: the caller's contract.
    unsafe { priv_display(dpy).nscreens }
}

/// Translation of `ConnectionNumber()`.
///
/// # Safety
///
/// `dpy` must be an open display.
pub unsafe fn ConnectionNumber(dpy: *mut Display) -> c_int {
    // SAFETY: the caller's contract.
    unsafe { priv_display(dpy).fd }
}

/// Translation of `BitmapBitOrder()`.
///
/// # Safety
///
/// `dpy` must be an open display.
pub unsafe fn BitmapBitOrder(dpy: *mut Display) -> c_int {
    // SAFETY: the caller's contract.
    unsafe { priv_display(dpy).bitmap_bit_order }
}

/// The `flags` of `struct _XDisplay` (Xlibint.h).
///
/// # Safety
///
/// `dpy` must be an open display.
pub unsafe fn DisplayFlags(dpy: *mut Display) -> c_ulong {
    // SAFETY: the caller's contract.
    unsafe { priv_display(dpy).flags }
}

/// Translation of `RootWindow()`.
///
/// # Safety
///
/// `dpy` must be an open display and `scr` one of its screens.
pub unsafe fn RootWindow(dpy: *mut Display, scr: c_int) -> Window {
    // SAFETY: the caller's contract.
    unsafe { (*ScreenOfDisplay(dpy, scr)).root }
}

/// Translation of `DefaultRootWindow()`.
///
/// # Safety
///
/// `dpy` must be an open display.
pub unsafe fn DefaultRootWindow(dpy: *mut Display) -> Window {
    // SAFETY: the caller's contract.
    unsafe { RootWindow(dpy, DefaultScreen(dpy)) }
}

/// Translation of `DefaultVisual()`.
///
/// # Safety
///
/// `dpy` must be an open display and `scr` one of its screens.
pub unsafe fn DefaultVisual(dpy: *mut Display, scr: c_int) -> *mut Visual {
    // SAFETY: the caller's contract.
    unsafe { (*ScreenOfDisplay(dpy, scr)).root_visual }
}

/// Translation of `DefaultDepth()`.
///
/// # Safety
///
/// `dpy` must be an open display and `scr` one of its screens.
pub unsafe fn DefaultDepth(dpy: *mut Display, scr: c_int) -> c_int {
    // SAFETY: the caller's contract.
    unsafe { (*ScreenOfDisplay(dpy, scr)).root_depth }
}

/// Translation of `DefaultColormap()`.
///
/// # Safety
///
/// `dpy` must be an open display and `scr` one of its screens.
pub unsafe fn DefaultColormap(dpy: *mut Display, scr: c_int) -> Colormap {
    // SAFETY: the caller's contract.
    unsafe { (*ScreenOfDisplay(dpy, scr)).cmap }
}

/// Translation of `BlackPixel()`.
///
/// # Safety
///
/// `dpy` must be an open display and `scr` one of its screens.
pub unsafe fn BlackPixel(dpy: *mut Display, scr: c_int) -> c_ulong {
    // SAFETY: the caller's contract.
    unsafe { (*ScreenOfDisplay(dpy, scr)).black_pixel }
}

/// Translation of `WhitePixel()`.
///
/// # Safety
///
/// `dpy` must be an open display and `scr` one of its screens.
pub unsafe fn WhitePixel(dpy: *mut Display, scr: c_int) -> c_ulong {
    // SAFETY: the caller's contract.
    unsafe { (*ScreenOfDisplay(dpy, scr)).white_pixel }
}

/// Translation of `DisplayWidth()`.
///
/// # Safety
///
/// `dpy` must be an open display and `scr` one of its screens.
pub unsafe fn DisplayWidth(dpy: *mut Display, scr: c_int) -> c_int {
    // SAFETY: the caller's contract.
    unsafe { (*ScreenOfDisplay(dpy, scr)).width }
}

/// Translation of `DisplayHeight()`.
///
/// # Safety
///
/// `dpy` must be an open display and `scr` one of its screens.
pub unsafe fn DisplayHeight(dpy: *mut Display, scr: c_int) -> c_int {
    // SAFETY: the caller's contract.
    unsafe { (*ScreenOfDisplay(dpy, scr)).height }
}

/// Translation of `DisplayWidthMM()`.
///
/// # Safety
///
/// `dpy` must be an open display and `scr` one of its screens.
pub unsafe fn DisplayWidthMM(dpy: *mut Display, scr: c_int) -> c_int {
    // SAFETY: the caller's contract.
    unsafe { (*ScreenOfDisplay(dpy, scr)).mwidth }
}

/// Translation of `DisplayHeightMM()`.
///
/// # Safety
///
/// `dpy` must be an open display and `scr` one of its screens.
pub unsafe fn DisplayHeightMM(dpy: *mut Display, scr: c_int) -> c_int {
    // SAFETY: the caller's contract.
    unsafe { (*ScreenOfDisplay(dpy, scr)).mheight }
}

/// Translation of `RootWindowOfScreen()`.
///
/// # Safety
///
/// `s` must be a screen of an open display.
pub unsafe fn RootWindowOfScreen(s: *mut Screen) -> Window {
    // SAFETY: the caller's contract.
    unsafe { (*s).root }
}

/// Translation of `WidthOfScreen()`.
///
/// # Safety
///
/// `s` must be a screen of an open display.
pub unsafe fn WidthOfScreen(s: *mut Screen) -> c_int {
    // SAFETY: the caller's contract.
    unsafe { (*s).width }
}

/// Translation of `HeightOfScreen()`.
///
/// # Safety
///
/// `s` must be a screen of an open display.
pub unsafe fn HeightOfScreen(s: *mut Screen) -> c_int {
    // SAFETY: the caller's contract.
    unsafe { (*s).height }
}

/// Translation of `XDestroyImage()` (a macro calling the image's own
/// `destroy_image`).
///
/// # Safety
///
/// `ximage` must be an image created by Xlib, not used afterwards.
pub unsafe fn XDestroyImage(ximage: *mut XImage) -> c_int {
    // SAFETY: the caller's contract; Xlib always fills in destroy_image.
    unsafe {
        match (*ximage).f.destroy_image {
            Some(destroy) => destroy(ximage),
            Option::None => 0,
        }
    }
}

/// Translation of `XGetPixel()`.
///
/// # Safety
///
/// `ximage` must be an image created by Xlib and (x, y) inside it.
pub unsafe fn XGetPixel(ximage: *mut XImage, x: c_int, y: c_int) -> c_ulong {
    // SAFETY: the caller's contract; Xlib always fills in get_pixel.
    unsafe {
        match (*ximage).f.get_pixel {
            Some(get) => get(ximage, x, y),
            Option::None => 0,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::mem::{offset_of, size_of};

    /// The sizes and offsets of the C structures, from a C program built
    /// against the system's X11 headers on x86-64 Linux.
    #[cfg(all(target_os = "linux", target_pointer_width = "64"))]
    #[test]
    fn layouts_match_c() {
        assert_eq!(size_of::<XEvent>(), 192);
        assert_eq!(size_of::<XAnyEvent>(), 40);
        assert_eq!(size_of::<XKeyEvent>(), 96);
        assert_eq!(size_of::<XButtonEvent>(), 96);
        assert_eq!(size_of::<XMotionEvent>(), 96);
        assert_eq!(size_of::<XCrossingEvent>(), 104);
        assert_eq!(size_of::<XFocusChangeEvent>(), 48);
        assert_eq!(size_of::<XExposeEvent>(), 64);
        assert_eq!(size_of::<XConfigureEvent>(), 88);
        assert_eq!(size_of::<XPropertyEvent>(), 64);
        assert_eq!(size_of::<XSelectionRequestEvent>(), 80);
        assert_eq!(size_of::<XSelectionEvent>(), 72);
        assert_eq!(size_of::<XClientMessageEvent>(), 96);
        assert_eq!(size_of::<XMappingEvent>(), 56);
        assert_eq!(size_of::<XErrorEvent>(), 40);
        assert_eq!(size_of::<XKeymapEvent>(), 72);
        assert_eq!(size_of::<XGenericEventCookie>(), 56);
        assert_eq!(size_of::<XWindowAttributes>(), 136);
        assert_eq!(size_of::<XSetWindowAttributes>(), 112);
        assert_eq!(size_of::<XVisualInfo>(), 64);
        assert_eq!(size_of::<Visual>(), 56);
        assert_eq!(size_of::<Screen>(), 128);
        assert_eq!(size_of::<XImage>(), 136);
        assert_eq!(size_of::<XSizeHints>(), 80);
        assert_eq!(size_of::<XWMHints>(), 56);
        assert_eq!(size_of::<XTextProperty>(), 32);
        assert_eq!(size_of::<XGCValues>(), 128);
        assert_eq!(size_of::<XColor>(), 16);
        assert_eq!(size_of::<XFontStruct>(), 96);
        assert_eq!(size_of::<XIMText>(), 32);
        assert_eq!(size_of::<XIMPreeditDrawCallbackStruct>(), 24);
        assert_eq!(size_of::<XkbStateRec>(), 18);
        assert_eq!(size_of::<XkbKeyTypeRec>(), 40);
        assert_eq!(size_of::<XkbKTMapEntryRec>(), 12);
        assert_eq!(size_of::<XkbSymMapRec>(), 8);
        assert_eq!(size_of::<XkbClientMapRec>(), 48);
        assert_eq!(size_of::<XkbServerMapRec>(), 64);
        assert_eq!(size_of::<XkbNamesRec>(), 496);
        assert_eq!(size_of::<XkbDescRec>(), 72);
        assert_eq!(size_of::<XkbAnyEvent>(), 48);
        assert_eq!(size_of::<XkbStateNotifyEvent>(), 104);
        assert_eq!(size_of::<XkbMapNotifyEvent>(), 104);
        assert_eq!(size_of::<XShmSegmentInfo>(), 32);
        assert_eq!(size_of::<XTransform>(), 36);
        assert_eq!(offset_of!(XPrivDisplay, fd), 16);
        assert_eq!(offset_of!(XPrivDisplay, byte_order), 80);
        assert_eq!(offset_of!(XPrivDisplay, bitmap_bit_order), 92);
        assert_eq!(offset_of!(XPrivDisplay, default_screen), 224);
        assert_eq!(offset_of!(XPrivDisplay, screens), 232);
        assert_eq!(offset_of!(XPrivDisplay, flags), 248);
        assert_eq!(offset_of!(XkbDescRec, server), 24);
        assert_eq!(offset_of!(XkbDescRec, names), 48);
        assert_eq!(offset_of!(XkbServerMapRec, vmods), 40);
        assert_eq!(offset_of!(XkbNamesRec, vmods), 40);
        assert_eq!(offset_of!(XClientMessageEvent, data), 56);
        assert_eq!(offset_of!(XkbStateNotifyEvent, changed), 48);
        assert_eq!(offset_of!(XkbStateNotifyEvent, base_mods), 72);
        assert_eq!(offset_of!(XPrivDisplay, min_keycode), 256);
        assert_eq!(size_of::<XIMPreeditCaretCallbackStruct>(), 12);
        assert_eq!(size_of::<XIMCallback>(), 16);
        assert_eq!(size_of::<XrmValue>(), 16);
        assert_eq!(size_of::<XPixmapFormatValues>(), 12);
        assert_eq!(size_of::<XModifierKeymap>(), 16);
        assert_eq!(size_of::<XCharStruct>(), 12);
        assert_eq!(size_of::<XFontSetExtents>(), 16);
        assert_eq!(size_of::<XUnmapEvent>(), 56);
        assert_eq!(size_of::<XMapEvent>(), 56);
        assert_eq!(size_of::<XReparentEvent>(), 72);
        assert_eq!(size_of::<XVisibilityEvent>(), 48);
        assert_eq!(size_of::<XDestroyWindowEvent>(), 48);
        assert_eq!(size_of::<XSelectionClearEvent>(), 56);
        assert_eq!(size_of::<XGenericEvent>(), 40);
    }

    /// The sizes of the extension structures whose headers aren't
    /// installed everywhere (computed from the field lists of
    /// libXrandr's, libXi's and libXcursor's headers for x86-64 Linux).
    #[cfg(all(target_os = "linux", target_pointer_width = "64"))]
    #[test]
    fn extension_layouts() {
        assert_eq!(size_of::<XRRModeInfo>(), 80);
        assert_eq!(size_of::<XRRScreenResources>(), 64);
        assert_eq!(size_of::<XRROutputInfo>(), 96);
        assert_eq!(size_of::<XRRCrtcInfo>(), 64);
        assert_eq!(size_of::<XRRCrtcTransformAttributes>(), 128);
        assert_eq!(size_of::<XRROutputChangeNotifyEvent>(), 80);
        assert_eq!(size_of::<XIDeviceEvent>(), 200);
        assert_eq!(size_of::<XIRawEvent>(), 96);
        assert_eq!(size_of::<XIEnterEvent>(), 184);
        assert_eq!(size_of::<XIDeviceInfo>(), 40);
        assert_eq!(size_of::<XIValuatorClassInfo>(), 56);
        assert_eq!(size_of::<XIScrollClassInfo>(), 32);
        assert_eq!(size_of::<XIButtonClassInfo>(), 40);
        assert_eq!(size_of::<XIHierarchyEvent>(), 64);
        assert_eq!(size_of::<XIGesturePinchEvent>(), 208);
        assert_eq!(size_of::<XFixesSelectionNotifyEvent>(), 80);
        assert_eq!(size_of::<XcursorImage>(), 40);
    }
}
