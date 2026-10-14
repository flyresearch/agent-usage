#!/bin/sh
# Local convenience entrypoint; source selection always comes from private config.
set -eu
project_dir=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
binary="$project_dir/target/release/agent-usage"
config="${AGENT_USAGE_CONFIG:-${XDG_CONFIG_HOME:-$HOME/.config}/agent-usage/config.json}"
state="${AGENT_USAGE_STATE_DIR:-${XDG_DATA_HOME:-$HOME/.local/share}/agent-usage}"
if [ ! -x "$binary" ]; then
    echo 'Build first: cargo build --release --locked' >&2
    echo '{"schema_version":1,"complete":false,"error":"binary_missing"}'
    exit 1
fi
if [ "$#" -gt 1 ]; then
    echo '{"schema_version":1,"complete":false,"error":"invalid_argument"}'
    exit 1
fi
case "${1:-refresh}" in
    refresh) exec "$binary" collect --config "$config" --cache "$state/cache.json" --output "$state/snapshot.json" --summary ;;
    report) exec "$binary" report --snapshot "$state/snapshot.json" ;;
    help|--help) exec "$binary" --help ;;
    *) echo '{"schema_version":1,"complete":false,"error":"invalid_command"}'; exit 1 ;;
esac
