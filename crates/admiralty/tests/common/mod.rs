#![allow(dead_code)]
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};

use admiralty::env::Env;
use admiralty::model::{self, Snapshot};
use admiralty::sandbox;
use admiralty::sources::Cache;
use serde_json::Value;

static N: AtomicUsize = AtomicUsize::new(0);

pub struct Sb {
    pub base: PathBuf,
    pub env: Env,
}

impl Drop for Sb {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.base);
    }
}

/// A fresh sandbox world per test (tests run in parallel; nothing global is touched).
pub fn sandbox() -> Sb {
    let base = std::env::temp_dir().join(format!("aw-test-{}-{}", std::process::id(), N.fetch_add(1, Ordering::SeqCst)));
    let exe = PathBuf::from(env!("CARGO_BIN_EXE_admiralty"));
    sandbox::create(&base, &exe).expect("sandbox");
    let env = Env::from_vars(&sandbox::env_for(&base));
    Sb { base: base.canonicalize().unwrap(), env }
}

pub fn snap(sb: &Sb) -> Snapshot {
    model::snapshot(&sb.env, &mut Cache::default())
}

pub fn lines(p: &Path) -> Vec<Value> {
    std::fs::read_to_string(p)
        .unwrap_or_default()
        .lines()
        .filter_map(|l| serde_json::from_str(l).ok())
        .collect()
}
