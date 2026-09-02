//! End-to-end reference vectors: DBC text in, physical values out.
//!
//! Every expected value here was computed by hand from the DBC definition and
//! the payload bytes, then cross-checked against an independent script
//! implementing the same two bit walks. None of them was produced by running
//! this crate — that is what lets these tests catch a decoder that is
//! self-consistently wrong.

use spatiax::dbc::{self, Database, Multiplexing};
use spatiax::{CanFrame, CanId};

fn fixture() -> Database {
    dbc::parse(include_str!("../fixtures/gt3_sample.dbc")).expect("fixture DBC parses")
}

/// Decode one named signal, returning `(raw, value)`.
fn decode_one(db: &Database, id: CanId, payload: &[u8], signal: &str) -> (u64, f64) {
    let frame = CanFrame::new(id, payload, 0).expect("payload fits a frame");
    db.decode_frame(&frame)
        .expect("identifier is in the fixture")
        .map(|r| r.expect("signal fits the payload"))
        .find(|d| d.signal.name == signal)
        .map(|d| (d.raw, d.value))
        .unwrap_or_else(|| panic!("fixture has no signal named {signal}"))
}

fn assert_close(actual: f64, expected: f64) {
    assert!(
        (actual - expected).abs() < 1e-9,
        "expected {expected}, got {actual}"
    );
}

#[test]
fn fixture_parses_all_messages_and_signals() {
    let db = fixture();
    assert_eq!(db.len(), 4, "four BO_ records in the fixture");
    assert_eq!(db.signal_count(), 10, "ten SG_ records in the fixture");
}

#[test]
fn vector_a_intel_word_is_little_endian() {
    // EngineRPM: 0|16@1+, factor 0.25. Bytes 0..1 = 34 12.
    // Intel: byte 0 is the low byte, so raw = 0x1234 = 4660. 4660 * 0.25 = 1165.0
    let payload = [0x34, 0x12, 0, 0, 0, 0, 0, 0];
    let (raw, value) = decode_one(&fixture(), CanId::Standard(256), &payload, "EngineRPM");
    assert_eq!(raw, 0x1234);
    assert_eq!(value, 1165.0);
}

#[test]
fn vector_b_motorola_word_is_big_endian() {
    // WheelSpeedFL: 7|16@0+, factor 0.01. Bytes 0..1 = 12 34.
    // Motorola from bit 7 reads byte 0 MSB-first then byte 1: raw = 0x1234 —
    // the same raw as vector A from the reversed layout. 4660 * 0.01 = 46.6
    let payload = [0x12, 0x34, 0, 0, 0, 0, 0, 0];
    let (raw, value) = decode_one(&fixture(), CanId::Standard(512), &payload, "WheelSpeedFL");
    assert_eq!(raw, 0x1234);
    assert_close(value, 46.6);
}

#[test]
fn vector_c_motorola_from_a_later_byte() {
    // WheelSpeedFR: 23|16@0+ starts at byte 2 bit 7 and runs into byte 3.
    // Bytes 2..3 = FF 00: raw = 0xFF00 = 65280. 65280 * 0.01 = 652.8
    let payload = [0, 0, 0xFF, 0x00, 0, 0, 0, 0];
    let (raw, value) = decode_one(&fixture(), CanId::Standard(512), &payload, "WheelSpeedFR");
    assert_eq!(raw, 0xFF00);
    assert_close(value, 652.8);
}

#[test]
fn vector_d_signed_signal_sign_extends_from_its_own_width() {
    // DamperPosFL: 8|16@1-, factor 0.1. Bytes 1..2 = FF FF.
    // Intel raw = 0xFFFF, which as a signed 16-bit value is -1. -1 * 0.1 = -0.1
    let payload = [0x00, 0xFF, 0xFF, 0, 0, 0, 0, 0];
    let (raw, value) = decode_one(&fixture(), CanId::Standard(768), &payload, "DamperPosFL");
    assert_eq!(raw, 0xFFFF);
    assert_close(value, -0.1);
}

#[test]
fn vector_e_factor_and_offset_apply_in_that_order() {
    // CoolantTemp: 16|8@1+, factor 0.5, offset -40. Byte 2 = 0x64 = 100.
    // 100 * 0.5 - 40 = 10.0
    let payload = [0, 0, 0x64, 0, 0, 0, 0, 0];
    let (raw, value) = decode_one(&fixture(), CanId::Standard(256), &payload, "CoolantTemp");
    assert_eq!(raw, 100);
    assert_eq!(value, 10.0);
}

#[test]
fn vector_f_motorola_in_the_upper_half_of_the_frame() {
    // OilPressure: 39|16@0+, factor 0.001. Bit 39 is byte 4 bit 7, so the
    // signal is bytes 4..5 = 0B B8: raw = 0x0BB8 = 3000. 3000 * 0.001 = 3.0
    let payload = [0, 0, 0, 0, 0x0B, 0xB8, 0, 0];
    let (raw, value) = decode_one(&fixture(), CanId::Standard(256), &payload, "OilPressure");
    assert_eq!(raw, 3000);
    assert_close(value, 3.0);
}

#[test]
fn vector_g_intel_and_motorola_signals_coexist_in_one_message() {
    // EngineData mixes Intel (EngineRPM) and Motorola (OilPressure). Decoding
    // both from one payload checks that neither walk disturbs the other.
    let payload = [0x34, 0x12, 0, 0, 0x0B, 0xB8, 0, 0];
    let db = fixture();
    assert_eq!(
        decode_one(&db, CanId::Standard(256), &payload, "EngineRPM").0,
        0x1234
    );
    assert_eq!(
        decode_one(&db, CanId::Standard(256), &payload, "OilPressure").0,
        3000
    );
}

#[test]
fn extended_identifier_message_is_reachable_by_its_masked_id() {
    // The fixture declares 2566843904 == 0x98FEEE00. Bit 31 is the DBC
    // extended flag, so the message lives at Extended(0x18FEEE00).
    let db = fixture();
    assert!(db.message(CanId::Extended(0x18FE_EE00)).is_some());
    assert!(db.message(CanId::Standard(256)).is_some());

    let payload = [0x42, 0, 0, 0, 0, 0, 0, 0];
    let (raw, value) = decode_one(&db, CanId::Extended(0x18FE_EE00), &payload, "ResponseCode");
    assert_eq!(raw, 0x42);
    assert_eq!(value, 66.0);
}

#[test]
fn vector_g_multiplexor_selects_the_page_that_decodes() {
    // DamperMux: 0|8@1+. DamperPosFL is m0, DamperPosFR is m1, both 8|16@1-.
    // Byte 0 = 0x01 selects page 1; bytes 1..2 = 0x10 0x00 -> raw 0x0010 = 16, * 0.1 = 1.6
    let db = fixture();
    let signals = &db.message(CanId::Standard(768)).unwrap().signals;
    assert_eq!(signals[0].multiplexing, Multiplexing::Multiplexor);
    assert_eq!(signals[1].multiplexing, Multiplexing::Multiplexed(0));
    assert_eq!(signals[2].multiplexing, Multiplexing::Multiplexed(1));

    let payload = [0x01, 0x10, 0x00, 0, 0, 0, 0, 0];
    let frame = CanFrame::new(CanId::Standard(768), &payload, 0).unwrap();
    let names: Vec<_> = db
        .decode_frame(&frame)
        .unwrap()
        .map(|d| d.unwrap().signal.name.clone())
        .collect();
    assert_eq!(names, ["DamperMux", "DamperPosFR"]);

    let (raw, value) = decode_one(&db, CanId::Standard(768), &payload, "DamperPosFR");
    assert_eq!(raw, 0x0010);
    assert_close(value, 1.6);
}

#[test]
fn vector_h_value_table_labels_the_raw_value() {
    // ResponseCode: 0|8@1+ with VAL_ 0 "OK" 1 "Overheat" 255 "Not available".
    // 0x01 -> "Overheat"; 0x42 has no entry; 0xFF -> the label with spaces.
    let db = fixture();
    let code = db
        .message(CanId::Extended(0x18FE_EE00))
        .unwrap()
        .signal("ResponseCode")
        .unwrap();
    assert_eq!(code.label(0x01), Some("Overheat"));
    assert_eq!(code.label(0x42), None);
    assert_eq!(code.label(0xFF), Some("Not available"));

    let mux = db
        .message(CanId::Standard(768))
        .unwrap()
        .signal("DamperMux")
        .unwrap();
    assert_eq!(mux.label(1), Some("Front right"));
}

#[test]
fn a_classic_can_frame_shorter_than_the_dlc_reports_which_signals_did_not_fit() {
    // Four bytes of an eight-byte message: EngineRPM, CoolantTemp, and
    // ThrottlePos fit; OilPressure (bytes 4..5) does not.
    let db = fixture();
    let frame = CanFrame::new(CanId::Standard(256), &[0x34, 0x12, 0x64, 0xFA], 0).unwrap();
    let results: Vec<_> = db.decode_frame(&frame).unwrap().collect();

    assert_eq!(results.len(), 4);
    assert!(results[0].is_ok(), "EngineRPM");
    assert!(results[1].is_ok(), "CoolantTemp");
    assert!(results[2].is_ok(), "ThrottlePos");
    assert!(results[3].is_err(), "OilPressure");
}
