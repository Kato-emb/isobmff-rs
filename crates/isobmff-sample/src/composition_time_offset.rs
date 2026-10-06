//! The composition time offset of a sample, as a `ctts` entry or a `trun` row carries it, ISO/IEC 14496-12 §8.6.1.3 and §8.8.8

use isobmff_boxes::CompositionTimeOffset;

/// Returns the offset `offset` states, one past [`i32::MAX`] being the negative value of the same 32 bits
pub(crate) fn resolved(offset: CompositionTimeOffset) -> i64 {
    let value = offset.get();

    if value > i64::from(i32::MAX) {
        value.wrapping_sub(1 << 32)
    } else {
        value
    }
}

/// Returns the offset to write for `offset`, or `None` where [`resolved`] would not read it back as `offset`
pub(crate) fn stated(offset: i64) -> Option<CompositionTimeOffset> {
    CompositionTimeOffset::new(offset).filter(|_| i32::try_from(offset).is_ok())
}

#[cfg(test)]
mod tests {
    use isobmff_boxes::CompositionTimeOffset;

    use super::{resolved, stated};

    #[test]
    fn an_offset_past_the_signed_range_resolves_as_the_negative_value_of_its_bits() {
        let offsets = [0, 1_024, i64::from(i32::MAX), 0xFFFF_FC00, -1_024];

        let resolutions =
            offsets.map(|offset| resolved(CompositionTimeOffset::new(offset).unwrap()));

        assert_eq!(resolutions, [0, 1_024, i64::from(i32::MAX), -1_024, -1_024]);
    }

    #[test]
    fn an_offset_is_stated_only_where_it_resolves_back_to_itself() {
        let offsets = [
            i64::from(i32::MIN) - 1,
            i64::from(i32::MIN),
            -1,
            i64::from(i32::MAX),
            1 << 31,
        ];

        let resolutions = offsets.map(|offset| stated(offset).map(resolved));

        assert_eq!(
            resolutions,
            [
                None,
                Some(i64::from(i32::MIN)),
                Some(-1),
                Some(i64::from(i32::MAX)),
                None
            ]
        );
    }
}
