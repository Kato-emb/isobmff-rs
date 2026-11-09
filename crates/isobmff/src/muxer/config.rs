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
    /// The most bytes the muxer holds for `poll_output` (16 MiB by default)
    ///
    /// A caller that polls after each sample needs room for its largest
    /// sample: 16 MiB holds any sample the demuxer lends by default, and a
    /// frame of 4K video coded as a sync sample takes a few MiB.
    pub buffer_limit: usize,
    /// The most samples the muxer takes of one track (2^20 by default)
    ///
    /// The muxer keeps some tens of bytes of each until it finishes, so 2^20
    /// of them take some tens of MiB; at 30 a second, they last more than
    /// nine hours.
    pub sample_count_limit: u64,
}

impl Default for MuxerConfig {
    // Why not derive Default: it would set every limit to 0, under which no
    // sample could be taken.
    fn default() -> Self {
        Self {
            buffer_limit: 16 * 1024 * 1024,
            sample_count_limit: 1 << 20,
        }
    }
}
