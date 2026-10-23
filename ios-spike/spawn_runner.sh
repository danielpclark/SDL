#!/bin/bash
# A cargo target runner: `xcrun simctl spawn $SPIKE_UDID PROGRAM ARGS...`,
# forwarding SPIKE_* and RUST_* as SIMCTL_CHILD_* (unless SPIKE_NO_FORWARD).
if [ -z "$SPIKE_NO_FORWARD" ]; then
  while IFS='=' read -r k _; do
    case "$k" in
      SPIKE_UDID|SPIKE_NO_FORWARD) ;;
      SPIKE_*|RUST_*) export "SIMCTL_CHILD_$k=${!k}" ;;
    esac
  done < <(env)
fi
echo "[spawn_runner] cwd=$PWD: xcrun simctl spawn $SPIKE_UDID $*" >&2
exec xcrun simctl spawn "$SPIKE_UDID" "$@"
