#!/bin/bash
# (diag) CPU load (two busy loops per CPU) while the emulator boots, as a
# long build makes, ended once sys.boot_completed is set.
ADB="$ANDROID_HOME/platform-tools/adb"
S="${SDL3_ANDROID_SERIAL:-emulator-5554}"
pids=()
for i in $(seq 1 $(( 2 * $(nproc) ))); do
  sh -c 'while :; do :; done' &
  pids+=($!)
done
t0=$(date +%s)
echo "load: ${#pids[@]} busy loops from $(date -u +%T)"
while :; do
  b=$(timeout 10 "$ADB" -s "$S" shell getprop sys.boot_completed 2>/dev/null | tr -d '\r')
  [ "$b" = 1 ] && break
  [ $(( $(date +%s) - t0 )) -gt 1200 ] && break
  sleep 2
done
kill "${pids[@]}"
echo "load: ended at $(date -u +%T), $(( $(date +%s) - t0 )) s later (boot_completed=$b)"
