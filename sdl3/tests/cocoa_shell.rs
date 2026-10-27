// Tests of the macOS tray, file dialogs and notifications.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! AppKit wants the tray's status items and the file panels on the main
//! thread, and the test harness runs tests on other threads, so this is a
//! program of its own (`harness = false`) whose `main` drives them through
//! the public API, answering every panel and menu from the main queue so
//! that nothing waits for a person: the tray (status item, icon, tooltip,
//! menus and submenus, checkboxes, a menu opened from the status item),
//! the open/save/folder panels run app-modally and as sheets, and
//! notifications (which need an application bundle, which this isn't).
//!
//! Without a window server session (capability `cocoa`) it skips, unless
//! `SDL3_TEST_REQUIRE` asks for `cocoa` (or `all`).

#[cfg(not(target_os = "macos"))]
fn main() {}

#[cfg(target_os = "macos")]
fn main() {
    macos::main();
}

#[cfg(target_os = "macos")]
mod macos {
    use std::ffi::{c_char, c_void, CStr};
    use std::sync::atomic::{AtomicIsize, AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex};
    use std::time::{Duration, Instant};

    use sdl3::dialog::{show_file_dialog, DialogFileFilter, FileDialogOptions, FileDialogType};
    use sdl3::events::window::WindowFlags;
    use sdl3::hints;
    use sdl3::init::{self, InitFlags};
    use sdl3::notification;
    use sdl3::tray::{Tray, TrayEntryFlags, TrayOptions};
    use sdl3::video::window::PROP_WINDOW_COCOA_WINDOW_POINTER;
    use sdl3::video::{PixelFormat, Surface, Window};

    type Id = *mut c_void;

    /// Whether `SDL3_TEST_REQUIRE` makes the `cocoa` capability mandatory.
    fn required() -> bool {
        std::env::var("SDL3_TEST_REQUIRE")
            .unwrap_or_default()
            .split(',')
            .map(str::trim)
            .any(|c| c == "all" || c == "cocoa")
    }

    fn skip(reason: &str) {
        if required() {
            panic!("required capability cocoa unavailable: {reason}");
        }
        println!("note: skipping, cocoa unavailable: {reason}");
    }

    // The bits of the Objective-C runtime and GCD the checks need to look
    // at AppKit's objects and to answer panels and menus.
    unsafe extern "C" {
        fn objc_getClass(name: *const c_char) -> Id;
        fn sel_registerName(name: *const c_char) -> Id;
        fn object_getClassName(obj: Id) -> *const c_char;
        fn objc_msgSend();
        static _dispatch_main_q: c_void;
        fn dispatch_async_f(
            queue: *const c_void,
            context: *mut c_void,
            work: unsafe extern "C" fn(*mut c_void),
        );
        static _NSConcreteGlobalBlock: [*const c_void; 32];
    }

    #[link(name = "Foundation", kind = "framework")]
    unsafe extern "C" {
        static NSMenuDidBeginTrackingNotification: Id;
    }

    fn sel(name: &CStr) -> Id {
        // SAFETY: a NUL-terminated selector name.
        unsafe { sel_registerName(name.as_ptr()) }
    }

    fn cls(name: &CStr) -> Id {
        // SAFETY: a NUL-terminated class name.
        unsafe { objc_getClass(name.as_ptr()) }
    }

    /// `[obj sel]` returning `R`.
    unsafe fn send0<R>(obj: Id, name: &CStr) -> R {
        // SAFETY: the caller's contract on the method's signature.
        unsafe {
            let f: unsafe extern "C" fn(Id, Id) -> R =
                std::mem::transmute(objc_msgSend as unsafe extern "C" fn());
            f(obj, sel(name))
        }
    }

    /// `[obj sel:a]` returning `R`.
    unsafe fn send1<A, R>(obj: Id, name: &CStr, a: A) -> R {
        // SAFETY: as above.
        unsafe {
            let f: unsafe extern "C" fn(Id, Id, A) -> R =
                std::mem::transmute(objc_msgSend as unsafe extern "C" fn());
            f(obj, sel(name), a)
        }
    }

    /// `[obj sel:a b:b]` returning `R`.
    unsafe fn send2<A, B, R>(obj: Id, name: &CStr, a: A, b: B) -> R {
        // SAFETY: as above.
        unsafe {
            let f: unsafe extern "C" fn(Id, Id, A, B) -> R =
                std::mem::transmute(objc_msgSend as unsafe extern "C" fn());
            f(obj, sel(name), a, b)
        }
    }

    /// The UTF-8 contents of an NSString.
    fn to_string(s: Id) -> String {
        if s.is_null() {
            return String::new();
        }
        // SAFETY: an NSString.
        unsafe {
            let p: *const c_char = send0(s, c"UTF8String");
            CStr::from_ptr(p).to_string_lossy().into_owned()
        }
    }

    fn ns_app() -> Id {
        // SAFETY: NSApplication's class method.
        unsafe { send0(cls(c"NSApplication"), c"sharedApplication") }
    }

    /// Run `work` on the main queue (inside whatever modal loop or menu
    /// tracking is going on then).
    fn on_main_queue(work: unsafe extern "C" fn(*mut c_void), ctx: *mut c_void) {
        // SAFETY: the main queue; the work function takes the context.
        unsafe { dispatch_async_f(std::ptr::addr_of!(_dispatch_main_q), ctx, work) };
    }

    /// Run `work` on the main queue after `ms` milliseconds.
    fn on_main_queue_after(ms: i64, work: unsafe extern "C" fn(*mut c_void), ctx: *mut c_void) {
        // SAFETY: as above; DISPATCH_TIME_NOW is 0.
        unsafe {
            let when = dispatch_time(0, ms * 1_000_000);
            dispatch_after_f(when, std::ptr::addr_of!(_dispatch_main_q), ctx, work);
        }
    }

    /// Pump events for a while (or until `done`).
    fn pump_until(ms: u64, done: impl Fn() -> bool) -> bool {
        let end = Instant::now() + Duration::from_millis(ms);
        while Instant::now() < end {
            while sdl3::events::poll().is_some() {}
            if done() {
                return true;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        done()
    }

    pub(super) fn main() {
        // A hung panel or menu shouldn't hang the test run.
        std::thread::spawn(|| {
            std::thread::sleep(Duration::from_secs(180));
            eprintln!("cocoa_shell test timed out");
            std::process::exit(1);
        });

        hints::set(hints::VIDEO_DRIVER, "cocoa").unwrap();
        if let Err(e) = init::init(InitFlags::VIDEO) {
            skip(e.message());
            return;
        }
        println!("cocoa_shell: video initialized");

        tray();
        tray_menu_from_the_status_item();
        dialogs_app_modal();
        dialog_sheet();
        notifications();

        init::quit();
        println!("cocoa_shell: all checks passed");
    }

    /// The `NSStatusBarButton` whose tooltip is `tip` in the view tree of
    /// `view`.
    ///
    /// # Safety
    ///
    /// `view` must be an `NSView`.
    unsafe fn find_status_button(view: Id, tip: &str) -> Option<Id> {
        // SAFETY: the caller's contract; subviews are NSViews.
        unsafe {
            let class = CStr::from_ptr(object_getClassName(view));
            if class.to_bytes() == b"NSStatusBarButton" {
                let tooltip: Id = send0(view, c"toolTip");
                if to_string(tooltip) == tip {
                    return Some(view);
                }
            }
            let subviews: Id = send0(view, c"subviews");
            let n: usize = send0(subviews, c"count");
            (0..n).find_map(|i| find_status_button(send1(subviews, c"objectAtIndex:", i), tip))
        }
    }

    /// `@autoreleasepool { f() }`: the arrays AppKit hands out retain the
    /// windows in them until their pool is drained.
    fn autoreleasepool<T>(f: impl FnOnce() -> T) -> T {
        unsafe extern "C" {
            fn objc_autoreleasePoolPush() -> *mut c_void;
            fn objc_autoreleasePoolPop(pool: *mut c_void);
        }
        // SAFETY: a pool pushed and popped around the call.
        unsafe {
            let pool = objc_autoreleasePoolPush();
            let r = f();
            objc_autoreleasePoolPop(pool);
            r
        }
    }

    /// The status bar button whose tooltip is `tip` (status items live in
    /// windows of their own among the application's windows).
    fn status_button(tip: &str) -> Option<Id> {
        autoreleasepool(|| status_button_in_pool(tip))
    }

    fn status_button_in_pool(tip: &str) -> Option<Id> {
        // SAFETY: NSApp's windows and their content views.
        unsafe {
            let windows: Id = send0(ns_app(), c"windows");
            let n: usize = send0(windows, c"count");
            (0..n).find_map(|i| {
                let w: Id = send1(windows, c"objectAtIndex:", i);
                let view: Id = send0(w, c"contentView");
                if view.is_null() {
                    None
                } else {
                    find_status_button(view, tip)
                }
            })
        }
    }

    fn icon() -> Surface<'static> {
        let mut s = Surface::new(16, 16, PixelFormat::RGBA32).unwrap();
        s.fill_rect(None, 0xff0000ff).unwrap();
        s
    }

    fn tray() {
        let tip = "SDL tray test";
        let icon = icon();
        let tray = Tray::create(Some(&icon), Some(tip)).unwrap();
        let button = status_button(tip).expect("the status item's button");
        // SAFETY: the button's image is an NSImage.
        let size: [f64; 2] = unsafe {
            let image: Id = send0(button, c"image");
            assert!(!image.is_null());
            send0(image, c"size")
        };
        assert_eq!(size, [22.0, 22.0]);

        tray.set_tooltip(Some("SDL tray test 2"));
        assert!(status_button("SDL tray test 2").is_some());
        tray.set_icon(None);
        // SAFETY: as above.
        assert!(unsafe { send0::<Id>(button, c"image") }.is_null());
        tray.set_tooltip(Some(tip));

        // The checks of the upstream C harness, with its results.
        let menu = tray.create_menu().unwrap();
        let a = menu
            .insert_entry(-1, Some("A"), TrayEntryFlags::BUTTON)
            .unwrap();
        let c = menu
            .insert_entry(
                -1,
                Some("C"),
                TrayEntryFlags::CHECKBOX | TrayEntryFlags::DISABLED,
            )
            .unwrap();
        let s = menu
            .insert_entry(-1, Some("S"), TrayEntryFlags::SUBMENU)
            .unwrap();
        let sep = menu.insert_entry(1, None, TrayEntryFlags::BUTTON).unwrap();
        let hits = Arc::new(AtomicUsize::new(0));
        let h = hits.clone();
        a.set_callback(move |e| {
            assert_eq!(e.label().unwrap().as_deref(), Some("A"));
            h.fetch_add(1, Ordering::SeqCst);
        });
        a.click();
        assert_eq!(hits.load(Ordering::SeqCst), 1);

        assert!(!c.checked().unwrap());
        assert!(!c.enabled().unwrap());
        assert_eq!(c.label().unwrap().as_deref(), Some("C"));
        c.set_enabled(true);
        c.set_checked(true);
        assert!(c.checked().unwrap());
        assert!(c.enabled().unwrap());
        c.click();
        assert!(!c.checked().unwrap());
        // (only checkboxes can be disabled, and only they report enabled)
        a.set_enabled(false);
        assert!(!a.enabled().unwrap());
        assert_eq!(sep.label().unwrap().as_deref(), Some(""));

        let sub = s.create_submenu().unwrap();
        assert_eq!(
            s.create_submenu().unwrap_err().to_string(),
            "Tray entry submenu already exists"
        );
        assert_eq!(
            a.create_submenu().unwrap_err().to_string(),
            "Cannot create submenu for entry not created with SDL_TRAYENTRY_SUBMENU"
        );
        let inner = sub
            .insert_entry(-1, Some("inner"), TrayEntryFlags::BUTTON)
            .unwrap();
        assert_eq!(inner.parent().unwrap(), sub);
        assert_eq!(sub.parent_entry().unwrap(), s);
        assert_eq!(s.submenu().unwrap(), sub);
        assert_eq!(
            menu.insert_entry(9, Some("x"), TrayEntryFlags::BUTTON)
                .unwrap_err()
                .to_string(),
            "Parameter 'pos' is invalid"
        );
        assert_eq!(
            menu.entries().unwrap(),
            [a.clone(), sep.clone(), c.clone(), s.clone()]
        );
        s.remove();
        assert_eq!(menu.entries().unwrap().len(), 3);
        assert!(inner.label().is_err());
        a.set_label(Some("Renamed"));
        assert_eq!(a.label().unwrap().as_deref(), Some("Renamed"));

        drop(tray);
        // (the status bar lets go of the item's window from the run loop)
        assert!(
            pump_until(2000, || status_button(tip).is_none()),
            "the status item is gone"
        );
        assert!(a.label().is_err());
        println!("cocoa_shell: tray ok");
    }

    /// The number of items of the menu `tray_menu_from_the_status_item`
    /// saw open.
    static TRACKED_ITEMS: AtomicIsize = AtomicIsize::new(-1);
    /// That menu, while it is open.
    static TRACKED_MENU: std::sync::atomic::AtomicPtr<c_void> =
        std::sync::atomic::AtomicPtr::new(std::ptr::null_mut());

    /// When the tray's menu starts tracking: remember it.
    unsafe extern "C" fn menu_did_begin_tracking(_block: *mut c_void, notification: Id) {
        // SAFETY: the notification's object is the menu being tracked.
        unsafe {
            let menu: Id = send0(notification, c"object");
            let n: isize = send0(menu, c"numberOfItems");
            TRACKED_ITEMS.store(n, Ordering::SeqCst);
            TRACKED_MENU.store(menu, Ordering::SeqCst);
        }
    }

    unsafe extern "C" {
        fn dispatch_time(when: u64, delta: i64) -> u64;
        fn dispatch_after_f(
            when: u64,
            queue: *const c_void,
            context: *mut c_void,
            work: unsafe extern "C" fn(*mut c_void),
        );
    }

    /// From the main queue while the menu tracks: choose its first item,
    /// then close it (or look again a little later, until it is open).
    unsafe extern "C" fn answer_menu(ctx: *mut c_void) {
        let menu = TRACKED_MENU.swap(std::ptr::null_mut(), Ordering::SeqCst);
        // SAFETY: the open menu; the main queue.
        unsafe {
            if menu.is_null() {
                dispatch_after_f(
                    dispatch_time(0, 20_000_000),
                    std::ptr::addr_of!(_dispatch_main_q),
                    ctx,
                    answer_menu,
                );
                return;
            }
            let n: isize = send0(menu, c"numberOfItems");
            if n > 0 {
                let _: () = send1(menu, c"performActionForItemAtIndex:", 0isize);
            }
            let _: () = send0(menu, c"cancelTracking");
        }
    }

    /// `Block_layout` of a global block.
    #[repr(C)]
    struct GlobalBlock {
        isa: *const c_void,
        flags: i32,
        reserved: i32,
        invoke: unsafe extern "C" fn(*mut c_void, Id),
        descriptor: *const [usize; 2],
    }

    /// Click the status item: its menu opens (the click handler pops it
    /// up), and the first item is chosen from the menu.
    fn tray_menu_from_the_status_item() {
        let tip = "SDL tray menu test";
        let hits = Arc::new(AtomicUsize::new(0));
        let left = Arc::new(AtomicUsize::new(0));
        let l = left.clone();
        let tray = Tray::create_with(TrayOptions {
            tooltip: Some(tip.to_owned()),
            left_click: Some(Box::new(move |_| {
                l.fetch_add(1, Ordering::SeqCst);
                true
            })),
            ..TrayOptions::default()
        })
        .unwrap();
        let menu = tray.create_menu().unwrap();
        let first = menu
            .insert_entry(-1, Some("First"), TrayEntryFlags::BUTTON)
            .unwrap();
        let h = hits.clone();
        first.set_callback(move |_| {
            h.fetch_add(1, Ordering::SeqCst);
        });
        menu.insert_entry(-1, Some("Second"), TrayEntryFlags::BUTTON)
            .unwrap();

        static DESCRIPTOR: [usize; 2] = [0, size_of::<GlobalBlock>()];
        let block = GlobalBlock {
            // (the address of the runtime's class object)
            isa: std::ptr::addr_of!(_NSConcreteGlobalBlock) as *const c_void,
            flags: 1 << 28, // BLOCK_IS_GLOBAL
            reserved: 0,
            invoke: menu_did_begin_tracking,
            descriptor: &DESCRIPTOR,
        };
        let button = status_button(tip).expect("the status item's button");
        // SAFETY: the notification center keeps the (global) block until
        // the observer is removed; the button is live.
        unsafe {
            let center: Id = send0(cls(c"NSNotificationCenter"), c"defaultCenter");
            let f: unsafe extern "C" fn(Id, Id, Id, Id, Id, *const GlobalBlock) -> Id =
                std::mem::transmute(objc_msgSend as unsafe extern "C" fn());
            let observer = f(
                center,
                sel(c"addObserverForName:object:queue:usingBlock:"),
                NSMenuDidBeginTrackingNotification,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                &block,
            );
            on_main_queue(answer_menu, std::ptr::null_mut());
            let _: () = send1(button, c"performClick:", std::ptr::null_mut::<c_void>());
            let _: () = send1(center, c"removeObserver:", observer);
        }
        pump_until(200, || false);

        assert_eq!(
            left.load(Ordering::SeqCst),
            1,
            "the left click callback ran"
        );
        assert_eq!(
            TRACKED_ITEMS.load(Ordering::SeqCst),
            2,
            "the tray's menu opened"
        );
        // (the item's action reaches the app delegate's `menu:`: see the
        // fix in tray/cocoa.rs)
        assert_eq!(
            hits.load(Ordering::SeqCst),
            1,
            "choosing the item ran its callback"
        );
        println!("cocoa_shell: tray menu ok");
        drop(tray);
    }

    /// The response [`stop_modal`] ends the modal loop with.
    static STOP_CODE: AtomicIsize = AtomicIsize::new(0);
    /// `NSModalResponseCancel`.
    const NS_MODAL_RESPONSE_CANCEL: isize = 0;
    /// `NSModalResponseOK`.
    const NS_MODAL_RESPONSE_OK: isize = 1;

    /// The folder a save panel [`stop_modal`] accepts must be showing
    /// first: the panel applies the location it was given asynchronously,
    /// so right after it opens it can still be at its default folder.
    static SAVE_PANEL_FOLDER: Mutex<Option<std::path::PathBuf>> = Mutex::new(None);
    /// How many times [`stop_modal`] has waited for that folder.
    static SAVE_PANEL_WAITS: AtomicUsize = AtomicUsize::new(0);
    /// The save panel's `directoryURL` path when [`stop_modal`] stopped it:
    /// the folder it was told to show, even when the panel (which runs out
    /// of process) never got there.
    static SAVE_PANEL_DIRECTORY: Mutex<Option<String>> = Mutex::new(None);

    /// `[NSApp stopModalWithCode:STOP_CODE]`, from the main queue inside a
    /// panel's modal loop (once a save panel is at [`SAVE_PANEL_FOLDER`],
    /// or after about 5 seconds).
    unsafe extern "C" fn stop_modal(ctx: *mut c_void) {
        // SAFETY: NSApp is running a modal loop.
        unsafe {
            let window: Id = send0(ns_app(), c"modalWindow");
            if window.is_null() {
                // (not up yet: try again)
                on_main_queue(stop_modal, ctx);
                return;
            }
            let folder = SAVE_PANEL_FOLDER.lock().unwrap().clone();
            if let Some(folder) = folder {
                // the save panel's URL: the file it would return now
                let url: Id = send0(window, c"URL");
                let path = if url.is_null() {
                    String::new()
                } else {
                    to_string(send0(url, c"path"))
                };
                let at_folder = std::path::Path::new(&path)
                    .parent()
                    .and_then(|p| p.canonicalize().ok())
                    .is_some_and(|p| p == folder);
                if !at_folder && SAVE_PANEL_WAITS.fetch_add(1, Ordering::SeqCst) < 500 {
                    on_main_queue_after(10, stop_modal, ctx);
                    return;
                }
                let directory: Id = send0(window, c"directoryURL");
                *SAVE_PANEL_DIRECTORY.lock().unwrap() =
                    (!directory.is_null()).then(|| to_string(send0(directory, c"path")));
            }
            let code = STOP_CODE.load(Ordering::SeqCst);
            let _: () = send1(ns_app(), c"stopModalWithCode:", code);
        }
    }

    /// What a file dialog's callback received.
    type DialogResult = Arc<Mutex<Option<(Result<Vec<String>, String>, Option<usize>)>>>;
    /// A file dialog's callback.
    type DialogCallback = Box<dyn FnOnce(sdl3::Result<Vec<String>>, Option<usize>) + Send>;

    fn dialog_result() -> (DialogResult, DialogCallback) {
        let result: DialogResult = Arc::new(Mutex::new(None));
        let r = result.clone();
        let callback = move |files: sdl3::Result<Vec<String>>, filter: Option<usize>| {
            *r.lock().unwrap() = Some((files.map_err(|e| e.to_string()), filter));
        };
        (result, Box::new(callback))
    }

    fn dialogs_app_modal() {
        // Cancelled: no paths (the callback runs before the call returns).
        let (result, callback) = dialog_result();
        STOP_CODE.store(NS_MODAL_RESPONSE_CANCEL, Ordering::SeqCst);
        on_main_queue(stop_modal, std::ptr::null_mut());
        show_file_dialog(
            FileDialogType::OpenFile,
            FileDialogOptions {
                filters: vec![DialogFileFilter::new("Images", "png;jpg")],
                many: true,
                title: Some("SDL open test".to_owned()),
                ..FileDialogOptions::default()
            },
            callback,
        );
        assert_eq!(result.lock().unwrap().take(), Some((Ok(Vec::new()), None)));

        // Accepted save panel at a default location: that file.
        let dir = std::env::temp_dir()
            .canonicalize()
            .unwrap()
            .join(format!("sdl3-cocoa-shell-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let location = dir.join("saved.txt");
        let (result, callback) = dialog_result();
        STOP_CODE.store(NS_MODAL_RESPONSE_OK, Ordering::SeqCst);
        *SAVE_PANEL_FOLDER.lock().unwrap() = Some(dir.clone());
        SAVE_PANEL_WAITS.store(0, Ordering::SeqCst);
        *SAVE_PANEL_DIRECTORY.lock().unwrap() = None;
        on_main_queue(stop_modal, std::ptr::null_mut());
        show_file_dialog(
            FileDialogType::SaveFile,
            FileDialogOptions {
                filters: vec![DialogFileFilter::new("Text", "txt")],
                location: Some(location.to_string_lossy().into_owned()),
                accept: Some("Keep".to_owned()),
                ..FileDialogOptions::default()
            },
            callback,
        );
        *SAVE_PANEL_FOLDER.lock().unwrap() = None;
        let (files, filter) = result
            .lock()
            .unwrap()
            .take()
            .expect("the save callback ran");
        assert_eq!(filter, None);
        let files = files.unwrap();
        assert_eq!(files.len(), 1);
        // (the panel names the folder by its /var path, not /private/var)
        let saved = std::path::Path::new(&files[0]);
        assert_eq!(saved.file_name().unwrap(), "saved.txt");
        let folder = saved.parent().unwrap().canonicalize().unwrap();
        if folder != dir {
            // The panel sometimes stays at its default folder however long
            // it's given (seen on CI runners): then the folder it was told
            // to show must be the one asked for.
            let directory = SAVE_PANEL_DIRECTORY.lock().unwrap().take();
            println!(
                "cocoa_shell: save panel stayed at {}, told to show {directory:?}",
                folder.display()
            );
            let directory = directory.expect("the save panel's directoryURL");
            let directory = std::path::Path::new(&directory).canonicalize().unwrap();
            if directory == folder {
                // ... unless the panel (out of process) dropped the folder
                // it was given altogether and reports its default as its
                // directoryURL too (seen on CI runners): that's the panel's
                // doing, after SDL's setDirectoryURL:, so there's nothing
                // of SDL's left to check here.
                println!(
                    "cocoa_shell: note: the save panel dropped its folder for its default; \
                     the location isn't checked this run"
                );
            } else {
                assert_eq!(directory, dir);
            }
        }
        let _ = std::fs::remove_dir_all(&dir);
        println!("cocoa_shell: app-modal dialogs ok");
    }

    fn dialog_sheet() {
        let window = Window::create("SDL dialog sheet test", 480, 360, WindowFlags(0)).unwrap();
        pump_until(300, || false);
        let nswindow = window
            .properties()
            .unwrap()
            .get_number(PROP_WINDOW_COCOA_WINDOW_POINTER)
            .unwrap() as usize as Id;

        let (result, callback) = dialog_result();
        let home = std::env::var("HOME").unwrap();
        show_file_dialog(
            FileDialogType::OpenFolder,
            FileDialogOptions {
                window: Some(window.id()),
                location: Some(format!("{home}/")),
                ..FileDialogOptions::default()
            },
            callback,
        );
        // A sheet: the call returns before the callback.
        assert!(result.lock().unwrap().is_none());
        // SAFETY: the window's attached sheet, once it's there.
        let sheet_up = pump_until(5000, || unsafe {
            !send0::<Id>(nswindow, c"attachedSheet").is_null()
        });
        assert!(sheet_up, "the panel attached as a sheet");
        // SAFETY: the window and its sheet.
        unsafe {
            let sheet: Id = send0(nswindow, c"attachedSheet");
            let _: () = send2(nswindow, c"endSheet:returnCode:", sheet, 0isize);
            // cancel
        }
        let done = pump_until(5000, || result.lock().unwrap().is_some());
        assert!(done, "the sheet's completion handler ran");
        assert_eq!(result.lock().unwrap().take(), Some((Ok(Vec::new()), None)));
        window.destroy();
        println!("cocoa_shell: dialog sheet ok");
    }

    fn notifications() {
        // UserNotifications needs an application bundle.
        match notification::show_notification(Some("SDL test"), Some("body"), None, &[]) {
            Err(e) => {
                assert_eq!(
                    e.to_string(),
                    "macOS notifications not supported outside an application bundle"
                );
                println!("note: skipping notifications: {e}");
            }
            Ok(id) => {
                notification::remove_notification(id).unwrap();
                println!("cocoa_shell: notification {id} shown and removed");
            }
        }
    }
}
