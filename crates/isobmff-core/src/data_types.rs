//! Values the fields of a box carry
//!
//! Wherever ISO/IEC 14496-12 gives the bytes of a field a shape of its own — a
//! four-character code, a UUID, a fixed-point number, a null-terminated
//! string, the `version` and `flags` a full box opens with — a type here
//! stands for that shape, and cites the subclause that settles it.
//!
//! What a box does with a value belongs to the box. These types convert between
//! the form the wire carries and the form a caller states values in, and do
//! nothing else: no arithmetic on fixed-point numbers.

pub(crate) mod fixed_point;
pub(crate) mod fourcc;
pub(crate) mod full_box;
#[cfg(feature = "alloc")]
pub(crate) mod utf8_string;
pub(crate) mod uuid;

pub use fixed_point::{I8F8, I16F16, U16F16};
pub use fourcc::FourCc;
pub use full_box::{FullBoxFlags, FullBoxHeader};
#[cfg(feature = "alloc")]
pub use utf8_string::Utf8String;
pub use uuid::Uuid;
