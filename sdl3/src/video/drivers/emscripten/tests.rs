// Tests of the Emscripten video driver under node (in a browser:
// sdl3/tests/emscripten_video.rs).

use crate::hints;
use crate::init::{self, InitFlags};
use crate::video::messagebox::{show_simple_message_box, MessageBoxFlags};
use crate::video::{current_video_driver, num_video_drivers, video_driver};

/// Under node there's no DOM: the driver isn't available, so video
/// initialization goes on to the next driver, as for any unavailable
/// driver (Note (upstream): upstream's device is created, and its first
/// JavaScript throws).
#[test]
fn unavailable_without_dom() {
    let _l = crate::test_support::test_lock();
    init::quit();

    // In the list, before offscreen and dummy, as upstream's bootstrap.
    let names: Vec<_> = (0..num_video_drivers())
        .map(|i| video_driver(i).unwrap())
        .collect();
    let at = |name| names.iter().position(|&n| n == name).unwrap();
    assert!(at("emscripten") < at("offscreen"), "{names:?}");
    assert!(at("offscreen") < at("dummy"), "{names:?}");

    assert!(!super::has_dom());
    assert!((super::EMSCRIPTEN_BOOTSTRAP.create)().is_none());

    // Asked for by name: not available.
    hints::set(hints::VIDEO_DRIVER, "emscripten").unwrap();
    let e = init::init(InitFlags::VIDEO).unwrap_err();
    assert_eq!(e.message(), "emscripten not available");

    // The first that works, after it.
    hints::set(hints::VIDEO_DRIVER, "emscripten,dummy").unwrap();
    init::init(InitFlags::VIDEO).unwrap();
    assert_eq!(current_video_driver().unwrap(), "dummy");
    init::quit();

    // Without the hint none is available under node (the dummy and
    // offscreen drivers want to be asked for).
    hints::reset(hints::VIDEO_DRIVER);
    let e = init::init(InitFlags::VIDEO).unwrap_err();
    assert_eq!(e.message(), "No available video device");

    // A message box without video tries the Emscripten driver's, which
    // fails without a DOM (Note (upstream): upstream's alert() throws).
    let e = show_simple_message_box(MessageBoxFlags::ERROR, "title", "message", None).unwrap_err();
    assert_eq!(
        e.message(),
        "Emscripten message boxes need a page (no DOM: not in a browser)"
    );
    init::quit();
}
