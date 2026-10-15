// Rust translation of src/video/cocoa/SDL_cocoakeyboard.m and
// SDL_cocoakeyboard.h from Simple DirectMedia Layer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! The keyboard: key events (scancodes from `scancodes_darwin.h`, the
//! modifier keys from the device-dependent modifier flags), the keymap of
//! the current keyboard layout (`UCKeyTranslate()`), the Option-as-Alt
//! hint, and text input through `SDL3TranslatorResponder`, the field
//! editor implementing `NSTextInputClient` for the input method.

use std::cell::RefCell;
use std::ffi::c_void;
use std::sync::Arc;
use std::time::Duration;

use super::events::{encoding, get_event_timestamp};
use super::sys::*;
use super::{video_data, OptionAsAlt, VideoData};
use crate::core::apple::foundation::{CFDataGetBytePtr, CfRef, NSPoint, NSRange, NSRect};
use crate::core::apple::objc::{
    autoreleasepool, class, imp, is_kind_of, ivar_ptr, msg_send, msg_send_super, sel, Bool, Class,
    ClassBuilder, DeclaredClass, Id, NSInteger, NSUInteger, Sel, StrongPtr, NIL, NO,
};
use crate::error::Result;
use crate::events::keyboard::{
    keyboard_focus, send_editing_text, send_keyboard_key, send_keyboard_text, set_keymap,
    text_input_active, toggle_mod_state, Keycode, Keymap, Keymod, Scancode, DEFAULT_KEYBOARD_ID,
    GLOBAL_KEYBOARD_ID,
};
use crate::events::scancodes_darwin::DARWIN_SCANCODE_TABLE;
use crate::events::WindowID;
use crate::hints;
use crate::properties::Properties;
use crate::video::Rect;

/// The instance variables of `SDL3TranslatorResponder`.
struct ResponderState {
    marked_text: StrongPtr,
    marked_range: NSRange,
    selected_range: NSRange,
    input_rect: Rect,
    pending_raw_code: i32,
    pending_scancode: Scancode,
    pending_timestamp: Duration,
}

static SDL3_TRANSLATOR_RESPONDER: DeclaredClass = DeclaredClass::new();

/// The `SDL3TranslatorResponder` class (`@interface SDL3TranslatorResponder
/// : NSView <NSTextInputClient>`), its instance variables in a Rust
/// structure the `_state` variable points to.
fn sdl3_translator_responder_class() -> Class {
    SDL3_TRANSLATOR_RESPONDER.get(|| {
        let mut b = ClassBuilder::new(c"SDL3TranslatorResponder", class!(NSView))
            .expect("SDL3TranslatorResponder");
        b.add_protocol(c"NSTextInputClient");
        b.add_ivar::<*mut c_void>(c"_state", c"^v");
        let range = "{_NSRange=QQ}";
        let rect = "{CGRect={CGPoint=dd}{CGSize=dd}}";
        // SAFETY: the implementations have the signatures of the encodings.
        unsafe {
            b.add_method(
                sel!(initWithFrame:),
                imp!(responder_init_with_frame as extern "C" fn(Id, Sel, NSRect) -> Id),
                &encoding(&format!("@@:{rect}")),
            );
            b.add_method(
                sel!(dealloc),
                imp!(responder_dealloc as extern "C" fn(Id, Sel)),
                c"v@:",
            );
            b.add_method(
                sel!(insertText:replacementRange:),
                imp!(responder_insert_text as extern "C" fn(Id, Sel, Id, NSRange)),
                &encoding(&format!("v@:@{range}")),
            );
            b.add_method(
                sel!(doCommandBySelector:),
                imp!(responder_do_command_by_selector as extern "C" fn(Id, Sel, Sel)),
                c"v@::",
            );
            b.add_method(
                sel!(hasMarkedText),
                imp!(responder_has_marked_text as extern "C" fn(Id, Sel) -> Bool),
                &encoding("B@:"),
            );
            b.add_method(
                sel!(markedRange),
                imp!(responder_marked_range as extern "C" fn(Id, Sel) -> NSRange),
                &encoding(&format!("{range}@:")),
            );
            b.add_method(
                sel!(selectedRange),
                imp!(responder_selected_range as extern "C" fn(Id, Sel) -> NSRange),
                &encoding(&format!("{range}@:")),
            );
            b.add_method(
                sel!(setMarkedText:selectedRange:replacementRange:),
                imp!(responder_set_marked_text as extern "C" fn(Id, Sel, Id, NSRange, NSRange)),
                &encoding(&format!("v@:@{range}{range}")),
            );
            b.add_method(
                sel!(unmarkText),
                imp!(responder_unmark_text as extern "C" fn(Id, Sel)),
                c"v@:",
            );
            b.add_method(
                sel!(firstRectForCharacterRange:actualRange:),
                imp!(
                    responder_first_rect_for_character_range
                        as extern "C" fn(Id, Sel, NSRange, *mut NSRange) -> NSRect
                ),
                &encoding(&format!("{rect}@:{range}^{range}")),
            );
            b.add_method(
                sel!(attributedSubstringForProposedRange:actualRange:),
                imp!(
                    responder_attributed_substring
                        as extern "C" fn(Id, Sel, NSRange, *mut NSRange) -> Id
                ),
                &encoding(&format!("@@:{range}^{range}")),
            );
            b.add_method(
                sel!(conversationIdentifier),
                imp!(responder_conversation_identifier as extern "C" fn(Id, Sel) -> NSInteger),
                c"q@:",
            );
            b.add_method(
                sel!(characterIndexForPoint:),
                imp!(
                    responder_character_index_for_point
                        as extern "C" fn(Id, Sel, NSPoint) -> NSUInteger
                ),
                c"Q@:{CGPoint=dd}",
            );
            b.add_method(
                sel!(validAttributesForMarkedText),
                imp!(responder_valid_attributes_for_marked_text as extern "C" fn(Id, Sel) -> Id),
                c"@@:",
            );
        }
        b.register()
    })
}

/// The instance variables of an `SDL3TranslatorResponder`, borrowed for
/// `f` (never across a call into AppKit or SDL's event functions).
fn with_responder<R>(this: Id, f: impl FnOnce(&mut ResponderState) -> R) -> R {
    // SAFETY: an SDL3TranslatorResponder, whose _state is the RefCell made
    // in initWithFrame: and freed in dealloc.
    let cell = unsafe {
        let p = *ivar_ptr::<*mut c_void>(this, sdl3_translator_responder_class(), c"_state");
        &*(p as *const RefCell<ResponderState>)
    };
    f(&mut cell.borrow_mut())
}

/// The responder's `initWithFrame:` (NSView's, then the instance variables).
extern "C" fn responder_init_with_frame(this: Id, _sel: Sel, frame: NSRect) -> Id {
    // SAFETY: NSView's initializer, then the state variable of the new
    // object.
    unsafe {
        let this: Id =
            msg_send_super![this, sdl3_translator_responder_class(), initWithFrame: frame];
        if !this.is_null() {
            let state = Box::new(RefCell::new(ResponderState {
                marked_text: StrongPtr::nil(),
                marked_range: NSRange::default(),
                selected_range: NSRange::default(),
                input_rect: Rect::default(),
                pending_raw_code: -1,
                pending_scancode: Scancode::UNKNOWN,
                pending_timestamp: Duration::ZERO,
            }));
            *ivar_ptr::<*mut c_void>(this, sdl3_translator_responder_class(), c"_state") =
                Box::into_raw(state) as *mut c_void;
        }
        this
    }
}

/// Free the instance variables (ARC's `-dealloc`).
extern "C" fn responder_dealloc(this: Id, _sel: Sel) {
    // SAFETY: the state made in initWithFrame:, freed once; then NSView's
    // dealloc.
    unsafe {
        let p = ivar_ptr::<*mut c_void>(this, sdl3_translator_responder_class(), c"_state");
        if !(*p).is_null() {
            drop(Box::from_raw(*p as *mut RefCell<ResponderState>));
            *p = std::ptr::null_mut();
        }
        let _: () = msg_send_super![this, sdl3_translator_responder_class(), dealloc];
    }
}

/// Translation of `-[SDL3TranslatorResponder setInputRect:]`.
fn set_input_rect(this: Id, rect: &Rect) {
    with_responder(this, |s| s.input_rect = *rect);
}

/// The UTF-8 text of an `NSString` or `NSAttributedString`.
///
/// # Safety
///
/// `a_string` must be one of those (or nil).
unsafe fn plain_string(a_string: Id) -> Id {
    // SAFETY: the caller's contract.
    unsafe {
        if is_kind_of(a_string, class!(NSAttributedString)) {
            msg_send![a_string, string]
        } else {
            a_string
        }
    }
}

/// Translation of `-[SDL3TranslatorResponder insertText:replacementRange:]`.
extern "C" fn responder_insert_text(this: Id, _sel: Sel, a_string: Id, replacement_range: NSRange) {
    // DEBUG_IME(@"insertText: %@ replacementRange: (%d, %d)", aString,
    //           (int)replacementRange.location, (int)replacementRange.length);

    /* Could be NSString or NSAttributedString, so we have
     * to test and convert it before return as SDL event */
    // SAFETY: an NSString or NSAttributedString.
    let str = unsafe { super::ns_to_string(plain_string(a_string)) };

    // We're likely sending the composed text, so we reset the IME status.
    if with_responder(this, |s| !s.marked_text.is_nil()) {
        responder_unmark_text(this, sel!(unmarkText));
    }

    // Deliver the raw key event that generated this text
    send_pending_key(this);

    if replacement_range.location as i32 != -1 {
        // We're replacing the last character
        send_keyboard_key(
            Duration::ZERO,
            GLOBAL_KEYBOARD_ID,
            0,
            Scancode::BACKSPACE,
            true,
        );
        send_keyboard_key(
            Duration::ZERO,
            GLOBAL_KEYBOARD_ID,
            0,
            Scancode::BACKSPACE,
            false,
        );
    }

    send_keyboard_text(&str);
}

/// Translation of `-[SDL3TranslatorResponder doCommandBySelector:]`.
extern "C" fn responder_do_command_by_selector(_this: Id, _sel: Sel, _myselector: Sel) {
    /* No need to do anything since we are not using Cocoa
    selectors to handle special keys, instead we use SDL
    key events to do the same job.
    */
}

/// Translation of `-[SDL3TranslatorResponder hasMarkedText]`.
extern "C" fn responder_has_marked_text(this: Id, _sel: Sel) -> Bool {
    Bool::from(with_responder(this, |s| !s.marked_text.is_nil()))
}

/// Translation of `-[SDL3TranslatorResponder markedRange]`.
extern "C" fn responder_marked_range(this: Id, _sel: Sel) -> NSRange {
    with_responder(this, |s| s.marked_range)
}

/// Translation of `-[SDL3TranslatorResponder selectedRange]`.
extern "C" fn responder_selected_range(this: Id, _sel: Sel) -> NSRange {
    with_responder(this, |s| s.selected_range)
}

/// The length in UTF-32 code points of the first `index` UTF-16 units of a
/// string (`[[aString substringToIndex:index] lengthOfBytesUsingEncoding:
/// NSUTF32StringEncoding] / 4`).
///
/// # Safety
///
/// `a_string` must be an `NSString` at least `index` units long.
unsafe fn utf32_length_to(a_string: Id, index: NSUInteger) -> NSUInteger {
    // SAFETY: the caller's contract.
    unsafe {
        let sub: Id = msg_send![a_string, substringToIndex: index];
        let bytes: NSUInteger = msg_send![sub, lengthOfBytesUsingEncoding: NSUTF32StringEncoding];
        bytes / 4
    }
}

/// Translation of `-[SDL3TranslatorResponder setMarkedText:selectedRange:replacementRange:]`.
extern "C" fn responder_set_marked_text(
    this: Id,
    _sel: Sel,
    a_string: Id,
    selected_range: NSRange,
    _replacement_range: NSRange,
) {
    // SAFETY: an NSString or NSAttributedString.
    unsafe {
        let a_string = plain_string(a_string);

        let length: NSUInteger = msg_send![a_string, length];
        if length == 0 {
            responder_unmark_text(this, sel!(unmarkText));
            return;
        }

        with_responder(this, |s| {
            if s.marked_text.get() != a_string {
                s.marked_text = StrongPtr::retain(a_string);
            }

            s.selected_range = selected_range;
            s.marked_range = NSRange::new(0, length);
        });

        // This key event was consumed by the IME
        clear_pending_key(this);

        let utf32_selected_range_location = utf32_length_to(a_string, selected_range.location);
        let utf32_selection_range_end =
            utf32_length_to(a_string, selected_range.location + selected_range.length);
        let utf32_selection_range_length =
            utf32_selection_range_end - utf32_selected_range_location;

        send_editing_text(
            &super::ns_to_string(a_string),
            utf32_selected_range_location as i32,
            utf32_selection_range_length as i32,
        );
    }

    // DEBUG_IME(@"setMarkedText: %@, (%d, %d) replacement range (%d, %d)", _markedText,
    //           (int)selectedRange.location, (int)selectedRange.length,
    //           (int)replacementRange.location, (int)replacementRange.length);
}

/// Translation of `-[SDL3TranslatorResponder unmarkText]`.
extern "C" fn responder_unmark_text(this: Id, _sel: Sel) {
    let marked = with_responder(this, |s| std::mem::take(&mut s.marked_text));
    drop(marked);

    // This key event was consumed by the IME
    clear_pending_key(this);

    send_editing_text("", 0, 0);
}

/// Translation of `-[SDL3TranslatorResponder firstRectForCharacterRange:actualRange:]`.
extern "C" fn responder_first_rect_for_character_range(
    this: Id,
    _sel: Sel,
    a_range: NSRange,
    actual_range: *mut NSRange,
) -> NSRect {
    let input_rect = with_responder(this, |s| s.input_rect);
    // SAFETY: the view's NSWindow; the out pointer is null or valid.
    unsafe {
        let window: Id = msg_send![this, window];
        let frame: NSRect = msg_send![window, frame];
        let content_rect: NSRect = msg_send![window, contentRectForFrameRect: frame];
        let window_height = content_rect.size.height as f32;
        let rect = NSRect::new(
            input_rect.x as f64,
            (window_height - input_rect.y as f32 - input_rect.h as f32) as f64,
            input_rect.w as f64,
            input_rect.h as f64,
        );

        if !actual_range.is_null() {
            *actual_range = a_range;
        }

        // DEBUG_IME(@"firstRectForCharacterRange: (%d, %d): windowHeight = %g, rect = %@",
        //           (int)aRange.location, (int)aRange.length, windowHeight,
        //           NSStringFromRect(rect));

        msg_send![window, convertRectToScreen: rect]
    }
}

/// Translation of `-[SDL3TranslatorResponder attributedSubstringForProposedRange:actualRange:]`.
extern "C" fn responder_attributed_substring(
    _this: Id,
    _sel: Sel,
    _a_range: NSRange,
    _actual_range: *mut NSRange,
) -> Id {
    // DEBUG_IME(@"attributedSubstringFromRange: (%d, %d)", (int)aRange.location, (int)aRange.length);
    NIL
}

/// Translation of `-[SDL3TranslatorResponder conversationIdentifier]`.
extern "C" fn responder_conversation_identifier(this: Id, _sel: Sel) -> NSInteger {
    this as NSInteger
}

/// This method returns the index for character that is
/// nearest to thePoint.  thPoint is in screen coordinate system.
/// Translation of `-[SDL3TranslatorResponder characterIndexForPoint:]`.
extern "C" fn responder_character_index_for_point(
    _this: Id,
    _sel: Sel,
    _the_point: NSPoint,
) -> NSUInteger {
    // DEBUG_IME(@"characterIndexForPoint: (%g, %g)", thePoint.x, thePoint.y);
    0
}

/// This method is the key to attribute extension.
/// We could add new attributes through this method.
/// NSInputServer examines the return value of this
/// method & constructs appropriate attributed string.
/// Translation of `-[SDL3TranslatorResponder validAttributesForMarkedText]`.
extern "C" fn responder_valid_attributes_for_marked_text(_this: Id, _sel: Sel) -> Id {
    // SAFETY: NSArray's class method.
    unsafe { msg_send![class!(NSArray), array] }
}

/// Translation of `-[SDL3TranslatorResponder setPendingKey:scancode:timestamp:]`.
fn set_pending_key(this: Id, rawcode: i32, scancode: Scancode, timestamp: Duration) {
    with_responder(this, |s| {
        s.pending_raw_code = rawcode;
        s.pending_scancode = scancode;
        s.pending_timestamp = timestamp;
    });
}

/// Translation of `-[SDL3TranslatorResponder sendPendingKey]`.
fn send_pending_key(this: Id) {
    let pending = with_responder(this, |s| {
        (s.pending_raw_code, s.pending_scancode, s.pending_timestamp)
    });
    if pending.0 < 0 {
        return;
    }

    send_keyboard_key(pending.2, DEFAULT_KEYBOARD_ID, pending.0, pending.1, true);
    clear_pending_key(this);
}

/// Translation of `-[SDL3TranslatorResponder clearPendingKey]`.
fn clear_pending_key(this: Id) {
    with_responder(this, |s| s.pending_raw_code = -1);
}

/// Translation of `IsModifierKeyPressed()`.
fn is_modifier_key_pressed(
    flags: u32,
    target_mask: u32,
    other_mask: u32,
    either_mask: u32,
) -> bool {
    let target_pressed = (flags & target_mask) != 0;
    let other_pressed = (flags & other_mask) != 0;
    let either_pressed = (flags & either_mask) != 0;

    if either_pressed != (target_pressed || other_pressed) {
        return either_pressed;
    }

    target_pressed
}

/// Translation of `HandleModifiers()`.
fn handle_modifiers(code: Scancode, modifier_flags: u32) {
    let pressed = if code == Scancode::LSHIFT {
        is_modifier_key_pressed(
            modifier_flags,
            NX_DEVICELSHIFTKEYMASK,
            NX_DEVICERSHIFTKEYMASK,
            NX_SHIFTMASK,
        )
    } else if code == Scancode::LCTRL {
        is_modifier_key_pressed(
            modifier_flags,
            NX_DEVICELCTLKEYMASK,
            NX_DEVICERCTLKEYMASK,
            NX_CONTROLMASK,
        )
    } else if code == Scancode::LALT {
        is_modifier_key_pressed(
            modifier_flags,
            NX_DEVICELALTKEYMASK,
            NX_DEVICERALTKEYMASK,
            NX_ALTERNATEMASK,
        )
    } else if code == Scancode::LGUI {
        is_modifier_key_pressed(
            modifier_flags,
            NX_DEVICELCMDKEYMASK,
            NX_DEVICERCMDKEYMASK,
            NX_COMMANDMASK,
        )
    } else if code == Scancode::RSHIFT {
        is_modifier_key_pressed(
            modifier_flags,
            NX_DEVICERSHIFTKEYMASK,
            NX_DEVICELSHIFTKEYMASK,
            NX_SHIFTMASK,
        )
    } else if code == Scancode::RCTRL {
        is_modifier_key_pressed(
            modifier_flags,
            NX_DEVICERCTLKEYMASK,
            NX_DEVICELCTLKEYMASK,
            NX_CONTROLMASK,
        )
    } else if code == Scancode::RALT {
        is_modifier_key_pressed(
            modifier_flags,
            NX_DEVICERALTKEYMASK,
            NX_DEVICELALTKEYMASK,
            NX_ALTERNATEMASK,
        )
    } else if code == Scancode::RGUI {
        is_modifier_key_pressed(
            modifier_flags,
            NX_DEVICERCMDKEYMASK,
            NX_DEVICELCMDKEYMASK,
            NX_COMMANDMASK,
        )
    } else {
        return;
    };

    send_keyboard_key(Duration::ZERO, DEFAULT_KEYBOARD_ID, 0, code, pressed);
}

/// Whether the keyboard is an ISO one (`KBGetLayoutType(LMGetKbdType()) == kKeyboardISO`).
fn is_iso_keyboard() -> bool {
    // SAFETY: HIToolbox queries without arguments of concern.
    unsafe { KBGetLayoutType(LMGetKbdType() as i16) == kKeyboardISO }
}

/// The modifier states the keymap is made for (`mods[]`).
const KEYMAP_MODS: [(u32, Keymod); 8] = [
    (0, Keymod::NONE),
    (shiftKey, Keymod::SHIFT),
    (alphaLock, Keymod::CAPS),
    (
        shiftKey | alphaLock,
        Keymod(Keymod::SHIFT.0 | Keymod::CAPS.0),
    ),
    (optionKey, Keymod::ALT),
    (
        optionKey | shiftKey,
        Keymod(Keymod::ALT.0 | Keymod::SHIFT.0),
    ),
    (
        optionKey | alphaLock,
        Keymod(Keymod::ALT.0 | Keymod::CAPS.0),
    ),
    (
        optionKey | shiftKey | alphaLock,
        Keymod(Keymod::ALT.0 | Keymod::SHIFT.0 | Keymod::CAPS.0),
    ),
];

/// Make a keymap with `translate` (`UCKeyTranslate()` of a key code under
/// Carbon modifier flags, `None` on error): the part of `UpdateKeymap()`
/// that doesn't depend on the system.
fn build_keymap(iso: bool, mut translate: impl FnMut(u16, u32) -> Option<Vec<u16>>) -> Keymap {
    let mut keymap = Keymap::new();
    for &(flags, modstate) in &KEYMAP_MODS {
        for (i, &table_scancode) in DARWIN_SCANCODE_TABLE.iter().enumerate() {
            // Make sure this scancode is a valid character scancode
            let mut scancode = table_scancode;
            if scancode == Scancode::UNKNOWN
                || scancode == Scancode::DELETE
                || (crate::events::keyboard::keymap::default_key_from_scancode(
                    scancode,
                    Keymod::NONE,
                )
                .0 & Keycode::SCANCODE_MASK)
                    != 0
            {
                continue;
            }

            /*
             * Swap the scancode for these two wrongly translated keys
             * UCKeyTranslate() function does not do its job properly for ISO layout keyboards, where the key '@',
             * which is located in the top left corner of the keyboard right under the Escape key, and the additional
             * key '<', which is on the right of the Shift key, are inverted
             */
            if (scancode == Scancode::NONUSBACKSLASH || scancode == Scancode::GRAVE) && iso {
                // see comments in scancodes_darwin.h
                scancode = Scancode((Scancode::NONUSBACKSLASH.0 + Scancode::GRAVE.0) - scancode.0);
            }

            let Some(s) = translate(i as u16, (flags >> 8) & 0xFF) else {
                continue;
            };

            if !s.is_empty() && s[0] != 0x10 {
                keymap.set_entry(scancode, modstate, Keycode(s[0] as u32));
            } else {
                // The default keymap doesn't have any SDL_KMOD_ALT entries, so we don't need to override them
                if modstate.0 & Keymod::ALT.0 == 0 {
                    keymap.set_entry(scancode, modstate, Keycode::UNKNOWN);
                }
            }
        }
    }
    keymap
}

/// Translation of `UpdateKeymap()`.
fn update_keymap(data: &VideoData, send_event: bool) {
    // See if the keymap needs to be updated
    // SAFETY: the copy is owned by the CfRef.
    let Some(key_layout) =
        (unsafe { CfRef::from_owned(TISCopyCurrentKeyboardLayoutInputSource()) })
    else {
        return;
    };
    {
        let mut st = data.state();
        if st
            .key_layout
            .as_ref()
            .is_some_and(|k| k.get() == key_layout.get())
        {
            // Note (upstream): upstream leaks the copy it compares here.
            return;
        }
        st.key_layout = Some(key_layout.clone());
    }

    // Try Unicode data first
    // SAFETY: a TIS input source; the data lives as long as it.
    let key_layout_ptr = unsafe {
        let uchr_data_ref =
            TISGetInputSourceProperty(key_layout.get(), kTISPropertyUnicodeKeyLayoutData);
        if uchr_data_ref.is_null() {
            std::ptr::null()
        } else {
            CFDataGetBytePtr(uchr_data_ref) as *const UCKeyboardLayout
        }
    };

    if key_layout_ptr.is_null() {
        return;
    }

    // SAFETY: HIToolbox's keyboard type.
    let keyboard_type = unsafe { LMGetKbdType() } as u32;

    let keymap = build_keymap(is_iso_keyboard(), |i, modifier_key_state| {
        let mut s = [0 as UniChar; 8];
        let mut len: UniCharCount = 0;
        let mut dead_key_state: u32 = 0;
        // SAFETY: the layout data of the input source we hold, and buffers
        // of the sizes given.
        let err = unsafe {
            UCKeyTranslate(
                key_layout_ptr,
                i,
                kUCKeyActionDown,
                modifier_key_state,
                keyboard_type,
                kUCKeyTranslateNoDeadKeysMask,
                &mut dead_key_state,
                8,
                &mut len,
                s.as_mut_ptr(),
            )
        };
        if err != noErr {
            return None;
        }
        Some(s[..(len as usize).min(8)].to_vec())
    });
    set_keymap(Some(keymap), send_event);
}

/// Translation of `SDL_MacOptionAsAltChanged()`.
fn mac_option_as_alt_changed(data: &VideoData, hint: Option<&str>) {
    let mut st = data.state();
    match hint {
        Some(hint) if !hint.is_empty() => {
            if hint == "none" {
                st.option_as_alt = OptionAsAlt::None;
            } else if hint == "only_left" {
                st.option_as_alt = OptionAsAlt::OnlyLeft;
            } else if hint == "only_right" {
                st.option_as_alt = OptionAsAlt::OnlyRight;
            } else if hint == "both" {
                st.option_as_alt = OptionAsAlt::Both;
            }
        }
        _ => st.option_as_alt = OptionAsAlt::None,
    }
}

/// Translation of `Cocoa_InitKeyboard()`.
pub(crate) fn init_keyboard(data: &Arc<VideoData>) {
    update_keymap(data, false);

    // Set our own names for the platform-dependent but layout-independent keys
    // This key is NumLock on the MacBook keyboard. :)
    // SDL_SetScancodeName(SDL_SCANCODE_NUMLOCKCLEAR, "Clear");
    let _ = Scancode::LALT.set_name(Some("Left Option"));
    let _ = Scancode::LGUI.set_name(Some("Left Command"));
    let _ = Scancode::RALT.set_name(Some("Right Option"));
    let _ = Scancode::RGUI.set_name(Some("Right Command"));

    // SAFETY: NSEvent's class method.
    let modifier_flags: NSUInteger = unsafe { msg_send![class!(NSEvent), modifierFlags] };
    data.state().modifier_flags = modifier_flags as u32;
    toggle_mod_state(
        Keymod::CAPS,
        (modifier_flags & NSEventModifierFlagCapsLock) != 0,
    );

    let d = data.clone();
    if let Ok(watch) = hints::watch(hints::MAC_OPTION_AS_ALT, move |change| {
        mac_option_as_alt_changed(&d, change.new_value)
    }) {
        data.state().hint_callbacks.push(watch);
    }
}

/// Translation of `Cocoa_StartTextInput()`.
pub(crate) fn start_text_input(
    data: &Arc<VideoData>,
    window: WindowID,
    _props: Option<&Properties>,
) -> Result<()> {
    autoreleasepool(|| {
        let Some(windata) = super::window::window_data(window) else {
            return;
        };
        let nswindow = windata.nswindow();

        // SAFETY: the window's content view and the field editor.
        unsafe {
            let parent_view: Id = msg_send![nswindow, contentView];

            /* We only keep one field editor per process, since only the front most
             * window can receive text input events, so it make no sense to keep more
             * than one copy. When we switched to another window and requesting for
             * text input, simply remove the field editor from its superview then add
             * it to the front most window's content view */
            let mut field_edit = data.state().field_edit.clone();
            if field_edit.is_nil() {
                let fe: Id = msg_send![super::alloc(sdl3_translator_responder_class()),
                    initWithFrame: NSRect::new(0.0, 0.0, 0.0, 0.0)];
                field_edit = StrongPtr::from_owned(fe);
                data.state().field_edit = field_edit.clone();
            }

            let superview: Id = msg_send![field_edit, superview];
            let same: Bool = if superview.is_null() {
                NO
            } else {
                msg_send![superview, isEqual: parent_view]
            };
            if !same.is_true() {
                // DEBUG_IME(@"add fieldEdit to window contentView");
                let _: () = msg_send![field_edit, removeFromSuperview];
                let _: () = msg_send![parent_view, addSubview: field_edit.get()];
                let _: Bool = msg_send![nswindow, makeFirstResponder: field_edit.get()];
            }
        }
    });
    update_text_input_area(data, window)
}

/// Translation of `Cocoa_StopTextInput()`.
pub(crate) fn stop_text_input(data: &Arc<VideoData>, _window: WindowID) -> Result<()> {
    autoreleasepool(|| {
        let field_edit = std::mem::take(&mut data.state().field_edit);
        if !field_edit.is_nil() {
            // SAFETY: the field editor.
            let _: () = unsafe { msg_send![field_edit, removeFromSuperview] };
        }
    });
    Ok(())
}

/// Translation of `Cocoa_UpdateTextInputArea()`.
pub(crate) fn update_text_input_area(data: &Arc<VideoData>, window: WindowID) -> Result<()> {
    let field_edit = data.state().field_edit.clone();
    if !field_edit.is_nil() {
        let rect = crate::video::core::with_window(window, |w| w.text_input_rect)?;
        set_input_rect(field_edit.get(), &rect);
    }
    Ok(())
}

/// Translation of `ReplaceEvent()`: with the Option-as-Alt hint, a key
/// event with Option held gets the characters without the modifiers.
fn replace_event(event: Id, option_as_alt: OptionAsAlt) -> Id {
    if option_as_alt == OptionAsAlt::None {
        return event;
    }

    // SAFETY: an NSEvent.
    let modflags = unsafe {
        let f: NSUInteger = msg_send![event, modifierFlags];
        f as u32
    };

    let mut ignore_alt_characters = false;

    let lalt_pressed = is_modifier_key_pressed(
        modflags,
        NX_DEVICELALTKEYMASK,
        NX_DEVICERALTKEYMASK,
        NX_ALTERNATEMASK,
    );
    let ralt_pressed = is_modifier_key_pressed(
        modflags,
        NX_DEVICERALTKEYMASK,
        NX_DEVICELALTKEYMASK,
        NX_ALTERNATEMASK,
    );

    if (option_as_alt == OptionAsAlt::OnlyLeft && lalt_pressed)
        || (option_as_alt == OptionAsAlt::OnlyRight && ralt_pressed)
        || (option_as_alt == OptionAsAlt::Both && (lalt_pressed || ralt_pressed))
    {
        ignore_alt_characters = true;
    }

    let cmd_pressed = modflags & NX_COMMANDMASK != 0;
    let ctrl_pressed = modflags & NX_CONTROLMASK != 0;

    ignore_alt_characters = ignore_alt_characters && !cmd_pressed && !ctrl_pressed;

    if ignore_alt_characters {
        // SAFETY: an NSEvent and NSEvent's constructor taking its fields.
        unsafe {
            let characters_ignoring_modifiers: Id = msg_send![event, charactersIgnoringModifiers];
            let ty: NSUInteger = msg_send![event, type];
            let location: NSPoint = msg_send![event, locationInWindow];
            let timestamp: f64 = msg_send![event, timestamp];
            let window_number: NSInteger = msg_send![event, windowNumber];
            let is_repeat: Bool = msg_send![event, isARepeat];
            let key_code: u16 = msg_send![event, keyCode];
            return msg_send![class!(NSEvent),
                keyEventWithType: ty
                location: location
                modifierFlags: modflags as NSUInteger
                timestamp: timestamp
                windowNumber: window_number
                context: NIL
                characters: characters_ignoring_modifiers
                charactersIgnoringModifiers: characters_ignoring_modifiers
                isARepeat: is_repeat
                keyCode: key_code];
        }
    }

    event
}

/// The SDL scancode of a Mac virtual key code (with the ISO swap), and the
/// key code as reported.
fn scancode_for_key_code(mut scancode: u16, iso: bool) -> (u16, Scancode) {
    if (scancode == 10 || scancode == 50) && iso {
        // see comments in scancodes_darwin.h
        scancode = 60 - scancode;
    }

    let code = if (scancode as usize) < DARWIN_SCANCODE_TABLE.len() {
        DARWIN_SCANCODE_TABLE[scancode as usize]
    } else {
        // Hmm, does this ever happen?  If so, need to extend the keymap...
        Scancode::UNKNOWN
    };
    (scancode, code)
}

/// Translation of `Cocoa_HandleKeyEvent()`.
pub(crate) fn handle_key_event(mut event: Id) {
    let Some(data) = video_data() else {
        return; // can happen when returning from fullscreen Space on shutdown
    };

    // SAFETY: an NSEvent.
    let ty: NSUInteger = unsafe { msg_send![event, type] };
    if ty == NSEventTypeKeyDown || ty == NSEventTypeKeyUp {
        let option_as_alt = data.state().option_as_alt;
        event = replace_event(event, option_as_alt);
    }

    // SAFETY: an NSEvent.
    let (key_code, timestamp): (u16, f64) =
        unsafe { (msg_send![event, keyCode], msg_send![event, timestamp]) };
    let (scancode, code) = scancode_for_key_code(key_code, is_iso_keyboard());

    match ty {
        NSEventTypeKeyDown => {
            // SAFETY: an NSEvent.
            let is_repeat: Bool = unsafe { msg_send![event, isARepeat] };
            if !is_repeat.is_true() {
                // See if we need to rebuild the keyboard layout
                update_keymap(&data, true);
            }

            let focus = keyboard_focus();
            if focus.is_some_and(text_input_active) {
                let field_edit = data.state().field_edit.clone();
                if !field_edit.is_nil() {
                    set_pending_key(
                        field_edit.get(),
                        scancode as i32,
                        code,
                        get_event_timestamp(timestamp),
                    );
                    // SAFETY: the field editor, given the event to interpret.
                    unsafe {
                        let events: Id = msg_send![class!(NSArray), arrayWithObject: event];
                        let _: () = msg_send![field_edit, interpretKeyEvents: events];
                    }
                    send_pending_key(field_edit.get());
                }
            } else if focus.is_some() {
                send_keyboard_key(
                    get_event_timestamp(timestamp),
                    DEFAULT_KEYBOARD_ID,
                    scancode as i32,
                    code,
                    true,
                );
            }
        }
        NSEventTypeKeyUp => {
            send_keyboard_key(
                get_event_timestamp(timestamp),
                DEFAULT_KEYBOARD_ID,
                scancode as i32,
                code,
                false,
            );
        }
        NSEventTypeFlagsChanged => {
            // see if the new modifierFlags mean any existing keys should be pressed/released...
            // SAFETY: an NSEvent.
            let modflags = unsafe {
                let f: NSUInteger = msg_send![event, modifierFlags];
                f as u32
            };
            handle_modifiers(Scancode::LSHIFT, modflags);
            handle_modifiers(Scancode::LCTRL, modflags);
            handle_modifiers(Scancode::LALT, modflags);
            handle_modifiers(Scancode::LGUI, modflags);
            handle_modifiers(Scancode::RSHIFT, modflags);
            handle_modifiers(Scancode::RCTRL, modflags);
            handle_modifiers(Scancode::RALT, modflags);
            handle_modifiers(Scancode::RGUI, modflags);
        }
        _ => {} // just to avoid compiler warnings
    }
}

/// Translation of `Cocoa_QuitKeyboard()`.
pub(crate) fn quit_keyboard(data: &Arc<VideoData>) {
    // (the keyboard layout copy and the field editor are released here)
    let mut st = data.state();
    st.key_layout = None;
    let field_edit = std::mem::take(&mut st.field_edit);
    drop(st);
    drop(field_edit);
}

/// Translation of `Cocoa_SetWindowKeyboardGrab()`.
pub(crate) fn set_window_keyboard_grab(_window: WindowID, _grabbed: bool) -> Result<()> {
    // (SDL_MAC_NO_SANDBOX builds call the private
    // CGSSetGlobalHotKeyOperatingMode(); upstream doesn't define it)
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn modifier_keys() {
        // Left shift alone
        assert!(is_modifier_key_pressed(
            NX_DEVICELSHIFTKEYMASK | NX_SHIFTMASK,
            NX_DEVICELSHIFTKEYMASK,
            NX_DEVICERSHIFTKEYMASK,
            NX_SHIFTMASK
        ));
        assert!(!is_modifier_key_pressed(
            NX_DEVICELSHIFTKEYMASK | NX_SHIFTMASK,
            NX_DEVICERSHIFTKEYMASK,
            NX_DEVICELSHIFTKEYMASK,
            NX_SHIFTMASK
        ));
        // The device-independent flag without either device flag (as some
        // keyboards and synthesized events report): both sides pressed
        assert!(is_modifier_key_pressed(
            NX_SHIFTMASK,
            NX_DEVICELSHIFTKEYMASK,
            NX_DEVICERSHIFTKEYMASK,
            NX_SHIFTMASK
        ));
        // A stale device flag without the independent one: released
        assert!(!is_modifier_key_pressed(
            NX_DEVICELCMDKEYMASK,
            NX_DEVICELCMDKEYMASK,
            NX_DEVICERCMDKEYMASK,
            NX_COMMANDMASK
        ));
    }

    #[test]
    fn key_codes_and_iso_swap() {
        assert_eq!(scancode_for_key_code(0, false), (0, Scancode::A));
        assert_eq!(
            scancode_for_key_code(10, false),
            (10, Scancode::NONUSBACKSLASH)
        );
        assert_eq!(scancode_for_key_code(50, false), (50, Scancode::GRAVE));
        assert_eq!(scancode_for_key_code(10, true), (50, Scancode::GRAVE));
        assert_eq!(
            scancode_for_key_code(50, true),
            (10, Scancode::NONUSBACKSLASH)
        );
        assert_eq!(scancode_for_key_code(200, false), (200, Scancode::UNKNOWN));
    }

    #[test]
    fn keymaps_from_key_translation() {
        // A fake layout: every key types 'a' + its code, uppercase with
        // shift; F1 types the function key character 0x10; Option gives no
        // character at all.
        let keymap = build_keymap(false, |code, mods| {
            if code == 122 {
                return Some(vec![0x10]);
            }
            if mods & ((optionKey >> 8) & 0xFF) != 0 {
                return Some(vec![]);
            }
            let base = if mods & ((shiftKey >> 8) & 0xFF) != 0 {
                b'A'
            } else {
                b'a'
            };
            Some(vec![(base + (code % 26) as u8) as u16])
        });
        // key code 1 is S: 'b' / 'B'
        assert_eq!(
            keymap.keycode(Scancode::S, Keymod::NONE),
            Keycode(b'b' as u32)
        );
        assert_eq!(
            keymap.keycode(Scancode::S, Keymod::SHIFT),
            Keycode(b'B' as u32)
        );
        // F1 isn't a character key: never translated (scancode-mask keycode)
        assert_eq!(
            keymap.keycode(Scancode::F1, Keymod::NONE),
            crate::events::keyboard::keymap::default_key_from_scancode(Scancode::F1, Keymod::NONE)
        );
        // DELETE is skipped too
        assert_eq!(
            keymap.keycode(Scancode::DELETE, Keymod::NONE),
            crate::events::keyboard::keymap::default_key_from_scancode(
                Scancode::DELETE,
                Keymod::NONE
            )
        );

        // ISO keyboards swap the two keys UCKeyTranslate() mixes up
        let iso = build_keymap(true, |code, _| Some(vec![code + 0x100]));
        assert_eq!(
            iso.keycode(Scancode::GRAVE, Keymod::NONE),
            Keycode(10 + 0x100)
        );
        assert_eq!(
            iso.keycode(Scancode::NONUSBACKSLASH, Keymod::NONE),
            Keycode(50 + 0x100)
        );
    }
}
