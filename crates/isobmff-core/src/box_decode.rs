//! [`BoxDecode`] and [`DecodeError`], the box payload of ISO/IEC 14496-12 §4.2 read into a value

#[cfg(feature = "alloc")]
use alloc::boxed::Box;
use core::error;
use core::fmt;
use core::str;

use crate::box_type::BoxType;
use crate::field::FieldReadError;
use crate::raw_box::RawBoxError;

/// Value that the payload of a box decodes into
///
/// # Examples
///
/// ```
/// use isobmff_core::{BoxDecode, DecodeError, FieldReadError, FieldReader};
///
/// // A box whose payload is one 32-bit sequence number
/// #[derive(PartialEq, Debug)]
/// struct SequenceNumberBox {
///     sequence_number: u32,
/// }
///
/// impl BoxDecode for SequenceNumberBox {
///     fn decode_payload(payload: &[u8]) -> Result<Self, DecodeError> {
///         let mut reader = FieldReader::new(payload);
///         let sequence_number = reader.read_u32()?;
///         reader.finish()?;
///
///         Ok(Self { sequence_number })
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
///     Err(DecodeError::Field(FieldReadError::UnexpectedEof {
///         needed: 4,
///         available: 3
///     }))
/// ));
///
/// // Bytes past the field are an error, not a remainder to skip over
/// assert!(matches!(
///     SequenceNumberBox::decode_payload(b"\0\0\0\x07!"),
///     Err(DecodeError::Field(FieldReadError::TrailingBytes { remaining: 1 }))
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
    /// [`TrailingBytes`](FieldReadError::TrailingBytes) rather than a remainder
    /// to pass over.
    ///
    /// A container reads the boxes its payload holds, so its failures reach
    /// past its own fields: the ones a child brings are listed on
    /// [`DecodeError`].
    ///
    /// # Errors
    ///
    /// * [`Field`](DecodeError::Field): `payload` ends inside a field of
    ///   `Self`, or holds bytes past them.
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
    /// Fields of the box do not read off its payload
    Field(FieldReadError),
    /// Full box declares a version the box does not read
    UnsupportedVersion(u8),
    /// Full box declares flags the box does not read
    UnsupportedFlags(u32),
    /// Full box declares flags the spec does not allow together
    ConflictingFlags(u32),
    /// Field the spec declares as text does not read as UTF-8
    InvalidUtf8(str::Utf8Error),
    /// Payload of a container does not split into the boxes it holds
    Framing(RawBoxError),
    /// Container lacks a child box the spec marks mandatory
    MissingMandatoryBox(BoxType),
    /// Container holds more of a child box than its quantity allows
    DuplicateBox(BoxType),
    /// Count a box declares does not match the entries it frames for itself
    ///
    /// A box whose entries frame themselves counts them twice over, and the two
    /// counts can disagree. Where the count is the framing — a table of entries
    /// of one fixed length — a count too large runs the payload out and a count
    /// too small leaves bytes past the entries, both of which are
    /// [`Field`](Self::Field).
    EntryCountMismatch {
        /// Entries the `entry_count` field declares
        declared: u64,
        /// Entries the payload holds
        actual: u64,
    },
    /// Count a box declares is past the entries the box reads
    ///
    /// A count that frames entries of no length is bounded by nothing in the
    /// payload, so the box that reads it states how many of them it holds.
    UnsupportedEntryCount {
        /// Entries the count field declares
        declared: u64,
        /// Entries the box reads
        limit: u64,
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

impl From<FieldReadError> for DecodeError {
    fn from(error: FieldReadError) -> Self {
        Self::Field(error)
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
            Self::Field(_) => {
                formatter.write_str("box payload does not read as the fields it holds")
            }
            Self::UnsupportedVersion(version) => write!(
                formatter,
                "full box declares version {version}, which this box does not read"
            ),
            Self::UnsupportedFlags(bits) => write!(
                formatter,
                "full box declares flags {bits:#08x}, which this box does not read"
            ),
            Self::ConflictingFlags(bits) => write!(
                formatter,
                "full box declares flags {bits:#08x}, which the spec does not allow together"
            ),
            Self::InvalidUtf8(_) => {
                formatter.write_str("box payload holds a string that is not UTF-8")
            }
            Self::Framing(_) => {
                formatter.write_str("container payload does not split into the boxes it holds")
            }
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
            Self::UnsupportedEntryCount { declared, limit } => write!(
                formatter,
                "box declares {declared} entries, past the {limit} this box reads"
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
            Self::UnsupportedVersion(_)
            | Self::UnsupportedFlags(_)
            | Self::ConflictingFlags(_)
            | Self::MissingMandatoryBox(_)
            | Self::DuplicateBox(_)
            | Self::EntryCountMismatch { .. }
            | Self::UnsupportedEntryCount { .. } => None,
            Self::Field(ref error) => Some(error),
            Self::InvalidUtf8(ref error) => Some(error),
            Self::Framing(ref error) => Some(error),
            #[cfg(feature = "alloc")]
            Self::Child { ref source, .. } => Some(source.as_ref()),
        }
    }
}

#[cfg(test)]
mod tests {
    #[cfg(feature = "alloc")]
    use alloc::string::String;
    use alloc::string::ToString as _;
    #[cfg(feature = "alloc")]
    use alloc::vec;
    use core::error::Error as _;

    use super::DecodeError;
    use crate::box_header::BoxHeaderError;
    use crate::box_type::BoxType;
    #[cfg(feature = "alloc")]
    use crate::field::FieldReadError;
    use crate::raw_box::RawBoxError;

    #[test]
    fn display_of_a_framing_failure_says_what_the_container_was_doing() {
        let framing_error = RawBoxError::Header(BoxHeaderError::TruncatedHeader {
            needed: 16,
            available: 12,
        });

        let error = DecodeError::Framing(framing_error);

        assert_eq!(
            error.to_string(),
            "container payload does not split into the boxes it holds"
        );
        assert_ne!(error.to_string(), framing_error.to_string());
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
    fn display_of_unsupported_flags_names_the_bits_the_box_does_not_read() {
        let error = DecodeError::UnsupportedFlags(0x0000_1000);

        assert_eq!(
            error.to_string(),
            "full box declares flags 0x001000, which this box does not read"
        );
    }

    #[test]
    fn display_of_conflicting_flags_names_the_bits_that_conflict() {
        let error = DecodeError::ConflictingFlags(0x0000_0404);

        assert_eq!(
            error.to_string(),
            "full box declares flags 0x000404, which the spec does not allow together"
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
    fn display_of_an_unsupported_entry_count_names_the_count_and_the_limit() {
        let error = DecodeError::UnsupportedEntryCount {
            declared: 4_294_967_295,
            limit: 1_048_576,
        };

        assert_eq!(
            error.to_string(),
            "box declares 4294967295 entries, past the 1048576 this box reads"
        );
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

    #[cfg(feature = "alloc")]
    #[test]
    fn nested_children_spell_out_the_path_to_the_box_that_failed() {
        let innermost = DecodeError::Field(FieldReadError::UnexpectedEof {
            needed: 20,
            available: 12,
        });
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
                "box payload does not read as the fields it holds",
                "box payload of 20 bytes cut short by an input of 12",
            ]
            .map(String::from)
            .to_vec()
        );
    }
}
