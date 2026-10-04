#!/bin/sh
# Overlay entrypoint. The overlay closes when admiralty-watch exits.
#   ADMIRALTY_WATCH_BIN               binary to run (default: PATH, then ~/.cargo/bin)
#   ADMIRALTY_WATCH_ARGS              extra arguments, word-split (e.g. "--sandbox /tmp/aw-sb");
#                                     when unset, read from $HERDR_PLUGIN_CONFIG_DIR/args
#   ADMIRALTY_WATCH_EXIT_AFTER_JUMP   1 (default here): close the overlay once enter jumps to a pane

bin=${ADMIRALTY_WATCH_BIN:-}
if [ -z "$bin" ]; then
    if command -v admiralty-watch >/dev/null 2>&1; then
        bin=admiralty-watch
    elif [ -x "$HOME/.cargo/bin/admiralty-watch" ]; then
        bin=$HOME/.cargo/bin/admiralty-watch
    else
        echo "[ERROR] admiralty-watch not found on PATH or in ~/.cargo/bin."
        echo "        Install it (cargo install --path <admiralty-watch repo>) or set ADMIRALTY_WATCH_BIN."
        printf 'Press enter to close. '
        read -r _
        exit 127
    fi
fi

if [ -z "${ADMIRALTY_WATCH_ARGS+set}" ] && [ -r "${HERDR_PLUGIN_CONFIG_DIR:-/nonexistent}/args" ]; then
    ADMIRALTY_WATCH_ARGS=$(cat "$HERDR_PLUGIN_CONFIG_DIR/args")
fi

ADMIRALTY_WATCH_EXIT_AFTER_JUMP=${ADMIRALTY_WATCH_EXIT_AFTER_JUMP:-1}
export ADMIRALTY_WATCH_EXIT_AFTER_JUMP
# shellcheck disable=SC2086 # ADMIRALTY_WATCH_ARGS is a word list on purpose
exec "$bin" ${ADMIRALTY_WATCH_ARGS:-}
