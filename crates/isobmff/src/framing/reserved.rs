//! Space reserved in a file with a `free` box, and the bytes that take its place once they are known

use alloc::vec;
use alloc::vec::Vec;

use isobmff_boxes::{Free, FreeSpaceBox, MediaDataBox};
use isobmff_core::{BoxDefinition, BoxHeader, BoxSize, BoxType, ExtendedSize};

use crate::byte_range::ByteRange;
use crate::convert::widen;
use crate::error::MuxError;
use crate::framing::{BoxEvent, BoxWriter};
use crate::output::Output;

/// Length of the header of a `free` box, the least a `free` box takes
const FREE_HEADER_BYTES: u64 = 8;

/// Type of a `free` box, §8.1.2
const FREE: BoxType = FreeSpaceBox::<Free>::BOX_TYPE;

/// Returns the bytes of a `free` box of `total_bytes`, its payload zeros, that reserves as much space in a file, ISO/IEC 14496-12 §8.1.2
///
/// # Errors
///
/// * [`ReservedSpaceMismatch`](MuxError::ReservedSpaceMismatch): a `free`
///   box takes no fewer than 8 bytes, the length of its header.
///
/// # Examples
///
/// ```
/// use isobmff::framing::reserve;
///
/// assert_eq!(reserve(12)?, b"\0\0\0\x0cfree\0\0\0\0");
/// # Ok::<(), isobmff::MuxError>(())
/// ```
pub fn reserve(total_bytes: u64) -> Result<Vec<u8>, MuxError> {
    let mismatch = MuxError::ReservedSpaceMismatch {
        reserved_bytes: Some(total_bytes),
        offered_bytes: 0,
    };
    let payload_len = total_bytes.checked_sub(FREE_HEADER_BYTES).ok_or(mismatch)?;
    let header = BoxHeader::with_payload_len(FREE, payload_len).ok_or(mismatch)?;
    let mut bytes = vec![0; usize::try_from(total_bytes).map_err(|_| mismatch)?];
    header
        .encode(&mut bytes)
        .map_err(|error| MuxError::Box { error })?;
    Ok(bytes)
}

impl BoxWriter {
    /// Writes a `free` box of `total_bytes`, as [`reserve`] makes it, and returns it and the file offset the writer counts for it
    ///
    /// A convenience over [`handle_event`](BoxWriter::handle_event), as
    /// [`write_box`](BoxWriter::write_box) is.
    ///
    /// # Errors
    ///
    /// * The failures of [`reserve`].
    /// * The failures of [`handle_event`](BoxWriter::handle_event).
    ///
    /// # Examples
    ///
    /// ```
    /// use isobmff::boxes::FileTypeBox;
    /// use isobmff::framing::BoxWriter;
    ///
    /// // The `free` box goes after the boxes written before it, and counts in what the writer writes
    /// let mut boxes = BoxWriter::new();
    /// boxes.write_box(&FileTypeBox::default())?;
    /// let after_ftyp = boxes.next_offset();
    /// let free = boxes.write_reserved(12)?;
    /// assert_eq!(free.offset(), after_ftyp);
    /// assert_eq!(free.bytes(), b"\0\0\0\x0cfree\0\0\0\0");
    /// assert_eq!(boxes.next_offset(), after_ftyp + 12);
    /// # Ok::<(), isobmff::MuxError>(())
    /// ```
    pub fn write_reserved(&mut self, total_bytes: u64) -> Result<Output, MuxError> {
        let bytes = reserve(total_bytes)?;
        let offset = self.next_offset();
        let header =
            BoxHeader::with_payload_len(FREE, total_bytes.saturating_sub(FREE_HEADER_BYTES))
                .ok_or(MuxError::ReservedSpaceMismatch {
                    reserved_bytes: Some(total_bytes),
                    offered_bytes: 0,
                })?;
        self.handle_event(BoxEvent::Header(header))?;
        self.handle_event(BoxEvent::Payload(
            bytes
                .get(usize::try_from(header.encoded_len()).unwrap_or(usize::MAX)..)
                .unwrap_or_default(),
        ))?;
        self.handle_event(BoxEvent::End)?;
        Ok(Output::new(offset, bytes))
    }
}

/// Returns the outputs that put `bytes` in the space `reserved`: the bytes at its start, and the header of a `free` box after them that spans the rest of it, where there is a rest
///
/// The outputs write over bytes written already, as an [`Output`] may. The
/// rest of the space keeps the bytes it holds, the payload of the `free` box
/// that reserved it, which the header after `bytes` takes as its payload.
///
/// # Errors
///
/// * [`ReservedSpaceMismatch`](MuxError::ReservedSpaceMismatch): `reserved`
///   has no length, `bytes` are longer than it, or they leave fewer bytes of
///   it than the 8 of the header of a `free` box.
///
/// # Examples
///
/// ```
/// use isobmff::framing::{fill, reserve};
/// use isobmff::{ByteRange, MuxError, Output};
///
/// // 24 bytes reserved at file offset 100, of which a box of 12 takes the start
/// let reserved = ByteRange::new(100, Some(24));
/// let bytes = b"\0\0\0\x0cskipABCD".to_vec();
/// assert_eq!(
///     fill(reserved, bytes.clone())?,
///     [Output::new(100, bytes.clone()), Output::new(112, b"\0\0\0\x0cfree".to_vec())]
/// );
/// // The 4 bytes 20 would leave hold no `free` box
/// assert!(matches!(
///     fill(ByteRange::new(100, Some(16)), bytes),
///     Err(MuxError::ReservedSpaceMismatch { reserved_bytes: Some(16), offered_bytes: 12, .. })
/// ));
/// # Ok::<(), MuxError>(())
/// ```
pub fn fill(reserved: ByteRange, bytes: Vec<u8>) -> Result<Vec<Output>, MuxError> {
    let offered_bytes = widen(bytes.len());
    let mismatch = MuxError::ReservedSpaceMismatch {
        reserved_bytes: reserved.length(),
        offered_bytes,
    };
    let rest = reserved
        .length()
        .and_then(|length| length.checked_sub(offered_bytes))
        .ok_or(mismatch)?;
    let mut outputs = vec![Output::new(reserved.offset(), bytes)];
    if rest > 0 {
        let header = rest
            .checked_sub(FREE_HEADER_BYTES)
            .and_then(|payload_len| BoxHeader::with_payload_len(FREE, payload_len))
            .ok_or(mismatch)?;
        let mut free = vec![0; 8];
        header
            .encode(&mut free)
            .map_err(|error| MuxError::Box { error })?;
        outputs.push(Output::new(
            reserved.offset().saturating_add(offered_bytes),
            free,
        ));
    }
    Ok(outputs)
}

/// Returns the output that turns the 16-byte `free` box at file offset `placeholder_offset` into the header of an `mdat` whose payload of `payload_len` bytes follows it, ISO/IEC 14496-12 §4.2.2, §8.1.1
///
/// A writer that does not know how long the media data it writes will be
/// reserves 16 bytes for the header of its `mdat` with [`reserve`], writes
/// the media data after them, and once it knows the length writes this
/// output over them. Where the box fits a 32-bit `size`, the output is a
/// `free` box of 8 bytes and the 8-byte header of the `mdat` after it;
/// otherwise it is the 16-byte header of the `mdat` with a `largesize`.
/// Either way the payload starts 16 bytes past `placeholder_offset`.
///
/// It is `None` where no box holds `payload_len` bytes after a header of 16
/// bytes.
///
/// # Examples
///
/// ```
/// use isobmff::framing::{media_data_header, reserve};
/// use isobmff::{BoxHeader, BoxSize, BoxType, CompactSize};
///
/// // The placeholder at file offset 32, then 4 bytes of media data
/// let mut file = vec![0; 32];
/// file.extend(reserve(16)?);
/// file.extend(b"ABCD");
///
/// // Once the media data is written, the header takes the place of the placeholder
/// let header = media_data_header(32, 4).ok_or("too long")?;
/// let at = usize::try_from(header.offset())?;
/// file.get_mut(at..at + header.bytes().len()).ok_or("short")?.copy_from_slice(header.bytes());
/// let (mdat, _) = BoxHeader::decode(file.get(40..).ok_or("short")?)?;
/// assert_eq!((mdat.box_type(), mdat.size()), (BoxType::compact(*b"mdat"), BoxSize::Compact(CompactSize::new(12).ok_or("size")?)));
/// # Ok::<(), Box<dyn core::error::Error>>(())
/// ```
#[must_use]
pub fn media_data_header(placeholder_offset: u64, payload_len: u64) -> Option<Output> {
    let compact = BoxHeader::with_payload_len(MediaDataBox::BOX_TYPE, payload_len)
        .filter(|header| matches!(header.size(), BoxSize::Compact(_)));
    let mut bytes = vec![0; 16];
    match compact {
        Some(header) => {
            let (free, mdat) = bytes.split_at_mut(8);
            BoxHeader::with_payload_len(FREE, 0)?.encode(free).ok()?;
            header.encode(mdat).ok()?;
        }
        None => {
            let total_bytes = payload_len.checked_add(16)?;
            BoxHeader::new(
                MediaDataBox::BOX_TYPE,
                BoxSize::Extended(ExtendedSize::new(total_bytes)?),
            )?
            .encode(&mut bytes)
            .ok()?;
        }
    }
    Some(Output::new(placeholder_offset, bytes))
}

#[cfg(test)]
mod tests {
    use alloc::vec::Vec;

    use isobmff_core::{BoxHeader, BoxSize, BoxType};

    use super::{fill, media_data_header, reserve};
    use crate::byte_range::ByteRange;
    use crate::error::MuxError;
    use crate::output::Output;

    #[test]
    fn bytes_fill_their_space_whole_or_with_a_free_box_after_them_and_are_refused_where_no_free_box_fits()
     {
        let bytes = Vec::from(*b"ABCDEFGH");
        let reserved = |length| ByteRange::new(40, Some(length));
        let mismatch = |reserved_bytes, offered_bytes| MuxError::ReservedSpaceMismatch {
            reserved_bytes,
            offered_bytes,
        };

        assert_eq!(
            [
                fill(reserved(8), bytes.clone()),
                fill(reserved(16), bytes.clone()),
                fill(reserved(12), bytes.clone()),
                fill(reserved(4), bytes.clone()),
                fill(ByteRange::new(40, None), bytes.clone()),
            ],
            [
                Ok(Vec::from([Output::new(40, bytes.clone())])),
                Ok(Vec::from([
                    Output::new(40, bytes.clone()),
                    Output::new(48, Vec::from(*b"\0\0\0\x08free")),
                ])),
                Err(mismatch(Some(12), 8)),
                Err(mismatch(Some(4), 8)),
                Err(mismatch(None, 8)),
            ]
        );
    }

    #[test]
    fn a_free_box_reserves_its_whole_length_and_none_is_shorter_than_its_header() {
        assert_eq!(
            [reserve(8), reserve(10), reserve(7)],
            [
                Ok(Vec::from(*b"\0\0\0\x08free")),
                Ok(Vec::from(*b"\0\0\0\x0afree\0\0")),
                Err(MuxError::ReservedSpaceMismatch {
                    reserved_bytes: Some(7),
                    offered_bytes: 0
                }),
            ]
        );
    }

    #[test]
    fn the_header_of_an_mdat_takes_the_place_of_its_placeholder_with_a_largesize_only_past_32_bits()
    {
        let compact_most = u64::from(u32::MAX) - 8;
        let free_then_mdat =
            |size: u32| [b"\0\0\0\x08free".as_slice(), &size.to_be_bytes(), b"mdat"].concat();
        let largesize = |size: u64| [b"\0\0\0\x01mdat".as_slice(), &size.to_be_bytes()].concat();

        assert_eq!(
            [4, compact_most, compact_most + 1, u64::MAX - 15]
                .map(|payload_len| media_data_header(100, payload_len)),
            [
                Some(Output::new(100, free_then_mdat(12))),
                Some(Output::new(100, free_then_mdat(u32::MAX))),
                Some(Output::new(100, largesize(compact_most + 17))),
                None,
            ]
        );
        let large = media_data_header(100, compact_most + 1).unwrap();
        let (header, rest) = BoxHeader::decode(large.bytes()).unwrap();
        assert_eq!(
            (header.box_type(), header.payload_len(), rest.len()),
            (BoxType::compact(*b"mdat"), Some(compact_most + 1), 0)
        );
        assert!(matches!(header.size(), BoxSize::Extended(_)));
    }
}
