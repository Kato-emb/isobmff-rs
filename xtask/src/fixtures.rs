//! The fixtures under `crates/isobmff/tests/fixtures/`, each written by FFmpeg, or by FFmpeg and then a value of one of its boxes changed

use std::fmt::Write as _;
use std::io::{self, ErrorKind};
use std::path::Path;
use std::process::{self, Command, ExitCode};
use std::{env, fs};

/// Where the fixtures are written, relative to the root of the repository
const DIRECTORY: &str = "crates/isobmff/tests/fixtures";

/// What FFmpeg is told before anything else: write over its outputs, and say nothing but errors
const FFMPEG: &[&str] = &["-nostdin", "-hide_banner", "-loglevel", "error", "-y"];

/// The synthetic video and audio every fixture is encoded from, 1.2 s of each
const INPUT: &[&str] = &[
    "-f",
    "lavfi",
    "-i",
    "testsrc=size=160x120:rate=10:duration=1.2",
    "-f",
    "lavfi",
    "-i",
    "sine=frequency=440:sample_rate=48000:duration=1.2",
];

/// H.264 with a sync sample every fourth sample, and mono AAC; bitexact, so
/// that one version of FFmpeg writes the same bytes on every run
const ENCODING: &[&str] = &[
    "-c:v",
    "libx264",
    "-preset",
    "ultrafast",
    "-threads",
    "1",
    "-x264-params",
    "keyint=4:min-keyint=4:scenecut=0",
    "-pix_fmt",
    "yuv420p",
    "-c:a",
    "aac",
    "-b:a",
    "32k",
    "-ac",
    "1",
    "-fflags",
    "+bitexact",
    "-flags",
    "+bitexact",
    "-map_metadata",
    "-1",
];

/// A fixture, and the command that writes it
struct Fixture {
    name: &'static str,
    holds: &'static str,
    program: &'static str,
    /// The arguments of [`Fixture::program`], laid end to end
    arguments: &'static [&'static [&'static str]],
    /// The files the program writes, laid end to end into the fixture
    outputs: &'static [&'static str],
}

const FIXTURES: &[Fixture] = &[
    Fixture {
        name: "movie_moov_last.mp4",
        holds: "a movie file with its moov after the mdat",
        program: "ffmpeg",
        arguments: &[FFMPEG, INPUT, ENCODING, &["-f", "mp4", "out.mp4"]],
        outputs: &["out.mp4"],
    },
    Fixture {
        name: "movie_moov_first.mp4",
        holds: "a movie file with its moov before the mdat",
        program: "ffmpeg",
        arguments: &[
            FFMPEG,
            INPUT,
            ENCODING,
            &["-movflags", "+faststart", "-f", "mp4", "out.mp4"],
        ],
        outputs: &["out.mp4"],
    },
    Fixture {
        name: "fragmented.mp4",
        holds: "a fragmented movie file: a moov with no samples, then three moof and mdat pairs",
        program: "ffmpeg",
        arguments: &[
            FFMPEG,
            INPUT,
            ENCODING,
            &[
                "-movflags",
                "+frag_keyframe+empty_moov+default_base_moof",
                "-f",
                "mp4",
                "out.mp4",
            ],
        ],
        outputs: &["out.mp4"],
    },
    Fixture {
        name: "segment.mp4",
        holds: "an initialization segment (ftyp, moov), then three media segments (styp, moof, mdat) with no sidx",
        program: "ffmpeg",
        arguments: &[
            FFMPEG,
            INPUT,
            ENCODING,
            &[
                "-f",
                "hls",
                "-hls_segment_type",
                "fmp4",
                "-hls_time",
                "0.4",
                "-hls_list_size",
                "0",
                "-hls_segment_options",
                "movflags=+skip_sidx",
                "-hls_fmp4_init_filename",
                "init.mp4",
                "-hls_segment_filename",
                "seg%d.m4s",
                "index.m3u8",
            ],
        ],
        outputs: &["init.mp4", "seg0.m4s", "seg1.m4s", "seg2.m4s"],
    },
];

/// A field of a box of a fixture, and the value it is changed to
struct Change {
    /// The box, by the types of the boxes that hold it and the index, from 0,
    /// of each among the boxes of its type in the box that holds it
    path: &'static [(&'static [u8; 4], usize)],
    /// Where the field lies, in bytes past the header of the box
    at: usize,
    value: Value,
}

/// What a field becomes
enum Value {
    /// A 32-bit field set to this
    Set32(u32),
    /// A 64-bit field this much greater
    Add64(u64),
}

/// A fixture that is another fixture with fields of its boxes changed, as a file the specification does not allow and FFmpeg reads
struct Changed {
    name: &'static str,
    holds: &'static str,
    from: &'static str,
    changes: &'static [Change],
}

/// The `stbl` of track `track`, from 0, of a movie file
const fn sample_table(track: usize, table: &'static [u8; 4]) -> [(&'static [u8; 4], usize); 6] {
    [
        (b"moov", 0),
        (b"trak", track),
        (b"mdia", 0),
        (b"minf", 0),
        (b"stbl", 0),
        (table, 0),
    ]
}

const CHANGED: &[Changed] = &[
    Changed {
        name: "lenient_tables.mp4",
        holds: "movie_moov_last.mp4 whose video time-to-sample table counts 10 of its 12 samples and whose sync sample table names sample 99 of 12, and whose audio chunks place 55 of its 58 samples",
        from: "movie_moov_last.mp4",
        changes: &[
            Change {
                path: &sample_table(0, b"stts"),
                at: 8,
                value: Value::Set32(10),
            },
            Change {
                path: &sample_table(0, b"stss"),
                at: 16,
                value: Value::Set32(99),
            },
            Change {
                path: &sample_table(1, b"stsc"),
                at: 24,
                value: Value::Set32(4),
            },
        ],
    },
    Changed {
        name: "lenient_chunks.mp4",
        holds: "movie_moov_last.mp4 whose video chunks place 2 samples each, 24 of the 12 its sample size table counts",
        from: "movie_moov_last.mp4",
        changes: &[Change {
            path: &sample_table(0, b"stsc"),
            at: 12,
            value: Value::Set32(2),
        }],
    },
    Changed {
        name: "lenient_tfdt.mp4",
        holds: "fragmented.mp4 whose second movie fragment gives its video a tfdt (version 1) 100 past the durations of the samples before it",
        from: "fragmented.mp4",
        changes: &[Change {
            path: &[(b"moof", 1), (b"traf", 0), (b"tfdt", 0)],
            at: 4,
            value: Value::Add64(100),
        }],
    },
];

/// Where the box `path` names starts in `file`, and where it ends, or `None` where `file` has no such box
fn find(file: &[u8], path: &[(&[u8; 4], usize)]) -> Option<(usize, usize)> {
    let mut within = (0_usize, file.len());
    let mut found = None;
    for &(box_type, index) in path {
        let (mut offset, end) = within;
        let mut seen = 0_usize;
        let (start, box_end) = loop {
            let header = file.get(offset..offset.checked_add(8)?)?;
            let size =
                usize::try_from(u32::from_be_bytes(header.get(..4)?.try_into().ok()?)).ok()?;
            let box_end = offset
                .checked_add(size)
                .filter(|&box_end| size >= 8 && box_end <= end)?;
            if header.get(4..8)? == box_type.as_slice() {
                if seen == index {
                    break (offset, box_end);
                }
                seen = seen.checked_add(1)?;
            }
            offset = box_end;
        };
        within = (start.checked_add(8)?, box_end);
        found = Some((start, box_end));
    }
    found
}

/// Writes `changed` into `directory`, from the fixture it is made from there
fn write_changed(changed: &Changed, directory: &Path) -> io::Result<()> {
    let mut file = fs::read(directory.join(changed.from))?;
    for change in changed.changes {
        let missing = || {
            io::Error::other(format!(
                "{} has no field {} past the header of {:?}",
                changed.from, change.at, change.path
            ))
        };
        let (offset, _) = find(&file, change.path).ok_or_else(missing)?;
        let at = offset
            .checked_add(8)
            .and_then(|body| body.checked_add(change.at))
            .ok_or_else(missing)?;
        match change.value {
            Value::Set32(value) => file
                .get_mut(at..at.checked_add(4).ok_or_else(missing)?)
                .ok_or_else(missing)?
                .copy_from_slice(&value.to_be_bytes()),
            Value::Add64(more) => {
                let field = file
                    .get_mut(at..at.checked_add(8).ok_or_else(missing)?)
                    .ok_or_else(missing)?;
                let value = u64::from_be_bytes(field.try_into().map_err(|_| missing())?)
                    .checked_add(more)
                    .ok_or_else(missing)?;
                field.copy_from_slice(&value.to_be_bytes());
            }
        }
    }
    fs::write(directory.join(changed.name), file)
}

/// What became of a fixture
enum Outcome {
    Written,
    ProgramNotFound,
}

/// What `cargo xtask fixtures --help` prints: each fixture, what it holds and how it was made
pub(crate) fn help() -> String {
    let mut text = format!(
        "Usage: cargo xtask fixtures\n\n\
         Regenerates {DIRECTORY}/ with the programs on PATH. The committed bytes \
         were written by FFmpeg 9.0.1 (libx264, aac); another version may encode \
         other bytes. A fixture whose program is not on PATH is skipped.\n"
    );
    for fixture in FIXTURES {
        let _ = write!(
            text,
            "\n{}\n  {}\n  {} {}\n",
            fixture.name,
            fixture.holds,
            fixture.program,
            fixture.arguments.concat().join(" ")
        );
        if fixture.outputs.len() > 1 {
            let _ = writeln!(
                text,
                "  then {} laid end to end",
                fixture.outputs.join(", ")
            );
        }
    }
    for changed in CHANGED {
        let _ = write!(
            text,
            "\n{}\n  {}\n  {} with {} field(s) of its boxes changed\n",
            changed.name,
            changed.holds,
            changed.from,
            changed.changes.len()
        );
    }
    text
}

/// Writes every fixture into [`DIRECTORY`], skipping one whose program is not on PATH
pub(crate) fn generate() -> ExitCode {
    let directory = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join(DIRECTORY);
    for fixture in FIXTURES {
        match write(fixture, &directory) {
            Ok(Outcome::Written) => println!("wrote {DIRECTORY}/{}", fixture.name),
            Ok(Outcome::ProgramNotFound) => {
                eprintln!(
                    "skipped {}: {} is not on PATH",
                    fixture.name, fixture.program
                );
            }
            Err(error) => {
                eprintln!("{}: {error}", fixture.name);
                return ExitCode::FAILURE;
            }
        }
    }
    for changed in CHANGED {
        if let Err(error) = write_changed(changed, &directory) {
            eprintln!("{}: {error}", changed.name);
            return ExitCode::FAILURE;
        }
        println!("wrote {DIRECTORY}/{}", changed.name);
    }
    ExitCode::SUCCESS
}

fn write(fixture: &Fixture, directory: &Path) -> io::Result<Outcome> {
    let work = env::temp_dir().join(format!("isobmff-xtask-{}-{}", process::id(), fixture.name));
    fs::create_dir_all(&work)?;
    let written = (|| {
        let status = match Command::new(fixture.program)
            .args(fixture.arguments.concat())
            .current_dir(&work)
            .status()
        {
            Err(error) if error.kind() == ErrorKind::NotFound => {
                return Ok(Outcome::ProgramNotFound);
            }
            status => status?,
        };
        if !status.success() {
            return Err(io::Error::other(format!(
                "{} exited with {status}",
                fixture.program
            )));
        }
        let outputs = fixture
            .outputs
            .iter()
            .map(|output| fs::read(work.join(output)))
            .collect::<io::Result<Vec<_>>>()?;
        fs::create_dir_all(directory)?;
        fs::write(directory.join(fixture.name), outputs.concat())?;
        Ok(Outcome::Written)
    })();
    fs::remove_dir_all(&work)?;
    written
}
