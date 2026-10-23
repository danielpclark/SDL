// Rust translation of src/process/dummy/SDL_dummyprocess.c from Simple
// DirectMedia Layer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! The process backend of a platform without processes (Emscripten, and
//! tvOS, where CMake's `posix_spawn()` check fails).

use super::{Process, ProcessBuilder};
use crate::error::{Error, Result};

/// The backend's part of a process (`SDL_ProcessData`): there are none.
#[derive(Debug)]
pub(super) enum ProcessData {}

/// Translation of `SDL_SYS_CreateProcessWithProperties()`.
pub(super) fn sys_create_process(_process: &mut Process, _builder: ProcessBuilder) -> Result<()> {
    Err(Error::unsupported())
}

/// Translation of `SDL_SYS_KillProcess()`.
pub(super) fn sys_kill_process(_process: &mut Process, _force: bool) -> Result<()> {
    Err(Error::unsupported())
}

/// Translation of `SDL_SYS_WaitProcess()`.
pub(super) fn sys_wait_process(_process: &mut Process, _block: bool) -> Result<Option<i32>> {
    Err(Error::unsupported())
}

/// Translation of `SDL_SYS_DestroyProcess()`.
pub(super) fn sys_destroy_process(_process: &mut Process) {}
