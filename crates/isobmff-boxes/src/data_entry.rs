//! [`DataEntryUrlBox`] (`url_`) and [`DataEntryUrnBox`] (`urn_`), the data
//! reference entries of ISO/IEC 14496-12 §8.7.2
//!
//! The fourcc of each ends in a space — `url ` and `urn ` — which the text here
//! writes with an underscore, where the space would not show.

use isobmff_core::{
    BoxDecode, BoxDefinition, BoxEncode, BoxType, Error, FieldReader, FieldWriter, FullBoxFields,
    FullBoxFlags, NullTerminatedString, RawBox,
};

/// Length of the version and the flags every entry opens with
const FULL_BOX_FIELDS_LEN: u64 = 4;

/// Flag §8.7.2.3 gives the media data lying in the file this entry is read from
const SELF_CONTAINED: FullBoxFlags = match FullBoxFlags::new(1) {
    Some(flags) => flags,
    // Why not unwrap: 1 is within the 24 bits the field carries, so the flags
    // always build, and a degenerate value stands in for the panic the lints
    // forbid.
    None => FullBoxFlags::ZERO,
};

/// Entry of a data reference, stating where the media data of a track lies
///
/// ISO/IEC 14496-12 §8.7.2. The spec closes the set: every entry of a `dref` is
/// either a [`DataEntryUrlBox`] or a [`DataEntryUrnBox`], and a `dref` holding a
/// box of any other type does not read.
#[non_exhaustive]
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub enum DataEntry {
    /// Entry naming the location of the media data, `url_`
    Url(DataEntryUrlBox),
    /// Entry naming the resource the media data is, `urn_`
    Urn(DataEntryUrnBox),
}

impl DataEntry {
    /// Reads the entry `child` holds, for a child of one of the two types
    pub(crate) fn decode(child: RawBox<'_>) -> Result<Self, Error> {
        let box_type = child.header().box_type();

        if box_type == DataEntryUrlBox::BOX_TYPE {
            DataEntryUrlBox::decode_payload(child.payload()).map(Self::Url)
        } else if box_type == DataEntryUrnBox::BOX_TYPE {
            DataEntryUrnBox::decode_payload(child.payload()).map(Self::Urn)
        } else {
            return Err(Error::forbidden_child_box(box_type));
        }
        .map_err(|error| error.in_container(box_type))
    }

    /// Returns the length this entry occupies, header and payload
    pub(crate) fn encoded_len(&self) -> u64 {
        match self {
            Self::Url(url) => url.encoded_len(),
            Self::Urn(urn) => urn.encoded_len(),
        }
    }

    /// Writes the entry into the front of `buffer` and returns what is left
    pub(crate) fn encode<'buffer>(
        &self,
        buffer: &'buffer mut [u8],
    ) -> Result<&'buffer mut [u8], Error> {
        match self {
            Self::Url(url) => url.encode(buffer),
            Self::Urn(urn) => urn.encode(buffer),
        }
    }
}

/// Entry that names where the media data of a track lies, as a URL
///
/// [`DataEntryUrlBox`] (`url_`), ISO/IEC 14496-12 §8.7.2. A `location` of `None`
/// is the self-contained entry: §8.7.2.3 has the media data lie in the same file
/// as the `moov` that holds this reference, and then "no string (not even an
/// empty one) shall be supplied". So the flag the spec defines is not held — it
/// is the `location` being there or not — and an entry that sets it while
/// carrying a string does not read.
///
/// Neither the version nor the other 23 bits of `flags` are held: the spec
/// declares the version zero and defines no other flag.
// Why not the fourcc itself: rustdoc refuses an alias that ends in a space, so
// the underscore stands in for the one `url ` would be.
#[doc(alias = "url_")]
#[non_exhaustive]
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub struct DataEntryUrlBox {
    location: Option<NullTerminatedString>,
}

impl DataEntryUrlBox {
    /// Creates the entry from the location of the media data, if it is elsewhere
    #[must_use]
    pub const fn new(location: Option<NullTerminatedString>) -> Self {
        Self { location }
    }

    /// Returns where the media data lies, or `None` where it lies in this file
    #[must_use]
    pub const fn location(&self) -> Option<&NullTerminatedString> {
        self.location.as_ref()
    }
}

impl BoxDefinition for DataEntryUrlBox {
    const BOX_TYPE: BoxType = BoxType::compact(*b"url ");
}

impl BoxDecode for DataEntryUrlBox {
    /// # Errors
    ///
    /// * [`UnsupportedVersion`](isobmff_core::ErrorKind::UnsupportedVersion): the box
    ///   declares a version other than 0.
    /// * [`TruncatedPayload`](isobmff_core::ErrorKind::TruncatedPayload): the payload
    ///   ends inside the version and the flags. A location carried while the box
    ///   states the media data is in this file is the
    ///   [`TrailingPayload`](isobmff_core::ErrorKind::TrailingPayload) the payload
    ///   contract refuses.
    /// * [`InvalidUtf8`](isobmff_core::ErrorKind::InvalidUtf8): the location is not
    ///   UTF-8.
    fn decode_fields(reader: &mut FieldReader<'_>) -> Result<Self, Error> {
        let full_box = FullBoxFields::from_bytes(reader.read_bytes::<4>()?);
        if full_box.version() != 0 {
            return Err(Error::unsupported_version(full_box.version()));
        }

        let location = if full_box.flags().bits() & SELF_CONTAINED.bits() == 0 {
            Some(NullTerminatedString::from_slice(reader.take_remainder())?)
        } else {
            None
        };

        Ok(Self { location })
    }
}

impl BoxEncode for DataEntryUrlBox {
    fn payload_len(&self) -> u64 {
        FULL_BOX_FIELDS_LEN.saturating_add(
            self.location
                .as_ref()
                .map_or(0, NullTerminatedString::encoded_len),
        )
    }

    fn encode_fields(&self, writer: &mut FieldWriter<'_>) -> Result<(), Error> {
        let flags = match self.location {
            Some(_) => FullBoxFlags::ZERO,
            None => SELF_CONTAINED,
        };

        writer.write_bytes(&FullBoxFields::new(0, flags).to_bytes())?;
        if let Some(location) = &self.location {
            location.encode(writer.take_remainder())?;
        }

        Ok(())
    }
}

/// Entry that names the resource the media data of a track is, as a URN
///
/// [`DataEntryUrnBox`] (`urn_`), ISO/IEC 14496-12 §8.7.2. The `name` is the URN
/// naming the resource, which every such entry states; the `location` is where
/// to find the resource so named, which §8.7.2.3 leaves optional.
///
/// Neither the version nor the `flags` are held: the spec declares the version
/// zero, and the one flag it defines states the media data is in this file,
/// which is the URL form of an entry rather than this one.
// Why not the fourcc itself: rustdoc refuses an alias that ends in a space, so
// the underscore stands in for the one `urn ` would be.
#[doc(alias = "urn_")]
#[non_exhaustive]
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub struct DataEntryUrnBox {
    name: NullTerminatedString,
    location: Option<NullTerminatedString>,
}

impl DataEntryUrnBox {
    /// Creates the entry from the name of the resource and where to find it
    #[must_use]
    pub const fn new(name: NullTerminatedString, location: Option<NullTerminatedString>) -> Self {
        Self { name, location }
    }

    /// Returns the URN naming the resource the media data is
    #[must_use]
    pub const fn name(&self) -> &NullTerminatedString {
        &self.name
    }

    /// Returns where to find the resource the name states, if the entry says
    #[must_use]
    pub const fn location(&self) -> Option<&NullTerminatedString> {
        self.location.as_ref()
    }
}

impl BoxDefinition for DataEntryUrnBox {
    const BOX_TYPE: BoxType = BoxType::compact(*b"urn ");
}

impl BoxDecode for DataEntryUrnBox {
    /// # Errors
    ///
    /// * [`UnsupportedVersion`](isobmff_core::ErrorKind::UnsupportedVersion): the box
    ///   declares a version other than 0.
    /// * [`TruncatedPayload`](isobmff_core::ErrorKind::TruncatedPayload): the payload
    ///   ends inside the version and the flags.
    /// * [`InvalidUtf8`](isobmff_core::ErrorKind::InvalidUtf8): the name or the
    ///   location is not UTF-8.
    fn decode_fields(reader: &mut FieldReader<'_>) -> Result<Self, Error> {
        let version = FullBoxFields::from_bytes(reader.read_bytes::<4>()?).version();
        if version != 0 {
            return Err(Error::unsupported_version(version));
        }

        let strings = reader.take_remainder();
        // Why not unwrap: the index `position` reports is within `strings`, so
        // both ranges always slice, and a degenerate value stands in for the
        // panic the lints forbid.
        let (name, rest) = match strings.iter().position(|byte| *byte == 0) {
            Some(terminator) => (
                strings.get(..terminator).unwrap_or(&[]),
                strings.get(terminator.saturating_add(1)..).unwrap_or(&[]),
            ),
            None => (strings, [].as_slice()),
        };

        Ok(Self {
            name: NullTerminatedString::from_slice(name)?,
            location: match rest {
                [] => None,
                location => Some(NullTerminatedString::from_slice(location)?),
            },
        })
    }
}

impl BoxEncode for DataEntryUrnBox {
    fn payload_len(&self) -> u64 {
        FULL_BOX_FIELDS_LEN
            .saturating_add(self.name.encoded_len())
            .saturating_add(
                self.location
                    .as_ref()
                    .map_or(0, NullTerminatedString::encoded_len),
            )
    }

    fn encode_fields(&self, writer: &mut FieldWriter<'_>) -> Result<(), Error> {
        writer.write_bytes(&FullBoxFields::new(0, FullBoxFlags::ZERO).to_bytes())?;

        let rest = self.name.encode(writer.take_remainder())?;
        if let Some(location) = &self.location {
            location.encode(rest)?;
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use alloc::string::String;
    use alloc::vec;
    use alloc::vec::Vec;

    use isobmff_core::{BoxDecode, BoxEncode, Error, NullTerminatedString};

    use super::{DataEntryUrlBox, DataEntryUrnBox};

    /// Text of a field, which the spec carries as a null-terminated string
    fn text(value: &str) -> NullTerminatedString {
        NullTerminatedString::new(String::from(value)).unwrap()
    }

    /// Writes the payload of an entry and returns the bytes it occupies
    fn encoded_payload(entry: &(impl BoxEncode + BoxDecode)) -> Vec<u8> {
        let mut buffer = vec![0; usize::try_from(entry.payload_len()).unwrap()];
        entry.encode_payload(&mut buffer).unwrap();

        buffer
    }

    #[test]
    fn an_entry_whose_data_lies_in_this_file_carries_no_location() {
        let self_contained = DataEntryUrlBox::new(None);

        let payload = encoded_payload(&self_contained);

        assert_eq!(payload, b"\0\0\0\x01");
        assert_eq!(
            DataEntryUrlBox::decode_payload(&payload).unwrap(),
            self_contained
        );
    }

    #[test]
    fn an_entry_whose_data_lies_elsewhere_reads_back_as_the_value_that_wrote_it() {
        let elsewhere = DataEntryUrlBox::new(Some(text("media.mp4")));

        let payload = encoded_payload(&elsewhere);

        assert_eq!(payload, b"\0\0\0\0media.mp4\0");
        assert_eq!(
            DataEntryUrlBox::decode_payload(&payload).unwrap(),
            elsewhere
        );
    }

    #[test]
    fn an_entry_stating_this_file_while_carrying_a_location_is_rejected() {
        let payload = b"\0\0\0\x01media.mp4\0";

        assert_eq!(
            DataEntryUrlBox::decode_payload(payload),
            Err(Error::trailing_payload(4, 14))
        );
    }

    #[test]
    fn a_urn_entry_reads_back_as_the_value_that_wrote_it_with_or_without_a_location() {
        for location in [None, Some(text("media.mp4"))] {
            let named = DataEntryUrnBox::new(text("urn:smpte:ul:0"), location);

            let payload = encoded_payload(&named);

            assert_eq!(DataEntryUrnBox::decode_payload(&payload).unwrap(), named);
        }
    }

    #[test]
    fn a_urn_entry_ends_its_name_at_the_terminator_and_takes_what_follows_as_the_location() {
        let payload = b"\0\0\0\0urn:smpte:ul:0\0media.mp4\0";

        assert_eq!(
            DataEntryUrnBox::decode_payload(payload).unwrap(),
            DataEntryUrnBox::new(text("urn:smpte:ul:0"), Some(text("media.mp4")))
        );
    }

    #[test]
    fn a_payload_shorter_than_the_version_and_the_flags_is_rejected() {
        assert_eq!(
            DataEntryUrlBox::decode_payload(&[0; 3]),
            Err(Error::truncated_payload(4, 3))
        );
        assert_eq!(
            DataEntryUrnBox::decode_payload(&[0; 3]),
            Err(Error::truncated_payload(4, 3))
        );
    }

    #[test]
    fn a_version_an_entry_does_not_read_is_rejected() {
        assert_eq!(
            DataEntryUrlBox::decode_payload(b"\x01\0\0\0"),
            Err(Error::unsupported_version(1))
        );
        assert_eq!(
            DataEntryUrnBox::decode_payload(b"\x01\0\0\0"),
            Err(Error::unsupported_version(1))
        );
    }
}
