# Changelog

Notable changes to this project, newest first. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and the version
numbers follow [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- A corpus test against real production databases. `tests/corpus.rs` parses
  every DBC in comma.ai's `opendbc`, fetched at a pinned commit by
  `scripts/fetch_dbc_corpus.sh` and never vendored, and compares what it
  read with `cantools` field by field: identifier, DLC, start bit, width,
  byte order, sign, factor, offset and multiplexing role. 52 of the 58
  databases parse, and on the 49 both implementations accept the two agree
  on 3,242 messages and 23,985 signals with no differences. `dbc::check` is
  compared with cantools' strict mode the same way and reaches the same
  verdict on all 49. A file refused for a reason the crate does not
  document as a limit fails the run, and the new `real DBC corpus` CI job
  publishes the table the test writes to `target/corpus-summary.md`.
- `docs/metrics.md`: every published number, what produces it, the command
  that reproduces it, and how each has moved from 0.1.0 to now.

- MoTeC `.ld` export, as the `ld` module and a `spatiax export` subcommand.
  A decoded log is resampled onto one fixed grid — sample and hold, at the
  first standard rate at or above the fastest message unless `--rate` says
  otherwise — and written as `f32` channels with the format's calibration
  fields left at identity, so the stored word is the physical value.
  `--driver`, `--vehicle`, `--venue` and `--event` supply the session
  metadata i2 shows.
- A second oracle, `ldparser`, pinned by commit and checksum and fetched by
  `scripts/fetch_ld_oracle.sh`. CI checks the exporter against it twice: it
  reads back every value of an exported lap, and its own writer reproduces
  `fixtures/gt3_sample.ld` byte for byte, which covers the fields a reader
  discards. `fixtures/gt3_sample.ld` is also a byte-level golden file.
- MoTeC i2 Pro 1.1 opens an exported lap and reads a 1:22.520 session from
  the sample count and rate in the channel headers, and generates its own
  `.ldx` for it. Channel traces were compared on an i2 graph the same day:
  `EngineRPM` peaks at 8600 rpm and `Speed` at 258 km/h, as the log has
  them. i2's stock worksheets look for MoTeC's standard channel names
  rather than the DBC's, so only `Gear` appears without adding channels by
  hand.
- `Error::Export`, for a session that cannot be built from the log and
  database given.

### Fixed

- `VECTOR__INDEPENDENT_SIG_MSG`, the container CANdb++ puts signals in when
  they belong to no message, is dropped with its signals rather than
  refused over the marker identifier it carries. `cantools` drops it too.
  Two `opendbc` databases could not be read before this.
- A `VAL_` record left without its `;` no longer swallows the record on the
  next line. It is taken as written — what the parser already did at the
  end of a file — and the next record is read normally. One `opendbc`
  database was refused over what amounts to a decorative line.

### Changed

- A signal marked `m` with no page number is still refused, but now says
  so: the error names the signal and both spellings it could have meant,
  instead of reporting an empty multiplexor selector.

## [0.1.0] — 2026-09-03

First release.

### Added

- DBC parsing of `BO_`, `SG_` and `VAL_` records, extended identifiers, and
  simple multiplexing (`M` / `m<N>`). Extended multiplexing is rejected at
  parse time rather than decoded wrongly.
- Bit-exact extraction and insertion for both byte orders, signed and
  unsigned, with factor and offset scaling. The decode path allocates
  nothing.
- Value-table labels, on `Decoded::label`.
- CAN FD payloads up to 64 bytes, and the data length codes that describe
  them — including the classic case of a code above the eight bytes a frame
  carries.
- `candump` log replay (`candump::LogReader`). A malformed line is reported
  with its number and reading carries on, so one corrupt record cannot hide
  the rest of a session.
- `dbc::check`: the two layout problems `cantools` refuses to load in strict
  mode — a signal that runs past its message's DLC, and two signals that can
  decode together but share a bit.
- Live capture from a SocketCAN interface (`live::Capture`), Linux only,
  behind the `socketcan` feature, with an optional read timeout so a bus
  going quiet ends the session instead of blocking forever.
- The `spatiax` binary — `decode`, `check` and `live`, in text or CSV, with
  exit codes following `grep` — behind the default `cli` feature. Library
  users can turn it off to keep `clap` out of their dependency tree.
- Benchmarks over the decode path (`cargo bench`), published in the README
  from what criterion measured and re-run by CI on every push.

### Evidence

The correctness claims above are held up by hand-computed reference vectors,
property tests over every signal layout from 1 to 64 bits, and a differential
test that decodes generated databases with both this crate and `cantools` and
requires them to agree — 500,000 signal values per CI run. All of it runs in
CI, together with a live capture over a virtual CAN interface.

[Unreleased]: https://github.com/saintparish4/spatiax/compare/v0.1.0...HEAD
[0.1.0]: https://github.com/saintparish4/spatiax/releases/tag/v0.1.0
