//! The compressed boxes, read back into the boxes they replace, ISO/IEC 14496-12 §8.16
//!
//! The payload of a compressed box is the payload of the box it replaces,
//! deflated (§8.16.1, IETF RFC 1951). Files are also written with that
//! deflate stream in a gzip member (IETF RFC 1952), and this is read too: a
//! gzip member opens with the byte `0x1f`, which no deflate stream does, as it
//! would open a block of the reserved type 3 (RFC 1951 §3.2.3). A zlib
//! stream (IETF RFC 1950) is not read: it cannot be told from a deflate
//! stream by its first bytes.

use alloc::boxed::Box;
use alloc::vec::Vec;

use isobmff_boxes::{
    CompressedMovieBox, CompressedMovieFragmentBox, CompressedSegmentIndexBox,
    CompressedSubsegmentIndexBox,
};
use isobmff_core::{BoxDefinition, BoxType, RawBox};
use miniz_oxide::inflate::TINFLStatus;
use miniz_oxide::inflate::core::{DecompressorOxide, decompress, inflate_flags};

use crate::error::DemuxError;
use crate::widen::widen;

/// Identification, compression method, flags, modification time, extra flags and operating system of a gzip member, RFC 1952 §2.3.1
const GZIP_HEADER_BYTES: usize = 10;
/// CRC-32 and length of the inflated data a gzip member ends with
const GZIP_TRAILER_BYTES: usize = 8;
/// First two bytes of a gzip member, and its compression method, deflate
const GZIP_OPENING: [u8; 3] = [0x1f, 0x8b, 8];
/// Flag of a header CRC-16
const FHCRC: u8 = 0x02;
/// Flag of extra fields
const FEXTRA: u8 = 0x04;
/// Flag of a file name, ending with a zero byte
const FNAME: u8 = 0x08;
/// Flag of a comment, ending with a zero byte
const FCOMMENT: u8 = 0x10;
/// Flags RFC 1952 reserves, which a reader refuses
const FRESERVED: u8 = 0xe0;
/// Length the payload of a compressed box is first grown to as it is inflated, within the limit
const MIN_GROWN_BYTES: usize = 1024;

/// Returns the replacement type of a compressed box of `box_type`, the type of the box it replaces (§8.16.1), or `None` for a box that is not compressed
#[must_use]
pub fn replacement_type(box_type: BoxType) -> Option<BoxType> {
    match box_type {
        CompressedMovieBox::BOX_TYPE => Some(CompressedMovieBox::REPLACEMENT_TYPE),
        CompressedMovieFragmentBox::BOX_TYPE => Some(CompressedMovieFragmentBox::REPLACEMENT_TYPE),
        CompressedSegmentIndexBox::BOX_TYPE => Some(CompressedSegmentIndexBox::REPLACEMENT_TYPE),
        CompressedSubsegmentIndexBox::BOX_TYPE => {
            Some(CompressedSubsegmentIndexBox::REPLACEMENT_TYPE)
        }
        BoxType::Compact(_) | BoxType::Extended(_) => None,
    }
}

/// Reads the compressed box `compressed_box` back into the box it replaces, its payload at most `limit_bytes`, and returns its type and payload, or `None` for a box that is not compressed
///
/// This is the processing model of §8.16.2.1: the box that replaces a
/// compressed box is of its [`replacement_type`], and its payload is that of
/// the compressed box, inflated. The offsets in the payload count in the file
/// as it would be with its compressed boxes so read back (§8.16.1).
///
/// The payload is grown as it is inflated, and never past `limit_bytes`: one
/// that would run past it is refused before it does (ISO/IEC 14496-12 sets no
/// bound on how far a compressed box inflates).
///
/// # Errors
///
/// * [`PayloadLimitExceeded`](DemuxError::PayloadLimitExceeded): the payload
///   inflates past `limit_bytes`.
/// * [`UnreadableCompressedData`](DemuxError::UnreadableCompressedData): the
///   payload is neither a deflate stream nor a gzip member, or the CRC-32 or
///   the length a gzip member ends with does not match what it inflates to.
pub fn decompressed(
    compressed_box: RawBox<'_>,
    limit_bytes: u64,
) -> Result<Option<(BoxType, Vec<u8>)>, DemuxError> {
    let box_type = compressed_box.header().box_type();
    replacement_type(box_type)
        .map(|replacement| {
            inflate(box_type, compressed_box.payload(), limit_bytes)
                .map(|payload| (replacement, payload))
        })
        .transpose()
}

/// Inflates `compressed_data`, the payload of the compressed box of `box_type`, into at most `limit_bytes`
fn inflate(
    box_type: BoxType,
    compressed_data: &[u8],
    limit_bytes: u64,
) -> Result<Vec<u8>, DemuxError> {
    if compressed_data.first() != GZIP_OPENING.first() {
        return deflated(box_type, compressed_data, limit_bytes);
    }
    let unreadable = DemuxError::UnreadableCompressedData { box_type };
    let (deflate_stream, crc, length) = gzip_member(compressed_data).ok_or(unreadable)?;
    // Why not rest on this check alone: the length is kept modulo 2^32, so a
    // member may inflate past the limit with a length that falls short of it.
    if u64::from(length) > limit_bytes {
        return Err(DemuxError::PayloadLimitExceeded {
            box_type,
            reached_bytes: u64::from(length),
            limit_bytes,
        });
    }
    let payload = deflated(box_type, deflate_stream, limit_bytes)?;
    // RFC 1952 keeps the length modulo 2^32.
    if crc32(&payload) != crc || widen(payload.len()) & u64::from(u32::MAX) != u64::from(length) {
        return Err(unreadable);
    }
    Ok(payload)
}

/// Inflates the deflate stream `stream` of the compressed box of `box_type` into at most `limit_bytes`, holding no more than that
fn deflated(box_type: BoxType, stream: &[u8], limit_bytes: u64) -> Result<Vec<u8>, DemuxError> {
    let limit = usize::try_from(limit_bytes).unwrap_or(usize::MAX);
    let unreadable = DemuxError::UnreadableCompressedData { box_type };
    let mut decompressor = Box::<DecompressorOxide>::default();
    let mut payload = Vec::new();
    let mut input = stream;
    let mut inflated = 0_usize;
    loop {
        // Why not `Vec::resize` alone, as `decompress_to_vec_with_limit` grows its output: it may reserve past the length it grows to, and so past the limit.
        let grown = payload
            .len()
            .saturating_mul(2)
            .max(MIN_GROWN_BYTES)
            .min(limit);
        payload.reserve_exact(grown.saturating_sub(payload.len()));
        payload.resize(grown, 0);
        let (status, consumed, produced) = decompress(
            &mut decompressor,
            input,
            &mut payload,
            inflated,
            inflate_flags::TINFL_FLAG_USING_NON_WRAPPING_OUTPUT_BUF,
        );
        inflated = inflated.saturating_add(produced);
        input = input.get(consumed..).ok_or(unreadable)?;
        match status {
            TINFLStatus::Done => {
                payload.truncate(inflated);
                return Ok(payload);
            }
            TINFLStatus::HasMoreOutput if payload.len() >= limit => {
                return Err(DemuxError::PayloadLimitExceeded {
                    box_type,
                    reached_bytes: limit_bytes.saturating_add(1),
                    limit_bytes,
                });
            }
            TINFLStatus::HasMoreOutput => {}
            TINFLStatus::FailedCannotMakeProgress
            | TINFLStatus::BadParam
            | TINFLStatus::Adler32Mismatch
            | TINFLStatus::Failed
            | TINFLStatus::NeedsMoreInput => return Err(unreadable),
        }
    }
}

/// Returns the deflate stream of the gzip member `member`, and the CRC-32 and the length modulo 2^32 of what it inflates to, or `None` for a member that does not read
#[expect(
    clippy::little_endian_bytes,
    reason = "a gzip member keeps its fields least significant byte first, RFC 1952 §2.1"
)]
fn gzip_member(member: &[u8]) -> Option<(&[u8], u32, u32)> {
    let (header, mut rest) = member.split_at_checked(GZIP_HEADER_BYTES)?;
    let (opening, fields) = header.split_at_checked(GZIP_OPENING.len())?;
    let flags = *fields.first()?;
    if opening != GZIP_OPENING || flags & FRESERVED != 0 {
        return None;
    }
    if flags & FEXTRA != 0 {
        let (length, after) = rest.split_first_chunk::<2>()?;
        rest = after.get(usize::from(u16::from_le_bytes(*length))..)?;
    }
    for flag in [FNAME, FCOMMENT] {
        if flags & flag != 0 {
            let end = rest.iter().position(|&byte| byte == 0)?;
            rest = rest.get(end.checked_add(1)?..)?;
        }
    }
    if flags & FHCRC != 0 {
        let (header_crc, after) = rest.split_first_chunk::<2>()?;
        let header_len = member.len().checked_sub(rest.len())?;
        let crc16 = u16::try_from(crc32(member.get(..header_len)?) & 0xffff).ok()?;
        if u16::from_le_bytes(*header_crc) != crc16 {
            return None;
        }
        rest = after;
    }
    let (stream, trailer) = rest.split_last_chunk::<GZIP_TRAILER_BYTES>()?;
    let (crc, length) = (trailer.first_chunk::<4>()?, trailer.last_chunk::<4>()?);
    Some((
        stream,
        u32::from_le_bytes(*crc),
        u32::from_le_bytes(*length),
    ))
}

/// Polynomial of the CRC-32 of IEEE 802.3 that gzip keeps, least significant bit first, RFC 1952 §8
const CRC32_POLYNOMIAL: u32 = 0xedb8_8320;

/// CRC-32 of each byte alone, which [`crc32`] takes a byte at a time from
#[expect(
    clippy::indexing_slicing,
    reason = "evaluated when compiled, where an index past the table fails the build; the index runs below 256"
)]
const CRC32_TABLE: [u32; 256] = {
    let mut table = [0; 256];
    let mut index = 0;
    let mut byte = 0_u32;
    while index < 256 {
        let mut crc = byte;
        let mut bit = 0;
        while bit < 8 {
            crc = (crc >> 1) ^ (CRC32_POLYNOMIAL & 0_u32.wrapping_sub(crc & 1));
            bit += 1;
        }
        table[index] = crc;
        index += 1;
        byte += 1;
    }
    table
};

/// Returns the CRC-32 of `bytes`, the one of IEEE 802.3 that gzip keeps, RFC 1952 §8
fn crc32(bytes: &[u8]) -> u32 {
    !bytes.iter().fold(u32::MAX, |crc, &byte| {
        let index = (crc ^ u32::from(byte)) & 0xff;
        let entry = CRC32_TABLE
            .get(usize::try_from(index).unwrap_or_default())
            .copied()
            .unwrap_or_default();
        entry ^ (crc >> 8)
    })
}

#[cfg(test)]
#[expect(
    clippy::little_endian_bytes,
    reason = "the gzip members the tests write keep their fields least significant byte first, RFC 1952 §2.1"
)]
mod tests {
    use alloc::vec;
    use alloc::vec::Vec;

    use isobmff_core::{BoxDefinition, BoxHeader, BoxType, RawBox};
    use miniz_oxide::deflate::compress_to_vec;

    use super::{FCOMMENT, FEXTRA, FHCRC, FNAME, crc32, decompressed, replacement_type};
    use crate::error::DemuxError;

    const MOVIE: BoxType = BoxType::compact(*b"!mov");
    const PAYLOAD: &[u8] = b"the payload of a moov, deflated and read back";

    /// Returns the bytes of a box of `box_type` and `payload`
    fn boxed(box_type: BoxType, payload: &[u8]) -> Vec<u8> {
        let header =
            BoxHeader::with_payload_len(box_type, u64::try_from(payload.len()).unwrap()).unwrap();
        let mut bytes = vec![0; usize::try_from(header.encoded_len()).unwrap()];
        header.encode(&mut bytes).unwrap();
        [bytes.as_slice(), payload].concat()
    }

    /// Reads `bytes`, a compressed box, back as [`decompressed`] does
    fn read_back(bytes: &[u8], limit_bytes: u64) -> Result<Option<(BoxType, Vec<u8>)>, DemuxError> {
        decompressed(RawBox::split_first(bytes).unwrap().0, limit_bytes)
    }

    /// Returns `payload` in a gzip member of `flags`, with the fields they call for
    fn gzip(payload: &[u8], flags: u8) -> Vec<u8> {
        let mut member = vec![0x1f, 0x8b, 8, flags, 0, 0, 0, 0, 0, 3];
        if flags & FEXTRA != 0 {
            member.extend_from_slice(&[4, 0, b'A', b'B', 2, 0]);
        }
        if flags & FNAME != 0 {
            member.extend_from_slice(b"moov.bin\0");
        }
        if flags & FCOMMENT != 0 {
            member.extend_from_slice(b"a comment\0");
        }
        if flags & FHCRC != 0 {
            let header_crc = u16::try_from(crc32(&member) & 0xffff).unwrap();
            member.extend_from_slice(&header_crc.to_le_bytes());
        }
        member.extend_from_slice(&compress_to_vec(payload, 6));
        member.extend_from_slice(&crc32(payload).to_le_bytes());
        member.extend_from_slice(&u32::try_from(payload.len()).unwrap().to_le_bytes());
        member
    }

    #[test]
    fn the_crc_32_of_the_check_string_is_that_of_ieee_802_3() {
        assert_eq!(crc32(b"123456789"), 0xcbf4_3926);
    }

    #[test]
    fn the_replacement_type_of_each_compressed_box_is_that_of_section_8_16() {
        assert_eq!(
            [*b"!mov", *b"!mof", *b"!six", *b"!ssx", *b"moov", *b"otyp"]
                .map(|four_cc| replacement_type(BoxType::compact(four_cc))),
            [
                Some(BoxType::compact(*b"moov")),
                Some(BoxType::compact(*b"moof")),
                Some(BoxType::compact(*b"sidx")),
                Some(BoxType::compact(*b"ssix")),
                None,
                None,
            ]
        );
    }

    #[test]
    fn a_deflate_stream_and_a_gzip_member_of_every_header_field_read_back_into_the_payload() {
        let read_back_payload = |compressed: Vec<u8>| {
            read_back(&boxed(MOVIE, &compressed), 1024).map(|read| {
                read.map(|(box_type, payload)| (*box_type.four_cc().as_bytes(), payload))
            })
        };
        let expected = Ok(Some((*b"moov", PAYLOAD.to_vec())));

        assert_eq!(read_back_payload(compress_to_vec(PAYLOAD, 6)), expected);
        for flags in [
            0,
            FEXTRA,
            FNAME,
            FCOMMENT,
            FHCRC,
            FEXTRA | FNAME | FCOMMENT | FHCRC,
        ] {
            assert_eq!(
                read_back_payload(gzip(PAYLOAD, flags)),
                expected,
                "flags {flags:#x}"
            );
        }
    }

    #[test]
    fn a_box_that_is_not_compressed_reads_back_as_none() {
        assert_eq!(
            read_back(&boxed(isobmff_boxes::MovieBox::BOX_TYPE, PAYLOAD), 1024),
            Ok(None)
        );
    }

    #[test]
    fn a_payload_that_inflates_past_the_limit_is_refused_and_one_that_inflates_to_it_is_grown_no_further()
     {
        let bomb = vec![0; 1 << 20];
        let past_limit = |reached_bytes, limit_bytes| {
            Err(DemuxError::PayloadLimitExceeded {
                box_type: MOVIE,
                reached_bytes,
                limit_bytes,
            })
        };
        let mut lying = gzip(&bomb, 0);
        let declared = lying.len() - 4;
        lying.splice(declared.., 10_u32.to_le_bytes());

        let inflated = read_back(&boxed(MOVIE, &compress_to_vec(PAYLOAD, 6)), 45)
            .unwrap()
            .unwrap()
            .1;

        assert_eq!(
            read_back(&boxed(MOVIE, &compress_to_vec(&bomb, 6)), 1000),
            past_limit(1001, 1000)
        );
        assert_eq!(
            read_back(&boxed(MOVIE, &gzip(&bomb, 0)), 1000),
            past_limit(1 << 20, 1000)
        );
        assert_eq!(
            read_back(&boxed(MOVIE, &lying), 1000),
            past_limit(1001, 1000)
        );
        assert_eq!(
            (inflated.len(), inflated.capacity() <= 45),
            (PAYLOAD.len(), true)
        );
        for zeros in [1500, 3000, 70_000] {
            let grown = read_back(
                &boxed(MOVIE, &compress_to_vec(&vec![0; zeros], 6)),
                u64::try_from(zeros).unwrap(),
            )
            .unwrap()
            .unwrap()
            .1;
            assert_eq!(
                (grown.len(), grown.capacity()),
                (zeros, zeros),
                "{zeros} bytes"
            );
        }
        assert_eq!(
            read_back(&boxed(MOVIE, &compress_to_vec(PAYLOAD, 6)), 44),
            past_limit(45, 44)
        );
    }

    #[test]
    fn a_payload_that_is_neither_a_deflate_stream_nor_a_gzip_member_that_inflates_as_it_declares_is_unreadable()
     {
        let unreadable = Err(DemuxError::UnreadableCompressedData { box_type: MOVIE });
        let member = gzip(PAYLOAD, FHCRC);
        let corrupt = |at: usize| {
            let mut corrupted = member.clone();
            let byte = corrupted.get_mut(at).unwrap();
            *byte ^= 0x01;
            corrupted
        };
        let mut reserved = gzip(PAYLOAD, 0);
        *reserved.get_mut(3).unwrap() = 0x20;
        let mut truncated = compress_to_vec(PAYLOAD, 6);
        truncated.truncate(truncated.len() / 2);
        let crc_at = member.len() - 8;
        let length_at = member.len() - 4;

        for compressed in [
            vec![0x78, 0x9c, 0x03, 0x00],
            vec![0x07],
            truncated,
            reserved,
            corrupt(11),
            corrupt(crc_at),
            corrupt(length_at),
            member.get(..9).unwrap().to_vec(),
            Vec::new(),
        ] {
            assert_eq!(
                read_back(&boxed(MOVIE, &compressed), 1024),
                unreadable,
                "{compressed:x?}"
            );
        }
    }
}
