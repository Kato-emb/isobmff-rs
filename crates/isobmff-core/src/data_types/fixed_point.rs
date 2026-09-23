//! Fixed-point numbers of ISO/IEC 14496-12 §6.2.2, one type per field width

/// Signed fixed-point number with sixteen fractional bits
///
/// The spec writes a field of this shape as a signed 32-bit integer whose low
/// sixteen bits lie below the point — `rate` in a movie header is one. The raw
/// integer is what the wire carries, and this type is that integer with its
/// scale named.
///
/// # Examples
///
/// ```
/// use isobmff_core::I16F16;
///
/// // Half of the rate the spec gives as its template value, as the field carries it
/// let half_speed = I16F16::from_raw(0x0000_8000);
/// assert_eq!(half_speed.raw(), 0x0000_8000);
/// assert_ne!(half_speed, I16F16::ONE);
/// ```
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct I16F16(i32);

impl I16F16 {
    /// Value zero
    pub const ZERO: Self = Self(0);

    /// Value one, which the field carries as `0x0001_0000`
    pub const ONE: Self = Self(0x0001_0000);

    /// Creates the number from the raw integer a field carries
    #[must_use]
    pub const fn from_raw(raw: i32) -> Self {
        Self(raw)
    }

    /// Returns the raw integer a field carries
    #[must_use]
    pub const fn raw(self) -> i32 {
        self.0
    }

    /// Creates the number whose bits above the point represent the given integer and whose bits below are zero
    #[must_use]
    pub const fn from_integer(integer: i16) -> Self {
        let [high, low] = integer.to_be_bytes();
        Self(i32::from_be_bytes([high, low, 0, 0]))
    }

    /// Returns the integer the bits above the point represent
    ///
    /// # Examples
    ///
    /// ```
    /// use isobmff_core::I16F16;
    ///
    /// // A rate of one and a half in reverse, whose bits above the point represent minus two
    /// let reversed = I16F16::from_raw(-0x0001_8000);
    /// assert_eq!(reversed.integer(), -2);
    /// ```
    #[must_use]
    pub const fn integer(self) -> i16 {
        let [high, low, _, _] = self.0.to_be_bytes();
        i16::from_be_bytes([high, low])
    }
}

/// Unsigned fixed-point number with sixteen fractional bits
///
/// The spec writes a field of this shape as an unsigned 32-bit integer whose
/// low sixteen bits lie below the point — the `width` and `height` of a track
/// header are two.
///
/// # Examples
///
/// ```
/// use isobmff_core::U16F16;
///
/// // A track 1920 units wide, with nothing below the point
/// let width = U16F16::from_integer(1920);
/// assert_eq!(width.raw(), 0x0780_0000);
/// assert_eq!(width.integer(), 1920);
/// ```
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct U16F16(u32);

impl U16F16 {
    /// Value zero
    pub const ZERO: Self = Self(0);

    /// Value one, which the field carries as `0x0001_0000`
    pub const ONE: Self = Self(0x0001_0000);

    /// Creates the number from the raw integer a field carries
    #[must_use]
    pub const fn from_raw(raw: u32) -> Self {
        Self(raw)
    }

    /// Returns the raw integer a field carries
    #[must_use]
    pub const fn raw(self) -> u32 {
        self.0
    }

    /// Creates the number whose bits above the point represent the given integer and whose bits below are zero
    #[must_use]
    pub const fn from_integer(integer: u16) -> Self {
        let [high, low] = integer.to_be_bytes();
        Self(u32::from_be_bytes([high, low, 0, 0]))
    }

    /// Returns the integer the bits above the point represent
    #[must_use]
    pub const fn integer(self) -> u16 {
        let [high, low, _, _] = self.0.to_be_bytes();
        u16::from_be_bytes([high, low])
    }
}

/// Signed fixed-point number with eight fractional bits
///
/// The spec writes a field of this shape as a signed 16-bit integer whose low
/// eight bits lie below the point — `volume` in a movie or track header is one.
///
/// # Examples
///
/// ```
/// use isobmff_core::I8F8;
///
/// // Half of the volume the spec gives as its template value, as the field carries it
/// let half_volume = I8F8::from_raw(0x0080);
/// assert_eq!(half_volume.raw(), 0x0080);
/// assert_ne!(half_volume, I8F8::ONE);
/// ```
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct I8F8(i16);

impl I8F8 {
    /// Value zero
    pub const ZERO: Self = Self(0);

    /// Value one, which the field carries as `0x0100`
    pub const ONE: Self = Self(0x0100);

    /// Creates the number from the raw integer a field carries
    #[must_use]
    pub const fn from_raw(raw: i16) -> Self {
        Self(raw)
    }

    /// Returns the raw integer a field carries
    #[must_use]
    pub const fn raw(self) -> i16 {
        self.0
    }

    /// Creates the number whose bits above the point represent the given integer and whose bits below are zero
    #[must_use]
    pub const fn from_integer(integer: i8) -> Self {
        let [integer_byte] = integer.to_be_bytes();
        Self(i16::from_be_bytes([integer_byte, 0]))
    }

    /// Returns the integer the bits above the point represent
    #[must_use]
    pub const fn integer(self) -> i8 {
        let [integer_byte, _] = self.0.to_be_bytes();
        i8::from_be_bytes([integer_byte])
    }
}

#[cfg(test)]
mod tests {
    use super::{I8F8, I16F16, U16F16};

    #[test]
    fn every_integer_reads_back_from_the_number_it_creates() {
        for integer in i16::MIN..=i16::MAX {
            assert_eq!(I16F16::from_integer(integer).integer(), integer);
        }
        for integer in u16::MIN..=u16::MAX {
            assert_eq!(U16F16::from_integer(integer).integer(), integer);
        }
        for integer in i8::MIN..=i8::MAX {
            assert_eq!(I8F8::from_integer(integer).integer(), integer);
        }
    }

    #[test]
    fn integer_one_creates_the_value_one() {
        assert_eq!(I16F16::from_integer(1), I16F16::ONE);
        assert_eq!(U16F16::from_integer(1), U16F16::ONE);
        assert_eq!(I8F8::from_integer(1), I8F8::ONE);
    }

    #[test]
    fn a_negative_integer_fills_the_bits_above_the_point() {
        assert_eq!(I16F16::from_integer(-1), I16F16::from_raw(-0x1_0000));
        assert_eq!(I8F8::from_integer(-1), I8F8::from_raw(-0x100));
    }

    #[test]
    fn a_negative_number_with_a_fraction_reads_as_the_integer_below_it() {
        assert_eq!(I16F16::from_raw(-0x1_8000).integer(), -2);
        assert_eq!(I8F8::from_raw(-0x180).integer(), -2);
    }
}
