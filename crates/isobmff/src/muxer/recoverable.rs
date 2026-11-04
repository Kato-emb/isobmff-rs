//! [`Recoverable`], what a muxer keeps of a recoverable file to lay it out again as one that is not fragmented

use isobmff_boxes::MovieBox;
use isobmff_core::Error;

use crate::error::MuxError;
use crate::framing::{BoxWriter, media_data_header};
use crate::output::Output;
use crate::sample::SampleLocation;
use crate::structure_data::SampleTableWriter;
use crate::widen::widen;

/// Length of the `free` box the header of the `mdat` takes the place of, as [`media_data_header`] takes it
const PLACEHOLDER_BYTES: u64 = 16;

/// What a muxer keeps of a recoverable file: where the `free` that becomes the header of the `mdat` lies, and the sample tables of the samples written as fragments
#[derive(Debug)]
pub(super) struct Recoverable {
    /// File offset of the `free` box
    placeholder: u64,
    tables: Tables,
    /// Number of samples written
    held_locations: u64,
    /// Number of samples the sample tables declare up to
    limit_locations: u64,
}

/// Sample tables of the samples of a recoverable file, or why it is not laid out again
#[derive(Debug)]
enum Tables {
    /// Declaring the samples as they are written
    Declaring(SampleTableWriter),
    /// Dropped: the file holds more samples than the limit
    PastLimit,
    /// Dropped: they refused samples, with this failure
    Refused(MuxError),
    /// Written in the `moov` the file is laid out again with
    LaidOut(SampleTableWriter),
}

impl Recoverable {
    /// Writes the `free` box the header of the `mdat` takes the place of through `boxes`, and returns it and what a muxer keeps of a recoverable file of the samples of the tracks of `movie`, `limit_locations` samples at most
    ///
    /// # Errors
    ///
    /// * The failures of [`SampleTableWriter::new`].
    /// * The failures of [`BoxWriter::write_reserved`].
    pub(super) fn new(
        boxes: &mut BoxWriter,
        movie: &MovieBox,
        limit_locations: u64,
    ) -> Result<(Self, Output), MuxError> {
        let tables = SampleTableWriter::new(movie.clone())?;
        let placeholder = boxes.write_reserved(PLACEHOLDER_BYTES)?;
        Ok((
            Self {
                placeholder: placeholder.offset(),
                tables: Tables::Declaring(tables),
                held_locations: 0,
                limit_locations,
            },
            placeholder,
        ))
    }

    /// Declares the samples of `locations`, written as a fragment, in the sample tables, unless they take the samples past the limit or the tables refuse them
    ///
    /// Either way the file is not laid out again: the sample tables are
    /// dropped, and [`finish`](Self::finish) returns why.
    pub(super) fn handle_locations(&mut self, locations: &[SampleLocation]) {
        self.held_locations = self.held_locations.saturating_add(widen(locations.len()));
        let Tables::Declaring(tables) = &mut self.tables else {
            return;
        };
        if self.held_locations > self.limit_locations {
            self.tables = Tables::PastLimit;
        } else if let Err(error) = tables.handle_locations(locations) {
            self.tables = Tables::Refused(error);
        }
    }

    /// Writes the `moov` of the samples written at the end of the file, and returns it and the output that turns the `free` into the header of the `mdat` of the fragments, in that order
    ///
    /// # Errors
    ///
    /// * [`NotFinalised`](MuxError::NotFinalised): the file holds more
    ///   samples than the limit.
    /// * The failure the sample tables refused samples with.
    /// * [`Box`](MuxError::Box), and the failures of
    ///   [`BoxWriter::handle_event`]: the `moov` does not write.
    pub(super) fn finish(&mut self, boxes: &mut BoxWriter) -> Result<[Output; 2], MuxError> {
        let tables = match &mut self.tables {
            Tables::Declaring(tables) => tables,
            Tables::PastLimit => {
                return Err(MuxError::NotFinalised {
                    held_locations: self.held_locations,
                    limit_locations: self.limit_locations,
                });
            }
            Tables::Refused(error) => return Err(*error),
            Tables::LaidOut(_) => return Err(MuxError::AlreadyFinished),
        };
        let movie_offset = boxes.next_offset();
        let movie = boxes.write_box(tables.finish())?;
        // Why not count the payload of the mdat from the end of the fragments: the mdat holds everything between the placeholder and the moov, the moov of the fragments too
        let payload_len = movie_offset
            .saturating_sub(self.placeholder)
            .saturating_sub(PLACEHOLDER_BYTES);
        let header = media_data_header(self.placeholder, payload_len).ok_or(MuxError::Box {
            error: Error::truncated_buffer(payload_len, u64::MAX),
        })?;
        if let Tables::Declaring(tables) = core::mem::replace(&mut self.tables, Tables::PastLimit) {
            self.tables = Tables::LaidOut(tables);
        }
        Ok([movie, header])
    }

    /// Returns the `moov` the file is laid out again with, once it is
    pub(super) const fn movie(&self) -> Option<&MovieBox> {
        match &self.tables {
            Tables::LaidOut(tables) => Some(tables.movie()),
            Tables::Declaring(_) | Tables::PastLimit | Tables::Refused(_) => None,
        }
    }

    /// Returns whether the file is laid out again as one that is not fragmented
    pub(super) const fn laid_out(&self) -> bool {
        matches!(self.tables, Tables::LaidOut(_))
    }
}
