//! The brands a `ftyp` or a `styp` lists, ISO/IEC 14496-12 §4.3, §8.16.2, §8.8.7.1

use core::iter;

use isobmff_core::FourCc;

/// Brands of Annex E whose features `iso5` requires support for, through `iso4`
const BRANDS_EARLIER_THAN_ISO5: [FourCc; 5] = [
    FourCc::new(*b"isom"),
    FourCc::new(*b"avc1"),
    FourCc::new(*b"iso2"),
    FourCc::new(*b"iso3"),
    FourCc::new(*b"iso4"),
];

/// Returns whether any of the brands, major or compatible, is one under
/// which `default-base-is-moof` shall not be used (§8.8.7.1)
pub(crate) fn forbids_default_base_is_moof(
    major_brand: FourCc,
    compatible_brands: &[FourCc],
) -> bool {
    iter::once(&major_brand)
        .chain(compatible_brands)
        .any(|brand| BRANDS_EARLIER_THAN_ISO5.contains(brand))
}

#[cfg(test)]
mod tests {
    use isobmff_core::FourCc;

    use super::forbids_default_base_is_moof;

    #[test]
    fn iso4_forbids_default_base_is_moof_and_iso5_does_not() {
        assert!(forbids_default_base_is_moof(
            FourCc::new(*b"iso5"),
            &[FourCc::new(*b"iso4")]
        ));
        assert!(!forbids_default_base_is_moof(
            FourCc::new(*b"iso5"),
            &[FourCc::new(*b"iso5")]
        ));
    }

    #[test]
    fn a_major_brand_earlier_than_iso5_forbids_default_base_is_moof_on_its_own() {
        assert!(forbids_default_base_is_moof(
            FourCc::new(*b"avc1"),
            &[FourCc::new(*b"iso6")]
        ));
    }
}
