// Rust translation of src/video/wayland/SDL_waylandcolor.c and
// SDL_waylandcolor.h from Simple DirectMedia Layer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! The color information (HDR headroom, SDR white level, ICC profile) of
//! windows and outputs, from the color-management-v1 protocol.
//!
//! A request in flight is a [`ColorInfoState`], kept in the window's or the
//! output's data; a new request replaces (cancels) the old one. Its
//! listeners find it by the object it belongs to and its serial.

use std::os::fd::OwnedFd;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;

use super::client::{AsProxy, EventQueue, Proxy};
use super::protocols::color_management_v1::*;
use super::video::{VideoData, WaylandVideo};
use crate::events::{EventType, WindowID};
use crate::video::sysvideo::HdrOutputProperties;

/// What a request is for (`object_type` and its union).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum ColorObject {
    /// `WAYLAND_COLOR_OBJECT_TYPE_WINDOW`
    Window(WindowID),
    /// `WAYLAND_COLOR_OBJECT_TYPE_DISPLAY` (by registry name)
    Display(u32),
}

/// A color information request. Translation of `Wayland_ColorInfoState`;
/// dropping it is `Wayland_FreeColorInfoState()`.
pub(crate) struct ColorInfoState {
    /// Which request this is (the listeners of a replaced one ignore their
    /// events).
    serial: u32,
    wp_image_description: Option<Proxy<WpImageDescriptionV1>>,
    wp_image_description_info: Option<Proxy<WpImageDescriptionInfoV1>>,
    queue: Option<Arc<EventQueue>>,

    hdr: HdrOutputProperties,

    /// The ICC profile (`icc_fd`, only valid if the size is non-zero).
    icc_fd: Option<OwnedFd>,
    icc_size: u32,

    deferred_event_processing: bool,
}

impl std::fmt::Debug for ColorInfoState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ColorInfoState")
            .field("serial", &self.serial)
            .finish()
    }
}

/// The serial of the next request.
static NEXT_SERIAL: AtomicU32 = AtomicU32::new(1);

impl ColorInfoState {
    /// Translation of `Wayland_CancelColorInfoRequest()`.
    fn cancel(&mut self) {
        self.wp_image_description_info = None;
        self.wp_image_description = None;
    }
}

impl Drop for ColorInfoState {
    /// Translation of `Wayland_FreeColorInfoState()` (the proxies go before
    /// their queue).
    fn drop(&mut self) {
        self.cancel();
        self.queue = None;
    }
}

/// The request of `object`, if it is still the one with `serial`.
fn state_of(d: &mut VideoData, object: ColorObject, serial: u32) -> Option<&mut ColorInfoState> {
    let state = match object {
        ColorObject::Window(w) => d.window_mut(w)?.color_info_state.as_mut(),
        ColorObject::Display(key) => d.output_mut(key)?.color_info_state.as_mut(),
    }?;
    (state.serial == serial).then_some(state)
}

/// Drop the request of `object` (if it is still `serial`): upstream's
/// `Wayland_FreeColorInfoState()` on it, which clears the owner's pointer.
fn free_state(d: &mut VideoData, object: ColorObject, serial: u32) -> Option<ColorInfoState> {
    let slot = match object {
        ColorObject::Window(w) => &mut d.window_mut(w)?.color_info_state,
        ColorObject::Display(key) => &mut d.output_mut(key)?.color_info_state,
    };
    if slot.as_ref().is_some_and(|s| s.serial == serial) {
        slot.take()
    } else {
        None
    }
}

impl WaylandVideo {
    /// The image description info listener (`image_description_info_listener`).
    fn handle_image_description_info_event(
        &self,
        object: ColorObject,
        serial: u32,
        event: WpImageDescriptionInfoV1Event,
    ) {
        match event {
            WpImageDescriptionInfoV1Event::Done => {
                self.image_description_info_handle_done(object, serial)
            }
            WpImageDescriptionInfoV1Event::IccFile { icc, icc_size } => {
                // Note (upstream): the ICC profile descriptors are never
                // closed, so one leaks with every profile change (and with
                // every display's profile, which isn't kept). Here the
                // descriptor is owned: a replaced or unused one is closed.
                self.with_data(|d| {
                    if let Some(state) = state_of(d, object, serial) {
                        state.icc_fd = Some(icc);
                        state.icc_size = icc_size;
                    }
                });
            }
            WpImageDescriptionInfoV1Event::Luminances {
                max_lum,
                reference_lum,
                ..
            } => self.with_data(|d| {
                if let Some(state) = state_of(d, object, serial) {
                    state.hdr.hdr_headroom = max_lum as f32 / reference_lum as f32;
                    state.hdr.sdr_white_level = reference_lum as f32 / 80.0;
                }
            }),
            WpImageDescriptionInfoV1Event::Primaries { .. }
            | WpImageDescriptionInfoV1Event::PrimariesNamed { .. }
            | WpImageDescriptionInfoV1Event::TfPower { .. }
            | WpImageDescriptionInfoV1Event::TfNamed { .. }
            | WpImageDescriptionInfoV1Event::TargetPrimaries { .. }
            | WpImageDescriptionInfoV1Event::TargetLuminance { .. }
            | WpImageDescriptionInfoV1Event::TargetMaxCll { .. }
            | WpImageDescriptionInfoV1Event::TargetMaxFall { .. } => {
                // NOP
            }
        }
    }

    /// Translation of `image_description_info_handle_done()`.
    fn image_description_info_handle_done(&self, object: ColorObject, serial: u32) {
        let Some((hdr, icc)) = self.with_data(|d| {
            let state = state_of(d, object, serial)?;
            state.cancel();
            Some((state.hdr, (state.icc_fd.take(), state.icc_size)))
        }) else {
            return;
        };

        match object {
            ColorObject::Window(window) => {
                crate::video::display::set_window_hdr_properties(window, hdr, true);
                if let (Some(icc_fd), icc_size @ 1..) = icc {
                    self.with_data(|d| {
                        if let Some(w) = d.window_mut(window) {
                            // (the window's previous profile is closed)
                            w.icc_fd = Some(icc_fd);
                            w.icc_size = icc_size;
                        }
                    });
                    crate::events::window::send_window_event(
                        window,
                        EventType::WINDOW_ICCPROF_CHANGED,
                        0,
                        0,
                    );
                }
            }
            ColorObject::Display(key) => {
                let display = self.with_data(|d| {
                    let o = d.output_mut(key)?;
                    o.hdr = hdr;
                    if o.display == 0 {
                        o.placeholder.hdr = hdr;
                    }
                    Some(o.display)
                });
                if let Some(display) = display.filter(|&id| id != 0) {
                    crate::video::display::set_display_hdr_properties(display, hdr);
                }
            }
        }
    }

    /// Translation of `PumpColorspaceEvents()`.
    fn pump_colorspace_events(&self, object: ColorObject, serial: u32) {
        // Run the image description sequence to completion in its own queue.
        loop {
            let queue = self.with_data(|d| {
                let state = state_of(d, object, serial)?;
                state.wp_image_description.as_ref()?;
                state.queue.clone()
            });
            let Some(queue) = queue else {
                break;
            };
            if self.conn.dispatch_queue(&queue).is_err() {
                break;
            }
        }

        let state = self.with_data(|d| free_state(d, object, serial));
        drop(state);
    }

    /// The image description listener (`image_description_listener`).
    fn handle_image_description_event(
        &self,
        object: ColorObject,
        serial: u32,
        event: WpImageDescriptionV1Event<'_>,
    ) {
        match event {
            WpImageDescriptionV1Event::Failed { .. } => {
                // Translation of `image_description_handle_failed()`.
                let deferred = self.with_data(|d| {
                    let state = state_of(d, object, serial)?;
                    state.cancel();
                    Some(state.deferred_event_processing)
                });
                if deferred == Some(true) {
                    let state = self.with_data(|d| free_state(d, object, serial));
                    drop(state);
                }
            }
            WpImageDescriptionV1Event::Ready { .. } | WpImageDescriptionV1Event::Ready2 { .. } => {
                self.image_description_handle_ready2(object, serial)
            }
        }
    }

    /// Translation of `image_description_handle_ready2()` (and
    /// `image_description_handle_ready()`).
    fn image_description_handle_ready2(&self, object: ColorObject, serial: u32) {
        let conn = self.conn.clone();
        let deferred = self.with_data(|d| {
            let state = state_of(d, object, serial)?;
            let desc = state.wp_image_description.as_ref()?;

            /* If event processing was deferred, then the image description is on the default queue.
             * Otherwise, it will inherit the queue from the image description object.
             */
            let mut info = if state.deferred_event_processing {
                let queue = conn.create_queue(c"SDL Color Management Queue")?;
                let image_desc_wrapper = desc.obj().create_wrapper(&queue);
                let info = image_desc_wrapper.get_information();
                drop(image_desc_wrapper);
                state.queue = Some(Arc::new(queue));
                info
            } else {
                desc.get_information()
            };
            self.listen(&mut info, move |v, _, ev| {
                v.handle_image_description_info_event(object, serial, ev)
            });
            state.wp_image_description_info = Some(info);
            Some(state.deferred_event_processing)
        });

        if deferred == Some(true) {
            self.pump_colorspace_events(object, serial);
        }
    }

    /// Start a request for the preferred color information of a window.
    /// Translation of `Wayland_GetColorInfoForWindow()`.
    pub(crate) fn wayland_get_color_info_for_window(
        &self,
        window: WindowID,
        defer_event_processing: bool,
    ) {
        let object = ColorObject::Window(window);
        let serial = self
            .with_data(|d| self.wayland_get_color_info_locked(d, object, defer_event_processing));
        if let Some(serial) = serial {
            if !defer_event_processing {
                self.pump_colorspace_events(object, serial);
            }
        }
    }

    /// Start a request for the color information of an output (by registry
    /// name). Translation of `Wayland_GetColorInfoForOutput()`.
    pub(crate) fn wayland_get_color_info_for_output(&self, key: u32, defer_event_processing: bool) {
        let object = ColorObject::Display(key);
        let serial = self
            .with_data(|d| self.wayland_get_color_info_locked(d, object, defer_event_processing));
        if let Some(serial) = serial {
            if !defer_event_processing {
                self.pump_colorspace_events(object, serial);
            }
        }
    }

    /// [`wayland_get_color_info_for_output`](Self::wayland_get_color_info_for_output)
    /// with the data borrowed, for a deferred request (which dispatches
    /// nothing).
    pub(crate) fn wayland_get_color_info_for_output_locked(
        &self,
        d: &mut VideoData,
        key: u32,
        defer_event_processing: bool,
    ) {
        debug_assert!(defer_event_processing);
        self.wayland_get_color_info_locked(d, ColorObject::Display(key), defer_event_processing);
    }

    /// The request part of `Wayland_GetColorInfoForWindow()` and
    /// `Wayland_GetColorInfoForOutput()`: the new request's serial.
    fn wayland_get_color_info_locked(
        &self,
        d: &mut VideoData,
        object: ColorObject,
        defer_event_processing: bool,
    ) -> Option<u32> {
        // Cancel any pending request, as it is out-of-date.
        let old = match object {
            ColorObject::Window(w) => d.window_mut(w)?.color_info_state.take(),
            ColorObject::Display(key) => d.output_mut(key)?.color_info_state.take(),
        };
        drop(old);

        let serial = NEXT_SERIAL.fetch_add(1, Ordering::Relaxed);
        let mut state = ColorInfoState {
            serial,
            wp_image_description: None,
            wp_image_description_info: None,
            queue: None,
            hdr: HdrOutputProperties::default(),
            icc_fd: None,
            icc_size: 0,
            deferred_event_processing: defer_event_processing,
        };

        let mut desc = match object {
            ColorObject::Window(w) => {
                let feedback = d.window(w)?.wp_color_management_surface_feedback.as_ref()?;
                if !defer_event_processing {
                    let queue = self.conn.create_queue(c"SDL Color Management Queue")?;
                    let wrapper = feedback.obj().create_wrapper(&queue);
                    let desc = wrapper.get_preferred();
                    drop(wrapper);
                    state.queue = Some(Arc::new(queue));
                    desc
                } else {
                    feedback.get_preferred()
                }
            }
            ColorObject::Display(key) => {
                let cmo = d.output(key)?.wp_color_management_output.as_ref()?;
                if !defer_event_processing {
                    let queue = self.conn.create_queue(c"SDL Color Management Queue")?;
                    let wrapper = cmo.obj().create_wrapper(&queue);
                    let desc = wrapper.get_image_description();
                    drop(wrapper);
                    state.queue = Some(Arc::new(queue));
                    desc
                } else {
                    cmo.get_image_description()
                }
            }
        };
        self.listen(&mut desc, move |v, _, ev| {
            v.handle_image_description_event(object, serial, ev)
        });
        state.wp_image_description = Some(desc);

        match object {
            ColorObject::Window(w) => d.window_mut(w)?.color_info_state = Some(state),
            ColorObject::Display(key) => d.output_mut(key)?.color_info_state = Some(state),
        }
        Some(serial)
    }
}

#[cfg(test)]
mod tests {
    use std::os::fd::{AsRawFd, FromRawFd};

    use super::super::tests::{Compositor, Video};
    use super::*;
    use crate::events::window::WindowFlags;
    use crate::video::window::Window;

    /// A non-blocking pipe: (read end, write end).
    fn pipe() -> (OwnedFd, OwnedFd) {
        let mut fds = [0; 2];
        // SAFETY: fds holds the two descriptors made.
        assert_eq!(
            unsafe { libc::pipe2(fds.as_mut_ptr(), libc::O_NONBLOCK | libc::O_CLOEXEC) },
            0
        );
        // SAFETY: the new descriptors, owned once each.
        unsafe { (OwnedFd::from_raw_fd(fds[0]), OwnedFd::from_raw_fd(fds[1])) }
    }

    /// Whether every write end of the pipe of `read` is closed.
    fn write_end_closed(read: &OwnedFd) -> bool {
        let mut b = 0u8;
        // SAFETY: a one-byte buffer.
        let n = unsafe { libc::read(read.as_raw_fd(), (&mut b as *mut u8).cast(), 1) };
        n == 0
    }

    /// Run a profile request to the window to its end, as the compositor
    /// would: an `icc_file` event with `icc`, then `done`.
    fn send_profile(video: &WaylandVideo, window: WindowID, icc: OwnedFd) {
        let serial = NEXT_SERIAL.fetch_add(1, Ordering::Relaxed);
        video.with_data(|d| {
            d.window_mut(window).unwrap().color_info_state = Some(ColorInfoState {
                serial,
                wp_image_description: None,
                wp_image_description_info: None,
                queue: None,
                hdr: HdrOutputProperties::default(),
                icc_fd: None,
                icc_size: 0,
                deferred_event_processing: false,
            })
        });
        let object = ColorObject::Window(window);
        video.handle_image_description_info_event(
            object,
            serial,
            WpImageDescriptionInfoV1Event::IccFile { icc, icc_size: 16 },
        );
        video.handle_image_description_info_event(
            object,
            serial,
            WpImageDescriptionInfoV1Event::Done,
        );
        video.with_data(|d| drop(free_state(d, object, serial)));
    }

    #[test]
    fn icc_profile_descriptors_are_closed() {
        let _l = crate::test_support::test_lock();
        let Some(_compositor) = Compositor::start(false) else {
            return;
        };
        let _video = Video::init();
        let video = super::super::util::current_device().unwrap();
        let w = Window::create("ICC test", 64, 48, WindowFlags::HIDDEN).unwrap();
        let id = w.id();

        let (first, first_icc) = pipe();
        send_profile(&video, id, first_icc);
        assert!(!write_end_closed(&first), "the window keeps its profile");

        // A new profile closes the old one.
        let (second, second_icc) = pipe();
        send_profile(&video, id, second_icc);
        assert!(write_end_closed(&first), "the replaced profile is closed");
        assert!(!write_end_closed(&second));

        // A profile request to nothing (here a window that's gone) closes
        // its descriptor too.
        let (third, third_icc) = pipe();
        video.handle_image_description_info_event(
            ColorObject::Window(id + 1000),
            0,
            WpImageDescriptionInfoV1Event::IccFile {
                icc: third_icc,
                icc_size: 16,
            },
        );
        assert!(write_end_closed(&third));

        // The window's profile is closed with it.
        w.destroy();
        assert!(write_end_closed(&second), "the window's profile is closed");
    }
}
