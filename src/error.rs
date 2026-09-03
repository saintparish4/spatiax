//! The crate's single error type.
//!
//! I keep this a plain enum rather than reaching for `anyhow` so a caller can
//! tell "this DBC is malformed" apart from "this signal does not fit the frame
//! it was decoded against" — those have different fixes. Decoding a
//! well-formed frame against a well-formed database never constructs one of
//! these, so nothing here is on the hot path.

use crate::frame::MAX_FRAME_LEN;

/// Errors produced while parsing a DBC database or decoding a CAN frame.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// The DBC text could not be parsed.
    #[error("DBC parse error on line {line}: {message}")]
    DbcParse {
        /// 1-based line number in the DBC source.
        line: usize,
        /// What was wrong on that line.
        message: String,
    },

    /// A line of a `candump` log could not be read as a frame.
    #[error("candump log parse error on line {line}: {message}")]
    CandumpParse {
        /// 1-based line number in the log.
        line: usize,
        /// What was wrong on that line.
        message: String,
    },

    /// A CAN identifier was outside the range its format permits.
    #[error("invalid CAN identifier {raw:#x}: {reason}")]
    InvalidId {
        /// The identifier as given.
        raw: u32,
        /// Why it was rejected.
        reason: &'static str,
    },

    /// A frame payload exceeded the CAN FD maximum.
    #[error("frame payload of {len} bytes exceeds the CAN FD maximum of {MAX_FRAME_LEN}")]
    FrameTooLong {
        /// The offending payload length in bytes.
        len: usize,
    },

    /// A data length code did not describe the payload it arrived with.
    #[error("data length code {dlc} does not describe a payload of {len} byte(s)")]
    DlcMismatch {
        /// The code as given.
        dlc: u8,
        /// Bytes the payload actually carried.
        len: usize,
    },

    /// A signal's bit range extended past the end of the frame it was
    /// decoded against. Sizes are in bytes because that is the unit of the
    /// DBC's DLC field.
    #[error("signal `{signal}` needs {required} byte(s) but the frame carries {available}")]
    SignalOutOfBounds {
        /// Name of the signal that did not fit.
        signal: String,
        /// Bytes the signal's layout requires.
        required: usize,
        /// Bytes the frame actually carried.
        available: usize,
    },

    /// A raw value had bits set above a signal's declared width.
    #[error("raw value {raw:#x} does not fit in the {length} bit(s) of signal `{signal}`")]
    RawOutOfRange {
        /// Name of the signal being encoded.
        signal: String,
        /// The signal's width in bits.
        length: u8,
        /// The value that did not fit.
        raw: u64,
    },

    /// A physical value fell outside what a signal's width and scaling can
    /// represent, or was not a finite number.
    #[error("value {value} cannot be encoded by signal `{signal}`")]
    ValueOutOfRange {
        /// Name of the signal being encoded.
        signal: String,
        /// The value that could not be represented.
        value: f64,
    },

    /// A session could not be built from what the log and database offered.
    #[error("cannot export: {reason}")]
    Export {
        /// What made the session impossible.
        reason: String,
    },

    /// An underlying I/O failure, typically while reading a DBC file.
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
}

/// Crate-wide result alias.
pub type Result<T> = core::result::Result<T, Error>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dbc_parse_error_reports_one_based_line_number() {
        let e = Error::DbcParse {
            line: 42,
            message: "unterminated signal".into(),
        };
        assert!(e.to_string().contains("line 42"));
    }

    #[test]
    fn out_of_bounds_error_names_the_signal_and_both_sizes() {
        let e = Error::SignalOutOfBounds {
            signal: "EngineRPM".into(),
            required: 8,
            available: 4,
        };
        let s = e.to_string();
        assert!(s.contains("EngineRPM"));
        assert!(s.contains('8'));
        assert!(s.contains('4'));
    }

    #[test]
    fn io_errors_convert_with_the_question_mark_operator() {
        fn read_missing() -> Result<String> {
            Ok(std::fs::read_to_string("/nonexistent/spatiax/test")?)
        }
        assert!(matches!(read_missing(), Err(Error::Io(_))));
    }
}
