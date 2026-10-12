//! Perforce-style paths (docs/design/cli-ux.md section 1): a path names one
//! member (`F` the admiral, `F/SQ` the commodore, `F/SQ/SHIP` a ship), or,
//! with `...` as its last segment, everything under it, the lead included
//! (`F/...`, `F/SQ/...`); `//...` (or `/...`, which MSYS makes of it) is
//! every fleet but the ADMIRALTY workspace. Several paths per command; a
//! member matched twice counts once.
use crate::types::*;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Path {
    /// one member
    Member(Id),
    /// `F/...`, `F/SQ/...` (`Some`), or `//...` (`None`)
    Under(Option<Id>),
}

impl std::fmt::Display for Path {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Path::Member(id) => write!(f, "{id}"),
            Path::Under(Some(id)) => write!(f, "{id}/..."),
            Path::Under(None) => write!(f, "//..."),
        }
    }
}

/// What a path looks like, for the usage errors.
pub const FORMS: &str = "F, F/SQ, F/SQ/SHIP, F/..., F/SQ/... or //...; names are [A-Z0-9_]+";

impl Path {
    pub fn parse(s: &str) -> Result<Path, String> {
        if s == "//..." || s == "/..." {
            return Ok(Path::Under(None));
        }
        let bad = || format!("{s:?} is not a path ({FORMS})");
        let (head, under) = match s.strip_suffix("/...") {
            Some(h) => (h, true),
            None => (s, false),
        };
        let id = Id(head.to_string());
        if !id.well_formed() {
            return Err(bad());
        }
        Ok(if under { Path::Under(Some(id)) } else { Path::Member(id) })
    }

    /// Does this path cover `id`?
    pub fn covers(&self, id: &Id) -> bool {
        match self {
            Path::Member(t) => id == t,
            Path::Under(Some(t)) => id == t || id.0.starts_with(&format!("{}/", t.0)),
            Path::Under(None) => id.fleet() != crate::restore::ADMIRALTY_WORKSPACE,
        }
    }

    /// The member an exact path names.
    pub fn member(&self) -> Option<&Id> {
        match self {
            Path::Member(id) => Some(id),
            Path::Under(_) => None,
        }
    }

    /// Does this path cover all of fleet `f` (`F/...`, `//...`)?
    pub fn whole_fleet(&self, f: &str) -> bool {
        match self {
            Path::Under(None) => f != crate::restore::ADMIRALTY_WORKSPACE,
            Path::Under(Some(t)) => t.0 == f,
            Path::Member(_) => false,
        }
    }
}

/// Parse every path (at least one).
pub fn parse_all<S: AsRef<str>>(args: &[S]) -> Result<Vec<Path>, String> {
    if args.is_empty() {
        return Err(format!("missing path ({FORMS})"));
    }
    args.iter().map(|a| Path::parse(a.as_ref())).collect()
}

/// Does any of `paths` cover `id`?
pub fn covered(paths: &[Path], id: &Id) -> bool {
    paths.iter().any(|p| p.covers(id))
}

/// The members `paths` match, in config order, each once. A path that
/// matches nothing (an unknown member, an empty `...`) is a refusal that
/// lists what is there.
pub fn resolve(snap: &Snapshot, paths: &[Path]) -> Result<Vec<Id>, Issue> {
    let all: Vec<&Id> = snap.fleets.iter().flat_map(|f| f.members()).map(|m| &m.id).collect();
    for p in paths {
        if !all.iter().any(|id| p.covers(id)) {
            return Err(unknown(snap, p));
        }
    }
    Ok(all.into_iter().filter(|id| covered(paths, id)).cloned().collect())
}

/// `member.unknown` for a path that matches nothing, with what is there:
/// the fleets for `//...` or a fleet that is missing, else the members of
/// the fleet the path names.
pub fn unknown(snap: &Snapshot, p: &Path) -> Issue {
    let fleet = match p {
        Path::Member(id) | Path::Under(Some(id)) => snap.fleets.iter().find(|f| f.id.fleet() == id.fleet()),
        Path::Under(None) => None,
    };
    let there = match fleet {
        Some(f) => format!("{} has {}", f.id, f.members().map(|m| m.id.0.clone()).collect::<Vec<_>>().join(", ")),
        None if snap.fleets.is_empty() => "there are no fleets".to_string(),
        None => format!("the fleets are {}", snap.fleets.iter().map(|f| f.id.0.clone()).collect::<Vec<_>>().join(", ")),
    };
    Issue { code: "member.unknown".into(), severity: Severity::Error, message: format!("{p} matches no member; {there}"), fix: Some("nimitz status".into()), id: p.member().cloned() }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paths_parse_and_cover() {
        let p = |s: &str| Path::parse(s).unwrap();
        assert_eq!(p("MAIN"), Path::Member(Id::admiral("MAIN")));
        assert_eq!(p("MAIN/ARGOS/..."), Path::Under(Some(Id::commodore("MAIN", "ARGOS"))));
        assert_eq!(p("//..."), Path::Under(None));
        assert_eq!(p("/..."), Path::Under(None), "MSYS rewrites a leading //");
        for bad in ["", "...", "main", "MAIN/", "MAIN/A/B/C", "MAIN/*", "MAIN/.../X", "//MAIN", "MAIN/..../"] {
            assert!(Path::parse(bad).is_err(), "{bad}");
        }
        let sq = p("MAIN/ARGOS/...");
        assert!(sq.covers(&Id::commodore("MAIN", "ARGOS")) && sq.covers(&Id::ship("MAIN", "ARGOS", "X")));
        assert!(!sq.covers(&Id::admiral("MAIN")) && !sq.covers(&Id::ship("MAIN", "ARGOSL", "X")));
        assert!(p("MAIN").covers(&Id::admiral("MAIN")) && !p("MAIN").covers(&Id::commodore("MAIN", "ARGOS")));
        assert!(p("//...").covers(&Id::ship("BETA", "S", "X")) && !p("//...").covers(&Id::admiral("ADMIRALTY")));
        assert!(p("MAIN/...").whole_fleet("MAIN") && !sq.whole_fleet("MAIN"));
        assert_eq!(p("MAIN/ARGOS/...").to_string(), "MAIN/ARGOS/...");
        assert!(parse_all::<&str>(&[]).is_err());
    }
}
