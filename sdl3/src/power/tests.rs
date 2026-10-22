use super::*;

/// `state/seconds/percent`, with upstream's `-1` for unknown.
#[allow(dead_code)] // (only the Linux tests compare formatted results)
fn format(info: Option<PowerInfo>) -> String {
    match info {
        None => "none".to_owned(),
        Some(info) => format!(
            "{:?}/{}/{}",
            info.state,
            info.time_left.map_or(-1, |t| t.as_secs() as i64),
            info.percent.map_or(-1, i32::from)
        ),
    }
}

/// The lines of `expected.txt`, from upstream's `SDL_syspower.c` (Linux)
/// built against the same trees and fake UPower devices, with its paths
/// pointed into the trees and its D-Bus calls answered from
/// `upower.txt`.
#[cfg(target_os = "linux")]
fn expected(name: &str) -> &'static str {
    include_str!("testdata/expected.txt")
        .lines()
        .find_map(|l| l.strip_prefix(name)?.strip_prefix(": "))
        .unwrap_or_else(|| panic!("no expected result for {name}"))
}

/// `(name, lines)` of each `== name` block of a test data file.
#[cfg(target_os = "linux")]
fn cases(data: &'static str) -> Vec<(&'static str, Vec<&'static str>)> {
    let mut cases: Vec<(&str, Vec<&str>)> = Vec::new();
    for line in data.lines() {
        if let Some(name) = line.strip_prefix("== ") {
            cases.push((name, Vec::new()));
        } else if !line.is_empty() {
            cases.last_mut().unwrap().1.push(line);
        }
    }
    cases
}

#[cfg(target_os = "linux")]
#[test]
fn linux_proc_and_sys_trees_match_upstream() {
    let _l = crate::test_support::test_lock();
    for (name, lines) in cases(include_str!("testdata/trees.txt")) {
        let tmp = crate::test_support::TempDir::new("power");
        for line in lines {
            let (kind, rest) = line.split_once(' ').unwrap();
            if kind == "d" {
                std::fs::create_dir_all(tmp.path(rest)).unwrap();
            } else {
                let (path, value) = rest.split_once('=').unwrap();
                let path = tmp.path(path);
                std::fs::create_dir_all(std::path::Path::new(&path).parent().unwrap()).unwrap();
                std::fs::write(&path, value.replace("\\n", "\n")).unwrap();
            }
        }
        *linux::TEST_ROOT.lock().unwrap() = tmp.0.clone();
        let got = format!(
            "sysfs={} acpi={} apm={}",
            format(linux::get_power_info_linux_sys_class_power_supply()),
            format(linux::get_power_info_linux_proc_acpi()),
            format(linux::get_power_info_linux_proc_apm())
        );
        linux::TEST_ROOT.lock().unwrap().clear();
        assert_eq!(got, expected(name), "{name}");
    }
}

#[cfg(target_os = "linux")]
#[test]
fn linux_upower_matches_upstream() {
    use crate::core::linux::dbus::test_bus::{self, peer, Bus};
    use crate::core::linux::dbus::{Arg, Message, Value};
    use std::sync::{Arc, Mutex};

    let _l = crate::test_support::test_lock();
    let Some(bus) = Bus::start() else { return };

    // A UPower daemon answering from the current case.
    let current: Arc<Mutex<Vec<&'static str>>> = Arc::default();
    let refreshed: Arc<Mutex<Vec<String>>> = Arc::default();
    let (c2, r2) = (current.clone(), refreshed.clone());
    let upower = peer(
        &bus.address,
        "org.freedesktop.UPower",
        move |_, msg: &Message| {
            let lines = c2.lock().unwrap().clone();
            let error = || msg.new_error("org.freedesktop.DBus.Error.Failed", "no");
            if msg.is_method_call("org.freedesktop.UPower", "GetDisplayDevice") {
                let Some(path) = lines.iter().find_map(|l| l.strip_prefix("display ")) else {
                    return error();
                };
                let mut reply = msg.new_method_return()?;
                reply.append_args(&[Arg::ObjectPath(path)]);
                return Some(reply);
            }
            if msg.is_method_call("org.freedesktop.UPower", "EnumerateDevices") {
                let Some(list) = lines.iter().find_map(|l| l.strip_prefix("enumerate")) else {
                    return error();
                };
                let paths: Vec<&str> = list.trim().split(',').filter(|p| !p.is_empty()).collect();
                let mut reply = msg.new_method_return()?;
                reply.writer().container(b'a', Some("o"), |a| {
                    paths.iter().all(|p| a.append(&Arg::ObjectPath(p)))
                });
                return Some(reply);
            }
            if msg.is_method_call("org.freedesktop.UPower.Device", "Refresh") {
                for line in &lines {
                    if let Some(rest) = line.strip_prefix("p ") {
                        let path = rest.split(' ').next().unwrap();
                        if msg.has_path(path) {
                            r2.lock().unwrap().push(path.to_owned());
                            break;
                        }
                    }
                }
                return msg.new_method_return();
            }
            if msg.is_method_call("org.freedesktop.DBus.Properties", "Get") {
                let args = msg.args();
                assert_eq!(
                    args.first(),
                    Some(&Value::Str("org.freedesktop.UPower.Device".into()))
                );
                let Some(Value::Str(property)) = args.get(1) else {
                    return error();
                };
                for line in &lines {
                    let Some(rest) = line.strip_prefix("p ") else {
                        continue;
                    };
                    let f: Vec<&str> = rest.split(' ').collect();
                    if f[1] != property || !msg.has_path(f[0]) {
                        continue;
                    }
                    let arg = match f[2] {
                        "u" => Arg::U32(f[3].parse().unwrap()),
                        "i" => Arg::I32(f[3].parse().unwrap()),
                        "b" => Arg::Bool(f[3] == "1"),
                        "d" => Arg::F64(f[3].parse().unwrap()),
                        "x" => Arg::I64(f[3].parse().unwrap()),
                        other => panic!("{other}"),
                    };
                    let mut reply = msg.new_method_return()?;
                    reply.writer().variant(f[2], |v| v.append(&arg));
                    return Some(reply);
                }
                return error();
            }
            None
        },
    );
    test_bus::use_as_session_and_system(&bus);

    let mut upower = Some(upower);
    for (name, lines) in cases(include_str!("testdata/upower.txt")) {
        if name == "no_upower" {
            upower.take().unwrap().stop(); // nobody owns the name now
        }
        *current.lock().unwrap() = lines;
        refreshed.lock().unwrap().clear();
        let got = format(linux::get_power_info_linux_org_freedesktop_upower());
        assert_eq!(format!("upower={got}"), expected(name), "{name}");
        if name == "display_discharging" {
            // The battery is asked to refresh before its state is read.
            assert_eq!(
                *refreshed.lock().unwrap(),
                ["/org/freedesktop/UPower/devices/DisplayDevice"]
            );
        }
    }
    test_bus::release_session();
}

/// The dispatcher's answer holds together on any machine.
#[test]
fn power_info_is_consistent() {
    let _l = crate::test_support::test_lock();
    let info = power_info();
    assert_ne!(info.state, PowerState::Error);
    if let Some(pct) = info.percent {
        assert!(pct <= 100, "{info:?}");
    }
    if matches!(info.state, PowerState::NoBattery | PowerState::Unknown) {
        // (only a battery's details are reported)
        #[cfg(windows)]
        assert_eq!((info.time_left, info.percent), (None, None), "{info:?}");
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos", windows)))]
    assert_eq!(info.state, PowerState::Unknown);
    // (always a definitive answer on macOS)
    #[cfg(target_os = "macos")]
    assert_ne!(info.state, PowerState::Unknown);
}

/// The Battery Status API, stubbed: a `navigator.getBattery()` whose
/// promise is a thenable that answers at once (a real one would answer
/// from the event loop, after the test), with a battery whose state the
/// test sets. The results are what upstream's C reports for the same
/// stub, built with emcc and run under node, with its `-1`s as `None` (and
/// one negative time, `-5` upstream, which `PowerInfo` can't hold).
#[cfg(target_os = "emscripten")]
#[test]
fn emscripten_battery_matches_upstream() {
    use sdl3_emscripten::em_asm;
    use PowerState::*;
    let _l = crate::test_support::test_lock();
    // (node has no Battery Status API)
    assert_eq!(power_info().state, Unknown);

    // SAFETY: plain JavaScript.
    unsafe {
        em_asm!(
            r#"
            Module['sdl3_test_battery'] = { level: 1, charging: true, chargingTime: 0, dischargingTime: Infinity };
            navigator.getBattery = () => ({ then: (f) => f(Module['sdl3_test_battery']) });
        "#
        )
    };
    let info = || {
        let i = power_info();
        (i.state, i.time_left.map(|t| t.as_secs()), i.percent)
    };
    // The first call has no data yet (upstream's uninitialized structure).
    assert_eq!(info(), (OnBattery, Some(0), Some(0)));
    let inf = f64::INFINITY;
    // (INT_MAX: Infinity, or past it)
    const MAX: u64 = i32::MAX as u64;
    type Battery = (f64, bool, f64, f64);
    let cases: [(Battery, _); 10] = [
        // level, charging, chargingTime, dischargingTime
        ((1.0, true, 0.0, inf), (NoBattery, None, None)),
        ((0.5, false, inf, 3600.0), (OnBattery, Some(3600), Some(0))),
        ((0.5, true, 1800.0, inf), (Charging, Some(MAX), Some(0))),
        ((0.8, true, 0.0, inf), (Charged, Some(MAX), Some(0))),
        ((1.0, true, 0.0, 120.0), (NoBattery, None, None)),
        ((1.0, false, 0.0, 0.0), (OnBattery, Some(0), Some(100))),
        (
            (0.999, false, inf, 7200.9),
            (OnBattery, Some(7200), Some(0)),
        ),
        ((0.25, false, inf, f64::NAN), (OnBattery, Some(0), Some(0))),
        ((0.25, false, inf, 1e12), (OnBattery, Some(MAX), Some(0))),
        ((0.25, false, inf, -5.0), (OnBattery, None, Some(0))),
    ];
    for ((level, charging, charging_time, discharging_time), expected) in cases {
        // SAFETY: plain JavaScript.
        unsafe {
            em_asm!(
                "Object.assign(Module['sdl3_test_battery'], { level: $0, charging: !!$1, chargingTime: $2, dischargingTime: $3 });",
                level,
                charging,
                charging_time,
                discharging_time
            )
        };
        assert_eq!(
            info(),
            expected,
            "{level} {charging} {charging_time} {discharging_time}"
        );
    }

    // SAFETY: plain JavaScript.
    unsafe { em_asm!("delete navigator.getBattery;") };
    assert_eq!(info(), (Unknown, None, None));
}

#[cfg(windows)]
#[test]
fn windows_power_status() {
    use windows_sys::Win32::System::Power::SYSTEM_POWER_STATUS;
    let status = |ac: u8, flag: u8, pct: u8, secs: u32| SYSTEM_POWER_STATUS {
        ACLineStatus: ac,
        BatteryFlag: flag,
        BatteryLifePercent: pct,
        SystemStatusFlag: 0,
        BatteryLifeTime: secs,
        BatteryFullLifeTime: u32::MAX,
    };
    let info = |s: Option<SYSTEM_POWER_STATUS>| {
        let i = windows::power_info_from_status(s.as_ref());
        (i.state, i.time_left.map(|t| t.as_secs()), i.percent)
    };
    use PowerState::*;
    assert_eq!(info(None), (Unknown, None, None));
    assert_eq!(info(Some(status(1, 0xFF, 50, 60))), (Unknown, None, None));
    assert_eq!(info(Some(status(1, 128, 50, 60))), (NoBattery, None, None));
    assert_eq!(
        info(Some(status(1, 8, 50, 60))),
        (Charging, Some(60), Some(50))
    );
    assert_eq!(
        info(Some(status(0, 8 | 1, 120, 60))),
        (Charging, Some(60), Some(100))
    );
    assert_eq!(
        info(Some(status(1, 1, 99, u32::MAX))),
        (Charged, None, Some(99))
    );
    assert_eq!(
        info(Some(status(0, 2, 255, 7200))),
        (OnBattery, Some(7200), None)
    );
    assert_eq!(
        info(Some(status(255, 4, 3, 0x8000_0000))),
        (OnBattery, None, Some(3))
    );

    // The live answer is the same as the API's.
    let live = windows::get_power_info_windows().unwrap();
    // SAFETY: SYSTEM_POWER_STATUS is plain data.
    let mut raw: SYSTEM_POWER_STATUS = unsafe { std::mem::zeroed() };
    // SAFETY: raw is a valid out-parameter.
    let ok = unsafe { windows_sys::Win32::System::Power::GetSystemPowerStatus(&mut raw) } != 0;
    assert_eq!(
        live.state,
        windows::power_info_from_status(ok.then_some(&raw)).state
    );
    assert_eq!(power_info().state, live.state);
}
