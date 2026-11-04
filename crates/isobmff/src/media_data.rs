//! Samples gathered out of the media data of a file as its bytes arrive
//!
//! Media data is the bytes of samples, which structure-data describes by
//! reference (ISO/IEC 14496-12 §8.1.1, A.3). A [`MediaDataReader`] takes the
//! [`SampleLocation`](crate::SampleLocation)s structure-data resolves to, and
//! the bytes of the file as they arrive, and gathers each sample out of them.
//! A [`MediaDataWriter`], the other way, lays the media data of samples
//! written together into an `mdat`, and returns the location of each.
//!
//! The writers of the layers write a file: here a fragmented movie file,
//! each `moof` its
//! [`MovieFragmentWriter`](crate::structure_data::MovieFragmentWriter)
//! returns followed by the `mdat` of its samples, which a
//! [`MediaDataReader`] gathers back out of the file.
//!
//! ```
//! use isobmff::boxes::{DataEntry, DataEntryUrlBox, FileTypeBox, MovieBox, SampleDescription, SampleEntry, TrackBox};
//! use isobmff::framing::{BoxEvent, BoxWriter};
//! use isobmff::media_data::{MediaDataLimits, MediaDataReader, MediaDataWriter};
//! use isobmff::structure_data::{FragmentedOptions, MovieFragmentWriter};
//! use isobmff::{BoxDefinition, BoxEncode, BoxHeader, FourCc, Sample, SampleProperties};
//! # use core::error::Error;
//!
//! /// Writes `value` as the next box at file level through `boxes`, onto `file`
//! fn write_box<Value: BoxEncode + BoxDefinition>(boxes: &mut BoxWriter, file: &mut Vec<u8>, value: &Value) -> Result<(), Box<dyn Error>> {
//!     let mut payload = vec![0; usize::try_from(value.payload_len())?];
//!     value.encode_payload(&mut payload)?;
//!     let header = BoxHeader::with_payload_len(Value::BOX_TYPE, value.payload_len()).ok_or("too long")?;
//!     for event in [BoxEvent::Header(header), BoxEvent::Payload(&payload), BoxEvent::End] {
//!         file.extend_from_slice(boxes.handle_event(event)?);
//!     }
//!     Ok(())
//! }
//!
//! // A movie of one track, its samples described by one sample entry, and in the file itself
//! let mut trak = TrackBox::default();
//! trak.tkhd.track_id = 1;
//! trak.mdia.mdhd.timescale = 1000;
//! let mut entry = SampleEntry::default();
//! entry.format = FourCc::new(*b"mp4a");
//! entry.data_reference_index = 1;
//! trak.mdia.minf.stbl.stsd.sample_descriptions = vec![SampleDescription::Typed(entry)];
//! trak.mdia.minf.dinf.dref.data_entries = vec![DataEntry::Url(DataEntryUrlBox::default())];
//! let mut movie = MovieBox::default();
//! movie.mvhd.timescale = 1000;
//! movie.mvhd.next_track_id = 2;
//! movie.trak = vec![trak];
//! let sample = |decoding_time, data: &[u8]| {
//!     let properties = SampleProperties {
//!         track_id: 1,
//!         decoding_time,
//!         sample_duration: 1024,
//!         sample_composition_time_offset: 0,
//!         sample_flags: Default::default(),
//!         sample_description_index: 1,
//!     };
//!     Sample::new(properties, data.to_vec())
//! };
//! let fragments_of_samples = [[sample(0, b"AB"), sample(1024, b"CD")], [sample(2048, b"EF"), sample(3072, b"GH")]];
//!
//! // The brands, then the `moov` the fragments extend
//! let (mut boxes, mut file) = (BoxWriter::new(), Vec::new());
//! let mut ftyp = FileTypeBox::default();
//! ftyp.major_brand = FourCc::new(*b"iso6");
//! write_box(&mut boxes, &mut file, &ftyp)?;
//! let mut fragments = MovieFragmentWriter::fragmented(movie, FragmentedOptions::default())?;
//! write_box(&mut boxes, &mut file, fragments.movie())?;
//!
//! // Each `moof`, then the `mdat` of its samples where the `moof` locates them
//! let media_data = MediaDataWriter::new();
//! let mut written = Vec::new();
//! for samples in &fragments_of_samples {
//!     let header = media_data.header(samples)?;
//!     let (moof, locations) = fragments.movie_fragment(samples, boxes.next_offset(), header.encoded_len())?;
//!     write_box(&mut boxes, &mut file, &moof)?;
//!     let (mdat, placed) = media_data.write(fragments.movie(), &mut boxes, header, samples)?;
//!     file.extend_from_slice(&mdat);
//!     assert_eq!(placed, locations);
//!     written.extend(locations);
//! }
//! boxes.finish()?;
//!
//! // A media data reader gathers the same samples back out of the file
//! let mut reader = MediaDataReader::new(MediaDataLimits {
//!     held_location_count: 16,
//!     sample_bytes: 16,
//!     held_bytes: 1024,
//! });
//! reader.handle_sample_locations(written)?;
//! reader.handle_input(0, &file)?;
//! reader.finish()?;
//! let samples: Vec<Sample> = core::iter::from_fn(|| reader.poll_sample()).collect();
//! assert_eq!(samples, fragments_of_samples.concat());
//! # Ok::<(), Box<dyn Error>>(())
//! ```

mod reader;
mod writer;

pub use reader::{MediaDataLimits, MediaDataReader};
pub use writer::MediaDataWriter;
