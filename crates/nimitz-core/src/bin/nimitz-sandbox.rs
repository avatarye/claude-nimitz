//! Test-only stand-in for the `nimitz` CLI, so the sandbox and its fake `claude`
//! can be driven before `crates/nimitz` exists. Owner: core-sessions.
//!
//! ```text
//! nimitz-sandbox create <base>              # build the world (bin/claude points back here)
//! nimitz-sandbox __fake-claude <base> ...   # the fake claude (what bin/claude execs)
//! nimitz-sandbox hook <Event>               # the hook binary (what the fake claude runs)
//! ```
//!
//! When `crates/nimitz` lands, `nimitz sandbox create`, `nimitz __fake-claude`
//! and `nimitz hook` replace this binary unchanged; tests only swap the path.
use std::io::Read;
use std::path::Path;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let code = match args.first().map(String::as_str) {
        Some("create") => match args.get(1) {
            Some(base) => {
                let exe = std::env::current_exe().unwrap_or_default();
                match nimitz_core::sandbox::create(Path::new(base), &exe) {
                    Ok(()) => {
                        println!("[OK] sandbox at {base}");
                        0
                    }
                    Err(e) => {
                        eprintln!("[ERROR] {e}");
                        1
                    }
                }
            }
            None => usage(),
        },
        Some("__fake-claude") => match args.get(1) {
            Some(base) => nimitz_core::sandbox::fake_claude(Path::new(base), &args[2..]),
            None => usage(),
        },
        Some("hook") => {
            let event = args.get(1).cloned().unwrap_or_default();
            let mut stdin = String::new();
            let _ = std::io::stdin().read_to_string(&mut stdin);
            let vars: std::collections::HashMap<String, String> = std::env::vars().collect();
            let env = nimitz_core::env::Env::from_vars(&vars);
            let out = nimitz_core::hook::handle(&env, &event, &stdin, &vars);
            if !out.is_empty() {
                println!("{out}");
            }
            0 // a hook never fails the session
        }
        _ => usage(),
    };
    std::process::exit(code);
}

fn usage() -> i32 {
    eprintln!("usage: nimitz-sandbox create <base> | __fake-claude <base> <args...> | hook <Event>");
    2
}
