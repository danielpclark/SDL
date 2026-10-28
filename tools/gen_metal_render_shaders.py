#!/usr/bin/env python3
"""Generate sdl3/src/render/metal/shaders*.rs from SDL's
src/render/metal/SDL_shaders_metal_*.h: the precompiled shaders of the
Metal 2D renderer (made by build-metal-shaders.sh with xcrun metal,
metal-ar and metallib, Metal 1.1, from SDL_shaders_metal.metal), a library
for each platform.

Usage:
  gen_metal_render_shaders.py <SDL source dir> > sdl3/src/render/metal/shaders.rs
  gen_metal_render_shaders.py <SDL source dir> PLATFORM > sdl3/src/render/metal/shaders_PLATFORM.rs

PLATFORM is one of upstream's: macos (the default), ios, iphonesimulator,
tvos or tvsimulator. The library is kept as a byte array, as upstream has
it, sixteen to a line. SDL_render_metal.m includes one of the headers,
chosen by the preprocessor; the renderer's module chooses the file by cfg
the same way.
"""
import re
import sys
import textwrap

src_dir = sys.argv[1]
platform = sys.argv[2] if len(sys.argv) > 2 else "macos"
metal_dir = f"{src_dir}/src/render/metal"

script = open(f"{metal_dir}/build-metal-shaders.sh", encoding="utf-8").read()
source = open(f"{metal_dir}/SDL_render_metal.m", encoding="utf-8").read()

# Each platform's library: Metal 1.1, the SDK and its minimum version.
versions = {
    m.group(1): (m.group(2), m.group(3), m.group(4))
    for m in re.finditer(r"^generate_shaders (\w+) (\w+) (\w+) ([\d.]+)$", script, re.M)
}
assert versions == {
    "macos": ("macos", "macosx", "10.11"),
    "ios": ("ios", "iphoneos", "8.0"),
    "iphonesimulator": ("ios", "iphonesimulator", "8.0"),
    "tvos": ("ios", "appletvos", "9.0"),
    "tvsimulator": ("ios", "appletvsimulator", "9.0"),
}, versions
assert "-std=$compileplatform-metal1.1" in script
if platform not in versions:
    sys.exit(f"unknown platform {platform!r}: one of {', '.join(versions)}")

# Which header SDL_render_metal.m includes where; each cfg is that
# condition's in the renderer's module.
chain = """\
#ifdef SDL_PLATFORM_MACOS
#include "SDL_shaders_metal_macos.h"
#elif defined(SDL_PLATFORM_TVOS)
#if TARGET_OS_SIMULATOR
#include "SDL_shaders_metal_tvsimulator.h"
#else
#include "SDL_shaders_metal_tvos.h"
#endif
#else
#if TARGET_OS_SIMULATOR
#include "SDL_shaders_metal_iphonesimulator.h"
#else
#include "SDL_shaders_metal_ios.h"
#endif
#endif
"""
assert "// Regenerate these with build-metal-shaders.sh\n" + chain in source
built_for, chosen = {
    "macos": (
        "macOS 10.11 and later",
        "It is macOS's (`SDL_PLATFORM_MACOS`); the other platforms' libraries"
        " are in `shaders_ios.rs`, `shaders_iphonesimulator.rs`,"
        " `shaders_tvos.rs` and `shaders_tvsimulator.rs`.",
    ),
    "tvsimulator": (
        "tvOS 9.0 and later, for the simulator",
        "It is the tvOS Simulator's (`SDL_PLATFORM_TVOS` with"
        " `TARGET_OS_SIMULATOR`).",
    ),
    "tvos": (
        "tvOS 9.0 and later, for devices",
        "It is the tvOS devices' (`SDL_PLATFORM_TVOS` without"
        " `TARGET_OS_SIMULATOR`).",
    ),
    "iphonesimulator": (
        "iOS 8.0 and later, for the simulator",
        "It is the simulators' of the other UIKit platforms (iOS, visionOS:"
        " neither `SDL_PLATFORM_MACOS` nor `SDL_PLATFORM_TVOS`, with"
        " `TARGET_OS_SIMULATOR`).",
    ),
    "ios": (
        "iOS 8.0 and later, for devices",
        "It is the devices' of the other UIKit platforms (iOS, visionOS, Mac"
        " Catalyst: neither `SDL_PLATFORM_MACOS` nor `SDL_PLATFORM_TVOS`,"
        " without `TARGET_OS_SIMULATOR`).",
    ),
}[platform]
assert f"{versions[platform][2]} and later" in built_for

header = f"SDL_shaders_metal_{platform}.h"
text = open(f"{metal_dir}/{header}", encoding="utf-8").read()

# xxd -i output, made const by the script's perl.
m = re.fullmatch(
    r"const unsigned char sdl_metallib\[\] = \{(.*?)\};\s*"
    r"const unsigned int sdl_metallib_len = (\d+);\s*",
    text,
    re.S,
)
assert m, "unexpected header layout"
data = bytes(int(b, 16) for b in re.findall(r"0x[0-9a-fA-F]{2}", m.group(1)))
assert len(data) == int(m.group(2)), "size"
# A Metal library: the `MTLB` magic, then the file size (a 64-bit
# little-endian integer at offset 16).
assert data[:4] == b"MTLB", "not a Metal library"
assert int.from_bytes(data[16:24], "little") == len(data), "file size"

# The functions SDL_render_metal.m asks the library for are in it.
functions = re.findall(r'return @"(SDL_\w+)";', source)
assert functions == [
    "SDL_Solid_vertex",
    "SDL_Copy_vertex",
    "SDL_Solid_fragment",
    "SDL_Palette_fragment",
    "SDL_Copy_fragment",
    "SDL_YUV_fragment",
    "SDL_NV12_fragment",
], functions
for f in functions:
    assert f.encode() + b"\0" in data, f"no function {f}"


def wrap(prefix, text):
    """Comment lines of the text (not breaking inside a code span)."""
    text = re.sub(r"`[^`]*`", lambda m: m.group(0).replace(" ", "\xa0"), text)
    return [
        line.replace("\xa0", " ")
        for line in textwrap.wrap(
            text, width=80, initial_indent=prefix, subsequent_indent=prefix,
            break_on_hyphens=False,
        )
    ]


out = []
out.extend(wrap("// ", f"Rust translation of src/render/metal/{header} from Simple DirectMedia Layer."))
out.append("// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>")
out.append("// This is an altered (translated) version of the original software; see LICENSE.txt.")
out.append("// Generated by tools/gen_metal_render_shaders.py; do not edit.")
out.append("")
listed = ", ".join(f"`{f}`" for f in functions[:-1]) + f" and `{functions[-1]}`"
out.extend(wrap("//! ", (
    f"The shaders of the Metal renderer: a Metal library (Metal 1.1, {built_for};"
    f" the {versions[platform][1]} SDK) compiled from `SDL_shaders_metal.metal`"
    " upstream (`build-metal-shaders.sh`, with `xcrun metal`, `metal-ar` and"
    f" `metallib`), holding {listed}. {chosen}"
)))
out.append("")
out.append("/// `sdl_metallib`.")
out.append("#[rustfmt::skip]")
out.append(f"pub(super) static SDL_METALLIB: [u8; {len(data)}] = [")
for i in range(0, len(data), 16):
    out.append("    " + ", ".join(f"0x{b:02x}" for b in data[i:i + 16]) + ",")
out.append("];")
print("\n".join(out))
