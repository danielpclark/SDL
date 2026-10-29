//! ASensorManager probe: the sensor list, and accelerometer events (with an
//! `adb emu sensor set` injected by the host when `inject` is set).

use crate::dl::Lib;
use crate::{cstr, f, request, say};
use std::ffi::{c_char, c_void};
use std::time::{Duration, Instant};

type P = *mut c_void;

pub fn sensors(label: &str, inject: bool) -> Result<(), String> {
    let l = Lib::open("libandroid.so")?;
    let get_for_pkg = f!(l, "ASensorManager_getInstanceForPackage", fn(*const c_char) -> P);
    let get_list = f!(l, "ASensorManager_getSensorList", fn(P, *mut *const P) -> i32);
    let get_default = f!(l, "ASensorManager_getDefaultSensor", fn(P, i32) -> P);
    let name = f!(l, "ASensor_getName", fn(P) -> *const c_char);
    let vendor = f!(l, "ASensor_getVendor", fn(P) -> *const c_char);
    let typ = f!(l, "ASensor_getType", fn(P) -> i32);
    let stype = f!(l, "ASensor_getStringType", fn(P) -> *const c_char);
    let min_delay = f!(l, "ASensor_getMinDelay", fn(P) -> i32);
    let looper_prepare = f!(l, "ALooper_prepare", fn(i32) -> P);
    let create_q = f!(l, "ASensorManager_createEventQueue", fn(P, P, i32, P, P) -> P);
    let destroy_q = f!(l, "ASensorManager_destroyEventQueue", fn(P, P) -> i32);
    let enable = f!(l, "ASensorEventQueue_enableSensor", fn(P, P) -> i32);
    let disable = f!(l, "ASensorEventQueue_disableSensor", fn(P, P) -> i32);
    let set_rate = f!(l, "ASensorEventQueue_setEventRate", fn(P, P, i32) -> i32);
    let get_events = f!(l, "ASensorEventQueue_getEvents", fn(P, *mut u8, usize) -> isize);
    let poll = f!(l, "ALooper_pollOnce", fn(i32, *mut i32, *mut i32, *mut P) -> i32);
    unsafe {
        let m = get_for_pkg(c"org.libsdl.sdl3_rust.spike".as_ptr());
        if m.is_null() {
            return Err("ASensorManager_getInstanceForPackage = NULL".into());
        }
        let mut list: *const P = std::ptr::null();
        let n = get_list(m, &mut list);
        say!("[{label}] sensors: {n}");
        for i in 0..n.max(0) as usize {
            let s = *list.add(i);
            say!("[{label}]   sensor {i}: type={} {:?} name={:?} vendor={:?} minDelay={}us",
                typ(s), cstr(stype(s)), cstr(name(s)), cstr(vendor(s)), min_delay(s));
        }
        let acc = get_default(m, 1);
        if acc.is_null() {
            say!("[{label}] no default accelerometer");
            return Ok(());
        }
        let looper = looper_prepare(1);
        let q = create_q(m, looper, 3, std::ptr::null_mut(), std::ptr::null_mut());
        let r1 = enable(q, acc);
        let r2 = set_rate(q, acc, 20_000);
        say!("[{label}] accelerometer queue: enable={r1} setEventRate={r2}");
        let t = Instant::now();
        let mut count = 0;
        let mut last = [f32::NAN; 3];
        let mut asked = false;
        let mut ev = [0u8; 104 * 8];
        while t.elapsed() < Duration::from_secs(if inject { 8 } else { 3 }) {
            if inject && !asked && t.elapsed() > Duration::from_millis(1500) {
                asked = true;
                request("adb emu sensor set acceleration 1.5:2.5:3.5");
            }
            let (mut fd, mut events, mut data) = (0, 0, std::ptr::null_mut());
            poll(100, &mut fd, &mut events, &mut data);
            loop {
                let k = get_events(q, ev.as_mut_ptr(), 8);
                if k <= 0 {
                    break;
                }
                for j in 0..k as usize {
                    let e = &ev[j * 104..];
                    let v = |o: usize| f32::from_le_bytes(e[24 + 4 * o..28 + 4 * o].try_into().unwrap());
                    let cur = [v(0), v(1), v(2)];
                    count += 1;
                    if count == 1 || cur.iter().zip(last.iter()).any(|(a, b)| (a - b).abs() > 0.3) {
                        say!("[{label}] accel event {count} at {:.2}s: {:.3} {:.3} {:.3}", t.elapsed().as_secs_f64(), cur[0], cur[1], cur[2]);
                        last = cur;
                    }
                }
            }
        }
        say!("[{label}] accelerometer: {count} events in {:?}", t.elapsed());
        disable(q, acc);
        destroy_q(m, q);
    }
    Ok(())
}
