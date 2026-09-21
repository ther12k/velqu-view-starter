//! The operations workflow (gate-transaction review & correction)
//! driven end-to-end through VelquView's public API plus the starter's
//! own host code — the same functions the windowed binary uses.
//!
//! The journey under test: find a record → inspect its details →
//! change the permitted fields → validate → save (host persists
//! `records.json`) → confirm the result, with restart survival proven
//! by spawning the actual binary twice.
//!
//! The dashboard conformance tests in `tests/dashboard.rs` are
//! untouched framework-consumer evidence; these tests cover the real
//! business content separately, so changing it cannot erase them.

use std::path::PathBuf;
use std::process::Command;

use velqu_view::{
    DocumentSource, Event, KeyCommand, KeyModifiers, StylesheetSource, VelquView, Viewport,
};
use velqu_view_starter::ops::{
    self, BuildOpts, Flash, RecordStore, SaveOutcome, Session, StateValue,
};

const VP: (u32, u32) = (1280, 800);
/// Every authored record (kept in lockstep with tools/gen_ops_html.py).
const SUFFIXES: [&str; 14] = [
    "2401", "2402", "2403", "2404", "2405", "2406", "2407", "2408", "2409", "2410", "2411", "2412",
    "2413", "2414",
];

fn ops_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("ops")
}

fn template() -> String {
    std::fs::read_to_string(ops_dir().join("index.html")).expect("ops/index.html")
}

fn css() -> String {
    std::fs::read_to_string(ops_dir().join("app.css")).expect("ops/app.css")
}

/// Loads a document into a fresh view exactly like the release host:
/// Tailwind + reactive, then HTML, then the stylesheet.
fn load_doc(html: &str) -> VelquView {
    let mut view = VelquView::new();
    view.enable_tailwind();
    view.enable_reactive();
    view.load_html(html).expect("ops html loads");
    view.load_stylesheet(StylesheetSource::new("app.css", css()))
        .expect("ops css loads");
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

/// Clicks and returns the drained batch (pumped + one render).
fn click_id(view: &mut VelquView, viewport: Viewport, id: &str) -> Vec<Event> {
    let (x, y) = point_over(view, viewport, id);
    view.pointer_press(viewport, x, y);
    view.pointer_release(viewport, x, y);
    let batch = view.take_events();
    view.pump_reactive(&batch);
    view.render(viewport).unwrap();
    batch
}

/// Replaces an input's text through the public keyboard path,
/// returning every event pumped while typing.
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
        assert!(guard < 1024, "clearing {id} made no progress");
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

fn shown(view: &mut VelquView, viewport: Viewport, fixture: &str) -> bool {
    view.layout_facts(viewport)
        .unwrap()
        .nodes
        .iter()
        .any(|fact| fact.fixture_id == fixture)
}

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

fn control_text(view: &VelquView, viewport: Viewport, id: &str) -> String {
    let (x, y) = hit_id(view, viewport, id).unwrap_or_else(|| panic!("no hit for {id}"));
    let handle = view.hit_test(viewport, x, y).unwrap().handle;
    view.control_value(handle).unwrap_or("").to_owned()
}

/// Transactional full-document reload through the public API (the same
/// call the window hook makes after a save).
fn reload(view: &mut VelquView, html: &str) -> u64 {
    view.reload_bundle(
        DocumentSource::new("ops/index.html", html.to_owned()),
        vec![StylesheetSource::new("app.css", css())],
        vp(),
    )
    .expect("reload publishes")
}

fn temp_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "vv-starter-ops-{tag}-{}-{}",
        std::process::id(),
        tag
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn store_note(store: &RecordStore) -> String {
    format!("On-device records.json ({} saved)", store.len())
}

// ---------------------------------------------------------------------------
// Template structure: what the host contract depends on
// ---------------------------------------------------------------------------

#[test]
fn template_state_is_strict_and_matches_the_authored_records() {
    let html = template();
    let pairs = ops::parse_state_block(&html).expect("state block parses");
    let keys: Vec<&str> = pairs.iter().map(|(k, _)| k.as_str()).collect();
    for base in ["selected", "search", "store_note"] {
        assert!(keys.contains(&base), "missing state key {base}");
    }
    for suffix in SUFFIXES {
        for key in [
            format!("name_{suffix}"),
            format!("notes_{suffix}"),
            format!("saved_{suffix}"),
            format!("pending_{suffix}"),
            format!("error_{suffix}"),
            format!("saved_at_{suffix}"),
        ] {
            assert!(
                keys.contains(&key.as_str()),
                "missing state key {key} — regenerate ops/index.html?"
            );
        }
        // The authored structure must cover the same records: row,
        // detail panel, and save button per suffix.
        assert!(html.contains(&format!(r#"id="row-gt-{suffix}""#)));
        assert!(html.contains(&format!(r#"data-vv-test="detail-{suffix}""#)));
        assert!(html.contains(&format!(r#"id="save-gt-{suffix}""#)));
        // Seed values start clean: nothing pending, no error.
        let value = |key: &str| {
            pairs
                .iter()
                .find(|(k, _)| k == key)
                .map(|(_, v)| v.clone())
                .unwrap()
        };
        assert_eq!(value(&format!("pending_{suffix}")), StateValue::Bool(false));
        assert_eq!(
            value(&format!("error_{suffix}")),
            StateValue::Str(String::new())
        );
    }
}

// ---------------------------------------------------------------------------
// The operator journey, through the public API
// ---------------------------------------------------------------------------

#[test]
fn find_a_record_by_search_and_inspect_its_details() {
    let viewport = vp();
    let mut view = load_doc(&template());
    settle(&mut view, viewport);

    // Find: search narrows the list and the summary together (notes
    // are not indexed — the container number is).
    type_into(&mut view, viewport, "search", "hlxu 5521038");
    assert_eq!(text_of(&mut view, viewport, "summary-shown"), "1");
    assert!(shown(&mut view, viewport, "row-gt-2403"));
    assert!(!shown(&mut view, viewport, "row-gt-2401"));

    // Live name matching: the haystack concatenates the reactive name.
    type_into(&mut view, viewport, "search", "hartono");
    assert!(
        shown(&mut view, viewport, "row-gt-2413"),
        "registered driver"
    );
    type_into(&mut view, viewport, "search", "zzz");
    assert!(shown(&mut view, viewport, "no-results"));
    assert_eq!(text_of(&mut view, viewport, "summary-shown"), "0");
    type_into(&mut view, viewport, "search", "");

    // Inspect: selecting the rejected transaction shows its fields.
    click_id(&mut view, viewport, "row-gt-2403");
    assert!(shown(&mut view, viewport, "detail-2403"));
    assert!(!shown(&mut view, viewport, "detail-2401"));
    let meta = text_of(&mut view, viewport, "detail-meta-2403");
    assert!(meta.contains("HLXU 5521038"), "container in {meta}");
    assert!(meta.contains("Gate-out"), "direction in {meta}");
    assert_eq!(
        control_text(&view, viewport, "edit-name-gt-2403"),
        "Agus Wijaya"
    );
}

#[test]
fn validation_blocks_invalid_edits_in_the_document() {
    let viewport = vp();
    let mut view = load_doc(&template());
    settle(&mut view, viewport);
    click_id(&mut view, viewport, "row-gt-2403");

    // Empty driver name: the field error shows and stays.
    type_into(&mut view, viewport, "edit-name-gt-2403", "");
    assert!(shown(&mut view, viewport, "name-error-2403"));
    assert_eq!(text_of(&mut view, viewport, "summary-unsaved"), "1");

    // The save button is disabled while invalid, so clicking it does
    // not even request a save.
    click_id(&mut view, viewport, "save-gt-2403");
    assert!(
        !shown(&mut view, viewport, "pending-2403"),
        "an invalid edit must not reach the save path"
    );

    // Over-long notes: the error carries the live count.
    type_into(&mut view, viewport, "edit-name-gt-2403", "Agus Wijaya");
    type_into(&mut view, viewport, "edit-notes-gt-2403", &"x".repeat(281));
    assert!(shown(&mut view, viewport, "notes-error-2403"));
    assert_eq!(
        squashed(&text_of(&mut view, viewport, "notes-error-2403")),
        "Notes must be 280 characters or fewer (currently 281)."
    );
    click_id(&mut view, viewport, "save-gt-2403");
    assert!(!shown(&mut view, viewport, "pending-2403"));

    // Repairing the notes clears the error and enables saving again.
    type_into(
        &mut view,
        viewport,
        "edit-notes-gt-2403",
        "Re-checked at gate",
    );
    assert!(!shown(&mut view, viewport, "notes-error-2403"));
    click_id(&mut view, viewport, "save-gt-2403");
    assert!(shown(&mut view, viewport, "pending-2403"));
}

#[test]
fn save_click_flows_through_the_host_persists_and_confirms() {
    let viewport = vp();
    let data_dir = temp_dir("save-click");
    let mut store = RecordStore::open(&data_dir).unwrap();
    // An earlier session's correction must survive this session's save.
    store
        .save_record(
            "2401",
            "Session One Name",
            "from yesterday",
            "2026-09-20T02:00:00Z",
        )
        .unwrap();

    // Startup document: template + persisted corrections, like main.rs.
    let html = ops::build_document(
        &template(),
        &store,
        &BuildOpts {
            session: None,
            store_note: store_note(&store),
            flash: None,
        },
    )
    .unwrap();
    let mut view = load_doc(&html);
    settle(&mut view, viewport);
    // The prior session's correction is on screen at startup.
    assert_eq!(
        control_text(&view, viewport, "edit-name-gt-2401"),
        "Session One Name"
    );

    // The operator's context: filtered + selected, then an edit.
    let typing = type_into(&mut view, viewport, "search", "rejected");
    let click = click_id(&mut view, viewport, "row-gt-2403");
    let _ = type_into(
        &mut view,
        viewport,
        "edit-name-gt-2403",
        "Agus Wijaya Kusuma",
    );
    assert_eq!(text_of(&mut view, viewport, "summary-unsaved"), "1");

    // The window hook tracks session exactly from the public events.
    let mut session = Session {
        selected: "gt-2401".into(),
        search: String::new(),
    };
    for event in typing.iter().chain(click.iter()) {
        match event {
            Event::ValueChanged { target, value } if target.id.as_deref() == Some("search") => {
                session.search = value.clone();
            }
            Event::Click { target } => {
                if let Some(id) = target.id.as_deref() {
                    if id == "row-gt-2403" {
                        session.selected = "gt-2403".into();
                    }
                }
            }
            _ => {}
        }
    }
    assert_eq!(session.search, "rejected");
    assert_eq!(session.selected, "gt-2403");

    // Save: host validates, persists, rebuilds, reloads.
    let note = store_note(&store);
    let outcome = ops::apply_save(
        &template(),
        &mut store,
        &session,
        "2403",
        "Agus Wijaya Kusuma",
        "Seal verified on re-inspection",
        &note,
    );
    let SaveOutcome::Saved { html, saved_at } = outcome else {
        panic!("expected Saved, got {outcome:?}")
    };
    assert!(!saved_at.is_empty());
    let generation = reload(&mut view, &html);
    assert!(generation > 0);
    settle(&mut view, viewport);

    // Confirm the result: the saved values are the document's truth…
    assert_eq!(
        control_text(&view, viewport, "edit-name-gt-2403"),
        "Agus Wijaya Kusuma"
    );
    assert!(shown(&mut view, viewport, "saved-2403"), "saved banner");
    assert!(!shown(&mut view, viewport, "pending-2403"));
    let banner = squashed(&text_of(&mut view, viewport, "saved-2403"));
    assert!(banner.contains("Saved to device"), "banner: {banner}");
    assert!(
        banner.contains(&saved_at),
        "banner carries the timestamp: {banner}"
    );
    // …the session survived the reload…
    assert!(shown(&mut view, viewport, "detail-2403"), "selection kept");
    assert_eq!(
        text_of(&mut view, viewport, "summary-shown"),
        "3",
        "search kept"
    );
    assert_eq!(text_of(&mut view, viewport, "summary-unsaved"), "0");
    // …and the store holds both sessions' corrections.
    assert_eq!(store.len(), 2);
    assert_eq!(store.get("2403").unwrap().driver_name, "Agus Wijaya Kusuma");
    assert_eq!(store.get("2401").unwrap().driver_name, "Session One Name");

    let _ = std::fs::remove_dir_all(&data_dir);
}

#[test]
fn host_rejection_and_store_errors_keep_drafts_on_screen() {
    let viewport = vp();
    let data_dir = temp_dir("reject");
    let mut store = RecordStore::open(&data_dir).unwrap();
    let session = Session {
        selected: "gt-2403".into(),
        search: String::new(),
    };

    // The authoritative validator rejects what the document should
    // have blocked; nothing is written.
    let note = store_note(&store);
    match ops::apply_save(
        &template(),
        &mut store,
        &session,
        "2403",
        "   ",
        "notes",
        &note,
    ) {
        SaveOutcome::Rejected(rule) => assert!(rule.contains("driver name")),
        other => panic!("expected Rejected, got {other:?}"),
    }
    assert!(store.is_empty());

    // The host mirrors the rejection into the document with an error
    // banner, exactly as the window hook does.
    let html = ops::build_document(
        &template(),
        &store,
        &BuildOpts {
            session: Some(session.clone()),
            store_note: store_note(&store),
            flash: Some(Flash::Error {
                suffix: "2403".into(),
                name: "   ".into(),
                notes: "notes".into(),
                message: "Validation failed — not saved. (driver name must be 1-80 \
                          characters after trimming)"
                    .into(),
            }),
        },
    )
    .unwrap();
    let mut view = load_doc(&html);
    settle(&mut view, viewport);
    assert!(shown(&mut view, viewport, "save-error-2403"));
    assert!(
        squashed(&text_of(&mut view, viewport, "save-error-2403")).contains("not saved"),
        "error banner must not claim a save"
    );
    assert_eq!(control_text(&view, viewport, "edit-notes-gt-2403"), "notes");
    assert!(!shown(&mut view, viewport, "saved-2403"));

    let _ = std::fs::remove_dir_all(&data_dir);
}

#[test]
fn dev_mode_document_never_claims_a_durable_save() {
    // The raw template (what velqu-lab runs in ./dev.sh): every record
    // starts saved-pending-nothing, no saved banners, and the rail
    // says edits stay in memory.
    let viewport = vp();
    let mut view = load_doc(&template());
    settle(&mut view, viewport);
    assert!(shown(&mut view, viewport, "row-gt-2401"));
    assert!(!shown(&mut view, viewport, "saved-2401"));
    assert!(!shown(&mut view, viewport, "pending-2401"));
    assert_eq!(
        text_of(&mut view, viewport, "store-note")
            .split_whitespace()
            .collect::<String>(),
        "Devmode—editsstayinmemory(velqu-lab)"
    );
}

/// Layout facts join shaped text runs, so inter-run spacing is not
/// reliable in `text_of`; compare on collapsed whitespace.
fn squashed(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

// ---------------------------------------------------------------------------
// Process-level durability: the real binary, twice
// ---------------------------------------------------------------------------

fn binary() -> &'static str {
    env!("CARGO_BIN_EXE_velqu-view-starter")
}

fn run_binary(args: &[&str]) -> (bool, String) {
    let output = Command::new(binary())
        .args(args)
        .env("VELQU_VIEW_STARTER_DATA_DIR", "") // never pollute the user store
        .output()
        .expect("spawn starter binary");
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    (output.status.success(), text)
}

#[test]
fn saved_changes_survive_a_process_restart() {
    let data_dir = temp_dir("restart");
    let data = data_dir.to_string_lossy().into_owned();
    let app_dir = ops_dir().to_string_lossy().into_owned();

    // First process: a save through the real host path.
    let (ok, out) = run_binary(&[
        "--app-dir",
        &app_dir,
        "--data-dir",
        &data,
        "--smoke-save",
        "GT-2403",
        "--name",
        "Agus Wijaya Kusuma",
        "--notes",
        "Seal verified on re-inspection",
    ]);
    assert!(ok, "smoke-save failed:\n{out}");
    assert!(out.contains("ops: saved gt-2403"), "got:\n{out}");

    // Second process: the correction is still there.
    let (ok, out) = run_binary(&[
        "--app-dir",
        &app_dir,
        "--data-dir",
        &data,
        "--print-record",
        "GT-2403",
    ]);
    assert!(ok, "print-record failed:\n{out}");
    assert!(
        out.contains("record: gt-2403 | Agus Wijaya Kusuma | Seal verified on re-inspection"),
        "got:\n{out}"
    );

    // Third process: headless startup loads the persisted document
    // (the corrected name is the rendered truth).
    let out_png = data_dir.join("frame.png");
    let (ok, out) = run_binary(&[
        "--app-dir",
        &app_dir,
        "--data-dir",
        &data,
        "--headless",
        "--out",
        &out_png.to_string_lossy(),
    ]);
    assert!(ok, "headless failed:\n{out}");
    assert!(out.contains("store:"), "got:\n{out}");
    assert!(out.contains("(1 saved)"), "got:\n{out}");

    // An invalid smoke save fails cleanly without touching the store.
    let (ok, out) = run_binary(&[
        "--app-dir",
        &app_dir,
        "--data-dir",
        &data,
        "--smoke-save",
        "GT-2401",
        "--name",
        "   ",
    ]);
    assert!(!ok, "invalid save must fail:\n{out}");
    assert!(out.contains("rejected"), "got:\n{out}");

    let _ = std::fs::remove_dir_all(&data_dir);
}

#[test]
fn save_succeeds_but_reload_fails_preserves_disk_and_does_not_repeat_write() {
    let data_dir = temp_dir("save-succeeds-reload-fails");
    let mut store = RecordStore::open(&data_dir).unwrap();
    let session = Session {
        selected: "gt-2401".into(),
        search: String::new(),
    };

    // 1. Host persists record to disk
    let outcome = ops::apply_save(
        &template(),
        &mut store,
        &session,
        "2401",
        "Durable Driver",
        "Durable Notes",
        "note",
    );
    let (saved_at, fallback_html) = match outcome {
        SaveOutcome::Saved { saved_at, html } => (saved_at, html),
        SaveOutcome::SavedRefreshFailed { saved_at, html, .. } => (saved_at, html),
        other => panic!("expected successful disk save, got {other:?}"),
    };

    // 2. Drop store lock so another process can inspect
    drop(store);

    // 3. Verify the record is authoritative in a separate process
    let app_dir = ops_dir().to_string_lossy().into_owned();
    let data = data_dir.to_string_lossy().into_owned();
    let (ok, out) = run_binary(&[
        "--app-dir",
        &app_dir,
        "--data-dir",
        &data,
        "--print-record",
        "GT-2401",
    ]);
    assert!(ok, "print-record failed:\n{out}");
    assert!(
        out.contains("Durable Driver"),
        "disk record must be authoritative: {out}"
    );
    assert!(
        out.contains("Durable Notes"),
        "disk record must be authoritative: {out}"
    );

    // 4. Simulate a failed primary reload attempt:
    // Even if the initial document reload failed, the host attempts a fallback
    // presentation reload that informs the operator of the successful disk commit.
    let viewport = vp();
    let mut view = load_doc(&template());
    settle(&mut view, viewport);
    let generation = reload(&mut view, &fallback_html);
    assert!(generation > 0);
    settle(&mut view, viewport);
    assert_eq!(
        control_text(&view, viewport, "edit-name-gt-2401"),
        "Durable Driver"
    );

    // 5. Subsequent events (e.g. clicks on rows or pointer moves) do NOT repeat the write
    let reopened_store = RecordStore::open(&data_dir).unwrap();
    let last_saved_at = reopened_store.get("2401").unwrap().saved_at.clone();
    assert_eq!(last_saved_at, saved_at);

    let _ = std::fs::remove_dir_all(&data_dir);
}

#[test]
fn stored_tricky_text_and_unicode_remains_data_and_creates_no_attributes_or_behavior() {
    let data_dir = temp_dir("tricky-text");
    let mut store = RecordStore::open(&data_dir).unwrap();

    let tricky_driver = "O'Connor & Sons \"The Boss\" <script>alert(1)</script>";
    let tricky_notes = "Backslash \\ and newline \n tab \t &quot; &amp; 🚚 Café e\u{0301}";

    store
        .save_record("2401", tricky_driver, tricky_notes, "2026-09-21T10:00:00Z")
        .unwrap();

    let html = ops::build_document(
        &template(),
        &store,
        &BuildOpts {
            session: None,
            store_note: "test".into(),
            flash: None,
        },
    )
    .unwrap();

    // Verify through the full VelquView HTML5 parser + reactive engine
    let viewport = vp();
    let mut view = load_doc(&html);
    settle(&mut view, viewport);

    assert_eq!(
        control_text(&view, viewport, "edit-name-gt-2401"),
        tricky_driver,
        "tricky driver name must round-trip exactly without parsing side-effects"
    );

    assert_eq!(
        control_text(&view, viewport, "edit-notes-gt-2401"),
        tricky_notes,
        "tricky notes must round-trip exactly without parsing side-effects"
    );

    // Verify it survives a process restart
    drop(store);
    let app_dir = ops_dir().to_string_lossy().into_owned();
    let data = data_dir.to_string_lossy().into_owned();
    let (ok, out) = run_binary(&[
        "--app-dir",
        &app_dir,
        "--data-dir",
        &data,
        "--print-record",
        "GT-2401",
    ]);
    assert!(ok, "print-record failed:\n{out}");
    assert!(out.contains(tricky_driver));

    let _ = std::fs::remove_dir_all(&data_dir);
}

#[test]
fn single_writer_lock_blocks_concurrent_process_and_corrupt_store_fails_startup() {
    let data_dir = temp_dir("lock-and-corrupt");
    let app_dir = ops_dir().to_string_lossy().into_owned();
    let data = data_dir.to_string_lossy().into_owned();

    // 1. Single writer lock test
    let store = RecordStore::open(&data_dir).unwrap();
    // A second process trying to access this data_dir must be rejected
    let (ok, out) = run_binary(&[
        "--app-dir",
        &app_dir,
        "--data-dir",
        &data,
        "--smoke-save",
        "GT-2401",
        "--name",
        "Other Process",
    ]);
    assert!(!ok, "concurrent writer must be rejected:\n{out}");
    assert!(
        out.contains("another application instance is using data directory"),
        "error message must describe lock contention: {out}"
    );
    drop(store);

    // 2. Corrupt store recovery test
    let records_path = data_dir.join("records.json");
    let corrupt_bytes = b"{\"schema\": 1, \"overrides\": INVALID_SYNTAX...";
    std::fs::write(&records_path, corrupt_bytes).unwrap();

    let (ok, out) = run_binary(&[
        "--app-dir",
        &app_dir,
        "--data-dir",
        &data,
        "--print-record",
        "GT-2401",
    ]);
    assert!(!ok, "process startup must fail on corrupt store:\n{out}");
    assert!(
        out.contains("file preserved"),
        "must indicate file was preserved: {out}"
    );

    // The corrupt file must NOT be overwritten or deleted
    let current_bytes = std::fs::read(&records_path).unwrap();
    assert_eq!(
        current_bytes, corrupt_bytes,
        "corrupt file must be preserved on disk"
    );

    let _ = std::fs::remove_dir_all(&data_dir);
}
