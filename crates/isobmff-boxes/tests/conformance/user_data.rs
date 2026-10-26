//! The user data of movies and tracks, read as GPAC reports it

use std::iter;

use isobmff_boxes::{CopyrightBox, KindBox, LanguageCode, MovieBox};
use isobmff_core::{Utf8String, UtfString};
use isobmff_test_support::serde_json::Value;
use isobmff_test_support::{ConformanceFile, conformance_files};

use crate::tests::{gpac_boxes, gpac_str, gpac_top_level, read_all, top_level};

/// Returns `cprt` with the fields GPAC reports for it set as GPAC reports them
fn gpac_copyright(ours: &CopyrightBox, node: &Value) -> CopyrightBox {
    let notice = gpac_str(node, "@CopyrightNotice");
    let mut copyright = ours.clone();
    copyright.language = LanguageCode::new(
        gpac_str(node, "@LanguageCode")
            .as_bytes()
            .try_into()
            .unwrap(),
    )
    .unwrap();
    if ours.notice.to_str().as_deref() != Some(notice) {
        copyright.notice = UtfString::new(notice.to_owned()).unwrap();
    }
    copyright
}

/// Returns `kind` with the fields GPAC reports for it set as GPAC reports them
///
/// GPAC reports an absent `value` as the empty string.
fn gpac_kind(ours: &KindBox, node: &Value) -> KindBox {
    let value = gpac_str(node, "@value");
    let mut kind = ours.clone();
    kind.scheme_uri = Utf8String::from_slice(gpac_str(node, "@schemeURI").as_bytes());
    if ours
        .value
        .as_ref()
        .map_or(b"".as_slice(), Utf8String::as_bytes)
        != value.as_bytes()
    {
        kind.value = Some(Utf8String::from_slice(value.as_bytes()));
    }
    kind
}

/// Copyright notices and kinds a `udta` holds
type UserData = (Vec<CopyrightBox>, Vec<KindBox>);

#[test]
#[ignore = "reads the conformance files ISOBMFF_CONFORMANCE_DIR names"]
fn the_user_data_of_a_movie_and_its_tracks_reads_as_gpac_reports_it() {
    let none: UserData = (Vec::new(), Vec::new());
    let mut differences = Vec::new();
    for ConformanceFile {
        name,
        bytes,
        report,
        ..
    } in conformance_files()
    {
        let ours: Vec<UserData> = read_all::<MovieBox>(&top_level(&bytes))
            .into_iter()
            .flat_map(|movie| {
                movie
                    .udta
                    .into_iter()
                    .chain(movie.trak.into_iter().flat_map(|track| track.udta))
            })
            .map(|user_data| (user_data.cprt, user_data.kind))
            .collect();
        let theirs: Vec<UserData> = gpac_top_level(&report, "moov")
            .into_iter()
            .flat_map(|movie| {
                gpac_boxes(movie, "udta").into_iter().chain(
                    gpac_boxes(movie, "trak")
                        .into_iter()
                        .flat_map(|track| gpac_boxes(track, "udta")),
                )
            })
            .zip(ours.iter().chain(iter::repeat(&none)))
            .map(|(user_data, (copyrights, kinds))| {
                (
                    gpac_boxes(user_data, "cprt")
                        .into_iter()
                        .zip(
                            copyrights
                                .iter()
                                .chain(iter::repeat(&CopyrightBox::default())),
                        )
                        .map(|(node, ours)| gpac_copyright(ours, node))
                        .collect(),
                    gpac_boxes(user_data, "kind")
                        .into_iter()
                        .zip(kinds.iter().chain(iter::repeat(&KindBox::default())))
                        .map(|(node, ours)| gpac_kind(ours, node))
                        .collect(),
                )
            })
            .collect();
        if ours != theirs {
            differences.push(format!("{name}: read {ours:?}, GPAC reports {theirs:?}"));
        }
    }

    assert_eq!(differences, Vec::<String>::new());
}
