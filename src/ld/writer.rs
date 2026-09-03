//! The byte layout of a `.ld` file.
//!
//! MoTeC has never published this format. The offsets and magic numbers
//! below come from the community reverse-engineering effort, and were
//! confirmed by writing a file and reading it back with an implementation
//! nobody here wrote. Each field is placed at a literal offset in a zeroed
//! buffer rather than appended in sequence, because the format is mostly
//! padding and an appended layout hides a miscounted pad byte until a
//! reader is confused by it.
//!
//! A file is a header, an event block, one header per channel chained by
//! pointers, and then every channel's samples end to end. Samples are `f32`
//! with the calibration fields left at identity, so the stored word is the
//! physical value and no scaling happens on the way in or out.
//!
//! Header fields, little-endian: marker at 0, first channel header at 8,
//! first samples at 12, event block at 36, device identity at 64–85,
//! channel count at 86, date at 94, time at 126, driver at 158, vehicle at
//! 222, venue at 350, the logging magic at 1502, and the comment at 1572.
//! A channel header is previous at 0, next at 4, samples at 8, count at 12,
//! counter at 16, type at 18 and 20, rate at 22, calibration at 24–31, name
//! at 32, short name at 64, and unit at 72.

use std::io::Write;

use crate::error::{Error, Result};
use crate::ld::types::Session;

const HEAD_LEN: usize = 1762;
const EVENT_LEN: usize = 1154;
const CHAN_LEN: usize = 124;

const EVENT_PTR: u32 = HEAD_LEN as u32;
const META_PTR: u32 = (HEAD_LEN + EVENT_LEN) as u32;

const MARKER: u32 = 0x40;
const DEVICE_SERIAL: u32 = 0x1f44;
const DEVICE_TYPE: &str = "ADL";
const DEVICE_VERSION: u16 = 420;
const PRO_LOGGING: u32 = 0x000c_81a4;
const COUNTER_BASE: u16 = 0x2ee1;

/// Type class and width for `f32` samples.
const FLOAT32: (u16, u16) = (0x07, 4);

/// Write `session` as a `.ld` file.
///
/// Fails if the channels do not all carry the same number of samples: a
/// reader takes each channel's count from its own header and would read
/// another channel's samples as its own.
pub fn write<W: Write>(session: &Session, mut out: W) -> Result<()> {
    let channels = session.channels();
    let len = session.sample_count();
    if let Some(odd) = channels.iter().find(|c| c.samples.len() != len) {
        return Err(Error::Export {
            reason: format!(
                "channel `{}` carries {} samples but the session has {len}",
                odd.name,
                odd.samples.len()
            ),
        });
    }
    let count = u32::try_from(channels.len()).map_err(|_| Error::Export {
        reason: format!("{} channels is more than the format counts", channels.len()),
    })?;
    let data_ptr = META_PTR + count * CHAN_LEN as u32;
    let (date, time) = date_and_time(session.started_at_us);

    let mut head = [0u8; HEAD_LEN];
    put_u32(&mut head, 0, MARKER);
    put_u32(&mut head, 8, META_PTR);
    put_u32(&mut head, 12, data_ptr);
    put_u32(&mut head, 36, EVENT_PTR);
    put_u16(&mut head, 64, 1);
    put_u16(&mut head, 66, 0x4240);
    put_u16(&mut head, 68, 0x000f);
    put_u32(&mut head, 70, DEVICE_SERIAL);
    put_str(&mut head, 74, 8, DEVICE_TYPE);
    put_u16(&mut head, 82, DEVICE_VERSION);
    put_u16(&mut head, 84, 0xadb0);
    put_u32(&mut head, 86, count);
    put_str(&mut head, 94, 16, &date);
    put_str(&mut head, 126, 16, &time);
    put_str(&mut head, 158, 64, &session.driver);
    put_str(&mut head, 222, 64, &session.vehicle);
    put_str(&mut head, 350, 64, &session.venue);
    put_u32(&mut head, 1502, PRO_LOGGING);
    put_str(&mut head, 1572, 64, &session.event);
    out.write_all(&head)?;

    let mut event = [0u8; EVENT_LEN];
    put_str(&mut event, 0, 64, &session.event);
    put_str(&mut event, 64, 64, "1");
    out.write_all(&event)?;

    let mut samples_at = data_ptr;
    for (index, channel) in channels.iter().enumerate() {
        let position = META_PTR + index as u32 * CHAN_LEN as u32;
        let mut header = [0u8; CHAN_LEN];
        put_u32(
            &mut header,
            0,
            if index > 0 {
                position - CHAN_LEN as u32
            } else {
                0
            },
        );
        put_u32(
            &mut header,
            4,
            if index + 1 < channels.len() {
                position + CHAN_LEN as u32
            } else {
                0
            },
        );
        put_u32(&mut header, 8, samples_at);
        put_u32(&mut header, 12, len as u32);
        put_u16(&mut header, 16, COUNTER_BASE.wrapping_add(index as u16));
        put_u16(&mut header, 18, FLOAT32.0);
        put_u16(&mut header, 20, FLOAT32.1);
        put_u16(&mut header, 22, session.rate_hz);
        put_i16(&mut header, 24, 0);
        put_i16(&mut header, 26, 1);
        put_i16(&mut header, 28, 1);
        put_i16(&mut header, 30, 0);
        put_str(&mut header, 32, 32, &channel.name);
        put_str(&mut header, 64, 8, &channel.short_name);
        put_str(&mut header, 72, 12, &channel.unit);
        out.write_all(&header)?;
        samples_at += len as u32 * 4;
    }

    for channel in channels {
        for sample in &channel.samples {
            out.write_all(&sample.to_le_bytes())?;
        }
    }
    Ok(())
}

fn put_u32(buf: &mut [u8], at: usize, value: u32) {
    buf[at..at + 4].copy_from_slice(&value.to_le_bytes());
}

fn put_u16(buf: &mut [u8], at: usize, value: u16) {
    buf[at..at + 2].copy_from_slice(&value.to_le_bytes());
}

fn put_i16(buf: &mut [u8], at: usize, value: i16) {
    buf[at..at + 2].copy_from_slice(&value.to_le_bytes());
}

/// Left-aligned, zero-padded, never longer than the field. The buffer is
/// already zeroed, so short text is terminated by what is already there.
fn put_str(buf: &mut [u8], at: usize, len: usize, text: &str) {
    let bytes = text.as_bytes();
    let take = bytes.len().min(len);
    buf[at..at + take].copy_from_slice(&bytes[..take]);
}

/// `DD/MM/YYYY` and `HH:MM:SS`, in UTC, from microseconds since the epoch.
fn date_and_time(started_at_us: u64) -> (String, String) {
    let seconds = started_at_us / 1_000_000;
    let (year, month, day) = civil_from_days(seconds / 86_400);
    let day_seconds = seconds % 86_400;
    (
        format!("{day:02}/{month:02}/{year:04}"),
        format!(
            "{:02}:{:02}:{:02}",
            day_seconds / 3600,
            day_seconds / 60 % 60,
            day_seconds % 60
        ),
    )
}

/// Gregorian year, month and day from a count of days since 1970-01-01.
///
/// Hinnant's `civil_from_days`, restricted to the non-negative half — a
/// Unix timestamp cannot land before the epoch, so the negative branch of
/// the published algorithm is unreachable here and is left out rather than
/// written untested. The era arithmetic is what makes 2100 a common year.
fn civil_from_days(days: u64) -> (u64, u64, u64) {
    let z = days + 719_468;
    let era = z / 146_097;
    let day_of_era = z - era * 146_097;
    let year_of_era =
        (day_of_era - day_of_era / 1_460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let year = year_of_era + era * 400;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let shifted_month = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * shifted_month + 2) / 5 + 1;
    let month = if shifted_month < 10 {
        shifted_month + 3
    } else {
        shifted_month - 9
    };
    (if month <= 2 { year + 1 } else { year }, month, day)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ld::types::Channel;

    fn session() -> Session {
        let mut session = Session::new(1_756_900_800_000_000, 20);
        session.driver = "driver".to_string();
        session.push(Channel::new(
            "EngineSpeed",
            "rpm",
            vec![0.0, 1000.0, 2000.0],
        ));
        session.push(Channel::new("WheelSpeedFL", "km/h", vec![0.0, 10.5, 21.0]));
        session
    }

    fn written() -> Vec<u8> {
        let mut buf = Vec::new();
        write(&session(), &mut buf).expect("a session with equal channels writes");
        buf
    }

    fn u32_at(buf: &[u8], at: usize) -> u32 {
        u32::from_le_bytes(buf[at..at + 4].try_into().unwrap())
    }

    #[test]
    fn the_file_is_header_event_channel_headers_and_samples_with_nothing_over() {
        assert_eq!(
            written().len(),
            HEAD_LEN + EVENT_LEN + 2 * CHAN_LEN + 2 * 3 * 4
        );
    }

    #[test]
    fn the_header_carries_the_marker_the_readers_look_for() {
        assert_eq!(u32_at(&written(), 0), MARKER);
        assert_eq!(u32_at(&written(), 1502), PRO_LOGGING);
    }

    #[test]
    fn the_header_points_at_the_first_channel_and_the_first_samples() {
        let buf = written();
        assert_eq!(u32_at(&buf, 8), META_PTR);
        assert_eq!(u32_at(&buf, 12), META_PTR + 2 * CHAN_LEN as u32);
        assert_eq!(u32_at(&buf, 36), EVENT_PTR);
        assert_eq!(u32_at(&buf, 86), 2);
    }

    #[test]
    fn the_channel_headers_chain_forwards_and_backwards_and_stop() {
        let buf = written();
        let first = META_PTR as usize;
        let second = first + CHAN_LEN;
        assert_eq!(u32_at(&buf, first), 0);
        assert_eq!(u32_at(&buf, first + 4), META_PTR + CHAN_LEN as u32);
        assert_eq!(u32_at(&buf, second), META_PTR);
        assert_eq!(u32_at(&buf, second + 4), 0);
    }

    #[test]
    fn each_channel_points_at_its_own_samples() {
        let buf = written();
        let data = META_PTR + 2 * CHAN_LEN as u32;
        assert_eq!(u32_at(&buf, META_PTR as usize + 8), data);
        assert_eq!(u32_at(&buf, META_PTR as usize + CHAN_LEN + 8), data + 3 * 4);
    }

    #[test]
    fn samples_are_written_as_little_endian_floats_in_channel_order() {
        let buf = written();
        let data = (META_PTR + 2 * CHAN_LEN as u32) as usize;
        let first = f32::from_le_bytes(buf[data..data + 4].try_into().unwrap());
        let second = f32::from_le_bytes(buf[data + 12..data + 16].try_into().unwrap());
        assert_eq!(first, 0.0);
        assert_eq!(second, 0.0);
        let mid = f32::from_le_bytes(buf[data + 16..data + 20].try_into().unwrap());
        assert_eq!(mid, 10.5);
    }

    #[test]
    fn channels_of_different_lengths_are_refused_rather_than_written() {
        let mut session = session();
        session.push(Channel::new("Odd", "", vec![1.0]));
        let mut buf = Vec::new();
        assert!(write(&session, &mut buf).is_err());
    }

    #[test]
    fn the_epoch_and_a_century_that_is_not_a_leap_year_convert_correctly() {
        assert_eq!(
            date_and_time(0),
            ("01/01/1970".to_string(), "00:00:00".to_string())
        );
        assert_eq!(
            date_and_time(1_000_000_000_000_000),
            ("09/09/2001".to_string(), "01:46:40".to_string())
        );
        assert_eq!(
            date_and_time(1_756_900_800_000_000),
            ("03/09/2025".to_string(), "12:00:00".to_string())
        );
        assert_eq!(
            date_and_time(4_102_444_800_000_000),
            ("01/01/2100".to_string(), "00:00:00".to_string())
        );
    }
}
