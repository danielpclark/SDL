// Rust translation of the mapping generators of src/joystick/SDL_gamepad.c
// from Simple DirectMedia Layer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! Mappings generated for the gamepads of drivers that know their layout
//! (HIDAPI, RAWINPUT and WGI), when the database has none.

use super::super::usb_ids::*;
use super::super::{
    gamepad_type_from_guid, is_joystick_amazon_luna_controller, is_joystick_dual_sense_edge,
    is_joystick_flydigi_controller, is_joystick_gamecube, is_joystick_gamesir_controller,
    is_joystick_google_stadia_controller, is_joystick_hori_steam_controller,
    is_joystick_nintendo_switch2_pro, is_joystick_nintendo_switch2_pro_input_only,
    is_joystick_nintendo_switch_joycon_pair, is_joystick_nintendo_switch_pro,
    is_joystick_nintendo_switch_pro_input_only, is_joystick_nvidia_shield_controller,
    is_joystick_sinput_controller, is_joystick_steam_controller, is_joystick_steam_deck,
    is_joystick_steam_triton, is_joystick_wheel, is_joystick_xbox_one_elite,
    is_joystick_xbox_series_x, joystick_guid_info, JoystickType, HARDWARE_BUS_USB,
};
use super::{private_add_mapping_for_guid, strlcat, GamepadType, MappingPriority};
use crate::guid::Guid;
use crate::hints;

/// The size of the C mapping buffers.
const MAPPING_SIZE: usize = 1024;

// From SDL_hidapi_nintendo.h
const SWITCH_DEVICE_INFO_CONTROLLER_TYPE_JOYCON_LEFT: u8 = 1;
const SWITCH_DEVICE_INFO_CONTROLLER_TYPE_JOYCON_RIGHT: u8 = 2;
const SWITCH_DEVICE_INFO_CONTROLLER_TYPE_HVC_LEFT: u8 = 7;
const SWITCH_DEVICE_INFO_CONTROLLER_TYPE_HVC_RIGHT: u8 = 8;
const SWITCH_DEVICE_INFO_CONTROLLER_TYPE_NES_LEFT: u8 = 9;
const SWITCH_DEVICE_INFO_CONTROLLER_TYPE_NES_RIGHT: u8 = 10;
const SWITCH_DEVICE_INFO_CONTROLLER_TYPE_SNES: u8 = 11;
const SWITCH_DEVICE_INFO_CONTROLLER_TYPE_N64: u8 = 12;
const SWITCH_DEVICE_INFO_CONTROLLER_TYPE_SEGA_GENESIS: u8 = 13;
const WII_EXTENSION_CONTROLLER_TYPE_NONE: u8 = 128;
const WII_EXTENSION_CONTROLLER_TYPE_NUNCHUK: u8 = 129;

// From SDL_hidapi_flydigi.h
const FLYDIGI_APEX5: u8 = 4;
const FLYDIGI_APEX6: u8 = 5;
const FLYDIGI_VADER2: u8 = 1 << 4;
const FLYDIGI_VADER5_PRO: u8 = FLYDIGI_VADER2 + 5;

// From SDL_hidapi_sinput.h: the number of values of each style
const SINPUT_ANALOGSTYLE_MAX: u16 = 4;
const SINPUT_BUMPERSTYLE_MAX: u16 = 3;
const SINPUT_TRIGGERSTYLE_MAX: u16 = 4;
const SINPUT_PADDLESTYLE_MAX: u16 = 3;
const SINPUT_METASTYLE_MAX: u16 = 4;
const SINPUT_TOUCHSTYLE_MAX: u16 = 3;
const SINPUT_MISCSTYLE_MAX: u16 = 5;

/// Translation of `SDL_SInputStyles_t` (each style as its raw value).
#[derive(Default)]
struct SInputStyles {
    analog_style: u16,
    bumper_style: u16,
    trigger_style: u16,
    paddle_style: u16,
    meta_style: u16,
    touch_style: u16,
    misc_style: u16,
}

/// Translation of `SDL_ADD_BUTTON_MAPPING()`.
fn add_button_mapping(mapping_string: &mut String, sdl_name: &str, button_id: i32, maxlen: usize) {
    strlcat(mapping_string, &format!("{sdl_name}:b{button_id},"), maxlen);
}

/// Translation of `SDL_ADD_AXIS_MAPPING()`.
fn add_axis_mapping(mapping_string: &mut String, sdl_name: &str, axis_id: i32, maxlen: usize) {
    strlcat(mapping_string, &format!("{sdl_name}:a{axis_id},"), maxlen);
}

/// Apply SInput decoded styles to the mapping string.
/// Translation of `SDL_SInputStylesMapExtraction()`.
fn sinput_styles_map_extraction(
    styles: &SInputStyles,
    mapping_string: &mut String,
    mapping_string_len: usize,
) {
    let mut current_button = 0;
    let mut current_axis = 0;
    let mut digital_triggers = false;
    let mut dualstage_triggers = false;
    let mut left_stick = false;
    let mut right_stick = false;

    // Determine how many misc buttons are used
    let misc_buttons = match styles.misc_style {
        1..=4 => i32::from(styles.misc_style),
        _ => 0,
    };
    // The share button is reserved as misc1, additional buttons start at misc2
    let misc_button = 2;
    let mut misc_end = misc_button + misc_buttons;

    let mut button = |s: &mut String, name: &str| {
        add_button_mapping(s, name, current_button, mapping_string_len);
        current_button += 1;
    };
    let mut axis = |s: &mut String, name: &str| {
        add_axis_mapping(s, name, current_axis, mapping_string_len);
        current_axis += 1;
    };

    // Analog joysticks (always come first in axis mapping)
    match styles.analog_style {
        1 => {
            // SINPUT_ANALOGSTYLE_LEFTONLY
            axis(mapping_string, "leftx");
            axis(mapping_string, "lefty");
            left_stick = true;
        }
        3 => {
            // SINPUT_ANALOGSTYLE_LEFTRIGHT
            axis(mapping_string, "leftx");
            axis(mapping_string, "lefty");
            axis(mapping_string, "rightx");
            axis(mapping_string, "righty");
            left_stick = true;
            right_stick = true;
        }
        2 => {
            // SINPUT_ANALOGSTYLE_RIGHTONLY
            axis(mapping_string, "rightx");
            axis(mapping_string, "righty");
            right_stick = true;
        }
        _ => {}
    }

    // Bumpers
    let bumpers = match styles.bumper_style {
        1 => 1, // SINPUT_BUMPERSTYLE_ONE
        2 => 2, // SINPUT_BUMPERSTYLE_TWO
        _ => 0,
    };

    // Analog triggers
    match styles.trigger_style {
        // Analog triggers (SINPUT_TRIGGERSTYLE_ANALOG)
        1 => {
            axis(mapping_string, "lefttrigger");
            axis(mapping_string, "righttrigger");
        }
        // Digital triggers (SINPUT_TRIGGERSTYLE_DIGITAL)
        2 => digital_triggers = true,
        // Analog triggers with digital press (SINPUT_TRIGGERSTYLE_DUALSTAGE)
        3 => {
            axis(mapping_string, "lefttrigger");
            axis(mapping_string, "righttrigger");
            dualstage_triggers = true;
        }
        _ => {}
    }

    let paddle_pairs = match styles.paddle_style {
        1 => 1, // SINPUT_PADDLESTYLE_TWO
        2 => 2, // SINPUT_PADDLESTYLE_FOUR
        _ => 0,
    };

    // Digital button mappings
    // ABXY buttons (always applied as South, East, West, North)
    button(mapping_string, "a"); // South (typically A on Xbox, X on PlayStation)
    button(mapping_string, "b"); // East  (typically B on Xbox, Circle on PlayStation)
    button(mapping_string, "x"); // West  (typically X on Xbox, Square on PlayStation)
    button(mapping_string, "y"); // North (typically Y on Xbox, Triangle on PlayStation)

    // D-Pad (always applied)
    strlcat(
        mapping_string,
        "dpup:h0.1,dpdown:h0.4,dpleft:h0.8,dpright:h0.2,",
        mapping_string_len,
    );

    // Left and Right stick buttons
    if left_stick {
        button(mapping_string, "leftstick");
    }
    if right_stick {
        button(mapping_string, "rightstick");
    }

    // Digital shoulder buttons (L/R Shoulder)
    if bumpers > 0 {
        button(mapping_string, "leftshoulder");
    }
    if bumpers > 1 {
        button(mapping_string, "rightshoulder");
    }

    // Digital trigger buttons (capability overrides analog)
    if digital_triggers {
        button(mapping_string, "lefttrigger");
        button(mapping_string, "righttrigger");
    } else if dualstage_triggers {
        // Dual-stage trigger buttons are appended as MISC buttons
        // By convention the trigger buttons are misc3 and misc4 for GameCube style controllers
        if misc_end < 3 {
            misc_end = 3;
        }
        button(mapping_string, &format!("misc{misc_end}"));
        misc_end += 1;
        button(mapping_string, &format!("misc{misc_end}"));
        misc_end += 1;
    }

    // Paddle 1/2
    if paddle_pairs > 0 {
        // Paddle 2 is first for left/right order of SInput
        button(mapping_string, "paddle2");
        button(mapping_string, "paddle1");
    }

    // Start/Plus
    button(mapping_string, "start");

    // Back/Minus, Guide/Home, Share/Capture
    match styles.meta_style {
        1 => {
            // SINPUT_METASTYLE_BACK
            button(mapping_string, "back");
        }
        2 => {
            // SINPUT_METASTYLE_BACKGUIDE
            button(mapping_string, "back");
            button(mapping_string, "guide");
        }
        3 => {
            // SINPUT_METASTYLE_BACKGUIDESHARE
            button(mapping_string, "back");
            button(mapping_string, "guide");
            button(mapping_string, "misc1");
        }
        _ => {}
    }

    // Paddle 3/4
    if paddle_pairs > 1 {
        // Paddle 4 is first for left/right order of SInput
        button(mapping_string, "paddle4");
        button(mapping_string, "paddle3");
    }

    // Touchpad buttons
    match styles.touch_style {
        1 => {
            // SINPUT_TOUCHSTYLE_SINGLE
            button(mapping_string, "touchpad");
        }
        2 => {
            // SINPUT_TOUCHSTYLE_DOUBLE
            button(mapping_string, "touchpad");
            // Add the second touchpad button at the end of the misc buttons
            button(mapping_string, &format!("misc{misc_end}"));
        }
        _ => {}
    }

    for misc_button in (misc_button..).take(misc_buttons as usize) {
        button(mapping_string, &format!("misc{misc_button}"));
    }
}

/// Decode the SInput features information packed into the version.
/// Translation of `SDL_CreateMappingStringForSInputGamepad()`.
fn create_mapping_string_for_sinput_gamepad(
    mut version: u16,
    face_style: u8,
    mapping_string: &mut String,
    mapping_string_len: usize,
) {
    let face = match face_style {
        2 => "face:axby,",
        3 => "face:bayx,",
        4 => "face:sony,",
        _ => "face:abxy,",
    };
    strlcat(mapping_string, face, mapping_string_len);

    // Interpret the mapping string
    // dynamically based on the feature responses
    let mut decoded = SInputStyles {
        misc_style: version % SINPUT_MISCSTYLE_MAX,
        ..SInputStyles::default()
    };
    version /= SINPUT_MISCSTYLE_MAX;

    decoded.touch_style = version % SINPUT_TOUCHSTYLE_MAX;
    version /= SINPUT_TOUCHSTYLE_MAX;

    decoded.meta_style = version % SINPUT_METASTYLE_MAX;
    version /= SINPUT_METASTYLE_MAX;

    decoded.paddle_style = version % SINPUT_PADDLESTYLE_MAX;
    version /= SINPUT_PADDLESTYLE_MAX;

    decoded.trigger_style = version % SINPUT_TRIGGERSTYLE_MAX;
    version /= SINPUT_TRIGGERSTYLE_MAX;

    decoded.bumper_style = version % SINPUT_BUMPERSTYLE_MAX;
    version /= SINPUT_BUMPERSTYLE_MAX;

    decoded.analog_style = version % SINPUT_ANALOGSTYLE_MAX;

    sinput_styles_map_extraction(&decoded, mapping_string, mapping_string_len);
}

fn add(guid: Guid, mapping_string: &str) -> Option<u64> {
    private_add_mapping_for_guid(guid, mapping_string, MappingPriority::Default)
        .ok()
        .map(|(id, _)| id)
}

/// Guess at a mapping for HIDAPI gamepads.
/// Translation of `SDL_CreateMappingForHIDAPIGamepad()`.
pub(super) fn create_mapping_for_hidapi_gamepad(guid: Guid) -> Option<u64> {
    let mut mapping_string = String::from("none,*,");
    let mut cat = |s: &str| strlcat(&mut mapping_string, s, MAPPING_SIZE);

    let (vendor, product, version, _) = joystick_guid_info(guid);
    let data15 = guid.0[15];

    if is_joystick_wheel(vendor, product, 0) {
        // We don't want to pick up Logitech FFB wheels here
        // Some versions of WINE will also not treat devices that show up as gamepads as wheels
        return None;
    }

    let vertical_joy_cons = || hints::get_bool(hints::JOYSTICK_HIDAPI_VERTICAL_JOY_CONS, false);

    if (vendor == USB_VENDOR_NINTENDO && product == USB_PRODUCT_NINTENDO_GAMECUBE_ADAPTER)
        || (vendor == USB_VENDOR_DRAGONRISE
            && (product == USB_PRODUCT_EVORETRO_GAMECUBE_ADAPTER1
                || product == USB_PRODUCT_EVORETRO_GAMECUBE_ADAPTER2
                || product == USB_PRODUCT_EVORETRO_GAMECUBE_ADAPTER3))
    {
        // GameCube driver has 12 buttons and 6 axes
        cat("a:b0,b:b2,dpdown:b6,dpleft:b4,dpright:b5,dpup:b7,lefttrigger:a4,leftx:a0,lefty:a1~,rightshoulder:b9,righttrigger:a5,rightx:a2,righty:a3~,start:b8,x:b1,y:b3,misc3:b11,misc4:b10,hint:!SDL_GAMECONTROLLER_USE_GAMECUBE_LABELS:=1,");
    } else if vendor == USB_VENDOR_NINTENDO
        && product == USB_PRODUCT_NINTENDO_SWITCH2_GAMECUBE_CONTROLLER
    {
        cat("a:b1,b:b3,dpdown:h0.4,dpleft:h0.8,dpright:h0.2,dpup:h0.1,guide:b4,leftshoulder:b6,lefttrigger:a4,leftx:a0,lefty:a1,rightshoulder:b7,righttrigger:a5,rightx:a2,righty:a3,start:b5,x:b0,y:b2,misc1:b8,misc2:b9,misc3:b10,misc4:b11,hint:!SDL_GAMECONTROLLER_USE_GAMECUBE_LABELS:=1,");
    } else if vendor == USB_VENDOR_NINTENDO && product == USB_PRODUCT_NINTENDO_SWITCH2_PRO {
        cat("a:b0,b:b1,back:b4,dpdown:h0.4,dpleft:h0.8,dpright:h0.2,dpup:h0.1,guide:b5,leftshoulder:b9,leftstick:b7,lefttrigger:a4,leftx:a0,lefty:a1,rightshoulder:b10,rightstick:b8,righttrigger:a5,rightx:a2,righty:a3,start:b6,x:b2,y:b3,misc1:b11,misc2:b12,paddle1:b13,paddle2:b14,hint:!SDL_GAMECONTROLLER_USE_BUTTON_LABELS:=1,");
    } else if vendor == USB_VENDOR_NINTENDO && product == USB_PRODUCT_NINTENDO_SWITCH2_JOYCON_LEFT {
        if vertical_joy_cons() {
            // Vertical mode
            cat("back:b4,dpdown:h0.4,dpleft:h0.8,dpright:h0.2,dpup:h0.1,leftshoulder:b9,leftstick:b7,lefttrigger:a4,leftx:a0,lefty:a1,misc1:b11,paddle2:b14,paddle4:b16,");
        } else {
            // Mini gamepad mode
            cat("a:b1,b:b2,guide:b5,leftshoulder:b9,leftstick:b7,leftx:a0,lefty:a1,rightshoulder:b10,start:b6,x:b3,y:b0,paddle2:b14,paddle4:b16,");
        }
    } else if vendor == USB_VENDOR_NINTENDO && product == USB_PRODUCT_NINTENDO_SWITCH2_JOYCON_RIGHT
    {
        if vertical_joy_cons() {
            // Vertical mode
            cat("a:b0,b:b1,guide:b5,rightshoulder:b10,rightstick:b8,righttrigger:a5,rightx:a2,righty:a3,start:b6,x:b2,y:b3,misc2:b12,paddle1:b13,paddle3:b15,hint:!SDL_GAMECONTROLLER_USE_BUTTON_LABELS:=1,");
        } else {
            // Mini gamepad mode
            cat("a:b1,b:b3,guide:b5,leftshoulder:b9,leftstick:b7,leftx:a0,lefty:a1,rightshoulder:b10,start:b6,x:b0,y:b2,misc2:b12,paddle1:b13,paddle3:b15,hint:!SDL_GAMECONTROLLER_USE_BUTTON_LABELS:=1,");
        }
    } else if vendor == USB_VENDOR_NINTENDO && product == USB_PRODUCT_NINTENDO_SWITCH2_JOYCON_PAIR {
        cat("a:b0,b:b1,back:b4,dpdown:h0.4,dpleft:h0.8,dpright:h0.2,dpup:h0.1,guide:b5,leftshoulder:b9,leftstick:b7,lefttrigger:a4,leftx:a0,lefty:a1,rightshoulder:b10,rightstick:b8,righttrigger:a5,rightx:a2,righty:a3,start:b6,x:b2,y:b3,misc1:b11,misc2:b12,paddle1:b13,paddle2:b14,paddle3:b15,paddle4:b16,hint:!SDL_GAMECONTROLLER_USE_BUTTON_LABELS:=1,");
    } else if vendor == USB_VENDOR_NINTENDO
        && matches!(
            data15,
            SWITCH_DEVICE_INFO_CONTROLLER_TYPE_HVC_LEFT
                | SWITCH_DEVICE_INFO_CONTROLLER_TYPE_HVC_RIGHT
                | SWITCH_DEVICE_INFO_CONTROLLER_TYPE_NES_LEFT
                | SWITCH_DEVICE_INFO_CONTROLLER_TYPE_NES_RIGHT
                | SWITCH_DEVICE_INFO_CONTROLLER_TYPE_SNES
                | SWITCH_DEVICE_INFO_CONTROLLER_TYPE_N64
                | SWITCH_DEVICE_INFO_CONTROLLER_TYPE_SEGA_GENESIS
                | WII_EXTENSION_CONTROLLER_TYPE_NONE
                | WII_EXTENSION_CONTROLLER_TYPE_NUNCHUK
                | SWITCH_DEVICE_INFO_CONTROLLER_TYPE_JOYCON_LEFT
                | SWITCH_DEVICE_INFO_CONTROLLER_TYPE_JOYCON_RIGHT
        )
    {
        match data15 {
            SWITCH_DEVICE_INFO_CONTROLLER_TYPE_HVC_LEFT => {
                cat("a:b0,b:b1,back:b4,dpdown:h0.4,dpleft:h0.8,dpright:h0.2,dpup:h0.1,leftshoulder:b9,rightshoulder:b10,start:b6,");
            }
            SWITCH_DEVICE_INFO_CONTROLLER_TYPE_HVC_RIGHT => {
                cat("a:b0,b:b1,dpdown:h0.4,dpleft:h0.8,dpright:h0.2,dpup:h0.1,leftshoulder:b9,rightshoulder:b10,");
            }
            SWITCH_DEVICE_INFO_CONTROLLER_TYPE_NES_LEFT
            | SWITCH_DEVICE_INFO_CONTROLLER_TYPE_NES_RIGHT => {
                cat("a:b0,b:b1,back:b4,dpdown:h0.4,dpleft:h0.8,dpright:h0.2,dpup:h0.1,leftshoulder:b9,rightshoulder:b10,start:b6,");
            }
            SWITCH_DEVICE_INFO_CONTROLLER_TYPE_SNES => {
                cat("a:b0,b:b1,back:b4,dpdown:h0.4,dpleft:h0.8,dpright:h0.2,dpup:h0.1,leftshoulder:b9,lefttrigger:a4,rightshoulder:b10,righttrigger:a5,start:b6,x:b2,y:b3,hint:!SDL_GAMECONTROLLER_USE_BUTTON_LABELS:=1,");
            }
            SWITCH_DEVICE_INFO_CONTROLLER_TYPE_N64 => {
                cat("a:b0,b:b1,back:b3,dpdown:h0.4,dpleft:h0.8,dpright:h0.2,dpup:h0.1,guide:b5,leftshoulder:b9,lefttrigger:a4,leftx:a0,lefty:a1,misc1:b11,misc2:b4,rightshoulder:b10,righttrigger:b7,start:b6,x:a5,y:b2,");
            }
            SWITCH_DEVICE_INFO_CONTROLLER_TYPE_SEGA_GENESIS => {
                cat("a:b0,b:b1,dpdown:h0.4,dpleft:h0.8,dpright:h0.2,dpup:h0.1,guide:b5,leftshoulder:b9,rightshoulder:b10,righttrigger:a5,start:b6,x:b2,y:b3,misc1:b11,");
            }
            WII_EXTENSION_CONTROLLER_TYPE_NONE => {
                cat("a:b0,b:b1,back:b4,dpdown:h0.4,dpleft:h0.8,dpright:h0.2,dpup:h0.1,guide:b5,start:b6,x:b2,y:b3,");
            }
            WII_EXTENSION_CONTROLLER_TYPE_NUNCHUK => {
                // FIXME: Should we map this to the left or right side?
                const MAP_NUNCHUCK_LEFT_SIDE: bool = true;

                if MAP_NUNCHUCK_LEFT_SIDE {
                    cat("a:b0,b:b1,back:b4,dpdown:b12,dpleft:b13,dpright:b14,dpup:b11,guide:b5,leftshoulder:b9,lefttrigger:a4,leftx:a0,lefty:a1,start:b6,x:b2,y:b3,");
                } else {
                    cat("a:b0,b:b1,back:b4,dpdown:b12,dpleft:b13,dpright:b14,dpup:b11,guide:b5,rightshoulder:b9,righttrigger:a4,rightx:a0,righty:a1,start:b6,x:b2,y:b3,");
                }
            }
            _ => {
                if vertical_joy_cons() {
                    // Vertical mode
                    if data15 == SWITCH_DEVICE_INFO_CONTROLLER_TYPE_JOYCON_LEFT {
                        cat("back:b4,dpdown:h0.4,dpleft:h0.8,dpright:h0.2,dpup:h0.1,leftshoulder:b9,leftstick:b7,lefttrigger:a4,leftx:a0,lefty:a1,misc1:b11,paddle2:b13,paddle4:b15,");
                    } else {
                        cat("a:b0,b:b1,guide:b5,rightshoulder:b10,rightstick:b8,righttrigger:a5,rightx:a2,righty:a3,start:b6,x:b2,y:b3,paddle1:b12,paddle3:b14,");
                    }
                } else {
                    // Mini gamepad mode
                    if data15 == SWITCH_DEVICE_INFO_CONTROLLER_TYPE_JOYCON_LEFT {
                        cat("a:b0,b:b1,guide:b5,leftshoulder:b9,leftstick:b7,leftx:a0,lefty:a1,rightshoulder:b10,start:b6,x:b2,y:b3,paddle2:b13,paddle4:b15,");
                    } else {
                        cat("a:b0,b:b1,guide:b5,leftshoulder:b9,leftstick:b7,leftx:a0,lefty:a1,rightshoulder:b10,start:b6,x:b2,y:b3,paddle1:b12,paddle3:b14,");
                    }
                }
            }
        }
    } else if vendor == USB_VENDOR_8BITDO
        && (product == USB_PRODUCT_8BITDO_SF30_PRO
            || product == USB_PRODUCT_8BITDO_SF30_PRO_BT
            || product == USB_PRODUCT_8BITDO_SN30_PRO
            || product == USB_PRODUCT_8BITDO_SN30_PRO_BT
            || product == USB_PRODUCT_8BITDO_PRO_2
            || product == USB_PRODUCT_8BITDO_PRO_2_BT
            || product == USB_PRODUCT_8BITDO_PRO_3)
    {
        cat("a:b1,b:b0,back:b4,dpdown:h0.4,dpleft:h0.8,dpright:h0.2,dpup:h0.1,guide:b5,leftshoulder:b9,leftstick:b7,lefttrigger:a4,leftx:a0,lefty:a1,rightshoulder:b10,rightstick:b8,righttrigger:a5,rightx:a2,righty:a3,start:b6,x:b3,y:b2,hint:!SDL_GAMECONTROLLER_USE_BUTTON_LABELS:=1,");
        if product == USB_PRODUCT_8BITDO_PRO_2 || product == USB_PRODUCT_8BITDO_PRO_2_BT {
            cat("paddle1:b14,paddle2:b13,");
        } else if product == USB_PRODUCT_8BITDO_PRO_3 {
            cat("paddle1:b12,paddle2:b11,paddle3:b14,paddle4:b13,");
        }
    } else if vendor == USB_VENDOR_8BITDO
        && (product == USB_PRODUCT_8BITDO_SF30_PRO || product == USB_PRODUCT_8BITDO_SF30_PRO_BT)
    {
        // FIXME (upstream): unreachable, the branch above takes these devices
        // This controller has no guide button
        cat("a:b1,b:b0,back:b4,dpdown:h0.4,dpleft:h0.8,dpright:h0.2,dpup:h0.1,leftshoulder:b9,leftstick:b7,lefttrigger:a4,leftx:a0,lefty:a1,rightshoulder:b10,rightstick:b8,righttrigger:a5,rightx:a2,righty:a3,start:b6,x:b3,y:b2,hint:!SDL_GAMECONTROLLER_USE_BUTTON_LABELS:=1,");
    } else if is_joystick_sinput_controller(vendor, product) {
        let face_style = (data15 & 0xE0) >> 5;

        create_mapping_string_for_sinput_gamepad(
            version,
            face_style,
            &mut mapping_string,
            MAPPING_SIZE,
        );
    } else if vendor == USB_VENDOR_MICROSOFT && product == USB_PRODUCT_XBOX360_BIGBUTTON_RECEIVER {
        cat("dpup:h0.1,dpdown:h0.4,dpleft:h0.8,dpright:h0.2,a:b0,b:b1,x:b2,y:b3,back:b4,guide:b5,start:b6,misc1:b7");
    } else {
        // All other gamepads have the standard set of 19 buttons and 6 axes
        if is_joystick_gamecube(vendor, product) {
            cat("a:b0,b:b2,back:b4,dpdown:h0.4,dpleft:h0.8,dpright:h0.2,dpup:h0.1,guide:b5,leftshoulder:b9,leftstick:b7,lefttrigger:a4,leftx:a0,lefty:a1,rightshoulder:b10,rightstick:b8,righttrigger:a5,rightx:a2,righty:a3,start:b6,x:b1,y:b3,hint:!SDL_GAMECONTROLLER_USE_GAMECUBE_LABELS:=1,");
        } else {
            cat("a:b0,b:b1,back:b4,dpdown:h0.4,dpleft:h0.8,dpright:h0.2,dpup:h0.1,guide:b5,leftshoulder:b9,leftstick:b7,lefttrigger:a4,leftx:a0,lefty:a1,rightshoulder:b10,rightstick:b8,righttrigger:a5,rightx:a2,righty:a3,start:b6,x:b2,y:b3,");
        }

        if is_joystick_steam_controller(vendor, product) {
            // Steam controllers have 2 back paddle buttons
            cat("paddle1:b11,paddle2:b12,");
        } else if is_joystick_steam_deck(vendor, product) {
            // The Steam Deck's built-in controller has QAM, 4 back buttons, L/R trackpads, and L/R capacitive touch sticks
            cat("misc1:b11,paddle1:b12,paddle2:b13,paddle3:b14,paddle4:b15,touchpad:b17,misc2:b16");
        } else if is_joystick_steam_triton(vendor, product) {
            // Second generation Steam controllers have 4 back paddle buttons
            cat("misc1:b11,paddle1:b12,paddle2:b13,paddle3:b14,paddle4:b15,touchpad:b17,misc2:b16");
        } else if is_joystick_nintendo_switch_pro(vendor, product)
            || is_joystick_nintendo_switch_pro_input_only(vendor, product)
        {
            // Nintendo Switch Pro controllers have a screenshot button
            cat("misc1:b11,");
        } else if is_joystick_nintendo_switch2_pro(vendor, product)
            || is_joystick_nintendo_switch2_pro_input_only(vendor, product)
        {
            // Nintendo Switch 2 Pro controllers have a screenshot button and C button
            cat("misc1:b11,misc2:b12");
        } else if is_joystick_nintendo_switch_joycon_pair(vendor, product) {
            // The Nintendo Switch Joy-Con combined controllers has a share button and paddles
            cat("misc1:b11,paddle1:b12,paddle2:b13,paddle3:b14,paddle4:b15,");
        } else if is_joystick_amazon_luna_controller(vendor, product) {
            // Amazon Luna Controller has a mic button under the guide button
            cat("misc1:b11,");
        } else if is_joystick_google_stadia_controller(vendor, product) {
            // The Google Stadia controller has a share button and a Google Assistant button
            cat("misc1:b11,misc2:b12,");
        } else if is_joystick_nvidia_shield_controller(vendor, product) {
            // The NVIDIA SHIELD controller has a share button between back and start buttons
            cat("misc1:b11,");

            if product == USB_PRODUCT_NVIDIA_SHIELD_CONTROLLER_V103 {
                // The original SHIELD controller has a touchpad and plus/minus buttons as well
                cat("touchpad:b12,misc2:b13,misc3:b14,");
            }
        } else if is_joystick_hori_steam_controller(vendor, product) {
            /* The Wireless HORIPad for Steam has QAM, Steam, Capsense L/R Sticks, 2 rear buttons, and 2 misc buttons */
            cat("paddle1:b13,paddle2:b12,paddle3:b15,paddle4:b14,misc1:b11");
        } else if is_joystick_flydigi_controller(vendor, product) {
            cat("paddle1:b11,paddle2:b12,paddle3:b13,paddle4:b14,");
            if data15 >= FLYDIGI_VADER2 {
                // Vader series of controllers have C/Z buttons
                cat("misc2:b15,misc3:b16,");
                if data15 == FLYDIGI_VADER5_PRO {
                    // Vader 5 has additional shoulder macro buttons and a circle button
                    cat("misc4:b17,misc5:b18,misc6:b19");
                }
            } else if data15 == FLYDIGI_APEX5 || data15 == FLYDIGI_APEX6 {
                // Apex 5 and Apex 6 have additional shoulder macro buttons
                cat("misc2:b15,misc3:b16,");
            }
        } else if is_joystick_gamesir_controller(vendor, product)
            && u16::from(guid.0[0]) == HARDWARE_BUS_USB
        {
            // The GameSir controllers have a set of paddles and shoulder macro buttons
            cat("misc1:b11,paddle1:b13,paddle2:b12,paddle3:b15,paddle4:b14,");
            if product == USB_PRODUCT_GAMESIR_GAMEPAD_TARANTULA_8K {
                cat("misc2:b16,misc3:b17,misc4:b18,misc5:b19,misc6:b20,");
            }
        } else if vendor == USB_VENDOR_8BITDO && product == USB_PRODUCT_8BITDO_ULTIMATE2_WIRELESS {
            cat("paddle1:b12,paddle2:b11,paddle3:b14,paddle4:b13,");
        } else {
            match gamepad_type_from_guid(guid, None) {
                GamepadType::Ps4 => {
                    // PS4 controllers have an additional touchpad button
                    cat("touchpad:b11,");
                }
                GamepadType::Ps5 => {
                    // PS5 controllers have a microphone button and an additional touchpad button
                    cat("touchpad:b11,misc1:b12,");
                    // DualSense Edge controllers have paddles
                    if is_joystick_dual_sense_edge(vendor, product) {
                        cat("paddle1:b16,paddle2:b15,paddle3:b14,paddle4:b13,");
                    }
                }
                GamepadType::XboxOne => {
                    if is_joystick_xbox_one_elite(vendor, product) {
                        // XBox One Elite Controllers have 4 back paddle buttons
                        cat("paddle1:b11,paddle2:b13,paddle3:b12,paddle4:b14,");
                    } else if is_joystick_xbox_series_x(vendor, product) {
                        // XBox Series X Controllers have a share button under the guide button
                        cat("misc1:b11,");
                    }
                }
                _ => {
                    if vendor == 0 && product == 0 {
                        // This is a Bluetooth Nintendo Switch Pro controller
                        cat("misc1:b11,");
                    }
                }
            }
        }
    }

    add(guid, &mapping_string)
}

/// Guess at a mapping for RAWINPUT gamepads.
/// Translation of `SDL_CreateMappingForRAWINPUTGamepad()`.
pub(super) fn create_mapping_for_rawinput_gamepad(guid: Guid) -> Option<u64> {
    let mut mapping_string = String::from("none,*,");
    strlcat(
        &mut mapping_string,
        "a:b0,b:b1,x:b2,y:b3,back:b6,guide:b10,start:b7,leftstick:b8,rightstick:b9,leftshoulder:b4,rightshoulder:b5,dpup:h0.1,dpdown:h0.4,dpleft:h0.8,dpright:h0.2,leftx:a0,lefty:a1,rightx:a2,righty:a3,lefttrigger:a4,righttrigger:a5,",
        MAPPING_SIZE,
    );

    add(guid, &mapping_string)
}

/// Guess at a mapping for WGI gamepads.
/// Translation of `SDL_CreateMappingForWGIGamepad()`.
pub(super) fn create_mapping_for_wgi_gamepad(guid: Guid) -> Option<u64> {
    if guid.0[15] != JoystickType::Gamepad as u8 {
        return None;
    }

    let mut mapping_string = String::from("none,*,");
    strlcat(
        &mut mapping_string,
        "a:b0,b:b1,x:b2,y:b3,back:b6,start:b7,leftstick:b8,rightstick:b9,leftshoulder:b4,rightshoulder:b5,dpup:b10,dpdown:b12,dpleft:b13,dpright:b11,leftx:a1,lefty:a0~,rightx:a3,righty:a2~,lefttrigger:a4,righttrigger:a5,",
        MAPPING_SIZE,
    );

    add(guid, &mapping_string)
}
