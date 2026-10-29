// `adb shell /data/local/tmp/spike/probe MODE [ARGS...]`: what a plain
// (non-app) program sees on the emulator, plus the platform probes from a
// shell process.
//
// MODE: info [paths...] | exit N | ret N | panic | abort | segv | big N |
//       stdin | sleep S | gfx | audio | sensors | camera | dlopen-self

use android_spike::{audio, camera, gfx, say, sensors};
use std::io::{Read, Write};
use std::process::ExitCode;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().collect();
    let mode = args.get(1).map(String::as_str).unwrap_or("info");
    match mode {
        "info" => info(&args[2..]),
        "exit" => {
            println!("probe: stdout before exit");
            eprintln!("probe: stderr before exit");
            print!("partial-line-no-newline");
            std::io::stdout().flush().unwrap();
            std::process::exit(args[2].parse().unwrap());
        }
        "ret" => {
            println!("probe: main() returning {} (args {:?})", args[2], args);
            return ExitCode::from(args[2].parse::<u8>().unwrap());
        }
        "panic" => {
            println!("probe: about to panic");
            panic!("probe panics on purpose");
        }
        "abort" => {
            println!("probe: about to abort");
            std::process::abort();
        }
        "segv" => {
            println!("probe: about to raise SIGSEGV");
            unsafe { raise(11) };
        }
        "big" => {
            let n: usize = args[2].parse().unwrap();
            let out = std::io::stdout();
            let mut out = out.lock();
            for i in 0..n {
                writeln!(out, "line {i} {}", "x".repeat(60)).unwrap();
            }
            drop(out);
            eprintln!("probe: big done ({n} lines)");
        }
        "stdin" => {
            let mut s = String::new();
            let r = std::io::stdin().read_to_string(&mut s);
            println!("probe: stdin read {:?}: {:?}", r.map(|_| ()), s);
        }
        "sleep" => {
            let s: u64 = args[2].parse().unwrap();
            println!("probe: sleeping {s}s");
            std::thread::sleep(std::time::Duration::from_secs(s));
            println!("probe: slept");
        }
        "gfx" => {
            if let Err(e) = gfx::egl("shell", std::ptr::null_mut()) {
                say!("[shell] EGL error: {e}");
            }
            if let Err(e) = gfx::vulkan("shell") {
                say!("[shell] Vulkan error: {e}");
            }
        }
        "audio" => {
            for input in [false, true] {
                if let Err(e) = audio::aaudio("shell", input) {
                    say!("[shell] AAudio error: {e}");
                }
            }
            if let Err(e) = audio::opensles("shell") {
                say!("[shell] OpenSL ES error: {e}");
            }
        }
        "sensors" => {
            if let Err(e) = sensors::sensors("shell", false) {
                say!("[shell] sensors error: {e}");
            }
        }
        "camera" => {
            if let Err(e) = camera::camera("shell", true) {
                say!("[shell] camera error: {e}");
            }
        }
        m => {
            eprintln!("unknown mode {m}");
            return ExitCode::from(64);
        }
    }
    ExitCode::SUCCESS
}

unsafe extern "C" {
    fn raise(sig: i32) -> i32;
    fn getuid() -> u32;
    fn getpid() -> i32;
    fn isatty(fd: i32) -> i32;
}

fn info(paths: &[String]) {
    println!("== probe info");
    println!("args = {:?}", std::env::args().collect::<Vec<_>>());
    println!("cwd = {:?}", std::env::current_dir());
    println!("exe = {:?}", std::env::current_exe());
    println!("temp_dir = {:?}", std::env::temp_dir());
    unsafe {
        println!("uid = {} pid = {}", getuid(), getpid());
        println!("isatty stdin={} stdout={} stderr={}", isatty(0), isatty(1), isatty(2));
    }
    let mut vars: Vec<_> = std::env::vars().collect();
    vars.sort();
    for (k, v) in vars {
        println!("env {k}={v}");
    }
    eprintln!("stderr: a line on stderr");
    for p in paths {
        match std::fs::metadata(p) {
            Ok(m) => println!("path {p}: exists dir={} len={}", m.is_dir(), m.len()),
            Err(e) => println!("path {p}: {e}"),
        }
    }
    match std::process::Command::new("/system/bin/sh").args(["-c", "echo child-ok; id"]).output() {
        Ok(o) => println!("spawned /system/bin/sh: {:?} {:?}", o.status, String::from_utf8_lossy(&o.stdout)),
        Err(e) => println!("spawn /system/bin/sh failed: {e}"),
    }
    match std::process::Command::new("/bin/sh").arg("-c").arg("true").status() {
        Ok(s) => println!("spawned /bin/sh: {s:?}"),
        Err(e) => println!("spawn /bin/sh failed: {e}"),
    }
    println!("== probe info end");
}
