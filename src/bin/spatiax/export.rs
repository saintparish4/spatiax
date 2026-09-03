//! `spatiax export`: decode a `candump` log and write it as a MoTeC `.ld`.
//!
//! The summary goes to stderr and the file to disk, so a pipeline that
//! redirects stderr still gets a clean file. Anything the export could not
//! use — an unreadable log line, a frame too short for its signals — is
//! counted and reported rather than silently dropped, and moves the exit
//! status to 1.

use std::fs::File;
use std::io::{BufReader, BufWriter};
use std::path::Path;
use std::process::ExitCode;

use spatiax::candump::LogReader;
use spatiax::{CanFrame, Error, ld};

use crate::{FOUND_SOMETHING, Outcome, load_dbc};

/// The strings i2 displays that neither the DBC nor the log carries.
#[derive(Debug, Default)]
pub struct Metadata {
    /// Driver name.
    pub driver: Option<String>,
    /// Vehicle identifier.
    pub vehicle: Option<String>,
    /// Venue name.
    pub venue: Option<String>,
    /// Free text describing the session.
    pub event: Option<String>,
}

impl Metadata {
    fn apply(self, session: &mut ld::Session) {
        if let Some(driver) = self.driver {
            session.driver = driver;
        }
        if let Some(vehicle) = self.vehicle {
            session.vehicle = vehicle;
        }
        if let Some(venue) = self.venue {
            session.venue = venue;
        }
        if let Some(event) = self.event {
            session.event = event;
        }
    }
}

pub fn run(dbc: &Path, log: &Path, output: &Path, rate: Option<u16>, meta: Metadata) -> Outcome {
    let db = load_dbc(dbc)?;
    let (frames, unreadable) = read_log(log)?;

    let sampled = ld::sample(&db, &frames, rate).map_err(|e| e.to_string())?;
    let mut session = sampled.session;
    meta.apply(&mut session);

    let file =
        File::create(output).map_err(|e| format!("cannot write {}: {e}", output.display()))?;
    ld::write(&session, BufWriter::new(file)).map_err(|e| format!("{}: {e}", output.display()))?;

    eprintln!(
        "spatiax: {} — {} channels at {} Hz, {} samples each, from {} frames",
        output.display(),
        session.channels().len(),
        session.rate_hz,
        session.sample_count(),
        frames.len()
    );
    if sampled.skipped > 0 {
        eprintln!(
            "spatiax: {} signal value(s) dropped: a frame was too short to carry them",
            sampled.skipped
        );
    }
    if unreadable > 0 {
        eprintln!("spatiax: {unreadable} log line(s) could not be read");
    }

    Ok(if sampled.skipped > 0 || unreadable > 0 {
        ExitCode::from(FOUND_SOMETHING)
    } else {
        ExitCode::SUCCESS
    })
}

/// Every frame in the log, and how many lines could not be read.
///
/// A malformed line is counted and skipped: one bad line in an hour of
/// logging should not cost the other fifty-nine minutes.
fn read_log(path: &Path) -> Result<(Vec<CanFrame>, usize), String> {
    let file = File::open(path).map_err(|e| format!("cannot read {}: {e}", path.display()))?;
    let mut frames = Vec::new();
    let mut unreadable = 0;
    for result in LogReader::new(BufReader::new(file)) {
        match result {
            Ok(frame) => frames.push(frame),
            Err(Error::CandumpParse { .. }) => unreadable += 1,
            Err(e) => return Err(format!("{}: {e}", path.display())),
        }
    }
    Ok((frames, unreadable))
}
