//! The fixtures under `crates/isobmff/tests/fixtures/`, each written by FFmpeg

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
