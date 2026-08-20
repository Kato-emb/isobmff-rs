//! [`RawBox`] and [`boxes`], a box of ISO/IEC 14496-12 §4.2 as the bytes it was framed as

use core::iter::FusedIterator;

use crate::box_header::BoxHeader;
use crate::error::{Error, byte_count};

/// Box as it lies in an input: its header, and the payload the header spans
///
/// The payload is borrowed from the input and left unread. Encoding the header
/// in front of it reproduces the bytes the box was framed from.
///
/// # Examples
///
/// ```
/// use isobmff_core::{BoxType, RawBox};
///
/// // Two boxes laid end to end: a free of twelve bytes, then an empty skip
/// let input = b"\0\0\0\x0cfreeAAAA\0\0\0\x08skip";
///
/// // Split the leading box off the input
/// let (first, rest) = RawBox::split_first(input).unwrap();
/// assert_eq!(first.header().box_type(), BoxType::compact(*b"free"));
/// assert_eq!(first.payload(), b"AAAA");
///
/// // What follows starts at the second box
/// let (second, after) = RawBox::split_first(rest).unwrap();
/// assert_eq!(second.header().box_type(), BoxType::compact(*b"skip"));
/// assert_eq!(second.payload(), b"");
/// assert_eq!(after, b"");
/// ```
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct RawBox<'a> {
    header: BoxHeader,
    payload: &'a [u8],
}

impl<'a> RawBox<'a> {
    /// Returns the header that introduces the box
    #[must_use]
    pub const fn header(self) -> BoxHeader {
        self.header
    }

    /// Returns the payload the header spans, header excluded
    #[must_use]
    pub const fn payload(self) -> &'a [u8] {
        self.payload
    }

    /// Splits the box that starts `input` off the front
    ///
    /// Returns the box and the bytes after it. The payload is carried as it
    /// lies and never interpreted: this is a split along the declared total,
    /// not a decode of what the box holds.
    ///
    /// A [`ToEndOfFile`](crate::BoxSize::ToEndOfFile) size frames the rest of
    /// `input` and leaves nothing after it. Whether the box sits where that
    /// form is allowed is not checked here; the size the frame carries states
    /// which form it was.
    ///
    /// # Errors
    ///
    /// * The failures of [`BoxHeader::decode`], for the header that introduces
    ///   the box.
    /// * [`TruncatedBox`](crate::ErrorKind::TruncatedBox): the declared total
    ///   overruns `input`. A caller that reads in chunks can extend `input` to
    ///   `needed` bytes and split again, so long as a slice that long can exist
    ///   on the target.
    pub fn split_first(input: &[u8]) -> Result<(RawBox<'_>, &[u8]), Error> {
        let (header, after_header) = BoxHeader::decode(input)?;

        let Some(total) = header.size().total_bytes() else {
            return Ok((
                RawBox {
                    header,
                    payload: after_header,
                },
                &[],
            ));
        };

        // Why not report a total beyond `usize` as its own error: such a total
        // exceeds any `input.len()` on the same target, so it is short input by
        // another name and folding it in keeps one error for one situation.
        let split = usize::try_from(total)
            .ok()
            .and_then(|total| input.split_at_checked(total));

        let Some((_framed, rest)) = split else {
            return Err(Error::truncated_box(total, byte_count(input.len())));
        };

        let payload = after_header
            .len()
            .checked_sub(rest.len())
            .and_then(|payload_length| after_header.get(..payload_length))
            // Why not unreachable: `rest` is a suffix of `after_header`, since
            // `decode` rejects a total below its header, so the fallback is a
            // degenerate value in place of a panic the lints forbid.
            .unwrap_or(&[]);

        Ok((RawBox { header, payload }, rest))
    }
}

/// Splits `input` into the boxes laid end to end in it
///
/// An empty input holds no boxes and iterates as an empty sequence.
#[must_use]
pub fn boxes(input: &[u8]) -> Boxes<'_> {
    Boxes {
        remaining: input,
        done: false,
    }
}

/// Iterator over the boxes of an input, as [`boxes`] returns
///
/// Each step splits one box off the front with
/// [`RawBox::split_first`], which states the contract the steps follow. A box
/// that fails to split ends the iteration: the error is yielded once, and every
/// step after it returns `None`.
#[derive(Clone, Debug)]
pub struct Boxes<'a> {
    remaining: &'a [u8],
    done: bool,
}

impl<'a> Iterator for Boxes<'a> {
    type Item = Result<RawBox<'a>, Error>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.done || self.remaining.is_empty() {
            return None;
        }

        match RawBox::split_first(self.remaining) {
            Ok((framed, rest)) => {
                self.remaining = rest;
                Some(Ok(framed))
            }
            Err(error) => {
                self.done = true;
                Some(Err(error))
            }
        }
    }
}

impl FusedIterator for Boxes<'_> {}

#[cfg(test)]
mod tests {
    use alloc::vec;
    use alloc::vec::Vec;

    use super::{Error, RawBox, boxes};
    use crate::box_header::BoxHeader;
    use crate::box_size::{BoxSize, CompactSize};
    use crate::box_type::BoxType;

    /// Header of a box declaring `total` in the compact `size` field
    fn compact_header(box_type: [u8; 4], total: u32) -> BoxHeader {
        BoxHeader::new(
            BoxType::compact(box_type),
            BoxSize::Compact(CompactSize::new(total).unwrap()),
        )
        .unwrap()
    }

    #[test]
    fn a_sized_box_splits_into_its_frame_and_the_bytes_after_it() {
        let input = b"\0\0\0\x0cfreeAAAA\0\0\0\x08skip";

        assert_eq!(
            RawBox::split_first(input),
            Ok((
                RawBox {
                    header: compact_header(*b"free", 12),
                    payload: b"AAAA",
                },
                b"\0\0\0\x08skip".as_slice()
            ))
        );
    }

    #[test]
    fn the_end_of_file_size_frames_the_rest_of_the_input() {
        let input = b"\0\0\0\0mdatPAYLOAD";

        assert_eq!(
            RawBox::split_first(input),
            Ok((
                RawBox {
                    header: BoxHeader::new(BoxType::compact(*b"mdat"), BoxSize::ToEndOfFile)
                        .unwrap(),
                    payload: b"PAYLOAD",
                },
                b"".as_slice()
            ))
        );
    }

    #[test]
    fn a_total_overrunning_the_input_is_rejected_as_truncated() {
        let input = b"\0\0\0\x10freeAAAA";

        assert_eq!(
            RawBox::split_first(input),
            Err(Error::truncated_box(16, 12))
        );
    }

    #[test]
    fn a_large_size_beyond_the_address_space_is_rejected_as_truncated() {
        let input = [
            0x00, 0x00, 0x00, 0x01, b'm', b'd', b'a', b't', 0xff, 0xff, 0xff, 0xff, 0xff, 0xff,
            0xff, 0xff,
        ];

        assert_eq!(
            RawBox::split_first(&input),
            Err(Error::truncated_box(u64::MAX, 16))
        );
    }

    #[test]
    fn an_input_ending_inside_the_header_fails_as_the_header_decode_does() {
        assert_eq!(
            RawBox::split_first(&[0x00, 0x00, 0x00]),
            Err(Error::truncated_header(8, 3))
        );
    }

    #[test]
    fn boxes_laid_end_to_end_iterate_in_the_order_they_appear() {
        let input = b"\0\0\0\x0cfreeAAAA\0\0\0\x08skip";

        assert_eq!(
            boxes(input).collect::<Vec<_>>(),
            vec![
                Ok(RawBox {
                    header: compact_header(*b"free", 12),
                    payload: b"AAAA",
                }),
                Ok(RawBox {
                    header: compact_header(*b"skip", 8),
                    payload: b"",
                }),
            ]
        );
    }

    #[test]
    fn an_empty_input_holds_no_boxes() {
        assert_eq!(boxes(b"").next(), None);
    }

    #[test]
    fn a_box_that_fails_to_split_ends_the_iteration_for_good() {
        let input = b"\0\0\0\x08free\0\0\0\x20free";
        let mut iterator = boxes(input);

        let framed = iterator.by_ref().collect::<Vec<_>>();

        assert_eq!(
            framed,
            vec![
                Ok(RawBox {
                    header: compact_header(*b"free", 8),
                    payload: b"",
                }),
                Err(Error::truncated_box(32, 8)),
            ]
        );
        assert_eq!(iterator.next(), None);
    }

    #[test]
    fn the_payload_of_a_container_splits_into_the_boxes_it_holds() {
        let input = b"\0\0\0\x1cmoov\0\0\0\x0cfreeAAAA\0\0\0\x08skip";
        let (container, rest) = RawBox::split_first(input).unwrap();

        assert_eq!(rest, b"");
        assert_eq!(
            boxes(container.payload()).collect::<Vec<_>>(),
            vec![
                Ok(RawBox {
                    header: compact_header(*b"free", 12),
                    payload: b"AAAA",
                }),
                Ok(RawBox {
                    header: compact_header(*b"skip", 8),
                    payload: b"",
                }),
            ]
        );
    }
}
