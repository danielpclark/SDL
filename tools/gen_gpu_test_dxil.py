#!/usr/bin/env python3
"""Generate sdl3/src/gpu/d3d12/test_dxil.rs: small DXIL (shader model 6.0)
shaders for the tests of the Direct3D 12 GPU backend, compiled from the
HLSL below with Microsoft's DirectX Shader Compiler; or, with --dxbc,
sdl3/src/gpu/d3d12/test_dxbc.rs: the same shaders as DXBC (shader model
5.1, which Wine's vkd3d runs), compiled with Microsoft's fxc; or, with
--render, sdl3/src/render/gpu/test_dxil.rs: the DXIL fragment shaders of the
GPU renderer's tests of render states (the shaders of
gen_gpu_test_spirv.py --render).

Usage: gen_gpu_test_dxil.py <dxc command> > sdl3/src/gpu/d3d12/test_dxil.rs
       gen_gpu_test_dxil.py --dxbc <fxc command> > sdl3/src/gpu/d3d12/test_dxbc.rs
       gen_gpu_test_dxil.py --render <dxc command> > sdl3/src/render/gpu/test_dxil.rs

where <dxc command> runs dxc: `dxc` (Linux or Windows), or for instance
`wine64 .../dxc.exe` with the dxc.exe, dxcompiler.dll and dxil.dll of the
Microsoft.Direct3D.DXC NuGet package (dxil.dll signs the shaders, which
Direct3D 12 requires); and <fxc command> runs fxc: `fxc` (Windows), or
`wine64 .../fxc.exe` with the fxc.exe and d3dcompiler_47.dll of the
Microsoft.Windows.SDK.CPP NuGet package (c/bin/<version>/x64; Wine's own
d3dcompiler_47 can't compile shader model 5.1).

The shaders bind their resources where the GPU API's Direct3D 12 backend
puts them: a vertex shader's uniform buffers in space 1, a fragment
shader's samplers and textures in space 2 and uniform buffers in space 3,
a compute shader's read-write storage buffers in space 1 and uniform
buffers in space 2. Vertex inputs use the TEXCOORD semantic. The render
state shaders take what the GPU renderer's triangle vertex shaders output
(`COLOR0` and `TEXCOORD0`), with storage buffers in space 2 after the
textures.
"""
import os
import subprocess
import sys
import tempfile

dxbc = sys.argv[1:2] == ["--dxbc"]
render = sys.argv[1:2] == ["--render"]
compiler = sys.argv[2:] if dxbc or render else sys.argv[1:]
assert compiler, __doc__

# The input of the render state shaders: the output of the GPU renderer's
# tri_color and tri_texture vertex shaders.
PS_INPUT = """
struct PSInput
{
    float4 v_color : COLOR0;
    float2 v_uv : TEXCOORD0;
};
"""

RENDER_SHADERS = [
    (
        "RENDER_TINT",
        "ps_6_0",
        "A render state fragment shader multiplying the color by a uniform.",
        PS_INPUT + """
cbuffer Tint : register(b0, space3)
{
    float4 tint;
};

float4 main(PSInput input) : SV_Target
{
    return input.v_color * tint;
}
""",
    ),
    (
        "RENDER_TWO_TEXTURES",
        "ps_6_0",
        "A render state fragment shader adding a second texture (sampler 1) scaled by a\n"
        "/// uniform (slot 1) to the texture drawn (sampler 0), modulated and tinted by a\n"
        "/// uniform (slot 0).",
        PS_INPUT + """
Texture2D u_texture : register(t0, space2);
SamplerState u_sampler : register(s0, space2);
Texture2D u_extra : register(t1, space2);
SamplerState u_extra_sampler : register(s1, space2);

cbuffer Tint : register(b0, space3)
{
    float4 tint;
};

cbuffer Scale : register(b1, space3)
{
    float4 scale;
};

float4 main(PSInput input) : SV_Target
{
    return u_texture.Sample(u_sampler, input.v_uv) * input.v_color * tint
        + u_extra.Sample(u_extra_sampler, input.v_uv) * scale;
}
""",
    ),
    (
        "RENDER_STORAGE_BUFFER",
        "ps_6_0",
        "A render state fragment shader multiplying the color by the first `float4` of a\n"
        "/// storage buffer.",
        PS_INPUT + """
StructuredBuffer<float4> colors : register(t0, space2);

float4 main(PSInput input) : SV_Target
{
    return input.v_color * colors[0];
}
""",
    ),
]

SHADERS = [
    (
        "VERTEX",
        "vs_6_0",
        "A vertex shader with a `float2` position and a `float4` color input\n"
        "/// (`TEXCOORD0`, `TEXCOORD1`) and a uniform buffer.",
        """
cbuffer VertexUniforms : register(b0, space1)
{
    float4 offset;
};

struct VSOut
{
    float4 color : TEXCOORD0;
    float4 pos : SV_Position;
};

VSOut main(float2 position : TEXCOORD0, float4 color : TEXCOORD1)
{
    VSOut o;
    o.color = color;
    o.pos = float4(position, 0.0, 1.0) + offset;
    return o;
}
""",
    ),
    (
        "FRAGMENT_SAMPLER",
        "ps_6_0",
        "A fragment shader sampling a texture (one sampler) and with a uniform\n"
        "/// buffer.",
        """
cbuffer FragmentUniforms : register(b0, space3)
{
    float4 tint;
};

Texture2D Tex : register(t0, space2);
SamplerState Smp : register(s0, space2);

float4 main(float4 color : TEXCOORD0) : SV_Target0
{
    return color * tint * Tex.Sample(Smp, color.xy);
}
""",
    ),
    (
        "FRAGMENT_SOLID",
        "ps_6_0",
        "A fragment shader without resources.",
        """
float4 main(float4 color : TEXCOORD0) : SV_Target0
{
    return color;
}
""",
    ),
    (
        "COMPUTE",
        "cs_6_0",
        "A compute shader (64 threads per group) writing a read-write storage\n"
        "/// buffer, with a uniform buffer.",
        """
cbuffer ComputeUniforms : register(b0, space2)
{
    uint scale;
};

RWByteAddressBuffer Output : register(u0, space1);

[numthreads(64, 1, 1)]
void main(uint3 id : SV_DispatchThreadID)
{
    Output.Store(id.x * 4, id.x * scale);
}
""",
    ),
]

out = []
out.append("// Generated by tools/gen_gpu_test_dxil.py; do not edit.")
out.append("// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>")
out.append("// This is an altered (translated) version of the original software; see LICENSE.txt.")
out.append("")
if dxbc:
    out.append("//! DXBC (shader model 5.1) shaders for the tests of the Direct3D 12")
    out.append("//! backend, compiled by Microsoft's fxc (the HLSL of `test_dxil`).")
else:
    version = subprocess.run(compiler + ["--version"], capture_output=True, text=True, check=True)
    version = version.stdout.strip().splitlines()[0]
    if render:
        out.append("//! DXIL (shader model 6.0) fragment shaders for the tests of the GPU")
        out.append("//! renderer's render states, compiled by dxc:")
    else:
        out.append("//! DXIL (shader model 6.0) shaders for the tests of the Direct3D 12")
        out.append("//! backend, compiled by dxc:")
    out.append(f"//! {version}.")
with tempfile.TemporaryDirectory() as tmp:
    for name, profile, doc, source in RENDER_SHADERS if render else SHADERS:
        with open(os.path.join(tmp, "shader.hlsl"), "w") as f:
            f.write(source)
        if dxbc:
            profile = profile.replace("_6_0", "_5_1")
            args = ["/nologo", "/T", profile, "/E", "main", "/Fo", "shader.dxbc", "shader.hlsl"]
        else:
            args = ["-T", profile, "-E", "main", "-Fo", "shader.dxbc", "shader.hlsl"]
        subprocess.run(
            compiler + args,
            cwd=tmp,
            check=True,
            stdout=subprocess.DEVNULL,
        )
        data = open(os.path.join(tmp, "shader.dxbc"), "rb").read()
        assert data[:4] == b"DXBC", name
        assert int.from_bytes(data[24:28], "little") == len(data), name
        assert any(data[4:20]), f"{name}: not signed (dxil.dll missing?)"
        chunks = [data[o:o + 4] for o in (int.from_bytes(data[32 + 4 * i:36 + 4 * i], "little")
                                          for i in range(int.from_bytes(data[28:32], "little")))]
        assert (b"SHEX" in chunks) == dxbc and (b"DXIL" in chunks) != dxbc, (name, chunks)
        out.append("")
        out.append(f"/// {doc}")
        out.append("#[rustfmt::skip]")
        out.append(f"pub(super) static {name}: [u8; {len(data)}] = [")
        for i in range(0, len(data), 16):
            out.append("    " + ", ".join(f"0x{b:02x}" for b in data[i:i + 16]) + ",")
        out.append("];")
print("\n".join(out))
