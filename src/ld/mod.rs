//! MoTeC `.ld` export: decoded CAN written as a file i2 opens.
//!
//! i2 is the analysis tool on GT3, GT4, LMP and most single-seater grids,
//! and it reads a format MoTeC has never documented. Writing it is what
//! turns "decode your log" into "decode your log and open it in the tool
//! you already use", so this module is the end of the pipeline the rest of
//! the crate builds: [`crate::candump`] or `live` supplies frames,
//! [`crate::dbc`] turns them into values, and this turns those into a file.
//!
//! Two things happen here that the rest of the crate refuses to do
//! elsewhere, and both are visible in the output: irregular CAN updates are
//! resampled onto one fixed grid ([`sample`]), and `f64` values are stored
//! as `f32`. The first is forced by the format, which has no per-sample
//! timestamp; the second is a choice, made so the stored word is the
//! physical value with no calibration arithmetic between them.
//!
//! The submodules are private and everything public is re-exported here,
//! unlike [`crate::dbc`]. The resampling function wants to be called
//! `sample`, and so does the module it lives in; Rust would allow both, but
//! `ld::sample` meaning two things depending on what follows it is a
//! needless thing to make a reader resolve.
//!
//! # Example
//!
//! ```
//! use spatiax::{CanFrame, CanId, dbc, ld};
//!
//! let db = dbc::parse(
//!     "BO_ 256 EngineData: 8 ECU\n \
//!      SG_ EngineRPM : 0|16@1+ (0.25,0) [0|16383.75] \"rpm\" DASH\n",
//! )?;
//! let frames = [
//!     CanFrame::new(CanId::Standard(256), &[0x34, 0x12], 0)?,
//!     CanFrame::new(CanId::Standard(256), &[0x35, 0x12], 1_000_000)?,
//! ];
//!
//! let sampled = ld::sample(&db, &frames, Some(10))?;
//! assert_eq!(sampled.session.channels()[0].name, "EngineRPM");
//! assert_eq!(sampled.session.sample_count(), 11);
//!
//! let mut file = Vec::new();
//! ld::write(&sampled.session, &mut file)?;
//! assert_eq!(&file[..4], &[0x40, 0x00, 0x00, 0x00]);
//! # Ok::<(), spatiax::Error>(())
//! ```

mod sample;
mod types;
mod writer;

pub use sample::{RATE_LADDER, Sampled, sample};
pub use types::{Channel, NAME_LEN, SHORT_NAME_LEN, Session, UNIT_LEN};
pub use writer::write;
