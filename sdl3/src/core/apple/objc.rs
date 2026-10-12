// The Objective-C runtime, declared by hand for the Apple backends.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! What the compiler does for upstream's Objective-C, done through the
//! runtime's C API (`<objc/runtime.h>`, `<objc/message.h>`):
//!
//! * `[obj selector:arg]` is [`msg_send!`]`[obj, selector: arg]`, an
//!   `objc_msgSend()` cast to the method's signature (the return type is
//!   inferred, so a method returning `void` is `let _: () = msg_send![..]`);
//!   `[super selector]` is [`msg_send_super!`];
//! * `@selector(a:b:)` is [`sel!`]`(a:b:)` and a class name is
//!   [`class!`]`(NSWindow)`, both looked up once per call site;
//! * `@interface Foo : Bar` with its methods and instance variables is a
//!   [`ClassBuilder`], registered at run time;
//! * `@autoreleasepool { }` is [`autoreleasepool`];
//! * an owned (`+1`, from `alloc`/`new`/`copy`) object is a [`StrongPtr`],
//!   released on drop, as ARC would.
//!
//! Objective-C exceptions can't be caught here (upstream's few `@try`
//! blocks say so where they are translated); one that reaches Rust aborts.

use std::ffi::{c_char, c_void, CStr};
use std::fmt;
use std::marker::PhantomData;
use std::ptr;
use std::sync::atomic::{AtomicPtr, Ordering};

/// An Objective-C object (`struct objc_object`).
#[repr(C)]
pub(crate) struct Object {
    _private: [u8; 0],
    _marker: PhantomData<(*mut u8, std::marker::PhantomPinned)>,
}

/// `id`.
pub(crate) type Id = *mut Object;

/// `nil`.
pub(crate) const NIL: Id = ptr::null_mut();

/// A class (`struct objc_class`).
#[repr(C)]
pub(crate) struct ObjcClass {
    _private: [u8; 0],
    _marker: PhantomData<(*mut u8, std::marker::PhantomPinned)>,
}

/// `Class`.
pub(crate) type Class = *const ObjcClass;

/// A method selector (`SEL`).
#[repr(transparent)]
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) struct Sel(*const c_void);

// SAFETY: selectors are interned, immutable runtime data.
unsafe impl Send for Sel {}
// SAFETY: as above.
unsafe impl Sync for Sel {}

impl Sel {
    /// `sel_registerName()`.
    pub(crate) fn register(name: &CStr) -> Sel {
        // SAFETY: a NUL-terminated name; the runtime interns it.
        unsafe { sel_registerName(name.as_ptr()) }
    }

    /// `sel_getName()`.
    pub(crate) fn name(self) -> &'static str {
        // SAFETY: the runtime keeps selector names for the process' life.
        unsafe { CStr::from_ptr(sel_getName(self)) }
            .to_str()
            .unwrap_or("")
    }

    /// The null selector (`NULL`), for comparisons.
    pub(crate) const NULL: Sel = Sel(ptr::null());
}

impl fmt::Debug for Sel {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "@selector({})", self.name())
    }
}

/// `BOOL`: `bool` on AArch64, `signed char` on x86-64; both are a byte
/// that is 0 or 1 for `NO` and `YES`.
#[repr(transparent)]
#[derive(Clone, Copy, PartialEq, Eq, Default)]
pub(crate) struct Bool(i8);

impl Bool {
    pub(crate) const YES: Bool = Bool(1);
    pub(crate) const NO: Bool = Bool(0);

    /// Whether the value is `YES` (any non-zero value, as C tests it).
    pub(crate) fn is_true(self) -> bool {
        self.0 != 0
    }
}

impl From<bool> for Bool {
    fn from(b: bool) -> Bool {
        Bool(b as i8)
    }
}

impl From<Bool> for bool {
    fn from(b: Bool) -> bool {
        b.is_true()
    }
}

impl fmt::Debug for Bool {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(if self.is_true() { "YES" } else { "NO" })
    }
}

/// `YES`.
pub(crate) const YES: Bool = Bool::YES;
/// `NO`.
pub(crate) const NO: Bool = Bool::NO;

/// `NSInteger`.
pub(crate) type NSInteger = isize;
/// `NSUInteger`.
pub(crate) type NSUInteger = usize;

/// An instance variable (`Ivar`).
pub(crate) type Ivar = *const c_void;
/// A method implementation (`IMP`).
pub(crate) type Imp = unsafe extern "C" fn();

/// `struct objc_super`.
#[repr(C)]
pub(crate) struct ObjcSuper {
    pub(crate) receiver: Id,
    pub(crate) super_class: Class,
}

#[link(name = "objc", kind = "dylib")]
unsafe extern "C" {
    fn sel_registerName(name: *const c_char) -> Sel;
    fn sel_getName(sel: Sel) -> *const c_char;
    fn objc_getClass(name: *const c_char) -> Class;
    fn objc_getProtocol(name: *const c_char) -> *const c_void;
    fn objc_allocateClassPair(superclass: Class, name: *const c_char, extra: usize) -> Class;
    fn objc_registerClassPair(cls: Class);
    fn class_addMethod(cls: Class, name: Sel, imp: Imp, types: *const c_char) -> Bool;
    fn class_addIvar(
        cls: Class,
        name: *const c_char,
        size: usize,
        alignment: u8,
        types: *const c_char,
    ) -> Bool;
    fn class_addProtocol(cls: Class, protocol: *const c_void) -> Bool;
    fn class_getInstanceVariable(cls: Class, name: *const c_char) -> Ivar;
    fn class_getName(cls: Class) -> *const c_char;
    fn class_getSuperclass(cls: Class) -> Class;
    fn class_respondsToSelector(cls: Class, sel: Sel) -> Bool;
    fn class_getInstanceMethod(cls: Class, sel: Sel) -> *const c_void;
    fn class_getClassMethod(cls: Class, sel: Sel) -> *const c_void;
    fn class_replaceMethod(cls: Class, name: Sel, imp: Imp, types: *const c_char) -> Option<Imp>;
    fn method_getImplementation(m: *const c_void) -> Option<Imp>;
    fn method_getTypeEncoding(m: *const c_void) -> *const c_char;
    fn object_getClass(obj: Id) -> Class;
    fn ivar_getOffset(ivar: Ivar) -> isize;
    pub(crate) fn objc_retain(obj: Id) -> Id;
    pub(crate) fn objc_release(obj: Id);
    pub(crate) fn objc_autorelease(obj: Id) -> Id;
    fn objc_autoreleasePoolPush() -> *mut c_void;
    fn objc_autoreleasePoolPop(pool: *mut c_void);
    fn objc_getAssociatedObject(object: Id, key: *const c_void) -> Id;
    fn objc_setAssociatedObject(object: Id, key: *const c_void, value: Id, policy: usize);
    pub(crate) fn objc_msgSend();
    pub(crate) fn objc_msgSendSuper();
    #[cfg(target_arch = "x86_64")]
    pub(crate) fn objc_msgSend_stret();
    #[cfg(target_arch = "x86_64")]
    pub(crate) fn objc_msgSendSuper_stret();
}

/// A cache for one call site's selector ([`sel!`]).
pub(crate) struct SelCache(AtomicPtr<c_void>);

impl SelCache {
    pub(crate) const fn new() -> SelCache {
        SelCache(AtomicPtr::new(ptr::null_mut()))
    }

    /// The selector of `name` (NUL-terminated), registered on first use.
    pub(crate) fn get(&self, name: &'static str) -> Sel {
        let p = self.0.load(Ordering::Relaxed);
        if !p.is_null() {
            return Sel(p);
        }
        let name = CStr::from_bytes_with_nul(name.as_bytes()).expect("selector name");
        let sel = Sel::register(name);
        self.0.store(sel.0 as *mut c_void, Ordering::Relaxed);
        sel
    }
}

/// A cache for one call site's class ([`class!`]).
pub(crate) struct ClassCache(AtomicPtr<ObjcClass>);

impl ClassCache {
    pub(crate) const fn new() -> ClassCache {
        ClassCache(AtomicPtr::new(ptr::null_mut()))
    }

    /// The class named `name` (NUL-terminated), or null when the runtime
    /// doesn't know it (yet: a framework loaded later can add it, so a
    /// miss isn't cached).
    pub(crate) fn get(&self, name: &'static str) -> Class {
        let p = self.0.load(Ordering::Relaxed);
        if !p.is_null() {
            return p;
        }
        let cls = get_class(CStr::from_bytes_with_nul(name.as_bytes()).expect("class name"));
        if !cls.is_null() {
            self.0.store(cls as *mut ObjcClass, Ordering::Relaxed);
        }
        cls
    }
}

/// `@selector(...)`: `sel!(init)`, `sel!(initWithFrame:styleMask:)`.
macro_rules! sel {
    ($name:ident) => {{
        static CACHE: $crate::core::apple::objc::SelCache = $crate::core::apple::objc::SelCache::new();
        CACHE.get(concat!(stringify!($name), "\0"))
    }};
    ($($name:ident :)+) => {{
        static CACHE: $crate::core::apple::objc::SelCache = $crate::core::apple::objc::SelCache::new();
        CACHE.get(concat!($(stringify!($name), ":"),+, "\0"))
    }};
}
pub(crate) use sel;

/// A class by name, `class!(NSWindow)` (`[NSWindow class]`); null if the
/// runtime doesn't know it.
macro_rules! class {
    ($name:ident) => {{
        static CACHE: $crate::core::apple::objc::ClassCache =
            $crate::core::apple::objc::ClassCache::new();
        CACHE.get(concat!(stringify!($name), "\0"))
    }};
}
pub(crate) use class;

/// `[obj selector]`, `[obj a:x b:y]`: `objc_msgSend()` with the arguments'
/// types and the inferred return type. Unsafe: the types must be the
/// method's, and `obj` a valid object (or nil, which returns zero).
macro_rules! msg_send {
    ($obj:expr, $name:ident) => {
        $crate::core::apple::objc::send(
            $crate::core::apple::objc::Receiver::as_id(&$obj),
            $crate::core::apple::objc::sel!($name),
            (),
        )
    };
    ($obj:expr, $($name:ident : $arg:expr)+) => {
        $crate::core::apple::objc::send(
            $crate::core::apple::objc::Receiver::as_id(&$obj),
            $crate::core::apple::objc::sel!($($name:)+),
            ($($arg,)+),
        )
    };
}
pub(crate) use msg_send;

/// `[super selector...]` in a method of `class` (the class that defines
/// the method, whose superclass's implementation is called).
#[allow(unused_macros)]
macro_rules! msg_send_super {
    ($obj:expr, $class:expr, $name:ident) => {
        $crate::core::apple::objc::send_super(
            $crate::core::apple::objc::Receiver::as_id(&$obj),
            $class,
            $crate::core::apple::objc::sel!($name),
            (),
        )
    };
    ($obj:expr, $class:expr, $($name:ident : $arg:expr)+) => {
        $crate::core::apple::objc::send_super(
            $crate::core::apple::objc::Receiver::as_id(&$obj),
            $class,
            $crate::core::apple::objc::sel!($($name:)+),
            ($($arg,)+),
        )
    };
}
#[allow(unused_imports)]
pub(crate) use msg_send_super;

/// What a message can be sent to.
pub(crate) trait Receiver {
    fn as_id(&self) -> Id;
}

impl Receiver for Id {
    fn as_id(&self) -> Id {
        *self
    }
}

impl Receiver for *const Object {
    fn as_id(&self) -> Id {
        *self as Id
    }
}

impl Receiver for Class {
    fn as_id(&self) -> Id {
        *self as Id
    }
}

impl Receiver for StrongPtr {
    fn as_id(&self) -> Id {
        self.0
    }
}

impl Receiver for &StrongPtr {
    fn as_id(&self) -> Id {
        self.0
    }
}

impl Receiver for WeakId {
    fn as_id(&self) -> Id {
        self.0
    }
}

/// Whether a return value of `R` comes back through memory the caller
/// passes (`objc_msgSend_stret`): structures over 16 bytes on x86-64. On
/// AArch64 `objc_msgSend` handles every return type.
#[cfg(target_arch = "x86_64")]
fn needs_stret<R>() -> bool {
    size_of::<R>() > 16
}

/// The arguments of a message, as a tuple.
pub(crate) trait MessageArgs: Sized {
    /// # Safety
    ///
    /// `imp` must be `objc_msgSend` (or a variant taking the same leading
    /// arguments) and the types those of the method `sel` names.
    unsafe fn invoke<R>(
        imp: unsafe extern "C" fn(),
        receiver: *mut c_void,
        sel: Sel,
        args: Self,
    ) -> R;
}

macro_rules! message_args {
    ($($a:ident : $t:ident),*) => {
        impl<$($t: Copy),*> MessageArgs for ($($t,)*) {
            #[allow(non_snake_case, unused_parens)]
            unsafe fn invoke<R>(
                imp: unsafe extern "C" fn(),
                receiver: *mut c_void,
                sel: Sel,
                args: Self,
            ) -> R {
                let ($($a,)*) = args;
                // SAFETY: objc_msgSend takes the receiver and selector, then
                // the method's arguments, and returns its value (the
                // caller's contract on the types).
                unsafe {
                    let f: unsafe extern "C" fn(*mut c_void, Sel $(, $t)*) -> R =
                        std::mem::transmute(imp);
                    f(receiver, sel $(, $a)*)
                }
            }
        }
    };
}

message_args!();
message_args!(a: A);
message_args!(a: A, b: B);
message_args!(a: A, b: B, c: C);
message_args!(a: A, b: B, c: C, d: D);
message_args!(a: A, b: B, c: C, d: D, e: E);
message_args!(a: A, b: B, c: C, d: D, e: E, f: F);
message_args!(a: A, b: B, c: C, d: D, e: E, f: F, g: G);
message_args!(a: A, b: B, c: C, d: D, e: E, f: F, g: G, h: H);
message_args!(a: A, b: B, c: C, d: D, e: E, f: F, g: G, h: H, i: I);
message_args!(a: A, b: B, c: C, d: D, e: E, f: F, g: G, h: H, i: I, j: J);
message_args!(a: A, b: B, c: C, d: D, e: E, f: F, g: G, h: H, i: I, j: J, k: K);
message_args!(a: A, b: B, c: C, d: D, e: E, f: F, g: G, h: H, i: I, j: J, k: K, l: L);

/// Send a message ([`msg_send!`]).
///
/// # Safety
///
/// `obj` must be nil or a valid object, and `A`/`R` the argument and return
/// types of its method `sel`.
#[inline]
pub(crate) unsafe fn send<A: MessageArgs, R>(obj: Id, sel: Sel, args: A) -> R {
    #[cfg(target_arch = "x86_64")]
    if needs_stret::<R>() {
        // SAFETY: the stret variant for large structures (caller's contract).
        return unsafe { A::invoke(objc_msgSend_stret, obj as *mut c_void, sel, args) };
    }
    // SAFETY: the caller's contract.
    unsafe { A::invoke(objc_msgSend, obj as *mut c_void, sel, args) }
}

/// Send a message to the superclass's implementation ([`msg_send_super!`]).
///
/// # Safety
///
/// As [`send`]; `class` must be the class of the method calling this.
#[inline]
pub(crate) unsafe fn send_super<A: MessageArgs, R>(obj: Id, class: Class, sel: Sel, args: A) -> R {
    let sup = ObjcSuper {
        receiver: obj,
        // SAFETY: a valid class.
        super_class: unsafe { class_getSuperclass(class) },
    };
    let p = &sup as *const ObjcSuper as *mut c_void;
    #[cfg(target_arch = "x86_64")]
    if needs_stret::<R>() {
        // SAFETY: as in send(), with the objc_super in place of the receiver.
        return unsafe { A::invoke(objc_msgSendSuper_stret, p, sel, args) };
    }
    // SAFETY: as above.
    unsafe { A::invoke(objc_msgSendSuper, p, sel, args) }
}

/// `objc_getClass()`.
pub(crate) fn get_class(name: &CStr) -> Class {
    // SAFETY: a NUL-terminated name.
    unsafe { objc_getClass(name.as_ptr()) }
}

/// `objc_getProtocol()`, null when unknown.
pub(crate) fn get_protocol(name: &CStr) -> *const c_void {
    // SAFETY: a NUL-terminated name.
    unsafe { objc_getProtocol(name.as_ptr()) }
}

/// `object_getClass()`.
///
/// # Safety
///
/// `obj` must be a valid object.
pub(crate) unsafe fn class_of(obj: Id) -> Class {
    // SAFETY: the caller's contract.
    unsafe { object_getClass(obj) }
}

/// `class_getName()`.
pub(crate) fn class_name(cls: Class) -> &'static str {
    if cls.is_null() {
        return "nil";
    }
    // SAFETY: a valid class; its name lives as long as the class.
    unsafe { CStr::from_ptr(class_getName(cls)) }
        .to_str()
        .unwrap_or("")
}

/// `class_getSuperclass()`.
pub(crate) fn superclass(cls: Class) -> Class {
    // SAFETY: a valid class.
    unsafe { class_getSuperclass(cls) }
}

/// `[cls instancesRespondToSelector:sel]` (`class_respondsToSelector()`).
pub(crate) fn class_responds_to(cls: Class, sel: Sel) -> bool {
    if cls.is_null() {
        return false;
    }
    // SAFETY: a valid class.
    unsafe { class_respondsToSelector(cls, sel) }.is_true()
}

/// `[obj respondsToSelector:sel]`.
///
/// # Safety
///
/// `obj` must be nil or a valid object.
pub(crate) unsafe fn responds_to(obj: Id, sel: Sel) -> bool {
    if obj.is_null() {
        return false;
    }
    // SAFETY: the caller's contract.
    let r: Bool = unsafe { send(obj, sel!(respondsToSelector:), (sel,)) };
    r.is_true()
}

/// `[obj isKindOfClass:cls]`.
///
/// # Safety
///
/// `obj` must be nil or a valid object.
pub(crate) unsafe fn is_kind_of(obj: Id, cls: Class) -> bool {
    if obj.is_null() || cls.is_null() {
        return false;
    }
    // SAFETY: the caller's contract.
    let r: Bool = unsafe { send(obj, sel!(isKindOfClass:), (cls,)) };
    r.is_true()
}

/// The implementation and type encoding of an instance method, for method
/// swizzling (`class_getInstanceMethod()`).
pub(crate) fn instance_method(cls: Class, sel: Sel) -> Option<(Imp, &'static CStr)> {
    // SAFETY: a valid class; the method and its encoding live with it.
    unsafe {
        let m = class_getInstanceMethod(cls, sel);
        if m.is_null() {
            return None;
        }
        let imp = method_getImplementation(m)?;
        Some((imp, CStr::from_ptr(method_getTypeEncoding(m))))
    }
}

/// Whether the metaclass of `cls` has the class method `sel`
/// (`class_getClassMethod()`).
pub(crate) fn has_class_method(cls: Class, sel: Sel) -> bool {
    // SAFETY: a valid class.
    !cls.is_null() && !unsafe { class_getClassMethod(cls, sel) }.is_null()
}

/// `class_replaceMethod()`: install `imp` for `sel` on `cls`, returning the
/// implementation it replaced.
///
/// # Safety
///
/// `imp` must have the signature `types` encodes.
pub(crate) unsafe fn replace_method(cls: Class, sel: Sel, imp: Imp, types: &CStr) -> Option<Imp> {
    // SAFETY: the caller's contract.
    unsafe { class_replaceMethod(cls, sel, imp, types.as_ptr()) }
}

/// The byte offset of instance variable `name` in instances of `cls`.
pub(crate) fn ivar_offset(cls: Class, name: &CStr) -> Option<isize> {
    // SAFETY: a valid class and a NUL-terminated name.
    let ivar = unsafe { class_getInstanceVariable(cls, name.as_ptr()) };
    if ivar.is_null() {
        None
    } else {
        // SAFETY: a valid ivar.
        Some(unsafe { ivar_getOffset(ivar) })
    }
}

/// A pointer to instance variable `name` (of type `T`) of `obj`, an
/// instance of `cls` or a subclass of it.
///
/// # Safety
///
/// `obj` must be a valid instance and `T` the ivar's type.
pub(crate) unsafe fn ivar_ptr<T>(obj: Id, cls: Class, name: &CStr) -> *mut T {
    let off = ivar_offset(cls, name).expect("instance variable");
    // SAFETY: the ivar lies inside the object (the caller's contract).
    unsafe { (obj as *mut u8).offset(off) as *mut T }
}

/// `objc_retain()`, nil-safe.
///
/// # Safety
///
/// `obj` must be nil or a valid object.
pub(crate) unsafe fn retain(obj: Id) -> Id {
    if obj.is_null() {
        return obj;
    }
    // SAFETY: the caller's contract.
    unsafe { objc_retain(obj) }
}

/// `objc_release()`, nil-safe.
///
/// # Safety
///
/// `obj` must be nil or an object the caller owns a reference to.
pub(crate) unsafe fn release(obj: Id) {
    if !obj.is_null() {
        // SAFETY: the caller's contract.
        unsafe { objc_release(obj) }
    }
}

/// `objc_autorelease()`, nil-safe.
///
/// # Safety
///
/// `obj` must be nil or an object the caller owns a reference to, which it
/// gives to the current autorelease pool.
pub(crate) unsafe fn autorelease(obj: Id) -> Id {
    if obj.is_null() {
        return obj;
    }
    // SAFETY: the caller's contract.
    unsafe { objc_autorelease(obj) }
}

/// `OBJC_ASSOCIATION_RETAIN_NONATOMIC`.
pub(crate) const OBJC_ASSOCIATION_RETAIN_NONATOMIC: usize = 1;
/// `OBJC_ASSOCIATION_ASSIGN`.
pub(crate) const OBJC_ASSOCIATION_ASSIGN: usize = 0;

/// `objc_getAssociatedObject()`.
///
/// # Safety
///
/// `obj` must be a valid object.
pub(crate) unsafe fn associated_object(obj: Id, key: *const c_void) -> Id {
    // SAFETY: the caller's contract.
    unsafe { objc_getAssociatedObject(obj, key) }
}

/// `objc_setAssociatedObject()`.
///
/// # Safety
///
/// `obj` must be a valid object, `value` nil or one.
pub(crate) unsafe fn set_associated_object(obj: Id, key: *const c_void, value: Id, policy: usize) {
    // SAFETY: the caller's contract.
    unsafe { objc_setAssociatedObject(obj, key, value, policy) }
}

/// `@autoreleasepool { f() }`.
pub(crate) fn autoreleasepool<T>(f: impl FnOnce() -> T) -> T {
    struct Pool(*mut c_void);
    impl Drop for Pool {
        fn drop(&mut self) {
            // SAFETY: the token from the matching push, popped once.
            unsafe { objc_autoreleasePoolPop(self.0) }
        }
    }
    // SAFETY: pushes a pool popped when `_pool` goes out of scope.
    let _pool = Pool(unsafe { objc_autoreleasePoolPush() });
    f()
}

/// An owned reference to an object (a strong reference under ARC): from
/// `alloc`/`new`/`copy`/`mutableCopy` (`StrongPtr::from_owned`) or retained
/// (`StrongPtr::retain`), released on drop. Null is allowed (nil).
pub(crate) struct StrongPtr(Id);

// SAFETY: the Apple backends hand objects between threads as upstream does;
// retain/release are thread safe, and what may be messaged where is the
// translated code's business (as in the C).
unsafe impl Send for StrongPtr {}
// SAFETY: as above.
unsafe impl Sync for StrongPtr {}

impl StrongPtr {
    /// Take ownership of a `+1` reference.
    ///
    /// # Safety
    ///
    /// `obj` must be nil or an object reference the caller owns.
    pub(crate) unsafe fn from_owned(obj: Id) -> StrongPtr {
        StrongPtr(obj)
    }

    /// Retain an object (a `+0` reference, such as an autoreleased one).
    ///
    /// # Safety
    ///
    /// `obj` must be nil or a valid object.
    pub(crate) unsafe fn retain(obj: Id) -> StrongPtr {
        // SAFETY: the caller's contract.
        StrongPtr(unsafe { retain(obj) })
    }

    /// nil.
    pub(crate) const fn nil() -> StrongPtr {
        StrongPtr(ptr::null_mut())
    }

    pub(crate) fn is_nil(&self) -> bool {
        self.0.is_null()
    }

    /// The object, still owned by `self`.
    pub(crate) fn get(&self) -> Id {
        self.0
    }

    /// Give up ownership without releasing (the caller owns the `+1`).
    pub(crate) fn into_raw(self) -> Id {
        let obj = self.0;
        std::mem::forget(self);
        obj
    }

    /// Give the reference to the autorelease pool (`autorelease`).
    pub(crate) fn autorelease(self) -> Id {
        // SAFETY: the reference we own goes to the pool.
        unsafe { autorelease(self.into_raw()) }
    }
}

impl Clone for StrongPtr {
    fn clone(&self) -> StrongPtr {
        // SAFETY: we hold a reference, so the object is valid.
        StrongPtr(unsafe { retain(self.0) })
    }
}

impl Drop for StrongPtr {
    fn drop(&mut self) {
        // SAFETY: the reference we own.
        unsafe { release(self.0) }
    }
}

impl Default for StrongPtr {
    fn default() -> StrongPtr {
        StrongPtr::nil()
    }
}

impl fmt::Debug for StrongPtr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "StrongPtr({:p})", self.0)
    }
}

/// An unretained reference (`__unsafe_unretained` / `assign`), for
/// objects whose lifetime something else guarantees, sendable between
/// threads like a [`StrongPtr`].
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) struct WeakId(pub(crate) Id);

// SAFETY: as StrongPtr; this is only a pointer.
unsafe impl Send for WeakId {}
// SAFETY: as above.
unsafe impl Sync for WeakId {}

impl WeakId {
    pub(crate) const NIL: WeakId = WeakId(ptr::null_mut());

    pub(crate) fn is_nil(self) -> bool {
        self.0.is_null()
    }
}

/// Declaring a subclass at run time (`@interface Name : Superclass`):
/// methods, instance variables and protocols, then [`ClassBuilder::register`].
pub(crate) struct ClassBuilder {
    cls: Class,
}

impl ClassBuilder {
    /// `objc_allocateClassPair()`; `None` if a class of that name exists.
    pub(crate) fn new(name: &CStr, superclass: Class) -> Option<ClassBuilder> {
        // SAFETY: a valid superclass and a NUL-terminated name.
        let cls = unsafe { objc_allocateClassPair(superclass, name.as_ptr(), 0) };
        if cls.is_null() {
            None
        } else {
            Some(ClassBuilder { cls })
        }
    }

    /// Add an instance method (`class_addMethod()`); `types` is its type
    /// encoding (`"v@:@"` for `- (void)foo:(id)x`).
    ///
    /// # Safety
    ///
    /// `imp` must be an `extern "C" fn(Id, Sel, ...)` of the signature
    /// `types` encodes, cast to [`Imp`].
    pub(crate) unsafe fn add_method(&mut self, sel: Sel, imp: Imp, types: &CStr) {
        // SAFETY: the caller's contract.
        let ok = unsafe { class_addMethod(self.cls, sel, imp, types.as_ptr()) };
        debug_assert!(ok.is_true(), "class_addMethod({sel:?})");
    }

    /// Add a class method (to the metaclass).
    ///
    /// # Safety
    ///
    /// As [`ClassBuilder::add_method`], the receiver being the class.
    pub(crate) unsafe fn add_class_method(&mut self, sel: Sel, imp: Imp, types: &CStr) {
        // SAFETY: a class's isa is its metaclass; the caller's contract.
        unsafe {
            let meta = object_getClass(self.cls as Id);
            let ok = class_addMethod(meta, sel, imp, types.as_ptr());
            debug_assert!(ok.is_true(), "class_addMethod(+{sel:?})");
        }
    }

    /// Add an instance variable of Rust type `T` (`class_addIvar()`);
    /// `types` is its encoding (`"^v"` for a pointer, `"i"` for an int).
    pub(crate) fn add_ivar<T>(&mut self, name: &CStr, types: &CStr) {
        let align = align_of::<T>().trailing_zeros() as u8;
        // SAFETY: a class being declared and NUL-terminated strings.
        let ok = unsafe {
            class_addIvar(
                self.cls,
                name.as_ptr(),
                size_of::<T>(),
                align,
                types.as_ptr(),
            )
        };
        debug_assert!(ok.is_true(), "class_addIvar({name:?})");
    }

    /// Adopt a protocol, when the runtime knows it (`class_addProtocol()`).
    pub(crate) fn add_protocol(&mut self, name: &CStr) {
        let proto = get_protocol(name);
        if !proto.is_null() {
            // SAFETY: a class being declared and a valid protocol.
            unsafe { class_addProtocol(self.cls, proto) };
        }
    }

    /// `objc_registerClassPair()`: the class, ready for use.
    pub(crate) fn register(self) -> Class {
        // SAFETY: a class allocated and not yet registered.
        unsafe { objc_registerClassPair(self.cls) };
        self.cls
    }
}

/// A class declared once (by `declare`) and kept for the process' life, as
/// an `@implementation` is.
pub(crate) struct DeclaredClass(std::sync::OnceLock<usize>);

impl DeclaredClass {
    pub(crate) const fn new() -> DeclaredClass {
        DeclaredClass(std::sync::OnceLock::new())
    }

    /// The class, declared by `declare` on first use.
    pub(crate) fn get(&self, declare: impl FnOnce() -> Class) -> Class {
        *self.0.get_or_init(|| declare() as usize) as Class
    }
}

/// Cast a method implementation to [`Imp`] for [`ClassBuilder::add_method`].
#[allow(unused_macros)]
macro_rules! imp {
    ($f:expr) => {{
        // SAFETY: a function pointer, to be called with its own signature
        // (the method's type encoding says it).
        #[allow(clippy::missing_transmute_annotations, unused_unsafe)]
        let imp: $crate::core::apple::objc::Imp = unsafe { std::mem::transmute($f as *const ()) };
        imp
    }};
}
#[allow(unused_imports)]
pub(crate) use imp;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::apple::foundation::{ns_string, string_from_ns};

    #[test]
    fn selectors_and_classes() {
        assert_eq!(sel!(init).name(), "init");
        assert_eq!(
            sel!(initWithFrame:styleMask:).name(),
            "initWithFrame:styleMask:"
        );
        assert_eq!(sel!(init), sel!(init));
        assert!(!class!(NSObject).is_null());
        assert_eq!(class_name(class!(NSString)), "NSString");
        assert!(class!(NoSuchClassAnywhere).is_null());
    }

    #[test]
    fn messages_and_ownership() {
        autoreleasepool(|| unsafe {
            let s = ns_string("héllo, world");
            assert_eq!(string_from_ns(s.get()).as_deref(), Some("héllo, world"));
            let len: NSUInteger = msg_send![s, length];
            assert_eq!(len, 12);
            let upper: Id = msg_send![s, uppercaseString];
            assert_eq!(string_from_ns(upper).as_deref(), Some("HÉLLO, WORLD"));
            let has: Bool = msg_send![s, hasPrefix: ns_string("hé").get()];
            assert!(has.is_true());
            let rc: NSUInteger = msg_send![s, retainCount];
            let s2 = s.clone();
            let rc2: NSUInteger = msg_send![s2, retainCount];
            assert!(rc2 > rc || rc == NSUInteger::MAX);
            assert!(is_kind_of(s.get(), class!(NSString)));
            assert!(responds_to(s.get(), sel!(length)));
            assert!(!responds_to(s.get(), sel!(noSuchMethodHere)));
        });
    }

    /// Structures passed and returned by value (`objc_msgSend_stret` for
    /// the 32-byte NSRect on x86-64).
    #[test]
    fn structures_by_value() {
        use crate::core::apple::foundation::{NSPoint, NSRange, NSRect};
        autoreleasepool(|| unsafe {
            let r = NSRect::new(1.5, -2.0, 300.0, 400.25);
            let v: Id = msg_send![class!(NSValue), valueWithRect: r];
            let back: NSRect = msg_send![v, rectValue];
            assert_eq!(back, r);
            let p: Id = msg_send![class!(NSValue), valueWithPoint: NSPoint::new(7.0, 8.0)];
            let back: NSPoint = msg_send![p, pointValue];
            assert_eq!(back, NSPoint::new(7.0, 8.0));
            let s = crate::core::apple::foundation::ns_string("abcdef");
            let sub: Id = msg_send![s, substringWithRange: NSRange::new(2, 3)];
            assert_eq!(string_from_ns(sub).as_deref(), Some("cde"));
        });
    }

    static SUBCLASS: DeclaredClass = DeclaredClass::new();

    extern "C" fn test_value(this: Id, _sel: Sel) -> i32 {
        unsafe { *ivar_ptr::<i32>(this, class_of(this), c"_value") * 2 }
    }

    extern "C" fn test_set_value(this: Id, _sel: Sel, v: i32) {
        unsafe { *ivar_ptr::<i32>(this, class_of(this), c"_value") = v }
    }

    extern "C" fn test_description(this: Id, _sel: Sel) -> Id {
        let sup: Id =
            unsafe { msg_send_super![this, SUBCLASS.get(|| unreachable!()), description] };
        let s = unsafe { string_from_ns(sup) }.unwrap_or_default();
        ns_string(&format!("sdl:{}", s.starts_with("<SDLTestObject"))).autorelease()
    }

    #[test]
    fn declared_subclass() {
        let cls = SUBCLASS.get(|| {
            let mut b = ClassBuilder::new(c"SDLTestObject", class!(NSObject)).unwrap();
            b.add_ivar::<i32>(c"_value", c"i");
            unsafe {
                b.add_method(
                    sel!(value),
                    imp!(test_value as extern "C" fn(Id, Sel) -> i32),
                    c"i@:",
                );
                b.add_method(
                    sel!(setValue:),
                    imp!(test_set_value as extern "C" fn(Id, Sel, i32)),
                    c"v@:i",
                );
                b.add_method(
                    sel!(description),
                    imp!(test_description as extern "C" fn(Id, Sel) -> Id),
                    c"@@:",
                );
            }
            b.register()
        });
        assert!(ClassBuilder::new(c"SDLTestObject", class!(NSObject)).is_none());
        autoreleasepool(|| unsafe {
            let obj = StrongPtr::from_owned(msg_send![cls, new]);
            let _: () = msg_send![obj, setValue: 21i32];
            let v: i32 = msg_send![obj, value];
            assert_eq!(v, 42);
            let d: Id = msg_send![obj, description];
            assert_eq!(string_from_ns(d).as_deref(), Some("sdl:true"));
        });
    }
}
