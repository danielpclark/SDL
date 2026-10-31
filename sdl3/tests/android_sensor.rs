// Tests of the Android sensor driver in an app.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! An app (`harness = false`), run by `tools/android_run.py`, whose main
//! function traces the android sensor driver (the NDK's sensor manager)
//! with the sensor subsystem alone:
//!
//! * the sensors (the emulator's: its virtual device has an accelerometer,
//!   a gyroscope, a magnetic field, orientation, proximity, light and
//!   pressure sensors, with Android's software ones), their names, types
//!   and non-portable types (Android's);
//! * the accelerometer opened, its values set by the runner (`sensor set
//!   acceleration X:Y:Z`, the emulator console's `sensor set`): the first
//!   event with those values (within a thousandth: the accelerometer's
//!   come through the emulator's device model; all six of the event's
//!   values traced), its data; the
//!   gyroscope opened with it and set, then the accelerometer set again
//!   while both are open; the light sensor (an unknown type to SDL, which
//!   reports on changes only) opened and set;
//! * each closed: no event for it after that;
//! * the subsystem started again: the same sensors.
//!
//! What depends on timing isn't traced: the events with other values
//! before the first one with the values set (the sensors report at 60 Hz,
//! the values in a queue when they're set), the timestamps and the
//! instance IDs. The runner puts the emulator's sensors back as they were
//! once the app ends.
//!
//! The expected trace is upstream's C SDL's, from the same program in C
//! (built with upstream's android-project, Gradle and CMake, run by the
//! runner's `--run-apk`) on the same emulator (API 35, Google APIs,
//! x86-64). Elsewhere this program does nothing.

#[cfg(not(target_os = "android"))]
fn main() {}

/// The runner's words: run as an app.
#[cfg(target_os = "android")]
#[used]
#[link_section = ".sdl3_runner"]
static RUNNER: [u8; 4] = *b"app\0";

#[cfg(target_os = "android")]
mod android_support;

#[cfg(target_os = "android")]
fn main() -> std::process::ExitCode {
    android_support::start();
    let status = android_support::checked(app::main);
    android_support::finish(status)
}

#[cfg(target_os = "android")]
mod app {
    use std::sync::Mutex;
    use std::time::{Duration, Instant};

    use sdl3::events::{self, Event, EventType, SensorID};
    use sdl3::init::{self, InitFlags};
    use sdl3::sensor::{self, Sensor, SensorType};

    use super::android_support::{check_trace, request};

    /// Upstream's trace (the C program, on the emulator: API 35, Google
    /// APIs, x86-64).
    const EXPECTED: &str = "\
init: ok
init: sensors: 18
init: sensor 0: name=Goldfish 3-axis Accelerometer type=1 non-portable=1
init: sensor 1: name=Goldfish 3-axis Gyroscope type=2 non-portable=4
init: sensor 2: name=Goldfish 3-axis Magnetic field sensor type=0 non-portable=2
init: sensor 3: name=Goldfish Orientation sensor type=0 non-portable=3
init: sensor 4: name=Goldfish Ambient Temperature sensor type=0 non-portable=13
init: sensor 5: name=Goldfish Proximity sensor type=0 non-portable=8
init: sensor 6: name=Goldfish Light sensor type=0 non-portable=5
init: sensor 7: name=Goldfish Pressure sensor type=0 non-portable=6
init: sensor 8: name=Goldfish Humidity sensor type=0 non-portable=12
init: sensor 9: name=Goldfish 3-axis Magnetic field sensor (uncalibrated) type=0 non-portable=14
init: sensor 10: name=Goldfish 3-axis Gyroscope (uncalibrated) type=0 non-portable=16
init: sensor 11: name=Goldfish 3-axis Accelerometer Uncalibrated type=0 non-portable=35
init: sensor 12: name=Game Rotation Vector Sensor type=0 non-portable=15
init: sensor 13: name=GeoMag Rotation Vector Sensor type=0 non-portable=20
init: sensor 14: name=Gravity Sensor type=0 non-portable=9
init: sensor 15: name=Linear Acceleration Sensor type=0 non-portable=10
init: sensor 16: name=Rotation Vector Sensor type=0 non-portable=11
init: sensor 17: name=Orientation Sensor type=0 non-portable=3
accel: open: ok
accel: name=Goldfish 3-axis Accelerometer type=1 non-portable=1
accel: event: 1.500 9.500 2.250 0.000 0.000 0.000
accel: data: ok 1.500 9.500 2.250
gyro: open: ok
gyro: name=Goldfish 3-axis Gyroscope type=2 non-portable=4
gyro: event: 0.250 -0.500 0.125 0.000 0.000 0.000
gyro: data: ok 0.250 -0.500 0.125
accel again: event: -2.500 9.000 1.000 0.000 0.000 0.000
accel again: data: ok -2.500 9.000 1.000
light: open: ok
light: name=Goldfish Light sensor type=0 non-portable=5
light: event: 321.500 0.000 0.000 0.000 0.000 0.000
light: data: ok 321.500 0.000 0.000
accel: closed: events after: 0
gyro: closed: events after: 0
light: closed: events after: 0
quit
init again: ok
init again: sensors: 18
init again: sensor 0: name=Goldfish 3-axis Accelerometer type=1 non-portable=1
init again: sensor 1: name=Goldfish 3-axis Gyroscope type=2 non-portable=4
init again: sensor 2: name=Goldfish 3-axis Magnetic field sensor type=0 non-portable=2
init again: sensor 3: name=Goldfish Orientation sensor type=0 non-portable=3
init again: sensor 4: name=Goldfish Ambient Temperature sensor type=0 non-portable=13
init again: sensor 5: name=Goldfish Proximity sensor type=0 non-portable=8
init again: sensor 6: name=Goldfish Light sensor type=0 non-portable=5
init again: sensor 7: name=Goldfish Pressure sensor type=0 non-portable=6
init again: sensor 8: name=Goldfish Humidity sensor type=0 non-portable=12
init again: sensor 9: name=Goldfish 3-axis Magnetic field sensor (uncalibrated) type=0 non-portable=14
init again: sensor 10: name=Goldfish 3-axis Gyroscope (uncalibrated) type=0 non-portable=16
init again: sensor 11: name=Goldfish 3-axis Accelerometer Uncalibrated type=0 non-portable=35
init again: sensor 12: name=Game Rotation Vector Sensor type=0 non-portable=15
init again: sensor 13: name=GeoMag Rotation Vector Sensor type=0 non-portable=20
init again: sensor 14: name=Gravity Sensor type=0 non-portable=9
init again: sensor 15: name=Linear Acceleration Sensor type=0 non-portable=10
init again: sensor 16: name=Rotation Vector Sensor type=0 non-portable=11
init again: sensor 17: name=Orientation Sensor type=0 non-portable=3
init again: new instance IDs: yes
main returns
";

    static TRACE: Mutex<Vec<String>> = Mutex::new(Vec::new());

    fn t(line: impl Into<String>) {
        let line = line.into();
        println!("trace: {line}");
        TRACE.lock().unwrap().push(line);
    }

    fn result<T>(r: &sdl3::Result<T>) -> String {
        match r {
            Ok(_) => "ok".to_string(),
            Err(e) => format!("error: {}", e.message()),
        }
    }

    fn values(data: &[f32]) -> String {
        data.iter()
            .map(|v| format!("{v:.3}"))
            .collect::<Vec<_>>()
            .join(" ")
    }

    /// The sensors' list, and the first sensor of each type the program
    /// uses.
    fn trace_sensors(step: &str) -> Vec<SensorID> {
        let ids = sensor::sensors();
        t(format!("{step}: sensors: {}", ids.len()));
        for (i, &id) in ids.iter().enumerate() {
            t(format!(
                "{step}: sensor {i}: name={} type={} non-portable={}",
                sensor::sensor_name_for_id(id)
                    .ok()
                    .flatten()
                    .unwrap_or_else(|| "(null)".into()),
                sensor::sensor_type_for_id(id).unwrap_or(SensorType::Invalid) as i32,
                sensor::sensor_non_portable_type_for_id(id).unwrap_or(-1)
            ));
        }
        ids
    }

    /// The first sensor whose non-portable (Android) type is `ty`.
    fn find(ids: &[SensorID], ty: i32) -> Option<SensorID> {
        ids.iter()
            .copied()
            .find(|&id| sensor::sensor_non_portable_type_for_id(id).ok() == Some(ty))
    }

    /// Android's sensor types.
    const TYPE_ACCELEROMETER: i32 = 1;
    const TYPE_LIGHT: i32 = 5;
    const TYPE_GYROSCOPE: i32 = 4;

    fn open(step: &str, id: Option<SensorID>) -> Option<Sensor> {
        let Some(id) = id else {
            t(format!("{step}: none"));
            return None;
        };
        let r = Sensor::open(id);
        t(format!("{step}: open: {}", result(&r)));
        let sensor = r.ok()?;
        t(format!(
            "{step}: name={} type={} non-portable={}",
            sensor
                .name()
                .ok()
                .flatten()
                .unwrap_or_else(|| "(null)".into()),
            sensor.sensor_type() as i32,
            sensor.non_portable_type()
        ));
        Some(sensor)
    }

    /// The emulator's sensor `name` set to `set` (the console's
    /// `X:Y:Z`), then the first event of the sensor's with those values
    /// (`expect`, within a thousandth), within 10 s, and its data.
    fn set_and_wait(step: &str, sensor: &Sensor, name: &str, set: &str, expect: &[f32]) {
        let _ = request(&format!("sensor set {name} {set}"));
        let end = Instant::now() + Duration::from_secs(10);
        let mut found = None;
        let mut last = None;
        while found.is_none() && Instant::now() < end {
            events::pump();
            while let Some(event) = events::poll() {
                if let Event::Sensor(e) = event {
                    if e.which != sensor.id() {
                        continue;
                    }
                    // (within a thousandth: the emulator's accelerometer
                    // values come through its device model's rotation)
                    let close = expect
                        .iter()
                        .zip(&e.data)
                        .all(|(want, got)| (want - got).abs() < 0.001);
                    if found.is_none() && close {
                        found = Some(e.data);
                    }
                    last = Some(e.data);
                }
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        match found {
            Some(data) => t(format!("{step}: event: {}", values(&data))),
            None => t(format!(
                "{step}: event: none in 10 s (last: {})",
                last.map_or("none".to_string(), |d| values(&d))
            )),
        }
        let mut data = [0f32; 3];
        let r = sensor.data(&mut data);
        t(format!("{step}: data: {} {}", result(&r), values(&data)));
    }

    /// A sensor closed: its events after that (none).
    fn close(step: &str, sensor: Sensor) {
        let id = sensor.id();
        drop(sensor);
        events::pump();
        events::flush_event(EventType::SENSOR_UPDATE);
        let end = Instant::now() + Duration::from_millis(500);
        let mut n = 0;
        while Instant::now() < end {
            events::pump();
            while let Some(event) = events::poll() {
                if matches!(event, Event::Sensor(e) if e.which == id) {
                    n += 1;
                }
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        t(format!("{step}: closed: events after: {n}"));
    }

    pub fn main() {
        let r = init::init(InitFlags::SENSOR);
        t(format!("init: {}", result(&r)));
        let ids = trace_sensors("init");

        let accel = open("accel", find(&ids, TYPE_ACCELEROMETER));
        if let Some(accel) = &accel {
            set_and_wait(
                "accel",
                accel,
                "acceleration",
                "1.5:9.5:2.25",
                &[1.5, 9.5, 2.25],
            );
        }
        let gyro = open("gyro", find(&ids, TYPE_GYROSCOPE));
        if let Some(gyro) = &gyro {
            set_and_wait(
                "gyro",
                gyro,
                "gyroscope",
                "0.25:-0.5:0.125",
                &[0.25, -0.5, 0.125],
            );
        }
        if let Some(accel) = &accel {
            set_and_wait(
                "accel again",
                accel,
                "acceleration",
                "-2.5:9:1",
                &[-2.5, 9.0, 1.0],
            );
        }
        let light = open("light", find(&ids, TYPE_LIGHT));
        if let Some(light) = &light {
            set_and_wait("light", light, "light", "321.5", &[321.5]);
        }
        if let Some(accel) = accel {
            close("accel", accel);
        }
        if let Some(gyro) = gyro {
            close("gyro", gyro);
        }
        if let Some(light) = light {
            close("light", light);
        }

        init::quit_subsystem(InitFlags::SENSOR);
        t("quit");
        let r = init::init_subsystem(InitFlags::SENSOR);
        t(format!("init again: {}", result(&r)));
        let again = trace_sensors("init again");
        // (new instance IDs: the old ones are gone)
        let new_ids = again.iter().all(|id| !ids.contains(id))
            && ids
                .iter()
                .all(|&id| sensor::sensor_name_for_id(id).is_err());
        t(format!(
            "init again: new instance IDs: {}",
            if new_ids { "yes" } else { "no" }
        ));
        init::quit();
        t("main returns");
        let trace = TRACE.lock().unwrap().clone();
        check_trace(&trace, EXPECTED);
    }
}
