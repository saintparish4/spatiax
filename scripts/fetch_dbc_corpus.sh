#!/usr/bin/env bash
# Fetch the corpus of real production DBC files that tests/corpus.rs reads.
#
# comma.ai's `opendbc` is a few hundred databases for real vehicles, written
# by manufacturers and by the people reverse engineering them. None of them
# were written with this parser in mind, which is the entire point: every
# other database in this repository is a shape the project chose.
#
# It is fetched, never vendored — the corpus is a test input, not part of
# this crate, and pinning it by commit is what keeps a run reproducible. The
# commit hash is the integrity check: git verifies the objects it fetches
# against it, so there is no separate checksum to keep in step.
#
# Run it through bash rather than as an executable: this repository has
# core.filemode disabled, so no script in scripts/ carries the executable
# bit in git and CI invokes each one through its interpreter.
#
# Usage: bash scripts/fetch_dbc_corpus.sh [destination-directory]

set -euo pipefail

REPO=https://github.com/commaai/opendbc.git
# opendbc master as of 2026-09-19.
COMMIT=b128914adaf0b8138158985868802445c174a083

dest="${1:-.venv/corpus}"
checkout="$dest/opendbc"

mkdir -p "$checkout"
cd "$checkout"

if [ ! -d .git ]; then
    git init -q .
    git remote add origin "$REPO"
fi

# A shallow fetch of the one commit: the corpus is 4 MB of DBC text and the
# history behind it is not.
git fetch -q --depth 1 origin "$COMMIT"
git checkout -q FETCH_HEAD

count=$(find opendbc/dbc -maxdepth 1 -name '*.dbc' | wc -l)
echo "fetch_dbc_corpus: $count databases in $checkout/opendbc/dbc at ${COMMIT:0:7}"
