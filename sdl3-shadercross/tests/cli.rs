// Tests of the `shadercross` command-line tool (src/bin/shadercross.rs, the
// translation of SDL_shadercross's src/cli.c).

use std::path::PathBuf;
use std::process::Command;

fn testdata(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("testdata")
        .join(name)
}

/// A fresh output directory for one test.
fn out_dir(test: &str) -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!("shadercross-cli-{test}"));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn shadercross(args: &[&std::ffi::OsStr]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_shadercross"))
        .args(args)
        .output()
        .unwrap()
}

/// The text of a section of testdata/reference.txt, without its
/// "entrypoint:" line.
fn reference(header: &str) -> String {
    let text = std::fs::read_to_string(testdata("reference.txt")).unwrap();
    let start = text.find(&format!("=== {header}\n")).unwrap() + header.len() + 5;
    let body = &text[start..];
    let body = &body[..body.find("\n=== ").map(|e| e + 1).unwrap_or(body.len())];
    body.split_once('\n').unwrap().1.to_string()
}

#[test]
fn json_reflection() {
    let dir = out_dir("json");
    let vs = dir.join("vs.json");
    let out = shadercross(&[
        testdata("vs_basic.spv").as_os_str(),
        "-t".as_ref(),
        "vertex".as_ref(),
        "-o".as_ref(),
        vs.as_os_str(),
    ]);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(
        std::fs::read_to_string(&vs).unwrap(),
        "{ \"samplers\": 0, \"storage_textures\": 0, \"storage_buffers\": 0, \"uniform_buffers\": 1, \
         \"inputs\": [{ \"name\": \"in_color\", \"type\": \"float4\", \"location\": 1 }, \
         { \"name\": \"in_uv\", \"type\": \"float2\", \"location\": 2 }, \
         { \"name\": \"in_position\", \"type\": \"float3\", \"location\": 0 }], \
         \"outputs\": [{ \"name\": \"out_color\", \"type\": \"float4\", \"location\": 0 }, \
         { \"name\": \"out_uv\", \"type\": \"float2\", \"location\": 1 }] }\n"
    );

    // The stage is inferred from ".comp" in the file name.
    let input = dir.join("basic.comp.spv");
    std::fs::copy(testdata("cs_basic.spv"), &input).unwrap();
    let cs = dir.join("cs.json");
    let out = shadercross(&[input.as_os_str(), "-o".as_ref(), cs.as_os_str()]);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(
        std::fs::read_to_string(&cs).unwrap(),
        "{ \"samplers\": 0, \"readonly_storage_textures\": 0, \"readonly_storage_buffers\": 1, \
         \"readwrite_storage_textures\": 0, \"readwrite_storage_buffers\": 1, \"uniform_buffers\": 1, \
         \"threadcount_x\": 64, \"threadcount_y\": 1, \"threadcount_z\": 1 }\n"
    );
}

#[test]
fn transpiles() {
    let dir = out_dir("transpile");
    let msl = dir.join("vs.metal");
    let out = shadercross(&[
        testdata("vs_basic.spv").as_os_str(),
        "--stage".as_ref(),
        "VERTEX".as_ref(),
        "-d".as_ref(),
        "msl".as_ref(),
        "--msl-version".as_ref(),
        "2.1.0".as_ref(),
        "-o".as_ref(),
        msl.as_os_str(),
    ]);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(
        std::fs::read_to_string(&msl).unwrap(),
        reference("vs_basic msl vertex 0 2.1.0 0")
    );

    let hlsl = dir.join("fs.hlsl");
    let out = shadercross(&[
        testdata("fs_textured.spv").as_os_str(),
        "-t".as_ref(),
        "fragment".as_ref(),
        "-p".as_ref(),
        "-o".as_ref(),
        hlsl.as_os_str(),
    ]);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(
        std::fs::read_to_string(&hlsl).unwrap(),
        reference("fs_textured hlsl fragment 50 - 1")
    );
}

#[test]
fn errors() {
    let dir = out_dir("errors");
    // Missing input.
    let out = shadercross(&[]);
    assert_eq!(out.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&out.stderr).contains(": missing input path"));

    // SPIR-V to SPIR-V.
    let spv = dir.join("out.spv");
    let out = shadercross(&[
        testdata("vs_basic.spv").as_os_str(),
        "-t".as_ref(),
        "vertex".as_ref(),
        "-o".as_ref(),
        spv.as_os_str(),
    ]);
    assert_eq!(out.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&out.stderr)
        .contains("Input and output are both SPIRV. Did you mean to do that?"));

    // DXIL needs DXC.
    let dxil = dir.join("out.dxil");
    let out = shadercross(&[
        testdata("vs_basic.spv").as_os_str(),
        "-t".as_ref(),
        "vertex".as_ref(),
        "-o".as_ref(),
        dxil.as_os_str(),
    ]);
    assert_eq!(out.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&out.stderr).contains("Failed to compile DXIL from SPIR-V: "));

    // An unknown option.
    let out = shadercross(&["--nope".as_ref()]);
    assert_eq!(out.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&out.stderr).contains(": Unknown argument: --nope"));

    // Help.
    let out = shadercross(&["--help".as_ref()]);
    assert_eq!(out.status.code(), Some(0));
    assert!(
        String::from_utf8_lossy(&out.stderr).starts_with("Usage: shadercross <input> [options]\n")
    );
}
