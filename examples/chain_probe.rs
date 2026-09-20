//! Second diagnostic probe (see docs/verification.md): replicates
//! velqu-lab's exact watch composition against the public shell API,
//! with a trace line at every hop. Wherever the lab's chain dies, this
//! names the hop.
//!
//! Hop 0 — watch registered on the app directory.
//! Hop 1 — notify callback fires (raw event on the watched dir).
//! Hop 2 — watch thread forwards the event and invokes the wake
//!         closure.
//! Hop 3 — wake closure sends ShellUserEvent::Watch through the proxy.
//! Hop 4 — the shell runs the hook on the main thread.
//!
//! Edit a file under ./app while this runs and read the numbers.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use velqu_view::VelquView;

use notify::Watcher;

fn t() -> f32 {
    START.elapsed().as_secs_f32()
}

static START: std::sync::LazyLock<Instant> = std::sync::LazyLock::new(Instant::now);

fn main() {
    let watch_dir = std::env::args()
        .nth(1)
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("app"));

    let mut view = VelquView::new();
    view.enable_tailwind();
    view.enable_reactive();
    let html = std::fs::read_to_string(watch_dir.join("index.html")).expect("index.html");
    view.load_html(&html).expect("html loads");
    let css = std::fs::read_to_string(watch_dir.join("app.css")).expect("app.css");
    view.load_stylesheet(velqu_view::StylesheetSource::new("app.css", css))
        .expect("css loads");

    let dir_for_thread = watch_dir.clone();
    let spawn_watcher = move |proxy: velqu_shell::ShellProxy| {
        let (events_tx, events_rx) = std::sync::mpsc::channel::<notify::Result<notify::Event>>();
        let mut watcher = notify::recommended_watcher(move |event| {
            println!("[{:.2}s] hop1 notify event {}", t(), event_ok(&event));
            let _ = events_tx.send(event);
        })
        .expect("watcher");
        let wake = Arc::new(move || {
            println!("[{:.2}s] hop2 wake closure", t());
            let _ = proxy.send_event(velqu_shell::ShellUserEvent::Watch);
            println!("[{:.2}s] hop3 proxy send", t());
        });
        let wake_for_thread = Arc::clone(&wake);
        std::thread::spawn(move || {
            let _ = watcher.watch(&dir_for_thread, notify::RecursiveMode::Recursive);
            println!(
                "[{:.2}s] hop0 watch registered on {}",
                t(),
                dir_for_thread.display()
            );
            // The lab sends one Rescan here at startup; mirror it.
            wake_for_thread();
            while let Ok(event) = events_rx.recv() {
                println!("[{:.2}s] hop2 thread forwarding {}", t(), event_ok(&event));
                wake_for_thread();
            }
        });
    };

    let hook: velqu_shell::WatchHook = Box::new(move |_view: &mut VelquView| {
        println!("[{:.2}s] hop4 HOOK RAN", t());
        false
    });

    let config = velqu_shell::ShellConfig::new("velqu-view-starter chain probe")
        .with_logical_size(1280.0, 800.0)
        .with_exit_after(Duration::from_secs(14));
    let stats =
        velqu_shell::run_with_watch(config, &mut view, spawn_watcher, hook).expect("shell run");
    println!(
        "window closed: {} frame(s) in {:.1?}",
        stats.frames, stats.uptime
    );
}

fn event_ok(event: &notify::Result<notify::Event>) -> String {
    match event {
        Ok(event) => format!("{:?} {:?}", event.kind, event.paths),
        Err(error) => format!("error {error}"),
    }
}
