//! Whatever payload reads as an MPEG-4 audio sample entry writes back to bytes
//! that read again as the same value

#![no_main]

use isobmff_core::BoxEncode;
use isobmff_mp4::MP4AudioSampleEntry;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|payload: &[u8]| {
    let Ok(entry) = MP4AudioSampleEntry::decode_payload(payload) else {
        return;
    };

    let Ok(length) = usize::try_from(entry.payload_len()) else {
        return;
    };
    let mut written = vec![0; length];
    entry
        .encode_payload(&mut written)
        .expect("an entry that was read writes back");

    // Why not compare the bytes: expandable sizes and reserved bits are written
    // in one canonical form, so a file that used another reads as the same
    // value without writing back to the same bytes.
    assert_eq!(
        MP4AudioSampleEntry::decode_payload(&written).unwrap(),
        entry,
        "the entry reads back as another value"
    );
});
