// Rust translation of src/sensor/SDL_sensor.c, SDL_syssensor.h, SDL_sensor_c.h
// and include/SDL3/SDL_sensor.h from Simple DirectMedia Layer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! Sensors: accelerometers and gyroscopes.
//!
//! A [`Sensor`] is an open sensor; it is closed when the last handle to it
//! is dropped (upstream reference counts `SDL_OpenSensor()` calls). The
//! sensor drivers are platform backends; so far only the dummy driver,
//! which reports no sensors, exists.
//!
//! Sensor data is in SI units: m/s² for accelerometers (a device at rest
//! reports [`STANDARD_GRAVITY`] away from the center of the earth) and
//! radians per second for gyroscopes, with the axes of the device in its
//! natural orientation (see `SDL_sensor.h`).

mod dummy;

use std::cell::RefCell;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::Duration;

use crate::error::{Error, Result};
use crate::events::queue::EVENT_LOCK;
use crate::events::{Event, EventType, SensorEvent, SensorID};
use crate::init::{self, InitFlags};
use crate::properties::Properties;
use crate::thread::{RawMutexGuard, ReentrantMutex};

/// A sensor at rest reports this acceleration (m/s²) away from the center of
/// the earth. Translation of `SDL_STANDARD_GRAVITY`.
pub const STANDARD_GRAVITY: f32 = 9.80665;

/// The different sensors defined by SDL. Translation of `SDL_SensorType`.
///
/// Additional sensors may be available, using platform dependent semantics
/// (see [`sensor_non_portable_type_for_id`]).
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default)]
pub enum SensorType {
    /// Returned for an invalid sensor
    #[default]
    Invalid = -1,
    /// Unknown sensor type
    Unknown,
    /// Accelerometer
    Accel,
    /// Gyroscope
    Gyro,
    /// Accelerometer for left Joy-Con controller and Wii nunchuk
    AccelL,
    /// Gyroscope for left Joy-Con controller
    GyroL,
    /// Accelerometer for right Joy-Con controller
    AccelR,
    /// Gyroscope for right Joy-Con controller
    GyroR,
}

impl SensorType {
    /// The number of sensor types (`SDL_SENSOR_COUNT`).
    pub const COUNT: usize = 7;

    /// The sensor type of a raw value, [`SensorType::Invalid`] for an unknown one.
    pub fn from_i32(v: i32) -> SensorType {
        match v {
            0 => SensorType::Unknown,
            1 => SensorType::Accel,
            2 => SensorType::Gyro,
            3 => SensorType::AccelL,
            4 => SensorType::GyroL,
            5 => SensorType::AccelR,
            6 => SensorType::GyroR,
            _ => SensorType::Invalid,
        }
    }
}

/// The state of an open sensor. Translation of `struct SDL_Sensor`.
pub(crate) struct SensorData {
    /// Device instance, monotonically increasing from 0
    pub(crate) instance_id: SensorID,
    /// Sensor name - system dependent
    pub(crate) name: Option<String>,
    /// Type of the sensor
    pub(crate) sensor_type: SensorType,
    /// Platform dependent type of the sensor
    pub(crate) non_portable_type: i32,
    /// The current state of the sensor
    pub(crate) data: [f32; 16],
    /// The index of the driver in the driver list
    driver: usize,
    /// Driver dependent information
    pub(crate) hwdata: Option<Box<dyn std::any::Any + Send>>,
    props: Option<Properties>,
    /// Reference count for multiple opens
    ref_count: i32,
    /// Identifies this open sensor (handles of an earlier open of the same
    /// instance id don't refer to it).
    serial: u64,
}

impl SensorData {
    /// A sensor about to be opened by a driver.
    pub(crate) fn new(
        instance_id: SensorID,
        sensor_type: SensorType,
        non_portable_type: i32,
    ) -> SensorData {
        SensorData {
            instance_id,
            name: None,
            sensor_type,
            non_portable_type,
            data: [0.0; 16],
            driver: 0,
            hwdata: None,
            props: None,
            ref_count: 0,
            serial: 0,
        }
    }
}

/// The functions of a sensor backend. Translation of `SDL_SensorDriver`.
///
/// The functions take `&self`: a driver keeps its state behind its own
/// interior mutability, since the update function delivers events whose
/// watchers may call back into the sensor API.
pub(crate) trait SensorDriver: Send + Sync {
    /// Scan the system for sensors. Sensor 0 should be the system default
    /// sensor. Fails on an unrecoverable fatal error.
    fn init(&self) -> Result<()>;

    /// The number of sensors available right now.
    fn count(&self) -> usize;

    /// Check to see if the available sensors have changed.
    fn detect(&self);

    /// The device-dependent name of a sensor.
    fn device_name(&self, device_index: usize) -> Option<String>;

    /// The type of a sensor.
    fn device_type(&self, device_index: usize) -> SensorType;

    /// The platform dependent type of a sensor.
    fn device_non_portable_type(&self, device_index: usize) -> i32;

    /// The current instance id of the sensor located at `device_index`.
    fn device_instance_id(&self, device_index: usize) -> SensorID;

    /// Open a sensor for use.
    fn open(&self, sensor: &mut SensorData, device_index: usize) -> Result<()>;

    /// Update the state of a sensor - called as a device poll. This
    /// function shouldn't update the sensor structure directly, but instead
    /// should call [`send_sensor_update`] to deliver events and update
    /// sensor device state.
    fn update(&self, sensor: SensorID);

    /// Close a sensor after use.
    fn close(&self, sensor: &mut SensorData);

    /// Perform any system-specific sensor related cleanup.
    fn quit(&self);
}

/// The available sensor drivers. Translation of `SDL_sensor_drivers`.
static SENSOR_DRIVERS: &[&dyn SensorDriver] = &[&dummy::DUMMY_SENSOR_DRIVER];

/// Translation of `SDL_sensors_locked`.
static SENSORS_LOCKED: std::sync::atomic::AtomicI32 = std::sync::atomic::AtomicI32::new(0);
/// Translation of `SDL_sensors_initialized`.
static SENSORS_INITIALIZED: AtomicBool = AtomicBool::new(false);
static NEXT_SERIAL: AtomicU64 = AtomicU64::new(1);

/// Translation of `SDL_sensors` (guarded by `SDL_event_lock` upstream): the
/// open sensors, most recently opened first. The `RefCell` borrow is never
/// held across a driver call or an event push.
static SENSORS: ReentrantMutex<RefCell<Vec<SensorData>>> =
    ReentrantMutex::new(RefCell::new(Vec::new()));

/// The sensor lock: `SDL_event_lock`, counted.
pub(crate) struct SensorLock {
    _guard: RawMutexGuard<'static>,
}

impl Drop for SensorLock {
    fn drop(&mut self) {
        // Translation of `SDL_UnlockSensors()`.
        SENSORS_LOCKED.fetch_sub(1, Ordering::Relaxed);
    }
}

/// Translation of `SDL_LockSensors()`; the guard unlocks on drop.
pub(crate) fn lock_sensors() -> SensorLock {
    let guard = EVENT_LOCK.guard();
    SENSORS_LOCKED.fetch_add(1, Ordering::Relaxed);
    SensorLock { _guard: guard }
}

/// Translation of `SDL_SensorsLocked()`.
#[allow(dead_code)] // (used by the sensor drivers)
pub(crate) fn sensors_locked() -> bool {
    SENSORS_LOCKED.load(Ordering::Relaxed) > 0
}

/// Translation of `SDL_AssertSensorsLocked()`.
#[allow(dead_code)] // (used by the sensor drivers)
fn assert_sensors_locked() {
    crate::sdl_assert!(sensors_locked());
}

/// Run `f` on the open sensors (the lock must be held).
fn with_sensors<R>(f: impl FnOnce(&mut Vec<SensorData>) -> R) -> R {
    let guard = SENSORS.lock();
    let mut sensors = guard.borrow_mut();
    f(&mut sensors)
}

/// Run `f` on the open sensor `instance_id` (the lock must be held).
pub(crate) fn with_sensor<R>(
    instance_id: SensorID,
    f: impl FnOnce(&mut SensorData) -> R,
) -> Option<R> {
    with_sensors(|s| s.iter_mut().find(|s| s.instance_id == instance_id).map(f))
}

/// Translation of `SDL_SensorsInitialized()`.
#[allow(dead_code)] // (used by the sensor drivers)
pub(crate) fn sensors_initialized() -> bool {
    SENSORS_INITIALIZED.load(Ordering::Relaxed)
}

/// Translation of `SDL_InitSensors()`.
pub(crate) fn init_sensors() -> Result<()> {
    init::init_subsystem(InitFlags::EVENTS)?;

    let status = {
        let _lock = lock_sensors();

        SENSORS_INITIALIZED.store(true, Ordering::Relaxed);

        let mut status = Err(Error::new("No sensor drivers available"));
        for driver in SENSOR_DRIVERS {
            if let Ok(()) = driver.init() {
                status = Ok(());
            }
        }
        status
    };

    if status.is_err() {
        quit_sensors();
    }

    status
}

/// Whether there are any sensors opened by the application.
/// Translation of `SDL_SensorsOpened()`.
pub(crate) fn sensors_opened() -> bool {
    let _lock = lock_sensors();
    with_sensors(|s| !s.is_empty())
}

/// The currently connected sensors. Translation of `SDL_GetSensors()`.
pub fn sensors() -> Vec<SensorID> {
    let _lock = lock_sensors();

    let total_sensors: usize = SENSOR_DRIVERS.iter().map(|d| d.count()).sum();
    let mut sensors = Vec::with_capacity(total_sensors);
    for driver in SENSOR_DRIVERS {
        let num_sensors = driver.count();
        for device_index in 0..num_sensors {
            crate::sdl_assert!(sensors.len() < total_sensors);
            let id = driver.device_instance_id(device_index);
            crate::sdl_assert!(id > 0);
            sensors.push(id);
        }
    }
    crate::sdl_assert!(sensors.len() == total_sensors);
    sensors
}

/// The driver and device index of a sensor instance ID. This should be
/// called while the sensor lock is held, to prevent another thread from
/// updating the list. Translation of `SDL_GetDriverAndSensorIndex()`.
fn driver_and_sensor_index(instance_id: SensorID) -> Result<(usize, usize)> {
    if instance_id > 0 {
        for (i, driver) in SENSOR_DRIVERS.iter().enumerate() {
            let num_sensors = driver.count();
            for device_index in 0..num_sensors {
                if driver.device_instance_id(device_index) == instance_id {
                    return Ok((i, device_index));
                }
            }
        }
    }
    Err(Error::new(format!("Sensor {instance_id} not found")))
}

/// The implementation dependent name of a sensor.
/// Translation of `SDL_GetSensorNameForID()`.
pub fn sensor_name_for_id(instance_id: SensorID) -> Result<Option<String>> {
    let _lock = lock_sensors();
    let (driver, device_index) = driver_and_sensor_index(instance_id)?;
    Ok(SENSOR_DRIVERS[driver].device_name(device_index))
}

/// The type of a sensor. Translation of `SDL_GetSensorTypeForID()`.
pub fn sensor_type_for_id(instance_id: SensorID) -> Result<SensorType> {
    let _lock = lock_sensors();
    let (driver, device_index) = driver_and_sensor_index(instance_id)?;
    Ok(SENSOR_DRIVERS[driver].device_type(device_index))
}

/// The platform dependent type of a sensor.
/// Translation of `SDL_GetSensorNonPortableTypeForID()`.
pub fn sensor_non_portable_type_for_id(instance_id: SensorID) -> Result<i32> {
    let _lock = lock_sensors();
    let (driver, device_index) = driver_and_sensor_index(instance_id)?;
    Ok(SENSOR_DRIVERS[driver].device_non_portable_type(device_index))
}

/// An open sensor. Translation of `SDL_Sensor *`; dropping the last handle
/// closes the sensor (`SDL_CloseSensor()`).
#[derive(Debug)]
pub struct Sensor {
    instance_id: SensorID,
    serial: u64,
}

impl Sensor {
    /// Open a sensor for use. Opening a sensor that is already open adds a
    /// reference to it. Translation of `SDL_OpenSensor()`.
    pub fn open(instance_id: SensorID) -> Result<Sensor> {
        let _lock = lock_sensors();

        let (driver_index, device_index) = driver_and_sensor_index(instance_id)?;
        let driver = SENSOR_DRIVERS[driver_index];

        /* If the sensor is already open, return it
         * it is important that we have a single sensor * for each instance id
         */
        if let Some(serial) = with_sensor(instance_id, |s| {
            s.ref_count += 1;
            s.serial
        }) {
            return Ok(Sensor {
                instance_id,
                serial,
            });
        }

        // Create and initialize the sensor
        let mut sensor = SensorData::new(
            instance_id,
            driver.device_type(device_index),
            driver.device_non_portable_type(device_index),
        );
        sensor.driver = driver_index;

        driver.open(&mut sensor, device_index)?;

        sensor.name = driver.device_name(device_index);

        // Add sensor to list
        sensor.ref_count += 1;
        let serial = NEXT_SERIAL.fetch_add(1, Ordering::Relaxed);
        sensor.serial = serial;
        // Link the sensor in the list
        with_sensors(|s| s.insert(0, sensor));

        driver.update(instance_id);

        Ok(Sensor {
            instance_id,
            serial,
        })
    }

    /// Another handle to an open sensor (adding a reference to it), or
    /// `None` if it isn't open. Translation of `SDL_GetSensorFromID()`.
    pub fn from_id(instance_id: SensorID) -> Option<Sensor> {
        let _lock = lock_sensors();
        with_sensor(instance_id, |s| {
            s.ref_count += 1;
            Sensor {
                instance_id,
                serial: s.serial,
            }
        })
    }

    /// Run `f` on this sensor's state (`CHECK_SENSOR_MAGIC`).
    fn with<R>(&self, f: impl FnOnce(&mut SensorData) -> R) -> Result<R> {
        let _lock = lock_sensors();
        with_sensors(|s| {
            s.iter_mut()
                .find(|s| s.instance_id == self.instance_id && s.serial == self.serial)
                .map(f)
        })
        .ok_or_else(|| Error::invalid_param("sensor"))
    }

    /// The properties of the sensor. Translation of `SDL_GetSensorProperties()`.
    pub fn properties(&self) -> Result<Properties> {
        self.with(|s| s.props.get_or_insert_with(Properties::new).clone())
    }

    /// The implementation dependent name of the sensor.
    /// Translation of `SDL_GetSensorName()`.
    pub fn name(&self) -> Result<Option<String>> {
        self.with(|s| s.name.clone())
    }

    /// The type of the sensor. Translation of `SDL_GetSensorType()`.
    pub fn sensor_type(&self) -> SensorType {
        self.with(|s| s.sensor_type).unwrap_or(SensorType::Invalid)
    }

    /// The platform dependent type of the sensor.
    /// Translation of `SDL_GetSensorNonPortableType()`.
    pub fn non_portable_type(&self) -> i32 {
        self.with(|s| s.non_portable_type).unwrap_or(-1)
    }

    /// The instance ID of the sensor. Translation of `SDL_GetSensorID()`.
    pub fn id(&self) -> SensorID {
        self.instance_id
    }

    /// The current state of the sensor: as many values as `data` holds, up
    /// to the sensor's 16. Translation of `SDL_GetSensorData()`.
    pub fn data(&self, data: &mut [f32]) -> Result<()> {
        self.with(|s| {
            let num_values = data.len().min(s.data.len());
            data[..num_values].copy_from_slice(&s.data[..num_values]);
        })
    }

    /// Poll the sensor's driver, as [`update_sensors`] does for all of
    /// them. Translation of `SDL_UpdateSensor()`.
    #[allow(dead_code)] // (used by the sensor drivers)
    pub(crate) fn update(&self) {
        let _lock = lock_sensors();
        if let Ok(driver) = self.with(|s| s.driver) {
            SENSOR_DRIVERS[driver].update(self.instance_id);
        }
    }
}

impl Drop for Sensor {
    fn drop(&mut self) {
        close_sensor(self.instance_id, self.serial);
    }
}

/// Close a sensor previously opened with [`Sensor::open`].
/// Translation of `SDL_CloseSensor()`.
fn close_sensor(instance_id: SensorID, serial: u64) {
    let _lock = lock_sensors();

    // First decrement ref count
    let remaining = with_sensors(|s| {
        s.iter_mut()
            .find(|s| s.instance_id == instance_id && s.serial == serial)
            .map(|s| {
                s.ref_count -= 1;
                s.ref_count
            })
    });
    match remaining {
        Some(n) if n <= 0 => {}
        _ => return,
    }

    // (unlink this entry)
    let Some(mut sensor) = with_sensors(|s| {
        let i = s
            .iter()
            .position(|s| s.instance_id == instance_id && s.serial == serial)?;
        Some(s.remove(i))
    }) else {
        return;
    };

    sensor.props = None;

    SENSOR_DRIVERS[sensor.driver].close(&mut sensor);
    sensor.hwdata = None;
}

/// Translation of `SDL_QuitSensors()`.
pub(crate) fn quit_sensors() {
    let _lock = lock_sensors();

    // Stop the event polling
    while let Some((id, serial)) = with_sensors(|s| {
        s.first_mut().map(|s| {
            s.ref_count = 1;
            (s.instance_id, s.serial)
        })
    }) {
        close_sensor(id, serial);
    }

    // Quit the sensor setup
    for driver in SENSOR_DRIVERS {
        driver.quit();
    }

    init::quit_subsystem(InitFlags::EVENTS);

    SENSORS_INITIALIZED.store(false, Ordering::Relaxed);
}

/// Deliver new sensor data: update the sensor's state and post a sensor
/// event (and the gamepad sensor events of gamepads using the sensor).
/// Called by drivers with the sensor lock held.
/// Translation of `SDL_SendSensorUpdate()`.
#[allow(dead_code)] // (used by the sensor drivers)
pub(crate) fn send_sensor_update(
    timestamp: Duration,
    sensor: SensorID,
    sensor_timestamp: u64,
    data: &[f32],
) {
    assert_sensors_locked();

    // Allow duplicate events, for things like steps and heartbeats

    // Update internal sensor state
    with_sensor(sensor, |s| {
        let num_values = data.len().min(s.data.len());
        s.data[..num_values].copy_from_slice(&data[..num_values]);
    });

    // Post the event, if desired
    if crate::events::queue::event_enabled(EventType::SENSOR_UPDATE) {
        let mut event = SensorEvent {
            timestamp,
            which: sensor,
            data: [0.0; 6],
            sensor_timestamp,
        };
        let num_values = data.len().min(event.data.len());
        event.data[..num_values].copy_from_slice(&data[..num_values]);
        let _ = crate::events::queue::push(Event::Sensor(event));
    }

    crate::joystick::gamepad::gamepad_sensor_watcher(timestamp, sensor, sensor_timestamp, data);
}

/// Poll the sensor drivers for new data and changes in the available
/// sensors (done by the event loop unless `SDL_HINT_AUTO_UPDATE_SENSORS` is
/// off). Translation of `SDL_UpdateSensors()`.
pub fn update_sensors() {
    if init::was_init(InitFlags::SENSOR).is_empty() {
        return;
    }

    let _lock = lock_sensors();

    let open: Vec<(SensorID, usize)> =
        with_sensors(|s| s.iter().map(|s| (s.instance_id, s.driver)).collect());
    for (id, driver) in open {
        SENSOR_DRIVERS[driver].update(id);
    }

    /* this needs to happen AFTER walking the sensor list above, so that any
      dangling hardware data from removed devices can be free'd
    */
    for driver in SENSOR_DRIVERS {
        driver.detect();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dummy_driver_has_no_sensors() {
        // (SENSOR implies EVENTS, whose events thread is the thread that
        // initializes it: hold the lock so no other test's subsystems run
        // on the wrong thread meanwhile)
        let _l = crate::test_support::test_lock();
        init::set_main_ready();
        init::init_subsystem(InitFlags::SENSOR).unwrap();
        assert!(sensors().is_empty());
        assert_eq!(
            sensor_name_for_id(1).unwrap_err().to_string(),
            "Sensor 1 not found"
        );
        assert_eq!(
            sensor_type_for_id(0).unwrap_err().to_string(),
            "Sensor 0 not found"
        );
        assert!(Sensor::open(3).is_err());
        assert!(Sensor::from_id(3).is_none());
        assert!(!sensors_opened());
        update_sensors();
        init::quit_subsystem(InitFlags::SENSOR);
    }

    #[test]
    fn sensor_types() {
        assert_eq!(SensorType::from_i32(2), SensorType::Gyro);
        assert_eq!(SensorType::from_i32(7), SensorType::Invalid);
        assert_eq!(SensorType::GyroR as i32, 6);
    }
}
