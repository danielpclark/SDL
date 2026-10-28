#!/bin/bash
# Upstream SDL3 for the tvOS Simulator and the C reference programs of the
# tvOS tests, run through tools/ios_run.py (diag branch only).
set +e
W="$RUNNER_TEMP/up"
REF="$(cd "$(dirname "$0")" && pwd)"
ROOT="$(cd "$REF/.." && pwd)"
h() { echo; echo "===== $*"; }
mkdir -p "$W"
U="$SDL3_TVOS_SIMULATOR"
h "simulator $U"
xcrun simctl list devicetypes | grep -i tv
xcrun simctl list runtimes | grep -i tv
xcrun simctl bootstatus "$U" -b > /dev/null 2>&1
xcrun simctl listapps "$U" 2>/dev/null | grep -E 'CFBundleIdentifier|CFBundleDisplayName' | head -60
h "clone upstream $UPSTREAM_COMMIT"
git clone -q --filter=blob:none https://github.com/libsdl-org/SDL "$W/SDL" && git -C "$W/SDL" checkout -q "$UPSTREAM_COMMIT"
git -C "$W/SDL" log -1 --format='%H %cd %s'
command -v ninja || brew install ninja
# OpenGL ES on tvOS, as the Xcode project's SDL_build_config_ios.h builds
# it (CMake leaves it out there): what the Rust translation follows.
sed -i '' 's/"SDL_VIDEO;NOT VISIONOS;NOT TVOS;NOT WATCHOS"/"SDL_VIDEO;NOT VISIONOS;NOT WATCHOS"/' "$W/SDL/CMakeLists.txt"
# Upstream's GLES2 program cache frees an entry twice when the renderer is
# destroyed after the scenes (uikit_gles): built with both fixes, as the
# Rust cache behaves (the same as the iOS reference).
G="$W/SDL/src/render/opengles2/SDL_render_gles2.c"
perl -0pi -e 's/(            if \(entry->prev\) \{\n                entry->prev->next = entry->next;\n            \}\n)/$1            if (data->program_cache.tail == entry) {\n                data->program_cache.tail = entry->prev;\n            }\n/' "$G"
perl -0pi -e 's/(        data->program_cache.tail = oldest->prev;\n)/$1        if (data->program_cache.tail) {\n            data->program_cache.tail->next = NULL;\n        }\n/' "$G"
git -C "$W/SDL" diff --stat
h "cmake configure (tvOS Simulator, static, Debug)"
cmake -S "$W/SDL" -B "$W/build" -G Ninja -DCMAKE_SYSTEM_NAME=tvOS -DCMAKE_OSX_SYSROOT=appletvsimulator \
  -DCMAKE_OSX_ARCHITECTURES=arm64 -DCMAKE_OSX_DEPLOYMENT_TARGET=14.0 -DSDL_SHARED=OFF -DSDL_STATIC=ON \
  -DSDL_TESTS=OFF -DSDL_EXAMPLES=OFF -DCMAKE_BUILD_TYPE=Debug > "$W/configure.log" 2>&1; echo "configure rc=$?"
sed -n '/Enabled backends/,/^--$/p' "$W/configure.log" | head -40
grep -i -E "opengl|gles|process|sensor|camera|hidapi" "$W/configure.log" | head -30
h "cmake build"
cmake --build "$W/build" > "$W/build.log" 2>&1; echo "build rc=$?"
tail -5 "$W/build.log"; grep -E "error:" "$W/build.log" | head -20
LIBS=$(grep -E '^Libs' "$W/build/sdl3.pc" | sed -E 's/^Libs(\.private)?://' | tr '\n' ' ' | sed -e 's/-L\${libdir}//' -e 's/-lSDL3//')
echo "link flags: $LIBS"
CONFIG_INC=$(ls -d "$W/build"/include-config-* | head -1)
grep -E "define SDL_(VIDEO|RENDER|JOYSTICK|SENSOR|PROCESS|CAMERA|HAPTIC|POWER|HIDAPI|AUDIO)" "$CONFIG_INC"/build_config/SDL_build_config.h | head -60
cc_ref() {
  local out="$1"; shift
  xcrun --sdk appletvsimulator clang -fobjc-arc -target arm64-apple-tvos14.0-simulator -g -O0 -Wall \
    -Wno-deprecated-declarations \
    -I "$W/SDL/include" -I "$W/build/include" -I "$CONFIG_INC" "$@" "$W/build/libSDL3.a" $LIBS \
    -framework UIKit -framework Foundation -framework GameController -framework Metal -framework QuartzCore \
    -framework OpenGLES -framework CoreHaptics -framework AVFoundation -framework CoreMedia \
    -framework CoreVideo -framework AudioToolbox -framework CoreFoundation -o "$W/$out"; echo "link $out rc=$?"
}
h "compile"
for p in $REF_PROGRAMS; do
  cc_ref "$p" "$REF/$p.m"
done
cd "$ROOT"
for p in $REF_PROGRAMS; do
  h "run $p (app)"
  SDL3_IOS_VERBOSE=1 SDL3_IOS_TIMEOUT=300 tools/ios_run.py "$W/$p"; echo "rc=$?"
done
exit 0
