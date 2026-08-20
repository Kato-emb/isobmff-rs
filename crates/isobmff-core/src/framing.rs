//! Boxes of ISO/IEC 14496-12 §4.2 as the size and type that frame them
//!
//! Every object in the format opens with a header giving both its size and its
//! type, and §4.2 has a reader ignore and skip a box whose type it does not
//! recognize — so for such a box the header is all it ever reads. The types
//! here are that header and the size and type it declares, together with the
//! box it delimits while the payload it spans stays unread.

pub(crate) mod box_header;
pub(crate) mod box_size;
pub(crate) mod box_type;
pub(crate) mod raw_box;

pub use box_header::BoxHeader;
pub use box_size::{BoxSize, CompactSize, ExtendedSize};
pub use box_type::{BoxType, CompactType};
pub use raw_box::{Boxes, RawBox, boxes};
