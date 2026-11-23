//! The Gaming Runtime probe, with no static imports beyond kernel32 and
//! version.dll: LoadLibraryW("xgameruntime.dll") and GetProcAddress for
//! four documented names (nothing is called, no export table is read);
//! then the appnotify API sets and Wine's twinapi.appcore, loaded by name.

#[cfg(windows)]
fn main() {
    use gdk_probe::{file_version, load, module_path, proc};

    println!("== xgameruntime.dll (by name, the loader's search order)");
    match load("xgameruntime.dll") {
        Err(e) => {
            println!("LoadLibraryW(\"xgameruntime.dll\") failed: GetLastError() = {e} (0x{e:08x})")
        }
        Ok(m) => {
            let path = module_path(m);
            println!("loaded: {path}");
            println!("file version: {:?}", file_version(&path));
            for name in [
                "XGameRuntimeInitialize",
                "XTaskQueueCreate",
                "XUserAddAsync",
                "XGameGetXboxTitleId",
            ] {
                match proc(m, name) {
                    Ok(_) => println!("GetProcAddress({name}): found"),
                    Err(e) => println!("GetProcAddress({name}): not found, GetLastError() = {e}"),
                }
            }
        }
    }

    println!("== appnotify, by name");
    for dll in [
        "api-ms-win-core-psm-appnotify-l1-1-0.dll",
        "api-ms-win-core-psm-appnotify-l1-1-1.dll",
        "twinapi.appcore.dll",
    ] {
        match load(dll) {
            Err(e) => println!("LoadLibraryW(\"{dll}\") failed: GetLastError() = {e}"),
            Ok(m) => {
                println!("LoadLibraryW(\"{dll}\"): {}", module_path(m));
                for name in [
                    "RegisterAppStateChangeNotification",
                    "UnregisterAppStateChangeNotification",
                    "RegisterAppConstrainedChangeNotification",
                    "UnregisterAppConstrainedChangeNotification",
                ] {
                    match proc(m, name) {
                        Ok(_) => println!("  {name}: found"),
                        Err(e) => println!("  {name}: not found, GetLastError() = {e}"),
                    }
                }
            }
        }
    }
}

#[cfg(not(windows))]
fn main() {}
