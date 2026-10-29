// `adb shell /data/local/tmp/spike/uinput_pad SECONDS` (as root): a uinput
// gamepad (an Xbox 360 pad's ids and buttons) that presses A and moves the
// left stick every 500 ms, then goes away.

use std::fs::OpenOptions;
use std::io::Write;
use std::os::fd::AsRawFd;
use std::time::{Duration, Instant};

unsafe extern "C" {
    fn ioctl(fd: i32, req: u64, ...) -> i32;
}

const UI_SET_EVBIT: u64 = 0x4004_5564;
const UI_SET_KEYBIT: u64 = 0x4004_5565;
const UI_SET_ABSBIT: u64 = 0x4004_5567;
const UI_DEV_CREATE: u64 = 0x5501;
const UI_DEV_DESTROY: u64 = 0x5502;
const EV_SYN: u16 = 0;
const EV_KEY: u16 = 1;
const EV_ABS: u16 = 3;

fn ev(f: &mut std::fs::File, t: u16, code: u16, value: i32) {
    let mut b = [0u8; 24];
    b[16..18].copy_from_slice(&t.to_le_bytes());
    b[18..20].copy_from_slice(&code.to_le_bytes());
    b[20..24].copy_from_slice(&value.to_le_bytes());
    f.write_all(&b).unwrap();
}

fn main() {
    let secs: u64 = std::env::args().nth(1).and_then(|s| s.parse().ok()).unwrap_or(10);
    let mut f = match OpenOptions::new().write(true).open("/dev/uinput") {
        Ok(f) => f,
        Err(e) => {
            println!("uinput_pad: open /dev/uinput failed: {e}");
            std::process::exit(1);
        }
    };
    let fd = f.as_raw_fd();
    let keys = [0x130u16, 0x131, 0x133, 0x134, 0x136, 0x137, 0x13a, 0x13b, 0x13c, 0x13d, 0x13e]; // A B X Y TL TR SELECT START MODE THUMBL THUMBR
    let axes = [0u16, 1, 2, 3, 4, 5, 0x10, 0x11]; // X Y Z RX RY RZ HAT0X HAT0Y
    unsafe {
        ioctl(fd, UI_SET_EVBIT, EV_KEY as i32);
        ioctl(fd, UI_SET_EVBIT, EV_ABS as i32);
        for k in keys {
            ioctl(fd, UI_SET_KEYBIT, k as i32);
        }
        for a in axes {
            ioctl(fd, UI_SET_ABSBIT, a as i32);
        }
    }
    // struct uinput_user_dev: name[80], input_id (bustype, vendor, product,
    // version: u16 each), ff_effects_max u32, absmax/absmin/absfuzz/absflat [i32; 64]
    let mut d = vec![0u8; 80 + 8 + 4 + 4 * 64 * 4];
    let name = b"SDL3 Spike uinput Pad";
    d[..name.len()].copy_from_slice(name);
    for (i, v) in [3u16, 0x045e, 0x028e, 0x0110].iter().enumerate() {
        d[80 + 2 * i..82 + 2 * i].copy_from_slice(&v.to_le_bytes());
    }
    for a in axes {
        let (min, max) = match a {
            2 | 5 => (0, 255),
            0x10 | 0x11 => (-1, 1),
            _ => (-32768, 32767),
        };
        let o_max = 92 + 4 * a as usize;
        let o_min = 92 + 256 + 4 * a as usize;
        d[o_max..o_max + 4].copy_from_slice(&(max as i32).to_le_bytes());
        d[o_min..o_min + 4].copy_from_slice(&(min as i32).to_le_bytes());
    }
    f.write_all(&d).unwrap();
    let r = unsafe { ioctl(fd, UI_DEV_CREATE) };
    println!("uinput_pad: UI_DEV_CREATE = {r}");
    let t = Instant::now();
    let mut i = 0;
    while t.elapsed() < Duration::from_secs(secs) {
        std::thread::sleep(Duration::from_millis(500));
        i += 1;
        ev(&mut f, EV_KEY, 0x130, (i % 2) as i32);
        ev(&mut f, EV_ABS, 0, if i % 2 == 1 { 30000 } else { 0 });
        ev(&mut f, EV_SYN, 0, 0);
    }
    unsafe { ioctl(fd, UI_DEV_DESTROY) };
    println!("uinput_pad: destroyed after {i} toggles");
}
