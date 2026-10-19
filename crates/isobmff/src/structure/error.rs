//! [`StructureError`], the reason the top-level boxes of a file do not form the structure it keeps

use isobmff_core::BoxType;

use crate::structure::{DemuxError, MuxError};

/// Reason the top-level boxes of a file do not form the structure it keeps
///
/// Its failures are carried into [`DemuxError`] and [`MuxError`] as the
/// variants of the same names.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum StructureError {
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

impl From<StructureError> for DemuxError {
    /// Carries the failure of the structure through as the variant of the same name
    fn from(error: StructureError) -> Self {
        match error {
            StructureError::MissingMandatoryBox { box_type } => {
                Self::MissingMandatoryBox { box_type }
            }
            StructureError::DuplicateBox { box_type } => Self::DuplicateBox { box_type },
            StructureError::BoxOutOfOrder { box_type } => Self::BoxOutOfOrder { box_type },
        }
    }
}

impl From<StructureError> for MuxError {
    /// Carries the failure of the structure through as the variant of the same name
    fn from(error: StructureError) -> Self {
        match error {
            StructureError::MissingMandatoryBox { box_type } => {
                Self::MissingMandatoryBox { box_type }
            }
            StructureError::DuplicateBox { box_type } => Self::DuplicateBox { box_type },
            StructureError::BoxOutOfOrder { box_type } => Self::BoxOutOfOrder { box_type },
        }
    }
}
