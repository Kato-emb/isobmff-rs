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
/// let width = U16F16::from_raw(0x0780_0000);
/// assert_eq!(width.raw() >> 16, 1920);
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
}
