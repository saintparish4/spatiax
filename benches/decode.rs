//! Benchmarks for the decode path, run with `cargo bench`.
//!
//! Every input is a file in `fixtures/` or a layout named here, so each
//! number can be tied to something a reader can open. `decode_frame` runs
//! the public entry point over the sample log as-is — unknown identifiers,
//! short frames and all. `decode_message` decodes each fixture message with
//! a full payload to give a cost per signal. `extract_raw` isolates the bit
//! extraction by layout, and `candump` measures the text parsing the binary
//! does before any of that.
//!
//! `scripts/bench_table.py` turns the results into the table in the README.

use std::hint::black_box;

use criterion::{Criterion, Throughput, criterion_group, criterion_main};
use spatiax::dbc::{ByteOrder, Multiplexing, Signal, ValueTable, ValueType};
use spatiax::{CanFrame, CanId, Database, candump, dbc, decode};

const DBC: &str = include_str!("../fixtures/gt3_sample.dbc");
const LOG: &str = include_str!("../fixtures/gt3_sample.log");

fn database() -> Database {
    dbc::parse(DBC).expect("fixture parses")
}

fn frames() -> Vec<CanFrame> {
    candump::LogReader::new(LOG.as_bytes())
        .collect::<Result<_, _>>()
        .expect("fixture log reads")
}

fn signal(start_bit: u16, length: u8, byte_order: ByteOrder) -> Signal {
    Signal {
        name: "bench".into(),
        start_bit,
        length,
        byte_order,
        value_type: ValueType::Unsigned,
        factor: 1.0,
        offset: 0.0,
        min: 0.0,
        max: 0.0,
        unit: String::new(),
        multiplexing: Multiplexing::None,
        value_table: ValueTable::default(),
    }
}

/// Decode every signal of a frame the database knows about. An unknown
/// identifier is ordinary bus traffic and costs one hash lookup.
fn decode_all(db: &Database, frame: &CanFrame) {
    if let Some(signals) = db.decode_frame(frame) {
        for decoded in signals {
            black_box(decoded.ok().map(|d| d.value));
        }
    }
}

fn decode_frame(c: &mut Criterion) {
    let db = database();
    let frames = frames();

    let mut group = c.benchmark_group("decode_frame");
    group.throughput(Throughput::Elements(frames.len() as u64));
    group.bench_function("gt3_sample", |b| {
        b.iter(|| {
            for frame in &frames {
                decode_all(black_box(&db), black_box(frame));
            }
        })
    });
    group.finish();
}

fn decode_message(c: &mut Criterion) {
    let db = database();
    let payloads: [(&str, u16, [u8; 8]); 3] = [
        (
            "EngineData",
            0x100,
            [0x34, 0x12, 0x64, 0x80, 0x12, 0x34, 0, 0],
        ),
        ("WheelSpeeds", 0x200, [0x12, 0x34, 0xFF, 0x00, 0, 0, 0, 0]),
        ("SuspensionData", 0x300, [0x01, 0x10, 0x00, 0, 0, 0, 0, 0]),
    ];

    let mut group = c.benchmark_group("decode_message");
    for (name, id, data) in payloads {
        let message = db.message(CanId::Standard(id)).expect("fixture message");
        let signals = message.decode(&data).count();
        group.throughput(Throughput::Elements(signals as u64));
        group.bench_function(name, |b| {
            b.iter(|| {
                for decoded in black_box(message).decode(black_box(&data)) {
                    black_box(decoded.ok().map(|d| d.value));
                }
            })
        });
    }
    group.finish();
}

fn extract_raw(c: &mut Criterion) {
    let classic = [0x34, 0x12, 0x64, 0x80, 0x12, 0x34, 0xAB, 0xCD];
    let fd = [0xA5; 16];
    let layouts: [(&str, Signal, &[u8]); 9] = [
        ("intel/1_bit", signal(3, 1, ByteOrder::Intel), &classic),
        (
            "intel/16_aligned",
            signal(0, 16, ByteOrder::Intel),
            &classic,
        ),
        (
            "intel/12_unaligned",
            signal(4, 12, ByteOrder::Intel),
            &classic,
        ),
        ("intel/64", signal(0, 64, ByteOrder::Intel), &classic),
        (
            "motorola/16_aligned",
            signal(7, 16, ByteOrder::Motorola),
            &classic,
        ),
        (
            "motorola/8_unaligned",
            signal(11, 8, ByteOrder::Motorola),
            &classic,
        ),
        ("motorola/64", signal(7, 64, ByteOrder::Motorola), &classic),
        // Sixty-four bits from an unaligned start touch nine bytes, which is
        // the one shape a single word load cannot cover.
        (
            "intel/64_over_9_bytes",
            signal(4, 64, ByteOrder::Intel),
            &fd,
        ),
        (
            "motorola/64_over_9_bytes",
            signal(3, 64, ByteOrder::Motorola),
            &fd,
        ),
    ];

    // Callers match on the `Result` at once, and so does this; handing the
    // whole `Result<u64, Error>` to the harness would time copying and
    // dropping a 48-byte enum instead of the extraction.
    let mut group = c.benchmark_group("extract_raw");
    group.throughput(Throughput::Elements(1));
    for (name, signal, data) in &layouts {
        group.bench_function(*name, |b| {
            b.iter(|| decode::extract_raw(black_box(data), black_box(signal)).expect("layout fits"))
        });
    }
    group.finish();
}

fn candump_parse_line(c: &mut Criterion) {
    let lines = [
        ("classic", "(1700000000.000000) can0 100#3412640000000000"),
        (
            "fd",
            "(1700000000.001750) can0 7FF##1DEADBEEFDEADBEEFDEADBEEF",
        ),
    ];

    let mut group = c.benchmark_group("candump");
    group.throughput(Throughput::Elements(1));
    for (name, line) in lines {
        group.bench_function(name, |b| b.iter(|| candump::parse_line(black_box(line), 1)));
    }
    group.finish();
}

criterion_group!(
    benches,
    decode_frame,
    decode_message,
    extract_raw,
    candump_parse_line
);
criterion_main!(benches);
