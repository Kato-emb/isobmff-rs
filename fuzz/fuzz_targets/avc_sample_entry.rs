//! Whatever payload reads as an AVC sample entry writes back to bytes that
//! read again as the same value

#![no_main]

use isobmff_avc::{AVCSampleEntry, AVCSampleEntryType};
use isobmff_core::BoxEncode;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|payload: &[u8]| {
    let Ok(entry) = AVCSampleEntry::decode_payload(AVCSampleEntryType::Avc1, payload) else {
        return;
    };

    let Ok(len) = usize::try_from(entry.payload_len()) else {
        return;
    };
    let mut written = vec![0; len];
    entry
        .encode_payload(&mut written)
        .expect("an entry that was read writes back");

    // Why not compare the bytes: reserved bits are masked on read and written
    // as ones, so a file that cleared one reads as the same value without
    // writing back to the same bytes.
    assert_eq!(
        AVCSampleEntry::decode_payload(AVCSampleEntryType::Avc1, &written).unwrap(),
        entry,
        "the entry reads back as another value"
    );
});
