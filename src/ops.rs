//! The operations-workflow host concern: durable, on-device persistence
//! for the gate-transaction review app.
//!
//! The division of labor (the reviewer's contract):
//!
//! * The **UI runtime** (VelquView + Velqu Reactive) never touches the
//!   disk. Clicking *Save* only sets a reactive `pending` flag — the
//!   document on screen never claims a durable save by itself.
//! * The **application host** (this binary) owns storage. It observes
//!   the public event queue, re-validates authoritatively, writes
//!   `records.json` in an explicit data directory (atomic tmp+rename),
//!   and reloads the document from persisted truth. A change that has
//!   not been through the host is in-memory only; a change that has
//!   survives a restart. No server integration exists in this pilot —
//!   a future host adapter would live beside [`RecordStore`], not in
//!   the renderer.
//!
//! The single seam between the two is the template's `vx-state`
//! attribute: [`parse_state_block`] reads the authored seed block
//! (strict one-pair-per-line format), the host applies overrides from
//! the store, and [`inject_state`] writes the attribute back before a
//! document reload. Everything else in `ops/index.html` is byte-stable
//! between dev mode (velqu-lab, seed values) and the release app.

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

/// Permitted driver-name length after trimming (workflow rule 1).
pub const NAME_MAX_CHARS: usize = 80;
/// Permitted notes length (workflow rule 2).
pub const NOTES_MAX_CHARS: usize = 280;

/// The authoritative validation rules. The document mirrors them
/// reactively (error lines + disabled Save); the host re-checks the
/// same rules before persisting — the UI is never trusted for storage.
pub fn validate(name: &str, notes: &str) -> Result<(), &'static str> {
    let trimmed = name.trim();
    if trimmed.is_empty() || trimmed.chars().count() > NAME_MAX_CHARS {
        return Err("driver name must be 1-80 characters after trimming");
    }
    if notes.chars().count() > NOTES_MAX_CHARS {
        return Err("notes must be 280 characters or fewer");
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// vx-state block: strict-format parse / render / inject
// ---------------------------------------------------------------------------

/// One `key: value` pair of the state block.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StateValue {
    /// A single-quoted JS string.
    Str(String),
    /// A boolean literal.
    Bool(bool),
}

impl StateValue {
    /// The string value, if this pair is a string.
    pub fn as_str(&self) -> Option<&str> {
        match self {
            StateValue::Str(value) => Some(value),
            StateValue::Bool(_) => None,
        }
    }
}

/// The attribute marker the host rewrites. Exactly one may appear.
const STATE_ATTR: &str = "vx-state=\"";

/// Parses the template's single `vx-state` block into ordered pairs.
///
/// The format is strict on purpose (the generator guarantees it for
/// seeds, the serializer for injected values): one `key: value` per
/// line, string values single-quoted with the escapes below, boolean
/// values bare. Anything else is an error, never a guess.
pub fn parse_state_block(html: &str) -> Result<Vec<(String, StateValue)>, String> {
    let occurrences = html.match_indices(STATE_ATTR).count();
    if occurrences != 1 {
        return Err(format!(
            "expected exactly one {STATE_ATTR} attribute, found {occurrences}"
        ));
    }
    let start = html.find(STATE_ATTR).expect("counted above") + STATE_ATTR.len();
    let end = html[start..]
        .find('"')
        .map(|offset| start + offset)
        .ok_or("unterminated vx-state attribute")?;
    let block = html[start..end].trim();
    let body = block
        .strip_prefix('{')
        .and_then(|rest| rest.strip_suffix('}'))
        .ok_or("state block must be brace-wrapped")?;

    let mut pairs = Vec::new();
    for line in body.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let line = line
            .strip_suffix(',')
            .ok_or_else(|| format!("state line must end with a comma: {line:?}"))?;
        let (key, value) = line
            .split_once(':')
            .ok_or_else(|| format!("state line must be key: value: {line:?}"))?;
        let key = key.trim();
        if key.is_empty() || !key.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
            return Err(format!("invalid state key {key:?}"));
        }
        let value = value.trim();
        let parsed = if value == "true" {
            StateValue::Bool(true)
        } else if value == "false" {
            StateValue::Bool(false)
        } else if value.starts_with('\'') && value.ends_with('\'') && value.len() >= 2 {
            StateValue::Str(unescape_js(&unescape_html(&value[1..value.len() - 1]))?)
        } else {
            return Err(format!("invalid state value for {key}: {value:?}"));
        };
        pairs.push((key.to_owned(), parsed));
    }
    Ok(pairs)
}

/// Renders pairs back into the attribute body, matching the authored
/// template layout (one pair per line, closing brace dedented).
pub fn render_state_body(pairs: &[(String, StateValue)]) -> String {
    let mut out = String::from("{\n");
    for (key, value) in pairs {
        match value {
            StateValue::Str(text) => {
                let _ = writeln!(out, "      {key}: '{}',", escape_for_attribute(text));
            }
            StateValue::Bool(flag) => {
                let _ = writeln!(out, "      {key}: {flag},");
            }
        }
    }
    out.push_str("    }");
    out
}

/// Replaces the template's state block with `pairs`.
pub fn inject_state(html: &str, pairs: &[(String, StateValue)]) -> Result<String, String> {
    let occurrences = html.match_indices(STATE_ATTR).count();
    if occurrences != 1 {
        return Err(format!(
            "expected exactly one {STATE_ATTR} attribute, found {occurrences}"
        ));
    }
    let attr_start = html.find(STATE_ATTR).expect("counted above");
    let value_start = attr_start + STATE_ATTR.len();
    let value_end = html[value_start..]
        .find('"')
        .map(|offset| value_start + offset)
        .ok_or("unterminated vx-state attribute")?;
    let mut out = String::with_capacity(html.len() + 256);
    out.push_str(&html[..value_start]);
    out.push_str(&render_state_body(pairs));
    out.push_str(&html[value_end..]);
    Ok(out)
}

/// Escapes a value for the two layers it crosses: the JS string literal
/// Escapes a value for the two layers it crosses: the JS string literal
/// first, then the double-quoted HTML attribute.
///
/// Quotes are encoded as Unicode escapes (`\u0027`, `\u0022`): this prevents
/// quotes from closing either the HTML attribute or the single-quoted JS literal,
/// while guaranteeing compatibility with reactive lexical scanners that do not
/// track backslash-escaped quote characters inside strings.
pub fn escape_for_attribute(value: &str) -> String {
    let mut out = String::with_capacity(value.len() * 2);
    for c in value.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '\'' => out.push_str("\\u0027"),
            '"' => out.push_str("\\u0022"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            other => out.push(other),
        }
    }
    out
}

/// Inverts the HTML-attribute escapes this module emits (`&amp;`,
/// `&quot;`, `&lt;`, `&gt;`). Entity-aware scanning prevents accidental
/// substring corruption (e.g. literal `&quot;` becoming `&"`).
fn unescape_html(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    let mut rest = value;
    while let Some(pos) = rest.find('&') {
        out.push_str(&rest[..pos]);
        let after = &rest[pos..];
        if let Some(stripped) = after.strip_prefix("&quot;") {
            out.push('"');
            rest = stripped;
        } else if let Some(stripped) = after.strip_prefix("&amp;") {
            out.push('&');
            rest = stripped;
        } else if let Some(stripped) = after.strip_prefix("&lt;") {
            out.push('<');
            rest = stripped;
        } else if let Some(stripped) = after.strip_prefix("&gt;") {
            out.push('>');
            rest = stripped;
        } else {
            out.push('&');
            rest = &after[1..];
        }
    }
    out.push_str(rest);
    out
}

/// Inverts the JS-string escapes. Supports named escapes (`\'`, `\"`, `\\`,
/// `\n`, `\r`, `\t`) and 4-digit unicode escapes (`\uXXXX`).
fn unescape_js(value: &str) -> Result<String, String> {
    let mut out = String::with_capacity(value.len());
    let mut chars = value.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('\'') => out.push('\''),
            Some('"') => out.push('"'),
            Some('\\') => out.push('\\'),
            Some('n') => out.push('\n'),
            Some('r') => out.push('\r'),
            Some('t') => out.push('\t'),
            Some('u') => {
                let mut hex = String::with_capacity(4);
                for _ in 0..4 {
                    if let Some(h) = chars.next() {
                        hex.push(h);
                    } else {
                        return Err(format!("incomplete \\u escape in {value:?}"));
                    }
                }
                let code = u32::from_str_radix(&hex, 16)
                    .map_err(|e| format!("invalid \\u escape {hex:?}: {e}"))?;
                let ch =
                    char::from_u32(code).ok_or_else(|| format!("invalid unicode char {code:?}"))?;
                out.push(ch);
            }
            other => return Err(format!("unknown escape {other:?} in {value:?}")),
        }
    }
    Ok(out)
}

// ---------------------------------------------------------------------------
// Record store: records.json in an explicit data directory
// ---------------------------------------------------------------------------

use std::fs::{File, OpenOptions};
use std::io::Write;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

static TMP_FILE_COUNTER: AtomicU64 = AtomicU64::new(1);

/// One persisted record correction. Keys are the numeric record suffix
/// (`"2401"` for GT-2401) so store entries never depend on the DOM.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Override {
    /// Corrected driver name.
    pub driver_name: String,
    /// Corrected notes.
    pub notes: String,
    /// When the host wrote this correction (RFC 3339, UTC).
    pub saved_at: String,
}

#[derive(Debug, serde::Serialize, serde::Deserialize)]
struct StoreFile {
    schema: u32,
    overrides: BTreeMap<String, Override>,
}

/// The on-device store. File format: `records.json` holding every
/// persisted correction over the template's seed values.
#[derive(Debug, Clone)]
pub struct RecordStore {
    data_dir: PathBuf,
    path: PathBuf,
    overrides: BTreeMap<String, Override>,
    _lock: Option<Arc<File>>,
}

impl RecordStore {
    /// Opens (creating the directory if needed) the store. A missing
    /// file is an empty store — the first run seeds nothing and every
    /// record shows its authored values.
    ///
    /// Enforces a single-writer policy per data directory via an advisory
    /// kernel file lock (`flock`). If another instance holds the lock,
    /// opening fails with a descriptive error.
    ///
    /// If an existing `records.json` is corrupt or has an invalid schema,
    /// this function fails and preserves the problematic file on disk
    /// (does not overwrite or reinitialize as empty).
    pub fn open(data_dir: &Path) -> Result<Self, String> {
        std::fs::create_dir_all(data_dir)
            .map_err(|e| format!("cannot create data dir {}: {e}", data_dir.display()))?;

        let lock_path = data_dir.join(".lock");
        let lock_file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(&lock_path)
            .map_err(|e| format!("cannot open lockfile {}: {e}", lock_path.display()))?;

        rustix::fs::flock(
            &lock_file,
            rustix::fs::FlockOperation::NonBlockingLockExclusive,
        )
        .map_err(|e| {
            if e == rustix::io::Errno::WOULDBLOCK || e == rustix::io::Errno::AGAIN {
                format!(
                    "another application instance is using data directory {} (lock on {})",
                    data_dir.display(),
                    lock_path.display()
                )
            } else {
                format!("failed to lock {}: {e}", lock_path.display())
            }
        })?;

        let path = data_dir.join("records.json");
        let overrides = match std::fs::read_to_string(&path) {
            Ok(text) => {
                let file: StoreFile = serde_json::from_str(&text).map_err(|e| {
                    format!(
                        "cannot parse corrupt or invalid records file {} (file preserved): {e}",
                        path.display()
                    )
                })?;
                if file.schema != 1 {
                    return Err(format!(
                        "records file {} has schema {} (expected 1; file preserved)",
                        path.display(),
                        file.schema
                    ));
                }
                file.overrides
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => BTreeMap::new(),
            Err(e) => return Err(format!("cannot read {}: {e}", path.display())),
        };
        Ok(Self {
            data_dir: data_dir.to_owned(),
            path,
            overrides,
            _lock: Some(Arc::new(lock_file)),
        })
    }

    /// The data directory path.
    pub fn data_dir(&self) -> &Path {
        &self.data_dir
    }

    /// The store file path (shown to operators in the rail note).
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Number of persisted corrections.
    pub fn len(&self) -> usize {
        self.overrides.len()
    }

    /// Whether nothing has been persisted yet.
    pub fn is_empty(&self) -> bool {
        self.overrides.is_empty()
    }

    /// One persisted correction, if any.
    pub fn get(&self, suffix: &str) -> Option<&Override> {
        self.overrides.get(suffix)
    }

    /// Persisted record suffixes, in store order.
    pub fn suffixes(&self) -> Vec<String> {
        self.overrides.keys().cloned().collect()
    }

    /// Persists one correction with local-filesystem crash durability:
    ///
    /// 1. Create a unique temporary file in the destination directory
    /// 2. Write the complete serialized record set
    /// 3. Flush the userspace writer
    /// 4. Sync the temporary file (`sync_all`)
    /// 5. Atomically rename over `records.json`
    /// 6. Sync the parent directory to commit the directory entry
    pub fn save_record(
        &mut self,
        suffix: &str,
        driver_name: &str,
        notes: &str,
        saved_at: &str,
    ) -> std::io::Result<()> {
        self.overrides.insert(
            suffix.to_owned(),
            Override {
                driver_name: driver_name.to_owned(),
                notes: notes.to_owned(),
                saved_at: saved_at.to_owned(),
            },
        );
        let file = StoreFile {
            schema: 1,
            overrides: self.overrides.clone(),
        };
        let text = serde_json::to_string_pretty(&file)
            .map_err(|e| std::io::Error::other(format!("serialize: {e}")))?;

        let counter = TMP_FILE_COUNTER.fetch_add(1, Ordering::Relaxed);
        let tmp = self.data_dir.join(format!(
            ".records.json.tmp.{}.{}.{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0),
            counter
        ));

        let mut tmp_file = OpenOptions::new().write(true).create_new(true).open(&tmp)?;
        tmp_file.write_all(text.as_bytes())?;
        tmp_file.flush()?;
        tmp_file.sync_all()?;
        drop(tmp_file);

        std::fs::rename(&tmp, &self.path)?;

        // The rename already made the new store own the pathname, so a
        // failure from here on must NOT be reported as "not saved" —
        // the write stands. But the directory entry's durability
        // across power loss is only established by this sync, so a
        // failure is surfaced loudly rather than swallowed.
        match File::open(&self.data_dir).and_then(|dir| dir.sync_all()) {
            Ok(()) => {}
            Err(error) => {
                eprintln!(
                    "ops: WARNING — {} is saved, but syncing its directory failed ({error}); \
                     the save may not survive a sudden power loss",
                    self.path.display()
                );
            }
        }

        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Document rebuild: template + store (+ session) → loadable HTML
// ---------------------------------------------------------------------------

/// Live session facts to preserve across a save-reload (search text and
/// selected record), so saving does not throw the operator back to an
/// unfiltered list.
#[derive(Debug, Clone, Default)]
pub struct Session {
    /// Selected record key, e.g. `"gt-2403"`.
    pub selected: String,
    /// Current search text.
    pub search: String,
}

/// The post-save state the reloaded document should carry.
#[derive(Debug, Clone)]
pub enum Flash {
    /// The save persisted; the banner shows the host timestamp.
    Saved {
        /// Record suffix.
        suffix: String,
        /// RFC 3339 timestamp written to the store.
        saved_at: String,
    },
    /// The save succeeded on disk, but presentation refresh had an issue.
    SavedRefreshFailed {
        /// Record suffix.
        suffix: String,
        /// RFC 3339 timestamp written to the store.
        saved_at: String,
        /// Notice explaining that disk persistence succeeded.
        message: String,
    },
    /// The save failed; drafts stay on screen with the error visible.
    Error {
        /// Record suffix.
        suffix: String,
        /// Attempted driver name (kept in the draft field).
        name: String,
        /// Attempted notes (kept in the draft field).
        notes: String,
        /// Operator-facing failure message.
        message: String,
    },
}

/// Everything [`build_document`] needs beyond the template and store.
#[derive(Debug, Clone, Default)]
pub struct BuildOpts {
    /// Session facts to preserve (`None` = template defaults).
    pub session: Option<Session>,
    /// The rail note distinguishing dev mode from the on-device store.
    pub store_note: String,
    /// Post-save state for the reloaded document.
    pub flash: Option<Flash>,
}

/// Upserts a string pair, appending keys the template did not declare.
fn upsert_str(
    pairs: &mut Vec<(String, StateValue)>,
    existing: &std::collections::HashSet<String>,
    key: String,
    value: String,
) {
    if existing.contains(&key) {
        for pair in pairs.iter_mut() {
            if pair.0 == key {
                pair.1 = StateValue::Str(value);
                break;
            }
        }
    } else {
        pairs.push((key, StateValue::Str(value)));
    }
}

/// Upserts a boolean pair, appending keys the template did not declare.
fn upsert_bool(
    pairs: &mut Vec<(String, StateValue)>,
    existing: &std::collections::HashSet<String>,
    key: String,
    value: bool,
) {
    if existing.contains(&key) {
        for pair in pairs.iter_mut() {
            if pair.0 == key {
                pair.1 = StateValue::Bool(value);
                break;
            }
        }
    } else {
        pairs.push((key, StateValue::Bool(value)));
    }
}

/// Builds the loadable document: template pairs, store overrides
/// applied, session preserved, flash honored.
pub fn build_document(
    template: &str,
    store: &RecordStore,
    opts: &BuildOpts,
) -> Result<String, String> {
    let mut pairs = parse_state_block(template)?;
    let existing: std::collections::HashSet<String> =
        pairs.iter().map(|(key, _)| key.clone()).collect();

    // Persisted corrections override the seed values for every record.
    for suffix in store.suffixes() {
        let entry = store.get(&suffix).expect("keys came from the store");
        upsert_str(
            &mut pairs,
            &existing,
            format!("name_{suffix}"),
            entry.driver_name.clone(),
        );
        upsert_str(
            &mut pairs,
            &existing,
            format!("notes_{suffix}"),
            entry.notes.clone(),
        );
        upsert_bool(&mut pairs, &existing, format!("saved_{suffix}"), true);
        upsert_str(
            &mut pairs,
            &existing,
            format!("saved_at_{suffix}"),
            entry.saved_at.clone(),
        );
    }
    if let Some(session) = &opts.session {
        upsert_str(
            &mut pairs,
            &existing,
            "selected".to_owned(),
            session.selected.clone(),
        );
        upsert_str(
            &mut pairs,
            &existing,
            "search".to_owned(),
            session.search.clone(),
        );
    }
    if !opts.store_note.is_empty() {
        upsert_str(
            &mut pairs,
            &existing,
            "store_note".to_owned(),
            opts.store_note.clone(),
        );
    }
    match &opts.flash {
        Some(Flash::Saved { suffix, saved_at }) => {
            upsert_str(
                &mut pairs,
                &existing,
                format!("saved_at_{suffix}"),
                saved_at.clone(),
            );
            upsert_bool(&mut pairs, &existing, format!("saved_{suffix}"), true);
            upsert_bool(&mut pairs, &existing, format!("pending_{suffix}"), false);
            upsert_str(
                &mut pairs,
                &existing,
                format!("error_{suffix}"),
                String::new(),
            );
        }
        Some(Flash::SavedRefreshFailed {
            suffix,
            saved_at,
            message,
        }) => {
            upsert_str(
                &mut pairs,
                &existing,
                format!("saved_at_{suffix}"),
                saved_at.clone(),
            );
            upsert_bool(&mut pairs, &existing, format!("saved_{suffix}"), true);
            upsert_bool(&mut pairs, &existing, format!("pending_{suffix}"), false);
            upsert_str(
                &mut pairs,
                &existing,
                format!("error_{suffix}"),
                message.clone(),
            );
        }
        Some(Flash::Error {
            suffix,
            name,
            notes,
            message,
        }) => {
            upsert_str(
                &mut pairs,
                &existing,
                format!("name_{suffix}"),
                name.clone(),
            );
            upsert_str(
                &mut pairs,
                &existing,
                format!("notes_{suffix}"),
                notes.clone(),
            );
            upsert_bool(&mut pairs, &existing, format!("saved_{suffix}"), false);
            upsert_bool(&mut pairs, &existing, format!("pending_{suffix}"), false);
            upsert_str(
                &mut pairs,
                &existing,
                format!("error_{suffix}"),
                message.clone(),
            );
        }
        None => {}
    }
    inject_state(template, &pairs)
}

/// The result of one host-side save attempt.
#[derive(Debug)]
pub enum SaveOutcome {
    /// Persisted; `html` is the reloaded document, `saved_at` the
    /// written timestamp.
    Saved {
        /// Document to load.
        html: String,
        /// Timestamp recorded in the store.
        saved_at: String,
    },
    /// The authoritative validator rejected the edit; nothing was
    /// written and the on-screen state is unchanged.
    Rejected(&'static str),
    /// The store write failed; `html` keeps the drafts with the error
    /// visible so the operator can retry.
    StoreError {
        /// Document to load.
        html: String,
        /// Error message.
        message: String,
    },
    /// The store write succeeded, but generating the normal replacement
    /// document failed. The stored record is authoritative.
    SavedRefreshFailed {
        /// Timestamp recorded in the store.
        saved_at: String,
        /// Fallback document to load.
        html: String,
        /// Notice message.
        message: String,
    },
}

/// The full host save path: validate → persist → rebuild.
pub fn apply_save(
    template: &str,
    store: &mut RecordStore,
    session: &Session,
    suffix: &str,
    name: &str,
    notes: &str,
    store_note: &str,
) -> SaveOutcome {
    if let Err(rule) = validate(name, notes) {
        return SaveOutcome::Rejected(rule);
    }
    let saved_at = now_rfc3339_utc();
    if let Err(error) = store.save_record(suffix, name, notes, &saved_at) {
        let message = format!(
            "Could not write {} — your edits are on screen only. Retry Save. ({error})",
            store.path().display()
        );
        let html = build_document(
            template,
            store,
            &BuildOpts {
                session: Some(session.clone()),
                store_note: store_note.to_owned(),
                flash: Some(Flash::Error {
                    suffix: suffix.to_owned(),
                    name: name.to_owned(),
                    notes: notes.to_owned(),
                    message: message.clone(),
                }),
            },
        )
        .unwrap_or_else(|e| {
            // Even the error flash failed to build: fall back to the
            // raw template rather than losing the session entirely.
            eprintln!("ops: rebuilding after store error failed: {e}");
            template.to_owned()
        });
        return SaveOutcome::StoreError { html, message };
    }
    match build_document(
        template,
        store,
        &BuildOpts {
            session: Some(session.clone()),
            store_note: store_note.to_owned(),
            flash: Some(Flash::Saved {
                suffix: suffix.to_owned(),
                saved_at: saved_at.clone(),
            }),
        },
    ) {
        Ok(html) => SaveOutcome::Saved { html, saved_at },
        Err(e) => {
            // The store write succeeded; only the document rebuild
            // failed. The record is durably on disk.
            eprintln!("ops: document rebuild after save failed: {e}");
            let message = format!("Saved to device at {saved_at}; screen refresh failed ({e})");
            let fallback_html = build_document(
                template,
                store,
                &BuildOpts {
                    session: Some(session.clone()),
                    store_note: store_note.to_owned(),
                    flash: Some(Flash::SavedRefreshFailed {
                        suffix: suffix.to_owned(),
                        saved_at: saved_at.clone(),
                        message: message.clone(),
                    }),
                },
            )
            .unwrap_or_else(|_| template.to_owned());
            SaveOutcome::SavedRefreshFailed {
                saved_at,
                html: fallback_html,
                message,
            }
        }
    }
}

/// Extracts the record suffix from a workflow element id:
/// `save-gt-2401`, `row-gt-2403`, `edit-name-gt-2401`,
/// `edit-notes-gt-2401` → `Some("2401")`.
pub fn record_suffix(id: &str) -> Option<&str> {
    let (_, suffix) = id.rsplit_once("-gt-")?;
    if !suffix.is_empty() && suffix.chars().all(|c| c.is_ascii_digit()) {
        Some(suffix)
    } else {
        None
    }
}

/// The default data directory when no flag/env is given:
/// `$XDG_DATA_HOME/velqu-view-starter`, else
/// `~/.local/share/velqu-view-starter`.
pub fn default_data_dir() -> PathBuf {
    if let Some(xdg) = std::env::var_os("XDG_DATA_HOME") {
        if !xdg.is_empty() {
            return PathBuf::from(xdg).join("velqu-view-starter");
        }
    }
    let home = std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."));
    home.join(".local").join("share").join("velqu-view-starter")
}

/// `YYYY-MM-DDTHH:MM:SSZ` from the system clock (UTC, no external
/// time crates; the civil-from-days algorithm is Howard Hinnant's).
pub fn now_rfc3339_utc() -> String {
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let days = (secs / 86_400) as i64;
    let (year, month, day) = civil_from_days(days);
    let rem = secs % 86_400;
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}Z",
        rem / 3600,
        (rem % 3600) / 60,
        rem % 60
    )
}

fn civil_from_days(days: i64) -> (i64, u64, u64) {
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as u64;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let year = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    (if month <= 2 { year + 1 } else { year }, month, day)
}

#[cfg(test)]
mod tests {
    use super::*;

    const MINIMAL: &str = r#"<main vx-state="{
      selected: 'gt-2401',
      search: '',
      name_2401: 'Budi Santoso',
      notes_2401: 'Late arrival, rescheduled',
      saved_2401: true,
      pending_2401: false,
      error_2401: '',
      saved_at_2401: '',
    }" class="flex"></main>"#;

    #[test]
    fn parse_reads_the_strict_format_in_order() {
        let pairs = parse_state_block(MINIMAL).unwrap();
        assert_eq!(
            pairs[0],
            ("selected".into(), StateValue::Str("gt-2401".into()))
        );
        assert_eq!(pairs[1], ("search".into(), StateValue::Str(String::new())));
        assert_eq!(pairs[2].0, "name_2401");
        assert_eq!(pairs[2].1.as_str(), Some("Budi Santoso"));
        // A comma inside a value survives (line-based, not comma-split).
        assert_eq!(pairs[3].1.as_str(), Some("Late arrival, rescheduled"));
        assert_eq!(pairs[4], ("saved_2401".into(), StateValue::Bool(true)));
        assert_eq!(pairs[5], ("pending_2401".into(), StateValue::Bool(false)));
    }

    #[test]
    fn second_attribute_or_missing_brace_is_an_error() {
        let doubled = format!("{MINIMAL}{MINIMAL}");
        assert!(parse_state_block(&doubled).is_err());
        let no_braces = r#"<main vx-state="selected: 'x'"></main>"#;
        assert!(parse_state_block(no_braces).is_err());
    }

    #[test]
    fn inject_round_trips_values_and_escapes_both_layers() {
        let pairs = vec![
            ("plain".to_owned(), StateValue::Str("Budi Santoso".into())),
            (
                "tricky".to_owned(),
                StateValue::Str("it's a \"test\" & \\ back\nline\ttab".into()),
            ),
            ("flag".to_owned(), StateValue::Bool(false)),
        ];
        let html = inject_state(MINIMAL, &pairs).unwrap();
        assert!(
            html.contains(
                "tricky: 'it\\u0027s a \\u0022test\\u0022 &amp; \\\\ back\\nline\\ttab',"
            )
        );
        let back = parse_state_block(&html).unwrap();
        assert_eq!(back, pairs);
    }

    #[test]
    fn validation_rules_match_the_documented_limits() {
        assert!(validate("Budi Santoso", "").is_ok());
        assert!(validate("  Budi  ", "notes").is_ok());
        assert!(validate("", "").is_err());
        assert!(validate("   ", "").is_err());
        assert!(validate(&"x".repeat(81), "").is_err());
        assert!(validate(&"x".repeat(80), "").is_ok());
        assert!(validate("ok", &"n".repeat(280)).is_ok());
        assert!(validate("ok", &"n".repeat(281)).is_err());
    }

    #[test]
    fn record_suffix_accepts_workflow_ids_only() {
        assert_eq!(record_suffix("save-gt-2401"), Some("2401"));
        assert_eq!(record_suffix("row-gt-2403"), Some("2403"));
        assert_eq!(record_suffix("edit-name-gt-2401"), Some("2401"));
        assert_eq!(record_suffix("edit-notes-gt-2401"), Some("2401"));
        assert_eq!(record_suffix("search"), None);
        assert_eq!(record_suffix("save-gt-24x1"), None);
        assert_eq!(record_suffix("gt-"), None);
    }

    #[test]
    fn store_round_trips_atomically_and_seeds_empty() {
        let dir = std::env::temp_dir().join(format!("vv-starter-store-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let mut store = RecordStore::open(&dir).unwrap();
        assert!(store.is_empty());
        store
            .save_record("2401", "Budi Jr", "ok", "2026-09-21T07:00:00Z")
            .unwrap();
        // Drop the first instance to release the lock, simulating process exit
        drop(store);
        // Reopen: the correction survived the process boundary.
        let reopened = RecordStore::open(&dir).unwrap();
        assert_eq!(reopened.len(), 1);
        assert_eq!(reopened.get("2401").unwrap().driver_name, "Budi Jr");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn civil_dates_match_known_days() {
        assert_eq!(civil_from_days(0), (1970, 1, 1));
        assert_eq!(civil_from_days(20_717), (2026, 9, 21));
    }

    #[test]
    fn build_document_applies_overrides_session_and_flash() {
        let dir = std::env::temp_dir().join(format!("vv-starter-build-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let mut store = RecordStore::open(&dir).unwrap();
        store
            .save_record(
                "2401",
                "Budi Santoso Jr.",
                "corrected",
                "2026-09-21T07:00:00Z",
            )
            .unwrap();
        let html = build_document(
            MINIMAL,
            &store,
            &BuildOpts {
                session: Some(Session {
                    selected: "gt-2403".into(),
                    search: "gate b".into(),
                }),
                store_note: "On-device store: /tmp/x".into(),
                flash: None,
            },
        )
        .unwrap();
        assert!(html.contains("name_2401: 'Budi Santoso Jr.',"));
        assert!(html.contains("notes_2401: 'corrected',"));
        assert!(html.contains("saved_at_2401: '2026-09-21T07:00:00Z',"));
        assert!(html.contains("selected: 'gt-2403',"));
        assert!(html.contains("search: 'gate b',"));
        assert!(html.contains("store_note: 'On-device store: /tmp/x',"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn apply_save_persists_then_rebuilds_or_reports() {
        let dir = std::env::temp_dir().join(format!("vv-starter-apply-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let mut store = RecordStore::open(&dir).unwrap();
        let session = Session::default();

        let rejected = apply_save(MINIMAL, &mut store, &session, "2401", "  ", "", "note");
        assert!(matches!(rejected, SaveOutcome::Rejected(_)));
        assert!(store.is_empty(), "a rejected edit must not touch the store");

        match apply_save(
            MINIMAL, &mut store, &session, "2401", "Budi Jr", "fixed", "note",
        ) {
            SaveOutcome::Saved { html, saved_at } => {
                assert!(!saved_at.is_empty());
                assert!(html.contains("name_2401: 'Budi Jr',"));
                assert!(store.get("2401").unwrap().driver_name == "Budi Jr");
            }
            other => panic!("expected Saved, got {other:?}"),
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn single_writer_lock_prevents_concurrent_access() {
        let dir = std::env::temp_dir().join(format!("vv-starter-lock-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let store1 = RecordStore::open(&dir).expect("first open succeeds");
        assert_eq!(store1.len(), 0);

        let err = RecordStore::open(&dir).expect_err("second open must fail while lock is held");
        assert!(
            err.contains("another application instance is using data directory"),
            "got error: {err}"
        );

        drop(store1);

        let store2 = RecordStore::open(&dir).expect("open succeeds after first is dropped");
        assert_eq!(store2.len(), 0);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn corrupt_store_file_is_preserved_and_never_overwritten() {
        let dir = std::env::temp_dir().join(format!("vv-starter-corrupt-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("records.json");
        let corrupt_payload = "{\"schema\": 1, \"overrides\": { TRUNCATED JSON...";
        std::fs::write(&path, corrupt_payload).unwrap();

        let err = RecordStore::open(&dir).expect_err("opening corrupt store must fail");
        assert!(err.contains("file preserved"), "got error: {err}");

        // Verify the file was not deleted or altered
        let on_disk = std::fs::read_to_string(&path).unwrap();
        assert_eq!(on_disk, corrupt_payload);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn unicode_and_tricky_characters_round_trip_cleanly() {
        let tricky_cases = [
            "O'Connor & Sons",
            "A quoted \"nickname\"",
            "Text containing <tag> & </tag>",
            "Backslashes \\ and line breaks \n\r\t tab",
            "Literal entity-looking text such as &quot; and &amp; and &lt;tag&gt;",
            "Trucks 🚚 and accents: Café au lait (e\u{0301})",
        ];

        for (i, text) in tricky_cases.iter().enumerate() {
            let key = format!("case_{i}");
            let pairs = vec![(key.clone(), StateValue::Str((*text).to_owned()))];
            let html = inject_state(MINIMAL, &pairs).unwrap();
            let parsed = parse_state_block(&html).unwrap();
            assert_eq!(parsed.len(), 1);
            assert_eq!(parsed[0].0, key);
            assert_eq!(
                parsed[0].1.as_str().unwrap(),
                *text,
                "failed round-trip for: {text:?}"
            );
        }
    }

    #[test]
    fn validation_counts_unicode_scalar_values_matching_js() {
        // Emojis: 1 Unicode scalar value each (4 bytes UTF-8, 2 UTF-16 code units)
        let eighty_emojis = "🚚".repeat(80);
        let eighty_one_emojis = "🚚".repeat(81);
        assert!(validate(&eighty_emojis, "").is_ok());
        assert!(validate(&eighty_one_emojis, "").is_err());

        // Accented Latin characters
        let eighty_accents = "é".repeat(80);
        let eighty_one_accents = "é".repeat(81);
        assert!(validate(&eighty_accents, "").is_ok());
        assert!(validate(&eighty_one_accents, "").is_err());

        // Combining sequence: e + combining acute (2 scalar values)
        let combining = "e\u{0301}";
        assert_eq!(combining.chars().count(), 2);
        let forty_combining = combining.repeat(40); // 80 chars
        let forty_one_combining = combining.repeat(41); // 82 chars
        assert!(validate(&forty_combining, "").is_ok());
        assert!(validate(&forty_one_combining, "").is_err());

        // Notes limit 280
        let two_eighty_emojis = "📦".repeat(280);
        let two_eighty_one_emojis = "📦".repeat(281);
        assert!(validate("Driver", &two_eighty_emojis).is_ok());
        assert!(validate("Driver", &two_eighty_one_emojis).is_err());
    }
}
