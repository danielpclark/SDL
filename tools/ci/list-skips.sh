#!/bin/sh
# List a test run's skipped checks (the SDL3_TEST_SKIP_LOG file:
# capability<TAB>test<TAB>reason lines, see docs/HARDWARE_TESTING.md) in
# the GitHub Actions job summary, with a count per capability.
#
#   tools/ci/list-skips.sh SKIP_LOG TITLE
set -eu

log=$1
title=$2
if [ -s "$log" ]; then
  {
    echo "### Skipped checks ($title)"
    echo
    echo "| capability | test | reason |"
    echo "|---|---|---|"
    sort -u "$log" | sed 's/|/\\|/g; s/\t/ | /g; s/^/| /; s/$/ |/'
  } >> "${GITHUB_STEP_SUMMARY:-/dev/stdout}"
  cut -f1 "$log" | sort | uniq -c
else
  echo "no checks skipped"
fi
