// The test runtime's OpenXR functions: instances, the system, sessions and
// their states, swapchains, spaces and the frame loop.
// Part of the repository's own test code (not a translation); see LICENSE.txt.

//! What each function does is the OpenXR 1.0 specification's (and
//! `XR_KHR_vulkan_enable2`'s, `XR_KHR_D3D12_enable`'s and, on Android,
//! `XR_KHR_android_create_instance`'s: see `android.rs`), for the
//! arguments the specification allows, with the runtime's fixed choices:
//!
//! * one system, for the head-mounted form factor (the handheld one is
//!   `XR_ERROR_FORM_FACTOR_UNSUPPORTED`), with the primary stereo view
//!   configuration and the opaque blend mode;
//! * Vulkan 1.0 to 1.3 (`xrGetVulkanGraphicsRequirements2KHR`), the first
//!   physical device of the application's instance;
//! * on Windows, Direct3D 12 with feature level 11_0 on WARP's adapter
//!   (`xrGetD3D12GraphicsRequirementsKHR`'s LUID: see `d3d12.rs`), whose
//!   device the session's binding must be made on;
//! * the swapchain formats [`FORMATS`] (Vulkan) or `d3d12::FORMATS`
//!   (Direct3D 12), in that order; [`IMAGE_COUNT`] images a swapchain (one
//!   for a static one), with exactly the usage asked for, made on the
//!   application's device: Vulkan images in their undefined layout (the
//!   runtime records no commands: it never reads the images, so it never
//!   transitions them either: the application's own first transition,
//!   from the undefined layout, is the one they get), Direct3D 12
//!   committed resources in the state the specification gives the images
//!   of their usage (the render target state for a color attachment, the
//!   depth write state for a depth one), else the common state;
//! * the session's states: IDLE then READY when it's made, SYNCHRONIZED
//!   when it begins, VISIBLE and FOCUSED after its first frame; an exit
//!   request goes VISIBLE, SYNCHRONIZED, STOPPING, then IDLE and EXITING
//!   once the session ends;
//! * time: [`FRAME_PERIOD`] a frame from [`START_TIME`], whatever the
//!   clock (nothing waits).
//!
//! One state behind a mutex; every function holds it for its whole call.

use std::collections::{BTreeMap, VecDeque};
use std::ffi::{c_char, c_void, CStr};
use std::ptr::null_mut;
use std::sync::{Mutex, MutexGuard};

#[cfg(windows)]
use crate::d3d12;
use crate::fail::{self, Outcome};
use crate::sys::*;
use crate::vulkan::{self, DeviceFns};

/// The swapchain formats (`VkFormat`s): `VK_FORMAT_R8G8B8A8_SRGB`,
/// `R8G8B8A8_UNORM`, `B8G8R8A8_SRGB`, `B8G8R8A8_UNORM`,
/// `R16G16B16A16_SFLOAT`, `D32_SFLOAT` and `S8_UINT`.
pub const FORMATS: [i64; 7] = [43, 37, 50, 44, 97, 126, 127];
/// The images of a (not static) swapchain.
pub const IMAGE_COUNT: u32 = 3;
/// The system's id.
pub const SYSTEM_ID: XrSystemId = 1;
/// The first frame's predicted display time.
pub const START_TIME: XrTime = 1_000_000_000;
/// The frame period (60 Hz, in nanoseconds).
pub const FRAME_PERIOD: XrDuration = 16_666_667;
/// The largest swapchain image and array.
pub const MAX_SWAPCHAIN_SIZE: u32 = 4096;
/// The most composition layers a frame.
pub const MAX_LAYERS: u32 = 16;

/// A Vulkan instance made by `xrCreateVulkanInstanceKHR`.
struct VulkanInstance {
    /// (`VkInstance`, as an address)
    instance: usize,
    gipa: PfnVkGetInstanceProcAddr,
    /// `xrGetVulkanGraphicsDevice2KHR`'s answer (a `VkPhysicalDevice`).
    physical_device: usize,
    /// The devices `xrCreateVulkanDeviceKHR` made from it (`VkDevice`s).
    devices: Vec<usize>,
}

struct Instance {
    vulkan_enable2: bool,
    d3d12_enable: bool,
    /// Whether `xrGetSystem` gave the system.
    system: bool,
    /// Whether `xrGetVulkanGraphicsRequirements2KHR` was called.
    requirements: bool,
    /// The LUID `xrGetD3D12GraphicsRequirementsKHR` gave, once called.
    #[cfg_attr(not(windows), allow(dead_code))]
    d3d12_luid: Option<Luid>,
    vulkan: Vec<VulkanInstance>,
    events: VecDeque<(XrSession, i32, XrTime)>,
    session: Option<XrSession>,
}

/// A session's graphics API, and the application's device of its binding.
#[derive(Clone, Copy)]
#[allow(clippy::large_enum_variant)] // (one a session, copied out of the lock)
enum Graphics {
    Vulkan(DeviceFns),
    #[cfg(windows)]
    D3D12(d3d12::Device),
}

/// A swapchain image: a Vulkan image with its memory, or a Direct3D 12
/// resource (its address).
#[derive(Clone, Copy)]
enum Image {
    Vulkan(VkImage, VkDeviceMemory),
    #[cfg(windows)]
    D3D12(usize),
}

impl Graphics {
    /// The swapchain formats of the API.
    fn formats(&self) -> &'static [i64] {
        match self {
            Graphics::Vulkan(_) => &FORMATS,
            #[cfg(windows)]
            Graphics::D3D12(_) => &d3d12::FORMATS,
        }
    }

    /// Destroy an image this device made.
    ///
    /// # Safety
    ///
    /// The device is alive, the image one of its, unused.
    unsafe fn destroy_image(&self, image: Image) {
        match (self, image) {
            // SAFETY: the caller's.
            (Graphics::Vulkan(device), Image::Vulkan(image, memory)) => unsafe {
                device.destroy_image((image, memory))
            },
            // SAFETY: the caller's; the resource's reference is the
            // runtime's.
            #[cfg(windows)]
            (Graphics::D3D12(_), Image::D3D12(resource)) => unsafe {
                d3d12::release(resource as *mut c_void)
            },
            #[cfg(windows)]
            _ => unreachable!("an image of another graphics API"),
        }
    }
}

struct Session {
    instance: XrInstance,
    graphics: Graphics,
    state: i32,
    running: bool,
    exit_requested: bool,
    /// Frames waited for and not begun.
    waited: u32,
    /// Whether a frame is begun and not ended.
    begun: bool,
    time: XrTime,
}

struct Swapchain {
    session: XrSession,
    images: Vec<Image>,
    array_size: u32,
    is_static: bool,
    /// The acquired images, oldest first, and whether each was waited for.
    acquired: VecDeque<(u32, bool)>,
    next: u32,
    acquisitions: u32,
    released: bool,
}

struct Space {
    session: XrSession,
}

pub struct Runtime {
    next_handle: u64,
    /// Whether the loader negotiated (it then handles the API layers).
    pub negotiated: bool,
    instances: BTreeMap<XrInstance, Instance>,
    sessions: BTreeMap<XrSession, Session>,
    swapchains: BTreeMap<XrSwapchain, Swapchain>,
    spaces: BTreeMap<XrSpace, Space>,
}

static RUNTIME: Mutex<Runtime> = Mutex::new(Runtime {
    next_handle: 1,
    negotiated: false,
    instances: BTreeMap::new(),
    sessions: BTreeMap::new(),
    swapchains: BTreeMap::new(),
    spaces: BTreeMap::new(),
});

pub fn runtime() -> MutexGuard<'static, Runtime> {
    RUNTIME.lock().unwrap_or_else(|e| e.into_inner())
}

impl Runtime {
    fn handle(&mut self) -> u64 {
        let handle = self.next_handle;
        self.next_handle += 1;
        handle
    }

    fn instance(&mut self, instance: XrInstance) -> Result<&mut Instance, XrResult> {
        self.instances
            .get_mut(&instance)
            .ok_or(XR_ERROR_HANDLE_INVALID)
    }

    /// Whether an instance is alive, and whether it was created with
    /// `XR_KHR_vulkan_enable2` and with `XR_KHR_D3D12_enable`.
    pub fn enabled(&self, instance: XrInstance) -> Option<(bool, bool)> {
        self.instances
            .get(&instance)
            .map(|i| (i.vulkan_enable2, i.d3d12_enable))
    }

    fn system(
        &mut self,
        instance: XrInstance,
        system_id: XrSystemId,
    ) -> Result<&mut Instance, XrResult> {
        let i = self.instance(instance)?;
        if system_id != SYSTEM_ID || !i.system {
            return Err(XR_ERROR_SYSTEM_INVALID);
        }
        Ok(i)
    }

    fn session(&mut self, session: XrSession) -> Result<&mut Session, XrResult> {
        self.sessions
            .get_mut(&session)
            .ok_or(XR_ERROR_HANDLE_INVALID)
    }

    /// A session's new state, and its event.
    fn set_state(&mut self, session: XrSession, state: i32) {
        let Some(s) = self.sessions.get_mut(&session) else {
            return;
        };
        s.state = state;
        let (instance, time) = (s.instance, s.time);
        if let Some(i) = self.instances.get_mut(&instance) {
            i.events.push_back((session, state, time));
        }
    }

    fn destroy_swapchain(&mut self, swapchain: XrSwapchain) {
        let Some(sc) = self.swapchains.remove(&swapchain) else {
            return;
        };
        if let Some(session) = self.sessions.get(&sc.session) {
            for image in sc.images {
                // SAFETY: the session's device (alive while the session is,
                // as the specification requires), its images.
                unsafe { session.graphics.destroy_image(image) };
            }
        }
    }

    fn destroy_session(&mut self, session: XrSession) {
        let swapchains: Vec<XrSwapchain> = self
            .swapchains
            .iter()
            .filter(|(_, sc)| sc.session == session)
            .map(|(&h, _)| h)
            .collect();
        for swapchain in swapchains {
            self.destroy_swapchain(swapchain);
        }
        self.spaces.retain(|_, s| s.session != session);
        if let Some(s) = self.sessions.remove(&session) {
            if let Some(i) = self.instances.get_mut(&s.instance) {
                i.session = None;
                i.events.retain(|e| e.0 != session);
            }
        }
    }
}

/// A C string array's text.
fn c_text(name: &[c_char]) -> &[u8] {
    // SAFETY: c_char and u8 have the same layout.
    let bytes = unsafe { std::slice::from_raw_parts(name.as_ptr().cast::<u8>(), name.len()) };
    let end = bytes.iter().position(|&b| b == 0).unwrap_or(bytes.len());
    &bytes[..end]
}

/// Copy a name into a C string array (truncated, terminated).
fn set_c_text(dst: &mut [c_char], src: &str) {
    let n = src.len().min(dst.len() - 1);
    for (d, &b) in dst.iter_mut().zip(&src.as_bytes()[..n]) {
        *d = b as c_char;
    }
    dst[n] = 0;
}

/// The extensions the runtime has (`XR_KHR_vulkan_enable2`, on Windows
/// `XR_KHR_D3D12_enable`, on Android `XR_KHR_android_create_instance`,
/// unless the failure list removes them): names and versions.
fn extensions() -> Vec<(&'static str, u32)> {
    let mut list = Vec::new();
    if !fail::extension_removed(XR_KHR_VULKAN_ENABLE2_EXTENSION_NAME) {
        list.push((
            XR_KHR_VULKAN_ENABLE2_EXTENSION_NAME,
            XR_KHR_VULKAN_ENABLE2_SPEC_VERSION,
        ));
    }
    if cfg!(windows) && !fail::extension_removed(XR_KHR_D3D12_ENABLE_EXTENSION_NAME) {
        list.push((
            XR_KHR_D3D12_ENABLE_EXTENSION_NAME,
            XR_KHR_D3D12_ENABLE_SPEC_VERSION,
        ));
    }
    if cfg!(target_os = "android")
        && !fail::extension_removed(XR_KHR_ANDROID_CREATE_INSTANCE_EXTENSION_NAME)
    {
        list.push((
            XR_KHR_ANDROID_CREATE_INSTANCE_EXTENSION_NAME,
            XR_KHR_ANDROID_CREATE_INSTANCE_SPEC_VERSION,
        ));
    }
    list
}

/// The two-call idiom: the count, then (with room) the elements.
///
/// # Safety
///
/// `count` is null or writable; `write` writes the element `i` of the
/// caller's array of `capacity`.
unsafe fn two_call(
    capacity: u32,
    count: *mut u32,
    n: usize,
    mut write: impl FnMut(usize) -> Result<(), XrResult>,
) -> XrResult {
    if count.is_null() {
        return XR_ERROR_VALIDATION_FAILURE;
    }
    // SAFETY: the caller's.
    unsafe { *count = n as u32 };
    if capacity == 0 {
        return XR_SUCCESS;
    }
    if (capacity as usize) < n {
        return XR_ERROR_SIZE_INSUFFICIENT;
    }
    for i in 0..n {
        if let Err(result) = write(i) {
            return result;
        }
    }
    XR_SUCCESS
}

/// Whether an optional input structure is null or of the type.
///
/// # Safety
///
/// `info` is null or points to a structure.
unsafe fn optional_type(info: *const c_void, ty: XrStructureType) -> bool {
    // SAFETY: the caller's; every structure starts with its type.
    info.is_null() || unsafe { *info.cast::<XrStructureType>() } == ty
}

/// Whether a required structure is there and of the type.
///
/// # Safety
///
/// `info` is null or points to a structure.
unsafe fn required_type(info: *const c_void, ty: XrStructureType) -> bool {
    // SAFETY: the caller's.
    !info.is_null() && unsafe { optional_type(info, ty) }
}

/// Return the injected failure of `point`, if any.
macro_rules! inject {
    ($point:expr) => {
        if let Some(result) = fail::xr_call($point) {
            return result;
        }
    };
}

/// Return the error of a `Result<T, XrResult>`, else its value.
macro_rules! tri {
    ($e:expr) => {
        match $e {
            Ok(v) => v,
            Err(result) => return result,
        }
    };
}

/// Return `XR_ERROR_VALIDATION_FAILURE` unless the condition holds.
macro_rules! valid {
    ($cond:expr) => {
        if !$cond {
            return XR_ERROR_VALIDATION_FAILURE;
        }
    };
}

// Instances

pub unsafe extern "system" fn enumerate_api_layer_properties(
    capacity: u32,
    count: *mut u32,
    _properties: *mut XrApiLayerProperties,
) -> XrResult {
    inject!("xrEnumerateApiLayerProperties");
    // (on Android, in an app's process, after xrInitializeLoaderKHR)
    #[cfg(target_os = "android")]
    if !crate::android::global_functions_allowed() {
        return XR_ERROR_INITIALIZATION_FAILED;
    }
    // (no layers: called only when the runtime is loaded in the loader's
    // place, the loader must not call the runtime for it)
    // SAFETY: the application's count.
    unsafe { two_call(capacity, count, 0, |_| Ok(())) }
}

pub unsafe extern "system" fn enumerate_instance_extension_properties(
    layer_name: *const c_char,
    capacity: u32,
    count: *mut u32,
    properties: *mut XrExtensionProperties,
) -> XrResult {
    inject!("xrEnumerateInstanceExtensionProperties");
    // (on Android, in an app's process, after xrInitializeLoaderKHR)
    #[cfg(target_os = "android")]
    if !crate::android::global_functions_allowed() {
        return XR_ERROR_INITIALIZATION_FAILED;
    }
    if !layer_name.is_null() {
        return XR_ERROR_API_LAYER_NOT_PRESENT;
    }
    let list = extensions();
    // SAFETY: the application's count and array of `capacity`.
    unsafe {
        two_call(capacity, count, list.len(), |i| {
            let p = &mut *properties.add(i);
            if p.ty != XR_TYPE_EXTENSION_PROPERTIES {
                return Err(XR_ERROR_VALIDATION_FAILURE);
            }
            set_c_text(&mut p.extension_name, list[i].0);
            p.extension_version = list[i].1;
            Ok(())
        })
    }
}

pub unsafe extern "system" fn create_instance(
    create_info: *const XrInstanceCreateInfo,
    instance: *mut XrInstance,
) -> XrResult {
    inject!("xrCreateInstance");
    // (on Android, in an app's process, after xrInitializeLoaderKHR)
    #[cfg(target_os = "android")]
    if !crate::android::global_functions_allowed() {
        return XR_ERROR_INITIALIZATION_FAILED;
    }
    // SAFETY: the application's structure.
    valid!(unsafe { required_type(create_info.cast(), XR_TYPE_INSTANCE_CREATE_INFO) });
    valid!(!instance.is_null());
    // SAFETY: checked above.
    let info = unsafe { &*create_info };
    valid!(info.create_flags == 0);
    if c_text(&info.application_info.application_name).is_empty() {
        return XR_ERROR_NAME_INVALID;
    }
    if xr_major_minor(info.application_info.api_version) != xr_make_version(1, 0, 0) {
        return XR_ERROR_API_VERSION_UNSUPPORTED;
    }
    let mut rt = runtime();
    // (through the loader, the layers are the loader's; loaded in its
    // place, the runtime has none)
    if !rt.negotiated && info.enabled_api_layer_count > 0 {
        return XR_ERROR_API_LAYER_NOT_PRESENT;
    }
    let available = extensions();
    let mut vulkan_enable2 = false;
    let mut d3d12_enable = false;
    #[cfg(target_os = "android")]
    let mut android_create_instance = false;
    for k in 0..info.enabled_extension_count as usize {
        valid!(!info.enabled_extension_names.is_null());
        // SAFETY: the application's array of names.
        let name = unsafe { *info.enabled_extension_names.add(k) };
        valid!(!name.is_null());
        // SAFETY: a NUL-terminated name.
        let name = unsafe { CStr::from_ptr(name) }.to_bytes();
        if !available.iter().any(|(e, _)| e.as_bytes() == name) {
            return XR_ERROR_EXTENSION_NOT_PRESENT;
        }
        vulkan_enable2 |= name == XR_KHR_VULKAN_ENABLE2_EXTENSION_NAME.as_bytes();
        d3d12_enable |= name == XR_KHR_D3D12_ENABLE_EXTENSION_NAME.as_bytes();
        #[cfg(target_os = "android")]
        {
            android_create_instance |=
                name == XR_KHR_ANDROID_CREATE_INSTANCE_EXTENSION_NAME.as_bytes();
        }
    }
    // (the app's Java VM and activity, which an Android runtime must have)
    #[cfg(target_os = "android")]
    // SAFETY: the application's chain.
    if let Err(result) =
        unsafe { crate::android::check_instance_create_info(info.next, android_create_instance) }
    {
        return result;
    }
    let handle = rt.handle();
    rt.instances.insert(
        handle,
        Instance {
            vulkan_enable2,
            d3d12_enable,
            system: false,
            requirements: false,
            d3d12_luid: None,
            vulkan: Vec::new(),
            events: VecDeque::new(),
            session: None,
        },
    );
    // SAFETY: checked above.
    unsafe { *instance = handle };
    XR_SUCCESS
}

pub unsafe extern "system" fn destroy_instance(instance: XrInstance) -> XrResult {
    inject!("xrDestroyInstance");
    let mut rt = runtime();
    let session = tri!(rt.instance(instance)).session;
    if let Some(session) = session {
        rt.destroy_session(session);
    }
    rt.instances.remove(&instance);
    XR_SUCCESS
}

pub unsafe extern "system" fn get_system(
    instance: XrInstance,
    get_info: *const XrSystemGetInfo,
    system_id: *mut XrSystemId,
) -> XrResult {
    inject!("xrGetSystem");
    let mut rt = runtime();
    let i = tri!(rt.instance(instance));
    // SAFETY: the application's structure.
    valid!(unsafe { required_type(get_info.cast(), XR_TYPE_SYSTEM_GET_INFO) });
    valid!(!system_id.is_null());
    // SAFETY: checked above.
    match unsafe { (*get_info).form_factor } {
        XR_FORM_FACTOR_HEAD_MOUNTED_DISPLAY => {
            i.system = true;
            // SAFETY: checked above.
            unsafe { *system_id = SYSTEM_ID };
            XR_SUCCESS
        }
        XR_FORM_FACTOR_HANDHELD_DISPLAY => XR_ERROR_FORM_FACTOR_UNSUPPORTED,
        _ => XR_ERROR_VALIDATION_FAILURE,
    }
}

pub unsafe extern "system" fn poll_event(
    instance: XrInstance,
    event_data: *mut XrEventDataBuffer,
) -> XrResult {
    inject!("xrPollEvent");
    let mut rt = runtime();
    let i = tri!(rt.instance(instance));
    // SAFETY: the application's structure.
    valid!(unsafe { required_type(event_data.cast(), XR_TYPE_EVENT_DATA_BUFFER) });
    let Some((session, state, time)) = i.events.pop_front() else {
        return XR_EVENT_UNAVAILABLE;
    };
    let event = XrEventDataSessionStateChanged {
        ty: XR_TYPE_EVENT_DATA_SESSION_STATE_CHANGED,
        next: std::ptr::null(),
        session,
        state,
        time,
    };
    // SAFETY: the buffer is larger than the event.
    unsafe {
        event_data
            .cast::<XrEventDataSessionStateChanged>()
            .write(event)
    };
    XR_SUCCESS
}

// XR_KHR_vulkan_enable2

pub unsafe extern "system" fn get_vulkan_graphics_requirements2(
    instance: XrInstance,
    system_id: XrSystemId,
    requirements: *mut XrGraphicsRequirementsVulkan2KHR,
) -> XrResult {
    inject!("xrGetVulkanGraphicsRequirements2KHR");
    let mut rt = runtime();
    let i = tri!(rt.system(instance, system_id));
    // SAFETY: the application's structure.
    valid!(unsafe {
        required_type(
            requirements.cast(),
            XR_TYPE_GRAPHICS_REQUIREMENTS_VULKAN2_KHR,
        )
    });
    i.requirements = true;
    // SAFETY: checked above.
    unsafe {
        (*requirements).min_api_version_supported = xr_make_version(1, 0, 0);
        (*requirements).max_api_version_supported = xr_make_version(1, 3, 0);
    }
    XR_SUCCESS
}

pub unsafe extern "system" fn create_vulkan_instance(
    instance: XrInstance,
    create_info: *const XrVulkanInstanceCreateInfoKHR,
    vulkan_instance: *mut VkInstance,
    vulkan_result: *mut VkResult,
) -> XrResult {
    let injected = fail::call("xrCreateVulkanInstanceKHR");
    if let Some(Outcome::Xr(result)) = injected {
        return result;
    }
    let mut rt = runtime();
    // SAFETY: the application's structure.
    valid!(unsafe { required_type(create_info.cast(), XR_TYPE_VULKAN_INSTANCE_CREATE_INFO_KHR) });
    // SAFETY: checked above.
    let info = unsafe { &*create_info };
    let i = tri!(rt.system(instance, info.system_id));
    valid!(info.create_flags == 0 && !info.vulkan_create_info.is_null());
    valid!(!vulkan_instance.is_null() && !vulkan_result.is_null());
    let Some(gipa) = info.pfn_get_instance_proc_addr else {
        return XR_ERROR_VALIDATION_FAILURE;
    };
    if let Some(Outcome::Vk(result)) = injected {
        // SAFETY: checked above.
        unsafe { *vulkan_result = result };
        return XR_SUCCESS;
    }
    let mut made = null_mut();
    // SAFETY: the application's vkGetInstanceProcAddr and create info.
    let Some(result) = (unsafe {
        vulkan::create_instance(
            gipa,
            info.vulkan_create_info,
            info.vulkan_allocator,
            &mut made,
        )
    }) else {
        return XR_ERROR_RUNTIME_FAILURE;
    };
    // SAFETY: checked above.
    unsafe { *vulkan_result = result };
    if result == VK_SUCCESS {
        i.vulkan.push(VulkanInstance {
            instance: made as usize,
            gipa,
            physical_device: 0,
            devices: Vec::new(),
        });
        // SAFETY: checked above.
        unsafe { *vulkan_instance = made };
    }
    XR_SUCCESS
}

pub unsafe extern "system" fn get_vulkan_graphics_device2(
    instance: XrInstance,
    get_info: *const XrVulkanGraphicsDeviceGetInfoKHR,
    vulkan_physical_device: *mut VkPhysicalDevice,
) -> XrResult {
    inject!("xrGetVulkanGraphicsDevice2KHR");
    let mut rt = runtime();
    // SAFETY: the application's structure.
    valid!(unsafe { required_type(get_info.cast(), XR_TYPE_VULKAN_GRAPHICS_DEVICE_GET_INFO_KHR,) });
    valid!(!vulkan_physical_device.is_null());
    // SAFETY: checked above.
    let info = unsafe { &*get_info };
    let i = tri!(rt.system(instance, info.system_id));
    let Some(v) = i
        .vulkan
        .iter_mut()
        .find(|v| v.instance == info.vulkan_instance as usize)
    else {
        return XR_ERROR_HANDLE_INVALID;
    };
    // SAFETY: an instance made through the application's function.
    let Some(physical_device) =
        (unsafe { vulkan::first_physical_device(v.gipa, v.instance as VkInstance) })
    else {
        return XR_ERROR_RUNTIME_FAILURE;
    };
    v.physical_device = physical_device as usize;
    // SAFETY: checked above.
    unsafe { *vulkan_physical_device = physical_device };
    XR_SUCCESS
}

pub unsafe extern "system" fn create_vulkan_device(
    instance: XrInstance,
    create_info: *const XrVulkanDeviceCreateInfoKHR,
    vulkan_device: *mut VkDevice,
    vulkan_result: *mut VkResult,
) -> XrResult {
    let injected = fail::call("xrCreateVulkanDeviceKHR");
    if let Some(Outcome::Xr(result)) = injected {
        return result;
    }
    let mut rt = runtime();
    // SAFETY: the application's structure.
    valid!(unsafe { required_type(create_info.cast(), XR_TYPE_VULKAN_DEVICE_CREATE_INFO_KHR) });
    // SAFETY: checked above.
    let info = unsafe { &*create_info };
    let i = tri!(rt.system(instance, info.system_id));
    valid!(info.create_flags == 0 && !info.vulkan_create_info.is_null());
    valid!(!vulkan_device.is_null() && !vulkan_result.is_null());
    let Some(gipa) = info.pfn_get_instance_proc_addr else {
        return XR_ERROR_VALIDATION_FAILURE;
    };
    // (the physical device must be xrGetVulkanGraphicsDevice2KHR's)
    let physical_device = info.vulkan_physical_device as usize;
    let Some(v) = i
        .vulkan
        .iter_mut()
        .find(|v| v.physical_device != 0 && v.physical_device == physical_device)
    else {
        return XR_ERROR_HANDLE_INVALID;
    };
    if let Some(Outcome::Vk(result)) = injected {
        // SAFETY: checked above.
        unsafe { *vulkan_result = result };
        return XR_SUCCESS;
    }
    let mut device = null_mut();
    // SAFETY: the application's vkGetInstanceProcAddr and create info,
    // the instance's physical device.
    let Some(result) = (unsafe {
        vulkan::create_device(
            gipa,
            v.instance as VkInstance,
            info.vulkan_physical_device,
            info.vulkan_create_info,
            info.vulkan_allocator,
            &mut device,
        )
    }) else {
        return XR_ERROR_RUNTIME_FAILURE;
    };
    // SAFETY: checked above.
    unsafe { *vulkan_result = result };
    if result == VK_SUCCESS {
        v.devices.push(device as usize);
        // SAFETY: checked above.
        unsafe { *vulkan_device = device };
    }
    XR_SUCCESS
}

// XR_KHR_D3D12_enable

/// A LUID no adapter has (`xrGetD3D12GraphicsRequirementsKHR:luid`).
#[cfg(windows)]
pub const NO_ADAPTER_LUID: Luid = Luid {
    low_part: 0xffff_fff0,
    high_part: 0x7fff_fff0,
};

#[cfg(windows)]
pub unsafe extern "system" fn get_d3d12_graphics_requirements(
    instance: XrInstance,
    system_id: XrSystemId,
    requirements: *mut XrGraphicsRequirementsD3D12KHR,
) -> XrResult {
    inject!("xrGetD3D12GraphicsRequirementsKHR");
    let mut rt = runtime();
    let i = tri!(rt.system(instance, system_id));
    // SAFETY: the application's structure.
    valid!(unsafe { required_type(requirements.cast(), XR_TYPE_GRAPHICS_REQUIREMENTS_D3D12_KHR) });
    let Some(luid) = d3d12::adapter_luid() else {
        return XR_ERROR_RUNTIME_FAILURE;
    };
    // (the requirements' call made to give a LUID no adapter has)
    let luid = if fail::call("xrGetD3D12GraphicsRequirementsKHR:luid").is_some() {
        NO_ADAPTER_LUID
    } else {
        luid
    };
    i.d3d12_luid = Some(luid);
    // SAFETY: checked above.
    unsafe {
        (*requirements).adapter_luid = luid;
        (*requirements).min_feature_level = D3D_FEATURE_LEVEL_11_0;
    }
    XR_SUCCESS
}

// Sessions

pub unsafe extern "system" fn create_session(
    instance: XrInstance,
    create_info: *const XrSessionCreateInfo,
    session: *mut XrSession,
) -> XrResult {
    inject!("xrCreateSession");
    let mut rt = runtime();
    tri!(rt.instance(instance));
    // SAFETY: the application's structure.
    valid!(unsafe { required_type(create_info.cast(), XR_TYPE_SESSION_CREATE_INFO) });
    valid!(!session.is_null());
    // SAFETY: checked above.
    let info = unsafe { &*create_info };
    valid!(info.create_flags == 0);
    let i = tri!(rt.system(instance, info.system_id));
    if !i.requirements && i.d3d12_luid.is_none() {
        return XR_ERROR_GRAPHICS_REQUIREMENTS_CALL_MISSING;
    }
    // The graphics binding, in the chain (of an enabled extension)
    let mut next = info.next;
    let header = loop {
        if next.is_null() {
            return XR_ERROR_GRAPHICS_DEVICE_INVALID;
        }
        // SAFETY: the application's chain of structures.
        let header = unsafe { &*next };
        if (header.ty == XR_TYPE_GRAPHICS_BINDING_VULKAN2_KHR && i.vulkan_enable2)
            || (header.ty == XR_TYPE_GRAPHICS_BINDING_D3D12_KHR && i.d3d12_enable)
        {
            break next;
        }
        next = header.next;
    };
    // SAFETY: a structure of the binding's type.
    let graphics = match unsafe { (*header).ty } {
        XR_TYPE_GRAPHICS_BINDING_VULKAN2_KHR => {
            if !i.requirements {
                return XR_ERROR_GRAPHICS_REQUIREMENTS_CALL_MISSING;
            }
            // SAFETY: a structure of that type.
            let binding = unsafe { &*header.cast::<XrGraphicsBindingVulkan2KHR>() };
            // (made by xrCreateVulkanInstanceKHR, xrGetVulkanGraphicsDevice2KHR's,
            // made by xrCreateVulkanDeviceKHR)
            let Some(v) = i.vulkan.iter().find(|v| {
                v.instance == binding.instance as usize
                    && v.physical_device == binding.physical_device as usize
                    && v.devices.contains(&(binding.device as usize))
            }) else {
                return XR_ERROR_GRAPHICS_DEVICE_INVALID;
            };
            if i.session.is_some() {
                return XR_ERROR_LIMIT_REACHED;
            }
            // SAFETY: the device made from that instance and physical device.
            let Some(device) = (unsafe {
                DeviceFns::load(
                    v.gipa,
                    binding.instance,
                    binding.physical_device,
                    binding.device,
                )
            }) else {
                return XR_ERROR_RUNTIME_FAILURE;
            };
            Graphics::Vulkan(device)
        }
        #[cfg(windows)]
        _ => {
            let Some(luid) = i.d3d12_luid else {
                return XR_ERROR_GRAPHICS_REQUIREMENTS_CALL_MISSING;
            };
            // SAFETY: a structure of that type.
            let binding = unsafe { &*header.cast::<XrGraphicsBindingD3D12KHR>() };
            // (the application's device and queue, on the requirements'
            // adapter)
            // SAFETY: the application's device, alive while the session is.
            let Some(device) = (unsafe { d3d12::Device::new(binding.device) }) else {
                return XR_ERROR_GRAPHICS_DEVICE_INVALID;
            };
            if binding.queue.is_null() || device.adapter_luid() != luid {
                return XR_ERROR_GRAPHICS_DEVICE_INVALID;
            }
            if i.session.is_some() {
                return XR_ERROR_LIMIT_REACHED;
            }
            Graphics::D3D12(device)
        }
        #[cfg(not(windows))]
        _ => return XR_ERROR_GRAPHICS_DEVICE_INVALID,
    };
    let handle = rt.handle();
    rt.sessions.insert(
        handle,
        Session {
            instance,
            graphics,
            state: 0,
            running: false,
            exit_requested: false,
            waited: 0,
            begun: false,
            time: START_TIME,
        },
    );
    if let Some(i) = rt.instances.get_mut(&instance) {
        i.session = Some(handle);
    }
    rt.set_state(handle, XR_SESSION_STATE_IDLE);
    rt.set_state(handle, XR_SESSION_STATE_READY);
    // SAFETY: checked above.
    unsafe { *session = handle };
    XR_SUCCESS
}

pub unsafe extern "system" fn destroy_session(session: XrSession) -> XrResult {
    inject!("xrDestroySession");
    let mut rt = runtime();
    tri!(rt.session(session));
    rt.destroy_session(session);
    XR_SUCCESS
}

pub unsafe extern "system" fn begin_session(
    session: XrSession,
    begin_info: *const XrSessionBeginInfo,
) -> XrResult {
    inject!("xrBeginSession");
    let mut rt = runtime();
    let s = tri!(rt.session(session));
    // SAFETY: the application's structure.
    valid!(unsafe { required_type(begin_info.cast(), XR_TYPE_SESSION_BEGIN_INFO) });
    if s.running {
        return XR_ERROR_SESSION_RUNNING;
    }
    if s.state != XR_SESSION_STATE_READY {
        return XR_ERROR_SESSION_NOT_READY;
    }
    // SAFETY: checked above.
    if unsafe { (*begin_info).primary_view_configuration_type }
        != XR_VIEW_CONFIGURATION_TYPE_PRIMARY_STEREO
    {
        return XR_ERROR_VIEW_CONFIGURATION_TYPE_UNSUPPORTED;
    }
    s.running = true;
    s.waited = 0;
    s.begun = false;
    rt.set_state(session, XR_SESSION_STATE_SYNCHRONIZED);
    XR_SUCCESS
}

pub unsafe extern "system" fn end_session(session: XrSession) -> XrResult {
    inject!("xrEndSession");
    let mut rt = runtime();
    let s = tri!(rt.session(session));
    if !s.running {
        return XR_ERROR_SESSION_NOT_RUNNING;
    }
    // (always not running after it)
    s.running = false;
    if s.state != XR_SESSION_STATE_STOPPING {
        return XR_ERROR_SESSION_NOT_STOPPING;
    }
    let exit = s.exit_requested;
    rt.set_state(session, XR_SESSION_STATE_IDLE);
    if exit {
        rt.set_state(session, XR_SESSION_STATE_EXITING);
    }
    XR_SUCCESS
}

pub unsafe extern "system" fn request_exit_session(session: XrSession) -> XrResult {
    inject!("xrRequestExitSession");
    let mut rt = runtime();
    let s = tri!(rt.session(session));
    if !s.running {
        return XR_ERROR_SESSION_NOT_RUNNING;
    }
    s.exit_requested = true;
    let state = s.state;
    if state == XR_SESSION_STATE_FOCUSED {
        rt.set_state(session, XR_SESSION_STATE_VISIBLE);
    }
    if (XR_SESSION_STATE_VISIBLE..=XR_SESSION_STATE_FOCUSED).contains(&state) {
        rt.set_state(session, XR_SESSION_STATE_SYNCHRONIZED);
    }
    if state != XR_SESSION_STATE_STOPPING {
        rt.set_state(session, XR_SESSION_STATE_STOPPING);
    }
    XR_SUCCESS
}

// Swapchains

pub unsafe extern "system" fn enumerate_swapchain_formats(
    session: XrSession,
    capacity: u32,
    count: *mut u32,
    formats: *mut i64,
) -> XrResult {
    inject!("xrEnumerateSwapchainFormats");
    let mut rt = runtime();
    let list = tri!(rt.session(session)).graphics.formats();
    // SAFETY: the application's count and array of `capacity`.
    unsafe {
        two_call(capacity, count, list.len(), |i| {
            *formats.add(i) = list[i];
            Ok(())
        })
    }
}

pub unsafe extern "system" fn create_swapchain(
    session: XrSession,
    create_info: *const XrSwapchainCreateInfo,
    swapchain: *mut XrSwapchain,
) -> XrResult {
    inject!("xrCreateSwapchain");
    let mut rt = runtime();
    let graphics = tri!(rt.session(session)).graphics;
    // SAFETY: the application's structure.
    valid!(unsafe { required_type(create_info.cast(), XR_TYPE_SWAPCHAIN_CREATE_INFO) });
    valid!(!swapchain.is_null());
    // SAFETY: checked above.
    let info = unsafe { &*create_info };
    if info.create_flags & XR_SWAPCHAIN_CREATE_PROTECTED_CONTENT_BIT != 0 {
        return XR_ERROR_FEATURE_UNSUPPORTED;
    }
    valid!(
        info.create_flags
            & !(XR_SWAPCHAIN_CREATE_PROTECTED_CONTENT_BIT | XR_SWAPCHAIN_CREATE_STATIC_IMAGE_BIT)
            == 0
    );
    if !graphics.formats().contains(&info.format) {
        return XR_ERROR_SWAPCHAIN_FORMAT_UNSUPPORTED;
    }
    // (no multisampling, no cube maps)
    if info.sample_count != 1 || info.face_count != 1 {
        return XR_ERROR_FEATURE_UNSUPPORTED;
    }
    let size_ok = |n: u32| (1..=MAX_SWAPCHAIN_SIZE).contains(&n);
    valid!(size_ok(info.width) && size_ok(info.height) && size_ok(info.array_size));
    valid!(info.mip_count >= 1 && info.mip_count <= 32);
    let is_static = info.create_flags & XR_SWAPCHAIN_CREATE_STATIC_IMAGE_BIT != 0;
    let count = if is_static { 1 } else { IMAGE_COUNT };
    // SAFETY: the session's device; a create info of valid values.
    let Some(images) = (unsafe { create_images(&graphics, info, count) }) else {
        return XR_ERROR_RUNTIME_FAILURE;
    };
    let handle = rt.handle();
    rt.swapchains.insert(
        handle,
        Swapchain {
            session,
            images,
            array_size: info.array_size,
            is_static,
            acquired: VecDeque::new(),
            next: 0,
            acquisitions: 0,
            released: false,
        },
    );
    // SAFETY: checked above.
    unsafe { *swapchain = handle };
    XR_SUCCESS
}

/// A swapchain's images, made on the session's device, or `None` (with
/// nothing left).
///
/// # Safety
///
/// The device is alive; the create info's values are valid for it.
unsafe fn create_images(
    graphics: &Graphics,
    info: &XrSwapchainCreateInfo,
    count: u32,
) -> Option<Vec<Image>> {
    let mut images = Vec::new();
    for _ in 0..count {
        // SAFETY: the caller's.
        let image = unsafe {
            match graphics {
                Graphics::Vulkan(device) => vulkan_image(device, info),
                #[cfg(windows)]
                Graphics::D3D12(device) => device
                    .create_image(info)
                    .map(|resource| Image::D3D12(resource as usize)),
            }
        };
        match image {
            Some(image) => images.push(image),
            None => {
                for image in images {
                    // SAFETY: the images just made.
                    unsafe { graphics.destroy_image(image) };
                }
                return None;
            }
        }
    }
    Some(images)
}

/// A Vulkan swapchain image.
///
/// # Safety
///
/// As [`create_images`].
unsafe fn vulkan_image(device: &DeviceFns, info: &XrSwapchainCreateInfo) -> Option<Image> {
    let (usage, flags) = vulkan::image_usage(info.usage_flags);
    let image_info = VkImageCreateInfo {
        s_type: VK_STRUCTURE_TYPE_IMAGE_CREATE_INFO,
        p_next: std::ptr::null(),
        flags,
        image_type: VK_IMAGE_TYPE_2D,
        format: info.format as i32,
        extent: VkExtent3D {
            width: info.width,
            height: info.height,
            depth: 1,
        },
        mip_levels: info.mip_count,
        array_layers: info.array_size,
        samples: VK_SAMPLE_COUNT_1_BIT,
        tiling: VK_IMAGE_TILING_OPTIMAL,
        usage,
        sharing_mode: VK_SHARING_MODE_EXCLUSIVE,
        queue_family_index_count: 0,
        p_queue_family_indices: std::ptr::null(),
        initial_layout: VK_IMAGE_LAYOUT_UNDEFINED,
    };
    // SAFETY: the caller's.
    unsafe { device.create_image(&image_info) }.map(|(image, memory)| Image::Vulkan(image, memory))
}

pub unsafe extern "system" fn destroy_swapchain(swapchain: XrSwapchain) -> XrResult {
    inject!("xrDestroySwapchain");
    let mut rt = runtime();
    if !rt.swapchains.contains_key(&swapchain) {
        return XR_ERROR_HANDLE_INVALID;
    }
    rt.destroy_swapchain(swapchain);
    XR_SUCCESS
}

pub unsafe extern "system" fn enumerate_swapchain_images(
    swapchain: XrSwapchain,
    capacity: u32,
    count: *mut u32,
    images: *mut c_void,
) -> XrResult {
    inject!("xrEnumerateSwapchainImages");
    let rt = runtime();
    let Some(sc) = rt.swapchains.get(&swapchain) else {
        return XR_ERROR_HANDLE_INVALID;
    };
    // SAFETY: the application's count and array of `capacity` (of the
    // session's graphics API's structure).
    unsafe {
        two_call(capacity, count, sc.images.len(), |i| match sc.images[i] {
            Image::Vulkan(vk_image, _) => {
                let image = &mut *images.cast::<XrSwapchainImageVulkan2KHR>().add(i);
                if image.ty != XR_TYPE_SWAPCHAIN_IMAGE_VULKAN2_KHR {
                    return Err(XR_ERROR_VALIDATION_FAILURE);
                }
                image.image = vk_image;
                Ok(())
            }
            #[cfg(windows)]
            Image::D3D12(resource) => {
                let image = &mut *images.cast::<XrSwapchainImageD3D12KHR>().add(i);
                if image.ty != XR_TYPE_SWAPCHAIN_IMAGE_D3D12_KHR {
                    return Err(XR_ERROR_VALIDATION_FAILURE);
                }
                image.texture = resource as *mut c_void;
                Ok(())
            }
        })
    }
}

pub unsafe extern "system" fn acquire_swapchain_image(
    swapchain: XrSwapchain,
    acquire_info: *const c_void,
    index: *mut u32,
) -> XrResult {
    inject!("xrAcquireSwapchainImage");
    let mut rt = runtime();
    let Some(sc) = rt.swapchains.get_mut(&swapchain) else {
        return XR_ERROR_HANDLE_INVALID;
    };
    // SAFETY: the application's structure.
    valid!(unsafe { optional_type(acquire_info, XR_TYPE_SWAPCHAIN_IMAGE_ACQUIRE_INFO) });
    valid!(!index.is_null());
    let n = sc.images.len() as u32;
    if sc.acquired.len() as u32 == n || (sc.is_static && sc.acquisitions > 0) {
        return XR_ERROR_CALL_ORDER_INVALID;
    }
    let i = sc.next;
    sc.next = (sc.next + 1) % n;
    sc.acquisitions += 1;
    sc.acquired.push_back((i, false));
    // SAFETY: checked above.
    unsafe { *index = i };
    XR_SUCCESS
}

pub unsafe extern "system" fn wait_swapchain_image(
    swapchain: XrSwapchain,
    wait_info: *const XrSwapchainImageWaitInfo,
) -> XrResult {
    inject!("xrWaitSwapchainImage");
    let mut rt = runtime();
    let Some(sc) = rt.swapchains.get_mut(&swapchain) else {
        return XR_ERROR_HANDLE_INVALID;
    };
    // SAFETY: the application's structure.
    valid!(unsafe { required_type(wait_info.cast(), XR_TYPE_SWAPCHAIN_IMAGE_WAIT_INFO) });
    // (the oldest acquired image, once)
    match sc.acquired.front_mut() {
        Some((_, waited)) if !*waited => {
            *waited = true;
            XR_SUCCESS
        }
        _ => XR_ERROR_CALL_ORDER_INVALID,
    }
}

pub unsafe extern "system" fn release_swapchain_image(
    swapchain: XrSwapchain,
    release_info: *const c_void,
) -> XrResult {
    inject!("xrReleaseSwapchainImage");
    let mut rt = runtime();
    let Some(sc) = rt.swapchains.get_mut(&swapchain) else {
        return XR_ERROR_HANDLE_INVALID;
    };
    // SAFETY: the application's structure.
    valid!(unsafe { optional_type(release_info, XR_TYPE_SWAPCHAIN_IMAGE_RELEASE_INFO) });
    match sc.acquired.front() {
        Some(&(_, true)) => {
            sc.acquired.pop_front();
            sc.released = true;
            XR_SUCCESS
        }
        _ => XR_ERROR_CALL_ORDER_INVALID,
    }
}

// Spaces and views

pub unsafe extern "system" fn create_reference_space(
    session: XrSession,
    create_info: *const XrReferenceSpaceCreateInfo,
    space: *mut XrSpace,
) -> XrResult {
    inject!("xrCreateReferenceSpace");
    let mut rt = runtime();
    tri!(rt.session(session));
    // SAFETY: the application's structure.
    valid!(unsafe { required_type(create_info.cast(), XR_TYPE_REFERENCE_SPACE_CREATE_INFO) });
    valid!(!space.is_null());
    // SAFETY: checked above.
    let info = unsafe { &*create_info };
    if !matches!(
        info.reference_space_type,
        XR_REFERENCE_SPACE_TYPE_VIEW
            | XR_REFERENCE_SPACE_TYPE_LOCAL
            | XR_REFERENCE_SPACE_TYPE_STAGE
    ) {
        return XR_ERROR_REFERENCE_SPACE_UNSUPPORTED;
    }
    let q = info.pose_in_reference_space.orientation;
    let norm = q.iter().map(|c| c * c).sum::<f32>();
    if (norm - 1.0).abs() > 1e-3 {
        return XR_ERROR_POSE_INVALID;
    }
    let handle = rt.handle();
    rt.spaces.insert(handle, Space { session });
    // SAFETY: checked above.
    unsafe { *space = handle };
    XR_SUCCESS
}

pub unsafe extern "system" fn destroy_space(space: XrSpace) -> XrResult {
    inject!("xrDestroySpace");
    let mut rt = runtime();
    match rt.spaces.remove(&space) {
        Some(_) => XR_SUCCESS,
        None => XR_ERROR_HANDLE_INVALID,
    }
}

pub unsafe extern "system" fn locate_views(
    session: XrSession,
    locate_info: *const XrViewLocateInfo,
    view_state: *mut XrViewState,
    capacity: u32,
    count: *mut u32,
    views: *mut XrView,
) -> XrResult {
    inject!("xrLocateViews");
    let mut rt = runtime();
    tri!(rt.session(session));
    // SAFETY: the application's structures.
    valid!(unsafe { required_type(locate_info.cast(), XR_TYPE_VIEW_LOCATE_INFO) });
    // SAFETY: the application's structure.
    valid!(unsafe { required_type(view_state.cast_const().cast(), XR_TYPE_VIEW_STATE) });
    // SAFETY: checked above.
    let info = unsafe { &*locate_info };
    if info.view_configuration_type != XR_VIEW_CONFIGURATION_TYPE_PRIMARY_STEREO {
        return XR_ERROR_VIEW_CONFIGURATION_TYPE_UNSUPPORTED;
    }
    if rt
        .spaces
        .get(&info.space)
        .is_none_or(|s| s.session != session)
    {
        return XR_ERROR_HANDLE_INVALID;
    }
    // (the eyes 64 mm apart, 90 degrees of view each way)
    let fov = std::f32::consts::FRAC_PI_4;
    // SAFETY: the application's count, structure and array of `capacity`.
    unsafe {
        let result = two_call(capacity, count, 2, |eye| {
            let view = &mut *views.add(eye);
            if view.ty != XR_TYPE_VIEW {
                return Err(XR_ERROR_VALIDATION_FAILURE);
            }
            view.pose = XrPosef {
                orientation: [0.0, 0.0, 0.0, 1.0],
                position: [if eye == 0 { -0.032 } else { 0.032 }, 0.0, 0.0],
            };
            view.fov = XrFovf {
                angle_left: -fov,
                angle_right: fov,
                angle_up: fov,
                angle_down: -fov,
            };
            Ok(())
        });
        if result == XR_SUCCESS {
            (*view_state).view_state_flags = XR_VIEW_STATE_ALL_VALID_AND_TRACKED;
        }
        result
    }
}

// Frames

pub unsafe extern "system" fn wait_frame(
    session: XrSession,
    wait_info: *const c_void,
    frame_state: *mut XrFrameState,
) -> XrResult {
    inject!("xrWaitFrame");
    let mut rt = runtime();
    let s = tri!(rt.session(session));
    // SAFETY: the application's structures.
    valid!(unsafe { optional_type(wait_info, XR_TYPE_FRAME_WAIT_INFO) });
    // SAFETY: the application's structure.
    valid!(unsafe { required_type(frame_state.cast_const().cast(), XR_TYPE_FRAME_STATE) });
    if !s.running {
        return XR_ERROR_SESSION_NOT_RUNNING;
    }
    s.time += FRAME_PERIOD;
    s.waited += 1;
    // SAFETY: checked above.
    unsafe {
        (*frame_state).predicted_display_time = s.time;
        (*frame_state).predicted_display_period = FRAME_PERIOD;
        (*frame_state).should_render = XrBool32::from(matches!(
            s.state,
            XR_SESSION_STATE_VISIBLE | XR_SESSION_STATE_FOCUSED
        ));
    }
    XR_SUCCESS
}

pub unsafe extern "system" fn begin_frame(
    session: XrSession,
    begin_info: *const c_void,
) -> XrResult {
    inject!("xrBeginFrame");
    let mut rt = runtime();
    let s = tri!(rt.session(session));
    // SAFETY: the application's structure.
    valid!(unsafe { optional_type(begin_info, XR_TYPE_FRAME_BEGIN_INFO) });
    if !s.running {
        return XR_ERROR_SESSION_NOT_RUNNING;
    }
    if s.waited == 0 {
        return XR_ERROR_CALL_ORDER_INVALID;
    }
    s.waited -= 1;
    // (a frame begun and not ended is discarded)
    let discarded = s.begun;
    s.begun = true;
    if discarded {
        XR_FRAME_DISCARDED
    } else {
        XR_SUCCESS
    }
}

/// Check a frame's composition layers.
///
/// # Safety
///
/// `info`'s layers are an array of `layer_count` pointers, each null or to
/// a layer structure.
unsafe fn check_layers(rt: &Runtime, session: XrSession, info: &XrFrameEndInfo) -> XrResult {
    if info.layer_count > MAX_LAYERS {
        return XR_ERROR_LAYER_LIMIT_EXCEEDED;
    }
    valid!(info.layer_count == 0 || !info.layers.is_null());
    for k in 0..info.layer_count as usize {
        // SAFETY: the caller's.
        let layer = unsafe { *info.layers.add(k) };
        if layer.is_null() {
            return XR_ERROR_LAYER_INVALID;
        }
        // SAFETY: the caller's.
        let header = unsafe { &*layer };
        if rt
            .spaces
            .get(&header.space)
            .is_none_or(|s| s.session != session)
        {
            return XR_ERROR_HANDLE_INVALID;
        }
        match header.ty {
            XR_TYPE_COMPOSITION_LAYER_PROJECTION => {
                // SAFETY: a structure of that type.
                let projection = unsafe { &*layer.cast::<XrCompositionLayerProjection>() };
                // (the primary stereo view configuration's two views)
                valid!(projection.view_count == 2 && !projection.views.is_null());
                for v in 0..2 {
                    // SAFETY: the layer's array of two views.
                    let view = unsafe { &*projection.views.add(v) };
                    valid!(view.ty == XR_TYPE_COMPOSITION_LAYER_PROJECTION_VIEW);
                    let Some(sc) = rt.swapchains.get(&view.sub_image.swapchain) else {
                        return XR_ERROR_HANDLE_INVALID;
                    };
                    if sc.session != session || !sc.released {
                        return XR_ERROR_LAYER_INVALID;
                    }
                    valid!(view.sub_image.image_array_index < sc.array_size);
                }
            }
            // (quads aren't read)
            XR_TYPE_COMPOSITION_LAYER_QUAD => {}
            _ => return XR_ERROR_LAYER_INVALID,
        }
    }
    XR_SUCCESS
}

pub unsafe extern "system" fn end_frame(
    session: XrSession,
    end_info: *const XrFrameEndInfo,
) -> XrResult {
    inject!("xrEndFrame");
    let mut rt = runtime();
    let s = tri!(rt.session(session));
    // SAFETY: the application's structure.
    valid!(unsafe { required_type(end_info.cast(), XR_TYPE_FRAME_END_INFO) });
    if !s.running {
        return XR_ERROR_SESSION_NOT_RUNNING;
    }
    if !s.begun {
        return XR_ERROR_CALL_ORDER_INVALID;
    }
    // SAFETY: checked above.
    let info = unsafe { &*end_info };
    valid!(info.display_time > 0);
    if info.environment_blend_mode != XR_ENVIRONMENT_BLEND_MODE_OPAQUE {
        return XR_ERROR_ENVIRONMENT_BLEND_MODE_UNSUPPORTED;
    }
    // SAFETY: the application's layers.
    let result = unsafe { check_layers(&rt, session, info) };
    if result != XR_SUCCESS {
        return result;
    }
    let s = tri!(rt.session(session));
    s.begun = false;
    if s.state == XR_SESSION_STATE_SYNCHRONIZED && !s.exit_requested {
        rt.set_state(session, XR_SESSION_STATE_VISIBLE);
        rt.set_state(session, XR_SESSION_STATE_FOCUSED);
    }
    XR_SUCCESS
}
