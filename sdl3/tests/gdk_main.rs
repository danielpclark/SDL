//! `run_app()` with the `gdk` feature (upstream's WinGDK `SDL_RunApp()`), as
//! a program's own entry point: without the Gaming Runtime (the CI runners
//! and Wine have no `xgameruntime.dll`), it shows upstream's "Fatal Error"
//! message box, "[GDK] Gaming Runtime library not found (xgameruntime.dll)",
//! and returns -1 without calling the main function. A thread of the
//! program finds the box among the process's windows, reads it and presses
//! its button. With the runtime (Gaming Services), the main function runs
//! and its result comes back.
//!
//! A program of its own (no test harness): `run_app()` sets up
//! process-wide state (the process task queue, the application-state
//! notifications). Built with the feature only; nothing to do but on
//! Windows.

#[cfg(all(windows, feature = "gdk"))]
mod gdk {
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::{mpsc, Arc};
    use std::time::{Duration, Instant};

    use windows_sys::core::BOOL;
    use windows_sys::Win32::Foundation::{FreeLibrary, HWND, LPARAM, WPARAM};
    use windows_sys::Win32::System::LibraryLoader::LoadLibraryW;
    use windows_sys::Win32::System::Threading::GetCurrentProcessId;
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        EnumChildWindows, EnumWindows, GetClassNameW, GetWindowTextW, GetWindowThreadProcessId,
        IsWindowVisible, PostMessageW, IDOK, WM_COMMAND,
    };

    /// The upstream message box's title and message without the runtime.
    const TITLE: &str = "Fatal Error";
    const MESSAGE: &str = "[GDK] Gaming Runtime library not found (xgameruntime.dll)";

    fn wide(s: &str) -> Vec<u16> {
        s.encode_utf16().chain(Some(0)).collect()
    }

    fn window_text(hwnd: HWND) -> String {
        let mut buffer = [0u16; 512];
        // SAFETY: buffer holds the given number of units.
        let n = unsafe { GetWindowTextW(hwnd, buffer.as_mut_ptr(), buffer.len() as i32) };
        String::from_utf16_lossy(&buffer[..n.max(0) as usize])
    }

    fn class_name(hwnd: HWND) -> String {
        let mut buffer = [0u16; 256];
        // SAFETY: as above.
        let n = unsafe { GetClassNameW(hwnd, buffer.as_mut_ptr(), buffer.len() as i32) };
        String::from_utf16_lossy(&buffer[..n.max(0) as usize])
    }

    /// The visible top-level windows of this process.
    fn process_windows() -> Vec<HWND> {
        unsafe extern "system" fn each(hwnd: HWND, lparam: LPARAM) -> BOOL {
            // SAFETY: lparam is the Vec below, for the call's duration.
            let found = unsafe { &mut *(lparam as *mut Vec<HWND>) };
            let mut pid = 0;
            // SAFETY: hwnd is a window EnumWindows gave; pid a valid out pointer.
            unsafe { GetWindowThreadProcessId(hwnd, &mut pid) };
            // SAFETY: no preconditions.
            if pid == unsafe { GetCurrentProcessId() } && unsafe { IsWindowVisible(hwnd) } != 0 {
                found.push(hwnd);
            }
            1
        }
        let mut found: Vec<HWND> = Vec::new();
        // SAFETY: the callback only uses found, which outlives the call.
        unsafe { EnumWindows(Some(each), &mut found as *mut Vec<HWND> as LPARAM) };
        found
    }

    /// The texts of a window's children (the message box's static text and
    /// button).
    fn child_texts(hwnd: HWND) -> Vec<(String, String)> {
        unsafe extern "system" fn each(child: HWND, lparam: LPARAM) -> BOOL {
            // SAFETY: lparam is the Vec below, for the call's duration.
            let found = unsafe { &mut *(lparam as *mut Vec<(String, String)>) };
            found.push((class_name(child), window_text(child)));
            1
        }
        let mut found: Vec<(String, String)> = Vec::new();
        // SAFETY: as above.
        unsafe {
            EnumChildWindows(
                hwnd,
                Some(each),
                &mut found as *mut Vec<(String, String)> as LPARAM,
            )
        };
        found
    }

    /// A box seen: its title and its children's (class, text).
    type Seen = (String, Vec<(String, String)>);

    /// Watch for a message box titled [`TITLE`] until `stop`; read it and
    /// press its button (`IDOK`, the return key's).
    fn dismiss_message_boxes(stop: Arc<AtomicBool>, seen: mpsc::Sender<Seen>) {
        let deadline = Instant::now() + Duration::from_secs(60);
        while !stop.load(Ordering::Acquire) && Instant::now() < deadline {
            for hwnd in process_windows() {
                if window_text(hwnd) == TITLE {
                    // (the text controls are made with the dialog; read once
                    // shown)
                    std::thread::sleep(Duration::from_millis(200));
                    let _ = seen.send((window_text(hwnd), child_texts(hwnd)));
                    // SAFETY: hwnd is a window of this process.
                    unsafe { PostMessageW(hwnd, WM_COMMAND, IDOK as WPARAM, 0) };
                    std::thread::sleep(Duration::from_millis(500));
                }
            }
            std::thread::sleep(Duration::from_millis(50));
        }
    }

    /// Whether the Gaming Runtime's library loads here.
    fn runtime_library_loads() -> bool {
        let name = wide("xgameruntime.dll");
        // SAFETY: name is NUL-terminated.
        let module = unsafe { LoadLibraryW(name.as_ptr()) };
        if module.is_null() {
            return false;
        }
        // SAFETY: loaded above, freed once.
        unsafe { FreeLibrary(module) };
        true
    }

    pub(super) fn main() {
        let with_runtime = runtime_library_loads();
        println!(
            "gdk_main: xgameruntime.dll {}",
            if with_runtime { "loads" } else { "isn't here" }
        );

        let stop = Arc::new(AtomicBool::new(false));
        let (seen_tx, seen_rx) = mpsc::channel();
        let watcher = {
            let stop = stop.clone();
            std::thread::spawn(move || dismiss_message_boxes(stop, seen_tx))
        };

        let mut called = false;
        let result = sdl3::app::run_app(|args| {
            called = true;
            println!("gdk_main: main function {args:?}");
            7
        });
        stop.store(true, Ordering::Release);
        watcher.join().unwrap();
        let seen: Vec<Seen> = seen_rx.try_iter().collect();
        println!("gdk_main: run_app returned {result}; message boxes: {seen:?}");

        if with_runtime {
            // (the library loads: whatever follows is run_app()'s, and a
            // runtime that can't be used shows a box; see the hardware
            // check hardware_gdk_run_app for the paths)
            if seen.is_empty() {
                assert!(called, "the main function didn't run");
                assert_eq!(result, 7);
            } else {
                assert!(!called);
                assert_eq!(result, -1);
            }
            println!("gdk_main: ok (with the runtime's library)");
            return;
        }

        assert_eq!(result, -1);
        assert!(!called, "the main function ran without the runtime");
        assert_eq!(seen.len(), 1, "one message box: {seen:?}");
        let (title, children) = &seen[0];
        assert_eq!(title, TITLE);
        assert!(
            children.iter().any(|(_, text)| text == MESSAGE),
            "the message box says {MESSAGE:?}: {children:?}"
        );
        println!("gdk_main: ok (no runtime: upstream's message box, -1)");
    }
}

fn main() {
    #[cfg(all(windows, feature = "gdk"))]
    gdk::main();
    #[cfg(not(all(windows, feature = "gdk")))]
    println!("gdk_main: WinGDK's run_app() is Windows' (with the gdk feature): nothing to do");
}
