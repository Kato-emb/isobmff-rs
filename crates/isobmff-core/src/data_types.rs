//! Values the fields of a box carry
//!
//! Wherever the spec gives the bytes of a box's fields a shape of their own —
//! a four-character code, a packed language code, a fixed-point number, a
//! null-terminated string, the fixed 32-byte name of a compressor, the `version` and `flags` a full box opens with — a
//! type here stands for that shape. §6.2.2 settles it for some of them, and
//! each type cites the section that settles its own.
//!
//! What a box does with a value belongs to the box. These types convert between
//! the raw form and the form a caller states values in, and do nothing else: no
//! arithmetic on fixed-point numbers, and no calendar.

pub(crate) mod compressor_name;
pub(crate) mod fixed_point;
pub(crate) mod fourcc;
pub(crate) mod full_box;
pub(crate) mod language_code;
pub(crate) mod matrix;
#[cfg(feature = "alloc")]
pub(crate) mod null_terminated_string;
pub(crate) mod times;
pub(crate) mod uuid;

pub use compressor_name::CompressorName;
pub use fixed_point::{I8F8, I16F16, U16F16};
pub use fourcc::FourCC;
pub use full_box::{FullBoxFields, FullBoxFlags};
pub use language_code::LanguageCode;
pub use matrix::Matrix;
#[cfg(feature = "alloc")]
pub use null_terminated_string::NullTerminatedString;
pub use times::Mp4EpochSeconds;
pub use uuid::Uuid;
