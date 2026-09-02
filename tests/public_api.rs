//! Exercises the crate the way a downstream user would: only through the
//! re-exports at the crate root.

use canline::{CanFrame, CanId, Error, MAX_FRAME_LEN, Result};

#[test]
fn extended_dbc_id_round_trips_through_a_frame() -> Result<()> {
    let id = CanId::from_dbc(0x98FE_EE00)?;
    let frame = CanFrame::new(id, &[0xDE, 0xAD, 0xBE, 0xEF], 1_000)?;

    assert_eq!(frame.id(), CanId::Extended(0x18FE_EE00));
    assert_eq!(frame.id().raw(), 0x18FE_EE00);
    assert_eq!(frame.data(), &[0xDE, 0xAD, 0xBE, 0xEF]);
    assert_eq!(frame.timestamp_us, 1_000);
    Ok(())
}

#[test]
fn standard_dbc_id_round_trips_through_a_frame() -> Result<()> {
    let id = CanId::from_dbc(0x7FF)?;
    let frame = CanFrame::new(id, &[], 0)?;

    assert_eq!(frame.id(), CanId::Standard(0x7FF));
    assert!(!frame.id().is_extended());
    assert!(frame.is_empty());
    Ok(())
}

#[test]
fn errors_are_distinguishable_by_variant_and_readable_as_text() {
    let too_long = CanFrame::new(CanId::Standard(1), &[0; MAX_FRAME_LEN + 1], 0);
    let bad_id = CanId::from_dbc(0x1000);

    assert!(matches!(too_long, Err(Error::FrameTooLong { len: 65 })));
    assert!(matches!(bad_id, Err(Error::InvalidId { raw: 0x1000, .. })));

    let text = bad_id.unwrap_err().to_string();
    assert!(text.contains("0x1000"), "got: {text}");
}

#[test]
fn ids_are_usable_as_map_keys_without_width_collisions() {
    use std::collections::HashMap;

    let mut seen: HashMap<CanId, &str> = HashMap::new();
    seen.insert(CanId::Standard(0x100), "standard");
    seen.insert(CanId::Extended(0x100), "extended");

    assert_eq!(seen.len(), 2);
    assert_eq!(seen[&CanId::Standard(0x100)], "standard");
    assert_eq!(seen[&CanId::Extended(0x100)], "extended");
}
