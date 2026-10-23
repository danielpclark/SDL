#!/bin/bash
# Wait for a background `run.py boot > $RUNNER_TEMP/boot.log` and print the UDID.
for i in $(seq 1 180); do
  tail -1 "$RUNNER_TEMP/boot.log" 2>/dev/null | grep -qE '^[0-9A-F-]{36}$' && break
  sleep 5
done
cat "$RUNNER_TEMP/boot.log" >&2
tail -1 "$RUNNER_TEMP/boot.log"
