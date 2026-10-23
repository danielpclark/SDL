// Rust translation of src/sensor/emscripten/SDL_emscriptensensor.c and
// SDL_emscriptensensor.h from Simple DirectMedia Layer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! The Emscripten sensor driver: the browser's `devicemotion` events.
//!
//! There are always two sensors, an accelerometer and a gyroscope, whether
//! the device has them or not (a desktop browser, node). Each `devicemotion`
//! event on the window (`emscripten_set_devicemotion_callback()`, set up
//! when the driver initializes) stores both sensors' values; the next
//! update of an open sensor sends what's new. The browser delivers the
//! events from its event loop, between main loop iterations.

use std::ffi::{c_int, c_void};
use std::sync::{Mutex, MutexGuard};
use std::time::Duration;

use sdl3_emscripten::sys::{self, EmscriptenDeviceMotionEvent};

use super::{send_sensor_update, SensorData, SensorDriver, SensorType, STANDARD_GRAVITY};
use crate::error::Result;
use crate::events::SensorID;

/// Translation of `EMSCRIPTEN_SENSOR_COUNT`.
const EMSCRIPTEN_SENSOR_COUNT: usize = 2;

/// Translation of `SDL_EmscriptenSensor`.
#[derive(Clone, Copy)]
struct EmscriptenSensor {
    sensor_type: SensorType,
    instance_id: SensorID,
    data: [f32; 3],
    new_data: bool,
}

const NO_SENSOR: EmscriptenSensor = EmscriptenSensor {
    sensor_type: SensorType::Invalid,
    instance_id: 0,
    data: [0.0; 3],
    new_data: false,
};

/// Translation of `SDL_sensors`.
static SENSORS: Mutex<[EmscriptenSensor; EMSCRIPTEN_SENSOR_COUNT]> =
    Mutex::new([NO_SENSOR; EMSCRIPTEN_SENSOR_COUNT]);

fn sensors() -> MutexGuard<'static, [EmscriptenSensor; EMSCRIPTEN_SENSOR_COUNT]> {
    SENSORS.lock().unwrap_or_else(|e| e.into_inner())
}

/// Translation of `SDL_EMSCRIPTEN_AccelerometerCallback()`.
fn accelerometer_callback(
    sensors: &mut [EmscriptenSensor; EMSCRIPTEN_SENSOR_COUNT],
    event: &EmscriptenDeviceMotionEvent,
) {
    // Convert from browser specific gravity constant to SDL_STANDARD_GRAVITY.
    let mut total_gravity = 0.0;
    total_gravity += (event.accelerationIncludingGravityX - event.accelerationX).abs();
    total_gravity += (event.accelerationIncludingGravityY - event.accelerationY).abs();
    total_gravity += (event.accelerationIncludingGravityZ - event.accelerationZ).abs();

    let gravity = [
        (event.accelerationIncludingGravityX - event.accelerationX) / total_gravity,
        (event.accelerationIncludingGravityY - event.accelerationY) / total_gravity,
        (event.accelerationIncludingGravityZ - event.accelerationZ) / total_gravity,
    ];

    // (SDL_STANDARD_GRAVITY is a float constant, widened in the double
    // arithmetic)
    let g = f64::from(STANDARD_GRAVITY);
    sensors[0].data[0] = (event.accelerationX + gravity[0] * g) as f32;
    sensors[0].data[1] = (event.accelerationY + gravity[1] * g) as f32;
    sensors[0].data[2] = (event.accelerationZ + gravity[2] * g) as f32;
    sensors[0].new_data = true;
}

/// Translation of `SDL_EMSCRIPTEN_GyroscopeCallback()`.
fn gyroscope_callback(
    sensors: &mut [EmscriptenSensor; EMSCRIPTEN_SENSOR_COUNT],
    event: &EmscriptenDeviceMotionEvent,
) {
    use std::f32::consts::PI; // SDL_PI_F
    sensors[1].data[0] = event.rotationRateAlpha as f32 * PI / 180.0;
    sensors[1].data[1] = event.rotationRateBeta as f32 * PI / 180.0;
    sensors[1].data[2] = event.rotationRateGamma as f32 * PI / 180.0;
    sensors[1].new_data = true;
}

/// Translation of `SDL_EMSCRIPTEN_SensorCallback()`: emscripten's
/// `devicemotion` handler calls it with the event it filled in.
unsafe extern "C-unwind" fn sensor_callback(
    _event_type: c_int,
    event: *const EmscriptenDeviceMotionEvent,
    _user_data: *mut c_void,
) -> bool {
    // SAFETY: emscripten passes its event structure, filled in.
    let event = unsafe { &*event };
    let mut sensors = sensors();
    accelerometer_callback(&mut sensors, event);
    gyroscope_callback(&mut sensors, event);

    true
}

/// Translation of `SDL_EMSCRIPTEN_SensorDriver`.
pub(super) struct EmscriptenSensorDriver;

pub(super) static EMSCRIPTEN_SENSOR_DRIVER: EmscriptenSensorDriver = EmscriptenSensorDriver;

impl SensorDriver for EmscriptenSensorDriver {
    fn init(&self) -> Result<()> {
        // (upstream ignores the result: under node, without a window, it's
        // EMSCRIPTEN_RESULT_UNKNOWN_TARGET and nothing is registered. Each
        // initialization registers the callback again, and nothing removes
        // it: SensorQuit() does nothing.)
        // SAFETY: an extern "C-unwind" fn with the parameters emscripten calls it
        // with, for the life of the program; the user data isn't used.
        unsafe {
            sys::emscripten_set_devicemotion_callback_on_thread(
                std::ptr::null_mut(),
                false,
                Some(sensor_callback),
                sys::EM_CALLBACK_THREAD_CONTEXT_CALLING_THREAD,
            )
        };

        let mut sensors = sensors();
        sensors[0].sensor_type = SensorType::Accel;
        sensors[0].instance_id = crate::utils::next_object_id();
        sensors[0].new_data = false;
        sensors[1].sensor_type = SensorType::Gyro;
        sensors[1].instance_id = crate::utils::next_object_id();
        sensors[1].new_data = false;

        Ok(())
    }

    fn count(&self) -> usize {
        EMSCRIPTEN_SENSOR_COUNT
    }

    fn detect(&self) {}

    fn device_name(&self, device_index: usize) -> Option<String> {
        if device_index < EMSCRIPTEN_SENSOR_COUNT {
            let name = match sensors()[device_index].sensor_type {
                SensorType::Accel => "Accelerometer",
                SensorType::Gyro => "Gyroscope",
                _ => "Unknown",
            };
            return Some(name.to_owned());
        }

        None
    }

    fn device_type(&self, device_index: usize) -> SensorType {
        if device_index < EMSCRIPTEN_SENSOR_COUNT {
            return sensors()[device_index].sensor_type;
        }

        SensorType::Invalid
    }

    fn device_non_portable_type(&self, device_index: usize) -> i32 {
        if device_index < EMSCRIPTEN_SENSOR_COUNT {
            return sensors()[device_index].sensor_type as i32;
        }

        -1
    }

    fn device_instance_id(&self, device_index: usize) -> SensorID {
        if device_index < EMSCRIPTEN_SENSOR_COUNT {
            return sensors()[device_index].instance_id;
        }

        // (-1 as an SDL_SensorID)
        SensorID::MAX
    }

    fn open(&self, _sensor: &mut SensorData, _device_index: usize) -> Result<()> {
        Ok(())
    }

    fn update(&self, sensor: SensorID) {
        let Some(sensor_type) = super::with_sensor(sensor, |s| s.sensor_type) else {
            return;
        };
        let index = match sensor_type {
            SensorType::Accel => 0,
            SensorType::Gyro => 1,
            _ => return,
        };
        // (the data is taken before it's sent: the event's watchers may
        // reach the sensors again)
        let data = {
            let mut sensors = sensors();
            if !sensors[index].new_data {
                return;
            }
            sensors[index].new_data = false;
            sensors[index].data
        };
        let timestamp = crate::timer::ticks_ns();
        send_sensor_update(Duration::from_nanos(timestamp), sensor, timestamp, &data);
    }

    fn close(&self, _sensor: &mut SensorData) {}

    fn quit(&self) {}
}
