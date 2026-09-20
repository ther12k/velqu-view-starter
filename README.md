# velqu-view-starter

The VelquView M7 reference dashboard (gate-operations screen) as a
**standalone application** built only on VelquView's public crates.
No framework checkout, no engine internals — this repository is the
external-consumer check: another developer can clone it, build it,
edit the HTML/Tailwind UI, and run it.

**Pinned baseline:** VelquView @ `1d0a562bccdcb21c657423ca8b568cc2beebff88`
(short id `1d0a562` — the Phase-1 closure record's amendment commit;
see `docs/evidence/phase1-closure.md` in the framework repository).
Git dependencies are pinned by full SHA in `Cargo.toml`, and the
committed `Cargo.lock` pins the entire dependency tree.

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
./dev.sh        # window + Tailwind + Velqu Reactive over ./app
```

One-time cost: `dev.sh` installs `velqu-lab` (the framework's
development host) from the same pinned revision. Extra flags pass
through, e.g. `./dev.sh --exit-after-ms 5000`.

**Hot reload is disabled at this revision.** `velqu-lab --watch`
never reconciles live (a debounce-clock defect in the dev host — full
diagnosis and proposed fix in
[docs/verification.md](docs/verification.md)). The dev loop is:
edit, close the window with `Esc`, rerun `./dev.sh`.

Headless inspection (snapshot + retained trace) and capture:

```sh
velqu-lab --headless --inspect --tailwind --reactive ./app
```

## Release build and launch

```sh
cargo build --release
./target/release/velqu-view-starter              # window
./target/release/velqu-view-starter --headless   # offscreen: sha256 + PNG
```

No development watching, no inspector capture. App files resolve in
this order, so launching never depends on the current directory or
the framework checkout:

1. `--app-dir DIR` when given
2. `./app` relative to the current directory
3. `app/` beside the executable, one and two levels up
4. the copies embedded into the binary at build time (last resort)

Fonts are embedded in the engine crate; the app itself needs no
asset files beyond `index.html` and `app.css`.

## Verify

```sh
cargo test
```

Drives the dashboard through VelquView's **public API** from this
crate: the initial frame must equal the accepted M7 baseline digest
`a89813c5…` pixel-for-pixel at 1280×800 @1.0, plus search filtering,
section navigation, row selection, edit/save, and wheel scrolling.
Diagnostic examples (`cargo run --example …`) document the hot-reload
finding — see the verification report.

## Where to edit the UI

| File | What lives there |
| --- | --- |
| `app/index.html` | Structure, Tailwind utility classes, `vx-*` reactive attributes (`vx-state`, `vx-show`, `vx-model`, `@click`, `:class`, `:disabled`) |
| `app/app.css` | Author CSS: `vv-*` hook classes and interaction styles (`:hover`, `:focus`) |
| `tests/dashboard.rs` | The public-API verification suite |
| `src/main.rs` | The host binary (window + headless, asset resolution) |

## v0 profile limitations (inherited from the pinned revision)

- **Tailwind variants deferred:** `hover:`/`focus:` utilities are not
  compiled; style interaction states in `app.css` instead. Classes
  starting with `vv-` are the app's own namespace — never compiled,
  never diagnosed.
- **`vx-for` deferred:** lists are authored rows filtered with
  `vx-show` predicates; there is one `vx-state` scope per document.
- **CSS subset:** v0 profile only — see the framework's
  `docs/reference-dashboard.md` at the pinned revision for the exact
  list. One documented engine deviation matters for layout: a scroll
  container (`overflow-auto`) with `flex-1` measures as `0px` flex
  basis rather than content height inside auto-height parents —
  recorded as a compatibility deviation in the Phase-1 closure record.
- **Hot reload (`--watch`) is defective** at this revision — see the
  verification report.
