// Tests for the RawInput joystick driver's bookkeeping: the WGI reference
// count (run under Wine on Linux).
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

use super::*;

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
