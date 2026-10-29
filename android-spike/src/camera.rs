//! Camera2 NDK probe: the camera list and characteristics, then opening
//! each camera and capturing frames through an AImageReader.

use crate::dl::Lib;
use crate::{cstr, f, say};
use std::ffi::{c_char, c_void};
use std::time::{Duration, Instant};

type P = *mut c_void;

#[repr(C)]
struct IdList {
    n: i32,
    ids: *const *const c_char,
}

#[repr(C)]
struct Entry {
    tag: u32,
    typ: u8,
    count: u32,
    data: *const c_void,
}

#[repr(C)]
struct DevCallbacks {
    ctx: P,
    on_disconnected: unsafe extern "C" fn(P, P),
    on_error: unsafe extern "C" fn(P, P, i32),
}

#[repr(C)]
struct SessCallbacks {
    ctx: P,
    on_closed: unsafe extern "C" fn(P, P),
    on_ready: unsafe extern "C" fn(P, P),
    on_active: unsafe extern "C" fn(P, P),
}

unsafe extern "C" fn dev_disconnected(_: P, _: P) {
    say!("[camera] device onDisconnected");
}
unsafe extern "C" fn dev_error(_: P, _: P, e: i32) {
    say!("[camera] device onError {e}");
}
unsafe extern "C" fn sess_cb(_: P, _: P) {}

const ACAMERA_LENS_FACING: u32 = 0x80005;
const ACAMERA_SCALER_AVAILABLE_STREAM_CONFIGURATIONS: u32 = 0xD000A;
const ACAMERA_INFO_SUPPORTED_HARDWARE_LEVEL: u32 = 0x150000;
const AIMAGE_FORMAT_YUV_420_888: i32 = 0x23;

pub fn camera(label: &str, capture: bool) -> Result<(), String> {
    let c = Lib::open("libcamera2ndk.so")?;
    let m = Lib::open("libmediandk.so")?;
    let mgr_create = f!(c, "ACameraManager_create", fn() -> P);
    let get_ids = f!(c, "ACameraManager_getCameraIdList", fn(P, *mut *mut IdList) -> i32);
    let get_chars = f!(c, "ACameraManager_getCameraCharacteristics", fn(P, *const c_char, *mut P) -> i32);
    let get_entry = f!(c, "ACameraMetadata_getConstEntry", fn(P, u32, *mut Entry) -> i32);
    let meta_free = f!(c, "ACameraMetadata_free", fn(P));
    let open_cam = f!(c, "ACameraManager_openCamera", fn(P, *const c_char, *mut DevCallbacks, *mut P) -> i32);
    let dev_close = f!(c, "ACameraDevice_close", fn(P) -> i32);
    let create_req = f!(c, "ACameraDevice_createCaptureRequest", fn(P, i32, *mut P) -> i32);
    let cont_create = f!(c, "ACaptureSessionOutputContainer_create", fn(*mut P) -> i32);
    let cont_add = f!(c, "ACaptureSessionOutputContainer_add", fn(P, P) -> i32);
    let out_create = f!(c, "ACaptureSessionOutput_create", fn(P, *mut P) -> i32);
    let target_create = f!(c, "ACameraOutputTarget_create", fn(P, *mut P) -> i32);
    let req_add = f!(c, "ACaptureRequest_addTarget", fn(P, P) -> i32);
    let create_sess = f!(c, "ACameraDevice_createCaptureSession", fn(P, P, *const SessCallbacks, *mut P) -> i32);
    let repeating = f!(c, "ACameraCaptureSession_setRepeatingRequest", fn(P, P, i32, *mut P, *mut i32) -> i32);
    let stop_rep = f!(c, "ACameraCaptureSession_stopRepeating", fn(P) -> i32);
    let sess_close = f!(c, "ACameraCaptureSession_close", fn(P));
    let reader_new = f!(m, "AImageReader_new", fn(i32, i32, i32, i32, *mut P) -> i32);
    let reader_win = f!(m, "AImageReader_getWindow", fn(P, *mut P) -> i32);
    let acquire = f!(m, "AImageReader_acquireLatestImage", fn(P, *mut P) -> i32);
    let reader_del = f!(m, "AImageReader_delete", fn(P));
    let img_w = f!(m, "AImage_getWidth", fn(P, *mut i32) -> i32);
    let img_h = f!(m, "AImage_getHeight", fn(P, *mut i32) -> i32);
    let img_ts = f!(m, "AImage_getTimestamp", fn(P, *mut i64) -> i32);
    let img_plane = f!(m, "AImage_getPlaneData", fn(P, i32, *mut *mut u8, *mut i32) -> i32);
    let img_del = f!(m, "AImage_delete", fn(P));
    unsafe {
        let mgr = mgr_create();
        let mut list: *mut IdList = std::ptr::null_mut();
        let r = get_ids(mgr, &mut list);
        if r != 0 {
            say!("[{label}] camera: getCameraIdList = {r}");
            return Ok(());
        }
        let n = (*list).n;
        say!("[{label}] cameras: {n}");
        for i in 0..n as usize {
            let id = *(*list).ids.add(i);
            let mut meta: P = std::ptr::null_mut();
            let r = get_chars(mgr, id, &mut meta);
            let mut facing = -1;
            let mut level = -1;
            let mut sizes = vec![];
            if r == 0 {
                let mut e = Entry { tag: 0, typ: 0, count: 0, data: std::ptr::null() };
                if get_entry(meta, ACAMERA_LENS_FACING, &mut e) == 0 && e.count > 0 {
                    facing = *(e.data as *const u8) as i32;
                }
                if get_entry(meta, ACAMERA_INFO_SUPPORTED_HARDWARE_LEVEL, &mut e) == 0 && e.count > 0 {
                    level = *(e.data as *const u8) as i32;
                }
                if get_entry(meta, ACAMERA_SCALER_AVAILABLE_STREAM_CONFIGURATIONS, &mut e) == 0 {
                    let d = std::slice::from_raw_parts(e.data as *const i32, e.count as usize);
                    for c4 in d.chunks(4) {
                        if c4.len() == 4 && c4[0] == AIMAGE_FORMAT_YUV_420_888 && c4[3] == 0 {
                            sizes.push((c4[1], c4[2]));
                        }
                    }
                }
                meta_free(meta);
            }
            say!("[{label}] camera {:?}: characteristics={r} facing={facing} (0 front, 1 back, 2 external) hwlevel={level} yuv sizes={sizes:?}",
                cstr(id));
            if !capture {
                continue;
            }
            let mut cb = DevCallbacks { ctx: std::ptr::null_mut(), on_disconnected: dev_disconnected, on_error: dev_error };
            let mut dev: P = std::ptr::null_mut();
            let t = Instant::now();
            let r = open_cam(mgr, id, &mut cb, &mut dev);
            say!("[{label}] camera {:?}: openCamera = {r} ({}) in {:?}", cstr(id),
                if r == -10013 { "ACAMERA_ERROR_PERMISSION_DENIED" } else if r == 0 { "ok" } else { "error" }, t.elapsed());
            if r != 0 {
                continue;
            }
            let (w, h) = sizes.iter().copied().find(|s| s.0 <= 640).unwrap_or((640, 480));
            let mut reader: P = std::ptr::null_mut();
            let rr = reader_new(w, h, AIMAGE_FORMAT_YUV_420_888, 4, &mut reader);
            let mut win: P = std::ptr::null_mut();
            reader_win(reader, &mut win);
            let (mut cont, mut out, mut target, mut req, mut sess) = (std::ptr::null_mut(), std::ptr::null_mut(), std::ptr::null_mut(), std::ptr::null_mut(), std::ptr::null_mut());
            cont_create(&mut cont);
            out_create(win, &mut out);
            cont_add(cont, out);
            let scb = SessCallbacks { ctx: std::ptr::null_mut(), on_closed: sess_cb, on_ready: sess_cb, on_active: sess_cb };
            let rs = create_sess(dev, cont, &scb, &mut sess);
            let rq = create_req(dev, 1, &mut req);
            target_create(win, &mut target);
            req_add(req, target);
            let mut seq = 0;
            let rp = repeating(sess, std::ptr::null_mut(), 1, &mut req, &mut seq);
            say!("[{label}] camera {:?}: reader={rr} {w}x{h} session={rs} request={rq} repeating={rp}", cstr(id));
            let t = Instant::now();
            let mut frames = 0;
            let mut first = None;
            while t.elapsed() < Duration::from_secs(4) && frames < 10 {
                let mut img: P = std::ptr::null_mut();
                if acquire(reader, &mut img) == 0 {
                    frames += 1;
                    if first.is_none() {
                        let (mut iw, mut ih, mut ts) = (0, 0, 0i64);
                        img_w(img, &mut iw);
                        img_h(img, &mut ih);
                        img_ts(img, &mut ts);
                        let (mut data, mut len) = (std::ptr::null_mut(), 0);
                        img_plane(img, 0, &mut data, &mut len);
                        let y = std::slice::from_raw_parts(data, len.max(0) as usize);
                        let mean = y.iter().map(|&b| b as u64).sum::<u64>() as f64 / y.len().max(1) as f64;
                        first = Some(format!("{iw}x{ih} ts={ts} Y plane {len} bytes, mean luma {mean:.1}, after {:?}", t.elapsed()));
                    }
                    img_del(img);
                } else {
                    std::thread::sleep(Duration::from_millis(20));
                }
            }
            say!("[{label}] camera {:?}: {frames} frames in {:?}; first: {}", cstr(id), t.elapsed(), first.unwrap_or_default());
            stop_rep(sess);
            sess_close(sess);
            dev_close(dev);
            reader_del(reader);
        }
    }
    Ok(())
}
