// Rust translation of src/SDL_error.c from Simple DirectMedia Layer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! Error type used throughout the crate.
//!
//! C SDL reports failure with a `false`/`NULL` return and a per-thread message
//! retrievable through `SDL_GetError()`. In Rust the message travels with the
//! failure instead: fallible functions return [`Result<T>`] and the
//! [`Error`] carries the same text SDL would have produced, plus a coarse
//! [`ErrorKind`] for the handful of errors SDL distinguishes
//! (`SDL_InvalidParamError`, `SDL_Unsupported`, `SDL_OutOfMemory`).

use std::borrow::Cow;
use std::fmt;

/// Coarse classification of an [`Error`].
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
#[non_exhaustive]
pub enum ErrorKind {
    /// A parameter was out of range or otherwise invalid (`SDL_InvalidParamError`).
    InvalidParam,
    /// The operation is not supported on this platform/backend (`SDL_Unsupported`).
    Unsupported,
    /// Memory allocation failed (`SDL_OutOfMemory`).
    OutOfMemory,
    /// Any other failure (`SDL_SetError` with a free-form message).
    Other,
}

/// An SDL error: a human-readable message and an [`ErrorKind`].
///
/// The message text matches what C SDL puts in `SDL_GetError()` for the same
/// failure, so existing knowledge of SDL's error strings carries over.
#[derive(Clone, PartialEq, Eq, Hash)]
pub struct Error {
    kind: ErrorKind,
    message: Cow<'static, str>,
}

impl Error {
    /// A free-form error. Translation of `SDL_SetError(fmt, ...)`.
    pub fn new(message: impl Into<Cow<'static, str>>) -> Self {
        Error {
            kind: ErrorKind::Other,
            message: message.into(),
        }
    }

    /// Translation of `SDL_InvalidParamError(param)`.
    pub fn invalid_param(param: &str) -> Self {
        Error {
            kind: ErrorKind::InvalidParam,
            message: format!("Parameter '{param}' is invalid").into(),
        }
    }

    /// Translation of `SDL_Unsupported()`.
    pub fn unsupported() -> Self {
        Error {
            kind: ErrorKind::Unsupported,
            message: "That operation is not supported".into(),
        }
    }

    /// Translation of `SDL_OutOfMemory()`.
    pub fn out_of_memory() -> Self {
        Error {
            kind: ErrorKind::OutOfMemory,
            message: "Out of memory".into(),
        }
    }

    /// The error's classification.
    pub fn kind(&self) -> ErrorKind {
        self.kind
    }

    /// The message text, as `SDL_GetError()` would return it.
    pub fn message(&self) -> &str {
        &self.message
    }
}

impl fmt::Debug for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Error")
            .field("kind", &self.kind)
            .field("message", &self.message)
            .finish()
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for Error {}

impl From<String> for Error {
    fn from(s: String) -> Self {
        Error::new(s)
    }
}

impl From<&'static str> for Error {
    fn from(s: &'static str) -> Self {
        Error::new(s)
    }
}

/// `Result` specialised to SDL's [`Error`].
pub type Result<T, E = Error> = std::result::Result<T, E>;

/// Build an [`Error`] with `format!` syntax. Translation of `SDL_SetError(fmt, ...)`.
///
/// ```
/// # use sdl3::{err, Error};
/// let e: Error = err!("Month out of range [1-12], requested: {}", 13);
/// assert_eq!(e.message(), "Month out of range [1-12], requested: 13");
/// ```
#[macro_export]
macro_rules! err {
    ($($arg:tt)*) => { $crate::Error::new(::std::format!($($arg)*)) };
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kinds_and_messages() {
        let e = Error::invalid_param("x");
        assert_eq!(e.kind(), ErrorKind::InvalidParam);
        assert_eq!(e.to_string(), "Parameter 'x' is invalid");
        assert_eq!(
            Error::unsupported().message(),
            "That operation is not supported"
        );
        assert_eq!(Error::out_of_memory().kind(), ErrorKind::OutOfMemory);
        let e = err!("value {}", 42);
        assert_eq!(e.kind(), ErrorKind::Other);
        assert_eq!(e.message(), "value 42");
        let e: Error = "static".into();
        assert_eq!(
            format!("{e:?}"),
            "Error { kind: Other, message: \"static\" }"
        );
    }
}
