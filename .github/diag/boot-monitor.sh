#!/bin/bash
# (diag) The device's state after its boot: LABEL SAMPLES INTERVAL. Waits
# for sys.boot_completed, then every INTERVAL s prints the uptime and, when
# it changed from the previous sample, the state (overlays, navigation
# mode, rotation, focus, insets, display size).
ADB="$ANDROID_HOME/platform-tools/adb"
S="${SDL3_ANDROID_SERIAL:-emulator-5554}"
label=$1
n=${2:-24}
iv=${3:-10}
t0=$(date +%s)
since() { echo $(( $(date +%s) - t0 )); }
while :; do
  b=$(timeout 10 "$ADB" -s "$S" shell getprop sys.boot_completed 2>/dev/null | tr -d '\r')
  [ "$b" = 1 ] && break
  if [ "$(since)" -gt 1200 ]; then echo "[$label] no boot in 1200 s"; exit 0; fi
  sleep 2
done
echo "[$label] $(date -u +%T) boot_completed $(since) s after the monitor started"
prev=""
for i in $(seq 1 "$n"); do
  s=$(timeout 40 "$ADB" -s "$S" shell '
    cmd overlay list 2>&1 | grep -iE "navbar|cutout" | grep -F "[x]" | sed "s/^ */overlay /" || true
    echo "navigation_mode=$(settings get secure navigation_mode) user_rotation=$(settings get system user_rotation) accelerometer_rotation=$(settings get system accelerometer_rotation)"
    dumpsys window 2>&1 | grep -E "^ *mRotation=|mCurrentFocus=" | sed "s/^ *//" | cut -c1-160 | sort -u | head -4
    dumpsys window displays 2>&1 | grep -oE "InsetsSource id=[^ ]* type=[a-zA-Z]+ frame=\[[0-9,]*\]\[[0-9,]*\] visible=[a-z]+" | sed "s/id=[^ ]* //" | sort -u | head -10
    dumpsys window displays 2>&1 | grep -m1 -oE "DisplayCutout\{[^}]*\}" | cut -c1-200
    wm size | tr "\n" " "; echo
  ' 2>&1 | tr -d '\r')
  up=$(timeout 10 "$ADB" -s "$S" shell cut -d' ' -f1 /proc/uptime 2>/dev/null | tr -d '\r')
  if [ "$s" = "$prev" ]; then
    echo "[$label] $(date -u +%T) +$(since)s up=$up unchanged"
  else
    echo "[$label] $(date -u +%T) +$(since)s up=$up"
    if [ -z "$prev" ]; then
      echo "$s" | sed 's/^/    /'
    else
      diff <(echo "$prev") <(echo "$s") | grep -E '^[<>]' | sed 's/^/    /'
    fi
    prev=$s
  fi
  sleep "$iv"
done
if [ -z "$prev" ] || [ "$label" = settle ]; then
  echo "[$label] all the navbar/cutout overlays:"
  timeout 30 "$ADB" -s "$S" shell 'cmd overlay list 2>&1 | grep -iE "navbar|cutout"' | sed 's/^/    /' || true
fi
echo "[$label] the log's overlay changes:"
timeout 30 "$ADB" -s "$S" shell 'logcat -d -b all -v time 2>/dev/null | grep -iE "cutout|navbar|NavigationMode|setEnabled|enable-exclusive|OverlayManager" | head -40' | cut -c1-220 | sed 's/^/    /' || true
LOG="/tmp/sdl3-android-run-$(id -u)/$S/emulator.log"
echo "[$label] the emulator's log ($LOG), its adb/overlay/cutout lines:"
{ grep -iE "cutout|overlay|adb|shell|boot|navig" "$LOG" || true; } | cut -c1-220 | head -40 | sed 's/^/    /'
