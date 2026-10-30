//! Admiralty's own file under the format rule (docs/design/migrations.md
//! section 6): `S/admiralty-ui.json` (`app::Saved`) is pinned to the
//! `admiralty-ui` artifact's version, beside nimitz's own shapes.
use std::path::Path;

/// A populated `Saved`, serialized, against
/// `nimitz-core/tests/fixtures/formats/shapes/admiralty-ui.v<N>.json`
/// (`NIMITZ_BLESS=1` writes it).
#[test]
fn formats_shapes_are_pinned() {
    let example = serde_json::json!({"format": 1, "expanded": {"MAIN": true}, "filter": "argos", "fleet_states": [true, false],
        "flagged_only": true, "show_stowaways": true, "tab": "MAIN", "anim": false,
        "finder": {"pane_only": false, "topic": true, "no_stowaways": true, "this_fleet": true, "ships_only": true}});
    let saved: admiralty::app::Saved = serde_json::from_value(example).unwrap();
    let text = serde_json::to_string_pretty(&saved).unwrap() + "\n";
    let v = nimitz_core::formats::current("admiralty-ui");
    let shapes = Path::new(env!("CARGO_MANIFEST_DIR")).join("../nimitz-core/tests/fixtures/formats/shapes");
    let p = shapes.join(format!("admiralty-ui.v{v}.json"));
    if std::env::var("NIMITZ_BLESS").is_ok_and(|b| b == "1") {
        for e in std::fs::read_dir(&shapes).unwrap().flatten() {
            if e.file_name().to_string_lossy().starts_with("admiralty-ui.v") && e.path() != p {
                std::fs::remove_file(e.path()).unwrap();
            }
        }
        std::fs::write(&p, &text).unwrap();
        return;
    }
    let golden = std::fs::read_to_string(&p).unwrap_or_else(|_| panic!("no pinned shape {}: run NIMITZ_BLESS=1 cargo test -p admiralty --test formats", p.display())).replace("\r\n", "\n");
    assert!(
        golden == text,
        "the shape of app::Saved changed (artifact admiralty-ui, version {v}): bump admiralty-ui's version in nimitz_core::formats::ARTIFACTS, add the step {v} -> {}, add the fixture nimitz-core/tests/fixtures/formats/admiralty-ui/v{v}.json, then run NIMITZ_BLESS=1 cargo test -p admiralty --test formats\n--- golden\n{golden}\n--- now\n{text}",
        v + 1
    );
}

/// The format-1 fixture through its step reads as `Saved`, the finder's
/// toggles at their defaults (#14).
#[test]
fn the_v1_state_reads_after_its_step() {
    let p = Path::new(env!("CARGO_MANIFEST_DIR")).join("../nimitz-core/tests/fixtures/formats/admiralty-ui/v1.json");
    let v2 = nimitz_core::formats::admiralty_ui_1_2(&std::fs::read(p).unwrap()).unwrap();
    let saved: admiralty::app::Saved = serde_json::from_slice(&v2).unwrap();
    assert_eq!((saved.format, saved.filter.as_str(), saved.finder), (2, "argos", admiralty::finder::Toggles::default()));
}
