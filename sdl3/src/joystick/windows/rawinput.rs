// Rust translation of src/joystick/windows/SDL_rawinputjoystick.c and
// SDL_rawinputjoystick_c.h from Simple DirectMedia Layer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! RAWINPUT Joystick API for better handling XInput-capable devices on
//! Windows.
//!
//! XInput is limited to 4 devices.
//! Windows.Gaming.Input does not get inputs from XBox One controllers when
//! not in the foreground.
//! DirectInput does not get inputs from XBox One controllers when not in
//! the foreground, nor rumble or accurate triggers.
//! RawInput does not get rumble or accurate triggers.
//!
//! So, combine them as best we can! The HID reports of XInput-capable
//! devices ("IG_" in their path) come as `WM_INPUT` messages to the
//! Windows driver's message window; each joystick is correlated with an
//! XInput slot and a Windows.Gaming.Input gamepad by matching their states,
//! for the guide button, separate triggers and rumble. This is the
//! configuration upstream builds with `xinput.h` and
//! `windows.gaming.input.h` (`SDL_JOYSTICK_RAWINPUT_XINPUT`,
//! `SDL_JOYSTICK_RAWINPUT_WGI`, so `SDL_JOYSTICK_RAWINPUT_MATCH_AXES` and
//! `SDL_JOYSTICK_RAWINPUT_MATCH_TRIGGERS`).
//!
//! The driver is off unless the `SDL_JOYSTICK_RAWINPUT` hint is set.

use std::cell::RefCell;
use std::ffi::CString;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use windows_sys::Win32::Foundation::{
    CloseHandle, GENERIC_READ, HANDLE, HWND, INVALID_HANDLE_VALUE, LPARAM, LRESULT, WPARAM,
};
use windows_sys::Win32::Storage::FileSystem::{
    CreateFileA, FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING,
};
use windows_sys::Win32::UI::Input::{
    GetRawInputData, GetRawInputDeviceInfoA, GetRawInputDeviceList, RegisterRawInputDevices,
    HRAWINPUT, RAWINPUT, RAWINPUTDEVICE, RAWINPUTDEVICELIST, RAWINPUTHEADER, RIDEV_DEVNOTIFY,
    RIDEV_INPUTSINK, RIDEV_REMOVE, RIDI_DEVICEINFO, RIDI_DEVICENAME, RIDI_PREPARSEDDATA,
    RID_DEVICE_INFO, RID_INPUT, RIM_TYPEHID,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    CallWindowProcW, DefWindowProcW, GetSystemMetrics, SM_REMOTESESSION,
};

use super::super::gamepad::{GamepadButton, GamepadMapping};
use super::super::usb_ids::{
    USB_PRODUCT_STEAM_VIRTUAL_GAMEPAD, USB_PRODUCT_XBOX360_XUSB_CONTROLLER,
    USB_PRODUCT_XBOX_ONE_XBOXGIP_CONTROLLER, USB_USAGEPAGE_BUTTON, USB_USAGEPAGE_GENERIC_DESKTOP,
    USB_USAGE_GENERIC_GAMEPAD, USB_USAGE_GENERIC_HAT, USB_USAGE_GENERIC_Z, USB_VENDOR_MICROSOFT,
    USB_VENDOR_VALVE,
};
use super::super::{
    create_joystick_guid, create_joystick_name, is_joystick_xbox_one,
    joystick_handled_by_another_driver, lock_joysticks, private_joystick_added,
    private_joystick_removed, send_joystick_axis, send_joystick_button, send_joystick_hat,
    send_joystick_power_info, should_ignore_joystick, with_joystick, JoystickData, JoystickDriver,
    HARDWARE_BUS_USB, HAT_CENTERED, HAT_DOWN, HAT_LEFT, HAT_RIGHT, HAT_UP,
    PROP_JOYSTICK_CAP_RUMBLE_BOOLEAN, PROP_JOYSTICK_CAP_TRIGGER_RUMBLE_BOOLEAN,
    RAWINPUT_DRIVER_INDEX,
};
use super::wgi_abi::*;
use crate::core::windows::com::ComPtr;
use crate::core::windows::hid::{
    hid, load_hid_dll, unload_hid_dll, HIDP_DATA, HIDP_REPORT_TYPE, HIDP_VALUE_CAPS,
};
use crate::core::windows::xinput::{
    load_xinput_dll, unload_xinput_dll, xinput, BATTERY_DEVTYPE_GAMEPAD, BATTERY_TYPE_UNKNOWN,
    XINPUT_BATTERY_INFORMATION_EX, XINPUT_FLAG_GAMEPAD, XINPUT_GAMEPAD, XINPUT_GAMEPAD_GUIDE,
    XINPUT_STATE, XINPUT_VIBRATION, XUSER_INDEX_ANY, XUSER_MAX_COUNT,
};
use crate::core::windows::{
    is_windows_vista_or_greater, ro_initialize, ro_uninitialize, wide_to_utf8,
};
use crate::error::{Error, Result};
use crate::events::JoystickID;
use crate::guid::Guid;
use crate::hints;
use crate::power::PowerState;
use crate::thread::ReentrantMutex;

/// `WM_INPUT_DEVICE_CHANGE`
const WM_INPUT_DEVICE_CHANGE: u32 = 0x00FE;
/// `WM_INPUT`
const WM_INPUT: u32 = 0x00FF;
/// `GIDC_ARRIVAL`
const GIDC_ARRIVAL: usize = 1;
/// `GIDC_REMOVAL`
const GIDC_REMOVAL: usize = 2;
/// `USB_PACKET_LENGTH`
const USB_PACKET_LENGTH: usize = 64;
/// `MAX_PATH`
const MAX_PATH: usize = 260;

/// stick + trigger axes. Translation of `SDL_JOYSTICK_RAWINPUT_MATCH_COUNT`.
const MATCH_COUNT: usize = 6;

/// Translation of `SDL_RAWINPUT_inited`.
static INITED: AtomicBool = AtomicBool::new(false);
/// Translation of `SDL_RAWINPUT_remote_desktop`.
static REMOTE_DESKTOP: AtomicBool = AtomicBool::new(false);

/// A RawInput device. Translation of `SDL_RAWINPUT_Device`, reference
/// counted (`refcount`) by its `Arc`; the joystick open on it
/// (`device->joystick`) is the context whose `device` is it.
#[derive(Debug)]
pub(super) struct RawinputDevice {
    name: String,
    path: String,
    vendor_id: u16,
    product_id: u16,
    #[allow(dead_code)] // (upstream only logs it)
    version: u16,
    guid: Guid,
    is_xinput: bool,
    is_xboxone: bool,
    steam_virtual_gamepad_slot: i32,
    /// The preparsed data, as `GetRawInputDeviceInfo()` returns it.
    preparsed_data: Vec<u8>,

    /// (`hDevice`, as an address)
    h_device: usize,
    joystick_id: JoystickID,
}

/// The private structure used to keep track of a joystick. Translation of
/// `struct joystick_hwdata` (`RAWINPUT_DeviceContext`).
struct DeviceContext {
    /// The joystick this belongs to (`device->joystick`).
    joystick: JoystickID,
    /// Its button, axis and hat counts (`joystick->nbuttons` and so on).
    nbuttons: usize,
    naxes: usize,
    nhats: usize,

    is_xinput: bool,
    is_xboxone: bool,
    max_data_length: u32,
    data: Vec<HIDP_DATA>,
    button_indices: Vec<u16>,
    axis_indices: Vec<u16>,
    hat_indices: Vec<u16>,
    guide_hack: bool,
    trigger_hack: bool,
    trigger_hack_index: u16,

    /// Lowest 16 bits for button states, higher 24 for 6 4bit axes
    match_state: u64,
    last_state_packet: u64,

    xinput_enabled: bool,
    xinput_correlated: bool,
    xinput_correlation_id: u8,
    xinput_correlation_count: u8,
    xinput_uncorrelate_count: u8,
    xinput_slot: u8,

    wgi_correlated: bool,
    wgi_correlation_id: u8,
    wgi_correlation_count: u8,
    wgi_uncorrelate_count: u8,
    /// The gamepad state (`wgi_slot`), by its id.
    wgi_slot: Option<u64>,
    vibration: GamepadVibration,

    triggers_rumbling: bool,

    device: Arc<RawinputDevice>,
}

/// Translation of `guide_button_candidate`.
#[derive(Default)]
struct GuideButtonCandidate {
    last_state_packet: u64,
    joystick: Option<JoystickID>,
    last_joystick: Option<JoystickID>,
}

/// The states compared to find the XInput slot and the WGI gamepad of a
/// device. Translation of `WindowsMatchState`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) struct WindowsMatchState {
    pub(super) match_axes: [i16; MATCH_COUNT],
    pub(super) xinput_buttons: u16,
    pub(super) wgi_buttons: u32,
    pub(super) any_data: bool,
}

/// An XInput slot. Translation of an element of `xinput_state`.
#[derive(Clone, Copy, Default)]
struct XInputSlotState {
    state: XINPUT_STATE,
    battery: XINPUT_BATTERY_INFORMATION_EX,
    /// Currently has an active XInput device
    connected: bool,
    /// Is currently mapped to an SDL device
    used: bool,
    correlation_id: u8,
}

/// A WGI gamepad. Translation of `WindowsGamingInputGamepadState`.
struct WgiGamepadState {
    /// Identifies the state (upstream uses its address).
    id: u64,
    gamepad: ComPtr<IGamepadVtbl>,
    state: GamepadReading,
    /// The joystick of the context correlated to it.
    correlated_context: Option<JoystickID>,
    /// Is currently mapped to an SDL device
    used: bool,
    /// Just used during update to track disconnected
    connected: bool,
    correlation_id: u8,
}

/// Translation of `wgi_state` (`need_device_list_update` is
/// [`WGI_NEED_DEVICE_LIST_UPDATE`], set by the event handlers).
#[derive(Default)]
struct WgiState {
    per_gamepad: Vec<WgiGamepadState>,
    initialized: bool,
    dirty: bool,
    ref_count: i32,
    gamepad_statics: Option<ComPtr<IGamepadStaticsVtbl>>,
    gamepad_added_token: EventRegistrationToken,
    gamepad_removed_token: EventRegistrationToken,
    next_id: u64,
}

/// `wgi_state.need_device_list_update`
static WGI_NEED_DEVICE_LIST_UPDATE: AtomicBool = AtomicBool::new(false);

/// The driver's state.
struct RawinputState {
    /// Translation of `SDL_RAWINPUT_devices` (`SDL_RAWINPUT_numjoysticks` is
    /// its length).
    devices: Vec<Arc<RawinputDevice>>,
    /// The open joysticks' contexts.
    contexts: Vec<DeviceContext>,
    guide_button_candidate: GuideButtonCandidate,
    xinput_state: [XInputSlotState; XUSER_MAX_COUNT as usize],
    xinput_device_change: bool,
    xinput_state_dirty: bool,
    wgi_state: WgiState,
}

/// Guarded by the joystick lock upstream; the `RefCell` borrow is never
/// held across an event push or a call back into the joystick API.
static STATE: ReentrantMutex<RefCell<Option<RawinputState>>> =
    ReentrantMutex::new(RefCell::new(None));

fn with_state<R>(f: impl FnOnce(&mut RawinputState) -> R) -> R {
    let guard = STATE.lock();
    let mut state = guard.borrow_mut();
    let state = state.get_or_insert_with(|| RawinputState {
        devices: Vec::new(),
        contexts: Vec::new(),
        guide_button_candidate: GuideButtonCandidate::default(),
        xinput_state: [XInputSlotState::default(); XUSER_MAX_COUNT as usize],
        xinput_device_change: true,
        xinput_state_dirty: true,
        wgi_state: WgiState::default(),
    });
    f(state)
}

/// An event to send once the state isn't borrowed.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) enum Event {
    Button {
        timestamp: u64,
        button: u8,
        down: bool,
    },
    Axis {
        timestamp: u64,
        axis: u8,
        value: i16,
    },
    Hat {
        timestamp: u64,
        hat: u8,
        value: u8,
    },
    Power {
        state: PowerState,
        percent: i32,
    },
}

/// Send the events of a joystick.
fn send_events(joystick: JoystickID, events: Vec<Event>) {
    for event in events {
        match event {
            Event::Button {
                timestamp,
                button,
                down,
            } => send_joystick_button(timestamp, joystick, button, down),
            Event::Axis {
                timestamp,
                axis,
                value,
            } => send_joystick_axis(timestamp, joystick, axis, value),
            Event::Hat {
                timestamp,
                hat,
                value,
            } => send_joystick_hat(timestamp, joystick, hat, value),
            Event::Power { state, percent } => send_joystick_power_info(joystick, state, percent),
        }
    }
}

/// Translation of `RAWINPUT_FillMatchState()`.
pub(super) fn fill_match_state(match_state: u64) -> WindowsMatchState {
    let mut state = WindowsMatchState::default();
    let mut any_axes_data = false;
    /*  SHORT state->match_axes[4] = {
        (match_state & 0x000F0000) >> 4,
        (match_state & 0x00F00000) >> 8,
        (match_state & 0x0F000000) >> 12,
        (match_state & 0xF0000000) >> 16,
    }; */
    for ii in 0..4 {
        state.match_axes[ii] = ((match_state & (0x000F0000u64 << (ii * 4))) >> (4 + ii * 4)) as i16;
        any_axes_data |= (state.match_axes[ii] as i32 + 0x1000) as u32 > 0x2000;
        // match_state bit is not 0xF, 0x1, or 0x2
    }
    for ii in 4..MATCH_COUNT {
        state.match_axes[ii] = ((match_state & (0x000F0000u64 << (ii * 4))) >> (4 + ii * 4)) as i16;
        any_axes_data |= state.match_axes[ii] != i16::MIN;
    }

    state.any_data = any_axes_data;

    state.xinput_buttons =
        // Bitwise map .RLDUWVQTS.KYXBA -> YXBA..WVQTKSRLDU
        (match_state << 12 | (match_state & 0x0780) >> 1 | (match_state & 0x0010) << 1 | (match_state & 0x0040) >> 2 | (match_state & 0x7800) >> 11) as u16;

    if state.xinput_buttons != 0 {
        state.any_data = true;
    }

    state.wgi_buttons =
        // Bitwise map .RLD UWVQ TS.K YXBA -> ..QT WVRL DUYX BAKS
        // RStick/LStick (QT)         RShould/LShould  (WV)                 DPad R/L/D/U                          YXBA                         bac(K)                      (S)tart
        ((match_state & 0x0180) << 5 | (match_state & 0x0600) << 1 | (match_state & 0x7800) >> 5 | (match_state & 0x000F) << 2 | (match_state & 0x0010) >> 3 | (match_state & 0x0040) >> 6) as u32;

    if state.wgi_buttons != 0 {
        state.any_data = true;
    }
    state
}

/// Match axes by checking if the distance between the high 4 bits of axis
/// and the 4 bits from match_state is 1 or less. Translation of
/// `XInputAxesMatch()`.
pub(super) fn xinput_axes_match(gamepad: &XINPUT_GAMEPAD, state: &WindowsMatchState) -> bool {
    let near = |value: i32, match_axis: i16| (value - match_axis as i32 + 0x1000) as u32 <= 0x2fff;
    near(gamepad.sThumbLX as i32, state.match_axes[0])
        && near(!(gamepad.sThumbLY as i32), state.match_axes[1])
        && near(gamepad.sThumbRX as i32, state.match_axes[2])
        && near(!(gamepad.sThumbRY as i32), state.match_axes[3])
}

/// Can only match trigger values if a single trigger has a value.
/// Translation of `XInputTriggersMatch()`.
pub(super) fn xinput_triggers_match(gamepad: &XINPUT_GAMEPAD, state: &WindowsMatchState) -> bool {
    (state.match_axes[4] == i16::MIN && state.match_axes[5] == i16::MIN)
        || (gamepad.bLeftTrigger != 0 && gamepad.bRightTrigger != 0)
        || ((gamepad.bLeftTrigger as i32 * 257 - 32768) - state.match_axes[4] as i32) as u32
            <= 0x2fff
        || ((gamepad.bRightTrigger as i32 * 257 - 32768) - state.match_axes[5] as i32) as u32
            <= 0x2fff
}

/// Match axes by checking if the distance between the high 4 bits of axis
/// and the 4 bits from match_state is 1 or less. Translation of
/// `WindowsGamingInputAxesMatch()`.
pub(super) fn wgi_axes_match(gamepad: &GamepadReading, state: &WindowsMatchState) -> bool {
    let near = |value: i32, match_axis: i16| {
        ((value & 0xF000) - match_axis as i32 + 0x1000) as u16 <= 0x2fff
    };
    let axis = |value: f64| (value * i16::MAX as f64) as i16 as i32;
    near(axis(gamepad.LeftThumbstickX), state.match_axes[0])
        && near(!axis(gamepad.LeftThumbstickY), state.match_axes[1])
        && near(axis(gamepad.RightThumbstickX), state.match_axes[2])
        && near(!axis(gamepad.RightThumbstickY), state.match_axes[3])
}

/// Translation of `WindowsGamingInputTriggersMatch()`.
pub(super) fn wgi_triggers_match(gamepad: &GamepadReading, state: &WindowsMatchState) -> bool {
    let trigger = |value: f64, match_axis: i16| {
        (((value * u16::MAX as f64) as i32 - 32768) - match_axis as i32) as u16 <= 0x2fff
    };
    (state.match_axes[4] == i16::MIN && state.match_axes[5] == i16::MIN)
        || (gamepad.LeftTrigger == 0.0 && gamepad.RightTrigger == 0.0)
        || trigger(gamepad.LeftTrigger, state.match_axes[4])
        || trigger(gamepad.RightTrigger, state.match_axes[5])
}

impl RawinputState {
    /// Translation of `RAWINPUT_UpdateXInput()`.
    fn update_xinput(&mut self) {
        let Some(x) = xinput() else {
            return;
        };
        if self.xinput_device_change {
            for (user_index, slot) in self.xinput_state.iter_mut().enumerate() {
                slot.connected = x.get_capabilities(user_index as u32, XINPUT_FLAG_GAMEPAD).0 == 0;
            }
            self.xinput_device_change = false;
            self.xinput_state_dirty = true;
        }
        if self.xinput_state_dirty {
            self.xinput_state_dirty = false;
            for (user_index, slot) in self.xinput_state.iter_mut().enumerate() {
                if slot.connected {
                    let (result, state) = x.get_state(user_index as u32);
                    if result == 0 {
                        slot.state = state;
                    } else {
                        slot.connected = false;
                    }
                    slot.battery.BatteryType = BATTERY_TYPE_UNKNOWN;
                    if let Some((_, battery)) =
                        x.get_battery_information(user_index as u32, BATTERY_DEVTYPE_GAMEPAD)
                    {
                        slot.battery = battery;
                    }
                }
            }
        }
    }

    /// Translation of `RAWINPUT_MarkXInputSlotUsed()`.
    fn mark_xinput_slot_used(&mut self, xinput_slot: u8) {
        if xinput_slot as u32 != XUSER_INDEX_ANY {
            self.xinput_state[xinput_slot as usize % 4].used = true;
        }
    }

    /// Translation of `RAWINPUT_MarkXInputSlotFree()`.
    fn mark_xinput_slot_free(&mut self, xinput_slot: u8) {
        if xinput_slot as u32 != XUSER_INDEX_ANY {
            self.xinput_state[xinput_slot as usize % 4].used = false;
        }
    }

    /// Translation of `RAWINPUT_MissingXInputSlot()`.
    fn missing_xinput_slot(&self) -> bool {
        self.xinput_state.iter().any(|s| s.connected && !s.used)
    }

    /// Translation of `RAWINPUT_XInputSlotMatches()`.
    fn xinput_slot_matches(&self, state: &WindowsMatchState, slot_idx: u8) -> bool {
        let Some(slot) = self.xinput_state.get(slot_idx as usize) else {
            return false;
        };
        if slot.connected {
            let xinput_buttons = slot.state.Gamepad.wButtons;
            if (xinput_buttons & !XINPUT_GAMEPAD_GUIDE) == state.xinput_buttons
                && xinput_axes_match(&slot.state.Gamepad, state)
                && xinput_triggers_match(&slot.state.Gamepad, state)
            {
                return true;
            }
        }
        false
    }

    /// Translation of `RAWINPUT_GuessXInputSlot()`: whether exactly one
    /// slot matches, with the correlation id and the slot.
    fn guess_xinput_slot(&mut self, state: &WindowsMatchState) -> (bool, u8, u8) {
        let mut correlation_id = 0;
        let mut slot_idx = 0;

        /* If there is only one available slot, let's use that
         * That will be right most of the time, and uncorrelation will fix any bad guesses
         */
        let mut match_count = 0;
        for (user_index, slot) in self.xinput_state.iter().enumerate() {
            if slot.connected && !slot.used {
                slot_idx = user_index as u8;
                match_count += 1;
            }
        }
        if match_count == 1 {
            let slot = &mut self.xinput_state[slot_idx as usize];
            slot.correlation_id = slot.correlation_id.wrapping_add(1);
            return (true, slot.correlation_id, slot_idx);
        }

        slot_idx = 0;

        match_count = 0;
        for user_index in 0..XUSER_MAX_COUNT as u8 {
            if !self.xinput_state[user_index as usize].used
                && self.xinput_slot_matches(state, user_index)
            {
                match_count += 1;
                slot_idx = user_index;
                // Incrementing correlation_id for any match, as negative evidence for others being correlated
                let slot = &mut self.xinput_state[user_index as usize];
                slot.correlation_id = slot.correlation_id.wrapping_add(1);
                correlation_id = slot.correlation_id;
            }
        }
        /* Only return a match if we match exactly one, and we have some non-zero data (buttons or axes) that matched.
        Note that we're still invalidating *other* potential correlations if we have more than one match or we have no
        data. */
        (match_count == 1 && state.any_data, correlation_id, slot_idx)
    }

    /// The context of an open joystick.
    fn context(&mut self, joystick: JoystickID) -> Option<&mut DeviceContext> {
        self.contexts.iter_mut().find(|c| c.joystick == joystick)
    }

    /// The WGI gamepad with this id.
    fn wgi_gamepad(&self, id: Option<u64>) -> Option<&WgiGamepadState> {
        let id = id?;
        self.wgi_state.per_gamepad.iter().find(|g| g.id == id)
    }

    /// Translation of `RAWINPUT_MarkWindowsGamingInputSlotUsed()`.
    fn mark_wgi_slot_used(&mut self, wgi_slot: Option<u64>, ctx: JoystickID) {
        if let Some(slot) = self
            .wgi_state
            .per_gamepad
            .iter_mut()
            .find(|g| Some(g.id) == wgi_slot)
        {
            slot.used = true;
            slot.correlated_context = Some(ctx);
        }
    }

    /// Translation of `RAWINPUT_MarkWindowsGamingInputSlotFree()`.
    fn mark_wgi_slot_free(&mut self, wgi_slot: Option<u64>) {
        if let Some(slot) = self
            .wgi_state
            .per_gamepad
            .iter_mut()
            .find(|g| Some(g.id) == wgi_slot)
        {
            slot.used = false;
            slot.correlated_context = None;
        }
    }

    /// Translation of `RAWINPUT_MissingWindowsGamingInputSlot()`.
    fn missing_wgi_slot(&self) -> bool {
        self.wgi_state.per_gamepad.iter().any(|g| !g.used)
    }

    /// Translation of `RAWINPUT_UpdateWindowsGamingInput()`.
    fn update_windows_gaming_input(&mut self) -> bool {
        let Some(statics) = self.wgi_state.gamepad_statics.clone() else {
            return true;
        };

        if !self.wgi_state.dirty {
            return true;
        }

        self.wgi_state.dirty = false;

        if WGI_NEED_DEVICE_LIST_UPDATE.swap(false, Ordering::AcqRel) {
            for gamepad_state in &mut self.wgi_state.per_gamepad {
                gamepad_state.connected = false;
            }

            // SAFETY: get_Gamepads stores a vector view on success.
            let gamepads = unsafe {
                ComPtr::from_out(|out| (statics.vtbl().get_Gamepads)(statics.as_ptr(), out))
            };
            if let Ok(gamepads) = gamepads {
                let mut num_gamepads = 0;
                // SAFETY: get_Size writes the count.
                let hr =
                    unsafe { (gamepads.vtbl().get_Size)(gamepads.as_ptr(), &mut num_gamepads) };
                if hr >= 0 {
                    for i in 0..num_gamepads {
                        // SAFETY: GetAt stores the gamepad on success.
                        let gamepad = unsafe {
                            ComPtr::from_out(|out| {
                                (gamepads.vtbl().GetAt)(gamepads.as_ptr(), i, out)
                            })
                        };
                        if let Ok(gamepad) = gamepad {
                            if let Some(state) = self
                                .wgi_state
                                .per_gamepad
                                .iter_mut()
                                .find(|g| g.gamepad.same(&gamepad))
                            {
                                state.connected = true;
                                // Already tracked (the new reference is released)
                            } else {
                                // New device, add it
                                self.wgi_state.next_id += 1;
                                self.wgi_state.per_gamepad.push(WgiGamepadState {
                                    id: self.wgi_state.next_id,
                                    gamepad,
                                    state: GamepadReading::default(),
                                    correlated_context: None,
                                    used: false,
                                    connected: true,
                                    correlation_id: 0,
                                });
                            }
                        }
                    }
                    for ii in (0..self.wgi_state.per_gamepad.len()).rev() {
                        if !self.wgi_state.per_gamepad[ii].connected {
                            // Device missing, must be disconnected
                            let gamepad_state = self.wgi_state.per_gamepad.swap_remove(ii);
                            if let Some(ctx) = gamepad_state
                                .correlated_context
                                .and_then(|c| self.context(c))
                            {
                                ctx.wgi_correlated = false;
                                ctx.wgi_slot = None;
                            }
                        }
                    }
                }
            }
        } // need_device_list_update

        for gamepad_state in &mut self.wgi_state.per_gamepad {
            let gamepad = &gamepad_state.gamepad;
            // SAFETY: GetCurrentReading writes the reading.
            let hr = unsafe {
                (gamepad.vtbl().GetCurrentReading)(gamepad.as_ptr(), &mut gamepad_state.state)
            };
            if hr < 0 {
                gamepad_state.connected = false; // Not used by anything, currently
            }
        }
        true
    }

    /// Translation of `RAWINPUT_InitWindowsGamingInput()`.
    fn init_windows_gaming_input(&mut self) {
        if !hints::get_bool(hints::JOYSTICK_WGI, true) {
            return;
        }

        self.wgi_state.ref_count += 1;
        if !self.wgi_state.initialized {
            if ro_initialize() < 0 {
                return;
            }
            self.wgi_state.initialized = true;
            self.wgi_state.dirty = true;

            let winrt = winrt();
            if winrt.has_activation() {
                if let Ok(statics) = winrt.get_activation_factory::<IGamepadStaticsVtbl>(
                    RUNTIME_CLASS_GAMEPAD,
                    &IID_IGAMEPADSTATICS,
                ) {
                    WGI_NEED_DEVICE_LIST_UPDATE.store(true, Ordering::Release);

                    // (a failure sets "add_GamepadAdded() failed: 0x%x" or
                    // "add_GamepadRemoved() failed: 0x%x" upstream, which
                    // nobody reads)
                    let mut token = EventRegistrationToken::default();
                    // SAFETY: the delegate is a static COM object.
                    unsafe {
                        (statics.vtbl().add_GamepadAdded)(
                            statics.as_ptr(),
                            GAMEPAD_ADDED.iface(),
                            &mut token,
                        );
                    }
                    self.wgi_state.gamepad_added_token = token;

                    // SAFETY: as above.
                    unsafe {
                        (statics.vtbl().add_GamepadRemoved)(
                            statics.as_ptr(),
                            GAMEPAD_REMOVED.iface(),
                            &mut token,
                        );
                    }
                    self.wgi_state.gamepad_removed_token = token;
                    self.wgi_state.gamepad_statics = Some(statics);
                }
            }
        }
    }

    /// Translation of `RAWINPUT_WindowsGamingInputSlotMatches()`.
    fn wgi_slot_matches(
        state: &WindowsMatchState,
        slot: &WgiGamepadState,
        xinput_correlated: bool,
    ) -> bool {
        let wgi_buttons = slot.state.Buttons;
        (wgi_buttons & 0x3FFF) == state.wgi_buttons
            && wgi_axes_match(&slot.state, state)
            // Don't try to match WGI triggers if getting values from XInput
            && (xinput_correlated || wgi_triggers_match(&slot.state, state))
    }

    /// Translation of `RAWINPUT_GuessWindowsGamingInputSlot()`: whether
    /// exactly one gamepad matches, with the correlation id and the slot.
    fn guess_wgi_slot(
        &mut self,
        state: &WindowsMatchState,
        xinput_correlated: bool,
    ) -> (bool, u8, Option<u64>) {
        let mut correlation_id = 0;
        let mut slot = None;

        /* If there is only one available slot, let's use that
         * That will be right most of the time, and uncorrelation will fix any bad guesses
         */
        let mut match_count = 0;
        for gamepad_state in &self.wgi_state.per_gamepad {
            if gamepad_state.connected && !gamepad_state.used {
                slot = Some(gamepad_state.id);
                match_count += 1;
            }
        }
        if match_count == 1 {
            // FIXME (upstream): this increments the correlation id of the
            // last gamepad in the list (the loop variable), not of the one
            // chosen.
            if let Some(last) = self.wgi_state.per_gamepad.last_mut() {
                last.correlation_id = last.correlation_id.wrapping_add(1);
                correlation_id = last.correlation_id;
            }
            return (true, correlation_id, slot);
        }

        match_count = 0;
        for gamepad_state in &mut self.wgi_state.per_gamepad {
            if Self::wgi_slot_matches(state, gamepad_state, xinput_correlated) {
                match_count += 1;
                slot = Some(gamepad_state.id);
                // Incrementing correlation_id for any match, as negative evidence for others being correlated
                gamepad_state.correlation_id = gamepad_state.correlation_id.wrapping_add(1);
                correlation_id = gamepad_state.correlation_id;
            }
        }
        /* Only return a match if we match exactly one, and we have some non-zero data (buttons or axes) that matched.
        Note that we're still invalidating *other* potential correlations if we have more than one match or we have no
        data. */
        (match_count == 1 && state.any_data, correlation_id, slot)
    }

    /// Translation of `RAWINPUT_QuitWindowsGamingInput()`.
    fn quit_windows_gaming_input(&mut self) {
        // FIXME (upstream): this is called for every closed joystick, while
        // RAWINPUT_InitWindowsGamingInput() only counts a reference when the
        // SDL_JOYSTICK_WGI hint is on, so the count can go negative (and WGI
        // then never be released).
        self.wgi_state.ref_count -= 1;
        if self.wgi_state.ref_count == 0 && self.wgi_state.initialized {
            self.wgi_state.per_gamepad.clear();
            if let Some(statics) = self.wgi_state.gamepad_statics.take() {
                // SAFETY: the tokens came from the add_* calls on statics.
                unsafe {
                    (statics.vtbl().remove_GamepadAdded)(
                        statics.as_ptr(),
                        self.wgi_state.gamepad_added_token,
                    );
                    (statics.vtbl().remove_GamepadRemoved)(
                        statics.as_ptr(),
                        self.wgi_state.gamepad_removed_token,
                    );
                }
            }
            ro_uninitialize();
            self.wgi_state.initialized = false;
        }
    }

    /// The device with this handle. Translation of `RAWINPUT_DeviceFromHandle()`.
    fn device_from_handle(&self, h_device: usize) -> Option<Arc<RawinputDevice>> {
        self.devices
            .iter()
            .find(|d| d.h_device == h_device)
            .cloned()
    }

    /// The context of the joystick open on a device (`device->joystick`).
    fn context_of(&mut self, device: &Arc<RawinputDevice>) -> Option<&mut DeviceContext> {
        self.contexts
            .iter_mut()
            .find(|c| Arc::ptr_eq(&c.device, device))
    }

    /// Translation of `RAWINPUT_ReleaseDevice()` for the device list's
    /// reference (dropping it frees the device with the last one).
    fn release_device(&mut self, device: Arc<RawinputDevice>) {
        let slot = self.context_of(&device).and_then(|ctx| {
            if ctx.xinput_enabled && ctx.xinput_correlated {
                ctx.xinput_correlated = false;
                Some(ctx.xinput_slot)
            } else {
                None
            }
        });
        if let Some(slot) = slot {
            self.mark_xinput_slot_free(slot);
        }
    }
}

/// `IEventHandler_CGamepadVtbl_QueryInterface()`
unsafe extern "system" fn gamepad_query_interface(
    this: *mut IEventHandlerGamepad,
    riid: *const windows_sys::core::GUID,
    ppv_object: *mut *mut std::ffi::c_void,
) -> windows_sys::core::HRESULT {
    // SAFETY: WinRT calls this on our delegate with valid arguments.
    unsafe { delegate_query_interface(this, riid, ppv_object, &IID_IEVENTHANDLER_GAMEPAD) }
}

/// `IEventHandler_CGamepadVtbl_InvokeAdded()` and `_InvokeRemoved()`.
unsafe extern "system" fn gamepad_invoke(
    _this: *mut IEventHandlerGamepad,
    _sender: *mut IInspectable,
    _e: *mut IGamepad,
) -> windows_sys::core::HRESULT {
    WGI_NEED_DEVICE_LIST_UPDATE.store(true, Ordering::Release);
    0
}

static GAMEPAD_ADDED_VTBL: IEventHandlerVtbl<IGamepadVtbl> = IEventHandlerVtbl {
    QueryInterface: gamepad_query_interface,
    AddRef: delegate_add_ref::<IGamepadVtbl>,
    Release: delegate_release::<IGamepadVtbl>,
    Invoke: gamepad_invoke,
};
static GAMEPAD_ADDED: Delegate<IGamepadVtbl> = Delegate::new(&GAMEPAD_ADDED_VTBL);

static GAMEPAD_REMOVED_VTBL: IEventHandlerVtbl<IGamepadVtbl> = IEventHandlerVtbl {
    QueryInterface: gamepad_query_interface,
    AddRef: delegate_add_ref::<IGamepadVtbl>,
    Release: delegate_release::<IGamepadVtbl>,
    Invoke: gamepad_invoke,
};
static GAMEPAD_REMOVED: Delegate<IGamepadVtbl> = Delegate::new(&GAMEPAD_REMOVED_VTBL);

/// The Steam virtual gamepad slot in a raw input device path, or -1. The
/// format for the raw input device path is documented here:
/// <https://partner.steamgames.com/doc/features/steam_controller/steam_input_gamepad_emulation_bestpractices>.
/// Translation of `GetSteamVirtualGamepadSlot()`.
pub(super) fn get_steam_virtual_gamepad_slot(
    vendor_id: u16,
    product_id: u16,
    device_path: &str,
) -> i32 {
    if vendor_id == USB_VENDOR_VALVE && product_id == USB_PRODUCT_STEAM_VIRTUAL_GAMEPAD {
        use super::Scan::*;
        if let Some(slot) = super::scan_int(
            device_path,
            &[
                Lit("\\\\.\\pipe\\HID#VID_045E&PID_028E&IG_00#"),
                SkipHex,
                Lit("&"),
                SkipHex,
                Lit("&"),
                SkipHex,
                Lit("#"),
                Int,
                Lit("#"),
                SkipUnsigned,
            ],
        ) {
            return slot;
        }
    }
    -1
}

/// `GetRawInputDeviceInfoA()` into a buffer of `size` bytes: the bytes
/// written, if it worked.
fn raw_input_device_info(h_device: usize, command: u32, buffer: &mut [u8]) -> Option<usize> {
    let mut size = buffer.len() as u32;
    // SAFETY: the buffer holds `size` bytes.
    let result = unsafe {
        GetRawInputDeviceInfoA(
            h_device as HANDLE,
            command,
            buffer.as_mut_ptr().cast(),
            &mut size,
        )
    };
    (result != u32::MAX).then_some(result as usize)
}

/// Translation of `RAWINPUT_AddDevice()`.
fn add_device(h_device: usize) {
    // Make sure we're not trying to add the same device twice
    if with_state(|s| s.device_from_handle(h_device).is_some()) {
        return;
    }

    // Figure out what kind of device it is
    // SAFETY: RID_DEVICE_INFO is plain data; all zeros is valid.
    let mut rdi: RID_DEVICE_INFO = unsafe { std::mem::zeroed() };
    // FIXME (upstream): rdi.cbSize is left 0, which the documentation says
    // must be sizeof(RID_DEVICE_INFO) for RIDI_DEVICEINFO.
    let mut size = size_of::<RID_DEVICE_INFO>() as u32;
    // SAFETY: rdi holds `size` bytes.
    let result = unsafe {
        GetRawInputDeviceInfoA(
            h_device as HANDLE,
            RIDI_DEVICEINFO,
            (&mut rdi as *mut RID_DEVICE_INFO).cast(),
            &mut size,
        )
    };
    if result == u32::MAX || rdi.dwType != RIM_TYPEHID {
        return;
    }
    // SAFETY: for a HID device the union holds the hid member.
    let hid_info = unsafe { rdi.Anonymous.hid };

    // Get the device "name" (HID Path)
    let mut dev_name = [0u8; MAX_PATH];
    if raw_input_device_info(h_device, RIDI_DEVICENAME, &mut dev_name).is_none() {
        return;
    }
    let name_len = dev_name
        .iter()
        .position(|&c| c == 0)
        .unwrap_or(dev_name.len());
    let dev_name = String::from_utf8_lossy(&dev_name[..name_len]).into_owned();
    // Only take XInput-capable devices
    let vendor_id = hid_info.dwVendorId as u16;
    let product_id = hid_info.dwProductId as u16;
    let version = hid_info.dwVersionNumber as u16;
    if !dev_name.contains("IG_")
        || should_ignore_joystick(vendor_id, product_id, version, Some(""))
        || joystick_handled_by_another_driver(
            RAWINPUT_DRIVER_INDEX,
            vendor_id,
            product_id,
            version,
            Some(""),
        )
    {
        return;
    }

    // Get HID Top-Level Collection Preparsed Data
    let mut size = 0u32;
    // SAFETY: a NULL buffer asks for the size.
    let result = unsafe {
        GetRawInputDeviceInfoA(
            h_device as HANDLE,
            RIDI_PREPARSEDDATA,
            std::ptr::null_mut(),
            &mut size,
        )
    };
    if result == u32::MAX {
        return;
    }
    let mut preparsed_data = vec![0u8; size as usize];
    if raw_input_device_info(h_device, RIDI_PREPARSEDDATA, &mut preparsed_data).is_none() {
        return;
    }

    let Ok(c_name) = CString::new(dev_name.as_str()) else {
        return;
    };
    // SAFETY: the name is NUL-terminated; no security attributes or template.
    let h_file = unsafe {
        CreateFileA(
            c_name.as_ptr().cast(),
            GENERIC_READ,
            FILE_SHARE_READ | FILE_SHARE_WRITE,
            std::ptr::null(),
            OPEN_EXISTING,
            0,
            std::ptr::null_mut(),
        )
    };
    if h_file == INVALID_HANDLE_VALUE {
        return;
    }

    let (manufacturer_string, product_string) = match hid() {
        Some(h) => (
            h.get_manufacturer_string(h_file).map(|s| wide_to_utf8(&s)),
            h.get_product_string(h_file).map(|s| wide_to_utf8(&s)),
        ),
        None => (None, None),
    };

    let device = RawinputDevice {
        name: create_joystick_name(
            vendor_id,
            product_id,
            manufacturer_string.as_deref(),
            product_string.as_deref(),
        )
        .unwrap_or_default(),
        guid: create_joystick_guid(
            HARDWARE_BUS_USB,
            vendor_id,
            product_id,
            version,
            manufacturer_string.as_deref(),
            product_string.as_deref(),
            b'r',
            0,
        ),
        steam_virtual_gamepad_slot: get_steam_virtual_gamepad_slot(
            vendor_id, product_id, &dev_name,
        ),
        path: dev_name,
        vendor_id,
        product_id,
        version,
        is_xinput: true,
        is_xboxone: is_joystick_xbox_one(vendor_id, product_id),
        preparsed_data,
        h_device,
        joystick_id: crate::utils::next_object_id(),
    };

    // SAFETY: the handle came from CreateFileA and is closed once.
    unsafe {
        CloseHandle(h_file);
    }

    // Add it to the list
    let joystick_id = device.joystick_id;
    with_state(|s| s.devices.push(Arc::new(device)));

    private_joystick_added(joystick_id);
}

/// Translation of `RAWINPUT_DelDevice()` (whose `send_event` isn't used:
/// the removal is always reported).
fn del_device(device: &Arc<RawinputDevice>) {
    let removed = with_state(|s| {
        let i = s.devices.iter().position(|d| Arc::ptr_eq(d, device))?;
        let device = s.devices.remove(i);
        Some(device)
    });
    if let Some(device) = removed {
        private_joystick_removed(device.joystick_id);

        with_state(|s| s.release_device(device));
    }
}

/// Translation of `RAWINPUT_DetectDevices()`.
fn detect_devices() {
    let mut device_count = 0u32;

    // SAFETY: a NULL list asks for the count.
    let result = unsafe {
        GetRawInputDeviceList(
            std::ptr::null_mut(),
            &mut device_count,
            size_of::<RAWINPUTDEVICELIST>() as u32,
        )
    };
    if result != u32::MAX && device_count > 0 {
        // SAFETY: RAWINPUTDEVICELIST is plain data; all zeros is valid.
        let mut devices =
            vec![unsafe { std::mem::zeroed::<RAWINPUTDEVICELIST>() }; device_count as usize];
        // SAFETY: the list holds device_count entries.
        let count = unsafe {
            GetRawInputDeviceList(
                devices.as_mut_ptr(),
                &mut device_count,
                size_of::<RAWINPUTDEVICELIST>() as u32,
            )
        };
        if count != u32::MAX {
            for device in devices.iter().take(count as usize) {
                add_device(device.hDevice as usize);
            }
        }
    }
}

/// Translation of `RAWINPUT_RemoveDevices()`.
fn remove_devices() {
    while let Some(device) = with_state(|s| s.devices.first().cloned()) {
        del_device(&device);
    }
    crate::sdl_assert!(with_state(|s| s.devices.is_empty()));
}

/// Translation of `RAWINPUT_IsEnabled()`.
pub(super) fn is_enabled() -> bool {
    INITED.load(Ordering::Acquire) && !REMOTE_DESKTOP.load(Ordering::Acquire)
}

/// Translation of `RAWINPUT_PostUpdate()`: the guide button events to send.
fn post_update(s: &mut RawinputState) -> Vec<(JoystickID, Event)> {
    let mut events = Vec::new();
    let mut unmapped_guide_pressed = false;

    if !s.wgi_state.dirty
        && s.wgi_state
            .per_gamepad
            .iter()
            .any(|g| !g.used && (g.state.Buttons & GAMEPAD_BUTTONS_GUIDE) != 0)
    {
        unmapped_guide_pressed = true;
    }
    s.wgi_state.dirty = true;

    if !s.xinput_state_dirty
        && s.xinput_state.iter().any(|x| {
            x.connected && !x.used && (x.state.Gamepad.wButtons & XINPUT_GAMEPAD_GUIDE) != 0
        })
    {
        unmapped_guide_pressed = true;
    }
    s.xinput_state_dirty = true;

    if unmapped_guide_pressed {
        if let (Some(joystick), None) = (
            s.guide_button_candidate.joystick,
            s.guide_button_candidate.last_joystick,
        ) {
            if let Some(ctx) = s.context(joystick) {
                if ctx.guide_hack {
                    let guide_button = ctx.nbuttons as i32 - 1;
                    events.push((
                        joystick,
                        Event::Button {
                            timestamp: crate::timer::ticks_ns(),
                            button: guide_button as u8,
                            down: true,
                        },
                    ));
                }
            }
            s.guide_button_candidate.last_joystick = Some(joystick);
        }
    } else if let Some(joystick) = s.guide_button_candidate.last_joystick {
        if let Some(ctx) = s.context(joystick) {
            if ctx.guide_hack {
                let guide_button = ctx.nbuttons as i32 - 1;
                events.push((
                    joystick,
                    Event::Button {
                        timestamp: crate::timer::ticks_ns(),
                        button: guide_button as u8,
                        down: false,
                    },
                ));
            }
        }
        s.guide_button_candidate.last_joystick = None;
    }
    s.guide_button_candidate.joystick = None;
    events
}

/// The data entry of a data index. Translation of `GetData()`.
pub(super) fn get_data(index: u16, data: &[HIDP_DATA]) -> Option<&HIDP_DATA> {
    // Check to see if the data is at the expected offset
    if let Some(item) = data.get(index as usize) {
        if item.DataIndex == index {
            return Some(item);
        }
    }

    // Loop through the data to find it
    data.iter().find(|item| item.DataIndex == index)
}

/// The match state bit of each of the first 10 buttons (`button_map`).
const BUTTON_MAP: [GamepadButton; 10] = [
    GamepadButton::South,
    GamepadButton::East,
    GamepadButton::West,
    GamepadButton::North,
    GamepadButton::LeftShoulder,
    GamepadButton::RightShoulder,
    GamepadButton::Back,
    GamepadButton::Start,
    GamepadButton::LeftStick,
    GamepadButton::RightStick,
];

const fn bit(button: GamepadButton) -> u64 {
    1u64 << button as i32
}

const HAT_MASK: u64 = bit(GamepadButton::DpadUp)
    | bit(GamepadButton::DpadDown)
    | bit(GamepadButton::DpadLeft)
    | bit(GamepadButton::DpadRight);

/// The match state bits of each hat value (`hat_map`).
const HAT_MAP: [u64; 10] = [
    0,
    bit(GamepadButton::DpadUp),
    bit(GamepadButton::DpadUp) | bit(GamepadButton::DpadRight),
    bit(GamepadButton::DpadRight),
    bit(GamepadButton::DpadDown) | bit(GamepadButton::DpadRight),
    bit(GamepadButton::DpadDown),
    bit(GamepadButton::DpadDown) | bit(GamepadButton::DpadLeft),
    bit(GamepadButton::DpadLeft),
    bit(GamepadButton::DpadUp) | bit(GamepadButton::DpadLeft),
    0,
];

/// The hat position of each hat value (`hat_states`).
const HAT_STATES: [u8; 10] = [
    HAT_CENTERED,
    HAT_UP,
    HAT_UP | HAT_RIGHT,
    HAT_RIGHT,
    HAT_DOWN | HAT_RIGHT,
    HAT_DOWN,
    HAT_DOWN | HAT_LEFT,
    HAT_LEFT,
    HAT_UP | HAT_LEFT,
    HAT_CENTERED,
];

/// Grab high 4 bits of value. Translation of `AddAxisToMatchState()`.
fn add_axis_to_match_state(match_state: &mut u64, axis: i32, value: i16) {
    *match_state = (*match_state & !(0xFu64 << (4 * axis + 16)))
        | ((value as i64 as u64) & 0xF000) << (4 * axis + 4);
}

/// The parsed HID data of a state packet, the part of a context it uses
/// and the state packet's events. Translation of the body of
/// `RAWINPUT_HandleStatePacket()` after `SDL_HidP_GetData()`: the events
/// and the new match state.
pub(super) struct StatePacket<'a> {
    pub(super) nbuttons: usize,
    pub(super) naxes: usize,
    pub(super) nhats: usize,
    pub(super) guide_hack: bool,
    pub(super) trigger_hack: bool,
    pub(super) trigger_hack_index: u16,
    pub(super) button_indices: &'a [u16],
    pub(super) axis_indices: &'a [u16],
    pub(super) hat_indices: &'a [u16],
    /// Whether XInput or WGI gives the triggers (correlated).
    pub(super) has_trigger_data: bool,
    pub(super) match_state: u64,
}

impl StatePacket<'_> {
    /// The events of the data and the new match state.
    pub(super) fn events(&self, data: &[HIDP_DATA], timestamp: u64) -> (Vec<Event>, u64) {
        let mut events = Vec::new();
        let mut match_state = self.match_state;
        let nbuttons = self.nbuttons - self.guide_hack as usize;
        let naxes = self.naxes - self.trigger_hack as usize * 2;
        let nhats = self.nhats;

        // FIXME (upstream): the button mask is 32 bits, shifted by the
        // button index, which is undefined past 31 buttons; here each
        // button's state is kept.
        let pressed: Vec<bool> = (0..nbuttons)
            .map(|i| {
                self.button_indices
                    .get(i)
                    .and_then(|&index| get_data(index, data))
                    .is_some_and(|item| item.on())
            })
            .collect();
        for (i, &down) in pressed.iter().enumerate() {
            // Update match_state with button bit, then fall through
            if let Some(&button) = BUTTON_MAP.get(i) {
                let button_bit = bit(button);
                match_state = (match_state & !button_bit) | (button_bit * down as u64);
            }
            events.push(Event::Button {
                timestamp,
                button: i as u8,
                down,
            });
        }

        for i in 0..naxes {
            if let Some(item) = self
                .axis_indices
                .get(i)
                .and_then(|&index| get_data(index, data))
            {
                let axis = (item.RawValue as u16 as i32 - 0x8000) as i16;
                // Grab high 4 bits of value, then fall through
                if i < 4 {
                    add_axis_to_match_state(&mut match_state, i as i32, axis);
                }
                events.push(Event::Axis {
                    timestamp,
                    axis: i as u8,
                    value: axis,
                });
            }
        }

        for i in 0..nhats {
            if let Some(item) = self
                .hat_indices
                .get(i)
                .and_then(|&index| get_data(index, data))
            {
                let mut hat = HAT_CENTERED;
                let state = item.RawValue;

                if (state as usize) < HAT_STATES.len() {
                    match_state = (match_state & !HAT_MASK) | HAT_MAP[state as usize];
                    hat = HAT_STATES[state as usize];
                }
                events.push(Event::Hat {
                    timestamp,
                    hat: i as u8,
                    value: hat,
                });
            }
        }

        if self.trigger_hack {
            let left_trigger = self.naxes as i32 - 2;
            let right_trigger = self.naxes as i32 - 1;

            if let Some(item) = get_data(self.trigger_hack_index, data) {
                let value = (item.RawValue as u16 as i32 - 0x8000) as i16 as i32;
                let left_value = if value > 0 {
                    (value * 2 - 32767) as i16
                } else {
                    i16::MIN
                };
                let right_value = if value < 0 {
                    (-value * 2 - 32769) as i16
                } else {
                    i16::MIN
                };

                // (AddTriggerToMatchState())
                let match_axis = |axis: i32| axis + MATCH_COUNT as i32 - self.naxes as i32;
                add_axis_to_match_state(&mut match_state, match_axis(left_trigger), left_value);
                add_axis_to_match_state(&mut match_state, match_axis(right_trigger), right_value);
                if !self.has_trigger_data {
                    events.push(Event::Axis {
                        timestamp,
                        axis: left_trigger as u8,
                        value: left_value,
                    });
                    events.push(Event::Axis {
                        timestamp,
                        axis: right_trigger as u8,
                        value: right_value,
                    });
                }
            }
        }
        (events, match_state)
    }
}

/// This is the packet format for Xbox 360 and Xbox One controllers on
/// Windows, however with this interface there is no rumble support, no
/// guide button, and the left and right triggers are tied together as a
/// single axis.
///
/// We use XInput and Windows.Gaming.Input to make up for these
/// shortcomings. Translation of `RAWINPUT_HandleStatePacket()`: the events
/// to send.
fn handle_state_packet(ctx: &mut DeviceContext, data: &[u8]) -> Vec<Event> {
    let Some(h) = hid() else {
        return Vec::new();
    };
    let timestamp = crate::timer::ticks_ns();

    ctx.data
        .resize(ctx.max_data_length as usize, HIDP_DATA::default());
    let Ok(data_length) = h.get_data(
        HIDP_REPORT_TYPE::Input,
        &mut ctx.data,
        &ctx.device.preparsed_data,
        data,
    ) else {
        return Vec::new();
    };

    let packet = StatePacket {
        nbuttons: ctx.nbuttons,
        naxes: ctx.naxes,
        nhats: ctx.nhats,
        guide_hack: ctx.guide_hack,
        trigger_hack: ctx.trigger_hack,
        trigger_hack_index: ctx.trigger_hack_index,
        button_indices: &ctx.button_indices,
        axis_indices: &ctx.axis_indices,
        hat_indices: &ctx.hat_indices,
        // Prefer XInput over WindowsGamingInput, it continues to provide data in the background
        has_trigger_data: (ctx.xinput_enabled && ctx.xinput_correlated) || ctx.wgi_correlated,
        match_state: ctx.match_state,
    };
    let (events, match_state) = packet.events(&ctx.data[..data_length], timestamp);

    if ctx.is_xinput {
        ctx.match_state = match_state;
        ctx.last_state_packet = crate::timer::ticks_ms();
    }
    events
}

/// The rumble state of an open joystick (`joystick->low_frequency_rumble`
/// and the others), which disables uncorrelation.
#[derive(Clone, Copy, Default)]
struct RumbleState {
    low_frequency_rumble: u16,
    high_frequency_rumble: u16,
    left_trigger_rumble: u16,
    right_trigger_rumble: u16,
}

/// Translation of `RAWINPUT_UpdateOtherAPIs()`: the events to send.
fn update_other_apis(
    s: &mut RawinputState,
    joystick: JoystickID,
    rumble: RumbleState,
) -> Vec<Event> {
    let mut events = Vec::new();
    let Some(ctx) = s.context(joystick) else {
        return events;
    };
    let mut has_trigger_data = false;
    let mut correlated = false;
    let match_state_xinput = fill_match_state(ctx.match_state);
    let guide_button = (ctx.nbuttons as i32 - 1) as u8;
    let left_trigger = (ctx.naxes as i32 - 2) as u8;
    let right_trigger = (ctx.naxes as i32 - 1) as u8;
    let xinput_correlated = ctx.xinput_correlated;

    // Parallel logic to WINDOWS_XINPUT below
    s.update_windows_gaming_input();
    let Some(ctx) = s.context(joystick) else {
        return events;
    };
    if ctx.wgi_correlated
        && rumble.low_frequency_rumble == 0
        && rumble.high_frequency_rumble == 0
        && rumble.left_trigger_rumble == 0
        && rumble.right_trigger_rumble == 0
    {
        // We have been previously correlated, ensure we are still matching, see comments in XINPUT section
        let wgi_slot = ctx.wgi_slot;
        let matches = s.wgi_gamepad(wgi_slot).is_some_and(|slot| {
            RawinputState::wgi_slot_matches(&match_state_xinput, slot, xinput_correlated)
        });
        let Some(ctx) = s.context(joystick) else {
            return events;
        };
        if matches {
            ctx.wgi_uncorrelate_count = 0;
        } else {
            ctx.wgi_uncorrelate_count = ctx.wgi_uncorrelate_count.wrapping_add(1);
            /* Only un-correlate if this is consistent over multiple Update() calls - the timing of polling/event
            pumping can easily cause this to uncorrelate for a frame.  2 seemed reliable in my testing, but
            let's set it to 5 to be safe.  An incorrect un-correlation will simply result in lower precision
            triggers for a frame. */
            if ctx.wgi_uncorrelate_count >= 5 {
                ctx.wgi_correlated = false;
                ctx.wgi_correlation_count = 0;
                let guide_hack = ctx.guide_hack;
                s.mark_wgi_slot_free(wgi_slot);
                // Force release of Guide button, it can't possibly be down on this device now.
                /* It gets left down if we were actually correlated incorrectly and it was released on the WindowsGamingInput
                device but we didn't get a state packet. */
                if guide_hack {
                    events.push(Event::Button {
                        timestamp: 0,
                        button: guide_button,
                        down: false,
                    });
                }
            }
        }
    }
    let Some(ctx) = s.context(joystick) else {
        return events;
    };
    if !ctx.wgi_correlated {
        let mut new_correlation_count = 0u8;
        if s.missing_wgi_slot() {
            let (guessed, correlation_id, slot_idx) =
                s.guess_wgi_slot(&match_state_xinput, xinput_correlated);
            if guessed {
                let Some(ctx) = s.context(joystick) else {
                    return events;
                };
                // we match exactly one WindowsGamingInput device
                /* Probably can do without wgi_correlation_count, just check and clear wgi_slot to NULL, unless we need
                even more frames to be sure. */
                let mut mark_used = None;
                if ctx.wgi_correlation_count != 0 && ctx.wgi_slot == slot_idx {
                    // was correlated previously, and still the same device
                    if ctx.wgi_correlation_id as i32 + 1 == correlation_id as i32 {
                        // no one else was correlated in the meantime
                        new_correlation_count = ctx.wgi_correlation_count.wrapping_add(1);
                        if new_correlation_count == 2 {
                            // correlation stayed steady and uncontested across multiple frames, guaranteed match
                            ctx.wgi_correlated = true;
                            correlated = true;
                            mark_used = Some(ctx.wgi_slot);
                        }
                    } else {
                        // someone else also possibly correlated to this device, start over
                        new_correlation_count = 1;
                    }
                } else {
                    // new possible correlation
                    new_correlation_count = 1;
                    ctx.wgi_slot = slot_idx;
                }
                ctx.wgi_correlation_id = correlation_id;
                if let Some(wgi_slot) = mark_used {
                    s.mark_wgi_slot_used(wgi_slot, joystick);
                    // If the generalized Guide button was using us, it doesn't need to anymore
                    clear_guide_button_candidate(s, joystick);
                }
            } else {
                // Match multiple WindowsGamingInput devices, or none (possibly due to no buttons pressed)
            }
        }
        if let Some(ctx) = s.context(joystick) {
            ctx.wgi_correlation_count = new_correlation_count;
        }
    } else {
        correlated = true;
    }

    // Parallel logic to WINDOWS_GAMING_INPUT above
    let xinput_enabled = s.context(joystick).is_some_and(|c| c.xinput_enabled);
    if xinput_enabled {
        s.update_xinput();
        let Some(ctx) = s.context(joystick) else {
            return events;
        };
        if ctx.xinput_correlated
            && rumble.low_frequency_rumble == 0
            && rumble.high_frequency_rumble == 0
        {
            // We have been previously correlated, ensure we are still matching
            /* This is required to deal with two (mostly) un-preventable mis-correlation situations:
              A) Since the HID data stream does not provide an initial state (but polling XInput does), if we open
                 5 controllers (#1-4 XInput mapped, #5 is not), and controller 1 had the A button down (and we don't
                 know), and the user presses A on controller #5, we'll see exactly 1 controller with A down (#5) and
                 exactly 1 XInput device with A down (#1), and incorrectly correlate.  This code will then un-correlate
                 when A is released from either controller #1 or #5.
              B) Since the app may not open all controllers, we could have a similar situation where only controller #5
                 is opened, and the user holds A on controllers #1 and #5 simultaneously - again we see only 1 controller
                 with A down and 1 XInput device with A down, and incorrectly correlate.  This should be very unusual
                 (only when apps do not open all controllers, yet are listening to Guide button presses, yet
                 for some reason want to ignore guide button presses on the un-opened controllers, yet users are
                 pressing buttons on the unopened controllers), and will resolve itself when either button is released
                 and we un-correlate.  We could prevent this by processing the state packets for *all* controllers,
                 even un-opened ones, as that would allow more precise correlation.
            */
            let xinput_slot = ctx.xinput_slot;
            let matches = s.xinput_slot_matches(&match_state_xinput, xinput_slot);
            let Some(ctx) = s.context(joystick) else {
                return events;
            };
            if matches {
                ctx.xinput_uncorrelate_count = 0;
            } else {
                ctx.xinput_uncorrelate_count = ctx.xinput_uncorrelate_count.wrapping_add(1);
                /* Only un-correlate if this is consistent over multiple Update() calls - the timing of polling/event
                pumping can easily cause this to uncorrelate for a frame.  2 seemed reliable in my testing, but
                let's set it to 5 to be safe.  An incorrect un-correlation will simply result in lower precision
                triggers for a frame. */
                if ctx.xinput_uncorrelate_count >= 5 {
                    ctx.xinput_correlated = false;
                    ctx.xinput_correlation_count = 0;
                    let guide_hack = ctx.guide_hack;
                    s.mark_xinput_slot_free(xinput_slot);
                    // Force release of Guide button, it can't possibly be down on this device now.
                    /* It gets left down if we were actually correlated incorrectly and it was released on the XInput
                    device but we didn't get a state packet. */
                    if guide_hack {
                        events.push(Event::Button {
                            timestamp: 0,
                            button: guide_button,
                            down: false,
                        });
                    }
                }
            }
        }
        let Some(ctx) = s.context(joystick) else {
            return events;
        };
        if !ctx.xinput_correlated {
            let mut new_correlation_count = 0u8;
            if s.missing_xinput_slot() {
                let (guessed, correlation_id, slot_idx) = s.guess_xinput_slot(&match_state_xinput);
                if guessed {
                    let Some(ctx) = s.context(joystick) else {
                        return events;
                    };
                    // we match exactly one XInput device
                    /* Probably can do without xinput_correlation_count, just check and clear xinput_slot to ANY, unless
                    we need even more frames to be sure */
                    let mut mark_used = None;
                    if ctx.xinput_correlation_count != 0 && ctx.xinput_slot == slot_idx {
                        // was correlated previously, and still the same device
                        if ctx.xinput_correlation_id as i32 + 1 == correlation_id as i32 {
                            // no one else was correlated in the meantime
                            new_correlation_count = ctx.xinput_correlation_count.wrapping_add(1);
                            if new_correlation_count == 2 {
                                // correlation stayed steady and uncontested across multiple frames, guaranteed match
                                ctx.xinput_correlated = true;
                                correlated = true;
                                mark_used = Some(ctx.xinput_slot);
                            }
                        } else {
                            // someone else also possibly correlated to this device, start over
                            new_correlation_count = 1;
                        }
                    } else {
                        // new possible correlation
                        new_correlation_count = 1;
                        ctx.xinput_slot = slot_idx;
                    }
                    ctx.xinput_correlation_id = correlation_id;
                    if let Some(xinput_slot) = mark_used {
                        s.mark_xinput_slot_used(xinput_slot);
                        // If the generalized Guide button was using us, it doesn't need to anymore
                        clear_guide_button_candidate(s, joystick);
                    }
                } else {
                    // Match multiple XInput devices, or none (possibly due to no buttons pressed)
                }
            }
            if let Some(ctx) = s.context(joystick) {
                ctx.xinput_correlation_count = new_correlation_count;
            }
        } else {
            correlated = true;
        }
    }

    // Poll for trigger data once (not per-state-packet)
    let Some(ctx) = s.context(joystick) else {
        return events;
    };
    // Prefer XInput over WindowsGamingInput, it continues to provide data in the background
    if !has_trigger_data && ctx.xinput_enabled && ctx.xinput_correlated {
        let xinput_slot = ctx.xinput_slot as usize;
        let (guide_hack, trigger_hack) = (ctx.guide_hack, ctx.trigger_hack);
        s.update_xinput();
        if let Some(slot) = s
            .xinput_state
            .get(xinput_slot)
            .filter(|slot| slot.connected)
        {
            let timestamp = if guide_hack || trigger_hack {
                crate::timer::ticks_ns()
            } else {
                // timestamp won't be used
                0
            };

            if guide_hack {
                let down = (slot.state.Gamepad.wButtons & XINPUT_GAMEPAD_GUIDE) != 0;
                events.push(Event::Button {
                    timestamp,
                    button: guide_button,
                    down,
                });
            }
            if trigger_hack {
                let gamepad = &slot.state.Gamepad;
                events.push(Event::Axis {
                    timestamp,
                    axis: left_trigger,
                    value: (gamepad.bLeftTrigger as i32 * 257 - 32768) as i16,
                });
                events.push(Event::Axis {
                    timestamp,
                    axis: right_trigger,
                    value: (gamepad.bRightTrigger as i32 * 257 - 32768) as i16,
                });
            }
            has_trigger_data = true;

            let (state, percent) = super::xinput::battery_information(&slot.battery);
            events.push(Event::Power { state, percent });
        }
    }

    let Some(ctx) = s.context(joystick) else {
        return events;
    };
    if !has_trigger_data && ctx.wgi_correlated {
        let (guide_hack, trigger_hack) = (ctx.guide_hack, ctx.trigger_hack);
        s.update_windows_gaming_input(); // May detect disconnect / cause uncorrelation
        let Some(ctx) = s.context(joystick) else {
            return events;
        };
        let wgi_slot = ctx.wgi_slot;
        if ctx.wgi_correlated {
            // Still connected
            if let Some(slot) = s.wgi_gamepad(wgi_slot) {
                let state = &slot.state;
                let timestamp = if guide_hack || trigger_hack {
                    crate::timer::ticks_ns()
                } else {
                    // timestamp won't be used
                    0
                };

                if guide_hack {
                    let down = (state.Buttons & GAMEPAD_BUTTONS_GUIDE) != 0;
                    events.push(Event::Button {
                        timestamp,
                        button: guide_button,
                        down,
                    });
                }
                if trigger_hack {
                    let trigger = |value: f64| ((value * u16::MAX as f64) as i32 - 32768) as i16;
                    events.push(Event::Axis {
                        timestamp,
                        axis: left_trigger,
                        value: trigger(state.LeftTrigger),
                    });
                    events.push(Event::Axis {
                        timestamp,
                        axis: right_trigger,
                        value: trigger(state.RightTrigger),
                    });
                }
            }
        }
    }

    if !correlated {
        let last_state_packet = s.context(joystick).map_or(0, |c| c.last_state_packet);
        let candidate = &mut s.guide_button_candidate;
        if candidate.joystick.is_none()
            || (last_state_packet != 0
                && (candidate.last_state_packet == 0
                    || last_state_packet >= candidate.last_state_packet))
        {
            candidate.joystick = Some(joystick);
            candidate.last_state_packet = last_state_packet;
        }
    }
    events
}

/// If the generalized Guide button was using `joystick`, it doesn't need
/// to anymore.
fn clear_guide_button_candidate(s: &mut RawinputState, joystick: JoystickID) {
    if s.guide_button_candidate.joystick == Some(joystick) {
        s.guide_button_candidate.joystick = None;
    }
    if s.guide_button_candidate.last_joystick == Some(joystick) {
        s.guide_button_candidate.last_joystick = None;
    }
}

/// Translation of `RAWINPUT_RegisterNotifications()`.
pub(super) fn register_notifications(hwnd: HWND) -> Result<()> {
    if !INITED.load(Ordering::Acquire) {
        return Ok(());
    }

    let rid = [RAWINPUTDEVICE {
        usUsagePage: USB_USAGEPAGE_GENERIC_DESKTOP,
        usUsage: USB_USAGE_GENERIC_GAMEPAD,
        dwFlags: RIDEV_DEVNOTIFY | RIDEV_INPUTSINK, // Receive messages when in background, including device add/remove
        hwndTarget: hwnd,
    }];

    // SAFETY: rid holds the given number of entries.
    if unsafe {
        RegisterRawInputDevices(
            rid.as_ptr(),
            rid.len() as u32,
            size_of::<RAWINPUTDEVICE>() as u32,
        )
    } == 0
    {
        return Err(Error::new("Couldn't register for raw input events"));
    }
    Ok(())
}

/// Translation of `RAWINPUT_UnregisterNotifications()`.
pub(super) fn unregister_notifications() -> Result<()> {
    if !INITED.load(Ordering::Acquire) {
        return Ok(());
    }

    let rid = [RAWINPUTDEVICE {
        usUsagePage: USB_USAGEPAGE_GENERIC_DESKTOP,
        usUsage: USB_USAGE_GENERIC_GAMEPAD,
        dwFlags: RIDEV_REMOVE,
        hwndTarget: std::ptr::null_mut(),
    }];

    // SAFETY: rid holds the given number of entries.
    if unsafe {
        RegisterRawInputDevices(
            rid.as_ptr(),
            rid.len() as u32,
            size_of::<RAWINPUTDEVICE>() as u32,
        )
    } == 0
    {
        return Err(Error::new("Couldn't unregister for raw input events"));
    }
    Ok(())
}

/// The buffer `WM_INPUT` data is read into (`sizeof(RAWINPUTHEADER) +
/// sizeof(RAWHID) + USB_PACKET_LENGTH` bytes), aligned for the header.
#[repr(C, align(8))]
struct RawInputBuffer([u8; RAW_INPUT_BUFFER_SIZE]);

/// The size of the [`RawInputBuffer`] data.
const RAW_INPUT_BUFFER_SIZE: usize = size_of::<RAWINPUTHEADER>()
    + size_of::<windows_sys::Win32::UI::Input::RAWHID>()
    + USB_PACKET_LENGTH;

/// The window procedure of the Windows driver's message window, for the
/// raw input messages. Translation of `RAWINPUT_WindowProc()`.
pub(super) unsafe extern "system" fn window_proc(
    hwnd: HWND,
    msg: u32,
    w_param: WPARAM,
    l_param: LPARAM,
) -> LRESULT {
    let mut result: LRESULT = -1;

    if INITED.load(Ordering::Acquire) {
        let _lock = lock_joysticks();

        match msg {
            WM_INPUT_DEVICE_CHANGE => {
                let h_device = l_param as usize;
                match w_param {
                    GIDC_ARRIVAL => add_device(h_device),
                    GIDC_REMOVAL => {
                        if let Some(device) = with_state(|s| s.device_from_handle(h_device)) {
                            del_device(&device);
                        }
                    }
                    _ => {}
                }
                result = 0;
            }

            WM_INPUT => {
                let mut data = RawInputBuffer([0; RAW_INPUT_BUFFER_SIZE]);
                let mut buffer_size = data.0.len() as u32;

                // SAFETY: lParam is the raw input handle of this message;
                // the buffer holds buffer_size bytes.
                let read = unsafe {
                    GetRawInputData(
                        l_param as HRAWINPUT,
                        RID_INPUT,
                        data.0.as_mut_ptr().cast(),
                        &mut buffer_size,
                        size_of::<RAWINPUTHEADER>() as u32,
                    )
                };
                if read as i32 > 0 {
                    // SAFETY: the buffer is aligned for and holds a header.
                    let header =
                        unsafe { std::ptr::read(data.0.as_ptr().cast::<RAWINPUTHEADER>()) };
                    let hid_offset = std::mem::offset_of!(RAWINPUT, data);
                    let field = |at: usize| {
                        u32::from_ne_bytes(data.0[at..at + 4].try_into().unwrap_or_default())
                    };
                    let dw_size_hid = field(hid_offset) as usize;
                    let raw_start = hid_offset + 8; // (bRawData, after dwSizeHid and dwCount)
                    let raw_end = raw_start.saturating_add(dw_size_hid).min(data.0.len());
                    let raw = &data.0[raw_start.min(raw_end)..raw_end];

                    let events = with_state(|s| {
                        let device = s.device_from_handle(header.hDevice as usize)?;
                        let ctx = s.context_of(&device)?;
                        let joystick = ctx.joystick;
                        Some((joystick, handle_state_packet(ctx, raw)))
                    });
                    if let Some((joystick, events)) = events {
                        send_events(joystick, events);
                    }
                }
                result = 0;
            }
            _ => {}
        }
    }

    if result >= 0 {
        return result;
    }
    // SAFETY: passing the message on to the default window procedure.
    unsafe { CallWindowProcW(Some(DefWindowProcW), hwnd, msg, w_param, l_param) }
}

/// Run `f` on the device at `device_index`.
fn with_device<R>(device_index: usize, f: impl FnOnce(&RawinputDevice) -> R) -> Option<R> {
    with_state(|s| s.devices.get(device_index).map(|d| f(d)))
}

/// Translation of `SDL_RAWINPUT_JoystickDriver`.
pub(in crate::joystick) struct RawinputJoystickDriver;

pub(in crate::joystick) static RAWINPUT_JOYSTICK_DRIVER: RawinputJoystickDriver =
    RawinputJoystickDriver;

impl RawinputJoystickDriver {
    /// Close the joystick of a context (all of `RAWINPUT_JoystickClose()`
    /// but the guide button candidate).
    fn close_context(ctx: DeviceContext) {
        with_state(|s| {
            s.xinput_device_change = true;
            if ctx.xinput_enabled {
                if ctx.xinput_correlated {
                    s.mark_xinput_slot_free(ctx.xinput_slot);
                }
                unload_xinput_dll();
            }
            // FIXME (upstream): a correlated WGI gamepad isn't freed here: it
            // stays marked used, its correlated_context pointing to the
            // freed context (which RAWINPUT_UpdateWindowsGamingInput()
            // writes to when the gamepad goes away); here the context is
            // looked up by its joystick, so a stale one is just not found.
            s.quit_windows_gaming_input();
        });

        // (device->joystick = NULL, and the context's reference to the
        // device is released, when ctx drops)
        drop(ctx);
    }
}

impl JoystickDriver for RawinputJoystickDriver {
    /// Translation of `RAWINPUT_JoystickInit()`.
    fn init(&self) -> Result<()> {
        crate::sdl_assert!(!INITED.load(Ordering::Acquire));

        // (SDL_UsingGameInputForXInputControllers() is false without GameInput)
        if !hints::get_bool(hints::JOYSTICK_RAWINPUT, false) {
            return Ok(());
        }

        if !is_windows_vista_or_greater() {
            // According to bug 6400, this doesn't work on Windows XP
            return Err(Error::new("RAWINPUT requires Windows Vista or later"));
        }

        if !load_hid_dll() {
            return Err(Error::new("Couldn't load hid.dll"));
        }

        INITED.store(true, Ordering::Release);

        detect_devices();

        Ok(())
    }

    /// Translation of `RAWINPUT_JoystickGetCount()`.
    fn count(&self) -> usize {
        with_state(|s| s.devices.len())
    }

    /// Translation of `RAWINPUT_JoystickDetect()`.
    fn detect(&self) {
        if !INITED.load(Ordering::Acquire) {
            return;
        }

        // SAFETY: GetSystemMetrics has no preconditions.
        let remote_desktop = unsafe { GetSystemMetrics(SM_REMOTESESSION) } != 0;
        if remote_desktop != REMOTE_DESKTOP.load(Ordering::Acquire) {
            REMOTE_DESKTOP.store(remote_desktop, Ordering::Release);

            super::rawinput_enabled_changed();

            if remote_desktop {
                remove_devices();
                super::joystick_detect();
            } else {
                super::joystick_detect();
                detect_devices();
            }
        }
        let events = with_state(post_update);
        for (joystick, event) in events {
            send_events(joystick, vec![event]);
        }
    }

    /// Translation of `RAWINPUT_JoystickIsDevicePresent()`.
    fn is_device_present(
        &self,
        vendor_id: u16,
        product_id: u16,
        _version: u16,
        name: Option<&str>,
    ) -> bool {
        with_state(|s| {
            // If we're being asked about a device, that means another API just detected one, so rescan
            s.xinput_device_change = true;

            s.devices.iter().any(|device| {
                if vendor_id == device.vendor_id && product_id == device.product_id {
                    return true;
                }

                /* The Xbox 360 wireless controller shows up as product 0 in WGI.
                Try to match it to a Raw Input device via name or known product ID. */
                if vendor_id == device.vendor_id
                    && product_id == 0
                    && (name.is_some_and(|name| device.name.contains(name))
                        || (device.vendor_id == USB_VENDOR_MICROSOFT
                            && device.product_id == USB_PRODUCT_XBOX360_XUSB_CONTROLLER))
                {
                    return true;
                }

                // The Xbox One controller shows up as a hardcoded raw input VID/PID
                name == Some("Xbox One Game Controller")
                    && device.vendor_id == USB_VENDOR_MICROSOFT
                    && device.product_id == USB_PRODUCT_XBOX_ONE_XBOXGIP_CONTROLLER
            })
        })
    }

    /// Translation of `RAWINPUT_JoystickGetDeviceName()`.
    fn device_name(&self, device_index: usize) -> Option<String> {
        with_device(device_index, |d| d.name.clone())
    }

    /// Translation of `RAWINPUT_JoystickGetDevicePath()`.
    fn device_path(&self, device_index: usize) -> Option<String> {
        with_device(device_index, |d| d.path.clone())
    }

    /// Translation of `RAWINPUT_JoystickGetDeviceSteamVirtualGamepadSlot()`.
    fn device_steam_virtual_gamepad_slot(&self, device_index: usize) -> i32 {
        with_device(device_index, |d| d.steam_virtual_gamepad_slot).unwrap_or(-1)
    }

    /// Translation of `RAWINPUT_JoystickGetDevicePlayerIndex()`.
    fn device_player_index(&self, _device_index: usize) -> i32 {
        // FIXME (upstream): this returns false, i.e. player index 0, rather
        // than -1 (no player index).
        0
    }

    /// Translation of `RAWINPUT_JoystickSetDevicePlayerIndex()`.
    fn set_device_player_index(&self, _device_index: usize, _player_index: i32) {}

    /// Translation of `RAWINPUT_JoystickGetDeviceGUID()`.
    fn device_guid(&self, device_index: usize) -> Guid {
        with_device(device_index, |d| d.guid).unwrap_or(Guid::ZERO)
    }

    /// Translation of `RAWINPUT_JoystickGetDeviceInstanceID()`.
    fn device_instance_id(&self, device_index: usize) -> JoystickID {
        with_device(device_index, |d| d.joystick_id).unwrap_or(0)
    }

    /// Translation of `RAWINPUT_JoystickOpen()`.
    fn open(&self, joystick: &mut JoystickData, device_index: usize) -> Result<()> {
        let Some(device) = with_state(|s| s.devices.get(device_index).cloned()) else {
            return Err(Error::new("No such device"));
        };

        let mut ctx = DeviceContext {
            joystick: joystick.instance_id,
            nbuttons: 0,
            naxes: 0,
            nhats: 0,
            is_xinput: device.is_xinput,
            is_xboxone: device.is_xboxone,
            max_data_length: 0,
            data: Vec::new(),
            button_indices: Vec::new(),
            axis_indices: Vec::new(),
            hat_indices: Vec::new(),
            guide_hack: false,
            trigger_hack: false,
            trigger_hack_index: 0,
            match_state: 0x0000008800000000, // Trigger axes at rest
            last_state_packet: 0,
            xinput_enabled: false,
            xinput_correlated: false,
            xinput_correlation_id: 0,
            xinput_correlation_count: 0,
            xinput_uncorrelate_count: 0,
            xinput_slot: 0,
            wgi_correlated: false,
            wgi_correlation_id: 0,
            wgi_correlation_count: 0,
            wgi_uncorrelate_count: 0,
            wgi_slot: None,
            vibration: GamepadVibration::default(),
            triggers_rumbling: false,
            device: device.clone(),
        };

        if device.is_xinput {
            // We'll try to get guide button and trigger axes from XInput
            with_state(|s| s.xinput_device_change = true);
            ctx.xinput_enabled = hints::get_bool(hints::JOYSTICK_RAWINPUT_CORRELATE_XINPUT, true);
            if ctx.xinput_enabled && !load_xinput_dll() {
                ctx.xinput_enabled = false;
            }
            ctx.xinput_slot = XUSER_INDEX_ANY as u8;
            with_state(|s| s.init_windows_gaming_input());
        }

        let result = (|| -> Result<()> {
            let h = hid().ok_or_else(|| Error::new("Couldn't get device capabilities"))?;
            let preparsed_data = &device.preparsed_data;
            ctx.max_data_length = h.max_data_list_length(HIDP_REPORT_TYPE::Input, preparsed_data);
            ctx.data = vec![HIDP_DATA::default(); ctx.max_data_length as usize];

            let caps = h
                .get_caps(preparsed_data)
                .map_err(|_| Error::new("Couldn't get device capabilities"))?;

            let button_caps = h
                .get_button_caps(
                    HIDP_REPORT_TYPE::Input,
                    caps.NumberInputButtonCaps,
                    preparsed_data,
                )
                .map_err(|_| Error::new("Couldn't get device button capabilities"))?;

            let mut value_caps: Vec<HIDP_VALUE_CAPS> = h
                .get_value_caps(
                    HIDP_REPORT_TYPE::Input,
                    caps.NumberInputValueCaps,
                    preparsed_data,
                )
                .map_err(|_| Error::new("Couldn't get device value capabilities"))?;

            // Sort the axes by usage, so X comes before Y, etc.
            // (Sort by Usage for single values, or UsageMax for range of
            // values; SDL_qsort() isn't stable, but caps with the same usage
            // don't happen on these devices.)
            value_caps.sort_by_key(|cap| cap.u.usage());

            // FIXME (upstream): a range with DataIndexMax < DataIndexMin
            // counts negatively, and upstream then writes more indices than
            // it allocated; here the indices are collected in a Vec.
            let mut nbuttons: i32 = 0;
            for cap in &button_caps {
                if cap.UsagePage == USB_USAGEPAGE_BUTTON {
                    let count = if cap.IsRange != 0 {
                        1 + (cap.u.DataIndexMax as i32 - cap.u.DataIndexMin as i32)
                    } else {
                        1
                    };
                    nbuttons += count;
                }
            }

            if nbuttons > 0 {
                for cap in &button_caps {
                    if cap.UsagePage == USB_USAGEPAGE_BUTTON {
                        if cap.IsRange != 0 {
                            let count = 1 + (cap.u.DataIndexMax as i32 - cap.u.DataIndexMin as i32);
                            for j in 0..count.max(0) {
                                ctx.button_indices
                                    .push((cap.u.DataIndexMin as i32 + j) as u16);
                            }
                        } else {
                            ctx.button_indices.push(cap.u.data_index());
                        }
                    }
                }
            }
            let mut nbuttons = nbuttons.max(0) as usize;
            if ctx.is_xinput && nbuttons == 10 {
                ctx.guide_hack = true;
                nbuttons += 1;
            }

            let mut naxes = 0usize;
            let mut nhats = 0usize;
            for cap in &value_caps {
                if cap.IsRange != 0 {
                    continue;
                }

                if ctx.trigger_hack && cap.u.usage() == USB_USAGE_GENERIC_Z {
                    continue;
                }

                if cap.u.usage() == USB_USAGE_GENERIC_HAT {
                    nhats += 1;
                    continue;
                }

                if ctx.is_xinput && cap.u.usage() == USB_USAGE_GENERIC_Z {
                    continue;
                }

                naxes += 1;
            }

            if naxes > 0 {
                for cap in &value_caps {
                    if cap.IsRange != 0 {
                        continue;
                    }

                    if cap.u.usage() == USB_USAGE_GENERIC_HAT {
                        continue;
                    }

                    if ctx.is_xinput && cap.u.usage() == USB_USAGE_GENERIC_Z {
                        ctx.trigger_hack = true;
                        ctx.trigger_hack_index = cap.u.data_index();
                        continue;
                    }

                    ctx.axis_indices.push(cap.u.data_index());
                }
            }
            if ctx.trigger_hack {
                naxes += 2;
            }

            if nhats > 0 {
                for cap in &value_caps {
                    if cap.IsRange != 0 {
                        continue;
                    }

                    if cap.u.usage() != USB_USAGE_GENERIC_HAT {
                        continue;
                    }

                    ctx.hat_indices.push(cap.u.data_index());
                }
            }

            ctx.nbuttons = nbuttons;
            ctx.naxes = naxes;
            ctx.nhats = nhats;
            Ok(())
        })();

        if let Err(e) = result {
            Self::close_context(ctx);
            return Err(e);
        }

        joystick.nbuttons = ctx.nbuttons;
        joystick.naxes = ctx.naxes;
        joystick.nhats = ctx.nhats;

        if ctx.is_xinput {
            let props = joystick.properties();
            let _ = props.set(PROP_JOYSTICK_CAP_RUMBLE_BOOLEAN, true);

            if ctx.is_xboxone {
                let _ = props.set(PROP_JOYSTICK_CAP_TRIGGER_RUMBLE_BOOLEAN, true);
            }
        }

        with_state(|s| s.contexts.push(ctx));
        Ok(())
    }

    /// Translation of `RAWINPUT_JoystickRumble()`.
    fn rumble(
        &self,
        joystick: JoystickID,
        low_frequency_rumble: u16,
        high_frequency_rumble: u16,
    ) -> Result<()> {
        with_state(|s| {
            let ctx = s
                .context(joystick)
                .ok_or_else(|| Error::invalid_param("joystick"))?;
            let mut rumbled = false;

            // Prefer XInput over WGI because it allows rumble in the background
            if !rumbled && ctx.xinput_correlated && !ctx.triggers_rumbling {
                let Some(x) = xinput() else {
                    return Err(Error::unsupported());
                };

                let mut vibration = XINPUT_VIBRATION {
                    wLeftMotorSpeed: low_frequency_rumble,
                    wRightMotorSpeed: high_frequency_rumble,
                };
                if x.set_state(ctx.xinput_slot as u32, &mut vibration) == 0 {
                    rumbled = true;
                } else {
                    return Err(Error::new("XInputSetState() failed"));
                }
            }

            // Save off the motor state in case trigger rumble is started
            ctx.vibration.LeftMotor = low_frequency_rumble as f64 / u16::MAX as f64;
            ctx.vibration.RightMotor = high_frequency_rumble as f64 / u16::MAX as f64;
            let (wgi_correlated, wgi_slot, vibration) =
                (ctx.wgi_correlated, ctx.wgi_slot, ctx.vibration);
            if !rumbled && wgi_correlated {
                if let Some(gamepad_state) = s.wgi_gamepad(wgi_slot) {
                    let gamepad = &gamepad_state.gamepad;
                    // SAFETY: the gamepad is alive; the vibration is passed by value.
                    let hr = unsafe { (gamepad.vtbl().put_Vibration)(gamepad.as_ptr(), vibration) };
                    if hr >= 0 {
                        rumbled = true;
                    }
                }
            }

            if !rumbled {
                return Err(Error::new(
                    "Controller isn't correlated yet, try hitting a button first",
                ));
            }
            Ok(())
        })
    }

    /// Translation of `RAWINPUT_JoystickRumbleTriggers()`.
    fn rumble_triggers(
        &self,
        joystick: JoystickID,
        left_rumble: u16,
        right_rumble: u16,
    ) -> Result<()> {
        with_state(|s| {
            let ctx = s
                .context(joystick)
                .ok_or_else(|| Error::invalid_param("joystick"))?;

            ctx.vibration.LeftTrigger = left_rumble as f64 / u16::MAX as f64;
            ctx.vibration.RightTrigger = right_rumble as f64 / u16::MAX as f64;
            if ctx.wgi_correlated {
                let (wgi_slot, vibration) = (ctx.wgi_slot, ctx.vibration);
                let hr = match s.wgi_gamepad(wgi_slot) {
                    Some(gamepad_state) => {
                        let gamepad = &gamepad_state.gamepad;
                        // SAFETY: the gamepad is alive; the vibration is passed by value.
                        unsafe { (gamepad.vtbl().put_Vibration)(gamepad.as_ptr(), vibration) }
                    }
                    None => windows_sys::Win32::Foundation::E_POINTER,
                };
                if hr < 0 {
                    return Err(Error::new(format!(
                        "Setting vibration failed: 0x{:x}",
                        hr as u32
                    )));
                }
                if let Some(ctx) = s.context(joystick) {
                    ctx.triggers_rumbling = left_rumble > 0 || right_rumble > 0;
                }
                Ok(())
            } else {
                Err(Error::new(
                    "Controller isn't correlated yet, try hitting a button first",
                ))
            }
        })
    }

    /// Translation of `RAWINPUT_JoystickSetLED()`.
    fn set_led(&self, _joystick: JoystickID, _red: u8, _green: u8, _blue: u8) -> Result<()> {
        Err(Error::unsupported())
    }

    /// Translation of `RAWINPUT_JoystickSendEffect()`.
    fn send_effect(&self, _joystick: JoystickID, _data: &[u8]) -> Result<()> {
        Err(Error::unsupported())
    }

    /// Translation of `RAWINPUT_JoystickSetSensorsEnabled()`.
    fn set_sensors_enabled(&self, _joystick: JoystickID, _enabled: bool) -> Result<()> {
        Err(Error::unsupported())
    }

    /// Translation of `RAWINPUT_JoystickUpdate()`.
    fn update(&self, joystick: JoystickID) {
        let rumble = with_joystick(joystick, |j| RumbleState {
            low_frequency_rumble: j.low_frequency_rumble,
            high_frequency_rumble: j.high_frequency_rumble,
            left_trigger_rumble: j.left_trigger_rumble,
            right_trigger_rumble: j.right_trigger_rumble,
        })
        .unwrap_or_default();
        let events = with_state(|s| update_other_apis(s, joystick, rumble));
        send_events(joystick, events);
    }

    /// Translation of `RAWINPUT_JoystickClose()`.
    fn close(&self, joystick: &mut JoystickData) {
        let ctx = with_state(|s| {
            clear_guide_button_candidate(s, joystick.instance_id);
            let i = s
                .contexts
                .iter()
                .position(|c| c.joystick == joystick.instance_id)?;
            Some(s.contexts.remove(i))
        });

        if let Some(ctx) = ctx {
            Self::close_context(ctx);
        }
    }

    /// Translation of `RAWINPUT_JoystickQuit()`.
    fn quit(&self) {
        if !INITED.load(Ordering::Acquire) {
            return;
        }

        remove_devices();

        unload_hid_dll();

        INITED.store(false, Ordering::Release);
    }

    /// Translation of `RAWINPUT_JoystickGetGamepadMapping()`.
    fn gamepad_mapping(&self, _device_index: usize) -> Option<GamepadMapping> {
        None
    }
}
