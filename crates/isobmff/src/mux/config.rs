//! [`MuxerConfig`], the limits a muxer holds what it takes to

/// The limits a muxer holds what it takes to
///
/// Build one from the default and set the fields that differ:
///
/// ```
/// use isobmff::MuxerConfig;
///
/// let mut config = MuxerConfig::default();
/// config.sample_count_limit = 1 << 24;
/// ```
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct MuxerConfig {
    /// The most bytes the muxer holds for `poll_output`, those that open the
    /// file (the `ftyp`, and the header of the `mdat` or the `moov`) among
    /// them, and the bytes of the samples of the movie fragment being written
    /// (16 MiB by default)
    ///
    /// A caller that polls after each sample needs room for its largest
    /// sample: 16 MiB holds any sample the demuxer lends by default, and a
    /// frame of 4K video coded as a sync sample takes a few MiB. A movie
    /// fragment holds its samples until it ends, so the limit bounds its
    /// length too: 16 MiB holds some seconds of 4K video.
    pub buffer_limit: usize,
    /// The most samples the muxer takes of one track, or of one movie
    /// fragment (2^20 by default)
    ///
    /// The muxer keeps some tens of bytes of each until it finishes, or ends
    /// the fragment, so 2^20 of them take some tens of MiB; at 30 a second,
    /// they last more than nine hours.
    pub sample_count_limit: u64,
    /// The most entries the random access table of a
    /// [`FragmentedMovieFileMuxer`](super::FragmentedMovieFileMuxer) holds, of
    /// all its tracks (2^16 by default)
    ///
    /// The table names the first sync sample of each track in each movie
    /// fragment. Past the limit, the muxer records no more entries and goes
    /// on writing fragments: the table is a hint, which need not list every
    /// sync sample, ISO/IEC 14496-12 §8.8.10. The muxer holds some tens of
    /// bytes an entry until it finishes, and some more while it writes the
    /// table, so 2^16 of them take some MiB; at a fragment of two tracks every
    /// two seconds, they last some eighteen hours.
    pub random_access_entry_limit: usize,
}

impl Default for MuxerConfig {
    // Why not derive Default: it would set every limit to 0, under which no
    // sample could be taken.
    fn default() -> Self {
        Self {
            buffer_limit: 16 * 1024 * 1024,
            sample_count_limit: 1 << 20,
            random_access_entry_limit: 1 << 16,
        }
    }
}
