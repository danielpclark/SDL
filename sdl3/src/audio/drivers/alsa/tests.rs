// Tests for the ALSA driver: the channel map negotiation against a stand-in
// PCM, and real playback and recording through libasound's `file` and
// `null` PCMs (no sound card needed) when libasound is installed.

use super::*;
use crate::audio::{
    audio_device_name, current_audio_driver, playback_devices, AudioDevice, AudioStream,
    AUDIO_DEVICE_DEFAULT_PLAYBACK, AUDIO_DEVICE_DEFAULT_RECORDING,
};
use crate::init::InitFlags;
use crate::test_support::TempDir;
use std::cell::RefCell;

/// A PCM that reports `queries` and records what gets installed.
struct FakePcm {
    queries: Option<Vec<ChmapQuery>>,
    set_chmap_status: c_int,
    installed: RefCell<Vec<Vec<u32>>>,
}

impl FakePcm {
    fn new(queries: Option<Vec<(c_int, Vec<u32>)>>) -> FakePcm {
        FakePcm {
            queries: queries.map(|q| {
                q.into_iter()
                    .map(|(type_, pos)| ChmapQuery { type_, pos })
                    .collect()
            }),
            set_chmap_status: 0,
            installed: RefCell::new(Vec::new()),
        }
    }
}

impl ChmapTarget for FakePcm {
    fn query_chmaps(&self) -> Option<ChmapQueries> {
        self.queries.clone().map(|list| ChmapQueries { list })
    }
    fn set_chmap(&self, chmap: &[u32]) -> c_int {
        self.installed.borrow_mut().push(chmap.to_vec());
        self.set_chmap_status
    }
    fn strerror(&self, errnum: c_int) -> String {
        format!("error {errnum}")
    }
    fn chmap_print(&self, chmap: &[u32]) -> String {
        format!("{chmap:?}")
    }
}

/// Run `alsa_chmap_cfg()` for `chans_n` channels.
fn chmap_cfg(chans_n: u32, pcm: &FakePcm) -> (Result<i32>, PcmCfgCtx) {
    let mut ctx = PcmCfgCtx::new(
        AudioSpec::new(AudioFormat::S16, chans_n as i32, 48000),
        1024,
    );
    ctx.chans_n = chans_n;
    let r = alsa_chmap_cfg(&mut ctx, pcm);
    (r, ctx)
}

const NA: u32 = 1; // SND_CHMAP_NA

#[test]
fn sdl_channel_maps_match_upstream() {
    // The SND_CHMAP_* positions of <alsa/pcm.h>, and upstream's table.
    assert_eq!(
        [
            SND_CHMAP_UNKNOWN,
            NA,
            SND_CHMAP_MONO,
            SND_CHMAP_FL,
            SND_CHMAP_FR,
            SND_CHMAP_RL
        ],
        [0, 1, 2, 3, 4, 5]
    );
    assert_eq!(
        [
            SND_CHMAP_RR,
            SND_CHMAP_FC,
            SND_CHMAP_LFE,
            SND_CHMAP_SL,
            SND_CHMAP_SR,
            SND_CHMAP_RC
        ],
        [6, 7, 8, 9, 10, 11]
    );
    assert_eq!(SDL_CHANNEL_MAPS[1][..1], [2]);
    assert_eq!(SDL_CHANNEL_MAPS[2][..2], [3, 4]);
    assert_eq!(SDL_CHANNEL_MAPS[3][..3], [3, 4, 8]);
    assert_eq!(SDL_CHANNEL_MAPS[4][..4], [3, 4, 5, 6]);
    assert_eq!(SDL_CHANNEL_MAPS[5][..5], [3, 4, 8, 5, 6]);
    assert_eq!(SDL_CHANNEL_MAPS[6][..6], [3, 4, 7, 8, 0, 0]);
    assert_eq!(SDL_CHANNEL_MAPS[7][..7], [3, 4, 7, 8, 11, 9, 10]);
    assert_eq!(SDL_CHANNEL_MAPS[8], [3, 4, 7, 8, 5, 6, 9, 10]);
}

#[test]
fn formats_map_to_alsa_with_the_same_endianness() {
    assert_eq!(alsa_format_of(AudioFormat::U8), Some(1));
    assert_eq!(alsa_format_of(AudioFormat::S8), Some(0));
    assert_eq!(alsa_format_of(AudioFormat::S16LE), Some(2));
    assert_eq!(alsa_format_of(AudioFormat::S16BE), Some(3));
    assert_eq!(alsa_format_of(AudioFormat::S32LE), Some(10));
    assert_eq!(alsa_format_of(AudioFormat::S32BE), Some(11));
    assert_eq!(alsa_format_of(AudioFormat::F32LE), Some(14));
    assert_eq!(alsa_format_of(AudioFormat::F32BE), Some(15));
    assert_eq!(alsa_format_of(AudioFormat::UNKNOWN), None);
}

#[test]
fn six_channel_maps_reduce_to_rear_or_side() {
    let base = SDL_CHANNEL_MAPS[6];
    let reduce = |alsa: [u32; 6]| {
        let mut sdl = base;
        sdl_6chans_set_rear_or_side_channels_from_alsa_6chans(&mut sdl, &alsa);
        [sdl[4], sdl[5]]
    };
    let (fl, fr, fc, lfe) = (SND_CHMAP_FL, SND_CHMAP_FR, SND_CHMAP_FC, SND_CHMAP_LFE);
    let (rl, rr, sl, sr) = (SND_CHMAP_RL, SND_CHMAP_RR, SND_CHMAP_SL, SND_CHMAP_SR);
    assert_eq!(reduce([fl, fr, fc, lfe, rl, rr]), [rl, rr]);
    assert_eq!(reduce([fl, fr, rl, rr, fc, lfe]), [rl, rr]);
    assert_eq!(reduce([fl, fr, fc, lfe, sl, sr]), [sl, sr]);
    // Both rear and side, or neither: unsupported.
    assert_eq!(reduce([fl, fr, fc, lfe, sl, rr]), [0, 0]);
    assert_eq!(reduce([fl, fr, fc, lfe, NA, NA]), [0, 0]);
    // Missing one of the four fronts: unsupported.
    assert_eq!(reduce([fl, fr, rl, lfe, sl, sr]), [0, 0]);
    assert!(has_pos(&[1, 2, 3, 4, 5, 6], 6));
    assert!(
        !has_pos(&[1, 2, 3, 4, 5, 6, 7], 7),
        "only six positions are scanned"
    );
}

#[test]
fn channel_map_negotiation() {
    let (fl, fr, fc, lfe) = (SND_CHMAP_FL, SND_CHMAP_FR, SND_CHMAP_FC, SND_CHMAP_LFE);
    let (rl, rr) = (SND_CHMAP_RL, SND_CHMAP_RR);

    // No channel map support: installed as-is, no swizzle.
    let pcm = FakePcm::new(None);
    let (r, ctx) = chmap_cfg(2, &pcm);
    assert_eq!(r.unwrap(), CHMAP_INSTALLED);
    assert!(ctx.device_chmap.is_none() && pcm.installed.borrow().is_empty());

    // An exact fixed map is installed, no swizzle.
    let pcm = FakePcm::new(Some(vec![(SND_CHMAP_TYPE_FIXED, vec![fl, fr])]));
    let (r, ctx) = chmap_cfg(2, &pcm);
    assert_eq!(r.unwrap(), CHMAP_INSTALLED);
    assert_eq!(*pcm.installed.borrow(), [vec![fl, fr]]);
    assert_eq!(ctx.alsa_chmap_installed[..2], [fl, fr]);
    assert!(ctx.device_chmap.is_none());

    // A swapped fixed map is installed and SDL swizzles.
    let pcm = FakePcm::new(Some(vec![(SND_CHMAP_TYPE_PAIRED, vec![fr, fl])]));
    let (r, ctx) = chmap_cfg(2, &pcm);
    assert_eq!(r.unwrap(), CHMAP_INSTALLED);
    assert_eq!(*pcm.installed.borrow(), [vec![fr, fl]]);
    assert_eq!(ctx.device_chmap, Some(vec![1, 0]));

    // A VAR map gets SDL's own order programmed.
    let pcm = FakePcm::new(Some(vec![(SND_CHMAP_TYPE_VAR, vec![fr, fl])]));
    let (r, ctx) = chmap_cfg(2, &pcm);
    assert_eq!(r.unwrap(), CHMAP_INSTALLED);
    assert_eq!(*pcm.installed.borrow(), [vec![fl, fr]]);
    assert!(ctx.device_chmap.is_none());

    // 5.1 with rear channels in ALSA's order: reduce to rear, swizzle.
    let pcm = FakePcm::new(Some(vec![(
        SND_CHMAP_TYPE_FIXED,
        vec![fl, fr, rl, rr, fc, lfe],
    )]));
    let (r, ctx) = chmap_cfg(6, &pcm);
    assert_eq!(r.unwrap(), CHMAP_INSTALLED);
    assert_eq!(ctx.sdl_chmap[..6], [fl, fr, fc, lfe, rl, rr]);
    assert_eq!(ctx.device_chmap, Some(vec![0, 1, 4, 5, 2, 3]));

    // The wrong channel count, duplicate positions or no match: try the next count.
    for queries in [
        vec![(SND_CHMAP_TYPE_FIXED, vec![fl, fr, fc])],
        vec![(SND_CHMAP_TYPE_FIXED, vec![NA, NA])],
        vec![(SND_CHMAP_TYPE_FIXED, vec![rl, rr])],
        vec![(0, vec![fl, fr])], // SND_CHMAP_TYPE_NONE
    ] {
        let pcm = FakePcm::new(Some(queries));
        let (r, _) = chmap_cfg(2, &pcm);
        assert_eq!(r.unwrap(), CHANS_N_NEXT);
        assert!(pcm.installed.borrow().is_empty());
    }

    // Installation failures are errors.
    let mut pcm = FakePcm::new(Some(vec![(SND_CHMAP_TYPE_FIXED, vec![fl, fr])]));
    pcm.set_chmap_status = -22;
    let (r, _) = chmap_cfg(2, &pcm);
    assert_eq!(
        r.unwrap_err().message(),
        "ALSA: failed to install channel map: error -22"
    );
}

#[test]
fn pcm_names() {
    let _l = crate::test_support::test_lock();
    for h in [
        crate::hints::AUDIO_ALSA_DEFAULT_DEVICE,
        crate::hints::AUDIO_ALSA_DEFAULT_PLAYBACK_DEVICE,
        crate::hints::AUDIO_ALSA_DEFAULT_RECORDING_DEVICE,
    ] {
        crate::hints::reset(h);
    }
    assert_eq!(get_pcm_str(&default_playback_handle()), "default");
    crate::hints::set(crate::hints::AUDIO_ALSA_DEFAULT_DEVICE, "dmix").unwrap();
    assert_eq!(get_pcm_str(&default_recording_handle()), "dmix");
    crate::hints::set(crate::hints::AUDIO_ALSA_DEFAULT_RECORDING_DEVICE, "dsnoop").unwrap();
    assert_eq!(get_pcm_str(&default_recording_handle()), "dsnoop");
    assert_eq!(get_pcm_str(&default_playback_handle()), "dmix");
    for h in [
        crate::hints::AUDIO_ALSA_DEFAULT_DEVICE,
        crate::hints::AUDIO_ALSA_DEFAULT_RECORDING_DEVICE,
    ] {
        crate::hints::reset(h);
    }

    let dev = AlsaDevice {
        id: "PCH".to_owned(),
        device_index: 3,
        name: "HDA Intel PCH:HDMI 0".to_owned(),
        recording: false,
        handle: 7,
    };
    let name = get_pcm_str(&dev);
    assert!(name.ends_with("CARD=PCH,DEV=3"), "{name}");
}

unsafe extern "C" fn fake_hw_params_any(_pcm: *mut SndPcm, _hw: *mut SndPcmHwParams) -> c_int {
    0
}

unsafe extern "C" fn fake_hw_params_set_access(
    _pcm: *mut SndPcm,
    _hw: *mut SndPcmHwParams,
    _access: c_int,
) -> c_int {
    0
}

unsafe extern "C" fn fake_hw_params_set_format_busy(
    _pcm: *mut SndPcm,
    _hw: *mut SndPcmHwParams,
    _format: c_int,
) -> c_int {
    -libc::EBUSY
}

#[test]
fn unsupported_format_reports_the_format_error() {
    let _l = crate::test_support::test_lock();
    if !have_libasound() {
        return;
    }
    // A PCM that takes any configuration and access mode, but no format
    let mut lib = load_alsa_library().unwrap();
    lib.snd_pcm_hw_params_any = fake_hw_params_any;
    lib.snd_pcm_hw_params_set_access = fake_hw_params_set_access;
    lib.snd_pcm_hw_params_set_format = fake_hw_params_set_format_busy;
    let target = PcmTarget {
        lib: &lib,
        pcm: ptr::null_mut(),
    };
    let mut ctx = PcmCfgCtx::new(AudioSpec::new(AudioFormat::S16, 2, 48000), 1024);
    ctx.hwparams = AlsaAlloc::new(64);
    let e = alsa_pcm_cfg_hw_chans_n_scan(
        &mut ctx,
        &target,
        CHANS_N_SCAN_MODE_EQUAL_OR_ABOVE_REQUESTED_CHANS_N,
    )
    .unwrap_err();
    assert_eq!(
        e.message(),
        format!(
            "ALSA: Unsupported audio format: {}",
            lib.strerror(-libc::EBUSY)
        )
    );
}

/// The hint list the fake `snd_device_name_hint()` hands out, and the
/// list the fake `snd_device_name_free_hint()` was given.
static FAKE_HINTS: AtomicUsize = AtomicUsize::new(0);
static FREED_HINTS: AtomicUsize = AtomicUsize::new(0);

unsafe extern "C" fn fake_name_hint(
    _card: c_int,
    _iface: *const c_char,
    hints: *mut *mut *mut c_void,
) -> c_int {
    // SAFETY: the caller passes a writable list pointer.
    unsafe { *hints = FAKE_HINTS.load(Ordering::SeqCst) as *mut *mut c_void };
    0
}

unsafe extern "C" fn fake_name_get_hint(_hint: *const c_void, _id: *const c_char) -> *mut c_char {
    ptr::null_mut()
}

unsafe extern "C" fn fake_name_free_hint(hints: *mut *mut c_void) -> c_int {
    FREED_HINTS.store(hints as usize, Ordering::SeqCst);
    0
}

#[test]
fn device_prefix_guess_frees_the_hint_list() {
    let _l = crate::test_support::test_lock();
    if !have_libasound() {
        return;
    }
    let mut lib = load_alsa_library().unwrap();
    lib.snd_device_name_hint = fake_name_hint;
    lib.snd_device_name_get_hint = fake_name_get_hint;
    lib.snd_device_name_free_hint = fake_name_free_hint;
    // One hint without a name, then the terminating NULL
    let mut hint = 0u8;
    let mut list: [*mut c_void; 2] = [(&mut hint as *mut u8).cast(), ptr::null_mut()];
    FAKE_HINTS.store(list.as_mut_ptr() as usize, Ordering::SeqCst);
    FREED_HINTS.store(0, Ordering::SeqCst);

    let saved = ALSA_DEVICE_PREFIX.lock().unwrap().take();
    alsa_guess_device_prefix(&lib);
    let guessed = std::mem::replace(&mut *ALSA_DEVICE_PREFIX.lock().unwrap(), saved);
    assert_eq!(guessed, Some("hw:"));
    assert_eq!(FREED_HINTS.load(Ordering::SeqCst), list.as_ptr() as usize);
}

/// The query list the fake `snd_pcm_query_chmaps()` hands out, and the
/// list the fake `snd_pcm_free_chmaps()` was given.
static FAKE_CHMAPS: AtomicUsize = AtomicUsize::new(0);
static FREED_CHMAPS: AtomicUsize = AtomicUsize::new(0);

unsafe extern "C" fn fake_query_chmaps(_pcm: *mut SndPcm) -> *mut *mut SndPcmChmapQuery {
    FAKE_CHMAPS.load(Ordering::SeqCst) as *mut *mut SndPcmChmapQuery
}

unsafe extern "C" fn fake_free_chmaps(maps: *mut *mut SndPcmChmapQuery) {
    FREED_CHMAPS.store(maps as usize, Ordering::SeqCst);
}

#[test]
fn channel_map_queries_are_freed_once_read() {
    let _l = crate::test_support::test_lock();
    if !have_libasound() {
        return;
    }
    let mut lib = load_alsa_library().unwrap();
    lib.snd_pcm_query_chmaps = fake_query_chmaps;
    lib.snd_pcm_free_chmaps = fake_free_chmaps;
    // One fixed stereo map (type, channels, positions), then the terminating NULL
    let mut query: [c_uint; 4] = [
        SND_CHMAP_TYPE_FIXED as c_uint,
        2,
        SND_CHMAP_FL,
        SND_CHMAP_FR,
    ];
    let mut list: [*mut SndPcmChmapQuery; 2] = [query.as_mut_ptr().cast(), ptr::null_mut()];
    FAKE_CHMAPS.store(list.as_mut_ptr() as usize, Ordering::SeqCst);
    FREED_CHMAPS.store(0, Ordering::SeqCst);

    let target = PcmTarget {
        lib: &lib,
        pcm: ptr::null_mut(),
    };
    let queries = target.query_chmaps().unwrap();
    assert_eq!(
        queries.list,
        [ChmapQuery {
            type_: SND_CHMAP_TYPE_FIXED,
            pos: vec![SND_CHMAP_FL, SND_CHMAP_FR],
        }]
    );
    // The C list is released as soon as it's copied, whatever happens next
    assert_eq!(FREED_CHMAPS.load(Ordering::SeqCst), list.as_ptr() as usize);
}

/// Whether libasound can be loaded here; prints a note when it can't.
fn have_libasound() -> bool {
    match SharedObject::load(ALSA_LIBRARY) {
        Ok(_) => true,
        Err(e) => {
            eprintln!("note: skipping the ALSA device test: {}", e.message());
            false
        }
    }
}

/// What a [`FifoSink`] has drained: the silent bytes before the first
/// non-zero one, then up to `keep` bytes from there on.
#[derive(Default)]
struct Drained {
    lead: usize,
    kept: Vec<u8>,
}

/// The reading end of a FIFO the `file` PCM writes into, drained on another
/// thread until dropped.
///
/// The `file` PCM's slave is the `null` PCM, which has no clock: it always
/// has a whole buffer free, so the device thread never sleeps in
/// `wait_device` and writes as fast as it can. Note (upstream):
/// `ALSA_WaitDevice()` only sleeps while `snd_pcm_avail()` is short, so
/// upstream spins on such a PCM just the same. Into a regular file that is
/// hundreds of MB a second, and a test reading the file back while a
/// raised-priority device thread keeps growing it never catches up on a
/// busy CPU. Through a FIFO the writes block until this drains them, so
/// the device thread goes at our pace, and only `keep` bytes are stored.
struct FifoSink {
    keep: usize,
    seen: Arc<Mutex<Drained>>,
    stop: Arc<AtomicBool>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl FifoSink {
    fn new(path: &str, keep: usize) -> FifoSink {
        let c = CString::new(path).unwrap();
        // SAFETY: c is NUL-terminated.
        assert_eq!(unsafe { libc::mkfifo(c.as_ptr(), 0o600) }, 0, "mkfifo");
        // Read-write: the open doesn't wait for a writer, and the FIFO never
        // reports end-of-file between the PCM's opens and closes.
        let fifo = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(path)
            .unwrap();
        let seen = Arc::new(Mutex::new(Drained::default()));
        let stop = Arc::new(AtomicBool::new(false));
        let (seen2, stop2) = (seen.clone(), stop.clone());
        let thread = std::thread::spawn(move || {
            use std::io::Read;
            use std::os::fd::AsRawFd;
            let mut fifo = fifo;
            let mut buf = vec![0u8; 65536];
            while !stop2.load(Ordering::SeqCst) {
                let mut pfd = libc::pollfd {
                    fd: fifo.as_raw_fd(),
                    events: libc::POLLIN,
                    revents: 0,
                };
                // SAFETY: pfd is one valid pollfd.
                if unsafe { libc::poll(&mut pfd, 1, 10) } <= 0 {
                    continue;
                }
                let n = fifo.read(&mut buf).unwrap();
                let mut d = seen2.lock().unwrap();
                let mut data = &buf[..n];
                if d.kept.is_empty() {
                    let silent = data.iter().position(|&b| b != 0).unwrap_or(n);
                    d.lead += silent;
                    data = &data[silent..];
                }
                let room = keep - d.kept.len();
                d.kept.extend_from_slice(&data[..data.len().min(room)]);
            }
        });
        FifoSink {
            keep,
            seen,
            stop,
            thread: Some(thread),
        }
    }

    /// Whether `keep` bytes have arrived since the first non-zero one.
    fn full(&self) -> bool {
        self.seen.lock().unwrap().kept.len() == self.keep
    }

    /// Stop draining and return what was seen.
    fn finish(mut self) -> Drained {
        self.stop.store(true, Ordering::SeqCst);
        self.thread.take().unwrap().join().unwrap();
        std::mem::take(&mut *self.seen.lock().unwrap())
    }
}

impl Drop for FifoSink {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

#[test]
fn playback_and_recording_through_file_and_null_pcms() {
    let _l = crate::test_support::test_lock();
    if !have_libasound() {
        return;
    }
    let _cleanup = crate::audio::drivers::tests::QuitAudioOnDrop;
    let tmp = TempDir::new("alsa");
    let out = tmp.path("out.raw");
    let expected = AudioSpec::new(AudioFormat::S16, 2, 44100);
    let pattern: Vec<u8> = (0..expected.frame_size() * 300)
        .map(|i| (i % 251) as u8 | 1)
        .collect();
    // (the pattern, then a few periods' worth of silence)
    let keep = pattern.len() + 16384;
    let sink = FifoSink::new(&out, keep);
    crate::hints::set(crate::hints::AUDIO_DRIVER, "alsa").unwrap();
    crate::hints::set(
        crate::hints::AUDIO_ALSA_DEFAULT_PLAYBACK_DEVICE,
        &format!("file:FILE={out},FORMAT=raw"),
    )
    .unwrap();
    crate::hints::set(crate::hints::AUDIO_ALSA_DEFAULT_RECORDING_DEVICE, "null").unwrap();
    let reset = || {
        for h in [
            crate::hints::AUDIO_DRIVER,
            crate::hints::AUDIO_ALSA_DEFAULT_PLAYBACK_DEVICE,
            crate::hints::AUDIO_ALSA_DEFAULT_RECORDING_DEVICE,
        ] {
            crate::hints::reset(h);
        }
    };
    if let Err(e) = crate::init::init_subsystem(InitFlags::AUDIO) {
        reset();
        panic!(
            "ALSA should initialize when libasound loads: {}",
            e.message()
        );
    }
    assert_eq!(current_audio_driver(), Some("alsa"));
    assert!(!playback_devices().unwrap().is_empty());
    assert_eq!(
        audio_device_name(AUDIO_DEVICE_DEFAULT_PLAYBACK).unwrap(),
        "ALSA default playback device"
    );
    assert_eq!(
        audio_device_name(AUDIO_DEVICE_DEFAULT_RECORDING).unwrap(),
        "ALSA default recording device"
    );

    // Playback: the device thread writes what we queue into the FIFO, in
    // the negotiated format, with silence around it (the device plays
    // silence until the stream is bound, and after it runs dry).
    let dev = match AudioDevice::open(AUDIO_DEVICE_DEFAULT_PLAYBACK, None) {
        Ok(dev) => dev,
        Err(e) => {
            // No alsa.conf (or no `file` plugin) on this system.
            eprintln!("note: skipping ALSA playback: {}", e.message());
            crate::init::quit_subsystem(InitFlags::AUDIO);
            reset();
            return;
        }
    };
    let (spec, frames) = dev.format().unwrap();
    assert_eq!(spec, expected);
    assert!(frames > 0);
    let stream = AudioStream::new(Some(&spec), None).unwrap();
    dev.bind(&stream).unwrap();
    stream.put_data(&pattern).unwrap();
    let start = std::time::Instant::now();
    while !sink.full() && start.elapsed() < Duration::from_secs(10) {
        std::thread::sleep(Duration::from_millis(5));
    }
    drop(dev);
    drop(stream);
    let written = sink.finish();
    assert_eq!(written.kept.len(), keep, "the queued data reaches the file");
    assert_eq!(written.lead % spec.frame_size(), 0);
    assert_eq!(written.kept[..pattern.len()], pattern[..]);
    assert!(
        written.kept[pattern.len()..].iter().all(|&b| b == 0),
        "then silence"
    );

    // Recording: the null PCM records silence.
    let rec = AudioDevice::open(AUDIO_DEVICE_DEFAULT_RECORDING, None).unwrap();
    let (rspec, _) = rec.format().unwrap();
    let rstream = AudioStream::new(None, Some(&rspec)).unwrap();
    rec.bind(&rstream).unwrap();
    let start = std::time::Instant::now();
    while rstream.available() < 64 && start.elapsed() < Duration::from_secs(10) {
        std::thread::sleep(Duration::from_millis(5));
    }
    let mut buf = [0xAAu8; 64];
    assert_eq!(rstream.get_data(&mut buf).unwrap(), 64);
    assert!(buf.iter().all(|&b| b == 0), "silence: {buf:?}");
    drop(rec);
    drop(rstream);

    crate::init::quit_subsystem(InitFlags::AUDIO);
    assert!(current_audio_driver().is_none());
    reset();
}
