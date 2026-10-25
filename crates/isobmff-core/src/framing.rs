//! Boxes of ISO/IEC 14496-12 as the header that frames them
//!
//! Every box opens with a header giving both its size and its type, and a
//! reader ignores and skips a box whose type it does not recognize — so for
//! such a box the header is all it ever reads. The types here are what that
//! header declares.

pub(crate) mod box_type;

pub use box_type::{BoxType, CompactType, ExtendedType};
