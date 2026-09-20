//! velqu-view-starter — a standalone, external-consumer application
//! built only on VelquView's public crates.
//!
//! Two apps ship in this repository:
//!
//! * **`ops` (default)** — the gate-transaction review workflow: find a
//!   record, inspect it, correct the permitted fields (driver name,
//!   notes), validate, save **to the device** (`records.json` in an
//!   explicit data directory, written by this host — the UI runtime
//!   never touches the disk), and confirm the saved result. Saves
//!   survive a restart; in dev mode (`./dev.sh`, velqu-lab) edits stay
//!   in memory and the document says so.
//! * **`dashboard`** — the M7 reference dashboard, kept byte-identical
//!   as the framework-consumer conformance fixture (its tests pin the
//!   accepted baseline digest).
//!
//! This binary is the **release host**: Tailwind + reactive enabled, a
//! native window through the app-owned loop ([`velqu_view_starter::
//! window`] — the host must observe public events for durable saves),
//! no development watching, no inspector capture. Development (hot
//! reload, inspection, headless capture) goes through `velqu-lab` at
//! the pinned dev-tool revision; see `dev.sh` and `README.md`.
//!
//! App files are located so launching never depends on the current
//! directory: `--app-dir DIR` when given, then `./<app>` relative to
//! the current directory, then beside the executable (one and two
//! levels up, covering `target/release/` and `target/` layouts), then
//! the copies embedded into the binary at build time.

use std::cell::RefCell;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::rc::Rc;
use std::time::{Duration, Instant};

use velqu_view::{
    Asset, AssetRequest, AssetResolver, DocumentSource, Event, SourceId, StylesheetSource,
    VelquView, Viewport,
};
use velqu_view_starter::ops::{self, BuildOpts, Flash, RecordStore, SaveOutcome, Session};
use velqu_view_starter::window::{self, FrameEventsHook, HostWindowConfig};

const EMBEDDED_DASHBOARD_HTML: &str = include_str!("../app/index.html");
const EMBEDDED_DASHBOARD_CSS: &str = include_str!("../app/app.css");
const EMBEDDED_OPS_HTML: &str = include_str!("../ops/index.html");
const EMBEDDED_OPS_CSS: &str = include_str!("../ops/app.css");

/// Which bundled app to run.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AppKind {
    /// Gate-transaction review workflow (default).
    Ops,
    /// M7 reference dashboard (conformance fixture).
    Dashboard,
}

impl AppKind {
    fn dir_name(self) -> &'static str {
        match self {
            AppKind::Ops => "ops",
            AppKind::Dashboard => "app",
        }
    }

    fn parse(value: &str) -> Result<Self, String> {
        match value {
            "ops" => Ok(AppKind::Ops),
            "dashboard" => Ok(AppKind::Dashboard),
            other => Err(format!(
                "unknown app {other:?} (expected `ops` or `dashboard`)"
            )),
        }
    }
}

struct Args {
    app: AppKind,
    app_dir: Option<PathBuf>,
    data_dir: Option<PathBuf>,
    headless: bool,
    size: (u32, u32),
    scale: f32,
    out: Option<PathBuf>,
    frames: u32,
    exit_after: Option<Duration>,
    measure: bool,
    /// `--smoke-save GT-2401`: run the host save path once and exit
    /// (used by the process-durability test).
    smoke_save: Option<String>,
    smoke_name: Option<String>,
    smoke_notes: Option<String>,
    /// `--print-record GT-2401`: print one record's persisted state
    /// and exit.
    print_record: Option<String>,
}

const USAGE: &str = "\
velqu-view-starter — gate-transaction review app on VelquView

USAGE:
    velqu-view-starter [OPTIONS]

OPTIONS:
    --app NAME         Bundled app: `ops` (default) or `dashboard`
    --app-dir DIR      App directory containing index.html (default:
                       searched: ./<app>, beside the executable, then the
                       embedded build-time copy)
    --data-dir DIR     Ops: where records.json lives (default:
                       $VELQU_VIEW_STARTER_DATA_DIR, else
                       ~/.local/share/velqu-view-starter)
    --headless         Render offscreen and exit; no window
    --out PATH         Headless: PNG output path (default <app>/out/frame.png)
    --frames N         Headless: render N frames, verify determinism (default 1)
    --size WxH         Logical window size (default 1280x800)
    --scale F          DPI scale factor (default 1.0)
    --exit-after-ms N  Window: auto-close after N milliseconds (smoke tests)
    --measure          Print `measure:` lines (launch to first presented
                       frame, exit stats)
    --smoke-save ID    Ops: persist one record (--name/--notes) and exit
    --name VALUE       --smoke-save: the corrected driver name
    --notes VALUE      --smoke-save: the corrected notes
    --print-record ID  Ops: print `record: ID | driver | notes | saved_at`
                       and exit
    -h, --help         Print this help

Save semantics: only this host writes records.json — clicking Save in
the UI asks the host to validate and persist; a change that has not
been saved is in-memory only. Dev mode (hot reload) is not in this
binary; run ./dev.sh, which uses velqu-lab from the pinned dev-tool
revision.
";

fn parse_args() -> Result<Args, String> {
    let mut args = Args {
        app: AppKind::Ops,
        app_dir: None,
        data_dir: None,
        headless: false,
        size: (1280, 800),
        scale: 1.0,
        out: None,
        frames: 1,
        exit_after: None,
        measure: false,
        smoke_save: None,
        smoke_name: None,
        smoke_notes: None,
        print_record: None,
    };
    let mut argv = std::env::args().skip(1);
    while let Some(arg) = argv.next() {
        let mut value = |flag: &str| -> Result<String, String> {
            argv.next()
                .ok_or_else(|| format!("{flag} requires a value"))
        };
        match arg.as_str() {
            "-h" | "--help" => {
                print!("{USAGE}");
                std::process::exit(0);
            }
            "--headless" => args.headless = true,
            "--measure" => args.measure = true,
            "--app" => args.app = AppKind::parse(&value("--app")?)?,
            "--app-dir" => args.app_dir = Some(PathBuf::from(value("--app-dir")?)),
            "--data-dir" => args.data_dir = Some(PathBuf::from(value("--data-dir")?)),
            "--size" => {
                let raw = value("--size")?;
                let (w, h) = raw
                    .split_once(['x', 'X'])
                    .ok_or_else(|| format!("size must look like 800x600, got {raw:?}"))?;
                let parse = |v: &str| {
                    v.parse::<u32>()
                        .map_err(|_| format!("invalid size {raw:?}"))
                };
                args.size = (parse(w)?, parse(h)?);
                if args.size.0 == 0 || args.size.1 == 0 {
                    return Err(format!("size must be non-zero, got {raw:?}"));
                }
            }
            "--scale" => {
                let raw = value("--scale")?;
                args.scale = raw.parse().map_err(|_| format!("invalid scale {raw:?}"))?;
                if !args.scale.is_finite() || args.scale <= 0.0 {
                    return Err(format!("scale must be > 0, got {raw:?}"));
                }
            }
            "--frames" => {
                let raw = value("--frames")?;
                args.frames = raw
                    .parse()
                    .map_err(|_| format!("invalid frame count {raw:?}"))?;
                if args.frames == 0 {
                    return Err("frame count must be >= 1".into());
                }
            }
            "--out" => args.out = Some(PathBuf::from(value("--out")?)),
            "--exit-after-ms" => {
                let raw = value("--exit-after-ms")?;
                let ms: u64 = raw
                    .parse()
                    .map_err(|_| format!("invalid duration {raw:?}"))?;
                args.exit_after = Some(Duration::from_millis(ms));
            }
            "--smoke-save" => args.smoke_save = Some(value("--smoke-save")?),
            "--name" => args.smoke_name = Some(value("--name")?),
            "--notes" => args.smoke_notes = Some(value("--notes")?),
            "--print-record" => args.print_record = Some(value("--print-record")?),
            other => return Err(format!("unknown option {other:?}")),
        }
    }
    Ok(args)
}

/// Where the app files were found.
enum AppSource {
    Directory(PathBuf),
    Embedded,
}

impl AppSource {
    fn describe(&self) -> String {
        match self {
            AppSource::Directory(dir) => format!("app dir: {}", dir.display()),
            AppSource::Embedded => "app: embedded build-time copy".to_owned(),
        }
    }
}

fn resolve_app(flag: Option<&Path>, app: AppKind) -> AppSource {
    if let Some(dir) = flag {
        return AppSource::Directory(dir.to_owned());
    }
    let name = app.dir_name();
    let mut candidates: Vec<PathBuf> = Vec::new();
    if let Ok(cwd) = std::env::current_dir() {
        candidates.push(cwd.join(name));
    }
    if let Ok(exe) = std::env::current_exe() {
        if let Some(exe_dir) = exe.parent() {
            candidates.push(exe_dir.join(name));
            if let Some(up) = exe_dir.parent() {
                candidates.push(up.join(name));
                if let Some(up2) = up.parent() {
                    candidates.push(up2.join(name));
                }
            }
        }
    }
    for candidate in candidates {
        if candidate.join("index.html").is_file() {
            return AppSource::Directory(candidate);
        }
    }
    AppSource::Embedded
}

/// The app's files, read once: the HTML document plus every `*.css` in
/// the directory (sorted by name — the same cascade order the dev host
/// uses). The ops app keeps the *template* HTML here; its loadable
/// document is derived per [`ops::build_document`].
struct AppFiles {
    /// Document/source name for diagnostics.
    name: String,
    /// The HTML (ops: the seed template).
    html: String,
    /// Stylesheets as `(id, css)` in cascade order.
    sheets: Vec<(String, String)>,
    /// Base directory for asset resolution, when files came from disk.
    base: Option<PathBuf>,
}

fn read_app(source: &AppSource, app: AppKind) -> Result<AppFiles, String> {
    match source {
        AppSource::Directory(dir) => {
            let index = dir.join("index.html");
            let html = std::fs::read_to_string(&index)
                .map_err(|e| format!("cannot read {}: {e}", index.display()))?;
            let mut css_files: Vec<PathBuf> = std::fs::read_dir(dir)
                .map_err(|e| format!("cannot read {}: {e}", dir.display()))?
                .filter_map(|entry| entry.ok().map(|e| e.path()))
                .filter(|path| path.extension().is_some_and(|ext| ext == "css"))
                .collect();
            css_files.sort();
            let mut sheets = Vec::new();
            for css_file in css_files {
                let css = std::fs::read_to_string(&css_file)
                    .map_err(|e| format!("cannot read {}: {e}", css_file.display()))?;
                sheets.push((
                    css_file
                        .file_name()
                        .map(|n| n.to_string_lossy().into_owned())
                        .unwrap_or_default(),
                    css,
                ));
            }
            Ok(AppFiles {
                name: index.to_string_lossy().into_owned(),
                html,
                sheets,
                base: Some(dir.clone()),
            })
        }
        AppSource::Embedded => {
            let (name, html, css) = match app {
                AppKind::Ops => ("ops/index.html", EMBEDDED_OPS_HTML, EMBEDDED_OPS_CSS),
                AppKind::Dashboard => (
                    "app/index.html",
                    EMBEDDED_DASHBOARD_HTML,
                    EMBEDDED_DASHBOARD_CSS,
                ),
            };
            Ok(AppFiles {
                name: name.to_owned(),
                html: html.to_owned(),
                sheets: vec![("app.css".to_owned(), css.to_owned())],
                base: None,
            })
        }
    }
}

/// The app-side asset resolver: maps renderer asset requests onto the
/// app directory, staying inside it (the renderer itself never touches
/// the disk).
struct DirAssets {
    root: PathBuf,
}

impl AssetResolver for DirAssets {
    fn resolve(&self, request: AssetRequest<'_>) -> Option<Asset> {
        let rel = request.path.trim_start_matches("./");
        if rel.starts_with('/') || rel.split(['/', '\\']).any(|segment| segment == "..") {
            return None;
        }
        let path = self.root.join(rel);
        let bytes = std::fs::read(&path).ok()?;
        Some(Asset {
            id: SourceId::new(rel),
            bytes,
        })
    }
}

/// Loads a document (with asset resolver over the app directory when
/// files came from disk) plus the stylesheets in cascade order.
fn load_into(view: &mut VelquView, files: &AppFiles, html: &str) -> Result<(), String> {
    let mut document = DocumentSource::new(files.name.clone(), html.to_owned());
    if let Some(base) = &files.base {
        document = document.with_base(base.to_string_lossy().into_owned());
        view.set_asset_resolver(Rc::new(DirAssets { root: base.clone() }));
    }
    view.load_document(document).map_err(|e| e.to_string())?;
    for (id, css) in &files.sheets {
        let sheet = StylesheetSource::new(id.clone(), css.clone());
        view.load_stylesheet(sheet).map_err(|e| e.to_string())?;
    }
    Ok(())
}

fn viewport_for(args: &Args) -> Result<Viewport, String> {
    let physical = |logical: u32| (logical as f32 * args.scale).round() as u32;
    Viewport::try_new(physical(args.size.0), physical(args.size.1), args.scale)
        .map_err(|e| e.to_string())
}

fn resolve_data_dir(flag: Option<&Path>) -> PathBuf {
    flag.map(|p| p.to_owned())
        .or_else(|| std::env::var_os("VELQU_VIEW_STARTER_DATA_DIR").map(PathBuf::from))
        .unwrap_or_else(ops::default_data_dir)
}

// ---------------------------------------------------------------------------
// Ops workflow host: template + store + session, observed through the
// public event queue
// ---------------------------------------------------------------------------

/// Host-side state for the operations workflow. One per process; the
/// window hook borrows it behind an `Rc` (single-threaded, like the
/// renderer).
struct OpsHost {
    files: AppFiles,
    store: RefCell<RecordStore>,
    session: RefCell<Session>,
    /// Record suffix → `[name, notes]` as last seen in ValueChanged
    /// events; unedited fields fall back to the store/template values.
    drafts: RefCell<HashMap<String, [String; 2]>>,
}

impl OpsHost {
    fn new(files: AppFiles, store: RecordStore) -> Self {
        Self {
            files,
            store: RefCell::new(store),
            session: RefCell::new(Session {
                selected: "gt-2401".to_owned(),
                search: String::new(),
            }),
            drafts: RefCell::new(HashMap::new()),
        }
    }

    fn store_note(&self) -> String {
        // Kept short on purpose: the rail is 256 px wide and a full
        // path clips. The absolute path is printed to the log instead.
        let store = self.store.borrow();
        format!("On-device records.json ({} saved)", store.len())
    }

    /// The document for process startup: template + persisted
    /// corrections, no session to preserve.
    fn startup_document(&self) -> Result<String, String> {
        ops::build_document(
            &self.files.html,
            &self.store.borrow(),
            &BuildOpts {
                session: None,
                store_note: self.store_note(),
                flash: None,
            },
        )
    }

    /// Field values the host would persist for `suffix`: the tracked
    /// drafts, else the store override, else the template seed.
    fn current_fields(&self, suffix: &str) -> (String, String) {
        if let Some(draft) = self.drafts.borrow().get(suffix) {
            return (draft[0].clone(), draft[1].clone());
        }
        let seed = |key: &str| {
            ops::parse_state_block(&self.files.html)
                .ok()
                .and_then(|pairs| {
                    pairs
                        .into_iter()
                        .find(|(k, _)| k == key)
                        .and_then(|(_, v)| v.as_str().map(str::to_owned))
                })
                .unwrap_or_default()
        };
        let mut name = seed(&format!("name_{suffix}"));
        let mut notes = seed(&format!("notes_{suffix}"));
        if let Some(entry) = self.store.borrow().get(suffix) {
            name = entry.driver_name.clone();
            notes = entry.notes.clone();
        }
        (name, notes)
    }

    /// Reloads the live document from `html`, transactionally. Returns
    /// the published generation on success.
    fn reload(&self, view: &mut VelquView, viewport: Viewport, html: &str) -> Result<u64, String> {
        let mut document = DocumentSource::new(self.files.name.clone(), html.to_owned());
        if let Some(base) = &self.files.base {
            document = document.with_base(base.to_string_lossy().into_owned());
        }
        let sheets = self
            .files
            .sheets
            .iter()
            .map(|(id, css)| StylesheetSource::new(id.clone(), css.clone()))
            .collect();
        view.reload_bundle(document, sheets, viewport)
            .map_err(|rejection| rejection.to_string())
    }

    /// The frame hook: track session + drafts from the public event
    /// batch, and when the operator clicked Save, run the authoritative
    /// host path (validate → persist → reload from persisted truth).
    fn on_events(&self, view: &mut VelquView, viewport: Viewport, batch: &[Event]) {
        let mut save_suffix: Option<String> = None;
        for event in batch {
            match event {
                Event::Click { target } => {
                    let Some(id) = target.id.as_deref() else {
                        continue;
                    };
                    let Some(suffix) = ops::record_suffix(id) else {
                        continue;
                    };
                    if id.starts_with("row-") {
                        self.session.borrow_mut().selected = format!("gt-{suffix}");
                    } else if id.starts_with("save-") {
                        save_suffix = Some(suffix.to_owned());
                    }
                }
                Event::ValueChanged { target, value } => {
                    let Some(id) = target.id.as_deref() else {
                        continue;
                    };
                    if id == "search" {
                        self.session.borrow_mut().search = value.clone();
                    } else if let Some(suffix) = ops::record_suffix(id) {
                        let (seed_name, seed_notes) = self.current_fields(suffix);
                        let mut drafts = self.drafts.borrow_mut();
                        let entry = drafts
                            .entry(suffix.to_owned())
                            .or_insert([seed_name, seed_notes]);
                        if id.starts_with("edit-name-") {
                            entry[0] = value.clone();
                        } else if id.starts_with("edit-notes-") {
                            entry[1] = value.clone();
                        }
                    }
                }
                _ => {}
            }
        }
        let Some(suffix) = save_suffix else { return };

        let (name, notes) = self.current_fields(&suffix);
        let session = self.session.borrow().clone();
        let store_note = self.store_note();
        // Hoisted out of the match on purpose: the `borrow_mut` in the
        // call is a match-scrutinee temporary that would otherwise
        // live through every arm while they borrow the store again.
        let outcome = ops::apply_save(
            &self.files.html,
            &mut self.store.borrow_mut(),
            &session,
            &suffix,
            &name,
            &notes,
            &store_note,
        );
        match outcome {
            SaveOutcome::Saved { html, saved_at } => match self.reload(view, viewport, &html) {
                Ok(generation) => {
                    println!(
                        "ops: saved gt-{suffix} at {saved_at} (generation {generation}, store: {})",
                        self.store.borrow().path().display()
                    );
                    self.drafts
                        .borrow_mut()
                        .insert(suffix.clone(), [name, notes]);
                    view.set_focus(Some(&format!("edit-name-gt-{suffix}")));
                }
                Err(rejection) => {
                    eprintln!(
                        "ops: saved gt-{suffix} to the store, but the reload was rejected: \
                         {rejection} — the change is durable; it appears after a restart."
                    );
                }
            },
            SaveOutcome::Rejected(rule) => {
                eprintln!("ops: save of gt-{suffix} rejected: {rule}");
                let message = format!("Validation failed — not saved. ({rule})");
                if let Ok(html) = ops::build_document(
                    &self.files.html,
                    &self.store.borrow(),
                    &BuildOpts {
                        session: Some(session),
                        store_note,
                        flash: Some(Flash::Error {
                            suffix: suffix.clone(),
                            name,
                            notes,
                            message,
                        }),
                    },
                ) {
                    if let Err(e) = self.reload(view, viewport, &html) {
                        eprintln!("ops: rejection reload failed: {e}");
                    }
                }
            }
            SaveOutcome::StoreError { html } => {
                eprintln!(
                    "ops: store write failed for gt-{suffix} — your edits are kept on screen, \
                     retry Save"
                );
                if let Err(e) = self.reload(view, viewport, &html) {
                    eprintln!("ops: error reload failed: {e}");
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Smoke modes (--smoke-save / --print-record): the same host paths,
// driven without a window, for process-level durability tests
// ---------------------------------------------------------------------------

fn record_suffix_of(id: &str) -> Result<String, String> {
    let suffix = id
        .trim_start_matches("GT-")
        .trim_start_matches("gt-")
        .trim();
    if !suffix.is_empty() && suffix.chars().all(|c| c.is_ascii_digit()) {
        Ok(suffix.to_owned())
    } else {
        Err(format!("record id must look like GT-2401, got {id:?}"))
    }
}

fn smoke_save(
    files: &AppFiles,
    data_dir: &Path,
    id: &str,
    name: &str,
    notes: &str,
) -> Result<(), String> {
    let suffix = record_suffix_of(id)?;
    let mut store = RecordStore::open(data_dir)?;
    let store_note = format!("On-device records.json ({} saved)", store.len());
    match ops::apply_save(
        &files.html,
        &mut store,
        &Session::default(),
        &suffix,
        name,
        notes,
        &store_note,
    ) {
        SaveOutcome::Saved { saved_at, .. } => {
            println!(
                "ops: saved gt-{suffix} at {saved_at} (store: {})",
                store.path().display()
            );
            Ok(())
        }
        SaveOutcome::Rejected(rule) => Err(format!("save rejected: {rule}")),
        SaveOutcome::StoreError { .. } => Err("store write failed".into()),
    }
}

fn print_record(files: &AppFiles, data_dir: &Path, id: &str) -> Result<(), String> {
    let suffix = record_suffix_of(id)?;
    let store = RecordStore::open(data_dir)?;
    let host = OpsHost::new(
        AppFiles {
            name: files.name.clone(),
            html: files.html.clone(),
            sheets: files.sheets.clone(),
            base: files.base.clone(),
        },
        store,
    );
    let (driver, notes, saved_at) = match host.store.borrow().get(&suffix) {
        Some(entry) => (
            entry.driver_name.clone(),
            entry.notes.clone(),
            entry.saved_at.clone(),
        ),
        None => {
            let (name, notes) = host.current_fields(&suffix);
            (name, notes, String::new())
        }
    };
    println!("record: gt-{suffix} | {driver} | {notes} | {saved_at}");
    Ok(())
}

// ---------------------------------------------------------------------------
// Headless / window
// ---------------------------------------------------------------------------

fn run_headless(
    args: &Args,
    view: &mut VelquView,
    source: &AppSource,
    store: Option<&RecordStore>,
) -> Result<(), String> {
    let viewport = viewport_for(args)?;
    let mut hashes: Vec<String> = Vec::new();
    let mut first: Option<velqu_view::FrameResult> = None;
    for frame in 0..args.frames {
        // One pump per frame, mirroring the redraw order.
        let batch = view.take_events();
        view.pump_reactive(&batch);
        drop(batch);
        let started = Instant::now();
        let result = view.render(viewport).map_err(|e| e.to_string())?;
        if frame == 0 && args.measure {
            println!("measure: first_render_ms={}", started.elapsed().as_millis());
        }
        hashes.push(result.frame.sha256_hex());
        first.get_or_insert(result);
    }
    let first = first.expect("frames >= 1");

    println!("{}", source.describe());
    println!(
        "viewport: {}x{} px @ {:.2}x (logical {}x{})",
        viewport.width(),
        viewport.height(),
        viewport.scale_factor(),
        args.size.0,
        args.size.1
    );
    println!(
        "frame 1: {} items, {} glyphs",
        first.stats.items, first.stats.glyphs
    );
    let diagnostics = view.tailwind_diagnostics();
    if diagnostics.is_empty() {
        println!("tailwind: all utility classes compiled");
    } else {
        for message in diagnostics {
            println!("tailwind diagnostic: {message}");
        }
    }
    if let Some(plan) = view.reactive_plan() {
        println!(
            "reactive: {} scope(s), {} binding(s), {} handler(s)",
            plan.scopes.len(),
            plan.bindings.len(),
            plan.events.len()
        );
        for message in view.reactive_diagnostics() {
            println!("reactive diagnostic: {message}");
        }
    }
    if let Some(store) = store {
        println!("store: {} ({} saved)", store.path().display(), store.len());
    }
    println!("sha256: {}", hashes[0]);
    if hashes.iter().any(|h| *h != hashes[0]) {
        return Err("determinism check FAILED: frames differ".into());
    }

    let out = match (&args.out, args.app) {
        (Some(path), _) => path.clone(),
        (None, AppKind::Ops) => PathBuf::from("ops").join("out").join("frame.png"),
        (None, AppKind::Dashboard) => PathBuf::from("app").join("out").join("frame.png"),
    };
    if let Some(parent) = out.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| format!("cannot create {}: {e}", parent.display()))?;
    }
    first
        .frame
        .save_png(&out)
        .map_err(|e| format!("cannot write {}: {e}", out.display()))?;
    println!("png: {}", out.display());
    Ok(())
}

fn run_window(
    args: &Args,
    launch: Instant,
    view: &mut VelquView,
    hook: Option<FrameEventsHook<'_>>,
) -> Result<(), String> {
    let title = match args.app {
        AppKind::Ops => "Gate Ops — velqu-view-starter",
        AppKind::Dashboard => "Gate Ops Dashboard — velqu-view-starter",
    };
    let mut config =
        HostWindowConfig::new(title).with_logical_size(args.size.0 as f32, args.size.1 as f32);
    if let Some(exit_after) = args.exit_after {
        config = config.with_exit_after(exit_after);
    }
    if args.measure {
        config = config.with_measure();
    }
    config.launch = launch;
    let stats = window::run_host_window(config, view, hook)?;
    if args.measure {
        println!(
            "measure: frames={} uptime_ms={}",
            stats.frames,
            stats.uptime.as_millis()
        );
    }
    println!(
        "window closed: {} frame(s) presented in {:.1?}",
        stats.frames, stats.uptime
    );
    Ok(())
}

fn main() -> ExitCode {
    let launch = Instant::now();
    let args = match parse_args() {
        Ok(args) => args,
        Err(message) => {
            eprintln!("velqu-view-starter: {message}\n\n{USAGE}");
            return ExitCode::from(2);
        }
    };

    let source = resolve_app(args.app_dir.as_deref(), args.app);
    let files = match read_app(&source, args.app) {
        Ok(files) => files,
        Err(message) => {
            eprintln!("velqu-view-starter: {message}");
            return ExitCode::from(2);
        }
    };

    // Ops smoke modes run the real host paths and exit, no window.
    if args.app == AppKind::Ops {
        let data_dir = resolve_data_dir(args.data_dir.as_deref());
        if let Some(id) = &args.print_record {
            return match print_record(&files, &data_dir, id) {
                Ok(()) => ExitCode::SUCCESS,
                Err(message) => {
                    eprintln!("velqu-view-starter: {message}");
                    ExitCode::from(2)
                }
            };
        }
        if let Some(id) = &args.smoke_save {
            let name = args.smoke_name.as_deref().unwrap_or("");
            let notes = args.smoke_notes.as_deref().unwrap_or("");
            return match smoke_save(&files, &data_dir, id, name, notes) {
                Ok(()) => ExitCode::SUCCESS,
                Err(message) => {
                    eprintln!("velqu-view-starter: {message}");
                    ExitCode::from(2)
                }
            };
        }
    }

    let mut view = VelquView::new();
    view.enable_tailwind();
    view.enable_reactive();

    let result = if args.app == AppKind::Ops {
        let data_dir = resolve_data_dir(args.data_dir.as_deref());
        let store = match RecordStore::open(&data_dir) {
            Ok(store) => store,
            Err(message) => {
                eprintln!("velqu-view-starter: {message}");
                return ExitCode::from(2);
            }
        };
        let host = Rc::new(OpsHost::new(files, store));
        let html = match host.startup_document() {
            Ok(html) => html,
            Err(message) => {
                eprintln!(
                    "velqu-view-starter: {message} — falling back to the raw template; \
                     saves are disabled this run"
                );
                host.files.html.clone()
            }
        };
        if let Err(message) = load_into(&mut view, &host.files, &html) {
            eprintln!("velqu-view-starter: {message}");
            return ExitCode::from(2);
        }
        if args.headless {
            let store = host.store.borrow().clone();
            run_headless(&args, &mut view, &source, Some(&store))
        } else {
            let hook_host = Rc::clone(&host);
            let hook: FrameEventsHook = Box::new(move |view, viewport, batch| {
                hook_host.on_events(view, viewport, batch);
            });
            run_window(&args, launch, &mut view, Some(hook))
        }
    } else {
        if let Err(message) = load_into(&mut view, &files, &files.html.clone()) {
            eprintln!("velqu-view-starter: {message}");
            return ExitCode::from(2);
        }
        if args.headless {
            run_headless(&args, &mut view, &source, None)
        } else {
            run_window(&args, launch, &mut view, None)
        }
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("velqu-view-starter: {message}");
            ExitCode::FAILURE
        }
    }
}
