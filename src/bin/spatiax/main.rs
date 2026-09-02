//! `spatiax` on the command line: decode a `candump` log against a DBC, or
//! check a DBC for layout problems.
//!
//! Exit status follows the `grep` convention so scripts can branch on it:
//! 0 when everything was read and nothing was wrong, 1 when the command ran
//! to completion but found something (malformed log lines, layout problems),
//! and 2 when it could not run at all.

mod check;
mod decode;
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
}

fn main() -> ExitCode {
    let outcome = match Cli::parse().command {
        Command::Decode { dbc, log, format } => decode::run(&dbc, &log, format),
        Command::Check { dbc } => check::run(&dbc),
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
