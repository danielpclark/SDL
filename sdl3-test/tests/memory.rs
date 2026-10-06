// The memory tracker as a program uses it: the TrackingAllocator as the
// global allocator, tracking started at the beginning of main. (Without the
// libtest harness, so that no other thread allocates meanwhile.)

use std::alloc::{alloc, dealloc, Layout};
use std::sync::{Arc, Mutex};

use sdl3_test::fuzzer;
use sdl3_test::memory::{self, TrackingAllocator};

#[global_allocator]
static ALLOCATOR: TrackingAllocator = TrackingAllocator;

/// The messages logged by `f`.
fn logged(lines: &Arc<Mutex<Vec<String>>>, f: impl FnOnce()) -> String {
    lines.lock().unwrap().clear();
    f();
    let lines = lines.lock().unwrap();
    lines.join("\n")
}

fn main() {
    let lines: Arc<Mutex<Vec<String>>> = Arc::default();
    let sink = lines.clone();
    sdl3::log::set_output(move |record| sink.lock().unwrap().push(record.message.to_owned()));

    // Not tracked yet: nothing to log.
    assert_eq!(logged(&lines, memory::log_allocations), "");

    let from_before = Box::new([1u8; 7]);
    let message = logged(&lines, memory::track_allocations);
    assert!(
        message.contains("previous allocations, disabling free() validation"),
        "{message}"
    );

    // An outstanding allocation, with its stack.
    let kept: Vec<u8> = Vec::with_capacity(1000);
    let message = logged(&lines, memory::log_allocations);
    assert!(message.starts_with("Memory allocations:\n"), "{message}");
    let entry = message
        .lines()
        .position(|l| l.starts_with("Allocation ") && l.ends_with(": 1000 bytes"))
        .unwrap_or_else(|| panic!("{message}"));
    if cfg!(any(target_os = "linux", windows)) {
        let frame = message.lines().nth(entry + 1).unwrap();
        assert!(frame.starts_with("\t0x"), "{message}");
    }
    assert!(
        message.lines().last().unwrap().starts_with("Total: "),
        "{message}"
    );
    drop(kept);
    let message = logged(&lines, memory::log_allocations);
    assert!(!message.contains(": 1000 bytes"), "{message}");

    // A block from before the tracking is an unknown free.
    drop(from_before);
    let message = logged(&lines, memory::log_allocations);
    assert!(message.contains(" unknown frees"), "{message}");

    // Random fill: the fuzzer's bytes.
    memory::rand_fill_allocations();
    let layout = Layout::from_size_align(16, 1).unwrap();
    fuzzer::init(77);
    // SAFETY: a non-zero size; the bytes were written by the allocator.
    let filled: Vec<u8> = unsafe {
        let p = alloc(layout);
        assert!(!p.is_null());
        let bytes = std::slice::from_raw_parts(p, 16).to_vec();
        dealloc(p, layout);
        bytes
    };
    // (the expected bytes go in a buffer allocated before, as allocating
    // draws from the fuzzer)
    let mut expected: Vec<u8> = Vec::with_capacity(16);
    fuzzer::init(77);
    expected.extend((0..16).map(|_| fuzzer::random_uint8()));
    assert_eq!(filled, expected);

    // Growing a block fills the new part.
    fuzzer::init(78);
    let mut grown: Vec<u8> = Vec::with_capacity(4);
    grown.extend_from_slice(&[9, 9, 9, 9]);
    grown.reserve_exact(4);
    let capacity = grown.capacity();
    // SAFETY: within the capacity; written by the allocator.
    let tail = unsafe { std::slice::from_raw_parts(grown.as_ptr().add(4), capacity - 4).to_vec() };
    let mut first: Vec<u8> = Vec::with_capacity(4);
    let mut rest: Vec<u8> = Vec::with_capacity(capacity - 4);
    fuzzer::init(78);
    first.extend((0..4).map(|_| fuzzer::random_uint8()));
    rest.extend((0..capacity - 4).map(|_| fuzzer::random_uint8()));
    assert_eq!(&grown[..4], &[9, 9, 9, 9]);
    assert_ne!(first, rest);
    assert_eq!(tail, rest);

    sdl3::log::reset_output();
    println!("memory tracker: ok");
}
