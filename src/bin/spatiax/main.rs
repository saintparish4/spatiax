//! `spatiax` on the command line: decode a `candump` log against a DBC,
//! check a DBC for layout problems, or export a decoded log for MoTeC i2.
//!
//! Exit status follows the `grep` convention so scripts can branch on it:
//! 0 when everything was read and nothing was wrong, 1 when the command ran
//! to completion but found something (malformed log lines, layout problems),
//! and 2 when it could not run at all.

mod check;
mod decode;
mod export;
#[cfg(all(feature = "socketcan", target_os = "linux"))]
mod live;
mod output;

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use clap::{Parser, Subcommand};
use spatiax::{Database, dbc};

/// What a subcommand hands back: an exit status, or a message for stderr
/// when it could not run at all.
type Outcome = Result<ExitCode, String>;

const FOUND_SOMETHING: u8 = 1;
const COULD_NOT_RUN: u8 = 2;

#[derive(Parser)]
#[command(
    name = "spatiax",
    version,
    about = "CAN bus / DBC decoder for motorsport telemetry"
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Decode the frames in a candump log
    Decode {
        /// DBC file describing the bus
        dbc: PathBuf,
        /// candump log file, or `-` for stdin
        log: PathBuf,
        /// Output format
        #[arg(long, value_enum, default_value_t = output::Format::Text)]
        format: output::Format,
    },
    /// Report signals that run past the DLC or overlap each other
    Check {
        /// DBC file to check
        dbc: PathBuf,
    },
    /// Write a decoded log as a MoTeC `.ld` file
    Export {
        /// DBC file describing the bus
        dbc: PathBuf,
        /// candump log file
        log: PathBuf,
        /// File to write
        #[arg(short, long)]
        output: PathBuf,
        /// Sample rate in Hz; defaults to the fastest message in the log
        #[arg(long)]
        rate: Option<u16>,
        /// Driver name, as i2 shows it
        #[arg(long)]
        driver: Option<String>,
        /// Vehicle identifier, as i2 shows it
        #[arg(long)]
        vehicle: Option<String>,
        /// Venue name, as i2 shows it
        #[arg(long)]
        venue: Option<String>,
        /// Free text describing the session
        #[arg(long)]
        event: Option<String>,
    },
    /// Decode frames as they arrive on a SocketCAN interface
    #[cfg(all(feature = "socketcan", target_os = "linux"))]
    Live {
        /// DBC file describing the bus
        dbc: PathBuf,
        /// Interface to listen on, such as `can0`
        iface: String,
        /// Output format
        #[arg(long, value_enum, default_value_t = output::Format::Text)]
        format: output::Format,
        /// Stop after this many seconds without a frame
        #[arg(long, value_name = "SECONDS", value_parser = live::seconds)]
        timeout: Option<std::time::Duration>,
    },
}

fn main() -> ExitCode {
    let outcome = match Cli::parse().command {
        Command::Decode { dbc, log, format } => decode::run(&dbc, &log, format),
        Command::Check { dbc } => check::run(&dbc),
        Command::Export {
            dbc,
            log,
            output,
            rate,
            driver,
            vehicle,
            venue,
            event,
        } => export::run(
            &dbc,
            &log,
            &output,
            rate,
            export::Metadata {
                driver,
                vehicle,
                venue,
                event,
            },
        ),
        #[cfg(all(feature = "socketcan", target_os = "linux"))]
        Command::Live {
            dbc,
            iface,
            format,
            timeout,
        } => live::run(&dbc, &iface, format, timeout),
    };
    outcome.unwrap_or_else(|message| {
        eprintln!("spatiax: {message}");
        ExitCode::from(COULD_NOT_RUN)
    })
}

fn load_dbc(path: &Path) -> Result<Database, String> {
    let text = std::fs::read_to_string(path)
        .map_err(|e| format!("cannot read {}: {e}", path.display()))?;
    dbc::parse(&text).map_err(|e| format!("{}: {e}", path.display()))
}
