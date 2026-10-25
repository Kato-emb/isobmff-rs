//! [`DataEntry`], one entry of a data reference, and the boxes it is one of

use isobmff_core::{
    AnyBox, BoxDecode, BoxDefinition, BoxEncode, BoxType, Error, FieldReader, FieldWriter,
    Utf8String,
};

use crate::full_box::{read_version_zero, write_version_zero};

/// Flag of `entry_flags` that places the media data in the same file as the data reference
const SELF_CONTAINED: u32 = 0x01;

/// [`DataEntryUrlBox`] (`url `), ISO/IEC 14496-12 §8.7.2
///
/// Location of the media data of a data reference, as a URL, or the
/// statement that the media data is in the same file as the data reference.
///
/// `location` is [`None`] where the flag of `entry_flags` that places the
/// media data in the same file is set, and no string is present: the box is
/// read so wherever the flag is set, and written with the flag set where
/// `location` is [`None`]. The other flags are read past and written as zero.
///
/// [`Default`] is the box of media data in the same file.
#[doc(alias = "url")]
#[non_exhaustive]
#[derive(Clone, Default, PartialEq, Eq, Hash, Debug)]
pub struct DataEntryUrlBox {
    /// URL of the media data, or [`None`] for media data in the same file as the data reference
    pub location: Option<Utf8String>,
}

impl BoxDefinition for DataEntryUrlBox {
    const BOX_TYPE: BoxType = BoxType::compact(*b"url ");
}

impl BoxDecode for DataEntryUrlBox {
    fn decode_fields(reader: &mut FieldReader<'_>) -> Result<Self, Error> {
        let entry_flags = read_version_zero(reader)?;

        Ok(Self {
            location: (entry_flags.bits() & SELF_CONTAINED == 0)
                .then(|| Utf8String::from_slice(reader.take_remainder())),
        })
    }
}

impl BoxEncode for DataEntryUrlBox {
    fn payload_len(&self) -> u64 {
        self.location
            .as_ref()
            .map_or(0, Utf8String::encoded_len)
            .saturating_add(4)
    }

    fn encode_fields(&self, writer: &mut FieldWriter<'_>) -> Result<(), Error> {
        let Some(location) = &self.location else {
            return writer.write_bytes(&[0, 0, 0, 1]);
        };
        write_version_zero(writer)?;
        location.encode(writer.take_remainder())?;

        Ok(())
    }
}

/// [`DataEntryUrnBox`] (`urn `), ISO/IEC 14496-12 §8.7.2
///
/// Media data of a data reference, named by a URN and, optionally, located by
/// a URL.
///
/// The `entry_flags` are read past and written as zero: the flag that places
/// the media data in the same file is for a [`DataEntryUrlBox`] alone.
#[doc(alias = "urn")]
#[non_exhaustive]
#[derive(Clone, Default, PartialEq, Eq, Hash, Debug)]
pub struct DataEntryUrnBox {
    /// URN of the media data
    pub name: Utf8String,
    /// URL to find the media data of the given name at
    pub location: Option<Utf8String>,
}

impl BoxDefinition for DataEntryUrnBox {
    const BOX_TYPE: BoxType = BoxType::compact(*b"urn ");
}

impl BoxDecode for DataEntryUrnBox {
    fn decode_fields(reader: &mut FieldReader<'_>) -> Result<Self, Error> {
        read_version_zero(reader)?;
        let mut strings = reader.take_remainder().splitn(2, |byte| *byte == 0);

        Ok(Self {
            name: Utf8String::from_slice(strings.next().unwrap_or_default()),
            location: strings
                .next()
                .filter(|location| !location.is_empty())
                .map(Utf8String::from_slice),
        })
    }
}

impl BoxEncode for DataEntryUrnBox {
    fn payload_len(&self) -> u64 {
        self.location
            .as_ref()
            .map_or(0, Utf8String::encoded_len)
            .saturating_add(self.name.encoded_len())
            .saturating_add(4)
    }

    fn encode_fields(&self, writer: &mut FieldWriter<'_>) -> Result<(), Error> {
        write_version_zero(writer)?;
        let rest = self.name.encode(writer.take_remainder())?;
        if let Some(location) = &self.location {
            location.encode(rest)?;
        }

        Ok(())
    }
}

/// [`DataEntryImdaBox`] (`imdt`), ISO/IEC 14496-12 §8.7.2
///
/// Media data of a data reference held in the `IdentifiedMediaDataBox` of the
/// given identifier, media data offsets counting from the first byte of its
/// payload.
///
/// The `entry_flags` are read past and written as zero.
#[doc(alias = "imdt")]
#[non_exhaustive]
#[derive(Clone, Default, PartialEq, Eq, Hash, Debug)]
pub struct DataEntryImdaBox {
    /// `imda_identifier` of the `IdentifiedMediaDataBox` that holds the media data
    pub imda_ref_identifier: u32,
}

impl BoxDefinition for DataEntryImdaBox {
    const BOX_TYPE: BoxType = BoxType::compact(*b"imdt");
}

impl BoxDecode for DataEntryImdaBox {
    fn decode_fields(reader: &mut FieldReader<'_>) -> Result<Self, Error> {
        read_version_zero(reader)?;

        Ok(Self {
            imda_ref_identifier: reader.read_u32()?,
        })
    }
}

impl BoxEncode for DataEntryImdaBox {
    fn payload_len(&self) -> u64 {
        4 + 4
    }

    fn encode_fields(&self, writer: &mut FieldWriter<'_>) -> Result<(), Error> {
        write_version_zero(writer)?;
        writer.write_u32(self.imda_ref_identifier)
    }
}

/// [`DataEntrySeqNumImdaBox`] (`snim`), ISO/IEC 14496-12 §8.7.2
///
/// Media data of a data reference held, for each sample, in the
/// `IdentifiedMediaDataBox` whose `imda_identifier` is the `sequence_number`
/// of the movie fragment that holds the sample. The box carries no field.
///
/// The `entry_flags` are read past and written as zero.
#[doc(alias = "snim")]
#[non_exhaustive]
#[derive(Clone, Default, PartialEq, Eq, Hash, Debug)]
pub struct DataEntrySeqNumImdaBox {}

impl BoxDefinition for DataEntrySeqNumImdaBox {
    const BOX_TYPE: BoxType = BoxType::compact(*b"snim");
}

impl BoxDecode for DataEntrySeqNumImdaBox {
    fn decode_fields(reader: &mut FieldReader<'_>) -> Result<Self, Error> {
        read_version_zero(reader)?;

        Ok(Self {})
    }
}

impl BoxEncode for DataEntrySeqNumImdaBox {
    fn payload_len(&self) -> u64 {
        4
    }

    fn encode_fields(&self, writer: &mut FieldWriter<'_>) -> Result<(), Error> {
        write_version_zero(writer)
    }
}

/// One entry of a data reference, an instance of a class derived from `DataEntryBaseBox`, ISO/IEC 14496-12 §8.7.2
#[doc(alias = "DataEntryBaseBox")]
#[non_exhaustive]
#[derive(Clone, PartialEq, Debug)]
pub enum DataEntry {
    /// Media data located by a URL, or in the same file
    Url(DataEntryUrlBox),
    /// Media data named by a URN
    Urn(DataEntryUrnBox),
    /// Media data in the `IdentifiedMediaDataBox` of an identifier
    Imda(DataEntryImdaBox),
    /// Media data in the `IdentifiedMediaDataBox` of the sequence number of each movie fragment
    SeqNumImda(DataEntrySeqNumImdaBox),
    /// Entry of a type this crate does not read, or of one it reads that does not decode, kept as its bytes
    Other(AnyBox),
}

#[cfg(test)]
mod tests {
    use alloc::string::String;

    use isobmff_core::{BoxDecode, Error, Utf8String};

    use super::{DataEntryImdaBox, DataEntrySeqNumImdaBox, DataEntryUrlBox, DataEntryUrnBox};
    use crate::encoded;

    /// `url ` of media data in the same file
    const URL_SELF_CONTAINED: &[u8] = b"\0\0\0\x0curl \0\0\0\x01";

    /// `url ` of media data at `a.mp4`
    const URL: &[u8] = b"\0\0\0\x12url \0\0\0\0a.mp4\0";

    /// `urn ` of the name `urn:x` and the location `a.mp4`
    const URN: &[u8] = b"\0\0\0\x18urn \0\0\0\0urn:x\0a.mp4\0";

    /// `urn ` of the name `urn:x` and no location
    const URN_WITHOUT_LOCATION: &[u8] = b"\0\0\0\x12urn \0\0\0\0urn:x\0";

    /// `imdt` of the identifier 7
    const IMDT: &[u8] = b"\0\0\0\x10imdt\0\0\0\0\0\0\0\x07";

    /// `snim`, which carries no field
    const SNIM: &[u8] = b"\0\0\0\x0csnim\0\0\0\0";

    fn string(text: &str) -> Utf8String {
        Utf8String::new(String::from(text)).unwrap()
    }

    fn url(location: Option<&str>) -> DataEntryUrlBox {
        DataEntryUrlBox {
            location: location.map(string),
        }
    }

    fn urn(location: Option<&str>) -> DataEntryUrnBox {
        DataEntryUrnBox {
            name: string("urn:x"),
            location: location.map(string),
        }
    }

    #[test]
    fn each_entry_reads_from_the_bytes_its_syntax_lays_out() {
        assert_eq!(
            (
                DataEntryUrlBox::decode(URL_SELF_CONTAINED),
                DataEntryUrlBox::decode(URL),
                DataEntryUrnBox::decode(URN),
                DataEntryUrnBox::decode(URN_WITHOUT_LOCATION),
                DataEntryImdaBox::decode(IMDT),
                DataEntrySeqNumImdaBox::decode(SNIM),
            ),
            (
                Ok((url(None), b"".as_slice())),
                Ok((url(Some("a.mp4")), b"".as_slice())),
                Ok((urn(Some("a.mp4")), b"".as_slice())),
                Ok((urn(None), b"".as_slice())),
                Ok((
                    DataEntryImdaBox {
                        imda_ref_identifier: 7
                    },
                    b"".as_slice()
                )),
                Ok((DataEntrySeqNumImdaBox {}, b"".as_slice())),
            )
        );
    }

    #[test]
    fn each_entry_writes_the_bytes_its_syntax_lays_out() {
        assert_eq!(
            [
                encoded(&url(None)),
                encoded(&url(Some("a.mp4"))),
                encoded(&urn(Some("a.mp4"))),
                encoded(&urn(None)),
                encoded(&DataEntryImdaBox {
                    imda_ref_identifier: 7
                }),
                encoded(&DataEntrySeqNumImdaBox {}),
            ],
            [
                URL_SELF_CONTAINED,
                URL,
                URN,
                URN_WITHOUT_LOCATION,
                IMDT,
                SNIM
            ]
            .map(<[u8]>::to_vec)
        );
    }

    #[test]
    fn a_url_with_the_self_contained_flag_set_reads_no_location_whatever_follows() {
        let (read, _) = DataEntryUrlBox::decode(b"\0\0\0\x0durl \0\0\0\x01\0").unwrap();

        assert_eq!(
            (read.clone(), encoded(&read)),
            (url(None), URL_SELF_CONTAINED.to_vec())
        );
    }

    #[test]
    fn flags_other_than_self_contained_are_read_past_and_written_as_zero() {
        let (location, _) = DataEntryUrlBox::decode(b"\0\0\0\x12url \0\0\x01\0a.mp4\0").unwrap();
        let (identified, _) =
            DataEntryImdaBox::decode(b"\0\0\0\x10imdt\0\0\0\x01\0\0\0\x07").unwrap();

        assert_eq!(
            (encoded(&location), encoded(&identified)),
            (URL.to_vec(), IMDT.to_vec())
        );
    }

    #[test]
    fn a_payload_ending_inside_a_field_is_refused() {
        assert_eq!(
            (
                DataEntryUrlBox::decode_payload(b"\0\0"),
                DataEntryImdaBox::decode_payload(b"\0\0\0\0\0\0"),
            ),
            (
                Err(Error::truncated_payload(4, 2)),
                Err(Error::truncated_payload(8, 6)),
            )
        );
    }

    #[test]
    fn a_version_the_entries_do_not_read_is_refused() {
        let version_1 = b"\x01\0\0\0\0\0\0\0";

        assert_eq!(
            (
                DataEntryUrlBox::decode_payload(version_1),
                DataEntryUrnBox::decode_payload(version_1),
                DataEntryImdaBox::decode_payload(version_1),
                DataEntrySeqNumImdaBox::decode_payload(version_1),
            ),
            (
                Err(Error::unsupported_version(1)),
                Err(Error::unsupported_version(1)),
                Err(Error::unsupported_version(1)),
                Err(Error::unsupported_version(1)),
            )
        );
    }
}
