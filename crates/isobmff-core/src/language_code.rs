//! [`LanguageCode`], the packed ISO-639-2/T code of ISO/IEC 14496-12 §8.4.2

/// Bits one letter occupies in the packed code
const LETTER_BITS: u16 = 0x1F;

/// Value a letter is offset by, which packs `a` as one
const LETTER_OFFSET: u8 = 0x60;

/// Bits the packed code occupies, the pad bit above them excluded
const PACKED_BITS: u16 = 0x7FFF;

/// Language of a track, as the spec's packed ISO-639-2/T code
///
/// The code is three lowercase letters packed five bits each into the low
/// fifteen bits of a 16-bit field, with the bit above them unused. The packed
/// form is what this type holds: five bits carry 32 values where the alphabet
/// claims 26, so a field may carry a code that spells no letters, and
/// [`letters`](Self::letters) is where that shows.
///
/// # Examples
///
/// ```
/// use isobmff_core::LanguageCode;
///
/// // A code the three letters of a language pack into
/// let japanese = LanguageCode::from_letters(b"jpn").unwrap();
/// assert_eq!(japanese.letters(), Some(*b"jpn"));
///
/// // The bit above the code is not part of it, so a field carrying it reads the same
/// assert_eq!(LanguageCode::from_raw(japanese.raw() | 0x8000), japanese);
///
/// // `und`, which a writer states when the language is undetermined
/// assert_eq!(LanguageCode::UND.letters(), Some(*b"und"));
///
/// // A packed value spelling no letters is still a value the field carries
/// assert_eq!(LanguageCode::from_raw(0).letters(), None);
/// ```
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct LanguageCode(u16);

impl LanguageCode {
    /// `und`, the code standing for a language left undetermined
    pub const UND: Self = Self(0x55C4);

    /// Creates the code from the raw value a field carries
    ///
    /// The bit above the packed code is not part of it and is dropped, so a
    /// field that carries it reads as the code without it.
    #[must_use]
    pub const fn from_raw(raw: u16) -> Self {
        Self(raw & PACKED_BITS)
    }

    /// Returns the packed code, which occupies the low fifteen bits
    #[must_use]
    pub const fn raw(self) -> u16 {
        self.0
    }

    /// Creates the code from the three letters it spells
    ///
    /// Returns `None` unless every letter is a lowercase ASCII one, which is
    /// all the five bits of a packed letter can carry.
    #[must_use]
    pub const fn from_letters(letters: &[u8; 3]) -> Option<Self> {
        let [first, second, third] = *letters;
        if !first.is_ascii_lowercase()
            || !second.is_ascii_lowercase()
            || !third.is_ascii_lowercase()
        {
            return None;
        }

        Some(Self(
            packed_letter(first).wrapping_shl(10)
                | packed_letter(second).wrapping_shl(5)
                | packed_letter(third),
        ))
    }

    /// Returns the three letters the code spells
    ///
    /// Returns `None` when any of the three five-bit values is not a letter,
    /// which a file may carry and this type does not refuse on the way in.
    #[must_use]
    pub fn letters(self) -> Option<[u8; 3]> {
        Some([
            letter_of(self.0.wrapping_shr(10))?,
            letter_of(self.0.wrapping_shr(5))?,
            letter_of(self.0)?,
        ])
    }
}

/// Returns the five-bit value a letter packs into
const fn packed_letter(letter: u8) -> u16 {
    letter.wrapping_sub(LETTER_OFFSET) as u16
}

/// Returns the letter the low five bits of `packed` spell
fn letter_of(packed: u16) -> Option<u8> {
    let value = u8::try_from(packed & LETTER_BITS).ok()?;
    if !(1..=26).contains(&value) {
        return None;
    }

    Some(value.wrapping_add(LETTER_OFFSET))
}

#[cfg(test)]
mod tests {
    use super::LanguageCode;

    #[test]
    fn a_code_reads_back_as_the_letters_it_was_built_from() {
        for letters in [b"und", b"eng", b"jpn", b"aaa", b"zzz"] {
            let code = LanguageCode::from_letters(letters);

            assert_eq!(code.and_then(LanguageCode::letters), Some(*letters));
        }
    }

    #[test]
    fn letters_the_five_bits_cannot_carry_are_refused() {
        for letters in [b"ENG", b"en1", b"e g"] {
            assert_eq!(LanguageCode::from_letters(letters), None);
        }
    }

    #[test]
    fn the_bit_above_the_code_is_dropped_on_the_way_in() {
        let code = LanguageCode::from_letters(b"jpn").unwrap();

        assert_eq!(LanguageCode::from_raw(code.raw() | 0x8000), code);
    }

    #[test]
    fn a_five_bit_value_that_is_not_a_letter_spells_nothing() {
        let padded_with_zero = LanguageCode::from_raw(0x0000);
        let padded_with_the_widest_value = LanguageCode::from_raw(0x7FFF);

        assert_eq!(padded_with_zero.letters(), None);
        assert_eq!(padded_with_the_widest_value.letters(), None);
    }
}
