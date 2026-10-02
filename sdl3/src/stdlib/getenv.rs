// Rust translation of src/stdlib/SDL_getenv.c from Simple DirectMedia Layer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! Environment variables: thread-safe [`Environment`] snapshots and the
//! process environment.
//!
//! As in C SDL, [`getenv`] reads a snapshot of the process environment taken
//! the first time it is needed; [`setenv_unsafe`] and [`unsetenv_unsafe`]
//! change both the snapshot and the real process environment. Changes made
//! behind SDL's back (e.g. with `std::env::set_var`) are not seen until the
//! snapshot is refreshed by [`init::quit`](crate::init::quit).

use std::collections::HashMap;
use std::sync::{Arc, Mutex, MutexGuard};

use crate::error::{Error, Result};

#[derive(Debug, Default)]
struct EnvInner {
    strings: HashMap<String, String>,
}

/// A thread-safe set of environment variables. Translation of `SDL_Environment`.
///
/// Cloning gives another handle to the same set.
#[derive(Clone, Debug)]
pub struct Environment {
    inner: Arc<Mutex<EnvInner>>,
}

/// Translation of `SDL_environment`.
static SDL_ENVIRONMENT: Mutex<Option<Environment>> = Mutex::new(None);

impl Environment {
    fn lock(&self) -> MutexGuard<'_, EnvInner> {
        self.inner.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Create a set of environment variables, optionally initialized from the
    /// process environment. Translation of `SDL_CreateEnvironment()`.
    ///
    /// Names on Windows are upper-cased, since the system table is
    /// case-insensitive. Values that are not valid UTF-8 are converted lossily.
    pub fn new(populated: bool) -> Environment {
        let mut strings = HashMap::new();
        if populated {
            for (name, value) in std::env::vars_os() {
                let mut variable = name.to_string_lossy().into_owned();
                if variable.is_empty() {
                    continue;
                }
                if cfg!(windows) {
                    // uppercase ASCII chars in environment variable names on Windows, since the system environment table is case-insensitive.
                    variable.make_ascii_uppercase();
                }
                strings.insert(variable, value.to_string_lossy().into_owned());
            }
        }
        Environment {
            inner: Arc::new(Mutex::new(EnvInner { strings })),
        }
    }

    /// The process environment as SDL sees it (created, populated, on first use).
    /// Translation of `SDL_GetEnvironment()`.
    pub fn process() -> Environment {
        let mut env = SDL_ENVIRONMENT.lock().unwrap_or_else(|e| e.into_inner());
        env.get_or_insert_with(|| Environment::new(true)).clone()
    }

    /// The value of a variable. Translation of `SDL_GetEnvironmentVariable()`.
    pub fn get(&self, name: &str) -> Option<String> {
        if name.is_empty() {
            return None;
        }
        self.lock().strings.get(name).cloned()
    }

    /// Every variable, as `"NAME=value"` strings. Translation of `SDL_GetEnvironmentVariables()`.
    pub fn variables(&self) -> Vec<String> {
        self.lock()
            .strings
            .iter()
            .map(|(k, v)| format!("{k}={v}"))
            .collect()
    }

    /// Set a variable; with `overwrite == false` an existing value is kept
    /// (and that still counts as success). Translation of `SDL_SetEnvironmentVariable()`.
    pub fn set(&self, name: &str, value: &str, overwrite: bool) -> Result<()> {
        if name.is_empty() || name.contains('=') {
            return Err(Error::invalid_param("name"));
        }
        let mut inner = self.lock();
        if overwrite || !inner.strings.contains_key(name) {
            inner.strings.insert(name.to_owned(), value.to_owned());
        }
        // (otherwise: it already existed, and we refused to overwrite it. Call it success.)
        Ok(())
    }

    /// Remove a variable (removing an unset one succeeds).
    /// Translation of `SDL_UnsetEnvironmentVariable()`.
    pub fn unset(&self, name: &str) -> Result<()> {
        if name.is_empty() || name.contains('=') {
            return Err(Error::invalid_param("name"));
        }
        self.lock().strings.remove(name);
        Ok(())
    }
}

/// Translation of `SDL_InitEnvironment()`.
pub(crate) fn init_environment() {
    let _ = Environment::process();
}

/// Translation of `SDL_QuitEnvironment()`: drop the snapshot so the next use
/// re-reads the process environment.
pub(crate) fn quit_environment() {
    SDL_ENVIRONMENT
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .take();
}

/// The value of an environment variable, from SDL's snapshot of the process
/// environment. Translation of `SDL_getenv()`.
pub fn getenv(name: &str) -> Option<String> {
    Environment::process().get(name)
}

/// The value of an environment variable, read directly from the process
/// environment. Translation of `SDL_getenv_unsafe()`.
pub fn getenv_unsafe(name: &str) -> Option<String> {
    if name.is_empty() {
        return None;
    }
    std::env::var_os(name).map(|v| v.to_string_lossy().into_owned())
}

/// Validation shared by the `*_unsafe` setters (upstream returns -1).
fn check_name(name: &str) -> Result<()> {
    if name.is_empty() || name.contains('=') || name.contains('\0') {
        Err(Error::invalid_param("name"))
    } else {
        Ok(())
    }
}

/// Set a variable in SDL's snapshot and in the process environment.
/// Translation of `SDL_setenv_unsafe()`.
///
/// "Unsafe" as in C: changing the process environment while another thread
/// reads it is a data race in the C library. Prefer an [`Environment`] (for
/// example to pass to a child process) when you can.
pub fn setenv_unsafe(name: &str, value: &str, overwrite: bool) -> Result<()> {
    check_name(name)?;
    if value.contains('\0') {
        return Err(Error::invalid_param("value"));
    }
    if let Some(env) = SDL_ENVIRONMENT
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .as_ref()
    {
        env.set(name, value, overwrite)?;
    }
    if !overwrite && std::env::var_os(name).is_some() {
        return Ok(()); // leave the existing one there.
    }
    std::env::set_var(name, value);
    Ok(())
}

/// Remove a variable from SDL's snapshot and from the process environment.
/// Translation of `SDL_unsetenv_unsafe()`. See [`setenv_unsafe`] about safety.
pub fn unsetenv_unsafe(name: &str) -> Result<()> {
    check_name(name)?;
    if let Some(env) = SDL_ENVIRONMENT
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .as_ref()
    {
        env.unset(name)?;
    }
    std::env::remove_var(name);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn environment_objects() {
        let env = Environment::new(false);
        assert_eq!(env.get("A"), None);
        env.set("A", "1", true).unwrap();
        env.set("A", "2", false).unwrap();
        assert_eq!(env.get("A").as_deref(), Some("1"));
        env.set("A", "3", true).unwrap();
        assert_eq!(env.get("A").as_deref(), Some("3"));
        assert_eq!(env.variables(), vec!["A=3".to_owned()]);
        assert!(env.set("", "x", true).is_err());
        assert!(env.set("B=C", "x", true).is_err());
        env.unset("A").unwrap();
        env.unset("A").unwrap();
        assert!(env.variables().is_empty());
        assert_eq!(env.get(""), None);

        // Handles share state.
        let other = env.clone();
        other.set("Z", "z", true).unwrap();
        assert_eq!(env.get("Z").as_deref(), Some("z"));
    }

    #[test]
    fn process_environment_snapshot() {
        let _l = crate::test_support::TEST_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let name = "SDL3_RS_TEST_GETENV";
        setenv_unsafe(name, "one", true).unwrap();
        assert_eq!(getenv(name).as_deref(), Some("one"));
        assert_eq!(getenv_unsafe(name).as_deref(), Some("one"));
        setenv_unsafe(name, "two", false).unwrap();
        assert_eq!(getenv(name).as_deref(), Some("one"));
        assert_eq!(getenv_unsafe(name).as_deref(), Some("one"));

        // Changes behind SDL's back are invisible until the snapshot is refreshed.
        std::env::set_var(name, "three");
        assert_eq!(getenv(name).as_deref(), Some("one"));
        quit_environment();
        assert_eq!(getenv(name).as_deref(), Some("three"));

        unsetenv_unsafe(name).unwrap();
        assert_eq!(getenv(name), None);
        assert_eq!(getenv_unsafe(name), None);
        assert!(setenv_unsafe("A=B", "x", true).is_err());
        assert!(Environment::new(true)
            .variables()
            .iter()
            .all(|v| v.contains('=')));
    }
}
