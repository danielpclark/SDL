// Rust translation of the JavaScript in src/joystick/emscripten/SDL_sysjoystick.c
// from Simple DirectMedia Layer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! The JavaScript of the Gamepad API joystick driver,
//! `joystick/emscripten/SDL_sysjoystick.c` (its `EM_JS_DEPS`,
//! `$stringToUTF8`, is in the dependency table).
//!
//! Each function is upstream's `MAIN_THREAD_EM_ASM` body, verbatim (in the
//! braces emscripten puts around an `EM_ASM`'s code): without threads the
//! main thread is the only one. They look the gamepad up themselves, by its
//! index in `navigator.getGamepads()` (the array as it is now, not the one
//! `emscripten_sample_gamepad_data()` kept); where there's no Gamepad API
//! (under node) `navigator.getGamepads` isn't a function and they throw a
//! TypeError, but the driver calls them only for gamepads the API reported.

use core::ffi::c_char;

// SDL_GetEmscriptenJoystickVendor()'s MAIN_THREAD_EM_ASM_INT, verbatim.
crate::em_js_dollar!(
    /// The USB vendor ID in the `id` of the gamepad at `index`: Chrome's
    /// `"… (… Vendor: 054c Product: 09cc)"`, or Firefox's and Safari's
    /// `"046d-c216-…"`; 0 when there's none (or no gamepad).
    pub fn sdl3_joystick_vendor(index: i32) -> i32,
    r#"{
        let gamepad = navigator['getGamepads']()[$0];
        if (!gamepad) {
            return 0;
        }

        // Chrome, Edge, Opera: Wireless Controller (STANDARD GAMEPAD Vendor: 054c Product: 09cc)
        let vendor_str = 'Vendor: ';
        if (gamepad['id']['indexOf'](vendor_str) > 0) {
            let vendor_str_index = gamepad['id']['indexOf'](vendor_str) + vendor_str['length'];
            return parseInt(gamepad['id']['substr'](vendor_str_index, 4), 16);
        }

        // Firefox, Safari: 046d-c216-Logitech Dual Action (or 46d-c216-Logicool Dual Action)
        let id_split = gamepad['id']['split']('-');
        if (id_split['length'] > 1 && !isNaN(parseInt(id_split[0], 16))) {
            return parseInt(id_split[0], 16);
        }

        return 0;
    }"#
);

// SDL_GetEmscriptenJoystickProduct()'s MAIN_THREAD_EM_ASM_INT, verbatim.
crate::em_js_dollar!(
    /// The USB product ID in the `id` of the gamepad at `index` (as
    /// [`sdl3_joystick_vendor`] finds the vendor's); 0 when there's none.
    pub fn sdl3_joystick_product(index: i32) -> i32,
    r#"{
        let gamepad = navigator['getGamepads']()[$0];
        if (!gamepad) {
            return 0;
        }

        // Chrome, Edge, Opera: Wireless Controller (STANDARD GAMEPAD Vendor: 054c Product: 09cc)
        let product_str = 'Product: ';
        if (gamepad['id']['indexOf'](product_str) > 0) {
            let product_str_index = gamepad['id']['indexOf'](product_str) + product_str['length'];
            return parseInt(gamepad['id']['substr'](product_str_index, 4), 16);
        }

        // Firefox, Safari: 046d-c216-Logitech Dual Action (or 46d-c216-Logicool Dual Action)
        let id_split = gamepad['id']['split']('-');
        if (id_split['length'] > 1 && !isNaN(parseInt(id_split[1], 16))) {
            return parseInt(id_split[1], 16);
        }

        return 0;
    }"#
);

// SDL_IsEmscriptenJoystickXInput()'s MAIN_THREAD_EM_ASM_INT, verbatim.
crate::em_js_dollar!(
    /// Whether the `id` of the gamepad at `index` says XInput (1): Chrome's
    /// `"… (XInput STANDARD GAMEPAD)"`, Firefox's `"xinput"`.
    pub fn sdl3_joystick_is_xinput(index: i32) -> i32,
    r#"{
        let gamepad = navigator['getGamepads']()[$0];
        if (!gamepad) {
            return 0;
        }

        // Chrome, Edge, Opera: Xbox 360 Controller (XInput STANDARD GAMEPAD)
        // Firefox: xinput
        // TODO: Safari
        return gamepad['id']['toLowerCase']()['indexOf']('xinput') >= 0;
    }"#
);

// SDL_GetEmscriptenOSID()'s MAIN_THREAD_EM_ASM_INT, verbatim.
crate::em_js_dollar!(
    /// The operating system `navigator.userAgent` names first in upstream's
    /// list: 1 Android, 2 Linux, 3 iPhone, 4 Macintosh, 5 Windows; 0 for
    /// none of them.
    pub fn sdl3_joystick_os_id() -> i32,
    r#"{
        const os = ([
            'Android',
            'Linux',
            'iPhone',
            'Macintosh',
            'Windows',
        ]);
        const ua = navigator['userAgent'];
        for (let i = 0; i < os.length; i++) {
            if (ua['indexOf'](os[i]) >= 0) {
                return i + 1;
            }
        }
        return 0;
    }"#
);

// SDL_GetEmscriptenNormalizedName()'s MAIN_THREAD_EM_ASM, verbatim.
crate::em_js_dollar!(
    /// Writes the name in the `id` of the gamepad at `index` into the `len`
    /// bytes at `out`, NUL-terminated UTF-8 (cut at a character before
    /// `len - 1` bytes): Chrome's id without its `" (STANDARD GAMEPAD …)"`,
    /// `" (Vendor: …)"` or `" (XInput …)"`, Firefox's and Safari's after
    /// the vendor and product IDs, trimmed; an empty string without a
    /// gamepad.
    pub fn sdl3_joystick_normalized_name(index: i32, out: *mut c_char, len: i32),
    r#"{
        let gamepad = navigator['getGamepads']()[$0];
        if (!gamepad) {
            stringToUTF8('\0', $1, $2); // Silence the compiler here, because '' is not valid
            return;
        }

        let id = gamepad['id'];
        let output = id;
        // Chrome
        if (id['indexOf'](' (STANDARD GAMEPAD') > 0) { // Wireless Controller (STANDARD GAMEPAD Vendor: 054c Product: 09cc)
            output = id['substr'](0, id['indexOf'](' (STANDARD GAMEPAD'));
        } else if (id['indexOf'](' (Vendor:') > 0) { // usb gamepad            (Vendor: 0810 Product: e501)
            output = id['substr'](0, id['indexOf'](' (Vendor:'));
        } else if (id['indexOf'](' (XInput') > 0) { // Xbox 360 Controller (XInput STANDARD GAMEPAD)
            output = id['substr'](0, id['indexOf'](' (XInput'));
        }

        // Firefox, Safari: "046d-c216-Logitech Dual Action", "46d-c216-Logicool Dual Action", or "xinput"
        let id_split = id['split']('-');
        if (id_split['length'] > 1 && !isNaN(parseInt(id_split[0], 16))) {
            // Let's not assume the length of the vendor/product IDs in the string
            // and just find the second '-' using indexOf
            let start = id['indexOf']('-', id['indexOf']('-')+1)+1;
            output = id['substr'](start);
        }

        stringToUTF8(output.trim(), $1, $2);
    }"#
);

// Emscripten_JoyStickConnected()'s MAIN_THREAD_EM_ASM_INT, verbatim.
crate::em_js_dollar!(
    /// Whether the gamepad at `index` can rumble its triggers (1): its
    /// `vibrationActuator`'s `effects` include `'trigger-rumble'`.
    pub fn sdl3_joystick_trigger_rumble_available(index: i32) -> i32,
    r#"{
        let gamepad = navigator['getGamepads']()[$0];
        // This effect is not supported in Safari, so it's okay for us to check the vibrationActuator.effects array here for the browsers that do support it
        if (!gamepad || !gamepad['vibrationActuator'] || !gamepad['vibrationActuator']['effects'] || !gamepad['vibrationActuator']['effects']['includes']('trigger-rumble')) {
            return false;
        }
        return true;
        }"#
);

// EMSCRIPTEN_JoystickOpen()'s MAIN_THREAD_EM_ASM_INT, verbatim.
crate::em_js_dollar!(
    /// Whether the gamepad at `index` can rumble (1): it has a
    /// `vibrationActuator`.
    pub fn sdl3_joystick_rumble_available(index: i32) -> i32,
    r#"{
        let gamepad = navigator['getGamepads']()[$0];
        // Don't check the vibrationActuator.effects array here, because it's not defined in Safari
        if (!gamepad || !gamepad['vibrationActuator']) {
            return false;
        }
        return true;
        }"#
);

// Emscripten_UpdateRumble()'s MAIN_THREAD_EM_ASM_INT, verbatim.
crate::em_js_dollar!(
    /// Plays a `'dual-rumble'` effect of 3 seconds on the gamepad at
    /// `index`'s `vibrationActuator`, with the four magnitudes (0 to
    /// 0xFFFF) as fractions; whether there was a gamepad (1). Throws if it
    /// has no `vibrationActuator`.
    pub fn sdl3_joystick_update_rumble(
        index: i32,
        weak_magnitude: i32,
        strong_magnitude: i32,
        left_trigger: i32,
        right_trigger: i32
    ) -> i32,
    r#"{
        let gamepad = navigator['getGamepads']()[$0];
        if (!gamepad) {
            return false;
        }
        // We check if rumble is available in EMSCRIPTEN_JoystickRumble() and EMSCRIPTEN_JoystickRumbleTriggers().
        // From my testing using "dual-rumble" here covers both main rumble and trigger rumble.
        gamepad['vibrationActuator']['playEffect']('dual-rumble', {
            'startDelay': 0,
            'duration': 3000,
            'weakMagnitude': $1 / 0xFFFF,
            'strongMagnitude': $2 / 0xFFFF,
            'leftTrigger': $3 / 0xFFFF,
            'rightTrigger': $4 / 0xFFFF,
        });

        return true;
        }"#
);
