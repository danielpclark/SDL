// Blocks and Grand Central Dispatch, declared by hand for the Apple backends.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! Upstream's `^{ ... }` literals, as heap blocks wrapping Rust closures
//! ([`RcBlock`], laid out as the Clang block ABI specifies), and the
//! `dispatch_*` calls the backends make (with function contexts in place of
//! blocks where GCD has `_f` variants).

use std::ffi::{c_char, c_int, c_ulong, c_void};
use std::sync::Arc;

/// `BLOCK_HAS_COPY_DISPOSE`.
const BLOCK_HAS_COPY_DISPOSE: c_int = 1 << 25;

/// `Block_descriptor_1` with `Block_descriptor_2` (copy/dispose).
#[repr(C)]
struct BlockDescriptor {
    reserved: c_ulong,
    size: c_ulong,
    copy_helper: unsafe extern "C" fn(dst: *mut BlockLayout, src: *const BlockLayout),
    dispose_helper: unsafe extern "C" fn(src: *mut BlockLayout),
}

/// How the closure a block captures is kept alive: an `Arc` of it, cloned
/// when the runtime copies the block and dropped when it disposes of one.
struct ClosureVtable {
    retain: unsafe fn(*const c_void),
    release: unsafe fn(*const c_void),
}

/// `Block_layout`, with the closure as its captured variables.
#[repr(C)]
struct BlockLayout {
    isa: *const c_void,
    flags: c_int,
    reserved: c_int,
    invoke: *const c_void,
    descriptor: *const BlockDescriptor,
    closure: *const c_void,
    vtable: &'static ClosureVtable,
}

#[link(name = "System", kind = "dylib")]
unsafe extern "C" {
    static _NSConcreteStackBlock: [*const c_void; 32];
    fn _Block_copy(block: *const c_void) -> *mut c_void;
    fn _Block_release(block: *const c_void);
}

unsafe extern "C" fn copy_helper(dst: *mut BlockLayout, src: *const BlockLayout) {
    // SAFETY: the runtime has copied `src` to `dst`, closure pointer and
    // all; the copy takes its own reference.
    unsafe {
        let _ = dst;
        ((*src).vtable.retain)((*src).closure)
    }
}

unsafe extern "C" fn dispose_helper(block: *mut BlockLayout) {
    // SAFETY: the block being freed drops its reference.
    unsafe { ((*block).vtable.release)((*block).closure) }
}

static DESCRIPTOR: BlockDescriptor = BlockDescriptor {
    reserved: 0,
    size: size_of::<BlockLayout>() as c_ulong,
    copy_helper,
    dispose_helper,
};

unsafe fn retain_closure<F>(p: *const c_void) {
    // SAFETY: `p` came from Arc::into_raw of an Arc<F>.
    unsafe { Arc::increment_strong_count(p as *const F) }
}

unsafe fn release_closure<F>(p: *const c_void) {
    // SAFETY: as above, dropping one reference.
    unsafe { Arc::decrement_strong_count(p as *const F) }
}

/// A heap block (`Block_copy()`'d), released on drop (`Block_release()`).
/// The runtime and the APIs it is passed to keep their own references.
pub(crate) struct RcBlock(*mut c_void);

// SAFETY: the closures are Send + Sync; the block's reference count is
// atomic.
unsafe impl Send for RcBlock {}
// SAFETY: as above.
unsafe impl Sync for RcBlock {}

impl RcBlock {
    /// A block whose `invoke` is `invoke` and whose captured state is `f`.
    fn with_invoke<F: Send + Sync + 'static>(f: F, invoke: *const c_void) -> RcBlock {
        let closure = Arc::into_raw(Arc::new(f)) as *const c_void;
        let vtable: &'static ClosureVtable = const {
            &ClosureVtable {
                retain: retain_closure::<F>,
                release: release_closure::<F>,
            }
        };
        let stack = BlockLayout {
            // SAFETY: taking the address of the runtime's class object.
            isa: std::ptr::addr_of!(_NSConcreteStackBlock) as *const c_void,
            flags: BLOCK_HAS_COPY_DISPOSE,
            reserved: 0,
            invoke,
            descriptor: &DESCRIPTOR,
            closure,
            vtable,
        };
        // SAFETY: a well-formed stack block; the copy calls copy_helper,
        // which takes a reference for the heap block, then the stack
        // block's reference is dropped.
        unsafe {
            let heap = _Block_copy(&stack as *const BlockLayout as *const c_void);
            release_closure::<F>(closure);
            RcBlock(heap)
        }
    }

    /// `^{ f(); }`.
    pub(crate) fn new0<R, F>(f: F) -> RcBlock
    where
        F: Fn() -> R + Send + Sync + 'static,
    {
        unsafe extern "C" fn invoke<R, F: Fn() -> R>(block: *mut BlockLayout) -> R {
            // SAFETY: the block holds a reference to its closure.
            unsafe { (*((*block).closure as *const F))() }
        }
        Self::with_invoke(f, invoke::<R, F> as *const c_void)
    }

    /// `^(A a) { f(a); }`.
    pub(crate) fn new1<A, R, F>(f: F) -> RcBlock
    where
        F: Fn(A) -> R + Send + Sync + 'static,
    {
        unsafe extern "C" fn invoke<A, R, F: Fn(A) -> R>(block: *mut BlockLayout, a: A) -> R {
            // SAFETY: as in new0.
            unsafe { (*((*block).closure as *const F))(a) }
        }
        Self::with_invoke(f, invoke::<A, R, F> as *const c_void)
    }

    /// `^(A a, B b) { f(a, b); }`.
    pub(crate) fn new2<A, B, R, F>(f: F) -> RcBlock
    where
        F: Fn(A, B) -> R + Send + Sync + 'static,
    {
        unsafe extern "C" fn invoke<A, B, R, F: Fn(A, B) -> R>(
            block: *mut BlockLayout,
            a: A,
            b: B,
        ) -> R {
            // SAFETY: as in new0.
            unsafe { (*((*block).closure as *const F))(a, b) }
        }
        Self::with_invoke(f, invoke::<A, B, R, F> as *const c_void)
    }

    /// `^(A a, B b, C c) { f(a, b, c); }`.
    pub(crate) fn new3<A, B, C, R, F>(f: F) -> RcBlock
    where
        F: Fn(A, B, C) -> R + Send + Sync + 'static,
    {
        unsafe extern "C" fn invoke<A, B, C, R, F: Fn(A, B, C) -> R>(
            block: *mut BlockLayout,
            a: A,
            b: B,
            c: C,
        ) -> R {
            // SAFETY: as in new0.
            unsafe { (*((*block).closure as *const F))(a, b, c) }
        }
        Self::with_invoke(f, invoke::<A, B, C, R, F> as *const c_void)
    }

    /// `^(A a, B b, C c, D d) { f(a, b, c, d); }`.
    pub(crate) fn new4<A, B, C, D, R, F>(f: F) -> RcBlock
    where
        F: Fn(A, B, C, D) -> R + Send + Sync + 'static,
    {
        unsafe extern "C" fn invoke<A, B, C, D, R, F: Fn(A, B, C, D) -> R>(
            block: *mut BlockLayout,
            a: A,
            b: B,
            c: C,
            d: D,
        ) -> R {
            // SAFETY: as in new0.
            unsafe { (*((*block).closure as *const F))(a, b, c, d) }
        }
        Self::with_invoke(f, invoke::<A, B, C, D, R, F> as *const c_void)
    }

    /// The block, to pass where an Objective-C or C API takes one.
    pub(crate) fn as_ptr(&self) -> *mut c_void {
        self.0
    }
}

impl Clone for RcBlock {
    fn clone(&self) -> RcBlock {
        // SAFETY: copying a heap block retains it.
        RcBlock(unsafe { _Block_copy(self.0) })
    }
}

impl Drop for RcBlock {
    fn drop(&mut self) {
        // SAFETY: the reference we own.
        unsafe { _Block_release(self.0) }
    }
}

// ---------------------------------------------------------------------------
// Grand Central Dispatch

/// `dispatch_queue_t`.
pub(crate) type DispatchQueue = *mut c_void;
/// `dispatch_semaphore_t`.
pub(crate) type DispatchSemaphore = *mut c_void;
/// `dispatch_time_t`.
pub(crate) type DispatchTime = u64;
/// `dispatch_function_t`.
pub(crate) type DispatchFunction = unsafe extern "C" fn(*mut c_void);

/// `DISPATCH_TIME_NOW`.
pub(crate) const DISPATCH_TIME_NOW: DispatchTime = 0;
/// `DISPATCH_TIME_FOREVER`.
pub(crate) const DISPATCH_TIME_FOREVER: DispatchTime = !0;
/// `DISPATCH_QUEUE_SERIAL`.
pub(crate) const DISPATCH_QUEUE_SERIAL: *const c_void = std::ptr::null();

#[link(name = "System", kind = "dylib")]
unsafe extern "C" {
    static _dispatch_main_q: c_void;
    pub(crate) fn dispatch_async_f(
        queue: DispatchQueue,
        context: *mut c_void,
        work: DispatchFunction,
    );
    pub(crate) fn dispatch_sync_f(
        queue: DispatchQueue,
        context: *mut c_void,
        work: DispatchFunction,
    );
    pub(crate) fn dispatch_async(queue: DispatchQueue, block: *mut c_void);
    pub(crate) fn dispatch_sync(queue: DispatchQueue, block: *mut c_void);
    pub(crate) fn dispatch_queue_create(label: *const c_char, attr: *const c_void)
        -> DispatchQueue;
    pub(crate) fn dispatch_get_global_queue(identifier: isize, flags: usize) -> DispatchQueue;
    pub(crate) fn dispatch_release(object: *mut c_void);
    pub(crate) fn dispatch_retain(object: *mut c_void);
    pub(crate) fn dispatch_semaphore_create(value: isize) -> DispatchSemaphore;
    pub(crate) fn dispatch_semaphore_wait(sema: DispatchSemaphore, timeout: DispatchTime) -> isize;
    pub(crate) fn dispatch_semaphore_signal(sema: DispatchSemaphore) -> isize;
    pub(crate) fn dispatch_time(when: DispatchTime, delta: i64) -> DispatchTime;
}

/// `dispatch_get_main_queue()`.
pub(crate) fn main_queue() -> DispatchQueue {
    // (the address of the main queue object libdispatch exports)
    std::ptr::addr_of!(_dispatch_main_q) as DispatchQueue
}

/// Run `f` on the main thread and wait for it (`dispatch_sync(
/// dispatch_get_main_queue(), ^{ ... })`); called directly when already on
/// the main thread, as upstream checks `[NSThread isMainThread]` first.
pub(crate) fn run_on_main_sync<R: Send, F: FnOnce() -> R + Send>(f: F) -> R {
    if super::foundation::is_main_thread() {
        return f();
    }
    struct Ctx<F, R> {
        f: Option<F>,
        r: Option<R>,
    }
    unsafe extern "C" fn work<F: FnOnce() -> R, R>(ctx: *mut c_void) {
        // SAFETY: the context outlives the synchronous call.
        let ctx = unsafe { &mut *(ctx as *mut Ctx<F, R>) };
        if let Some(f) = ctx.f.take() {
            ctx.r = Some(f());
        }
    }
    let mut ctx = Ctx {
        f: Some(f),
        r: None,
    };
    // SAFETY: dispatch_sync_f returns after `work` ran with the context.
    unsafe {
        dispatch_sync_f(
            main_queue(),
            &mut ctx as *mut Ctx<F, R> as *mut c_void,
            work::<F, R>,
        );
    }
    ctx.r.expect("main queue work ran")
}

/// Run `f` on the main thread later (`dispatch_async(dispatch_get_main_queue(), ^{ ... })`).
pub(crate) fn run_on_main_async(f: impl FnOnce() + Send + 'static) {
    run_async(main_queue(), f)
}

/// Run `f` on `queue` later (`dispatch_async(queue, ^{ ... })`).
pub(crate) fn run_async(queue: DispatchQueue, f: impl FnOnce() + Send + 'static) {
    type Work = Box<dyn FnOnce() + Send>;
    unsafe extern "C" fn work(ctx: *mut c_void) {
        // SAFETY: the box leaked below, reclaimed once.
        let f = unsafe { Box::from_raw(ctx as *mut Work) };
        f();
    }
    let ctx: Box<Work> = Box::new(Box::new(f));
    // SAFETY: the queue is valid; `work` takes the context back.
    unsafe { dispatch_async_f(queue, Box::into_raw(ctx) as *mut c_void, work) }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[test]
    fn blocks_call_and_free_their_closures() {
        static DROPS: AtomicUsize = AtomicUsize::new(0);
        struct Probe;
        impl Drop for Probe {
            fn drop(&mut self) {
                DROPS.fetch_add(1, Ordering::SeqCst);
            }
        }
        let probe = Probe;
        let b = RcBlock::new2(move |a: i32, b: f64| -> f64 {
            let _ = &probe;
            a as f64 * b
        });
        // Invoke through the block ABI: the function pointer at `invoke`.
        let call = |blk: &RcBlock, a: i32, b: f64| -> f64 {
            unsafe {
                let layout = blk.as_ptr() as *const BlockLayout;
                let f: unsafe extern "C" fn(*const BlockLayout, i32, f64) -> f64 =
                    std::mem::transmute((*layout).invoke);
                f(layout, a, b)
            }
        };
        assert_eq!(call(&b, 3, 1.5), 4.5);
        let b2 = b.clone();
        drop(b);
        assert_eq!(call(&b2, 2, 2.0), 4.0);
        assert_eq!(DROPS.load(Ordering::SeqCst), 0);
        drop(b2);
        assert_eq!(DROPS.load(Ordering::SeqCst), 1);
    }

    /// A four-argument block (GCKeyboard's `keyChangedHandler` takes a
    /// keyboard, a key, a key code and a pressed flag), called through the
    /// block ABI with arguments of different sizes.
    #[test]
    fn four_argument_blocks() {
        let b = RcBlock::new4(|a: *mut c_void, b: *mut c_void, c: isize, d: bool| -> i64 {
            (a as i64) * 1000 + (b as i64) * 100 + c as i64 * 10 + d as i64
        });
        let r = unsafe {
            let layout = b.as_ptr() as *const BlockLayout;
            let f: unsafe extern "C" fn(
                *const BlockLayout,
                *mut c_void,
                *mut c_void,
                isize,
                bool,
            ) -> i64 = std::mem::transmute((*layout).invoke);
            f(layout, 4 as *mut c_void, 3 as *mut c_void, 2, true)
        };
        assert_eq!(r, 4321);
    }

    #[test]
    fn dispatch_to_a_queue() {
        let q =
            unsafe { dispatch_queue_create(c"org.libsdl.test".as_ptr(), DISPATCH_QUEUE_SERIAL) };
        let sem = unsafe { dispatch_semaphore_create(0) };
        let sem_addr = sem as usize;
        let hits = Arc::new(AtomicUsize::new(0));
        let h = hits.clone();
        run_async(q, move || {
            h.fetch_add(1, Ordering::SeqCst);
            unsafe { dispatch_semaphore_signal(sem_addr as DispatchSemaphore) };
        });
        let b = RcBlock::new0({
            let h = hits.clone();
            move || {
                h.fetch_add(10, Ordering::SeqCst);
                unsafe { dispatch_semaphore_signal(sem_addr as DispatchSemaphore) };
            }
        });
        unsafe { dispatch_async(q, b.as_ptr()) };
        drop(b);
        for _ in 0..2 {
            unsafe { dispatch_semaphore_wait(sem, DISPATCH_TIME_FOREVER) };
        }
        assert_eq!(hits.load(Ordering::SeqCst), 11);
        unsafe {
            dispatch_release(sem);
            dispatch_release(q);
        }
    }
}
