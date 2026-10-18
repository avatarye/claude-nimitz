//! The plugin's frontmatter is strict YAML (#7): Claude Code parses each
//! agent's and `SKILL.md`'s frontmatter as YAML, and when that fails it
//! retries with a fallback that quotes a `key: value` line holding `: `,
//! but only on LF lines (its line pattern ends in `(\S.*)$`, and `.` never
//! takes the `\r` of a CRLF checkout). An unquoted `: ` in a description
//! parsed on Linux and dropped the skill on Windows. So every frontmatter
//! here must parse strictly as it is, in LF and in CRLF, and so must the
//! frontmatter nimitz writes into a fleet's plugin.
use std::path::{Path, PathBuf};
use yaml_rust2::{Yaml, YamlLoader};

fn plugin() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../plugin")
}

fn markdown(dir: &Path, out: &mut Vec<PathBuf>) {
    for e in std::fs::read_dir(dir).unwrap().flatten() {
        let p = e.path();
        if p.is_dir() {
            markdown(&p, out);
        } else if p.extension().is_some_and(|x| x == "md") {
            out.push(p);
        }
    }
}

/// The frontmatter block as Claude Code cuts it: after a first line `---`,
/// up to the next line `---` (a BOM dropped first).
fn block(text: &str) -> Option<String> {
    let t = text.strip_prefix('\u{feff}').unwrap_or(text);
    let mut lines = t.split_inclusive('\n');
    if lines.next()?.trim_end() != "---" {
        return None;
    }
    let mut out = String::new();
    for l in lines {
        if l.trim_end() == "---" {
            return Some(out);
        }
        out.push_str(l);
    }
    None
}

/// The block parsed strictly: one mapping, else why not.
fn strict(block: &str) -> Result<Yaml, String> {
    let docs = YamlLoader::load_from_str(block).map_err(|e| e.to_string())?;
    match docs.into_iter().next() {
        Some(y @ Yaml::Hash(_)) => Ok(y),
        other => Err(format!("not a mapping: {other:?}")),
    }
}

#[test]
fn every_plugin_frontmatter_is_strict_yaml_in_lf_and_crlf() {
    let mut files = vec![];
    markdown(&plugin(), &mut files);
    files.sort();
    let mut checked = vec![];
    let mut bad = vec![];
    for f in &files {
        let text = std::fs::read_to_string(f).unwrap();
        let Some(lf) = block(&text) else { continue };
        let crlf = block(&text.replace("\r\n", "\n").replace('\n', "\r\n")).expect("the CRLF copy has the same block");
        for (eol, b) in [("LF", &lf), ("CRLF", &crlf)] {
            match strict(b) {
                Ok(y) => {
                    for key in ["name", "description"] {
                        if y[key].as_str().is_none_or(|s| s.trim().is_empty()) {
                            bad.push(format!("{} ({eol}): no {key}", f.display()));
                        }
                    }
                }
                Err(e) => bad.push(format!("{} ({eol}): {e}", f.display())),
            }
        }
        checked.push(f.clone());
    }
    for want in ["agents/kb-scribe.md", "agents/kb-ref.md", "skills/kb-ingest/SKILL.md", "skills/kb-query/SKILL.md", "skills/fleet/SKILL.md"] {
        assert!(checked.iter().any(|f| f.ends_with(want)), "{want} was not checked: {checked:?}");
    }
    assert!(bad.is_empty(), "frontmatter that is not strict YAML:\n{}", bad.join("\n"));
}

/// What `write_frontmatter` writes reads back strictly, value for value,
/// whatever the value holds.
#[test]
fn the_frontmatter_nimitz_writes_is_strict_yaml() {
    let values = ["kb-scribe's procedure: ingest one note", "a: b # c", "--- the end", "\"quoted\" and \\ back", "line\nbreak", "[not, a, list]", "{not: a map}", "plain", "ünïcödé 🚢", "@at `tick` |pipe >gt %pct !bang &amp *star"];
    for v in values {
        let pairs = vec![("name".to_string(), "x".to_string()), ("description".to_string(), v.to_string())];
        let text = nimitz_core::harness::write_frontmatter(&pairs, "body\n");
        let y = strict(&block(&text).expect("a block")).unwrap_or_else(|e| panic!("{v:?}: {e}\n{text}"));
        assert_eq!(y["description"].as_str(), Some(v), "{text}");
        assert_eq!(y["name"].as_str(), Some("x"));
    }
}
