//! [`DemuxerConfig`], the limits a demuxer holds what it reads to

/// The limits a [`Demuxer`](super::Demuxer) holds what it reads to
///
/// Each bounds an allocation whose size the input declares. Build one from the
/// default and set the fields that differ:
///
/// ```
/// use isobmff::{Demuxer, DemuxerConfig};
///
/// let mut config = DemuxerConfig::default();
/// config.sample_size_limit = 64 * 1024 * 1024;
/// let demuxer = Demuxer::new(config);
/// ```
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct DemuxerConfig {
    /// The most bytes the payload of a box read into a value, the `ftyp` or
    /// the `moov`, may hold (16 MiB by default)
    ///
    /// A `moov` grows with the samples it declares, by some bytes a sample;
    /// 16 MiB holds that of a movie of hours.
    pub payload_limit: u64,
    /// The most sample locations the demuxer holds at once, those of the
    /// `moov` or the `moof` it reads among them (2^20 by default)
    ///
    /// The demuxer holds the location of each, some tens of bytes; 2^20 of
    /// them take some tens of MiB. A `moof` whose samples would pass it is
    /// reported and not resolved, and the decoding times of its tracks do
    /// not move on. The samples that wait for an `imda` not yet read are
    /// held, as many again at most, until it is.
    pub sample_count_limit: u64,
    /// The most bytes a sample of a subscribed track may hold (16 MiB by
    /// default)
    ///
    /// A sample is a frame of video or audio; even a frame of 4K video coded
    /// as a sync sample takes a few MiB.
    pub sample_size_limit: u64,
    /// The most entries the `tfra`s of an `mfra` may declare, of all its
    /// tracks (2^16 by default, as a muxer writes at most by default)
    ///
    /// The demuxer holds some tens of bytes an entry once it reads the box;
    /// 2^16 of them take some MiB, and list the sync samples of hours of
    /// fragments. An `mfra` that declares more is reported and not read.
    pub random_access_entry_limit: u64,
    /// The most `imda` the demuxer remembers where it lies, ISO/IEC 14496-12
    /// §8.1.4 (2^16 by default)
    ///
    /// The demuxer remembers the identifier and the offset of each `imda`
    /// until the input ends, some tens of bytes an `imda` with what holds
    /// them; 2^16 of them take some MiB, and are one `imda` a movie fragment
    /// of a few seconds for a day. An `imda` past the limit is reported and
    /// not remembered, so the samples that lie in it are not resolved.
    pub identified_media_data_limit: u64,
}

impl Default for DemuxerConfig {
    // Why not derive Default: it would set every limit to 0, under which no
    // `moov` and no sample could be read.
    fn default() -> Self {
        Self {
            payload_limit: 16 * 1024 * 1024,
            sample_count_limit: 1 << 20,
            sample_size_limit: 16 * 1024 * 1024,
            random_access_entry_limit: 1 << 16,
            identified_media_data_limit: 1 << 16,
        }
    }
}
