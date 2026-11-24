//! The WinGDK core's checks: without the Gaming Runtime (Wine, the CI
//! runners) the task queue and the default user fail as upstream's do, and
//! the notifications register (on Windows) or fail (Wine's are stubs); with
//! the runtime, the hardware check.

use super::*;
use crate::test_support::{skip, test_lock};
use std::sync::mpsc;
use std::time::Duration;

#[test]
fn the_task_queue_and_the_default_user_fail_without_the_runtime() {
    let _l = test_lock();
    if runtime().is_ok() {
        eprintln!("note: the Gaming Runtime is here: hardware_gdk_runtime checks it");
        return;
    }

    for _ in 0..2 {
        assert_eq!(
            get_task_queue().unwrap_err().to_string(),
            "[GDK] Could not create global task queue"
        );
        assert_eq!(
            crate::gdk::task_queue().unwrap_err().to_string(),
            "[GDK] Could not create global task queue"
        );
    }
    // (no queue was made: nothing to dispatch)
    assert_eq!(*global_task_queue(), 0);
    dispatch_task_queue();
    quit_task_queue();

    // E_GAMERUNTIME_DLL_NOT_FOUND has no system message
    let e = get_default_user().unwrap_err().to_string();
    assert!(e.starts_with("XUserAddAsync:"), "{e}");
    let e = crate::gdk::default_user().unwrap_err().to_string();
    assert!(e.starts_with("XUserAddAsync:"), "{e}");
}

#[test]
fn the_notifications_register_and_unregister() {
    let _l = test_lock();
    let result = register_change_notifications();
    if crate::core::windows::is_wine() {
        // Wine's RegisterAppStateChangeNotification() is a stub
        // (ERROR_CALL_NOT_IMPLEMENTED, a null registration)
        assert_eq!(
            result.unwrap_err().to_string(),
            "[GDK] Unable to call RegisterAppStateChangeNotification"
        );
        assert_eq!(change_notifications().plm, 0);
    } else {
        // (an unpackaged program's constrained registration is null on
        // Windows Server, the CI runners')
        result.unwrap();
        assert_ne!(change_notifications().plm, 0);
    }
    assert_ne!(change_notifications().suspend_complete, 0);
    unregister_change_notifications();
    // (the closed event isn't left set: nothing to signal)
    assert_eq!(change_notifications().suspend_complete, 0);
    suspend_complete();

    // the registrations are cleared, and a null one isn't unregistered
    // (UnregisterAppStateChangeNotification(NULL) crashes on Windows): a
    // second teardown does nothing
    assert_eq!(change_notifications().plm, 0);
    assert_eq!(change_notifications().cplm, 0);
    unregister_change_notifications();
}

#[test]
fn the_suspend_handler_waits_for_suspend_complete() {
    let _l = test_lock();
    // (the event is made before the registrations, which may fail)
    let _ = register_change_notifications();

    let (tx, rx) = mpsc::channel();
    let tx = Mutex::new(tx);
    let _watch = crate::events::queue::add_watch(move |event| {
        let t = event.event_type();
        if t == EventType::DID_ENTER_BACKGROUND || t == EventType::WILL_ENTER_FOREGROUND {
            let _ = tx.lock().unwrap().send(t);
        }
    });

    // resumed: the event, no wait
    // SAFETY: the handler takes any value of its arguments.
    unsafe { app_state_change(false, std::ptr::null()) };
    assert_eq!(rx.recv().unwrap(), EventType::WILL_ENTER_FOREGROUND);

    // suspended: the event, then a wait, on Windows' thread, for
    // suspend_complete(), which doesn't wait on the handler
    let (done_tx, done_rx) = mpsc::channel();
    let handler = std::thread::spawn(move || {
        // SAFETY: as above.
        unsafe { app_state_change(true, std::ptr::null()) };
        done_tx.send(()).unwrap();
    });
    assert_eq!(rx.recv().unwrap(), EventType::DID_ENTER_BACKGROUND);
    assert!(done_rx.recv_timeout(Duration::from_millis(200)).is_err());
    suspend_complete();
    done_rx.recv_timeout(Duration::from_secs(10)).unwrap();
    handler.join().unwrap();

    unregister_change_notifications();
    // a late suspension has no event to wait on: it doesn't wait
    let (done_tx, done_rx) = mpsc::channel();
    let handler = std::thread::spawn(move || {
        // SAFETY: as above.
        unsafe { app_state_change(true, std::ptr::null()) };
        done_tx.send(()).unwrap();
    });
    assert_eq!(rx.recv().unwrap(), EventType::DID_ENTER_BACKGROUND);
    done_rx.recv_timeout(Duration::from_secs(10)).unwrap();
    handler.join().unwrap();
}

/// The Gaming Runtime's paths, on a machine that has it (Gaming Services):
/// the runtime initialized, SDL's global task queue made, given out,
/// dispatched, given out again once the references are closed, quit and
/// made again, and the default user (which may show the sign-in UI).
#[test]
#[ignore = "hardware: needs the Gaming Runtime (Gaming Services)"]
fn hardware_gdk_runtime() {
    let _l = test_lock();
    let runtime = match runtime() {
        Ok(runtime) => runtime,
        Err(e) => {
            skip(
                "gdk-runtime",
                format_args!("no usable Gaming Runtime here ({e})"),
            );
            return;
        }
    };
    // SAFETY: no preconditions; undone below.
    let hr = unsafe { (runtime.XGameRuntimeInitialize)() };
    println!("XGameRuntimeInitialize: 0x{:08X}", hr as u32);
    assert!(
        hr >= 0,
        "XGameRuntimeInitialize failed: 0x{:08X}",
        hr as u32
    );

    let first = crate::gdk::task_queue().unwrap();
    let second = crate::gdk::task_queue().unwrap();
    println!("task queues: {:p}, {:p}", first.as_raw(), second.as_raw());
    drop(first);
    drop(second);
    dispatch_task_queue();
    // (the global queue is still open: SDL's reference)
    let third = crate::gdk::task_queue().unwrap();
    dispatch_task_queue();
    // (terminated and closed; the next call makes a new queue, while the
    // reference given out stays open until it's dropped)
    quit_task_queue();
    assert_eq!(*global_task_queue(), 0);
    drop(third);
    let fourth = crate::gdk::task_queue().unwrap();
    dispatch_task_queue();
    drop(fourth);
    quit_task_queue();

    match crate::gdk::default_user() {
        Ok(user) => println!("default user: {:p}", user.as_raw()),
        Err(e) => println!("no default user: {e}"),
    }

    // SAFETY: initialized above.
    unsafe { (runtime.XGameRuntimeUninitialize)() };
}
