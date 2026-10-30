//! Samples gathered out of the media data of a file as its bytes arrive
//!
//! Media data is the bytes of samples, which structure-data describes by
//! reference (ISO/IEC 14496-12 §8.1.1, A.3). A [`MediaDataReader`] takes the
//! [`SampleLocation`](crate::SampleLocation)s structure-data resolves to, and
//! the bytes of the file as they arrive, and gathers each sample out of them.
//! A writer, the other way, lays the media data of the samples a muxer writes
//! together into an `mdat`.

mod reader;
mod writer;

pub use reader::{MediaDataLimits, MediaDataReader};
pub(crate) use writer::MediaDataWriter;
