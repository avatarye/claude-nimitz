#!/bin/sh
# Compatibility shim for a Herdr that still has the pre-0.5.0 manifest cached
# (its overlay runs ./bin/watch.sh): the overlay is `admiralty herdr watch` now.
# `herdr plugin link <repo>/herdr-plugin` again (or a Herdr restart) switches
# to the manifest's direct argv, and this file can go.
bin=${ADMIRALTY_BIN:-admiralty}
command -v "$bin" >/dev/null 2>&1 || bin=$HOME/.cargo/bin/admiralty
exec "$bin" herdr watch "$@"
