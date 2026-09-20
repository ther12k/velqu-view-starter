#!/usr/bin/env bash
# Development command for velqu-view-starter.
#
# Uses velqu-lab — VelquView's development host — installed from the
# pinned DEV-TOOL revision below into a revision-specific directory
# (so "a binary named velqu-lab already exists" can never silently
# mean the wrong revision), with --locked so the dependency
# resolution of the pinned tool revision is retained. This opens the
# app in a window with Tailwind compilation, Velqu Reactive, and hot
# reload over the app directory: save a file and the window reconciles
# on its own — no restart, no second save.
#
# Hot reload works as of dev-tool revision 3c1ee34 ("post-closure
# correction 0001"), which repaired live-window watch scheduling;
# at the older 1d0a562 tool the window never reconciled by itself.
#
# Usage:
#   ./dev.sh               the operations app (ops/) — the default
#   ./dev.sh ops           same as above
#   ./dev.sh dashboard     the M7 reference dashboard (app/)
#
# Dev mode keeps edits in memory: the velqu-lab host has no records
# store, and the app's rail says so. Durable saves are the packaged
# app's job (cargo run --release, or the built binary).
#
# Extra velqu-lab flags pass through, e.g.:
#   ./dev.sh --watch=poll        polling watcher (no inotify)
#   ./dev.sh --exit-after-ms 5000
#
# Inspection (headless only): velqu-lab prints the inspector snapshot
# and trace with --inspect, e.g.
#   "$TOOL_ROOT/bin/velqu-lab" --headless --inspect --tailwind --reactive ./ops
set -euo pipefail

here="$(cd "$(dirname "$0")" && pwd)"
REPO="https://github.com/ther12k/velqu-view.git"
# DEV-TOOL revision (keep in lockstep with README.md's revision table;
# the app's RUNTIME revision is pinned separately in Cargo.toml).
REV="3c1ee343a71117984204c04be49c4536604e91b8"
TOOL_ROOT="$HOME/.velqu-tools/$REV"
LAB="$TOOL_ROOT/bin/velqu-lab"

APP="ops"
DIR="$here/ops"
if [ "${1:-}" = "ops" ]; then
  shift
elif [ "${1:-}" = "dashboard" ]; then
  APP="dashboard"
  DIR="$here/app"
  shift
fi

if [ ! -x "$LAB" ]; then
  echo "dev: velqu-lab ${REV:0:7} not installed — building into $TOOL_ROOT"
  echo "dev: (one-time per revision; compiles the framework, takes a few minutes)"
  cargo install --locked --git "$REPO" --rev "$REV" --root "$TOOL_ROOT" velqu-lab
fi

exec "$LAB" --tailwind --reactive --watch "$DIR" "$@"
