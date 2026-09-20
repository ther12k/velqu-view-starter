# velqu-view-starter

A **standalone application** built only on VelquView's public crates:
the gate-transaction **operations workflow** (find a record → inspect
its details → correct the permitted fields → validate → save to the
device → confirm), plus the M7 reference dashboard kept as the
framework-consumer conformance fixture. No framework checkout, no
engine internals — another developer can clone this repository, build
it, edit the HTML/Tailwind UI, and run it.

**Pinned revisions** (recorded separately; neither upgrades silently):

| What | Revision | Pinned where |
| --- | --- | --- |
| Runtime (`velqu-view` crate) | `1d0a562bccdcb21c657423ca8b568cc2beebff88` — the Phase-1 closure record's amendment commit (`docs/evidence/phase1-closure.md` in the framework repo) | `rev = …` in `Cargo.toml` + committed `Cargo.lock` |
| Development tool (`velqu-lab` binary) | `3c1ee343a71117984204c04be49c4536604e91b8` — "post-closure correction 0001", which repaired live-window watch scheduling | `dev.sh` (installs `--locked` into `~/.velqu-tools/<full-sha>`) |

## The workflow and what "save" means

The default app (`ops/`) is a gate-transaction review screen: 14
authored transactions (id, container, truck, direction, gate, time,
status) with two **permitted editable fields** per record — driver
name and notes — validated by fixed rules (name 1–80 characters after
trimming; notes ≤ 280) and saved **on the device**:

- The UI runtime never touches the disk. Clicking *Save to device*
  only asks the host to persist (the document shows "Saving…" — it
  never claims a save by itself).
- The **application host** (this binary) observes the public event
  queue, re-validates authoritatively, writes `records.json` in an
  explicit data directory (atomic tmp+rename), and reloads the
  document from persisted truth. The reloaded screen shows
  "Saved to device at `<UTC time>` — survives restart."
- A change that has not been saved is in-memory only. In dev mode
  (`./dev.sh` via velqu-lab there is no store) the rail says so
  explicitly: *"Dev mode — edits stay in memory"*.
- Storage is an **application-host concern**, deliberately: there is
  no server integration in this pilot. A future adapter (server sync,
  MQTT, …) would live beside `src/ops.rs`'s `RecordStore` — never in
  the renderer, and never as new runtime capabilities.

Default data directory: `$VELQU_VIEW_STARTER_DATA_DIR`, else
`~/.local/share/velqu-view-starter/records.json` (override with
`--data-dir`). The store holds corrections over the template's seed
values, so an empty store simply shows the authored dataset.

## Prerequisites

- Rust via rustup — this repo pins toolchain 1.96.0 in
  `rust-toolchain.toml` (rustup fetches it automatically; the
  framework's MSRV is 1.87).
- A Linux windowed session for window mode (winit + softbuffer).
  Headless modes need no display.
- Read access to the VelquView git repository (it may be private —
  your normal git credentials apply; `.cargo/config.toml` enables
  `git-fetch-with-cli` for exactly that).

## Develop

```sh
./dev.sh                # the operations app (ops/): window + hot reload
./dev.sh dashboard      # the M7 reference dashboard (app/)
```

One-time cost per revision: `dev.sh` installs `velqu-lab` (the
framework's development host) into a revision-specific directory,
with `--locked`, from the dev-tool revision above. Extra flags pass
through, e.g. `./dev.sh --exit-after-ms 5000` or
`./dev.sh --watch=poll`.

**Hot reload works**: save a file under the app directory and the
window reconciles on its own — one save, no restart, no extra input.
This required the framework repair in the dev-tool revision above; at
the older `1d0a562` tool the window never reconciled by itself (found
by this starter — see [docs/verification.md](docs/verification.md)).

Dev-mode edits stay in memory (velqu-lab has no records store); run
the packaged app for durable saves.

Headless inspection (snapshot + retained trace) and capture:

```sh
velqu-lab --headless --inspect --tailwind --reactive ./ops
```

## Release build and launch

```sh
cargo build --release
./target/release/velqu-view-starter              # ops workflow, window
./target/release/velqu-view-starter --app dashboard    # reference dashboard
./target/release/velqu-view-starter --headless   # offscreen: sha256 + PNG
```

No development watching, no inspector capture. App files resolve in
this order, so launching never depends on the current directory or
the framework checkout:

1. `--app-dir DIR` when given
2. `./ops` (or `./app` for `--app dashboard`) relative to the current
   directory
3. beside the executable, one and two levels up
4. the copies embedded into the binary at build time (last resort)

Fonts are embedded in the engine crate; the app itself needs no
asset files beyond `index.html` and `app.css`.

Useful flags: `--data-dir DIR` (store location), `--measure`
(launch→first-presentation timing), `--print-record GT-2403` (dump one
record's persisted state), `--smoke-save GT-2403 --name … --notes …`
(persist one correction without a window).

## Verify

```sh
cargo test
```

Two suites, deliberately separate:

- `tests/dashboard.rs` — the **framework-consumer checks** (unchanged
  from the accepted starter baseline): the M7 dashboard's initial
  frame must equal the accepted digest `a89813c5…` pixel-for-pixel at
  1280×800 @1.0, plus search/navigation/selection/edit-save/wheel.
- `tests/operations.rs` — the **real workflow** through the public API
  plus the starter's own host code: template/state structural
  contract, find/inspect, validation blocking, save→persist→confirm
  with session preservation, rejection handling, dev-mode honesty,
  and process-level durability (the binary run twice: save in one
  process, read back in a fresh one).

Diagnostic examples (`cargo run --example …`) document the hot-reload
finding — see the verification report.

## Where to edit the UI

| File | What lives there |
| --- | --- |
| `ops/index.html` | The workflow screen: structure, Tailwind utilities, `vx-*` reactive attributes, the seed `vx-state` block (strict one-pair-per-line format — parsed by `src/ops.rs`) |
| `ops/app.css` | Author CSS: `vv-*` hook classes and interaction styles (`:hover`, `:focus`) |
| `tools/gen_ops_html.py` | Authoring aid that generated the repetitive rows/panels; the committed `ops/index.html` is the source of truth (regenerating overwrites manual edits) |
| `src/ops.rs` | Host workflow: state parse/inject, `records.json` store, validation, save path |
| `src/window.rs` | The app-owned window loop (the host must observe public events; see its module docs) |
| `app/index.html` + `app/app.css` | The M7 reference dashboard — **byte-frozen conformance fixture; do not edit** |
| `tests/dashboard.rs`, `tests/operations.rs` | The two verification suites |
| `src/main.rs` | The host binary (apps, data dir, window/headless, smoke flags) |

## v0 profile limitations (inherited from the pinned revision)

- **Tailwind variants deferred:** `hover:`/`focus:` utilities are not
  compiled; style interaction states in `ops/app.css` instead. Classes
  starting with `vv-` are the app's own namespace — never compiled,
  never diagnosed.
- **`vx-for` deferred:** lists are authored rows filtered with
  `vx-show` predicates; there is one `vx-state` scope per document.
  Adding/removing *records* means regenerating or editing
  `ops/index.html` (the dataset is authored, not dynamic).
- **Search indexes the registered fields** (transaction, direction,
  gate, container, truck, status) plus the live driver name; notes are
  not searchable.
- **CSS subset:** v0 profile only — see the framework's
  `docs/reference-dashboard.md` at the pinned revision for the exact
  list. One documented engine deviation matters for layout: a scroll
  container (`overflow-auto`) with `flex-1` measures as `0px` flex
  basis rather than content height inside auto-height parents —
  recorded as a compatibility deviation in the Phase-1 closure record.
- **Hot reload** works via the dev-tool revision pinned in `dev.sh`
  (framework post-closure correction 0001); the runtime crates at
  `1d0a562` are unaffected by it.
- **Focus after save:** the host restores focus to the saved record's
  name field, but the caret position and any transient reactive state
  reset on the post-save document reload (state is re-seeded from
  persisted truth + preserved search/selection).
