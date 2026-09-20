#!/usr/bin/env bash
# Development command for velqu-view-starter.
#
# Uses velqu-lab — VelquView's development host — installed from the
# SAME pinned revision the application builds against, so the tooling
# and the runtime can never drift apart. One-time install; afterwards
# this opens the dashboard in a window with Tailwind compilation and
# Velqu Reactive over ./app.
#
# HOT RELOAD IS DISABLED AT THIS REVISION. velqu-lab's --watch never
# reconciles live: the debounce clock restarts on every wakeup while
# the startup rescan keeps the dirty set non-empty, so the 120ms quiet
# interval can never elapse inside a waiting event loop. See
# docs/verification.md ("Live hot reload defect") for the full
# diagnosis and the proposed fix. Until it lands, the dev loop is:
# edit, close the window (Esc), rerun ./dev.sh.
#
# Extra velqu-lab flags pass through, e.g.:
#   ./dev.sh --watch=poll          try the watcher anyway (after the fix)
#   ./dev.sh --exit-after-ms 5000
#
# Inspection (headless only): velqu-lab prints the inspector snapshot
# and trace with --inspect, e.g.
#   velqu-lab --headless --inspect --tailwind --reactive ./app
set -euo pipefail

here="$(cd "$(dirname "$0")" && pwd)"
REPO="https://github.com/ther12k/velqu-view.git"
# Keep in lockstep with the pinned revision in Cargo.toml.
REV="1d0a562bccdcb21c657423ca8b568cc2beebff88"

if ! command -v velqu-lab >/dev/null 2>&1; then
  echo "dev: velqu-lab not found — installing the VelquView dev host at rev ${REV:0:7}"
  echo "dev: (one-time; compiles the framework, takes a few minutes)"
  cargo install --git "$REPO" --rev "$REV" velqu-lab
fi

exec velqu-lab --tailwind --reactive "$here/app" "$@"
