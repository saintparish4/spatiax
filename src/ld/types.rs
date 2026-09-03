//! The shape of a `.ld` session, before any of it is bytes.
//!
//! MoTeC i2 reads a fixed-rate file: a channel is an array of samples at one
//! frequency, and there is no per-sample timestamp anywhere in the format.
//! So a [`Session`] is a start time, one rate, and a set of equal-length
//! channels — the shape the writer serialises and the shape the sampler has
//! to produce out of irregular CAN traffic.
//!
//! Nothing here does I/O or looks at a CAN frame.
//!
//! Names and units are fixed-width byte arrays in the file, and the readers
//! that exist decode them as ASCII, so text that would not survive that
//! round trip is cut here. I would rather a unit read `C` than come back
//! from a reader as the empty string.

/// Longest channel name the format stores, in bytes.
pub const NAME_LEN: usize = 32;

/// Longest abbreviated channel name, in bytes.
pub const SHORT_NAME_LEN: usize = 8;

/// Longest unit string, in bytes.
pub const UNIT_LEN: usize = 12;

/// One channel: what i2 lists in its channel tree, and one sample per grid
/// point.
#[derive(Debug, Clone, PartialEq)]
pub struct Channel {
    /// Full name, at most [`NAME_LEN`] bytes.
    pub name: String,
    /// Abbreviated name, at most [`SHORT_NAME_LEN`] bytes.
    pub short_name: String,
    /// Physical unit as the DBC spells it, at most [`UNIT_LEN`] bytes.
    pub unit: String,
    /// One value per grid point. Every channel in a session carries the
    /// same number of these.
    pub samples: Vec<f32>,
}

impl Channel {
    /// Build a channel, cutting `name` and `unit` to what the format holds.
    pub fn new(name: &str, unit: &str, samples: Vec<f32>) -> Self {
        let name = field(name, NAME_LEN);
        let short_name = field(&name, SHORT_NAME_LEN);
        Self {
            name,
            short_name,
            unit: field(unit, UNIT_LEN),
            samples,
        }
    }
}

/// A recording: when it started, how fast it was sampled, and the channels
/// in it.
///
/// Every channel must carry the same number of samples. [`Session::push`]
/// does not enforce that — the writer reports it, because that is the point
/// at which an unequal channel would produce a file no reader could make
/// sense of.
#[derive(Debug, Clone)]
pub struct Session {
    /// Start of the grid, in microseconds since the Unix epoch.
    pub started_at_us: u64,
    /// Sample rate of every channel, in Hz.
    pub rate_hz: u16,
    /// Driver name, as i2 shows it.
    pub driver: String,
    /// Vehicle identifier, as i2 shows it.
    pub vehicle: String,
    /// Venue name, as i2 shows it.
    pub venue: String,
    /// Free text; lands in both the event block and the header comment.
    pub event: String,
    channels: Vec<Channel>,
}

impl Session {
    /// An empty session starting at `started_at_us`, sampled at `rate_hz`.
    pub fn new(started_at_us: u64, rate_hz: u16) -> Self {
        Self {
            started_at_us,
            rate_hz,
            driver: String::new(),
            vehicle: String::new(),
            venue: String::new(),
            event: String::new(),
            channels: Vec::new(),
        }
    }

    /// The channels, in the order they will be written.
    pub fn channels(&self) -> &[Channel] {
        &self.channels
    }

    /// Samples per channel, or 0 for a session with no channels.
    pub fn sample_count(&self) -> usize {
        self.channels.first().map_or(0, |c| c.samples.len())
    }

    /// Add a channel, renaming it if the session already has that name.
    ///
    /// i2 keys its channel tree on the name, so two channels called
    /// `WheelSpeed` would show up as one channel holding whichever data was
    /// read last. DBC signal names are unique per message, not per
    /// database, so this is reachable with an ordinary file.
    pub fn push(&mut self, mut channel: Channel) {
        if self.taken(&channel.name) {
            let mut n = 2;
            while self.taken(&numbered(&channel.name, n)) {
                n += 1;
            }
            channel.name = numbered(&channel.name, n);
            channel.short_name = field(&channel.name, SHORT_NAME_LEN);
        }
        self.channels.push(channel);
    }

    fn taken(&self, name: &str) -> bool {
        self.channels.iter().any(|c| c.name == name)
    }
}

/// The bytes of `text` a reader will give back unchanged: ASCII graphic
/// characters and spaces, at most `limit` of them. Every retained character
/// is one byte, so the count is also the byte count.
fn field(text: &str, limit: usize) -> String {
    text.chars()
        .filter(|c| c.is_ascii_graphic() || *c == ' ')
        .take(limit)
        .collect::<String>()
        .trim()
        .to_string()
}

/// `name` with `_n` appended, shortened so the result still fits the name
/// field.
fn numbered(name: &str, n: u32) -> String {
    let suffix = format!("_{n}");
    let room = NAME_LEN.saturating_sub(suffix.len());
    format!("{}{suffix}", field(name, room))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_long_name_is_cut_to_the_field_the_format_provides() {
        let channel = Channel::new(&"N".repeat(40), "rpm", vec![1.0]);
        assert_eq!(channel.name.len(), NAME_LEN);
        assert_eq!(channel.short_name.len(), SHORT_NAME_LEN);
    }

    #[test]
    fn the_short_name_is_the_head_of_the_full_name() {
        let channel = Channel::new("EngineSpeed", "rpm", vec![]);
        assert_eq!(channel.name, "EngineSpeed");
        assert_eq!(channel.short_name, "EngineSp");
    }

    #[test]
    fn a_unit_the_readers_cannot_decode_loses_the_bytes_they_would_reject() {
        assert_eq!(Channel::new("Coolant", "°C", vec![]).unit, "C");
    }

    #[test]
    fn a_repeated_name_is_numbered_rather_than_written_twice() {
        let mut session = Session::new(0, 50);
        session.push(Channel::new("WheelSpeed", "km/h", vec![1.0]));
        session.push(Channel::new("WheelSpeed", "km/h", vec![2.0]));
        session.push(Channel::new("WheelSpeed", "km/h", vec![3.0]));
        let names: Vec<_> = session.channels().iter().map(|c| c.name.as_str()).collect();
        assert_eq!(names, ["WheelSpeed", "WheelSpeed_2", "WheelSpeed_3"]);
    }

    #[test]
    fn numbering_a_name_that_already_fills_the_field_keeps_it_within_it() {
        let long = "N".repeat(NAME_LEN);
        let mut session = Session::new(0, 50);
        session.push(Channel::new(&long, "", vec![]));
        session.push(Channel::new(&long, "", vec![]));
        assert!(session.channels()[1].name.len() <= NAME_LEN);
        assert!(session.channels()[1].name.ends_with("_2"));
    }

    #[test]
    fn the_sample_count_is_the_length_every_channel_must_share() {
        let mut session = Session::new(0, 10);
        assert_eq!(session.sample_count(), 0);
        session.push(Channel::new("A", "", vec![1.0, 2.0, 3.0]));
        assert_eq!(session.sample_count(), 3);
    }
}
