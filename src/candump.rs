//! Reader for `candump` log files — the format `candump -l` writes and
//! `canplayer` reads back, so a session captured on the car can be decoded
//! again on the pit wall:
//!
//! ```text
//! (1690000000.123456) can0 123#DEADBEEF
//! (1690000000.124011) can0 18FEEE00#0102030405060708
//! (1690000000.124388) can0 123##1DEADBEEFCAFEBABE0011223344556677
//! ```
//!
//! Three-digit identifiers are standard and eight-digit ones extended, as
//! can-utils writes them. `##` introduces a CAN FD frame whose first hex
//! digit is its flags, and a trailing `_<dlc>` is the data length code of a
//! classic frame that declares more than the eight bytes it carries. Remote
//! frames (`123#R`), error frames (bit 29 of the identifier set), and blank
//! lines carry no signals and are skipped. A line that does not parse is
//! reported with its line number and reading carries on with the next, so one
//! corrupt record cannot hide the rest of a session.

use std::io::BufRead;

use crate::error::{Error, Result};
use crate::frame::{CanFrame, CanId, MAX_FRAME_LEN, dlc_to_len, len_to_dlc};

const ERROR_FRAME_FLAG: u32 = 0x2000_0000;

/// Iterator over the data frames in a `candump` log.
pub struct LogReader<R> {
    lines: std::io::Lines<R>,
    line_no: usize,
}

impl<R: BufRead> LogReader<R> {
    /// Read frames from any buffered source: a file, stdin, or a byte slice.
    pub fn new(reader: R) -> Self {
        Self {
            lines: reader.lines(),
            line_no: 0,
        }
    }
}

impl<R: BufRead> Iterator for LogReader<R> {
    type Item = Result<CanFrame>;

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            let line = match self.lines.next()? {
                Ok(line) => line,
                Err(e) => return Some(Err(e.into())),
            };
            self.line_no += 1;
            match parse_line(&line, self.line_no) {
                Ok(Some(frame)) => return Some(Ok(frame)),
                Ok(None) => {}
                Err(e) => return Some(Err(e)),
            }
        }
    }
}

/// Parse one log line. `Ok(None)` is a line with no data frame on it.
///
/// The frame is the last whitespace-separated token; a leading `(...)` is
/// the timestamp. The interface name between them is not kept. A bare
/// `123#DEADBEEF` with neither is accepted too, with a zero timestamp,
/// since that is what people type by hand.
pub fn parse_line(line: &str, line_no: usize) -> Result<Option<CanFrame>> {
    let tokens: Vec<&str> = line.split_whitespace().collect();
    let Some(&last) = tokens.last() else {
        return Ok(None);
    };
    let timestamp_us = match tokens[0].strip_prefix('(') {
        Some(stamp) => parse_timestamp(stamp.trim_end_matches(')'), line_no)?,
        None => 0,
    };
    parse_frame(last, timestamp_us, line_no)
}

/// `<seconds>.<fraction>` to microseconds. The fraction is six digits from
/// can-utils, but I take what is there.
fn parse_timestamp(text: &str, line: usize) -> Result<u64> {
    let (seconds, fraction) = text.split_once('.').unwrap_or((text, ""));
    let not_a_timestamp = || log_error(line, format!("`({text})` is not a timestamp"));

    let seconds: u64 = seconds.parse().map_err(|_| not_a_timestamp())?;
    if !fraction.bytes().all(|b| b.is_ascii_digit()) {
        return Err(not_a_timestamp());
    }
    let micros: u64 = format!("{:0<6}", &fraction[..fraction.len().min(6)])
        .parse()
        .map_err(|_| not_a_timestamp())?;

    seconds
        .checked_mul(1_000_000)
        .and_then(|us| us.checked_add(micros))
        .ok_or_else(|| log_error(line, format!("timestamp {text} overflows")))
}

/// `<id>#<data>`, `<id>#<data>_<dlc>`, `<id>#R[<dlc>]`, or
/// `<id>##<flags><data>`.
fn parse_frame(token: &str, timestamp_us: u64, line: usize) -> Result<Option<CanFrame>> {
    let (id_hex, rest) = token
        .split_once('#')
        .ok_or_else(|| log_error(line, format!("expected `<id>#<data>`, found `{token}`")))?;
    let Some(id) = parse_id(id_hex, line)? else {
        return Ok(None);
    };

    match rest.strip_prefix('#') {
        Some(fd) => fd_frame(id, fd, timestamp_us, line),
        None if rest.starts_with('R') => Ok(None),
        None => classic_frame(id, rest, timestamp_us, line),
    }
}

/// `<flags><data>`. CAN FD carries only the lengths its data length codes
/// can express, so any other payload size is a corrupt record — the same
/// reading can-utils gives one.
fn fd_frame(id: CanId, text: &str, timestamp_us: u64, line: usize) -> Result<Option<CanFrame>> {
    let data = parse_hex(strip_fd_flags(text, line)?, line)?;
    if len_to_dlc(data.len()).is_none() {
        return Err(log_error(
            line,
            format!("no CAN FD frame carries {} byte(s)", data.len()),
        ));
    }
    Ok(Some(CanFrame::new(id, &data, timestamp_us)?))
}

/// `<data>`, or `<data>_<dlc>` for a classic frame whose data length code
/// says more than the eight bytes it carries.
fn classic_frame(
    id: CanId,
    text: &str,
    timestamp_us: u64,
    line: usize,
) -> Result<Option<CanFrame>> {
    let Some((data_hex, dlc_hex)) = text.split_once('_') else {
        let data = parse_hex(text, line)?;
        return Ok(Some(CanFrame::new(id, &data, timestamp_us)?));
    };
    let data = parse_hex(data_hex, line)?;
    let dlc = parse_dlc(dlc_hex, line)?;
    CanFrame::with_dlc(id, &data, timestamp_us, dlc)
        .map(Some)
        .map_err(|e| log_error(line, e.to_string()))
}

/// The single hex digit after `_`.
fn parse_dlc(hex: &str, line: usize) -> Result<u8> {
    u8::from_str_radix(hex, 16)
        .ok()
        .filter(|dlc| dlc_to_len(*dlc).is_some())
        .ok_or_else(|| log_error(line, format!("`_{hex}` is not a data length code")))
}

/// Three hex digits are a standard identifier and more are extended, the
/// way can-utils reads them. `None` is an error frame.
fn parse_id(hex: &str, line: usize) -> Result<Option<CanId>> {
    let raw = u32::from_str_radix(hex, 16)
        .map_err(|_| log_error(line, format!("`{hex}` is not a hexadecimal identifier")))?;
    if hex.len() <= 3 {
        return CanId::standard(raw as u16)
            .map(Some)
            .map_err(|e| log_error(line, e.to_string()));
    }
    if raw & ERROR_FRAME_FLAG != 0 {
        return Ok(None);
    }
    CanId::extended(raw)
        .map(Some)
        .map_err(|e| log_error(line, e.to_string()))
}

fn strip_fd_flags(text: &str, line: usize) -> Result<&str> {
    let mut chars = text.chars();
    match chars.next() {
        Some(flags) if flags.is_ascii_hexdigit() => Ok(chars.as_str()),
        _ => Err(log_error(
            line,
            "CAN FD frame has no flags digit after `##`",
        )),
    }
}

fn parse_hex(hex: &str, line: usize) -> Result<Vec<u8>> {
    if !hex.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(log_error(line, format!("data `{hex}` is not hexadecimal")));
    }
    if hex.len() % 2 != 0 {
        return Err(log_error(
            line,
            format!("data `{hex}` has an odd number of hex digits"),
        ));
    }
    if hex.len() > 2 * MAX_FRAME_LEN {
        return Err(log_error(
            line,
            format!(
                "data of {} bytes exceeds the CAN FD maximum of {MAX_FRAME_LEN}",
                hex.len() / 2
            ),
        ));
    }
    Ok((0..hex.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&hex[i..i + 2], 16).expect("checked hex digits"))
        .collect())
}

fn log_error(line: usize, message: impl Into<String>) -> Error {
    Error::CandumpParse {
        line,
        message: message.into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frames(log: &str) -> Vec<Result<CanFrame>> {
        LogReader::new(log.as_bytes()).collect()
    }

    fn one(log: &str) -> CanFrame {
        let mut all = frames(log);
        assert_eq!(all.len(), 1, "expected one frame from {log:?}, got {all:?}");
        all.remove(0).unwrap()
    }

    fn line_of(result: &Result<CanFrame>) -> usize {
        match result {
            Err(Error::CandumpParse { line, .. }) => *line,
            other => panic!("expected a log parse error, got {other:?}"),
        }
    }

    #[test]
    fn reads_a_classic_frame_with_its_timestamp() {
        let frame = one("(1690000000.123456) can0 123#DEADBEEF\n");
        assert_eq!(frame.id(), CanId::Standard(0x123));
        assert_eq!(frame.data(), [0xDE, 0xAD, 0xBE, 0xEF]);
        assert_eq!(frame.timestamp_us, 1_690_000_000_123_456);
    }

    #[test]
    fn eight_digit_identifiers_are_extended_even_when_small() {
        assert_eq!(
            one("(0.0) can0 18FEEE00#01\n").id(),
            CanId::Extended(0x18FE_EE00)
        );
        assert_eq!(one("(0.0) can0 00000123#01\n").id(), CanId::Extended(0x123));
    }

    #[test]
    fn reads_a_can_fd_frame_and_drops_its_flags_digit() {
        let hex = "0123456789ABCDEF".repeat(8);
        let frame = one(&format!("(0.0) can0 123##1{hex}\n"));
        assert_eq!(frame.len(), 64);
        assert_eq!(frame.data()[0], 0x01);
        assert_eq!(frame.data()[63], 0xEF);

        assert!(one("(0.0) can0 123##0\n").is_empty());
        assert_eq!(line_of(&frames("(0.0) can0 123##\n")[0]), 1);
    }

    #[test]
    fn skips_remote_frames_error_frames_and_blank_lines() {
        let log = "(0.0) can0 123#R\n\
                   (0.0) can0 123#R3\n\
                   \n\
                   (0.0) can0 20000004#0000000000000008\n\
                   (0.0) can0 123#01\n";
        let all = frames(log);
        assert_eq!(all.len(), 1);
        assert_eq!(all[0].as_ref().unwrap().data(), [1]);
    }

    #[test]
    fn keeps_the_data_length_code_of_a_len8_dlc_frame() {
        let frame = one("(0.0) can0 123#0011223344556677_9\n");
        assert_eq!(frame.len(), 8);
        assert_eq!(frame.dlc(), Some(9));
        assert_eq!(one("(0.0) can0 123#0011223344556677_F\n").dlc(), Some(15));
        // No suffix, so the payload length gives the code.
        assert_eq!(one("(0.0) can0 123#001122\n").dlc(), Some(3));
    }

    #[test]
    fn rejects_a_data_length_code_that_contradicts_its_payload() {
        let log = "(0.0) can0 123#001122_9\n\
                   (0.0) can0 123#0011223344556677_G\n\
                   (0.0) can0 123#0011223344556677_10\n";
        let all = frames(log);
        assert_eq!(all.len(), 3);
        for (index, result) in all.iter().enumerate() {
            assert_eq!(line_of(result), index + 1);
        }
    }

    #[test]
    fn rejects_a_can_fd_payload_of_a_length_no_code_can_express() {
        let all = frames("(0.0) can0 123##1001122334455667788\n");
        assert!(
            matches!(&all[0], Err(Error::CandumpParse { message, .. }) if message.contains("9 byte")),
            "{all:?}"
        );
        // The lengths CAN FD does carry are read as before.
        assert_eq!(one("(0.0) can0 123##100112233445566778899AABB\n").len(), 12);
    }

    #[test]
    fn accepts_a_bare_frame_and_a_short_fraction() {
        let frame = one("123#DEADBEEF\n");
        assert_eq!(frame.timestamp_us, 0);
        assert_eq!(frame.data(), [0xDE, 0xAD, 0xBE, 0xEF]);

        assert_eq!(one("(12.5) 123#00\n").timestamp_us, 12_500_000);
        assert_eq!(one("(12) 123#00\n").timestamp_us, 12_000_000);
        assert_eq!(one("(0.1234567) 123#00\n").timestamp_us, 123_456);
    }

    #[test]
    fn reports_malformed_lines_by_number_and_keeps_going() {
        let log = "(0.0) can0 123#01\n\
                   (0.0) can0 123#012\n\
                   (0.0) can0 123#GG\n\
                   (0.0) can0 123 01\n\
                   (0.0) can0 XYZ#01\n\
                   (0.0) can0 800#01\n\
                   (0.0) can0 C0000000#01\n\
                   (abc) can0 123#01\n\
                   (0.0) can0 123#02\n";
        let all = frames(log);
        assert_eq!(all.len(), 9);
        assert_eq!(all[0].as_ref().unwrap().data(), [1]);
        for (index, result) in all.iter().enumerate().take(8).skip(1) {
            assert_eq!(line_of(result), index + 1);
        }
        assert_eq!(all[8].as_ref().unwrap().data(), [2]);
    }

    #[test]
    fn rejects_a_payload_over_the_can_fd_maximum_with_a_line_number() {
        let hex = "00".repeat(65);
        let all = frames(&format!("\n(0.0) can0 123##1{hex}\n"));
        assert_eq!(line_of(&all[0]), 2);
    }

    #[test]
    fn a_human_readable_candump_line_is_an_error_rather_than_a_guess() {
        let all = frames("  can0  123   [4]  DE AD BE EF\n");
        assert!(matches!(
            &all[0],
            Err(Error::CandumpParse { message, .. }) if message.contains("<id>#<data>")
        ));
    }

    #[test]
    fn io_failures_come_through_as_errors() {
        struct Failing;
        impl std::io::Read for Failing {
            fn read(&mut self, _: &mut [u8]) -> std::io::Result<usize> {
                Err(std::io::Error::other("disk on fire"))
            }
        }
        let mut reader = LogReader::new(std::io::BufReader::new(Failing));
        assert!(matches!(reader.next(), Some(Err(Error::Io(_)))));
    }
}
