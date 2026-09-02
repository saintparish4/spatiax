//! Property tests over every signal layout the decoder accepts.
//!
//! The hand-computed vectors catch gross errors; these catch the ones nobody
//! writes a vector for — a 13-bit Motorola signal starting at bit 3 of byte
//! 5, say. Each property runs over random start bits, every width from 1 to
//! 64, both byte orders, and both value types, inside a 64-byte payload.
//!
//! The extraction check compares against an oracle written here from a
//! different formulation of the DBC bit-numbering rule, so it does not just
//! confirm that the encoder and decoder agree with each other.

use proptest::prelude::*;
use spatiax::dbc::{self, ByteOrder, Multiplexing, Signal, ValueType};
use spatiax::decode::{extract_raw, required_bytes};
use spatiax::encode::insert_raw;

const PAYLOAD_BYTES: usize = 64;
const PAYLOAD_BITS: usize = PAYLOAD_BYTES * 8;

fn signal(start_bit: u16, length: u8, byte_order: ByteOrder, value_type: ValueType) -> Signal {
    Signal {
        name: "S".into(),
        start_bit,
        length,
        byte_order,
        value_type,
        factor: 1.0,
        offset: 0.0,
        min: 0.0,
        max: 0.0,
        unit: String::new(),
        multiplexing: Multiplexing::None,
    }
}

/// Every payload bit a signal occupies, most-significant first.
///
/// Motorola is derived by transposing to an MSB-first linear index, taking
/// a contiguous run there, and transposing back — a different computation
/// from the decoder's step-by-step walk.
fn oracle_positions(s: &Signal) -> Vec<usize> {
    let start = usize::from(s.start_bit);
    let length = usize::from(s.length);
    match s.byte_order {
        ByteOrder::Intel => (start..start + length).rev().collect(),
        ByteOrder::Motorola => {
            let msb_first = |pos: usize| (pos / 8) * 8 + (7 - pos % 8);
            let first = msb_first(start);
            (first..first + length).map(msb_first).collect()
        }
    }
}

fn oracle_extract(data: &[u8], s: &Signal) -> u64 {
    oracle_positions(s).into_iter().fold(0u64, |acc, pos| {
        (acc << 1) | u64::from((data[pos / 8] >> (pos % 8)) & 1)
    })
}

fn mask(length: u8) -> u64 {
    u64::MAX >> (64 - u32::from(length))
}

fn sign_extend(raw: u64, length: u8) -> i128 {
    let half = 1i128 << (length - 1);
    let raw = i128::from(raw);
    if raw >= half {
        raw - (1i128 << length)
    } else {
        raw
    }
}

/// A layout that fits inside a 64-byte payload, built constructively so no
/// generated case is wasted on rejection.
fn layout() -> impl Strategy<Value = (u16, u8, ByteOrder)> {
    (
        0..PAYLOAD_BYTES,
        0..8usize,
        prop_oneof![Just(ByteOrder::Intel), Just(ByteOrder::Motorola)],
    )
        .prop_flat_map(|(byte, bit, order)| {
            let start = byte * 8 + bit;
            let room = match order {
                ByteOrder::Intel => PAYLOAD_BITS - start,
                ByteOrder::Motorola => (bit + 1) + 8 * (PAYLOAD_BYTES - 1 - byte),
            };
            let max_length = room.min(64) as u8;
            (Just(start as u16), 1..=max_length, Just(order))
        })
}

fn value_type() -> impl Strategy<Value = ValueType> {
    prop_oneof![Just(ValueType::Unsigned), Just(ValueType::Signed)]
}

fn any_signal() -> impl Strategy<Value = Signal> {
    (layout(), value_type()).prop_map(|((start, len, order), vt)| signal(start, len, order, vt))
}

/// A signal plus a raw value that fits its width.
fn signal_and_raw() -> impl Strategy<Value = (Signal, u64)> {
    (any_signal(), any::<u64>()).prop_map(|(s, bits)| {
        let raw = bits & mask(s.length);
        (s, raw)
    })
}

/// Scalings whose arithmetic is common in real DBCs.
fn scaling() -> impl Strategy<Value = (f64, f64)> {
    let factor = prop::sample::select(vec![1.0, 2.0, 0.5, 0.25, 0.125, 0.1, 0.01, 0.001, 10.0]);
    let offset = prop::sample::select(vec![0.0, -40.0, 1.0, -1.0, 0.5, -273.15, 100.0, -128.0]);
    (factor, offset)
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(2048))]

    #[test]
    fn extract_agrees_with_an_independent_formulation_of_the_bit_rule(
        s in any_signal(),
        data in any::<[u8; PAYLOAD_BYTES]>(),
    ) {
        prop_assert_eq!(extract_raw(&data, &s).unwrap(), oracle_extract(&data, &s));
    }

    #[test]
    fn insert_then_extract_returns_the_raw_value(
        (s, raw) in signal_and_raw(),
        mut data in any::<[u8; PAYLOAD_BYTES]>(),
    ) {
        insert_raw(&mut data, &s, raw).unwrap();
        prop_assert_eq!(extract_raw(&data, &s).unwrap(), raw);
    }

    #[test]
    fn insert_changes_no_bit_outside_the_signal(
        (s, raw) in signal_and_raw(),
        original in any::<[u8; PAYLOAD_BYTES]>(),
    ) {
        let mut data = original;
        insert_raw(&mut data, &s, raw).unwrap();

        let inside = oracle_positions(&s);
        for pos in 0..PAYLOAD_BITS {
            let before = (original[pos / 8] >> (pos % 8)) & 1;
            let after = (data[pos / 8] >> (pos % 8)) & 1;
            if !inside.contains(&pos) {
                prop_assert_eq!(before, after, "bit {} changed outside the signal", pos);
            }
        }
    }

    #[test]
    fn extract_then_insert_leaves_the_payload_unchanged(
        s in any_signal(),
        original in any::<[u8; PAYLOAD_BYTES]>(),
    ) {
        let mut data = original;
        let raw = extract_raw(&data, &s).unwrap();
        insert_raw(&mut data, &s, raw).unwrap();
        prop_assert_eq!(data, original);
    }

    #[test]
    fn required_bytes_is_exactly_the_span_the_signal_touches(
        s in any_signal(),
        data in any::<[u8; PAYLOAD_BYTES]>(),
    ) {
        let last_byte = oracle_positions(&s).into_iter().max().unwrap() / 8;
        let required = required_bytes(&s);
        prop_assert_eq!(required, last_byte + 1);

        prop_assert!(extract_raw(&data[..required], &s).is_ok());
        prop_assert!(extract_raw(&data[..required - 1], &s).is_err());
    }

    #[test]
    fn scale_applies_sign_extension_from_the_signal_width(
        (mut s, raw) in signal_and_raw(),
        (factor, offset) in scaling(),
    ) {
        s.factor = factor;
        s.offset = offset;
        let numeric = match s.value_type {
            ValueType::Unsigned => i128::from(raw),
            ValueType::Signed => sign_extend(raw, s.length),
        };
        prop_assert_eq!(s.scale(raw), numeric as f64 * factor + offset);
    }

    #[test]
    fn unscale_inverts_scale_wherever_the_arithmetic_can_round_back(
        (mut s, bits) in signal_and_raw(),
        (factor, offset) in scaling(),
    ) {
        // Beyond ~48 bits the float error in `raw * factor` can exceed half
        // a raw step, so the inverse is not expected to recover `raw` there.
        let length = s.length.min(48);
        s.length = length;
        s.factor = factor;
        s.offset = offset;
        let raw = bits & mask(length);
        prop_assert_eq!(s.unscale(s.scale(raw)), Some(raw));
    }

    #[test]
    fn a_raw_value_too_wide_for_the_signal_is_always_rejected(
        s in any_signal().prop_filter("needs headroom above the width", |s| s.length < 64),
        extra in 1..=u64::MAX,
    ) {
        let mut data = [0u8; PAYLOAD_BYTES];
        // Any value with a bit set at or above `length` must be refused.
        let too_wide = (1 << s.length) | (extra << s.length) | (extra & mask(s.length));
        prop_assert!(insert_raw(&mut data, &s, too_wide).is_err());
        prop_assert_eq!(data, [0u8; PAYLOAD_BYTES]);
    }

    #[test]
    fn both_byte_orders_agree_on_whole_bytes_and_single_bits(
        byte in 0..PAYLOAD_BYTES as u16,
        bit in 0..8u16,
        data in any::<[u8; PAYLOAD_BYTES]>(),
    ) {
        let intel = signal(byte * 8, 8, ByteOrder::Intel, ValueType::Unsigned);
        let motorola = signal(byte * 8 + 7, 8, ByteOrder::Motorola, ValueType::Unsigned);
        prop_assert_eq!(extract_raw(&data, &intel).unwrap(), extract_raw(&data, &motorola).unwrap());

        let pos = byte * 8 + bit;
        let intel_bit = signal(pos, 1, ByteOrder::Intel, ValueType::Unsigned);
        let motorola_bit = signal(pos, 1, ByteOrder::Motorola, ValueType::Unsigned);
        prop_assert_eq!(
            extract_raw(&data, &intel_bit).unwrap(),
            extract_raw(&data, &motorola_bit).unwrap()
        );
    }

    #[test]
    fn a_signal_survives_a_round_trip_through_dbc_text(
        (mut s, _) in signal_and_raw(),
        (factor, offset) in scaling(),
        min in -1e6..1e6f64,
        max in -1e6..1e6f64,
    ) {
        s.name = "Rt".into();
        s.factor = factor;
        s.offset = offset;
        s.min = min;
        s.max = max;
        s.unit = "u".into();

        let order = match s.byte_order { ByteOrder::Intel => 1, ByteOrder::Motorola => 0 };
        let sign = match s.value_type { ValueType::Unsigned => '+', ValueType::Signed => '-' };
        let text = format!(
            "BO_ 1 M: {PAYLOAD_BYTES} X\n SG_ {} : {}|{}@{order}{sign} ({},{}) [{}|{}] \"{}\" X\n",
            s.name, s.start_bit, s.length, s.factor, s.offset, s.min, s.max, s.unit,
        );
        let db = dbc::parse(&text).unwrap();
        let parsed = &db.messages().next().unwrap().signals[0];
        prop_assert_eq!(parsed, &s);
    }
}
