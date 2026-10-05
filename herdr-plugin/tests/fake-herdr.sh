#!/bin/sh
# Stand-in for `herdr`, enough for bin/toggle.sh. Open panes live in $FAKE_DIR/panes
# as "<pane_id> <tab_id> [label]" lines (label defaults to the watch's); every call is appended to $FAKE_DIR/calls.
# FAKE_OPEN_FAIL=1 makes `plugin pane open` fail.
set -u
dir=${FAKE_DIR:?}
touch "$dir/panes" "$dir/calls"
echo "$*" >> "$dir/calls"
case "$*" in
    "pane get "*)
        line=$(grep "^$3 " "$dir/panes") || { echo '{"error":{"code":"pane_not_found"}}' >&2; exit 1; }
        set -- $line "Admiralty"
        printf '{"id":"cli:pane:get","result":{"pane":{"label":"%s","pane_id":"%s","tab_id":"%s"},"type":"pane_info"}}\n' "$3" "$1" "$2"
        ;;
    "plugin pane close "*)
        grep -v "^$4 " "$dir/panes" > "$dir/panes.new"; mv "$dir/panes.new" "$dir/panes"
        echo '{"id":"cli:plugin","result":{"type":"plugin_pane_closed"}}'
        ;;
    "plugin pane open "*)
        [ "${FAKE_OPEN_FAIL:-0}" = 1 ] && { echo "boom" >&2; exit 1; }
        n=$(( $(cat "$dir/next" 2>/dev/null || echo 1) + 1 )); echo "$n" > "$dir/next"
        tab=${HERDR_TAB_ID:-w1:t1}
        echo "${tab%%:*}:p$n $tab" >> "$dir/panes"
        printf '{"id":"cli:plugin","result":{"plugin_pane":{"entrypoint":"watch","pane":{"pane_id":"%s","tab_id":"%s"},"plugin_id":"admiralty"},"type":"plugin_pane_opened"}}\n' "${tab%%:*}:p$n" "$tab"
        ;;
    *) echo "[fake herdr] unsupported: $*" >&2; exit 2 ;;
esac
