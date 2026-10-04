// Tests for the Nintendo Switch HIDAPI drivers.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! The cases in `data.rs` come from running upstream's
//! SDL_hidapi_switch.c, with stubs for the SDL functions it uses, on
//! generated reports, with its stick and IMU calibration read from
//! generated SPI flash replies.

use super::super::tests::{run, test_device};
use super::*;
use crate::hidapi::{BusType, DeviceInfo};

mod data;

pub(super) struct StateCase {
    name: &'static str,
    controller_type: u8,
    vertical: bool,
    labels: bool,
    switch2: bool,
    sensors: bool,
    stick: Option<([u8; 9], [u8; 9])>,
    /// Per stick and axis: center, min, max, the extents' min and max and
    /// the simple extents' min and max
    calibration: &'static [[i16; 7]],
    imu: Option<(&'static [u8], Option<&'static [u8]>)>,
    scale: Option<([u32; 6], [i16; 3])>,
    reports: &'static [(&'static [u8], &'static [&'static str])],
}

pub(super) struct RumbleCase {
    high_amplitude: u16,
    low_amplitude: u16,
    high: u8,
    low: u16,
    product_id: u16,
    data: [u8; 4],
}

fn pro_controller(bluetooth: bool) -> Arc<HidapiDevice> {
    test_device(&DeviceInfo {
        vendor_id: USB_VENDOR_NINTENDO,
        product_id: USB_PRODUCT_NINTENDO_SWITCH_PRO,
        interface_number: 0,
        bus_type: if bluetooth {
            BusType::Bluetooth
        } else {
            BusType::Usb
        },
        ..DeviceInfo::default()
    })
}

fn scale_bits(scale: &ImuScaleData) -> ([u32; 6], [i16; 3]) {
    (
        [
            scale.accel_scale_x.to_bits(),
            scale.accel_scale_y.to_bits(),
            scale.accel_scale_z.to_bits(),
            scale.gyro_scale_x.to_bits(),
            scale.gyro_scale_y.to_bits(),
            scale.gyro_scale_z.to_bits(),
        ],
        [
            scale.gyro_offset_x,
            scale.gyro_offset_y,
            scale.gyro_offset_z,
        ],
    )
}

#[test]
fn state_reports() {
    let device = pro_controller(false);
    for (n, case) in data::STATE_CASES.iter().enumerate() {
        let mut ctx = SwitchContext {
            controller_type: ControllerType(case.controller_type),
            vertical_mode: case.vertical,
            use_button_labels: case.labels,
            switch2: case.switch2,
            report_sensors: case.sensors,
            input_only: case.name == "input_only",
            imu_update_interval_ns: 5_000_000,
            max_write_attempts: 5,
            sync_write: true,
            ..SwitchContext::default()
        };

        if let Some((left, right)) = case.stick {
            ctx.set_stick_calibration(&left, &right);
            let calibration: Vec<[i16; 7]> = (0..4)
                .map(|i| {
                    let (stick, axis) = (i / 2, i % 2);
                    let cal = ctx.stick_cal_data[stick][axis];
                    let extents = ctx.stick_extents[stick][axis];
                    let simple = ctx.simple_stick_extents[stick][axis];
                    [
                        cal.center,
                        cal.min,
                        cal.max,
                        extents.min,
                        extents.max,
                        simple.min,
                        simple.max,
                    ]
                })
                .collect();
            assert_eq!(calibration, case.calibration, "case {n} calibration");
        }
        if let Some((factory, user)) = case.imu {
            ctx.imu_scale_data = imu_scale_data(factory, user);
            assert_eq!(
                Some(scale_bits(&ctx.imu_scale_data)),
                case.scale,
                "case {n} IMU scale"
            );
        }

        for (r, (report, expected)) in case.reports.iter().enumerate() {
            let mut packet = [0u8; SWITCH_MAX_OUTPUT_PACKET_LENGTH];
            packet[..report.len()].copy_from_slice(report);
            let (_, events) = run(&device, |d| match case.name {
                "input_only" => ctx.handle_input_only_controller_state(d, 1, &packet),
                "simple" => ctx.handle_simple_controller_state(d, 1, &packet),
                _ => ctx.handle_full_controller_state(d, Link::Device(&device), 1, &packet),
            });
            assert_eq!(events, *expected, "case {n} ({}) report {r}", case.name);
        }
    }
}

#[test]
fn imu_defaults() {
    assert_eq!(scale_bits(&default_imu_scale_data()), data::DEFAULT_SCALE);
}

#[test]
fn rumble_encoding() {
    for (n, case) in data::RUMBLE_CASES.iter().enumerate() {
        assert_eq!(
            encode_rumble_high_amplitude(case.high_amplitude),
            case.high,
            "rumble {n}"
        );
        assert_eq!(
            encode_rumble_low_amplitude(case.low_amplitude),
            case.low,
            "rumble {n}"
        );
        let high = if n % 5 != 0 { case.high } else { 0 };
        let low = if n % 7 != 0 { case.low } else { 0 };
        assert_eq!(
            encode_rumble(
                USB_VENDOR_NINTENDO,
                case.product_id,
                0x0074,
                high,
                0x3D,
                low
            ),
            case.data,
            "rumble {n}"
        );
    }
}

#[test]
fn output_packets() {
    let mut ctx = SwitchContext {
        command_number: 15,
        rumble_packet: CommonOutputPacket {
            rumble_data: [[1, 2, 3, 4], [5, 6, 7, 8]],
            ..CommonOutputPacket::default()
        },
        ..SwitchContext::default()
    };
    let packet = ctx.construct_subcommand(SUBCOMMAND_SPI_FLASH_READ, &spi_op_data(0x603D, 0x12));
    assert_eq!(
        packet[..16],
        [0x01, 15, 1, 2, 3, 4, 5, 6, 7, 8, 0x10, 0x3D, 0x60, 0x00, 0x00, 0x12]
    );
    assert!(packet[16..].iter().all(|&b| b == 0));
    // The command number wraps at 16
    assert_eq!(ctx.command_number, 0);
    assert_eq!(SPI_STICK_FACTORY_CALIBRATION_LENGTH, 0x12);
    assert_eq!(SPI_STICK_USER_CALIBRATION_LENGTH, 0x16);
    assert_eq!(SPI_IMU_SCALE_LENGTH, 0x18);
    assert_eq!(SPI_IMU_USER_SCALE_LENGTH, 0x14);
    assert_eq!(SUBCOMMAND_DATA_SIZE, 38);

    // Without a device, nothing is written
    let device = pro_controller(false);
    ctx.max_write_attempts = 2;
    ctx.sync_write = true;
    assert!(!ctx.write_subcommand(Link::Device(&device), SUBCOMMAND_ENABLE_IMU, &[1]));
    // Too much subcommand data isn't sent
    assert!(!ctx.write_subcommand(Link::Device(&device), SUBCOMMAND_ENABLE_IMU, &[0; 39]));
    assert!(!ctx.write_packet(Link::Device(&device), &[0; 50]));
}

#[test]
fn home_led() {
    assert_eq!(home_led_intensity(0), 0);
    assert_eq!(home_led_intensity(5), 1);
    assert_eq!(home_led_intensity(64), 6);
    // ceil(15 * 0.65^2.13)
    assert_eq!(home_led_intensity(65), 6);
    assert_eq!(home_led_intensity(100), 15);
    assert_eq!(home_led_hint_value(None), None);
    assert_eq!(home_led_hint_value(Some("")), None);
    assert_eq!(home_led_hint_value(Some("1")), Some(100));
    assert_eq!(home_led_hint_value(Some("0")), Some(0));
    assert_eq!(home_led_hint_value(Some("0.5")), Some(50));
    assert_eq!(home_led_hint_value(Some("9.0")), Some(255));
    assert_eq!(home_led_hint_value(Some("-0.5")), Some(206));
}

#[test]
fn input_modes() {
    let usb = pro_controller(false);
    let bluetooth = pro_controller(true);
    let mut ctx = SwitchContext::default();

    // Wired controllers never use simple reports
    assert_eq!(
        ctx.get_default_input_mode(&usb),
        INPUT_REPORT_FULL_CONTROLLER_STATE
    );
    assert_eq!(
        ctx.get_default_input_mode(&bluetooth),
        INPUT_REPORT_SIMPLE_CONTROLLER_STATE
    );
    ctx.enhanced_report_hint = EnhancedReportHint::On;
    assert_eq!(
        ctx.get_default_input_mode(&bluetooth),
        INPUT_REPORT_FULL_CONTROLLER_STATE
    );
    ctx.initial_input_mode = INPUT_REPORT_FULL_CONTROLLER_AND_MCU_STATE;
    assert_eq!(
        ctx.get_default_input_mode(&bluetooth),
        INPUT_REPORT_FULL_CONTROLLER_AND_MCU_STATE
    );
    ctx.enhanced_report_hint = EnhancedReportHint::Auto;
    assert_eq!(
        ctx.get_default_input_mode(&bluetooth),
        INPUT_REPORT_FULL_CONTROLLER_AND_MCU_STATE
    );
    assert_eq!(
        ctx.get_sensor_input_mode(),
        INPUT_REPORT_FULL_CONTROLLER_AND_MCU_STATE
    );
    ctx.initial_input_mode = INPUT_REPORT_SIMPLE_CONTROLLER_STATE;
    assert_eq!(
        ctx.get_sensor_input_mode(),
        INPUT_REPORT_FULL_CONTROLLER_STATE
    );

    // Joy-Cons use full reports in auto mode
    let joycon = test_device(&DeviceInfo {
        vendor_id: USB_VENDOR_NINTENDO,
        product_id: USB_PRODUCT_NINTENDO_SWITCH_JOYCON_LEFT,
        bus_type: BusType::Bluetooth,
        ..DeviceInfo::default()
    });
    assert_eq!(
        ctx.get_default_input_mode(&joycon),
        INPUT_REPORT_FULL_CONTROLLER_STATE
    );
    ctx.enhanced_report_hint = EnhancedReportHint::Off;
    assert_eq!(
        ctx.get_default_input_mode(&joycon),
        INPUT_REPORT_SIMPLE_CONTROLLER_STATE
    );
}

#[test]
fn controller_types() {
    let grip = |interface_number| {
        test_device(&DeviceInfo {
            vendor_id: USB_VENDOR_NINTENDO,
            product_id: USB_PRODUCT_NINTENDO_SWITCH_JOYCON_GRIP,
            interface_number,
            ..DeviceInfo::default()
        })
    };
    assert_eq!(
        calculate_controller_type(&grip(1), ControllerType::UNKNOWN),
        ControllerType::JOYCON_LEFT
    );
    assert_eq!(
        calculate_controller_type(&grip(2), ControllerType::UNKNOWN),
        ControllerType::JOYCON_RIGHT
    );
    assert_eq!(
        calculate_controller_type(&grip(1), ControllerType(4)),
        ControllerType(4)
    );
    let n64 = test_device(&DeviceInfo {
        vendor_id: USB_VENDOR_NINTENDO,
        product_id: USB_PRODUCT_NINTENDO_N64_CONTROLLER,
        ..DeviceInfo::default()
    });
    assert_eq!(
        calculate_controller_type(&n64, ControllerType::PRO_CONTROLLER),
        ControllerType::N64
    );
    assert_eq!(
        neutral_rumble(USB_VENDOR_NINTENDO, USB_PRODUCT_NINTENDO_N64_CONTROLLER),
        [0, 0, 1, 0x40]
    );

    let mut ctx = SwitchContext {
        controller_type: ControllerType::PRO_CONTROLLER,
        ..SwitchContext::default()
    };
    assert!(ctx.has_home_led(USB_VENDOR_NINTENDO, USB_PRODUCT_NINTENDO_SWITCH_PRO));
    assert!(!ctx.has_home_led(0, 0));
    assert!(!ctx.has_home_led(0x0f0d, 0x00f6));
    ctx.controller_type = ControllerType::SNES;
    assert!(!ctx.has_home_led(USB_VENDOR_NINTENDO, USB_PRODUCT_NINTENDO_SNES_CONTROLLER));
    assert!(ctx.has_home_led(0x1234, 0x5678));
    ctx.controller_type = ControllerType::LIC_PRO_CONTROLLER;
    assert!(!ctx.has_home_led(0x1234, 0x5678));

    assert!(always_uses_labels(0, 0, ControllerType::N64));
    assert!(!always_uses_labels(0, 0, ControllerType::SNES));
    assert!(always_uses_labels(0, 0, ControllerType::SEGA_GENESIS));

    ctx.use_button_labels = true;
    assert_eq!(
        ctx.remap_button(GamepadButton::South),
        GamepadButton::East as u8
    );
    assert_eq!(
        ctx.remap_button(GamepadButton::West),
        GamepadButton::North as u8
    );
    assert_eq!(
        ctx.remap_button(GamepadButton::Back),
        GamepadButton::Back as u8
    );
}

#[test]
fn device_identity() {
    let _l = crate::test_support::test_lock();
    let _lock = crate::joystick::lock_joysticks();

    let device = test_device(&DeviceInfo {
        vendor_id: USB_VENDOR_NINTENDO,
        product_id: USB_PRODUCT_NINTENDO_SWITCH_JOYCON_GRIP,
        interface_number: 1,
        ..DeviceInfo::default()
    });
    let mut ctx = SwitchContext {
        controller_type: ControllerType::JOYCON_LEFT,
        mac_address: [0x98, 0xb6, 0xe9, 0x01, 0x02, 0x03],
        ..SwitchContext::default()
    };
    run(&device, |d| ctx.update_device_identity(d));
    assert_eq!(device.name(), "Nintendo Switch Joy-Con (L)");
    assert_eq!(device.gamepad_type(), GamepadType::NintendoSwitchJoyconLeft);
    assert_eq!(device.serial().as_deref(), Some("98-b6-e9-01-02-03"));
    assert_eq!(device.guid().0[15], 1);
}

#[test]
fn supported_devices() {
    let classic = NintendoClassicDriver;
    let joycons = JoyConsDriver;
    let switch = SwitchDriver;
    let supported = |driver: &dyn DriverImpl, name: &str, gamepad_type, product_id| {
        driver.is_supported_device(
            None,
            name,
            gamepad_type,
            USB_VENDOR_NINTENDO,
            product_id,
            0,
            0,
            0,
            0,
            0,
        )
    };
    let pro = GamepadType::NintendoSwitchPro;
    assert!(supported(
        &classic,
        "NES Controller (R)",
        pro,
        USB_PRODUCT_NINTENDO_SWITCH_JOYCON_RIGHT
    ));
    assert!(!supported(
        &classic,
        "Joy-Con (R)",
        pro,
        USB_PRODUCT_NINTENDO_SWITCH_JOYCON_RIGHT
    ));
    assert!(supported(
        &classic,
        "",
        pro,
        USB_PRODUCT_NINTENDO_N64_CONTROLLER
    ));
    assert!(supported(
        &joycons,
        "Joy-Con (R)",
        pro,
        USB_PRODUCT_NINTENDO_SWITCH_JOYCON_RIGHT
    ));
    assert!(!supported(
        &joycons,
        "",
        pro,
        USB_PRODUCT_NINTENDO_SWITCH_PRO
    ));
    assert!(supported(&switch, "", pro, USB_PRODUCT_NINTENDO_SWITCH_PRO));
    assert!(!supported(
        &switch,
        "HORI Wireless Switch Pad",
        pro,
        USB_PRODUCT_NINTENDO_SWITCH_PRO
    ));
    assert!(!supported(
        &switch,
        "",
        pro,
        USB_PRODUCT_NINTENDO_SWITCH2_PRO
    ));
    assert!(!supported(
        &switch,
        "",
        pro,
        USB_PRODUCT_NINTENDO_SWITCH_JOYCON_LEFT
    ));
    assert!(!supported(&switch, "", GamepadType::Standard, 0x1234));
}
