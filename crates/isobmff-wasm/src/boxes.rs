//! [`isobmff::boxes`]: [`CONTAINER_BOXES`](isobmff::boxes::CONTAINER_BOXES), the boxes from the `moov` down to the `stsd` as handles into one root, and the data types their fields and samples carry

use alloc::rc::Rc;
use core::cell::RefCell;

use isobmff::boxes;
use serde::Serialize;
use tsify::{Ts, Tsify};
use wasm_bindgen::prelude::wasm_bindgen;

use crate::Failure;
use crate::framing::{BoxType, FourCc};

/// [`isobmff::boxes::CONTAINER_BOXES`], called as a function
///
/// # Errors
///
/// `"binding"`: a value did not convert to a plain object.
#[wasm_bindgen(js_name = CONTAINER_BOXES)]
pub fn container_boxes() -> Result<Vec<Ts<BoxType>>, Failure> {
    Ok(boxes::CONTAINER_BOXES
        .iter()
        .map(|&box_type| BoxType::from(box_type).into_ts())
        .collect::<Result<_, _>>()?)
}

/// [`isobmff::boxes::HeaderDuration`], the duration it states or none where it is indeterminate
#[derive(Debug, Clone, Copy, Serialize, Tsify)]
#[tsify(large_number_types_as_bigints)]
pub struct HeaderDuration(Option<u64>);

/// [`isobmff::boxes::IsLeading`]
#[derive(Debug, Clone, Copy, Serialize, Tsify)]
#[serde(rename_all = "camelCase")]
#[non_exhaustive]
pub enum IsLeading {
    /// [`isobmff::boxes::IsLeading::Unknown`]
    Unknown,
    /// [`isobmff::boxes::IsLeading::LeadingWithDependency`]
    LeadingWithDependency,
    /// [`isobmff::boxes::IsLeading::NotLeading`]
    NotLeading,
    /// [`isobmff::boxes::IsLeading::LeadingWithoutDependency`]
    LeadingWithoutDependency,
}

/// [`isobmff::boxes::SampleDependsOn`]
#[derive(Debug, Clone, Copy, Serialize, Tsify)]
#[serde(rename_all = "camelCase")]
#[non_exhaustive]
pub enum SampleDependsOn {
    /// [`isobmff::boxes::SampleDependsOn::Unknown`]
    Unknown,
    /// [`isobmff::boxes::SampleDependsOn::DependsOnOthers`]
    DependsOnOthers,
    /// [`isobmff::boxes::SampleDependsOn::DoesNotDependOnOthers`]
    DoesNotDependOnOthers,
    /// [`isobmff::boxes::SampleDependsOn::Reserved`]
    Reserved,
}

/// [`isobmff::boxes::SampleIsDependedOn`]
#[derive(Debug, Clone, Copy, Serialize, Tsify)]
#[serde(rename_all = "camelCase")]
#[non_exhaustive]
pub enum SampleIsDependedOn {
    /// [`isobmff::boxes::SampleIsDependedOn::Unknown`]
    Unknown,
    /// [`isobmff::boxes::SampleIsDependedOn::NotDisposable`]
    NotDisposable,
    /// [`isobmff::boxes::SampleIsDependedOn::Disposable`]
    Disposable,
    /// [`isobmff::boxes::SampleIsDependedOn::Reserved`]
    Reserved,
}

/// [`isobmff::boxes::SampleHasRedundancy`]
#[derive(Debug, Clone, Copy, Serialize, Tsify)]
#[serde(rename_all = "camelCase")]
#[non_exhaustive]
pub enum SampleHasRedundancy {
    /// [`isobmff::boxes::SampleHasRedundancy::Unknown`]
    Unknown,
    /// [`isobmff::boxes::SampleHasRedundancy::RedundantCoding`]
    RedundantCoding,
    /// [`isobmff::boxes::SampleHasRedundancy::NoRedundantCoding`]
    NoRedundantCoding,
    /// [`isobmff::boxes::SampleHasRedundancy::Reserved`]
    Reserved,
}

/// [`isobmff::boxes::SampleDependencyTypeEntry`]
#[derive(Debug, Clone, Copy, Serialize, Tsify)]
#[serde(rename_all = "camelCase")]
#[non_exhaustive]
pub struct SampleDependencyTypeEntry {
    /// [`isobmff::boxes::SampleDependencyTypeEntry::is_leading`]
    pub is_leading: IsLeading,
    /// [`isobmff::boxes::SampleDependencyTypeEntry::sample_depends_on`]
    pub sample_depends_on: SampleDependsOn,
    /// [`isobmff::boxes::SampleDependencyTypeEntry::sample_is_depended_on`]
    pub sample_is_depended_on: SampleIsDependedOn,
    /// [`isobmff::boxes::SampleDependencyTypeEntry::sample_has_redundancy`]
    pub sample_has_redundancy: SampleHasRedundancy,
}

impl From<boxes::SampleDependencyTypeEntry> for SampleDependencyTypeEntry {
    fn from(entry: boxes::SampleDependencyTypeEntry) -> Self {
        Self {
            is_leading: match entry.is_leading() {
                boxes::IsLeading::Unknown => IsLeading::Unknown,
                boxes::IsLeading::LeadingWithDependency => IsLeading::LeadingWithDependency,
                boxes::IsLeading::NotLeading => IsLeading::NotLeading,
                boxes::IsLeading::LeadingWithoutDependency => IsLeading::LeadingWithoutDependency,
            },
            sample_depends_on: match entry.sample_depends_on() {
                boxes::SampleDependsOn::Unknown => SampleDependsOn::Unknown,
                boxes::SampleDependsOn::DependsOnOthers => SampleDependsOn::DependsOnOthers,
                boxes::SampleDependsOn::DoesNotDependOnOthers => {
                    SampleDependsOn::DoesNotDependOnOthers
                }
                boxes::SampleDependsOn::Reserved => SampleDependsOn::Reserved,
            },
            sample_is_depended_on: match entry.sample_is_depended_on() {
                boxes::SampleIsDependedOn::Unknown => SampleIsDependedOn::Unknown,
                boxes::SampleIsDependedOn::NotDisposable => SampleIsDependedOn::NotDisposable,
                boxes::SampleIsDependedOn::Disposable => SampleIsDependedOn::Disposable,
                boxes::SampleIsDependedOn::Reserved => SampleIsDependedOn::Reserved,
            },
            sample_has_redundancy: match entry.sample_has_redundancy() {
                boxes::SampleHasRedundancy::Unknown => SampleHasRedundancy::Unknown,
                boxes::SampleHasRedundancy::RedundantCoding => SampleHasRedundancy::RedundantCoding,
                boxes::SampleHasRedundancy::NoRedundantCoding => {
                    SampleHasRedundancy::NoRedundantCoding
                }
                boxes::SampleHasRedundancy::Reserved => SampleHasRedundancy::Reserved,
            },
        }
    }
}

/// [`isobmff::boxes::PaddingBitsEntry`]
#[derive(Debug, Clone, Copy, Serialize, Tsify)]
#[non_exhaustive]
pub struct PaddingBitsEntry {
    /// [`isobmff::boxes::PaddingBitsEntry::pad`]
    pub pad: u8,
}

/// [`isobmff::boxes::DegradationPriorityEntry`]
#[derive(Debug, Clone, Copy, Serialize, Tsify)]
#[non_exhaustive]
pub struct DegradationPriorityEntry {
    /// [`isobmff::boxes::DegradationPriorityEntry::priority`]
    pub priority: u16,
}

/// [`isobmff::boxes::SampleFlags`], its fields as the tables stating them do
#[derive(Debug, Clone, Copy, Serialize, Tsify)]
#[serde(rename_all = "camelCase")]
#[non_exhaustive]
pub struct SampleFlags {
    /// [`isobmff::boxes::SampleFlags::sample_dependency_type`]
    pub sample_dependency_type: SampleDependencyTypeEntry,
    /// [`isobmff::boxes::SampleFlags::padding_bits`]
    pub padding_bits: PaddingBitsEntry,
    /// [`isobmff::boxes::SampleFlags::sample_is_non_sync_sample`]
    pub sample_is_non_sync_sample: bool,
    /// [`isobmff::boxes::SampleFlags::degradation_priority`]
    pub degradation_priority: DegradationPriorityEntry,
}

impl From<boxes::SampleFlags> for SampleFlags {
    fn from(flags: boxes::SampleFlags) -> Self {
        Self {
            sample_dependency_type: flags.sample_dependency_type().into(),
            padding_bits: PaddingBitsEntry {
                pad: flags.padding_bits().pad(),
            },
            sample_is_non_sync_sample: flags.sample_is_non_sync_sample(),
            degradation_priority: DegradationPriorityEntry {
                priority: flags.degradation_priority().priority(),
            },
        }
    }
}

/// The path to one `trak` of a root `moov`, followed again on every call
#[derive(Debug, Clone)]
struct TrackPath {
    /// The `moov` the handles share
    root: Rc<RefCell<boxes::MovieBox>>,
    /// The position of the `trak` among those of the `moov`
    trak: usize,
}

impl TrackPath {
    /// Returns what `read` makes of the `trak`, or the failure of a path that leads to no `trak`
    fn read<T>(&self, read: impl FnOnce(&boxes::TrackBox) -> T) -> Result<T, Failure> {
        self.root
            .borrow()
            .trak()
            .get(self.trak)
            .map(read)
            .ok_or_else(Failure::handle)
    }
}

/// [`isobmff::boxes::MovieBox`] (`moov`), ISO/IEC 14496-12 §8.2.1, the root its handles share
#[doc(alias = "moov")]
#[wasm_bindgen]
#[derive(Debug)]
pub struct MovieBox(Rc<RefCell<boxes::MovieBox>>);

impl MovieBox {
    /// Makes `movie` the root of the handles reached from it
    pub(crate) fn new(movie: boxes::MovieBox) -> Self {
        Self(Rc::new(RefCell::new(movie)))
    }
}

#[wasm_bindgen]
impl MovieBox {
    /// [`isobmff::boxes::MovieBox::trak`], a handle to each
    pub fn trak(&self) -> Vec<TrackBox> {
        (0..self.0.borrow().trak().len())
            .map(|trak| {
                TrackBox(TrackPath {
                    root: Rc::clone(&self.0),
                    trak,
                })
            })
            .collect()
    }
}

/// [`isobmff::boxes::TrackBox`] (`trak`), ISO/IEC 14496-12 §8.3.1, a handle into its root
#[doc(alias = "trak")]
#[wasm_bindgen]
#[derive(Debug)]
pub struct TrackBox(TrackPath);

#[wasm_bindgen]
impl TrackBox {
    /// [`isobmff::boxes::TrackBox::tkhd`], a handle
    pub fn tkhd(&self) -> TrackHeaderBox {
        TrackHeaderBox(self.0.clone())
    }

    /// [`isobmff::boxes::TrackBox::mdia`], a handle
    pub fn mdia(&self) -> MediaBox {
        MediaBox(self.0.clone())
    }
}

/// [`isobmff::boxes::TrackHeaderBox`] (`tkhd`), ISO/IEC 14496-12 §8.3.2, a handle into its root
#[doc(alias = "tkhd")]
#[wasm_bindgen]
#[derive(Debug)]
pub struct TrackHeaderBox(TrackPath);

#[wasm_bindgen]
impl TrackHeaderBox {
    /// [`isobmff::boxes::TrackHeaderBox::track_id`]
    ///
    /// # Errors
    ///
    /// `"handle"`: the path leads to no box.
    #[wasm_bindgen(js_name = trackId)]
    pub fn track_id(&self) -> Result<u32, Failure> {
        self.0.read(|trak| trak.tkhd().track_id())
    }
}

/// [`isobmff::boxes::MediaBox`] (`mdia`), ISO/IEC 14496-12 §8.4.1, a handle into its root
#[doc(alias = "mdia")]
#[wasm_bindgen]
#[derive(Debug)]
pub struct MediaBox(TrackPath);

#[wasm_bindgen]
impl MediaBox {
    /// [`isobmff::boxes::MediaBox::mdhd`], a handle
    pub fn mdhd(&self) -> MediaHeaderBox {
        MediaHeaderBox(self.0.clone())
    }

    /// [`isobmff::boxes::MediaBox::hdlr`], a handle
    pub fn hdlr(&self) -> HandlerBox {
        HandlerBox(self.0.clone())
    }

    /// [`isobmff::boxes::MediaBox::minf`], a handle
    pub fn minf(&self) -> MediaInformationBox {
        MediaInformationBox(self.0.clone())
    }
}

/// [`isobmff::boxes::MediaHeaderBox`] (`mdhd`), ISO/IEC 14496-12 §8.4.2, a handle into its root
#[doc(alias = "mdhd")]
#[wasm_bindgen]
#[derive(Debug)]
pub struct MediaHeaderBox(TrackPath);

#[wasm_bindgen]
impl MediaHeaderBox {
    /// [`isobmff::boxes::MediaHeaderBox::timescale`]
    ///
    /// # Errors
    ///
    /// `"handle"`: the path leads to no box.
    pub fn timescale(&self) -> Result<u32, Failure> {
        self.0.read(|trak| trak.mdia().mdhd().timescale())
    }

    /// [`isobmff::boxes::MediaHeaderBox::duration`]
    ///
    /// # Errors
    ///
    /// `"handle"`: the path leads to no box; `"binding"`: the value did not
    /// convert to a plain object.
    pub fn duration(&self) -> Result<Ts<HeaderDuration>, Failure> {
        Ok(self
            .0
            .read(|trak| HeaderDuration(trak.mdia().mdhd().duration().get()))?
            .into_ts()?)
    }
}

/// [`isobmff::boxes::HandlerBox`] (`hdlr`), ISO/IEC 14496-12 §8.4.3, a handle into its root
#[doc(alias = "hdlr")]
#[wasm_bindgen]
#[derive(Debug)]
pub struct HandlerBox(TrackPath);

#[wasm_bindgen]
impl HandlerBox {
    /// [`isobmff::boxes::HandlerBox::handler_type`]
    ///
    /// # Errors
    ///
    /// `"handle"`: the path leads to no box; `"binding"`: the value did not
    /// convert to a plain object.
    #[wasm_bindgen(js_name = handlerType)]
    pub fn handler_type(&self) -> Result<Ts<FourCc>, Failure> {
        Ok(FourCc::from(self.0.read(|trak| trak.mdia().hdlr().handler_type())?).into_ts()?)
    }
}

/// [`isobmff::boxes::MediaInformationBox`] (`minf`), ISO/IEC 14496-12 §8.4.4, a handle into its root
#[doc(alias = "minf")]
#[wasm_bindgen]
#[derive(Debug)]
pub struct MediaInformationBox(TrackPath);

#[wasm_bindgen]
impl MediaInformationBox {
    /// [`isobmff::boxes::MediaInformationBox::stbl`], a handle
    pub fn stbl(&self) -> SampleTableBox {
        SampleTableBox(self.0.clone())
    }
}

/// [`isobmff::boxes::SampleTableBox`] (`stbl`), ISO/IEC 14496-12 §8.5.1, a handle into its root
#[doc(alias = "stbl")]
#[wasm_bindgen]
#[derive(Debug)]
pub struct SampleTableBox(TrackPath);

#[wasm_bindgen]
impl SampleTableBox {
    /// [`isobmff::boxes::SampleTableBox::stsd`], a handle
    pub fn stsd(&self) -> SampleDescriptionBox {
        SampleDescriptionBox(self.0.clone())
    }
}

/// [`isobmff::boxes::SampleDescriptionBox`] (`stsd`), ISO/IEC 14496-12 §8.5.2, a handle into its root
#[doc(alias = "stsd")]
#[wasm_bindgen]
#[derive(Debug)]
pub struct SampleDescriptionBox(TrackPath);

#[wasm_bindgen]
impl SampleDescriptionBox {
    /// [`isobmff::boxes::SampleDescriptionBox::entries`], a handle to each
    ///
    /// # Errors
    ///
    /// `"handle"`: the path leads to no box.
    pub fn entries(&self) -> Result<Vec<AnyBox>, Failure> {
        let count = self
            .0
            .read(|trak| trak.mdia().minf().stbl().stsd().entries().len())?;
        Ok((0..count)
            .map(|entry| AnyBox {
                track: self.0.clone(),
                entry,
            })
            .collect())
    }
}

/// [`isobmff::AnyBox`], an entry of a `stsd`, a handle into its root
#[wasm_bindgen]
#[derive(Debug)]
pub struct AnyBox {
    /// The `trak` whose `stsd` holds the entry
    track: TrackPath,
    /// The position of the entry in the `stsd`
    entry: usize,
}

#[wasm_bindgen]
impl AnyBox {
    /// [`isobmff::AnyBox::box_type`]
    ///
    /// # Errors
    ///
    /// `"handle"`: the path leads to no box; `"binding"`: the value did not
    /// convert to a plain object.
    #[wasm_bindgen(js_name = boxType)]
    pub fn box_type(&self) -> Result<Ts<BoxType>, Failure> {
        let box_type = self
            .track
            .read(|trak| {
                trak.mdia()
                    .minf()
                    .stbl()
                    .stsd()
                    .entries()
                    .get(self.entry)
                    .map(isobmff::AnyBox::box_type)
            })?
            .ok_or_else(Failure::handle)?;
        Ok(BoxType::from(box_type).into_ts()?)
    }
}
