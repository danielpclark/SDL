// sdl3-shadercross on the GPU API's Metal backend.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! The library's `gpu_shaders_and_pipelines_from_spirv` test, on Metal:
//! the Metal backend only comes up with a video driver that makes Metal
//! views (upstream's `METAL_PrepareDriver()`), which on macOS is the Cocoa
//! driver, and that only starts on the main thread. So this is a program of
//! its own (`harness = false`): every test shader (testdata/*.spv, the bad
//! ones left out) goes through MSL into a Metal shader or compute pipeline.
//! Without a window server session or a Metal device it skips, unless
//! `SDL3_TEST_REQUIRE` asks for `cocoa` or `metal`.

#[cfg(not(target_os = "macos"))]
fn main() {}

#[cfg(target_os = "macos")]
fn main() {
    use sdl3_shadercross as sc;

    let skip = |capability: &str, reason: &str| {
        let list = std::env::var("SDL3_TEST_REQUIRE").unwrap_or_default();
        let required = list
            .split(',')
            .map(str::trim)
            .any(|c| c == "all" || c == capability);
        assert!(
            !required,
            "required capability {capability} unavailable: {reason}"
        );
        println!("note: skipping, {capability} unavailable: {reason}");
    };

    sdl3::hints::set(sdl3::hints::VIDEO_DRIVER, "cocoa").unwrap();
    if let Err(e) = sdl3::init::init(sdl3::init::InitFlags::VIDEO) {
        skip("cocoa", e.message());
        return;
    }
    sc::init().unwrap();
    let device = match sdl3::gpu::Device::new(sc::get_spirv_shader_formats(), true, None) {
        Ok(device) => device,
        Err(e) => {
            skip("metal", &format!("no GPU device ({})", e.message()));
            return;
        }
    };
    assert_eq!(device.driver(), "metal");
    assert!(device
        .shader_formats()
        .contains(sdl3::gpu::ShaderFormat::MSL));

    // The test shaders of the library's tests (SHADERS), the bad ones left out.
    let names = [
        "cs_basic",
        "cs_basic_opt",
        "cs_image",
        "cs_layout",
        "cs_math",
        "fs_complex",
        "fs_complex_opt",
        "fs_sampling",
        "fs_storage",
        "fs_textured",
        "vs_basic",
        "vs_push",
        "vs_storage",
    ];
    let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/testdata");
    for name in names {
        let code = std::fs::read(format!("{dir}/{name}.spv")).unwrap();
        let shader_stage = match name.as_bytes()[0] {
            b'v' => sc::ShaderStage::Vertex,
            b'f' => sc::ShaderStage::Fragment,
            _ => sc::ShaderStage::Compute,
        };
        let info = sc::SpirvInfo {
            bytecode: &code,
            entrypoint: "main",
            shader_stage,
            props: None,
        };
        if shader_stage == sc::ShaderStage::Compute {
            if name == "cs_image" {
                // Note (upstream): before MSL 3.1 SPIRV-Cross emulates
                // cs_image's image atomic with a buffer, and the emulation
                // declares a function constant
                // (spvLinearTextureAlignmentOverride). SDL's Metal backend
                // makes the function with newFunctionWithName: and gives no
                // constant values, so Metal's validation aborts on making
                // its pipeline (and SDL doesn't bind the buffer either). Its
                // MSL, with that buffer after the others, is checked against
                // testdata/reference.txt by src/tests.rs.
                continue;
            }
            let metadata = sc::reflect_compute_spirv(&code, None).unwrap();
            sc::compile_compute_pipeline_from_spirv(&device, &info, &metadata, None)
                .unwrap_or_else(|e| panic!("{name}: {}", e.message()));
        } else {
            let metadata = sc::reflect_graphics_spirv(&code, None).unwrap();
            sc::compile_graphics_shader_from_spirv(&device, &info, &metadata.resource_info, None)
                .unwrap_or_else(|e| panic!("{name}: {}", e.message()));
        }
    }
    drop(device);
    sc::quit();
    sdl3::init::quit();
    println!(
        "metal: {} shaders and pipelines from SPIR-V ok (cs_image skipped)",
        names.len() - 1
    );
}
