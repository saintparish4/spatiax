//! `spatiax` — a CAN bus / DBC decoder for motorsport telemetry.
//!
//! The organising claim is not "this decodes CAN" but "this decodes CAN
//! correctly, and here is the evidence". That shapes the layout: bit
//! extraction is kept small and free of I/O so it can be property-tested and
//! differentially tested against `cantools`, while parsing, capture, and
//! export live outside it.
//!
//! Today the crate provides the frame, identifier, and error types, the DBC
//! signal definitions, and bit extraction for both byte orders. The DBC text
//! parser is not here yet.
//!
//! # Example
//!
//! ```
//! use spatiax::{CanFrame, CanId};
//!
//! // DBC writes extended identifiers with bit 31 set.
//! let id = CanId::from_dbc(0x98FE_EE00)?;
//! assert!(id.is_extended());
//!
//! let frame = CanFrame::new(id, &[0x01, 0x02], 0)?;
//! assert_eq!(frame.data(), &[0x01, 0x02]);
//! # Ok::<(), spatiax::Error>(())
//! ```

#![deny(missing_docs)]
#![deny(rust_2018_idioms)]
#![warn(clippy::all)]

pub mod dbc;
pub mod decode;
pub mod error;
pub mod frame;

pub use dbc::{Database, Signal};
pub use error::{Error, Result};
pub use frame::{CanFrame, CanId, MAX_FRAME_LEN};
