// What the OpenXR test programs share: the capability checks, Monado's
// service, the test runtime, the scenarios' child processes and the trace.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! The programs (`harness = false`) run each scenario in a child process
//! of their own (the program itself, or with `--run-c PATH` the C twin, so
//! that one harness runs both): with a private runtime directory
//! (`XDG_RUNTIME_DIR`, mode 0700), Mesa's lavapipe as the only Vulkan
//! driver (`VK_ICD_FILENAMES`), the offscreen video driver, and for the
//! scenarios that need it Monado's service with its null compositor
//! (`monado-service`, `XRT_COMPOSITOR_NULL=1`), started for the scenario
//! with a pipe as its standard input (it exits at once on `/dev/null`) and
//! stopped by closing the pipe. The child's trace is its standard output's
//! `xrtrace: ` lines; the loader's, the runtime's and the validation
//! layers' messages stay out of it, the Vulkan validation layer's errors
//! and the OpenXR loader's and layers' valid usage messages counted.
//!
//! Capability `openxr`: the OpenXR loader (`libopenxr_loader.so.1`),
//! `monado-service` in `PATH`, the core validation API layer
//! (`XR_APILAYER_LUNARG_core_validation`) and lavapipe. A program skips
//! without them, unless `SDL3_TEST_REQUIRE` names `openxr`.
//!
//! Capability `openxr-runtime`: the repository's test runtime
//! (sdl3-openxr-test-runtime's `libsdl3_openxr_test_runtime.so`, beside the
//! program, or where `SDL3_OPENXR_TEST_RUNTIME` says), reached through the
//! loader with a manifest of it made in the scenario's runtime directory
//! (`XR_RUNTIME_JSON`) or in the loader's place (`SDL_OPENXR_LIBRARY`),
//! with the loader, the core validation API layer and lavapipe.

#![allow(dead_code)] // (each program uses some of it)

pub mod xr;

use std::ffi::CStr;
use std::io::{Read, Write};
use std::os::unix::fs::{DirBuilderExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, ExitCode, Stdio};
use std::time::{Duration, Instant};

/// The variable naming a child's scenario.
pub const SCENARIO: &str = "SDL3_GPU_OPENXR_SCENARIO";
/// The prefix of a child's trace lines.
pub const PREFIX: &str = "xrtrace: ";
/// Lavapipe's ICD manifest (Ubuntu's mesa-vulkan-drivers).
pub const LAVAPIPE_ICD: &str = "/usr/share/vulkan/icd.d/lvp_icd.json";
/// The core validation API layer's name.
pub const XR_VALIDATION_LAYER: &str = "XR_APILAYER_LUNARG_core_validation";

/// Whether `SDL3_TEST_REQUIRE` makes a capability mandatory.
pub fn required(capability: &str) -> bool {
    std::env::var("SDL3_TEST_REQUIRE")
        .unwrap_or_default()
        .split(',')
        .map(str::trim)
        .any(|c| c == "all" || c == capability)
}

/// Skip (all of) the program, as the library's tests do.
pub fn skip(program: &str, capability: &str, reason: &str) -> ExitCode {
    assert!(
        !required(capability),
        "required capability {capability} unavailable: {reason}"
    );
    eprintln!("note: skipping, {capability} unavailable: {reason}");
    if let Ok(path) = std::env::var("SDL3_TEST_SKIP_LOG") {
        if let Ok(mut file) = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
        {
            let _ = writeln!(file, "{capability}\t{program}\t{reason}");
        }
    }
    ExitCode::SUCCESS
}

/// Whether a library can be loaded (`dlopen()`).
pub fn loadable(name: &CStr) -> bool {
    // SAFETY: dlopen of a NUL-terminated name; the handle is closed at
    // once.
    unsafe {
        let handle = libc::dlopen(name.as_ptr(), libc::RTLD_LAZY | libc::RTLD_LOCAL);
        if handle.is_null() {
            return false;
        }
        libc::dlclose(handle);
    }
    true
}

/// A program in `PATH`.
pub fn in_path(program: &str) -> Option<PathBuf> {
    std::env::split_paths(&std::env::var_os("PATH")?)
        .map(|dir| dir.join(program))
        .find(|path| {
            path.metadata()
                .is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
        })
}

/// Whether the loader finds an explicit API layer (in `XR_API_LAYER_PATH`,
/// or the system's and the user's directories).
pub fn api_layer_installed(name: &str) -> bool {
    let mut dirs: Vec<PathBuf> = match std::env::var_os("XR_API_LAYER_PATH") {
        Some(path) => std::env::split_paths(&path).collect(),
        None => Vec::new(),
    };
    for base in ["/usr/share", "/usr/local/share", "/etc"] {
        dirs.push(Path::new(base).join("openxr/1/api_layers/explicit.d"));
    }
    dirs.iter().any(|dir| {
        std::fs::read_dir(dir).is_ok_and(|entries| {
            entries.flatten().any(|e| {
                std::fs::read_to_string(e.path())
                    .is_ok_and(|text| text.contains(&format!("\"{name}\"")))
            })
        })
    })
}

/// Why the `openxr` capability is missing, if it is.
pub fn missing_openxr() -> Option<String> {
    if !loadable(c"libopenxr_loader.so.1") {
        return Some("no OpenXR loader (libopenxr_loader.so.1)".into());
    }
    if in_path("monado-service").is_none() {
        return Some("no monado-service in PATH".into());
    }
    if !api_layer_installed(XR_VALIDATION_LAYER) {
        return Some(format!("no {XR_VALIDATION_LAYER} API layer"));
    }
    if !Path::new(LAVAPIPE_ICD).exists() {
        return Some(format!("no lavapipe ({LAVAPIPE_ICD})"));
    }
    None
}

/// The variable naming the repository's test runtime library, when it
/// isn't beside the program.
pub const TEST_RUNTIME_PATH: &str = "SDL3_OPENXR_TEST_RUNTIME";
/// The test runtime's failure list (see sdl3-openxr-test-runtime's fail.rs).
pub const TEST_RUNTIME_FAIL: &str = "SDL3_OPENXR_TEST_RUNTIME_FAIL";
/// The test runtime's file name.
pub const TEST_RUNTIME_FILE: &str = "libsdl3_openxr_test_runtime.so";

/// The repository's test runtime (sdl3-openxr-test-runtime, which sdl3's
/// dev-dependency on it builds beside the test programs, in `deps/`), or
/// the library [`TEST_RUNTIME_PATH`] names.
pub fn test_runtime() -> Option<PathBuf> {
    if let Some(path) = std::env::var_os(TEST_RUNTIME_PATH) {
        return Some(PathBuf::from(path)).filter(|p| p.is_file());
    }
    let exe = std::env::current_exe().ok()?;
    let deps = exe.parent()?;
    [
        deps.join(TEST_RUNTIME_FILE),
        deps.parent()?.join(TEST_RUNTIME_FILE),
    ]
    .into_iter()
    .find(|p| p.is_file())
}

/// Why the `openxr-runtime` capability is missing, if it is: the test
/// runtime, the OpenXR loader (for the scenarios through it, with a
/// manifest), the core validation API layer (which the loader then finds,
/// and the main scenario enables) and lavapipe.
pub fn missing_openxr_runtime() -> Option<String> {
    if test_runtime().is_none() {
        return Some(format!(
            "no test runtime ({TEST_RUNTIME_FILE} beside the program, or {TEST_RUNTIME_PATH}: cargo build -p sdl3-openxr-test-runtime)"
        ));
    }
    if !loadable(c"libopenxr_loader.so.1") {
        return Some("no OpenXR loader (libopenxr_loader.so.1)".into());
    }
    if !api_layer_installed(XR_VALIDATION_LAYER) {
        return Some(format!("no {XR_VALIDATION_LAYER} API layer"));
    }
    if !Path::new(LAVAPIPE_ICD).exists() {
        return Some(format!("no lavapipe ({LAVAPIPE_ICD})"));
    }
    None
}

/// A runtime manifest for the test runtime in `dir` (for `XR_RUNTIME_JSON`).
pub fn write_test_runtime_manifest(dir: &Path, library: &Path) -> PathBuf {
    let manifest = dir.join("sdl3_openxr_test_runtime.json");
    let path = library
        .to_string_lossy()
        .replace('\\', "\\\\")
        .replace('"', "\\\"");
    let text = format!(
        "{{\n    \"file_format_version\": \"1.0.0\",\n    \"runtime\": {{\n        \"name\": \"sdl3-openxr-test-runtime\",\n        \"library_path\": \"{path}\"\n    }}\n}}\n"
    );
    std::fs::write(&manifest, text).expect("the runtime manifest");
    manifest
}

/// A private runtime directory (mode 0700), removed when dropped.
pub struct RuntimeDir(pub PathBuf);

impl RuntimeDir {
    pub fn new(name: &str) -> RuntimeDir {
        let dir = std::env::temp_dir().join(format!(
            "sdl3-{name}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |d| d.subsec_nanos())
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::DirBuilder::new()
            .mode(0o700)
            .create(&dir)
            .expect("the runtime directory");
        RuntimeDir(dir)
    }
}

impl Drop for RuntimeDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// Monado's service, with its null compositor, in a runtime directory.
pub struct Monado {
    child: Child,
    stdin: Option<ChildStdin>,
    pub log: PathBuf,
}

impl Monado {
    /// Start it and wait for its socket (`monado_comp_ipc`), for up to 30 s.
    pub fn start(dir: &Path) -> Result<Monado, String> {
        let log = dir.join("monado.log");
        let out = std::fs::File::create(&log).map_err(|e| e.to_string())?;
        let err = out.try_clone().map_err(|e| e.to_string())?;
        let mut child = Command::new("monado-service")
            .env("XDG_RUNTIME_DIR", dir)
            .env("XRT_COMPOSITOR_NULL", "1")
            .env("VK_ICD_FILENAMES", LAVAPIPE_ICD)
            .stdin(Stdio::piped())
            .stdout(out)
            .stderr(err)
            .spawn()
            .map_err(|e| format!("monado-service: {e}"))?;
        let stdin = child.stdin.take();
        let mut monado = Monado { child, stdin, log };
        let socket = dir.join("monado_comp_ipc");
        let end = Instant::now() + Duration::from_secs(30);
        while !socket.exists() {
            if let Ok(Some(status)) = monado.child.try_wait() {
                return Err(format!(
                    "monado-service exited ({status}): {}",
                    monado.log_tail()
                ));
            }
            if Instant::now() >= end {
                return Err(format!(
                    "no Monado socket after 30 s: {}",
                    monado.log_tail()
                ));
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        Ok(monado)
    }

    /// The last lines of its log.
    pub fn log_tail(&self) -> String {
        let text = std::fs::read_to_string(&self.log).unwrap_or_default();
        let lines: Vec<&str> = text.lines().collect();
        lines[lines.len().saturating_sub(20)..].join("\n")
    }
}

impl Drop for Monado {
    fn drop(&mut self) {
        // (closing its standard input stops it)
        drop(self.stdin.take());
        let end = Instant::now() + Duration::from_secs(10);
        while Instant::now() < end {
            if let Ok(Some(_)) = self.child.try_wait() {
                return;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// What a scenario's child printed.
pub struct ChildOutput {
    /// Its trace (the `xrtrace: ` lines, without the prefix).
    pub trace: Vec<String>,
    /// How it ended: `exit N`, `signal N` or `timeout`.
    pub status: String,
    /// The Vulkan validation layer's messages (lines with `Validation
    /// Error`), in both streams.
    pub validation: Vec<String>,
    /// The OpenXR loader's and API layers' messages of valid usage (the
    /// other lines with `VUID-`).
    pub openxr_usage: Vec<String>,
    pub stdout: String,
    pub stderr: String,
}

/// Run a scenario's child (`program`, with the scenario in [`SCENARIO`])
/// in the runtime directory, for up to `timeout`.
pub fn run_child(
    program: &Path,
    scenario: &str,
    dir: &Path,
    env: &[(&str, &str)],
    timeout: Duration,
) -> ChildOutput {
    use std::os::unix::process::ExitStatusExt;
    let mut command = Command::new(program);
    command
        .env(SCENARIO, scenario)
        .env("XDG_RUNTIME_DIR", dir)
        .env("VK_ICD_FILENAMES", LAVAPIPE_ICD)
        .env("SDL_VIDEO_DRIVER", "offscreen")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    for (name, value) in env {
        command.env(name, value);
    }
    let mut child = command.spawn().expect("the scenario's child");
    let mut out = child.stdout.take().expect("stdout");
    let mut err = child.stderr.take().expect("stderr");
    let out_thread = std::thread::spawn(move || {
        let mut s = String::new();
        let _ = out.read_to_string(&mut s);
        s
    });
    let err_thread = std::thread::spawn(move || {
        let mut s = String::new();
        let _ = err.read_to_string(&mut s);
        s
    });
    let end = Instant::now() + timeout;
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                break match (status.code(), status.signal()) {
                    (Some(code), _) => format!("exit {code}"),
                    (None, Some(signal)) => format!("signal {signal}"),
                    _ => "exit ?".to_owned(),
                }
            }
            Ok(None) if Instant::now() < end => std::thread::sleep(Duration::from_millis(50)),
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                break "timeout".to_owned();
            }
        }
    };
    let stdout = out_thread.join().unwrap_or_default();
    let stderr = err_thread.join().unwrap_or_default();
    let trace = stdout
        .lines()
        .filter_map(|l| l.strip_prefix(PREFIX))
        .map(str::to_owned)
        .collect();
    let validation = stdout
        .lines()
        .chain(stderr.lines())
        .filter(|l| l.contains("Validation Error"))
        .map(str::to_owned)
        .collect();
    let openxr_usage = stdout
        .lines()
        .chain(stderr.lines())
        .filter(|l| l.contains("VUID-") && !l.contains("Validation Error"))
        .map(str::to_owned)
        .collect();
    ChildOutput {
        trace,
        status,
        validation,
        openxr_usage,
        stdout,
        stderr,
    }
}

/// The installed versions of the packages the trace depends on (the
/// reference's preamble).
pub fn versions() -> String {
    let packages = [
        "monado-service",
        "libopenxr-loader1",
        "openxr-layer-corevalidation",
        "mesa-vulkan-drivers",
        "vulkan-validationlayers",
    ];
    let out = Command::new("dpkg-query")
        .args(["-W", "-f", "${Package} ${Version}\\n"])
        .args(packages)
        .output();
    let text = out
        .map(|o| String::from_utf8_lossy(&o.stdout).into_owned())
        .unwrap_or_default();
    let found: Vec<&str> = text.lines().filter(|l| !l.is_empty()).collect();
    format!("versions: {}", found.join(", "))
}

/// The trace, printed as it's made.
pub struct Trace {
    pub lines: Vec<String>,
    /// Rust-only failures (lines `rust: `), apart from the trace
    pub rust_failures: Vec<String>,
}

impl Default for Trace {
    fn default() -> Trace {
        Trace::new()
    }
}

impl Trace {
    /// A trace whose first line is the versions (the preamble).
    pub fn new() -> Trace {
        let mut t = Trace {
            lines: Vec::new(),
            rust_failures: Vec::new(),
        };
        t.line(versions());
        t
    }

    pub fn line(&mut self, line: impl Into<String>) {
        let line = line.into();
        println!("trace: {line}");
        std::io::stdout().flush().unwrap();
        self.lines.push(line);
    }

    /// A Rust-only check's outcome (not in the trace).
    pub fn rust(&mut self, ok: bool, line: impl Into<String>) {
        let line = line.into();
        println!("rust: {line}");
        if !ok {
            self.rust_failures.push(line);
        }
    }

    /// Compare the trace with upstream's (its preamble apart): whether it
    /// is, with no Rust-only failure.
    pub fn finish(self, program: &str, expected: &str) -> bool {
        let mut ok = true;
        for f in &self.rust_failures {
            println!("{program}: {f}");
            ok = false;
        }
        let expected: Vec<&str> = expected.lines().collect();
        let actual: Vec<&str> = self.lines.iter().map(String::as_str).collect();
        if actual.get(1..) == expected.get(1..) {
            println!(
                "{program}: the trace is upstream's ({} lines)",
                self.lines.len()
            );
            return ok;
        }
        if expected.first() != actual.first() {
            println!(
                "{program}: the reference's {:?}, here {:?}",
                expected.first(),
                actual.first()
            );
        }
        for (i, line) in actual.iter().enumerate().skip(1) {
            if expected.get(i) != Some(line) {
                println!(
                    "{program}: line {}: {line:?}, upstream's {:?}",
                    i + 1,
                    expected.get(i)
                );
                break;
            }
        }
        if expected.len() != actual.len() {
            println!(
                "{program}: {} lines, upstream's {}",
                actual.len(),
                expected.len()
            );
        }
        false
    }
}
