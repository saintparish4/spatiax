#!/usr/bin/env bash
# Fetch the independent `.ld` reader the export tests compare against.
#
# `ldparser` is GPL-3.0 and this crate is MIT, so it is never vendored into
# this tree. It is also a bare module with no packaging, so pip cannot
# install it. Both facts point at the same answer: download the single file,
# pinned by commit and verified by checksum, into the gitignored directory
# where the cantools oracle's interpreter already lives.
#
# Run it through bash rather than as an executable: this repository has
# core.filemode disabled, so no script in scripts/ carries the executable
# bit in git and CI invokes each one through its interpreter.
#
# Usage: bash scripts/fetch_ld_oracle.sh [destination-directory]

set -euo pipefail

COMMIT=57935b7d7b15cce2532a593afba66728dc0927fe
SHA256=df453e0b74309bc425f92e659f7d9bedc99a74496bbe961b2c279817961f56d5
URL="https://raw.githubusercontent.com/gotzl/ldparser/${COMMIT}/ldparser.py"

dest="${1:-.venv/oracle}"
mkdir -p "$dest"
target="$dest/ldparser.py"

curl -sSfL --max-time 60 -o "$target" "$URL"

if ! echo "$SHA256  $target" | sha256sum --check --status; then
    rm -f "$target"
    echo "fetch_ld_oracle: checksum mismatch for $URL" >&2
    exit 1
fi

echo "fetch_ld_oracle: $target at ${COMMIT:0:7}"
