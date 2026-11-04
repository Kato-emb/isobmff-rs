//! [`FragmentedOptions`] and [`SegmentOptions`], what a [`MovieFragmentWriter`](super::MovieFragmentWriter) writes a fragmented movie file and a media segment with

/// Options of a fragmented movie file, as a [`MovieFragmentWriter`](super::MovieFragmentWriter) writes it, A.10
///
/// # Examples
///
/// ```
/// use isobmff::boxes::MovieBox;
/// use isobmff::structure_data::{FragmentedOptions, MovieFragmentWriter};
///
/// // Fragments numbered from 1, the file ending with an `mfra`
/// let mut options = FragmentedOptions::default();
/// options.movie_fragment_random_access = true;
/// let fragments = MovieFragmentWriter::fragmented(MovieBox::default(), options)?;
/// assert!(fragments.movie().mvex.is_some());
/// # Ok::<(), Box<dyn core::error::Error>>(())
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

/// Options of a media segment, as a [`MovieFragmentWriter`](super::MovieFragmentWriter) writes it, §8.14
///
/// # Examples
///
/// ```
/// use isobmff::boxes::{MovieBox, TrackBox};
/// use isobmff::structure_data::{MovieFragmentWriter, SegmentOptions};
///
/// // Fragments numbered from 1, the segment of 4 subsegments indexed by a `sidx`
/// let mut options = SegmentOptions::default();
/// options.segment_index = Some(4);
/// let mut movie = MovieBox::default();
/// movie.trak.push(TrackBox::default());
/// let fragments = MovieFragmentWriter::segment(movie, options)?;
/// assert!(fragments.segment_index_len().is_some());
/// # Ok::<(), Box<dyn core::error::Error>>(())
/// ```
#[non_exhaustive]
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct SegmentOptions {
    /// `sequence_number` of the first `moof`, §8.8.5; 1 by default
    pub first_sequence_number: u32,
    /// Number of subsegments, each `moof` and its `mdat`, of the first track of the movie, that a `sidx` after the `styp` indexes, or `None` for no `sidx`, §8.14.3; none by default
    ///
    /// The number is that of the calls of
    /// [`movie_fragment`](super::MovieFragmentWriter::movie_fragment): a
    /// writer reserves the
    /// [`segment_index_len`](super::MovieFragmentWriter::segment_index_len)
    /// bytes of the `sidx` before the first `moof`, and writes the `sidx`
    /// [`finish`](super::MovieFragmentWriter::finish) returns over them, so
    /// that the segment is never held in memory. A fragment past the number
    /// is refused, and so is a finish after fewer. A `sidx` references up to
    /// 65,535 subsegments.
    ///
    /// The times are the presentation times the edit list of the first
    /// track maps the composition times of its samples to, as a
    /// [`Timeline`](crate::timeline::Timeline) maps them.
    pub segment_index: Option<u32>,
}

impl Default for SegmentOptions {
    fn default() -> Self {
        Self {
            first_sequence_number: 1,
            segment_index: None,
        }
    }
}
