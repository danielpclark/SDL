// Foundation and CoreFoundation, declared by hand for the Apple backends.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! The Foundation types the backends pass by value (`NSPoint`, `NSRect`,
//! `NSRange`...), `NSString` conversions, and the CoreFoundation objects
//! upstream's C backends use (`CFStringRef`, `CFArrayRef`, `CFNumberRef`,
//! `CFDictionaryRef`, `CFRunLoopRef`), owned through [`CfRef`].

use std::ffi::{c_char, c_void, CStr};
use std::ptr;

use super::objc::{class, msg_send, Bool, Id, NSInteger, NSUInteger, StrongPtr};

/// `CGFloat` (64-bit platforms).
pub(crate) type CGFloat = f64;

/// `NSPoint` / `CGPoint`.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct NSPoint {
    pub(crate) x: CGFloat,
    pub(crate) y: CGFloat,
}

impl NSPoint {
    pub(crate) const fn new(x: CGFloat, y: CGFloat) -> NSPoint {
        NSPoint { x, y }
    }
}

/// `NSSize` / `CGSize`.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct NSSize {
    pub(crate) width: CGFloat,
    pub(crate) height: CGFloat,
}

impl NSSize {
    pub(crate) const fn new(width: CGFloat, height: CGFloat) -> NSSize {
        NSSize { width, height }
    }
}

/// `NSRect` / `CGRect`.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct NSRect {
    pub(crate) origin: NSPoint,
    pub(crate) size: NSSize,
}

impl NSRect {
    /// `NSMakeRect()`.
    pub(crate) const fn new(x: CGFloat, y: CGFloat, w: CGFloat, h: CGFloat) -> NSRect {
        NSRect {
            origin: NSPoint { x, y },
            size: NSSize {
                width: w,
                height: h,
            },
        }
    }

    /// `NSMinX()`.
    pub(crate) fn min_x(&self) -> CGFloat {
        self.origin.x
    }
    /// `NSMinY()`.
    pub(crate) fn min_y(&self) -> CGFloat {
        self.origin.y
    }
    /// `NSMaxX()`.
    pub(crate) fn max_x(&self) -> CGFloat {
        self.origin.x + self.size.width
    }
    /// `NSMaxY()`.
    pub(crate) fn max_y(&self) -> CGFloat {
        self.origin.y + self.size.height
    }
    /// `NSWidth()`.
    pub(crate) fn width(&self) -> CGFloat {
        self.size.width
    }
    /// `NSHeight()`.
    pub(crate) fn height(&self) -> CGFloat {
        self.size.height
    }

    /// `NSPointInRect()`: the minimum edges are inside, the maximum ones
    /// outside (for an unflipped coordinate system).
    pub(crate) fn contains(&self, p: NSPoint) -> bool {
        p.x >= self.min_x() && p.y >= self.min_y() && p.x < self.max_x() && p.y < self.max_y()
    }

    /// `NSIsEmptyRect()`.
    pub(crate) fn is_empty(&self) -> bool {
        !(self.size.width > 0.0 && self.size.height > 0.0)
    }
}

/// `CGPoint`.
pub(crate) type CGPoint = NSPoint;
/// `CGSize`.
pub(crate) type CGSize = NSSize;
/// `CGRect`.
pub(crate) type CGRect = NSRect;

/// `NSRange` / `CFRange` (with `NSUInteger` fields).
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct NSRange {
    pub(crate) location: NSUInteger,
    pub(crate) length: NSUInteger,
}

impl NSRange {
    /// `NSMakeRange()`.
    pub(crate) const fn new(location: NSUInteger, length: NSUInteger) -> NSRange {
        NSRange { location, length }
    }
}

/// `NSNotFound`.
pub(crate) const NS_NOT_FOUND: isize = isize::MAX;

/// `NSUTF8StringEncoding`.
pub(crate) const NS_UTF8_STRING_ENCODING: NSUInteger = 4;

/// `NSTimeInterval`.
pub(crate) type NSTimeInterval = f64;

/// An `NSString` of `s` (`[[NSString alloc] initWithBytes:length:encoding:]`,
/// owned).
pub(crate) fn ns_string(s: &str) -> StrongPtr {
    // SAFETY: NSString exists in every process linking Foundation; the
    // bytes are valid UTF-8 of the given length.
    unsafe {
        let obj: Id = msg_send![class!(NSString), alloc];
        let obj: Id = msg_send![obj, initWithBytes: s.as_ptr() as *const c_void
                                      length: s.len() as NSUInteger
                                      encoding: NS_UTF8_STRING_ENCODING];
        StrongPtr::from_owned(obj)
    }
}

/// The UTF-8 contents of an `NSString` (`[str UTF8String]`); `None` for nil.
///
/// # Safety
///
/// `s` must be nil or a valid `NSString`.
pub(crate) unsafe fn string_from_ns(s: Id) -> Option<String> {
    if s.is_null() {
        return None;
    }
    // SAFETY: the caller's contract; UTF8String's buffer lives at least as
    // long as the autorelease pool, and is copied at once.
    unsafe {
        let p: *const c_char = msg_send![s, UTF8String];
        if p.is_null() {
            return None;
        }
        Some(CStr::from_ptr(p).to_string_lossy().into_owned())
    }
}

/// `[array count]`.
///
/// # Safety
///
/// `array` must be nil or a valid `NSArray`.
pub(crate) unsafe fn ns_array_count(array: Id) -> NSUInteger {
    // SAFETY: the caller's contract.
    unsafe { msg_send![array, count] }
}

/// `[array objectAtIndex:i]`.
///
/// # Safety
///
/// `array` must be a valid `NSArray` with more than `i` elements.
pub(crate) unsafe fn ns_array_get(array: Id, i: NSUInteger) -> Id {
    // SAFETY: the caller's contract.
    unsafe { msg_send![array, objectAtIndex: i] }
}

/// The elements of an `NSArray` (unretained).
///
/// # Safety
///
/// `array` must be nil or a valid `NSArray`.
pub(crate) unsafe fn ns_array_items(array: Id) -> Vec<Id> {
    // SAFETY: the caller's contract.
    unsafe {
        let n = ns_array_count(array);
        (0..n).map(|i| ns_array_get(array, i)).collect()
    }
}

/// `[NSNumber numberWithBool:b]` (autoreleased).
pub(crate) fn ns_number_bool(b: bool) -> Id {
    // SAFETY: a class method of NSNumber.
    unsafe { msg_send![class!(NSNumber), numberWithBool: Bool::from(b)] }
}

/// `[NSThread isMainThread]` (`pthread_main_np()`).
pub(crate) fn is_main_thread() -> bool {
    // SAFETY: no arguments; answers for the calling thread.
    unsafe { libc::pthread_main_np() != 0 }
}

// ---------------------------------------------------------------------------
// CoreFoundation

/// `CFTypeRef`.
pub(crate) type CFTypeRef = *const c_void;
/// `CFStringRef`.
pub(crate) type CFStringRef = *const c_void;
/// `CFMutableStringRef`.
pub(crate) type CFMutableStringRef = *mut c_void;
/// `CFArrayRef`.
pub(crate) type CFArrayRef = *const c_void;
/// `CFMutableArrayRef`.
pub(crate) type CFMutableArrayRef = *mut c_void;
/// `CFDictionaryRef`.
pub(crate) type CFDictionaryRef = *const c_void;
/// `CFMutableDictionaryRef`.
pub(crate) type CFMutableDictionaryRef = *mut c_void;
/// `CFNumberRef`.
pub(crate) type CFNumberRef = *const c_void;
/// `CFBooleanRef`.
pub(crate) type CFBooleanRef = *const c_void;
/// `CFDataRef`.
pub(crate) type CFDataRef = *const c_void;
/// `CFURLRef`.
pub(crate) type CFURLRef = *const c_void;
/// `CFUUIDRef`.
pub(crate) type CFUUIDRef = *const c_void;
/// `CFRunLoopRef`.
pub(crate) type CFRunLoopRef = *mut c_void;
/// `CFRunLoopSourceRef`.
pub(crate) type CFRunLoopSourceRef = *mut c_void;
/// `CFRunLoopTimerRef`.
pub(crate) type CFRunLoopTimerRef = *mut c_void;
/// `CFRunLoopObserverRef`.
pub(crate) type CFRunLoopObserverRef = *mut c_void;
/// `CFAllocatorRef`.
pub(crate) type CFAllocatorRef = *const c_void;
/// `CFBundleRef`.
pub(crate) type CFBundleRef = *mut c_void;
/// `CFLocaleRef`.
pub(crate) type CFLocaleRef = *const c_void;
/// `CFTypeID`.
pub(crate) type CFTypeID = usize;
/// `CFIndex`.
pub(crate) type CFIndex = isize;
/// `CFOptionFlags`.
pub(crate) type CFOptionFlags = usize;
/// `CFStringEncoding`.
pub(crate) type CFStringEncoding = u32;
/// `CFNumberType`.
pub(crate) type CFNumberType = CFIndex;
/// `CFTimeInterval`.
pub(crate) type CFTimeInterval = f64;
/// `CFAbsoluteTime`.
pub(crate) type CFAbsoluteTime = f64;
/// `Boolean` (an unsigned char).
pub(crate) type Boolean = u8;

/// `CFRange`.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct CFRange {
    pub(crate) location: CFIndex,
    pub(crate) length: CFIndex,
}

/// `kCFStringEncodingUTF8`.
pub(crate) const K_CF_STRING_ENCODING_UTF8: CFStringEncoding = 0x0800_0100;
/// `kCFStringEncodingASCII`.
pub(crate) const K_CF_STRING_ENCODING_ASCII: CFStringEncoding = 0x0600;

/// `kCFNumberSInt32Type`.
pub(crate) const K_CF_NUMBER_SINT32_TYPE: CFNumberType = 3;
/// `kCFNumberSInt64Type`.
pub(crate) const K_CF_NUMBER_SINT64_TYPE: CFNumberType = 4;
/// `kCFNumberFloat64Type`.
pub(crate) const K_CF_NUMBER_FLOAT64_TYPE: CFNumberType = 6;
/// `kCFNumberIntType`.
pub(crate) const K_CF_NUMBER_INT_TYPE: CFNumberType = 9;
/// `kCFNumberLongType`.
pub(crate) const K_CF_NUMBER_LONG_TYPE: CFNumberType = 10;
/// `kCFNumberDoubleType`.
pub(crate) const K_CF_NUMBER_DOUBLE_TYPE: CFNumberType = 13;

/// `kCFRunLoopRunFinished`.
pub(crate) const K_CF_RUN_LOOP_RUN_FINISHED: i32 = 1;
/// `kCFRunLoopRunStopped`.
pub(crate) const K_CF_RUN_LOOP_RUN_STOPPED: i32 = 2;
/// `kCFRunLoopRunTimedOut`.
pub(crate) const K_CF_RUN_LOOP_RUN_TIMED_OUT: i32 = 3;
/// `kCFRunLoopRunHandledSource`.
pub(crate) const K_CF_RUN_LOOP_RUN_HANDLED_SOURCE: i32 = 4;

/// `CFArrayCallBacks` (`kCFTypeArrayCallBacks` is what callers pass).
#[repr(C)]
pub(crate) struct CFArrayCallBacks {
    _private: [u8; 0],
}

/// `CFDictionaryKeyCallBacks`.
#[repr(C)]
pub(crate) struct CFDictionaryKeyCallBacks {
    _private: [u8; 0],
}

/// `CFDictionaryValueCallBacks`.
#[repr(C)]
pub(crate) struct CFDictionaryValueCallBacks {
    _private: [u8; 0],
}

#[link(name = "CoreFoundation", kind = "framework")]
unsafe extern "C" {
    pub(crate) static kCFAllocatorDefault: CFAllocatorRef;
    pub(crate) static kCFAllocatorNull: CFAllocatorRef;
    pub(crate) static kCFBooleanTrue: CFBooleanRef;
    pub(crate) static kCFBooleanFalse: CFBooleanRef;
    pub(crate) static kCFTypeArrayCallBacks: CFArrayCallBacks;
    pub(crate) static kCFTypeDictionaryKeyCallBacks: CFDictionaryKeyCallBacks;
    pub(crate) static kCFTypeDictionaryValueCallBacks: CFDictionaryValueCallBacks;
    pub(crate) static kCFRunLoopDefaultMode: CFStringRef;
    pub(crate) static kCFRunLoopCommonModes: CFStringRef;

    pub(crate) fn CFRetain(cf: CFTypeRef) -> CFTypeRef;
    pub(crate) fn CFRelease(cf: CFTypeRef);
    pub(crate) fn CFGetTypeID(cf: CFTypeRef) -> CFTypeID;
    pub(crate) fn CFEqual(cf1: CFTypeRef, cf2: CFTypeRef) -> Boolean;

    pub(crate) fn CFStringGetTypeID() -> CFTypeID;
    pub(crate) fn CFStringCreateWithBytes(
        alloc: CFAllocatorRef,
        bytes: *const u8,
        num_bytes: CFIndex,
        encoding: CFStringEncoding,
        is_external_representation: Boolean,
    ) -> CFStringRef;
    pub(crate) fn CFStringCreateWithCString(
        alloc: CFAllocatorRef,
        c_str: *const c_char,
        encoding: CFStringEncoding,
    ) -> CFStringRef;
    pub(crate) fn CFStringGetLength(s: CFStringRef) -> CFIndex;
    pub(crate) fn CFStringGetMaximumSizeForEncoding(
        length: CFIndex,
        encoding: CFStringEncoding,
    ) -> CFIndex;
    pub(crate) fn CFStringGetCString(
        s: CFStringRef,
        buffer: *mut c_char,
        buffer_size: CFIndex,
        encoding: CFStringEncoding,
    ) -> Boolean;
    pub(crate) fn CFStringGetCStringPtr(
        s: CFStringRef,
        encoding: CFStringEncoding,
    ) -> *const c_char;

    pub(crate) fn CFNumberGetTypeID() -> CFTypeID;
    pub(crate) fn CFNumberCreate(
        alloc: CFAllocatorRef,
        the_type: CFNumberType,
        value_ptr: *const c_void,
    ) -> CFNumberRef;
    pub(crate) fn CFNumberGetValue(
        number: CFNumberRef,
        the_type: CFNumberType,
        value_ptr: *mut c_void,
    ) -> Boolean;

    pub(crate) fn CFBooleanGetTypeID() -> CFTypeID;
    pub(crate) fn CFBooleanGetValue(boolean: CFBooleanRef) -> Boolean;

    pub(crate) fn CFArrayGetTypeID() -> CFTypeID;
    pub(crate) fn CFArrayCreate(
        alloc: CFAllocatorRef,
        values: *const *const c_void,
        num_values: CFIndex,
        callbacks: *const CFArrayCallBacks,
    ) -> CFArrayRef;
    pub(crate) fn CFArrayCreateMutable(
        alloc: CFAllocatorRef,
        capacity: CFIndex,
        callbacks: *const CFArrayCallBacks,
    ) -> CFMutableArrayRef;
    pub(crate) fn CFArrayAppendValue(array: CFMutableArrayRef, value: *const c_void);
    pub(crate) fn CFArrayGetCount(array: CFArrayRef) -> CFIndex;
    pub(crate) fn CFArrayGetValueAtIndex(array: CFArrayRef, idx: CFIndex) -> *const c_void;

    pub(crate) fn CFDictionaryGetTypeID() -> CFTypeID;
    pub(crate) fn CFDictionaryCreate(
        alloc: CFAllocatorRef,
        keys: *const *const c_void,
        values: *const *const c_void,
        num_values: CFIndex,
        key_callbacks: *const CFDictionaryKeyCallBacks,
        value_callbacks: *const CFDictionaryValueCallBacks,
    ) -> CFDictionaryRef;
    pub(crate) fn CFDictionaryCreateMutable(
        alloc: CFAllocatorRef,
        capacity: CFIndex,
        key_callbacks: *const CFDictionaryKeyCallBacks,
        value_callbacks: *const CFDictionaryValueCallBacks,
    ) -> CFMutableDictionaryRef;
    pub(crate) fn CFDictionarySetValue(
        dict: CFMutableDictionaryRef,
        key: *const c_void,
        value: *const c_void,
    );
    pub(crate) fn CFDictionaryGetValue(dict: CFDictionaryRef, key: *const c_void) -> *const c_void;
    pub(crate) fn CFDictionaryGetValueIfPresent(
        dict: CFDictionaryRef,
        key: *const c_void,
        value: *mut *const c_void,
    ) -> Boolean;
    pub(crate) fn CFDictionaryGetCount(dict: CFDictionaryRef) -> CFIndex;

    pub(crate) fn CFDataGetTypeID() -> CFTypeID;
    pub(crate) fn CFDataGetLength(data: CFDataRef) -> CFIndex;
    pub(crate) fn CFDataGetBytePtr(data: CFDataRef) -> *const u8;

    pub(crate) fn CFRunLoopGetCurrent() -> CFRunLoopRef;
    pub(crate) fn CFRunLoopGetMain() -> CFRunLoopRef;
    pub(crate) fn CFRunLoopRun();
    pub(crate) fn CFRunLoopRunInMode(
        mode: CFStringRef,
        seconds: CFTimeInterval,
        return_after_source_handled: Boolean,
    ) -> i32;
    pub(crate) fn CFRunLoopStop(rl: CFRunLoopRef);
    pub(crate) fn CFRunLoopWakeUp(rl: CFRunLoopRef);
    pub(crate) fn CFRunLoopAddSource(
        rl: CFRunLoopRef,
        source: CFRunLoopSourceRef,
        mode: CFStringRef,
    );
    pub(crate) fn CFRunLoopRemoveSource(
        rl: CFRunLoopRef,
        source: CFRunLoopSourceRef,
        mode: CFStringRef,
    );
    pub(crate) fn CFRunLoopSourceInvalidate(source: CFRunLoopSourceRef);

    pub(crate) fn CFURLCreateWithBytes(
        allocator: CFAllocatorRef,
        url_bytes: *const u8,
        length: CFIndex,
        encoding: CFStringEncoding,
        base_url: CFURLRef,
    ) -> CFURLRef;
    pub(crate) fn CFURLGetFileSystemRepresentation(
        url: CFURLRef,
        resolve_against_base: Boolean,
        buffer: *mut u8,
        max_buf_len: CFIndex,
    ) -> Boolean;

    pub(crate) fn CFBundleGetMainBundle() -> CFBundleRef;
    pub(crate) fn CFBundleGetValueForInfoDictionaryKey(
        bundle: CFBundleRef,
        key: CFStringRef,
    ) -> CFTypeRef;

    pub(crate) fn CFAbsoluteTimeGetCurrent() -> CFAbsoluteTime;
}

/// An owned CoreFoundation object (from a `Create`/`Copy` function, or
/// retained), released on drop. Never null.
pub(crate) struct CfRef(CFTypeRef);

// SAFETY: CoreFoundation objects are reference counted thread-safely; the
// immutable ones may be read from any thread, and the backends keep the
// mutable ones to the threads upstream uses them on.
unsafe impl Send for CfRef {}
// SAFETY: as above.
unsafe impl Sync for CfRef {}

impl CfRef {
    /// Take ownership of a `+1` reference (the Create rule); `None` for
    /// null.
    ///
    /// # Safety
    ///
    /// `cf` must be null or a CF object the caller owns a reference to.
    pub(crate) unsafe fn from_owned(cf: CFTypeRef) -> Option<CfRef> {
        if cf.is_null() {
            None
        } else {
            Some(CfRef(cf))
        }
    }

    /// Retain a `+0` reference (the Get rule); `None` for null.
    ///
    /// # Safety
    ///
    /// `cf` must be null or a valid CF object.
    pub(crate) unsafe fn retain(cf: CFTypeRef) -> Option<CfRef> {
        if cf.is_null() {
            None
        } else {
            // SAFETY: the caller's contract.
            Some(CfRef(unsafe { CFRetain(cf) }))
        }
    }

    /// The object, still owned by `self`.
    pub(crate) fn get(&self) -> CFTypeRef {
        self.0
    }

    /// Give up ownership without releasing.
    pub(crate) fn into_raw(self) -> CFTypeRef {
        let cf = self.0;
        std::mem::forget(self);
        cf
    }

    /// Whether the object is of `type_id` (`CFGetTypeID(cf) == type_id`).
    pub(crate) fn is_type(&self, type_id: CFTypeID) -> bool {
        // SAFETY: a valid object.
        unsafe { CFGetTypeID(self.0) == type_id }
    }
}

impl Clone for CfRef {
    fn clone(&self) -> CfRef {
        // SAFETY: we hold a reference.
        CfRef(unsafe { CFRetain(self.0) })
    }
}

impl Drop for CfRef {
    fn drop(&mut self) {
        // SAFETY: the reference we own.
        unsafe { CFRelease(self.0) }
    }
}

impl std::fmt::Debug for CfRef {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "CfRef({:p})", self.0)
    }
}

/// A `CFString` of `s` (owned).
pub(crate) fn cf_string(s: &str) -> CfRef {
    // SAFETY: valid UTF-8 bytes of the given length.
    unsafe {
        CfRef::from_owned(CFStringCreateWithBytes(
            kCFAllocatorDefault,
            s.as_ptr(),
            s.len() as CFIndex,
            K_CF_STRING_ENCODING_UTF8,
            0,
        ))
    }
    .expect("CFStringCreateWithBytes")
}

/// The UTF-8 contents of a `CFString` (`CFStringGetCStringPtr()`, else
/// `CFStringGetCString()` into a buffer of the maximum size), `None` if it
/// can't be converted or is null.
///
/// # Safety
///
/// `s` must be null or a valid `CFString`.
pub(crate) unsafe fn string_from_cf(s: CFStringRef) -> Option<String> {
    if s.is_null() {
        return None;
    }
    // SAFETY: the caller's contract; the buffer is as large as the size
    // CFStringGetCString() is told.
    unsafe {
        let p = CFStringGetCStringPtr(s, K_CF_STRING_ENCODING_UTF8);
        if !p.is_null() {
            return Some(CStr::from_ptr(p).to_string_lossy().into_owned());
        }
        let len = CFStringGetLength(s);
        let size = CFStringGetMaximumSizeForEncoding(len, K_CF_STRING_ENCODING_UTF8) + 1;
        let mut buf = vec![0u8; size as usize];
        if CFStringGetCString(
            s,
            buf.as_mut_ptr() as *mut c_char,
            size,
            K_CF_STRING_ENCODING_UTF8,
        ) == 0
        {
            return None;
        }
        Some(
            CStr::from_ptr(buf.as_ptr() as *const c_char)
                .to_string_lossy()
                .into_owned(),
        )
    }
}

/// A `CFNumber` of an `i32` (`kCFNumberSInt32Type`, owned).
pub(crate) fn cf_number_i32(v: i32) -> CfRef {
    // SAFETY: a pointer to a value of the type named.
    unsafe {
        CfRef::from_owned(CFNumberCreate(
            kCFAllocatorDefault,
            K_CF_NUMBER_SINT32_TYPE,
            &v as *const i32 as *const c_void,
        ))
    }
    .expect("CFNumberCreate")
}

/// The value of a `CFNumber` as `i32` (`CFNumberGetValue(kCFNumberSInt32Type)`),
/// `None` if `n` isn't a number.
///
/// # Safety
///
/// `n` must be null or a valid CF object.
pub(crate) unsafe fn cf_number_get_i32(n: CFTypeRef) -> Option<i32> {
    // SAFETY: the caller's contract; the out pointer is of the type named.
    unsafe {
        if n.is_null() || CFGetTypeID(n) != CFNumberGetTypeID() {
            return None;
        }
        let mut v = 0i32;
        CFNumberGetValue(
            n,
            K_CF_NUMBER_SINT32_TYPE,
            &mut v as *mut i32 as *mut c_void,
        );
        Some(v)
    }
}

/// The value of a `CFNumber` as `i64` (`kCFNumberSInt64Type`).
///
/// # Safety
///
/// `n` must be null or a valid CF object.
pub(crate) unsafe fn cf_number_get_i64(n: CFTypeRef) -> Option<i64> {
    // SAFETY: as above.
    unsafe {
        if n.is_null() || CFGetTypeID(n) != CFNumberGetTypeID() {
            return None;
        }
        let mut v = 0i64;
        CFNumberGetValue(
            n,
            K_CF_NUMBER_SINT64_TYPE,
            &mut v as *mut i64 as *mut c_void,
        );
        Some(v)
    }
}

/// The value of a `CFNumber` as `f64` (`kCFNumberFloat64Type`).
///
/// # Safety
///
/// `n` must be null or a valid CF object.
pub(crate) unsafe fn cf_number_get_f64(n: CFTypeRef) -> Option<f64> {
    // SAFETY: as above.
    unsafe {
        if n.is_null() || CFGetTypeID(n) != CFNumberGetTypeID() {
            return None;
        }
        let mut v = 0f64;
        CFNumberGetValue(
            n,
            K_CF_NUMBER_FLOAT64_TYPE,
            &mut v as *mut f64 as *mut c_void,
        );
        Some(v)
    }
}

/// `CFDictionaryGetValue()` with a string key, null when absent.
///
/// # Safety
///
/// `dict` must be a valid `CFDictionary`.
pub(crate) unsafe fn cf_dictionary_get(dict: CFDictionaryRef, key: &str) -> CFTypeRef {
    let key = cf_string(key);
    // SAFETY: the caller's contract.
    unsafe { CFDictionaryGetValue(dict, key.get()) }
}

/// A `CFDictionary` with CF-type callbacks from key/value pairs (owned).
pub(crate) fn cf_dictionary(pairs: &[(CFTypeRef, CFTypeRef)]) -> CfRef {
    let keys: Vec<*const c_void> = pairs.iter().map(|p| p.0).collect();
    let values: Vec<*const c_void> = pairs.iter().map(|p| p.1).collect();
    // SAFETY: two arrays of the given length; the callbacks retain them.
    unsafe {
        CfRef::from_owned(CFDictionaryCreate(
            kCFAllocatorDefault,
            keys.as_ptr(),
            values.as_ptr(),
            pairs.len() as CFIndex,
            &kCFTypeDictionaryKeyCallBacks,
            &kCFTypeDictionaryValueCallBacks,
        ))
    }
    .expect("CFDictionaryCreate")
}

/// Null for an absent CF object.
pub(crate) const CF_NULL: CFTypeRef = ptr::null();

/// `NSOperatingSystemVersion`.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct NSOperatingSystemVersion {
    pub(crate) major: NSInteger,
    pub(crate) minor: NSInteger,
    pub(crate) patch: NSInteger,
}

/// Whether the system is at least `major.minor.patch`
/// (`-[NSProcessInfo isOperatingSystemAtLeastVersion:]`, what `@available()`
/// amounts to), in the platform's own numbering.
pub(crate) fn os_version_at_least(major: NSInteger, minor: NSInteger, patch: NSInteger) -> bool {
    let version = NSOperatingSystemVersion {
        major,
        minor,
        patch,
    };
    // SAFETY: NSProcessInfo's class method, and an instance method taking
    // the version structure by value.
    unsafe {
        let info: Id = msg_send![class!(NSProcessInfo), processInfo];
        let r: Bool = msg_send![info, isOperatingSystemAtLeastVersion: version];
        r.is_true()
    }
}

/// The platforms an [`available!`] check names.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum ApplePlatform {
    /// `macOS` (`target_os = "macos"`).
    MacOs,
    /// `iOS` (`target_os = "ios"`).
    Ios,
    /// `tvOS` (`target_os = "tvos"`).
    TvOs,
    /// `visionOS` (`target_os = "visionos"`).
    VisionOs,
}

impl ApplePlatform {
    /// The platform built for (`None`: one no check names, watchOS).
    pub(crate) const CURRENT: Option<ApplePlatform> = if cfg!(target_os = "macos") {
        Some(ApplePlatform::MacOs)
    } else if cfg!(target_os = "ios") {
        Some(ApplePlatform::Ios)
    } else if cfg!(target_os = "tvos") {
        Some(ApplePlatform::TvOs)
    } else if cfg!(target_os = "visionos") {
        Some(ApplePlatform::VisionOs)
    } else {
        None
    };
}

/// `@available()` with these minimum versions (`[major, minor, patch]`):
/// the current platform's is checked at run time; a platform the list
/// doesn't name passes (the `*`). [`available!`] builds the list.
pub(crate) fn available_on(versions: &[(ApplePlatform, [NSInteger; 3])]) -> bool {
    match versions
        .iter()
        .find(|(platform, _)| Some(*platform) == ApplePlatform::CURRENT)
    {
        Some(&(_, [major, minor, patch])) => os_version_at_least(major, minor, patch),
        None => true,
    }
}

/// `@available(macOS 11.0, iOS 14.0, tvOS 14.0, *)` is
/// `available!(macos(11, 0), ios(14, 0), tvos(14, 0))`: true if the system
/// is at least the version given for the platform built for (`macos`,
/// `ios`, `tvos` or `visionos`; one to three numbers), or if that platform
/// isn't named (the `*`, which every `@available()` ends with). Checked at
/// run time, as upstream's are when it is built with the newest SDK.
macro_rules! available {
    (@platform macos) => {
        $crate::core::apple::foundation::ApplePlatform::MacOs
    };
    (@platform ios) => {
        $crate::core::apple::foundation::ApplePlatform::Ios
    };
    (@platform tvos) => {
        $crate::core::apple::foundation::ApplePlatform::TvOs
    };
    (@platform visionos) => {
        $crate::core::apple::foundation::ApplePlatform::VisionOs
    };
    (@version $major:expr) => {
        [$major, 0, 0]
    };
    (@version $major:expr, $minor:expr) => {
        [$major, $minor, 0]
    };
    (@version $major:expr, $minor:expr, $patch:expr) => {
        [$major, $minor, $patch]
    };
    ($($platform:ident ( $($version:expr),+ )),+ $(,)?) => {
        $crate::core::apple::foundation::available_on(&[$((
            $crate::core::apple::foundation::available!(@platform $platform),
            $crate::core::apple::foundation::available!(@version $($version),+),
        )),+])
    };
}
pub(crate) use available;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cf_strings_numbers_dictionaries() {
        let s = cf_string("Ünïcode ✓");
        assert_eq!(
            unsafe { string_from_cf(s.get()) }.as_deref(),
            Some("Ünïcode ✓")
        );
        assert!(s.is_type(unsafe { CFStringGetTypeID() }));
        let n = cf_number_i32(-42);
        assert_eq!(unsafe { cf_number_get_i32(n.get()) }, Some(-42));
        assert_eq!(unsafe { cf_number_get_i64(n.get()) }, Some(-42));
        assert_eq!(unsafe { cf_number_get_i32(s.get()) }, None);
        let k = cf_string("answer");
        let d = cf_dictionary(&[(k.get(), n.get())]);
        assert_eq!(
            unsafe { cf_number_get_i32(cf_dictionary_get(d.get(), "answer")) },
            Some(-42)
        );
        assert!(unsafe { cf_dictionary_get(d.get(), "missing") }.is_null());
    }

    /// `available!()`: the current platform's version is checked, the
    /// others pass.
    #[test]
    fn availability() {
        assert!(os_version_at_least(1, 0, 0));
        assert!(!os_version_at_least(1000, 0, 0));
        assert!(available!(macos(1), ios(1, 0), tvos(1, 0, 0), visionos(1)));
        assert!(!available!(
            macos(1000, 0),
            ios(1000, 0),
            tvos(1000),
            visionos(1000)
        ));
        // (another platform's version isn't checked: `*`)
        let other = if cfg!(target_os = "macos") {
            available!(ios(1000, 0), tvos(1000, 0))
        } else {
            available!(macos(1000, 0))
        };
        assert!(other);
        assert_eq!(
            ApplePlatform::CURRENT.is_some(),
            !cfg!(target_os = "watchos")
        );
        // (the minor and patch versions count)
        let v = unsafe {
            let info: Id = msg_send![class!(NSProcessInfo), processInfo];
            let v: NSOperatingSystemVersion = msg_send![info, operatingSystemVersion];
            v
        };
        assert!(os_version_at_least(v.major, v.minor, v.patch));
        assert!(!os_version_at_least(v.major, v.minor, v.patch + 1));
        assert!(!os_version_at_least(v.major, v.minor + 1, 0));
    }

    #[test]
    fn geometry() {
        let r = NSRect::new(10.0, 20.0, 30.0, 40.0);
        assert_eq!((r.max_x(), r.max_y()), (40.0, 60.0));
        assert!(r.contains(NSPoint::new(10.0, 20.0)));
        assert!(!r.contains(NSPoint::new(40.0, 20.0)));
        assert!(NSRect::new(0.0, 0.0, 0.0, 5.0).is_empty());
        assert_eq!(size_of::<NSRect>(), 32);
    }
}
