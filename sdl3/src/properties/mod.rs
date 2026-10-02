// Rust translation of src/SDL_properties.c from Simple DirectMedia Layer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! Property groups: named, dynamically typed values.
//!
//! A [`Properties`] is a shareable, thread-safe map from names to
//! [`Value`]s (translation of `SDL_PropertiesID`). SDL uses property groups
//! both as "extra arguments" when creating objects and as per-object
//! metadata.
//!
//! Direct translation of `SDL_properties.c`, with Rust's ownership replacing
//! the C API's integer handles, `void *` payloads and cleanup callbacks:
//!
//! * `SDL_PropertiesID` → a cloneable [`Properties`] handle (`Arc` inside);
//!   destroying the group is dropping the last handle.
//! * pointer properties with cleanup functions → [`Value::Any`], an
//!   `Arc<dyn Any + Send + Sync>`; cleanup is the value's `Drop`.
//! * `SDL_LockProperties` → [`Properties::lock`], a guard with the same
//!   methods so several operations can be done atomically.

use std::any::Any;
use std::collections::HashMap;
use std::fmt;
use std::sync::{Arc, LazyLock, Mutex, MutexGuard};

use crate::error::{Error, Result};

/// A property value. Translation of `SDL_Property`'s tagged union.
#[derive(Clone)]
pub enum Value {
    /// Arbitrary shared data (translation of a pointer property).
    Any(Arc<dyn Any + Send + Sync>),
    String(String),
    Number(i64),
    Float(f32),
    Bool(bool),
}

/// The type of a [`Value`]. Translation of `SDL_PropertyType`.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum ValueKind {
    Any,
    String,
    Number,
    Float,
    Bool,
}

impl Value {
    /// Wrap any `Send + Sync + 'static` value.
    pub fn any<T: Any + Send + Sync>(value: T) -> Value {
        Value::Any(Arc::new(value))
    }

    pub fn kind(&self) -> ValueKind {
        match self {
            Value::Any(_) => ValueKind::Any,
            Value::String(_) => ValueKind::String,
            Value::Number(_) => ValueKind::Number,
            Value::Float(_) => ValueKind::Float,
            Value::Bool(_) => ValueKind::Bool,
        }
    }

    /// Downcast an [`Value::Any`].
    pub fn downcast<T: Any + Send + Sync>(&self) -> Option<Arc<T>> {
        match self {
            Value::Any(a) => Arc::clone(a).downcast::<T>().ok(),
            _ => None,
        }
    }

    /// As text, with SDL's conversions: numbers print as decimal, floats as
    /// C `%f`, booleans as `"true"`/`"false"`. `Any` has no text form.
    /// Translation of `SDL_GetStringProperty()`.
    pub fn as_string(&self) -> Option<String> {
        match self {
            Value::String(s) => Some(s.clone()),
            Value::Number(n) => Some(n.to_string()),
            Value::Float(f) => Some(format!("{f:.6}")),
            Value::Bool(b) => Some(if *b { "true" } else { "false" }.to_owned()),
            Value::Any(_) => None,
        }
    }

    /// As an integer, with SDL's conversions: strings parse like `strtoll(s, 0)`,
    /// floats round, booleans are 0/1. Translation of `SDL_GetNumberProperty()`.
    pub fn as_number(&self) -> Option<i64> {
        match self {
            Value::String(s) => Some(crate::stdlib::strtoll_base0(s)),
            Value::Number(n) => Some(*n),
            Value::Float(f) => Some((*f as f64).round() as i64),
            Value::Bool(b) => Some(*b as i64),
            Value::Any(_) => None,
        }
    }

    /// As a float, with SDL's conversions. Translation of `SDL_GetFloatProperty()`.
    pub fn as_float(&self) -> Option<f32> {
        match self {
            Value::String(s) => Some(crate::stdlib::atof(s) as f32),
            Value::Number(n) => Some(*n as f32),
            Value::Float(f) => Some(*f),
            Value::Bool(b) => Some((*b as i32) as f32),
            Value::Any(_) => None,
        }
    }

    /// As a boolean, with SDL's conversions (strings via
    /// [`hints::string_to_bool`](crate::hints::string_to_bool), numbers non-zero).
    /// Translation of `SDL_GetBooleanProperty()`.
    pub fn as_bool(&self) -> Option<bool> {
        match self {
            Value::String(s) => Some(crate::hints::string_to_bool(Some(s), false)),
            Value::Number(n) => Some(*n != 0),
            Value::Float(f) => Some(*f != 0.0),
            Value::Bool(b) => Some(*b),
            Value::Any(_) => None,
        }
    }
}

impl fmt::Debug for Value {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Value::Any(a) => write!(f, "Any({:?})", Arc::as_ptr(a)),
            Value::String(s) => write!(f, "String({s:?})"),
            Value::Number(n) => write!(f, "Number({n})"),
            Value::Float(x) => write!(f, "Float({x})"),
            Value::Bool(b) => write!(f, "Bool({b})"),
        }
    }
}

impl From<&str> for Value {
    fn from(s: &str) -> Value {
        Value::String(s.to_owned())
    }
}
impl From<String> for Value {
    fn from(s: String) -> Value {
        Value::String(s)
    }
}
impl From<i64> for Value {
    fn from(n: i64) -> Value {
        Value::Number(n)
    }
}
impl From<i32> for Value {
    fn from(n: i32) -> Value {
        Value::Number(n as i64)
    }
}
impl From<f32> for Value {
    fn from(f: f32) -> Value {
        Value::Float(f)
    }
}
impl From<bool> for Value {
    fn from(b: bool) -> Value {
        Value::Bool(b)
    }
}

type Map = HashMap<String, Value>;

/// A group of properties. Translation of `SDL_PropertiesID`.
///
/// Cloning yields another handle to the same group. All methods lock the
/// group for their duration; use [`Properties::lock`] to hold the lock across
/// several operations.
#[derive(Clone, Default)]
pub struct Properties {
    inner: Arc<Mutex<Map>>,
}

impl fmt::Debug for Properties {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.inner.try_lock() {
            Ok(map) => f.debug_map().entries(map.iter()).finish(),
            Err(_) => f.write_str("Properties { <locked> }"),
        }
    }
}

impl PartialEq for Properties {
    /// Handle identity: two handles are equal if they refer to the same group.
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.inner, &other.inner)
    }
}
impl Eq for Properties {}

static GLOBAL: LazyLock<Properties> = LazyLock::new(Properties::new);

impl Properties {
    /// Create an empty group. Translation of `SDL_CreateProperties()`.
    pub fn new() -> Properties {
        Properties {
            inner: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    /// The global SDL properties. Translation of `SDL_GetGlobalProperties()`.
    pub fn global() -> Properties {
        GLOBAL.clone()
    }

    /// Lock the group for several operations. Translation of `SDL_LockProperties()`.
    pub fn lock(&self) -> Locked<'_> {
        Locked {
            map: self.inner.lock().unwrap_or_else(|e| e.into_inner()),
        }
    }

    /// Set (or replace) a property. Translation of `SDL_Set*Property()`.
    pub fn set(&self, name: &str, value: impl Into<Value>) -> Result<()> {
        self.lock().set(name, value)
    }

    /// Store any `Send + Sync + 'static` value. Translation of
    /// `SDL_SetPointerPropertyWithCleanup()`; the value's `Drop` is the cleanup.
    pub fn set_any<T: Any + Send + Sync>(&self, name: &str, value: T) -> Result<()> {
        self.lock().set(name, Value::any(value))
    }

    /// The value, if present. Translation of `SDL_Get*Property()`.
    pub fn get(&self, name: &str) -> Option<Value> {
        self.lock().get(name)
    }

    /// A downcast [`Value::Any`]. Translation of `SDL_GetPointerProperty()`.
    pub fn get_any<T: Any + Send + Sync>(&self, name: &str) -> Option<Arc<T>> {
        self.lock().get_any(name)
    }

    /// See [`Value::as_string`]. Translation of `SDL_GetStringProperty()`.
    pub fn get_string(&self, name: &str) -> Option<String> {
        self.lock().get_string(name)
    }

    /// See [`Value::as_number`]. Translation of `SDL_GetNumberProperty()`.
    pub fn get_number(&self, name: &str) -> Option<i64> {
        self.lock().get_number(name)
    }

    /// See [`Value::as_float`]. Translation of `SDL_GetFloatProperty()`.
    pub fn get_float(&self, name: &str) -> Option<f32> {
        self.lock().get_float(name)
    }

    /// See [`Value::as_bool`]. Translation of `SDL_GetBooleanProperty()`.
    pub fn get_bool(&self, name: &str) -> Option<bool> {
        self.lock().get_bool(name)
    }

    /// The type of a property. Translation of `SDL_GetPropertyType()`.
    pub fn kind(&self, name: &str) -> Option<ValueKind> {
        self.lock().kind(name)
    }

    /// Translation of `SDL_HasProperty()`.
    pub fn contains(&self, name: &str) -> bool {
        self.lock().contains(name)
    }

    /// Remove a property, returning it. Translation of `SDL_ClearProperty()`.
    pub fn remove(&self, name: &str) -> Option<Value> {
        self.lock().remove(name)
    }

    /// Number of properties. Translation of `SDL_GetNumProperties()`.
    pub fn len(&self) -> usize {
        self.lock().len()
    }

    pub fn is_empty(&self) -> bool {
        self.lock().is_empty()
    }

    /// Property names, in no particular order. Translation of `SDL_EnumerateProperties()`.
    pub fn names(&self) -> Vec<String> {
        self.lock().names()
    }

    /// Copy every property from `src` into this group, overwriting
    /// duplicates. Translation of `SDL_CopyProperties()`.
    ///
    /// Upstream skips pointer properties that have cleanup functions because
    /// it cannot duplicate them; here [`Value::Any`] is reference counted, so
    /// those are shared instead of skipped.
    pub fn copy_from(&self, src: &Properties) -> Result<()> {
        if Arc::ptr_eq(&self.inner, &src.inner) {
            return Ok(());
        }
        let copies: Vec<(String, Value)> = src
            .lock()
            .map
            .iter()
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect();
        let mut dst = self.lock();
        for (name, value) in copies {
            dst.map.insert(name, value);
        }
        Ok(())
    }

    /// Log every property at info level. Translation of `SDL_DumpProperties()`.
    pub fn dump(&self) {
        self.lock().dump()
    }
}

/// A locked [`Properties`]; same operations, lock held until dropped.
pub struct Locked<'a> {
    map: MutexGuard<'a, Map>,
}

impl fmt::Debug for Locked<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_map().entries(self.map.iter()).finish()
    }
}

impl Locked<'_> {
    /// Translation of `SDL_PrivateSetProperty()`.
    pub fn set(&mut self, name: &str, value: impl Into<Value>) -> Result<()> {
        if name.is_empty() {
            return Err(Error::invalid_param("name"));
        }
        self.map.insert(name.to_owned(), value.into());
        Ok(())
    }

    pub fn get(&self, name: &str) -> Option<Value> {
        self.map.get(name).cloned()
    }

    pub fn get_any<T: Any + Send + Sync>(&self, name: &str) -> Option<Arc<T>> {
        self.map.get(name)?.downcast::<T>()
    }

    pub fn get_string(&self, name: &str) -> Option<String> {
        self.map.get(name)?.as_string()
    }

    pub fn get_number(&self, name: &str) -> Option<i64> {
        self.map.get(name)?.as_number()
    }

    pub fn get_float(&self, name: &str) -> Option<f32> {
        self.map.get(name)?.as_float()
    }

    pub fn get_bool(&self, name: &str) -> Option<bool> {
        self.map.get(name)?.as_bool()
    }

    pub fn kind(&self, name: &str) -> Option<ValueKind> {
        self.map.get(name).map(Value::kind)
    }

    pub fn contains(&self, name: &str) -> bool {
        self.map.contains_key(name)
    }

    pub fn remove(&mut self, name: &str) -> Option<Value> {
        self.map.remove(name)
    }

    pub fn len(&self) -> usize {
        self.map.len()
    }

    pub fn is_empty(&self) -> bool {
        self.map.is_empty()
    }

    pub fn names(&self) -> Vec<String> {
        self.map.keys().cloned().collect()
    }

    /// Iterate over `(name, value)` pairs.
    pub fn iter(&self) -> impl Iterator<Item = (&str, &Value)> {
        self.map.iter().map(|(k, v)| (k.as_str(), v))
    }

    /// Translation of `SDL_DumpPropertiesCallback()`.
    pub fn dump(&self) {
        for (name, value) in self.iter() {
            match value {
                Value::Any(a) => crate::log::info!("{name}: {:?}", Arc::as_ptr(a)),
                Value::String(s) => crate::log::info!("{name}: \"{s}\""),
                Value::Number(n) => crate::log::info!("{name}: {n} ({n:x})"),
                Value::Float(x) => crate::log::info!("{name}: {x}"),
                Value::Bool(b) => {
                    crate::log::info!("{name}: {}", if *b { "true" } else { "false" })
                }
            }
        }
    }
}

/// Translation of `SDL_QuitProperties()`: clears the global group.
pub(crate) fn quit_properties() {
    GLOBAL.lock().map.clear();
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[test]
    fn basic_types_and_conversions() {
        let p = Properties::new();
        p.set("s", "hello").unwrap();
        p.set("n", 42i64).unwrap();
        p.set("f", 1.5f32).unwrap();
        p.set("b", true).unwrap();
        assert_eq!(p.len(), 4);
        assert_eq!(p.kind("s"), Some(ValueKind::String));
        assert_eq!(p.get_string("s").as_deref(), Some("hello"));
        assert_eq!(p.get_string("n").as_deref(), Some("42"));
        assert_eq!(p.get_string("f").as_deref(), Some("1.500000"));
        assert_eq!(p.get_string("b").as_deref(), Some("true"));
        assert_eq!(p.get_number("s"), Some(0));
        assert_eq!(p.get_number("f"), Some(2));
        assert_eq!(p.get_number("b"), Some(1));
        assert_eq!(p.get_float("n"), Some(42.0));
        assert_eq!(p.get_bool("n"), Some(true));
        assert_eq!(p.get_bool("s"), Some(true)); // non-"0"/"false" string
        assert_eq!(p.get_number("missing"), None);
        assert!(p.contains("n"));
        assert_eq!(p.remove("n").map(|v| v.kind()), Some(ValueKind::Number));
        assert!(!p.contains("n"));
        assert_eq!(
            p.set("", 1).unwrap_err().kind(),
            crate::ErrorKind::InvalidParam
        );
        p.set("hex", "0x10").unwrap();
        assert_eq!(p.get_number("hex"), Some(16));
        let mut names = p.names();
        names.sort();
        assert_eq!(names, ["b", "f", "hex", "s"]);
        assert!(format!("{p:?}").contains("\"s\": String(\"hello\")"));
    }

    struct Tracked(Arc<AtomicUsize>);
    impl Drop for Tracked {
        fn drop(&mut self) {
            self.0.fetch_add(1, Ordering::SeqCst);
        }
    }

    #[test]
    fn any_values_and_drop_as_cleanup() {
        let drops = Arc::new(AtomicUsize::new(0));
        let p = Properties::new();
        p.set_any("t", Tracked(drops.clone())).unwrap();
        assert_eq!(p.kind("t"), Some(ValueKind::Any));
        assert!(p.get_any::<Tracked>("t").is_some());
        assert!(p.get_any::<String>("t").is_none());
        assert_eq!(p.get_string("t"), None);
        // replacing runs the "cleanup" (Drop)
        p.set_any("t", Tracked(drops.clone())).unwrap();
        assert_eq!(drops.load(Ordering::SeqCst), 1);
        // a copy shares the value
        let q = Properties::new();
        q.set("keep", 1).unwrap();
        q.copy_from(&p).unwrap();
        assert!(q.contains("t") && q.contains("keep"));
        drop(p);
        assert_eq!(drops.load(Ordering::SeqCst), 1); // still alive in q
        drop(q);
        assert_eq!(drops.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn handles_and_locking() {
        let p = Properties::new();
        let p2 = p.clone();
        assert_eq!(p, p2);
        assert_ne!(p, Properties::new());
        {
            let mut locked = p.lock();
            locked.set("a", 1).unwrap();
            locked.set("b", 2).unwrap();
            assert_eq!(locked.len(), 2);
            assert_eq!(locked.iter().count(), 2);
        }
        assert_eq!(p2.get_number("b"), Some(2));
        p.dump();
        assert_eq!(Properties::global(), Properties::global());
        Properties::global().set("sdl3.test", true).unwrap();
        assert_eq!(Properties::global().get_bool("sdl3.test"), Some(true));
    }
}
