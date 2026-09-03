//! End-to-end tests of the `spatiax` binary: real process, real files, exit
//! status and both output streams checked.
#![cfg(feature = "cli")]

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

const DBC: &str = "fixtures/gt3_sample.dbc";
const LOG: &str = "fixtures/gt3_sample.log";

const EXPECTED_TEXT: &str = "\
1700000000.000000 100 EngineData
  EngineRPM: 1165 rpm
  CoolantTemp: 10 degC
  ThrottlePos: 0 %
  OilPressure: 0 bar
1700000000.000250 200 WheelSpeeds
  WheelSpeedFL: 46.6 km/h
  WheelSpeedFR: 652.8 km/h
1700000000.000500 300 SuspensionData
  DamperMux: Front right (1)
  DamperPosFR: 1.6 mm
1700000000.000750 300 SuspensionData
  DamperMux: Front left (0)
  DamperPosFL: -0.1 mm
1700000000.001000 18FEEE00 DiagResponse
  ResponseCode: Overheat (1)
1700000000.002000 100 EngineData
  EngineRPM: 1165 rpm
";

fn spatiax(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_spatiax"))
        .args(args)
        .output()
        .expect("spatiax binary runs")
}

fn stdout(output: &Output) -> String {
    String::from_utf8(output.stdout.clone()).expect("stdout is UTF-8")
}

fn stderr(output: &Output) -> String {
    String::from_utf8(output.stderr.clone()).expect("stderr is UTF-8")
}

/// A file under the test target directory, named for the test that made it.
fn scratch(name: &str, contents: &str) -> PathBuf {
    let path = Path::new(env!("CARGO_TARGET_TMPDIR")).join(name);
    std::fs::write(&path, contents).expect("scratch file is writable");
    path
}

#[test]
fn decode_prints_each_frame_with_its_signals_labels_and_units() {
    let output = spatiax(&["decode", DBC, LOG]);
    assert_eq!(stdout(&output), EXPECTED_TEXT);
    assert_eq!(
        stderr(&output).trim(),
        "spatiax: decoded 6 frame(s), 1 not in the DBC, 3 signal(s) did not fit their frame"
    );
    assert_eq!(output.status.code(), Some(0));
}

#[test]
fn decode_csv_writes_a_header_and_one_row_per_decoded_signal() {
    let output = spatiax(&["decode", DBC, LOG, "--format", "csv"]);
    let text = stdout(&output);
    let lines: Vec<_> = text.lines().collect();
    assert_eq!(lines[0], "timestamp,id,message,signal,raw,value,unit,label");
    assert_eq!(lines.len(), 1 + 12);
    assert_eq!(
        lines[1],
        "1700000000.000000,100,EngineData,EngineRPM,4660,1165,rpm,"
    );
    assert_eq!(
        lines[7],
        "1700000000.000500,300,SuspensionData,DamperMux,1,1,,Front right"
    );
    assert_eq!(
        lines[11],
        "1700000000.001000,18FEEE00,DiagResponse,ResponseCode,1,1,,Overheat"
    );
    assert_eq!(output.status.code(), Some(0));
}

#[test]
fn decode_reads_the_log_from_stdin_when_given_a_dash() {
    let mut child = Command::new(env!("CARGO_BIN_EXE_spatiax"))
        .args(["decode", DBC, "-"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spatiax binary starts");
    child
        .stdin
        .take()
        .expect("stdin is piped")
        .write_all(std::fs::read(LOG).unwrap().as_slice())
        .unwrap();
    let output = child.wait_with_output().unwrap();
    assert_eq!(stdout(&output), EXPECTED_TEXT);
    assert_eq!(output.status.code(), Some(0));
}

#[test]
fn decode_reports_malformed_lines_by_number_and_exits_with_status_1() {
    let log = scratch(
        "malformed.log",
        "(1.0) can0 100#3412640000000000\n\
         (1.0) can0 100#GG\n\
         this is not a frame\n\
         (2.0) can0 18FEEE00#FF\n",
    );
    let output = spatiax(&["decode", DBC, log.to_str().unwrap()]);
    let out = stdout(&output);
    assert!(out.contains("  EngineRPM: 1165 rpm\n"), "{out}");
    assert!(
        out.contains("  ResponseCode: Not available (255)\n"),
        "{out}"
    );

    let err = stderr(&output);
    assert!(err.contains("candump log parse error on line 2"), "{err}");
    assert!(err.contains("candump log parse error on line 3"), "{err}");
    assert!(err.contains("2 line(s) could not be read"), "{err}");
    assert_eq!(output.status.code(), Some(1));
}

#[test]
fn decode_csv_quotes_a_label_that_contains_a_comma() {
    let dbc = scratch(
        "comma.dbc",
        "BO_ 1 Status: 1 ECU\n \
         SG_ Mode : 0|8@1+ (1,0) [0|255] \"\" DASH\n\n\
         VAL_ 1 Mode 0 \"Off, fully\" 1 \"On\" ;\n",
    );
    let log = scratch("comma.log", "(0.0) can0 001#00\n(0.0) can0 001#01\n");
    let output = spatiax(&[
        "decode",
        dbc.to_str().unwrap(),
        log.to_str().unwrap(),
        "--format",
        "csv",
    ]);
    let text = stdout(&output);
    assert!(
        text.contains("0.000000,001,Status,Mode,0,0,,\"Off, fully\"\n"),
        "{text}"
    );
    assert!(
        text.contains("0.000000,001,Status,Mode,1,1,,On\n"),
        "{text}"
    );
}

#[test]
fn decode_with_an_unreadable_dbc_exits_with_status_2() {
    let output = spatiax(&["decode", "fixtures/no_such.dbc", LOG]);
    assert!(stdout(&output).is_empty());
    assert!(stderr(&output).contains("cannot read fixtures/no_such.dbc"));
    assert_eq!(output.status.code(), Some(2));
}

#[test]
fn a_malformed_dbc_is_reported_with_its_line_and_exits_with_status_2() {
    let dbc = scratch(
        "broken.dbc",
        "BO_ 1 Fine: 8 ECU\n \
         SG_ A : 0|8@1+ (1,0) [0|0] \"\" X\n\
         BO_ 2 Broken: 8 ECU\n \
         SG_ B : 0|99@1+ (1,0) [0|0] \"\" X\n",
    );
    let output = spatiax(&["check", dbc.to_str().unwrap()]);
    let err = stderr(&output);
    assert!(err.contains("DBC parse error on line 4"), "{err}");
    assert_eq!(output.status.code(), Some(2));
}

#[test]
fn check_passes_the_fixture_and_summarises_it() {
    let output = spatiax(&["check", DBC]);
    assert_eq!(
        stdout(&output).trim(),
        "fixtures/gt3_sample.dbc: 4 message(s), 10 signal(s), 0 problem(s)"
    );
    assert_eq!(output.status.code(), Some(0));
}

#[test]
fn check_lists_each_problem_and_exits_with_status_1() {
    let dbc = scratch(
        "problems.dbc",
        "BO_ 1 Short: 2 ECU\n \
         SG_ Spills : 8|16@1+ (1,0) [0|0] \"\" X\n\
         BO_ 2 Tangled: 8 ECU\n \
         SG_ A : 0|16@1+ (1,0) [0|0] \"\" X\n \
         SG_ B : 15|8@1+ (1,0) [0|0] \"\" X\n",
    );
    let output = spatiax(&["check", dbc.to_str().unwrap()]);
    let lines: Vec<_> = stdout(&output).lines().map(str::to_string).collect();
    assert_eq!(
        lines[..2],
        [
            "Short: signal `Spills` needs 3 byte(s) but the message declares 2",
            "Tangled: signals `A` and `B` overlap",
        ]
    );
    assert!(
        lines[2].ends_with("2 message(s), 3 signal(s), 2 problem(s)"),
        "{}",
        lines[2]
    );
    assert_eq!(output.status.code(), Some(1));
}
