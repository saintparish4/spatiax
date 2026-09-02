//! DBC database: types, parsing, and frame decoding.
//!
//! `decode_frame` returns `None` rather than an error for an unknown
//! identifier. On a real bus most traffic is not described by any one DBC,
//! so an unrecognised frame is an ordinary event, not a failure. A signal
//! that does not fit the frame *is* an error, reported per signal so one
//! short frame cannot discard the signals that did decode.
//!
//! Multiplexed signals are selected by the multiplexor's raw value. A
//! selector no `m<N>` signal claims is not an error either — a DBC often
//! describes only the pages a team cares about — so such a frame yields just
//! its plain signals and the multiplexor.

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

impl Message {
    /// Decode the signals that apply to `data`.
    ///
    /// Yields one item per applicable signal in DBC order; a signal that does
    /// not fit the payload yields `Err` without affecting its neighbours.
    /// Multiplexed signals appear only when the multiplexor's raw value
    /// selects them. If the multiplexor itself does not fit, its `Err` is
    /// reported once and the signals depending on it are skipped, since
    /// nothing can say whether they were present. Nothing here allocates.
    pub fn decode<'a>(&'a self, data: &'a [u8]) -> impl Iterator<Item = Result<Decoded<'a>>> + 'a {
        let selector = self.multiplexor().map(|m| extract_raw(data, m).ok());
        self.signals
            .iter()
            .filter(move |signal| applies(signal, selector))
            .map(move |signal| {
                let raw = extract_raw(data, signal)?;
                Ok(Decoded {
                    signal,
                    raw,
                    value: signal.scale(raw),
                })
            })
    }
}

/// `selector` is `None` when the message has no multiplexor at all, in which
/// case an `m<N>` signal is treated as plain — the same reading `cantools`
/// gives such a file.
fn applies(signal: &Signal, selector: Option<Option<u64>>) -> bool {
    match (signal.multiplexing, selector) {
        (Multiplexing::Multiplexed(n), Some(found)) => found == Some(u64::from(n)),
        _ => true,
    }
}

impl Database {
    /// Decode the message matching this frame's identifier.
    ///
    /// Returns `None` if no message is defined for the identifier; otherwise
    /// behaves as [`Message::decode`] on the frame's payload.
    pub fn decode_frame<'a>(
        &'a self,
        frame: &'a CanFrame,
    ) -> Option<impl Iterator<Item = Result<Decoded<'a>>> + 'a> {
        Some(self.message(frame.id())?.decode(frame.data()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::Error;
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

    // The multiplexor is deliberately not the first signal, and the two
    // pages share the same bits.
    const MUXED: &str = "BO_ 768 Susp: 8 ECU\n \
                         SG_ Plain : 56|8@1+ (1,0) [0|255] \"\" X\n \
                         SG_ Page M : 0|8@1+ (1,0) [0|255] \"\" X\n \
                         SG_ PosFL m0 : 8|16@1- (0.1,0) [0|0] \"mm\" X\n \
                         SG_ PosFR m1 : 8|16@1- (0.1,0) [0|0] \"mm\" X\n";

    fn names(decoded: &[Result<Decoded<'_>>]) -> Vec<String> {
        decoded
            .iter()
            .map(|d| d.as_ref().unwrap().signal.name.clone())
            .collect()
    }

    #[test]
    fn the_multiplexor_value_selects_which_page_decodes() {
        let db = parse(MUXED).unwrap();
        let page0 =
            CanFrame::new(CanId::Standard(768), &[0, 0x34, 0x12, 0, 0, 0, 0, 9], 0).unwrap();
        let page1 =
            CanFrame::new(CanId::Standard(768), &[1, 0x34, 0x12, 0, 0, 0, 0, 9], 0).unwrap();

        let decoded = decode_all(&db, &page0);
        assert_eq!(names(&decoded), ["Plain", "Page", "PosFL"]);
        assert_eq!(decoded[2].as_ref().unwrap().raw, 0x1234);

        assert_eq!(names(&decode_all(&db, &page1)), ["Plain", "Page", "PosFR"]);
    }

    #[test]
    fn an_unclaimed_selector_yields_only_the_plain_signals_and_the_multiplexor() {
        let db = parse(MUXED).unwrap();
        let frame = CanFrame::new(CanId::Standard(768), &[7, 0, 0, 0, 0, 0, 0, 9], 0).unwrap();
        assert_eq!(names(&decode_all(&db, &frame)), ["Plain", "Page"]);
    }

    #[test]
    fn multiplexed_signals_are_skipped_when_the_multiplexor_does_not_fit() {
        let text = "BO_ 1 A: 8 E\n \
                    SG_ Page M : 56|8@1+ (1,0) [0|0] \"\" X\n \
                    SG_ Low m0 : 0|8@1+ (1,0) [0|0] \"\" X\n";
        let db = parse(text).unwrap();
        let frame = CanFrame::new(CanId::Standard(1), &[0], 0).unwrap();

        let decoded = decode_all(&db, &frame);
        assert_eq!(decoded.len(), 1);
        assert!(matches!(
            decoded[0],
            Err(Error::SignalOutOfBounds { ref signal, .. }) if signal == "Page"
        ));
    }

    #[test]
    fn a_multiplexed_signal_without_any_multiplexor_decodes_as_plain() {
        let db = parse("BO_ 1 A: 8 E\n SG_ S m3 : 0|8@1+ (1,0) [0|0] \"\" X\n").unwrap();
        let frame = CanFrame::new(CanId::Standard(1), &[0x2A; 8], 0).unwrap();
        let decoded = decode_all(&db, &frame);
        assert_eq!(decoded.len(), 1);
        assert_eq!(decoded[0].as_ref().unwrap().raw, 0x2A);
    }

    #[test]
    fn message_decode_is_usable_without_a_frame() {
        let db = parse(MUXED).unwrap();
        let message = db.message(CanId::Standard(768)).unwrap();
        let count = message.decode(&[1, 0, 0, 0, 0, 0, 0, 0]).count();
        assert_eq!(count, 3);
    }
}
