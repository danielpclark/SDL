// Unit tests of the gamepad mapping string handling (the database as a
// whole is compared with upstream in joystick::tests).

use super::*;

#[test]
fn mapping_string_fields() {
    let m = "03000000aaaa0000bbbb000000000000, Name ,  a:b0,b:b1 \t";
    assert_eq!(
        private_get_gamepad_guid_from_mapping_string(m).as_deref(),
        Some("03000000aaaa0000bbbb000000000000")
    );
    assert_eq!(
        private_get_gamepad_name_from_mapping_string(m).as_deref(),
        Some(" Name ")
    );
    assert_eq!(
        private_get_gamepad_mapping_from_mapping_string(m).as_deref(),
        Some("a:b0,b:b1")
    );
    assert_eq!(
        private_get_gamepad_name_from_mapping_string("guid,name"),
        None
    );
    assert_eq!(private_get_gamepad_guid_from_mapping_string("guid"), None);
}

#[test]
fn created_mapping_strings() {
    let _lock = lock_joysticks();
    let platform = crate::init::platform();
    let guid = Guid::ZERO;
    assert_eq!(
        create_mapping_string("Pad", "a:b0", guid),
        format!("{guid},Pad,a:b0,platform:{platform},")
    );
    assert_eq!(
        create_mapping_string("Pad", "a:b0,", guid),
        format!("{guid},Pad,a:b0,platform:{platform},")
    );
    // An empty mapping gets no empty field before the platform
    assert_eq!(
        create_mapping_string("Pad", "", guid),
        format!("{guid},Pad,platform:{platform},")
    );
    assert_eq!(
        create_mapping_string("Pad", "a:b0,platform:Other,", guid),
        format!("{guid},Pad,a:b0,platform:Other,")
    );
}

#[test]
fn parse_elements() {
    let _lock = lock_joysticks();
    let mut bindings = Vec::new();
    private_parse_gamepad_config_string(
        &mut bindings,
        "a:b0,leftx:-a1~,+righty:+a2,dpup:h0.4,lefttrigger:a5,bogus:b9,a:b0",
    )
    .unwrap();
    assert_eq!(
        bindings,
        vec![
            GamepadBinding {
                input: GamepadBindingInput::Button(0),
                output: GamepadBindingOutput::Button(GamepadButton::South),
            },
            GamepadBinding {
                input: GamepadBindingInput::Axis {
                    axis: 1,
                    axis_min: -32768,
                    axis_max: 0
                },
                output: GamepadBindingOutput::Axis {
                    axis: GamepadAxis::LeftX,
                    axis_min: -32768,
                    axis_max: 32767
                },
            },
            GamepadBinding {
                input: GamepadBindingInput::Axis {
                    axis: 2,
                    axis_min: 0,
                    axis_max: 32767
                },
                output: GamepadBindingOutput::Axis {
                    axis: GamepadAxis::RightY,
                    axis_min: 0,
                    axis_max: 32767
                },
            },
            GamepadBinding {
                input: GamepadBindingInput::Hat {
                    hat: 0,
                    hat_mask: 4
                },
                output: GamepadBindingOutput::Button(GamepadButton::DpadUp),
            },
            GamepadBinding {
                input: GamepadBindingInput::Axis {
                    axis: 5,
                    axis_min: -32768,
                    axis_max: 32767
                },
                output: GamepadBindingOutput::Axis {
                    axis: GamepadAxis::LeftTrigger,
                    axis_min: 0,
                    axis_max: 32767
                },
            },
        ]
    );

    // A second ':' restarts the joystick input over the previous characters
    let mut bindings = Vec::new();
    private_parse_gamepad_config_string(&mut bindings, "b:b12:b3,").unwrap();
    assert_eq!(bindings[0].input, GamepadBindingInput::Button(32));

    let mut bindings = Vec::new();
    let e =
        private_parse_gamepad_config_string(&mut bindings, "a:b0,averyveryverylongbuttonname:b1")
            .unwrap_err();
    assert_eq!(e.to_string(), "Button name too large: averyveryverylongbu");
    assert_eq!(bindings.len(), 1);
}

#[test]
fn positional_conversion() {
    assert_eq!(
        convert_mapping_to_positional_baxy(
            "g,n,a:b0,b:b1,x:b2,y:b3,hint:SDL_GAMECONTROLLER_USE_BUTTON_LABELS:=1,"
        ),
        "g,n,b:b0,a:b1,y:b2,x:b3,hint:!SDL_GAMECONTROLLER_USE_BUTTON_LABELS:=1,"
    );
    assert_eq!(
        convert_mapping_to_positional_axby(
            "g,n,a:b0,b:b1,x:b2,hint:SDL_GAMECONTROLLER_USE_GAMECUBE_LABELS:=1,"
        ),
        "g,n,a:b0,x:b1,b:b2,hint:!SDL_GAMECONTROLLER_USE_GAMECUBE_LABELS:=1,"
    );
}

#[test]
fn hidapi_fixup() {
    let _lock = lock_joysticks();
    let button = |input, output| GamepadBinding {
        input: GamepadBindingInput::Button(input),
        output: GamepadBindingOutput::Button(output),
    };
    let mut bindings = vec![
        button(0, GamepadButton::South),
        button(11, GamepadButton::DpadUp),
        button(14, GamepadButton::DpadRight),
        button(15, GamepadButton::Misc1),
    ];
    fixup_hidapi_mapping(&mut bindings);
    assert_eq!(
        bindings[1].input,
        GamepadBindingInput::Hat {
            hat: 0,
            hat_mask: 1
        }
    );
    assert_eq!(
        bindings[2].input,
        GamepadBindingInput::Hat {
            hat: 0,
            hat_mask: 2
        }
    );
    assert_eq!(bindings[3].input, GamepadBindingInput::Button(11));
}

#[test]
fn ignored_gamepads() {
    assert!(should_ignore_gamepad(0x1234, 0x5678, 0, Some("uinput-fpc")));
    assert!(should_ignore_gamepad(
        0x1234,
        0x5678,
        0,
        Some("Usb Keyboard Consumer Control")
    ));
    assert!(!should_ignore_gamepad(
        0x1234,
        0x5678,
        0,
        Some("PG-9076 Keyboard")
    ));
    assert!(!should_ignore_gamepad(
        0x1234,
        0x5678,
        0,
        Some("Xbox Controller")
    ));
}
