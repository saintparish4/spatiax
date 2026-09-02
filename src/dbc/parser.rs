//! DBC text -> [`Database`].
//!
//! Line-oriented and hand-written. A DBC is a flat sequence of records, and
//! the thing a caller most needs on failure is the line number, which I would
//! lose behind a parser-combinator stack.
//!
//! Records understood: `BO_` (message) and `SG_` (signal). Everything else —
//! `CM_`, `BA_`, `BA_DEF_`, `VAL_`, `BO_TX_BU_`, `BU_`, `NS_`, `BS_` — is
//! skipped, because a database that refuses to load over a comment record is
//! useless in a garage.

use std::str::FromStr;

use crate::dbc::types::{ByteOrder, Database, Message, Multiplexing, Signal, ValueType};
use crate::error::{Error, Result};
use crate::frame::CanId;

/// Parse DBC source text into a [`Database`].
pub fn parse(text: &str) -> Result<Database> {
    let mut db = Database::new();
    let mut current: Option<Message> = None;

    for (index, raw_line) in text.lines().enumerate() {
        let line_no = index + 1;
        let line = raw_line.trim();

        // The trailing space keeps `BO_` from matching `BO_TX_BU_`.
        if let Some(rest) = line.strip_prefix("BO_ ") {
            if let Some(finished) = current.replace(parse_message(rest, line_no)?) {
                db.insert(finished);
            }
        } else if let Some(rest) = line.strip_prefix("SG_ ") {
            let message = current
                .as_mut()
                .ok_or_else(|| parse_error(line_no, "SG_ record appears before any BO_ message"))?;
            message.signals.push(parse_signal(rest, line_no)?);
        }
    }

    if let Some(finished) = current {
        db.insert(finished);
    }
    Ok(db)
}

/// `<id> <Name>: <dlc> <Sender>`
fn parse_message(rest: &str, line: usize) -> Result<Message> {
    let mut tokens = rest.split_whitespace();

    let id_token = tokens
        .next()
        .ok_or_else(|| parse_error(line, "message record has no identifier"))?;
    let raw_id = parse_number(id_token, line, "message identifier")?;
    let id = CanId::from_dbc(raw_id).map_err(|e| parse_error(line, e.to_string()))?;

    let name_token = tokens
        .next()
        .ok_or_else(|| parse_error(line, "message record has no name"))?;
    let name = name_token.trim_end_matches(':').to_string();

    // The colon is usually attached to the name but may stand alone.
    let mut dlc_token = tokens.next();
    if dlc_token == Some(":") {
        dlc_token = tokens.next();
    }
    let dlc_token = dlc_token.ok_or_else(|| parse_error(line, "message record has no DLC"))?;
    let dlc = parse_number(dlc_token, line, "DLC")?;

    Ok(Message {
        id,
        name,
        dlc,
        sender: tokens.next().unwrap_or_default().to_string(),
        signals: Vec::new(),
    })
}

/// `<Name> [M|m<N>] : <start>|<len>@<order><sign> (<factor>,<offset>) [<min>|<max>] "<unit>" <receivers>`
fn parse_signal(rest: &str, line: usize) -> Result<Signal> {
    let (head, tail) = rest
        .split_once(':')
        .ok_or_else(|| parse_error(line, "signal record has no `:`"))?;

    let (name, multiplexing) = parse_signal_head(head, line)?;
    let (layout, tail) = parse_layout(tail, line)?;
    let ((factor, offset), tail) = parse_pair(tail, ('(', ')'), ',', ("factor", "offset"), line)?;
    let ((min, max), tail) = parse_pair(tail, ('[', ']'), '|', ("minimum", "maximum"), line)?;
    let unit = parse_quoted(tail, line)?;

    Ok(Signal {
        name,
        start_bit: layout.start_bit,
        length: layout.length,
        byte_order: layout.byte_order,
        value_type: layout.value_type,
        factor,
        offset,
        min,
        max,
        unit,
        multiplexing,
    })
}

fn parse_signal_head(head: &str, line: usize) -> Result<(String, Multiplexing)> {
    let mut tokens = head.split_whitespace();
    let name = tokens
        .next()
        .ok_or_else(|| parse_error(line, "signal record has no name"))?;
    let multiplexing = match tokens.next() {
        None => Multiplexing::None,
        Some(token) => parse_multiplexing(token, line)?,
    };
    Ok((name.to_string(), multiplexing))
}

fn parse_multiplexing(token: &str, line: usize) -> Result<Multiplexing> {
    if token == "M" {
        return Ok(Multiplexing::Multiplexor);
    }
    match token.strip_prefix('m') {
        Some(selector) => Ok(Multiplexing::Multiplexed(parse_number(
            selector,
            line,
            "multiplexor selector",
        )?)),
        None => Err(parse_error(
            line,
            format!("unexpected token `{token}` after signal name"),
        )),
    }
}

struct Layout {
    start_bit: u16,
    length: u8,
    byte_order: ByteOrder,
    value_type: ValueType,
}

/// `<start>|<len>@<order><sign>`, returning what follows it.
fn parse_layout(text: &str, line: usize) -> Result<(Layout, &str)> {
    let (start, after_bar) = text.split_once('|').ok_or_else(|| {
        parse_error(
            line,
            "signal record has no `|` between start bit and length",
        )
    })?;
    let (length, after_at) = after_bar
        .split_once('@')
        .ok_or_else(|| parse_error(line, "signal record has no `@` before the byte order"))?;

    let start_bit = parse_number(start.trim(), line, "start bit")?;
    let length = parse_length(length.trim(), line)?;

    let mut flags = after_at.chars();
    let byte_order = parse_byte_order(flags.next(), line)?;
    let value_type = parse_value_type(flags.next(), line)?;

    let layout = Layout {
        start_bit,
        length,
        byte_order,
        value_type,
    };
    Ok((layout, flags.as_str()))
}

fn parse_length(token: &str, line: usize) -> Result<u8> {
    let length: u8 = parse_number(token, line, "signal length")?;
    if !(1..=64).contains(&length) {
        return Err(parse_error(
            line,
            format!("signal length {length} is outside the supported range 1..=64"),
        ));
    }
    Ok(length)
}

fn parse_byte_order(flag: Option<char>, line: usize) -> Result<ByteOrder> {
    match flag {
        Some('1') => Ok(ByteOrder::Intel),
        Some('0') => Ok(ByteOrder::Motorola),
        other => Err(parse_error(
            line,
            format!(
                "expected byte order 0 or 1 after `@`, found {}",
                describe(other)
            ),
        )),
    }
}

fn parse_value_type(flag: Option<char>, line: usize) -> Result<ValueType> {
    match flag {
        Some('+') => Ok(ValueType::Unsigned),
        Some('-') => Ok(ValueType::Signed),
        other => Err(parse_error(
            line,
            format!(
                "expected sign + or - after byte order, found {}",
                describe(other)
            ),
        )),
    }
}

/// Two numbers as `<open>a<sep>b<close>`, returning what follows the close.
fn parse_pair<'a>(
    text: &'a str,
    (open, close): (char, char),
    separator: char,
    (first, second): (&str, &str),
    line: usize,
) -> Result<((f64, f64), &'a str)> {
    let group = format!("{first}/{second}");
    let (_, after_open) = text
        .split_once(open)
        .ok_or_else(|| parse_error(line, format!("signal record has no `{open}` for {group}")))?;
    let (inner, rest) = after_open
        .split_once(close)
        .ok_or_else(|| parse_error(line, format!("signal record has no `{close}` for {group}")))?;
    let (a, b) = inner.split_once(separator).ok_or_else(|| {
        parse_error(
            line,
            format!("signal record has no `{separator}` in {group}"),
        )
    })?;

    let a = parse_number(a.trim(), line, first)?;
    let b = parse_number(b.trim(), line, second)?;
    Ok(((a, b), rest))
}

/// Contents of the first `"..."` group. An empty unit is legal.
fn parse_quoted(text: &str, line: usize) -> Result<String> {
    let (_, after_open) = text
        .split_once('"')
        .ok_or_else(|| parse_error(line, "signal record has no quoted unit"))?;
    let (unit, _) = after_open
        .split_once('"')
        .ok_or_else(|| parse_error(line, "signal record has an unterminated unit string"))?;
    Ok(unit.to_string())
}

fn parse_number<T: FromStr>(token: &str, line: usize, what: &str) -> Result<T> {
    token
        .parse()
        .map_err(|_| parse_error(line, format!("`{token}` is not a valid {what}")))
}

fn describe(c: Option<char>) -> String {
    c.map_or_else(|| "end of line".to_string(), |c| format!("`{c}`"))
}

fn parse_error(line: usize, message: impl Into<String>) -> Error {
    Error::DbcParse {
        line,
        message: message.into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ONE_MESSAGE: &str = r#"
BO_ 256 EngineData: 8 ECU
 SG_ EngineRPM : 0|16@1+ (0.25,0) [0|16383.75] "rpm" DASH,LOGGER
 SG_ CoolantTemp : 16|8@1+ (0.5,-40) [-40|87.5] "degC" DASH
"#;

    fn line_of(result: Result<Database>) -> usize {
        match result {
            Err(Error::DbcParse { line, .. }) => line,
            other => panic!("expected a parse error with a line number, got {other:?}"),
        }
    }

    #[test]
    fn parses_signals_rather_than_silently_producing_none() {
        let db = parse(ONE_MESSAGE).unwrap();
        assert_eq!(db.len(), 1);
        assert_eq!(db.signal_count(), 2);
    }

    #[test]
    fn parses_every_field_of_a_signal_record() {
        let db = parse(ONE_MESSAGE).unwrap();
        let rpm = &db.message(CanId::Standard(256)).unwrap().signals[0];

        assert_eq!(rpm.name, "EngineRPM");
        assert_eq!(rpm.start_bit, 0);
        assert_eq!(rpm.length, 16);
        assert_eq!(rpm.byte_order, ByteOrder::Intel);
        assert_eq!(rpm.value_type, ValueType::Unsigned);
        assert_eq!(rpm.factor, 0.25);
        assert_eq!(rpm.offset, 0.0);
        assert_eq!(rpm.min, 0.0);
        assert_eq!(rpm.max, 16383.75);
        assert_eq!(rpm.unit, "rpm");
        assert_eq!(rpm.multiplexing, Multiplexing::None);
    }

    #[test]
    fn parses_a_negative_offset_and_minimum() {
        let db = parse(ONE_MESSAGE).unwrap();
        let temp = &db.message(CanId::Standard(256)).unwrap().signals[1];
        assert_eq!(temp.offset, -40.0);
        assert_eq!(temp.min, -40.0);
    }

    #[test]
    fn parses_message_header_fields() {
        let db = parse(ONE_MESSAGE).unwrap();
        let message = db.message(CanId::Standard(256)).unwrap();
        assert_eq!(message.name, "EngineData");
        assert_eq!(message.dlc, 8);
        assert_eq!(message.sender, "ECU");
    }

    #[test]
    fn accepts_a_detached_colon_in_the_message_header() {
        let db = parse("BO_ 256 EngineData : 8 ECU\n").unwrap();
        let message = db.message(CanId::Standard(256)).unwrap();
        assert_eq!(message.name, "EngineData");
        assert_eq!(message.dlc, 8);
    }

    #[test]
    fn interprets_the_dbc_extended_identifier_flag() {
        // 2566843904 == 0x98FEEE00; bit 31 marks it extended.
        let db =
            parse("BO_ 2566843904 Diag: 8 ECU\n SG_ Code : 0|8@1+ (1,0) [0|255] \"\" X\n").unwrap();
        assert!(db.message(CanId::Extended(0x18FE_EE00)).is_some());
    }

    #[test]
    fn parses_motorola_and_signed_flags() {
        let db = parse("BO_ 1 M: 8 E\n SG_ S : 7|16@0- (1,0) [0|0] \"\" X\n").unwrap();
        let s = &db.message(CanId::Standard(1)).unwrap().signals[0];
        assert_eq!(s.byte_order, ByteOrder::Motorola);
        assert_eq!(s.value_type, ValueType::Signed);
    }

    #[test]
    fn parses_multiplexor_and_multiplexed_markers() {
        let text = "BO_ 768 Susp: 8 ECU\n \
                    SG_ Mux M : 0|8@1+ (1,0) [0|255] \"\" X\n \
                    SG_ PosFL m0 : 8|16@1- (0.1,0) [0|0] \"mm\" X\n \
                    SG_ PosFR m12 : 8|16@1- (0.1,0) [0|0] \"mm\" X\n";
        let db = parse(text).unwrap();
        let signals = &db.message(CanId::Standard(768)).unwrap().signals;
        assert_eq!(signals[0].multiplexing, Multiplexing::Multiplexor);
        assert_eq!(signals[1].multiplexing, Multiplexing::Multiplexed(0));
        assert_eq!(signals[2].multiplexing, Multiplexing::Multiplexed(12));
    }

    #[test]
    fn skips_records_it_does_not_understand() {
        let text = "VERSION \"x\"\n\
                    NS_ :\n\
                    BS_:\n\
                    BU_: ECU DASH\n\
                    BO_ 256 EngineData: 8 ECU\n \
                    SG_ EngineRPM : 0|16@1+ (0.25,0) [0|16383.75] \"rpm\" DASH\n\
                    CM_ SG_ 256 EngineRPM \"Engine speed\";\n\
                    BA_ \"GenMsgCycleTime\" BO_ 256 10;\n\
                    BO_TX_BU_ 256 : ECU;\n";
        let db = parse(text).unwrap();
        assert_eq!(db.len(), 1);
        assert_eq!(db.signal_count(), 1);
    }

    #[test]
    fn parses_several_messages_and_keeps_signals_with_their_own_message() {
        let text = "BO_ 1 A: 8 E\n SG_ S1 : 0|8@1+ (1,0) [0|0] \"\" X\n\
                    BO_ 2 B: 8 E\n SG_ S2 : 0|8@1+ (1,0) [0|0] \"\" X\n \
                    SG_ S3 : 8|8@1+ (1,0) [0|0] \"\" X\n";
        let db = parse(text).unwrap();
        assert_eq!(db.message(CanId::Standard(1)).unwrap().signals.len(), 1);
        assert_eq!(db.message(CanId::Standard(2)).unwrap().signals.len(), 2);
    }

    #[test]
    fn reports_the_line_number_of_a_malformed_signal() {
        let text = "BO_ 256 M: 8 E\n SG_ Broken : notanumber|16@1+ (1,0) [0|0] \"\" X\n";
        assert_eq!(line_of(parse(text)), 2);
    }

    #[test]
    fn reports_the_line_number_of_an_invalid_identifier() {
        // 0x800 without the extended flag does not fit 11 bits.
        let text = "BO_ 1 Ok: 8 E\n\nBO_ 2048 TooBig: 8 E\n";
        assert_eq!(line_of(parse(text)), 3);
    }

    #[test]
    fn rejects_a_signal_wider_than_64_bits() {
        let text = "BO_ 256 M: 8 E\n SG_ Wide : 0|65@1+ (1,0) [0|0] \"\" X\n";
        assert_eq!(line_of(parse(text)), 2);
    }

    #[test]
    fn rejects_a_zero_width_signal() {
        let text = "BO_ 256 M: 8 E\n SG_ Empty : 0|0@1+ (1,0) [0|0] \"\" X\n";
        assert_eq!(line_of(parse(text)), 2);
    }

    #[test]
    fn rejects_a_signal_that_precedes_any_message() {
        let text = " SG_ Orphan : 0|8@1+ (1,0) [0|0] \"\" X\n";
        assert_eq!(line_of(parse(text)), 1);
    }

    #[test]
    fn rejects_an_unknown_byte_order_or_sign() {
        assert_eq!(
            line_of(parse("BO_ 1 M: 8 E\n SG_ S : 0|8@2+ (1,0) [0|0] \"\" X\n")),
            2
        );
        assert_eq!(
            line_of(parse("BO_ 1 M: 8 E\n SG_ S : 0|8@1* (1,0) [0|0] \"\" X\n")),
            2
        );
    }

    #[test]
    fn rejects_a_truncated_signal_record() {
        assert_eq!(line_of(parse("BO_ 1 M: 8 E\n SG_ S : 0|8@1\n")), 2);
        assert_eq!(line_of(parse("BO_ 1 M: 8 E\n SG_ S : 0|8@1+ (1,0)\n")), 2);
        assert_eq!(
            line_of(parse(
                "BO_ 1 M: 8 E\n SG_ S : 0|8@1+ (1,0) [0|0] \"unterminated\n"
            )),
            2
        );
    }

    #[test]
    fn accepts_an_empty_unit_string() {
        let db = parse("BO_ 1 M: 8 E\n SG_ S : 0|8@1+ (1,0) [0|255] \"\" X\n").unwrap();
        assert_eq!(db.message(CanId::Standard(1)).unwrap().signals[0].unit, "");
    }

    #[test]
    fn an_empty_document_is_an_empty_database() {
        assert!(parse("").unwrap().is_empty());
        assert!(parse("\n\n  \n").unwrap().is_empty());
    }
}
