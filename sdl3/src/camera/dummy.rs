// Rust translation of src/camera/dummy/SDL_camera_dummy.c from Simple
// DirectMedia Layer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! The dummy camera driver: no cameras (and opening one is unsupported).

use std::sync::Arc;

use super::{CameraBackend, CameraBootStrap, CameraDevice, CameraDriverImpl, CameraSpec};
use crate::error::{Error, Result};

struct DummyCamera;

impl CameraDriverImpl for DummyCamera {
    fn detect_devices(&self) {}

    fn open_device(
        &self,
        _device: &Arc<CameraDevice>,
        _spec: &CameraSpec,
    ) -> Result<Arc<dyn CameraBackend>> {
        Err(Error::unsupported())
    }
}

/// Translation of `DUMMYCAMERA_Init()`.
fn dummycamera_init() -> Result<Arc<dyn CameraDriverImpl>> {
    Ok(Arc::new(DummyCamera))
}

/// Translation of `DUMMYCAMERA_bootstrap`.
pub(super) static DUMMYCAMERA_BOOTSTRAP: CameraBootStrap = CameraBootStrap {
    name: "dummy",
    desc: "SDL dummy camera driver",
    init: dummycamera_init,
    demand_only: true,
};
