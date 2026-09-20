//! Diagnostic probe for the dev-command finding (see
//! docs/verification.md): does a `ShellUserEvent::Watch` sent from a
//! non-main thread wake a waiting X11 event loop in `velqu-shell` at
//! the pinned revision?
//!
//! A thread sends Watch at t = 3s, 6s, and 9s; the watch hook prints
//! when it actually runs. If the proxy wake works, a "hook ran" line
//! follows each "sent Watch" within milliseconds. If the loop only
//! wakes at the exit deadline, every send is silent until one final
//! hook run at exit — the watch chain cannot reconcile while idle.
//!
//! Not part of the application surface; this exists to make the
//! finding reproducible with the public API alone.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use velqu_view::VelquView;

fn main() {
    let mut view = VelquView::new();
    view.enable_tailwind();
    view.enable_reactive();
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("app");
    let html = std::fs::read_to_string(dir.join("index.html")).expect("index.html");
    view.load_html(&html).expect("html loads");
    let css = std::fs::read_to_string(dir.join("app.css")).expect("app.css");
    view.load_stylesheet(velqu_view::StylesheetSource::new("app.css", css))
        .expect("css loads");

    let start = Instant::now();
    let stamp = move || start.elapsed().as_secs_f32();

    let spawn_watcher = move |proxy: velqu_shell::ShellProxy| {
        let sender = Arc::new(move || {
            let _ = proxy.send_event(velqu_shell::ShellUserEvent::Watch);
        });
        std::thread::spawn(move || {
            for at in [3.0f32, 6.0, 9.0] {
                let now = sender_time();
                if at > now {
                    std::thread::sleep(Duration::from_secs_f32(at - now));
                }
                println!("[{:.2}s] sent Watch", sender_time());
                sender();
            }
        });
    };

    let hook: velqu_shell::WatchHook = Box::new(move |_view: &mut VelquView| {
        println!("[{:.2}s] hook ran", stamp());
        false
    });

    let config = velqu_shell::ShellConfig::new("velqu-view-starter watch probe")
        .with_logical_size(1280.0, 800.0)
        .with_exit_after(Duration::from_secs(12));
    let stats =
        velqu_shell::run_with_watch(config, &mut view, spawn_watcher, hook).expect("shell run");
    println!(
        "window closed: {} frame(s) in {:.1?}",
        stats.frames, stats.uptime
    );
}

// The sender thread's own clock (kept off the closure's captures).
fn sender_time() -> f32 {
    START.elapsed().as_secs_f32()
}

static START: std::sync::LazyLock<Instant> = std::sync::LazyLock::new(Instant::now);
