#!/usr/bin/env python3
"""Generate sdl3/src/render/gpu/shaders.rs from SDL's
src/render/gpu/SDL_shaders_gpu.c, SDL_shaders_gpu.h and the precompiled
SPIR-V and DXIL headers they include (shaders/*.spv.h and *.dxil.h, made by
build-shaders.sh with SDL_shadercross and xxd).

Usage: gen_gpu_render_shaders.py <SDL source dir> > sdl3/src/render/gpu/shaders.rs

The blobs are kept as byte arrays, as upstream has them, sixteen to a line;
the shader IDs and the source table are translated around them. The
functions that create the shaders on a device (CompileShader(),
GPU_InitShaders()...) are translated by hand in mod.rs.

The DXIL is compiled in on Windows only, with the Direct3D 12 GPU backend
(upstream's HAVE_DXIL60_SHADERS with SDL_GPU_D3D12). The MSL blobs aren't
taken: they go with the Metal GPU backend (HAVE_METAL_SHADERS), which isn't
translated yet.
"""
import re
import sys

src_dir = sys.argv[1]
gpu_dir = f"{src_dir}/src/render/gpu"


def read(name):
    return open(f"{gpu_dir}/{name}", encoding="utf-8").read()


def enum_values(header, name):
    body = re.search(r"typedef enum\s*\{([^}]*)\}\s*" + name + ";", header).group(1)
    values = [v.split("=")[0].strip() for v in body.split(",") if v.strip()]
    assert values[0].endswith("_INVALID"), values
    assert values[-1].startswith("NUM_"), values
    return values[1:-1]


header = read("SDL_shaders_gpu.h")
vert_ids = enum_values(header, "GPU_VertexShaderID")
frag_ids = enum_values(header, "GPU_FragmentShaderID")
# FRAG_SHADER_TEXTURE_CUSTOM is the application's shader of an
# SDL_GPURenderState, which has no source.
assert frag_ids[-1] == "FRAG_SHADER_TEXTURE_CUSTOM", frag_ids
frag_source_ids = frag_ids[:-1]

source = read("SDL_shaders_gpu.c")


def table(name, ids):
    body = re.search(name + r"\[\w+\] = \{(.*?)\n\};", source, re.S).group(1)
    entries = {}
    for m in re.finditer(r"\[(\w+)\] = \{(.*?)\n    \}", body, re.S):
        fields = m.group(2)
        entries[m.group(1)] = (
            int(re.search(r"\.num_samplers = (\d+)", fields).group(1)),
            int(re.search(r"\.num_uniform_buffers = (\d+)", fields).group(1)),
            re.search(r"SHADER_SPIRV\((\w+)\)", fields).group(1),
            re.search(r"SHADER_DXIL60\((\w+)\)", fields).group(1),
        )
    assert list(entries) == ids, (list(entries), ids)
    return [entries[i] for i in ids]


vert_sources = table("vert_shader_sources", vert_ids)
frag_sources = table("frag_shader_sources", frag_source_ids)



def read_blobs(index, suffix):
    """The blobs of the headers `index` includes, by name, in include order."""
    includes = re.findall(r'#include "(\S+' + re.escape(suffix) + ')"', read(f"shaders/{index}"))
    blobs = {}
    for include in includes:
        text = read(f"shaders/{include}")
        m = re.search(r"static const unsigned char (\w+)\[\] = \{(.*?)\};", text, re.S)
        name, body = m.group(1), m.group(2)
        data = bytes(int(b, 16) for b in re.findall(r"0x[0-9a-fA-F]{2}", body))
        length = int(re.search(name + r"_len = (\d+);", text).group(1))
        assert len(data) == length, f"{include}: size"
        blobs[name] = (include, data)
    return blobs


blobs = read_blobs("spir-v.h", ".spv.h")
for include, data in blobs.values():
    assert int.from_bytes(data[:4], "little") == 0x07230203, f"{include}: not SPIR-V"
used = {s[2] for s in vert_sources + frag_sources}
assert used == set(blobs), (used, set(blobs))

dxil_blobs = read_blobs("dxil.h", ".dxil.h")
for include, data in dxil_blobs.values():
    # A DXBC container (the magic, a digest, version 1.0, the total size)
    # holding a DXIL part; signed (a nonzero digest), as Direct3D 12 needs.
    assert data[:4] == b"DXBC", f"{include}: not a DXBC container"
    assert any(data[4:20]), f"{include}: unsigned"
    assert int.from_bytes(data[24:28], "little") == len(data), f"{include}: size"
    parts = int.from_bytes(data[28:32], "little")
    offsets = [int.from_bytes(data[32 + 4 * i:36 + 4 * i], "little") for i in range(parts)]
    fourccs = [data[o:o + 4] for o in offsets]
    assert b"DXIL" in fourccs, f"{include}: no DXIL part ({fourccs})"
used = {s[3] for s in vert_sources + frag_sources}
assert used == set(dxil_blobs), (used, set(dxil_blobs))

# The SPIR-V generator of the blobs (the third word: 14 is Google's
# spiregg, the SPIR-V back end of DXC that SDL_shadercross uses).
generators = {int.from_bytes(d[8:12], "little") >> 16 for _, d in blobs.values()}
assert generators == {14}, generators


def rust_variant(c_name):
    """FRAG_SHADER_TEXTURE_RGBA -> TextureRgba"""
    return "".join(p.capitalize() for p in c_name.split("_")[2:])


def rust_const(c_name):
    """texture_rgba_frag_spv -> TEXTURE_RGBA_FRAG_SPV"""
    return c_name.upper()


def ids_enum(out, rust_name, c_name, ids, what, extra=(), custom=False):
    out.append(f"/// The {what} shaders. Translation of `{c_name}` (`*_INVALID` is")
    out.append("/// `None` where upstream would keep it).")
    out.extend(extra)
    out.append("#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]")
    out.append(f"pub(super) enum {rust_name} {{")
    for i in ids:
        out.append(f"    {rust_variant(i)},")
    out.append("}")
    out.append("")
    out.append(f"impl {rust_name} {{")
    count = re.sub(r"ShaderId$", "", rust_name)
    out.append(f"    /// Translation of `NUM_{'VERT' if count == 'Vertex' else 'FRAG'}_SHADERS`.")
    out.append(f"    pub(super) const COUNT: usize = {len(ids)};")
    out.append("")
    out.append("    /// All the shaders, in order.")
    out.append(f"    pub(super) const ALL: [{rust_name}; {rust_name}::COUNT] = [")
    for i in ids:
        out.append(f"        {rust_name}::{rust_variant(i)},")
    out.append("    ];")
    out.append("")
    sources = "VERT_SHADER_SOURCES" if count == "Vertex" else "FRAG_SHADER_SOURCES"
    if custom:
        # (the custom shader is the last, without an entry)
        out.append("    /// The shader's code and resources (`None` for the custom shader,")
        out.append("    /// which is the application's).")
        out.append("    pub(super) fn sources(self) -> Option<&'static ShaderSources> {")
        out.append(f"        {sources}.get(self as usize)")
    else:
        out.append("    /// The shader's code and resources.")
        out.append("    pub(super) fn sources(self) -> &'static ShaderSources {")
        out.append(f"        &{sources}[self as usize]")
    out.append("    }")
    out.append("}")


def sources_table(out, rust_name, c_name, ids, sources):
    out.append("")
    out.append(f"/// Translation of `{c_name}`.")
    out.append(f"static {rust_name}: [ShaderSources; {len(ids)}] = [")
    for i, (num_samplers, num_uniform_buffers, spirv, dxil) in zip(ids, sources):
        out.append(f"    // {i}")
        out.append("    ShaderSources {")
        out.append(f"        spirv: &{rust_const(spirv)},")
        out.append("        #[cfg(windows)]")
        out.append(f"        dxil60: &{rust_const(dxil)},")
        out.append(f"        num_samplers: {num_samplers},")
        out.append(f"        num_uniform_buffers: {num_uniform_buffers},")
        out.append("    },")
    out.append("];")


out = []
out.append("// Rust translation of the tables of src/render/gpu/SDL_shaders_gpu.c and")
out.append("// SDL_shaders_gpu.h, and the SPIR-V and DXIL headers they include, from")
out.append("// Simple DirectMedia Layer.")
out.append("// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>")
out.append("// This is an altered (translated) version of the original software; see LICENSE.txt.")
out.append("// Generated by tools/gen_gpu_render_shaders.py; do not edit.")
out.append("")
out.append("//! The shaders of the GPU renderer: SPIR-V and DXIL (shader model 6.0)")
out.append("//! compiled from the HLSL sources upstream (`build-shaders.sh`, with")
out.append("//! SDL_shadercross, whose SPIR-V and DXIL come from DXC), one per")
out.append("//! [`VertexShaderId`] and [`FragmentShaderId`]. The functions that create")
out.append("//! them on a device are in the renderer (`CompileShader()`,")
out.append("//! `GPU_InitShaders()`).")
out.append("//!")
out.append("//! The DXIL is here on Windows only, with the Direct3D 12 GPU backend")
out.append("//! (upstream's `SDL_GPU_D3D12`). Upstream also compiles in MSL blobs, with")
out.append("//! its Metal GPU backend; there is none yet, so there are no MSL shaders.")
out.append("")
out.append("/// A shader's code and the resources it uses. Translation of")
out.append("/// `GPU_ShaderSources` (with its SPIR-V and DXIL `GPU_ShaderModuleSource`s).")
out.append("pub(super) struct ShaderSources {")
out.append("    /// The SPIR-V (`SHADER_SPIRV()`).")
out.append("    pub(super) spirv: &'static [u8],")
out.append("    /// The DXIL, shader model 6.0 (`SHADER_DXIL60()`).")
out.append("    #[cfg(windows)]")
out.append("    pub(super) dxil60: &'static [u8],")
out.append("    pub(super) num_samplers: u32,")
out.append("    pub(super) num_uniform_buffers: u32,")
out.append("}")
out.append("")
ids_enum(out, "VertexShaderId", "GPU_VertexShaderID", vert_ids, "vertex")
out.append("")
ids_enum(out, "FragmentShaderId", "GPU_FragmentShaderID", frag_ids, "fragment", [
    "///",
    "/// `FRAG_SHADER_TEXTURE_CUSTOM` is the fragment shader of an",
    "/// `SDL_GPURenderState`, the application's: it has no sources.",
], custom=True)
sources_table(out, "VERT_SHADER_SOURCES", "vert_shader_sources", vert_ids, vert_sources)
sources_table(out, "FRAG_SHADER_SOURCES", "frag_shader_sources", frag_source_ids, frag_sources)


def blob_statics(out, blobs, cfg=None):
    for name, (include, data) in blobs.items():
        out.append("")
        out.append(f"/// `{name}`, from `shaders/{include}`.")
        if cfg:
            out.append(f"#[cfg({cfg})]")
        out.append("#[rustfmt::skip]")
        out.append(f"static {rust_const(name)}: [u8; {len(data)}] = [")
        for i in range(0, len(data), 16):
            out.append("    " + ", ".join(f"0x{b:02x}" for b in data[i:i + 16]) + ",")
        out.append("];")


blob_statics(out, blobs)
blob_statics(out, dxil_blobs, "windows")
print("\n".join(out))
