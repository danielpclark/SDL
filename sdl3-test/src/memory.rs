// Rust translation of src/test/SDL_test_memory.c and include/SDL3/SDL_test_memory.h
// from Simple DirectMedia Layer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! Memory tracking related functions of SDL test framework.
//!
//! This is a simple tracking allocator to demonstrate the use of SDL's
//! memory allocation replacement functionality.
//!
//! It gets slow with large numbers of allocations and shouldn't be used
//! for production code.
//!
//! Rust programs don't allocate through `SDL_malloc()`, so where upstream
//! replaces SDL's memory functions with `SDL_SetMemoryFunctions()`, this
//! tracks Rust's own allocations: a program makes [`TrackingAllocator`]
//! its global allocator, and then [`track_allocations`] starts tracking
//! (until then it only passes the allocations on to the system's
//! allocator, as upstream's functions before they are installed):
//!
//! ```no_run
//! use sdl3_test::memory::{self, TrackingAllocator};
//!
//! #[global_allocator]
//! static ALLOCATOR: TrackingAllocator = TrackingAllocator;
//!
//! fn main() {
//!     memory::track_allocations();
//!     // ... the test ...
//!     memory::log_allocations();
//! }
//! ```
//!
//! Each allocation's stack is recorded where upstream records it: through
//! the system's unwinder (`_Unwind_Backtrace()`, where upstream uses
//! libunwind) with names from `dladdr()` on Unix, and with
//! `RtlCaptureStackBackTrace()` and dbghelp.dll (loaded at run time) on
//! Windows.

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::{Cell, UnsafeCell};
use std::ptr;
use std::sync::atomic::{AtomicBool, AtomicI32, AtomicIsize, Ordering};

use sdl3::stdlib::string::strcasecmp;

use crate::crc32::Crc32Context;

const MAXIMUM_TRACKED_STACK_DEPTH: usize = 32;

/// Translation of `SDL_tracked_allocation`.
struct TrackedAllocation {
    mem: *mut u8,
    size: usize,
    stack: [u64; MAXIMUM_TRACKED_STACK_DEPTH],
    next: *mut TrackedAllocation,
}

/// The table of tracked allocations, only touched with the allocator lock
/// held.
struct Buckets(UnsafeCell<[*mut TrackedAllocation; 256]>);

// SAFETY: the table is only read and written with LOCK held.
unsafe impl Sync for Buckets {}

static S_CRC32_CONTEXT: Crc32Context = Crc32Context::new();
/// Whether the tracking functions are installed (upstream's
/// `SDL_malloc_orig != NULL`).
static S_TRACKING: AtomicBool = AtomicBool::new(false);
static S_PREVIOUS_ALLOCATIONS: AtomicI32 = AtomicI32::new(0);
static S_UNKNOWN_FREES: AtomicI32 = AtomicI32::new(0);
static S_TRACKED_ALLOCATIONS: Buckets = Buckets(UnsafeCell::new([ptr::null_mut(); 256]));
static S_RANDFILL_ALLOCATIONS: AtomicBool = AtomicBool::new(false);
static S_LOCK: AtomicI32 = AtomicI32::new(0);
/// Whether the stacks are logged with the names of their functions.
/// Translation of `s_unwind_symbol_names`.
static S_SYMBOL_NAMES: AtomicBool = AtomicBool::new(true);

/// The allocations made through the [`TrackingAllocator`] and not yet
/// freed: `SDL_GetNumAllocations()` for Rust's allocations.
static NUM_ALLOCATIONS: AtomicIsize = AtomicIsize::new(0);
/// Whether an allocation went through the [`TrackingAllocator`], so that it
/// is the global allocator.
static INSTALLED: AtomicBool = AtomicBool::new(false);

thread_local! {
    /// Set while this thread is in the tracking code, whose own allocations
    /// (an assertion report) are passed on untracked.
    // (const and without a destructor, so using it doesn't allocate)
    static IN_TRACKER: Cell<bool> = const { Cell::new(false) };
}

fn lock_allocator() {
    loop {
        if S_LOCK
            .compare_exchange(0, 1, Ordering::Acquire, Ordering::Relaxed)
            .is_ok()
        {
            break;
        }
        std::hint::spin_loop();
    }
}

fn unlock_allocator() {
    S_LOCK.store(0, Ordering::Release);
}

/// The table, for use with the lock held.
///
/// # Safety
///
/// The caller holds the allocator lock, and no other reference from this
/// function is alive.
#[allow(clippy::mut_from_ref)]
unsafe fn buckets() -> &'static mut [*mut TrackedAllocation; 256] {
    // SAFETY: per the contract, the lock makes this the only reference.
    unsafe { &mut *S_TRACKED_ALLOCATIONS.0.get() }
}

/// Translation of `get_allocation_bucket()`.
fn get_allocation_bucket(mem: *mut u8) -> usize {
    let crc_value = S_CRC32_CONTEXT.calc(&(mem as usize).to_ne_bytes());
    (crc_value as usize) & (256 - 1)
}

/// Translation of `SDL_GetTrackedAllocation()`.
fn get_tracked_allocation(mem: *mut u8) -> Option<usize> {
    lock_allocator();
    let index = get_allocation_bucket(mem);
    // SAFETY: the lock is held; entries are only freed with it held.
    let mut entry = unsafe { buckets()[index] };
    while !entry.is_null() {
        // SAFETY: entries in the table are valid while the lock is held.
        let e = unsafe { &*entry };
        if mem == e.mem {
            let size = e.size;
            unlock_allocator();
            return Some(size);
        }
        entry = e.next;
    }
    unlock_allocator();
    None
}

/// Translation of `SDL_GetTrackedAllocationSize()` (`None` for `SIZE_MAX`).
fn get_tracked_allocation_size(mem: *mut u8) -> Option<usize> {
    get_tracked_allocation(mem)
}

/// Translation of `SDL_IsAllocationTracked()`.
fn is_allocation_tracked(mem: *mut u8) -> bool {
    get_tracked_allocation(mem).is_some()
}

/// Translation of `SDL_TrackAllocation()`.
fn track_allocation(mem: *mut u8, size: usize) {
    let index = get_allocation_bucket(mem);

    if is_allocation_tracked(mem) {
        return;
    }
    let layout = Layout::new::<TrackedAllocation>();
    // SAFETY: the layout has a non-zero size.
    let entry = unsafe { System.alloc(layout) } as *mut TrackedAllocation;
    if entry.is_null() {
        return;
    }

    /* Generate the stack trace for the allocation */
    let mut stack = [0u64; MAXIMUM_TRACKED_STACK_DEPTH];
    capture_stack(&mut stack);

    lock_allocator();
    // SAFETY: the entry was just allocated for a TrackedAllocation, and the
    // lock is held for the table.
    unsafe {
        entry.write(TrackedAllocation {
            mem,
            size,
            stack,
            next: buckets()[index],
        });
        buckets()[index] = entry;
    }
    unlock_allocator();
}

/// Translation of `SDL_UntrackAllocation()`.
fn untrack_allocation(mem: *mut u8) {
    let index = get_allocation_bucket(mem);

    lock_allocator();
    // SAFETY: the lock is held for the table and its entries.
    unsafe {
        let mut prev_next_ptr: *mut *mut TrackedAllocation = &mut buckets()[index];
        let mut entry = *prev_next_ptr;
        while !entry.is_null() {
            if mem == (*entry).mem {
                *prev_next_ptr = (*entry).next;
                System.dealloc(entry as *mut u8, Layout::new::<TrackedAllocation>());
                unlock_allocator();
                return;
            }
            prev_next_ptr = &mut (*entry).next;
            entry = (*entry).next;
        }
    }
    S_UNKNOWN_FREES.fetch_add(1, Ordering::Relaxed);
    unlock_allocator();
}

/// Translation of `rand_fill_memory()`.
///
/// # Safety
///
/// `ptr` is valid for writes of `end` bytes.
unsafe fn rand_fill_memory(ptr: *mut u8, start: usize, end: usize) {
    if !S_RANDFILL_ALLOCATIONS.load(Ordering::Relaxed) {
        return;
    }

    for i in start..end {
        // SAFETY: i < end, within the allocation per the contract.
        unsafe { ptr.add(i).write(crate::fuzzer::random_uint8()) };
    }
}

/// Whether to pass an allocation on untracked: tracking is off, or this
/// thread is already in the tracking code.
fn untracked() -> bool {
    !S_TRACKING.load(Ordering::Acquire) || IN_TRACKER.with(Cell::get)
}

/// Run the tracking code with this thread marked as in it.
fn in_tracker<R>(f: impl FnOnce() -> R) -> R {
    IN_TRACKER.with(|t| t.set(true));
    let result = f();
    IN_TRACKER.with(|t| t.set(false));
    result
}

/// A global allocator that can track the allocations: everything goes to
/// the system's allocator ([`System`]), and once [`track_allocations`] is
/// called, each allocation is recorded with its stack until it is freed.
/// Translation of `SDLTest_TrackedMalloc()`, `SDLTest_TrackedCalloc()`,
/// `SDLTest_TrackedRealloc()` and `SDLTest_TrackedFree()`.
#[derive(Clone, Copy, Default, Debug)]
pub struct TrackingAllocator;

// SAFETY: every request is passed on to the system allocator as it is; the
// tracking only records the pointers, and writes (the random fill) only
// within the blocks the system returned.
unsafe impl GlobalAlloc for TrackingAllocator {
    /// Translation of `SDLTest_TrackedMalloc()`.
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        // SAFETY: the caller's layout, passed on.
        let mem = unsafe { System.alloc(layout) };
        if !mem.is_null() {
            INSTALLED.store(true, Ordering::Relaxed);
            NUM_ALLOCATIONS.fetch_add(1, Ordering::Relaxed);
            if !untracked() {
                in_tracker(|| {
                    track_allocation(mem, layout.size());
                    // SAFETY: the block has layout.size() bytes.
                    unsafe { rand_fill_memory(mem, 0, layout.size()) };
                });
            }
        }
        mem
    }

    /// Translation of `SDLTest_TrackedCalloc()`.
    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        // SAFETY: the caller's layout, passed on.
        let mem = unsafe { System.alloc_zeroed(layout) };
        if !mem.is_null() {
            INSTALLED.store(true, Ordering::Relaxed);
            NUM_ALLOCATIONS.fetch_add(1, Ordering::Relaxed);
            if !untracked() {
                in_tracker(|| track_allocation(mem, layout.size()));
            }
        }
        mem
    }

    /// Translation of `SDLTest_TrackedRealloc()`.
    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        if untracked() {
            // SAFETY: the caller's block and sizes, passed on.
            return unsafe { System.realloc(ptr, layout, new_size) };
        }
        in_tracker(|| {
            // Note (upstream): the C code asserts that the block is tracked
            // even when blocks from before the tracking can exist (where its
            // free() validation is off), to know its old size; the old size
            // is in the layout here, and the assertion is made where the
            // free() validation is.
            let old_size = get_tracked_allocation_size(ptr).unwrap_or(layout.size());
            if S_PREVIOUS_ALLOCATIONS.load(Ordering::Relaxed) == 0 {
                sdl3::sdl_assert!(is_allocation_tracked(ptr));
            }
            // SAFETY: the caller's block and sizes, passed on.
            let mem = unsafe { System.realloc(ptr, layout, new_size) };
            // FIXME (upstream): the old block is untracked even when the
            // reallocation fails, leaving it allocated.
            untrack_allocation(ptr);
            if !mem.is_null() {
                track_allocation(mem, new_size);
                if new_size > old_size {
                    // SAFETY: the block has new_size bytes.
                    unsafe { rand_fill_memory(mem, old_size, new_size) };
                }
            }
            mem
        })
    }

    /// Translation of `SDLTest_TrackedFree()`.
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        if ptr.is_null() {
            return;
        }
        NUM_ALLOCATIONS.fetch_sub(1, Ordering::Relaxed);

        if !untracked() {
            in_tracker(|| {
                if S_PREVIOUS_ALLOCATIONS.load(Ordering::Relaxed) == 0 {
                    sdl3::sdl_assert!(is_allocation_tracked(ptr));
                }
                untrack_allocation(ptr);
            });
        }
        // SAFETY: the caller's block and layout, passed on.
        unsafe { System.dealloc(ptr, layout) };
    }
}

/// Start tracking memory allocations: the allocations made through the
/// [`TrackingAllocator`] (which has to be the global allocator) from now on.
/// Frees of blocks allocated before are only validated if there were none.
///
/// This should be called before anything else allocates, for complete
/// tracking coverage. Translation of `SDLTest_TrackAllocations()`.
pub fn track_allocations() {
    if S_TRACKING.load(Ordering::Acquire) {
        return;
    }

    // (SDLTest_Crc32Init(): the context is a constant)

    // Allocate once, to know whether the allocations come through here.
    drop(std::hint::black_box(Box::new(0u8)));
    let previous = if INSTALLED.load(Ordering::Relaxed) {
        NUM_ALLOCATIONS
            .load(Ordering::Relaxed)
            .clamp(0, i32::MAX as isize) as i32
    } else {
        -1
    };
    S_PREVIOUS_ALLOCATIONS.store(previous, Ordering::Relaxed);

    // Note (upstream): the SDL_TRACKMEM_SYMBOL_NAMES environment variable
    // only applies to builds whose libunwind can't name an address later,
    // which name the frames as the allocation is made; here it turns the
    // names off everywhere (and dbghelp.dll isn't loaded on Windows).
    /* Don't use SDL_GetHint: SDL_malloc is off limits. */
    if let Some(env_trackmem) = sdl3::stdlib::getenv_unsafe("SDL_TRACKMEM_SYMBOL_NAMES") {
        let is = |value: &str| strcasecmp(&env_trackmem, value) == std::cmp::Ordering::Equal;
        if is("1") || is("yes") || is("true") {
            S_SYMBOL_NAMES.store(true, Ordering::Relaxed);
        } else if is("0") || is("no") || is("false") {
            S_SYMBOL_NAMES.store(false, Ordering::Relaxed);
        }
    }

    if S_SYMBOL_NAMES.load(Ordering::Relaxed) {
        symbols::init();
    }

    S_TRACKING.store(true, Ordering::Release);

    if previous < 0 {
        sdl3::log!(
            "The TrackingAllocator is not the global allocator, no allocation will be tracked"
        );
    } else if previous != 0 {
        sdl3::log!(
            "SDLTest_TrackAllocations(): There are {} previous allocations, disabling free() validation",
            previous
        );
    }
}

/// Fill allocations with random data from the fuzzer (whose sequence the
/// harness sets for each test). This implicitly calls
/// [`track_allocations`]. Translation of `SDLTest_RandFillAllocations()`.
pub fn rand_fill_allocations() {
    track_allocations();

    S_RANDFILL_ALLOCATIONS.store(true, Ordering::Relaxed);
}

/// What [`log_allocations`] reports of an allocation.
#[derive(Clone, Copy)]
struct Snapshot {
    size: usize,
    stack: [u64; MAXIMUM_TRACKED_STACK_DEPTH],
}

/// Print a log of any outstanding allocations: each one's size and stack,
/// then the total (and the number of frees of blocks that weren't tracked).
/// This can be called after SDL's `quit()`. Translation of
/// `SDLTest_LogAllocations()`.
pub fn log_allocations() {
    if !S_TRACKING.load(Ordering::Acquire) {
        return;
    }

    // The entries are copied out with the lock held, into memory from the
    // system's allocator (upstream's SDL_realloc_orig()), so that the
    // allocations made for the message don't change them or wait for the
    // lock.
    lock_allocator();
    // SAFETY: the lock is held for the table and its entries; the copy has
    // room for every entry counted.
    let (snapshots, count) = unsafe {
        let mut count = 0;
        for &head in buckets().iter() {
            let mut entry = head;
            while !entry.is_null() {
                count += 1;
                entry = (*entry).next;
            }
        }
        let layout = Layout::array::<Snapshot>(count.max(1)).unwrap_or(Layout::new::<Snapshot>());
        let snapshots = System.alloc(layout) as *mut Snapshot;
        if snapshots.is_null() {
            unlock_allocator();
            return;
        }
        let mut n = 0;
        for &head in buckets().iter() {
            let mut entry = head;
            while !entry.is_null() {
                snapshots.add(n).write(Snapshot {
                    size: (*entry).size,
                    stack: (*entry).stack,
                });
                n += 1;
                entry = (*entry).next;
            }
        }
        (snapshots, count)
    };
    unlock_allocator();
    let unknown_frees = S_UNKNOWN_FREES.load(Ordering::Relaxed);

    let mut message = String::new();
    message += "Memory allocations:\n";

    let mut total_allocated: u64 = 0;
    for index in 0..count {
        // SAFETY: index < count, which the copy holds.
        let entry = unsafe { &*snapshots.add(index) };
        message += &format!("Allocation {}: {} bytes\n", index, entry.size as i32);
        /* Start at stack index 1 to skip our tracking functions */
        for &address in &entry.stack[1..] {
            if address == 0 {
                break;
            }
            let stack_entry_description = if S_SYMBOL_NAMES.load(Ordering::Relaxed) {
                symbols::describe(address)
            } else {
                "???".to_owned()
            };
            message += &format!("\t0x{address:x}: {stack_entry_description}\n");
        }
        total_allocated += entry.size as u64;
    }
    // SAFETY: allocated above with this layout.
    unsafe {
        let layout = Layout::array::<Snapshot>(count.max(1)).unwrap_or(Layout::new::<Snapshot>());
        System.dealloc(snapshots as *mut u8, layout);
    }
    message += &format!(
        "Total: {:.2} Kb in {} allocations",
        total_allocated as f64 / 1024.0,
        count
    );
    if unknown_frees != 0 {
        message += &format!(", {unknown_frees} unknown frees");
    }
    message += "\n";

    sdl3::log!("{}", message);
}

/// Record the stack of the caller of the tracking code (frame 0 is the
/// tracking code itself; upstream starts at its caller too).
#[cfg(all(
    any(target_os = "linux", target_os = "macos", target_os = "freebsd"),
    not(target_arch = "arm")
))]
fn capture_stack(stack: &mut [u64; MAXIMUM_TRACKED_STACK_DEPTH]) {
    use std::ffi::{c_int, c_void};

    #[repr(C)]
    struct UnwindContext {
        _private: [u8; 0],
    }
    type UnwindTraceFn = extern "C" fn(*mut UnwindContext, *mut c_void) -> c_int;
    const URC_NO_REASON: c_int = 0;
    const URC_END_OF_STACK: c_int = 5;

    // The system's unwinder, which the Rust runtime links.
    extern "C" {
        fn _Unwind_Backtrace(trace: UnwindTraceFn, trace_argument: *mut c_void) -> c_int;
        fn _Unwind_GetIP(context: *mut UnwindContext) -> usize;
    }

    struct Trace<'a> {
        stack: &'a mut [u64; MAXIMUM_TRACKED_STACK_DEPTH],
        frame: usize,
        stack_index: usize,
    }

    extern "C" fn trace(context: *mut UnwindContext, argument: *mut c_void) -> c_int {
        // SAFETY: the argument is the Trace given to _Unwind_Backtrace().
        let trace = unsafe { &mut *(argument as *mut Trace<'_>) };
        trace.frame += 1;
        if trace.frame == 1 {
            return URC_NO_REASON;
        }
        // SAFETY: the context of the frame being visited.
        let pc = unsafe { _Unwind_GetIP(context) };
        trace.stack[trace.stack_index] = pc as u64;
        trace.stack_index += 1;

        if trace.stack_index == MAXIMUM_TRACKED_STACK_DEPTH {
            return URC_END_OF_STACK;
        }
        URC_NO_REASON
    }

    let mut state = Trace {
        stack,
        frame: 0,
        stack_index: 0,
    };
    // SAFETY: the callback only uses the Trace passed with it.
    unsafe {
        _Unwind_Backtrace(trace, &mut state as *mut Trace<'_> as *mut c_void);
    }
}

#[cfg(windows)]
fn capture_stack(stack: &mut [u64; MAXIMUM_TRACKED_STACK_DEPTH]) {
    use windows_sys::Win32::System::Diagnostics::Debug::RtlCaptureStackBackTrace;

    let mut frames = [ptr::null_mut(); 63];

    // SAFETY: the array has room for the frames asked for.
    let count = unsafe {
        RtlCaptureStackBackTrace(1, frames.len() as u32, frames.as_mut_ptr(), ptr::null_mut())
    } as usize;

    let count = count.min(MAXIMUM_TRACKED_STACK_DEPTH);
    for i in 0..count {
        stack[i] = frames[i] as usize as u64;
    }
}

/// Without a way to walk the stack, no stack is recorded (as upstream
/// without libunwind).
#[cfg(not(any(
    windows,
    all(
        any(target_os = "linux", target_os = "macos", target_os = "freebsd"),
        not(target_arch = "arm")
    )
)))]
fn capture_stack(_stack: &mut [u64; MAXIMUM_TRACKED_STACK_DEPTH]) {}

/// The names of the addresses on the recorded stacks.
#[cfg(unix)]
mod symbols {
    use std::ffi::CStr;

    pub(super) fn init() {}

    /// `name+0xoffset`, as upstream's `unw_get_proc_name_by_ip()`
    /// description, from the dynamic symbol table (`???+0x0` without a
    /// name).
    pub(super) fn describe(address: u64) -> String {
        let mut info = libc::Dl_info {
            dli_fname: std::ptr::null(),
            dli_fbase: std::ptr::null_mut(),
            dli_sname: std::ptr::null(),
            dli_saddr: std::ptr::null_mut(),
        };
        // SAFETY: dladdr() only reads the address and fills the info.
        let found = unsafe { libc::dladdr(address as usize as *const libc::c_void, &mut info) };
        if found != 0 && !info.dli_sname.is_null() {
            // SAFETY: a NUL-terminated name from the loader.
            let name = unsafe { CStr::from_ptr(info.dli_sname) }.to_string_lossy();
            let offset = (address as usize).wrapping_sub(info.dli_saddr as usize);
            format!("{name}+0x{offset:x}")
        } else {
            "???+0x0".to_owned()
        }
    }
}

/// The names of the addresses on the recorded stacks, from dbghelp.dll
/// (loaded at run time, as upstream).
#[cfg(windows)]
mod symbols {
    use std::ffi::CStr;
    use std::sync::Mutex;

    use sdl3::loadso::SharedObject;
    use windows_sys::core::BOOL;
    use windows_sys::Win32::Foundation::HANDLE;
    use windows_sys::Win32::System::Diagnostics::Debug::{
        IMAGEHLP_LINE64, MAX_SYM_NAME, SYMBOL_INFO,
    };
    use windows_sys::Win32::System::Threading::GetCurrentProcess;

    type SymInitializeFn = unsafe extern "system" fn(HANDLE, *const u8, BOOL) -> BOOL;
    type SymFromAddrFn = unsafe extern "system" fn(HANDLE, u64, *mut u64, *mut SYMBOL_INFO) -> BOOL;
    type SymGetLineFromAddr64Fn =
        unsafe extern "system" fn(HANDLE, u64, *mut u32, *mut IMAGEHLP_LINE64) -> BOOL;

    /// Translation of `dyn_dbghelp`.
    struct DynDbghelp {
        _module: SharedObject,
        p_sym_from_addr: SymFromAddrFn,
        p_sym_get_line_from_addr64: SymGetLineFromAddr64Fn,
    }

    static DYN_DBGHELP: Mutex<Option<DynDbghelp>> = Mutex::new(None);

    /// Load dbghelp.dll and initialize its symbol handler.
    pub(super) fn init() {
        *DYN_DBGHELP.lock().unwrap_or_else(|e| e.into_inner()) = load();
    }

    /// Translation of the loading of `dyn_dbghelp` in `SDLTest_TrackAllocations()`.
    fn load() -> Option<DynDbghelp> {
        let module = SharedObject::load("dbghelp.dll").ok()?;
        // SAFETY: the types of dbghelp.dll's functions.
        let (p_sym_initialize, p_sym_from_addr, p_sym_get_line_from_addr64) = unsafe {
            (
                module.function::<SymInitializeFn>("SymInitialize").ok()?,
                module.function::<SymFromAddrFn>("SymFromAddr").ok()?,
                module
                    .function::<SymGetLineFromAddr64Fn>("SymGetLineFromAddr64")
                    .ok()?,
            )
        };
        // SAFETY: the current process's pseudo handle, no search path.
        if unsafe { p_sym_initialize(GetCurrentProcess(), std::ptr::null(), 1) } == 0 {
            return None;
        }
        Some(DynDbghelp {
            _module: module,
            p_sym_from_addr,
            p_sym_get_line_from_addr64,
        })
    }

    /// `name+0xdisplacement file:line`, as upstream's description.
    pub(super) fn describe(address: u64) -> String {
        // SYMBOL_INFO with room for a name of MAX_SYM_NAME characters.
        #[repr(C)]
        struct SymbolBuffer {
            info: SYMBOL_INFO,
            name: [u8; MAX_SYM_NAME as usize],
        }
        let mut name = "???".to_owned();
        let mut displacement: u64 = 0;
        let mut file = String::new();
        let mut line_number = 0u32;

        let dbghelp = DYN_DBGHELP.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(dbghelp) = dbghelp.as_ref() {
            // SAFETY: plain data, zeroed.
            let mut symbol: SymbolBuffer = unsafe { std::mem::zeroed() };
            symbol.info.SizeOfStruct = std::mem::size_of::<SYMBOL_INFO>() as u32;
            symbol.info.MaxNameLen = MAX_SYM_NAME;
            // SAFETY: plain data, zeroed.
            let mut dbg_line: IMAGEHLP_LINE64 = unsafe { std::mem::zeroed() };
            dbg_line.SizeOfStruct = std::mem::size_of::<IMAGEHLP_LINE64>() as u32;
            let mut line_column = 0u32;

            // SAFETY: the buffers are as large as their sizes say.
            unsafe {
                if (dbghelp.p_sym_from_addr)(
                    GetCurrentProcess(),
                    address,
                    &mut displacement,
                    &mut symbol.info,
                ) != 0
                {
                    let name_ptr = symbol.info.Name.as_ptr() as *const std::ffi::c_char;
                    name = CStr::from_ptr(name_ptr).to_string_lossy().into_owned();
                } else {
                    displacement = 0;
                }
                if (dbghelp.p_sym_get_line_from_addr64)(
                    GetCurrentProcess(),
                    address,
                    &mut line_column,
                    &mut dbg_line,
                ) != 0
                    && !dbg_line.FileName.is_null()
                {
                    file = CStr::from_ptr(dbg_line.FileName as *const std::ffi::c_char)
                        .to_string_lossy()
                        .into_owned();
                    line_number = dbg_line.LineNumber;
                }
            }
        }
        format!("{name}+0x{displacement:x} {file}:{line_number}")
    }
}

#[cfg(not(any(unix, windows)))]
mod symbols {
    pub(super) fn init() {}

    pub(super) fn describe(_address: u64) -> String {
        "???".to_owned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn buckets_and_table() {
        // The bucket is the CRC32 of the pointer's bytes.
        let mem = 0x1234_5678usize as *mut u8;
        let crc = Crc32Context::new().calc(&(mem as usize).to_ne_bytes());
        assert_eq!(get_allocation_bucket(mem), (crc & 0xff) as usize);

        // Tracking and untracking by hand (this test binary doesn't use the
        // TrackingAllocator, so nothing else adds to the table).
        let a = 0x1000usize as *mut u8;
        let b = 0x2000usize as *mut u8;
        track_allocation(a, 10);
        track_allocation(b, 20);
        track_allocation(a, 99); // (already tracked: kept as it is)
        assert_eq!(get_tracked_allocation_size(a), Some(10));
        assert_eq!(get_tracked_allocation_size(b), Some(20));
        let unknown = S_UNKNOWN_FREES.load(Ordering::Relaxed);
        untrack_allocation(a);
        assert!(!is_allocation_tracked(a));
        assert!(is_allocation_tracked(b));
        untrack_allocation(a);
        assert_eq!(S_UNKNOWN_FREES.load(Ordering::Relaxed), unknown + 1);
        untrack_allocation(b);
        assert_eq!(get_tracked_allocation_size(b), None);
    }
}
