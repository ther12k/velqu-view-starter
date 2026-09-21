# velqu-view-starter — verification report

Date: 2026-09-20 · Machine: Linux 6.x, X11 (`DISPLAY=:1`), rustup
toolchain 1.96.0.

## 1. Scope and pinned baseline

The claim under test: **another developer can build and run an
application on VelquView without the framework's development workspace
or private implementation details.**

- VelquView revision: `1d0a562bccdcb21c657423ca8b568cc2beebff88`
  (short `1d0a562` — the Phase-1 closure record's amendment-identity
  commit; the closure record itself is
  `docs/evidence/phase1-closure.md` in the framework repository).
  Pinned by full SHA as a `rev = …` git dependency for `velqu-view`
  and `velqu-shell` in `Cargo.toml`; `Cargo.lock` (committed) pins
  the full dependency tree.
- Boundary: **public crates only.** No engine code copied, no private
  APIs, no `[patch]` or source replacement (the framework's
  dependencies are all public on crates.io at this revision). The
  framework repository was not modified.
- The dev host `velqu-lab` is consumed as a binary via
  `cargo install --git … --rev … velqu-lab` (same revision), never as
  a library.
- Application assets (`app/index.html`, `app/app.css`) are copied from
  `examples/reference-dashboard` at `1d0a562`; the digest identity
  below proves they render byte-identically.

## 2. Verification runs

| Check | Command | Result |
| --- | --- | --- |
| Clean build (debug) | `cargo build` | ok, 1m10s — git deps fetched from the (private) repo via CLI credentials |
| Release build | `cargo build --release` | ok, 2m21s |
| Public-API suite | `cargo test` | **6/6 passed** |
| Window smoke | `./target/release/velqu-view-starter --exit-after-ms 4000` | ok — 31 frames presented in 4.0s |
| Launch from unrelated dir (exe-relative assets) | `cd /tmp && …/target/release/velqu-view-starter --headless --out /tmp/…` | ok — resolved `app/` via the executable path; `sha256: a89813c5…` |
| Launch with **no app dir anywhere** (embedded fallback) | bare binary copied to `/tmp`, run there | ok — `app: embedded build-time copy`; same `sha256: a89813c5…` |
| Dev wrapper | `./dev.sh --exit-after-ms 4000` | ok — uses installed `velqu-lab`; 2 frames in 4.0s |
| Dev host install | `cargo install --git … --rev 1d0a562… velqu-lab` | ok — 2m34s, `~/.cargo/bin/velqu-lab` |

**Digest identity.** The starter's initial frame at 1280×800 @1.0
equals the accepted M7 baseline
`a89813c53c651e717abfd7926bcac5974149440005055bf1f45f93cd747a8c5c`
(frozen in the framework's conformance suite at the pinned revision)
— reproduced in the `cargo test` suite, in headless CLI runs, from an
unrelated working directory, and from the embedded fallback. The
external git-dependency pipeline renders pixel-for-pixel like the
framework workspace build.

**Interactions through the public API** (`tests/dashboard.rs`, all
through `velqu_view` as an external consumer): search filtering
updates the summary and hides/shows rows together; section navigation
switches `vx-show` sections; row selection swaps the detail panel and
the hit-testable edit inputs; editing emits `ValueChanged`, flips the
unsaved counter, and Save restores it; wheel over the records list
emits `Scrolled` with a clamped offset. Pointer positions come from
hit-test scans, never fixed coordinates.

## 3. Live hot reload defect (the concrete blocker)

**Claim: `velqu-lab --watch` never reconciles a live window at this
revision** — for any backend, any number of saves, with or without
unrelated input. This is the open release-qualification item
("live-window wakeup counts under `--watch`") from the Phase-1
closure record, now with a root cause.

### Evidence

Seven lab runs (installed binary **and** a rebuild from the framework
checkout at `1d0a562`, which behaves identically): native and poll
backends; single save, double save 2s apart, unregistered-file write,
metadata touch, and synthetic mouse input — **zero reconciles** in
every run, while each half of the chain works standalone:

- `examples/watch_probe.rs`: sends `ShellUserEvent::Watch` from a
  thread at t=3/6/9s through the **public** `velqu_shell::
  run_with_watch` — the hook runs within ~10ms of every send. The
  shell's proxy→hook path is sound.
- A standalone `notify` 8.2 probe on the same directory receives
  `Modify(Data)` inotify events for appends. Event delivery is sound.
- `examples/chain_probe.rs`: replicates the lab's exact composition
  (notify watcher → wake closure → proxy → hook) on the public API,
  with a trace at every hop — all hops fire and the hook runs at
  edit time.

### Root cause (both parts inside velqu-lab, not the library)

1. **The debounce clock can never elapse.** The startup rescan leaves
   the coordinator's dirty set non-empty (it drains only inside a
   reconcile), and `Coordinator::notify` restarts the quiet-interval
   clock (`last_change_ms := now`) on *every* wakeup while the dirty
   set is non-empty — even when the notification matched no registered
   path. The hook runs only at wake instants, so `now − last_change`
   is ~0 at every invocation and `due()` (≥120ms) never holds.
2. **Nothing re-wakes the loop after the quiet interval.** Under
   `ControlFlow::Wait` the loop sleeps until an event; even with (1)
   fixed, a single save would need a second wake ≥120ms later, and no
   timer or delayed re-send exists. (Synthetic input does not help:
   the shell runs the hook only on a Watch wakeup, not on input.)

The M6c evidence base is logical tests with synthetic clocks (`now_ms`
driven past the quiet window), which is why this survived review; live
wake timing was explicitly the deferred qualification item.

### Smallest proposed change (framework-side, no public API change)

In `velqu-lab`: (a) `Coordinator::notify` updates `last_change_ms`
only when the notification actually marked something (a rescan or ≥1
registered-path match); (b) the lab's wake closure — it already holds
the loop proxy — schedules one delayed re-send of
`ShellUserEvent::Watch` at `quiet_ms + ε` after each notification, so
the quiet interval can elapse inside a waiting loop. Suggested
regression: a live smoke that edits a registered file and asserts a
published reload within ~1s with no input events.

### Starter impact

`dev.sh` runs `velqu-lab` **without** `--watch` and documents the
restart loop (edit → `Esc` → rerun). No framework scope was widened
and no workaround host was built — the defect is reported for a
future framework slice, per the external-consumer contract.

## 4. Boundaries of this verification

- Single machine, single session type (X11). Wayland and other
  platforms unverified here.
- Windowed presentation was smoke-tested (frames presented, clean
  auto-close); interactive input was driven programmatically through
  the public API, not scripted GUI input.
- No startup/latency or memory measurements here — those belong to
  the separate release-qualification track against this starter, not
  to the starter's acceptance.
- The embedded fallback equals the on-disk assets as of the build;
  editing files requires a rebuild only when no `app/` directory is
  found.

## 5. Resolution (2026-09-20): defect repaired, dev tool repinned

The §3 defect was repaired in the framework as **post-closure
correction 0001** (commit `3c1ee343a71117984204c04be49c4536604e91b8`,
CI green; `docs/evidence/post-closure-watch-scheduling.md` there). The
repair separates relevant filesystem notifications (which move the
debounce deadline) from deadline wakes (which only check it), adds a
bounded re-armable scheduler inside velqu-lab, and is gated by a live
single-save/no-input regression that fails on `1d0a562` and passes on
the repair (native and polling backends), plus burst, unregistered
noise, rejected-then-repair, and shutdown-with-pending-timer
scenarios. X11 verified; Wayland lane unavailable on this machine.

Consequences for this starter:

- `dev.sh` now pins the **dev-tool revision**
  `3c1ee343a71117984204c04be49c4536604e91b8`, installs with
  `--locked` into `~/.velqu-tools/<full-sha>` (one install per
  revision — a stale velqu-lab elsewhere on PATH can never be
  picked up), and runs with `--watch`: hot reload works — one save,
  no restart.
- The **runtime revision stays `1d0a562…`** (Cargo.toml/Cargo.lock
  unchanged): the repair touched only the lab. `cargo test` still
  pins the accepted M7 baseline digest.

Verified through this starter's own `dev.sh`: window opened with
`--watch`, one save to `app.css` with no further input, the window
reconciled on its own (`watch: reconciled kind=Stylesheets
generation=None`), clean exit. The startup rescan pass counted as one
unchanged-skip reconciliation, exactly as designed.

## 6. Operations workflow + qualification (2026-09-21)

### 6.1 The workflow pilot (commit `cd0d3e7`)

The first real operator journey, per the accepted work order: find a
record → inspect its details → change the permitted fields → validate
→ save → confirm. Business content: gate-transaction review &
correction (`ops/`, 14 authored transactions; editable driver name +
notes; rules: name 1–80 chars after trim, notes ≤ 280).

**Save semantics** (the order's central requirement): the UI runtime
never touches the disk — a Save click only marks the document
"Saving…". The application host observes the public event queue,
re-validates authoritatively, writes `records.json` (atomic
tmp+rename) into an explicit data dir, and reloads the document from
persisted truth with the host timestamp shown ("Saved to device at …
— survives restart."). Unsaved edits are in-memory only; dev mode
(velqu-lab) has no store and the rail says so. No server integration
exists in the pilot; a future host adapter lives beside `RecordStore`,
not in the renderer.

**Why the host owns its window loop** (`src/window.rs`):
`velqu_shell::run` forwards input to the view and drops the event
batch by design, so no host can observe Save clicks through it. The
starter's loop is public-API only (winit + softbuffer + arboard +
VelquView), mirrors the shell's semantics (Wait control flow,
AltGr-safe routing, IME/clipboard via public traits), and adds the
one thing the workflow needs: the drained batch is handed to the host
hook between the reactive pump and the render. `velqu-shell` remains
a dev-dependency for the §3 probes only.

Evidence:

- `tests/operations.rs` (10 tests, public API + host code): template
  structural contract; find/inspect (live-name search); validation
  blocking (empty name, 281-char notes, disabled-save click cannot
  even request a save); save→persist→confirm with search/selection
  preserved and an earlier session's override intact; rejection
  handling keeps drafts on screen; dev-mode document never claims a
  durable save; process-level durability (binary run twice — save in
  one process, `--print-record` in a fresh one, plus a headless
  startup over the populated store); single-writer lock enforcement via
  `flock` preventing concurrent process corruption; corrupt-store
  preservation (corrupted `records.json` causes visible startup abort
  without deleting or overwriting the damaged file); save success
  versus UI reload failure split (disk write is authoritative, subsequent
  events do not re-save); and Unicode scalar counting + non-ASCII/HTML
  entity round-trip testing (accents, emojis, quotes, tags, and
  backslashes without parsing side-effects).
- Windowed end-to-end on X11 with real input (xdotool): row select →
  ctrl+a → type → Save click → `ops: saved gt-2403 at … (generation
  2)` on stdout, `records.json` written, on-screen confirmation
  banner carrying the same timestamp. This drive found and fixed a
  match-scrutinee `RefCell` double-borrow that crashed the host after
  the first successful save.
- `tests/dashboard.rs` and `app/` are byte-unchanged: the
  framework-consumer evidence (M7 baseline digest) survives the
  business-content change, as required.
- Dev loop: `./dev.sh` (ops default) hot-reloads through the pinned
  dev tool — `watch: reconciled kind=Stylesheets …` after a CSS edit,
  native backend, X11.

### 6.2 Qualification (release app, watching/inspection disabled)

Via `tools/qualify.sh` (committed; raw logs in its work dir). Machine:
the development workstation, X11 lane (`:1`). **Wayland lane
unavailable** (no compositor socket) — not claimed. Distinctions kept:
launch→first presentation ≠ render wall time; current RSS ≠ peak; a
bounded reload run establishes behavior over that run only.

**A. Launch → first presentation** (release, windowed, 5 runs):
91 / 110 / 135 / 103 / 171 ms (median ≈ 110 ms) at 1280×800@1.0.
The measurement endpoint is completion of `softbuffer::Buffer::present()`
(an application-side presentation milestone; it does not measure
physical photon emission on glass).
Render wall time (headless, single first render, 3 runs): 12–14 ms.
The remaining ~100 ms difference represents overall process startup
(OS execution, store initialization, template injection, DOM parse,
Tailwind compilation, layout, and presentation setup), without
speculating on individual uninstrumented proportions.

**B. Idle, 60 s, release app**: RSS 59,408 → 59,488 kB (+80 kB,
HWM equal — no sustained sampled RSS growth during the 60-second idle
run). Wakeup metrics track Linux `/proc/$PID/status` context switches:
in a dedicated, undisturbed 12-second observation window, the release
process logged **0 voluntary context switches** and **0 ms CPU**,
confirming `ControlFlow::Wait` suspends the process. Over the full
60-second observation in an active desktop session, 290 voluntary + 40
nonvoluntary switches (~5.7/s) occurred due to ambient X11 display
server events; total CPU over 60 s was 290 ms (≤0.5%).

**B2. Idle, 60 s, development configuration** (velqu-lab `--watch`,
dev-tool rev `3c1ee34`): RSS flat 56,608 kB; context switches 593
voluntary (≈10/s under the native inotify watcher, attributed to the
dev tool's event loop, not the application); CPU 290 ms ≈ 0.5%.

**C. Bounded reload run** (25 Save cycles through the windowed app —
each cycle = full host path: validate → persist → `reload_bundle` →
render; real keyboard/pointer input): 24 saves published (generation
reached 25; one click raced its 0.55 s step budget), confirmed in
stdout and `records.json`. RSS: 59,488 kB at t=0 → 98,604 kB by save 5
→ **exactly flat at 98,604 kB through save 25** (HWM equal). RSS
increased during the first five saves and then plateaued through save
25; the cause of the retained resident memory has not been isolated.
Over this run the reload path is memory-bounded; it is not a universal
leak-free guarantee. CPU ≈ 170 ms per type+save+reload+present cycle.

**C2. Bounded reload, development configuration** (25 CSS edits, one
per ~1.2 s, under `--watch`): 25/25 reconciles published
(`reconciliations: 26`, no deferrals) — the repaired watcher holds up
under a sustained edit loop on the ops app as well.

### 6.3 Observations (not fixed here, recorded for scoping)

- `velqu-lab --watch` with a **relative** app-dir registers relative
  source paths while native notifications arrive as absolute paths,
  so nothing matches and no reconcile is ever scheduled. All
  supported entry points (dev.sh) pass absolute paths; the live-watch
  regression tests use absolute temp dirs. A framework-side
  canonicalization fix would be its own small post-closure change.
- Layout-facts `text_runs` join without inter-run spacing, so
  multi-word assertions in public-API tests must normalize whitespace
  (the rendered rasters are correct — verified visually).
