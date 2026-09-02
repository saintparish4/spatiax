//! Signal, message, and database definitions parsed from a DBC file.
//!
//! Everything here describes what a signal *is*; `decode.rs` holds only the
//! arithmetic that pulls it out of a payload. I keep that seam clean so the
//! bit extraction can be tested in isolation from parsing and scaling.
//!
//! Invariant: a `Signal` reaching the decoder always has `1 <= length <= 64`.
//! The parser enforces this; the decoder does not re-check it.

use crate::frame::CanId;
use std::collections::{BTreeMap, HashMap};

/// Bit ordering of a signal within its frame — the DBC `@` field.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ByteOrder {
    /// `@1`. Little-endian: `start_bit` is the signal's least-significant
    /// bit and positions ascend.
    Intel,
    /// `@0`. Big-endian: `start_bit` is the signal's *most*-significant bit
    /// and positions descend within a byte, jumping to bit 7 of the next
    /// byte on leaving bit 0.
    Motorola,
}

/// Whether a signal's raw value is two's-complement.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ValueType {
    /// DBC `+`.
    Unsigned,
    /// DBC `-`.
    Signed,
}

/// A signal's role in a multiplexed message.
///
/// Simple multiplexing only: one `M` per message and each `m<N>` signal
/// present for exactly one raw value of it. The parser rejects extended
/// multiplexing (`m<N>M`, `SG_MUL_VAL_` ranges) rather than load a file it
/// would then decode wrongly.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Multiplexing {
    /// Always present.
    None,
    /// DBC `M` — this signal's raw value selects which multiplexed signals apply.
    Multiplexor,
    /// DBC `m<N>` — present only when the multiplexor's raw value equals `N`.
    Multiplexed(u16),
}

/// Labels for particular raw values of a signal — the DBC `VAL_` record.
///
/// Keys are matched against the raw value after sign interpretation, so a
/// signed 8-bit signal reading `0xFF` matches a key of `-1`, never `255`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ValueTable {
    /// Sorted by key, keys unique.
    entries: Vec<(i64, String)>,
}

impl ValueTable {
    /// Build from `(key, label)` pairs. A later pair for the same key
    /// replaces an earlier one, which is how `cantools` reads a record too.
    pub fn new(entries: impl IntoIterator<Item = (i64, String)>) -> Self {
        let deduplicated: BTreeMap<i64, String> = entries.into_iter().collect();
        Self {
            entries: deduplicated.into_iter().collect(),
        }
    }

    /// The label for a sign-interpreted raw value, if one is defined.
    pub fn label(&self, numeric: i64) -> Option<&str> {
        self.entries
            .binary_search_by_key(&numeric, |(key, _)| *key)
            .ok()
            .map(|index| self.entries[index].1.as_str())
    }

    /// `(key, label)` pairs in ascending key order.
    pub fn iter(&self) -> impl Iterator<Item = (i64, &str)> {
        self.entries
            .iter()
            .map(|(key, label)| (*key, label.as_str()))
    }

    /// Number of labelled values.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Whether no values are labelled.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

/// One signal within a CAN message.
#[derive(Debug, Clone, PartialEq)]
pub struct Signal {
    /// Signal name as written in the DBC.
    pub name: String,
    /// Bit position of the signal's LSB (Intel) or MSB (Motorola).
    pub start_bit: u16,
    /// Width in bits. Always in `1..=64`.
    pub length: u8,
    /// Bit ordering.
    pub byte_order: ByteOrder,
    /// Signed or unsigned interpretation of the raw value.
    pub value_type: ValueType,
    /// Physical value = raw * factor + offset.
    pub factor: f64,
    /// Physical value = raw * factor + offset.
    pub offset: f64,
    /// Declared minimum physical value. Not enforced during decode.
    pub min: f64,
    /// Declared maximum physical value. Not enforced during decode.
    pub max: f64,
    /// Engineering unit, e.g. `"rpm"`. May be empty.
    pub unit: String,
    /// Multiplexing role.
    pub multiplexing: Multiplexing,
    /// Labels for particular raw values. Empty for most signals.
    pub value_table: ValueTable,
}

impl Signal {
    /// The label the value table gives a raw value, if any.
    pub fn label(&self, raw: u64) -> Option<&str> {
        let numeric = match self.value_type {
            ValueType::Unsigned => i64::try_from(raw).ok()?,
            ValueType::Signed => {
                let shift = 64 - u32::from(self.length);
                ((raw << shift) as i64) >> shift
            }
        };
        self.value_table.label(numeric)
    }

    /// Apply sign interpretation and the linear transform to a raw value.
    ///
    /// For signed signals I shift the value up so its MSB lands in bit 63 and
    /// arithmetic-shift back down. That is branch-free and, unlike masking
    /// with `!((1 << length) - 1)`, stays correct at `length == 64`.
    pub fn scale(&self, raw: u64) -> f64 {
        let numeric = match self.value_type {
            ValueType::Unsigned => raw as f64,
            ValueType::Signed => {
                let shift = 64 - u32::from(self.length);
                (((raw << shift) as i64) >> shift) as f64
            }
        };
        numeric * self.factor + self.offset
    }

    /// Invert [`scale`](Self::scale): the raw bit pattern whose physical
    /// value is nearest to `value`.
    ///
    /// Returns `None` when `value` is not finite or the nearest raw integer
    /// does not fit the signal's width. Signed results come back as their
    /// two's-complement pattern in `length` bits, ready for insertion.
    pub fn unscale(&self, value: f64) -> Option<u64> {
        let numeric = ((value - self.offset) / self.factor).round();
        if !numeric.is_finite() {
            return None;
        }

        let bits = u32::from(self.length);
        match self.value_type {
            ValueType::Unsigned => {
                // 2^64 is exactly representable, so this bound also holds at length 64.
                let limit = 2f64.powi(bits as i32);
                (numeric >= 0.0 && numeric < limit).then_some(numeric as u64)
            }
            ValueType::Signed => {
                let limit = 2f64.powi(bits as i32 - 1);
                (numeric >= -limit && numeric < limit)
                    .then(|| (numeric as i64 as u64) & (u64::MAX >> (64 - bits)))
            }
        }
    }
}

/// A CAN message definition: an identifier and the signals it carries.
#[derive(Debug, Clone, PartialEq)]
pub struct Message {
    /// Identifier, with the DBC extended-flag already interpreted.
    pub id: CanId,
    /// Message name as written in the DBC.
    pub name: String,
    /// Declared payload length in bytes.
    pub dlc: u8,
    /// Transmitting node, or empty.
    pub sender: String,
    /// Signals, in the order the DBC listed them.
    pub signals: Vec<Signal>,
}

impl Message {
    /// The signal named `name`, if the message carries one.
    pub fn signal(&self, name: &str) -> Option<&Signal> {
        self.signals.iter().find(|s| s.name == name)
    }

    /// Mutable access to the signal named `name`.
    pub fn signal_mut(&mut self, name: &str) -> Option<&mut Signal> {
        self.signals.iter_mut().find(|s| s.name == name)
    }

    /// The signal whose raw value selects which multiplexed signals apply.
    pub fn multiplexor(&self) -> Option<&Signal> {
        self.signals
            .iter()
            .find(|s| s.multiplexing == Multiplexing::Multiplexor)
    }
}

/// A parsed DBC database, indexed by identifier for O(1) frame lookup.
#[derive(Debug, Clone, Default)]
pub struct Database {
    messages: HashMap<CanId, Message>,
}

impl Database {
    /// An empty database.
    pub fn new() -> Self {
        Self::default()
    }

    /// Insert a message, replacing any existing definition for its id.
    pub fn insert(&mut self, message: Message) {
        self.messages.insert(message.id, message);
    }

    /// Look up the definition for an identifier.
    pub fn message(&self, id: CanId) -> Option<&Message> {
        self.messages.get(&id)
    }

    /// Mutable access to the definition for an identifier.
    pub fn message_mut(&mut self, id: CanId) -> Option<&mut Message> {
        self.messages.get_mut(&id)
    }

    /// Number of messages defined.
    pub fn len(&self) -> usize {
        self.messages.len()
    }

    /// Whether the database defines no messages.
    pub fn is_empty(&self) -> bool {
        self.messages.is_empty()
    }

    /// Total signals across all messages.
    pub fn signal_count(&self) -> usize {
        self.messages.values().map(|m| m.signals.len()).sum()
    }

    /// Every message, in unspecified order.
    pub fn messages(&self) -> impl Iterator<Item = &Message> {
        self.messages.values()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sig(length: u8, value_type: ValueType, factor: f64, offset: f64) -> Signal {
        Signal {
            name: "T".into(),
            start_bit: 0,
            length,
            byte_order: ByteOrder::Intel,
            value_type,
            factor,
            offset,
            min: 0.0,
            max: 0.0,
            unit: String::new(),
            multiplexing: Multiplexing::None,
            value_table: ValueTable::default(),
        }
    }

    fn msg(id: u16, signals: Vec<Signal>) -> Message {
        Message {
            id: CanId::Standard(id),
            name: format!("M{id}"),
            dlc: 8,
            sender: String::new(),
            signals,
        }
    }

    #[test]
    fn unsigned_scaling_applies_factor_then_offset() {
        // 100 * 0.5 - 40 = 10.0, the standard automotive temperature encoding.
        let s = sig(8, ValueType::Unsigned, 0.5, -40.0);
        assert_eq!(s.scale(100), 10.0);
    }

    #[test]
    fn signed_scaling_sign_extends_from_the_signal_width() {
        // 0xFFF as a 12-bit two's-complement value is -1, not 4095.
        let s = sig(12, ValueType::Signed, 1.0, 0.0);
        assert_eq!(s.scale(0xFFF), -1.0);
    }

    #[test]
    fn signed_scaling_leaves_positive_values_alone() {
        let s = sig(12, ValueType::Signed, 1.0, 0.0);
        assert_eq!(s.scale(0x7FF), 2047.0);
    }

    #[test]
    fn signed_scaling_is_correct_at_full_64_bit_width() {
        let s = sig(64, ValueType::Signed, 1.0, 0.0);
        assert_eq!(s.scale(u64::MAX), -1.0);
    }

    #[test]
    fn signed_scaling_is_correct_at_single_bit_width() {
        let s = sig(1, ValueType::Signed, 1.0, 0.0);
        assert_eq!(s.scale(1), -1.0);
        assert_eq!(s.scale(0), 0.0);
    }

    #[test]
    fn unscale_inverts_factor_and_offset_with_rounding() {
        let s = sig(8, ValueType::Unsigned, 0.5, -40.0);
        assert_eq!(s.unscale(10.0), Some(100));
        // 10.2 -> raw 100.4, rounds to 100; 10.3 -> 100.6, rounds to 101.
        assert_eq!(s.unscale(10.2), Some(100));
        assert_eq!(s.unscale(10.3), Some(101));
    }

    #[test]
    fn unscale_rejects_values_outside_the_unsigned_width() {
        let s = sig(8, ValueType::Unsigned, 1.0, 0.0);
        assert_eq!(s.unscale(255.0), Some(255));
        assert_eq!(s.unscale(256.0), None);
        assert_eq!(s.unscale(-1.0), None);
    }

    #[test]
    fn unscale_produces_a_twos_complement_pattern_for_signed_signals() {
        let s = sig(12, ValueType::Signed, 1.0, 0.0);
        assert_eq!(s.unscale(-1.0), Some(0xFFF));
        assert_eq!(s.unscale(-2048.0), Some(0x800));
        assert_eq!(s.unscale(2047.0), Some(0x7FF));
        assert_eq!(s.unscale(2048.0), None);
        assert_eq!(s.unscale(-2049.0), None);
    }

    #[test]
    fn unscale_is_correct_at_the_width_extremes() {
        let one = sig(1, ValueType::Signed, 1.0, 0.0);
        assert_eq!(one.unscale(-1.0), Some(1));
        assert_eq!(one.unscale(1.0), None);

        let wide = sig(64, ValueType::Unsigned, 1.0, 0.0);
        assert_eq!(wide.unscale(0.0), Some(0));
        assert_eq!(wide.unscale(2f64.powi(64)), None);

        let wide_signed = sig(64, ValueType::Signed, 1.0, 0.0);
        assert_eq!(wide_signed.unscale(-1.0), Some(u64::MAX));
    }

    #[test]
    fn unscale_rejects_non_finite_input() {
        let s = sig(16, ValueType::Unsigned, 1.0, 0.0);
        assert_eq!(s.unscale(f64::NAN), None);
        assert_eq!(s.unscale(f64::INFINITY), None);
    }

    fn table(entries: &[(i64, &str)]) -> ValueTable {
        ValueTable::new(entries.iter().map(|(k, l)| (*k, l.to_string())))
    }

    #[test]
    fn value_table_sorts_entries_and_keeps_the_last_label_for_a_repeated_key() {
        let t = table(&[(5, "five"), (-1, "minus"), (5, "FIVE")]);
        assert_eq!(t.len(), 2);
        assert_eq!(t.iter().collect::<Vec<_>>(), [(-1, "minus"), (5, "FIVE")]);
        assert_eq!(t.label(5), Some("FIVE"));
        assert_eq!(t.label(4), None);
        assert!(ValueTable::default().is_empty());
    }

    #[test]
    fn labels_match_the_sign_interpreted_raw_value() {
        let mut signed = sig(8, ValueType::Signed, 1.0, 0.0);
        signed.value_table = table(&[(-1, "minus one"), (255, "never")]);
        assert_eq!(signed.label(0xFF), Some("minus one"));
        assert_eq!(signed.label(0x7F), None);

        let mut unsigned = sig(8, ValueType::Unsigned, 1.0, 0.0);
        unsigned.value_table = table(&[(-1, "never"), (255, "max")]);
        assert_eq!(unsigned.label(0xFF), Some("max"));
    }

    #[test]
    fn labels_are_keyed_by_raw_rather_than_physical_value() {
        let mut s = sig(8, ValueType::Unsigned, 0.5, -40.0);
        s.value_table = table(&[(100, "raw hundred"), (10, "physical ten")]);
        assert_eq!(s.label(100), Some("raw hundred"));
    }

    #[test]
    fn a_raw_value_beyond_i64_never_matches_a_label() {
        let mut s = sig(64, ValueType::Unsigned, 1.0, 0.0);
        s.value_table = table(&[(-1, "minus one")]);
        assert_eq!(s.label(u64::MAX), None);
    }

    #[test]
    fn mutable_lookups_reach_the_stored_signal() {
        let mut db = Database::new();
        db.insert(msg(1, vec![sig(8, ValueType::Unsigned, 1.0, 0.0)]));
        db.message_mut(CanId::Standard(1))
            .and_then(|m| m.signal_mut("T"))
            .unwrap()
            .value_table = table(&[(0, "off")]);
        assert_eq!(
            db.message(CanId::Standard(1)).unwrap().signals[0].label(0),
            Some("off")
        );
        assert!(db.message_mut(CanId::Standard(2)).is_none());
        assert!(
            db.message_mut(CanId::Standard(1))
                .unwrap()
                .signal_mut("U")
                .is_none()
        );
    }

    #[test]
    fn signal_count_sums_across_messages() {
        let one = || sig(8, ValueType::Unsigned, 1.0, 0.0);
        let mut db = Database::new();
        db.insert(msg(1, vec![one()]));
        db.insert(msg(2, vec![one(), one()]));
        assert_eq!(db.len(), 2);
        assert_eq!(db.signal_count(), 3);
    }

    #[test]
    fn inserting_the_same_id_twice_replaces_the_definition() {
        let mut db = Database::new();
        db.insert(msg(1, vec![]));
        db.insert(msg(1, vec![sig(8, ValueType::Unsigned, 1.0, 0.0)]));
        assert_eq!(db.len(), 1);
        assert_eq!(db.signal_count(), 1);
    }

    #[test]
    fn empty_database_reports_empty() {
        let db = Database::new();
        assert!(db.is_empty());
        assert_eq!(db.message(CanId::Standard(1)), None);
        assert_eq!(db.messages().count(), 0);
    }
}
