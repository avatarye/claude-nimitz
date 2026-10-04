//! Lean argument parsing: positionals, boolean flags, and value flags.
//! Each command names the flags it accepts; anything else is a usage error.
use std::collections::BTreeMap;

/// Flags that take a value (`--x v` or `--x=v`). All others are booleans.
const VALUE_FLAGS: &[&str] = &[
    "--into", "--dir", "--machine", "--prompt", "--prompt-file", "--mode", "--depends", "--brief", "--comms", "--label",
    "--admiral-mode", "--no-dir", "--hostname",
];

/// Flags every command accepts.
const GLOBAL_FLAGS: &[&str] = &["--json", "--plan", "--yes", "--force", "--help"];

#[derive(Debug, Default)]
pub struct Args {
    pub pos: Vec<String>,
    flags: BTreeMap<String, Vec<String>>,
}

impl Args {
    pub fn parse(argv: &[String]) -> Result<Args, String> {
        let mut a = Args::default();
        let mut it = argv.iter();
        let mut only_pos = false;
        while let Some(x) = it.next() {
            if only_pos || !x.starts_with("--") || x == "-" {
                a.pos.push(x.clone());
                continue;
            }
            if x == "--" {
                only_pos = true;
                continue;
            }
            let (name, inline) = match x.split_once('=') {
                Some((n, v)) => (n.to_string(), Some(v.to_string())),
                None => (x.clone(), None),
            };
            if VALUE_FLAGS.contains(&name.as_str()) {
                let v = match inline {
                    Some(v) => v,
                    None => it.next().cloned().ok_or(format!("{name} needs a value"))?,
                };
                a.flags.entry(name).or_default().push(v);
            } else {
                if inline.is_some() {
                    return Err(format!("{name} takes no value"));
                }
                a.flags.entry(name).or_default();
            }
        }
        Ok(a)
    }

    pub fn flag(&self, name: &str) -> bool {
        self.flags.contains_key(name)
    }

    pub fn value(&self, name: &str) -> Option<&str> {
        self.flags.get(name).and_then(|v| v.last()).map(String::as_str)
    }

    pub fn values(&self, name: &str) -> Vec<String> {
        self.flags.get(name).cloned().unwrap_or_default()
    }

    /// Refuse flags the command does not know (global flags are always allowed).
    pub fn allow(&self, known: &[&str]) -> Result<(), String> {
        for k in self.flags.keys() {
            if !known.contains(&k.as_str()) && !GLOBAL_FLAGS.contains(&k.as_str()) {
                return Err(format!("unknown flag {k}"));
            }
        }
        Ok(())
    }

    /// Positional `i`, or a usage error naming it.
    pub fn need(&self, i: usize, what: &str) -> Result<&str, String> {
        self.pos.get(i).map(String::as_str).ok_or(format!("missing {what}"))
    }

    /// Fail when there are more than `n` positionals.
    pub fn at_most(&self, n: usize) -> Result<(), String> {
        match self.pos.get(n) {
            Some(x) => Err(format!("unexpected argument {x:?}")),
            None => Ok(()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(xs: &[&str]) -> Args {
        Args::parse(&xs.iter().map(|s| s.to_string()).collect::<Vec<_>>()).unwrap()
    }

    #[test]
    fn values_booleans_and_positionals() {
        let a = p(&["MAIN/ARGOS", "--dir", "/x", "--yes", "--mode=auto", "--", "--not-a-flag"]);
        assert_eq!(a.pos, vec!["MAIN/ARGOS", "--not-a-flag"]);
        assert_eq!(a.value("--dir"), Some("/x"));
        assert_eq!(a.value("--mode"), Some("auto"));
        assert!(a.flag("--yes"));
        assert!(a.allow(&["--dir", "--mode"]).is_ok());
        assert!(a.allow(&["--dir"]).is_err());
    }

    #[test]
    fn a_value_flag_without_a_value_is_an_error() {
        assert!(Args::parse(&["--dir".to_string()]).is_err());
        assert!(Args::parse(&["--yes=1".to_string()]).is_err());
    }
}
