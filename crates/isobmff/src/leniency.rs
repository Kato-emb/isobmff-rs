//! [`Leniency`], what a reader read past in a file rather than refuse it

use core::fmt;

use isobmff_core::BoxType;

/// Disagreement in a file that a reader read past, as conforming readers do, rather than refuse the file
///
/// A reader keeps each leniency it took, so a caller can ask after the fact
/// what in the file did not hold together, and how the reader read it.
///
/// # Examples
///
/// ```
/// use isobmff::{Leniency, SampleTableDisagreement};
///
/// fn report(leniency: &Leniency) {
///     match leniency {
///         Leniency::SampleTable {
///             track_id,
///             box_type,
///             kind: SampleTableDisagreement::Short { missing_samples, .. },
///             ..
///         } => println!("{box_type} of track {track_id} is {missing_samples} samples short"),
///         other => println!("{other}"),
///     }
/// }
/// ```
#[non_exhaustive]
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub enum Leniency {
    /// Table of the sample table of a track that does not agree with the samples the track has
    ///
    /// The samples of a track are those the `stsc` and the `stco` or the
    /// `co64` lay out in chunks, as far as the `stsz` or the `stz2` gives
    /// them sizes. A table short of them leaves the samples past its end
    /// the last duration of the `stts`, or zero for the values the other
    /// tables state. A table past them has the rest of its entries left out,
    /// and so has an `stss` the sample numbers it marks out of order.
    SampleTable {
        /// `track_ID` of the track
        track_id: u32,
        /// Type of the box of the table
        box_type: BoxType,
        /// How the table disagrees with the samples of the track
        kind: SampleTableDisagreement,
    },
    /// Track fragment whose `tfdt` gives a decoding time before the one the earlier samples of its track reach
    ///
    /// ISO/IEC 14496-12 §8.8.12 does not allow it. The first sample of the
    /// track fragment takes the decoding time of the `tfdt`, as FFmpeg reads
    /// it, so the samples of the track overlap by as much.
    DecodingTimeOverlap {
        /// `track_ID` of the track
        track_id: u32,
        /// Ticks of the media timescale by which the `tfdt` is before the decoding time the earlier samples reach
        overlap: u64,
    },
    /// Track fragment whose `tfdt` gives a decoding time past the one the earlier samples of its track reach
    ///
    /// ISO/IEC 14496-12 §8.8.12 extends the duration of the sample before
    /// the track fragment to the decoding time its `tfdt` gives. The reader
    /// has returned that sample with the duration it declares, so its first
    /// sample of the track fragment takes the decoding time of the `tfdt`,
    /// and the extension is the gap between the two.
    DecodingTimeGap {
        /// `track_ID` of the track
        track_id: u32,
        /// Ticks of the media timescale by which the `tfdt` is past the decoding time the earlier samples reach
        gap: u64,
    },
}

/// How a table of a sample table disagrees with the samples of its track
#[non_exhaustive]
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum SampleTableDisagreement {
    /// The table states fewer samples than the track has
    Short {
        /// Number of samples past the end of the table
        missing_samples: u64,
    },
    /// The table states samples past the last one of the track
    Long {
        /// Number of samples the table states past the last one of the track
        excess_samples: u64,
    },
    /// The `stss` marks sample numbers that do not follow the one it marks before them in increasing order
    Unordered {
        /// Number of sample numbers marked out of order
        sample_numbers: u64,
    },
}

impl fmt::Display for Leniency {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::SampleTable {
                track_id,
                box_type,
                kind: SampleTableDisagreement::Short { missing_samples },
            } => write!(
                formatter,
                "{box_type} of track {track_id} is {missing_samples} samples short of the track"
            ),
            Self::SampleTable {
                track_id,
                box_type,
                kind: SampleTableDisagreement::Long { excess_samples },
            } => write!(
                formatter,
                "{box_type} of track {track_id} states {excess_samples} samples past the last of the track"
            ),
            Self::SampleTable {
                track_id,
                box_type,
                kind: SampleTableDisagreement::Unordered { sample_numbers },
            } => write!(
                formatter,
                "{box_type} of track {track_id} marks {sample_numbers} sample numbers out of order"
            ),
            Self::DecodingTimeOverlap { track_id, overlap } => write!(
                formatter,
                "tfdt of track {track_id} is {overlap} ticks before the decoding time its samples reach"
            ),
            Self::DecodingTimeGap { track_id, gap } => write!(
                formatter,
                "tfdt of track {track_id} is {gap} ticks past the decoding time its samples reach"
            ),
        }
    }
}
