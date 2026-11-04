#!/bin/bash
# (diag) Who writes the rotation settings around the boot: every 2 s, from
# the first adb answer to 120 s after sys.boot_completed, SettingsProvider's
# record of accelerometer_rotation and user_rotation (its value and the
# package that wrote it last), sys.boot_completed, the home activity and
# the provisioning settings, printed when they change.
ADB="$ANDROID_HOME/platform-tools/adb"
S="${SDL3_ANDROID_SERIAL:-emulator-5554}"
t0=$(date +%s)
booted=""
prev=""
while :; do
  now=$(date +%s)
  [ $(( now - t0 )) -gt 1500 ] && break
  if [ -n "$booted" ] && [ $(( now - booted )) -gt 120 ]; then break; fi
  s=$(timeout 15 "$ADB" -s "$S" shell '
    echo "boot_completed=$(getprop sys.boot_completed) provisioned=$(settings get global device_provisioned 2>/dev/null) home=$(cmd package resolve-activity --brief -a android.intent.action.MAIN -c android.intent.category.HOME 2>/dev/null | tail -1)"
    dumpsys settings 2>/dev/null | grep -E "name:(accelerometer_rotation|user_rotation) " | sed -E "s/^ *_id:[0-9]+ //; s/ default:.*//" | sort -u
  ' 2>/dev/null | tr -d '\r')
  [ -z "$s" ] && { sleep 2; continue; }
  case "$s" in *boot_completed=1*) [ -z "$booted" ] && booted=$now;; esac
  if [ "$s" != "$prev" ]; then
    echo "[writers] $(date -u +%T) +$(( now - t0 ))s"
    echo "$s" | sed 's/^/    /'
    prev=$s
  fi
  sleep 2
done
LOG="/tmp/sdl3-android-run-$(id -u)/$S/emulator.log"
echo "[writers] the emulator's log, its lines of settings, rotation, adb and shell:"
{ grep -iE "settings|rotat|orientation|adb|shell|boot" "$LOG" || true; } | grep -v androidboot | cut -c1-220 | head -40 | sed 's/^/    /'
