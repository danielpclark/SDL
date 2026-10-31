// Tests of the Android sensor driver (the program spawned on the device:
// the NDK's sensor manager needs no Java VM; in an app, with the
// emulator's sensors set from outside: sdl3/tests/android_sensor.rs).

use std::time::{Duration, Instant};

use super::super::*;

/// The sensor manager's sensors: their names, types (an accelerometer and
/// a gyroscope are SDL's, the others unknown, with Android's types as the
/// non-portable ones), the events of an open one, and the same sensors
/// once the subsystem is started again (with new instance IDs).
#[test]
fn sensor_manager_sensors() {
    let _l = crate::test_support::test_lock();
    init::set_main_ready();
    init::init_subsystem(InitFlags::SENSOR).unwrap();
    let ids = sensors();
    println!("android sensors: {}", ids.len());
    let mut names = std::collections::HashMap::new();
    for &id in &ids {
        let sensor_type = sensor_type_for_id(id).unwrap();
        let non_portable_type = sensor_non_portable_type_for_id(id).unwrap();
        let name = sensor_name_for_id(id).unwrap();
        println!("  {id}: {name:?}, {sensor_type:?} ({non_portable_type})");
        names.insert(id, name);
        assert_eq!(
            sensor_type,
            match non_portable_type {
                1 => SensorType::Accel,
                4 => SensorType::Gyro,
                _ => SensorType::Unknown,
            }
        );
    }
    // (the emulator's virtual device has both: tools/android_run.py)
    let accel = ids
        .iter()
        .copied()
        .find(|&id| sensor_type_for_id(id).unwrap() == SensorType::Accel);
    let Some(accel) = accel else {
        crate::test_support::skip("sensor", "the device has no accelerometer");
        init::quit_subsystem(InitFlags::SENSOR);
        return;
    };
    assert!(ids
        .iter()
        .any(|&id| sensor_type_for_id(id).unwrap() == SensorType::Gyro));

    // An open accelerometer's events: the sensor thread updates its data
    // (the device's at rest isn't zero: gravity)
    let sensor = Sensor::open(accel).unwrap();
    assert_eq!(sensor.sensor_type(), SensorType::Accel);
    assert_eq!(sensor.name().unwrap(), sensor_name_for_id(accel).unwrap());
    assert!(sensors_opened());
    let start = Instant::now();
    let mut data = [0f32; 3];
    while data == [0.0; 3] && start.elapsed() < Duration::from_secs(5) {
        std::thread::sleep(Duration::from_millis(10));
        sensor.data(&mut data).unwrap();
    }
    println!("accelerometer: {data:?}");
    assert_ne!(data, [0.0; 3], "no accelerometer event in 5 s");
    let magnitude = data.iter().map(|v| v * v).sum::<f32>().sqrt();
    assert!(
        (magnitude - STANDARD_GRAVITY).abs() < 1.0,
        "the accelerometer at rest: {data:?}"
    );

    // Opened again: the same sensor; closed once both are dropped
    let again = Sensor::open(accel).unwrap();
    drop(sensor);
    assert!(sensors_opened());
    drop(again);
    assert!(!sensors_opened());

    assert!(Sensor::open(0).is_err());
    init::quit_subsystem(InitFlags::SENSOR);

    // The same sensors after a restart (new instance IDs)
    init::init_subsystem(InitFlags::SENSOR).unwrap();
    let again = sensors();
    assert_eq!(again.len(), ids.len());
    for (&old, &new) in ids.iter().zip(&again) {
        assert_ne!(old, new);
        assert_eq!(
            sensor_name_for_id(old).unwrap_err().to_string(),
            format!("Sensor {old} not found")
        );
        assert_eq!(sensor_name_for_id(new).unwrap(), names[&old]);
    }
    init::quit_subsystem(InitFlags::SENSOR);
}
