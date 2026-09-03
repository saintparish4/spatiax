//! The demo lap in `fixtures/demo` is what the README shows people, so it
//! must keep decoding cleanly and keep looking like a lap: every frame known
//! to the database, every signal in range, lap distance and time only ever
//! going up.

use std::collections::HashMap;

use spatiax::CanFrame;
use spatiax::candump::LogReader;
use spatiax::dbc::{self, Database};

const DBC: &str = include_str!("../fixtures/demo/gt3.dbc");
const LOG: &str = include_str!("../fixtures/demo/synthetic_lap.log");
const FRAMES: usize = 23_102;

fn database() -> Database {
    dbc::parse(DBC).expect("demo DBC parses")
}

fn frames() -> Vec<CanFrame> {
    LogReader::new(LOG.as_bytes())
        .collect::<Result<_, _>>()
        .expect("every line of the demo log parses")
}

/// Every decoded value, in log order, grouped by signal name.
fn channels(db: &Database, frames: &[CanFrame]) -> HashMap<String, Vec<f64>> {
    let mut channels: HashMap<String, Vec<f64>> = HashMap::new();
    for frame in frames {
        let decoded = db.decode_frame(frame).expect("frame is in the DBC");
        for d in decoded {
            let d = d.expect("signal fits its frame");
            channels
                .entry(d.signal.name.clone())
                .or_default()
                .push(d.value);
        }
    }
    channels
}

fn range(values: &[f64]) -> (f64, f64) {
    values
        .iter()
        .fold((f64::MAX, f64::MIN), |(lo, hi), &v| (lo.min(v), hi.max(v)))
}

fn assert_within(channels: &HashMap<String, Vec<f64>>, name: &str, lo: f64, hi: f64) {
    let (min, max) = range(&channels[name]);
    assert!(
        min >= lo && max <= hi,
        "{name} spans {min}..{max}, expected within {lo}..{hi}"
    );
}

#[test]
fn the_demo_database_is_clean() {
    let db = database();
    assert!(dbc::check(&db).is_empty());
    assert_eq!(db.messages().count(), 7);
}

#[test]
fn every_frame_of_the_demo_lap_decodes_without_error() {
    let db = database();
    let frames = frames();
    assert_eq!(frames.len(), FRAMES);
    let channels = channels(&db, &frames);
    assert_eq!(channels.len(), 43);
    assert!(channels.values().all(|values| !values.is_empty()));
}

#[test]
fn the_demo_lap_reads_like_a_gt3_lap() {
    let db = database();
    let channels = channels(&db, &frames());
    assert_within(&channels, "Speed", 60.0, 300.0);
    assert_within(&channels, "EngineRPM", 4000.0, 8800.0);
    assert_within(&channels, "Gear", 2.0, 6.0);
    assert_within(&channels, "ThrottlePos", 0.0, 100.0);
    assert_within(&channels, "BrakePressureFront", 0.0, 120.0);
    assert_within(&channels, "SteeringAngle", -90.0, 90.0);
    assert_within(&channels, "LatAccel", -1.6, 1.6);
    assert_within(&channels, "LongAccel", -1.6, 1.0);
    assert_within(&channels, "TyreTempOuterFL", 60.0, 120.0);
    assert_within(&channels, "TyrePressureFL", 1.5, 2.5);
    assert_within(&channels, "LapNumber", 7.0, 7.0);
}

#[test]
fn lap_distance_and_time_only_go_up() {
    let db = database();
    let channels = channels(&db, &frames());
    for name in ["LapDistance", "LapTime"] {
        let values = &channels[name];
        assert!(
            values.windows(2).all(|w| w[0] <= w[1]),
            "{name} went backwards"
        );
    }
    let (_, distance) = range(&channels["LapDistance"]);
    let (_, time) = range(&channels["LapTime"]);
    assert!((3400.0..3700.0).contains(&distance), "lap is {distance} m");
    assert!((75.0..95.0).contains(&time), "lap took {time} s");
}

#[cfg(feature = "cli")]
mod binary {
    use std::process::Command;

    use super::FRAMES;

    #[test]
    fn the_binary_turns_the_demo_lap_into_clean_csv() {
        let output = Command::new(env!("CARGO_BIN_EXE_spatiax"))
            .args([
                "decode",
                "--format",
                "csv",
                "fixtures/demo/gt3.dbc",
                "fixtures/demo/synthetic_lap.log",
            ])
            .output()
            .expect("spatiax binary runs");
        assert_eq!(output.status.code(), Some(0));
        assert_eq!(
            String::from_utf8_lossy(&output.stderr).trim(),
            format!("spatiax: decoded {FRAMES} frame(s)")
        );
        let csv = String::from_utf8(output.stdout).expect("CSV is UTF-8");
        let rows: Vec<&str> = csv.lines().skip(1).collect();
        assert!(rows.len() > 100_000, "{} rows", rows.len());
        // Values print at the precision the DBC gives them, never as float noise.
        let noisy = rows
            .iter()
            .filter(|row| row.contains("0000000") || row.contains("9999999"))
            .count();
        assert_eq!(noisy, 0);
    }
}
