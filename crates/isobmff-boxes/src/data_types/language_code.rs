//! [`LanguageCode`], a language packed into fifteen bits

/// Three-letter ISO 639-2/T language code, packed five bits to a letter, ISO/IEC 14496-12 §8.4.2
///
/// Each letter is packed as the difference between its ASCII value and
/// `0x60`, so a code of three lowercase letters packs to values that are all
/// strictly positive. A field read from a file is kept as found, whether or
/// not its bits spell lowercase letters — a value below `0x400`, which
/// QuickTime gives a Macintosh language code, among them.
///
/// [`Default`] is [`UND`](Self::UND).
///
/// # Examples
///
/// ```
/// use isobmff_boxes::LanguageCode;
///
/// // A code is built from the letters it spells, which are lowercase
/// let english = LanguageCode::new(*b"eng").unwrap();
/// assert_eq!(english.letters(), *b"eng");
/// assert_eq!(LanguageCode::new(*b"ENG"), None);
/// ```
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct LanguageCode(u16);

impl LanguageCode {
    /// `und`, the code of a language left undetermined
    pub const UND: Self = Self(0x55c4);

    /// Creates the code that spells `letters`, or `None` unless every letter is a lowercase ASCII one
    #[must_use]
    pub fn new(letters: [u8; 3]) -> Option<Self> {
        letters
            .iter()
            .try_fold(0_u16, |packed, letter| {
                let difference = letter.is_ascii_lowercase().then_some(letter & 0x1f)?;
                Some(packed.checked_mul(32)? | u16::from(difference))
            })
            .map(Self)
    }

    /// Returns the three characters the code packs, each as `0x60` plus its five bits
    ///
    /// The characters are lowercase letters for every code built with
    /// [`new`](Self::new); a code read from a file may pack values that are
    /// not.
    #[must_use]
    pub fn letters(self) -> [u8; 3] {
        [10, 5, 0].map(|shift| {
            let [_, five] = ((self.0 >> shift) & 0x1f).to_be_bytes();
            five | 0x60
        })
    }

    /// Creates the code from the sixteen bits a field stores it in, dropping the bit above the fifteen it packs
    pub(crate) const fn from_bits(bits: u16) -> Self {
        Self(bits & 0x7fff)
    }

    /// Returns the fifteen bits of the code, the bit above them zero
    pub(crate) const fn bits(self) -> u16 {
        self.0
    }
}

impl Default for LanguageCode {
    fn default() -> Self {
        Self::UND
    }
}
