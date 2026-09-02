//! `spatiax` — a CAN bus / DBC decoder for motorsport telemetry.
//!
//! The organising claim is not "this decodes CAN" but "this decodes CAN
//! correctly, and here is the evidence". That shapes the layout: bit
//! extraction ([`decode`]) and insertion ([`encode`]) are small and free of
//! I/O so they can be tested exhaustively, while parsing ([`dbc`]) and frame
//! types ([`frame`]) live outside them.
//!
//! What exists today: a DBC parser for `BO_`/`SG_`/`VAL_` records, bit-exact
//! extraction and insertion for Intel and Motorola byte orders, signed and
//! unsigned signals, factor/offset scaling, extended identifiers,
//! multiplexed messages (simple multiplexing: one `M` per message),
//! value-table labels, and replay of `candump` logs ([`candump`]).
//!
//! The evidence: hand-computed reference vectors, property tests over every
//! layout from 1 to 64 bits, and a differential test that decodes generated
//! databases with both this crate and `cantools` and requires agreement.
//!
//! # Example
//!
//! ```
//! use spatiax::{CanFrame, CanId, dbc};
//!
//! let db = dbc::parse(
//!     "BO_ 256 EngineData: 8 ECU\n \
//!      SG_ EngineRPM : 0|16@1+ (0.25,0) [0|16383.75] \"rpm\" DASH\n \
//!      SG_ CoolantTemp : 16|8@1+ (0.5,-40) [-40|87.5] \"degC\" DASH\n",
//! )?;
//!
//! let frame = CanFrame::new(CanId::Standard(256), &[0x34, 0x12, 0x64], 0)?;
//! let decoded: Vec<_> = db
//!     .decode_frame(&frame)
//!     .expect("0x100 is in the database")
//!     .collect::<Result<_, _>>()?;
//!
//! assert_eq!(decoded[0].signal.name, "EngineRPM");
//! assert_eq!(decoded[0].value, 1165.0);
//! assert_eq!(decoded[1].signal.name, "CoolantTemp");
//! assert_eq!(decoded[1].value, 10.0);
//! # Ok::<(), spatiax::Error>(())
//! ```

#![deny(missing_docs)]
#![deny(rust_2018_idioms)]
#![warn(clippy::all)]

pub mod candump;
pub mod dbc;
pub mod decode;
pub mod encode;
pub mod error;
pub mod frame;

pub use dbc::{Database, Decoded, Message, Signal};
pub use error::{Error, Result};
pub use frame::{CanFrame, CanId, MAX_FRAME_LEN};
