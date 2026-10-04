// Ownership of COM interface pointers for the Windows code. Not a
// translation of an upstream file: upstream calls the `IUnknown_AddRef()`
// and `IUnknown_Release()` macros by hand, where this releases on drop.

//! [`ComPtr`], an owned reference to a COM object (one `AddRef()` worth),
//! released on drop, over the hand-declared vtables of
//! [`ComObject`](super::immdevice::ComObject).

use std::ffi::c_void;
use std::ptr::NonNull;

use windows_sys::core::{GUID, HRESULT};

pub(crate) use super::immdevice::{ComObject, IUnknownVtbl};

/// An owned reference to a COM object whose vtable is a `V`.
pub(crate) struct ComPtr<V> {
    ptr: NonNull<ComObject<V>>,
}

// SAFETY: the COM objects SDL keeps (DirectInput devices and effects, the
// Windows.Gaming.Input objects, which are agile) may be used from any thread;
// SDL serializes their use with the joystick lock, as upstream does.
unsafe impl<V> Send for ComPtr<V> {}

impl<V> ComPtr<V> {
    /// Take ownership of one reference to `ptr` (`None` for NULL).
    ///
    /// # Safety
    ///
    /// `ptr` must be NULL or a live COM object whose vtable is a `V`; the
    /// caller gives up one reference to it.
    pub(crate) unsafe fn from_raw(ptr: *mut ComObject<V>) -> Option<ComPtr<V>> {
        NonNull::new(ptr).map(|ptr| ComPtr { ptr })
    }

    /// Call a function that returns a new reference through an out
    /// parameter (`CreateDevice()`, `QueryInterface()`, `get_*()`...):
    /// the reference on success, else the failing `HRESULT` (`S_OK` with a
    /// NULL pointer is `E_POINTER`).
    ///
    /// # Safety
    ///
    /// On success, `f` must have stored NULL or an owned reference to a COM
    /// object whose vtable is a `V`.
    pub(crate) unsafe fn from_out(
        f: impl FnOnce(*mut *mut ComObject<V>) -> HRESULT,
    ) -> Result<ComPtr<V>, HRESULT> {
        const E_POINTER: HRESULT = 0x8000_4003_u32 as HRESULT;
        let mut out: *mut ComObject<V> = std::ptr::null_mut();
        let hr = f(&mut out);
        if hr < 0 {
            return Err(hr);
        }
        // SAFETY: the caller's contract.
        unsafe { ComPtr::from_raw(out) }.ok_or(E_POINTER)
    }

    /// The interface pointer, still owned by `self`.
    pub(crate) fn as_ptr(&self) -> *mut ComObject<V> {
        self.ptr.as_ptr()
    }

    /// The object's vtable.
    pub(crate) fn vtbl(&self) -> &V {
        // SAFETY: the object is alive while we hold a reference, and its
        // vtable is a `V` (from_raw's contract).
        unsafe { ComObject::vtbl(self.ptr.as_ptr()) }
    }

    /// The `IUnknown` part of the vtable.
    fn unknown(&self) -> &IUnknownVtbl {
        // SAFETY: every COM vtable starts with IUnknown's.
        unsafe { &*(*self.ptr.as_ptr()).vtbl.cast::<IUnknownVtbl>() }
    }

    /// `QueryInterface()` for the interface `iid`, whose vtable is a `W`.
    pub(crate) fn query<W>(&self, iid: &GUID) -> Result<ComPtr<W>, HRESULT> {
        let query_interface = self.unknown().query_interface;
        let this = self.ptr.as_ptr().cast::<c_void>();
        // SAFETY: QueryInterface stores an owned reference to the object's
        // `iid` interface on success; the caller names the matching vtable.
        unsafe {
            ComPtr::from_out(|out: *mut *mut ComObject<W>| query_interface(this, iid, out.cast()))
        }
    }

    /// Whether `self` and `other` are the same interface pointer.
    pub(crate) fn same(&self, other: &ComPtr<V>) -> bool {
        self.ptr == other.ptr
    }
}

impl<V> Clone for ComPtr<V> {
    /// Another reference (`AddRef()`).
    fn clone(&self) -> ComPtr<V> {
        let add_ref = self.unknown().add_ref;
        // SAFETY: the object is alive; AddRef takes one more reference.
        unsafe {
            add_ref(self.ptr.as_ptr().cast());
        }
        ComPtr { ptr: self.ptr }
    }
}

impl<V> Drop for ComPtr<V> {
    /// Give the reference back (`Release()`).
    fn drop(&mut self) {
        let release = self.unknown().release;
        // SAFETY: we own one reference, released exactly once here.
        unsafe {
            release(self.ptr.as_ptr().cast());
        }
    }
}

impl<V> std::fmt::Debug for ComPtr<V> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "ComPtr({:p})", self.ptr)
    }
}

/// `IID_IUnknown`.
pub(crate) const IID_IUNKNOWN: GUID = GUID {
    data1: 0x00000000,
    data2: 0x0000,
    data3: 0x0000,
    data4: [0xC0, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x46],
};

/// `IID_IMarshal`.
pub(crate) const IID_IMARSHAL: GUID = GUID {
    data1: 0x00000003,
    data2: 0x0000,
    data3: 0x0000,
    data4: [0xC0, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x46],
};

/// `IID_IAgileObject`.
pub(crate) const IID_IAGILEOBJECT: GUID = GUID {
    data1: 0x94ea2b94,
    data2: 0xe9cc,
    data3: 0x49e0,
    data4: [0xc0, 0xff, 0xee, 0x64, 0xca, 0x8f, 0x5b, 0x90],
};

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU32, Ordering};

    #[repr(C)]
    struct Counted {
        vtbl: *const IUnknownVtbl,
        refs: AtomicU32,
    }

    unsafe extern "system" fn qi(
        this: *mut c_void,
        iid: *const GUID,
        out: *mut *mut c_void,
    ) -> HRESULT {
        // SAFETY: test object; the arguments are valid.
        unsafe {
            if super::super::is_equal_guid(&*iid, &IID_IUNKNOWN) {
                add_ref(this);
                *out = this;
                return 0;
            }
            *out = std::ptr::null_mut();
        }
        0x8000_4002_u32 as HRESULT // E_NOINTERFACE
    }
    unsafe extern "system" fn add_ref(this: *mut c_void) -> u32 {
        // SAFETY: `this` is a Counted.
        unsafe {
            (*this.cast::<Counted>())
                .refs
                .fetch_add(1, Ordering::SeqCst)
                + 1
        }
    }
    unsafe extern "system" fn release(this: *mut c_void) -> u32 {
        // SAFETY: `this` is a Counted.
        unsafe {
            (*this.cast::<Counted>())
                .refs
                .fetch_sub(1, Ordering::SeqCst)
                - 1
        }
    }
    static VTBL: IUnknownVtbl = IUnknownVtbl {
        query_interface: qi,
        add_ref,
        release,
    };

    #[test]
    fn references_are_counted() {
        let mut object = Counted {
            vtbl: &VTBL,
            refs: AtomicU32::new(1),
        };
        let raw = (&mut object as *mut Counted).cast::<ComObject<IUnknownVtbl>>();
        // SAFETY: `raw` is a live object with one reference, given to `ptr`.
        let ptr = unsafe { ComPtr::from_raw(raw) }.unwrap();
        let second = ptr.clone();
        assert!(ptr.same(&second));
        assert_eq!(object_refs(&ptr), 2);
        let third: ComPtr<IUnknownVtbl> = ptr.query(&IID_IUNKNOWN).unwrap();
        assert_eq!(object_refs(&ptr), 3);
        assert_eq!(
            ptr.query::<IUnknownVtbl>(&IID_IAGILEOBJECT).unwrap_err(),
            0x8000_4002_u32 as HRESULT
        );
        drop(third);
        drop(second);
        assert_eq!(object_refs(&ptr), 1);
        drop(ptr);
        assert_eq!(object.refs.load(Ordering::SeqCst), 0);
        // SAFETY: NULL is no object.
        assert!(unsafe { ComPtr::<IUnknownVtbl>::from_raw(std::ptr::null_mut()) }.is_none());
    }

    fn object_refs(ptr: &ComPtr<IUnknownVtbl>) -> u32 {
        // SAFETY: the object is a Counted.
        unsafe {
            (*ptr.as_ptr().cast::<Counted>())
                .refs
                .load(Ordering::SeqCst)
        }
    }
}
