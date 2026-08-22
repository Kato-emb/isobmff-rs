//! [`TrackFragmentBox`] (`traf`), ISO/IEC 14496-12 §8.8.6

use alloc::vec::Vec;

use isobmff_core::{
    AnyBox, BoxDecode, BoxDefinition, BoxEncode, BoxType, ChildBoxes, Error, FieldReader,
    FieldWriter, OtherBoxes, boxes,
};

use crate::tfdt::TrackFragmentBaseMediaDecodeTimeBox;
use crate::tfhd::TrackFragmentHeaderBox;
use crate::trun::TrackRunBox;

/// Box that carries what one movie fragment adds to one track
///
/// [`TrackFragmentBox`] (`traf`), ISO/IEC 14496-12 §8.8.6. The `tfhd` sets up
/// what the runs share and each `trun` documents a contiguous run of samples, so
/// a fragment adding nothing but time to a track carries no run at all.
///
/// A `tfhd` stating `duration-is-empty` declares that the fragment holds no
/// samples, and §8.8.8 has such a fragment hold no track runs — so this box
/// refuses the two together, in [`new`](Self::new) as in
/// [`decode_payload`](BoxDecode::decode_payload).
///
/// The `sdtp`, `sbgp`, `subs`, `saiz`, and `saio` children have no fields yet, so
/// they are kept in [`other_boxes`](Self::other_boxes) and written back unread.
///
/// On encode the children are written in the order the spec lists them — `tfhd`,
/// `tfdt`, then the runs — and then the children no field claims, so a round-trip
/// settles the order rather than preserving it.
#[doc(alias = "traf")]
#[non_exhaustive]
#[derive(Clone, PartialEq, Debug)]
pub struct TrackFragmentBox {
    tfhd: TrackFragmentHeaderBox,
    tfdt: Option<TrackFragmentBaseMediaDecodeTimeBox>,
    trun: Vec<TrackRunBox>,
    other_boxes: OtherBoxes,
}

impl TrackFragmentBox {
    /// Creates the box from the header, the decode time, and the runs of samples
    ///
    /// Returns `None` when `tfhd` states `duration-is-empty` while `trun` holds a
    /// run, which the spec has hold no runs at all.
    #[must_use]
    pub fn new(
        tfhd: TrackFragmentHeaderBox,
        tfdt: Option<TrackFragmentBaseMediaDecodeTimeBox>,
        trun: Vec<TrackRunBox>,
    ) -> Option<Self> {
        if tfhd.duration_is_empty() && !trun.is_empty() {
            return None;
        }

        Some(Self {
            tfhd,
            tfdt,
            trun,
            other_boxes: OtherBoxes::new(),
        })
    }

    /// Returns what the runs of this fragment share
    #[must_use]
    pub const fn tfhd(&self) -> &TrackFragmentHeaderBox {
        &self.tfhd
    }

    /// Returns the decode time the samples of this fragment start at
    #[must_use]
    pub const fn tfdt(&self) -> Option<&TrackFragmentBaseMediaDecodeTimeBox> {
        self.tfdt.as_ref()
    }

    /// Returns the runs of samples this fragment adds, in the order they came
    #[must_use]
    pub fn trun(&self) -> &[TrackRunBox] {
        &self.trun
    }

    /// Returns the children no field of this box claims, in the order they came
    #[must_use]
    pub fn other_boxes(&self) -> &[AnyBox] {
        self.other_boxes.as_slice()
    }
}

impl BoxDefinition for TrackFragmentBox {
    const BOX_TYPE: BoxType = BoxType::compact(*b"traf");
}

impl BoxDecode for TrackFragmentBox {
    /// # Errors
    ///
    /// * The failures of [`boxes`]: a child does not frame as a box.
    /// * [`MissingMandatoryBox`](isobmff_core::ErrorKind::MissingMandatoryBox): no `tfhd`.
    /// * [`DuplicateBox`](isobmff_core::ErrorKind::DuplicateBox): more than one `tfhd`, or
    ///   more than one `tfdt`.
    /// * [`ForbiddenChildBox`](isobmff_core::ErrorKind::ForbiddenChildBox): the `tfhd` states
    ///   `duration-is-empty` and a `trun` follows it.
    /// * Whatever the child reports, on the [`containers`](Error::containers) path: a child does
    ///   not decode.
    fn decode_fields(reader: &mut FieldReader<'_>) -> Result<Self, Error> {
        let mut tfhd_boxes = ChildBoxes::new();
        let mut tfdt_boxes = ChildBoxes::new();
        let mut trun_boxes = ChildBoxes::new();
        let mut other_boxes = OtherBoxes::new();

        for child in boxes(reader.take_remainder()) {
            let child = child?;
            let box_type = child.header().box_type();

            if box_type == TrackFragmentHeaderBox::BOX_TYPE {
                tfhd_boxes.push(child);
            } else if box_type == TrackFragmentBaseMediaDecodeTimeBox::BOX_TYPE {
                tfdt_boxes.push(child);
            } else if box_type == TrackRunBox::BOX_TYPE {
                trun_boxes.push(child);
            } else {
                other_boxes.keep(child);
            }
        }

        let tfhd: TrackFragmentHeaderBox = tfhd_boxes.exactly_one()?;
        // Why not weighing the runs once they are read: the rule turns on whether
        // a run is there at all, and a fragment that declares an empty duration
        // would have every run of an input it goes on to refuse decoded first.
        if tfhd.duration_is_empty() && !trun_boxes.is_empty() {
            return Err(Error::forbidden_child_box(TrackRunBox::BOX_TYPE));
        }

        Ok(Self {
            tfhd,
            tfdt: tfdt_boxes.zero_or_one()?,
            trun: trun_boxes.zero_or_more()?,
            other_boxes,
        })
    }
}

impl BoxEncode for TrackFragmentBox {
    fn payload_len(&self) -> u64 {
        let decode_time = self.tfdt.as_ref().map_or(0, |tfdt| tfdt.encoded_len());
        let runs = self.trun.iter().fold(0_u64, |total, trun| {
            total.saturating_add(trun.encoded_len())
        });
        let others = self
            .other_boxes
            .as_slice()
            .iter()
            .fold(0_u64, |total, other| {
                total.saturating_add(other.encoded_len())
            });

        self.tfhd
            .encoded_len()
            .saturating_add(decode_time)
            .saturating_add(runs)
            .saturating_add(others)
    }

    fn encode_fields(&self, writer: &mut FieldWriter<'_>) -> Result<(), Error> {
        let mut rest = self.tfhd.encode(writer.take_remainder())?;
        if let Some(tfdt) = &self.tfdt {
            rest = tfdt.encode(rest)?;
        }
        for trun in &self.trun {
            rest = trun.encode(rest)?;
        }
        for other in self.other_boxes.as_slice() {
            rest = other.encode(rest)?;
        }

        Ok(())
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use alloc::vec;
    use alloc::vec::Vec;

    use isobmff_core::{
        BoxDecode, BoxDefinition as _, BoxEncode, BoxType, Error, FullBoxFlags, boxes,
    };

    use super::TrackFragmentBox;
    use crate::tfdt::TrackFragmentBaseMediaDecodeTimeBox;
    use crate::tfhd::TrackFragmentHeaderBox;
    use crate::trun::{TrackRunBox, TrackRunSample};

    /// Flags of a fragment declaring that it holds no samples
    fn duration_is_empty() -> FullBoxFlags {
        FullBoxFlags::new(0x0001_0000).unwrap()
    }

    /// Fragment header of a track whose samples all last the same time
    fn track_fragment_header(flags: FullBoxFlags, track_id: u32) -> TrackFragmentHeaderBox {
        TrackFragmentHeaderBox::new(flags, track_id, None, None, Some(1_024), None, None).unwrap()
    }

    /// Run of one sample stating its size
    fn track_run() -> TrackRunBox {
        TrackRunBox::new(
            Some(0),
            None,
            vec![TrackRunSample::new(None, Some(1_024), None, None).unwrap()],
        )
        .unwrap()
    }

    /// Track fragment adding one run of samples to the track it names
    pub(crate) fn track_fragment(track_id: u32) -> TrackFragmentBox {
        TrackFragmentBox::new(
            track_fragment_header(FullBoxFlags::ZERO, track_id),
            Some(TrackFragmentBaseMediaDecodeTimeBox::new(1_024)),
            vec![track_run()],
        )
        .unwrap()
    }

    /// Writes the payload of the box and returns the bytes it occupies
    fn encoded_payload(track_fragment: &TrackFragmentBox) -> Vec<u8> {
        let mut buffer = vec![0; usize::try_from(track_fragment.payload_len()).unwrap()];
        track_fragment.encode_payload(&mut buffer).unwrap();

        buffer
    }

    #[test]
    fn a_box_reads_back_as_the_value_that_wrote_it() {
        let payload = encoded_payload(&track_fragment(1));

        assert_eq!(
            TrackFragmentBox::decode_payload(&payload).unwrap(),
            track_fragment(1)
        );
    }

    #[test]
    fn the_children_are_written_in_the_order_the_spec_lists_them() {
        let payload = encoded_payload(&track_fragment(1));

        let box_types: Vec<BoxType> = boxes(&payload)
            .map(|child| child.unwrap().header().box_type())
            .collect();

        assert_eq!(
            box_types,
            [
                TrackFragmentHeaderBox::BOX_TYPE,
                TrackFragmentBaseMediaDecodeTimeBox::BOX_TYPE,
                TrackRunBox::BOX_TYPE,
            ]
        );
    }

    #[test]
    fn a_fragment_adding_no_run_of_samples_reads_back_as_the_value_that_wrote_it() {
        let empty = TrackFragmentBox::new(
            track_fragment_header(FullBoxFlags::ZERO, 1),
            None,
            Vec::new(),
        )
        .unwrap();

        let payload = encoded_payload(&empty);

        assert_eq!(TrackFragmentBox::decode_payload(&payload).unwrap(), empty);
    }

    #[test]
    fn a_fragment_declaring_an_empty_duration_alongside_a_run_cannot_be_built() {
        assert_eq!(
            TrackFragmentBox::new(
                track_fragment_header(duration_is_empty(), 1),
                None,
                vec![track_run()]
            ),
            None
        );
    }

    #[test]
    fn a_payload_holding_a_run_the_empty_duration_forbids_is_rejected() {
        let empty = TrackFragmentBox::new(
            track_fragment_header(duration_is_empty(), 1),
            None,
            Vec::new(),
        )
        .unwrap();
        let run = track_run();
        let mut encoded_run = vec![0; usize::try_from(run.encoded_len()).unwrap()];
        run.encode(&mut encoded_run).unwrap();

        let payload = [encoded_payload(&empty), encoded_run].concat();

        assert_eq!(
            TrackFragmentBox::decode_payload(&payload),
            Err(Error::forbidden_child_box(TrackRunBox::BOX_TYPE))
        );
    }

    #[test]
    fn a_box_holding_no_fragment_header_is_rejected() {
        assert_eq!(
            TrackFragmentBox::decode_payload(b""),
            Err(Error::missing_mandatory_box(BoxType::compact(*b"tfhd")))
        );
    }

    #[test]
    fn the_children_this_box_has_no_field_for_are_kept_unread() {
        let payload = [
            encoded_payload(&track_fragment(1)),
            vec![0, 0, 0, 0x0c, b's', b'd', b't', b'p', 0, 0, 0, 0],
        ]
        .concat();

        let track_fragment = TrackFragmentBox::decode_payload(&payload).unwrap();

        assert_eq!(encoded_payload(&track_fragment), payload);
    }
}
