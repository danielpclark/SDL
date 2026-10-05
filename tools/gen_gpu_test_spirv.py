#!/usr/bin/env python3
"""Generate sdl3/src/gpu/vulkan/test_spirv.rs: small SPIR-V 1.0 shaders for
the tests of the Vulkan GPU backend, assembled here (no glslang needed).

Usage: gen_gpu_test_spirv.py > sdl3/src/gpu/vulkan/test_spirv.rs

The shaders bind their resources where the GPU API's Vulkan backend puts
them: a vertex shader's uniform buffers in set 1, a fragment shader's
samplers in set 2 and uniform buffers in set 3, a compute shader's
read-write storage buffers in set 1.
"""
import struct

# Opcodes
OP_CAPABILITY = 17
OP_MEMORY_MODEL = 14
OP_ENTRY_POINT = 15
OP_EXECUTION_MODE = 16
OP_NAME = 5
OP_DECORATE = 71
OP_MEMBER_DECORATE = 72
OP_TYPE_VOID = 19
OP_TYPE_INT = 21
OP_TYPE_FLOAT = 22
OP_TYPE_VECTOR = 23
OP_TYPE_IMAGE = 25
OP_TYPE_SAMPLED_IMAGE = 27
OP_TYPE_RUNTIME_ARRAY = 29
OP_TYPE_STRUCT = 30
OP_TYPE_POINTER = 32
OP_TYPE_FUNCTION = 33
OP_CONSTANT = 43
OP_CONSTANT_COMPOSITE = 44
OP_FUNCTION = 54
OP_FUNCTION_END = 56
OP_VARIABLE = 59
OP_LOAD = 61
OP_STORE = 62
OP_ACCESS_CHAIN = 65
OP_COMPOSITE_CONSTRUCT = 80
OP_COMPOSITE_EXTRACT = 81
OP_IMAGE_SAMPLE_IMPLICIT_LOD = 87
OP_FADD = 129
OP_FMUL = 133
OP_LABEL = 248
OP_RETURN = 253

# Enumerants
CAPABILITY_SHADER = 1
ADDRESSING_LOGICAL = 0
MEMORY_GLSL450 = 1
MODEL_VERTEX = 0
MODEL_FRAGMENT = 4
MODEL_GLCOMPUTE = 5
MODE_ORIGIN_UPPER_LEFT = 7
MODE_LOCAL_SIZE = 17
DEC_BLOCK = 2
DEC_BUFFER_BLOCK = 3
DEC_ARRAY_STRIDE = 6
DEC_BUILTIN = 11
DEC_LOCATION = 30
DEC_BINDING = 33
DEC_DESCRIPTOR_SET = 34
DEC_OFFSET = 35
BUILTIN_POSITION = 0
SC_UNIFORM_CONSTANT = 0
SC_INPUT = 1
SC_UNIFORM = 2
SC_OUTPUT = 3
DIM_2D = 1


def string_words(s):
    data = s.encode() + b'\0'
    data += b'\0' * (-len(data) % 4)
    return list(struct.unpack('<%dI' % (len(data) // 4), data))


def f32(x):
    return struct.unpack('<I', struct.pack('<f', x))[0]


class Module:
    """Instructions with symbolic ids (strings starting with %)."""

    def __init__(self):
        self.ids = {}
        self.insts = []

    def id(self, name):
        if name not in self.ids:
            self.ids[name] = len(self.ids) + 1
        return self.ids[name]

    def op(self, opcode, *operands):
        words = []
        for o in operands:
            if isinstance(o, str) and o.startswith('%'):
                words.append(self.id(o))
            elif isinstance(o, str):
                words += string_words(o)
            else:
                words.append(o)
        self.insts.append([((len(words) + 1) << 16) | opcode] + words)

    def words(self):
        header = [0x07230203, 0x00010000, 0, len(self.ids) + 1, 0]
        return header + [w for inst in self.insts for w in inst]


def common(m):
    m.op(OP_CAPABILITY, CAPABILITY_SHADER)
    m.op(OP_MEMORY_MODEL, ADDRESSING_LOGICAL, MEMORY_GLSL450)


def vertex_null():
    """gl_Position = vec4(0, 0, 0, 1)"""
    m = Module()
    common(m)
    m.op(OP_ENTRY_POINT, MODEL_VERTEX, '%main', 'main', '%gl_pos')
    m.op(OP_DECORATE, '%gl_pos', DEC_BUILTIN, BUILTIN_POSITION)
    m.op(OP_TYPE_VOID, '%void')
    m.op(OP_TYPE_FUNCTION, '%fn', '%void')
    m.op(OP_TYPE_FLOAT, '%float', 32)
    m.op(OP_TYPE_VECTOR, '%v4', '%float', 4)
    m.op(OP_TYPE_POINTER, '%ptr_out_v4', SC_OUTPUT, '%v4')
    m.op(OP_VARIABLE, '%ptr_out_v4', '%gl_pos', SC_OUTPUT)
    m.op(OP_CONSTANT, '%float', '%f0', f32(0.0))
    m.op(OP_CONSTANT, '%float', '%f1', f32(1.0))
    m.op(OP_CONSTANT_COMPOSITE, '%v4', '%pos', '%f0', '%f0', '%f0', '%f1')
    m.op(OP_FUNCTION, '%void', '%main', 0, '%fn')
    m.op(OP_LABEL, '%entry')
    m.op(OP_STORE, '%gl_pos', '%pos')
    m.op(OP_RETURN)
    m.op(OP_FUNCTION_END)
    return m.words()


def vertex_input_uniform():
    """layout(location = 0) in vec2 pos;
    layout(set = 1, binding = 0) uniform U { vec4 offset; };
    gl_Position = vec4(pos, 0, 1) + offset"""
    m = Module()
    common(m)
    m.op(OP_ENTRY_POINT, MODEL_VERTEX, '%main', 'main', '%pos_in', '%gl_pos')
    m.op(OP_DECORATE, '%pos_in', DEC_LOCATION, 0)
    m.op(OP_DECORATE, '%gl_pos', DEC_BUILTIN, BUILTIN_POSITION)
    m.op(OP_DECORATE, '%U', DEC_BLOCK)
    m.op(OP_MEMBER_DECORATE, '%U', 0, DEC_OFFSET, 0)
    m.op(OP_DECORATE, '%u', DEC_DESCRIPTOR_SET, 1)
    m.op(OP_DECORATE, '%u', DEC_BINDING, 0)
    m.op(OP_TYPE_VOID, '%void')
    m.op(OP_TYPE_FUNCTION, '%fn', '%void')
    m.op(OP_TYPE_FLOAT, '%float', 32)
    m.op(OP_TYPE_VECTOR, '%v2', '%float', 2)
    m.op(OP_TYPE_VECTOR, '%v4', '%float', 4)
    m.op(OP_TYPE_POINTER, '%ptr_in_v2', SC_INPUT, '%v2')
    m.op(OP_VARIABLE, '%ptr_in_v2', '%pos_in', SC_INPUT)
    m.op(OP_TYPE_POINTER, '%ptr_out_v4', SC_OUTPUT, '%v4')
    m.op(OP_VARIABLE, '%ptr_out_v4', '%gl_pos', SC_OUTPUT)
    m.op(OP_TYPE_STRUCT, '%U', '%v4')
    m.op(OP_TYPE_POINTER, '%ptr_u_U', SC_UNIFORM, '%U')
    m.op(OP_VARIABLE, '%ptr_u_U', '%u', SC_UNIFORM)
    m.op(OP_TYPE_INT, '%int', 32, 1)
    m.op(OP_CONSTANT, '%int', '%int0', 0)
    m.op(OP_TYPE_POINTER, '%ptr_u_v4', SC_UNIFORM, '%v4')
    m.op(OP_CONSTANT, '%float', '%f0', f32(0.0))
    m.op(OP_CONSTANT, '%float', '%f1', f32(1.0))
    m.op(OP_FUNCTION, '%void', '%main', 0, '%fn')
    m.op(OP_LABEL, '%entry')
    m.op(OP_LOAD, '%v2', '%p', '%pos_in')
    m.op(OP_COMPOSITE_EXTRACT, '%float', '%x', '%p', 0)
    m.op(OP_COMPOSITE_EXTRACT, '%float', '%y', '%p', 1)
    m.op(OP_COMPOSITE_CONSTRUCT, '%v4', '%v', '%x', '%y', '%f0', '%f1')
    m.op(OP_ACCESS_CHAIN, '%ptr_u_v4', '%offset_ptr', '%u', '%int0')
    m.op(OP_LOAD, '%v4', '%offset', '%offset_ptr')
    m.op(OP_FADD, '%v4', '%sum', '%v', '%offset')
    m.op(OP_STORE, '%gl_pos', '%sum')
    m.op(OP_RETURN)
    m.op(OP_FUNCTION_END)
    return m.words()


def fragment_solid():
    """layout(location = 0) out vec4 color; color = vec4(1, 0, 0, 1)"""
    m = Module()
    common(m)
    m.op(OP_ENTRY_POINT, MODEL_FRAGMENT, '%main', 'main', '%color')
    m.op(OP_EXECUTION_MODE, '%main', MODE_ORIGIN_UPPER_LEFT)
    m.op(OP_DECORATE, '%color', DEC_LOCATION, 0)
    m.op(OP_TYPE_VOID, '%void')
    m.op(OP_TYPE_FUNCTION, '%fn', '%void')
    m.op(OP_TYPE_FLOAT, '%float', 32)
    m.op(OP_TYPE_VECTOR, '%v4', '%float', 4)
    m.op(OP_TYPE_POINTER, '%ptr_out_v4', SC_OUTPUT, '%v4')
    m.op(OP_VARIABLE, '%ptr_out_v4', '%color', SC_OUTPUT)
    m.op(OP_CONSTANT, '%float', '%f0', f32(0.0))
    m.op(OP_CONSTANT, '%float', '%f1', f32(1.0))
    m.op(OP_CONSTANT_COMPOSITE, '%v4', '%red', '%f1', '%f0', '%f0', '%f1')
    m.op(OP_FUNCTION, '%void', '%main', 0, '%fn')
    m.op(OP_LABEL, '%entry')
    m.op(OP_STORE, '%color', '%red')
    m.op(OP_RETURN)
    m.op(OP_FUNCTION_END)
    return m.words()


def fragment_sampled_uniform():
    """layout(set = 2, binding = 0) uniform sampler2D tex;
    layout(set = 3, binding = 0) uniform T { vec4 tint; };
    layout(location = 0) out vec4 color;
    color = texture(tex, vec2(0.5)) * tint"""
    m = Module()
    common(m)
    m.op(OP_ENTRY_POINT, MODEL_FRAGMENT, '%main', 'main', '%color')
    m.op(OP_EXECUTION_MODE, '%main', MODE_ORIGIN_UPPER_LEFT)
    m.op(OP_DECORATE, '%color', DEC_LOCATION, 0)
    m.op(OP_DECORATE, '%tex', DEC_DESCRIPTOR_SET, 2)
    m.op(OP_DECORATE, '%tex', DEC_BINDING, 0)
    m.op(OP_DECORATE, '%T', DEC_BLOCK)
    m.op(OP_MEMBER_DECORATE, '%T', 0, DEC_OFFSET, 0)
    m.op(OP_DECORATE, '%t', DEC_DESCRIPTOR_SET, 3)
    m.op(OP_DECORATE, '%t', DEC_BINDING, 0)
    m.op(OP_TYPE_VOID, '%void')
    m.op(OP_TYPE_FUNCTION, '%fn', '%void')
    m.op(OP_TYPE_FLOAT, '%float', 32)
    m.op(OP_TYPE_VECTOR, '%v2', '%float', 2)
    m.op(OP_TYPE_VECTOR, '%v4', '%float', 4)
    m.op(OP_TYPE_POINTER, '%ptr_out_v4', SC_OUTPUT, '%v4')
    m.op(OP_VARIABLE, '%ptr_out_v4', '%color', SC_OUTPUT)
    m.op(OP_TYPE_IMAGE, '%image', '%float', DIM_2D, 0, 0, 0, 1, 0)
    m.op(OP_TYPE_SAMPLED_IMAGE, '%sampled_image', '%image')
    m.op(OP_TYPE_POINTER, '%ptr_uc_si', SC_UNIFORM_CONSTANT, '%sampled_image')
    m.op(OP_VARIABLE, '%ptr_uc_si', '%tex', SC_UNIFORM_CONSTANT)
    m.op(OP_TYPE_STRUCT, '%T', '%v4')
    m.op(OP_TYPE_POINTER, '%ptr_u_T', SC_UNIFORM, '%T')
    m.op(OP_VARIABLE, '%ptr_u_T', '%t', SC_UNIFORM)
    m.op(OP_TYPE_INT, '%int', 32, 1)
    m.op(OP_CONSTANT, '%int', '%int0', 0)
    m.op(OP_TYPE_POINTER, '%ptr_u_v4', SC_UNIFORM, '%v4')
    m.op(OP_CONSTANT, '%float', '%half', f32(0.5))
    m.op(OP_CONSTANT_COMPOSITE, '%v2', '%coord', '%half', '%half')
    m.op(OP_FUNCTION, '%void', '%main', 0, '%fn')
    m.op(OP_LABEL, '%entry')
    m.op(OP_LOAD, '%sampled_image', '%si', '%tex')
    m.op(OP_IMAGE_SAMPLE_IMPLICIT_LOD, '%v4', '%sample', '%si', '%coord')
    m.op(OP_ACCESS_CHAIN, '%ptr_u_v4', '%tint_ptr', '%t', '%int0')
    m.op(OP_LOAD, '%v4', '%tint', '%tint_ptr')
    m.op(OP_FMUL, '%v4', '%product', '%sample', '%tint')
    m.op(OP_STORE, '%color', '%product')
    m.op(OP_RETURN)
    m.op(OP_FUNCTION_END)
    return m.words()


def compute_fill():
    """layout(local_size_x = 1) in;
    layout(set = 1, binding = 0) buffer B { uint data[]; };
    data[0] = 1"""
    m = Module()
    common(m)
    m.op(OP_ENTRY_POINT, MODEL_GLCOMPUTE, '%main', 'main')
    m.op(OP_EXECUTION_MODE, '%main', MODE_LOCAL_SIZE, 1, 1, 1)
    m.op(OP_DECORATE, '%rta', DEC_ARRAY_STRIDE, 4)
    m.op(OP_MEMBER_DECORATE, '%B', 0, DEC_OFFSET, 0)
    m.op(OP_DECORATE, '%B', DEC_BUFFER_BLOCK)
    m.op(OP_DECORATE, '%b', DEC_DESCRIPTOR_SET, 1)
    m.op(OP_DECORATE, '%b', DEC_BINDING, 0)
    m.op(OP_TYPE_VOID, '%void')
    m.op(OP_TYPE_FUNCTION, '%fn', '%void')
    m.op(OP_TYPE_INT, '%uint', 32, 0)
    m.op(OP_TYPE_RUNTIME_ARRAY, '%rta', '%uint')
    m.op(OP_TYPE_STRUCT, '%B', '%rta')
    m.op(OP_TYPE_POINTER, '%ptr_u_B', SC_UNIFORM, '%B')
    m.op(OP_VARIABLE, '%ptr_u_B', '%b', SC_UNIFORM)
    m.op(OP_TYPE_INT, '%int', 32, 1)
    m.op(OP_CONSTANT, '%int', '%int0', 0)
    m.op(OP_TYPE_POINTER, '%ptr_u_uint', SC_UNIFORM, '%uint')
    m.op(OP_CONSTANT, '%uint', '%uint1', 1)
    m.op(OP_FUNCTION, '%void', '%main', 0, '%fn')
    m.op(OP_LABEL, '%entry')
    m.op(OP_ACCESS_CHAIN, '%ptr_u_uint', '%element', '%b', '%int0', '%int0')
    m.op(OP_STORE, '%element', '%uint1')
    m.op(OP_RETURN)
    m.op(OP_FUNCTION_END)
    return m.words()


SHADERS = [
    ('VERTEX_NULL', 'A vertex shader without inputs or resources', vertex_null),
    ('VERTEX_INPUT_UNIFORM',
     'A vertex shader with a `vec2` input at location 0 and a uniform buffer', vertex_input_uniform),
    ('FRAGMENT_SOLID', 'A fragment shader without resources', fragment_solid),
    ('FRAGMENT_SAMPLED_UNIFORM', 'A fragment shader with a sampler and a uniform buffer',
     fragment_sampled_uniform),
    ('COMPUTE_FILL', 'A compute shader with a read-write storage buffer', compute_fill),
]

print('// Generated by tools/gen_gpu_test_spirv.py; do not edit.')
print('// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>')
print('// This is an altered (translated) version of the original software; see LICENSE.txt.')
print()
print('//! SPIR-V 1.0 shaders for the tests of the Vulkan GPU backend.')
for name, doc, gen in SHADERS:
    words = gen()
    print()
    print('/// %s.' % doc)
    print('pub(super) const %s: &[u32] = &[' % name)
    for i in range(0, len(words), 8):
        print('    ' + ' '.join('0x%08x,' % w for w in words[i:i + 8]))
    print('];')
