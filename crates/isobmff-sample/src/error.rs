//! [`SampleError`], the reason the samples of a presentation do not resolve

use isobmff_core::Category;

mod kind;
mod report;

use crate::error::kind::Representation;
pub use kind::SampleErrorKind;

/// Bytes a `trun` row states a sample of at most, ISO/IEC 14496-12 §8.8.8
const LARGEST_SAMPLE_SIZE: u64 = u32::MAX as u64;

/// Bytes into a fragment a `trun` states its data at most, ISO/IEC 14496-12 §8.8.8
const FURTHEST_DATA_OFFSET: u64 = i32::MAX as u64;

/// Reason the samples of a presentation do not resolve
///
/// What went wrong is one [`kind`](Self::kind): a failure of the samples
/// themselves — a fragment of a track the movie never declared, a timeline or
/// an offset run past what its field carries, a sample that never arrived
/// whole — or a failure of one box, which
/// [`isobmff_core::Error`] names and this type carries through whole, as
/// [`box_error`](Self::box_error). What a caller does about either is one
/// [`category`](Self::category).
///
/// The values a failure of the samples carries follow from its kind, and each
/// kind names its own on [`SampleErrorKind`]. A carried box failure keeps its
/// values and its container path on [`box_error`](Self::box_error), so the
/// accessors here report `None` for it.
///
/// # Examples
///
/// ```
/// use isobmff_core::{BoxType, Category};
/// use isobmff_sample::{SampleError, SampleErrorKind};
///
/// // A failure of the samples names its own kind
/// let failure = SampleError::decode_time_overflow(3);
/// assert_eq!(failure.kind(), SampleErrorKind::DecodeTimeOverflow);
/// assert_eq!(failure.category(), Category::Malformed);
/// assert_eq!(failure.track_id(), Some(3));
/// assert_eq!(failure.box_error(), None);
///
/// // A failure of one box is carried through whole
/// let missing = isobmff_core::Error::missing_mandatory_box(BoxType::compact(*b"trex"));
/// let carried = SampleError::from(missing);
/// assert_eq!(
///     carried.kind(),
///     SampleErrorKind::Box(isobmff_core::ErrorKind::MissingMandatoryBox)
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

    /// Returns the failure of a fragment carrying samples of a track the movie never declared
    #[must_use]
    pub const fn unknown_track_id(track_id: u32) -> Self {
        Self {
            representation: Representation::UnknownTrackId { track_id },
        }
    }

    /// Returns the failure of samples described by an `stsd` entry their track has none of
    #[must_use]
    pub const fn unknown_sample_description_index(
        track_id: u32,
        sample_description_index: u32,
    ) -> Self {
        Self {
            representation: Representation::UnknownSampleDescriptionIndex {
                track_id,
                sample_description_index,
            },
        }
    }

    /// Returns the failure of a movie that carries no `mvex`, and so no fragments
    #[must_use]
    pub const fn missing_movie_extends() -> Self {
        Self {
            representation: Representation::MissingMovieExtends,
        }
    }

    /// Returns the failure of an `stsd` entry naming a `dref` entry the track has none of
    #[must_use]
    pub const fn unknown_data_reference_index(track_id: u32, data_reference_index: u16) -> Self {
        Self {
            representation: Representation::UnknownDataReferenceIndex {
                track_id,
                data_reference_index,
            },
        }
    }

    /// Returns the failure of a `dref` entry naming a resource other than the file itself
    #[must_use]
    pub const fn external_data_reference(track_id: u32, data_reference_index: u16) -> Self {
        Self {
            representation: Representation::ExternalDataReference {
                track_id,
                data_reference_index,
            },
        }
    }

    /// Returns the failure of the sample tables of a track counting different numbers of samples
    #[must_use]
    pub const fn sample_count_mismatch(track_id: u32) -> Self {
        Self {
            representation: Representation::SampleCountMismatch { track_id },
        }
    }

    /// Returns the failure of a run of chunks starting at a chunk outside the range open to it
    #[must_use]
    pub const fn first_chunk_out_of_range(track_id: u32, first_chunk: u32) -> Self {
        Self {
            representation: Representation::FirstChunkOutOfRange {
                track_id,
                first_chunk,
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

    /// Returns the failure of a sample whose bytes never arrived whole
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

    /// Returns the failure of a call made after the samples were declared over
    #[must_use]
    pub const fn already_finished() -> Self {
        Self {
            representation: Representation::AlreadyFinished,
        }
    }

    /// Returns the failure of a sample handed over, or a fragment closed, while no fragment was open
    #[must_use]
    pub const fn no_fragment_open() -> Self {
        Self {
            representation: Representation::NoFragmentOpen,
        }
    }

    /// Returns the failure of a fragment begun while the one before it was still open
    #[must_use]
    pub const fn fragment_still_open() -> Self {
        Self {
            representation: Representation::FragmentStillOpen,
        }
    }

    /// Returns the failure of a sample longer than the field a `trun` row states its length in reaches
    #[must_use]
    pub const fn sample_size_out_of_range(track_id: u32, declared: u64) -> Self {
        Self {
            representation: Representation::SampleSizeOutOfRange { track_id, declared },
        }
    }

    /// Returns the failure of a sample lying further into its fragment than a `trun` offset reaches
    #[must_use]
    pub const fn data_offset_out_of_range(track_id: u32, offset: u64) -> Self {
        Self {
            representation: Representation::DataOffsetOutOfRange { track_id, offset },
        }
    }

    /// Returns the failure of a sample stating a composition time offset neither version of a `trun` writes
    #[must_use]
    pub const fn composition_time_offset_out_of_range(track_id: u32, offset: i64) -> Self {
        Self {
            representation: Representation::CompositionTimeOffsetOutOfRange { track_id, offset },
        }
    }

    /// Returns the failure of a sample stating `stated` where the samples of its track before it reach `reached`
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

    /// Returns the failure of a fragment starting a track at `stated`, before the `reached` its samples written reach
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

    /// Returns the failure of a sample described by entry `stated` in a fragment describing its track by `established`
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
        self.representation.kind()
    }

    /// Returns what a caller does about the failure
    #[must_use]
    pub const fn category(self) -> Category {
        self.representation.category()
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
