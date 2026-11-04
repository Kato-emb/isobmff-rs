//! [`FileStructure`], which of the structures of ISO/IEC 14496-12 a file has, and the options a [`Muxer`](crate::Muxer) writes each with

/// Structure of a file: a movie file that is not fragmented, a fragmented movie file, or a segment, ISO/IEC 14496-12 §4.1, A.10, §8.14
///
/// A [`Muxer`](crate::Muxer) is made for one of them, with the options of
/// that structure, and returns it from
/// [`file_structure`](crate::Muxer::file_structure). Each call of
/// [`Muxer::handle_samples`](crate::Muxer::handle_samples) writes its
/// samples into one `mdat` (§8.1.1), declared by a `moof` before it, or by
/// the sample tables of the `moov` at the end of the file.
#[non_exhaustive]
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum FileStructure {
    /// Movie file that is not fragmented: the `ftyp`, an `mdat` for each call, and the `moov` at the end
    NonFragmented,
    /// Fragmented movie file, A.10: the `ftyp` and the `moov`, then a `moof` and an `mdat` for each call
    Fragmented,
    /// Media segment, §8.14: the `styp`, then a `moof` and an `mdat` for each call, of the movie its initialization segment holds
    ///
    /// The initialization segment is the fragmented movie file of the
    /// `ftyp` and the `moov` alone.
    Segment,
}

/// Options of a movie file that is not fragmented, as a [`Muxer`](crate::Muxer) writes it
///
/// The sample table of a track counts decoding times from 0 at its first
/// sample (§8.6.1.2). The first sample of a track without an edit list is at
/// decoding time 0; that of a track with one may be at any, and the edits of
/// the track have their `media_time` moved back by as much, so that they
/// present the same media.
///
/// # Examples
///
/// ```
/// use isobmff::{Muxer, NonFragmentedOptions};
///
/// let muxer = Muxer::non_fragmented(NonFragmentedOptions::default());
/// assert_eq!(muxer.file_structure(), isobmff::FileStructure::NonFragmented);
/// ```
#[non_exhaustive]
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default)]
pub struct NonFragmentedOptions {}

/// Options of a fragmented movie file, as a [`Muxer`](crate::Muxer) or a [`MovieFragmentWriter`](crate::structure_data::MovieFragmentWriter) writes it, A.10
///
/// # Examples
///
/// ```
/// use isobmff::{FragmentedOptions, Muxer};
///
/// // Fragments numbered from 1, the file ending with an `mfra`
/// let mut options = FragmentedOptions::default();
/// options.movie_fragment_random_access = true;
/// let muxer = Muxer::fragmented(options);
/// assert_eq!(muxer.file_structure(), isobmff::FileStructure::Fragmented);
/// ```
#[non_exhaustive]
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct FragmentedOptions {
    /// `sequence_number` of the first `moof`, §8.8.5; 1 by default
    pub first_sequence_number: u32,
    /// Whether the file ends with an `mfra` that lists the first sync sample of each track in each `moof`, §8.8.9; not by default
    ///
    /// A track none of whose fragments has a sync sample has no `tfra`. A
    /// file of compressed boxes has no `mfra` (§8.16.1); this crate writes
    /// no compressed box.
    pub movie_fragment_random_access: bool,
}

impl Default for FragmentedOptions {
    fn default() -> Self {
        Self {
            first_sequence_number: 1,
            movie_fragment_random_access: false,
        }
    }
}

/// Options of a media segment, as a [`Muxer`](crate::Muxer) or a [`MovieFragmentWriter`](crate::structure_data::MovieFragmentWriter) writes it, §8.14
///
/// # Examples
///
/// ```
/// use isobmff::{Muxer, SegmentOptions};
///
/// // Fragments numbered from 1, the segment indexed by a `sidx`
/// let mut options = SegmentOptions::default();
/// options.segment_index = true;
/// let muxer = Muxer::segment(options);
/// assert_eq!(muxer.file_structure(), isobmff::FileStructure::Segment);
/// ```
#[non_exhaustive]
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct SegmentOptions {
    /// `sequence_number` of the first `moof`, §8.8.5; 1 by default
    pub first_sequence_number: u32,
    /// Whether a `sidx` after the `styp` indexes the segment, each `moof` and its `mdat` a subsegment of the samples of the first track of the movie, §8.14.3; not by default
    ///
    /// The times are the presentation times the edit list of the first
    /// track maps the composition times of its samples to, as a
    /// [`Timeline`](crate::timeline::Timeline) maps them.
    ///
    /// The `sidx` states the bytes of what follows it, so the muxer holds
    /// the boxes of the segment back until
    /// [`finish`](crate::Muxer::finish) writes it, and returns them after
    /// it: the segment is in memory whole.
    pub segment_index: bool,
}

impl Default for SegmentOptions {
    fn default() -> Self {
        Self {
            first_sequence_number: 1,
            segment_index: false,
        }
    }
}
