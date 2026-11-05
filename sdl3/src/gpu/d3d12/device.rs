// Rust translation of the device parts of src/gpu/d3d12/SDL_gpu_d3d12.c from
// Simple DirectMedia Layer: PrepareDriver, the debug layers, the blit
// resources, the OpenXR entry points and CreateDevice.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! Device creation: `D3D12_PrepareDriver()` (can this machine create a
//! Direct3D 12 device for the application's shader formats?),
//! `D3D12_CreateDevice()` with the adapter selection, the vendored
//! Direct3D 12 runtime of the Agility SDK, the DXGI and Direct3D 12 debug
//! layers and their message logging, and the blit resources.
//!
//! A device made with OpenXR (`HAVE_GPU_OPENXR`, on every target, as
//! upstream's CMake builds it) has its instance and system made by
//! `SDL_OPENXR_INTERNAL_GPUInitOpenXR()`, and its adapter is the one the
//! runtime asks for (`XR_KHR_D3D12_enable`:
//! `xrGetD3D12GraphicsRequirementsKHR`'s LUID); the OpenXR entry points
//! (`CreateXRSession`...) are here, between PrepareDriver and CreateDevice,
//! where upstream has them.
//!
//! The Xbox paths (`SDL_PLATFORM_XBOXONE`, `SDL_PLATFORM_XBOXSERIES`) are
//! not translated. Upstream also builds this backend on other systems over
//! DXVK's `libdxvk_d3d12.so`; here it is Windows only.

use std::ffi::{c_char, c_void, CStr, CString};
use std::mem::ManuallyDrop;
use std::ptr::{null, null_mut};
use std::sync::atomic::{AtomicI32, AtomicU32};
use std::sync::{Arc, Mutex};

use super::d3d::*;
use super::descriptors::{
    create_descriptor_heap, create_staging_descriptor_pool, GpuDescriptorHeapPool,
};
use super::resources::{
    calc_subresource, ContainerRef, D3D12Sampler, D3D12Texture, PendingDestroys, TextureContainer,
    TextureContainerState, TextureSubresource,
};
use super::shaders::*;
use super::tables::{sdl_to_d3d12_texture_format, SDL_TO_D3D12_TEXTURE_FORMAT};
use super::{
    d3d12_error, lock, set_string_error, BlitResources, D3D12Renderer, LiveObjectReporter,
    WinPixEventRuntimeFns, D3D12_DLL, D3D_FEATURE_LEVEL_CHOICE, D3D_FEATURE_LEVEL_CHOICE_STR,
    DXGIDEBUG_DLL, DXGI_DLL, SAMPLER_GPU_DESCRIPTOR_COUNT, VIEW_GPU_DESCRIPTOR_COUNT,
    WINPIXEVENTRUNTIME_DLL,
};
use crate::core::windows::com::ComPtr;
use crate::core::windows::{is_windows_11_or_greater, wide_to_utf8};
use crate::error::{Error, Result};
use crate::gpu::sysgpu::{BackendObject, BlitPipelineCache};
use crate::gpu::xr::gpu_openxr::{
    c_name as xr_name, destroy_unreturned_xr_instance, gpu_init_openxr, GpuOpenXr,
};
use crate::gpu::xr::openxrdyn::{self, XrInstancePfns};
use crate::gpu::xr::sys::{
    Luid, PfnXrGetD3D12GraphicsRequirementsKHR, XrExtensionProperties, XrGraphicsBindingD3D12KHR,
    XrGraphicsRequirementsD3D12KHR, XrInstance, XrSession, XrSessionCreateInfo, XrSwapchain,
    XrSwapchainCreateInfo, XrSwapchainImageD3D12KHR, XrSystemId,
    XR_KHR_D3D12_ENABLE_EXTENSION_NAME, XR_NULL_HANDLE, XR_NULL_SYSTEM_ID,
    XR_SWAPCHAIN_USAGE_COLOR_ATTACHMENT_BIT, XR_TYPE_EXTENSION_PROPERTIES,
    XR_TYPE_GRAPHICS_BINDING_D3D12_KHR, XR_TYPE_GRAPHICS_REQUIREMENTS_D3D12_KHR,
    XR_TYPE_SWAPCHAIN_IMAGE_D3D12_KHR,
};
use crate::gpu::xr::{XrError, XrResult};
use crate::gpu::{
    CompareOp, Filter, IndexedIndirectDrawCommand, IndirectDispatchCommand, IndirectDrawCommand,
    SampleCount, Sampler, SamplerAddressMode, SamplerCreateInfo, SamplerMipmapMode, Shader,
    ShaderCreateInfo, ShaderFormat, ShaderStage, TextureCreateInfo, TextureFormat, TextureType,
    TextureUsageFlags, PROP_GPU_DEVICE_CREATE_XR_ENABLE_BOOLEAN,
};
use crate::loadso::SharedObject;
use crate::log::Category;
use crate::properties::Properties;
use crate::render::direct3d11::d3d::*;
use crate::video::sysvideo::VideoDriver;

/// The entry points of `d3d12.dll` and `dxgi.dll` that PrepareDriver and
/// CreateDevice need, with the libraries.
struct Libraries {
    _d3d12_dll: SharedObject,
    create_device: PfnD3d12CreateDevice,
    _dxgi_dll: SharedObject,
    create_dxgi_factory1: PfnCreateDxgiFactory,
}

/// Create the DXGI factory (`CreateDXGIFactory1()` of `IDXGIFactory1`).
fn create_dxgi_factory1(f: PfnCreateDxgiFactory) -> std::result::Result<DxgiFactory1, i32> {
    // SAFETY: CreateDXGIFactory1 stores an owned IDXGIFactory1, the
    // interface asked for.
    unsafe { out(|o| f(&IID_IDXGIFACTORY1, o)) }
}

/// The OpenXR instance of a device being made, destroyed when the device
/// fails (see `create_device()`'s Note (upstream)).
struct UnreturnedXrInstance(Option<GpuOpenXr>);

impl Drop for UnreturnedXrInstance {
    fn drop(&mut self) {
        destroy_unreturned_xr_instance(self.0.as_ref());
    }
}

/// Select the adapter for rendering: the first by GPU preference when
/// DXGI 1.6 is there, else the first one.
fn select_adapter(
    factory: &DxgiFactory1,
    gpu_preference: DxgiGpuPreference,
) -> std::result::Result<DxgiAdapter1, i32> {
    match factory.query::<IDXGIFactory6Vtbl>(&IID_IDXGIFACTORY6) {
        Ok(factory6) => factory6.enum_adapter_by_gpu_preference(0, gpu_preference),
        Err(_) => factory.enum_adapters1(0),
    }
}

/// `D3D12CreateDevice()` on `adapter`.
fn create_device_on(
    create_device: PfnD3d12CreateDevice,
    adapter: &DxgiAdapter1,
) -> std::result::Result<D3d12Device, i32> {
    // SAFETY: a live adapter; D3D12CreateDevice stores an owned
    // ID3D12Device, the interface asked for.
    unsafe { out(|o| create_device(raw(adapter), D3D_FEATURE_LEVEL_CHOICE, &IID_ID3D12DEVICE, o)) }
}

/// Whether Direct3D 12 can work here for the creation properties'
/// shader formats, and with OpenXR whether the runtime has its Direct3D 12
/// extension. Translation of `D3D12_PrepareDriver()`.
///
/// Note (upstream): C leaks `d3d12.dll` when `dxgi.dll` or its
/// `CreateDXGIFactory1()` can't be loaded; the library is unloaded here.
/// And on Windows 11, without DXIL and with fewer resource slots allowed,
/// it returns before the OpenXR check.
pub(super) fn prepare_driver(_video: &dyn VideoDriver, props: &Properties) -> bool {
    let mut supports_64_uavs = false;
    let needs_64_uavs = !props
        .get_bool(crate::gpu::PROP_GPU_DEVICE_CREATE_D3D12_ALLOW_FEWER_RESOURCE_SLOTS_BOOLEAN)
        .unwrap_or(false);

    // Early check to see if the app has _any_ D3D12 formats, if not we don't
    // have to fuss with loading D3D in the first place.
    let has_dxbc = props
        .get_bool(crate::gpu::PROP_GPU_DEVICE_CREATE_SHADERS_DXBC_BOOLEAN)
        .unwrap_or(false);
    let has_dxil = props
        .get_bool(crate::gpu::PROP_GPU_DEVICE_CREATE_SHADERS_DXIL_BOOLEAN)
        .unwrap_or(false);
    let mut supports_dxil = false;
    // TODO SM7: bool has_spirv = SDL_GetBooleanProperty(props, SDL_PROP_GPU_DEVICE_CREATE_SHADERS_SPIRV_BOOLEAN, false);
    // TODO SM7: bool supports_spirv = false;
    if !has_dxbc && !has_dxil {
        return false;
    }

    let Some(libraries) = load_libraries(|message| {
        crate::log::warn!(Category::Gpu, "D3D12: {}", message);
    }) else {
        return false;
    };

    // Can we create a device?

    // Create the DXGI factory
    let Ok(factory) = create_dxgi_factory1(libraries.create_dxgi_factory1) else {
        crate::log::warn!(Category::Gpu, "D3D12: Could not create DXGIFactory");
        return false;
    };

    // Check for DXGI 1.4 support
    if factory
        .query::<IDXGIFactory4Vtbl>(&IID_IDXGIFACTORY4)
        .is_err()
    {
        crate::log::warn!(
            Category::Gpu,
            "D3D12: Failed to find DXGI1.4 support, required for DX12"
        );
        return false;
    }

    let Ok(adapter) = select_adapter(&factory, DXGI_GPU_PREFERENCE_HIGH_PERFORMANCE) else {
        crate::log::warn!(
            Category::Gpu,
            "D3D12: Failed to find adapter for D3D12Device"
        );
        return false;
    };

    const _: () = assert!(D3D_FEATURE_LEVEL_CHOICE < D3D_FEATURE_LEVEL_11_1);

    /* If Windows 11 is running and the app has neither DXIL nor TIER2
     * requirements, we can skip doing any device checks entirely
     */
    if !needs_64_uavs && !has_dxil && is_windows_11_or_greater() {
        return true;
    }

    let device = create_device_on(libraries.create_device, &adapter);
    if let Ok(device) = &device {
        // Only check for Tier 2 resource binding if the app needs it
        if needs_64_uavs {
            let mut feature_options = FeatureDataD3d12Options::default();

            let res =
                device.check_feature_support(D3D12_FEATURE_D3D12_OPTIONS, &mut feature_options);
            if res >= 0 && feature_options.resource_binding_tier >= D3D12_RESOURCE_BINDING_TIER_2 {
                supports_64_uavs = true;
            }
        }

        // Only check for SM6 support if DXIL is provided
        if has_dxil {
            let mut shader_model = FeatureDataShaderModel {
                highest_shader_model: D3D_SHADER_MODEL_6_0,
            };

            let res = device.check_feature_support(D3D12_FEATURE_SHADER_MODEL, &mut shader_model);
            if res >= 0 && shader_model.highest_shader_model >= D3D_SHADER_MODEL_6_0 {
                supports_dxil = true;
            }
        }
    }
    let created = device.is_ok();
    drop(device);
    drop(adapter);
    drop(factory);
    drop(libraries);

    if !supports_64_uavs && needs_64_uavs {
        crate::log::warn!(
            Category::Gpu,
            "D3D12: Tier 2 Resource Binding is not supported"
        );
        return false;
    }

    if !supports_dxil && !has_dxbc {
        crate::log::warn!(
            Category::Gpu,
            "D3D12: DXIL is not supported and DXBC is not being provided"
        );
        return false;
    }

    if !created {
        crate::log::warn!(
            Category::Gpu,
            "D3D12: Could not create D3D12Device with feature level {}",
            D3D_FEATURE_LEVEL_CHOICE_STR
        );
        return false;
    }

    // Check for OpenXR support if requested
    let xr = props
        .get_bool(PROP_GPU_DEVICE_CREATE_XR_ENABLE_BOOLEAN)
        .unwrap_or(false);
    if xr {
        if let Err(e) = openxrdyn::load_library() {
            crate::log::warn!(
                Category::Gpu,
                "D3D12: Failed to load OpenXR: {}",
                e.message()
            );
            return false;
        }

        if search_for_openxr_gpu_extension().is_none() {
            crate::log::warn!(
                Category::Gpu,
                "D3D12: Failed to find {} extension",
                XR_KHR_D3D12_ENABLE_EXTENSION_NAME
            );
            openxrdyn::unload_library();
            return false;
        }
        openxrdyn::unload_library();
    }

    true
}

/// Load `d3d12.dll` and `dxgi.dll` and the entry points PrepareDriver
/// checks, reporting what's missing (PrepareDriver's messages).
fn load_libraries(report: impl Fn(&str)) -> Option<Libraries> {
    // Can we load D3D12?

    let Ok(d3d12_dll) = SharedObject::load(D3D12_DLL) else {
        report(&format!("Could not find {D3D12_DLL}"));
        return None;
    };

    // SAFETY: the signature of d3d12.dll's export.
    let Ok(create_device) =
        (unsafe { d3d12_dll.function::<PfnD3d12CreateDevice>(D3D12_CREATE_DEVICE_FUNC) })
    else {
        report(&format!(
            "Could not find function {D3D12_CREATE_DEVICE_FUNC} in {D3D12_DLL}"
        ));
        return None;
    };

    // Can we load DXGI?

    let Ok(dxgi_dll) = SharedObject::load(DXGI_DLL) else {
        report(&format!("Could not find {DXGI_DLL}"));
        return None;
    };

    // SAFETY: the signature of dxgi.dll's export.
    let Ok(create_dxgi_factory1) =
        (unsafe { dxgi_dll.function::<PfnCreateDxgiFactory>(CREATE_DXGI_FACTORY1_FUNC) })
    else {
        report(&format!(
            "Could not find function {CREATE_DXGI_FACTORY1_FUNC} in {DXGI_DLL}"
        ));
        return None;
    };

    Some(Libraries {
        _d3d12_dll: d3d12_dll,
        create_device,
        _dxgi_dll: dxgi_dll,
        create_dxgi_factory1,
    })
}

const D3D12_CREATE_DEVICE_FUNC: &str = "D3D12CreateDevice";
const D3D12_SERIALIZE_ROOT_SIGNATURE_FUNC: &str = "D3D12SerializeRootSignature";
const CREATE_DXGI_FACTORY1_FUNC: &str = "CreateDXGIFactory1";
const DXGI_GET_DEBUG_INTERFACE_FUNC: &str = "DXGIGetDebugInterface";
const D3D12_GET_DEBUG_INTERFACE_FUNC: &str = "D3D12GetDebugInterface";
const D3D12_GET_INTERFACE_FUNC: &str = "D3D12GetInterface";
const PIX_BEGIN_EVENT_ON_COMMAND_LIST_FUNC: &str = "PIXBeginEventOnCommandList";
const PIX_END_EVENT_ON_COMMAND_LIST_FUNC: &str = "PIXEndEventOnCommandList";
const PIX_SET_MARKER_ON_COMMAND_LIST_FUNC: &str = "PIXSetMarkerOnCommandList";

/// The DXGI debug interfaces, when `dxgidebug.dll` has them: the library,
/// `IDXGIDebug` and `IDXGIInfoQueue`. Translation of
/// `D3D12_INTERNAL_TryInitializeDXGIDebug()`; upstream keeps the library
/// loaded when getting the interfaces fails, which is the `Err` here.
fn try_initialize_dxgi_debug(
) -> std::result::Result<(SharedObject, DxgiDebug, DxgiInfoQueue), Option<SharedObject>> {
    let dxgidebug_dll = SharedObject::load(DXGIDEBUG_DLL).map_err(|_| None)?;

    // SAFETY: the signature of dxgidebug.dll's export.
    let Ok(dxgi_get_debug_interface) = (unsafe {
        dxgidebug_dll.function::<PfnD3d12GetDebugInterface>(DXGI_GET_DEBUG_INTERFACE_FUNC)
    }) else {
        return Err(Some(dxgidebug_dll));
    };

    // SAFETY: DXGIGetDebugInterface stores an owned interface of the IID
    // asked for.
    let Ok(dxgi_debug) =
        (unsafe { out::<IDXGIDebugVtbl>(|o| dxgi_get_debug_interface(&IID_IDXGIDEBUG, o)) })
    else {
        return Err(Some(dxgidebug_dll));
    };

    // SAFETY: as above.
    let Ok(dxgi_info_queue) = (unsafe {
        out::<IDXGIInfoQueueVtbl>(|o| dxgi_get_debug_interface(&IID_IDXGIINFOQUEUE, o))
    }) else {
        return Err(Some(dxgidebug_dll));
    };

    Ok((dxgidebug_dll, dxgi_debug, dxgi_info_queue))
}

/// Enable the Direct3D 12 debug layer: the vendored runtime's, through its
/// device factory, or `d3d12.dll`'s. Translation of
/// `D3D12_INTERNAL_TryInitializeD3D12Debug()`.
fn try_initialize_d3d12_debug(
    d3d12_dll: &SharedObject,
    factory: Option<&D3d12DeviceFactory>,
) -> Option<D3d12Debug> {
    if let Some(factory) = factory {
        let d3d12_debug = factory.debug_interface().ok()?;

        d3d12_debug.enable_debug_layer();
        return Some(d3d12_debug);
    }

    // SAFETY: the signature of d3d12.dll's export.
    let d3d12_get_debug_interface =
        unsafe { d3d12_dll.function::<PfnD3d12GetDebugInterface>(D3D12_GET_DEBUG_INTERFACE_FUNC) }
            .ok()?;

    // SAFETY: D3D12GetDebugInterface stores an owned ID3D12Debug, the
    // interface asked for.
    let d3d12_debug =
        unsafe { out::<ID3D12DebugVtbl>(|o| d3d12_get_debug_interface(&IID_ID3D12DEBUG, o)) }
            .ok()?;

    d3d12_debug.enable_debug_layer();
    Some(d3d12_debug)
}

/// The device's info queue, set to store no info messages and to break on
/// corruption. Translation of
/// `D3D12_INTERNAL_TryInitializeD3D12DebugInfoQueue()`.
fn try_initialize_d3d12_debug_info_queue(device: &D3d12Device) -> Option<D3d12InfoQueue> {
    let info_queue = device
        .query::<ID3D12InfoQueueVtbl>(&IID_ID3D12INFOQUEUE)
        .ok()?;

    let severities = [D3D12_MESSAGE_SEVERITY_INFO];
    let mut filter = InfoQueueFilter::default();
    filter.deny_list.num_severities = 1;
    filter.deny_list.severity_list = severities.as_ptr();
    info_queue.push_storage_filter(&filter);

    info_queue.set_break_on_severity(D3D12_MESSAGE_SEVERITY_CORRUPTION, true);

    Some(info_queue)
}

/// The name of a message category (`D3D12_INTERNAL_OnD3D12DebugInfoMsg()`'s
/// `catStr`).
pub(super) fn message_category_name(category: u32) -> &'static str {
    match category {
        D3D12_MESSAGE_CATEGORY_APPLICATION_DEFINED => "APPLICATION_DEFINED",
        D3D12_MESSAGE_CATEGORY_MISCELLANEOUS => "MISCELLANEOUS",
        D3D12_MESSAGE_CATEGORY_INITIALIZATION => "INITIALIZATION",
        D3D12_MESSAGE_CATEGORY_CLEANUP => "CLEANUP",
        D3D12_MESSAGE_CATEGORY_COMPILATION => "COMPILATION",
        D3D12_MESSAGE_CATEGORY_STATE_CREATION => "STATE_CREATION",
        D3D12_MESSAGE_CATEGORY_STATE_SETTING => "STATE_SETTING",
        D3D12_MESSAGE_CATEGORY_STATE_GETTING => "STATE_GETTING",
        D3D12_MESSAGE_CATEGORY_RESOURCE_MANIPULATION => "RESOURCE_MANIPULATION",
        D3D12_MESSAGE_CATEGORY_EXECUTION => "EXECUTION",
        D3D12_MESSAGE_CATEGORY_SHADER => "SHADER",
        _ => "UNKNOWN",
    }
}

/// The name of a message severity (`sevStr`).
pub(super) fn message_severity_name(severity: u32) -> &'static str {
    match severity {
        D3D12_MESSAGE_SEVERITY_CORRUPTION => "CORRUPTION",
        D3D12_MESSAGE_SEVERITY_ERROR => "ERROR",
        D3D12_MESSAGE_SEVERITY_WARNING => "WARNING",
        D3D12_MESSAGE_SEVERITY_INFO => "INFO",
        D3D12_MESSAGE_SEVERITY_MESSAGE => "MESSAGE",
        _ => "UNKNOWN",
    }
}

/// Log a debug layer message. The body of
/// `D3D12_INTERNAL_OnD3D12DebugInfoMsg()`.
pub(super) fn log_debug_info_msg(category: u32, severity: u32, id: i32, description: &str) {
    let cat_str = message_category_name(category);
    let sev_str = message_severity_name(severity);

    if severity <= D3D12_MESSAGE_SEVERITY_ERROR {
        crate::log::error!(
            Category::Gpu,
            "D3D12 ERROR: {} [{} {} #{}]",
            description,
            cat_str,
            sev_str,
            id
        );
    } else {
        crate::log::warn!(
            Category::Gpu,
            "D3D12 WARNING: {} [{} {} #{}]",
            description,
            cat_str,
            sev_str,
            id
        );
    }
}

/// The text of a message description (NULL is empty).
fn description_text(description: *const c_char) -> String {
    if description.is_null() {
        return String::new();
    }
    // SAFETY: the debug layer passes a NUL-terminated description.
    unsafe { CStr::from_ptr(description) }
        .to_string_lossy()
        .into_owned()
}

/// Translation of `D3D12_INTERNAL_OnD3D12DebugInfoMsg()`, the info queue's
/// message callback.
unsafe extern "system" fn on_d3d12_debug_info_msg(
    category: u32,
    severity: u32,
    id: i32,
    description: *const c_char,
    _context: *mut c_void,
) {
    log_debug_info_msg(category, severity, id, &description_text(description));
}

/// Register [`on_d3d12_debug_info_msg`] with the device's
/// `ID3D12InfoQueue1`: whether that worked
/// (`InfoQueueMessageCallbackSupported`). Translation of
/// `D3D12_INTERNAL_TryInitializeD3D12DebugInfoLogger()`.
fn try_initialize_d3d12_debug_info_logger(device: &D3d12Device) -> bool {
    let Ok(info_queue) = device.query::<ID3D12InfoQueue1Vtbl>(&IID_ID3D12INFOQUEUE1) else {
        return false;
    };

    info_queue
        .register_message_callback(on_d3d12_debug_info_msg, D3D12_MESSAGE_CALLBACK_FLAG_NONE)
        .is_ok()
}

impl D3D12Renderer {
    /// Log the messages the info queue stored, when no callback logs them
    /// as they come. Translation of
    /// `D3D12_INTERNAL_DrainInfoQueueMessages()`.
    pub(super) fn drain_info_queue_messages(&self) {
        let Some(info_queue) = &self.debug_info_queue else {
            return;
        };
        if self.info_queue_message_callback_supported {
            return;
        }

        let count = info_queue.num_stored_messages();
        if count == 0 {
            return;
        }

        for i in 0..count {
            info_queue.with_message(i, |message| {
                log_debug_info_msg(
                    message.category,
                    message.severity,
                    message.id,
                    &description_text(message.description),
                );
            });
        }

        info_queue.clear_stored_messages();
    }

    /// Create the blit shaders and samplers (the pipelines are made when
    /// first needed). Translation of `D3D12_INTERNAL_InitBlitResources()`:
    /// like upstream, a failure is logged and leaves that resource out.
    fn init_blit_resources(&self) -> BlitResources {
        let shader = |code: &[u8], stage: ShaderStage, uniforms: u32, error: &str| {
            let shader_create_info = ShaderCreateInfo {
                code,
                entrypoint: "",
                format: ShaderFormat::DXBC,
                stage,
                num_samplers: uniforms,
                num_storage_textures: 0,
                num_storage_buffers: 0,
                num_uniform_buffers: uniforms,
                props: None,
            };
            match self.create_shader_internal(&shader_create_info) {
                Ok(shader) => Some(Shader::from_backend(BackendObject::new(shader))),
                Err(_) => {
                    crate::log::error!(Category::Gpu, "{}", error);
                    None
                }
            }
        };

        // Fullscreen vertex shader
        let vertex_shader = shader(
            &D3D12_FULLSCREEN_VERT,
            ShaderStage::Vertex,
            0,
            "Failed to compile vertex shader for blit!",
        );

        // BlitFrom2D pixel shader
        let from_2d_shader = shader(
            &D3D12_BLIT_FROM_2D,
            ShaderStage::Fragment,
            1,
            "Failed to compile BlitFrom2D pixel shader!",
        );

        // BlitFrom2DArray pixel shader
        let from_2d_array_shader = shader(
            &D3D12_BLIT_FROM_2D_ARRAY,
            ShaderStage::Fragment,
            1,
            "Failed to compile BlitFrom2DArray pixel shader!",
        );

        // BlitFrom3D pixel shader
        let from_3d_shader = shader(
            &D3D12_BLIT_FROM_3D,
            ShaderStage::Fragment,
            1,
            "Failed to compile BlitFrom3D pixel shader!",
        );

        // BlitFromCube pixel shader
        let from_cube_shader = shader(
            &D3D12_BLIT_FROM_CUBE,
            ShaderStage::Fragment,
            1,
            "Failed to compile BlitFromCube pixel shader!",
        );

        // BlitFromCubeArray pixel shader
        let from_cube_array_shader = shader(
            &D3D12_BLIT_FROM_CUBE_ARRAY,
            ShaderStage::Fragment,
            1,
            "Failed to compile BlitFromCubeArray pixel shader!",
        );

        // Create samplers
        let mut sampler_create_info = SamplerCreateInfo {
            address_mode_u: SamplerAddressMode::ClampToEdge,
            address_mode_v: SamplerAddressMode::ClampToEdge,
            address_mode_w: SamplerAddressMode::ClampToEdge,
            enable_anisotropy: false,
            enable_compare: false,
            mag_filter: Filter::Nearest,
            min_filter: Filter::Nearest,
            mipmap_mode: SamplerMipmapMode::Nearest,
            mip_lod_bias: 0.0,
            min_lod: 0.0,
            max_lod: 1000.0,
            max_anisotropy: 1.0,
            compare_op: CompareOp::Never,
            props: None,
        };
        let sampler =
            |info: &SamplerCreateInfo, error: &str| match self.create_sampler_internal(info) {
                Ok(sampler) => Some(Sampler::from_backend(BackendObject(sampler))),
                Err(_) => {
                    crate::log::error!(Category::Gpu, "{}", error);
                    None
                }
            };

        let nearest_sampler = sampler(
            &sampler_create_info,
            "Failed to create blit nearest sampler!",
        );

        sampler_create_info.mag_filter = Filter::Linear;
        sampler_create_info.min_filter = Filter::Linear;
        sampler_create_info.mipmap_mode = SamplerMipmapMode::Linear;

        let linear_sampler = sampler(
            &sampler_create_info,
            "Failed to create blit linear sampler!",
        );

        BlitResources {
            vertex_shader,
            from_2d_shader,
            from_2d_array_shader,
            from_3d_shader,
            from_cube_shader,
            from_cube_array_shader,
            nearest_sampler,
            linear_sampler,
            pipelines: BlitPipelineCache::PerFormat(Vec::with_capacity(2)),
        }
    }

    /// Release the blit shaders, samplers and pipelines. Translation of
    /// `D3D12_INTERNAL_ReleaseBlitPipelines()`.
    pub(super) fn release_blit_pipelines(&self) {
        let blit = std::mem::take(&mut *lock(&self.blit));
        for sampler in [blit.linear_sampler, blit.nearest_sampler]
            .into_iter()
            .flatten()
        {
            if let Some(sampler) = sampler.raw.downcast::<D3D12Sampler>() {
                self.release_sampler_internal(sampler);
            }
        }
        // (The shaders are freed as they are dropped.)

        let (BlitPipelineCache::PerFormat(pipelines)
        | BlitPipelineCache::FormatAgnostic(pipelines)) = blit.pipelines;
        for entry in pipelines {
            if let Some(pipeline) = entry.pipeline.raw.downcast() {
                self.release_graphics_pipeline_internal(pipeline);
            }
        }
    }
}

// OpenXR D3D12 Support

/// The runtime's Direct3D 12 extension (`XR_KHR_D3D12_enable`), if it has
/// it. Translation of `D3D12_INTERNAL_SearchForOpenXrGpuExtension()`.
fn search_for_openxr_gpu_extension() -> Option<XrExtensionProperties> {
    // (a loader without the function: see openxrdyn::get_sym)
    let xr_enumerate_instance_extension_properties =
        openxrdyn::globals().xrEnumerateInstanceExtensionProperties?;

    let mut extension_count: u32 = 0;
    // SAFETY: a count query.
    let result = unsafe {
        xr_enumerate_instance_extension_properties(null(), 0, &mut extension_count, null_mut())
    };
    if result != XrResult::SUCCESS {
        return None;
    }

    let mut extension_properties = vec![
        XrExtensionProperties {
            ty: XR_TYPE_EXTENSION_PROPERTIES,
            ..Default::default()
        };
        extension_count as usize
    ];

    // SAFETY: an array of extension_count structures.
    let result = unsafe {
        xr_enumerate_instance_extension_properties(
            null(),
            extension_count,
            &mut extension_count,
            extension_properties.as_mut_ptr(),
        )
    };
    if result != XrResult::SUCCESS {
        return None;
    }

    for extension in extension_properties.iter().take(extension_count as usize) {
        if xr_name(&extension.extension_name) == XR_KHR_D3D12_ENABLE_EXTENSION_NAME {
            crate::log::info!(
                Category::Gpu,
                "Found {} extension",
                XR_KHR_D3D12_ENABLE_EXTENSION_NAME
            );

            return Some(*extension);
        }
    }

    None
}

/// The runtime's Direct3D 12 requirements for the system: the minimum
/// feature level and the adapter (its LUID), or the failed call's result.
/// Translation of `D3D12_INTERNAL_GetXrGraphicsRequirements()`.
fn get_xr_graphics_requirements(
    instance: XrInstance,
    system_id: XrSystemId,
) -> std::result::Result<(i32, Luid), XrResult> {
    // SAFETY: the function's type.
    let (xr_result, xr_get_d3d12_graphics_requirements_khr) = unsafe {
        openxrdyn::xr_get_instance_proc_addr::<PfnXrGetD3D12GraphicsRequirementsKHR>(
            instance,
            "xrGetD3D12GraphicsRequirementsKHR",
        )
    };
    let Some(xr_get_d3d12_graphics_requirements_khr) = xr_get_d3d12_graphics_requirements_khr
    else {
        crate::log::debug!(
            Category::Gpu,
            "Failed to get xrGetD3D12GraphicsRequirementsKHR"
        );
        return Err(xr_result);
    };

    let mut graphics_requirements_d3d12 = XrGraphicsRequirementsD3D12KHR {
        ty: XR_TYPE_GRAPHICS_REQUIREMENTS_D3D12_KHR,
        next: null_mut(),
        adapter_luid: Luid::default(),
        min_feature_level: 0,
    };
    // SAFETY: the instance's function, with the system and an out-structure.
    let xr_result = unsafe {
        xr_get_d3d12_graphics_requirements_khr(
            instance,
            system_id,
            &mut graphics_requirements_d3d12,
        )
    };
    if xr_result != XrResult::SUCCESS {
        crate::log::debug!(
            Category::Gpu,
            "Failed to get D3D12 graphics requirements, got OpenXR error {}",
            xr_result.0
        );
        return Err(xr_result);
    }

    Ok((
        graphics_requirements_d3d12.min_feature_level,
        graphics_requirements_d3d12.adapter_luid,
    ))
}

/// The adapter of a LUID, from the factory's adapters. Translation of
/// `D3D12_INTERNAL_GetAdapterByLuid()`.
fn get_adapter_by_luid(luid: Luid, factory: &DxgiFactory1) -> Option<DxgiAdapter1> {
    // We need to iterate over all the adapters in the system to find the one with the matching LUID
    for adapter_index in 0u32.. {
        // EnumAdapters1 will fail with DXGI_ERROR_NOT_FOUND when there are no more adapters to enumerate.
        let adapter = match factory.enum_adapters1(adapter_index) {
            Ok(adapter) => adapter,
            Err(res) => {
                crate::log::error!(
                    Category::Gpu,
                    "Failed to get an adapter when iterating, i: {}, res: {}",
                    adapter_index as i32,
                    res
                );
                return None;
            }
        };

        let adapter_desc = match adapter.desc1() {
            Ok(adapter_desc) => adapter_desc,
            Err(res) => {
                crate::log::error!(
                    Category::Gpu,
                    "Failed to get description of adapter, i: {}, res {}",
                    adapter_index as i32,
                    res
                );
                return None;
            }
        };
        if adapter_desc.adapter_luid == [luid.low_part, luid.high_part as u32] {
            return Some(adapter);
        }
    }
    None
}

impl D3D12Renderer {
    /// The OpenXR instance's functions (`renderer->xr`).
    ///
    /// FIXME (upstream): the OpenXR entry points use `renderer->xr`, which
    /// is NULL on a device made without OpenXR, so C dereferences NULL
    /// there (an access violation); here it panics.
    fn xr_pfns(&self) -> &XrInstancePfns {
        self.xr
            .as_ref()
            .expect("the GPU device was made without OpenXR")
    }

    /// Destroy an OpenXR swapchain and the textures of its images.
    /// Translation of `D3D12_DestroyXRSwapchain()`.
    ///
    /// Note (upstream): a texture that isn't an OpenXR swapchain's gives
    /// "Invalid GPU Texture handle." once the textures before it are
    /// destroyed, and the swapchain is kept. An array shorter than the
    /// swapchain's images is read past its end; here the destruction stops
    /// at its end.
    pub(super) fn destroy_xr_swapchain(
        &self,
        swapchain: XrSwapchain,
        swapchain_images: &[BackendObject],
    ) -> std::result::Result<(), XrError> {
        let xr = self.xr_pfns();

        let _ = self.wait_internal();

        let mut swapchain_count: u32 = 0;
        // SAFETY: the instance's function, a count query.
        let result = unsafe {
            (xr.xrEnumerateSwapchainImages)(swapchain, 0, &mut swapchain_count, null_mut())
        };
        if result != XrResult::SUCCESS {
            return Err(XrError::from_result(result));
        }

        // We always want to destroy the swapchain images, so don't early return if xrDestroySwapchain fails for some reason
        for image in swapchain_images.iter().take(swapchain_count as usize) {
            // Note (upstream): C casts each texture container to a texture
            // (`D3D12Texture *container = (D3D12Texture *)swapchainImages[i]`),
            // so it reads `externallyManaged` at the texture's offset (a
            // padding byte of the container's, uninitialized) and destroys
            // the container as a texture (its subresources' array is the
            // container's height and layer count), then frees it twice: an
            // error or a crash, as the byte happens to be. Here the
            // container's own flag is read, and its texture is destroyed.
            let container = super::object::<TextureContainer>(image);

            if !container.externally_managed {
                return Err(XrError::new(
                    XrResult::ERROR_HANDLE_INVALID,
                    Error::new("Invalid GPU Texture handle."),
                ));
            }

            // (the texture, whose resource stays the runtime's, goes with
            // its container's last reference)
            drop(std::mem::take(&mut lock(&container.state).textures));

            // Free the container now that it's unused
            // (the front end drops its handles next)
        }

        // (SDL_free(swapchainImages): the front end's)

        // SAFETY: the instance's function, with the swapchain.
        let result = unsafe { (xr.xrDestroySwapchain)(swapchain) };
        if result.failed() {
            return Err(XrError::from_result(result));
        }
        Ok(())
    }

    /// The session's swapchain formats that have a texture format, in the
    /// runtime's order (the first texture format of each). Translation of
    /// `D3D12_GetXRSwapchainFormats()`.
    ///
    /// Note (upstream): when the runtime's call fails, C returns `NULL`
    /// with no error set; and it frees the runtime's array twice and its
    /// own never (harmless where `SDL_stack_alloc` is `alloca`). Here the
    /// error says which call failed, and the arrays are owned.
    pub(super) fn get_xr_swapchain_formats(
        &self,
        session: XrSession,
    ) -> Result<Vec<TextureFormat>> {
        let xr = self.xr_pfns();

        let mut num_supported_formats: u32 = 0;
        // SAFETY: the instance's function, a count query.
        let result = unsafe {
            (xr.xrEnumerateSwapchainFormats)(session, 0, &mut num_supported_formats, null_mut())
        };
        if result != XrResult::SUCCESS {
            return Err(Error::new(format!(
                "xrEnumerateSwapchainFormats failed ({result})"
            )));
        }

        let mut supported_formats = vec![0i64; num_supported_formats as usize];
        // SAFETY: an array of num_supported_formats formats.
        let result = unsafe {
            (xr.xrEnumerateSwapchainFormats)(
                session,
                num_supported_formats,
                &mut num_supported_formats,
                supported_formats.as_mut_ptr(),
            )
        };
        if result != XrResult::SUCCESS {
            return Err(Error::new(format!(
                "xrEnumerateSwapchainFormats failed ({result})"
            )));
        }

        let mut sdl_formats = Vec::with_capacity(num_supported_formats as usize);

        // Iterate over all formats the runtime supports
        for &supported_format in supported_formats
            .iter()
            .take(num_supported_formats as usize)
        {
            // Iterate over all formats we support
            if let Some(j) = SDL_TO_D3D12_TEXTURE_FORMAT
                .iter()
                .position(|&dxgi_format| i64::from(dxgi_format) == supported_format)
            {
                // Add the format match we found, linearly. The output order should match the order of the runtime.
                sdl_formats.push(TextureFormat(j as u32));
            }
        }

        if sdl_formats.is_empty() {
            return Err(Error::new(
                "Failed to find a swapchain format supported by both OpenXR and SDL",
            ));
        }

        Ok(sdl_formats)
    }

    /// Create an OpenXR swapchain in a texture format, and a texture of
    /// each of its images (a container of one texture, never cycled, whose
    /// resource is the runtime's). Translation of
    /// `D3D12_CreateXRSwapchain()`.
    ///
    /// FIXME (upstream): when a call after `xrCreateSwapchain` fails, the
    /// swapchain is left alive (C also leaks the images' array when their
    /// count can't be had).
    ///
    /// Note (upstream): the textures' usage is COLOR_TARGET whatever the
    /// swapchain's usage flags (their render target views are made only
    /// with `XR_SWAPCHAIN_USAGE_COLOR_ATTACHMENT_BIT`), they have no shader
    /// resource view (they can't be sampled), and their type is 2D whatever
    /// the array size.
    ///
    /// FIXME (upstream): so every image is taken to be in the color target
    /// usage's default state, `D3D12_RESOURCE_STATE_RENDER_TARGET` (the
    /// state the runtime gives a color attachment's images, which upstream
    /// never transitions here), which Direct3D 12 allows only for resources
    /// made with `D3D12_RESOURCE_FLAG_ALLOW_RENDER_TARGET`: the images of a
    /// swapchain without `XR_SWAPCHAIN_USAGE_COLOR_ATTACHMENT_BIT`, on a
    /// runtime whose images have exactly the flags its usage asks for (the
    /// repository's test runtime makes them in the common state), get
    /// barriers from a state they can't be in on their first use (a copy's
    /// or a render pass's, the latter with no view to render to). Kept, as
    /// upstream's C does it.
    pub(super) fn create_xr_swapchain(
        &self,
        session: XrSession,
        old_create_info: &XrSwapchainCreateInfo,
        format: TextureFormat,
    ) -> std::result::Result<(XrSwapchain, Vec<(BackendObject, TextureCreateInfo)>), XrError> {
        let xr = self.xr_pfns();

        let mut create_info = *old_create_info;
        create_info.format = i64::from(sdl_to_d3d12_texture_format(format));

        let mut swapchain: XrSwapchain = XR_NULL_HANDLE;
        // SAFETY: the instance's function, with the create info (the
        // application's chain is its to keep valid) and an out-pointer.
        let result = unsafe { (xr.xrCreateSwapchain)(session, &create_info, &mut swapchain) };
        if result != XrResult::SUCCESS {
            return Err(XrError::from_result(result));
        }

        let mut swapchain_image_count: u32 = 0;
        // SAFETY: the instance's function, a count query.
        let result = unsafe {
            (xr.xrEnumerateSwapchainImages)(swapchain, 0, &mut swapchain_image_count, null_mut())
        };
        if result != XrResult::SUCCESS {
            return Err(XrError::from_result(result));
        }

        let mut swapchain_images = vec![
            XrSwapchainImageD3D12KHR {
                ty: XR_TYPE_SWAPCHAIN_IMAGE_D3D12_KHR,
                next: null_mut(),
                texture: null_mut(),
            };
            swapchain_image_count as usize
        ];

        // SAFETY: an array of swapchain_image_count Direct3D 12 images,
        // whose header the structures begin with.
        let result = unsafe {
            (xr.xrEnumerateSwapchainImages)(
                swapchain,
                swapchain_image_count,
                &mut swapchain_image_count,
                swapchain_images.as_mut_ptr().cast(),
            )
        };
        if result != XrResult::SUCCESS {
            return Err(XrError::from_result(result));
        }

        let mut texture_containers = Vec::with_capacity(swapchain_image_count as usize);

        let depth = 1;

        for swapchain_image in swapchain_images.iter().take(swapchain_image_count as usize) {
            // (the runtime's reference, borrowed: an externally managed
            // texture never releases it)
            // SAFETY: the runtime's ID3D12Resource, alive while the
            // swapchain is.
            let Some(d3d12_texture) = (unsafe { ComPtr::from_raw(swapchain_image.texture.cast()) })
            else {
                // (a NULL image, which C would dereference)
                return Err(XrError::new(
                    XrResult::ERROR_RUNTIME_FAILURE,
                    Error::new("The OpenXR runtime gave a NULL swapchain image"),
                ));
            };
            let resource: ManuallyDrop<D3d12Resource> = ManuallyDrop::new(d3d12_texture);

            let subresource_count = (create_info.array_size * create_info.mip_count) as usize;
            let mut subresources: Vec<TextureSubresource> = Vec::with_capacity(subresource_count);
            subresources.resize_with(subresource_count, Default::default);

            for layer_index in 0..create_info.array_size {
                for level_index in 0..create_info.mip_count {
                    let subresource_index =
                        calc_subresource(level_index, layer_index, create_info.mip_count);
                    let subresource = &mut subresources[subresource_index as usize];

                    subresource.layer = layer_index;
                    subresource.level = level_index;
                    subresource.depth = depth;
                    subresource.index = subresource_index;

                    if create_info.usage_flags & XR_SWAPCHAIN_USAGE_COLOR_ATTACHMENT_BIT != 0 {
                        subresource.rtv_handles.reserve_exact(depth as usize);

                        for _depth_index in 0..depth {
                            // Note (upstream): C ignores a failure to get a
                            // descriptor; the swapchain fails here.
                            let rtv = self
                                .assign_staging_descriptor_handle(D3D12_DESCRIPTOR_HEAP_TYPE_RTV)
                                .map_err(|e| XrError::new(XrResult::ERROR_RUNTIME_FAILURE, e))?;

                            let mut rtv_desc = RenderTargetViewDesc {
                                format: sdl_to_d3d12_texture_format(format),
                                ..Default::default()
                            };

                            // For XR we typically use 2D array textures for stereo rendering
                            if create_info.array_size > 1 {
                                rtv_desc.view_dimension = D3D12_RTV_DIMENSION_TEXTURE2DARRAY;
                                rtv_desc.u.texture_2d_array = Tex2dArrayRtv {
                                    mip_slice: level_index,
                                    first_array_slice: layer_index,
                                    array_size: 1,
                                    plane_slice: 0,
                                };
                            } else {
                                rtv_desc.view_dimension = D3D12_RTV_DIMENSION_TEXTURE2D;
                                rtv_desc.u.texture_2d = Tex2dRtv {
                                    mip_slice: level_index,
                                    plane_slice: 0,
                                };
                            }

                            self.device.create_render_target_view(
                                &resource,
                                &rtv_desc,
                                rtv.cpu_handle,
                            );
                            subresource.rtv_handles.push(rtv);
                        }
                    }
                }
            }

            let info = TextureCreateInfo {
                texture_type: TextureType::Texture2D,
                format,
                usage: TextureUsageFlags::COLOR_TARGET,
                width: create_info.width,
                height: create_info.height,
                layer_count_or_depth: create_info.array_size,
                num_levels: create_info.mip_count,
                sample_count: SampleCount::One,
                props: None,
            };

            let texture = Arc::new(D3D12Texture {
                container: Mutex::new(None),
                subresources,
                resource,
                srv_handle: None,
                reference_count: AtomicI32::new(0),
                externally_managed: true,
                info: info.clone(),
            });

            let container = Arc::new(TextureContainer {
                info: info.clone(),
                state: Mutex::new(TextureContainerState {
                    active_texture: texture.clone(),
                    textures: vec![texture.clone()],
                    debug_name: None,
                }),
                can_be_cycled: false,
                externally_managed: true,
            });

            *lock(&texture.container) = Some(ContainerRef {
                container: Arc::downgrade(&container),
                index: 0,
            });
            texture_containers.push((BackendObject(container), info));
        }

        Ok((swapchain, texture_containers))
    }

    /// Create an OpenXR session of the device's system, with the device's
    /// graphics binding (its device and queue) before the application's
    /// chain of structures. Translation of `D3D12_CreateXRSession()`.
    pub(super) fn create_xr_session(
        &self,
        createinfo: &XrSessionCreateInfo,
    ) -> std::result::Result<XrSession, XrError> {
        let xr = self.xr_pfns();

        // Copy out the existing next ptr so that we can append it to the end of the chain we create
        let current_next_ptr = createinfo.next;

        let graphics_binding = XrGraphicsBindingD3D12KHR {
            ty: XR_TYPE_GRAPHICS_BINDING_D3D12_KHR,
            next: current_next_ptr,
            device: raw(&self.device),
            queue: raw(&self.command_queue),
        };

        let mut session_create_info = *createinfo;
        session_create_info.system_id = self.xr_system_id;
        session_create_info.next = (&graphics_binding as *const XrGraphicsBindingD3D12KHR).cast();

        let mut session: XrSession = XR_NULL_HANDLE;
        // SAFETY: the instance's function, with the create info and its
        // chain (the application's part is its to keep valid), and an
        // out-pointer.
        let result =
            unsafe { (xr.xrCreateSession)(self.xr_instance, &session_create_info, &mut session) };
        if result.failed() {
            return Err(XrError::from_result(result));
        }
        Ok(session)
    }
}

/// The adapter's description, as `D3D12_CreateDevice()` logs it and
/// records it in the properties: the name and the driver version.
fn record_adapter(
    props: &Properties,
    adapter_desc: &AdapterDesc1,
    umd_version: i64,
    verbose_logs: bool,
) -> Result<()> {
    if verbose_logs {
        crate::log::info!(Category::Gpu, "SDL_GPU Driver: D3D12");
    }

    // Record device name
    let device_name = wide_to_utf8(&adapter_desc.description);
    props.set(
        crate::gpu::PROP_GPU_DEVICE_NAME_STRING,
        device_name.as_str(),
    )?;
    if verbose_logs {
        crate::log::info!(Category::Gpu, "D3D12 Adapter: {}", device_name);
    }

    // Record driver version
    let driver_ver = driver_version(umd_version);
    props.set(
        crate::gpu::PROP_GPU_DEVICE_DRIVER_VERSION_STRING,
        driver_ver.as_str(),
    )?;
    if verbose_logs {
        crate::log::info!(Category::Gpu, "D3D12 Driver: {}", driver_ver);
    }
    Ok(())
}

/// The driver version of a user mode driver version (a `LARGE_INTEGER`)
/// as `D3D12_CreateDevice()` prints it: `HIWORD(HighPart)`,
/// `LOWORD(HighPart)`, `HIWORD(LowPart)`, `LOWORD(LowPart)`.
pub(super) fn driver_version(umd_version: i64) -> String {
    let high = (umd_version >> 32) as u32;
    let low = umd_version as u32;
    format!(
        "{}.{}.{}.{}",
        high >> 16,
        high & 0xffff,
        low >> 16,
        low & 0xffff
    )
}

/// The vendored Direct3D 12 runtime of the Agility SDK, when the creation
/// properties name one: a device factory that allows a D3D12 redistributable
/// provided by the client to be loaded (the `ID3D12DeviceFactory` part of
/// `D3D12_CreateDevice()`).
fn agility_sdk_device_factory(
    d3d12_dll: &SharedObject,
    props: &Properties,
) -> Option<D3d12DeviceFactory> {
    let version_name = crate::gpu::PROP_GPU_DEVICE_CREATE_D3D12_AGILITY_SDK_VERSION_NUMBER;
    let path_name = crate::gpu::PROP_GPU_DEVICE_CREATE_D3D12_AGILITY_SDK_PATH_STRING;
    if !(props.contains(path_name) && props.contains(version_name)) {
        return None;
    }
    let d3d12_sdk_version = props.get_number(version_name).unwrap_or(0) as i32;
    let d3d12_sdk_path = props
        .get_string(path_name)
        .unwrap_or_else(|| ".\\D3D12\\".to_owned());
    let d3d12_sdk_path = CString::new(d3d12_sdk_path).unwrap_or_default();

    // SAFETY: the signature of d3d12.dll's export.
    let Ok(d3d12_get_interface) =
        (unsafe { d3d12_dll.function::<PfnD3d12GetInterface>(D3D12_GET_INTERFACE_FUNC) })
    else {
        crate::log::warn!(
            Category::Gpu,
            "Could not load D3D12GetInterface, custom D3D12 SDK will not load."
        );
        return None;
    };

    // SAFETY: D3D12GetInterface stores an owned ID3D12SDKConfiguration, the
    // interface asked for.
    let Ok(sdk_config) = (unsafe {
        out::<ID3D12SDKConfigurationVtbl>(|o| {
            d3d12_get_interface(
                &CLSID_ID3D12SDKCONFIGURATION,
                &IID_ID3D12SDKCONFIGURATION,
                o,
            )
        })
    }) else {
        crate::log::warn!(
            Category::Gpu,
            "Failed to load vendored D3D12 SDK Configuration"
        );
        return None;
    };

    let sdk_config1 = sdk_config
        .query::<ID3D12SDKConfiguration1Vtbl>(&IID_ID3D12SDKCONFIGURATION1)
        .ok()?;
    match sdk_config1.create_device_factory(d3d12_sdk_version as u32, &d3d12_sdk_path) {
        Ok(factory) => {
            crate::log::info!(Category::Gpu, "Loaded vendored D3D12Core.dll");
            Some(factory)
        }
        Err(_) => {
            crate::log::warn!(Category::Gpu, "Failed to load vendored D3D12Core.dll");
            None
        }
    }
}

/// Translation of `D3D12_CreateDevice()`: the renderer and the shader
/// formats it takes (DXBC, and DXIL with shader model 6); with OpenXR, the
/// instance and system first (`SDL_OPENXR_INTERNAL_GPUInitOpenXR()`), and
/// the adapter of the runtime's LUID.
///
/// Upstream frees what it made so far (`D3D12_INTERNAL_DestroyRenderer()`)
/// when a step fails; here that is dropping it.
///
/// Note (upstream): upstream refuses a device made with OpenXR without
/// its out pointers for the instance and the system; here they're the
/// renderer's (`Device::xr_instance`, `Device::xr_system_id`). When a step
/// fails once the instance is made, C leaves it to the application, which
/// has it through the out pointer (and `renderer->xr` leaks; and the
/// loader's reference too after the adapter's steps); here the application
/// never sees it, so it's destroyed (the loader's reference is released or
/// kept as upstream does).
pub(super) fn create_device(
    debug_mode: bool,
    prefer_low_power: bool,
    props: &Properties,
) -> Result<(D3D12Renderer, ShaderFormat)> {
    let verbose_logs = props
        .get_bool(crate::gpu::PROP_GPU_DEVICE_CREATE_VERBOSE_BOOLEAN)
        .unwrap_or(true);

    let mut has_dxgi_debug = false;

    // Load the DXGI library
    let dxgi_dll = SharedObject::load(DXGI_DLL)
        .map_err(|_| set_string_error(debug_mode, &format!("Could not find {DXGI_DLL}")))?;

    // Initialize the DXGI debug layer, if applicable
    let mut dxgidebug_dll = None;
    let mut dxgi_debug = None;
    let mut dxgi_info_queue = None;
    if debug_mode {
        match try_initialize_dxgi_debug() {
            Ok((dll, debug, info_queue)) => {
                dxgidebug_dll = Some(dll);
                dxgi_debug = Some(debug);
                dxgi_info_queue = Some(info_queue);
                has_dxgi_debug = true;
            }
            Err(dll) => dxgidebug_dll = dll,
        }
    }

    // Load the PIX runtime DLL so that we can set D3D12 debug events on command lists
    let winpixeventruntime_dll = SharedObject::load(WINPIXEVENTRUNTIME_DLL).ok();
    let winpixeventruntime_fns = match &winpixeventruntime_dll {
        // Load the specific functions we need from the PIX runtime
        // SAFETY: the signatures of the PIX runtime's exports.
        Some(dll) => unsafe {
            WinPixEventRuntimeFns {
                begin_event_on_command_list: dll
                    .function::<PfnBeginEventOnCommandList>(PIX_BEGIN_EVENT_ON_COMMAND_LIST_FUNC)
                    .ok(),
                end_event_on_command_list: dll
                    .function::<PfnEndEventOnCommandList>(PIX_END_EVENT_ON_COMMAND_LIST_FUNC)
                    .ok(),
                set_marker_on_command_list: dll
                    .function::<PfnBeginEventOnCommandList>(PIX_SET_MARKER_ON_COMMAND_LIST_FUNC)
                    .ok(),
            }
        },
        None => {
            crate::log::warn!(
                Category::Gpu,
                "WinPixEventRuntime.dll is not available. \
                 It is required for SDL_Push/PopGPUDebugGroup and SDL_InsertGPUDebugLabel to function correctly. \
                 See here for instructions on how to obtain it: https://devblogs.microsoft.com/pix/winpixeventruntime/"
            );
            WinPixEventRuntimeFns::default()
        }
    };

    // Load the CreateDXGIFactory1 function
    // SAFETY: the signature of dxgi.dll's export.
    let create_dxgi_factory1_fn =
        unsafe { dxgi_dll.function::<PfnCreateDxgiFactory>(CREATE_DXGI_FACTORY1_FUNC) }.map_err(
            |_| {
                set_string_error(
                    debug_mode,
                    &format!("Could not load function: {CREATE_DXGI_FACTORY1_FUNC}"),
                )
            },
        )?;

    // Create the DXGI factory
    let factory1 = create_dxgi_factory1(create_dxgi_factory1_fn)
        .map_err(|res| d3d12_error(debug_mode, None, "Could not create DXGIFactory", res))?;

    // Check for DXGI 1.4 support
    let factory = factory1
        .query::<IDXGIFactory4Vtbl>(&IID_IDXGIFACTORY4)
        .map_err(|res| {
            d3d12_error(
                debug_mode,
                None,
                "DXGI1.4 support not found, required for DX12",
                res,
            )
        })?;
    drop(factory1);

    // Check for explicit tearing support
    let mut supports_tearing = false;
    if let Ok(factory5) = factory.query::<IDXGIFactory5Vtbl>(&IID_IDXGIFACTORY5) {
        supports_tearing = factory5
            .check_feature_support_bool(DXGI_FEATURE_PRESENT_ALLOW_TEARING)
            .unwrap_or(false);
    }

    let xr = props
        .get_bool(PROP_GPU_DEVICE_CREATE_XR_ENABLE_BOOLEAN)
        .unwrap_or(false);
    // FIXME (upstream): the runtime's minimum feature level is read into
    // this and never used: the device is made with D3D_FEATURE_LEVEL_CHOICE
    // (11_0) whatever the runtime asks for.
    let mut _feature_level = D3D_FEATURE_LEVEL_11_0 as i32;
    let mut xr_instance = UnreturnedXrInstance(None);

    let adapter = if xr {
        if openxrdyn::load_library().is_err() {
            return Err(set_string_error(
                debug_mode,
                "This should have failed in PrepareDevice first!",
            ));
        }

        let Some(gpu_extension) = search_for_openxr_gpu_extension() else {
            crate::log::debug!(
                Category::Gpu,
                "Failed to find a compatible OpenXR D3D12 extension"
            );
            openxrdyn::unload_library();
            return Err(set_string_error(
                debug_mode,
                &format!("Failed to find {XR_KHR_D3D12_ENABLE_EXTENSION_NAME} extension"),
            ));
        };

        let Some(init) = gpu_init_openxr(debug_mode, &gpu_extension, props) else {
            crate::log::debug!(Category::Gpu, "Failed to init OpenXR");
            openxrdyn::unload_library();
            return Err(set_string_error(debug_mode, "Failed to initialize OpenXR"));
        };
        let (instance, system_id) = (init.instance, init.system_id);
        xr_instance.0 = Some(init);

        let xr_adapter = match get_xr_graphics_requirements(instance, system_id) {
            Ok((feature_level, xr_adapter)) => {
                _feature_level = feature_level;
                xr_adapter
            }
            Err(_) => {
                drop(xr_instance);
                openxrdyn::unload_library();
                return Err(set_string_error(
                    debug_mode,
                    "Failed to get XR graphics requirements",
                ));
            }
        };

        // Need factory1 again for adapter enumeration
        let factory1_for_xr = match create_dxgi_factory1(create_dxgi_factory1_fn) {
            Ok(factory1_for_xr) => factory1_for_xr,
            Err(res) => {
                drop(xr_instance);
                openxrdyn::unload_library();
                return Err(d3d12_error(
                    debug_mode,
                    None,
                    "Could not create DXGIFactory for XR adapter lookup",
                    res,
                ));
            }
        };

        let Some(adapter) = get_adapter_by_luid(xr_adapter, &factory1_for_xr) else {
            drop(factory1_for_xr);
            drop(xr_instance);
            openxrdyn::unload_library();
            return Err(set_string_error(
                debug_mode,
                "Failed to find adapter matching XR LUID",
            ));
        };
        drop(factory1_for_xr);
        adapter
    } else {
        // Select the appropriate device for rendering
        let gpu_preference = if prefer_low_power {
            DXGI_GPU_PREFERENCE_MINIMUM_POWER
        } else {
            DXGI_GPU_PREFERENCE_HIGH_PERFORMANCE
        };
        match factory.query::<IDXGIFactory6Vtbl>(&IID_IDXGIFACTORY6) {
            Ok(factory6) => factory6.enum_adapter_by_gpu_preference(0, gpu_preference),
            Err(_) => factory.enum_adapters1(0),
        }
        .map_err(|res| {
            d3d12_error(
                debug_mode,
                None,
                "Could not find adapter for D3D12Device",
                res,
            )
        })?
    };

    // Get information about the selected adapter. Used for logging info.
    let adapter_desc = adapter
        .desc1()
        .map_err(|res| d3d12_error(debug_mode, None, "Could not get adapter description", res))?;
    let umd_version = adapter
        .check_interface_support(&IID_IDXGIDEVICE)
        .map_err(|res| {
            d3d12_error(
                debug_mode,
                None,
                "Could not get adapter driver version",
                res,
            )
        })?;

    let renderer_props = Properties::new();
    record_adapter(&renderer_props, &adapter_desc, umd_version, verbose_logs)?;

    // Load the D3D library
    let d3d12_dll = SharedObject::load(D3D12_DLL)
        .map_err(|_| set_string_error(debug_mode, &format!("Could not find {D3D12_DLL}")))?;

    // Load the CreateDevice function
    // SAFETY: the signature of d3d12.dll's export.
    let d3d12_create_device =
        unsafe { d3d12_dll.function::<PfnD3d12CreateDevice>(D3D12_CREATE_DEVICE_FUNC) }.map_err(
            |_| {
                set_string_error(
                    debug_mode,
                    &format!("Could not load function: {D3D12_CREATE_DEVICE_FUNC}"),
                )
            },
        )?;

    // SAFETY: as above.
    let serialize_root_signature = unsafe {
        d3d12_dll.function::<PfnD3d12SerializeRootSignature>(D3D12_SERIALIZE_ROOT_SIGNATURE_FUNC)
    }
    .map_err(|_| {
        set_string_error(
            debug_mode,
            &format!("Could not load function: {D3D12_SERIALIZE_ROOT_SIGNATURE_FUNC}"),
        )
    })?;

    // A device factory allows a D3D12 redistributable provided by the client to be loaded.
    let device_factory = agility_sdk_device_factory(&d3d12_dll, props);

    // Initialize the D3D12 debug layer, if applicable
    let mut d3d12_debug = None;
    if debug_mode {
        d3d12_debug = try_initialize_d3d12_debug(&d3d12_dll, device_factory.as_ref());
        let has_d3d12_debug = d3d12_debug.is_some();
        if has_dxgi_debug && has_d3d12_debug {
            crate::log::info!(
                Category::Gpu,
                "Validation layers enabled, expect debug level performance!"
            );
        } else if has_dxgi_debug || has_d3d12_debug {
            crate::log::warn!(
                Category::Gpu,
                "Validation layers partially enabled, some warnings may not be available"
            );
        } else {
            crate::log::warn!(
                Category::Gpu,
                "Validation layers not found, continuing without validation"
            );
        }
    }

    // Create the D3D12Device
    let device = match device_factory {
        Some(device_factory) => {
            device_factory.set_flags(D3D12_DEVICE_FACTORY_FLAG_ALLOW_RETURNING_EXISTING_DEVICE);
            device_factory.create_device(&adapter, D3D_FEATURE_LEVEL_CHOICE)
        }
        None => create_device_on(d3d12_create_device, &adapter),
    }
    .map_err(|res| d3d12_error(debug_mode, None, "Could not create D3D12Device", res))?;

    // Initialize the D3D12 debug info queue, if applicable
    let mut debug_info_queue = None;
    let mut info_queue_message_callback_supported = false;
    if debug_mode {
        debug_info_queue = try_initialize_d3d12_debug_info_queue(&device);
        info_queue_message_callback_supported = try_initialize_d3d12_debug_info_logger(&device);
    }

    // Check "GPU Upload Heap" support (for fast uniform buffers)
    let mut options16 = FeatureDataD3d12Options16::default(); // 15 wasn't enough, huh?
    let mut gpu_upload_heap_supported = false;
    let res = device.check_feature_support(D3D12_FEATURE_D3D12_OPTIONS16, &mut options16);

    if res >= 0 {
        gpu_upload_heap_supported = options16.gpu_upload_heap_supported != 0;
    }

    // Check for unrestricted texture-buffer copy pitch support
    let mut options13 = FeatureDataD3d12Options13::default();
    let mut unrestricted_buffer_texture_copy_pitch_supported = false;
    let res = device.check_feature_support(D3D12_FEATURE_D3D12_OPTIONS13, &mut options13);

    if res >= 0 {
        unrestricted_buffer_texture_copy_pitch_supported =
            options13.unrestricted_buffer_texture_copy_pitch_supported != 0;
    } else {
        crate::log::warn!(
            Category::Gpu,
            "CheckFeatureSupport for UnrestrictedBufferTextureCopyPitchSupported failed. You may need to provide a vendored D3D12Core.dll through the Agility SDK on older platforms."
        );
    }

    // Create command queue
    let queue_desc = CommandQueueDesc {
        flags: D3D12_COMMAND_QUEUE_FLAG_NONE,
        ty: D3D12_COMMAND_LIST_TYPE_DIRECT,
        node_mask: 0,
        priority: 0,
    };

    let command_queue = device.create_command_queue(&queue_desc).map_err(|res| {
        d3d12_error(
            debug_mode,
            Some(&device),
            "Could not create D3D12CommandQueue",
            res,
        )
    })?;

    // Create indirect command signatures
    let command_signature = |ty: u32, byte_stride: usize, error: &str| {
        let indirect_argument_desc = IndirectArgumentDesc::new(ty);
        let command_signature_desc = CommandSignatureDesc {
            node_mask: 0,
            byte_stride: byte_stride as u32,
            num_argument_descs: 1,
            argument_descs: &indirect_argument_desc,
        };
        device
            .create_command_signature(&command_signature_desc)
            .map_err(|res| d3d12_error(debug_mode, Some(&device), error, res))
    };

    let indirect_draw_command_signature = command_signature(
        D3D12_INDIRECT_ARGUMENT_TYPE_DRAW,
        size_of::<IndirectDrawCommand>(),
        "Could not create indirect draw command signature",
    )?;

    let indirect_indexed_draw_command_signature = command_signature(
        D3D12_INDIRECT_ARGUMENT_TYPE_DRAW_INDEXED,
        size_of::<IndexedIndirectDrawCommand>(),
        "Could not create indirect indexed draw command signature",
    )?;

    let indirect_dispatch_command_signature = command_signature(
        D3D12_INDIRECT_ARGUMENT_TYPE_DISPATCH,
        size_of::<IndirectDispatchCommand>(),
        "Could not create indirect dispatch command signature",
    )?;

    // Initialize pools

    // Initialize staging descriptor pools
    let staging_descriptor_pools = [
        create_staging_descriptor_pool(
            &device,
            debug_mode,
            D3D12_DESCRIPTOR_HEAP_TYPE_CBV_SRV_UAV,
        )?,
        create_staging_descriptor_pool(&device, debug_mode, D3D12_DESCRIPTOR_HEAP_TYPE_SAMPLER)?,
        create_staging_descriptor_pool(&device, debug_mode, D3D12_DESCRIPTOR_HEAP_TYPE_RTV)?,
        create_staging_descriptor_pool(&device, debug_mode, D3D12_DESCRIPTOR_HEAP_TYPE_DSV)?,
    ];
    const _: () = assert!(D3D12_DESCRIPTOR_HEAP_TYPE_NUM_TYPES == 4);

    // Initialize GPU descriptor heaps
    let gpu_heaps = |heap_type: u32| -> Result<GpuDescriptorHeapPool> {
        let count = if heap_type == D3D12_DESCRIPTOR_HEAP_TYPE_CBV_SRV_UAV {
            VIEW_GPU_DESCRIPTOR_COUNT
        } else {
            SAMPLER_GPU_DESCRIPTOR_COUNT
        };
        let heaps = (0..4)
            .map(|_| create_descriptor_heap(&device, debug_mode, heap_type, count, false))
            .collect::<Result<Vec<_>>>()?;
        Ok(GpuDescriptorHeapPool {
            heaps: Mutex::new(heaps),
        })
    };
    let gpu_descriptor_heap_pools = [
        gpu_heaps(D3D12_DESCRIPTOR_HEAP_TYPE_CBV_SRV_UAV)?,
        gpu_heaps(D3D12_DESCRIPTOR_HEAP_TYPE_SAMPLER)?,
    ];

    let semantic = props
        .get_string(crate::gpu::PROP_GPU_DEVICE_CREATE_D3D12_SEMANTIC_NAME_STRING)
        .unwrap_or_else(|| "TEXCOORD".to_owned());
    // (A name with a NUL ends there, as upstream's strdup'd string does.)
    let semantic =
        CString::new(semantic.split('\0').next().unwrap_or_default()).unwrap_or_default();

    let (xr_instance_handle, xr_system_id) = xr_instance
        .0
        .as_ref()
        .map_or((XR_NULL_HANDLE, XR_NULL_SYSTEM_ID), |init| {
            (init.instance, init.system_id)
        });
    let xr_pfns = xr_instance.0.as_ref().map(|init| init.xr);

    let renderer = D3D12Renderer {
        uniform_buffer_pool: Mutex::new(Vec::with_capacity(4)),
        dispose: Mutex::new(PendingDestroys::default()),
        blit: Mutex::new(BlitResources::default()),
        staging_descriptor_pools,
        gpu_descriptor_heap_pools,
        claimed_windows: Mutex::new(Vec::new()),
        submit_lock: Mutex::new(Vec::new()),
        command_buffer_pool: Mutex::new(Vec::new()),
        fence_pool: Mutex::new(Vec::new()),
        allowed_frames_in_flight: AtomicU32::new(2),
        props: renderer_props,
        semantic,
        debug_mode,
        gpu_upload_heap_supported,
        unrestricted_buffer_texture_copy_pitch_supported,
        supports_tearing,
        info_queue_message_callback_supported,
        winpixeventruntime_fns,
        serialize_root_signature,
        indirect_draw_command_signature,
        indirect_indexed_draw_command_signature,
        indirect_dispatch_command_signature,
        command_queue,
        debug_info_queue,
        device,
        d3d12_debug,
        adapter,
        factory,
        dxgi_info_queue,
        dxgi_debug: dxgi_debug.map(LiveObjectReporter),
        d3d12_dll,
        dxgi_dll,
        dxgidebug_dll,
        winpixeventruntime_dll,
        xr_instance: xr_instance_handle,
        xr_system_id,
        xr: xr_pfns,
    };
    // (the device has the instance now, and the application next)
    xr_instance.0 = None;

    // Blit resources
    let blit = renderer.init_blit_resources();
    *lock(&renderer.blit) = blit;

    // Create the SDL_GPU Device
    let mut shader_formats = ShaderFormat::DXBC;

    let mut shader_model = FeatureDataShaderModel {
        highest_shader_model: D3D_SHADER_MODEL_6_0,
    };

    let res = renderer
        .device
        .check_feature_support(D3D12_FEATURE_SHADER_MODEL, &mut shader_model);
    if res >= 0 && shader_model.highest_shader_model >= D3D_SHADER_MODEL_6_0 {
        shader_formats |= ShaderFormat::DXIL;
    }

    Ok((renderer, shader_formats))
}
