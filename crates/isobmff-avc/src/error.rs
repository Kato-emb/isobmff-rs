//! [`Error`], the reason the NAL units of a sample do not frame or read back

use core::error;
use core::fmt;

use isobmff_core::Category;

/// Failure of framing the NAL units of an AVC sample, or of reading them back
///
/// The sample entries and the decoder configuration read and write through
/// [`BoxDecode`](isobmff_core::BoxDecode) and
/// [`BoxEncode`](isobmff_core::BoxEncode), and fail with an
/// [`isobmff_core::Error`]; this type reports what
/// [`LengthSizeMinusOne::frame`](crate::LengthSizeMinusOne::frame) and
/// [`LengthSizeMinusOne::nal_units`](crate::LengthSizeMinusOne::nal_units)
/// add.
///
/// # Examples
///
/// ```
/// use isobmff_avc::{Error, ErrorKind, LengthSizeMinusOne};
///
/// // A sample that ends inside the NAL unit its length announces
/// let failure = LengthSizeMinusOne::FOUR_BYTES
///     .nal_units(&[0, 0, 0, 2, 0x65])
///     .next()
///     .unwrap()
///     .unwrap_err();
/// assert_eq!(failure, Error::truncated_sample(2, 1));
/// assert_eq!(failure.kind(), ErrorKind::TruncatedSample);
/// ```
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct Error {
    representation: Representation,
}

impl Error {
    /// Returns the failure of a NAL unit of `length` bytes, longer than a
    /// `NALUnitLength` field of `length_size` bytes can state
    #[must_use]
    pub const fn nal_unit_length_out_of_range(length: u64, length_size: u64) -> Self {
        Self {
            representation: Representation::NALUnitLengthOutOfRange {
                length,
                length_size,
            },
        }
    }

    /// Returns the failure of a sample that ends with `available` bytes where
    /// a `NALUnitLength` field or the NAL unit it measures needs `needed`
    #[must_use]
    pub const fn truncated_sample(needed: u64, available: u64) -> Self {
        Self {
            representation: Representation::TruncatedSample { needed, available },
        }
    }

    /// Returns the kind of the failure
    #[must_use]
    pub const fn kind(self) -> ErrorKind {
        match self.representation {
            Representation::NALUnitLengthOutOfRange { .. } => ErrorKind::NALUnitLengthOutOfRange,
            Representation::TruncatedSample { .. } => ErrorKind::TruncatedSample,
        }
    }

    /// Returns the category the failure falls in
    #[must_use]
    pub const fn category(self) -> Category {
        match self.representation {
            Representation::NALUnitLengthOutOfRange { .. } => Category::Usage,
            Representation::TruncatedSample { .. } => Category::Malformed,
        }
    }

    /// Returns the bytes the failure required
    ///
    /// For [`NALUnitLengthOutOfRange`](ErrorKind::NALUnitLengthOutOfRange)
    /// this is the width of the `NALUnitLength` field the length did not fit.
    #[must_use]
    pub const fn needed_bytes(self) -> Option<u64> {
        match self.representation {
            Representation::NALUnitLengthOutOfRange { length_size, .. } => Some(length_size),
            Representation::TruncatedSample { needed, .. } => Some(needed),
        }
    }

    /// Returns the bytes the failure had to hand, for the kinds that count bytes
    #[must_use]
    pub const fn available_bytes(self) -> Option<u64> {
        match self.representation {
            Representation::TruncatedSample { available, .. } => Some(available),
            Representation::NALUnitLengthOutOfRange { .. } => None,
        }
    }

    /// Returns the value a field was given, for the kinds that name one
    ///
    /// For [`NALUnitLengthOutOfRange`](ErrorKind::NALUnitLengthOutOfRange)
    /// this is the length of the NAL unit.
    #[must_use]
    pub const fn value(self) -> Option<u64> {
        match self.representation {
            Representation::NALUnitLengthOutOfRange { length, .. } => Some(length),
            Representation::TruncatedSample { .. } => None,
        }
    }
}

impl fmt::Display for Error {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.representation {
            Representation::NALUnitLengthOutOfRange {
                length,
                length_size,
            } => write!(
                formatter,
                "NAL unit of {length} bytes does not fit a NALUnitLength field of {length_size} bytes"
            ),
            Representation::TruncatedSample { needed, available } => write!(
                formatter,
                "sample needs {needed} more bytes where {available} remain"
            ),
        }
    }
}

impl fmt::Debug for Error {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "Error({:?}: {self})", self.kind())
    }
}

impl error::Error for Error {}

/// Kind of an [`Error`], the situation without the values
#[non_exhaustive]
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum ErrorKind {
    /// A NAL unit is longer than the `NALUnitLength` field can state
    NALUnitLengthOutOfRange,
    /// A sample ends inside a `NALUnitLength` field or the NAL unit it measures
    TruncatedSample,
}

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
enum Representation {
    NALUnitLengthOutOfRange { length: u64, length_size: u64 },
    TruncatedSample { needed: u64, available: u64 },
}
