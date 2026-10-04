// Rust translation of src/video/wayland/SDL_waylandshmbuffer.c and
// SDL_waylandshmbuffer.h from Simple DirectMedia Layer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! Shared memory pools for `wl_buffer`s (cursors, icons, the framebuffer)
//! and single pixel buffers.

use std::os::fd::{AsFd, FromRawFd, OwnedFd};
use std::ptr::NonNull;

use super::client::{AsProxy, Obj, Proxy};
use super::protocols::single_pixel_buffer_v1::*;
use super::protocols::wayland::*;
use super::video::Globals;
use crate::error::{Error, Result};

/// Translation of `SetTempFileSize()`.
fn set_temp_file_size(fd: &OwnedFd, size: libc::off_t) -> bool {
    use std::os::fd::AsRawFd;

    #[cfg(any(target_os = "linux", target_os = "android", target_os = "freebsd"))]
    {
        // SAFETY: the signal sets are plain data initialized by sigemptyset;
        // the descriptor is ours.
        let ret = unsafe {
            let mut set: libc::sigset_t = std::mem::zeroed();
            let mut old_set: libc::sigset_t = std::mem::zeroed();

            /* SIGALRM can potentially block a large posix_fallocate() operation
             * from succeeding, so block it.
             */
            libc::sigemptyset(&mut set);
            libc::sigaddset(&mut set, libc::SIGALRM);
            libc::sigprocmask(libc::SIG_BLOCK, &set, &mut old_set);

            let mut ret;
            loop {
                ret = libc::posix_fallocate(fd.as_raw_fd(), 0, size);
                if ret != libc::EINTR {
                    break;
                }
            }

            libc::sigprocmask(libc::SIG_SETMASK, &old_set, std::ptr::null_mut());
            ret
        };

        if ret == 0 {
            return true;
        } else if ret != libc::EINVAL
            && std::io::Error::last_os_error().raw_os_error() != Some(libc::EOPNOTSUPP)
        {
            return false;
        }
    }

    // SAFETY: the descriptor is ours.
    if unsafe { libc::ftruncate(fd.as_raw_fd(), size) } < 0 {
        return false;
    }
    true
}

/// Translation of `CreateTempFD()`.
fn create_temp_fd(size: libc::off_t) -> Option<OwnedFd> {
    #[cfg(any(target_os = "linux", target_os = "android", target_os = "freebsd"))]
    // SAFETY: the name is NUL-terminated.
    let fd =
        unsafe { libc::memfd_create(c"SDL".as_ptr(), libc::MFD_CLOEXEC | libc::MFD_ALLOW_SEALING) };
    #[cfg(not(any(target_os = "linux", target_os = "android", target_os = "freebsd")))]
    let fd = -1;

    let fd = if fd >= 0 {
        // SAFETY: a new descriptor we own.
        let fd = unsafe { OwnedFd::from_raw_fd(fd) };
        #[cfg(any(target_os = "linux", target_os = "android", target_os = "freebsd"))]
        // SAFETY: the descriptor is ours.
        unsafe {
            libc::fcntl(
                std::os::fd::AsRawFd::as_raw_fd(&fd),
                libc::F_ADD_SEALS,
                libc::F_SEAL_SHRINK | libc::F_SEAL_SEAL,
            );
        }
        fd
    } else {
        const TEMPLATE: &str = "/sdl-shared-XXXXXX";

        let xdg_path = crate::stdlib::getenv("XDG_RUNTIME_DIR")?;

        let mut tmp_path = Vec::with_capacity(xdg_path.len() + TEMPLATE.len() + 1);
        tmp_path.extend_from_slice(xdg_path.as_bytes());
        tmp_path.extend_from_slice(TEMPLATE.as_bytes());
        tmp_path.retain(|&b| b != 0);
        tmp_path.push(0);

        // SAFETY: the template is NUL-terminated and writable.
        let fd = unsafe { libc::mkostemp(tmp_path.as_mut_ptr().cast(), libc::O_CLOEXEC) };
        if fd < 0 {
            return None;
        }

        // Need to manually unlink the temp files, or they can persist after close and fill up the temp storage.
        // SAFETY: mkostemp filled in the name.
        unsafe { libc::unlink(tmp_path.as_ptr().cast()) };
        // SAFETY: a new descriptor we own.
        unsafe { OwnedFd::from_raw_fd(fd) }
    };

    if !set_temp_file_size(&fd, size) {
        return None;
    }

    Some(fd)
}

/// A shared memory pool, mapped in this process. Translation of
/// `struct Wayland_SHMPool`; dropping it is `Wayland_ReleaseSHMPool()` (the
/// buffers made from it stay valid).
pub(crate) struct ShmPool {
    shm_pool: Proxy<WlShmPool>,
    shm_pool_memory: NonNull<u8>,
    shm_pool_size: usize,
    offset: usize,
}

// SAFETY: the mapping is owned by the pool and only reached through it.
unsafe impl Send for ShmPool {}

impl std::fmt::Debug for ShmPool {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ShmPool")
            .field("size", &self.shm_pool_size)
            .finish()
    }
}

impl ShmPool {
    /// Translation of `Wayland_AllocSHMPool()`.
    pub(crate) fn alloc(shm: Obj<'_, WlShm>, size: i32) -> Result<ShmPool> {
        if size <= 0 {
            return Err(Error::invalid_param("size"));
        }

        let shm_pool_size = ((size + 15) & !15) as usize;

        let Some(shm_fd) = create_temp_fd(shm_pool_size as libc::off_t) else {
            return Err(Error::new("Creating SHM buffer failed."));
        };

        // SAFETY: a shared read/write mapping of the whole (sized) file.
        let memory = unsafe {
            libc::mmap(
                std::ptr::null_mut(),
                shm_pool_size,
                libc::PROT_READ | libc::PROT_WRITE,
                libc::MAP_SHARED,
                std::os::fd::AsRawFd::as_raw_fd(&shm_fd),
                0,
            )
        };
        if memory == libc::MAP_FAILED {
            return Err(Error::new("mmap() failed."));
        }
        let Some(shm_pool_memory) = NonNull::new(memory.cast::<u8>()) else {
            return Err(Error::new("mmap() failed."));
        };

        let shm_pool = shm.create_pool(shm_fd.as_fd(), shm_pool_size as i32);
        // (the descriptor is closed when dropped: the compositor has its own)
        drop(shm_fd);

        Ok(ShmPool {
            shm_pool,
            shm_pool_memory,
            shm_pool_size,
            offset: 0,
        })
    }

    /// An ARGB8888 buffer of `width`x`height` from the pool, and its pixels.
    /// Translation of `Wayland_AllocBufferFromPool()`.
    pub(crate) fn alloc_buffer(
        &mut self,
        width: i32,
        height: i32,
    ) -> Option<(Proxy<WlBuffer>, &mut [u8])> {
        const SHM_FMT: WlShmFormat = WlShmFormat::ARGB8888;
        self.alloc_buffer_with_format(width, height, SHM_FMT)
    }

    /// A buffer of `width`x`height` 32-bit pixels in `format` from the pool,
    /// and its pixels (`Wayland_AllocBufferFromPool()` with a format, for the
    /// framebuffer).
    pub(crate) fn alloc_buffer_with_format(
        &mut self,
        width: i32,
        height: i32,
        format: WlShmFormat,
    ) -> Option<(Proxy<WlBuffer>, &mut [u8])> {
        if width <= 0 || height <= 0 {
            return None;
        }
        let len = width as usize * height as usize * 4;
        if self.offset + len > self.shm_pool_size {
            // (upstream trusts its callers to size the pool)
            return None;
        }

        let buffer =
            self.shm_pool
                .create_buffer(self.offset as i32, width, height, width * 4, format);
        // (the buffer release event is a no-op, as upstream's listener)

        // SAFETY: the range is inside the mapping and no other slice of it
        // is handed out (the offset moves past it).
        let data = unsafe {
            std::slice::from_raw_parts_mut(self.shm_pool_memory.as_ptr().add(self.offset), len)
        };
        self.offset += len;

        Some((buffer, data))
    }
}

impl ShmPool {
    /// The whole mapping of the pool.
    pub(crate) fn memory_mut(&mut self) -> &mut [u8] {
        // SAFETY: the mapping is `shm_pool_size` bytes long; the pool is
        // borrowed mutably, so no other slice of it is alive.
        unsafe { std::slice::from_raw_parts_mut(self.shm_pool_memory.as_ptr(), self.shm_pool_size) }
    }
}

impl Drop for ShmPool {
    fn drop(&mut self) {
        // (the pool proxy is destroyed when the field drops)
        // SAFETY: the mapping was made by alloc() with this size, and the
        // slices of it borrowed the pool, so they are gone.
        unsafe {
            libc::munmap(self.shm_pool_memory.as_ptr().cast(), self.shm_pool_size);
        }
    }
}

/// A 1x1 buffer of one color (components scaled to 32 bits). Translation of
/// `Wayland_CreateSinglePixelBuffer()`.
pub(crate) fn wayland_create_single_pixel_buffer(
    globals: &Globals,
    r: u32,
    g: u32,
    b: u32,
    a: u32,
) -> Option<Proxy<WlBuffer>> {
    // The single-pixel buffer protocol is preferred, as the compositor can choose an optimal format.
    if let Some(m) = &globals.single_pixel_buffer_manager {
        Some(m.create_u32_rgba_buffer(r, g, b, a))
    } else {
        let shm = globals.shm.as_ref()?;
        let mut pool = ShmPool::alloc(shm.obj(), 4).ok()?;

        let (wl_buffer, mem) = pool.alloc_buffer(1, 1)?;

        // FIXME (upstream): ARGB8888 is B, G, R, A in memory on little-endian
        // machines, so red and blue are swapped here (harmless for the black
        // masks SDL makes, kept as upstream).
        let pixel: [u8; 4] = [
            (r >> 24) as u8,
            (g >> 24) as u8,
            (b >> 24) as u8,
            (a >> 24) as u8,
        ];
        mem.copy_from_slice(&pixel);

        drop(pool);
        Some(wl_buffer)
    }
}
