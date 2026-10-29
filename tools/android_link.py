#!/usr/bin/env python3
"""Link a program for Android (aarch64-linux-android,
armv7-linux-androideabi, x86_64-linux-android, i686-linux-android):
Cargo's linker for those targets (.cargo/config.toml), which runs the
Android NDK's clang for the program's ABI, at API level 21 (upstream's
minimum), with what rustc passes.

Usage: android_link.py CLANG-ARGUMENTS...

The ABI is the one of the program's own object files (the `.o` files on
the command line: their ELF machine), since Cargo gives every Android
target this same linker: the NDK's `<triple>21-clang` for it
(`armv7a-linux-androideabi21-clang` for armv7) links it against the NDK's
sysroot (libc, libm, libdl, liblog, the C runtime objects and libunwind),
which Rust's standard library needs. Nothing is compiled: the NDK is only
the toolchain. The program's segments are aligned to 16 KB pages
(`-z max-page-size=16384`), which Android 15 devices may have.

Programs run on Android as executables (spawned by tools/android_run.py
through `adb shell`), which is how they're linked here. The programs that
will run as apps get linked as shared libraries (the app's libmain.so) in
`link_options()`, once there is an app mode.

The NDK: SDL3_ANDROID_NDK's directory, else ANDROID_NDK_HOME's, else
ANDROID_NDK_ROOT's, else the SDK's NDK r29 (29.0.14206865, the version CI
uses), else the SDK's newest NDK (the SDK: ANDROID_HOME, else
ANDROID_SDK_ROOT, else the usual place on Linux and macOS). It must be
r25 or later (Rust's libunwind needs it).

Environment:
  SDL3_ANDROID_NDK      the NDK's directory
  ANDROID_NDK_HOME, ANDROID_NDK_ROOT
                        the same, when SDL3_ANDROID_NDK isn't set
  ANDROID_HOME, ANDROID_SDK_ROOT
                        the SDK, whose ndk/ directory has the NDKs
  SDL3_ANDROID_VERBOSE  1: the NDK and the clang command, on stderr
"""
import os
import re
import shutil
import struct
import subprocess
import sys

PROGRAM = os.path.basename(__file__)

VERBOSE = os.environ.get("SDL3_ANDROID_VERBOSE", "") not in ("", "0")

# Upstream's minimum API level (android-project's minSdkVersion, and
# CMake's ANDROID_PLATFORM in its docs), the one Rust's Android targets
# are built for.
API = 21

# The NDK CI uses, which tools/android_run.py's emulator image was tested
# with; and the oldest one Rust can link with.
PREFERRED_NDK = "29.0.14206865"
OLDEST_NDK = 25

# ELF e_machine: the triple of the NDK's clang for it.
CLANG_TRIPLES = {
    183: "aarch64-linux-android",  # EM_AARCH64
    40: "armv7a-linux-androideabi",  # EM_ARM
    62: "x86_64-linux-android",  # EM_X86_64
    3: "i686-linux-android",  # EM_386
}

# The page size the segments are aligned to: Android 15 devices may have
# 16 KB pages (NDK r28 and later do this by default; r27 doesn't).
PAGE_SIZE_OPTION = "-Wl,-z,max-page-size=16384"


class Failure(Exception):
    """An error this script reports (and exits 1 for)."""


def note(message):
    print(f"{PROGRAM}: {message}", file=sys.stderr, flush=True)


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


def elf_machine(path):
    """The ELF e_machine of the object file `path` (None if it isn't
    one)."""
    try:
        with open(path, "rb") as f:
            header = f.read(20)
    except OSError:
        return None
    if len(header) < 20 or header[:4] != b"\x7fELF":
        return None
    order = "<" if header[5] == 1 else ">"
    return struct.unpack_from(order + "H", header, 18)[0]


def clang_triple(arguments):
    """The NDK clang's triple for the program's ABI, from its object
    files' ELF machine."""
    machines = {elf_machine(a) for a in arguments if a.endswith(".o")}
    machines.discard(None)
    if not machines:
        raise Failure("no ELF object file among the arguments, to tell the ABI by")
    if len(machines) > 1:
        raise Failure(f"object files of different ELF machines ({sorted(machines)})")
    machine = machines.pop()
    if machine not in CLANG_TRIPLES:
        raise Failure(f"ELF machine {machine} isn't one of Android's ABIs")
    return CLANG_TRIPLES[machine]


def sdk_dir():
    for name in ("ANDROID_HOME", "ANDROID_SDK_ROOT"):
        if os.environ.get(name):
            return os.environ[name]
    for candidate in ("~/Android/Sdk", "~/Library/Android/sdk"):
        path = os.path.expanduser(candidate)
        if os.path.isdir(path):
            return path
    return None


def ndk_version(ndk):
    """The NDK's version (Pkg.Revision of its source.properties), as a tuple
    of numbers, and as written; (None, None) without one."""
    try:
        with open(os.path.join(ndk, "source.properties"), encoding="utf-8") as f:
            text = f.read()
    except OSError:
        return None, None
    match = re.search(r"^Pkg\.Revision\s*=\s*([0-9][0-9.]*)", text, re.M)
    if not match:
        return None, None
    return tuple(int(p) for p in match.group(1).split(".") if p), match.group(1)


def find_ndk():
    for name in ("SDL3_ANDROID_NDK", "ANDROID_NDK_HOME", "ANDROID_NDK_ROOT"):
        path = os.environ.get(name)
        if path:
            if not os.path.isdir(path):
                raise Failure(f"{name} is {path!r}, which isn't a directory")
            return path, name
    sdk = sdk_dir()
    ndks = os.path.join(sdk, "ndk") if sdk else None
    if not ndks or not os.path.isdir(ndks):
        raise Failure(
            "no Android NDK: set SDL3_ANDROID_NDK (or ANDROID_NDK_HOME) to its directory, or ANDROID_HOME to an "
            f"SDK with it installed (sdkmanager --install 'ndk;{PREFERRED_NDK}')"
        )
    preferred = os.path.join(ndks, PREFERRED_NDK)
    if os.path.isdir(preferred):
        return preferred, f"the SDK's NDK {PREFERRED_NDK}"
    installed = [os.path.join(ndks, n) for n in os.listdir(ndks)]
    installed = [p for p in installed if ndk_version(p)[0]]
    if not installed:
        raise Failure(f"no NDK in {ndks} (sdkmanager --install 'ndk;{PREFERRED_NDK}')")
    newest = max(installed, key=lambda p: ndk_version(p)[0])
    return newest, "the SDK's newest NDK"


def host_tag():
    if sys.platform.startswith("linux"):
        return "linux-x86_64"
    if sys.platform == "darwin":
        # (the NDK's macOS toolchain is universal, under this name)
        return "darwin-x86_64"
    if sys.platform in ("win32", "cygwin"):
        return "windows-x86_64"
    raise Failure(f"the NDK has no toolchain for {sys.platform}")


def find_clang(triple):
    ndk, source = find_ndk()
    version, written = ndk_version(ndk)
    if version is None:
        raise Failure(f"{ndk} ({source}) isn't an NDK (no Pkg.Revision in its source.properties)")
    if version[0] < OLDEST_NDK:
        raise Failure(
            f"{ndk} ({source}) is NDK {written}; Rust needs r{OLDEST_NDK} or later (CI uses {PREFERRED_NDK})"
        )
    bindir = os.path.join(ndk, "toolchains", "llvm", "prebuilt", host_tag(), "bin")
    name = f"{triple}{API}-clang"
    if os.name == "nt":
        name += ".cmd"
    clang = os.path.join(bindir, name)
    if not os.path.isfile(clang):
        raise Failure(f"{ndk} ({source}, NDK {written}) has no {name} in {bindir}")
    if VERBOSE:
        note(f"NDK {written} at {ndk} ({source})")
    return clang


def link_options(arguments):
    """The options after rustc's: the page size. (The programs that ask to
    run as apps, with the word `app` in a section `.sdl3_runner` of their
    own object files, will be linked here with `-shared` in place of
    rustc's `-pie`, `-Wl,--defsym=SDL_main=main` and `JNI_OnLoad` and
    `SDL_main` exported, so that the program is an app's libmain.so;
    there is no app mode yet.)"""
    return [PAGE_SIZE_OPTION]


def main():
    arguments = sys.argv[1:]
    seen = []
    for argument in arguments:
        if argument.startswith("@"):
            seen += response_file_arguments(argument[1:])
        else:
            seen.append(argument)
    try:
        clang = find_clang(clang_triple(seen))
        argv = [clang, *arguments, *link_options(seen)]
    except Failure as e:
        note(str(e))
        return 1
    if VERBOSE:
        note(" ".join(a for a in argv if not a.endswith((".o", ".rlib"))))
    sys.stdout.flush()
    sys.stderr.flush()
    if os.name == "posix":
        try:
            os.execv(clang, argv)
        except OSError as e:
            note(f"can't run {clang}: {e}")
            return 127
    # (on Windows the NDK's clang wrappers are batch files)
    return subprocess.call([shutil.which(clang) or clang, *argv[1:]])


if __name__ == "__main__":
    sys.exit(main())
