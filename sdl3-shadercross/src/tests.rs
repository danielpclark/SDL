// Tests: the translation's output, byte for byte against SDL_shadercross
// and SPIRV-Cross run on the same SPIR-V (testdata/reference.txt; see
// tools/gen_shadercross_testdata.py).

use crate::spirv_cross::c_api::*;

/// The test shaders (testdata/*.spv), by name.
pub(crate) const SHADERS: &[(&str, &[u8])] = &[
    ("cs_basic", include_bytes!("../testdata/cs_basic.spv")),
    (
        "cs_basic_opt",
        include_bytes!("../testdata/cs_basic_opt.spv"),
    ),
    ("cs_image", include_bytes!("../testdata/cs_image.spv")),
    ("cs_layout", include_bytes!("../testdata/cs_layout.spv")),
    ("cs_math", include_bytes!("../testdata/cs_math.spv")),
    ("fs_complex", include_bytes!("../testdata/fs_complex.spv")),
    (
        "fs_complex_opt",
        include_bytes!("../testdata/fs_complex_opt.spv"),
    ),
    ("fs_sampling", include_bytes!("../testdata/fs_sampling.spv")),
    ("fs_storage", include_bytes!("../testdata/fs_storage.spv")),
    ("fs_textured", include_bytes!("../testdata/fs_textured.spv")),
    ("vs_basic", include_bytes!("../testdata/vs_basic.spv")),
    ("vs_push", include_bytes!("../testdata/vs_push.spv")),
    ("vs_storage", include_bytes!("../testdata/vs_storage.spv")),
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
