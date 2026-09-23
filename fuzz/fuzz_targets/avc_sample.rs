//! Whatever NAL units a sample reads as, up to a failure, frame back into a
//! sample that reads as the same NAL units
//!
//! The first input byte picks the length of the `NALUnitLength` field among
//! the ones the spec allows; the rest is the sample.

#![no_main]

use isobmff_avc::LengthSizeMinusOne;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|input: &[u8]| {
    let Some((&selector, sample)) = input.split_first() else {
        return;
    };
    let length_size = LengthSizeMinusOne::new([0, 1, 3][usize::from(selector % 3)]).unwrap();

    let nal_units = length_size
        .nal_units(sample)
        .map_while(Result::ok)
        .collect::<Vec<_>>();

    let framed = length_size
        .frame(&nal_units)
        .expect("NAL units read behind a length field frame behind one as long");
    assert_eq!(
        length_size
            .nal_units(&framed)
            .collect::<Result<Vec<_>, _>>()
            .unwrap(),
        nal_units,
        "the framed sample reads back as other NAL units"
    );
});
