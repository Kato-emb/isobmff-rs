//! [`BoxEvent`], one step of the sequence of boxes of ISO/IEC 14496-12 §4.2, and the bytes written for it

use alloc::vec::Vec;
use core::ops::Deref;

use isobmff_core::BoxHeader;

/// Step of the sequence of boxes, owning the bytes it carries
///
/// A box is one [`Header`](Self::Header), then as many
/// [`Payload`](Self::Payload) steps as its payload was cut into, then
/// [`End`](Self::End). A container is one box like any other: its payload is
/// carried as it lies rather than descended into, and so is the payload of a
/// box a specification reads into a value.
///
/// The bytes one step is made of: the header alone for a
/// [`Header`](Self::Header), that part of the payload for a
/// [`Payload`](Self::Payload), and none at all for an [`End`](Self::End), which
/// stands where the box ended. The steps of a file lie end to end, so together
/// they measure it.
///
/// A step says what the file holds and not where it holds it, so the same step
/// is what [`BoxReader`](crate::BoxReader) reports and what
/// [`BoxWriter`](crate::BoxWriter) takes. Where it lies is the extent each of
/// them names for the step it last handled —
/// [`BoxReader::event_extent`](crate::BoxReader::event_extent) and
/// [`BoxWriter::event_extent`](crate::BoxWriter::event_extent).
#[non_exhaustive]
#[derive(Clone, PartialEq, Debug)]
pub enum BoxEvent {
    /// Header of the box that started
    Header(BoxHeader),
    /// Part of the payload of the box that started, as it lay in the input
    Payload(Vec<u8>),
    /// End of the box that started, its declared total reached
    End,
}

/// The bytes one event was written to, owned by whoever takes them
///
/// The bytes of a [`Header`](BoxEvent::Header) are held inline, and those
/// of a [`Payload`](BoxEvent::Payload) are the very allocation the event
/// carried: a payload crosses [`BoxWriter`](crate::BoxWriter) without being
/// copied, and [`into_vec`](Self::into_vec) hands that allocation back as it
/// stands. A header never had one, so `into_vec` takes one there.
///
/// The bytes are read through the deref to `[u8]`, which is what `len`,
/// `iter`, and slicing reach.
///
/// # Examples
///
/// ```
/// use isobmff_core::{BoxHeader, BoxSize, BoxType, CompactSize};
/// use isobmff_sequence::{BoxEvent, BoxWriter};
///
/// // An `mdat` box carrying one payload of four bytes
/// let total = BoxSize::Compact(CompactSize::new(12).unwrap());
/// let header = BoxHeader::new(BoxType::compact(*b"mdat"), total).unwrap();
/// let payload = b"SAMP".to_vec();
/// let carried = payload.as_ptr();
///
/// // The events are handed over
/// let mut writer = BoxWriter::new();
/// writer.handle_event(BoxEvent::Header(header)).unwrap();
/// writer.handle_event(BoxEvent::Payload(payload)).unwrap();
///
/// // The header comes out as the eight bytes it was encoded to
/// let written_header = writer.poll_output().unwrap();
/// assert_eq!(*written_header, *b"\0\0\0\x0cmdat");
///
/// // The payload comes out in the allocation it arrived in
/// let written_payload = writer.poll_output().unwrap();
/// assert_eq!(*written_payload, *b"SAMP");
///
/// let handed_over = written_payload.into_vec();
/// assert_eq!(handed_over.as_ptr(), carried);
/// ```
#[non_exhaustive]
#[derive(Clone, PartialEq, Debug)]
pub struct EventBytes {
    bytes: Bytes,
}

impl EventBytes {
    /// Encodes the header of a box that started
    pub(crate) fn header(header: BoxHeader) -> Self {
        let mut bytes = [0; BoxHeader::MAX_ENCODED_LEN];
        let len = header.encode(&mut bytes).len();

        Self {
            bytes: Bytes::Header { bytes, len },
        }
    }

    /// Takes the payload a box was handed, as it lies
    pub(crate) const fn payload(payload: Vec<u8>) -> Self {
        Self {
            bytes: Bytes::Payload(payload),
        }
    }

    /// Hands the bytes over as a `Vec`
    ///
    /// The payload of a box is handed back in the allocation it arrived in,
    /// neither copied nor grown. The bytes of a header are held inline, so they
    /// take an allocation here, once.
    #[must_use]
    pub fn into_vec(self) -> Vec<u8> {
        match self.bytes {
            Bytes::Payload(payload) => payload,
            Bytes::Header { .. } => self.to_vec(),
        }
    }
}

impl Deref for EventBytes {
    type Target = [u8];

    fn deref(&self) -> &Self::Target {
        match &self.bytes {
            // Why not unreachable: `len` is what encoding the header reported,
            // which the array it was written to holds, and the fallback is an
            // empty slice in place of a panic the lints forbid.
            Bytes::Header { bytes, len } => bytes.get(..*len).unwrap_or_default(),
            Bytes::Payload(payload) => payload,
        }
    }
}

impl AsRef<[u8]> for EventBytes {
    fn as_ref(&self) -> &[u8] {
        self
    }
}

/// The two shapes the bytes of an event come in
#[derive(Clone, PartialEq, Debug)]
enum Bytes {
    /// Bytes a header was encoded to, held inline
    Header {
        bytes: [u8; BoxHeader::MAX_ENCODED_LEN],
        len: usize,
    },
    /// Bytes a payload event carried, held as they arrived
    Payload(Vec<u8>),
}
