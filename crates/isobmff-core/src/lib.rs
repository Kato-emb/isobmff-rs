//! Minimal, dependency-free extension layer for ISO base media file format boxes

mod box_header;
mod box_size;
mod box_type;
mod fourcc;
mod uuid;

pub use box_header::{BoxHeader, DecodeError};
pub use box_size::{BoxSize, CompactSize, ExtendedSize};
pub use box_type::{BoxType, CompactType};
pub use fourcc::FourCC;
pub use uuid::Uuid;
