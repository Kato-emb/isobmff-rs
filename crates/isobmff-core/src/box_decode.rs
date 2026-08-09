//! [`BoxDecode`] and [`DecodeError`], the box payload of ISO/IEC 14496-12 §4.2 read into a value

#[cfg(feature = "alloc")]
use alloc::boxed::Box;
use core::error;
use core::fmt;
use core::str;

use crate::box_type::BoxType;
use crate::raw_box::RawBoxError;

/// Value that the payload of a box decodes into
///
/// # Examples
///
/// ```
/// use isobmff_core::{BoxDecode, DecodeError};
///
/// // A box whose payload is one 32-bit sequence number
/// #[derive(PartialEq, Debug)]
/// struct SequenceNumberBox {
///     sequence_number: u32,
/// }
///
/// impl BoxDecode for SequenceNumberBox {
///     fn decode_payload(payload: &[u8]) -> Result<Self, DecodeError> {
///         let (field, rest) =
///             payload
///                 .split_first_chunk::<4>()
///                 .ok_or(DecodeError::TruncatedPayload {
///                     needed: 4,
///                     available: u64::try_from(payload.len()).unwrap_or(u64::MAX),
///                 })?;
///
///         if !rest.is_empty() {
///             return Err(DecodeError::TrailingBytes {
///                 remaining: u64::try_from(rest.len()).unwrap_or(u64::MAX),
///             });
///         }
///
///         Ok(Self {
///             sequence_number: u32::from_be_bytes(*field),
///         })
///     }
/// }
///
/// // The payload arrives whole, the header already consumed by the caller
/// assert_eq!(
///     SequenceNumberBox::decode_payload(b"\0\0\0\x07").unwrap(),
///     SequenceNumberBox { sequence_number: 7 }
/// );
///
/// // A payload ending inside the field says how far it had to reach
/// assert!(matches!(
///     SequenceNumberBox::decode_payload(b"\0\0\0"),
///     Err(DecodeError::TruncatedPayload {
///         needed: 4,
///         available: 3
///     })
/// ));
///
/// // Bytes past the field are an error, not a remainder to skip over
/// assert!(matches!(
///     SequenceNumberBox::decode_payload(b"\0\0\0\x07!"),
///     Err(DecodeError::TrailingBytes { remaining: 1 })
/// ));
/// ```
pub trait BoxDecode: Sized {
    /// Decodes the payload of one box into a value
    ///
    /// `payload` is that payload whole and nothing besides: framing is settled
    /// before the call and the header is gone, as
    /// [`RawBox::payload`](crate::RawBox::payload) leaves it. Routing a box type
    /// to the implementation that reads it is the caller's part as well.
    ///
    /// Reading is strict. Every byte of `payload` belongs to a field of `Self`,
    /// and bytes the fields do not claim are
    /// [`TrailingBytes`](DecodeError::TrailingBytes) rather than a remainder to
    /// pass over.
    ///
    /// A container reads the boxes its payload holds, so its failures reach
    /// past its own fields: the ones a child brings are listed on
    /// [`DecodeError`].
    ///
    /// # Errors
    ///
    /// * [`TruncatedPayload`](DecodeError::TruncatedPayload): `payload` ends
    ///   inside a field.
    /// * [`TrailingBytes`](DecodeError::TrailingBytes): `payload` holds bytes
    ///   past the fields of `Self`.
    /// * The container failures of [`DecodeError`], for a box whose payload is
    ///   the boxes it contains.
    fn decode_payload(payload: &[u8]) -> Result<Self, DecodeError>;
}

/// Reason a payload does not read as the box it was framed as
///
/// Framing is settled before a payload reaches [`BoxDecode::decode_payload`],
/// so a frame that does not hold is [`RawBoxError`], the error of the layer
/// before this one. A container is the exception: it frames the boxes its own
/// payload holds, and carries what that reports as
/// [`Framing`](Self::Framing).
///
/// # `alloc`
///
/// [`Child`](Self::Child) needs the `alloc` feature, which is on by default.
/// Without it a container cannot own its children either, so the variant has
/// nothing to report.
#[non_exhaustive]
#[derive(Debug)]
pub enum DecodeError {
    /// Payload ends inside a field
    TruncatedPayload {
        /// Bytes the fields read so far require
        needed: u64,
        /// Bytes the payload offered
        available: u64,
    },
    /// Payload holds bytes past the fields of the box
    TrailingBytes {
        /// Bytes left over once every field was read
        remaining: u64,
    },
    /// Full box declares a version the box does not read
    UnsupportedVersion(u8),
    /// Field the spec declares as text does not read as UTF-8
    InvalidUtf8(str::Utf8Error),
    /// Payload of a container does not split into the boxes it holds
    Framing(RawBoxError),
    /// Container lacks a child box the spec marks mandatory
    MissingMandatoryBox(BoxType),
    /// Container holds more of a child box than its quantity allows
    DuplicateBox(BoxType),
    /// Count a box declares does not match the entries it holds
    EntryCountMismatch {
        /// Entries the `entry_count` field declares
        declared: u64,
        /// Entries the payload holds
        actual: u64,
    },
    /// Child box of a container does not decode
    ///
    /// Needs the `alloc` feature.
    #[cfg(feature = "alloc")]
    Child {
        /// Box type of the child that failed
        box_type: BoxType,
        /// Failure the child reported
        source: Box<dyn error::Error + Send + Sync>,
    },
}

impl DecodeError {
    /// Wraps the failure of a child box under the box type that names it
    ///
    /// Nesting the failures on the way out builds the path from the outermost
    /// container to the box that actually failed, which
    /// [`source`](error::Error::source) then walks.
    ///
    /// Needs the `alloc` feature.
    #[cfg(feature = "alloc")]
    #[must_use]
    pub fn child<Source>(box_type: BoxType, source: Source) -> Self
    where
        Source: error::Error + Send + Sync + 'static,
    {
        Self::Child {
            box_type,
            source: Box::new(source),
        }
    }
}

impl From<RawBoxError> for DecodeError {
    fn from(error: RawBoxError) -> Self {
        Self::Framing(error)
    }
}

impl fmt::Display for DecodeError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            Self::TruncatedPayload { needed, available } => write!(
                formatter,
                "box payload of {needed} bytes cut short by an input of {available}"
            ),
            Self::TrailingBytes { remaining } => write!(
                formatter,
                "box payload leaves {remaining} bytes past the fields it holds"
            ),
            Self::UnsupportedVersion(version) => write!(
                formatter,
                "full box declares version {version}, which this box does not read"
            ),
            Self::InvalidUtf8(_) => {
                formatter.write_str("box payload holds a string that is not UTF-8")
            }
            Self::Framing(ref error) => error.fmt(formatter),
            Self::MissingMandatoryBox(box_type) => {
                write!(formatter, "container holds no mandatory {box_type} box")
            }
            Self::DuplicateBox(box_type) => write!(
                formatter,
                "container holds more than one {box_type} box, which may appear once"
            ),
            Self::EntryCountMismatch { declared, actual } => write!(
                formatter,
                "box declares {declared} entries but holds {actual}"
            ),
            #[cfg(feature = "alloc")]
            Self::Child { box_type, .. } => {
                write!(formatter, "child {box_type} box does not decode")
            }
        }
    }
}

impl error::Error for DecodeError {
    fn source(&self) -> Option<&(dyn error::Error + 'static)> {
        match *self {
            Self::TruncatedPayload { .. }
            | Self::TrailingBytes { .. }
            | Self::UnsupportedVersion(_)
            | Self::MissingMandatoryBox(_)
            | Self::DuplicateBox(_)
            | Self::EntryCountMismatch { .. } => None,
            Self::InvalidUtf8(ref error) => Some(error),
            Self::Framing(ref error) => Some(error),
            #[cfg(feature = "alloc")]
            Self::Child { ref source, .. } => Some(source.as_ref()),
        }
    }
}

#[cfg(test)]
mod tests {
    use alloc::string::{String, ToString as _};
    use alloc::vec;
    use core::error::Error as _;

    use super::DecodeError;
    use crate::box_header::BoxHeaderError;
    use crate::box_type::BoxType;
    use crate::raw_box::RawBoxError;

    #[test]
    fn display_of_a_truncated_payload_names_both_lengths() {
        let error = DecodeError::TruncatedPayload {
            needed: 16,
            available: 12,
        };

        assert_eq!(
            error.to_string(),
            "box payload of 16 bytes cut short by an input of 12"
        );
    }

    #[test]
    fn display_of_trailing_bytes_names_how_many_are_left() {
        let error = DecodeError::TrailingBytes { remaining: 4 };

        assert_eq!(
            error.to_string(),
            "box payload leaves 4 bytes past the fields it holds"
        );
    }

    #[test]
    fn display_of_a_framing_failure_reads_as_the_framing_error_itself() {
        let framing_error = RawBoxError::Header(BoxHeaderError::TruncatedHeader {
            needed: 16,
            available: 12,
        });

        assert_eq!(
            DecodeError::Framing(framing_error).to_string(),
            framing_error.to_string()
        );
    }

    #[test]
    fn display_of_an_unsupported_version_names_the_version() {
        let error = DecodeError::UnsupportedVersion(2);

        assert_eq!(
            error.to_string(),
            "full box declares version 2, which this box does not read"
        );
    }

    #[test]
    fn display_of_a_missing_mandatory_box_names_the_box_type() {
        let error = DecodeError::MissingMandatoryBox(BoxType::compact(*b"mvhd"));

        assert_eq!(error.to_string(), "container holds no mandatory mvhd box");
    }

    #[test]
    fn display_of_a_duplicate_box_names_the_box_type() {
        let error = DecodeError::DuplicateBox(BoxType::compact(*b"tkhd"));

        assert_eq!(
            error.to_string(),
            "container holds more than one tkhd box, which may appear once"
        );
    }

    #[test]
    fn display_of_an_entry_count_mismatch_names_both_counts() {
        let error = DecodeError::EntryCountMismatch {
            declared: 4,
            actual: 2,
        };

        assert_eq!(error.to_string(), "box declares 4 entries but holds 2");
    }

    #[test]
    fn a_framing_failure_carries_the_framing_error_as_its_source() {
        let framing_error = RawBoxError::TruncatedBox {
            needed: 32,
            available: 24,
        };

        let source = DecodeError::Framing(framing_error)
            .source()
            .map(|error| error.to_string());

        assert_eq!(source, Some(framing_error.to_string()));
    }

    #[test]
    fn nested_children_spell_out_the_path_to_the_box_that_failed() {
        let innermost = DecodeError::TruncatedPayload {
            needed: 20,
            available: 12,
        };
        let error = DecodeError::child(
            BoxType::compact(*b"trak"),
            DecodeError::child(BoxType::compact(*b"tkhd"), innermost),
        );

        let mut path = vec![error.to_string()];
        let mut step = error.source();
        while let Some(current) = step {
            path.push(current.to_string());
            step = current.source();
        }

        assert_eq!(
            path,
            [
                "child trak box does not decode",
                "child tkhd box does not decode",
                "box payload of 20 bytes cut short by an input of 12",
            ]
            .map(String::from)
            .to_vec()
        );
    }
}
