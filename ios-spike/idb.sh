#!/bin/bash
# Optional: fb-idb taps into the spike app.
set +e
U="$SPIKE_UDID"
B="$PWD/target/aarch64-apple-ios-sim/debug"
ID=org.libsdl.sdl3-rust.spike
h() { echo; echo "===== $*"; }
t() { python3 -c 'import time;print(time.time())'; }
h "install idb-companion and fb-idb"
s=$(t)
brew tap facebook/fb > /dev/null 2>&1; brew install facebook/fb/idb-companion 2>&1 | tail -5
if ! command -v idb_companion; then
  X26=$(ls -d /Applications/Xcode_26*.app | grep -vi beta | sort -V | tail -1)
  CUR=$(xcode-select -p)
  echo "retrying the install with $X26 selected, then switching back to $CUR"
  sudo xcode-select -s "$X26" && brew install facebook/fb/idb-companion 2>&1 | tail -5
  sudo xcode-select -s "$CUR"; xcodebuild -version
fi
python3 -m venv "$RUNNER_TEMP/idbenv" && "$RUNNER_TEMP/idbenv/bin/pip" install -q fb-idb 2>&1 | tail -3
python3 -c "import time;print('idb install elapsed %.1fs' % (time.time()-$s))"
IDB="$RUNNER_TEMP/idbenv/bin/idb"
command -v idb_companion; idb_companion --version 2>&1 | head -3; "$IDB" --help 2>&1 | head -3
h "idb list-targets / describe"
perl -e 'alarm 60; exec @ARGV' "$IDB" list-targets 2>&1 | head
h "launch the app waiting for a touch, then idb ui tap"
./run.py bundle "$B/uikit_app" "$RUNNER_TEMP/idb.app" "$ID" > /dev/null
./run.py launch "$U" "$RUNNER_TEMP/idb.app" "$ID" pty 90 --wait-touch 60 --want-key --exit 0 &
lp=$!
sleep 15
perl -e 'alarm 60; exec @ARGV' "$IDB" ui tap --udid "$U" 100 300 2>&1; echo "tap rc=$?"
sleep 2
perl -e 'alarm 60; exec @ARGV' "$IDB" ui key --udid "$U" 4 2>&1; echo "key rc=$?"
sleep 2
perl -e 'alarm 60; exec @ARGV' "$IDB" ui text --udid "$U" "a" 2>&1; echo "text rc=$?"
perl -e 'alarm 60; exec @ARGV' "$IDB" ui describe-point --udid "$U" 100 300 2>&1 | head -5
wait $lp
