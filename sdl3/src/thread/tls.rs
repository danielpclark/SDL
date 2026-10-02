// Rust translation of the thread-local storage functions of src/thread/SDL_thread.c
// from Simple DirectMedia Layer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

use std::any::Any;
use std::cell::RefCell;
use std::rc::Rc;
use std::sync::atomic::{AtomicI32, Ordering};

use crate::error::Result;

// The storage is local to the thread, but the IDs are global for the process
/// Translation of `SDL_tls_id`.
static SDL_TLS_ID: AtomicI32 = AtomicI32::new(0);

/// This is how many TLS entries we allocate at once. Translation of `TLS_ALLOC_CHUNKSIZE`.
const TLS_ALLOC_CHUNKSIZE: usize = 4;

type Slot = Option<Rc<dyn Any>>;

thread_local! {
    /// The current thread's `SDL_TLSData` (`array[i].data`; the destructor is `Drop`).
    static STORAGE: RefCell<Vec<Slot>> = const { RefCell::new(Vec::new()) };
}

/// A thread-local storage slot, shared by all threads, each of which sees
/// its own value. Translation of `SDL_TLSID`.
///
/// Declare one as a `static`; it is assigned an index the first time any
/// thread sets it. Values are dropped (the `SDL_TLSDestructorCallback`)
/// when the thread exits, when [`cleanup_tls`] runs, or when replaced.
///
/// ```
/// use sdl3::thread::TlsId;
/// static COUNTER: TlsId = TlsId::new();
/// COUNTER.set(5u32).unwrap();
/// assert_eq!(COUNTER.get::<u32>(), Some(5));
/// assert_eq!(std::thread::spawn(|| COUNTER.get::<u32>()).join().unwrap(), None);
/// ```
#[derive(Debug, Default)]
pub struct TlsId(AtomicI32);

impl TlsId {
    /// An unassigned slot (`SDL_TLSID id = { 0 }`).
    pub const fn new() -> Self {
        TlsId(AtomicI32::new(0))
    }

    /// The storage index, or `None` if no thread has set this slot yet.
    fn index(&self) -> Option<usize> {
        let storage_index = self.0.load(Ordering::Acquire) - 1;
        (storage_index >= 0).then_some(storage_index as usize)
    }

    /// Get the current thread's value, if it is set and of type `T`, and
    /// run `f` on it. Translation of `SDL_GetTLS()`.
    ///
    /// `f` may itself read or write thread-local storage.
    pub fn with<T: 'static, R>(&self, f: impl FnOnce(Option<&T>) -> R) -> R {
        let value: Slot = match self.index() {
            Some(i) => STORAGE.with(|s| s.borrow().get(i).cloned().flatten()),
            None => None,
        };
        f(value.as_deref().and_then(|v| v.downcast_ref::<T>()))
    }

    /// A clone of the current thread's value, if set and of type `T`.
    pub fn get<T: Clone + 'static>(&self) -> Option<T> {
        self.with(|v: Option<&T>| v.cloned())
    }

    /// Set the current thread's value. Any previous value is dropped.
    /// Translation of `SDL_SetTLS()` (the destructor is `T`'s `Drop`).
    pub fn set<T: 'static>(&self, value: T) -> Result<()> {
        self.store(Some(Rc::new(value)));
        Ok(())
    }

    /// Clear the current thread's value (dropping it).
    /// Translation of `SDL_SetTLS(id, NULL, NULL)`.
    pub fn clear(&self) {
        if self.index().is_some() {
            self.store(None);
        }
    }

    fn store(&self, value: Slot) {
        // Get the storage index associated with the ID in a thread-safe way
        let mut storage_index = self.0.load(Ordering::Acquire) - 1;
        if storage_index < 0 {
            let new_id = SDL_TLS_ID.fetch_add(1, Ordering::AcqRel) + 1;
            let _ = self
                .0
                .compare_exchange(0, new_id, Ordering::AcqRel, Ordering::Acquire);
            /* If there was a race condition we'll have wasted an ID, but every thread
             * will have the same storage index for this id.
             */
            storage_index = self.0.load(Ordering::Acquire) - 1;
        } else {
            // Make sure we don't allocate an ID clobbering this one
            let mut tls_id = SDL_TLS_ID.load(Ordering::Acquire);
            while storage_index >= tls_id {
                if SDL_TLS_ID
                    .compare_exchange(
                        tls_id,
                        storage_index + 1,
                        Ordering::AcqRel,
                        Ordering::Acquire,
                    )
                    .is_ok()
                {
                    break;
                }
                tls_id = SDL_TLS_ID.load(Ordering::Acquire);
            }
        }
        let storage_index = storage_index as usize;

        // Get the storage for the current thread, growing it in chunks
        let old = STORAGE.with(|s| {
            let mut storage = s.borrow_mut();
            if storage_index >= storage.len() {
                let newlimit = storage_index + TLS_ALLOC_CHUNKSIZE;
                storage.resize(newlimit, None);
            }
            std::mem::replace(&mut storage[storage_index], value)
        });
        // Drop the previous value only after releasing the borrow: its Drop
        // may use TLS too.
        drop(old);
    }
}

/// Drop all of the current thread's TLS values. Called automatically when a
/// [`Thread`](super::Thread) ends; call it yourself on threads not created
/// by SDL if you need the values gone before the thread exits.
/// Translation of `SDL_CleanupTLS()`.
pub fn cleanup_tls() {
    // Cleanup the storage for the current thread
    let storage = STORAGE.with(|s| std::mem::take(&mut *s.borrow_mut()));
    drop(storage); // destructors run in index order, outside the borrow
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;

    struct Noisy<'a>(&'a Cell<u32>);
    impl Drop for Noisy<'_> {
        fn drop(&mut self) {
            self.0.set(self.0.get() + 1);
        }
    }

    #[test]
    fn per_thread_values() {
        static A: TlsId = TlsId::new();
        static B: TlsId = TlsId::new();
        assert_eq!(A.get::<i32>(), None);
        A.set(1i32).unwrap();
        B.set(String::from("b")).unwrap();
        assert_eq!(A.get::<i32>(), Some(1));
        assert_eq!(A.get::<String>(), None, "wrong type reads as unset");
        assert_eq!(B.get::<String>().as_deref(), Some("b"));
        let other = std::thread::spawn(|| {
            assert_eq!(A.get::<i32>(), None);
            A.set(2i32).unwrap();
            A.get::<i32>()
        })
        .join()
        .unwrap();
        assert_eq!(other, Some(2));
        assert_eq!(A.get::<i32>(), Some(1));
        assert_ne!(A.index(), B.index());

        // Re-entrant use inside `with`.
        A.with(|v: Option<&i32>| {
            assert_eq!(v, Some(&1));
            A.set(3i32).unwrap();
        });
        assert_eq!(A.get::<i32>(), Some(3));
        A.clear();
        assert_eq!(A.get::<i32>(), None);
    }

    #[test]
    fn destructors_run_on_replace_and_cleanup() {
        static C: TlsId = TlsId::new();
        let drops: &'static Cell<u32> = Box::leak(Box::new(Cell::new(0)));
        C.set(Noisy(drops)).unwrap();
        C.set(Noisy(drops)).unwrap();
        assert_eq!(drops.get(), 1);
        cleanup_tls();
        assert_eq!(drops.get(), 2);
        assert!(C.with(|v: Option<&Noisy>| v.is_none()));
    }
}
