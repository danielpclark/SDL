// Rust translation of the device parts of src/gpu/d3d12/SDL_gpu_d3d12.c from
// Simple DirectMedia Layer: PrepareDriver, the debug layers, the blit
// resources and CreateDevice.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! Device creation: `D3D12_PrepareDriver()` (can this machine create a
//! Direct3D 12 device for the application's shader formats?),
//! `D3D12_CreateDevice()` with the adapter selection, the vendored
//! Direct3D 12 runtime of the Agility SDK, the DXGI and Direct3D 12 debug
//! layers and their message logging, and the blit resources.
//!
//! The Xbox paths (`SDL_PLATFORM_XBOXONE`, `SDL_PLATFORM_XBOXSERIES`) and
//! OpenXR (`HAVE_GPU_OPENXR`) are not translated. Upstream also builds this
//! backend on other systems over DXVK's `libdxvk_d3d12.so`; here it is
//! Windows only.

use std::ffi::{c_char, c_void, CStr, CString};
use std::sync::atomic::AtomicU32;
use std::sync::Mutex;

use super::d3d::*;
use super::descriptors::{
    create_descriptor_heap, create_staging_descriptor_pool, GpuDescriptorHeapPool,
};
use super::resources::{D3D12Sampler, PendingDestroys};
use super::shaders::*;
use super::{
    d3d12_error, lock, set_string_error, BlitResources, D3D12Renderer, LiveObjectReporter,
    WinPixEventRuntimeFns, D3D12_DLL, D3D_FEATURE_LEVEL_CHOICE, D3D_FEATURE_LEVEL_CHOICE_STR,
    DXGIDEBUG_DLL, DXGI_DLL, SAMPLER_GPU_DESCRIPTOR_COUNT, VIEW_GPU_DESCRIPTOR_COUNT,
    WINPIXEVENTRUNTIME_DLL,
};
use crate::core::windows::{is_windows_11_or_greater, wide_to_utf8};
use crate::error::Result;
use crate::gpu::sysgpu::{BackendObject, BlitPipelineCache};
use crate::gpu::{
    CompareOp, Filter, IndexedIndirectDrawCommand, IndirectDispatchCommand, IndirectDrawCommand,
    Sampler, SamplerAddressMode, SamplerCreateInfo, SamplerMipmapMode, Shader, ShaderCreateInfo,
    ShaderFormat, ShaderStage,
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
/// shader formats. Translation of `D3D12_PrepareDriver()`.
///
/// Note (upstream): C leaks `d3d12.dll` when `dxgi.dll` or its
/// `CreateDXGIFactory1()` can't be loaded; the library is unloaded here.
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

    // (HAVE_GPU_OPENXR: OpenXR is not translated.)

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
/// formats it takes (DXBC, and DXIL with shader model 6).
///
/// Upstream frees what it made so far (`D3D12_INTERNAL_DestroyRenderer()`)
/// when a step fails; here that is dropping it.
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

    // (HAVE_GPU_OPENXR: OpenXR is not translated.)

    // Select the appropriate device for rendering
    let gpu_preference = if prefer_low_power {
        DXGI_GPU_PREFERENCE_MINIMUM_POWER
    } else {
        DXGI_GPU_PREFERENCE_HIGH_PERFORMANCE
    };
    let adapter = match factory.query::<IDXGIFactory6Vtbl>(&IID_IDXGIFACTORY6) {
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
    })?;

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
    };

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
