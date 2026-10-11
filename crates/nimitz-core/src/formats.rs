//! Format versions and chained migrations (docs/design/migrations.md).
//!
//! Every file nimitz owns and reads back has a format version (`ARTIFACTS`).
//! A change to what a file holds bumps its version and registers one `Step`
//! from the old version to the new; the runner chains them. Local steps run
//! by themselves (the first nimitz run after an update); shared steps (the
//! data root every machine reads) only on `nimitz migrate --shared --yes`.
//! The versions of local artifacts that carry none of their own, and the
//! nimitz that last ran here, are in the manifest `S/formats.json`.
//!
//! A feature nimitz dropped leaves files behind: a `Cleanup` (`CLEANUPS`)
//! removes or archives them, in the same runs and with the same backups and
//! log, but moves no version up (0.7.0: the aides).
use crate::env::Env;
use crate::types::*;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// This nimitz's version: what the manifest records, what a `min_nimitz` is
/// compared with, and the suffix of the backups a migration keeps.
pub const NIMITZ: &str = env!("CARGO_PKG_VERSION");

/// Who reads an artifact: this machine alone, or every machine of the data root.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Scope {
    Shared,
    Local,
}

impl Scope {
    pub fn as_str(&self) -> &'static str {
        match self {
            Scope::Shared => "shared",
            Scope::Local => "local",
        }
    }
}

/// Where an artifact keeps its version.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kept {
    /// a top-level key of each TOML file (absent = 1)
    Toml(&'static str),
    /// a top-level key of each JSON file (absent = 1)
    Json(&'static str),
    /// `S/formats.json` (`artifacts.<name>`; absent = 1): many small files,
    /// or a shape nimitz shares with its tests
    Manifest,
    /// its own mechanism (the role briefs: `rules::briefs` and the rules version)
    Own,
}

/// What a step does to the files of its artifact.
#[derive(Clone, Copy)]
pub enum Apply {
    /// one file, bytes in, bytes out (the output carries the new version)
    Bytes(fn(&[u8]) -> Result<Vec<u8>, String>),
    /// a folder artifact: the files under the folder, rewritten in place
    Folder(fn(&Path) -> Result<(), String>),
    /// nothing to rewrite (the new fields are optional): the manifest records `to`
    Nothing,
}

/// One step of a chain: `artifact` from version `from` to `to`. Pure (no
/// clock, no env) and run only on a file at `from` (the runner checks the
/// version first, so a second run leaves the file alone).
pub struct Step {
    pub artifact: &'static str,
    pub from: u32,
    pub to: u32,
    pub scope: Scope,
    pub summary: &'static str,
    /// the nimitz every reader needs once the step ran (a shared step: each machine first)
    pub min_nimitz: Option<&'static str>,
    pub apply: Apply,
}

/// A file (or a family of files) nimitz owns.
pub struct Artifact {
    pub name: &'static str,
    /// where it lives, as the docs write it (H = NIMITZ_HOME, S = NIMITZ_STATE)
    pub path: &'static str,
    pub scope: Scope,
    /// the version this nimitz writes
    pub current: u32,
    /// the oldest version the chain starts from; an older file is brought to
    /// it by `lift` (the code that has always read it), then the chain runs
    pub oldest: u32,
    pub kept: Kept,
    pub steps: &'static [Step],
    /// the files on this machine (`Kept::Manifest`: the file or the folder, when there)
    pub find: fn(&Env) -> Vec<PathBuf>,
    /// the version a file's content needs, when lower than `current` will do
    /// (a fleet stays at 1: 0.4.x machines still read it)
    pub wanted: Option<fn(&[u8]) -> u32>,
    /// a file older than `oldest`, brought to it (one summary, the code)
    pub lift: Option<(&'static str, fn(&Env) -> Result<(), String>)>,
}

impl Artifact {
    pub fn step_from(&self, v: u32) -> Option<&'static Step> {
        self.steps.iter().find(|s| s.from == v)
    }

    /// The version `bytes` hold (`Kept::Toml` / `Kept::Json`; absent = 1).
    /// None: the file does not parse (its reader reports it; no step runs on it).
    pub fn version_of(&self, bytes: &[u8]) -> Option<u32> {
        match self.kept {
            Kept::Toml(k) => {
                let t: toml::Table = toml::from_str(std::str::from_utf8(bytes).ok()?).ok()?;
                match t.get(k) {
                    None => Some(1),
                    Some(n) => n.as_integer().and_then(|n| u32::try_from(n).ok()),
                }
            }
            Kept::Json(k) => {
                let v = crate::evidence::first_json(bytes)?;
                v.is_object().then_some(())?;
                match v.get(k) {
                    None => Some(1),
                    Some(n) => n.as_u64().and_then(|n| u32::try_from(n).ok()),
                }
            }
            Kept::Manifest | Kept::Own => None,
        }
    }

    /// The version a file at `bytes` should be at.
    pub fn wanted_of(&self, bytes: &[u8]) -> u32 {
        self.wanted.map(|w| w(bytes)).unwrap_or(self.current)
    }
}

// ------------------------------------------------------------------ the artifacts

/// Every file nimitz owns and reads back (docs/design/migrations.md section 2).
pub static ARTIFACTS: &[Artifact] = &[
    Artifact { name: "fleet", path: "H/<fleet>/fleet.toml", scope: Scope::Shared, current: 2, oldest: 1, kept: Kept::Toml("format"), steps: FLEET_STEPS, find: fleet_files, wanted: Some(fleet_wanted), lift: None },
    Artifact { name: "squadron", path: "H/<fleet>/squadrons/<sq>/squadron.toml", scope: Scope::Shared, current: 1, oldest: 1, kept: Kept::Toml("format"), steps: &[], find: squadron_files, wanted: None, lift: None },
    Artifact { name: "legacy", path: "H/<fleet>/legacy.toml", scope: Scope::Shared, current: 1, oldest: 1, kept: Kept::Toml("format"), steps: &[], find: legacy_files, wanted: None, lift: None },
    Artifact { name: "brief", path: "lead CLAUDE.md", scope: Scope::Shared, current: 1, oldest: 1, kept: Kept::Own, steps: &[], find: nothing, wanted: None, lift: None },
    Artifact { name: "ship-lock", path: "<ship folder>/.nimitz-ship.lock", scope: Scope::Shared, current: 1, oldest: 1, kept: Kept::Json("format"), steps: &[], find: ship_locks, wanted: None, lift: None },
    Artifact { name: "registry", path: "S/registry.json", scope: Scope::Local, current: 3, oldest: 2, kept: Kept::Json("version"), steps: REGISTRY_STEPS, find: registry_file, wanted: None, lift: Some(("the 0.3.x registry, read and saved as version 2 (the old file kept as registry.json.v1)", crate::registry::lift_v1)) },
    Artifact { name: "active", path: "S/active.json", scope: Scope::Local, current: 1, oldest: 1, kept: Kept::Json("version"), steps: &[], find: active_file, wanted: None, lift: None },
    Artifact { name: "rules-delivered", path: "S/rules-delivered.json", scope: Scope::Local, current: 1, oldest: 1, kept: Kept::Manifest, steps: &[], find: rules_delivered, wanted: None, lift: None },
    Artifact { name: "events", path: "S/events.jsonl", scope: Scope::Local, current: 1, oldest: 1, kept: Kept::Manifest, steps: &[], find: events_file, wanted: None, lift: None },
    Artifact { name: "restore", path: "S/restore.json", scope: Scope::Local, current: 1, oldest: 1, kept: Kept::Manifest, steps: &[], find: restore_file, wanted: None, lift: None },
    Artifact { name: "pending", path: "S/pending/*.json", scope: Scope::Local, current: 1, oldest: 1, kept: Kept::Manifest, steps: &[], find: pending_dir, wanted: None, lift: None },
    Artifact { name: "inbox-records", path: "S/inbox/*.json", scope: Scope::Local, current: 1, oldest: 1, kept: Kept::Manifest, steps: &[], find: inbox_dir, wanted: None, lift: None },
    Artifact { name: "needs", path: "S/needs/*.json", scope: Scope::Local, current: 2, oldest: 1, kept: Kept::Manifest, steps: NEEDS_STEPS, find: needs_dir, wanted: None, lift: None },
    Artifact { name: "own-pins", path: "S/pins.json", scope: Scope::Local, current: 1, oldest: 1, kept: Kept::Manifest, steps: &[], find: pins_file, wanted: None, lift: None },
    Artifact { name: "admiralty-ui", path: "S/admiralty-ui.json", scope: Scope::Local, current: 1, oldest: 1, kept: Kept::Json("format"), steps: &[], find: admiralty_ui, wanted: None, lift: None },
];

/// The artifact called `name`.
pub fn artifact(name: &str) -> Option<&'static Artifact> {
    ARTIFACTS.iter().find(|a| a.name == name)
}

/// The version this nimitz writes of `name` (1 for a name it does not know).
pub fn current(name: &str) -> u32 {
    artifact(name).map(|a| a.current).unwrap_or(1)
}

static FLEET_STEPS: &[Step] = &[Step {
    artifact: "fleet",
    from: 1,
    to: 2,
    scope: Scope::Shared,
    summary: "format = 2 and min_nimitz = \"0.5.0\" (0.5 and 0.6: a fleet that held [[aide]]; 0.7 wants no fleet at 2, so it never runs)",
    min_nimitz: Some("0.5.0"),
    apply: Apply::Bytes(fleet_1_2),
}];

static REGISTRY_STEPS: &[Step] = &[
    Step {
        artifact: "registry",
        from: 2,
        to: 3,
        scope: Scope::Local,
        summary: "version 3 and min_nimitz \"0.5.0\" (role aide; records of an unknown role kept); no record changes",
        min_nimitz: Some("0.5.0"),
        apply: Apply::Bytes(registry_2_3),
    },
];

static NEEDS_STEPS: &[Step] = &[Step {
    artifact: "needs",
    from: 1,
    to: 2,
    scope: Scope::Local,
    summary: "nothing to rewrite (a dead letter's optional `item`); the manifest records 2",
    min_nimitz: None,
    apply: Apply::Nothing,
}];

/// `fleet` 1 -> 2: `format = 2` and `min_nimitz = "0.5.0"` above the first
/// table; the rest of the file, comments included, as it was.
pub fn fleet_1_2(bytes: &[u8]) -> Result<Vec<u8>, String> {
    let text = std::str::from_utf8(bytes).map_err(|e| e.to_string())?;
    toml::from_str::<toml::Table>(text).map_err(|e| format!("does not parse: {}", e.message().trim()))?;
    Ok(stamp_toml(text, 2, Some("0.5.0")).into_bytes())
}

/// `text` with `format = N` (and `min_nimitz`) as its first keys: any of the
/// two above the first table goes, the new ones go on top.
pub fn stamp_toml(text: &str, format: u32, min_nimitz: Option<&str>) -> String {
    let mut out = format!("format = {format}\n");
    if let Some(m) = min_nimitz {
        out.push_str(&format!("min_nimitz = {}\n", toml::Value::String(m.into())));
    }
    let mut in_table = false;
    for line in text.split_inclusive('\n') {
        let t = line.trim_start();
        in_table |= t.starts_with('[');
        let key = t.split('=').next().unwrap_or("").trim();
        if !in_table && t.contains('=') && (key == "format" || key == "min_nimitz") {
            continue;
        }
        out.push_str(line);
    }
    out
}

/// `fleet`: 1. Format 2 was a fleet with `[[aide]]` (0.5.0 to 0.6.x); the
/// aides are gone, and their cleanup (`drop_aides`) takes a fleet back to 1.
fn fleet_wanted(_bytes: &[u8]) -> u32 {
    1
}

/// A registry with `version: to` and `min_nimitz` right after it (on top
/// when it had no version); the other keys as they were, through `keep`.
fn restamp_registry(bytes: &[u8], to: u32, min_nimitz: &str, keep: impl Fn(&str, Value) -> Value) -> Result<Vec<u8>, String> {
    let v: Value = serde_json::from_slice(bytes).map_err(|e| format!("does not parse: {e}"))?;
    let Value::Object(old) = v else { return Err("not a JSON object".into()) };
    let mut m = serde_json::Map::new();
    if !old.contains_key("version") {
        m.insert("version".into(), Value::from(to));
        m.insert("min_nimitz".into(), Value::from(min_nimitz));
    }
    for (k, v) in old {
        match k.as_str() {
            "version" => {
                m.insert(k, Value::from(to));
                m.insert("min_nimitz".into(), Value::from(min_nimitz));
            }
            "min_nimitz" => {}
            _ => {
                let v = keep(&k, v);
                m.insert(k, v);
            }
        }
    }
    serde_json::to_vec_pretty(&Value::Object(m)).map_err(|e| e.to_string())
}

/// `registry` 2 -> 3: `version: 3` and `min_nimitz: "0.5.0"` (right after
/// `version`); every record as it was.
pub fn registry_2_3(bytes: &[u8]) -> Result<Vec<u8>, String> {
    restamp_registry(bytes, 3, "0.5.0", |_, v| v)
}

/// `registry` (a cleanup, no version moves): every record of role `aide`
/// dropped, from `identities` and `retired` (0.7.0 has no such role); every
/// other record, and every other key, as it was.
pub fn drop_aide_records(bytes: &[u8]) -> Result<Vec<u8>, String> {
    let v: Value = serde_json::from_slice(bytes).map_err(|e| format!("does not parse: {e}"))?;
    let Value::Object(mut m) = v else { return Err("not a JSON object".into()) };
    for k in ["identities", "retired"] {
        if let Some(Value::Object(recs)) = m.get_mut(k) {
            recs.retain(|_, r| !is_aide_record(r));
        }
    }
    serde_json::to_vec_pretty(&Value::Object(m)).map_err(|e| e.to_string())
}

fn is_aide_record(r: &Value) -> bool {
    r.get("role").and_then(Value::as_str) == Some("aide")
}

// ------------------------------------------------------------------ cleanups

/// What a dropped feature left on disk, removed or archived (never a version
/// up: the file goes back to, or stays at, what every nimitz reads). Local
/// cleanups run by themselves with the local steps, shared ones on `nimitz
/// migrate --shared --yes`; each file is backed up first (as a step's) and
/// logged.
pub struct Cleanup {
    /// what it does, as a plan line shows it (`fleet: aides dropped`)
    pub label: &'static str,
    pub artifact: &'static str,
    pub scope: Scope,
    pub summary: &'static str,
    /// the files (or folders) it is due on now
    pub find: fn(&Env) -> Vec<PathBuf>,
    pub apply: CleanupApply,
}

#[derive(Clone, Copy)]
pub enum CleanupApply {
    /// one file, bytes in, bytes out (atomic write over it)
    Bytes(fn(&[u8]) -> Result<Vec<u8>, String>),
    /// moved whole into `S/archive/<folder>/` (never deleted)
    Archive(&'static str),
}

impl Cleanup {
    /// Due on `file`: from its version to the version of what the cleanup
    /// makes of it (a folder, or a file it cannot read: 0).
    fn due(&self, file: &Path) -> Due {
        let a = artifact(self.artifact);
        let bytes = std::fs::read(file).ok();
        let version = |b: &[u8]| a.and_then(|a| a.version_of(b)).unwrap_or(0);
        let from = bytes.as_deref().map(version).unwrap_or(0);
        let to = match (self.apply, bytes.as_deref()) {
            (CleanupApply::Bytes(apply), Some(b)) => apply(b).map(|o| version(&o)).unwrap_or(from),
            _ => from,
        };
        Due { artifact: self.artifact, scope: self.scope, file: file.to_path_buf(), from, to, summary: self.summary, min_nimitz: None, silent: false, cleanup: Some(self.label) }
    }
}

/// 0.7.0: the aides are gone.
pub static CLEANUPS: &[Cleanup] = &[
    Cleanup {
        label: "aide records dropped",
        artifact: "registry",
        scope: Scope::Local,
        summary: "the records of the aides (role aide, gone in 0.7.0) dropped from the identities and the retired; the rest as it was",
        find: registry_with_aides,
        apply: CleanupApply::Bytes(drop_aide_records),
    },
    Cleanup {
        label: "aides dropped",
        artifact: "fleet",
        scope: Scope::Shared,
        summary: "the [[aide]] tables dropped (aides are gone in 0.7.0), and the format 2 / min_nimitz stamp they needed: every nimitz reads it again (an aide job still running: `nimitz check` names it aide.retired, with its stop)",
        find: fleets_with_aides,
        apply: CleanupApply::Bytes(drop_aides),
    },
    Cleanup {
        label: "aide state archived",
        artifact: "aides",
        scope: Scope::Local,
        summary: "the 0.6 aides' queues, snapshots, doorbell files and needs-you notices, moved to S/archive/aides-0.6/ (aides are gone in 0.7.0; nothing deleted)",
        find: aide_state,
        apply: CleanupApply::Archive("aides-0.6"),
    },
];

/// The fleet.toml files that still hold `[[aide]]`, or the format 2 stamp
/// only an aide asked for (a file too new for this nimitz is left alone).
fn fleets_with_aides(env: &Env) -> Vec<PathBuf> {
    fleet_files(env)
        .into_iter()
        .filter(|f| {
            let Ok(raw) = std::fs::read_to_string(f) else { return false };
            if guard_toml("fleet", f, &raw).is_err() {
                return false;
            }
            let Ok(t) = toml::from_str::<toml::Table>(&raw) else { return false };
            t.contains_key("aide") || t.get("format").and_then(toml::Value::as_integer) == Some(2)
        })
        .collect()
}

/// The registry, when it holds a record of role `aide` (and this nimitz may read it).
fn registry_with_aides(env: &Env) -> Vec<PathBuf> {
    registry_file(env)
        .into_iter()
        .filter(|f| {
            let Ok(bytes) = std::fs::read(f) else { return false };
            let Some(v) = crate::evidence::first_json(&bytes) else { return false };
            guard_json("registry", f, &v, "version").is_ok() && ["identities", "retired"].iter().any(|k| v.get(k).and_then(Value::as_object).is_some_and(|m| m.values().any(is_aide_record)))
        })
        .collect()
}

/// What a 0.5 or 0.6 nimitz kept for its aides in this machine's state:
/// `S/queues`, `S/aides` (snapshots), the doorbell's `ring.lock` and
/// `ring.stamp`, and the needs-you notices of an aide or a dead letter.
fn aide_state(env: &Env) -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = ["queues", "aides", "ring.lock", "ring.stamp"].iter().map(|n| env.state.join(n)).filter(|p| p.exists()).collect();
    let mut notices: Vec<PathBuf> = std::fs::read_dir(crate::notify::dir(env))
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.file_name().is_some_and(|n| {
            let n = n.to_string_lossy();
            n.ends_with(".json") && (n.contains("#dead-") || n.contains("%2F@"))
        }))
        .collect();
    notices.sort();
    out.extend(notices);
    out
}

/// `fleet`: every `[[aide]]` table (with its `[aide.*]` sub-tables) taken
/// out of the text; then `format = 2` and a `min_nimitz` below 0.7.0 (the
/// stamp the aides needed) go from the top. Comments and the rest of the
/// file stay as they were, and the result must parse to the same tables,
/// less those keys, or nothing is written.
pub fn drop_aides(bytes: &[u8]) -> Result<Vec<u8>, String> {
    let text = std::str::from_utf8(bytes).map_err(|e| e.to_string())?;
    let mut want: toml::Table = toml::from_str(text).map_err(|e| format!("does not parse: {}", e.message().trim()))?;
    want.remove("aide");
    let stamp_goes = |k: &str, v: Option<&toml::Value>| match k {
        "format" => v.and_then(toml::Value::as_integer) == Some(2),
        "min_nimitz" => v.and_then(toml::Value::as_str).is_some_and(|m| crate::compat::older(m, "0.7.0")),
        _ => false,
    };
    for k in ["format", "min_nimitz"] {
        if stamp_goes(k, want.get(k)) {
            want.remove(k);
        }
    }
    let mut out = String::new();
    let (mut in_table, mut skip) = (false, false);
    // comments and blank lines inside an aide's tables: held, then kept
    // when a table that is not an aide's follows (they belong to it)
    let mut held = String::new();
    for line in text.split_inclusive('\n') {
        let t = line.trim_start();
        if t.starts_with('[') {
            in_table = true;
            let name = t.trim_start_matches('[').split(']').next().unwrap_or("").trim();
            let head = name.split('.').next().unwrap_or("").trim().trim_matches('"');
            skip = head == "aide";
            if skip {
                held.clear();
            } else {
                out.push_str(&std::mem::take(&mut held));
            }
        }
        if skip {
            if t.is_empty() || t.starts_with('#') {
                held.push_str(line);
            } else {
                held.clear();
            }
            continue;
        }
        if !in_table && t.contains('=') {
            let (k, v) = t.split_once('=').unwrap_or_default();
            let k = k.trim();
            if (k == "format" || k == "min_nimitz") && stamp_goes(k, toml::from_str::<toml::Table>(&format!("v = {}", v.trim())).ok().as_ref().and_then(|t| t.get("v"))) {
                continue;
            }
        }
        out.push_str(line);
    }
    let got: toml::Table = toml::from_str(&out).map_err(|e| format!("the file without its aides does not parse ({}): left as it was", e.message().trim()))?;
    if got != want {
        return Err("taking out the [[aide]] tables would change more than them: left as it was (remove them by hand)".into());
    }
    Ok(out.into_bytes())
}

/// One cleanup on one file or folder: its backup (a file's bytes; an
/// archived thing is its own copy), then the change, then the log line.
fn run_cleanup(env: &Env, c: &Cleanup, f: &Path, nimitz: &str, ran: &mut Ran) {
    let d = c.due(f);
    let res = match c.apply {
        CleanupApply::Bytes(apply) => cleanup_bytes(env, c, apply, f, nimitz, ran),
        CleanupApply::Archive(folder) => archive(env, f, folder).map(|_| true),
    };
    match res {
        Ok(true) => {
            let backup = backup_path(f, nimitz);
            log(env, nimitz, &d, None, backup.exists().then_some(backup.as_path()));
            ran.done.push(d);
        }
        Ok(false) => {}
        Err(e) => fail(env, nimitz, ran, d, e),
    }
}

/// A byte cleanup on `f`: read, apply, write under the lock while the file
/// is still what was read; a file another writer changed meanwhile (a hook
/// on the registry) is read again, as `run_file` does. The backup is the
/// first read (as a step's: the file before this nimitz touched it), so a
/// write that came between is not in it. Ok(false): nothing to do (too new
/// for this nimitz: said in `ran.too_new`).
fn cleanup_bytes(env: &Env, c: &Cleanup, apply: fn(&[u8]) -> Result<Vec<u8>, String>, f: &Path, nimitz: &str, ran: &mut Ran) -> Result<bool, String> {
    for _ in 0..64 {
        let bytes = std::fs::read(f).map_err(|e| e.to_string())?;
        if let Some(a) = artifact(c.artifact) {
            if let Err(t) = a.guard_file(f, &bytes) {
                ran.too_new.push(t);
                return Ok(false);
            }
        }
        let out = apply(&bytes)?;
        if a_version_moved(c, &bytes, &out) {
            return Err("the cleanup moved the file's version: left as it was".into());
        }
        backup_once(f, Some(&bytes), nimitz, ran)?;
        let write = || -> Result<bool, String> {
            if std::fs::read(f).map_err(|e| e.to_string())? != bytes {
                return Ok(false);
            }
            write_bytes(f, &out).map(|_| true).map_err(|e| e.to_string())
        };
        let wrote = match c.scope {
            Scope::Local => crate::registry::with_lock(env, None, write).map_err(|e| e.to_string())?,
            Scope::Shared => write(),
        }?;
        if wrote {
            return Ok(true);
        }
    }
    Err("the file kept changing under the cleanup: left for the next run".into())
}

/// A cleanup may take a file down to a version every nimitz reads (the
/// fleet: 2 to 1), never up: that is a step's.
fn a_version_moved(c: &Cleanup, before: &[u8], after: &[u8]) -> bool {
    let Some(a) = artifact(c.artifact) else { return false };
    a.version_of(after).unwrap_or(0) > a.version_of(before).unwrap_or(0)
}

/// Move `p` (under the state folder) to `S/archive/<folder>/<its path under
/// S>`; a name already there gets `.2`, `.3`, ... (nothing is replaced).
fn archive(env: &Env, p: &Path, folder: &str) -> Result<PathBuf, String> {
    let rel = p.strip_prefix(&env.state).map_err(|_| format!("{} is not under {}", p.display(), env.state.display()))?;
    let base = env.state.join("archive").join(folder).join(rel);
    let mut to = base.clone();
    let mut n = 2;
    while to.exists() {
        let mut s = base.as_os_str().to_os_string();
        s.push(format!(".{n}"));
        to = PathBuf::from(s);
        n += 1;
    }
    std::fs::create_dir_all(to.parent().unwrap_or(&env.state)).map_err(|e| e.to_string())?;
    if let Err(e) = std::fs::rename(p, &to) {
        // a file held open (Windows: a scanner, a leftover process) stops a
        // rename. A file: copied, then the original removed (all or
        // nothing: a copy whose original stays is removed again). A folder
        // is never copied and removed piecemeal (a file that cannot go would
        // leave the rest half moved): it waits, whole, for the next run
        if p.is_dir() {
            return Err(format!("{} -> {}: {e} (left as it was; the next nimitz command tries again)", p.display(), to.display()));
        }
        if let Err(e2) = std::fs::copy(p, &to).and_then(|_| std::fs::remove_file(p)) {
            if p.exists() {
                let _ = std::fs::remove_file(&to);
            }
            return Err(format!("{} -> {}: {e} (and a copy: {e2}; left as it was)", p.display(), to.display()));
        }
    }
    Ok(to)
}

// ------------------------------------------------------------------ where they live

fn nothing(_: &Env) -> Vec<PathBuf> {
    vec![]
}

fn existing(p: PathBuf) -> Vec<PathBuf> {
    if p.exists() { vec![p] } else { vec![] }
}

fn subdirs(dir: &Path) -> Vec<PathBuf> {
    let mut v: Vec<PathBuf> = std::fs::read_dir(dir).into_iter().flatten().flatten().map(|e| e.path()).filter(|p| p.is_dir()).collect();
    v.sort();
    v
}

/// Fleet folders under NIMITZ_HOME (those with a fleet.toml).
fn fleet_dirs(env: &Env) -> Vec<PathBuf> {
    subdirs(&env.home).into_iter().filter(|d| d.join("fleet.toml").is_file()).collect()
}

fn fleet_files(env: &Env) -> Vec<PathBuf> {
    fleet_dirs(env).into_iter().map(|d| d.join("fleet.toml")).collect()
}

fn squadron_files(env: &Env) -> Vec<PathBuf> {
    fleet_dirs(env).iter().flat_map(|d| subdirs(&d.join("squadrons"))).map(|d| d.join("squadron.toml")).filter(|p| p.is_file()).collect()
}

fn legacy_files(env: &Env) -> Vec<PathBuf> {
    fleet_dirs(env).into_iter().map(|d| d.join("legacy.toml")).filter(|p| p.is_file()).collect()
}

/// The locks of the managed ship folders (`H/<fleet>/ships/<sq>/<ship>/`).
fn ship_locks(env: &Env) -> Vec<PathBuf> {
    fleet_dirs(env).iter().flat_map(|d| subdirs(&d.join("ships"))).flat_map(|d| subdirs(&d)).map(|d| d.join(".nimitz-ship.lock")).filter(|p| p.is_file()).collect()
}

fn registry_file(env: &Env) -> Vec<PathBuf> {
    existing(env.registry_path())
}

fn active_file(env: &Env) -> Vec<PathBuf> {
    existing(crate::active::path(env))
}

fn rules_delivered(env: &Env) -> Vec<PathBuf> {
    existing(env.state.join("rules-delivered.json"))
}

fn events_file(env: &Env) -> Vec<PathBuf> {
    existing(env.events_path())
}

fn restore_file(env: &Env) -> Vec<PathBuf> {
    existing(env.state.join("restore.json"))
}

/// A folder, when it holds a file.
fn nonempty(dir: PathBuf) -> Vec<PathBuf> {
    if std::fs::read_dir(&dir).into_iter().flatten().flatten().next().is_some() { vec![dir] } else { vec![] }
}

fn pending_dir(env: &Env) -> Vec<PathBuf> {
    nonempty(env.state.join("pending"))
}

fn inbox_dir(env: &Env) -> Vec<PathBuf> {
    nonempty(env.state.join("inbox"))
}

fn needs_dir(env: &Env) -> Vec<PathBuf> {
    nonempty(crate::notify::dir(env))
}

fn pins_file(env: &Env) -> Vec<PathBuf> {
    existing(env.state.join("pins.json"))
}

fn admiralty_ui(env: &Env) -> Vec<PathBuf> {
    existing(env.state.join("admiralty-ui.json"))
}

// ------------------------------------------------------------------ the manifest

/// `S/formats.json`: the nimitz that last ran here (its local steps all
/// done), and the versions of the local artifacts that keep none of their own.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Manifest {
    #[serde(default)]
    pub nimitz: String,
    #[serde(default)]
    pub artifacts: BTreeMap<String, u32>,
    /// what a newer nimitz wrote here: kept as it was
    #[serde(flatten)]
    pub extra: BTreeMap<String, Value>,
}

pub fn manifest_path(env: &Env) -> PathBuf {
    env.state.join("formats.json")
}

/// The migration log: one JSON line per step (time, artifact, file, from, to, ok or error).
pub fn log_path(env: &Env) -> PathBuf {
    env.state.join("migrations.log")
}

impl Manifest {
    pub fn load(env: &Env) -> Option<Manifest> {
        serde_json::from_slice(&std::fs::read(manifest_path(env)).ok()?).ok()
    }

    /// The manifest's version of `name` (a missing entry is 1).
    pub fn version(&self, name: &str) -> u32 {
        self.artifacts.get(name).copied().unwrap_or(1)
    }

    /// Atomic save; nothing when the state folder is not there (a machine
    /// before `nimitz init`: nothing to migrate, nothing to record).
    pub fn save(&self, env: &Env) -> Result<(), String> {
        if !env.state.is_dir() {
            return Ok(());
        }
        let text = serde_json::to_string_pretty(self).map_err(|e| e.to_string())?;
        crate::config::atomic_write(&manifest_path(env), &text).map_err(|e| format!("{}: {e}", manifest_path(env).display()))
    }
}

/// Did this nimitz already migrate here? One small read: the manifest's
/// `nimitz` is this version. A machine with no state folder has nothing to
/// migrate either.
pub fn up_to_date(env: &Env) -> bool {
    if !env.state.is_dir() {
        return true;
    }
    #[derive(Deserialize)]
    struct Head {
        #[serde(default)]
        nimitz: String,
    }
    std::fs::read(manifest_path(env)).ok().and_then(|b| serde_json::from_slice::<Head>(&b).ok()).is_some_and(|h| h.nimitz == NIMITZ)
}

// ------------------------------------------------------------------ the guard

/// A file this nimitz must not read: it needs a newer nimitz (`min_nimitz`),
/// or holds a format above the one this nimitz knows (or one that is no
/// version at all: below 1).
#[derive(Clone, Debug, PartialEq)]
pub struct TooNew {
    pub file: PathBuf,
    pub artifact: &'static str,
    /// the file's `min_nimitz`, when it is above this nimitz
    pub needs: Option<String>,
    /// the file's format as written (a negative one is kept as it is)
    pub format: i64,
    pub known: u32,
}

impl TooNew {
    /// What the file asks for, without the file: `needs nimitz >= X (this machine runs Y): update nimitz here`.
    pub fn reason(&self) -> String {
        match &self.needs {
            Some(x) => format!("needs nimitz >= {x} (this machine runs {NIMITZ}): update nimitz here"),
            None if self.format < 1 => format!("carries format {}, which is no format version (nimitz {NIMITZ} reads 1 to {}): fix the file by hand, or update nimitz here", self.format, self.known),
            None => format!("is format {}, newer than this nimitz reads (it runs {NIMITZ} and reads up to format {}): update nimitz here", self.format, self.known),
        }
    }

    pub fn issue(&self, id: Option<Id>) -> Issue {
        Issue {
            code: "config.too_new".into(),
            severity: Severity::Error,
            message: format!("{self}; it is never read further nor rewritten by this nimitz"),
            fix: Some(match &self.needs {
                Some(x) => format!("update nimitz on this machine to {x} or later"),
                None => "update nimitz on this machine".into(),
            }),
            id,
        }
    }
}

impl std::fmt::Display for TooNew {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} {}", self.file.display(), self.reason())
    }
}

/// A file's stamp, read leniently: its version key (as written) and its
/// `min_nimitz`. Nothing else of the file is read.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Stamp {
    pub format: Option<i64>,
    pub min_nimitz: Option<String>,
}

/// The stamp of a TOML file: from the parsed table; when the file does not
/// parse (a syntax a newer nimitz may write), from a scan of its lines above
/// the first table (`format = N`, `min_nimitz = "X"`), so the reader is still
/// told which nimitz it needs.
pub fn toml_stamp(raw: &str, key: &str) -> Stamp {
    if let Ok(t) = toml::from_str::<toml::Table>(raw) {
        return Stamp { format: t.get(key).and_then(toml::Value::as_integer), min_nimitz: t.get("min_nimitz").and_then(toml::Value::as_str).map(str::to_string) };
    }
    let mut st = Stamp::default();
    for line in raw.lines() {
        let t = line.trim();
        if t.starts_with('[') {
            break;
        }
        let Some((k, v)) = t.split_once('=') else { continue };
        let v = v.split('#').next().unwrap_or("").trim();
        match k.trim() {
            k2 if k2 == key => st.format = v.parse::<i64>().ok(),
            "min_nimitz" => st.min_nimitz = v.strip_prefix('"').and_then(|x| x.strip_suffix('"')).map(str::to_string),
            _ => {}
        }
    }
    st
}

/// The stamp of a JSON value (a number too big for i64 reads as i64::MAX).
pub fn json_stamp(v: &Value, key: &str) -> Stamp {
    let format = v.get(key).and_then(|n| n.as_i64().or_else(|| n.as_u64().map(|_| i64::MAX)));
    Stamp { format, min_nimitz: v.get("min_nimitz").and_then(Value::as_str).map(str::to_string) }
}

/// The guard on a stamp: a `min_nimitz` above this nimitz, a format above
/// the artifact's current one, or a format below 1, is refused.
pub fn guard(artifact: &str, file: &Path, stamp: &Stamp) -> Result<(), TooNew> {
    guard_known(artifact, current(artifact), file, stamp)
}

/// `guard` with the version this nimitz knows given (`known`).
fn guard_known(artifact: &str, known: u32, file: &Path, stamp: &Stamp) -> Result<(), TooNew> {
    let format = stamp.format.unwrap_or(1);
    let needs = stamp.min_nimitz.as_deref().filter(|m| crate::compat::older(NIMITZ, m)).map(str::to_string);
    if needs.is_some() || format > i64::from(known) || format < 1 {
        let artifact = artifact_name(artifact);
        return Err(TooNew { file: file.to_path_buf(), artifact, needs, format, known });
    }
    Ok(())
}

fn artifact_name(name: &str) -> &'static str {
    artifact(name).map(|a| a.name).unwrap_or("unknown")
}

/// The guard on a shared TOML, before its strict parse: only `format` and
/// `min_nimitz` are read (`toml_stamp`; lines above the first table when the
/// file does not parse).
pub fn guard_toml(artifact: &str, file: &Path, raw: &str) -> Result<(), TooNew> {
    guard(artifact, file, &toml_stamp(raw, "format"))
}

/// The guard on a JSON file whose version lives in `key` (`version` for the
/// registry and the active set, `format` for the rest).
pub fn guard_json(artifact: &str, file: &Path, v: &Value, key: &str) -> Result<(), TooNew> {
    guard(artifact, file, &json_stamp(v, key))
}

/// The guard on the bytes of a JSON file (`guard_json`); a file that does not
/// parse passes (its reader says why).
pub fn guard_json_bytes(artifact: &str, file: &Path, bytes: &[u8], key: &str) -> Result<(), TooNew> {
    match crate::evidence::first_json(bytes) {
        Some(v) => guard_json(artifact, file, &v, key),
        None => Ok(()),
    }
}

impl Artifact {
    /// The guard on one file of this artifact (its stamp against `current`).
    pub fn guard_file(&self, f: &Path, bytes: &[u8]) -> Result<(), TooNew> {
        let stamp = match self.kept {
            Kept::Toml(k) => {
                let Ok(raw) = std::str::from_utf8(bytes) else { return Ok(()) };
                toml_stamp(raw, k)
            }
            Kept::Json(k) => {
                let Some(v) = crate::evidence::first_json(bytes) else { return Ok(()) };
                json_stamp(&v, k)
            }
            Kept::Manifest | Kept::Own => return Ok(()),
        };
        guard_known(self.name, self.current, f, &stamp)
    }
}

/// Every file of this machine's state, and of the data root, that this
/// nimitz must not read.
pub fn too_new(env: &Env) -> Vec<TooNew> {
    let mut out = vec![];
    for a in ARTIFACTS {
        for f in (a.find)(env) {
            let Ok(bytes) = std::fs::read(&f) else { continue };
            if let Err(t) = a.guard_file(&f, &bytes) {
                out.push(t);
            }
        }
    }
    if let Some(m) = Manifest::load(env) {
        for a in ARTIFACTS.iter().filter(|a| a.kept == Kept::Manifest) {
            if m.version(a.name) > a.current {
                out.push(manifest_too_new(env, a, &m));
            }
        }
    }
    out
}

/// A manifest entry above what this nimitz knows of `a`.
fn manifest_too_new(env: &Env, a: &Artifact, m: &Manifest) -> TooNew {
    TooNew { file: manifest_path(env), artifact: a.name, needs: None, format: i64::from(m.version(a.name)), known: a.current }
}

/// The issues of this machine's own files (`check`): a local file too new
/// for this nimitz (`config.too_new`; the shared ones are their fleet's
/// `config.too_new`), and a migration step that failed (`format.failed`).
pub fn issues(env: &Env) -> Vec<Issue> {
    let mut out: Vec<Issue> = too_new(env).into_iter().filter(|t| artifact(t.artifact).is_none_or(|a| a.scope == Scope::Local)).map(|t| t.issue(None)).collect();
    out.extend(failed(env).into_iter().map(|f| f.issue()));
    out
}

// ------------------------------------------------------------------ machines

/// The first nimitz that records itself in the fleets it runs in (and has
/// the guard): a machine with no record runs 0.4.x, or has not run since.
pub const RECORDS_SINCE: &str = "0.5.0";

/// `H/<fleet>/machines/<label>.json`: the nimitz a machine of the fleet last ran.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct MachineRecord {
    pub nimitz: String,
    /// when this machine first ran that nimitz here (epoch seconds)
    pub seen: f64,
}

pub fn machine_record_path(fleet_dir: &Path, label: &str) -> PathBuf {
    fleet_dir.join("machines").join(format!("{label}.json"))
}

pub fn machine_record(fleet_dir: &Path, label: &str) -> Option<MachineRecord> {
    serde_json::from_slice(&std::fs::read(machine_record_path(fleet_dir, label)).ok()?).ok()
}

/// Record this nimitz in every fleet whose `[machines]` lists this machine
/// (its hostname): written only when the version changed, so a synced data
/// root sees one write per machine per update. The fleet.toml is read
/// leniently (its `machines` alone): a fleet this nimitz cannot load still
/// learns which nimitz runs here.
pub fn record_machine(env: &Env) {
    for dir in fleet_dirs(env) {
        let Some(t) = std::fs::read_to_string(dir.join("fleet.toml")).ok().and_then(|raw| toml::from_str::<toml::Table>(&raw).ok()) else { continue };
        let Some(machines) = t.get("machines").and_then(toml::Value::as_table) else { continue };
        let here = machines.iter().find(|(_, m)| m.get("hostname").and_then(toml::Value::as_str).is_some_and(|h| env.matches_hostname(h)));
        let Some((label, _)) = here.filter(|(l, _)| crate::config::valid_label(l)) else { continue };
        if machine_record(&dir, label).is_some_and(|r| r.nimitz == NIMITZ) {
            continue;
        }
        let rec = MachineRecord { nimitz: NIMITZ.into(), seen: crate::evidence::now() };
        let _ = serde_json::to_vec_pretty(&rec).map(|b| write_bytes(&machine_record_path(&dir, label), &b));
    }
}

/// The machines of `f` that cannot be trusted to read it (doctor): a listed
/// machine with no record (0.4.x, or no 0.5 run since), or one older than
/// the fleet's `min_nimitz`.
pub fn machine_issues(f: &FleetOnDisk) -> Vec<Issue> {
    let need = f.config.min_nimitz.clone().filter(|m| !crate::compat::older(m, RECORDS_SINCE)).unwrap_or_else(|| RECORDS_SINCE.into());
    let then = "update it";
    f.config
        .machines
        .keys()
        .filter_map(|label| {
            let seen = match machine_record(&f.dir, label) {
                Some(r) if !crate::compat::older(&r.nimitz, &need) => return None,
                Some(r) => format!(" (it last ran {})", r.nimitz),
                None => String::new(),
            };
            Some(Issue {
                code: "format.machine".into(),
                severity: Severity::Warn,
                message: format!("machine {label}: no nimitz >= {need} seen{seen}; {then}"),
                fix: Some(format!("update nimitz on {label}, then run any nimitz command there")),
                id: Some(Id::admiral(&f.config.fleet)),
            })
        })
        .collect()
}

// ------------------------------------------------------------------ the runner

/// One step due on one file (or, for a manifest artifact, on its file or folder).
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Due {
    pub artifact: &'static str,
    pub scope: Scope,
    pub file: PathBuf,
    pub from: u32,
    pub to: u32,
    pub summary: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub min_nimitz: Option<&'static str>,
    /// a manifest artifact with nothing on disk: only the manifest changes
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub silent: bool,
    /// a cleanup (`CLEANUPS`), not a step: what it does to the file
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cleanup: Option<&'static str>,
}

impl Due {
    /// `registry 3->4`, or a cleanup's `fleet: aides dropped`.
    pub fn label(&self) -> String {
        match self.cleanup {
            Some(c) => format!("{}: {c}", self.artifact),
            None => format!("{} {}->{}", self.artifact, self.from, self.to),
        }
    }
}

/// Every step due here, in chain order per file (`scope`: one scope, or both).
pub fn due(env: &Env, arts: &[Artifact], scope: Option<Scope>) -> Vec<Due> {
    let man = Manifest::load(env).unwrap_or_default();
    let mut out = vec![];
    for a in arts.iter().filter(|a| scope.is_none_or(|s| a.scope == s)) {
        match a.kept {
            Kept::Toml(_) | Kept::Json(_) => {
                for f in (a.find)(env) {
                    let Ok(bytes) = std::fs::read(&f) else { continue };
                    let Some(v) = a.version_of(&bytes) else { continue };
                    out.extend(chain(a, &f, v, a.wanted_of(&bytes), false));
                }
            }
            Kept::Manifest => {
                let found = (a.find)(env);
                let file = found.first().cloned().unwrap_or_else(|| manifest_path(env));
                out.extend(chain(a, &file, man.version(a.name), a.current, found.is_empty()));
            }
            Kept::Own => {}
        }
    }
    out.extend(CLEANUPS.iter().filter(|c| scope.is_none_or(|s| c.scope == s)).flat_map(|c| (c.find)(env).into_iter().map(|f| c.due(&f))));
    out
}

/// The steps from `v` to `to` of `a` on `file` (a lift first, below `oldest`).
fn chain(a: &Artifact, file: &Path, mut v: u32, to: u32, silent: bool) -> Vec<Due> {
    let mut out = vec![];
    if v < a.oldest && v < to {
        if let Some((summary, _)) = a.lift {
            out.push(Due { artifact: a.name, scope: a.scope, file: file.to_path_buf(), from: v, to: a.oldest, summary, min_nimitz: None, silent, cleanup: None });
        }
        v = a.oldest;
    }
    while v < to {
        let Some(s) = a.step_from(v) else { break };
        out.push(Due { artifact: a.name, scope: a.scope, file: file.to_path_buf(), from: s.from, to: s.to, summary: s.summary, min_nimitz: s.min_nimitz, silent, cleanup: None });
        v = s.to;
    }
    out
}

/// What a run did.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Ran {
    pub done: Vec<Due>,
    pub failed: Vec<(Due, String)>,
    pub backups: Vec<PathBuf>,
    /// files left alone: a newer nimitz wrote them (the guard refuses them)
    pub too_new: Vec<TooNew>,
    /// `S/migrations.log`, where each step and cleanup is written
    pub log: Option<PathBuf>,
}

impl Ran {
    /// The one stderr line of an automatic run: what moved, else what failed;
    /// None when only the manifest changed.
    pub fn line(&self) -> Option<String> {
        if let Some((d, e)) = self.failed.first() {
            let backup = backup_path(&d.file, NIMITZ);
            let kept = if backup.exists() { format!("; its backup is {}", backup.display()) } else { String::new() };
            let what = if d.cleanup.is_some() { "cleanup" } else { "migrating" };
            let log = self.log.as_ref().map(|l| l.display().to_string()).unwrap_or_else(|| "S/migrations.log".into());
            return Some(format!("[ERROR] nimitz {NIMITZ}: {what} {} failed: {} {e} (the file is as it was{kept}; the log is {log}; nimitz doctor)", d.label(), d.file.display()));
        }
        let moved = moved(&self.done);
        if moved.is_empty() {
            return None;
        }
        let backups = if self.backups.is_empty() { String::new() } else { format!("backups *.pre-{NIMITZ}; ") };
        Some(format!("[OK] nimitz {NIMITZ}: migrated {} ({backups}nimitz migrate --plan)", moved.join(", ")))
    }
}

/// `registry 2->3, needs 1->2`: each artifact once, from its lowest to its highest version.
pub fn moved(done: &[Due]) -> Vec<String> {
    let mut by: Vec<(&str, u32, u32)> = vec![];
    let mut cleaned: Vec<String> = vec![];
    for d in done.iter().filter(|d| !d.silent) {
        if d.cleanup.is_some() {
            if !cleaned.contains(&d.label()) {
                cleaned.push(d.label());
            }
            continue;
        }
        match by.iter_mut().find(|(a, _, _)| *a == d.artifact) {
            Some(e) => {
                e.1 = e.1.min(d.from);
                e.2 = e.2.max(d.to);
            }
            None => by.push((d.artifact, d.from, d.to)),
        }
    }
    by.into_iter().map(|(a, f, t)| format!("{a} {f}->{t}")).chain(cleaned).collect()
}

/// `<file>.pre-<nimitz>`: the file as it was before this nimitz migrated it.
pub fn backup_path(file: &Path, nimitz: &str) -> PathBuf {
    let mut s = file.as_os_str().to_os_string();
    s.push(format!(".pre-{nimitz}"));
    PathBuf::from(s)
}

/// Run every step due in `scope` (the local ones: automatic; the shared ones:
/// `nimitz migrate --shared --yes`), under `S/migrate.lock` (one migration
/// at a time on this machine). Per file: a backup `<file>.pre-<this
/// version>` of the bytes the step reads, never replaced (a resumed run keeps
/// the first), the step, an atomic write (the file carries its new version,
/// so a crash between two steps resumes at the second), a line in
/// `S/migrations.log`. A manifest artifact's version is saved after each of
/// its steps. A local run that did every step, with no file too new for this
/// nimitz, sets the manifest's `nimitz` (never lowered: a newer nimitz that
/// ran here keeps its mark).
pub fn run(env: &Env, arts: &[Artifact], scope: Scope) -> Ran {
    run_as(env, arts, scope, NIMITZ)
}

/// `run`, as nimitz `nimitz` (tests).
pub fn run_as(env: &Env, arts: &[Artifact], scope: Scope, nimitz: &str) -> Ran {
    match locked(env) {
        Ok(_lock) => run_unlocked(env, arts, scope, nimitz),
        Err(e) => lock_failed(env, scope, e),
    }
}

/// The automatic local migration: under `S/migrate.lock`, and only while
/// the manifest names another nimitz once the lock is held (a command that
/// waited for another's migration finds it done, and does nothing).
pub fn migrate_local(env: &Env) -> Ran {
    if up_to_date(env) {
        return Ran::default();
    }
    match locked(env) {
        Ok(_lock) if up_to_date(env) => Ran::default(),
        Ok(_lock) => run_unlocked(env, ARTIFACTS, Scope::Local, NIMITZ),
        Err(e) => lock_failed(env, Scope::Local, e),
    }
}

/// `S/migrate.lock`, held exclusively until the returned file drops (None:
/// no state folder here, so nothing local to guard).
fn locked(env: &Env) -> std::io::Result<Option<std::fs::File>> {
    if !env.state.is_dir() {
        return Ok(None);
    }
    let p = env.state.join("migrate.lock");
    // never truncate: another process may hold the lock on this very file
    let f = std::fs::OpenOptions::new().create(true).truncate(false).write(true).open(&p).map_err(|e| std::io::Error::new(e.kind(), format!("{}: {e}", p.display())))?;
    f.lock()?;
    Ok(Some(f))
}

fn lock_failed(env: &Env, scope: Scope, e: std::io::Error) -> Ran {
    let d = Due { artifact: "migrate.lock", scope, file: env.state.join("migrate.lock"), from: 0, to: 0, summary: "", min_nimitz: None, silent: false, cleanup: None };
    Ran { failed: vec![(d, e.to_string())], log: Some(log_path(env)), ..Default::default() }
}

fn run_unlocked(env: &Env, arts: &[Artifact], scope: Scope, nimitz: &str) -> Ran {
    let mut ran = Ran { log: Some(log_path(env)), ..Default::default() };
    let mut man = Manifest::load(env).unwrap_or_default();
    for a in arts.iter().filter(|a| a.scope == scope) {
        match a.kept {
            Kept::Toml(_) | Kept::Json(_) => {
                for f in (a.find)(env) {
                    run_file(env, a, &f, nimitz, &mut ran);
                }
            }
            Kept::Manifest => run_manifest(env, a, &mut man, nimitz, &mut ran),
            Kept::Own => {}
        }
    }
    for c in CLEANUPS.iter().filter(|c| c.scope == scope) {
        for f in (c.find)(env) {
            run_cleanup(env, c, &f, nimitz, &mut ran);
        }
    }
    // the mark says every local file is this nimitz's to read: never while a
    // step failed, nor while a file is too new (the next run must stop again)
    if scope == Scope::Local && ran.failed.is_empty() && ran.too_new.is_empty() {
        for a in arts.iter().filter(|a| a.kept == Kept::Manifest) {
            let v = man.version(a.name).max(a.current);
            man.artifacts.insert(a.name.to_string(), v);
        }
        // a newer nimitz that ran here keeps its mark: this one then checks
        // on every run (and finds what that one wrote too new to read)
        if man.nimitz.is_empty() || !crate::compat::older(nimitz, &man.nimitz) {
            man.nimitz = nimitz.to_string();
        }
        if let Err(e) = man.save(env) {
            let d = Due { artifact: "manifest", scope, file: manifest_path(env), from: 0, to: 0, summary: "", min_nimitz: None, silent: false, cleanup: None };
            ran.failed.push((d, e));
        }
    }
    ran
}

/// One file's chain: lift, then each step, until it is at the version its
/// content wants. Each pass re-reads the file: what the last step wrote is
/// where the next starts, and a file another writer changed meanwhile is
/// read again, never counted as a failure.
fn run_file(env: &Env, a: &Artifact, f: &Path, nimitz: &str, ran: &mut Ran) {
    for _ in 0..64 {
        let Ok(bytes) = std::fs::read(f) else { return };
        if let Err(t) = a.guard_file(f, &bytes) {
            ran.too_new.push(t);
            return;
        }
        let Some(v) = a.version_of(&bytes) else { return };
        let want = a.wanted_of(&bytes);
        if v >= want {
            return;
        }
        let Some(d) = chain(a, f, v, want, false).into_iter().next() else {
            let d = Due { artifact: a.name, scope: a.scope, file: f.to_path_buf(), from: v, to: want, summary: "", min_nimitz: None, silent: false, cleanup: None };
            fail(env, nimitz, ran, d, format!("no step from version {v}"));
            return;
        };
        if let Err(e) = backup_once(f, Some(&bytes), nimitz, ran) {
            fail(env, nimitz, ran, d, format!("backup: {e}"));
            return;
        }
        let res = if v < a.oldest {
            a.lift.ok_or_else(|| "no lift".to_string()).and_then(|(_, lift)| lift(env)).and_then(|_| {
                let now = std::fs::read(f).map_err(|e| e.to_string())?;
                (a.version_of(&now).unwrap_or(0) >= a.oldest).then_some(true).ok_or_else(|| format!("still below version {}", a.oldest))
            })
        } else {
            let step = a.step_from(v).expect("chain found it");
            step_file(env, a, step, f, &bytes)
        };
        match res {
            Ok(true) => {
                log(env, nimitz, &d, None, Some(&backup_path(f, nimitz)));
                ran.done.push(d);
            }
            Ok(false) => {} // changed under the step: read it again
            Err(e) => {
                fail(env, nimitz, ran, d, e);
                return;
            }
        }
    }
}

/// Apply `step` to `bytes` and write the result over `f` (atomic; under the
/// registry lock for a local file, as every writer of the state folder).
/// Ok(false): the file is no longer `bytes` (another writer was first), and
/// nothing was written.
fn step_file(env: &Env, a: &Artifact, step: &Step, f: &Path, bytes: &[u8]) -> Result<bool, String> {
    let Apply::Bytes(apply) = step.apply else { return Err("a file step must rewrite bytes".into()) };
    let out = apply(bytes)?;
    if a.version_of(&out) != Some(step.to) {
        return Err(format!("the step wrote version {:?}, not {}", a.version_of(&out), step.to));
    }
    let write = || -> Result<bool, String> {
        if std::fs::read(f).map_err(|e| e.to_string())? != bytes {
            return Ok(false);
        }
        write_bytes(f, &out).map(|_| true).map_err(|e| e.to_string())
    };
    match a.scope {
        Scope::Local => crate::registry::with_lock(env, None, write).map_err(|e| e.to_string())?,
        Scope::Shared => write(),
    }
}

/// A manifest artifact's chain: each step (on its folder or file, when it
/// rewrites), then the manifest records the step's `to`. An entry above what
/// this nimitz knows is too new: nothing runs.
fn run_manifest(env: &Env, a: &Artifact, man: &mut Manifest, nimitz: &str, ran: &mut Ran) {
    if man.version(a.name) > a.current {
        ran.too_new.push(manifest_too_new(env, a, man));
        return;
    }
    let found = (a.find)(env);
    let file = found.first().cloned().unwrap_or_else(|| manifest_path(env));
    for d in chain(a, &file, man.version(a.name), a.current, found.is_empty()) {
        let step = a.step_from(d.from);
        let res: Result<(), String> = match step.map(|s| s.apply) {
            None => Err(format!("no step from version {}", d.from)),
            Some(Apply::Nothing) => Ok(()),
            Some(Apply::Folder(apply)) => found.iter().try_for_each(|p| {
                backup_once(p, None, nimitz, ran)?;
                apply(p)
            }),
            Some(Apply::Bytes(apply)) => found.iter().filter(|p| p.is_file()).try_for_each(|p| {
                let bytes = std::fs::read(p).map_err(|e| e.to_string())?;
                backup_once(p, Some(&bytes), nimitz, ran)?;
                let out = apply(&bytes)?;
                crate::registry::with_lock(env, None, || write_bytes(p, &out)).map_err(|e| e.to_string())?.map_err(|e| e.to_string())
            }),
        };
        if let Err(e) = res {
            fail(env, nimitz, ran, d, e);
            return;
        }
        man.artifacts.insert(a.name.to_string(), d.to);
        if let Err(e) = man.save(env) {
            fail(env, nimitz, ran, d, e);
            return;
        }
        if !d.silent {
            let backup = backup_path(&d.file, nimitz);
            log(env, nimitz, &d, None, backup.exists().then_some(backup.as_path()));
        }
        ran.done.push(d);
    }
}

fn fail(env: &Env, nimitz: &str, ran: &mut Ran, d: Due, e: String) {
    let backup = backup_path(&d.file, nimitz);
    log(env, nimitz, &d, Some(&e), backup.exists().then_some(backup.as_path()));
    ran.failed.push((d, e));
}

/// `<p>.pre-<nimitz>`, once: `bytes` (what the step reads), or the folder
/// copied whole. An existing backup is never replaced (it is the file as it
/// was before this nimitz first touched it): the file is linked into place
/// from a temp file, which fails when the backup is there.
fn backup_once(p: &Path, bytes: Option<&[u8]>, nimitz: &str, ran: &mut Ran) -> Result<(), String> {
    let b = backup_path(p, nimitz);
    if b.exists() {
        return Ok(());
    }
    let made = match bytes {
        None => copy_dir(p, &b).map(|_| true),
        Some(bytes) => write_new(&b, bytes),
    };
    match made {
        Ok(true) => {
            ran.backups.push(b);
            Ok(())
        }
        Ok(false) => Ok(()),
        Err(e) => Err(format!("{}: {e}", b.display())),
    }
}

/// Write `bytes` to `p` only when `p` is not there (complete or not at all):
/// Ok(false) when it is.
fn write_new(p: &Path, bytes: &[u8]) -> std::io::Result<bool> {
    let tmp = temp_beside(p);
    std::fs::write(&tmp, bytes)?;
    let r = match std::fs::hard_link(&tmp, p) {
        Ok(()) => Ok(true),
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => Ok(false),
        // no hard links here: create_new, which never replaces either
        Err(_) => match std::fs::OpenOptions::new().write(true).create_new(true).open(p) {
            Ok(mut f) => {
                use std::io::Write;
                f.write_all(bytes).map(|_| true)
            }
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => Ok(false),
            Err(e) => Err(e),
        },
    };
    let _ = std::fs::remove_file(&tmp);
    r
}

/// A temp file name beside `p`, unique to this process and call.
fn temp_beside(p: &Path) -> PathBuf {
    static N: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let n = N.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let dir = p.parent().unwrap_or(Path::new("."));
    dir.join(format!(".{}.tmp-{}-{n}", p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default(), std::process::id()))
}

fn copy_dir(from: &Path, to: &Path) -> std::io::Result<()> {
    let tmp = temp_beside(to);
    copy_tree(from, &tmp)?;
    let r = crate::os::replace(&tmp, to);
    let _ = std::fs::remove_dir_all(&tmp);
    r
}

fn copy_tree(from: &Path, to: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(to)?;
    for e in std::fs::read_dir(from)?.flatten() {
        let p = e.path();
        if p.is_dir() {
            copy_tree(&p, &to.join(e.file_name()))?;
        } else {
            std::fs::copy(&p, to.join(e.file_name()))?;
        }
    }
    Ok(())
}

/// A temp file beside `p`, then a rename over it (`os::replace`).
fn write_bytes(p: &Path, bytes: &[u8]) -> std::io::Result<()> {
    std::fs::create_dir_all(p.parent().unwrap_or(Path::new(".")))?;
    let tmp = temp_beside(p);
    std::fs::write(&tmp, bytes)?;
    crate::os::replace(&tmp, p)
}

/// One line of `S/migrations.log`.
fn log(env: &Env, nimitz: &str, d: &Due, error: Option<&str>, backup: Option<&Path>) {
    if !env.state.is_dir() {
        return;
    }
    let mut line = serde_json::json!({
        "at": crate::evidence::now(),
        "nimitz": nimitz,
        "artifact": d.artifact,
        "file": d.file,
        "from": d.from,
        "to": d.to,
        "ok": error.is_none(),
    });
    if let Some(c) = d.cleanup {
        line["cleanup"] = Value::from(c);
    }
    if let Some(e) = error {
        line["error"] = Value::from(e);
    }
    if let Some(b) = backup {
        line["backup"] = Value::from(b.to_string_lossy().into_owned());
    }
    use std::io::Write;
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(log_path(env)) {
        let _ = writeln!(f, "{line}");
    }
}

/// A step the log says failed, with no later success on the same file and step.
#[derive(Clone, Debug, PartialEq)]
pub struct Failed {
    pub artifact: String,
    pub file: String,
    pub from: u32,
    pub to: u32,
    pub error: String,
    pub backup: Option<String>,
    /// a cleanup's label (`CLEANUPS`), when it was one
    pub cleanup: Option<String>,
    /// the log line as written
    pub line: String,
}

impl Failed {
    pub fn issue(&self) -> Issue {
        let restore = match &self.backup {
            Some(b) => format!("; the file as it was before is {b}"),
            None => String::new(),
        };
        Issue {
            code: "format.failed".into(),
            severity: Severity::Error,
            message: match &self.cleanup {
                Some(c) => format!("cleanup {}: {c} failed on {}: {}{restore} (log: {})", self.artifact, self.file, self.error, self.line),
                None => format!("migrating {} {}->{} failed on {}: {}{restore} (log: {})", self.artifact, self.from, self.to, self.file, self.error, self.line),
            },
            fix: Some("fix what the error names, then nimitz migrate --yes (or restore the backup over the file)".into()),
            id: None,
        }
    }
}

/// The steps that failed and were not done since (`S/migrations.log`).
pub fn failed(env: &Env) -> Vec<Failed> {
    let text = std::fs::read_to_string(log_path(env)).unwrap_or_default();
    let mut last: Vec<((String, String, u64), (bool, String, Value))> = vec![];
    for l in text.lines() {
        let Ok(v) = serde_json::from_str::<Value>(l) else { continue };
        // (a cleanup's line is keyed apart from a step's on the same file and version)
        let art = match v["cleanup"].as_str() {
            Some(c) => format!("{}#{c}", v["artifact"].as_str().unwrap_or("")),
            None => v["artifact"].as_str().unwrap_or("").to_string(),
        };
        let key = (art, v["file"].as_str().unwrap_or("").to_string(), v["from"].as_u64().unwrap_or(0));
        let ok = v["ok"].as_bool().unwrap_or(false);
        // a cleanup done on a file supersedes a cleanup that failed on it
        // before (never a step's failure: that is the step's to clear)
        if ok && v["cleanup"].is_string() {
            for e in last.iter_mut().filter(|(k, (_, _, ev))| k.1 == key.1 && ev["cleanup"].is_string()) {
                e.1 .0 = true;
            }
        }
        match last.iter_mut().find(|(k, _)| *k == key) {
            Some(e) => e.1 = (ok, l.to_string(), v),
            None => last.push((key, (ok, l.to_string(), v))),
        }
    }
    // a file now past the failed step's `from` was migrated since (by hand or
    // a restore); a cleanup's, once the cleanup no longer finds it
    let cleaned = |v: &Value, file: &str| -> Option<bool> {
        let c = CLEANUPS.iter().find(|c| Some(c.label) == v["cleanup"].as_str())?;
        Some(!(c.find)(env).iter().any(|f| f.to_string_lossy() == file))
    };
    let done_since = |art: &str, file: &str, from: u64| -> bool {
        let Some(a) = artifact(art) else { return false };
        match a.kept {
            Kept::Toml(_) | Kept::Json(_) => std::fs::read(file).ok().and_then(|b| a.version_of(&b)).is_some_and(|v| u64::from(v) > from),
            Kept::Manifest => Manifest::load(env).is_some_and(|m| u64::from(m.version(art)) > from),
            Kept::Own => false,
        }
    };
    last.into_iter()
        // (a step of an artifact this nimitz no longer has, on a file gone since
        // (0.6's queues, archived), can never be done again: dropped)
        .filter(|(k, (_, _, v))| v["cleanup"].is_string() || artifact(&k.0).is_some() || Path::new(&k.1).exists())
        .filter(|(k, (ok, _, v))| !ok && !cleaned(v, &k.1).unwrap_or_else(|| done_since(&k.0, &k.1, k.2)))
        .map(|(k, (_, line, v))| Failed {
            artifact: k.0.split('#').next().unwrap_or("").to_string(),
            file: k.1,
            from: k.2 as u32,
            to: v["to"].as_u64().unwrap_or(0) as u32,
            error: v["error"].as_str().unwrap_or("").to_string(),
            backup: v["backup"].as_str().map(str::to_string),
            cleanup: v["cleanup"].as_str().map(str::to_string),
            line,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::tests::test_env;

    fn fixture(rel: &str) -> Vec<u8> {
        std::fs::read(Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/formats").join(rel)).unwrap()
    }

    // ---------------------------------------------------------------- the steps

    #[test]
    fn registry_2_3_stamps_the_0_4_0_file_and_keeps_every_record() {
        let v2 = fixture("registry/v2.json");
        let a = artifact("registry").unwrap();
        assert_eq!(a.version_of(&v2), Some(2));
        let v3 = registry_2_3(&v2).unwrap();
        assert_eq!(a.version_of(&v3), Some(3));
        let (old, new): (Value, Value) = (serde_json::from_slice(&v2).unwrap(), serde_json::from_slice(&v3).unwrap());
        assert_eq!(new["min_nimitz"], "0.5.0");
        let keys: Vec<&String> = new.as_object().unwrap().keys().collect();
        assert_eq!(keys[..2], [&"version".to_string(), &"min_nimitz".to_string()], "the stamp leads the file");
        for k in ["machine", "updated_at", "identities"] {
            assert_eq!(old[k], new[k], "{k}");
        }
        // it parses with the current types, and the stamp passes the guard here
        let r = crate::registry::parse(std::str::from_utf8(&v3).unwrap()).unwrap();
        assert_eq!((r.version, r.min_nimitz.as_deref()), (3, Some("0.5.0")));
        assert!(guard_json_bytes("registry", Path::new("r"), &v3, "version").is_ok());
        // idempotent: the step on its own output is the same file
        assert_eq!(registry_2_3(&v3).unwrap(), v3);
    }

    #[test]
    fn fleet_1_2_stamps_the_top_and_keeps_the_rest() {
        let v1 = fixture("fleet/v1.aide.toml");
        let a = artifact("fleet").unwrap();
        assert_eq!((a.version_of(&v1), a.wanted_of(&v1)), (Some(1), 1), "0.7.0: no fleet wants format 2 (the aides are gone)");
        assert_eq!(a.wanted_of(&fixture("fleet/v1.toml")), 1, "a fleet stays at 1 (0.4.x reads it)");
        let v2 = fleet_1_2(&v1).unwrap();
        let text = String::from_utf8(v2.clone()).unwrap();
        assert!(text.starts_with("format = 2\nmin_nimitz = \"0.5.0\"\nfleet = \"MAIN\"\n"), "{text}");
        assert!(text.ends_with(std::str::from_utf8(&v1).unwrap()), "the rest as it was");
        let c: FleetConfig = toml::from_str(&text).unwrap();
        assert_eq!((c.format, c.min_nimitz.as_deref(), c.aide.len()), (Some(2), Some("0.5.0"), 1));
        assert_eq!(fleet_1_2(&v2).unwrap(), v2, "idempotent");
        // a stamp inside a table is the table's, never moved
        let t = stamp_toml("fleet = \"X\"\nformat = 1\n[admiral]\nformat = 7\n", 2, None);
        assert_eq!(t, "format = 2\nfleet = \"X\"\n[admiral]\nformat = 7\n");
        assert!(fleet_1_2(b"fleet = \"X\"\n[[broken").is_err());
    }

    #[test]
    fn the_aide_records_go_from_the_registry_and_nothing_else_moves() {
        let v3 = fixture("registry/v3.aide.json");
        let a = artifact("registry").unwrap();
        assert_eq!(a.version_of(&v3), Some(3));
        let out = drop_aide_records(&v3).unwrap();
        let (old, new): (Value, Value) = (serde_json::from_slice(&v3).unwrap(), serde_json::from_slice(&out).unwrap());
        let mut want = old.clone();
        want["identities"].as_object_mut().unwrap().remove("MAIN/@KB").expect("the fixture has an aide");
        want["retired"].as_object_mut().unwrap().remove("MAIN/@REF").expect("and a retired one");
        assert_eq!(new, want, "the version, the stamp and every other record as they were");
        let keys: Vec<&String> = new.as_object().unwrap().keys().collect();
        assert_eq!(keys[..2], [&"version".to_string(), &"min_nimitz".to_string()], "the key order kept");
        let r = crate::registry::parse(std::str::from_utf8(&out).unwrap()).unwrap();
        assert!(r.unknown.records.is_empty() && r.unknown.roles.is_empty(), "{:?}", r.unknown);
        assert_eq!(drop_aide_records(&out).unwrap(), out, "idempotent");
        // run on this machine: due, done once, backed up, then gone from the plan
        let (_td, env) = test_env();
        std::fs::create_dir_all(&env.state).unwrap();
        std::fs::write(env.registry_path(), &v3).unwrap();
        let d: Vec<(String, u32, u32)> = due(&env, ARTIFACTS, Some(Scope::Local)).iter().filter(|d| d.cleanup.is_some()).map(|d| (d.label(), d.from, d.to)).collect();
        assert_eq!(d, [("registry: aide records dropped".to_string(), 3, 3)]);
        let ran = run(&env, ARTIFACTS, Scope::Local);
        assert!(ran.failed.is_empty(), "{ran:?}");
        assert_eq!(moved(&ran.done), ["registry: aide records dropped"]);
        assert_eq!(std::fs::read(backup_path(&env.registry_path(), NIMITZ)).unwrap(), v3);
        assert!(crate::registry::load(&env).unknown.records.is_empty());
        assert!(due(&env, ARTIFACTS, None).is_empty());
    }

    #[test]
    fn the_aides_cleanup_keeps_the_rest_of_the_fleet_file_byte_for_byte() {
        let v2 = fixture("fleet/v2.aide.toml");
        let out = String::from_utf8(drop_aides(&v2).unwrap()).unwrap();
        assert_eq!(out, "# the home fleet (a comment the cleanup keeps)\nfleet = \"MAIN\"\n\n[machines.sandbox]\nhostname = \"sandbox-host\"\nos = \"linux\"\n\n[admiral]\n\n");
        let c: FleetConfig = toml::from_str(&out).unwrap();
        assert!(c.aide.is_empty() && c.format.is_none() && c.min_nimitz.is_none(), "{c:?}");
        assert_eq!(drop_aides(out.as_bytes()).unwrap(), out.as_bytes(), "idempotent");
        // a fleet at format 1 with aides: only the tables go; a later stamp stays
        let v1 = fixture("fleet/v1.aide.toml");
        let o1 = String::from_utf8(drop_aides(&v1).unwrap()).unwrap();
        assert!(o1.starts_with("fleet = \"MAIN\"\n") && !o1.contains("aide"), "{o1}");
        let later = "format = 2\nmin_nimitz = \"0.9.0\"\nfleet = \"X\"\n";
        assert_eq!(String::from_utf8(drop_aides(later.as_bytes()).unwrap()).unwrap(), "min_nimitz = \"0.9.0\"\nfleet = \"X\"\n");
        // an aide the text cannot take out cleanly: nothing written
        let inline = "fleet = \"X\"\naide = [{ name = \"KB\" }]\n";
        assert!(drop_aides(inline.as_bytes()).unwrap_err().contains("remove them by hand"));
        assert!(drop_aides(b"fleet = \"X\"\n[[broken").is_err());
    }

    /// An aide between other tables: the comments of the table after it stay.
    #[test]
    fn the_aides_cleanup_keeps_the_comments_of_the_table_after_an_aide() {
        let text = "fleet = \"X\"\n\n[[aide]]\n# the notes keeper\nname = \"KB\"\n\n[aide.dirs]\nm = \"/kb\"\n\n# the machines\n[machines.m]\nhostname = \"h\"\n";
        let out = String::from_utf8(drop_aides(text.as_bytes()).unwrap()).unwrap();
        assert_eq!(out, "fleet = \"X\"\n\n\n# the machines\n[machines.m]\nhostname = \"h\"\n");
    }

    /// A folder that cannot be renamed into the archive is left whole (never
    /// copied and removed piecemeal); a file is copied, then removed.
    #[cfg(unix)]
    #[test]
    fn an_archive_that_cannot_rename_leaves_a_folder_whole() {
        use std::os::unix::fs::PermissionsExt;
        let (_td, env) = test_env();
        let q = env.state.join("queues/MAIN%2F@KB");
        std::fs::create_dir_all(&q).unwrap();
        std::fs::write(q.join("queue.json"), "{}").unwrap();
        // the state folder read-only: no rename out of it, no remove
        std::fs::create_dir_all(env.state.join("archive/aides-0.6")).unwrap();
        let mode = |p: &Path, m: u32| std::fs::set_permissions(p, std::fs::Permissions::from_mode(m)).unwrap();
        mode(&env.state, 0o555);
        let e = archive(&env, &env.state.join("queues"), "aides-0.6");
        mode(&env.state, 0o755);
        if e.is_ok() {
            return; // running as root: nothing to test
        }
        assert!(e.unwrap_err().contains("left as it was"));
        assert!(q.join("queue.json").is_file() && !env.state.join("archive/aides-0.6/queues").exists());
    }

    /// A cleanup that fails (here: `S/archive` is a file, so nothing can be
    /// archived) is an error line naming it and the log, leaves the files,
    /// keeps the mark unset so the next command tries again, and is in
    /// `failed()` until a later run does it. A log line of an artifact this
    /// nimitz no longer has never shows.
    #[test]
    fn a_failed_cleanup_is_said_logged_and_retried_until_done() {
        let (_td, env) = test_env();
        std::fs::create_dir_all(env.state.join("queues/MAIN%2F@KB")).unwrap();
        std::fs::write(env.state.join("archive"), "in the way").unwrap();
        let ran = run(&env, ARTIFACTS, Scope::Local);
        let line = ran.line().unwrap();
        assert!(line.starts_with(&format!("[ERROR] nimitz {NIMITZ}: cleanup aides: aide state archived failed: ")), "{line}");
        assert!(line.contains(&format!("the log is {}", log_path(&env).display())), "{line}");
        assert!(env.state.join("queues/MAIN%2F@KB").is_dir(), "nothing moved");
        assert!(!up_to_date(&env), "the next command tries again");
        let f = failed(&env);
        assert_eq!(f.len(), 1, "{f:?}");
        assert!(f[0].issue().message.starts_with("cleanup aides: aide state archived failed on "), "{}", f[0].issue().message);
        // an old line of a step this nimitz no longer has: dropped
        let mut log = std::fs::read_to_string(log_path(&env)).unwrap();
        log.push_str("{\"artifact\":\"queues\",\"file\":\"/x/queue.json\",\"from\":1,\"to\":2,\"ok\":false,\"error\":\"old\"}\n");
        std::fs::write(log_path(&env), log).unwrap();
        assert_eq!(failed(&env).len(), 1);
        std::fs::remove_file(env.state.join("archive")).unwrap();
        let ran = run(&env, ARTIFACTS, Scope::Local);
        assert!(ran.failed.is_empty(), "{ran:?}");
        assert!(env.state.join("archive/aides-0.6/queues/MAIN%2F@KB").is_dir());
        assert!(up_to_date(&env));
        assert!(failed(&env).is_empty(), "{:?}", failed(&env));
    }

    #[test]
    fn the_aides_cleanup_is_shared_and_takes_the_fleet_back_to_format_1() {
        let (_td, env) = test_env();
        std::fs::create_dir_all(&env.state).unwrap();
        let fleet = env.home.join("main");
        std::fs::create_dir_all(&fleet).unwrap();
        std::fs::write(fleet.join("fleet.toml"), fixture("fleet/v2.aide.toml")).unwrap();
        // it loads as it is (the aides carried, unused), and says what to run
        let f = crate::config::load_fleet(&env, "MAIN").unwrap();
        assert_eq!(f.config.aide.len(), 2);
        let w: Vec<Issue> = crate::config::validate(&env, &f).into_iter().filter(|i| i.code == "aide.legacy").collect();
        assert_eq!(w.len(), 1, "{w:?}");
        assert_eq!(w[0].severity, Severity::Warn);
        assert!(w[0].fix.as_deref().unwrap().contains("nimitz migrate --shared --yes"));
        // a local run leaves it; the plan lists it as shared
        let ran = run(&env, ARTIFACTS, Scope::Local);
        assert!(ran.failed.is_empty() && !ran.done.iter().any(|d| d.artifact == "fleet"), "{ran:?}");
        let d = due(&env, ARTIFACTS, Some(Scope::Shared));
        assert_eq!(d.iter().map(|d| (d.label(), d.from, d.to)).collect::<Vec<_>>(), [("fleet: aides dropped".to_string(), 2, 1)]);
        let s = run(&env, ARTIFACTS, Scope::Shared);
        assert!(s.failed.is_empty(), "{s:?}");
        assert_eq!(moved(&s.done), ["fleet: aides dropped"]);
        assert_eq!(std::fs::read(backup_path(&fleet.join("fleet.toml"), NIMITZ)).unwrap(), fixture("fleet/v2.aide.toml"));
        let f = crate::config::load_fleet(&env, "MAIN").unwrap();
        assert!(f.config.aide.is_empty() && f.config.format.is_none() && f.config.min_nimitz.is_none());
        assert!(crate::config::validate(&env, &f).iter().all(|i| i.code != "aide.legacy"));
        assert!(due(&env, ARTIFACTS, None).is_empty());
        assert!(run(&env, ARTIFACTS, Scope::Shared).done.is_empty(), "idempotent");
        assert!(failed(&env).is_empty());
    }

    #[test]
    fn the_aides_local_state_is_archived_never_deleted() {
        let (_td, env) = test_env();
        let s = &env.state;
        for (p, body) in [
            ("queues/MAIN%2F@KB/queue.json", "{}"),
            ("queues/MAIN%2F@KB/items/000000000001.json", "{}"),
            ("aides/MAIN%2F@REF/snapshot/page.md", "x"),
            ("ring.lock", ""),
            ("ring.stamp", "1"),
            ("needs/MAIN%2F@KB.json", "{}"),
            ("needs/MAIN%2F@KB#dead-000000000001.json", "{}"),
            ("needs/MAIN%2FARGOS%2FS.json", "{}"),
        ] {
            std::fs::create_dir_all(s.join(p).parent().unwrap()).unwrap();
            std::fs::write(s.join(p), body).unwrap();
        }
        let local = due(&env, ARTIFACTS, Some(Scope::Local));
        let cleaned: Vec<String> = local.iter().filter(|d| d.cleanup.is_some()).map(|d| d.file.strip_prefix(s).unwrap().to_string_lossy().into_owned()).collect();
        assert_eq!(cleaned, ["queues", "aides", "ring.lock", "ring.stamp", "needs/MAIN%2F@KB#dead-000000000001.json", "needs/MAIN%2F@KB.json"]);
        let ran = run(&env, ARTIFACTS, Scope::Local);
        assert!(ran.failed.is_empty(), "{ran:?}");
        assert!(moved(&ran.done).contains(&"aides: aide state archived".to_string()), "{:?}", moved(&ran.done));
        let arch = s.join("archive/aides-0.6");
        for p in ["queues/MAIN%2F@KB/items/000000000001.json", "aides/MAIN%2F@REF/snapshot/page.md", "ring.stamp", "needs/MAIN%2F@KB.json", "needs/MAIN%2F@KB#dead-000000000001.json"] {
            assert!(arch.join(p).is_file(), "{p} archived");
        }
        assert!(!s.join("queues").exists() && !s.join("ring.lock").exists());
        assert!(s.join("needs/MAIN%2FARGOS%2FS.json").is_file(), "a ship's notice stays");
        assert!(due(&env, ARTIFACTS, None).iter().all(|d| d.cleanup.is_none()));
        // a second leftover never replaces the first archive
        std::fs::create_dir_all(s.join("queues/x")).unwrap();
        assert!(run(&env, ARTIFACTS, Scope::Local).failed.is_empty());
        assert!(arch.join("queues.2/x").is_dir() && arch.join("queues/MAIN%2F@KB").is_dir());
    }

    // ---------------------------------------------------------------- the guard

    #[test]
    fn the_guard_refuses_a_min_nimitz_above_this_nimitz_only() {
        let p = Path::new("/h/main/fleet.toml");
        let above = guard_toml("fleet", p, "min_nimitz = \"99.0.0\"\nformat = 2\nfleet = \"MAIN\"\n").unwrap_err();
        assert_eq!(above.to_string(), format!("/h/main/fleet.toml needs nimitz >= 99.0.0 (this machine runs {NIMITZ}): update nimitz here"));
        assert_eq!(above.issue(None).code, "config.too_new");
        assert!(guard_toml("fleet", p, &format!("min_nimitz = \"{NIMITZ}\"\nformat = 2\nfleet = \"MAIN\"\n")).is_ok(), "equal: read");
        assert!(guard_toml("fleet", p, "fleet = \"MAIN\"\n").is_ok(), "absent: read");
        assert!(guard_toml("fleet", p, "fleet = [broken").is_ok(), "not TOML: the strict parse says why");
        // a format above the one this nimitz knows, with no min_nimitz
        let fmt = guard_toml("squadron", p, "format = 2\nsquadron = \"A\"\n").unwrap_err();
        assert!(fmt.to_string().contains("is format 2, newer than this nimitz reads"), "{fmt}");
        let reg = serde_json::json!({"version": REGISTRY_VERSION + 1, "machine": "m"});
        assert!(guard_json("registry", p, &reg, "version").is_err());
        assert!(guard_json("registry", p, &serde_json::json!({"version": 2}), "version").is_ok());
    }

    // ---------------------------------------------------------------- the runner

    fn thing(env: &Env) -> Vec<PathBuf> {
        existing(env.state.join("thing.json"))
    }

    fn bump(bytes: &[u8], to: u32) -> Result<Vec<u8>, String> {
        let mut v: Value = serde_json::from_slice(bytes).map_err(|e| e.to_string())?;
        v["v"] = Value::from(to);
        v[format!("added_{to}")] = Value::from(true);
        serde_json::to_vec(&v).map_err(|e| e.to_string())
    }
    fn to2(b: &[u8]) -> Result<Vec<u8>, String> {
        bump(b, 2)
    }
    fn to3(b: &[u8]) -> Result<Vec<u8>, String> {
        bump(b, 3)
    }
    fn broken(_: &[u8]) -> Result<Vec<u8>, String> {
        Err("disk on fire".into())
    }

    static ONE_STEP: &[Step] = &[Step { artifact: "thing", from: 1, to: 2, scope: Scope::Local, summary: "", min_nimitz: None, apply: Apply::Bytes(to2) }];
    static TWO_STEPS: &[Step] = &[
        Step { artifact: "thing", from: 1, to: 2, scope: Scope::Local, summary: "", min_nimitz: None, apply: Apply::Bytes(to2) },
        Step { artifact: "thing", from: 2, to: 3, scope: Scope::Local, summary: "", min_nimitz: None, apply: Apply::Bytes(to3) },
    ];
    static BROKEN: &[Step] = &[
        Step { artifact: "thing", from: 1, to: 2, scope: Scope::Local, summary: "", min_nimitz: None, apply: Apply::Bytes(to2) },
        Step { artifact: "thing", from: 2, to: 3, scope: Scope::Local, summary: "", min_nimitz: None, apply: Apply::Bytes(broken) },
    ];

    fn things(current: u32, steps: &'static [Step]) -> [Artifact; 1] {
        [Artifact { name: "thing", path: "S/thing.json", scope: Scope::Local, current, oldest: 1, kept: Kept::Json("v"), steps, find: thing, wanted: None, lift: None }]
    }

    fn log_lines(env: &Env) -> Vec<Value> {
        std::fs::read_to_string(log_path(env)).unwrap_or_default().lines().map(|l| serde_json::from_str(l).unwrap()).collect()
    }

    #[test]
    fn the_runner_backs_up_once_logs_each_step_and_a_second_run_does_nothing() {
        let (_td, env) = test_env();
        std::fs::create_dir_all(&env.state).unwrap();
        let f = env.state.join("thing.json");
        std::fs::write(&f, r#"{"v": 1, "keep": "me"}"#).unwrap();
        let arts = things(3, TWO_STEPS);
        assert_eq!(due(&env, &arts, None).iter().map(|d| (d.from, d.to)).collect::<Vec<_>>(), [(1, 2), (2, 3)]);
        let ran = run_as(&env, &arts, Scope::Local, "9.9.9");
        assert_eq!((ran.done.len(), ran.failed.len(), ran.backups.len()), (2, 0, 1), "{ran:?}");
        let now: Value = serde_json::from_slice(&std::fs::read(&f).unwrap()).unwrap();
        assert_eq!((now["v"].as_u64(), now["keep"].as_str(), now["added_2"].as_bool()), (Some(3), Some("me"), Some(true)));
        assert_eq!(std::fs::read_to_string(backup_path(&f, "9.9.9")).unwrap(), r#"{"v": 1, "keep": "me"}"#);
        let log = log_lines(&env);
        assert_eq!(log.iter().map(|l| (l["from"].as_u64().unwrap(), l["to"].as_u64().unwrap(), l["ok"].as_bool().unwrap())).collect::<Vec<_>>(), [(1, 2, true), (2, 3, true)]);
        assert_eq!(Manifest::load(&env).unwrap().nimitz, "9.9.9", "every local step done: the manifest says so");
        assert!(no_tmp(&env.state), "atomic writes leave no temp file");
        // a second run: nothing due, nothing written, nothing logged
        let before = std::fs::read(&f).unwrap();
        let again = run_as(&env, &arts, Scope::Local, "9.9.9");
        assert!(again.done.is_empty() && again.failed.is_empty() && again.backups.is_empty(), "{again:?}");
        assert_eq!(std::fs::read(&f).unwrap(), before);
        assert_eq!(log_lines(&env).len(), 2);
        assert!(again.line().is_none());
    }

    fn no_tmp(dir: &Path) -> bool {
        std::fs::read_dir(dir).unwrap().flatten().all(|e| !e.file_name().to_string_lossy().contains(".tmp-"))
    }

    #[test]
    fn a_crash_between_two_steps_resumes_at_the_second_and_keeps_the_first_backup() {
        let (_td, env) = test_env();
        std::fs::create_dir_all(&env.state).unwrap();
        let f = env.state.join("thing.json");
        std::fs::write(&f, r#"{"v": 1}"#).unwrap();
        // the first step wrote (it stamps its version), then the process died
        // before the second: the manifest's mark was never set
        let first = run_as(&env, &things(2, ONE_STEP), Scope::Local, "9.9.9");
        assert_eq!(first.done.len(), 1);
        let mut m = Manifest::load(&env).unwrap();
        m.nimitz = String::new();
        m.save(&env).unwrap();
        assert!(!up_to_date(&env));
        // the next run starts at 2: one step, the backup still the original
        let resumed = run_as(&env, &things(3, TWO_STEPS), Scope::Local, "9.9.9");
        assert_eq!(resumed.done.iter().map(|d| (d.from, d.to)).collect::<Vec<_>>(), [(2, 3)]);
        assert!(resumed.backups.is_empty(), "the backup is taken once");
        assert_eq!(std::fs::read_to_string(backup_path(&f, "9.9.9")).unwrap(), r#"{"v": 1}"#);
        assert_eq!(serde_json::from_slice::<Value>(&std::fs::read(&f).unwrap()).unwrap()["v"], 3);
        assert_eq!(Manifest::load(&env).unwrap().nimitz, "9.9.9");
    }

    #[test]
    fn a_failed_step_leaves_its_file_logs_the_error_and_never_sets_the_mark() {
        let (_td, env) = test_env();
        std::fs::create_dir_all(&env.state).unwrap();
        let f = env.state.join("thing.json");
        std::fs::write(&f, r#"{"v": 1}"#).unwrap();
        let ran = run_as(&env, &things(3, BROKEN), Scope::Local, "9.9.9");
        assert_eq!((ran.done.len(), ran.failed.len()), (1, 1));
        assert_eq!(serde_json::from_slice::<Value>(&std::fs::read(&f).unwrap()).unwrap()["v"], 2, "the step that ran stays; the failed one wrote nothing");
        assert!(Manifest::load(&env).unwrap_or_default().nimitz.is_empty(), "not every local step is done");
        let last = log_lines(&env).pop().unwrap();
        assert_eq!((last["ok"].as_bool(), last["error"].as_str(), last["from"].as_u64()), (Some(false), Some("disk on fire"), Some(2)));
        assert!(last["backup"].as_str().unwrap().ends_with("thing.json.pre-9.9.9"));
        let line = ran.line().unwrap();
        assert!(line.starts_with("[ERROR] nimitz") && line.contains("thing 2->3 failed") && line.contains("disk on fire"), "{line}");
        let f = failed(&env);
        assert_eq!(f.len(), 1);
        assert_eq!(f[0].issue().code, "format.failed");
        assert!(f[0].issue().message.contains("thing.json.pre-9.9.9"), "{:?}", f[0].issue());
        // fixed: the step runs, and the failure is history
        let ok = run_as(&env, &things(3, TWO_STEPS), Scope::Local, "9.9.9");
        assert_eq!((ok.done.len(), ok.failed.len()), (1, 0));
        assert!(failed(&env).is_empty());
    }

    #[test]
    fn a_manifest_artifact_moves_in_the_manifest_silently_when_it_has_no_files() {
        let (_td, env) = test_env();
        std::fs::create_dir_all(&env.state).unwrap();
        let needs = artifact("needs").unwrap();
        let arts = std::slice::from_ref(needs);
        // no needs/ folder: the manifest records 2, no line, no log
        let quiet = run(&env, arts, Scope::Local);
        assert_eq!(quiet.done.iter().map(|d| (d.from, d.to, d.silent)).collect::<Vec<_>>(), [(1, 2, true)]);
        assert!(quiet.line().is_none());
        assert_eq!(Manifest::load(&env).unwrap().version("needs"), 2);
        assert!(log_lines(&env).is_empty());
        // a 0.4.0 notice on disk: the same step, said, and the notice untouched
        let (_td2, env2) = test_env();
        let p = crate::notify::path(&env2, "MAIN/ARGOS/CLEAN");
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(&p, fixture("needs/v1.json")).unwrap();
        let said = run(&env2, arts, Scope::Local);
        assert_eq!(said.line().unwrap(), format!("[OK] nimitz {NIMITZ}: migrated needs 1->2 (nimitz migrate --plan)"));
        assert_eq!(std::fs::read(&p).unwrap(), fixture("needs/v1.json"));
        assert!(crate::notify::read(&env2, "MAIN/ARGOS/CLEAN").is_some(), "the current type reads it");
        assert!(run(&env2, arts, Scope::Local).done.is_empty(), "idempotent");
    }

    #[test]
    fn the_0_4_0_state_migrates_by_itself_and_the_shared_fleet_waits() {
        let (_td, env) = test_env();
        std::fs::create_dir_all(&env.state).unwrap();
        std::fs::write(env.registry_path(), fixture("registry/v2.json")).unwrap();
        let fleet = env.home.join("main");
        std::fs::create_dir_all(&fleet).unwrap();
        std::fs::write(fleet.join("fleet.toml"), fixture("fleet/v1.aide.toml")).unwrap();
        assert!(!up_to_date(&env));
        let ran = run(&env, ARTIFACTS, Scope::Local);
        assert!(ran.failed.is_empty(), "{ran:?}");
        assert_eq!(moved(&ran.done), ["registry 2->3"]);
        assert_eq!(ran.line().unwrap(), format!("[OK] nimitz {NIMITZ}: migrated registry 2->3 (backups *.pre-{NIMITZ}; nimitz migrate --plan)"));
        assert_eq!(std::fs::read(backup_path(&env.registry_path(), NIMITZ)).unwrap(), fixture("registry/v2.json"));
        assert!(up_to_date(&env));
        let r = crate::registry::parse(&std::fs::read_to_string(env.registry_path()).unwrap()).unwrap();
        let was = crate::registry::parse(std::str::from_utf8(&fixture("registry/v2.json")).unwrap()).unwrap();
        assert_eq!((r.version, &r.identities), (3, &was.identities));
        // the shared cleanup (a 0.5 fleet's aide) is due, never run by the local run
        let shared = due(&env, ARTIFACTS, Some(Scope::Shared));
        assert_eq!(shared.iter().map(|d| (d.label(), d.from, d.to)).collect::<Vec<_>>(), [("fleet: aides dropped".to_string(), 1, 1)]);
        assert_eq!(std::fs::read(fleet.join("fleet.toml")).unwrap(), fixture("fleet/v1.aide.toml"));
        let s = run(&env, ARTIFACTS, Scope::Shared);
        assert_eq!(moved(&s.done), ["fleet: aides dropped"]);
        assert!(due(&env, ARTIFACTS, None).is_empty());
        let c = crate::config::load_fleet(&env, "MAIN").unwrap().config;
        assert!(c.format.is_none() && c.aide.is_empty());
    }

    #[test]
    fn a_machine_records_its_nimitz_once_and_doctor_names_the_ones_behind() {
        let (_td, env) = test_env();
        crate::config::tests::make_fleet(&env);
        let mut f = crate::config::load_fleet(&env, "MAIN").unwrap();
        let mine = f.machine.clone().unwrap();
        let p = machine_record_path(&f.dir, &mine);
        assert!(machine_issues(&f).iter().any(|i| i.message == format!("machine {mine}: no nimitz >= 0.5.0 seen; update it")));
        record_machine(&env);
        let first = machine_record(&f.dir, &mine).unwrap();
        assert_eq!(first.nimitz, NIMITZ);
        assert!(machine_issues(&f).is_empty(), "{:?}", machine_issues(&f));
        let bytes = std::fs::read(&p).unwrap();
        record_machine(&env);
        assert_eq!(std::fs::read(&p).unwrap(), bytes, "written only when the version changes");
        // another machine listed, never seen; one seen on an older nimitz
        f.config.machines.insert("far".into(), MachineDef { hostname: "far-box".into(), os: None });
        f.config.machines.insert("old".into(), MachineDef { hostname: "old-box".into(), os: None });
        write_bytes(&machine_record_path(&f.dir, "old"), br#"{"nimitz": "0.4.9", "seen": 1.0}"#).unwrap();
        let msgs: Vec<String> = machine_issues(&f).into_iter().map(|i| i.message).collect();
        assert_eq!(msgs, ["machine far: no nimitz >= 0.5.0 seen; update it", "machine old: no nimitz >= 0.5.0 seen (it last ran 0.4.9); update it"]);
    }

    /// Two first commands at once (B used to pass the backup check before
    /// A's backup landed, then back up the migrated file over it and report
    /// a false failure): one migrates, the others wait on `S/migrate.lock`
    /// and find the work done. One backup, the 0.4.0 bytes; no error.
    #[test]
    fn first_commands_at_once_migrate_once_with_one_backup_of_the_old_bytes() {
        for _ in 0..5 {
            let (_td, env) = test_env();
            std::fs::create_dir_all(&env.state).unwrap();
            let v2 = fixture("registry/v2.json");
            std::fs::write(env.registry_path(), &v2).unwrap();
            let runs: Vec<Ran> = std::thread::scope(|sc| {
                let hs: Vec<_> = (0..6).map(|_| sc.spawn(|| migrate_local(&env))).collect();
                hs.into_iter().map(|h| h.join().unwrap()).collect()
            });
            assert!(runs.iter().all(|r| r.failed.is_empty() && r.too_new.is_empty()), "{runs:?}");
            assert_eq!(runs.iter().flat_map(|r| r.done.iter().filter(|d| !d.silent)).count(), 1, "{runs:?}");
            assert_eq!(runs.iter().map(|r| r.backups.len()).sum::<usize>(), 1);
            assert_eq!(std::fs::read(backup_path(&env.registry_path(), NIMITZ)).unwrap(), v2, "the backup holds the old bytes");
            assert_eq!(artifact("registry").unwrap().version_of(&std::fs::read(env.registry_path()).unwrap()), Some(3));
            assert!(up_to_date(&env));
            assert!(failed(&env).is_empty());
            assert!(no_tmp(&env.state));
        }
    }

    #[test]
    fn a_backup_is_never_replaced_and_a_file_changed_under_a_step_is_read_again() {
        let (_td, env) = test_env();
        std::fs::create_dir_all(&env.state).unwrap();
        let b = env.state.join("x.pre-1");
        assert!(write_new(&b, b"first").unwrap());
        assert!(!write_new(&b, b"second").unwrap());
        assert_eq!(std::fs::read(&b).unwrap(), b"first");
        // the step read v2 bytes, then another writer changed the file: nothing written, no error
        let f = env.registry_path();
        let v2 = fixture("registry/v2.json");
        std::fs::write(&f, b"{\"version\": 2, \"machine\": \"other\"}").unwrap();
        let a = artifact("registry").unwrap();
        assert_eq!(step_file(&env, a, a.step_from(2).unwrap(), &f, &v2), Ok(false));
        assert_eq!(std::fs::read(&f).unwrap(), b"{\"version\": 2, \"machine\": \"other\"}");
        // the runner reads it again and migrates what is there now
        let ran = run(&env, ARTIFACTS, Scope::Local);
        assert!(ran.failed.is_empty(), "{ran:?}");
        assert_eq!(serde_json::from_slice::<Value>(&std::fs::read(&f).unwrap()).unwrap()["machine"], "other");
    }

    /// A 0.3.x (v1) registry through the runner: the lift (the v1 load,
    /// saved; registry.json.v1 kept), then 2 -> 3; the backup is the v1 file.
    #[test]
    fn a_0_3_registry_is_lifted_then_stepped() {
        let (_td, env) = test_env();
        std::fs::create_dir_all(&env.state).unwrap();
        let v1 = serde_json::json!({"version": 1, "machine": "test-mac", "identities": {
            "MAIN/ARGOS/ARGOS": {"role": "ship", "name": "[MAIN.ARGOS]-ARGOS", "cwd": "/w", "current": "sid-1",
                "lineage": [{"session_id": "sid-1", "bound_at": 1.0, "via": "launch"}]},
            "MAIN/ARGOS/~fixci": {"role": "tender", "name": "[MAIN.ARGOS]-COMMODORE/fixci", "cwd": "/sq", "current": "sid-t",
                "lineage": [{"session_id": "sid-t", "bound_at": 2.0, "via": "launch"}]}}})
        .to_string();
        std::fs::write(env.registry_path(), &v1).unwrap();
        assert_eq!(due(&env, ARTIFACTS, Some(Scope::Local)).iter().filter(|d| d.artifact == "registry").map(|d| (d.from, d.to)).collect::<Vec<_>>(), [(1, 2), (2, 3)]);
        let ran = run(&env, ARTIFACTS, Scope::Local);
        assert!(ran.failed.is_empty(), "{ran:?}");
        assert_eq!(moved(&ran.done), ["registry 1->3"]);
        assert_eq!(std::fs::read_to_string(backup_path(&env.registry_path(), NIMITZ)).unwrap(), v1);
        assert_eq!(std::fs::read_to_string(crate::registry::v1_path(&env)).unwrap(), v1);
        let r = crate::registry::parse(&std::fs::read_to_string(env.registry_path()).unwrap()).unwrap();
        assert_eq!((r.version, r.min_nimitz.as_deref()), (3, Some("0.5.0")));
        assert_eq!(r.identities["MAIN/ARGOS/ARGOS"].session.as_deref(), Some("sid-1"));
        assert_eq!(r.identities["MAIN/ARGOS/ARGOS"].lineage[0].at, 1.0);
        assert!(r.retired.contains_key("MAIN/ARGOS/~fixci"), "the tender is history");
    }

    /// The manifest gone, a registry too new beside it: the mark is never
    /// set, so every later run stops again (no fast path to an empty registry).
    #[test]
    fn no_mark_while_a_file_is_too_new() {
        let (_td, env) = test_env();
        std::fs::create_dir_all(&env.state).unwrap();
        std::fs::write(env.registry_path(), serde_json::json!({"version": 4, "machine": "m", "identities": {}}).to_string()).unwrap();
        for _ in 0..2 {
            let ran = migrate_local(&env);
            assert_eq!(ran.too_new.len(), 1, "{ran:?}");
            assert!(!up_to_date(&env));
            assert!(Manifest::load(&env).unwrap_or_default().nimitz.is_empty());
        }
        // a manifest entry above what this nimitz knows stops it the same way
        let (_td2, env2) = test_env();
        std::fs::create_dir_all(&env2.state).unwrap();
        Manifest { artifacts: [("needs".to_string(), 9)].into(), ..Default::default() }.save(&env2).unwrap();
        let ran = migrate_local(&env2);
        assert_eq!(ran.too_new.iter().map(|t| (t.artifact, t.format)).collect::<Vec<_>>(), [("needs", 9)]);
        assert!(!up_to_date(&env2));
        assert_eq!(Manifest::load(&env2).unwrap().version("needs"), 9, "never lowered");
    }

    /// The lenient read: a TOML that does not parse is still read for its
    /// stamp above the first table; a negative format is refused, never
    /// wrapped into a huge one.
    #[test]
    fn the_stamp_reads_a_file_that_does_not_parse_and_refuses_a_negative_format() {
        let p = Path::new("/h/main/fleet.toml");
        let raw = "format = 3 # a comment\nmin_nimitz = \"9.1.0\"\nfleet = \"MAIN\"\nnew_syntax = <<\n[machines]\nmin_nimitz = \"99\"\n";
        assert!(toml::from_str::<toml::Table>(raw).is_err());
        assert_eq!(toml_stamp(raw, "format"), Stamp { format: Some(3), min_nimitz: Some("9.1.0".into()) });
        assert!(guard_toml("fleet", p, raw).unwrap_err().to_string().contains("needs nimitz >= 9.1.0"));
        assert!(guard_toml("fleet", p, "fleet = [broken").is_ok(), "no stamp: the strict parse says why");
        let neg = guard_toml("fleet", p, "format = -1\nfleet = \"MAIN\"\n").unwrap_err();
        assert_eq!(neg.format, -1);
        assert!(neg.reason().contains("format -1, which is no format version"), "{}", neg.reason());
        assert!(guard_json("registry", p, &serde_json::json!({"version": -2}), "version").is_err());
        assert!(guard_json("registry", p, &serde_json::json!({"version": u64::MAX}), "version").is_err());
        assert_eq!(artifact("fleet").unwrap().version_of(b"format = -1\n"), None, "no version: no step runs on it");
    }

    #[test]
    fn the_version_constants_are_the_artifacts_current_ones() {
        assert_eq!(current("registry"), REGISTRY_VERSION);
        assert_eq!(current("active"), crate::active::VERSION);
    }

    #[test]
    fn a_newer_nimitz_mark_is_kept_and_its_files_are_left_alone() {
        let (_td, env) = test_env();
        std::fs::create_dir_all(&env.state).unwrap();
        let raw = serde_json::json!({"version": 9, "min_nimitz": "9.0.0", "machine": "m", "identities": {}}).to_string();
        std::fs::write(env.registry_path(), &raw).unwrap();
        Manifest { nimitz: "9.0.0".into(), ..Default::default() }.save(&env).unwrap();
        let ran = run(&env, ARTIFACTS, Scope::Local);
        assert_eq!(ran.too_new.len(), 1);
        assert!(ran.done.iter().all(|d| d.silent), "{ran:?}");
        assert_eq!(std::fs::read_to_string(env.registry_path()).unwrap(), raw, "never rewritten");
        assert_eq!(Manifest::load(&env).unwrap().nimitz, "9.0.0", "the newer mark stays");
        assert!(!up_to_date(&env), "so every run checks again");
        assert!(issues(&env).iter().any(|i| i.code == "config.too_new"));
    }
}
