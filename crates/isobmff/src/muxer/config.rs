//! [`MuxerConfig`], the limit a muxer holds what it takes to

/// The limit a [`Muxer`](super::Muxer) holds what it takes to
///
/// A muxer keeps what the sample tables declare of each sample until it
/// finishes. Build one from the default and set the fields that differ:
///
/// ```
/// use isobmff::{Muxer, MuxerConfig};
///
/// let mut config = MuxerConfig::default();
/// config.sample_count_limit = 1 << 24;
/// ```
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct MuxerConfig {
    /// The most samples the muxer takes of one track (2^20 by default)
    ///
    /// The muxer keeps some tens of bytes of each, so 2^20 of them take some
    /// tens of MiB; at 30 a second, they last more than nine hours.
    pub sample_count_limit: u64,
}

impl Default for MuxerConfig {
    // Why not derive Default: it would set the limit to 0, under which no
    // sample could be taken.
    fn default() -> Self {
        Self {
            sample_count_limit: 1 << 20,
        }
    }
}
