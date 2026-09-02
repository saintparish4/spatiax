//! The two output shapes of `spatiax decode`.
//!
//! Text groups signals under their frame for reading at a terminal; CSV
//! puts one decoded signal per row for anything downstream. Identifiers are
//! printed the way `candump` writes them — three hex digits for standard,
//! eight for extended — so a value on screen can be grepped for in the log.
//! Both are written by hand: neither is complicated enough to earn a
//! dependency.

use std::io::{self, Write};

use clap::ValueEnum;
use spatiax::{CanFrame, CanId, Decoded, Message};

#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum)]
pub enum Format {
    /// One block per frame, one indented line per signal
    Text,
    /// One row per decoded signal, with a header row
    Csv,
}

pub struct Writer<W> {
    out: W,
    format: Format,
}

impl<W: Write> Writer<W> {
    pub fn new(out: W, format: Format) -> Self {
        Self { out, format }
    }

    pub fn begin(&mut self) -> io::Result<()> {
        match self.format {
            Format::Text => Ok(()),
            Format::Csv => writeln!(self.out, "timestamp,id,message,signal,raw,value,unit,label"),
        }
    }

    pub fn frame(&mut self, frame: &CanFrame, message: &Message) -> io::Result<()> {
        match self.format {
            Format::Text => writeln!(
                self.out,
                "{} {} {}",
                timestamp(frame),
                id(frame.id()),
                message.name
            ),
            Format::Csv => Ok(()),
        }
    }

    pub fn signal(
        &mut self,
        frame: &CanFrame,
        message: &Message,
        decoded: &Decoded<'_>,
    ) -> io::Result<()> {
        match self.format {
            Format::Text => self.text_signal(decoded),
            Format::Csv => self.csv_signal(frame, message, decoded),
        }
    }

    pub fn flush(&mut self) -> io::Result<()> {
        self.out.flush()
    }

    fn text_signal(&mut self, d: &Decoded<'_>) -> io::Result<()> {
        let signal = d.signal;
        let unit = if signal.unit.is_empty() {
            String::new()
        } else {
            format!(" {}", signal.unit)
        };
        match d.label() {
            Some(label) => writeln!(self.out, "  {}: {label} ({}{unit})", signal.name, d.value),
            None => writeln!(self.out, "  {}: {}{unit}", signal.name, d.value),
        }
    }

    fn csv_signal(
        &mut self,
        frame: &CanFrame,
        message: &Message,
        d: &Decoded<'_>,
    ) -> io::Result<()> {
        writeln!(
            self.out,
            "{},{},{},{},{},{},{},{}",
            timestamp(frame),
            id(frame.id()),
            csv_field(&message.name),
            csv_field(&d.signal.name),
            d.raw,
            d.value,
            csv_field(&d.signal.unit),
            csv_field(d.label().unwrap_or_default()),
        )
    }
}

fn timestamp(frame: &CanFrame) -> String {
    format!(
        "{}.{:06}",
        frame.timestamp_us / 1_000_000,
        frame.timestamp_us % 1_000_000
    )
}

fn id(id: CanId) -> String {
    match id {
        CanId::Standard(raw) => format!("{raw:03X}"),
        CanId::Extended(raw) => format!("{raw:08X}"),
    }
}

/// Quote a field only when RFC 4180 says it needs it.
fn csv_field(text: &str) -> String {
    if text.contains([',', '"', '\n', '\r']) {
        format!("\"{}\"", text.replace('"', "\"\""))
    } else {
        text.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identifiers_print_the_way_candump_writes_them() {
        assert_eq!(id(CanId::Standard(0x10)), "010");
        assert_eq!(id(CanId::Extended(0x10)), "00000010");
        assert_eq!(id(CanId::Extended(0x18FE_EE00)), "18FEEE00");
    }

    #[test]
    fn timestamps_keep_six_digits_of_microseconds() {
        let frame = CanFrame::new(CanId::Standard(1), &[], 1_700_000_000_000_250).unwrap();
        assert_eq!(timestamp(&frame), "1700000000.000250");
        let zero = CanFrame::new(CanId::Standard(1), &[], 0).unwrap();
        assert_eq!(timestamp(&zero), "0.000000");
    }

    #[test]
    fn csv_fields_are_quoted_only_when_they_need_to_be() {
        assert_eq!(csv_field("rpm"), "rpm");
        assert_eq!(csv_field(""), "");
        assert_eq!(csv_field("Not available"), "Not available");
        assert_eq!(csv_field("a,b"), "\"a,b\"");
        assert_eq!(csv_field("say \"hi\""), "\"say \"\"hi\"\"\"");
    }
}
