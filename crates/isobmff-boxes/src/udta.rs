//! [`UserDataBox`], the container of the user information about a presentation or a track

use alloc::vec::Vec;

use isobmff_core::{
    AnyBox, BoxDecode, BoxDefinition, BoxEncode, BoxType, ChildBoxes, Error, FieldReader,
    FieldWriter, OtherBoxes,
};

use crate::cprt::CopyrightBox;
use crate::hinf::HintStatisticsBox;
use crate::hint_info::HintInformationBox;
use crate::kind::KindBox;
use crate::labl::LabelBox;
use crate::ludt::LoudnessBox;
use crate::strk::SubTrackBox;
use crate::tsel::TrackSelectionBox;

/// [`UserDataBox`] (`udta`), ISO/IEC 14496-12 §8.10.1
///
/// Holds boxes of informative user data about the box that holds it, the
/// presentation or a track.
///
/// The box writes the `cprt`s, `tsel`, the `kind`s, the `labl`s, the `strk`s
/// and the `ludt`s in this order, whatever order they were read in: the order
/// of ISO/IEC 14496-12 Table 2, which lists `labl` after `kind` in the
/// `udta` of a `prsl`. Then come `hnti` and `hinf`, which Table 2 does not
/// list, in the order of §9.1.4 and §9.1.5. Children of other types, and a
/// child of these types that does not read, are kept in
/// [`other_boxes`](Self::other_boxes), and written back after them; a `tsel`,
/// `hnti` or `hinf` that does not read reads back [`None`].
#[doc(alias = "udta")]
#[non_exhaustive]
#[derive(Clone, Default, PartialEq, Debug)]
pub struct UserDataBox {
    /// Copyright notices, in the order they came
    pub cprt: Vec<CopyrightBox>,
    /// Switch group and attributes of the track whose `udta` holds the box
    pub tsel: Option<TrackSelectionBox>,
    /// Roles or kinds of the track, in the order they came
    pub kind: Vec<KindBox>,
    /// Textual descriptions, in the order they came
    pub labl: Vec<LabelBox>,
    /// Sub tracks of the track, in the order they came
    pub strk: Vec<SubTrackBox>,
    /// Loudness declarations of the audio track, in the order they came
    pub ludt: Vec<LoudnessBox>,
    /// SDP information of the movie or of the hint track
    pub hnti: Option<HintInformationBox>,
    /// Statistics of the hint track
    pub hinf: Option<HintStatisticsBox>,
    /// Children of a type the box holds no field for
    pub other_boxes: OtherBoxes,
}

impl BoxDefinition for UserDataBox {
    const BOX_TYPE: BoxType = BoxType::compact(*b"udta");
}

impl BoxDecode for UserDataBox {
    fn decode_fields(reader: &mut FieldReader<'_>) -> Result<Self, Error> {
        let mut children = ChildBoxes::decode(reader.take_remainder())?;

        Ok(Self {
            cprt: children.take_zero_or_more_if_decoded(),
            tsel: children.take_zero_or_one_if_decoded()?,
            kind: children.take_zero_or_more_if_decoded(),
            labl: children.take_zero_or_more_if_decoded(),
            strk: children.take_zero_or_more_if_decoded(),
            ludt: children.take_zero_or_more_if_decoded(),
            hnti: children.take_zero_or_one_if_decoded()?,
            hinf: children.take_zero_or_one_if_decoded()?,
            other_boxes: OtherBoxes::from(children),
        })
    }
}

impl BoxEncode for UserDataBox {
    fn payload_len(&self) -> u64 {
        self.cprt
            .iter()
            .map(BoxEncode::encoded_len)
            .chain(self.tsel.iter().map(BoxEncode::encoded_len))
            .chain(self.kind.iter().map(BoxEncode::encoded_len))
            .chain(self.labl.iter().map(BoxEncode::encoded_len))
            .chain(self.strk.iter().map(BoxEncode::encoded_len))
            .chain(self.ludt.iter().map(BoxEncode::encoded_len))
            .chain(self.hnti.iter().map(BoxEncode::encoded_len))
            .chain(self.hinf.iter().map(BoxEncode::encoded_len))
            .chain(self.other_boxes.as_slice().iter().map(AnyBox::encoded_len))
            .fold(0, u64::saturating_add)
    }

    fn encode_fields(&self, writer: &mut FieldWriter<'_>) -> Result<(), Error> {
        let mut rest = writer.take_remainder();
        for copyright in &self.cprt {
            rest = copyright.encode(rest)?;
        }
        if let Some(tsel) = &self.tsel {
            rest = tsel.encode(rest)?;
        }
        for kind in &self.kind {
            rest = kind.encode(rest)?;
        }
        for label in &self.labl {
            rest = label.encode(rest)?;
        }
        for sub_track in &self.strk {
            rest = sub_track.encode(rest)?;
        }
        for loudness in &self.ludt {
            rest = loudness.encode(rest)?;
        }
        if let Some(hnti) = &self.hnti {
            rest = hnti.encode(rest)?;
        }
        if let Some(hinf) = &self.hinf {
            rest = hinf.encode(rest)?;
        }
        for kept in self.other_boxes.as_slice() {
            rest = kept.encode(rest)?;
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use alloc::vec;

    use isobmff_core::{BoxDecode, BoxDefinition, ChildBoxes, Error, OtherBoxes};

    use super::UserDataBox;
    use crate::cprt::CopyrightBox;
    use crate::encoded;
    use crate::hinf::HintStatisticsBox;
    use crate::hint_info::HintInformationBox;
    use crate::kind::KindBox;
    use crate::labl::LabelBox;
    use crate::ludt::LoudnessBox;
    use crate::strk::SubTrackBox;
    use crate::tsel::TrackSelectionBox;

    /// `cprt` of the language `und` and the notice `(c)`
    const CPRT: &[u8] = b"\0\0\0\x12cprt\0\0\0\0\x55\xc4(c)\0";

    /// `tsel` of switch group 1 and no attribute
    const TSEL: &[u8] = b"\0\0\0\x10tsel\0\0\0\0\0\0\0\x01";

    /// `kind` of the URI `urn:example` alone
    const KIND: &[u8] = b"\0\0\0\x18kind\0\0\0\0urn:example\0";

    /// `labl` of label 0, in `en`, `A`
    const LABL: &[u8] = b"\0\0\0\x13labl\0\0\0\0\0\0en\0A\0";

    /// `strk` of an `stri` of sub track 2 and an `strd` holding nothing
    const STRK: &[u8] = b"\0\0\0\x24strk\0\0\0\x14stri\0\0\0\0\0\0\0\0\0\0\0\x02\0\0\0\x08strd";

    /// `ludt` holding nothing
    const LUDT: &[u8] = b"\0\0\0\x08ludt";

    /// `hnti` holding nothing
    const HNTI: &[u8] = b"\0\0\0\x08hnti";

    /// `hinf` holding nothing
    const HINF: &[u8] = b"\0\0\0\x08hinf";

    /// Box of a type `udta` holds no field for
    const OTHER: &[u8] = b"\0\0\0\x08xtra";

    /// `strk` of an `stri` and no `strd`, which does not read
    const STRK_UNREAD: &[u8] = b"\0\0\0\x1cstrk\0\0\0\x14stri\0\0\0\0\0\0\0\0\0\0\0\x02";

    fn user_data() -> UserDataBox {
        UserDataBox {
            cprt: vec![CopyrightBox::decode(CPRT).unwrap().0],
            tsel: Some(TrackSelectionBox::decode(TSEL).unwrap().0),
            kind: vec![KindBox::decode(KIND).unwrap().0],
            labl: vec![LabelBox::decode(LABL).unwrap().0],
            strk: vec![SubTrackBox::decode(STRK).unwrap().0],
            ludt: vec![LoudnessBox::default()],
            hnti: Some(HintInformationBox::default()),
            hinf: Some(HintStatisticsBox::default()),
            other_boxes: OtherBoxes::from(ChildBoxes::decode(OTHER).unwrap()),
        }
    }

    #[test]
    fn reads_its_children_in_any_order_and_keeps_the_child_it_has_no_field_for() {
        let bytes = [
            b"\0\0\0\x99udta".as_slice(),
            OTHER,
            HINF,
            HNTI,
            LUDT,
            STRK,
            LABL,
            KIND,
            TSEL,
            CPRT,
        ]
        .concat();

        assert_eq!(
            UserDataBox::decode(&bytes),
            Ok((user_data(), b"".as_slice()))
        );
    }

    #[test]
    fn writes_its_children_in_the_order_of_table_2_then_the_hint_boxes_then_the_child_it_has_no_field_for()
     {
        assert_eq!(
            encoded(&user_data()),
            [
                b"\0\0\0\x99udta".as_slice(),
                CPRT,
                TSEL,
                KIND,
                LABL,
                STRK,
                LUDT,
                HNTI,
                HINF,
                OTHER
            ]
            .concat()
        );
    }

    #[test]
    fn a_sub_track_that_does_not_read_is_kept_as_its_bytes() {
        let bytes = [b"\0\0\0\x24udta".as_slice(), STRK_UNREAD].concat();
        let (read, _) = UserDataBox::decode(&bytes).unwrap();
        let kept = UserDataBox {
            other_boxes: OtherBoxes::from(ChildBoxes::decode(STRK_UNREAD).unwrap()),
            ..UserDataBox::default()
        };

        assert_eq!((encoded(&read), read), (bytes, kept));
    }

    #[test]
    fn a_box_of_two_track_selections_is_refused() {
        assert_eq!(
            UserDataBox::decode_payload(&[TSEL, TSEL].concat()),
            Err(Error::duplicate_box(TrackSelectionBox::BOX_TYPE))
        );
    }
}
