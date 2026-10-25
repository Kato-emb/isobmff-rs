//! What the tests of the workspace check reading and writing against: MPEG's `FileFormatConformance` files, GPAC's reports of them, and FFmpeg's `ffprobe`
//!
//! `scripts/fetch-conformance.sh` fetches the files into a directory, each with
//! the box structure GPAC reports for it, and `ISOBMFF_CONFORMANCE_DIR` names
//! that directory to [`conformance_files`]. `ISOBMFF_FFPROBE` names `ffprobe`
//! to [`ffprobe`] where it is not on the path.
//!
//! The functions panic on what they cannot read: they serve tests, which fail
//! on a panic.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::{env, fs};

pub use serde_json;
use serde_json::Value;

/// Conformance file, with the box structure GPAC reports for it
#[non_exhaustive]
#[derive(Debug)]
pub struct ConformanceFile {
    /// File name, such as `01_simple.mp4`
    pub name: String,
    /// Contents of the file
    pub bytes: Vec<u8>,
    /// GPAC's report of the file, which `IsoMediaFile` opens
    pub report: Value,
}

/// Returns the conformance files in the directory `ISOBMFF_CONFORMANCE_DIR` names, in the order of their names
///
/// # Panics
///
/// * `ISOBMFF_CONFORMANCE_DIR` is not set, or names a directory that holds no
///   conformance file.
/// * A file, or the report of it, does not read.
#[must_use]
#[expect(
    clippy::expect_used,
    clippy::unwrap_used,
    reason = "a test reading files that are not there fails"
)]
pub fn conformance_files() -> Vec<ConformanceFile> {
    let directory = PathBuf::from(env::var_os("ISOBMFF_CONFORMANCE_DIR").expect(
        "ISOBMFF_CONFORMANCE_DIR names the directory scripts/fetch-conformance.sh fetched into",
    ));
    let mut files = fs::read_dir(&directory)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| path.extension().is_some_and(|extension| extension == "mp4"))
        .collect::<Vec<_>>();
    files.sort();
    assert!(
        !files.is_empty(),
        "{} holds no conformance file",
        directory.display()
    );

    files
        .into_iter()
        .map(|file| {
            let stem = file.file_stem().unwrap().to_str().unwrap();
            let report = serde_json::from_slice(
                &fs::read(file.with_file_name(format!("{stem}_gpac.json"))).unwrap(),
            )
            .unwrap();

            ConformanceFile {
                name: file.file_name().unwrap().to_str().unwrap().to_owned(),
                bytes: fs::read(&file).unwrap(),
                report,
            }
        })
        .collect()
}

/// Returns a four-character code as GPAC spells it: printable ASCII as it is, any other byte in hexadecimal
#[must_use]
pub fn gpac_type(four_cc: &[u8; 4]) -> String {
    four_cc
        .iter()
        .map(|byte| {
            if byte.is_ascii_graphic() || *byte == b' ' {
                char::from(*byte).to_string()
            } else {
                format!("{byte:02X}")
            }
        })
        .collect()
}

/// Returns the boxes GPAC reports directly inside `node`, in the order of the names GPAC gives their classes
///
/// GPAC reports the boxes of one class as an array under the class name, so
/// boxes of different classes do not keep the order they lie in.
#[must_use]
pub fn gpac_children(node: &Value) -> Vec<&Value> {
    node.as_object()
        .into_iter()
        .flatten()
        .filter(|(key, _)| !key.starts_with('@'))
        .flat_map(|(_, value)| {
            if let Value::Array(items) = value {
                items.iter().collect()
            } else {
                vec![value]
            }
        })
        .filter(|item| item.get("@Type").is_some() && item.get("@Size").is_some())
        .collect()
}

/// Returns the type and total GPAC reports for a box, as GPAC spells them
#[must_use]
pub fn gpac_frame(node: &Value) -> (String, u64) {
    let box_type = node
        .get("@Type")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_owned();
    let size = node
        .get("@Size")
        .and_then(Value::as_str)
        .and_then(|size| size.parse().ok())
        .unwrap_or_default();

    (box_type, size)
}

/// Runs `ffprobe` with `arguments` on `file`
///
/// # Panics
///
/// * `ffprobe` does not run.
#[must_use]
#[expect(
    clippy::unwrap_used,
    reason = "a test checking against an ffprobe that does not run fails"
)]
pub fn ffprobe(arguments: &[&str], file: &Path) -> Output {
    let ffprobe = env::var_os("ISOBMFF_FFPROBE").unwrap_or_else(|| "ffprobe".into());

    Command::new(ffprobe)
        .args(arguments)
        .arg(file)
        .output()
        .unwrap()
}
