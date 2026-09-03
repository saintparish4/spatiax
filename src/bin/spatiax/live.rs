//! `spatiax live`: decode frames as they arrive on a SocketCAN interface.
//!
//! Output is flushed after every frame so a terminal sees each one as it
//! happens. The command runs until interrupted, until the socket fails — an
//! interface going down is a reason to stop, not to spin — or, with
//! `--timeout`, until the bus has been quiet for that long.

use std::io::{self, Write};
use std::path::Path;
use std::process::ExitCode;
use std::time::Duration;

use spatiax::Database;
use spatiax::live::Capture;

use crate::decode::{Tally, closed_early, decode_frame};
use crate::output::{Format, Writer};
use crate::{Outcome, load_dbc};

pub fn run(dbc: &Path, iface: &str, format: Format, timeout: Option<Duration>) -> Outcome {
    let db = load_dbc(dbc)?;
    let capture = Capture::open(iface).map_err(|e| format!("cannot open {iface}: {e}"))?;
    capture
        .set_read_timeout(timeout)
        .map_err(|e| format!("cannot set a read timeout on {iface}: {e}"))?;
    let mut writer = Writer::new(io::stdout().lock(), format);

    let (stop, tally) = stream(&db, capture, &mut writer);
    match stop {
        Stop::Quiet => {
            eprintln!("spatiax: {tally}");
            Ok(ExitCode::SUCCESS)
        }
        Stop::Socket(e) => {
            eprintln!("spatiax: {tally}");
            Err(format!("{iface}: {e}"))
        }
        Stop::Output(e) if closed_early(&e) => Ok(ExitCode::SUCCESS),
        Stop::Output(e) => Err(format!("cannot write output: {e}")),
    }
}

/// A `--timeout` argument: seconds, as a number a stopwatch would show.
pub fn seconds(text: &str) -> Result<Duration, String> {
    match text.parse::<f64>() {
        Ok(seconds) if seconds > 0.0 && seconds.is_finite() => Ok(Duration::from_secs_f64(seconds)),
        _ => Err("expected a positive number of seconds".to_string()),
    }
}

/// Why a session ended. Without `--timeout` it never ends on its own.
enum Stop {
    /// No frame arrived within the read timeout.
    Quiet,
    Socket(spatiax::Error),
    Output(io::Error),
}

fn stream<W: Write>(db: &Database, mut capture: Capture, writer: &mut Writer<W>) -> (Stop, Tally) {
    let mut tally = Tally::default();
    if let Err(e) = writer.begin().and_then(|()| writer.flush()) {
        return (Stop::Output(e), tally);
    }
    loop {
        let frame = match capture.read() {
            Ok(Some(frame)) => frame,
            Ok(None) => return (Stop::Quiet, tally),
            Err(e) => return (Stop::Socket(e), tally),
        };
        let shown = decode_frame(db, &frame, writer, &mut tally).and_then(|()| writer.flush());
        if let Err(e) = shown {
            return (Stop::Output(e), tally);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_timeout_argument_is_a_positive_number_of_seconds() {
        assert_eq!(seconds("2"), Ok(Duration::from_secs(2)));
        assert_eq!(seconds("0.25"), Ok(Duration::from_millis(250)));
        for rejected in ["0", "-1", "", "soon", "inf", "NaN"] {
            assert!(seconds(rejected).is_err(), "{rejected}");
        }
    }
}
