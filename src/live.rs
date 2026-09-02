//! Live capture from a SocketCAN interface. Linux only, behind the
//! `socketcan` feature.
//!
//! Frames carry the kernel's receive time — the same clock `candump` logs —
//! so decoding a bus live and replaying a log of it give the same
//! timestamps. The socket is opened FD-capable, which still receives classic
//! frames, so one reader covers both.

use std::time::{SystemTime, UNIX_EPOCH};

use socketcan::{CanAnyFrame, CanFdSocket, EmbeddedFrame, Id, Socket, SocketOptions};

use crate::error::Result;
use crate::frame::{CanFrame, CanId};

/// Data frames arriving on one interface, in receive order.
pub struct Capture {
    socket: CanFdSocket,
}

impl Capture {
    /// Bind to an interface such as `can0` or `vcan0`.
    pub fn open(iface: &str) -> Result<Self> {
        let socket = CanFdSocket::open(iface)?;
        socket.set_recv_timestamp(true)?;
        Ok(Self { socket })
    }

    /// Block until the next data frame arrives.
    pub fn read(&mut self) -> Result<CanFrame> {
        loop {
            let (frame, received) = self.socket.read_frame_with_timestamp()?;
            if let Some(frame) = data_frame(&frame, received) {
                return frame;
            }
        }
    }
}

impl Iterator for Capture {
    type Item = Result<CanFrame>;

    /// Never `None`: a bus has no end, so stopping is the caller's decision.
    fn next(&mut self) -> Option<Self::Item> {
        Some(self.read())
    }
}

/// Convert a received frame. `None` for remote and error frames, which
/// carry no signals.
pub fn data_frame(frame: &CanAnyFrame, received: SystemTime) -> Option<Result<CanFrame>> {
    match frame {
        CanAnyFrame::Normal(_) | CanAnyFrame::Fd(_) => {}
        CanAnyFrame::Remote(_) | CanAnyFrame::Error(_) => return None,
    }
    Some(CanFrame::new(
        can_id(frame.id()),
        frame.data(),
        micros_since_epoch(received),
    ))
}

fn can_id(id: Id) -> CanId {
    match id {
        Id::Standard(id) => CanId::Standard(id.as_raw()),
        Id::Extended(id) => CanId::Extended(id.as_raw()),
    }
}

/// A clock set before 1970 reads as zero rather than failing the frame.
fn micros_since_epoch(at: SystemTime) -> u64 {
    at.duration_since(UNIX_EPOCH).map_or(0, |since| {
        u64::try_from(since.as_micros()).unwrap_or(u64::MAX)
    })
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use socketcan::{
        CanDataFrame, CanErrorFrame, CanFdFrame, CanRemoteFrame, ExtendedId, StandardId,
    };

    use super::*;

    fn at(micros: u64) -> SystemTime {
        UNIX_EPOCH + Duration::from_micros(micros)
    }

    #[test]
    fn a_classic_data_frame_keeps_its_identifier_payload_and_receive_time() {
        let id = StandardId::new(0x100).unwrap();
        let frame = CanAnyFrame::Normal(CanDataFrame::new(id, &[0x34, 0x12, 0x64]).unwrap());
        let converted = data_frame(&frame, at(1_700_000_000_000_250))
            .unwrap()
            .unwrap();
        assert_eq!(converted.id(), CanId::Standard(0x100));
        assert_eq!(converted.data(), [0x34, 0x12, 0x64]);
        assert_eq!(converted.timestamp_us, 1_700_000_000_000_250);
    }

    #[test]
    fn an_fd_frame_with_an_extended_identifier_carries_all_its_bytes() {
        let id = ExtendedId::new(0x18FE_EE00).unwrap();
        let frame = CanAnyFrame::Fd(CanFdFrame::new(id, &[0xAB; 12]).unwrap());
        let converted = data_frame(&frame, at(0)).unwrap().unwrap();
        assert_eq!(converted.id(), CanId::Extended(0x18FE_EE00));
        assert_eq!(converted.len(), 12);
        assert_eq!(converted.data(), [0xAB; 12]);
    }

    #[test]
    fn remote_and_error_frames_are_dropped() {
        let id = StandardId::new(0x100).unwrap();
        let remote = CanAnyFrame::Remote(CanRemoteFrame::new_remote(id, 4).unwrap());
        assert!(data_frame(&remote, at(0)).is_none());
        let error = CanAnyFrame::Error(CanErrorFrame::new_error(0x2000_0004, &[0; 8]).unwrap());
        assert!(data_frame(&error, at(0)).is_none());
    }

    #[test]
    fn a_receive_time_before_the_epoch_reads_as_zero() {
        let id = StandardId::new(1).unwrap();
        let frame = CanAnyFrame::Normal(CanDataFrame::new(id, &[]).unwrap());
        let before = UNIX_EPOCH - Duration::from_secs(1);
        assert_eq!(data_frame(&frame, before).unwrap().unwrap().timestamp_us, 0);
    }
}
