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

/// Returns whether any of the brands, major or compatible, is one under
/// which `default-base-is-moof` shall not be used (§8.8.7.1)
pub(crate) fn forbids_default_base_is_moof(
    major_brand: FourCC,
    compatible_brands: &[FourCC],
) -> bool {
    iter::once(&major_brand)
        .chain(compatible_brands)
        .any(|brand| BRANDS_EARLIER_THAN_ISO5.contains(brand))
}

#[cfg(test)]
mod tests {
    use isobmff_core::FourCC;

    use super::forbids_default_base_is_moof;

    #[test]
    fn iso4_forbids_default_base_is_moof_and_iso5_does_not() {
        assert!(forbids_default_base_is_moof(
            FourCC::new(*b"iso5"),
            &[FourCC::new(*b"iso4")]
        ));
        assert!(!forbids_default_base_is_moof(
            FourCC::new(*b"iso5"),
            &[FourCC::new(*b"iso5")]
        ));
    }

    #[test]
    fn a_major_brand_earlier_than_iso5_forbids_default_base_is_moof_on_its_own() {
        assert!(forbids_default_base_is_moof(
            FourCC::new(*b"avc1"),
            &[FourCC::new(*b"iso6")]
        ));
    }
}
