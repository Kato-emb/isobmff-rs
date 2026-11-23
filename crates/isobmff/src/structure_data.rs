//! The `structure-data` of a file, which gives the samples their location, size, timing and characteristics
//!
//! The `moov` and `moof` of a file declare its samples in tables, ISO/IEC
//! 14496-12 §3.1.54. [`timeline`] reads when the samples of a sample table
//! are decoded and composed, and how long they last.

mod error;
pub mod timeline;

pub use error::Error;
