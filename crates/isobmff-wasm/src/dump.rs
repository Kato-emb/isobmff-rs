//! [`dump`], the tree of boxes a file is formed as

use core::ops::Range;
use std::io::{self, Read};

use isobmff::core::{BoxHeader, BoxType, boxes};
use isobmff::io::Error;
use isobmff::sequence::{BoxEvent, BoxReader};
use isobmff::structure;
use wasm_bindgen::prelude::wasm_bindgen;

/// The boxes whose payload is read as the boxes it holds
const CONTAINERS: [BoxType; 12] = [
    BoxType::compact(*b"moov"),
    BoxType::compact(*b"trak"),
    BoxType::compact(*b"edts"),
    BoxType::compact(*b"mdia"),
    BoxType::compact(*b"minf"),
    BoxType::compact(*b"dinf"),
    BoxType::compact(*b"stbl"),
    BoxType::compact(*b"mvex"),
    BoxType::compact(*b"moof"),
    BoxType::compact(*b"traf"),
    BoxType::compact(*b"mfra"),
    BoxType::compact(*b"udta"),
];

/// One box of a file, where it lies and how deep it is held
#[wasm_bindgen(getter_with_clone, inspectable)]
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct BoxRecord {
    /// The fourcc of the box, or `uuid` and its user type
    pub box_type: String,
    /// The offset in the file of the first byte of the box
    pub offset: u64,
    /// The bytes the box spans, header included, or none when it runs to the end of the file
    pub size: Option<u64>,
    /// The number of containers the box is held in, 0 for a box at the top of the file
    pub depth: u32,
}

/// Returns the boxes `source` is formed as, each container followed by the boxes it holds
///
/// # Errors
///
/// The failure of `source`, the reason the box reader or [`boxes`] refuses
/// what it reads, or a box that ends past `u64::MAX`.
pub(crate) fn dump<S: Read>(source: S) -> Result<Vec<BoxRecord>, Error> {
    let mut container: Option<(u64, Vec<u8>)> = None;
    let mut records = Vec::new();
    read_boxes(source, |event, extent| {
        match event {
            BoxEvent::Header(header) => {
                records.push(record(header, extent.start, 0));
                container = CONTAINERS
                    .contains(&header.box_type())
                    .then(|| (extent.end, Vec::new()));
            }
            BoxEvent::Payload(payload) => {
                if let Some((_, children)) = &mut container {
                    children.extend(payload);
                }
            }
            BoxEvent::End => {
                if let Some((offset, children)) = container.take() {
                    push_children(&mut records, &children, offset, 1)?;
                }
            }
            _ => {}
        }
        Ok(())
    })?;
    Ok(records)
}

/// Reads `source` to its end through a box reader, handing each event and the extent of the file it was read from to `on_event`
///
/// # Errors
///
/// The failure of `source`, the reason the box reader refuses what it reads,
/// or the failure `on_event` returns, which ends the reading.
pub(crate) fn read_boxes<S: Read>(
    mut source: S,
    mut on_event: impl FnMut(BoxEvent, Range<u64>) -> Result<(), Error>,
) -> Result<(), Error> {
    let mut reader = BoxReader::new();
    let mut cut = vec![0; 64 * 1024];
    loop {
        let read = source.read(&mut cut)?;
        if read == 0 {
            reader.finish().map_err(structure::Error::from)?;
        } else {
            reader
                .handle_input(cut.get(..read).unwrap_or_default())
                .map_err(structure::Error::from)?;
        }
        while let Some((event, extent)) = reader.poll_event().zip(reader.event_extent()) {
            on_event(event, extent)?;
        }
        if read == 0 {
            return Ok(());
        }
    }
}

/// Pushes the boxes laid end to end in `payload`, the first starting at `offset` of the file
fn push_children(
    records: &mut Vec<BoxRecord>,
    payload: &[u8],
    mut offset: u64,
    depth: u32,
) -> Result<(), Error> {
    for child in boxes(payload) {
        let child = child.map_err(structure::Error::from)?;
        let header = child.header();
        records.push(record(header, offset, depth));
        let payload_offset = offset
            .checked_add(u64::try_from(header.encoded_len()).map_err(io::Error::other)?)
            .ok_or_else(|| io::Error::other("a box ends past u64::MAX"))?;
        if CONTAINERS.contains(&header.box_type()) {
            push_children(
                records,
                child.payload(),
                payload_offset,
                depth.saturating_add(1),
            )?;
        }
        offset = payload_offset
            .checked_add(u64::try_from(child.payload().len()).map_err(io::Error::other)?)
            .ok_or_else(|| io::Error::other("a box ends past u64::MAX"))?;
    }
    Ok(())
}

/// Returns the record of the box `header` starts, at `offset` of the file and `depth` containers deep
fn record(header: BoxHeader, offset: u64, depth: u32) -> BoxRecord {
    BoxRecord {
        box_type: header.box_type().to_string(),
        offset,
        size: header.size().total_bytes(),
        depth,
    }
}

#[cfg(test)]
mod tests {
    use std::io;

    use isobmff_test_support::{
        file_running_to_its_end, fragmented_file_with_samples, segment_file_with_samples,
    };

    use super::{BoxRecord, dump};

    fn listed(box_type: &str, offset: u64, size: Option<u64>, depth: u32) -> BoxRecord {
        BoxRecord {
            box_type: box_type.to_owned(),
            offset,
            size,
            depth,
        }
    }

    #[test]
    fn a_fragmented_file_lists_each_container_followed_by_the_boxes_it_holds() {
        let records = dump(io::Cursor::new(fragmented_file_with_samples())).unwrap();

        assert_eq!(
            records,
            vec![
                listed("ftyp", 0, Some(24), 0),
                listed("moov", 24, Some(513), 0),
                listed("mvhd", 32, Some(108), 1),
                listed("trak", 140, Some(357), 1),
                listed("tkhd", 148, Some(92), 2),
                listed("mdia", 240, Some(257), 2),
                listed("mdhd", 248, Some(32), 3),
                listed("hdlr", 280, Some(45), 3),
                listed("minf", 325, Some(172), 3),
                listed("vmhd", 333, Some(20), 4),
                listed("dinf", 353, Some(36), 4),
                listed("dref", 361, Some(28), 5),
                listed("stbl", 389, Some(108), 4),
                listed("stsd", 397, Some(32), 5),
                listed("stts", 429, Some(16), 5),
                listed("stsc", 445, Some(16), 5),
                listed("stsz", 461, Some(20), 5),
                listed("stco", 481, Some(16), 5),
                listed("mvex", 497, Some(40), 1),
                listed("trex", 505, Some(32), 2),
                listed("moof", 537, Some(84), 0),
                listed("mfhd", 545, Some(16), 1),
                listed("traf", 561, Some(60), 1),
                listed("tfhd", 569, Some(16), 2),
                listed("tfdt", 585, Some(16), 2),
                listed("trun", 601, Some(20), 2),
                listed("mdat", 621, Some(32), 0),
            ]
        );
    }

    #[test]
    fn a_media_segment_lists_its_fragments_in_file_order() {
        let records = dump(io::Cursor::new(segment_file_with_samples())).unwrap();

        assert_eq!(
            records,
            vec![
                listed("styp", 0, Some(24), 0),
                listed("moof", 24, Some(84), 0),
                listed("mfhd", 32, Some(16), 1),
                listed("traf", 48, Some(60), 1),
                listed("tfhd", 56, Some(16), 2),
                listed("tfdt", 72, Some(16), 2),
                listed("trun", 88, Some(20), 2),
                listed("mdat", 108, Some(32), 0),
                listed("moof", 140, Some(84), 0),
                listed("mfhd", 148, Some(16), 1),
                listed("traf", 164, Some(60), 1),
                listed("tfhd", 172, Some(16), 2),
                listed("tfdt", 188, Some(16), 2),
                listed("trun", 204, Some(20), 2),
                listed("mdat", 224, Some(24), 0),
            ]
        );
    }

    #[test]
    fn a_box_running_to_the_end_of_the_file_has_no_size() {
        let records = dump(io::Cursor::new(file_running_to_its_end())).unwrap();

        assert_eq!(
            records,
            vec![
                listed("free", 0, Some(8), 0),
                listed("skip", 8, Some(48), 0),
                listed("uuid 01234567-89ab-cdef-fedc-ba9876543210", 56, Some(32), 0),
                listed("mdat", 88, Some(72), 0),
                listed("mdat", 160, None, 0),
            ]
        );
    }
}
