#!/bin/bash
# Install and run the upstream C reference APK; capture its SDL_Log trace.
set +e
apk=$1
pkg=org.libsdl.app
t() { python3 -c 'import time;print(time.time())'; }
el() { python3 -c "import time;print('%.2fs' % (time.time()-$1))"; }
timeout 30 adb wait-for-device || { echo "no device"; exit 1; }
s=$(t); adb install -r -t -g "$apk"; echo "TIMING install $(el $s)"
adb logcat -c
s=$(t)
adb shell am start -W -n $pkg/org.libsdl.app.SDLActivity
echo "TIMING am start -W $(el $s)"
for i in $(seq 60); do
  if adb logcat -d -s SDL/APP:V | grep -q "sdl3-android-runner: exit"; then break; fi
  sleep 0.5
done
echo "TIMING launch to sentinel $(el $s)"
sleep 2
echo "pid after the sentinel (+2 s): '$(adb shell pidof $pkg)'"
adb shell dumpsys activity activities | grep -E "mResumedActivity|topResumedActivity" | head -3
echo "--- logcat -v raw -s SDL/APP SDL"
adb logcat -d -v raw -s SDL/APP:V SDL:V
echo "--- logcat (time) of SDL, AndroidRuntime, ActivityManager"
adb logcat -d -v time -s SDL:V SDL/APP:V AndroidRuntime:E ActivityTaskManager:I libc:F DEBUG:F | tail -120
echo "--- crash buffer"
adb logcat -d -b crash | tail -40
adb shell am force-stop $pkg
