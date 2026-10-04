#!/bin/sh
# Toggle the admiralty-watch overlay.
#   open in this tab       -> close it
#   open in another tab    -> close it there, open it here
#   not open               -> open it here, focused
# The id of the pane this script opened is kept in the plugin state dir. A stale id
# (the watch quit on its own) fails `pane get`; one that now names some other pane
# (ids can restart with the server) fails the label check and is left alone.
set -u

herdr=${HERDR_BIN_PATH:-herdr}
plugin=${HERDR_PLUGIN_ID:-admiralty-watch}
state_dir=${HERDR_PLUGIN_STATE_DIR:-${TMPDIR:-/tmp}/herdr-admiralty-watch}
state=$state_dir/pane_id
label="Admiralty Watch" # the [[panes]] title, which Herdr reports as the pane label

mkdir -p "$state_dir" || exit 1

# the "key":"value" string field of a one-object JSON reply (no jq dependency)
field() {
    printf '%s\n' "$2" | sed -n "s/.*\"$1\"[[:space:]]*:[[:space:]]*\"\([^\"]*\)\".*/\1/p" | head -n 1
}

if [ -s "$state" ]; then
    id=$(cat "$state")
    rm -f "$state"
    if info=$("$herdr" pane get "$id" 2>/dev/null) && [ "$(field label "$info")" = "$label" ]; then
        "$herdr" plugin pane close "$id" >/dev/null || exit 1
        here=$(field tab_id "${HERDR_PLUGIN_CONTEXT_JSON:-}")
        here=${here:-${HERDR_TAB_ID:-}}
        if [ -z "$here" ] || [ "$(field tab_id "$info")" = "$here" ]; then
            exit 0
        fi
    fi
fi

out=$("$herdr" plugin pane open --plugin "$plugin" --entrypoint watch --focus) || {
    echo "[ERROR] herdr plugin pane open failed" >&2
    exit 1
}
id=$(field pane_id "$out")
if [ -z "$id" ]; then
    echo "[WARN] opened the watch but found no pane_id in: $out" >&2
    exit 0
fi
printf '%s\n' "$id" > "$state"
