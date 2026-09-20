#!/usr/bin/env bash
# Startup / idle / memory qualification for the operations app.
#
# Distinctions kept deliberately (they are the point of the report):
#   - launch→first presentation  ≠  render wall time (the first
#     includes process start, store open, template injection, document
#     load, layout, and present; the second is one render call)
#   - current RSS (VmRSS)  ≠  peak (VmHWM)
#   - a bounded reload run establishes behavior over that run only —
#     it is not a universal leak-free guarantee
#
# Runtime baseline: the packaged RELEASE app, watching and inspection
# disabled. The development configuration (velqu-lab --watch at the
# pinned dev-tool revision) is measured separately so watcher activity
# is never attributed to the application.
#
# Lanes: X11 on this machine. Wayland: not available here (no
# compositor socket) — recorded, not claimed.
set -u
export DISPLAY="${DISPLAY:-:1}"

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
BIN="$ROOT/target/release/velqu-view-starter"
APP="$ROOT/ops"
LAB="$HOME/.velqu-tools/3c1ee343a71117984204c04be49c4536604e91b8/bin/velqu-lab"
WORK="$(mktemp -d /tmp/vv-qualify-XXXXXX)"
echo "qualify: work dir $WORK"

one_proc_field() { # pid file field
  awk -v key="$3:" '$1 == key { print $2 }' "/proc/$1/$2" 2>/dev/null
}

sample() { # label pid
  local pid="$2"
  [ -d "/proc/$pid" ] || { echo "$1: (process gone)"; return; }
  local rss hwm vcs nvcs
  rss=$(one_proc_field "$pid" status VmRSS)
  hwm=$(one_proc_field "$pid" status VmHWM)
  vcs=$(one_proc_field "$pid" status voluntary_ctxt_switches)
  nvcs=$(one_proc_field "$pid" status nonvoluntary_ctxt_switches)
  local cpu
  cpu=$(awk '{print ($14 + $15) * 10 " ms"}' "/proc/$pid/stat" 2>/dev/null)
  echo "$1: rss=${rss:-?}kB hwm=${hwm:-?}kB wakeups=${vcs:-?}v+${nvcs:-?}nv cpu=${cpu:-?}"
}

# --- A. launch → first presentation (release, 5 runs) --------------------
echo
echo "== A. launch → first presentation (release windowed, 5 runs) =="
for i in 1 2 3 4 5; do
  "$BIN" --app-dir "$APP" --data-dir "$WORK/data" --measure \
    --exit-after-ms 3000 2>/dev/null | grep first_present
done
echo "-- render wall time (headless first render, release, 3 runs) --"
for i in 1 2 3; do
  "$BIN" --app-dir "$APP" --data-dir "$WORK/data" --headless --measure \
    --out "$WORK/h.png" 2>/dev/null | grep first_render
done

# --- B. idle: wakeups / CPU / RSS over 60 s -------------------------------
idle_run() { # label command...
  local label="$1"; shift
  "$@" > "$WORK/idle-$label.log" 2>&1 &
  local pid=$!
  sleep 2
  sample "$label t=2s" "$pid"
  sleep 58
  sample "$label t=60s" "$pid"
  kill -TERM "$pid" 2>/dev/null
  wait "$pid" 2>/dev/null
}
echo
echo "== B. idle over 60 s (release app baseline) =="
idle_run release "$BIN" --app-dir "$APP" --data-dir "$WORK/data2" --exit-after-ms 63000
echo
echo "== B2. idle over 60 s (development configuration: velqu-lab --watch) =="
idle_run dev "$LAB" --tailwind --reactive --watch "$APP" --exit-after-ms 63000

# --- C. bounded reload run: 25 real saves through the window --------------
echo
echo "== C. bounded reload run: 25 saves via the windowed app =="
rm -rf "$WORK/data3"
"$BIN" --app-dir "$APP" --data-dir "$WORK/data3" --measure > "$WORK/reload.log" 2>&1 &
APP_PID=$!
sleep 1.5
FRAME=$(xdotool search --name "Gate Ops — velqu-view-starter" | head -1)
CLIENT=$(xwininfo -id "$FRAME" -children | awk '/1280x800/{print $1}' | head -1)
xdotool windowfocus --sync "$CLIENT"
xdotool mousemove --window "$CLIENT" 470 484 click 1   # select GT-2403
sleep 0.5
xdotool mousemove --window "$CLIENT" 1000 484 click 1  # focus name input
sleep 0.3
sample "reload t=0" "$APP_PID"
for i in $(seq 1 25); do
  xdotool key ctrl+a
  xdotool type --delay 15 "Qual Run $((i % 2))"
  xdotool mousemove --window "$CLIENT" 990 658 click 1  # Save to device
  sleep 0.55
  if [ $((i % 5)) -eq 0 ]; then sample "reload after $i" "$APP_PID"; fi
done
sleep 0.5
sample "reload final" "$APP_PID"
kill -TERM "$APP_PID" 2>/dev/null
wait "$APP_PID" 2>/dev/null
echo "saves published: $(grep -c 'ops: saved' "$WORK/reload.log")"
echo "last: $(grep 'ops: saved' "$WORK/reload.log" | tail -1)"
echo "-- bounded reload, development configuration: 25 css edits under --watch --"
"$LAB" --tailwind --reactive --watch "$APP" --exit-after-ms 45000 > "$WORK/devreload.log" 2>&1 &
DEV_PID=$!
sleep 5
cp "$APP/app.css" "$WORK/app.css.bak"
for i in $(seq 1 25); do
  printf '\n/* qual %s */\n.vv-row:hover { background: #e0e7ff; }\n' "$i" >> "$APP/app.css"
  sleep 1.2
done
kill -TERM "$DEV_PID" 2>/dev/null
wait "$DEV_PID" 2>/dev/null
cp "$WORK/app.css.bak" "$APP/app.css"
echo "dev reconciles: $(grep -c 'watch: reconciled' "$WORK/devreload.log")"
grep 'watch: reconciled' "$WORK/devreload.log" | tail -1

echo
echo "qualify: raw logs in $WORK"
