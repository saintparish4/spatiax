//! The parser against real production databases.
//!
//! Every other database in this repository is a shape the project chose:
//! the fixtures by hand, the differential harness by generator. comma.ai's
//! `opendbc` is the opposite — files written by vehicle manufacturers and
//! by people reverse engineering them, none of them written with this
//! parser in mind. What they carry that a generator never produces is mess:
//! comments, attribute definitions, node lists, global value tables,
//! transmitter records, CRLF endings, and the occasional record whose
//! author read the format loosely.
//!
//! `scripts/fetch_dbc_corpus.sh` fetches the corpus at a pinned commit and
//! never vendors it. When it is absent the tests here skip with a message,
//! unless `SPATIAX_REQUIRE_CORPUS` is set — CI sets it, so this cannot
//! quietly become a no-op there. `SPATIAX_DBC_CORPUS` points at a different
//! directory of `.dbc` files.
//!
//! What is asserted is narrow on purpose. A file that fails to parse is
//! only acceptable if it failed for a reason this crate documents as
//! unsupported; anything else fails the run. Layout problems found by
//! `dbc::check` are counted and reported rather than asserted away — they
//! are findings about the databases, not about the parser.

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::process::Command;

use spatiax::dbc::{self, Multiplexing, check};

/// Floors that make an accidentally empty or truncated corpus fail rather
/// than pass quietly. The pinned commit has 58 files, 3,900 messages and
/// 29,000 signals; these sit well below that so a corpus bump does not
/// need a code change, and well above zero.
const MINIMUM_FILES: usize = 40;
const MINIMUM_MESSAGES: usize = 2_500;
const MINIMUM_SIGNALS: usize = 20_000;

/// Where `scripts/fetch_dbc_corpus.sh` puts the corpus by default.
const DEFAULT_CORPUS: &str = ".venv/corpus/opendbc/opendbc/dbc";

fn manifest_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

/// The corpus directory, or `None` with a reason when it is not there.
fn corpus_dir() -> Result<PathBuf, String> {
    let path = match std::env::var_os("SPATIAX_DBC_CORPUS") {
        Some(dir) => PathBuf::from(dir),
        None => manifest_dir().join(DEFAULT_CORPUS),
    };
    if path.is_dir() {
        Ok(path)
    } else {
        Err(format!("{} is not a directory", path.display()))
    }
}

/// Every `.dbc` file directly in `dir`, in name order.
///
/// Deliberately not recursive: `opendbc` keeps template fragments under
/// `generator/`, and a fragment is not a database.
fn databases_in(dir: &Path) -> Vec<PathBuf> {
    let mut files: Vec<PathBuf> = std::fs::read_dir(dir)
        .expect("corpus directory is readable")
        .filter_map(|entry| entry.ok().map(|e| e.path()))
        .filter(|path| path.extension().is_some_and(|ext| ext == "dbc"))
        .collect();
    files.sort();
    files
}

/// What one file contributed to the totals.
struct FileReport {
    name: String,
    messages: usize,
    signals: usize,
    /// Parse failure, if the file did not load at all.
    rejection: Option<String>,
    /// Problems `dbc::check` found, by kind.
    past_dlc: usize,
    overlaps: usize,
    extended_ids: usize,
    multiplexed: usize,
    labelled: usize,
}

/// The limits this crate documents, each paired with the text its error
/// carries. A rejection that matches none of these is a bug, and fails the
/// run.
const DOCUMENTED_LIMITS: [(&str, &str); 3] = [
    (
        "exceeds 11 bits",
        "declares a standard identifier wider than 11 bits",
    ),
    (
        "without naming a page",
        "marks a signal multiplexed without a page number",
    ),
    ("extended multiplexing", "uses extended multiplexing"),
];

/// Which documented limit a rejection falls under, if any.
fn documented_limit(message: &str) -> Option<&'static str> {
    DOCUMENTED_LIMITS
        .iter()
        .find(|(needle, _)| message.contains(needle))
        .map(|(_, label)| *label)
}

fn report(path: &Path) -> FileReport {
    let name = path
        .file_name()
        .expect("corpus entry has a file name")
        .to_string_lossy()
        .into_owned();
    let text = std::fs::read_to_string(path).expect("corpus file is readable");

    let db = match dbc::parse(&text) {
        Ok(db) => db,
        Err(error) => {
            return FileReport {
                name,
                messages: 0,
                signals: 0,
                rejection: Some(error.to_string()),
                past_dlc: 0,
                overlaps: 0,
                extended_ids: 0,
                multiplexed: 0,
                labelled: 0,
            };
        }
    };

    let mut file = FileReport {
        name,
        messages: db.len(),
        signals: db.signal_count(),
        rejection: None,
        past_dlc: 0,
        overlaps: 0,
        extended_ids: 0,
        multiplexed: 0,
        labelled: 0,
    };
    for message in db.messages() {
        if message.id.is_extended() {
            file.extended_ids += 1;
        }
        if message.multiplexor().is_some() {
            file.multiplexed += 1;
        }
        for signal in &message.signals {
            if !signal.value_table.is_empty() {
                file.labelled += 1;
            }
        }
    }
    for problem in check(&db) {
        match problem {
            check::Problem::SignalPastDlc { .. } => file.past_dlc += 1,
            check::Problem::Overlap { .. } => file.overlaps += 1,
        }
    }
    file
}

/// Markdown, so CI can paste it into the job summary unchanged.
fn summary(reports: &[FileReport]) -> String {
    let parsed = reports.iter().filter(|r| r.rejection.is_none()).count();
    let messages: usize = reports.iter().map(|r| r.messages).sum();
    let signals: usize = reports.iter().map(|r| r.signals).sum();
    let extended: usize = reports.iter().map(|r| r.extended_ids).sum();
    let multiplexed: usize = reports.iter().map(|r| r.multiplexed).sum();
    let labelled: usize = reports.iter().map(|r| r.labelled).sum();
    let past_dlc: usize = reports.iter().map(|r| r.past_dlc).sum();
    let overlaps: usize = reports.iter().map(|r| r.overlaps).sum();

    let mut out = String::new();
    let _ = writeln!(out, "| Measure | Count |");
    let _ = writeln!(out, "|---|---|");
    let _ = writeln!(out, "| Files seen | {} |", reports.len());
    let _ = writeln!(out, "| Files parsed | {parsed} |");
    let _ = writeln!(out, "| Messages | {messages} |");
    let _ = writeln!(out, "| Signals | {signals} |");
    let _ = writeln!(out, "| Messages with an extended identifier | {extended} |");
    let _ = writeln!(out, "| Multiplexed messages | {multiplexed} |");
    let _ = writeln!(out, "| Signals carrying a value table | {labelled} |");
    let _ = writeln!(out, "| `check`: signal past the DLC | {past_dlc} |");
    let _ = writeln!(out, "| `check`: overlapping signals | {overlaps} |");

    let rejected: Vec<&FileReport> = reports.iter().filter(|r| r.rejection.is_some()).collect();
    if !rejected.is_empty() {
        let _ = writeln!(out, "\nRejected:\n");
        for file in rejected {
            let message = file.rejection.as_deref().unwrap_or_default();
            let _ = writeln!(
                out,
                "- `{}` — {} — {message}",
                file.name,
                documented_limit(message).unwrap_or("UNDOCUMENTED"),
            );
        }
    }

    let _ = writeln!(out, "\n| File | Messages | Signals | Past DLC | Overlaps |");
    let _ = writeln!(out, "|---|---|---|---|---|");
    for file in reports {
        let _ = writeln!(
            out,
            "| `{}` | {} | {} | {} | {} |",
            file.name, file.messages, file.signals, file.past_dlc, file.overlaps
        );
    }
    out
}

#[test]
fn every_database_in_the_corpus_parses_or_fails_for_a_documented_reason() {
    let dir = match corpus_dir() {
        Ok(dir) => dir,
        Err(message) => {
            assert!(
                std::env::var_os("SPATIAX_REQUIRE_CORPUS").is_none(),
                "{message}, and SPATIAX_REQUIRE_CORPUS is set"
            );
            eprintln!("skipping the corpus test: {message}");
            eprintln!("run `bash scripts/fetch_dbc_corpus.sh` to fetch it");
            return;
        }
    };

    let files = databases_in(&dir);
    let reports: Vec<FileReport> = files.iter().map(|path| report(path)).collect();
    let table = summary(&reports);
    println!("{table}");

    let out = manifest_dir().join("target/corpus-summary.md");
    if let Some(parent) = out.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let _ = std::fs::write(&out, &table);

    let undocumented: BTreeMap<&str, &str> = reports
        .iter()
        .filter_map(|r| r.rejection.as_deref().map(|m| (r.name.as_str(), m)))
        .filter(|(_, message)| documented_limit(message).is_none())
        .collect();
    assert!(
        undocumented.is_empty(),
        "these databases failed to parse for reasons this crate does not document \
         as unsupported: {undocumented:#?}"
    );

    let messages: usize = reports.iter().map(|r| r.messages).sum();
    let signals: usize = reports.iter().map(|r| r.signals).sum();
    assert!(
        reports.len() >= MINIMUM_FILES,
        "corpus has {} files, expected at least {MINIMUM_FILES}",
        reports.len()
    );
    assert!(
        messages >= MINIMUM_MESSAGES,
        "corpus yielded {messages} messages, expected at least {MINIMUM_MESSAGES}"
    );
    assert!(
        signals >= MINIMUM_SIGNALS,
        "corpus yielded {signals} signals, expected at least {MINIMUM_SIGNALS}"
    );
}

#[test]
fn the_corpus_exercises_the_features_the_fixtures_only_sample() {
    let Ok(dir) = corpus_dir() else {
        return;
    };

    let mut extended = 0usize;
    let mut motorola = 0usize;
    let mut signed = 0usize;
    let mut multiplexed_pages = 0usize;
    let mut widths: BTreeMap<u8, usize> = BTreeMap::new();

    for path in databases_in(&dir) {
        let text = std::fs::read_to_string(&path).expect("corpus file is readable");
        let Ok(db) = dbc::parse(&text) else {
            continue;
        };
        for message in db.messages() {
            if message.id.is_extended() {
                extended += 1;
            }
            for signal in &message.signals {
                if signal.byte_order == dbc::ByteOrder::Motorola {
                    motorola += 1;
                }
                if signal.value_type == dbc::ValueType::Signed {
                    signed += 1;
                }
                if matches!(signal.multiplexing, Multiplexing::Multiplexed(_)) {
                    multiplexed_pages += 1;
                }
                *widths.entry(signal.length).or_default() += 1;
            }
        }
    }

    println!(
        "corpus features: {extended} extended ids, {motorola} Motorola signals, \
         {signed} signed signals, {multiplexed_pages} multiplexed signals, \
         {} distinct widths",
        widths.len()
    );

    // Real databases lean Motorola and use the whole width range. If any of
    // these came back zero the corpus would not be testing what it is here
    // to test.
    assert!(motorola > 0, "no Motorola signals in the corpus");
    assert!(signed > 0, "no signed signals in the corpus");
    assert!(extended > 0, "no extended identifiers in the corpus");
    assert!(
        widths.len() > 8,
        "corpus signal widths are suspiciously few"
    );
}

// ------------------------------------------------- the same files, read by cantools

/// One corpus file as `cantools` read it.
#[derive(Default)]
struct OracleFile {
    parsed: bool,
    error: String,
    /// Whether `cantools` accepted the file in strict mode, where it refuses
    /// the layout problems [`check`] reports.
    strict_ok: bool,
    strict_error: String,
    /// `M` and `S` lines, normalised the way [`describe`] normalises this
    /// crate's own view of the same file.
    lines: Vec<String>,
}

/// `M <id> <extended> <dlc> <name>` and
/// `S <id> <start> <len> <order> <sign> <factor> <offset> <mux> <name>`,
/// sorted. Numbers are formatted by Rust on both sides — the oracle's floats
/// are parsed before they are printed again — so no comparison depends on
/// how Python renders a number.
fn describe(db: &dbc::Database) -> Vec<String> {
    let mut lines = Vec::new();
    for message in db.messages() {
        lines.push(format!(
            "M {} {} {} {}",
            message.id.raw(),
            u8::from(message.id.is_extended()),
            message.dlc,
            message.name
        ));
        for signal in &message.signals {
            let order = match signal.byte_order {
                dbc::ByteOrder::Motorola => 'B',
                dbc::ByteOrder::Intel => 'L',
            };
            let sign = match signal.value_type {
                dbc::ValueType::Signed => 'S',
                dbc::ValueType::Unsigned => 'U',
            };
            let mux = match signal.multiplexing {
                Multiplexing::None => "-".to_string(),
                Multiplexing::Multiplexor => "M".to_string(),
                Multiplexing::Multiplexed(page) => format!("m{page}"),
            };
            lines.push(format!(
                "S {} {} {} {order} {sign} {} {} {mux} {}",
                message.id.raw(),
                signal.start_bit,
                signal.length,
                signal.factor,
                signal.offset,
                signal.name
            ));
        }
    }
    lines.sort();
    lines
}

fn read_oracle(text: &str) -> BTreeMap<String, OracleFile> {
    let mut files: BTreeMap<String, OracleFile> = BTreeMap::new();
    for line in text.lines() {
        let mut fields = line.split(' ');
        let (Some(kind), Some(name)) = (fields.next(), fields.next()) else {
            continue;
        };
        let rest: Vec<&str> = fields.collect();
        let file = files.entry(name.to_string()).or_default();
        match kind {
            "F" => {
                file.parsed = rest.first() == Some(&"1");
                file.error = rest[1..].join(" ");
            }
            "K" => {
                file.strict_ok = rest.first() == Some(&"1");
                file.strict_error = rest[1..].join(" ");
            }
            "M" => file.lines.push(format!("M {}", rest.join(" "))),
            "S" => {
                // <id> <start> <len> <order> <sign> <factor> <offset> <mux> <name>
                let factor: f64 = rest[5].parse().expect("oracle factor is a number");
                let offset: f64 = rest[6].parse().expect("oracle offset is a number");
                file.lines.push(format!(
                    "S {} {} {} {} {} {factor} {offset} {} {}",
                    rest[0], rest[1], rest[2], rest[3], rest[4], rest[7], rest[8]
                ));
            }
            _ => {}
        }
    }
    for file in files.values_mut() {
        file.lines.sort();
    }
    files
}

fn oracle_python() -> Option<PathBuf> {
    let candidates = [
        std::env::var_os("SPATIAX_ORACLE_PYTHON").map(PathBuf::from),
        Some(manifest_dir().join(".venv/bin/python")),
        Some(PathBuf::from("python3")),
    ];
    let usable = candidates.into_iter().flatten().find(|python| {
        Command::new(python)
            .args(["-c", "import cantools"])
            .output()
            .is_ok_and(|out| out.status.success())
    });

    if usable.is_none() {
        let message = "cantools is not importable by any candidate Python interpreter";
        assert!(
            std::env::var_os("SPATIAX_REQUIRE_ORACLE").is_none(),
            "{message}, and SPATIAX_REQUIRE_ORACLE is set"
        );
        eprintln!("skipping the corpus comparison: {message}");
    }
    usable
}

#[test]
fn every_message_and_signal_in_the_corpus_reads_the_same_as_cantools() {
    let dir = match corpus_dir() {
        Ok(dir) => dir,
        Err(message) => {
            assert!(
                std::env::var_os("SPATIAX_REQUIRE_CORPUS").is_none(),
                "{message}, and SPATIAX_REQUIRE_CORPUS is set"
            );
            eprintln!("skipping the corpus comparison: {message}");
            return;
        }
    };
    let Some(python) = oracle_python() else {
        return;
    };

    let script = manifest_dir().join("tests/oracle/corpus_oracle.py");
    let output = Command::new(&python)
        .arg(&script)
        .arg(&dir)
        .output()
        .expect("launch the oracle interpreter");
    assert!(
        output.status.success(),
        "corpus oracle failed with {}:\n{}",
        output.status,
        String::from_utf8_lossy(&output.stderr)
    );
    let oracle = read_oracle(&String::from_utf8_lossy(&output.stdout));

    let mut mismatches: Vec<String> = Vec::new();
    let (mut compared_files, mut compared_messages, mut compared_signals) = (0, 0, 0);
    let (mut ours_only, mut theirs_only) = (Vec::new(), Vec::new());
    let mut strict_disagreements = Vec::new();
    let (mut both_refuse_layout, mut both_accept_layout) = (0, 0);

    for path in databases_in(&dir) {
        let name = path.file_name().unwrap().to_string_lossy().into_owned();
        let their = oracle
            .get(&name)
            .expect("oracle saw every file this test saw");
        let text = std::fs::read_to_string(&path).expect("corpus file is readable");
        let ours = dbc::parse(&text);

        match (&ours, their.parsed) {
            (Err(error), false) => {
                // Both refuse it. The reasons are compared in the test above;
                // agreeing that a file is unusable is the point here.
                let _ = error;
            }
            (Err(error), true) => ours_only.push(format!("{name}: we refuse it — {error}")),
            (Ok(_), false) => {
                theirs_only.push(format!("{name}: cantools refuses it — {}", their.error));
            }
            (Ok(db), true) => {
                compared_files += 1;
                let mine = describe(db);
                compared_messages += mine.iter().filter(|l| l.starts_with("M ")).count();
                compared_signals += mine.iter().filter(|l| l.starts_with("S ")).count();
                if mine != their.lines {
                    let only_mine: Vec<&String> =
                        mine.iter().filter(|l| !their.lines.contains(l)).collect();
                    let only_theirs: Vec<&String> =
                        their.lines.iter().filter(|l| !mine.contains(l)).collect();
                    mismatches.push(format!(
                        "{name}: {} line(s) only here, {} only in cantools\n    here:     {:?}\n    cantools: {:?}",
                        only_mine.len(),
                        only_theirs.len(),
                        only_mine.iter().take(3).collect::<Vec<_>>(),
                        only_theirs.iter().take(3).collect::<Vec<_>>(),
                    ));
                }

                // `check` claims to find what cantools refuses in strict
                // mode. On a file both implementations load, the two should
                // reach the same verdict.
                let problems = check(db);
                if problems.is_empty() == their.strict_ok {
                    if problems.is_empty() {
                        both_accept_layout += 1;
                    } else {
                        both_refuse_layout += 1;
                    }
                } else {
                    strict_disagreements.push(format!(
                        "{name}: check found {} problem(s), cantools strict {}{}",
                        problems.len(),
                        if their.strict_ok {
                            "accepted"
                        } else {
                            "refused"
                        },
                        if their.strict_ok {
                            String::new()
                        } else {
                            format!(" — {}", their.strict_error)
                        }
                    ));
                }
            }
        }
    }

    println!(
        "compared {compared_files} files, {compared_messages} messages, {compared_signals} signals"
    );
    if !ours_only.is_empty() {
        println!("refused here but not by cantools:");
        for line in &ours_only {
            println!("  {line}");
        }
    }
    if !theirs_only.is_empty() {
        println!("refused by cantools but not here:");
        for line in &theirs_only {
            println!("  {line}");
        }
    }
    println!(
        "dbc::check and cantools strict mode reached the same verdict on \
         {} of {compared_files} files ({both_refuse_layout} refused by both, \
         {both_accept_layout} accepted by both)",
        both_refuse_layout + both_accept_layout
    );
    if !strict_disagreements.is_empty() {
        println!("check vs cantools strict:");
        for line in &strict_disagreements {
            println!("  {line}");
        }
    }

    assert!(
        mismatches.is_empty(),
        "{} file(s) read differently from cantools:\n{}",
        mismatches.len(),
        mismatches.join("\n")
    );
    assert!(
        strict_disagreements.is_empty(),
        "`dbc::check` and cantools strict mode disagreed about {} file(s):\n  {}",
        strict_disagreements.len(),
        strict_disagreements.join("\n  ")
    );
    assert!(
        compared_files >= MINIMUM_FILES,
        "only {compared_files} files were compared, expected at least {MINIMUM_FILES}"
    );
    assert!(
        compared_signals >= MINIMUM_SIGNALS,
        "only {compared_signals} signals were compared, expected at least {MINIMUM_SIGNALS}"
    );
}
