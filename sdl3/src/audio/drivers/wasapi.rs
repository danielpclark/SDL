// Rust translation of src/audio/wasapi/SDL_wasapi.c and
// src/audio/wasapi/SDL_wasapi.h from Simple DirectMedia Layer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

// The WASAPI driver, over the MMDevice glue in core::windows::immdevice.
// The audio client interfaces aren't in windows-sys, so their vtables are
// declared by hand here.
//
// Upstream's PrepDevice applies the negotiated format with
// SDL_AudioDeviceFormatChangedAlreadyLocked() from the management thread,
// while the thread that opened the device holds its lock and waits. Here
// the management thread hands the negotiated format back and the waiting
// thread applies it: OpenDevice to the state it was given, recovery
// through the device lock.

use std::collections::{HashMap, VecDeque};
use std::ffi::c_void;
use std::ptr;
use std::sync::atomic::{AtomicBool, AtomicI32, AtomicPtr, AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, Weak};

use windows_sys::core::{BOOL, GUID, HRESULT};
use windows_sys::Win32::Foundation::{CloseHandle, HANDLE, S_OK, WAIT_OBJECT_0, WAIT_TIMEOUT};
use windows_sys::Win32::Media::Audio::{
    AUDCLNT_BUFFERFLAGS_SILENT, AUDCLNT_E_BUFFER_TOO_LARGE, AUDCLNT_E_DEVICE_INVALIDATED,
    AUDCLNT_SHAREMODE, AUDCLNT_SHAREMODE_SHARED, AUDCLNT_STREAMFLAGS_EVENTCALLBACK,
    AUDCLNT_STREAMFLAGS_LOOPBACK, AUDCLNT_S_BUFFER_EMPTY, AUDIO_STREAM_CATEGORY, WAVEFORMATEX,
};
use windows_sys::Win32::System::Com::{CoTaskMemFree, CLSCTX_ALL};
use windows_sys::Win32::System::Threading::{CreateEventW, WaitForSingleObjectEx};

use crate::audio::device::{
    audio_device_disconnected, audio_device_format_changed,
    audio_device_format_changed_while_opening, default_audio_device_changed, default_thread_init,
    find_physical_audio_device_by_handle, ref_physical_audio_device, silence_value_of,
    unref_physical_audio_device, with_device_state, AudioBootStrap, AudioDriverImpl, DeviceBackend,
    DriverFlags, PhysState, PhysicalDevice,
};
use crate::audio::{AudioFormat, AudioSpec};
use crate::core::windows::immdevice::{
    self, failed, query_interface, release, succeeded, ComObject, IMMDevice, IUnknownVtbl,
    ImmDeviceCallbacks,
};
use crate::core::windows::{
    co_initialize, co_uninitialize, error_from_hresult, set_error, utf8_to_wide,
};
use crate::error::{Error, Result};
use crate::loadso::SharedObject;
use crate::thread::{Semaphore, Thread};

// These constants aren't available in older SDKs
#[allow(dead_code)]
const AUDCLNT_STREAMFLAGS_RATEADJUST: u32 = 0x0010_0000;
const AUDCLNT_STREAMFLAGS_SRC_DEFAULT_QUALITY: u32 = 0x0800_0000;
const AUDCLNT_STREAMFLAGS_AUTOCONVERTPCM: u32 = 0x8000_0000;

type ReferenceTime = i64;

// --- the audio client interfaces (<audioclient.h>) ---

/// `IAudioClientVtbl`.
#[repr(C)]
struct IAudioClientVtbl {
    base: IUnknownVtbl,
    initialize: unsafe extern "system" fn(
        *mut IAudioClient,
        AUDCLNT_SHAREMODE,
        u32,
        ReferenceTime,
        ReferenceTime,
        *const WAVEFORMATEX,
        *const GUID,
    ) -> HRESULT,
    get_buffer_size: unsafe extern "system" fn(*mut IAudioClient, *mut u32) -> HRESULT,
    get_stream_latency: unsafe extern "system" fn(*mut IAudioClient, *mut ReferenceTime) -> HRESULT,
    get_current_padding: unsafe extern "system" fn(*mut IAudioClient, *mut u32) -> HRESULT,
    is_format_supported: unsafe extern "system" fn(
        *mut IAudioClient,
        AUDCLNT_SHAREMODE,
        *const WAVEFORMATEX,
        *mut *mut WAVEFORMATEX,
    ) -> HRESULT,
    get_mix_format: unsafe extern "system" fn(*mut IAudioClient, *mut *mut WAVEFORMATEX) -> HRESULT,
    get_device_period: unsafe extern "system" fn(
        *mut IAudioClient,
        *mut ReferenceTime,
        *mut ReferenceTime,
    ) -> HRESULT,
    start: unsafe extern "system" fn(*mut IAudioClient) -> HRESULT,
    stop: unsafe extern "system" fn(*mut IAudioClient) -> HRESULT,
    reset: unsafe extern "system" fn(*mut IAudioClient) -> HRESULT,
    set_event_handle: unsafe extern "system" fn(*mut IAudioClient, HANDLE) -> HRESULT,
    get_service:
        unsafe extern "system" fn(*mut IAudioClient, *const GUID, *mut *mut c_void) -> HRESULT,
}
type IAudioClient = ComObject<IAudioClientVtbl>;

/// `IAudioClient2Vtbl`.
#[repr(C)]
struct IAudioClient2Vtbl {
    base: IAudioClientVtbl,
    is_offload_capable:
        unsafe extern "system" fn(*mut IAudioClient2, AUDIO_STREAM_CATEGORY, *mut BOOL) -> HRESULT,
    set_client_properties:
        unsafe extern "system" fn(*mut IAudioClient2, *const SdlAudioClientProperties) -> HRESULT,
    get_buffer_size_limits: unsafe extern "system" fn(
        *mut IAudioClient2,
        *const WAVEFORMATEX,
        BOOL,
        *mut ReferenceTime,
        *mut ReferenceTime,
    ) -> HRESULT,
}
type IAudioClient2 = ComObject<IAudioClient2Vtbl>;

/// `IAudioClient3Vtbl`.
#[repr(C)]
struct IAudioClient3Vtbl {
    base: IAudioClient2Vtbl,
    get_shared_mode_engine_period: unsafe extern "system" fn(
        *mut IAudioClient3,
        *const WAVEFORMATEX,
        *mut u32,
        *mut u32,
        *mut u32,
        *mut u32,
    ) -> HRESULT,
    get_current_shared_mode_engine_period:
        unsafe extern "system" fn(*mut IAudioClient3, *mut *mut WAVEFORMATEX, *mut u32) -> HRESULT,
    initialize_shared_audio_stream: unsafe extern "system" fn(
        *mut IAudioClient3,
        u32,
        u32,
        *const WAVEFORMATEX,
        *const GUID,
    ) -> HRESULT,
}
type IAudioClient3 = ComObject<IAudioClient3Vtbl>;

/// `IAudioRenderClientVtbl`.
#[repr(C)]
struct IAudioRenderClientVtbl {
    base: IUnknownVtbl,
    get_buffer: unsafe extern "system" fn(*mut IAudioRenderClient, u32, *mut *mut u8) -> HRESULT,
    release_buffer: unsafe extern "system" fn(*mut IAudioRenderClient, u32, u32) -> HRESULT,
}
type IAudioRenderClient = ComObject<IAudioRenderClientVtbl>;

/// `IAudioCaptureClientVtbl`.
#[repr(C)]
struct IAudioCaptureClientVtbl {
    base: IUnknownVtbl,
    get_buffer: unsafe extern "system" fn(
        *mut IAudioCaptureClient,
        *mut *mut u8,
        *mut u32,
        *mut u32,
        *mut u64,
        *mut u64,
    ) -> HRESULT,
    release_buffer: unsafe extern "system" fn(*mut IAudioCaptureClient, u32) -> HRESULT,
    get_next_packet_size: unsafe extern "system" fn(*mut IAudioCaptureClient, *mut u32) -> HRESULT,
}
type IAudioCaptureClient = ComObject<IAudioCaptureClientVtbl>;

/// `SDL_AudioClientProperties`: `AudioClientProperties` with the
/// `Options` field Windows 8.1 added.
#[repr(C)]
#[derive(Default)]
struct SdlAudioClientProperties {
    cb_size: u32,
    b_is_offload: BOOL,
    e_category: AUDIO_STREAM_CATEGORY,
    options: i32, // AUDCLNT_STREAMOPTIONS
}

// AUDCLNT_STREAMOPTIONS and AudioClientProperties->Options were
// added in Windows 8.1: This ugliness is here to make sure that
// we can build against older SDK versions.
const SDL_AUDCLNT_STREAMOPTIONS_RAW: i32 = 0x1;
/// `sizeof (AudioClientProperties)` in the current SDK.
const SIZEOF_AUDIOCLIENTPROPERTIES: u32 = 16;

// Some GUIDs we need to know without linking to libraries that aren't available before Vista.
const SDL_IID_IAUDIORENDERCLIENT: GUID = GUID {
    data1: 0xf294acfc,
    data2: 0x3146,
    data3: 0x4483,
    data4: [0xa7, 0xbf, 0xad, 0xdc, 0xa7, 0xc2, 0x60, 0xe2],
};
const SDL_IID_IAUDIOCAPTURECLIENT: GUID = GUID {
    data1: 0xc8adbd64,
    data2: 0xe71e,
    data3: 0x48a0,
    data4: [0xa4, 0xde, 0x18, 0x5c, 0x39, 0x5c, 0xd3, 0x17],
};
const SDL_IID_IAUDIOCLIENT: GUID = GUID {
    data1: 0x1cb9ad4c,
    data2: 0xdbfa,
    data3: 0x4c32,
    data4: [0xb1, 0x78, 0xc2, 0xf5, 0x68, 0xa7, 0x03, 0xb2],
};
const SDL_IID_IAUDIOCLIENT2: GUID = GUID {
    data1: 0x726778cd,
    data2: 0xf60a,
    data3: 0x4eda,
    data4: [0x82, 0xde, 0xe4, 0x76, 0x10, 0xcd, 0x78, 0xaa],
};
const SDL_IID_IAUDIOCLIENT3: GUID = GUID {
    data1: 0x7ed4ee07,
    data2: 0x8e67,
    data3: 0x4cd4,
    data4: [0x8c, 0x1a, 0x2b, 0x7a, 0x59, 0x87, 0xad, 0x42],
};

/// handle to Avrt.dll--Vista and later!--for flagging the callback thread
/// as "Pro Audio" (low latency), with the two functions SDL uses.
struct Avrt {
    av_set_mm_thread_characteristics_w:
        Option<unsafe extern "system" fn(*const u16, *mut u32) -> HANDLE>,
    av_revert_mm_thread_characteristics: Option<unsafe extern "system" fn(HANDLE) -> BOOL>,
    _lib: SharedObject,
}

/// `libavrt`, `pAvSetMmThreadCharacteristicsW` and `pAvRevertMmThreadCharacteristics`.
static LIBAVRT: Mutex<Option<Avrt>> = Mutex::new(None);

static IMMDEVICE_INITIALIZED: AtomicBool = AtomicBool::new(false);
static SUPPORTS_RECORDING_ON_PLAYBACK_DEVICES: AtomicBool = AtomicBool::new(false);

// WASAPI is _really_ particular about various things happening on the same thread, for COM and such,
//  so we proxy various stuff to a single background thread to manage.

/// Translation of `ManagementThreadTask`.
type ManagementThreadTask = Box<dyn FnOnce() -> Result<()> + Send>;

/// Where a waiting caller gets a task's result (`task_complete_sem`, `result`, `errorstr`).
struct TaskWaiter {
    task_complete_sem: Semaphore,
    result: Mutex<Option<Result<()>>>,
}

/// Translation of `ManagementThreadPendingTask`.
struct ManagementThreadPendingTask {
    fn_: ManagementThreadTask,
    waiter: Option<Arc<TaskWaiter>>,
}

/// The management thread's statics: `ManagementThread`,
/// `ManagementThreadPendingTasks` (behind `ManagementThreadLock`),
/// `ManagementThreadCondition` and `ManagementThreadShutdown`.
struct Management {
    pending_tasks: Mutex<VecDeque<ManagementThreadPendingTask>>,
    condition: Condvar,
    shutdown: AtomicBool,
    thread: Mutex<Option<Thread>>,
    thread_id: AtomicU64,
}

static MANAGEMENT: Management = Management {
    pending_tasks: Mutex::new(VecDeque::new()),
    condition: Condvar::new(),
    shutdown: AtomicBool::new(false),
    thread: Mutex::new(None),
    thread_id: AtomicU64::new(0),
};

fn pending_tasks() -> MutexGuard<'static, VecDeque<ManagementThreadPendingTask>> {
    MANAGEMENT
        .pending_tasks
        .lock()
        .unwrap_or_else(|e| e.into_inner())
}

/// Translation of `ManagementThreadMainloop()`.
fn management_thread_mainloop() {
    let mut pending = pending_tasks();
    loop {
        let task = pending.pop_front();
        if task.is_none() && MANAGEMENT.shutdown.load(Ordering::Acquire) {
            break;
        }
        match task {
            None => {
                pending = MANAGEMENT
                    .condition
                    .wait(pending)
                    .unwrap_or_else(|e| e.into_inner()); // block until there's something to do.
            }
            Some(task) => {
                // (the task is off the pending list)
                drop(pending); // let other things add to the list while we chew on this task.
                let result = (task.fn_)(); // run this task.
                if let Some(waiter) = task.waiter {
                    // something waiting on result?
                    *waiter.result.lock().unwrap_or_else(|e| e.into_inner()) = Some(result);
                    waiter.task_complete_sem.signal();
                } // nothing waiting, we're done, free it.
                pending = pending_tasks(); // regrab the lock so we can get the next task; if nothing to do, we'll release the lock in SDL_WaitCondition.
            }
        }
    }
    // told to shut down and out of tasks, let go of the lock and return.
}

/// Translation of `WASAPI_ProxyToManagementThread()`: queue `task` on the
/// management thread; with `wait_on_result`, block for its result (the
/// `Some` value). An `Err` means the task couldn't be added.
///
/// BE CAREFUL: if you are holding the device lock and proxy to the
/// management thread with wait_until_complete, and grab the lock again,
/// you will deadlock.
fn wasapi_proxy_to_management_thread(
    task: impl FnOnce() -> Result<()> + Send + 'static,
    wait_on_result: bool,
) -> Result<Option<Result<()>>> {
    // We want to block for a result, but we are already running from the management thread! Just run the task now so we don't deadlock.
    if wait_on_result
        && crate::thread::current_thread_id() == MANAGEMENT.thread_id.load(Ordering::Acquire)
    {
        return Ok(Some(task())); // completed!
    }

    if MANAGEMENT.shutdown.load(Ordering::Acquire) {
        return Err(Error::new("Can't add task, we're shutting down"));
    }

    let waiter = wait_on_result.then(|| {
        Arc::new(TaskWaiter {
            task_complete_sem: Semaphore::new(0),
            result: Mutex::new(None),
        })
    });

    {
        let mut pending = pending_tasks();

        // add to end of task list.
        pending.push_back(ManagementThreadPendingTask {
            fn_: Box::new(task),
            waiter: waiter.clone(),
        });

        // task is added to the end of the pending list, let management thread rip!
        MANAGEMENT.condition.notify_one();
    }

    if let Some(waiter) = waiter {
        waiter.task_complete_sem.wait();
        let result = waiter
            .result
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .take()
            .unwrap_or(Ok(()));
        return Ok(Some(result));
    }

    Ok(None) // successfully added (and possibly executed)!
}

/// Run `task` on the management thread and wait for it (`WASAPI_ProxyToManagementThread(task, userdata, &rc) && rc`).
fn proxy_and_wait(task: impl FnOnce() -> Result<()> + Send + 'static) -> Result<()> {
    wasapi_proxy_to_management_thread(task, true)?.unwrap_or(Ok(()))
}

/// Queue `task` on the management thread without waiting (`WASAPI_ProxyToManagementThread(task, userdata, NULL)`).
fn proxy_no_wait(task: impl FnOnce() -> Result<()> + Send + 'static) {
    let _ = wasapi_proxy_to_management_thread(task, false);
}

/// The opened devices' private data, for `WASAPI_DisconnectDevice()`
/// (upstream's `device->hidden`).
static OPENED: Mutex<Option<HashMap<usize, Weak<WasapiDevice>>>> = Mutex::new(None);

fn hidden_of(device: &PhysicalDevice) -> Option<Arc<WasapiDevice>> {
    OPENED
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .as_ref()
        .and_then(|m| m.get(&device.handle))
        .and_then(Weak::upgrade)
}

/// Translation of `AudioDeviceDisconnected()`.
fn audio_device_disconnected_callback(device: Option<Arc<PhysicalDevice>>) {
    if let Some(device) = device {
        wasapi_disconnect_device(&device, hidden_of(&device).as_deref());
    }
}

/// Translation of `mgmtthrtask_DefaultAudioDeviceChanged()`.
fn mgmtthrtask_default_audio_device_changed(device: Arc<PhysicalDevice>) -> Result<()> {
    default_audio_device_changed(&device);
    unref_physical_audio_device(&device); // make sure this lived until the task completes.
    Ok(())
}

/// Translation of `DefaultAudioDeviceChanged()`.
fn default_audio_device_changed_callback(new_default_device: Option<Arc<PhysicalDevice>>) {
    // don't wait on this, IMMDevice's own thread needs to return or everything will deadlock.
    if let Some(device) = new_default_device {
        ref_physical_audio_device(&device); // make sure this lives until the task completes.
        proxy_no_wait(move || mgmtthrtask_default_audio_device_changed(device));
    }
}

/// Translation of `StopWasapiHotplug()`.
fn stop_wasapi_hotplug() {
    if IMMDEVICE_INITIALIZED.swap(false, Ordering::AcqRel) {
        immdevice::quit();
    }
}

/// Translation of `Deinit()`.
fn deinit() {
    *LIBAVRT.lock().unwrap_or_else(|e| e.into_inner()) = None; // FreeLibrary(libavrt)

    stop_wasapi_hotplug();

    co_uninitialize();
}

/// Translation of `ManagementThreadPrepare()`.
fn management_thread_prepare() -> Result<()> {
    let callbacks = ImmDeviceCallbacks {
        audio_device_disconnected: audio_device_disconnected_callback,
        default_audio_device_changed: default_audio_device_changed_callback,
    };
    if failed(co_initialize()) {
        return Err(Error::new("CoInitialize() failed"));
    }
    // FIXME (upstream): when SDL_IMMDevice_Init() fails, this thread's
    // CoInitialize() is never balanced.
    immdevice::init(Some(callbacks))?; // Error string is set by SDL_IMMDevice_Init

    IMMDEVICE_INITIALIZED.store(true, Ordering::Release);

    // this library is available in Vista and later. No WinXP, so have to LoadLibrary to use it for now!
    if let Ok(lib) = SharedObject::load("avrt.dll") {
        // SAFETY: the types are the functions' signatures from <avrt.h>;
        // they're only called while `_lib` keeps the DLL loaded.
        let avrt = unsafe {
            Avrt {
                av_set_mm_thread_characteristics_w: lib
                    .function("AvSetMmThreadCharacteristicsW")
                    .ok(),
                av_revert_mm_thread_characteristics: lib
                    .function("AvRevertMmThreadCharacteristics")
                    .ok(),
                _lib: lib,
            }
        };
        *LIBAVRT.lock().unwrap_or_else(|e| e.into_inner()) = Some(avrt);
    }

    // (ManagementThreadLock and ManagementThreadCondition are statics)

    Ok(())
}

/// Translation of `ManagementThreadEntry()`.
fn management_thread_entry(ready_sem: Arc<Semaphore>, errorstr: Arc<Mutex<Option<Error>>>) -> i32 {
    if let Err(e) = management_thread_prepare() {
        *errorstr.lock().unwrap_or_else(|e| e.into_inner()) = Some(e);
        ready_sem.signal(); // unblock calling thread.
        return 0;
    }

    ready_sem.signal(); // unblock calling thread.
    management_thread_mainloop();

    deinit();
    0
}

/// Translation of `InitManagementThread()`.
fn init_management_thread() -> Result<()> {
    let ready_sem = Arc::new(Semaphore::new(0));
    let errorstr: Arc<Mutex<Option<Error>>> = Arc::new(Mutex::new(None));

    pending_tasks().clear();
    MANAGEMENT.shutdown.store(false, Ordering::Release);
    let (sem, err) = (ready_sem.clone(), errorstr.clone());
    let thread = Thread::builder()
        .name("SDLWASAPIMgmt")
        .stack_size(256 * 1024) // !!! FIXME: maybe even smaller stack size?
        .spawn(move || management_thread_entry(sem, err))?;
    MANAGEMENT.thread_id.store(thread.id(), Ordering::Release);

    ready_sem.wait();

    let error = errorstr.lock().unwrap_or_else(|e| e.into_inner()).take();
    if let Some(e) = error {
        thread.wait();
        MANAGEMENT.thread_id.store(0, Ordering::Release);
        return Err(e);
    }

    *MANAGEMENT.thread.lock().unwrap_or_else(|e| e.into_inner()) = Some(thread);
    Ok(())
}

/// Translation of `DeinitManagementThread()`.
fn deinit_management_thread() {
    let thread = MANAGEMENT
        .thread
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .take();
    if let Some(thread) = thread {
        MANAGEMENT.shutdown.store(true, Ordering::Release);
        {
            let _pending = pending_tasks();
            MANAGEMENT.condition.notify_one();
        }
        thread.wait();
        MANAGEMENT.thread_id.store(0, Ordering::Release);
    }

    crate::sdl_assert!(pending_tasks().is_empty());

    MANAGEMENT.shutdown.store(false, Ordering::Release);
}

/// A COM pointer handed to the management thread (WASAPI's objects are
/// free-threaded).
struct SendPtr<T>(*mut T);
// SAFETY: see above; each one is used by one thread at a time.
unsafe impl<T> Send for SendPtr<T> {}

/// An opened device. Translation of `struct SDL_PrivateAudioData` (the
/// `devid` field is never set upstream and is left out).
struct WasapiDevice {
    /// The physical device, for the management thread's work on it.
    device: Weak<PhysicalDevice>,
    recording: bool,

    waveformat: AtomicPtr<WAVEFORMATEX>,
    client: AtomicPtr<IAudioClient>,
    render: AtomicPtr<IAudioRenderClient>,
    capture: AtomicPtr<IAudioCaptureClient>,
    event: AtomicPtr<c_void>,
    task: AtomicPtr<c_void>,
    coinitialized: AtomicBool,
    framesize: AtomicI32,
    device_disconnecting: AtomicI32,
    device_lost: AtomicBool,
    device_dead: AtomicBool,
    isplayback: AtomicBool,

    /// `device->sample_frames`, as the entry points read it.
    sample_frames: AtomicI32,
    /// The endpoint buffer GetDeviceBuf got, for PlayDevice.
    render_buffer: AtomicPtr<u8>,
    /// The format PrepDevice negotiated, for the waiting thread to apply.
    prepared: Mutex<Option<(AudioSpec, i32)>>,
}

// SAFETY: WASAPI's objects are free-threaded; the pointers are atomics,
// used from the device thread and the management thread in turn, as
// upstream does.
unsafe impl Send for WasapiDevice {}
// SAFETY: as above.
unsafe impl Sync for WasapiDevice {}

/// Translation of `WASAPI_DisconnectDevice()`: don't hold the device lock when calling this!
fn wasapi_disconnect_device(device: &PhysicalDevice, hidden: Option<&WasapiDevice>) {
    // don't block in here; IMMDevice's own thread needs to return or everything will deadlock.
    if hidden.is_none_or(|h| {
        h.device_disconnecting
            .compare_exchange(0, 1, Ordering::AcqRel, Ordering::Acquire)
            .is_ok()
    }) {
        audio_device_disconnected(device); // this proxies the work to the main thread now, so no point in proxying to the management thread.
    }
}

impl WasapiDevice {
    fn client(&self) -> *mut IAudioClient {
        self.client.load(Ordering::Acquire)
    }

    /// Translation of `WasapiFailed()`.
    fn wasapi_failed(&self, err: HRESULT) -> bool {
        if err == S_OK {
            return false;
        } else if err == AUDCLNT_E_DEVICE_INVALIDATED {
            self.device_lost.store(true, Ordering::Release);
        } else {
            self.device_dead.store(true, Ordering::Release);
        }

        true
    }

    /// Translation of `ResetWasapiDevice()`.
    fn reset_wasapi_device(&self) {
        // just queue up all the tasks in the management thread and don't block.
        // We don't care when any of these actually get free'd.

        let client = self.client.swap(ptr::null_mut(), Ordering::AcqRel);
        if !client.is_null() {
            let client = SendPtr(client);
            // mgmtthrtask_StopAndReleaseClient
            proxy_no_wait(move || {
                let client = client;
                // SAFETY: our reference to the client, released once.
                unsafe {
                    (IAudioClient::vtbl(client.0).stop)(client.0);
                    release(client.0);
                }
                Ok(())
            });
        }

        let render = self.render.swap(ptr::null_mut(), Ordering::AcqRel);
        if !render.is_null() {
            let render = SendPtr(render);
            // mgmtthrtask_ReleaseRenderClient
            proxy_no_wait(move || {
                let render = render;
                // SAFETY: our reference, released once.
                unsafe { release(render.0) };
                Ok(())
            });
        }

        let capture = self.capture.swap(ptr::null_mut(), Ordering::AcqRel);
        if !capture.is_null() {
            let capture = SendPtr(capture);
            // mgmtthrtask_ReleaseCaptureClient
            proxy_no_wait(move || {
                let capture = capture;
                // SAFETY: our reference, released once.
                unsafe { release(capture.0) };
                Ok(())
            });
        }

        let waveformat = self.waveformat.swap(ptr::null_mut(), Ordering::AcqRel);
        if !waveformat.is_null() {
            let ptr = SendPtr(waveformat);
            // mgmtthrtask_CoTaskMemFree
            proxy_no_wait(move || {
                let ptr = ptr;
                // SAFETY: GetMixFormat()'s allocation, freed once.
                unsafe { CoTaskMemFree(ptr.0.cast()) };
                Ok(())
            });
        }

        let event = self.event.swap(ptr::null_mut(), Ordering::AcqRel);
        if !event.is_null() {
            let event = SendPtr(event);
            // mgmtthrtask_CloseHandle
            proxy_no_wait(move || {
                let event = event;
                // SAFETY: our event handle, closed once.
                unsafe { CloseHandle(event.0) };
                Ok(())
            });
        }
    }

    /// Translation of `ActivateWasapiDevice()`: activate (and prepare) the
    /// endpoint for a device at `spec` and `sample_frames`; the negotiated
    /// format is left in `prepared`.
    fn activate_wasapi_device(self: &Arc<Self>, spec: AudioSpec, sample_frames: i32) -> Result<()> {
        // this blocks because we're either being notified from a background thread or we're running during device open,
        //  both of which won't deadlock vs the device thread.
        let h = self.clone();
        proxy_and_wait(move || h.mgmtthrtask_activate_device(spec, sample_frames))
    }

    /// Translation of `mgmtthrtask_ActivateDevice()`.
    fn mgmtthrtask_activate_device(
        self: &Arc<Self>,
        spec: AudioSpec,
        sample_frames: i32,
    ) -> Result<()> {
        let Some(device) = self.device.upgrade() else {
            return Err(Error::new("WASAPI can't find requested audio endpoint"));
        };

        let immdevice = match immdevice::get(&device, self.recording) {
            Ok(d) => d,
            Err(e) => {
                self.client.store(ptr::null_mut(), Ordering::Release);
                return Err(e); // This is already set by SDL_IMMDevice_Get
            }
        };

        // SAFETY: a live endpoint.
        self.isplayback.store(
            !unsafe { immdevice::get_is_capture(immdevice) },
            Ordering::Release,
        );

        // this is _not_ async in standard win32, yay!
        let mut client: *mut IAudioClient = ptr::null_mut();
        // SAFETY: the endpoint is live and released once; `client` receives the interface.
        let ret = unsafe {
            let ret = (IMMDevice::vtbl(immdevice).activate)(
                immdevice,
                &SDL_IID_IAUDIOCLIENT,
                CLSCTX_ALL,
                ptr::null_mut(),
                ptr::from_mut(&mut client).cast(),
            );
            release(immdevice);
            ret
        };

        if failed(ret) {
            crate::sdl_assert!(client.is_null());
            return Err(error_from_hresult(
                Some("WASAPI can't activate audio endpoint"),
                ret,
            ));
        }

        crate::sdl_assert!(!client.is_null());
        self.client.store(client, Ordering::Release);
        self.wasapi_prep_device(spec, sample_frames)?; // not async, fire it right away.

        Ok(()) // good to go.
    }

    /// Translation of `WASAPI_PrepDevice()`: this is called once a device
    /// is activated, possibly asynchronously.
    fn wasapi_prep_device(self: &Arc<Self>, spec: AudioSpec, sample_frames: i32) -> Result<()> {
        let h = self.clone();
        proxy_and_wait(move || h.mgmtthrtask_prep_device(spec, sample_frames))
    }

    /// Translation of `RecoverWasapiDevice()`. do not call when holding the device lock!
    fn recover_wasapi_device(self: &Arc<Self>, device: &PhysicalDevice) -> bool {
        self.reset_wasapi_device(); // dump the lost device's handles.

        // This handles a non-default device that simply had its format changed in the Windows Control Panel.
        let (spec, sample_frames) = with_device_state(device, |st| (st.spec, st.sample_frames));
        if self.activate_wasapi_device(spec, sample_frames).is_err() {
            wasapi_disconnect_device(device, Some(self));
            return false;
        }
        // FIXME (upstream): PrepDevice applies the new format with
        // SDL_AudioDeviceFormatChangedAlreadyLocked() here although nothing
        // holds the device lock; here it takes the lock.
        let prepared = self
            .prepared
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .take();
        if let (Some((newspec, frames)), Some(arc)) = (prepared, self.device.upgrade()) {
            let _ = audio_device_format_changed(&arc, &newspec, frames);
            self.sample_frames.store(frames, Ordering::Release);
        }

        self.device_lost.store(false, Ordering::Release);

        true // okay, carry on with new device details!
    }

    /// Translation of `RecoverWasapiIfLost()`. do not call when holding the device lock!
    fn recover_wasapi_if_lost(self: &Arc<Self>, device: &PhysicalDevice) -> bool {
        if device.shutting_down() {
            return false; // closing, stop trying.
        } else if self.device_disconnecting.load(Ordering::Acquire) != 0 {
            return false; // failing via the WASAPI management thread, stop trying.
        } else if self.device_dead.load(Ordering::Acquire) {
            // had a fatal error elsewhere, clean up and quit
            let client = self.client();
            // FIXME (upstream): this calls IAudioClient_Stop() without
            // checking for a NULL client; here a NULL one is skipped.
            if !client.is_null() {
                // SAFETY: a live client.
                unsafe { (IAudioClient::vtbl(client).stop)(client) };
            }
            wasapi_disconnect_device(device, Some(self));
            // FIXME (upstream): SDL_AudioDeviceDisconnected() marks the device a
            // zombie, it doesn't set `shutdown`, so this assertion fails.
            crate::sdl_assert!(device.shutting_down()); // so we don't come back through here.
            return false; // already failed.
        } else if device.is_zombie() {
            return false; // we're already dead, so just leave and let the Zombie implementations take over.
        } else if self.client().is_null() {
            return true; // still waiting for activation.
        }

        if self.device_lost.load(Ordering::Acquire) {
            self.recover_wasapi_device(device)
        } else {
            true
        }
    }

    /// Translation of `mgmtthrtask_PrepDevice()`.
    fn mgmtthrtask_prep_device(
        self: &Arc<Self>,
        spec: AudioSpec,
        sample_frames: i32,
    ) -> Result<()> {
        /* !!! FIXME: we could request an exclusive mode stream, which is lower latency;
        !!!  it will write into the kernel's audio buffer directly instead of
        !!!  shared memory that a user-mode mixer then writes to the kernel with
        !!!  everything else. Doing this means any other sound using this device will
        !!!  stop playing, including the user's MP3 player and system notification
        !!!  sounds. You'd probably need to release the device when the app isn't in
        !!!  the foreground, to be a good citizen of the system. It's doable, but it's
        !!!  more work and causes some annoyances, and I don't know what the latency
        !!!  wins actually look like. Maybe add a hint to force exclusive mode at
        !!!  some point. To be sure, defaulting to shared mode is the right thing to
        !!!  do in any case. */
        let sharemode = AUDCLNT_SHAREMODE_SHARED;

        let client = self.client();
        crate::sdl_assert!(!client.is_null());

        // SAFETY: default security, auto-reset, unsignaled, unnamed.
        let event = unsafe { CreateEventW(ptr::null(), 0, 0, ptr::null()) };
        if event.is_null() {
            return Err(set_error("WASAPI can't create an event handle"));
        }
        self.event.store(event, Ordering::Release);

        // SAFETY (the client calls below): `client` is our live client; the
        // out-pointers are valid; `waveformat` is its mix format, which we
        // own (freed in reset_wasapi_device()).
        let mut waveformat: *mut WAVEFORMATEX = ptr::null_mut();
        let ret = unsafe { (IAudioClient::vtbl(client).get_mix_format)(client, &mut waveformat) };
        if failed(ret) {
            return Err(error_from_hresult(
                Some("WASAPI can't determine mix format"),
                ret,
            ));
        }
        crate::sdl_assert!(!waveformat.is_null());
        self.waveformat.store(waveformat, Ordering::Release);

        // SAFETY: GetMixFormat() returns a whole (possibly extensible) format.
        let (n_channels, n_samples_per_sec, wasapi_format) = unsafe {
            let wf = waveformat.read_unaligned();
            (
                wf.nChannels,
                wf.nSamplesPerSec,
                crate::core::windows::wave_format_ex_to_sdl_format(&*waveformat),
            )
        };

        // Make sure we have a valid format that we can convert to whatever WASAPI wants.
        let Some(format) = choose_format(spec.format, wasapi_format) else {
            return Err(Error::new(format!(
                "{}: Unsupported audio format",
                "wasapi"
            )));
        };

        let mut default_period: ReferenceTime = 0;
        // SAFETY: as above; the minimum period may be NULL.
        let ret = unsafe {
            (IAudioClient::vtbl(client).get_device_period)(
                client,
                &mut default_period,
                ptr::null_mut(),
            )
        };
        if failed(ret) {
            return Err(error_from_hresult(
                Some("WASAPI can't determine minimum device period"),
                ret,
            ));
        }

        let mut streamflags: u32 = 0;

        /* we've gotten reports that WASAPI's resampler introduces distortions, but in the short term
        it fixes some other WASAPI-specific quirks we haven't quite tracked down.
        Refer to bug #6326 for the immediate concern. */
        // favor WASAPI's resampler over our own
        if spec.freq as u32 != n_samples_per_sec {
            streamflags |=
                AUDCLNT_STREAMFLAGS_AUTOCONVERTPCM | AUDCLNT_STREAMFLAGS_SRC_DEFAULT_QUALITY;
            // SAFETY: our mix format; the struct is packed, so write unaligned.
            unsafe { set_wave_format_rate(waveformat, spec.freq as u32) };
        }

        // SAFETY: as above.
        let freq = unsafe { ptr::addr_of!((*waveformat).nSamplesPerSec).read_unaligned() };
        let newspec = AudioSpec::new(format, n_channels as u8 as i32, freq as i32);

        if self.recording && self.isplayback.load(Ordering::Acquire) {
            streamflags |= AUDCLNT_STREAMFLAGS_LOOPBACK;
        }

        streamflags |= AUDCLNT_STREAMFLAGS_EVENTCALLBACK;

        let mut new_sample_frames: i32 = 0;
        let mut iaudioclient3_initialized = false;

        let mut client2: *mut IAudioClient2 = ptr::null_mut();
        // SAFETY: as above.
        let ret = unsafe { query_interface(client, &SDL_IID_IAUDIOCLIENT2, &mut client2) };
        if succeeded(ret) {
            let mut audio_props = SdlAudioClientProperties {
                cb_size: size_of::<SdlAudioClientProperties>() as u32,
                ..SdlAudioClientProperties::default()
            };

            // Setting AudioCategory_GameChat breaks audio on several devices, including Behringer U-PHORIA UM2 and RODE NT-USB Mini.
            // We'll disable this for now until we understand more about what's happening.
            // (#if 0: the SDL_HINT_AUDIO_DEVICE_STREAM_ROLE categories)

            if crate::core::windows::is_windows_81_or_greater()
                && crate::hints::get_bool(crate::hints::AUDIO_DEVICE_RAW_STREAM, false)
            {
                audio_props.options = SDL_AUDCLNT_STREAMOPTIONS_RAW;
            } else {
                audio_props.cb_size = SIZEOF_AUDIOCLIENTPROPERTIES;
            }

            // SAFETY: a live IAudioClient2, released once; the properties are valid.
            let ret = unsafe {
                (IAudioClient2::vtbl(client2).set_client_properties)(client2, &audio_props)
            };
            if failed(ret) {
                // This isn't fatal, let's log it instead of failing
                crate::warn!(
                    crate::log::Category::Audio,
                    "IAudioClient2_SetClientProperties failed: 0x{:x}",
                    ret as u32
                );
            }
            // SAFETY: as above.
            unsafe { release(client2) };
        }

        // Try querying IAudioClient3 if sharemode is AUDCLNT_SHAREMODE_SHARED
        if sharemode == AUDCLNT_SHAREMODE_SHARED {
            let mut client3: *mut IAudioClient3 = ptr::null_mut();
            // SAFETY: as above.
            let ret = unsafe { query_interface(client, &SDL_IID_IAUDIOCLIENT3, &mut client3) };
            if succeeded(ret) {
                let mut default_period_in_frames: u32 = 0;
                let mut fundamental_period_in_frames: u32 = 0;
                let mut min_period_in_frames: u32 = 0;
                let mut max_period_in_frames: u32 = 0;
                // SAFETY: a live IAudioClient3, released once; valid out-pointers and format.
                unsafe {
                    let v3 = IAudioClient3::vtbl(client3);
                    let ret = (v3.get_shared_mode_engine_period)(
                        client3,
                        waveformat,
                        &mut default_period_in_frames,
                        &mut fundamental_period_in_frames,
                        &mut min_period_in_frames,
                        &mut max_period_in_frames,
                    );
                    if succeeded(ret) {
                        // IAudioClient3_InitializeSharedAudioStream only accepts the integral multiple of fundamental_period_in_frames
                        let period_in_frames = shared_mode_period_in_frames(
                            sample_frames,
                            fundamental_period_in_frames,
                            min_period_in_frames,
                            max_period_in_frames,
                        );

                        let ret = (v3.initialize_shared_audio_stream)(
                            client3,
                            streamflags,
                            period_in_frames,
                            waveformat,
                            ptr::null(),
                        );
                        if succeeded(ret) {
                            new_sample_frames = period_in_frames as i32;
                            iaudioclient3_initialized = true;
                        }
                    }

                    release(client3);
                }
            }
        }

        let mut ret = S_OK;
        if !iaudioclient3_initialized {
            // SAFETY: as above.
            ret = unsafe {
                (IAudioClient::vtbl(client).initialize)(
                    client,
                    sharemode,
                    streamflags,
                    0,
                    0,
                    waveformat,
                    ptr::null(),
                )
            };
        }

        if failed(ret) {
            return Err(error_from_hresult(
                Some("WASAPI can't initialize audio client"),
                ret,
            ));
        }

        // SAFETY: as above.
        let ret = unsafe { (IAudioClient::vtbl(client).set_event_handle)(client, event) };
        if failed(ret) {
            return Err(error_from_hresult(
                Some("WASAPI can't set event handle"),
                ret,
            ));
        }

        let mut bufsize: u32 = 0; // this is in sample frames, not samples, not bytes.
                                  // SAFETY: as above.
        let ret = unsafe { (IAudioClient::vtbl(client).get_buffer_size)(client, &mut bufsize) };
        if failed(ret) {
            return Err(error_from_hresult(
                Some("WASAPI can't determine buffer size"),
                ret,
            ));
        }

        // Match the callback size to the period size to cut down on the number of
        // interrupts waited for in each call to WaitDevice
        if new_sample_frames <= 0 {
            new_sample_frames = period_sample_frames(default_period, newspec.freq);
        }

        // regardless of what we calculated for the period size, clamp it to the expected hardware buffer size.
        if new_sample_frames > bufsize as i32 {
            new_sample_frames = bufsize as i32;
        }

        // Update the fragment size as size in bytes
        // (SDL_AudioDeviceFormatChangedAlreadyLocked(): the waiting thread applies it.)
        *self.prepared.lock().unwrap_or_else(|e| e.into_inner()) =
            Some((newspec, new_sample_frames));

        self.framesize
            .store(newspec.frame_size() as i32, Ordering::Release);

        if self.recording {
            let mut capture: *mut IAudioCaptureClient = ptr::null_mut();
            // SAFETY: as above; `capture` receives the service.
            let ret = unsafe {
                (IAudioClient::vtbl(client).get_service)(
                    client,
                    &SDL_IID_IAUDIOCAPTURECLIENT,
                    ptr::from_mut(&mut capture).cast(),
                )
            };
            if failed(ret) {
                return Err(error_from_hresult(
                    Some("WASAPI can't get capture client service"),
                    ret,
                ));
            }

            crate::sdl_assert!(!capture.is_null());
            self.capture.store(capture, Ordering::Release);
            // SAFETY: as above.
            let ret = unsafe { (IAudioClient::vtbl(client).start)(client) };
            if failed(ret) {
                return Err(error_from_hresult(Some("WASAPI can't start capture"), ret));
            }

            self.flush(None); // MSDN says you should flush the recording endpoint right after startup.
        } else {
            let mut render: *mut IAudioRenderClient = ptr::null_mut();
            // SAFETY: as above; `render` receives the service.
            let ret = unsafe {
                (IAudioClient::vtbl(client).get_service)(
                    client,
                    &SDL_IID_IAUDIORENDERCLIENT,
                    ptr::from_mut(&mut render).cast(),
                )
            };
            if failed(ret) {
                return Err(error_from_hresult(
                    Some("WASAPI can't get render client service"),
                    ret,
                ));
            }

            crate::sdl_assert!(!render.is_null());
            self.render.store(render, Ordering::Release);
            // SAFETY: as above.
            let ret = unsafe { (IAudioClient::vtbl(client).start)(client) };
            if failed(ret) {
                return Err(error_from_hresult(Some("WASAPI can't start playback"), ret));
            }
        }

        Ok(()) // good to go.
    }

    /// Translation of `WASAPI_FlushRecording()` (`device` is `None` while
    /// preparing, where upstream's device isn't shutting down).
    fn flush(&self, device: Option<&PhysicalDevice>) {
        let mut ptr_: *mut u8 = ptr::null_mut();
        let mut frames: u32 = 0;
        let mut flags: u32 = 0;

        // just read until we stop getting packets, throwing them away.
        loop {
            let capture = self.capture.load(Ordering::Acquire);
            if device.is_some_and(PhysicalDevice::shutting_down) || capture.is_null() {
                break;
            }
            // SAFETY: a live capture client; valid out-pointers.
            let ret = unsafe {
                (IAudioCaptureClient::vtbl(capture).get_buffer)(
                    capture,
                    &mut ptr_,
                    &mut frames,
                    &mut flags,
                    ptr::null_mut(),
                    ptr::null_mut(),
                )
            };
            if ret == AUDCLNT_S_BUFFER_EMPTY {
                break; // no more buffered data; we're done.
            } else if self.wasapi_failed(ret) {
                break; // failed for some other reason, abort.
                       // SAFETY: we got `frames` frames from this client.
            } else if self.wasapi_failed(unsafe {
                (IAudioCaptureClient::vtbl(capture).release_buffer)(capture, frames)
            }) {
                break; // something broke.
            }
        }
    }
}

/// Write a mix format's sample rate and the byte rate that goes with it
/// (the `AUDCLNT_STREAMFLAGS_AUTOCONVERTPCM` branch of `mgmtthrtask_PrepDevice()`).
///
/// # Safety
///
/// `waveformat` must point to a writable `WAVEFORMATEX`.
unsafe fn set_wave_format_rate(waveformat: *mut WAVEFORMATEX, freq: u32) {
    // SAFETY: the caller's contract; the struct is packed.
    unsafe {
        let channels = ptr::addr_of!((*waveformat).nChannels).read_unaligned() as u32;
        let bits = ptr::addr_of!((*waveformat).wBitsPerSample).read_unaligned() as u32;
        ptr::addr_of_mut!((*waveformat).nSamplesPerSec).write_unaligned(freq);
        ptr::addr_of_mut!((*waveformat).nAvgBytesPerSec)
            .write_unaligned(freq * channels * (bits / 8));
    }
}

/// The closest SDL format to `requested` that WASAPI's mix format is (the
/// format loop of `mgmtthrtask_PrepDevice()`).
fn choose_format(
    requested: AudioFormat,
    wasapi_format: Option<AudioFormat>,
) -> Option<AudioFormat> {
    requested
        .closest_formats()
        .iter()
        .copied()
        .find(|&test_format| Some(test_format) == wasapi_format)
}

/// The callback size for a device period (in 100-nanosecond units), so
/// WaitDevice waits for one period at a time.
fn period_sample_frames(default_period: ReferenceTime, freq: i32) -> i32 {
    let period_millis = default_period as f32 / 10000.0f32;
    let period_frames = period_millis * freq as f32 / 1000.0f32;
    period_frames.ceil() as i32 // SDL_ceilf
}

/// The shared-mode period for `IAudioClient3_InitializeSharedAudioStream()`,
/// which only accepts integral multiples of the fundamental period.
fn shared_mode_period_in_frames(sample_frames: i32, fundamental: u32, min: u32, max: u32) -> u32 {
    let period_in_frames = fundamental
        .wrapping_mul(crate::stdlib::math::round(sample_frames as f64 / fundamental as f64) as u32);
    // SDL_clamp(), which (unlike `u32::clamp`) tolerates `max < min`.
    if period_in_frames < min {
        min
    } else if period_in_frames > max {
        max
    } else {
        period_in_frames
    }
}

impl DeviceBackend for Arc<WasapiDevice> {
    /// Translation of `WASAPI_ThreadInit()`.
    fn thread_init(&self, device: &PhysicalDevice) {
        // this thread uses COM.
        if succeeded(co_initialize()) {
            // can't report errors, hope it worked!
            self.coinitialized.store(true, Ordering::Release);
        }

        // Set this thread to very high "Pro Audio" priority.
        let set = LIBAVRT
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .as_ref()
            .and_then(|a| a.av_set_mm_thread_characteristics_w);
        if let Some(set) = set {
            let mut idx: u32 = 0;
            let name = utf8_to_wide("Pro Audio");
            // SAFETY: a NUL-terminated task name and a valid index out-pointer.
            let task = unsafe { set(name.as_ptr(), &mut idx) };
            self.task.store(task, Ordering::Release);
        } else {
            default_thread_init(device);
        }
    }

    /// Translation of `WASAPI_ThreadDeinit()`.
    fn thread_deinit(&self, _device: &PhysicalDevice) {
        // Set this thread back to normal priority.
        let revert = LIBAVRT
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .as_ref()
            .and_then(|a| a.av_revert_mm_thread_characteristics);
        let task = self.task.load(Ordering::Acquire);
        if let (false, Some(revert)) = (task.is_null(), revert) {
            // SAFETY: the handle AvSetMmThreadCharacteristicsW() gave this thread.
            unsafe { revert(task) };
            self.task.store(ptr::null_mut(), Ordering::Release);
        }

        if self.coinitialized.swap(false, Ordering::AcqRel) {
            co_uninitialize();
        }
    }

    /// Translation of `WASAPI_GetDeviceBuf()`.
    fn get_device_buf(&self, _device: &PhysicalDevice, buffer_size: usize) -> Option<usize> {
        // get an endpoint buffer from WASAPI.
        let mut buffer: *mut u8 = ptr::null_mut();

        let render = self.render.load(Ordering::Acquire);
        if !render.is_null() {
            let frames = self.sample_frames.load(Ordering::Acquire) as u32;
            // SAFETY: a live render client; `buffer` receives the endpoint buffer.
            let ret = unsafe {
                (IAudioRenderClient::vtbl(render).get_buffer)(render, frames, &mut buffer)
            };
            if ret == AUDCLNT_E_BUFFER_TOO_LARGE {
                crate::sdl_assert!(buffer.is_null());
                return Some(0); // just go back to WaitDevice and try again after the hardware has consumed some more data.
            } else if self.wasapi_failed(ret) {
                crate::sdl_assert!(buffer.is_null());
                if self.device_lost.load(Ordering::Acquire) {
                    // just use an available buffer, we won't be playing it anyhow.
                    return Some(0); // we'll recover during WaitDevice and try again.
                }
            }
        }

        // (a NULL buffer with the size unchanged is a failure to the core)
        if buffer.is_null() {
            return None;
        }
        self.render_buffer.store(buffer, Ordering::Release);
        Some(buffer_size)
    }

    /// Translation of `WASAPI_PlayDevice()`.
    fn play_device(&self, _device: &PhysicalDevice, buffer: &[u8]) -> bool {
        let render = self.render.load(Ordering::Acquire);
        if !render.is_null() && self.device_disconnecting.load(Ordering::Acquire) == 0 {
            // definitely activated?
            let frames = self.sample_frames.load(Ordering::Acquire) as u32;
            let endpoint = self.render_buffer.swap(ptr::null_mut(), Ordering::AcqRel);
            if !endpoint.is_null() {
                // (the core mixed into its own buffer; move it to the endpoint's)
                let n = buffer
                    .len()
                    .min(frames as usize * self.framesize.load(Ordering::Acquire) as usize);
                // SAFETY: GetBuffer() handed out room for `frames` frames.
                unsafe { ptr::copy_nonoverlapping(buffer.as_ptr(), endpoint, n) };
            }
            // WasapiFailed() will mark the device for reacquisition or removal elsewhere.
            // SAFETY: a live render client; we got `frames` frames from it.
            self.wasapi_failed(unsafe {
                (IAudioRenderClient::vtbl(render).release_buffer)(render, frames, 0)
            });
        }
        true
    }

    /// Translation of `WASAPI_WaitDevice()`.
    fn wait_device(&self, device: &PhysicalDevice) -> bool {
        // WaitDevice does not hold the device lock, so check for recovery/disconnect details here.
        while self.recover_wasapi_if_lost(device)
            && !self.client().is_null()
            && !self.event.load(Ordering::Acquire).is_null()
        {
            let client = self.client();
            let event = self.event.load(Ordering::Acquire);
            if device.recording {
                // Recording devices should return immediately if there is any data available
                let mut padding: u32 = 0;
                // SAFETY: a live client; a valid out-pointer.
                if !self.wasapi_failed(unsafe {
                    (IAudioClient::vtbl(client).get_current_padding)(client, &mut padding)
                }) {
                    //SDL_Log("WASAPI EVENT! padding=%u maxpadding=%u", (unsigned int)padding, (unsigned int)maxpadding);
                    if padding > 0 {
                        break;
                    }
                }

                // SAFETY: our event handle.
                match unsafe { WaitForSingleObjectEx(event, 200, 0) } {
                    WAIT_OBJECT_0 | WAIT_TIMEOUT => {}
                    _ => {
                        //SDL_Log("WASAPI FAILED EVENT!");
                        // SAFETY: a live client.
                        unsafe { (IAudioClient::vtbl(client).stop)(client) };
                        return false;
                    }
                }
            } else {
                // SAFETY: our event handle.
                let wait_result = unsafe { WaitForSingleObjectEx(event, 200, 0) };
                if wait_result == WAIT_OBJECT_0 {
                    let mut padding: u32 = 0;
                    // SAFETY: a live client; a valid out-pointer.
                    if !self.wasapi_failed(unsafe {
                        (IAudioClient::vtbl(client).get_current_padding)(client, &mut padding)
                    }) {
                        //SDL_Log("WASAPI EVENT! padding=%u maxpadding=%u", (unsigned int)padding, (unsigned int)maxpadding);
                        if padding <= self.sample_frames.load(Ordering::Acquire) as u32 {
                            break;
                        }
                    }
                } else if wait_result != WAIT_TIMEOUT {
                    //SDL_Log("WASAPI FAILED EVENT!");*/
                    // SAFETY: a live client.
                    unsafe { (IAudioClient::vtbl(client).stop)(client) };
                    return false;
                }
            }
        }

        true
    }

    /// Translation of `WASAPI_WaitDevice()` (also used for recording).
    fn wait_recording_device(&self, device: &PhysicalDevice) -> bool {
        self.wait_device(device)
    }

    /// Translation of `WASAPI_RecordDevice()`.
    fn record_device(&self, device: &PhysicalDevice, buffer: &mut [u8]) -> Result<usize> {
        let mut ptr_: *mut u8 = ptr::null_mut();
        let mut frames: u32 = 0;
        let mut flags: u32 = 0;
        let buflen = buffer.len();

        // FIXME (upstream): on a GetBuffer() failure other than an empty buffer,
        // this keeps calling it until the device is marked disconnecting.
        loop {
            let capture = self.capture.load(Ordering::Acquire);
            if capture.is_null() || self.device_disconnecting.load(Ordering::Acquire) != 0 {
                break;
            }
            // SAFETY: a live capture client; valid out-pointers.
            let ret = unsafe {
                (IAudioCaptureClient::vtbl(capture).get_buffer)(
                    capture,
                    &mut ptr_,
                    &mut frames,
                    &mut flags,
                    ptr::null_mut(),
                    ptr::null_mut(),
                )
            };
            if ret == AUDCLNT_S_BUFFER_EMPTY {
                return Ok(0); // in theory we should have waited until there was data, but oh well, we'll go back to waiting. Returning 0 is safe in SDL3.
            }

            self.wasapi_failed(ret); // mark device lost/failed if necessary.

            if ret == S_OK {
                let total = frames as usize * self.framesize.load(Ordering::Acquire) as usize;
                let cpy = buflen.min(total);
                let leftover = total - cpy;
                let silent = (flags & AUDCLNT_BUFFERFLAGS_SILENT as u32) != 0;

                crate::sdl_assert!(leftover == 0); // according to MSDN, this isn't everything available, just one "packet" of data per-GetBuffer call.

                if silent {
                    buffer[..cpy].fill(silence_value_of(device));
                } else {
                    // SAFETY: the packet holds `total` bytes; we copy `cpy` of them.
                    unsafe { ptr::copy_nonoverlapping(ptr_, buffer.as_mut_ptr(), cpy) };
                }

                // SAFETY: we got `frames` frames from this client.
                self.wasapi_failed(unsafe {
                    (IAudioCaptureClient::vtbl(capture).release_buffer)(capture, frames)
                });

                return Ok(cpy);
            }
        }

        Err(Error::new("WASAPI recording failed")) // unrecoverable error.
    }

    /// Translation of `WASAPI_FlushRecording()`.
    fn flush_recording(&self, device: &PhysicalDevice) {
        self.flush(Some(device));
    }

    /// Translation of `WASAPI_CloseDevice()`.
    fn close_device(&self, device: &PhysicalDevice) {
        self.reset_wasapi_device();
        if let Some(m) = OPENED.lock().unwrap_or_else(|e| e.into_inner()).as_mut() {
            m.remove(&device.handle);
        }
    }
}

/// The WASAPI driver (`WASAPI_Init()`).
struct Wasapi;

impl AudioDriverImpl for Wasapi {
    fn flags(&self) -> DriverFlags {
        DriverFlags {
            has_recording_support: true,
            ..DriverFlags::default()
        }
    }

    /// Translation of `WASAPI_DetectDevices()`.
    fn detect_devices(&self) -> (Option<Arc<PhysicalDevice>>, Option<Arc<PhysicalDevice>>) {
        // this blocks because it needs to finish before the audio subsystem inits
        type Defaults = (Option<Arc<PhysicalDevice>>, Option<Arc<PhysicalDevice>>);
        let data: Arc<Mutex<Defaults>> = Arc::new(Mutex::new((None, None)));
        let d = data.clone();
        // mgmtthrtask_DetectDevices
        let _ = proxy_and_wait(move || {
            *d.lock().unwrap_or_else(|e| e.into_inner()) = immdevice::enumerate_endpoints(
                AudioFormat::F32,
                SUPPORTS_RECORDING_ON_PLAYBACK_DEVICES.load(Ordering::Acquire),
            );
            Ok(())
        });
        let r = data.lock().unwrap_or_else(|e| e.into_inner()).clone();
        r
    }

    /// Translation of `WASAPI_OpenDevice()`.
    fn open_device(
        &self,
        device: &PhysicalDevice,
        state: &mut PhysState,
    ) -> Result<Arc<dyn DeviceBackend>> {
        // Initialize all variables that we clean on shutdown
        let h = Arc::new(WasapiDevice {
            device: find_physical_audio_device_by_handle(device.handle)
                .as_ref()
                .map_or_else(Weak::new, Arc::downgrade),
            recording: device.recording,
            waveformat: AtomicPtr::new(ptr::null_mut()),
            client: AtomicPtr::new(ptr::null_mut()),
            render: AtomicPtr::new(ptr::null_mut()),
            capture: AtomicPtr::new(ptr::null_mut()),
            event: AtomicPtr::new(ptr::null_mut()),
            task: AtomicPtr::new(ptr::null_mut()),
            coinitialized: AtomicBool::new(false),
            framesize: AtomicI32::new(0),
            device_disconnecting: AtomicI32::new(0),
            device_lost: AtomicBool::new(false),
            device_dead: AtomicBool::new(false),
            isplayback: AtomicBool::new(false),
            sample_frames: AtomicI32::new(state.sample_frames),
            render_buffer: AtomicPtr::new(ptr::null_mut()),
            prepared: Mutex::new(None),
        });
        OPENED
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get_or_insert_with(HashMap::new)
            .insert(device.handle, Arc::downgrade(&h));

        let fail = |e: Error| {
            DeviceBackend::close_device(&h, device); // (the core's WASAPI_CloseDevice() call)
            e
        };

        h.activate_wasapi_device(state.spec, state.sample_frames)
            .map_err(fail)?; // already set error.

        // (PrepDevice's SDL_AudioDeviceFormatChangedAlreadyLocked(), see the top of this file)
        let prepared = h.prepared.lock().unwrap_or_else(|e| e.into_inner()).take();
        if let Some((newspec, new_sample_frames)) = prepared {
            audio_device_format_changed_while_opening(device, state, &newspec, new_sample_frames)
                .map_err(fail)?;
        }
        h.sample_frames
            .store(state.sample_frames, Ordering::Release);

        /* Ready, but possibly waiting for async device activation.
        Until activation is successful, we will report silence from recording
        devices and ignore data on playback devices. Upon activation, we'll make
        sure any bound audio streams are adjusted for the final device format. */

        Ok(Arc::new(h))
    }

    /// Translation of `WASAPI_FreeDeviceHandle()`.
    fn free_device_handle(&self, device: &PhysicalDevice) {
        let handle = device.handle;
        // mgmtthrtask_FreeDeviceHandle
        let _ = proxy_and_wait(move || {
            immdevice::free_device_handle(handle);
            Ok(())
        });
    }

    /// Translation of `WASAPI_DeinitializeStart()`.
    fn deinitialize_start(&self) {
        // mgmtthrtask_DeinitializeStart
        let _ = proxy_and_wait(|| {
            stop_wasapi_hotplug();
            Ok(())
        });
    }

    /// Translation of `WASAPI_Deinitialize()`.
    fn deinitialize(&self) {
        deinit_management_thread();
    }
}

/// Translation of `WASAPI_Init()`.
fn wasapi_init() -> Option<Arc<dyn AudioDriverImpl>> {
    init_management_thread().ok()?;

    SUPPORTS_RECORDING_ON_PLAYBACK_DEVICES.store(
        crate::hints::get_bool(crate::hints::AUDIO_INCLUDE_MONITORS, false),
        Ordering::Release,
    );

    Some(Arc::new(Wasapi))
}

/// Translation of `WASAPI_bootstrap`.
pub(crate) static WASAPI_BOOTSTRAP: AudioBootStrap = AudioBootStrap {
    name: "wasapi",
    desc: "WASAPI",
    init: wasapi_init,
    demand_only: false,
    is_preferred: false,
};

#[cfg(test)]
mod tests;
