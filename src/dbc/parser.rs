//! DBC text -> [`Database`].
//!
//! Line-oriented and hand-written. A DBC is a flat sequence of records, and
//! the thing a caller most needs on failure is the line number, which I would
//! lose behind a parser-combinator stack.
//!
//! Records understood: `BO_` (message), `SG_` (signal), `VAL_` (value
//! table), and `SG_MUL_VAL_` (checked against the `SG_` tokens, not stored).
//! The last two must follow the `BO_` block they refer to, which is where
//! every DBC writer puts them. Everything else — `CM_`, `BA_`, `BA_DEF_`,
//! `VAL_TABLE_`, `BO_TX_BU_`, `BU_`, `NS_`, `BS_` — is skipped, because a
//! database that refuses to load over a comment record is useless in a
//! garage.

use std::str::FromStr;

use crate::dbc::types::{
    ByteOrder, Database, Message, Multiplexing, Signal, ValueTable, ValueType,
};
use crate::error::{Error, Result};
use crate::frame::CanId;

/// Parse DBC source text into a [`Database`].
pub fn parse(text: &str) -> Result<Database> {
    let mut parser = Parser::default();
    for (index, raw_line) in text.lines().enumerate() {
        parser.line(index + 1, raw_line.trim())?;
    }
    parser.finish()
}

#[derive(Default)]
struct Parser {
    db: Database,
    current: Option<Message>,
    /// A `VAL_` record still waiting for its `;`: the line it began on and
    /// the text gathered so far. Long tables are sometimes wrapped.
    pending: Option<(usize, String)>,
}

impl Parser {
    fn line(&mut self, line_no: usize, line: &str) -> Result<()> {
        if let Some((start, mut text)) = self.pending.take() {
            text.push(' ');
            text.push_str(line);
            return self.value_record(start, text);
        }

        // The trailing space keeps `BO_` from matching `BO_TX_BU_`.
        if let Some(rest) = line.strip_prefix("BO_ ") {
            self.flush();
            self.current = Some(parse_message(rest, line_no)?);
        } else if let Some(rest) = line.strip_prefix("SG_ ") {
            let message = self
                .current
                .as_mut()
                .ok_or_else(|| parse_error(line_no, "SG_ record appears before any BO_ message"))?;
            push_signal(message, parse_signal(rest, line_no)?, line_no)?;
        } else if let Some(rest) = line.strip_prefix("SG_MUL_VAL_ ") {
            self.flush();
            check_multiplexer_values(&self.db, rest, line_no)?;
        } else if let Some(rest) = line.strip_prefix("VAL_ ") {
            self.flush();
            self.value_record(line_no, rest.to_string())?;
        }
        Ok(())
    }

    fn finish(mut self) -> Result<Database> {
        if let Some((start, text)) = self.pending.take() {
            self.value_table(start, &text)?;
        }
        self.flush();
        Ok(self.db)
    }

    fn flush(&mut self) {
        if let Some(finished) = self.current.take() {
            self.db.insert(finished);
        }
    }

    fn value_record(&mut self, start: usize, text: String) -> Result<()> {
        match text.trim_end().strip_suffix(';') {
            Some(body) => self.value_table(start, body),
            None => {
                self.pending = Some((start, text));
                Ok(())
            }
        }
    }

    /// `<id> <signal> <key> "<label>" ... ;`
    ///
    /// A record naming a message or signal this file does not define is
    /// dropped without complaint, and so is the `VAL_ <EnvVar> ...` form for
    /// environment variables; `cantools` does the same. A later record for
    /// the same signal replaces the earlier table rather than merging.
    fn value_table(&mut self, line: usize, body: &str) -> Result<()> {
        let (id_token, after_id) =
            split_token(body).ok_or_else(|| parse_error(line, "VAL_ record has no identifier"))?;
        let Ok(raw_id) = id_token.parse::<u32>() else {
            return Ok(());
        };
        let (signal_name, entries_text) = split_token(after_id)
            .ok_or_else(|| parse_error(line, "VAL_ record has no signal name"))?;
        let entries = parse_value_entries(entries_text, line)?;

        let target = CanId::from_dbc(raw_id)
            .ok()
            .and_then(|id| self.db.message_mut(id))
            .and_then(|message| message.signal_mut(signal_name));
        if let Some(signal) = target {
            signal.value_table = ValueTable::new(entries);
        }
        Ok(())
    }
}

fn push_signal(message: &mut Message, signal: Signal, line: usize) -> Result<()> {
    if signal.multiplexing == Multiplexing::Multiplexor {
        if let Some(existing) = message.multiplexor() {
            return Err(parse_error(
                line,
                format!(
                    "message `{}` already has multiplexor `{}`; extended multiplexing is not supported",
                    message.name, existing.name
                ),
            ));
        }
    }
    message.signals.push(signal);
    Ok(())
}

/// `<id> <Name>: <dlc> <Sender>`
fn parse_message(rest: &str, line: usize) -> Result<Message> {
    let mut tokens = rest.split_whitespace();

    let id_token = expect_token(&mut tokens, line, "message record has no identifier")?;
    let raw_id = parse_number(id_token, line, "message identifier")?;
    let id = CanId::from_dbc(raw_id).map_err(|e| parse_error(line, e.to_string()))?;

    let name_token = expect_token(&mut tokens, line, "message record has no name")?;
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
        value_table: ValueTable::default(),
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
        Some(selector) if selector.ends_with('M') => Err(parse_error(
            line,
            format!("`{token}` marks a nested multiplexor; extended multiplexing is not supported"),
        )),
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

/// `<id> <signal> <multiplexor> <from>-<to>[, <from>-<to>]... ;`
///
/// CANdb++ writes one of these per multiplexed signal even for simple
/// multiplexing, so the record is accepted when it merely restates the
/// signal's `m<N>` token. Anything it would add — a different multiplexor,
/// a range, several ranges — is extended multiplexing, which I reject rather
/// than decode against the wrong selector.
fn check_multiplexer_values(db: &Database, rest: &str, line: usize) -> Result<()> {
    let mut tokens = rest.trim_end_matches(';').split_whitespace();
    let id_token = expect_token(&mut tokens, line, "SG_MUL_VAL_ record has no identifier")?;
    let raw_id: u32 = parse_number(id_token, line, "message identifier")?;
    let id = CanId::from_dbc(raw_id).map_err(|e| parse_error(line, e.to_string()))?;
    let signal_name = expect_token(&mut tokens, line, "SG_MUL_VAL_ record has no signal name")?;
    let multiplexor_name =
        expect_token(&mut tokens, line, "SG_MUL_VAL_ record has no multiplexor")?;
    let ranges: String = tokens.collect();

    let message = db.message(id).ok_or_else(|| {
        parse_error(
            line,
            format!("SG_MUL_VAL_ refers to unknown message {raw_id}"),
        )
    })?;
    let signal = message.signal(signal_name).ok_or_else(|| {
        parse_error(
            line,
            format!("SG_MUL_VAL_ refers to unknown signal `{signal_name}`"),
        )
    })?;

    let same_multiplexor = message
        .multiplexor()
        .is_some_and(|m| m.name == multiplexor_name);
    let restates_token = match signal.multiplexing {
        Multiplexing::Multiplexed(n) => ranges == format!("{n}-{n}"),
        _ => false,
    };
    if same_multiplexor && restates_token {
        return Ok(());
    }
    Err(parse_error(
        line,
        format!("signal `{signal_name}` uses extended multiplexing, which is not supported"),
    ))
}

/// `<key> "<label>"` pairs until the text runs out. The key may butt up
/// against the opening quote, as `cantools` allows.
fn parse_value_entries(mut text: &str, line: usize) -> Result<Vec<(i64, String)>> {
    let mut entries = Vec::new();
    loop {
        text = text.trim_start();
        if text.is_empty() {
            return Ok(entries);
        }

        let key_end = text
            .find(|c: char| c.is_whitespace() || c == '"')
            .unwrap_or(text.len());
        let key = parse_number(&text[..key_end], line, "value table key")?;

        let after_key = text[key_end..].trim_start();
        let unquoted = after_key.strip_prefix('"').ok_or_else(|| {
            parse_error(line, format!("value table key {key} has no quoted label"))
        })?;
        let (label, rest) = unquoted
            .split_once('"')
            .ok_or_else(|| parse_error(line, "value table has an unterminated label"))?;

        entries.push((key, label.to_string()));
        text = rest;
    }
}

/// The first whitespace-delimited token and what follows it.
fn split_token(text: &str) -> Option<(&str, &str)> {
    let text = text.trim_start();
    if text.is_empty() {
        return None;
    }
    let end = text.find(char::is_whitespace).unwrap_or(text.len());
    Some((&text[..end], &text[end..]))
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

fn expect_token<'a>(
    tokens: &mut impl Iterator<Item = &'a str>,
    line: usize,
    missing: &'static str,
) -> Result<&'a str> {
    tokens.next().ok_or_else(|| parse_error(line, missing))
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

    const MUXED: &str = "BO_ 768 Susp: 8 ECU\n \
                         SG_ Mux M : 0|8@1+ (1,0) [0|255] \"\" X\n \
                         SG_ PosFL m0 : 8|16@1- (0.1,0) [0|0] \"mm\" X\n \
                         SG_ PosFR m12 : 8|16@1- (0.1,0) [0|0] \"mm\" X\n";

    fn message_of(result: Result<Database>) -> String {
        match result {
            Err(Error::DbcParse { message, .. }) => message,
            other => panic!("expected a parse error, got {other:?}"),
        }
    }

    #[test]
    fn rejects_a_second_multiplexor_in_one_message() {
        let text = format!("{MUXED} SG_ Mux2 M : 24|8@1+ (1,0) [0|0] \"\" X\n");
        assert_eq!(line_of(parse(&text)), 5);
        assert!(message_of(parse(&text)).contains("extended multiplexing"));
    }

    #[test]
    fn rejects_a_nested_multiplexor_token() {
        let text = format!("{MUXED} SG_ Sub m1M : 24|8@1+ (1,0) [0|0] \"\" X\n");
        assert_eq!(line_of(parse(&text)), 5);
        assert!(message_of(parse(&text)).contains("extended multiplexing"));
    }

    #[test]
    fn accepts_multiplexer_value_records_that_restate_the_selector() {
        let text =
            format!("{MUXED}\nSG_MUL_VAL_ 768 PosFL Mux 0-0;\nSG_MUL_VAL_ 768 PosFR Mux 12-12 ;\n");
        let db = parse(&text).unwrap();
        assert_eq!(db.signal_count(), 3);
    }

    #[test]
    fn rejects_multiplexer_value_records_that_add_a_range() {
        let text = format!("{MUXED}\nSG_MUL_VAL_ 768 PosFL Mux 0-3;\n");
        assert_eq!(line_of(parse(&text)), 6);
        assert!(message_of(parse(&text)).contains("PosFL"));

        let text = format!("{MUXED}\nSG_MUL_VAL_ 768 PosFR Mux 12-12, 14-14;\n");
        assert_eq!(line_of(parse(&text)), 6);
    }

    #[test]
    fn rejects_multiplexer_value_records_naming_another_multiplexor() {
        let text = format!("{MUXED}\nSG_MUL_VAL_ 768 PosFL PosFR 0-0;\n");
        assert_eq!(line_of(parse(&text)), 6);
    }

    #[test]
    fn rejects_multiplexer_value_records_with_dangling_references() {
        let text = format!("{MUXED}\nSG_MUL_VAL_ 769 PosFL Mux 0-0;\n");
        assert!(message_of(parse(&text)).contains("unknown message"));
        let text = format!("{MUXED}\nSG_MUL_VAL_ 768 Nope Mux 0-0;\n");
        assert!(message_of(parse(&text)).contains("unknown signal"));
    }

    const GEAR: &str = "BO_ 1 Trans: 8 E\n \
                        SG_ Gear : 0|8@1- (1,0) [0|0] \"\" X\n \
                        SG_ Mode : 8|8@1+ (1,0) [0|0] \"\" X\n";

    fn labels(db: &Database, signal: &str) -> Vec<(i64, String)> {
        db.message(CanId::Standard(1))
            .unwrap()
            .signal(signal)
            .unwrap()
            .value_table
            .iter()
            .map(|(k, l)| (k, l.to_string()))
            .collect()
    }

    #[test]
    fn parses_value_tables_onto_their_signal() {
        let text = format!("{GEAR}\nVAL_ 1 Gear -1 \"Reverse\" 0 \"Neutral\" 1 \"First gear\" ;\n");
        let db = parse(&text).unwrap();
        assert_eq!(
            labels(&db, "Gear"),
            [
                (-1, "Reverse".into()),
                (0, "Neutral".into()),
                (1, "First gear".into())
            ]
        );
        assert!(labels(&db, "Mode").is_empty());
    }

    #[test]
    fn a_later_value_table_replaces_the_earlier_one() {
        let text = format!("{GEAR}VAL_ 1 Gear 0 \"N\" 1 \"D\";\nVAL_ 1 Gear 2 \"R\";\n");
        let db = parse(&text).unwrap();
        assert_eq!(labels(&db, "Gear"), [(2, "R".into())]);
    }

    #[test]
    fn value_tables_may_wrap_across_lines_until_the_semicolon() {
        let text = format!("{GEAR}VAL_ 1 Gear 0 \"N\"\n 1 \"D\"\n 2 \"R\"\n;\n");
        let db = parse(&text).unwrap();
        assert_eq!(labels(&db, "Gear").len(), 3);

        // Left open at the end of the file, the record is taken as written.
        let text = format!("{GEAR}VAL_ 1 Gear 0 \"N\"\n 1 \"D\"\n");
        assert_eq!(labels(&parse(&text).unwrap(), "Gear").len(), 2);
    }

    #[test]
    fn value_tables_accept_the_spacing_variants_cantools_does() {
        let text = format!("{GEAR}VAL_ 1 Gear 0\"N\"1 \"D\" 2 \"\";\n");
        let db = parse(&text).unwrap();
        assert_eq!(
            labels(&db, "Gear"),
            [(0, "N".into()), (1, "D".into()), (2, String::new())]
        );

        let empty = format!("{GEAR}VAL_ 1 Gear ;\n");
        assert!(labels(&parse(&empty).unwrap(), "Gear").is_empty());
    }

    #[test]
    fn value_tables_for_unknown_targets_and_environment_variables_are_dropped() {
        let text = format!(
            "{GEAR}VAL_ 2 Gear 0 \"N\";\nVAL_ 1 Nope 0 \"N\";\nVAL_ 2048 Gear 0 \"N\";\nVAL_ EnvVar 0 \"N\";\n"
        );
        let db = parse(&text).unwrap();
        assert!(labels(&db, "Gear").is_empty());
    }

    #[test]
    fn value_tables_reach_signals_of_extended_messages() {
        let text = "BO_ 2566843904 Diag: 8 ECU\n SG_ Code : 0|8@1+ (1,0) [0|255] \"\" X\n\
                    VAL_ 2566843904 Code 1 \"Overheat\";\n";
        let db = parse(text).unwrap();
        let code = db
            .message(CanId::Extended(0x18FE_EE00))
            .unwrap()
            .signal("Code")
            .unwrap();
        assert_eq!(code.label(1), Some("Overheat"));
    }

    #[test]
    fn reports_the_starting_line_of_a_malformed_value_table() {
        let text = format!("{GEAR}\nVAL_ 1 Gear 0 \"N\" one \"D\";\n");
        assert_eq!(line_of(parse(&text)), 5);
        assert!(message_of(parse(&text)).contains("value table key"));

        let text = format!("{GEAR}\nVAL_ 1 Gear 0 N;\n");
        assert!(message_of(parse(&text)).contains("quoted label"));

        let text = format!("{GEAR}\nVAL_ 1 Gear 0 \"N\n 1 \"D\";\n");
        assert_eq!(line_of(parse(&text)), 5);

        let text = format!("{GEAR}\nVAL_ 1;\n");
        assert!(message_of(parse(&text)).contains("signal name"));
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
                    VAL_TABLE_ OnOff 0 \"Off\" 1 \"On\";\n\
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
