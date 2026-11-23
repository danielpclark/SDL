//! The appnotify registrations through windows-sys's static imports
//! (api-ms-win-core-psm-appnotify-l1-1-0 for the app state pair, -l1-1-1
//! for the constrained pair): if an import doesn't resolve, the program
//! doesn't start at all, which is the question for Wine.

#[cfg(windows)]
fn main() {
    use std::ffi::c_void;
    use windows_sys::Win32::UI::Shell::{
        RegisterAppConstrainedChangeNotification, RegisterAppStateChangeNotification,
        UnregisterAppConstrainedChangeNotification, UnregisterAppStateChangeNotification,
        PAPPCONSTRAIN_REGISTRATION, PAPPSTATE_REGISTRATION,
    };

    unsafe extern "system" fn on_state(quiesced: bool, _context: *const c_void) {
        println!("  app state callback: quiesced = {quiesced}");
    }
    unsafe extern "system" fn on_constrained(constrained: bool, _context: *const c_void) {
        println!("  constrained callback: constrained = {constrained}");
    }

    println!("== RegisterAppStateChangeNotification");
    let mut reg: PAPPSTATE_REGISTRATION = 0;
    // SAFETY: a callback that lives as long as the program, no context.
    let r =
        unsafe { RegisterAppStateChangeNotification(Some(on_state), std::ptr::null(), &mut reg) };
    println!("result = {r} (0x{r:08x}), registration = 0x{reg:x}");
    if r == 0 && reg != 0 {
        // SAFETY: the registration made above.
        unsafe { UnregisterAppStateChangeNotification(reg) };
        println!("UnregisterAppStateChangeNotification: returned");
    }

    println!("== RegisterAppConstrainedChangeNotification");
    let mut creg: PAPPCONSTRAIN_REGISTRATION = 0;
    // SAFETY: as above.
    let r = unsafe {
        RegisterAppConstrainedChangeNotification(Some(on_constrained), std::ptr::null(), &mut creg)
    };
    println!("result = {r} (0x{r:08x}), registration = 0x{creg:x}");
    if r == 0 {
        // (as upstream's GDK_UnregisterChangeNotifications() does, whatever
        // the registration is, a null one too)
        // SAFETY: the registration made above.
        unsafe { UnregisterAppConstrainedChangeNotification(creg) };
        println!("UnregisterAppConstrainedChangeNotification(0x{creg:x}): returned");
    }
    println!("== UnregisterAppStateChangeNotification(0)");
    // SAFETY: a null registration (what upstream's static holds when the
    // registration failed or never ran).
    unsafe { UnregisterAppStateChangeNotification(0) };
    println!("returned");
}

#[cfg(not(windows))]
fn main() {}
