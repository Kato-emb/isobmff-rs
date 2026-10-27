//! The user data of movies and tracks, read as GPAC reports it

use std::iter;

use isobmff_boxes::{
    CopyrightBox, HintBytesSent64Box, HintBytesSentBox, HintImmediateBytesSentBox,
    HintInformationBox, HintLargestPacketBox, HintLongestPacketBox, HintMaxRelativeTimeBox,
    HintMediaBytesSentBox, HintMinRelativeTimeBox, HintPacketsSent64Box, HintPacketsSentBox,
    HintPayloadBytesSent64Box, HintPayloadBytesSentBox, HintPayloadIdBox, HintRepeatedBytesSentBox,
    HintStatisticsBox, KindBox, LanguageCode, MovieBox, UserDataBox,
};
use isobmff_core::{BoxEncode, Utf8String, UtfString};
use isobmff_test_support::serde_json::Value;
use isobmff_test_support::{ConformanceFile, conformance_files};

use crate::tests::{
    gpac_boxes, gpac_four_cc, gpac_integer, gpac_str, gpac_top_level, read_all, top_level,
};

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

/// Returns the `udta`s of the movies of `bytes` and of their tracks, the movie's first
fn user_data_boxes(bytes: &[u8]) -> Vec<UserDataBox> {
    read_all::<MovieBox>(&top_level(bytes))
        .into_iter()
        .flat_map(|movie| {
            movie
                .udta
                .into_iter()
                .chain(movie.trak.into_iter().flat_map(|track| track.udta))
        })
        .collect()
}

/// Returns the `udta`s GPAC reports in `report` for the movies and their tracks, in the order of [`user_data_boxes`]
fn gpac_user_data_boxes(report: &Value) -> Vec<&Value> {
    gpac_top_level(report, "moov")
        .into_iter()
        .flat_map(|movie| {
            gpac_boxes(movie, "udta").into_iter().chain(
                gpac_boxes(movie, "trak")
                    .into_iter()
                    .flat_map(|track| gpac_boxes(track, "udta")),
            )
        })
        .collect()
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
        let ours: Vec<UserData> = user_data_boxes(&bytes)
            .into_iter()
            .map(|user_data| (user_data.cprt, user_data.kind))
            .collect();
        let theirs: Vec<UserData> = gpac_user_data_boxes(&report)
            .into_iter()
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

/// Returns the box GPAC reports directly inside `node` of the four-character code `box_type`, read into `Box` by `read`, or [`None`] where it reports none
fn gpac_statistic<Box: Default>(
    node: &Value,
    box_type: &str,
    read: impl FnOnce(&mut Box, &Value),
) -> Option<Box> {
    gpac_boxes(node, box_type).first().map(|reported| {
        let mut statistic = Box::default();
        read(&mut statistic, reported);
        statistic
    })
}

/// Returns `hinf` with the statistics GPAC reports for it set as GPAC reports them
///
/// GPAC reports no field of `maxr`, which is kept as read.
fn gpac_hint_statistics(ours: &HintStatisticsBox, node: &Value) -> HintStatisticsBox {
    let mut statistics = ours.clone();
    statistics.trpy = gpac_statistic(node, "trpy", |trpy: &mut HintBytesSent64Box, node| {
        trpy.bytes_sent = gpac_integer(node, "@RTPBytesSent");
    });
    statistics.nump = gpac_statistic(node, "nump", |nump: &mut HintPacketsSent64Box, node| {
        nump.packets_sent = gpac_integer(node, "@PacketsSent");
    });
    statistics.tpyl = gpac_statistic(
        node,
        "tpyl",
        |tpyl: &mut HintPayloadBytesSent64Box, node| {
            tpyl.bytes_sent = gpac_integer(node, "@BytesSent");
        },
    );
    statistics.totl = gpac_statistic(node, "totl", |totl: &mut HintBytesSentBox, node| {
        totl.bytes_sent = gpac_integer(node, "@RTPBytesSent");
    });
    statistics.npck = gpac_statistic(node, "npck", |npck: &mut HintPacketsSentBox, node| {
        npck.packets_sent = gpac_integer(node, "@packetsSent");
    });
    statistics.tpay = gpac_statistic(node, "tpay", |tpay: &mut HintPayloadBytesSentBox, node| {
        tpay.bytes_sent = gpac_integer(node, "@BytesSent");
    });
    statistics.dmed = gpac_statistic(node, "dmed", |dmed: &mut HintMediaBytesSentBox, node| {
        dmed.bytes_sent = gpac_integer(node, "@BytesSent");
    });
    statistics.dimm = gpac_statistic(
        node,
        "dimm",
        |dimm: &mut HintImmediateBytesSentBox, node| {
            dimm.bytes_sent = gpac_integer(node, "@BytesSent");
        },
    );
    statistics.drep = gpac_statistic(node, "drep", |drep: &mut HintRepeatedBytesSentBox, node| {
        drep.bytes_sent = gpac_integer(node, "@RepeatedBytes");
    });
    statistics.tmin = gpac_statistic(node, "tmin", |tmin: &mut HintMinRelativeTimeBox, node| {
        tmin.time = gpac_integer(node, "@MinimumTransmitTime");
    });
    statistics.tmax = gpac_statistic(node, "tmax", |tmax: &mut HintMaxRelativeTimeBox, node| {
        tmax.time = gpac_integer(node, "@MaximumTransmitTime");
    });
    statistics.pmax = gpac_statistic(node, "pmax", |pmax: &mut HintLargestPacketBox, node| {
        pmax.bytes = gpac_integer(node, "@MaximumSize");
    });
    statistics.dmax = gpac_statistic(node, "dmax", |dmax: &mut HintLongestPacketBox, node| {
        dmax.time = gpac_integer(node, "@MaximumDuration");
    });
    statistics.payt = gpac_boxes(node, "payt")
        .into_iter()
        .map(|node| {
            let mut payload_id = HintPayloadIdBox::default();
            payload_id.payload_id = gpac_integer(node, "@PayloadID");
            payload_id.rtpmap_string = gpac_str(node, "@PayloadString").as_bytes().to_vec();
            payload_id
        })
        .collect();
    statistics
}

/// Returns `hnti` with the description format GPAC reports for its `rtp ` set as GPAC reports it
///
/// GPAC reports no text of an SDP, only the size of its box, which
/// [`hint_information_sizes`] holds against.
fn gpac_hint_information(ours: &HintInformationBox, node: &Value) -> HintInformationBox {
    let mut information = ours.clone();
    if let (Some(rtp), Some(reported)) = (&mut information.rtp, gpac_boxes(node, "rtp ").first()) {
        rtp.description_format = gpac_four_cc(reported, "@descriptionformat");
    }
    information
}

/// Returns the sizes of the `rtp ` and `sdp ` boxes of a `hnti`, as written and as GPAC reports them in `node`
fn hint_information_sizes(
    ours: &HintInformationBox,
    node: &Value,
) -> [(Option<u64>, Option<u64>); 2] {
    [
        (
            ours.rtp.as_ref().map(BoxEncode::encoded_len),
            gpac_boxes(node, "rtp ")
                .first()
                .map(|rtp| gpac_integer(rtp, "@Size")),
        ),
        (
            ours.sdp.as_ref().map(BoxEncode::encoded_len),
            gpac_boxes(node, "sdp ")
                .first()
                .map(|sdp| gpac_integer(sdp, "@Size")),
        ),
    ]
}

#[test]
#[ignore = "reads the conformance files ISOBMFF_CONFORMANCE_DIR names"]
fn the_hint_information_and_statistics_of_a_movie_and_its_tracks_read_as_gpac_reports_them() {
    let mut differences = Vec::new();
    let mut compared = 0;
    for ConformanceFile {
        name,
        bytes,
        report,
        ..
    } in conformance_files()
    {
        let ours = user_data_boxes(&bytes);
        let reported = gpac_user_data_boxes(&report);
        if ours.len() != reported.len() {
            differences.push(format!(
                "{name}: {} udta, GPAC reports {}",
                ours.len(),
                reported.len()
            ));
            continue;
        }
        for (user_data, node) in ours.iter().zip(reported) {
            let information = gpac_boxes(node, "hnti").first().copied();
            let statistics = gpac_boxes(node, "hinf").first().copied();
            if information.is_some() != user_data.hnti.is_some()
                || statistics.is_some() != user_data.hinf.is_some()
            {
                differences.push(format!("{name}: read {user_data:?}, GPAC reports {node}"));
                continue;
            }
            if let (Some(ours), Some(node)) = (&user_data.hnti, information) {
                compared += 1;
                let theirs = gpac_hint_information(ours, node);
                let sizes = hint_information_sizes(ours, node);
                if *ours != theirs || sizes.iter().any(|(ours, theirs)| ours != theirs) {
                    differences.push(format!(
                        "{name}: read {ours:?}, GPAC reports {theirs:?} of the sizes {sizes:?}"
                    ));
                }
            }
            if let (Some(ours), Some(node)) = (&user_data.hinf, statistics) {
                compared += 1;
                let theirs = gpac_hint_statistics(ours, node);
                if *ours != theirs {
                    differences.push(format!("{name}: read {ours:?}, GPAC reports {theirs:?}"));
                }
            }
        }
    }

    assert_eq!((differences, compared > 0), (Vec::<String>::new(), true));
}
