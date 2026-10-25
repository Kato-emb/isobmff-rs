//! [`FreeSpaceBox`], under either of the box types `free` and `skip`

use alloc::vec::Vec;
use core::marker::PhantomData;

use isobmff_core::{BoxDecode, BoxDefinition, BoxEncode, BoxType, Error, FieldReader, FieldWriter};

mod sealed {
    /// Closes [`FreeType`](super::FreeType) to the marker types of this module
    #[expect(
        unnameable_types,
        reason = "a supertrait no other crate can name is what seals FreeType"
    )]
    pub trait Sealed {}
}

/// `free_type` a [`FreeSpaceBox`] is written under, `free` or `skip` and nothing else
pub trait FreeType: sealed::Sealed {
    /// Box type the [`FreeSpaceBox`] takes
    const FREE_TYPE: BoxType;
}

/// [`FreeType`] of a [`FreeSpaceBox`] under the box type `free`
#[non_exhaustive]
#[derive(Clone, Copy, Default, PartialEq, Eq, Hash, Debug)]
pub struct Free;

impl sealed::Sealed for Free {}

impl FreeType for Free {
    const FREE_TYPE: BoxType = BoxType::compact(*b"free");
}

/// [`FreeType`] of a [`FreeSpaceBox`] under the box type `skip`
#[non_exhaustive]
#[derive(Clone, Copy, Default, PartialEq, Eq, Hash, Debug)]
pub struct Skip;

impl sealed::Sealed for Skip {}

impl FreeType for Skip {
    const FREE_TYPE: BoxType = BoxType::compact(*b"skip");
}

/// [`FreeSpaceBox`] (`free`, `skip`), ISO/IEC 14496-12 §8.1.2
///
/// Space whose contents are irrelevant: a reader may ignore it, or delete
/// the box, without affecting the presentation. `Type` is the box type it is
/// read and written under, [`Free`] or [`Skip`].
///
/// # Examples
///
/// ```
/// use isobmff_boxes::{Free, FreeSpaceBox, Skip};
/// use isobmff_core::{BoxDecode, BoxEncode};
///
/// // Each box type is a type of its own
/// let mut skip = FreeSpaceBox::<Skip>::default();
/// skip.data = vec![0; 4];
/// let mut buffer = [0; 12];
/// skip.encode(&mut buffer).unwrap();
/// assert_eq!(&buffer, b"\0\0\0\x0cskip\0\0\0\0");
///
/// // and reads only the box type it names
/// assert!(FreeSpaceBox::<Free>::decode(&buffer).is_err());
/// ```
#[doc(alias = "free", alias = "skip")]
#[non_exhaustive]
#[derive(Clone, Default, PartialEq, Eq, Hash, Debug)]
pub struct FreeSpaceBox<Type: FreeType> {
    /// Contents of the space, which carry no meaning
    pub data: Vec<u8>,
    free_type: PhantomData<Type>,
}

impl<Type: FreeType> BoxDefinition for FreeSpaceBox<Type> {
    const BOX_TYPE: BoxType = Type::FREE_TYPE;
}

impl<Type: FreeType> BoxDecode for FreeSpaceBox<Type> {
    fn decode_fields(reader: &mut FieldReader<'_>) -> Result<Self, Error> {
        Ok(Self {
            data: reader.take_remainder().to_vec(),
            free_type: PhantomData,
        })
    }
}

impl<Type: FreeType> BoxEncode for FreeSpaceBox<Type> {
    fn payload_len(&self) -> u64 {
        u64::try_from(self.data.len()).unwrap_or(u64::MAX)
    }

    fn encode_fields(&self, writer: &mut FieldWriter<'_>) -> Result<(), Error> {
        writer.write_slice(&self.data)
    }
}

#[cfg(test)]
mod tests {
    use alloc::vec;

    use core::marker::PhantomData;

    use isobmff_core::{BoxDecode, BoxDefinition, Error};

    use super::{Free, FreeSpaceBox, FreeType, Skip};
    use crate::encoded;

    /// `free` of four bytes of space
    const FREE: &[u8] = b"\0\0\0\x0cfree\x01\x02\x03\x04";

    /// `skip` of four bytes of space
    const SKIP: &[u8] = b"\0\0\0\x0cskip\x01\x02\x03\x04";

    fn space<Type: FreeType>() -> FreeSpaceBox<Type> {
        FreeSpaceBox {
            data: vec![1, 2, 3, 4],
            free_type: PhantomData,
        }
    }

    #[test]
    fn reads_from_the_bytes_its_syntax_lays_out_under_either_box_type() {
        assert_eq!(
            (
                FreeSpaceBox::<Free>::decode(FREE),
                FreeSpaceBox::<Skip>::decode(SKIP)
            ),
            (Ok((space(), b"".as_slice())), Ok((space(), b"".as_slice())))
        );
    }

    #[test]
    fn writes_the_bytes_its_syntax_lays_out_under_either_box_type() {
        assert_eq!(
            (encoded(&space::<Free>()), encoded(&space::<Skip>())),
            (FREE.to_vec(), SKIP.to_vec())
        );
    }

    #[test]
    fn a_box_of_the_other_free_type_is_refused() {
        assert_eq!(
            FreeSpaceBox::<Free>::decode(SKIP),
            Err(Error::box_type_mismatch(
                Free::FREE_TYPE,
                FreeSpaceBox::<Skip>::BOX_TYPE
            ))
        );
    }
}
