# SpatiaX

A CAN bus / DBC decoder for motorsport telemetry, written in Rust.

The point of this project is not that it decodes CAN — `cantools` already
does that, and does it well. The point is that it decodes CAN **and ships
the evidence that it does so correctly**: hand-computed reference vectors,
exhaustive property tests, and a differential harness that checks every
decode against the reference implementation the industry already trusts.

> **Status: rebuild in progress.** This repository is being rewritten from
> the ground up. Nothing below is claimed as working unless the status table
> says so. See [Current state](#current-state) — it is deliberately blunt.

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

These are the decisions the rebuild is built on, not a description of code
that already exists — the [status table](#current-state) governs what has
actually landed.

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

**One runtime dependency.** `thiserror`. Parsing a DBC is line-oriented text
handling and decoding is integer shifts; both are the substance of the
project rather than something to outsource.

## Correctness strategy

Three layers, in increasing order of what they catch:

1. **Hand-computed reference vectors.** A small set of signals whose
   expected raw and physical values were worked out on paper from the DBC
   definition and the payload bytes. These are the only expectations in the
   project not produced by a machine, which is exactly why they come first.
   A decoder cannot pass these by being self-consistently wrong.
2. **Property tests.** `encode(decode(x)) == x` across every start bit,
   every width from 1 to 64, both byte orders, signed and unsigned —
   especially signals straddling byte boundaries, which is where hand-written
   vectors run out of imagination.
3. **Differential testing against `cantools`.** Generate a random but valid
   DBC and random frames, decode with both `spatiax` and the Python
   reference, and assert agreement. This is the layer that turns "I believe
   this is correct" into "here is the harness, run it yourself."

Layer 3 is the differentiator. `cantools` is what motorsport data engineers
already reach for, so agreement with it is a claim anyone can evaluate in
about thirty seconds.

## Current state

Honest accounting. A row is only "done" when there is a test named for it
and CI runs that test.

| Capability | State |
|---|---|
| CAN frame and identifier types | Done — `frame::tests`, `tests/public_api.rs` |
| DBC extended-identifier handling | Done — `dbc_id_with_bit31_set_is_extended_and_strips_the_flag` |
| DBC parser (`BO_` / `SG_` records) | Not yet landed |
| Bit extraction, Intel byte order | Not yet landed |
| Bit extraction, Motorola byte order | Not yet landed |
| Signed signals, factor/offset scaling | Not yet landed |
| Reference vector suite | Not yet landed |
| Property tests | Planned |
| Differential harness vs. `cantools` | Planned |
| Multiplexed signals, CAN FD, value tables | Planned |
| SocketCAN live capture, `candump` replay | Planned |
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

No system libraries, no ML runtimes, no Docker, and one dependency
(`thiserror`). Requires Rust 1.85 or later. Live CAN capture (Linux,
`socketcan`) arrives later behind a feature flag, so the default build stays
portable on macOS and Windows.

## Licence

MIT. See [LICENSE](LICENSE).
