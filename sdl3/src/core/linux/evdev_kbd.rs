// Rust translation of src/core/linux/SDL_evdev_kbd.c and SDL_evdev_kbd.h from
// Simple DirectMedia Layer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! The console keyboard of the evdev input reader: text from key codes
//! through the console's keymap (`KDGKBENT` on a virtual console, else the
//! built-in tables of [`super::evdev_kbd_tables`]), the console muted while
//! SDL reads the keyboard (`K_OFF`) and restored at exit or on a fatal
//! signal, and the virtual console switches SDL is asked to handle
//! (`VT_PROCESS`).
//!
//! Upstream's Linux (`SDL_INPUT_LINUXKD`) path; the FreeBSD (`kbio`) one
//! isn't translated.

// This logic is adapted from drivers/tty/vt/keyboard.c in the Linux kernel source

use std::ffi::{c_char, c_int, c_uint, c_void};
use std::sync::atomic::{AtomicBool, AtomicI32, AtomicPtr, Ordering};

use super::evdev_kbd_tables::{DEFAULT_ACCENTS, DEFAULT_KEY_MAPS};
use super::input::{ioctl_int, ioctl_ptr, ioctl_read};
use super::kd::*;
use crate::hints;

/*
 * Handler Tables.
 */

type KHandlerFn = fn(&mut EvdevKeyboardState, u8, bool);
static K_HANDLER: [KHandlerFn; 16] = [
    k_self,
    k_fn,
    k_spec,
    k_pad,
    k_dead,
    k_cons,
    k_cur,
    k_shift,
    k_meta,
    k_ascii,
    k_lock,
    k_lowercase,
    k_slock,
    k_dead2,
    k_brl,
    k_ignore,
];

type FnHandlerFn = fn(&mut EvdevKeyboardState);
#[rustfmt::skip]
static FN_HANDLER: [Option<FnHandlerFn>; 20] = [
    None, Some(fn_enter), None, None,
    None, None, None, Some(fn_caps_toggle),
    Some(fn_num), None, None, None,
    None, Some(fn_caps_on), Some(fn_compose), None,
    None, None, None, Some(fn_num),
];

/// A virtual console switch handler (`SDL_EVDEV_kbd_set_vt_switch_callbacks()`'s
/// function and data pointers, as one closure).
pub(crate) type VtSwitchCallback = Box<dyn FnMut() + Send>;

/*
 * Keyboard State
 */

/// The console's keymaps read before it's muted: by map (`MAX_NR_KEYMAPS`
/// of them, read where there's a built-in table), the keys' `KDGKBENT`
/// values.
type MutedKeyMaps = Vec<Option<Box<[Option<u16>; NR_KEYS]>>>;

/// Translation of `struct SDL_EVDEV_keyboard_state`.
pub(crate) struct EvdevKeyboardState {
    console_fd: c_int,
    muted: bool,
    old_kbd_mode: c_int,
    // Note (upstream): the console's keymaps as they were when it was muted
    // (KDGKBENT of each key of the maps there are built-in tables for;
    // None where it failed), which the keysyms come from while
    // it is. KDGKBENT gives K_HOLE for the Unicode keysyms of a console that
    // isn't in K_UNICODE, as a muted one (K_OFF) isn't: upstream, which
    // asks while muted, makes no text of them (every key past ASCII in a
    // keymap loaded in Unicode, as console-setup's). A keymap loaded while
    // muted isn't seen until the console is muted again.
    muted_key_maps: Option<MutedKeyMaps>,
    key_maps: &'static [Option<&'static [u16; NR_KEYS]>; MAX_NR_KEYMAPS],
    shift_down: [u8; NR_SHIFT], // shift state counters..
    dead_key_next: bool,
    npadch: c_int, // -1 or number assembled on pad
    // Note: upstream points this at the static default table, which
    // KDGKBDIACR then overwrites with the console's (for every later
    // keyboard state too); here each state has its own copy.
    accents: kbdiacrs,
    diacr: c_uint,
    rep: bool, // flag telling character repeat
    lockstate: u8,
    slockstate: u8,
    ledflagstate: u8,
    // (a char upstream, used as the unsigned char of the shift_final sum)
    shift_state: u8,
    text: [u8; 128],
    text_len: c_uint,
    vt_release_callback: Option<VtSwitchCallback>,
    vt_acquire_callback: Option<VtSwitchCallback>,
}

/// Translation of `kbd_cleanup_state`, the keyboard to restore on exit or
/// on a fatal signal: its console and keyboard mode (atomics, which the
/// signal handler can read), while `ACTIVE`.
struct KbdCleanupState {
    active: AtomicBool,
    console_fd: AtomicI32,
    old_kbd_mode: AtomicI32,
}
static KBD_CLEANUP_STATE: KbdCleanupState = KbdCleanupState {
    active: AtomicBool::new(false),
    console_fd: AtomicI32::new(-1),
    old_kbd_mode: AtomicI32::new(0),
};
static KBD_CLEANUP_SIGACTIONS_INSTALLED: AtomicBool = AtomicBool::new(false);
static KBD_CLEANUP_ATEXIT_INSTALLED: AtomicBool = AtomicBool::new(false);

/// `NSIG` (glibc's: `_NSIG` + 1)
const NSIG: usize = if cfg!(any(target_arch = "mips", target_arch = "mips64")) {
    129
} else {
    65
};

/// Translation of `static struct sigaction old_sigaction[NSIG]`: written
/// when the handlers are installed, read by the signal handler.
struct OldSigactions(std::cell::UnsafeCell<[libc::sigaction; NSIG]>);
// SAFETY: as upstream's array: each entry is written by the thread
// installing a handler for that signal before the handler can run, and only
// read afterwards (by the handler, or when restoring it).
unsafe impl Sync for OldSigactions {}
// SAFETY: sigaction is plain data, for which all zeroes is valid.
static OLD_SIGACTION: OldSigactions =
    OldSigactions(std::cell::UnsafeCell::new(unsafe { std::mem::zeroed() }));

/// `&old_sigaction[signum]`
fn old_sigaction(signum: c_int) -> *mut libc::sigaction {
    let index = usize::try_from(signum).expect("a signal number");
    assert!(index < NSIG, "signal {signum} out of range");
    // SAFETY: index is within the array; no reference to it is made.
    unsafe { OLD_SIGACTION.0.get().cast::<libc::sigaction>().add(index) }
}

static FATAL_SIGNALS: [c_int; 9] = [
    // Handlers for SIGTERM and SIGINT are installed in SDL_InitQuit.
    libc::SIGHUP,
    libc::SIGQUIT,
    libc::SIGILL,
    libc::SIGABRT,
    libc::SIGFPE,
    libc::SIGSEGV,
    libc::SIGPIPE,
    libc::SIGBUS,
    libc::SIGSYS,
];

/// Translation of `kbd_cleanup()` (async-signal-safe: atomics and an
/// ioctl).
fn kbd_cleanup() {
    if !KBD_CLEANUP_STATE.active.swap(false, Ordering::SeqCst) {
        return;
    }

    ioctl_int(
        KBD_CLEANUP_STATE.console_fd.load(Ordering::SeqCst),
        KDSKBMODE,
        KBD_CLEANUP_STATE.old_kbd_mode.load(Ordering::SeqCst),
    );
}

/// Translation of `SDL_EVDEV_kbd_reraise_signal()`.
fn reraise_signal(sig: c_int) {
    // SAFETY: raise is async-signal-safe.
    unsafe {
        libc::raise(sig);
    }
}

static SDL_EVDEV_KDB_CLEANUP_SIGINFO: AtomicPtr<libc::siginfo_t> =
    AtomicPtr::new(std::ptr::null_mut());
static SDL_EVDEV_KDB_CLEANUP_UCONTEXT: AtomicPtr<c_void> = AtomicPtr::new(std::ptr::null_mut());

/// Translation of `kbd_cleanup_signal_action()`.
extern "C" fn kbd_cleanup_signal_action(
    signum: c_int,
    info: *mut libc::siginfo_t,
    ucontext: *mut c_void,
) {
    let old_action_p = old_sigaction(signum);

    // SAFETY: sigaction, sigemptyset, sigaddset and sigprocmask are
    // async-signal-safe; old_action_p was filled in when this handler was
    // installed.
    unsafe {
        // Restore original signal handler before going any further.
        libc::sigaction(signum, old_action_p, std::ptr::null_mut());

        // Unmask current signal.
        let mut sigset: libc::sigset_t = std::mem::zeroed();
        libc::sigemptyset(&mut sigset);
        libc::sigaddset(&mut sigset, signum);
        libc::sigprocmask(libc::SIG_UNBLOCK, &sigset, std::ptr::null_mut());
    }

    // Save original signal info and context for archeologists.
    SDL_EVDEV_KDB_CLEANUP_SIGINFO.store(info, Ordering::SeqCst);
    SDL_EVDEV_KDB_CLEANUP_UCONTEXT.store(ucontext, Ordering::SeqCst);

    // Restore keyboard.
    kbd_cleanup();

    // Note (upstream): a fault the kernel signaled (SIGSEGV, SIGBUS, SIGILL
    // or SIGFPE with a positive si_code) isn't raised again: returning runs
    // the faulting instruction again, so the restored handler gets the
    // fault's own siginfo and context. Upstream raises it, which gives that
    // handler raise()'s (SI_TKILL, no fault address): Rust std's
    // SIGSEGV/SIGBUS handler, which tells a stack overflow by the fault's
    // address, then reports none (the process dies of the SIGSEGV), and a
    // crash reporter installed before SDL sees this handler's stack.
    if is_fault(signum, info) {
        return;
    }

    // Reraise signal.
    reraise_signal(signum);
}

/// Whether the signal is a fault the kernel signaled, which the faulting
/// instruction, run again, signals again.
fn is_fault(signum: c_int, info: *const libc::siginfo_t) -> bool {
    matches!(
        signum,
        libc::SIGSEGV | libc::SIGBUS | libc::SIGILL | libc::SIGFPE
    ) && !info.is_null()
        // SAFETY: the kernel's siginfo for this signal.
        && unsafe { (*info).si_code } > 0
}

/// `&kbd_cleanup_signal_action` as a `sa_sigaction`.
fn cleanup_signal_action_address() -> libc::sighandler_t {
    kbd_cleanup_signal_action as extern "C" fn(c_int, *mut libc::siginfo_t, *mut c_void)
        as libc::sighandler_t
}

/// Translation of `kbd_unregister_emerg_cleanup()`.
fn kbd_unregister_emerg_cleanup() {
    KBD_CLEANUP_STATE.active.store(false, Ordering::SeqCst);

    if !KBD_CLEANUP_SIGACTIONS_INSTALLED.swap(false, Ordering::SeqCst) {
        return;
    }

    for &signum in &FATAL_SIGNALS {
        let old_action_p = old_sigaction(signum);

        // SAFETY: sigaction reads or writes the actions given; old_action_p
        // was filled in when the handler was installed.
        unsafe {
            // Examine current signal action
            let mut cur_action: libc::sigaction = std::mem::zeroed();
            if libc::sigaction(signum, std::ptr::null(), &mut cur_action) != 0 {
                continue;
            }

            // Check if action installed and not modified
            if (cur_action.sa_flags & libc::SA_SIGINFO) == 0
                || cur_action.sa_sigaction != cleanup_signal_action_address()
            {
                continue;
            }

            // Restore original action
            libc::sigaction(signum, old_action_p, std::ptr::null_mut());
        }
    }
}

/// Translation of `kbd_cleanup_atexit()`.
extern "C" fn kbd_cleanup_atexit() {
    // Restore keyboard.
    kbd_cleanup();

    // Try to restore signal handlers in case shared library is being unloaded
    kbd_unregister_emerg_cleanup();
}

/// Translation of `kbd_register_emerg_cleanup()`.
fn kbd_register_emerg_cleanup(kbd: &EvdevKeyboardState) {
    if KBD_CLEANUP_STATE.active.load(Ordering::SeqCst) {
        return;
    }
    KBD_CLEANUP_STATE
        .console_fd
        .store(kbd.console_fd, Ordering::SeqCst);
    KBD_CLEANUP_STATE
        .old_kbd_mode
        .store(kbd.old_kbd_mode, Ordering::SeqCst);
    KBD_CLEANUP_STATE.active.store(true, Ordering::SeqCst);

    if !KBD_CLEANUP_ATEXIT_INSTALLED.load(Ordering::SeqCst) {
        /* Since glibc 2.2.3, atexit() (and on_exit(3)) can be used within a shared library to establish
         * functions that are called when the shared library is unloaded.
         * -- man atexit(3)
         */
        // SAFETY: kbd_cleanup_atexit is an extern "C" fn that doesn't unwind.
        unsafe {
            libc::atexit(kbd_cleanup_atexit);
        }
        KBD_CLEANUP_ATEXIT_INSTALLED.store(true, Ordering::SeqCst);
    }

    if KBD_CLEANUP_SIGACTIONS_INSTALLED.swap(true, Ordering::SeqCst) {
        return;
    }

    for &signum in &FATAL_SIGNALS {
        let old_action_p = old_sigaction(signum);
        // SAFETY: sigaction reads the current action into the table's
        // entry and installs the new one; the handler doesn't unwind.
        unsafe {
            if libc::sigaction(signum, std::ptr::null(), old_action_p) != 0 {
                continue;
            }

            /* Skip SIGHUP and SIGPIPE if handler is already installed
             * - assume the handler will do the cleanup
             */
            if (signum == libc::SIGHUP || signum == libc::SIGPIPE)
                && (*old_action_p).sa_sigaction != libc::SIG_DFL
            {
                continue;
            }

            let mut new_action = *old_action_p;
            new_action.sa_flags |= libc::SA_SIGINFO;
            new_action.sa_sigaction = cleanup_signal_action_address();
            libc::sigaction(signum, &new_action, std::ptr::null_mut());
        }
    }
}

const VT_SIGNAL_NONE: c_int = 0;
const VT_SIGNAL_RELEASE: c_int = 1;
const VT_SIGNAL_ACQUIRE: c_int = 2;
static VT_RELEASE_SIGNAL: AtomicI32 = AtomicI32::new(0);
static VT_ACQUIRE_SIGNAL: AtomicI32 = AtomicI32::new(0);
static VT_SIGNAL_PENDING: AtomicI32 = AtomicI32::new(VT_SIGNAL_NONE);

type SignalHandler = extern "C" fn(c_int);

/// Translation of `kbd_vt_release_signal_action()`.
extern "C" fn kbd_vt_release_signal_action(_signum: c_int) {
    VT_SIGNAL_PENDING.store(VT_SIGNAL_RELEASE, Ordering::SeqCst);
}

/// Translation of `kbd_vt_acquire_signal_action()`.
extern "C" fn kbd_vt_acquire_signal_action(_signum: c_int) {
    VT_SIGNAL_PENDING.store(VT_SIGNAL_ACQUIRE, Ordering::SeqCst);
}

/// Translation of `setup_vt_signal()`.
fn setup_vt_signal(signum: c_int, handler: SignalHandler) -> bool {
    // Note (upstream): the old action is saved straight into
    // old_sigaction[signum], so trying a signal that's in use (the release
    // signal, when find_free_signal() looks for the acquire signal) loses
    // the action saved for it, and kbd_vt_quit() leaves the release handler
    // installed (one more signal taken by each SDL_EVDEV_Init()). It's
    // saved there only once the signal is taken here.
    // SAFETY: sigaction installs the new action and saves the old one; the
    // handler only stores an atomic.
    unsafe {
        let mut old_action: libc::sigaction = std::mem::zeroed();
        let mut new_action: libc::sigaction = std::mem::zeroed();
        new_action.sa_sigaction = handler as libc::sighandler_t;
        new_action.sa_flags = libc::SA_RESTART;
        if libc::sigaction(signum, &new_action, &mut old_action) < 0 {
            return false;
        }
        if old_action.sa_sigaction != libc::SIG_DFL {
            // This signal is already in use
            libc::sigaction(signum, &old_action, std::ptr::null_mut());
            return false;
        }
        *old_sigaction(signum) = old_action;
    }
    true
}

/// Translation of `find_free_signal()`.
fn find_free_signal(handler: SignalHandler) -> c_int {
    for i in (libc::SIGRTMIN() + 2)..=libc::SIGRTMAX() {
        if setup_vt_signal(i, handler) {
            return i;
        }
    }
    if setup_vt_signal(libc::SIGUSR1, handler) {
        return libc::SIGUSR1;
    }
    if setup_vt_signal(libc::SIGUSR2, handler) {
        return libc::SIGUSR2;
    }
    0
}

/// Translation of `kbd_vt_quit()`.
fn kbd_vt_quit(console_fd: c_int) {
    for signal in [&VT_RELEASE_SIGNAL, &VT_ACQUIRE_SIGNAL] {
        let signum = signal.swap(0, Ordering::SeqCst);
        if signum != 0 {
            // SAFETY: the table's entry holds the action saved by
            // setup_vt_signal().
            unsafe {
                libc::sigaction(signum, old_sigaction(signum), std::ptr::null_mut());
            }
        }
    }

    let mut mode = vt_mode {
        mode: VT_AUTO,
        ..vt_mode::default()
    };
    // SAFETY: VT_SETMODE reads a vt_mode.
    unsafe {
        ioctl_ptr(console_fd, VT_SETMODE, (&mut mode as *mut vt_mode).cast());
    }
}

/// Translation of `kbd_vt_init()`.
fn kbd_vt_init(console_fd: c_int) -> bool {
    VT_RELEASE_SIGNAL.store(
        find_free_signal(kbd_vt_release_signal_action),
        Ordering::SeqCst,
    );
    VT_ACQUIRE_SIGNAL.store(
        find_free_signal(kbd_vt_acquire_signal_action),
        Ordering::SeqCst,
    );
    let release = VT_RELEASE_SIGNAL.load(Ordering::SeqCst);
    let acquire = VT_ACQUIRE_SIGNAL.load(Ordering::SeqCst);
    if release == 0 || acquire == 0 {
        kbd_vt_quit(console_fd);
        return false;
    }

    let mut mode = vt_mode {
        mode: VT_PROCESS,
        relsig: release as _,
        acqsig: acquire as _,
        frsig: libc::SIGIO as _,
        ..vt_mode::default()
    };
    // SAFETY: VT_SETMODE reads a vt_mode.
    if unsafe { ioctl_ptr(console_fd, VT_SETMODE, (&mut mode as *mut vt_mode).cast()) } < 0 {
        kbd_vt_quit(console_fd);
        return false;
    }
    true
}

/// Translation of `kbd_vt_update()`.
fn kbd_vt_update(state: &mut EvdevKeyboardState) {
    let signal_pending = VT_SIGNAL_PENDING.load(Ordering::SeqCst);
    if signal_pending != VT_SIGNAL_NONE {
        if signal_pending == VT_SIGNAL_RELEASE {
            if let Some(callback) = &mut state.vt_release_callback {
                callback();
            }
            ioctl_int(state.console_fd, VT_RELDISP, 1);
        } else {
            if let Some(callback) = &mut state.vt_acquire_callback {
                callback();
            }
            ioctl_int(state.console_fd, VT_RELDISP, VT_ACKACQ);
        }
        let _ = VT_SIGNAL_PENDING.compare_exchange(
            signal_pending,
            VT_SIGNAL_NONE,
            Ordering::SeqCst,
            Ordering::SeqCst,
        );
    }
}

impl EvdevKeyboardState {
    /// Open the console keyboard (`/dev/tty`, when it's a virtual
    /// console). Translation of `SDL_EVDEV_kbd_init()`.
    pub(crate) fn init() -> EvdevKeyboardState {
        let mut kbd = EvdevKeyboardState {
            console_fd: -1,
            muted: false,
            old_kbd_mode: 0,
            muted_key_maps: None,
            key_maps: &DEFAULT_KEY_MAPS,
            shift_down: [0; NR_SHIFT],
            dead_key_next: false,
            npadch: 0,
            accents: DEFAULT_ACCENTS,
            diacr: 0,
            rep: false,
            lockstate: 0,
            slockstate: 0,
            ledflagstate: 0,
            shift_state: 0,
            text: [0; 128],
            text_len: 0,
            vt_release_callback: None,
            vt_acquire_callback: None,
        };
        let mut flag_state: c_char = 0;
        let mut kbtype: c_char = 0;
        let mut shift_state: [c_char; size_of::<std::ffi::c_long>()] =
            [0; size_of::<std::ffi::c_long>()];
        shift_state[0] = TIOCL_GETSHIFTSTATE;

        // This might fail if we're not connected to a tty (e.g. on the Steam Link)
        // SAFETY: the path is NUL-terminated.
        kbd.console_fd =
            unsafe { libc::open(c"/dev/tty".as_ptr(), libc::O_RDONLY | libc::O_CLOEXEC) };
        if !(ioctl_read(kbd.console_fd, KDGKBTYPE, &mut kbtype) == 0
            && (kbtype == KB_101 || kbtype == KB_84))
        {
            // SAFETY: closing our descriptor (or -1, which fails harmlessly).
            unsafe {
                libc::close(kbd.console_fd);
            }
            kbd.console_fd = -1;
        }

        kbd.npadch = -1;

        if ioctl_read(kbd.console_fd, TIOCLINUX, &mut shift_state) == 0 {
            kbd.shift_state = shift_state[0] as u8;
        }

        if ioctl_read(kbd.console_fd, KDGKBLED, &mut flag_state) == 0 {
            kbd.ledflagstate = flag_state as u8;
        }

        kbd.accents = DEFAULT_ACCENTS;
        kbd.key_maps = &DEFAULT_KEY_MAPS;

        if ioctl_read(kbd.console_fd, KDGKBMODE, &mut kbd.old_kbd_mode) == 0 {
            // Set the keyboard in UNICODE mode and load the keymaps
            ioctl_int(kbd.console_fd, KDSKBMODE, K_UNICODE);
        }

        kbd_vt_init(kbd.console_fd);

        kbd
    }

    /// Mute the console's keyboard while SDL reads the keyboards (or
    /// restore it). Translation of `SDL_EVDEV_kbd_set_muted()`.
    pub(crate) fn set_muted(&mut self, muted: bool) {
        if muted == self.muted {
            return;
        }

        if muted {
            if hints::get_bool(hints::MUTE_CONSOLE_KEYBOARD, true) {
                /* Mute the keyboard so keystrokes only generate evdev events
                 * and do not leak through to the console
                 */
                if self.console_fd >= 0 {
                    self.muted_key_maps = Some(self.read_key_maps());
                }
                ioctl_int(self.console_fd, KDSKBMODE, K_OFF);

                /* Make sure to restore keyboard if application fails to call
                 * SDL_Quit before exit or fatal signal is raised.
                 */
                if !hints::get_bool(hints::NO_SIGNAL_HANDLERS, false) {
                    kbd_register_emerg_cleanup(self);
                }
            }
        } else {
            kbd_unregister_emerg_cleanup();
            self.muted_key_maps = None;

            // Restore the original keyboard mode
            ioctl_int(self.console_fd, KDSKBMODE, self.old_kbd_mode);
        }
        self.muted = muted;
    }

    /// The handlers of virtual console switches away from SDL's console
    /// (`release`) and back (`acquire`), called from the event pump.
    /// Translation of `SDL_EVDEV_kbd_set_vt_switch_callbacks()`.
    pub(crate) fn set_vt_switch_callbacks(
        &mut self,
        release_callback: Option<VtSwitchCallback>,
        acquire_callback: Option<VtSwitchCallback>,
    ) {
        self.vt_release_callback = release_callback;
        self.vt_acquire_callback = acquire_callback;
    }

    /// Handle a pending virtual console switch. Translation of
    /// `SDL_EVDEV_kbd_update()`.
    pub(crate) fn update(&mut self) {
        kbd_vt_update(self);
    }

    /// Restore the console and close it. Translation of
    /// `SDL_EVDEV_kbd_quit()`.
    pub(crate) fn quit(mut self) {
        self.set_muted(false);

        kbd_vt_quit(self.console_fd);

        if self.console_fd >= 0 {
            // SAFETY: closing our descriptor.
            unsafe {
                libc::close(self.console_fd);
            }
            self.console_fd = -1;
        }

        // (the keymaps are always the built-in ones, which aren't freed)
    }

    /// A keyboard state as [`init`](Self::init) makes it without a console
    /// (not reaching for the test's terminal, which may be one).
    #[cfg(test)]
    pub(crate) fn without_console() -> EvdevKeyboardState {
        EvdevKeyboardState {
            console_fd: -1,
            muted: false,
            old_kbd_mode: 0,
            muted_key_maps: None,
            key_maps: &DEFAULT_KEY_MAPS,
            shift_down: [0; NR_SHIFT],
            dead_key_next: false,
            npadch: -1,
            accents: DEFAULT_ACCENTS,
            diacr: 0,
            rep: false,
            lockstate: 0,
            slockstate: 0,
            ledflagstate: 0,
            shift_state: 0,
            text: [0; 128],
            text_len: 0,
            vt_release_callback: None,
            vt_acquire_callback: None,
        }
    }

    /// Whether the console's keyboard is muted.
    #[cfg(test)]
    pub(crate) fn muted(&self) -> bool {
        self.muted
    }
}

/*
 * Helper Functions.
 */

/// Translation of `put_queue()`.
fn put_queue(kbd: &mut EvdevKeyboardState, c: c_uint) {
    // c is already part of a UTF-8 sequence and safe to add as a character
    if (kbd.text_len as usize) < kbd.text.len() - 1 {
        kbd.text[kbd.text_len as usize] = c as u8;
        kbd.text_len += 1;
    }
}

/// Translation of `put_utf8()`.
fn put_utf8(kbd: &mut EvdevKeyboardState, c: c_uint) {
    if c < 0x80 {
        put_queue(kbd, c); /*  0******* */
    } else if c < 0x800 {
        /* 110***** 10****** */
        put_queue(kbd, 0xc0 | (c >> 6));
        put_queue(kbd, 0x80 | (c & 0x3f));
    } else if c < 0x10000 {
        if (0xD800..0xE000).contains(&c) {
            return;
        }
        if c == 0xFFFF {
            return;
        }
        /* 1110**** 10****** 10****** */
        put_queue(kbd, 0xe0 | (c >> 12));
        put_queue(kbd, 0x80 | ((c >> 6) & 0x3f));
        put_queue(kbd, 0x80 | (c & 0x3f));
    } else if c < 0x110000 {
        /* 11110*** 10****** 10****** 10****** */
        put_queue(kbd, 0xf0 | (c >> 18));
        put_queue(kbd, 0x80 | ((c >> 12) & 0x3f));
        put_queue(kbd, 0x80 | ((c >> 6) & 0x3f));
        put_queue(kbd, 0x80 | (c & 0x3f));
    }
}

/*
 * We have a combining character DIACR here, followed by the character CH.
 * If the combination occurs in the table, return the corresponding value.
 * Otherwise, if CH is a space or equals DIACR, return DIACR.
 * Otherwise, conclude that DIACR was not combining after all,
 * queue it and return CH.
 */
/// Translation of `handle_diacr()`.
fn handle_diacr(kbd: &mut EvdevKeyboardState, ch: c_uint) -> c_uint {
    let d = kbd.diacr;

    kbd.diacr = 0;

    if kbd.console_fd >= 0 && ioctl_read(kbd.console_fd, KDGKBDIACR, &mut kbd.accents) < 0 {
        // No worries, we'll use the default accent table
    }

    let count = (kbd.accents.kb_cnt as usize).min(MAX_DIACR);
    for accent in &kbd.accents.kbdiacr[..count] {
        if c_uint::from(accent.diacr) == d && c_uint::from(accent.base) == ch {
            return c_uint::from(accent.result);
        }
    }

    if ch == c_uint::from(b' ') || ch == d {
        return d;
    }

    put_utf8(kbd, d);

    ch
}

/// Translation of `vc_kbd_led()`.
fn vc_kbd_led(kbd: &EvdevKeyboardState, flag: u8) -> bool {
    (kbd.ledflagstate & flag) != 0
}

/// Translation of `set_vc_kbd_led()`.
fn set_vc_kbd_led(kbd: &mut EvdevKeyboardState, flag: u8) {
    kbd.ledflagstate |= flag;
    ioctl_int(kbd.console_fd, KDSETLED, c_int::from(kbd.ledflagstate));
}

/// Translation of `clr_vc_kbd_led()`.
fn clr_vc_kbd_led(kbd: &mut EvdevKeyboardState, flag: u8) {
    kbd.ledflagstate &= !flag;
    ioctl_int(kbd.console_fd, KDSETLED, c_int::from(kbd.ledflagstate));
}

/// `1 << flag`, as the lock states (unsigned chars) keep it. Note
/// (upstream): C's shift is undefined from 32 on; no keymap has such a
/// lock.
fn lock_bit(flag: u8) -> u8 {
    1u32.checked_shl(u32::from(flag)).unwrap_or(0) as u8
}

/// Translation of `chg_vc_kbd_lock()`.
fn chg_vc_kbd_lock(kbd: &mut EvdevKeyboardState, flag: u8) {
    kbd.lockstate ^= lock_bit(flag);
}

/// Translation of `chg_vc_kbd_slock()`.
fn chg_vc_kbd_slock(kbd: &mut EvdevKeyboardState, flag: u8) {
    kbd.slockstate ^= lock_bit(flag);
}

/// Translation of `chg_vc_kbd_led()`.
fn chg_vc_kbd_led(kbd: &mut EvdevKeyboardState, flag: u8) {
    kbd.ledflagstate ^= flag;
    ioctl_int(kbd.console_fd, KDSETLED, c_int::from(kbd.ledflagstate));
}

/*
 * Special function handlers
 */

/// Translation of `fn_enter()`.
fn fn_enter(kbd: &mut EvdevKeyboardState) {
    if kbd.diacr != 0 {
        put_utf8(kbd, kbd.diacr);
        kbd.diacr = 0;
    }
}

/// Translation of `fn_caps_toggle()`.
fn fn_caps_toggle(kbd: &mut EvdevKeyboardState) {
    if kbd.rep {
        return;
    }

    chg_vc_kbd_led(kbd, K_CAPSLOCK);
}

/// Translation of `fn_caps_on()`.
fn fn_caps_on(kbd: &mut EvdevKeyboardState) {
    if kbd.rep {
        return;
    }

    set_vc_kbd_led(kbd, K_CAPSLOCK);
}

/// Translation of `fn_num()`.
fn fn_num(kbd: &mut EvdevKeyboardState) {
    if !kbd.rep {
        chg_vc_kbd_led(kbd, K_NUMLOCK);
    }
}

/// Translation of `fn_compose()`.
fn fn_compose(kbd: &mut EvdevKeyboardState) {
    kbd.dead_key_next = true;
}

/*
 * Special key handlers
 */

/// Translation of `k_ignore()`.
fn k_ignore(_kbd: &mut EvdevKeyboardState, _value: u8, _up_flag: bool) {}

/// Translation of `k_spec()`.
fn k_spec(kbd: &mut EvdevKeyboardState, value: u8, up_flag: bool) {
    if up_flag {
        return;
    }
    if usize::from(value) >= FN_HANDLER.len() {
        return;
    }
    if let Some(handler) = FN_HANDLER[usize::from(value)] {
        handler(kbd);
    }
}

/// Translation of `k_lowercase()`.
fn k_lowercase(_kbd: &mut EvdevKeyboardState, _value: u8, _up_flag: bool) {}

/// Translation of `k_self()`.
fn k_self(kbd: &mut EvdevKeyboardState, value: u8, up_flag: bool) {
    if up_flag {
        return; // no action, if this is a key release
    }

    // (an unsigned char upstream, which handle_diacr()'s result is
    // truncated to)
    let mut value = value;
    if kbd.diacr != 0 {
        value = handle_diacr(kbd, c_uint::from(value)) as u8;
    }

    if kbd.dead_key_next {
        kbd.dead_key_next = false;
        kbd.diacr = c_uint::from(value);
        return;
    }
    put_utf8(kbd, c_uint::from(value));
}

/// Translation of `k_deadunicode()`.
fn k_deadunicode(kbd: &mut EvdevKeyboardState, value: c_uint, up_flag: bool) {
    if up_flag {
        return;
    }

    kbd.diacr = if kbd.diacr != 0 {
        handle_diacr(kbd, value)
    } else {
        value
    };
}

/// Translation of `k_dead()`.
fn k_dead(kbd: &mut EvdevKeyboardState, value: u8, up_flag: bool) {
    let mut ret_diacr = [0u8; NR_DEAD];
    ret_diacr[..6].copy_from_slice(b"`'^~\",");

    // Note (upstream): a dead key past NR_DEAD reads past the array; here
    // it's 0.
    let diacr = ret_diacr.get(usize::from(value)).copied().unwrap_or(0);
    k_deadunicode(kbd, c_uint::from(diacr), up_flag);
}

/// Translation of `k_dead2()`.
fn k_dead2(kbd: &mut EvdevKeyboardState, value: u8, up_flag: bool) {
    k_deadunicode(kbd, c_uint::from(value), up_flag);
}

/// Translation of `k_cons()`.
fn k_cons(_kbd: &mut EvdevKeyboardState, _value: u8, _up_flag: bool) {}

/// Translation of `k_fn()`.
fn k_fn(_kbd: &mut EvdevKeyboardState, _value: u8, _up_flag: bool) {}

/// Translation of `k_cur()`.
fn k_cur(_kbd: &mut EvdevKeyboardState, _value: u8, _up_flag: bool) {}

/// Translation of `k_pad()`.
fn k_pad(kbd: &mut EvdevKeyboardState, value: u8, up_flag: bool) {
    const PAD_CHARS: &[u8] = b"0123456789+-*/\x0d,.?()#\0";

    if up_flag {
        return; // no action, if this is a key release
    }

    if !vc_kbd_led(kbd, K_NUMLOCK) {
        // unprintable action
        return;
    }

    // Note (upstream): a pad key past the table reads past the string;
    // here it's ignored.
    if let Some(&c) = PAD_CHARS.get(usize::from(value)) {
        put_queue(kbd, c_uint::from(c));
    }
}

/// Translation of `k_shift()`.
fn k_shift(kbd: &mut EvdevKeyboardState, value: u8, up_flag: bool) {
    let old_state = kbd.shift_state;

    if kbd.rep {
        return;
    }
    /*
     * Mimic typewriter:
     * a CapsShift key acts like Shift but undoes CapsLock
     */
    let mut value = value;
    if value == kval(K_CAPSSHIFT) {
        value = kval(K_SHIFT);
        if !up_flag {
            clr_vc_kbd_led(kbd, K_CAPSLOCK);
        }
    }

    // Note (upstream): a shift key past NR_SHIFT indexes past the counters;
    // here it's ignored.
    let Some(shift_down) = kbd.shift_down.get_mut(usize::from(value)) else {
        return;
    };
    if up_flag {
        /*
         * handle the case that two shift or control
         * keys are depressed simultaneously
         */
        if *shift_down != 0 {
            *shift_down -= 1;
        }
    } else {
        *shift_down = shift_down.wrapping_add(1);
    }

    if *shift_down != 0 {
        kbd.shift_state |= lock_bit(value);
    } else {
        kbd.shift_state &= !lock_bit(value);
    }

    // kludge
    if up_flag && kbd.shift_state != old_state && kbd.npadch != -1 {
        put_utf8(kbd, kbd.npadch as c_uint);
        kbd.npadch = -1;
    }
}

/// Translation of `k_meta()`.
fn k_meta(_kbd: &mut EvdevKeyboardState, _value: u8, _up_flag: bool) {}

/// Translation of `k_ascii()`.
fn k_ascii(kbd: &mut EvdevKeyboardState, value: u8, up_flag: bool) {
    if up_flag {
        return;
    }

    let mut value = value;
    let base = if value < 10 {
        // decimal input of code, while Alt depressed
        10
    } else {
        // hexadecimal input of code, while AltGr depressed
        value -= 10;
        16
    };

    if kbd.npadch == -1 {
        kbd.npadch = c_int::from(value);
    } else {
        kbd.npadch = kbd
            .npadch
            .wrapping_mul(base)
            .wrapping_add(c_int::from(value));
    }
}

/// Translation of `k_lock()`.
fn k_lock(kbd: &mut EvdevKeyboardState, value: u8, up_flag: bool) {
    if up_flag || kbd.rep {
        return;
    }

    chg_vc_kbd_lock(kbd, value);
}

/// Translation of `k_slock()`.
fn k_slock(kbd: &mut EvdevKeyboardState, value: u8, up_flag: bool) {
    k_shift(kbd, value, up_flag);
    if up_flag || kbd.rep {
        return;
    }

    chg_vc_kbd_slock(kbd, value);
    // try to make Alt, oops, AltGr and such work
    if kbd.key_maps[usize::from(kbd.lockstate ^ kbd.slockstate)].is_none() {
        kbd.slockstate = 0;
        chg_vc_kbd_slock(kbd, value);
    }
}

/// Translation of `k_brl()`.
fn k_brl(_kbd: &mut EvdevKeyboardState, _value: u8, _up_flag: bool) {}

impl EvdevKeyboardState {
    /// The console's keymaps there are built-in tables for, read
    /// (`KDGKBENT`) before it's muted (see `muted_key_maps`).
    fn read_key_maps(&self) -> MutedKeyMaps {
        let mut maps: MutedKeyMaps = vec![None; MAX_NR_KEYMAPS];
        for (table, map) in self.key_maps.iter().enumerate() {
            if map.is_none() {
                continue;
            }
            let mut entries = Box::new([None; NR_KEYS]);
            for (index, entry) in entries.iter_mut().enumerate() {
                let mut kbe = kbentry {
                    kb_table: table as u8,
                    kb_index: index as u8,
                    kb_value: 0,
                };
                if ioctl_read(self.console_fd, KDGKBENT, &mut kbe) == 0 {
                    *entry = Some(kbe.kb_value);
                }
            }
            maps[table] = Some(entries);
        }
        maps
    }

    /// The keysym of a key in a keymap: the console's (`KDGKBENT`; while
    /// it's muted, as it was then), or the built-in table's without one.
    fn keysym(&self, key_map: &[u16; NR_KEYS], shift_final: u8, keycode: usize) -> Option<u16> {
        if self.console_fd < 0 {
            Some(key_map[keycode])
        } else if let Some(maps) = &self.muted_key_maps {
            maps[usize::from(shift_final)].as_ref()?[keycode].map(|value| value ^ 0xf000)
        } else {
            let mut kbe = kbentry {
                kb_table: shift_final,
                kb_index: keycode as u8,
                kb_value: 0,
            };
            if ioctl_read(self.console_fd, KDGKBENT, &mut kbe) == 0 {
                Some(kbe.kb_value ^ 0xf000)
            } else {
                None
            }
        }
    }

    /// Turn a key press (`down` 1), repeat (2) or release (0) into text
    /// (sent as `SDL_EVENT_TEXT_INPUT`) and console keyboard state.
    /// Translation of `SDL_EVDEV_kbd_keycode()`.
    pub(crate) fn keycode(&mut self, keycode: c_uint, down: c_int) {
        self.rep = down == 2;

        let mut shift_final = (self.shift_state | self.slockstate) ^ self.lockstate;
        let Some(mut key_map) = self.key_maps[usize::from(shift_final)] else {
            // Unsupported shift state (e.g. ctrl = 4, alt = 8), just reset to the default state
            self.shift_state = 0;
            self.slockstate = 0;
            self.lockstate = 0;
            return;
        };

        if keycode as usize >= NR_KEYS {
            return;
        }
        let Some(mut keysym) = self.keysym(key_map, shift_final, keycode as usize) else {
            return;
        };

        let mut ty = ktyp(keysym);

        if ty < 0xf0 {
            if down != 0 {
                put_utf8(self, c_uint::from(keysym));
            }
        } else {
            ty -= 0xf0;

            // if type is KT_LETTER then it can be affected by Caps Lock
            if ty == KT_LETTER {
                ty = KT_LATIN;

                if vc_kbd_led(self, K_CAPSLOCK) {
                    shift_final ^= 1 << KG_SHIFT;
                    if let Some(map) = self.key_maps[usize::from(shift_final)] {
                        key_map = map;
                        if let Some(sym) = self.keysym(key_map, shift_final, keycode as usize) {
                            keysym = sym;
                        }
                    }
                }
            }

            K_HANDLER[usize::from(ty)](self, kval(keysym), down == 0);

            if ty != KT_SLOCK {
                self.slockstate = 0;
            }
        }

        if self.text_len > 0 {
            self.text[self.text_len as usize] = 0;
            // (a C string: up to its first NUL)
            let len = self.text.iter().position(|&b| b == 0).unwrap_or(0);
            crate::events::keyboard::send_keyboard_text(&String::from_utf8_lossy(
                &self.text[..len],
            ));
            self.text_len = 0;
        }
    }
}

#[cfg(test)]
mod tests;
