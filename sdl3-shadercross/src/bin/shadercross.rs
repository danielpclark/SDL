// Rust translation of src/cli.c from SDL_shadercross.
// Simple DirectMedia Layer Shader Cross Compiler
// Copyright (C) 2024 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! `shadercross`: the command-line front end of SDL_shadercross, translating
//! SPIR-V (or, with DXC, HLSL) to DXBC, DXIL, MSL, SPIR-V, HLSL or JSON
//! reflection data. Run `shadercross --help` for the options.

use std::ffi::OsString;
use std::path::PathBuf;
use std::process::ExitCode;

use sdl3::io::IoStream;
use sdl3::log::Category;
use sdl3::properties::Properties;
use sdl3::stdlib::string::{strcasecmp, strcasestr, utf8strlcpy_len, utf8strlen};
use sdl3_shadercross::{
    ComputePipelineMetadata, GraphicsShaderMetadata, HlslDefine, HlslInfo, IOVarType, ShaderStage,
    SpirvInfo, PROP_SHADER_CULL_UNUSED_BINDINGS_BOOLEAN, PROP_SHADER_DEBUG_ENABLE_BOOLEAN,
    PROP_SHADER_DEBUG_NAME_STRING, PROP_SPIRV_MSL_VERSION_STRING,
    PROP_SPIRV_PSSL_COMPATIBILITY_BOOLEAN,
};

// We can emit HLSL and JSON as a destination, so let's redefine the shader format enum.
#[derive(Clone, Copy, PartialEq, Eq)]
enum ShaderFormat {
    Invalid,
    Spirv,
    Dxbc,
    Dxil,
    Msl,
    Hlsl,
    Json,
}

fn print_help() {
    let column_width = 32;
    let row = |name: &str, text: &str| sdl3::log!("  {:<column_width$} {}", name, text);
    sdl3::log!("Usage: shadercross <input> [options]");
    sdl3::log!("Required options:\n");
    row(
        "-s | --source <value>",
        "Source language format. May be inferred from the filename. Values: [SPIRV, HLSL]",
    );
    row(
        "-d | --dest <value>",
        "Destination format. May be inferred from the filename. Values: [DXBC, DXIL, MSL, SPIRV, HLSL, JSON]",
    );
    row(
        "-t | --stage <value>",
        "Shader stage. May be inferred from the filename. Values: [vertex, fragment, compute]",
    );
    row(
        "-e | --entrypoint <value>",
        "Entrypoint function name. Default: \"main\".",
    );
    row("-o | --output <value>", "Output file.");
    sdl3::log!("\n");
    sdl3::log!("Optional options:\n");
    row(
        "-I | --include <value>",
        "HLSL include directory. Only used with HLSL source.",
    );
    row(
        "-D<name>[=<value>]",
        "HLSL define. Only used with HLSL source. Can be repeated.",
    );
    row(
        "",
        "If =<value> is omitted the define will be treated as equal to 1.",
    );
    row(
        "--msl-version <value>",
        "Target MSL version. Only used when transpiling to MSL. The default is 1.2.0.",
    );
    row(
        "-c | --cull",
        "Allow the compiler to cull unused resource bindings. This may lead to surprising binding behavior so be careful when enabling this!",
    );
    row(
        "-g | --debug",
        "Generate debug information when possible. Shaders are valid only when graphics debuggers are attached.",
    );
    row(
        "-p | --pssl",
        "Generate PSSL-compatible shader. Destination format should be HLSL.",
    );
}

fn io_var_type_to_string(io_var_type: IOVarType, vector_size: u32) -> &'static str {
    const NAMES: [(IOVarType, [&str; 4]); 11] = [
        (IOVarType::Int8, ["byte", "byte2", "byte3", "byte4"]),
        (IOVarType::UInt8, ["ubyte", "ubyte2", "ubyte3", "ubyte4"]),
        (IOVarType::Int16, ["short", "short2", "short3", "short4"]),
        (
            IOVarType::UInt16,
            ["ushort", "ushort2", "ushort3", "ushort4"],
        ),
        (IOVarType::Int32, ["int", "int2", "int3", "int4"]),
        (IOVarType::UInt32, ["uint", "uint2", "uint3", "uint4"]),
        (IOVarType::Int64, ["long", "long2", "long3", "long4"]),
        (IOVarType::UInt64, ["ulong", "ulong2", "ulong3", "ulong4"]),
        (IOVarType::Float16, ["half", "half2", "half3", "half4"]),
        (IOVarType::Float32, ["float", "float2", "float3", "float4"]),
        (
            IOVarType::Float64,
            ["double", "double2", "double3", "double4"],
        ),
    ];
    for (t, names) in NAMES {
        if t == io_var_type && (1..=4).contains(&vector_size) {
            return names[vector_size as usize - 1];
        }
    }

    sdl3::warn!(
        Category::Application,
        "Unknown IO variable type: vector_type={} vector_size={}",
        io_var_type_raw(io_var_type),
        vector_size
    );
    "unknown"
}

/// The `SDL_ShaderCross_IOVarType` value, as the C prints it.
fn io_var_type_raw(t: IOVarType) -> u32 {
    match t {
        IOVarType::Unknown => 0,
        IOVarType::Int8 => 1,
        IOVarType::UInt8 => 2,
        IOVarType::Int16 => 3,
        IOVarType::UInt16 => 4,
        IOVarType::Int32 => 5,
        IOVarType::UInt32 => 6,
        IOVarType::Int64 => 7,
        IOVarType::UInt64 => 8,
        IOVarType::Float16 => 9,
        IOVarType::Float32 => 10,
        IOVarType::Float64 => 11,
    }
}

fn write_graphics_reflect_json(output_io: &mut IoStream<'_>, info: &GraphicsShaderMetadata) {
    let mut out = format!(
        "{{ \"samplers\": {}, \"storage_textures\": {}, \"storage_buffers\": {}, \"uniform_buffers\": {}, ",
        info.resource_info.num_samplers,
        info.resource_info.num_storage_textures,
        info.resource_info.num_storage_buffers,
        info.resource_info.num_uniform_buffers
    );

    out += "\"inputs\": [";
    for (i, input) in info.inputs.iter().enumerate() {
        out += &format!(
            "{{ \"name\": \"{}\", \"type\": \"{}\", \"location\": {} }}{}",
            input.name,
            io_var_type_to_string(input.vector_type, input.vector_size),
            input.location,
            if i + 1 < info.inputs.len() { ", " } else { "" }
        );
    }
    out += "], ";

    out += "\"outputs\": [";
    for (i, output) in info.outputs.iter().enumerate() {
        out += &format!(
            "{{ \"name\": \"{}\", \"type\": \"{}\", \"location\": {} }}{}",
            output.name,
            io_var_type_to_string(output.vector_type, output.vector_size),
            output.location,
            if i + 1 < info.outputs.len() { ", " } else { "" }
        );
    }
    out += "] }\n";
    output_io.write(out.as_bytes());
}

fn write_compute_reflect_json(output_io: &mut IoStream<'_>, info: &ComputePipelineMetadata) {
    let out = format!(
        "{{ \"samplers\": {}, \"readonly_storage_textures\": {}, \"readonly_storage_buffers\": {}, \"readwrite_storage_textures\": {}, \"readwrite_storage_buffers\": {}, \"uniform_buffers\": {}, \"threadcount_x\": {}, \"threadcount_y\": {}, \"threadcount_z\": {} }}\n",
        info.num_samplers,
        info.num_readonly_storage_textures,
        info.num_readonly_storage_buffers,
        info.num_readwrite_storage_textures,
        info.num_readwrite_storage_buffers,
        info.num_uniform_buffers,
        info.threadcount_x,
        info.threadcount_y,
        info.threadcount_z
    );
    output_io.write(out.as_bytes());
}

/// `SDL_utf8strlcpy(dst, src, dst_bytes)` into a fresh string.
fn utf8strlcpy(src: &str, dst_bytes: usize) -> String {
    let n = utf8strlcpy_len(src, dst_bytes);
    String::from_utf8_lossy(&src.as_bytes()[..n]).into_owned()
}

/// `SDL_strcasecmp(a, b) == 0`.
fn eq_ignore_case(a: &str, b: &str) -> bool {
    strcasecmp(a, b) == std::cmp::Ordering::Equal
}

/// Sets a property; the C ignores the result.
fn set_property(props: &Properties, name: &str, value: impl Into<sdl3::properties::Value>) {
    let _ = props.set(name, value);
}

/// The SPIR-V info properties: the debug, cull, MSL version and PSSL flags.
struct Options<'a> {
    filename: &'a str,
    enable_debug: bool,
    cull_unused_bindings: bool,
    msl_version: Option<&'a str>,
    pssl_compat: bool,
}

fn main() -> ExitCode {
    let argv: Vec<OsString> = std::env::args_os().collect();
    let args: Vec<String> = argv
        .iter()
        .map(|a| a.to_string_lossy().into_owned())
        .collect();
    let argv0 = args.first().map(String::as_str).unwrap_or("shadercross");
    ExitCode::from(run(&argv, &args, argv0))
}

fn run(argv: &[OsString], args: &[String], argv0: &str) -> u8 {
    let mut source_valid = false;
    let mut destination_valid = false;
    let mut stage_valid = false;

    let mut spirv_source = false;
    let mut destination_format = ShaderFormat::Invalid;
    let mut shader_stage = ShaderStage::Vertex;
    let mut output_filename: Option<usize> = None;
    let mut entrypoint_name: &str = "main";
    let mut include_dir: Option<&str> = None;

    let mut filename: Option<usize> = None;
    let mut accept_optionals = true;

    let mut defines: Vec<HlslDefine> = Vec::new();

    let mut cull_unused_bindings = false;
    let mut enable_debug = false;
    let mut msl_version: Option<&str> = None;

    let mut pssl_compat = false;

    macro_rules! requires_argument {
        ($i:expr, $arg:expr) => {
            if $i + 1 >= args.len() {
                sdl3::error!(Category::Application, "{} requires an argument", $arg);
                print_help();
                return 1;
            }
        };
    }

    let mut i = 1;
    while i < args.len() {
        let arg = args[i].as_str();

        if accept_optionals && arg.starts_with('-') {
            if arg == "-h" || arg == "--help" {
                print_help();
                return 0;
            } else if arg == "-s" || arg == "--source" {
                requires_argument!(i, arg);
                i += 1;
                if eq_ignore_case(&args[i], "spirv") {
                    spirv_source = true;
                    source_valid = true;
                } else if eq_ignore_case(&args[i], "hlsl") {
                    spirv_source = false;
                    source_valid = true;
                } else {
                    sdl3::error!(
                        Category::Application,
                        "Unrecognized source input {}, source must be SPIRV or HLSL!",
                        args[i]
                    );
                    print_help();
                    return 1;
                }
            } else if arg == "-d" || arg == "--dest" {
                requires_argument!(i, arg);
                i += 1;
                let value = args[i].as_str();
                if eq_ignore_case(value, "DXBC") {
                    destination_format = ShaderFormat::Dxbc;
                    destination_valid = true;
                } else if eq_ignore_case(value, "DXIL") {
                    destination_format = ShaderFormat::Dxil;
                    destination_valid = true;
                } else if eq_ignore_case(value, "MSL") {
                    destination_format = ShaderFormat::Msl;
                    destination_valid = true;
                } else if eq_ignore_case(value, "SPIRV") {
                    destination_format = ShaderFormat::Spirv;
                    destination_valid = true;
                } else if eq_ignore_case(value, "HLSL") {
                    destination_format = ShaderFormat::Hlsl;
                    destination_valid = true;
                } else if eq_ignore_case(value, "JSON") {
                    destination_format = ShaderFormat::Json;
                    destination_valid = true;
                } else {
                    sdl3::error!(
                        Category::Application,
                        "Unrecognized destination input {}, destination must be DXBC, DXIL, MSL or SPIRV!",
                        value
                    );
                    print_help();
                    return 1;
                }
            } else if arg == "-t" || arg == "--stage" {
                requires_argument!(i, arg);
                i += 1;
                let value = args[i].as_str();
                if eq_ignore_case(value, "vertex") {
                    shader_stage = ShaderStage::Vertex;
                    stage_valid = true;
                } else if eq_ignore_case(value, "fragment") {
                    shader_stage = ShaderStage::Fragment;
                    stage_valid = true;
                } else if eq_ignore_case(value, "compute") {
                    shader_stage = ShaderStage::Compute;
                    stage_valid = true;
                } else {
                    sdl3::error!(
                        Category::Application,
                        "Unrecognized shader stage input {}, must be vertex, fragment, or compute.",
                        value
                    );
                    print_help();
                    return 1;
                }
            } else if arg == "-e" || arg == "--entrypoint" {
                requires_argument!(i, arg);
                i += 1;
                entrypoint_name = &args[i];
            } else if arg == "-I" || arg == "--include" {
                if include_dir.is_some() {
                    sdl3::error!(Category::Application, "'{}' can only be used once", arg);
                    print_help();
                    return 1;
                }
                requires_argument!(i, arg);
                i += 1;
                include_dir = Some(&args[i]);
            } else if arg == "-o" || arg == "--output" {
                requires_argument!(i, arg);
                i += 1;
                output_filename = Some(i);
            } else if let Some(define) = arg.strip_prefix("-D") {
                // The name is copied with SDL_utf8strlcpy() into a buffer
                // sized from the argument (by code points, without '=').
                match arg.find('=') {
                    Some(eq) => {
                        let len = eq - 1;
                        defines.push(HlslDefine {
                            name: utf8strlcpy(define, len),
                            value: Some(arg[eq + 1..].to_string()),
                        });
                    }
                    None => {
                        // no '=' was found
                        let len = utf8strlen(arg) + 1 - 2;
                        defines.push(HlslDefine {
                            name: utf8strlcpy(define, len),
                            value: None,
                        });
                    }
                }
            } else if arg == "--msl-version" {
                requires_argument!(i, arg);
                i += 1;
                msl_version = Some(&args[i]);
            } else if arg == "-c" || arg == "--cull" {
                cull_unused_bindings = true;
            } else if arg == "-g" || arg == "--debug" {
                enable_debug = true;
            } else if arg == "-p" || arg == "--pssl" {
                pssl_compat = true;
            } else if arg == "--" {
                accept_optionals = false;
            } else {
                sdl3::error!(
                    Category::Application,
                    "{}: Unknown argument: {}",
                    argv0,
                    arg
                );
                print_help();
                return 1;
            }
        } else if filename.is_none() {
            filename = Some(i);
        } else {
            sdl3::error!(
                Category::Application,
                "{}: Unknown argument: {}",
                argv0,
                arg
            );
            print_help();
            return 1;
        }
        i += 1;
    }
    let Some(filename_index) = filename else {
        sdl3::error!(Category::Application, "{}: missing input path", argv0);
        print_help();
        return 1;
    };
    let Some(output_index) = output_filename else {
        sdl3::error!(Category::Application, "{}: missing output path", argv0);
        print_help();
        return 1;
    };
    let filename = args[filename_index].as_str();
    let output_filename = args[output_index].as_str();
    let file_data = match sdl3::io::load_file(PathBuf::from(&argv[filename_index])) {
        Ok(data) => data,
        Err(e) => {
            sdl3::error!(Category::Application, "Invalid file ({})", e.message());
            return 1;
        }
    };

    if sdl3_shadercross::init().is_err() {
        sdl3::error!(Category::Gpu, "{}", "Failed to initialize shadercross!");
        return 1;
    }

    if !source_valid {
        if filename.contains(".spv") {
            spirv_source = true;
        } else if filename.contains(".hlsl") {
            spirv_source = false;
        } else {
            sdl3::error!(
                Category::Application,
                "{}",
                "Could not infer source format!"
            );
            print_help();
            return 1;
        }
    }

    if !destination_valid {
        if output_filename.contains(".dxbc") {
            destination_format = ShaderFormat::Dxbc;
        } else if output_filename.contains(".dxil") {
            destination_format = ShaderFormat::Dxil;
        } else if output_filename.contains(".msl") {
            destination_format = ShaderFormat::Msl;
        } else if output_filename.contains(".spv") {
            destination_format = ShaderFormat::Spirv;
        } else if output_filename.contains(".hlsl") {
            destination_format = ShaderFormat::Hlsl;
        } else if output_filename.contains(".json") {
            destination_format = ShaderFormat::Json;
        } else {
            sdl3::error!(
                Category::Application,
                "{}",
                "Could not infer destination format!"
            );
            print_help();
            return 1;
        }
    }

    if !stage_valid {
        if strcasestr(filename, ".vert").is_some() {
            shader_stage = ShaderStage::Vertex;
        } else if strcasestr(filename, ".frag").is_some() {
            shader_stage = ShaderStage::Fragment;
        } else if strcasestr(filename, ".comp").is_some() {
            shader_stage = ShaderStage::Compute;
        } else {
            sdl3::error!(
                Category::Application,
                "Could not infer shader stage from filename!"
            );
            print_help();
            return 1;
        }
    }

    let mut output_io = match IoStream::from_file(PathBuf::from(&argv[output_index]), "w") {
        Ok(io) => io,
        Err(e) => {
            sdl3::error!(Category::Application, "{}", e.message());
            return 1;
        }
    };

    let options = Options {
        filename,
        enable_debug,
        cull_unused_bindings,
        msl_version,
        pssl_compat,
    };
    let result = if spirv_source {
        from_spirv(
            &mut output_io,
            &file_data,
            entrypoint_name,
            shader_stage,
            destination_format,
            &options,
        )
    } else {
        // SDL_LoadFile() NUL-terminates the data, which is the HLSL source
        // string.
        let end = file_data
            .iter()
            .position(|&b| b == 0)
            .unwrap_or(file_data.len());
        let source = String::from_utf8_lossy(&file_data[..end]);
        from_hlsl(
            &mut output_io,
            &source,
            entrypoint_name,
            include_dir,
            &defines,
            shader_stage,
            destination_format,
            &options,
        )
    };

    let _ = output_io.close();
    sdl3_shadercross::quit();
    sdl3::init::quit();

    result
}

/// The `spirvSource` branch of `main()`.
fn from_spirv(
    output_io: &mut IoStream<'_>,
    file_data: &[u8],
    entrypoint_name: &str,
    shader_stage: ShaderStage,
    destination_format: ShaderFormat,
    options: &Options<'_>,
) -> u8 {
    let mut result = 0;
    let props = Properties::new();
    if options.enable_debug {
        set_property(&props, PROP_SHADER_DEBUG_ENABLE_BOOLEAN, true);
        // FIXME (upstream): the debug name is set as a boolean property (to
        // true), not as the file name.
        set_property(&props, PROP_SHADER_DEBUG_NAME_STRING, true);
    }
    if options.cull_unused_bindings {
        set_property(&props, PROP_SHADER_CULL_UNUSED_BINDINGS_BOOLEAN, true);
    }
    if let Some(msl_version) = options.msl_version {
        set_property(&props, PROP_SPIRV_MSL_VERSION_STRING, msl_version);
    }
    if options.pssl_compat {
        set_property(&props, PROP_SPIRV_PSSL_COMPATIBILITY_BOOLEAN, true);
    }
    let spirv_info = SpirvInfo {
        bytecode: file_data,
        entrypoint: entrypoint_name,
        shader_stage,
        props: Some(&props),
    };

    match destination_format {
        ShaderFormat::Dxbc => match sdl3_shadercross::compile_dxbc_from_spirv(&spirv_info) {
            Err(e) => {
                sdl3::error!(
                    Category::Application,
                    "Failed to compile DXBC from SPIR-V: {}",
                    e.message()
                );
                result = 1;
            }
            Ok(buffer) => {
                output_io.write(&buffer);
            }
        },

        ShaderFormat::Dxil => match sdl3_shadercross::compile_dxil_from_spirv(&spirv_info) {
            Err(e) => {
                sdl3::error!(
                    Category::Application,
                    "Failed to compile DXIL from SPIR-V: {}",
                    e.message()
                );
                result = 1;
            }
            Ok(buffer) => {
                output_io.write(&buffer);
            }
        },

        ShaderFormat::Msl => match sdl3_shadercross::transpile_msl_from_spirv(&spirv_info) {
            Err(e) => {
                sdl3::error!(
                    Category::Application,
                    "Failed to transpile MSL from SPIR-V: {}",
                    e.message()
                );
                result = 1;
            }
            Ok(buffer) => {
                output_io.write(buffer.as_bytes());
            }
        },

        ShaderFormat::Hlsl => match sdl3_shadercross::transpile_hlsl_from_spirv(&spirv_info) {
            Err(e) => {
                sdl3::error!(
                    Category::Application,
                    "Failed to transpile HLSL from SPIRV: {}",
                    e.message()
                );
                result = 1;
            }
            Ok(buffer) => {
                output_io.write(buffer.as_bytes());
            }
        },

        ShaderFormat::Spirv => {
            sdl3::error!(
                Category::Application,
                "Input and output are both SPIRV. Did you mean to do that?"
            );
            result = 1;
        }

        ShaderFormat::Json => result = write_reflection_json(output_io, file_data, shader_stage),

        ShaderFormat::Invalid => {
            sdl3::error!(Category::Application, "Destination format not provided!");
            result = 1;
        }
    }
    result
}

/// The JSON cases: reflects the SPIR-V and writes it out.
fn write_reflection_json(
    output_io: &mut IoStream<'_>,
    spirv: &[u8],
    shader_stage: ShaderStage,
) -> u8 {
    if shader_stage == ShaderStage::Compute {
        match sdl3_shadercross::reflect_compute_spirv(spirv, None) {
            Ok(info) => write_compute_reflect_json(output_io, &info),
            Err(e) => {
                sdl3::error!(
                    Category::Application,
                    "Failed to reflect SPIRV: {}",
                    e.message()
                );
                return 1;
            }
        }
    } else {
        match sdl3_shadercross::reflect_graphics_spirv(spirv, None) {
            Ok(info) => write_graphics_reflect_json(output_io, &info),
            Err(e) => {
                sdl3::error!(
                    Category::Application,
                    "Failed to reflect SPIRV: {}",
                    e.message()
                );
                return 1;
            }
        }
    }
    0
}

/// The HLSL-source branch of `main()`.
#[allow(clippy::too_many_arguments)]
fn from_hlsl(
    output_io: &mut IoStream<'_>,
    source: &str,
    entrypoint_name: &str,
    include_dir: Option<&str>,
    defines: &[HlslDefine],
    shader_stage: ShaderStage,
    destination_format: ShaderFormat,
    options: &Options<'_>,
) -> u8 {
    let mut result = 0;
    let props = Properties::new();

    if options.enable_debug {
        set_property(&props, PROP_SHADER_DEBUG_ENABLE_BOOLEAN, true);
        set_property(&props, PROP_SHADER_DEBUG_NAME_STRING, options.filename);
    }

    if options.cull_unused_bindings {
        set_property(&props, PROP_SHADER_CULL_UNUSED_BINDINGS_BOOLEAN, true);
    }

    let hlsl_info = HlslInfo {
        source,
        entrypoint: entrypoint_name,
        include_dir,
        // The C passes NULL when there are no defines.
        defines: if defines.is_empty() {
            None
        } else {
            Some(defines)
        },
        shader_stage,
        props: Some(&props),
    };

    match destination_format {
        ShaderFormat::Dxbc => match sdl3_shadercross::compile_dxbc_from_hlsl(&hlsl_info) {
            Err(e) => {
                sdl3::error!(
                    Category::Application,
                    "Failed to compile DXBC from HLSL: {}",
                    e.message()
                );
                result = 1;
            }
            Ok(buffer) => {
                output_io.write(&buffer);
            }
        },

        ShaderFormat::Dxil => match sdl3_shadercross::compile_dxil_from_hlsl(&hlsl_info) {
            Err(e) => {
                sdl3::error!(
                    Category::Application,
                    "Failed to compile DXIL from HLSL: {}",
                    e.message()
                );
                result = 1;
            }
            Ok(buffer) => {
                output_io.write(&buffer);
            }
        },

        // TODO: Should we have TranspileMSLFromHLSL?
        ShaderFormat::Msl => match sdl3_shadercross::compile_spirv_from_hlsl(&hlsl_info) {
            Err(e) => {
                sdl3::error!(
                    Category::Application,
                    "Failed to transpile MSL from HLSL: {}",
                    e.message()
                );
                result = 1;
            }
            Ok(spirv) => {
                let spirv_props = Properties::new();

                if options.enable_debug {
                    set_property(&spirv_props, PROP_SHADER_DEBUG_ENABLE_BOOLEAN, true);
                    set_property(
                        &spirv_props,
                        PROP_SHADER_DEBUG_NAME_STRING,
                        options.filename,
                    );
                }
                if options.cull_unused_bindings {
                    // FIXME (upstream): this sets the debug property, not the
                    // cull one.
                    set_property(&spirv_props, PROP_SHADER_DEBUG_ENABLE_BOOLEAN, true);
                }
                if let Some(msl_version) = options.msl_version {
                    set_property(&spirv_props, PROP_SPIRV_MSL_VERSION_STRING, msl_version);
                }
                let spirv_info = SpirvInfo {
                    bytecode: &spirv,
                    entrypoint: entrypoint_name,
                    shader_stage,
                    props: Some(&spirv_props),
                };

                match sdl3_shadercross::transpile_msl_from_spirv(&spirv_info) {
                    Err(e) => {
                        sdl3::error!(
                            Category::Application,
                            "Failed to transpile MSL from HLSL: {}",
                            e.message()
                        );
                        result = 1;
                    }
                    Ok(buffer) => {
                        output_io.write(buffer.as_bytes());
                    }
                }
            }
        },

        ShaderFormat::Spirv => match sdl3_shadercross::compile_spirv_from_hlsl(&hlsl_info) {
            Err(e) => {
                sdl3::error!(
                    Category::Application,
                    "Failed to compile SPIR-V From HLSL: {}",
                    e.message()
                );
                result = 1;
            }
            Ok(buffer) => {
                output_io.write(&buffer);
            }
        },

        ShaderFormat::Hlsl => match sdl3_shadercross::compile_spirv_from_hlsl(&hlsl_info) {
            Err(e) => {
                sdl3::error!(
                    Category::Application,
                    "Failed to compile HLSL to SPIRV: {}",
                    e.message()
                );
                result = 1;
            }
            Ok(spirv) => {
                let spirv_props = Properties::new();

                if options.enable_debug {
                    set_property(&spirv_props, PROP_SHADER_DEBUG_ENABLE_BOOLEAN, true);
                    set_property(
                        &spirv_props,
                        PROP_SHADER_DEBUG_NAME_STRING,
                        options.filename,
                    );
                }
                if options.cull_unused_bindings {
                    set_property(&spirv_props, PROP_SHADER_CULL_UNUSED_BINDINGS_BOOLEAN, true);
                }
                if options.pssl_compat {
                    set_property(&spirv_props, PROP_SPIRV_PSSL_COMPATIBILITY_BOOLEAN, true);
                }
                let spirv_info = SpirvInfo {
                    bytecode: &spirv,
                    entrypoint: entrypoint_name,
                    shader_stage,
                    props: Some(&spirv_props),
                };

                match sdl3_shadercross::transpile_hlsl_from_spirv(&spirv_info) {
                    Err(e) => {
                        sdl3::error!(
                            Category::Application,
                            "Failed to transpile HLSL from SPIRV: {}",
                            e.message()
                        );
                        result = 1;
                    }
                    Ok(buffer) => {
                        output_io.write(buffer.as_bytes());
                    }
                }
            }
        },

        ShaderFormat::Json => match sdl3_shadercross::compile_spirv_from_hlsl(&hlsl_info) {
            Err(e) => {
                sdl3::error!(
                    Category::Application,
                    "Failed to compile HLSL to SPIRV: {}",
                    e.message()
                );
                result = 1;
            }
            Ok(spirv) => result = write_reflection_json(output_io, &spirv, shader_stage),
        },

        ShaderFormat::Invalid => {
            sdl3::error!(Category::Application, "Destination format not provided!");
            result = 1;
        }
    }
    result
}
