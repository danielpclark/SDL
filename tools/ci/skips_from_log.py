#!/usr/bin/env python3
"""Make a skip log (SDL3_TEST_SKIP_LOG's capability<TAB>test<TAB>reason
lines, see docs/HARDWARE_TESTING.md) from test runs' output, for the
programs that can't write the file themselves (Emscripten's, in their own
in-memory file system; the iOS Simulator's apps, in their containers): the
`note: skipping, CAPABILITY unavailable: REASON` lines they print (with
RUST_TEST_NOCAPTURE=1), each given the test libtest names next, `test NAME
... ` (the programs without the test harness print such lines too).

  tools/ci/skips_from_log.py LOG... > SKIP_LOG

A log that doesn't exist counts as empty. (With tests running in parallel
a note may be put on a test that ended meanwhile, and may come in the
middle of libtest's line for another test, after its `test NAME ... `:
it is taken from anywhere in a line.)
"""
import sys


def skips(path):
    try:
        log = open(path, errors="replace")
    except FileNotFoundError:
        return
    pending = []
    with log:
        for line in log:
            line = line.rstrip("\n").rstrip("\r")
            if "note: skipping, " in line:
                line, note = line.split("note: skipping, ", 1)
                if " unavailable: " in note:
                    pending.append(note.split(" unavailable: ", 1))
            if line.startswith("test ") and " ... " in line:
                for capability, reason in pending:
                    yield capability, line.split()[1], reason
                pending = []


def main():
    if len(sys.argv) < 2:
        print(__doc__.split("\n\n", 2)[1], file=sys.stderr)
        return 2
    for path in sys.argv[1:]:
        for capability, test, reason in skips(path):
            print(f"{capability}\t{test}\t{reason}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
