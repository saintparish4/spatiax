//! Irregular CAN traffic onto the fixed grid a `.ld` file requires.
//!
//! A signal updates when its message arrives; a channel is an array sampled
//! at one rate. I hold the last value received until the next one arrives,
//! so the trace steps exactly where the bus stepped. Interpolating would
//! draw values no ECU ever sent, which is the one thing a decoder whose
//! whole claim is fidelity must not do in its last mile.
//!
//! Before a channel's first update there is nothing to hold, so its first
//! value is carried backwards to the start of the grid. A leading zero
//! would draw a trace descending from zero that never happened.
//!
//! Signals a frame was too short to carry are counted, not guessed at: a
//! caller that wants to know how much of the log it lost gets a number.

use std::collections::HashMap;

use crate::dbc::Database;
use crate::error::{Error, Result};
use crate::frame::CanFrame;
use crate::ld::types::{Channel, Session};

/// Rates a logger is plausibly set to. The default rate is the first of
/// these at or above the fastest message on the bus, so a 50 Hz message is
/// never sampled at 47 Hz and never padded up to 1 kHz.
pub const RATE_LADDER: &[u16] = &[1, 2, 5, 10, 20, 25, 50, 100, 200, 500, 1000];

/// Refuse to build a session whose samples would exceed this. A long log at
/// a high rate is a plausible mistake, and the failure without this guard
/// is the allocator's, not ours.
const MAX_SAMPLE_BYTES: usize = 512 * 1024 * 1024;

/// A resampled session, and what could not be used to build it.
#[derive(Debug)]
pub struct Sampled {
    /// The session, ready to write.
    pub session: Session,
    /// Signals a frame was too short to carry. Frames whose identifier is
    /// not in the database are not counted here — on a real bus most
    /// traffic is not described by any one DBC.
    pub skipped: usize,
}

/// Decode `frames` against `db` and resample every signal onto one grid.
///
/// `rate_hz` of `None` takes the rate from the log. The grid starts at the
/// earliest frame and ends at the latest, so an empty log has no grid and
/// is an error rather than an empty file.
pub fn sample(db: &Database, frames: &[CanFrame], rate_hz: Option<u16>) -> Result<Sampled> {
    let (start, end) = span(frames)?;
    let rate = match rate_hz {
        Some(0) => return Err(export("a sample rate of 0 Hz describes no grid")),
        Some(rate) => rate,
        None => default_rate(frames, end - start),
    };
    let len = ((end - start) * u64::from(rate) / 1_000_000) as usize + 1;

    let layout = Layout::of(db);
    let mut updates: Vec<Vec<(u64, f32)>> = vec![Vec::new(); layout.channels.len()];
    let mut skipped = 0;
    for frame in frames {
        let Some(decoded) = db.decode_frame(frame) else {
            continue;
        };
        for result in decoded {
            match result {
                Ok(d) => {
                    if let Some(slot) = layout.slot(frame.id().raw(), &d.signal.name) {
                        updates[slot].push((frame.timestamp_us, d.value as f32));
                    }
                }
                Err(_) => skipped += 1,
            }
        }
    }

    let carried = updates.iter().filter(|u| !u.is_empty()).count();
    if carried == 0 {
        return Err(export(
            "no signal in the database was decoded from this log",
        ));
    }
    let bytes = carried.saturating_mul(len).saturating_mul(4);
    if bytes > MAX_SAMPLE_BYTES {
        return Err(export(format!(
            "{carried} channels at {rate} Hz over this log need {bytes} bytes of samples; \
             pass a lower rate"
        )));
    }

    let mut session = Session::new(start, rate);
    for (slot, (name, unit)) in layout.channels.iter().copied().enumerate() {
        if updates[slot].is_empty() {
            continue;
        }
        // A merged log can hold frames out of order; the walk below reads
        // updates as sorted, so make them so rather than assume it.
        updates[slot].sort_by_key(|(at, _)| *at);
        let samples = resample(&updates[slot], start, rate, len);
        session.push(Channel::new(name, unit, samples));
    }
    Ok(Sampled { session, skipped })
}

/// Which channel each of a database's signals will be written as.
struct Layout<'a> {
    /// Name and unit per channel, in the order they will be written.
    channels: Vec<(&'a str, &'a str)>,
    /// Identifier, then signal name, to a position in `channels`.
    slots: HashMap<u32, HashMap<&'a str, usize>>,
}

impl<'a> Layout<'a> {
    /// Every signal in the database, ordered by identifier and then by the
    /// order the DBC declares them.
    ///
    /// `Database::messages()` is `HashMap`-ordered, so it is sorted here
    /// for the same reason `dbc::check` sorts: the output must not depend
    /// on the hasher.
    fn of(db: &'a Database) -> Self {
        let mut messages: Vec<_> = db.messages().collect();
        messages.sort_by_key(|m| m.id.raw());

        let mut channels = Vec::new();
        let mut slots: HashMap<u32, HashMap<&str, usize>> = HashMap::new();
        for message in messages {
            let by_name = slots.entry(message.id.raw()).or_default();
            for signal in &message.signals {
                by_name.insert(signal.name.as_str(), channels.len());
                channels.push((signal.name.as_str(), signal.unit.as_str()));
            }
        }
        Self { channels, slots }
    }

    fn slot(&self, id: u32, signal: &str) -> Option<usize> {
        self.slots.get(&id)?.get(signal).copied()
    }
}

/// Hold each update until the next one falls due, and carry the first value
/// back to the start of the grid.
fn resample(updates: &[(u64, f32)], start: u64, rate: u16, len: usize) -> Vec<f32> {
    let mut samples = Vec::with_capacity(len);
    let mut held = updates[0].1;
    let mut next = 0;
    for k in 0..len {
        let at = start + k as u64 * 1_000_000 / u64::from(rate);
        while next < updates.len() && updates[next].0 <= at {
            held = updates[next].1;
            next += 1;
        }
        samples.push(held);
    }
    samples
}

/// First and last timestamp in the log.
fn span(frames: &[CanFrame]) -> Result<(u64, u64)> {
    if frames.is_empty() {
        return Err(export("the log holds no frames"));
    }
    let start = frames.iter().map(|f| f.timestamp_us).min().unwrap_or(0);
    let end = frames.iter().map(|f| f.timestamp_us).max().unwrap_or(0);
    Ok((start, end))
}

/// The ladder step at or above the fastest message in the log.
fn default_rate(frames: &[CanFrame], span_us: u64) -> u16 {
    let last = *RATE_LADDER.last().expect("the ladder is not empty");
    if span_us == 0 {
        return RATE_LADDER[0];
    }
    let mut counts: HashMap<u32, usize> = HashMap::new();
    for frame in frames {
        *counts.entry(frame.id().raw()).or_default() += 1;
    }
    let seconds = span_us as f64 / 1_000_000.0;
    let fastest = counts
        .values()
        .map(|&n| (n as f64 - 1.0) / seconds)
        .fold(0.0, f64::max);
    RATE_LADDER
        .iter()
        .copied()
        .find(|&rate| f64::from(rate) >= fastest)
        .unwrap_or(last)
}

fn export(reason: impl Into<String>) -> Error {
    Error::Export {
        reason: reason.into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dbc;
    use crate::frame::CanId;

    fn database() -> Database {
        dbc::parse(
            "BO_ 256 EngineData: 8 ECU\n \
             SG_ EngineRPM : 0|16@1+ (1,0) [0|65535] \"rpm\" DASH\n",
        )
        .expect("fixture parses")
    }

    fn frame(rpm: u16, at_us: u64) -> CanFrame {
        let bytes = rpm.to_le_bytes();
        CanFrame::new(CanId::Standard(256), &bytes, at_us).expect("fits")
    }

    #[test]
    fn a_signal_holds_its_last_value_until_the_next_frame() {
        let db = database();
        let frames = [frame(100, 0), frame(200, 500_000)];
        let sampled = sample(&db, &frames, Some(10)).expect("one channel");
        let samples = &sampled.session.channels()[0].samples;
        // 10 Hz over half a second: six grid points, stepping at 0.5 s.
        assert_eq!(samples, &[100.0, 100.0, 100.0, 100.0, 100.0, 200.0]);
    }

    #[test]
    fn a_channel_starts_at_its_first_value_rather_than_zero() {
        let db = database();
        let frames = [frame(1, 0), frame(700, 900_000)];
        let sampled = sample(&db, &frames, Some(2)).expect("one channel");
        assert_eq!(sampled.session.channels()[0].samples[0], 1.0);
    }

    #[test]
    fn the_grid_spans_the_log_from_first_frame_to_last() {
        let db = database();
        let frames = [frame(1, 1_000_000), frame(2, 3_000_000)];
        let sampled = sample(&db, &frames, Some(1)).expect("one channel");
        assert_eq!(sampled.session.started_at_us, 1_000_000);
        assert_eq!(sampled.session.sample_count(), 3);
    }

    #[test]
    fn the_default_rate_is_the_ladder_step_at_or_above_the_fastest_message() {
        // 41 frames over one second is 40 Hz, which the ladder rounds to 50.
        let frames: Vec<_> = (0..41).map(|k| frame(0, k * 25_000)).collect();
        assert_eq!(default_rate(&frames, 1_000_000), 50);
    }

    #[test]
    fn a_log_with_no_frames_is_an_error() {
        assert!(sample(&database(), &[], Some(10)).is_err());
    }

    #[test]
    fn a_signal_no_frame_carried_gets_no_channel() {
        let db = dbc::parse(
            "BO_ 256 EngineData: 8 ECU\n \
             SG_ EngineRPM : 0|16@1+ (1,0) [0|65535] \"rpm\" DASH\n\
             BO_ 512 Absent: 8 ECU\n \
             SG_ NeverSeen : 0|8@1+ (1,0) [0|255] \"\" DASH\n",
        )
        .expect("fixture parses");
        let sampled = sample(&db, &[frame(1, 0)], Some(1)).expect("one channel");
        assert_eq!(sampled.session.channels().len(), 1);
        assert_eq!(sampled.session.channels()[0].name, "EngineRPM");
    }

    #[test]
    fn a_frame_too_short_for_its_signals_is_counted_not_guessed() {
        let db = database();
        let short = CanFrame::new(CanId::Standard(256), &[0x01], 0).expect("fits");
        let frames = [frame(5, 0), short];
        let sampled = sample(&db, &frames, Some(1)).expect("one channel");
        assert_eq!(sampled.skipped, 1);
    }
}
