// Rust translation of src/io from Simple DirectMedia Layer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! Byte streams: [`IoStream`] over files, memory and custom
//! [`IoInterface`] backends, and asynchronous file I/O with [`AsyncIo`].
//! Translation of `SDL_iostream.h` and `SDL_asyncio.h`.

mod asyncio;
mod iostream;

pub(crate) use asyncio::quit_async_io;
pub use asyncio::{
    load_file_async, AsyncIo, AsyncIoOutcome, AsyncIoQueue, AsyncIoResult, AsyncIoTaskType,
};

pub(crate) use iostream::strerror;
pub use iostream::{
    load_file, save_file, IoInterface, IoStatus, IoStop, IoStream, IoWhence,
    PROP_IOSTREAM_DYNAMIC_CHUNKSIZE_NUMBER, PROP_IOSTREAM_FILE_DESCRIPTOR_NUMBER,
    PROP_IOSTREAM_MEMORY_SIZE_NUMBER,
};
