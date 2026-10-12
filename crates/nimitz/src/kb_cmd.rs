//! `nimitz kb ...`: the fleet's knowledge base on this machine (its folder
//! from the fleet's settings, `nimitz_core::fleet_settings::kb`) and its git
//! (`nimitz_core::kb`). The kb-scribe and kb-ref subagents run these inside
//! a fleet member's session, so the fleet is the member's own by default;
//! `--fleet F` names any.
use crate::out;
use crate::Ctx;
use nimitz_core::fleet_settings;
use nimitz_core::kb::{self, Committed};

/// The fleet a command is about: `--fleet F`, else a member's own.
/// `Err(Ok(code))`: refused, said already.
pub fn fleet_of(ctx: &Ctx) -> Result<String, Result<i32, String>> {
    match (nimitz_core::sessions::verified(&ctx.caller), ctx.args.value("--fleet")) {
        (_, Some(f)) => crate::members::fleet_name(f, "--fleet names the one fleet whose settings, knowledge base or harness it is").map_err(Err),
        (Some(id), None) => Ok(id.fleet().to_string()),
        (None, None) => Err(Err("name the fleet: --fleet F (inside a fleet member's session it is the member's own)".into())),
    }
}

/// A comma list (`--paths a,b`), each part trimmed, empty ones dropped.
fn list(v: Option<&str>) -> Vec<String> {
    v.unwrap_or("").split(',').map(str::trim).filter(|s| !s.is_empty()).map(str::to_string).collect()
}

fn print(text: &str) {
    let t = text.trim_end_matches('\n');
    if !t.is_empty() {
        println!("{t}");
    }
}

pub fn kb(ctx: &Ctx) -> Result<i32, String> {
    let verb = ctx.args.need(0, "path|status|head|show|grep|hash|commit|restore|sync|clone")?.to_string();
    let fleet = match fleet_of(ctx) {
        Ok(f) => f,
        Err(done) => return done,
    };
    if verb == "clone" {
        return clone(ctx, &fleet);
    }
    let k = fleet_settings::kb(&ctx.env, &fleet)?;
    let dir = &k.dir;
    if !dir.join(".git").exists() {
        return Err(format!("fleet {fleet}'s knowledge base is not cloned on this machine ({}); the user runs: nimitz kb clone --fleet {fleet} --yes", dir.display()));
    }
    // the folder is the KB's clone, not another repo the settings point at
    if let Some(remote) = &k.remote {
        let got = kb::origin(dir);
        if !got.as_deref().is_some_and(|g| kb::same_remote(g, remote)) {
            return Err(format!("{} is no clone of the fleet's kb.remote {remote} (its origin is {}): nimitz works only in the KB's own clone", dir.display(), got.as_deref().unwrap_or("none")));
        }
    }
    let res: Result<String, String> = match verb.as_str() {
        "path" => {
            ctx.args.allow(&["--fleet"])?;
            ctx.args.at_most(1)?;
            Ok(dir.to_string_lossy().into_owned())
        }
        "status" => {
            ctx.args.allow(&["--fleet"])?;
            ctx.args.at_most(1)?;
            kb::status(dir)
        }
        "head" => {
            ctx.args.allow(&["--fleet"])?;
            ctx.args.at_most(1)?;
            kb::head(dir)
        }
        "show" => {
            ctx.args.allow(&["--fleet", "--at"])?;
            ctx.args.at_most(2)?;
            kb::show(dir, ctx.args.value("--at"), ctx.args.need(1, "the path")?)
        }
        "grep" => {
            ctx.args.allow(&["--fleet", "--at"])?;
            ctx.args.at_most(3)?;
            kb::grep(dir, ctx.args.value("--at"), ctx.args.need(1, "the term")?, ctx.args.pos.get(2).map(String::as_str))
        }
        "hash" => {
            ctx.args.allow(&["--fleet"])?;
            ctx.args.at_most(2)?;
            kb::hash(dir, ctx.args.need(1, "the path")?)
        }
        "commit" => {
            ctx.args.allow(&["--fleet", "--title", "--paths", "--pages", "--note", "--from", "--no-sync"])?;
            ctx.args.at_most(1)?;
            let note = ctx.args.value("--note").ok_or("commit needs --note <sha256> (nimitz kb hash <capture>)")?;
            let from = ctx.args.value("--from").ok_or("commit needs --from \"<sender kind> <sender id>\"")?;
            let paths = list(ctx.args.value("--paths"));
            match kb::commit(&ctx.env, dir, ctx.args.value("--title").unwrap_or(""), &paths, &list(ctx.args.value("--pages")), note, from)? {
                Committed::Already(sha) => Ok(format!("already committed {sha}")),
                Committed::New(sha) => {
                    println!("committed {sha}");
                    if !ctx.args.flag("--no-sync") {
                        sync_line(ctx, dir);
                    }
                    Ok(String::new())
                }
            }
        }
        "restore" => {
            ctx.args.allow(&["--fleet", "--paths"])?;
            ctx.args.at_most(1)?;
            let backup = ctx.env.state.join("kb-restore").join(&fleet).join(format!("{:.0}-{}", nimitz_core::evidence::now(), std::process::id()));
            kb::restore(&ctx.env, dir, &list(ctx.args.value("--paths")), &backup).map(|done| done.join("\n"))
        }
        "sync" => {
            ctx.args.allow(&["--fleet"])?;
            ctx.args.at_most(1)?;
            kb::sync(&ctx.env, dir).map(|s| format!("[OK] {s}"))
        }
        v => return Err(format!("kb {v}: use path, status, head, show, grep, hash, commit, restore, sync or clone")),
    };
    match res {
        Ok(text) => {
            print(&text);
            Ok(0)
        }
        Err(e) => {
            out::error(&format!("kb {verb} ({fleet}): {e}"));
            Ok(1)
        }
    }
}

/// After a commit: push it (a failure leaves the commit, said as a warning).
fn sync_line(ctx: &Ctx, dir: &std::path::Path) {
    match kb::sync(&ctx.env, dir) {
        Ok(s) => out::ok(&s),
        Err(e) => out::warn(&format!("not synced: {e}; the commit stands (nimitz kb sync retries)")),
    }
}

/// `nimitz kb clone --fleet F`: the remote into this machine's
/// folder, both from the fleet's settings; `--plan` says what it would do.
fn clone(ctx: &Ctx, fleet: &str) -> Result<i32, String> {
    ctx.args.allow(&["--fleet"])?;
    ctx.args.at_most(1)?;
    let k = fleet_settings::kb(&ctx.env, fleet)?;
    let remote = k.remote.clone().ok_or(format!("fleet {fleet}'s knowledge base names no remote: nimitz settings set kb.remote <git url> --fleet {fleet}"))?;
    if k.dir.join(".git").exists() {
        out::ok(&format!("{} is cloned already", k.dir.display()));
        return Ok(0);
    }
    out::line(&format!("clone {remote} into {} (fleet {fleet}'s knowledge base on {})", k.dir.display(), k.label));
    if ctx.plan_only() {
        return Ok(0);
    }
    if !ctx.yes() {
        out::error("pass --yes to clone, or --plan to only show it");
        return Ok(1);
    }
    kb::clone(&remote, &k.dir).map_err(|e| format!("clone: {e}"))?;
    out::ok(&format!("cloned into {}", k.dir.display()));
    Ok(0)
}

/// `nimitz settings get [<key>]` / `set <key> <value>` (`null` removes it):
/// the fleet's settings.json (`--fleet F`, a member's default its own),
/// read, or changed planned.
pub fn settings(ctx: &Ctx) -> Result<i32, String> {
    let verb = ctx.args.need(0, "get|set")?.to_string();
    ctx.args.allow(&["--fleet"])?;
    let fleet = match fleet_of(ctx) {
        Ok(f) => f,
        Err(done) => return done,
    };
    match verb.as_str() {
        "get" => {
            ctx.args.at_most(2)?;
            let f = nimitz_core::config::load_fleet(&ctx.env, &fleet).map_err(|i| i.message)?;
            let m = fleet_settings::load(&f)?;
            let v = match ctx.args.pos.get(1) {
                None => serde_json::Value::Object(m),
                Some(key) => match fleet_settings::get(&m, key)? {
                    Some(v) => v.clone(),
                    None => {
                        out::error(&format!("fleet {fleet} has no setting {key}"));
                        return Ok(1);
                    }
                },
            };
            match &v {
                serde_json::Value::String(s) => println!("{s}"),
                v => println!("{}", serde_json::to_string_pretty(v).unwrap_or_default()),
            }
            Ok(0)
        }
        "set" => {
            ctx.args.at_most(3)?;
            let key = ctx.args.need(1, "the key (e.g. kb.remote)")?;
            let value = fleet_settings::parse_value(ctx.args.need(2, "the value (JSON, or text; null removes the key)")?);
            let plan = fleet_settings::plan_set(&ctx.env, &fleet, key, (!value.is_null()).then_some(value));
            Ok(crate::crud::config_plan(ctx, plan))
        }
        v => Err(format!("settings {v}: use get or set")),
    }
}
