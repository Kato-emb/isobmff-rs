//! Whatever payload reads as an AVC sample entry writes back to the same
//! bytes, and reads again as the same value

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

    assert_eq!(
        written, payload,
        "the entry writes back to other bytes than it was read from"
    );
    assert_eq!(
        AVCSampleEntry::decode_payload(AVCSampleEntryType::Avc1, &written).unwrap(),
        entry,
        "the entry reads back as another value"
    );
});
