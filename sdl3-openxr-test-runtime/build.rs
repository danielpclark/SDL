// The test runtime's link options.
// Part of the repository's own test code (not a translation); see LICENSE.txt.

//! The library exports OpenXR's official names for the four global
//! functions (so that it can be loaded in the loader's place): on the ELF
//! targets the OpenXR loader's specification then asks for `-Bsymbolic`,
//! so that the library's references to its own symbols are bound to its
//! own definitions, never to a loader's of the same names loaded before it.

fn main() {
    let os = std::env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();
    if matches!(
        os.as_str(),
        "linux" | "android" | "freebsd" | "netbsd" | "openbsd"
    ) {
        println!("cargo:rustc-cdylib-link-arg=-Wl,-Bsymbolic");
    }
    println!("cargo:rerun-if-changed=build.rs");
}
