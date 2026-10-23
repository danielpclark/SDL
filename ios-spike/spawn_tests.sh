#!/bin/bash
# simctl spawn experiments. Needs SPIKE_UDID and the built spike binaries.
set +e
U="$SPIKE_UDID"
B="$PWD/target/aarch64-apple-ios-sim/debug"
to() { perl -e 'alarm shift; exec @ARGV' "$@"; }
h() { echo; echo "===== $*"; }

echo "host file from the runner" > "$RUNNER_TEMP/hostfile.txt"
echo "host file in /tmp" > /tmp/spike-hostfile.txt
mkdir -p "$HOME/spike-home-dir" && echo "home file" > "$HOME/spike-home-dir/f.txt"

h "spawn info: env forwarding (SIMCTL_CHILD_SPIKE_VAR, plain SPIKE_PLAIN), cwd, host paths"
( cd "$RUNNER_TEMP" && SIMCTL_CHILD_SPIKE_VAR=forwarded SPIKE_PLAIN=not-prefixed SIMCTL_CHILD_RUST_BACKTRACE=full \
  to 60 xcrun simctl spawn "$U" "$B/spawn_probe" info "$GITHUB_WORKSPACE" "$GITHUB_WORKSPACE/README.md" \
  "$RUNNER_TEMP/hostfile.txt" "$RUNNER_TEMP" /tmp/spike-hostfile.txt /tmp /private/tmp "$HOME/spike-home-dir/f.txt" \
  "$HOME" /Users /Applications /usr/bin /bin/sh relative-path.txt \
  > "$RUNNER_TEMP/spawn-out.txt" 2> "$RUNNER_TEMP/spawn-err.txt" ); echo "rc=$?"
echo "--- stdout:"; cat "$RUNNER_TEMP/spawn-out.txt"; echo "--- stderr:"; cat "$RUNNER_TEMP/spawn-err.txt"
echo "--- host-side: files written from the simulator:"; ls -la "$RUNNER_TEMP"/spike-write-probe.txt /tmp/spike-write-probe.txt "$GITHUB_WORKSPACE"/spike-write-probe.txt 2>&1

h "spawn: argument with spaces and unicode"
to 60 xcrun simctl spawn "$U" "$B/spawn_probe" info "a b  c" "ünïcødé" 2>/dev/null | grep '^args'; echo "rc=${PIPESTATUS[0]}"

h "spawn: from a terminal-less parent, isatty"
to 60 xcrun simctl spawn "$U" "$B/spawn_probe" info < /dev/null 2>/dev/null | grep isatty

for c in 0 3 255 256; do
  h "spawn: exit $c"
  to 60 xcrun simctl spawn "$U" "$B/spawn_probe" exit $c; rc=$?; echo; echo "rc=$rc"
done
h "spawn: panic"
to 60 xcrun simctl spawn "$U" "$B/spawn_probe" panic; echo "rc=$?"
h "spawn: panic with RUST_BACKTRACE forwarded"
SIMCTL_CHILD_RUST_BACKTRACE=1 to 60 xcrun simctl spawn "$U" "$B/spawn_probe" panic 2>&1 | head -20; echo "rc=${PIPESTATUS[0]}"
h "spawn: abort"
to 60 xcrun simctl spawn "$U" "$B/spawn_probe" abort; echo "rc=$?"
h "spawn: SIGSEGV"
to 60 xcrun simctl spawn "$U" "$B/spawn_probe" segv; echo "rc=$?"
h "spawn: 200000 lines of stdout"
start=$(python3 -c "import time;print(time.time())")
n=$(to 120 xcrun simctl spawn "$U" "$B/spawn_probe" big 200000 2>/dev/null | wc -l); echo "lines=$n rc=${PIPESTATUS[0]}"
python3 -c "import time;print('elapsed %.2fs' % (time.time()-$start))"
h "spawn: stdin"
echo "hello from stdin" | to 30 xcrun simctl spawn "$U" "$B/spawn_probe" stdin; echo "rc=$?"
h "spawn: startup latency (exit 0, 5 runs)"
for i in 1 2 3 4 5; do python3 - "$U" "$B/spawn_probe" <<'PY'
import subprocess, sys, time
t = time.monotonic(); subprocess.run(["xcrun", "simctl", "spawn", sys.argv[1], sys.argv[2], "exit", "0"], capture_output=True)
print("spawn round trip %.3fs" % (time.monotonic() - t))
PY
done
h "spawn: startup latency through the simctl binary directly (no xcrun), 3 runs"
SIMCTL=$(xcrun -f simctl); echo "simctl=$SIMCTL"
for i in 1 2 3; do python3 - "$SIMCTL" "$U" "$B/spawn_probe" <<'PY'
import subprocess, sys, time
t = time.monotonic(); subprocess.run([sys.argv[1], "spawn", sys.argv[2], sys.argv[3], "exit", "0"], capture_output=True)
print("direct simctl spawn round trip %.3fs" % (time.monotonic() - t))
PY
done
h "spawn: xcrun alone (overhead)"
python3 -c "import subprocess,time;t=time.monotonic();subprocess.run(['xcrun','-f','simctl'],capture_output=True);print('xcrun -f simctl %.3fs'%(time.monotonic()-t))"
h "spawn: killing simctl spawn (SIGTERM) -- does the child die?"
xcrun simctl spawn "$U" "$B/spawn_probe" sleep 60 & sp=$!
sleep 4; echo "host pgrep before:"; pgrep -fl "spawn_probe sleep" ; kill -TERM $sp; sleep 3
echo "host pgrep after SIGTERM to simctl:"; pgrep -fl "spawn_probe sleep" || echo "(child gone)"
pkill -f "spawn_probe sleep"
h "spawn: killing simctl spawn (SIGINT)"
xcrun simctl spawn "$U" "$B/spawn_probe" sleep 60 & sp=$!
sleep 4; kill -INT $sp; sleep 3
pgrep -fl "spawn_probe sleep" || echo "(child gone)"
pkill -f "spawn_probe sleep"
h "spawn: a binary for the device target (aarch64-apple-ios) in the simulator"
to 60 xcrun simctl spawn "$U" "$PWD/target/aarch64-apple-ios/debug/spawn_probe" exit 0; rc=$?; echo; echo "rc=$rc"
h "spawn: the simulator's own processes visible on the host?"
ps -axo pid,command | grep -i "CoreSimulator/Devices/$U" | head -5
