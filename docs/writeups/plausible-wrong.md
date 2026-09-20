# The expensive kind of wrong

*How three CAN decoder bugs produced traces that looked fine, and what it took
to make sure they could not come back.*

---

A decoder that crashes is a decoder you fix on Monday. A decoder that returns
the wrong number is a decoder that costs you a test session, because nobody
looks at it. The engineer looks at the trace, sees something odd in the
suspension data, and spends the afternoon chasing a damper problem that does
not exist.

I rewrote a CAN/DBC decoder recently. Reading the old code properly turned up
three bugs. None of them crashed. All three produced output that a reasonable
person would have believed.

## Bug one: every message loaded with zero signals

The DBC parser trimmed each line and then checked whether the result started
with `" SG_ "`.

Trimming removes the leading space. The test could never be true. Every
message in every database loaded with **zero signals** — and the parser
reported success, because as far as it was concerned it had read the file
without incident.

The failure mode is not "no data." It is a database that parses cleanly, a
decoder that runs without complaint, and an empty result set that looks like a
quiet bus.

## Bug two: half of all production DBCs decoded wrongly

Motorola (big-endian) bit extraction walked bit positions *upward*. That is the
Intel rule applied to Motorola data.

Intel and Motorola signals disagree about which direction a multi-byte field
grows across the payload. Applying the wrong rule does not throw. It reads real
bits from the real payload in the wrong order and hands back a number.

Roughly half of production motorsport DBCs use Motorola layout. So half the
channels were wrong, and they were wrong in a way that scales with the value:
small numbers stayed smallish, large ones went strange. That is exactly the
signature of a sensor fault.

## Bug three: every extended identifier was ~2.1 billion too high

DBC files set bit 31 of a 29-bit identifier as a flag meaning "this is
extended." You mask it off before using the number. The old parser did not.

Every extended ID came out around 2,147,483,648 too high — and was then
rejected by the project's own validator, which is the one piece of luck in the
whole story. It failed loudly instead of silently. It just failed loudly for
a reason nobody could read from the error.

## What these three have in common

Not one is exotic. Each is a single line. Each would have been caught by one
test.

There were no tests. Roughly 8,200 lines, six crates, CAN and serial and
network ingestion and signal processing and machine learning and alerting —
and the workspace listed two member crates that had never existed, so
`cargo metadata` failed before dependency resolution. The binary the README
told you to run had no package to build it.

The README also carried a throughput table. For a workspace that had never
compiled.

That is the actual lesson, and it is not "write tests." It is that **breadth is
cheap and proves nothing.** Six subsystems in a file tree is a screenshot. One
decoder that is provably correct is a tool.

## Making it not come back

Three independent layers, because each catches what the others miss.

**Hand-computed reference vectors.** Seven cases where I worked out the
expected value on paper before writing any code: Intel word, Motorola word,
signed signal sign-extending from its own width, factor/offset scaling,
multiplexed page selection. These are slow to produce and worth it, because
they are the only layer that is not derived from another implementation. If
every other layer agrees with `cantools` and `cantools` is wrong, these still
catch it.

**Property tests over every layout.** Ten properties, over every signal layout
from 1 to 64 bits — 2,048 cases each locally, 16,384 in CI. The load-bearing
one is round-trip: `insert_raw` then `extract_raw` returns what went in, for
every start bit, every width, both byte orders. Bug two dies here and cannot
come back.

**A differential harness against `cantools`.** Generated databases and
payloads, decoded by both implementations, compared value by value. At least
100,000 cases are enforced; CI runs 500,000 on every push. Value-table labels
are compared too, with 10,000 enforced.

That last layer needs a defence, because it looks like cheating. `cantools`
already decodes CAN, and it does it well. Why write another one?

Because the project is not "decode CAN." It is "decode CAN **and ship the
evidence that it does so correctly**." Differential testing against the
implementation the industry already trusts is not a substitute for
understanding the format — you still have to generate valid databases, which
means you still have to know the rules. It is a way to make one person's
understanding checkable against a decade of other people's bug reports.

And it is asymmetric in the right direction. A disagreement is always
interesting. If I am wrong, I learn it in CI instead of in a debrief.

## The last layer is not a test

Every claim above is a `cargo test` away from being verified, and all of them
would still be true if the exported file were unreadable by the tool an
engineer actually opens.

So: the exporter writes a MoTeC `.ld` file. CI reads it back with an
independent `.ld` implementation and checks all 70,993 values of the demo lap,
and reproduces a reference file byte for byte. Then, once, by hand — I opened
the exported lap in **MoTeC i2 Pro 1.1**. It derived a 1:22.520 session from
the sample count and rate in the channel headers, wrote its own `.ldx`, and
plotted `EngineRPM` peaking at 8600 and `Speed` at 258 km/h, which is what the
log has.

That step is not automated and probably cannot be. It is also the only step
that proves the format is right rather than self-consistent. A file every one
of my tests loves and i2 refuses to open is a file that is wrong.

---

**The rule I took from this:** a test suite tells you the code does what you
think. A differential harness tells you what you think matches what everyone
else thinks. Opening the file in the real tool tells you whether any of it
mattered. You need all three, and the third one is the one people skip.

*Source: [`saintparish4/spatiax`](https://github.com/saintparish4/spatiax) —
Rust, ~8,700 lines, CAN FD to 64 bytes, live SocketCAN capture, MoTeC `.ld`
export.*
