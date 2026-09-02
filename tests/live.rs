//! Live capture over a virtual CAN interface: the library's `Capture` and
//! the `spatiax live` command, fed from a second socket in this process.
//!
//! Needs a `vcan` interface, which this machine may not have, so the tests
//! skip unless `SPATIAX_VCAN` names one. CI brings `vcan0` up and sets
//! `SPATIAX_REQUIRE_VCAN` so a skip there is a failure.
//!
//! ```bash
//! sudo modprobe vcan
//! sudo ip link add dev vcan0 type vcan
//! sudo ip link set vcan0 mtu 72   # so FD frames pass
//! sudo ip link set up vcan0
//! SPATIAX_VCAN=vcan0 cargo test --features socketcan --test live
//! ```
#![cfg(all(feature = "socketcan", target_os = "linux"))]

use std::io::Read;
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use socketcan::{
    CanDataFrame, CanFdFrame, CanFdSocket, EmbeddedFrame, ExtendedId, Socket, StandardId,
};
use spatiax::live::Capture;
use spatiax::{CanFrame, CanId, dbc};

const DBC: &str = "fixtures/gt3_sample.dbc";

fn interface() -> Option<String> {
    match std::env::var("SPATIAX_VCAN") {
        Ok(iface) if !iface.is_empty() => Some(iface),
        _ if std::env::var_os("SPATIAX_REQUIRE_VCAN").is_some() => {
            panic!("SPATIAX_REQUIRE_VCAN is set but SPATIAX_VCAN names no interface")
        }
        _ => {
            eprintln!("skipping: set SPATIAX_VCAN to a vcan interface to run the live tests");
            None
        }
    }
}

fn send(iface: &str, id: CanId, data: &[u8]) {
    let socket = CanFdSocket::open(iface).expect("open a sending socket");
    match id {
        CanId::Standard(raw) => {
            let id = StandardId::new(raw).unwrap();
            socket
                .write_frame(&CanDataFrame::new(id, data).unwrap())
                .unwrap();
        }
        CanId::Extended(raw) => {
            let id = ExtendedId::new(raw).unwrap();
            socket
                .write_frame(&CanFdFrame::new(id, data).unwrap())
                .unwrap();
        }
    }
}

fn now_us() -> u64 {
    u64::try_from(
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_micros(),
    )
    .unwrap()
}

/// Frames from `capture` on a channel, so a test that never gets the frame
/// it is waiting for fails on a timeout instead of hanging the job.
fn frames_from(mut capture: Capture) -> impl Iterator<Item = CanFrame> {
    let (tx, rx) = mpsc::channel();
    thread::spawn(move || while tx.send(capture.read()).is_ok() {});
    std::iter::from_fn(move || {
        Some(
            rx.recv_timeout(Duration::from_secs(5))
                .expect("a frame arrives within five seconds")
                .expect("the socket read succeeds"),
        )
    })
}

#[test]
fn capture_yields_sent_frames_with_a_receive_timestamp_from_the_kernel_clock() {
    let Some(iface) = interface() else { return };
    let capture = Capture::open(&iface).expect("open the interface");
    let mut frames = frames_from(capture);

    let before = now_us();
    send(
        &iface,
        CanId::Standard(0x100),
        &[0x34, 0x12, 0x64, 0, 0, 0, 0, 0],
    );
    send(&iface, CanId::Extended(0x18FE_EE00), &[0xFF; 12]);
    let after = now_us();

    // The other test shares the interface, so look for this test's frames
    // rather than assuming the next one is ours.
    let classic = frames.find(|f| f.id() == CanId::Standard(0x100)).unwrap();
    assert_eq!(classic.data()[..3], [0x34, 0x12, 0x64]);
    assert!(
        (before..=after).contains(&classic.timestamp_us),
        "receive time {} is not between {before} and {after}",
        classic.timestamp_us
    );

    let fd = frames
        .find(|f| f.id() == CanId::Extended(0x18FE_EE00))
        .unwrap();
    assert_eq!(fd.len(), 12);
    assert!(fd.timestamp_us >= classic.timestamp_us);

    let db = dbc::parse(&std::fs::read_to_string(DBC).unwrap()).unwrap();
    let rpm = db.decode_frame(&classic).unwrap().next().unwrap().unwrap();
    assert_eq!((rpm.signal.name.as_str(), rpm.value), ("EngineRPM", 1165.0));
}

#[test]
fn live_command_prints_frames_as_they_arrive_and_stops_when_told() {
    let Some(iface) = interface() else { return };
    let mut child = Command::new(env!("CARGO_BIN_EXE_spatiax"))
        .args(["live", DBC, &iface, "--format", "csv"])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spatiax binary starts");

    // Give the child time to bind before anything is sent.
    thread::sleep(Duration::from_millis(500));
    send(
        &iface,
        CanId::Standard(0x300),
        &[0x01, 0x10, 0, 0, 0, 0, 0, 0],
    );
    send(&iface, CanId::Standard(0x7FF), &[0; 8]);
    thread::sleep(Duration::from_millis(500));

    child.kill().unwrap();
    let mut stdout = String::new();
    child
        .stdout
        .take()
        .unwrap()
        .read_to_string(&mut stdout)
        .unwrap();
    child.wait().unwrap();

    let lines: Vec<_> = stdout.lines().collect();
    assert_eq!(
        lines.first().copied(),
        Some("timestamp,id,message,signal,raw,value,unit,label"),
        "{stdout}"
    );
    assert!(
        lines
            .iter()
            .any(|l| l.ends_with(",300,SuspensionData,DamperMux,1,1,,Front right")),
        "{stdout}"
    );
    assert!(
        lines
            .iter()
            .any(|l| l.ends_with(",300,SuspensionData,DamperPosFR,16,1.6,mm,")),
        "{stdout}"
    );
    assert!(
        !stdout.contains("7FF"),
        "an undescribed frame is not printed: {stdout}"
    );
}
