// Rust translation of include/dav1d/common.h, include/dav1d/data.h and
// src/data.c from dav1d (https://code.videolan.org/videolan/dav1d, at the
// revision SDL_image's external/dav1d pins).
// Copyright © 2018, VideoLAN and dav1d authors
// Copyright © 2018, Two Orioles, LLC
// SPDX-License-Identifier: BSD-2-Clause (see LICENSE.txt)
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! Input data buffers and their metadata.
//!
//! The reference counting of `Dav1dRef` is `Arc`: a `Dav1dData` shares its
//! bytes with the tile data the decoder keeps, and the free callback of
//! `dav1d_data_wrap()` is the drop of the last `Arc`.

use std::any::Any;
use std::fmt;
use std::sync::Arc;

/// Translation of `Dav1dDataProps`: input packet metadata which are
/// copied from the input data used to decode each image into the matching
/// structure of the output image returned back to the user. Since these
/// are metadata fields, they can be used for other purposes than the
/// documented ones, they will still be passed from input data to output
/// picture without being used internally.
#[derive(Clone)]
pub(crate) struct Dav1dDataProps {
    /// container timestamp of input data, `i64::MIN` if unknown (default)
    pub(crate) timestamp: i64,
    /// container duration of input data, 0 if unknown (default)
    pub(crate) duration: i64,
    /// stream offset of input data, -1 if unknown (default)
    pub(crate) offset: i64,
    /// packet size, default `Dav1dData.sz`
    pub(crate) size: usize,
    /// user-configurable data, default `None` (`Dav1dUserData`)
    pub(crate) user_data: Option<Arc<dyn Any + Send + Sync>>,
}

impl fmt::Debug for Dav1dDataProps {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Dav1dDataProps")
            .field("timestamp", &self.timestamp)
            .field("duration", &self.duration)
            .field("offset", &self.offset)
            .field("size", &self.size)
            .field("user_data", &self.user_data.is_some())
            .finish()
    }
}

impl Default for Dav1dDataProps {
    /// Translation of `dav1d_data_props_set_defaults()`.
    fn default() -> Self {
        Dav1dDataProps {
            timestamp: i64::MIN,
            duration: 0,
            offset: -1,
            size: 0,
            user_data: None,
        }
    }
}

/// Translation of `Dav1dData`: `buf[offset..offset + sz]` are the bytes
/// (`data`, `sz`), `buf` the allocation origin (`ref`).
#[derive(Clone, Debug, Default)]
pub(crate) struct Dav1dData {
    pub(crate) buf: Option<Arc<[u8]>>,
    pub(crate) offset: usize,
    pub(crate) sz: usize,
    /// user provided metadata passed to the output picture
    pub(crate) m: Dav1dDataProps,
}

impl Dav1dData {
    /// Translation of `dav1d_data_wrap()`: wrap an existing data array
    /// (its last reference going away is the free callback).
    pub(crate) fn wrap(buf: Arc<[u8]>) -> Dav1dData {
        let sz = buf.len();
        Dav1dData {
            buf: Some(buf),
            offset: 0,
            sz,
            m: Dav1dDataProps {
                size: sz,
                ..Dav1dDataProps::default()
            },
        }
    }

    /// Translation of `dav1d_data_create()` followed by a copy of `data`
    /// into the new buffer.
    pub(crate) fn from_slice(data: &[u8]) -> Dav1dData {
        Dav1dData::wrap(Arc::from(data))
    }

    /// Translation of `dav1d_data_wrap_user_data()`.
    pub(crate) fn wrap_user_data(&mut self, user_data: Arc<dyn Any + Send + Sync>) {
        self.m.user_data = Some(user_data);
    }

    /// The `data` pointer: the bytes not consumed yet.
    pub(crate) fn data(&self) -> &[u8] {
        match &self.buf {
            Some(b) => &b[self.offset..self.offset + self.sz],
            None => &[],
        }
    }

    /// Whether `data` is non-`NULL`.
    pub(crate) fn has_data(&self) -> bool {
        self.buf.is_some()
    }

    /// Translation of `dav1d_data_unref()` (`dav1d_data_unref_internal()`).
    pub(crate) fn unref(&mut self) {
        *self = Dav1dData::default();
    }
}

/// Translation of `dav1d_data_props_unref()`.
pub(crate) fn data_props_unref(props: &mut Dav1dDataProps) {
    *props = Dav1dDataProps::default();
}
