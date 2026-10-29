//! AAudio and OpenSL ES probes.

use crate::dl::Lib;
use crate::{cstr, f, say};
use std::ffi::{c_char, c_void};
use std::time::{Duration, Instant};

type P = *mut c_void;

/// Open an AAudio stream (`input`: recording), run it for about a second,
/// and report what was negotiated.
pub fn aaudio(label: &str, input: bool) -> Result<(), String> {
    let l = Lib::open("libaaudio.so")?;
    let create = f!(l, "AAudio_createStreamBuilder", fn(*mut P) -> i32);
    let text = f!(l, "AAudio_convertResultToText", fn(i32) -> *const c_char);
    let set_dir = f!(l, "AAudioStreamBuilder_setDirection", fn(P, i32));
    let set_rate = f!(l, "AAudioStreamBuilder_setSampleRate", fn(P, i32));
    let set_ch = f!(l, "AAudioStreamBuilder_setChannelCount", fn(P, i32));
    let set_fmt = f!(l, "AAudioStreamBuilder_setFormat", fn(P, i32));
    let set_perf = f!(l, "AAudioStreamBuilder_setPerformanceMode", fn(P, i32));
    let open = f!(l, "AAudioStreamBuilder_openStream", fn(P, *mut P) -> i32);
    let del = f!(l, "AAudioStreamBuilder_delete", fn(P) -> i32);
    let g_rate = f!(l, "AAudioStream_getSampleRate", fn(P) -> i32);
    let g_ch = f!(l, "AAudioStream_getChannelCount", fn(P) -> i32);
    let g_fmt = f!(l, "AAudioStream_getFormat", fn(P) -> i32);
    let g_burst = f!(l, "AAudioStream_getFramesPerBurst", fn(P) -> i32);
    let g_cap = f!(l, "AAudioStream_getBufferCapacityInFrames", fn(P) -> i32);
    let g_dev = f!(l, "AAudioStream_getDeviceId", fn(P) -> i32);
    let g_perf = f!(l, "AAudioStream_getPerformanceMode", fn(P) -> i32);
    let g_share = f!(l, "AAudioStream_getSharingMode", fn(P) -> i32);
    let start = f!(l, "AAudioStream_requestStart", fn(P) -> i32);
    let stop = f!(l, "AAudioStream_requestStop", fn(P) -> i32);
    let close = f!(l, "AAudioStream_close", fn(P) -> i32);
    let write = f!(l, "AAudioStream_write", fn(P, *const c_void, i32, i64) -> i32);
    let read = f!(l, "AAudioStream_read", fn(P, *mut c_void, i32, i64) -> i32);
    let xruns = f!(l, "AAudioStream_getXRunCount", fn(P) -> i32);
    let state = f!(l, "AAudioStream_getState", fn(P) -> i32);
    let dir = if input { "input" } else { "output" };
    unsafe {
        let mut b: P = std::ptr::null_mut();
        let r = create(&mut b);
        if r != 0 {
            return Err(format!("createStreamBuilder {}", cstr(text(r))));
        }
        set_dir(b, input as i32);
        set_rate(b, 48000);
        set_ch(b, if input { 1 } else { 2 });
        set_fmt(b, 2); // float
        set_perf(b, 12); // low latency
        let mut s: P = std::ptr::null_mut();
        let t0 = Instant::now();
        let r = open(b, &mut s);
        del(b);
        if r != 0 {
            say!("[{label}] AAudio {dir}: openStream failed: {} ({r}) after {:?}", cstr(text(r)), t0.elapsed());
            return Ok(());
        }
        say!("[{label}] AAudio {dir}: opened in {:?}: rate={} channels={} format={} burst={} capacity={} device={} perf={} sharing={}",
            t0.elapsed(), g_rate(s), g_ch(s), g_fmt(s), g_burst(s), g_cap(s), g_dev(s), g_perf(s), g_share(s));
        let r = start(s);
        say!("[{label}] AAudio {dir}: requestStart = {} state={}", cstr(text(r)), state(s));
        let ch = g_ch(s) as usize;
        let rate = g_rate(s).max(1) as usize;
        let frames = 480usize;
        let mut buf = vec![0f32; frames * ch];
        let t1 = Instant::now();
        let (mut total, mut calls, mut peak) = (0i64, 0, 0f32);
        let mut first_err = None;
        while t1.elapsed() < Duration::from_millis(1000) {
            let n = if input {
                read(s, buf.as_mut_ptr().cast(), frames as i32, 100_000_000)
            } else {
                for (i, x) in buf.iter_mut().enumerate() {
                    let t = (total as usize + i / ch) as f32 / rate as f32;
                    *x = 0.1 * (t * 440.0 * std::f32::consts::TAU).sin();
                }
                write(s, buf.as_ptr().cast(), frames as i32, 100_000_000)
            };
            calls += 1;
            if n < 0 {
                first_err.get_or_insert(cstr(text(n)));
                std::thread::sleep(Duration::from_millis(20));
                continue;
            }
            if input {
                for x in &buf[..n as usize * ch] {
                    peak = peak.max(x.abs());
                }
            }
            total += n as i64;
        }
        let secs = t1.elapsed().as_secs_f64();
        say!("[{label}] AAudio {dir}: {total} frames in {secs:.2}s ({:.0} frames/s) calls={calls} xruns={} first_error={first_err:?}{}",
            total as f64 / secs, xruns(s), if input { format!(" peak={peak}") } else { String::new() });
        stop(s);
        close(s);
    }
    Ok(())
}

unsafe fn vt<T: Copy>(itf: P, idx: usize) -> T {
    // An SL interface is a pointer to a pointer to a table of functions.
    let table = unsafe { *(itf as *const *const P) };
    unsafe { std::mem::transmute_copy(&*table.add(idx)) }
}

#[repr(C)]
struct DataSource {
    locator: *mut c_void,
    format: *mut c_void,
}

/// OpenSL ES: engine, output mix, a buffer-queue player; checks that the
/// enqueued buffers are consumed.
pub fn opensles(label: &str) -> Result<(), String> {
    let l = Lib::open("libOpenSLES.so")?;
    let create = f!(l, "slCreateEngine", fn(*mut P, u32, *const c_void, u32, *const P, *const u32) -> u32);
    let iid = |n: &str| -> Result<P, String> {
        let p = l.raw(n);
        if p.is_null() { Err(format!("no {n}")) } else { Ok(unsafe { *(p as *const P) }) }
    };
    let iid_engine = iid("SL_IID_ENGINE")?;
    let iid_play = iid("SL_IID_PLAY")?;
    let iid_bq = iid("SL_IID_ANDROIDSIMPLEBUFFERQUEUE")?;
    unsafe {
        let mut eng: P = std::ptr::null_mut();
        let r = create(&mut eng, 0, std::ptr::null(), 0, std::ptr::null(), std::ptr::null());
        if r != 0 {
            return Err(format!("slCreateEngine = {r}"));
        }
        let realize: unsafe extern "C" fn(P, u32) -> u32 = vt(eng, 0);
        let r = realize(eng, 0);
        say!("[{label}] OpenSL ES: engine created, Realize = {r}");
        let get_itf: unsafe extern "C" fn(P, P, *mut P) -> u32 = vt(eng, 3);
        let mut ei: P = std::ptr::null_mut();
        let r = get_itf(eng, iid_engine, &mut ei);
        if r != 0 {
            return Err(format!("GetInterface(ENGINE) = {r}"));
        }
        let create_mix: unsafe extern "C" fn(P, *mut P, u32, *const P, *const u32) -> u32 = vt(ei, 7);
        let mut mix: P = std::ptr::null_mut();
        let r = create_mix(ei, &mut mix, 0, std::ptr::null(), std::ptr::null());
        let r2 = if r == 0 { vt::<unsafe extern "C" fn(P, u32) -> u32>(mix, 0)(mix, 0) } else { 99 };
        say!("[{label}] OpenSL ES: output mix CreateOutputMix={r} Realize={r2}");
        let mut loc_bq = [0x800007BDu32, 2];
        let mut fmt = [2u32, 2, 48_000_000, 16, 16, 3, 2]; // PCM, 2 ch, 48 kHz, s16, FL|FR, little-endian
        #[repr(C)]
        struct LocMix {
            t: u32,
            mix: P,
        }
        let mut loc_mix = LocMix { t: 4, mix };
        let src = DataSource { locator: loc_bq.as_mut_ptr().cast(), format: fmt.as_mut_ptr().cast() };
        let snk = DataSource { locator: (&mut loc_mix as *mut LocMix).cast(), format: std::ptr::null_mut() };
        let create_player: unsafe extern "C" fn(P, *mut P, *const DataSource, *const DataSource, u32, *const P, *const u32) -> u32 = vt(ei, 2);
        let mut pl: P = std::ptr::null_mut();
        let ids = [iid_bq];
        let req = [1u32];
        let r = create_player(ei, &mut pl, &src, &snk, 1, ids.as_ptr(), req.as_ptr());
        if r != 0 {
            say!("[{label}] OpenSL ES: CreateAudioPlayer = {r}");
            return Ok(());
        }
        let r = vt::<unsafe extern "C" fn(P, u32) -> u32>(pl, 0)(pl, 0);
        let mut play: P = std::ptr::null_mut();
        let mut bq: P = std::ptr::null_mut();
        let ra = vt::<unsafe extern "C" fn(P, P, *mut P) -> u32>(pl, 3)(pl, iid_play, &mut play);
        let rb = vt::<unsafe extern "C" fn(P, P, *mut P) -> u32>(pl, 3)(pl, iid_bq, &mut bq);
        say!("[{label}] OpenSL ES: player Realize={r} PLAY={ra} BUFFERQUEUE={rb}");
        if ra != 0 || rb != 0 {
            return Ok(());
        }
        let buf: Vec<i16> = (0..4800 * 2).map(|i| (((i / 2) as f32 * 440.0 * std::f32::consts::TAU / 48000.0).sin() * 3000.0) as i16).collect();
        let enqueue: unsafe extern "C" fn(P, *const c_void, u32) -> u32 = vt(bq, 0);
        let get_state: unsafe extern "C" fn(P, *mut [u32; 2]) -> u32 = vt(bq, 2);
        let e1 = enqueue(bq, buf.as_ptr().cast(), (buf.len() * 2) as u32);
        let e2 = enqueue(bq, buf.as_ptr().cast(), (buf.len() * 2) as u32);
        let set_state: unsafe extern "C" fn(P, u32) -> u32 = vt(play, 0);
        let rp = set_state(play, 3);
        let t = Instant::now();
        let mut st = [0u32; 2];
        while t.elapsed() < Duration::from_secs(3) {
            get_state(bq, &mut st);
            if st[0] == 0 {
                break;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        say!("[{label}] OpenSL ES: Enqueue={e1},{e2} SetPlayState(PLAYING)={rp}; queue count={} index={} after {:?} (2 x 100 ms buffers)",
            st[0], st[1], t.elapsed());
        set_state(play, 1);
        vt::<unsafe extern "C" fn(P)>(pl, 6)(pl);
        vt::<unsafe extern "C" fn(P)>(mix, 6)(mix);
        vt::<unsafe extern "C" fn(P)>(eng, 6)(eng);
    }
    Ok(())
}
