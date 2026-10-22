// Build script of sdl3-emscripten: no code is generated or compiled.
//
// The integration tests (tests/) are linked the way an application that
// passes its own `-sEXPORTED_FUNCTIONS` is: that list replaces the one rustc
// passes (every `#[no_mangle]` function), so nothing but `main` is exported
// to JavaScript. What the tests check (em_js!, em_asm!, the dependency
// table and, above all, the callback registry) has to work without exports.
// Cargo has no per-test link arguments; this applies to all of this
// package's tests and to nothing else (not to sdl3's, not to applications).

fn main() {
    println!("cargo::rerun-if-changed=build.rs");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("emscripten") {
        println!("cargo::rustc-link-arg-tests=-sEXPORTED_FUNCTIONS=_main");
    }
}
