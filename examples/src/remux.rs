//! [`movie_file`] and [`fragmented_movie_file`], which write what [`read`](crate::demux::read) read with the muxers

use isobmff::muxer::{Error, MovieDescription, TrackDescription};
use isobmff::{FragmentedMovieFileMuxer, MovieFileMuxer, MuxerConfig};

use crate::demux::Read;

/// Returns the movie file of the samples `source` holds: its `ftyp`, one `mdat` and the `moov` that declares them
///
/// # Errors
/// What [`MovieFileMuxer`] refuses of the movie or its samples.
pub fn movie_file(source: &Read) -> Result<Vec<u8>, Error> {
    let mut muxer = MovieFileMuxer::new(
        MovieDescription::from(&source.movie),
        MuxerConfig::default(),
        source.file_type.clone(),
    )?;
    for trak in &source.movie.trak {
        muxer.add_track(TrackDescription::from(trak))?;
    }
    let mut written = Vec::new();
    for (properties, bytes) in &source.samples {
        muxer.handle_sample(*properties, bytes)?;
        muxer.poll_output(&mut written);
    }
    let header = muxer.finish()?;
    muxer.poll_output(&mut written);
    if let Some(first) = usize::try_from(header.offset)
        .ok()
        .and_then(|offset| written.get_mut(offset..))
        .and_then(|rest| rest.get_mut(..header.bytes.len()))
    {
        first.copy_from_slice(&header.bytes);
    }
    Ok(written)
}

/// Returns the fragmented movie file of the samples `source` holds, a fragment ending where one of `source` does, or where the next sample would pass the limits of the muxer
///
/// # Errors
/// What [`FragmentedMovieFileMuxer`] refuses of the movie or its samples.
pub fn fragmented_movie_file(source: &Read) -> Result<Vec<u8>, Error> {
    let mut muxer = FragmentedMovieFileMuxer::new(
        MovieDescription::from(&source.movie),
        MuxerConfig::default(),
        source.file_type.clone(),
    )?;
    for trak in &source.movie.trak {
        muxer.add_track(TrackDescription::from(trak))?;
    }
    let mut written = Vec::new();
    for (index, (properties, bytes)) in source.samples.iter().enumerate() {
        if source.fragment_starts.binary_search(&index).is_ok() {
            muxer.finish_fragment()?;
        }
        match muxer.handle_sample(*properties, bytes) {
            Err(
                Error::BufferFull { .. }
                | Error::StructureData(isobmff::structure_data::Error::SampleCountOverLimit {
                    ..
                }),
            ) => {
                muxer.poll_output(&mut written);
                muxer.finish_fragment()?;
                muxer.poll_output(&mut written);
                muxer.handle_sample(*properties, bytes)?;
            }
            result => result?,
        }
        muxer.poll_output(&mut written);
    }
    muxer.finish()?;
    muxer.poll_output(&mut written);
    Ok(written)
}
