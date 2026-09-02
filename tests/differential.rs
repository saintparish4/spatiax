//! Differential test against `cantools`, the reference DBC implementation.
//!
//! I generate random but valid databases and random frames, decode them
//! here, and hand the same inputs to `tests/oracle/cantools_oracle.py`. Raw
//! values must agree exactly; physical values must agree to within float
//! noise. The encoder is checked the same way: re-encoding the raw values
//! must reproduce the bytes `cantools` produces.
//!
//! About a third of the generated messages are multiplexed, with pages that
//! overlap one another, so the set of signals each side considers present is
//! compared too. Multiplexors are generated unscaled because `cantools`
//! selects the page by the *scaled* multiplexor value when scaling is on,
//! which is a quirk rather than something to agree with.
//!
//! The oracle needs Python 3 with `cantools` importable. When it is not
//! available the test skips with a message, unless `SPATIAX_REQUIRE_ORACLE`
//! is set — CI sets it, so this cannot silently degrade into a no-op there.
//!
//! Every run prints its seed; rerun with `SPATIAX_DIFFERENTIAL_SEED=<seed>`
//! to reproduce a failure. On failure the generated files are left in
//! `target/differential/<seed>/` for inspection.

use std::collections::{HashMap, HashSet};
use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::process::Command;

use rand::rngs::StdRng;
use rand::seq::{IndexedRandom, SliceRandom};
use rand::{Rng, SeedableRng};
use spatiax::dbc::{self, ByteOrder, Database, Message, Multiplexing, Signal, ValueType};
use spatiax::encode::insert_raw;
use spatiax::{CanFrame, CanId};

/// The published exit criterion: at least this many decoded signal values.
const MINIMUM_CASES: usize = 100_000;
const DEFAULT_CASE_TARGET: usize = 150_000;
const FRAMES_PER_MESSAGE: usize = 48;
const MAX_MISMATCHES_REPORTED: usize = 20;
const MULTIPLEXED_MESSAGE_RATE: f64 = 0.35;
/// How often a frame of a multiplexed message is steered onto a defined page.
const CLAIMED_SELECTOR_RATE: f64 = 0.85;

/// Classic CAN lengths weighted towards 8, plus every CAN FD length.
const DLCS: &[usize] = &[
    1, 2, 3, 4, 5, 6, 7, 8, 8, 8, 8, 8, 12, 16, 20, 24, 32, 48, 64,
];
const FACTORS: &[f64] = &[
    1.0, 2.0, 0.5, 0.25, 0.125, 0.1, 0.01, 0.001, 0.0625, 10.0, 100.0,
];
const OFFSETS: &[f64] = &[
    0.0, 0.0, -40.0, 1.0, -1.0, 0.5, -273.15, 100.0, -128.0, 1000.0,
];

struct Generated {
    text: String,
    /// Per message, in DBC order: its name, identifier, and frame payloads.
    frames: Vec<(String, CanId, Vec<Vec<u8>>)>,
    /// Signal values the frames are expected to produce; the loop that
    /// decides how many databases to generate works from this estimate.
    cases: usize,
}

#[test]
fn spatiax_agrees_with_cantools_on_generated_databases() {
    let Some(python) = oracle_python() else {
        return;
    };

    let seed = env_number("SPATIAX_DIFFERENTIAL_SEED").unwrap_or_else(rand::random);
    let target = env_number("SPATIAX_DIFFERENTIAL_CASES").unwrap_or(DEFAULT_CASE_TARGET);
    eprintln!("differential seed = {seed}, case target = {target}");

    let dir = workspace_dir(seed);
    let databases = write_databases(&dir, StdRng::seed_from_u64(seed), target);
    let summary = run_oracle(&python, &dir);
    eprintln!("oracle: {}", summary.trim());

    let mut mismatches = Vec::new();
    let mut cases = 0;
    for (index, generated) in databases.iter().enumerate() {
        let db = dbc::parse(&generated.text).expect("generated DBC parses");
        let expected = read_expected(&dir.join(format!("{index:03}.expected")));
        let comparison = Comparison::new(&db, &expected, index).run(generated);
        mismatches.extend(comparison.mismatches);
        cases += comparison.cases;
    }

    report(&mismatches, seed, &dir, cases);
    std::fs::remove_dir_all(&dir).expect("clean up generated files");
}

fn report(mismatches: &[String], seed: u64, dir: &Path, cases: usize) {
    if !mismatches.is_empty() {
        let shown: Vec<_> = mismatches.iter().take(MAX_MISMATCHES_REPORTED).collect();
        panic!(
            "{} mismatch(es) against cantools (seed {seed}, files kept in {}):\n{}",
            mismatches.len(),
            dir.display(),
            shown
                .iter()
                .map(|m| format!("  {m}"))
                .collect::<Vec<_>>()
                .join("\n"),
        );
    }
    eprintln!("{cases} decoded signal values agreed with cantools");
    assert!(
        cases >= MINIMUM_CASES,
        "only {cases} cases were generated; the criterion is {MINIMUM_CASES}"
    );
}

// ---------------------------------------------------------------- oracle

/// The interpreter to run the oracle with, or `None` to skip the test.
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
        eprintln!("skipping differential test: {message}");
    }
    usable
}

fn run_oracle(python: &Path, dir: &Path) -> String {
    let script = manifest_dir().join("tests/oracle/cantools_oracle.py");
    let output = Command::new(python)
        .arg(&script)
        .arg(dir)
        .output()
        .expect("launch the oracle interpreter");
    assert!(
        output.status.success(),
        "oracle failed with {}:\n{}",
        output.status,
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout).into_owned()
}

/// `S <frame> <signal> <raw> <value>`, `E <frame> <hex>`, and
/// `U <frame> <selector>` lines, grouped by frame index.
#[derive(Default)]
struct Expected {
    signals: HashMap<usize, HashMap<String, (i128, f64)>>,
    encoded: HashMap<usize, Vec<u8>>,
    unclaimed: HashMap<usize, i128>,
}

fn read_expected(path: &Path) -> Expected {
    let text = std::fs::read_to_string(path).expect("oracle wrote an expectation file");
    let mut expected = Expected::default();
    for line in text.lines() {
        let fields: Vec<&str> = line.split_whitespace().collect();
        let frame: usize = fields[1].parse().unwrap();
        match fields[0] {
            "S" => {
                let raw = fields[3].parse().unwrap();
                let value = fields[4].parse().unwrap();
                expected
                    .signals
                    .entry(frame)
                    .or_default()
                    .insert(fields[2].to_string(), (raw, value));
            }
            "E" => {
                expected.encoded.insert(frame, unhex(fields[2]));
            }
            "U" => {
                expected.unclaimed.insert(frame, fields[2].parse().unwrap());
            }
            other => panic!("unexpected oracle record `{other}`"),
        }
    }
    expected
}

// ------------------------------------------------------------ comparison

struct Comparison<'a> {
    db: &'a Database,
    expected: &'a Expected,
    db_index: usize,
    mismatches: Vec<String>,
    cases: usize,
}

impl<'a> Comparison<'a> {
    fn new(db: &'a Database, expected: &'a Expected, db_index: usize) -> Self {
        Self {
            db,
            expected,
            db_index,
            mismatches: Vec::new(),
            cases: 0,
        }
    }

    fn run(mut self, generated: &Generated) -> Self {
        let mut frame_index = 0;
        for (name, id, payloads) in &generated.frames {
            let message = self.db.message(*id).expect("generated message parses");
            assert_eq!(&message.name, name);
            for payload in payloads {
                let context = format!("db {:03} frame {frame_index} ({name})", self.db_index);
                self.frame(message, payload, frame_index, &context);
                frame_index += 1;
            }
        }
        self
    }

    fn frame(&mut self, message: &Message, payload: &[u8], frame_index: usize, context: &str) {
        let frame = CanFrame::new(message.id, payload, 0).unwrap();
        let decoded: Vec<_> = self
            .db
            .decode_frame(&frame)
            .expect("id is in the database")
            .map(|d| d.expect("generated signals fit their frames"))
            .collect();

        if let Some(&selector) = self.expected.unclaimed.get(&frame_index) {
            self.unclaimed_frame(&decoded, selector, context);
            return;
        }

        let want = &self.expected.signals[&frame_index];
        for d in &decoded {
            match want.get(&d.signal.name) {
                Some(&(raw, value)) => self.signal(d, raw, value, context),
                None => self.mismatch(format!(
                    "{context} {}: present here, absent in cantools",
                    d.signal.name
                )),
            }
        }
        if decoded.len() != want.len() {
            self.mismatch(format!(
                "{context}: decoded {} signals, cantools decoded {}",
                decoded.len(),
                want.len()
            ));
        }

        let re_encoded = encode_all(&decoded, payload.len());
        if re_encoded != self.expected.encoded[&frame_index] {
            self.mismatch(format!(
                "{context}: re-encoded {} != cantools {}",
                hex(&re_encoded),
                hex(&self.expected.encoded[&frame_index])
            ));
        }
    }

    fn signal(&mut self, d: &dbc::Decoded<'_>, want_raw: i128, want_value: f64, context: &str) {
        self.cases += 1;
        let got_raw = as_reference_raw(d.raw, d.signal);
        if got_raw != want_raw {
            self.mismatch(format!(
                "{context} {}: raw {got_raw} != cantools {want_raw} ({})",
                d.signal.name,
                describe(d.signal)
            ));
        } else if !close(d.value, want_value) {
            self.mismatch(format!(
                "{context} {}: value {} != cantools {want_value} ({})",
                d.signal.name,
                d.value,
                describe(d.signal)
            ));
        }
    }

    /// `cantools` refused the frame because no page matches the selector it
    /// saw. I must have read the same selector and yielded no page at all.
    fn unclaimed_frame(&mut self, decoded: &[dbc::Decoded<'_>], selector: i128, context: &str) {
        self.cases += 1;
        let multiplexor = decoded
            .iter()
            .find(|d| d.signal.multiplexing == Multiplexing::Multiplexor);
        match multiplexor {
            Some(m) if i128::from(m.raw) != selector => self.mismatch(format!(
                "{context}: multiplexor {} != cantools {selector}",
                m.raw
            )),
            Some(_) => {}
            None => self.mismatch(format!("{context}: no multiplexor decoded")),
        }
        for d in decoded {
            if let Multiplexing::Multiplexed(n) = d.signal.multiplexing {
                self.mismatch(format!(
                    "{context} {}: page {n} decoded, cantools found no page for {selector}",
                    d.signal.name
                ));
            }
        }
    }

    fn mismatch(&mut self, text: String) {
        self.mismatches.push(text);
    }
}

/// Insert every decoded signal's raw value into a zeroed payload, as
/// `cantools` does with `padding=False`.
fn encode_all(decoded: &[dbc::Decoded<'_>], len: usize) -> Vec<u8> {
    let mut out = vec![0u8; len];
    for d in decoded {
        insert_raw(&mut out, d.signal, d.raw).unwrap();
    }
    out
}

/// `cantools` reports signed raws as negative integers; mine are bit patterns.
fn as_reference_raw(raw: u64, signal: &Signal) -> i128 {
    match signal.value_type {
        ValueType::Unsigned => i128::from(raw),
        ValueType::Signed => {
            let shift = 64 - u32::from(signal.length);
            i128::from(((raw << shift) as i64) >> shift)
        }
    }
}

/// `cantools` uses exact integer arithmetic when factor and offset are
/// integral, so beyond 2^53 the two sides can differ by float rounding.
/// Raw values are compared exactly, so this only guards the scaling step.
fn close(a: f64, b: f64) -> bool {
    let scale = a.abs().max(b.abs());
    (a - b).abs() <= 1e-12 * scale.max(1.0)
}

fn describe(s: &Signal) -> String {
    let order = match s.byte_order {
        ByteOrder::Intel => "@1",
        ByteOrder::Motorola => "@0",
    };
    let sign = match s.value_type {
        ValueType::Unsigned => '+',
        ValueType::Signed => '-',
    };
    format!(
        "{}{}|{}{order}{sign} ({},{})",
        multiplexing_token(s),
        s.start_bit,
        s.length,
        s.factor,
        s.offset
    )
}

// ------------------------------------------------------------ generation

fn write_databases(dir: &Path, mut rng: StdRng, target: usize) -> Vec<Generated> {
    std::fs::create_dir_all(dir).expect("create the working directory");
    let mut databases = Vec::new();
    let mut cases = 0;
    while cases < target {
        let generated = generate_database(&mut rng);
        let index = databases.len();
        std::fs::write(dir.join(format!("{index:03}.dbc")), &generated.text).unwrap();
        std::fs::write(
            dir.join(format!("{index:03}.frames")),
            render_frames(&generated),
        )
        .unwrap();
        cases += generated.cases;
        databases.push(generated);
    }
    databases
}

fn render_frames(generated: &Generated) -> String {
    let mut out = String::new();
    for (name, _, payloads) in &generated.frames {
        for payload in payloads {
            writeln!(out, "{name} {}", hex(payload)).unwrap();
        }
    }
    out
}

fn generate_database(rng: &mut StdRng) -> Generated {
    let mut text = String::new();
    let mut frames = Vec::new();
    let mut cases = 0;
    let mut ids = HashSet::new();

    let message_count = rng.random_range(1..=8);
    while frames.len() < message_count {
        let id = random_id(rng);
        if !ids.insert(id) {
            continue;
        }
        let message = generate_message(rng, id, frames.len());
        let payloads: Vec<_> = (0..FRAMES_PER_MESSAGE)
            .map(|_| random_frame(rng, &message))
            .collect();

        text.push_str(&render_message(&message));
        cases += payloads
            .iter()
            .map(|p| message.decode(p).count())
            .sum::<usize>();
        frames.push((message.name, id, payloads));
    }

    Generated {
        text,
        frames,
        cases,
    }
}

fn random_id(rng: &mut StdRng) -> CanId {
    if rng.random_bool(0.3) {
        CanId::Extended(rng.random_range(0..=0x1FFF_FFFF))
    } else {
        CanId::Standard(rng.random_range(0..=0x7FF))
    }
}

/// A message whose signals fit the DLC and whose signals present together
/// never overlap — both conditions `cantools` enforces in strict mode.
/// Signals on different pages of a multiplexed message may overlap, and
/// usually do.
fn generate_message(rng: &mut StdRng, id: CanId, index: usize) -> Message {
    let dlc = *DLCS.choose(rng).unwrap();
    let mut layout = Layout::new(dlc * 8);
    let mut signals = Vec::new();

    let multiplexor = rng
        .random_bool(MULTIPLEXED_MESSAGE_RATE)
        .then(|| layout.place_multiplexor(rng));
    let plain = match multiplexor {
        Some(_) => rng.random_range(0..=6),
        None => rng.random_range(1..=16),
    };
    signals.extend(layout.place_signals(rng, plain, Multiplexing::None));
    if let Some(mut multiplexor) = multiplexor {
        let pages = layout.place_pages(rng, &multiplexor);
        // A one-byte payload can be full before any page fits; a multiplexor
        // with nothing to select is then just a plain signal.
        if pages.is_empty() {
            multiplexor.multiplexing = Multiplexing::None;
        }
        signals.extend(pages);
        signals.push(multiplexor);
    }
    signals.shuffle(rng);

    Message {
        id,
        name: format!("M{index}"),
        dlc: dlc as u8,
        sender: "Node".into(),
        signals,
    }
}

/// Bit occupancy of a payload while signals are being placed into it.
struct Layout {
    occupied: Vec<bool>,
    next_name: usize,
}

impl Layout {
    fn new(bits: usize) -> Self {
        Self {
            occupied: vec![false; bits],
            next_name: 0,
        }
    }

    /// Up to `wanted` non-overlapping signals; attempts that collide are
    /// dropped. The first attempt on an empty payload always succeeds.
    fn place_signals(
        &mut self,
        rng: &mut StdRng,
        wanted: usize,
        multiplexing: Multiplexing,
    ) -> Vec<Signal> {
        let mut placed = Vec::new();
        for _ in 0..64 {
            if placed.len() == wanted {
                break;
            }
            if let Some(signal) = self.try_place(rng, multiplexing) {
                placed.push(signal);
            }
        }
        placed
    }

    /// Placed first, so it always fits somewhere.
    fn place_multiplexor(&mut self, rng: &mut StdRng) -> Signal {
        loop {
            if let Some(signal) = self.try_place(rng, Multiplexing::Multiplexor) {
                return signal;
            }
        }
    }

    /// Each page starts from the occupancy the plain signals and multiplexor
    /// left, so pages overlap one another but never those.
    fn place_pages(&mut self, rng: &mut StdRng, multiplexor: &Signal) -> Vec<Signal> {
        let base = self.occupied.clone();
        let mut signals = Vec::new();
        for selector in random_selectors(rng, multiplexor.length) {
            self.occupied.clone_from(&base);
            let wanted = rng.random_range(1..=4);
            signals.extend(self.place_signals(rng, wanted, Multiplexing::Multiplexed(selector)));
        }
        signals
    }

    fn try_place(&mut self, rng: &mut StdRng, multiplexing: Multiplexing) -> Option<Signal> {
        let bits = self.occupied.len();
        let is_multiplexor = multiplexing == Multiplexing::Multiplexor;
        let byte_order = if rng.random_bool(0.5) {
            ByteOrder::Intel
        } else {
            ByteOrder::Motorola
        };
        let start = rng.random_range(0..bits);
        let room = match byte_order {
            ByteOrder::Intel => bits - start,
            ByteOrder::Motorola => (start % 8 + 1) + (bits - start / 8 * 8 - 8),
        };
        let max_length = if is_multiplexor { 8 } else { 64 };
        let length = random_length(rng, room.min(max_length));

        let positions = positions(start, length, byte_order);
        if positions.iter().any(|&p| self.occupied[p]) {
            return None;
        }
        for &p in &positions {
            self.occupied[p] = true;
        }

        let name = format!("S{}", self.next_name);
        self.next_name += 1;
        Some(Signal {
            name,
            start_bit: start as u16,
            length: length as u8,
            byte_order,
            value_type: if is_multiplexor || rng.random_bool(0.5) {
                ValueType::Unsigned
            } else {
                ValueType::Signed
            },
            factor: if is_multiplexor {
                1.0
            } else {
                random_scale(rng, FACTORS)
            },
            offset: if is_multiplexor {
                0.0
            } else {
                random_scale(rng, OFFSETS)
            },
            min: 0.0,
            max: 0.0,
            unit: String::new(),
            multiplexing,
        })
    }
}

/// Between one and four distinct page selectors a multiplexor of `bits`
/// bits can carry.
fn random_selectors(rng: &mut StdRng, bits: u8) -> Vec<u16> {
    let range = 1usize << bits;
    let count = rng.random_range(1..=4.min(range));
    rand::seq::index::sample(rng, range, count)
        .into_iter()
        .map(|selector| selector as u16)
        .collect()
}

/// Lengths biased towards the small widths real DBCs use, while still
/// reaching 64 often enough to exercise the wide paths.
fn random_length(rng: &mut StdRng, max: usize) -> usize {
    let cap = match rng.random_range(0..3) {
        0 => 8,
        1 => 16,
        _ => 64,
    };
    rng.random_range(1..=max.min(cap))
}

/// Mostly values from the table, sometimes an arbitrary short decimal.
fn random_scale(rng: &mut StdRng, table: &[f64]) -> f64 {
    if rng.random_bool(0.8) {
        *table.choose(rng).unwrap()
    } else {
        let mantissa = rng.random_range(-99_999..=99_999);
        let decimals = rng.random_range(0..=4);
        f64::from(mantissa) / 10f64.powi(decimals)
    }
}

/// Absolute bit positions a signal occupies, using the transposed-index
/// formulation rather than the decoder's walk.
fn positions(start: usize, length: usize, byte_order: ByteOrder) -> Vec<usize> {
    match byte_order {
        ByteOrder::Intel => (start..start + length).collect(),
        ByteOrder::Motorola => {
            let msb_first = |pos: usize| (pos / 8) * 8 + (7 - pos % 8);
            let first = msb_first(start);
            (first..first + length).map(msb_first).collect()
        }
    }
}

/// A random payload, usually steered onto one of the message's pages so the
/// multiplexed signals actually get exercised.
fn random_frame(rng: &mut StdRng, message: &Message) -> Vec<u8> {
    let mut payload = random_payload(rng, usize::from(message.dlc));
    let Some(multiplexor) = message.multiplexor() else {
        return payload;
    };
    if rng.random_bool(CLAIMED_SELECTOR_RATE) {
        let claimed: Vec<u16> = message
            .signals
            .iter()
            .filter_map(|s| match s.multiplexing {
                Multiplexing::Multiplexed(n) => Some(n),
                _ => None,
            })
            .collect();
        let selector = *claimed
            .choose(rng)
            .expect("every multiplexed message has a page");
        insert_raw(&mut payload, multiplexor, u64::from(selector)).unwrap();
    }
    payload
}

fn random_payload(rng: &mut StdRng, len: usize) -> Vec<u8> {
    let mut payload = vec![0u8; len];
    match rng.random_range(0..10) {
        0 => payload.fill(0xFF),
        1 => {}
        2 => payload.fill(0x80),
        _ => rng.fill(&mut payload[..]),
    }
    payload
}

fn render_message(message: &Message) -> String {
    let dbc_id = match message.id {
        CanId::Standard(id) => u32::from(id),
        CanId::Extended(id) => id | 0x8000_0000,
    };
    let mut out = format!(
        "BO_ {dbc_id} {}: {} {}\n",
        message.name, message.dlc, message.sender
    );
    for s in &message.signals {
        let order = match s.byte_order {
            ByteOrder::Intel => 1,
            ByteOrder::Motorola => 0,
        };
        let sign = match s.value_type {
            ValueType::Unsigned => '+',
            ValueType::Signed => '-',
        };
        writeln!(
            out,
            " SG_ {} {}: {}|{}@{order}{sign} ({},{}) [0|0] \"\" Node",
            s.name,
            multiplexing_token(s),
            s.start_bit,
            s.length,
            s.factor,
            s.offset
        )
        .unwrap();
    }
    out.push('\n');
    out
}

/// The DBC token for a signal's multiplexing role, with a trailing space
/// when there is one.
fn multiplexing_token(s: &Signal) -> String {
    match s.multiplexing {
        Multiplexing::None => String::new(),
        Multiplexing::Multiplexor => "M ".into(),
        Multiplexing::Multiplexed(n) => format!("m{n} "),
    }
}

// ------------------------------------------------------------- utilities

fn manifest_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn workspace_dir(seed: u64) -> PathBuf {
    manifest_dir()
        .join("target/differential")
        .join(seed.to_string())
}

fn env_number<T: std::str::FromStr>(name: &str) -> Option<T> {
    std::env::var(name).ok().and_then(|v| v.parse().ok())
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn unhex(text: &str) -> Vec<u8> {
    (0..text.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&text[i..i + 2], 16).unwrap())
        .collect()
}
