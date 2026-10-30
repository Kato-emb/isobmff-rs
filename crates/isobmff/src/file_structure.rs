//! [`FileStructure`], how a [`Muxer`](crate::Muxer) lays out the file it writes

/// How a [`Muxer`](crate::Muxer) lays out the samples it writes, and the box that declares them
///
/// Each call of [`Muxer::handle_samples`](crate::Muxer::handle_samples)
/// writes its samples into one `mdat` (ISO/IEC 14496-12 §8.1.1), declared
/// by a `moof` before it, or by the sample tables of the `moov` at the end
/// of the file.
#[non_exhaustive]
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum FileStructure {
    /// Fragmented movie file, A.10: the `ftyp` and the `moov`, then a `moof` and an `mdat` for each call
    Fragmented {
        /// `sequence_number` of the first `moof`, §8.8.5
        first_sequence_number: u32,
    },
    /// Movie file that is not fragmented: the `ftyp`, an `mdat` for each call, and the `moov` at the end
    NonFragmented,
    /// Media segment, §8.14: the `styp`, then a `moof` and an `mdat` for each call, of the movie its initialization segment holds
    ///
    /// The initialization segment is the file
    /// [`Fragmented`](Self::Fragmented) writes of the `ftyp` and the `moov`
    /// alone.
    Segment {
        /// `sequence_number` of the first `moof`, §8.8.5
        first_sequence_number: u32,
    },
}
