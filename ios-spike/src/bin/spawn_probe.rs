// `xcrun simctl spawn UDID spawn_probe MODE [ARGS...]`: what a plain
// (non-app) program sees in the simulator.
//
// MODE: info [paths...] | exit N | panic | abort | segv | big N | stdin | sleep S

use std::io::{Read, Write};

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let mode = args.get(1).map(String::as_str).unwrap_or("info");
    match mode {
        "info" => info(&args[2..]),
        "exit" => {
            println!("spawn_probe: stdout before exit");
            eprintln!("spawn_probe: stderr before exit");
            // no newline: does a partial line come through?
            print!("partial-line-no-newline");
            std::io::stdout().flush().unwrap();
            std::process::exit(args[2].parse().unwrap());
        }
        "panic" => {
            println!("spawn_probe: about to panic");
            panic!("spawn_probe panics on purpose");
        }
        "abort" => {
            println!("spawn_probe: about to abort");
            std::process::abort();
        }
        "segv" => {
            println!("spawn_probe: about to raise SIGSEGV");
            unsafe { libc_raise(11) };
        }
        "big" => {
            let n: usize = args[2].parse().unwrap();
            let out = std::io::stdout();
            let mut out = out.lock();
            for i in 0..n {
                writeln!(out, "line {i} {}", "x".repeat(60)).unwrap();
            }
            drop(out);
            eprintln!("spawn_probe: big done ({n} lines)");
        }
        "stdin" => {
            let mut s = String::new();
            let r = std::io::stdin().read_to_string(&mut s);
            println!("spawn_probe: stdin read {:?}: {:?}", r.map(|_| ()), s);
        }
        "sleep" => {
            let s: u64 = args[2].parse().unwrap();
            println!("spawn_probe: sleeping {s}s");
            std::thread::sleep(std::time::Duration::from_secs(s));
            println!("spawn_probe: slept");
        }
        m => {
            eprintln!("unknown mode {m}");
            std::process::exit(64);
        }
    }
}

unsafe extern "C" {
    #[link_name = "raise"]
    fn libc_raise(sig: i32) -> i32;
    fn getuid() -> u32;
    fn getpid() -> i32;
    fn isatty(fd: i32) -> i32;
}

fn info(paths: &[String]) {
    println!("== spawn_probe info");
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
        let path = std::path::Path::new(p);
        match std::fs::metadata(path) {
            Ok(m) if m.is_dir() => match std::fs::read_dir(path) {
                Ok(rd) => {
                    let names: Vec<_> = rd
                        .take(8)
                        .filter_map(|e| e.ok())
                        .map(|e| e.file_name().to_string_lossy().into_owned())
                        .collect();
                    println!("path {p}: dir, readable, first entries {names:?}");
                    let probe = path.join("spike-write-probe.txt");
                    match std::fs::write(&probe, b"written from the simulator\n") {
                        Ok(()) => println!("path {p}: writable"),
                        Err(e) => println!("path {p}: not writable: {e}"),
                    }
                }
                Err(e) => println!("path {p}: dir, read_dir failed: {e}"),
            },
            Ok(_) => match std::fs::read(path) {
                Ok(b) => println!(
                    "path {p}: file, read {} bytes: {:?}",
                    b.len(),
                    String::from_utf8_lossy(&b[..b.len().min(60)])
                ),
                Err(e) => println!("path {p}: file, read failed: {e}"),
            },
            Err(e) => println!("path {p}: stat failed: {e}"),
        }
    }
    println!("== spawn_probe info end");
}
