// Tests: the translation's output, byte for byte against SDL_shadercross
// and SPIRV-Cross run on the same SPIR-V (testdata/reference.txt; see
// tools/gen_shadercross_testdata.py).

use crate::spirv_cross::c_api::*;

/// The test shaders (testdata/*.spv), by name.
pub(crate) const SHADERS: &[(&str, &[u8])] = &[
    ("cs_bad_flip", include_bytes!("../testdata/cs_bad_flip.spv")),
    (
        "cs_bad_flip2",
        include_bytes!("../testdata/cs_bad_flip2.spv"),
    ),
    (
        "cs_bad_flip3",
        include_bytes!("../testdata/cs_bad_flip3.spv"),
    ),
    (
        "cs_bad_long_count",
        include_bytes!("../testdata/cs_bad_long_count.spv"),
    ),
    (
        "cs_bad_magic",
        include_bytes!("../testdata/cs_bad_magic.spv"),
    ),
    ("cs_basic", include_bytes!("../testdata/cs_basic.spv")),
    (
        "cs_basic_opt",
        include_bytes!("../testdata/cs_basic_opt.spv"),
    ),
    ("cs_image", include_bytes!("../testdata/cs_image.spv")),
    ("cs_layout", include_bytes!("../testdata/cs_layout.spv")),
    ("cs_math", include_bytes!("../testdata/cs_math.spv")),
    ("fs_bad_flip", include_bytes!("../testdata/fs_bad_flip.spv")),
    (
        "fs_bad_header",
        include_bytes!("../testdata/fs_bad_header.spv"),
    ),
    (
        "fs_bad_zero_count",
        include_bytes!("../testdata/fs_bad_zero_count.spv"),
    ),
    ("fs_complex", include_bytes!("../testdata/fs_complex.spv")),
    (
        "fs_complex_opt",
        include_bytes!("../testdata/fs_complex_opt.spv"),
    ),
    ("fs_sampling", include_bytes!("../testdata/fs_sampling.spv")),
    ("fs_storage", include_bytes!("../testdata/fs_storage.spv")),
    ("fs_textured", include_bytes!("../testdata/fs_textured.spv")),
    (
        "vs_bad_truncated",
        include_bytes!("../testdata/vs_bad_truncated.spv"),
    ),
    (
        "vs_bad_version",
        include_bytes!("../testdata/vs_bad_version.spv"),
    ),
    ("vs_basic", include_bytes!("../testdata/vs_basic.spv")),
    ("vs_push", include_bytes!("../testdata/vs_push.spv")),
    ("vs_storage", include_bytes!("../testdata/vs_storage.spv")),
];

/// Malformed shaders on which SDL_shadercross's SPIRV-Cross reads out of
/// bounds (crashing, or failing depending on the heap); not in the
/// reference.
const UB_SHADERS: &[(&str, &[u8])] = &[
    ("cs_ub_flip", include_bytes!("../testdata/cs_ub_flip.spv")),
    ("fs_ub_flip", include_bytes!("../testdata/fs_ub_flip.spv")),
    (
        "vs_ub_entry_id",
        include_bytes!("../testdata/vs_ub_entry_id.spv"),
    ),
];

const REFERENCE: &str = include_str!("../testdata/reference.txt");

/// The sections of the reference file: (header words, text).
pub(crate) fn reference_sections() -> Vec<(Vec<&'static str>, &'static str)> {
    let mut out = Vec::new();
    let mut rest = REFERENCE;
    while let Some(start) = rest.find("=== ") {
        let after = &rest[start + 4..];
        let eol = after.find('\n').unwrap();
        let header = after[..eol].split(' ').collect();
        let body = &after[eol + 1..];
        let end = body.find("\n=== ").map(|e| e + 1).unwrap_or(body.len());
        out.push((header, &body[..end]));
        rest = &body[end..];
    }
    out
}

pub(crate) fn words(code: &[u8]) -> Vec<u32> {
    code.chunks_exact(4)
        .map(|c| u32::from_le_bytes([c[0], c[1], c[2], c[3]]))
        .collect()
}

fn shader(name: &str) -> &'static [u8] {
    SHADERS.iter().find(|s| s.0 == name).unwrap().1
}

/// SPIRV-Cross's GLSL backend, as the harness runs it through the C API.
fn glsl(code: &[u8], version: u32, es: bool, vulkan: bool) -> String {
    let mut context = SpvcContext::create();
    let r = (|| {
        let ir = context.parse_spirv(&words(code))?;
        let mut compiler = context.create_compiler(SpvcBackend::Glsl, ir)?;
        let mut options = compiler.create_compiler_options();
        for (o, v) in [
            (SPVC_COMPILER_OPTION_GLSL_VERSION, version),
            (SPVC_COMPILER_OPTION_GLSL_ES, es as u32),
            (SPVC_COMPILER_OPTION_GLSL_VULKAN_SEMANTICS, vulkan as u32),
        ] {
            let r = options.set_uint(&mut context, o, v);
            if r < 0 {
                return Err(r);
            }
        }
        compiler.install_compiler_options(&options);
        compiler.compile(&mut context)
    })();
    match r {
        Ok(s) => s,
        Err(_) => format!("ERROR: {}\n", context.get_last_error_string()),
    }
}

#[test]
fn glsl_matches_spirv_cross() {
    let mut count = 0;
    for (header, expected) in reference_sections() {
        if header[1] != "glsl" {
            continue;
        }
        let version = header[2].parse().unwrap();
        let out = glsl(
            shader(header[0]),
            version,
            header[3] == "1",
            header[4] == "1",
        );
        assert_eq!(out, expected, "=== {}", header.join(" "));
        count += 1;
    }
    assert_eq!(count, SHADERS.len() * 3);
}

fn stage(name: &str) -> crate::ShaderStage {
    match name {
        "vertex" => crate::ShaderStage::Vertex,
        "fragment" => crate::ShaderStage::Fragment,
        _ => crate::ShaderStage::Compute,
    }
}

/// `SDL_ShaderCross_INTERNAL_TranspileFromSPIRV()`, as the harness prints
/// it.
fn transpile(header: &[&str]) -> String {
    let props = sdl3::properties::Properties::new();
    if header[4] != "-" {
        props
            .set(crate::PROP_SPIRV_MSL_VERSION_STRING, header[4])
            .unwrap();
    }
    if header[5] == "1" {
        props
            .set(crate::PROP_SPIRV_PSSL_COMPATIBILITY_BOOLEAN, true)
            .unwrap();
    }
    let backend = if header[1] == "msl" {
        SpvcBackend::Msl
    } else {
        SpvcBackend::Hlsl
    };
    match crate::shadercross::transpile_for_tests(
        backend,
        header[3].parse().unwrap(),
        stage(header[2]),
        shader(header[0]),
        "main",
        Some(&props),
    ) {
        Ok((source, entrypoint)) => {
            format!(
                "entrypoint: {}\n{}",
                entrypoint.as_deref().unwrap_or("(null)"),
                source
            )
        }
        Err(e) => format!("ERROR: {}\n", e.message()),
    }
}

#[test]
fn msl_matches_spirv_cross() {
    let mut count = 0;
    for (header, expected) in reference_sections() {
        if header[1] != "msl" {
            continue;
        }
        let out = transpile(&header);
        assert_eq!(out, expected, "=== {}", header.join(" "));
        count += 1;
    }
    assert_eq!(count, SHADERS.len() * 2);
}

#[test]
fn hlsl_matches_spirv_cross() {
    let mut count = 0;
    for (header, expected) in reference_sections() {
        if header[1] != "hlsl" {
            continue;
        }
        let out = transpile(&header);
        assert_eq!(out, expected, "=== {}", header.join(" "));
        count += 1;
    }
    assert_eq!(count, SHADERS.len() * 3);
}

fn iovar_type(t: crate::IOVarType) -> &'static str {
    use crate::IOVarType::*;
    match t {
        Unknown => "unknown",
        Int8 => "int8",
        UInt8 => "uint8",
        Int16 => "int16",
        UInt16 => "uint16",
        Int32 => "int32",
        UInt32 => "uint32",
        Int64 => "int64",
        UInt64 => "uint64",
        Float16 => "float16",
        Float32 => "float32",
        Float64 => "float64",
    }
}

/// The reflection functions, as the harness prints them.
fn reflect(name: &str) -> String {
    let code = shader(name);
    if name.starts_with('c') {
        match crate::reflect_compute_spirv(code, None) {
            Ok(m) => format!(
                "samplers={} readonly_storage_textures={} readonly_storage_buffers={} readwrite_storage_textures={} \
                 readwrite_storage_buffers={} uniform_buffers={} threadcount={},{},{}\n",
                m.num_samplers,
                m.num_readonly_storage_textures,
                m.num_readonly_storage_buffers,
                m.num_readwrite_storage_textures,
                m.num_readwrite_storage_buffers,
                m.num_uniform_buffers,
                m.threadcount_x,
                m.threadcount_y,
                m.threadcount_z
            ),
            Err(e) => format!("ERROR: {}\n", e.message()),
        }
    } else {
        match crate::reflect_graphics_spirv(code, None) {
            Ok(m) => {
                let r = m.resource_info;
                let mut out = format!(
                    "samplers={} storage_textures={} storage_buffers={} uniform_buffers={}\n",
                    r.num_samplers,
                    r.num_storage_textures,
                    r.num_storage_buffers,
                    r.num_uniform_buffers
                );
                for (kind, vars) in [("input", &m.inputs), ("output", &m.outputs)] {
                    for v in vars {
                        out += &format!(
                            "{kind} {} location={} type={} size={}\n",
                            v.name,
                            v.location,
                            iovar_type(v.vector_type),
                            v.vector_size
                        );
                    }
                }
                out
            }
            Err(e) => format!("ERROR: {}\n", e.message()),
        }
    }
}

#[test]
fn reflection_matches_shadercross() {
    let mut count = 0;
    for (header, expected) in reference_sections() {
        if header[1] != "reflect" {
            continue;
        }
        assert_eq!(reflect(header[0]), expected, "=== {}", header.join(" "));
        count += 1;
    }
    assert_eq!(count, SHADERS.len());
}

#[test]
fn msl_version_strings() {
    // SDL_sscanf("%u.%u.%u") as the version parser reads it.
    let parse = crate::shadercross::parse_version_number_for_tests;
    assert_eq!(parse("1.2.0"), 10200);
    assert_eq!(parse(" 2.1.0"), 20100);
    assert_eq!(parse("3.2.1junk"), 30201);
    assert_eq!(parse("2.1"), -1);
    assert_eq!(parse("2..1"), -1);
    assert_eq!(parse(""), -1);
    let props = sdl3::properties::Properties::new();
    props
        .set(crate::PROP_SPIRV_MSL_VERSION_STRING, "metal")
        .unwrap();
    let info = crate::SpirvInfo {
        bytecode: shader("vs_basic"),
        entrypoint: "main",
        shader_stage: crate::ShaderStage::Vertex,
        props: Some(&props),
    };
    let err = crate::transpile_msl_from_spirv(&info).unwrap_err();
    assert_eq!(
        err.message(),
        "failed to parse MSL version string \"metal\""
    );
}

#[test]
fn malformed_spirv_fails_cleanly() {
    // SDL_shadercross crashes on fs_ub_flip (in SPIRV-Cross's parser) and
    // cs_ub_flip (in its GLSL backend), and fails depending on what the heap
    // holds past its ID array on vs_ub_entry_id; the translation reports the
    // malformed input.
    for (name, code) in UB_SHADERS {
        let compute = name.starts_with('c');
        let vertex = name.starts_with('v');
        let props = sdl3::properties::Properties::new();
        let info = crate::SpirvInfo {
            bytecode: code,
            entrypoint: "main",
            shader_stage: stage(if compute {
                "compute"
            } else if vertex {
                "vertex"
            } else {
                "fragment"
            }),
            props: Some(&props),
        };
        let _ = crate::transpile_msl_from_spirv(&info);
        let _ = crate::transpile_hlsl_from_spirv(&info);
        if compute {
            let _ = crate::reflect_compute_spirv(code, None);
        } else {
            let _ = crate::reflect_graphics_spirv(code, None);
        }
        let out = glsl(code, 450, false, false);
        assert!(out.starts_with("ERROR: "), "{name}: {out}");
    }
    // SPIRV-Cross allocates decorations for 15 million struct members here;
    // the translation reports the impossible member index.
    let code = include_bytes!("../testdata/cs_guard_member_index.spv");
    assert_eq!(
        crate::reflect_compute_spirv(code, None)
            .unwrap_err()
            .message(),
        "spvc_context_parse_spirv failed: Member index is out of range."
    );
    // SDL_shadercross never returns from this one: a block chain that comes
    // back around.
    let code = include_bytes!("../testdata/fs_hang_block_chain.spv");
    let info = crate::SpirvInfo {
        bytecode: code,
        entrypoint: "main",
        shader_stage: crate::ShaderStage::Fragment,
        props: None,
    };
    for r in [
        crate::transpile_msl_from_spirv(&info),
        crate::transpile_hlsl_from_spirv(&info),
    ] {
        assert_eq!(
            r.unwrap_err().message(),
            "spvc_compiler_compile failed: Block chain loops back on itself."
        );
    }
    assert_eq!(
        glsl(code, 450, false, false),
        "ERROR: Block chain loops back on itself.\n"
    );
    // Every truncation of a shader fails, without panicking.
    let code = shader("vs_push");
    for len in 0..code.len() {
        let info = crate::SpirvInfo {
            bytecode: &code[..len],
            entrypoint: "main",
            shader_stage: crate::ShaderStage::Vertex,
            props: None,
        };
        let r = crate::transpile_hlsl_from_spirv(&info);
        assert!(r.is_err(), "truncated to {len} bytes");
    }
}

/// `SDL_VIDEO_DRIVER=offscreen` with the video subsystem up, for a GPU
/// device.
struct OffscreenVideo;

impl OffscreenVideo {
    fn init() -> sdl3::Result<OffscreenVideo> {
        sdl3::hints::set(sdl3::hints::VIDEO_DRIVER, "offscreen")?;
        sdl3::init::init(sdl3::init::InitFlags::VIDEO)?;
        Ok(OffscreenVideo)
    }
}

impl Drop for OffscreenVideo {
    fn drop(&mut self) {
        sdl3::init::quit_subsystem(sdl3::init::InitFlags::VIDEO);
        sdl3::hints::reset(sdl3::hints::VIDEO_DRIVER);
    }
}

/// Whether this is Wine, whose d3dcompiler_47.dll doesn't implement shader
/// model 5.1 (D3DCompile returns E_NOTIMPL), so there's no DXBC.
fn under_wine() -> bool {
    cfg!(windows)
        && sdl3::loadso::SharedObject::load("ntdll.dll")
            .and_then(|ntdll| ntdll.symbol("wine_get_version").map(|_| ()))
            .is_ok()
}

/// The formats shadercross can make here, as far as the system's tools go.
fn usable_formats() -> sdl3::gpu::ShaderFormat {
    let formats = crate::get_spirv_shader_formats();
    if under_wine() {
        sdl3::gpu::ShaderFormat(formats.bits() & !sdl3::gpu::ShaderFormat::DXBC.bits())
    } else {
        formats
    }
}

#[test]
fn gpu_shaders_and_pipelines_from_spirv() {
    // Declared before the device, so dropped after it.
    let video = OffscreenVideo::init();
    crate::init().unwrap();
    let device = match video
        .as_ref()
        .map_err(Clone::clone)
        .and_then(|_| sdl3::gpu::Device::new(usable_formats(), false, None))
    {
        Ok(device) => device,
        Err(e) => {
            // SDL3_TEST_REQUIRE (docs/HARDWARE_TESTING.md) makes a missing
            // GPU a failure.
            let list = std::env::var("SDL3_TEST_REQUIRE").unwrap_or_default();
            let required = list.split(',').any(|c| {
                c == "all"
                    || (cfg!(target_os = "linux") && c == "vulkan")
                    || (cfg!(windows) && c == "d3d12")
            });
            assert!(!required, "no GPU device: {e}");
            eprintln!("skipped: no GPU device ({e})");
            return;
        }
    };
    // A device without SPIR-V or MSL gets its shaders through HLSL. vs_push's
    // std430 push constant block (a mat2 in 16 bytes, then a vec2) has no
    // HLSL cbuffer packing, and upstream's SPIRV-Cross writes the same
    // overlapping packoffsets, which D3DCompile rejects (X4019).
    let formats = device.shader_formats();
    let through_hlsl = !formats.contains(sdl3::gpu::ShaderFormat::SPIRV)
        && !formats.contains(sdl3::gpu::ShaderFormat::MSL);
    let mut count = 0;
    for (name, code) in SHADERS {
        if name.contains("_bad_") || (through_hlsl && *name == "vs_push") {
            continue;
        }
        let shader_stage = stage(match name.as_bytes()[0] {
            b'v' => "vertex",
            b'f' => "fragment",
            _ => "compute",
        });
        let info = crate::SpirvInfo {
            bytecode: code,
            entrypoint: "main",
            shader_stage,
            props: None,
        };
        if shader_stage == crate::ShaderStage::Compute {
            let metadata = crate::reflect_compute_spirv(code, None).unwrap();
            crate::compile_compute_pipeline_from_spirv(&device, &info, &metadata, None)
                .unwrap_or_else(|e| panic!("{name}: {}", e.message()));
        } else {
            let metadata = crate::reflect_graphics_spirv(code, None).unwrap();
            crate::compile_graphics_shader_from_spirv(
                &device,
                &info,
                &metadata.resource_info,
                None,
            )
            .unwrap_or_else(|e| panic!("{name}: {}", e.message()));
        }
        count += 1;
    }
    assert_eq!(count, if through_hlsl { 12 } else { 13 });
}

#[cfg(windows)]
#[test]
fn dxbc_from_spirv() {
    // d3dcompiler_47.dll is a system DLL on Windows (and a builtin one under
    // Wine); without it there's no DXBC.
    crate::init().unwrap();
    if !usable_formats().contains(sdl3::gpu::ShaderFormat::DXBC) {
        assert!(crate::compile_dxbc_from_spirv(&crate::SpirvInfo {
            bytecode: shader("vs_basic"),
            entrypoint: "main",
            shader_stage: crate::ShaderStage::Vertex,
            props: None,
        })
        .is_err());
        eprintln!("skipped: no shader model 5.1 d3dcompiler_47.dll");
        return;
    }
    for name in ["vs_basic", "fs_textured", "cs_basic"] {
        let info = crate::SpirvInfo {
            bytecode: shader(name),
            entrypoint: "main",
            shader_stage: stage(match name.as_bytes()[0] {
                b'v' => "vertex",
                b'f' => "fragment",
                _ => "compute",
            }),
            props: None,
        };
        let dxbc = crate::compile_dxbc_from_spirv(&info)
            .unwrap_or_else(|e| panic!("{name}: {}", e.message()));
        assert_eq!(&dxbc[..4], b"DXBC", "{name}");
    }
}
