// The Objective-C runtime, Foundation, CoreFoundation, blocks and GCD for
// the Apple backends.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! Glue the Apple backends share. Upstream writes those backends in
//! Objective-C (`.m` files); here the runtime is reached through its C API,
//! declared by hand like the rest of the platform layer:
//!
//! * [`objc`]: message sending (`msg_send!`), selectors, classes, classes
//!   declared at run time, ownership ([`objc::StrongPtr`]) and autorelease
//!   pools;
//! * [`foundation`]: the geometry types, `NSString` conversions and the
//!   CoreFoundation objects;
//! * [`block`]: blocks wrapping Rust closures, and Grand Central Dispatch.
//!
//! The system frameworks are linked, as upstream links them (Foundation
//! and CoreFoundation here, the others by the backends that use them);
//! frameworks upstream links weakly are loaded at run time instead.

pub(crate) mod block;
pub(crate) mod foundation;
/// IOKit (the HID manager, the I/O Registry) and ForceFeedback.
#[cfg(target_os = "macos")]
pub(crate) mod iokit;
pub(crate) mod objc;

// Foundation's classes (NSString, NSArray, NSNumber, NSThread...) are
// looked up by name, so nothing references the framework's symbols.
#[link(name = "Foundation", kind = "framework")]
unsafe extern "C" {}
