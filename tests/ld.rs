//! `.ld` export end to end: the demo lap resampled, a golden file that
//! fails if the layout moves, and a file read back by an implementation
//! nobody here wrote.
//!
//! The oracle test skips when `ldparser` cannot be imported, the way the
//! `cantools` differential test skips. `SPATIAX_REQUIRE_LDPARSER=1` turns a
//! skip into a failure, which is what CI sets. `scripts/fetch_ld_oracle.sh`
//! puts the reader where this looks for it.

use std::collections::HashSet;
use std::ffi::OsString;
use std::io::{BufReader, BufWriter};
use std::path::PathBuf;
use std::process::Command;

use spatiax::{CanFrame, Database, candump, dbc, ld};

const DEMO_DBC: &str = "fixtures/demo/gt3.dbc";
const DEMO_LOG: &str = "fixtures/demo/synthetic_lap.log";
const SAMPLE_DBC: &str = "fixtures/gt3_sample.dbc";
const SAMPLE_LOG: &str = "fixtures/gt3_sample.log";
const GOLDEN: &str = "fixtures/gt3_sample.ld";

/// The rate the golden file was written at. Three samples per channel is
/// enough for a wrong stride between channels to show up as a mismatch.
const GOLDEN_RATE: u16 = 1000;

/// The rate the oracle test exports at. Lower than the log's own 50 Hz so
/// the comparison stays under a hundred thousand values.
const ORACLE_RATE: u16 = 20;

fn manifest_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn load(dbc_path: &str, log_path: &str) -> (Database, Vec<CanFrame>) {
    let text =
        std::fs::read_to_string(manifest_dir().join(dbc_path)).expect("fixture DBC is readable");
    let db = dbc::parse(&text).expect("fixture DBC parses");
    let file = std::fs::File::open(manifest_dir().join(log_path)).expect("fixture log is readable");
    let frames = candump::LogReader::new(BufReader::new(file))
        .filter_map(Result::ok)
        .collect();
    (db, frames)
}

#[test]
fn the_demo_lap_resamples_at_the_rate_its_fastest_message_implies() {
    let (db, frames) = load(DEMO_DBC, DEMO_LOG);
    let sampled = ld::sample(&db, &frames, None).expect("the demo lap has frames");

    // The fastest messages on the lap arrive at 50 Hz and the log spans
    // 82.502976 s, so the grid is 82502976 × 50 / 1e6 + 1 points.
    assert_eq!(sampled.session.rate_hz, 50);
    assert_eq!(sampled.session.sample_count(), 4126);
    for channel in sampled.session.channels() {
        assert_eq!(
            channel.samples.len(),
            4126,
            "channel `{}` is a different length",
            channel.name
        );
    }
}

#[test]
fn every_signal_the_log_carried_becomes_exactly_one_channel() {
    let (db, frames) = load(DEMO_DBC, DEMO_LOG);
    let mut carried = HashSet::new();
    for frame in &frames {
        let Some(decoded) = db.decode_frame(frame) else {
            continue;
        };
        for d in decoded.flatten() {
            carried.insert((frame.id().raw(), d.signal.name.clone()));
        }
    }

    let sampled = ld::sample(&db, &frames, None).expect("the demo lap has frames");
    let channels = sampled.session.channels();
    assert_eq!(channels.len(), carried.len());

    let names: HashSet<_> = channels.iter().map(|c| c.name.as_str()).collect();
    assert_eq!(names.len(), channels.len(), "channel names must be unique");
}

#[test]
fn the_golden_file_still_describes_the_same_bytes() {
    let (db, frames) = load(SAMPLE_DBC, SAMPLE_LOG);
    let sampled = ld::sample(&db, &frames, Some(GOLDEN_RATE)).expect("the fixture log has frames");
    let mut written = Vec::new();
    ld::write(&sampled.session, &mut written).expect("equal-length channels write");

    let golden = std::fs::read(manifest_dir().join(GOLDEN)).expect("the golden file is committed");
    assert_eq!(
        written.len(),
        golden.len(),
        "the file changed length: wrote {} bytes, golden has {}",
        written.len(),
        golden.len()
    );
    if let Some(at) = written.iter().zip(&golden).position(|(a, b)| a != b) {
        panic!(
            "byte {at} changed: wrote {:#04x}, golden has {:#04x}",
            written[at], golden[at]
        );
    }
}

#[test]
fn an_independent_reader_agrees_with_every_value_in_the_file() {
    let Some(python) = ldparser_python() else {
        return;
    };

    let (db, frames) = load(DEMO_DBC, DEMO_LOG);
    let sampled = ld::sample(&db, &frames, Some(ORACLE_RATE)).expect("the demo lap has frames");
    let mut session = sampled.session;
    session.driver = "Reference".to_string();
    session.vehicle = "GT3".to_string();
    session.venue = "Synthetic".to_string();

    let path = std::env::temp_dir().join("spatiax_ld_oracle.ld");
    let file = std::fs::File::create(&path).expect("create the file to read back");
    ld::write(&session, BufWriter::new(file)).expect("equal-length channels write");

    let output = Command::new(&python)
        .arg(manifest_dir().join("tests/oracle/ldparser_oracle.py"))
        .arg(&path)
        .env("PYTHONPATH", ldparser_path())
        .output()
        .expect("launch the oracle interpreter");
    assert!(
        output.status.success(),
        "oracle failed with {}:\n{}",
        output.status,
        String::from_utf8_lossy(&output.stderr)
    );

    let report = String::from_utf8_lossy(&output.stdout);
    let (mut described, mut compared) = (0, 0);
    for line in report.lines() {
        let (tag, rest) = line.split_once(' ').expect("every oracle line has a tag");
        match tag {
            "H" => {
                let fields: Vec<&str> = rest.split('|').collect();
                assert_eq!(fields[0], session.driver);
                assert_eq!(fields[1], session.vehicle);
                assert_eq!(fields[2], session.venue);
                assert_eq!(
                    fields[4].parse::<usize>().expect("a channel count"),
                    session.channels().len()
                );
            }
            "C" => {
                let (index, body) = rest.split_once(' ').expect("a channel line has an index");
                let channel = &session.channels()[index.parse::<usize>().expect("an index")];
                let fields: Vec<&str> = body.split('|').collect();
                assert_eq!(fields[0], channel.name);
                assert_eq!(fields[1], channel.short_name);
                assert_eq!(fields[2], channel.unit);
                assert_eq!(fields[3].parse::<u16>().expect("a rate"), session.rate_hz);
                assert_eq!(
                    fields[4].parse::<usize>().expect("a length"),
                    channel.samples.len()
                );
                assert_eq!(fields[5], "float32");
                described += 1;
            }
            "V" => {
                let mut fields = rest.split(' ');
                let index: usize = fields.next().expect("an index").parse().expect("an index");
                let at: usize = fields
                    .next()
                    .expect("a position")
                    .parse()
                    .expect("a position");
                let bits: u32 = fields.next().expect("bits").parse().expect("bits");
                let channel = &session.channels()[index];
                assert_eq!(
                    bits,
                    channel.samples[at].to_bits(),
                    "channel `{}` sample {at}",
                    channel.name
                );
                compared += 1;
            }
            other => panic!("unexpected oracle line beginning `{other}`"),
        }
    }

    assert_eq!(described, session.channels().len());
    assert_eq!(compared, described * session.sample_count());
    eprintln!("{compared} samples across {described} channels read back identically");
}

/// Where `ldparser.py` was fetched to, which the oracle needs on its path.
fn ldparser_path() -> OsString {
    std::env::var_os("SPATIAX_LDPARSER_PATH")
        .unwrap_or_else(|| manifest_dir().join(".venv/oracle").into_os_string())
}

/// The interpreter that can import `ldparser`, or `None` to skip.
fn ldparser_python() -> Option<PathBuf> {
    let candidates = [
        std::env::var_os("SPATIAX_LDPARSER_PYTHON").map(PathBuf::from),
        Some(manifest_dir().join(".venv/bin/python")),
        Some(PathBuf::from("python3")),
    ];
    let usable = candidates.into_iter().flatten().find(|python| {
        Command::new(python)
            .args(["-c", "import ldparser"])
            .env("PYTHONPATH", ldparser_path())
            .output()
            .is_ok_and(|out| out.status.success())
    });
    if usable.is_none() {
        let message =
            "no candidate interpreter can import ldparser; run scripts/fetch_ld_oracle.sh";
        assert!(
            std::env::var_os("SPATIAX_REQUIRE_LDPARSER").is_none(),
            "{message}"
        );
        eprintln!("skipping the .ld oracle test: {message}");
    }
    usable
}
