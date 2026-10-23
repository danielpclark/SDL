// Rust translation of src/video/cocoa/SDL_cocoaclipboard.m and
// SDL_cocoaclipboard.h from Simple DirectMedia Layer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! The pasteboard: the app's clipboard data offered through
//! `Cocoa_PasteboardDataProvider` (produced on request, by UTI), text set
//! directly so it outlives the app, other apps' data read by mime type or
//! UTI, and clipboard updates noticed through the change count.

use std::ffi::c_void;
use std::sync::{Arc, OnceLock};

use super::sys::*;
use super::VideoData;
use crate::core::apple::foundation::{
    available, cf_string, ns_array_items, ns_string, CFStringRef, CfRef,
};
use crate::core::apple::objc::{
    autoreleasepool, class, imp, ivar_ptr, msg_send, msg_send_super, sel, Bool, Class,
    ClassBuilder, DeclaredClass, Id, NSInteger, NSUInteger, Sel, StrongPtr,
};
use crate::error::{Error, Result};
use crate::events::window::send_clipboard_update;
use crate::video::clipboard::{
    clipboard_mime_types, current_clipboard_callback, ClipboardDataCallback,
};

static COCOA_PASTEBOARD_DATA_PROVIDER: DeclaredClass = DeclaredClass::new();

/// The `Cocoa_PasteboardDataProvider` class (`@interface
/// Cocoa_PasteboardDataProvider : NSObject<NSPasteboardItemDataProvider>`),
/// its callback (`m_callback` with `m_userdata`) boxed in `_callback`.
fn cocoa_pasteboard_data_provider_class() -> Class {
    COCOA_PASTEBOARD_DATA_PROVIDER.get(|| {
        let mut b = ClassBuilder::new(c"Cocoa_PasteboardDataProvider", class!(NSObject))
            .expect("Cocoa_PasteboardDataProvider");
        b.add_protocol(c"NSPasteboardItemDataProvider");
        b.add_ivar::<*mut c_void>(c"_callback", c"^v");
        // SAFETY: the implementations have the signatures of the encodings.
        unsafe {
            b.add_method(
                sel!(pasteboard:item:provideDataForType:),
                imp!(provider_provide_data as extern "C" fn(Id, Sel, Id, Id, Id)),
                c"v@:@@@",
            );
            b.add_method(
                sel!(dealloc),
                imp!(provider_dealloc as extern "C" fn(Id, Sel)),
                c"v@:",
            );
        }
        b.register()
    })
}

/// The provider's callback variable.
fn provider_callback(this: Id) -> *mut *mut c_void {
    // SAFETY: a Cocoa_PasteboardDataProvider, which has the variable.
    unsafe { ivar_ptr::<*mut c_void>(this, cocoa_pasteboard_data_provider_class(), c"_callback") }
}

/// Translation of `-[Cocoa_PasteboardDataProvider initWith:userData:]`.
fn new_provider(callback: Option<ClipboardDataCallback>) -> StrongPtr {
    // SAFETY: NSObject's init, then the callback variable of the new
    // object (freed in dealloc).
    unsafe {
        let this: Id = msg_send![super::alloc(cocoa_pasteboard_data_provider_class()), init];
        if !this.is_null() {
            *provider_callback(this) = match callback {
                Some(cb) => Box::into_raw(Box::new(cb)) as *mut c_void,
                None => std::ptr::null_mut(),
            };
        }
        StrongPtr::from_owned(this)
    }
}

/// Translation of `-[Cocoa_PasteboardDataProvider pasteboard:item:provideDataForType:]`.
extern "C" fn provider_provide_data(this: Id, _sel: Sel, _pasteboard: Id, item: Id, ty: Id) {
    autoreleasepool(|| {
        // SAFETY: the callback the provider owns, and the UTI string.
        unsafe {
            let callback = *provider_callback(this);
            if callback.is_null() {
                return;
            }
            let callback = &*(callback as *const ClipboardDataCallback);
            let Some(mime_type) = CfRef::from_owned(UTTypeCopyPreferredTagWithClass(
                ty as CFStringRef,
                kUTTagClassMIMEType,
            )) else {
                return;
            };
            let mime_type =
                crate::core::apple::foundation::string_from_cf(mime_type.get()).unwrap_or_default();
            let Some(callback_data) = callback(&mime_type) else {
                return;
            };
            if callback_data.is_empty() {
                return;
            }
            let data: Id = msg_send![class!(NSData),
                dataWithBytes: callback_data.as_ptr() as *const c_void
                length: callback_data.len() as NSUInteger];
            let _: Bool = msg_send![item, setData: data forType: ty];
        }
    })
}

/// Free the callback (ARC's `-dealloc`).
extern "C" fn provider_dealloc(this: Id, _sel: Sel) {
    // SAFETY: the box made in new_provider(), freed once; then NSObject's
    // dealloc.
    unsafe {
        let p = provider_callback(this);
        if !(*p).is_null() {
            drop(Box::from_raw(*p as *mut ClipboardDataCallback));
            *p = std::ptr::null_mut();
        }
        let _: () = msg_send_super![this, cocoa_pasteboard_data_provider_class(), dealloc];
    }
}

/// The UniformTypeIdentifiers framework (`UTType`, macOS 11), loaded on
/// first use (upstream links it weakly).
fn uniform_type_identifiers() -> Option<usize> {
    static HANDLE: OnceLock<Option<usize>> = OnceLock::new();
    *HANDLE.get_or_init(|| {
        load_framework(
            c"/System/Library/Frameworks/UniformTypeIdentifiers.framework/UniformTypeIdentifiers",
        )
    })
}

/// `[NSPasteboard generalPasteboard]`.
fn general_pasteboard() -> Id {
    // SAFETY: NSPasteboard's class method.
    unsafe { msg_send![class!(NSPasteboard), generalPasteboard] }
}

/// The mime types (and UTIs) of the pasteboard's items. Translation of
/// `GetMimeTypes()`.
fn get_mime_types() -> Vec<String> {
    let mut new_mime_types = Vec::new();

    // SAFETY: the pasteboard's items and their NSString types; UTType's
    // class method.
    unsafe {
        let items: Id = msg_send![general_pasteboard(), pasteboardItems];
        for item in ns_array_items(items) {
            let types: Id = msg_send![item, types];
            for ty in ns_array_items(types) {
                if available!(macos(11, 0)) && uniform_type_identifiers().is_some() {
                    let uttype: Id = msg_send![class!(UTType), typeWithIdentifier: ty];
                    let mime_type: Id = if uttype.is_null() {
                        std::ptr::null_mut()
                    } else {
                        msg_send![uttype, preferredMIMEType]
                    };
                    if let Some(mime_type) =
                        crate::core::apple::foundation::string_from_ns(mime_type)
                    {
                        new_mime_types.push(mime_type);
                    }
                }
                new_mime_types.push(super::ns_to_string(ty));
            }
        }
    }
    new_mime_types
}

/// Tell SDL about pasteboard changes made by others. Translation of
/// `Cocoa_CheckClipboardUpdate()`.
pub(crate) fn check_clipboard_update(data: &Arc<VideoData>) {
    autoreleasepool(|| {
        // SAFETY: the general pasteboard.
        let count: NSInteger = unsafe { msg_send![general_pasteboard(), changeCount] };
        if count != data.state().clipboard_count {
            if count != 0 {
                let new_mime_types = get_mime_types();
                if !new_mime_types.is_empty() {
                    send_clipboard_update(false, new_mime_types);
                }
            }
            data.state().clipboard_count = count;
        }
    })
}

/// A UTI for a mime type (`UTTypeCreatePreferredIdentifierForTag()`).
fn uti_for_mime_type(mime_type: &str) -> Option<CfRef> {
    let mime_type = cf_string(mime_type);
    // SAFETY: CFStrings; the result is owned by the CfRef.
    unsafe {
        CfRef::from_owned(UTTypeCreatePreferredIdentifierForTag(
            kUTTagClassMIMEType,
            mime_type.get(),
            std::ptr::null(),
        ))
    }
}

/// Translation of `Cocoa_SetClipboardData()`.
pub(crate) fn set_clipboard_data(data: &Arc<VideoData>) -> Result<()> {
    autoreleasepool(|| {
        let pasteboard = general_pasteboard();
        let callback = current_clipboard_callback();
        let mime_types = clipboard_mime_types().unwrap_or_default();

        // SAFETY: the general pasteboard and the Foundation objects given to
        // it.
        unsafe {
            // SetClipboardText specialization so text is available after the app quits
            if let Some(callback) = callback.as_ref() {
                if mime_types.len() == 1 && mime_types[0].starts_with("text/plain;charset=utf-8") {
                    let types = [NSPasteboardTypeString];
                    let array: Id = msg_send![class!(NSArray),
                        arrayWithObjects: types.as_ptr() count: 1 as NSUInteger];
                    let _: NSInteger = msg_send![pasteboard, declareTypes: array owner: std::ptr::null_mut::<c_void>()];
                    // (upstream reads the text from the userdata of
                    // SDL_SetClipboardText()'s callback)
                    let text = callback(&mime_types[0]).unwrap_or_default();
                    let text =
                        String::from_utf8_lossy(text.split(|&b| b == 0).next().unwrap_or(&[]))
                            .into_owned();
                    let string = ns_string(&text);
                    let _: Bool = msg_send![pasteboard, setString: string.get() forType: NSPasteboardTypeString];
                    data.state().clipboard_count = msg_send![pasteboard, changeCount];
                    return Ok(());
                }
            }

            let new_item = StrongPtr::from_owned(msg_send![class!(NSPasteboardItem), new]);
            let uti_types = StrongPtr::from_owned(msg_send![class!(NSMutableArray), new]);
            let provider = new_provider(callback.clone());

            if callback.is_some() {
                for mime_type in &mime_types {
                    if let Some(uti_type) = uti_for_mime_type(mime_type) {
                        let _: () = msg_send![uti_types, addObject: uti_type.get() as Id];
                    }
                }
                let item_result: Bool =
                    msg_send![new_item, setDataProvider: provider.get() forTypes: uti_types.get()];
                if !item_result.is_true() {
                    return Err(Error::new("Unable to set clipboard item data"));
                }

                let _: NSInteger = msg_send![pasteboard, clearContents];
                let items = [new_item.get()];
                let array: Id = msg_send![class!(NSArray),
                    arrayWithObjects: items.as_ptr() count: 1 as NSUInteger];
                let write_result: Bool = msg_send![pasteboard, writeObjects: array];
                if !write_result.is_true() {
                    return Err(Error::new("Unable to set clipboard data"));
                }
            } else {
                let _: NSInteger = msg_send![pasteboard, clearContents];
            }
            data.state().clipboard_count = msg_send![pasteboard, changeCount];
        }
        Ok(())
    })
}

/// Translation of `IsMimeType()`.
fn is_mime_type(tag: &str) -> bool {
    if tag.contains('/') {
        // MIME types have slashes
        true
    } else if tag.contains('.') {
        // UTI identifiers have periods
        false
    } else {
        // Not sure, but it's not a UTI identifier
        true
    }
}

/// Translation of `GetUTIType()`.
fn get_uti_type(tag: &str) -> Option<CfRef> {
    if is_mime_type(tag) {
        uti_for_mime_type(tag)
    } else {
        Some(cf_string(tag))
    }
}

/// Translation of `Cocoa_GetClipboardData()`.
pub(crate) fn get_clipboard_data(mime_type: &str) -> Option<Vec<u8>> {
    autoreleasepool(|| {
        let pasteboard = general_pasteboard();
        // SAFETY: the pasteboard's items and the NSData they give.
        unsafe {
            let items: Id = msg_send![pasteboard, pasteboardItems];
            for item in ns_array_items(items) {
                let uti_type = get_uti_type(mime_type)?;
                let types = [uti_type.get() as Id];
                let array: Id = msg_send![class!(NSArray),
                    arrayWithObjects: types.as_ptr() count: 1 as NSUInteger];
                let available_type: Id = msg_send![pasteboard, availableTypeFromArray: array];
                let item_data: Id = msg_send![item, dataForType: available_type];
                drop(uti_type);
                if !item_data.is_null() {
                    let length: NSUInteger = msg_send![item_data, length];
                    let mut data = vec![0u8; length];
                    let _: () = msg_send![item_data, getBytes: data.as_mut_ptr() as *mut c_void
                                                         length: length];
                    // (upstream adds a zero Uint32 after the data; the
                    // clipboard front end terminates text itself)
                    return Some(data);
                }
            }
        }
        None
    })
}

/// Translation of `Cocoa_HasClipboardData()`.
pub(crate) fn has_clipboard_data(mime_type: &str) -> bool {
    let mut result = false;
    autoreleasepool(|| {
        let pasteboard = general_pasteboard();
        if let Some(uti_type) = get_uti_type(mime_type) {
            let types = [uti_type.get() as Id];
            // SAFETY: the general pasteboard and an array of a UTI string.
            unsafe {
                let array: Id = msg_send![class!(NSArray),
                    arrayWithObjects: types.as_ptr() count: 1 as NSUInteger];
                let can: Bool = msg_send![pasteboard, canReadItemWithDataConformingToTypes: array];
                if can.is_true() {
                    result = true;
                }
            }
        }
    });
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mime_types_and_utis() {
        assert!(is_mime_type("text/plain;charset=utf-8"));
        assert!(!is_mime_type("public.utf8-plain-text"));
        assert!(is_mime_type("TEXT"));
        let uti = get_uti_type("text/plain;charset=utf-8").unwrap();
        // SAFETY: a CFString.
        let uti = unsafe { crate::core::apple::foundation::string_from_cf(uti.get()) };
        assert_eq!(uti.as_deref(), Some("public.utf8-plain-text"));
        let uti = get_uti_type("public.png").unwrap();
        // SAFETY: a CFString.
        let uti = unsafe { crate::core::apple::foundation::string_from_cf(uti.get()) };
        assert_eq!(uti.as_deref(), Some("public.png"));
    }
}
