//! [`SampleError`], the reason the samples of a presentation do not read or write

use core::error;
use core::fmt;

use isobmff_core::Category;

/// Bytes one `trun` row states for a sample, the width of its `sample_size`
const LARGEST_SAMPLE_SIZE: u64 = u32::MAX as u64;

/// Furthest past its base a `trun` places its data, the width of its `data_offset`
const FURTHEST_DATA_OFFSET: u64 = i32::MAX.unsigned_abs() as u64;

/// Reason the samples of a presentation do not read or write
///
/// What went wrong is one [`kind`](Self::kind): a failure of the samples
/// themselves — data that lies behind what was read, a sample that never
/// arrived whole, a sample that states more than a fragment carries — or a
/// failure of one box, which [`isobmff_core::Error`] names and this type carries
/// through whole, as [`box_error`](Self::box_error). What a caller does about
/// either is one [`category`](Self::category).
///
/// The values a failure of the samples carries follow from its kind, and each
/// kind names its own on [`SampleErrorKind`]. A carried box failure keeps its
/// values and its container path on [`box_error`](Self::box_error), so the
/// accessors here report `None` for it.
///
/// # Examples
///
/// ```
/// use isobmff::{BoxType, Category, Error, ErrorKind, SampleError, SampleErrorKind};
///
/// // A failure of the samples names its own kind
/// let failure = SampleError::unknown_track_id(3);
/// assert_eq!(failure.kind(), SampleErrorKind::UnknownTrackId);
/// assert_eq!(failure.category(), Category::Malformed);
/// assert_eq!(failure.value(), Some(3));
/// assert_eq!(failure.box_error(), None);
///
/// // A failure of one box is carried through whole
/// let missing = Error::missing_mandatory_box(BoxType::compact(*b"trex"));
/// let carried = SampleError::from(missing);
/// assert_eq!(
///     carried.kind(),
///     SampleErrorKind::Box(ErrorKind::MissingMandatoryBox)
/// );
/// assert_eq!(
///     carried.box_error().and_then(|box_error| box_error.box_type()),
///     Some(BoxType::compact(*b"trex"))
/// );
/// ```
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct SampleError {
    representation: Representation,
}

impl SampleError {
    /// Returns the failure of sample data lying behind what was read already
    #[must_use]
    pub const fn backward_data_offset(requested: u64, read_so_far: u64) -> Self {
        Self {
            representation: Representation::BackwardDataOffset {
                requested,
                read_so_far,
            },
        }
    }

    /// Returns the failure of a sample declared past the limit a reader holds
    #[must_use]
    pub const fn sample_size_limit_exceeded(track_id: u32, declared: u64, limit: u64) -> Self {
        Self {
            representation: Representation::SampleSizeLimitExceeded {
                track_id,
                declared,
                limit,
            },
        }
    }

    /// Returns the failure of a fragment carrying samples of a track the movie never declared
    #[must_use]
    pub const fn unknown_track_id(track_id: u32) -> Self {
        Self {
            representation: Representation::UnknownTrackId { track_id },
        }
    }

    /// Returns the failure of a sample whose data never arrived whole
    #[must_use]
    pub const fn unfinished_sample(track_id: u32, needed: u64, available: u64) -> Self {
        Self {
            representation: Representation::UnfinishedSample {
                track_id,
                needed,
                available,
            },
        }
    }

    /// Returns the failure of a decode time running past what 64 bits carry
    #[must_use]
    pub const fn decode_time_overflow(track_id: u32) -> Self {
        Self {
            representation: Representation::DecodeTimeOverflow { track_id },
        }
    }

    /// Returns the failure of a data offset running past what 64 bits carry
    #[must_use]
    pub const fn data_offset_overflow(track_id: u32) -> Self {
        Self {
            representation: Representation::DataOffsetOverflow { track_id },
        }
    }

    /// Returns the failure of an extent covering a different length than the data handed with it
    #[must_use]
    pub const fn extent_length_mismatch(needed: u64, available: u64) -> Self {
        Self {
            representation: Representation::ExtentLengthMismatch { needed, available },
        }
    }

    /// Returns the failure of a call made after the samples were declared over
    #[must_use]
    pub const fn already_finished() -> Self {
        Self {
            representation: Representation::AlreadyFinished,
        }
    }

    /// Returns the failure of a movie that carries no `mvex`, and so no fragments
    #[must_use]
    pub const fn missing_movie_extends() -> Self {
        Self {
            representation: Representation::MissingMovieExtends,
        }
    }

    /// Returns the failure of a sample or a close offered while no fragment is open
    #[must_use]
    pub const fn no_fragment_open() -> Self {
        Self {
            representation: Representation::NoFragmentOpen,
        }
    }

    /// Returns the failure of a fragment begun or the samples declared over while one is open
    #[must_use]
    pub const fn fragment_still_open() -> Self {
        Self {
            representation: Representation::FragmentStillOpen,
        }
    }

    /// Returns the failure of a sample longer than a `trun` row states
    #[must_use]
    pub const fn sample_size_out_of_range(track_id: u32, declared: u64) -> Self {
        Self {
            representation: Representation::SampleSizeOutOfRange { track_id, declared },
        }
    }

    /// Returns the failure of sample data lying further into a fragment than a `trun` reaches
    #[must_use]
    pub const fn data_offset_out_of_range(track_id: u32, offset: u64) -> Self {
        Self {
            representation: Representation::DataOffsetOutOfRange { track_id, offset },
        }
    }

    /// Returns the failure of a composition time offset neither version of a `trun` writes
    #[must_use]
    pub const fn composition_time_offset_out_of_range(track_id: u32, offset: i64) -> Self {
        Self {
            representation: Representation::CompositionTimeOffsetOutOfRange { track_id, offset },
        }
    }

    /// Returns the failure of a sample that does not carry on from the one before it
    #[must_use]
    pub const fn decode_time_mismatch(track_id: u32, stated: u64, reached: u64) -> Self {
        Self {
            representation: Representation::DecodeTimeMismatch {
                track_id,
                stated,
                reached,
            },
        }
    }

    /// Returns the failure of one fragment of a track described by two `stsd` entries
    #[must_use]
    pub const fn sample_description_index_mismatch(
        track_id: u32,
        stated: u32,
        established: u32,
    ) -> Self {
        Self {
            representation: Representation::SampleDescriptionIndexMismatch {
                track_id,
                stated,
                established,
            },
        }
    }

    /// Returns the failure of a fragment starting before the track it belongs to reached
    #[must_use]
    pub const fn backward_decode_time(track_id: u32, stated: u64, reached: u64) -> Self {
        Self {
            representation: Representation::BackwardDecodeTime {
                track_id,
                stated,
                reached,
            },
        }
    }

    /// Returns the failure of samples that do not build the boxes of a fragment
    #[must_use]
    pub const fn fragment_not_representable() -> Self {
        Self {
            representation: Representation::FragmentNotRepresentable,
        }
    }

    /// Returns what went wrong
    #[must_use]
    pub const fn kind(self) -> SampleErrorKind {
        match self.representation {
            Representation::Box(box_error) => SampleErrorKind::Box(box_error.kind()),
            Representation::BackwardDataOffset { .. } => SampleErrorKind::BackwardDataOffset,
            Representation::SampleSizeLimitExceeded { .. } => {
                SampleErrorKind::SampleSizeLimitExceeded
            }
            Representation::UnknownTrackId { .. } => SampleErrorKind::UnknownTrackId,
            Representation::UnfinishedSample { .. } => SampleErrorKind::UnfinishedSample,
            Representation::DecodeTimeOverflow { .. } => SampleErrorKind::DecodeTimeOverflow,
            Representation::DataOffsetOverflow { .. } => SampleErrorKind::DataOffsetOverflow,
            Representation::ExtentLengthMismatch { .. } => SampleErrorKind::ExtentLengthMismatch,
            Representation::AlreadyFinished => SampleErrorKind::AlreadyFinished,
            Representation::MissingMovieExtends => SampleErrorKind::MissingMovieExtends,
            Representation::NoFragmentOpen => SampleErrorKind::NoFragmentOpen,
            Representation::FragmentStillOpen => SampleErrorKind::FragmentStillOpen,
            Representation::SampleSizeOutOfRange { .. }
            | Representation::DataOffsetOutOfRange { .. }
            | Representation::CompositionTimeOffsetOutOfRange { .. } => SampleErrorKind::OutOfRange,
            Representation::DecodeTimeMismatch { .. } => SampleErrorKind::DecodeTimeMismatch,
            Representation::SampleDescriptionIndexMismatch { .. } => {
                SampleErrorKind::SampleDescriptionIndexMismatch
            }
            Representation::BackwardDecodeTime { .. } => SampleErrorKind::BackwardDecodeTime,
            Representation::FragmentNotRepresentable => SampleErrorKind::FragmentNotRepresentable,
        }
    }

    /// Returns what a caller does about the failure
    #[must_use]
    pub const fn category(self) -> Category {
        match self.representation {
            Representation::Box(box_error) => box_error.category(),
            Representation::BackwardDataOffset { .. }
            | Representation::SampleSizeLimitExceeded { .. }
            | Representation::SampleSizeOutOfRange { .. }
            | Representation::DataOffsetOutOfRange { .. }
            | Representation::CompositionTimeOffsetOutOfRange { .. } => Category::Unsupported,
            Representation::UnknownTrackId { .. }
            | Representation::UnfinishedSample { .. }
            | Representation::DecodeTimeOverflow { .. }
            | Representation::DataOffsetOverflow { .. } => Category::Malformed,
            Representation::ExtentLengthMismatch { .. }
            | Representation::AlreadyFinished
            | Representation::MissingMovieExtends
            | Representation::NoFragmentOpen
            | Representation::FragmentStillOpen
            | Representation::DecodeTimeMismatch { .. }
            | Representation::SampleDescriptionIndexMismatch { .. }
            | Representation::BackwardDecodeTime { .. }
            | Representation::FragmentNotRepresentable => Category::Usage,
        }
    }

    /// Returns the failure of one box the reader carried through, when it holds one
    ///
    /// The values that failure carries, and the boxes it was reached through,
    /// are read off the [`isobmff_core::Error`] itself.
    #[must_use]
    pub const fn box_error(self) -> Option<isobmff_core::Error> {
        match self.representation {
            Representation::Box(box_error) => Some(box_error),
            Representation::BackwardDataOffset { .. }
            | Representation::SampleSizeLimitExceeded { .. }
            | Representation::UnknownTrackId { .. }
            | Representation::UnfinishedSample { .. }
            | Representation::DecodeTimeOverflow { .. }
            | Representation::DataOffsetOverflow { .. }
            | Representation::ExtentLengthMismatch { .. }
            | Representation::AlreadyFinished
            | Representation::MissingMovieExtends
            | Representation::NoFragmentOpen
            | Representation::FragmentStillOpen
            | Representation::SampleSizeOutOfRange { .. }
            | Representation::DataOffsetOutOfRange { .. }
            | Representation::CompositionTimeOffsetOutOfRange { .. }
            | Representation::DecodeTimeMismatch { .. }
            | Representation::SampleDescriptionIndexMismatch { .. }
            | Representation::BackwardDecodeTime { .. }
            | Representation::FragmentNotRepresentable => None,
        }
    }

    /// Returns the value the failure names: the track it is about, or an offset
    #[must_use]
    pub const fn value(self) -> Option<u64> {
        match self.representation {
            Representation::BackwardDataOffset { requested, .. } => Some(requested),
            Representation::SampleSizeLimitExceeded { track_id, .. }
            | Representation::UnknownTrackId { track_id }
            | Representation::UnfinishedSample { track_id, .. }
            | Representation::DecodeTimeOverflow { track_id }
            | Representation::DataOffsetOverflow { track_id }
            | Representation::SampleSizeOutOfRange { track_id, .. }
            | Representation::DataOffsetOutOfRange { track_id, .. }
            | Representation::CompositionTimeOffsetOutOfRange { track_id, .. }
            | Representation::DecodeTimeMismatch { track_id, .. }
            | Representation::SampleDescriptionIndexMismatch { track_id, .. }
            | Representation::BackwardDecodeTime { track_id, .. } => Some(track_id as u64),
            Representation::Box(_)
            | Representation::ExtentLengthMismatch { .. }
            | Representation::AlreadyFinished
            | Representation::MissingMovieExtends
            | Representation::NoFragmentOpen
            | Representation::FragmentStillOpen
            | Representation::FragmentNotRepresentable => None,
        }
    }

    /// Returns the bytes the failure required, for the kinds that count bytes
    #[must_use]
    pub const fn needed_bytes(self) -> Option<u64> {
        match self.representation {
            Representation::SampleSizeLimitExceeded { declared, .. }
            | Representation::SampleSizeOutOfRange { declared, .. } => Some(declared),
            Representation::UnfinishedSample { needed, .. }
            | Representation::ExtentLengthMismatch { needed, .. } => Some(needed),
            Representation::DataOffsetOutOfRange { offset, .. } => Some(offset),
            Representation::Box(_)
            | Representation::BackwardDataOffset { .. }
            | Representation::UnknownTrackId { .. }
            | Representation::DecodeTimeOverflow { .. }
            | Representation::DataOffsetOverflow { .. }
            | Representation::AlreadyFinished
            | Representation::MissingMovieExtends
            | Representation::NoFragmentOpen
            | Representation::FragmentStillOpen
            | Representation::CompositionTimeOffsetOutOfRange { .. }
            | Representation::DecodeTimeMismatch { .. }
            | Representation::SampleDescriptionIndexMismatch { .. }
            | Representation::BackwardDecodeTime { .. }
            | Representation::FragmentNotRepresentable => None,
        }
    }

    /// Returns the bytes the failure had to hand, for the kinds that count bytes
    #[must_use]
    pub const fn available_bytes(self) -> Option<u64> {
        match self.representation {
            Representation::BackwardDataOffset { read_so_far, .. } => Some(read_so_far),
            Representation::SampleSizeLimitExceeded { limit, .. } => Some(limit),
            Representation::UnfinishedSample { available, .. }
            | Representation::ExtentLengthMismatch { available, .. } => Some(available),
            Representation::SampleSizeOutOfRange { .. } => Some(LARGEST_SAMPLE_SIZE),
            Representation::DataOffsetOutOfRange { .. } => Some(FURTHEST_DATA_OFFSET),
            Representation::Box(_)
            | Representation::UnknownTrackId { .. }
            | Representation::DecodeTimeOverflow { .. }
            | Representation::DataOffsetOverflow { .. }
            | Representation::AlreadyFinished
            | Representation::MissingMovieExtends
            | Representation::NoFragmentOpen
            | Representation::FragmentStillOpen
            | Representation::CompositionTimeOffsetOutOfRange { .. }
            | Representation::DecodeTimeMismatch { .. }
            | Representation::SampleDescriptionIndexMismatch { .. }
            | Representation::BackwardDecodeTime { .. }
            | Representation::FragmentNotRepresentable => None,
        }
    }
}

impl From<isobmff_core::Error> for SampleError {
    /// Carries the failure of one box through as it stands
    fn from(box_error: isobmff_core::Error) -> Self {
        Self {
            representation: Representation::Box(box_error),
        }
    }
}

impl fmt::Display for SampleError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.representation {
            Representation::Box(box_error) => write!(formatter, "{box_error}"),
            Representation::BackwardDataOffset {
                requested,
                read_so_far,
            } => write!(
                formatter,
                "sample data at {requested} lies behind the {read_so_far} bytes read already"
            ),
            Representation::SampleSizeLimitExceeded {
                track_id,
                declared,
                limit,
            } => write!(
                formatter,
                "track {track_id} declares a sample of {declared} bytes, past the {limit}-byte limit"
            ),
            Representation::UnknownTrackId { track_id } => {
                write!(formatter, "movie declares no track {track_id}")
            }
            Representation::UnfinishedSample {
                track_id,
                needed,
                available,
            } => write!(
                formatter,
                "sample of track {track_id} takes {needed} bytes, and {available} arrived"
            ),
            Representation::DecodeTimeOverflow { track_id } => write!(
                formatter,
                "decode time of track {track_id} runs past what 64 bits carry"
            ),
            Representation::DataOffsetOverflow { track_id } => write!(
                formatter,
                "data offset of track {track_id} runs past what 64 bits carry"
            ),
            Representation::ExtentLengthMismatch { needed, available } => write!(
                formatter,
                "extent covers {needed} bytes, and {available} arrived with it"
            ),
            Representation::AlreadyFinished => {
                formatter.write_str("samples were declared over and take nothing more")
            }
            Representation::MissingMovieExtends => formatter.write_str("the movie carries no mvex"),
            Representation::NoFragmentOpen => {
                formatter.write_str("no fragment is open to carry a sample or be closed")
            }
            Representation::FragmentStillOpen => formatter.write_str("fragment is still open"),
            Representation::SampleSizeOutOfRange { track_id, declared } => write!(
                formatter,
                "track {track_id} states a sample of {declared} bytes, past the {LARGEST_SAMPLE_SIZE} a trun row carries"
            ),
            Representation::DataOffsetOutOfRange { track_id, offset } => write!(
                formatter,
                "sample data of track {track_id} lies {offset} bytes into the fragment, past the {FURTHEST_DATA_OFFSET} a trun carries"
            ),
            Representation::CompositionTimeOffsetOutOfRange { track_id, offset } => write!(
                formatter,
                "track {track_id} states a composition time offset of {offset}, which neither version of a trun writes"
            ),
            Representation::DecodeTimeMismatch {
                track_id,
                stated,
                reached,
            } => write!(
                formatter,
                "track {track_id} states decode time {stated} where the samples before it reach {reached}"
            ),
            Representation::SampleDescriptionIndexMismatch {
                track_id,
                stated,
                established,
            } => write!(
                formatter,
                "track {track_id} states sample description index {stated} where its fragment holds {established}"
            ),
            Representation::BackwardDecodeTime {
                track_id,
                stated,
                reached,
            } => write!(
                formatter,
                "track {track_id} goes back to decode time {stated} from the {reached} it reached"
            ),
            Representation::FragmentNotRepresentable => {
                formatter.write_str("samples do not build the boxes of a fragment")
            }
        }
    }
}

impl fmt::Debug for SampleError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut fields = formatter.debug_struct("SampleError");
        fields.field("kind", &self.kind());
        fields.field("category", &self.category());

        if let Some(box_error) = self.box_error() {
            fields.field("box_error", &box_error);
        }
        if let Some(value) = self.value() {
            fields.field("value", &value);
        }
        if let Some(needed) = self.needed_bytes() {
            fields.field("needed_bytes", &needed);
        }
        if let Some(available) = self.available_bytes() {
            fields.field("available_bytes", &available);
        }

        fields.finish()
    }
}

impl error::Error for SampleError {
    /// Returns the failure of one box the reader carried through, when it holds one
    fn source(&self) -> Option<&(dyn error::Error + 'static)> {
        match &self.representation {
            Representation::Box(box_error) => Some(box_error),
            Representation::BackwardDataOffset { .. }
            | Representation::SampleSizeLimitExceeded { .. }
            | Representation::UnknownTrackId { .. }
            | Representation::UnfinishedSample { .. }
            | Representation::DecodeTimeOverflow { .. }
            | Representation::DataOffsetOverflow { .. }
            | Representation::ExtentLengthMismatch { .. }
            | Representation::AlreadyFinished
            | Representation::MissingMovieExtends
            | Representation::NoFragmentOpen
            | Representation::FragmentStillOpen
            | Representation::SampleSizeOutOfRange { .. }
            | Representation::DataOffsetOutOfRange { .. }
            | Representation::CompositionTimeOffsetOutOfRange { .. }
            | Representation::DecodeTimeMismatch { .. }
            | Representation::SampleDescriptionIndexMismatch { .. }
            | Representation::BackwardDecodeTime { .. }
            | Representation::FragmentNotRepresentable => None,
        }
    }
}

/// What a failure of the samples of a presentation is
///
/// The vocabulary is this crate's own: resolving where a sample lies, gathering
/// it, placing it on the media timeline, and laying it out in a fragment name
/// their failures here. A failure of one box is not translated: it keeps the
/// kind [`isobmff_core::ErrorKind`] gives it, carried on [`Box`](Self::Box).
///
/// The situations this layer reaches are added to as ISO/IEC 14496-12 is read
/// further, so a match on this must leave room for kinds that are not here yet.
#[non_exhaustive]
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum SampleErrorKind {
    /// Failure of one box, carried through as `isobmff-core` names it
    ///
    /// The values that failure carries, and the boxes it was reached through,
    /// are on [`box_error`](SampleError::box_error).
    Box(isobmff_core::ErrorKind),
    /// Fragment claims sample data lying behind what the reader has read
    ///
    /// The reader takes a presentation as it arrives and never reaches back,
    /// so data it has already passed is out of its reach.
    /// [`value`](SampleError::value) is the offset the claim starts at, and
    /// [`available_bytes`](SampleError::available_bytes) how far the reader has
    /// read.
    BackwardDataOffset,
    /// Fragment declares a sample past the limit the reader holds
    ///
    /// [`value`](SampleError::value) is the track it belongs to,
    /// [`needed_bytes`](SampleError::needed_bytes) the length it declares, and
    /// [`available_bytes`](SampleError::available_bytes) the length the reader
    /// gathers for one sample at most.
    SampleSizeLimitExceeded,
    /// Fragment carries samples of a track the movie never declared
    ///
    /// [`value`](SampleError::value) is the track it names.
    UnknownTrackId,
    /// Samples were declared over while the data of one had still to arrive
    ///
    /// [`value`](SampleError::value) is the track it belongs to,
    /// [`needed_bytes`](SampleError::needed_bytes) the length it takes, and
    /// [`available_bytes`](SampleError::available_bytes) the length that
    /// arrived.
    UnfinishedSample,
    /// Decode times of a track run past what 64 bits carry
    ///
    /// [`value`](SampleError::value) is the track they belong to.
    DecodeTimeOverflow,
    /// Data offsets of a track run past what 64 bits carry
    ///
    /// [`value`](SampleError::value) is the track they belong to.
    DataOffsetOverflow,
    /// Extent covers a different number of bytes than the data handed with it
    ///
    /// [`needed_bytes`](SampleError::needed_bytes) is the length the extent
    /// covers, [`available_bytes`](SampleError::available_bytes) the length the
    /// data holds.
    ExtentLengthMismatch,
    /// Samples were declared over, and take nothing more
    AlreadyFinished,
    /// Movie carries no `mvex`, and so continues in no fragments the reader could read
    MissingMovieExtends,
    /// Sample was offered, or a fragment closed, while no fragment was open
    NoFragmentOpen,
    /// Fragment was begun, or the samples declared over, while one was still open
    FragmentStillOpen,
    /// Sample states a value the field a fragment carries it in does not reach
    ///
    /// The length of a sample, how far into a fragment its data lies, and the
    /// composition time offset it states are each written into a field of fixed
    /// width, and a sample may state more than one of them carries. Which of
    /// them it was is named by the `Display` of the failure.
    /// [`value`](SampleError::value) is the track it belongs to, and for the two
    /// that count bytes, [`needed_bytes`](SampleError::needed_bytes) is the
    /// value the sample stated and
    /// [`available_bytes`](SampleError::available_bytes) the width it did not
    /// fit. A composition time offset carries neither, being no count of bytes.
    OutOfRange,
    /// Sample does not start where the one before it in its track ends
    ///
    /// A `trun` states how long a sample lasts and not when it is decoded, so
    /// the decode times of the samples of one fragment are only written if each
    /// carries on from the one before it. [`value`](SampleError::value) is the
    /// track it belongs to.
    DecodeTimeMismatch,
    /// Samples of one fragment of one track are described by two `stsd` entries
    ///
    /// A `traf` states which entry describes its samples once, for all of them.
    /// [`value`](SampleError::value) is the track they belong to.
    SampleDescriptionIndexMismatch,
    /// Fragment of a track starts before the samples written for it reach
    ///
    /// A `tfdt` is the sum of the durations of the samples before it
    /// (ISO/IEC 14496-12 §8.8.12), which a decode time going back cannot be.
    /// [`value`](SampleError::value) is the track it belongs to.
    BackwardDecodeTime,
    /// Samples do not build the boxes of a fragment
    ///
    /// The writer holds the samples it is given to what the boxes of a fragment
    /// state, so this is never reached from samples a caller offers.
    FragmentNotRepresentable,
}

/// Values a failure carries, keyed by what went wrong
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
enum Representation {
    /// Failure of one box, carried through whole
    Box(isobmff_core::Error),
    /// Sample data lying behind what was read already
    BackwardDataOffset { requested: u64, read_so_far: u64 },
    /// Sample declared past the limit a reader holds
    SampleSizeLimitExceeded {
        track_id: u32,
        declared: u64,
        limit: u64,
    },
    /// Fragment carrying samples of a track the movie never declared
    UnknownTrackId { track_id: u32 },
    /// Sample whose data never arrived whole
    UnfinishedSample {
        track_id: u32,
        needed: u64,
        available: u64,
    },
    /// Decode time running past what 64 bits carry
    DecodeTimeOverflow { track_id: u32 },
    /// Data offset running past what 64 bits carry
    DataOffsetOverflow { track_id: u32 },
    /// Extent covering a different length than the data handed with it
    ExtentLengthMismatch { needed: u64, available: u64 },
    /// Call made after the samples were declared over
    AlreadyFinished,
    /// Movie carrying no `mvex`, and so no fragments
    MissingMovieExtends,
    /// Sample offered, or a fragment closed, while no fragment was open
    NoFragmentOpen,
    /// Fragment begun, or the samples declared over, while one was still open
    FragmentStillOpen,
    /// Sample longer than a `trun` row states
    SampleSizeOutOfRange { track_id: u32, declared: u64 },
    /// Sample data lying further into a fragment than a `trun` reaches
    DataOffsetOutOfRange { track_id: u32, offset: u64 },
    /// Composition time offset neither version of a `trun` writes
    CompositionTimeOffsetOutOfRange { track_id: u32, offset: i64 },
    /// Sample that does not carry on from the one before it
    DecodeTimeMismatch {
        track_id: u32,
        stated: u64,
        reached: u64,
    },
    /// Fragment of one track described by two `stsd` entries
    SampleDescriptionIndexMismatch {
        track_id: u32,
        stated: u32,
        established: u32,
    },
    /// Fragment starting before the track it belongs to reached
    BackwardDecodeTime {
        track_id: u32,
        stated: u64,
        reached: u64,
    },
    /// Samples that do not build the boxes of a fragment
    FragmentNotRepresentable,
}

#[cfg(test)]
mod tests {
    use alloc::format;
    use alloc::string::ToString as _;

    use isobmff_core::{BoxType, Category};

    use super::SampleError;

    #[test]
    fn a_kind_falls_in_the_category_its_situation_asks_for() {
        assert_eq!(
            SampleError::backward_data_offset(64, 128).category(),
            Category::Unsupported
        );
        assert_eq!(
            SampleError::sample_size_limit_exceeded(1, 32, 16).category(),
            Category::Unsupported
        );
        assert_eq!(
            SampleError::unknown_track_id(3).category(),
            Category::Malformed
        );
        assert_eq!(SampleError::already_finished().category(), Category::Usage);
        assert_eq!(
            SampleError::from(isobmff_core::Error::unsupported_version(2)).category(),
            Category::Unsupported
        );
    }

    #[test]
    fn a_failure_carries_only_the_values_its_kind_names() {
        let error = SampleError::unfinished_sample(2, 1_024, 512);

        assert_eq!(error.value(), Some(2));
        assert_eq!(error.needed_bytes(), Some(1_024));
        assert_eq!(error.available_bytes(), Some(512));
        assert_eq!(error.box_error(), None);
        assert_eq!(SampleError::already_finished().value(), None);
        assert_eq!(SampleError::unknown_track_id(3).needed_bytes(), None);
    }

    #[test]
    fn a_failure_of_one_box_keeps_its_values_and_the_boxes_it_was_reached_through() {
        let box_error = isobmff_core::Error::missing_mandatory_box(BoxType::compact(*b"trex"))
            .in_container(BoxType::compact(*b"mvex"));
        let carried = SampleError::from(box_error);

        assert_eq!(carried.box_error(), Some(box_error));
        assert_eq!(carried.value(), None);
        assert_eq!(carried.needed_bytes(), None);
    }

    #[test]
    fn display_of_a_failure_of_the_samples_states_the_reason() {
        assert_eq!(
            SampleError::backward_data_offset(64, 128).to_string(),
            "sample data at 64 lies behind the 128 bytes read already"
        );
        assert_eq!(
            SampleError::sample_size_limit_exceeded(1, 32, 16).to_string(),
            "track 1 declares a sample of 32 bytes, past the 16-byte limit"
        );
        assert_eq!(
            SampleError::unknown_track_id(3).to_string(),
            "movie declares no track 3"
        );
        assert_eq!(
            SampleError::unfinished_sample(2, 1_024, 512).to_string(),
            "sample of track 2 takes 1024 bytes, and 512 arrived"
        );
        assert_eq!(
            SampleError::decode_time_overflow(1).to_string(),
            "decode time of track 1 runs past what 64 bits carry"
        );
        assert_eq!(
            SampleError::data_offset_overflow(1).to_string(),
            "data offset of track 1 runs past what 64 bits carry"
        );
        assert_eq!(
            SampleError::extent_length_mismatch(6, 4).to_string(),
            "extent covers 6 bytes, and 4 arrived with it"
        );
        assert_eq!(
            SampleError::already_finished().to_string(),
            "samples were declared over and take nothing more"
        );
        assert_eq!(
            SampleError::missing_movie_extends().to_string(),
            "the movie carries no mvex"
        );
    }

    #[test]
    fn display_of_a_failure_of_one_box_reads_as_that_failure() {
        let box_error = isobmff_core::Error::missing_mandatory_box(BoxType::compact(*b"mvex"));

        assert_eq!(
            SampleError::from(box_error).to_string(),
            box_error.to_string()
        );
    }

    #[test]
    fn debug_leaves_out_the_values_a_kind_does_not_carry() {
        let error = SampleError::already_finished();

        assert_eq!(
            format!("{error:?}"),
            "SampleError { kind: AlreadyFinished, category: Usage }"
        );
    }

    #[test]
    fn debug_names_the_values_a_kind_carries() {
        let error = SampleError::sample_size_limit_exceeded(1, 32, 16);

        assert_eq!(
            format!("{error:?}"),
            "SampleError { kind: SampleSizeLimitExceeded, category: Unsupported, value: 1, needed_bytes: 32, available_bytes: 16 }"
        );
    }
}
