// Rust translation of src/power/linux/SDL_syspower.c from Simple DirectMedia Layer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! The Linux power backends: UPower over D-Bus, `/sys/class/power_supply`,
//! `/proc/acpi` and `/proc/apm`, in that order of preference.

use std::borrow::Cow;
use std::cmp::Ordering;
use std::io::Read;

use super::{PowerInfo, PowerState};
use crate::core::linux::dbus::{self, Arg, Connection, Value};
use crate::stdlib::string::{strcasecmp, strtol};

const PROC_APM_PATH: &str = "/proc/apm";
const PROC_ACPI_BATTERY_PATH: &str = "/proc/acpi/battery";
const PROC_ACPI_AC_ADAPTER_PATH: &str = "/proc/acpi/ac_adapter";
const SYS_CLASS_POWER_SUPPLY_PATH: &str = "/sys/class/power_supply";

/// A directory the tests point the `/proc` and `/sys` paths into
/// (prepended to them), so fake trees can be parsed.
#[cfg(test)]
pub(super) static TEST_ROOT: std::sync::Mutex<String> = std::sync::Mutex::new(String::new());

/// One of the paths above (under [`TEST_ROOT`] in tests).
fn root_path(path: &'static str) -> Cow<'static, str> {
    #[cfg(test)]
    {
        let root = TEST_ROOT.lock().unwrap_or_else(|e| e.into_inner());
        if !root.is_empty() {
            return Cow::Owned(format!("{root}{path}"));
        }
    }
    Cow::Borrowed(path)
}

/// `SDL_strcasecmp(a, b) == 0`.
fn eq_ignore_case(a: &[u8], b: &str) -> bool {
    strcasecmp(a, b) == Ordering::Equal
}

/// `SDL_strtol(str, &endptr, base)`: the value and the rest of the string.
fn strtol_rest(s: &[u8], base: u32) -> (i32, &[u8]) {
    let (value, len) = strtol(s, base);
    (value as i32, &s[len..])
}

/// `SDL_atoi()`.
fn atoi(s: &[u8]) -> i32 {
    strtol(s, 10).0 as i32
}

/// Translation of `open_power_file()`: `base/node/key`, opened read-only
/// (and close-on-exec, as std always opens).
fn open_power_file(base: &str, node: &str, key: &str) -> Option<std::fs::File> {
    std::fs::File::open(format!("{base}/{node}/{key}")).ok()
}

/// Read up to `buflen - 1` bytes in one `read()`, as a C string (up to the
/// first NUL). Translation of `read_power_file()`.
fn read_power_file(base: &str, node: &str, key: &str, buflen: usize) -> Option<Vec<u8>> {
    let mut f = open_power_file(base, node, key)?;
    read_c_string(&mut f, buflen)
}

/// One `read()` of at most `buflen - 1` bytes, cut at the first NUL like
/// the C string it becomes.
fn read_c_string(f: &mut std::fs::File, buflen: usize) -> Option<Vec<u8>> {
    let mut buf = vec![0u8; buflen - 1];
    let br = f.read(&mut buf).ok()?;
    buf.truncate(br); // null-terminate the string.
    if let Some(nul) = buf.iter().position(|&b| b == 0) {
        buf.truncate(nul);
    }
    Some(buf)
}

/// Translation of `make_proc_acpi_key_val()`: the next `key: value` line
/// from `ptr`, which is advanced past it.
fn make_proc_acpi_key_val<'a>(ptr: &mut &'a [u8]) -> Option<(&'a [u8], &'a [u8])> {
    let mut p = *ptr;

    while let [b' ', rest @ ..] = p {
        p = rest; // skip whitespace.
    }

    if p.is_empty() {
        return None; // EOF.
    }

    let colon = p.iter().position(|&c| c == b':');
    let Some(colon) = colon else {
        return None; // (unexpected) EOF.
    };
    let key = &p[..colon]; // terminate the key.
    p = &p[colon + 1..];

    while let [b' ', rest @ ..] = p {
        p = rest; // skip whitespace.
    }

    if p.is_empty() {
        return None; // (unexpected) EOF.
    }

    let end = p.iter().position(|&c| c == b'\n').unwrap_or(p.len());
    let val = &p[..end];
    p = &p[end..];
    if !p.is_empty() {
        p = &p[1..]; // terminate the value.
    }

    *ptr = p; // store for next time.
    Some((key, val))
}

/// The directory's entries, `.` and `..` included, as `readdir()` lists
/// them (std leaves those two out; `readdir()` reports them first on the
/// filesystems `/proc` and `/sys` are, procfs and sysfs, and on tmpfs).
fn readdir_all(path: &str) -> Option<Vec<String>> {
    let dir = std::fs::read_dir(path).ok()?;
    let mut names = vec![".".to_owned(), "..".to_owned()];
    for ent in dir {
        let Ok(ent) = ent else {
            break; // (readdir() returning NULL on error ends the loop too.)
        };
        names.push(ent.file_name().to_string_lossy().into_owned());
    }
    Some(names)
}

/// The battery state that [`check_proc_acpi_battery`] accumulates.
struct AcpiBatteries {
    have_battery: bool,
    charging: bool,
    seconds: i32,
    percent: i32,
}

/// Translation of `check_proc_acpi_battery()`.
fn check_proc_acpi_battery(base: &str, node: &str, acc: &mut AcpiBatteries) {
    let mut charge = false;
    let mut choose = false;
    let mut maximum = -1;
    let mut remaining = -1;
    let secs = -1;
    let mut pct = -1;

    let Some(state) = read_power_file(base, node, "state", 1024) else {
        return;
    };
    let Some(info) = read_power_file(base, node, "info", 1024) else {
        return;
    };

    let mut ptr = &state[..];
    while let Some((key, val)) = make_proc_acpi_key_val(&mut ptr) {
        if eq_ignore_case(key, "present") {
            if eq_ignore_case(val, "yes") {
                acc.have_battery = true;
            }
        } else if eq_ignore_case(key, "charging state") {
            // !!! FIXME: what exactly _does_ charging/discharging mean?
            if eq_ignore_case(val, "charging/discharging") || eq_ignore_case(val, "charging") {
                charge = true;
            }
        } else if eq_ignore_case(key, "remaining capacity") {
            let (cvt, endptr) = strtol_rest(val, 10);
            if endptr.first() == Some(&b' ') {
                remaining = cvt;
            }
        }
    }

    let mut ptr = &info[..];
    while let Some((key, val)) = make_proc_acpi_key_val(&mut ptr) {
        if eq_ignore_case(key, "design capacity") {
            let (cvt, endptr) = strtol_rest(val, 10);
            if endptr.first() == Some(&b' ') {
                maximum = cvt;
            }
        }
    }

    if maximum >= 0 && remaining >= 0 {
        pct = ((remaining as f32 / maximum as f32) * 100.0f32) as i32;
        pct = pct.clamp(0, 100);
    }

    // !!! FIXME: calculate (secs).

    /*
     * We pick the battery that claims to have the most minutes left.
     *  (failing a report of minutes, we'll take the highest percent.)
     */
    if secs < 0 && acc.seconds < 0 {
        if pct < 0 && acc.percent < 0 {
            choose = true; // at least we know there's a battery.
        }
        if pct > acc.percent {
            choose = true;
        }
    } else if secs > acc.seconds {
        choose = true;
    }

    if choose {
        acc.seconds = secs;
        acc.percent = pct;
        acc.charging = charge;
    }
}

/// Translation of `check_proc_acpi_ac_adapter()`.
fn check_proc_acpi_ac_adapter(base: &str, node: &str, have_ac: &mut bool) {
    let Some(state) = read_power_file(base, node, "state", 256) else {
        return;
    };

    let mut ptr = &state[..];
    while let Some((key, val)) = make_proc_acpi_key_val(&mut ptr) {
        if eq_ignore_case(key, "state") && eq_ignore_case(val, "on-line") {
            *have_ac = true;
        }
    }
}

/// The result of an implementation, from upstream's `int` out-parameters
/// (`-1`, or any negative value, is unknown).
fn power_info(state: PowerState, seconds: i32, percent: i32) -> PowerInfo {
    PowerInfo {
        state,
        time_left: u64::try_from(seconds)
            .ok()
            .map(std::time::Duration::from_secs),
        percent: u8::try_from(percent).ok(),
    }
}

/// Translation of `SDL_GetPowerInfo_Linux_proc_acpi()`.
pub(super) fn get_power_info_linux_proc_acpi() -> Option<PowerInfo> {
    let mut acc = AcpiBatteries {
        have_battery: false,
        charging: false,
        seconds: -1,
        percent: -1,
    };
    let mut have_ac = false;

    let base = root_path(PROC_ACPI_BATTERY_PATH);
    // can't use this interface.
    for node in readdir_all(&base)? {
        check_proc_acpi_battery(&base, &node, &mut acc);
    }

    let base = root_path(PROC_ACPI_AC_ADAPTER_PATH);
    // can't use this interface.
    for node in readdir_all(&base)? {
        check_proc_acpi_ac_adapter(&base, &node, &mut have_ac);
    }

    let state = if !acc.have_battery {
        PowerState::NoBattery
    } else if acc.charging {
        PowerState::Charging
    } else if have_ac {
        PowerState::Charged
    } else {
        PowerState::OnBattery
    };

    Some(power_info(state, acc.seconds, acc.percent)) // definitive answer.
}

/// Translation of `next_string()`: the next space-separated word.
fn next_string<'a>(ptr: &mut &'a [u8]) -> Option<&'a [u8]> {
    let mut p = *ptr;

    while let [b' ', rest @ ..] = p {
        p = rest; // skip any spaces...
    }

    if p.is_empty() {
        return None;
    }

    let end = p
        .iter()
        .position(|&c| c == b' ' || c == b'\n')
        .unwrap_or(p.len());
    let s = &p[..end];
    p = &p[end..];
    if !p.is_empty() {
        p = &p[1..];
    }

    *ptr = p;
    Some(s)
}

/// Translation of `int_string()`: the whole word as an integer (base 0).
fn int_string(s: &[u8]) -> Option<i32> {
    let (val, endptr) = strtol_rest(s, 0);
    (!s.is_empty() && endptr.is_empty()).then_some(val)
}

/// Translation of `SDL_GetPowerInfo_Linux_proc_apm()`.
///
/// See http://lxr.linux.no/linux+v2.6.29/drivers/char/apm-emulation.c
pub(super) fn get_power_info_linux_proc_apm() -> Option<PowerInfo> {
    let mut need_details = false;

    // can't use this interface.
    let mut f = std::fs::File::open(&*root_path(PROC_APM_PATH)).ok()?;
    let buf = read_c_string(&mut f, 128)?;
    drop(f);

    let mut ptr = &buf[..];
    next_string(&mut ptr)?; // driver version
    next_string(&mut ptr)?; // BIOS version
    next_string(&mut ptr)?; // APM flags

    let ac_status = int_string(next_string(&mut ptr)?)?; // AC line status
    let _battery_status = int_string(next_string(&mut ptr)?)?; // battery status
    let battery_flag = int_string(next_string(&mut ptr)?)?; // battery flag

    let mut s = next_string(&mut ptr)?; // remaining battery life percent
    if let [rest @ .., b'%'] = s {
        s = rest;
    }
    let battery_percent = int_string(s)?;

    let mut battery_time = int_string(next_string(&mut ptr)?)?; // remaining battery life time

    // remaining battery life time units
    if eq_ignore_case(next_string(&mut ptr)?, "min") {
        battery_time = battery_time.wrapping_mul(60);
    }

    let state = if battery_flag == 0xFF {
        // unknown state
        PowerState::Unknown
    } else if battery_flag & (1 << 7) != 0 {
        // no battery
        PowerState::NoBattery
    } else if battery_flag & (1 << 3) != 0 {
        // charging
        need_details = true;
        PowerState::Charging
    } else if ac_status == 1 {
        need_details = true;
        PowerState::Charged // on AC, not charging.
    } else {
        need_details = true;
        PowerState::OnBattery
    };

    let mut percent = -1;
    let mut seconds = -1;
    if need_details {
        let pct = battery_percent;
        let secs = battery_time;

        if pct >= 0 {
            // -1 == unknown
            percent = pct.min(100); // clamp between 0%, 100%
        }
        if secs >= 0 {
            // -1 == unknown
            seconds = secs;
        }
    }

    Some(power_info(state, seconds, percent))
}

/// The best battery so far, as the `*state`, `*seconds` and `*percent`
/// out-parameters the sysfs and UPower scans update.
struct Best {
    state: PowerState,
    seconds: i32,
    percent: i32,
}

impl Best {
    /// "We pick the battery that claims to have the most minutes left."
    fn consider(&mut self, st: PowerState, secs: i32, pct: i32) {
        let mut choose = false;

        /*
         * We pick the battery that claims to have the most minutes left.
         *  (failing a report of minutes, we'll take the highest percent.)
         */
        if secs < 0 && self.seconds < 0 {
            if (pct < 0 && self.percent < 0) || pct > self.percent {
                choose = true; // at least we know there's a battery.
            }
        } else if secs > self.seconds {
            choose = true;
        }

        if choose {
            self.seconds = secs;
            self.percent = pct;
            self.state = st;
        }
    }

    fn info(&self) -> PowerInfo {
        power_info(self.state, self.seconds, self.percent)
    }
}

/// Translation of `SDL_GetPowerInfo_Linux_sys_class_power_supply()`.
pub(super) fn get_power_info_linux_sys_class_power_supply() -> Option<PowerInfo> {
    let base = root_path(SYS_CLASS_POWER_SUPPLY_PATH);
    let dirp = std::fs::read_dir(&*base).ok()?;

    let mut best = Best {
        state: PowerState::NoBattery, // assume we're just plugged in.
        seconds: -1,
        percent: -1,
    };

    // (std never reports "." and "..", which upstream skips.)
    for dent in dirp {
        let Ok(dent) = dent else {
            break;
        };
        let name = dent.file_name();
        let name = name.to_string_lossy();
        let read = |key: &str| read_power_file(&base, &name, key, 64);

        let Some(ty) = read("type") else {
            continue; // Don't know _what_ we're looking at. Give up on it.
        };
        if !eq_ignore_case(&ty, "Battery\n") {
            continue; // we don't care about UPS and such.
        }

        /* if the scope is "device," it might be something like a PS4
        controller reporting its own battery, and not something that powers
        the system. Most system batteries don't list a scope at all; we
        assume it's a system battery if not specified. */
        if read("scope").is_some_and(|s| eq_ignore_case(&s, "Device\n")) {
            continue; // skip external devices with their own batteries.
        }

        // some drivers don't offer this, so if it's not explicitly reported assume it's present.
        let st = if read("present").is_some_and(|s| s == b"0\n") {
            PowerState::NoBattery
        } else {
            match read("status") {
                None => PowerState::Unknown, // uh oh
                Some(s) if eq_ignore_case(&s, "Charging\n") => PowerState::Charging,
                Some(s) if eq_ignore_case(&s, "Discharging\n") => PowerState::OnBattery,
                Some(s) if eq_ignore_case(&s, "Full\n") || eq_ignore_case(&s, "Not charging\n") => {
                    PowerState::Charged
                }
                Some(_) => PowerState::Unknown, // uh oh
            }
        };

        let pct = match read("capacity") {
            None => -1,
            Some(s) => atoi(&s).min(100), // clamp between 0%, 100%
        };

        let secs = if let Some(s) = read("time_to_empty_now") {
            let secs = atoi(&s);
            if secs <= 0 {
                -1 // 0 == unknown
            } else {
                secs
            }
        } else if st == PowerState::OnBattery {
            /* energy is Watt*hours and power is Watts */
            let energy = read("energy_now").map_or(-1, |s| atoi(&s));
            let power = read("power_now").map_or(-1, |s| atoi(&s));
            if energy >= 0 && power > 0 {
                ((3600i64 * energy as i64) / power as i64) as i32
            } else {
                -1
            }
        } else {
            -1
        };

        best.consider(st, secs, pct);
    }

    Some(best.info()) // don't look any further.
}

// d-bus queries to org.freedesktop.UPower.
const UPOWER_DBUS_NODE: &str = "org.freedesktop.UPower";
const UPOWER_DBUS_PATH: &str = "/org/freedesktop/UPower";
const UPOWER_DBUS_INTERFACE: &str = "org.freedesktop.UPower";
const UPOWER_DEVICE_DBUS_INTERFACE: &str = "org.freedesktop.UPower.Device";

/// Translation of `check_upower_device()`.
fn check_upower_device(conn: &Connection, path: &str, best: &mut Best) {
    let query = |property: &str| {
        dbus::query_property_on_connection(
            conn,
            UPOWER_DBUS_NODE,
            path,
            UPOWER_DEVICE_DBUS_INTERFACE,
            property,
        )
    };
    let query_u32 = |property: &str| match query(property) {
        Some(Value::U32(v)) => Some(v),
        _ => None,
    };
    let query_bool = |property: &str| match query(property) {
        Some(Value::Bool(v)) => Some(v),
        _ => None,
    };

    match query_u32("Type") {
        None => return,    // Don't know _what_ we're looking at. Give up on it.
        Some(2) => {}      // 2==Battery
        Some(_) => return, // we don't care about UPS and such.
    }
    match query_bool("PowerSupply") {
        None => return,
        Some(false) => return, // we don't care about random devices with batteries, like wireless controllers, etc
        Some(true) => {}
    }

    let Some(is_present) = query_bool("IsPresent") else {
        return;
    };
    let st = if !is_present {
        PowerState::NoBattery
    } else {
        /* Get updated information on the battery status
         * This can occasionally fail, and we'll just return slightly stale data in that case
         */
        dbus::call_method_on_connection(
            conn,
            UPOWER_DBUS_NODE,
            path,
            UPOWER_DEVICE_DBUS_INTERFACE,
            "Refresh",
            &[],
        );

        match query_u32("State") {
            None => PowerState::Unknown,     // uh oh
            Some(1) => PowerState::Charging, // 1 == charging
            /* 2 == discharging;
             * 3 == empty;
             * 6 == "pending discharge" which GNOME interprets as equivalent
             * to discharging */
            Some(2 | 3 | 6) => PowerState::OnBattery,
            /* 4 == full;
             * 5 == "pending charge" which GNOME shows as "Not charging",
             * used when a battery is configured to stop charging at a
             * lower than 100% threshold */
            Some(4 | 5) => PowerState::Charged,
            Some(_) => PowerState::Unknown, // uh oh
        }
    };

    let pct = match query("Percentage") {
        Some(Value::F64(d)) => (d as i32).min(100), // clamp between 0%, 100%
        _ => -1, // some old/cheap batteries don't set this property.
    };

    let secs = match query("TimeToEmpty") {
        Some(Value::I64(si64)) => {
            let secs = si64 as i32;
            if secs <= 0 {
                -1 // 0 == unknown
            } else {
                secs
            }
        }
        _ => -1,
    };

    best.consider(st, secs, pct);
}

/// Translation of `SDL_GetPowerInfo_Linux_org_freedesktop_upower()`.
pub(super) fn get_power_info_linux_org_freedesktop_upower() -> Option<PowerInfo> {
    // try a different approach than UPower.
    let ctx = dbus::context()?;
    // (SDL_DBus_CallMethodOnConnection() fails without a system bus.)
    let conn = ctx.system_conn.as_ref()?;

    let call = |method: &str| {
        dbus::call_method_on_connection(
            conn,
            UPOWER_DBUS_NODE,
            UPOWER_DBUS_PATH,
            UPOWER_DBUS_INTERFACE,
            method,
            &[] as &[Arg<'_>],
        )
        .map(|reply| reply.args())
    };

    let display_device = call("GetDisplayDevice").and_then(|args| match args.into_iter().next() {
        Some(Value::ObjectPath(path)) => Some(path),
        _ => None,
    });
    // Clearly we can use this interface.
    let mut best = Best {
        state: PowerState::NoBattery, // assume we're just plugged in.
        seconds: -1,
        percent: -1,
    };
    if let Some(path) = display_device {
        check_upower_device(conn, &path, &mut best);
    } else {
        let paths = call("EnumerateDevices").and_then(|args| match args.into_iter().next() {
            Some(Value::Array(paths))
                if paths.iter().all(|p| matches!(p, Value::ObjectPath(_))) =>
            {
                Some(paths)
            }
            _ => None,
        })?;
        for path in &paths {
            if let Value::ObjectPath(path) = path {
                check_upower_device(conn, path, &mut best);
            }
        }
    }
    Some(best.info())
}
