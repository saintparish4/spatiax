#!/usr/bin/env bash
# Records the terminal demo in docs/demo/decode.gif: check the demo database,
# decode the demo lap to the terminal and to CSV, and plot it.
#
# Needs a release build, asciinema, agg (cargo install --git
# https://github.com/asciinema/agg), and a python3 with matplotlib on PATH.
# The session itself runs in a temporary directory, so nothing lands in the
# checkout except the GIF.
#
#   scripts/record_demo.sh
set -euo pipefail

COLS=96
ROWS=26

# The part that gets recorded: type each command as a person would, run it,
# then pause long enough to read the output.
session() {
    run 'spatiax check gt3.dbc' 2.5
    run 'spatiax decode gt3.dbc lap.log | head -n 12' 4
    run 'time spatiax decode --format csv gt3.dbc lap.log > lap.csv' 2.5
    run 'wc -l lap.csv' 2
    run 'python3 plot_lap.py lap.csv -o lap.png' 3
}

run() {
    printf '\033[1;32m$\033[0m '
    local command=$1 i
    for ((i = 0; i < ${#command}; i++)); do
        printf '%s' "${command:i:1}"
        sleep 0.035
    done
    sleep 0.3
    printf '\n'
    eval "$command"
    sleep "$2"
}

if [[ ${1:-} == --session ]]; then
    session
    exit 0
fi

root=$(cd "$(dirname "$0")/.." && pwd)
cd "$root"
cargo build --release --quiet

stage=$(mktemp -d)
trap 'rm -rf "$stage"' EXIT
cp fixtures/demo/gt3.dbc "$stage/gt3.dbc"
cp fixtures/demo/synthetic_lap.log "$stage/lap.log"
cp scripts/plot_lap.py "$stage/plot_lap.py"

export PATH="$root/target/release:$PATH"
mkdir -p docs/demo
(
    cd "$stage"
    asciinema rec --quiet --overwrite --cols "$COLS" --rows "$ROWS" \
        --command "bash '$root/scripts/record_demo.sh' --session" session.cast
)
agg --cols "$COLS" --rows "$ROWS" --font-size 16 --speed 1.0 \
    --theme 1a1a19,e8e6df,1a1a19,e34948,0ca30c,eda100,3987e5,e87ba4,1baf7a,c3c2b7 \
    "$stage/session.cast" docs/demo/decode.gif
echo "wrote docs/demo/decode.gif"
