#!/bin/sh
# Print the crash reports macOS wrote for the processes that crashed in this
# job (*.ips in ~/Library/Logs/DiagnosticReports, or the system's
# /Library/Logs/DiagnosticReports, whose names start with the process's),
# each one's exception and every thread's frames, the crashed
# one marked: what a test binary's "signal: 11, SIGSEGV" in cargo's output
# leaves out.
#
#   tools/ci/macos-crash-reports.sh [NAME_PREFIX]
#
# NAME_PREFIX (by default every report): only the reports of processes whose
# names start with it. An .ips file is a line of JSON (the report's header),
# then the report's JSON body; jq is on the macOS runner images.
set -eu

prefix=${1:-}
reports() {
  ls "$HOME/Library/Logs/DiagnosticReports/$prefix"*.ips \
    "/Library/Logs/DiagnosticReports/$prefix"*.ips 2> /dev/null || true
}

# (ReportCrash writes a report a few seconds after the crash)
tries=0
while [ "$tries" -lt 15 ]; do
  if [ -n "$(reports)" ]; then
    break
  fi
  tries=$((tries + 1))
  sleep 2
done

found=0
for f in $(reports); do
  [ -f "$f" ] || continue
  found=1
  echo "::group::$(basename "$f")"
  # (the system's reports may be root's only: the runner's sudo needs no
  # password)
  { tail -n +2 "$f" 2> /dev/null || sudo -n tail -n +2 "$f"; } | jq -r '
    .usedImages as $images
    | "process: \(.procName // "?") (\(.procPath // "?"))",
      "exception: \(.exception.type // "?") \(.exception.signal // "") \(.exception.subtype // "")",
      "faulting thread: \(.faultingThread // "?")",
      ((.threads // []) | to_entries[]
        | "-- thread \(.key) \(.value.name // .value.queue // "")\(if .value.triggered then " CRASHED" else "" end)",
          ((.value.frames // [])[:40][]
            | "   \(if .imageIndex == null then "?" else ($images[.imageIndex].name // "?") end) \(.symbol // "?") +\(.symbolLocation // .imageOffset // "")"))
  ' || echo "(not a report jq could read)"
  echo "::endgroup::"
done
if [ "$found" -eq 0 ]; then
  echo "no crash reports${prefix:+ for $prefix*}"
fi
