//! Bit extraction: pulling a signal's raw integer out of a CAN payload.
//!
//! No I/O, no allocation, no knowledge of DBC text. I keep this module small
//! on purpose — it is the one the correctness claim rests on, so it has to
//! stay easy to property-test and to compare against a reference decoder.
//!
//! Bit numbering, which is the whole subtlety: byte `n` of the payload holds
//! positions `8n..=8n+7`, and `8n+7` is that byte's most-significant bit.
//! Intel signals name their LSB in `start_bit` and ascend. Motorola signals
//! name their MSB and descend within a byte; on leaving bit 0 they continue
//! at bit 7 of the *next* byte, which is a `+15` jump in absolute position.
//!
//! Relies on the parser's invariant that `1 <= signal.length <= 64`.

use crate::dbc::types::{ByteOrder, Signal};
use crate::error::{Error, Result};

/// How many payload bytes a signal needs in order to decode.
///
/// Split by byte order because the two walks run in opposite directions
/// from the same `start_bit` and so touch different byte spans.
pub fn required_bytes(signal: &Signal) -> usize {
    let start = usize::from(signal.start_bit);
    let length = usize::from(signal.length);

    match signal.byte_order {
        ByteOrder::Intel => (start + length - 1) / 8 + 1,
        ByteOrder::Motorola => {
            let bits_in_first_byte = (start % 8) + 1;
            let remaining = length.saturating_sub(bits_in_first_byte);
            start / 8 + remaining.div_ceil(8) + 1
        }
    }
}

/// Extract a signal's raw, unscaled integer from a payload.
///
/// Returns [`Error::SignalOutOfBounds`] if the payload is shorter than the
/// signal needs. That is a legitimate runtime condition, not a bug: a DBC's
/// declared DLC can exceed what a given frame actually carried.
pub fn extract_raw(data: &[u8], signal: &Signal) -> Result<u64> {
    debug_assert!(
        (1..=64).contains(&signal.length),
        "signal length invariant violated upstream"
    );

    let required = required_bytes(signal);
    if required > data.len() {
        return Err(Error::SignalOutOfBounds {
            signal: signal.name.clone(),
            required,
            available: data.len(),
        });
    }

    let start = usize::from(signal.start_bit);
    let length = usize::from(signal.length);

    Ok(match signal.byte_order {
        ByteOrder::Intel => extract_intel(data, start, length),
        ByteOrder::Motorola => extract_motorola(data, start, length),
    })
}

/// Extract and scale a signal to its physical value.
pub fn decode_signal(data: &[u8], signal: &Signal) -> Result<f64> {
    Ok(signal.scale(extract_raw(data, signal)?))
}

/// The position after `pos` in a Motorola walk.
///
/// Shared with the encoder so the two directions cannot drift apart. The
/// walk itself is checked against an independent formulation in the
/// property tests and against `cantools` in the differential harness.
pub(crate) fn motorola_step(pos: usize) -> usize {
    // Leaving bit 0: cross to the next byte (+8) and climb to its MSB (+7).
    if pos % 8 == 0 { pos + 15 } else { pos - 1 }
}

fn bit_at(data: &[u8], pos: usize) -> u64 {
    u64::from((data[pos / 8] >> (pos % 8)) & 1)
}

fn extract_intel(data: &[u8], start: usize, length: usize) -> u64 {
    let mut raw = 0u64;
    for i in 0..length {
        raw |= bit_at(data, start + i) << i;
    }
    raw
}

fn extract_motorola(data: &[u8], start: usize, length: usize) -> u64 {
    let mut raw = 0u64;
    let mut pos = start;
    for _ in 0..length {
        raw = (raw << 1) | bit_at(data, pos);
        pos = motorola_step(pos);
    }
    raw
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dbc::types::{Multiplexing, ValueType};

    fn sig(start_bit: u16, length: u8, byte_order: ByteOrder) -> Signal {
        Signal {
            name: "T".into(),
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
        }
    }

    #[test]
    fn intel_reads_a_byte_aligned_word_little_endian() {
        let data = [0x34, 0x12, 0, 0, 0, 0, 0, 0];
        let s = sig(0, 16, ByteOrder::Intel);
        assert_eq!(extract_raw(&data, &s).unwrap(), 0x1234);
    }

    #[test]
    fn motorola_reads_a_byte_aligned_word_big_endian() {
        // Same raw result as the Intel case, from the reversed byte layout.
        let data = [0x12, 0x34, 0, 0, 0, 0, 0, 0];
        let s = sig(7, 16, ByteOrder::Motorola);
        assert_eq!(extract_raw(&data, &s).unwrap(), 0x1234);
    }

    #[test]
    fn motorola_straddles_a_byte_boundary_from_an_unaligned_start() {
        // start_bit 11 is byte 1, bit 3: the low nibble of byte 1 followed by
        // the high nibble of byte 2.
        let data = [0x00, 0x0F, 0xF0, 0, 0, 0, 0, 0];
        let s = sig(11, 8, ByteOrder::Motorola);
        assert_eq!(extract_raw(&data, &s).unwrap(), 0xFF);
    }

    #[test]
    fn intel_straddles_a_byte_boundary_from_an_unaligned_start() {
        // start_bit 4, length 8: high nibble of byte 0 is the low nibble of
        // the result, low nibble of byte 1 is the high nibble.
        let data = [0xA0, 0x0B, 0, 0, 0, 0, 0, 0];
        let s = sig(4, 8, ByteOrder::Intel);
        assert_eq!(extract_raw(&data, &s).unwrap(), 0xBA);
    }

    #[test]
    fn intel_and_motorola_disagree_on_the_same_payload() {
        // Guards against a change that makes both branches identical, which
        // would still pass every single-order test.
        let data = [0x12, 0x34, 0, 0, 0, 0, 0, 0];
        let i = extract_raw(&data, &sig(0, 16, ByteOrder::Intel)).unwrap();
        let m = extract_raw(&data, &sig(7, 16, ByteOrder::Motorola)).unwrap();
        assert_eq!(i, 0x3412);
        assert_eq!(m, 0x1234);
        assert_ne!(i, m);
    }

    #[test]
    fn single_bit_signals_decode_at_every_position_in_a_byte() {
        let data = [0b1010_1010, 0, 0, 0, 0, 0, 0, 0];
        for bit in 0..8u16 {
            let expected = u64::from(bit % 2 == 1);
            assert_eq!(
                extract_raw(&data, &sig(bit, 1, ByteOrder::Intel)).unwrap(),
                expected,
                "intel bit {bit}"
            );
            assert_eq!(
                extract_raw(&data, &sig(bit, 1, ByteOrder::Motorola)).unwrap(),
                expected,
                "motorola bit {bit}"
            );
        }
    }

    #[test]
    fn required_bytes_differs_between_orders_for_the_same_start_bit() {
        // From bit 7, 16 bits: Intel climbs into byte 2, Motorola stays in 0..=1.
        assert_eq!(required_bytes(&sig(7, 16, ByteOrder::Intel)), 3);
        assert_eq!(required_bytes(&sig(7, 16, ByteOrder::Motorola)), 2);
    }

    #[test]
    fn required_bytes_for_motorola_unaligned_starts() {
        // start 11 (byte 1 bit 3) has 4 bits left in its byte; 8 bits need
        // one more byte, so bytes 1..=2 — 3 bytes total.
        assert_eq!(required_bytes(&sig(11, 8, ByteOrder::Motorola)), 3);
        // 4 bits from bit 11 fit entirely in byte 1.
        assert_eq!(required_bytes(&sig(11, 4, ByteOrder::Motorola)), 2);
        // One bit at position 0 needs only byte 0.
        assert_eq!(required_bytes(&sig(0, 1, ByteOrder::Motorola)), 1);
    }

    #[test]
    fn a_signal_longer_than_the_payload_is_an_error_not_a_panic() {
        let data = [0x00, 0x01];
        let s = sig(0, 32, ByteOrder::Intel);
        assert!(matches!(
            extract_raw(&data, &s),
            Err(Error::SignalOutOfBounds {
                required: 4,
                available: 2,
                ..
            })
        ));
    }

    #[test]
    fn a_motorola_signal_longer_than_the_payload_is_an_error_not_a_panic() {
        let data = [0x00];
        let s = sig(7, 16, ByteOrder::Motorola);
        assert!(matches!(
            extract_raw(&data, &s),
            Err(Error::SignalOutOfBounds {
                required: 2,
                available: 1,
                ..
            })
        ));
    }

    #[test]
    fn decode_signal_applies_scaling_to_the_extracted_bits() {
        let data = [0x34, 0x12, 0, 0, 0, 0, 0, 0];
        let mut s = sig(0, 16, ByteOrder::Intel);
        s.factor = 0.25;
        assert_eq!(decode_signal(&data, &s).unwrap(), 1165.0);
    }

    #[test]
    fn a_full_width_64_bit_signal_extracts_without_overflow() {
        let data = [0xFF; 8];
        assert_eq!(
            extract_raw(&data, &sig(0, 64, ByteOrder::Intel)).unwrap(),
            u64::MAX
        );
        assert_eq!(
            extract_raw(&data, &sig(7, 64, ByteOrder::Motorola)).unwrap(),
            u64::MAX
        );
    }

    #[test]
    fn a_64_bit_motorola_signal_preserves_byte_order() {
        let data = [0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08];
        assert_eq!(
            extract_raw(&data, &sig(7, 64, ByteOrder::Motorola)).unwrap(),
            0x0102_0304_0506_0708
        );
        assert_eq!(
            extract_raw(&data, &sig(0, 64, ByteOrder::Intel)).unwrap(),
            0x0807_0605_0403_0201
        );
    }
}
