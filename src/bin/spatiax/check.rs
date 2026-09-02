//! `spatiax check`: the layout problems `dbc::check` finds, one per line,
//! then a summary. Exit status 1 when there were any.

use std::path::Path;
use std::process::ExitCode;

use spatiax::dbc;

use crate::{FOUND_SOMETHING, Outcome, load_dbc};

pub fn run(path: &Path) -> Outcome {
    let db = load_dbc(path)?;
    let problems = dbc::check(&db);
    for problem in &problems {
        println!("{problem}");
    }
    println!(
        "{}: {} message(s), {} signal(s), {} problem(s)",
        path.display(),
        db.len(),
        db.signal_count(),
        problems.len()
    );
    Ok(if problems.is_empty() {
        ExitCode::SUCCESS
    } else {
        ExitCode::from(FOUND_SOMETHING)
    })
}
