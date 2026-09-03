//! CAN frame and identifier types.
//!
//! The payload is a fixed `[u8; 64]` plus a length rather than a `Vec`. CAN FD
//! caps a payload at 64 bytes, so the maximum is known at compile time, and
//! avoiding one heap allocation per frame is what keeps the decode path
//! allocation-free. `CanFrame` is `Copy` as a result.
//!
//! DBC marks a 29-bit extended identifier by setting bit 31 of the message ID.
//! [`CanId::from_dbc`] is the only place in this crate that interprets that
//! flag. If you find yourself masking `0x1FFF_FFFF` anywhere else, that is a
//! bug.

use crate::error::{Error, Result};

/// Maximum CAN FD payload, in bytes. Classic CAN uses at most 8 of these.
pub const MAX_FRAME_LEN: usize = 64;

/// The payload length each of the sixteen data length codes stands for.
const FD_PAYLOAD_LEN: [u8; 16] = [0, 1, 2, 3, 4, 5, 6, 7, 8, 12, 16, 20, 24, 32, 48, 64];

/// The lowest classic CAN code that says more than the eight bytes it carries.
const FIRST_LEN8_DLC: u8 = 9;

const DBC_EXTENDED_FLAG: u32 = 0x8000_0000;
const EXTENDED_ID_MASK: u32 = 0x1FFF_FFFF;
const STANDARD_ID_MAX: u32 = 0x7FF;

/// The payload length a data length code stands for.
///
/// The four-bit code is the length itself up to eight bytes. Above that CAN
/// FD packs its seven remaining sizes — 12, 16, 20, 24, 32, 48 and 64 bytes —
/// into the codes 9 to 15, which is why a code cannot simply be read as a
/// byte count. `None` for a value too large to be a code.
pub const fn dlc_to_len(dlc: u8) -> Option<usize> {
    if (dlc as usize) < FD_PAYLOAD_LEN.len() {
        Some(FD_PAYLOAD_LEN[dlc as usize] as usize)
    } else {
        None
    }
}

/// The data length code for a payload of exactly `len` bytes.
///
/// `None` for a length no CAN frame can carry: 9 to 11 bytes, and the other
/// gaps between the CAN FD sizes.
pub const fn len_to_dlc(len: usize) -> Option<u8> {
    let mut dlc = 0;
    while dlc < FD_PAYLOAD_LEN.len() {
        if FD_PAYLOAD_LEN[dlc] as usize == len {
            return Some(dlc as u8);
        }
        dlc += 1;
    }
    None
}

/// A CAN identifier, carrying its own width.
///
/// Standard and extended are distinct variants because an 11-bit `0x100` and
/// a 29-bit `0x100` are different frames on the wire, and I do not want them
/// comparing equal by accident.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum CanId {
    /// 11-bit identifier.
    Standard(u16),
    /// 29-bit identifier.
    Extended(u32),
}

impl CanId {
    /// Build a standard 11-bit identifier, rejecting out-of-range values.
    pub fn standard(raw: u16) -> Result<Self> {
        if u32::from(raw) > STANDARD_ID_MAX {
            return Err(Error::InvalidId {
                raw: u32::from(raw),
                reason: "exceeds the 11-bit standard identifier range",
            });
        }
        Ok(CanId::Standard(raw))
    }

    /// Build an extended 29-bit identifier, rejecting out-of-range values.
    pub fn extended(raw: u32) -> Result<Self> {
        if raw > EXTENDED_ID_MASK {
            return Err(Error::InvalidId {
                raw,
                reason: "exceeds the 29-bit extended identifier range",
            });
        }
        Ok(CanId::Extended(raw))
    }

    /// Interpret a message identifier as written in a DBC `BO_` record.
    ///
    /// Bit 31 set means extended, and the remaining 29 bits are the
    /// identifier. Bit 31 clear means standard, and the value must fit in 11
    /// bits — anything larger is a malformed DBC, not something to truncate.
    pub fn from_dbc(raw: u32) -> Result<Self> {
        if raw & DBC_EXTENDED_FLAG != 0 {
            CanId::extended(raw & EXTENDED_ID_MASK)
        } else if raw > STANDARD_ID_MAX {
            Err(Error::InvalidId {
                raw,
                reason: "no extended flag set but value exceeds 11 bits",
            })
        } else {
            Ok(CanId::Standard(raw as u16))
        }
    }

    /// The numeric identifier, without the width distinction.
    pub fn raw(self) -> u32 {
        match self {
            CanId::Standard(v) => u32::from(v),
            CanId::Extended(v) => v,
        }
    }

    /// Whether this is a 29-bit identifier.
    pub fn is_extended(self) -> bool {
        matches!(self, CanId::Extended(_))
    }
}

/// A single CAN or CAN FD frame.
///
/// `Copy`, and never heap-allocates. [`CanFrame::new`] is the only way to set
/// the length, so `len <= MAX_FRAME_LEN` is an invariant the decoder can rely
/// on without re-checking.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CanFrame {
    id: CanId,
    data: [u8; MAX_FRAME_LEN],
    len: u8,
    /// Set only when a classic frame declares a code above the eight bytes
    /// it carries; otherwise the payload length gives the code back.
    len8_dlc: Option<u8>,
    /// Capture time in microseconds since an arbitrary epoch. The decoder
    /// does not read this; it is carried through for downstream consumers.
    pub timestamp_us: u64,
}

impl CanFrame {
    /// Build a frame from a payload slice, rejecting over-long payloads.
    pub fn new(id: CanId, payload: &[u8], timestamp_us: u64) -> Result<Self> {
        if payload.len() > MAX_FRAME_LEN {
            return Err(Error::FrameTooLong { len: payload.len() });
        }
        let mut data = [0u8; MAX_FRAME_LEN];
        data[..payload.len()].copy_from_slice(payload);
        Ok(Self {
            id,
            data,
            len: payload.len() as u8,
            len8_dlc: None,
            timestamp_us,
        })
    }

    /// Build a frame that declares a data length code of its own.
    ///
    /// Classic CAN allows the codes 9 to 15 on a frame that still carries
    /// eight bytes, and an ECU can mean something by which one it sends, so
    /// the code is kept rather than recomputed from the payload. Any other
    /// code has to be the one the payload length implies: a code that
    /// disagrees with the bytes beside it is a corrupt record, not a frame.
    pub fn with_dlc(id: CanId, payload: &[u8], timestamp_us: u64, dlc: u8) -> Result<Self> {
        let len8 =
            payload.len() == 8 && (FIRST_LEN8_DLC..FD_PAYLOAD_LEN.len() as u8).contains(&dlc);
        if !len8 && dlc_to_len(dlc) != Some(payload.len()) {
            return Err(Error::DlcMismatch {
                dlc,
                len: payload.len(),
            });
        }
        Ok(Self {
            len8_dlc: len8.then_some(dlc),
            ..Self::new(id, payload, timestamp_us)?
        })
    }

    /// The frame's identifier.
    pub fn id(&self) -> CanId {
        self.id
    }

    /// The payload actually carried, not the full 64-byte backing array.
    pub fn data(&self) -> &[u8] {
        &self.data[..self.len as usize]
    }

    /// Payload length in bytes.
    pub fn len(&self) -> usize {
        self.len as usize
    }

    /// The data length code the frame declares.
    ///
    /// Usually the code its payload length implies; a classic frame built by
    /// [`CanFrame::with_dlc`] can declare a higher one. `None` for a payload
    /// length no CAN frame can carry, which a truncated capture still
    /// produces.
    pub fn dlc(&self) -> Option<u8> {
        self.len8_dlc.or_else(|| len_to_dlc(self.len()))
    }

    /// Whether the frame carries no payload.
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dbc_id_without_flag_is_standard() {
        assert_eq!(CanId::from_dbc(0x100).unwrap(), CanId::Standard(0x100));
    }

    #[test]
    fn dbc_id_with_bit31_set_is_extended_and_strips_the_flag() {
        let id = CanId::from_dbc(0x98FE_EE00).unwrap();
        assert_eq!(id, CanId::Extended(0x18FE_EE00));
        assert!(id.is_extended());
        assert_eq!(id.raw(), 0x18FE_EE00);
    }

    #[test]
    fn dbc_id_over_11_bits_without_flag_is_rejected_not_truncated() {
        assert!(matches!(
            CanId::from_dbc(0x800),
            Err(Error::InvalidId { .. })
        ));
    }

    #[test]
    fn standard_and_extended_ids_with_equal_value_are_not_equal() {
        assert_ne!(CanId::Standard(0x100), CanId::Extended(0x100));
    }

    #[test]
    fn frame_data_returns_only_the_payload_length() {
        let f = CanFrame::new(CanId::Standard(0x100), &[1, 2, 3], 0).unwrap();
        assert_eq!(f.data(), &[1, 2, 3]);
        assert_eq!(f.len(), 3);
    }

    #[test]
    fn frame_accepts_a_full_64_byte_can_fd_payload() {
        let payload = [0xAAu8; MAX_FRAME_LEN];
        let f = CanFrame::new(CanId::Standard(0x1), &payload, 0).unwrap();
        assert_eq!(f.len(), MAX_FRAME_LEN);
    }

    #[test]
    fn frame_rejects_a_payload_over_the_can_fd_maximum() {
        let payload = [0u8; MAX_FRAME_LEN + 1];
        assert!(matches!(
            CanFrame::new(CanId::Standard(0x1), &payload, 0),
            Err(Error::FrameTooLong { len: 65 })
        ));
    }

    #[test]
    fn every_data_length_code_maps_to_the_length_can_fd_gives_it() {
        let lengths: Vec<_> = (0..16).map(|dlc| dlc_to_len(dlc).unwrap()).collect();
        assert_eq!(
            lengths,
            [0, 1, 2, 3, 4, 5, 6, 7, 8, 12, 16, 20, 24, 32, 48, 64]
        );
        assert_eq!(dlc_to_len(16), None);
        for dlc in 0..16u8 {
            assert_eq!(len_to_dlc(dlc_to_len(dlc).unwrap()), Some(dlc));
        }
    }

    #[test]
    fn a_length_no_frame_can_carry_has_no_data_length_code() {
        for len in [9, 10, 11, 13, 17, 63, 65] {
            assert_eq!(len_to_dlc(len), None, "{len} bytes");
        }
    }

    #[test]
    fn a_frame_reports_the_code_its_payload_length_implies() {
        let dlc = |len: usize| {
            CanFrame::new(CanId::Standard(1), &vec![0; len], 0)
                .unwrap()
                .dlc()
        };
        assert_eq!(dlc(0), Some(0));
        assert_eq!(dlc(8), Some(8));
        assert_eq!(dlc(12), Some(9));
        assert_eq!(dlc(64), Some(15));
        // A capture truncated mid-frame; no code describes what is left.
        assert_eq!(dlc(9), None);
    }

    #[test]
    fn a_classic_frame_keeps_a_data_length_code_above_its_eight_bytes() {
        let frame = CanFrame::with_dlc(CanId::Standard(0x123), &[0xAA; 8], 0, 9).unwrap();
        assert_eq!(frame.dlc(), Some(9));
        assert_eq!(frame.len(), 8);
        assert_eq!(frame.data(), [0xAA; 8]);
        assert_eq!(
            CanFrame::with_dlc(CanId::Standard(1), &[0; 8], 0, 15)
                .unwrap()
                .dlc(),
            Some(15)
        );
    }

    #[test]
    fn a_data_length_code_may_restate_the_payload_length_but_not_contradict_it() {
        let ok = |payload: &[u8], dlc| CanFrame::with_dlc(CanId::Standard(1), payload, 0, dlc);
        assert!(ok(&[0; 3], 3).is_ok());
        assert!(ok(&[0; 12], 9).is_ok());
        assert!(matches!(
            ok(&[0; 3], 9),
            Err(Error::DlcMismatch { dlc: 9, len: 3 })
        ));
        assert!(matches!(ok(&[0; 12], 10), Err(Error::DlcMismatch { .. })));
        assert!(matches!(ok(&[0; 8], 16), Err(Error::DlcMismatch { .. })));
    }

    #[test]
    fn two_frames_that_differ_only_in_their_data_length_code_are_not_equal() {
        let plain = CanFrame::new(CanId::Standard(1), &[0; 8], 0).unwrap();
        let coded = CanFrame::with_dlc(CanId::Standard(1), &[0; 8], 0, 9).unwrap();
        assert_ne!(plain, coded);
    }

    #[test]
    fn frame_is_copy_and_therefore_allocation_free() {
        fn consume(_: CanFrame) {}
        let f = CanFrame::new(CanId::Standard(0x100), &[1], 0).unwrap();
        consume(f);
        assert_eq!(f.len(), 1);
    }
}
