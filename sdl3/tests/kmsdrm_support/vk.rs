// The bits of Vulkan the KMS/DRM driver's Vulkan programs call themselves.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! Through the `vkGetInstanceProcAddr` of the loader SDL loaded
//! (`SDL_Vulkan_GetVkGetInstanceProcAddr()`): an instance, its physical
//! devices, their properties, displays and planes (`VK_KHR_display`).

use std::ffi::{c_char, c_void, CStr, CString};

pub type VkInstance = *mut c_void;
pub type VkPhysicalDevice = *mut c_void;
type PfnVoid = unsafe extern "system" fn();
type PfnGetInstanceProcAddr =
    unsafe extern "system" fn(VkInstance, *const c_char) -> Option<PfnVoid>;

/// `VkInstanceCreateInfo`.
#[repr(C)]
struct VkInstanceCreateInfo {
    s_type: i32,
    p_next: *const c_void,
    flags: u32,
    p_application_info: *const c_void,
    enabled_layer_count: u32,
    pp_enabled_layer_names: *const *const c_char,
    enabled_extension_count: u32,
    pp_enabled_extension_names: *const *const c_char,
}

/// `VkApplicationInfo`.
#[repr(C)]
struct VkApplicationInfo {
    s_type: i32,
    p_next: *const c_void,
    p_application_name: *const c_char,
    application_version: u32,
    p_engine_name: *const c_char,
    engine_version: u32,
    api_version: u32,
}

/// `VkPhysicalDeviceProperties` (824 bytes), of which the programs read
/// the API version, the device type and the name.
#[repr(C, align(8))]
struct VkPhysicalDeviceProperties([u8; 1024]);

/// `VkSurfaceCapabilitiesKHR`.
#[repr(C)]
#[derive(Clone, Copy, Default, Debug)]
pub struct VkSurfaceCapabilitiesKHR {
    pub min_image_count: u32,
    pub max_image_count: u32,
    pub current_extent: [u32; 2],
    pub min_image_extent: [u32; 2],
    pub max_image_extent: [u32; 2],
    pub max_image_array_layers: u32,
    pub supported_transforms: u32,
    pub current_transform: u32,
    pub supported_composite_alpha: u32,
    pub supported_usage_flags: u32,
}

/// A physical device's properties.
pub struct DeviceProperties {
    pub api_major: u32,
    pub api_minor: u32,
    /// `VkPhysicalDeviceType`: 1 integrated, 2 discrete, 3 virtual, 4 CPU
    pub device_type: u32,
    pub name: String,
}

/// The functions of an instance (or the global ones, for a null one).
pub struct Vk {
    get: PfnGetInstanceProcAddr,
}

impl Vk {
    /// Through the loader SDL loaded.
    pub fn new() -> Vk {
        let get = sdl3::video::vulkan::vulkan_get_vk_get_instance_proc_addr().unwrap();
        // SAFETY: the loader's vkGetInstanceProcAddr.
        Vk {
            get: unsafe { std::mem::transmute::<usize, PfnGetInstanceProcAddr>(get) },
        }
    }

    /// A function of `instance`, as `F` (`None`: NULL).
    ///
    /// # Safety
    ///
    /// `F` must be the function's pointer type.
    pub unsafe fn f<F: Copy>(&self, instance: VkInstance, name: &CStr) -> Option<F> {
        // SAFETY: the caller's contract.
        unsafe {
            let f = (self.get)(instance, name.as_ptr())?;
            Some(std::mem::transmute_copy::<PfnVoid, F>(&f))
        }
    }

    /// `vkCreateInstance()` (Vulkan 1.0) with these extensions: the
    /// instance, or the `VkResult`.
    pub fn create_instance(&self, extensions: &[&str]) -> Result<VkInstance, i32> {
        let names: Vec<CString> = extensions
            .iter()
            .map(|e| CString::new(*e).unwrap())
            .collect();
        let pointers: Vec<*const c_char> = names.iter().map(|n| n.as_ptr()).collect();
        let app = VkApplicationInfo {
            s_type: 0, // VK_STRUCTURE_TYPE_APPLICATION_INFO
            p_next: std::ptr::null(),
            p_application_name: std::ptr::null(),
            application_version: 0,
            p_engine_name: std::ptr::null(),
            engine_version: 0,
            api_version: 1 << 22, // VK_API_VERSION_1_0
        };
        let info = VkInstanceCreateInfo {
            s_type: 1, // VK_STRUCTURE_TYPE_INSTANCE_CREATE_INFO
            p_next: std::ptr::null(),
            flags: 0,
            p_application_info: (&raw const app).cast(),
            enabled_layer_count: 0,
            pp_enabled_layer_names: std::ptr::null(),
            enabled_extension_count: pointers.len() as u32,
            pp_enabled_extension_names: pointers.as_ptr(),
        };
        type Create =
            unsafe extern "system" fn(*const VkInstanceCreateInfo, *const c_void, *mut VkInstance) -> i32;
        let mut instance: VkInstance = std::ptr::null_mut();
        // SAFETY: the global function, with the info above.
        let result = unsafe {
            let create: Create = self
                .f(std::ptr::null_mut(), c"vkCreateInstance")
                .expect("vkCreateInstance");
            create(&info, std::ptr::null(), &mut instance)
        };
        if result == 0 {
            Ok(instance)
        } else {
            Err(result)
        }
    }

    pub fn destroy_instance(&self, instance: VkInstance) {
        type Destroy = unsafe extern "system" fn(VkInstance, *const c_void);
        // SAFETY: the instance's function.
        unsafe {
            let destroy: Destroy = self
                .f(instance, c"vkDestroyInstance")
                .expect("vkDestroyInstance");
            destroy(instance, std::ptr::null());
        }
    }

    /// The physical devices.
    pub fn physical_devices(&self, instance: VkInstance) -> Vec<VkPhysicalDevice> {
        type Enumerate =
            unsafe extern "system" fn(VkInstance, *mut u32, *mut VkPhysicalDevice) -> i32;
        // SAFETY: the instance's function, with room for the count.
        unsafe {
            let enumerate: Enumerate = self
                .f(instance, c"vkEnumeratePhysicalDevices")
                .expect("vkEnumeratePhysicalDevices");
            let mut count = 0;
            enumerate(instance, &mut count, std::ptr::null_mut());
            let mut devices = vec![std::ptr::null_mut(); count as usize];
            enumerate(instance, &mut count, devices.as_mut_ptr());
            devices.truncate(count as usize);
            devices
        }
    }

    /// The device's API version, type and name.
    pub fn device_properties(
        &self,
        instance: VkInstance,
        device: VkPhysicalDevice,
    ) -> DeviceProperties {
        type Get = unsafe extern "system" fn(VkPhysicalDevice, *mut VkPhysicalDeviceProperties);
        let mut props = VkPhysicalDeviceProperties([0; 1024]);
        // SAFETY: the instance's function, with room for the properties.
        unsafe {
            let get: Get = self
                .f(instance, c"vkGetPhysicalDeviceProperties")
                .expect("vkGetPhysicalDeviceProperties");
            get(device, &mut props);
        }
        let api = u32::from_ne_bytes(props.0[0..4].try_into().unwrap());
        DeviceProperties {
            api_major: api >> 22,
            api_minor: (api >> 12) & 0x3ff,
            device_type: u32::from_ne_bytes(props.0[16..20].try_into().unwrap()),
            name: CStr::from_bytes_until_nul(&props.0[20..276])
                .unwrap()
                .to_string_lossy()
                .into_owned(),
        }
    }

    /// The counts of `vkGetPhysicalDeviceDisplayPropertiesKHR()` and
    /// `vkGetPhysicalDeviceDisplayPlanePropertiesKHR()` (`None`: the
    /// instance hasn't `VK_KHR_display`).
    pub fn display_and_plane_counts(
        &self,
        instance: VkInstance,
        device: VkPhysicalDevice,
    ) -> Option<(u32, u32)> {
        type Count = unsafe extern "system" fn(VkPhysicalDevice, *mut u32, *mut c_void) -> i32;
        // SAFETY: the instance's functions: a NULL array asks for the count.
        unsafe {
            let displays: Count = self.f(instance, c"vkGetPhysicalDeviceDisplayPropertiesKHR")?;
            let planes: Count =
                self.f(instance, c"vkGetPhysicalDeviceDisplayPlanePropertiesKHR")?;
            let (mut d, mut p) = (0, 0);
            displays(device, &mut d, std::ptr::null_mut());
            planes(device, &mut p, std::ptr::null_mut());
            Some((d, p))
        }
    }

    /// `vkGetPhysicalDeviceSurfaceCapabilitiesKHR()`: the result and the
    /// capabilities.
    pub fn surface_capabilities(
        &self,
        instance: VkInstance,
        device: VkPhysicalDevice,
        surface: u64,
    ) -> (i32, VkSurfaceCapabilitiesKHR) {
        type Get =
            unsafe extern "system" fn(VkPhysicalDevice, u64, *mut VkSurfaceCapabilitiesKHR) -> i32;
        let mut caps = VkSurfaceCapabilitiesKHR::default();
        // SAFETY: the instance's function.
        let result = unsafe {
            let get: Get = self
                .f(instance, c"vkGetPhysicalDeviceSurfaceCapabilitiesKHR")
                .expect("vkGetPhysicalDeviceSurfaceCapabilitiesKHR");
            get(device, surface, &mut caps)
        };
        (result, caps)
    }
}
