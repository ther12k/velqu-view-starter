//! External-consumer verification: the dashboard driven end-to-end
//! through VelquView's public API from **this** application — the same
//! git-pinned crate any outside developer would depend on.
//!
//! The initial-frame digest is the accepted M7 baseline
//! (`docs/reference-dashboard.md` in the VelquView repository at the
//! pinned revision). Matching it here proves the git-dependency
//! pipeline renders pixel-for-pixel like the framework workspace build.

use std::path::PathBuf;

use velqu_view::{Event, KeyCommand, KeyModifiers, VelquView, Viewport};

/// Reviewed-then-frozen M7 initial raster digest at 1280x800 @ 1.0x.
const M7_INITIAL_DIGEST: &str = "a89813c53c651e717abfd7926bcac5974149440005055bf1f45f93cd747a8c5c";

const VP: (u32, u32) = (1280, 800);

fn app_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("app")
}

/// Loads the app exactly as the starter binary and velqu-lab do:
/// index.html plus every `*.css` sorted by name (cascade order),
/// Tailwind + reactive enabled.
fn load_app() -> VelquView {
    let html = std::fs::read_to_string(app_dir().join("index.html")).expect("index.html");
    let mut css_files: Vec<PathBuf> = std::fs::read_dir(app_dir())
        .expect("app dir")
        .filter_map(|entry| entry.ok().map(|e| e.path()))
        .filter(|path| path.extension().is_some_and(|ext| ext == "css"))
        .collect();
    css_files.sort();
    let mut view = VelquView::new();
    view.enable_tailwind();
    view.enable_reactive();
    view.load_html(&html).expect("app html loads");
    for css in css_files {
        let sheet = std::fs::read_to_string(&css).expect("css");
        let id = css.file_name().unwrap().to_string_lossy().into_owned();
        view.load_stylesheet(velqu_view::StylesheetSource::new(id, sheet))
            .expect("sheet loads");
    }
    view
}

fn vp() -> Viewport {
    Viewport::try_new(VP.0, VP.1, 1.0).unwrap()
}

fn settle(view: &mut VelquView, viewport: Viewport) {
    view.render(viewport).unwrap();
    let batch = view.take_events();
    view.pump_reactive(&batch);
    view.render(viewport).unwrap();
    let _ = view.take_events();
}

/// A viewport point over an element by HTML id (hit-test scan, no fixed
/// coordinates).
fn point_over(view: &VelquView, viewport: Viewport, id: &str) -> (f32, f32) {
    for y in (0..viewport.height()).step_by(4) {
        for x in (0..viewport.width()).step_by(8) {
            if view
                .hit_test(viewport, x as f32 + 0.5, y as f32 + 0.5)
                .is_some_and(|target| target.element_id.as_deref() == Some(id))
            {
                return (x as f32 + 0.5, y as f32 + 0.5);
            }
        }
    }
    panic!("no hit target for {id}");
}

fn click_id(view: &mut VelquView, viewport: Viewport, id: &str) {
    let (x, y) = point_over(view, viewport, id);
    view.pointer_press(viewport, x, y);
    view.pointer_release(viewport, x, y);
    let batch = view.take_events();
    view.pump_reactive(&batch);
    view.render(viewport).unwrap();
}

/// Replaces an input's text through the public keyboard path
/// (focus → End → backspace-clear → insert), returning every event
/// pumped while typing so tests can assert on them.
fn type_into(view: &mut VelquView, viewport: Viewport, id: &str, text: &str) -> Vec<Event> {
    view.set_focus(Some(id));
    let _ = view.take_events();
    view.key_command(KeyCommand::End, KeyModifiers::default());
    let _ = view.take_events();
    let handle = {
        let (x, y) = point_over(view, viewport, id);
        view.hit_test(viewport, x, y).unwrap().handle
    };
    let mut seen = Vec::new();
    let mut guard = 0;
    while view
        .control_value(handle)
        .is_some_and(|value| !value.is_empty())
    {
        guard += 1;
        assert!(guard < 256, "clearing {id} made no progress");
        view.key_command(KeyCommand::Backspace, KeyModifiers::default());
        let batch = view.take_events();
        view.pump_reactive(&batch);
        seen.extend(batch.iter().cloned());
        view.render(viewport).unwrap();
    }
    if !text.is_empty() {
        view.insert_text(text);
        let batch = view.take_events();
        view.pump_reactive(&batch);
        seen.extend(batch.iter().cloned());
        view.render(viewport).unwrap();
    }
    seen
}

fn text_of(view: &mut VelquView, viewport: Viewport, fixture: &str) -> String {
    view.layout_facts(viewport)
        .unwrap()
        .nodes
        .into_iter()
        .find(|fact| fact.fixture_id == fixture)
        .unwrap_or_else(|| panic!("no fact {fixture}"))
        .text_runs
        .join("")
}

/// Like [`point_over`] but returns `None` instead of panicking —
/// hidden elements produce no boxes, so a miss IS the hidden check.
fn hit_id(view: &VelquView, viewport: Viewport, id: &str) -> Option<(f32, f32)> {
    (0..viewport.height())
        .step_by(4)
        .flat_map(|y| (0..viewport.width()).step_by(8).map(move |x| (x, y)))
        .find_map(|(x, y)| {
            view.hit_test(viewport, x as f32 + 0.5, y as f32 + 0.5)
                .is_some_and(|target| target.element_id.as_deref() == Some(id))
                .then_some((x as f32 + 0.5, y as f32 + 0.5))
        })
}

/// A vx-show-hidden element produces no layout fact at all, so
/// presence in the facts IS visibility.
fn shown(view: &mut VelquView, viewport: Viewport, fixture: &str) -> bool {
    view.layout_facts(viewport)
        .unwrap()
        .nodes
        .iter()
        .any(|fact| fact.fixture_id == fixture)
}

#[test]
fn initial_frame_matches_the_accepted_m7_baseline() {
    let viewport = vp();
    let mut view = load_app();
    settle(&mut view, viewport);
    let first = view.render(viewport).unwrap().frame.sha256_hex();
    let second = view.render(viewport).unwrap().frame.sha256_hex();
    assert_eq!(first, second, "render must be deterministic");
    assert_eq!(
        first, M7_INITIAL_DIGEST,
        "the external git-dependency pipeline must reproduce the accepted \
         M7 baseline pixel-for-pixel",
    );
}

#[test]
fn search_filters_rows_and_the_summary_together() {
    let viewport = vp();
    let mut view = load_app();
    settle(&mut view, viewport);

    type_into(&mut view, viewport, "search", "gate b");
    assert_eq!(text_of(&mut view, viewport, "summary-shown"), "2");
    assert!(shown(&mut view, viewport, "row-2210"));
    assert!(shown(&mut view, viewport, "row-7742"));
    assert!(!shown(&mut view, viewport, "row-8841"));
    assert!(!shown(&mut view, viewport, "no-results"));

    type_into(&mut view, viewport, "search", "");
    assert_eq!(text_of(&mut view, viewport, "summary-shown"), "6");
    assert!(shown(&mut view, viewport, "row-8841"));

    type_into(&mut view, viewport, "search", "zzz");
    assert_eq!(text_of(&mut view, viewport, "summary-shown"), "0");
    assert!(shown(&mut view, viewport, "no-results"));
}

#[test]
fn navigation_switches_sections() {
    let viewport = vp();
    let mut view = load_app();
    settle(&mut view, viewport);
    assert!(shown(&mut view, viewport, "section-shipments"));

    click_id(&mut view, viewport, "nav-gates");
    assert!(!shown(&mut view, viewport, "section-shipments"));
    assert!(shown(&mut view, viewport, "section-gates"));

    click_id(&mut view, viewport, "nav-shipments");
    assert!(shown(&mut view, viewport, "section-shipments"));
}

#[test]
fn selecting_a_row_switches_the_detail_panel() {
    let viewport = vp();
    let mut view = load_app();
    settle(&mut view, viewport);
    assert!(shown(&mut view, viewport, "detail-8841"));

    click_id(&mut view, viewport, "row-5519");
    assert!(!shown(&mut view, viewport, "detail-8841"));
    assert!(shown(&mut view, viewport, "detail-5519"));
    // And it is the 5519 card: its edit input is hit-testable while the
    // deselected card's is not (hidden elements produce no boxes).
    assert!(
        hit_id(&view, viewport, "edit-name-5519").is_some(),
        "the selected record's edit input must be interactive"
    );
    assert!(
        hit_id(&view, viewport, "edit-name-8841").is_none(),
        "the deselected record's edit input must not be"
    );
}

#[test]
fn editing_marks_unsaved_and_save_clears_it() {
    let viewport = vp();
    let mut view = load_app();
    settle(&mut view, viewport);
    assert_eq!(text_of(&mut view, viewport, "summary-unsaved"), "0");

    let events = type_into(&mut view, viewport, "edit-name-8841", "Budi Santoso Jr.");
    assert!(
        events.iter().any(|event| matches!(
            event,
            Event::ValueChanged { target, .. }
                if target.id.as_deref() == Some("edit-name-8841")
        )),
        "typing must emit ValueChanged through the public event queue"
    );
    assert_eq!(text_of(&mut view, viewport, "summary-unsaved"), "1");
    assert!(!shown(&mut view, viewport, "saved-flag"));

    click_id(&mut view, viewport, "save-8841");
    assert_eq!(text_of(&mut view, viewport, "summary-unsaved"), "0");
    assert!(shown(&mut view, viewport, "saved-flag"));
}

#[test]
fn wheel_scrolls_the_records_list() {
    let viewport = vp();
    let mut view = load_app();
    settle(&mut view, viewport);

    let (row_x, row_y) = point_over(&view, viewport, "row-2210");
    view.wheel(viewport, row_x, row_y, 0.0, 80.0);
    let batch = view.take_events();
    assert!(
        batch
            .iter()
            .any(|event| matches!(event, Event::Scrolled { y, .. } if *y > 0.0)),
        "wheel over the records list must scroll it: {batch:?}"
    );
    view.pump_reactive(&batch);
    view.render(viewport).unwrap();
}
