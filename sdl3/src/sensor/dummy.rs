// Rust translation of src/sensor/dummy/SDL_dummysensor.c from Simple
// DirectMedia Layer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! The dummy sensor driver: no sensors.

use super::{SensorData, SensorDriver, SensorType};
use crate::error::{Error, Result};
use crate::events::SensorID;

/// Translation of `SDL_DUMMY_SensorDriver`.
pub(super) struct DummySensorDriver;

pub(super) static DUMMY_SENSOR_DRIVER: DummySensorDriver = DummySensorDriver;

impl SensorDriver for DummySensorDriver {
    fn init(&self) -> Result<()> {
        Ok(())
    }

    fn count(&self) -> usize {
        0
    }

    fn detect(&self) {}

    fn device_name(&self, _device_index: usize) -> Option<String> {
        None
    }

    fn device_type(&self, _device_index: usize) -> SensorType {
        SensorType::Invalid
    }

    fn device_non_portable_type(&self, _device_index: usize) -> i32 {
        -1
    }

    fn device_instance_id(&self, _device_index: usize) -> SensorID {
        // (-1 as an SDL_SensorID)
        SensorID::MAX
    }

    fn open(&self, _sensor: &mut SensorData, _device_index: usize) -> Result<()> {
        Err(Error::unsupported())
    }

    fn update(&self, _sensor: SensorID) {}

    fn close(&self, _sensor: &mut SensorData) {}

    fn quit(&self) {}
}
