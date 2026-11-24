//! [`Observation`], what a demuxer observed that decides its next step, and [`observe`], which reads an event of the box reader into one

use alloc::boxed::Box;

use isobmff_boxes::{FileTypeBox, MovieBox};
use isobmff_core::{BoxDecode, BoxDefinition, BoxHeader, BoxType};

use crate::SampleLocation;
use crate::file_level;

/// What the demuxer observed that decides its next step
pub(super) enum Observation {
    /// A location of the samples of the `moov` not yet reported
    Unreported(SampleLocation),
    /// The `moov` at file offset `offset`, read into its value
    Movie { offset: u64, movie: Box<MovieBox> },
    /// The `ftyp`, read into its value
    FileType(FileTypeBox),
    /// A box at file level not read into a value
    Box { offset: u64, header: BoxHeader },
    /// The payload of the box at file offset `offset` of a type read into a
    /// value, which does not read
    Decode {
        offset: u64,
        box_type: BoxType,
        source: isobmff_core::Error,
    },
    /// A box at file level the box reader cannot read
    FileLevel(file_level::Error),
    /// Nothing without more input
    NeedMoreData,
}

/// How the demuxer reads the payload of a box of a type it reads into a value, given the file offset of the box
type DecodePayload = fn(u64, &[u8]) -> Result<Observation, isobmff_core::Error>;

/// The types of the boxes the demuxer reads into values, each with how it reads the payload of one
pub(super) const PAYLOAD_BOX_TYPES: [(BoxType, DecodePayload); 2] = [
    (FileTypeBox::BOX_TYPE, |_, payload| {
        Ok(Observation::FileType(FileTypeBox::decode_payload(payload)?))
    }),
    (MovieBox::BOX_TYPE, |offset, payload| {
        Ok(Observation::Movie {
            offset,
            movie: Box::new(MovieBox::decode_payload(payload)?),
        })
    }),
];

/// Reads `event` of the box reader into what the demuxer observes, the payload of a box into its value where the demuxer reads one
pub(super) fn observe(event: file_level::Event<'_>) -> Observation {
    match event {
        file_level::Event::Box { offset, header }
        | file_level::Event::FrontPart { offset, header, .. } => {
            Observation::Box { offset, header }
        }
        file_level::Event::BoxPayload {
            offset,
            box_type,
            payload,
            ..
        } => PAYLOAD_BOX_TYPES
            .iter()
            .find(|&&(payload_box_type, _)| payload_box_type == box_type)
            // Why not unreachable: the box reader lends the payload of the
            // types of this table alone, so no other type comes here, and a
            // reader that takes bytes reports what it cannot read as an event
            // rather than fail.
            .map_or(
                Err(isobmff_core::Error::unsupported_box(box_type)),
                |&(_, decode)| decode(offset, payload),
            )
            .unwrap_or_else(|source| Observation::Decode {
                offset,
                box_type,
                source,
            }),
        file_level::Event::Error(error) => Observation::FileLevel(error),
    }
}
