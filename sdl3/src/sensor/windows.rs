// Rust translation of src/sensor/windows/SDL_windowssensor.c and
// SDL_windowssensor.h from Simple DirectMedia Layer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! The Windows sensor driver: the Sensor API (`ISensorManager`) through
//! COM, for 3D accelerometers and gyrometers, with the data delivered by
//! the sensors' event sinks. The interfaces aren't in `windows-sys`, so
//! their vtables (from `sensorsapi.h`) are declared by hand.

use std::ffi::c_void;
use std::sync::Mutex;
use std::time::Duration;

use windows_sys::core::{BSTR, GUID, HRESULT};
use windows_sys::Win32::Devices::Sensors::{
    SensorState, SENSOR_CATEGORY_ALL, SENSOR_DATA_TYPE_ACCELERATION_X_G,
    SENSOR_DATA_TYPE_ACCELERATION_Y_G, SENSOR_DATA_TYPE_ACCELERATION_Z_G,
    SENSOR_TYPE_ACCELEROMETER_3D, SENSOR_TYPE_GYROMETER_3D,
};
use windows_sys::Win32::Foundation::{
    SysFreeString, E_INVALIDARG, E_NOINTERFACE, FILETIME, PROPERTYKEY, SYSTEMTIME, S_OK,
};
use windows_sys::Win32::System::Com::StructuredStorage::PROPVARIANT;
use windows_sys::Win32::System::Com::{CoCreateInstance, CLSCTX_INPROC_SERVER};
use windows_sys::Win32::System::Time::SystemTimeToFileTime;
use windows_sys::Win32::System::Variant::VT_R8;

use super::STANDARD_GRAVITY;
use super::{lock_sensors, send_sensor_update, SensorData, SensorDriver, SensorType};
use crate::core::windows::com::{ComObject, ComPtr, IUnknownVtbl, IID_IUNKNOWN};
use crate::core::windows::{
    co_initialize, co_uninitialize, error_from_hresult, is_equal_guid, wide_to_utf8,
};
use crate::error::{Error, Result};
use crate::events::SensorID;

const fn guid(data1: u32, data2: u16, data3: u16, data4: [u8; 8]) -> GUID {
    GUID {
        data1,
        data2,
        data3,
        data4,
    }
}

const SDL_CLSID_SENSOR_MANAGER: GUID = guid(
    0x77A1C827,
    0xFCD2,
    0x4689,
    [0x89, 0x15, 0x9D, 0x61, 0x3C, 0xC5, 0xFA, 0x3E],
);
const SDL_IID_SENSOR_MANAGER: GUID = guid(
    0xBD77DB67,
    0x45A8,
    0x42DC,
    [0x8D, 0x00, 0x6D, 0xCF, 0x15, 0xF8, 0x37, 0x7A],
);
const SDL_IID_SENSOR_MANAGER_EVENTS: GUID = guid(
    0x9B3B0B86,
    0x266A,
    0x4AAD,
    [0xB2, 0x1F, 0xFD, 0xE5, 0x50, 0x10, 0x01, 0xB7],
);
const SDL_IID_SENSOR_EVENTS: GUID = guid(
    0x5D8DCC91,
    0x4641,
    0x47E7,
    [0xB7, 0xC3, 0xB7, 0x4F, 0x48, 0xA6, 0xC3, 0x91],
);

// These constants aren't available in Visual Studio 2015 or earlier Windows SDK
const SDL_SENSOR_DATA_TYPE_ANGULAR_VELOCITY_FMTID: GUID = guid(
    0x3F8A69A2,
    0x7C5,
    0x4E48,
    [0xA9, 0x65, 0xCD, 0x79, 0x7A, 0xAB, 0x56, 0xD5],
);
const SDL_SENSOR_DATA_TYPE_ANGULAR_VELOCITY_X_DEGREES_PER_SECOND: PROPERTYKEY = PROPERTYKEY {
    fmtid: SDL_SENSOR_DATA_TYPE_ANGULAR_VELOCITY_FMTID,
    pid: 10,
}; //[VT_R8]
const SDL_SENSOR_DATA_TYPE_ANGULAR_VELOCITY_Y_DEGREES_PER_SECOND: PROPERTYKEY = PROPERTYKEY {
    fmtid: SDL_SENSOR_DATA_TYPE_ANGULAR_VELOCITY_FMTID,
    pid: 11,
}; //[VT_R8]
const SDL_SENSOR_DATA_TYPE_ANGULAR_VELOCITY_Z_DEGREES_PER_SECOND: PROPERTYKEY = PROPERTYKEY {
    fmtid: SDL_SENSOR_DATA_TYPE_ANGULAR_VELOCITY_FMTID,
    pid: 12,
}; //[VT_R8]

type Method = unsafe extern "system" fn();

/// `ISensorManagerVtbl`.
#[repr(C)]
struct ISensorManagerVtbl {
    base: IUnknownVtbl,
    get_sensors_by_category: unsafe extern "system" fn(
        *mut ISensorManager,
        *const GUID,
        *mut *mut ISensorCollection,
    ) -> HRESULT,
    get_sensors_by_type: Method,
    get_sensor_by_id: Method,
    set_event_sink:
        unsafe extern "system" fn(*mut ISensorManager, *mut ISensorManagerEvents) -> HRESULT,
    request_permissions: Method,
}
type ISensorManager = ComObject<ISensorManagerVtbl>;

/// `ISensorCollectionVtbl`.
#[repr(C)]
struct ISensorCollectionVtbl {
    base: IUnknownVtbl,
    get_at: unsafe extern "system" fn(*mut ISensorCollection, u32, *mut *mut ISensor) -> HRESULT,
    get_count: unsafe extern "system" fn(*mut ISensorCollection, *mut u32) -> HRESULT,
    add: Method,
    remove: Method,
    remove_by_id: Method,
    clear: Method,
}
type ISensorCollection = ComObject<ISensorCollectionVtbl>;

/// `ISensorVtbl`.
#[repr(C)]
struct ISensorVtbl {
    base: IUnknownVtbl,
    get_id: unsafe extern "system" fn(*mut ISensor, *mut GUID) -> HRESULT,
    get_category: Method,
    get_type: unsafe extern "system" fn(*mut ISensor, *mut GUID) -> HRESULT,
    get_friendly_name: unsafe extern "system" fn(*mut ISensor, *mut BSTR) -> HRESULT,
    get_property: Method,
    get_properties: Method,
    get_supported_data_fields: Method,
    set_properties: Method,
    supports_data_field: Method,
    get_state: unsafe extern "system" fn(*mut ISensor, *mut SensorState) -> HRESULT,
    get_data: Method,
    supports_event: Method,
    get_event_interest: Method,
    set_event_interest: Method,
    set_event_sink: unsafe extern "system" fn(*mut ISensor, *mut ISensorEvents) -> HRESULT,
}
type ISensor = ComObject<ISensorVtbl>;

/// `ISensorDataReportVtbl`.
#[repr(C)]
struct ISensorDataReportVtbl {
    base: IUnknownVtbl,
    get_timestamp: unsafe extern "system" fn(*mut ISensorDataReport, *mut SYSTEMTIME) -> HRESULT,
    get_sensor_value: unsafe extern "system" fn(
        *mut ISensorDataReport,
        *const PROPERTYKEY,
        *mut PROPVARIANT,
    ) -> HRESULT,
    get_sensor_values: Method,
}
type ISensorDataReport = ComObject<ISensorDataReportVtbl>;

/// `ISensorManagerEventsVtbl`.
#[repr(C)]
struct ISensorManagerEventsVtbl {
    query_interface: unsafe extern "system" fn(
        *mut ISensorManagerEvents,
        *const GUID,
        *mut *mut c_void,
    ) -> HRESULT,
    add_ref: unsafe extern "system" fn(*mut ISensorManagerEvents) -> u32,
    release: unsafe extern "system" fn(*mut ISensorManagerEvents) -> u32,
    on_sensor_enter:
        unsafe extern "system" fn(*mut ISensorManagerEvents, *mut ISensor, SensorState) -> HRESULT,
}
type ISensorManagerEvents = ComObject<ISensorManagerEventsVtbl>;

/// `ISensorEventsVtbl`.
#[repr(C)]
struct ISensorEventsVtbl {
    query_interface:
        unsafe extern "system" fn(*mut ISensorEvents, *const GUID, *mut *mut c_void) -> HRESULT,
    add_ref: unsafe extern "system" fn(*mut ISensorEvents) -> u32,
    release: unsafe extern "system" fn(*mut ISensorEvents) -> u32,
    on_state_changed:
        unsafe extern "system" fn(*mut ISensorEvents, *mut ISensor, SensorState) -> HRESULT,
    on_data_updated: unsafe extern "system" fn(
        *mut ISensorEvents,
        *mut ISensor,
        *mut ISensorDataReport,
    ) -> HRESULT,
    on_event: unsafe extern "system" fn(
        *mut ISensorEvents,
        *mut ISensor,
        *const GUID,
        *mut c_void,
    ) -> HRESULT,
    on_leave: unsafe extern "system" fn(*mut ISensorEvents, *const GUID) -> HRESULT,
}
type ISensorEvents = ComObject<ISensorEventsVtbl>;

/// Translation of `SDL_Windows_Sensor`.
struct WindowsSensor {
    id: SensorID,
    sensor_id: GUID,
    name: String,
    sensor_type: SensorType,
    sensor_opened: Option<SensorID>,
    // (last: released after the rest)
    sensor: ComPtr<ISensorVtbl>,
}

/// The driver's globals.
struct State {
    /// Translation of `SDL_windowscoinit`.
    windowscoinit: bool,
    /// Translation of `SDL_sensors` (and `SDL_num_sensors`).
    sensors: Vec<WindowsSensor>,
    /// Translation of `SDL_sensor_manager`.
    sensor_manager: Option<ComPtr<ISensorManagerVtbl>>,
}

/// Guarded by the sensor lock as upstream (taken first), and by this mutex
/// for Rust's sake; neither is held across an event push.
static STATE: Mutex<State> = Mutex::new(State {
    windowscoinit: false,
    sensors: Vec::new(),
    sensor_manager: None,
});

fn state() -> std::sync::MutexGuard<'static, State> {
    STATE.lock().unwrap_or_else(|e| e.into_inner())
}

/// A static COM object whose vtable is a `V` (the event sinks).
#[repr(C)]
struct StaticComObject<V: 'static> {
    vtbl: &'static V,
}

// SAFETY: the object is a pointer to a static vtable, never written.
unsafe impl<V> Sync for StaticComObject<V> {}

impl<V> StaticComObject<V> {
    fn as_ptr(&'static self) -> *mut ComObject<V> {
        (self as *const Self).cast_mut().cast()
    }
}

/// Translation of `ISensorManagerEventsVtbl_QueryInterface()`.
unsafe extern "system" fn sensor_manager_events_query_interface(
    this: *mut ISensorManagerEvents,
    riid: *const GUID,
    ppv_object: *mut *mut c_void,
) -> HRESULT {
    if ppv_object.is_null() {
        return E_INVALIDARG;
    }

    // SAFETY: COM passes a valid IID and out-pointer.
    unsafe {
        *ppv_object = std::ptr::null_mut();
        if is_equal_guid(&*riid, &IID_IUNKNOWN)
            || is_equal_guid(&*riid, &SDL_IID_SENSOR_MANAGER_EVENTS)
        {
            *ppv_object = this.cast();
            return S_OK;
        }
    }
    E_NOINTERFACE
}

/// Translation of `ISensorManagerEventsVtbl_AddRef()`.
unsafe extern "system" fn sensor_manager_events_add_ref(_this: *mut ISensorManagerEvents) -> u32 {
    1
}

/// Translation of `ISensorManagerEventsVtbl_Release()`.
unsafe extern "system" fn sensor_manager_events_release(_this: *mut ISensorManagerEvents) -> u32 {
    1
}

/// Translation of `ISensorManagerEventsVtbl_OnSensorEnter()`.
unsafe extern "system" fn sensor_manager_events_on_sensor_enter(
    _this: *mut ISensorManagerEvents,
    p_sensor: *mut ISensor,
    _state: SensorState,
) -> HRESULT {
    // SAFETY: the sensor manager passes a live sensor.
    let _ = unsafe { connect_sensor(p_sensor) };
    S_OK
}

static SENSOR_MANAGER_EVENTS_VTBL: ISensorManagerEventsVtbl = ISensorManagerEventsVtbl {
    query_interface: sensor_manager_events_query_interface,
    add_ref: sensor_manager_events_add_ref,
    release: sensor_manager_events_release,
    on_sensor_enter: sensor_manager_events_on_sensor_enter,
};
static SENSOR_MANAGER_EVENTS: StaticComObject<ISensorManagerEventsVtbl> = StaticComObject {
    vtbl: &SENSOR_MANAGER_EVENTS_VTBL,
};

/// Translation of `ISensorEventsVtbl_QueryInterface()`.
unsafe extern "system" fn sensor_events_query_interface(
    this: *mut ISensorEvents,
    riid: *const GUID,
    ppv_object: *mut *mut c_void,
) -> HRESULT {
    if ppv_object.is_null() {
        return E_INVALIDARG;
    }

    // SAFETY: COM passes a valid IID and out-pointer.
    unsafe {
        *ppv_object = std::ptr::null_mut();
        if is_equal_guid(&*riid, &IID_IUNKNOWN) || is_equal_guid(&*riid, &SDL_IID_SENSOR_EVENTS) {
            *ppv_object = this.cast();
            return S_OK;
        }
    }
    E_NOINTERFACE
}

/// Translation of `ISensorEventsVtbl_AddRef()`.
unsafe extern "system" fn sensor_events_add_ref(_this: *mut ISensorEvents) -> u32 {
    1
}

/// Translation of `ISensorEventsVtbl_Release()`.
unsafe extern "system" fn sensor_events_release(_this: *mut ISensorEvents) -> u32 {
    1
}

/// Translation of `ISensorEventsVtbl_OnStateChanged()` (which only logs
/// under `DEBUG_SENSORS`).
unsafe extern "system" fn sensor_events_on_state_changed(
    _this: *mut ISensorEvents,
    _p_sensor: *mut ISensor,
    _state: SensorState,
) -> HRESULT {
    S_OK
}

/// The value of a `VT_R8` data field of a report.
///
/// # Safety
///
/// `report` must be a live data report.
unsafe fn report_value(report: *mut ISensorDataReport, key: &PROPERTYKEY) -> Option<f64> {
    // SAFETY: PROPVARIANT is plain data; all zeroes is VT_EMPTY.
    let mut value: PROPVARIANT = unsafe { std::mem::zeroed() };
    // SAFETY: the caller's contract; the key and value outlive the call.
    let hr = unsafe { (ComObject::vtbl(report).get_sensor_value)(report, key, &mut value) };
    // SAFETY: the variant's type tag says which member is set.
    unsafe {
        if hr >= 0 && value.Anonymous.Anonymous.vt == VT_R8 {
            Some(value.Anonymous.Anonymous.Anonymous.dblVal)
        } else {
            None
        }
    }
}

/// An accelerometer report (in g) in m/s².
fn accel_values(x: f64, y: f64, z: f64) -> [f32; 3] {
    [
        x as f32 * STANDARD_GRAVITY,
        y as f32 * STANDARD_GRAVITY,
        z as f32 * STANDARD_GRAVITY,
    ]
}

/// A gyrometer report (in degrees per second) in radians per second.
fn gyro_values(x: f64, y: f64, z: f64) -> [f32; 3] {
    const DEGREES_TO_RADIANS: f32 = std::f32::consts::PI / 180.0;
    [
        x as f32 * DEGREES_TO_RADIANS,
        y as f32 * DEGREES_TO_RADIANS,
        z as f32 * DEGREES_TO_RADIANS,
    ]
}

/// A `FILETIME` in nanoseconds (since 1601).
fn filetime_to_ns(filetime: &FILETIME) -> u64 {
    let sensor_time =
        (u64::from(filetime.dwHighDateTime) << 32) | u64::from(filetime.dwLowDateTime);
    sensor_time.wrapping_mul(100)
}

/// Translation of `ISensorEventsVtbl_OnDataUpdated()`.
unsafe extern "system" fn sensor_events_on_data_updated(
    _this: *mut ISensorEvents,
    p_sensor: *mut ISensor,
    p_new_data: *mut ISensorDataReport,
) -> HRESULT {
    let timestamp = crate::timer::ticks_ns();

    let _lock = lock_sensors();
    let found = state()
        .sensors
        .iter()
        .find(|s| s.sensor.as_ptr() == p_sensor)
        .map(|s| (s.sensor_opened, s.sensor_type));
    if let Some((Some(sensor_opened), sensor_type)) = found {
        // SAFETY: SYSTEMTIME and FILETIME are plain data.
        let mut sensor_systemtime: SYSTEMTIME = unsafe { std::mem::zeroed() };
        // SAFETY: as above.
        let mut sensor_filetime: FILETIME = unsafe { std::mem::zeroed() };

        // SAFETY: the sensor passes a live report; the out-pointers are valid.
        let sensor_timestamp = unsafe {
            if (ComObject::vtbl(p_new_data).get_timestamp)(p_new_data, &mut sensor_systemtime) >= 0
                && SystemTimeToFileTime(&sensor_systemtime, &mut sensor_filetime) != 0
            {
                filetime_to_ns(&sensor_filetime)
            } else {
                timestamp
            }
        };

        let keys = match sensor_type {
            SensorType::Accel => Some([
                &SENSOR_DATA_TYPE_ACCELERATION_X_G,
                &SENSOR_DATA_TYPE_ACCELERATION_Y_G,
                &SENSOR_DATA_TYPE_ACCELERATION_Z_G,
            ]),
            SensorType::Gyro => Some([
                &SDL_SENSOR_DATA_TYPE_ANGULAR_VELOCITY_X_DEGREES_PER_SECOND,
                &SDL_SENSOR_DATA_TYPE_ANGULAR_VELOCITY_Y_DEGREES_PER_SECOND,
                &SDL_SENSOR_DATA_TYPE_ANGULAR_VELOCITY_Z_DEGREES_PER_SECOND,
            ]),
            // FIXME: Need to know how to interpret the data for this sensor
            _ => None,
        };
        if let Some(keys) = keys {
            // SAFETY: the report is live for the call.
            let values = unsafe {
                (
                    report_value(p_new_data, keys[0]),
                    report_value(p_new_data, keys[1]),
                    report_value(p_new_data, keys[2]),
                )
            };
            if let (Some(x), Some(y), Some(z)) = values {
                let values = if sensor_type == SensorType::Accel {
                    accel_values(x, y, z)
                } else {
                    gyro_values(x, y, z)
                };
                send_sensor_update(
                    Duration::from_nanos(timestamp),
                    sensor_opened,
                    sensor_timestamp,
                    &values,
                );
            }
        }
    }

    S_OK
}

/// Translation of `ISensorEventsVtbl_OnEvent()` (which only logs under
/// `DEBUG_SENSORS`).
unsafe extern "system" fn sensor_events_on_event(
    _this: *mut ISensorEvents,
    _p_sensor: *mut ISensor,
    _event_id: *const GUID,
    _p_event_data: *mut c_void,
) -> HRESULT {
    S_OK
}

/// Translation of `ISensorEventsVtbl_OnLeave()`.
unsafe extern "system" fn sensor_events_on_leave(
    _this: *mut ISensorEvents,
    id: *const GUID,
) -> HRESULT {
    let _lock = lock_sensors();
    // SAFETY: the sensor passes a valid sensor ID.
    let id = unsafe { *id };
    // FIXME (upstream): ConnectSensor() never stores the sensor's ID (it
    // stays zeroed), so this only matches a sensor whose ID is GUID_NULL.
    let leaving: Vec<*mut ISensor> = state()
        .sensors
        .iter()
        .filter(|s| is_equal_guid(&id, &s.sensor_id))
        .map(|s| s.sensor.as_ptr())
        .collect();
    for sensor in leaving {
        disconnect_sensor(sensor);
    }

    S_OK
}

static SENSOR_EVENTS_VTBL: ISensorEventsVtbl = ISensorEventsVtbl {
    query_interface: sensor_events_query_interface,
    add_ref: sensor_events_add_ref,
    release: sensor_events_release,
    on_state_changed: sensor_events_on_state_changed,
    on_data_updated: sensor_events_on_data_updated,
    on_event: sensor_events_on_event,
    on_leave: sensor_events_on_leave,
};
static SENSOR_EVENTS: StaticComObject<ISensorEventsVtbl> = StaticComObject {
    vtbl: &SENSOR_EVENTS_VTBL,
};

/// The SDL sensor type of a Sensor API sensor type, if SDL supports it.
fn sensor_type_from_id(type_id: &GUID) -> Option<SensorType> {
    if is_equal_guid(type_id, &SENSOR_TYPE_ACCELEROMETER_3D) {
        Some(SensorType::Accel)
    } else if is_equal_guid(type_id, &SENSOR_TYPE_GYROMETER_3D) {
        Some(SensorType::Gyro)
    } else {
        None
    }
}

/// Translation of `ConnectSensor()`.
///
/// # Safety
///
/// `sensor` must be a live sensor (the caller keeps its reference).
unsafe fn connect_sensor(sensor: *mut ISensor) -> Result<()> {
    let mut sensor_id = guid(0, 0, 0, [0; 8]);
    let mut type_id = guid(0, 0, 0, [0; 8]);
    let mut bstr_name: BSTR = std::ptr::null();
    // SAFETY: the caller's contract.
    let vtbl = unsafe { ComObject::vtbl(sensor) };

    // SAFETY: the out-pointer is valid.
    let hr = unsafe { (vtbl.get_id)(sensor, &mut sensor_id) };
    if hr < 0 {
        return Err(error_from_hresult(Some("Couldn't get sensor ID"), hr));
    }

    // SAFETY: as above.
    let hr = unsafe { (vtbl.get_type)(sensor, &mut type_id) };
    if hr < 0 {
        return Err(error_from_hresult(Some("Couldn't get sensor type"), hr));
    }

    let Some(sensor_type) = sensor_type_from_id(&type_id) else {
        return Err(Error::new("Unknown sensor type"));
    };

    // SAFETY: as above.
    let hr = unsafe { (vtbl.get_friendly_name)(sensor, &mut bstr_name) };
    let name = if hr >= 0 && !bstr_name.is_null() {
        // SAFETY: a BSTR is NUL-terminated after its length.
        unsafe {
            let len = *(bstr_name.cast::<u32>().sub(1)) as usize / 2;
            wide_to_utf8(std::slice::from_raw_parts(bstr_name, len))
        }
    } else {
        "Unknown Sensor".to_string()
    };
    if !bstr_name.is_null() {
        // SAFETY: the name is ours to free.
        unsafe { SysFreeString(bstr_name) };
    }

    let _lock = lock_sensors();

    // SAFETY: the caller's sensor is live; AddRef() takes our reference.
    let sensor = unsafe {
        (vtbl.base.add_ref)(sensor.cast());
        ComPtr::from_raw(sensor).expect("non-null sensor")
    };
    // SAFETY: the event sink is a static object.
    unsafe { (vtbl.set_event_sink)(sensor.as_ptr(), SENSOR_EVENTS.as_ptr()) };

    state().sensors.push(WindowsSensor {
        id: crate::utils::next_object_id(),
        // FIXME (upstream): the ID read above isn't stored (SDL_zerop()
        // leaves it zeroed), which OnLeave() compares against.
        sensor_id: {
            let _ = sensor_id;
            guid(0, 0, 0, [0; 8])
        },
        name,
        sensor_type,
        sensor_opened: None,
        sensor,
    });

    Ok(())
}

/// Translation of `DisconnectSensor()`.
fn disconnect_sensor(sensor: *mut ISensor) {
    let _lock = lock_sensors();
    let mut state = state();
    if let Some(i) = state
        .sensors
        .iter()
        .position(|s| s.sensor.as_ptr() == sensor)
    {
        /* This call hangs for some reason:
         * https://github.com/libsdl-org/SDL/issues/5288
         */
        // ISensor_SetEventSink(sensor, NULL);
        let old_sensor = state.sensors.remove(i);
        drop(state);
        drop(old_sensor);
    }
}

/// Translation of `SDL_WINDOWS_SensorDriver`.
pub(super) struct WindowsSensorDriver;

pub(super) static WINDOWS_SENSOR_DRIVER: WindowsSensorDriver = WindowsSensorDriver;

impl SensorDriver for WindowsSensorDriver {
    /// Translation of `SDL_WINDOWS_SensorInit()`.
    fn init(&self) -> Result<()> {
        if co_initialize() == S_OK {
            state().windowscoinit = true;
        }

        // SAFETY: the out-pointer receives an ISensorManager on success.
        let manager = unsafe {
            ComPtr::<ISensorManagerVtbl>::from_out(|out| {
                CoCreateInstance(
                    &SDL_CLSID_SENSOR_MANAGER,
                    std::ptr::null_mut(),
                    CLSCTX_INPROC_SERVER,
                    &SDL_IID_SENSOR_MANAGER,
                    out.cast(),
                )
            })
        };
        let Ok(manager) = manager else {
            // If we can't create a sensor manager (i.e. on Wine), we won't have any sensors, but don't fail the init
            return Ok(()); // WIN_SetErrorFromHRESULT("Couldn't create the sensor manager", hr);
        };

        // SAFETY: the event sink is a static object.
        let hr = unsafe {
            (manager.vtbl().set_event_sink)(manager.as_ptr(), SENSOR_MANAGER_EVENTS.as_ptr())
        };
        if hr < 0 {
            return Err(error_from_hresult(
                Some("Couldn't set the sensor manager event sink"),
                hr,
            ));
        }

        // SAFETY: the out-pointer receives an ISensorCollection on success.
        let sensor_collection = unsafe {
            ComPtr::<ISensorCollectionVtbl>::from_out(|out| {
                (manager.vtbl().get_sensors_by_category)(
                    manager.as_ptr(),
                    &SENSOR_CATEGORY_ALL,
                    out,
                )
            })
        };
        state().sensor_manager = Some(manager);
        if let Ok(sensor_collection) = sensor_collection {
            let mut count = 0u32;
            // SAFETY: the out-pointer is valid.
            let hr = unsafe {
                (sensor_collection.vtbl().get_count)(sensor_collection.as_ptr(), &mut count)
            };
            if hr >= 0 {
                for i in 0..count {
                    // SAFETY: the out-pointer receives an ISensor on success.
                    let sensor = unsafe {
                        ComPtr::<ISensorVtbl>::from_out(|out| {
                            (sensor_collection.vtbl().get_at)(sensor_collection.as_ptr(), i, out)
                        })
                    };
                    if let Ok(sensor) = sensor {
                        let mut sensor_state: SensorState = 0;
                        // SAFETY: the out-pointer is valid.
                        let hr = unsafe {
                            (sensor.vtbl().get_state)(sensor.as_ptr(), &mut sensor_state)
                        };
                        if hr >= 0 {
                            // SAFETY: the sensor is live while we hold it.
                            unsafe {
                                sensor_manager_events_on_sensor_enter(
                                    SENSOR_MANAGER_EVENTS.as_ptr(),
                                    sensor.as_ptr(),
                                    sensor_state,
                                );
                            }
                        }
                        // FIXME (upstream): released as an ISensorManager
                        // (ISensorManager_Release(sensor)), which works
                        // since Release() is at the same place in every
                        // vtable.
                        drop(sensor);
                    }
                }
            }
        }
        Ok(())
    }

    /// Translation of `SDL_WINDOWS_SensorGetCount()`.
    fn count(&self) -> usize {
        state().sensors.len()
    }

    /// Translation of `SDL_WINDOWS_SensorDetect()`.
    fn detect(&self) {}

    /// Translation of `SDL_WINDOWS_SensorGetDeviceName()`.
    fn device_name(&self, device_index: usize) -> Option<String> {
        state().sensors.get(device_index).map(|s| s.name.clone())
    }

    /// Translation of `SDL_WINDOWS_SensorGetDeviceType()`.
    fn device_type(&self, device_index: usize) -> SensorType {
        state()
            .sensors
            .get(device_index)
            .map_or(SensorType::Invalid, |s| s.sensor_type)
    }

    /// Translation of `SDL_WINDOWS_SensorGetDeviceNonPortableType()`.
    fn device_non_portable_type(&self, _device_index: usize) -> i32 {
        -1
    }

    /// Translation of `SDL_WINDOWS_SensorGetDeviceInstanceID()`.
    fn device_instance_id(&self, device_index: usize) -> SensorID {
        state()
            .sensors
            .get(device_index)
            .map_or(SensorID::MAX, |s| s.id)
    }

    /// Translation of `SDL_WINDOWS_SensorOpen()`.
    fn open(&self, sensor: &mut SensorData, device_index: usize) -> Result<()> {
        let mut state = state();
        let Some(s) = state.sensors.get_mut(device_index) else {
            return Err(Error::invalid_param("device_index"));
        };
        s.sensor_opened = Some(sensor.instance_id);
        Ok(())
    }

    /// Translation of `SDL_WINDOWS_SensorUpdate()`.
    fn update(&self, _sensor: SensorID) {}

    /// Translation of `SDL_WINDOWS_SensorClose()`.
    fn close(&self, sensor: &mut SensorData) {
        let mut state = state();
        if let Some(s) = state
            .sensors
            .iter_mut()
            .find(|s| s.sensor_opened == Some(sensor.instance_id))
        {
            s.sensor_opened = None;
        }
    }

    /// Translation of `SDL_WINDOWS_SensorQuit()`.
    fn quit(&self) {
        loop {
            let Some(first) = state().sensors.first().map(|s| s.sensor.as_ptr()) else {
                break;
            };
            disconnect_sensor(first);
        }

        let manager = state().sensor_manager.take();
        if let Some(manager) = manager {
            // SAFETY: clearing the event sink.
            unsafe { (manager.vtbl().set_event_sink)(manager.as_ptr(), std::ptr::null_mut()) };
            drop(manager);
        }

        let mut state = state();
        if state.windowscoinit {
            co_uninitialize();
            // (so a later init doesn't uninitialize twice)
            state.windowscoinit = false;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn data_mapping() {
        assert_eq!(
            accel_values(1.0, -0.5, 0.0),
            [STANDARD_GRAVITY, -STANDARD_GRAVITY / 2.0, 0.0]
        );
        let g = gyro_values(180.0, -90.0, 0.0);
        assert!((g[0] - std::f32::consts::PI).abs() < 1e-6);
        assert!((g[1] + std::f32::consts::FRAC_PI_2).abs() < 1e-6);
        assert_eq!(g[2], 0.0);
        let ft = FILETIME {
            dwLowDateTime: 2,
            dwHighDateTime: 1,
        };
        assert_eq!(filetime_to_ns(&ft), ((1u64 << 32) + 2) * 100);
        assert_eq!(
            sensor_type_from_id(&SENSOR_TYPE_ACCELEROMETER_3D),
            Some(SensorType::Accel)
        );
        assert_eq!(
            sensor_type_from_id(&SENSOR_TYPE_GYROMETER_3D),
            Some(SensorType::Gyro)
        );
        assert_eq!(sensor_type_from_id(&SENSOR_CATEGORY_ALL), None);
    }

    /// The method slots, as `offsetof()` in `sensorsapi.h` gives them
    /// (checked with a mingw-w64 C harness).
    #[test]
    fn vtable_layouts() {
        use std::mem::offset_of;
        let slot = |offset: usize| offset / size_of::<usize>();
        assert_eq!(
            slot(offset_of!(ISensorManagerVtbl, get_sensors_by_category)),
            3
        );
        assert_eq!(slot(offset_of!(ISensorManagerVtbl, set_event_sink)), 6);
        assert_eq!(slot(size_of::<ISensorManagerVtbl>()), 8);
        assert_eq!(slot(offset_of!(ISensorCollectionVtbl, get_at)), 3);
        assert_eq!(slot(offset_of!(ISensorCollectionVtbl, get_count)), 4);
        assert_eq!(slot(size_of::<ISensorCollectionVtbl>()), 9);
        assert_eq!(slot(offset_of!(ISensorVtbl, get_id)), 3);
        assert_eq!(slot(offset_of!(ISensorVtbl, get_type)), 5);
        assert_eq!(slot(offset_of!(ISensorVtbl, get_friendly_name)), 6);
        assert_eq!(slot(offset_of!(ISensorVtbl, get_state)), 12);
        assert_eq!(slot(offset_of!(ISensorVtbl, set_event_sink)), 17);
        assert_eq!(slot(offset_of!(ISensorDataReportVtbl, get_timestamp)), 3);
        assert_eq!(slot(offset_of!(ISensorDataReportVtbl, get_sensor_value)), 4);
        assert_eq!(
            slot(offset_of!(ISensorManagerEventsVtbl, on_sensor_enter)),
            3
        );
        assert_eq!(slot(offset_of!(ISensorEventsVtbl, on_state_changed)), 3);
        assert_eq!(slot(offset_of!(ISensorEventsVtbl, on_data_updated)), 4);
        assert_eq!(slot(offset_of!(ISensorEventsVtbl, on_event)), 5);
        assert_eq!(slot(offset_of!(ISensorEventsVtbl, on_leave)), 6);
        assert_eq!(size_of::<PROPVARIANT>(), 24);
    }

    #[test]
    fn event_sinks_answer_query_interface() {
        let mut out: *mut c_void = std::ptr::null_mut();
        let sink = SENSOR_EVENTS.as_ptr();
        // SAFETY: the sinks are static objects; the IIDs and out-pointer are valid.
        unsafe {
            let vtbl = ComObject::vtbl(sink);
            assert_eq!(
                (vtbl.query_interface)(sink, &SDL_IID_SENSOR_EVENTS, &mut out),
                S_OK
            );
            assert_eq!(out, sink.cast());
            assert_eq!((vtbl.query_interface)(sink, &IID_IUNKNOWN, &mut out), S_OK);
            assert_eq!(
                (vtbl.query_interface)(sink, &SDL_IID_SENSOR_MANAGER_EVENTS, &mut out),
                E_NOINTERFACE
            );
            assert!(out.is_null());
            assert_eq!(
                (vtbl.query_interface)(sink, &IID_IUNKNOWN, std::ptr::null_mut()),
                E_INVALIDARG
            );
            assert_eq!((vtbl.add_ref)(sink), 1);
            assert_eq!((vtbl.release)(sink), 1);

            let manager_sink = SENSOR_MANAGER_EVENTS.as_ptr();
            let vtbl = ComObject::vtbl(manager_sink);
            assert_eq!(
                (vtbl.query_interface)(manager_sink, &SDL_IID_SENSOR_MANAGER_EVENTS, &mut out),
                S_OK
            );
            assert_eq!(out, manager_sink.cast());
            assert_eq!(
                (vtbl.query_interface)(manager_sink, &SDL_IID_SENSOR_EVENTS, &mut out),
                E_NOINTERFACE
            );
        }
    }
}
