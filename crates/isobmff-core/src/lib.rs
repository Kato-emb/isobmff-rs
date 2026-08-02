//! Minimal, dependency-free extension layer for ISO base media file format boxes

mod box_header;
mod box_size;
mod box_type;
mod fourcc;
mod raw_box;
mod uuid;

pub use box_header::{BoxHeader, DecodeError};
pub use box_size::{BoxSize, CompactSize, ExtendedSize};
pub use box_type::{BoxType, CompactType};
pub use fourcc::FourCC;
pub use raw_box::{Boxes, RawBox, boxes};
pub use uuid::Uuid;
