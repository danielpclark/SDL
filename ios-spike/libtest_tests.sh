#!/bin/bash
# cargo test through a simctl-spawn target runner.
set +e
export CARGO_TARGET_AARCH64_APPLE_IOS_SIM_RUNNER="$PWD/spawn_runner.sh"
h() { echo; echo "===== $*"; }
t() { python3 -c 'import time;print(time.time())'; }

h "cargo test (all: unit, integration, doctest), forwarded env SPIKE_VAR"
s=$(t); SPIKE_VAR=hello cargo test --target aarch64-apple-ios-sim -- --nocapture 2>&1; echo "rc=$?"
python3 -c "import time;print('elapsed %.1fs' % (time.time()-$s))"

h "cargo test --test probe with SPIKE_FAIL=1 (a failing test: exit status)"
SPIKE_FAIL=1 cargo test --target aarch64-apple-ios-sim --test probe 2>&1; echo "rc=$?"

h "cargo test --test probe without forwarding (SPIKE_FAIL=1 but SPIKE_NO_FORWARD=1)"
SPIKE_NO_FORWARD=1 SPIKE_FAIL=1 cargo test --target aarch64-apple-ios-sim --test probe 2>&1 | tail -15; echo "rc=${PIPESTATUS[0]}"

h "cargo test -- --list and --include-ignored"
cargo test --target aarch64-apple-ios-sim --test probe -- --list 2>&1 | tail -12
cargo test --target aarch64-apple-ios-sim --test probe -- --include-ignored ignored 2>&1 | tail -6

h "cargo test --doc only"
cargo test --target aarch64-apple-ios-sim --doc -- --nocapture 2>&1 | tail -15; echo "rc=${PIPESTATUS[0]}"

h "libtest json format (unstable via RUSTC_BOOTSTRAP)"
RUSTC_BOOTSTRAP=1 cargo test --target aarch64-apple-ios-sim --test probe -- -Zunstable-options --format json 2>&1 | head -8
