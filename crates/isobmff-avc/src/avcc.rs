//! [`AVCConfigurationBox`] (`avcC`), ISO/IEC 14496-15 §5.4.2

use isobmff_core::{BoxDecode, BoxDefinition, BoxEncode, BoxType, Error, FieldReader, FieldWriter};

use crate::decoder_configuration_record::AVCDecoderConfigurationRecord;

/// Box an AVC sample entry holds to carry the decoder configuration record
///
/// [`AVCConfigurationBox`] (`avcC`), ISO/IEC 14496-15 §5.4.2. The record is
/// the whole of the payload.
#[doc(alias = "avcC")]
#[non_exhaustive]
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub struct AVCConfigurationBox {
    avc_config: AVCDecoderConfigurationRecord,
}

impl AVCConfigurationBox {
    /// Creates the box around the record it carries
    #[must_use]
    pub const fn new(avc_config: AVCDecoderConfigurationRecord) -> Self {
        Self { avc_config }
    }

    /// Returns the decoder configuration record
    #[must_use]
    pub const fn avc_config(&self) -> &AVCDecoderConfigurationRecord {
        &self.avc_config
    }
}

impl BoxDefinition for AVCConfigurationBox {
    const BOX_TYPE: BoxType = BoxType::compact(*b"avcC");
}

impl BoxDecode for AVCConfigurationBox {
    /// # Errors
    ///
    /// * What [`AVCDecoderConfigurationRecord::decode_fields`] reports.
    fn decode_fields(reader: &mut FieldReader<'_>) -> Result<Self, Error> {
        Ok(Self {
            avc_config: AVCDecoderConfigurationRecord::decode_fields(reader)?,
        })
    }
}

impl BoxEncode for AVCConfigurationBox {
    fn payload_len(&self) -> u64 {
        self.avc_config.encoded_len()
    }

    fn encode_fields(&self, writer: &mut FieldWriter<'_>) -> Result<(), Error> {
        self.avc_config.encode_fields(writer)
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use alloc::vec;
    use alloc::vec::Vec;

    use isobmff_core::{BoxDecode, BoxEncode, Error};

    use super::AVCConfigurationBox;
    use crate::decoder_configuration_record::{
        AVCDecoderConfigurationRecord, HighProfileFields, LengthSizeMinusOne,
    };

    /// SPS of a Constrained Baseline stream at level 3.0, as an encoder emits it
    pub(crate) fn sequence_parameter_set() -> Vec<u8> {
        vec![0x67, 0x42, 0xc0, 0x1e, 0xd9, 0x00, 0xb4, 0x3d, 0xa1]
    }

    /// PPS to go with [`sequence_parameter_set`]
    pub(crate) fn picture_parameter_set() -> Vec<u8> {
        vec![0x68, 0xce, 0x3c, 0x80]
    }

    /// The `avcC` of a Constrained Baseline stream
    pub(crate) fn baseline_configuration() -> AVCConfigurationBox {
        AVCConfigurationBox::new(
            AVCDecoderConfigurationRecord::from_parameter_sets(
                LengthSizeMinusOne::FOUR_BYTES,
                vec![sequence_parameter_set()],
                vec![picture_parameter_set()],
                None,
            )
            .unwrap(),
        )
    }

    fn encoded_payload(configuration: &AVCConfigurationBox) -> Vec<u8> {
        let mut buffer = vec![0; usize::try_from(configuration.payload_len()).unwrap()];
        configuration.encode_payload(&mut buffer).unwrap();

        buffer
    }

    #[test]
    fn a_box_reads_back_as_the_value_that_wrote_it() {
        let configuration = baseline_configuration();

        let payload = encoded_payload(&configuration);

        assert_eq!(
            payload,
            [
                b"\x01\x42\xc0\x1e\xff\xe1\0\x09".as_slice(),
                &sequence_parameter_set(),
                b"\x01\0\x04",
                &picture_parameter_set(),
            ]
            .concat()
        );
        assert_eq!(
            AVCConfigurationBox::decode_payload(&payload).unwrap(),
            configuration
        );
    }

    #[test]
    fn a_high_profile_record_reads_back_with_its_fields() {
        let configuration = AVCConfigurationBox::new(
            AVCDecoderConfigurationRecord::new(
                100,
                0,
                0x28,
                LengthSizeMinusOne::FOUR_BYTES,
                vec![sequence_parameter_set()],
                vec![picture_parameter_set()],
                Some(HighProfileFields::new(1, 0, 0, vec![vec![0x6d, 0x01]]).unwrap()),
            )
            .unwrap(),
        );

        let payload = encoded_payload(&configuration);

        assert_eq!(
            payload.get(payload.len() - 8..),
            Some(b"\xfd\xf8\xf8\x01\0\x02\x6d\x01".as_slice())
        );
        assert_eq!(
            AVCConfigurationBox::decode_payload(&payload).unwrap(),
            configuration
        );
    }

    #[test]
    fn a_high_profile_record_that_leaves_its_fields_off_reads_without_them() {
        let payload = b"\x01\x64\0\x28\xff\xe0\0";

        let configuration = AVCConfigurationBox::decode_payload(payload).unwrap();

        assert_eq!(configuration.avc_config().high_profile_fields(), None);
        assert_eq!(encoded_payload(&configuration), payload);
    }

    #[test]
    fn a_configuration_version_the_box_does_not_read_is_rejected() {
        assert_eq!(
            AVCConfigurationBox::decode_payload(b"\x02\x42\xc0\x1e\xff\xe0\0"),
            Err(Error::unsupported_version(2))
        );
    }

    #[test]
    fn a_parameter_set_running_past_the_payload_is_rejected_as_truncated() {
        assert_eq!(
            AVCConfigurationBox::decode_payload(b"\x01\x42\xc0\x1e\xff\xe1\0\x09\x67"),
            Err(Error::truncated_payload(17, 9))
        );
    }

    #[test]
    fn a_length_size_the_spec_forbids_still_reads() {
        let configuration =
            AVCConfigurationBox::decode_payload(b"\x01\x42\xc0\x1e\xfe\xe0\0").unwrap();

        assert_eq!(
            configuration
                .avc_config()
                .length_size_minus_one()
                .length_size_minus_one(),
            2
        );
        assert_eq!(LengthSizeMinusOne::new(2), None);
    }

    #[test]
    fn high_profile_fields_are_refused_for_a_profile_that_has_none() {
        assert_eq!(
            AVCDecoderConfigurationRecord::new(
                66,
                0xc0,
                0x1e,
                LengthSizeMinusOne::FOUR_BYTES,
                vec![sequence_parameter_set()],
                vec![picture_parameter_set()],
                Some(HighProfileFields::new(1, 0, 0, Vec::new()).unwrap()),
            ),
            None
        );
    }

    #[test]
    fn a_record_needs_a_first_sps_long_enough_to_state_the_profile() {
        assert_eq!(
            AVCDecoderConfigurationRecord::from_parameter_sets(
                LengthSizeMinusOne::FOUR_BYTES,
                vec![vec![0x67, 0x42, 0xc0]],
                Vec::new(),
                None,
            ),
            None
        );
        assert_eq!(
            AVCDecoderConfigurationRecord::from_parameter_sets(
                LengthSizeMinusOne::FOUR_BYTES,
                Vec::new(),
                Vec::new(),
                None,
            ),
            None
        );
    }

    #[test]
    fn more_parameter_sets_than_the_record_can_count_are_refused() {
        assert_eq!(
            AVCDecoderConfigurationRecord::from_parameter_sets(
                LengthSizeMinusOne::FOUR_BYTES,
                vec![sequence_parameter_set(); 32],
                Vec::new(),
                None,
            ),
            None
        );
    }
}
