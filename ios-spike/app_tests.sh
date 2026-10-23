#!/bin/bash
# UIKit app experiments: bundle, sign, install, launch.
set +e
U="$SPIKE_UDID"
B="$PWD/target/aarch64-apple-ios-sim/debug"
ID=org.libsdl.sdl3-rust.spike
APP="$RUNNER_TEMP/uikit_app.app"
h() { echo; echo "===== $*"; }
echo "host file" > /tmp/spike-hostfile.txt

h "marker section in the linked Mach-O"
./run.py section "$B/uikit_app"
otool -s __DATA __sdl3_runner "$B/uikit_app"
otool -l "$B/uikit_app" | grep -A5 LC_BUILD_VERSION | head -8
codesign -dv "$B/uikit_app" 2>&1 | head -5

h "bundle"
./run.py bundle "$B/uikit_app" "$APP" "$ID"
ls -la "$APP"

h "launch --console-pty: 120 ticks, exit 120, env, host paths"
SIMCTL_CHILD_SPIKE_VAR=forwarded-to-app ./run.py launch "$U" "$APP" "$ID" pty 120 --ticks 120 \
  --read "$GITHUB_WORKSPACE/README.md" --read /tmp/spike-hostfile.txt --read "$RUNNER_TEMP" --read "$HOME"

h "launch --console-pty: exit 3 after 5 ticks"
./run.py launch "$U" "$APP" "$ID" pty 120 --ticks 5 --exit 3

h "launch --console-pty: exit 0 after 5 ticks"
./run.py launch "$U" "$APP" "$ID" pty 120 --ticks 5 --exit 0

h "launch --console (no pty): exit 4 after 5 ticks"
./run.py launch "$U" "$APP" "$ID" console 120 --ticks 5 --exit 4

h "launch --stdout=/--stderr= files: exit 5 after 5 ticks"
./run.py launch "$U" "$APP" "$ID" files 60 --ticks 5 --exit 5

h "launch --console-pty: panic inside the CADisplayLink callback"
./run.py launch "$U" "$APP" "$ID" pty 120 --panic-in-tick
sleep 8; echo "--- crash reports:"; ls -la ~/Library/Logs/DiagnosticReports/ 2>&1 | tail -10
f=$(ls -t ~/Library/Logs/DiagnosticReports/uikit_app*.ips 2>/dev/null | head -1)
[ -n "$f" ] && { echo "--- $f (head)"; head -c 1500 "$f"; echo; python3 -c "
import json,sys
t=open(sys.argv[1]).read().split('\n',1)[1]; d=json.loads(t)
print('exception:', d.get('exception')); print('termination:', d.get('termination')); print('asi:', d.get('asi'))" "$f"; }

h "launch --console-pty with the scene manifest in Info.plist (app delegate without scene methods)"
./run.py bundle "$B/uikit_app" "$RUNNER_TEMP/uikit_scene.app" "$ID.scene" --scene >/dev/null
./run.py launch "$U" "$RUNNER_TEMP/uikit_scene.app" "$ID.scene" pty 60 --ticks 10 --exit 10

h "launch --console-pty without codesign (unsigned bundle after modifying the binary)"
cp -R "$APP" "$RUNNER_TEMP/unsigned.app"; codesign --remove-signature "$RUNNER_TEMP/unsigned.app/uikit_app"; rm -rf "$RUNNER_TEMP/unsigned.app/_CodeSignature"
codesign -dv "$RUNNER_TEMP/unsigned.app" 2>&1 | head -2
./run.py launch "$U" "$RUNNER_TEMP/unsigned.app" "$ID" pty 60 --ticks 5 --exit 6

h "launch --console-pty: in-process rotation (requestGeometryUpdateWithPreferences)"
./run.py launch "$U" "$APP" "$ID" pty 60 --ticks 110 --exit 0 --rotate

h "background: launch Preferences over the app, then relaunch the app"
./run.py launch "$U" "$APP" "$ID" pty 90 --wait-background 60 --exit 0 &
lp=$!
sleep 12
xcrun simctl launch "$U" com.apple.Preferences; echo "launch Preferences rc=$?"
sleep 5
xcrun simctl launch "$U" "$ID"; echo "relaunch rc=$?"
wait $lp
xcrun simctl terminate "$U" com.apple.Preferences

h "simulator log for the app (last 3 minutes, filtered)"
xcrun simctl spawn "$U" log show --last 3m --style compact --predicate 'process == "uikit_app"' 2>&1 | tail -60
