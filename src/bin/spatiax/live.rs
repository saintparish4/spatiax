//! `spatiax live`: decode frames as they arrive on a SocketCAN interface.
//!
//! Output is flushed after every frame so a terminal sees each one as it
//! happens. The command runs until interrupted or until the socket fails —
//! an interface going down is a reason to stop, not to spin.

use std::io::{self, Write};
use std::path::Path;
use std::process::ExitCode;

use spatiax::Database;
use spatiax::live::Capture;

use crate::decode::{Tally, closed_early, decode_frame};
use crate::output::{Format, Writer};
use crate::{Outcome, load_dbc};

pub fn run(dbc: &Path, iface: &str, format: Format) -> Outcome {
    let db = load_dbc(dbc)?;
    let capture = Capture::open(iface).map_err(|e| format!("cannot open {iface}: {e}"))?;
    let mut writer = Writer::new(io::stdout().lock(), format);

    match stream(&db, capture, &mut writer) {
        Stop::Output(e) if closed_early(&e) => Ok(ExitCode::SUCCESS),
        Stop::Output(e) => Err(format!("cannot write output: {e}")),
        Stop::Socket(e) => Err(format!("{iface}: {e}")),
    }
}

/// Why a session ended. It never ends on its own.
enum Stop {
    Socket(spatiax::Error),
    Output(io::Error),
}

fn stream<W: Write>(db: &Database, mut capture: Capture, writer: &mut Writer<W>) -> Stop {
    let mut tally = Tally::default();
    if let Err(e) = writer.begin().and_then(|()| writer.flush()) {
        return Stop::Output(e);
    }
    loop {
        let frame = match capture.read() {
            Ok(frame) => frame,
            Err(e) => return Stop::Socket(e),
        };
        let shown = decode_frame(db, &frame, writer, &mut tally).and_then(|()| writer.flush());
        if let Err(e) = shown {
            return Stop::Output(e);
        }
    }
}
