// Rust translation of src/camera/v4l2/SDL_camera_v4l2.c from Simple
// DirectMedia Layer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! The Video4Linux2 camera driver: `/dev/video*` capture devices, talked to
//! through raw ioctls (the parts of `<linux/videodev2.h>` SDL uses are
//! declared here), with memory-mapped, user-pointer or `read()` I/O, and
//! hotplug through udev (or a one-time scan of `/dev` without it).
//!
//! Upstream lends the app the driver's buffers; here a frame owns its
//! pixels (see the camera module). A memory-mapped buffer is copied into the
//! frame, a user-pointer buffer *is* the frame's `Vec` (the kernel fills it
//! while it's queued, and it goes back to the kernel when the frame is
//! released), and `read()` reads into the frame. As upstream, a dequeued
//! buffer is only queued again when its frame is released.

use std::ffi::{c_int, c_ulong, c_void, CString};
use std::mem::size_of;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd, RawFd};
use std::ptr::NonNull;
use std::sync::{Arc, Mutex, MutexGuard};

use super::{
    add_camera, camera_disconnected, camera_permission_outcome, find_physical_camera_by_callback,
    AcquiredFrame, CameraBackend, CameraBootStrap, CameraDevice, CameraDriverImpl,
    CameraFrameResult, CameraPosition, CameraSpec,
};
use crate::core::linux::evdev_capabilities::DeviceClass;
use crate::core::linux::input::{ior, iow, ioc_read_write};
use crate::core::linux::udev::{self, UdevDeviceEvent};
use crate::error::{Error, Result};
use crate::video::surface::calculate_surface_size;
use crate::video::{Colorspace, PixelFormat};

// --- the parts of <linux/videodev2.h> SDL uses ---

/// `v4l2_fourcc(a, b, c, d)`.
const fn v4l2_fourcc(a: u8, b: u8, c: u8, d: u8) -> u32 {
    (a as u32) | ((b as u32) << 8) | ((c as u32) << 16) | ((d as u32) << 24)
}

const V4L2_PIX_FMT_YUYV: u32 = v4l2_fourcc(b'Y', b'U', b'Y', b'V');
const V4L2_PIX_FMT_MJPEG: u32 = v4l2_fourcc(b'M', b'J', b'P', b'G');
const V4L2_PIX_FMT_RGBX32: u32 = v4l2_fourcc(b'X', b'B', b'2', b'4');

const V4L2_CAP_VIDEO_CAPTURE: u32 = 0x0000_0001;
const V4L2_CAP_READWRITE: u32 = 0x0100_0000;
const V4L2_CAP_STREAMING: u32 = 0x0400_0000;

const V4L2_BUF_TYPE_VIDEO_CAPTURE: u32 = 1;
const V4L2_FIELD_ANY: u32 = 0;

// enum v4l2_memory
const V4L2_MEMORY_MMAP: u32 = 1;
const V4L2_MEMORY_USERPTR: u32 = 2;

// enum v4l2_frmsizetypes
const V4L2_FRMSIZE_TYPE_DISCRETE: u32 = 1;
const V4L2_FRMSIZE_TYPE_CONTINUOUS: u32 = 2;
const V4L2_FRMSIZE_TYPE_STEPWISE: u32 = 3;

// enum v4l2_frmivaltypes
const V4L2_FRMIVAL_TYPE_DISCRETE: u32 = 1;
const V4L2_FRMIVAL_TYPE_CONTINUOUS: u32 = 2;
const V4L2_FRMIVAL_TYPE_STEPWISE: u32 = 3;

/// `struct v4l2_capability`.
#[repr(C)]
#[derive(Clone, Copy, Default)]
struct V4l2Capability {
    driver: [u8; 16],
    card: [u8; 32],
    bus_info: [u8; 32],
    version: u32,
    capabilities: u32,
    device_caps: u32,
    reserved: [u32; 3],
}

/// `struct v4l2_fmtdesc`.
#[repr(C)]
#[derive(Clone, Copy, Default)]
struct V4l2Fmtdesc {
    index: u32,
    type_: u32,
    flags: u32,
    description: [u8; 32],
    pixelformat: u32,
    mbus_code: u32,
    reserved: [u32; 3],
}

/// `struct v4l2_fract`.
#[repr(C)]
#[derive(Clone, Copy, Default, Debug, PartialEq)]
struct V4l2Fract {
    numerator: u32,
    denominator: u32,
}

/// `struct v4l2_frmsizeenum`. The union of `struct v4l2_frmsize_discrete`
/// (`width`, `height`) and `struct v4l2_frmsize_stepwise` (`min_width`,
/// `max_width`, `step_width`, `min_height`, `max_height`, `step_height`)
/// is kept as its six words, read through [`V4l2Frmsizeenum::discrete`] and
/// [`V4l2Frmsizeenum::stepwise`].
#[repr(C)]
#[derive(Clone, Copy, Default)]
struct V4l2Frmsizeenum {
    index: u32,
    pixel_format: u32,
    type_: u32,
    u: [u32; 6],
    reserved: [u32; 2],
}

/// `struct v4l2_frmsize_stepwise`.
#[derive(Clone, Copy, Default, Debug)]
struct FrmsizeStepwise {
    min_width: u32,
    max_width: u32,
    step_width: u32,
    min_height: u32,
    max_height: u32,
    step_height: u32,
}

impl V4l2Frmsizeenum {
    /// `discrete.width`, `discrete.height`.
    fn discrete(&self) -> (u32, u32) {
        (self.u[0], self.u[1])
    }

    /// `stepwise`.
    fn stepwise(&self) -> FrmsizeStepwise {
        let [min_width, max_width, step_width, min_height, max_height, step_height] = self.u;
        FrmsizeStepwise {
            min_width,
            max_width,
            step_width,
            min_height,
            max_height,
            step_height,
        }
    }
}

/// `struct v4l2_frmivalenum`. The union of `discrete` (a fraction) and
/// `struct v4l2_frmival_stepwise` (`min`, `max`, `step` fractions) is kept
/// as its six words.
#[repr(C)]
#[derive(Clone, Copy, Default)]
struct V4l2Frmivalenum {
    index: u32,
    pixel_format: u32,
    width: u32,
    height: u32,
    type_: u32,
    u: [u32; 6],
    reserved: [u32; 2],
}

/// `struct v4l2_frmival_stepwise`.
#[derive(Clone, Copy, Default, Debug)]
struct FrmivalStepwise {
    min: V4l2Fract,
    max: V4l2Fract,
    step: V4l2Fract,
}

impl V4l2Frmivalenum {
    /// `discrete`.
    fn discrete(&self) -> V4l2Fract {
        V4l2Fract {
            numerator: self.u[0],
            denominator: self.u[1],
        }
    }

    /// `stepwise`.
    fn stepwise(&self) -> FrmivalStepwise {
        let fract = |i: usize| V4l2Fract {
            numerator: self.u[i],
            denominator: self.u[i + 1],
        };
        FrmivalStepwise {
            min: fract(0),
            max: fract(2),
            step: fract(4),
        }
    }
}

/// `struct v4l2_rect`.
#[repr(C)]
#[derive(Clone, Copy, Default)]
struct V4l2Rect {
    left: i32,
    top: i32,
    width: u32,
    height: u32,
}

/// `struct v4l2_cropcap`.
#[repr(C)]
#[derive(Clone, Copy, Default)]
struct V4l2Cropcap {
    type_: u32,
    bounds: V4l2Rect,
    defrect: V4l2Rect,
    pixelaspect: V4l2Fract,
}

/// `struct v4l2_crop`.
#[repr(C)]
#[derive(Clone, Copy, Default)]
struct V4l2Crop {
    type_: u32,
    c: V4l2Rect,
}

/// `struct v4l2_pix_format`.
#[repr(C)]
#[derive(Clone, Copy, Default)]
struct V4l2PixFormat {
    width: u32,
    height: u32,
    pixelformat: u32,
    field: u32,
    bytesperline: u32,
    sizeimage: u32,
    colorspace: u32,
    priv_: u32,
    flags: u32,
    ycbcr_enc: u32,
    quantization: u32,
    xfer_func: u32,
}

/// The `fmt` union of `struct v4l2_format` (200 bytes, pointer-aligned
/// because of `struct v4l2_window`); SDL only uses `pix`.
#[repr(C, align(8))]
#[derive(Clone, Copy)]
struct V4l2FormatUnion {
    pix: V4l2PixFormat,
    raw_data: [u8; 200 - size_of::<V4l2PixFormat>()],
}

/// `struct v4l2_format`.
#[repr(C)]
#[derive(Clone, Copy)]
struct V4l2Format {
    type_: u32,
    fmt: V4l2FormatUnion,
}

impl Default for V4l2Format {
    fn default() -> V4l2Format {
        V4l2Format {
            type_: 0,
            fmt: V4l2FormatUnion {
                pix: V4l2PixFormat::default(),
                raw_data: [0; 200 - size_of::<V4l2PixFormat>()],
            },
        }
    }
}

/// `struct v4l2_captureparm`.
#[repr(C)]
#[derive(Clone, Copy, Default)]
struct V4l2Captureparm {
    capability: u32,
    capturemode: u32,
    timeperframe: V4l2Fract,
    extendedmode: u32,
    readbuffers: u32,
    reserved: [u32; 4],
}

/// `struct v4l2_streamparm`; of the `parm` union (200 bytes), SDL only uses
/// `capture`.
#[repr(C)]
#[derive(Clone, Copy)]
struct V4l2Streamparm {
    type_: u32,
    capture: V4l2Captureparm,
    raw_data: [u8; 200 - size_of::<V4l2Captureparm>()],
}

impl Default for V4l2Streamparm {
    fn default() -> V4l2Streamparm {
        V4l2Streamparm {
            type_: 0,
            capture: V4l2Captureparm::default(),
            raw_data: [0; 200 - size_of::<V4l2Captureparm>()],
        }
    }
}

/// `struct v4l2_requestbuffers`.
#[repr(C)]
#[derive(Clone, Copy, Default)]
struct V4l2Requestbuffers {
    count: u32,
    type_: u32,
    memory: u32,
    capabilities: u32,
    flags: u8,
    reserved: [u8; 3],
}

/// `struct v4l2_timecode`.
#[repr(C)]
#[derive(Clone, Copy, Default)]
struct V4l2Timecode {
    type_: u32,
    flags: u32,
    frames: u8,
    seconds: u8,
    minutes: u8,
    hours: u8,
    userbits: [u8; 4],
}

/// `struct v4l2_buffer`. The `m` union (`offset`, `userptr`, `planes`,
/// `fd`) is kept as an `unsigned long`; see [`V4l2Buffer::offset`].
#[repr(C)]
#[derive(Clone, Copy)]
struct V4l2Buffer {
    index: u32,
    type_: u32,
    bytesused: u32,
    flags: u32,
    field: u32,
    timestamp: libc::timeval,
    timecode: V4l2Timecode,
    sequence: u32,
    memory: u32,
    m: c_ulong,
    length: u32,
    reserved2: u32,
    request_fd: u32,
}

impl V4l2Buffer {
    /// `SDL_zero(buf)` with the type and memory set.
    fn new(memory: u32) -> V4l2Buffer {
        V4l2Buffer {
            index: 0,
            type_: V4L2_BUF_TYPE_VIDEO_CAPTURE,
            bytesused: 0,
            flags: 0,
            field: 0,
            timestamp: libc::timeval {
                tv_sec: 0,
                tv_usec: 0,
            },
            timecode: V4l2Timecode::default(),
            sequence: 0,
            memory,
            m: 0,
            length: 0,
            reserved2: 0,
            request_fd: 0,
        }
    }

    /// `m.offset`: the union's first 32 bits.
    fn offset(&self) -> u32 {
        let bytes = self.m.to_ne_bytes();
        u32::from_ne_bytes([bytes[0], bytes[1], bytes[2], bytes[3]])
    }

    /// The frame's timestamp in nanoseconds.
    fn timestamp_ns(&self) -> u64 {
        (self.timestamp.tv_sec as u64)
            .wrapping_mul(1_000_000_000)
            .wrapping_add((self.timestamp.tv_usec as u64).wrapping_mul(1000))
    }
}

const VIDIOC_QUERYCAP: c_ulong = ior(b'V', 0, size_of::<V4l2Capability>());
const VIDIOC_ENUM_FMT: c_ulong = ioc_read_write(b'V', 2, size_of::<V4l2Fmtdesc>());
const VIDIOC_G_FMT: c_ulong = ioc_read_write(b'V', 4, size_of::<V4l2Format>());
const VIDIOC_S_FMT: c_ulong = ioc_read_write(b'V', 5, size_of::<V4l2Format>());
const VIDIOC_REQBUFS: c_ulong = ioc_read_write(b'V', 8, size_of::<V4l2Requestbuffers>());
const VIDIOC_QUERYBUF: c_ulong = ioc_read_write(b'V', 9, size_of::<V4l2Buffer>());
const VIDIOC_QBUF: c_ulong = ioc_read_write(b'V', 15, size_of::<V4l2Buffer>());
const VIDIOC_DQBUF: c_ulong = ioc_read_write(b'V', 17, size_of::<V4l2Buffer>());
const VIDIOC_STREAMON: c_ulong = iow(b'V', 18, size_of::<c_int>());
const VIDIOC_STREAMOFF: c_ulong = iow(b'V', 19, size_of::<c_int>());
const VIDIOC_G_PARM: c_ulong = ioc_read_write(b'V', 21, size_of::<V4l2Streamparm>());
const VIDIOC_S_PARM: c_ulong = ioc_read_write(b'V', 22, size_of::<V4l2Streamparm>());
const VIDIOC_CROPCAP: c_ulong = ioc_read_write(b'V', 58, size_of::<V4l2Cropcap>());
const VIDIOC_S_CROP: c_ulong = iow(b'V', 60, size_of::<V4l2Crop>());
const VIDIOC_ENUM_FRAMESIZES: c_ulong = ioc_read_write(b'V', 74, size_of::<V4l2Frmsizeenum>());
const VIDIOC_ENUM_FRAMEINTERVALS: c_ulong = ioc_read_write(b'V', 75, size_of::<V4l2Frmivalenum>());

/// The V4L2 argument structs: plain data that the kernel reads and
/// writes whole, any bit pattern of which is a valid value.
///
/// # Safety
///
/// Implementors must be `repr(C)` structs of integers (and arrays of them).
unsafe trait IoctlArg: Copy {}
// SAFETY: all of these are repr(C) structs of integers.
unsafe impl IoctlArg for V4l2Capability {}
// SAFETY: as above.
unsafe impl IoctlArg for V4l2Fmtdesc {}
// SAFETY: as above.
unsafe impl IoctlArg for V4l2Frmsizeenum {}
// SAFETY: as above.
unsafe impl IoctlArg for V4l2Frmivalenum {}
// SAFETY: as above.
unsafe impl IoctlArg for V4l2Cropcap {}
// SAFETY: as above.
unsafe impl IoctlArg for V4l2Crop {}
// SAFETY: as above.
unsafe impl IoctlArg for V4l2Format {}
// SAFETY: as above.
unsafe impl IoctlArg for V4l2Streamparm {}
// SAFETY: as above.
unsafe impl IoctlArg for V4l2Requestbuffers {}
// SAFETY: as above (`timeval` is two integers).
unsafe impl IoctlArg for V4l2Buffer {}
// SAFETY: an integer.
unsafe impl IoctlArg for c_int {}

/// `errno` after a failed call.
fn errno() -> c_int {
    crate::core::linux::input::errno()
}

/// `ioctl(fd, request, arg)`: true on success.
fn ioctl<T: IoctlArg>(fd: RawFd, request: c_ulong, arg: &mut T) -> bool {
    // SAFETY: `arg` is a valid, writable `T` of the size `request` encodes
    // (each request is declared with its argument's type), and any bytes
    // the kernel writes form a valid `T` (IoctlArg).
    unsafe { crate::core::linux::input::ioctl_ptr(fd, request, (arg as *mut T).cast()) == 0 }
}

/// `xioctl()`: `ioctl()`, retried while it's interrupted by a signal.
fn xioctl<T: IoctlArg>(fd: RawFd, request: c_ulong, arg: &mut T) -> bool {
    loop {
        if ioctl(fd, request, arg) {
            return true;
        }
        if errno() != libc::EINTR {
            return false;
        }
    }
}

/// `open(path, O_RDWR | O_NONBLOCK, 0)`.
fn open_device_node(path: &str) -> std::io::Result<OwnedFd> {
    let cpath = CString::new(path).map_err(|_| std::io::Error::from_raw_os_error(libc::ENOENT))?;
    // SAFETY: the path is NUL-terminated; open() has no other preconditions.
    let fd = unsafe { libc::open(cpath.as_ptr(), libc::O_RDWR | libc::O_NONBLOCK, 0) };
    if fd == -1 {
        return Err(std::io::Error::last_os_error());
    }
    // SAFETY: open() just returned this descriptor, which nothing else owns.
    Ok(unsafe { OwnedFd::from_raw_fd(fd) })
}

/// Whether `fd` is a character device (`fstat()` + `S_ISCHR()`); `Err` if
/// it can't be stat'ed.
fn is_char_device(fd: RawFd) -> std::io::Result<bool> {
    // SAFETY: an all-zero `stat` is a valid value to be overwritten.
    let mut st: libc::stat = unsafe { std::mem::zeroed() };
    // SAFETY: `st` is a valid, writable `stat`.
    if unsafe { libc::fstat(fd, &mut st) } == -1 {
        return Err(std::io::Error::last_os_error());
    }
    Ok((st.st_mode & libc::S_IFMT) == libc::S_IFCHR)
}

/// A NUL-terminated byte array as a string (up to the first NUL).
fn c_array_str(bytes: &[u8]) -> String {
    let end = bytes.iter().position(|&b| b == 0).unwrap_or(bytes.len());
    String::from_utf8_lossy(&bytes[..end]).into_owned()
}

// --- the driver ---

/// Where to find a camera. Translation of `V4L2DeviceHandle`.
struct V4l2DeviceHandle {
    bus_info: String,
    path: String,
}

/// A memory-mapped V4L2 buffer, unmapped on drop.
struct Mapping {
    start: NonNull<u8>,
    length: usize,
}

// SAFETY: the mapping is plain shared memory; it's only read (under the
// backend's mutex) while the kernel isn't filling it (the buffer is
// dequeued).
unsafe impl Send for Mapping {}

impl Mapping {
    /// `mmap(NULL, length, PROT_READ | PROT_WRITE, MAP_SHARED, fd, offset)`.
    fn new(fd: RawFd, length: usize, offset: u32) -> Option<Mapping> {
        // SAFETY: a new shared mapping of the device's buffer; no Rust
        // memory is affected.
        let start = unsafe {
            libc::mmap(
                std::ptr::null_mut(), // start anywhere
                length,
                libc::PROT_READ | libc::PROT_WRITE, // required
                libc::MAP_SHARED,                   // recommended
                fd,
                offset as libc::off_t,
            )
        };
        if start == libc::MAP_FAILED {
            return None;
        }
        NonNull::new(start.cast()).map(|start| Mapping { start, length })
    }

    /// The buffer's first `len` bytes.
    fn bytes(&self, len: usize) -> &[u8] {
        // SAFETY: the mapping is `length` readable bytes while it lives.
        unsafe { std::slice::from_raw_parts(self.start.as_ptr(), len.min(self.length)) }
    }
}

impl Drop for Mapping {
    fn drop(&mut self) {
        // SAFETY: the mapping was made by mmap() with this length and is
        // unmapped once, here. (Upstream sets the error "munmap" if this
        // fails; a drop has nowhere to report it.)
        unsafe {
            libc::munmap(self.start.as_ptr().cast::<c_void>(), self.length);
        }
    }
}

/// A memory-mapped buffer (`struct buffer` for `IO_METHOD_MMAP`).
struct MmapBuffer {
    map: Mapping,
    /// Is available in userspace (dequeued).
    available: bool,
    /// The address of the frame copied out of this buffer, while the app
    /// holds it.
    lent: Option<usize>,
}

/// A user-pointer buffer (`struct buffer` for `IO_METHOD_USERPTR`): the
/// memory the kernel fills, which becomes the frame's pixels while the app
/// holds it.
struct UserBuffer {
    /// `None` while lent to the app.
    data: Option<Vec<u8>>,
    /// The buffer's address (`start`), which the frame keeps.
    start: usize,
    length: usize,
    /// Is available in userspace (dequeued).
    available: bool,
}

/// How frames are read from the device, with its buffers. Translation of
/// `io_method` (`IO_METHOD_INVALID` is the absence of one).
enum Io {
    /// `IO_METHOD_READ`: `read()` into the one buffer (`None` while lent).
    Read {
        buffer: Option<Vec<u8>>,
        length: usize,
    },
    /// `IO_METHOD_MMAP`.
    Mmap {
        buffers: Vec<MmapBuffer>,
        /// Frames given back, to copy the next ones into.
        spare: Vec<Vec<u8>>,
    },
    /// `IO_METHOD_USERPTR`.
    UserPtr { buffers: Vec<UserBuffer> },
}

/// An opened device. Translation of `struct SDL_PrivateCameraData`. It's
/// closed when dropped (`V4L2_CloseDevice()`).
struct Hidden {
    io: Io,
    driver_pitch: i32,
    fd: OwnedFd,
}

impl Drop for Hidden {
    fn drop(&mut self) {
        // Translation of `V4L2_CloseDevice()`: stop streaming, then the
        // buffers are freed or unmapped and the device closed (the fields'
        // drops). (The kernel is done with user-pointer buffers once
        // streaming is off.)
        if matches!(self.io, Io::Mmap { .. } | Io::UserPtr { .. }) {
            let mut type_: c_int = V4L2_BUF_TYPE_VIDEO_CAPTURE as c_int;
            xioctl(self.fd.as_raw_fd(), VIDIOC_STREAMOFF, &mut type_);
        }
    }
}

impl Hidden {
    /// Translation of `EnqueueBuffers()`.
    fn enqueue_buffers(&mut self) -> Result<()> {
        let fd = self.fd.as_raw_fd();
        match &mut self.io {
            Io::Read { .. } => {}

            Io::Mmap { buffers, .. } => {
                for (i, b) in buffers.iter().enumerate() {
                    if !b.available {
                        let mut buf = V4l2Buffer::new(V4L2_MEMORY_MMAP);
                        buf.index = i as u32;

                        if !xioctl(fd, VIDIOC_QBUF, &mut buf) {
                            return Err(Error::new("VIDIOC_QBUF"));
                        }
                    }
                }
            }

            Io::UserPtr { buffers } => {
                for (i, b) in buffers.iter().enumerate() {
                    if !b.available {
                        let mut buf = V4l2Buffer::new(V4L2_MEMORY_USERPTR);
                        buf.index = i as u32;
                        buf.m = b.start as c_ulong;
                        buf.length = b.length as u32;

                        if !xioctl(fd, VIDIOC_QBUF, &mut buf) {
                            return Err(Error::new("VIDIOC_QBUF"));
                        }
                    }
                }
            }
        }
        Ok(())
    }
}

/// A zero-filled buffer of `size` bytes at an address of its own
/// (`SDL_calloc(1, size)`, which gives each zero-byte buffer a distinct
/// pointer, too).
fn alloc_buffer(size: usize) -> Vec<u8> {
    let mut v = Vec::with_capacity(size.max(1));
    v.resize(size, 0);
    v
}

/// Translation of `AllocBufferMmap()`.
fn alloc_buffer_mmap(fd: RawFd, nb_buffers: u32) -> Result<Vec<MmapBuffer>> {
    let mut buffers = Vec::new();
    for i in 0..nb_buffers {
        let mut buf = V4l2Buffer::new(V4L2_MEMORY_MMAP);
        buf.index = i;

        if !xioctl(fd, VIDIOC_QUERYBUF, &mut buf) {
            return Err(Error::new("VIDIOC_QUERYBUF"));
        }

        let map = Mapping::new(fd, buf.length as usize, buf.offset())
            .ok_or_else(|| Error::new("mmap"))?;
        buffers.push(MmapBuffer {
            map,
            available: false,
            lent: None,
        });
    }
    Ok(buffers)
}

/// Translation of `AllocBufferUserPtr()`.
fn alloc_buffer_user_ptr(nb_buffers: u32, buffer_size: usize) -> Vec<UserBuffer> {
    (0..nb_buffers)
        .map(|_| {
            let data = alloc_buffer(buffer_size);
            UserBuffer {
                start: data.as_ptr() as usize,
                data: Some(data),
                length: buffer_size,
                available: false,
            }
        })
        .collect()
}

/// Translation of `format_v4l2_to_sdl()`.
fn format_v4l2_to_sdl(fmt: u32) -> (PixelFormat, Colorspace) {
    match fmt {
        V4L2_PIX_FMT_YUYV => (PixelFormat::YUY2, Colorspace::BT709_LIMITED),
        V4L2_PIX_FMT_MJPEG => (PixelFormat::MJPG, Colorspace::SRGB),
        V4L2_PIX_FMT_RGBX32 => (PixelFormat::RGBX32, Colorspace::SRGB),
        _ => (PixelFormat::UNKNOWN, Colorspace::UNKNOWN),
    }
}

/// Translation of `format_sdl_to_v4l2()`.
fn format_sdl_to_v4l2(fmt: PixelFormat) -> u32 {
    match fmt {
        PixelFormat::YUY2 => V4L2_PIX_FMT_YUYV,
        PixelFormat::MJPG => V4L2_PIX_FMT_MJPEG,
        PixelFormat::RGBX32 => V4L2_PIX_FMT_RGBX32,
        _ => 0,
    }
}

/// The opened-device interface (`device->hidden` and the per-device
/// `V4L2_*` functions).
struct V4l2Camera {
    hidden: Mutex<Option<Hidden>>,
}

impl V4l2Camera {
    fn hidden(&self) -> MutexGuard<'_, Option<Hidden>> {
        self.hidden.lock().unwrap_or_else(|e| e.into_inner())
    }
}

/// What a failed `read()`/`VIDIOC_DQBUF` means. Part of `V4L2_AcquireFrame()`.
/// (Upstream also sets an error message, "read" or "VIDIOC_DQBUF"; the
/// front end has no use for it: the device is treated as lost.)
fn frame_error() -> CameraFrameResult {
    match errno() {
        libc::EAGAIN => CameraFrameResult::Skip,
        // EIO: Could ignore EIO, see spec. (fall through)
        _ => CameraFrameResult::Error,
    }
}

impl CameraBackend for V4l2Camera {
    fn wait_device(&self, device: &CameraDevice) -> bool {
        // Translation of `V4L2_WaitDevice()`.
        // (the descriptor stays open until close_device(), which only runs
        // once this thread is done)
        let Some(fd) = self.hidden().as_ref().map(|h| h.fd.as_raw_fd()) else {
            return false;
        };

        loop {
            // (upstream select()s on the descriptor; poll() waits for the
            // same thing without select()'s FD_SETSIZE limit: FD_SET() of a
            // descriptor past it is undefined behavior.)
            let mut pfd = libc::pollfd {
                fd,
                events: libc::POLLIN,
                revents: 0,
            };
            // SAFETY: one valid pollfd.
            let mut rc = unsafe { libc::poll(&mut pfd, 1, 100) };
            if rc == -1 && errno() == libc::EINTR {
                rc = 0; // pretend it was a timeout, keep looping.
            } else if rc > 0 {
                return true;
            }

            // Thread is requested to shut down
            if device.shutting_down() {
                return true;
            }

            if rc != 0 {
                return false;
            }
        }
    }

    fn acquire_frame(&self, _device: &CameraDevice) -> CameraFrameResult {
        // Translation of `V4L2_AcquireFrame()`.
        let mut guard = self.hidden();
        let Some(hidden) = guard.as_mut() else {
            return CameraFrameResult::Error;
        };
        let fd = hidden.fd.as_raw_fd();
        let driver_pitch = hidden.driver_pitch;

        match &mut hidden.io {
            Io::Read { buffer, length } => {
                // Note (upstream): every frame is read into the same
                // buffer, so a frame the app still holds is overwritten by
                // the next read; here each frame owns its buffer.
                let mut pixels = buffer.take().unwrap_or_else(|| alloc_buffer(*length));
                // SAFETY: `pixels` is `length` writable bytes.
                let amount = unsafe { libc::read(fd, pixels.as_mut_ptr().cast(), pixels.len()) };
                if amount == -1 {
                    *buffer = Some(pixels);
                    return frame_error();
                }

                let pitch = if driver_pitch != 0 {
                    driver_pitch
                } else {
                    amount as i32
                };
                CameraFrameResult::Ready(AcquiredFrame {
                    pixels,
                    pitch,
                    timestamp_ns: crate::timer::ticks_ns(), // oh well, close enough.
                    rotation: 0.0,
                })
            }

            Io::Mmap { buffers, spare } => {
                let mut buf = V4l2Buffer::new(V4L2_MEMORY_MMAP);

                if !xioctl(fd, VIDIOC_DQBUF, &mut buf) {
                    return frame_error(); // SDL_SetError("VIDIOC_DQBUF: %d", errno)
                }

                let Some(b) = buffers.get_mut(buf.index as usize) else {
                    return CameraFrameResult::Error; // SDL_SetError("invalid buffer index")
                };

                let pitch = if driver_pitch != 0 {
                    driver_pitch
                } else {
                    buf.bytesused as i32
                };

                // (the frame gets a copy of what the driver put in the buffer)
                let used = if buf.bytesused != 0 {
                    buf.bytesused as usize
                } else {
                    b.map.length
                };
                let src = b.map.bytes(used);
                let mut pixels = spare.pop().unwrap_or_default();
                pixels.clear();
                pixels.extend_from_slice(src);
                b.available = true;
                b.lent = Some(pixels.as_ptr() as usize);

                CameraFrameResult::Ready(AcquiredFrame {
                    pixels,
                    pitch,
                    timestamp_ns: buf.timestamp_ns(),
                    rotation: 0.0,
                })
            }

            Io::UserPtr { buffers } => {
                let size = buffers.first().map_or(0, |b| b.length);
                let mut buf = V4l2Buffer::new(V4L2_MEMORY_USERPTR);

                if !xioctl(fd, VIDIOC_DQBUF, &mut buf) {
                    return frame_error();
                }

                let found = buffers
                    .iter()
                    .position(|b| buf.m == b.start as c_ulong && buf.length as usize == size);
                let Some(i) = found else {
                    return CameraFrameResult::Error; // SDL_SetError("invalid buffer index")
                };
                let Some(pixels) = buffers[i].data.take() else {
                    // (the kernel handed back a buffer that wasn't queued)
                    return CameraFrameResult::Error; // SDL_SetError("invalid buffer index")
                };

                let pitch = if driver_pitch != 0 {
                    driver_pitch
                } else {
                    buf.bytesused as i32
                };
                buffers[i].available = true;

                CameraFrameResult::Ready(AcquiredFrame {
                    pixels,
                    pitch,
                    timestamp_ns: buf.timestamp_ns(),
                    rotation: 0.0,
                })
            }
        }
    }

    fn release_frame(&self, _device: &CameraDevice, pixels: Vec<u8>) {
        // Translation of `V4L2_ReleaseFrame()`.
        let mut guard = self.hidden();
        let Some(hidden) = guard.as_mut() else {
            return; // (closed since; the frame is simply freed)
        };
        let fd = hidden.fd.as_raw_fd();
        let addr = pixels.as_ptr() as usize;

        match &mut hidden.io {
            Io::Read { buffer, .. } => {
                // (the buffer is read into again)
                if buffer.is_none() {
                    *buffer = Some(pixels);
                }
            }

            Io::Mmap { buffers, spare } => {
                let Some(i) = buffers.iter().position(|b| b.lent == Some(addr)) else {
                    return; // oh well, we didn't own this.
                };
                buffers[i].lent = None;
                spare.push(pixels);

                let mut buf = V4l2Buffer::new(V4L2_MEMORY_MMAP);
                buf.index = i as u32;

                if !xioctl(fd, VIDIOC_QBUF, &mut buf) {
                    // !!! FIXME: disconnect the device.
                    return; //SDL_SetError("VIDIOC_QBUF");
                }
                buffers[i].available = false;
            }

            Io::UserPtr { buffers } => {
                let Some(i) = buffers.iter().position(|b| b.start == addr) else {
                    return; // oh well, we didn't own this.
                };

                let mut buf = V4l2Buffer::new(V4L2_MEMORY_USERPTR);
                buf.index = i as u32;
                buf.m = addr as c_ulong;
                buf.length = buffers[i].length as u32;

                // (the buffer must be ours again before the kernel may fill it)
                buffers[i].data = Some(pixels);
                if !xioctl(fd, VIDIOC_QBUF, &mut buf) {
                    // !!! FIXME: disconnect the device.
                    return; //SDL_SetError("VIDIOC_QBUF");
                }
                buffers[i].available = false;
            }
        }
    }

    fn close_device(&self, _device: &CameraDevice) {
        // Translation of `V4L2_CloseDevice()` (see `Hidden`'s drop).
        *self.hidden() = None;
    }
}

/// The ioctls that list a device's formats, sizes and frame intervals
/// (true when the ioctl succeeded), so that the enumeration can be tested
/// without a device.
trait FormatEnumerator {
    /// `ioctl(fd, VIDIOC_ENUM_FMT, desc) == 0`
    fn enum_fmt(&self, desc: &mut V4l2Fmtdesc) -> bool;
    /// `ioctl(fd, VIDIOC_ENUM_FRAMESIZES, e) == 0`
    fn enum_framesizes(&self, e: &mut V4l2Frmsizeenum) -> bool;
    /// `ioctl(fd, VIDIOC_ENUM_FRAMEINTERVALS, e) == 0`
    fn enum_frameintervals(&self, e: &mut V4l2Frmivalenum) -> bool;
}

/// An opened device node.
struct DeviceFd(RawFd);

impl FormatEnumerator for DeviceFd {
    fn enum_fmt(&self, desc: &mut V4l2Fmtdesc) -> bool {
        ioctl(self.0, VIDIOC_ENUM_FMT, desc)
    }
    fn enum_framesizes(&self, e: &mut V4l2Frmsizeenum) -> bool {
        ioctl(self.0, VIDIOC_ENUM_FRAMESIZES, e)
    }
    fn enum_frameintervals(&self, e: &mut V4l2Frmivalenum) -> bool {
        ioctl(self.0, VIDIOC_ENUM_FRAMEINTERVALS, e)
    }
}

/// The values `start`, `start + step`, ... up to `end`, as upstream's
/// `for (v = start; v <= end; v += step)` loops count.
///
/// FIXME (upstream): a step of 0 (or one past `INT_MAX`, negative as an
/// `int`) never ends those loops, which then overflow (undefined behavior
/// in C); so does counting past `INT_MAX`. Here such a step gives just
/// `start`, and the counting stops before it would overflow.
fn steps(start: i32, end: i32, step: i32) -> impl Iterator<Item = i32> {
    let mut next = Some(start);
    std::iter::from_fn(move || {
        let v = next.filter(|&v| v <= end)?;
        next = if step <= 0 { None } else { v.checked_add(step) };
        Some(v)
    })
}

/// Translation of `AddCameraFormat()`: the frame rates of one format and
/// size.
fn add_camera_format(
    dev: &impl FormatEnumerator,
    specs: &mut Vec<CameraSpec>,
    sdlfmt: PixelFormat,
    colorspace: Colorspace,
    v4l2fmt: u32,
    w: i32,
    h: i32,
) {
    let mut add = |framerate_numerator, framerate_denominator| {
        super::add_camera_format(
            specs,
            sdlfmt,
            colorspace,
            w,
            h,
            framerate_numerator,
            framerate_denominator,
        );
    };

    let mut frmivalenum = V4l2Frmivalenum {
        pixel_format: v4l2fmt,
        width: w as u32,
        height: h as u32,
        ..V4l2Frmivalenum::default()
    };

    // FIXME (upstream): an interval type that's none of the three never
    // advances the index, so this loops forever (the kernel only reports
    // the three).
    while dev.enum_frameintervals(&mut frmivalenum) {
        if frmivalenum.type_ == V4L2_FRMIVAL_TYPE_DISCRETE {
            let discrete = frmivalenum.discrete();
            let numerator = discrete.numerator as i32;
            let denominator = discrete.denominator as i32;
            // Note (upstream): SDL_AddCameraFormat() fails only when out of
            // memory, ending the enumeration; adding a spec can't fail here.
            add(denominator, numerator);
            frmivalenum.index += 1; // set up for the next one.
        } else if frmivalenum.type_ == V4L2_FRMIVAL_TYPE_STEPWISE {
            let stepwise = frmivalenum.stepwise();
            let mut d = stepwise.min.denominator as i32;
            // !!! FIXME: should we step by the numerator...?
            for n in steps(
                stepwise.min.numerator as i32,
                stepwise.max.numerator as i32,
                stepwise.step.numerator as i32,
            ) {
                // SDL expects framerate, V4L2 provides interval
                add(d, n);
                d = d.wrapping_add(stepwise.step.denominator as i32);
            }
            break;
        } else if frmivalenum.type_ == V4L2_FRMIVAL_TYPE_CONTINUOUS {
            // FIXME: The current API does not enable exposing continuous ranges, so for now let's expose some common values that are within the range
            let stepwise = frmivalenum.stepwise();
            let min_numer = stepwise.min.numerator as i32;
            let min_denom = stepwise.min.denominator as i32;
            let max_numer = stepwise.max.numerator as i32;
            let max_denom = stepwise.max.denominator as i32;
            // (C compares the float rates with double constants)
            let minrate = (min_numer as f32 / min_denom as f32) as f64;
            let maxrate = (max_numer as f32 / max_denom as f32) as f64;
            for (interval, num, den) in [
                (1.001 / 24.0, 24000, 1001),
                (1.000 / 24.0, 24000, 1000),
                (1.001 / 30.0, 30000, 1001),
                (1.000 / 30.0, 30000, 1000),
                (1.000 / 50.0, 50000, 1000),
                (1.001 / 60.0, 60000, 1001),
                (1.000 / 60.0, 60000, 1000),
            ] {
                if minrate <= interval && maxrate >= interval {
                    add(num, den);
                }
            }
            break;
        }
    }
}

/// The specs of a device: every format SDL supports, in each size, at each
/// frame rate. Part of `MaybeAddDevice()`.
fn gather_specs(dev: &impl FormatEnumerator) -> Vec<CameraSpec> {
    let mut specs = Vec::new();

    let mut fmtdesc = V4l2Fmtdesc {
        type_: V4L2_BUF_TYPE_VIDEO_CAPTURE,
        ..V4l2Fmtdesc::default()
    };
    while dev.enum_fmt(&mut fmtdesc) {
        let (sdlfmt, colorspace) = format_v4l2_to_sdl(fmtdesc.pixelformat);

        fmtdesc.index += 1; // prepare for next iteration.

        if sdlfmt == PixelFormat::UNKNOWN {
            continue; // unsupported by SDL atm.
        }

        let mut frmsizeenum = V4l2Frmsizeenum {
            pixel_format: fmtdesc.pixelformat,
            ..V4l2Frmsizeenum::default()
        };

        // FIXME (upstream): a size type that's none of the three never
        // advances the index, so this loops forever (the kernel only
        // reports the three).
        while dev.enum_framesizes(&mut frmsizeenum) {
            if frmsizeenum.type_ == V4L2_FRMSIZE_TYPE_DISCRETE {
                let (w, h) = frmsizeenum.discrete();
                add_camera_format(
                    dev,
                    &mut specs,
                    sdlfmt,
                    colorspace,
                    fmtdesc.pixelformat,
                    w as i32,
                    h as i32,
                );
                frmsizeenum.index += 1; // set up for the next one.
            } else if frmsizeenum.type_ == V4L2_FRMSIZE_TYPE_STEPWISE
                || frmsizeenum.type_ == V4L2_FRMSIZE_TYPE_CONTINUOUS
            {
                let s = frmsizeenum.stepwise();
                for w in steps(s.min_width as i32, s.max_width as i32, s.step_width as i32) {
                    for h in steps(
                        s.min_height as i32,
                        s.max_height as i32,
                        s.step_height as i32,
                    ) {
                        add_camera_format(
                            dev,
                            &mut specs,
                            sdlfmt,
                            colorspace,
                            fmtdesc.pixelformat,
                            w,
                            h,
                        );
                    }
                }
                break;
            }
        }
    }

    specs
}

/// Translation of `FindV4L2CameraByBusInfoCallback()`.
fn find_v4l2_camera_by_bus_info(device: &CameraDevice, bus_info: &str) -> bool {
    handle_of(device).is_some_and(|h| h.bus_info == bus_info)
}

/// A device's `V4L2DeviceHandle`.
fn handle_of(device: &CameraDevice) -> Option<&V4l2DeviceHandle> {
    device.handle.downcast_ref::<V4l2DeviceHandle>()
}

/// Translation of `MaybeAddDevice()`.
fn maybe_add_device(path: &str) {
    let Ok(fd) = open_device_node(path) else {
        return; // can't open it? skip it.
    };
    match is_char_device(fd.as_raw_fd()) {
        Err(_) => return,    // can't stat it? skip it.
        Ok(false) => return, // not a character device.
        Ok(true) => {}
    }

    let mut vcap = V4l2Capability::default();
    if !ioctl(fd.as_raw_fd(), VIDIOC_QUERYCAP, &mut vcap) {
        return; // probably not a v4l2 device at all.
    } else if (vcap.device_caps & V4L2_CAP_VIDEO_CAPTURE) == 0 {
        return; // not a video capture device.
    }
    let bus_info = c_array_str(&vcap.bus_info);
    if find_physical_camera_by_callback(|d| find_v4l2_camera_by_bus_info(d, &bus_info)).is_ok() {
        return; // already have it.
    }

    let specs = gather_specs(&DeviceFd(fd.as_raw_fd()));

    drop(fd);

    if !specs.is_empty() {
        let handle = V4l2DeviceHandle {
            path: path.to_owned(),
            bus_info,
        };
        add_camera(
            &c_array_str(&vcap.card),
            CameraPosition::Unknown,
            &specs,
            Box::new(handle),
        );
    }
}

/// Translation of `FindV4L2CameraByPathCallback()`.
fn find_v4l2_camera_by_path(device: &CameraDevice, path: &str) -> bool {
    handle_of(device).is_some_and(|h| h.path == path)
}

/// Translation of `MaybeRemoveDevice()`.
fn maybe_remove_device(path: &str) {
    if let Ok(device) = find_physical_camera_by_callback(|d| find_v4l2_camera_by_path(d, path)) {
        camera_disconnected(&device);
    }
}

/// Translation of `CameraUdevCallback()`.
fn camera_udev_callback(udev_type: UdevDeviceEvent, udev_class: DeviceClass, devpath: &str) {
    // FIXME (upstream): udev reports removals without a device class, so
    // the VIDEO_CAPTURE check below drops every removal and an unplugged
    // camera is never reported as disconnected.
    if udev_class.intersects(DeviceClass::VIDEO_CAPTURE) {
        match udev_type {
            UdevDeviceEvent::Added => maybe_add_device(devpath),
            UdevDeviceEvent::Removed => maybe_remove_device(devpath),
        }
    }
}

/// The number in a `/dev` entry named like `video%d` (`SDL_sscanf()`
/// semantics: leading space and a sign are allowed, trailing text ignored).
fn video_device_number(name: &str) -> Option<i32> {
    let rest = name.strip_prefix("video")?.trim_start();
    let sign_len = usize::from(rest.starts_with(['-', '+']));
    let digits = rest[sign_len..]
        .bytes()
        .take_while(u8::is_ascii_digit)
        .count();
    (digits > 0).then(|| crate::stdlib::atoi(&rest[..sign_len + digits]))
}

/// The driver (`V4L2_Init()`'s function table).
struct V4l2Driver;

impl CameraDriverImpl for V4l2Driver {
    fn detect_devices(&self) {
        // Translation of `V4L2_DetectDevices()`.
        if udev::init().is_ok() {
            if udev::add_callback(camera_udev_callback).is_ok() {
                let _ = udev::scan(); // Force a scan to build the initial device list
            }
            return;
        }

        if let Ok(dir) = std::fs::read_dir("/dev") {
            for dent in dir.flatten() {
                if let Some(num) = video_device_number(&dent.file_name().to_string_lossy()) {
                    let fullpath = format!("/dev/video{num}");
                    maybe_add_device(&fullpath);
                }
            }
        }
    }

    fn open_device(
        &self,
        device: &Arc<CameraDevice>,
        spec: &CameraSpec,
    ) -> Result<Arc<dyn CameraBackend>> {
        // Translation of `V4L2_OpenDevice()`.
        let Some(handle) = handle_of(device) else {
            return Err(Error::new("Not a V4L2 device"));
        };
        let path = &handle.path;
        let strerror = |e: &std::io::Error| {
            crate::core::linux::input::errno_string(e.raw_os_error().unwrap_or(0))
        };

        // most of this probably shouldn't fail unless the filesystem node changed out from under us since MaybeAddDevice().
        let fd = open_device_node(path).map_err(|e| {
            Error::new(format!(
                "Cannot open '{path}': {}, {}",
                e.raw_os_error().unwrap_or(0),
                strerror(&e)
            ))
        })?;
        match is_char_device(fd.as_raw_fd()) {
            Err(e) => {
                return Err(Error::new(format!(
                    "Cannot identify '{path}': {}, {}",
                    e.raw_os_error().unwrap_or(0),
                    strerror(&e)
                )));
            }
            Ok(false) => return Err(Error::new(format!("{path} is not a character device"))),
            Ok(true) => {}
        }
        let mut cap = V4l2Capability::default();
        if !xioctl(fd.as_raw_fd(), VIDIOC_QUERYCAP, &mut cap) {
            let err = errno();
            if err == libc::EINVAL {
                return Err(Error::new(format!(
                    "{path} is unexpectedly not a V4L2 device"
                )));
            }
            return Err(Error::new(format!(
                "Error VIDIOC_QUERYCAP errno={err} device{path} is no V4L2 device"
            )));
        } else if (cap.device_caps & V4L2_CAP_VIDEO_CAPTURE) == 0 {
            return Err(Error::new(format!(
                "{path} is unexpectedly not a video capture device"
            )));
        }
        let fd_raw = fd.as_raw_fd();

        // Select video input, video standard and tune here.
        // errors in the crop code are not fatal.
        let mut cropcap = V4l2Cropcap {
            type_: V4L2_BUF_TYPE_VIDEO_CAPTURE,
            ..V4l2Cropcap::default()
        };
        if xioctl(fd_raw, VIDIOC_CROPCAP, &mut cropcap) {
            let mut crop = V4l2Crop {
                type_: V4L2_BUF_TYPE_VIDEO_CAPTURE,
                c: cropcap.defrect, // reset to default
            };
            xioctl(fd_raw, VIDIOC_S_CROP, &mut crop);
        }

        let mut fmt = V4l2Format {
            type_: V4L2_BUF_TYPE_VIDEO_CAPTURE,
            ..V4l2Format::default()
        };
        fmt.fmt.pix.width = spec.width as u32;
        fmt.fmt.pix.height = spec.height as u32;
        fmt.fmt.pix.pixelformat = format_sdl_to_v4l2(spec.format);
        //fmt.fmt.pix.field = V4L2_FIELD_INTERLACED;
        fmt.fmt.pix.field = V4L2_FIELD_ANY;

        if !xioctl(fd_raw, VIDIOC_S_FMT, &mut fmt) {
            return Err(Error::new("Error VIDIOC_S_FMT"));
        }

        if spec.framerate_numerator != 0 && spec.framerate_denominator != 0 {
            let mut setfps = V4l2Streamparm {
                type_: V4L2_BUF_TYPE_VIDEO_CAPTURE,
                ..V4l2Streamparm::default()
            };
            if xioctl(fd_raw, VIDIOC_G_PARM, &mut setfps) {
                let tpf = setfps.capture.timeperframe;
                if tpf.denominator != spec.framerate_numerator as u32
                    || tpf.numerator != spec.framerate_denominator as u32
                {
                    setfps.type_ = V4L2_BUF_TYPE_VIDEO_CAPTURE;
                    setfps.capture.timeperframe.numerator = spec.framerate_denominator as u32;
                    setfps.capture.timeperframe.denominator = spec.framerate_numerator as u32;
                    if !xioctl(fd_raw, VIDIOC_S_PARM, &mut setfps) {
                        return Err(Error::new("Error VIDIOC_S_PARM"));
                    }
                }
            }
        }

        let mut fmt = V4l2Format {
            type_: V4L2_BUF_TYPE_VIDEO_CAPTURE,
            ..V4l2Format::default()
        };
        if !xioctl(fd_raw, VIDIOC_G_FMT, &mut fmt) {
            return Err(Error::new("Error VIDIOC_G_FMT"));
        }
        let driver_pitch = fmt.fmt.pix.bytesperline as i32;

        #[derive(Clone, Copy)]
        enum Method {
            Read,
            Mmap,
            UserPtr,
        }
        let mut io = None;
        let mut nb_buffers = 0;
        if (cap.device_caps & V4L2_CAP_STREAMING) != 0 {
            let mut req = V4l2Requestbuffers {
                count: 8,
                type_: V4L2_BUF_TYPE_VIDEO_CAPTURE,
                memory: V4L2_MEMORY_MMAP,
                ..V4l2Requestbuffers::default()
            };
            if xioctl(fd_raw, VIDIOC_REQBUFS, &mut req) && req.count >= 2 {
                io = Some(Method::Mmap);
                nb_buffers = req.count;
            } else {
                // mmap didn't work out? Try USERPTR.
                let mut req = V4l2Requestbuffers {
                    count: 8,
                    type_: V4L2_BUF_TYPE_VIDEO_CAPTURE,
                    memory: V4L2_MEMORY_USERPTR,
                    ..V4l2Requestbuffers::default()
                };
                if xioctl(fd_raw, VIDIOC_REQBUFS, &mut req) {
                    io = Some(Method::UserPtr);
                    nb_buffers = 8;
                }
            }
        }

        if io.is_none() && (cap.device_caps & V4L2_CAP_READWRITE) != 0 {
            io = Some(Method::Read);
        }

        let Some(io) = io else {
            return Err(Error::new("Don't have a way to talk to this device"));
        };

        // FIXME (upstream): the buffer size comes from device->spec, which
        // SDL_OpenCamera() only fills in after OpenDevice() returns, so it's
        // all zeros here and the read() and user-pointer buffers are 0
        // bytes long.
        let app_spec = device.lock().borrow().spec;
        let (size, _pitch) =
            calculate_surface_size(app_spec.format, app_spec.width, app_spec.height, false)?;

        let io = match io {
            Method::Read => Io::Read {
                buffer: Some(alloc_buffer(size)),
                length: size,
            },
            Method::Mmap => Io::Mmap {
                buffers: alloc_buffer_mmap(fd_raw, nb_buffers)?,
                spare: Vec::new(),
            },
            Method::UserPtr => Io::UserPtr {
                buffers: alloc_buffer_user_ptr(nb_buffers, size),
            },
        };
        let mut hidden = Hidden {
            io,
            driver_pitch,
            fd,
        };

        hidden.enqueue_buffers()?;
        if !matches!(hidden.io, Io::Read { .. }) {
            let mut type_: c_int = V4L2_BUF_TYPE_VIDEO_CAPTURE as c_int;
            if !xioctl(fd_raw, VIDIOC_STREAMON, &mut type_) {
                return Err(Error::new("VIDIOC_STREAMON"));
            }
        }

        // Currently there is no user permission prompt for camera access, but maybe there will be a D-Bus portal interface at some point.
        camera_permission_outcome(device, true);

        Ok(Arc::new(V4l2Camera {
            hidden: Mutex::new(Some(hidden)),
        }))
    }

    fn free_device_handle(&self, _device: &CameraDevice) {
        // Translation of `V4L2_FreeDeviceHandle()`: the handle is freed with
        // the device.
    }

    fn deinitialize(&self) {
        // Translation of `V4L2_Deinitialize()`.
        udev::del_callback(camera_udev_callback);
        udev::quit();
    }
}

/// Translation of `V4L2_Init()`.
fn v4l2_init() -> Result<Arc<dyn CameraDriverImpl>> {
    Ok(Arc::new(V4l2Driver))
}

/// Translation of `V4L2_bootstrap`.
pub(super) static V4L2_BOOTSTRAP: CameraBootStrap = CameraBootStrap {
    name: "v4l2",
    desc: "SDL Video4Linux2 camera driver",
    init: v4l2_init,
    demand_only: false,
};

#[cfg(test)]
mod tests;
