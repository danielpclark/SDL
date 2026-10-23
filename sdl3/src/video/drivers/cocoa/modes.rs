// Rust translation of src/video/cocoa/SDL_cocoamodes.m and
// SDL_cocoamodes.h from Simple DirectMedia Layer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! Displays and their modes, through CoreGraphics: the online displays,
//! their names, HDR headroom and usable bounds (from their `NSScreen`), the
//! display modes (with the duplicates `SetDisplayMode` needs to try), mode
//! switches with a fade, and the display reconfiguration callback.

use std::ffi::c_void;
use std::sync::Arc;

use super::sys::*;
use super::{video_data, VideoData};
use crate::core::apple::foundation::{
    available, cf_string, kCFAllocatorDefault, kCFBooleanTrue, kCFTypeArrayCallBacks,
    kCFTypeDictionaryValueCallBacks, string_from_cf, CFArrayAppendValue, CFArrayCreateMutable,
    CFArrayGetCount, CFArrayGetValueAtIndex, CFArrayRef, CFDictionaryCreate, CFEqual, CFIndex,
    CfRef,
};
use crate::core::apple::objc::{autoreleasepool, class, msg_send, Id, NSUInteger};
use crate::error::{Error, Result};
use crate::events::{DisplayID, EventType};
use crate::video::core::{with_device, with_display};
use crate::video::display::{
    add_fullscreen_display_mode, add_video_display, del_video_display, finalize_display_mode,
    set_desktop_display_mode, set_display_hdr_properties, update_desktop_bounds,
};
use crate::video::sysvideo::{DisplayMode, HdrOutputProperties, VideoDisplay};
use crate::video::{PixelFormat, Rect};

/// The driver data of a display. Translation of `struct SDL_DisplayData`
/// (plus the `SDL_DisplayModeData` of its modes).
pub(crate) struct DisplayData {
    pub(crate) display: CGDirectDisplayID,
    pub(crate) usable_bounds: Rect,
    /// The `SDL_DisplayModeData` of the desktop mode: the `CGDisplayMode`s
    /// to try for it (`desktop_mode.internal->modes`).
    desktop_modes: CfRef,
    /// The fullscreen modes with their `SDL_DisplayModeData`, as finalized
    /// by the video core.
    modes: Vec<(DisplayMode, CfRef)>,
}

/// Run `f` on a display's driver data.
pub(crate) fn with_display_data<R>(
    display: DisplayID,
    f: impl FnOnce(&mut DisplayData) -> R,
) -> Option<R> {
    with_display(display, |d| {
        d.internal
            .as_mut()
            .and_then(|i| i.downcast_mut::<DisplayData>())
            .map(f)
    })
    .flatten()
}

/// Translation of `CG_SetError()`.
fn cg_set_error(prefix: &str, result: CGError) -> Error {
    let error = match result {
        kCGErrorFailure => "kCGErrorFailure",
        kCGErrorIllegalArgument => "kCGErrorIllegalArgument",
        kCGErrorInvalidConnection => "kCGErrorInvalidConnection",
        kCGErrorInvalidContext => "kCGErrorInvalidContext",
        kCGErrorCannotComplete => "kCGErrorCannotComplete",
        kCGErrorNotImplemented => "kCGErrorNotImplemented",
        kCGErrorRangeCheck => "kCGErrorRangeCheck",
        kCGErrorTypeCheck => "kCGErrorTypeCheck",
        kCGErrorInvalidOperation => "kCGErrorInvalidOperation",
        kCGErrorNoneAvailable => "kCGErrorNoneAvailable",
        _ => "Unknown Error",
    };
    Error::new(format!("{prefix}: {error}"))
}

/// The `CGDirectDisplayID` of an `NSScreen` (its `NSScreenNumber`).
///
/// # Safety
///
/// `screen` must be an `NSScreen`.
pub(crate) unsafe fn screen_display_id(screen: Id) -> CGDirectDisplayID {
    // SAFETY: the caller's contract; the description is a dictionary of
    // NSNumbers.
    unsafe {
        let description: Id = msg_send![screen, deviceDescription];
        let key = crate::core::apple::foundation::ns_string("NSScreenNumber");
        let number: Id = msg_send![description, objectForKey: key.get()];
        msg_send![number, unsignedIntValue]
    }
}

/// Translation of `GetNSScreenForDisplayID()`.
fn get_ns_screen_for_display_id(display_id: CGDirectDisplayID) -> Id {
    // SAFETY: NSScreen's class method; the array holds NSScreens.
    unsafe {
        let screens: Id = msg_send![class!(NSScreen), screens];

        // !!! FIXME: maybe track the NSScreen in SDL_DisplayData?
        for screen in crate::core::apple::foundation::ns_array_items(screens) {
            let this_display = screen_display_id(screen);
            if this_display == display_id {
                return screen;
            }
        }
    }
    std::ptr::null_mut()
}

/// Translation of `Cocoa_FindSDLDisplayByCGDirectDisplayID()`.
pub(crate) fn find_sdl_display_by_cg_direct_display_id(
    displayid: CGDirectDisplayID,
) -> Option<DisplayID> {
    with_device(|v| {
        v.displays
            .iter()
            .find(|d| {
                d.internal
                    .as_ref()
                    .and_then(|i| i.downcast_ref::<DisplayData>())
                    .is_some_and(|data| data.display == displayid)
            })
            .map(|d| d.id)
    })
    .ok()
    .flatten()
}

/// Translation of `GetDisplayModeRefreshRate()`.
fn get_display_mode_refresh_rate(vidmode: CGDisplayModeRef, link: CVDisplayLinkRef) -> f32 {
    // SAFETY: a valid display mode and (if not null) display link.
    unsafe {
        let mut refresh_rate = CGDisplayModeGetRefreshRate(vidmode) as f32;

        // CGDisplayModeGetRefreshRate can return 0 (eg for built-in displays).
        if refresh_rate == 0.0 && !link.is_null() {
            let time = CVDisplayLinkGetNominalOutputVideoRefreshPeriod(link);
            if (time.flags & kCVTimeIsIndefinite) == 0 && time.time_value != 0 {
                refresh_rate = time.time_scale as f32 / time.time_value as f32;
            }
        }

        refresh_rate
    }
}

/// Translation of `HasValidDisplayModeFlags()`.
fn has_valid_display_mode_flags(vidmode: CGDisplayModeRef) -> bool {
    // SAFETY: a valid display mode.
    let ioflags = unsafe { CGDisplayModeGetIOFlags(vidmode) };

    // Filter out modes which have flags that we don't want.
    if ioflags & (kDisplayModeNeverShowFlag | kDisplayModeNotGraphicsQualityFlag) != 0 {
        return false;
    }

    // Filter out modes which don't have flags that we want.
    if (ioflags & kDisplayModeValidFlag) == 0 || (ioflags & kDisplayModeSafeFlag) == 0 {
        return false;
    }

    true
}

/// Translation of `GetDisplayModePixelFormat()`.
fn get_display_mode_pixel_format(vidmode: CGDisplayModeRef) -> PixelFormat {
    // This API is deprecated in 10.11 with no good replacement (as of 10.15).
    // SAFETY: a valid display mode; the copy is released by CfRef.
    let Some(fmt) = (unsafe { CfRef::from_owned(CGDisplayModeCopyPixelEncoding(vidmode)) }) else {
        return PixelFormat::UNKNOWN;
    };
    let compare = |name: &str| {
        let name = cf_string(name);
        // SAFETY: two CFStrings.
        unsafe {
            CFStringCompare(fmt.get(), name.get(), kCFCompareCaseInsensitive) == kCFCompareEqualTo
        }
    };

    if compare(IO32BitDirectPixels) {
        PixelFormat::ARGB8888
    } else if compare(IO16BitDirectPixels) {
        PixelFormat::ARGB1555
    } else if compare(kIO30BitDirectPixels) {
        PixelFormat::ARGB2101010
    } else {
        // ignore 8-bit and such for now.
        PixelFormat::UNKNOWN
    }
}

/// A display mode, with its `SDL_DisplayModeData` (the `CGDisplayMode`s to
/// try when switching to it). Translation of `GetDisplayMode()`.
fn get_display_mode(
    vidmode: CGDisplayModeRef,
    vidmode_current: bool,
    modelist: CFArrayRef,
    link: CVDisplayLinkRef,
) -> Option<(DisplayMode, CfRef)> {
    // SAFETY: a valid display mode, mode list and display link (if not
    // null); the mode array is owned by the CfRef.
    unsafe {
        let usable_for_gui = CGDisplayModeIsUsableForDesktopGUI(vidmode);
        let width = CGDisplayModeGetWidth(vidmode);
        let height = CGDisplayModeGetHeight(vidmode);
        let ioflags = CGDisplayModeGetIOFlags(vidmode);
        let refreshrate = get_display_mode_refresh_rate(vidmode, link);
        let format = get_display_mode_pixel_format(vidmode);
        let interlaced = (ioflags & kDisplayModeInterlacedFlag) != 0;

        if format == PixelFormat::UNKNOWN {
            return None;
        }

        /* Don't fail the current mode based on flags because this could prevent Cocoa_InitModes from
         * succeeding if the current mode lacks certain flags (esp kDisplayModeSafeFlag). */
        if !vidmode_current && !has_valid_display_mode_flags(vidmode) {
            return None;
        }

        let modes =
            CfRef::from_owned(
                CFArrayCreateMutable(std::ptr::null(), 0, &kCFTypeArrayCallBacks) as *const c_void,
            )?;
        CFArrayAppendValue(modes.get() as *mut c_void, vidmode);

        /* If a list of possible display modes is passed in, use it to filter out
         * modes that have duplicate sizes. We don't just rely on SDL's higher level
         * duplicate filtering because this code can choose what properties are
         * preferred, and it can add CGDisplayModes to the DisplayModeData's list of
         * modes to try (see comment below for why that's necessary). */
        let pixel_w = CGDisplayModeGetPixelWidth(vidmode);
        let pixel_h = CGDisplayModeGetPixelHeight(vidmode);

        if !modelist.is_null() {
            let modescount = CFArrayGetCount(modelist);

            for i in 0..modescount {
                let othermode = CFArrayGetValueAtIndex(modelist, i) as CGDisplayModeRef;
                let otherioflags = CGDisplayModeGetIOFlags(othermode);

                if CFEqual(vidmode, othermode) != 0 {
                    continue;
                }

                if !has_valid_display_mode_flags(othermode) {
                    continue;
                }

                let other_w = CGDisplayModeGetWidth(othermode);
                let other_h = CGDisplayModeGetHeight(othermode);
                let otherpixel_w = CGDisplayModeGetPixelWidth(othermode);
                let otherpixel_h = CGDisplayModeGetPixelHeight(othermode);
                let otherrefresh = get_display_mode_refresh_rate(othermode, link);
                let otherformat = get_display_mode_pixel_format(othermode);
                let other_gui = CGDisplayModeIsUsableForDesktopGUI(othermode);

                /* Ignore this mode if it's interlaced and there's a non-interlaced
                 * mode in the list with the same properties.
                 */
                if interlaced
                    && (otherioflags & kDisplayModeInterlacedFlag) == 0
                    && width == other_w
                    && height == other_h
                    && pixel_w == otherpixel_w
                    && pixel_h == otherpixel_h
                    && refreshrate == otherrefresh
                    && format == otherformat
                    && usable_for_gui == other_gui
                {
                    return None;
                }

                /* Ignore this mode if it's not usable for desktop UI and its
                 * properties are equal to another GUI-capable mode in the list.
                 */
                if width == other_w
                    && height == other_h
                    && pixel_w == otherpixel_w
                    && pixel_h == otherpixel_h
                    && !usable_for_gui
                    && other_gui
                    && refreshrate == otherrefresh
                    && format == otherformat
                {
                    return None;
                }

                /* If multiple modes have the exact same properties, they'll all
                 * go in the list of modes to try when SetDisplayMode is called.
                 * This is needed because kCGDisplayShowDuplicateLowResolutionModes
                 * (which is used to expose highdpi display modes) can make the
                 * list of modes contain duplicates (according to their properties
                 * obtained via public APIs) which don't work with SetDisplayMode.
                 * Those duplicate non-functional modes *do* have different pixel
                 * formats according to their internal data structure viewed with
                 * NSLog, but currently no public API can detect that.
                 * https://bugzilla.libsdl.org/show_bug.cgi?id=4822
                 *
                 * As of macOS 10.15.0, those duplicates have the exact same
                 * properties via public APIs in every way (even their IO flags and
                 * CGDisplayModeGetIODisplayModeID is the same), so we could test
                 * those for equality here too, but I'm intentionally not doing that
                 * in case there are duplicate modes with different IO flags or IO
                 * display mode IDs in the future. In that case I think it's better
                 * to try them all in SetDisplayMode than to risk one of them being
                 * correct but it being filtered out by SDL_AddFullscreenDisplayMode
                 * as being a duplicate.
                 */
                if width == other_w
                    && height == other_h
                    && pixel_w == otherpixel_w
                    && pixel_h == otherpixel_h
                    && usable_for_gui == other_gui
                    && refreshrate == otherrefresh
                    && format == otherformat
                {
                    CFArrayAppendValue(modes.get() as *mut c_void, othermode);
                }
            }
        }

        let mode = DisplayMode {
            format,
            w: width as i32,
            h: height as i32,
            pixel_density: pixel_w as f32 / width as f32,
            refresh_rate: refreshrate,
            ..DisplayMode::default()
        };
        Some((mode, modes))
    }
}

/// Translation of `Cocoa_GetDisplayName()`.
fn get_display_name(display_id: CGDirectDisplayID) -> Option<String> {
    if available!(macos(10, 15)) {
        let screen = get_ns_screen_for_display_id(display_id);
        if !screen.is_null() {
            // SAFETY: an NSScreen's localized name.
            let name: Id = unsafe { msg_send![screen, localizedName] };
            // SAFETY: an NSString or nil.
            if let Some(name) = unsafe { crate::core::apple::foundation::string_from_ns(name) } {
                return Some(name);
            }
        }
    }

    // This API is deprecated in 10.9 with no good replacement (as of 10.15).
    // (CGDisplayIOServicePort() is looked up at run time: newer SDKs don't
    // declare it)
    type CGDisplayIOServicePortFn = unsafe extern "C" fn(CGDirectDisplayID) -> io_service_t;
    // SAFETY: the symbol has this signature where it exists.
    let service_port_fn = unsafe {
        let p = libc::dlsym(libc::RTLD_DEFAULT, c"CGDisplayIOServicePort".as_ptr());
        if p.is_null() {
            None
        } else {
            Some(std::mem::transmute::<*mut c_void, CGDisplayIOServicePortFn>(p))
        }
    }?;
    // SAFETY: the dictionary IOKit creates is released by CfRef; its
    // product names are a dictionary of strings.
    unsafe {
        let service_port = service_port_fn(display_id);
        let device_info = CfRef::from_owned(IODisplayCreateInfoDictionary(
            service_port,
            kIODisplayOnlyPreferredName,
        ))?;
        let localized_names: Id = crate::core::apple::foundation::cf_dictionary_get(
            device_info.get(),
            kDisplayProductName,
        ) as Id;
        let mut display_name = None;

        let count: NSUInteger = msg_send![localized_names, count];
        if count > 0 {
            let keys: Id = msg_send![localized_names, allKeys];
            let first: Id = msg_send![keys, objectAtIndex: 0 as NSUInteger];
            let name: Id = msg_send![localized_names, objectForKey: first];
            display_name = crate::core::apple::foundation::string_from_ns(name);
        }
        display_name
    }
}

/// Translation of `Cocoa_GetHDRProperties()`.
fn get_hdr_properties(display_id: CGDirectDisplayID) -> HdrOutputProperties {
    let mut hdr = HdrOutputProperties {
        sdr_white_level: 1.0,
        hdr_headroom: 1.0,
    };

    if available!(macos(10, 15)) {
        let screen = get_ns_screen_for_display_id(display_id);
        if !screen.is_null() {
            // SAFETY: an NSScreen's EDR properties.
            unsafe {
                let current: f64 =
                    msg_send![screen, maximumExtendedDynamicRangeColorComponentValue];
                if current > 1.0 {
                    hdr.hdr_headroom = current as f32;
                } else {
                    let potential: f64 = msg_send![
                        screen,
                        maximumPotentialExtendedDynamicRangeColorComponentValue
                    ];
                    hdr.hdr_headroom = potential as f32;
                }
            }
        }
    }
    hdr
}

/// Translation of `Cocoa_GetUsableBounds()`.
fn get_usable_bounds(data: &VideoData, display_id: CGDirectDisplayID) -> Option<Rect> {
    let screen = get_ns_screen_for_display_id(display_id);

    if screen.is_null() {
        return None;
    }

    // SAFETY: an NSScreen's visible frame.
    let frame: crate::core::apple::foundation::NSRect = unsafe { msg_send![screen, visibleFrame] };
    let main_display_height = data.main_display_height();
    Some(Rect::new(
        frame.origin.x as i32,
        (main_display_height - frame.origin.y - frame.size.height) as i32,
        frame.size.width as i32,
        frame.size.height as i32,
    ))
}

/// Translation of `Cocoa_AddDisplay()`.
pub(crate) fn add_display(data: &VideoData, display: CGDirectDisplayID, send_event: bool) -> bool {
    // SAFETY: CoreGraphics and CoreVideo objects, released here.
    unsafe {
        let moderef = CGDisplayCopyDisplayMode(display);
        if moderef.is_null() {
            return false;
        }

        let mut link: CVDisplayLinkRef = std::ptr::null_mut();
        CVDisplayLinkCreateWithCGDisplay(display, &mut link);

        let mut viddisplay = VideoDisplay::new();
        viddisplay.name = get_display_name(display);

        let mode = get_display_mode(moderef, true, std::ptr::null(), link);
        if !link.is_null() {
            CVDisplayLinkRelease(link);
        }
        CGDisplayModeRelease(moderef);
        let Some((mode, modes)) = mode else {
            return false;
        };

        viddisplay.hdr = get_hdr_properties(display);

        let usable_bounds = get_usable_bounds(data, display).unwrap_or_default();

        viddisplay.desktop_mode = mode;
        viddisplay.internal = Some(Box::new(DisplayData {
            display,
            usable_bounds,
            desktop_modes: modes,
            modes: Vec::new(),
        }));
        add_video_display(viddisplay, send_event) != 0
    }
}

/// Translation of `Cocoa_DisplayReconfigurationCallback()`.
unsafe extern "C" fn display_reconfiguration_callback(
    displayid: CGDirectDisplayID,
    mut flags: CGDisplayChangeSummaryFlags,
    _user_info: *mut c_void,
) {
    let Some(data) = video_data() else {
        return;
    };
    let mut display = find_sdl_display_by_cg_direct_display_id(displayid); // will be NULL for newly-added (or newly-unmirrored) displays!

    if flags & kCGDisplayDisabledFlag != 0 {
        flags |= kCGDisplayRemoveFlag; // treat this like a display leaving, even though it's still plugged in.
    }

    if flags & kCGDisplayEnabledFlag != 0 {
        flags |= kCGDisplayAddFlag; // treat this like a display leaving, even though it's still plugged in.
    }

    if flags & kCGDisplayMirrorFlag != 0 {
        flags |= kCGDisplayRemoveFlag; // treat this like a display leaving, even though it's still actually here.
    }

    if flags & kCGDisplayUnMirrorFlag != 0 {
        flags |= kCGDisplayAddFlag; // treat this like a new display arriving, even though it was here all along.
    }

    if (flags & kCGDisplayAddFlag) != 0 && (flags & kCGDisplayRemoveFlag) != 0 {
        // sometimes you get a removed device that gets Add and Remove flags at the same time but the display dimensions are 0x0 or 1x1, hence the `> 1` test.
        // Mirrored things are always removed, since they don't represent a discrete display in this state.
        // SAFETY: a display ID from CoreGraphics.
        if (flags & kCGDisplayMirrorFlag) == 0 && unsafe { CGDisplayPixelsWide(displayid) } > 1 {
            // Final state is connected
            flags &= !kCGDisplayRemoveFlag;
        } else {
            // Final state is disconnected
            flags &= !kCGDisplayAddFlag;
        }
    }

    if flags & kCGDisplayAddFlag != 0 && display.is_none() {
        if !add_display(&data, displayid, true) {
            return; // oh well.
        }
        display = find_sdl_display_by_cg_direct_display_id(displayid);
        crate::sdl_assert!(display.is_some());
    }

    if flags & kCGDisplayRemoveFlag != 0 {
        if let Some(d) = display.take() {
            del_video_display(d, true);
        }
    }

    if flags & kCGDisplaySetModeFlag != 0 {
        if let Some(d) = display {
            // SAFETY: CoreGraphics and CoreVideo objects, released here.
            unsafe {
                let moderef = CGDisplayCopyDisplayMode(displayid);
                if !moderef.is_null() {
                    let mut link: CVDisplayLinkRef = std::ptr::null_mut();
                    CVDisplayLinkCreateWithCGDisplay(displayid, &mut link);
                    if !link.is_null() {
                        if let Some((mode, modes)) =
                            get_display_mode(moderef, true, std::ptr::null(), link)
                        {
                            with_display_data(d, |dd| dd.desktop_modes = modes);
                            set_desktop_display_mode(d, &mode);
                        }
                        CVDisplayLinkRelease(link);
                    }
                    CGDisplayModeRelease(moderef);
                }
            }
        }
    }

    if flags & kCGDisplaySetMainFlag != 0 {
        if let Some(d) = display {
            let moved = with_device(|v| {
                let i = v.displays.iter().position(|x| x.id == d)?;
                if i > 0 {
                    // move this display to the front of _this->displays so it's treated as primary.
                    let item = v.displays.remove(i);
                    v.displays.insert(0, item);
                }
                Some(())
            })
            .ok()
            .flatten();
            if moved.is_some() {
                flags |= kCGDisplayMovedFlag; // we don't have an SDL event atm for "this display became primary," so at least let everyone know it "moved".
            }
        }
    }

    if flags & kCGDisplayMovedFlag != 0 {
        if let Some(d) = display {
            crate::events::window::send_display_event(d, EventType::DISPLAY_MOVED, 0, 0);
        }
    }

    if flags & kCGDisplayDesktopShapeChangedFlag != 0 {
        update_desktop_bounds();
    }

    // SAFETY: the main display's height.
    let height = unsafe { CGDisplayPixelsHigh(kCGDirectMainDisplay()) } as f64;
    data.state().main_display_height = height;
}

/// Add the online displays (the main one first) and watch for changes.
/// Translation of `Cocoa_InitModes()`.
pub(crate) fn init_modes(data: &Arc<VideoData>) {
    autoreleasepool(|| {
        // SAFETY: CoreGraphics display queries into buffers of the counts
        // given.
        unsafe {
            data.state().main_display_height = CGDisplayPixelsHigh(kCGDirectMainDisplay()) as f64;

            let mut num_displays: CGDisplayCount = 0;
            let result = CGGetOnlineDisplayList(0, std::ptr::null_mut(), &mut num_displays);
            if result != kCGErrorSuccess {
                // (upstream sets the error, which nothing reads)
                let _ = cg_set_error("CGGetOnlineDisplayList()", result);
                return;
            }

            let mut displays = vec![0 as CGDirectDisplayID; num_displays as usize];

            let result =
                CGGetOnlineDisplayList(num_displays, displays.as_mut_ptr(), &mut num_displays);
            if result != kCGErrorSuccess {
                // (upstream sets the error, which nothing reads)
                let _ = cg_set_error("CGGetOnlineDisplayList()", result);
                return;
            }
            displays.truncate(num_displays as usize);

            // future updates to the display graph will come through this callback.
            CGDisplayRegisterReconfigurationCallback(
                display_reconfiguration_callback,
                std::ptr::null_mut(),
            );

            // Pick up the primary display in the first pass, then get the rest
            for pass in 0..2 {
                for &display in &displays {
                    if pass == 0 {
                        if CGDisplayIsMain(display) == 0 {
                            continue;
                        }
                    } else if CGDisplayIsMain(display) != 0 {
                        continue;
                    }

                    if CGDisplayMirrorsDisplay(display) != kCGNullDirectDisplay {
                        continue;
                    }

                    add_display(data, display, false);
                }
            }
        }
    })
}

/// Translation of `Cocoa_UpdateDisplays()`.
pub(crate) fn update_displays() {
    let Some(data) = video_data() else {
        return;
    };
    let displays = with_device(|v| {
        v.displays
            .iter()
            .filter_map(|d| {
                d.internal
                    .as_ref()
                    .and_then(|i| i.downcast_ref::<DisplayData>())
                    .map(|dd| (d.id, dd.display, dd.usable_bounds))
            })
            .collect::<Vec<_>>()
    })
    .unwrap_or_default();

    for (id, cgdisplay, usable_bounds) in displays {
        let hdr = get_hdr_properties(cgdisplay);
        set_display_hdr_properties(id, hdr);

        if let Some(rect) = get_usable_bounds(&data, cgdisplay) {
            if usable_bounds != rect {
                with_display_data(id, |dd| dd.usable_bounds = rect);
                crate::events::window::send_display_event(
                    id,
                    EventType::DISPLAY_USABLE_BOUNDS_CHANGED,
                    0,
                    0,
                );
            }
        }
    }
}

/// Translation of `Cocoa_GetDisplayBounds()`.
pub(crate) fn get_display_bounds(display: DisplayID) -> Result<Rect> {
    let cgdisplay =
        with_display_data(display, |d| d.display).ok_or_else(|| Error::new("Invalid display"))?;
    // SAFETY: a display ID from CoreGraphics.
    let cgrect = unsafe { CGDisplayBounds(cgdisplay) };
    Ok(Rect::new(
        cgrect.origin.x as i32,
        cgrect.origin.y as i32,
        cgrect.size.width as i32,
        cgrect.size.height as i32,
    ))
}

/// Translation of `Cocoa_GetDisplayUsableBounds()`.
pub(crate) fn get_display_usable_bounds(display: DisplayID) -> Result<Rect> {
    with_display_data(display, |d| d.usable_bounds).ok_or_else(|| Error::new("Invalid display"))
}

/// Add a display's fullscreen modes. Translation of `Cocoa_GetDisplayModes()`.
pub(crate) fn get_display_modes(display: DisplayID) {
    let Some(cgdisplay) = with_display_data(display, |d| d.display) else {
        return;
    };
    // SAFETY: CoreGraphics and CoreVideo objects, released here (the
    // mode list by CfRef).
    unsafe {
        let mut link: CVDisplayLinkRef = std::ptr::null_mut();
        CVDisplayLinkCreateWithCGDisplay(cgdisplay, &mut link);

        /* By default, CGDisplayCopyAllDisplayModes will only get a subset of the
         * system's available modes. For example on a 15" 2016 MBP, users can
         * choose 1920x1080@2x in System Preferences but it won't show up here,
         * unless we specify the option below.
         * The display modes returned by CGDisplayCopyAllDisplayModes are also not
         * high dpi-capable unless this option is set.
         * macOS 10.15 also seems to have a bug where entering, exiting, and
         * re-entering exclusive fullscreen with a low dpi display mode can cause
         * the content of the screen to move up, which this setting avoids:
         * https://bugzilla.libsdl.org/show_bug.cgi?id=4822
         */

        let dictkeys = [kCGDisplayShowDuplicateLowResolutionModes];
        let dictvalues = [kCFBooleanTrue];
        let dict = CfRef::from_owned(CFDictionaryCreate(
            kCFAllocatorDefault,
            dictkeys.as_ptr(),
            dictvalues.as_ptr(),
            1,
            &kCFCopyStringDictionaryKeyCallBacks,
            &kCFTypeDictionaryValueCallBacks,
        ));

        let modes = CfRef::from_owned(CGDisplayCopyAllDisplayModes(
            cgdisplay,
            dict.as_ref().map_or(std::ptr::null(), |d| d.get()),
        ));
        drop(dict);

        if let Some(modes) = modes {
            let count: CFIndex = CFArrayGetCount(modes.get());

            for i in 0..count {
                let moderef = CFArrayGetValueAtIndex(modes.get(), i) as CGDisplayModeRef;

                if let Some((mode, data)) = get_display_mode(moderef, false, modes.get(), link) {
                    if add_fullscreen_display_mode(display, &mode) {
                        let mut finalized = mode;
                        finalized.display_id = display;
                        finalize_display_mode(&mut finalized);
                        with_display_data(display, |d| d.modes.push((finalized, data)));
                    }
                }
            }
        }

        if !link.is_null() {
            CVDisplayLinkRelease(link);
        }
    }
}

/// Translation of `SetDisplayModeForDisplay()`.
fn set_display_mode_for_display(display: CGDirectDisplayID, modes: &CfRef) -> CGError {
    /* SDL_DisplayModeData can contain multiple CGDisplayModes to try (with
     * identical properties), some of which might not work. See GetDisplayMode.
     */
    let mut result = kCGErrorFailure;
    // SAFETY: a mutable array of CGDisplayModes.
    unsafe {
        let count = CFArrayGetCount(modes.get());
        for i in 0..count {
            let moderef = CFArrayGetValueAtIndex(modes.get(), i) as CGDisplayModeRef;
            result = CGDisplaySetDisplayMode(display, moderef, std::ptr::null());
            if result == kCGErrorSuccess {
                // If this mode works, try it first next time.
                if i > 0 {
                    CFArrayExchangeValuesAtIndices(modes.get() as *mut c_void, i, 0);
                }
                break;
            }
        }
    }
    result
}

/// Translation of `Cocoa_SetDisplayMode()`.
pub(crate) fn set_display_mode(display: DisplayID, mode: &DisplayMode) -> Result<()> {
    let desktop_mode =
        with_display(display, |d| d.desktop_mode).ok_or_else(|| Error::new("Invalid display"))?;
    // (upstream compares the modes' internal pointers: the desktop mode is
    // the one the video core passes from display->desktop_mode)
    let is_desktop = *mode == desktop_mode;
    let found = with_display_data(display, |d| {
        let data = if is_desktop {
            Some(d.desktop_modes.clone())
        } else {
            d.modes
                .iter()
                .find(|(m, _)| m == mode || crate::video::display::cmpmodes(m, mode) == 0)
                .map(|(_, data)| data.clone())
        };
        (d.display, data)
    });
    let Some((cgdisplay, Some(data))) = found else {
        return Err(Error::new("Invalid display mode"));
    };

    let mut fade_token = kCGDisplayFadeReservationInvalidToken;
    let mut result = kCGErrorSuccess;

    super::window::B_IN_MODE_TRANSITION.store(true, std::sync::atomic::Ordering::Relaxed);

    // SAFETY: a fade reservation, released below.
    unsafe {
        // Fade to black to hide resolution-switching flicker
        if CGAcquireDisplayFadeReservation(5.0, &mut fade_token) == kCGErrorSuccess {
            CGDisplayFade(
                fade_token,
                0.3,
                kCGDisplayBlendNormal,
                kCGDisplayBlendSolidColor,
                0.0,
                0.0,
                0.0,
                1,
            );
        }
    }

    if is_desktop {
        // Restoring desktop mode
        set_display_mode_for_display(cgdisplay, &data);
    } else {
        // Do the physical switch
        result = set_display_mode_for_display(cgdisplay, &data);
    }

    // SAFETY: the reservation acquired above.
    unsafe {
        // Fade in again (asynchronously)
        if fade_token != kCGDisplayFadeReservationInvalidToken {
            CGDisplayFade(
                fade_token,
                0.5,
                kCGDisplayBlendSolidColor,
                kCGDisplayBlendNormal,
                0.0,
                0.0,
                0.0,
                0,
            );
            CGReleaseDisplayFadeReservation(fade_token);
        }
    }

    super::window::B_IN_MODE_TRANSITION.store(false, std::sync::atomic::Ordering::Relaxed);

    if result != kCGErrorSuccess {
        return Err(cg_set_error("CGDisplaySwitchToMode()", result));
    }
    Ok(())
}

/// Restore the desktop modes and stop watching for changes. Translation of
/// `Cocoa_QuitModes()`.
pub(crate) fn quit_modes(_data: &Arc<VideoData>) {
    // SAFETY: the callback registered by init_modes().
    unsafe {
        CGDisplayRemoveReconfigurationCallback(
            display_reconfiguration_callback,
            std::ptr::null_mut(),
        );
    }

    let displays = with_device(|v| {
        v.displays
            .iter()
            .map(|d| (d.id, d.current_mode(), d.desktop_mode))
            .collect::<Vec<_>>()
    })
    .unwrap_or_default();
    for (id, current, desktop) in displays {
        if current != desktop {
            let _ = set_display_mode(id, &desktop);
        }

        // (the mode lists are released with the display data)
    }
}

/// The name a `CFString` carries (for the HDR/name helpers' tests).
#[allow(dead_code)]
pub(crate) fn cf_to_string(s: crate::core::apple::foundation::CFStringRef) -> Option<String> {
    // SAFETY: a CFString or null.
    unsafe { string_from_cf(s) }
}
