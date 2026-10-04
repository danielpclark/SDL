// Rust translation of src/audio/alsa/SDL_alsa_audio.c and
// src/audio/alsa/SDL_alsa_audio.h from Simple DirectMedia Layer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

// The ALSA PCM driver. libasound is loaded at run time
// (SDL_AUDIO_DRIVER_ALSA_DYNAMIC), and only the parts of its API SDL uses
// are declared here. This is the configuration upstream builds by default:
// blocking playback (SDL_ALSA_NON_BLOCKING 0), the hotplug thread
// (SDL_ALSA_HOTPLUG_THREAD 1) and debug logging (SDL_ALSA_DEBUG 1). The
// udev layer isn't translated yet, so hotplug always uses the thread, as in
// a build without SDL_USE_LIBUDEV.

use std::collections::HashMap;
use std::ffi::{c_char, c_int, c_long, c_uint, c_ulong, c_void, CStr, CString};
use std::ptr;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use crate::audio::device::{
    add_audio_device, audio_device_disconnected, default_thread_init,
    find_physical_audio_device_by_handle, updated_audio_device_format, AudioBootStrap,
    AudioDriverImpl, DeviceBackend, DriverFlags, PhysState, PhysicalDevice,
};
use crate::audio::format::AudioSpec;
use crate::audio::queue::store_chmap;
use crate::audio::AudioFormat;
use crate::error::{Error, Result};
use crate::loadso::SharedObject;
use crate::log::Category;
use crate::thread::{Thread, ThreadPriority};

// this turns off debug logging completely (but by default this goes to the bitbucket).
macro_rules! logdebug {
    ($($arg:tt)*) => {
        crate::debug!(Category::Audio, "ALSA: {}", format_args!($($arg)*))
    };
}

const SDL_AUDIO_ALSA_CHMAP_CHANS_N_MAX: usize = 8;
const SDL_AUDIO_ALSA_SDL_CHMAPS_N: usize = 9; // from 0 channels to 8 channels

// The parts of <alsa/asoundlib.h> SDL uses.

macro_rules! opaque {
    ($($name:ident),*) => {
        $(
            #[repr(C)]
            struct $name {
                _private: [u8; 0],
            }
        )*
    };
}
opaque!(
    SndPcm,
    SndPcmHwParams,
    SndPcmSwParams,
    SndCtl,
    SndCtlCardInfo,
    SndPcmInfo
);

type SndPcmUframes = c_ulong;
type SndPcmSframes = c_long;

const SND_PCM_STREAM_PLAYBACK: c_int = 0;
const SND_PCM_STREAM_CAPTURE: c_int = 1;
const SND_PCM_NONBLOCK: c_int = 0x0000_0001;
const SND_PCM_ACCESS_RW_INTERLEAVED: c_int = 3;

const SND_PCM_FORMAT_S8: c_int = 0;
const SND_PCM_FORMAT_U8: c_int = 1;
const SND_PCM_FORMAT_S16_LE: c_int = 2;
const SND_PCM_FORMAT_S16_BE: c_int = 3;
const SND_PCM_FORMAT_S32_LE: c_int = 10;
const SND_PCM_FORMAT_S32_BE: c_int = 11;
const SND_PCM_FORMAT_FLOAT_LE: c_int = 14;
const SND_PCM_FORMAT_FLOAT_BE: c_int = 15;

const SND_PCM_STATE_PREPARED: c_int = 2;
const SND_PCM_STATE_XRUN: c_int = 4;

const SND_CHMAP_TYPE_FIXED: c_int = 1;
const SND_CHMAP_TYPE_VAR: c_int = 2;
const SND_CHMAP_TYPE_PAIRED: c_int = 3;

const SND_CHMAP_UNKNOWN: u32 = 0;
const SND_CHMAP_MONO: u32 = 2;
const SND_CHMAP_FL: u32 = 3;
const SND_CHMAP_FR: u32 = 4;
const SND_CHMAP_RL: u32 = 5;
const SND_CHMAP_RR: u32 = 6;
const SND_CHMAP_FC: u32 = 7;
const SND_CHMAP_LFE: u32 = 8;
const SND_CHMAP_SL: u32 = 9;
const SND_CHMAP_SR: u32 = 10;
const SND_CHMAP_RC: u32 = 11;

/// `snd_pcm_chmap_t`: `channels` followed by that many positions.
#[repr(C)]
struct SndPcmChmap {
    channels: c_uint,
    pos: [c_uint; 0],
}

/// `snd_pcm_chmap_query_t`.
#[repr(C)]
struct SndPcmChmapQuery {
    type_: c_int,
    map: SndPcmChmap,
}

macro_rules! alsa_syms {
    ($($name:ident: fn($($arg:ty),*) $(-> $ret:ty)?;)*) => {
        /// The libasound entry points SDL uses (the `ALSA_snd_*` function
        /// pointers), with the library they came from. (All are loaded, as
        /// upstream does, though not every one is called.)
        #[allow(dead_code)]
        struct AlsaLib {
            $($name: unsafe extern "C" fn($($arg),*) $(-> $ret)?,)*
            _handle: SharedObject,
        }

        impl AlsaLib {
            /// Translation of `load_alsa_syms()`.
            fn load_syms(handle: SharedObject) -> Result<AlsaLib> {
                Ok(AlsaLib {
                    $(
                        // SAFETY: the declared type is the symbol's C
                        // signature from <alsa/asoundlib.h>, and the
                        // pointer is only called while `_handle` keeps the
                        // library loaded.
                        $name: unsafe { handle.function(stringify!($name))? },
                    )*
                    _handle: handle,
                })
            }
        }
    };
}

alsa_syms! {
    snd_pcm_open: fn(*mut *mut SndPcm, *const c_char, c_int, c_int) -> c_int;
    snd_pcm_close: fn(*mut SndPcm) -> c_int;
    snd_pcm_start: fn(*mut SndPcm) -> c_int;
    snd_pcm_writei: fn(*mut SndPcm, *const c_void, SndPcmUframes) -> SndPcmSframes;
    snd_pcm_readi: fn(*mut SndPcm, *mut c_void, SndPcmUframes) -> SndPcmSframes;
    snd_pcm_recover: fn(*mut SndPcm, c_int, c_int) -> c_int;
    snd_pcm_prepare: fn(*mut SndPcm) -> c_int;
    snd_pcm_drain: fn(*mut SndPcm) -> c_int;
    snd_strerror: fn(c_int) -> *const c_char;
    snd_pcm_hw_params_sizeof: fn() -> usize;
    snd_pcm_sw_params_sizeof: fn() -> usize;
    snd_pcm_hw_params_copy: fn(*mut SndPcmHwParams, *const SndPcmHwParams);
    snd_pcm_hw_params_any: fn(*mut SndPcm, *mut SndPcmHwParams) -> c_int;
    snd_pcm_hw_params_set_access: fn(*mut SndPcm, *mut SndPcmHwParams, c_int) -> c_int;
    snd_pcm_hw_params_set_format: fn(*mut SndPcm, *mut SndPcmHwParams, c_int) -> c_int;
    snd_pcm_hw_params_set_channels: fn(*mut SndPcm, *mut SndPcmHwParams, c_uint) -> c_int;
    snd_pcm_hw_params_get_channels: fn(*const SndPcmHwParams, *mut c_uint) -> c_int;
    snd_pcm_hw_params_set_rate_near: fn(*mut SndPcm, *mut SndPcmHwParams, *mut c_uint, *mut c_int) -> c_int;
    snd_pcm_hw_params_set_period_size_near: fn(*mut SndPcm, *mut SndPcmHwParams, *mut SndPcmUframes, *mut c_int) -> c_int;
    snd_pcm_hw_params_get_period_size: fn(*const SndPcmHwParams, *mut SndPcmUframes, *mut c_int) -> c_int;
    snd_pcm_hw_params_set_periods_min: fn(*mut SndPcm, *mut SndPcmHwParams, *mut c_uint, *mut c_int) -> c_int;
    snd_pcm_hw_params_set_periods_first: fn(*mut SndPcm, *mut SndPcmHwParams, *mut c_uint, *mut c_int) -> c_int;
    snd_pcm_hw_params_get_periods: fn(*const SndPcmHwParams, *mut c_uint, *mut c_int) -> c_int;
    snd_pcm_hw_params_set_buffer_size_near: fn(*mut SndPcm, *mut SndPcmHwParams, *mut SndPcmUframes) -> c_int;
    snd_pcm_hw_params_get_buffer_size: fn(*const SndPcmHwParams, *mut SndPcmUframes) -> c_int;
    snd_pcm_hw_params: fn(*mut SndPcm, *mut SndPcmHwParams) -> c_int;
    snd_pcm_sw_params_current: fn(*mut SndPcm, *mut SndPcmSwParams) -> c_int;
    snd_pcm_sw_params_set_start_threshold: fn(*mut SndPcm, *mut SndPcmSwParams, SndPcmUframes) -> c_int;
    snd_pcm_sw_params: fn(*mut SndPcm, *mut SndPcmSwParams) -> c_int;
    snd_pcm_nonblock: fn(*mut SndPcm, c_int) -> c_int;
    snd_pcm_wait: fn(*mut SndPcm, c_int) -> c_int;
    snd_pcm_sw_params_set_avail_min: fn(*mut SndPcm, *mut SndPcmSwParams, SndPcmUframes) -> c_int;
    snd_pcm_reset: fn(*mut SndPcm) -> c_int;
    snd_pcm_state: fn(*mut SndPcm) -> c_int;
    snd_device_name_hint: fn(c_int, *const c_char, *mut *mut *mut c_void) -> c_int;
    snd_device_name_get_hint: fn(*const c_void, *const c_char) -> *mut c_char;
    snd_device_name_free_hint: fn(*mut *mut c_void) -> c_int;
    snd_pcm_avail: fn(*mut SndPcm) -> SndPcmSframes;
    snd_ctl_card_info_sizeof: fn() -> usize;
    snd_pcm_info_sizeof: fn() -> usize;
    snd_card_next: fn(*mut c_int) -> c_int;
    snd_ctl_open: fn(*mut *mut SndCtl, *const c_char, c_int) -> c_int;
    snd_ctl_close: fn(*mut SndCtl) -> c_int;
    snd_ctl_card_info: fn(*mut SndCtl, *mut SndCtlCardInfo) -> c_int;
    snd_ctl_pcm_next_device: fn(*mut SndCtl, *mut c_int) -> c_int;
    snd_pcm_info_get_subdevices_count: fn(*const SndPcmInfo) -> c_uint;
    snd_pcm_info_set_device: fn(*mut SndPcmInfo, c_uint);
    snd_pcm_info_set_subdevice: fn(*mut SndPcmInfo, c_uint);
    snd_pcm_info_set_stream: fn(*mut SndPcmInfo, c_int);
    snd_ctl_pcm_info: fn(*mut SndCtl, *mut SndPcmInfo) -> c_int;
    snd_ctl_card_info_get_id: fn(*const SndCtlCardInfo) -> *const c_char;
    snd_pcm_info_get_name: fn(*const SndPcmInfo) -> *const c_char;
    snd_pcm_info_get_subdevice_name: fn(*const SndPcmInfo) -> *const c_char;
    snd_ctl_card_info_get_name: fn(*const SndCtlCardInfo) -> *const c_char;
    snd_ctl_card_info_clear: fn(*mut SndCtlCardInfo);
    snd_pcm_hw_free: fn(*mut SndPcm) -> c_int;
    snd_pcm_hw_params_set_channels_near: fn(*mut SndPcm, *mut SndPcmHwParams, *mut c_uint) -> c_int;
    snd_pcm_query_chmaps: fn(*mut SndPcm) -> *mut *mut SndPcmChmapQuery;
    snd_pcm_free_chmaps: fn(*mut *mut SndPcmChmapQuery);
    snd_pcm_set_chmap: fn(*mut SndPcm, *const SndPcmChmap) -> c_int;
    snd_pcm_chmap_print: fn(*const SndPcmChmap, usize, *mut c_char) -> c_int;
}

/// A NUL-terminated C string from ALSA as a `String` (empty for NULL).
///
/// # Safety
///
/// `s` must be NULL or point to a NUL-terminated string.
unsafe fn c_str(s: *const c_char) -> String {
    if s.is_null() {
        String::new()
    } else {
        // SAFETY: non-NULL and NUL-terminated (the caller's contract).
        unsafe { CStr::from_ptr(s) }.to_string_lossy().into_owned()
    }
}

/// Zeroed storage for one of ALSA's opaque, run-time-sized structs
/// (`snd_pcm_hw_params_alloca()`, `SDL_small_alloc()` + `SDL_memset()`),
/// aligned for any of their members.
struct AlsaAlloc(Vec<u64>);

impl AlsaAlloc {
    fn new(size: usize) -> AlsaAlloc {
        AlsaAlloc(vec![0u64; size.div_ceil(8).max(1)])
    }

    fn as_ptr<T>(&mut self) -> *mut T {
        self.0.as_mut_ptr().cast()
    }

    fn zero(&mut self) {
        self.0.fill(0);
    }
}

impl AlsaLib {
    /// `ALSA_snd_strerror()` as a `String`.
    fn strerror(&self, errnum: c_int) -> String {
        // SAFETY: snd_strerror() accepts any value and returns a static string.
        unsafe { c_str((self.snd_strerror)(errnum)) }
    }

    /// `ALSA_snd_pcm_chmap_print()` into a 128-byte buffer, for debug logging.
    fn chmap_print(&self, chmap: &[u32]) -> String {
        let map = chmap_to_install(chmap);
        let mut buf = [0 as c_char; 128];
        // SAFETY: `map` is a valid snd_pcm_chmap_t (channels + positions), and
        // the buffer holds the 128 bytes we report.
        unsafe {
            (self.snd_pcm_chmap_print)(map.as_ptr().cast(), buf.len(), buf.as_mut_ptr());
        }
        buf[buf.len() - 1] = 0;
        // SAFETY: the buffer is NUL-terminated (forced above).
        unsafe { c_str(buf.as_ptr()) }
    }
}

/// A `snd_pcm_chmap_t` for `chmap` (`1 + chans_n` unsigned ints).
fn chmap_to_install(chmap: &[u32]) -> Vec<c_uint> {
    let mut v = Vec::with_capacity(1 + chmap.len());
    v.push(chmap.len() as c_uint);
    v.extend_from_slice(chmap);
    v
}

/// The library and the state the hotplug thread shares with the driver.
struct AlsaShared {
    lib: AlsaLib,
    /// `hotplug_devices`: the devices seen by the last hotplug iteration.
    hotplug_devices: Mutex<Vec<AlsaDevice>>,
    /// What every handle given to `add_audio_device` stands for, until the
    /// core frees the device. (Upstream frees an `ALSA_Device` as soon as
    /// it disappears, while the core may still hold its handle; keeping the
    /// handle's data until `FreeDeviceHandle` gives the same behaviour
    /// without the dangling pointer.)
    handles: Mutex<HashMap<usize, AlsaDevice>>,
    next_handle: AtomicUsize,
    /// `ALSA_hotplug_shutdown`.
    hotplug_shutdown: AtomicBool,
}

/// The ALSA driver (`ALSA_Init()`).
struct Alsa {
    shared: Arc<AlsaShared>,
    /// `ALSA_hotplug_thread`.
    hotplug_thread: Mutex<Option<Thread>>,
}

/// Translation of `LoadALSALibrary()`.
fn load_alsa_library() -> Result<AlsaLib> {
    // Don't call SDL_SetError(): SDL_LoadObject already did.
    let handle = SharedObject::load(ALSA_LIBRARY)?;
    AlsaLib::load_syms(handle) // (UnloadALSALibrary() is dropping it)
}

/// `SDL_AUDIO_DRIVER_ALSA_DYNAMIC`.
const ALSA_LIBRARY: &str = "libasound.so.2";

/// `ALSA_device_prefix`, calculated once per process.
static ALSA_DEVICE_PREFIX: Mutex<Option<&'static str>> = Mutex::new(None);

fn alsa_device_prefix() -> &'static str {
    ALSA_DEVICE_PREFIX
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .unwrap_or("")
}

/// Translation of `ALSA_guess_device_prefix()`.
fn alsa_guess_device_prefix(lib: &AlsaLib) {
    let mut device_prefix = ALSA_DEVICE_PREFIX.lock().unwrap_or_else(|e| e.into_inner());
    if device_prefix.is_some() {
        return; // already calculated.
    }

    // Apparently there are several different ways that ALSA lists
    //  actual hardware. It could be prefixed with "hw:" or "default:"
    //  or "sysdefault:" and maybe others. Go through the list and see
    //  if we can find a preferred prefix for the system.

    const PREFIXES: [&str; 3] = ["hw:", "sysdefault:", "default:"];

    let mut hints: *mut *mut c_void = ptr::null_mut();
    // SAFETY: "pcm" is NUL-terminated and `hints` receives a NULL-terminated array.
    if unsafe { (lib.snd_device_name_hint)(-1, c"pcm".as_ptr(), &mut hints) } == 0 {
        let mut i = 0;
        loop {
            // SAFETY: snd_device_name_hint() returned a NULL-terminated array;
            // we stop at the NULL.
            let hint = unsafe { *hints.add(i) };
            if hint.is_null() {
                break;
            }
            // SAFETY: `hint` is an element of that array; "NAME" is NUL-terminated.
            let name = unsafe { (lib.snd_device_name_get_hint)(hint, c"NAME".as_ptr()) };
            if !name.is_null() {
                // SAFETY: snd_device_name_get_hint() returns a malloc'd C string.
                let name_str = unsafe { c_str(name) };
                for prefix in PREFIXES {
                    if name_str.as_bytes().starts_with(prefix.as_bytes()) {
                        *device_prefix = Some(prefix);
                        break;
                    }
                }
                // SAFETY: the string was malloc'd by libasound and is freed once.
                unsafe { libc::free(name.cast()) }; // This should NOT be SDL_free()

                if device_prefix.is_some() {
                    break;
                }
            }
            i += 1;
        }
        // SAFETY: the list came from snd_device_name_hint() and is freed once.
        unsafe { (lib.snd_device_name_free_hint)(hints) };
    }

    if device_prefix.is_none() {
        *device_prefix = Some(PREFIXES[0]); // oh well.
    }

    logdebug!(
        "device prefix is probably '{}'",
        device_prefix.unwrap_or("")
    );
}

/// Translation of `ALSA_Device`.
#[derive(Clone, Debug, PartialEq)]
struct AlsaDevice {
    // the unicity key is the triple (id,device_index,recording)
    /// empty means canonical default
    id: String,
    device_index: i32,
    name: String,
    recording: bool,
    /// The handle given to `add_audio_device` (the `ALSA_Device *` upstream).
    handle: usize,
}

const DEFAULT_PLAYBACK_HANDLE: usize = 1;
const DEFAULT_RECORDING_HANDLE: usize = 2;

/// Translation of `default_playback_handle`.
fn default_playback_handle() -> AlsaDevice {
    AlsaDevice {
        id: String::new(),
        device_index: 0,
        name: "default".to_owned(),
        recording: false,
        handle: DEFAULT_PLAYBACK_HANDLE,
    }
}

/// Translation of `default_recording_handle`.
fn default_recording_handle() -> AlsaDevice {
    AlsaDevice {
        id: String::new(),
        device_index: 0,
        name: "default".to_owned(),
        recording: true,
        handle: DEFAULT_RECORDING_HANDLE,
    }
}

/// Translation of `get_pcm_str()`.
///
/// TODO: Figure out the "right"(TM) way. For the moment we presume that if a system is using a
/// software mixer for application audio sharing which is not the linux native alsa[dmix], for
/// instance jack/pulseaudio2[pipewire]/pulseaudio1/esound/etc, we expect the system integrators did
/// configure the canonical default to the right alsa PCM plugin for their software mixer.
///
/// All the above may be completely wrong.
fn get_pcm_str(dev: &AlsaDevice) -> String {
    if dev.id.is_empty() {
        // If the user does not want to go thru the default PCM or the canonical default, the
        // the configuration space being _massive_, give the user the ability to specify
        // its own PCMs using environment variables. It will have to fit SDL constraints though.
        crate::hints::get(if dev.recording {
            crate::hints::AUDIO_ALSA_DEFAULT_RECORDING_DEVICE
        } else {
            crate::hints::AUDIO_ALSA_DEFAULT_PLAYBACK_DEVICE
        })
        .or_else(|| crate::hints::get(crate::hints::AUDIO_ALSA_DEFAULT_DEVICE))
        .unwrap_or_else(|| "default".to_owned())
    } else {
        format!(
            "{}CARD={},DEV={}",
            alsa_device_prefix(),
            dev.id,
            dev.device_index
        )
    }
}

/// An opened PCM (`struct SDL_PrivateAudioData`; the core owns the mix
/// buffer, so `mixbuf` isn't needed).
struct AlsaPcm {
    shared: Arc<AlsaShared>,
    /// The audio device handle
    pcm: *mut SndPcm,
    // (the device fields the entry points read, fixed once opened)
    sample_frames: i32,
    freq: i32,
    frame_size: usize,
}

// SAFETY: the PCM handle is only used by the device thread (wait, play,
// record, flush) and, once that thread has been joined, by close_device;
// libasound doesn't tie a PCM to the thread that opened it.
unsafe impl Send for AlsaPcm {}
// SAFETY: as above; the core never calls into one PCM from two threads at once.
unsafe impl Sync for AlsaPcm {}

impl AlsaPcm {
    fn lib(&self) -> &AlsaLib {
        &self.shared.lib
    }
}

/// Translation of `RecoverALSADevice()`.
fn recover_alsa_device(lib: &AlsaLib, pcm: *mut SndPcm, errnum: c_int) -> c_int {
    // SAFETY (all calls below): `pcm` is an open PCM handle.
    let prerecovery = unsafe { (lib.snd_pcm_state)(pcm) };
    // SAFETY: as above.
    let status = unsafe { (lib.snd_pcm_recover)(pcm, errnum, 0) }; // !!! FIXME: third parameter is non-zero to prevent libasound from printing error messages. Should we do that?
    if status == 0 {
        // SAFETY: as above.
        let postrecovery = unsafe { (lib.snd_pcm_state)(pcm) };
        if prerecovery == SND_PCM_STATE_XRUN && postrecovery == SND_PCM_STATE_PREPARED {
            // SAFETY: as above.
            unsafe { (lib.snd_pcm_start)(pcm) }; // restart the device if it stopped due to an overrun or underrun.
        }
    }
    status
}

impl DeviceBackend for AlsaPcm {
    /// Translation of `ALSA_ThreadInit()`.
    fn thread_init(&self, device: &PhysicalDevice) {
        default_thread_init(device); // (SDL_SetCurrentThreadPriority, as the default does)
                                     // do snd_pcm_start as close to the first time we PlayDevice as possible to prevent an underrun at startup.
                                     // SAFETY: `pcm` is open.
        unsafe { (self.lib().snd_pcm_start)(self.pcm) };
    }

    /// Translation of `ALSA_WaitDevice()`: this function waits until it is
    /// possible to write a full sound buffer.
    fn wait_device(&self, device: &PhysicalDevice) -> bool {
        let sample_frames = self.sample_frames;
        let fulldelay = ((sample_frames as u64 * 1000) / self.freq as u64) as i32;
        let delay = fulldelay.clamp(1, 5);

        while !device.shutting_down() {
            // SAFETY: `pcm` is open.
            let rc = unsafe { (self.lib().snd_pcm_avail)(self.pcm) } as c_int;
            if rc < 0 {
                let status = recover_alsa_device(self.lib(), self.pcm, rc);
                if status < 0 {
                    // Hmm, not much we can do - abort
                    crate::error!(
                        Category::Audio,
                        "ALSA wait failed (unrecoverable): {}",
                        self.lib().strerror(rc)
                    );
                    return false;
                }
            }
            if rc >= sample_frames {
                break;
            }
            crate::timer::delay(Duration::from_millis(delay as u64));
        }
        true
    }

    /// Translation of `ALSA_PlayDevice()`.
    fn play_device(&self, device: &PhysicalDevice, buffer: &[u8]) -> bool {
        let frame_size = self.frame_size;
        let mut sample_buf = buffer;
        let mut frames_left = (buffer.len() / frame_size) as SndPcmUframes;

        while frames_left > 0 && !device.shutting_down() {
            // SAFETY: `pcm` is open and `sample_buf` holds `frames_left` whole frames.
            let rc = unsafe {
                (self.lib().snd_pcm_writei)(self.pcm, sample_buf.as_ptr().cast(), frames_left)
            } as c_int;
            //SDL_LogInfo(SDL_LOG_CATEGORY_AUDIO, "ALSA PLAYDEVICE: WROTE %d of %d bytes", (rc >= 0) ? ((int) (rc * frame_size)) : rc, (int) (frames_left * frame_size));
            crate::sdl_assert!(rc != 0); // assuming this can't happen if we used snd_pcm_wait and queried for available space.
            if rc < 0 {
                crate::sdl_assert!(rc != -libc::EAGAIN); // assuming this can't happen if we used snd_pcm_wait and queried for available space. snd_pcm_recover won't handle it!
                let status = recover_alsa_device(self.lib(), self.pcm, rc);
                if status < 0 {
                    // Hmm, not much we can do - abort
                    crate::error!(
                        Category::Audio,
                        "ALSA write failed (unrecoverable): {}",
                        self.lib().strerror(rc)
                    );
                    return false;
                }
                continue;
            }

            sample_buf = &sample_buf[rc as usize * frame_size..];
            frames_left -= rc as SndPcmUframes;
        }

        true
    }

    /// Translation of `ALSA_GetDeviceBuf()`.
    fn get_device_buf(&self, _device: &PhysicalDevice, buffer_size: usize) -> Option<usize> {
        // SAFETY: `pcm` is open.
        let mut rc = unsafe { (self.lib().snd_pcm_avail)(self.pcm) };
        if rc <= 0 {
            // Wait a bit and try again, maybe the hardware isn't quite ready yet?
            crate::timer::delay(Duration::from_millis(1));

            // SAFETY: as above.
            rc = unsafe { (self.lib().snd_pcm_avail)(self.pcm) };
            if rc <= 0 {
                // We'll catch it next time
                return Some(0);
            }
        }

        let requested_frames = (self.sample_frames as SndPcmSframes).min(rc) as i32;
        let requested_bytes = requested_frames as usize * self.frame_size;
        crate::sdl_assert!(requested_bytes <= buffer_size);
        //SDL_LogInfo(SDL_LOG_CATEGORY_AUDIO, "ALSA GETDEVICEBUF: NEED %d BYTES", requested_bytes);
        Some(requested_bytes)
    }

    /// Translation of `ALSA_WaitDevice()` (also used for recording).
    fn wait_recording_device(&self, device: &PhysicalDevice) -> bool {
        self.wait_device(device)
    }

    /// Translation of `ALSA_RecordDevice()`.
    fn record_device(&self, _device: &PhysicalDevice, buffer: &mut [u8]) -> Result<usize> {
        let frame_size = self.frame_size;
        let buflen = buffer.len();
        crate::sdl_assert!(buflen.is_multiple_of(frame_size));

        // SAFETY: `pcm` is open.
        let total_available = unsafe { (self.lib().snd_pcm_avail)(self.pcm) };
        if total_available == 0 {
            return Ok(0); // go back to WaitDevice and try again.
        }

        let total_frames = ((buflen / frame_size) as SndPcmSframes).min(total_available) as i32;
        let rc = if total_frames < 0 {
            // FIXME (upstream): a negative (error) result from snd_pcm_avail()
            // is passed to snd_pcm_readi() as a huge frame count, which could
            // overrun `buffer`; here the error goes straight to the recovery
            // below, which is what snd_pcm_readi() reports in that state.
            total_frames
        } else {
            // SAFETY: `pcm` is open and `buffer` has room for `total_frames` frames.
            let read = unsafe {
                (self.lib().snd_pcm_readi)(
                    self.pcm,
                    buffer.as_mut_ptr().cast(),
                    total_frames as SndPcmUframes,
                )
            };
            read as c_int
        };

        crate::sdl_assert!(rc != -libc::EAGAIN); // assuming this can't happen if we used snd_pcm_wait and queried for available space. snd_pcm_recover won't handle it!

        if rc < 0 {
            let status = recover_alsa_device(self.lib(), self.pcm, rc);
            if status < 0 {
                // Hmm, not much we can do - abort
                let msg = format!(
                    "ALSA read failed (unrecoverable): {}",
                    self.lib().strerror(rc)
                );
                crate::error!(Category::Audio, "{msg}");
                return Err(Error::new(msg));
            }
            return Ok(0); // go back to WaitDevice and try again.
        }

        //SDL_LogInfo(SDL_LOG_CATEGORY_AUDIO, "ALSA: recorded %d bytes", rc * frame_size);

        Ok(rc as usize * frame_size)
    }

    /// Translation of `ALSA_FlushRecording()`.
    fn flush_recording(&self, _device: &PhysicalDevice) {
        // SAFETY: `pcm` is open.
        unsafe { (self.lib().snd_pcm_reset)(self.pcm) };
    }

    /// Translation of `ALSA_CloseDevice()`.
    fn close_device(&self, _device: &PhysicalDevice) {
        if !self.pcm.is_null() {
            // SAFETY: `pcm` is open, and this is the last use of it.
            unsafe { (self.lib().snd_pcm_close)(self.pcm) };
        }
    }
}

/// What ALSA reports for one channel map it supports (`snd_pcm_chmap_query_t`).
#[derive(Clone, Debug, PartialEq)]
struct ChmapQuery {
    type_: c_int,
    pos: Vec<u32>,
}

/// The result of `ALSA_snd_pcm_query_chmaps()`, copied out of the C list
/// (which is freed right away).
struct ChmapQueries {
    list: Vec<ChmapQuery>,
}

/// Where the channel map configuration below installs its result: the
/// opened PCM, or a stand-in in tests.
trait ChmapTarget {
    /// `ALSA_snd_pcm_query_chmaps()`.
    fn query_chmaps(&self) -> Option<ChmapQueries>;
    /// `ALSA_snd_pcm_set_chmap()`, returning ALSA's status.
    fn set_chmap(&self, chmap: &[u32]) -> c_int;
    /// `ALSA_snd_strerror()`.
    fn strerror(&self, errnum: c_int) -> String;
    /// `ALSA_snd_pcm_chmap_print()`.
    fn chmap_print(&self, chmap: &[u32]) -> String;
}

/// The opened PCM being configured.
struct PcmTarget<'a> {
    lib: &'a AlsaLib,
    pcm: *mut SndPcm,
}

impl ChmapTarget for PcmTarget<'_> {
    fn query_chmaps(&self) -> Option<ChmapQueries> {
        // SAFETY: `pcm` is open.
        let raw = unsafe { (self.lib.snd_pcm_query_chmaps)(self.pcm) };
        if raw.is_null() {
            return None;
        }
        let mut list = Vec::new();
        let mut i = 0;
        loop {
            // SAFETY: snd_pcm_query_chmaps() returns a NULL-terminated array
            // of queries; we stop at the NULL.
            let q = unsafe { *raw.add(i) };
            if q.is_null() {
                break;
            }
            // SAFETY: `q` points to a query whose map holds `channels`
            // positions right after the count.
            let entry = unsafe {
                let channels = (*q).map.channels as usize;
                let pos = ptr::addr_of!((*q).map.pos).cast::<c_uint>();
                ChmapQuery {
                    type_: (*q).type_,
                    pos: std::slice::from_raw_parts(pos, channels).to_vec(),
                }
            };
            list.push(entry);
            i += 1;
        }
        // SAFETY: the list came from snd_pcm_query_chmaps(), is freed once,
        // and nothing points into it any more.
        unsafe { (self.lib.snd_pcm_free_chmaps)(raw) };
        Some(ChmapQueries { list })
    }

    fn set_chmap(&self, chmap: &[u32]) -> c_int {
        let map = chmap_to_install(chmap);
        // SAFETY: `pcm` is open and `map` is a valid snd_pcm_chmap_t.
        unsafe { (self.lib.snd_pcm_set_chmap)(self.pcm, map.as_ptr().cast()) }
    }

    fn strerror(&self, errnum: c_int) -> String {
        self.lib.strerror(errnum)
    }

    fn chmap_print(&self, chmap: &[u32]) -> String {
        self.lib.chmap_print(chmap)
    }
}

/// Translation of `struct ALSA_pcm_cfg_ctx`: to make easier to track
/// parameters during the whole alsa pcm configuration.
struct PcmCfgCtx {
    // (the device fields the configuration reads)
    spec: AudioSpec,
    sample_frames: i32,
    /// The channel map for the core's swizzler (`device->chmap`).
    device_chmap: Option<Vec<i32>>,

    hwparams: AlsaAlloc,
    swparams: AlsaAlloc,

    matched_sdl_format: AudioFormat,
    chans_n: u32,
    rate: u32,
    /// alsa period size, SDL audio device sample_frames
    persize: SndPcmUframes,
    chmap_queries: Option<ChmapQueries>,
    sdl_chmap: [u32; SDL_AUDIO_ALSA_CHMAP_CHANS_N_MAX],
    alsa_chmap_installed: [u32; SDL_AUDIO_ALSA_CHMAP_CHANS_N_MAX],

    periods: u32,
}

impl PcmCfgCtx {
    fn new(spec: AudioSpec, sample_frames: i32) -> PcmCfgCtx {
        PcmCfgCtx {
            spec,
            sample_frames,
            device_chmap: None,
            hwparams: AlsaAlloc::new(0),
            swparams: AlsaAlloc::new(0),
            matched_sdl_format: AudioFormat::UNKNOWN,
            chans_n: 0,
            rate: 0,
            persize: 0,
            chmap_queries: None,
            sdl_chmap: [0; SDL_AUDIO_ALSA_CHMAP_CHANS_N_MAX],
            alsa_chmap_installed: [0; SDL_AUDIO_ALSA_CHMAP_CHANS_N_MAX],
            periods: 0,
        }
    }

    fn queries(&self) -> &[ChmapQuery] {
        self.chmap_queries.as_ref().map_or(&[], |q| &q.list)
    }
}

/// The following are SDL channel maps with alsa position values, from 0
/// channels to 8 channels. Translation of `sdl_channel_maps`.
/// See SDL3/SDL_audio.h
/// Strictly speaking those are "parameters" of channel maps, like alsa hwparams and swparams, they
/// have to be "reduced/refined" until an exact channel map. Only the 6 channels map requires such
/// "reduction/refine".
const SDL_CHANNEL_MAPS: [[u32; SDL_AUDIO_ALSA_CHMAP_CHANS_N_MAX]; SDL_AUDIO_ALSA_SDL_CHMAPS_N] = [
    // 0 channels
    [0, 0, 0, 0, 0, 0, 0, 0],
    // 1 channel
    [SND_CHMAP_MONO, 0, 0, 0, 0, 0, 0, 0],
    // 2 channels
    [SND_CHMAP_FL, SND_CHMAP_FR, 0, 0, 0, 0, 0, 0],
    // 3 channels
    [SND_CHMAP_FL, SND_CHMAP_FR, SND_CHMAP_LFE, 0, 0, 0, 0, 0],
    // 4 channels
    [
        SND_CHMAP_FL,
        SND_CHMAP_FR,
        SND_CHMAP_RL,
        SND_CHMAP_RR,
        0,
        0,
        0,
        0,
    ],
    // 5 channels
    [
        SND_CHMAP_FL,
        SND_CHMAP_FR,
        SND_CHMAP_LFE,
        SND_CHMAP_RL,
        SND_CHMAP_RR,
        0,
        0,
        0,
    ],
    // 6 channels
    // XXX: here we encode not a uniq channel map but a set of channel maps. We will reduce it each
    // time we are going to work with an alsa 6 channels map.
    [
        SND_CHMAP_FL,
        SND_CHMAP_FR,
        SND_CHMAP_FC,
        SND_CHMAP_LFE,
        // The 2 following channel positions are (SND_CHMAP_SL,SND_CHMAP_SR) or
        // (SND_CHMAP_RL,SND_CHMAP_RR)
        SND_CHMAP_UNKNOWN,
        SND_CHMAP_UNKNOWN,
        0,
        0,
    ],
    // 7 channels
    [
        SND_CHMAP_FL,
        SND_CHMAP_FR,
        SND_CHMAP_FC,
        SND_CHMAP_LFE,
        SND_CHMAP_RC,
        SND_CHMAP_SL,
        SND_CHMAP_SR,
        0,
    ],
    // 8 channels
    [
        SND_CHMAP_FL,
        SND_CHMAP_FR,
        SND_CHMAP_FC,
        SND_CHMAP_LFE,
        SND_CHMAP_RL,
        SND_CHMAP_RR,
        SND_CHMAP_SL,
        SND_CHMAP_SR,
    ],
];

/// Helper for the function right below. Translation of `has_pos()`.
fn has_pos(chmap: &[u32], pos: u32) -> bool {
    let mut chan_idx = 0;
    loop {
        if chan_idx == 6 {
            return false;
        }
        if chmap[chan_idx] == pos {
            return true;
        }
        chan_idx += 1;
    }
}

// XXX: Each time we are going to work on an alsa 6 channels map, we must reduce the set of channel
// maps which is encoded in sdl_channel_maps[6] to a uniq one.
const HAVE_NONE: u32 = 0;
const HAVE_REAR: u32 = 1;
const HAVE_SIDE: u32 = 2;
const HAVE_BOTH: u32 = 3;

/// Translation of `sdl_6chans_set_rear_or_side_channels_from_alsa_6chans()`.
fn sdl_6chans_set_rear_or_side_channels_from_alsa_6chans(
    sdl_6chans: &mut [u32],
    alsa_6chans: &[u32],
) {
    // For alsa channel maps with 6 channels and with SND_CHMAP_FL,SND_CHMAP_FR,SND_CHMAP_FC,
    // SND_CHMAP_LFE, reduce our 6 channels maps to a uniq one.
    if !has_pos(alsa_6chans, SND_CHMAP_FL)
        || !has_pos(alsa_6chans, SND_CHMAP_FR)
        || !has_pos(alsa_6chans, SND_CHMAP_FC)
        || !has_pos(alsa_6chans, SND_CHMAP_LFE)
    {
        sdl_6chans[4] = SND_CHMAP_UNKNOWN;
        sdl_6chans[5] = SND_CHMAP_UNKNOWN;
        logdebug!("6channels:unsupported channel map");
        return;
    }

    let mut state = HAVE_NONE;
    for &pos in &alsa_6chans[..6] {
        if pos == SND_CHMAP_SL || pos == SND_CHMAP_SR {
            if state == HAVE_NONE {
                state = HAVE_SIDE;
            } else if state == HAVE_REAR {
                state = HAVE_BOTH;
                break;
            }
        } else if pos == SND_CHMAP_RL || pos == SND_CHMAP_RR {
            if state == HAVE_NONE {
                state = HAVE_REAR;
            } else if state == HAVE_SIDE {
                state = HAVE_BOTH;
                break;
            }
        }
    }

    if state == HAVE_BOTH || state == HAVE_NONE {
        sdl_6chans[4] = SND_CHMAP_UNKNOWN;
        sdl_6chans[5] = SND_CHMAP_UNKNOWN;
        logdebug!("6channels:unsupported channel map");
    } else if state == HAVE_REAR {
        sdl_6chans[4] = SND_CHMAP_RL;
        sdl_6chans[5] = SND_CHMAP_RR;
        logdebug!("6channels:sdl map set to rear");
    } else {
        // state == HAVE_SIDE
        sdl_6chans[4] = SND_CHMAP_SL;
        sdl_6chans[5] = SND_CHMAP_SR;
        logdebug!("6channels:sdl map set to side");
    }
}

/// Translation of `swizzle_map_compute_alsa_subscan()`.
fn swizzle_map_compute_alsa_subscan(ctx: &PcmCfgCtx, swizzle_map: &mut [i32], sdl_pos_idx: usize) {
    swizzle_map[sdl_pos_idx] = -1;
    let mut alsa_pos_idx = 0;
    loop {
        crate::sdl_assert!(alsa_pos_idx != ctx.chans_n as usize); // no 0 channels or not found matching position should happen here (actually enforce playback/recording symmetry).
        if alsa_pos_idx == ctx.chans_n as usize {
            return; // (the C code would read past the installed map here)
        }
        if ctx.alsa_chmap_installed[alsa_pos_idx] == ctx.sdl_chmap[sdl_pos_idx] {
            logdebug!("swizzle SDL {} <-> alsa {}", sdl_pos_idx, alsa_pos_idx);
            swizzle_map[sdl_pos_idx] = alsa_pos_idx as i32;
            return;
        }
        alsa_pos_idx += 1;
    }
}

/// Translation of `swizzle_map_compute()`; returns `needs_swizzle`.
/// XXX: this must stay playback/recording symmetric.
fn swizzle_map_compute(ctx: &PcmCfgCtx, swizzle_map: &mut [i32]) -> bool {
    let mut needs_swizzle = false;
    for sdl_pos_idx in 0..ctx.chans_n as usize {
        swizzle_map_compute_alsa_subscan(ctx, swizzle_map, sdl_pos_idx);
        if swizzle_map[sdl_pos_idx] != sdl_pos_idx as i32 {
            needs_swizzle = true;
        }
    }
    needs_swizzle
}

const CHMAP_INSTALLED: i32 = 0;
const CHANS_N_NEXT: i32 = 1;
const CHMAP_NOT_FOUND: i32 = 2;

/// Translation of `alsa_chmap_install()`.
///
/// Should always be a queried alsa channel map unless the queried alsa channel map was of type VAR,
/// namely we can program the channel positions directly from the SDL channel map.
fn alsa_chmap_install(ctx: &mut PcmCfgCtx, target: &dyn ChmapTarget, chmap: &[u32]) -> Result<i32> {
    let chmap = &chmap[..ctx.chans_n as usize];

    logdebug!("channel map to install:{}", target.chmap_print(chmap));

    let status = target.set_chmap(chmap);
    if status < 0 {
        // FIXME (upstream): the channel map buffer isn't released on this path.
        return Err(Error::new(format!(
            "ALSA: failed to install channel map: {}",
            target.strerror(status)
        )));
    }
    ctx.alsa_chmap_installed[..chmap.len()].copy_from_slice(chmap);

    Ok(CHMAP_INSTALLED)
}

/// Translation of `alsa_chmap_has_duplicate_position()`.
///
/// We restrict the alsa channel maps because in the unordered matches we do only simple accounting.
/// In the end, this will handle mostly alsa channel maps with more than one SND_CHMAP_NA position fillers.
fn alsa_chmap_has_duplicate_position(ctx: &PcmCfgCtx, pos: &[u32]) -> bool {
    if ctx.chans_n < 2 {
        // we need at least 2 positions
        logdebug!("channel map:no duplicate");
        return false;
    }

    for chan_idx in 1..ctx.chans_n as usize {
        for seen_idx in 0..chan_idx {
            if pos[seen_idx] == pos[chan_idx] {
                logdebug!("channel map:have duplicate");
                return true;
            }
        }
    }

    logdebug!("channel map:no duplicate");
    false
}

/// Translation of `alsa_chmap_cfg_ordered_fixed_or_paired()`.
fn alsa_chmap_cfg_ordered_fixed_or_paired(
    ctx: &mut PcmCfgCtx,
    target: &dyn ChmapTarget,
) -> Result<i32> {
    let queries = ctx.queries().to_vec();
    for chmap_query in &queries {
        if chmap_query.pos.len() != ctx.chans_n as usize
            || (chmap_query.type_ != SND_CHMAP_TYPE_FIXED
                && chmap_query.type_ != SND_CHMAP_TYPE_PAIRED)
        {
            continue;
        }

        logdebug!(
            "channel map:ordered:fixed|paired:{}",
            target.chmap_print(&chmap_query.pos)
        );

        let n = ctx.chans_n as usize;
        ctx.sdl_chmap[..n].copy_from_slice(&SDL_CHANNEL_MAPS[n][..n]);

        let alsa_chmap = &chmap_query.pos;
        if ctx.chans_n == 6 {
            sdl_6chans_set_rear_or_side_channels_from_alsa_6chans(&mut ctx.sdl_chmap, alsa_chmap);
        }
        if alsa_chmap_has_duplicate_position(ctx, alsa_chmap) {
            continue;
        }

        // FIXME (upstream): the C loop compares one position past the end
        // of the map (it checks `chan_idx == chans_n` only after comparing
        // index `chans_n`), so whether an exact match installs depends on
        // the memory after the query; here a match of all `chans_n`
        // positions installs.
        if ctx.sdl_chmap[..n] == alsa_chmap[..n] {
            return alsa_chmap_install(ctx, target, alsa_chmap);
        }
    }
    Ok(CHMAP_NOT_FOUND)
}

/// Translation of `alsa_chmap_cfg_ordered_var()`.
///
/// Here, the alsa channel positions can be programmed in the alsa frame (cf HDMI).
/// If the alsa channel map is VAR, we only check we have the unordered set of channel positions we
/// are looking for.
fn alsa_chmap_cfg_ordered_var(ctx: &mut PcmCfgCtx, target: &dyn ChmapTarget) -> Result<i32> {
    let queries = ctx.queries().to_vec();
    for chmap_query in &queries {
        if chmap_query.pos.len() != ctx.chans_n as usize || chmap_query.type_ != SND_CHMAP_TYPE_VAR
        {
            continue;
        }

        logdebug!(
            "channel map:ordered:var:{}",
            target.chmap_print(&chmap_query.pos)
        );

        let n = ctx.chans_n as usize;
        ctx.sdl_chmap[..n].copy_from_slice(&SDL_CHANNEL_MAPS[n][..n]);

        let alsa_chmap = &chmap_query.pos;
        if ctx.chans_n == 6 {
            sdl_6chans_set_rear_or_side_channels_from_alsa_6chans(&mut ctx.sdl_chmap, alsa_chmap);
        }
        if alsa_chmap_has_duplicate_position(ctx, alsa_chmap) {
            continue;
        }

        let mut pos_matches_n = 0;
        for &sdl_pos in &ctx.sdl_chmap[..n] {
            for &alsa_pos in &alsa_chmap[..n] {
                if sdl_pos == alsa_pos {
                    pos_matches_n += 1;
                    break;
                }
            }
        }

        if pos_matches_n == n {
            let sdl_chmap = ctx.sdl_chmap;
            return alsa_chmap_install(ctx, target, &sdl_chmap); // XXX: we program the SDL chmap here
        }
    }

    Ok(CHMAP_NOT_FOUND)
}

/// Translation of `alsa_chmap_cfg_ordered()`.
fn alsa_chmap_cfg_ordered(ctx: &mut PcmCfgCtx, target: &dyn ChmapTarget) -> Result<i32> {
    let status = alsa_chmap_cfg_ordered_fixed_or_paired(ctx, target)?;
    if status != CHMAP_NOT_FOUND {
        Ok(status)
    } else {
        alsa_chmap_cfg_ordered_var(ctx, target)
    }
}

/// Translation of `alsa_chmap_cfg_unordered()`.
///
/// In the unordered case, we are just interested to get the same unordered set of alsa channel
/// positions than in the SDL channel map since we will swizzle (no duplicate channel position).
fn alsa_chmap_cfg_unordered(ctx: &mut PcmCfgCtx, target: &dyn ChmapTarget) -> Result<i32> {
    let queries = ctx.queries().to_vec();
    for chmap_query in &queries {
        if chmap_query.pos.len() != ctx.chans_n as usize
            || (chmap_query.type_ != SND_CHMAP_TYPE_FIXED
                && chmap_query.type_ != SND_CHMAP_TYPE_PAIRED)
        {
            continue;
        }

        logdebug!(
            "channel map:unordered:fixed|paired:{}",
            target.chmap_print(&chmap_query.pos)
        );

        let n = ctx.chans_n as usize;
        ctx.sdl_chmap[..n].copy_from_slice(&SDL_CHANNEL_MAPS[n][..n]);

        let alsa_chmap = &chmap_query.pos;
        if ctx.chans_n == 6 {
            sdl_6chans_set_rear_or_side_channels_from_alsa_6chans(&mut ctx.sdl_chmap, alsa_chmap);
        }

        if alsa_chmap_has_duplicate_position(ctx, alsa_chmap) {
            continue;
        }

        let mut pos_matches_n = 0;
        for &sdl_pos in &ctx.sdl_chmap[..n] {
            for &alsa_pos in &alsa_chmap[..n] {
                if sdl_pos == alsa_pos {
                    pos_matches_n += 1;
                    break;
                }
            }
        }

        if pos_matches_n == n {
            return alsa_chmap_install(ctx, target, alsa_chmap);
        }
    }

    Ok(CHMAP_NOT_FOUND)
}

/// Translation of `alsa_chmap_cfg()`.
fn alsa_chmap_cfg(ctx: &mut PcmCfgCtx, target: &dyn ChmapTarget) -> Result<i32> {
    ctx.chmap_queries = target.query_chmaps();
    if ctx.chmap_queries.is_none() {
        // We couldn't query the channel map, assume no swizzle necessary
        logdebug!("couldn't query channel map, swizzling off");
        return Ok(CHMAP_INSTALLED);
    }

    //----------------------------------------------------------------------------------------------
    let status = alsa_chmap_cfg_ordered(ctx, target)?; // we prefer first channel maps we don't need to swizzle
    if status == CHMAP_INSTALLED {
        logdebug!("swizzling off");
        return Ok(status);
    }

    // Fall-thru
    //----------------------------------------------------------------------------------------------
    let status = alsa_chmap_cfg_unordered(ctx, target)?; // those we will have to swizzle
    if status == CHMAP_INSTALLED {
        logdebug!("swizzling on");

        let mut swizzle_map = vec![0i32; ctx.chans_n as usize];
        let needs_swizzle = swizzle_map_compute(ctx, &mut swizzle_map); // fine grained swizzle configuration
        if needs_swizzle {
            // let SDL's swizzler handle this one.
            ctx.device_chmap = Some(swizzle_map);
        }
    }

    if status == CHMAP_NOT_FOUND {
        return Ok(CHANS_N_NEXT);
    }

    Ok(status)
}

const CHANS_N_SCAN_MODE_EQUAL_OR_ABOVE_REQUESTED_CHANS_N: u32 = 0; // target more hardware pressure
const CHANS_N_SCAN_MODE_BELOW_REQUESTED_CHANS_N: u32 = 1; // target less hardware pressure
const CHANS_N_CONFIGURED: i32 = 0;
const CHANS_N_NOT_CONFIGURED: i32 = 1;

/// The SDL format's ALSA equivalent, if it has one (the `switch` in
/// `ALSA_pcm_cfg_hw_chans_n_scan()`).
/// XXX: we are forcing the same endianness, namely we won't need byte swapping upon
/// writing/reading to/from the SDL audio buffer.
fn alsa_format_of(format: AudioFormat) -> Option<c_int> {
    Some(match format {
        AudioFormat::U8 => SND_PCM_FORMAT_U8,
        AudioFormat::S8 => SND_PCM_FORMAT_S8,
        AudioFormat::S16LE => SND_PCM_FORMAT_S16_LE,
        AudioFormat::S16BE => SND_PCM_FORMAT_S16_BE,
        AudioFormat::S32LE => SND_PCM_FORMAT_S32_LE,
        AudioFormat::S32BE => SND_PCM_FORMAT_S32_BE,
        AudioFormat::F32LE => SND_PCM_FORMAT_FLOAT_LE,
        AudioFormat::F32BE => SND_PCM_FORMAT_FLOAT_BE,
        _ => return None,
    })
}

/// Translation of `ALSA_pcm_cfg_hw_chans_n_scan()`.
fn alsa_pcm_cfg_hw_chans_n_scan(
    ctx: &mut PcmCfgCtx,
    target: &PcmTarget<'_>,
    mode: u32,
) -> Result<i32> {
    let lib = target.lib;
    let pcm = target.pcm;
    let mut target_chans_n = ctx.spec.channels as u32; // we start at what was specified
    if mode == CHANS_N_SCAN_MODE_BELOW_REQUESTED_CHANS_N {
        target_chans_n = target_chans_n.wrapping_sub(1);
    }
    loop {
        if mode == CHANS_N_SCAN_MODE_EQUAL_OR_ABOVE_REQUESTED_CHANS_N {
            if target_chans_n > SDL_AUDIO_ALSA_CHMAP_CHANS_N_MAX as u32 {
                return Ok(CHANS_N_NOT_CONFIGURED);
            }
            // else: CHANS_N_SCAN_MODE__BELOW_REQUESTED_CHANS_N
        } else if target_chans_n == 0 {
            return Ok(CHANS_N_NOT_CONFIGURED);
        }

        logdebug!("target chans_n is {}", target_chans_n);

        let hw = ctx.hwparams.as_ptr::<SndPcmHwParams>();
        // SAFETY (the hw_params calls below): `pcm` is open and `hw` points
        // to zeroed storage of snd_pcm_hw_params_sizeof() bytes.
        let mut status = unsafe { (lib.snd_pcm_hw_params_any)(pcm, hw) };
        if status < 0 {
            return Err(Error::new(format!(
                "ALSA: Couldn't get hardware config: {}",
                lib.strerror(status)
            )));
        }
        // SDL only uses interleaved sample output
        // SAFETY: as above.
        status =
            unsafe { (lib.snd_pcm_hw_params_set_access)(pcm, hw, SND_PCM_ACCESS_RW_INTERLEAVED) };
        if status < 0 {
            return Err(Error::new(format!(
                "ALSA: Couldn't set interleaved access: {}",
                lib.strerror(status)
            )));
        }
        // Try for a closest match on audio format
        ctx.matched_sdl_format = AudioFormat::UNKNOWN;
        status = -libc::EINVAL; // (reported if no format has an ALSA equivalent)
        for &closefmt in ctx.spec.format.closest_formats() {
            ctx.matched_sdl_format = closefmt;
            let Some(alsa_format) = alsa_format_of(closefmt) else {
                ctx.matched_sdl_format = AudioFormat::UNKNOWN;
                continue;
            };
            // SAFETY: as above.
            status = unsafe { (lib.snd_pcm_hw_params_set_format)(pcm, hw, alsa_format) };
            if status >= 0 {
                break;
            }
            ctx.matched_sdl_format = AudioFormat::UNKNOWN;
        }
        if ctx.matched_sdl_format == AudioFormat::UNKNOWN {
            // (with the error of the last format tried)
            return Err(Error::new(format!(
                "ALSA: Unsupported audio format: {}",
                lib.strerror(status)
            )));
        }
        // let alsa approximate the number of channels
        ctx.chans_n = target_chans_n;
        // SAFETY: as above.
        status = unsafe { (lib.snd_pcm_hw_params_set_channels_near)(pcm, hw, &mut ctx.chans_n) };
        if status < 0 {
            return Err(Error::new(format!(
                "ALSA: Couldn't set audio channels: {}",
                lib.strerror(status)
            )));
        }
        // let alsa approximate the audio rate
        ctx.rate = ctx.spec.freq as u32;
        // SAFETY: as above; the direction may be NULL.
        status = unsafe {
            (lib.snd_pcm_hw_params_set_rate_near)(pcm, hw, &mut ctx.rate, ptr::null_mut())
        };
        if status < 0 {
            return Err(Error::new(format!(
                "ALSA: Couldn't set audio frequency: {}",
                lib.strerror(status)
            )));
        }
        // let approximate the period size to the requested buffer size
        ctx.persize = ctx.sample_frames as SndPcmUframes;
        // SAFETY: as above.
        status = unsafe {
            (lib.snd_pcm_hw_params_set_period_size_near)(pcm, hw, &mut ctx.persize, ptr::null_mut())
        };
        if status < 0 {
            return Err(Error::new(format!(
                "ALSA: Couldn't set the period size: {}",
                lib.strerror(status)
            )));
        }
        // let approximate the minimum number of periods per buffer (we target a double buffer)
        ctx.periods = 2;
        // SAFETY: as above.
        status = unsafe {
            (lib.snd_pcm_hw_params_set_periods_min)(pcm, hw, &mut ctx.periods, ptr::null_mut())
        };
        if status < 0 {
            return Err(Error::new(format!(
                "ALSA: Couldn't set the minimum number of periods per buffer: {}",
                lib.strerror(status)
            )));
        }
        // restrict the number of periods per buffer to an approximation of the approximated minimum
        // number of periods per buffer done right above
        // SAFETY: as above.
        status = unsafe {
            (lib.snd_pcm_hw_params_set_periods_first)(pcm, hw, &mut ctx.periods, ptr::null_mut())
        };
        if status < 0 {
            return Err(Error::new(format!(
                "ALSA: Couldn't set the number of periods per buffer: {}",
                lib.strerror(status)
            )));
        }
        // install the hw parameters
        // SAFETY: as above.
        status = unsafe { (lib.snd_pcm_hw_params)(pcm, hw) };
        if status < 0 {
            return Err(Error::new(format!(
                "ALSA: installation of hardware parameter failed: {}",
                lib.strerror(status)
            )));
        }
        //==========================================================================================
        // Here the alsa pcm is in SND_PCM_STATE_PREPARED state, let's figure out a good fit for
        // SDL channel map, it may request to change the target number of channels though.
        let status = alsa_chmap_cfg(ctx, target)?; // we forward the SDL error
        if status == CHMAP_INSTALLED {
            return Ok(CHANS_N_CONFIGURED); // we are finished here
        }

        // status == CHANS_N_NEXT
        ctx.chmap_queries = None;
        // SAFETY: `pcm` is open.
        unsafe { (lib.snd_pcm_hw_free)(pcm) }; // uninstall those hw params

        if mode == CHANS_N_SCAN_MODE_EQUAL_OR_ABOVE_REQUESTED_CHANS_N {
            target_chans_n += 1;
        } else {
            // CHANS_N_SCAN_MODE__BELOW_REQUESTED_CHANS_N
            target_chans_n -= 1;
        }
    }
}

/// Translation of `ALSA_pcm_cfg_hw()`.
fn alsa_pcm_cfg_hw(ctx: &mut PcmCfgCtx, target: &PcmTarget<'_>) -> Result<()> {
    logdebug!("target chans_n, equal or above requested chans_n mode");
    let status = alsa_pcm_cfg_hw_chans_n_scan(
        ctx,
        target,
        CHANS_N_SCAN_MODE_EQUAL_OR_ABOVE_REQUESTED_CHANS_N,
    )?; // something went too wrong
    if status == CHANS_N_CONFIGURED {
        return Ok(());
    }

    // Here, status == CHANS_N_NOT_CONFIGURED
    logdebug!("target chans_n, below requested chans_n mode");
    let status =
        alsa_pcm_cfg_hw_chans_n_scan(ctx, target, CHANS_N_SCAN_MODE_BELOW_REQUESTED_CHANS_N)?; // something went too wrong
    if status == CHANS_N_CONFIGURED {
        return Ok(());
    }

    // Here, status == CHANS_N_NOT_CONFIGURED
    Err(Error::new(
        "ALSA: Couldn't configure targeting any SDL supported channel number",
    ))
}

/// Translation of `ALSA_pcm_cfg_sw()`.
fn alsa_pcm_cfg_sw(ctx: &mut PcmCfgCtx, target: &PcmTarget<'_>) -> Result<()> {
    let lib = target.lib;
    let pcm = target.pcm;
    let sw = ctx.swparams.as_ptr::<SndPcmSwParams>();

    // SAFETY (the sw_params calls below): `pcm` is open and `sw` points to
    // zeroed storage of snd_pcm_sw_params_sizeof() bytes.
    let mut status = unsafe { (lib.snd_pcm_sw_params_current)(pcm, sw) };
    if status < 0 {
        return Err(Error::new(format!(
            "ALSA: Couldn't get software config: {}",
            lib.strerror(status)
        )));
    }

    // SAFETY: as above.
    status = unsafe { (lib.snd_pcm_sw_params_set_avail_min)(pcm, sw, ctx.persize) }; // will become device->sample_frames if the alsa pcm configuration is successful
    if status < 0 {
        return Err(Error::new(format!(
            "Couldn't set minimum available samples: {}",
            lib.strerror(status)
        )));
    }

    // SAFETY: as above.
    status = unsafe { (lib.snd_pcm_sw_params_set_start_threshold)(pcm, sw, 1) };
    if status < 0 {
        return Err(Error::new(format!(
            "ALSA: Couldn't set start threshold: {}",
            lib.strerror(status)
        )));
    }
    // SAFETY: as above.
    status = unsafe { (lib.snd_pcm_sw_params)(pcm, sw) };
    if status < 0 {
        return Err(Error::new(format!(
            "Couldn't set software audio parameters: {}",
            lib.strerror(status)
        )));
    }
    Ok(())
}

impl Alsa {
    /// What a device handle stands for (`(ALSA_Device *)device->handle`).
    fn device_of_handle(&self, handle: usize) -> Option<AlsaDevice> {
        match handle {
            DEFAULT_PLAYBACK_HANDLE => Some(default_playback_handle()),
            DEFAULT_RECORDING_HANDLE => Some(default_recording_handle()),
            _ => self
                .shared
                .handles
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .get(&handle)
                .cloned(),
        }
    }
}

impl AlsaShared {
    /// Translation of `hotplug_device_process()`.
    fn hotplug_device_process(
        &self,
        ctl: *mut SndCtl,
        ctl_card_info: *mut SndCtlCardInfo,
        dev_idx: c_int,
        direction: c_int,
        unseen: &mut Vec<AlsaDevice>,
        seen: &mut Vec<AlsaDevice>,
    ) -> std::result::Result<(), ()> {
        let lib = &self.lib;
        let mut subdevs_n: c_uint = 1; // we have at least one subdevice (substream since the direction is a stream in alsa terminology)
        let mut subdev_idx: c_uint = 0;
        let recording = direction == SND_PCM_STREAM_CAPTURE; // used for the unicity of the device
                                                             // SAFETY: snd_pcm_info_sizeof() has no preconditions.
        let mut pcm_info_storage = AlsaAlloc::new(unsafe { (lib.snd_pcm_info_sizeof)() });
        let pcm_info = pcm_info_storage.as_ptr::<SndPcmInfo>();

        loop {
            // SAFETY (the pcm_info calls below): `pcm_info` points to zeroed
            // storage of snd_pcm_info_sizeof() bytes and `ctl` is open.
            unsafe {
                (lib.snd_pcm_info_set_stream)(pcm_info, direction);
                (lib.snd_pcm_info_set_device)(pcm_info, dev_idx as c_uint);
                (lib.snd_pcm_info_set_subdevice)(pcm_info, subdev_idx); // we have at least one subdevice (substream) of index 0
            }

            // SAFETY: as above.
            let r = unsafe { (lib.snd_ctl_pcm_info)(ctl, pcm_info) };
            if r < 0 {
                // first call to ALSA_snd_ctl_pcm_info
                if subdev_idx == 0 && r == -libc::ENOENT {
                    // no such direction/stream for this device
                    return Ok(());
                }
                return Err(());
            }

            if subdev_idx == 0 {
                // SAFETY: as above.
                subdevs_n = unsafe { (lib.snd_pcm_info_get_subdevices_count)(pcm_info) };
            }

            // SAFETY: `ctl_card_info` was filled by snd_ctl_card_info(); the id is a C string.
            let card_id = unsafe { c_str((lib.snd_ctl_card_info_get_id)(ctl_card_info)) };

            // building the unseen list scanning the list of hotplug devices, if it is already there
            // using the id, move it to the seen list.
            let found = unseen.iter().position(|adev| {
                // the unicity key is the triple (id,device_index,recording)
                adev.id == card_id && adev.device_index == dev_idx && adev.recording == recording
            });
            if let Some(i) = found {
                // unchain from unseen
                let adev = unseen.remove(i);
                // chain to seen
                seen.insert(0, adev);
            } else {
                // newly seen device
                // SAFETY: both structs were filled by ALSA above; the names are C strings.
                let name = unsafe {
                    format!(
                        "{}:{}",
                        c_str((lib.snd_ctl_card_info_get_name)(ctl_card_info)),
                        c_str((lib.snd_pcm_info_get_name)(pcm_info))
                    )
                };
                let adev = AlsaDevice {
                    id: card_id,
                    device_index: dev_idx,
                    name,
                    recording: direction == SND_PCM_STREAM_CAPTURE,
                    handle: self.next_handle.fetch_add(1, Ordering::Relaxed),
                };

                self.handles
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .insert(adev.handle, adev.clone());
                if add_audio_device(recording, &adev.name, None, None, adev.handle).is_none() {
                    self.handles
                        .lock()
                        .unwrap_or_else(|e| e.into_inner())
                        .remove(&adev.handle);
                    return Err(());
                }

                seen.insert(0, adev);
            }

            subdev_idx += 1;
            if subdev_idx == subdevs_n {
                return Ok(());
            }

            pcm_info_storage.zero();
        }
    }

    /// Translation of `ALSA_HotplugIteration()`.
    fn hotplug_iteration(
        &self,
        has_default_output: Option<&mut bool>,
        has_default_recording: Option<&mut bool>,
    ) {
        let lib = &self.lib;
        if let Some(h) = has_default_output {
            *h = true;
        }

        if let Some(h) = has_default_recording {
            *h = true;
        }

        // SAFETY: snd_ctl_card_info_sizeof() has no preconditions.
        let mut ctl_card_info_storage = AlsaAlloc::new(unsafe { (lib.snd_ctl_card_info_sizeof)() });
        let ctl_card_info = ctl_card_info_storage.as_ptr::<SndCtlCardInfo>();

        let mut ctl: *mut SndCtl = ptr::null_mut();
        let mut unseen = std::mem::take(
            &mut *self
                .hotplug_devices
                .lock()
                .unwrap_or_else(|e| e.into_inner()),
        );
        let mut seen: Vec<AlsaDevice> = Vec::new();
        let mut card_idx: c_int = -1;
        let failed = 'scan: loop {
            // SAFETY: card_idx is a valid in/out pointer.
            let r = unsafe { (lib.snd_card_next)(&mut card_idx) };
            if r < 0 {
                break 'scan true;
            } else if card_idx == -1 {
                break 'scan false;
            }

            let ctl_name = format!("{}{}", alsa_device_prefix(), card_idx); // card_idx >= 0
            logdebug!("hotplug ctl_name = '{}'", ctl_name);

            let ctl_cname = CString::new(ctl_name).unwrap_or_default();
            // SAFETY: `ctl` receives the handle; the name is NUL-terminated.
            let r = unsafe { (lib.snd_ctl_open)(&mut ctl, ctl_cname.as_ptr(), 0) };
            if r < 0 {
                continue;
            }

            // SAFETY: `ctl` is open; `ctl_card_info` is zeroed storage of the right size.
            let r = unsafe { (lib.snd_ctl_card_info)(ctl, ctl_card_info) };
            if r < 0 {
                break 'scan true;
            }

            let mut dev_idx: c_int = -1;
            loop {
                // SAFETY: `ctl` is open; dev_idx is a valid in/out pointer.
                let r = unsafe { (lib.snd_ctl_pcm_next_device)(ctl, &mut dev_idx) };
                if r < 0 {
                    break 'scan true;
                } else if dev_idx == -1 {
                    break;
                }

                if self
                    .hotplug_device_process(
                        ctl,
                        ctl_card_info,
                        dev_idx,
                        SND_PCM_STREAM_PLAYBACK,
                        &mut unseen,
                        &mut seen,
                    )
                    .is_err()
                {
                    break 'scan true;
                }

                if self
                    .hotplug_device_process(
                        ctl,
                        ctl_card_info,
                        dev_idx,
                        SND_PCM_STREAM_CAPTURE,
                        &mut unseen,
                        &mut seen,
                    )
                    .is_err()
                {
                    break 'scan true;
                }
            }
            // SAFETY: `ctl` is open and closed once here; `ctl_card_info` is ours.
            unsafe {
                (lib.snd_ctl_close)(ctl);
                (lib.snd_ctl_card_info_clear)(ctl_card_info);
            }
            // FIXME (upstream): `ctl` isn't reset after closing, so a later
            // failure closes it a second time; here it's reset instead.
            ctl = ptr::null_mut();
        };

        if !failed {
            // remove only the unseen devices
            for dev in unseen {
                if let Some(device) = find_physical_audio_device_by_handle(dev.handle) {
                    audio_device_disconnected(&device);
                }
            }

            // update hotplug devices to be the seen devices
            *self
                .hotplug_devices
                .lock()
                .unwrap_or_else(|e| e.into_inner()) = seen;
            return;
        }

        // (failed:)
        if !ctl.is_null() {
            // SAFETY: `ctl` is still open (see the FIXME above).
            unsafe { (lib.snd_ctl_close)(ctl) };
        }

        // remove the unseen, then the seen
        for dev in unseen.into_iter().chain(seen) {
            if let Some(device) = find_physical_audio_device_by_handle(dev.handle) {
                audio_device_disconnected(&device);
            }
        }

        self.hotplug_devices
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clear();
    }
}

/// Translation of `ALSA_HotplugThread()`.
fn alsa_hotplug_thread(shared: Arc<AlsaShared>) -> i32 {
    let _ = crate::thread::set_current_thread_priority(ThreadPriority::Low);

    while !shared.hotplug_shutdown.load(Ordering::Acquire) {
        // Block awhile before checking again, unless we're told to stop.
        let ticks = crate::timer::ticks() + Duration::from_millis(5000);
        while !shared.hotplug_shutdown.load(Ordering::Acquire) && crate::timer::ticks() < ticks {
            crate::timer::delay(Duration::from_millis(100));
        }

        shared.hotplug_iteration(None, None); // run the check.
    }

    0
}

/// Translation of `ALSA_start_udev()` (without SDL_USE_LIBUDEV).
fn alsa_start_udev() -> bool {
    false
}

/// Translation of `ALSA_stop_udev()` (without SDL_USE_LIBUDEV).
fn alsa_stop_udev() {}

impl AudioDriverImpl for Alsa {
    fn flags(&self) -> DriverFlags {
        DriverFlags {
            has_recording_support: true,
            ..DriverFlags::default()
        }
    }

    /// Translation of `ALSA_DetectDevices()`.
    fn detect_devices(&self) -> (Option<Arc<PhysicalDevice>>, Option<Arc<PhysicalDevice>>) {
        alsa_guess_device_prefix(&self.shared.lib);

        // ALSA doesn't have a concept of a changeable default device, afaik, so we expose a generic default
        // device here. It's the best we can do at this level.
        let mut has_default_playback = false;
        let mut has_default_recording = false;
        self.shared.hotplug_iteration(
            Some(&mut has_default_playback),
            Some(&mut has_default_recording),
        ); // run once now before a thread continues to check.
        let mut default_playback = None;
        let mut default_recording = None;
        if has_default_playback {
            default_playback = add_audio_device(
                /*recording=*/ false,
                "ALSA default playback device",
                None,
                None,
                DEFAULT_PLAYBACK_HANDLE,
            );
        }
        if has_default_recording {
            default_recording = add_audio_device(
                /*recording=*/ true,
                "ALSA default recording device",
                None,
                None,
                DEFAULT_RECORDING_HANDLE,
            );
        }

        if !alsa_start_udev() {
            self.shared.hotplug_shutdown.store(false, Ordering::Release);
            let shared = self.shared.clone();
            // if the thread doesn't spin, oh well, you just don't get further hotplug events.
            if let Ok(thread) = Thread::spawn("SDLHotplugALSA", move || alsa_hotplug_thread(shared))
            {
                *self
                    .hotplug_thread
                    .lock()
                    .unwrap_or_else(|e| e.into_inner()) = Some(thread);
            }
        }

        (default_playback, default_recording)
    }

    /// Translation of `ALSA_OpenDevice()`.
    fn open_device(
        &self,
        device: &PhysicalDevice,
        state: &mut PhysState,
    ) -> Result<Arc<dyn DeviceBackend>> {
        let recording = device.recording;
        let lib = &self.shared.lib;

        //device->spec.channels = 8;
        //SDL_SetLogPriority(SDL_LOG_CATEGORY_AUDIO, SDL_LOG_PRIORITY_VERBOSE);
        logdebug!("channels requested {}", state.spec.channels);
        // XXX: We do not use the SDL internal swizzler yet.
        state.chmap = None;

        let mut cfg_ctx = PcmCfgCtx::new(state.spec, state.sample_frames);

        // Open the audio device
        // (get_pcm_str() asserts the handle is valid: SDL2 used NULL to mean "default" but that's not true in SDL3.)
        let dev = self
            .device_of_handle(device.handle)
            .ok_or_else(|| Error::new("ALSA: Couldn't open audio device: unknown device handle"))?;
        let pcm_str = get_pcm_str(&dev);
        logdebug!("PCM open '{}'", pcm_str);
        let Ok(pcm_cstr) = CString::new(pcm_str) else {
            return Err(Error::invalid_param("devname"));
        };
        let mut pcm: *mut SndPcm = ptr::null_mut();
        // SAFETY: `pcm` receives the handle; the name is NUL-terminated.
        let status = unsafe {
            (lib.snd_pcm_open)(
                &mut pcm,
                pcm_cstr.as_ptr(),
                if recording {
                    SND_PCM_STREAM_CAPTURE
                } else {
                    SND_PCM_STREAM_PLAYBACK
                },
                SND_PCM_NONBLOCK,
            )
        };
        if status < 0 {
            return Err(Error::new(format!(
                "ALSA: Couldn't open audio device: {}",
                lib.strerror(status)
            )));
        }
        let close_pcm = || {
            // SAFETY: `pcm` is open and closed once, on this error path (err_close_pcm).
            unsafe { (lib.snd_pcm_close)(pcm) };
        };

        // Now we need to configure the opened pcm as close as possible from the requested parameters we
        // can reasonably deal with (and that could change)
        // SAFETY: the sizeof functions have no preconditions.
        unsafe {
            cfg_ctx.hwparams = AlsaAlloc::new((lib.snd_pcm_hw_params_sizeof)());
            cfg_ctx.swparams = AlsaAlloc::new((lib.snd_pcm_sw_params_sizeof)());
        }

        let target = PcmTarget { lib, pcm };
        if let Err(e) = alsa_pcm_cfg_hw(&mut cfg_ctx, &target) {
            // alsa pcm "hardware" part of the pcm
            close_pcm();
            return Err(e);
        }

        // from here, hwparams is uninstalled upon pcm closing (the alsa chmap queries were freed
        // as soon as they were read)

        // This is useful for debugging
        let mut bufsize: SndPcmUframes = 0;
        // SAFETY: hwparams was filled by the configuration above.
        unsafe {
            (lib.snd_pcm_hw_params_get_buffer_size)(cfg_ctx.hwparams.as_ptr(), &mut bufsize);
        }
        crate::debug!(
            Category::Audio,
            "ALSA: period size = {}, periods = {}, buffer size = {}",
            cfg_ctx.persize,
            cfg_ctx.periods,
            bufsize
        );

        if let Err(e) = alsa_pcm_cfg_sw(&mut cfg_ctx, &target) {
            // alsa pcm "software" part of the pcm
            // (err_cleanup_ctx:)
            close_pcm();
            return Err(e);
        }

        // Now we can update the following parameters in the spec:
        state.spec.format = cfg_ctx.matched_sdl_format;
        state.spec.channels = cfg_ctx.chans_n as i32;
        state.spec.freq = cfg_ctx.rate as i32;
        state.sample_frames = cfg_ctx.persize as i32;
        if let Some(chmap) = &cfg_ctx.device_chmap {
            state.chmap = store_chmap(Some(chmap), cfg_ctx.chans_n as i32);
        }
        // Calculate the final parameters for this audio specification
        updated_audio_device_format(state);

        // (the core allocates the silenced mixing buffer)

        if !recording {
            // SAFETY: `pcm` is open.
            unsafe { (lib.snd_pcm_nonblock)(pcm, 0) };
        }

        Ok(Arc::new(AlsaPcm {
            shared: self.shared.clone(),
            pcm,
            sample_frames: state.sample_frames,
            freq: state.spec.freq,
            frame_size: state.spec.frame_size(),
        })) // We're ready to rock and roll. :-)
    }

    /// Frees what a hotplugged device's handle stands for (see
    /// `AlsaShared::handles`); upstream has no `FreeDeviceHandle` for ALSA.
    fn free_device_handle(&self, device: &PhysicalDevice) {
        self.shared
            .handles
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(&device.handle);
    }

    /// Translation of `ALSA_DeinitializeStart()`.
    fn deinitialize_start(&self) {
        let thread = self
            .hotplug_thread
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .take();
        if let Some(thread) = thread {
            self.shared.hotplug_shutdown.store(true, Ordering::Release);
            thread.wait();
        }
        alsa_stop_udev();

        // Shutting down! Clean up any data we've gathered.
        // FIXME (upstream): each device's `id` string is leaked here (only `name` is freed).
        self.shared
            .hotplug_devices
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clear();
    }

    /// Translation of `ALSA_Deinitialize()`.
    fn deinitialize(&self) {
        // (UnloadALSALibrary(): the library is released with the last reference to it.)
    }
}

/// Translation of `ALSA_Init()`.
fn alsa_init() -> Option<Arc<dyn AudioDriverImpl>> {
    let lib = load_alsa_library().ok()?;

    Some(Arc::new(Alsa {
        shared: Arc::new(AlsaShared {
            lib,
            hotplug_devices: Mutex::new(Vec::new()),
            handles: Mutex::new(HashMap::new()),
            next_handle: AtomicUsize::new(DEFAULT_RECORDING_HANDLE + 1),
            hotplug_shutdown: AtomicBool::new(false),
        }),
        hotplug_thread: Mutex::new(None),
    }))
}

/// Translation of `ALSA_bootstrap`.
pub(crate) static ALSA_BOOTSTRAP: AudioBootStrap = AudioBootStrap {
    name: "alsa",
    desc: "ALSA PCM audio",
    init: alsa_init,
    demand_only: false,
    is_preferred: false,
};

#[cfg(test)]
mod tests;
