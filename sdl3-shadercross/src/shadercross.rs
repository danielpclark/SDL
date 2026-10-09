// Rust translation of src/SDL_shadercross.c from SDL_shadercross.
// Copyright (C) 2024 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! The SDL_shadercross library (`SDL_shadercross.c`).
//!
//! This is the build without DXC (`SDL_SHADERCROSS_DXC` undefined): the
//! DXIL and HLSL-to-SPIR-V paths fail with upstream's messages. DXBC comes
//! from `d3dcompiler_47.dll` on Windows; upstream's non-Windows fallback,
//! the vkd3d-utils library, isn't a system library and isn't loaded.

use std::sync::Mutex;

use sdl3::gpu::{
    ComputePipeline, ComputePipelineCreateInfo, Device, Shader, ShaderCreateInfo, ShaderFormat,
    PROP_GPU_COMPUTEPIPELINE_CREATE_NAME_STRING, PROP_GPU_SHADER_CREATE_NAME_STRING,
};
use sdl3::properties::Properties;
use sdl3::{Error, Result};

use crate::spirv_cross::c_api::*;
use crate::spirv_cross::spirv::*;
use crate::{
    ComputePipelineMetadata, GraphicsShaderMetadata, GraphicsShaderResourceInfo, HlslInfo,
    IOVarMetadata, IOVarType, ShaderStage, SpirvInfo, PROP_HLSL_SKIP_SPIRV_ROUNDTRIP_BOOLEAN,
    PROP_SHADER_DEBUG_ENABLE_BOOLEAN, PROP_SHADER_DEBUG_NAME_STRING, PROP_SPIRV_MSL_VERSION_STRING,
    PROP_SPIRV_PSSL_COMPATIBILITY_BOOLEAN,
};

/* Constants */
// (MAX_DEFINES and MAX_DEFINE_STRING_LENGTH are only used by the DXC path.)

fn get_boolean_property(props: Option<&Properties>, name: &str, default_value: bool) -> bool {
    props
        .and_then(|p| p.get_bool(name))
        .unwrap_or(default_value)
}

fn get_string_property(props: Option<&Properties>, name: &str) -> Option<String> {
    props.and_then(|p| p.get_string(name))
}

/* DXIL via DXC */

fn compile_using_dxc(info: &HlslInfo<'_>, spirv: bool) -> Result<Vec<u8>> {
    let _ = (info, spirv);
    // (SDL_SHADERCROSS_DXC isn't defined in this build.)
    Err(Error::new(
        "Shadercross was not built with DXC support, cannot compile using DXC!",
    ))
}

/// Compile DXIL bytecode from HLSL code. Translation of
/// `SDL_ShaderCross_CompileDXILFromHLSL()`.
///
/// This build has no DXC, so this fails like upstream's build without it.
pub fn compile_dxil_from_hlsl(info: &HlslInfo<'_>) -> Result<Vec<u8>> {
    if get_boolean_property(info.props, PROP_HLSL_SKIP_SPIRV_ROUNDTRIP_BOOLEAN, false) {
        return compile_using_dxc(info, false);
    }

    // Roundtrip to SPIR-V to support things like Structured Buffers.
    let spirv = compile_spirv_from_hlsl(info)?;

    let spirv_info = SpirvInfo {
        bytecode: &spirv,
        entrypoint: info.entrypoint,
        shader_stage: info.shader_stage,
        props: info.props,
    };

    let translated_source = transpile_hlsl_from_spirv(&spirv_info)?;

    let mut translated_hlsl_info = *info;
    translated_hlsl_info.source = &translated_source;

    compile_using_dxc(&translated_hlsl_info, false)
}

/// Compile SPIR-V bytecode from HLSL code. Translation of
/// `SDL_ShaderCross_CompileSPIRVFromHLSL()`.
///
/// This build has no DXC, so this fails like upstream's build without it.
pub fn compile_spirv_from_hlsl(info: &HlslInfo<'_>) -> Result<Vec<u8>> {
    compile_using_dxc(info, true)
}

/* DXBC via FXC */

/// D3DCompiler, loaded from `d3dcompiler_47.dll` by [`init`].
#[cfg(windows)]
#[allow(unsafe_code)]
mod d3dcompiler {
    use std::ffi::{c_char, c_void, CString};

    use sdl3::loadso::SharedObject;
    use sdl3::{Error, Result};

    #[allow(clippy::upper_case_acronyms)]
    type HRESULT = i32;

    /* ID3DBlob definition, used by both D3DCompiler and DXCompiler */
    #[repr(C)]
    struct ID3DBlobVtbl {
        query_interface:
            unsafe extern "system" fn(*mut ID3DBlob, *const c_void, *mut *mut c_void) -> HRESULT,
        add_ref: unsafe extern "system" fn(*mut ID3DBlob) -> u32,
        release: unsafe extern "system" fn(*mut ID3DBlob) -> u32,
        get_buffer_pointer: unsafe extern "system" fn(*mut ID3DBlob) -> *mut c_void,
        get_buffer_size: unsafe extern "system" fn(*mut ID3DBlob) -> usize,
    }

    #[repr(C)]
    struct ID3DBlob {
        lp_vtbl: *const ID3DBlobVtbl,
    }

    type PfnD3DCompile = unsafe extern "system" fn(
        p_src_data: *const c_void,
        src_data_size: usize,
        p_source_name: *const c_char,
        p_defines: *const c_void,
        p_include: *mut c_void,
        p_entrypoint: *const c_char,
        p_target: *const c_char,
        flags1: u32,
        flags2: u32,
        pp_code: *mut *mut ID3DBlob,
        pp_error_msgs: *mut *mut ID3DBlob,
    ) -> HRESULT;

    #[derive(Debug)]
    pub(super) struct D3DCompiler {
        // (Kept loaded while `d3d_compile` is in use.)
        _dll: SharedObject,
        d3d_compile: PfnD3DCompile,
    }

    /// A blob's contents, copied out, with the blob released.
    unsafe fn take_blob(blob: *mut ID3DBlob) -> Vec<u8> {
        // SAFETY: (caller) `blob` is a live ID3DBlob D3DCompile returned.
        unsafe {
            let vtbl = &*(*blob).lp_vtbl;
            let size = (vtbl.get_buffer_size)(blob);
            let ptr = (vtbl.get_buffer_pointer)(blob) as *const u8;
            let out = if ptr.is_null() || size == 0 {
                Vec::new()
            } else {
                std::slice::from_raw_parts(ptr, size).to_vec()
            };
            (vtbl.release)(blob);
            out
        }
    }

    impl D3DCompiler {
        pub(super) fn load(name: &str) -> Option<D3DCompiler> {
            let dll = SharedObject::load(name).ok()?;
            let sym = dll.symbol("D3DCompile").ok()?;
            // SAFETY: D3DCompile has this signature (d3dcompiler.h).
            let d3d_compile =
                unsafe { std::mem::transmute::<*mut c_void, PfnD3DCompile>(sym.as_ptr()) };
            Some(D3DCompiler {
                _dll: dll,
                d3d_compile,
            })
        }

        // FIXME: includes and defines
        pub(super) fn compile(
            &self,
            hlsl_source: &str,
            entrypoint: &str,
            shader_profile: &str,
            enable_debug: bool,
        ) -> Result<Vec<u8>> {
            let entry = CString::new(entrypoint).map_err(|_| Error::invalid_param("entrypoint"))?;
            let profile =
                CString::new(shader_profile).map_err(|_| Error::invalid_param("shaderProfile"))?;
            let mut blob: *mut ID3DBlob = std::ptr::null_mut();
            let mut error_blob: *mut ID3DBlob = std::ptr::null_mut();

            // SAFETY: the arguments are valid for the documented D3DCompile
            // contract; the source is passed with its length.
            let ret = unsafe {
                (self.d3d_compile)(
                    hlsl_source.as_ptr() as *const c_void,
                    hlsl_source.len(),
                    std::ptr::null(),
                    std::ptr::null(),
                    std::ptr::null_mut(),
                    entry.as_ptr(),
                    profile.as_ptr(),
                    if enable_debug { 1 } else { 0 }, // D3DCOMPILE_DEBUG = 1
                    0,
                    &mut blob,
                    &mut error_blob,
                )
            };

            if ret < 0 {
                if !blob.is_null() {
                    // SAFETY: a blob D3DCompile returned.
                    unsafe { take_blob(blob) };
                }
                if !error_blob.is_null() {
                    // SAFETY: a blob D3DCompile returned.
                    let mut msg = unsafe { take_blob(error_blob) };
                    if let Some(nul) = msg.iter().position(|&b| b == 0) {
                        msg.truncate(nul);
                    }
                    return Err(Error::new(format!(
                        "HLSL compilation failed: {}",
                        String::from_utf8_lossy(&msg)
                    )));
                } else {
                    return Err(Error::new("HLSL compilation failed for an unknown reason."));
                }
            }

            if !error_blob.is_null() {
                // SAFETY: a blob D3DCompile returned (warnings).
                unsafe { take_blob(error_blob) };
            }
            if blob.is_null() {
                return Err(Error::new("HLSL compilation failed for an unknown reason."));
            }
            // SAFETY: a blob D3DCompile returned.
            Ok(unsafe { take_blob(blob) })
        }
    }
}

/// D3DCompiler stand-in where there's no system D3DCompiler to load.
#[cfg(not(windows))]
mod d3dcompiler {
    use sdl3::Result;

    #[derive(Debug)]
    pub(super) struct D3DCompiler {}

    impl D3DCompiler {
        pub(super) fn load(_name: &str) -> Option<D3DCompiler> {
            // (Upstream loads libvkd3d-utils here, which isn't a system
            // library; DXBC isn't available outside Windows.)
            None
        }

        pub(super) fn compile(&self, _: &str, _: &str, _: &str, _: bool) -> Result<Vec<u8>> {
            unreachable!("no D3DCompiler is ever loaded on this platform")
        }
    }
}

/* Dynamic Library / Linking */
#[cfg(windows)]
const D3DCOMPILER_DLL: &str = "d3dcompiler_47.dll";
#[cfg(target_vendor = "apple")]
const D3DCOMPILER_DLL: &str = "libvkd3d-utils.1.dylib";
#[cfg(not(any(windows, target_vendor = "apple")))]
const D3DCOMPILER_DLL: &str = "libvkd3d-utils.so.1";

/* D3DCompiler */
static D3DCOMPILER: Mutex<Option<d3dcompiler::D3DCompiler>> = Mutex::new(None);

fn d3dcompiler_loaded() -> bool {
    D3DCOMPILER
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .is_some()
}

// FIXME: includes and defines
fn compile_dxbc(
    hlsl_source: &str,
    entrypoint: &str,
    shader_profile: &str,
    enable_debug: bool,
) -> Result<Vec<u8>> {
    let guard = D3DCOMPILER.lock().unwrap_or_else(|e| e.into_inner());
    let Some(d3dcompiler) = guard.as_ref() else {
        return Err(Error::new("Could not load D3DCompile!"));
    };

    d3dcompiler.compile(hlsl_source, entrypoint, shader_profile, enable_debug)
}

fn compile_dxbc_from_hlsl_internal(info: &HlslInfo<'_>, enable_roundtrip: bool) -> Result<Vec<u8>> {
    let mut transpiled_source: Option<String> = None;

    if enable_roundtrip {
        // Need to roundtrip to SM 5.1
        let spirv = compile_spirv_from_hlsl(info)?;

        let spirv_info = SpirvInfo {
            bytecode: &spirv,
            entrypoint: info.entrypoint,
            shader_stage: info.shader_stage,
            props: info.props,
        };

        transpiled_source = Some(transpile_hlsl_from_spirv(&spirv_info)?);
    }

    let shader_profile = match info.shader_stage {
        ShaderStage::Vertex => "vs_5_1",
        ShaderStage::Fragment => "ps_5_1",
        // compute
        ShaderStage::Compute => "cs_5_1",
    };

    compile_dxbc(
        transpiled_source.as_deref().unwrap_or(info.source),
        info.entrypoint,
        shader_profile,
        get_boolean_property(info.props, PROP_SHADER_DEBUG_ENABLE_BOOLEAN, false),
    )
}

/// Compile DXBC bytecode from HLSL code; returns the raw bytecode.
/// Translation of `SDL_ShaderCross_CompileDXBCFromHLSL()`.
pub fn compile_dxbc_from_hlsl(info: &HlslInfo<'_>) -> Result<Vec<u8>> {
    compile_dxbc_from_hlsl_internal(
        info,
        !get_boolean_property(info.props, PROP_HLSL_SKIP_SPIRV_ROUNDTRIP_BOOLEAN, false),
    )
}

/// `SPVC_ERROR(func)`.
fn spvc_error(func: &str, context: &SpvcContext) -> Error {
    Error::new(format!(
        "{func} failed: {}",
        context.get_last_error_string()
    ))
}

/// `SDL_sscanf(str, "%u.%u.%u", ...)` (as glibc's `sscanf` reads `%u`) and
/// the version arithmetic of `parse_version_number()`.
fn parse_version_number(s: &str) -> i32 {
    let b = s.as_bytes();
    let mut pos = 0;
    let scan_u = |pos: &mut usize| -> Option<u32> {
        while *pos < b.len() && b[*pos].is_ascii_whitespace() {
            *pos += 1;
        }
        let mut negative = false;
        if *pos < b.len() && (b[*pos] == b'-' || b[*pos] == b'+') {
            negative = b[*pos] == b'-';
            *pos += 1;
        }
        let start = *pos;
        let mut value: u64 = 0;
        let mut overflow = false;
        while *pos < b.len() && b[*pos].is_ascii_digit() {
            match value
                .checked_mul(10)
                .and_then(|v| v.checked_add(u64::from(b[*pos] - b'0')))
            {
                Some(v) => value = v,
                None => overflow = true,
            }
            *pos += 1;
        }
        if *pos == start {
            return None;
        }
        if overflow {
            value = u64::MAX;
        } else if negative {
            value = value.wrapping_neg();
        }
        Some(value as u32)
    };
    let Some(major) = scan_u(&mut pos) else {
        return -1;
    };
    if b.get(pos) != Some(&b'.') {
        return -1;
    }
    pos += 1;
    let Some(minor) = scan_u(&mut pos) else {
        return -1;
    };
    if b.get(pos) != Some(&b'.') {
        return -1;
    }
    pos += 1;
    let Some(patch) = scan_u(&mut pos) else {
        return -1;
    };
    (major.wrapping_mul(10000))
        .wrapping_add(minor.wrapping_mul(100))
        .wrapping_add(patch) as i32
}

/// `SPIRVTranspileContext`: the translated source and the entry point's
/// name in it.
#[derive(Debug)]
struct TranspileContext {
    translated_source: String,
    /// `None` where the C API returns NULL.
    cleansed_entrypoint: Option<String>,
}

/// The SPIR-V words of a bytecode buffer (`code` cast to `const SpvId *`,
/// `codeSize / sizeof(SpvId)` of them).
fn spirv_words(code: &[u8]) -> Result<Vec<u32>> {
    let mut words = Vec::new();
    if words.try_reserve_exact(code.len() / 4).is_err() {
        return Err(Error::out_of_memory());
    }
    words.extend(
        code.chunks_exact(4)
            .map(|c| u32::from_ne_bytes([c[0], c[1], c[2], c[3]])),
    );
    Ok(words)
}

/// The descriptor set and binding checks shared by the resource loops of
/// `SDL_ShaderCross_INTERNAL_TranspileFromSPIRV()` and the reflection
/// functions.
fn check_set_and_binding(compiler: &SpvcCompiler, id: u32) -> Result<()> {
    if !compiler.has_decoration(id, DecorationDescriptorSet)
        || !compiler.has_decoration(id, DecorationBinding)
    {
        return Err(Error::new(
            "Shader resources must have descriptor set and binding index!",
        ));
    }
    Ok(())
}

fn resource_list<'a>(
    context: &mut SpvcContext,
    resources: &'a SpvcResources,
    type_: SpvcResourceType,
) -> Result<&'a [SpvcReflectedResource]> {
    resources
        .get_resource_list_for_type(context, type_)
        .map_err(|_| spvc_error("spvc_resources_get_resource_list_for_type", context))
}

fn binding_2(stage: ExecutionModel, desc_set: u32, binding: u32) -> SpvcMslResourceBinding2 {
    // FIXME (upstream): the compute path doesn't zero its binding arrays,
    // leaving the MSL indices each resource doesn't use uninitialized, and
    // the buffer of an emulated image atomic takes its texture binding's
    // uninitialized msl_buffer; zero here (as a zero-initializing build).
    SpvcMslResourceBinding2 {
        stage,
        desc_set,
        binding,
        count: 1,
        ..Default::default()
    }
}

fn transpile_from_spirv(
    backend: SpvcBackend,
    shadermodel: u32,          // only used for HLSL
    shader_stage: ShaderStage, // only used for MSL
    code: &[u8],
    entrypoint: &str,
    props: Option<&Properties>,
) -> Result<TranspileContext> {
    /* Create the SPIRV-Cross context */
    let mut context = SpvcContext::create();

    /* Parse the SPIR-V into IR */
    let ir = context
        .parse_spirv(&spirv_words(code)?)
        .map_err(|_| spvc_error("spvc_context_parse_spirv", &context))?;

    /* Create the cross-compiler */
    let mut compiler = context
        .create_compiler(backend, ir)
        .map_err(|_| spvc_error("spvc_context_create_compiler", &context))?;

    /* Set up the cross-compiler options */
    let mut options = compiler.create_compiler_options();

    if backend == SpvcBackend::Hlsl {
        options.set_uint(
            &mut context,
            SPVC_COMPILER_OPTION_HLSL_SHADER_MODEL,
            shadermodel,
        );
        options.set_uint(
            &mut context,
            SPVC_COMPILER_OPTION_HLSL_NONWRITABLE_UAV_TEXTURE_AS_SRV,
            1,
        );
        options.set_uint(
            &mut context,
            SPVC_COMPILER_OPTION_HLSL_FLATTEN_MATRIX_VERTEX_INPUT_SEMANTICS,
            1,
        );
        options.set_bool(
            &mut context,
            SPVC_COMPILER_OPTION_HLSL_USE_ENTRY_POINT_NAME,
            !get_boolean_property(props, PROP_SPIRV_PSSL_COMPATIBILITY_BOOLEAN, false),
        );
        options.set_bool(
            &mut context,
            SPVC_COMPILER_OPTION_HLSL_POINT_SIZE_COMPAT,
            true,
        );
    }

    let execution_model = match shader_stage {
        ShaderStage::Vertex => ExecutionModelVertex,
        ShaderStage::Fragment => ExecutionModelFragment,
        // compute
        ShaderStage::Compute => {
            if backend == SpvcBackend::Hlsl {
                ExecutionModelKernel
            } else {
                ExecutionModelGLCompute
            }
        }
    };

    if backend == SpvcBackend::Msl {
        let msl_version_string = get_string_property(props, PROP_SPIRV_MSL_VERSION_STRING);
        let msl_version_string = msl_version_string.as_deref().unwrap_or("1.2.0");
        let msl_version = parse_version_number(msl_version_string);
        if msl_version == -1 {
            return Err(Error::new(format!(
                "failed to parse MSL version string \"{msl_version_string}\""
            )));
        }
        options.set_uint(
            &mut context,
            SPVC_COMPILER_OPTION_MSL_VERSION,
            msl_version as u32,
        );
    }

    // MSL doesn't have descriptor sets, so we have to set up index remapping
    if backend == SpvcBackend::Msl && shader_stage != ShaderStage::Compute {
        // FIXME (upstream): the binding lists are fixed arrays of 32, overrun by
        // a shader with more resources; they grow here.
        let mut buffer_bindings: Vec<SpvcMslResourceBinding2> = Vec::new();
        let mut texture_bindings: Vec<SpvcMslResourceBinding2> = Vec::new();
        let mut num_separate_samplers = 0;

        let active_variables = compiler
            .get_active_interface_variables(&mut context)
            .map_err(|_| spvc_error("spvc_compiler_get_active_interface_variables", &context))?;

        let resources = compiler
            .create_shader_resources_for_active_variables(&mut context, &active_variables)
            .map_err(|_| {
                spvc_error(
                    "spvc_compiler_create_shader_resources_for_active_variables",
                    &context,
                )
            })?;

        // Combined texture-samplers
        let mut reflected_resources =
            resource_list(&mut context, &resources, SPVC_RESOURCE_TYPE_SAMPLED_IMAGE)?;

        // If source is HLSL, we might have separate images and samplers
        if reflected_resources.is_empty() {
            reflected_resources = resource_list(
                &mut context,
                &resources,
                SPVC_RESOURCE_TYPE_SEPARATE_SAMPLERS,
            )?;
            num_separate_samplers = reflected_resources.len();
        }

        for r in reflected_resources {
            check_set_and_binding(&compiler, r.id)?;

            let descriptor_set_index = compiler.get_decoration(r.id, DecorationDescriptorSet);
            if !(descriptor_set_index == 0 || descriptor_set_index == 2) {
                return Err(Error::new(
                    "Descriptor set index for graphics texture-sampler must be 0 or 2!",
                ));
            }

            let binding_index = compiler.get_decoration(r.id, DecorationBinding);

            // assign binding index after we have collected all resources
            texture_bindings.push(binding_2(
                execution_model,
                descriptor_set_index,
                binding_index,
            ));
        }

        // Storage textures
        let reflected_resources =
            resource_list(&mut context, &resources, SPVC_RESOURCE_TYPE_STORAGE_IMAGE)?;

        for r in reflected_resources {
            check_set_and_binding(&compiler, r.id)?;

            let descriptor_set_index = compiler.get_decoration(r.id, DecorationDescriptorSet);
            if !(descriptor_set_index == 0 || descriptor_set_index == 2) {
                return Err(Error::new(
                    "Descriptor set index for graphics storage texture must be 0 or 2!",
                ));
            }

            let binding_index = compiler.get_decoration(r.id, DecorationBinding);

            // assign binding index after we have collected all resources
            texture_bindings.push(binding_2(
                execution_model,
                descriptor_set_index,
                binding_index,
            ));
        }

        // If source is HLSL, storage images might be marked as separate images
        let reflected_resources =
            resource_list(&mut context, &resources, SPVC_RESOURCE_TYPE_SEPARATE_IMAGE)?;

        // We only want to iterate the images that don't have an associated sampler
        for r in reflected_resources.iter().skip(num_separate_samplers) {
            check_set_and_binding(&compiler, r.id)?;

            let descriptor_set_index = compiler.get_decoration(r.id, DecorationDescriptorSet);
            if !(descriptor_set_index == 0 || descriptor_set_index == 2) {
                return Err(Error::new(
                    "Descriptor set index for graphics storage texture must be 0 or 2!",
                ));
            }

            let binding_index = compiler.get_decoration(r.id, DecorationBinding);

            // assign binding index after we have collected all resources
            texture_bindings.push(binding_2(
                execution_model,
                descriptor_set_index,
                binding_index,
            ));
        }

        // Uniform buffers
        let reflected_resources =
            resource_list(&mut context, &resources, SPVC_RESOURCE_TYPE_UNIFORM_BUFFER)?;

        for r in reflected_resources {
            check_set_and_binding(&compiler, r.id)?;

            let descriptor_set_index = compiler.get_decoration(r.id, DecorationDescriptorSet);
            if !(descriptor_set_index == 1 || descriptor_set_index == 3) {
                return Err(Error::new(
                    "Descriptor set index for graphics uniform buffer must be 1 or 3!",
                ));
            }

            let binding_index = compiler.get_decoration(r.id, DecorationBinding);

            // assign binding index after we have collected all resources
            buffer_bindings.push(binding_2(
                execution_model,
                descriptor_set_index,
                binding_index,
            ));
        }

        // Storage buffers
        let reflected_resources =
            resource_list(&mut context, &resources, SPVC_RESOURCE_TYPE_STORAGE_BUFFER)?;

        for r in reflected_resources {
            check_set_and_binding(&compiler, r.id)?;

            let descriptor_set_index = compiler.get_decoration(r.id, DecorationDescriptorSet);
            if !(descriptor_set_index == 0 || descriptor_set_index == 2) {
                return Err(Error::new(
                    "Descriptor set index for graphics storage buffer must be 0 or 2!",
                ));
            }

            let binding_index = compiler.get_decoration(r.id, DecorationBinding);

            // assign binding index after we have collected all resources
            buffer_bindings.push(binding_2(
                execution_model,
                descriptor_set_index,
                binding_index,
            ));
        }

        // Textures come first so we can just use the binding slot
        let mut result = SPVC_SUCCESS;
        for b in texture_bindings.iter_mut() {
            b.msl_texture = b.binding;
            b.msl_sampler = b.binding;
            result = compiler.msl_add_resource_binding_2(&mut context, b);
        }

        if result < 0 {
            return Err(spvc_error(
                "spvc_compiler_msl_add_resource_binding_2",
                &context,
            ));
        }

        // Calculate number of uniform buffers
        let uniform_buffer_count = buffer_bindings
            .iter()
            .filter(|b| b.desc_set == 1 || b.desc_set == 3)
            .count() as u32;
        let num_texture_bindings = texture_bindings.len() as u32;

        // Calculate resource indices
        for b in buffer_bindings.iter_mut() {
            if b.desc_set == 1 || b.desc_set == 3 {
                // Uniform buffers are alone in the descriptor set
                b.msl_buffer = b.binding;
            } else {
                // Subtract by the texture count because the textures precede the storage buffers in the descriptor set
                b.msl_buffer =
                    uniform_buffer_count.wrapping_add(b.binding.wrapping_sub(num_texture_bindings));
            }

            if compiler.msl_add_resource_binding_2(&mut context, b) < 0 {
                return Err(spvc_error(
                    "spvc_compiler_msl_add_resource_binding_2",
                    &context,
                ));
            }
        }
    }

    if backend == SpvcBackend::Msl && shader_stage == ShaderStage::Compute {
        // FIXME (upstream): the binding lists are fixed arrays of 32, overrun by
        // a shader with more resources; they grow here.
        let mut buffer_bindings: Vec<SpvcMslResourceBinding2> = Vec::new();
        let mut texture_bindings: Vec<SpvcMslResourceBinding2> = Vec::new();
        let mut num_separate_samplers = 0;

        let active_variables = compiler
            .get_active_interface_variables(&mut context)
            .map_err(|_| spvc_error("spvc_compiler_get_active_interface_variables", &context))?;

        let resources = compiler
            .create_shader_resources_for_active_variables(&mut context, &active_variables)
            .map_err(|_| {
                spvc_error(
                    "spvc_compiler_create_shader_resources_for_active_variables",
                    &context,
                )
            })?;

        // Combined texture-samplers
        let mut reflected_resources =
            resource_list(&mut context, &resources, SPVC_RESOURCE_TYPE_SAMPLED_IMAGE)?;

        // If source is HLSL, we might have separate images and samplers
        if reflected_resources.is_empty() {
            reflected_resources = resource_list(
                &mut context,
                &resources,
                SPVC_RESOURCE_TYPE_SEPARATE_SAMPLERS,
            )?;
            num_separate_samplers = reflected_resources.len();
        }

        for r in reflected_resources {
            check_set_and_binding(&compiler, r.id)?;

            let descriptor_set_index = compiler.get_decoration(r.id, DecorationDescriptorSet);
            if descriptor_set_index != 0 {
                return Err(Error::new(
                    "Descriptor set index for compute texture-sampler must be 0!",
                ));
            }

            let binding_index = compiler.get_decoration(r.id, DecorationBinding);

            // assign binding index after we have collected all resources
            texture_bindings.push(binding_2(
                execution_model,
                descriptor_set_index,
                binding_index,
            ));
        }

        // Readonly storage textures
        let reflected_resources =
            resource_list(&mut context, &resources, SPVC_RESOURCE_TYPE_STORAGE_IMAGE)?;

        for r in reflected_resources {
            check_set_and_binding(&compiler, r.id)?;

            let descriptor_set_index = compiler.get_decoration(r.id, DecorationDescriptorSet);
            if !(descriptor_set_index == 0 || descriptor_set_index == 1) {
                return Err(Error::new(
                    "Descriptor set index for compute storage texture must be 0 or 1!",
                ));
            }

            // Skip readwrite textures
            if descriptor_set_index != 0 {
                continue;
            }

            let binding_index = compiler.get_decoration(r.id, DecorationBinding);

            // assign binding index after we have collected all resources
            texture_bindings.push(binding_2(
                execution_model,
                descriptor_set_index,
                binding_index,
            ));
        }

        // If source is HLSL, storage images might be marked as separate images
        let reflected_resources =
            resource_list(&mut context, &resources, SPVC_RESOURCE_TYPE_SEPARATE_IMAGE)?;

        // We only want to iterate the images that don't have an associated sampler
        for r in reflected_resources.iter().skip(num_separate_samplers) {
            check_set_and_binding(&compiler, r.id)?;

            let descriptor_set_index = compiler.get_decoration(r.id, DecorationDescriptorSet);
            if !(descriptor_set_index == 0 || descriptor_set_index == 1) {
                return Err(Error::new(
                    "Descriptor set index for compute storage texture must be 0 or 1!",
                ));
            }

            // Skip readwrite textures
            if descriptor_set_index != 0 {
                continue;
            }

            let binding_index = compiler.get_decoration(r.id, DecorationBinding);

            // assign binding index after we have collected all resources
            texture_bindings.push(binding_2(
                execution_model,
                descriptor_set_index,
                binding_index,
            ));
        }

        // Readwrite storage textures
        let reflected_resources =
            resource_list(&mut context, &resources, SPVC_RESOURCE_TYPE_STORAGE_IMAGE)?;

        for r in reflected_resources {
            let descriptor_set_index = compiler.get_decoration(r.id, DecorationDescriptorSet);

            // Skip readonly textures
            if descriptor_set_index != 1 {
                continue;
            }

            let binding_index = compiler.get_decoration(r.id, DecorationBinding);

            // assign binding index after we have collected all resources
            texture_bindings.push(binding_2(
                execution_model,
                descriptor_set_index,
                binding_index,
            ));
        }

        // If source is HLSL, storage images might be marked as separate images
        let reflected_resources =
            resource_list(&mut context, &resources, SPVC_RESOURCE_TYPE_SEPARATE_IMAGE)?;

        // We only want to iterate the images that don't have an associated sampler
        for r in reflected_resources.iter().skip(num_separate_samplers) {
            let descriptor_set_index = compiler.get_decoration(r.id, DecorationDescriptorSet);

            // Skip readonly textures
            if descriptor_set_index != 1 {
                continue;
            }

            let binding_index = compiler.get_decoration(r.id, DecorationBinding);

            // assign binding index after we have collected all resources
            texture_bindings.push(binding_2(
                execution_model,
                descriptor_set_index,
                binding_index,
            ));
        }

        // Uniform buffers
        let reflected_resources =
            resource_list(&mut context, &resources, SPVC_RESOURCE_TYPE_UNIFORM_BUFFER)?;

        for r in reflected_resources {
            check_set_and_binding(&compiler, r.id)?;

            let descriptor_set_index = compiler.get_decoration(r.id, DecorationDescriptorSet);
            if descriptor_set_index != 2 {
                return Err(Error::new(
                    "Descriptor set index for compute uniform buffer must be 2!",
                ));
            }

            let binding_index = compiler.get_decoration(r.id, DecorationBinding);

            // assign binding index after we have collected all resources
            buffer_bindings.push(binding_2(
                execution_model,
                descriptor_set_index,
                binding_index,
            ));
        }

        // Storage buffers
        let reflected_resources =
            resource_list(&mut context, &resources, SPVC_RESOURCE_TYPE_STORAGE_BUFFER)?;

        // Readonly storage buffers
        for r in reflected_resources {
            check_set_and_binding(&compiler, r.id)?;

            let descriptor_set_index = compiler.get_decoration(r.id, DecorationDescriptorSet);
            if !(descriptor_set_index == 0 || descriptor_set_index == 1) {
                return Err(Error::new(
                    "Descriptor set index for compute storage buffer must be 0 or 1!",
                ));
            }

            // Skip readwrite buffers
            if descriptor_set_index != 0 {
                continue;
            }

            let binding_index = compiler.get_decoration(r.id, DecorationBinding);

            // assign binding index after we have collected all resources
            buffer_bindings.push(binding_2(
                execution_model,
                descriptor_set_index,
                binding_index,
            ));
        }

        // Readwrite storage buffers
        for r in reflected_resources {
            let descriptor_set_index = compiler.get_decoration(r.id, DecorationDescriptorSet);

            // Skip readonly buffers
            if descriptor_set_index != 1 {
                continue;
            }

            let binding_index = compiler.get_decoration(r.id, DecorationBinding);

            // assign binding index after we have collected all resources
            buffer_bindings.push(binding_2(
                execution_model,
                descriptor_set_index,
                binding_index,
            ));
        }

        // Calculate binding offsets

        let readonly_texture_count =
            texture_bindings.iter().filter(|b| b.desc_set == 0).count() as u32;
        let readwrite_texture_count =
            texture_bindings.iter().filter(|b| b.desc_set == 1).count() as u32;

        let uniform_buffer_count =
            buffer_bindings.iter().filter(|b| b.desc_set == 2).count() as u32;
        let readonly_buffer_count =
            buffer_bindings.iter().filter(|b| b.desc_set == 0).count() as u32;

        // Calculate resource indices

        for b in texture_bindings.iter_mut() {
            if b.desc_set == 0 {
                // readonly textures
                b.msl_texture = b.binding;
                b.msl_sampler = b.binding;
            } else {
                // readwrite textures
                b.msl_texture = readonly_texture_count.wrapping_add(b.binding);
                b.msl_sampler = readonly_texture_count.wrapping_add(b.binding);
            }
            if compiler.msl_add_resource_binding_2(&mut context, b) < 0 {
                return Err(spvc_error(
                    "spvc_compiler_msl_add_resource_binding_2",
                    &context,
                ));
            }
        }

        for b in buffer_bindings.iter_mut() {
            if b.desc_set == 0 {
                // Subtract by the readonly texture count because they precede readonly buffers in the descriptor set
                b.msl_buffer = uniform_buffer_count
                    .wrapping_add(b.binding.wrapping_sub(readonly_texture_count));
            } else if b.desc_set == 1 {
                // Subtract by the readwrite texture count because they precede readwrite buffers in the descriptor set
                b.msl_buffer = uniform_buffer_count
                    .wrapping_add(readonly_buffer_count)
                    .wrapping_add(b.binding.wrapping_sub(readwrite_texture_count));
            } else {
                // Uniform buffers are alone in the descriptor set
                b.msl_buffer = b.binding;
            }

            if compiler.msl_add_resource_binding_2(&mut context, b) < 0 {
                return Err(spvc_error(
                    "spvc_compiler_msl_add_resource_binding_2",
                    &context,
                ));
            }
        }
    }

    if compiler.install_compiler_options(&options) < 0 {
        return Err(spvc_error(
            "spvc_compiler_install_compiler_options",
            &context,
        ));
    }

    /* Compile to the target shader language */
    let translated_source = compiler
        .compile(&mut context)
        .map_err(|_| spvc_error("spvc_compiler_compile", &context))?;

    let cleansed_entrypoint = if backend == SpvcBackend::Msl {
        // Metal doesn't allow a "main" entrypoint, so determine the "cleansed" entrypoint name (e.g. main -> main0 on MSL)
        let model = compiler.get_execution_model();
        compiler.get_cleansed_entry_point_name(&mut context, entrypoint, model)
    } else {
        Some(entrypoint.to_string())
    };

    Ok(TranspileContext {
        translated_source,
        cleansed_entrypoint,
    })
}

fn get_io_vars(
    context: &mut SpvcContext,
    compiler: &SpvcCompiler,
    reflected_resources: &[SpvcReflectedResource],
) -> Result<Vec<IOVarMetadata>> {
    let mut vars = Vec::new();
    if vars.try_reserve_exact(reflected_resources.len()).is_err() {
        return Err(Error::out_of_memory());
    }
    for resource in reflected_resources {
        // FIXME (upstream): a NULL type handle is dereferenced.
        let Some(type_) = compiler.get_type_handle(context, resource.base_type_id) else {
            return Err(spvc_error("spvc_compiler_get_type_handle", context));
        };

        let vector_type = match spvc_type_get_basetype(type_) {
            SPVC_BASETYPE_INT8 => IOVarType::Int8,
            SPVC_BASETYPE_UINT8 => IOVarType::UInt8,
            SPVC_BASETYPE_INT16 => IOVarType::Int16,
            SPVC_BASETYPE_UINT16 => IOVarType::UInt16,
            SPVC_BASETYPE_INT32 => IOVarType::Int32,
            SPVC_BASETYPE_UINT32 => IOVarType::UInt32,
            SPVC_BASETYPE_INT64 => IOVarType::Int64,
            SPVC_BASETYPE_UINT64 => IOVarType::UInt64,
            SPVC_BASETYPE_FP16 => IOVarType::Float16,
            SPVC_BASETYPE_FP32 => IOVarType::Float32,
            SPVC_BASETYPE_FP64 => IOVarType::Float64,
            _ => IOVarType::Unknown,
        };

        let vector_size = spvc_type_get_vector_size(type_);

        vars.push(IOVarMetadata {
            name: resource.name.clone(),
            location: compiler.get_decoration(resource.id, DecorationLocation),
            vector_type,
            vector_size,
        });
    }
    Ok(vars)
}

fn reflection_compiler(code: &[u8]) -> Result<(SpvcContext, SpvcCompiler, SpvcResources)> {
    /* Create the SPIRV-Cross context */
    let mut context = SpvcContext::create();

    /* Parse the SPIR-V into IR */
    let ir = context
        .parse_spirv(&spirv_words(code)?)
        .map_err(|_| spvc_error("spvc_context_parse_spirv", &context))?;

    /* Create a reflection-only compiler */
    let mut compiler = context
        .create_compiler(SpvcBackend::None, ir)
        .map_err(|_| spvc_error("spvc_context_create_compiler", &context))?;

    let active_variables = compiler
        .get_active_interface_variables(&mut context)
        .map_err(|_| spvc_error("spvc_compiler_get_active_interface_variables", &context))?;

    let resources = compiler
        .create_shader_resources_for_active_variables(&mut context, &active_variables)
        .map_err(|_| {
            spvc_error(
                "spvc_compiler_create_shader_resources_for_active_variables",
                &context,
            )
        })?;

    Ok((context, compiler, resources))
}

/// Get reflection metadata from a graphics shader's SPIR-V. Translation of
/// `SDL_ShaderCross_ReflectGraphicsSPIRV()`.
// Acquire metadata from SPIRV bytecode.
// TODO: validate descriptor sets
pub fn reflect_graphics_spirv(
    code: &[u8],
    props: Option<&Properties>,
) -> Result<GraphicsShaderMetadata> {
    let _ = props;
    let (mut context, compiler, resources) = reflection_compiler(code)?;
    let mut num_separate_samplers = 0; // HLSL edge case

    // Combined texture-samplers
    let mut num_texture_samplers =
        resource_list(&mut context, &resources, SPVC_RESOURCE_TYPE_SAMPLED_IMAGE)?.len();

    // If source is HLSL, we might have separate images and samplers
    if num_texture_samplers == 0 {
        num_separate_samplers = resource_list(
            &mut context,
            &resources,
            SPVC_RESOURCE_TYPE_SEPARATE_SAMPLERS,
        )?
        .len();
        num_texture_samplers = num_separate_samplers;
    }

    // Storage textures
    let mut num_storage_textures =
        resource_list(&mut context, &resources, SPVC_RESOURCE_TYPE_STORAGE_IMAGE)?.len();

    // If source is HLSL, storage images might be marked as separate images
    let num_separate_images =
        resource_list(&mut context, &resources, SPVC_RESOURCE_TYPE_SEPARATE_IMAGE)?.len(); // HLSL edge case
                                                                                           // The number of storage textures is the number of separate images minus the number of samplers.
    num_storage_textures =
        num_storage_textures.wrapping_add(num_separate_images.wrapping_sub(num_separate_samplers));

    // Storage buffers
    let num_storage_buffers =
        resource_list(&mut context, &resources, SPVC_RESOURCE_TYPE_STORAGE_BUFFER)?.len();

    // Uniform buffers
    let num_uniform_buffers =
        resource_list(&mut context, &resources, SPVC_RESOURCE_TYPE_UNIFORM_BUFFER)?.len();

    // Inputs
    let reflected_resources =
        resource_list(&mut context, &resources, SPVC_RESOURCE_TYPE_STAGE_INPUT)?;
    let inputs = get_io_vars(&mut context, &compiler, reflected_resources)?;

    // Outputs
    let reflected_resources =
        resource_list(&mut context, &resources, SPVC_RESOURCE_TYPE_STAGE_OUTPUT)?;
    let outputs = get_io_vars(&mut context, &compiler, reflected_resources)?;

    Ok(GraphicsShaderMetadata {
        resource_info: GraphicsShaderResourceInfo {
            num_samplers: num_texture_samplers as u32,
            num_storage_textures: num_storage_textures as u32,
            num_storage_buffers: num_storage_buffers as u32,
            num_uniform_buffers: num_uniform_buffers as u32,
        },
        inputs,
        outputs,
    })
}

/// Get reflection metadata from a compute shader's SPIR-V. Translation of
/// `SDL_ShaderCross_ReflectComputeSPIRV()`.
pub fn reflect_compute_spirv(
    bytecode: &[u8],
    props: Option<&Properties>,
) -> Result<ComputePipelineMetadata> {
    let _ = props;
    let (mut context, compiler, resources) = reflection_compiler(bytecode)?;
    let mut num_readonly_storage_textures: u32 = 0;
    let mut num_readonly_storage_buffers: u32 = 0;
    let mut num_readwrite_storage_textures: u32 = 0;
    let mut num_readwrite_storage_buffers: u32 = 0;
    let mut num_separate_samplers = 0; // HLSL edge case

    // Combined texture-samplers
    let mut num_texture_samplers =
        resource_list(&mut context, &resources, SPVC_RESOURCE_TYPE_SAMPLED_IMAGE)?.len();

    // If source is HLSL, we might have separate images and samplers
    if num_texture_samplers == 0 {
        num_separate_samplers = resource_list(
            &mut context,
            &resources,
            SPVC_RESOURCE_TYPE_SEPARATE_SAMPLERS,
        )?
        .len();
        num_texture_samplers = num_separate_samplers;
    }

    // Storage textures
    let reflected_resources =
        resource_list(&mut context, &resources, SPVC_RESOURCE_TYPE_STORAGE_IMAGE)?;

    for r in reflected_resources {
        check_set_and_binding(&compiler, r.id)?;

        let descriptor_set_index = compiler.get_decoration(r.id, DecorationDescriptorSet);
        if descriptor_set_index == 0 {
            num_readonly_storage_textures += 1;
        } else if descriptor_set_index == 1 {
            num_readwrite_storage_textures += 1;
        } else {
            return Err(Error::new(
                "Descriptor set index for compute storage texture must be 0 or 1!",
            ));
        }
    }

    // If source is HLSL, readonly storage images might be marked as separate images
    let reflected_resources =
        resource_list(&mut context, &resources, SPVC_RESOURCE_TYPE_SEPARATE_IMAGE)?;

    // (The number of storage textures, separate images minus samplers added
    // to it here, isn't used.)
    for r in reflected_resources.iter().skip(num_separate_samplers) {
        check_set_and_binding(&compiler, r.id)?;

        let descriptor_set_index = compiler.get_decoration(r.id, DecorationDescriptorSet);
        if descriptor_set_index == 0 {
            num_readonly_storage_textures += 1;
        } else if descriptor_set_index == 1 {
            num_readwrite_storage_textures += 1;
        } else {
            return Err(Error::new(
                "Descriptor set index for compute storage texture must be 0 or 1!",
            ));
        }
    }

    // Storage buffers
    let reflected_resources =
        resource_list(&mut context, &resources, SPVC_RESOURCE_TYPE_STORAGE_BUFFER)?;

    // Readonly storage buffers
    for r in reflected_resources {
        check_set_and_binding(&compiler, r.id)?;

        let descriptor_set_index = compiler.get_decoration(r.id, DecorationDescriptorSet);
        if !(descriptor_set_index == 0 || descriptor_set_index == 1) {
            return Err(Error::new(
                "Descriptor set index for compute storage buffer must be 0 or 1!",
            ));
        }

        if descriptor_set_index == 0 {
            num_readonly_storage_buffers += 1;
        } else {
            num_readwrite_storage_buffers += 1;
        }
    }

    // Uniform buffers
    let num_uniform_buffers =
        resource_list(&mut context, &resources, SPVC_RESOURCE_TYPE_UNIFORM_BUFFER)?.len();

    // Threadcount
    Ok(ComputePipelineMetadata {
        threadcount_x: compiler.get_execution_mode_argument_by_index(ExecutionModeLocalSize, 0),
        threadcount_y: compiler.get_execution_mode_argument_by_index(ExecutionModeLocalSize, 1),
        threadcount_z: compiler.get_execution_mode_argument_by_index(ExecutionModeLocalSize, 2),
        num_samplers: num_texture_samplers as u32,
        num_readonly_storage_textures,
        num_readonly_storage_buffers,
        num_readwrite_storage_textures,
        num_readwrite_storage_buffers,
        num_uniform_buffers: num_uniform_buffers as u32,
    })
}

/// A shader or compute pipeline made by
/// `SDL_ShaderCross_INTERNAL_CompileFromSPIRV()` and
/// `SDL_ShaderCross_INTERNAL_CreateShaderFromSPIRV()`.
enum ShaderObject {
    Shader(Shader),
    ComputePipeline(ComputePipeline),
}

/// The metadata `SDL_ShaderCross_INTERNAL_CreateShaderFromSPIRV()` takes.
#[derive(Clone, Copy)]
enum Metadata<'a> {
    Graphics(&'a GraphicsShaderResourceInfo),
    Compute(&'a ComputePipelineMetadata),
}

fn debug_name_props(info: &SpirvInfo<'_>, prop_name: &str) -> Result<Option<Properties>> {
    let debug_name = get_string_property(info.props, PROP_SHADER_DEBUG_NAME_STRING);
    match debug_name {
        Some(debug_name) => {
            let props = Properties::new();
            props.set(prop_name, debug_name)?;
            Ok(Some(props))
        }
        None => Ok(None),
    }
}

/// The shader code `SDL_ShaderCross_INTERNAL_CompileFromSPIRV()` hands to
/// SDL: the DXBC or DXIL compiled from the HLSL, or the MSL source with its
/// terminating NUL.
fn compiled_code(target_format: ShaderFormat, hlsl_info: &HlslInfo<'_>) -> Result<Vec<u8>> {
    if target_format == ShaderFormat::DXBC {
        compile_dxbc_from_hlsl_internal(hlsl_info, false)
    } else if target_format == ShaderFormat::DXIL {
        compile_dxil_from_hlsl(hlsl_info)
    } else {
        // MSL
        let mut code = Vec::new();
        if code.try_reserve_exact(hlsl_info.source.len() + 1).is_err() {
            return Err(Error::out_of_memory());
        }
        code.extend_from_slice(hlsl_info.source.as_bytes());
        code.push(0);
        Ok(code)
    }
}

fn compile_from_spirv(
    device: &Device,
    info: &SpirvInfo<'_>,
    target_format: ShaderFormat,
    metadata_props: Option<&Properties>,
) -> Result<ShaderObject> {
    let backend;
    let mut shadermodel = 0;
    if target_format == ShaderFormat::DXBC {
        backend = SpvcBackend::Hlsl;
        shadermodel = 51;
    } else if target_format == ShaderFormat::DXIL {
        backend = SpvcBackend::Hlsl;
        shadermodel = 60;
    } else if target_format == ShaderFormat::MSL {
        backend = SpvcBackend::Msl;
    } else {
        return Err(Error::new(
            "SDL_ShaderCross_INTERNAL_CompileFromSPIRV: Unexpected SDL_GPUBackend",
        ));
    }

    let transpile_context = transpile_from_spirv(
        backend,
        shadermodel,
        info.shader_stage,
        info.bytecode,
        info.entrypoint,
        info.props,
    )?;

    // (A NULL cleansed entry point makes SDL_CreateGPUShader() fail.)
    let Some(cleansed_entrypoint) = transpile_context.cleansed_entrypoint.as_deref() else {
        return Err(Error::invalid_param("entrypoint"));
    };

    let hlsl_info = HlslInfo {
        source: &transpile_context.translated_source,
        entrypoint: cleansed_entrypoint,
        include_dir: None,
        defines: None,
        shader_stage: info.shader_stage,
        props: info.props,
    };

    if info.shader_stage == ShaderStage::Compute {
        // FIXME (upstream): a reflection failure here is dereferenced as NULL.
        let pipeline_info = reflect_compute_spirv(info.bytecode, metadata_props)?;
        let code = compiled_code(target_format, &hlsl_info)?;

        let create_info = ComputePipelineCreateInfo {
            code: &code,
            entrypoint: cleansed_entrypoint,
            format: target_format,
            num_samplers: pipeline_info.num_samplers,
            num_readonly_storage_textures: pipeline_info.num_readonly_storage_textures,
            num_readonly_storage_buffers: pipeline_info.num_readonly_storage_buffers,
            num_readwrite_storage_textures: pipeline_info.num_readwrite_storage_textures,
            num_readwrite_storage_buffers: pipeline_info.num_readwrite_storage_buffers,
            num_uniform_buffers: pipeline_info.num_uniform_buffers,
            threadcount_x: pipeline_info.threadcount_x,
            threadcount_y: pipeline_info.threadcount_y,
            threadcount_z: pipeline_info.threadcount_z,
            props: debug_name_props(info, PROP_GPU_COMPUTEPIPELINE_CREATE_NAME_STRING)?,
        };

        Ok(ShaderObject::ComputePipeline(
            device.create_compute_pipeline(&create_info)?,
        ))
    } else {
        let shader_info = reflect_graphics_spirv(info.bytecode, metadata_props)?;
        let code = compiled_code(target_format, &hlsl_info)?;

        let create_info = ShaderCreateInfo {
            code: &code,
            entrypoint: cleansed_entrypoint,
            format: target_format,
            stage: gpu_shader_stage(info.shader_stage),
            num_samplers: shader_info.resource_info.num_samplers,
            num_storage_textures: shader_info.resource_info.num_storage_textures,
            num_storage_buffers: shader_info.resource_info.num_storage_buffers,
            num_uniform_buffers: shader_info.resource_info.num_uniform_buffers,
            props: debug_name_props(info, PROP_GPU_SHADER_CREATE_NAME_STRING)?,
        };

        Ok(ShaderObject::Shader(device.create_shader(&create_info)?))
    }
}

/// `(SDL_GPUShaderStage)info->shader_stage` (only called for the vertex
/// and fragment stages, which share their values).
fn gpu_shader_stage(stage: ShaderStage) -> sdl3::gpu::ShaderStage {
    match stage {
        ShaderStage::Fragment => sdl3::gpu::ShaderStage::Fragment,
        _ => sdl3::gpu::ShaderStage::Vertex,
    }
}

/// Transpile to MSL code from SPIR-V code. Translation of
/// `SDL_ShaderCross_TranspileMSLFromSPIRV()`.
///
/// You must remember to rename the entry point when using the result with
/// Metal: "main" becomes "main0", for example (see the `shadercross`
/// tool).
pub fn transpile_msl_from_spirv(info: &SpirvInfo<'_>) -> Result<String> {
    let context = transpile_from_spirv(
        SpvcBackend::Msl,
        0,
        info.shader_stage,
        info.bytecode,
        info.entrypoint,
        info.props,
    )?;

    Ok(context.translated_source)
}

/// Transpile to HLSL code from SPIR-V code. Translation of
/// `SDL_ShaderCross_TranspileHLSLFromSPIRV()`.
pub fn transpile_hlsl_from_spirv(info: &SpirvInfo<'_>) -> Result<String> {
    let context = transpile_from_spirv(
        SpvcBackend::Hlsl,
        if get_boolean_property(info.props, PROP_SPIRV_PSSL_COMPATIBILITY_BOOLEAN, false) {
            50
        } else {
            60
        },
        info.shader_stage,
        info.bytecode,
        info.entrypoint,
        info.props,
    )?;

    Ok(context.translated_source)
}

/// Compile DXBC bytecode from SPIR-V code. Translation of
/// `SDL_ShaderCross_CompileDXBCFromSPIRV()`.
pub fn compile_dxbc_from_spirv(info: &SpirvInfo<'_>) -> Result<Vec<u8>> {
    let context = transpile_from_spirv(
        SpvcBackend::Hlsl,
        51,
        info.shader_stage,
        info.bytecode,
        info.entrypoint,
        info.props,
    )?;

    let hlsl_info = HlslInfo {
        source: &context.translated_source,
        entrypoint: context.cleansed_entrypoint.as_deref().unwrap_or(""),
        include_dir: None,
        defines: None,
        shader_stage: info.shader_stage,
        props: info.props,
    };

    compile_dxbc_from_hlsl_internal(&hlsl_info, false)
}

/// Compile DXIL bytecode from SPIR-V code. Translation of
/// `SDL_ShaderCross_CompileDXILFromSPIRV()`.
///
/// This build has no DXC, so this fails like upstream's build without it.
pub fn compile_dxil_from_spirv(info: &SpirvInfo<'_>) -> Result<Vec<u8>> {
    // (SDL_SHADERCROSS_DXC isn't defined in this build.)
    let _ = info;
    Err(Error::new(
        "Shadercross was not compiled with DXC support, cannot compile to SPIR-V!",
    ))
}

fn create_shader_from_spirv(
    device: &Device,
    info: &SpirvInfo<'_>,
    metadata: Metadata<'_>,
    metadata_props: Option<&Properties>,
) -> Result<ShaderObject> {
    let format;
    let shader_formats = device.shader_formats();
    if shader_formats.contains(ShaderFormat::SPIRV) {
        match metadata {
            Metadata::Compute(pipeline_metadata) if info.shader_stage == ShaderStage::Compute => {
                let create_info = ComputePipelineCreateInfo {
                    code: info.bytecode,
                    entrypoint: info.entrypoint,
                    format: ShaderFormat::SPIRV,
                    num_samplers: pipeline_metadata.num_samplers,
                    num_readonly_storage_textures: pipeline_metadata.num_readonly_storage_textures,
                    num_readonly_storage_buffers: pipeline_metadata.num_readonly_storage_buffers,
                    num_readwrite_storage_textures: pipeline_metadata
                        .num_readwrite_storage_textures,
                    num_readwrite_storage_buffers: pipeline_metadata.num_readwrite_storage_buffers,
                    num_uniform_buffers: pipeline_metadata.num_uniform_buffers,
                    threadcount_x: pipeline_metadata.threadcount_x,
                    threadcount_y: pipeline_metadata.threadcount_y,
                    threadcount_z: pipeline_metadata.threadcount_z,
                    props: debug_name_props(info, PROP_GPU_COMPUTEPIPELINE_CREATE_NAME_STRING)?,
                };
                return Ok(ShaderObject::ComputePipeline(
                    device.create_compute_pipeline(&create_info)?,
                ));
            }
            Metadata::Graphics(resource_info) if info.shader_stage != ShaderStage::Compute => {
                let create_info = ShaderCreateInfo {
                    code: info.bytecode,
                    entrypoint: info.entrypoint,
                    format: ShaderFormat::SPIRV,
                    stage: gpu_shader_stage(info.shader_stage),
                    num_samplers: resource_info.num_samplers,
                    num_storage_textures: resource_info.num_storage_textures,
                    num_storage_buffers: resource_info.num_storage_buffers,
                    num_uniform_buffers: resource_info.num_uniform_buffers,
                    props: debug_name_props(info, PROP_GPU_SHADER_CREATE_NAME_STRING)?,
                };
                return Ok(ShaderObject::Shader(device.create_shader(&create_info)?));
            }
            // FIXME (upstream): the metadata pointer is reinterpreted as the
            // other struct when the shader stage doesn't match the function.
            _ => return Err(Error::invalid_param("metadata")),
        }
    } else if shader_formats.contains(ShaderFormat::MSL) {
        format = ShaderFormat::MSL;
    } else if shader_formats.contains(ShaderFormat::DXBC) && d3dcompiler_loaded() {
        format = ShaderFormat::DXBC;
    } else {
        // (DXIL needs DXC, which this build doesn't have.)
        return Err(Error::new(
            "SDL_ShaderCross_INTERNAL_CreateShaderFromSPIRV: Unexpected SDL_GPUBackend",
        ));
    }

    compile_from_spirv(device, info, format, metadata_props)
}

/// Compile an SDL GPU shader from SPIR-V code, in whichever format the
/// device takes. Translation of
/// `SDL_ShaderCross_CompileGraphicsShaderFromSPIRV()`.
pub fn compile_graphics_shader_from_spirv(
    device: &Device,
    info: &SpirvInfo<'_>,
    resource_info: &GraphicsShaderResourceInfo,
    props: Option<&Properties>,
) -> Result<Shader> {
    match create_shader_from_spirv(device, info, Metadata::Graphics(resource_info), props)? {
        ShaderObject::Shader(shader) => Ok(shader),
        ShaderObject::ComputePipeline(_) => Err(Error::invalid_param("info")),
    }
}

/// Compile an SDL GPU compute pipeline from SPIR-V code, in whichever
/// format the device takes. Translation of
/// `SDL_ShaderCross_CompileComputePipelineFromSPIRV()`.
pub fn compile_compute_pipeline_from_spirv(
    device: &Device,
    info: &SpirvInfo<'_>,
    metadata: &ComputePipelineMetadata,
    props: Option<&Properties>,
) -> Result<ComputePipeline> {
    match create_shader_from_spirv(device, info, Metadata::Compute(metadata), props)? {
        ShaderObject::ComputePipeline(pipeline) => Ok(pipeline),
        ShaderObject::Shader(_) => Err(Error::invalid_param("info")),
    }
}

/// Initializes SDL_shadercross: loads the D3D shader compiler where there
/// is one. Translation of `SDL_ShaderCross_Init()`.
pub fn init() -> Result<()> {
    let mut guard = D3DCOMPILER.lock().unwrap_or_else(|e| e.into_inner());
    *guard = d3dcompiler::D3DCompiler::load(D3DCOMPILER_DLL);
    Ok(())
}

/// De-initializes SDL_shadercross. Translation of `SDL_ShaderCross_Quit()`.
pub fn quit() {
    let mut guard = D3DCOMPILER.lock().unwrap_or_else(|e| e.into_inner());
    *guard = None;
}

/// The formats that can be output from SPIR-V. Translation of
/// `SDL_ShaderCross_GetSPIRVShaderFormats()`.
pub fn get_spirv_shader_formats() -> ShaderFormat {
    /* SPIRV and MSL can always be output as-is with no preprocessing since we require SPIRV-Cross */
    let mut supported_formats = ShaderFormat::SPIRV | ShaderFormat::MSL;

    /* SPIRV-Cross + DXC allows us to cross-compile to HLSL, then compile to DXIL */
    // (SDL_SHADERCROSS_DXC isn't defined in this build.)

    /* SPIRV-Cross + FXC allows us to cross-compile to HLSL, then compile to DXBC */
    if d3dcompiler_loaded() {
        supported_formats |= ShaderFormat::DXBC;
    }

    supported_formats
}

/// The formats that can be output from HLSL. Translation of
/// `SDL_ShaderCross_GetHLSLShaderFormats()`.
pub fn get_hlsl_shader_formats() -> ShaderFormat {
    let mut supported_formats = ShaderFormat::INVALID;

    /* DXC allows compilation from HLSL to SPIRV */
    // (SDL_SHADERCROSS_DXC isn't defined in this build.)

    /* FXC allows compilation of HLSL to DXBC */
    if d3dcompiler_loaded() {
        supported_formats |= ShaderFormat::DXBC;
    }

    supported_formats
}

#[cfg(test)]
pub(crate) fn transpile_for_tests(
    backend: SpvcBackend,
    shadermodel: u32,
    shader_stage: ShaderStage,
    code: &[u8],
    entrypoint: &str,
    props: Option<&Properties>,
) -> Result<(String, Option<String>)> {
    transpile_from_spirv(backend, shadermodel, shader_stage, code, entrypoint, props)
        .map(|c| (c.translated_source, c.cleansed_entrypoint))
}

#[cfg(test)]
pub(crate) fn parse_version_number_for_tests(s: &str) -> i32 {
    parse_version_number(s)
}
