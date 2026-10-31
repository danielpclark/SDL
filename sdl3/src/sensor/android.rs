// Rust translation of src/sensor/android/SDL_androidsensor.c and
// SDL_androidsensor.h from Simple DirectMedia Layer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! The Android sensor driver (`SDL_SENSOR_ANDROID`): the device's sensors
//! through the NDK's sensor manager (`<android/sensor.h>`, in libandroid,
//! linked as upstream links it), not Java's.
//!
//! The driver lists the sensor manager's sensors when it initializes (all
//! of them: an accelerometer and a gyroscope are SDL's types, the others
//! [`SensorType::Unknown`] with Android's sensor type as their
//! non-portable one) and starts a thread, "Sensors", whose looper
//! (`ALooper_prepare()`) the event queues of the open sensors are attached
//! to. Opening a sensor makes its event queue, enables the sensor and asks
//! for 60 events a second (or the sensor's slowest rate when that is
//! slower); the thread waits on its looper, and once an event queue has
//! events it sends them, with the sensor lock held, each with all 16 of
//! the event's values. Updating a sensor does nothing.
//!
//! It works without a Java VM too (a spawned program): the sensor manager
//! is the NDK's.
//!
//! Note (upstream), kept as upstream does it:
//!
//! * the sensor list isn't read again (`FIXME: Is the sensor list
//!   dynamic?`), and detecting does nothing;
//! * an event's timestamps (SDL's and the sensor's) are the time the
//!   thread started waiting for events, not the event's own;
//! * a sensor whose event queue couldn't be made or enabled is still
//!   marked as the open sensor's (until another open of it succeeds).

use std::ffi::{c_int, c_void, CStr};
use std::ptr;
use std::sync::atomic::{AtomicBool, AtomicPtr, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use super::{
    assert_sensors_locked, lock_sensors, send_sensor_update, with_sensors_unlocked, SensorData,
    SensorDriver, SensorType,
};
use crate::core::android::ndk::{
    ALooper, ALooper_pollOnce, ALooper_prepare, ALooper_wake, ASensorEvent, ASensorEventQueue,
    ASensorEventQueue_disableSensor, ASensorEventQueue_enableSensor, ASensorEventQueue_getEvents,
    ASensorEventQueue_setEventRate, ASensorList, ASensorManager, ASensorManager_createEventQueue,
    ASensorManager_destroyEventQueue, ASensorManager_getInstance, ASensorManager_getSensorList,
    ASensorRef, ASensor_getMinDelay, ASensor_getName, ASensor_getType,
    ALOOPER_PREPARE_ALLOW_NON_CALLBACKS,
};
use crate::error::{Error, Result};
use crate::events::SensorID;
use crate::thread::{Semaphore, Thread, ThreadPriority};

// (`#ifndef LOOPER_ID_USER`: android_native_app_glue's)
const LOOPER_ID_USER: c_int = 3;

/// Translation of `SDL_AndroidSensor`.
struct AndroidSensor {
    asensor: ASensorRef,
    instance_id: SensorID,
    event_queue: *mut ASensorEventQueue,
    /// The open sensor (upstream's `SDL_Sensor *`), by its instance ID.
    sensor: Option<SensorID>,
}

// SAFETY: the sensor manager's sensors live as long as the process, and an
// event queue is used with the sensor lock held.
unsafe impl Send for AndroidSensor {}

/// Translation of `SDL_AndroidSensorThreadContext` (its `running` is
/// [`RUNNING`]).
#[derive(Default)]
struct AndroidSensorThreadContext {
    thread: Option<Thread>,
    sem: Option<Arc<Semaphore>>,
}

/// The driver's state: `SDL_sensors` (whose length is
/// `SDL_sensors_count`, guarded by the sensor lock upstream, and here by
/// this mutex too, taken after it) and `SDL_sensor_thread_context`.
struct AndroidSensorState {
    sensors: Vec<AndroidSensor>,
    thread_context: AndroidSensorThreadContext,
}

static STATE: Mutex<AndroidSensorState> = Mutex::new(AndroidSensorState {
    sensors: Vec::new(),
    thread_context: AndroidSensorThreadContext {
        thread: None,
        sem: None,
    },
});

/// `SDL_sensor_manager`.
static SENSOR_MANAGER: AtomicPtr<ASensorManager> = AtomicPtr::new(ptr::null_mut());
/// `SDL_sensor_looper`.
static SENSOR_LOOPER: AtomicPtr<ALooper> = AtomicPtr::new(ptr::null_mut());
/// `SDL_sensor_thread_context.running`.
static RUNNING: AtomicBool = AtomicBool::new(false);

fn state() -> MutexGuard<'static, AndroidSensorState> {
    STATE.lock().unwrap_or_else(|e| e.into_inner())
}

/// Translation of `SDL_ANDROID_SensorThread()`.
fn sensor_thread(sem: Arc<Semaphore>) -> i32 {
    let _ = crate::thread::set_current_thread_priority(ThreadPriority::High);

    // SAFETY: plain call; this thread's looper.
    let looper = unsafe { ALooper_prepare(ALOOPER_PREPARE_ALLOW_NON_CALLBACKS) };
    SENSOR_LOOPER.store(looper, Ordering::Release);
    sem.signal();

    while RUNNING.load(Ordering::Acquire) {
        let timestamp = crate::timer::ticks_ns();

        let mut events: c_int = 0;
        let mut source: *mut c_void = ptr::null_mut();
        // SAFETY: this thread's looper; the out-parameters are valid.
        let ident = unsafe { ALooper_pollOnce(-1, ptr::null_mut(), &mut events, &mut source) };
        if ident == LOOPER_ID_USER {
            let _lock = lock_sensors();
            let queues: Vec<(*mut ASensorEventQueue, Option<SensorID>)> = state()
                .sensors
                .iter()
                .map(|s| (s.event_queue, s.sensor))
                .collect();
            for (event_queue, sensor) in queues {
                if event_queue.is_null() {
                    continue;
                }

                // SAFETY: zero is a valid ASensorEvent.
                let mut event: ASensorEvent = unsafe { std::mem::zeroed() };
                // SAFETY: an open sensor's event queue (destroyed only with
                // the sensor lock held), and room for one event.
                while unsafe { ASensorEventQueue_getEvents(event_queue, &mut event, 1) } > 0 {
                    if let Some(sensor) = sensor {
                        send_sensor_update(
                            Duration::from_nanos(timestamp),
                            sensor,
                            timestamp,
                            &event.data,
                        );
                    }
                }
            }
        }
    }

    SENSOR_LOOPER.store(ptr::null_mut(), Ordering::Release);

    0
}

/// Translation of `SDL_ANDROID_StopSensorThread()`.
fn stop_sensor_thread() {
    RUNNING.store(false, Ordering::Release);

    let thread = state().thread_context.thread.take();
    if let Some(thread) = thread {
        let looper = SENSOR_LOOPER.load(Ordering::Acquire);
        if !looper.is_null() {
            // SAFETY: the sensor thread's looper, which lives while the
            // thread runs.
            unsafe { ALooper_wake(looper) };
        }
        let _result = thread.wait();
    }

    state().thread_context.sem = None;
}

/// Translation of `SDL_ANDROID_StartSensorThread()`.
fn start_sensor_thread() -> Result<()> {
    let sem = Arc::new(Semaphore::new(0));
    state().thread_context.sem = Some(sem.clone());

    RUNNING.store(true, Ordering::Release);
    let thread_sem = sem.clone();
    match Thread::spawn("Sensors", move || sensor_thread(thread_sem)) {
        Ok(thread) => state().thread_context.thread = Some(thread),
        Err(e) => {
            stop_sensor_thread();
            return Err(e);
        }
    }

    // Wait for the sensor thread to start
    sem.wait();

    Ok(())
}

/// The name of a sensor of the sensor manager.
fn sensor_name(asensor: ASensorRef) -> Option<String> {
    // SAFETY: one of the sensor manager's sensors.
    let name = unsafe { ASensor_getName(asensor) };
    if name.is_null() {
        return None;
    }
    // SAFETY: a NUL-terminated string the sensor keeps.
    Some(
        unsafe { CStr::from_ptr(name) }
            .to_string_lossy()
            .into_owned(),
    )
}

/// Translation of `SDL_ANDROID_SensorDriver`.
pub(super) struct AndroidSensorDriver;

pub(super) static ANDROID_SENSOR_DRIVER: AndroidSensorDriver = AndroidSensorDriver;

impl SensorDriver for AndroidSensorDriver {
    /// Translation of `SDL_ANDROID_SensorInit()`.
    fn init(&self) -> Result<()> {
        // SAFETY: plain call.
        let manager = unsafe { ASensorManager_getInstance() };
        SENSOR_MANAGER.store(manager, Ordering::Release);
        if manager.is_null() {
            return Err(Error::new("Couldn't create sensor manager"));
        }

        // FIXME: Is the sensor list dynamic?
        let mut sensors: ASensorList = ptr::null();
        // SAFETY: the sensor manager; the list stays the manager's.
        let sensors_count = unsafe { ASensorManager_getSensorList(manager, &mut sensors) };
        if sensors_count > 0 && !sensors.is_null() {
            // SAFETY: the manager's list holds `sensors_count` sensors.
            let list = unsafe { std::slice::from_raw_parts(sensors, sensors_count as usize) };
            state().sensors = list
                .iter()
                .map(|&asensor| AndroidSensor {
                    asensor,
                    instance_id: crate::utils::next_object_id(),
                    event_queue: ptr::null_mut(),
                    sensor: None,
                })
                .collect();
        }

        start_sensor_thread()
    }

    /// Translation of `SDL_ANDROID_SensorGetCount()`.
    fn count(&self) -> usize {
        state().sensors.len()
    }

    /// Translation of `SDL_ANDROID_SensorDetect()`.
    fn detect(&self) {}

    /// Translation of `SDL_ANDROID_SensorGetDeviceName()`.
    fn device_name(&self, device_index: usize) -> Option<String> {
        let asensor = state().sensors.get(device_index)?.asensor;
        sensor_name(asensor)
    }

    /// Translation of `SDL_ANDROID_SensorGetDeviceType()`.
    fn device_type(&self, device_index: usize) -> SensorType {
        let Some(asensor) = state().sensors.get(device_index).map(|s| s.asensor) else {
            return SensorType::Invalid;
        };
        // SAFETY: one of the sensor manager's sensors.
        match unsafe { ASensor_getType(asensor) } {
            0x00000001 => SensorType::Accel,
            0x00000004 => SensorType::Gyro,
            _ => SensorType::Unknown,
        }
    }

    /// Translation of `SDL_ANDROID_SensorGetDeviceNonPortableType()`.
    fn device_non_portable_type(&self, device_index: usize) -> i32 {
        let Some(asensor) = state().sensors.get(device_index).map(|s| s.asensor) else {
            return -1;
        };
        // SAFETY: one of the sensor manager's sensors.
        unsafe { ASensor_getType(asensor) }
    }

    /// Translation of `SDL_ANDROID_SensorGetDeviceInstanceID()`.
    fn device_instance_id(&self, device_index: usize) -> SensorID {
        state()
            .sensors
            .get(device_index)
            // (-1 as an SDL_SensorID)
            .map_or(SensorID::MAX, |s| s.instance_id)
    }

    /// Translation of `SDL_ANDROID_SensorOpen()`.
    fn open(&self, sensor: &mut SensorData, device_index: usize) -> Result<()> {
        let manager = SENSOR_MANAGER.load(Ordering::Acquire);
        let looper = SENSOR_LOOPER.load(Ordering::Acquire);

        let _lock = lock_sensors();
        let mut state = state();
        let Some(s) = state.sensors.get_mut(device_index) else {
            return Err(Error::invalid_param("device_index"));
        };
        s.sensor = Some(sensor.instance_id);
        // SAFETY: the sensor manager, and the sensor thread's looper.
        s.event_queue = unsafe {
            ASensorManager_createEventQueue(manager, looper, LOOPER_ID_USER, None, ptr::null_mut())
        };
        if s.event_queue.is_null() {
            return Err(Error::new("Couldn't create sensor event queue"));
        }

        // SAFETY: the new event queue and one of the manager's sensors.
        if unsafe { ASensorEventQueue_enableSensor(s.event_queue, s.asensor) } < 0 {
            // SAFETY: the manager's event queue, destroyed once.
            unsafe { ASensorManager_destroyEventQueue(manager, s.event_queue) };
            s.event_queue = ptr::null_mut();
            return Err(Error::new("Couldn't enable sensor"));
        }

        // Use 60 Hz update rate if possible
        // FIXME: Maybe add a hint for this?
        let mut delay_us = 1000000 / 60;
        // SAFETY: one of the manager's sensors.
        let min_delay_us = unsafe { ASensor_getMinDelay(s.asensor) };
        if delay_us < min_delay_us {
            delay_us = min_delay_us;
        }
        // SAFETY: the event queue, and its enabled sensor.
        unsafe { ASensorEventQueue_setEventRate(s.event_queue, s.asensor, delay_us) };

        Ok(())
    }

    /// Translation of `SDL_ANDROID_SensorUpdate()`.
    fn update(&self, _sensor: SensorID) {}

    /// Translation of `SDL_ANDROID_SensorClose()`.
    fn close(&self, sensor: &mut SensorData) {
        let manager = SENSOR_MANAGER.load(Ordering::Acquire);
        let _lock = lock_sensors();
        let mut state = state();
        if let Some(s) = state
            .sensors
            .iter_mut()
            .find(|s| s.sensor == Some(sensor.instance_id))
        {
            // SAFETY: the sensor's event queue (the open's), destroyed
            // once, with the sensor lock held.
            unsafe {
                ASensorEventQueue_disableSensor(s.event_queue, s.asensor);
                ASensorManager_destroyEventQueue(manager, s.event_queue);
            }
            s.event_queue = ptr::null_mut();
            s.sensor = None;
        }
    }

    /// Translation of `SDL_ANDROID_SensorQuit()`.
    fn quit(&self) {
        // All sensors are closed, but we need to unblock the sensor thread
        assert_sensors_locked();
        with_sensors_unlocked(stop_sensor_thread);

        state().sensors.clear();
    }
}

#[cfg(test)]
mod tests;
