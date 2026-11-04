# Licensing of a Rust translation of SDL

*This document explains the reasoning behind the license choice for this
repository. It is not legal advice; for a product decision, talk to a lawyer.*

## The short answer

**A translation of SDL is a derivative work of SDL, so SDL's license terms
follow it.** Rewriting C into Rust by hand or by machine does not create an
independent, un-encumbered work: copyright protects the expression of the
program (its structure, sequence, organisation, comments, tables, algorithmic
choices), not merely the literal characters, and a faithful translation
carries all of that over.

The good news is that SDL is under the **zlib license**, one of the most
permissive licenses in existence. It says, in full effect:

1. you may use the software for any purpose, including commercially;
2. you may alter it and redistribute it freely;
3. you must not misrepresent its origin or claim you wrote the original;
4. altered versions must be plainly marked as such;
5. the notice may not be removed from any source distribution.

Nothing in it requires that your changes, or the work as a whole, stay under
zlib. So:

* **Yes, the Rust translation can be distributed under a different license**
  (MIT, Apache-2.0, MPL-2.0, GPL, a proprietary EULA, ...). The new license
  governs the new expression you added (the Rust-specific decisions, the
  idiomatic restructuring, the tests, the docs) and the terms under which you
  offer the combined work.
* **No, you cannot strip SDL's terms off the translated portions.** Whatever
  license you choose must be *compatible* with the zlib conditions, i.e. it
  must still (a) keep the zlib notice in the source distribution, (b) mark
  the work as an altered version of SDL, and (c) not claim the original is
  yours. Every mainstream license is compatible with these conditions, because
  they only require attribution and honesty about provenance.
* **You cannot stop anyone else from doing the same thing.** Because the
  translation is derived from zlib code, the parts that are SDL's expression
  remain available to everyone under zlib from the upstream project. Your new
  license only restricts *your* additions.

## What this repository does

This repository keeps the **zlib license** for everything, for three reasons:

1. It is the simplest honest position: one notice, one set of terms, no
   question about which lines are "SDL's" and which are "ours".
2. It keeps the door open for contributing fixes found during translation
   back upstream, and for pulling upstream changes down.
3. It matches the expectations of SDL's users; a `sdl3` crate with surprising
   terms would be a trap.

`LICENSE.txt` contains the unaltered SDL notice (condition 5), states that this
is an altered version (condition 4) and names the origin (condition 3). Each
source file repeats this in its header.

The exceptions are the translations of the third-party libraries SDL_ttf,
SDL_mixer and SDL_shadercross bundle or vendor, which keep their own
licenses, among them the LGPL of the four crates `sdl3-mixer` uses for
MIDI, MP3 and game music by default (`sdl3-mixer-timidity`,
`sdl3-mixer-fluidsynth`, `sdl3-mixer-mpg123`, `sdl3-mixer-gme`). The
README's License section lists them, with a note on what the LGPL asks
of a program that links them statically (as Cargo links every crate),
and of an app on iOS, tvOS and visionOS, and on Android (below).

## If you want a different license later

You may relicense **your own contributions** at any time, since you hold their
copyright. To relicense the repository as a whole you would:

1. keep `LICENSE.txt`'s SDL notice and the "altered version" statement intact;
2. add your license text alongside it and state clearly which terms apply to
   which parts (or that the whole is offered under the new terms *in addition
   to* honoring the zlib conditions);
3. obtain agreement from every other contributor whose work is included, or
   have a contributor license agreement that allows it.

What you *cannot* do, whatever the chosen license, is remove the SDL notice or
present the work as an independent original.

## Clean-room alternative

The only way to produce a Rust SDL that is **not** a derivative work is a
clean-room implementation: one team writes a specification from the public
API documentation and observed behaviour, and a separate team that has never
read SDL's source implements it. API names and signatures themselves are at
most thinly protected (see *Google v. Oracle*, 2021), so a clean-room
`sdl3`-compatible library could be licensed however its authors like. That is a
different project from this one, which deliberately translates the real
implementation so that behaviour (and bugs, and quirks) match upstream.

## Third-party code inside SDL

Upstream SDL vendors a few pieces under other permissive licenses (for example
parts of `src/libm` from Sun's fdlibm, `SDL_qsort.c`, the HIDAPI code, parts of
`SDL_malloc.c` from dlmalloc, and the public-domain CRC/murmur3 snippets). When
those are translated, their original notices come along with them in the same
way; the per-file header will name the original license. All of them are
compatible with the zlib terms used here.

## The console keyboard (`SDL_evdev_kbd.c`)

Upstream's console keyboard, `src/core/linux/SDL_evdev_kbd.c` (the text
and the virtual console switching of the evdev input reader the KMS/DRM
video driver uses), says of its key handling "This logic is adapted from
drivers/tty/vt/keyboard.c in the Linux kernel source": its handlers
(`k_self()`, `k_shift()`, `k_pad()`, `handle_diacr()`, the `fn_*`
functions, ...) keep that file's names and structure. The kernel's file
is under the GPL (version 2); upstream ships `SDL_evdev_kbd.c` under the
zlib license, as the rest of SDL. This repository translates it as the
zlib file upstream ships (`sdl3/src/core/linux/evdev_kbd.rs`), with that
comment kept verbatim, by the maintainer's decision of 2026-11-04,
recorded here with its provenance so that whoever redistributes the
crate can weigh it. Its built-in keymaps and accent table
(`sdl3/src/core/linux/evdev_kbd_tables.rs`, generated by
`tools/gen_evdev_kbd_tables.py` from upstream's
`SDL_evdev_kbd_default_keymap.h` and `SDL_evdev_kbd_default_accents.h`)
are data, which upstream made with its own `DUMP_KEYMAP` and
`DUMP_ACCENTS` code from a console's tables. No other code of the
kernel's is translated; the kernel's interfaces the reader uses are
declarations (below).

## libdrm, libgbm and `drm_fourcc.h` (the KMS/DRM driver)

The KMS/DRM video driver loads libdrm and libgbm at run time, as
upstream's `SDL_KMSDRM_SHARED` build does (nothing is linked), and
declares what it uses of their headers by hand
(`sdl3/src/video/drivers/kmsdrm/sys.rs`: the structures of `xf86drm.h`
and `xf86drmMode.h`, `gbm.h`'s handles and flags, `drm_fourcc.h`'s format
codes; the function types in `dyn.rs`, as upstream's `SDL_kmsdrmsym.h`
lists them). libdrm, Mesa's libgbm and `drm_fourcc.h` are under the MIT
license; the values they carry from the kernel's `drm.h` and `drm_mode.h`
(the ioctl structures' layouts, capability and property constants) are
the kernel's user-space API, under its syscall note, as below. What the
declarations carry over is interface facts (layouts, names, values),
checked against a C program built with the headers, not the headers'
expression. `tools/linux_console_run.py` (stdlib Python) declares the few
DRM ioctls it uses to read the card the same way, and makes
`kmsdrm_displays`'s vkms device through vkms's configfs interface
(`/sys/kernel/config/vkms`: the directories, attribute files and symbolic
links the kernel documents for user space), which carries no code of the
kernel's either.

## System headers declared by hand

The platform layers declare the system interfaces they call by hand, as
Rust, instead of binding generated headers: Windows' through `windows-sys`,
the C library's through `libc`, and the rest (X11, Wayland, Vulkan, the
Objective-C runtime, the kernel's input interfaces and the virtual
console's (`<linux/input.h>`, `<linux/uinput.h>`, `<linux/kd.h>`,
`<linux/keyboard.h>`, `<linux/vt.h>`, `<linux/tiocl.h>`: the kernel's
user-space API, which its syscall note leaves out of the GPL's reach),
...) in the modules that use them. What such a declaration carries over is the interface's
layout and names (a structure's fields, a function's parameters, a
table's slot order, a constant's value), the facts a program needs to
call it, not the header's expression (its comments, its macros, its
arrangement). Android's are of this kind: the JNI function tables
(`sdl3/src/core/android/jni.rs`, whose slot order is checked against the
NDK's `jni.h`) and the NDK's C interfaces upstream includes are declared
from the NDK's headers: `<android/log.h>`, `<android/asset_manager.h>`,
`<android/native_window.h>`, `<android/looper.h>` and `<android/sensor.h>`
(`sdl3/src/core/android/ndk.rs`), `<aaudio/AAudio.h>`
(`audio/drivers/aaudio/sys.rs`), the OpenSL ES headers
(`audio/drivers/opensles/sys.rs`), the camera2 and media NDK's
(`camera/android/sys.rs`), and `AHardwareBuffer_describe()` of
`<android/hardware_buffer.h>` (the Vulkan renderer). The NDK's headers
are AOSP's, under the Apache License 2.0 (`jni.h` is libnativehelper's),
but the OpenSL ES ones, which are the Khronos Group's.

## Android apps

What an Android build of this repository contains, as facts about the
code (how a license applies to an app is not described here):

* **No Java.** Upstream's Java (android-project's `org.libsdl.app`) is
  translated to Rust (`sdl3/src/core/android/java/`), as its C is. The
  classes the Android framework instantiates are declared in
  `sdl3/src/core/android/classes.rs`, with upstream's names and method
  signatures, and written as a DEX file by `sdl3/src/core/android/dex.rs`
  (kept generated in `classes_dex.rs`, in the library's
  `.sdl3_android_dex` section): their methods are native, implemented in
  Rust; the only bytecode is the constructors' calls of their
  superclasses' and `SDLActivity`'s static initializer's
  `System.loadLibrary("main")`.
* **The NDK.** Its headers are used for declarations only (above), and
  it is the linker's toolchain (`tools/android_link.py` runs its clang):
  nothing is compiled with it, and no NDK code is translated. A program
  links the
  device's system libraries through the NDK's stubs: the C library
  (`libc`, `libm`, `libdl`), and, as upstream links them, `liblog`,
  `libandroid` and `libOpenSLES`; the others (EGL, OpenGL ES, Vulkan,
  AAudio, the camera and media NDK) are loaded at run time, as upstream
  loads them. Linked statically is only what the NDK's clang adds to
  every program it links for Android: the C runtime's start and end
  objects, and LLVM's runtime libraries (the unwinder, `libunwind`, which
  Rust's standard library needs, and compiler-rt's builtins).
* **One library.** An app's code is its `libmain.so`, the program and
  every crate it uses, sdl3 among them, linked into it statically, as
  Cargo links every crate (there is no `libSDL3.so`). So are the LGPL
  crates of `sdl3-mixer`'s default features
  (`sdl3-mixer-timidity`, `sdl3-mixer-fluidsynth`, `sdl3-mixer-mpg123`,
  `sdl3-mixer-gme`) when the program uses them: they are part of
  `libmain.so`, not separate shared libraries. The README's License
  section has what that means for relinking and installing.
