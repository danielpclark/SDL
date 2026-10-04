// Rust translation of src/core/unix/SDL_poll.c and src/core/unix/SDL_appid.c
// from Simple DirectMedia Layer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! Unix glue shared by the backends: waiting on a file descriptor
//! (`SDL_IOReady()`) and the application ID the desktop knows the app by
//! (`SDL_GetAppID()`).

use std::os::fd::RawFd;

/// What [`io_ready`] waits for (`SDL_IOR_READ`, `SDL_IOR_WRITE`,
/// `SDL_IOR_NO_RETRY`).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) struct IoReadyFlags(u32);

impl IoReadyFlags {
    pub(crate) const READ: IoReadyFlags = IoReadyFlags(0x1);
    pub(crate) const WRITE: IoReadyFlags = IoReadyFlags(0x2);
    /// Return on `EINTR` instead of polling again.
    pub(crate) const NO_RETRY: IoReadyFlags = IoReadyFlags(0x4);

    fn contains(self, other: IoReadyFlags) -> bool {
        self.0 & other.0 != 0
    }
}

impl std::ops::BitOr for IoReadyFlags {
    type Output = IoReadyFlags;
    fn bitor(self, rhs: IoReadyFlags) -> IoReadyFlags {
        IoReadyFlags(self.0 | rhs.0)
    }
}

/// Wait until `fd` is readable and/or writable, for up to `timeout_ns`
/// nanoseconds (negative waits forever). Returns poll(2)'s result: 1 if
/// ready, 0 on timeout, -1 on error. Translation of `SDL_IOReady()`.
pub(crate) fn io_ready(fd: RawFd, flags: IoReadyFlags, timeout_ns: i64) -> i32 {
    debug_assert!(flags.contains(IoReadyFlags::READ | IoReadyFlags::WRITE));

    // Note: We don't bother to account for elapsed time if we get EINTR
    loop {
        let mut info = libc::pollfd {
            fd,
            events: 0,
            revents: 0,
        };
        if flags.contains(IoReadyFlags::READ) {
            info.events |= libc::POLLIN | libc::POLLPRI;
        }
        if flags.contains(IoReadyFlags::WRITE) {
            info.events |= libc::POLLOUT;
        }

        #[cfg(any(target_os = "linux", target_os = "android", target_os = "freebsd"))]
        let result = {
            let ts;
            let timeout = if timeout_ns >= 0 {
                ts = libc::timespec {
                    tv_sec: (timeout_ns / 1_000_000_000) as _,
                    tv_nsec: (timeout_ns % 1_000_000_000) as _,
                };
                &ts as *const libc::timespec
            } else {
                std::ptr::null()
            };
            // SAFETY: info is one valid pollfd; timeout is NULL or points to
            // a timespec that outlives the call; no signal mask is passed.
            unsafe { libc::ppoll(&mut info, 1, timeout, std::ptr::null()) }
        };
        #[cfg(not(any(target_os = "linux", target_os = "android", target_os = "freebsd")))]
        let result = {
            let timeout_ms = if timeout_ns > 0 {
                ((timeout_ns + (1_000_000 - 1)) / 1_000_000) as i32
            } else if timeout_ns == 0 {
                0
            } else {
                -1
            };
            // SAFETY: info is one valid pollfd.
            unsafe { libc::poll(&mut info, 1, timeout_ms) }
        };

        if !(result < 0
            && std::io::Error::last_os_error().raw_os_error() == Some(libc::EINTR)
            && !flags.contains(IoReadyFlags::NO_RETRY))
        {
            return result;
        }
    }
}

/// The application ID: the app's identifier metadata, then (on Linux) the
/// Flatpak ID, then the executable name, then "SDL_App". Translation of
/// `SDL_GetAppID()`.
pub(crate) fn app_id() -> String {
    let mut id = crate::init::app_metadata_property(crate::init::AppMetadata::Identifier);

    #[cfg(target_os = "linux")]
    if id.is_none() {
        id = crate::stdlib::getenv("FLATPAK_ID");
    }

    if id.is_none() {
        // If the hint isn't set, try to use the application's executable name
        id = crate::filesystem::get_exe_name().ok();
    }

    // Finally, use the default we've used forever
    id.unwrap_or_else(|| "SDL_App".to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use std::os::fd::AsRawFd;
    use std::os::unix::net::UnixStream;

    #[test]
    fn io_ready_on_a_socket() {
        let (mut a, b) = UnixStream::pair().unwrap();
        let fd = b.as_raw_fd();
        // Writable at once, not readable until data arrives
        assert_eq!(io_ready(fd, IoReadyFlags::WRITE, 0), 1);
        assert_eq!(io_ready(fd, IoReadyFlags::READ, 0), 0);
        assert_eq!(io_ready(fd, IoReadyFlags::READ, 1_000_000), 0);
        a.write_all(b"x").unwrap();
        assert_eq!(io_ready(fd, IoReadyFlags::READ, -1), 1);
        assert_eq!(
            io_ready(fd, IoReadyFlags::READ | IoReadyFlags::NO_RETRY, 0),
            1
        );
    }

    #[test]
    fn app_id_falls_back_to_the_executable() {
        let _l = crate::test_support::TEST_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        crate::init::set_app_metadata_property(
            crate::init::AppMetadata::Identifier,
            Some("org.libsdl.test"),
        );
        assert_eq!(app_id(), "org.libsdl.test");
        crate::init::set_app_metadata_property(crate::init::AppMetadata::Identifier, None);
        if crate::stdlib::getenv("FLATPAK_ID").is_none() {
            assert_eq!(app_id(), crate::filesystem::get_exe_name().unwrap());
        }
    }
}
