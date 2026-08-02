//! Minimal, dependency-free extension layer for ISO base media file format boxes

mod box_type;
mod fourcc;
mod uuid;

pub use box_type::{BoxType, CompactType};
pub use fourcc::FourCC;
pub use uuid::Uuid;
