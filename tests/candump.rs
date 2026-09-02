//! Replay of a `candump` log through a DBC: the offline path end to end.
//!
//! The log in `fixtures/gt3_sample.log` carries the same payloads as the
//! hand-computed vectors in `tests/vectors.rs`, plus the records a real
//! capture mixes in — a remote frame, an error frame, a CAN FD frame from an
//! identifier the DBC does not describe, and a short frame.

use spatiax::candump::LogReader;
use spatiax::dbc::{self, Database};
use spatiax::{CanFrame, CanId};

const LOG: &str = include_str!("../fixtures/gt3_sample.log");

fn fixture() -> Database {
    dbc::parse(include_str!("../fixtures/gt3_sample.dbc")).expect("fixture DBC parses")
}

fn replay() -> Vec<CanFrame> {
    LogReader::new(LOG.as_bytes())
        .collect::<Result<_, _>>()
        .expect("every line of the fixture log parses")
}

/// `(signal, value, label)` for every signal that decoded from the frame.
fn decode(db: &Database, frame: &CanFrame) -> Vec<(String, f64, Option<String>)> {
    db.decode_frame(frame)
        .expect("frame is described by the DBC")
        .filter_map(Result::ok)
        .map(|d| {
            (
                d.signal.name.clone(),
                d.value,
                d.label().map(str::to_string),
            )
        })
        .collect()
}

#[test]
fn the_fixture_log_yields_only_its_data_frames_in_order() {
    let frames = replay();
    let ids: Vec<_> = frames.iter().map(CanFrame::id).collect();
    assert_eq!(
        ids,
        [
            CanId::Standard(0x100),
            CanId::Standard(0x200),
            CanId::Standard(0x300),
            CanId::Standard(0x300),
            CanId::Extended(0x18FE_EE00),
            CanId::Standard(0x7FF),
            CanId::Standard(0x100),
        ]
    );
    assert_eq!(frames[0].timestamp_us, 1_700_000_000_000_000);
    assert_eq!(frames[6].timestamp_us, 1_700_000_000_002_000);
}

#[test]
fn replayed_frames_decode_to_the_reference_values() {
    let db = fixture();
    let frames = replay();

    let engine = decode(&db, &frames[0]);
    assert_eq!(engine[0], ("EngineRPM".into(), 1165.0, None));
    assert_eq!(engine[1], ("CoolantTemp".into(), 10.0, None));

    let wheels = decode(&db, &frames[1]);
    assert_eq!(wheels[0].0, "WheelSpeedFL");
    assert!((wheels[0].1 - 46.6).abs() < 1e-9);
    assert!((wheels[1].1 - 652.8).abs() < 1e-9);
}

#[test]
fn replayed_multiplexed_frames_select_their_page_and_label_the_selector() {
    let db = fixture();
    let frames = replay();

    let page1 = decode(&db, &frames[2]);
    assert_eq!(
        page1[0],
        ("DamperMux".into(), 1.0, Some("Front right".into()))
    );
    assert_eq!(page1[1].0, "DamperPosFR");
    assert!((page1[1].1 - 1.6).abs() < 1e-9);

    let page0 = decode(&db, &frames[3]);
    assert_eq!(page0[0].2.as_deref(), Some("Front left"));
    assert_eq!(page0[1].0, "DamperPosFL");
    assert!((page0[1].1 + 0.1).abs() < 1e-9);
}

#[test]
fn a_replayed_extended_frame_reaches_its_message_and_label() {
    let db = fixture();
    let diag = decode(&db, &replay()[4]);
    assert_eq!(
        diag,
        [("ResponseCode".into(), 1.0, Some("Overheat".into()))]
    );
}

#[test]
fn frames_the_dbc_does_not_describe_are_none_rather_than_errors() {
    let db = fixture();
    let frames = replay();
    assert_eq!(frames[5].len(), 12, "the FD frame carries all its bytes");
    assert!(db.decode_frame(&frames[5]).is_none());
}

#[test]
fn a_short_replayed_frame_reports_the_signals_that_did_not_fit() {
    let db = fixture();
    let frames = replay();
    let results: Vec<_> = db.decode_frame(&frames[6]).unwrap().collect();
    assert_eq!(results.len(), 4);
    assert!(results[0].is_ok(), "EngineRPM fits two bytes");
    assert!(results[1..].iter().all(Result::is_err));
}
