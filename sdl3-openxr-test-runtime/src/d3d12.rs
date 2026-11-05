// The test runtime's Direct3D 12 objects: the adapter it asks for, and its
// swapchains' images, made on the application's device.
// Part of the repository's own test code (not a translation); see LICENSE.txt.

//! `XR_KHR_D3D12_enable` on Windows: the adapter's LUID comes from DXGI
//! (`dxgi.dll`, an OS library, loaded at run time: `CreateDXGIFactory1`,
//! then `IDXGIFactory4::EnumWarpAdapter`, WARP's adapter, which every
//! Windows has and the CI runners use; or the first adapter of
//! `IDXGIFactory1::EnumAdapters1` where there's no WARP, as under Wine),
//! and the swapchains' images are committed resources made through the
//! application's `ID3D12Device` (its vtable: the runtime loads no
//! Direct3D 12 library). The COM interfaces are declared here by slot, for
//! the few methods called.

use std::ffi::{c_char, c_void};
use std::sync::OnceLock;

use crate::sys::*;

/// `HRESULT`.
type Hresult = i32;

/// `GUID`.
#[repr(C)]
pub struct Guid {
    data1: u32,
    data2: u16,
    data3: u16,
    data4: [u8; 8],
}

impl Guid {
    const fn from_u128(v: u128) -> Guid {
        Guid {
            data1: (v >> 96) as u32,
            data2: (v >> 80) as u16,
            data3: (v >> 64) as u16,
            data4: (v as u64).to_be_bytes(),
        }
    }
}

const IID_IDXGIFACTORY1: Guid = Guid::from_u128(0x770aae78_f26f_4dba_a829_253c83d1b387);
const IID_IDXGIFACTORY4: Guid = Guid::from_u128(0x1bc6ea02_ef36_464f_bf0c_21ca39e5168a);
const IID_IDXGIADAPTER1: Guid = Guid::from_u128(0x29038f61_3839_4626_91fd_086879011a05);
const IID_ID3D12RESOURCE: Guid = Guid::from_u128(0x696442be_a72e_4059_bc79_5b5c98040fad);

/// The swapchain formats (`DXGI_FORMAT`s): `R8G8B8A8_UNORM_SRGB`,
/// `R8G8B8A8_UNORM`, `B8G8R8A8_UNORM_SRGB`, `B8G8R8A8_UNORM`,
/// `R16G16B16A16_FLOAT`, `D32_FLOAT` and `D24_UNORM_S8_UINT` (as the
/// Vulkan formats, the last two depth formats).
pub const FORMATS: [i64; 7] = [29, 28, 91, 87, 10, 40, 45];

const D3D12_HEAP_TYPE_DEFAULT: u32 = 1;
const D3D12_RESOURCE_DIMENSION_TEXTURE2D: u32 = 3;
const D3D12_RESOURCE_FLAG_ALLOW_RENDER_TARGET: u32 = 0x1;
const D3D12_RESOURCE_FLAG_ALLOW_DEPTH_STENCIL: u32 = 0x2;
const D3D12_RESOURCE_FLAG_ALLOW_UNORDERED_ACCESS: u32 = 0x4;
const D3D12_RESOURCE_FLAG_DENY_SHADER_RESOURCE: u32 = 0x8;
const D3D12_RESOURCE_STATE_COMMON: u32 = 0;
const D3D12_RESOURCE_STATE_RENDER_TARGET: u32 = 0x4;
const D3D12_RESOURCE_STATE_DEPTH_WRITE: u32 = 0x10;

/// `DXGI_ADAPTER_DESC1`.
#[repr(C)]
struct AdapterDesc1 {
    description: [u16; 128],
    vendor_id: u32,
    device_id: u32,
    sub_sys_id: u32,
    revision: u32,
    dedicated_video_memory: usize,
    dedicated_system_memory: usize,
    shared_system_memory: usize,
    adapter_luid: Luid,
    flags: u32,
}

/// `D3D12_HEAP_PROPERTIES`.
#[repr(C)]
struct HeapProperties {
    ty: u32,
    cpu_page_property: u32,
    memory_pool_preference: u32,
    creation_node_mask: u32,
    visible_node_mask: u32,
}

/// `DXGI_SAMPLE_DESC`.
#[repr(C)]
struct SampleDesc {
    count: u32,
    quality: u32,
}

/// `D3D12_RESOURCE_DESC`.
#[repr(C)]
struct ResourceDesc {
    dimension: u32,
    alignment: u64,
    width: u64,
    height: u32,
    depth_or_array_size: u16,
    mip_levels: u16,
    format: u32,
    sample_desc: SampleDesc,
    layout: u32,
    flags: u32,
}

#[link(name = "kernel32")]
extern "system" {
    fn LoadLibraryW(name: *const u16) -> *mut c_void;
    fn GetProcAddress(module: *mut c_void, name: *const c_char) -> *mut c_void;
}

type PfnCreateDxgiFactory1 =
    unsafe extern "system" fn(riid: *const Guid, factory: *mut *mut c_void) -> Hresult;

/// The vtable slots of the methods called (from 0, `IUnknown`'s three
/// first).
mod slot {
    pub const QUERY_INTERFACE: usize = 0;
    pub const RELEASE: usize = 2;
    /// `IDXGIFactory1::EnumAdapters1` (after `IDXGIObject`'s four and
    /// `IDXGIFactory`'s five).
    pub const ENUM_ADAPTERS1: usize = 12;
    /// `IDXGIFactory4::EnumWarpAdapter` (after `IDXGIFactory1`'s two,
    /// `IDXGIFactory2`'s eleven, `IDXGIFactory3`'s one and
    /// `EnumAdapterByLuid`).
    pub const ENUM_WARP_ADAPTER: usize = 27;
    /// `IDXGIAdapter1::GetDesc1` (after `IDXGIObject`'s four and
    /// `IDXGIAdapter`'s three).
    pub const GET_DESC1: usize = 10;
    /// `ID3D12Device::CreateCommittedResource` (after `ID3D12Object`'s
    /// four and twenty of `ID3D12Device`'s).
    pub const CREATE_COMMITTED_RESOURCE: usize = 27;
    /// `ID3D12Device::GetAdapterLuid`, the last of `ID3D12Device`'s.
    pub const GET_ADAPTER_LUID: usize = 43;
}

/// A COM object's method, as `F`.
///
/// # Safety
///
/// `object` is a live COM object whose vtable has the method `F` at `slot`.
unsafe fn method<F: Copy>(object: *mut c_void, slot: usize) -> F {
    const { assert!(std::mem::size_of::<F>() == std::mem::size_of::<usize>()) };
    // SAFETY: the caller's; a COM object starts with its vtable's address.
    unsafe {
        let vtbl = *object.cast::<*const usize>();
        std::mem::transmute_copy(&*vtbl.add(slot))
    }
}

/// `IUnknown::Release()`.
///
/// # Safety
///
/// `object` is a live COM object, one of whose references is the caller's.
pub unsafe fn release(object: *mut c_void) {
    // SAFETY: the caller's; IUnknown's method.
    unsafe {
        let f: unsafe extern "system" fn(*mut c_void) -> u32 = method(object, slot::RELEASE);
        f(object);
    }
}

/// An owned COM reference, released when dropped.
struct Com(*mut c_void);

impl Drop for Com {
    fn drop(&mut self) {
        // SAFETY: the reference is ours.
        unsafe { release(self.0) };
    }
}

/// `dxgi.dll`'s `CreateDXGIFactory1`, loaded once (the library stays
/// loaded).
fn create_dxgi_factory1() -> Option<PfnCreateDxgiFactory1> {
    static CREATE: OnceLock<Option<usize>> = OnceLock::new();
    let f = (*CREATE.get_or_init(|| {
        let name: Vec<u16> = "dxgi.dll\0".encode_utf16().collect();
        // SAFETY: a NUL-terminated name, a NUL-terminated symbol.
        unsafe {
            let module = LoadLibraryW(name.as_ptr());
            if module.is_null() {
                return None;
            }
            let f = GetProcAddress(module, c"CreateDXGIFactory1".as_ptr());
            (!f.is_null()).then_some(f as usize)
        }
    }))?;
    // SAFETY: the export's address, of that type.
    Some(unsafe { std::mem::transmute::<usize, PfnCreateDxgiFactory1>(f) })
}

/// The adapter the runtime asks for: WARP's (`EnumWarpAdapter`), else the
/// first one's (where there is no WARP, as under Wine): its LUID.
pub fn adapter_luid() -> Option<Luid> {
    let create = create_dxgi_factory1()?;
    // SAFETY: the DXGI calls, with owned references released by `Com`.
    unsafe {
        let mut factory1 = std::ptr::null_mut();
        if create(&IID_IDXGIFACTORY1, &mut factory1) < 0 || factory1.is_null() {
            return None;
        }
        let factory1 = Com(factory1);
        let mut adapter = std::ptr::null_mut();
        let query: unsafe extern "system" fn(
            *mut c_void,
            *const Guid,
            *mut *mut c_void,
        ) -> Hresult = method(factory1.0, slot::QUERY_INTERFACE);
        let mut factory4 = std::ptr::null_mut();
        if query(factory1.0, &IID_IDXGIFACTORY4, &mut factory4) >= 0 && !factory4.is_null() {
            let factory4 = Com(factory4);
            let enum_warp_adapter: unsafe extern "system" fn(
                *mut c_void,
                *const Guid,
                *mut *mut c_void,
            ) -> Hresult = method(factory4.0, slot::ENUM_WARP_ADAPTER);
            if enum_warp_adapter(factory4.0, &IID_IDXGIADAPTER1, &mut adapter) < 0 {
                adapter = std::ptr::null_mut();
            }
        }
        if adapter.is_null() {
            let enum_adapters1: unsafe extern "system" fn(
                *mut c_void,
                u32,
                *mut *mut c_void,
            ) -> Hresult = method(factory1.0, slot::ENUM_ADAPTERS1);
            if enum_adapters1(factory1.0, 0, &mut adapter) < 0 || adapter.is_null() {
                return None;
            }
        }
        let adapter = Com(adapter);
        let get_desc1: unsafe extern "system" fn(*mut c_void, *mut AdapterDesc1) -> Hresult =
            method(adapter.0, slot::GET_DESC1);
        let mut desc: AdapterDesc1 = std::mem::zeroed();
        if get_desc1(adapter.0, &mut desc) < 0 {
            return None;
        }
        Some(desc.adapter_luid)
    }
}

/// The application's `ID3D12Device` of a session (its address: the state
/// is shared between threads), which the application keeps alive while
/// the session is, as the specification requires.
#[derive(Clone, Copy)]
pub struct Device(usize);

impl Device {
    /// The binding's device.
    ///
    /// # Safety
    ///
    /// `device` is null or the application's live `ID3D12Device`.
    pub unsafe fn new(device: *mut c_void) -> Option<Device> {
        (!device.is_null()).then_some(Device(device as usize))
    }

    fn ptr(&self) -> *mut c_void {
        self.0 as *mut c_void
    }

    /// `ID3D12Device::GetAdapterLuid()`.
    pub fn adapter_luid(&self) -> Luid {
        let mut luid = Luid::default();
        // SAFETY: the live device; the method writes the LUID it returns
        // (C's declaration of a method returning a structure).
        unsafe {
            let f: unsafe extern "system" fn(*mut c_void, *mut Luid) -> *mut Luid =
                method(self.ptr(), slot::GET_ADAPTER_LUID);
            f(self.ptr(), &mut luid);
        }
        luid
    }

    /// A swapchain image: a committed 2D texture resource (the default
    /// heap) of the swapchain's size, format, array size and mip count,
    /// with the resource flags of its usage and in the state its usage
    /// begins in ([`resource_flags`]); `None` if the device can't make it.
    ///
    /// # Safety
    ///
    /// The device is alive.
    pub unsafe fn create_image(&self, info: &XrSwapchainCreateInfo) -> Option<*mut c_void> {
        let (flags, state) = resource_flags(info.usage_flags);
        let heap = HeapProperties {
            ty: D3D12_HEAP_TYPE_DEFAULT,
            cpu_page_property: 0,
            memory_pool_preference: 0,
            creation_node_mask: 0,
            visible_node_mask: 0,
        };
        let desc = ResourceDesc {
            dimension: D3D12_RESOURCE_DIMENSION_TEXTURE2D,
            alignment: 0,
            width: u64::from(info.width),
            height: info.height,
            depth_or_array_size: info.array_size as u16,
            mip_levels: info.mip_count as u16,
            format: info.format as u32,
            sample_desc: SampleDesc {
                count: info.sample_count,
                quality: 0,
            },
            layout: 0,
            flags,
        };
        let mut resource = std::ptr::null_mut();
        // SAFETY: the caller's; the structures outlive the call, which
        // stores an owned resource of the IID asked for.
        unsafe {
            let f: unsafe extern "system" fn(
                *mut c_void,
                *const HeapProperties,
                u32,
                *const ResourceDesc,
                u32,
                *const c_void,
                *const Guid,
                *mut *mut c_void,
            ) -> Hresult = method(self.ptr(), slot::CREATE_COMMITTED_RESOURCE);
            let res = f(
                self.ptr(),
                &heap,
                0,
                &desc,
                state,
                std::ptr::null(),
                &IID_ID3D12RESOURCE,
                &mut resource,
            );
            (res >= 0 && !resource.is_null()).then_some(resource)
        }
    }
}

/// The Direct3D 12 resource flags of OpenXR swapchain usage flags
/// (`XR_KHR_D3D12_enable`'s table: exactly those asked for, but for
/// `D3D12_RESOURCE_FLAG_DENY_SHADER_RESOURCE` without `SAMPLED`, which
/// Direct3D 12 allows only with `ALLOW_DEPTH_STENCIL`: elsewhere the
/// shader resource isn't denied, which the specification allows), and the
/// state the image is made in: the render target state for a color
/// attachment, the depth write state for a depth one (the states the
/// specification gives the images when waited for), else the common
/// state.
pub fn resource_flags(usage: XrFlags64) -> (u32, u32) {
    let mut flags = 0;
    if usage & XR_SWAPCHAIN_USAGE_COLOR_ATTACHMENT_BIT != 0 {
        flags |= D3D12_RESOURCE_FLAG_ALLOW_RENDER_TARGET;
    }
    if usage & XR_SWAPCHAIN_USAGE_DEPTH_STENCIL_ATTACHMENT_BIT != 0 {
        flags |= D3D12_RESOURCE_FLAG_ALLOW_DEPTH_STENCIL;
        if usage & XR_SWAPCHAIN_USAGE_SAMPLED_BIT == 0 {
            flags |= D3D12_RESOURCE_FLAG_DENY_SHADER_RESOURCE;
        }
    }
    if usage & XR_SWAPCHAIN_USAGE_UNORDERED_ACCESS_BIT != 0 {
        flags |= D3D12_RESOURCE_FLAG_ALLOW_UNORDERED_ACCESS;
    }
    let state = if usage & XR_SWAPCHAIN_USAGE_COLOR_ATTACHMENT_BIT != 0 {
        D3D12_RESOURCE_STATE_RENDER_TARGET
    } else if usage & XR_SWAPCHAIN_USAGE_DEPTH_STENCIL_ATTACHMENT_BIT != 0 {
        D3D12_RESOURCE_STATE_DEPTH_WRITE
    } else {
        D3D12_RESOURCE_STATE_COMMON
    };
    (flags, state)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The layouts the sdl3 crate's Direct3D 12 declarations have (checked
    /// there against a mingw-w64 C harness).
    #[cfg(target_pointer_width = "64")]
    #[test]
    fn layouts_match_c() {
        use std::mem::{offset_of, size_of};
        assert_eq!(size_of::<AdapterDesc1>(), 312);
        assert_eq!(offset_of!(AdapterDesc1, adapter_luid), 296);
        assert_eq!(offset_of!(AdapterDesc1, flags), 304);
        assert_eq!(size_of::<HeapProperties>(), 20);
        assert_eq!(size_of::<ResourceDesc>(), 56);
        assert_eq!(offset_of!(ResourceDesc, width), 16);
        assert_eq!(offset_of!(ResourceDesc, depth_or_array_size), 28);
        assert_eq!(offset_of!(ResourceDesc, format), 32);
        assert_eq!(offset_of!(ResourceDesc, flags), 48);
        assert_eq!(size_of::<Guid>(), 16);
        assert_eq!(
            IID_ID3D12RESOURCE.data4,
            [0xbc, 0x79, 0x5b, 0x5c, 0x98, 0x04, 0x0f, 0xad]
        );
    }

    #[test]
    fn flags_follow_the_usage() {
        assert_eq!(resource_flags(0), (0, D3D12_RESOURCE_STATE_COMMON));
        assert_eq!(
            resource_flags(
                XR_SWAPCHAIN_USAGE_COLOR_ATTACHMENT_BIT | XR_SWAPCHAIN_USAGE_TRANSFER_SRC_BIT
            ),
            (
                D3D12_RESOURCE_FLAG_ALLOW_RENDER_TARGET,
                D3D12_RESOURCE_STATE_RENDER_TARGET
            )
        );
        assert_eq!(
            resource_flags(XR_SWAPCHAIN_USAGE_DEPTH_STENCIL_ATTACHMENT_BIT),
            (
                D3D12_RESOURCE_FLAG_ALLOW_DEPTH_STENCIL | D3D12_RESOURCE_FLAG_DENY_SHADER_RESOURCE,
                D3D12_RESOURCE_STATE_DEPTH_WRITE
            )
        );
        assert_eq!(
            resource_flags(
                XR_SWAPCHAIN_USAGE_SAMPLED_BIT | XR_SWAPCHAIN_USAGE_UNORDERED_ACCESS_BIT
            ),
            (
                D3D12_RESOURCE_FLAG_ALLOW_UNORDERED_ACCESS,
                D3D12_RESOURCE_STATE_COMMON
            )
        );
    }

    /// DXGI gives an adapter (WARP on Windows, the first under Wine).
    #[test]
    fn an_adapter_is_found() {
        assert!(adapter_luid().is_some());
    }
}
