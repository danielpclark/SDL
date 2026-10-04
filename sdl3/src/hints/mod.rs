// Rust translation of src/SDL_hints.c from Simple DirectMedia Layer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! Configuration hints.
//!
//! Hints are named string settings that let an application (or a user, via
//! environment variables of the same name) tell the library how it would like
//! things to work. They may or may not be supported or applicable on any
//! given platform; see the constants in this module (`hints::VIDEO_DRIVER`
//! etc.) for the full list and the upstream header for their meaning.
//!
//! Direct translation of `SDL_hints.c`. Upstream stores hints as pointer
//! properties inside an `SDL_PropertiesID`; the translation keeps the same
//! data (value, priority, watcher list) in a dedicated map under the same
//! kind of recursive lock, which is observably identical.

use std::cell::RefCell;
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use crate::err;
use crate::error::{Error, Result};
use crate::thread::ReentrantMutex;

mod names;
pub use names::*;

/// Hint priorities. Translation of `SDL_HintPriority`.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug, Default)]
pub enum Priority {
    #[default]
    Default,
    Normal,
    Override,
}

/// What a hint watcher is told. Translation of the `SDL_HintCallback` arguments.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct HintChange<'a> {
    pub name: &'a str,
    pub old_value: Option<&'a str>,
    pub new_value: Option<&'a str>,
}

type WatchFn = dyn Fn(&HintChange<'_>) + Send + Sync + 'static;

/// Translation of `SDL_HintWatch`.
#[derive(Clone)]
struct HintWatch {
    id: u64,
    callback: Arc<WatchFn>,
}

/// Translation of `SDL_Hint`.
struct Hint {
    value: Option<String>,
    priority: Priority,
    callbacks: Vec<HintWatch>,
}

type HintMap = HashMap<String, Hint>;

/// Translation of `SDL_hint_props` and its lock.
static HINTS: ReentrantMutex<RefCell<Option<HintMap>>> = ReentrantMutex::new(RefCell::new(None));
static NEXT_WATCH_ID: AtomicU64 = AtomicU64::new(1);

/// Translation of `SDL_QuitHints()`.
pub(crate) fn quit_hints() {
    let guard = HINTS.lock();
    *guard.borrow_mut() = None;
}

/// Translation of `GetHintEnvironmentVariable()`.
fn environment_variable(name: &str) -> Option<String> {
    let mut result = crate::stdlib::getenv(name);
    if result.is_none() && !name.is_empty() {
        // fall back to old (SDL2) names of environment variables that
        // are important to users (e.g. many use SDL_VIDEODRIVER=wayland)
        if name == VIDEO_DRIVER {
            result = crate::stdlib::getenv("SDL_VIDEODRIVER");
        } else if name == AUDIO_DRIVER {
            result = crate::stdlib::getenv("SDL_AUDIODRIVER");
        }
    }
    result
}

fn notify(callbacks: &[HintWatch], name: &str, old_value: Option<&str>, new_value: Option<&str>) {
    let change = HintChange {
        name,
        old_value,
        new_value,
    };
    for entry in callbacks {
        (entry.callback)(&change);
    }
}

/// Set a hint with a specific priority.
///
/// The priority controls what happens when the hint already has a value:
/// hints replace existing hints of their priority and lower. Environment
/// variables are considered to have override priority.
///
/// Returns `Ok(true)` if the hint was set, `Ok(false)` if an existing hint
/// of higher priority was left in place, and an error for an empty name or
/// when an environment variable is taking priority.
/// Translation of `SDL_SetHintWithPriority()`.
pub fn set_with_priority(name: &str, value: Option<&str>, priority: Priority) -> Result<bool> {
    if name.is_empty() {
        return Err(Error::invalid_param("name"));
    }

    let env = environment_variable(name);
    if env.is_some() && priority < Priority::Override {
        return Err(err!("An environment variable is taking priority"));
    }

    let guard = HINTS.lock();

    // Phase 1: update storage, collecting watchers to notify.
    let (result, to_notify) = {
        let mut cell = guard.borrow_mut();
        let hints = cell.get_or_insert_with(HashMap::new);
        match hints.get_mut(name) {
            Some(hint) => {
                if priority >= hint.priority {
                    let mut notify = None;
                    if hint.value.as_deref() != value {
                        let old_value = hint.value.take();
                        hint.value = value.map(str::to_owned);
                        notify = Some((hint.callbacks.clone(), old_value));
                    }
                    hint.priority = priority;
                    (true, notify)
                } else {
                    (false, None)
                }
            }
            None => {
                // Couldn't find the hint? Add a new one.
                hints.insert(
                    name.to_owned(),
                    Hint {
                        value: value.map(str::to_owned),
                        priority,
                        callbacks: Vec::new(),
                    },
                );
                (true, None)
            }
        }
    };

    // Phase 2: notify with the RefCell borrow released (watchers may re-enter;
    // the recursive lock stays held, exactly as upstream).
    if let Some((callbacks, old_value)) = to_notify {
        notify(&callbacks, name, old_value.as_deref(), value);
    }
    Ok(result)
}

/// Set a hint with normal priority. Translation of `SDL_SetHint()`.
///
/// Returns `Ok(false)` if an override-priority hint already exists, and an
/// error if an environment variable takes precedence; use
/// [`set_with_priority`] with [`Priority::Override`] to force it.
pub fn set(name: &str, value: &str) -> Result<bool> {
    set_with_priority(name, Some(value), Priority::Normal)
}

/// Shared body of `SDL_ResetHint()` / `ResetHintsCallback()`. Caller holds the lock.
fn reset_one(
    guard: &crate::thread::ReentrantMutexGuard<'_, RefCell<Option<HintMap>>>,
    name: &str,
) -> bool {
    let env = environment_variable(name);
    let to_notify = {
        let mut cell = guard.borrow_mut();
        let Some(hints) = cell.as_mut() else {
            return false;
        };
        let Some(hint) = hints.get_mut(name) else {
            return false;
        };
        let changed = env.as_deref() != hint.value.as_deref();
        let old_value = hint.value.take();
        hint.priority = Priority::Default;
        changed.then(|| (hint.callbacks.clone(), old_value))
    };
    if let Some((callbacks, old_value)) = to_notify {
        notify(&callbacks, name, old_value.as_deref(), env.as_deref());
    }
    true
}

/// Reset a hint to its default: the environment variable's value, or unset.
/// Watchers are notified normally. Returns whether the hint existed.
/// Translation of `SDL_ResetHint()`.
pub fn reset(name: &str) -> bool {
    if name.is_empty() {
        return false;
    }
    let guard = HINTS.lock();
    reset_one(&guard, name)
}

/// Reset all hints to their defaults. Translation of `SDL_ResetHints()`.
pub fn reset_all() {
    let guard = HINTS.lock();
    let names: Vec<String> = match guard.borrow().as_ref() {
        Some(hints) => hints.keys().cloned().collect(),
        None => return,
    };
    for name in names {
        reset_one(&guard, &name);
    }
}

/// The value of a hint, from the environment or a previous [`set`].
/// Translation of `SDL_GetHint()`.
pub fn get(name: &str) -> Option<String> {
    let mut result = environment_variable(name);
    let guard = HINTS.lock();
    if let Some(hints) = guard.borrow().as_ref() {
        if let Some(hint) = hints.get(name) {
            if result.is_none() || hint.priority == Priority::Override {
                result = hint.value.clone();
            }
        }
    }
    result
}

/// Interpret a hint-style string as an integer: `"true"`/`"false"` are 1/0,
/// strings starting with a digit or `-` parse like `atoi`, anything else
/// (including empty/absent) is `default`. Translation of `SDL_GetStringInteger()`.
pub fn string_to_integer(value: Option<&str>, default: i32) -> i32 {
    let Some(value) = value else { return default };
    if value.is_empty() {
        return default;
    }
    if value.eq_ignore_ascii_case("false") {
        return 0;
    }
    if value.eq_ignore_ascii_case("true") {
        return 1;
    }
    let first = value.as_bytes()[0];
    if first == b'-' || first.is_ascii_digit() {
        return crate::stdlib::atoi(value);
    }
    default
}

/// Interpret a hint-style string as a boolean: anything other than a string
/// starting with `'0'` or equal to `"false"` is true; empty/absent is
/// `default`. Translation of `SDL_GetStringBoolean()`.
pub fn string_to_bool(value: Option<&str>, default: bool) -> bool {
    let Some(value) = value else { return default };
    if value.is_empty() {
        return default;
    }
    !(value.as_bytes()[0] == b'0' || value.eq_ignore_ascii_case("false"))
}

/// The boolean value of a hint. Translation of `SDL_GetHintBoolean()`.
pub fn get_bool(name: &str, default: bool) -> bool {
    string_to_bool(get(name).as_deref(), default)
}

/// The integer value of a hint (see [`string_to_integer`]).
pub fn get_integer(name: &str, default: i32) -> i32 {
    string_to_integer(get(name).as_deref(), default)
}

/// A registered hint watcher. Translation of the (callback, userdata) pair
/// that identifies a watcher to `SDL_RemoveHintCallback()`.
///
/// Dropping it removes the watcher. Use [`Callback::detach`] to keep watching
/// for the life of the process.
#[must_use = "dropping a hint Callback unregisters it; call .detach() to keep it"]
#[derive(Debug)]
pub struct Callback {
    name: String,
    id: u64,
}

impl Callback {
    /// Stop watching. Translation of `SDL_RemoveHintCallback()`.
    pub fn remove(self) {
        drop(self);
    }

    /// Keep watching for the life of the process.
    pub fn detach(self) {
        std::mem::forget(self);
    }

    /// Internal: convert into a value whose drop still removes the watcher,
    /// for subsystems that store their watcher in a static.
    pub(crate) fn detach_guard(self) -> Callback {
        self
    }
}

impl Drop for Callback {
    fn drop(&mut self) {
        let guard = HINTS.lock();
        let mut cell = guard.borrow_mut();
        if let Some(hint) = cell.as_mut().and_then(|h| h.get_mut(&self.name)) {
            hint.callbacks.retain(|w| w.id != self.id);
        }
    }
}

/// Watch a hint for changes.
///
/// `callback` is called *during* this function with the hint's current value
/// (as both old and new), and again each time the value changes. It runs on
/// whichever thread changes the hint, while the hint lock is held; it may
/// call back into this module.
/// Translation of `SDL_AddHintCallback()`.
pub fn watch(
    name: &str,
    callback: impl Fn(&HintChange<'_>) + Send + Sync + 'static,
) -> Result<Callback> {
    if name.is_empty() {
        return Err(Error::invalid_param("name"));
    }

    let callback: Arc<WatchFn> = Arc::new(callback);
    let id = NEXT_WATCH_ID.fetch_add(1, Ordering::Relaxed);

    let guard = HINTS.lock();
    {
        let mut cell = guard.borrow_mut();
        let hints = cell.get_or_insert_with(HashMap::new);
        let hint = hints.entry(name.to_owned()).or_insert_with(|| Hint {
            value: None,
            priority: Priority::Default,
            callbacks: Vec::new(),
        });
        // Add it to the callbacks for this hint (upstream prepends)
        hint.callbacks.insert(
            0,
            HintWatch {
                id,
                callback: callback.clone(),
            },
        );
    }

    // Now call it with the current value
    let value = get(name);
    callback(&HintChange {
        name,
        old_value: value.as_deref(),
        new_value: value.as_deref(),
    });

    Ok(Callback {
        name: name.to_owned(),
        id,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicUsize;
    use std::sync::Mutex;

    // Tests share global state; serialize them.

    #[test]
    fn set_get_priority() {
        let _l = crate::test_support::test_lock();
        let name = "SDL_TEST_HINT_A";
        assert_eq!(set(name, "1"), Ok(true));
        assert_eq!(get(name).as_deref(), Some("1"));
        assert_eq!(
            set_with_priority(name, Some("2"), Priority::Override),
            Ok(true)
        );
        assert_eq!(set(name, "3"), Ok(false)); // lower priority can't replace
        assert_eq!(get(name).as_deref(), Some("2"));
        assert!(reset(name));
        assert_eq!(get(name), None);
        assert!(!reset("SDL_TEST_HINT_NEVER_SET"));
        assert_eq!(set(name, "x"), Ok(true));
        assert!(get_bool(name, false));
        assert_eq!(get_integer(name, 7), 7);
        assert_eq!(
            set("", "x").unwrap_err().kind(),
            crate::ErrorKind::InvalidParam
        );
    }

    #[test]
    fn string_helpers() {
        assert!(string_to_bool(None, true));
        assert!(!string_to_bool(Some(""), false));
        assert!(!string_to_bool(Some("0"), true));
        assert!(!string_to_bool(Some("FALSE"), true));
        assert!(string_to_bool(Some("anything"), false));
        assert_eq!(string_to_integer(None, 7), 7);
        assert_eq!(string_to_integer(Some("true"), 7), 1);
        assert_eq!(string_to_integer(Some("-12"), 7), -12);
        assert_eq!(string_to_integer(Some("abc"), 7), 7);
    }

    #[test]
    fn callbacks() {
        let _l = crate::test_support::test_lock();
        let name = "SDL_TEST_HINT_B";
        reset(name);
        let calls = Arc::new(AtomicUsize::new(0));
        let last = Arc::new(Mutex::new(None::<(String, Option<String>, Option<String>)>));
        let (c, l) = (calls.clone(), last.clone());
        let cb = watch(name, move |ch| {
            c.fetch_add(1, Ordering::SeqCst);
            *l.lock().unwrap() = Some((
                ch.name.to_owned(),
                ch.old_value.map(str::to_owned),
                ch.new_value.map(str::to_owned),
            ));
        })
        .unwrap();
        assert_eq!(calls.load(Ordering::SeqCst), 1); // initial call
        set(name, "v1").unwrap();
        assert_eq!(calls.load(Ordering::SeqCst), 2);
        assert_eq!(
            last.lock().unwrap().clone().unwrap(),
            (name.to_owned(), None, Some("v1".to_owned()))
        );
        set(name, "v1").unwrap(); // unchanged: no callback
        assert_eq!(calls.load(Ordering::SeqCst), 2);
        reset(name);
        assert_eq!(calls.load(Ordering::SeqCst), 3);
        cb.remove();
        set(name, "v2").unwrap();
        assert_eq!(calls.load(Ordering::SeqCst), 3);

        // Re-entrancy: a watcher that reads and sets hints.
        let c = "SDL_TEST_HINT_C";
        let cb = watch(c, |ch| {
            let _ = get(ch.name);
            if ch.new_value == Some("trigger") {
                set("SDL_TEST_HINT_C_OTHER", "set-from-callback").unwrap();
            }
        })
        .unwrap();
        set(c, "trigger").unwrap();
        assert_eq!(
            get("SDL_TEST_HINT_C_OTHER").as_deref(),
            Some("set-from-callback")
        );
        drop(cb);

        // Dropping the token unregisters; detach keeps it.
        let d = "SDL_TEST_HINT_D";
        let n = Arc::new(AtomicUsize::new(0));
        let n2 = n.clone();
        watch(d, move |_| {
            n2.fetch_add(1, Ordering::SeqCst);
        })
        .unwrap()
        .detach();
        set(d, "1").unwrap();
        assert_eq!(n.load(Ordering::SeqCst), 2);
        reset_all();
        assert_eq!(get(d), None);
        assert_eq!(n.load(Ordering::SeqCst), 3);
    }

    #[test]
    fn environment_takes_priority() {
        let _l = crate::test_support::test_lock();
        let name = "SDL_TEST_HINT_ENV";
        crate::stdlib::setenv_unsafe(name, "from-env", true).unwrap();
        assert!(set(name, "x").is_err());
        assert_eq!(get(name).as_deref(), Some("from-env"));
        assert_eq!(
            set_with_priority(name, Some("over"), Priority::Override),
            Ok(true)
        );
        assert_eq!(get(name).as_deref(), Some("over"));
        crate::stdlib::unsetenv_unsafe(name).unwrap();
        reset(name);
    }

    #[test]
    fn names_exist() {
        assert_eq!(VIDEO_DRIVER, "SDL_VIDEO_DRIVER");
        assert_eq!(LOGGING, "SDL_LOGGING");
        assert_eq!(TIMER_RESOLUTION, "SDL_TIMER_RESOLUTION");
    }
}
