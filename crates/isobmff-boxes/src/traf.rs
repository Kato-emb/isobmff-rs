//! [`TrackFragmentBox`], the samples a movie fragment adds to one track

use alloc::vec::Vec;

use isobmff_core::{
    AnyBox, BoxDecode, BoxDefinition, BoxEncode, BoxType, ChildBoxes, Error, FieldReader,
    FieldWriter, OtherBoxes,
};

use crate::csgp::CompactSampleToGroupBox;
use crate::meta::MetaBox;
use crate::rsot::RedundantSampleOriginalTimingBox;
use crate::saio::SampleAuxiliaryInformationOffsetsBox;
use crate::saiz::SampleAuxiliaryInformationSizesBox;
use crate::sbgp::SampleToGroupBox;
use crate::sgpd::SampleGroupDescriptionBox;
use crate::subs::SubSampleInformationBox;
use crate::tfdt::TrackFragmentBaseMediaDecodeTimeBox;
use crate::tfhd::TrackFragmentHeaderBox;
use crate::trun::TrackRunBox;
use crate::udta::UserDataBox;

/// [`TrackFragmentBox`] (`traf`), ISO/IEC 14496-12 §8.8.6
///
/// The samples, and the empty time, a movie fragment adds to one track, as
/// zero or more track runs.
///
/// The box writes `tfhd`, `tfdt`, the track runs, the `sbgp`s, the `csgp`s,
/// the `sgpd`s, the `subs`es, the `saiz`es, the `saio`s, `meta`, `rsot` and
/// the `udta`s in this order, whatever order they were read in: the order of
/// ISO/IEC 14496-12 Table 2, but for `tfdt`, which §8.8.12.1 places after
/// `tfhd` and before the first track run, and `csgp` and `rsot`, which Table 2
/// does not list: `csgp` is written right after `sbgp`, and `rsot` after
/// `meta`. Children of other types are kept in
/// [`other_boxes`](Self::other_boxes), and written back after `rsot` and
/// before the `udta`s, which §7.3.4 places last. An optional child that does
/// not read is kept there too; a `tfdt`, a `meta` or an `rsot` that does not
/// read reads back [`None`].
#[doc(alias = "traf")]
#[non_exhaustive]
#[derive(Clone, Default, PartialEq, Debug)]
pub struct TrackFragmentBox {
    /// Information and defaults the track runs of the fragment use
    pub tfhd: TrackFragmentHeaderBox,
    /// Decoding timestamp of the first sample of the fragment
    pub tfdt: Option<TrackFragmentBaseMediaDecodeTimeBox>,
    /// Track runs of the fragment, in the order they came
    pub trun: Vec<TrackRunBox>,
    /// Sample group of each sample of the fragment, one box for each sample grouping
    pub sbgp: Vec<SampleToGroupBox>,
    /// Sample group of each sample of the fragment in repeating patterns, one box for each sample grouping
    pub csgp: Vec<CompactSampleToGroupBox>,
    /// Sample group description entries of the fragment, one box for each sample grouping
    pub sgpd: Vec<SampleGroupDescriptionBox>,
    /// Sub-samples of the samples of the fragment, one box for each value of `flags`
    pub subs: Vec<SubSampleInformationBox>,
    /// Sizes of the sample auxiliary information, one box for each type and stream
    pub saiz: Vec<SampleAuxiliaryInformationSizesBox>,
    /// Offsets of the sample auxiliary information, one box for each type and stream
    pub saio: Vec<SampleAuxiliaryInformationOffsetsBox>,
    /// Untimed metadata and items of the track fragment
    pub meta: Option<MetaBox>,
    /// Original timing of the first and last samples of the fragment
    pub rsot: Option<RedundantSampleOriginalTimingBox>,
    /// User data of the track fragment, in the order they came
    pub udta: Vec<UserDataBox>,
    /// Children of a type the box holds no field for
    pub other_boxes: OtherBoxes,
}

impl BoxDefinition for TrackFragmentBox {
    const BOX_TYPE: BoxType = BoxType::compact(*b"traf");
}

impl BoxDecode for TrackFragmentBox {
    fn decode_fields(reader: &mut FieldReader<'_>) -> Result<Self, Error> {
        let mut children = ChildBoxes::decode(reader.take_remainder())?;

        Ok(Self {
            tfhd: children.take_exactly_one()?,
            tfdt: children.take_zero_or_one_if_decoded()?,
            trun: children.take_zero_or_more_if_decoded(),
            sbgp: children.take_zero_or_more_if_decoded(),
            csgp: children.take_zero_or_more_if_decoded(),
            sgpd: children.take_zero_or_more_if_decoded(),
            subs: children.take_zero_or_more_if_decoded(),
            saiz: children.take_zero_or_more_if_decoded(),
            saio: children.take_zero_or_more_if_decoded(),
            meta: children.take_zero_or_one_if_decoded()?,
            rsot: children.take_zero_or_one_if_decoded()?,
            udta: children.take_zero_or_more_if_decoded(),
            other_boxes: OtherBoxes::from(children),
        })
    }
}

impl BoxEncode for TrackFragmentBox {
    fn payload_len(&self) -> u64 {
        [
            self.tfhd.encoded_len(),
            self.tfdt.as_ref().map_or(0, BoxEncode::encoded_len),
            self.meta.as_ref().map_or(0, BoxEncode::encoded_len),
            self.rsot.as_ref().map_or(0, BoxEncode::encoded_len),
        ]
        .into_iter()
        .chain(self.trun.iter().map(BoxEncode::encoded_len))
        .chain(self.sbgp.iter().map(BoxEncode::encoded_len))
        .chain(self.csgp.iter().map(BoxEncode::encoded_len))
        .chain(self.sgpd.iter().map(BoxEncode::encoded_len))
        .chain(self.subs.iter().map(BoxEncode::encoded_len))
        .chain(self.saiz.iter().map(BoxEncode::encoded_len))
        .chain(self.saio.iter().map(BoxEncode::encoded_len))
        .chain(self.udta.iter().map(BoxEncode::encoded_len))
        .chain(self.other_boxes.as_slice().iter().map(AnyBox::encoded_len))
        .fold(0, u64::saturating_add)
    }

    fn encode_fields(&self, writer: &mut FieldWriter<'_>) -> Result<(), Error> {
        let mut rest = self.tfhd.encode(writer.take_remainder())?;
        if let Some(tfdt) = &self.tfdt {
            rest = tfdt.encode(rest)?;
        }
        for track_run in &self.trun {
            rest = track_run.encode(rest)?;
        }
        for sample_to_group in &self.sbgp {
            rest = sample_to_group.encode(rest)?;
        }
        for compact_sample_to_group in &self.csgp {
            rest = compact_sample_to_group.encode(rest)?;
        }
        for description in &self.sgpd {
            rest = description.encode(rest)?;
        }
        for sub_samples in &self.subs {
            rest = sub_samples.encode(rest)?;
        }
        for sizes in &self.saiz {
            rest = sizes.encode(rest)?;
        }
        for offsets in &self.saio {
            rest = offsets.encode(rest)?;
        }
        if let Some(meta) = &self.meta {
            rest = meta.encode(rest)?;
        }
        if let Some(rsot) = &self.rsot {
            rest = rsot.encode(rest)?;
        }
        for kept in self.other_boxes.as_slice() {
            rest = kept.encode(rest)?;
        }
        for user_data in &self.udta {
            rest = user_data.encode(rest)?;
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use alloc::vec;

    use isobmff_core::{BoxDecode, BoxDefinition, ChildBoxes, Error, OtherBoxes};

    use super::TrackFragmentBox;
    use crate::encoded;
    use crate::meta::MetaBox;
    use crate::rsot::RedundantSampleOriginalTimingBox;
    use crate::saio::SampleAuxiliaryInformationOffsetsBox;
    use crate::saiz::SampleAuxiliaryInformationSizesBox;
    use crate::sbgp::SampleToGroupBox;
    use crate::sgpd::SampleGroupDescriptionBox;
    use crate::subs::SubSampleInformationBox;
    use crate::tfdt::TrackFragmentBaseMediaDecodeTimeBox;
    use crate::tfhd::TrackFragmentHeaderBox;
    use crate::trun::TrackRunBox;
    use crate::udta::UserDataBox;

    /// `tfhd` of track 1 based at the `moof`, with a default size of 300 alone
    const TFHD: &[u8] = b"\0\0\0\x14tfhd\0\x02\0\x10\0\0\0\x01\0\0\x01\x2c";

    /// `tfdt` of version 0 and a time of 90000
    const TFDT: &[u8] = b"\0\0\0\x10tfdt\0\0\0\0\0\x01\x5f\x90";

    /// `sbgp` of version 0 and `grouping_type` `roll`: 2 samples of entry 1
    const SBGP: &[u8] = b"\0\0\0\x1csbgp\0\0\0\0roll\0\0\0\x01\0\0\0\x02\0\0\0\x01";

    /// `sgpd` of version 1 and `grouping_type` `roll`, of one entry of 2 bytes, `roll_distance` -1
    const SGPD: &[u8] = b"\0\0\0\x1asgpd\x01\0\0\0roll\0\0\0\x02\0\0\0\x01\xff\xff";

    /// `subs` of version 0 and no entry
    const SUBS: &[u8] = b"\0\0\0\x10subs\0\0\0\0\0\0\0\0";

    /// `saiz` of no type, a default size of 8 for 2 samples
    const SAIZ: &[u8] = b"\0\0\0\x11saiz\0\0\0\0\x08\0\0\0\x02";

    /// `saio` of version 0 and no type, of one offset `0x200`
    const SAIO: &[u8] = b"\0\0\0\x14saio\0\0\0\0\0\0\0\x01\0\0\x02\0";

    /// `trun` of no sample
    const TRUN: &[u8] = b"\0\0\0\x10trun\0\0\0\0\0\0\0\0";

    /// `rsot` of the elapsed duration alone, 1000
    const RSOT: &[u8] = b"\0\0\0\x10rsot\0\0\0\x02\0\0\x03\xe8";

    /// `meta` holding nothing
    const META: &[u8] = b"\0\0\0\x0cmeta\0\0\0\0";

    /// `senc` holding nothing, a box `traf` holds no field for
    const SENC: &[u8] = b"\0\0\0\x10senc\0\0\0\0\0\0\0\0";

    /// `udta` holding nothing
    const UDTA: &[u8] = b"\0\0\0\x08udta";

    fn track_fragment() -> TrackFragmentBox {
        TrackFragmentBox {
            tfhd: TrackFragmentHeaderBox::decode(TFHD).unwrap().0,
            tfdt: Some(TrackFragmentBaseMediaDecodeTimeBox::decode(TFDT).unwrap().0),
            trun: vec![TrackRunBox::decode(TRUN).unwrap().0],
            sbgp: vec![SampleToGroupBox::decode(SBGP).unwrap().0],
            csgp: vec![],
            sgpd: vec![SampleGroupDescriptionBox::decode(SGPD).unwrap().0],
            subs: vec![SubSampleInformationBox::decode(SUBS).unwrap().0],
            saiz: vec![SampleAuxiliaryInformationSizesBox::decode(SAIZ).unwrap().0],
            saio: vec![
                SampleAuxiliaryInformationOffsetsBox::decode(SAIO)
                    .unwrap()
                    .0,
            ],
            meta: Some(MetaBox::default()),
            rsot: Some(RedundantSampleOriginalTimingBox::decode(RSOT).unwrap().0),
            udta: vec![UserDataBox::default()],
            other_boxes: OtherBoxes::from(ChildBoxes::decode(SENC).unwrap()),
        }
    }

    #[test]
    fn reads_its_children_in_any_order_and_keeps_the_child_it_has_no_field_for() {
        let bytes = [
            b"\0\0\0\xdbtraf".as_slice(),
            TFHD,
            RSOT,
            SAIZ,
            UDTA,
            SAIO,
            SENC,
            META,
            TRUN,
            SGPD,
            SBGP,
            SUBS,
            TFDT,
        ]
        .concat();

        assert_eq!(
            TrackFragmentBox::decode(&bytes),
            Ok((track_fragment(), b"".as_slice()))
        );
    }

    #[test]
    fn writes_tfhd_then_tfdt_then_the_order_of_table_2_then_the_child_it_has_no_field_for_then_udta()
     {
        assert_eq!(
            encoded(&track_fragment()),
            [
                b"\0\0\0\xdbtraf".as_slice(),
                TFHD,
                TFDT,
                TRUN,
                SBGP,
                SGPD,
                SUBS,
                SAIZ,
                SAIO,
                META,
                RSOT,
                SENC,
                UDTA,
            ]
            .concat()
        );
    }

    #[test]
    fn a_child_it_has_no_field_for_read_after_udta_is_written_before_it() {
        let payload = [TFHD, UDTA, SENC].concat();
        let track_fragment = TrackFragmentBox::decode_payload(&payload).unwrap();

        assert_eq!(
            encoded(&track_fragment),
            [b"\0\0\0\x34traf".as_slice(), TFHD, SENC, UDTA].concat()
        );
    }

    #[test]
    fn a_box_of_no_tfhd_is_refused() {
        assert_eq!(
            TrackFragmentBox::decode_payload(TFDT),
            Err(Error::missing_mandatory_box(
                TrackFragmentHeaderBox::BOX_TYPE
            ))
        );
    }

    #[test]
    fn a_tfdt_that_does_not_read_is_kept_as_its_bytes() {
        let unread = b"\0\0\0\x0ctfdt\x02\0\0\0";
        let bytes = [b"\0\0\0\x28traf".as_slice(), TFHD, unread].concat();
        let (read, _) = TrackFragmentBox::decode(&bytes).unwrap();

        assert_eq!((encoded(&read), read.tfdt), (bytes, None));
    }
}
