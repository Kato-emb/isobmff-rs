//! The samples of a track as the structure-data describes them

use isobmff_boxes::SampleFlags;

/// What the structure-data declares of a `sample`, apart from where its bytes lie
///
/// The `sample`, ISO/IEC 14496-12 §3.1.45, as a sample table, §8.5, or a track fragment, §8.8.6, declares it
#[expect(
    clippy::exhaustive_structs,
    reason = "a caller of a writer builds one, and the fields are those a sample table or a track fragment declares of a sample"
)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct SampleProperties {
    /// `track_ID` of the track the sample belongs to
    pub track_id: u32,
    /// Decoding time of the sample, in the timescale of the media, as given by
    /// the sample table (§8.6.1.2) or track fragment (§8.8.12)
    pub decoding_time: u64,
    /// `sample_duration` of the sample, the `sample_delta` of the sample table
    /// (§8.6.1.2) or the `sample_duration` of the track fragment (§8.8.7, §8.8.8)
    pub sample_duration: u32,
    /// `sample_composition_time_offset`, the difference between the
    /// composition time of the sample and its decoding time
    pub sample_composition_time_offset: i64,
    /// `sample_flags`, as §8.8.3.1 packs them; for a sample of a sample
    /// table, the sync sample table and the sample dependency types give them
    pub sample_flags: SampleFlags,
    /// `sample_description_index`, from 1, of the sample entry that describes
    /// the sample
    pub sample_description_index: u32,
}

/// Where the bytes of a sample lie, the `location of samples`, with what the structure-data declares of it
///
/// The `location of samples`, ISO/IEC 14496-12 §6.1
///
/// The bytes of the sample lie in its `container`, and `offset` and `size`
/// give their byte range there, counted from the `data_origin`.
#[expect(
    clippy::exhaustive_structs,
    reason = "a caller of a writer builds one, and a sample lies in a container at an offset from an origin for a size whatever else it declares"
)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct SampleLocation {
    /// What the structure-data declares of the sample
    pub properties: SampleProperties,
    /// Where the bytes of the sample lie, as the data reference entry of its
    /// sample entry specifies, §6.1 and §8.7.2
    pub container: MediaDataContainer,
    /// What `offset` counts from, which the data reference entry sets for a
    /// sample table, §6.1 Table 1, and the track fragment header with it for
    /// a track fragment, §8.8.7.1 Table 6
    pub data_origin: DataOrigin,
    /// Data offset of the first byte of the sample, from the `data_origin`:
    /// the start of its chunk (§8.7.5) or of the data of its track fragment
    /// run (§8.8.8), and the sizes of the samples before it there
    pub offset: u64,
    /// Number of the bytes of the sample, as the sample size table (§8.7.3)
    /// or the track fragment (§8.8.7, §8.8.8) gives it
    pub size: u32,
}

/// Where the bytes of a sample lie, the `container` the data reference entry of its sample entry specifies
///
/// The container for the sample data, ISO/IEC 14496-12 §6.1 and §8.7.2
///
/// A reader takes it from the data reference entry; a location that
/// [`DataOrigins`](crate::structure_data::data_layout::DataOrigins) puts at
/// its file offset names the container its bytes lie in there.
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MediaDataContainer {
    /// The same file where the data reference entry is present: as read, a
    /// `DataEntryUrlBox` whose flags say so, §8.7.2.3; as put at its file
    /// offset, also a sample whose data offset counted from a
    /// `MovieFragmentBox` or from the payload of an `IdentifiedMediaDataBox`
    /// of the file
    SameFile,
    /// A resource other than the same file: one a `DataEntryUrlBox` or a
    /// `DataEntryUrnBox` identifies by its URL or URN, or one a data
    /// reference entry of a type this crate does not read names
    Resource,
    /// The `IdentifiedMediaDataBox` whose `imda_identifier` is
    /// `imda_identifier`: the `imda_ref_identifier` of a `DataEntryImdaBox`,
    /// or, for a `DataEntrySeqNumImdaBox`, the `sequence_number` of the movie
    /// fragment that contains the sample, §8.7.2.1
    IdentifiedMediaData {
        /// `imda_identifier` of the `IdentifiedMediaDataBox`, §8.1.4.2
        imda_identifier: u32,
    },
}

/// What a data offset counts from, the `data origin`
///
/// The `data origin`, ISO/IEC 14496-12 §6.1 Table 1, and §8.8.7.1 Table 6 for a track fragment
///
/// Table 1 gives the origins of the data offsets of items too; these are
/// those of samples alone.
// Why not let a caller match every variant: the origins are those a version of
// the specification sets, and the structure-data resolves them in one place.
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DataOrigin {
    /// The first byte of the file the data reference entry identifies:
    /// file-relative, §6.1 Table 1 and §8.7.5.3
    File,
    /// The first byte of the `MovieFragmentBox` that defines the sample:
    /// movie-fragment-relative, §6.1 Table 1 and §8.8.7.1 Table 6
    MovieFragment,
    /// The first byte of the payload of the `IdentifiedMediaDataBox` the data
    /// reference entry names, by the `imda_ref_identifier` of a
    /// `DataEntryImdaBox` or, for a `DataEntrySeqNumImdaBox`, the
    /// `sequence_number` of the movie fragment: §6.1 Table 1, §8.7.2.1 and
    /// §8.7.5.3
    IdentifiedMediaData,
}
