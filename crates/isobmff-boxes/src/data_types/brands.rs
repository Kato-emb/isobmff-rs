//! The brands a `ftyp` or a `styp` lists, ISO/IEC 14496-12 §4.3, §8.16.2, §8.8.7.1

use core::iter;

use isobmff_core::FourCC;

/// Brands of Annex E whose features `iso5` requires support for, through `iso4`
const BRANDS_EARLIER_THAN_ISO5: [FourCC; 5] = [
    FourCC::new(*b"isom"),
    FourCC::new(*b"avc1"),
    FourCC::new(*b"iso2"),
    FourCC::new(*b"iso3"),
    FourCC::new(*b"iso4"),
];

/// Returns the first of the brands, major then compatible, under which
/// `default-base-is-moof` shall not be used (§8.8.7.1)
pub(crate) fn brand_forbidding_default_base_is_moof(
    major_brand: FourCC,
    compatible_brands: &[FourCC],
) -> Option<FourCC> {
    iter::once(&major_brand)
        .chain(compatible_brands)
        .copied()
        .find(|brand| BRANDS_EARLIER_THAN_ISO5.contains(brand))
}

#[cfg(test)]
mod tests {
    use isobmff_core::FourCC;

    use super::brand_forbidding_default_base_is_moof;

    #[test]
    fn iso4_forbids_default_base_is_moof_and_iso5_does_not() {
        assert_eq!(
            brand_forbidding_default_base_is_moof(FourCC::new(*b"iso5"), &[FourCC::new(*b"iso4")]),
            Some(FourCC::new(*b"iso4"))
        );
        assert_eq!(
            brand_forbidding_default_base_is_moof(FourCC::new(*b"iso5"), &[FourCC::new(*b"iso5")]),
            None
        );
    }

    #[test]
    fn the_major_brand_is_named_before_a_forbidding_compatible_brand() {
        assert_eq!(
            brand_forbidding_default_base_is_moof(
                FourCC::new(*b"avc1"),
                &[FourCC::new(*b"isom"), FourCC::new(*b"iso2")]
            ),
            Some(FourCC::new(*b"avc1"))
        );
    }
}
