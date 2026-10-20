// Tests for the RawInput joystick driver's bookkeeping: the WGI gamepad
// correlation and the WGI reference count (run under Wine on Linux).
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

use super::*;
use crate::core::windows::com::{ComObject, IUnknownVtbl};

use std::ffi::c_void;

unsafe extern "system" fn fake_query_interface(
    _this: *mut c_void,
    _iid: *const windows_sys::core::GUID,
    out: *mut *mut c_void,
) -> windows_sys::core::HRESULT {
    // SAFETY: a valid out pointer.
    unsafe { *out = std::ptr::null_mut() };
    0x8000_4002_u32 as windows_sys::core::HRESULT // E_NOINTERFACE
}

unsafe extern "system" fn fake_add_ref_release(_this: *mut c_void) -> u32 {
    1
}

static FAKE_UNKNOWN: IUnknownVtbl = IUnknownVtbl {
    query_interface: fake_query_interface,
    add_ref: fake_add_ref_release,
    release: fake_add_ref_release,
};

/// A static object that only answers `IUnknown` (dropping a `ComPtr` to
/// it does nothing); the correlation code never calls the gamepad.
struct FakeGamepad(ComObject<IGamepadVtbl>);

// SAFETY: the object is immutable; its methods do nothing.
unsafe impl Sync for FakeGamepad {}

static FAKE_GAMEPAD: FakeGamepad = FakeGamepad(ComObject {
    vtbl: (&FAKE_UNKNOWN as *const IUnknownVtbl).cast(),
});

fn gamepad(id: u64, connected: bool, used: bool) -> WgiGamepadState {
    let object = (&FAKE_GAMEPAD.0 as *const ComObject<IGamepadVtbl>).cast_mut();
    WgiGamepadState {
        id,
        // SAFETY: a static object whose release does nothing.
        gamepad: unsafe { ComPtr::from_raw(object) }.unwrap(),
        state: GamepadReading::default(),
        correlated_context: None,
        used,
        connected,
        correlation_id: 0,
    }
}

#[test]
fn the_only_available_gamepad_is_the_one_correlated() {
    let mut s = RawinputState::new();
    s.wgi_state.per_gamepad = vec![
        gamepad(1, true, false),
        gamepad(2, true, true),
        gamepad(3, false, false),
    ];
    let (guessed, correlation_id, slot) = s.guess_wgi_slot(&WindowsMatchState::default(), false);
    assert_eq!((guessed, correlation_id, slot), (true, 1, Some(1)));
    let ids: Vec<u8> = s
        .wgi_state
        .per_gamepad
        .iter()
        .map(|g| g.correlation_id)
        .collect();
    assert_eq!(ids, [1, 0, 0]);

    // and again: the same gamepad, one more
    let (guessed, correlation_id, slot) = s.guess_wgi_slot(&WindowsMatchState::default(), false);
    assert_eq!((guessed, correlation_id, slot), (true, 2, Some(1)));
}

fn context(xinput: bool) -> DeviceContext {
    let device = Arc::new(RawinputDevice {
        name: String::new(),
        path: String::new(),
        vendor_id: 0x045e,
        product_id: 0x02a1,
        version: 0,
        guid: Guid::ZERO,
        is_xinput: xinput,
        is_xboxone: false,
        steam_virtual_gamepad_slot: -1,
        preparsed_data: Vec::new(),
        h_device: 1,
        joystick_id: 1,
    });
    DeviceContext::new(1, &device)
}

#[test]
fn closing_a_joystick_releases_only_the_wgi_reference_it_took() {
    let mut s = RawinputState::new();

    // Not an XInput device (or SDL_JOYSTICK_WGI off): no reference taken,
    // none given back.
    let ctx = context(false);
    s.release_context(&ctx);
    assert_eq!(s.wgi_state.ref_count, 0);

    // One that took one gives it back.
    s.wgi_state.ref_count = 1;
    let mut ctx = context(true);
    ctx.wgi_enabled = true;
    s.release_context(&ctx);
    assert_eq!(s.wgi_state.ref_count, 0);
}
