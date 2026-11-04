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
    echo "device_provisioned=$(settings get global device_provisioned) user_setup_complete=$(settings get secure user_setup_complete) home=$(cmd package resolve-activity --brief -a android.intent.action.MAIN -c android.intent.category.HOME 2>&1 | tail -1)"
    echo "cutout resource: $(cmd overlay lookup android android:string/config_mainBuiltInDisplayCutout 2>&1 | head -c 60)"
    dumpsys window 2>&1 | grep -E "^ *mRotation=|mCurrentFocus=" | sed "s/^ *//" | cut -c1-160 | sort -u | head -4
    dumpsys window 2>&1 | grep -oE "(mUserRotationMode|mUserRotation|mLastSensorRotation|mLastOrientation|mCurrentAppOrientation)=[^ ]*" | sort -u | tr "\n" " "; echo
    dumpsys window displays 2>&1 | grep -oE "InsetsSource id=[^ ]* type=[a-zA-Z]+ frame=\[[0-9,]*\]\[[0-9,]*\] visible=[a-z]+" | sed "s/id=[^ ]* //" | sort -u | head -10
    dumpsys window displays 2>&1 | grep -m1 -oE "DisplayCutout\{[^}]*\}" | cut -c1-200
    wm size | tr "\n" " "; echo
  ' 2>&1 | tr -d '\r')
  s="$s
host sensor: $(timeout 10 "$ADB" -s "$S" emu sensor get acceleration 2>/dev/null | tr -d '\r' | head -1 | sed -E 's/([0-9]\.[0-9])[0-9]*/\1/g')"
  up=$(timeout 10 "$ADB" -s "$S" shell 'cut -d " " -f1 /proc/uptime' 2>/dev/null | tr -d '\r')
  if [ "$i" = 1 ]; then
    timeout 30 "$ADB" -s "$S" shell 'for n in global secure system; do settings list $n | sed "s/^/$n /"; done; getprop' > "/tmp/diag-$label-first.txt" 2>&1
  fi
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
timeout 30 "$ADB" -s "$S" shell 'for n in global secure system; do settings list $n | sed "s/^/$n /"; done; getprop' > "/tmp/diag-$label-last.txt" 2>&1
echo "[$label] the settings and properties changed since the first sample:"
diff "/tmp/diag-$label-first.txt" "/tmp/diag-$label-last.txt" | grep -E '^[<>]' | cut -c1-200 | head -60 | sed 's/^/    /'
echo "[$label] the log's cutout, rotation and setup lines:"
timeout 30 "$ADB" -s "$S" shell 'logcat -d -b all -v time 2>/dev/null | grep -iE "cutout|accelerometer_rotation|user_rotation|sdksetup|DisplayDeviceConfig|LocalDisplayAdapter|DisplayDeviceInfo|emulator|BOOT_COMPLETED|FallbackHome|Displayed " | grep -v idmap2d | head -150' | cut -c1-240 | sed 's/^/    /' || true
LOG="/tmp/sdl3-android-run-$(id -u)/$S/emulator.log"
echo "[$label] the emulator's log ($LOG), its adb/overlay/cutout lines:"
{ grep -vE "^INFO +\|   androidboot" "$LOG" || true; } | cut -c1-220 | head -120 | sed 's/^/    /'
