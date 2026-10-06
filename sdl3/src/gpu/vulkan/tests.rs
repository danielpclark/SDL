// Tests of the Vulkan GPU backend.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! The tables are checked against upstream's values, and the allocator's
//! bookkeeping against what upstream's lists hold after the same calls.
//! The device tests make a device on the offscreen video driver (Mesa's
//! lavapipe on Linux CI) through the front end and directly, create and
//! release every kind of resource, follow the allocator's regions and
//! defragment an allocation. Then they record and submit command buffers
//! and read the results back exactly: clears, uploads, copies, draws with
//! vertex buffers, uniforms and samplers (indexed and indirect too),
//! compute dispatches, blits and mipmaps, fences; and present frames to
//! swapchains of offscreen windows (and X11 ones when there is a display).
//! Without a Vulkan loader and driver they report a skip (capability
//! `vulkan`) and pass. In debug mode the device enables
//! `VK_LAYER_KHRONOS_validation` when it's installed.

use std::ptr::null;
use std::sync::atomic::Ordering;
use std::sync::Arc;

use super::binding::find_best_memory_types;
use super::device::{driver_version_string, VulkanExtensions};
use super::memory::{
    next_highest_alignment, MemoryAllocator, LARGE_ALLOCATION_INCREMENT, SMALL_ALLOCATION_SIZE,
};
use super::pipelines::is_valid_shader_bytecode;
use super::resources::{
    default_buffer_usage_mode, default_texture_usage_mode, BufferContainer, VulkanBuffer,
    VulkanBufferType, VulkanCommandBuffer, VulkanTextureUsageMode,
    VULKAN_BUFFER_USAGE_MODE_COMPUTE_STORAGE_READ_WRITE, VULKAN_BUFFER_USAGE_MODE_INDEX_READ,
    VULKAN_BUFFER_USAGE_MODE_VERTEX_READ,
};
use super::tables::*;
use super::test_spirv;
use super::*;
use crate::gpu::{
    BlendFactor, BlendOp, BufferCreateInfo, ColorTargetDescription, CompareOp, CullMode, Device,
    Filter, GraphicsPipelineTargetInfo, MultisampleState, PrimitiveType, RasterizerState,
    SamplerAddressMode, SamplerMipmapMode, ShaderStage, StencilOp, TransferBuffer,
    TransferBufferCreateInfo, VertexAttribute, VertexBufferDescription, VertexElementFormat,
    VertexInputRate, VertexInputState, VulkanFeatureStructure, VulkanOptions,
    PROP_GPU_DEVICE_CREATE_DEBUGMODE_BOOLEAN, PROP_GPU_DEVICE_CREATE_SHADERS_SPIRV_BOOLEAN,
    PROP_GPU_DEVICE_CREATE_VULKAN_OPTIONS_POINTER, PROP_GPU_DEVICE_DRIVER_VERSION_STRING,
    PROP_GPU_DEVICE_NAME_STRING, PROP_GPU_TEXTURE_CREATE_NAME_STRING,
};
use crate::hints;
use crate::init::{self, InitFlags};

// ---------------------------------------------------------------------------
// Without a device
// ---------------------------------------------------------------------------

#[test]
fn texture_format_table() {
    assert_eq!(SDL_TO_VK_TEXTURE_FORMAT.len(), 105);
    let f = sdl_to_vk_texture_format;
    // (the values of vulkan_core.h)
    assert_eq!(f(TextureFormat::INVALID), 0);
    assert_eq!(f(TextureFormat::A8_UNORM), 9); // R8_UNORM, swizzled
    assert_eq!(f(TextureFormat::R8G8B8A8_UNORM), 37);
    assert_eq!(f(TextureFormat::R10G10B10A2_UNORM), 64); // A2B10G10R10_UNORM_PACK32
    assert_eq!(f(TextureFormat::B5G6R5_UNORM), 4); // R5G6B5_UNORM_PACK16
    assert_eq!(f(TextureFormat::B5G5R5A1_UNORM), 8); // A1R5G5B5_UNORM_PACK16
    assert_eq!(f(TextureFormat::B4G4R4A4_UNORM), 3);
    assert_eq!(f(TextureFormat::BC6H_RGB_FLOAT), 144);
    assert_eq!(f(TextureFormat::R11G11B10_UFLOAT), 122);
    assert_eq!(f(TextureFormat::R8_INT), 14);
    assert_eq!(f(TextureFormat::BC2_RGBA_UNORM_SRGB), 136);
    assert_eq!(f(TextureFormat::D24_UNORM), 125); // X8_D24_UNORM_PACK32
    assert_eq!(f(TextureFormat::D32_FLOAT_S8_UINT), 130);
    assert_eq!(f(TextureFormat::ASTC_4x4_UNORM), 157);
    assert_eq!(f(TextureFormat::ASTC_12x12_UNORM_SRGB), 184);
    assert_eq!(f(TextureFormat::ASTC_4x4_FLOAT), 1000066000);
    assert_eq!(f(TextureFormat::ASTC_12x12_FLOAT), 1000066013);
    // (out of range: no format)
    assert_eq!(f(TextureFormat(105)), VK_FORMAT_UNDEFINED);
    assert_eq!(f(TextureFormat(u32::MAX)), VK_FORMAT_UNDEFINED);

    // Every format past INVALID has a Vulkan format.
    for i in 1..105 {
        assert_ne!(f(TextureFormat(i)), VK_FORMAT_UNDEFINED, "{i}");
    }
}

#[test]
fn swizzles() {
    let identity = IDENTITY_SWIZZLE;
    assert_eq!(
        swizzle_for_sdl_format(TextureFormat::R8G8B8A8_UNORM),
        identity
    );
    let a8 = swizzle_for_sdl_format(TextureFormat::A8_UNORM);
    assert_eq!((a8.r, a8.g, a8.b, a8.a), (1, 1, 1, 3)); // ZERO, ZERO, ZERO, R
    let bgra4 = swizzle_for_sdl_format(TextureFormat::B4G4R4A4_UNORM);
    assert_eq!((bgra4.r, bgra4.g, bgra4.b, bgra4.a), (4, 3, 6, 5)); // G, R, A, B
    let hdr10 = SWAPCHAIN_COMPOSITION_SWIZZLE[3];
    assert_eq!((hdr10.r, hdr10.g, hdr10.b, hdr10.a), (3, 4, 5, 6));
    assert_eq!(SWAPCHAIN_COMPOSITION_SWIZZLE[0], identity);
}

#[test]
fn enum_tables() {
    assert_eq!(SDL_TO_VK_VERTEX_FORMAT.len(), 31);
    assert_eq!(sdl_to_vk_vertex_format(VertexElementFormat::Int), 99); // R32_SINT
    assert_eq!(sdl_to_vk_vertex_format(VertexElementFormat::Float3), 106);
    assert_eq!(sdl_to_vk_vertex_format(VertexElementFormat::Ubyte4Norm), 37);
    assert_eq!(sdl_to_vk_vertex_format(VertexElementFormat::Half4), 97);

    assert_eq!(sdl_to_vk_blend_factor(BlendFactor::Zero), 0);
    assert_eq!(sdl_to_vk_blend_factor(BlendFactor::ConstantColor), 10);
    assert_eq!(sdl_to_vk_blend_factor(BlendFactor::SrcAlphaSaturate), 14);
    assert_eq!(SDL_TO_VK_BLEND_FACTOR[0], 0); // INVALID
    assert_eq!(sdl_to_vk_blend_op(BlendOp::Add), 0);
    assert_eq!(sdl_to_vk_blend_op(BlendOp::Max), 4);
    assert_eq!(sdl_to_vk_compare_op(CompareOp::Never), 0);
    assert_eq!(sdl_to_vk_compare_op(CompareOp::Always), 7);
    assert_eq!(sdl_to_vk_stencil_op(StencilOp::Keep), 0);
    assert_eq!(sdl_to_vk_stencil_op(StencilOp::DecrementAndWrap), 7);

    assert_eq!(sdl_to_vk_primitive_type(PrimitiveType::TriangleList), 3);
    assert_eq!(sdl_to_vk_primitive_type(PrimitiveType::TriangleStrip), 4);
    assert_eq!(sdl_to_vk_primitive_type(PrimitiveType::PointList), 0);
    assert_eq!(sdl_to_vk_cull_mode(CullMode::Back), 2);
    assert_eq!(sdl_to_vk_front_face(crate::gpu::FrontFace::Clockwise), 1);
    assert_eq!(sdl_to_vk_load_op(crate::gpu::LoadOp::DontCare), 2);
    assert_eq!(sdl_to_vk_store_op(crate::gpu::StoreOp::Resolve), 1); // DONT_CARE
    assert_eq!(sdl_to_vk_store_op(crate::gpu::StoreOp::ResolveAndStore), 0); // STORE
    assert_eq!(sdl_to_vk_sample_count(SampleCount::Eight), 8);
    assert_eq!(sdl_to_vk_vertex_input_rate(VertexInputRate::Instance), 1);
    assert_eq!(sdl_to_vk_filter(Filter::Linear), 1);
    assert_eq!(sdl_to_vk_sampler_mipmap_mode(SamplerMipmapMode::Linear), 1);
    assert_eq!(
        sdl_to_vk_sampler_address_mode(SamplerAddressMode::ClampToEdge),
        2
    );
    assert_eq!(
        sdl_to_vk_index_type(crate::gpu::IndexElementSize::Bits32),
        1
    );
    assert_eq!(sdl_to_vk_present_mode(PresentMode::Vsync), 2); // FIFO
    assert_eq!(sdl_to_vk_present_mode(PresentMode::Mailbox), 1);

    assert_eq!(SWAPCHAIN_COMPOSITION_TO_FORMAT, [44, 50, 97, 64]);
    assert_eq!(SWAPCHAIN_COMPOSITION_TO_FALLBACK_FORMAT, [37, 43, 0, 0]);
    assert_eq!(
        SWAPCHAIN_COMPOSITION_TO_COLOR_SPACE,
        [0, 0, 1000104002, 1000104008]
    );
    assert_eq!(
        swapchain_composition_to_sdl_format(SwapchainComposition::Sdr, true),
        TextureFormat::R8G8B8A8_UNORM
    );
    assert_eq!(
        swapchain_composition_to_sdl_format(SwapchainComposition::SdrLinear, false),
        TextureFormat::B8G8R8A8_UNORM_SRGB
    );
    assert_eq!(
        swapchain_composition_to_sdl_format(SwapchainComposition::Hdr10St2084, false),
        TextureFormat::R10G10B10A2_UNORM
    );
}

#[test]
fn error_messages() {
    assert_eq!(vk_error_messages(-2), "VK_ERROR_OUT_OF_DEVICE_MEMORY");
    assert_eq!(
        vk_error_messages(-1000069000),
        "VK_ERROR_OUT_OF_POOL_MEMORY"
    );
    assert_eq!(vk_error_messages(1000001003), "VK_SUBOPTIMAL_KHR");
    assert_eq!(vk_error_messages(0), "Unhandled VkResult!");
    assert_eq!(vk_error_messages(-13), "Unhandled VkResult!"); // (not in the list)
    let e = vulkan_error(false, -4, "vkCreateBuffer");
    assert_eq!(e.message(), "vkCreateBuffer VK_ERROR_DEVICE_LOST");
}

#[test]
fn shader_bytecode_checks() {
    let spirv = bytes(test_spirv::VERTEX_NULL);
    assert!(is_valid_shader_bytecode(&spirv));
    // (the other byte order)
    let swapped: Vec<u8> = test_spirv::VERTEX_NULL
        .iter()
        .flat_map(|w| w.swap_bytes().to_ne_bytes())
        .collect();
    assert!(is_valid_shader_bytecode(&swapped));
    assert!(!is_valid_shader_bytecode(&spirv[..3]));
    assert!(!is_valid_shader_bytecode(b"DXBC"));
    assert!(!is_valid_shader_bytecode(&[]));
}

#[test]
fn feature_structures() {
    // (the members of vulkan_core.h's structures)
    assert_eq!(VkPhysicalDeviceFeatures::NAMES.len(), 55);
    assert_eq!(VkPhysicalDeviceFeatures::NAMES[0], "robustBufferAccess");
    assert_eq!(VkPhysicalDeviceFeatures::NAMES[54], "inheritedQueries");
    assert_eq!(VkPhysicalDeviceVulkan11Features::NAMES.len(), 12);
    assert_eq!(VkPhysicalDeviceVulkan12Features::NAMES.len(), 47);
    assert_eq!(
        VkPhysicalDeviceVulkan12Features::NAMES[46],
        "subgroupBroadcastDynamicId"
    );
    assert_eq!(VkPhysicalDeviceVulkan13Features::NAMES.len(), 15);
    assert_eq!(VkPhysicalDeviceVulkan13Features::NAMES[14], "maintenance4");
    let mut f = VkPhysicalDeviceFeatures::default();
    *f.bools_mut()[11] = VK_TRUE;
    assert_eq!(f.depth_clamp, VK_TRUE);
    assert_eq!(f.bools().iter().filter(|&&b| b != 0).count(), 1);
}

#[test]
fn alignment() {
    assert_eq!(next_highest_alignment(0, 256), 0);
    assert_eq!(next_highest_alignment(1, 256), 256);
    assert_eq!(next_highest_alignment(256, 256), 256);
    assert_eq!(next_highest_alignment(257, 256), 512);
    assert_eq!(
        next_highest_alignment(SMALL_ALLOCATION_SIZE + 1, LARGE_ALLOCATION_INCREMENT),
        LARGE_ALLOCATION_INCREMENT
    );
    assert_eq!(memory::next_highest_alignment32(33, 16), 48,);
}

/// Memory properties with the given types' flags (one heap).
fn memory_properties(types: &[VkMemoryPropertyFlags]) -> VkPhysicalDeviceMemoryProperties {
    let mut props = VkPhysicalDeviceMemoryProperties {
        memory_type_count: types.len() as u32,
        memory_heap_count: 1,
        ..Default::default()
    };
    for (t, &flags) in props.memory_types.iter_mut().zip(types) {
        t.property_flags = flags;
    }
    props
}

#[test]
fn memory_type_preferences() {
    const DL: VkMemoryPropertyFlags = VK_MEMORY_PROPERTY_DEVICE_LOCAL_BIT;
    const HV: VkMemoryPropertyFlags = VK_MEMORY_PROPERTY_HOST_VISIBLE_BIT;
    const HC: VkMemoryPropertyFlags = VK_MEMORY_PROPERTY_HOST_COHERENT_BIT;
    const CA: VkMemoryPropertyFlags = VK_MEMORY_PROPERTY_HOST_CACHED_BIT;
    // A discrete GPU: device-local, host-visible, host-cached, BAR.
    let props = memory_properties(&[DL, HV | HC, HV | HC | CA, DL | HV | HC]);

    // GPU buffers prefer device-local memory.
    assert_eq!(find_best_memory_types(&props, 0xF, 0, DL, 0), [0, 3, 1, 2]);
    // Uniform buffers need host-visible coherent memory, device-local if possible.
    assert_eq!(
        find_best_memory_types(&props, 0xF, HV | HC, DL, 0),
        [3, 1, 2]
    );
    // Transfer buffers: host-visible coherent, cached preferred, device-local tolerated.
    assert_eq!(
        find_best_memory_types(&props, 0xF, HV | HC, CA, DL),
        [2, 1, 3]
    );
    // The type filter.
    assert_eq!(find_best_memory_types(&props, 0b0110, 0, DL, 0), [1, 2]);
    // Integrated memory: only device-local host-visible types.
    let uma = memory_properties(&[DL | HV | HC | CA]);
    assert_eq!(find_best_memory_types(&uma, 1, HV | HC, CA, DL), [0]);
    assert_eq!(find_best_memory_types(&uma, 1, 0, DL, 0), [0]);
    assert!(find_best_memory_types(&uma, 0, 0, DL, 0).is_empty());
}

#[test]
fn suballocation_bookkeeping() {
    let mut a = MemoryAllocator::new();
    let small = SMALL_ALLOCATION_SIZE;
    let key = a.add_allocation(2, small, 1, None, true);
    assert_eq!(a.allocation_count(2), 1);
    assert_eq!(a.sorted_free_regions(2), [(key, 0, small)]);

    // Three regions, aligned to 256 (the second one's padding is its own).
    let mut regions = Vec::new();
    for (size, alignment) in [(1000, 256), (100, 256), (5000, 4)] {
        let sel = a.select_region(2, size, alignment).unwrap();
        assert_eq!(sel.allocation, key);
        let used = a.use_region(
            sel.allocation,
            Some(sel.region),
            sel.aligned_offset,
            size,
            size,
            alignment,
            true,
        );
        regions.push(used);
        a.check_indices();
    }
    let offsets: Vec<_> = regions
        .iter()
        .map(|r| (r.offset, r.size, r.resource_offset))
        .collect();
    assert_eq!(
        offsets,
        [(0, 1000, 0), (1000, 24 + 100, 1024), (1124, 5000, 1124)]
    );
    let info = a.allocation_info(key);
    assert_eq!(info.used_space, 1000 + 124 + 5000);
    assert_eq!(info.free_regions, [(6124, small - 6124)]);

    // Large resources don't go into small allocations.
    assert!(a.select_region(2, 3 << 20, 256).is_none());
    // Nor small ones into others.
    let large_key = a.add_allocation(2, LARGE_ALLOCATION_INCREMENT, 2, None, true);
    let sorted = a.sorted_free_regions(2);
    assert_eq!(sorted[0], (large_key, 0, LARGE_ALLOCATION_INCREMENT));
    assert_eq!(a.select_region(2, 64, 4).unwrap().allocation, key);
    assert_eq!(
        a.select_region(2, 3 << 20, 256).unwrap().allocation,
        large_key
    );

    // Freeing the middle region leaves two free regions; freeing the first
    // merges them with it.
    a.remove_memory_used_region(&regions[1]);
    a.check_indices();
    let info = a.allocation_info(key);
    assert_eq!(info.free_regions.len(), 2);
    assert!(info.free_regions.contains(&(1000, 124)));
    assert!(!a.check_empty_allocations);
    a.remove_memory_used_region(&regions[0]);
    a.check_indices();
    let info = a.allocation_info(key);
    assert!(info.free_regions.contains(&(0, 1124)));
    assert_eq!(info.free_regions.len(), 2);

    // The search walks the sorted list from the smallest region: a small
    // request takes the hole at the front, not the tail.
    let sel = a.select_region(2, 512, 256).unwrap();
    assert_eq!((sel.allocation, sel.aligned_offset), (key, 0));

    // A fragmented allocation is marked for defrag and its free regions
    // leave the sorted list.
    a.mark_allocations_for_defrag();
    assert_eq!(a.allocations_to_defrag, [key]);
    let info = a.allocation_info(key);
    assert!(!info.available_for_allocation);
    assert!(a
        .sorted_free_regions(2)
        .iter()
        .all(|&(k, _, _)| k == large_key));
    a.check_indices();
    // (and new free regions of it don't enter the list)
    a.remove_memory_used_region(&regions[2]);
    assert!(a.check_empty_allocations);
    assert_eq!(a.allocation_info(key).free_regions, [(0, small)]);
    assert!(a
        .sorted_free_regions(2)
        .iter()
        .all(|&(k, _, _)| k == large_key));

    // Deallocating cancels the defrag.
    let index = (0..a.allocation_count(2))
        .find(|&i| a.allocation_key(2, i) == key)
        .unwrap();
    assert_eq!(a.deallocate_memory(2, index), 1);
    assert!(a.allocations_to_defrag.is_empty());
    assert_eq!(a.allocation_count(2), 1);
    a.check_indices();

    // The slot of the freed allocation is reused.
    let again = a.add_allocation(2, small, 3, None, true);
    assert_eq!(again, key);
    assert_eq!(a.allocation(again).memory, 3);
}

#[test]
fn free_regions_merge_both_ways() {
    // (eighths of a small allocation, 2 MiB each: small resources)
    const Q: VkDeviceSize = SMALL_ALLOCATION_SIZE / 8;
    let mut a = MemoryAllocator::new();
    let key = a.add_allocation(0, SMALL_ALLOCATION_SIZE, 1, None, true);
    let used: Vec<_> = (0..8)
        .map(|_| {
            let sel = a.select_region(0, Q, 1024).unwrap();
            a.use_region(key, Some(sel.region), sel.aligned_offset, Q, Q, 1024, false)
        })
        .collect();
    assert!(a.allocation_info(key).free_regions.is_empty());
    assert!(a.sorted_free_regions(0).is_empty());
    // 1 and 3, then 2 (merges to its left and its right).
    a.remove_memory_used_region(&used[1]);
    a.remove_memory_used_region(&used[3]);
    a.check_indices();
    assert_eq!(a.sorted_free_regions(0).len(), 2);
    a.remove_memory_used_region(&used[2]);
    a.check_indices();
    assert_eq!(a.sorted_free_regions(0), [(key, Q, 3 * Q)]);
    a.remove_memory_used_region(&used[0]);
    a.check_indices();
    assert_eq!(a.sorted_free_regions(0), [(key, 0, 4 * Q)]);
    for u in &used[4..] {
        a.remove_memory_used_region(u);
    }
    a.check_indices();
    assert_eq!(a.sorted_free_regions(0), [(key, 0, SMALL_ALLOCATION_SIZE)]);
    assert_eq!(a.allocation_info(key).used_space, 0);
}

#[test]
fn usage_modes() {
    use crate::gpu::BufferUsageFlags as B;
    use crate::gpu::TextureUsageFlags as T;
    let used_region = {
        let mut a = MemoryAllocator::new();
        let key = a.add_allocation(0, 64, 1, None, true);
        a.use_region(key, None, 0, 16, 16, 4, true)
    };
    let buffer = |usage| VulkanBuffer {
        container: Default::default(),
        buffer: 0,
        used_region: used_region.clone(),
        ty: VulkanBufferType::Gpu,
        usage,
        size: 16,
        reference_count: Default::default(),
        transitioned: Default::default(),
        marked_for_destroy: Default::default(),
        uniform_buffer_for_defrag: Default::default(),
    };
    assert_eq!(
        default_buffer_usage_mode(&buffer(B::VERTEX | B::INDEX)),
        VULKAN_BUFFER_USAGE_MODE_VERTEX_READ | VULKAN_BUFFER_USAGE_MODE_INDEX_READ
    );
    assert_eq!(
        default_buffer_usage_mode(&buffer(B::COMPUTE_STORAGE_WRITE)),
        VULKAN_BUFFER_USAGE_MODE_COMPUTE_STORAGE_READ_WRITE
    );
    // (read flags win over read-write)
    assert_eq!(
        default_buffer_usage_mode(&buffer(B::VERTEX | B::COMPUTE_STORAGE_WRITE)),
        VULKAN_BUFFER_USAGE_MODE_VERTEX_READ
    );

    let texture = |usage| resources::VulkanTexture {
        container: Default::default(),
        used_region: None,
        image: 0,
        full_view: 0,
        swizzle: IDENTITY_SWIZZLE,
        aspect_flags: 0,
        depth: 1,
        level_count: 1,
        layer_count: 1,
        ty: TextureType::Texture2D,
        usage,
        subresources: Vec::new(),
        marked_for_destroy: Default::default(),
        externally_managed: false,
        reference_count: Default::default(),
    };
    assert_eq!(
        default_texture_usage_mode(&texture(T::SAMPLER | T::COLOR_TARGET)),
        VulkanTextureUsageMode::Sampler
    );
    assert_eq!(
        default_texture_usage_mode(&texture(T::COLOR_TARGET | T::COMPUTE_STORAGE_READ)),
        VulkanTextureUsageMode::ColorAttachment
    );
    assert_eq!(
        default_texture_usage_mode(&texture(T::COMPUTE_STORAGE_SIMULTANEOUS_READ_WRITE)),
        VulkanTextureUsageMode::ComputeStorageReadWrite
    );
}

#[test]
fn device_extensions_and_driver_versions() {
    let extension = |name: &str| {
        let mut e = VkExtensionProperties::default();
        for (d, s) in e.extension_name.iter_mut().zip(name.bytes()) {
            *d = s as std::ffi::c_char;
        }
        e
    };
    let (supports, required) = VulkanExtensions::check(&[
        extension("VK_KHR_swapchain"),
        extension("VK_KHR_driver_properties"),
    ]);
    assert!(!required);
    assert!(supports.khr_swapchain && supports.khr_driver_properties);
    let (supports, required) = VulkanExtensions::check(&[
        extension("VK_EXT_texture_compression_astc_hdr"),
        extension("VK_KHR_maintenance1"),
        extension("VK_KHR_swapchain"),
        extension("VK_KHR_maintenance2"),
    ]);
    assert!(required);
    let names: Vec<_> = supports
        .names()
        .iter()
        .map(|n| n.to_str().unwrap())
        .collect();
    assert_eq!(
        names,
        [
            "VK_KHR_swapchain",
            "VK_KHR_maintenance1",
            "VK_EXT_texture_compression_astc_hdr"
        ]
    );

    // Mesa 25.0.7 (10|10|12), NVIDIA 535.183.1.0 (10|8|8|6)
    assert_eq!(driver_version_string((25 << 22) | 7, 0x10005), "25.0.7");
    assert_eq!(
        driver_version_string((535 << 22) | (183 << 14) | (1 << 6), 0x10de),
        "535.183.1.0"
    );
    let intel = driver_version_string((101 << 14) | 5444, 0x8086);
    if cfg!(windows) {
        assert_eq!(intel, "101.5444");
    } else {
        assert_eq!(intel, "0.405.1348");
    }
}

#[test]
fn vulkan_options() {
    let props = Properties::new();
    let mut features = device::VulkanFeatures::default();
    device::add_opt_in_vulkan_options(&props, true, &mut features);
    assert!(!features.uses_custom_vulkan_options);

    // A property of another type is upstream's NULL options.
    props
        .set(PROP_GPU_DEVICE_CREATE_VULKAN_OPTIONS_POINTER, 1)
        .unwrap();
    device::add_opt_in_vulkan_options(&props, true, &mut features);
    assert!(!features.uses_custom_vulkan_options);

    let mut vk10 = vec![false; 55];
    vk10[1] = true; // fullDrawIndexUint32
    let options = VulkanOptions {
        vulkan_api_version: vk_make_api_version(0, 1, 2, 0),
        feature_list: vec![
            VulkanFeatureStructure {
                s_type: VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_FEATURES_2,
                features: {
                    let mut f = vec![false; 55];
                    f[19] = true; // samplerAnisotropy
                    f
                },
            },
            VulkanFeatureStructure {
                s_type: VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_VARIABLE_POINTERS_FEATURES,
                features: vec![true, false], // variablePointersStorageBuffer
            },
            VulkanFeatureStructure {
                s_type: VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_VULKAN_1_2_FEATURES,
                features: vec![false, true], // drawIndirectCount
            },
            VulkanFeatureStructure {
                // (1.3 structures need API version 1.3)
                s_type: VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_VULKAN_1_3_FEATURES,
                features: vec![true],
            },
        ],
        vulkan_10_physical_device_features: Some(vk10),
        device_extension_names: vec!["VK_KHR_maintenance2".into()],
        instance_extension_names: vec!["VK_KHR_surface".into(), "bad\0name".into()],
    };
    props
        .set_any(PROP_GPU_DEVICE_CREATE_VULKAN_OPTIONS_POINTER, options)
        .unwrap();
    let mut features = device::VulkanFeatures::default();
    device::add_opt_in_vulkan_options(&props, true, &mut features);
    assert!(features.uses_custom_vulkan_options);
    assert_eq!(features.desired_api_version, (1 << 22) | (2 << 12));
    let v10 = &features.desired_vulkan10_device_features;
    assert_eq!((v10.full_draw_index_uint32, v10.sampler_anisotropy), (1, 1));
    let v11 = &features.desired_vulkan11_device_features;
    assert_eq!(
        v11.s_type,
        VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_VULKAN_1_1_FEATURES
    );
    assert_eq!(
        (v11.variable_pointers_storage_buffer, v11.variable_pointers),
        (1, 0)
    );
    let v12 = &features.desired_vulkan12_device_features;
    assert_eq!(
        (v12.sampler_mirror_clamp_to_edge, v12.draw_indirect_count),
        (0, 1)
    );
    let v13 = &features.desired_vulkan13_device_features;
    assert_eq!(v13.robust_image_access, 0);
    assert_eq!(
        features.additional_device_extension_names,
        [c"VK_KHR_maintenance2".to_owned()]
    );
    assert_eq!(
        features.additional_instance_extension_names,
        [c"VK_KHR_surface".to_owned(), c"".to_owned()]
    );
}

// ---------------------------------------------------------------------------
// With a device
// ---------------------------------------------------------------------------

/// The shader code as bytes.
fn bytes(words: &[u32]) -> Vec<u8> {
    words.iter().flat_map(|w| w.to_ne_bytes()).collect()
}

/// Video up on the offscreen driver; quit when dropped.
struct Video;

impl Video {
    fn init() -> Video {
        init::quit();
        hints::set(hints::VIDEO_DRIVER, "offscreen").unwrap();
        init::init(InitFlags::VIDEO).unwrap();
        Video
    }
}

impl Drop for Video {
    fn drop(&mut self) {
        init::quit();
        hints::reset(hints::VIDEO_DRIVER);
    }
}

/// A debug-mode renderer made without the front end, destroyed when
/// dropped.
struct TestRenderer(VulkanRenderer);

impl std::ops::Deref for TestRenderer {
    type Target = VulkanRenderer;
    fn deref(&self) -> &VulkanRenderer {
        &self.0
    }
}

impl Drop for TestRenderer {
    fn drop(&mut self) {
        self.0.destroy();
    }
}

/// The renderer, or `None` (with the skip reported) without Vulkan.
fn renderer() -> Option<TestRenderer> {
    let props = Properties::new();
    props
        .set(PROP_GPU_DEVICE_CREATE_SHADERS_SPIRV_BOOLEAN, true)
        .unwrap();
    props
        .set(PROP_GPU_DEVICE_CREATE_DEBUGMODE_BOOLEAN, true)
        .unwrap();
    let video = crate::video::core::driver().unwrap();
    if !device::prepare_driver(&*video, &props) {
        crate::test_support::skip("vulkan", "no Vulkan GPU device (PrepareDriver failed)");
        return None;
    }
    match device::create_device(true, false, &props) {
        Ok(renderer) => Some(TestRenderer(renderer)),
        Err(e) => {
            crate::test_support::skip(
                "vulkan",
                format_args!("no Vulkan GPU device ({})", e.message()),
            );
            None
        }
    }
}

/// A device through the front end, or `None` (with the skip reported).
fn front_end_device() -> Option<Device> {
    match Device::new(ShaderFormat::SPIRV, true, Some("vulkan")) {
        Ok(device) => Some(device),
        Err(e) => {
            crate::test_support::skip(
                "vulkan",
                format_args!("no Vulkan GPU device ({})", e.message()),
            );
            None
        }
    }
}

fn texture_info(
    texture_type: TextureType,
    format: TextureFormat,
    usage: TextureUsageFlags,
    layers: u32,
    levels: u32,
) -> TextureCreateInfo {
    TextureCreateInfo {
        texture_type,
        format,
        usage,
        width: 16,
        height: 16,
        layer_count_or_depth: layers,
        num_levels: levels,
        sample_count: SampleCount::One,
        props: None,
    }
}

#[test]
fn device_formats_and_properties() {
    let _l = crate::test_support::test_lock();
    let _v = Video::init();
    let Some(device) = front_end_device() else {
        return;
    };
    assert_eq!(device.driver(), "vulkan");
    assert_eq!(device.shader_formats(), ShaderFormat::SPIRV);
    assert!(crate::gpu::supports_shader_formats(
        ShaderFormat::SPIRV,
        Some("vulkan")
    ));
    // (Direct3D 12 takes DXIL on Windows)
    #[cfg(not(windows))]
    assert!(!crate::gpu::supports_shader_formats(
        ShaderFormat::DXIL,
        None
    ));

    let props = device.properties();
    let name = props.get_string(PROP_GPU_DEVICE_NAME_STRING).unwrap();
    let version = props
        .get_string(PROP_GPU_DEVICE_DRIVER_VERSION_STRING)
        .unwrap();
    assert!(!name.is_empty());
    assert!(version.split('.').count() >= 2, "{version}");
    eprintln!(
        "note: Vulkan device {name}, driver {:?} {version}",
        props.get_string(crate::gpu::PROP_GPU_DEVICE_DRIVER_NAME_STRING)
    );

    // The formats upstream's documentation guarantees.
    use TextureFormat as F;
    use TextureUsageFlags as U;
    let t2d = TextureType::Texture2D;
    for format in [F::R8G8B8A8_UNORM, F::B8G8R8A8_UNORM, F::R16G16B16A16_FLOAT] {
        assert!(device.texture_supports_format(format, t2d, U::SAMPLER | U::COLOR_TARGET));
    }
    assert!(device.texture_supports_format(
        F::D16_UNORM,
        t2d,
        U::SAMPLER | U::DEPTH_STENCIL_TARGET
    ));
    assert!(device.texture_supports_format(F::R32_FLOAT, t2d, U::COMPUTE_STORAGE_WRITE));
    assert!(device.texture_supports_format(F::R8G8B8A8_UNORM, TextureType::Cube, U::SAMPLER));
    assert!(device.texture_supports_format(F::R8G8B8A8_UNORM, TextureType::Texture3D, U::SAMPLER));
    // (sRGB isn't storable)
    assert!(!device.texture_supports_format(F::R8G8B8A8_UNORM_SRGB, t2d, U::COMPUTE_STORAGE_WRITE));
    // (whatever the driver says about compressed formats, the query works)
    let _ = device.texture_supports_format(F::BC7_RGBA_UNORM, t2d, U::SAMPLER);
    let _ = device.texture_supports_format(F::ASTC_4x4_UNORM, t2d, U::SAMPLER);

    assert!(device.texture_supports_sample_count(F::R8G8B8A8_UNORM, SampleCount::One));
    assert!(device.texture_supports_sample_count(F::R8G8B8A8_UNORM, SampleCount::Four));
    assert!(device.texture_supports_sample_count(F::D32_FLOAT, SampleCount::Four));

    // An empty command buffer submits, and one cancels.
    device.acquire_command_buffer().unwrap().submit().unwrap();
    device.acquire_command_buffer().unwrap().cancel().unwrap();
    device.wait_for_idle().unwrap();
}

#[test]
fn device_resources() {
    let _l = crate::test_support::test_lock();
    let _v = Video::init();
    let Some(device) = front_end_device() else {
        return;
    };
    use crate::gpu::BufferUsageFlags as B;
    use TextureFormat as F;
    use TextureUsageFlags as U;

    // Buffers
    let buffers: Vec<_> = [
        B::VERTEX,
        B::INDEX,
        B::INDIRECT,
        B::GRAPHICS_STORAGE_READ,
        B::COMPUTE_STORAGE_READ | B::COMPUTE_STORAGE_WRITE,
        B::VERTEX | B::INDEX | B::INDIRECT,
    ]
    .into_iter()
    .map(|usage| {
        device
            .create_buffer(&BufferCreateInfo {
                usage,
                size: 4096,
                props: None,
            })
            .unwrap()
    })
    .collect();
    buffers[0].set_name("vertices");
    let big = device
        .create_buffer(&BufferCreateInfo {
            usage: B::VERTEX,
            size: 3 << 20,
            props: None,
        })
        .unwrap();

    // Transfer buffers: persistently mapped.
    let mut upload = device
        .create_transfer_buffer(&TransferBufferCreateInfo {
            usage: TransferBufferUsage::Upload,
            size: 1024,
            props: None,
        })
        .unwrap();
    {
        let mut map = upload.map(false).unwrap();
        assert_eq!(map.len(), 1024);
        for (i, b) in map.iter_mut().enumerate() {
            *b = i as u8;
        }
    }
    {
        let map = upload.map(true).unwrap(); // (unused by the GPU: not cycled)
        assert!(map.iter().enumerate().all(|(i, &b)| b == i as u8));
    }
    let download = device
        .create_transfer_buffer(&TransferBufferCreateInfo {
            usage: TransferBufferUsage::Download,
            size: 64,
            props: None,
        })
        .unwrap();

    // Textures of several types, formats and usages.
    let named = Properties::new();
    named
        .set(PROP_GPU_TEXTURE_CREATE_NAME_STRING, "named texture")
        .unwrap();
    let mut infos = vec![
        texture_info(
            TextureType::Texture2D,
            F::R8G8B8A8_UNORM,
            U::SAMPLER | U::COLOR_TARGET,
            1,
            5,
        ),
        texture_info(
            TextureType::Texture2D,
            F::B8G8R8A8_UNORM_SRGB,
            U::COLOR_TARGET,
            1,
            1,
        ),
        texture_info(TextureType::Texture2D, F::A8_UNORM, U::SAMPLER, 1, 1),
        texture_info(TextureType::Texture2D, F::B4G4R4A4_UNORM, U::SAMPLER, 1, 1),
        texture_info(
            TextureType::Texture2D,
            F::D16_UNORM,
            U::SAMPLER | U::DEPTH_STENCIL_TARGET,
            1,
            1,
        ),
        texture_info(
            TextureType::Texture2D,
            F::R32_FLOAT,
            U::COMPUTE_STORAGE_READ | U::COMPUTE_STORAGE_WRITE,
            1,
            2,
        ),
        texture_info(
            TextureType::Texture2D,
            F::R8G8B8A8_UNORM,
            U::GRAPHICS_STORAGE_READ,
            1,
            1,
        ),
        texture_info(
            TextureType::Texture2D,
            F::R32_UINT,
            U::COMPUTE_STORAGE_SIMULTANEOUS_READ_WRITE,
            1,
            1,
        ),
        texture_info(
            TextureType::Texture2DArray,
            F::R16G16B16A16_FLOAT,
            U::SAMPLER | U::COLOR_TARGET,
            4,
            2,
        ),
        texture_info(
            TextureType::Texture3D,
            F::R8G8B8A8_UNORM,
            U::SAMPLER | U::COLOR_TARGET,
            4,
            1,
        ),
        texture_info(
            TextureType::Cube,
            F::R8G8B8A8_UNORM,
            U::SAMPLER | U::COLOR_TARGET,
            6,
            3,
        ),
        texture_info(TextureType::CubeArray, F::R8G8B8A8_UNORM, U::SAMPLER, 12, 1),
    ];
    infos[0].props = Some(named);
    for format in [F::D24_UNORM_S8_UINT, F::D32_FLOAT_S8_UINT, F::D32_FLOAT] {
        if device.texture_supports_format(format, TextureType::Texture2D, U::DEPTH_STENCIL_TARGET) {
            infos.push(texture_info(
                TextureType::Texture2D,
                format,
                U::DEPTH_STENCIL_TARGET,
                1,
                1,
            ));
        }
    }
    let mut multisample = texture_info(
        TextureType::Texture2D,
        F::R8G8B8A8_UNORM,
        U::COLOR_TARGET,
        1,
        1,
    );
    multisample.sample_count = SampleCount::Four;
    infos.push(multisample);
    let textures: Vec<_> = infos
        .iter()
        .map(|info| device.create_texture(info).unwrap())
        .collect();
    textures[1].set_name("srgb");

    // Samplers
    let samplers: Vec<_> = [
        SamplerCreateInfo::default(),
        SamplerCreateInfo {
            min_filter: Filter::Linear,
            mag_filter: Filter::Linear,
            mipmap_mode: SamplerMipmapMode::Linear,
            address_mode_u: SamplerAddressMode::ClampToEdge,
            address_mode_v: SamplerAddressMode::MirroredRepeat,
            max_lod: 8.0,
            enable_anisotropy: true,
            max_anisotropy: 4.0,
            ..Default::default()
        },
        SamplerCreateInfo {
            enable_compare: true,
            compare_op: CompareOp::LessOrEqual,
            ..Default::default()
        },
    ]
    .iter()
    .map(|info| device.create_sampler(info).unwrap())
    .collect();

    // Shaders
    let shader = |code: &[u32], stage, samplers, uniforms| {
        device
            .create_shader(&ShaderCreateInfo {
                code: &bytes(code),
                entrypoint: "main",
                format: ShaderFormat::SPIRV,
                stage,
                num_samplers: samplers,
                num_uniform_buffers: uniforms,
                ..Default::default()
            })
            .unwrap()
    };
    let vertex_null = shader(test_spirv::VERTEX_NULL, ShaderStage::Vertex, 0, 0);
    let vertex_input = shader(test_spirv::VERTEX_INPUT_UNIFORM, ShaderStage::Vertex, 0, 1);
    let fragment_solid = shader(test_spirv::FRAGMENT_SOLID, ShaderStage::Fragment, 0, 0);
    let fragment_sampled = shader(
        test_spirv::FRAGMENT_SAMPLED_UNIFORM,
        ShaderStage::Fragment,
        1,
        1,
    );
    // Not SPIR-V
    let e = device
        .create_shader(&ShaderCreateInfo {
            code: b"DXBC....",
            entrypoint: "main",
            format: ShaderFormat::SPIRV,
            stage: ShaderStage::Vertex,
            ..Default::default()
        })
        .unwrap_err();
    assert_eq!(e.message(), "The provided shader code is not valid SPIR-V!");

    // Graphics pipelines
    let color_target = [ColorTargetDescription {
        format: F::R8G8B8A8_UNORM,
        blend_state: Default::default(),
    }];
    let blended = [ColorTargetDescription {
        format: F::B8G8R8A8_UNORM,
        blend_state: crate::gpu::ColorTargetBlendState {
            src_color_blendfactor: BlendFactor::SrcAlpha,
            dst_color_blendfactor: BlendFactor::OneMinusSrcAlpha,
            color_blend_op: BlendOp::Add,
            src_alpha_blendfactor: BlendFactor::One,
            dst_alpha_blendfactor: BlendFactor::Zero,
            alpha_blend_op: BlendOp::Add,
            color_write_mask: crate::gpu::ColorComponentFlags::R
                | crate::gpu::ColorComponentFlags::A,
            enable_blend: true,
            enable_color_write_mask: true,
        },
    }];
    let simple = GraphicsPipelineCreateInfo {
        vertex_shader: &vertex_null,
        fragment_shader: &fragment_solid,
        vertex_input_state: Default::default(),
        primitive_type: PrimitiveType::TriangleList,
        rasterizer_state: Default::default(),
        multisample_state: Default::default(),
        depth_stencil_state: Default::default(),
        target_info: GraphicsPipelineTargetInfo {
            color_target_descriptions: &color_target,
            depth_stencil_format: F::D16_UNORM,
            has_depth_stencil_target: false,
        },
        props: None,
    };
    let vertex_buffers = [VertexBufferDescription {
        slot: 0,
        pitch: 8,
        input_rate: VertexInputRate::Vertex,
        instance_step_rate: 0,
    }];
    let vertex_attributes = [VertexAttribute {
        location: 0,
        buffer_slot: 0,
        format: VertexElementFormat::Float2,
        offset: 0,
    }];
    let pipelines = vec![
        device.create_graphics_pipeline(&simple).unwrap(),
        device
            .create_graphics_pipeline(&GraphicsPipelineCreateInfo {
                vertex_shader: &vertex_input,
                fragment_shader: &fragment_sampled,
                vertex_input_state: VertexInputState {
                    vertex_buffer_descriptions: &vertex_buffers,
                    vertex_attributes: &vertex_attributes,
                },
                primitive_type: PrimitiveType::TriangleStrip,
                rasterizer_state: RasterizerState {
                    cull_mode: CullMode::Back,
                    fill_mode: crate::gpu::FillMode::Line,
                    enable_depth_bias: true,
                    depth_bias_constant_factor: 1.0,
                    enable_depth_clip: true,
                    ..Default::default()
                },
                multisample_state: Default::default(),
                depth_stencil_state: crate::gpu::DepthStencilState {
                    compare_op: CompareOp::Less,
                    enable_depth_test: true,
                    enable_depth_write: true,
                    ..Default::default()
                },
                target_info: GraphicsPipelineTargetInfo {
                    color_target_descriptions: &blended,
                    depth_stencil_format: F::D16_UNORM,
                    has_depth_stencil_target: true,
                },
                props: None,
            })
            .unwrap(),
        device
            .create_graphics_pipeline(&GraphicsPipelineCreateInfo {
                multisample_state: MultisampleState {
                    sample_count: SampleCount::Four,
                    ..Default::default()
                },
                primitive_type: PrimitiveType::LineStrip,
                ..simple.clone()
            })
            .unwrap(),
        // (no color targets: depth only)
        device
            .create_graphics_pipeline(&GraphicsPipelineCreateInfo {
                primitive_type: PrimitiveType::LineList,
                target_info: GraphicsPipelineTargetInfo {
                    color_target_descriptions: &[],
                    depth_stencil_format: F::D16_UNORM,
                    has_depth_stencil_target: true,
                },
                ..simple.clone()
            })
            .unwrap(),
    ];
    assert_eq!(pipelines[1].header.num_vertex_uniform_buffers, 1);
    assert_eq!(pipelines[1].header.num_fragment_samplers, 1);
    assert_eq!(pipelines[1].header.num_fragment_uniform_buffers, 1);
    assert_eq!(pipelines[0].header, Default::default());
    // The shaders can go once the pipelines are made.
    drop((vertex_null, vertex_input, fragment_solid, fragment_sampled));

    // Compute pipelines
    let code = bytes(test_spirv::COMPUTE_FILL);
    let compute = device
        .create_compute_pipeline(&ComputePipelineCreateInfo {
            code: &code,
            entrypoint: "main",
            format: ShaderFormat::SPIRV,
            num_readwrite_storage_buffers: 1,
            threadcount_x: 1,
            threadcount_y: 1,
            threadcount_z: 1,
            ..Default::default()
        })
        .unwrap();
    assert_eq!(compute.header.num_readwrite_storage_buffers, 1);
    let e = device
        .create_compute_pipeline(&ComputePipelineCreateInfo {
            code: &code[..2],
            entrypoint: "main",
            format: ShaderFormat::SPIRV,
            threadcount_x: 1,
            threadcount_y: 1,
            threadcount_z: 1,
            ..Default::default()
        })
        .unwrap_err();
    assert_eq!(e.message(), "The provided shader code is not valid SPIR-V!");

    // Release everything; waiting destroys it.
    drop((
        buffers, big, upload, download, textures, samplers, pipelines, compute,
    ));
    device.wait_for_idle().unwrap();
}

/// The memory type and allocation of a buffer.
fn region_of(container: &BufferContainer) -> Arc<memory::UsedRegion> {
    lock(&container.state).active_buffer.used_region.clone()
}

/// The allocation info of the allocation a region is in.
fn info_of(r: &VulkanRenderer, region: &memory::UsedRegion) -> memory::AllocationInfo {
    let allocator = r.memory_allocator.lock();
    let a = allocator.borrow();
    for t in 0..VK_MAX_MEMORY_TYPES {
        for i in 0..a.allocation_count(t) {
            let key = a.allocation_key(t, i);
            if Arc::ptr_eq(a.allocation(key), &region.allocation) {
                a.check_indices();
                return a.allocation_info(key);
            }
        }
    }
    panic!("the region's allocation isn't in the allocator");
}

#[test]
fn allocator_on_a_device() {
    let _l = crate::test_support::test_lock();
    let _v = Video::init();
    let Some(r) = renderer() else {
        return;
    };
    use crate::gpu::BufferUsageFlags as B;

    // The uniform buffer pool: 32 buffers of UNIFORM_BUFFER_SIZE in a small
    // allocation.
    let uniform_regions: Vec<_> = lock(&r.uniform_buffer_pool)
        .iter()
        .map(|u| lock(&u.state).buffer.used_region.clone())
        .collect();
    assert_eq!(uniform_regions.len(), 32);
    let uniform_info = info_of(&r, &uniform_regions[0]);
    assert_eq!(uniform_info.size, SMALL_ALLOCATION_SIZE);
    assert!(uniform_regions
        .iter()
        .all(|u| Arc::ptr_eq(&u.allocation, &uniform_regions[0].allocation)));
    assert!(uniform_regions.iter().all(|u| u.resource_size == 32768));
    assert!(uniform_info.used_space >= 32 * 32768);

    // A small GPU buffer goes into a small allocation, a large one into its
    // own of a multiple of 64 MiB.
    let small = r
        .create_buffer_container(1000, B::VERTEX, VulkanBufferType::Gpu, false, None)
        .unwrap();
    let small_region = region_of(&small);
    assert_eq!(small_region.resource_size, 1000);
    let small_info = info_of(&r, &small_region);
    assert_eq!(small_info.size, SMALL_ALLOCATION_SIZE);
    assert!(small_info
        .used_regions
        .contains(&(small_region.offset, small_region.size)));

    let large = r
        .create_buffer_container(65 << 20, B::INDEX, VulkanBufferType::Gpu, false, None)
        .unwrap();
    let large_info = info_of(&r, &region_of(&large));
    assert_eq!(large_info.size, 2 * LARGE_ALLOCATION_INCREMENT);
    assert_eq!(large_info.used_regions.len(), 1);

    // A transfer buffer is dedicated: an allocation of its own, of its size.
    let transfer = r
        .create_buffer_container(
            5000,
            B(0),
            VulkanBufferType::Transfer,
            true,
            Some("staging"),
        )
        .unwrap();
    let transfer_region = region_of(&transfer);
    let transfer_info = info_of(&r, &transfer_region);
    assert_eq!(transfer_info.used_regions, [(0, transfer_info.size)]);
    assert!(transfer_info.size >= 5000);
    assert!(transfer_region.allocation.map_pointer().is_some());

    // Mapping, and cycling a buffer the GPU uses.
    let p = r.map_transfer_buffer_internal(&transfer, false).unwrap();
    // SAFETY: the transfer buffer's 5000 mapped bytes, used by nothing else.
    unsafe { p.as_ptr().write_bytes(0xA5, 5000) };
    let first = lock(&transfer.state).active_buffer.clone();
    first.reference_count.store(1, Ordering::SeqCst);
    let q = r.map_transfer_buffer_internal(&transfer, true).unwrap();
    assert_ne!(p, q);
    {
        let state = lock(&transfer.state);
        assert_eq!(state.buffers.len(), 2);
        assert!(!Arc::ptr_eq(&state.active_buffer, &first));
    }
    // SAFETY: the first buffer's mapping, still valid.
    assert_eq!(unsafe { *p.as_ptr().add(4999) }, 0xA5);
    first.reference_count.store(0, Ordering::SeqCst);
    // (the first buffer is free again: cycling takes it back)
    lock(&transfer.state)
        .active_buffer
        .reference_count
        .store(1, Ordering::SeqCst);
    assert_eq!(r.map_transfer_buffer_internal(&transfer, true).unwrap(), p);
    lock(&transfer.state).buffers[1]
        .reference_count
        .store(0, Ordering::SeqCst);

    // Releasing and waiting gives the regions back.
    let small_used_before = small_info.used_space;
    r.release_buffer_container(&small);
    assert_eq!(lock(&r.dispose).buffers_to_destroy.len(), 1);
    // (released twice: still once)
    r.release_buffer_container(&small);
    r.wait_internal().unwrap();
    assert!(lock(&r.dispose).buffers_to_destroy.is_empty());
    let after = info_of(&r, &small_region);
    assert_eq!(after.used_space, small_used_before - small_region.size);
    assert!(!after
        .used_regions
        .contains(&(small_region.offset, small_region.size)));

    r.release_buffer_container(&large);
    r.release_buffer_container(&transfer);
    r.wait_internal().unwrap();
    let a = r.memory_allocator.lock();
    assert!(a.borrow().check_empty_allocations);
}

/// A primary command buffer being recorded, with its pool.
struct RawCommandBuffer<'a> {
    r: &'a VulkanRenderer,
    pool: VkCommandPool,
    command_buffer: VkCommandBuffer,
}

impl<'a> RawCommandBuffer<'a> {
    fn begin(r: &'a VulkanRenderer) -> RawCommandBuffer<'a> {
        let pool_info = VkCommandPoolCreateInfo {
            s_type: VK_STRUCTURE_TYPE_COMMAND_POOL_CREATE_INFO,
            queue_family_index: r.queue_family_index,
            ..Default::default()
        };
        let mut pool = 0;
        // SAFETY: a valid create info.
        let res =
            unsafe { (r.dev.create_command_pool)(r.logical_device, &pool_info, null(), &mut pool) };
        assert_eq!(res, VK_SUCCESS);
        let alloc_info = VkCommandBufferAllocateInfo {
            s_type: VK_STRUCTURE_TYPE_COMMAND_BUFFER_ALLOCATE_INFO,
            command_pool: pool,
            level: VK_COMMAND_BUFFER_LEVEL_PRIMARY,
            command_buffer_count: 1,
            ..Default::default()
        };
        let mut command_buffer = std::ptr::null_mut();
        // SAFETY: a valid allocate info.
        let res = unsafe {
            (r.dev.allocate_command_buffers)(r.logical_device, &alloc_info, &mut command_buffer)
        };
        assert_eq!(res, VK_SUCCESS);
        let begin_info = VkCommandBufferBeginInfo {
            s_type: VK_STRUCTURE_TYPE_COMMAND_BUFFER_BEGIN_INFO,
            flags: VK_COMMAND_BUFFER_USAGE_ONE_TIME_SUBMIT_BIT,
            ..Default::default()
        };
        // SAFETY: a new command buffer.
        let res = unsafe { (r.dev.begin_command_buffer)(command_buffer, &begin_info) };
        assert_eq!(res, VK_SUCCESS);
        RawCommandBuffer {
            r,
            pool,
            command_buffer,
        }
    }

    /// End, submit and wait for the command buffer.
    fn submit_and_wait(self) {
        let r = self.r;
        // SAFETY: the command buffer, recorded.
        assert_eq!(
            unsafe { (r.dev.end_command_buffer)(self.command_buffer) },
            VK_SUCCESS
        );
        let submit_info = VkSubmitInfo {
            s_type: VK_STRUCTURE_TYPE_SUBMIT_INFO,
            command_buffer_count: 1,
            p_command_buffers: &self.command_buffer,
            ..Default::default()
        };
        // SAFETY: the device's queue and a recorded command buffer.
        unsafe {
            assert_eq!(
                (r.dev.queue_submit)(r.unified_queue, 1, &submit_info, 0),
                VK_SUCCESS
            );
            assert_eq!((r.dev.queue_wait_idle)(r.unified_queue), VK_SUCCESS);
        }
    }
}

impl Drop for RawCommandBuffer<'_> {
    fn drop(&mut self) {
        // SAFETY: the pool, whose command buffer is done.
        unsafe { (self.r.dev.destroy_command_pool)(self.r.logical_device, self.pool, null()) };
    }
}

#[test]
fn defragmentation_moves_buffers() {
    let _l = crate::test_support::test_lock();
    let _v = Video::init();
    let Some(r) = renderer() else {
        return;
    };
    use crate::gpu::BufferUsageFlags as B;
    const MIB: VkDeviceSize = 1 << 20;

    // Fill a small allocation with 1 MiB buffers.
    let mut containers = Vec::new();
    let first = r
        .create_buffer_container(MIB, B::VERTEX, VulkanBufferType::Gpu, false, None)
        .unwrap();
    let fragmented = region_of(&first).allocation.clone();
    containers.push(first);
    loop {
        let info = info_of(&r, &region_of(&containers[0]));
        if !info.free_regions.iter().any(|&(_, size)| size >= MIB + 256) {
            break;
        }
        let c = r
            .create_buffer_container(MIB, B::VERTEX, VulkanBufferType::Gpu, false, None)
            .unwrap();
        assert!(Arc::ptr_eq(&region_of(&c).allocation, &fragmented));
        containers.push(c);
    }
    assert!(containers.len() >= 4);

    // Holes: release every other buffer (not the last, which may border the
    // free tail).
    let kept: Vec<_> = containers
        .iter()
        .enumerate()
        .filter(|&(i, _)| i % 2 == 0 || i == containers.len() - 1)
        .map(|(_, c)| c.clone())
        .collect();
    for (i, c) in containers.iter().enumerate() {
        if i % 2 == 1 && i != containers.len() - 1 {
            r.release_buffer_container(c);
        }
    }
    r.wait_internal().unwrap();
    let holes = info_of(&r, &region_of(&kept[0])).free_regions.len();
    assert!(holes > 1, "{holes}");

    // A buffer that fits no hole: the fragmented allocation is marked for
    // defrag and the buffer goes into a new one.
    let bigger = r
        .create_buffer_container(MIB + MIB / 2, B::VERTEX, VulkanBufferType::Gpu, false, None)
        .unwrap();
    assert!(!Arc::ptr_eq(&region_of(&bigger).allocation, &fragmented));
    let info = info_of(&r, &region_of(&kept[0]));
    assert!(!info.available_for_allocation);
    assert_eq!(
        r.memory_allocator
            .lock()
            .borrow()
            .allocations_to_defrag
            .len(),
        1
    );
    let uniform_buffers: Vec<_> = lock(&r.uniform_buffer_pool).clone();
    let moved_uniforms: Vec<bool> = uniform_buffers
        .iter()
        .map(|u| Arc::ptr_eq(&lock(&u.state).buffer.used_region.allocation, &fragmented))
        .collect();

    // Defragment: the buffers move, the old ones are released.
    let old: Vec<Arc<VulkanBuffer>> = kept
        .iter()
        .map(|c| lock(&c.state).active_buffer.clone())
        .collect();
    let raw = RawCommandBuffer::begin(&r);
    let mut command_buffer = VulkanCommandBuffer::new(raw.command_buffer);
    r.defragment_memory(&mut command_buffer).unwrap();
    assert!(command_buffer.is_defrag);
    assert!(r.defrag_in_progress.load(Ordering::SeqCst));
    assert!(r
        .memory_allocator
        .lock()
        .borrow()
        .allocations_to_defrag
        .is_empty());
    raw.submit_and_wait();
    for (c, old) in kept.iter().zip(&old) {
        let state = lock(&c.state);
        assert!(!Arc::ptr_eq(&state.active_buffer, old));
        assert!(Arc::ptr_eq(&state.buffers[0], &state.active_buffer));
        assert!(!Arc::ptr_eq(
            &state.active_buffer.used_region.allocation,
            &fragmented
        ));
        assert!(old.marked_for_destroy.load(Ordering::SeqCst));
        let container = lock(&state.active_buffer.container);
        assert!(container
            .as_ref()
            .unwrap()
            .container
            .ptr_eq(&Arc::downgrade(c)));
    }
    for (u, moved) in uniform_buffers.iter().zip(&moved_uniforms) {
        let buffer = lock(&u.state).buffer.clone();
        assert!(!Arc::ptr_eq(&buffer.used_region.allocation, &fragmented));
        assert!(
            lock(&buffer.uniform_buffer_for_defrag).upgrade().is_some(),
            "{moved}"
        );
    }

    // (VULKAN_INTERNAL_CleanCommandBuffer's part: the references go)
    for b in command_buffer.used_buffers.drain(..) {
        b.reference_count.fetch_sub(1, Ordering::SeqCst);
    }
    r.defrag_in_progress.store(false, Ordering::SeqCst);
    r.perform_pending_destroys();

    // The fragmented allocation is empty now; freeing it is Submit's job
    // (see the submission test).
    let allocator = r.memory_allocator.lock();
    let a = allocator.borrow_mut();
    assert!(a.check_empty_allocations);
    let (t, i) = (0..VK_MAX_MEMORY_TYPES)
        .flat_map(|t| (0..a.allocation_count(t)).map(move |i| (t, i)))
        .find(|&(t, i)| Arc::ptr_eq(a.allocation(a.allocation_key(t, i)), &fragmented))
        .unwrap();
    let info = a.allocation_info(a.allocation_key(t, i));
    assert!(info.used_regions.is_empty());
    assert_eq!(info.free_regions, [(0, SMALL_ALLOCATION_SIZE)]);
    a.check_indices();
    drop(a);
    drop(allocator);
    r.deallocate_memory(t, i);

    // (the front end releases everything before the device goes)
    for c in kept.iter().chain([&bigger]) {
        r.release_buffer_container(c);
    }
    r.wait_internal().unwrap();
}

// ---------------------------------------------------------------------------
// Command buffers, passes, swapchains and fences
// ---------------------------------------------------------------------------

/// The bytes of 32-bit words.
fn word_bytes(words: &[u32]) -> Vec<u8> {
    words.iter().flat_map(|w| w.to_ne_bytes()).collect()
}

/// The bytes of floats.
fn float_bytes(floats: &[f32]) -> Vec<u8> {
    floats.iter().flat_map(|f| f.to_ne_bytes()).collect()
}

/// An upload transfer buffer holding `data`.
fn upload_buffer(device: &Device, data: &[u8]) -> TransferBuffer {
    let mut transfer_buffer = device
        .create_transfer_buffer(&TransferBufferCreateInfo {
            usage: TransferBufferUsage::Upload,
            size: data.len() as u32,
            props: None,
        })
        .unwrap();
    transfer_buffer.map(false).unwrap().copy_from_slice(data);
    transfer_buffer
}

/// A download transfer buffer of `size` bytes.
fn download_buffer(device: &Device, size: u32) -> TransferBuffer {
    device
        .create_transfer_buffer(&TransferBufferCreateInfo {
            usage: TransferBufferUsage::Download,
            size,
            props: None,
        })
        .unwrap()
}

/// What a transfer buffer holds.
fn read_back(transfer_buffer: &mut TransferBuffer) -> Vec<u8> {
    transfer_buffer.map(false).unwrap().to_vec()
}

/// Submit a command buffer and wait for its fence.
fn submit_and_wait(device: &Device, command_buffer: CommandBuffer) {
    let fence = command_buffer.submit_and_acquire_fence().unwrap();
    device.wait_for_fences(true, &[&fence]).unwrap();
    assert!(fence.is_signaled());
}

/// A GPU buffer.
fn gpu_buffer(device: &Device, usage: BufferUsageFlags, size: u32) -> crate::gpu::Buffer {
    device
        .create_buffer(&BufferCreateInfo {
            usage,
            size,
            props: None,
        })
        .unwrap()
}

/// Upload data into a GPU buffer (in its own command buffer).
fn fill_buffer(device: &Device, buffer: &crate::gpu::Buffer, data: &[u8]) {
    let upload = upload_buffer(device, data);
    let mut cb = device.acquire_command_buffer().unwrap();
    {
        let mut copy = cb.begin_copy_pass().unwrap();
        copy.upload_to_buffer(
            &TransferBufferLocation {
                transfer_buffer: &upload,
                offset: 0,
            },
            &BufferRegion {
                buffer,
                offset: 0,
                size: data.len() as u32,
            },
            false,
        );
    }
    submit_and_wait(device, cb);
}

/// The bytes of a GPU buffer.
fn buffer_contents(device: &Device, buffer: &crate::gpu::Buffer, size: u32) -> Vec<u8> {
    let mut download = download_buffer(device, size);
    let mut cb = device.acquire_command_buffer().unwrap();
    {
        let mut copy = cb.begin_copy_pass().unwrap();
        copy.download_from_buffer(
            &BufferRegion {
                buffer,
                offset: 0,
                size,
            },
            &TransferBufferLocation {
                transfer_buffer: &download,
                offset: 0,
            },
        );
    }
    submit_and_wait(device, cb);
    read_back(&mut download)
}

/// A 2D RGBA8 texture.
fn rgba_texture(device: &Device, w: u32, h: u32, usage: TextureUsageFlags, levels: u32) -> Texture {
    device
        .create_texture(&TextureCreateInfo {
            width: w,
            height: h,
            ..texture_info(
                TextureType::Texture2D,
                TextureFormat::R8G8B8A8_UNORM,
                usage,
                1,
                levels,
            )
        })
        .unwrap()
}

/// Upload RGBA8 pixels into a texture's level (in its own command buffer).
fn fill_texture(device: &Device, texture: &Texture, level: u32, w: u32, h: u32, pixels: &[u8]) {
    let upload = upload_buffer(device, pixels);
    let mut cb = device.acquire_command_buffer().unwrap();
    {
        let mut copy = cb.begin_copy_pass().unwrap();
        copy.upload_to_texture(
            &TextureTransferInfo {
                transfer_buffer: &upload,
                offset: 0,
                pixels_per_row: w,
                rows_per_layer: h,
            },
            &TextureRegion {
                texture,
                mip_level: level,
                layer: 0,
                x: 0,
                y: 0,
                z: 0,
                w,
                h,
                d: 1,
            },
            false,
        );
    }
    submit_and_wait(device, cb);
}

/// The RGBA8 pixels of a texture's level.
fn texture_pixels(device: &Device, texture: &Texture, level: u32, w: u32, h: u32) -> Vec<u8> {
    let mut download = download_buffer(device, w * h * 4);
    let mut cb = device.acquire_command_buffer().unwrap();
    {
        let mut copy = cb.begin_copy_pass().unwrap();
        copy.download_from_texture(
            &TextureRegion {
                texture,
                mip_level: level,
                layer: 0,
                x: 0,
                y: 0,
                z: 0,
                w,
                h,
                d: 1,
            },
            &TextureTransferInfo {
                transfer_buffer: &download,
                offset: 0,
                pixels_per_row: w,
                rows_per_layer: h,
            },
        );
    }
    submit_and_wait(device, cb);
    read_back(&mut download)
}

/// `w * h` pixels of one color.
fn solid(w: u32, h: u32, color: [u8; 4]) -> Vec<u8> {
    (0..w * h).flat_map(|_| color).collect()
}

/// A color target cleared to a color.
fn clear_target(texture: &Texture, color: FColor) -> ColorTargetInfo<'_> {
    ColorTargetInfo {
        load_op: crate::gpu::LoadOp::Clear,
        clear_color: color,
        ..ColorTargetInfo::new(texture)
    }
}

const RED: [u8; 4] = [255, 0, 0, 255];
const BLUE: [u8; 4] = [0, 0, 255, 255];
const BLUE_F: FColor = FColor {
    r: 0.0,
    g: 0.0,
    b: 1.0,
    a: 1.0,
};

/// The texture container of a front-end texture.
fn container_of(texture: &Texture) -> Arc<TextureContainer> {
    texture.raw.downcast::<TextureContainer>().unwrap()
}

#[test]
fn clear_a_texture_and_download_it() {
    let _l = crate::test_support::test_lock();
    let _v = Video::init();
    let Some(device) = front_end_device() else {
        return;
    };
    use TextureUsageFlags as U;

    let target = rgba_texture(&device, 4, 4, U::COLOR_TARGET | U::SAMPLER, 1);

    // A new texture is in its default layout (the creation submitted the
    // transition); the clear sets every pixel.
    let mut cb = device.acquire_command_buffer().unwrap();
    {
        let color = FColor {
            r: 1.0,
            g: 0.0,
            b: 0.2,
            a: 1.0,
        };
        let pass = cb
            .begin_render_pass(&[clear_target(&target, color)], None)
            .unwrap();
        pass.end();
    }
    cb.submit().unwrap();
    device.wait_for_idle().unwrap();

    assert_eq!(
        texture_pixels(&device, &target, 0, 4, 4),
        solid(4, 4, [255, 0, 51, 255])
    );

    // A depth target clears too (and its render pass is another one).
    let depth = device
        .create_texture(&texture_info(
            TextureType::Texture2D,
            TextureFormat::D16_UNORM,
            U::DEPTH_STENCIL_TARGET,
            1,
            1,
        ))
        .unwrap();
    let small = rgba_texture(&device, 16, 16, U::COLOR_TARGET | U::SAMPLER, 1);
    let mut cb = device.acquire_command_buffer().unwrap();
    {
        let depth_target = DepthStencilTargetInfo {
            load_op: crate::gpu::LoadOp::Clear,
            clear_depth: 1.0,
            store_op: crate::gpu::StoreOp::DontCare,
            stencil_load_op: crate::gpu::LoadOp::DontCare,
            stencil_store_op: crate::gpu::StoreOp::DontCare,
            ..DepthStencilTargetInfo::new(&depth)
        };
        cb.begin_render_pass(&[clear_target(&small, BLUE_F)], Some(&depth_target))
            .unwrap()
            .end();
        // (a depth-only pass)
        cb.begin_render_pass(&[], Some(&depth_target))
            .unwrap()
            .end();
    }
    submit_and_wait(&device, cb);
    assert_eq!(
        texture_pixels(&device, &small, 0, 16, 16),
        solid(16, 16, BLUE)
    );

    // The render passes and framebuffers are cached.
    let renderer = device_renderer(&device);
    assert_eq!(lock(&renderer.render_pass_hash_table).len(), 3);
    assert_eq!(lock(&renderer.framebuffer_hash_table).len(), 3);
}

/// The Vulkan renderer of a front-end device.
fn device_renderer(device: &Device) -> &VulkanRenderer {
    let driver: &dyn GpuDriver = &*device.shared.driver;
    // SAFETY: the vulkan device's driver is a VulkanRenderer.
    unsafe { &*(driver as *const dyn GpuDriver as *const VulkanRenderer) }
}

#[test]
fn upload_copy_and_download_buffers_and_textures() {
    let _l = crate::test_support::test_lock();
    let _v = Video::init();
    let Some(device) = front_end_device() else {
        return;
    };
    use crate::gpu::BufferUsageFlags as B;
    use TextureUsageFlags as U;

    // Buffers: upload, copy into the middle of another, download.
    let data: Vec<u8> = (0..256u32).map(|i| (i * 7 + 3) as u8).collect();
    let a = gpu_buffer(&device, B::VERTEX, 256);
    let b = gpu_buffer(&device, B::COMPUTE_STORAGE_READ, 512);
    fill_buffer(&device, &a, &data);
    fill_buffer(&device, &b, &[0xAA; 512]);
    let mut cb = device.acquire_command_buffer().unwrap();
    {
        let mut copy = cb.begin_copy_pass().unwrap();
        copy.copy_buffer_to_buffer(
            &BufferLocation {
                buffer: &a,
                offset: 16,
            },
            &BufferLocation {
                buffer: &b,
                offset: 100,
            },
            200,
            false,
        );
    }
    submit_and_wait(&device, cb);
    assert_eq!(buffer_contents(&device, &a, 256), data);
    let contents = buffer_contents(&device, &b, 512);
    assert!(contents[..100].iter().all(|&x| x == 0xAA));
    assert_eq!(&contents[100..300], &data[16..216]);
    assert!(contents[300..].iter().all(|&x| x == 0xAA));

    // Textures: upload a pattern, copy a region into another texture,
    // download both.
    let pattern: Vec<u8> = (0..8 * 8 * 4).map(|i| (i * 5 % 251) as u8).collect();
    let src = rgba_texture(&device, 8, 8, U::SAMPLER, 1);
    let dst = rgba_texture(&device, 8, 8, U::SAMPLER | U::COLOR_TARGET, 1);
    fill_texture(&device, &src, 0, 8, 8, &pattern);
    fill_texture(&device, &dst, 0, 8, 8, &solid(8, 8, BLUE));
    let mut cb = device.acquire_command_buffer().unwrap();
    {
        let mut copy = cb.begin_copy_pass().unwrap();
        copy.copy_texture_to_texture(
            &TextureLocation {
                texture: &src,
                mip_level: 0,
                layer: 0,
                x: 2,
                y: 1,
                z: 0,
            },
            &TextureLocation {
                texture: &dst,
                mip_level: 0,
                layer: 0,
                x: 4,
                y: 4,
                z: 0,
            },
            3,
            2,
            1,
            false,
        )
        .unwrap();
    }
    submit_and_wait(&device, cb);
    assert_eq!(texture_pixels(&device, &src, 0, 8, 8), pattern);
    let copied = texture_pixels(&device, &dst, 0, 8, 8);
    for y in 0..8usize {
        for x in 0..8usize {
            let got = &copied[(y * 8 + x) * 4..][..4];
            if (4..7).contains(&x) && (4..6).contains(&y) {
                let (sx, sy) = (x - 4 + 2, y - 4 + 1);
                assert_eq!(got, &pattern[(sy * 8 + sx) * 4..][..4], "({x}, {y})");
            } else {
                assert_eq!(got, BLUE, "({x}, {y})");
            }
        }
    }

    // Uploading twice with cycling in one command buffer: the second
    // upload goes to a new texture, as the first one is in use.
    let upload = upload_buffer(&device, &solid(8, 8, RED));
    let mut cb = device.acquire_command_buffer().unwrap();
    {
        let mut copy = cb.begin_copy_pass().unwrap();
        for _ in 0..2 {
            copy.upload_to_texture(
                &TextureTransferInfo {
                    transfer_buffer: &upload,
                    offset: 0,
                    pixels_per_row: 8,
                    rows_per_layer: 8,
                },
                &TextureRegion {
                    texture: &dst,
                    mip_level: 0,
                    layer: 0,
                    x: 0,
                    y: 0,
                    z: 0,
                    w: 8,
                    h: 8,
                    d: 1,
                },
                true,
            );
        }
    }
    submit_and_wait(&device, cb);
    assert_eq!(lock(&container_of(&dst).state).textures.len(), 2);
    assert_eq!(texture_pixels(&device, &dst, 0, 8, 8), solid(8, 8, RED));
    device.wait_for_idle().unwrap();
}

/// The shaders and pipeline of the draw tests: `vec2` positions plus a
/// vertex uniform offset, a solid red fragment shader, an RGBA8 target.
fn solid_pipeline(device: &Device) -> crate::gpu::GraphicsPipeline {
    let vertex = device
        .create_shader(&ShaderCreateInfo {
            code: &bytes(test_spirv::VERTEX_INPUT_UNIFORM),
            entrypoint: "main",
            format: ShaderFormat::SPIRV,
            stage: ShaderStage::Vertex,
            num_uniform_buffers: 1,
            ..Default::default()
        })
        .unwrap();
    let fragment = device
        .create_shader(&ShaderCreateInfo {
            code: &bytes(test_spirv::FRAGMENT_SOLID),
            entrypoint: "main",
            format: ShaderFormat::SPIRV,
            stage: ShaderStage::Fragment,
            ..Default::default()
        })
        .unwrap();
    position_pipeline(device, &vertex, &fragment)
}

/// A pipeline drawing `vec2` positions from vertex buffer slot 0 into an
/// RGBA8 target.
fn position_pipeline(
    device: &Device,
    vertex: &crate::gpu::Shader,
    fragment: &crate::gpu::Shader,
) -> crate::gpu::GraphicsPipeline {
    let color_target = [ColorTargetDescription {
        format: TextureFormat::R8G8B8A8_UNORM,
        blend_state: Default::default(),
    }];
    let vertex_buffers = [VertexBufferDescription {
        slot: 0,
        pitch: 8,
        input_rate: VertexInputRate::Vertex,
        instance_step_rate: 0,
    }];
    let vertex_attributes = [VertexAttribute {
        location: 0,
        buffer_slot: 0,
        format: VertexElementFormat::Float2,
        offset: 0,
    }];
    device
        .create_graphics_pipeline(&GraphicsPipelineCreateInfo {
            vertex_shader: vertex,
            fragment_shader: fragment,
            vertex_input_state: VertexInputState {
                vertex_buffer_descriptions: &vertex_buffers,
                vertex_attributes: &vertex_attributes,
            },
            primitive_type: PrimitiveType::TriangleList,
            rasterizer_state: Default::default(),
            multisample_state: Default::default(),
            depth_stencil_state: Default::default(),
            target_info: GraphicsPipelineTargetInfo {
                color_target_descriptions: &color_target,
                depth_stencil_format: TextureFormat::D16_UNORM,
                has_depth_stencil_target: false,
            },
            props: None,
        })
        .unwrap()
}

/// The top-left quadrant of clip space (+y is up) as two triangles.
const QUAD_TOP_LEFT: [f32; 12] = [
    -1.0, 0.0, 0.0, 0.0, -1.0, 1.0, //
    -1.0, 1.0, 0.0, 0.0, 0.0, 1.0,
];

/// Which quadrants of an 8x8 image are red (top-left, top-right,
/// bottom-left, bottom-right); the rest is blue.
fn quadrants(red: [bool; 4]) -> Vec<u8> {
    let mut pixels = Vec::new();
    for y in 0..8 {
        for x in 0..8 {
            let quadrant = (y >= 4) as usize * 2 + (x >= 4) as usize;
            pixels.extend(if red[quadrant] { RED } else { BLUE });
        }
    }
    pixels
}

#[test]
fn draw_triangles_with_a_vertex_buffer_and_uniforms() {
    let _l = crate::test_support::test_lock();
    let _v = Video::init();
    let Some(device) = front_end_device() else {
        return;
    };
    use crate::gpu::BufferUsageFlags as B;
    use TextureUsageFlags as U;

    let pipeline = solid_pipeline(&device);
    let vertices = gpu_buffer(&device, B::VERTEX, 48);
    fill_buffer(&device, &vertices, &float_bytes(&QUAD_TOP_LEFT));
    let target = rgba_texture(&device, 8, 8, U::COLOR_TARGET | U::SAMPLER, 1);

    // Two draws of the quad, moved by the vertex uniform: the top-left
    // quadrant (the viewport is flipped, so +y is up) and the bottom-right
    // one.
    let mut cb = device.acquire_command_buffer().unwrap();
    {
        let mut pass = cb
            .begin_render_pass(&[clear_target(&target, BLUE_F)], None)
            .unwrap();
        pass.bind_graphics_pipeline(&pipeline);
        pass.bind_vertex_buffers(
            0,
            &[BufferBinding {
                buffer: &vertices,
                offset: 0,
            }],
        );
        pass.command_buffer()
            .push_vertex_uniform_data(0, &float_bytes(&[0.0; 4]))
            .unwrap();
        pass.draw_primitives(6, 1, 0, 0).unwrap();
        pass.command_buffer()
            .push_vertex_uniform_data(0, &float_bytes(&[1.0, -1.0, 0.0, 0.0]))
            .unwrap();
        pass.draw_primitives(6, 1, 0, 0).unwrap();
    }
    submit_and_wait(&device, cb);
    assert_eq!(
        texture_pixels(&device, &target, 0, 8, 8),
        quadrants([true, false, false, true])
    );

    // Many draws in one command buffer move to another uniform buffer.
    let mut cb = device.acquire_command_buffer().unwrap();
    {
        let mut pass = cb
            .begin_render_pass(&[clear_target(&target, BLUE_F)], None)
            .unwrap();
        pass.bind_graphics_pipeline(&pipeline);
        pass.bind_vertex_buffers(
            0,
            &[BufferBinding {
                buffer: &vertices,
                offset: 0,
            }],
        );
        let min_alignment = device_renderer(&device).min_ubo_alignment.max(16) as usize;
        let draws = 32768 / min_alignment + 4;
        for i in 0..draws {
            // (only the last draw is on screen: top-right)
            let x = if i == draws - 1 { 1.0 } else { 4.0 };
            pass.command_buffer()
                .push_vertex_uniform_data(0, &float_bytes(&[x, 0.0, 0.0, 0.0]))
                .unwrap();
            pass.draw_primitives(6, 1, 0, 0).unwrap();
        }
    }
    submit_and_wait(&device, cb);
    assert_eq!(
        texture_pixels(&device, &target, 0, 8, 8),
        quadrants([false, true, false, false])
    );
    device.wait_for_idle().unwrap();
}

#[test]
fn indexed_and_indirect_draws() {
    let _l = crate::test_support::test_lock();
    let _v = Video::init();
    let Some(device) = front_end_device() else {
        return;
    };
    use crate::gpu::BufferUsageFlags as B;
    use TextureUsageFlags as U;

    let pipeline = solid_pipeline(&device);
    // The quad as four corners and six 16-bit indices, and as six vertices.
    let corners = gpu_buffer(&device, B::VERTEX, 32);
    fill_buffer(
        &device,
        &corners,
        &float_bytes(&[-1.0, 0.0, 0.0, 0.0, -1.0, 1.0, 0.0, 1.0]),
    );
    let indices = gpu_buffer(&device, B::INDEX, 12);
    let index_data: Vec<u8> = [0u16, 1, 2, 2, 1, 3]
        .iter()
        .flat_map(|i| i.to_ne_bytes())
        .collect();
    fill_buffer(&device, &indices, &index_data);
    let vertices = gpu_buffer(&device, B::VERTEX, 48);
    fill_buffer(&device, &vertices, &float_bytes(&QUAD_TOP_LEFT));
    // An indirect draw (after 4 bytes of padding) and an indexed one.
    let indirect = gpu_buffer(&device, B::INDIRECT, 4 + 16 + 20);
    let indirect_data = word_bytes(&[
        0xDEAD, // (padding)
        6, 1, 0, 0, // IndirectDrawCommand
        6, 1, 0, 0, 0, // IndexedIndirectDrawCommand
    ]);
    fill_buffer(&device, &indirect, &indirect_data);
    let target = rgba_texture(&device, 8, 8, U::COLOR_TARGET | U::SAMPLER, 1);

    let mut cb = device.acquire_command_buffer().unwrap();
    {
        let mut pass = cb
            .begin_render_pass(&[clear_target(&target, BLUE_F)], None)
            .unwrap();
        pass.bind_graphics_pipeline(&pipeline);

        // Indexed: the top-left quadrant.
        pass.bind_vertex_buffers(
            0,
            &[BufferBinding {
                buffer: &corners,
                offset: 0,
            }],
        );
        pass.bind_index_buffer(
            &BufferBinding {
                buffer: &indices,
                offset: 0,
            },
            IndexElementSize::Bits16,
        );
        pass.command_buffer()
            .push_vertex_uniform_data(0, &float_bytes(&[0.0; 4]))
            .unwrap();
        pass.draw_indexed_primitives(6, 1, 0, 0, 0).unwrap();

        // Indexed indirect: the top-right quadrant.
        pass.command_buffer()
            .push_vertex_uniform_data(0, &float_bytes(&[1.0, 0.0, 0.0, 0.0]))
            .unwrap();
        pass.draw_indexed_primitives_indirect(&indirect, 20, 1)
            .unwrap();

        // Indirect: the bottom-right quadrant.
        pass.bind_vertex_buffers(
            0,
            &[BufferBinding {
                buffer: &vertices,
                offset: 0,
            }],
        );
        pass.command_buffer()
            .push_vertex_uniform_data(0, &float_bytes(&[1.0, -1.0, 0.0, 0.0]))
            .unwrap();
        pass.draw_primitives_indirect(&indirect, 4, 1).unwrap();
    }
    submit_and_wait(&device, cb);
    assert_eq!(
        texture_pixels(&device, &target, 0, 8, 8),
        quadrants([true, true, false, true])
    );
    device.wait_for_idle().unwrap();
}

#[test]
fn draw_with_a_sampler_and_fragment_uniforms() {
    let _l = crate::test_support::test_lock();
    let _v = Video::init();
    let Some(device) = front_end_device() else {
        return;
    };
    use crate::gpu::BufferUsageFlags as B;
    use TextureUsageFlags as U;

    let vertex = device
        .create_shader(&ShaderCreateInfo {
            code: &bytes(test_spirv::VERTEX_INPUT_UNIFORM),
            entrypoint: "main",
            format: ShaderFormat::SPIRV,
            stage: ShaderStage::Vertex,
            num_uniform_buffers: 1,
            ..Default::default()
        })
        .unwrap();
    let fragment = device
        .create_shader(&ShaderCreateInfo {
            code: &bytes(test_spirv::FRAGMENT_SAMPLED_UNIFORM),
            entrypoint: "main",
            format: ShaderFormat::SPIRV,
            stage: ShaderStage::Fragment,
            num_samplers: 1,
            num_uniform_buffers: 1,
            ..Default::default()
        })
        .unwrap();
    let pipeline = position_pipeline(&device, &vertex, &fragment);
    drop((vertex, fragment));

    // A triangle over the whole target.
    let vertices = gpu_buffer(&device, B::VERTEX, 24);
    fill_buffer(
        &device,
        &vertices,
        &float_bytes(&[-1.0, -1.0, 3.0, -1.0, -1.0, 3.0]),
    );
    let source = rgba_texture(&device, 2, 2, U::SAMPLER, 1);
    fill_texture(&device, &source, 0, 2, 2, &solid(2, 2, [0, 255, 0, 255]));
    let sampler = device
        .create_sampler(&SamplerCreateInfo::default())
        .unwrap();
    let target = rgba_texture(&device, 4, 4, U::COLOR_TARGET | U::SAMPLER, 1);

    let mut cb = device.acquire_command_buffer().unwrap();
    {
        let mut pass = cb
            .begin_render_pass(&[clear_target(&target, BLUE_F)], None)
            .unwrap();
        pass.bind_graphics_pipeline(&pipeline);
        pass.bind_vertex_buffers(
            0,
            &[BufferBinding {
                buffer: &vertices,
                offset: 0,
            }],
        );
        pass.bind_fragment_samplers(
            0,
            &[TextureSamplerBinding {
                texture: &source,
                sampler: &sampler,
            }],
        )
        .unwrap();
        pass.command_buffer()
            .push_vertex_uniform_data(0, &float_bytes(&[0.0; 4]))
            .unwrap();
        // (the texture times the tint)
        pass.command_buffer()
            .push_fragment_uniform_data(0, &float_bytes(&[1.0, 0.2, 1.0, 1.0]))
            .unwrap();
        pass.draw_primitives(3, 1, 0, 0).unwrap();
    }
    submit_and_wait(&device, cb);
    assert_eq!(
        texture_pixels(&device, &target, 0, 4, 4),
        solid(4, 4, [0, 51, 0, 255])
    );
    device.wait_for_idle().unwrap();
}

#[test]
fn compute_dispatches_write_a_storage_buffer() {
    let _l = crate::test_support::test_lock();
    let _v = Video::init();
    let Some(device) = front_end_device() else {
        return;
    };
    use crate::gpu::BufferUsageFlags as B;

    let code = bytes(test_spirv::COMPUTE_INDEX);
    let pipeline = device
        .create_compute_pipeline(&ComputePipelineCreateInfo {
            code: &code,
            entrypoint: "main",
            format: ShaderFormat::SPIRV,
            num_readwrite_storage_buffers: 1,
            num_uniform_buffers: 1,
            threadcount_x: 1,
            threadcount_y: 1,
            threadcount_z: 1,
            ..Default::default()
        })
        .unwrap();
    let storage = gpu_buffer(&device, B::COMPUTE_STORAGE_WRITE, 64);
    let indirect = gpu_buffer(&device, B::INDIRECT, 12);
    fill_buffer(&device, &indirect, &word_bytes(&[8, 1, 1]));

    // 16 invocations write i + 100, then an indirect dispatch of 8 writes
    // i + 1000 over the first ones.
    let mut cb = device.acquire_command_buffer().unwrap();
    {
        let mut pass = cb
            .begin_compute_pass(
                &[],
                &[StorageBufferReadWriteBinding {
                    buffer: &storage,
                    cycle: false,
                }],
            )
            .unwrap();
        pass.bind_compute_pipeline(&pipeline);
        pass.command_buffer()
            .push_compute_uniform_data(0, &word_bytes(&[100]))
            .unwrap();
        pass.dispatch(16, 1, 1).unwrap();
    }
    {
        let mut pass = cb
            .begin_compute_pass(
                &[],
                &[StorageBufferReadWriteBinding {
                    buffer: &storage,
                    cycle: false,
                }],
            )
            .unwrap();
        pass.bind_compute_pipeline(&pipeline);
        pass.command_buffer()
            .push_compute_uniform_data(0, &word_bytes(&[1000]))
            .unwrap();
        pass.dispatch_indirect(&indirect, 0).unwrap();
    }
    submit_and_wait(&device, cb);
    let expected: Vec<u32> = (0..16)
        .map(|i| if i < 8 { i + 1000 } else { i + 100 })
        .collect();
    assert_eq!(
        buffer_contents(&device, &storage, 64),
        word_bytes(&expected)
    );

    // A cycled storage buffer is a new one (the old one is in use by the
    // recorded command buffer): only the second pass's writes are there.
    let mut cb = device.acquire_command_buffer().unwrap();
    for (add, groups) in [(5, 16), (7, 4)] {
        let mut pass = cb
            .begin_compute_pass(
                &[],
                &[StorageBufferReadWriteBinding {
                    buffer: &storage,
                    cycle: true,
                }],
            )
            .unwrap();
        pass.bind_compute_pipeline(&pipeline);
        pass.command_buffer()
            .push_compute_uniform_data(0, &word_bytes(&[add]))
            .unwrap();
        pass.dispatch(groups, 1, 1).unwrap();
    }
    submit_and_wait(&device, cb);
    let contents = buffer_contents(&device, &storage, 64);
    assert_eq!(&contents[..16], &word_bytes(&[7, 8, 9, 10])[..]);
    let storage_container = storage.raw.downcast::<BufferContainer>().unwrap();
    assert_eq!(lock(&storage_container.state).buffers.len(), 2);
    device.wait_for_idle().unwrap();
}

#[test]
fn blits_and_mipmaps() {
    let _l = crate::test_support::test_lock();
    let _v = Video::init();
    let Some(device) = front_end_device() else {
        return;
    };
    use TextureUsageFlags as U;

    // Mipmaps of a solid texture are that color down to 1x1.
    let mipmapped = rgba_texture(&device, 8, 8, U::SAMPLER | U::COLOR_TARGET, 4);
    let color = [200, 100, 50, 255];
    fill_texture(&device, &mipmapped, 0, 8, 8, &solid(8, 8, color));
    let mut cb = device.acquire_command_buffer().unwrap();
    cb.generate_mipmaps(&mipmapped).unwrap();
    submit_and_wait(&device, cb);
    for level in 1..4 {
        let size = 8 >> level;
        assert_eq!(
            texture_pixels(&device, &mipmapped, level, size, size),
            solid(size, size, color),
            "level {level}"
        );
    }

    // A horizontally flipped blit with the nearest filter: the left (red)
    // and right (blue) halves trade places.
    let mut halves = Vec::new();
    for _ in 0..8 {
        for x in 0..8 {
            halves.extend(if x < 4 { RED } else { BLUE });
        }
    }
    let source = rgba_texture(&device, 8, 8, U::SAMPLER, 1);
    fill_texture(&device, &source, 0, 8, 8, &halves);
    let destination = rgba_texture(&device, 8, 8, U::SAMPLER | U::COLOR_TARGET, 1);
    let region = |texture| crate::gpu::BlitRegion {
        texture,
        mip_level: 0,
        layer_or_depth_plane: 0,
        x: 0,
        y: 0,
        w: 8,
        h: 8,
    };
    let mut cb = device.acquire_command_buffer().unwrap();
    cb.blit_texture(&BlitInfo {
        source: region(&source),
        destination: region(&destination),
        load_op: crate::gpu::LoadOp::DontCare,
        clear_color: FColor::default(),
        flip_mode: crate::video::FlipMode::Horizontal,
        filter: Filter::Nearest,
        cycle: false,
    })
    .unwrap();
    submit_and_wait(&device, cb);
    let flipped = texture_pixels(&device, &destination, 0, 8, 8);
    for (i, pixel) in flipped.chunks(4).enumerate() {
        let x = i % 8;
        assert_eq!(pixel, if x < 4 { BLUE } else { RED }, "pixel {i}");
    }

    // A blit into a corner, after clearing the destination (to black).
    let mut cb = device.acquire_command_buffer().unwrap();
    cb.blit_texture(&BlitInfo {
        source: region(&source),
        destination: crate::gpu::BlitRegion {
            w: 4,
            h: 4,
            ..region(&destination)
        },
        load_op: crate::gpu::LoadOp::Clear,
        clear_color: FColor {
            r: 0.0,
            g: 0.0,
            b: 0.0,
            a: 1.0,
        },
        flip_mode: crate::video::FlipMode::None,
        filter: Filter::Nearest,
        cycle: false,
    })
    .unwrap();
    submit_and_wait(&device, cb);
    let corner = texture_pixels(&device, &destination, 0, 8, 8);
    for (i, pixel) in corner.chunks(4).enumerate() {
        let (x, y) = (i % 8, i / 8);
        let expected = if x < 4 && y < 4 {
            // (the source's 8x8 scaled to 4x4: red, then blue)
            if x < 2 {
                RED
            } else {
                BLUE
            }
        } else {
            [0, 0, 0, 255]
        };
        assert_eq!(pixel, expected, "({x}, {y})");
    }
    device.wait_for_idle().unwrap();
}

#[test]
fn fences() {
    let _l = crate::test_support::test_lock();
    let _v = Video::init();
    let Some(device) = front_end_device() else {
        return;
    };

    let first = device
        .acquire_command_buffer()
        .unwrap()
        .submit_and_acquire_fence()
        .unwrap();
    let second = device
        .acquire_command_buffer()
        .unwrap()
        .submit_and_acquire_fence()
        .unwrap();
    device.wait_for_fences(false, &[&first, &second]).unwrap();
    device.wait_for_fences(true, &[&first, &second]).unwrap();
    assert!(first.is_signaled());
    assert!(second.is_signaled());

    // Once the command buffers are cleaned, the fences are only the
    // application's.
    let renderer = device_renderer(&device);
    assert!(lock(&renderer.submit_lock).is_empty());
    let handle = first.raw.downcast::<VulkanFenceHandle>().unwrap();
    assert_eq!(handle.reference_count.load(Ordering::SeqCst), 1);

    // A released fence goes back to the pool, and is reset when reused.
    drop(first);
    assert_eq!(lock(&renderer.fence_pool).len(), 1);
    let third = device
        .acquire_command_buffer()
        .unwrap()
        .submit_and_acquire_fence()
        .unwrap();
    assert!(Arc::ptr_eq(
        &third.raw.downcast::<VulkanFenceHandle>().unwrap(),
        &handle
    ));
    assert!(lock(&renderer.fence_pool).is_empty());
    device.wait_for_fences(true, &[&third]).unwrap();
    assert!(third.is_signaled());

    // A plain submission's fence goes back to the pool when it's cleaned.
    device.acquire_command_buffer().unwrap().submit().unwrap();
    device.wait_for_idle().unwrap();
    assert_eq!(lock(&renderer.fence_pool).len(), 1);
    drop((second, third));
    assert_eq!(lock(&renderer.fence_pool).len(), 3);

    // The command buffers went back to this thread's pool.
    let pools = lock(&renderer.command_pools);
    let pool = &pools[&std::thread::current().id()];
    assert!(!pool.inactive_command_buffers.is_empty());
}

#[test]
fn submission_frees_empty_allocations() {
    let _l = crate::test_support::test_lock();
    let _v = Video::init();
    let Some(r) = renderer() else {
        return;
    };
    use crate::gpu::BufferUsageFlags as B;

    let count = |r: &VulkanRenderer| -> usize {
        let a = r.memory_allocator.lock();
        let a = a.borrow();
        (0..VK_MAX_MEMORY_TYPES)
            .map(|t| a.allocation_count(t))
            .sum()
    };
    let before = count(&r);
    let large = r
        .create_buffer_container(65 << 20, B::INDEX, VulkanBufferType::Gpu, false, None)
        .unwrap();
    assert_eq!(count(&r), before + 1);
    r.release_buffer_container(&large);
    r.wait_internal().unwrap();
    assert!(r.memory_allocator.lock().borrow().check_empty_allocations);
    assert_eq!(count(&r), before + 1);

    // (no claimed window: every submission cleans up)
    let command_buffer = r.acquire_command_buffer_internal().unwrap();
    r.submit_internal(command_buffer).unwrap();
    assert_eq!(count(&r), before);
    assert!(!r.memory_allocator.lock().borrow().check_empty_allocations);
    r.wait_internal().unwrap();
}

/// The video drivers whose windows the swapchain test presents to: the
/// offscreen driver's headless surfaces, and X11's when there is a
/// display (Windows' on Windows).
fn swapchain_video_drivers() -> Vec<&'static str> {
    if cfg!(windows) {
        vec!["windows"]
    } else {
        let mut drivers = vec!["offscreen"];
        if std::env::var_os("DISPLAY").is_some() {
            drivers.push("x11");
        }
        drivers
    }
}

/// Video up on a driver, or `None` (with the skip reported); quit when
/// dropped.
struct VideoOn;

impl VideoOn {
    fn init(driver: &str) -> Option<VideoOn> {
        init::quit();
        hints::set(hints::VIDEO_DRIVER, driver).unwrap();
        if let Err(e) = init::init(InitFlags::VIDEO) {
            let capability = match driver {
                "x11" => "xvfb",
                "windows" => "desktop",
                _ => "vulkan",
            };
            crate::test_support::skip(
                capability,
                format_args!("no {driver} video ({})", e.message()),
            );
            hints::reset(hints::VIDEO_DRIVER);
            return None;
        }
        Some(VideoOn)
    }
}

impl Drop for VideoOn {
    fn drop(&mut self) {
        init::quit();
        hints::reset(hints::VIDEO_DRIVER);
    }
}

#[test]
fn swapchains_on_windows() {
    let _l = crate::test_support::test_lock();
    for driver in swapchain_video_drivers() {
        let Some(_video) = VideoOn::init(driver) else {
            continue;
        };
        swapchain_frames(driver);
    }
}

/// Claim a window of the current video driver, present frames to it
/// (with the swapchain remade for its parameters and a resize), and
/// release it.
fn swapchain_frames(driver: &str) {
    let Some(device) = front_end_device() else {
        return;
    };
    let window =
        Window::create("gpu", 64, 48, crate::events::window::WindowFlags::default()).unwrap();
    let _ = window.show();
    let _ = window.sync();

    // Not claimed yet
    assert!(!device.window_supports_present_mode(&window, PresentMode::Vsync));
    assert!(device.swapchain_texture_format(&window).is_err());
    {
        let mut cb = device.acquire_command_buffer().unwrap();
        let e = cb.acquire_swapchain_texture(&window).unwrap_err();
        assert_eq!(
            e.message(),
            "Cannot acquire a swapchain texture from an unclaimed window!"
        );
        cb.cancel().unwrap();
    }

    if let Err(e) = device.claim_window(&window) {
        crate::test_support::skip(
            "vulkan",
            format_args!("no Vulkan surface for a {driver} window ({})", e.message()),
        );
        return;
    }
    assert!(device.window_supports_swapchain_composition(&window, SwapchainComposition::Sdr));
    assert!(device.window_supports_present_mode(&window, PresentMode::Vsync));
    let format = device.swapchain_texture_format(&window).unwrap();
    assert!(
        format == TextureFormat::B8G8R8A8_UNORM || format == TextureFormat::R8G8B8A8_UNORM,
        "{format:?}"
    );

    // A second claim is counted, and so is its release.
    device.claim_window(&window).unwrap();
    device.release_window(&window);
    assert_eq!(lock(&device_renderer(&device).claimed_windows).len(), 1);

    // Several frames: acquire, clear, present.
    let frame = |device: &Device, block: bool| -> Option<(u32, u32)> {
        let mut cb = device.acquire_command_buffer().unwrap();
        let texture = if block {
            cb.wait_and_acquire_swapchain_texture(&window).unwrap()
        } else {
            cb.acquire_swapchain_texture(&window).unwrap()
        };
        let size = texture.as_ref().map(|t| (t.width, t.height));
        if let Some(texture) = &texture {
            assert_eq!(texture.texture.info.format, format);
            cb.begin_render_pass(&[clear_target(&texture.texture, BLUE_F)], None)
                .unwrap()
                .end();
        }
        cb.submit().unwrap();
        size
    };
    for _ in 0..5 {
        assert_eq!(frame(&device, true), Some((64, 48)));
    }
    device.wait_for_swapchain(&window).unwrap();
    // (without blocking a frame can be skipped, which isn't an error)
    for _ in 0..3 {
        if let Some(size) = frame(&device, false) {
            assert_eq!(size, (64, 48));
        }
    }

    // Frames in flight and the swapchain parameters remake the swapchain.
    device.set_allowed_frames_in_flight(3).unwrap();
    for _ in 0..4 {
        assert_eq!(frame(&device, true), Some((64, 48)));
    }
    device.set_allowed_frames_in_flight(1).unwrap();
    assert_eq!(frame(&device, true), Some((64, 48)));
    device
        .set_swapchain_parameters(&window, SwapchainComposition::Sdr, PresentMode::Vsync)
        .unwrap();
    assert_eq!(frame(&device, true), Some((64, 48)));
    device.set_allowed_frames_in_flight(2).unwrap();

    // A resize remakes the swapchain at the new size.
    window.set_size(40, 30).unwrap();
    let _ = window.sync();
    for _ in 0..3 {
        assert_eq!(frame(&device, true), Some((40, 30)));
    }

    // Released: the window can't present any more (and can be claimed
    // again).
    device.release_window(&window);
    assert!(lock(&device_renderer(&device).claimed_windows).is_empty());
    assert!(device.swapchain_texture_format(&window).is_err());
    device.claim_window(&window).unwrap();
    assert_eq!(frame(&device, true), Some((40, 30)));
    // (the device releases the windows it still has when it goes)
    drop(device);
    window.destroy();
}
