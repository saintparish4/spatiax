//! Layout problems in a database that parsed cleanly.
//!
//! The parser is lenient on purpose: a DBC with one broken message is still
//! useful for the other hundred. But a broken layout decodes silently — a
//! signal that runs past the DLC just never fits, and two overlapping
//! signals just read each other's bits — so `check` exists to say so out
//! loud. These are the two problems `cantools` rejects in strict mode.

use std::fmt;

use crate::decode::{bit_positions, required_bytes};
use crate::frame::MAX_FRAME_LEN;

use super::types::{Database, Message, Multiplexing, Signal};

/// One problem found by [`check`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Problem {
    /// A signal needs more bytes than its message declares, so it can never
    /// decode from a frame of the declared length.
    SignalPastDlc {
        /// The message the signal belongs to.
        message: String,
        /// The signal that does not fit.
        signal: String,
        /// Bytes the signal's layout requires.
        required: usize,
        /// The message's declared length.
        dlc: u8,
    },
    /// Two signals that decode together share at least one bit.
    Overlap {
        /// The message both signals belong to.
        message: String,
        /// The earlier of the two signals in DBC order.
        first: String,
        /// The later of the two.
        second: String,
    },
}

impl fmt::Display for Problem {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::SignalPastDlc {
                message,
                signal,
                required,
                dlc,
            } => write!(
                f,
                "{message}: signal `{signal}` needs {required} byte(s) but the message declares {dlc}"
            ),
            Self::Overlap {
                message,
                first,
                second,
            } => write!(f, "{message}: signals `{first}` and `{second}` overlap"),
        }
    }
}

/// Every layout problem in the database, in message order.
pub fn check(db: &Database) -> Vec<Problem> {
    db.messages().flat_map(check_message).collect()
}

fn check_message(message: &Message) -> Vec<Problem> {
    let mut problems: Vec<Problem> = message
        .signals
        .iter()
        .filter_map(|signal| past_dlc(message, signal))
        .collect();

    let masks: Vec<BitMask> = message.signals.iter().map(BitMask::of).collect();
    for (i, a) in message.signals.iter().enumerate() {
        for (j, b) in message.signals.iter().enumerate().skip(i + 1) {
            if decode_together(a, b) && masks[i].intersects(&masks[j]) {
                problems.push(Problem::Overlap {
                    message: message.name.clone(),
                    first: a.name.clone(),
                    second: b.name.clone(),
                });
            }
        }
    }
    problems
}

fn past_dlc(message: &Message, signal: &Signal) -> Option<Problem> {
    let required = required_bytes(signal);
    (required > usize::from(message.dlc)).then(|| Problem::SignalPastDlc {
        message: message.name.clone(),
        signal: signal.name.clone(),
        required,
        dlc: message.dlc,
    })
}

/// Signals on different multiplexor pages never appear in the same frame,
/// so they may share bits. Everything else can.
fn decode_together(a: &Signal, b: &Signal) -> bool {
    match (a.multiplexing, b.multiplexing) {
        (Multiplexing::Multiplexed(x), Multiplexing::Multiplexed(y)) => x == y,
        _ => true,
    }
}

/// One bit per position of the largest possible frame. Positions beyond it
/// cannot decode from any frame and are left out; `past_dlc` reports those.
struct BitMask([u64; MAX_FRAME_LEN / 8]);

impl BitMask {
    fn of(signal: &Signal) -> Self {
        let mut words = [0u64; MAX_FRAME_LEN / 8];
        for pos in bit_positions(signal).filter(|&pos| pos < 8 * MAX_FRAME_LEN) {
            words[pos / 64] |= 1 << (pos % 64);
        }
        Self(words)
    }

    fn intersects(&self, other: &Self) -> bool {
        self.0.iter().zip(&other.0).any(|(a, b)| a & b != 0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dbc::parse;

    fn problems(dbc: &str) -> Vec<String> {
        check(&parse(dbc).unwrap())
            .iter()
            .map(ToString::to_string)
            .collect()
    }

    #[test]
    fn a_clean_database_has_no_problems() {
        assert!(problems(include_str!("../../fixtures/gt3_sample.dbc")).is_empty());
    }

    #[test]
    fn a_signal_running_past_the_dlc_is_reported_with_both_sizes() {
        let dbc = "BO_ 256 Short: 2 ECU\n \
                   SG_ Fits : 0|16@1+ (1,0) [0|0] \"\" X\n \
                   SG_ Spills : 16|8@1+ (1,0) [0|0] \"\" X\n";
        assert_eq!(
            problems(dbc),
            ["Short: signal `Spills` needs 3 byte(s) but the message declares 2"]
        );
    }

    #[test]
    fn overlapping_signals_are_reported_once_per_pair_in_dbc_order() {
        let dbc = "BO_ 256 Tangled: 8 ECU\n \
                   SG_ A : 0|16@1+ (1,0) [0|0] \"\" X\n \
                   SG_ B : 15|8@1+ (1,0) [0|0] \"\" X\n \
                   SG_ C : 22|1@1+ (1,0) [0|0] \"\" X\n";
        assert_eq!(
            problems(dbc),
            [
                "Tangled: signals `A` and `B` overlap",
                "Tangled: signals `B` and `C` overlap",
            ]
        );
    }

    #[test]
    fn motorola_overlap_is_judged_on_the_bits_actually_walked() {
        // Motorola from bit 7 for 16 bits covers bytes 0..=1 — not byte 2,
        // which the Intel reading of the same numbers would reach.
        let touching = "BO_ 1 M: 8 E\n \
                        SG_ A : 7|16@0+ (1,0) [0|0] \"\" X\n \
                        SG_ B : 23|8@0+ (1,0) [0|0] \"\" X\n";
        assert!(problems(touching).is_empty());

        let crossing = "BO_ 1 M: 8 E\n \
                        SG_ A : 7|16@0+ (1,0) [0|0] \"\" X\n \
                        SG_ B : 8|1@0+ (1,0) [0|0] \"\" X\n";
        assert_eq!(problems(crossing), ["M: signals `A` and `B` overlap"]);
    }

    #[test]
    fn signals_on_different_pages_may_share_bits_but_not_with_the_multiplexor() {
        let paged = "BO_ 1 P: 8 E\n \
                     SG_ Mux M : 0|8@1+ (1,0) [0|0] \"\" X\n \
                     SG_ Page0 m0 : 8|16@1+ (1,0) [0|0] \"\" X\n \
                     SG_ Page1 m1 : 8|16@1+ (1,0) [0|0] \"\" X\n \
                     SG_ Page1b m1 : 16|8@1+ (1,0) [0|0] \"\" X\n \
                     SG_ Plain : 4|8@1+ (1,0) [0|0] \"\" X\n";
        assert_eq!(
            problems(paged),
            [
                "P: signals `Mux` and `Plain` overlap",
                "P: signals `Page0` and `Plain` overlap",
                "P: signals `Page1` and `Page1b` overlap",
                "P: signals `Page1` and `Plain` overlap",
            ]
        );
    }

    #[test]
    fn a_start_bit_beyond_any_frame_is_past_the_dlc_rather_than_a_panic() {
        let dbc = "BO_ 1 Far: 8 E\n \
                   SG_ A : 600|8@1+ (1,0) [0|0] \"\" X\n \
                   SG_ B : 600|8@1+ (1,0) [0|0] \"\" X\n";
        assert_eq!(
            problems(dbc),
            [
                "Far: signal `A` needs 76 byte(s) but the message declares 8",
                "Far: signal `B` needs 76 byte(s) but the message declares 8",
            ]
        );
    }
}
