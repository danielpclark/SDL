// Rust translation of src/events/SDL_quit.c from Simple DirectMedia Layer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! General quit handling code for SDL: SIGINT and SIGTERM become a `QUIT`
//! event the next time the event loop pumps, unless
//! `SDL_HINT_NO_SIGNAL_HANDLERS` is set or the app installed its own
//! handlers first.

#[cfg(unix)]
mod sig {
    use std::sync::atomic::{AtomicBool, Ordering};

    static DISABLE_SIGNALS: AtomicBool = AtomicBool::new(false);
    pub(super) static SEND_QUIT_PENDING: AtomicBool = AtomicBool::new(false);

    extern "C" fn handle_sig(sig: libc::c_int) {
        // Send a quit event next time the event loop pumps.
        // We can't send it in signal handler; SDL_malloc() might be interrupted!
        if sig == libc::SIGINT || sig == libc::SIGTERM {
            SEND_QUIT_PENDING.store(true, Ordering::SeqCst);
        }
    }

    fn handler_address() -> libc::sighandler_t {
        handle_sig as extern "C" fn(libc::c_int) as libc::sighandler_t
    }

    /// Translation of `SDL_EventSignal_Init()`: install the handler only if
    /// the signal still has its default action.
    fn event_signal_init(sig: libc::c_int) {
        // SAFETY: sigaction with a NULL new action only reads the current
        // one into a zeroed struct; the handler installed is async-signal-
        // safe (one atomic store).
        unsafe {
            let mut action: libc::sigaction = std::mem::zeroed();
            libc::sigaction(sig, std::ptr::null(), &mut action);
            if action.sa_sigaction == libc::SIG_DFL {
                action.sa_sigaction = handler_address();
                libc::sigaction(sig, &action, std::ptr::null_mut());
            }
        }
    }

    /// Translation of `SDL_EventSignal_Quit()`: restore the default action
    /// if our handler is still installed.
    fn event_signal_quit(sig: libc::c_int) {
        // SAFETY: as above.
        unsafe {
            let mut action: libc::sigaction = std::mem::zeroed();
            libc::sigaction(sig, std::ptr::null(), &mut action);
            if action.sa_sigaction == handler_address() {
                action.sa_sigaction = libc::SIG_DFL;
                libc::sigaction(sig, &action, std::ptr::null_mut());
            }
        }
    }

    /// Translation of `SDL_QuitInit_Internal()`.
    pub(super) fn quit_init_internal() {
        // Both SIGINT and SIGTERM are translated into quit interrupts
        event_signal_init(libc::SIGINT);
        event_signal_init(libc::SIGTERM);
    }

    /// Translation of `SDL_QuitQuit_Internal()`.
    pub(super) fn quit_quit_internal() {
        if !DISABLE_SIGNALS.load(Ordering::Relaxed) {
            event_signal_quit(libc::SIGINT);
            event_signal_quit(libc::SIGTERM);
        }
    }
}

/// Install the signal handlers unless `SDL_HINT_NO_SIGNAL_HANDLERS` is set.
/// Translation of `SDL_InitQuit()`.
pub(crate) fn init_quit() {
    #[cfg(unix)]
    if !crate::hints::get_bool(crate::hints::NO_SIGNAL_HANDLERS, false) {
        sig::quit_init_internal();
    }
}

/// Remove the signal handlers. Translation of `SDL_QuitQuit()`.
pub(crate) fn quit_quit() {
    #[cfg(unix)]
    sig::quit_quit_internal();
}

/// Post the `QUIT` event a signal asked for. Translation of
/// `SDL_SendPendingSignalEvents()`.
pub(crate) fn send_pending_signal_events() {
    #[cfg(unix)]
    if sig::SEND_QUIT_PENDING.load(std::sync::atomic::Ordering::SeqCst) {
        send_quit();
        debug_assert!(!sig::SEND_QUIT_PENDING.load(std::sync::atomic::Ordering::SeqCst));
    }
}

/// Post a `QUIT` event. Translation of `SDL_SendQuit()`.
pub fn send_quit() {
    #[cfg(unix)]
    sig::SEND_QUIT_PENDING.store(false, std::sync::atomic::Ordering::SeqCst);
    super::queue::send_app_event(super::EventType::QUIT);
}

#[cfg(all(test, unix))]
mod tests {
    use crate::events::{self, Event, EventType};
    use crate::init::{self, InitFlags};

    #[test]
    fn sigterm_becomes_a_quit_event() {
        let _l = crate::test_support::test_lock();
        crate::hints::reset(crate::hints::NO_SIGNAL_HANDLERS);
        init::init(InitFlags::EVENTS).unwrap();
        events::flush_events(EventType::FIRST, EventType::LAST);

        // SAFETY: reads the current action only.
        let installed = unsafe {
            let mut action: libc::sigaction = std::mem::zeroed();
            libc::sigaction(libc::SIGTERM, std::ptr::null(), &mut action);
            action.sa_sigaction
        };
        // (never raise SIGTERM without our handler: it would end the test run)
        assert_ne!(installed, libc::SIG_DFL, "the handler is installed");
        // SAFETY: raise() delivers SIGTERM to this thread, whose handler is
        // ours (checked above) and only sets a flag.
        assert_eq!(unsafe { libc::raise(libc::SIGTERM) }, 0);
        events::pump();
        let mut got_quit = false;
        while let Some(e) = events::poll() {
            got_quit |= matches!(e, Event::Quit(_));
        }
        assert!(got_quit);

        init::quit_subsystem(InitFlags::EVENTS);
        // The default action is back.
        // SAFETY: reads the current action only.
        unsafe {
            let mut action: libc::sigaction = std::mem::zeroed();
            libc::sigaction(libc::SIGTERM, std::ptr::null(), &mut action);
            assert_eq!(action.sa_sigaction, libc::SIG_DFL);
        }

        // With the hint, no handler is installed.
        crate::hints::set(crate::hints::NO_SIGNAL_HANDLERS, "1").unwrap();
        init::init(InitFlags::EVENTS).unwrap();
        // SAFETY: as above.
        unsafe {
            let mut action: libc::sigaction = std::mem::zeroed();
            libc::sigaction(libc::SIGTERM, std::ptr::null(), &mut action);
            assert_eq!(action.sa_sigaction, libc::SIG_DFL);
        }
        init::quit_subsystem(InitFlags::EVENTS);
        crate::hints::reset(crate::hints::NO_SIGNAL_HANDLERS);
    }
}
