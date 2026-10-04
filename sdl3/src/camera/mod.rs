// Rust translation of src/camera/SDL_camera.c, SDL_syscamera.h,
// SDL_camera_c.h and include/SDL3/SDL_camera.h from Simple DirectMedia Layer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! Video capture from cameras.
//!
//! [`cameras`] lists the available devices; [`Camera::open`] opens one,
//! optionally asking for a [`CameraSpec`] (the closest the device supports
//! is chosen, and SDL scales and converts frames to what was asked for).
//! The user may have to approve access to the camera first: watch for
//! `CAMERA_DEVICE_APPROVED`/`CAMERA_DEVICE_DENIED` events, or poll
//! [`Camera::permission_state`].
//!
//! Frames arrive on a device thread; [`Camera::acquire_frame`] hands the
//! oldest one to the app as a [`CameraFrame`], which goes back to the
//! camera when dropped. Up to eight frames are buffered; while the app
//! holds them all, new frames are dropped.
//!
//! Upstream lends the app surfaces whose pixels may point straight at the
//! backend's (DMA) buffers. Here the backends hand over owned buffers, which
//! move into the app's surface without a copy and return to the backend
//! when the frame is dropped. A frame still held when its camera closes
//! stays valid (upstream's pointer dangles) and is freed when dropped.
//!
//! The camera backends are platform drivers; so far only the dummy
//! driver, which reports no cameras and is used only when requested with
//! [`hints::CAMERA_DRIVER`], exists. Without it, initializing the camera
//! subsystem fails with "No available camera driver", as upstream does on
//! a platform without camera support.

mod dummy;
#[cfg(target_os = "linux")]
mod v4l2;

use std::any::Any;
use std::cell::RefCell;
use std::cmp::Ordering as CmpOrdering;
use std::collections::{BTreeMap, VecDeque};
use std::ops::{Deref, DerefMut};
use std::sync::atomic::{AtomicBool, AtomicI32, Ordering};
use std::sync::{Arc, RwLock, RwLockReadGuard, RwLockWriteGuard};
use std::time::Duration;

use crate::error::{Error, Result};
use crate::events::{CameraDeviceEvent, Event, EventType};
use crate::hints;
use crate::properties::Properties;
use crate::thread::{ReentrantMutex, ReentrantMutexGuard, Thread, ThreadPriority};
use crate::video::surface::{
    convert_pixels, Pixels, ScaleMode, Surface, PROP_SURFACE_ROTATION_FLOAT,
};
use crate::video::{ColorRange, Colorspace, PixelFormat};

pub use crate::events::CameraID;

/// The details of an output format for a camera device. Translation of
/// `SDL_CameraSpec`.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default)]
pub struct CameraSpec {
    /// Frame format
    pub format: PixelFormat,
    /// Frame colorspace
    pub colorspace: Colorspace,
    /// Frame width
    pub width: i32,
    /// Frame height
    pub height: i32,
    /// Frame rate numerator ((num / denom) == FPS, (denom / num) == duration in seconds)
    pub framerate_numerator: i32,
    /// Frame rate denominator ((num / denom) == FPS, (denom / num) == duration in seconds)
    pub framerate_denominator: i32,
}

/// The position of a camera in relation to the system device. Translation
/// of `SDL_CameraPosition`.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default)]
pub enum CameraPosition {
    #[default]
    Unknown,
    FrontFacing,
    BackFacing,
}

/// Whether the user has allowed access to a camera. Translation of
/// `SDL_CameraPermissionState`.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug, Default)]
pub enum CameraPermissionState {
    Denied = -1,
    #[default]
    Pending = 0,
    Approved = 1,
}

/// What a backend's `acquire_frame` produced. Translation of
/// `SDL_CameraFrameResult` (with the frame for `SDL_CAMERA_FRAME_READY`).
pub(crate) enum CameraFrameResult {
    /// A fatal error: the device is treated as lost.
    Error,
    /// No frame available yet; not an error.
    #[allow(dead_code)] // (used by the camera drivers)
    Skip,
    /// A new frame.
    Ready(AcquiredFrame),
}

/// A frame from a backend: the pixels (in the device's actual format) and
/// what upstream's `AcquireFrame` writes through its out-parameters.
pub(crate) struct AcquiredFrame {
    /// The frame's pixels; returned to the backend's `release_frame` when
    /// SDL is done with them.
    pub(crate) pixels: Vec<u8>,
    /// The pitch; for YUV formats, of the (1-byte-per-pixel) Y plane.
    pub(crate) pitch: i32,
    /// The backend's timestamp of the frame, in nanoseconds.
    pub(crate) timestamp_ns: u64,
    /// Degrees (positive is clockwise) to rotate the image so it would be
    /// right-side up, in 90 degree increments.
    pub(crate) rotation: f32,
}

/// A backend's driver-wide entry points. Translation of the
/// `SDL_CameraDriverImpl` functions that aren't about an opened device.
pub(crate) trait CameraDriverImpl: Send + Sync {
    /// Translation of `DetectDevices`: call [`add_camera`] for every
    /// device found.
    fn detect_devices(&self);

    /// Translation of `OpenDevice`: open the device at `spec` and return
    /// its per-open interface (`hidden`). The backend calls
    /// [`camera_permission_outcome`] when the user decides about access.
    fn open_device(
        &self,
        device: &Arc<CameraDevice>,
        spec: &CameraSpec,
    ) -> Result<Arc<dyn CameraBackend>>;

    /// Translation of `FreeDeviceHandle`: SDL is done with this device;
    /// free the handle from [`add_camera`].
    fn free_device_handle(&self, _device: &CameraDevice) {}

    /// Translation of `Deinitialize`.
    fn deinitialize(&self) {}

    /// Translation of `ProvidesOwnCallbackThread`: the backend drives
    /// [`camera_thread_iterate`] itself instead of SDL's device thread.
    fn provides_own_callback_thread(&self) -> bool {
        false
    }
}

/// The entry points of an opened device. Translation of the per-device
/// `SDL_CameraDriverImpl` functions (and `SDL_PrivateCameraData`).
///
/// `wait_device` runs on the device thread without the device lock;
/// `acquire_frame` with it held, so it SHOULD NOT BLOCK. `release_frame`
/// can be called from any thread.
pub(crate) trait CameraBackend: Send + Sync {
    /// Translation of `WaitDevice`: block until a frame may be available.
    /// Returning false treats the device as lost.
    fn wait_device(&self, device: &CameraDevice) -> bool;

    /// Translation of `AcquireFrame`.
    fn acquire_frame(&self, device: &CameraDevice) -> CameraFrameResult;

    /// Translation of `ReleaseFrame`: reclaim a frame's pixels.
    fn release_frame(&self, device: &CameraDevice, pixels: Vec<u8>);

    /// Translation of `CloseDevice`.
    fn close_device(&self, device: &CameraDevice);
}

/// Translation of `CameraBootStrap`.
pub(crate) struct CameraBootStrap {
    pub(crate) name: &'static str,
    pub(crate) desc: &'static str,
    pub(crate) init: fn() -> Result<Arc<dyn CameraDriverImpl>>,
    /// if true: request explicitly, or it won't be available.
    pub(crate) demand_only: bool,
}

/// Available camera drivers. Translation of `bootstrap`.
/// (The test build puts its test driver first; it's demand-only.)
static BOOTSTRAP: &[&CameraBootStrap] = &[
    #[cfg(test)]
    &tests::TESTCAMERA_BOOTSTRAP,
    #[cfg(target_os = "linux")]
    &v4l2::V4L2_BOOTSTRAP,
    &dummy::DUMMYCAMERA_BOOTSTRAP,
];

/// The number of output surfaces a device buffers frames in.
const NUM_OUTPUT_SURFACES: usize = 8;

/// An output surface and its place in the frame queues. Translation of
/// `SurfaceList`.
#[derive(Default)]
struct OutputSlot {
    /// `None` while the app holds the frame (or before the surfaces are
    /// prepared).
    surface: Option<Surface<'static>>,
    timestamp_ns: u64,
    /// The surface's pixels are the zombie frame's (not the backend's).
    zombie_frame: bool,
}

/// The fields of `SDL_Camera` behind its `lock`.
#[derive(Default)]
struct CameraState {
    /// The zombie implementations replace the backend's (upstream swaps
    /// the function pointers).
    zombie_ops: bool,
    /// The device's actual specification that the camera is outputting, before conversion.
    actual_spec: CameraSpec,
    /// The device's current camera specification, after conversions.
    spec: CameraSpec,
    /// Dropping the first frame(s) after open seems to help timing on some platforms.
    drop_frames: i32,
    /// Backend timestamp of first acquired frame, so we can keep these meaningful regardless of epoch.
    base_timestamp: u64,
    /// SDL timestamp of first acquired frame, so we can roughly convert to SDL ticks.
    adjust_timestamp: u64,
    /// Pixel data flows from the driver into these, then gets converted for the app if necessary.
    acquire_surface: Option<Surface<'static>>,
    /// acquire_surface converts or scales to this surface before landing in output_surfaces, if necessary.
    conversion_surface: Option<Surface<'static>>,
    /// The surfaces that buffer converted/scaled frames of video until the app claims them.
    output_surfaces: Vec<OutputSlot>,
    /// Filled output surfaces, newest first (this is FIFO).
    filled_output_surfaces: VecDeque<usize>,
    /// Empty output surfaces; the next one is last (this is LIFO).
    empty_output_surfaces: Vec<usize>,
    /// Output surfaces the app holds, newest first.
    app_held_output_surfaces: Vec<usize>,
    /// A fake video frame we allocate if the camera fails/disconnects.
    zombie_pixels: Option<Vec<u8>>,
    /// non-zero if acquire_surface needs to be scaled for final output.
    /// -1: downscale, 0: no scaling, 1: upscale
    needs_scaling: i32,
    /// true if acquire_surface needs to be converted for final output.
    needs_conversion: bool,
    /// A thread to feed the camera device
    thread: Option<Thread>,
    /// Optional properties.
    props: Option<Properties>,
    /// Current state of user permission check.
    permission: CameraPermissionState,
    /// Data private to this driver, used when device is opened and running.
    hidden: Option<Arc<dyn CameraBackend>>,
    /// Counts closes, so frames from an earlier opening aren't taken back.
    generation: u64,
}

type DeviceGuard<'a> = ReentrantMutexGuard<'a, RefCell<CameraState>>;

/// A camera device, whether it is opened or not. Translation of
/// `SDL_Camera` (upstream doesn't separate physical and logical devices).
pub(crate) struct CameraDevice {
    /// A mutex for locking
    lock: ReentrantMutex<RefCell<CameraState>>,
    /// When refcount hits zero, we destroy the device object.
    refcount: AtomicI32,
    /// Human-readable device name.
    pub(crate) name: String,
    /// Position of camera (front-facing, back-facing, etc).
    pub(crate) position: CameraPosition,
    /// All supported formats/dimensions for this device.
    pub(crate) all_specs: Vec<CameraSpec>,
    /// Unique value assigned at creation time.
    pub(crate) instance_id: CameraID,
    /// Driver-specific hardware data on how to open device.
    #[allow(dead_code)] // (used by the camera drivers)
    pub(crate) handle: Box<dyn Any + Send + Sync>,
    /// Current state flags
    shutdown: AtomicBool,
    zombie: AtomicBool,
}

impl CameraDevice {
    fn lock(&self) -> DeviceGuard<'_> {
        self.lock.lock()
    }

    /// Whether the device thread is being asked to end.
    #[allow(dead_code)] // (used by the camera drivers)
    pub(crate) fn shutting_down(&self) -> bool {
        self.shutdown.load(Ordering::Acquire)
    }

    /// The device's actual (hardware) specification.
    #[allow(dead_code)] // (used by the camera drivers)
    pub(crate) fn actual_spec(&self) -> CameraSpec {
        self.lock().borrow().actual_spec
    }
}

/// Translation of `SDL_CameraDriver` (`camera_driver`), behind its
/// `device_hash_lock`.
#[derive(Default)]
struct CurrentCamera {
    /// The name of this camera driver
    name: Option<&'static str>,
    /// The description of this camera driver
    #[allow(dead_code)]
    desc: Option<&'static str>,
    /// the backend's interface
    driver: Option<Arc<dyn CameraDriverImpl>>,
    /// the collection of currently-available camera devices
    device_hash: Option<BTreeMap<CameraID, Arc<CameraDevice>>>,
    pending_events: Vec<(EventType, CameraID)>,
    device_count: i32,
    /// true during SDL_Quit, so we known not to accept any last-minute device hotplugs.
    shutting_down: bool,
}

static CAMERA_DRIVER: RwLock<Option<CurrentCamera>> = RwLock::new(None);

fn driver_read() -> RwLockReadGuard<'static, Option<CurrentCamera>> {
    CAMERA_DRIVER.read().unwrap_or_else(|e| e.into_inner())
}

fn driver_write() -> RwLockWriteGuard<'static, Option<CurrentCamera>> {
    CAMERA_DRIVER.write().unwrap_or_else(|e| e.into_inner())
}

/// The current driver's interface (cloned out so it's called without the
/// lock).
fn current_driver() -> Option<Arc<dyn CameraDriverImpl>> {
    driver_read().as_ref().and_then(|c| c.driver.clone())
}

/// The number of built-in camera drivers. Translation of
/// `SDL_GetNumCameraDrivers()`.
pub fn num_camera_drivers() -> usize {
    BOOTSTRAP.len()
}

/// The name of a built-in camera driver ("v4l2", "coremedia", ...).
/// Translation of `SDL_GetCameraDriver()`.
pub fn camera_driver(index: usize) -> Result<&'static str> {
    BOOTSTRAP
        .get(index)
        .map(|b| b.name)
        .ok_or_else(|| Error::invalid_param("index"))
}

/// The name of the current camera driver, or `None` if the camera
/// subsystem isn't initialized. Translation of `SDL_GetCurrentCameraDriver()`.
pub fn current_camera_driver() -> Option<&'static str> {
    driver_read().as_ref().and_then(|c| c.name)
}

/// A standardized name for a thread to power a specific camera device.
/// Translation of `SDL_GetCameraThreadName()`.
pub(crate) fn camera_thread_name(device: &CameraDevice) -> String {
    format!("SDLCamera{}", device.instance_id as i32)
}

/// Common utility functionality to gather up camera specs (not required).
/// Translation of `SDL_AddCameraFormat()`.
#[allow(dead_code)] // (used by the camera drivers)
pub(crate) fn add_camera_format(
    specs: &mut Vec<CameraSpec>,
    format: PixelFormat,
    colorspace: Colorspace,
    w: i32,
    h: i32,
    framerate_numerator: i32,
    framerate_denominator: i32,
) {
    specs.push(CameraSpec {
        format,
        colorspace,
        width: w,
        height: h,
        framerate_numerator,
        framerate_denominator,
    });
}

// Zombie device implementation...

// These get used when a device is disconnected or fails. Apps that ignore the
//  loss notifications will get black frames but otherwise keep functioning.

/// Frames per second a zombie camera assumes when its spec has no usable
/// framerate.
const ZOMBIE_FALLBACK_FPS: u32 = 30;

/// How long a zombie camera waits between frames: one frame interval at
/// the spec's framerate. Part of `ZombieWaitDevice()`.
fn zombie_frame_delay_ms(spec: &CameraSpec) -> u32 {
    // (upstream divides by the numerator unchecked: a device whose specs
    // carry no framerate (0/0, as a device that lists no specs gets) makes
    // the delay infinite or NaN, and converting that to Uint32 is undefined
    // behavior, in practice no delay, so the zombie thread spins. Fixed
    // here by assuming a fallback rate when the framerate isn't positive.)
    if spec.framerate_numerator <= 0 || spec.framerate_denominator <= 0 {
        return 1000 / ZOMBIE_FALLBACK_FPS;
    }
    // !!! FIXME: this is bad for several reasons (uses double, could be precalculated, doesn't track elapsed time).
    let duration = spec.framerate_denominator as f64 / spec.framerate_numerator as f64;
    (duration * 1000.0) as u32
}

/// Translation of `ZombieWaitDevice()`.
fn zombie_wait_device(device: &CameraDevice) -> bool {
    if !device.shutdown.load(Ordering::Acquire) {
        let ms = zombie_frame_delay_ms(&device.actual_spec());
        crate::timer::delay(Duration::from_millis(ms as u64));
    }
    true
}

/// Translation of `GetFrameBufLen()`.
fn frame_buf_len(spec: &CameraSpec) -> usize {
    let w = spec.width as usize;
    let h = spec.height as usize;
    let wxh = w * h;
    let fmt = spec.format;

    match fmt {
        // Some YUV formats have a larger Y plane than their U or V planes.
        PixelFormat::YV12 | PixelFormat::IYUV | PixelFormat::NV12 | PixelFormat::NV21 => {
            return wxh + (wxh / 2);
        }
        _ => {}
    }

    // this is correct for most things.
    wxh * fmt.bytes_per_pixel() as usize
}

/// Translation of `ZombieAcquireFrame()`.
fn zombie_acquire_frame(st: &mut CameraState) -> CameraFrameResult {
    let spec = st.actual_spec;

    let zombie_pixels = st.zombie_pixels.get_or_insert_with(|| {
        // attempt to allocate and initialize a fake frame of pixels.
        let buflen = frame_buf_len(&spec);
        let mut dst = vec![0u8; buflen];
        let wxh = (spec.width as usize) * (spec.height as usize);
        match spec.format {
            // in YUV formats, the U and V values must be 128 to get a black frame. If set to zero, it'll be bright green.
            PixelFormat::YV12 | PixelFormat::IYUV | PixelFormat::NV12 | PixelFormat::NV21 => {
                dst[..wxh].fill(0); // set Y to zero.
                dst[wxh..wxh + wxh / 2].fill(128); // set U and V to 128.
            }

            PixelFormat::YUY2 | PixelFormat::YVYU => {
                // Interleaved Y1[U1|V1]Y2[U2|V2].
                for chunk in dst.chunks_mut(4) {
                    for (b, v) in chunk.iter_mut().zip([0, 128, 0, 128]) {
                        *b = v;
                    }
                }
            }

            PixelFormat::UYVY => {
                // Interleaved [U1|V1]Y1[U2|V2]Y2.
                for chunk in dst.chunks_mut(4) {
                    for (b, v) in chunk.iter_mut().zip([128, 0, 128, 0]) {
                        *b = v;
                    }
                }
            }

            _ => {
                // just zero everything else, it'll _probably_ be okay.
            }
        }
        dst
    });

    // SDL (currently) wants the pitch of YUV formats to be the pitch of the (1-byte-per-pixel) Y plane.
    let mut pitch = spec.width;
    if !spec.format.is_fourcc() {
        // checking if it's not FOURCC to only do this for non-YUV data is good enough for now.
        pitch *= spec.format.bytes_per_pixel() as i32;
    }

    // (upstream lends every frame the same buffer; frames here own theirs)
    CameraFrameResult::Ready(AcquiredFrame {
        pixels: zombie_pixels.clone(),
        pitch,
        timestamp_ns: crate::timer::ticks_ns(),
        rotation: 0.0,
    }) // frame is available.
}

/// Give a frame's pixels back: to the backend, unless they are the zombie
/// frame's (`ZombieReleaseFrame()`/the backend's `ReleaseFrame`).
fn release_pixels(
    device: &CameraDevice,
    backend: &Option<Arc<dyn CameraBackend>>,
    pixels: Vec<u8>,
    zombie_frame: bool,
) {
    if !zombie_frame {
        // this was a frame from before the disconnect event; let the backend make an attempt to free it.
        if let Some(backend) = backend {
            backend.release_frame(device, pixels);
        }
    }
    // we just leave zombie_pixels alone, as we'll reuse it for every new frame until the camera is closed.
}

/// Move the pixels out of a surface (leaving it without pixels).
fn take_pixels(surface: &mut Surface<'static>) -> Option<Vec<u8>> {
    match std::mem::replace(&mut surface.pixels, Pixels::None) {
        Pixels::Owned { buf, .. } => {
            surface.pitch = 0;
            Some(buf)
        }
        other => {
            surface.pixels = other;
            None
        }
    }
}

/// Lend a surface owned pixels (as upstream points `pixels` at the
/// backend's buffer).
fn put_pixels(surface: &mut Surface<'static>, pixels: Vec<u8>, pitch: i32) {
    let len = pixels.len();
    surface.pixels = Pixels::Owned {
        buf: pixels,
        offset: 0,
        len,
    };
    surface.pitch = pitch;
}

/// Translation of `ClosePhysicalCamera()`.
fn close_physical_camera(device: &Arc<CameraDevice>) {
    if device.lock().borrow().hidden.is_none() {
        return; // device is not open.
    }

    device.shutdown.store(true, Ordering::Release);

    // !!! FIXME: the close_cond stuff from audio might help the race condition here.

    let thread = device.lock().borrow_mut().thread.take();
    if let Some(thread) = thread {
        thread.wait();
    }

    let guard = obtain_physical_camera_obj(device);

    // release frames that are queued up somewhere...
    let (backend, to_release) = {
        let mut st = guard.borrow_mut();
        let mut to_release = Vec::new();
        if !st.needs_conversion && st.needs_scaling == 0 {
            let filled: Vec<usize> = st.filled_output_surfaces.iter().copied().collect();
            for i in filled {
                let slot = &mut st.output_surfaces[i];
                let zombie_frame = slot.zombie_frame;
                if let Some(pixels) = slot.surface.as_mut().and_then(take_pixels) {
                    to_release.push((pixels, zombie_frame));
                }
            }
            // (frames the app holds are its own now; see CameraFrame)
        }
        (st.hidden.clone(), to_release)
    };
    for (pixels, zombie_frame) in to_release {
        release_pixels(device, &backend, pixels, zombie_frame);
    }

    if let Some(backend) = &backend {
        backend.close_device(device);
    }

    {
        let mut st = guard.borrow_mut();
        st.hidden = None; // just in case backend didn't reset this.

        st.props = None;

        st.acquire_surface = None;
        st.conversion_surface = None;
        st.output_surfaces.clear();

        st.permission = CameraPermissionState::Pending;
        st.zombie_pixels = None;
        st.filled_output_surfaces.clear();
        st.empty_output_surfaces.clear();
        st.app_held_output_surfaces.clear();

        st.base_timestamp = 0;
        st.adjust_timestamp = 0;

        st.spec = CameraSpec::default();
        st.generation += 1;
    }
    unref_physical_camera(device); // we're closed, release a reference.

    release_camera(device, guard);
}

/// Translation of `RefPhysicalCamera()`.
pub(crate) fn ref_physical_camera(device: &CameraDevice) {
    device.refcount.fetch_add(1, Ordering::AcqRel);
}

/// Don't hold the device lock when calling this, as we may destroy the
/// device! Translation of `UnrefPhysicalCamera()`.
pub(crate) fn unref_physical_camera(device: &Arc<CameraDevice>) {
    if device.refcount.fetch_sub(1, Ordering::AcqRel) == 1 {
        // take it out of the device list.
        let removed = {
            let mut cam = driver_write();
            match cam.as_mut() {
                Some(cam) => {
                    let removed = cam
                        .device_hash
                        .as_mut()
                        .and_then(|h| h.remove(&device.instance_id))
                        .is_some();
                    if removed {
                        cam.device_count -= 1;
                    }
                    removed
                }
                None => false,
            }
        };
        // (upstream's hash table calls DestroyCameraHashItem with the lock held)
        if removed {
            destroy_camera_hash_item(device);
        }
    }
}

/// Translation of `ObtainPhysicalCameraObj()`: reference and lock a device.
fn obtain_physical_camera_obj(device: &CameraDevice) -> DeviceGuard<'_> {
    ref_physical_camera(device);
    device.lock()
}

/// Translation of `ObtainPhysicalCamera()`: find a device by instance ID
/// and reference it (lock it with [`CameraDevice::lock`]).
fn obtain_physical_camera(devid: CameraID) -> Result<Arc<CameraDevice>> {
    if current_camera_driver().is_none() {
        return Err(Error::new("Camera subsystem is not initialized"));
    }

    let device = driver_read()
        .as_ref()
        .and_then(|c| c.device_hash.as_ref())
        .and_then(|h| h.get(&devid).cloned());
    match device {
        None => Err(Error::new("Invalid camera device instance ID")),
        Some(device) => {
            ref_physical_camera(&device);
            Ok(device)
        }
    }
}

/// Translation of `ReleaseCamera()`: unlock and unreference.
fn release_camera(device: &Arc<CameraDevice>, guard: DeviceGuard<'_>) {
    drop(guard);
    unref_physical_camera(device);
}

/// Run `f` with a device found by instance ID locked.
fn with_camera<R>(
    devid: CameraID,
    f: impl FnOnce(&Arc<CameraDevice>, &DeviceGuard<'_>) -> R,
) -> Result<R> {
    let device = obtain_physical_camera(devid)?;
    let guard = device.lock();
    let r = f(&device, &guard);
    release_camera(&device, guard);
    Ok(r)
}

// we want these sorted by format first, so you can find a block of all
// resolutions that are supported for a format. The formats are sorted in
// "best" order, but that's subjective: right now, we prefer planar
// formats, since they're likely what the cameras prefer to produce
// anyhow, and they basically send the same information in less space
// than an RGB-style format. After that, sort by bits-per-pixel.

// we want specs sorted largest to smallest dimensions, larger width taking precedence over larger height.
/// Translation of `CameraSpecCmp()`.
#[allow(dead_code)] // (used through add_camera)
fn camera_spec_cmp(a: &CameraSpec, b: &CameraSpec) -> CmpOrdering {
    use CmpOrdering::{Equal, Greater, Less};

    // driver shouldn't send specs like this, check here since we're eventually going to sniff the whole array anyhow.
    crate::sdl_assert!(a.format != PixelFormat::UNKNOWN);
    crate::sdl_assert!(a.width > 0);
    crate::sdl_assert!(a.height > 0);
    crate::sdl_assert!(b.format != PixelFormat::UNKNOWN);
    crate::sdl_assert!(b.width > 0);
    crate::sdl_assert!(b.height > 0);

    let afmt = a.format;
    let bfmt = b.format;
    if afmt.is_fourcc() && !bfmt.is_fourcc() {
        return Less;
    } else if !afmt.is_fourcc() && bfmt.is_fourcc() {
        return Greater;
    } else if afmt.bits_per_pixel() > bfmt.bits_per_pixel() {
        return Less;
    } else if bfmt.bits_per_pixel() > afmt.bits_per_pixel() {
        return Greater;
    } else if a.width > b.width {
        return Less;
    } else if b.width > a.width {
        return Greater;
    } else if a.height > b.height {
        return Less;
    } else if b.height > a.height {
        return Greater;
    }

    // still here? We care about framerate less than format or size, but faster is better than slow.
    if a.framerate_numerator != 0 && b.framerate_numerator == 0 {
        return Less;
    } else if a.framerate_numerator == 0 && b.framerate_numerator != 0 {
        return Greater;
    }

    let fpsa = a.framerate_numerator as f32 / a.framerate_denominator as f32;
    let fpsb = b.framerate_numerator as f32 / b.framerate_denominator as f32;
    if fpsa > fpsb {
        return Less;
    } else if fpsb > fpsa {
        return Greater;
    }

    if a.colorspace.range() == ColorRange::Full && b.colorspace.range() != ColorRange::Full {
        return Less;
    }
    if a.colorspace.range() != ColorRange::Full && b.colorspace.range() == ColorRange::Full {
        return Greater;
    }

    Equal // apparently, they're equal.
}

/// The camera backends call this when a new device is plugged in (and for
/// every device found by `detect_devices`). Translation of `SDL_AddCamera()`.
#[allow(dead_code)] // (used by the camera drivers)
pub(crate) fn add_camera(
    name: &str,
    position: CameraPosition,
    specs: &[CameraSpec],
    handle: Box<dyn Any + Send + Sync>,
) -> Option<Arc<CameraDevice>> {
    let shutting_down = driver_read().as_ref().is_none_or(|c| c.shutting_down);
    if shutting_down {
        return None; // we're shutting down, don't add any devices that are hotplugged at the last possible moment.
    }

    let mut all_specs = specs.to_vec();
    if !all_specs.is_empty() {
        // (SDL_qsort isn't stable; this sort is, so specs that compare
        // equal keep their order.)
        all_specs.sort_by(camera_spec_cmp);

        // weed out duplicates, just in case.
        all_specs.dedup();
    }

    let device = Arc::new(CameraDevice {
        lock: ReentrantMutex::new(RefCell::new(CameraState::default())),
        refcount: AtomicI32::new(0),
        name: name.to_owned(),
        position,
        all_specs,
        instance_id: crate::utils::next_object_id(),
        handle,
        shutdown: AtomicBool::new(false),
        zombie: AtomicBool::new(false),
    });
    ref_physical_camera(&device);

    let mut cam = driver_write();
    let cam = cam.as_mut()?;
    let hash = cam.device_hash.as_mut()?;
    if hash.contains_key(&device.instance_id) {
        return None;
    }
    hash.insert(device.instance_id, device.clone());
    cam.device_count += 1;

    // Add a device add event to the pending list, to be pushed when the event queue is pumped (away from any of our internal threads).
    cam.pending_events
        .push((EventType::CAMERA_DEVICE_ADDED, device.instance_id));

    Some(device)
}

/// Called when a device is removed from the system, or it fails
/// unexpectedly, from any thread, possibly even the camera device's thread.
/// Translation of `SDL_CameraDisconnected()`.
pub(crate) fn camera_disconnected(device: &Arc<CameraDevice>) {
    // Save off removal info in a list so we can send events for each, next
    //  time the event queue pumps, in case something tries to close a device
    //  from an event filter, as this would risk deadlocks and other disasters
    //  if done from the device thread.
    let guard = obtain_physical_camera_obj(device);

    let first_disconnect = device
        .zombie
        .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
        .is_ok();
    if first_disconnect {
        // if already disconnected this device, don't do it twice.
        // Swap in "Zombie" versions of the usual platform interfaces, so the device will keep
        // making progress until the app closes it.
        let mut st = guard.borrow_mut();
        st.zombie_ops = true;

        // Zombie functions will just report the timestamp as SDL_GetTicksNS(), so we don't need to adjust anymore to get it to match.
        st.adjust_timestamp = 0;
        st.base_timestamp = 0;
        drop(st);

        unref_physical_camera(device); // camera is disconnected, drop its reference
    }

    release_camera(device, guard);

    if first_disconnect {
        if let Some(cam) = driver_write().as_mut() {
            cam.pending_events
                .push((EventType::CAMERA_DEVICE_REMOVED, device.instance_id));
        }
    }
}

/// Backends call this when the user has approved/denied access to a
/// camera. Translation of `SDL_CameraPermissionOutcome()`.
#[allow(dead_code)] // (used by the camera drivers)
pub(crate) fn camera_permission_outcome(device: &Arc<CameraDevice>, approved: bool) {
    let permission = if approved {
        CameraPermissionState::Approved
    } else {
        CameraPermissionState::Denied
    };

    let guard = obtain_physical_camera_obj(device);
    let changed = {
        let mut st = guard.borrow_mut();
        let changed = st.permission != permission;
        st.permission = permission;
        changed
    };
    release_camera(device, guard);

    if changed {
        let event_type = if approved {
            EventType::CAMERA_DEVICE_APPROVED
        } else {
            EventType::CAMERA_DEVICE_DENIED
        };
        if let Some(cam) = driver_write().as_mut() {
            cam.pending_events.push((event_type, device.instance_id));
        }
    }
}

/// Find a device selected by a callback. DOES NOT LOCK THE DEVICE.
/// Translation of `SDL_FindPhysicalCameraByCallback()`.
#[allow(dead_code)] // (used by the camera drivers)
pub(crate) fn find_physical_camera_by_callback(
    callback: impl Fn(&CameraDevice) -> bool,
) -> Result<Arc<CameraDevice>> {
    if current_camera_driver().is_none() {
        return Err(Error::new("Camera subsystem is not initialized"));
    }

    driver_read()
        .as_ref()
        .and_then(|c| c.device_hash.as_ref())
        .and_then(|h| h.values().find(|d| callback(d)).cloned())
        .ok_or_else(|| Error::new("Device not found"))
}

/// The human-readable device name of a camera. Translation of
/// `SDL_GetCameraName()`.
pub fn camera_name(instance_id: CameraID) -> Result<String> {
    with_camera(instance_id, |device, _| device.name.clone())
}

/// The position of a camera in relation to the system ([`Unknown`] if
/// the ID is invalid). Translation of `SDL_GetCameraPosition()`.
///
/// [`Unknown`]: CameraPosition::Unknown
pub fn camera_position(instance_id: CameraID) -> CameraPosition {
    with_camera(instance_id, |device, _| device.position).unwrap_or_default()
}

/// The cameras currently connected. Translation of `SDL_GetCameras()`.
pub fn cameras() -> Result<Vec<CameraID>> {
    if current_camera_driver().is_none() {
        return Err(Error::new("Camera subsystem is not initialized"));
    }

    let cam = driver_read();
    let Some(cam) = cam.as_ref() else {
        return Ok(Vec::new());
    };
    let result: Vec<CameraID> = cam
        .device_hash
        .iter()
        .flat_map(|h| h.keys().copied())
        .collect();
    crate::sdl_assert!(result.len() == cam.device_count as usize);
    Ok(result)
}

/// The formats a camera supports, from "best" to "worst" (planar formats
/// first, then by bits per pixel, then largest to smallest size). It may
/// be empty if the camera doesn't say; then any spec is accepted.
/// Translation of `SDL_GetCameraSupportedFormats()`.
pub fn camera_supported_formats(instance_id: CameraID) -> Result<Vec<CameraSpec>> {
    with_camera(instance_id, |device, _| device.all_specs.clone())
}

// Camera device thread. This is split into chunks, so drivers that need to control this directly can use the pieces they need without duplicating effort.

/// Translation of `SDL_CameraThreadSetup()`.
pub(crate) fn camera_thread_setup(_device: &CameraDevice) {
    // The camera capture is always a high priority thread
    let _ = crate::thread::set_current_thread_priority(ThreadPriority::High);
}

/// One iteration of the device thread: acquire a frame, then convert or
/// scale it if needed and queue it for the app. Returns false when the
/// thread should end. Translation of `SDL_CameraThreadIterate()`.
pub(crate) fn camera_thread_iterate(device: &Arc<CameraDevice>) -> bool {
    let guard = device.lock();

    if device.shutdown.load(Ordering::Acquire) {
        return false; // we're done, shut it down.
    }

    let permission = guard.borrow().permission;
    if permission <= CameraPermissionState::Pending {
        // if permission was denied, shut it down. if undecided, we're done for now.
        return permission == CameraPermissionState::Pending;
    }

    let mut failed = false; // set to true if disaster worthy of treating the device as lost has happened.
    let mut work = None;

    let (zombie_ops, backend) = {
        let st = guard.borrow();
        (st.zombie_ops, st.hidden.clone())
    };

    // AcquireFrame SHOULD NOT BLOCK, as we are holding a lock right now. Block in WaitDevice instead!
    let rc = if zombie_ops {
        zombie_acquire_frame(&mut guard.borrow_mut())
    } else {
        match &backend {
            Some(backend) => backend.acquire_frame(device),
            None => CameraFrameResult::Error,
        }
    };

    match rc {
        CameraFrameResult::Ready(frame) => {
            // new frame acquired!
            let mut st = guard.borrow_mut();
            if st.drop_frames > 0 {
                st.drop_frames -= 1;
                drop(st);
                release_pixels(device, &backend, frame.pixels, zombie_ops);
            } else if st.empty_output_surfaces.is_empty() {
                // uhoh, no output frames available! Either the app is slow, or it forgot to release frames when done with them. Drop this new frame.
                drop(st);
                release_pixels(device, &backend, frame.pixels, zombie_ops);
            } else {
                if st.adjust_timestamp == 0 {
                    st.adjust_timestamp = crate::timer::ticks_ns();
                    st.base_timestamp = frame.timestamp_ns;
                }
                let timestamp_ns = frame
                    .timestamp_ns
                    .wrapping_sub(st.base_timestamp)
                    .wrapping_add(st.adjust_timestamp);

                let slot = st.empty_output_surfaces.pop().unwrap_or_default();
                st.output_surfaces[slot].timestamp_ns = timestamp_ns;
                let output = st.output_surfaces[slot].surface.take();
                let acquire = st.acquire_surface.take();
                let conversion = st.conversion_surface.take();
                let scaling = (st.needs_scaling, st.needs_conversion);
                work = Some((slot, output, acquire, conversion, scaling, frame));
            }
        }
        CameraFrameResult::Skip => {} // no frame available yet; not an error.
        CameraFrameResult::Error => {
            failed = true; // fatal error!
        }
    }

    // we can let go of the lock once we've tried to grab a frame of video and maybe moved the output frame off the empty list.
    // this lets us chew up the CPU for conversion and scaling without blocking other threads.
    drop(guard);

    if failed {
        crate::sdl_assert!(work.is_none());
        camera_disconnected(device); // doh.
    } else if let Some((slot, output, mut acquired, mut conversion, scaling, frame)) = work {
        // we have a new frame, scale/convert if necessary and queue it for the app!
        let (needs_scaling, needs_conversion) = scaling;
        let Some(mut output) = output else {
            return true;
        };
        let mut zombie_frame = false;

        if needs_scaling == 0 && !needs_conversion {
            // no conversion needed? Just move the pixels into the output surface.
            if let Some(acquired) = &acquired {
                output.w = acquired.w;
                output.h = acquired.h;
            }
            put_pixels(&mut output, frame.pixels, frame.pitch);
            zombie_frame = zombie_ops;
        } else if let Some(src) = acquired.as_mut() {
            // convert/scale into a different surface.
            put_pixels(src, frame.pixels, frame.pitch);
            convert_and_scale(
                src,
                conversion.as_mut(),
                &mut output,
                needs_scaling,
                needs_conversion,
            );

            // we made a copy, so we can give the driver back its resources.
            if let Some(pixels) = take_pixels(src) {
                release_pixels(device, &backend, pixels, zombie_ops);
            }
        }

        let _ = output
            .properties()
            .set(PROP_SURFACE_ROTATION_FLOAT, frame.rotation);

        // make the filled output surface available to the app.
        let guard = device.lock();
        let mut st = guard.borrow_mut();
        if st.output_surfaces.len() > slot {
            st.output_surfaces[slot].surface = Some(output);
            st.output_surfaces[slot].zombie_frame = zombie_frame;
            st.acquire_surface = acquired;
            st.conversion_surface = conversion;
            st.filled_output_surfaces.push_front(slot);
        }
    }

    true // always go on if not shutting down, even if device failed.
}

/// The scaling and conversion steps of [`camera_thread_iterate`]:
/// downscaling happens first, upscaling last, with `conversion` as the
/// middleman when both scaling and conversion are needed.
fn convert_and_scale(
    src: &mut Surface<'static>,
    conversion: Option<&mut Surface<'static>>,
    output: &mut Surface<'static>,
    needs_scaling: i32,
    needs_conversion: bool,
) {
    // !!! FIXME: linear scale? letterboxing?
    let convert = |src: &Surface<'_>, dst: &mut Surface<'_>| {
        let (w, h, pitch, format) = (src.w, src.h, src.pitch, src.format);
        let (dst_format, dst_pitch) = (dst.format, dst.pitch);
        if let (Some(s), Some(d)) = (src.pixels.bytes(), dst.pixels.bytes_mut()) {
            let _ = convert_pixels(w, h, format, s, pitch, dst_format, d, dst_pitch);
        }
    };

    match (needs_scaling, needs_conversion, conversion) {
        // downscaling? Do it first.  -1: downscale, 0: no scaling, 1: upscale
        (-1, true, Some(middle)) => {
            let _ = src.stretch(None, middle, None, ScaleMode::Nearest);
            convert(middle, output);
        }
        (-1, false, _) | (1, false, _) => {
            let _ = src.stretch(None, output, None, ScaleMode::Nearest);
        }
        (0, true, _) => convert(src, output),
        // upscaling? Do it last.
        (1, true, Some(middle)) => {
            convert(src, middle);
            let _ = middle.stretch(None, output, None, ScaleMode::Nearest);
        }
        _ => {}
    }
}

/// Translation of `SDL_CameraThreadShutdown()`.
pub(crate) fn camera_thread_shutdown(_device: &CameraDevice) {}

/// The device's `WaitDevice`: the backend's, or the zombie one.
fn wait_device(device: &Arc<CameraDevice>) -> bool {
    let (zombie_ops, backend) = {
        let guard = device.lock();
        let st = guard.borrow();
        (st.zombie_ops, st.hidden.clone())
    };
    if zombie_ops {
        return zombie_wait_device(device);
    }
    backend.is_none_or(|b| b.wait_device(device))
}

/// Actual thread entry point, if driver didn't handle this itself.
/// Translation of `CameraThread()`.
fn camera_thread(device: Arc<CameraDevice>) -> i32 {
    ref_physical_camera(&device); // this thread holds a reference.

    camera_thread_setup(&device);

    loop {
        if !wait_device(&device) {
            camera_disconnected(&device); // doh. (but don't break out of the loop, just be a zombie for now!)
        }
        if !camera_thread_iterate(&device) {
            break;
        }
    }

    camera_thread_shutdown(&device);

    unref_physical_camera(&device); // this thread no longer holds a reference.

    0
}

/// Set up the surfaces frames pass through. Backends that only learn
/// their format after opening (Emscripten, waiting for permission) call
/// this themselves; most backends should _not_ call this directly!
/// Translation of `SDL_PrepareCameraSurfaces()`.
pub(crate) fn prepare_camera_surfaces(device: &CameraDevice) -> Result<()> {
    let guard = device.lock();
    let mut st = guard.borrow_mut();
    let result = prepare_surfaces(&mut st);
    if result.is_err() {
        st.acquire_surface = None;
        st.conversion_surface = None;
        st.output_surfaces.clear();
    }
    result
}

fn prepare_surfaces(st: &mut CameraState) -> Result<()> {
    let devspec = st.actual_spec; // the hardware is set to this format.

    crate::sdl_assert!(st.acquire_surface.is_none()); // shouldn't call this function twice on an opened camera!
    crate::sdl_assert!(devspec.format != PixelFormat::UNKNOWN); // fix the backend, it should have an actual format by now.
    crate::sdl_assert!(devspec.width >= 0); // fix the backend, it should have an actual format by now.
    crate::sdl_assert!(devspec.height >= 0); // fix the backend, it should have an actual format by now.

    // the app wants this format.
    let appspec = &mut st.spec;
    if appspec.width <= 0 || appspec.height <= 0 {
        appspec.width = devspec.width;
        appspec.height = devspec.height;
    }

    if appspec.format == PixelFormat::UNKNOWN {
        appspec.format = devspec.format;
    }

    if appspec.framerate_denominator == 0 {
        appspec.framerate_numerator = devspec.framerate_numerator;
        appspec.framerate_denominator = devspec.framerate_denominator;
    }
    let appspec = *appspec;

    st.needs_scaling = if devspec.width == appspec.width && devspec.height == appspec.height {
        0
    } else {
        let srcarea = devspec.width as u64 * devspec.height as u64;
        let dstarea = appspec.width as u64 * appspec.height as u64;
        if dstarea <= srcarea {
            -1 // downscaling (or changing to new aspect ratio with same area)
        } else {
            1 // upscaling
        }
    };

    st.needs_conversion = devspec.format != appspec.format;

    let mut acquire = Surface::without_pixels(devspec.width, devspec.height, devspec.format)?;
    acquire.set_colorspace(devspec.colorspace);
    st.acquire_surface = Some(acquire);

    // if we have to scale _and_ convert, we need a middleman surface, since we can't do both changes at once.
    if st.needs_scaling != 0 && st.needs_conversion {
        let downscaling_first = st.needs_scaling < 0;
        let s = if downscaling_first {
            &appspec
        } else {
            &devspec
        };
        let fmt = if downscaling_first {
            devspec.format
        } else {
            appspec.format
        };
        let mut conversion = Surface::new_uninitialized(s.width, s.height, fmt)?;
        conversion.set_colorspace(devspec.colorspace);
        st.conversion_surface = Some(conversion);
    }

    // output surfaces are in the app-requested format. If no conversion is necessary, we'll just use the pixels
    // the backend hands over, and you can get all the way from the camera hardware
    // to the app without a single copy. Otherwise, these will be full surfaces that hold converted/scaled copies.

    st.empty_output_surfaces = (0..NUM_OUTPUT_SURFACES).rev().collect();

    st.output_surfaces.clear();
    for _ in 0..NUM_OUTPUT_SURFACES {
        let mut surf = if st.needs_scaling != 0 || st.needs_conversion {
            Surface::new_uninitialized(appspec.width, appspec.height, appspec.format)?
        } else {
            Surface::without_pixels(appspec.width, appspec.height, appspec.format)?
        };
        surf.set_colorspace(devspec.colorspace);

        st.output_surfaces.push(OutputSlot {
            surface: Some(surf),
            ..OutputSlot::default()
        });
    }

    Ok(())
}

/// Find the closest available native format/size to `spec` (or the best
/// one, without a request). Translation of `ChooseBestCameraSpec()`.
fn choose_best_camera_spec(all_specs: &[CameraSpec], spec: Option<&CameraSpec>) -> CameraSpec {
    // Find the closest available native format/size...
    //
    // We want the exact size if possible, even if we have
    // to convert formats, because we can _probably_ do that
    // conversion losslessly at less expense verses scaling.
    //
    // Failing that, we want the size that's closest to the
    // requested aspect ratio, then the closest size within
    // that.

    let mut closest = CameraSpec::default();

    if all_specs.is_empty() {
        // device listed no specs! You get whatever you want!
        if let Some(spec) = spec {
            closest = *spec;
        }
        return closest;
    }

    match spec {
        None => {
            // nothing specifically requested, get the best format we can...
            // we sorted this into the "best" format order when adding the camera.
            closest = all_specs[0];
        }
        Some(spec) => {
            // specific thing requested, try to get as close to that as possible...
            let wantw = spec.width;
            let wanth = spec.height;

            if wantw > 0 && wanth > 0 {
                // Find the sizes with the closest aspect ratio and then find the best fit of those.
                let wantaspect = wantw as f32 / wanth as f32;
                let epsilon = 1e-6f32;
                let mut closestaspect = -9999999.0f32;
                let mut closestdiff = 999999.0f32;
                let mut closestdiffw = 9999999;

                for thisspec in all_specs {
                    let thisw = thisspec.width;
                    let thish = thisspec.height;
                    let thisaspect = thisw as f32 / thish as f32;
                    let aspectdiff = (wantaspect - thisaspect).abs();
                    let diff = (closestaspect - thisaspect).abs();
                    let diffw = (thisw - wantw).abs();
                    if diff < epsilon {
                        // matches current closestaspect? See if resolution is closer in size.
                        if diffw < closestdiffw {
                            closestdiffw = diffw;
                            closest.width = thisw;
                            closest.height = thish;
                        }
                    } else if aspectdiff < closestdiff {
                        // this is a closer aspect ratio? Take it, reset resolution checks.
                        closestdiff = aspectdiff;
                        closestaspect = thisaspect;
                        closestdiffw = diffw;
                        closest.width = thisw;
                        closest.height = thish;
                    }
                }
            } else {
                closest = all_specs[0];
            }

            crate::sdl_assert!(closest.width > 0);
            crate::sdl_assert!(closest.height > 0);

            // okay, we have what we think is the best resolution, now we just need the best format that supports it...
            let wantfmt = spec.format;
            let mut best_format = PixelFormat::UNKNOWN;
            let mut best_colorspace = Colorspace::UNKNOWN;
            for thisspec in all_specs {
                if thisspec.width == closest.width && thisspec.height == closest.height {
                    if best_format == PixelFormat::UNKNOWN {
                        best_format = thisspec.format; // spec list is sorted by what we consider "best" format, so unless we find an exact match later, first size match is the one!
                        best_colorspace = thisspec.colorspace;
                    }
                    if thisspec.format == wantfmt {
                        best_format = thisspec.format;
                        best_colorspace = thisspec.colorspace;
                        break; // exact match, stop looking.
                    }
                }
            }

            crate::sdl_assert!(best_format != PixelFormat::UNKNOWN);
            crate::sdl_assert!(best_colorspace != Colorspace::UNKNOWN);
            closest.format = best_format;
            closest.colorspace = best_colorspace;

            // We have a resolution and a format, find the closest framerate...
            let fps = |num: i32, den: i32| {
                if den != 0 {
                    num as f32 / den as f32
                } else {
                    0.0
                }
            };
            let wantfps = fps(spec.framerate_numerator, spec.framerate_denominator);
            let mut closestfps = 9999999.0f32;
            for thisspec in all_specs {
                if thisspec.format == closest.format
                    && thisspec.width == closest.width
                    && thisspec.height == closest.height
                {
                    if thisspec.framerate_numerator == spec.framerate_numerator
                        && thisspec.framerate_denominator == spec.framerate_denominator
                    {
                        closest.framerate_numerator = thisspec.framerate_numerator;
                        closest.framerate_denominator = thisspec.framerate_denominator;
                        break; // exact match, stop looking.
                    }

                    let thisfps = fps(thisspec.framerate_numerator, thisspec.framerate_denominator);
                    let fpsdiff = (wantfps - thisfps).abs();
                    if fpsdiff < closestfps {
                        // this is a closest FPS? Take it until something closer arrives.
                        closestfps = fpsdiff;
                        closest.framerate_numerator = thisspec.framerate_numerator;
                        closest.framerate_denominator = thisspec.framerate_denominator;
                    }
                }
            }
        }
    }

    crate::sdl_assert!(closest.width > 0);
    crate::sdl_assert!(closest.height > 0);
    crate::sdl_assert!(closest.format != PixelFormat::UNKNOWN);

    closest
}

/// An opened camera. Translation of `SDL_Camera *` as returned by
/// `SDL_OpenCamera()`; dropping it closes the camera (`SDL_CloseCamera()`).
pub struct Camera {
    device: Arc<CameraDevice>,
}

impl std::fmt::Debug for Camera {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Camera")
            .field("instance_id", &self.device.instance_id)
            .field("name", &self.device.name)
            .finish()
    }
}

impl Camera {
    /// Open a camera, asking for `spec` (or the device's best format, with
    /// `None`). The closest format the device supports is used, and frames
    /// are converted and scaled to what was asked for; the camera might
    /// need the user's permission first. A camera can only be opened once.
    /// Translation of `SDL_OpenCamera()`.
    pub fn open(instance_id: CameraID, spec: Option<&CameraSpec>) -> Result<Camera> {
        let device = obtain_physical_camera(instance_id)?;
        let guard = device.lock();

        if guard.borrow().hidden.is_some() {
            release_camera(&device, guard);
            return Err(Error::new("Camera already opened")); // we may remove this limitation at some point.
        }

        device.shutdown.store(false, Ordering::Release);

        // These start with the backend's implementation, but we might swap them out with zombie versions later.
        guard.borrow_mut().zombie_ops = false;

        let closest = choose_best_camera_spec(&device.all_specs, spec);

        let Some(driver) = current_driver() else {
            release_camera(&device, guard);
            return Err(Error::new("Camera subsystem is not initialized"));
        };
        match driver.open_device(&device, &closest) {
            Ok(backend) => {
                guard.borrow_mut().hidden = Some(backend);
                // we're open, hold a reference. (upstream takes this only once
                // everything below has succeeded, but close_physical_camera()
                // drops it whenever the backend is open, so each failure path
                // below dropped a reference it never took and could destroy
                // the device; fixed here by taking it as soon as we're open.)
                ref_physical_camera(&device);
            }
            Err(e) => {
                close_physical_camera(&device); // in case anything is half-initialized.
                release_camera(&device, guard);
                return Err(e);
            }
        }

        {
            let mut st = guard.borrow_mut();
            st.spec = *spec.unwrap_or(&closest);
            st.actual_spec = closest;
        }

        // PixelFormat::UNKNOWN here is taken as a signal that the backend
        //  doesn't know its format yet (Emscripten waiting for user permission,
        //  in this case), and the backend will call prepare_camera_surfaces()
        //  itself, later but before the app is allowed to acquire images.
        if closest.format != PixelFormat::UNKNOWN {
            if let Err(e) = prepare_camera_surfaces(&device) {
                close_physical_camera(&device);
                release_camera(&device, guard);
                return Err(e);
            }
        }

        guard.borrow_mut().drop_frames = 1;

        // Start the camera thread if necessary
        if !driver.provides_own_callback_thread() {
            let threadname = camera_thread_name(&device);
            let thread_device = device.clone();
            match Thread::spawn(threadname, move || camera_thread(thread_device)) {
                Ok(thread) => guard.borrow_mut().thread = Some(thread),
                Err(_) => {
                    close_physical_camera(&device);
                    release_camera(&device, guard);
                    return Err(Error::new("Couldn't create camera thread"));
                }
            }
        }

        release_camera(&device, guard); // unlock, we're good to go!

        Ok(Camera { device })
    }

    /// The instance ID of the camera. Translation of `SDL_GetCameraID()`.
    pub fn id(&self) -> CameraID {
        self.device.instance_id
    }

    /// The properties of the camera. Translation of
    /// `SDL_GetCameraProperties()`.
    pub fn properties(&self) -> Properties {
        let guard = obtain_physical_camera_obj(&self.device);
        let props = guard
            .borrow_mut()
            .props
            .get_or_insert_with(Properties::new)
            .clone();
        release_camera(&self.device, guard);
        props
    }

    /// Whether the user has approved access to the camera. Translation of
    /// `SDL_GetCameraPermissionState()`.
    pub fn permission_state(&self) -> CameraPermissionState {
        let guard = obtain_physical_camera_obj(&self.device);
        let result = guard.borrow().permission;
        release_camera(&self.device, guard);
        result
    }

    /// The spec the camera's frames come in (after conversion), once
    /// permission is granted. Translation of `SDL_GetCameraFormat()`.
    pub fn format(&self) -> Result<CameraSpec> {
        let guard = obtain_physical_camera_obj(&self.device);
        let result = {
            let st = guard.borrow();
            if st.permission > CameraPermissionState::Pending {
                Ok(st.spec)
            } else {
                Err(Error::new("Camera permission has not been granted"))
            }
        };
        release_camera(&self.device, guard);
        result
    }

    /// The oldest new frame, if one is available (this doesn't block).
    /// Dropping the frame gives its surface back for new frames, so don't
    /// hold on to frames longer than needed. Translation of
    /// `SDL_AcquireCameraFrame()`.
    pub fn acquire_frame(&self) -> Result<Option<CameraFrame>> {
        let device = &self.device;
        let guard = obtain_physical_camera_obj(device);

        if guard.borrow().permission <= CameraPermissionState::Pending {
            release_camera(device, guard);
            return Err(Error::new("Camera permission has not been granted"));
        }

        let result = {
            let mut st = guard.borrow_mut();
            // frames are in this list from newest to oldest, so find the end of the list...
            st.filled_output_surfaces.pop_back().map(|slot| {
                // report the oldest frame.
                let output = &mut st.output_surfaces[slot];
                let timestamp_ns = output.timestamp_ns;
                let surface = output.surface.take();
                let zombie_frame = output.zombie_frame;
                st.app_held_output_surfaces.insert(0, slot); // add to app_held list.
                CameraFrame {
                    surface,
                    timestamp_ns,
                    slot,
                    zombie_frame,
                    generation: st.generation,
                    device: device.clone(),
                }
            })
        };

        release_camera(device, guard);

        Ok(result)
    }

    /// Give a frame back to the camera (the same as dropping it).
    /// Translation of `SDL_ReleaseCameraFrame()`.
    pub fn release_frame(&self, frame: CameraFrame) {
        drop(frame);
    }
}

impl Drop for Camera {
    fn drop(&mut self) {
        // Translation of `SDL_CloseCamera()`.
        close_physical_camera(&self.device);
    }
}

/// A frame of video from a [`Camera`]: a surface in the camera's
/// [format](Camera::format), with the rotation that would make it right-side
/// up in its [`PROP_SURFACE_ROTATION_FLOAT`] property. Dropping it gives it
/// back to the camera (`SDL_ReleaseCameraFrame()`).
pub struct CameraFrame {
    surface: Option<Surface<'static>>,
    timestamp_ns: u64,
    slot: usize,
    zombie_frame: bool,
    generation: u64,
    device: Arc<CameraDevice>,
}

impl std::fmt::Debug for CameraFrame {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CameraFrame")
            .field("surface", &self.surface)
            .field("timestamp_ns", &self.timestamp_ns)
            .finish()
    }
}

impl CameraFrame {
    /// The frame's timestamp, in nanoseconds, roughly comparable to
    /// [`ticks_ns`](crate::timer::ticks_ns).
    pub fn timestamp_ns(&self) -> u64 {
        self.timestamp_ns
    }

    /// The frame's surface.
    pub fn surface(&self) -> &Surface<'static> {
        self.surface.as_ref().expect("frame surface")
    }
}

impl Deref for CameraFrame {
    type Target = Surface<'static>;
    fn deref(&self) -> &Surface<'static> {
        self.surface()
    }
}

impl DerefMut for CameraFrame {
    fn deref_mut(&mut self) -> &mut Surface<'static> {
        self.surface.as_mut().expect("frame surface")
    }
}

impl Drop for CameraFrame {
    fn drop(&mut self) {
        // Translation of `SDL_ReleaseCameraFrame()`.
        let Some(mut frame) = self.surface.take() else {
            return;
        };
        let device = &self.device;
        let guard = obtain_physical_camera_obj(device);

        let (backend, to_release) = {
            let mut st = guard.borrow_mut();
            let held = st
                .app_held_output_surfaces
                .iter()
                .position(|&s| s == self.slot);
            let Some(held) = held.filter(|_| st.generation == self.generation) else {
                // (the camera closed since; the frame is simply freed)
                drop(st);
                release_camera(device, guard);
                return;
            };

            // these pixels were the backend's, give them back.
            let to_release = if !st.needs_conversion && st.needs_scaling == 0 {
                take_pixels(&mut frame)
            } else {
                None
            };

            let slot = &mut st.output_surfaces[self.slot];
            slot.timestamp_ns = 0;
            slot.surface = Some(frame);

            // remove from app_held list...
            st.app_held_output_surfaces.remove(held);

            // insert at front of empty list (and we'll use it first when we need to fill a new frame).
            st.empty_output_surfaces.push(self.slot);
            (st.hidden.clone(), to_release)
        };

        if let Some(pixels) = to_release {
            release_pixels(device, &backend, pixels, self.zombie_frame);
        }

        release_camera(device, guard);
    }
}

/// Translation of `SDL_QuitCamera()`.
pub(crate) fn quit_camera() {
    if current_camera_driver().is_none() {
        return; // not initialized?!
    }
    let Some(driver) = current_driver() else {
        return;
    };

    let device_hash = {
        let mut cam = driver_write();
        let Some(cam) = cam.as_mut() else { return };
        cam.shutting_down = true;
        let hash = cam.device_hash.take();
        cam.pending_events.clear();
        cam.device_count = 0;
        hash
    };

    for device in device_hash.into_iter().flat_map(|h| h.into_values()) {
        destroy_camera_hash_item(&device);
    }

    // Free the driver data
    driver.deinitialize();

    *driver_write() = None;
}

/// Physical camera objects are only destroyed when removed from the
/// device hash. Translation of `DestroyCameraHashItem()`.
fn destroy_camera_hash_item(device: &Arc<CameraDevice>) {
    close_physical_camera(device);
    if let Some(driver) = current_driver() {
        driver.free_device_handle(device);
    }
}

/// Start the camera subsystem with the named driver (a comma-separated
/// list to try in order), or the [`hints::CAMERA_DRIVER`] hint, or the
/// first available one. Translation of `SDL_InitCamera()`.
pub(crate) fn init_camera(driver_name: Option<&str>) -> Result<()> {
    if current_camera_driver().is_some() {
        quit_camera(); // shutdown driver if already running.
    }

    // Select the proper camera driver
    let driver_name = driver_name
        .map(str::to_owned)
        .or_else(|| hints::get(hints::CAMERA_DRIVER));

    let mut initialized = None;
    let mut tried_to_init = false;
    let mut init_error = None;

    let mut try_init = |b: &'static CameraBootStrap| {
        tried_to_init = true;
        *driver_write() = Some(CurrentCamera {
            device_hash: Some(BTreeMap::new()),
            ..CurrentCamera::default()
        });
        match (b.init)() {
            Ok(driver) => Some((b, driver)),
            Err(e) => {
                init_error = Some(e);
                None
            }
        }
    };

    match driver_name.as_deref() {
        Some(name) if !name.is_empty() => {
            for driver_attempt in name.split(',') {
                if driver_attempt.is_empty() || initialized.is_some() {
                    break;
                }
                if let Some(b) = BOOTSTRAP
                    .iter()
                    .find(|b| b.name.eq_ignore_ascii_case(driver_attempt))
                {
                    initialized = try_init(b);
                }
            }
        }
        _ => {
            for b in BOOTSTRAP {
                if initialized.is_some() {
                    break;
                }
                if b.demand_only {
                    continue;
                }
                initialized = try_init(b);
            }
        }
    }

    let Some((bootstrap, driver)) = initialized else {
        *driver_write() = None;
        // specific drivers will set the error message if they fail, but otherwise we do it here.
        return Err(match init_error {
            Some(e) if tried_to_init => e,
            _ => match driver_name {
                Some(name) => Error::new(format!("Camera driver '{name}' not available")),
                None => Error::new("No available camera driver"),
            },
        });
    };

    {
        let mut cam = driver_write();
        if let Some(cam) = cam.as_mut() {
            cam.name = Some(bootstrap.name);
            cam.desc = Some(bootstrap.desc);
            cam.driver = Some(driver.clone());
        }
    }
    crate::debug!(
        crate::log::Category::System,
        "SDL chose camera backend '{}'",
        bootstrap.name
    );

    // Make sure we have a list of devices available at startup...
    driver.detect_devices();

    Ok(())
}

/// Push the pending camera device events. Called from the event pump.
/// Translation of `SDL_UpdateCamera()`.
pub(crate) fn update_camera() {
    let pending_events = {
        match driver_read().as_ref() {
            Some(c) if !c.pending_events.is_empty() => {}
            _ => return, // nothing to do, check next time.
        }
        // okay, let's take this whole list of events so we can dump the lock, and new ones can queue up for a later update.
        match driver_write().as_mut() {
            Some(c) => std::mem::take(&mut c.pending_events), // in case this changed...
            None => return,
        }
    };

    for (event_type, devid) in pending_events {
        if crate::events::event_enabled(event_type) {
            let _ = crate::events::push(Event::CameraDevice(CameraDeviceEvent {
                event_type,
                timestamp: Duration::ZERO,
                which: devid,
            }));
        }
    }
}

#[cfg(test)]
mod tests;
