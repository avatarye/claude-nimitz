//! Opt-in (`NIMITZ_HERDR_E2E=1`): the overlay leaves zoom as it found it,
//! against a PRIVATE Herdr server. A wrapper script unsets every `HERDR_*` and
//! points XDG_CONFIG_HOME / XDG_STATE_HOME / XDG_RUNTIME_DIR at a temp dir, so
//! client, server, socket and plugin link all live there; the user's Herdr is
//! never asked anything. The overlay is a stand-in plugin pane labelled
//! `Admiralty` running `sh`, which exits when a stop file appears, as the watch
//! does after a jump; panes are never sent input.
#![cfg(unix)]
use admiralty::herdr_plugin::{fill_overlay_with, restore_zoom, settle_zoom_with, toggle_with, Fill, Toggled};
use serde_json::Value;
use std::collections::HashMap;
use std::path::Path;
use std::process::Command;
use std::time::{Duration, Instant};

#[test]
fn the_overlay_leaves_zoom_as_it_found_it_on_a_private_herdr() {
    if std::env::var("NIMITZ_HERDR_E2E").ok().as_deref() != Some("1") {
        println!("[WARN] skipped (set NIMITZ_HERDR_E2E=1 to run it against a private Herdr server)");
        return;
    }
    let td = tempfile::Builder::new().prefix("adm-zoom-").tempdir_in("/tmp").expect("tempdir");
    let tmp = td.path();
    std::fs::create_dir_all(tmp.join("config/herdr")).unwrap();
    std::fs::create_dir_all(tmp.join("run")).unwrap();
    std::fs::write(tmp.join("config/herdr/config.toml"), "onboarding = false\n").unwrap();
    let wrapper = tmp.join("herdr-iso");
    std::fs::write(
        &wrapper,
        format!(
            "#!/bin/sh\nfor v in $(env | sed -n 's/^\\(HERDR_[A-Za-z_0-9]*\\)=.*/\\1/p'); do unset \"$v\"; done\n\
             export XDG_CONFIG_HOME=\"{t}/config\" XDG_STATE_HOME=\"{t}/state\" XDG_RUNTIME_DIR=\"{t}/run\"\nexec herdr \"$@\"\n",
            t = tmp.display()
        ),
    )
    .unwrap();
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(&wrapper, std::fs::Permissions::from_mode(0o755)).unwrap();
    let sock = tmp.join("config/herdr/herdr.sock");
    // the wrapper must name our socket before a server is started through it
    let status = Command::new(&wrapper).arg("status").output().expect("herdr status");
    assert!(String::from_utf8_lossy(&status.stdout).contains(&*sock.to_string_lossy()), "not isolated");
    let mut server = Command::new(&wrapper).arg("server").stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null()).spawn().expect("start private herdr server");
    let r = std::panic::catch_unwind(|| body(&wrapper, &sock, tmp));
    // stop only a server whose socket is in our temp dir
    if let Ok(out) = Command::new(&wrapper).arg("status").output() {
        if String::from_utf8_lossy(&out.stdout).contains(&*sock.to_string_lossy()) {
            let _ = Command::new(&wrapper).args(["server", "stop"]).output();
        }
    }
    let _ = server.kill();
    let _ = server.wait();
    if let Err(p) = r {
        std::panic::resume_unwind(p);
    }
}

fn body(wrapper: &Path, sock: &Path, tmp: &Path) {
    let herdr = |a: &[&str]| -> Option<String> {
        let out = Command::new(wrapper).args(a).output().ok()?;
        out.status.success().then(|| String::from_utf8_lossy(&out.stdout).into_owned())
    };
    let api = |a: &[&str]| -> Value { herdr(a).and_then(|s| serde_json::from_str(&s).ok()).unwrap_or(Value::Null) };
    let end = Instant::now() + Duration::from_secs(15);
    while !herdr(&["status"]).is_some_and(|s| s.contains(&*sock.to_string_lossy()) && s.contains("status: running")) {
        assert!(Instant::now() < end, "the private herdr server did not come up");
        std::thread::sleep(Duration::from_millis(200));
    }
    // the stand-in overlay: plugin `probe`, entrypoint `watch`, label Admiralty
    let plugin = tmp.join("probe");
    std::fs::create_dir_all(&plugin).unwrap();
    let stop = tmp.join("stop");
    std::fs::write(
        plugin.join("herdr-plugin.toml"),
        format!(
            "id = \"probe\"\nname = \"Probe\"\nversion = \"0.0.1\"\nmin_herdr_version = \"0.9.0\"\ndescription = \"probe\"\nplatforms = [\"linux\", \"macos\"]\n\n\
             [[panes]]\nid = \"watch\"\ntitle = \"Admiralty\"\nplacement = \"overlay\"\ncommand = [\"sh\", \"-c\", \"while [ ! -f {} ]; do sleep 0.1; done\"]\n",
            stop.display()
        ),
    )
    .unwrap();
    herdr(&["plugin", "link", plugin.to_str().unwrap()]).expect("plugin link");
    // tab 1: p1 | p2; tab 2: p3 | p4; every pane an agent (`agent focus`)
    herdr(&["workspace", "create", "--label", "zoom-e2e", "--cwd", "/tmp"]).expect("workspace");
    herdr(&["pane", "split", "w1:p1", "--direction", "right", "--no-focus"]).expect("split");
    herdr(&["tab", "create", "--workspace", "w1", "--cwd", "/tmp", "--label", "two"]).expect("tab");
    herdr(&["pane", "split", "w1:p3", "--direction", "right", "--no-focus"]).expect("split");
    for p in ["w1:p1", "w1:p2", "w1:p3", "w1:p4"] {
        herdr(&["pane", "report-agent", "--source", "e2e", "--agent", "claude", "--state", "idle", p]).expect("report-agent");
    }
    let tab_of = |p: &str| -> (String, Option<String>) {
        let l = api(&["pane", "layout", "--pane", p]);
        let l = &l["result"]["layout"];
        (l["focused_pane_id"].as_str().unwrap_or_default().to_string(), (l["zoomed"] == true).then(|| l["focused_pane_id"].as_str().unwrap_or_default().to_string()))
    };
    let state = tmp.join("state-dir").join("pane_id");
    // (zoomed in tab 1 before, zoomed in tab 2 before, jump target or none)
    //   -> zoomed in tab 1 after, zoomed in tab 2 after
    let cases: [(Option<&str>, Option<&str>, Option<&str>, Option<&str>, Option<&str>); 7] = [
        (None, None, Some("w1:p2"), None, None),
        (Some("w1:p1"), None, Some("w1:p2"), None, None),
        (Some("w1:p1"), None, Some("w1:p1"), Some("w1:p1"), None),
        (None, None, None, None, None),
        (Some("w1:p1"), None, None, Some("w1:p1"), None),
        (Some("w1:p1"), None, Some("w1:p4"), Some("w1:p1"), None),
        // another tab's zoom is Herdr's: it follows that tab's focus, untouched
        (None, Some("w1:p3"), Some("w1:p4"), None, Some("w1:p4")),
    ];
    for (z1, z2, target, want1, want2) in cases {
        let _ = std::fs::remove_file(&stop);
        for (t, p, z) in [("w1:t2", "w1:p3", z2), ("w1:t1", "w1:p1", z1)] {
            herdr(&["tab", "focus", t]);
            herdr(&["agent", "focus", p]);
            herdr(&["pane", "zoom", p, "--off"]);
            if let Some(z) = z {
                herdr(&["pane", "zoom", z, "--on"]);
            }
        }
        let ctx = r#"{"tab_id":"w1:t1","focused_pane_id":"w1:p1"}"#;
        let vars: HashMap<String, String> = [("HERDR_PLUGIN_ID", "probe"), ("HERDR_TAB_ID", "w1:t1"), ("HERDR_PLUGIN_CONTEXT_JSON", ctx)].map(|(k, v)| (k.to_string(), v.to_string())).into();
        let Toggled::Opened(ov) = toggle_with(&vars, &state, &herdr) else { panic!("the overlay did not open") };
        let watch: HashMap<String, String> =
            [("HERDR_PANE_ID", ov.as_str()), ("HERDR_TAB_ID", "w1:t1"), ("HERDR_PLUGIN_STATE_DIR", state.parent().unwrap().to_str().unwrap())].map(|(k, v)| (k.to_string(), v.to_string())).into();
        assert_eq!(fill_overlay_with(&watch, &herdr), Fill::Filled);
        if let Some(t) = target {
            let tab = api(&["pane", "get", t])["result"]["pane"]["tab_id"].as_str().unwrap().to_string();
            herdr(&["workspace", "focus", "w1"]);
            herdr(&["tab", "focus", &tab]);
            herdr(&["agent", "focus", t]).expect("agent focus");
        }
        // the watch quits: settle, its process exits, then the detached rezoom
        settle_zoom_with(&watch, Fill::Filled, &herdr);
        std::fs::write(&stop, "").unwrap();
        let end = Instant::now() + Duration::from_secs(5);
        while herdr(&["pane", "get", &ov]).is_some() {
            assert!(Instant::now() < end, "the overlay did not exit");
            std::thread::sleep(Duration::from_millis(100));
        }
        let _ = std::fs::remove_file(&state);
        restore_zoom(&state, "w1:t1", &herdr);
        let got = (tab_of("w1:p1").1, tab_of("w1:p3").1);
        assert_eq!(got, (want1.map(String::from), want2.map(String::from)), "before {z1:?} {z2:?}, jump to {target:?}");
    }
}
