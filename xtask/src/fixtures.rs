//! The fixtures under `crates/isobmff/tests/fixtures/`, each written by FFmpeg, or by FFmpeg and then a value of one of its boxes changed, or its boxes rewritten

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
    Fixture {
        name: "segment_index_global.mp4",
        holds: "fragmented.mp4 with a sidx for each track after the moov, indexing the three moof and mdat pairs as its subsegments",
        program: "ffmpeg",
        arguments: &[
            FFMPEG,
            INPUT,
            ENCODING,
            &[
                "-movflags",
                "+frag_keyframe+empty_moov+default_base_moof+global_sidx",
                "-f",
                "mp4",
                "out.mp4",
            ],
        ],
        outputs: &["out.mp4"],
    },
    Fixture {
        name: "segment_index_per_fragment.mp4",
        holds: "fragmented.mp4 with a sidx for each track before each moof, indexing that moof and mdat pair",
        program: "ffmpeg",
        arguments: &[
            FFMPEG,
            INPUT,
            ENCODING,
            &[
                "-movflags",
                "+frag_keyframe+empty_moov+default_base_moof+dash",
                "-f",
                "mp4",
                "out.mp4",
            ],
        ],
        outputs: &["out.mp4"],
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

/// A fixture that is another fixture with fields of its boxes changed, as a file FFmpeg does not write and reads
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

/// The `elst` of the video track of a movie file
const VIDEO_EDITS: [(&[u8; 4], usize); 4] =
    [(b"moov", 0), (b"trak", 0), (b"edts", 0), (b"elst", 0)];

/// Where the fields of the only entry of an `elst` of version 0 lie, past its header
const EDIT_DURATION: usize = 8;
const MEDIA_TIME: usize = 12;
const MEDIA_RATE: usize = 16;

const CHANGED: &[Changed] = &[
    Changed {
        name: "mismatch_tables.mp4",
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
        name: "mismatch_chunks.mp4",
        holds: "movie_moov_last.mp4 whose video chunks place 2 samples each, 24 of the 12 its sample size table counts",
        from: "movie_moov_last.mp4",
        changes: &[Change {
            path: &sample_table(0, b"stsc"),
            at: 12,
            value: Value::Set32(2),
        }],
    },
    Changed {
        name: "mismatch_tfdt.mp4",
        holds: "fragmented.mp4 whose second movie fragment gives its video a tfdt (version 1) 100 past the durations of the samples before it",
        from: "fragmented.mp4",
        changes: &[Change {
            path: &[(b"moof", 1), (b"traf", 0), (b"tfdt", 0)],
            at: 4,
            value: Value::Add64(100),
        }],
    },
    Changed {
        name: "edit_cut.mp4",
        holds: "movie_moov_last.mp4 whose video edit plays 24000 of the movie timescale (5120 of its media) from media time 1536, inside its second sample, to inside its seventh",
        from: "movie_moov_last.mp4",
        changes: &[
            Change {
                path: &VIDEO_EDITS,
                at: EDIT_DURATION,
                value: Value::Set32(24_000),
            },
            Change {
                path: &VIDEO_EDITS,
                at: MEDIA_TIME,
                value: Value::Set32(1_536),
            },
        ],
    },
    Changed {
        name: "edit_dwell.mp4",
        holds: "movie_moov_last.mp4 whose video edit is a dwell of media rate 0 holding media time 2560, inside its third sample, for 48000 of the movie timescale",
        from: "movie_moov_last.mp4",
        changes: &[
            Change {
                path: &VIDEO_EDITS,
                at: EDIT_DURATION,
                value: Value::Set32(48_000),
            },
            Change {
                path: &VIDEO_EDITS,
                at: MEDIA_TIME,
                value: Value::Set32(2_560),
            },
            Change {
                path: &VIDEO_EDITS,
                at: MEDIA_RATE,
                value: Value::Set32(0),
            },
        ],
    },
    Changed {
        name: "edit_repeat.mp4",
        holds: "movie_moov_last.mp4 whose video edit list, of RepeatEdits, plays its first 9600 of the movie timescale (2048 of its media) and repeats it for the 24000 its tkhd lasts",
        from: "movie_moov_last.mp4",
        changes: &[
            Change {
                path: &VIDEO_EDITS,
                at: 0,
                value: Value::Set32(1),
            },
            Change {
                path: &VIDEO_EDITS,
                at: EDIT_DURATION,
                value: Value::Set32(9_600),
            },
            Change {
                path: &[(b"moov", 0), (b"trak", 0), (b"tkhd", 0)],
                at: 20,
                value: Value::Set32(24_000),
            },
        ],
    },
    Changed {
        name: "chunks_out_of_order.mp4",
        holds: "movie_moov_last.mp4 whose video chunk offset table names the offsets of its second and third chunks, of one sample each, the other way round: their samples are no longer in decoding order in the file, and the bytes of each lie where the other's did, so they no longer decode as pictures, but the tables agree as the specification requires",
        from: "movie_moov_last.mp4",
        changes: &[
            Change {
                path: &sample_table(0, b"stco"),
                at: 12,
                value: Value::Set32(5_874),
            },
            Change {
                path: &sample_table(0, b"stco"),
                at: 16,
                value: Value::Set32(5_063),
            },
        ],
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
        // Why past 8 more bytes in a `dref`: its entries follow its version,
        // flags and entry_count, ISO/IEC 14496-12 §8.7.2.2.
        let fields = if box_type == b"dref" { 16 } else { 8 };
        within = (start.checked_add(fields)?, box_end);
        found = Some((start, box_end));
    }
    found
}

/// Writes `changed` into `directory`, from the fixture it is made from there
fn write_changed(changed: &Changed, directory: &Path) -> io::Result<()> {
    let mut file = fs::read(directory.join(changed.from))?;
    for change in changed.changes {
        change_field(&mut file, change.path, change.at, &change.value)?;
    }
    fs::write(directory.join(changed.name), file)
}

/// Changes the field `at` bytes past the header of the box `path` names in `file` to `value`
fn change_field(
    file: &mut [u8],
    path: &[(&[u8; 4], usize)],
    at: usize,
    value: &Value,
) -> io::Result<()> {
    let missing = || io::Error::other(format!("no field {at} past the header of {path:?}"));
    let (offset, _) = find(file, path).ok_or_else(missing)?;
    let at = offset
        .checked_add(8)
        .and_then(|body| body.checked_add(at))
        .ok_or_else(missing)?;
    match *value {
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
    Ok(())
}

/// A box of a file, by the types of the boxes that hold it and the index, from 0, of each among the boxes of its type in the box that holds it
type BoxPath = Vec<(&'static [u8; 4], usize)>;

/// A change to the boxes of a fixture
enum Edit {
    /// A 32-bit field `at` bytes past the header of a box set to a value
    Set32 {
        path: BoxPath,
        at: usize,
        value: u32,
    },
    /// The type of a box changed
    Retype { path: BoxPath, box_type: [u8; 4] },
    /// Bytes inserted at `at` past the header of a box, which it and the
    /// boxes that hold it grow by
    Insert {
        path: BoxPath,
        at: usize,
        bytes: Vec<u8>,
    },
    /// A box at file level moved to just before another
    MoveBefore { path: BoxPath, before: BoxPath },
    /// A box at file level removed
    Remove { path: BoxPath },
    /// The `size` of the last box at file level set to 0, which runs it to
    /// the end of the file, ISO/IEC 14496-12 §4.2.2
    SizeToEnd { path: BoxPath },
    /// Every `chunk_offset` of an `stco`, a file offset in the first `mdat`,
    /// counted from the payload of that `mdat`, `plus` bytes more: from the
    /// payload of the `imda` it becomes, past its 4-byte identifier, or from
    /// the first byte of a file of its payload
    CountChunkOffsetsFromMediaData { path: BoxPath, plus: u32 },
    /// The payload of a box appended to the file of media data the fixture
    /// comes with
    CopyPayload { path: BoxPath },
    /// The `tfhd` of each track fragment of each movie fragment given the
    /// `base_data_offset` where the data of its first run lies in the file of
    /// media data the payloads of the `mdat`s, in file order, make, with no
    /// `default-base-is-moof`, and its first run the `data_offset` 0,
    /// ISO/IEC 14496-12 §8.8.7.1 Table 6
    SetTrackFragmentBases,
}

/// A fixture that is another fixture with its boxes rewritten, as a file FFmpeg does not write
struct Rewritten {
    name: &'static str,
    holds: &'static str,
    from: &'static str,
    edits: fn() -> Vec<Edit>,
    /// Name of the file of media data the fixture comes with, which its
    /// `CopyPayload` edits write
    media_data: Option<&'static str>,
}

/// The flags of a `tfhd` FFmpeg writes, default sample duration, size and
/// flags, without its `default-base-is-moof`, ISO/IEC 14496-12 §8.8.7.1
const TFHD_FLAGS: u32 = 0x38;

/// Where the `data_offset` of a `trun` lies, past its header
const DATA_OFFSET: usize = 8;

/// The data reference entry of track `track`, from 0, of a movie
fn data_entry(track: usize) -> BoxPath {
    vec![
        (b"moov", 0),
        (b"trak", track),
        (b"mdia", 0),
        (b"minf", 0),
        (b"dinf", 0),
        (b"dref", 0),
        (b"url ", 0),
    ]
}

/// `fragmented.mp4` with its media data in the `imda` of the sequence number of each movie fragment, after it
///
/// The `url ` of each track becomes a `snim`, of the same length and no
/// flag; the `mdat` after each `moof` the `imda` of its `sequence_number`
/// (1, 2 and 3). Its `tfhd`s lose `default-base-is-moof`, so that §8.8.7.1
/// Table 6 counts the data of the first track fragment from the payload of
/// the `imda`, from 4, past the identifier, and that of the second from the
/// end of the first, from 0. The `mfra`, whose offsets the identifiers
/// move, becomes `free`.
fn fragments_in_identified_media_data() -> Vec<Edit> {
    let mut edits = Vec::new();
    for track in 0..2 {
        edits.push(Edit::Set32 {
            path: data_entry(track),
            at: 0,
            value: 0,
        });
        edits.push(Edit::Retype {
            path: data_entry(track),
            box_type: *b"snim",
        });
    }
    for (moof, sequence_number) in (0..3).zip(1_u8..) {
        edits.push(Edit::Retype {
            path: vec![(b"mdat", 0)],
            box_type: *b"imda",
        });
        edits.push(Edit::Insert {
            path: vec![(b"imda", moof)],
            at: 0,
            bytes: vec![0, 0, 0, sequence_number],
        });
        for (traf, data_offset) in [(0, 4), (1, 0)] {
            edits.push(Edit::Set32 {
                path: vec![(b"moof", moof), (b"traf", traf), (b"tfhd", 0)],
                at: 0,
                value: TFHD_FLAGS,
            });
            edits.push(Edit::Set32 {
                path: vec![(b"moof", moof), (b"traf", traf), (b"trun", 0)],
                at: DATA_OFFSET,
                value: data_offset,
            });
        }
    }
    edits.push(Edit::Retype {
        path: vec![(b"mfra", 0)],
        box_type: *b"free",
    });
    edits
}

/// A movie file with its `mdat` the `imda` of the identifier 1, each track naming it with an `imdt` and counting its chunk offsets from its payload
fn movie_in_identified_media_data() -> Vec<Edit> {
    // Why count the chunk offsets first: they are file offsets in the
    // `mdat`, which the `imdt`s grow the `moov` before it by.
    let mut edits: Vec<Edit> = (0..2)
        .map(|track| Edit::CountChunkOffsetsFromMediaData {
            path: sample_table(track, b"stco").to_vec(),
            plus: 4,
        })
        .collect();
    for track in 0..2 {
        edits.push(Edit::Set32 {
            path: data_entry(track),
            at: 0,
            value: 0,
        });
        edits.push(Edit::Insert {
            path: data_entry(track),
            at: 4,
            bytes: vec![0, 0, 0, 1],
        });
        edits.push(Edit::Retype {
            path: data_entry(track),
            box_type: *b"imdt",
        });
    }
    edits.push(Edit::Retype {
        path: vec![(b"mdat", 0)],
        box_type: *b"imda",
    });
    edits.push(Edit::Insert {
        path: vec![(b"imda", 0)],
        at: 0,
        bytes: vec![0, 0, 0, 1],
    });
    edits
}

/// The `url ` of track `track` of a movie naming the file `name` instead of the file itself: no flag, and a `location`, ISO/IEC 14496-12 §8.7.2.3
///
/// The `location` is `name` with the null that terminates the string.
fn data_entry_elsewhere(track: usize, name: &str) -> [Edit; 2] {
    [
        Edit::Set32 {
            path: data_entry(track),
            at: 0,
            value: 0,
        },
        Edit::Insert {
            path: data_entry(track),
            at: 4,
            bytes: [name.as_bytes(), &[0]].concat(),
        },
    ]
}

/// `movie_moov_last.mp4` with the media data of its video track in `external_movie.dat`, a copy of the payload of its `mdat`, its chunk offsets counted from the first byte of that file, §8.7.5.1
fn movie_in_another_file() -> Vec<Edit> {
    let mut edits = vec![
        Edit::CountChunkOffsetsFromMediaData {
            path: sample_table(0, b"stco").to_vec(),
            plus: 0,
        },
        Edit::CopyPayload {
            path: vec![(b"mdat", 0)],
        },
    ];
    edits.extend(data_entry_elsewhere(0, "external_movie.dat"));
    edits
}

/// `fragmented.mp4` with the media data of its movie fragments in `external_fragments.dat`, the payloads of its `mdat`s laid end to end, each track fragment based in that file, §8.8.7.1 Table 6, the `mdat`s removed and the `mfra` free space
fn fragments_in_another_file() -> Vec<Edit> {
    let mut edits = vec![Edit::SetTrackFragmentBases];
    for media_data in 0..3 {
        edits.push(Edit::CopyPayload {
            path: vec![(b"mdat", media_data)],
        });
    }
    for _ in 0..3 {
        edits.push(Edit::Remove {
            path: vec![(b"mdat", 0)],
        });
    }
    edits.push(Edit::Retype {
        path: vec![(b"mfra", 0)],
        box_type: *b"free",
    });
    for track in 0..2 {
        edits.extend(data_entry_elsewhere(track, "external_fragments.dat"));
    }
    edits
}

const REWRITTEN: &[Rewritten] = &[
    Rewritten {
        name: "imda_after_moof.mp4",
        holds: "fragmented.mp4 with the media data of each movie fragment in the imda of its sequence number, after its moof, which its tracks name with a snim and count from as Table 6 of 8.8.7.1 sets out, and the mfra free space",
        from: "fragmented.mp4",
        edits: fragments_in_identified_media_data,
        media_data: None,
    },
    Rewritten {
        name: "imda_before_moof.mp4",
        holds: "imda_after_moof.mp4 with each imda before its moof",
        from: "imda_after_moof.mp4",
        edits: || {
            (0..3)
                .map(|fragment| Edit::MoveBefore {
                    path: vec![(b"imda", fragment)],
                    before: vec![(b"moof", fragment)],
                })
                .collect()
        },
        media_data: None,
    },
    Rewritten {
        name: "imda_to_end_of_file.mp4",
        holds: "imda_after_moof.mp4 without the free space after its last imda, which runs to the end of the file",
        from: "imda_after_moof.mp4",
        edits: || {
            vec![
                Edit::Remove {
                    path: vec![(b"free", 0)],
                },
                Edit::SizeToEnd {
                    path: vec![(b"imda", 2)],
                },
            ]
        },
        media_data: None,
    },
    Rewritten {
        name: "imda_missing.mp4",
        holds: "imda_after_moof.mp4 whose second imda has the identifier 9, so that the file has no imda of the sequence number of its second movie fragment",
        from: "imda_after_moof.mp4",
        edits: || {
            vec![Edit::Set32 {
                path: vec![(b"imda", 1)],
                at: 0,
                value: 9,
            }]
        },
        media_data: None,
    },
    Rewritten {
        name: "imda_duplicate.mp4",
        holds: "imda_after_moof.mp4 whose second imda has the identifier 1 of the first, which 8.1.4.3 forbids, so that the file has no imda of the sequence number of its second movie fragment",
        from: "imda_after_moof.mp4",
        edits: || {
            vec![Edit::Set32 {
                path: vec![(b"imda", 1)],
                at: 0,
                value: 1,
            }]
        },
        media_data: None,
    },
    Rewritten {
        name: "imda_movie_moov_last.mp4",
        holds: "movie_moov_last.mp4 with its mdat the imda of the identifier 1, which each track names with an imdt and counts its chunk offsets from",
        from: "movie_moov_last.mp4",
        edits: movie_in_identified_media_data,
        media_data: None,
    },
    Rewritten {
        name: "imda_movie_moov_first.mp4",
        holds: "movie_moov_first.mp4 with its mdat the imda of the identifier 1, which each track names with an imdt and counts its chunk offsets from",
        from: "movie_moov_first.mp4",
        edits: movie_in_identified_media_data,
        media_data: None,
    },
    Rewritten {
        name: "external_movie.mp4",
        holds: "movie_moov_last.mp4 with the media data of its video track in external_movie.dat, a copy of the payload of its mdat, which its url names, its chunk offsets those of that file; its audio track in the file itself",
        from: "movie_moov_last.mp4",
        edits: movie_in_another_file,
        media_data: Some("external_movie.dat"),
    },
    Rewritten {
        name: "external_fragments.mp4",
        holds: "fragmented.mp4 with the media data of its movie fragments in external_fragments.dat, the payloads of its mdats laid end to end, which the url of each track names, each track fragment given a base_data_offset in that file as Table 6 of 8.8.7.1 sets out, its mdats removed and its mfra free space",
        from: "fragmented.mp4",
        edits: fragments_in_another_file,
        media_data: Some("external_fragments.dat"),
    },
];

/// Writes `rewritten` into `directory`, from the fixture it is made from there
fn write_rewritten(rewritten: &Rewritten, directory: &Path) -> io::Result<()> {
    let mut file = fs::read(directory.join(rewritten.from))?;
    let mut media_data = Vec::new();
    for edit in (rewritten.edits)() {
        apply(&mut file, &mut media_data, &edit)?;
    }
    if let Some(name) = rewritten.media_data {
        fs::write(directory.join(name), media_data)?;
    }
    fs::write(directory.join(rewritten.name), file)
}

/// Applies `edit` to `file`, and to the file of media data it comes with
fn apply(file: &mut Vec<u8>, media_data: &mut Vec<u8>, edit: &Edit) -> io::Result<()> {
    match edit {
        Edit::Set32 { path, at, value } => change_field(file, path, *at, &Value::Set32(*value)),
        Edit::Retype { path, box_type } => {
            let (start, _) = located(file, path)?;
            bytes_at(file, past(start, 4)?)?.copy_from_slice(box_type);
            Ok(())
        }
        Edit::Insert { path, at, bytes } => insert(file, path, *at, bytes),
        Edit::MoveBefore { path, before } => {
            let (start, end) = located(file, path)?;
            let moved: Vec<u8> = file.drain(start..end).collect();
            let (at, _) = located(file, before)?;
            file.splice(at..at, moved);
            Ok(())
        }
        Edit::Remove { path } => {
            let (start, end) = located(file, path)?;
            file.drain(start..end);
            Ok(())
        }
        Edit::SizeToEnd { path } => {
            let (start, end) = located(file, path)?;
            if end != file.len() {
                return Err(io::Error::other(format!("{path:?} is not the last box")));
            }
            bytes_at(file, start)?.copy_from_slice(&[0; 4]);
            Ok(())
        }
        Edit::CountChunkOffsetsFromMediaData { path, plus } => {
            let (media_data, _) = located(file, &[(b"mdat", 0)])?;
            let payload = u32::try_from(past(media_data, 8)?)
                .map_err(|_| io::Error::other("the mdat starts past 32 bits"))?;
            let (start, end) = located(file, path)?;
            let mut entry = past(start, 16)?;
            while entry < end {
                let field = bytes_at(file, entry)?;
                let counted = u32::from_be_bytes(*field)
                    .checked_sub(payload)
                    .and_then(|offset| offset.checked_add(*plus))
                    .ok_or_else(|| {
                        io::Error::other(format!("{path:?} has an offset before {payload}"))
                    })?;
                field.copy_from_slice(&counted.to_be_bytes());
                entry = past(entry, 4)?;
            }
            Ok(())
        }
        Edit::CopyPayload { path } => {
            let (start, end) = located(file, path)?;
            media_data.extend_from_slice(file.get(past(start, 8)?..end).unwrap_or_default());
            Ok(())
        }
        Edit::SetTrackFragmentBases => {
            // Why find every base before giving any: a base given grows its
            // `moof`, which moves the data the `data_offset` of the next
            // track fragment counts to.
            let mut bases = Vec::new();
            for moof in 0.. {
                let Some((moof_start, _)) = find(file, &[(b"moof", moof)]) else {
                    break;
                };
                for traf in 0.. {
                    let trun = [(b"moof", moof), (b"traf", traf), (b"trun", 0)];
                    let Some((trun_start, _)) = find(file, &trun) else {
                        break;
                    };
                    let data_offset = i32::from_be_bytes(*bytes_at(file, past(trun_start, 16)?)?);
                    let data = isize::try_from(data_offset)
                        .ok()
                        .and_then(|data_offset| moof_start.checked_add_signed(data_offset))
                        .ok_or_else(|| {
                            io::Error::other(format!("{trun:?} has its data before the file"))
                        })?;
                    bases.push((moof, traf, offset_in_media_data(file, data)?));
                }
            }
            for &(moof, traf, base) in bases.iter().rev() {
                let tfhd: BoxPath = vec![(b"moof", moof), (b"traf", traf), (b"tfhd", 0)];
                let (trun_start, _) =
                    located(file, &[(b"moof", moof), (b"traf", traf), (b"trun", 0)])?;
                bytes_at(file, past(trun_start, 16)?)?.copy_from_slice(&[0; 4]);
                let (tfhd_start, _) = located(file, &tfhd)?;
                let flags = bytes_at(file, past(tfhd_start, 8)?)?;
                let based =
                    (u32::from_be_bytes(*flags) | BASE_DATA_OFFSET_PRESENT) & !DEFAULT_BASE_IS_MOOF;
                flags.copy_from_slice(&based.to_be_bytes());
                let base = u64::try_from(base).map_err(io::Error::other)?;
                insert(file, &tfhd, 8, &base.to_be_bytes())?;
            }
            Ok(())
        }
    }
}

/// Where the file offset `data`, in the payload of an `mdat` of `file`, lies in the file the payloads of its `mdat`s, in file order, make
fn offset_in_media_data(file: &[u8], data: usize) -> io::Result<usize> {
    let mut before = 0_usize;
    for index in 0.. {
        let (start, end) = located(file, &[(b"mdat", index)])?;
        let payload = past(start, 8)?;
        if (payload..end).contains(&data) {
            return past(before, data.saturating_sub(payload));
        }
        before = past(before, end.saturating_sub(payload))?;
    }
    Err(io::Error::other(format!("no mdat holds {data}")))
}

/// The `base-data-offset-present` flag of a `tfhd`, ISO/IEC 14496-12 §8.8.7.1
const BASE_DATA_OFFSET_PRESENT: u32 = 0x1;

/// The `default-base-is-moof` flag of a `tfhd`, ISO/IEC 14496-12 §8.8.7.1
const DEFAULT_BASE_IS_MOOF: u32 = 0x2_0000;

/// Inserts `bytes` at `at` past the header of the box `path` names, growing it and the boxes that hold it by their length
fn insert(
    file: &mut Vec<u8>,
    path: &[(&[u8; 4], usize)],
    at: usize,
    bytes: &[u8],
) -> io::Result<()> {
    let more = u32::try_from(bytes.len()).map_err(io::Error::other)?;
    let (start, end) = located(file, path)?;
    let at = past(past(start, 8)?, at)?;
    if at > end {
        return Err(io::Error::other(format!("{path:?} ends before {at}")));
    }
    // Why find every box that holds it before growing any: a box grown
    // before its bytes are in no longer fits the file.
    let holders = (1..=path.len())
        .map(|depth| located(file, path.get(..depth).unwrap_or_default()))
        .collect::<io::Result<Vec<_>>>()?;
    for (holder, _) in holders {
        let size = bytes_at(file, holder)?;
        let grown = Some(u32::from_be_bytes(*size))
            .filter(|&size| size >= 8)
            .and_then(|size| size.checked_add(more))
            .ok_or_else(|| io::Error::other(format!("no 32-bit size at {holder}")))?;
        size.copy_from_slice(&grown.to_be_bytes());
    }
    file.splice(at..at, bytes.iter().copied());
    Ok(())
}

/// Where the box `path` names starts in `file`, and where it ends
fn located(file: &[u8], path: &[(&[u8; 4], usize)]) -> io::Result<(usize, usize)> {
    find(file, path).ok_or_else(|| io::Error::other(format!("no box {path:?}")))
}

/// `start` and `more` bytes past it
fn past(start: usize, more: usize) -> io::Result<usize> {
    start
        .checked_add(more)
        .ok_or_else(|| io::Error::other(format!("no offset {more} past {start}")))
}

/// The 4 bytes of `file` from `start`
fn bytes_at(file: &mut [u8], start: usize) -> io::Result<&mut [u8; 4]> {
    let end = past(start, 4)?;
    file.get_mut(start..end)
        .and_then(|bytes| <&mut [u8; 4]>::try_from(bytes).ok())
        .ok_or_else(|| io::Error::other(format!("no 4 bytes at {start}")))
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
    for rewritten in REWRITTEN {
        let _ = write!(
            text,
            "\n{}\n  {}\n  {} with its boxes rewritten\n",
            rewritten.name, rewritten.holds, rewritten.from
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
    for rewritten in REWRITTEN {
        if let Err(error) = write_rewritten(rewritten, &directory) {
            eprintln!("{}: {error}", rewritten.name);
            return ExitCode::FAILURE;
        }
        println!("wrote {DIRECTORY}/{}", rewritten.name);
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
