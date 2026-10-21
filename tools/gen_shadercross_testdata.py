#!/usr/bin/env python3
"""Generate sdl3-shadercross/testdata/*.spv: small SPIR-V shaders for the
tests of sdl3-shadercross, compiled from the GLSL sources below with
glslang (and, for the "_opt" variants, optimized with spirv-opt, which
turns locals into OpPhi and restructures the control flow).

Usage: gen_shadercross_testdata.py GLSLANG SPIRV_OPT
       gen_shadercross_testdata.py --assembled

GLSLANG and SPIRV_OPT are the paths of the glslang and spirv-opt
executables (the tests' reference outputs were made with glslang 16.6.0 and
the SPIRV-Tools of the same release). With --assembled, it only writes the
hand-assembled modules (below), which need neither.

It also writes malformed variants of some of them ("*_bad_*.spv",
"*_ub_*.spv", "*_guard_*.spv" and "*_hang_*.spv", listed in MALFORMED
below): truncated, with a broken header, an instruction claiming zero or
too many words, out-of-range IDs and member indices, and flipped bytes.
The reference has SDL_shadercross's errors (or output) for the "bad"
ones too. And it writes modules it assembles itself ("*_asm_*.spv", listed
in ASSEMBLED), for what glslang doesn't emit: each reaches an upstream
SPIRV-Cross bug sdl3-shadercross fixes.

The shaders bind their resources where SDL's GPU API wants them (see
SDL_CreateGPUShader): a vertex shader's sampled textures, storage textures
and storage buffers in set 0 and uniform buffers in set 1, a fragment
shader's in sets 2 and 3, and a compute shader's sampled textures,
read-only storage textures and read-only storage buffers in set 0,
read-write storage textures and buffers in set 1 and uniform buffers in
set 2. Between them they use uniform, storage and push constant blocks
with arrays, structs and matrices, combined samplers of the different
dimensions, storage images, atomics, shared memory and barriers, and
loops, switches, discards and function calls.

The reference outputs in sdl3-shadercross/testdata/reference.txt come
from SDL_shadercross and its SPIRV-Cross (built from the commits the
crate translates) run on these files, with SDL_shadercross compiled with
-ftrivial-auto-var-init=zero: its compute path leaves the MSL indices of
the resource bindings that don't use them uninitialized (the translation
zeroes them too). Both are patched where sdl3-shadercross fixes their
bugs (the `Note (upstream)` notes that say so): one of those
uninitialized indices, the buffer of an emulated image atomic, reached
the output.
"""
import os
import struct
import subprocess
import sys
import tempfile

SHADERS = []


def shader(name, stage, source, optimize=False):
    SHADERS.append((name, stage, source, optimize))


shader('vs_basic', 'vert', '''
#version 450
layout(location = 0) in vec3 in_position;
layout(location = 1) in vec4 in_color;
layout(location = 2) in vec2 in_uv;
layout(location = 0) out vec4 out_color;
layout(location = 1) out vec2 out_uv;
layout(set = 1, binding = 0) uniform Transform {
    mat4 mvp;
    vec4 tint;
} transform;
void main()
{
    out_color = in_color * transform.tint;
    out_uv = in_uv;
    gl_Position = transform.mvp * vec4(in_position, 1.0);
}
''')

shader('vs_storage', 'vert', '''
#version 450
struct Sprite {
    vec2 position;
    vec2 size;
    vec4 color;
    uint flags;
    float rotation;
};
layout(set = 0, binding = 0) uniform sampler2D heightmap;
layout(set = 0, binding = 1, std430) readonly buffer Sprites {
    Sprite sprites[];
};
layout(set = 1, binding = 0) uniform Camera {
    mat4 view_projection;
    vec4 corners[4];
    int mode;
} camera;
layout(location = 0) out vec4 out_color;
layout(location = 1) flat out uint out_flags;
layout(location = 2) out vec2 out_uv;
void main()
{
    Sprite s = sprites[gl_InstanceIndex];
    vec2 corner = camera.corners[gl_VertexIndex % 4].xy;
    float c = cos(s.rotation);
    float n = sin(s.rotation);
    vec2 p = vec2(corner.x * c - corner.y * n, corner.x * n + corner.y * c) * s.size + s.position;
    float h = textureLod(heightmap, p * 0.01, 0.0).r;
    if (camera.mode == 1) {
        h *= 2.0;
    } else if (camera.mode > 1) {
        h = -h;
    }
    out_color = s.color;
    out_flags = s.flags | uint(gl_VertexIndex << 16);
    out_uv = corner * 0.5 + 0.5;
    gl_Position = camera.view_projection * vec4(p, h, 1.0);
    gl_PointSize = 1.0;
}
''')

shader('vs_push', 'vert', '''
#version 450
layout(push_constant) uniform Push {
    mat2 rotation;
    vec2 offset;
    float scale;
} push;
layout(location = 0) in vec2 in_position;
layout(location = 1) in ivec2 in_index;
layout(location = 0) out vec2 out_position;
layout(location = 1) flat out ivec2 out_index;
void main()
{
    out_position = push.rotation * in_position * push.scale + push.offset;
    out_index = in_index * 2 + ivec2(1, -1);
    gl_Position = vec4(out_position, 0.0, 1.0);
}
''')

shader('fs_textured', 'frag', '''
#version 450
layout(set = 2, binding = 0) uniform sampler2D albedo;
layout(set = 2, binding = 1) uniform samplerCube environment;
layout(set = 3, binding = 0) uniform Material {
    vec4 base_color;
    vec3 light_dir;
    float roughness;
    int sample_count;
    float alpha_cutoff;
} material;
layout(location = 0) in vec4 in_color;
layout(location = 1) in vec2 in_uv;
layout(location = 0) out vec4 out_color;
void main()
{
    vec4 color = texture(albedo, in_uv) * in_color * material.base_color;
    if (color.a < material.alpha_cutoff) {
        discard;
    }
    vec3 env = vec3(0.0);
    for (int i = 0; i < material.sample_count; i++) {
        vec3 dir = normalize(material.light_dir + vec3(float(i) * 0.1, 0.0, 0.0));
        env += texture(environment, dir).rgb;
    }
    if (material.sample_count > 0) {
        env /= float(material.sample_count);
    } else {
        env = vec3(1.0);
    }
    out_color = vec4(mix(color.rgb, env, material.roughness), color.a);
}
''')

COMPLEX_FRAG = '''
#version 450
struct Light {
    vec3 position;
    float radius;
    vec4 color;
};
layout(set = 2, binding = 0) uniform sampler2DArray layers;
layout(set = 2, binding = 1) uniform sampler2DShadow shadow_map;
layout(set = 2, binding = 2) uniform sampler2D lookup;
layout(set = 3, binding = 0) uniform Scene {
    Light lights[4];
    mat3 normal_matrix;
    ivec4 counts;
    vec2 shadow_offsets[3];
} scene;
layout(set = 3, binding = 1) uniform Extra {
    uvec2 mask;
    float bias;
} extra;
layout(location = 0) in vec3 in_normal;
layout(location = 1) in vec3 in_world;
layout(location = 2) flat in int in_layer;
layout(location = 3) in vec4 in_shadow_coord;
layout(location = 0) out vec4 out_color;
layout(location = 1) out vec4 out_normal;

float attenuate(Light light, vec3 p, inout float count)
{
    float d = distance(light.position, p);
    if (d > light.radius) {
        return 0.0;
    }
    count += 1.0;
    return 1.0 - smoothstep(0.0, light.radius, d);
}

float shadow(vec4 coord)
{
    float sum = 0.0;
    for (int i = 0; i < 3; i++) {
        vec3 c = coord.xyz / coord.w;
        c.xy += scene.shadow_offsets[i];
        sum += texture(shadow_map, vec3(c.xy, c.z - extra.bias));
    }
    return sum / 3.0;
}

void main()
{
    vec3 n = normalize(scene.normal_matrix * in_normal);
    vec3 lit = vec3(0.0);
    float count = 0.0;
    int i = 0;
    while (true) {
        if (i >= scene.counts.x) {
            break;
        }
        if ((extra.mask.x & (1u << uint(i))) == 0u) {
            i++;
            continue;
        }
        Light l = scene.lights[i];
        float a = attenuate(l, in_world, count);
        lit += l.color.rgb * a * max(dot(n, normalize(l.position - in_world)), 0.0);
        i++;
    }
    vec4 base;
    switch (in_layer) {
    case 0:
        base = texture(layers, vec3(in_world.xy, 0.0));
        break;
    case 1:
    case 2:
        base = texture(layers, vec3(in_world.xy, float(in_layer)));
        break;
    case 3:
        base = texelFetch(lookup, ivec2(gl_FragCoord.xy) % textureSize(lookup, 0), 0);
        break;
    default:
        base = vec4(1.0, 0.0, 1.0, 1.0);
        break;
    }
    float s = shadow(in_shadow_coord);
    float edge = fwidth(in_world.x) + abs(dFdy(in_world.y));
    out_color = vec4(base.rgb * lit * s + edge, count / 4.0);
    out_normal = vec4(n * (gl_FrontFacing ? 0.5 : -0.5) + 0.5, 1.0);
}
'''

shader('fs_complex', 'frag', COMPLEX_FRAG)
shader('fs_complex_opt', 'frag', COMPLEX_FRAG, optimize=True)

shader('fs_storage', 'frag', '''
#version 450
layout(set = 2, binding = 0) uniform sampler2D noise;
layout(set = 2, binding = 1, rgba8) uniform readonly image2D palette;
layout(set = 2, binding = 2, std430) readonly buffer Weights {
    float weights[16];
    uint count;
} weights;
layout(set = 3, binding = 0) uniform Params {
    vec4 scale;
    float depth_bias;
} params;
layout(location = 0) in vec2 in_uv;
layout(location = 0) out vec4 out_color;
void main()
{
    ivec2 size = imageSize(palette);
    vec4 sum = vec4(0.0);
    for (uint i = 0u; i < min(weights.count, 16u); i++) {
        ivec2 p = ivec2(int(i) % size.x, int(i) / size.x);
        sum += imageLoad(palette, p) * weights.weights[i];
    }
    sum *= texture(noise, in_uv * params.scale.xy);
    out_color = clamp(sum, vec4(0.0), vec4(1.0));
    gl_FragDepth = gl_FragCoord.z + params.depth_bias;
}
''')

BASIC_COMP = '''
#version 450
layout(local_size_x = 64, local_size_y = 1, local_size_z = 1) in;
layout(set = 0, binding = 0, std430) readonly buffer Input {
    vec4 values[];
} input_data;
layout(set = 1, binding = 0, std430) buffer Output {
    uint total;
    uint histogram[8];
    vec4 values[];
} output_data;
layout(set = 2, binding = 0) uniform Params {
    uint count;
    float threshold;
    vec2 range;
} params;
shared vec4 scratch[64];
void main()
{
    uint id = gl_GlobalInvocationID.x;
    uint local = gl_LocalInvocationIndex;
    vec4 v = id < params.count ? input_data.values[id] : vec4(0.0);
    scratch[local] = v;
    barrier();
    for (uint stride = 32u; stride > 0u; stride >>= 1) {
        if (local < stride) {
            scratch[local] += scratch[local + stride];
        }
        memoryBarrierShared();
        barrier();
    }
    if (local == 0u) {
        output_data.values[gl_WorkGroupID.x] = scratch[0];
    }
    if (v.x > params.threshold) {
        atomicAdd(output_data.total, 1u);
        uint bucket = uint(clamp((v.x - params.range.x) / (params.range.y - params.range.x), 0.0, 0.999) * 8.0);
        atomicMax(output_data.histogram[bucket], id);
    }
}
'''

shader('cs_basic', 'comp', BASIC_COMP)
shader('cs_basic_opt', 'comp', BASIC_COMP, optimize=True)

shader('cs_image', 'comp', '''
#version 450
layout(local_size_x = 8, local_size_y = 8, local_size_z = 1) in;
layout(set = 0, binding = 0) uniform sampler2D source;
layout(set = 0, binding = 1, r32f) uniform readonly image2D mask;
layout(set = 1, binding = 0, rgba32f) uniform writeonly image2D destination;
layout(set = 1, binding = 1, r32ui) uniform uimage2D counters;
layout(set = 2, binding = 0) uniform Params {
    ivec2 offset;
    uint flags;
    float gain;
} params;
void main()
{
    ivec2 p = ivec2(gl_GlobalInvocationID.xy);
    ivec2 size = imageSize(counters);
    if (any(greaterThanEqual(p, size))) {
        return;
    }
    vec4 c = texelFetch(source, p + params.offset, 0);
    float m = imageLoad(mask, p).r;
    uint bits = bitCount(params.flags) + uint(findMSB(params.flags));
    uint packed = packUnorm4x8(c);
    c = unpackUnorm4x8(packed ^ (params.flags << 8)) * m * params.gain;
    imageStore(destination, p, c + float(bits));
    imageAtomicAdd(counters, p, bits);
}
''')

shader('cs_math', 'comp', '''
#version 450
layout(local_size_x = 4, local_size_y = 4, local_size_z = 2) in;
layout(set = 1, binding = 0, std430) buffer Matrices {
    mat4 m[];
} matrices;
layout(set = 2, binding = 0) uniform Params {
    mat3 basis;
    ivec4 divisors;
    uvec4 bits;
} params;
mat4 build(uint i)
{
    mat4 r = matrices.m[i];
    r = transpose(r) * inverse(r + mat4(1.0));
    r[3] = vec4(outerProduct(params.basis[0], params.basis[1]) * vec3(determinant(r)), 1.0);
    return r;
}
void main()
{
    uint i = gl_GlobalInvocationID.x + gl_GlobalInvocationID.y * 4u + gl_GlobalInvocationID.z * 16u;
    mat4 r = build(i);
    ivec4 q = ivec4(r[0] * 100.0);
    q = q / params.divisors + q % params.divisors;
    uvec4 u = (params.bits >> uvec4(1, 2, 3, 4)) | (params.bits << 3);
    r[1] = vec4(q) + vec4(u) * pow(abs(r[2]), vec4(1.5)) + exp2(r[0]) - log(abs(r[3]) + 1.0);
    r[2] = fract(r[2]) + step(0.5, r[1]) + sign(r[0]) * inversesqrt(abs(r[1]) + 1.0);
    matrices.m[i] = r;
}
''')

shader('cs_layout', 'comp', '''
#version 450
layout(local_size_x = 16, local_size_y = 1, local_size_z = 1) in;
struct Item {
    vec3 position;
    float weight;
    mat3x4 frame;
    uint tags[3];
};
layout(set = 0, binding = 0) uniform sampler2DArray layers;
layout(set = 0, binding = 1, std430) readonly buffer Source {
    Item items[];
} source;
layout(set = 1, binding = 0, std430, row_major) buffer Destination {
    Item best;
    Item items[];
} destination;
layout(set = 2, binding = 0) uniform Params {
    uint count;
    float eta;
    vec2 scale;
} params;
void main()
{
    uint i = gl_GlobalInvocationID.x;
    Item a = source.items[i];
    Item b = source.items[(i + 1u) % params.count];
    Item c = a.weight > b.weight ? a : b;
    ivec3 size = textureSize(layers, 0);
    int levels = textureQueryLevels(layers);
    c.tags[0] = bitfieldInsert(c.tags[0], uint(levels), 4, 4) + bitfieldExtract(c.tags[1], 2, 6);
    c.tags[2] = packHalf2x16(params.scale * vec2(size.xy)) ^ uint(findLSB(c.tags[2]));
    c.weight = fma(refract(c.weight, 1.0, params.eta), mod(c.weight, 3.0), float(size.z));
    c.position = vec4(c.position, 1.0) * c.frame;
    destination.items[i] = c;
    if (i == 0u) {
        destination.best = c;
    }
}
''')

shader('fs_sampling', 'frag', '''
#version 450
layout(set = 2, binding = 0) uniform sampler2D color;
layout(set = 2, binding = 1) uniform sampler2DShadow shadow;
layout(set = 2, binding = 2) uniform samplerCube environment;
layout(set = 3, binding = 0) uniform Params {
    vec4 tint;
    float lod;
    float bias;
} params;
layout(location = 0) in vec2 in_uv;
layout(location = 1) in vec3 in_normal;
layout(location = 2) flat in int in_layer;
layout(location = 0) out vec4 out_color;
void main()
{
    vec2 dx = dFdx(in_uv);
    vec2 dy = dFdy(in_uv);
    vec4 c = textureGrad(color, in_uv, dx, dy) + textureLod(color, in_uv, params.lod);
    c += texture(color, in_uv, params.bias) + textureOffset(color, in_uv, ivec2(1, -1));
    c *= textureGather(color, in_uv, 1);
    float s = texture(shadow, vec3(in_uv, fwidth(in_uv.x)));
    vec4 e = texture(environment, reflect(normalize(in_normal), vec3(0.0, 0.0, 1.0)));
    if (c.a < 0.01 && in_layer > 2) {
        discard;
    }
    out_color = mix(c, e, s) * params.tint;
}
''')


def words(data):
    return [int.from_bytes(data[i:i + 4], 'little') for i in range(0, len(data) - len(data) % 4, 4)]


def pack(ws):
    return b''.join(w.to_bytes(4, 'little') for w in ws)


def instruction_offsets(ws):
    """The word offsets of the instructions after the 5-word header."""
    out, i = [], 5
    while i < len(ws):
        out.append(i)
        n = ws[i] >> 16
        if n == 0:
            break
        i += n
    return out


def set_word(ws, index, value):
    ws = list(ws)
    ws[index] = value
    return ws


def first_op(ws, opcode):
    return next(i for i in instruction_offsets(ws) if ws[i] & 0xffff == opcode)


def nth_op(ws, n):
    return instruction_offsets(ws)[n]


def flip(data, positions, mask):
    return bytes(b ^ mask if i in positions else b for i, b in enumerate(data))


# (name, source shader, transform of its bytes)
MALFORMED = [
    # Truncated in the middle of an instruction and of the header.
    ('vs_bad_truncated', 'vs_basic', lambda d: d[:len(d) * 3 // 5]),
    ('fs_bad_header', 'fs_textured', lambda d: d[:18]),
    # A wrong magic number and a version from the future.
    ('cs_bad_magic', 'cs_basic', lambda d: pack(set_word(words(d), 0, 0x03022307))),
    ('vs_bad_version', 'vs_push', lambda d: pack(set_word(words(d), 1, 0x00ff0000))),
    # An instruction with a zero word count, and one running past the end.
    ('fs_bad_zero_count', 'fs_storage',
     lambda d: pack(set_word(words(d), nth_op(words(d), 12), words(d)[nth_op(words(d), 12)] & 0xffff))),
    ('cs_bad_long_count', 'cs_image',
     lambda d: pack(set_word(words(d), nth_op(words(d), 30), words(d)[nth_op(words(d), 30)] | 0xfff00000))),
    # An OpMemberDecorate with a member index far past any struct: SPIRV-Cross
    # allocates decorations for that many members (gigabytes), where the
    # translation stops with an error; not in the reference either.
    ('cs_guard_member_index', 'cs_math',
     lambda d: pack(set_word(words(d), first_op(words(d), 72) + 2, 0x00f00000))),
    # Byte flips in the function bodies.
    ('fs_bad_flip', 'fs_complex', lambda d: flip(d, (len(d) * 5 // 8,), 0x10)),
    ('cs_bad_flip', 'cs_layout', lambda d: flip(d, (len(d) * 3 // 5,), 0x08)),
    ('cs_bad_flip2', 'cs_layout', lambda d: flip(d, (len(d) * 5 // 6,), 0x01)),
    ('cs_bad_flip3', 'cs_layout', lambda d: flip(d, (len(d) * 11 // 20,), 0x20)),
    # Where SDL_shadercross's SPIRV-Cross reads out of bounds, so that what
    # it does depends on the heap: two flips that crash it (in the parser
    # and in the GLSL backend) and an OpEntryPoint naming a function ID past
    # the bound. Not in the reference: the tests check that the translation
    # fails cleanly.
    ('fs_ub_flip', 'fs_complex', lambda d: flip(d, (len(d) // 2, len(d) * 3 // 4), 0x5a)),
    ('cs_ub_flip', 'cs_layout', lambda d: flip(d, (len(d) * 2 // 3,), 0x01)),
    ('vs_ub_entry_id', 'vs_storage',
     lambda d: pack(set_word(words(d), first_op(words(d), 15) + 2, words(d)[3] + 5))),
    # A flip that sends a block chain back to an earlier block without a
    # loop header, where SDL_shadercross loops forever; the translation
    # stops with an error. Not in the reference.
    ('fs_hang_block_chain', 'fs_complex', lambda d: flip(d, (4588,), 0x02)),
]


# Hand-assembled modules ("*_asm_*.spv"), for what glslang doesn't emit (or
# not without options this script doesn't pass), each reaching an upstream
# SPIRV-Cross bug sdl3-shadercross fixes. Written by a small assembler: an
# operand is a word, an '%id' (numbered in order of first use), a string,
# or a float.

OPCODES = {
    'OpUndef': 1, 'OpName': 5, 'OpMemberName': 6, 'OpExtension': 10, 'OpExtInstImport': 11,
    'OpExtInst': 12, 'OpMemoryModel': 14, 'OpEntryPoint': 15, 'OpExecutionMode': 16,
    'OpCapability': 17, 'OpTypeVoid': 19, 'OpTypeInt': 21, 'OpTypeFloat': 22,
    'OpTypeVector': 23, 'OpTypeImage': 25, 'OpTypeSampledImage': 27, 'OpTypeArray': 28,
    'OpTypeStruct': 30, 'OpTypePointer': 32, 'OpTypeFunction': 33, 'OpConstant': 43,
    'OpConstantComposite': 44, 'OpFunction': 54, 'OpFunctionEnd': 56, 'OpVariable': 59,
    'OpLoad': 61, 'OpStore': 62, 'OpAccessChain': 65, 'OpDecorate': 71,
    'OpMemberDecorate': 72, 'OpCompositeConstruct': 80, 'OpImageSampleImplicitLod': 87,
    'OpFAdd': 129, 'OpLabel': 248, 'OpReturn': 253,
}

# (enumerants used below)
SHADER, PHYSICAL_STORAGE_BUFFER_ADDRESSES = 1, 5347
LOGICAL, PHYSICAL_STORAGE_BUFFER_64, GLSL450 = 0, 5348, 1
VERTEX, FRAGMENT, GL_COMPUTE = 0, 4, 5
ORIGIN_UPPER_LEFT, LOCAL_SIZE = 7, 17
UNIFORM_CONSTANT, INPUT, UNIFORM, OUTPUT, PHYSICAL_STORAGE_BUFFER = 0, 1, 2, 3, 5349
BLOCK, BUFFER_BLOCK, ARRAY_STRIDE, LOCATION, BINDING, DESCRIPTOR_SET, OFFSET = 2, 3, 6, 30, 33, 34, 35
ALIGNED = 2
DIM_2D, DIM_3D = 1, 2
GLSL_STD_450_ASINH = 22


def assemble(lines):
    """A module from its instructions: (opcode name, operands...)."""
    ids = {}

    def word_list(operand):
        if isinstance(operand, str) and operand.startswith('%'):
            return [ids.setdefault(operand, len(ids) + 1)]
        if isinstance(operand, str):
            b = operand.encode() + b'\0'
            b += b'\0' * (-len(b) % 4)
            return words(b)
        if isinstance(operand, float):
            return words(struct.pack('<f', operand))
        return [operand]

    body = []
    for name, *operands in lines:
        ws = [w for o in operands for w in word_list(o)]
        body += [(len(ws) + 1) << 16 | OPCODES[name]] + ws
    return [0x07230203, 0x00010000, 0, len(ids) + 1, 0] + body


def compute_header(*capabilities):
    return [('OpCapability', SHADER)] + [('OpCapability', c) for c in capabilities]


ASSEMBLED = []

# A void OpUndef (as DXC's debug information has), before the struct a
# uniform block holds: SPIRV-Cross's HLSL and MSL backends stopped
# declaring the structs and constants after it.
ASSEMBLED.append(('cs_asm_void_undef', compute_header() + [
    ('OpMemoryModel', LOGICAL, GLSL450),
    ('OpEntryPoint', GL_COMPUTE, '%main', 'main'),
    ('OpExecutionMode', '%main', LOCAL_SIZE, 1, 1, 1),
    ('OpName', '%main', 'main'),
    ('OpName', '%S', 'S'), ('OpMemberName', '%S', 0, 'x'),
    ('OpName', '%Params', 'Params'), ('OpMemberName', '%Params', 0, 's'),
    ('OpName', '%params', 'params'),
    ('OpName', '%Out', 'Out'), ('OpMemberName', '%Out', 0, 'value'),
    ('OpName', '%out', 'out_data'),
    ('OpMemberDecorate', '%S', 0, OFFSET, 0),
    ('OpMemberDecorate', '%Params', 0, OFFSET, 0),
    ('OpDecorate', '%Params', BLOCK),
    ('OpDecorate', '%params', DESCRIPTOR_SET, 2), ('OpDecorate', '%params', BINDING, 0),
    ('OpMemberDecorate', '%Out', 0, OFFSET, 0),
    ('OpDecorate', '%Out', BUFFER_BLOCK),
    ('OpDecorate', '%out', DESCRIPTOR_SET, 1), ('OpDecorate', '%out', BINDING, 0),
    ('OpTypeVoid', '%void'),
    ('OpTypeFunction', '%fn', '%void'),
    ('OpUndef', '%void', '%undef'),
    ('OpTypeFloat', '%float', 32),
    ('OpTypeStruct', '%S', '%float'),
    ('OpTypeStruct', '%Params', '%S'),
    ('OpTypePointer', '%ptr_params', UNIFORM, '%Params'),
    ('OpVariable', '%ptr_params', '%params', UNIFORM),
    ('OpTypeStruct', '%Out', '%float'),
    ('OpTypePointer', '%ptr_out', UNIFORM, '%Out'),
    ('OpVariable', '%ptr_out', '%out', UNIFORM),
    ('OpTypeInt', '%int', 32, 1),
    ('OpConstant', '%int', '%c0', 0),
    ('OpTypePointer', '%ptr_float', UNIFORM, '%float'),
    ('OpFunction', '%void', '%main', 0, '%fn'),
    ('OpLabel', '%entry'),
    ('OpAccessChain', '%ptr_float', '%p', '%params', '%c0', '%c0'),
    ('OpLoad', '%float', '%v', '%p'),
    ('OpAccessChain', '%ptr_float', '%q', '%out', '%c0'),
    ('OpStore', '%q', '%v'),
    ('OpReturn',),
    ('OpFunctionEnd',),
]))

# A whole array stored into a storage buffer: SPIRV-Cross's HLSL backend
# took the array's base type enumerator for the ID of its type (and looped
# forever on this one).
ASSEMBLED.append(('cs_asm_array_store', compute_header() + [
    ('OpMemoryModel', LOGICAL, GLSL450),
    ('OpEntryPoint', GL_COMPUTE, '%main', 'main'),
    ('OpExecutionMode', '%main', LOCAL_SIZE, 1, 1, 1),
    ('OpName', '%main', 'main'),
    ('OpName', '%Out', 'Out'), ('OpMemberName', '%Out', 0, 'values'),
    ('OpName', '%out', 'out_data'),
    ('OpDecorate', '%arr', ARRAY_STRIDE, 4),
    ('OpMemberDecorate', '%Out', 0, OFFSET, 0),
    ('OpDecorate', '%Out', BUFFER_BLOCK),
    ('OpDecorate', '%out', DESCRIPTOR_SET, 1), ('OpDecorate', '%out', BINDING, 0),
    ('OpTypeVoid', '%void'),
    ('OpTypeFunction', '%fn', '%void'),
    ('OpTypeFloat', '%float', 32),
    ('OpTypeInt', '%uint', 32, 0),
    ('OpConstant', '%uint', '%c4', 4),
    ('OpTypeArray', '%arr', '%float', '%c4'),
    ('OpTypeStruct', '%Out', '%arr'),
    ('OpTypePointer', '%ptr_out', UNIFORM, '%Out'),
    ('OpVariable', '%ptr_out', '%out', UNIFORM),
    ('OpTypeInt', '%int', 32, 1),
    ('OpConstant', '%int', '%c0', 0),
    ('OpTypePointer', '%ptr_arr', UNIFORM, '%arr'),
    ('OpConstant', '%float', '%f1', 1.0),
    ('OpConstant', '%float', '%f2', 2.0),
    ('OpConstant', '%float', '%f3', 3.0),
    ('OpConstant', '%float', '%f4', 4.0),
    ('OpConstantComposite', '%arr', '%values', '%f1', '%f2', '%f3', '%f4'),
    ('OpFunction', '%void', '%main', 0, '%fn'),
    ('OpLabel', '%entry'),
    ('OpAccessChain', '%ptr_arr', '%q', '%out', '%c0'),
    ('OpStore', '%q', '%values'),
    ('OpReturn',),
    ('OpFunctionEnd',),
]))

# Buffer-device-address pointers to two-dimensional arrays (float[3][4] and
# float[3][2]): SPIRV-Cross's GLSL backend named the pointer types after
# their outer dimension twice (float3_stride_163_stride_4Pointer).
ASSEMBLED.append(('cs_asm_pointer_arrays', compute_header(PHYSICAL_STORAGE_BUFFER_ADDRESSES) + [
    ('OpExtension', 'SPV_KHR_physical_storage_buffer'),
    ('OpMemoryModel', PHYSICAL_STORAGE_BUFFER_64, GLSL450),
    ('OpEntryPoint', GL_COMPUTE, '%main', 'main'),
    ('OpExecutionMode', '%main', LOCAL_SIZE, 1, 1, 1),
    ('OpName', '%main', 'main'),
    ('OpName', '%Params', 'Params'),
    ('OpMemberName', '%Params', 0, 'a'), ('OpMemberName', '%Params', 1, 'b'),
    ('OpName', '%params', 'params'),
    ('OpDecorate', '%a4', ARRAY_STRIDE, 4),
    ('OpDecorate', '%a34', ARRAY_STRIDE, 16),
    ('OpDecorate', '%a2', ARRAY_STRIDE, 4),
    ('OpDecorate', '%a32', ARRAY_STRIDE, 8),
    ('OpMemberDecorate', '%Params', 0, OFFSET, 0),
    ('OpMemberDecorate', '%Params', 1, OFFSET, 8),
    ('OpDecorate', '%Params', BLOCK),
    ('OpDecorate', '%params', DESCRIPTOR_SET, 2), ('OpDecorate', '%params', BINDING, 0),
    ('OpTypeVoid', '%void'),
    ('OpTypeFunction', '%fn', '%void'),
    ('OpTypeFloat', '%float', 32),
    ('OpTypeInt', '%uint', 32, 0),
    ('OpConstant', '%uint', '%c2', 2),
    ('OpConstant', '%uint', '%c3', 3),
    ('OpConstant', '%uint', '%c4', 4),
    ('OpTypeArray', '%a4', '%float', '%c4'),
    ('OpTypeArray', '%a34', '%a4', '%c3'),
    ('OpTypeArray', '%a2', '%float', '%c2'),
    ('OpTypeArray', '%a32', '%a2', '%c3'),
    ('OpTypePointer', '%ptr_a', PHYSICAL_STORAGE_BUFFER, '%a34'),
    ('OpTypePointer', '%ptr_b', PHYSICAL_STORAGE_BUFFER, '%a32'),
    ('OpTypeStruct', '%Params', '%ptr_a', '%ptr_b'),
    ('OpTypePointer', '%ptr_params', UNIFORM, '%Params'),
    ('OpVariable', '%ptr_params', '%params', UNIFORM),
    ('OpTypeInt', '%int', 32, 1),
    ('OpConstant', '%int', '%i0', 0),
    ('OpConstant', '%int', '%i1', 1),
    ('OpConstant', '%int', '%i2', 2),
    ('OpTypePointer', '%ptr_u_a', UNIFORM, '%ptr_a'),
    ('OpTypePointer', '%ptr_u_b', UNIFORM, '%ptr_b'),
    ('OpTypePointer', '%ptr_float', PHYSICAL_STORAGE_BUFFER, '%float'),
    ('OpFunction', '%void', '%main', 0, '%fn'),
    ('OpLabel', '%entry'),
    ('OpAccessChain', '%ptr_u_a', '%pa', '%params', '%i0'),
    ('OpLoad', '%ptr_a', '%a', '%pa'),
    ('OpAccessChain', '%ptr_u_b', '%pb', '%params', '%i1'),
    ('OpLoad', '%ptr_b', '%b', '%pb'),
    ('OpAccessChain', '%ptr_float', '%ea', '%a', '%i1', '%i2'),
    ('OpLoad', '%float', '%v', '%ea', ALIGNED, 4),
    ('OpAccessChain', '%ptr_float', '%eb', '%b', '%i1', '%i1'),
    ('OpStore', '%eb', '%v', ALIGNED, 4),
    ('OpReturn',),
    ('OpFunctionEnd',),
]))


def main():
    assembled_only = sys.argv[1:] == ['--assembled']
    if len(sys.argv) != 3 and not assembled_only:
        sys.exit(__doc__)
    out_dir = os.path.join(os.path.dirname(os.path.abspath(__file__)), '..', 'sdl3-shadercross', 'testdata')
    os.makedirs(out_dir, exist_ok=True)
    if not assembled_only:
        glslang, spirv_opt = sys.argv[1:]
        with tempfile.TemporaryDirectory() as tmp:
            for name, stage, source, optimize in SHADERS:
                src = os.path.join(tmp, name + '.' + stage)
                with open(src, 'w') as f:
                    f.write(source.lstrip())
                out = os.path.join(out_dir, name + '.spv')
                spv = os.path.join(tmp, name + '.spv') if optimize else out
                subprocess.run([glslang, '-V', '--quiet', '-o', spv, src], check=True)
                if optimize:
                    subprocess.run([spirv_opt, '-O', spv, '-o', out], check=True)
        for name, source, transform in MALFORMED:
            with open(os.path.join(out_dir, source + '.spv'), 'rb') as f:
                data = transform(f.read())
            with open(os.path.join(out_dir, name + '.spv'), 'wb') as f:
                f.write(data)
    for name, lines in ASSEMBLED:
        with open(os.path.join(out_dir, name + '.spv'), 'wb') as f:
            f.write(pack(assemble(lines)))


if __name__ == '__main__':
    main()
