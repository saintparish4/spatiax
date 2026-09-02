//! `spatiax decode`: replay a `candump` log through a DBC.
//!
//! Frames the DBC does not describe are counted rather than printed — on a
//! real bus most traffic belongs to some other DBC. A malformed log line is
//! reported on stderr as it is met and reading carries on; the exit status
//! says at the end whether that happened.

use std::fs::File;
use std::io::{self, BufRead, BufReader, BufWriter, Write};
use std::path::Path;
use std::process::ExitCode;

use spatiax::candump::LogReader;
use spatiax::{CanFrame, Database};

use crate::output::{Format, Writer};
use crate::{FOUND_SOMETHING, Outcome, load_dbc};

pub fn run(dbc: &Path, log: &Path, format: Format) -> Outcome {
    let db = load_dbc(dbc)?;
    let reader = open_log(log)?;
    let mut writer = Writer::new(BufWriter::new(io::stdout().lock()), format);
    let mut tally = Tally::default();

    let written = replay(&db, reader, &mut writer, &mut tally);
    match written {
        Ok(()) => {}
        // Downstream closed the pipe (`| head`); nothing is wrong on this side.
        Err(e) if e.kind() == io::ErrorKind::BrokenPipe => return Ok(ExitCode::SUCCESS),
        Err(e) => return Err(format!("cannot write output: {e}")),
    }

    eprintln!("spatiax: {tally}");
    Ok(if tally.bad_lines == 0 {
        ExitCode::SUCCESS
    } else {
        ExitCode::from(FOUND_SOMETHING)
    })
}

fn open_log(path: &Path) -> Result<Box<dyn BufRead>, String> {
    if path.as_os_str() == "-" {
        return Ok(Box::new(io::stdin().lock()));
    }
    let file = File::open(path).map_err(|e| format!("cannot read {}: {e}", path.display()))?;
    Ok(Box::new(BufReader::new(file)))
}

fn replay<W: Write>(
    db: &Database,
    reader: Box<dyn BufRead>,
    writer: &mut Writer<W>,
    tally: &mut Tally,
) -> io::Result<()> {
    writer.begin()?;
    for result in LogReader::new(reader) {
        match result {
            Ok(frame) => decode_frame(db, &frame, writer, tally)?,
            Err(e) => {
                tally.bad_lines += 1;
                eprintln!("spatiax: {e}");
            }
        }
    }
    writer.finish()
}

fn decode_frame<W: Write>(
    db: &Database,
    frame: &CanFrame,
    writer: &mut Writer<W>,
    tally: &mut Tally,
) -> io::Result<()> {
    let Some(message) = db.message(frame.id()) else {
        tally.unknown += 1;
        return Ok(());
    };
    tally.frames += 1;
    writer.frame(frame, message)?;
    for result in message.decode(frame.data()) {
        match result {
            Ok(decoded) => writer.signal(frame, message, &decoded)?,
            Err(_) => tally.unfit += 1,
        }
    }
    Ok(())
}

#[derive(Default)]
struct Tally {
    frames: usize,
    unknown: usize,
    unfit: usize,
    bad_lines: usize,
}

impl std::fmt::Display for Tally {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "decoded {} frame(s)", self.frames)?;
        if self.unknown > 0 {
            write!(f, ", {} not in the DBC", self.unknown)?;
        }
        if self.unfit > 0 {
            write!(f, ", {} signal(s) did not fit their frame", self.unfit)?;
        }
        if self.bad_lines > 0 {
            write!(f, ", {} line(s) could not be read", self.bad_lines)?;
        }
        Ok(())
    }
}
