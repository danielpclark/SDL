#!/bin/bash
# The app experiments (questions 3-5, 7-10): install the APK, check ART's
# verification, then run each mode through app_run.py.
# Args: APK [MODES...]
set +e
here=$(cd "$(dirname "$0")" && pwd)
apk=$1; shift
pkg=org.libsdl.sdl3_rust.spike
h() { echo; echo "===== $*"; }
t() { python3 -c 'import time;print(time.time())'; }
el() { python3 -c "import time;print('%.2fs' % (time.time()-$1))"; }

h "install"
adb logcat -c
s=$(t); adb install -r -t -g "$apk"; echo "TIMING adb install $(el $s)"
adb shell pm path $pkg
adb shell dumpsys package $pkg | grep -E "versionCode|primaryCpuAbi|nativeLibraryDir|flags=|pkgFlags|targetSdk|dataDir" | head
h "ART: dexopt status, verification"
adb shell pm art get-dexopt-status $pkg 2>&1 | head -12
s=$(t); adb shell cmd package compile -m verify -f $pkg; echo "compile -m verify: $(el $s)"
adb shell pm art get-dexopt-status $pkg 2>&1 | head -8
s=$(t); adb shell cmd package compile -m speed -f $pkg; echo "compile -m speed: $(el $s)"
adb shell pm art get-dexopt-status $pkg 2>&1 | head -8
adb logcat -d | grep -iE "dex2oat|verif|$pkg|SDLActivity|Rejecting|hidden" | grep -v "logcat" | tail -30

modes=("$@")
[ ${#modes[@]} -eq 0 ] && modes=(exit-now exit-now-raw basic dlopen sdl-main gfx audio sensors devices input gamepad lifecycle camera panic sdl-main-rust-thread)
port=38100
for m in "${modes[@]}"; do
  h "app mode $m"
  python3 "$here/app_run.py" "$m" --port $port --timeout 75
  echo "app_run rc=$?"
  port=$((port + 2))
done
