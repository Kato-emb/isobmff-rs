//! Boxes of ISO/IEC 14496-12 as the header that frames them
//!
//! Every box opens with a header giving both its size and its type, and a
//! reader ignores and skips a box whose type it does not recognize — so for
//! such a box the header is all it ever reads. The types here are that
//! header, what it declares, and the box it delimits while the payload it
//! spans stays unread.

pub(crate) mod box_header;
pub(crate) mod box_size;
pub(crate) mod box_type;
pub(crate) mod raw_box;

pub use box_header::BoxHeader;
pub use box_size::{BoxSize, CompactSize, ExtendedSize};
pub use box_type::{BoxType, CompactType, ExtendedType};
pub use raw_box::{Boxes, RawBox};
