#!/bin/bash
# adb push / adb shell experiments with the probe program (question 2).
# Args: PROBE (x86_64 build). Env: RUNNER_TEMP.
set +e
P=$1
D=/data/local/tmp/spike
h() { echo; echo "===== $*"; }
t() { python3 -c 'import time;print(time.time())'; }
el() { python3 -c "import time;print('elapsed %.2fs' % (time.time()-$1))"; }

adb shell mkdir -p $D
h "adb version, features, shell protocol"
adb version | head -2
adb features 2>&1 | tr ',' ' '
adb shell getprop ro.build.version.sdk
adb shell id

h "push the probe ($(stat -c %s "$P") bytes)"
s=$(t); adb push "$P" $D/probe; el $s
adb shell chmod 755 $D/probe

h "exit status pass-through"
for m in "exit 0" "exit 3" "exit 255" "exit 256" "ret 42" "panic" "abort" "segv"; do
  adb shell $D/probe $m >$RUNNER_TEMP/o.txt 2>$RUNNER_TEMP/e.txt; rc=$?
  echo "mode '$m': rc=$rc stdout=$(head -c 200 $RUNNER_TEMP/o.txt | tr '\n' '|') stderr=$(head -c 300 $RUNNER_TEMP/e.txt | tr '\n' '|')"
done
adb shell 'exit 7'; echo "adb shell 'exit 7' rc=$?"
adb shell -x $D/probe exit 3 >/dev/null 2>&1; echo "adb shell -x (no shell protocol) exit 3 rc=$?"
adb shell -T $D/probe exit 4 >/dev/null 2>&1; echo "adb shell -T exit 4 rc=$?"
adb shell -t $D/probe exit 5 >/dev/null 2>&1 </dev/null; echo "adb shell -t (pty) exit 5 rc=$?"

h "stdout/stderr separation"
adb shell $D/probe info /data/local/tmp /sdcard /system/bin/sh /home/runner >$RUNNER_TEMP/o.txt 2>$RUNNER_TEMP/e.txt; echo rc=$?
echo "--- stdout:"; cat $RUNNER_TEMP/o.txt
echo "--- stderr:"; cat $RUNNER_TEMP/e.txt
adb shell -t $D/probe exit 1 >$RUNNER_TEMP/o.txt 2>$RUNNER_TEMP/e.txt </dev/null
echo "--- with -t: stdout=$(cat -A $RUNNER_TEMP/o.txt | tr '\n' '|') stderr=$(cat $RUNNER_TEMP/e.txt | tr '\n' '|')"

h "environment passing"
adb shell "SPIKE_VAR='hello world' RUST_BACKTRACE=1 $D/probe info" | grep -E "^env (SPIKE|RUST|HOME|TMPDIR|PATH|ANDROID|EXTERNAL)|^cwd|^uid|^isatty"
SPIKE_HOST_ONLY=1 adb shell $D/probe info | grep -c SPIKE_HOST_ONLY | sed 's/^/host-only variable seen on the device (count): /'
adb shell "cd /data/local/tmp && env -i PATH=/system/bin HOME=/data/local/tmp X=1 $D/probe info" | grep -E "^env|^cwd"

h "quoting: spaces, quotes and unicode in arguments"
adb shell $D/probe info "'a b'" "'c\"d'" "'é✓'" | grep '^args'
adb shell "$D/probe info 'a b' 'c\"d' 'é✓' '\$HOME'" | grep '^args'

h "stdin"
echo "piped stdin" | adb shell $D/probe stdin
echo "piped stdin" | adb shell -T $D/probe stdin

h "big output: 200000 lines"
s=$(t); adb shell $D/probe big 200000 >$RUNNER_TEMP/o.txt 2>$RUNNER_TEMP/e.txt; rc=$?; el $s
echo "lines=$(wc -l <$RUNNER_TEMP/o.txt) rc=$rc stderr=$(cat $RUNNER_TEMP/e.txt)"

h "latency: 10 x adb shell true, 10 x the probe"
s=$(t); for i in $(seq 10); do adb shell true; done; el $s
s=$(t); for i in $(seq 10); do adb shell $D/probe exit 0 >/dev/null 2>&1; done; el $s

h "kill: SIGTERM to the adb client while the probe sleeps"
adb shell $D/probe sleep 30 & cp=$!
sleep 2; kill -TERM $cp; sleep 2
adb shell "ps -A -o PID,NAME | grep probe || echo '(no probe process left)'"
adb shell pkill -9 -f $D/probe 2>/dev/null

h "push throughput"
for kind in random zero; do
  if [ $kind = random ]; then head -c 400M /dev/urandom >$RUNNER_TEMP/big.bin; else head -c 400M /dev/zero >$RUNNER_TEMP/big.bin; fi
  s=$(t); adb push $RUNNER_TEMP/big.bin $D/big.bin | tail -1; el $s
  s=$(t); adb push -z lz4 $RUNNER_TEMP/big.bin $D/big.bin 2>&1 | tail -1; el $s
  s=$(t); adb push --sync $RUNNER_TEMP/big.bin $D/big.bin 2>&1 | tail -1; echo "(--sync, unchanged)"; el $s
done
adb shell rm -f $D/big.bin; rm -f $RUNNER_TEMP/big.bin
adb shell df -h /data | tail -1

h "std::process::Command and paths, as root (adb root) vs shell"
adb shell id
adb shell $D/probe info | grep -E "^spawn|^uid"
