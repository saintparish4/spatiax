//! Bit insertion: writing a signal's raw integer into a CAN payload.
//!
//! The mirror of [`decode`](crate::decode), and held to the same rules: no
//! I/O, no allocation, no knowledge of DBC text. It exists for callers that
//! build frames — test rigs, simulators, replay tools — and because a decoder
//! that can be inverted is a decoder whose bit walk can be property-tested
//! across every layout rather than a handful of vectors.
//!
//! Bits outside the target signal are never touched, so a payload can be
//! assembled one signal at a time in any order.

use crate::dbc::types::{ByteOrder, Signal};
use crate::decode::{motorola_step, required_bytes};
use crate::error::{Error, Result};

/// Write a raw, unscaled integer into a payload at a signal's position.
///
/// Returns [`Error::RawOutOfRange`] if `raw` has bits set above
/// `signal.length`, and [`Error::SignalOutOfBounds`] if the payload is too
/// short for the signal's layout. The payload is unmodified on error.
pub fn insert_raw(data: &mut [u8], signal: &Signal, raw: u64) -> Result<()> {
    let length = usize::from(signal.length);
    if length < 64 && raw >> length != 0 {
        return Err(Error::RawOutOfRange {
            signal: signal.name.clone(),
            length: signal.length,
            raw,
        });
    }

    let required = required_bytes(signal);
    if required > data.len() {
        return Err(Error::SignalOutOfBounds {
            signal: signal.name.clone(),
            required,
            available: data.len(),
        });
    }

    let start = usize::from(signal.start_bit);
    match signal.byte_order {
        ByteOrder::Intel => insert_intel(data, start, length, raw),
        ByteOrder::Motorola => insert_motorola(data, start, length, raw),
    }
    Ok(())
}

/// Scale a physical value to its raw encoding and write it.
///
/// The physical value is rounded to the nearest representable raw integer.
/// Returns the raw value written, or [`Error::ValueOutOfRange`] if no raw
/// integer of the signal's width can represent `value`.
pub fn encode_signal(data: &mut [u8], signal: &Signal, value: f64) -> Result<u64> {
    let raw = signal
        .unscale(value)
        .ok_or_else(|| Error::ValueOutOfRange {
            signal: signal.name.clone(),
            value,
        })?;
    insert_raw(data, signal, raw)?;
    Ok(raw)
}

fn set_bit(data: &mut [u8], pos: usize, bit: u64) {
    let mask = 1u8 << (pos % 8);
    if bit & 1 == 1 {
        data[pos / 8] |= mask;
    } else {
        data[pos / 8] &= !mask;
    }
}

fn insert_intel(data: &mut [u8], start: usize, length: usize, raw: u64) {
    for i in 0..length {
        set_bit(data, start + i, raw >> i);
    }
}

fn insert_motorola(data: &mut [u8], start: usize, length: usize, raw: u64) {
    let mut pos = start;
    for i in (0..length).rev() {
        set_bit(data, pos, raw >> i);
        pos = motorola_step(pos);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dbc::types::{Multiplexing, ValueTable, ValueType};
    use crate::decode::extract_raw;

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
            value_table: ValueTable::default(),
        }
    }

    #[test]
    fn intel_writes_a_byte_aligned_word_little_endian() {
        let mut data = [0u8; 8];
        insert_raw(&mut data, &sig(0, 16, ByteOrder::Intel), 0x1234).unwrap();
        assert_eq!(data, [0x34, 0x12, 0, 0, 0, 0, 0, 0]);
    }

    #[test]
    fn motorola_writes_a_byte_aligned_word_big_endian() {
        let mut data = [0u8; 8];
        insert_raw(&mut data, &sig(7, 16, ByteOrder::Motorola), 0x1234).unwrap();
        assert_eq!(data, [0x12, 0x34, 0, 0, 0, 0, 0, 0]);
    }

    #[test]
    fn motorola_straddles_a_byte_boundary_from_an_unaligned_start() {
        // Inverse of the decode test: 0xFF from bit 11 lands in the low
        // nibble of byte 1 and the high nibble of byte 2.
        let mut data = [0u8; 8];
        insert_raw(&mut data, &sig(11, 8, ByteOrder::Motorola), 0xFF).unwrap();
        assert_eq!(data, [0x00, 0x0F, 0xF0, 0, 0, 0, 0, 0]);
    }

    #[test]
    fn intel_straddles_a_byte_boundary_from_an_unaligned_start() {
        let mut data = [0u8; 8];
        insert_raw(&mut data, &sig(4, 8, ByteOrder::Intel), 0xBA).unwrap();
        assert_eq!(data, [0xA0, 0x0B, 0, 0, 0, 0, 0, 0]);
    }

    #[test]
    fn insertion_clears_ones_as_well_as_setting_them() {
        let mut data = [0xFF; 8];
        insert_raw(&mut data, &sig(8, 8, ByteOrder::Intel), 0x00).unwrap();
        assert_eq!(data, [0xFF, 0x00, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF]);
    }

    #[test]
    fn insertion_leaves_neighbouring_bits_alone() {
        let mut data = [0xFF; 8];
        // Four bits in the middle of byte 0: bits 2..=5.
        insert_raw(&mut data, &sig(2, 4, ByteOrder::Intel), 0b0000).unwrap();
        assert_eq!(data[0], 0b1100_0011);
        assert_eq!(&data[1..], &[0xFF; 7]);
    }

    #[test]
    fn a_raw_value_wider_than_the_signal_is_rejected_without_writing() {
        let mut data = [0u8; 8];
        let err = insert_raw(&mut data, &sig(0, 8, ByteOrder::Intel), 0x100).unwrap_err();
        assert!(matches!(
            err,
            Error::RawOutOfRange {
                length: 8,
                raw: 0x100,
                ..
            }
        ));
        assert_eq!(data, [0u8; 8]);
    }

    #[test]
    fn a_signal_longer_than_the_payload_is_an_error_not_a_panic() {
        let mut data = [0u8; 2];
        let err = insert_raw(&mut data, &sig(0, 32, ByteOrder::Intel), 1).unwrap_err();
        assert!(matches!(
            err,
            Error::SignalOutOfBounds {
                required: 4,
                available: 2,
                ..
            }
        ));
    }

    #[test]
    fn full_width_64_bit_signals_insert_without_overflow() {
        let mut data = [0u8; 8];
        insert_raw(&mut data, &sig(0, 64, ByteOrder::Intel), u64::MAX).unwrap();
        assert_eq!(data, [0xFF; 8]);

        let mut data = [0u8; 8];
        insert_raw(
            &mut data,
            &sig(7, 64, ByteOrder::Motorola),
            0x0102_0304_0506_0708,
        )
        .unwrap();
        assert_eq!(data, [0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08]);
    }

    #[test]
    fn encode_signal_scales_and_returns_the_raw_written() {
        let mut data = [0u8; 8];
        let mut s = sig(16, 8, ByteOrder::Intel);
        s.factor = 0.5;
        s.offset = -40.0;
        assert_eq!(encode_signal(&mut data, &s, 10.0).unwrap(), 100);
        assert_eq!(data[2], 100);
    }

    #[test]
    fn encode_signal_rejects_an_unrepresentable_value() {
        let mut data = [0u8; 8];
        let s = sig(0, 8, ByteOrder::Intel);
        assert!(matches!(
            encode_signal(&mut data, &s, 300.0),
            Err(Error::ValueOutOfRange { .. })
        ));
        assert_eq!(data, [0u8; 8]);
    }

    #[test]
    fn insert_then_extract_returns_the_original_at_a_few_awkward_layouts() {
        // The exhaustive version of this lives in the property tests.
        let cases = [
            (sig(3, 13, ByteOrder::Intel), 0x1ABC),
            (sig(13, 13, ByteOrder::Motorola), 0x1ABC),
            (sig(62, 2, ByteOrder::Intel), 0b10),
            (sig(56, 1, ByteOrder::Motorola), 1),
        ];
        for (s, raw) in cases {
            let mut data = [0u8; 8];
            insert_raw(&mut data, &s, raw).unwrap();
            assert_eq!(extract_raw(&data, &s).unwrap(), raw, "{s:?}");
        }
    }
}
