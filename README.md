# SpatiaX

A CAN bus / DBC decoder for motorsport telemetry, written in Rust.

The point of this project is not that it decodes CAN — `cantools` already
does that, and does it well. The point is that it decodes CAN **and ships
the evidence that it does so correctly**: hand-computed reference vectors,
exhaustive property tests, and a differential harness that checks every
decode against the reference implementation the industry already trusts.

> **Status: early.** The parser, decoder, encoder, `candump` replay, live
> SocketCAN capture, and the `spatiax` command-line tool exist and are
> backed by the three layers of evidence below — hand-computed vectors,
> property tests over every layout, and a differential test against
> `cantools` that CI runs on every push. Performance numbers do not exist
> yet. Nothing below is claimed as working unless the status table says so.
> See [Current state](#current-state).

---

## Why this exists

Every car on a GT3, GT4, LMP, or single-seater grid runs CAN. Turning raw
frames into engineering values — RPM, damper positions, brake temperatures,
wheel speeds — is the first link in every telemetry chain, and it is a link
teams maintain in-house. It is also a link where being *subtly* wrong is
worse than being obviously broken: a decoder that silently misreads a
big-endian signal produces plausible-looking traces that send an engineer
chasing a problem that does not exist.

So correctness is the product. The design follows from that.

## Design

These are the decisions the rebuild is built on. Where a decision describes
behaviour, the [status table](#current-state) governs whether that behaviour
has actually landed and is tested.

**Bit extraction is isolated and tiny.** `src/decode.rs` contains no I/O, no
allocation, and no knowledge of DBC text — just the arithmetic that pulls a
signal out of a payload. Keeping it small is what makes it possible to test
exhaustively.

**Frames don't allocate.** CAN FD bounds a payload at 64 bytes, so a frame
is a fixed `[u8; 64]` plus a length rather than a `Vec`. `CanFrame` is
`Copy`, and the decode path performs no heap allocation.

**The extended-identifier flag has exactly one interpretation site.** DBC
marks a 29-bit identifier by setting bit 31 of the message ID. That is
handled in `CanId::from_dbc` and nowhere else — a rule that exists because
the previous iteration of this code got it wrong in one place and then
rejected its own output in another.

**One runtime dependency for the library.** `thiserror`. Parsing a DBC is
line-oriented text handling and decoding is integer shifts; both are the
substance of the project rather than something to outsource. The `spatiax`
binary adds `clap`, behind the default-on `cli` feature; a crate that only
wants the decoder turns it off with `default-features = false`.

## Using it

```bash
cargo install --path .

spatiax decode car.dbc session.log                # text, grouped by frame
spatiax decode car.dbc session.log --format csv   # one row per signal
candump -L can0 | spatiax decode car.dbc -        # from a pipe
spatiax check car.dbc                             # layout problems

cargo install --path . --features socketcan       # Linux only
spatiax live car.dbc can0                         # decode as frames arrive
```

`decode` reads the format `candump -l` writes. Frames the DBC does not
describe are counted rather than printed; a malformed log line is reported
on stderr with its line number and reading carries on. `live` does the same
for frames arriving on a SocketCAN interface, stamped with the kernel's
receive time — the clock `candump` logs — so a live decode and a later
replay of the same session agree. The text form looks like this:

```text
1700000000.000500 300 SuspensionData
  DamperMux: Front right (1)
  DamperPosFR: 1.6 mm
1700000000.001000 18FEEE00 DiagResponse
  ResponseCode: Overheat (1)
```

`check` reports the two layout problems `cantools` refuses to load in strict
mode — a signal that runs past its message's DLC, and two signals that can
decode together but share a bit — since the parser here is lenient and
would otherwise decode such a file quietly.

Exit status follows `grep`: 0 when nothing was wrong, 1 when the command
finished but found something (malformed lines, layout problems), 2 when it
could not run.

## Correctness strategy

Three layers, in increasing order of what they catch:

1. **Hand-computed reference vectors.** A small set of signals whose
   expected raw and physical values were worked out on paper from the DBC
   definition and the payload bytes. These are the only expectations in the
   project not produced by a machine, which is exactly why they come first.
   A decoder cannot pass these by being self-consistently wrong.
2. **Property tests.** `tests/properties.rs` runs `proptest` over random
   start bits, every width from 1 to 64, both byte orders, and both value
   types inside a 64-byte payload: insert-then-extract round-trips,
   insertion never touches a bit outside the signal, `required_bytes` is
   exactly the span touched, and extraction agrees with an oracle written
   from a different formulation of the bit-numbering rule. Signals
   straddling byte boundaries — where hand-written vectors run out of
   imagination — are the common case here, not the edge case.
3. **Differential testing against `cantools`.** `tests/differential.rs`
   generates random but valid DBCs (standard and extended identifiers,
   payloads from 1 to 64 bytes, non-overlapping Intel and Motorola signals,
   signed and unsigned, assorted scalings, multiplexed pages, value tables)
   and random frames, decodes them with both `spatiax` and the Python
   reference, and requires exact agreement on raw values, on which signals
   are present, and on labels, with float-noise agreement on physical
   values. Re-encoding the raw values must also reproduce the bytes
   `cantools` produces. The test refuses to pass with fewer than 100,000
   decoded signal values; CI runs 500,000 and fails if the oracle is
   missing.

Layer 3 is the differentiator. `cantools` is what motorsport data engineers
already reach for, so agreement with it is a claim anyone can evaluate in
about thirty seconds:

```bash
python3 -m venv .venv && .venv/bin/pip install cantools==43.0.2
cargo test --test differential -- --nocapture
```

The test finds `.venv/bin/python` on its own, or honours
`SPATIAX_ORACLE_PYTHON`. Every run prints its seed; `SPATIAX_DIFFERENTIAL_SEED`
replays one, and a failure leaves the generated files under
`target/differential/<seed>/`.

## Current state

Honest accounting. A row is only "done" when there is a test named for it
and CI runs that test.

| Capability | State |
|---|---|
| CAN frame and identifier types | Done — `frame::tests`, `tests/public_api.rs` |
| DBC extended-identifier handling | Done — `dbc_id_with_bit31_set_is_extended_and_strips_the_flag` |
| DBC parser (`BO_` / `SG_` / `VAL_` records) | Done — `dbc::parser::tests`, `fixture_parses_all_messages_and_signals` |
| Bit extraction, Intel byte order | Done — `vector_a_intel_word_is_little_endian` |
| Bit extraction, Motorola byte order | Done — `vector_b_motorola_word_is_big_endian`, `vector_c`, `vector_f` |
| Signed signals, factor/offset scaling | Done — `vector_d_signed_signal_sign_extends_from_its_own_width`, `vector_e` |
| Reference vector suite | Done — `tests/vectors.rs`, expected values computed by hand |
| Signal encoder (`insert_raw`, `encode_signal`) | Done — `encode::tests`, `insert_then_extract_returns_the_raw_value`, re-encoding checked against `cantools` |
| 64-byte (CAN FD) payloads | Done — every layout up to 64 bytes in `tests/properties.rs` and `tests/differential.rs` |
| Property tests | Done — `tests/properties.rs`, 10 properties, 2,048 cases each locally and 16,384 in CI |
| Differential test vs. `cantools` | Done — `tests/differential.rs`, ≥100,000 generated cases enforced, 500,000 in CI |
| Multiplexed signals (simple multiplexing) | Done — `the_multiplexor_value_selects_which_page_decodes`, `vector_g`, multiplexed messages in `tests/differential.rs` |
| Extended multiplexing (`m<N>M`, `SG_MUL_VAL_` ranges) | Rejected at parse time with a clear error, rather than decoded wrongly |
| Value tables (`VAL_`, `Decoded::label`) | Done — `parses_value_tables_onto_their_signal`, `labels_match_the_sign_interpreted_raw_value`, labels compared in `tests/differential.rs` (≥10,000 enforced) |
| CAN FD frame I/O | Done — FD frames read from `candump` logs (`reads_a_can_fd_frame_and_drops_its_flags_digit`) and from SocketCAN (`an_fd_frame_with_an_extended_identifier_carries_all_its_bytes`, `tests/live.rs`) |
| `candump` log replay (`candump::LogReader`) | Done — `candump::tests`, `tests/candump.rs` replays `fixtures/gt3_sample.log` |
| DBC layout check (`dbc::check`) | Done — `dbc::check::tests` |
| `spatiax` binary (`decode`, `check`) | Done — `tests/cli.rs` runs the built binary end to end |
| SocketCAN live capture (`live::Capture`, `spatiax live`) | Done — `live::tests`, `tests/live.rs` sends frames over `vcan0` in CI and reads them back through both |
| Benchmarks | Planned |
| MoTeC `.ld` export | Stretch goal |

There are **no published performance numbers**, and there will not be any
until `cargo bench` produces them in CI on a machine the reader can
identify. The previous version of this README carried a table of measured
throughput and latency figures for a workspace that had never compiled;
removing it was the first task of the rebuild.

## Roadmap

In order, each building on the one before:

- **Salvage.** Collapse six crates to one, delete the subsystems that were
  breadth rather than depth, restore a green build, and strip every unearned
  claim.
- **The decoder.** DBC parsing and bit-exact extraction for both byte
  orders, against hand-computed vectors.
- **Proof.** Property tests and the differential harness, both enforced in
  CI.
- **Real-world coverage.** Multiplexed signals, CAN FD, value tables,
  SocketCAN capture, and `candump` replay.
- **Performance.** Byte-aligned fast paths, benchmarks, and the first
  numbers this project is willing to publish.

## Stretch goal — MoTeC `.ld` export

The intended endpoint is **writing decoded output as a MoTeC `.ld` file.**

MoTeC i2 is the de facto analysis tool on GT3, GT4, LMP, and most
single-seater grids; engineers work inside it for the whole of a session. A
tool that emits `.ld` slots into a workflow that already exists instead of
asking anyone to adopt a new viewer — "decode your CAN log, open it in i2"
is a complete and useful sentence, in a way that "decode your log and then
look at my custom dashboard" is not.

It is also the part of this project that cannot be faked. The format is
binary, undocumented by MoTeC, and community-reverse-engineered, so a
working exporter is real evidence of both the reverse-engineering and the
domain knowledge — that channels carry units and per-channel sample rates,
and that those rates differ across a car. Verification is a round-trip:
export a decoded log, open it in i2, and confirm the traces match the
source, backed by a byte-level golden-file test.

Deliberately *not* the stretch goal: a web dashboard, or a bespoke binary
log format. Neither proves anything a motorsport team cares about.

## Why the rewrite

This started as a broad "high-speed telemetry platform" — six crates
covering CAN, serial and network ingestion, signal processing, machine
learning, and alerting. None of it compiled: the workspace listed two member
crates that had never existed, so `cargo metadata` failed before dependency
resolution, and the binary the README told you to run had no package to
build it. There were no tests anywhere in roughly 8,200 lines.

Reading it properly turned up three genuine decoder bugs, all of which a
single test would have caught:

- The DBC parser trimmed each line and then tested whether it started with
  `" SG_ "` — a branch that can never be taken. Every message loaded with
  **zero signals**, and the parser reported success.
- Motorola (big-endian) bit extraction walked bit positions upward, which is
  the Intel rule applied to Motorola data. Roughly half of production
  motorsport DBCs use Motorola layout.
- Extended identifiers were parsed without masking the DBC flag bit, so
  every 29-bit ID came out around 2.1 billion too high — and was then
  rejected by this project's own validator.

The lesson taken from that is the one this rebuild is organised around:
breadth is cheap and proves nothing. One decoder that is provably correct is
worth more than six subsystems that merely look impressive in a file tree.

## Building

```bash
cargo test
```

No system libraries, no ML runtimes, no Docker. The library depends on
`thiserror` alone; the binary adds `clap` behind the `cli` feature, and
`proptest` and `rand` are dev-dependencies. Requires Rust 1.85 or later.
The differential test additionally wants Python 3 with `cantools` and skips
with a message when it cannot find one — see
[Correctness strategy](#correctness-strategy) for the two-line setup.

Live capture is behind the `socketcan` feature and only does anything on
Linux, so the default build stays portable on macOS and Windows. Its tests
need a virtual CAN interface and skip unless `SPATIAX_VCAN` names one:

```bash
sudo modprobe vcan && sudo ip link add dev vcan0 type vcan
sudo ip link set vcan0 mtu 72 && sudo ip link set up vcan0
SPATIAX_VCAN=vcan0 cargo test --features socketcan --test live
```

## Licence

MIT. See [LICENSE](LICENSE).
