//! DBC database: types, parsing, and frame decoding.
//!
//! `decode_frame` returns `None` rather than an error for an unknown
//! identifier. On a real bus most traffic is not described by any one DBC,
//! so an unrecognised frame is an ordinary event, not a failure. A signal
//! that does not fit the frame *is* an error, reported per signal so one
//! short frame cannot discard the signals that did decode.

pub mod parser;
pub mod types;

pub use parser::parse;
pub use types::{ByteOrder, Database, Message, Multiplexing, Signal, ValueType};

use crate::decode::extract_raw;
use crate::error::Result;
use crate::frame::CanFrame;

/// One decoded signal: its definition, its raw bits, and its physical value.
///
/// I carry `raw` alongside `value` so an extraction bug and a scaling bug
/// are distinguishable, and so a reference comparison can be done on
/// integers where float tolerance is not a confound.
#[derive(Debug, Clone, PartialEq)]
pub struct Decoded<'a> {
    /// The signal definition this came from.
    pub signal: &'a Signal,
    /// The unscaled, unsigned bit pattern.
    pub raw: u64,
    /// The physical value after sign interpretation and scaling.
    pub value: f64,
}

impl Database {
    /// Decode every signal of the message matching this frame's identifier.
    ///
    /// Returns `None` if no message is defined for the identifier. The
    /// iterator yields one item per signal in DBC order; a signal that does
    /// not fit the payload yields `Err` without affecting its neighbours.
    /// Nothing here allocates.
    pub fn decode_frame<'a>(
        &'a self,
        frame: &'a CanFrame,
    ) -> Option<impl Iterator<Item = Result<Decoded<'a>>> + 'a> {
        let message = self.message(frame.id())?;
        let data = frame.data();

        Some(message.signals.iter().map(move |signal| {
            let raw = extract_raw(data, signal)?;
            Ok(Decoded {
                signal,
                raw,
                value: signal.scale(raw),
            })
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::frame::CanId;

    const DBC: &str = "BO_ 256 EngineData: 8 ECU\n \
                       SG_ EngineRPM : 0|16@1+ (0.25,0) [0|16383.75] \"rpm\" DASH\n \
                       SG_ CoolantTemp : 16|8@1+ (0.5,-40) [-40|87.5] \"degC\" DASH\n";

    fn decode_all<'a>(db: &'a Database, frame: &'a CanFrame) -> Vec<Result<Decoded<'a>>> {
        db.decode_frame(frame)
            .expect("message is defined")
            .collect()
    }

    #[test]
    fn decodes_every_signal_of_a_known_frame() {
        let db = parse(DBC).unwrap();
        let frame =
            CanFrame::new(CanId::Standard(256), &[0x34, 0x12, 0x64, 0, 0, 0, 0, 0], 0).unwrap();

        let decoded = decode_all(&db, &frame);
        assert_eq!(decoded.len(), 2);

        let rpm = decoded[0].as_ref().unwrap();
        assert_eq!(rpm.signal.name, "EngineRPM");
        assert_eq!(rpm.raw, 0x1234);
        assert_eq!(rpm.value, 1165.0);

        let temp = decoded[1].as_ref().unwrap();
        assert_eq!(temp.signal.name, "CoolantTemp");
        assert_eq!(temp.raw, 100);
        assert_eq!(temp.value, 10.0);
    }

    #[test]
    fn an_unknown_identifier_is_none_rather_than_an_error() {
        let db = parse(DBC).unwrap();
        let frame = CanFrame::new(CanId::Standard(0x7FF), &[0; 8], 0).unwrap();
        assert!(db.decode_frame(&frame).is_none());
    }

    #[test]
    fn a_standard_id_does_not_match_an_extended_message_of_equal_value() {
        let db = parse("BO_ 2147483904 Ext: 8 E\n SG_ S : 0|8@1+ (1,0) [0|0] \"\" X\n").unwrap();
        assert!(db.message(CanId::Extended(256)).is_some());

        let frame = CanFrame::new(CanId::Standard(256), &[0; 8], 0).unwrap();
        assert!(db.decode_frame(&frame).is_none());
    }

    #[test]
    fn a_short_frame_still_yields_the_signals_that_fit() {
        let db = parse(DBC).unwrap();
        // Two bytes: EngineRPM fits, CoolantTemp does not.
        let frame = CanFrame::new(CanId::Standard(256), &[0x34, 0x12], 0).unwrap();

        let decoded = decode_all(&db, &frame);
        assert_eq!(decoded[0].as_ref().unwrap().raw, 0x1234);
        assert!(decoded[1].is_err());
    }

    #[test]
    fn a_message_with_no_signals_yields_an_empty_iterator() {
        let db = parse("BO_ 1 Empty: 8 E\n").unwrap();
        let frame = CanFrame::new(CanId::Standard(1), &[0; 8], 0).unwrap();
        assert_eq!(db.decode_frame(&frame).unwrap().count(), 0);
    }
}
