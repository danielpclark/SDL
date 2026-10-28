// Tests of the CoreMedia camera driver on UIKit, in an app denied the
// camera.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! `uikit_camera`'s checks (`uikit_support/camera.rs`) in an app the
//! runner denies the camera (`privacy=-camera`: `simctl privacy revoke`,
//! so it isn't asked), against upstream's C run the same way on the same
//! simulator (an iPhone 16 with iOS 18.5). Elsewhere this program does
//! nothing.

#[cfg(not(target_os = "ios"))]
fn main() {}

/// The runner's words: run as an app, denied the camera.
#[cfg(target_os = "ios")]
#[used]
#[link_section = "__DATA,__sdl3_runner"]
static RUNNER: [u8; 20] = *b"app privacy=-camera\0";

#[cfg(target_os = "ios")]
mod uikit_support;

/// Upstream's trace (the C program, on an iPhone 16 simulator with iOS
/// 18.5).
#[cfg(target_os = "ios")]
const EXPECTED: &str = "\
app state: active
status bar orientation: 1
UIDevice orientation: 0 generating=0
UIDevice orientation (tracking): 0 generating=1
UIDevice orientation tracking ended: generating=0
app: authorization status: 2
app: camera drivers: coremedia, dummy
app: init camera: ok
app: camera driver: coremedia
app: cameras: 0
app: camera events: (none)
app: open 0: Invalid camera device instance ID
app: open 0 with a spec: Invalid camera device instance ID
app: supported formats 0: Invalid camera device instance ID count=0
app: name 0: Invalid camera device instance ID
app: position 0: 0
app: open 12345: Invalid camera device instance ID
app: open 12345 with a spec: Invalid camera device instance ID
app: supported formats 12345: Invalid camera device instance ID count=0
app: name 12345: Invalid camera device instance ID
app: position 12345: 0
app: camera events: (none)
app: quit camera: driver=(null)
app: dummy: init ok driver=dummy cameras=0
app: coremedia again: init ok driver=coremedia cameras=0
app: authorization status: 2
app: end: camera events: (none)
UIDevice generating orientation notifications: 0
done
";

#[cfg(target_os = "ios")]
fn main() {
    uikit_support::camera::main("uikit_camera_denied", EXPECTED)
}
