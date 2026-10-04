// Rust translation of src/video/wayland/SDL_waylanddatamanager.c and
// SDL_waylanddatamanager.h from Simple DirectMedia Layer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! Data sources (what we offer: the clipboard and the primary selection)
//! and data offers (what others offer: selections and drag and drop), with
//! the pipe transfers between them.
//!
//! The data devices of the seats are in [`super::events`] with their
//! listeners. A source's listener holds what it needs to answer requests
//! (the data callback and the device ID); an offer's listener appends to its
//! mime type list.

use std::os::fd::{AsFd, AsRawFd, FromRawFd, OwnedFd};
use std::sync::{Arc, Mutex};

use super::client::{AsProxy, Proxy};
use super::protocols::wayland::*;
use super::protocols::wp_primary_selection_unstable_v1::*;
use super::video::{VideoData, WaylandVideo};
use crate::core::unix::{io_ready, IoReadyFlags};
use crate::error::{Error, Result};
use crate::events::WindowID;
use crate::video::clipboard::ClipboardDataCallback;

pub(crate) const TEXT_MIME: &str = "text/plain;charset=utf-8";
pub(crate) const FILE_MIME: &str = "text/uri-list";
pub(crate) const FILE_PORTAL_MIME: &str = "application/vnd.portal.filetransfer";
pub(crate) const SDL_DATA_ORIGIN_MIME: &str = "application/x-sdl3-source-id";

/* This is arbitrary, but reading while polling should block for less than a frame, to
 * prevent hanging while pumping events.
 *
 * When querying the clipboard data directly, a larger value is needed to avoid timing
 * out if the source needs to process or transfer a large amount of data.
 */
const DEFAULT_PIPE_TIMEOUT_NS: i64 = 14_000_000;
const EXTENDED_PIPE_TIMEOUT_NS: i64 = 5_000_000_000;

/// `PIPE_BUF`
const PIPE_BUF: usize = libc::PIPE_BUF;

/// The mime types of an offer (`SDL_MimeDataList` without data: offers
/// never store any), newest first as upstream's list.
pub(crate) type MimeList = Arc<Mutex<Vec<String>>>;

fn mimes(list: &MimeList) -> std::sync::MutexGuard<'_, Vec<String>> {
    list.lock().unwrap_or_else(|e| e.into_inner())
}

/// What a source hands out: the callback producing the data, and the ID of
/// the data device for the origin mime type (`SDL_WaylandUserdata` and the
/// callback of `SDL_WaylandDataSource`).
#[derive(Default)]
pub(crate) struct SourceData {
    pub(crate) callback: Option<ClipboardDataCallback>,
    /// The clipboard sequence (0 for primary selection text).
    pub(crate) sequence: u32,
    /// `data_device->id_str`
    pub(crate) id_str: Option<String>,
}

pub(crate) type SharedSourceData = Arc<Mutex<SourceData>>;

fn source_data(s: &SharedSourceData) -> std::sync::MutexGuard<'_, SourceData> {
    s.lock().unwrap_or_else(|e| e.into_inner())
}

/// A clipboard source. Translation of `SDL_WaylandDataSource`.
pub(crate) struct DataSource {
    pub(crate) source: Proxy<WlDataSource>,
    /// The seat of the data device the source is set on (`data_device`).
    pub(crate) data_device: Option<u32>,
    pub(crate) data: SharedSourceData,
}

/// A primary selection source. Translation of
/// `SDL_WaylandPrimarySelectionSource`.
pub(crate) struct PrimarySelectionSource {
    pub(crate) source: Proxy<ZwpPrimarySelectionSourceV1>,
    /// The seat of the device the source is set on (`primary_selection_device`).
    pub(crate) primary_selection_device: Option<u32>,
    pub(crate) data: SharedSourceData,
}

/// An offer of the clipboard or of a drag. Translation of
/// `SDL_WaylandDataOffer`.
pub(crate) struct DataOffer {
    pub(crate) offer: Proxy<WlDataOffer>,
    pub(crate) mimes: MimeList,
    /// The seat of the data device (`data_device`).
    #[allow(dead_code)] // (the seat is known where the offer is used)
    pub(crate) data_device: u32,

    // Callback data for queued receive.
    pub(crate) callback: Option<Proxy<WlCallback>>,
    pub(crate) read_fd: Option<OwnedFd>,
}

/// An offer of the primary selection. Translation of
/// `SDL_WaylandPrimarySelectionOffer`.
pub(crate) struct PrimarySelectionOffer {
    pub(crate) offer: Proxy<ZwpPrimarySelectionOfferV1>,
    pub(crate) mimes: MimeList,
    /// The seat of the device (`primary_selection_device`).
    #[allow(dead_code)] // (the seat is known where the offer is used)
    pub(crate) primary_selection_device: u32,
}

/// The data device of a seat. Translation of `struct SDL_WaylandDataDevice`.
pub(crate) struct DataDevice {
    pub(crate) data_device: Proxy<WlDataDevice>,
    pub(crate) seat: u32,
    pub(crate) id_str: String,

    // Drag and Drop
    pub(crate) drag_serial: u32,
    pub(crate) drag_offer: Option<DataOffer>,
    pub(crate) selection_offer: Option<DataOffer>,
    /// Offers announced (`data_offer`) and not yet used for a drag or a
    /// selection (upstream finds them through the proxy's user data).
    pub(crate) new_offers: Vec<DataOffer>,
    pub(crate) mime_type: Option<&'static str>,
    pub(crate) has_mime_file: bool,
    pub(crate) has_mime_text: bool,
    pub(crate) dnd_window: Option<WindowID>,
    pub(crate) dnd_surface: usize,

    // Clipboard and Primary Selection
    pub(crate) selection_serial: u32,
    pub(crate) selection_source: Option<DataSource>,
}

/// The primary selection device of a seat. Translation of
/// `struct SDL_WaylandPrimarySelectionDevice`.
pub(crate) struct PrimarySelectionDevice {
    pub(crate) primary_selection_device: Proxy<ZwpPrimarySelectionDeviceV1>,
    pub(crate) seat: u32,

    pub(crate) selection_serial: u32,
    pub(crate) selection_source: Option<PrimarySelectionSource>,
    pub(crate) selection_offer: Option<PrimarySelectionOffer>,
    /// Offers announced and not yet selected.
    pub(crate) new_offers: Vec<PrimarySelectionOffer>,
}

/// Translation of `WritePipe()`: write the next chunk of `buffer` from
/// `pos`, with SIGPIPE blocked.
fn write_pipe(fd: &OwnedFd, buffer: &[u8], pos: &mut usize) -> isize {
    let mut bytes_written: isize = 0;
    let length = buffer.len() as isize - *pos as isize;

    let ready = io_ready(fd.as_raw_fd(), IoReadyFlags::WRITE, DEFAULT_PIPE_TIMEOUT_NS);

    // SAFETY: the signal sets are plain data initialized by sigemptyset.
    let (sig_set, old_sig_set) = unsafe {
        let mut sig_set: libc::sigset_t = std::mem::zeroed();
        let mut old_sig_set: libc::sigset_t = std::mem::zeroed();
        libc::sigemptyset(&mut sig_set);
        libc::sigaddset(&mut sig_set, libc::SIGPIPE);
        libc::pthread_sigmask(libc::SIG_BLOCK, &sig_set, &mut old_sig_set);
        (sig_set, old_sig_set)
    };

    if ready > 0 {
        if length > 0 {
            let n = (length as usize).min(PIPE_BUF);
            // SAFETY: the range is inside the buffer; the descriptor is ours.
            bytes_written =
                unsafe { libc::write(fd.as_raw_fd(), buffer.as_ptr().add(*pos).cast(), n) };
        }

        if bytes_written > 0 {
            *pos += bytes_written as usize;
        }
    } else if ready == 0 {
        // ("Pipe timeout")
    } else {
        // ("Pipe poll error")
    }

    // Consume a SIGPIPE raised by the write, if any, then restore the mask.
    // SAFETY: the sets are initialized; a zero timeout doesn't wait.
    unsafe {
        let zerotime = libc::timespec {
            tv_sec: 0,
            tv_nsec: 0,
        };
        #[cfg(any(target_os = "linux", target_os = "android", target_os = "freebsd"))]
        libc::sigtimedwait(&sig_set, std::ptr::null_mut(), &zerotime);
        #[cfg(not(any(target_os = "linux", target_os = "android", target_os = "freebsd")))]
        let _ = (&sig_set, &zerotime);
        libc::pthread_sigmask(libc::SIG_SETMASK, &old_sig_set, std::ptr::null_mut());
    }

    bytes_written
}

/// Translation of `ReadPipe()`: append what can be read within the
/// timeout; the number of bytes read (0 at the end or on timeout).
fn read_pipe(fd: &OwnedFd, buffer: &mut Option<Vec<u8>>, timeout_ns: i64) -> isize {
    let mut temp = [0u8; PIPE_BUF];
    let mut bytes_read: isize = 0;

    let ready = io_ready(fd.as_raw_fd(), IoReadyFlags::READ, timeout_ns);

    if ready > 0 {
        // SAFETY: temp is writable for its length; the descriptor is ours.
        bytes_read = unsafe { libc::read(fd.as_raw_fd(), temp.as_mut_ptr().cast(), temp.len()) };
    } else if ready == 0 {
        // ("Pipe timeout")
    } else {
        // ("Pipe poll error")
    }

    if bytes_read > 0 {
        // (upstream keeps 4 zero bytes after the data; a Vec knows its
        // length)
        buffer
            .get_or_insert_with(Vec::new)
            .extend_from_slice(&temp[..bytes_read as usize]);
    }

    bytes_read
}

/// A pipe, both ends close-on-exec and non-blocking (`pipe2()`).
fn pipe2() -> Option<(OwnedFd, OwnedFd)> {
    let mut pipefd = [0; 2];
    // SAFETY: pipefd has room for the two descriptors.
    if unsafe { libc::pipe2(pipefd.as_mut_ptr(), libc::O_CLOEXEC | libc::O_NONBLOCK) } == 0 {
        // SAFETY: two new descriptors we own.
        unsafe {
            Some((
                OwnedFd::from_raw_fd(pipefd[0]),
                OwnedFd::from_raw_fd(pipefd[1]),
            ))
        }
    } else {
        None
    }
}

/// Translation of `SendData()`: write all of `data` (closing the pipe).
fn send_data(data: Option<&[u8]>, fd: OwnedFd) -> usize {
    let mut result = 0;

    if let Some(data) = data.filter(|d| !d.is_empty()) {
        while write_pipe(&fd, data, &mut result) > 0 {
            // Just keep spinning
        }
    }
    drop(fd);

    result
}

/// Answer a request for `mime_type` from a clipboard source. Translation of
/// `Wayland_DataSourceSend()`.
pub(crate) fn wayland_data_source_send(
    source: &SharedSourceData,
    mime_type: &str,
    fd: OwnedFd,
) -> usize {
    let (callback, id_str) = {
        let s = source_data(source);
        (s.callback.clone(), s.id_str.clone())
    };
    if mime_type == SDL_DATA_ORIGIN_MIME {
        let id = id_str.unwrap_or_default();
        send_data(Some(id.as_bytes()), fd)
    } else if let Some(callback) = callback {
        let data = callback(mime_type);
        send_data(data.as_deref(), fd)
    } else {
        send_data(None, fd)
    }
}

/// Answer a request from a primary selection source. Translation of
/// `Wayland_PrimarySelectionSourceSend()`.
pub(crate) fn wayland_primary_selection_source_send(
    source: &SharedSourceData,
    mime_type: &str,
    fd: OwnedFd,
) -> usize {
    let callback = source_data(source).callback.clone();
    match callback {
        Some(callback) => {
            let data = callback(mime_type);
            send_data(data.as_deref(), fd)
        }
        None => send_data(None, fd),
    }
}

/// Translation of `CloneDataBuffer()`: a copy of the data (none if empty).
fn clone_data_buffer(buffer: Option<Vec<u8>>) -> Option<Vec<u8>> {
    buffer.filter(|b| !b.is_empty())
}

/// The data of a primary selection source of ours. Translation of
/// `Wayland_PrimarySelectionSourceGetData()`.
pub(crate) fn wayland_primary_selection_source_get_data(
    source: Option<&PrimarySelectionSource>,
    mime_type: &str,
) -> Result<Option<Vec<u8>>> {
    let Some(source) = source else {
        return Err(Error::new("Invalid primary selection source"));
    };
    let callback = source_data(&source.data).callback.clone();
    Ok(callback.and_then(|cb| clone_data_buffer(cb(mime_type))))
}

impl WaylandVideo {
    /// The data source listener (`data_source_listener`).
    fn handle_data_source_event(
        &self,
        source: &SharedSourceData,
        raw: usize,
        event: WlDataSourceEvent<'_>,
    ) {
        match event {
            WlDataSourceEvent::Target { .. } => {}
            WlDataSourceEvent::Send { mime_type, fd } => {
                wayland_data_source_send(source, &mime_type.to_string_lossy(), fd);
            }
            WlDataSourceEvent::Cancelled => {
                // Translation of `data_source_handle_cancelled()`.
                let removed = self.with_data(|d| {
                    for s in d.seat_list.iter_mut() {
                        if let Some(dd) = s.data_device.as_mut() {
                            if dd
                                .selection_source
                                .as_ref()
                                .is_some_and(|src| src.source.raw() as usize == raw)
                            {
                                return dd.selection_source.take();
                            }
                        }
                    }
                    None
                });
                if let Some(source) = removed {
                    wayland_data_source_destroy(source);
                }
            }
            WlDataSourceEvent::DndDropPerformed => {}
            WlDataSourceEvent::DndFinished => {}
            WlDataSourceEvent::Action { .. } => {}
        }
    }

    /// Translation of `Wayland_DataSourceCreate()`.
    pub(crate) fn wayland_data_source_create(&self, video_data: &VideoData) -> Option<DataSource> {
        let manager = video_data.g.data_device_manager.as_ref()?;
        let mut id = manager.create_data_source();
        let data: SharedSourceData = Arc::default();
        let shared = data.clone();
        let raw = id.raw() as usize;
        self.listen(&mut id, move |v, _, ev| {
            v.handle_data_source_event(&shared, raw, ev)
        });

        Some(DataSource {
            source: id,
            data_device: None,
            data,
        })
    }

    /// The primary selection source listener
    /// (`primary_selection_source_listener`).
    fn handle_primary_selection_source_event(
        &self,
        source: &SharedSourceData,
        raw: usize,
        event: ZwpPrimarySelectionSourceV1Event<'_>,
    ) {
        match event {
            ZwpPrimarySelectionSourceV1Event::Send { mime_type, fd } => {
                wayland_primary_selection_source_send(source, &mime_type.to_string_lossy(), fd);
            }
            ZwpPrimarySelectionSourceV1Event::Cancelled => {
                let removed = self.with_data(|d| {
                    for s in d.seat_list.iter_mut() {
                        if let Some(pd) = s.primary_selection_device.as_mut() {
                            if pd
                                .selection_source
                                .as_ref()
                                .is_some_and(|src| src.source.raw() as usize == raw)
                            {
                                return pd.selection_source.take();
                            }
                        }
                    }
                    None
                });
                drop(removed);
            }
        }
    }

    /// Translation of `Wayland_PrimarySelectionSourceCreate()`.
    pub(crate) fn wayland_primary_selection_source_create(
        &self,
        video_data: &VideoData,
    ) -> Option<PrimarySelectionSource> {
        let manager = video_data.g.primary_selection_device_manager.as_ref()?;
        let mut id = manager.create_source();
        let data: SharedSourceData = Arc::default();
        let shared = data.clone();
        let raw = id.raw() as usize;
        self.listen(&mut id, move |v, _, ev| {
            v.handle_primary_selection_source_event(&shared, raw, ev)
        });

        Some(PrimarySelectionSource {
            source: id,
            primary_selection_device: None,
            data,
        })
    }

    /// The offer listener of `offer_source_listener`: the origin check of a
    /// selection offer is done. Translation of `offer_source_done_handler()`.
    fn offer_source_done_handler(&self, seat: u32, callback: usize) {
        if callback == 0 {
            return;
        }

        let found = self.with_data(|d| {
            let dd = d.seat_mut(seat)?.data_device.as_mut()?;
            let offer = dd.selection_offer.as_mut()?;
            if offer.callback.as_ref().map(|c| c.raw() as usize) != Some(callback) {
                return None;
            }

            // (wl_callback_destroy())
            offer.callback = None;

            let mut id: Option<Vec<u8>> = None;
            if let Some(fd) = offer.read_fd.take() {
                while read_pipe(&fd, &mut id, DEFAULT_PIPE_TIMEOUT_NS) > 0 {}
                drop(fd);
            }

            Some((id, dd.id_str.clone()))
        });
        let Some((Some(id), id_str)) = found else {
            return;
        };

        let length = id.len();
        let source_is_external = !strncmp_eq(id_str.as_bytes(), &id, length);
        if source_is_external {
            self.selection_offer_notify_from_mimes(seat, false);
        } else {
            // Recursive data offer; just destroy it.
            let offer = self.with_data(|d| {
                d.seat_mut(seat)?
                    .data_device
                    .as_mut()?
                    .selection_offer
                    .take()
            });
            drop(offer);
        }
    }

    /// Translation of `DataOfferCheckSource()`: ask the offer for its origin
    /// ID, which comes back after a sync point.
    fn data_offer_check_source(&self, d: &mut VideoData, seat: u32, mime_type: &str) {
        let conn = self.conn.clone();
        let weak = self.weak();
        let Some(offer) = d
            .seat_mut(seat)
            .and_then(|s| s.data_device.as_mut())
            .and_then(|dd| dd.selection_offer.as_mut())
        else {
            return;
        };

        if let Some((read, write)) = pipe2() {
            // (dropping the old callback and descriptor destroys and closes them)
            offer.callback = None;
            offer.read_fd = Some(read);

            offer.offer.receive(mime_type, write.as_fd());
            drop(write);

            let mut callback = conn.obj().sync();
            callback.listen(move |cb, _| {
                if let Some(v) = weak.upgrade() {
                    v.offer_source_done_handler(seat, cb.raw() as usize);
                }
            });
            offer.callback = Some(callback);

            let _ = conn.flush();
        }
    }

    /// Translation of `UpdateSeatOffers()`: the clipboard sequences to
    /// cancel come back (`SDL_CancelClipboardData()` is called by the
    /// caller, with nothing borrowed).
    fn update_seat_offers(&self, video_data: &mut VideoData, offer_seat: u32) -> Vec<DataSource> {
        let mut destroyed = Vec::new();

        // Clear any existing references to the existing clipboard data before replacing the current offer.
        for s in video_data.seat_list.iter_mut() {
            let key = s.registry_id;
            if let Some(dd) = s.data_device.as_mut() {
                destroyed.extend(dd.selection_source.take());

                // Don't clear the offer that is about to be set.
                if key != offer_seat {
                    dd.selection_offer = None;
                }
            }
        }

        video_data.current_data_offer_seat = Some(offer_seat);
        destroyed
    }

    /// Translation of `SelectionOfferNotifyFromMIMEs()`.
    pub(crate) fn selection_offer_notify_from_mimes(&self, seat: u32, check_origin: bool) {
        let result = self.with_data(|d| {
            let offer_mimes = d
                .seat(seat)
                .and_then(|s| s.data_device.as_ref())
                .and_then(|dd| dd.selection_offer.as_ref())
                .map(|o| mimes(&o.mimes).clone());

            let mut new_mime_types = Vec::new();
            if let Some(list) = offer_mimes {
                for mime_type in &list {
                    // If origin metadata is found, queue a check and wait for confirmation that this offer isn't recursive.
                    if check_origin && mime_type == SDL_DATA_ORIGIN_MIME {
                        self.data_offer_check_source(d, seat, mime_type);
                        return None;
                    }
                    new_mime_types.push(mime_type.clone());
                }
            }

            let destroyed = self.update_seat_offers(d, seat);
            Some((new_mime_types, destroyed))
        });

        if let Some((new_mime_types, destroyed)) = result {
            for source in destroyed {
                wayland_data_source_destroy(source);
            }
            crate::events::window::send_clipboard_update(false, new_mime_types);
        }
    }

    /// Translation of `Wayland_DataDeviceSetSelectionOffer()`.
    pub(crate) fn wayland_data_device_set_selection_offer(
        &self,
        seat: u32,
        offer: Option<DataOffer>,
    ) {
        let notify = self.with_data(|d| {
            let dd = d.seat_mut(seat)?.data_device.as_mut()?;
            // Don't notify when clearing the old selection offer if doing so will inadvertently clear the selection source.
            let notify = offer.is_some()
                || dd
                    .selection_offer
                    .as_ref()
                    .is_some_and(|o| o.callback.is_none() || dd.selection_source.is_none());
            dd.selection_offer = offer;
            Some(notify)
        });
        if notify == Some(true) {
            self.selection_offer_notify_from_mimes(seat, true);
        }
    }

    /// Receive the data of an offer in a mime type. Translation of
    /// `Wayland_DataOfferReceive()`.
    pub(crate) fn wayland_data_offer_receive(
        &self,
        offer: Option<&DataOffer>,
        mime_type: &str,
        extended_timeout: bool,
    ) -> Result<Option<Vec<u8>>> {
        let timeout = if extended_timeout {
            EXTENDED_PIPE_TIMEOUT_NS
        } else {
            DEFAULT_PIPE_TIMEOUT_NS
        };

        let Some(offer) = offer else {
            return Err(Error::new("Invalid data offer"));
        };

        let mut buffer = None;
        if let Some((read, write)) = pipe2() {
            offer.offer.receive(mime_type, write.as_fd());
            drop(write);

            let _ = self.conn.flush();

            while read_pipe(&read, &mut buffer, timeout) > 0 {}
            drop(read);
        } else {
            return Err(Error::new("Could not create pipe"));
        }

        crate::trace!(
            crate::log::Category::Input,
            ". In Wayland_data_offer_receive for '{}', buffer ({}) at {:?}",
            mime_type,
            buffer.as_ref().map_or(0, Vec::len),
            buffer.as_ref().map(|b| b.as_ptr())
        );

        Ok(buffer)
    }

    /// Receive the primary selection in a mime type. Translation of
    /// `Wayland_PrimarySelectionOfferReceive()`.
    pub(crate) fn wayland_primary_selection_offer_receive(
        &self,
        offer: Option<&PrimarySelectionOffer>,
        mime_type: &str,
    ) -> Result<Option<Vec<u8>>> {
        let Some(offer) = offer else {
            return Err(Error::new("Invalid data offer"));
        };

        let mut buffer = None;
        if let Some((read, write)) = pipe2() {
            offer.offer.receive(mime_type, write.as_fd());
            drop(write);

            let _ = self.conn.flush();

            while read_pipe(&read, &mut buffer, EXTENDED_PIPE_TIMEOUT_NS) > 0 {}
            drop(read);
        } else {
            return Err(Error::new("Could not create pipe"));
        }

        crate::trace!(
            crate::log::Category::Input,
            ". In Wayland_primary_selection_offer_receive for '{}', buffer ({}) at {:?}",
            mime_type,
            buffer.as_ref().map_or(0, Vec::len),
            buffer.as_ref().map(|b| b.as_ptr())
        );

        Ok(buffer)
    }
}

/// `strncmp(a, b, n) == 0` for NUL-terminated strings `a` and `b`.
fn strncmp_eq(a: &[u8], b: &[u8], n: usize) -> bool {
    for i in 0..n {
        let x = a.get(i).copied().unwrap_or(0);
        let y = b.get(i).copied().unwrap_or(0);
        if x != y {
            return false;
        }
        if x == 0 {
            return true;
        }
    }
    true
}

/// Translation of `Wayland_DataSourceDestroy()` (the device no longer
/// refers to the source: callers take it out first).
pub(crate) fn wayland_data_source_destroy(source: DataSource) {
    let DataSource { source, data, .. } = source;
    drop(source);
    let (sequence, callback) = {
        let mut d = source_data(&data);
        (d.sequence, d.callback.take())
    };
    if sequence != 0 {
        crate::video::clipboard::cancel_clipboard_data(sequence);
    }
    // (else the callback owns its data: dropping it frees it)
    drop(callback);
}

/// Translation of `Wayland_DataSourceSetCallback()`.
pub(crate) fn wayland_data_source_set_callback(
    source: &DataSource,
    callback: Option<ClipboardDataCallback>,
    sequence: u32,
) {
    let mut d = source_data(&source.data);
    d.callback = callback;
    d.sequence = sequence;
}

/// Translation of `Wayland_PrimarySelectionSourceSetCallback()`.
pub(crate) fn wayland_primary_selection_source_set_callback(
    source: &PrimarySelectionSource,
    callback: ClipboardDataCallback,
) {
    let mut d = source_data(&source.data);
    d.callback = Some(callback);
    d.sequence = 0;
}

/// Translation of `Wayland_DataOfferAddMIME()` (and
/// `Wayland_PrimarySelectionOfferAddMIME()`): `MIMEDataListAdd()` without
/// data, which inserts at the head of the list.
pub(crate) fn wayland_offer_add_mime(list: &MimeList, mime_type: &str) {
    let mut list = mimes(list);
    if !list.iter().any(|m| m == mime_type) {
        list.insert(0, mime_type.to_owned());
    }
}

/// Translation of `Wayland_DataOfferHasMIME()`.
pub(crate) fn wayland_data_offer_has_mime(offer: Option<&DataOffer>, mime_type: &str) -> bool {
    offer.is_some_and(|o| mimes(&o.mimes).iter().any(|m| m == mime_type))
}

/// Translation of `Wayland_PrimarySelectionOfferHasMIME()`.
pub(crate) fn wayland_primary_selection_offer_has_mime(
    offer: Option<&PrimarySelectionOffer>,
    mime_type: &str,
) -> bool {
    offer.is_some_and(|o| mimes(&o.mimes).iter().any(|m| m == mime_type))
}

/// Offer a source as the clipboard. Translation of
/// `Wayland_DataDeviceSetSelectionSource()`: the old source, if any, comes
/// back to be destroyed (with nothing borrowed).
#[allow(clippy::result_large_err)] // (the sources come back to be destroyed)
pub(crate) fn wayland_data_device_set_selection_source(
    data_device: Option<&mut DataDevice>,
    mut source: DataSource,
    mime_types: &[String],
) -> std::result::Result<Option<DataSource>, (Error, DataSource, Option<DataSource>)> {
    let Some(data_device) = data_device else {
        return Err((Error::new("Invalid Data Device"), source, None));
    };

    if !mime_types.is_empty() {
        for mime_type in mime_types {
            source.source.offer(mime_type);
        }

        // Advertise the data origin MIME
        source.source.offer(SDL_DATA_ORIGIN_MIME);

        // Only set if there is a valid serial if not set it later
        if data_device.selection_serial != 0 {
            data_device
                .data_device
                .set_selection(Some(source.source.obj()), data_device.selection_serial);
        }
        let old = data_device.selection_source.take();
        source.data_device = Some(data_device.seat);
        source_data(&source.data).id_str = Some(data_device.id_str.clone());
        data_device.selection_source = Some(source);
        Ok(old)
    } else {
        let old = data_device.selection_source.take();
        Err((Error::new("No mime data"), source, old))
    }
}

/// Offer a source as the primary selection. Translation of
/// `Wayland_PrimarySelectionDeviceSetSelection()`: the old source comes
/// back.
pub(crate) fn wayland_primary_selection_device_set_selection(
    primary_selection_device: Option<&mut PrimarySelectionDevice>,
    mut source: PrimarySelectionSource,
    mime_types: &[&str],
) -> std::result::Result<Option<PrimarySelectionSource>, (Error, PrimarySelectionSource)> {
    let Some(primary_selection_device) = primary_selection_device else {
        return Err((Error::new("Invalid Primary Selection Device"), source));
    };

    if !mime_types.is_empty() {
        for mime_type in mime_types {
            source.source.offer(mime_type);
        }

        // Only set if there is a valid serial if not set it later
        if primary_selection_device.selection_serial != 0 {
            primary_selection_device
                .primary_selection_device
                .set_selection(
                    Some(source.source.obj()),
                    primary_selection_device.selection_serial,
                );
        }
        let old = primary_selection_device.selection_source.take();
        source.primary_selection_device = Some(primary_selection_device.seat);
        primary_selection_device.selection_source = Some(source);
        Ok(old)
    } else {
        primary_selection_device.selection_source = None;
        Err((Error::new("No mime data"), source))
    }
}

/// Translation of `Wayland_DataDeviceSetSerial()`.
pub(crate) fn wayland_data_device_set_serial(data_device: Option<&mut DataDevice>, serial: u32) {
    if let Some(data_device) = data_device {
        // If there was no serial and there is a pending selection, set it now.
        if data_device.selection_serial == 0 {
            if let Some(source) = &data_device.selection_source {
                data_device
                    .data_device
                    .set_selection(Some(source.source.obj()), serial);
            }
        }

        data_device.selection_serial = serial;
    }
}

/// Translation of `Wayland_PrimarySelectionDeviceSetSerial()`.
pub(crate) fn wayland_primary_selection_device_set_serial(
    primary_selection_device: Option<&mut PrimarySelectionDevice>,
    serial: u32,
) {
    if let Some(device) = primary_selection_device {
        // If there was no serial and there is a pending selection, set it now.
        if device.selection_serial == 0 {
            if let Some(source) = &device.selection_source {
                device
                    .primary_selection_device
                    .set_selection(Some(source.source.obj()), serial);
            }
        }

        device.selection_serial = serial;
    }
}
