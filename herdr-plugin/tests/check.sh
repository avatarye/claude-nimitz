#!/bin/sh
# Static and offline checks. Never touches the running Herdr: every herdr call runs
# with HERDR_* unset and XDG_CONFIG_HOME / XDG_STATE_HOME pointed at a temp dir.
#   1. scripts parse and are executable
#   2. the manifest links into a throwaway Herdr config (Herdr's own validation)
#   3. the README keybinding snippet passes `herdr config check`
#   4. bin/toggle.sh logic, against tests/fake-herdr.sh
set -u
root=$(cd "$(dirname "$0")/.." && pwd)
tmp=$(mktemp -d "${TMPDIR:-/tmp}/haw-check.XXXXXX")
trap 'rm -rf "$tmp"' EXIT
fails=0
ok() { echo "[OK] $*"; }
bad() { echo "[ERROR] $*"; fails=$((fails + 1)); }

# 1
for f in bin/toggle.sh bin/watch.sh tests/fake-herdr.sh tests/check.sh; do
    if sh -n "$root/$f" && [ -x "$root/$f" ]; then ok "$f parses, executable"; else bad "$f"; fi
done

iso() {
    env -u HERDR_SOCKET_PATH -u HERDR_CLIENT_SOCKET_PATH -u HERDR_ENV -u HERDR_SESSION \
        -u HERDR_PANE_ID -u HERDR_TAB_ID -u HERDR_WORKSPACE_ID -u HERDR_BIN_PATH \
        XDG_CONFIG_HOME="$tmp/config" XDG_STATE_HOME="$tmp/state" "$@"
}

if command -v herdr >/dev/null 2>&1; then
    # 2
    mkdir -p "$tmp/config/herdr"
    if out=$(iso herdr plugin link "$root" 2>&1) \
        && echo "$out" | grep -q '"type":"plugin_linked"' \
        && echo "$out" | grep -q '"id":"toggle"' \
        && echo "$out" | grep -q '"placement":"overlay"' \
        && ! echo "$out" | grep -q '"warnings"'; then
        ok "manifest links (action toggle, overlay pane watch, no warnings)"
    else
        bad "manifest link: $out"
    fi
    # 3: the snippet is the toml block after "keys.command" in README.md
    sed -n '/^\[\[keys.command\]\]/,/^description/p' "$root/README.md" > "$tmp/config/herdr/config.toml"
    if grep -q 'admiralty-watch.toggle' "$tmp/config/herdr/config.toml" \
        && out=$(iso herdr config check 2>&1) && echo "$out" | grep -q '^config: ok'; then
        ok "README keybinding passes herdr config check"
    else
        bad "keybinding snippet: $out"
    fi
else
    echo "[WARN] herdr not on PATH: skipped manifest and config checks"
fi

# 4
run() { # tab -> runs toggle.sh in that tab against the fake
    HERDR_BIN_PATH="$root/tests/fake-herdr.sh" FAKE_DIR="$tmp/fake" \
        HERDR_PLUGIN_STATE_DIR="$tmp/fake/state" HERDR_TAB_ID="$1" \
        HERDR_PLUGIN_CONTEXT_JSON="{\"tab_id\":\"$1\"}" \
        "$root/bin/toggle.sh" 2>"$tmp/fake/stderr"
}
expect() { # label, expected panes file contents, expected state
    got_p=$(tr '\n' ',' < "$tmp/fake/panes"); got_s=$(cat "$tmp/fake/state/pane_id" 2>/dev/null)
    if [ "$got_p" = "$2" ] && [ "$got_s" = "$3" ]; then ok "toggle: $1"; else
        bad "toggle: $1: panes [$got_p] want [$2], state [$got_s] want [$3]"; fi
}
rm -rf "$tmp/fake"; mkdir -p "$tmp/fake"; : > "$tmp/fake/panes"
run w1:t1; expect "opens when closed" "w1:p2 w1:t1," "w1:p2"
run w1:t1; expect "closes when open in this tab" "" ""
run w1:t1; run w1:t2; expect "moves to the calling tab" "w1:p4 w1:t2," "w1:p4"
: > "$tmp/fake/panes"   # the watch quit on its own (q, or a jump)
run w1:t2; expect "stale id opens a new one" "w1:p5 w1:t2," "w1:p5"
printf 'w1:p5 w1:t2 zsh\n' > "$tmp/fake/panes"   # the id now names someone else's pane
run w1:t2; expect "never closes a pane that is not the watch" "w1:p5 w1:t2 zsh,w1:p6 w1:t2," "w1:p6"
: > "$tmp/fake/panes"; rm -f "$tmp/fake/state/pane_id"
export FAKE_OPEN_FAIL=1
if run w1:t1; then bad "toggle: open failure should exit non-zero"; else
    expect "open failure leaves no state" "" ""; fi
unset FAKE_OPEN_FAIL

if [ "$fails" -eq 0 ]; then echo "[OK] all checks passed"; else echo "[ERROR] $fails check(s) failed"; exit 1; fi
