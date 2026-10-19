//! [`Error`], the reason the top-level boxes of a file do not form the structure it keeps

use isobmff_core::BoxType;

/// Reason the top-level boxes of a file do not form the structure it keeps
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Error {
    /// Box the structure requires is not there
    MissingMandatoryBox {
        /// Type of the box that is missing
        box_type: BoxType,
    },
    /// Box the structure carries once is there twice
    DuplicateBox {
        /// Type of the box that came again
        box_type: BoxType,
    },
    /// Box lies out of the order the structure keeps
    BoxOutOfOrder {
        /// Type of the box that came out of order
        box_type: BoxType,
    },
}
