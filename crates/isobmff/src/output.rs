//! [`Output`], the bytes a writer returns and the file offset they go to

use alloc::vec::Vec;

/// Bytes a writer returns, and the file offset they go to
///
/// The offset counts from the first byte of the file the writer writes. A
/// writer returns its output in the order the caller is to write it. The
/// caller keeps where the file ends, and writes each output there by its
/// offset:
///
/// * an offset at the end of the file extends the file with the bytes;
/// * an offset before the end, the bytes ending no later than it, writes the
///   bytes over those the file holds there;
/// * any other offset is a failure of the writer.
///
/// A caller that cannot seek, such as one writing to a pipe, takes outputs
/// whose offset is the end of the file, and stops at the first that is not.
/// A [`Muxer`](crate::Muxer) returns only outputs at the end of the file.
///
/// # Examples
///
/// ```
/// # use isobmff::boxes::{DataEntry, DataEntryUrlBox, MovieBox, SampleDescription, SampleEntry, TrackBox};
/// use isobmff::{FourCc, Muxer, NonFragmentedOptions, Sample, SampleProperties};
///
/// # let mut trak = TrackBox::default();
/// # trak.tkhd.track_id = 1;
/// # trak.mdia.mdhd.timescale = 1000;
/// # let mut entry = SampleEntry::default();
/// # entry.format = FourCc::new(*b"mp4a");
/// # entry.data_reference_index = 1;
/// # trak.mdia.minf.stbl.stsd.sample_descriptions = vec![SampleDescription::Typed(entry)];
/// # trak.mdia.minf.dinf.dref.data_entries = vec![DataEntry::Url(DataEntryUrlBox::default())];
/// # let mut movie = MovieBox::default();
/// # movie.mvhd.timescale = 1000;
/// # movie.mvhd.next_track_id = 2;
/// # movie.trak = vec![trak];
/// # let properties = SampleProperties {
/// #     track_id: 1,
/// #     decoding_time: 0,
/// #     sample_duration: 1024,
/// #     sample_composition_time_offset: 0,
/// #     sample_flags: Default::default(),
/// #     sample_description_index: 1,
/// # };
/// let mut muxer = Muxer::non_fragmented(NonFragmentedOptions::default());
/// muxer.handle_movie(movie)?;
/// muxer.handle_samples([Sample::new(properties, b"AB".to_vec())])?;
/// muxer.finish()?;
///
/// // A sink that cannot seek writes each output at the end, and stops at any other
/// let mut pipe: Vec<u8> = Vec::new();
/// while let Some(output) = muxer.poll_output() {
///     if output.offset() != u64::try_from(pipe.len())? {
///         return Err("the output goes elsewhere than the end".into());
///     }
///     pipe.extend_from_slice(output.bytes());
/// }
/// assert_eq!(pipe.get(4..8), Some(b"mdat".as_slice()));
/// # Ok::<(), Box<dyn core::error::Error>>(())
/// ```
#[non_exhaustive]
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub struct Output {
    offset: u64,
    bytes: Vec<u8>,
}

impl Output {
    /// Creates the output of `bytes` that go to file offset `offset`
    ///
    /// A writer outside this crate returns its output as this crate's
    /// writers do, under the same contract, so that one loop writes the
    /// output of either.
    #[must_use]
    pub const fn new(offset: u64, bytes: Vec<u8>) -> Self {
        Self { offset, bytes }
    }

    /// Returns the file offset the bytes go to
    #[must_use]
    pub const fn offset(&self) -> u64 {
        self.offset
    }

    /// Returns the bytes
    #[must_use]
    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }

    /// Returns the bytes, giving up the offset
    #[must_use]
    pub fn into_bytes(self) -> Vec<u8> {
        self.bytes
    }
}
