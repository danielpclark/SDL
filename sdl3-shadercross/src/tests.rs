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
    ("cs_math", include_bytes!("../testdata/cs_math.spv")),
    ("fs_complex", include_bytes!("../testdata/fs_complex.spv")),
    (
        "fs_complex_opt",
        include_bytes!("../testdata/fs_complex_opt.spv"),
    ),
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
