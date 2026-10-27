//! Boxes of a file framed out of bytes that arrive in pieces, and written back as bytes
//!
//! A file is a run of boxes laid end to end, each opening with a header that
//! gives its size and type. [`BoxReader`] finds where each box starts and ends
//! as the bytes of the file arrive, and hands its payload on as the caller
//! asks: gathered whole, streamed in pieces, or skipped. [`BoxWriter`] lays
//! boxes end to end the same way.

mod box_reader;
mod box_writer;

pub use box_reader::{BoxEvent, BoxEvents, BoxReader, OpenBox};
pub use box_writer::BoxWriter;

/// Boxes in every header form, which the tests of reading and writing share
#[cfg(test)]
mod header_forms {
    use alloc::vec::Vec;

    const EXTENDED_TYPE: [u8; 16] = [
        0x01, 0x23, 0x45, 0x67, 0x89, 0xab, 0xcd, 0xef, 0xfe, 0xdc, 0xba, 0x98, 0x76, 0x54, 0x32,
        0x10,
    ];

    pub(super) const PAYLOAD: &[u8] = b"payload!";

    /// One box in each header form, in the wire order `size`, `boxtype`, `largesize`, `extended_type`, then [`PAYLOAD`]
    pub(super) fn every_header_form() -> [Vec<u8>; 6] {
        let with_payload = |header: &[u8]| [header, PAYLOAD].concat();
        [
            with_payload(&[0, 0, 0, 16, b'f', b'r', b'e', b'e']),
            with_payload(&[0, 0, 0, 1, b'm', b'd', b'a', b't', 0, 0, 0, 0, 0, 0, 0, 24]),
            with_payload(
                &[
                    [0, 0, 0, 32, b'u', b'u', b'i', b'd'].as_slice(),
                    &EXTENDED_TYPE,
                ]
                .concat(),
            ),
            with_payload(
                &[
                    [0, 0, 0, 1, b'u', b'u', b'i', b'd', 0, 0, 0, 0, 0, 0, 0, 40].as_slice(),
                    &EXTENDED_TYPE,
                ]
                .concat(),
            ),
            with_payload(&[0, 0, 0, 0, b'm', b'd', b'a', b't']),
            with_payload(
                &[
                    [0, 0, 0, 0, b'u', b'u', b'i', b'd'].as_slice(),
                    &EXTENDED_TYPE,
                ]
                .concat(),
            ),
        ]
    }
}
