//! [`isobmff::boxes`]: [`CONTAINER_BOXES`](isobmff::boxes::CONTAINER_BOXES), the `ftyp`, the boxes from the `moov` down to the sample tables, each a root of its own or a handle into one, the data types samples carry, and [`Mp4EpochSeconds`]
//!
//! The boxes are the `ftyp` of ISO/IEC 14496-12 §4.3 and those of §8.2.1
//! (`moov`) to §8.7.5 (`stco`, `co64`), each class citing its own section;
//! the data types are the sample flags of §8.8.3.1 and the `sdtp` entry of
//! §8.6.4 within them, and the time of §6.2.2 that [`Mp4EpochSeconds`]
//! converts.

use alloc::rc::Rc;
use core::cell::RefCell;
use core::fmt;

use isobmff::boxes;
use serde::Serialize;
use wasm_bindgen::prelude::wasm_bindgen;

use crate::framing::{BoxType, JsBoxType};
use crate::{Failure, to_plain};

mod movie;
mod sample_table;

pub use movie::{
    EditBox, EditListBox, HandlerBox, MediaBox, MediaHeaderBox, MediaInformationBox, MovieBox,
    MovieHeaderBox, TrackBox, TrackHeaderBox,
};
pub use sample_table::{
    AnyBox, ChunkOffsets, JsSampleChunks, SampleDescriptionBox, SampleSizeBox, SampleSizes,
    SampleTableBox, SampleToChunkBox, SyncSampleBox, TimeToSampleBox,
};

/// [`isobmff::boxes::CONTAINER_BOXES`], called as a function
///
/// # Errors
///
/// `"binding"`: a value did not convert to a plain object.
#[wasm_bindgen(js_name = CONTAINER_BOXES)]
pub fn container_boxes() -> Result<Vec<JsBoxType>, Failure> {
    boxes::CONTAINER_BOXES
        .iter()
        .map(|&box_type| to_plain(&BoxType::from(box_type)))
        .collect()
}

/// [`isobmff::boxes::IsLeading`]
#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "camelCase")]
enum IsLeading {
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
#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "camelCase")]
enum SampleDependsOn {
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
#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "camelCase")]
enum SampleIsDependedOn {
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
#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "camelCase")]
enum SampleHasRedundancy {
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
#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "camelCase")]
struct SampleDependencyTypeEntry {
    /// [`isobmff::boxes::SampleDependencyTypeEntry::is_leading`]
    is_leading: IsLeading,
    /// [`isobmff::boxes::SampleDependencyTypeEntry::sample_depends_on`]
    sample_depends_on: SampleDependsOn,
    /// [`isobmff::boxes::SampleDependencyTypeEntry::sample_is_depended_on`]
    sample_is_depended_on: SampleIsDependedOn,
    /// [`isobmff::boxes::SampleDependencyTypeEntry::sample_has_redundancy`]
    sample_has_redundancy: SampleHasRedundancy,
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

/// [`isobmff::boxes::SampleFlags`], its fields as the tables stating them do
#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SampleFlags {
    /// [`isobmff::boxes::SampleFlags::sample_dependency_type`]
    sample_dependency_type: SampleDependencyTypeEntry,
    /// [`isobmff::boxes::SampleFlags::padding_bits`], its [`isobmff::boxes::PaddingBitsEntry::pad`]
    padding_bits: u8,
    /// [`isobmff::boxes::SampleFlags::sample_is_non_sync_sample`]
    sample_is_non_sync_sample: bool,
    /// [`isobmff::boxes::SampleFlags::degradation_priority`], its [`isobmff::boxes::DegradationPriorityEntry::priority`]
    degradation_priority: u16,
}

impl From<boxes::SampleFlags> for SampleFlags {
    fn from(flags: boxes::SampleFlags) -> Self {
        Self {
            sample_dependency_type: flags.sample_dependency_type().into(),
            padding_bits: flags.padding_bits().pad(),
            sample_is_non_sync_sample: flags.sample_is_non_sync_sample(),
            degradation_priority: flags.degradation_priority().priority(),
        }
    }
}

#[wasm_bindgen(typescript_custom_section)]
const SAMPLE_FLAGS: &str = r#"
/** `isobmff::boxes::IsLeading` */
export type IsLeading = "unknown" | "leadingWithDependency" | "notLeading" | "leadingWithoutDependency";

/** `isobmff::boxes::SampleDependsOn` */
export type SampleDependsOn = "unknown" | "dependsOnOthers" | "doesNotDependOnOthers" | "reserved";

/** `isobmff::boxes::SampleIsDependedOn` */
export type SampleIsDependedOn = "unknown" | "notDisposable" | "disposable" | "reserved";

/** `isobmff::boxes::SampleHasRedundancy` */
export type SampleHasRedundancy = "unknown" | "redundantCoding" | "noRedundantCoding" | "reserved";

/** `isobmff::boxes::SampleDependencyTypeEntry` */
export interface SampleDependencyTypeEntry {
    isLeading: IsLeading;
    sampleDependsOn: SampleDependsOn;
    sampleIsDependedOn: SampleIsDependedOn;
    sampleHasRedundancy: SampleHasRedundancy;
}

/** `isobmff::boxes::SampleFlags`, its fields as the tables stating them do */
export interface SampleFlags {
    sampleDependencyType: SampleDependencyTypeEntry;
    /** `isobmff::boxes::PaddingBitsEntry::pad` */
    paddingBits: number;
    sampleIsNonSyncSample: boolean;
    /** `isobmff::boxes::DegradationPriorityEntry::priority` */
    degradationPriority: number;
}
"#;

/// [`isobmff::Mp4EpochSeconds`], ISO/IEC 14496-12 §6.2.2, a time being the `BigInt` of its seconds since 1904 wherever a method takes or returns one
#[wasm_bindgen]
#[derive(Debug)]
#[non_exhaustive]
pub struct Mp4EpochSeconds {}

#[wasm_bindgen]
impl Mp4EpochSeconds {
    /// [`isobmff::Mp4EpochSeconds::from_unix_seconds`], the seconds since 1904, or none where they do not fit in 64 bits
    #[wasm_bindgen(js_name = fromUnixSeconds)]
    pub fn from_unix_seconds(seconds: u64) -> Option<u64> {
        isobmff::Mp4EpochSeconds::from_unix_seconds(seconds).map(isobmff::Mp4EpochSeconds::seconds)
    }
}

/// [`isobmff::boxes::FileTypeBox`] (`ftyp`), ISO/IEC 14496-12 §4.3, a root of its own
#[doc(alias = "ftyp")]
#[wasm_bindgen]
#[derive(Debug)]
pub struct FileTypeBox(pub(crate) boxes::FileTypeBox);

#[wasm_bindgen]
impl FileTypeBox {
    /// [`isobmff::boxes::FileTypeBox::forbids_default_base_is_moof`]
    #[wasm_bindgen(js_name = forbidsDefaultBaseIsMoof)]
    pub fn forbids_default_base_is_moof(&self) -> bool {
        self.0.forbids_default_base_is_moof()
    }
}

/// A box of type `T` in the tree a root holds, reached from the root again on every call
///
/// The root is the box itself, or the box a path of the library's `_mut`
/// accessors leads down from; handles to one root share it.
struct Place<T>(Rc<Visit<T>>);

/// Hands the box a [`Place`] reaches to the visitor, or fails where the path leads to no box
type Visit<T> = dyn Fn(&mut dyn FnMut(&mut T)) -> Result<(), Failure>;

impl<T: 'static> Place<T> {
    /// Makes `value` a root of its own
    fn root(value: T) -> Self {
        Self::shared(Rc::new(RefCell::new(value)))
    }

    /// Returns the place of the box `root` holds, a root its holder reads too
    fn shared(root: Rc<RefCell<T>>) -> Self {
        Self(Rc::new(move |visit| {
            visit(&mut root.borrow_mut());
            Ok(())
        }))
    }

    /// Returns the place `step` leads to from this one
    fn child<C: 'static>(&self, step: impl Fn(&mut T) -> Option<&mut C> + 'static) -> Place<C> {
        let parent = Rc::clone(&self.0);
        Place(Rc::new(move |visit| {
            let mut reached = false;
            parent(&mut |value| {
                if let Some(child) = step(value) {
                    reached = true;
                    visit(child);
                }
            })?;
            if reached {
                Ok(())
            } else {
                Err(Failure::handle())
            }
        }))
    }

    /// Returns the place `step` leads to from this one, for a box the library lends no `_mut` path to
    ///
    /// Each call hands over a copy of the box, so a change made through the
    /// place is lost.
    fn child_copy<C: Clone + 'static>(
        &self,
        step: impl Fn(&T) -> Option<&C> + 'static,
    ) -> Place<C> {
        let parent = self.clone();
        Place(Rc::new(move |visit| {
            let mut copy = parent
                .read(|value| step(value).cloned())?
                .ok_or_else(Failure::handle)?;
            visit(&mut copy);
            Ok(())
        }))
    }

    /// Returns what `read` makes of the box
    ///
    /// # Errors
    ///
    /// `"handle"`: the path leads to no box.
    fn read<R>(&self, read: impl FnOnce(&T) -> R) -> Result<R, Failure> {
        let mut made = None;
        self.write(|value| made = Some(read(value)))?;
        made.ok_or_else(Failure::handle)
    }

    /// Changes the box by `write`
    ///
    /// # Errors
    ///
    /// `"handle"`: the path leads to no box.
    fn write(&self, write: impl FnOnce(&mut T)) -> Result<(), Failure> {
        let mut write = Some(write);
        (self.0)(&mut |value| {
            if let Some(write) = write.take() {
                write(value);
            }
        })
    }
}

impl<T> Clone for Place<T> {
    fn clone(&self) -> Self {
        Self(Rc::clone(&self.0))
    }
}

impl<T> fmt::Debug for Place<T> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("Place")
    }
}
