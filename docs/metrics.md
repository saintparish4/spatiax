# Metrics

Every number this project publishes, what produces it, and the command that
reproduces it. The rule is the same one the README follows: **a number only
appears here if CI re-derives it, or if the line says plainly that it does
not.**

Snapshot taken 2026-09-20. Re-run the commands and the figures should come
back the same, up to the noise each row admits.

## Evidence

| Measure | Now | Produced by | Reproduce |
|---|---|---|---|
| Decoded values checked against `cantools` per CI run | 500,000 | `tests/differential.rs`, generated databases and frames | `SPATIAX_DIFFERENTIAL_CASES=500000 cargo test --test differential -- --nocapture` |
| Of those, values carrying a value-table label | ≥ 10,000 | same | same |
| Floor below which the differential test refuses to pass | 100,000 | `MINIMUM_CASES` | — |
| Property-test cases per property, in CI | 16,384 × 10 properties | `tests/properties.rs` | `PROPTEST_CASES=16384 cargo test --test properties` |
| Signal layouts checked bit-walk against fast path | > 1,000,000 | `decode::tests`, exhaustive over start bit × width × payload length | `cargo test --lib decode` |
| Reference vectors computed by hand | 12 | `tests/vectors.rs` | `cargo test --test vectors` |
| Real production databases parsed | 52 of 58 | `tests/corpus.rs` over `opendbc` | `bash scripts/fetch_dbc_corpus.sh && cargo test --test corpus -- --nocapture` |
| Messages read from the corpus | 3,470 | same | same |
| Signals read from the corpus | 25,044 | same | same |
| Corpus files compared field by field with `cantools` | 49 | same | same |
| Messages that comparison covers | 3,242 | same | same |
| Signals that comparison covers | 23,985 | same | same |
| Differences found in that comparison | 0 | same | same |
| Files where `dbc::check` and cantools strict mode agree | 49 of 49 | same (9 refused by both, 40 accepted by both) | same |
| `.ld` values read back by an independent reader | 70,993 across 43 channels | `tests/ld.rs` against `ldparser` | `bash scripts/fetch_ld_oracle.sh && cargo test --test ld -- --nocapture` |
| `.ld` golden bytes reproduced exactly | 4,276 | same, via `ldparser`'s own writer | same |
| Opened in MoTeC i2 Pro 1.1 | 2026-09-03, session read as 1:22.520 | done by hand, **not reproduced by CI** | install i2 and open an exported lap |

## The corpus, file by file

`tests/corpus.rs` writes `target/corpus-summary.md` on every run: totals,
the files it refused with the reason, and a row per database. CI publishes
that file in the `real DBC corpus` job summary. It is the source for every
corpus number above — nothing here is typed in by hand.

The six databases refused, and why:

| Database | Reason | `cantools` |
|---|---|---|
| `chrysler_cusw.dbc` | standard identifier wider than 11 bits | refuses it too |
| `fca_giorgio.dbc` | standard identifier wider than 11 bits | refuses it too |
| `gm_global_a_lowspeed.dbc` | standard identifier wider than 11 bits | refuses it too |
| `toyota_2017_ref_pt.dbc` | standard identifier wider than 11 bits | refuses it too |
| `vw_mqbevo.dbc` | standard identifier wider than 11 bits | refuses it too |
| `vw_pq.dbc` | a signal marked `m` with no page number | loads it, and silently cannot decode the four pages |

Three go the other way: `mazda_2017.dbc`, `psa_aee2010_r3.dbc` and
`toyota_radar_dsu_tssp.dbc` parse here and `cantools` refuses them, over
message and signal names that begin with a digit. That changes nothing
about how a frame decodes, so the parser stays lenient about it.

## Code and release

| Measure | Now | Reproduce |
|---|---|---|
| Rust lines (`src` + `tests` + `benches`) | 8,376 | `find src tests benches -name '*.rs' \| xargs wc -l \| tail -1` |
| — of which library and binary | 5,088 | `find src -name '*.rs' \| xargs wc -l \| tail -1` |
| — of which tests | 3,107 | `find tests -name '*.rs' \| xargs wc -l \| tail -1` |
| Test functions | 224 | `find src tests -name '*.rs' \| xargs grep -h '#\[test\]' \| wc -l` |
| Tests run by `cargo test --all-features` | 226 | `cargo test --all-features` |
| Runtime dependencies of the library | 1 (`thiserror`) | `cargo tree --no-default-features` |
| Optional dependencies | `clap` (CLI), `socketcan` (live capture) | `Cargo.toml` |
| Minimum supported Rust | 1.85, edition 2024 | the `msrv` CI job |
| CI jobs per push | 7 | `.github/workflows/ci.yml` |

## Performance

The full table, with the machine that produced it and the caveats that go
with a laptop, is in the README's [Performance](../README.md#performance)
section. The headline rows:

| Benchmark | Rate |
|---|---|
| `decode_frame/gt3_sample` | 25.6 M frames/s |
| `decode_message/EngineData` | 184.0 M signals/s |
| `candump/classic` | 5.1 M lines/s |

Re-run with `cargo bench --bench decode` and print the table with
`python3 scripts/bench_table.py`. Benchmark figures are the one place this
project does not treat a single run as a fact: clock behaviour on a laptop
moved every row by up to 38% between runs on untouched code, so a number
only replaces the published one after two quiet runs agree.

## History

Where the project has actually moved, release to release.

| Measure | 0.1.0 (2026-09-03) | 0.2.0 (2026-09-20) |
|---|---|---|
| Rust lines | 6,326 | 8,376 |
| Test functions | 189 | 224 |
| CI jobs | 5 | 7 |
| Differential values per run | 500,000 | 500,000 |
| Real databases parsed | none | 52 of 58 |
| Corpus signals agreeing with `cantools` | none | 23,985 |
| MoTeC `.ld` export | written, unreleased | released |
| `.ld` opened by MoTeC i2 | confirmed by hand | confirmed by hand |

The two line and test-function counts are `git`-reproducible at both
points; run the commands in the table above against `v0.1.0` and `v0.2.0`.
