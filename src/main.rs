//! velqu-view-starter — the M7 reference dashboard as a standalone,
//! external-consumer application built only on VelquView's public
//! crates.
//!
//! This binary is the **release host**: it loads the app directory with
//! Tailwind and Velqu Reactive enabled, opens a native window, and does
//! nothing else — no development watching, no inspector capture.
//! Development (hot reload, inspection, headless capture) goes through
//! `velqu-lab`, the framework's dev host at the same pinned revision;
//! see `dev.sh` and `README.md`.
//!
//! The app files (`app/index.html` + `app/*.css`) are located in this
//! order, so launching never depends on the current directory:
//!
//! 1. `--app-dir DIR` when given;
//! 2. `./app` relative to the current directory;
//! 3. `app/` next to the executable, one and two levels up
//!    (covers `target/release/` and `target/` launch layouts);
//! 4. the copies embedded into the binary at build time (last resort —
//!    editing files on disk is preferred whenever they are found).

use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::rc::Rc;
use std::time::Duration;

use velqu_view::{
    Asset, AssetRequest, AssetResolver, DocumentSource, SourceId, StylesheetSource, VelquView,
    Viewport,
};

/// Build-time copies of the app, used only when no `app/` directory is
/// found on disk.
const EMBEDDED_HTML: &str = include_str!("../app/index.html");
const EMBEDDED_CSS: &str = include_str!("../app/app.css");

struct Args {
    app_dir: Option<PathBuf>,
    headless: bool,
    size: (u32, u32),
    scale: f32,
    out: Option<PathBuf>,
    frames: u32,
    exit_after: Option<Duration>,
}

const USAGE: &str = "\
velqu-view-starter — reference dashboard as a standalone app

USAGE:
    velqu-view-starter [OPTIONS]

OPTIONS:
    --app-dir DIR      App directory containing index.html (default:
                       searched: ./app, beside the executable, then the
                       embedded build-time copy)
    --headless         Render offscreen and exit; no window
    --out PATH         Headless: PNG output path (default app/out/frame.png)
    --frames N         Headless: render N frames, verify determinism (default 1)
    --size WxH         Logical window size (default 1280x800, the reviewed
                       dashboard viewport)
    --scale F          DPI scale factor (default 1.0)
    --exit-after-ms N  Window: auto-close after N milliseconds (smoke tests)
    -h, --help         Print this help

Development (Tailwind + reactive + hot reload + inspection) is not in
this binary; run ./dev.sh, which uses velqu-lab from the same pinned
VelquView revision.
";

fn parse_args() -> Result<Args, String> {
    let mut args = Args {
        app_dir: None,
        headless: false,
        size: (1280, 800),
        scale: 1.0,
        out: None,
        frames: 1,
        exit_after: None,
    };
    let mut argv = std::env::args().skip(1);
    while let Some(arg) = argv.next() {
        match arg.as_str() {
            "-h" | "--help" => {
                print!("{USAGE}");
                std::process::exit(0);
            }
            "--headless" => args.headless = true,
            "--app-dir" => {
                let value = argv.next().ok_or("--app-dir requires a path")?;
                args.app_dir = Some(PathBuf::from(value));
            }
            "--size" => {
                let value = argv.next().ok_or("--size requires WxH")?;
                let (w, h) = value
                    .split_once(['x', 'X'])
                    .ok_or_else(|| format!("size must look like 800x600, got {value:?}"))?;
                let parse = |v: &str| {
                    v.parse::<u32>()
                        .map_err(|_| format!("invalid size {value:?}"))
                };
                args.size = (parse(w)?, parse(h)?);
                if args.size.0 == 0 || args.size.1 == 0 {
                    return Err(format!("size must be non-zero, got {value:?}"));
                }
            }
            "--scale" => {
                let value = argv.next().ok_or("--scale requires a number")?;
                args.scale = value
                    .parse()
                    .map_err(|_| format!("invalid scale {value:?}"))?;
                if !args.scale.is_finite() || args.scale <= 0.0 {
                    return Err(format!("scale must be > 0, got {value:?}"));
                }
            }
            "--frames" => {
                let value = argv.next().ok_or("--frames requires a count")?;
                args.frames = value
                    .parse()
                    .map_err(|_| format!("invalid frame count {value:?}"))?;
                if args.frames == 0 {
                    return Err("frame count must be >= 1".into());
                }
            }
            "--out" => {
                let value = argv.next().ok_or("--out requires a path")?;
                args.out = Some(PathBuf::from(value));
            }
            "--exit-after-ms" => {
                let value = argv.next().ok_or("--exit-after-ms requires a count")?;
                let ms: u64 = value
                    .parse()
                    .map_err(|_| format!("invalid duration {value:?}"))?;
                args.exit_after = Some(Duration::from_millis(ms));
            }
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

fn resolve_app(flag: Option<&Path>) -> AppSource {
    if let Some(dir) = flag {
        return AppSource::Directory(dir.to_owned());
    }
    let mut candidates: Vec<PathBuf> = Vec::new();
    if let Ok(cwd) = std::env::current_dir() {
        candidates.push(cwd.join("app"));
    }
    if let Ok(exe) = std::env::current_exe() {
        if let Some(exe_dir) = exe.parent() {
            candidates.push(exe_dir.join("app"));
            if let Some(up) = exe_dir.parent() {
                candidates.push(up.join("app"));
                if let Some(up2) = up.parent() {
                    candidates.push(up2.join("app"));
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

/// Loads `index.html` plus every `*.css` in the app directory (sorted by
/// name, the cascade order the dev host uses too), with a host asset
/// resolver installed over the directory.
fn load_app(view: &mut VelquView, source: &AppSource) -> Result<(), String> {
    match source {
        AppSource::Directory(dir) => {
            let index = dir.join("index.html");
            let html = std::fs::read_to_string(&index)
                .map_err(|e| format!("cannot read {}: {e}", index.display()))?;
            let document = DocumentSource::new(index.to_string_lossy().into_owned(), html)
                .with_base(dir.to_string_lossy().into_owned());
            view.load_document(document).map_err(|e| e.to_string())?;
            view.set_asset_resolver(Rc::new(DirAssets { root: dir.clone() }));
            let mut css_files: Vec<PathBuf> = std::fs::read_dir(dir)
                .map_err(|e| format!("cannot read {}: {e}", dir.display()))?
                .filter_map(|entry| entry.ok().map(|e| e.path()))
                .filter(|path| path.extension().is_some_and(|ext| ext == "css"))
                .collect();
            css_files.sort();
            for css_file in css_files {
                let css = std::fs::read_to_string(&css_file)
                    .map_err(|e| format!("cannot read {}: {e}", css_file.display()))?;
                let sheet = StylesheetSource::new(css_file.to_string_lossy().into_owned(), css);
                view.load_stylesheet(sheet).map_err(|e| e.to_string())?;
            }
        }
        AppSource::Embedded => {
            let document = DocumentSource::new("app/index.html".to_owned(), EMBEDDED_HTML);
            view.load_document(document).map_err(|e| e.to_string())?;
            let sheet = StylesheetSource::new("app.css".to_owned(), EMBEDDED_CSS);
            view.load_stylesheet(sheet).map_err(|e| e.to_string())?;
        }
    }
    Ok(())
}

fn viewport_for(args: &Args) -> Result<Viewport, String> {
    let physical = |logical: u32| (logical as f32 * args.scale).round() as u32;
    Viewport::try_new(physical(args.size.0), physical(args.size.1), args.scale)
        .map_err(|e| e.to_string())
}

fn run_headless(args: &Args, view: &mut VelquView, source: &AppSource) -> Result<(), String> {
    let viewport = viewport_for(args)?;
    let mut hashes: Vec<String> = Vec::new();
    let mut first: Option<velqu_view::FrameResult> = None;
    for _ in 0..args.frames {
        // One pump per frame, mirroring the shell's redraw order.
        let batch = view.take_events();
        view.pump_reactive(&batch);
        drop(batch);
        let result = view.render(viewport).map_err(|e| e.to_string())?;
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
    println!("sha256: {}", hashes[0]);
    if hashes.iter().any(|h| *h != hashes[0]) {
        return Err("determinism check FAILED: frames differ".into());
    }

    let out = args
        .out
        .clone()
        .unwrap_or_else(|| PathBuf::from("app").join("out").join("frame.png"));
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

fn run_window(args: &Args, view: &mut VelquView) -> Result<(), String> {
    let mut config = velqu_shell::ShellConfig::new("Gate Ops — velqu-view-starter")
        .with_logical_size(args.size.0 as f32, args.size.1 as f32);
    if let Some(exit_after) = args.exit_after {
        config = config.with_exit_after(exit_after);
    }
    let stats = velqu_shell::run(config, view).map_err(|e| e.to_string())?;
    println!(
        "window closed: {} frame(s) presented in {:.1?}",
        stats.frames, stats.uptime
    );
    Ok(())
}

fn main() -> ExitCode {
    let args = match parse_args() {
        Ok(args) => args,
        Err(message) => {
            eprintln!("velqu-view-starter: {message}\n\n{USAGE}");
            return ExitCode::from(2);
        }
    };

    let source = resolve_app(args.app_dir.as_deref());
    let mut view = VelquView::new();
    view.enable_tailwind();
    view.enable_reactive();
    if let Err(message) = load_app(&mut view, &source) {
        eprintln!("velqu-view-starter: {message}");
        return ExitCode::from(2);
    }

    let result = if args.headless {
        run_headless(&args, &mut view, &source)
    } else {
        run_window(&args, &mut view)
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("velqu-view-starter: {message}");
            ExitCode::FAILURE
        }
    }
}
