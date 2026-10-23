#!/bin/bash
# Upstream SDL3 (the commit the Rust port follows) for the iOS simulator,
# and a C trace program through the same bundle/launch path.
set +e
U="$SPIKE_UDID"
W="$RUNNER_TEMP/up"
h() { echo; echo "===== $*"; }
t() { python3 -c 'import time;print(time.time())'; }
mkdir -p "$W"
h "clone upstream"
s=$(t)
git clone -q --filter=blob:none https://github.com/libsdl-org/SDL "$W/SDL" && git -C "$W/SDL" checkout -q "$UPSTREAM_COMMIT"
git -C "$W/SDL" log -1 --format='%H %cd %s'
python3 -c "import time;print('clone elapsed %.1fs' % (time.time()-$s))"
command -v ninja || brew install ninja
h "cmake configure (iOS simulator, static)"
s=$(t)
cmake -S "$W/SDL" -B "$W/build" -G Ninja -DCMAKE_SYSTEM_NAME=iOS -DCMAKE_OSX_SYSROOT=iphonesimulator \
  -DCMAKE_OSX_ARCHITECTURES=arm64 -DSDL_SHARED=OFF -DSDL_STATIC=ON -DSDL_TESTS=OFF -DSDL_EXAMPLES=OFF \
  -DCMAKE_BUILD_TYPE=RelWithDebInfo > "$W/configure.log" 2>&1; echo "configure rc=$?"
tail -60 "$W/configure.log"
python3 -c "import time;print('configure elapsed %.1fs' % (time.time()-$s))"
h "cmake build"
s=$(t)
cmake --build "$W/build" > "$W/build.log" 2>&1; echo "build rc=$?"
tail -15 "$W/build.log"; grep -E "error:" "$W/build.log" | head -20
python3 -c "import time;print('build elapsed %.1fs' % (time.time()-$s))"
ls -la "$W/build"/*.a; cat "$W/build/sdl3.pc"
LIBS=$(grep -E '^Libs' "$W/build/sdl3.pc" | sed -E 's/^Libs(\.private)?://' | tr '\n' ' ' | sed -e 's/-L\${libdir}//' -e 's/-lSDL3//')
echo "link flags from sdl3.pc: $LIBS"
h "compile and link the C trace"
xcrun --sdk iphonesimulator clang -target arm64-apple-ios14.0-simulator -g -O1 -Wall \
  -I "$W/SDL/include" -I "$W/build/include" -I "$W/build/include-config-relwithdebinfo" c/sdl_trace.c "$W/build/libSDL3.a" $LIBS \
  -o "$W/sdl_trace"; echo "link rc=$?"
if [ ! -x "$W/sdl_trace" ]; then
  echo "retrying with an explicit framework list"
  xcrun --sdk iphonesimulator clang -target arm64-apple-ios14.0-simulator -g -O1 -I "$W/SDL/include" c/sdl_trace.c "$W/build/libSDL3.a" \
    -framework AVFoundation -framework AudioToolbox -framework CoreAudio -framework CoreBluetooth -framework CoreGraphics \
    -framework CoreHaptics -framework CoreMedia -framework CoreMotion -framework CoreVideo -framework Foundation \
    -framework GameController -framework Metal -framework OpenGLES -framework QuartzCore -framework UIKit \
    -framework UniformTypeIdentifiers -liconv -lc++ -o "$W/sdl_trace"; echo "link rc=$?"
fi
otool -l "$W/sdl_trace" | grep -A5 LC_BUILD_VERSION | head -7
h "waiting for the simulator"
[ -z "$U" ] && U=$(./wait_boot.sh)
echo "UDID=$U"
h "bundle and launch the C trace (no scene manifest)"
./run.py bundle "$W/sdl_trace" "$W/sdl_trace.app" org.libsdl.sdl3-c.trace > /dev/null
SIMCTL_CHILD_SPIKE_VAR=fwd ./run.py launch "$U" "$W/sdl_trace.app" org.libsdl.sdl3-c.trace pty 120 arg1 "arg two"
h "bundle and launch the C trace (with UIApplicationSceneManifest)"
./run.py bundle "$W/sdl_trace" "$W/sdl_trace_scene.app" org.libsdl.sdl3-c.trace-scene --scene > /dev/null
./run.py launch "$U" "$W/sdl_trace_scene.app" org.libsdl.sdl3-c.trace-scene pty 120
h "the C trace under simctl spawn (not an app)"
perl -e 'alarm 60; exec @ARGV' xcrun simctl spawn "$U" "$W/sdl_trace" 2>&1 | tail -30; echo "rc=${PIPESTATUS[0]}"
