// Rust translation of src/sensor/coremotion/SDL_coremotionsensor.m and
// SDL_coremotionsensor.h from Simple DirectMedia Layer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! The CoreMotion sensor driver (`SDL_SENSOR_COREMOTION`, which upstream's
//! CMake build has on iOS and visionOS): the device's accelerometer and
//! gyroscope, through a `CMMotionManager` made when the driver first
//! initializes and kept for the life of the process (as upstream's). Each
//! sensor the manager has (`accelerometerAvailable`, `gyroAvailable`) is
//! one sensor; opening it starts the manager's updates, closing it stops
//! them, and each update of an open sensor sends the manager's latest data
//! when it changed. The iOS Simulator has neither, so there are no sensors
//! there.
//!
//! CoreMotion is linked, as upstream links it.

use std::sync::{Mutex, MutexGuard};
use std::time::Duration;

use super::{send_sensor_update, SensorData, SensorDriver, SensorType, STANDARD_GRAVITY};
use crate::core::apple::objc::{class, msg_send, Bool, Id, StrongPtr};
use crate::error::Result;
use crate::events::SensorID;

#[link(name = "CoreMotion", kind = "framework")]
unsafe extern "C" {}

/// `CMAcceleration` (in G's).
#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
struct CMAcceleration {
    x: f64,
    y: f64,
    z: f64,
}

/// `CMRotationRate` (in radians per second).
#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
struct CMRotationRate {
    x: f64,
    y: f64,
    z: f64,
}

/// Translation of `SDL_CoreMotionSensor`.
#[derive(Clone, Copy)]
struct CoreMotionSensor {
    sensor_type: SensorType,
    instance_id: SensorID,
}

/// Translation of `struct sensor_hwdata`: the data last sent.
struct SensorHwdata {
    data: [f32; 3],
}

/// The driver's state: `SDL_motion_manager` and `SDL_sensors` (whose length
/// is `SDL_sensors_count`).
struct CoreMotionState {
    motion_manager: StrongPtr,
    sensors: Vec<CoreMotionSensor>,
}

static STATE: Mutex<CoreMotionState> = Mutex::new(CoreMotionState {
    motion_manager: StrongPtr::nil(),
    sensors: Vec::new(),
});

fn state() -> MutexGuard<'static, CoreMotionState> {
    STATE.lock().unwrap_or_else(|e| e.into_inner())
}

/// The motion manager (nil before the driver initializes).
fn motion_manager() -> StrongPtr {
    state().motion_manager.clone()
}

/// Translation of `SDL_COREMOTION_SensorDriver`.
pub(super) struct CoreMotionSensorDriver;

pub(super) static COREMOTION_SENSOR_DRIVER: CoreMotionSensorDriver = CoreMotionSensorDriver;

impl SensorDriver for CoreMotionSensorDriver {
    /// Translation of `SDL_COREMOTION_SensorInit()`.
    fn init(&self) -> Result<()> {
        let mut state = state();
        if state.motion_manager.is_nil() {
            // SAFETY: CMMotionManager's alloc and init (owned).
            state.motion_manager = unsafe {
                let manager: Id = msg_send![class!(CMMotionManager), alloc];
                StrongPtr::from_owned(msg_send![manager, init])
            };
        }
        let manager = state.motion_manager.clone();

        // SAFETY: CMMotionManager's BOOL properties.
        let (accelerometer_available, gyro_available): (Bool, Bool) = unsafe {
            (
                msg_send![manager, isAccelerometerAvailable],
                msg_send![manager, isGyroAvailable],
            )
        };
        let accelerometer_available = accelerometer_available.is_true();
        let gyro_available = gyro_available.is_true();

        let mut sensors = Vec::new();
        if accelerometer_available {
            sensors.push(CoreMotionSensor {
                sensor_type: SensorType::Accel,
                instance_id: crate::utils::next_object_id(),
            });
        }
        if gyro_available {
            sensors.push(CoreMotionSensor {
                sensor_type: SensorType::Gyro,
                instance_id: crate::utils::next_object_id(),
            });
        }
        // (without sensors, those of an earlier initialization are kept, as
        // upstream's count is)
        if !sensors.is_empty() {
            state.sensors = sensors;
        }
        Ok(())
    }

    /// Translation of `SDL_COREMOTION_SensorGetCount()`.
    fn count(&self) -> usize {
        state().sensors.len()
    }

    /// Translation of `SDL_COREMOTION_SensorDetect()`.
    fn detect(&self) {}

    /// Translation of `SDL_COREMOTION_SensorGetDeviceName()`.
    fn device_name(&self, device_index: usize) -> Option<String> {
        let name = match state().sensors.get(device_index).map(|s| s.sensor_type) {
            Some(SensorType::Accel) => "Accelerometer",
            Some(SensorType::Gyro) => "Gyro",
            _ => "Unknown",
        };
        Some(name.to_owned())
    }

    /// Translation of `SDL_COREMOTION_SensorGetDeviceType()`.
    fn device_type(&self, device_index: usize) -> SensorType {
        state()
            .sensors
            .get(device_index)
            .map_or(SensorType::Invalid, |s| s.sensor_type)
    }

    /// Translation of `SDL_COREMOTION_SensorGetDeviceNonPortableType()`.
    fn device_non_portable_type(&self, device_index: usize) -> i32 {
        state()
            .sensors
            .get(device_index)
            .map_or(-1, |s| s.sensor_type as i32)
    }

    /// Translation of `SDL_COREMOTION_SensorGetDeviceInstanceID()`.
    fn device_instance_id(&self, device_index: usize) -> SensorID {
        state()
            .sensors
            .get(device_index)
            // (-1 as an SDL_SensorID)
            .map_or(SensorID::MAX, |s| s.instance_id)
    }

    /// Translation of `SDL_COREMOTION_SensorOpen()`.
    fn open(&self, sensor: &mut SensorData, _device_index: usize) -> Result<()> {
        sensor.hwdata = Some(Box::new(SensorHwdata { data: [0.0; 3] }));

        let manager = motion_manager();
        // SAFETY: CMMotionManager's methods.
        unsafe {
            match sensor.sensor_type {
                SensorType::Accel => {
                    let () = msg_send![manager, startAccelerometerUpdates];
                }
                SensorType::Gyro => {
                    let () = msg_send![manager, startGyroUpdates];
                }
                _ => {}
            }
        }
        Ok(())
    }

    /// Translation of `SDL_COREMOTION_SensorUpdate()`.
    fn update(&self, sensor: SensorID) {
        let timestamp = crate::timer::ticks_ns();

        let Some(sensor_type) = super::with_sensor(sensor, |s| s.sensor_type) else {
            return;
        };
        let manager = motion_manager();
        // SAFETY: CMMotionManager's properties (nil before the first
        // update), and the data objects' structure properties.
        let data = unsafe {
            match sensor_type {
                SensorType::Accel => {
                    let accelerometer_data: Id = msg_send![manager, accelerometerData];
                    if accelerometer_data.is_null() {
                        return;
                    }
                    let acceleration: CMAcceleration = msg_send![accelerometer_data, acceleration];
                    [
                        (-acceleration.x * f64::from(STANDARD_GRAVITY)) as f32,
                        (-acceleration.y * f64::from(STANDARD_GRAVITY)) as f32,
                        (-acceleration.z * f64::from(STANDARD_GRAVITY)) as f32,
                    ]
                }
                SensorType::Gyro => {
                    let gyro_data: Id = msg_send![manager, gyroData];
                    if gyro_data.is_null() {
                        return;
                    }
                    let rotation_rate: CMRotationRate = msg_send![gyro_data, rotationRate];
                    [
                        rotation_rate.x as f32,
                        rotation_rate.y as f32,
                        rotation_rate.z as f32,
                    ]
                }
                _ => return,
            }
        };

        // (SDL_memcmp() of the floats: compared bit for bit)
        let changed = super::with_sensor(sensor, |s| {
            let hwdata = s
                .hwdata
                .as_mut()
                .and_then(|h| h.downcast_mut::<SensorHwdata>())?;
            let changed = data.map(f32::to_bits) != hwdata.data.map(f32::to_bits);
            if changed {
                hwdata.data = data;
            }
            Some(changed)
        })
        .flatten();
        if changed == Some(true) {
            send_sensor_update(Duration::from_nanos(timestamp), sensor, timestamp, &data);
        }
    }

    /// Translation of `SDL_COREMOTION_SensorClose()`.
    fn close(&self, sensor: &mut SensorData) {
        if sensor.hwdata.is_some() {
            let manager = motion_manager();
            // SAFETY: CMMotionManager's methods.
            unsafe {
                match sensor.sensor_type {
                    SensorType::Accel => {
                        let () = msg_send![manager, stopAccelerometerUpdates];
                    }
                    SensorType::Gyro => {
                        let () = msg_send![manager, stopGyroUpdates];
                    }
                    _ => {}
                }
            }
            sensor.hwdata = None;
        }
    }

    /// Translation of `SDL_COREMOTION_SensorQuit()`.
    fn quit(&self) {}
}
