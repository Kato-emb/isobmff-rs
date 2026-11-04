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
/// use isobmff_mux::{Muxer, NonFragmentedOptions};
///
/// let muxer = Muxer::non_fragmented(NonFragmentedOptions::default());
/// assert_eq!(muxer.file_structure(), isobmff_mux::FileStructure::NonFragmented);
/// ```
#[non_exhaustive]
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default)]
pub struct NonFragmentedOptions {
    /// Whether the muxer writes the samples as movie fragments, and lays the file out again as one that is not fragmented as it finishes; not by default
    ///
    /// Until [`finish`](crate::Muxer::finish), the file written is a
    /// fragmented movie file (A.10): the `ftyp`, a 16-byte `free`, the `moov`
    /// of the movie fragments, then a `moof` and an `mdat` for each call, so
    /// that a file cut short by a crash reads to its last fragment. As it
    /// finishes, the muxer writes the `moov` of every sample at the end, and
    /// then turns the `free` into the header of an `mdat` that holds all the
    /// fragments (§8.1.1): readers take the file as one that is not
    /// fragmented, and the fragments as media data. FFmpeg calls this
    /// `hybrid_fragmented`, OBS a hybrid MP4.
    ///
    /// The brands of the file name one of `iso5` to `isod`, as those of a
    /// fragmented movie file do (§8.8.7.1). A file whose samples the sample
    /// tables cannot declare, or that holds more samples than
    /// [`MuxLimits::held_location_count`](crate::MuxLimits::held_location_count),
    /// is not laid out again: [`finish`](crate::Muxer::finish) fails, and the
    /// file stays the fragmented movie file written.
    pub recoverable: bool,
}
