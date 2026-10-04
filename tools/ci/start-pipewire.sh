#!/bin/sh
# Start a PipeWire server and WirePlumber in the background, with a null
# audio sink and source, so the PipeWire audio and camera tests have a
# server to talk to on a machine without sound hardware (a CI runner).
#
#   export XDG_RUNTIME_DIR=/some/private/dir   # optional
#   tools/ci/start-pipewire.sh
#   cargo test --workspace
#
# The server listens on $XDG_RUNTIME_DIR/pipewire-0; without XDG_RUNTIME_DIR
# it uses (and prints) a fresh directory, which the tests then need too. On
# GitHub Actions the directory is also exported to later steps.
set -eu

if [ -z "${XDG_RUNTIME_DIR:-}" ]; then
    XDG_RUNTIME_DIR=$(mktemp -d)
    export XDG_RUNTIME_DIR
    echo "XDG_RUNTIME_DIR=$XDG_RUNTIME_DIR"
fi
mkdir -p "$XDG_RUNTIME_DIR"
chmod 700 "$XDG_RUNTIME_DIR"
if [ -n "${GITHUB_ENV:-}" ]; then
    echo "XDG_RUNTIME_DIR=$XDG_RUNTIME_DIR" >>"$GITHUB_ENV"
fi
log=$XDG_RUNTIME_DIR/pipewire.log

# WirePlumber needs a session bus. It gets a private one, which the tests
# don't see (they start their own buses where they need one).
DBUS_SESSION_BUS_ADDRESS=$(dbus-daemon --session --fork --print-address=1)
export DBUS_SESSION_BUS_ADDRESS

nohup pipewire >"$log" 2>&1 &
i=0
until pw-cli info 0 >/dev/null 2>&1; do
    i=$((i + 1))
    if [ $i -gt 100 ]; then
        echo "pipewire didn't start:" >&2
        cat "$log" >&2
        exit 1
    fi
    sleep 0.1
done
nohup wireplumber >>"$log" 2>&1 &

# The driver lists only nodes with a name and a description.
pw-cli create-node adapter '{ factory.name=support.null-audio-sink
    node.name=ci-sink node.description="CI sink" media.class=Audio/Sink
    object.linger=true audio.position=[FL FR] }' >/dev/null
pw-cli create-node adapter '{ factory.name=support.null-audio-sink
    node.name=ci-source node.description="CI source" media.class=Audio/Source
    object.linger=true audio.position=[FL FR] }' >/dev/null

# Wait for WirePlumber to pick the defaults.
i=0
until wpctl inspect @DEFAULT_AUDIO_SINK@ >/dev/null 2>&1; do
    i=$((i + 1))
    if [ $i -gt 100 ]; then
        echo "wireplumber didn't pick a default sink:" >&2
        cat "$log" >&2
        exit 1
    fi
    sleep 0.1
done
wpctl status
