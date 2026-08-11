//! [`Matrix`], the transformation matrix of ISO/IEC 14496-12 §6.2.2

/// Bytes the nine values of a matrix occupy on the wire
const ENCODED_LEN: usize = 36;

/// Transformation matrix a presentation or a track is rendered under
///
/// The matrix is nine 32-bit values written in the order `a`, `b`, `u`, `c`,
/// `d`, `v`, `x`, `y`, `w`, held as the raw integers the wire carries. They do
/// not share one scale: the six of the first two columns are 16.16 fixed point
/// and the three of the last are 2.30.
///
/// # Examples
///
/// ```
/// use isobmff_core::Matrix;
///
/// // The 36 bytes of a matrix read back as the value that wrote them
/// let bytes = Matrix::UNITY.to_bytes();
/// assert_eq!(Matrix::from_bytes(&bytes), Matrix::UNITY);
///
/// // A matrix mirroring the image about its vertical axis
/// let mirrored = Matrix::from_raw([-0x0001_0000, 0, 0, 0, 0x0001_0000, 0, 0, 0, 0x4000_0000]);
/// assert_ne!(mirrored, Matrix::UNITY);
/// ```
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct Matrix([i32; 9]);

impl Matrix {
    /// Matrix the spec gives as the template value, which leaves the image as it is
    pub const UNITY: Self = Self([0x0001_0000, 0, 0, 0, 0x0001_0000, 0, 0, 0, 0x4000_0000]);

    /// Creates the matrix from the raw integers its fields carry
    #[must_use]
    pub const fn from_raw(values: [i32; 9]) -> Self {
        Self(values)
    }

    /// Returns the raw integers the fields of the matrix carry
    #[must_use]
    pub const fn raw(&self) -> &[i32; 9] {
        &self.0
    }

    /// Reads the matrix from the bytes its fields occupy on the wire
    #[must_use]
    pub fn from_bytes(bytes: &[u8; ENCODED_LEN]) -> Self {
        let mut values = [0; 9];
        let mut rest: &[u8] = bytes;
        for value in &mut values {
            // Why not unwrap: nine words fit these bytes exactly, so the split
            // always holds, and stopping early keeps the panic-free path.
            let Some((word, tail)) = rest.split_first_chunk::<4>() else {
                break;
            };
            *value = i32::from_be_bytes(*word);
            rest = tail;
        }

        Self(values)
    }

    /// Returns the bytes the fields of the matrix occupy on the wire
    #[must_use]
    pub fn to_bytes(&self) -> [u8; ENCODED_LEN] {
        let mut bytes = [0; ENCODED_LEN];
        for (word, value) in bytes.chunks_exact_mut(4).zip(self.0) {
            word.copy_from_slice(&value.to_be_bytes());
        }

        bytes
    }
}

#[cfg(test)]
mod tests {
    use super::Matrix;

    /// A matrix with a different value in every one of its nine fields
    const EVERY_FIELD_DISTINCT: Matrix = Matrix::from_raw([1, -2, 3, -4, 5, -6, 7, -8, 9]);

    #[test]
    fn a_matrix_reads_back_as_the_value_that_wrote_it() {
        let bytes = EVERY_FIELD_DISTINCT.to_bytes();

        assert_eq!(Matrix::from_bytes(&bytes), EVERY_FIELD_DISTINCT);
    }

    #[test]
    fn the_fields_are_written_in_the_order_the_spec_lists_them() {
        let bytes = EVERY_FIELD_DISTINCT.to_bytes();

        assert_eq!(
            bytes,
            [
                0x00, 0x00, 0x00, 0x01, // a
                0xff, 0xff, 0xff, 0xfe, // b
                0x00, 0x00, 0x00, 0x03, // u
                0xff, 0xff, 0xff, 0xfc, // c
                0x00, 0x00, 0x00, 0x05, // d
                0xff, 0xff, 0xff, 0xfa, // v
                0x00, 0x00, 0x00, 0x07, // x
                0xff, 0xff, 0xff, 0xf8, // y
                0x00, 0x00, 0x00, 0x09, // w
            ]
        );
    }
}
