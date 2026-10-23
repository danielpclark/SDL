#!/usr/bin/env python3
"""Link a program for wasm32-unknown-emscripten: Cargo's linker for the
target (.cargo/config.toml), which runs emcc with what rustc passes, plus
the link options the program asks for.

Usage: emscripten_link.py EMCC-ARGUMENTS...

Cargo has no link options per test program (a build script's
`rustc-link-arg-tests` reaches every test of its package, and the
`rustflags` of .cargo/config.toml every program), so a program that needs
its own, as one linked with Asyncify does, asks for them: a custom section
`sdl3_link` in one of its object files (a `#[used] #[link_section =
"sdl3_link"]` static, as sdl3/tests/emscripten_asyncify.rs has), holding
emcc options separated by NUL bytes, such as `-sASYNCIFY=1`. Only the
program's own objects count (the `.o` files on the command line, not the
members of the libraries, `.rlib` and `.a` archives), so a library can't
ask for options for the programs that use it. The options go after rustc's
(and the rustflags' `-Clink-arg`s), which they override where they're the
same setting. A program that asks for nothing is linked as emcc would link
it.

Environment:
  EMCC                     the emcc to run (default: emcc, from PATH)
  SDL3_EMSCRIPTEN_VERBOSE  1: the options a program asks for, on stderr
"""
import os
import shutil
import subprocess
import sys

PROGRAM = os.path.basename(__file__)
SECTION = b"sdl3_link"

VERBOSE = os.environ.get("SDL3_EMSCRIPTEN_VERBOSE", "") not in ("", "0")


def note(message):
    print(f"{PROGRAM}: {message}", file=sys.stderr, flush=True)


def leb128(data, pos):
    value = shift = 0
    while True:
        byte = data[pos]
        pos += 1
        value |= (byte & 0x7F) << shift
        shift += 7
        if byte < 0x80:
            return value, pos


def requested_options(path):
    """The options in the `sdl3_link` sections of the wasm object file
    `path` (none if it isn't one)."""
    try:
        with open(path, "rb") as f:
            data = f.read()
    except OSError:
        return []
    if data[:4] != b"\0asm" or SECTION not in data:
        return []
    options = []
    pos = 8
    try:
        while pos < len(data):
            section_id = data[pos]
            size, pos = leb128(data, pos + 1)
            end = pos + size
            if section_id == 0:
                name_len, name_pos = leb128(data, pos)
                if data[name_pos:name_pos + name_len] == SECTION:
                    payload = data[name_pos + name_len:end]
                    options += [o.decode() for o in payload.split(b"\0") if o]
            pos = end
    except (IndexError, UnicodeDecodeError) as e:
        note(f"{path}: a malformed section {SECTION.decode()} ({e})")
        sys.exit(1)
    return options


def response_file_arguments(path):
    """The arguments in a response file `@path` (rustc writes one, an
    argument per line with backslashes escaping, when the command line is
    too long)."""
    try:
        with open(path, encoding="utf-8") as f:
            lines = f.read().splitlines()
    except OSError:
        return []
    arguments = []
    for line in lines:
        argument, escaped = [], False
        for c in line:
            if escaped or c != "\\":
                argument.append(c)
                escaped = False
            else:
                escaped = True
        arguments.append("".join(argument))
    return arguments


def main():
    arguments = sys.argv[1:]
    seen = []
    for argument in arguments:
        if argument.startswith("@"):
            seen += response_file_arguments(argument[1:])
        else:
            seen.append(argument)
    options = []
    for argument in seen:
        if argument.endswith(".o"):
            for option in requested_options(argument):
                if not option.startswith("-"):
                    note(f"{argument}: {option!r} in section {SECTION.decode()} isn't an option")
                    return 1
                if option not in options:
                    options.append(option)
    if options and VERBOSE:
        note("the program asks for " + " ".join(options))
    emcc = os.environ.get("EMCC") or "emcc"
    argv = [emcc, *arguments, *options]
    sys.stdout.flush()
    sys.stderr.flush()
    if os.name == "posix":
        try:
            os.execvp(emcc, argv)
        except OSError as e:
            note(f"can't run {emcc}: {e}")
            return 127
    # (elsewhere emcc is a batch file, emcc.bat, which PATHEXT finds)
    return subprocess.call([shutil.which(emcc) or emcc, *argv[1:]])


if __name__ == "__main__":
    sys.exit(main())
