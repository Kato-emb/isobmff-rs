//! The items of meta boxes, read as GPAC reports them

use isobmff_boxes::{
    ItemLocation, ItemLocationBox, ItemLocationExtent, ItemType, MetaBox, MovieBox,
};
use isobmff_test_support::serde_json::Value;
use isobmff_test_support::{ConformanceFile, conformance_files};

use crate::tests::{
    gpac_boxes, gpac_elements, gpac_integer, gpac_str, gpac_top_level, read_all, top_level,
};

/// Returns `iloc` as GPAC reports it in `node`
fn gpac_item_location(node: &Value) -> ItemLocationBox {
    let mut item_location = ItemLocationBox::default();
    item_location.version = gpac_integer(node, "@Version");
    item_location.offset_size = gpac_integer(node, "@offset_size");
    item_location.length_size = gpac_integer(node, "@length_size");
    item_location.base_offset_size = gpac_integer(node, "@base_offset_size");
    item_location.index_size = gpac_integer(node, "@index_size");
    item_location.items = gpac_elements(node, "ItemLocationEntry")
        .into_iter()
        .map(|reported| {
            let mut item = ItemLocation::default();
            item.item_id = gpac_integer(reported, "@item_ID");
            item.construction_method = gpac_integer(reported, "@construction_method");
            item.data_reference_index = gpac_integer(reported, "@data_reference_index");
            item.base_offset = gpac_integer(reported, "@base_offset");
            item.extents = gpac_elements(reported, "ItemExtentEntry")
                .into_iter()
                .map(|reported| {
                    let mut extent = ItemLocationExtent::default();
                    extent.item_reference_index = gpac_integer(reported, "@extent_index");
                    extent.extent_offset = gpac_integer(reported, "@extent_offset");
                    extent.extent_length = gpac_integer(reported, "@extent_length");
                    extent
                })
                .collect();
            item
        })
        .collect();
    item_location
}

#[test]
#[ignore = "reads the conformance files ISOBMFF_CONFORMANCE_DIR names"]
fn the_item_locations_of_the_file_movie_and_tracks_read_as_gpac_reports_them() {
    let mut differences = Vec::new();
    for ConformanceFile {
        name,
        bytes,
        report,
        ..
    } in conformance_files()
    {
        let boxes = top_level(&bytes);
        let movies = read_all::<MovieBox>(&boxes);
        let metas = read_all::<MetaBox>(&boxes);
        let ours: Vec<Option<ItemLocationBox>> = metas
            .iter()
            .chain(movies.iter().filter_map(|movie| movie.meta.as_ref()))
            .chain(
                movies
                    .iter()
                    .flat_map(|movie| &movie.trak)
                    .filter_map(|track| track.meta.as_ref()),
            )
            .map(|meta| meta.iloc.clone())
            .collect();
        let gpac_movies = gpac_top_level(&report, "moov");
        let theirs: Vec<Option<ItemLocationBox>> = gpac_top_level(&report, "meta")
            .into_iter()
            .chain(
                gpac_movies
                    .iter()
                    .flat_map(|movie| gpac_boxes(movie, "meta")),
            )
            .chain(
                gpac_movies
                    .iter()
                    .flat_map(|movie| gpac_boxes(movie, "trak"))
                    .flat_map(|track| gpac_boxes(track, "meta")),
            )
            .map(|meta| {
                gpac_boxes(meta, "iloc")
                    .first()
                    .copied()
                    .map(gpac_item_location)
            })
            .collect();
        if ours != theirs {
            differences.push(format!("{name}: read {ours:?}, GPAC reports {theirs:?}"));
        }
    }

    assert_eq!(differences, Vec::<String>::new());
}

/// Returns the `item_ID`, `item_protection_index`, `item_name` and content type of each entry of `iinf`, as GPAC reports them in `node`
///
/// GPAC reports an empty string as `(null)`.
fn gpac_item_infos(node: &Value) -> Vec<(u32, u16, String, String)> {
    let text = |entry: &Value, field: &str| match gpac_str(entry, field) {
        "(null)" => String::new(),
        spelled => spelled.to_owned(),
    };
    gpac_boxes(node, "infe")
        .into_iter()
        .map(|entry| {
            (
                gpac_integer(entry, "@item_ID"),
                gpac_integer(entry, "@item_protection_index"),
                text(entry, "@item_name"),
                text(entry, "@content_type"),
            )
        })
        .collect()
}

#[test]
#[ignore = "reads the conformance files ISOBMFF_CONFORMANCE_DIR names"]
fn the_item_infos_of_the_file_and_movie_read_as_gpac_reports_them() {
    let mut differences = Vec::new();
    for ConformanceFile {
        name,
        bytes,
        report,
        ..
    } in conformance_files()
    {
        let boxes = top_level(&bytes);
        let metas = read_all::<MetaBox>(&boxes);
        let movies = read_all::<MovieBox>(&boxes);
        let ours: Vec<Vec<(u32, u16, String, String)>> = metas
            .iter()
            .chain(movies.iter().filter_map(|movie| movie.meta.as_ref()))
            .filter_map(|meta| meta.iinf.as_ref())
            .map(|item_info| {
                item_info
                    .item_infos
                    .iter()
                    .map(|entry| {
                        let content_type = match &entry.item_type {
                            ItemType::ContentType { content_type, .. }
                            | ItemType::Mime { content_type, .. } => {
                                content_type.as_str().unwrap().to_owned()
                            }
                            ItemType::Uri { .. } | ItemType::Other { .. } | _ => String::new(),
                        };
                        (
                            entry.item_id,
                            entry.item_protection_index,
                            entry.item_name.as_str().unwrap().to_owned(),
                            content_type,
                        )
                    })
                    .collect()
            })
            .collect();
        let theirs: Vec<Vec<(u32, u16, String, String)>> = gpac_top_level(&report, "meta")
            .into_iter()
            .chain(
                gpac_top_level(&report, "moov")
                    .into_iter()
                    .flat_map(|movie| gpac_boxes(movie, "meta")),
            )
            .flat_map(|meta| gpac_boxes(meta, "iinf"))
            .map(gpac_item_infos)
            .collect();
        if ours != theirs {
            differences.push(format!("{name}: read {ours:?}, GPAC reports {theirs:?}"));
        }
    }

    assert_eq!(differences, Vec::<String>::new());
}
