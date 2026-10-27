//! Framing properties of [`Boxes`] and [`RawBox::split_first`]
//!
//! The input is the bytes of a file as they lie, so a file of boxes is a seed
//! as it is. One run checks four properties of it:
//!
//! 1. no split panics
//! 2. the iteration ends, taking at most one step for every eight bytes
//! 3. every box framed lies where the one before it ended, and its header
//!    encodes to the bytes it was framed from, unless it declares a reserved
//!    `extended_type`
//! 4. at every box framed, and at the end of the input, an input cut at any
//!    length up to the longest header, or not cut at all, fails only for a
//!    length it does not offer: a truncation error states the length the cut
//!    offered and asks for more, and splitting again at the length it asks for
//!    either succeeds or asks for more still

#![no_main]

use isobmff::{Boxes, Error, ErrorKind, RawBox};
use libfuzzer_sys::fuzz_target;

/// Length of the longest header: a `largesize` field and an `extended_type` field, ISO/IEC 14496-12 §4.2.2
const LONGEST_HEADER_LEN: usize = 32;

fuzz_target!(|bytes: &[u8]| {
    let mut offset = 0;
    let mut steps = 0;

    for framed in Boxes::new(bytes) {
        steps += 1;
        let Ok(framed) = framed else { break };
        let remaining = &bytes[offset..];
        truncation_asks_for_more_than_the_cut_offered(remaining);

        let header = framed.header();
        let header_len = usize::try_from(header.encoded_len()).unwrap();

        let mut encoded = [0; LONGEST_HEADER_LEN];
        match header.encode(&mut encoded) {
            Ok(_) => assert_eq!(
                encoded.get(..header_len),
                remaining.get(..header_len),
                "header does not encode to the bytes it was framed from"
            ),
            Err(error) => assert!(
                matches!(error.kind(), ErrorKind::InvalidValue { .. }),
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

    truncation_asks_for_more_than_the_cut_offered(&bytes[offset..]);
    assert!(
        steps <= bytes.len() / 8 + 1,
        "iterating {} bytes took {steps} steps",
        bytes.len()
    );
});

fn truncation_asks_for_more_than_the_cut_offered(input: &[u8]) {
    let cuts = (0..=input.len().min(LONGEST_HEADER_LEN)).chain([input.len()]);

    for cut in cuts {
        let Some(needed) = bytes_needed(RawBox::split_first(&input[..cut]), cut) else {
            continue;
        };
        let Some(retried) = usize::try_from(needed)
            .ok()
            .and_then(|end| input.get(..end))
        else {
            continue;
        };

        if let Some(needed_again) = bytes_needed(RawBox::split_first(retried), retried.len()) {
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
/// more than that.
fn bytes_needed(result: Result<(RawBox<'_>, &[u8]), Error>, offered: usize) -> Option<u64> {
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
