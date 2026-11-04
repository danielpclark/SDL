#!/bin/bash
# (diag) The device's log, from its start, filtered to the display cutout,
# the rotation settings, the first-boot setup, and processes dying or not
# responding (the runner clears the log before each app: this reads it as
# it comes).
ADB="$ANDROID_HOME/platform-tools/adb"
S="${SDL3_ANDROID_SERIAL:-emulator-5554}"
for i in 1 2 3 4 5 6; do
  timeout 1500 "$ADB" -s "$S" wait-for-device logcat -v time -b main,system,events,crash 2>&1 |
    grep --line-buffered -iE "cutout|accelerometer_rotation|user_rotation|sdksetup|ANR in|not responding|FATAL EXCEPTION|has died|am_proc_died.*(systemui|launcher|sdksetup)|am_crash|Start proc [0-9]+:(com.android.systemui|com.google.android.apps.nexuslauncher|com.google.android.googlesdksetup)|sys.boot_completed|wm_set_requested_orientation|setRotation|updateRotation|navigation_mode|OverlayManager.*(enabl|disabl)" |
    cut -c1-260
  echo "(logcat ended: $(date -u +%T))"
  sleep 5
done
