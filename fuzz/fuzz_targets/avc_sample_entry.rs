//! Whatever payload reads as an AVC sample entry writes back to bytes that
//! read again as the same value

#![no_main]

use isobmff_avc::{AVCSampleEntry, Avc1};
use isobmff_core::{BoxDecode, BoxEncode};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|payload: &[u8]| {
    let Ok(entry) = AVCSampleEntry::<Avc1>::decode_payload(payload) else {
        return;
    };

    let Ok(length) = usize::try_from(entry.payload_len()) else {
        return;
    };
    let mut written = vec![0; length];
    entry
        .encode_payload(&mut written)
        .expect("an entry that was read writes back");

    // Why not compare the bytes: reserved bits are masked on read and written
    // as ones, so a file that cleared one reads as the same value without
    // writing back to the same bytes.
    assert_eq!(
        AVCSampleEntry::<Avc1>::decode_payload(&written).unwrap(),
        entry,
        "the entry reads back as another value"
    );
});
