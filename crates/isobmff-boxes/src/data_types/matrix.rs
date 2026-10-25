//! [`Matrix`], the transformation matrix of a video

use isobmff_core::{Error, FieldReader, FieldWriter};

/// Transformation matrix of a video, ISO/IEC 14496-12 §7.3.2
///
/// The nine values, in the order the field stores them, are
/// `{a, b, u, c, d, v, x, y, w}`: a point `(p, q)` maps to `(m/z, n/z)` where
/// `m = ap + cq + x`, `n = bp + dq + y` and `z = up + vq + w`. Every value is
/// a 16.16 fixed-point number, except `u`, `v` and `w`, which are 2.30
/// fixed-point numbers. The values are held as the raw integers the field
/// carries.
///
/// [`Default`] is [`UNITY`](Self::UNITY), the template value of every box
/// that carries a matrix.
///
/// # Examples
///
/// ```
/// use isobmff_boxes::Matrix;
///
/// // A matrix that doubles the size of the image, in raw fixed-point values
/// let doubling = Matrix::from_raw([0x0002_0000, 0, 0, 0, 0x0002_0000, 0, 0, 0, 0x4000_0000]);
/// assert_ne!(doubling, Matrix::UNITY);
/// assert_eq!(doubling.raw()[4], 0x0002_0000);
/// ```
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct Matrix([i32; 9]);

impl Matrix {
    /// Matrix that leaves the image as it is
    pub const UNITY: Self = Self([0x0001_0000, 0, 0, 0, 0x0001_0000, 0, 0, 0, 0x4000_0000]);

    /// Creates the matrix from the raw integers the field carries, in the order `{a, b, u, c, d, v, x, y, w}`
    #[must_use]
    pub const fn from_raw(values: [i32; 9]) -> Self {
        Self(values)
    }

    /// Returns the raw integers the field carries, in the order `{a, b, u, c, d, v, x, y, w}`
    #[must_use]
    pub const fn raw(&self) -> &[i32; 9] {
        &self.0
    }

    /// Number of bytes the field takes
    pub(crate) const ENCODED_LEN: u64 = 9 * 4;

    /// Reads the field off the front of what `reader` has left
    ///
    /// # Errors
    ///
    /// * [`TruncatedPayload`](isobmff_core::ErrorKind::TruncatedPayload): the
    ///   payload ends inside the field.
    pub(crate) fn read(reader: &mut FieldReader<'_>) -> Result<Self, Error> {
        let mut values = [0; 9];
        for value in &mut values {
            *value = reader.read_i32()?;
        }

        Ok(Self(values))
    }

    /// Writes the field onto the front of what `writer` has left
    ///
    /// # Errors
    ///
    /// * [`TruncatedBuffer`](isobmff_core::ErrorKind::TruncatedBuffer): the
    ///   buffer ends inside the field.
    pub(crate) fn write(&self, writer: &mut FieldWriter<'_>) -> Result<(), Error> {
        self.0.iter().try_for_each(|value| writer.write_i32(*value))
    }
}

impl Default for Matrix {
    fn default() -> Self {
        Self::UNITY
    }
}
