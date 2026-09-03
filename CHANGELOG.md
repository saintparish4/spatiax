# Changelog

Notable changes to this project, newest first. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and the version
numbers follow [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

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

[Unreleased]: https://github.com/saintparish4/spatial/compare/v0.1.0...HEAD
[0.1.0]: https://github.com/saintparish4/spatial/releases/tag/v0.1.0
