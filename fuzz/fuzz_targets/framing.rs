//! Framing properties of [`Boxes`] and [`RawBox::split_first`]
//!
//! The input is the bytes of a file as they lie, so a file of boxes is a seed
//! as it is. One run checks four properties of it:
//!
//! 1. no split panics
//! 2. the iteration ends, taking at most one step for every eight bytes
//! 3. every box framed lies where the one before it ended, and its header
//!    encodes to the bytes it was framed from; it fails to encode, with
//!    `InvalidValue`, exactly when it declares an `extended_type` that
//!    ISO/IEC 14496-12 §4.2.3 reserves
//! 4. at every box framed, and at the end of the input, an input cut at any
//!    length up to the longest header, or not cut at all, fails for a length
//!    it does not offer: a truncation error states the length the cut offered
//!    and asks for more, and splitting again at the length it asks for either
//!    succeeds or asks for more still. A cut of a box that frames fails with
//!    no other error, both before and after splitting again; a cut of the end
//!    of the input that does not frame may also fail with another

#![no_main]

use isobmff::{Boxes, Error, ErrorKind, RawBox};
use libfuzzer_sys::fuzz_target;

/// Length of the longest header: a `largesize` field and an `extended_type` field, ISO/IEC 14496-12 §4.2.2
const LONGEST_HEADER_LEN: usize = 32;

/// Twelve bytes ISO/IEC 14496-12 §4.2.3 reserves after the four-character code a UUID maps
const RESERVED_UUID_SUFFIX: [u8; 12] = [
    0x00, 0x11, 0x00, 0x10, 0x80, 0x00, 0x00, 0xAA, 0x00, 0x38, 0x9B, 0x71,
];

fuzz_target!(|bytes: &[u8]| {
    let mut offset = 0;
    let mut steps = 0;

    for framed in Boxes::new(bytes) {
        steps += 1;
        let Ok(framed) = framed else { break };
        let remaining = &bytes[offset..];
        truncation_asks_for_more_than_the_cut_offered(remaining, true);

        let header = framed.header();
        let header_len = usize::try_from(header.encoded_len()).unwrap();

        let mut encoded = [0; LONGEST_HEADER_LEN];
        match header.encode(&mut encoded) {
            Ok(_) => {
                assert!(
                    !declares_reserved_extended_type(remaining),
                    "header declaring a reserved `extended_type` encodes"
                );
                assert_eq!(
                    encoded.get(..header_len),
                    remaining.get(..header_len),
                    "header does not encode to the bytes it was framed from"
                );
            }
            Err(error) => assert!(
                matches!(error.kind(), ErrorKind::InvalidValue { .. })
                    && declares_reserved_extended_type(remaining),
                "header framed from the input fails to encode: {error}"
            ),
        }

        let payload_at = remaining.get(header_len..).unwrap();
        assert_eq!(
            framed.payload().as_ptr(),
            payload_at.as_ptr(),
            "payload does not lie right after its header"
        );
        offset += header_len + framed.payload().len();
    }

    truncation_asks_for_more_than_the_cut_offered(&bytes[offset..], false);
    assert!(
        steps <= bytes.len() / 8 + 1,
        "iterating {} bytes took {steps} steps",
        bytes.len()
    );
});

/// Returns whether the header `input` starts with declares an `extended_type` of the form ISO/IEC 14496-12 §4.2.3 reserves
fn declares_reserved_extended_type(input: &[u8]) -> bool {
    // Why not ask `ExtendedType::new` of the decoded header: the target would
    // then take the reserved form from the code it checks.
    let extended_type_at = if input.get(..4) == Some(&[0, 0, 0, 1]) {
        16
    } else {
        8
    };

    input.get(4..8) == Some(b"uuid")
        && input.get(extended_type_at + 4..extended_type_at + 16) == Some(&RESERVED_UUID_SUFFIX)
}

/// Checks property 4 over the cuts of `input`
///
/// `frames` states that the first box of `input` frames, so that a cut of it
/// can fail for its length alone.
fn truncation_asks_for_more_than_the_cut_offered(input: &[u8], frames: bool) {
    let cuts = (0..=input.len().min(LONGEST_HEADER_LEN)).chain([input.len()]);

    for cut in cuts {
        let Some(needed) = bytes_needed(RawBox::split_first(&input[..cut]), cut, frames) else {
            continue;
        };
        let Some(retried) = usize::try_from(needed)
            .ok()
            .and_then(|end| input.get(..end))
        else {
            continue;
        };

        if let Some(needed_again) =
            bytes_needed(RawBox::split_first(retried), retried.len(), frames)
        {
            assert!(
                needed_again > needed,
                "splitting again at {needed} bytes asks for {needed_again}"
            );
        }
    }
}

/// Length the input must grow to, for an error that says it was cut short
///
/// Checks that the error states `offered` as the length it had, and asks for
/// more than that. Where `frames`, checks that the error is one of a cut.
fn bytes_needed(
    result: Result<(RawBox<'_>, &[u8]), Error>,
    offered: usize,
    frames: bool,
) -> Option<u64> {
    let error = result.err()?;
    let (ErrorKind::TruncatedHeader {
        needed_bytes,
        available_bytes,
        ..
    }
    | ErrorKind::TruncatedBox {
        needed_bytes,
        available_bytes,
        ..
    }) = error.kind()
    else {
        assert!(
            !frames,
            "cut over {offered} bytes of a box that frames fails with {error}"
        );
        return None;
    };

    assert_eq!(
        available_bytes, offered as u64,
        "truncation over {offered} bytes states {available_bytes}"
    );
    assert!(
        needed_bytes > available_bytes,
        "truncation over {offered} bytes asks for only {needed_bytes}"
    );

    Some(needed_bytes)
}
