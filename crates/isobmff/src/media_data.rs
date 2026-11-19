//! The samples of a file assembled from the bytes of its media data, at the locations the structure-data gives
//!
//! [`MediaDataReader`] takes the [`SampleLocation`](crate::SampleLocation) of
//! each sample to assemble, names where it wants the bytes of the media data
//! next, and lends each sample once its bytes are in hand. It reads the media
//! data of any file, such as one a data reference names, ISO/IEC 14496-12
//! §8.7.2, whose bytes the [`Demuxer`](crate::Demuxer) does not request.

mod error;
mod event;
mod reader;

pub use error::Error;
pub use event::{Event, Sample};
pub use reader::MediaDataReader;
